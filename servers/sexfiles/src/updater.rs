//! `update`: signed system updates into the slot that is not running, tried once
//! (see `sex_disk::update`). Files holds the disk, so it checks the signature, writes the
//! slot and keeps the boot menu's trial state: started when Files starts, passed when the
//! desktop's terminal first asks it for something.
use crate::backends::diskfs::DiskFs;
use crate::installer::{why, FileDev, RawDisk};
use alloc::format;
use alloc::string::String;
use sex_disk::gpt::Guid;
use sex_disk::update::{self, restate, State, System};
use sex_disk::{Error, SECTOR};
use sex_pdx::fs as w;
use sex_pdx::memory_region::{QUERY_VIEW, READ};
use sexfs::{Fs, Kind};

/// The release key this system was built with, if any.
fn release_key() -> Option<[u8; 32]> { update::unhex::<32>(env!("SEXOS_RELEASE_KEY_HEX").as_bytes()) }

/// What the boot loader said about where the kernel came from: the GUID it named, and the
/// raw record to search when its layout is not the one expected.
fn boot_facts() -> Option<&'static [u8]> {
    let view = sex_pdx::memory_region_view(w::BOOT_FACTS_SLOT, 0, 4096, QUERY_VIEW | READ).ok()?;
    Some(unsafe { core::slice::from_raw_parts(view.virtual_address as *const u8, 96) })
}
/// The system slot the running kernel came from, if it came from one.
fn booted_slot(sys: &System) -> Option<update::Slot> {
    let facts = boot_facts()?;
    (0..=facts.len() - 16).step_by(4).find_map(|at| {
        let g = Guid(facts[at..at + 16].try_into().ok()?);
        if g.is_zero() { None } else { sys.slot_of(g) }
    })
}

/// The installed system this one started from, its slot, and its menu state.
fn installed() -> Option<(RawDisk, System, update::Slot, State)> {
    let layout = DiskFs::diskfs_block_call(sex_pdx::BLOCK_LAYOUT, 0, 0, 0);
    if layout & 0xFF != sex_pdx::BLOCK_LAYOUT_STORE { return None; }
    let mut disk = RawDisk { sectors: layout >> 8 };
    let found = System::find(&mut disk);
    let slot = found.as_ref().ok().and_then(booted_slot);
    let st = found.as_ref().ok().map(|sys| sys.read_menu(&mut disk).and_then(|m| State::parse(&m)));
    let has_system = found.is_ok();
    let (Ok(sys), Some(slot), Some(Ok(st))) = (found, slot, st) else {
        sex_pdx::serial_record!("[files.update.installed] none system={} slot={:?}", has_system as u8, slot);
        return None;
    };
    Some((disk, sys, slot, st))
}

fn step(next: fn(&State, update::Slot) -> Option<State>, what: &str) {
    let Some((mut disk, sys, slot, st)) = installed() else { return };
    let Some(new) = next(&st, slot) else {
        sex_pdx::serial_record!("[files.update.{}] slot={:?} trial={:?} version={} unchanged=1", what, slot, st.trial, st.version);
        return;
    };
    let ok = sys.read_menu(&mut disk).and_then(|m| restate(&m, &new)).and_then(|m| sys.write_menu(&mut disk, &m)).is_ok()
        && DiskFs::diskfs_block_sync() == 0;
    sex_pdx::serial_record!("[files.update.{}] slot={:?} trial={:?} version={} default_entry={} ok={}", what, slot, new.trial, new.version, new.default_entry(), ok as u8);
}

/// Files has started in the system slot `booted` names.
pub fn on_start() { step(State::on_start, "start"); }

/// The desktop works: once per run.
pub fn on_healthy() {
    static mut DONE: bool = false;
    if unsafe { DONE } { return; }
    unsafe { DONE = true; }
    step(State::on_healthy, "healthy");
}

