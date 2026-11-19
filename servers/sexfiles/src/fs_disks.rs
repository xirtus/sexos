// SPDX-License-Identifier: GPL-3.0-only
//! Raw removable disks for a run the person lets reshape one (`sex_pdx::disks`): Corset.
//!
//! The disks are SexUSB's sticks and disks, and the disk images mounted under `/media`
//! (loop images); the system disk is listed so a program can show it, and is never
//! opened raw. A run asks for a disk by using it: its first DISK_IO raises a question
//! that names the disk's product and size, shown by the person's picker (the Files app),
//! as PICK's questions are. A yes is a grant for that run and that disk only. It ends at
//! DISK_DONE, when the run exits (its next sibling finds it stale) or when the disk is
//! pulled out.
//!
//! Before the first write on a granted disk, every volume Files mounted from it is
//! written out and unmounted and every grant into those volumes is taken back
//! (`volumes::release`, as eject does); DISK_DONE scans the disk again.
//!
//! Refused here, whatever was answered: any IO on the system disk, and writes to a disk
//! holding `/home`'s encrypted image or the partition this system booted from.
use crate::fs_service::Disk as FsDisk;
use crate::volumes;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_pdx::disks as d;
use sex_pdx::fs as w;
use sexfs::Fs;

/// One question to the person per program row.
#[derive(Clone)]
struct Ask { ticket: u32, disk: u16, text: String, answer: Answer }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer { Waiting, No }
/// One disk a run may use now.
#[derive(Clone, Copy)]
struct Session { disk: u16, sectors: u64, read_only: bool, released: bool }
/// A program row's run and what it was given.
struct Row { row: usize, target: u32, task: u32, sessions: Vec<Session>, ask: Option<Ask> }
static mut ROWS: Vec<Row> = Vec::new();
static mut TICKET: u32 = 0;
/// Images that were mounted (by file id and path), so they stay listed once unmounted.
const IMAGES_MAX: usize = 8;
static mut IMAGES: Vec<(u64, Vec<u8>)> = Vec::new();
fn rows() -> &'static mut Vec<Row> { unsafe { &mut *core::ptr::addr_of_mut!(ROWS) } }
fn images() -> &'static mut Vec<(u64, Vec<u8>)> { unsafe { &mut *core::ptr::addr_of_mut!(IMAGES) } }

/// A disk as DISKS lists it.
struct Info { id: u16, flags: u8, sectors: u64, name: String, product: String }

/// `volumes::mount` just mounted this image.
pub fn image_seen(path: &[u8], id: u64) {
    let list = images();
    list.retain(|(i, p)| *i != id && p.as_slice() != path);
    if list.len() >= IMAGES_MAX { list.remove(0); }
    list.push((id, path.to_vec()));
}
/// SexUSB's disks now: a run loses a disk that was pulled out.
pub fn usb_present(mask: u64) {
    for r in rows().iter_mut() {
        r.sessions.retain(|s| {
            let gone = s.disk & 0xFF00 == d::KIND_USB && mask & (1 << (s.disk & 0xFF)) == 0;
            if gone { sex_pdx::serial_record!("[files.disks.gone] row={} disk={:#x}", r.row, s.disk); }
            !gone
        });
    }
}
/// A run holds this USB disk raw: Files does not mount it meanwhile.
pub fn usb_busy(disk: u64) -> bool {
    rows().iter().any(|r| r.sessions.iter().any(|s| s.disk == d::KIND_USB | disk as u16 && s.released))
}

fn system_sectors() -> u64 { crate::backends::diskfs::DiskFs::diskfs_block_call(sex_pdx::BLOCK_CAPACITY, 0, 0, 0) }
fn image_path(id: u16) -> Option<(u64, Vec<u8>)> {
    if id & 0xFF00 != d::KIND_IMAGE { return None; }
    images().get((id & 0xFF) as usize).cloned()
}
fn system_image(path: &[u8]) -> bool { path == sex_pdx::home_lock::IMAGE }

