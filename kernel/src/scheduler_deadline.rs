// SPDX-License-Identifier: GPL-3.0-only
//! Deadline class (stage 7b). Work that must be done by a time (a sound buffer refill,
//! a display frame) names a period; each wake must finish within one period of it.
//! Waiting deadline tasks run earliest deadline first, ahead of interactive and normal
//! work, and preempt it on wake. Deadline work on one core may take `SHARE_PERCENT` of
//! each window; past that it waits behind everything else until the window rolls, so it
//! can never starve the rest of the machine. Host-tested (tests/scheduler_deadline.rs).
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

/// Deadline tasks waiting on one core at once; a further wake waits in the normal queue.
pub const SLOTS: usize = 8;
/// Share of each window that deadline work may take on one core.
pub const SHARE_PERCENT: u64 = 50;
/// Window length in timer ticks (milliseconds).
pub const WINDOW_TICKS: u64 = 10;
/// Periods a task may ask for, in microseconds.
pub const PERIOD_MIN_US: u64 = 500;
pub const PERIOD_MAX_US: u64 = 100_000;

/// The waiting deadline tasks of one core. Any core publishes; only the owner takes.
pub struct DeadlineSet<T> {
    slots: [AtomicPtr<T>; SLOTS],
    at: [AtomicU64; SLOTS],
}

impl<T> DeadlineSet<T> {
    pub const fn new() -> Self {
        Self { slots: [const { AtomicPtr::new(ptr::null_mut()) }; SLOTS], at: [const { AtomicU64::new(0) }; SLOTS] }
    }
    fn reserved() -> *mut T { 1usize as *mut T }

    /// Publishes `task`, due at `deadline`, or returns it when every slot is taken.
    pub fn try_push(&self, task: *mut T, deadline: u64) -> Result<(), *mut T> {
        for (slot, at) in self.slots.iter().zip(&self.at) {
            if slot.compare_exchange(ptr::null_mut(), Self::reserved(), Ordering::Acquire, Ordering::Relaxed).is_ok() {
                at.store(deadline, Ordering::Relaxed);
                slot.store(task, Ordering::Release);
                return Ok(());
            }
        }
        Err(task)
    }

    /// The waiting task due first; null if none.
    pub fn take(&self) -> *mut T {
        loop {
            let mut best: Option<(usize, u64, *mut T)> = None;
            for (i, slot) in self.slots.iter().enumerate() {
                let task = slot.load(Ordering::Acquire);
                if task.is_null() || task == Self::reserved() { continue; }
                let at = self.at[i].load(Ordering::Relaxed);
                if best.map_or(true, |(_, b, _)| (at.wrapping_sub(b) as i64) < 0) { best = Some((i, at, task)); }
            }
            let Some((i, _, task)) = best else { return ptr::null_mut() };
            if self.slots[i].compare_exchange(task, ptr::null_mut(), Ordering::AcqRel, Ordering::Relaxed).is_ok() { return task; }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|s| { let t = s.load(Ordering::Relaxed); t.is_null() || t == Self::reserved() })
    }
}

/// How much of the current window deadline work on one core has used (TSC cycles).
pub struct Budget {
    win_tick: AtomicU64,
    win_tsc: AtomicU64,
    /// The last full window in cycles; before the first, 10 ms at 2 GHz.
    win_len: AtomicU64,
    used: AtomicU64,
    since: AtomicU64,
    running: AtomicBool,
}

impl Budget {
    pub const fn new() -> Self {
        Self { win_tick: AtomicU64::new(0), win_tsc: AtomicU64::new(0), win_len: AtomicU64::new(20_000_000),
               used: AtomicU64::new(0), since: AtomicU64::new(0), running: AtomicBool::new(false) }
    }

    /// The core switches at `tsc` to a task that is (or is not) deadline work.
    pub fn switched(&self, tick: u64, tsc: u64, deadline: bool) {
        if self.running.swap(deadline, Ordering::Relaxed) {
            self.used.fetch_add(tsc.wrapping_sub(self.since.load(Ordering::Relaxed)), Ordering::Relaxed);
        }
        self.since.store(tsc, Ordering::Relaxed);
        if tick.wrapping_sub(self.win_tick.load(Ordering::Relaxed)) >= WINDOW_TICKS {
            let start = self.win_tsc.swap(tsc, Ordering::Relaxed);
            if start != 0 { self.win_len.store(tsc.wrapping_sub(start).max(1), Ordering::Relaxed); }
            self.win_tick.store(tick, Ordering::Relaxed);
            self.used.store(0, Ordering::Relaxed);
        }
    }

    pub fn exhausted(&self) -> bool {
        self.used.load(Ordering::Relaxed) * 100 > self.win_len.load(Ordering::Relaxed) * SHARE_PERCENT
    }
    /// Deadline work runs on this core now.
    pub fn running(&self) -> bool { self.running.load(Ordering::Relaxed) }
    /// A period in microseconds as cycles of this core's clock.
    pub fn cycles(&self, us: u64) -> u64 {
        us.saturating_mul(self.win_len.load(Ordering::Relaxed) / (WINDOW_TICKS * 1000)).max(1)
    }
}

/// Built-in deadline work by domain, as periods in microseconds: sexdisplay a 60 Hz
/// frame, sexbell a sound buffer refill. Other tasks ask with `sex_pdx::sched`.
pub fn default_period_us(pd: u64) -> u64 {
    match pd { 1 => 16_667, 10 => 5_000, _ => 0 }
}

/// A period asked for through syscall 104, clamped; 0 leaves the class.
pub fn clamp_period_us(us: u64) -> u64 {
    if us == 0 { 0 } else { us.clamp(PERIOD_MIN_US, PERIOD_MAX_US) }
}
