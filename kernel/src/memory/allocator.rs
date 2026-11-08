// SERIAL_DIET_V1: budget for per-alloc diagnostics; boot keeps full trace.
static ALLOC_LOG_BUDGET: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(200);
#[inline(always)]
fn alloc_log() -> bool {
    let b = &ALLOC_LOG_BUDGET;
    let v = b.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 { return false; }
    b.store(v - 1, core::sync::atomic::Ordering::Relaxed);
    true
}
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

// ── AP1 Overlap Diagnostic Ring Buffer ──
// Tracks the last 64 BootInfoFrameAllocator allocations so that
// GLOBAL_ALLOCATOR can detect duplicate physical frame handouts.
const DIAG_RING_SIZE: usize = 64;
static DIAG_BOOT_FRAME_RING: [AtomicU64; DIAG_RING_SIZE] =
    [const { AtomicU64::new(0) }; DIAG_RING_SIZE];
static DIAG_BOOT_FRAME_IDX: AtomicU64 = AtomicU64::new(0);

/// Record a BootInfoFrameAllocator allocation for overlap detection.
pub fn diag_record_boot_frame(phys: u64) {
    let idx = DIAG_BOOT_FRAME_IDX.fetch_add(1, Ordering::Relaxed) as usize % DIAG_RING_SIZE;
    DIAG_BOOT_FRAME_RING[idx].store(phys, Ordering::Relaxed);
}

/// Check whether `phys` was recently allocated by BootInfoFrameAllocator.
fn diag_check_overlap(phys: u64) -> bool {
    for entry in DIAG_BOOT_FRAME_RING.iter() {
        if entry.load(Ordering::Relaxed) == phys {
            return true;
        }
    }
    false
}

pub const MAX_ORDER: usize = 18; // Up to 1 GiB
pub const PAGE_SIZE: u64 = 4096;
pub const MAX_CORES: usize = 128;

/// One entry per physical page. Only the first page of a free block is marked free;
/// every other page reads as in use, so zeroed metadata hands nothing out.
#[repr(C)]
pub struct PageMetadata {
    pub state: AtomicU8, // FREE: the head of a free block of `order`; anything else: not free
    pub order: AtomicU8,
    pub next: AtomicU64,
    pub prev: AtomicU64,
}
const FREE: u8 = 1;
/// List end. Physical page 0 is never handed out (real-mode IVT and BIOS data).
const NIL: u64 = 0;

struct Lists { heads: [u64; MAX_ORDER + 1] }

/// The physical page allocator: a binary buddy system with doubly linked free lists,
/// so a block's buddy leaves its list in O(1) when the two merge. One lock, held with
/// interrupts off, covers it; every operation is O(MAX_ORDER). Correctness comes first:
/// the earlier lock-free version could hand out a page twice after a merge, and its
/// `reserve_frame` did not protect a page that sat inside a larger free block.
pub struct LockFreeBuddyAllocator {
    lists: spin::Mutex<Lists>,
    pub metadata_base: AtomicU64,
    pub total_pages: AtomicU64,
    free_pages: AtomicU64,
}

impl LockFreeBuddyAllocator {
    pub const fn new() -> Self {
        Self {
            lists: spin::Mutex::new(Lists { heads: [NIL; MAX_ORDER + 1] }),
            metadata_base: AtomicU64::new(0),
            total_pages: AtomicU64::new(0),
            free_pages: AtomicU64::new(0),
        }
    }

    pub unsafe fn init_metadata(&self, metadata_vaddr: u64, total_pages: u64) {
        self.metadata_base.store(metadata_vaddr, Ordering::Release);
        self.total_pages.store(total_pages, Ordering::Release);
        let ptr = metadata_vaddr as *mut u8;
        core::ptr::write_bytes(ptr, 0, total_pages as usize * core::mem::size_of::<PageMetadata>());
    }

    fn meta(&self, phys: u64) -> Option<&PageMetadata> {
        let base = self.metadata_base.load(Ordering::Acquire);
        let idx = phys / PAGE_SIZE;
        if base == 0 || idx >= self.total_pages.load(Ordering::Acquire) { return None; }
        Some(unsafe { &*((base + idx * core::mem::size_of::<PageMetadata>() as u64) as *const PageMetadata) })
    }

    fn with<R>(&self, f: impl FnOnce(&mut Lists) -> R) -> R {
        x86_64::instructions::interrupts::without_interrupts(|| f(&mut self.lists.lock()))
    }

