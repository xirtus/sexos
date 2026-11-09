// SPDX-License-Identifier: GPL-3.0-only
//! What C++ code needs from the C library beyond C: running static constructors and
//! recording `atexit` work. A SexOS program ends by finishing its task, so exit-time
//! destructors are recorded and run only through `exit()`.
use crate::{c_int, c_void};

#[cfg(target_os = "none")]
unsafe extern "C" {
    static __init_array_start: [unsafe extern "C" fn(); 0];
    static __init_array_end: [unsafe extern "C" fn(); 0];
}

/// Runs the program's static constructors (`.init_array`) once, in order. A program
/// with C++ calls this at start, before any C++ code.
#[cfg(target_os = "none")]
pub unsafe fn run_constructors() {
    static DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    if DONE.swap(true, core::sync::atomic::Ordering::AcqRel) { return; }
    let mut at = core::ptr::addr_of!(__init_array_start) as *const unsafe extern "C" fn();
    let end = core::ptr::addr_of!(__init_array_end) as *const unsafe extern "C" fn();
    while at < end {
        unsafe { (*at)() };
        at = unsafe { at.add(1) };
    }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub static mut __dso_handle: *mut c_void = core::ptr::null_mut();

const MAX_EXIT: usize = 64;
static mut AT_EXIT: [(usize, usize); MAX_EXIT] = [(0, 0); MAX_EXIT];
static COUNT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Records a destructor for `exit()`. Static objects register here.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn __cxa_atexit(f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void, _dso: *mut c_void) -> c_int {
    let i = COUNT.fetch_add(1, core::sync::atomic::Ordering::AcqRel);
    if i >= MAX_EXIT { return 0; } // never run: the program ends without them
    unsafe { (*core::ptr::addr_of_mut!(AT_EXIT))[i] = (f as usize, arg as usize) };
    0
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn atexit(f: unsafe extern "C" fn()) -> c_int {
    unsafe extern "C" fn call(f: *mut c_void) { let f: unsafe extern "C" fn() = unsafe { core::mem::transmute(f) }; unsafe { f() } }
    unsafe { __cxa_atexit(call, f as *mut c_void, core::ptr::null_mut()) }
}

/// Runs recorded exit work, newest first. Called by `exit()`.
pub(crate) fn run_at_exit() {
    let n = COUNT.swap(0, core::sync::atomic::Ordering::AcqRel).min(MAX_EXIT);
    for i in (0..n).rev() {
        let (f, arg) = unsafe { (*core::ptr::addr_of!(AT_EXIT))[i] };
        let f: unsafe extern "C" fn(*mut c_void) = unsafe { core::mem::transmute(f) };
        unsafe { f(arg as *mut c_void) };
    }
}
