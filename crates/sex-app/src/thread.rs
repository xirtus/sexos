// SPDX-License-Identifier: GPL-3.0-only
//! Threads: work on more than one core. A worker shares the program's memory, so
//! anything `Send` can move to it and anything `Sync` can be shared with it.
//!
//! `parallel(n, f)` is the usual shape: it runs `f(0)` .. `f(n-1)` at once, `f(0)` on the
//! calling task and the rest on workers, and returns when all are done, so `f` may
//! borrow the caller's data (bands of an image, rows of a canvas). `spawn` starts one
//! worker for a `'static` job and `join` waits for it.
//!
//! A worker that panics ends the whole program, as the main task does. When the
//! program ends, its workers end with it. Each worker has its own copy of the
//! program's `#[thread_local]` statics (see `tls`).
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use sex_pdx::program_threads as t;

/// Stack for each worker. Heap memory, so deep recursion belongs on the main task.
pub const STACK_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { NoMemory, Refused(u64) }

struct Packet { done: AtomicU32, job: Option<Box<dyn FnOnce() + Send>>, tls: crate::tls::Block }

/// A started worker. Dropping it without `join` waits too: the worker's stack must
/// outlive it.
pub struct JoinHandle { packet: Box<Packet>, _stack: Vec<u128> }

extern "C" fn worker_main(packet: *mut Packet) -> ! {
    let packet = unsafe { &mut *packet };
    // Its own thread-locals first; a refusal ends the run as a fault would.
    if !packet.tls.install() { sex_pdx::task_finish(3) }
    // So `exit` can find where to say this worker is done.
    unsafe { *crate::tls::slot(DONE_SLOT) = packet.done.as_ptr() as usize; }
    if let Some(job) = packet.job.take() { job(); }
    // The kernel stores 1 in `done` only once this worker can no longer run.
    t::exit(packet.done.as_ptr())
}

/// The control-block word that holds the calling worker's `done` address.
const DONE_SLOT: usize = 2;

/// Ends the calling worker now, as if its job had returned; its `JoinHandle` sees it
/// finish. On the main task (or before any worker started) it ends the program.
pub fn exit() -> ! {
    let done = if crate::tls::pointer() == 0 { 0 } else { unsafe { *crate::tls::slot(DONE_SLOT) } };
    if done == 0 { sex_pdx::task_finish(0) }
    t::exit(done as *const u32)
}

/// Whether the calling task is a worker (not the program's main task).
pub fn is_worker() -> bool {
    crate::tls::pointer() != 0 && unsafe { *crate::tls::slot(DONE_SLOT) } != 0
}

/// Lets other tasks run while the caller waits for something it must look for itself:
/// a few yields first, then short sleeps (`round` counts the looks so far).
pub fn pause(round: u32) {
    static NEVER: AtomicU32 = AtomicU32::new(0);
    if round < 64 { sex_pdx::sys_yield(); } else { t::wait_for(NEVER.as_ptr(), 0, 1); }
}

/// How many cores workers spread over. `parallel` uses this many tasks by default.
pub fn cores() -> usize { t::cores() as usize }

/// Starts `job` on a new worker.
pub fn spawn<F: FnOnce() + Send + 'static>(job: F) -> Result<JoinHandle, Error> {
    spawn_with_stack(STACK_BYTES, job)
}

/// Starts `job` on a new worker with a stack of `bytes` (at least 16 KiB).
pub fn spawn_with_stack<F: FnOnce() + Send + 'static>(bytes: usize, job: F) -> Result<JoinHandle, Error> {
    let job: Box<dyn FnOnce() + Send> = Box::new(job);
    unsafe { spawn_boxed(job, bytes) }
}

/// # Safety
/// Whatever `job` borrows must outlive the worker: the caller joins before it ends.
unsafe fn spawn_boxed(job: Box<dyn FnOnce() + Send + '_>, bytes: usize) -> Result<JoinHandle, Error> {
    let job: Box<dyn FnOnce() + Send + 'static> = core::mem::transmute(job);
    let words = bytes.max(16 * 1024).div_ceil(16);
    let mut stack: Vec<u128> = Vec::new();
    stack.try_reserve_exact(words).map_err(|_| Error::NoMemory)?;
    stack.resize(words, 0);
    let top = stack.as_ptr() as u64 + words as u64 * 16;
    // The caller needs its own block before any worker exists (see `tls::init`).
    if !crate::tls::init() { return Err(Error::NoMemory); }
    let tls = crate::tls::Block::new().ok_or(Error::NoMemory)?;
    let mut packet = Box::new(Packet { done: AtomicU32::new(0), job: Some(job), tls });
    let arg = &mut *packet as *mut Packet as u64;
    match t::spawn(worker_main as extern "C" fn(*mut Packet) -> ! as usize as u64, top, arg) {
        Ok(_) => Ok(JoinHandle { packet, _stack: stack }),
        Err(e) => Err(Error::Refused(e)),
    }
}

