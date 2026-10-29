// SPDX-License-Identifier: GPL-3.0-only
//! Byte allocation inside an ELF-owned arena; no frame or mapping authority.
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::{self, NonNull};
use linked_list_allocator::Heap;
use spin::Mutex;

pub const HEAP_BYTES: usize = 64 * 1024;

#[repr(C, align(4096))]
struct AlignedBytes([u8; HEAP_BYTES]);

pub struct Arena {
    heap: Mutex<Heap>,
    storage: UnsafeCell<AlignedBytes>,
}

// Allocation metadata and one-time initialization share the same lock. Once
// allocated, disjoint payload bytes belong to the respective allocation caller.
unsafe impl Sync for Arena {}

impl Arena {
    pub const fn new() -> Self {
        Self { heap: Mutex::new(Heap::empty()), storage: UnsafeCell::new(AlignedBytes([0; HEAP_BYTES])) }
    }
}

/// Holding a static reference pins the storage for the allocator's lifetime.
/// Multiple handles to one arena share its initialization and free-list lock.
pub struct BoundedAllocator { arena: &'static Arena }

impl BoundedAllocator {
    pub const fn new(arena: &'static Arena) -> Self { Self { arena } }

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
}

unsafe impl GlobalAlloc for BoundedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 || layout.size() > HEAP_BYTES { return ptr::null_mut(); }
        let mut heap = self.arena.heap.lock();
        self.ensure_initialized(&mut heap);
        heap.allocate_first_fit(layout).map_or(ptr::null_mut(), NonNull::as_ptr)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if layout.size() == 0 { return; }
        if let Some(pointer) = NonNull::new(pointer) {
            // GlobalAlloc's caller owns this exact pointer/layout pair.
            self.arena.heap.lock().deallocate(pointer, layout);
        }
    }
}
