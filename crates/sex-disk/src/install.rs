// SPDX-License-Identifier: GPL-3.0-only
//! Installing SexOS beside whatever is already on a disk.
//!
//! The plan adds three partitions in free space (system slots A and B, and the SexOS
//! store), puts Limine on the EFI System partition the other systems already boot from,
//! and gives Limine's menu an entry for each system it finds there. Nothing that exists is
//! moved, resized or rewritten: the other systems' partitions are never written, and on
//! the shared ESP only new files are added (Limine in `\EFI\SEXOS`, its menu in
//! `\EFI\BOOT\LIMINE.CFG`, and Limine as the fallback loader only when there is none).
//!
//! Order matters for a failure half way: the slot image, the ESP files and the cleared
//! store come first, the partition table last. Until that last write the disk's table is
//! the one it had, and everything written so far sits in free space or in new ESP files.
use crate::fat::{self, Fat};
use crate::gpt::{
    Extent, Gpt, Guid, TYPE_BASIC_DATA, TYPE_ESP, TYPE_LINUX_FS, TYPE_LINUX_HOME,
    TYPE_LINUX_ROOT_X64, TYPE_LINUX_SWAP, TYPE_MSR, TYPE_SEXOS_BOOT, TYPE_SEXOS_DATA,
    TYPE_WINDOWS_RE,
};
use crate::probe::{self, Foreign, Layout};
use crate::{BlockDev, Error, Window, SECTOR};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const MIB: u64 = 2048;
/// Partitions start on 1 MiB boundaries, as every current partitioning tool does.
pub const ALIGN: u64 = MIB;
/// A new ESP, when the disk has none (a blank disk).
pub const ESP_SECTORS: u64 = 260 * MIB;
/// Each system slot holds one boot image: Limine's FAT volume with the kernel and services.
pub const SLOT_SECTORS: u64 = 256 * MIB;
/// The least store worth installing: the program store, DiskFS and a working filesystem.
pub const MIN_STORE_SECTORS: u64 = 1024 * MIB;
/// Minimum usable sectors required to safely replace a single partition with SexOS A/B + Store (1.5 GiB).
pub const MIN_REPLACE_SECTORS: u64 = 2 * SLOT_SECTORS + MIN_STORE_SECTORS;
/// The menu Limine reads, at a fixed size so updates can rewrite it in place.
pub const MENU_PATH: &str = "/EFI/BOOT/LIMINE.CFG";
pub const MENU_BYTES: usize = 16 * 1024;
pub const LOADER_PATH: &str = "/EFI/SEXOS/BOOTX64.EFI";
pub const FALLBACK_PATH: &str = "/EFI/BOOT/BOOTX64.EFI";
/// The first line of every menu SexOS writes; a menu without it belongs to someone else.
pub const MENU_MARK: &str = "# SexOS boot menu";
/// The store's start that is cleared, so Files formats it instead of reading old bytes:
/// past the DiskFS area, the program store and the old boot slot, into the filesystem.
pub const STORE_CLEAR_SECTORS: u64 = 131072 + 2048;
/// Places Limine looks for its menu before `\EFI\BOOT`: if another system put one there,
/// it would win, so the installer refuses rather than show the wrong menu.
const EARLIER_MENUS: [&str; 4] = ["/limine.cfg", "/limine/limine.cfg", "/boot/limine.cfg", "/boot/limine/limine.cfg"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Use free space, `Some(n)` sectors of it for the store or all of the largest run.
    FreeSpace(Option<u64>),
    /// A blank or SexOS-only disk: a new table over all of it.
    WholeDisk,
    /// Safely replace a single non-ESP/non-MSR GPT partition (0-based GPT entry index)
    /// with SexOS Slot A + Slot B + Store carved strictly inside that partition's LBA bounds.
    ReplacePartition(usize),
}

/// High-level partition classification for CLI and Candy GUI display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    Esp,
    Msr,
    Windows,
    Linux,
    SexOs,
    Other,
}

impl PartKind {
    pub fn from_type(t: Guid) -> Self {
        if t == TYPE_ESP {
            Self::Esp
        } else if t == TYPE_MSR {
            Self::Msr
        } else if t == TYPE_BASIC_DATA || t == TYPE_WINDOWS_RE {
            Self::Windows
        } else if t == TYPE_LINUX_FS || t == TYPE_LINUX_ROOT_X64 || t == TYPE_LINUX_HOME || t == TYPE_LINUX_SWAP {
            Self::Linux
        } else if t == TYPE_SEXOS_BOOT || t == TYPE_SEXOS_DATA {
            Self::SexOs
        } else {
            Self::Other
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Esp => "ESP",
            Self::Msr => "MSR",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::SexOs => "SexOS",
            Self::Other => "Other",
        }
    }

