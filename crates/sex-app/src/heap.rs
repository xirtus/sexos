// SPDX-License-Identifier: GPL-3.0-only
//! The program's heap, so `alloc` works: `Vec`, `String`, `Box`, `BTreeMap`, and the
//! many `no_std` crates that need them.
//!
//! The allocator is talc (MIT). Its memory is the run's own: the kernel hands out the
//! pages of the program row's BOOT window that follow the image, on the row's hardware
//! key, zeroed, and takes them back when the run ends (`sex_pdx::program_memory`).
//! Nothing is asked for until the first allocation, and the heap grows in steps as it
//! fills. When the window is used up an allocation fails like any out-of-memory:
//! `try_reserve` reports it, and an infallible one ends the program through its panic
//! handler. Freed memory is reused by this run; pages return only at exit.
use core::alloc::Layout;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};
use sex_pdx::program_memory as memory;
use talc::base::binning::Binning;
use talc::base::Talc;

/// Least the heap grows by at once, so small allocations don't each cost a call.
const STEP_PAGES: u64 = 16;

/// A spin lock. A program's tasks share one heap.
pub struct Lock(AtomicBool);
unsafe impl talc::lock_api::RawMutex for Lock {
    #[allow(clippy::declare_interior_mutable_const)]
    const INIT: Self = Lock(AtomicBool::new(false));
    type GuardMarker = talc::lock_api::GuardSend;
    fn lock(&self) { while !self.try_lock() { core::hint::spin_loop(); } }
    fn try_lock(&self) -> bool { self.0.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() }
    unsafe fn unlock(&self) { self.0.store(false, Ordering::Release); }
}

/// Where the heap's memory comes from: the run's window, one contiguous span that
/// only grows. `end` is the heap's end as talc last reported it.
#[derive(Debug)]
pub struct Window { end: Option<NonNull<u8>> }
// Only ever touched under the heap's lock.
unsafe impl Send for Window {}

impl Window {
    pub const fn new() -> Self { Self { end: None } }
}
impl Default for Window { fn default() -> Self { Self::new() } }

/// Asks for `pages` in calls the kernel accepts. Returns the run's memory after the
/// last call that succeeded, or None if not one did.
fn ask(pages: u64) -> Option<memory::Memory> {
    let mut left = pages;
    let mut last = None;
    while left > 0 {
        let n = left.min(memory::MAX_PAGES_PER_CALL);
        match memory::grow(n) { Ok(m) => last = Some(m), Err(_) => break }
        left -= n;
    }
    last
}

// The source never allocates, so it never re-enters the heap it feeds.
unsafe impl talc::source::Source for Window {
    fn acquire<B: Binning>(talc: &mut Talc<Self, B>, layout: Layout) -> Result<(), ()> {
        // Room for the block, its alignment, and talc's own bookkeeping.
        let bytes = layout.size().checked_add(layout.align()).and_then(|b| b.checked_add(1024)).ok_or(())? as u64;
        let need = bytes.div_ceil(memory::PAGE);
        // A big step first; near the window's end, exactly what this one needs.
        let got = ask(need.max(STEP_PAGES)).or_else(|| ask(need)).ok_or(())?;
        let end = got.end() as *mut u8;
        match talc.source.end.take() {
            Some(old) => talc.source.end = Some(unsafe { talc.extend(old, end) }),
            None => talc.source.end = unsafe { talc.claim(got.base as *mut u8, (got.end() - got.base) as usize) },
        }
        if talc.source.end.is_some() { Ok(()) } else { Err(()) }
    }
}

pub type Heap = talc::TalcLock<Lock, Window>;

#[cfg(feature = "heap")]
#[global_allocator]
static HEAP: Heap = Heap::new(Window::new());

/// The heap's pages so far, in bytes.
pub fn held() -> usize {
    memory::grow(0).map(|m| (m.pages * memory::PAGE) as usize).unwrap_or(0)
}
