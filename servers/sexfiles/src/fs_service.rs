//! Hosts the filesystem on the disk past the fixed DiskFS area. Paths come through
//! a region the client can still write, so they are copied before use.
use crate::backends::diskfs::DiskFs;
use sex_pdx::fs as w;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::SLOT_BUF_LEND;
use sexfs::{Device, Error, Fs, Kind, Memory, Slot, BLOCK};

const SECTORS: u64 = (BLOCK / 512) as u64;
/// Blocks of tree cached in memory, once Files' memory has grown to hold them.
const CACHE: usize = 1024;
/// What Files has when its memory cannot grow: 64 blocks and a map for 16 GiB.
const SMALL_CACHE: usize = 64;
const SMALL_MAP_WORDS: usize = 65536;
static mut SMALL_BLOCKS: [[u8; BLOCK]; SMALL_CACHE] = [[0; BLOCK]; SMALL_CACHE];
static mut SMALL_SLOTS: [Slot; SMALL_CACHE] = [Slot::EMPTY; SMALL_CACHE];
static mut SMALL_USED: [u64; SMALL_MAP_WORDS] = [0; SMALL_MAP_WORDS];
static mut SMALL_PENDING: [u64; SMALL_MAP_WORDS] = [0; SMALL_MAP_WORDS];
/// Where the filesystem's memory lives: (cache, slots, words, used, pending).
static mut PLACE: (*mut [u8; BLOCK], *mut Slot, usize, usize, *mut u64, *mut u64) =
    (core::ptr::null_mut(), core::ptr::null_mut(), 0, 0, core::ptr::null_mut(), core::ptr::null_mut());
