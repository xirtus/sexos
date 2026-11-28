//! `install`: puts SexOS beside whatever is on the disk (see `sex_disk::install`).
//!
//! Files is the only service with a route to the disk driver, so the installer lives here
//! and reaches the whole disk through the driver's raw requests. Without `go` it only
//! plans and says what it would do; nothing is written.
use crate::backends::diskfs::DiskFs;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_disk::install::{self, Mode, Seeds};
use sex_disk::{BlockDev, Error, RoDev, SECTOR};
use sex_pdx::fs as w;
use sex_pdx::memory_region::{QUERY_VIEW, READ};
use sexfs::{Device, Fs, Kind, Memory, Slot, BLOCK};

/// The whole disk through the driver's raw requests, in runs as long as the shared buffer.
pub(crate) struct RawDisk { pub(crate) sectors: u64 }
impl RawDisk {
    fn buffer() -> Result<&'static mut [u8], Error> {
        let va = crate::vfs::diskfs_bridge_get_buf_va();
        let len = crate::vfs::diskfs_bridge_buf_len() as usize;
        if va == 0 || va == u64::MAX || len < 4096 { return Err(Error::Io); }
        Ok(unsafe { core::slice::from_raw_parts_mut(va as *mut u8, len) })
    }
}
impl BlockDev for RawDisk {
    fn sectors(&self) -> u64 { self.sectors }
    fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), Error> {
        let buf = Self::buffer()?;
        for (i, chunk) in out.chunks_mut(buf.len()).enumerate() {
            let at = lba * SECTOR as u64 + (i * buf.len()) as u64;
            if DiskFs::diskfs_block_call(sex_pdx::BLOCK_RAW_READ, at, chunk.len() as u64, sex_pdx::SLOT_BUF_LEND) != 0 { return Err(Error::Io); }
            chunk.copy_from_slice(&buf[..chunk.len()]);
        }
        Ok(())
    }
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), Error> {
        let buf = Self::buffer()?;
        for (i, chunk) in data.chunks(buf.len()).enumerate() {
            let at = lba * SECTOR as u64 + (i * buf.len()) as u64;
            buf[..chunk.len()].copy_from_slice(chunk);
            if DiskFs::diskfs_block_call(sex_pdx::BLOCK_RAW_WRITE, at, chunk.len() as u64, sex_pdx::SLOT_BUF_LEND) != 0 { return Err(Error::Io); }
        }
        Ok(())
    }
}

/// An image file in Files' own filesystem.
pub(crate) struct FileDev<'a> { pub(crate) fs: &'a mut Fs<'static, crate::fs_service::Disk>, pub(crate) id: u64, pub(crate) sectors: u64 }
impl BlockDev for FileDev<'_> {
    fn sectors(&self) -> u64 { self.sectors }
    fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), Error> {
        if lba + (out.len() / SECTOR) as u64 > self.sectors { return Err(Error::Invalid); }
        let n = self.fs.read(self.id, lba * SECTOR as u64, out).map_err(|_| Error::Io)?;
        if n != out.len() { return Err(Error::Corrupt); }
        Ok(())
    }
    fn write(&mut self, _: u64, _: &[u8]) -> Result<(), Error> { Err(Error::Io) }
}

/// GUIDs need to be unique, not secret: the time stamp counter, stirred.
pub(crate) fn seeds() -> Seeds {
    let mut x = unsafe { core::arch::x86_64::_rdtsc() } ^ sex_pdx::get_ticks().rotate_left(32);
    let mut next = || { x = x.wrapping_add(0x9E37_79B9_7F4A_7C15); let mut z = x; z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9); z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB); z ^ (z >> 31) };
    Seeds(core::array::from_fn(|_| { let mut b = [0u8; 16]; b[..8].copy_from_slice(&next().to_le_bytes()); b[8..].copy_from_slice(&next().to_le_bytes()); b }))
}

