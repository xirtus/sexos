// SPDX-License-Identifier: GPL-3.0-only
//! The kernel heap starts small and grows. BOOT maps `HEAP_INITIAL`; the page tables
//! for the rest of the heap's address space (up to `HEAP_SIZE`) are made once the page
//! allocator is up, and an allocation that does not fit maps more pages on the spot.
//! A 512 MiB machine used to give half its memory to a heap that held 15 MiB.
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};
use linked_list_allocator::Heap;

/// Least the heap grows by, so growth is rare.
const STEP: usize = 4 * 1024 * 1024;
static TABLES: AtomicBool = AtomicBool::new(false);

pub struct KernelHeap(spin::Mutex<Heap>);
impl KernelHeap {
    pub const fn empty() -> Self { Self(spin::Mutex::new(Heap::empty())) }
    pub fn lock(&self) -> spin::MutexGuard<'_, Heap> { self.0.lock() }
}

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut heap = self.0.lock();
        if let Ok(p) = heap.allocate_first_fit(layout) { return p.as_ptr(); }
        if grow(&mut heap, layout) {
            if let Ok(p) = heap.allocate_first_fit(layout) { return p.as_ptr(); }
        }
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(p) = NonNull::new(ptr) { self.0.lock().deallocate(p, layout); }
    }
}

/// Maps pages past the heap's top, from the page allocator, and hands them to the heap.
fn grow(heap: &mut Heap, layout: Layout) -> bool {
    if !TABLES.load(Ordering::Acquire) { return false; }
    let top = heap.top() as usize;
    let want = (layout.size() + layout.align() + 4096).max(STEP).next_multiple_of(4096);
    let end = crate::HEAP_START + crate::HEAP_SIZE;
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
    let end = (crate::HEAP_START + crate::HEAP_SIZE) as u64;
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
    crate::serial_println!("[kernel.heap.tables] initial={} max={}", crate::HEAP_INITIAL, crate::HEAP_SIZE);
}
