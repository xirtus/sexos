// SPDX-License-Identifier: GPL-3.0-only
//! Workers: more tasks for a running program (`sex_pdx::program_threads`).
//!
//! A worker shares its program's domain, so it has the same key, memory and
//! authority; starting one mints nothing. It belongs to one run: when the run ends
//! every worker is told to exit, and the run cannot be retired (its frames reused)
//! until every worker is Dead. A worker that faults ends the whole run.
//!
//! Worker tasks are kept and reused: a queue may still hold a pointer to a task that
//! just died, so a task's memory is never freed, only re-armed once it is Dead.
use crate::scheduler::{task_ownership::{ParkResult, Phase}, Task, SCHEDULERS};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use sex_pdx::program_threads as t;
use spin::Mutex;

/// `dead_since`: the tick a spawn first saw the task Dead, 0 if not seen. A task is
/// reused only a few ticks after that, past the moment the core that released it
/// last touches it.
struct Worker { target: u32, run: u32, task: *mut Task, dead_since: u64 }
struct Waiter { target: u32, task: *mut Task, at: u64 }
struct Table { workers: Vec<Worker>, waiters: Vec<Waiter> }
// Task pointers are owned by this table for good; tasks synchronise through their atomics.
unsafe impl Send for Table {}
const REUSE_TICKS: u64 = 4;
static TABLE: Mutex<Table> = Mutex::new(Table { workers: Vec::new(), waiters: Vec::new() });

/// Interrupts are off in syscalls; a holder on another core may be waiting for this
/// one to answer a shootdown, so answer while waiting.
fn table() -> spin::MutexGuard<'static, Table> {
    loop {
        if let Some(held) = TABLE.try_lock() { return held; }
        crate::smp::tlb::service();
        core::hint::spin_loop();
    }
}

fn dead(task: *mut Task) -> bool {
    let s = unsafe { (*task).state.snapshot() };
    matches!(s.phase, Phase::Constructed | Phase::Dead) && s.owner_core.is_none()
}

/// The core for a run's `n`th worker: spread over the cores programs use, away from
/// the main task's own. Rows that share a key stay on their one core.
fn core_for(target: u32, n: usize) -> u32 {
    let home = crate::smp::home_core(target as u64);
    if crate::program_tenancy::shared(target) { return home; }
    let spare = crate::smp::program_cores();
    if spare <= 1 { return home; }
    2 + ((home - 2) + 1 + n as u32) % spare
}

/// Cores this run's workers use.
fn cores(target: u32) -> u64 {
    if crate::program_tenancy::shared(target) { 1 } else { crate::smp::program_cores().max(1) as u64 }
}

/// Starts a worker for `target`'s run `run`. The caller holds the program table and
/// has checked that `run` is the row's live run, so no end can race this.
pub fn spawn(pd: &crate::capability::ProtectionDomain, run: u32, id: u32, entry: u64, stack: u64, arg: u64) -> Result<u64, u64> {
    if entry == 0 || stack < 16 { return Err(t::INVALID); }
    let mut table = table();
    let live = table.workers.iter().filter(|w| w.target == pd.id && w.run == run && !dead(w.task)).count();
    if live >= t::MAX_WORKERS { return Err(t::FULL); }
    let n = table.workers.iter().filter(|w| w.target == pd.id && w.run == run).count();
    // Reuse a task of this row that has been Dead for a while, or make one.
    let now = crate::interrupts::TICKS.load(Ordering::Relaxed).max(1);
    for w in table.workers.iter_mut().filter(|w| w.target == pd.id) {
        if !dead(w.task) { w.dead_since = 0; } else if w.dead_since == 0 { w.dead_since = now; }
    }
    let task = match table.workers.iter_mut().find(|w| w.target == pd.id && w.dead_since != 0 && now - w.dead_since >= REUSE_TICKS) {
        Some(w) => { w.run = run; w.dead_since = 0; w.task }
        None => {
            let task = Box::leak(Box::new(Task::with_kernel_stack(id, entry, stack, pd, true, 65536))) as *mut Task;
            table.workers.push(Worker { target: pd.id, run, task, dead_since: 0 });
            task
        }
    };
    unsafe {
        (*task).rearm(id, entry, stack, pd);
        // The argument arrives in rdi: the forged frame's seventh slot from its base.
        let frame = (*task).context.kstack_top as *mut u64;
        *frame.add(6) = arg;
        (*task).context.rdi = arg;
        (*task).home.store(core_for(pd.id, n), Ordering::Release);
    }
    drop(table);
    let core = crate::core_local::CoreLocal::get().core_id as usize;
    if !SCHEDULERS[core].activate_task(task) { return Err(t::DENIED); }
    crate::serial_println!("[program.worker.start] target={} run={} worker={} core={}", pd.id, run, id, unsafe { (*task).home.load(Ordering::Relaxed) });
    Ok(id as u64)
}

