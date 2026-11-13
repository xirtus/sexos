// SPDX-License-Identifier: GPL-3.0-only
//! POSIX for ordinary C programs, as a library inside the program.
//!
//! On the ordinary-program target (`x86_64-sex-std.json`) a C program gets files,
//! descriptors, buffered `FILE` streams, folders, standard input and output, `exit`
//! and the clock, so `int main(int argc, char **argv)` code that uses `<stdio.h>`,
//! `<fcntl.h>`, `<unistd.h>`, `<sys/stat.h>` and `<dirent.h>` builds and runs as it is.
//!
//! None of it is in the kernel or a service. Every call becomes a call of the
//! program's Rust standard-library layer (crates/sex-std: `sys_open`, `sys_read`, ...),
//! which answers it with the program's own grants: files in the program's own folder,
//! which is `/` here, its window for the terminal, its workers for threads. So a C
//! program holds exactly what its BOOT row holds, like a Rust one.
//!
//! There is no process model (no fork, exec or process ids): a program is a run of
//! its row, not a Unix process. `mmap` of a file is a library emulation (see below).
//!
//! The C types are musl's (x86-64), the headers the program compiles against.
use crate::format::{self, Sink};
use crate::stdio::{c_str, Va};
use crate::{c_char, c_int, c_long, c_void, set_errno};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::VaList;
use core::ptr::null_mut;

#[repr(C)]
struct HStat {
    dev: u64, ino: u64, nlink: u64, mode: u32, uid: u32, gid: u32, rdev: u64, size: i64,
    blksize: i64, blocks: i64, atim: [i64; 2], mtim: [i64; 2], ctim: [i64; 2],
}

extern "C" {
    fn sys_open(name: *const c_char, flags: i32, mode: i32) -> i32;
    fn sys_close(fd: i32) -> i32;
    fn sys_read(fd: i32, buf: *mut u8, len: usize) -> isize;
    fn sys_write(fd: i32, buf: *const u8, len: usize) -> isize;
    fn sys_lseek(fd: i32, offset: isize, whence: i32) -> isize;
    fn sys_stat(name: *const c_char, out: *mut HStat) -> i32;
    fn sys_fstat(fd: i32, out: *mut HStat) -> i32;
    fn sys_unlink(name: *const c_char) -> i32;
    fn sys_mkdir(name: *const i8, mode: u32) -> i32;
    fn sys_rmdir(name: *const c_char) -> i32;
    fn sys_isatty(fd: i32) -> i32;
    fn sys_getdents64(fd: i32, dirp: *mut u8, count: usize) -> i64;
    fn sys_clock_gettime(clock: i32, tp: *mut [i64; 2]) -> i32;
    fn sys_exit(status: i32) -> !;
    fn sys_abort() -> !;
    fn sys_usleep(usecs: u64);
    // SexOS additions to the interface (crates/sex-std).
    fn sys_rename(from: *const c_char, to: *const c_char) -> i32;
    fn sys_fsync(fd: i32) -> i32;
    fn sys_ftruncate(fd: i32, size: i64) -> i32;
    fn sys_pread(fd: i32, buf: *mut u8, len: usize, offset: i64) -> isize;
    fn sys_pwrite(fd: i32, buf: *const u8, len: usize, offset: i64) -> isize;
    fn sys_dup(fd: i32) -> i32;
    fn sys_yield();
}

/// A negative error number becomes `errno` and -1.
fn ret(r: i64) -> i64 { if r < 0 { set_errno(-r as c_int); -1 } else { r } }

pub(crate) fn clock() -> (i64, u64) {
    let (mut wall, mut mono) = ([0i64; 2], [0i64; 2]);
    unsafe { sys_clock_gettime(1, &mut wall); sys_clock_gettime(4, &mut mono); }
    (wall[0] * 1_000_000_000 + wall[1], (mono[0] * 1_000_000_000 + mono[1]) as u64)
}

pub(crate) fn log_line(bytes: &[u8]) {
    unsafe { flush_stream(core::ptr::addr_of_mut!(STDOUT)); sys_write(2, bytes.as_ptr(), bytes.len()); sys_write(2, b"\n".as_ptr(), 1); }
}

/// Runs a C program's `main` with `argv`, then ends the program as `exit` does.
pub fn run(main: unsafe extern "C" fn(c_int, *mut *mut c_char) -> c_int, argv: &mut [*mut c_char]) -> ! {
    unsafe {
        crate::cxx::run_constructors();
        let argc = argv.iter().position(|p| p.is_null()).unwrap_or(argv.len()) as c_int;
        let code = main(argc, argv.as_mut_ptr());
        exit(code)
    }
}