static mut SCRATCH: [u8; BLOCK] = [0; BLOCK];
/// Files' own copy of one request's data.
static mut OWN: [u8; w::DATA_MAX] = [0; w::DATA_MAX];
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
/// Bytes written outside a WRITE request (a mounted volume's image): committed like one.
pub fn wrote(bytes: u64) { unsafe { UNCOMMITTED += bytes.max(1); LAST_WRITE = sex_pdx::get_ticks(); } }
/// Called when no request is waiting. Commits once writes have rested `COMMIT_DELAY` ticks.
pub fn commit_when_idle() {
    if !writes_pending() || sex_pdx::get_ticks().saturating_sub(unsafe { LAST_WRITE }) < COMMIT_DELAY { return; }
    if let State::Ready(fs) = unsafe { &mut *core::ptr::addr_of_mut!(STATE) } { commit_writes(fs); }
}
fn commit_writes(fs: &mut Fs<'static, Disk>) {
    let bytes = unsafe { core::mem::replace(&mut *core::ptr::addr_of_mut!(UNCOMMITTED), 0) };
    // Home's own commit first: its blocks then go out with the system's.
    crate::home::sync(fs);
    if let Err(e) = fs.commit() { forget_stats(None); sex_pdx::serial_record!("[files.fs.commit.failed] bytes={} error={:?}", bytes, e); }
}
/// Clients' regions, found once: (address, 0 until looked up, u64::MAX if absent).
static mut REGIONS: [u64; 64] = [0; 64];
// Files serves requests serially. Device counters are sampled only around an
// authenticated client's READ, never exposed as other clients' global activity.
static mut DEVICE_READ_CALLS: u64 = 0;
static mut DEVICE_READ_BYTES: u64 = 0;
static mut READ_IO: [w::ReadIoStats; 64] = [w::ReadIoStats::ZERO; 64];
const _: () = assert!(w::PROGRAM_INDEX_BASE + PROGRAM_ROWS <= 64);
fn client_cache(index: usize) -> Option<&'static [u8]> {
    let regions = unsafe { &mut *core::ptr::addr_of_mut!(REGIONS) };
    let slot = regions.get_mut(index)?;
    if *slot == 0 {
        *slot = sex_pdx::memory_region_view(w::PROVIDER_SLOT_BASE + index as u64, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE)
            .map(|v| v.virtual_address).unwrap_or(u64::MAX);
    }
    if *slot == u64::MAX { return None; }
    Some(unsafe { core::slice::from_raw_parts((*slot as usize + w::CACHE_AT) as *const u8, w::CACHE_BYTES) })
}
/// Clears what clients were told about `file` (a file id), or about
/// everything, before the change can be seen. Only Files' own answers are touched,
/// never a client's data.
fn forget_stats(file: Option<u64>) {
    static mut CLEARED: u64 = 0;
    for index in 0..w::PROGRAM_INDEX_BASE + PROGRAM_ROWS {
        if let Some(cache) = client_cache(index) {
            let n = w::stat_cache::forget(cache, file) as u64;
            unsafe { CLEARED += n; if n != 0 && CLEARED.is_power_of_two() { sex_pdx::serial_record!("[files.stat_cache.forget] cleared={}", CLEARED); } }
        }
    }
}
/// Before power goes: commits waiting writes and flushes the disk's own cache.
pub fn flush_for_power() -> bool {
    let committed = match unsafe { &mut *core::ptr::addr_of_mut!(STATE) } {
        State::Ready(fs) => {
            if writes_pending() { commit_writes(fs); }
            !fs.has_changes()
        }
        _ => true,
    };
    // Flushes and notifies the drive of the power loss (NVMe CC.SHN) in one call.
    committed && DiskFs::diskfs_block_shutdown() == 0
}
enum State { Untried, Absent, Ready(Fs<'static, Disk>) }

pub struct Disk { blocks: u64 }
impl Disk {
    /// The shared block buffer, as long as one command may move.
    fn buffer() -> Result<&'static mut [u8], ()> {
        let va = crate::vfs::diskfs_bridge_get_buf_va();
        let len = crate::vfs::diskfs_bridge_buf_len() as usize;
        if va == 0 || va == u64::MAX || len < BLOCK { return Err(()); }
        Ok(unsafe { core::slice::from_raw_parts_mut(va as *mut u8, len) })
    }
    fn offset(block: u64) -> u64 { (w::BASE_LBA + block * SECTORS) * 512 }
}
impl Device for Disk {
    fn blocks(&self) -> u64 { self.blocks }
    fn read(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> { self.read_run(block, out) }
    fn write(&mut self, block: u64, data: &[u8; BLOCK]) -> Result<(), ()> { self.write_run(block, data) }
    fn run_blocks(&self) -> usize { crate::vfs::diskfs_bridge_buf_len() as usize / BLOCK }
    // One command per run: per-command cost, not bytes, is what a disk charges for.
    fn read_run(&mut self, first: u64, out: &mut [u8]) -> Result<(), ()> {
        let buf = Self::buffer()?;
        if out.is_empty() || out.len() % BLOCK != 0 || out.len() > buf.len() { return Err(()); }
        if DiskFs::diskfs_block_read(Self::offset(first), out.len() as u64, SLOT_BUF_LEND) != 0 { return Err(()); }
        out.copy_from_slice(&buf[..out.len()]);
        Ok(())
    }
    fn read_data(&mut self, block: u64, out: &mut [u8; BLOCK]) -> Result<(), ()> {
        self.read_data_run(block, out)
    }
    fn read_data_run(&mut self, first: u64, out: &mut [u8]) -> Result<(), ()> {
        self.read_run(first, out)?;
        unsafe {
            DEVICE_READ_CALLS = DEVICE_READ_CALLS.saturating_add(1);
            DEVICE_READ_BYTES = DEVICE_READ_BYTES.saturating_add(out.len() as u64);
        }
        Ok(())
    }
    fn write_run(&mut self, first: u64, data: &[u8]) -> Result<(), ()> {
        let buf = Self::buffer()?;
        if data.is_empty() || data.len() % BLOCK != 0 || data.len() > buf.len() { return Err(()); }
        buf[..data.len()].copy_from_slice(data);
        if DiskFs::diskfs_block_write(Self::offset(first), data.len() as u64, SLOT_BUF_LEND) != 0 { return Err(()); }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), ()> {
        if DiskFs::diskfs_block_sync() == 0 { Ok(()) } else { Err(()) }
    }
}
/// Memory sized to the disk: a free-space bit per block, so any disk fits whole.
/// Falls back to the fixed 16 GiB maps if Files' memory cannot grow that far.
fn place(blocks: u64) {
    use alloc::vec::Vec;
    let words = (blocks as usize).div_ceil(64) + 1;
    fn grab<T: Clone>(n: usize, v: T) -> Option<Vec<T>> {
        let mut x: Vec<T> = Vec::new();
        x.try_reserve_exact(n).ok()?;
        x.resize(n, v);
        Some(x)
    }
    // Kept owned until all four exist, so a failure frees what was already taken.
    let big = (|| Some((grab(CACHE, [0u8; BLOCK])?, grab(CACHE, Slot::EMPTY)?, grab(words, 0u64)?, grab(words, 0u64)?)))()
        .map(|(c, s, u, p)| (c.leak().as_mut_ptr(), s.leak().as_mut_ptr(), u.leak().as_mut_ptr(), p.leak().as_mut_ptr()));
    unsafe {
        PLACE = match big {
            Some((cache, slots, used, pending)) => (cache, slots, CACHE, words, used, pending),
            None => (core::ptr::addr_of_mut!(SMALL_BLOCKS).cast(), core::ptr::addr_of_mut!(SMALL_SLOTS).cast(), SMALL_CACHE,
                SMALL_MAP_WORDS, core::ptr::addr_of_mut!(SMALL_USED).cast(), core::ptr::addr_of_mut!(SMALL_PENDING).cast()),
        };
    }
    let (_, _, cache, words, _, _) = unsafe { PLACE };
    sex_pdx::serial_record!("[files.fs.memory] blocks={} cache_blocks={} map_words={} grown={}", blocks, cache, words, big.is_some() as u8);
}
fn memory() -> Memory<'static> {
    unsafe {
        let (cache, slots, n, words, used, pending) = PLACE;
        // Every slot is reset by mount/format; only one Fs holds these at a time.
        for i in 0..n { *slots.add(i) = Slot::EMPTY; }
        Memory {
            cache: core::slice::from_raw_parts_mut(cache, n),
            slots: core::slice::from_raw_parts_mut(slots, n),
            used: core::slice::from_raw_parts_mut(used, words),
            pending: core::slice::from_raw_parts_mut(pending, words),
            scratch: &mut *core::ptr::addr_of_mut!(SCRATCH),
        }
    }
}
fn bring_up() -> State {
    let sectors = DiskFs::diskfs_block_call(sex_pdx::BLOCK_CAPACITY, 0, 0, 0);
    let state = bring_up_on(sectors);
    // No SexOS filesystem here (a Windows RAID volume, say): its own volumes, read only.
    if matches!(state, State::Absent) && sectors >= DEVICE_MIN_SECTORS { crate::volumes::attach_device(sectors); }
    state
}
/// Smallest disk worth scanning for other systems' volumes: 1 MiB.
const DEVICE_MIN_SECTORS: u64 = 2048;
fn bring_up_on(sectors: u64) -> State {
    if sectors < w::MIN_SECTORS {
        sex_pdx::serial_record!("[files.fs.absent] sectors={}", sectors);
        return State::Absent;
    }
    let blocks = (sectors - w::BASE_LBA) / SECTORS;
    place(blocks);
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
        // Damaged: salvage what can be read into a new tree beside the old one (a
        // power cut during repair leaves the disk as it was), then use that.
        Err(Error::Corrupt) => {
            sex_pdx::serial_record!("[files.fs.repair.start] blocks={}", blocks);
            match sexfs::check::repair(Disk { blocks }, memory(), false) {
                Ok((fs, r)) => {
                    sex_pdx::serial_record!("[files.fs.repaired] generation={} bad_nodes={} from_older={} orphans={} dangling={} stray_blocks={} shared_blocks={} files={} dirs={}",
                        r.generation, r.bad_nodes, r.from_older, r.orphans, r.dangling_entries, r.stray_blocks, r.shared_blocks, r.files, r.dirs);
                    State::Ready(fs)
                }
                Err(e) => { sex_pdx::serial_record!("[files.fs.mount.failed] error={:?}", e); State::Absent }
            }
        }
        Err(e) => { sex_pdx::serial_record!("[files.fs.mount.failed] error={:?}", e); State::Absent }
    }
}
pub fn status(e: Error) -> u8 {
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
pub fn replies_pending() -> bool { (unsafe { (*core::ptr::addr_of!(REPLIES)).iter().any(|r| r.0 != 0) }) || crate::recording_runtime::pending() }
pub fn poll_replies() {
    let replies = unsafe { &mut *core::ptr::addr_of_mut!(REPLIES) };
    for entry in replies.iter_mut().filter(|r| r.0 != 0) {
        if sex_pdx::desktop_recording::ENABLED&&entry.0==sex_pdx::desktop_recording::TARGET{
            sex_pdx::serial_record!("[record.reply.unscoped.drop] target={}",entry.0);*entry=(0,0);continue;
        }
        if sex_pdx::pdx_reply(entry.0, entry.1) == 0 { *entry = (0, 0); }
    }
    crate::recording_runtime::poll();
}
/// Each program row's current run and the folder it may use. A run Shell never bound,
/// such as the boot image's own program, has no files at all.
const PROGRAM_ROWS: usize = sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Program);
const ROOT_MAX: usize = 64;
#[derive(Clone, Copy)]
struct Binding { task: u32, root: [u8; ROOT_MAX], len: usize, args: [u8; w::ARGS_MAX], args_len: usize, picked: [u8; PICKED_MAX], picked_len: usize }
/// Paths the person chose for a run in the picker, space separated, as `@/path` tokens.
const PICKED_MAX: usize = 1024;
/// Constructor only, never admission: the provider installs this after an exact
/// task/epoch check and actual typed bind-ACK delivery. No generic loader state.
fn recording_binding(task:u32)->Option<Binding>{
    if task==0 { return None; }
    const _:()=assert!(sex_pdx::desktop_recording::ROOT.len()<=ROOT_MAX);
    let mut root=[0;ROOT_MAX];let length=sex_pdx::desktop_recording::ROOT.len();
    root[..length].copy_from_slice(sex_pdx::desktop_recording::ROOT);
    Some(Binding{task,root,len:length,args:[0;w::ARGS_MAX],args_len:0,picked:[0;PICKED_MAX],picked_len:0})
}

