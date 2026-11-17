// SPDX-License-Identifier: GPL-3.0-only
//! The kernel heap starts small and grows. BOOT maps `HEAP_INITIAL`; the page tables
//! for the rest of the heap's address space (up to `HEAP_SIZE`) are made once the page
//! allocator is up, and an allocation that does not fit maps more pages on the spot.
//! A 512 MiB machine used to give half its memory to a heap that held 15 MiB.
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use linked_list_allocator::Heap;

/// Least the heap grows by, so growth is rare.
const STEP: usize = 4 * 1024 * 1024;
static TABLES: AtomicBool = AtomicBool::new(false);
/// How far the heap may grow, set once its page tables are made.
static LIMIT: AtomicUsize = AtomicUsize::new(0);
/// Blocks handed out and not yet freed, and the most bytes ever in use: a leak shows
/// as a count that climbs across open/close cycles (SYS_STATS `OP_KERNEL`).
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

pub struct KernelHeap(spin::Mutex<Heap>);
impl KernelHeap {
    pub const fn empty() -> Self { Self(spin::Mutex::new(Heap::empty())) }
    pub fn lock(&self) -> spin::MutexGuard<'_, Heap> { self.0.lock() }
}

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut heap = self.0.lock();
        let mut got = heap.allocate_first_fit(layout);
        if got.is_err() && grow(&mut heap, layout) { got = heap.allocate_first_fit(layout); }
        let Ok(p) = got else { return core::ptr::null_mut() };
        LIVE.fetch_add(1, Ordering::Relaxed);
        PEAK.fetch_max(heap.used(), Ordering::Relaxed);
        p.as_ptr()
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(p) = NonNull::new(ptr) { self.0.lock().deallocate(p, layout); LIVE.fetch_sub(1, Ordering::Relaxed); }
    }
}

/// SYS_STATS `OP_KERNEL`. Part 0: heap bytes in use, bytes mapped, live blocks, most
/// bytes ever in use. Part 1: the machine's free frames.
pub fn syscall(regs: &mut crate::interrupts::SyscallRegs, part: u64) -> u64 {
    if part == 1 { regs.rsi = crate::memory::allocator::debug_global_free_frames(); return sex_pdx::system_stats::OK; }
    if part != 0 { return sex_pdx::system_stats::INVALID; }
    let (used, size) = { let h = crate::ALLOCATOR.lock(); (h.used(), h.size()) };
    regs.rsi = used as u64; regs.rdx = size as u64;
    regs.r10 = LIVE.load(Ordering::Relaxed) as u64; regs.r8 = PEAK.load(Ordering::Relaxed) as u64;
    sex_pdx::system_stats::OK
}

/// Maps pages past the heap's top, from the page allocator, and hands them to the heap.
fn grow(heap: &mut Heap, layout: Layout) -> bool {
    if !TABLES.load(Ordering::Acquire) { return false; }
    let top = heap.top() as usize;
    let want = (layout.size() + layout.align() + 4096).max(STEP).next_multiple_of(4096);
    let end = crate::HEAP_START + LIMIT.load(Ordering::Acquire);
    let want = want.min(end.saturating_sub(top)) & !4095;
    if want == 0 { return false; }
    let mut mapped = 0;
    while mapped < want {
        let va = (top + mapped) as u64;
        let (Some(leaf), Some(frame)) = (crate::boot_memory::existing_leaf_address(va), crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(0)) else { break };
        // Present and writable, kernel only, key 0: as BOOT maps the first part.
        unsafe { core::ptr::write_volatile(leaf as *mut u64, frame | 0b11); }
        mapped += 4096;
    }
    if mapped == 0 { return false; }
    unsafe { heap.extend(mapped); }
    // No logging here: the heap lock is held, and logging may allocate. The new pages may
    // join a free tail, so the caller retries whenever any were added.
    true
}

/// Page tables for the heap's whole address space, every leaf past BOOT's part clear.
/// Runs once the page allocator owns free memory.
pub fn prepare_growth() {
    use x86_64::structures::paging::PageTableFlags as F;
    const SPAN: u64 = 2 * 1024 * 1024;
    let first = ((crate::HEAP_START + crate::HEAP_INITIAL) as u64).next_multiple_of(SPAN);
    // A quarter of the machine's memory, at least 1 GiB: page tables cost 1/512 of it.
    let free = crate::memory::allocator::debug_global_free_frames() as usize * 4096;
    let limit = (free / 4).next_power_of_two().clamp(1 << 30, crate::HEAP_SIZE);
    LIMIT.store(limit, Ordering::Release);
    let end = (crate::HEAP_START + limit) as u64;
    let scratch = crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(0).expect("KHEAP_TABLE_SCRATCH");
    let mut span = first;
    while span < end {
        if crate::boot_memory::existing_leaf_address(span).is_none() {
            crate::memory::manager::GLOBAL_VAS.lock().as_mut().expect("KHEAP_NO_VAS")
                .map_physical_range(x86_64::VirtAddr::new(span), scratch, 4096, F::PRESENT | F::WRITABLE, 0).expect("KHEAP_TABLE_MAP");
            let leaf = crate::boot_memory::existing_leaf_address(span).expect("KHEAP_TABLE_LEAF");
            unsafe { core::ptr::write_volatile(leaf as *mut u64, 0); }
            x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(span));
        }
        span += SPAN;
    }
    crate::memory::allocator::GLOBAL_ALLOCATOR.free(scratch, 0);
    TABLES.store(true, Ordering::Release);
    crate::serial_println!("[kernel.heap.tables] initial={} max={}", crate::HEAP_INITIAL, limit);
}