// ---- descriptors --------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mut ap: ...) -> c_int {
    let mode = if flags & 0o100 != 0 { unsafe { ap.next_arg::<c_int>() } } else { 0 };
    ret(unsafe { sys_open(path, flags, mode) } as i64) as c_int
}
#[no_mangle]
pub unsafe extern "C" fn creat(path: *const c_char, mode: c_int) -> c_int {
    ret(unsafe { sys_open(path, 0o1 | 0o100 | 0o1000, mode) } as i64) as c_int
}
#[no_mangle]
pub unsafe extern "C" fn close(fd: c_int) -> c_int { ret(unsafe { sys_close(fd) } as i64) as c_int }
#[no_mangle]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize {
    ret(unsafe { sys_read(fd, buf.cast(), n) } as i64) as isize
}
#[no_mangle]
pub unsafe extern "C" fn write(fd: c_int, buf: *const c_void, n: usize) -> isize {
    if fd == 1 || fd == 2 { unsafe { flush_stream(if fd == 1 { core::ptr::addr_of_mut!(STDOUT) } else { core::ptr::addr_of_mut!(STDERR) }); } }
    ret(unsafe { sys_write(fd, buf.cast(), n) } as i64) as isize
}
#[no_mangle]
pub unsafe extern "C" fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64 {
    ret(unsafe { sys_lseek(fd, offset as isize, whence) } as i64)
}
#[no_mangle]
pub unsafe extern "C" fn fsync(fd: c_int) -> c_int { ret(unsafe { sys_fsync(fd) } as i64) as c_int }
#[no_mangle]
pub unsafe extern "C" fn ftruncate(fd: c_int, size: i64) -> c_int { ret(unsafe { sys_ftruncate(fd, size) } as i64) as c_int }
#[no_mangle]
pub unsafe extern "C" fn isatty(fd: c_int) -> c_int { unsafe { sys_isatty(fd) } }
#[no_mangle]
pub unsafe extern "C" fn unlink(path: *const c_char) -> c_int { ret(unsafe { sys_unlink(path) } as i64) as c_int }
#[no_mangle]
pub unsafe extern "C" fn rmdir(path: *const c_char) -> c_int { ret(unsafe { sys_rmdir(path) } as i64) as c_int }
#[no_mangle]
pub unsafe extern "C" fn mkdir(path: *const c_char, mode: u32) -> c_int { ret(unsafe { sys_mkdir(path.cast(), mode) } as i64) as c_int }
#[no_mangle]
pub unsafe extern "C" fn rename(from: *const c_char, to: *const c_char) -> c_int { ret(unsafe { sys_rename(from, to) } as i64) as c_int }
#[no_mangle]
pub unsafe extern "C" fn remove(path: *const c_char) -> c_int {
    let r = unsafe { sys_unlink(path) };
    let r = if r == -21 { unsafe { sys_rmdir(path) } } else { r };
    ret(r as i64) as c_int
}
#[no_mangle]
pub unsafe extern "C" fn access(path: *const c_char, _mode: c_int) -> c_int {
    let mut st: HStat = unsafe { core::mem::zeroed() };
    ret(unsafe { sys_stat(path, &mut st) } as i64) as c_int
}
#[no_mangle]
pub unsafe extern "C" fn getcwd(buf: *mut c_char, size: usize) -> *mut c_char {
    if buf.is_null() || size < 2 { set_errno(crate::ERANGE); return null_mut(); }
    unsafe { *buf = b'/' as c_char; *buf.add(1) = 0; }
    buf
}
/// The program's folder is its only folder.
#[no_mangle]
pub unsafe extern "C" fn chdir(path: *const c_char) -> c_int {
    if unsafe { c_str(path) } == b"/" { 0 } else { set_errno(crate::ENOSYS); -1 }
}
#[no_mangle]
pub unsafe extern "C" fn sleep(seconds: u32) -> u32 { unsafe { sys_usleep(seconds as u64 * 1_000_000) }; 0 }
#[no_mangle]
pub unsafe extern "C" fn usleep(us: u32) -> c_int { unsafe { sys_usleep(us as u64) }; 0 }
#[no_mangle]
pub unsafe extern "C" fn nanosleep(req: *const [i64; 2], _rem: *mut [i64; 2]) -> c_int {
    let t = unsafe { *req };
    unsafe { sys_usleep((t[0] as u64) * 1_000_000 + t[1] as u64 / 1000) };
    0
}

