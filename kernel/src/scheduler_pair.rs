//! Pair affinity (kernel pass 2, K3c): a program that keeps calling one partner on
//! another core moves to that core, so each round trip is a same-core handoff and
//! not two cross-core interrupts.
//!
//! Every answer to a call (`ipc::router::send_reply`, IPC v2 replies) is noted here
//! for the task it wakes, with the core the answer came from. After
//! `MOVE_STREAK` answers in a row from one other core the task's home becomes that
//! core; it takes effect at the task's next admission (`Scheduler::admit` reads the
//! home each time). A task that talks to partners on two cores never builds a
//! streak, so it stays where it is.
//!
//! Only programs and apps move, only between the cores programs use (2 and up), never
//! a row that shares its key (those stay on one core), never records or services, and
//! only off a core busier than the average program core onto a less busy one. A
//! program's start-up chatter with Files, on quiet cores, moves nothing.
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use super::{Task, SCHEDULERS};

/// Answers in a row from one other core before a task moves there.
pub const MOVE_STREAK: u32 = 64;
/// Moves made, and the first few are logged (`[sched.pair.move]`).
pub static MOVES: AtomicU64 = AtomicU64::new(0);
const LOGGED_MOVES: u64 = 32;

/// Per domain: the core its last answer came from (high byte, +1; 0 = none) and how
/// many answers in a row came from it (low 24 bits).
static STREAKS: [AtomicU32; crate::ipc::MAX_DOMAINS] = [const { AtomicU32::new(0) }; crate::ipc::MAX_DOMAINS];

fn home_of(task: &Task) -> u32 {
    match task.home.load(Ordering::Acquire) { u32::MAX => crate::smp::home_core(task.context.pd_id), core => core }
}

/// Whether domain `pd` may run on program core `core`.
pub fn movable(pd: u64, core: u32) -> bool {
    let spare = crate::smp::program_cores();
    if spare < 2 || core < 2 || core >= 2 + spare { return false; }
    if crate::program_records::is_record(pd) { return false; }
    // Apps and program rows only: services keep the cores they were proven on.
    if sex_pdx::app_catalog::app_of(pd as u32).is_none() { return false; }
    !crate::program_tenancy::shared(pd as u32)
}

/// Next streak word after an answer from `core` (`MOVE_STREAK` reached: due).
pub fn advance(word: u32, core: u32) -> (u32, bool) {
    let tag = (core + 1) << 24;
    if word & 0xFF00_0000 != tag { return (tag | 1, MOVE_STREAK <= 1); }
    let n = (word & 0x00FF_FFFF) + 1;
    if n >= MOVE_STREAK { (tag, true) } else { (tag | n, false) }
}

/// `home` is busier than the average program core and `to` is less busy than `home`.
fn crowded(home: u32, to: u32) -> bool {
    let spare = crate::smp::program_cores();
    if home < 2 || home >= 2 + spare { return false; }
    let load = |core: u32| SCHEDULERS[core as usize].load();
    let total: usize = (2..2 + spare).map(load).sum();
    let mine = load(home);
    mine * spare as usize > total && load(to) < mine
}

/// `task` is woken by an answer from the task running on this core.
pub fn note_answer(task: *mut Task) {
    if task.is_null() { return; }
    let task = unsafe { &*task };
    let pd = task.context.pd_id;
    let Some(slot) = STREAKS.get(pd as usize) else { return };
    let here = crate::core_local::CoreLocal::get().core_id;
    let home = home_of(task);
    if home == here { slot.store(0, Ordering::Relaxed); return; }
    let (word, due) = advance(slot.load(Ordering::Relaxed), here);
    slot.store(word, Ordering::Relaxed);
    if !due || !movable(pd, here) { return; }
    if !crowded(home, here) { return; }
    task.home.store(here, Ordering::Release);
    let n = MOVES.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= LOGGED_MOVES { crate::serial_println!("[sched.pair.move] pd={} from={} to={} moves={}", pd, home, here, n); }
}

/// Wakes `task` with an answer to its call: its home core runs it next, this core or
/// another (the handoff slot, with a preempt), and the answer counts toward a move.
pub fn wake_answer(task: *mut Task) {
    note_answer(task);
    let sched = &SCHEDULERS[crate::core_local::CoreLocal::get().core_id as usize];
    sched.arm_handoff(true);
    super::unpark_thread(task);
    sched.arm_handoff(false);
}
