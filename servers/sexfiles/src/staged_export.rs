// SPDX-License-Identifier: GPL-3.0-only
//! Files owns this staging namespace. A run receives a ticket, never a stage path.
//! The only visible destination transition is SexFS's checked COW move and commit.
use crate::fs_service;
use alloc::vec::Vec;
use sex_pdx::fs as w;
use sexfs::{Device, Error, Fs, Kind, NAME_MAX};
use sha2::{Digest, Sha256};

const ROOT: &[u8] = b"/.__sexfiles_staged";
const MARK: &[u8] = b"/.__sexfiles_staged/.owner";
const MAGIC: &[u8] = b"SexOS Files staged export v1";
static mut NEXT: u64 = 0;
static mut ACTIVE: [Option<Stage>; 2] = [const { None }; 2];
static mut CLEAN: [bool; 2] = [false; 2];

#[derive(Clone, Copy)]
pub enum Volume {
    Primary,
    Home,
}
impl Volume {
    const fn index(self) -> usize {
        match self {
            Self::Primary => 0,
            Self::Home => 1,
        }
    }
}

struct Stage {
    row: usize,
    target: u32,
    task: u32,
    ticket: u64,
    id: u64,
    source: Vec<u8>,
    destination: Vec<u8>,
    expected_size: u64,
    expected_hash: [u8; 32],
    written: u64,
    hash: Sha256,
}

pub fn reserved(path: &[u8]) -> bool {
    let mut parts = path.split(|b| *b == b'/').filter(|part| !part.is_empty());
    let first = parts.next();
    first == Some(b".__sexfiles_staged".as_slice())
        || first == Some(b"home".as_slice())
            && parts.next() == Some(b".__sexfiles_staged".as_slice())
}
pub fn sealed_image_alias(path: &[u8]) -> bool {
    let mut parts = path.split(|b| *b == b'/').filter(|p| !p.is_empty());
    parts.next() == sex_pdx::home_lock::IMAGE.strip_prefix(b"/") && parts.next().is_none()
}
pub fn visible_entry(at_volume_root: bool, name: &[u8]) -> bool {
    !at_volume_root || name != b".__sexfiles_staged"
}

fn code(e: Error) -> u8 {
    fs_service::status(e)
}
fn now() -> u64 {
    #[cfg(test)]
    {
        return 0;
    }
    #[cfg(not(test))]
    sex_pdx::civil_time_ms().map(|ms| ms / 1000).unwrap_or(0)
}
fn ticks() -> u64 {
    #[cfg(test)]
    {
        return 1;
    }
    #[cfg(not(test))]
    sex_pdx::get_ticks()
}

fn named(ticket: u64) -> Vec<u8> {
    let mut path = ROOT.to_vec();
    path.extend_from_slice(b"/s");
    for shift in (0..16).rev() {
        let n = ((ticket >> (shift * 4)) & 15) as u8;
        path.push(if n < 10 { b'0' + n } else { b'a' + n - 10 });
    }
    path
}
/// At mount, discard only our own incomplete files, before a new ticket is issued.
/// If the marker is missing or cleanup fails, staged saves stay disabled while
/// ordinary files remain usable. The namespace remains inaccessible to callers.
pub fn recover<D: Device>(fs: &mut Fs<'static, D>, volume: Volume) {
    let slot = volume.index();
    unsafe {
        CLEAN[slot] = false;
        ACTIVE[slot] = None;
    }
    let root = match fs.open(ROOT) {
        Ok(s) => s,
        Err(Error::NotFound) => {
            unsafe {
                CLEAN[slot] = true;
            }
            return;
        }
        Err(_) => return,
    };
    if root.kind != Kind::Dir {
        return;
    }
    let Ok(mark) = fs.open(MARK) else {
        return;
    };
    if mark.size != MAGIC.len() as u64 {
        return;
    }
    let mut buf = [0u8; 32];
    if fs.read(mark.id, 0, &mut buf[..MAGIC.len()]) != Ok(MAGIC.len())
        || &buf[..MAGIC.len()] != MAGIC
    {
        return;
    }
    loop {
        let mut name = [0u8; NAME_MAX];
        let entry = match fs.read_dir(root.id, 0, &mut name) {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(_) => return,
        };
        let (len, kind, _, _) = entry;
        if &name[..len] == b".owner" {
            // Skip the marker with a cursor; all other names must be stage files.
            let entry = match fs.read_dir(root.id, entry.3, &mut name) {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(_) => return,
            };
            if entry.0 != 17
                || entry.1 != Kind::File
                || name[0] != b's'
                || !name[1..entry.0].iter().all(u8::is_ascii_hexdigit)
            {
                return;
            }
            let mut path = ROOT.to_vec();
            path.push(b'/');
            path.extend_from_slice(&name[..entry.0]);
            if fs.remove_path(&path).and_then(|_| fs.commit()).is_err() {
                let _ = fs.rollback();
                return;
            }
        } else {
            if len != 17
                || name[0] != b's'
                || kind != Kind::File
                || !name[1..len].iter().all(u8::is_ascii_hexdigit)
            {
                return;
            }
            let mut path = ROOT.to_vec();
            path.push(b'/');
            path.extend_from_slice(&name[..len]);
            if fs.remove_path(&path).and_then(|_| fs.commit()).is_err() {
                let _ = fs.rollback();
                return;
            }
        }
    }
    unsafe {
        CLEAN[slot] = true;
    }
}