    pub const fn is_protected(self) -> bool {
        matches!(self, Self::Esp | Self::Msr)
    }
}

/// One existing GPT partition, inspected for `install parts` and the Candy GUI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartInfo {
    /// 0-based index in the GPT entry array (`#N` for users is `index + 1`).
    pub index: usize,
    pub kind: PartKind,
    pub name: String,
    pub first_lba: u64,
    pub sectors: u64,
    pub replaceable: bool,
    pub protect_reason: Option<&'static str>,
}

/// Returns the 1 MiB-aligned usable extent strictly inside `[first_lba, last_lba]`.
fn aligned_inside(first_lba: u64, last_lba: u64) -> Option<Extent> {
    let aligned_first = first_lba.next_multiple_of(ALIGN);
    let end_excl = last_lba.checked_add(1)?;
    if end_excl <= aligned_first {
        return None;
    }
    let sectors = (end_excl - aligned_first) / ALIGN * ALIGN;
    if sectors == 0 {
        return None;
    }
    Some(Extent { first: aligned_first, sectors })
}

fn entry_name_string(e: &crate::gpt::Entry) -> String {
    let mut buf = [0u8; 36];
    let n = e.name_ascii(&mut buf);
    if n == 0 {
        String::from("(unnamed)")
    } else {
        String::from(core::str::from_utf8(&buf[..n]).unwrap_or("(unnamed)"))
    }
}

/// Reads a GPT disk and returns `(total_sectors, partitions, aligned_free_runs)`.
pub fn list_partitions<D: BlockDev + ?Sized>(dev: &mut D) -> Result<(u64, Vec<PartInfo>, Vec<Extent>), Error> {
    let layout = probe::classify(dev)?;
    match layout {
        Layout::Foreign(Foreign::Gpt) | Layout::Sexos { .. } => {}
        _ => return Err(Error::Unsupported),
    }
    let gpt = Gpt::read(dev)?;
    if gpt.from_backup {
        return Err(Error::Corrupt);
    }
    gpt.check()?;
    let mut parts = Vec::new();
    for (index, e) in gpt.used() {
        let kind = PartKind::from_type(e.type_guid());
        let sectors = e.sectors();
        let usable = aligned_inside(e.first(), e.last()).map_or(0, |ex| ex.sectors);
        let protect_reason = if kind == PartKind::Esp {
            Some("EFI System Partition")
        } else if kind == PartKind::Msr {
            Some("Microsoft Reserved")
        } else if sectors < MIN_REPLACE_SECTORS || usable < MIN_REPLACE_SECTORS {
            Some("smaller than 1.5 GiB")
        } else {
            None
        };
        parts.push(PartInfo {
            index,
            kind,
            name: entry_name_string(e),
            first_lba: e.first(),
            sectors,
            replaceable: protect_reason.is_none(),
            protect_reason,
        });
    }
    let free = gpt.free(ALIGN);
    Ok((dev.sectors(), parts, free))
}

/// Metadata about the single partition being replaced in [`Mode::ReplacePartition`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplacedPart {
    pub index: usize,
    pub kind: PartKind,
    pub name: String,
    pub first_lba: u64,
    pub last_lba: u64,
    pub sectors: u64,
}

/// Metadata about an existing partition that will remain untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UntouchedPart {
    pub index: usize,
    pub kind: PartKind,
    pub name: String,
    pub sectors: u64,
}

/// Another system Limine's menu will start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Other { pub name: String, pub loader: String }

#[derive(Clone, Debug)]
pub struct Plan {
    pub gpt: Gpt,
    pub new_table: bool,
    pub esp: Extent,
    pub new_esp: bool,
    pub slot_a: Extent,
    pub slot_b: Extent,
    pub store: Extent,
    pub slot_a_guid: Guid,
    pub slot_b_guid: Guid,
    pub store_guid: Guid,
    pub others: Vec<Other>,
    /// The ESP has no fallback loader: Limine goes there too, so firmware finds it unasked.
    pub fallback_free: bool,
    /// Present when planning [`Mode::ReplacePartition`].
    pub replaced: Option<ReplacedPart>,
    /// Existing partitions preserved byte-for-byte on the disk.
    pub untouched: Vec<UntouchedPart>,
}

