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
/// If a worker cannot start, its share runs on the caller instead.
pub fn parallel(count: usize, f: &(dyn Fn(usize) + Sync)) {
    if count == 0 { return; }
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