/// What the boot loader named as where this system started (partition GUIDs at any
/// 4-byte offset; see `sex_pdx::fs::BOOT_FACTS_SLOT`).
fn boot_facts() -> Option<&'static [u8]> {
    use sex_pdx::memory_region::{QUERY_VIEW, READ};
    let view = sex_pdx::memory_region_view(w::BOOT_FACTS_SLOT, 0, 4096, QUERY_VIEW | READ).ok()?;
    Some(unsafe { core::slice::from_raw_parts(view.virtual_address as *const u8, 96) })
}
/// The system booted from a partition on this disk (a SexOS stick, say).
fn holds_boot(disk: &mut dyn sex_volumes::Disk) -> bool {
    struct Dev<'a>(&'a mut dyn sex_volumes::Disk);
    impl sex_disk::BlockDev for Dev<'_> {
        fn sectors(&self) -> u64 { self.0.sectors() }
        fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), sex_disk::Error> { self.0.read(lba, buf).map_err(|_| sex_disk::Error::Io) }
        fn write(&mut self, _: u64, _: &[u8]) -> Result<(), sex_disk::Error> { Err(sex_disk::Error::Io) }
    }
    let Some(facts) = boot_facts() else { return false };
    let Ok(gpt) = sex_disk::gpt::Gpt::read(&mut Dev(disk)) else { return false };
    let named = |g: sex_disk::gpt::Guid| !g.is_zero() && (0..=facts.len() - 16).step_by(4).any(|at| facts[at..at + 16] == g.0);
    gpt.entries.iter().filter(|e| e.is_used()).any(|e| named(e.part_guid()))
}

fn list(mut fs: Option<&mut Fs<'static, FsDisk>>) -> Vec<Info> {
    let mut out = Vec::new();
    let sys = system_sectors();
    if sys > 0 && sys != u64::MAX {
        out.push(Info { id: d::KIND_SYSTEM, flags: d::FLAG_SYSTEM, sectors: sys, name: "system".into(), product: "The disk SexOS runs from".into() });
    }
    for (disk, sectors, read_only, product) in volumes::usb_disks() {
        let system = holds_boot(&mut volumes::UsbDisk { disk, sectors });
        out.push(Info { id: d::KIND_USB | disk as u16, flags: (read_only as u8 * d::FLAG_READ_ONLY) | (system as u8 * d::FLAG_SYSTEM),
                        sectors, name: format!("usb{}", disk), product: product.trim().into() });
    }
    if let Some(fs) = fs.as_deref_mut() {
        for (k, (id, path)) in images().iter().enumerate() {
            let Ok(st) = fs.open(path) else { continue };
            if st.id != *id || st.size < 512 { continue; }
            let name = path.rsplit(|b| *b == b'/').next().unwrap_or(path);
            out.push(Info { id: d::KIND_IMAGE | k as u16, flags: system_image(path) as u8 * d::FLAG_SYSTEM, sectors: st.size / 512,
                            name: String::from_utf8_lossy(&name[..name.len().min(d::NAME_MAX)]).into_owned(),
                            product: format!("Disk image {}", String::from_utf8_lossy(path)) });
        }
    }
    out
}

/// Ends every session of runs that are gone: their disks are scanned again.
fn tidy(mut fs: Option<&mut Fs<'static, FsDisk>>) {
    let mut stale = Vec::new();
    rows().retain(|r| {
        if crate::fs_service::live_run(r.target, r.task) { return true; }
        stale.extend(r.sessions.iter().copied().map(|s| (r.row, s)));
        false
    });
    for (row, s) in stale {
        sex_pdx::serial_record!("[files.disks.ended] row={} disk={:#x} why=exited", row, s.disk);
        finish(fs.as_deref_mut(), s);
    }
}
fn finish(fs: Option<&mut Fs<'static, FsDisk>>, s: Session) {
    if s.disk & 0xFF00 == d::KIND_USB {
        let disk = (s.disk & 0xFF) as u64;
        if !s.read_only { let _ = volumes::usb_call(sex_pdx::usb_disk::OP_SYNC, disk, 0, 0); }
        if s.released { volumes::usb_rescan(disk); }
    } else if let (Some(fs), Some((_, path))) = (fs, image_path(s.disk)) {
        if s.released {
            if crate::fs_service::writes_pending() { crate::fs_service::commit_now(fs); }
            let mut text = [0u8; 512];
            let _ = volumes::mount(fs, &path, &mut text);
        }
    }
}

