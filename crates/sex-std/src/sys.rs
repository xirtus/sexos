// SPDX-License-Identifier: GPL-3.0-only
//! The calls Rust's std makes on this target (Hermit's library interface), answered
//! inside the program. Errors are returned as negative error numbers, as std expects.
//!
//! `SystemTime` is the kernel's civil time (UTC from the machine's clock, carried
//! forward by its millisecond tick); on a machine without one it falls back to the
//! time this crate was built plus the time since boot. `Instant` is the kernel's tick.
#![allow(clippy::missing_safety_doc)]
use crate::{args, console, net, path};
use core::ffi::{c_char, CStr};
use sex_app::files::direct as files;
use sex_pdx::fs as w;
use std::os::hermit::hermit_abi::{self as abi, errno, dirent64, stat, timespec, Tid};
use std::sync::{Arc, Mutex};

// ---- start and end -------------------------------------------------------------------

extern "C" {
    /// std's own start: sets up `env` and runs `main`.
    fn runtime_entry(argc: i32, argv: *const *const c_char, env: *const *const c_char) -> !;
}

/// Where the program starts: its thread-local block first (std keeps its state there),
/// then the words it was started with, then std.
#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    if !sex_app::tls::init() { sex_pdx::task_finish(3); }
    let mut line = [0u8; w::ARGS_MAX];
    let n = files::args(&mut line);
    let mut words = vec![b"program".to_vec()];
    words.extend(args::split(&line[..n]));
    let owned: Vec<std::ffi::CString> = words.into_iter().map(|w| std::ffi::CString::new(w).unwrap_or_default()).collect();
    let mut argv: Vec<*const c_char> = owned.iter().map(|s| s.as_ptr()).collect();
    argv.push(core::ptr::null());
    let env: [*const c_char; 1] = [core::ptr::null()];
    let argc = owned.len() as i32;
    // Both live for the whole run: std keeps pointers into them.
    std::mem::forget(owned);
    let argv = argv.leak();
    unsafe { runtime_entry(argc, argv.as_ptr(), Box::leak(Box::new(env)).as_ptr()) }
}

#[no_mangle]
pub extern "C" fn sys_exit(status: i32) -> ! {
    let me = sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT).map_or((0, 0), |i| (i.target_pd, i.task_id));
    sex_pdx::serial_record!("[std.exit] target={} task={} status={}", me.0, me.1, status);
    console::finish(&if status == 0 { "Finished".to_string() } else { format!("Finished with status {status}") });
    sex_pdx::task_finish(status as u32 as u64)
}

#[no_mangle]
pub extern "C" fn sys_abort() -> ! {
    sex_pdx::serial_record!("[std.abort]");
    console::finish("Stopped: the program failed");
    sex_pdx::task_finish(134)
}

// ---- memory ----------------------------------------------------------------------------

static HEAP: sex_app::heap::Heap = sex_app::heap::Heap::new(sex_app::heap::Window::new());

fn layout(size: usize, align: usize) -> Option<std::alloc::Layout> { std::alloc::Layout::from_size_align(size, align).ok() }

#[no_mangle]
pub extern "C" fn sys_malloc(size: usize, align: usize) -> *mut u8 {
    use std::alloc::GlobalAlloc;
    layout(size, align).map_or(core::ptr::null_mut(), |l| unsafe { HEAP.alloc(l) })
}
#[no_mangle]
pub extern "C" fn sys_alloc(size: usize, align: usize) -> *mut u8 { sys_malloc(size, align) }
#[no_mangle]
pub extern "C" fn sys_alloc_zeroed(size: usize, align: usize) -> *mut u8 {
    use std::alloc::GlobalAlloc;
    layout(size, align).map_or(core::ptr::null_mut(), |l| unsafe { HEAP.alloc_zeroed(l) })
}
#[no_mangle]
pub unsafe extern "C" fn sys_realloc(ptr: *mut u8, size: usize, align: usize, new_size: usize) -> *mut u8 {
    use std::alloc::GlobalAlloc;
    layout(size, align).map_or(core::ptr::null_mut(), |l| unsafe { HEAP.realloc(ptr, l, new_size) })
}
#[no_mangle]
pub unsafe extern "C" fn sys_free(ptr: *mut u8, size: usize, align: usize) {
    use std::alloc::GlobalAlloc;
    if let Some(l) = layout(size, align) { unsafe { HEAP.dealloc(ptr, l) } }
}
#[no_mangle]
pub unsafe extern "C" fn sys_dealloc(ptr: *mut u8, size: usize, align: usize) { unsafe { sys_free(ptr, size, align) } }