fn read_file(fs: &mut Fs<'static, crate::fs_service::Disk>, path: &[u8], max: usize) -> Result<alloc::vec::Vec<u8>, sexfs::Error> {
    let st = fs.open(path)?;
    if st.kind == Kind::Dir { return Err(sexfs::Error::IsDir); }
    if st.size as usize > max { return Err(sexfs::Error::TooBig); }
    let mut buf = alloc::vec![0u8; st.size as usize];
    let n = fs.read(st.id, 0, &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Checks an update, and with an image writes it. `paths` holds the index, signature and
/// image file paths, separated by NUL; without the image it only says whether the index
/// is good and newer. Writes a report into `out`; returns the reply status (OK: go on,
/// EXISTS: nothing newer, DENIED: refused) and the report's length.
pub fn run(fs: &mut Fs<'static, crate::fs_service::Disk>, paths: &[u8], out: &mut [u8]) -> Result<(u8, u32), sexfs::Error> {
    let mut text = String::new();
    let finish = |status: u8, text: String, out: &mut [u8]| { let n = text.len().min(out.len()); out[..n].copy_from_slice(&text.as_bytes()[..n]); Ok((status, n as u32)) };
    let mut parts = paths.split(|b| *b == 0);
    let (Some(index_path), Some(sig_path)) = (parts.next(), parts.next()) else { return Err(sexfs::Error::Name) };
    let image_path = parts.next().unwrap_or(&[]);
    let Some(key) = release_key() else {
        text.push_str("Not updated. This build has no release key, so it takes no updates.\n");
        return finish(w::DENIED, text, out);
    };
    let index = read_file(fs, index_path, 4096)?;
    let sig = read_file(fs, sig_path, 512)?;
    let index = match update::verify_index(&index, &sig, &key) {
        Ok(i) => i,
        Err(e) => {
            sex_pdx::serial_record!("[files.update.refused] error={:?}", e);
            text.push_str("Not updated. "); text.push_str(why(e)); text.push('\n');
            return finish(w::DENIED, text, out);
        }
    };
    let Some((mut disk, sys, slot, st)) = installed() else {
        text.push_str("Not updated. Updates go to an installed SexOS; this one started from somewhere else (a USB stick or CD).\n");
        return finish(w::DENIED, text, out);
    };
    if index.version <= st.version {
        sex_pdx::serial_record!("[files.update.current] version={} offered={}", st.version, index.version);
        text.push_str(&format!("Up to date: the newest release is version {}, and this system is version {}.\n", index.version, st.version));
        return finish(w::EXISTS, text, out);
    }
    if image_path.is_empty() {
        sex_pdx::serial_record!("[files.update.available] version={} from={} bytes={}", index.version, st.version, index.bytes);
        text.push_str(&format!("Version {} is available (this is version {}): {} MiB to download.\n", index.version, st.version, index.bytes >> 20));
        return finish(w::OK, text, out);
    }
    let image = fs.open(image_path)?;
    if image.kind == Kind::Dir { return Err(sexfs::Error::IsDir); }
    sex_pdx::serial_record!("[files.update.plan] running={:?} target={:?} version={} from={} bytes={}", slot, slot.other(), index.version, st.version, index.bytes);
    let mut progress = |s: &str| sex_pdx::serial_record!("[files.update.step] {}", s);
    let booted = sys.slot(slot).1;
    let result = update::apply(&mut disk, &sys, booted, &index, &mut FileDev { fs, id: image.id, sectors: image.size / SECTOR as u64 }, &mut progress)
        .and_then(|s| if DiskFs::diskfs_block_sync() == 0 { Ok(s) } else { Err(Error::Io) });
    match result {
        Ok(target) => {
            sex_pdx::serial_record!("[files.update.done] slot={:?} version={}", target, index.version);
            text.push_str(&format!("Version {} is in system slot {:?}. Restart to try it: if it does not reach the desktop, the next start goes back to this system.\n", index.version, target));
            finish(w::OK, text, out)
        }
        Err(e) => {
            sex_pdx::serial_record!("[files.update.failed] error={:?}", e);
            text.push_str("Not updated. "); text.push_str(why(e));
            text.push_str(" The boot menu was not changed, so this system still starts.\n");
            finish(w::DENIED, text, out)
        }
    }
}