/// musl's x86-64 `struct stat`.
#[repr(C)]
pub struct stat {
    pub st_dev: u64, pub st_ino: u64, pub st_nlink: u64, pub st_mode: u32, pub st_uid: u32, pub st_gid: u32,
    __pad0: u32, pub st_rdev: u64, pub st_size: i64, pub st_blksize: i64, pub st_blocks: i64,
    pub st_atim: [i64; 2], pub st_mtim: [i64; 2], pub st_ctim: [i64; 2], __unused: [i64; 3],
}
fn to_musl(h: &HStat, out: *mut stat) {
    unsafe {
        *out = stat { st_dev: h.dev, st_ino: h.ino, st_nlink: h.nlink, st_mode: h.mode, st_uid: 0, st_gid: 0, __pad0: 0,
            st_rdev: 0, st_size: h.size, st_blksize: h.blksize, st_blocks: h.blocks, st_atim: h.atim, st_mtim: h.mtim, st_ctim: h.ctim, __unused: [0; 3] };
    }
}
#[no_mangle]
pub unsafe extern "C" fn stat(path: *const c_char, out: *mut stat) -> c_int {
    let mut h: HStat = unsafe { core::mem::zeroed() };
    let r = unsafe { sys_stat(path, &mut h) };
    if r == 0 { to_musl(&h, out); }
    ret(r as i64) as c_int
}
#[no_mangle]
pub unsafe extern "C" fn lstat(path: *const c_char, out: *mut stat) -> c_int { unsafe { stat(path, out) } }
#[no_mangle]
pub unsafe extern "C" fn fstat(fd: c_int, out: *mut stat) -> c_int {
    let mut h: HStat = unsafe { core::mem::zeroed() };
    let r = unsafe { sys_fstat(fd, &mut h) };
    if r == 0 { to_musl(&h, out); }
    ret(r as i64) as c_int
}

// ---- folders ----------------------------------------------------------------------------

/// `DIR`: the folder's entries, read at open, handed out one at a time.
pub struct DIR { buf: Vec<u8>, at: usize }

#[no_mangle]
pub unsafe extern "C" fn opendir(path: *const c_char) -> *mut DIR {
    let fd = unsafe { sys_open(path, 0o200000, 0) };
    if fd < 0 { set_errno(-fd); return null_mut(); }
    let mut size = 4096;
    loop {
        let mut buf = alloc::vec![0u8; size];
        let n = unsafe { sys_getdents64(fd, buf.as_mut_ptr(), size) };
        if n == -22 && size < 1 << 24 { size *= 4; continue; }
        unsafe { sys_close(fd) };
        if n < 0 { set_errno(-n as c_int); return null_mut(); }
        buf.truncate(n as usize);
        return Box::into_raw(Box::new(DIR { buf, at: 0 }));
    }
}
/// The next entry, laid out as musl's `struct dirent` (the same as `dirent64`).
#[no_mangle]
pub unsafe extern "C" fn readdir(dir: *mut DIR) -> *mut c_void {
    let d = unsafe { &mut *dir };
    if d.at + 19 > d.buf.len() { return null_mut(); }
    let entry = unsafe { d.buf.as_mut_ptr().add(d.at) };
    let len = u16::from_le_bytes([d.buf[d.at + 16], d.buf[d.at + 17]]) as usize;
    d.at += len.max(24);
    entry.cast()
}
#[no_mangle]
pub unsafe extern "C" fn closedir(dir: *mut DIR) -> c_int {
    if !dir.is_null() { drop(unsafe { Box::from_raw(dir) }); }
    0
}
#[no_mangle]
pub unsafe extern "C" fn rewinddir(dir: *mut DIR) { unsafe { (*dir).at = 0 } }

// ---- streams ---------------------------------------------------------------------------

const BUF: usize = 8192;

/// A C stream over a descriptor, buffered both ways.
#[repr(C)]
pub struct FILE {
    fd: c_int,
    writable: bool,
    readable: bool,
    eof: bool,
    error: bool,
    /// Bytes waiting to be written.
    out: Vec<u8>,
    /// Bytes read ahead, and how many are used.
    input: Vec<u8>,
    taken: usize,
    /// 0 unbuffered, 1 line, 2 full.
    mode: u8,
}

const fn console(fd: c_int, mode: u8) -> FILE {
    FILE { fd, writable: fd != 0, readable: fd == 0, eof: false, error: false, out: Vec::new(), input: Vec::new(), taken: 0, mode }
}
static mut STDIN: FILE = console(0, 1);
static mut STDOUT: FILE = console(1, 1);
static mut STDERR: FILE = console(2, 0);
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static mut stdin: *mut FILE = core::ptr::addr_of_mut!(STDIN);
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static mut stdout: *mut FILE = core::ptr::addr_of_mut!(STDOUT);
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static mut stderr: *mut FILE = core::ptr::addr_of_mut!(STDERR);

