// SPDX-License-Identifier: GPL-3.0-only
//! `<pthread.h>` and `<sched.h>`: threads for C code, on the program's own workers
//! (`sex_app::thread`). A C thread is a worker: it shares the program's memory and
//! authority, runs on another core, and ends with the program. Nothing here reaches
//! past the program, so threads add speed, not power.
//!
//! Locks sleep in the kernel on an address (`program_threads` WAIT/WAKE), like a
//! futex. The C types are musl's sizes; only their first words are used:
//! mutex `[type, lock, owner, count]`, cond `[sequence]`, rwlock `[state]`,
//! attr `[stack bytes, detached]`.
use crate::{c_int, c_uint, c_void};
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use sex_app::thread::{self, JoinHandle};
use sex_pdx::program_threads as t;

pub const EBUSY: c_int = 16;
pub const EDEADLK: c_int = 35;
pub const EAGAIN: c_int = 11;
pub const EPERM: c_int = 1;
pub const ETIMEDOUT: c_int = 110;
pub const PTHREAD_MUTEX_NORMAL: c_int = 0;
pub const PTHREAD_MUTEX_RECURSIVE: c_int = 1;
pub const PTHREAD_MUTEX_ERRORCHECK: c_int = 2;
pub const PTHREAD_CREATE_DETACHED: c_int = 1;
pub const KEYS: usize = 128;

/// One C thread. The main task has a static one; each worker's lives on the heap
/// until it is joined (or, if detached, reaped after it finishes).
pub struct Thread {
    id: u32,
    result: *mut c_void,
    handle: Option<JoinHandle>,
    detached: bool,
    values: [*mut c_void; KEYS],
}
pub type pthread_t = *mut Thread;
type Start = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

static mut MAIN: Thread = Thread { id: 1, result: core::ptr::null_mut(), handle: None, detached: false, values: [core::ptr::null_mut(); KEYS] };
static NEXT_ID: AtomicU32 = AtomicU32::new(2);
/// The control-block word holding the calling worker's `Thread`.
const SELF_SLOT: usize = 1;

#[repr(C)] pub struct pthread_attr_t { words: [u64; 7] }
#[repr(C)] pub struct pthread_mutex_t { words: [u32; 10] }
#[repr(C)] pub struct pthread_mutexattr_t { kind: c_uint }
#[repr(C)] pub struct pthread_cond_t { words: [u32; 12] }
#[repr(C)] pub struct pthread_condattr_t { value: c_uint }
#[repr(C)] pub struct pthread_rwlock_t { words: [u32; 14] }
pub type pthread_once_t = c_int;
pub type pthread_key_t = c_uint;

fn atom(word: &u32) -> &AtomicU32 { unsafe { &*(word as *const u32 as *const AtomicU32) } }
fn wait(word: &AtomicU32, expected: u32) { t::wait(word.as_ptr(), expected); }
fn wake(word: &AtomicU32, count: u64) { t::wake(word.as_ptr(), count); }

#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn pthread_self() -> pthread_t {
    let slot = if sex_app::tls::pointer() == 0 { 0 } else { unsafe { *sex_app::tls::slot(SELF_SLOT) } };
    if slot == 0 { core::ptr::addr_of_mut!(MAIN) } else { slot as pthread_t }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn pthread_equal(a: pthread_t, b: pthread_t) -> c_int { (a == b) as c_int }

/// Detached threads that have finished: their stacks go back to the heap here.
static REAP: spin_lock::Lock = spin_lock::Lock::new();
static mut DETACHED: alloc::vec::Vec<pthread_t> = alloc::vec::Vec::new();
fn reap() {
    let _held = REAP.lock();
    let list = unsafe { &mut *core::ptr::addr_of_mut!(DETACHED) };
    list.retain(|&th| {
        let done = unsafe { (*th).handle.as_ref().map_or(true, |h| h.finished()) };
        if done { drop(unsafe { Box::from_raw(th) }); }
        !done
    });
}