// ---- time, entropy, cores ----------------------------------------------------------------

const BUILT: i64 = match i64::from_str_radix(env!("SEX_STD_BUILT"), 10) { Ok(v) => v, Err(_) => 0 };

#[no_mangle]
pub unsafe extern "C" fn sys_clock_gettime(clock: abi::clockid_t, tp: *mut timespec) -> i32 {
    if tp.is_null() { return -errno::EINVAL; }
    let ms = sex_pdx::get_ticks() as i64;
    let (sec, nsec) = match clock {
        abi::CLOCK_MONOTONIC => (ms / 1000, (ms % 1000) * 1_000_000),
        abi::CLOCK_REALTIME => match sex_pdx::civil_time_ms() {
            Ok(utc) => ((utc / 1000) as i64, ((utc % 1000) * 1_000_000) as i64),
            Err(_) => (BUILT + ms / 1000, (ms % 1000) * 1_000_000),
        },
        _ => return -errno::EINVAL,
    };
    unsafe { *tp = timespec { tv_sec: sec, tv_nsec: nsec as _ }; }
    0
}

fn sleep_ms(ms: u64) {
    static NEVER: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    let end = sex_pdx::get_ticks().saturating_add(ms);
    loop {
        let now = sex_pdx::get_ticks();
        if now >= end { return; }
        sex_pdx::program_threads::wait_for(NEVER.as_ptr(), 0, end - now);
    }
}

#[no_mangle]
pub extern "C" fn sys_usleep(usecs: u64) {
    if usecs < 1000 { sex_pdx::sys_yield(); } else { sleep_ms(usecs.div_ceil(1000)); }
}
#[no_mangle]
pub unsafe extern "C" fn sys_nanosleep(req: *const timespec) -> i32 {
    if req.is_null() { return -errno::EINVAL; }
    let t = unsafe { *req };
    sys_usleep((t.tv_sec as u64).saturating_mul(1_000_000).saturating_add(t.tv_nsec as u64 / 1000));
    0
}
#[no_mangle]
pub extern "C" fn sys_yield() { sex_pdx::sys_yield(); }

#[no_mangle]
pub extern "C" fn sys_available_parallelism() -> usize { sex_app::thread::cores().max(1) }
#[no_mangle]
pub extern "C" fn sys_get_processor_count() -> usize { sys_available_parallelism() }
#[no_mangle]
pub extern "C" fn sys_getpagesize() -> i32 { 4096 }
#[no_mangle]
pub extern "C" fn sys_getpid() -> abi::pid_t { sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT).map_or(1, |i| i.target_pd as i32) }

/// Random bytes for `HashMap` keys and `rand`'s seed: the CPU's generator where it has
/// one, otherwise a mix of the time stamp counter. Not for keys or secrets.
#[no_mangle]
pub unsafe extern "C" fn sys_read_entropy(buf: *mut u8, len: usize, _flags: u32) -> isize {
    let out = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    let has_rdrand = core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0;
    let mut state = unsafe { core::arch::x86_64::_rdtsc() } ^ 0x9E37_79B9_7F4A_7C15;
    for chunk in out.chunks_mut(8) {
        let mut v = 0u64;
        let mut ok = false;
        if has_rdrand {
            for _ in 0..10 { if rdrand(&mut v) { ok = true; break; } }
        }
        if !ok {
            state = state.wrapping_add(unsafe { core::arch::x86_64::_rdtsc() }).wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            v = z ^ (z >> 31);
        }
        chunk.copy_from_slice(&v.to_le_bytes()[..chunk.len()]);
    }
    len as isize
}
fn rdrand(v: &mut u64) -> bool {
    let ok: u8;
    unsafe { core::arch::asm!("rdrand {v}", "setc {ok}", v = out(reg) *v, ok = out(reg_byte) ok, options(nomem, nostack)) };
    ok != 0
}

// ---- threads and waiting -------------------------------------------------------------------

/// Started threads by id; their stacks live until they are joined.
static THREADS: Mutex<Vec<(Tid, sex_app::thread::JoinHandle)>> = Mutex::new(Vec::new());
static NEXT_TID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1);

