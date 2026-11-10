// SPDX-License-Identifier: GPL-3.0-only
//! Messages one task took from the program's inbox that belong to another.
//!
//! A program's tasks share one inbox. The main task waits on it for keys and closes,
//! and any task waits on it for the answer to its own file or network request, so a
//! task can be handed a message meant for another: a worker waiting for its file can
//! receive a key, and the main task waiting for a key can receive a worker's file
//! answer. The task that takes it puts it here, and the task it belongs to looks here
//! before it waits again. The box is small and keeps the newest messages: what falls
//! out is an answer nobody is waiting for any more.
use core::sync::atomic::{AtomicBool, Ordering};
use sex_pdx::PdxMessage;

const SLOTS: usize = 32;
static LOCK: AtomicBool = AtomicBool::new(false);
static mut BOX: [Option<(u64, PdxMessage)>; SLOTS] = [None; SLOTS];
static mut NEXT: u64 = 0;

fn with<R>(f: impl FnOnce(&mut [Option<(u64, PdxMessage)>; SLOTS], &mut u64) -> R) -> R {
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() { core::hint::spin_loop(); }
    // Only the holder of LOCK touches the box.
    let r = unsafe { f(&mut *core::ptr::addr_of_mut!(BOX), &mut *core::ptr::addr_of_mut!(NEXT)) };
    LOCK.store(false, Ordering::Release);
    r
}

/// Keeps `m` for the task it belongs to. When the box is full the oldest goes.
pub fn put(m: PdxMessage) {
    with(|slots, next| {
        *next += 1;
        let at = slots.iter().position(Option::is_none)
            .unwrap_or_else(|| slots.iter().enumerate().min_by_key(|(_, s)| s.map_or(0, |s| s.0)).map_or(0, |(i, _)| i));
        slots[at] = Some((*next, m));
    })
}

/// The oldest kept message that `wanted` accepts, taken out of the box.
pub fn take(wanted: impl Fn(&PdxMessage) -> bool) -> Option<PdxMessage> {
    with(|slots, _| {
        let at = slots.iter().enumerate()
            .filter_map(|(i, s)| s.filter(|s| wanted(&s.1)).map(|s| (i, s.0)))
            .min_by_key(|(_, order)| *order)?.0;
        slots[at].take().map(|s| s.1)
    })
}