struct Job(pthread_t, Start, *mut c_void);
unsafe impl Send for Job {}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_create(out: *mut pthread_t, attr: *const pthread_attr_t, start: Start, arg: *mut c_void) -> c_int {
    reap();
    let (stack, detached) = if attr.is_null() { (0, false) } else { unsafe { ((*attr).words[0] as usize, (*attr).words[1] != 0) } };
    let stack = if stack == 0 { thread::STACK_BYTES } else { stack };
    let th = Box::into_raw(Box::new(Thread { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), result: core::ptr::null_mut(), handle: None, detached, values: [core::ptr::null_mut(); KEYS] }));
    let job = Job(th, start, arg);
    let spawned = thread::spawn_with_stack(stack, move || {
        let job = job;
        unsafe {
            *sex_app::tls::slot(SELF_SLOT) = job.0 as usize;
            let result = (job.1)(job.2);
            finish(job.0, result);
        }
    });
    match spawned {
        Ok(handle) => {
            unsafe { (*th).handle = Some(handle); *out = th; }
            if detached { let _held = REAP.lock(); unsafe { (*core::ptr::addr_of_mut!(DETACHED)).push(th) }; }
            0
        }
        Err(_) => { drop(unsafe { Box::from_raw(th) }); EAGAIN }
    }
}