/// One request of `sex_pdx::disks`. `run`: the program row and its task (None for Spindle);
/// `whole`: the requester holds the whole filesystem and may answer questions.
pub fn handle(mut fs: Option<&mut Fs<'static, FsDisk>>, run: Option<(usize, u32, u32)>, whole: bool, program_name: &dyn Fn(usize) -> Vec<u8>,
              r: w::Request, data: &mut [u8]) -> (u8, u32) {
    tidy(fs.as_deref_mut());
    match r.kind {
        d::DISKS => {
            let mine = run.and_then(|(row, _, task)| rows().iter().find(|x| x.row == row && x.task == task));
            let mut used = 0;
            for i in list(fs) {
                let granted = mine.is_some_and(|m| m.sessions.iter().any(|s| s.disk == i.id));
                let e = d::Entry { id: i.id, flags: i.flags | granted as u8 * d::FLAG_GRANTED, sectors: i.sectors, name: i.name.as_bytes(), product: i.product.as_bytes() };
                match e.encode(data, used) { Some(end) => used = end, None => break }
            }
            (w::OK, used as u32)
        }
        d::DISK_IO => io(fs, run, r, data),
        d::DISK_DONE => {
            let Some((row, _, task)) = run else { return (w::DENIED, 0) };
            let Some(x) = rows().iter_mut().find(|x| x.row == row && x.task == task) else { return (w::NOT_FOUND, 0) };
            let Some(at) = x.sessions.iter().position(|s| s.disk as u64 == r.length) else { return (w::NOT_FOUND, 0) };
            let s = x.sessions.remove(at);
            finish(fs, s);
            sex_pdx::serial_record!("[files.disks.done] row={} disk={:#x} rescanned={}", row, s.disk, s.released as u8);
            (w::OK, 0)
        }
        d::DISK_ASKS => {
            if !whole { return (w::DENIED, 0); }
            let mut used = 0;
            for x in rows().iter() {
                let Some(a) = x.ask.as_ref().filter(|a| a.answer == Answer::Waiting) else { continue };
                let name = program_name(x.row);
                let (name, text) = (&name[..name.len().min(64)], &a.text.as_bytes()[..a.text.len().min(200)]);
                if used + 7 + name.len() + text.len() > 4000 { break; }
                data[used] = x.row as u8;
                data[used + 1..used + 5].copy_from_slice(&a.ticket.to_le_bytes());
                data[used + 5] = name.len() as u8;
                data[used + 6..used + 6 + name.len()].copy_from_slice(name);
                used += 6 + name.len();
                data[used] = text.len() as u8;
                data[used + 1..used + 1 + text.len()].copy_from_slice(text);
                used += 1 + text.len();
            }
            (w::OK, used as u32)
        }
        d::DISK_ANSWER => {
            if !whole { return (w::DENIED, 0); }
            let (row, ticket, yes) = (r.offset as usize, (r.length & 0xFFFF_FFFF) as u32, r.length & d::YES != 0);
            let Some(x) = rows().iter_mut().find(|x| x.row == row) else { return (w::NOT_FOUND, 0) };
            let Some(a) = x.ask.as_mut().filter(|a| a.ticket == ticket && a.answer == Answer::Waiting) else { return (w::NOT_FOUND, 0) };
            if !yes {
                a.answer = Answer::No;
                sex_pdx::serial_record!("[files.disks.refused] row={} disk={:#x} why=person", row, a.disk);
                return (w::OK, 0);
            }
            let disk = a.disk;
            x.ask = None;
            // The disk as it is now: it may have gone, or changed, since the question.
            let Some(i) = list(fs).into_iter().find(|i| i.id == disk) else { return (w::NOT_FOUND, 0) };
            if i.flags & d::FLAG_SYSTEM != 0 { return (w::DENIED, 0); }
            x.sessions.push(Session { disk, sectors: i.sectors, read_only: i.flags & d::FLAG_READ_ONLY != 0, released: false });
            sex_pdx::serial_record!("[files.disks.granted] row={} task={} disk={:#x} name={} product={} sectors={}", row, x.task, disk, i.name, i.product, i.sectors);
            (w::OK, 0)
        }
        _ => (w::INVALID, 0),
    }
}