/// Entropy for the new disk and partition GUIDs.
pub struct Seeds(pub [[u8; 16]; 5]);

pub fn plan<D: BlockDev + ?Sized>(dev: &mut D, mode: Mode, seeds: &Seeds) -> Result<Plan, Error> {
    let layout = probe::classify(dev)?;
    let total = dev.sectors();
    let (mut gpt, new_table) = match (mode, layout) {
        (Mode::WholeDisk, Layout::Blank | Layout::LegacySexos) => (Gpt::new_empty(total, Guid::random(seeds.0[0]))?, true),
        (Mode::WholeDisk, _) => return Err(Error::Exists),
        (Mode::FreeSpace(_), Layout::Foreign(Foreign::Gpt)) => (Gpt::read(dev)?, false),
        (Mode::FreeSpace(_), Layout::Sexos { .. }) => return Err(Error::Exists),
        (Mode::FreeSpace(_), Layout::Blank | Layout::LegacySexos) => (Gpt::new_empty(total, Guid::random(seeds.0[0]))?, true),
        (Mode::FreeSpace(_), _) => return Err(Error::Unsupported),
        (Mode::ReplacePartition(_), Layout::Foreign(Foreign::Gpt)) => (Gpt::read(dev)?, false),
        (Mode::ReplacePartition(_), Layout::Sexos { .. }) => return Err(Error::Exists),
        (Mode::ReplacePartition(_), _) => return Err(Error::Unsupported),
    };
    // A table recovered from its backup would need repairing first; that is the other
    // system's tools' job, and refusing here is before any write.
    if gpt.from_backup { return Err(Error::Corrupt); }
    gpt.check()?;
    let existing_esp = if new_table { None } else { gpt.find_type(TYPE_ESP).next().map(|e| Extent { first: e.first(), sectors: e.sectors() }) };
    let (esp, new_esp, slot_a, slot_b, store, replaced, untouched) = match mode {
        Mode::ReplacePartition(idx) => {
            let Some(target) = gpt.entries.get(idx).filter(|e| e.is_used()) else {
                return Err(Error::NotFound);
            };
            let kind = PartKind::from_type(target.type_guid());
            if kind.is_protected() {
                return Err(Error::Invalid);
            }
            let target_first = target.first();
            let target_last = target.last();
            let target_sectors = target.sectors();
            if target_sectors < MIN_REPLACE_SECTORS {
                return Err(Error::NoSpace);
            }
            let inner = aligned_inside(target_first, target_last).ok_or(Error::NoSpace)?;
            if inner.sectors < MIN_REPLACE_SECTORS {
                return Err(Error::NoSpace);
            }
            let replaced = ReplacedPart {
                index: idx,
                kind,
                name: entry_name_string(target),
                first_lba: target_first,
                last_lba: target_last,
                sectors: target_sectors,
            };
            let untouched: Vec<UntouchedPart> = gpt
                .used()
                .filter(|(i, _)| *i != idx)
                .map(|(i, e)| UntouchedPart {
                    index: i,
                    kind: PartKind::from_type(e.type_guid()),
                    name: entry_name_string(e),
                    sectors: e.sectors(),
                })
                .collect();
            let (esp, new_esp, base, usable) = match existing_esp {
                Some(e) => (e, false, inner.first, inner.sectors),
                None => {
                    if inner.sectors < ESP_SECTORS + MIN_REPLACE_SECTORS {
                        return Err(Error::NoSpace);
                    }
                    (
                        Extent { first: inner.first, sectors: ESP_SECTORS },
                        true,
                        inner.first + ESP_SECTORS,
                        inner.sectors - ESP_SECTORS,
                    )
                }
            };
            let slot_a = Extent { first: base, sectors: SLOT_SECTORS };
            let slot_b = Extent { first: base + SLOT_SECTORS, sectors: SLOT_SECTORS };
            let store = Extent { first: base + 2 * SLOT_SECTORS, sectors: usable - 2 * SLOT_SECTORS };
            // Remove ONLY the target partition entry; every other entry stays at its exact index and bytes.
            gpt.remove(idx)?;
            (esp, new_esp, slot_a, slot_b, store, Some(replaced), untouched)
        }
        Mode::FreeSpace(_) | Mode::WholeDisk => {
            let untouched: Vec<UntouchedPart> = if new_table {
                Vec::new()
            } else {
                gpt.used()
                    .map(|(i, e)| UntouchedPart {
                        index: i,
                        kind: PartKind::from_type(e.type_guid()),
                        name: entry_name_string(e),
                        sectors: e.sectors(),
                    })
                    .collect()
            };
            let mut free = gpt.free(ALIGN);
            let mut take = |sectors: u64| -> Result<Extent, Error> {
                let run = free.iter_mut().filter(|r| r.sectors >= sectors).min_by_key(|r| r.first).ok_or(Error::NoSpace)?;
                let got = Extent { first: run.first, sectors };
                run.first += sectors; run.sectors -= sectors;
                Ok(got)
            };
            let (esp, new_esp) = match existing_esp { Some(e) => (e, false), None => (take(ESP_SECTORS)?, true) };
            // Slots and store go together in the largest free run.
            let need_store = match mode { Mode::FreeSpace(Some(n)) => n.next_multiple_of(ALIGN), _ => MIN_STORE_SECTORS };
            if need_store < MIN_STORE_SECTORS { return Err(Error::Invalid); }
            free.sort_by(|a, b| b.sectors.cmp(&a.sectors));
            let run = *free.first().ok_or(Error::NoSpace)?;
            if run.sectors < 2 * SLOT_SECTORS + need_store { return Err(Error::NoSpace); }
            let slot_a = Extent { first: run.first, sectors: SLOT_SECTORS };
            let slot_b = Extent { first: run.first + SLOT_SECTORS, sectors: SLOT_SECTORS };
            let store_sectors = match mode { Mode::FreeSpace(Some(_)) => need_store, _ => run.sectors - 2 * SLOT_SECTORS };
            let store = Extent { first: run.first + 2 * SLOT_SECTORS, sectors: store_sectors };
            (esp, new_esp, slot_a, slot_b, store, None, untouched)
        }
    };
    let (slot_a_guid, slot_b_guid, store_guid) = (Guid::random(seeds.0[1]), Guid::random(seeds.0[2]), Guid::random(seeds.0[3]));
    if new_esp { gpt.add(TYPE_ESP, Guid::random(seeds.0[4]), esp.first, esp.sectors, "EFI system partition")?; }
    gpt.add(TYPE_SEXOS_BOOT, slot_a_guid, slot_a.first, slot_a.sectors, "SexOS system A")?;
    gpt.add(TYPE_SEXOS_BOOT, slot_b_guid, slot_b.first, slot_b.sectors, "SexOS system B")?;
    gpt.add(TYPE_SEXOS_DATA, store_guid, store.first, store.sectors, "SexOS")?;
    gpt.check()?;
    let (others, fallback_free) = if new_esp { (Vec::new(), true) } else {
        let mut w = Window::new(dev, esp.first, esp.sectors)?;
        let f = Fat::open(&mut w)?;
        for path in EARLIER_MENUS { if f.find(&mut w, path).is_ok() { return Err(Error::Exists); } }
        if let Ok(menu) = f.read_file(&mut w, MENU_PATH) { if !menu.starts_with(MENU_MARK.as_bytes()) { return Err(Error::Exists); } }
        let fallback_free = f.find(&mut w, FALLBACK_PATH).is_err();
        (find_others(&f, &mut w)?, fallback_free)
    };
    Ok(Plan { gpt, new_table, esp, new_esp, slot_a, slot_b, store, slot_a_guid, slot_b_guid, store_guid, others, fallback_free, replaced, untouched })
}

