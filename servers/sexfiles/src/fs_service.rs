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
/// Writes are not committed one request at a time (each commit costs two device
/// flushes). They are committed on SYNC, before any other change, once this many bytes
/// wait, and when no request has come for `COMMIT_DELAY` ticks.
const COMMIT_BYTES: u64 = 8 * 1024 * 1024;
const COMMIT_DELAY: u64 = 20;
static mut UNCOMMITTED: u64 = 0;
static mut LAST_WRITE: u64 = 0;
/// Uncommitted writes are waiting: the server polls rather than blocks until they commit.
pub fn writes_pending() -> bool { unsafe { UNCOMMITTED != 0 } }
/// Called when no request is waiting. Commits once writes have rested `COMMIT_DELAY` ticks.
pub fn commit_when_idle() {
    if !writes_pending() || sex_pdx::get_ticks().saturating_sub(unsafe { LAST_WRITE }) < COMMIT_DELAY { return; }
    if let State::Ready(fs) = unsafe { &mut *core::ptr::addr_of_mut!(STATE) } { commit_writes(fs); }
}
fn commit_writes(fs: &mut Fs<'static, Disk>) {
    let bytes = unsafe { core::mem::replace(&mut *core::ptr::addr_of_mut!(UNCOMMITTED), 0) };
    if let Err(e) = fs.commit() { sex_pdx::serial_record!("[files.fs.commit.failed] bytes={} error={:?}", bytes, e); }
}
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
/// Each program row's current run and the folder it may use. A run Shell never bound,
/// such as the boot image's own program, has no files at all.
const PROGRAM_ROWS: usize = sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Program);
const ROOT_MAX: usize = 64;
#[derive(Clone, Copy)]
struct Binding { task: u32, root: [u8; ROOT_MAX], len: usize, args: [u8; w::ARGS_MAX], args_len: usize }
static mut BINDINGS: [Option<Binding>; PROGRAM_ROWS] = [None; PROGRAM_ROWS];
/// The last program file read, by its base name, waiting for Shell to say where it runs.
static mut LOADED: ([u8; 32], usize) = ([0; 32], 0);
/// The arguments typed with the program last read.
static mut LOADED_ARGS: ([u8; w::ARGS_MAX], usize) = ([0; w::ARGS_MAX], 0);
fn program_index(target: u32) -> Option<usize> {
    sex_pdx::app_catalog::INCARNATIONS.iter().filter(|i| i.app == sex_pdx::app_catalog::App::Program).position(|i| i.target == target)
}
pub fn loaded(path: &[u8]) {
    let name = path.rsplit(|b| *b == b'/').next().unwrap_or(path);
    let loaded = unsafe { &mut *core::ptr::addr_of_mut!(LOADED) };
    let n = name.len().min(loaded.0.len());
    loaded.0[..n].copy_from_slice(&name[..n]);
    loaded.1 = n;
}
/// Shell's word that the file just read runs as `target`'s `task`. Its folder is made
/// here, so the program finds it waiting.
pub fn bind(target: u32, task: u32) {
    let Some(index) = program_index(target) else { return };
    let loaded = unsafe { &mut *core::ptr::addr_of_mut!(LOADED) };
    let bindings = unsafe { &mut *core::ptr::addr_of_mut!(BINDINGS) };
    if loaded.1 == 0 { bindings[index] = None; return; }
    let data = sex_pdx::program_store::DATA;
    let mut root = [0u8; ROOT_MAX];
    root[..data.len()].copy_from_slice(data);
    root[data.len()..data.len() + loaded.1].copy_from_slice(&loaded.0[..loaded.1]);
    let len = data.len() + loaded.1;
    loaded.1 = 0;
    let state = unsafe { &mut *core::ptr::addr_of_mut!(STATE) };
    if matches!(state, State::Untried) { *state = bring_up(); }
    if let State::Ready(fs) = state {
        // The rollback below must only undo this mkdir: commit waiting writes first,
        // and make no folder if they cannot be committed.
        if writes_pending() { commit_writes(fs); }
        for end in [data.len() - 1, len] {
            if fs.has_changes() { break; }
            if fs.open(&root[..end]).is_err() && fs.mkdir(&root[..end], 0).and_then(|_| fs.commit()).is_err() { let _ = fs.rollback(); }
        }
    }
    let args = unsafe { &mut *core::ptr::addr_of_mut!(LOADED_ARGS) };
    bindings[index] = Some(Binding { task, root, len, args: args.0, args_len: core::mem::take(&mut args.1) });
    sex_pdx::serial_record!("[files.program.bound] target={} task={} root={}", target, task, core::str::from_utf8(&root[..len]).unwrap_or("?"));
}
/// A program's path inside its folder. `..` never climbs out: it is refused.
fn confine(binding: &Binding, path: &[u8], out: &mut [u8; w::PATH_BYTES]) -> Option<usize> {
    // `@/path` names a file the person gave this run when starting it, and only that file.
    if path.first() == Some(&b'@') {
        let granted = binding.args[..binding.args_len].split(|b| *b == b' ').any(|token| token == path);
        let abs = &path[1..];
        if !granted || abs.first() != Some(&b'/') || abs.split(|b| *b == b'/').any(|p| p == b".." || p == b".") || abs.len() > out.len() { return None; }
        out[..abs.len()].copy_from_slice(abs);
        return Some(abs.len());
    }
    let mut n = binding.len;
    out[..n].copy_from_slice(&binding.root[..n]);
    for part in path.split(|b| *b == b'/').filter(|p| !p.is_empty() && *p != b".") {
        if part == b".." || part.contains(&0) { return None; }
        if n + 1 + part.len() > out.len() { return None; }
        out[n] = b'/';
        out[n + 1..n + 1 + part.len()].copy_from_slice(part);
        n += 1 + part.len();
    }
    Some(n)
}
/// The row is running, and running the very run Shell bound: the kernel names its task.
fn program_live(target: u32, task: u32) -> bool {
    let Some(row) = sex_pdx::app_catalog::by_target(target) else { return false };
    matches!(sex_pdx::app_lifecycle::catalog(row.lifecycle_slot), Ok(c) if c.is_subject(target, row.surface, 3) && c.task_id == task)
        && sex_pdx::app_lifecycle::info(row.lifecycle_slot) == Err(sex_pdx::app_lifecycle::LIVE)
}
/// The answer for one request. Never touches another client's region.
pub fn handle(caller: u32, a: u64, offset: u64, length: u64) -> u64 {
    let Some(r) = w::Request::decode(a, offset, length) else { return w::reply((a & 0xFF_FFFF) as u32, w::INVALID, 0) };
    let program = program_index(caller);
    let index = match (sex_pdx::app_catalog::SPINDLE_TARGETS.iter().position(|t| *t == caller), program) {
        (Some(index), _) => { if !crate::spindle_history::live(caller) { return w::reply(r.seq, w::DENIED, 0); } index }
        (None, Some(k)) => {
            let bound = unsafe { (*core::ptr::addr_of!(BINDINGS))[k] };
            if !bound.is_some_and(|b| program_live(caller, b.task)) || r.kind == w::PREPARE || r.kind == w::INSTALL { return w::reply(r.seq, w::DENIED, 0); }
            w::PROGRAM_INDEX_BASE + k
        }
        _ => return w::reply(r.seq, w::DENIED, 0),
    };
    let Ok(view) = sex_pdx::memory_region_view(w::PROVIDER_SLOT_BASE + index as u64, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE) else { return w::reply(r.seq, w::IO, 0) };
    let region = unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) };
    let state = unsafe { &mut *core::ptr::addr_of_mut!(STATE) };
    if matches!(state, State::Untried) { *state = bring_up(); }
    let State::Ready(fs) = state else { return w::reply(r.seq, w::UNAVAILABLE, 0) };
    let mut path = [0u8; w::PATH_BYTES];
    let mut path_len = r.path_len as usize;
    path[..path_len].copy_from_slice(&region[..path_len]);
    if let (Some(k), true) = (program, r.kind == w::ARGS) {
        let binding = unsafe { (*core::ptr::addr_of!(BINDINGS))[k] }.unwrap();
        region[w::PATH_BYTES..w::PATH_BYTES + binding.args_len].copy_from_slice(&binding.args[..binding.args_len]);
        return w::reply(r.seq, w::OK, binding.args_len as u32);
    }
    if let Some(k) = program {
        // Every path, both halves of a rename too, lands inside the program's folder.
        let binding = unsafe { (*core::ptr::addr_of!(BINDINGS))[k] }.unwrap();
        let mut own = [0u8; w::PATH_BYTES];
        let raw = path;
        let n = if r.kind == w::RENAME {
            let Some(split) = raw[..path_len].iter().position(|b| *b == 0) else { return w::reply(r.seq, w::NAME, 0) };
            let mut second = [0u8; w::PATH_BYTES];
            let (Some(a), Some(b)) = (confine(&binding, &raw[..split], &mut own), confine(&binding, &raw[split + 1..path_len], &mut second)) else { return w::reply(r.seq, w::NAME, 0) };
            if a + 1 + b > own.len() { return w::reply(r.seq, w::NAME, 0); }
            own[a] = 0; own[a + 1..a + 1 + b].copy_from_slice(&second[..b]);
            a + 1 + b
        } else {
            let Some(n) = confine(&binding, &raw[..path_len], &mut own) else { return w::reply(r.seq, w::NAME, 0) };
            n
        };
        path = own; path_len = n;
    }
    let data = &mut region[w::PATH_BYTES..];
    // Reads change nothing; a write fails before it changes anything (sexfs checks
    // first, and a device error poisons the filesystem). Every other change commits
    // waiting writes first, so its own rollback can only undo itself.
    let read_only = matches!(r.kind, w::STAT | w::LIST | w::READ | w::STATFS);
    let batched = r.kind == w::WRITE;
    if !read_only && !batched && writes_pending() {
        commit_writes(fs);
        if fs.has_changes() { return w::reply(r.seq, w::IO, 0); }
    }
    let result = run(fs, r, &path[..path_len], data);
    match result {
        Ok(value) => {
            if batched && value != 0 {
                unsafe { UNCOMMITTED += value as u64; LAST_WRITE = sex_pdx::get_ticks(); }
                if unsafe { UNCOMMITTED } >= COMMIT_BYTES { commit_writes(fs); }
            }
            if r.kind == w::SYNC { unsafe { UNCOMMITTED = 0; } }
            w::reply(r.seq, w::OK, value)
        }
        Err(e) => {
            // Nothing half done becomes durable: back to the last commit.
            if !read_only && !batched && !matches!(e, Error::Io | Error::Corrupt | Error::Poisoned) { let _ = fs.rollback(); }
            w::reply(r.seq, status(e), 0)
        }
    }
}
/// One path waiting for Shell to ask for it, and the ticket that names it.
static mut PREPARED: ([u8; w::PATH_BYTES], usize, u32) = ([0; w::PATH_BYTES], 0, 0);
static mut PREPARED_ARGS: ([u8; w::ARGS_MAX], usize) = ([0; w::ARGS_MAX], 0);
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
    loaded(path);
    sex_pdx::serial_record!("[files.program.file] bytes={}", size);
    Program::Loaded(size as u32)
}
/// A bare name looks in the program directory.
pub fn read_named(name: &[u8], image: &mut [u8]) -> Program {
    unsafe { (*core::ptr::addr_of_mut!(LOADED)).1 = 0; (*core::ptr::addr_of_mut!(LOADED_ARGS)).1 = 0; }
    use sex_pdx::program_store as store;
    let mut path = [0u8; 32];
    let at = store::BIN.len();
    path[..at].copy_from_slice(store::BIN);
    path[at..at + name.len()].copy_from_slice(name);
    read_program(&path[..at + name.len()], image)
}
/// The prepared path, once. A stale ticket finds nothing.
pub fn read_ticket(ticket: u64, image: &mut [u8]) -> Program {
    unsafe { (*core::ptr::addr_of_mut!(LOADED)).1 = 0; }
    let prepared = unsafe { &mut *core::ptr::addr_of_mut!(PREPARED) };
    if prepared.1 == 0 || prepared.2 as u64 != ticket { return Program::Absent; }
    let len = core::mem::take(&mut prepared.1);
    unsafe { *core::ptr::addr_of_mut!(LOADED_ARGS) = *core::ptr::addr_of!(PREPARED_ARGS); }
    read_program(&prepared.0[..len], image)
}
fn run(fs: &mut Fs<'static, Disk>, r: w::Request, path: &[u8], data: &mut [u8]) -> Result<u32, Error> {
    let now = 0;
    // Test builds only: a path that makes Files fall over, to show it comes back.
    if option_env!("SEXOS_FILES_CRASH_PROBE").is_some() && path == b"/crash.probe" {
        sex_pdx::serial_record!("[files.crash.probe]");
        unsafe { core::ptr::write_volatile(8 as *mut u8, 1) };
    }
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
                // After progress, an error ends the write short: the prefix is real and
                // counted, so it commits with the rest (a short write, as on Linux).
                let wrote = match fs.write(st.id, r.offset + done as u64, &own[..take], now) {
                    Ok(n) => n, Err(e) if done == 0 || matches!(e, Error::Io | Error::Corrupt | Error::Poisoned) => return Err(e), Err(_) => break,
                };
                done += wrote;
                if wrote < take { break; }
            }
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
            // Arguments travel with the path, after it in the data area.
            let args = unsafe { &mut *core::ptr::addr_of_mut!(PREPARED_ARGS) };
            let n = (r.length as usize).min(w::ARGS_MAX).min(data.len());
            args.0[..n].copy_from_slice(&data[..n]);
            args.1 = n;
            // Never zero, and within the reply's 24 bits.
            let ticket = unsafe { NEXT_TICKET = NEXT_TICKET % 0xFF_FFFE + 1; NEXT_TICKET };
            prepared.2 = ticket;
            Ok(ticket)
        }
        w::INSTALL => {
            // No file: the image this system booted with.
            let system = if path.is_empty() {
                let info = sex_pdx::memory_region_info(w::BOOT_IMAGE_SLOT).map_err(|_| Error::NotFound)?;
                let view = sex_pdx::memory_region_view(w::BOOT_IMAGE_SLOT, 0, info.length, QUERY_VIEW | READ).map_err(|_| Error::NotFound)?;
                Some(unsafe { core::slice::from_raw_parts(view.virtual_address as *const u8, view.length as usize) })
            } else { None };
            let (id, size) = match system {
                Some(image) => (0, (image.len() as u64) & !511),
                None => { let st = fs.open(path)?; if st.kind == Kind::Dir { return Err(Error::IsDir); } (st.id, st.size) }
            };
            if size == 0 || size > w::BOOT_SECTORS * 512 || size % 512 != 0 { return Err(Error::TooBig); }
            struct Source { id: u64, size: u64 }
            let st = Source { id, size };
            let va = crate::vfs::diskfs_bridge_get_buf_va();
            if va == 0 || va == u64::MAX { return Err(Error::Io); }
            let mut own = [0u8; BLOCK];
            let mut at = 0u64;
            while at < st.size {
                let take = (st.size - at).min(BLOCK as u64) as usize;
                own[take..].fill(0);
                match system {
                    Some(image) => own[..take].copy_from_slice(&image[at as usize..at as usize + take]),
                    None => if fs.read(st.id, at, &mut own[..take])? != take { return Err(Error::Corrupt); },
                }
                unsafe { core::slice::from_raw_parts_mut(va as *mut u8, BLOCK) }.copy_from_slice(&own);
                if DiskFs::diskfs_block_write(w::BOOT_LBA * 512 + at, BLOCK as u64, SLOT_BUF_LEND) != 0 { return Err(Error::Io); }
                at += take as u64;
            }
            // Last: one bootable EFI System partition over the slot, so a half-written image
            // never boots.
            let mut mbr = [0u8; 512];
            let entry = &mut mbr[446..462];
            entry[0] = 0x80; entry[1..4].copy_from_slice(&[0xFE, 0xFF, 0xFF]); entry[4] = 0xEF; entry[5..8].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
            entry[8..12].copy_from_slice(&(w::BOOT_LBA as u32).to_le_bytes());
            entry[12..16].copy_from_slice(&((st.size / 512) as u32).to_le_bytes());
            mbr[510] = 0x55; mbr[511] = 0xAA;
            let view = unsafe { core::slice::from_raw_parts_mut(va as *mut u8, BLOCK) };
            view.fill(0);
            view[..512].copy_from_slice(&mbr);
            if DiskFs::diskfs_block_write(0, 512, SLOT_BUF_LEND) != 0 { return Err(Error::Io); }
            if DiskFs::diskfs_block_sync() != 0 { return Err(Error::Io); }
            sex_pdx::serial_record!("[files.install.done] bytes={} lba={}", st.size, w::BOOT_LBA);
            Ok((st.size / 512) as u32)
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