fn io(mut fs: Option<&mut Fs<'static, FsDisk>>, run: Option<(usize, u32, u32)>, r: w::Request, data: &mut [u8]) -> (u8, u32) {
    let Some((disk, write, sectors)) = d::decode_io(r.length) else { return (w::INVALID, 0) };
    // The system disk is never opened raw, whoever asks and whatever was answered.
    if disk == d::KIND_SYSTEM {
        sex_pdx::serial_record!("[files.disks.refused] disk=system write={} lba={} why=system", write as u8, r.offset);
        return (w::DENIED, 0);
    }
    let Some((row, target, task)) = run else { return (w::DENIED, 0) };
    if !rows().iter().any(|x| x.row == row) { rows().push(Row { row, target, task, sessions: Vec::new(), ask: None }); }
    let x = rows().iter_mut().find(|x| x.row == row).unwrap();
    if x.task != task { *x = Row { row, target, task, sessions: Vec::new(), ask: None }; }
    let Some(s) = x.sessions.iter_mut().find(|s| s.disk == disk) else {
        // Not given yet: ask the person, naming the disk; a no is said once.
        match &x.ask {
            Some(a) if a.disk == disk && a.answer == Answer::Waiting => return (w::PENDING, 0),
            Some(a) if a.disk == disk => { x.ask = None; return (w::DENIED, 0); }
            _ => {}
        }
        let Some(i) = list(fs.as_deref_mut()).into_iter().find(|i| i.id == disk) else { return (w::NOT_FOUND, 0) };
        if i.flags & d::FLAG_SYSTEM != 0 {
            sex_pdx::serial_record!("[files.disks.refused] row={} disk={:#x} write={} why=system", row, disk, write as u8);
            return (w::DENIED, 0);
        }
        let ticket = unsafe { TICKET = TICKET % 0xFFFF_FFFE + 1; TICKET };
        let text = format!("{} ({}, {}{})", i.name, if i.product.is_empty() { "unnamed disk" } else { i.product.trim() }, size_text(i.sectors * 512),
                           if i.flags & d::FLAG_READ_ONLY != 0 { ", write protected" } else { "" });
        sex_pdx::serial_record!("[files.disks.asked] row={} disk={:#x} ticket={} text={}", row, disk, ticket, text);
        x.ask = Some(Ask { ticket, disk, text, answer: Answer::Waiting });
        return (w::PENDING, 0);
    };
    // A write of no sectors: what was written is on the medium when this answers.
    if write && sectors == 0 {
        if disk & 0xFF00 == d::KIND_USB && volumes::usb_call(sex_pdx::usb_disk::OP_SYNC, (disk & 0xFF) as u64, 0, 0) != Some(sex_pdx::usb_disk::OK) { return (w::IO, 0); }
        return (w::OK, 0);
    }
    // Every range inside the disk, and inside the client's data area.
    let bytes = sectors as usize * d::SECTOR;
    if sectors == 0 || sectors as u64 > d::IO_SECTORS || bytes > data.len() || r.offset.checked_add(sectors as u64).is_none_or(|end| end > s.sectors) {
        sex_pdx::serial_record!("[files.disks.refused] row={} disk={:#x} lba={} sectors={} why=bounds", row, disk, r.offset, sectors);
        return (w::INVALID, 0);
    }
    if write {
        if s.read_only { return (w::READ_ONLY, 0); }
        if !s.released {
            // Nothing Files holds open on the disk may be written over: out with it all.
            let (usb, image) = if disk & 0xFF00 == d::KIND_USB { (Some((disk & 0xFF) as u64), None) } else { (None, image_path(disk).map(|p| p.0)) };
            match volumes::release(fs.as_deref_mut(), usb, image) {
                Ok(n) => sex_pdx::serial_record!("[files.disks.released] row={} disk={:#x} volumes={}", row, disk, n),
                Err(st) => { sex_pdx::serial_record!("[files.disks.refused] row={} disk={:#x} why=busy status={}", row, disk, st); return (st, 0); }
            }
            s.released = true;
        }
    }
    let s = *s;
    let result = if disk & 0xFF00 == d::KIND_USB {
        let mut dev = volumes::UsbDisk { disk: (disk & 0xFF) as u64, sectors: s.sectors };
        transfer(&mut dev, write, r.offset, &mut data[..bytes])
    } else if let (Some(fs), Some((id, _))) = (fs, image_path(disk)) {
        let mut dev = volumes::ImageDisk { fs, id, sectors: s.sectors };
        transfer(&mut dev, write, r.offset, &mut data[..bytes])
    } else { Err(w::NOT_FOUND) };
    match result {
        Ok(()) => {
            if write { sex_pdx::serial_record!("[files.disks.write] disk={:#x} lba={} sectors={}", disk, r.offset, sectors); }
            (w::OK, sectors)
        }
        Err(st) => { sex_pdx::serial_record!("[files.disks.io.failed] disk={:#x} write={} lba={} sectors={} status={}", disk, write as u8, r.offset, sectors, st); (st, 0) }
    }
}
/// The client may change its region at any time: a write is copied before it goes.
fn transfer(dev: &mut dyn sex_volumes::Disk, write: bool, lba: u64, data: &mut [u8]) -> Result<(), u8> {
    if write {
        let own = data.to_vec();
        dev.write(lba, &own).map_err(volumes::status)
    } else {
        let mut own = vec![0u8; data.len()];
        dev.read(lba, &mut own).map_err(volumes::status)?;
        data.copy_from_slice(&own);
        Ok(())
    }
}
fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut u = 0;
    let mut v = bytes;
    while v >= 1024 * 10 && u + 1 < UNITS.len() { v /= 1024; u += 1; }
    if u == 0 && v >= 1024 { return format!("{} KiB", v / 1024); }
    format!("{} {}", v, UNITS[u])
}