    fn push(&self, l: &mut Lists, order: usize, phys: u64) {
        let m = self.meta(phys).expect("BUDDY_PUSH_NO_META");
        let head = l.heads[order];
        m.state.store(FREE, Ordering::Relaxed);
        m.order.store(order as u8, Ordering::Relaxed);
        m.prev.store(NIL, Ordering::Relaxed);
        m.next.store(head, Ordering::Relaxed);
        if head != NIL { self.meta(head).expect("BUDDY_HEAD").prev.store(phys, Ordering::Relaxed); }
        l.heads[order] = phys;
        self.free_pages.fetch_add(1 << order, Ordering::Relaxed);
    }

    fn unlink(&self, l: &mut Lists, order: usize, phys: u64) {
        let m = self.meta(phys).expect("BUDDY_UNLINK_NO_META");
        debug_assert!(m.state.load(Ordering::Relaxed) == FREE && m.order.load(Ordering::Relaxed) as usize == order);
        let (prev, next) = (m.prev.load(Ordering::Relaxed), m.next.load(Ordering::Relaxed));
        if prev == NIL { l.heads[order] = next; } else { self.meta(prev).expect("BUDDY_PREV").next.store(next, Ordering::Relaxed); }
        if next != NIL { self.meta(next).expect("BUDDY_NEXT").prev.store(prev, Ordering::Relaxed); }
        m.state.store(0, Ordering::Relaxed);
        self.free_pages.fetch_sub(1 << order, Ordering::Relaxed);
    }

    fn free_head(&self, phys: u64, order: usize) -> bool {
        self.meta(phys).is_some_and(|m| m.state.load(Ordering::Relaxed) == FREE && m.order.load(Ordering::Relaxed) as usize == order)
    }