/// Streams `fopen` made, so `exit` and `fflush(NULL)` can flush them.
static mut OPEN: Vec<*mut FILE> = Vec::new();

unsafe fn flush_stream(f: *mut FILE) -> c_int {
    let f = unsafe { &mut *f };
    let mut done = 0;
    while done < f.out.len() {
        let n = unsafe { sys_write(f.fd, f.out[done..].as_ptr(), f.out.len() - done) };
        if n <= 0 { f.error = true; f.out.clear(); if n < 0 { set_errno(-n as c_int); } return -1; }
        done += n as usize;
    }
    f.out.clear();
    0
}

/// Read-ahead belongs to where the stream was; writing or seeking first gives it back.
unsafe fn drop_input(f: &mut FILE) {
    let ahead = f.input.len() - f.taken;
    if ahead > 0 { unsafe { sys_lseek(f.fd, -(ahead as isize), 1) }; }
    f.input.clear();
    f.taken = 0;
}

unsafe fn put(f: *mut FILE, bytes: &[u8]) -> bool {
    let s = unsafe { &mut *f };
    if !s.writable { s.error = true; set_errno(crate::EBADF); return false; }
    if !s.input.is_empty() { unsafe { drop_input(s) }; }
    s.out.extend_from_slice(bytes);
    let flush = match s.mode { 0 => true, 1 => bytes.contains(&b'\n') || s.out.len() >= BUF, _ => s.out.len() >= BUF };
    if flush { unsafe { flush_stream(f) == 0 } } else { true }
}

/// Fills the read-ahead; false at the end or on an error.
unsafe fn fill(f: &mut FILE) -> bool {
    if f.taken < f.input.len() { return true; }
    if f.fd == 0 { unsafe { flush_stream(core::ptr::addr_of_mut!(STDOUT)) }; }
    if !f.out.is_empty() { unsafe { flush_stream(f) }; }
    f.input.resize(BUF, 0);
    let n = unsafe { sys_read(f.fd, f.input.as_mut_ptr(), BUF) };
    f.taken = 0;
    if n <= 0 {
        f.input.clear();
        if n == 0 { f.eof = true } else { f.error = true; set_errno(-n as c_int) }
        return false;
    }
    f.input.truncate(n as usize);
    true
}

fn mode_flags(mode: &[u8]) -> Option<(c_int, bool, bool)> {
    let plus = mode.contains(&b'+');
    let excl = if mode.contains(&b'x') { 0o200 } else { 0 };
    Some(match mode.first()? {
        b'r' => (if plus { 0o2 } else { 0 }, true, plus),
        b'w' => ((if plus { 0o2 } else { 0o1 }) | 0o100 | 0o1000 | excl, plus, true),
        b'a' => ((if plus { 0o2 } else { 0o1 }) | 0o100 | 0o2000 | excl, plus, true),
        _ => return None,
    })
}

fn stream(fd: c_int, readable: bool, writable: bool) -> *mut FILE {
    let f = Box::into_raw(Box::new(FILE { fd, readable, writable, ..console(fd, 2) }));
    unsafe { (*core::ptr::addr_of_mut!(OPEN)).push(f) };
    f
}

