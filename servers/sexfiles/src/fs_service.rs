//! Hosts the filesystem on the disk past the fixed DiskFS area. Paths come through
//! a region the client can still write, so they are copied before use.
use crate::backends::diskfs::DiskFs;
use sex_pdx::fs as w;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::SLOT_BUF_LEND;
use sexfs::{Device, Error, Fs, Kind, Memory, Slot, BLOCK};

const SECTORS: u64 = (BLOCK / 512) as u64;
const CACHE: usize = 64;
/// 16 GiB until free space stops needing a bitmap held in memory.
const MAP_WORDS: usize = 65536;
static mut CACHE_BLOCKS: [[u8; BLOCK]; CACHE] = [[0; BLOCK]; CACHE];
static mut SLOTS: [Slot; CACHE] = [Slot::EMPTY; CACHE];
static mut USED: [u64; MAP_WORDS] = [0; MAP_WORDS];
static mut PENDING: [u64; MAP_WORDS] = [0; MAP_WORDS];
static mut SCRATCH: [u8; BLOCK] = [0; BLOCK];
static mut STATE: State = State::Untried;
enum State { Untried, Absent, Ready(Fs<'static, Disk>) }

pub struct Disk { blocks: u64 }
impl Device for Disk {
    fn blocks(&self) -> u64 { self.blocks }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> {
        let va = crate::vfs::diskfs_bridge_get_buf_va();
        if va == 0 || va == u64::MAX { return Err(()); }
        if DiskFs::diskfs_block_read((w::BASE_LBA + block * SECTORS) * 512, BLOCK as u64, SLOT_BUF_LEND) != 0 { return Err(()); }
        out.copy_from_slice(unsafe { core::slice::from_raw_parts(va as *const u8, BLOCK) });
        Ok(())
    }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<(), ()> {
        let va = crate::vfs::diskfs_bridge_get_buf_va();
        if va == 0 || va == u64::MAX { return Err(()); }
        unsafe { core::slice::from_raw_parts_mut(va as *mut u8, BLOCK) }.copy_from_slice(data);
        if DiskFs::diskfs_block_write((w::BASE_LBA + block * SECTORS) * 512, BLOCK as u64, SLOT_BUF_LEND) != 0 { return Err(()); }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), ()> {
        if DiskFs::diskfs_block_sync() == 0 { Ok(()) } else { Err(()) }
    }
}
fn memory() -> Memory<'static> {
    unsafe {
        Memory {
            cache: &mut *core::ptr::addr_of_mut!(CACHE_BLOCKS),
            slots: &mut *core::ptr::addr_of_mut!(SLOTS),
            used: &mut *core::ptr::addr_of_mut!(USED),
            pending: &mut *core::ptr::addr_of_mut!(PENDING),
            scratch: &mut *core::ptr::addr_of_mut!(SCRATCH),
        }
    }
}
fn bring_up() -> State {
    let sectors = DiskFs::diskfs_block_call(sex_pdx::BLOCK_CAPACITY, 0, 0, 0);
    if sectors < w::MIN_SECTORS {
        sex_pdx::serial_record!("[files.fs.absent] sectors={}", sectors);
        return State::Absent;
    }
    let blocks = (sectors - w::BASE_LBA) / SECTORS;
    match Fs::mount(Disk { blocks }, memory()) {
        Ok(fs) => {
            let s = fs.statfs();
            sex_pdx::serial_record!("[files.fs.mounted] total={} free={} generation={}", s.total, s.free, s.generation);
            State::Ready(fs)
        }
        Err(Error::Unformatted) => match Fs::format(Disk { blocks }, memory()) {
            Ok(fs) => {
                sex_pdx::serial_record!("[files.fs.formatted] total={}", fs.statfs().total);
                let mut fs = fs;
                if fs.mkdir(b"/bin", 0).and_then(|_| fs.commit()).is_err() { sex_pdx::serial_record!("[files.fs.bin.failed]"); }
                State::Ready(fs)
            }
            Err(e) => { sex_pdx::serial_record!("[files.fs.format.failed] error={:?}", e); State::Absent }
        },
        Err(e) => { sex_pdx::serial_record!("[files.fs.mount.failed] error={:?}", e); State::Absent }
    }
}
fn status(e: Error) -> u8 {
    match e {
        Error::Io | Error::Poisoned => w::IO, Error::Corrupt => w::CORRUPT, Error::NoSpace => w::NO_SPACE, Error::NotFound => w::NOT_FOUND,
        Error::Exists => w::EXISTS, Error::NotDir => w::NOT_DIR, Error::IsDir => w::IS_DIR, Error::NotEmpty => w::NOT_EMPTY,
        Error::Name | Error::TooBig => w::NAME, Error::Unformatted => w::UNAVAILABLE,
    }
}
/// A client's reply slot may hold another service's answer, so ours waits its turn.
/// Each client has one request in flight, so one entry per caller is all it takes.
static mut REPLIES: [(u32, u64); 64] = [(0, 0); 64];
pub fn reply_later(caller: u32, word: u64) {
    let replies = unsafe { &mut *core::ptr::addr_of_mut!(REPLIES) };
    match replies.iter_mut().find(|r| r.0 == 0 || r.0 == caller) {
        Some(entry) => *entry = (caller, word),
        None => sex_pdx::serial_record!("[files.fs.reply.lost] caller={}", caller),
    }
}
pub fn replies_pending() -> bool { unsafe { (*core::ptr::addr_of!(REPLIES)).iter().any(|r| r.0 != 0) } }
pub fn poll_replies() {
    let replies = unsafe { &mut *core::ptr::addr_of_mut!(REPLIES) };
    for entry in replies.iter_mut().filter(|r| r.0 != 0) {
        if sex_pdx::pdx_reply(entry.0, entry.1) == 0 { *entry = (0, 0); }
    }
}
/// The answer for one request. Never touches another client's region.
pub fn handle(caller: u32, a: u64, offset: u64, length: u64) -> u64 {
    let Some(r) = w::Request::decode(a, offset, length) else { return w::reply((a & 0xFF_FFFF) as u32, w::INVALID, 0) };
    let Some(index) = sex_pdx::app_catalog::SPINDLE_TARGETS.iter().position(|t| *t == caller) else { return w::reply(r.seq, w::DENIED, 0) };
    if !crate::spindle_history::live(caller) { return w::reply(r.seq, w::DENIED, 0); }
    let Ok(view) = sex_pdx::memory_region_view(w::PROVIDER_SLOT_BASE + index as u64, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE) else { return w::reply(r.seq, w::IO, 0) };
    let region = unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) };
    let state = unsafe { &mut *core::ptr::addr_of_mut!(STATE) };
    if matches!(state, State::Untried) { *state = bring_up(); }
    let State::Ready(fs) = state else { return w::reply(r.seq, w::UNAVAILABLE, 0) };
    let mut path = [0u8; w::PATH_BYTES];
    let path_len = r.path_len as usize;
    path[..path_len].copy_from_slice(&region[..path_len]);
    let data = &mut region[w::PATH_BYTES..];
    let result = run(fs, r, &path[..path_len], data);
    match result {
        Ok(value) => w::reply(r.seq, w::OK, value),
        Err(e) => {
            // Nothing half done becomes durable: back to the last commit.
            if !matches!(e, Error::Io | Error::Corrupt | Error::Poisoned) { let _ = fs.rollback(); }
            w::reply(r.seq, status(e), 0)
        }
    }
}
/// One path waiting for Shell to ask for it, and the ticket that names it.
static mut PREPARED: ([u8; w::PATH_BYTES], usize, u32) = ([0; w::PATH_BYTES], 0, 0);
static mut NEXT_TICKET: u32 = 0;
pub enum Program { Absent, Failed(u32), Loaded(u32) }
/// Copies a file into `image`. A disk with no filesystem has no such file.
fn read_program(path: &[u8], image: &mut [u8]) -> Program {
    use sex_pdx::program_store as store;
    let state = unsafe { &mut *core::ptr::addr_of_mut!(STATE) };
    if matches!(state, State::Untried) { *state = bring_up(); }
    let State::Ready(fs) = state else { return Program::Absent };
    let st = match fs.open(path) {
        Ok(st) => st,
        Err(Error::NotFound | Error::NotDir) => return Program::Absent,
        Err(_) => return Program::Failed(store::IO),
    };
    if st.kind == Kind::Dir { return Program::Failed(store::NOT_A_PROGRAM); }
    if st.size == 0 { return Program::Failed(store::NOT_A_PROGRAM); }
    if st.size > image.len() as u64 { return Program::Failed(store::TOO_LARGE); }
    let (mut done, size) = (0usize, st.size as usize);
    while done < size {
        match fs.read(st.id, done as u64, &mut image[done..size]) {
            Ok(0) => return Program::Failed(store::CORRUPT),
            Ok(n) => done += n,
            Err(Error::Corrupt) => return Program::Failed(store::CORRUPT),
            Err(_) => return Program::Failed(store::IO),
        }
    }
    sex_pdx::serial_record!("[files.program.file] bytes={}", size);
    Program::Loaded(size as u32)
}
/// A bare name looks in the program directory.
pub fn read_named(name: &[u8], image: &mut [u8]) -> Program {
    use sex_pdx::program_store as store;
    let mut path = [0u8; 32];
    let at = store::BIN.len();
    path[..at].copy_from_slice(store::BIN);
    path[at..at + name.len()].copy_from_slice(name);
    read_program(&path[..at + name.len()], image)
}
/// The prepared path, once. A stale ticket finds nothing.
pub fn read_ticket(ticket: u64, image: &mut [u8]) -> Program {
    let prepared = unsafe { &mut *core::ptr::addr_of_mut!(PREPARED) };
    if prepared.1 == 0 || prepared.2 as u64 != ticket { return Program::Absent; }
    let len = core::mem::take(&mut prepared.1);
    read_program(&prepared.0[..len], image)
}
fn run(fs: &mut Fs<'static, Disk>, r: w::Request, path: &[u8], data: &mut [u8]) -> Result<u32, Error> {
    let now = 0;
    match r.kind {
        w::STAT => {
            let st = fs.open(path)?;
            data[..8].copy_from_slice(&((st.kind == Kind::Dir) as u64).to_le_bytes());
            data[8..16].copy_from_slice(&st.size.to_le_bytes());
            data[16..24].copy_from_slice(&st.mtime.to_le_bytes());
            data[24..32].copy_from_slice(&st.id.to_le_bytes());
            Ok(32)
        }
        w::LIST => {
            let id = fs.open(path)?.id;
            let (mut cursor, mut used, mut name) = (r.offset, w::LIST_HEAD, [0u8; sexfs::NAME_MAX]);
            let mut next = 0;
            while let Some((len, kind, child, after)) = fs.read_dir(id, cursor, &mut name)? {
                if used + w::ENTRY_HEAD + len > 4000 { next = cursor; break; }
                let size = if kind == Kind::File { fs.stat(child)?.size } else { 0 };
                data[used] = (kind == Kind::Dir) as u8;
                data[used + 1] = len as u8;
                data[used + 2..used + 10].copy_from_slice(&size.to_le_bytes());
                data[used + w::ENTRY_HEAD..used + w::ENTRY_HEAD + len].copy_from_slice(&name[..len]);
                used += w::ENTRY_HEAD + len;
                cursor = after;
            }
            data[..8].copy_from_slice(&next.to_le_bytes());
            Ok(used as u32)
        }
        w::MKDIR => { fs.mkdir(path, now)?; fs.commit()?; Ok(0) }
        w::CREATE => { fs.create(path, now)?; fs.commit()?; Ok(0) }
        w::REMOVE => { fs.remove_path(path)?; fs.commit()?; Ok(0) }
        w::RENAME => {
            let split = path.iter().position(|b| *b == 0).ok_or(Error::Name)?;
            fs.rename(&path[..split], &path[split + 1..])?;
            fs.commit()?;
            Ok(0)
        }
        w::READ => {
            let st = fs.open(path)?;
            let want = (r.length as usize).min(data.len());
            Ok(fs.read(st.id, r.offset, &mut data[..want])? as u32)
        }
        w::WRITE => {
            if r.length as usize > data.len() { return Err(Error::TooBig); }
            let st = fs.open(path)?;
            let mut own = [0u8; BLOCK];
            let (mut done, total) = (0usize, r.length as usize);
            while done < total {
                let take = (total - done).min(BLOCK);
                own[..take].copy_from_slice(&data[done..done + take]);
                let wrote = fs.write(st.id, r.offset + done as u64, &own[..take], now)?;
                done += wrote;
                if wrote < take { break; }
            }
            fs.commit()?;
            Ok(done as u32)
        }
        w::TRUNCATE => { let st = fs.open(path)?; fs.truncate(st.id, r.offset, now)?; fs.commit()?; Ok(0) }
        w::SYNC => { fs.commit()?; Ok(0) }
        w::PREPARE => {
            let st = fs.open(path)?;
            if st.kind == Kind::Dir { return Err(Error::IsDir); }
            let prepared = unsafe { &mut *core::ptr::addr_of_mut!(PREPARED) };
            prepared.0[..path.len()].copy_from_slice(path);
            prepared.1 = path.len();
            // Never zero, and within the reply's 24 bits.
            let ticket = unsafe { NEXT_TICKET = NEXT_TICKET % 0xFF_FFFE + 1; NEXT_TICKET };
            prepared.2 = ticket;
            Ok(ticket)
        }
        w::STATFS => {
            let s = fs.statfs();
            data[..8].copy_from_slice(&s.total.to_le_bytes());
            data[8..16].copy_from_slice(&s.free.to_le_bytes());
            data[16..24].copy_from_slice(&s.generation.to_le_bytes());
            Ok(24)
        }
        _ => Err(Error::Name),
    }
}