pub fn invalidate(volume: Volume) {
    let slot = volume.index();
    unsafe {
        CLEAN[slot] = false;
        ACTIVE[slot] = None;
    }
}
fn ensure_root<D: Device>(fs: &mut Fs<'static, D>) -> Result<(), u8> {
    if fs.open(ROOT).is_ok() {
        return Ok(());
    }
    // Folder and marker appear in one commit, including after a power cut.
    let made = (|| {
        fs.mkdir(ROOT, now())?;
        let marker = fs.create(MARK, now())?;
        let n = fs.write(marker, 0, MAGIC, now())?;
        if n != MAGIC.len() {
            return Err(Error::NoSpace);
        }
        fs.commit()
    })();
    if let Err(e) = made {
        let _ = fs.rollback();
        return Err(code(e));
    }
    Ok(())
}
fn ticket_stage(
    volume: Volume,
    row: usize,
    task: u32,
    token: u64,
    destination: &[u8],
) -> Result<&'static mut Stage, u8> {
    let stage = unsafe { &mut (*core::ptr::addr_of_mut!(ACTIVE))[volume.index()] };
    let s = stage.as_mut().ok_or(w::STALE)?;
    if s.row != row || s.task != task || s.ticket != token || s.destination != destination {
        return Err(w::STALE);
    }
    Ok(s)
}