pub(crate) fn why(e: Error) -> &'static str {
    match e {
        Error::NoSpace => "Not enough free space. SexOS needs 1.5 GiB that no partition uses; shrink another system's partition first (in that system's own disk tool).",
        Error::Exists => "Something is in the way: SexOS is already on this disk, or another boot menu (limine.cfg) would hide SexOS's.",
        Error::Unsupported => "This disk's layout isn't one SexOS installs beside: it needs a GPT disk (every UEFI PC with Windows 8 or later, or a current Linux).",
        Error::Corrupt => "The disk's partition table or EFI system partition is damaged. Nothing was written.",
        Error::Io => "A disk error stopped the install.",
        Error::Invalid => "The store must be at least 1 GiB.",
        Error::NotFound => "The boot image lacks Limine or its menu.",
        Error::BadSignature => "The update is not signed by this system's release key, or its image is not the one the signature covers.",
        Error::Older => "The update is not newer than the running system.",
        Error::Pending => "An update already waits for a restart. Restart to try it, then update again.",
    }
}

fn fmt_size(sectors: u64) -> String {
    if sectors < 2048 * 1024 {
        format!("{} MiB", sectors / 2048)
    } else {
        let gib = sectors / (2048 * 1024);
        let tenths = sectors % (2048 * 1024) * 10 / (2048 * 1024);
        format!("{gib}.{tenths} GiB")
    }
}

fn kind_code(k: install::PartKind) -> &'static str {
    match k {
        install::PartKind::Esp => "esp",
        install::PartKind::Msr => "msr",
        install::PartKind::Windows => "win",
        install::PartKind::Linux => "lin",
        install::PartKind::SexOs => "sex",
        install::PartKind::Other => "oth",
    }
}

fn fmt_part_desc(p: &install::PartInfo) -> String {
    let mut out = String::new();
    if p.name != "(unnamed)" {
        out.push_str(&p.name);
    }
    if !p.fs_label.is_empty() && p.fs_label != p.name {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push('"');
        out.push_str(&p.fs_label);
        out.push('"');
    }
    if let Some(u) = p.short_uuid() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push('[');
        out.push_str(&u);
        out.push(']');
    }
    if out.is_empty() {
        out.push_str("(unnamed)");
    }
    out
}

fn fmt_raw_name(p: &install::PartInfo) -> String {
    match p.fs_type {
        Some(fs) => {
            if !p.fs_label.is_empty() {
                format!("{} ({})", p.fs_label, fs)
            } else if p.name != "(unnamed)" {
                format!("{} ({})", p.name, fs)
            } else if let Some(u) = p.short_uuid() {
                format!("{} [{}]", fs, u)
            } else {
                String::from(fs)
            }
        }
        None => p.name.clone(),
    }
}

fn list_report(disk: &mut RawDisk, raw: bool) -> String {
    let mut text = String::new();
    match install::list_partitions(disk) {
        Ok((total, parts, free)) => {
            if raw {
                text.push_str(&format!("DISK {total}\n"));
                for p in &parts {
                    text.push_str(&format!(
                        "PART {} {} {} {} {} {}\n",
                        p.index + 1,
                        kind_code(p.kind),
                        p.first_lba,
                        p.sectors,
                        if p.replaceable { 1 } else { 0 },
                        fmt_raw_name(p)
                    ));
                }
                for f in &free {
                    if f.sectors >= install::MIN_REPLACE_SECTORS {
                        text.push_str(&format!("FREE {} {}\n", f.first, f.sectors));
                    }
                }
            } else {
                text.push_str(&format!("GPT Disk ({}):\n", fmt_size(total)));
                text.push_str("  #   Type     FS     Size        Status                         Name / Label\n");
                for p in &parts {
                    let status = if p.replaceable {
                        String::from("replaceable")
                    } else {
                        format!("LOCKED ({})", p.protect_reason.unwrap_or("protected"))
                    };
                    text.push_str(&format!(
                        "  {:<3} {:<8} {:<6} {:<11} {:<30} {}\n",
                        p.index + 1,
                        p.kind.label(),
                        p.fs_type.unwrap_or("-"),
                        fmt_size(p.sectors),
                        status,
                        fmt_part_desc(p)
                    ));
                }
                let big_free: u64 = free
                    .iter()
                    .filter(|f| f.sectors >= install::MIN_REPLACE_SECTORS)
                    .map(|f| f.sectors)
                    .max()
                    .unwrap_or(0);
                if big_free > 0 {
                    text.push_str(&format!("Largest free space: {} (usable with `install`)\n", fmt_size(big_free)));
                }
                text.push_str("To preview replacing one partition: install replace <N>\n");
            }
        }
        Err(e) => {
            if raw {
                text.push_str(&format!("ERR {:?}\n", e));
            } else {
                text.push_str("Cannot list partitions. ");
                text.push_str(why(e));
                text.push('\n');
            }
        }
    }
    text
}