#[no_mangle]
pub extern "C" fn sys_spawn2(func: extern "C" fn(usize), arg: usize, _prio: u8, stack_size: usize, _core: isize) -> Tid {
    let job = move || func(arg);
    match sex_app::thread::spawn_with_stack(stack_size.max(64 * 1024), job) {
        Ok(handle) => {
            let tid = NEXT_TID.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1;
            THREADS.lock().unwrap_or_else(|e| e.into_inner()).push((tid, handle));
            tid
        }
        Err(_) => 0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn sys_spawn(id: *mut Tid, func: extern "C" fn(usize), arg: usize, prio: u8, core: isize) -> i32 {
    let tid = sys_spawn2(func, arg, prio, sex_app::thread::STACK_BYTES * 4, core);
    if tid == 0 { return -errno::EAGAIN; }
    if !id.is_null() { unsafe { *id = tid } }
    0
}
#[no_mangle]
pub extern "C" fn sys_join(id: Tid) -> i32 {
    let handle = {
        let mut all = THREADS.lock().unwrap_or_else(|e| e.into_inner());
        all.iter().position(|(t, _)| *t == id).map(|i| all.swap_remove(i).1)
    };
    match handle { Some(h) => { h.join(); 0 } None => -errno::ESRCH }
}

#[no_mangle]
pub unsafe extern "C" fn sys_futex_wait(address: *mut u32, expected: u32, timeout: *const timespec, flags: u32) -> i32 {
    if address.is_null() || flags & !abi::FUTEX_RELATIVE_TIMEOUT != 0 { return -errno::EINVAL; }
    let start = sex_pdx::get_ticks();
    let ms = if timeout.is_null() { 0 } else {
        let t = unsafe { *timeout };
        if t.tv_sec < 0 || t.tv_nsec < 0 { return -errno::EINVAL; }
        let mut ms = (t.tv_sec as u64).saturating_mul(1000).saturating_add((t.tv_nsec as u64).div_ceil(1_000_000));
        if flags & abi::FUTEX_RELATIVE_TIMEOUT == 0 { ms = ms.saturating_sub(start); }
        if ms == 0 { return -errno::ETIMEDOUT; }
        ms
    };
    if !sex_pdx::program_threads::wait_for(address, expected, ms) { return -errno::EAGAIN; }
    if ms != 0 && sex_pdx::get_ticks().saturating_sub(start) >= ms && unsafe { core::ptr::read_volatile(address) } == expected {
        return -errno::ETIMEDOUT;
    }
    0
}
#[no_mangle]
pub extern "C" fn sys_futex_wake(address: *mut u32, count: i32) -> i32 {
    if address.is_null() || count < 0 { return -errno::EINVAL; }
    let n = if count == i32::MAX { u32::MAX as u64 } else { count as u64 };
    sex_pdx::program_threads::wake(address, n).min(i32::MAX as u64) as i32
}

// ---- descriptors ----------------------------------------------------------------------------

enum Open {
    Console(usize),
    /// Shared by a descriptor and its duplicates, as one open file is.
    File(Arc<OpenFile>),
    /// A socket, shared by its duplicates.
    Socket(Arc<net::Sock>),
    Dir { entries: Vec<files::Entry>, given: bool },
}

struct OpenFile {
    path: Vec<u8>,
    read: bool,
    write: bool,
    append: bool,
    /// The position, held from choosing an offset until the read or write has moved
    /// it, so two tasks on one file never use the same bytes.
    pos: Mutex<u64>,
}
impl OpenFile {
    fn pos(&self) -> std::sync::MutexGuard<'_, u64> { self.pos.lock().unwrap_or_else(|e| e.into_inner()) }
}
fn file(fd: i32) -> Result<Arc<OpenFile>, i32> {
    match table().get(fd as usize) {
        Some(Some(Open::File(f))) => Ok(f.clone()),
        Some(Some(_)) => Err(-errno::EINVAL),
        _ => Err(-errno::EBADF),
    }
}

/// Open descriptors; 0, 1 and 2 are the console.
static TABLE: Mutex<Vec<Option<Open>>> = Mutex::new(Vec::new());

fn table() -> std::sync::MutexGuard<'static, Vec<Option<Open>>> {
    let mut t = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    if t.is_empty() { t.extend([Some(Open::Console(0)), Some(Open::Console(1)), Some(Open::Console(2))]); }
    t
}

fn insert(open: Open) -> i32 {
    let mut t = table();
    match t.iter().position(Option::is_none) {
        Some(i) => { t[i] = Some(open); i as i32 }
        None => { t.push(Some(open)); (t.len() - 1) as i32 }
    }
}

/// Files' answer as an error number.
fn errno_of(e: sex_app::files::Error) -> i32 {
    -match e {
        w::NOT_FOUND => errno::ENOENT, w::EXISTS => errno::EEXIST, w::NOT_DIR => errno::ENOTDIR,
        w::IS_DIR => errno::EISDIR, w::NOT_EMPTY => errno::ENOTEMPTY, w::NAME => errno::ENAMETOOLONG,
        w::NO_SPACE => errno::ENOSPC, w::DENIED => errno::EACCES, w::INVALID => errno::EINVAL,
        _ => errno::EIO,
    }
}

unsafe fn resolve(name: *const c_char) -> Result<Vec<u8>, i32> {
    if name.is_null() { return Err(-errno::EFAULT); }
    let bytes = unsafe { CStr::from_ptr(name) }.to_bytes();
    path::resolve(bytes).map_err(|e| match e { path::Refused::AboveFolder => -errno::EACCES, path::Refused::NotUtf8OrNul => -errno::EINVAL })
}

fn fill_stat(st: &files::Stat, out: *mut stat) {
    let mut s: stat = unsafe { core::mem::zeroed() };
    s.st_ino = st.id;
    s.st_nlink = 1;
    s.st_mode = if st.dir { abi::S_IFDIR | 0o755 } else { abi::S_IFREG | 0o644 };
    s.st_size = st.size as i64;
    s.st_blksize = 4096;
    s.st_blocks = st.size.div_ceil(512) as i64;
    unsafe { *out = s; }
}

#[no_mangle]
pub unsafe extern "C" fn sys_open(name: *const c_char, flags: i32, _mode: i32) -> i32 {
    let p = match unsafe { resolve(name) } { Ok(p) => p, Err(e) => return e };
    let access = flags & 0o3;
    let (read, write) = (access != abi::O_WRONLY, access != abi::O_RDONLY);
    let found = files::stat(&p);
    if flags & abi::O_DIRECTORY != 0 {
        return match found {
            Ok(st) if st.dir => match files::list(&p) { Ok(entries) => insert(Open::Dir { entries, given: false }), Err(e) => errno_of(e) },
            Ok(_) => -errno::ENOTDIR,
            Err(e) => errno_of(e),
        };
    }
    match found {
        Ok(_) if flags & abi::O_CREAT != 0 && flags & abi::O_EXCL != 0 => return -errno::EEXIST,
        Ok(st) if st.dir => return if write { -errno::EISDIR } else { -errno::EISDIR },
        Ok(_) => if flags & abi::O_TRUNC != 0 && write { if let Err(e) = files::truncate(&p, 0) { return errno_of(e); } },
        Err(w::NOT_FOUND) if flags & abi::O_CREAT != 0 => if let Err(e) = files::create(&p) { return errno_of(e); },
        Err(e) => return errno_of(e),
    }
    insert(Open::File(Arc::new(OpenFile { path: p, pos: Mutex::new(0), read, write, append: flags & abi::O_APPEND != 0 })))
}

#[no_mangle]
pub unsafe extern "C" fn sys_opendir(name: *const c_char) -> i32 { unsafe { sys_open(name, abi::O_RDONLY | abi::O_DIRECTORY, 0) } }

#[no_mangle]
pub extern "C" fn sys_close(fd: i32) -> i32 {
    let mut t = table();
    match t.get_mut(fd as usize) {
        Some(slot @ Some(_)) => { *slot = None; 0 }
        _ => -errno::EBADF,
    }
}

#[no_mangle]
pub unsafe extern "C" fn sys_read(fd: i32, buf: *mut u8, len: usize) -> isize {
    let out = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    let f = {
        let t = table();
        match t.get(fd as usize) {
            Some(Some(Open::Console(0))) => None,
            Some(Some(Open::File(f))) if f.read => Some(f.clone()),
            Some(Some(Open::Socket(s))) => { let s = s.clone(); drop(t); return net::recv(&s, out, false); }
            _ => return -errno::EBADF as isize,
        }
    };
    let Some(f) = f else { return console::read(out) as isize };
    let mut pos = f.pos();
    match files::read_at(&f.path, *pos, out) {
        Ok(n) => { *pos += n as u64; n as isize }
        Err(e) => errno_of(e) as isize,
    }
}

#[no_mangle]
pub unsafe extern "C" fn sys_write(fd: i32, buf: *const u8, len: usize) -> isize {
    let data = unsafe { core::slice::from_raw_parts(buf, len) };
    let f = {
        let t = table();
        match t.get(fd as usize) {
            Some(Some(Open::Console(n @ (1 | 2)))) => { let n = *n; drop(t); console::write(n - 1, data); return len as isize; }
            Some(Some(Open::File(f))) if f.write => f.clone(),
            Some(Some(Open::Socket(s))) => { let s = s.clone(); drop(t); return net::send(&s, data); }
            _ => return -errno::EBADF as isize,
        }
    };
    // Appends from two descriptors of one file still race on the end; within one open
    // file the position lock orders them.
    let mut pos = f.pos();
    let at = if f.append { match files::stat(&f.path) { Ok(st) => st.size, Err(e) => return errno_of(e) as isize } } else { *pos };
    match files::write_at(&f.path, at, data) {
        Ok(0) if !data.is_empty() => -errno::ENOSPC as isize,
        Ok(n) => { *pos = at + n as u64; n as isize }
        Err(e) => errno_of(e) as isize,
    }
}

#[no_mangle]
pub unsafe extern "C" fn sys_readv(fd: i32, iov: *const abi::iovec, count: usize) -> isize {
    let mut total = 0isize;
    for v in unsafe { core::slice::from_raw_parts(iov, count) } {
        let n = unsafe { sys_read(fd, v.iov_base as *mut u8, v.iov_len) };
        if n < 0 { return if total > 0 { total } else { n }; }
        total += n;
        if (n as usize) < v.iov_len { break; }
    }
    total
}
#[no_mangle]
pub unsafe extern "C" fn sys_writev(fd: i32, iov: *const abi::iovec, count: usize) -> isize {
    let mut total = 0isize;
    for v in unsafe { core::slice::from_raw_parts(iov, count) } {
        let n = unsafe { sys_write(fd, v.iov_base as *const u8, v.iov_len) };
        if n < 0 { return if total > 0 { total } else { n }; }
        total += n;
        if (n as usize) < v.iov_len { break; }
    }
    total
}

#[no_mangle]
pub extern "C" fn sys_lseek(fd: i32, offset: isize, whence: i32) -> isize {
    let f = match file(fd) { Ok(f) => f, Err(e) => return if e == -errno::EINVAL { -errno::ESPIPE as isize } else { e as isize } };
    let mut pos = f.pos();
    let base = match whence {
        abi::SEEK_SET => 0i64,
        abi::SEEK_CUR => *pos as i64,
        abi::SEEK_END => match files::stat(&f.path) { Ok(st) => st.size as i64, Err(e) => return errno_of(e) as isize },
        _ => return -errno::EINVAL as isize,
    };
    let to = base + offset as i64;
    if to < 0 { return -errno::EINVAL as isize; }
    *pos = to as u64;
    to as isize
}

#[no_mangle]
pub unsafe extern "C" fn sys_fstat(fd: i32, out: *mut stat) -> i32 {
    let path = match table().get(fd as usize) {
        Some(Some(Open::File(f))) => f.path.clone(),
        Some(Some(Open::Console(_))) => {
            let mut s: stat = unsafe { core::mem::zeroed() };
            s.st_mode = abi::S_IFCHR | 0o620;
            unsafe { *out = s; }
            return 0;
        }
        Some(Some(Open::Dir { .. })) => { fill_stat(&files::Stat { dir: true, ..Default::default() }, out); return 0; }
        Some(Some(Open::Socket(_))) => {
            let mut s: stat = unsafe { core::mem::zeroed() };
            s.st_mode = abi::S_IFSOCK | 0o777;
            unsafe { *out = s; }
            return 0;
        }
        _ => return -errno::EBADF,
    };
    match files::stat(&path) { Ok(st) => { fill_stat(&st, out); 0 } Err(e) => errno_of(e) }
}
#[no_mangle]
pub unsafe extern "C" fn sys_stat(name: *const c_char, out: *mut stat) -> i32 {
    let p = match unsafe { resolve(name) } { Ok(p) => p, Err(e) => return e };
    if p.is_empty() { fill_stat(&files::Stat { dir: true, ..Default::default() }, out); return 0; }
    match files::stat(&p) { Ok(st) => { fill_stat(&st, out); 0 } Err(e) => errno_of(e) }
}
#[no_mangle]
pub unsafe extern "C" fn sys_lstat(name: *const c_char, out: *mut stat) -> i32 { unsafe { sys_stat(name, out) } }

#[no_mangle]
pub unsafe extern "C" fn sys_getdents64(fd: i32, dirp: *mut dirent64, count: usize) -> i64 {
    let mut t = table();
    let Some(Some(Open::Dir { entries, given })) = t.get_mut(fd as usize) else { return -errno::EBADF as i64 };
    if *given { return 0; }
    let name_at = core::mem::offset_of!(dirent64, d_name);
    let need: usize = entries.iter().map(|e| (name_at + e.name.len() + 1).next_multiple_of(8)).sum();
    if need > count { return -errno::EINVAL as i64; }
    let out = dirp as *mut u8;
    let mut at = 0usize;
    for (i, e) in entries.iter().enumerate() {
        let len = (name_at + e.name.len() + 1).next_multiple_of(8);
        unsafe {
            let d = out.add(at);
            core::ptr::write_bytes(d, 0, len);
            (d as *mut u64).write_unaligned(i as u64 + 1);
            (d.add(8) as *mut i64).write_unaligned((at + len) as i64);
            (d.add(16) as *mut u16).write_unaligned(len as u16);
            d.add(18).write(if e.dir { abi::DT_DIR } else { abi::DT_REG });
            core::ptr::copy_nonoverlapping(e.name.as_ptr(), d.add(name_at), e.name.len());
        }
        at += len;
    }
    *given = true;
    at as i64
}

#[no_mangle]
pub unsafe extern "C" fn sys_mkdir(name: *const i8, _mode: u32) -> i32 {
    match unsafe { resolve(name as *const c_char) } { Ok(p) => files::mkdir(&p).map_or_else(errno_of, |_| 0), Err(e) => e }
}
#[no_mangle]
pub unsafe extern "C" fn sys_unlink(name: *const c_char) -> i32 {
    let p = match unsafe { resolve(name) } { Ok(p) => p, Err(e) => return e };
    match files::stat(&p) { Ok(st) if st.dir => -errno::EISDIR, Ok(_) => files::remove(&p).map_or_else(errno_of, |_| 0), Err(e) => errno_of(e) }
}
#[no_mangle]
pub unsafe extern "C" fn sys_rmdir(name: *const c_char) -> i32 {
    let p = match unsafe { resolve(name) } { Ok(p) => p, Err(e) => return e };
    match files::stat(&p) { Ok(st) if !st.dir => -errno::ENOTDIR, Ok(_) => files::remove(&p).map_or_else(errno_of, |_| 0), Err(e) => errno_of(e) }
}

// SexOS additions, for the C library (sex-libc's POSIX layer): std has no use for them.

#[no_mangle]
pub unsafe extern "C" fn sys_rename(from: *const c_char, to: *const c_char) -> i32 {
    let (a, b) = match unsafe { (resolve(from), resolve(to)) } { (Ok(a), Ok(b)) => (a, b), (Err(e), _) | (_, Err(e)) => return e };
    files::rename(&a, &b).map_or_else(errno_of, |_| 0)
}
/// Makes everything the program wrote durable (Files commits all of it at once).
#[no_mangle]
pub extern "C" fn sys_fsync(fd: i32) -> i32 {
    if !matches!(table().get(fd as usize), Some(Some(_))) { return -errno::EBADF; }
    files::sync().map_or_else(errno_of, |_| 0)
}
#[no_mangle]
pub extern "C" fn sys_ftruncate(fd: i32, size: i64) -> i32 {
    if size < 0 { return -errno::EINVAL; }
    let f = match file(fd) { Ok(f) if f.write => f, Ok(_) => return -errno::EINVAL, Err(e) => return e };
    files::truncate(&f.path, size as u64).map_or_else(errno_of, |_| 0)
}

#[no_mangle]
pub extern "C" fn sys_isatty(fd: i32) -> i32 { matches!(table().get(fd as usize), Some(Some(Open::Console(_)))) as i32 }

#[no_mangle]
pub extern "C" fn sys_dup(fd: i32) -> i32 {
    let copy = match table().get(fd as usize) {
        Some(Some(Open::Console(n))) => Open::Console(*n),
        Some(Some(Open::File(f))) => Open::File(f.clone()),
        Some(Some(Open::Socket(s))) => Open::Socket(s.clone()),
        Some(Some(Open::Dir { entries, given })) => Open::Dir { entries: entries.clone(), given: *given },
        _ => return -errno::EBADF,
    };
    insert(copy)
}

#[no_mangle]
pub extern "C" fn sys_fcntl(fd: i32, cmd: i32, arg: i32) -> i32 {
    let entry = table().get(fd as usize).map(|o| o.as_ref().map(|o| match o { Open::Socket(s) => Some(s.clone()), _ => None }));
    match (entry, cmd) {
        (Some(Some(Some(s))), abi::F_GETFL) => if net::nonblocking(&s) { abi::O_NONBLOCK } else { 0 },
        (Some(Some(Some(s))), abi::F_SETFL) => { net::set_nonblocking(&s, arg & abi::O_NONBLOCK != 0); 0 }
        (Some(Some(_)), _) => 0,
        _ => -errno::EBADF,
    }
}

// ---- errors --------------------------------------------------------------------------

#[thread_local]
static mut ERRNO: i32 = 0;
#[no_mangle]
pub extern "C" fn sys_get_errno() -> i32 { unsafe { ERRNO } }
#[no_mangle]
pub extern "C" fn sys_errno() -> i32 { unsafe { ERRNO } }

// ---- sockets (see net.rs) -------------------------------------------------------------------

fn sock(fd: i32) -> Result<Arc<net::Sock>, i32> {
    match table().get(fd as usize) {
        Some(Some(Open::Socket(s))) => Ok(s.clone()),
        Some(Some(_)) => Err(-errno::ENOTSOCK),
        _ => Err(-errno::EBADF),
    }
}
macro_rules! on_sock { ($fd:expr, $s:ident => $body:expr) => { match sock($fd) { Ok($s) => $body, Err(e) => e as _ } } }

#[no_mangle]
pub extern "C" fn sys_socket(domain: i32, kind: i32, protocol: i32) -> i32 {
    match net::socket(domain, kind, protocol) { Ok(s) => insert(Open::Socket(Arc::new(s))), Err(e) => e }
}
#[no_mangle]
pub unsafe extern "C" fn sys_bind(s: i32, addr: *const abi::sockaddr, len: abi::socklen_t) -> i32 { on_sock!(s, k => unsafe { net::bind(&k, addr, len) }) }
#[no_mangle]
pub extern "C" fn sys_listen(s: i32, _backlog: i32) -> i32 { on_sock!(s, k => net::listen(&k)) }
#[no_mangle]
pub unsafe extern "C" fn sys_connect(s: i32, addr: *const abi::sockaddr, len: abi::socklen_t) -> i32 { on_sock!(s, k => unsafe { net::connect(&k, addr, len) }) }
#[no_mangle]
pub unsafe extern "C" fn sys_accept(s: i32, addr: *mut abi::sockaddr, len: *mut abi::socklen_t) -> i32 {
    on_sock!(s, k => match unsafe { net::accept(&k, addr, len) } { Ok(new) => insert(Open::Socket(Arc::new(new))), Err(e) => e })
}
#[no_mangle]
pub unsafe extern "C" fn sys_recv(s: i32, buf: *mut u8, len: usize, flags: i32) -> isize {
    let out = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    on_sock!(s, k => net::recv(&k, out, flags & abi::MSG_PEEK != 0))
}
#[no_mangle]
pub unsafe extern "C" fn sys_recvfrom(s: i32, buf: *mut u8, len: usize, flags: i32, addr: *mut abi::sockaddr, alen: *mut abi::socklen_t) -> isize {
    let out = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    on_sock!(s, k => match net::recv_from(&k, out, flags & abi::MSG_PEEK != 0) {
        Ok((n, ip, port)) => { unsafe { net::write_address_pub(addr, alen, &ip, port) }; n as isize }
        Err(e) => e as isize,
    })
}
#[no_mangle]
pub unsafe extern "C" fn sys_send(s: i32, buf: *const core::ffi::c_void, len: usize, _flags: i32) -> isize {
    let data = unsafe { core::slice::from_raw_parts(buf as *const u8, len) };
    on_sock!(s, k => net::send(&k, data))
}
#[no_mangle]
pub unsafe extern "C" fn sys_sendto(s: i32, buf: *const core::ffi::c_void, len: usize, _flags: i32, addr: *const abi::sockaddr, alen: abi::socklen_t) -> isize {
    let data = unsafe { core::slice::from_raw_parts(buf as *const u8, len) };
    on_sock!(s, k => {
        if addr.is_null() { net::send(&k, data) } else {
            match unsafe { net::read_address_pub(addr, alen) } { Ok((ip, port)) => net::send_to(&k, data, &ip, port), Err(e) => e as isize }
        }
    })
}
#[no_mangle]
pub extern "C" fn sys_shutdown(s: i32, how: i32) -> i32 { on_sock!(s, k => net::shutdown(&k, how)) }
#[no_mangle]
pub unsafe extern "C" fn sys_getpeername(s: i32, addr: *mut abi::sockaddr, len: *mut abi::socklen_t) -> i32 { on_sock!(s, k => unsafe { net::peer_name(&k, addr, len) }) }
#[no_mangle]
pub unsafe extern "C" fn sys_getsockname(s: i32, addr: *mut abi::sockaddr, len: *mut abi::socklen_t) -> i32 { on_sock!(s, k => unsafe { net::sock_name(&k, addr, len) }) }
#[no_mangle]
pub extern "C" fn sys_getsockopt(s: i32, level: i32, name: i32, value: *mut core::ffi::c_void, len: *mut abi::socklen_t) -> i32 { on_sock!(s, k => net::get_option(&k, level, name, value, len)) }
#[no_mangle]
pub extern "C" fn sys_setsockopt(s: i32, level: i32, name: i32, value: *const core::ffi::c_void, len: abi::socklen_t) -> i32 { on_sock!(s, k => net::set_option(&k, level, name, value, len)) }
#[no_mangle]
pub extern "C" fn sys_ioctl(s: i32, cmd: i32, argp: *mut core::ffi::c_void) -> i32 {
    if cmd != abi::FIONBIO { return -errno::EINVAL; }
    if argp.is_null() { return -errno::EFAULT; }
    let on = unsafe { *(argp as *const i32) } != 0;
    on_sock!(s, k => { net::set_nonblocking(&k, on); 0 })
}

/// Waits until one of `fds` is ready or `timeout` milliseconds pass (-1: no limit).
/// Files and the console are always ready; sockets say what they have.
#[no_mangle]
pub unsafe extern "C" fn sys_poll(fds: *mut abi::pollfd, n: abi::nfds_t, timeout: i32) -> i32 {
    if fds.is_null() && n > 0 { return -errno::EFAULT; }
    let fds = unsafe { core::slice::from_raw_parts_mut(fds, n) };
    let began = sex_pdx::get_ticks();
    let mut round = 0u32;
    loop {
        let mut count = 0;
        for p in fds.iter_mut() {
            p.revents = if p.fd < 0 { 0 } else {
                let entry = table().get(p.fd as usize).map(|o| o.as_ref().map(|o| match o { Open::Socket(s) => Some(s.clone()), _ => None }));
                match entry {
                    Some(Some(Some(s))) => net::ready(&s, p.events),
                    Some(Some(None)) => p.events & (abi::POLLIN | abi::POLLOUT),
                    _ => abi::POLLNVAL,
                }
            };
            if p.revents != 0 { count += 1; }
        }
        if count > 0 || timeout == 0 { return count; }
        if timeout > 0 && sex_pdx::get_ticks().saturating_sub(began) >= timeout as u64 { return 0; }
        round += 1;
        sex_app::thread::pause(round);
    }
}

#[no_mangle]
pub extern "C" fn sys_eventfd(_init: u64, _flags: i16) -> i32 { -errno::ENOSYS }

/// Names to addresses through the network service. The list is one allocation per
/// entry, freed by `sys_freeaddrinfo`.
#[no_mangle]
pub unsafe extern "C" fn sys_getaddrinfo(name: *const c_char, _service: *const c_char, hints: *const abi::addrinfo, out: *mut *mut abi::addrinfo) -> i32 {
    if name.is_null() || out.is_null() { return abi::EAI_FAIL; }
    let host = unsafe { CStr::from_ptr(name) }.to_bytes();
    let (family, socktype) = if hints.is_null() { (abi::AF_UNSPEC, 0) } else { unsafe { ((*hints).ai_family, (*hints).ai_socktype) } };
    let found = match net::lookup(host) { Ok(f) => f, Err(e) => return e };
    let mut head: *mut abi::addrinfo = core::ptr::null_mut();
    for ip in found.iter().rev() {
        let fam = if ip.is_ipv6() { abi::AF_INET6 } else { abi::AF_INET };
        if family != abi::AF_UNSPEC && family != fam { continue; }
        let storage = Box::into_raw(Box::new(unsafe { core::mem::zeroed::<abi::sockaddr_storage>() })) as *mut abi::sockaddr;
        let mut len = core::mem::size_of::<abi::sockaddr_storage>() as abi::socklen_t;
        unsafe { net::write_address_pub(storage, &mut len, ip, 0) };
        head = Box::into_raw(Box::new(abi::addrinfo {
            ai_flags: 0, ai_family: fam, ai_socktype: socktype, ai_protocol: 0, ai_addrlen: len,
            ai_canonname: core::ptr::null_mut(), ai_addr: storage, ai_next: head,
        }));
    }
    if head.is_null() { return abi::EAI_NONAME; }
    unsafe { *out = head; }
    0
}
#[no_mangle]
pub unsafe extern "C" fn sys_freeaddrinfo(mut ai: *mut abi::addrinfo) {
    while !ai.is_null() {
        let entry = unsafe { Box::from_raw(ai) };
        if !entry.ai_addr.is_null() { drop(unsafe { Box::from_raw(entry.ai_addr as *mut abi::sockaddr_storage) }); }
        ai = entry.ai_next;
    }
}
