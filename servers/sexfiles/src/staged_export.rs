// SPDX-License-Identifier: GPL-3.0-only
//! Files owns this staging namespace. A run receives a ticket, never a stage path.
//! The only visible destination transition is SexFS's checked COW move and commit.
use crate::fs_service::{self, Disk};
use alloc::vec::Vec;
use sex_pdx::fs as w;
use sexfs::{Error, Fs, Kind, NAME_MAX};
use sha2::{Digest, Sha256};

const ROOT: &[u8] = b"/.__sexfiles_staged";
const MARK: &[u8] = b"/.__sexfiles_staged/.owner";
const MAGIC: &[u8] = b"SexOS Files staged export v1";
static mut NEXT: u64 = 0;
static mut ACTIVE: Option<Stage> = None;
static mut CLEAN: bool = false;

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
    path.split(|b| *b == b'/').find(|part| !part.is_empty())
        == Some(b".__sexfiles_staged".as_slice())
}

fn code(e: Error) -> u8 {
    fs_service::status(e)
}
fn now() -> u64 {
    sex_pdx::civil_time_ms().map(|ms| ms / 1000).unwrap_or(0)
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
pub fn recover(fs: &mut Fs<'static, Disk>) {
    unsafe {
        CLEAN = false;
        ACTIVE = None;
    }
    let root = match fs.open(ROOT) {
        Ok(s) => s,
        Err(Error::NotFound) => {
            unsafe {
                CLEAN = true;
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
        CLEAN = true;
    }
}

fn ensure_root(fs: &mut Fs<'static, Disk>) -> Result<(), u8> {
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
    row: usize,
    task: u32,
    token: u64,
    destination: &[u8],
) -> Result<&'static mut Stage, u8> {
    let stage = unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE) };
    let s = stage.as_mut().ok_or(w::STALE)?;
    if s.row != row || s.task != task || s.ticket != token || s.destination != destination {
        return Err(w::STALE);
    }
    Ok(s)
}

pub fn request(
    fs: &mut Fs<'static, Disk>,
    row: usize,
    target: u32,
    task: u32,
    r: w::Request,
    destination: &[u8],
    data: &mut [u8],
) -> Result<u32, u8> {
    if !unsafe { CLEAN } {
        return Err(w::UNAVAILABLE);
    }
    if reserved(destination) || destination.is_empty() || destination == b"/" {
        return Err(w::DENIED);
    }
    if fs_service::writes_pending() {
        fs_service::commit_now(fs);
        if fs.has_changes() {
            return Err(w::IO);
        }
    }
    match r.kind {
        w::STAGE_BEGIN => {
            if r.length != 32 || r.offset > (4 << 20) {
                return Err(w::INVALID);
            }
            if let Some(s) = unsafe { &*core::ptr::addr_of!(ACTIVE) } {
                if fs_service::live_run(s.target, s.task) {
                    return Err(w::PENDING);
                }
                // The old run ended; only its original object may be discarded.
                fs.remove_path_if_id(&s.source, s.id)
                    .and_then(|_| fs.commit())
                    .map_err(code)?;
                unsafe {
                    ACTIVE = None;
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
                NEXT ^ sex_pdx::get_ticks().rotate_left(17)
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
                ACTIVE = Some(Stage {
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
            let s = ticket_stage(row, task, token, destination)?;
            let bytes = &data[8..n];
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
            let s = ticket_stage(row, task, r.offset, destination)?;
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
                ACTIVE = None;
            }
            Ok(0)
        }
        w::STAGE_ABORT => {
            if r.length != 0 {
                return Err(w::INVALID);
            }
            let s = ticket_stage(row, task, r.offset, destination)?;
            fs.remove_path_if_id(&s.source, s.id)
                .and_then(|_| fs.commit())
                .map_err(code)?;
            unsafe {
                ACTIVE = None;
            }
            Ok(0)
        }
        _ => Err(w::INVALID),
    }
}