/// The systems already on an ESP, by the loaders they leave in `\EFI`.
pub fn find_others<D: BlockDev + ?Sized>(f: &Fat, dev: &mut D) -> Result<Vec<Other>, Error> {
    let mut out = Vec::new();
    let Ok(dirs) = f.list(dev, "/EFI") else { return Ok(out) };
    for d in dirs.iter().filter(|d| d.is_dir && d.name != "." && d.name != "..") {
        let upper = d.name.to_ascii_uppercase();
        if upper == "BOOT" || upper == "SEXOS" { continue; }
        let dir = format!("/EFI/{}", d.name);
        let files = f.list(dev, &dir).unwrap_or_default();
        let has = |n: &str| files.iter().find(|e| !e.is_dir && e.name.eq_ignore_ascii_case(n)).map(|e| format!("{dir}/{}", e.name));
        let (name, loader) = if upper == "MICROSOFT" {
            let boot = f.list(dev, &format!("{dir}/Boot")).unwrap_or_default();
            match boot.iter().find(|e| e.name.eq_ignore_ascii_case("bootmgfw.efi")) {
                Some(e) => (String::from("Windows"), format!("{dir}/Boot/{}", e.name)),
                None => continue,
            }
        } else if let Some(l) = has("shimx64.efi").or_else(|| has("grubx64.efi")) {
            (d.name.clone(), l)
        } else if let Some(l) = has("systemd-bootx64.efi") {
            (String::from("Linux (systemd-boot)"), l)
        } else if let Some(e) = files.iter().find(|e| !e.is_dir && e.name.to_ascii_uppercase().ends_with(".EFI")) {
            (d.name.clone(), format!("{dir}/{}", e.name))
        } else { continue };
        out.push(Other { name, loader });
        if out.len() == 8 { break; }
    }
    if let Ok(e) = f.find(dev, FALLBACK_PATH) {
        if out.is_empty() && !e.is_dir { out.push(Other { name: String::from("Other system"), loader: String::from(FALLBACK_PATH) }); }
    }
    Ok(out)
}

