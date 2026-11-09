// SPDX-License-Identifier: GPL-3.0-only
//! The C library for C code that runs inside a SexOS program: FFmpeg first.
//!
//! SexOS is not POSIX and this is not a port of one. It is the small C surface a
//! portable C library such as FFmpeg actually calls, written in Rust on top of the
//! program's own heap (`alloc`) and nothing else. There are no files, processes,
//! signals or threads here: a C library gets its input and output from the Rust
//! program that links it, through callbacks (FFmpeg's `AVIOContext`), never by
//! reaching the system itself. So C code inside a program holds exactly the
//! authority of that program's BOOT row, like the program's Rust code. Threads
//! (`pthread`) are the program's own workers, so they add cores, not authority.
//!
//! Headers are musl's (MIT), installed by `scripts/build_ffmpeg.sh`; `long double`
//! is `double` there, because the soft-float target has no x87.
//!
//! Symbols are exported only for the SexOS target (`target_os = "none"`). Host
//! tests call the same functions as ordinary Rust, so they never replace the host's
//! own `malloc` or `printf`.
#![cfg_attr(not(test), no_std)]
#![feature(c_variadic)]
#![allow(clippy::missing_safety_doc)]
#![allow(non_camel_case_types)]

extern crate alloc;

pub mod format;
pub mod mem;
pub mod stdio;
pub mod stdlib;
pub mod string;
pub mod math;
pub mod time;
pub mod pthread;
pub mod cxx;
pub mod locale;

pub use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

/// `errno`: one per program; SexOS programs that use this library are single-threaded.
static mut ERRNO: c_int = 0;

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn __errno_location() -> *mut c_int {
    core::ptr::addr_of_mut!(ERRNO)
}

pub(crate) fn set_errno(value: c_int) {
    unsafe { *__errno_location() = value };
}

pub const EINVAL: c_int = 22;
pub const ENOENT: c_int = 2;
pub const ENOMEM: c_int = 12;
pub const ERANGE: c_int = 34;
pub const ENOSYS: c_int = 38;
pub const EBADF: c_int = 9;

/// Where the program sends C diagnostics (`fprintf(stderr, ...)`, FFmpeg's log).
/// A program sets it once at start, usually to its serial record.
static mut LOG: Option<fn(&[u8])> = None;

pub fn set_log(sink: fn(&[u8])) {
    unsafe { LOG = Some(sink) };
}

/// Sets the log only if the program set none: for a library that links C code.
pub fn set_log_default(sink: fn(&[u8])) {
    unsafe { if (*core::ptr::addr_of!(LOG)).is_none() { LOG = Some(sink) } };
}

pub(crate) fn log(bytes: &[u8]) {
    if let Some(sink) = unsafe { LOG } {
        sink(bytes);
    }
}

/// Ends the program. Set by the program, which knows how it ends itself.
static mut EXIT: Option<fn(c_int) -> !> = None;

pub fn set_exit(exit: fn(c_int) -> !) {
    unsafe { EXIT = Some(exit) };
}

/// Sets the exit only if the program set none.
pub fn set_exit_default(exit: fn(c_int) -> !) {
    unsafe { if (*core::ptr::addr_of!(EXIT)).is_none() { EXIT = Some(exit) } };
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn abort() -> ! {
    log(b"abort() called");
    match unsafe { EXIT } {
        Some(exit) => exit(134),
        None => panic!("abort"),
    }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn exit(code: c_int) -> ! {
    cxx::run_at_exit();
    match unsafe { EXIT } {
        Some(exit) => exit(code),
        None => panic!("exit"),
    }
}

/// No environment: a program's settings come from its arguments and its own folder.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn getenv(_name: *const c_char) -> *mut c_char {
    core::ptr::null_mut()
}

/// Character set conversion is not offered; FFmpeg falls back to UTF-8 only.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn iconv_open(_to: *const c_char, _from: *const c_char) -> *mut c_void {
    set_errno(EINVAL);
    usize::MAX as *mut c_void
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn iconv(_cd: *mut c_void, _i: *mut *mut c_char, _il: *mut usize, _o: *mut *mut c_char, _ol: *mut usize) -> usize {
    set_errno(EINVAL);
    usize::MAX
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn iconv_close(_cd: *mut c_void) -> c_int {
    0
}

/// No file system path reaches C code; see the crate notes.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn open(_path: *const c_char, _flags: c_int, _: ...) -> c_int {
    set_errno(ENOENT);
    -1
}