/// Whether `task` is a worker; if so, its row.
fn worker_row(task: *mut Task) -> Option<u32> {
    table().workers.iter().find(|w| w.task == task).map(|w| w.target)
}

/// The run of `target` has ended: every worker is told to exit. Blocked and queued
/// ones die without running again; one running on a core is switched out at its next tick.
pub fn end(target: u32) {
    let mut table = table();
    table.waiters.retain(|w| w.target != target);
    for w in table.workers.iter().filter(|w| w.target == target) {
        unsafe {
            if (*w.task).state.exit() {
                crate::scheduler::unpark_thread(w.task);
                crate::serial_println!("[program.worker.end] target={} run={} worker={}", target, w.run, (*w.task).id);
            }
        }
    }
}

/// Every worker of `target` is Dead (or never started), so the run's memory may go.
pub fn all_dead(target: u32) -> bool {
    table().workers.iter().filter(|w| w.target == target).all(|w| dead(w.task))
}

/// A task faulted and is being ended. A worker's fault ends its whole run.
pub fn faulted(task: *mut Task) {
    let Some(target) = worker_row(task) else { return };
    let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(target) else { return };
    let main = pd.main_task.load(Ordering::Acquire);
    if !main.is_null() && unsafe { (*main).state.exit() } {
        crate::scheduler::unpark_thread(main);
        crate::serial_println!("[program.worker.fault] target={} worker={} run_ended=1", target, unsafe { (*task).id });
    }
}

fn wake_at(table: &mut Table, target: u32, at: u64, most: u64) -> u64 {
    let mut woken = 0;
    let mut i = 0;
    while i < table.waiters.len() && woken < most {
        let w = &table.waiters[i];
        if w.target == target && w.at == at {
            let task = table.waiters.swap_remove(i).task;
            unsafe { (*task).wait_deadline.cancel(); }
            crate::scheduler::unpark_thread(task);
            woken += 1;
        } else { i += 1; }
    }
    woken
}

/// Core 0's timer, every tick: timed waits whose deadline has passed wake up. Skipped
/// if the table is busy; the next tick looks again.
pub fn poll(now: u64) {
    let Some(mut table) = TABLE.try_lock() else { return };
    let mut i = 0;
    while i < table.waiters.len() {
        let task = table.waiters[i].task;
        if unsafe { (*task).wait_deadline.take_due(now) }.is_some() {
            table.waiters.swap_remove(i);
            crate::scheduler::unpark_thread(task);
        } else { i += 1; }
    }
}