#[no_mangle]
pub unsafe extern "C" fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE {
    let Some((flags, readable, writable)) = mode_flags(unsafe { c_str(mode) }) else { set_errno(crate::EINVAL); return null_mut() };
    let fd = unsafe { sys_open(path, flags, 0o644) };
    if fd < 0 { set_errno(-fd); return null_mut(); }
    stream(fd, readable, writable)
}
#[no_mangle]
pub unsafe extern "C" fn fdopen(fd: c_int, mode: *const c_char) -> *mut FILE {
    let Some((_, readable, writable)) = mode_flags(unsafe { c_str(mode) }) else { set_errno(crate::EINVAL); return null_mut() };
    stream(fd, readable, writable)
}
#[no_mangle]
pub unsafe extern "C" fn freopen(path: *const c_char, mode: *const c_char, f: *mut FILE) -> *mut FILE {
    let new = unsafe { fopen(path, mode) };
    if new.is_null() { return null_mut(); }
    unsafe {
        flush_stream(f);
        let (s, n) = (&mut *f, &*new);
        if s.fd > 2 { sys_close(s.fd); }
        s.fd = n.fd; s.readable = n.readable; s.writable = n.writable;
        s.eof = false; s.error = false; s.input.clear(); s.taken = 0;
        (*core::ptr::addr_of_mut!(OPEN)).retain(|p| *p != new);
        // The new stream's descriptor now belongs to `f`.
        drop(Box::from_raw(new));
    }
    f
}
#[no_mangle]
pub unsafe extern "C" fn fclose(f: *mut FILE) -> c_int {
    if f.is_null() { return -1; }
    let r = unsafe { flush_stream(f) };
    let fd = unsafe { (*f).fd };
    let open = unsafe { &mut *core::ptr::addr_of_mut!(OPEN) };
    if let Some(i) = open.iter().position(|p| *p == f) {
        open.swap_remove(i);
        drop(unsafe { Box::from_raw(f) });
        unsafe { sys_close(fd) };
    }
    r
}
#[no_mangle]
pub unsafe extern "C" fn fflush(f: *mut FILE) -> c_int {
    if !f.is_null() { return unsafe { flush_stream(f) }; }
    unsafe { flush_all() };
    0
}
unsafe fn flush_all() {
    unsafe {
        flush_stream(core::ptr::addr_of_mut!(STDOUT));
        flush_stream(core::ptr::addr_of_mut!(STDERR));
        for f in (*core::ptr::addr_of!(OPEN)).iter() { flush_stream(*f); }
    }
}
#[no_mangle]
pub unsafe extern "C" fn setvbuf(f: *mut FILE, _buf: *mut c_char, mode: c_int, _size: usize) -> c_int {
    // musl: _IOFBF 0, _IOLBF 1, _IONBF 2.
    unsafe { (*f).mode = match mode { 0 => 2, 1 => 1, _ => 0 } };
    0
}
#[no_mangle]
pub unsafe extern "C" fn setbuf(f: *mut FILE, buf: *mut c_char) { unsafe { setvbuf(f, buf, if buf.is_null() { 2 } else { 0 }, BUF) }; }