/// Limine's menu for slot `slot` of an image whose own menu is `image_cfg`: the image's
/// entries with `boot:///` pointed at the slot's partition, then the other systems.
/// Padded with a comment to exactly [`MENU_BYTES`] so it can be rewritten in place.
pub fn menu(image_cfg: &str, slot: Guid, others: &[Other]) -> Result<Vec<u8>, Error> {
    let blocks: Vec<String> = others.iter().map(|o| {
        let name: String = o.name.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').collect();
        format!(":{name}\n    PROTOCOL=efi_chainload\n    IMAGE_PATH=boot://{}\n\n", o.loader)
    }).collect();
    compose(image_cfg, slot, &blocks, &crate::update::State::fresh())
}

/// The menu text: `image_cfg`'s entries aimed at `slot`, then `extra` entries as written,
/// with `state` and the default entry it implies at the top, padded to [`MENU_BYTES`].
pub fn compose(image_cfg: &str, slot: Guid, extra: &[String], state: &crate::update::State) -> Result<Vec<u8>, Error> {
    let target = format!("guid://{}/", core::str::from_utf8(&slot.text()).unwrap());
    let mut out = String::new();
    out.push_str(MENU_MARK);
    out.push_str(". Written by SexOS install; SexOS rewrites it in place.\n");
    out.push_str(&state.line());
    out.push_str(&format!("DEFAULT_ENTRY={}\n", state.default_entry()));
    let mut entries = false;
    for line in image_cfg.lines() {
        let t = line.trim_start();
        if t.starts_with(':') { entries = true; }
        if !entries && (t.starts_with("DEFAULT_ENTRY") || t.starts_with('#')) { continue; }
        if t.contains("boot.img") { continue; }
        out.push_str(&line.replace("boot:///", &target));
        out.push('\n');
    }
    if !entries { return Err(Error::Corrupt); }
    for b in extra { out.push_str(b); }
    pad(out.into_bytes())
}

/// Fills a menu out to its fixed size with one comment line.
pub(crate) fn pad(mut bytes: Vec<u8>) -> Result<Vec<u8>, Error> {
    if bytes.len() + 2 > MENU_BYTES { return Err(Error::NoSpace); }
    bytes.push(b'#');
    bytes.resize(MENU_BYTES - 1, b' ');
    bytes.push(b'\n');
    Ok(bytes)
}

/// What the installer did, for the caller to report.
#[derive(Clone, Copy, Debug)]
pub struct Done { pub menu: fat::Extent, pub image_sectors: u64 }