/// Plans, lists partitions, or with `INSTALL_GO` installs. Writes what happened into `out`
/// as lines of text and returns their length. `image` is a file path, or empty for the running system.
pub fn run(fs: Option<&mut Fs<'static, crate::fs_service::Disk>>, image: &[u8], arg: u64, flags: u64, out: &mut [u8]) -> Result<u32, sexfs::Error> {
    let disk_sectors = DiskFs::diskfs_block_call(sex_pdx::BLOCK_LAYOUT, 0, 0, 0) >> 8;
    let finish = |text: String, out: &mut [u8]| { let n = text.len().min(out.len()); out[..n].copy_from_slice(&text.as_bytes()[..n]); Ok(n as u32) };
    let mut disk = RawDisk { sectors: disk_sectors };
    if flags & w::INSTALL_LIST != 0 {
        let raw = flags & w::INSTALL_LIST_RAW != 0;
        let text = list_report(&mut disk, raw);
        sex_pdx::serial_record!("[files.install.list] raw={} bytes={}", raw as u8, text.len());
        return finish(text, out);
    }
    let replace = flags & w::INSTALL_REPLACE != 0;
    let go = flags & w::INSTALL_GO != 0;
    let mut text = String::new();
    let layout = sex_disk::probe::classify(&mut disk);
    let mode = if replace {
        if arg == 0 || arg > 1024 {
            text.push_str("Not installed. Partition number must be >= 1 (run `install parts` to list partitions).\n");
            return finish(text, out);
        }
        Mode::ReplacePartition((arg - 1) as usize)
    } else {
        match layout {
            Ok(sex_disk::probe::Layout::Blank) | Ok(sex_disk::probe::Layout::LegacySexos) => Mode::WholeDisk,
            _ => Mode::FreeSpace(if arg == 0 { None } else { Some(arg * 2048) }),
        }
    };
    let mut plan = match install::plan(&mut disk, mode, &seeds()) {
        Ok(p) => p,
        Err(e) => {
            sex_pdx::serial_record!("[files.install.refused] error={:?} layout={:?} replace={}", e, layout, replace as u8);
            text.push_str("Not installed. ");
            if replace {
                match e {
                    Error::Invalid => text.push_str("That partition is protected (EFI System Partition or Microsoft Reserved) and cannot be replaced."),
                    Error::NotFound => text.push_str(&format!("Partition #{} was not found on this disk. Run `install parts` to see valid numbers.", arg)),
                    Error::NoSpace => text.push_str("That partition is smaller than 1.5 GiB (SexOS needs 256 MiB Slot A + 256 MiB Slot B + at least 1 GiB Store)."),
                    _ => text.push_str(why(e)),
                }
            } else {
                text.push_str(why(e));
            }
            text.push('\n');
            return finish(text, out);
        }
    };
    if mode == Mode::WholeDisk { text.push_str("This disk is blank or holds only an old SexOS: it becomes SexOS's alone.\n"); }
    for line in install::describe(&plan) { text.push_str(&line); text.push('\n'); }
    sex_pdx::serial_record!("[files.install.plan] whole={} esp_new={} store_sectors={} others={} replace={}", plan.new_table as u8, plan.new_esp as u8, plan.store.sectors, plan.others.len(), replace as u8);
    if !go {
        if replace {
            text.push_str(&format!("Nothing written yet. To install: install replace {} go\n", arg));
        } else {
            text.push_str("Nothing written yet. To install: install go");
            if arg != 0 { text.push_str(&format!(" {}", arg / 1024)); }
            text.push('\n');
        }
        return finish(text, out);
    }
    // The image: the running system's, or a file.
    let mut progress = |step: &str| sex_pdx::serial_record!("[files.install.step] {}", step);
    let done = if image.is_empty() {
        let info = sex_pdx::memory_region_info(w::BOOT_IMAGE_SLOT).map_err(|_| sexfs::Error::NotFound)?;
        let view = sex_pdx::memory_region_view(w::BOOT_IMAGE_SLOT, 0, info.length, QUERY_VIEW | READ).map_err(|_| sexfs::Error::NotFound)?;
        let bytes = unsafe { core::slice::from_raw_parts(view.virtual_address as *const u8, view.length as usize) };
        let bytes = &bytes[..bytes.len() & !(SECTOR - 1)];
        install::apply(&mut disk, &mut plan, &mut RoDev(bytes), &mut progress)
    } else {
        let fs = fs.ok_or(sexfs::Error::NotFound)?;
        let st = fs.open(image)?;
        if st.kind == Kind::Dir { return Err(sexfs::Error::IsDir); }
        install::apply(&mut disk, &mut plan, &mut FileDev { fs, id: st.id, sectors: st.size / SECTOR as u64 }, &mut progress)
    };
    // Only what reached the disk counts.
    let done = done.and_then(|d| if DiskFs::diskfs_block_sync() == 0 { Ok(d) } else { Err(Error::Io) });
    match done {
        Ok(d) => {
            sex_pdx::serial_record!("[files.install.done] bytes={} slot_a={} store={} menu_lba={}", d.image_sectors * SECTOR as u64, plan.slot_a.first, plan.store.first, d.menu.first_lba);
            if let Some((count, bytes)) = seed_store_programs(&mut disk, plan.store.first, plan.store.sectors) {
                text.push_str(&format!("Seeded {} programs ({} MiB) into /bin for offline boot.\n", count, bytes >> 20));
            }
            text.push_str("Installed. Restart and pick SexOS in the boot menu.\n");
            if !plan.fallback_free {
                text.push_str("This PC's firmware starts the other system by default: choose \\EFI\\SEXOS\\BOOTX64.EFI once in its boot menu or setup (often F12, F8 or Esc at power on) and make it first.\n");
            }
        }
        Err(e) => {
            sex_pdx::serial_record!("[files.install.failed] error={:?}", e);
            text.push_str("Install stopped. "); text.push_str(why(e));
            text.push_str(" Unless the disk could not confirm its last writes, the partition table was not changed, so every system starts as before.\n");
        }
    }
    text.push_str("The running system stops writing to this disk until it restarts.\n");
    finish(text, out)
}