#[no_mangle]
pub unsafe extern "C" fn fwrite(p: *const c_void, size: usize, count: usize, f: *mut FILE) -> usize {
    let n = size.saturating_mul(count);
    if n == 0 { return 0; }
    if unsafe { put(f, core::slice::from_raw_parts(p.cast(), n)) } { count } else { 0 }
}
#[no_mangle]
pub unsafe extern "C" fn fread(p: *mut c_void, size: usize, count: usize, f: *mut FILE) -> usize {
    let want = size.saturating_mul(count);
    if want == 0 { return 0; }
    let s = unsafe { &mut *f };
    if !s.readable { s.error = true; return 0; }
    let out = unsafe { core::slice::from_raw_parts_mut(p.cast::<u8>(), want) };
    let mut got = 0;
    while got < want {
        if s.taken == s.input.len() && want - got >= BUF {
            // Large reads go straight to the caller.
            if !s.out.is_empty() { unsafe { flush_stream(f) }; }
            let n = unsafe { sys_read(s.fd, out[got..].as_mut_ptr(), want - got) };
            if n <= 0 { if n == 0 { s.eof = true } else { s.error = true }; break; }
            got += n as usize;
            continue;
        }
        if !unsafe { fill(s) } { break; }
        let n = (s.input.len() - s.taken).min(want - got);
        out[got..got + n].copy_from_slice(&s.input[s.taken..s.taken + n]);
        s.taken += n;
        got += n;
    }
    got / size
}
#[no_mangle]
pub unsafe extern "C" fn fgetc(f: *mut FILE) -> c_int {
    let s = unsafe { &mut *f };
    if !unsafe { fill(s) } { return -1; }
    s.taken += 1;
    s.input[s.taken - 1] as c_int
}
#[no_mangle]
pub unsafe extern "C" fn getc(f: *mut FILE) -> c_int { unsafe { fgetc(f) } }
#[no_mangle]
pub unsafe extern "C" fn getchar() -> c_int { unsafe { fgetc(stdin) } }
#[no_mangle]
pub unsafe extern "C" fn ungetc(c: c_int, f: *mut FILE) -> c_int {
    if c < 0 { return -1; }
    let s = unsafe { &mut *f };
    if s.taken > 0 { s.taken -= 1; s.input[s.taken] = c as u8; } else { s.input.insert(0, c as u8); }
    s.eof = false;
    c
}
#[no_mangle]
pub unsafe extern "C" fn fgets(buf: *mut c_char, size: c_int, f: *mut FILE) -> *mut c_char {
    if size <= 0 { return null_mut(); }
    let mut n = 0usize;
    while n + 1 < size as usize {
        let c = unsafe { fgetc(f) };
        if c < 0 { break; }
        unsafe { *buf.add(n) = c as c_char };
        n += 1;
        if c == b'\n' as c_int { break; }
    }
    if n == 0 { return null_mut(); }
    unsafe { *buf.add(n) = 0 };
    buf
}
#[no_mangle]
pub unsafe extern "C" fn fputc(c: c_int, f: *mut FILE) -> c_int {
    if unsafe { put(f, &[c as u8]) } { c & 0xFF } else { -1 }
}
#[no_mangle]
pub unsafe extern "C" fn putc(c: c_int, f: *mut FILE) -> c_int { unsafe { fputc(c, f) } }
#[no_mangle]
pub unsafe extern "C" fn putchar(c: c_int) -> c_int { unsafe { fputc(c, stdout) } }
#[no_mangle]
pub unsafe extern "C" fn fputs(s: *const c_char, f: *mut FILE) -> c_int {
    if unsafe { put(f, c_str(s)) } { 0 } else { -1 }
}
#[no_mangle]
pub unsafe extern "C" fn puts(s: *const c_char) -> c_int {
    let ok = unsafe { put(stdout, c_str(s)) && put(stdout, b"\n") };
    if ok { 0 } else { -1 }
}
#[no_mangle]
pub unsafe extern "C" fn perror(s: *const c_char) {
    let e = unsafe { *crate::__errno_location() };
    let text = unsafe { c_str(crate::string::strerror(e)) };
    unsafe {
        if !s.is_null() && *s != 0 { put(stderr, c_str(s)); put(stderr, b": "); }
        put(stderr, text);
        put(stderr, b"\n");
    }
}
#[no_mangle]
pub unsafe extern "C" fn fseek(f: *mut FILE, offset: c_long, whence: c_int) -> c_int {
    unsafe { fseeko(f, offset as i64, whence) }
}
#[no_mangle]
pub unsafe extern "C" fn fseeko(f: *mut FILE, offset: i64, whence: c_int) -> c_int {
    let s = unsafe { &mut *f };
    if unsafe { flush_stream(f) } != 0 { return -1; }
    let offset = if whence == 1 { offset - (s.input.len() - s.taken) as i64 } else { offset };
    s.input.clear();
    s.taken = 0;
    s.eof = false;
    if unsafe { lseek(s.fd, offset, whence) } < 0 { -1 } else { 0 }
}
#[no_mangle]
pub unsafe extern "C" fn ftello(f: *mut FILE) -> i64 {
    let s = unsafe { &mut *f };
    let at = unsafe { lseek(s.fd, 0, 1) };
    if at < 0 { return -1; }
    at - (s.input.len() - s.taken) as i64 + s.out.len() as i64
}
#[no_mangle]
pub unsafe extern "C" fn ftell(f: *mut FILE) -> c_long { unsafe { ftello(f) as c_long } }
#[no_mangle]
pub unsafe extern "C" fn rewind(f: *mut FILE) { unsafe { fseeko(f, 0, 0); (*f).error = false; } }
#[no_mangle]
pub unsafe extern "C" fn feof(f: *mut FILE) -> c_int { unsafe { (*f).eof as c_int } }
#[no_mangle]
pub unsafe extern "C" fn ferror(f: *mut FILE) -> c_int { unsafe { (*f).error as c_int } }
#[no_mangle]
pub unsafe extern "C" fn clearerr(f: *mut FILE) { unsafe { (*f).eof = false; (*f).error = false; } }
#[no_mangle]
pub unsafe extern "C" fn fileno(f: *mut FILE) -> c_int { unsafe { (*f).fd } }

struct Stream(*mut FILE, bool);
impl Sink for Stream {
    fn put(&mut self, bytes: &[u8]) { if !unsafe { put(self.0, bytes) } { self.1 = false; } }
}
#[no_mangle]
pub unsafe extern "C" fn vfprintf(f: *mut FILE, fmt: *const c_char, mut ap: VaList) -> c_int {
    if f.is_null() { return -1; }
    let mut out = Stream(f, true);
    let n = format::format(unsafe { c_str(fmt) }, &mut Va(&mut ap), &mut out);
    if out.1 { n as c_int } else { -1 }
}
#[no_mangle]
pub unsafe extern "C" fn fprintf(f: *mut FILE, fmt: *const c_char, ap: ...) -> c_int { unsafe { vfprintf(f, fmt, ap) } }
#[no_mangle]
pub unsafe extern "C" fn vprintf(fmt: *const c_char, ap: VaList) -> c_int { unsafe { vfprintf(stdout, fmt, ap) } }
#[no_mangle]
pub unsafe extern "C" fn printf(fmt: *const c_char, ap: ...) -> c_int { unsafe { vfprintf(stdout, fmt, ap) } }
#[no_mangle]
pub unsafe extern "C" fn vdprintf(fd: c_int, fmt: *const c_char, mut ap: VaList) -> c_int {
    struct Fd(c_int);
    impl Sink for Fd { fn put(&mut self, b: &[u8]) { unsafe { sys_write(self.0, b.as_ptr(), b.len()) }; } }
    format::format(unsafe { c_str(fmt) }, &mut Va(&mut ap), &mut Fd(fd)) as c_int
}
#[no_mangle]
pub unsafe extern "C" fn dprintf(fd: c_int, fmt: *const c_char, ap: ...) -> c_int { unsafe { vdprintf(fd, fmt, ap) } }