/// Syscall 83. May not return (EXIT, or WAIT that sleeps).
pub fn call(regs: &mut crate::interrupts::SyscallRegs, op: u64, a: u64, b: u64, c: u64) -> u64 {
    let core = crate::core_local::CoreLocal::get();
    let pd = core.current_pd_ref();
    let current = SCHEDULERS[core.core_id as usize].current_task.load(Ordering::Acquire);
    let reply = |regs: &mut crate::interrupts::SyscallRegs, status: u64, out: u64| { regs.rsi = out; status };
    match op {
        t::SPAWN => match crate::app_resources::spawn_worker(pd, a, b, c) {
            Ok(id) => reply(regs, t::OK, id),
            Err(e) => reply(regs, e, 0),
        },
        t::CORES => {
            if crate::app_resources::live_run(pd).is_err() { return reply(regs, t::DENIED, 0); }
            reply(regs, t::OK, cores(pd.id))
        }
        t::EXIT => {
            if current.is_null() || worker_row(current) != Some(pd.id) { return reply(regs, t::DENIED, 0); }
            // The worker stops before its joiner hears: once this task is exited no core
            // runs it again, and the store and wake below happen in its last syscall.
            unsafe { (*current).state.exit(); }
            if a != 0 && a % 4 == 0 && crate::syscalls::user_bytes_writable(a, 4) {
                unsafe { core::ptr::write_volatile(a as *mut u32, 1); }
                let mut table = table();
                wake_at(&mut table, pd.id, a, u64::MAX);
            }
            // A clean end: reason 0, so the run goes on.
            regs.rdi = 0;
            unsafe { crate::scheduler::finish_current_task(regs as *const _) }
        }
        t::WAIT => {
            if a % 4 != 0 || !crate::syscalls::user_bytes_readable(a, 4) || current.is_null() { return reply(regs, t::INVALID, 0); }
            if crate::app_resources::live_run(pd).is_err() { return reply(regs, t::DENIED, 0); }
            {
                let mut table = table();
                // Checked under the table lock, which every WAKE takes: a value changed
                // and woken after this read finds the waiter registered.
                if unsafe { core::ptr::read_volatile(a as *const u32) } as u64 != b & 0xFFFF_FFFF { return reply(regs, t::AGAIN, 0); }
                table.waiters.retain(|w| w.task != current);
                table.waiters.push(Waiter { target: pd.id, task: current, at: a });
                // A timed wait: `poll` wakes it once the deadline passes. Long waits wake
                // early at the deadline cap; the caller checks its clock and waits again.
                unsafe {
                    (*current).wait_deadline.cancel();
                    if c != 0 {
                        let ticks = c.min(crate::scheduler::self_wait::MAX_TICKS);
                        (*current).wait_deadline.arm(crate::interrupts::TICKS.load(Ordering::Relaxed), ticks);
                    }
                }
            }
            regs.rsi = 0;
            match unsafe { (*current).state.park(core.core_id) } {
                ParkResult::WakeConsumed => {
                    unsafe { (*current).wait_deadline.cancel(); }
                    table().waiters.retain(|w| w.task != current);
                    t::OK
                }
                ParkResult::Blocked | ParkResult::Exited => {
                    regs.rax = t::OK;
                    unsafe { crate::scheduler::yield_and_switch(regs as *const _) }
                }
                ParkResult::NotOwner => panic!("PROGRAM_WAIT_WITHOUT_OWNER"),
            }
        }
        t::TLS => {
            // A user address only: the kernel never reads through FS, but a kernel
            // pointer here would be a value no user code should hold.
            if current.is_null() || a >= 0x0000_8000_0000_0000 { return reply(regs, t::INVALID, 0); }
            if crate::app_resources::live_run(pd).is_err() { return reply(regs, t::DENIED, 0); }
            unsafe {
                (*current).context.fs_base = a;
                x86_64::registers::model_specific::FsBase::write(x86_64::VirtAddr::new(a));
            }
            reply(regs, t::OK, 0)
        }
        t::WAKE => {
            if a % 4 != 0 { return reply(regs, t::INVALID, 0); }
            let mut table = table();
            let n = wake_at(&mut table, pd.id, a, b);
            reply(regs, t::OK, n)
        }
        _ => reply(regs, t::INVALID, 0),
    }
}