/// A thread's end: its result, then its keys' destructors, as POSIX orders them.
unsafe fn finish(th: pthread_t, result: *mut c_void) {
    unsafe { (*th).result = result; }
    for _round in 0..4 {
        let mut ran = false;
        for key in 0..KEYS {
            let value = unsafe { (*th).values[key] };
            let dtor = DESTRUCTORS[key].load(Ordering::Acquire);
            if !value.is_null() && dtor > 1 {
                unsafe { (*th).values[key] = core::ptr::null_mut(); }
                let f: unsafe extern "C" fn(*mut c_void) = unsafe { core::mem::transmute(dtor) };
                unsafe { f(value) };
                ran = true;
            }
        }
        if !ran { break; }
    }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_exit(result: *mut c_void) -> ! {
    let th = pthread_self();
    unsafe { finish(th, result) };
    thread::exit()
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_join(th: pthread_t, result: *mut *mut c_void) -> c_int {
    if th == pthread_self() { return EDEADLK; }
    if th.is_null() || unsafe { (*th).detached } { return crate::EINVAL; }
    let handle = unsafe { (*th).handle.take() };
    if let Some(h) = handle { h.join(); }
    if !result.is_null() { unsafe { *result = (*th).result; } }
    drop(unsafe { Box::from_raw(th) });
    0
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_detach(th: pthread_t) -> c_int {
    if th.is_null() || unsafe { (*th).detached } { return crate::EINVAL; }
    unsafe { (*th).detached = true; }
    let _held = REAP.lock();
    unsafe { (*core::ptr::addr_of_mut!(DETACHED)).push(th) };
    0
}

#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn sched_yield() -> c_int { sex_pdx::sys_yield(); 0 }

// ── attributes ──────────────────────────────────────────────────────────────

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_attr_init(a: *mut pthread_attr_t) -> c_int { unsafe { (*a).words = [0; 7] }; 0 }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_attr_destroy(_a: *mut pthread_attr_t) -> c_int { 0 }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_attr_setstacksize(a: *mut pthread_attr_t, bytes: usize) -> c_int {
    if bytes < 16 * 1024 { return crate::EINVAL; }
    unsafe { (*a).words[0] = bytes as u64 }; 0
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_attr_getstacksize(a: *const pthread_attr_t, out: *mut usize) -> c_int {
    let b = unsafe { (*a).words[0] } as usize;
    unsafe { *out = if b == 0 { thread::STACK_BYTES } else { b } }; 0
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_attr_setdetachstate(a: *mut pthread_attr_t, state: c_int) -> c_int {
    unsafe { (*a).words[1] = (state == PTHREAD_CREATE_DETACHED) as u64 }; 0
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_attr_getdetachstate(a: *const pthread_attr_t, out: *mut c_int) -> c_int {
    unsafe { *out = (*a).words[1] as c_int }; 0
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_setname_np(_th: pthread_t, _name: *const crate::c_char) -> c_int { 0 }

// ── mutexes ─────────────────────────────────────────────────────────────────
// lock word: 0 free, 1 held, 2 held with sleepers.

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_init(a: *mut pthread_mutexattr_t) -> c_int { unsafe { (*a).kind = 0 }; 0 }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_destroy(_a: *mut pthread_mutexattr_t) -> c_int { 0 }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_settype(a: *mut pthread_mutexattr_t, kind: c_int) -> c_int {
    if !(0..=2).contains(&kind) { return crate::EINVAL; }
    unsafe { (*a).kind = kind as c_uint }; 0
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_mutex_init(m: *mut pthread_mutex_t, a: *const pthread_mutexattr_t) -> c_int {
    unsafe { (*m).words = [0; 10]; if !a.is_null() { (*m).words[0] = (*a).kind; } }
    0
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_mutex_destroy(_m: *mut pthread_mutex_t) -> c_int { 0 }

fn me() -> u32 { unsafe { (*pthread_self()).id } }

fn lock_word(lock: &AtomicU32) {
    if lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok() { return; }
    for _ in 0..100 {
        core::hint::spin_loop();
        if lock.load(Ordering::Relaxed) == 0 && lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok() { return; }
    }
    while lock.swap(2, Ordering::Acquire) != 0 { wait(lock, 2); }
}
fn unlock_word(lock: &AtomicU32) {
    if lock.swap(0, Ordering::Release) == 2 { wake(lock, 1); }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_mutex_lock(m: *mut pthread_mutex_t) -> c_int {
    let w = unsafe { &(*m).words };
    let kind = w[0] as c_int;
    if kind != PTHREAD_MUTEX_NORMAL && atom(&w[2]).load(Ordering::Relaxed) == me() {
        if kind == PTHREAD_MUTEX_ERRORCHECK { return EDEADLK; }
        atom(&w[3]).fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    lock_word(atom(&w[1]));
    atom(&w[2]).store(me(), Ordering::Relaxed);
    atom(&w[3]).store(1, Ordering::Relaxed);
    0
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_mutex_trylock(m: *mut pthread_mutex_t) -> c_int {
    let w = unsafe { &(*m).words };
    if w[0] as c_int == PTHREAD_MUTEX_RECURSIVE && atom(&w[2]).load(Ordering::Relaxed) == me() {
        atom(&w[3]).fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    if atom(&w[1]).compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() { return EBUSY; }
    atom(&w[2]).store(me(), Ordering::Relaxed);
    atom(&w[3]).store(1, Ordering::Relaxed);
    0
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_mutex_unlock(m: *mut pthread_mutex_t) -> c_int {
    let w = unsafe { &(*m).words };
    let kind = w[0] as c_int;
    if kind != PTHREAD_MUTEX_NORMAL {
        if atom(&w[2]).load(Ordering::Relaxed) != me() { return EPERM; }
        if atom(&w[3]).fetch_sub(1, Ordering::Relaxed) > 1 { return 0; }
    }
    atom(&w[2]).store(0, Ordering::Relaxed);
    unlock_word(atom(&w[1]));
    0
}

// ── condition variables ─────────────────────────────────────────────────────

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_condattr_init(a: *mut pthread_condattr_t) -> c_int { unsafe { (*a).value = 0 }; 0 }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_condattr_destroy(_a: *mut pthread_condattr_t) -> c_int { 0 }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_condattr_setclock(a: *mut pthread_condattr_t, clock: c_int) -> c_int { unsafe { (*a).value = clock as c_uint }; 0 }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_cond_init(c: *mut pthread_cond_t, a: *const pthread_condattr_t) -> c_int {
    unsafe { (*c).words = [0; 12]; if !a.is_null() { (*c).words[1] = (*a).value; } }
    0
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_cond_destroy(_c: *mut pthread_cond_t) -> c_int { 0 }

/// A mutex's recursion depth survives the wait: it is put back on relock.
unsafe fn release_for_wait(m: *mut pthread_mutex_t) -> u32 {
    let w = unsafe { &(*m).words };
    let depth = atom(&w[3]).load(Ordering::Relaxed);
    atom(&w[2]).store(0, Ordering::Relaxed);
    unlock_word(atom(&w[1]));
    depth
}
unsafe fn reacquire(m: *mut pthread_mutex_t, depth: u32) {
    let w = unsafe { &(*m).words };
    // Woken waiters may race: take the slow path's "sleepers" mark so none is lost.
    let lock = atom(&w[1]);
    while lock.swap(2, Ordering::Acquire) != 0 { wait(lock, 2); }
    atom(&w[2]).store(me(), Ordering::Relaxed);
    atom(&w[3]).store(depth, Ordering::Relaxed);
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_cond_wait(c: *mut pthread_cond_t, m: *mut pthread_mutex_t) -> c_int {
    let seq = atom(unsafe { &(*c).words[0] });
    let seen = seq.load(Ordering::Acquire);
    let depth = unsafe { release_for_wait(m) };
    wait(seq, seen);
    unsafe { reacquire(m, depth) };
    0
}

/// Sleeps in the kernel until signalled or until the deadline, on the condition's
/// clock (CLOCK_REALTIME unless `pthread_condattr_setclock` chose another).
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_cond_timedwait(c: *mut pthread_cond_t, m: *mut pthread_mutex_t, at: *const crate::time::timespec) -> c_int {
    let seq = atom(unsafe { &(*c).words[0] });
    let seen = seq.load(Ordering::Acquire);
    let clock = unsafe { (*c).words[1] } as c_int;
    let deadline = unsafe { (*at).tv_sec as i128 * 1_000_000_000 + (*at).tv_nsec as i128 };
    let depth = unsafe { release_for_wait(m) };
    let mut status = 0;
    while seq.load(Ordering::Acquire) == seen {
        let mut now = crate::time::timespec { tv_sec: 0, tv_nsec: 0 };
        unsafe { crate::time::clock_gettime(clock, &mut now) };
        let left = deadline - (now.tv_sec as i128 * 1_000_000_000 + now.tv_nsec as i128);
        if left <= 0 { status = ETIMEDOUT; break; }
        let ms = (left as u128).div_ceil(1_000_000).min(u32::MAX as u128) as u64;
        t::wait_for(seq.as_ptr(), seen, ms);
    }
    unsafe { reacquire(m, depth) };
    status
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_cond_signal(c: *mut pthread_cond_t) -> c_int {
    let seq = atom(unsafe { &(*c).words[0] });
    seq.fetch_add(1, Ordering::Release);
    wake(seq, 1);
    0
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_cond_broadcast(c: *mut pthread_cond_t) -> c_int {
    let seq = atom(unsafe { &(*c).words[0] });
    seq.fetch_add(1, Ordering::Release);
    wake(seq, u64::MAX);
    0
}

// ── read-write locks ────────────────────────────────────────────────────────
// state: readers in the low bits, WRITER when a writer holds it.

const WRITER: u32 = 1 << 31;

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_init(l: *mut pthread_rwlock_t, _a: *const c_void) -> c_int { unsafe { (*l).words = [0; 14] }; 0 }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_destroy(_l: *mut pthread_rwlock_t) -> c_int { 0 }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_rdlock(l: *mut pthread_rwlock_t) -> c_int {
    let s = atom(unsafe { &(*l).words[0] });
    loop {
        let v = s.load(Ordering::Relaxed);
        if v & WRITER == 0 { if s.compare_exchange_weak(v, v + 1, Ordering::Acquire, Ordering::Relaxed).is_ok() { return 0; } }
        else { wait(s, v); }
    }
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_tryrdlock(l: *mut pthread_rwlock_t) -> c_int {
    let s = atom(unsafe { &(*l).words[0] });
    let v = s.load(Ordering::Relaxed);
    if v & WRITER == 0 && s.compare_exchange(v, v + 1, Ordering::Acquire, Ordering::Relaxed).is_ok() { 0 } else { EBUSY }
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_wrlock(l: *mut pthread_rwlock_t) -> c_int {
    let s = atom(unsafe { &(*l).words[0] });
    loop {
        match s.compare_exchange_weak(0, WRITER, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => return 0,
            Err(v) => wait(s, v),
        }
    }
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_trywrlock(l: *mut pthread_rwlock_t) -> c_int {
    let s = atom(unsafe { &(*l).words[0] });
    if s.compare_exchange(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok() { 0 } else { EBUSY }
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_unlock(l: *mut pthread_rwlock_t) -> c_int {
    let s = atom(unsafe { &(*l).words[0] });
    let v = s.load(Ordering::Relaxed);
    let after = if v & WRITER != 0 { s.store(0, Ordering::Release); 0 } else { s.fetch_sub(1, Ordering::Release) - 1 };
    if after == 0 { wake(s, u64::MAX); }
    0
}

// ── once and keys ───────────────────────────────────────────────────────────

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_once(once: *mut pthread_once_t, init: unsafe extern "C" fn()) -> c_int {
    let s = unsafe { &*(once as *const AtomicU32) };
    loop {
        match s.compare_exchange(0, 1, Ordering::Acquire, Ordering::Acquire) {
            Ok(_) => { unsafe { init() }; s.store(2, Ordering::Release); wake(s, u64::MAX); return 0; }
            Err(2) => return 0,
            Err(v) => wait(s, v),
        }
    }
}

/// 0: free; 1: in use with no destructor; otherwise the destructor's address.
static DESTRUCTORS: [AtomicUsize; KEYS] = [const { AtomicUsize::new(0) }; KEYS];

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_key_create(key: *mut pthread_key_t, dtor: Option<unsafe extern "C" fn(*mut c_void)>) -> c_int {
    let value = dtor.map_or(1, |f| f as usize);
    for (i, d) in DESTRUCTORS.iter().enumerate() {
        if d.compare_exchange(0, value, Ordering::AcqRel, Ordering::Relaxed).is_ok() { unsafe { *key = i as pthread_key_t }; return 0; }
    }
    EAGAIN
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_key_delete(key: pthread_key_t) -> c_int {
    match DESTRUCTORS.get(key as usize) { Some(d) => { d.store(0, Ordering::Release); 0 } None => crate::EINVAL }
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_getspecific(key: pthread_key_t) -> *mut c_void {
    if key as usize >= KEYS { return core::ptr::null_mut(); }
    unsafe { (*pthread_self()).values[key as usize] }
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn pthread_setspecific(key: pthread_key_t, value: *const c_void) -> c_int {
    if key as usize >= KEYS { return crate::EINVAL; }
    unsafe { (*pthread_self()).values[key as usize] = value as *mut c_void };
    0
}

/// `sysconf(_SC_NPROCESSORS_ONLN)` and friends: the cores this program's threads use.
pub fn cores() -> usize { thread::cores() }

mod spin_lock {
    use core::sync::atomic::{AtomicBool, Ordering};
    pub struct Lock(AtomicBool);
    pub struct Held<'a>(&'a Lock);
    impl Lock {
        pub const fn new() -> Self { Lock(AtomicBool::new(false)) }
        pub fn lock(&self) -> Held<'_> {
            while self.0.swap(true, Ordering::Acquire) { sex_pdx::sys_yield(); }
            Held(self)
        }
    }
    impl Drop for Held<'_> { fn drop(&mut self) { self.0 .0.store(false, Ordering::Release); } }
}