// ---- positioned reads and writes, duplicates ----------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn pread(fd: c_int, buf: *mut c_void, n: usize, offset: i64) -> isize {
    ret(unsafe { sys_pread(fd, buf.cast(), n, offset) } as i64) as isize
}
#[no_mangle]
pub unsafe extern "C" fn pwrite(fd: c_int, buf: *const c_void, n: usize, offset: i64) -> isize {
    ret(unsafe { sys_pwrite(fd, buf.cast(), n, offset) } as i64) as isize
}
#[no_mangle]
pub unsafe extern "C" fn dup(fd: c_int) -> c_int { ret(unsafe { sys_dup(fd) } as i64) as c_int }
#[no_mangle]
pub unsafe extern "C" fn fdatasync(fd: c_int) -> c_int { unsafe { fsync(fd) } }
#[no_mangle]
pub unsafe extern "C" fn sync() { unsafe { sys_fsync(0) }; }
#[no_mangle]
pub unsafe extern "C" fn truncate(path: *const c_char, size: i64) -> c_int {
    let fd = unsafe { sys_open(path, 0o1, 0) };
    if fd < 0 { return ret(fd as i64) as c_int; }
    let r = unsafe { sys_ftruncate(fd, size) };
    unsafe { sys_close(fd) };
    ret(r as i64) as c_int
}

// ---- mmap, as a library ------------------------------------------------------------------
//
// There is no paging behind a file here: a program's memory is its own heap, and its
// files are Files' (see the crate notes). So `mmap` of a file is emulated the way a
// library can: the bytes are read into fresh page-aligned memory. `MAP_PRIVATE` is
// then exact (changes stay in memory). `MAP_SHARED` with `PROT_WRITE` writes the
// memory back to the file at `msync` and `munmap` (and when the program exits): up to
// the file's end, never extending it. What it cannot do: show another descriptor's
// writes to the file in the mapping, place a mapping at a fixed address (`MAP_FIXED`),
// or unmap part of a mapping.

const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;
const MAP_PRIVATE: c_int = 2;
const MAP_FIXED: c_int = 0x10;
const MAP_ANONYMOUS: c_int = 0x20;
const MS_SYNC: c_int = 4;
const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;

#[derive(Clone, Copy)]
struct Mapping {
    addr: usize,
    /// Rounded up to whole pages: what was allocated.
    pages: usize,
    /// As asked: what may go back to the file.
    len: usize,
    /// A duplicate of the file's descriptor, kept for write-back; -1 for none.
    fd: c_int,
    offset: i64,
}

static mut MAPS: Vec<Mapping> = Vec::new();
static MAPS_HELD: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
struct MapsGuard;
impl Drop for MapsGuard { fn drop(&mut self) { MAPS_HELD.store(false, core::sync::atomic::Ordering::Release); } }
fn maps() -> (MapsGuard, &'static mut Vec<Mapping>) {
    while MAPS_HELD.swap(true, core::sync::atomic::Ordering::Acquire) { unsafe { sys_yield() }; }
    (MapsGuard, unsafe { &mut *core::ptr::addr_of_mut!(MAPS) })
}

fn page_layout(pages: usize) -> Option<alloc::alloc::Layout> { alloc::alloc::Layout::from_size_align(pages, crate::posix_util::PAGE).ok() }

