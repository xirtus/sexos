// SPDX-License-Identifier: GPL-3.0-only
//! More tasks for a running program: workers that share its memory, its key and its
//! authority, and run on other cores. Like `program_memory`, asking mints nothing: a
//! worker can do only what the program could do itself, and it ends with the run.
//!
//! The call is self-only. Workers belong to the run that started them: when the run
//! ends (its main task finishes, faults, or is closed) every worker ends too, and a
//! worker that faults ends the whole run.
//!
//! rdi = op, rsi/rdx/r10 = arguments → rax = status, rsi = result.
pub const SYS_PROGRAM_THREAD: u64 = 83;

/// rsi = entry, rdx = stack top, r10 = argument (arrives in rdi) → rsi = worker id.
/// The entry is called with RSP = 8 mod 16, as if by `call`; it must never return.
pub const SPAWN: u64 = 1;
/// rsi = address of a u32 or 0. Ends the calling worker. After it can no longer run,
/// the kernel stores 1 at the address and wakes every waiter there, so a joiner may
/// free the worker's stack as soon as it sees the 1. Never returns.
pub const EXIT: u64 = 2;
/// rsi = address of a u32, rdx = expected value. Sleeps until a WAKE on the address,
/// unless the value already differs (AGAIN). Wakes may be spurious: check again.
pub const WAIT: u64 = 3;
/// rsi = address, rdx = most waiters to wake → rsi = waiters woken.
pub const WAKE: u64 = 4;
/// → rsi = cores this run's workers are spread over (1 when all share one core).
pub const CORES: u64 = 5;

pub const OK: u64 = 0;
pub const INVALID: u64 = 1;
pub const RETRY: u64 = 2;
/// Not this row's current run.
pub const DENIED: u64 = 3;
/// The run already has `MAX_WORKERS` live workers.
pub const FULL: u64 = 6;
/// WAIT: the value had already changed.
pub const AGAIN: u64 = 8;

/// Live workers a run may have at once.
pub const MAX_WORKERS: usize = 63;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
#[inline]
pub fn call(op: u64, a: u64, b: u64, c: u64) -> (u64, u64) {
    let (status, out): (u64, u64);
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") SYS_PROGRAM_THREAD => status,
            inlateout("rdi") op => _,
            inlateout("rsi") a => out, inlateout("rdx") b => _,
            inlateout("r10") c => _, lateout("r8") _,
            out("rcx") _, out("r11") _, options(nostack));
    }
    (status, out)
}
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
pub fn call(_op: u64, _a: u64, _b: u64, _c: u64) -> (u64, u64) { (u64::MAX, 0) }

/// Starts a worker at `entry` on the stack ending at `stack_top`, with `arg` in rdi.
pub fn spawn(entry: u64, stack_top: u64, arg: u64) -> Result<u64, u64> {
    loop {
        match call(SPAWN, entry, stack_top, arg) {
            (OK, id) => return Ok(id),
            (RETRY, _) => crate::sys_yield(),
            (e, _) => return Err(e),
        }
    }
}
/// Ends the calling worker; stores 1 at `done` (if not null) once it has stopped.
pub fn exit(done: *const u32) -> ! {
    call(EXIT, done as u64, 0, 0);
    // Only reached if the caller is not a worker: end like any task.
    crate::task_finish(1)
}
/// Sleeps while `*at == expected`. Returns false if it had already changed.
pub fn wait(at: *const u32, expected: u32) -> bool {
    loop {
        match call(WAIT, at as u64, expected as u64, 0) {
            (OK, _) => return true,
            (RETRY, _) => crate::sys_yield(),
            _ => return false,
        }
    }
}
/// Wakes up to `count` waiters on `at`; returns how many woke.
pub fn wake(at: *const u32, count: u64) -> u64 {
    loop {
        match call(WAKE, at as u64, count, 0) {
            (OK, n) => return n,
            (RETRY, _) => crate::sys_yield(),
            _ => return 0,
        }
    }
}
/// Cores the run's workers are spread over.
pub fn cores() -> u64 {
    match call(CORES, 0, 0, 0) { (OK, n) => n.max(1), _ => 1 }
}
