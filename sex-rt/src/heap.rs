// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Byte allocation inside an ELF-owned arena, then, once that is full, inside pages the
//! kernel grants a service with a growable arena (SYS_PROGRAM_MEMORY). A domain without
//! one keeps the fixed arena alone; asking costs one denied call, once.
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::{self, NonNull};
use linked_list_allocator::Heap;
use spin::Mutex;

pub const HEAP_BYTES: usize = 128 * 1024;

/// One coherent allocator snapshot. Byte usage includes allocator alignment and
/// bookkeeping; these counts confer no frame, mapping or endpoint authority.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Usage { pub used:usize, pub free:usize, pub total:usize }

#[repr(C, align(4096))]
struct AlignedBytes([u8; HEAP_BYTES]);

pub struct Arena {
    heap: Mutex<Heap>,
    growth: Mutex<Growth>,
    storage: UnsafeCell<AlignedBytes>,
}

/// Pages from the kernel, contiguous from the arena's base.
struct Growth { heap: Heap, state: u8 }
const UNASKED: u8 = 0;
const GROWS: u8 = 1;
const FIXED: u8 = 2;
/// Pages asked for at a time, so small allocations don't each cost a call.
const STEP_PAGES: usize = 64;
/// The kernel's limit for one request (sex_pdx::program_memory::MAX_PAGES_PER_CALL).
const MAX_PAGES_PER_CALL: usize = 256;

// Allocation metadata and one-time initialization share the same lock. Once
// allocated, disjoint payload bytes belong to the respective allocation caller.
unsafe impl Sync for Arena {}

impl Arena {
    pub const fn new() -> Self {
        Self { heap: Mutex::new(Heap::empty()), growth: Mutex::new(Growth { heap: Heap::empty(), state: UNASKED }),
            storage: UnsafeCell::new(AlignedBytes([0; HEAP_BYTES])) }
    }
}

/// Holding a static reference pins the storage for the allocator's lifetime.
/// Multiple handles to one arena share its initialization and free-list lock.
pub struct BoundedAllocator { arena: &'static Arena, grow: Option<Grow> }

/// Asks the kernel for `pages` more pages; replies (base, pages held in all).
pub type Grow = fn(u64) -> Result<(u64, u64), u64>;

impl BoundedAllocator {
    /// The fixed arena only.
    pub const fn new(arena: &'static Arena) -> Self { Self { arena, grow: None } }
    /// The fixed arena, then pages from `grow` once it is full.
    pub const fn growing(arena: &'static Arena, grow: Grow) -> Self { Self { arena, grow: Some(grow) } }

    fn ensure_initialized(&self, heap: &mut Heap) {
        if heap.bottom().is_null() {
            let start = unsafe { ptr::addr_of_mut!((*self.arena.storage.get()).0).cast::<u8>() };
            // Arena is static, exclusively managed by this shared lock, and
            // this branch can run once only. Existing allocations never reset.
            unsafe { heap.init(start, HEAP_BYTES); }
        }
    }

    pub fn init(&self) {
        self.ensure_initialized(&mut self.arena.heap.lock());
    }

    pub fn usage(&self)->Usage {
        let mut heap=self.arena.heap.lock();self.ensure_initialized(&mut heap);
        let growth=self.arena.growth.lock();
        let (gu,gf,gt)=if growth.state==GROWS{(growth.heap.used(),growth.heap.free(),growth.heap.size())}else{(0,0,0)};
        Usage{used:heap.used()+gu,free:heap.free()+gf,total:heap.size()+gt}
    }

    fn in_fixed(&self, pointer: *mut u8) -> bool {
        let start = unsafe { ptr::addr_of_mut!((*self.arena.storage.get()).0).cast::<u8>() } as usize;
        (start..start + HEAP_BYTES).contains(&(pointer as usize))
    }

    /// From granted pages, asking the kernel for more when they run out.
    unsafe fn alloc_grown(&self, layout: Layout) -> *mut u8 {
        let Some(grow) = self.grow else { return ptr::null_mut(); };
        let mut growth = self.arena.growth.lock();
        if growth.state == FIXED { return ptr::null_mut(); }
        if growth.state == UNASKED {
            // A restarted service adopts the pages it already holds.
            match grow(0) {
                Ok((base, pages)) => {
                    growth.state = GROWS;
                    if pages != 0 { growth.heap.init(base as *mut u8, (pages * 4096) as usize); }
                }
                Err(_) => { growth.state = FIXED; return ptr::null_mut(); }
            }
        }
        loop {
            if !growth.heap.bottom().is_null() {
                if let Ok(p) = growth.heap.allocate_first_fit(layout) { return p.as_ptr(); }
            }
            let need = (layout.size() + layout.align() + 64).div_ceil(4096).clamp(STEP_PAGES, MAX_PAGES_PER_CALL);
            let Ok((base, pages)) = grow(need as u64) else { return ptr::null_mut(); };
            let end = base as usize + pages as usize * 4096;
            if growth.heap.bottom().is_null() {
                growth.heap.init(base as *mut u8, end - base as usize);
            } else {
                let top = growth.heap.top() as usize;
                if end <= top { return ptr::null_mut(); }
                growth.heap.extend(end - top);
            }
        }
    }
}

unsafe impl GlobalAlloc for BoundedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 { return ptr::null_mut(); }
        if layout.size() <= HEAP_BYTES {
            let mut heap = self.arena.heap.lock();
            self.ensure_initialized(&mut heap);
            if let Ok(p) = heap.allocate_first_fit(layout) { return p.as_ptr(); }
        }
        self.alloc_grown(layout)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if layout.size() == 0 { return; }
        if let Some(pointer) = NonNull::new(pointer) {
            // GlobalAlloc's caller owns this exact pointer/layout pair.
            if self.in_fixed(pointer.as_ptr()) { self.arena.heap.lock().deallocate(pointer, layout); }
            else { self.arena.growth.lock().heap.deallocate(pointer, layout); }
        }
    }
}