/// Writes a shared mapping's bytes back to its file, up to the file's present end.
unsafe fn write_back(m: &Mapping) -> i32 {
    if m.fd < 0 { return 0; }
    let mut h: HStat = unsafe { core::mem::zeroed() };
    let r = unsafe { sys_fstat(m.fd, &mut h) };
    if r < 0 { return r; }
    let n = crate::posix_util::writeback_len(m.len, m.offset as u64, h.size.max(0) as u64);
    let mut done = 0;
    while done < n {
        let w = unsafe { sys_pwrite(m.fd, (m.addr + done) as *const u8, n - done, m.offset + done as i64) };
        if w <= 0 { return if w < 0 { w as i32 } else { -28 }; } // ENOSPC
        done += w as usize;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void {
    let fail = |e: c_int| { set_errno(e); MAP_FAILED };
    let kind = flags & (MAP_SHARED | MAP_PRIVATE); // 3: MAP_SHARED_VALIDATE, shared
    if kind == 0 || offset < 0 || offset as usize % crate::posix_util::PAGE != 0 { return fail(crate::EINVAL); }
    if flags & MAP_FIXED != 0 { let _ = addr; return fail(95); } // ENOTSUP: no fixed placement
    let Some(pages) = crate::posix_util::map_len(len) else { return fail(crate::EINVAL) };
    let Some(layout) = page_layout(pages) else { return fail(crate::ENOMEM) };
    let anonymous = flags & MAP_ANONYMOUS != 0;
    let shared_write = !anonymous && kind & MAP_SHARED != 0 && prot & PROT_WRITE != 0;
    if !anonymous {
        // A regular file, open for reading (and writing, to share writes).
        let mut h: HStat = unsafe { core::mem::zeroed() };
        let r = unsafe { sys_fstat(fd, &mut h) };
        if r < 0 { return fail(-r); }
        if h.mode & 0o170000 != 0o100000 { return fail(19); } // ENODEV
        if unsafe { sys_pread(fd, null_mut(), 0, offset) } < 0 { return fail(13); } // EACCES
        if shared_write && unsafe { sys_pwrite(fd, core::ptr::null(), 0, offset) } < 0 { return fail(13); }
    }
    let p = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if p.is_null() { return fail(crate::ENOMEM); }
    if !anonymous {
        let mut done = 0;
        while done < len {
            let n = unsafe { sys_pread(fd, p.add(done), len - done, offset + done as i64) };
            if n < 0 { unsafe { alloc::alloc::dealloc(p, layout) }; return fail(-n as c_int); }
            if n == 0 { break; } // past the end: zeros, as a mapping shows
            done += n as usize;
        }
    }
    let keep = if shared_write { unsafe { sys_dup(fd) } } else { -1 };
    if shared_write && keep < 0 { unsafe { alloc::alloc::dealloc(p, layout) }; return fail(-keep); }
    let (_held, all) = maps();
    all.push(Mapping { addr: p as usize, pages, len, fd: keep, offset });
    p as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn munmap(addr: *mut c_void, len: usize) -> c_int {
    let m = {
        let (_held, all) = maps();
        let Some(i) = all.iter().position(|m| m.addr == addr as usize) else { set_errno(crate::EINVAL); return -1 };
        if crate::posix_util::map_len(len) != Some(all[i].pages) { set_errno(crate::EINVAL); return -1; } // whole mappings only
        all.swap_remove(i)
    };
    let r = unsafe { write_back(&m) };
    if m.fd >= 0 { unsafe { sys_close(m.fd) }; }
    if let Some(layout) = page_layout(m.pages) { unsafe { alloc::alloc::dealloc(m.addr as *mut u8, layout) }; }
    ret(r as i64) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn msync(addr: *mut c_void, len: usize, flags: c_int) -> c_int {
    let (at, end) = (addr as usize, (addr as usize).saturating_add(len));
    let found = {
        let (_held, all) = maps();
        all.iter().find(|m| at >= m.addr && end <= m.addr + m.pages).copied()
    };
    let Some(m) = found else { set_errno(crate::ENOMEM); return -1 };
    let mut r = unsafe { write_back(&m) };
    if r == 0 && m.fd >= 0 && flags & MS_SYNC != 0 { r = unsafe { sys_fsync(m.fd) }; }
    ret(r as i64) as c_int
}

/// Writes every shared mapping back, as the end of a program unmaps them.
unsafe fn unmap_all() {
    let all: Vec<Mapping> = { let (_held, all) = maps(); core::mem::take(all) };
    for m in &all { unsafe { write_back(m) }; if m.fd >= 0 { unsafe { sys_close(m.fd) }; } }
}

/// Memory here is always readable and writable: protection changes are accepted.
#[no_mangle]
pub unsafe extern "C" fn mprotect(_addr: *mut c_void, _len: usize, _prot: c_int) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn madvise(_addr: *mut c_void, _len: usize, _advice: c_int) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn posix_madvise(_addr: *mut c_void, _len: usize, _advice: c_int) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn mlock(_addr: *const c_void, _len: usize) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn munlock(_addr: *const c_void, _len: usize) -> c_int { 0 }

// ---- ending ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn exit(code: c_int) -> ! {
    crate::cxx::run_at_exit();
    unsafe { flush_all(); unmap_all(); sys_exit(code) }
}
#[no_mangle]
pub unsafe extern "C" fn _exit(code: c_int) -> ! { unsafe { sys_exit(code) } }
#[no_mangle]
pub unsafe extern "C" fn _Exit(code: c_int) -> ! { unsafe { sys_exit(code) } }
#[no_mangle]
pub unsafe extern "C" fn abort() -> ! {
    unsafe { flush_all(); sys_abort() }
}