struct StoreSliceDev<'a> {
    disk: &'a mut RawDisk,
    base_lba: u64,
    blocks: u64,
}

impl Device for StoreSliceDev<'_> {
    fn blocks(&self) -> u64 { self.blocks }
    fn run_blocks(&self) -> usize {
        (crate::vfs::diskfs_bridge_buf_len() as usize / BLOCK).max(1)
    }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> core::result::Result<(), ()> {
        self.read_run(block, out)
    }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> core::result::Result<(), ()> {
        self.write_run(block, data)
    }
    fn read_run(&mut self, first: u64, out: &mut [u8]) -> core::result::Result<(), ()> {
        if out.is_empty() || out.len() % BLOCK != 0 { return Err(()); }
        let n = (out.len() / BLOCK) as u64;
        if first.checked_add(n).is_none_or(|e| e > self.blocks) { return Err(()); }
        let spb = (BLOCK / SECTOR) as u64;
        self.disk.read(self.base_lba + first * spb, out).map_err(|_| ())
    }
    fn write_run(&mut self, first: u64, data: &[u8]) -> core::result::Result<(), ()> {
        if data.is_empty() || data.len() % BLOCK != 0 { return Err(()); }
        let n = (data.len() / BLOCK) as u64;
        if first.checked_add(n).is_none_or(|e| e > self.blocks) { return Err(()); }
        let spb = (BLOCK / SECTOR) as u64;
        self.disk.write(self.base_lba + first * spb, data).map_err(|_| ())
    }
    fn flush(&mut self) -> core::result::Result<(), ()> {
        if DiskFs::diskfs_block_sync() == 0 { Ok(()) } else { Err(()) }
    }
}