pub fn request<D: Device>(
    fs: &mut Fs<'static, D>,
    volume: Volume,
    row: usize,
    target: u32,
    task: u32,
    r: w::Request,
    destination: &[u8],
    data: &mut [u8],
) -> Result<u32, u8> {
    let slot = volume.index();
    if !unsafe { CLEAN[slot] } {
        return Err(w::UNAVAILABLE);
    }
    if reserved(destination) || destination.is_empty() || destination == b"/" {
        return Err(w::DENIED);
    }
    match r.kind {
        w::STAGE_BEGIN => {
            if r.length != 32 || r.offset > (4 << 20) {
                return Err(w::INVALID);
            }
            if let Some(s) = unsafe { &(*core::ptr::addr_of!(ACTIVE))[slot] } {
                if fs_service::live_run(s.target, s.task) {
                    return Err(w::PENDING);
                }
                // The old run ended; only its original object may be discarded.
                fs.remove_path_if_id(&s.source, s.id)
                    .and_then(|_| fs.commit())
                    .map_err(code)?;
                unsafe {
                    ACTIVE[slot] = None;
                }
            }
            match fs.open(destination) {
                Ok(_) => return Err(w::EXISTS),
                Err(Error::NotFound) => {}
                Err(e) => return Err(code(e)),
            }
            ensure_root(fs)?;
            let ticket = unsafe {
                NEXT = NEXT.wrapping_add(1).max(1);
                NEXT ^ ticks().rotate_left(17)
            };
            let source = named(ticket);
            let id = fs
                .create(&source, now())
                .and_then(|id| {
                    fs.commit()?;
                    Ok(id)
                })
                .map_err(code)?;
            let expected_hash: [u8; 32] = data[..32].try_into().unwrap();
            unsafe {
                ACTIVE[slot] = Some(Stage {
                    row,
                    target,
                    task,
                    ticket,
                    id,
                    source,
                    destination: destination.to_vec(),
                    expected_size: r.offset,
                    expected_hash,
                    written: 0,
                    hash: Sha256::new(),
                });
            }
            data[..8].copy_from_slice(&ticket.to_le_bytes());
            Ok(8)
        }
        w::STAGE_CHUNK => {
            let n = r.length as usize;
            if n < 8 || n > data.len() {
                return Err(w::INVALID);
            }
            let token = u64::from_le_bytes(data[..8].try_into().unwrap());
            let s = ticket_stage(volume, row, task, token, destination)?;
            let bytes = fs_service::staged_private_chunk(&data[8..n]).ok_or(w::INVALID)?;
            if r.offset != s.written
                || s.written
                    .checked_add(bytes.len() as u64)
                    .is_none_or(|end| end > s.expected_size)
            {
                return Err(w::INVALID);
            }
            let wrote = fs
                .write_path_if_id(&s.source, s.id, r.offset, bytes, now())
                .map_err(code)?;
            if wrote != bytes.len() {
                let _ = fs.rollback();
                return Err(w::NO_SPACE);
            }
            fs.commit().map_err(code)?;
            s.written += wrote as u64;
            s.hash.update(bytes);
            Ok(wrote as u32)
        }
        w::STAGE_PUBLISH => {
            if r.length != 0 {
                return Err(w::INVALID);
            }
            let s = ticket_stage(volume, row, task, r.offset, destination)?;
            let computed: [u8; 32] = s.hash.clone().finalize().into();
            if s.written != s.expected_size || computed != s.expected_hash {
                return Err(w::INVALID);
            }
            let stat = fs.open(&s.source).map_err(code)?;
            if stat.id != s.id || stat.size != s.expected_size {
                return Err(w::STALE);
            }
            fs.move_file_if_id(&s.source, destination, s.id)
                .map_err(code)?;
            if let Err(e) = fs.commit() {
                let _ = fs.rollback();
                return Err(code(e));
            }
            unsafe {
                ACTIVE[slot] = None;
            }
            Ok(0)
        }
        w::STAGE_ABORT => {
            if r.length != 0 {
                return Err(w::INVALID);
            }
            let s = ticket_stage(volume, row, task, r.offset, destination)?;
            fs.remove_path_if_id(&s.source, s.id)
                .and_then(|_| fs.commit())
                .map_err(code)?;
            unsafe {
                ACTIVE[slot] = None;
            }
            Ok(0)
        }
        _ => Err(w::INVALID),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use sexfs::{Device, Memory, Slot, BLOCK};
    use std::{boxed::Box, cell::RefCell, rc::Rc, sync::Mutex, vec, vec::Vec};
    static LOCK: Mutex<()> = Mutex::new(());

    #[derive(Clone)]
    struct Disk(Rc<RefCell<Vec<[u8; BLOCK]>>>);
    impl Device for Disk {
        fn blocks(&self) -> u64 {
            self.0.borrow().len() as u64
        }
        fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> {
            *out = self.0.borrow()[block as usize];
            Ok(())
        }
        fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<(), ()> {
            self.0.borrow_mut()[block as usize] = *data;
            Ok(())
        }
        fn flush(&mut self) -> Result<(), ()> {
            Ok(())
        }
    }
    fn memory() -> Memory<'static> {
        Memory {
            cache: Box::leak(vec![[0; BLOCK]; 24].into_boxed_slice()),
            slots: Box::leak(vec![Slot::EMPTY; 24].into_boxed_slice()),
            used: Box::leak(vec![0; 32].into_boxed_slice()),
            pending: Box::leak(vec![0; 32].into_boxed_slice()),
            scratch: Box::leak(Box::new([0; BLOCK])),
        }
    }
    fn req(kind: u8, offset: u64, length: u64) -> w::Request {
        w::Request {
            seq: 1,
            kind,
            path_len: 0,
            offset,
            length,
        }
    }
    fn recover(fs: &mut Fs<'static, Disk>) {
        super::recover(fs, Volume::Primary);
    }
    fn request(
        fs: &mut Fs<'static, Disk>,
        row: usize,
        target: u32,
        task: u32,
        r: w::Request,
        destination: &[u8],
        data: &mut [u8],
    ) -> Result<u32, u8> {
        super::request(fs, Volume::Primary, row, target, task, r, destination, data)
    }

    #[test]
    fn ticket_checks_and_mount_scavenging() {
        let _lock = LOCK.lock().unwrap();
        let disk = Disk(Rc::new(RefCell::new(vec![[0; BLOCK]; 1024])));
        let mut fs = Fs::format(disk.clone(), memory()).unwrap();
        recover(&mut fs);
        let payload = b"original";
        let hash: [u8; 32] = Sha256::digest(payload).into();
        let mut data = [0u8; 64];
        data[..32].copy_from_slice(&hash);
        assert_eq!(
            request(
                &mut fs,
                1,
                41,
                7,
                req(w::STAGE_BEGIN, 8, 32),
                b"/chosen",
                &mut data
            ),
            Ok(8)
        );
        let ticket = u64::from_le_bytes(data[..8].try_into().unwrap());
        data[..8].copy_from_slice(&ticket.to_le_bytes());
        data[8..16].copy_from_slice(payload);
        assert_eq!(
            request(
                &mut fs,
                2,
                41,
                7,
                req(w::STAGE_CHUNK, 0, 16),
                b"/chosen",
                &mut data
            ),
            Err(w::STALE)
        );
        assert_eq!(
            request(
                &mut fs,
                1,
                41,
                8,
                req(w::STAGE_CHUNK, 0, 16),
                b"/chosen",
                &mut data
            ),
            Err(w::STALE)
        );
        assert_eq!(
            request(
                &mut fs,
                1,
                41,
                7,
                req(w::STAGE_CHUNK, 0, 16),
                b"/other",
                &mut data
            ),
            Err(w::STALE)
        );
        assert_eq!(
            request(
                &mut fs,
                1,
                41,
                7,
                req(w::STAGE_PUBLISH, ticket, 0),
                b"/chosen",
                &mut data
            ),
            Err(w::INVALID)
        );
        assert_eq!(
            request(
                &mut fs,
                1,
                41,
                7,
                req(w::STAGE_CHUNK, 0, 16),
                b"/chosen",
                &mut data
            ),
            Ok(8)
        );
        assert_eq!(
            request(
                &mut fs,
                1,
                41,
                7,
                req(w::STAGE_PUBLISH, ticket, 0),
                b"/chosen",
                &mut data
            ),
            Ok(0)
        );
        let chosen = fs.open(b"/chosen").unwrap();
        let mut got = [0; 8];
        assert_eq!(fs.read(chosen.id, 0, &mut got), Ok(8));
        assert_eq!(&got, payload);
        data[..32].copy_from_slice(&hash);
        assert_eq!(
            request(
                &mut fs,
                1,
                41,
                7,
                req(w::STAGE_BEGIN, 8, 32),
                b"/chosen",
                &mut data
            ),
            Err(w::EXISTS)
        );
        assert_eq!(
            request(
                &mut fs,
                1,
                41,
                7,
                req(w::STAGE_BEGIN, 8, 32),
                b"/later",
                &mut data
            ),
            Ok(8)
        );
        let abandoned = unsafe {
            (*core::ptr::addr_of!(ACTIVE))[0]
                .as_ref()
                .unwrap()
                .source
                .clone()
        };
        assert!(fs.open(&abandoned).is_ok());
        drop(fs);
        let mut remounted = Fs::mount(disk, memory()).unwrap();
        recover(&mut remounted);
        assert_eq!(remounted.open(b"/later"), Err(Error::NotFound));
        assert_eq!(remounted.open(&abandoned), Err(Error::NotFound));
        assert_eq!(remounted.open(b"/chosen").unwrap().id, chosen.id);
    }

    #[test]
    fn home_lock_invalidates_ticket_until_unlock_recovery() {
        let _lock = LOCK.lock().unwrap();
        let disk = Disk(Rc::new(RefCell::new(vec![[0; BLOCK]; 1024])));
        let mut fs = Fs::format(disk, memory()).unwrap();
        super::recover(&mut fs, Volume::Home);
        let mut data = [0u8; 64];
        let hash: [u8; 32] = Sha256::digest(b"complete").into();
        data[..32].copy_from_slice(&hash);
        assert_eq!(
            super::request(
                &mut fs,
                Volume::Home,
                2,
                41,
                9,
                req(w::STAGE_BEGIN, 8, 32),
                b"/chosen",
                &mut data
            ),
            Ok(8)
        );
        let ticket = u64::from_le_bytes(data[..8].try_into().unwrap());
        let abandoned = unsafe {
            (*core::ptr::addr_of!(ACTIVE))[1]
                .as_ref()
                .unwrap()
                .source
                .clone()
        };
        invalidate(Volume::Home);
        data[..8].copy_from_slice(&ticket.to_le_bytes());
        data[8..16].copy_from_slice(b"complete");
        assert_eq!(
            super::request(
                &mut fs,
                Volume::Home,
                2,
                41,
                9,
                req(w::STAGE_CHUNK, 0, 16),
                b"/chosen",
                &mut data
            ),
            Err(w::UNAVAILABLE)
        );
        super::recover(&mut fs, Volume::Home);
        assert_eq!(fs.open(&abandoned), Err(Error::NotFound));
        assert_eq!(fs.open(b"/chosen"), Err(Error::NotFound));
    }

    #[test]
    fn publish_rejects_complete_but_wrong_bytes_and_abort_cleans_stage() {
        let _lock = LOCK.lock().unwrap();
        let disk = Disk(Rc::new(RefCell::new(vec![[0; BLOCK]; 1024])));
        let mut fs = Fs::format(disk, memory()).unwrap();
        super::recover(&mut fs, Volume::Primary);
        let mut data = [0u8; 64];
        let expected: [u8; 32] = Sha256::digest(b"original").into();
        data[..32].copy_from_slice(&expected);
        assert_eq!(
            super::request(
                &mut fs,
                Volume::Primary,
                1,
                41,
                7,
                req(w::STAGE_BEGIN, 8, 32),
                b"/chosen",
                &mut data
            ),
            Ok(8)
        );
        let ticket = u64::from_le_bytes(data[..8].try_into().unwrap());
        let source = unsafe {
            (*core::ptr::addr_of!(ACTIVE))[0]
                .as_ref()
                .unwrap()
                .source
                .clone()
        };
        data[..8].copy_from_slice(&ticket.to_le_bytes());
        data[8..16].copy_from_slice(b"tampered");
        assert_eq!(
            super::request(
                &mut fs,
                Volume::Primary,
                1,
                41,
                7,
                req(w::STAGE_CHUNK, 0, 16),
                b"/chosen",
                &mut data
            ),
            Ok(8)
        );
        assert_eq!(
            super::request(
                &mut fs,
                Volume::Primary,
                1,
                41,
                7,
                req(w::STAGE_PUBLISH, ticket, 0),
                b"/chosen",
                &mut data
            ),
            Err(w::INVALID)
        );
        assert_eq!(fs.open(b"/chosen"), Err(Error::NotFound));
        assert_eq!(
            super::request(
                &mut fs,
                Volume::Primary,
                1,
                41,
                7,
                req(w::STAGE_ABORT, ticket, 0),
                b"/chosen",
                &mut data
            ),
            Ok(0)
        );
        assert_eq!(fs.open(&source), Err(Error::NotFound));
    }

    #[test]
    fn namespace_and_sealed_image_aliases() {
        assert!(reserved(b"/.__sexfiles_staged"));
        assert!(reserved(b"//.__sexfiles_staged/file"));
        for path in [
            b"/.home.sexcrypt".as_slice(),
            b"/.home.sexcrypt/",
            b"//.home.sexcrypt",
        ] {
            assert!(sealed_image_alias(path));
        }
        assert!(!sealed_image_alias(b"/folder/.home.sexcrypt"));
        assert!(!visible_entry(true, b".__sexfiles_staged"));
        assert!(visible_entry(false, b".__sexfiles_staged"));
        assert!(visible_entry(true, b"personal.txt"));
    }
}