/// Carries out a plan with `image` (a SexOS boot image: FAT with Limine, its menu, the
/// kernel and services). `progress` hears each step's name.
pub fn apply<D: BlockDev + ?Sized, I: BlockDev + ?Sized>(dev: &mut D, plan: &mut Plan, image: &mut I, progress: &mut dyn FnMut(&str)) -> Result<Done, Error> {
    // What the image brings: its menu and Limine itself.
    let img = Fat::open(image)?;
    let image_cfg = img.read_file(image, "/limine.cfg")?;
    let loader = img.read_file(image, FALLBACK_PATH)?;
    let image_cfg = core::str::from_utf8(&image_cfg).map_err(|_| Error::Corrupt)?;
    let menu = menu(image_cfg, plan.slot_a_guid, &plan.others)?;
    let image_sectors = image.sectors();
    if image_sectors > plan.slot_a.sectors { return Err(Error::NoSpace); }

    progress("system slot A");
    copy(image, dev, plan.slot_a.first, image_sectors)?;
    progress("clearing the store");
    zero(dev, plan.store.first, STORE_CLEAR_SECTORS.min(plan.store.sectors))?;
    // Slot B starts empty: no stale boot sector from whatever the space held.
    zero(dev, plan.slot_b.first, 64)?;
    if plan.new_table {
        // A new table over an old layout: clear the backup area before anything reads it.
        zero(dev, 0, 34)?;
        zero(dev, dev.sectors() - 34, 34)?;
    }
    let mut esp = Window::new(dev, plan.esp.first, plan.esp.sectors)?;
    if plan.new_esp {
        progress("formatting the EFI system partition");
        fat::format_fat32(&mut esp, b"EFI SYSTEM ")?;
    }
    progress("adding Limine to the EFI system partition");
    let f = Fat::open(&mut esp)?;
    f.mkdir_p(&mut esp, "/EFI/SEXOS")?;
    f.write_file(&mut esp, LOADER_PATH, &loader)?;
    f.mkdir_p(&mut esp, "/EFI/BOOT")?;
    if plan.fallback_free { f.write_file(&mut esp, FALLBACK_PATH, &loader)?; }
    let at = f.write_file(&mut esp, MENU_PATH, &menu)?;
    let menu_at = fat::Extent { first_lba: at.first_lba + plan.esp.first, sectors: at.sectors };
    progress("writing the partition table");
    plan.gpt.write(dev)?;
    Ok(Done { menu: menu_at, image_sectors })
}

const CHUNK: usize = 256;
fn copy<S: BlockDev + ?Sized, D: BlockDev + ?Sized>(src: &mut S, dst: &mut D, at: u64, sectors: u64) -> Result<(), Error> {
    let mut buf = vec![0u8; CHUNK * SECTOR];
    let mut done = 0;
    while done < sectors {
        let n = (sectors - done).min(CHUNK as u64) as usize;
        src.read(done, &mut buf[..n * SECTOR])?;
        dst.write(at + done, &buf[..n * SECTOR])?;
        done += n as u64;
    }
    Ok(())
}
fn zero<D: BlockDev + ?Sized>(dst: &mut D, at: u64, sectors: u64) -> Result<(), Error> {
    let buf = vec![0u8; CHUNK * SECTOR];
    let mut done = 0;
    while done < sectors {
        let n = (sectors - done).min(CHUNK as u64) as usize;
        dst.write(at + done, &buf[..n * SECTOR])?;
        done += n as u64;
    }
    Ok(())
}

/// The plan in words, one line each, for Spindle and the installer window.
pub fn describe(plan: &Plan) -> Vec<String> {
    let gib = |s: u64| if s < 2048 * 1024 { format!("{} MiB", s / 2048) } else { format!("{}.{} GiB", s / (2048 * 1024), s % (2048 * 1024) * 10 / (2048 * 1024)) };
    let mut out = Vec::new();
    if let Some(ref rep) = plan.replaced {
        out.push(format!(
            "REPLACES ONLY Partition #{} ({}, {}, {}) with SexOS A/B + Store",
            rep.index + 1,
            rep.name,
            rep.kind.label(),
            gib(rep.sectors)
        ));
    } else {
        out.push(if plan.new_table { String::from("New partition table over the whole disk") } else { String::from("Keeps every existing partition; uses free space only") });
    }
    out.push(format!("EFI system partition: {} ({})", if plan.new_esp { "new" } else { "shared, files added in \\EFI\\SEXOS" }, gib(plan.esp.sectors)));
    out.push(format!("SexOS system A and B: {} each", gib(plan.slot_a.sectors)));
    out.push(format!("SexOS store: {}", gib(plan.store.sectors)));
    if plan.replaced.is_some() {
        for u in &plan.untouched {
            out.push(format!("UNTOUCHED: #{} {} ({}, {})", u.index + 1, u.name, u.kind.label(), gib(u.sectors)));
        }
    }
    for o in &plan.others { out.push(format!("Boot menu keeps: {} ({})", o.name, o.loader)); }
    out
}

#[cfg(test)]
pub(crate) mod tests;