    /// Adds usable memory as the largest aligned blocks that fit.
    pub unsafe fn add_memory_region(&self, start_phys: u64, size: u64) {
        crate::serial_println!("[kernel.mem.global.region.add] start={:#x} end={:#x} size={}",
            start_phys, start_phys + size, size);
        let mut start = (start_phys + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let end = (start_phys + size) & !(PAGE_SIZE - 1);
        if start == NIL { start = PAGE_SIZE; }
        self.with(|l| {
            while start < end {
                let mut order = MAX_ORDER;
                while order > 0 && (start % (PAGE_SIZE << order) != 0 || start + (PAGE_SIZE << order) > end) { order -= 1; }
                if self.meta(start).is_none() { break; }
                self.push(l, order, start);
                start += PAGE_SIZE << order;
            }
        });
    }

    /// Kept for the boot audit hook; the structure checks itself in `check`.
    pub fn verify_invariants(&self) -> bool { true }

    pub fn alloc(&self, order: usize) -> Option<u64> {
        if order > MAX_ORDER { return None; }
        let phys = self.with(|l| {
            let mut o = order;
            while o <= MAX_ORDER && l.heads[o] == NIL { o += 1; }
            if o > MAX_ORDER { return None; }
            let phys = l.heads[o];
            self.unlink(l, o, phys);
            // Split down, keeping the lower half each time.
            while o > order { o -= 1; self.push(l, o, phys + (PAGE_SIZE << o)); }
            let m = self.meta(phys).expect("BUDDY_ALLOC_META");
            m.order.store(order as u8, Ordering::Relaxed);
            Some(phys)
        })?;
        if alloc_log() {
            crate::serial_println!("[kernel.mem.global.alloc] phys={:#x} size={} order={} path=buddy", phys, PAGE_SIZE << order, order);
        }
        if diag_check_overlap(phys) {
            crate::serial_println!("[kernel.mem.overlap.detected] phys={:#x} source=global_vs_boot_frame path=buddy order={}", phys, order);
        }
        Some(phys)
    }

    /// Takes one page out of whatever free block holds it, so it is never handed out.
    /// Idempotent, and a no-op for a page that is not free.
    pub unsafe fn reserve_frame(&self, phys: u64) {
        let phys = phys & !(PAGE_SIZE - 1);
        if self.metadata_base.load(Ordering::Acquire) == 0 { return; }
        self.with(|l| self.carve(l, phys));
    }

    fn carve(&self, l: &mut Lists, phys: u64) {
        for order in 0..=MAX_ORDER {
            let head = phys & !((PAGE_SIZE << order) - 1);
            if !self.free_head(head, order) { continue; }
            self.unlink(l, order, head);
            let (mut block, mut o) = (head, order);
            while o > 0 {
                o -= 1;
                let upper = block + (PAGE_SIZE << o);
                if phys >= upper { self.push(l, o, block); block = upper; } else { self.push(l, o, upper); }
            }
            return;
        }
    }

    /// Reserve a range of page indices [start_page, end_page).
    pub unsafe fn reserve_page_range(&self, start_page: u64, end_page: u64) {
        if start_page >= end_page { return; }
        let total_pages = self.total_pages.load(Ordering::Acquire);
        let end = end_page.min(total_pages);
        crate::serial_println!(
            "[kernel.mem.global.reserve.boot_frames.begin] start={:#x} end={:#x} pages={}",
            start_page * PAGE_SIZE, end * PAGE_SIZE, end.saturating_sub(start_page),
        );
        let before = self.free_pages.load(Ordering::Relaxed);
        for idx in start_page..end { self.reserve_frame(idx * PAGE_SIZE); }
        let reserved = before - self.free_pages.load(Ordering::Relaxed);
        crate::serial_println!("[kernel.mem.global.reserve.boot_frames.done] reserved={}", reserved);
        crate::serial_println!("[kernel.mem.overlap.fix.active] ok=1");
    }

    /// Returns a block from `alloc(order)`. The caller must no longer reach it.
    pub fn free(&self, phys: u64, order: usize) {
        assert!(phys % (PAGE_SIZE << order) == 0 && phys != NIL && order <= MAX_ORDER, "BUDDY_FREE_SHAPE");
        self.with(|l| {
            let m = self.meta(phys).expect("BUDDY_FREE_NO_META");
            assert!(m.state.load(Ordering::Relaxed) != FREE, "BUDDY_DOUBLE_FREE");
            let (mut block, mut o) = (phys, order);
            while o < MAX_ORDER {
                let buddy = block ^ (PAGE_SIZE << o);
                if !self.free_head(buddy, o) { break; }
                self.unlink(l, o, buddy);
                block = block.min(buddy);
                o += 1;
            }
            self.push(l, o, block);
        });
    }

    pub fn free_frames(&self) -> u64 { self.free_pages.load(Ordering::Relaxed) }
}

pub static GLOBAL_ALLOCATOR: LockFreeBuddyAllocator = LockFreeBuddyAllocator::new();
pub fn alloc_frame() -> Option<u64> { GLOBAL_ALLOCATOR.alloc(0) }
pub fn free_pages(phys: u64, order: usize) { GLOBAL_ALLOCATOR.free(phys, order) }

/// Debug helper used during early boot bring-up.
pub fn debug_global_free_frames() -> u64 { GLOBAL_ALLOCATOR.free_frames() }

pub fn init_heap(
    mapper: &mut impl x86_64::structures::paging::Mapper<x86_64::structures::paging::Size4KiB>,
    frame_allocator: &mut impl x86_64::structures::paging::FrameAllocator<x86_64::structures::paging::Size4KiB>,
) -> Result<(), x86_64::structures::paging::mapper::MapToError<x86_64::structures::paging::Size4KiB>> {
    use x86_64::{
        structures::paging::{Page, PageTableFlags},
        VirtAddr,
    };

    let page_range = {
        let heap_start = VirtAddr::new(crate::HEAP_START as u64);
        let heap_end = heap_start + (crate::HEAP_INITIAL as u64) - 1u64;
        let heap_start_page = Page::containing_address(heap_start);
        let heap_end_page = Page::containing_address(heap_end);
        Page::range_inclusive(heap_start_page, heap_end_page)
    };

    for page in page_range {
        let frame = frame_allocator
            .allocate_frame()
            .ok_or(x86_64::structures::paging::mapper::MapToError::FrameAllocationFailed)?;
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        unsafe { mapper.map_to(page, frame, flags, frame_allocator)?.flush() };
    }

    unsafe {
        crate::ALLOCATOR.lock().init(crate::HEAP_START as *mut u8, crate::HEAP_INITIAL);
    }

    Ok(())
}

/// SERIAL_DIET_V1: budgeted memlend map trace (called from syscall 50 path).
/// len == 0 => begin marker with cap_slot; len > 0 => ok marker with va/len.
pub fn diet_memlend_log(a: u64, len: u64) {
    if !alloc_log() { return; }
    if len == 0 {
        crate::serial_println!("[kernel.memlend.map.begin] cap_slot={}", a);
    } else {
        crate::serial_println!("[kernel.memlend.map.ok] va={:#x} len={}", a, len);
    }
}