impl JoinHandle {
    /// Waits for the worker to finish.
    pub fn join(self) { drop(self) }
    /// Whether the worker has finished, so dropping this handle will not wait.
    pub fn finished(&self) -> bool { self.packet.done.load(Ordering::Acquire) != 0 }
    fn wait(&self) {
        while self.packet.done.load(Ordering::Acquire) == 0 {
            t::wait(self.packet.done.as_ptr(), 0);
        }
    }
}
impl Drop for JoinHandle {
    fn drop(&mut self) { self.wait() }
}

/// Runs `f(i)` for every `i` in `0..count` at once and returns when all are done.
/// `f(0)` runs on the caller and each other index on its own worker. The workers stay
/// after the call and wait for the next one, so a call costs a wake, not a start;
/// their thread-locals carry over from call to call. A call made while another is
/// running (from a worker, or from two tasks at once) starts fresh workers instead.
/// If a worker cannot start, its share runs on the caller.
pub fn parallel(count: usize, f: &(dyn Fn(usize) + Sync)) {
    if count == 0 { return; }
    if count == 1 { return f(0); }
    if POOL_BUSY.swap(true, Ordering::Acquire) { return parallel_fresh(count, f); }
    // Only the holder of POOL_BUSY touches the pool's list.
    let slots = unsafe { &mut *core::ptr::addr_of_mut!(POOL) };
    while slots.len() < count - 1 {
        let slot: &'static Slot = Box::leak(Box::new(Slot { go: AtomicU32::new(0), index: slots.len() + 1 }));
        match spawn(move || pool_worker(slot)) {
            // The pool lasts as long as the program; its workers end with it.
            Ok(h) => { core::mem::forget(h); slots.push(slot); }
            Err(_) => { drop(unsafe { Box::from_raw(slot as *const Slot as *mut Slot) }); break; }
        }
    }
    let pooled = slots.len().min(count - 1);
    // The job outlives every worker's use of it: this call waits for them below.
    unsafe { *core::ptr::addr_of_mut!(POOL_JOB) = Some(core::mem::transmute::<&(dyn Fn(usize) + Sync), &'static (dyn Fn(usize) + Sync)>(f)); }
    POOL_LEFT.store(pooled as u32, Ordering::Release);
    for slot in &slots[..pooled] {
        slot.go.fetch_add(1, Ordering::Release);
        t::wake(slot.go.as_ptr(), 1);
    }
    f(0);
    for i in pooled + 1..count { f(i); }
    let mut spins = 0;
    loop {
        let left = POOL_LEFT.load(Ordering::Acquire);
        if left == 0 { break; }
        if spins < 2000 { core::hint::spin_loop(); spins += 1; } else { t::wait(POOL_LEFT.as_ptr(), left); }
    }
    unsafe { *core::ptr::addr_of_mut!(POOL_JOB) = None; }
    POOL_BUSY.store(false, Ordering::Release);
}

/// One pool worker's doorbell: `go` counts the calls it has been handed.
struct Slot { go: AtomicU32, index: usize }

static POOL_BUSY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static POOL_LEFT: AtomicU32 = AtomicU32::new(0);
static mut POOL: Vec<&'static Slot> = Vec::new();
static mut POOL_JOB: Option<&'static (dyn Fn(usize) + Sync)> = None;

fn pool_worker(slot: &'static Slot) {
    let mut seen = 0;
    loop {
        let mut go = slot.go.load(Ordering::Acquire);
        // A short spin first: back-to-back calls (frames, draws) rarely need a sleep.
        let mut spins = 0;
        while go == seen {
            if spins < 2000 { core::hint::spin_loop(); spins += 1; } else { t::wait(slot.go.as_ptr(), seen); }
            go = slot.go.load(Ordering::Acquire);
        }
        seen = go;
        if let Some(f) = unsafe { *core::ptr::addr_of!(POOL_JOB) } { f(slot.index); }
        if POOL_LEFT.fetch_sub(1, Ordering::AcqRel) == 1 { t::wake(POOL_LEFT.as_ptr(), 1); }
    }
}

/// `parallel` with workers started for this call alone.
fn parallel_fresh(count: usize, f: &(dyn Fn(usize) + Sync)) {
    let mut handles = Vec::with_capacity(count - 1);
    let mut inline = Vec::new();
    for i in 1..count {
        // The handles are joined below, before `f` can go out of scope.
        match unsafe { spawn_boxed(Box::new(move || f(i)), STACK_BYTES) } {
            Ok(h) => handles.push(h),
            Err(_) => inline.push(i),
        }
    }
    f(0);
    for i in inline { f(i); }
    drop(handles);
}