fn alloc_vec<T: Clone>(len: usize, val: T) -> Option<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).ok()?;
    v.resize(len, val);
    Some(v)
}

/// Formats the newly created SexOS Store `sexfs` partition and seeds `/bin/<name>` from any
/// mounted Live USB `/media/<vol>/programs/` directory so apps work offline without USB attached.
/// Graceful no-op when no `/media/` USB programs are present.
fn seed_store_programs(disk: &mut RawDisk, store_first: u64, store_sectors: u64) -> Option<(usize, u64)> {
    let progs = crate::volumes::usb_programs();
    if progs.is_empty() {
        return None;
    }
    let spb = (BLOCK / SECTOR) as u64;
    if store_sectors <= w::BASE_LBA + 512 * spb {
        return None;
    }
    let blocks = (store_sectors - w::BASE_LBA) / spb;
    let cache_len = 64usize;
    let mut cache = alloc_vec(cache_len, [0u8; BLOCK])?;
    let mut slots = alloc_vec(cache_len, Slot::EMPTY)?;
    let full_words = (blocks as usize).div_ceil(64) + 1;
    let (mut used, mut pending) = match (alloc_vec(full_words, 0u64), alloc_vec(full_words, 0u64)) {
        (Some(u), Some(p)) => (u, p),
        _ => {
            let cap_words = full_words.min(65536);
            (alloc_vec(cap_words, 0u64)?, alloc_vec(cap_words, 0u64)?)
        }
    };
    let mut scratch = [0u8; BLOCK];
    let mem = Memory {
        cache: &mut cache,
        slots: &mut slots,
        used: &mut used,
        pending: &mut pending,
        scratch: &mut scratch,
    };
    let dev = StoreSliceDev {
        disk,
        base_lba: store_first + w::BASE_LBA,
        blocks,
    };
    let mut fs = Fs::format(dev, mem).ok()?;
    let now = sex_pdx::civil_time_ms().map(|ms| ms / 1000).unwrap_or(0);
    if fs.mkdir(b"/bin", now).is_err() || fs.commit().is_err() {
        return None;
    }

    let mut chunk = alloc_vec(64 * 1024, 0u8)?;
    let reserve_blocks = (fs.statfs().total / 8).clamp(4096, 16384);
    let mut seeded_count = 0usize;
    let mut seeded_bytes = 0u64;

    for p in progs {
        let need_blocks = p.size.div_ceil(BLOCK as u64) + 16;
        if fs.statfs().free <= reserve_blocks + need_blocks {
            continue;
        }
        let mut path = [0u8; 32];
        let prefix = sex_pdx::program_store::BIN;
        path[..prefix.len()].copy_from_slice(prefix);
        path[prefix.len()..prefix.len() + p.name.len()].copy_from_slice(p.name.as_bytes());
        let full_path = &path[..prefix.len() + p.name.len()];
        let Ok(id) = fs.create(full_path, now) else {
            let _ = fs.rollback();
            continue;
        };
        let wrote = crate::volumes::stream_usb_program(p.mount_idx, p.name.as_bytes(), &mut chunk, |off, bytes| {
            let mut put = 0usize;
            while put < bytes.len() {
                match fs.write(id, off + put as u64, &bytes[put..], now) {
                    Ok(0) | Err(_) => return Err(()),
                    Ok(n) => put += n,
                }
            }
            Ok(())
        });
        match wrote {
            Ok(bytes) if fs.commit().is_ok() => {
                seeded_count += 1;
                seeded_bytes += bytes;
            }
            _ => {
                let _ = fs.rollback();
            }
        }
    }
    let _ = DiskFs::diskfs_block_sync();
    sex_pdx::serial_record!("[files.install.seed] programs={} bytes={}", seeded_count, seeded_bytes);
    (seeded_count > 0).then_some((seeded_count, seeded_bytes))
}