pub(super) fn prepare_recording_root()->sex_pdx::desktop_recording::BindStatus {
    use sex_pdx::desktop_recording::{BindStatus as Status,ROOT};
    let state=unsafe{&mut *core::ptr::addr_of_mut!(STATE)};
    if matches!(state,State::Untried){*state=bring_up();}
    let State::Ready(fs)=state else{return Status::Unavailable;};
    if writes_pending(){commit_writes(fs);}
    if fs.has_changes(){return Status::Io;}
    match fs.open(ROOT){
        Ok(stat)=>if stat.kind==Kind::Dir{Status::Ok}else{Status::Denied},
        Err(Error::NotFound)=>{
            if fs.mkdir(ROOT,0).and_then(|_|fs.commit()).is_ok(){forget_stats(None);Status::Ok}
            else{let _=fs.rollback();Status::Io}
        },
        Err(_)=>Status::Io,
    }
}
pub(super) fn install_recording_binding(task:Option<u32>){
    let Some(index)=program_index(sex_pdx::desktop_recording::TARGET)else{return;};
    let binding=task.and_then(recording_binding);
    if let Some(cache)=client_cache(w::PROGRAM_INDEX_BASE+index){w::stat_cache::forget(cache,None);}
    unsafe{
        (*core::ptr::addr_of_mut!(BINDINGS))[index]=binding;
        (*core::ptr::addr_of_mut!(PICKS))[index]=None;
        READ_IO[w::PROGRAM_INDEX_BASE+index]=w::ReadIoStats::ZERO;
    }
}
/// One question to the person per program row: (ticket, mode, suggestion, answer).
#[derive(Clone, Copy)]
struct Pick { ticket: u32, mode: u8, hint: [u8; w::PICK_HINT_MAX], hint_len: usize, answer: Answer }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer { Waiting, Cancelled, Chosen }
static mut PICKS: [Option<Pick>; PROGRAM_ROWS] = [None; PROGRAM_ROWS];
static mut PICK_TICKET: u32 = 0;
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
    if sex_pdx::desktop_recording::ENABLED&&target==sex_pdx::desktop_recording::TARGET{return;}
    let Some(index) = program_index(target) else { return };
    unsafe { READ_IO[w::PROGRAM_INDEX_BASE + index] = w::ReadIoStats::ZERO; }
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
    if let Some(cache) = client_cache(w::PROGRAM_INDEX_BASE + index) { w::stat_cache::forget(cache, None); }
    bindings[index] = Some(Binding { task, root, len, args: args.0, args_len: core::mem::take(&mut args.1), picked: [0; PICKED_MAX], picked_len: 0 });
    unsafe { (*core::ptr::addr_of_mut!(PICKS))[index] = None; }
    sex_pdx::serial_record!("[files.program.bound] target={} task={} root={}", target, task, core::str::from_utf8(&root[..len]).unwrap_or("?"));
}
/// A volume under `/media` went away (ejected or pulled out): every run loses the grants
/// it held to `prefix` or anything inside it, so a stick that later mounts under the same
/// name is not reachable through them. Returns how many grants were taken back.
pub fn revoke_under(prefix: &[u8]) -> usize {
    fn under(token: &[u8], prefix: &[u8]) -> bool {
        token.len() > 1 && token[0] == b'@' && { let abs = &token[1..]; abs.starts_with(prefix) && (abs.len() == prefix.len() || abs[prefix.len()] == b'/') }
    }
    fn keep(buf: &mut [u8], len: &mut usize, prefix: &[u8]) -> usize {
        let mut out = [0u8; PICKED_MAX];
        let (mut n, mut gone) = (0usize, 0usize);
        for token in buf[..*len].split(|b| *b == b' ').filter(|t| !t.is_empty()) {
            if under(token, prefix) { gone += 1; continue; }
            if n > 0 { out[n] = b' '; n += 1; }
            out[n..n + token.len()].copy_from_slice(token);
            n += token.len();
        }
        buf[..n].copy_from_slice(&out[..n]);
        *len = n;
        gone
    }
    let bindings = unsafe { &mut *core::ptr::addr_of_mut!(BINDINGS) };
    let mut total = 0;
    for b in bindings.iter_mut().flatten() {
        total += keep(&mut b.args, &mut b.args_len, prefix) + keep(&mut b.picked, &mut b.picked_len, prefix);
    }
    // What any run was told about paths on that volume is stale now.
    forget_stats(None);
    sex_pdx::serial_record!("[files.media.revoked] prefix={} grants={}", core::str::from_utf8(prefix).unwrap_or("?"), total);
    total
}
/// A program's path inside its folder. `..` never climbs out: it is refused.
fn confine(binding: &Binding, path: &[u8], out: &mut [u8; w::PATH_BYTES]) -> Option<usize> {
    // `@/path` names a file or folder the person gave this run when starting it: that
    // file, or anything inside that folder (`@/` is the whole filesystem), and nothing else.
    if path.first() == Some(&b'@') {
        let granted = binding.args[..binding.args_len].split(|b| *b == b' ').chain(binding.picked[..binding.picked_len].split(|b| *b == b' ')).filter(|t| t.len() > 1 && t[0] == b'@').any(|token| {
            token == path || path.starts_with(token) && (token.ends_with(b"/") || path[token.len()] == b'/')
        });
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
/// The picker portal: questions from programs, answers from the run showing the picker.
fn picker(program: Option<usize>, r: w::Request, path: &[u8], data: &mut [u8]) -> (u8, u32) {
    let picks = unsafe { &mut *core::ptr::addr_of_mut!(PICKS) };
    let bindings = unsafe { &mut *core::ptr::addr_of_mut!(BINDINGS) };
    match r.kind {
        w::PICK => {
            let Some(k) = program else { return (w::DENIED, 0) };
            let mode = data[0];
            if mode > w::PICK_FOLDER { return (w::INVALID, 0); }
            let n = (r.length as usize).min(w::PICK_HINT_MAX);
            let mut hint = [0u8; w::PICK_HINT_MAX];
            // Only a name: a suggestion can't point anywhere.
            for (i, b) in data[1..1 + n].iter().enumerate() { hint[i] = if *b == b'/' || *b < 0x20 { b'_' } else { *b }; }
            let ticket = unsafe { PICK_TICKET = PICK_TICKET % 0xFF_FFFE + 1; PICK_TICKET };
            picks[k] = Some(Pick { ticket, mode, hint, hint_len: n, answer: Answer::Waiting });
            sex_pdx::serial_record!("[files.pick.asked] row={} mode={} ticket={}", k, mode, ticket);
            (w::OK, ticket)
        }
        w::PICKED => {
            let Some(k) = program else { return (w::DENIED, 0) };
            let Some(pick) = picks[k].filter(|p| p.ticket as u64 == r.offset) else { return (w::NOT_FOUND, 0) };
            match pick.answer {
                Answer::Waiting => (w::PENDING, 0),
                Answer::Cancelled => { picks[k] = None; (w::DENIED, 0) }
                Answer::Chosen => {
                    picks[k] = None;
                    // The newest grant is the last token.
                    let Some(b) = bindings[k].as_ref() else { return (w::DENIED, 0) };
                    let token = b.picked[..b.picked_len].rsplit(|c| *c == b' ').next().unwrap_or(b"");
                    data[..token.len()].copy_from_slice(token);
                    (w::OK, token.len() as u32)
                }
            }
        }
        // Only the person's own picker: Spindle, or a run they gave the whole filesystem.
        w::PICKS | w::ANSWER => {
            let whole = match program {
                None => true,
                Some(k) => bindings[k].is_some_and(|b| b.args[..b.args_len].split(|c| *c == b' ').any(|t| t == b"@/")),
            };
            if !whole { return (w::DENIED, 0); }
            if r.kind == w::PICKS {
                let mut used = 0;
                for (row, pick) in picks.iter().enumerate() {
                    let Some(p) = pick.filter(|p| p.answer == Answer::Waiting) else { continue };
                    let Some(b) = bindings[row].as_ref() else { continue };
                    let data_dir = sex_pdx::program_store::DATA;
                    let name = &b.root[data_dir.len().min(b.len)..b.len];
                    if used + 8 + name.len() + p.hint_len > 4000 { break; }
                    data[used] = row as u8; data[used + 1] = p.mode;
                    data[used + 2..used + 6].copy_from_slice(&p.ticket.to_le_bytes());
                    data[used + 6] = name.len() as u8;
                    data[used + 7..used + 7 + name.len()].copy_from_slice(name);
                    used += 7 + name.len();
                    data[used] = p.hint_len as u8;
                    data[used + 1..used + 1 + p.hint_len].copy_from_slice(&p.hint[..p.hint_len]);
                    used += 1 + p.hint_len;
                }
                return (w::OK, used as u32);
            }
            let row = r.offset as usize;
            let Some(pick) = picks.get_mut(row).and_then(|p| p.as_mut()).filter(|p| p.ticket as u64 == r.length && p.answer == Answer::Waiting) else { return (w::NOT_FOUND, 0) };
            if path.is_empty() { pick.answer = Answer::Cancelled; sex_pdx::serial_record!("[files.pick.cancelled] row={}", row); return (w::OK, 0); }
            // The chosen path: absolute, plain, and one the answering run itself holds.
            let abs = if path.first() == Some(&b'@') { &path[1..] } else { path };
            if abs.first() != Some(&b'/') || abs.split(|c| *c == b'/').any(|p| p == b".." || p == b".") || abs.contains(&b' ') || abs.contains(&0) { return (w::NAME, 0); }
            if let Some(k) = program {
                let b = bindings[k].unwrap();
                let mut out = [0u8; w::PATH_BYTES];
                let mut token = [0u8; w::PATH_BYTES + 1];
                token[0] = b'@'; token[1..1 + abs.len()].copy_from_slice(abs);
                if confine(&b, &token[..1 + abs.len()], &mut out).is_none() { return (w::DENIED, 0); }
            }
            let Some(target) = bindings[row].as_mut() else { return (w::NOT_FOUND, 0) };
            let sep = (target.picked_len > 0) as usize;
            if target.picked_len + sep + 1 + abs.len() > PICKED_MAX { return (w::NO_SPACE, 0); }
            if sep == 1 { target.picked[target.picked_len] = b' '; }
            let at = target.picked_len + sep;
            target.picked[at] = b'@';
            target.picked[at + 1..at + 1 + abs.len()].copy_from_slice(abs);
            target.picked_len = at + 1 + abs.len();
            pick.answer = Answer::Chosen;
            sex_pdx::serial_record!("[files.pick.granted] row={} path={}", row, core::str::from_utf8(abs).unwrap_or("?"));
            (w::OK, 0)
        }
        _ => (w::INVALID, 0),
    }
}
/// The row is running, and running the very run Shell bound: the kernel names its task.
/// The run holds `+packages`: Spindle put it in the run's arguments for `run +pkg`, and
/// only Spindle prepares runs, so a program cannot give it to itself.
fn holds_packages(binding: &Binding) -> bool {
    binding.args[..binding.args_len].split(|b| *b == b' ').any(|t| t == sex_pdx::package::GRANT_TOKEN)
}
/// A package request from a program holding `+packages`: the argument Files acts on, or
/// None (refused, logged). Trusting a publisher and the person's answers to grants stay
/// Spindle's; no downgrades; package files only from the program's own folder.
fn program_package(binding: &Binding, r: w::Request, arg: &[u8], out: &mut [u8; w::PATH_BYTES]) -> Option<usize> {
    use sex_pdx::package as p;
    let program = core::str::from_utf8(&binding.root[sex_pdx::program_store::DATA.len()..binding.len]).unwrap_or("?");
    let refuse = |why: &str| { sex_pdx::serial_record!("[files.pkg.denied] program={} op={} why={}", program, r.offset, why); None };
    match r.offset {
        p::INSTALL if r.length & p::ALLOW_DOWNGRADE != 0 => refuse("downgrade"),
        p::INSTALL | p::CHECK => match confine(binding, arg, out) { Some(n) => Some(n), None => refuse("path") },
        p::REMOVE | p::LIST | p::CATALOG | p::SEARCH | p::LOCATE | p::SOURCE => {
            if arg.len() > out.len() { return refuse("length"); }
            out[..arg.len()].copy_from_slice(arg);
            Some(arg.len())
        }
        p::TRUST | p::CONSENT => refuse("spindle_only"),
        _ => refuse("unknown"),
    }
}
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
        (Some(index), _) => {
            if !crate::spindle_history::live(caller) { return w::reply(r.seq, w::DENIED, 0); }
            // The desktop's terminal is up and talking: a system on trial passes.
            crate::updater::on_healthy();
            index
        }
        (None, Some(k)) => {
            let bound = unsafe { (*core::ptr::addr_of!(BINDINGS))[k] };
            // A program may eject (UNMOUNT) a volume its grant covers whole, checked with
            // its other paths below; mounting stays Spindle's.
            if !bound.is_some_and(|b| program_live(caller, b.task)) || matches!(r.kind, w::PREPARE | w::INSTALL | w::UPDATE | w::MOUNT) { return w::reply(r.seq, w::DENIED, 0); }
            // Packages only for a run the person started with `run +pkg` (see `program_package`).
            if r.kind == sex_pdx::package::KIND && !bound.is_some_and(|b| holds_packages(&b)) {
                sex_pdx::serial_record!("[files.pkg.denied] target={} op={} why=no_grant", caller, r.offset);
                return w::reply(r.seq, w::DENIED, 0);
            }
            w::PROGRAM_INDEX_BASE + k
        }
        _ => return w::reply(r.seq, w::DENIED, 0),
    };
    if r.kind == w::PING { return w::reply(r.seq, w::OK, 0); }
    let before = unsafe { (DEVICE_READ_CALLS, DEVICE_READ_BYTES) };
    let answer = handle_request(index, program, r);
    if r.kind == w::READ {
        unsafe {
            let own = &mut *core::ptr::addr_of_mut!(READ_IO).cast::<w::ReadIoStats>().add(index);
            own.requests = own.requests.saturating_add(1);
            own.device_calls = own.device_calls.saturating_add(DEVICE_READ_CALLS.saturating_sub(before.0));
            own.device_bytes = own.device_bytes.saturating_add(DEVICE_READ_BYTES.saturating_sub(before.1));
        }
    }
    answer
}
fn handle_request(index: usize, program: Option<usize>, r: w::Request) -> u64 {
    let Ok(view) = sex_pdx::memory_region_view(w::PROVIDER_SLOT_BASE + index as u64, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE) else { return w::reply(r.seq, w::IO, 0) };
    let region = unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) };
    if r.kind == w::READ_IO_STATS {
        if r.path_len != 0 || r.offset != 0 || r.length != 0 { return w::reply(r.seq, w::INVALID, 0); }
        unsafe { READ_IO[index] }.encode(&mut region[w::PATH_BYTES..]).unwrap();
        return w::reply(r.seq, w::OK, w::ReadIoStats::BYTES as u32);
    }
    // A client found now may have been absent when Files last looked: keep it, so its
    // cache is cleared with the others.
    if let Some(slot) = unsafe { (*core::ptr::addr_of_mut!(REGIONS)).get_mut(index) } { *slot = view.virtual_address; }
    let state = unsafe { &mut *core::ptr::addr_of_mut!(STATE) };
    if matches!(state, State::Untried) {
        *state = bring_up();
        // A restarted Files may have lost writes its clients' caches still describe.
        if matches!(state, State::Ready(_)) { forget_stats(None); }
    }
    if r.kind == w::INSTALL {
        // Works without a filesystem: a live system on another OS's disk has none.
        let path_len = (r.path_len as usize).min(w::PATH_BYTES);
        let mut path = [0u8; w::PATH_BYTES];
        path[..path_len].copy_from_slice(&region[..path_len]);
        let go = r.length & w::INSTALL_GO != 0;
        if go && writes_pending() { if let State::Ready(fs) = state { commit_writes(fs); } }
        let fs = match state { State::Ready(fs) => Some(fs), _ => None };
        let result = crate::installer::run(fs, &path[..path_len], r.offset, go, &mut region[w::PATH_BYTES..]);
        // Once the installer has written raw, the driver stops this system's writes and the
        // layout under the mounted filesystem may have changed: drop it until restart.
        if go && DiskFs::diskfs_block_call(sex_pdx::BLOCK_LAYOUT, 0, 0, 0) & 0xFF == sex_pdx::BLOCK_LAYOUT_LOCKED { *state = State::Absent; }
        return match result { Ok(n) => w::reply(r.seq, w::OK, n), Err(e) => w::reply(r.seq, status(e), 0) };
    }
    if r.kind == w::UPDATE {
        let State::Ready(fs) = state else { return w::reply(r.seq, w::UNAVAILABLE, 0) };
        let path_len = (r.path_len as usize).min(w::PATH_BYTES);
        let mut path = [0u8; w::PATH_BYTES];
        path[..path_len].copy_from_slice(&region[..path_len]);
        if writes_pending() { commit_writes(fs); }
        let result = crate::updater::run(fs, &path[..path_len], &mut region[w::PATH_BYTES..]);
        return match result { Ok((st, n)) => w::reply(r.seq, st, n), Err(e) => w::reply(r.seq, status(e), 0) };
    }
    if r.kind == sex_pdx::package::KIND {
        let State::Ready(fs) = state else { return w::reply(r.seq, w::UNAVAILABLE, 0) };
        let path_len = (r.path_len as usize).min(w::PATH_BYTES);
        let mut path = [0u8; w::PATH_BYTES];
        path[..path_len].copy_from_slice(&region[..path_len]);
        forget_stats(None);
        // Others' writes are saved first, so undoing a refused package undoes only it.
        if writes_pending() { commit_writes(fs); }
        // A program's request acts inside its own folder: Files is told which.
        let mut own = [0u8; w::PATH_BYTES];
        let binding = program.and_then(|k| unsafe { (*core::ptr::addr_of!(BINDINGS))[k] });
        let arg = match &binding {
            None => &path[..path_len],
            Some(b) => match program_package(b, r, &path[..path_len], &mut own) { Some(n) => &own[..n], None => return w::reply(r.seq, w::DENIED, 0) },
        };
        let folder = binding.as_ref().map(|b| &b.root[..b.len]);
        let (st, n) = crate::packages::handle(fs, r.offset, arg, r.length, &mut region[w::PATH_BYTES..w::CACHE_AT], folder);
        return w::reply(r.seq, st, n);
    }
    let fs = match state { State::Ready(fs) => Some(fs), _ => None };
    let mut path = [0u8; w::PATH_BYTES];
    let mut path_len = r.path_len as usize;
    path[..path_len].copy_from_slice(&region[..path_len]);
    if matches!(r.kind, w::PICK | w::PICKED | w::PICKS | w::ANSWER) {
        let (status, value) = picker(program, r, &path[..path_len], &mut region[w::PATH_BYTES..]);
        return w::reply(r.seq, status, value);
    }
    if matches!(r.kind, sex_pdx::home_lock::SETUP | sex_pdx::home_lock::UNLOCK | sex_pdx::home_lock::LOCK | sex_pdx::home_lock::STATE | sex_pdx::home_lock::CHANGE) {
        // Only Spindle, Shell or a run holding the whole filesystem.
        if let Some(k) = program {
            let b = unsafe { (*core::ptr::addr_of!(BINDINGS))[k] }.unwrap();
            if !b.args[..b.args_len].split(|c| *c == b' ').any(|t| t == b"@/") { return w::reply(r.seq, w::DENIED, 0); }
        }
        let Some(fs) = fs else { return w::reply(r.seq, w::UNAVAILABLE, 0) };
        if writes_pending() { commit_writes(fs); }
        let n = (r.length as usize).min(512);
        let mut secret = [0u8; 512];
        secret[..n].copy_from_slice(&region[w::PATH_BYTES..w::PATH_BYTES + n]);
        // The password doesn't stay in the client's region either.
        region[w::PATH_BYTES..w::PATH_BYTES + n].fill(0);
        forget_stats(None);
        let result = crate::home::control(fs, r.kind, r.offset, &secret[..n]);
        secret.fill(0);
        return match result { Ok(v) => w::reply(r.seq, w::OK, v), Err(st) => w::reply(r.seq, st, 0) };
    }
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
    // The client's spelling, for its cache; `path` is what it resolves to.
    let mut spelled = [0u8; w::PATH_BYTES];
    let spelled_len = (r.path_len as usize).min(w::PATH_BYTES);
    spelled[..spelled_len].copy_from_slice(&region[..spelled_len]);
    let (head, cache) = region.split_at_mut(w::CACHE_AT);
    let data = &mut head[w::PATH_BYTES..];
    let Some(fs) = fs else {
        // No SexOS filesystem on this disk: only its own volumes under /media, read only.
        let path = &path[..path_len];
        if !crate::volumes::owns(path) { return w::reply(r.seq, w::UNAVAILABLE, 0); }
        if r.kind == w::RENAME || r.kind == w::MOUNT || r.kind == w::UNMOUNT { return w::reply(r.seq, w::READ_ONLY, 0); }
        return match crate::volumes::run(None, r, path, data) { Ok(value) => w::reply(r.seq, w::OK, value), Err(status) => w::reply(r.seq, status, 0) };
    };
    // Reads change nothing; a write fails before it changes anything (sexfs checks
    // first, and a device error poisons the filesystem). Every other change commits
    // waiting writes first, so its own rollback can only undo itself.
    let read_only = matches!(r.kind, w::STAT | w::LIST | w::READ | w::STATFS);
    let batched = r.kind == w::WRITE;
    if !read_only && !batched && writes_pending() {
        commit_writes(fs);
        if fs.has_changes() { return w::reply(r.seq, w::IO, 0); }
    }
    let path = &path[..path_len];
    // Before anything changes: forget what clients were told about it.
    match r.kind {
        w::STAT | w::LIST | w::READ | w::STATFS | w::SYNC | w::PREPARE => {}
        // By file id, so every spelling of the path (`a//b`, `a/b/`) is cleared.
        w::WRITE | w::TRUNCATE | w::CREATE | w::MKDIR => { if let Ok(st) = fs.open(path) { forget_stats(Some(st.id)); } }
        _ => forget_stats(None),
    }
    // Other systems' volumes, under /media.
    let rename_halves = (r.kind == w::RENAME).then(|| path.iter().position(|b| *b == 0)).flatten().map(|i| (&path[..i], &path[i + 1..]));
    let media = match (r.kind, rename_halves) {
        (w::MOUNT, _) => Some(crate::volumes::mount(fs, path, data)),
        (w::UNMOUNT, _) => Some(crate::volumes::unmount(fs, path)),
        (w::RENAME, Some((a, b))) if crate::volumes::owns(a) || crate::volumes::owns(b) => Some(crate::volumes::rename(Some(&mut *fs), a, b)),
        (w::RENAME, _) => None,
        _ if crate::volumes::owns(path) => Some(crate::volumes::run(Some(&mut *fs), r, path, data)),
        _ => None,
    };
    if let Some(result) = media {
        return match result { Ok(value) => w::reply(r.seq, w::OK, value), Err(status) => w::reply(r.seq, status, 0) };
    }
    // The encrypted home: its own filesystem, reached only once unlocked.
    let home = match (r.kind, rename_halves) {
        (w::RENAME, Some((a, b))) if crate::home::crosses(a, b) => Some(Err(w::OTHER_DISK)),
        (w::RENAME, Some((a, _))) if crate::home::owns(a) => Some(crate::home::run(&mut *fs, r, path, data)),
        (w::RENAME, _) => None,
        (w::PREPARE | w::MOUNT | w::UNMOUNT, _) if crate::home::owns(path) => Some(Err(w::UNSUPPORTED)),
        _ if crate::home::owns(path) => Some(crate::home::run(&mut *fs, r, path, data)),
        _ => None,
    };
    if let Some(result) = home {
        if !read_only { forget_stats(None); }
        return match result { Ok(value) => w::reply(r.seq, w::OK, value), Err(status) => w::reply(r.seq, status, 0) };
    }
    // Its sealed file changes only through it.
    if !read_only && (path == sex_pdx::home_lock::IMAGE || rename_halves.is_some_and(|(_, b)| b == sex_pdx::home_lock::IMAGE)) { return w::reply(r.seq, w::DENIED, 0); }
    // A mounted image changes only through its volume.
    if matches!(r.kind, w::WRITE | w::TRUNCATE | w::REMOVE) && fs.open(path).is_ok_and(|st| crate::volumes::busy(st.id)) {
        return w::reply(r.seq, w::DENIED, 0);
    }
    if r.kind == w::SYNC { crate::volumes::sync_all(fs); crate::home::sync(fs); }
    let result = run(fs, r, path, data);
    if r.kind == w::STAT && result.is_ok() {
        let word = |i: usize| u64::from_le_bytes(data[i * 8..i * 8 + 8].try_into().unwrap());
        w::stat_cache::fill(cache, &spelled[..spelled_len], word(3), word(0) != 0, word(1), word(2), word(3));
    }
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
            if !read_only && !batched && !matches!(e, Error::Io | Error::Corrupt | Error::Poisoned) { let _ = fs.rollback(); forget_stats(None); }
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
fn read_program(path: &[u8], image: &mut [u8], remember: bool) -> Program {
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
    if remember { loaded(path); }
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
    read_program(&path[..at + name.len()], image, true)
}
/// Read a bare image without consuming prepared tickets, arguments or the generic
/// loaded-name latch. Source identity is independently enforced by the kernel.
pub fn read_named_unbound(name:&[u8], image:&mut [u8])->Program {
    use sex_pdx::program_store as store;
    if name.is_empty() || name.len()>store::NAME_BYTES || name.iter().any(|byte|matches!(*byte,b'/'|0)) {
        return Program::Failed(store::NOT_A_PROGRAM);
    }
    let mut path=[0u8;32];let at=store::BIN.len();
    path[..at].copy_from_slice(store::BIN);path[at..at+name.len()].copy_from_slice(name);
    read_program(&path[..at+name.len()],image,false)
}
/// The prepared path, once. A stale ticket finds nothing. When the path is
/// `/bin/NAME` and no such file exists, the packed NAME comes back too, so the disk
/// program store can serve it with the same arguments.
pub fn read_ticket(ticket: u64, image: &mut [u8]) -> (Program, Option<[u64; 2]>) {
    unsafe { (*core::ptr::addr_of_mut!(LOADED)).1 = 0; }
    let prepared = unsafe { &mut *core::ptr::addr_of_mut!(PREPARED) };
    if prepared.1 == 0 || prepared.2 as u64 != ticket { return (Program::Absent, None); }
    let len = core::mem::take(&mut prepared.1);
    unsafe { *core::ptr::addr_of_mut!(LOADED_ARGS) = *core::ptr::addr_of!(PREPARED_ARGS); }
    let path = &prepared.0[..len];
    let found = read_program(path, image, true);
    let store_name = path.strip_prefix(sex_pdx::program_store::BIN)
        .filter(|name| !name.contains(&b'/'))
        .and_then(sex_pdx::program_store::pack_name);
    match found { Program::Absent => (Program::Absent, store_name), other => (other, None) }
}
fn run(fs: &mut Fs<'static, Disk>, r: w::Request, path: &[u8], data: &mut [u8]) -> Result<u32, Error> {
    // Test builds only: a path that makes Files fall over, to show it comes back.
    if option_env!("SEXOS_FILES_CRASH_PROBE").is_some() && path == b"/crash.probe" {
        sex_pdx::serial_record!("[files.crash.probe]");
        unsafe { core::ptr::write_volatile(8 as *mut u8, 1) };
    }
    if r.kind == w::PREPARE { return prepare(fs, r, path, data); }
    run_on(fs, r, path, data)
}
/// The requests every SexOS filesystem answers alike: the system disk's, and home's.
pub fn run_on<D: sexfs::Device>(fs: &mut Fs<'static, D>, r: w::Request, path: &[u8], data: &mut [u8]) -> Result<u32, Error> {
    // Modification times: Unix seconds from the civil clock (0 if it is unknown).
    let now = sex_pdx::civil_time_ms().map(|ms| ms / 1000).unwrap_or(0);
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
        // Both go through private memory: the client can change its region at any time, and
        // checksums must be over the bytes we keep, not bytes it might still be changing.
        w::READ => {
            let st = fs.open(path)?;
            let own = unsafe { &mut *core::ptr::addr_of_mut!(OWN) };
            let want = (r.length as usize).min(data.len()).min(own.len());
            let n = fs.read(st.id, r.offset, &mut own[..want])?;
            data[..n].copy_from_slice(&own[..n]);
            Ok(n as u32)
        }
        w::WRITE => {
            let total = r.length as usize;
            let own = unsafe { &mut *core::ptr::addr_of_mut!(OWN) };
            if total > data.len() || total > own.len() { return Err(Error::TooBig); }
            let st = fs.open(path)?;
            own[..total].copy_from_slice(&data[..total]);
            // sexfs ends the write short after progress, as Linux does, so the counted
            // prefix commits with the rest.
            Ok(fs.write(st.id, r.offset, &own[..total], now)? as u32)
        }
        w::TRUNCATE => { let st = fs.open(path)?; fs.truncate(st.id, r.offset, now)?; fs.commit()?; Ok(0) }
        w::SYNC => { fs.commit()?; Ok(0) }
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
fn prepare(fs: &mut Fs<'static, Disk>, r: w::Request, path: &[u8], data: &mut [u8]) -> Result<u32, Error> {
    {
        {
            // `/bin/NAME` with no such file may still be the disk program store's NAME:
            // the load looks there, and refuses then if neither has it.
            let store_name = path.strip_prefix(sex_pdx::program_store::BIN)
                .is_some_and(|name| !name.contains(&b'/') && sex_pdx::program_store::pack_name(name).is_some());
            match fs.open(path) {
                Ok(st) if st.kind == Kind::Dir => return Err(Error::IsDir),
                Ok(_) => {}
                Err(Error::NotFound) if store_name => {}
                Err(e) => return Err(e),
            }
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
            // A program a package installed: what it asks for and what the person said.
            let note = path.strip_prefix(sex_pdx::program_store::BIN).and_then(|name| crate::packages::note(fs, name));
            match note { Some(n) => n.encode(data), None => data[..sex_pdx::package::NOTE_BYTES].fill(0) }
            Ok(ticket)
        }
    }
}
