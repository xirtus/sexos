#[path = "../sex-rt/src/heap.rs"]
mod heap;
use core::alloc::{GlobalAlloc, Layout};
use heap::{Arena, BoundedAllocator, HEAP_BYTES};

fn allocator() -> BoundedAllocator {
    // Test harness allocations use std's allocator. Only explicit alloc calls
    // below target the production allocator; it is not installed globally here.
    BoundedAllocator::new(Box::leak(Box::new(Arena::new())))
}

#[test]
fn shell_first_640_bytes_succeeds_without_syscalls() {
    let allocator = allocator();
    allocator.init();
    let layout = Layout::from_size_align(640, 8).unwrap();
    let pointer = unsafe { allocator.alloc(layout) };
    assert!(!pointer.is_null());
    unsafe { allocator.dealloc(pointer, layout); }
}

#[test]
fn repeated_init_preserves_live_allocations() {
    let allocator = allocator();
    let layout = Layout::from_size_align(640, 8).unwrap();
    let first = unsafe { allocator.alloc(layout) };
    assert!(!first.is_null());
    unsafe { core::ptr::write_bytes(first, 0xa7, 640); }
    for _ in 0..100 { allocator.init(); }
    let second = unsafe { allocator.alloc(layout) };
    assert!(!second.is_null());
    assert_ne!(first, second);
    unsafe {
        assert_eq!(core::slice::from_raw_parts(first, 640), &[0xa7; 640]);
        allocator.dealloc(first, layout);
        allocator.dealloc(second, layout);
    }
}

#[test]
fn repeated_name_allocations_recycle_without_growth() {
    let allocator = allocator();
    let layout = Layout::from_size_align(24, 1).unwrap();
    let first = unsafe { allocator.alloc(layout) };
    unsafe { allocator.dealloc(first, layout); }
    for _ in 0..100_000 {
        let pointer = unsafe { allocator.alloc(layout) };
        assert_eq!(pointer, first);
        unsafe { allocator.dealloc(pointer, layout); }
    }
}

#[test]
fn alignment_up_to_page_is_respected() {
    let allocator = allocator();
    for align in [1, 2, 4, 8, 16, 64, 512, 4096] {
        let layout = Layout::from_size_align(117, align).unwrap();
        let pointer = unsafe { allocator.alloc(layout) };
        assert!(!pointer.is_null());
        assert_eq!(pointer as usize % align, 0);
        unsafe { allocator.dealloc(pointer, layout); }
    }
}

#[test]
fn exhaustion_returns_null_and_free_coalesces() {
    let allocator = allocator();
    let layout = Layout::from_size_align(4096, 4096).unwrap();
    let mut allocations = Vec::new();
    loop {
        let pointer = unsafe { allocator.alloc(layout) };
        if pointer.is_null() { break; }
        allocations.push(pointer);
        assert!(allocations.len() <= 16);
    }
    assert_eq!(allocations.len(), 16);
    for pointer in allocations { unsafe { allocator.dealloc(pointer, layout); } }
    let whole = Layout::from_size_align(HEAP_BYTES, 4096).unwrap();
    let pointer = unsafe { allocator.alloc(whole) };
    assert!(!pointer.is_null());
    unsafe { allocator.dealloc(pointer, whole); }
}

#[test]
fn oversized_and_zero_requests_are_bounded() {
    let allocator = allocator();
    for size in [0, HEAP_BYTES + 1, usize::MAX / 2] {
        let layout = Layout::from_size_align(size, 1).unwrap();
        assert!(unsafe { allocator.alloc(layout) }.is_null());
    }
}

#[test]
fn concurrent_init_and_allocations_do_not_overlap() {
    let allocator = std::sync::Arc::new(allocator());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let mut workers = Vec::new();
    for worker in 0..8u8 {
        let allocator = allocator.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..500 {
                allocator.init();
                let layout = Layout::from_size_align(640, 64).unwrap();
                let pointer = unsafe { allocator.alloc(layout) };
                assert!(!pointer.is_null());
                unsafe { core::ptr::write_bytes(pointer, worker, 640); }
                std::thread::yield_now();
                unsafe {
                    assert_eq!(core::slice::from_raw_parts(pointer, 640), &[worker; 640]);
                    allocator.dealloc(pointer, layout);
                }
            }
        }));
    }
    for worker in workers { worker.join().unwrap(); }
}

#[test]
fn separate_handles_share_one_arena_init_lock() {
    let arena = Box::leak(Box::new(Arena::new()));
    let a = BoundedAllocator::new(arena);
    let b = BoundedAllocator::new(arena);
    let layout = Layout::from_size_align(640, 8).unwrap();
    let first = unsafe { a.alloc(layout) };
    b.init();
    let second = unsafe { b.alloc(layout) };
    assert!(!first.is_null() && !second.is_null());
    assert_ne!(first, second);
    unsafe { a.dealloc(first, layout); b.dealloc(second, layout); }
}

#[test]
fn coherent_usage_preserves_live_bytes_and_returns_to_empty_after_reuse() {
    let allocator=allocator();assert_eq!(allocator.usage(),heap::Usage{used:0,free:HEAP_BYTES,total:HEAP_BYTES});
    let layout=Layout::from_size_align(27000,8).unwrap();
    for _ in 0..64 {
        let p=unsafe{allocator.alloc(layout)};assert!(!p.is_null());unsafe{core::ptr::write_bytes(p,0x95,layout.size());}
        let usage=allocator.usage();assert!(usage.used>=layout.size());assert_eq!(usage.used+usage.free,usage.total);assert_eq!(usage.total,HEAP_BYTES);
        assert_eq!(unsafe{core::slice::from_raw_parts(p,layout.size())},vec![0x95;layout.size()]);
        unsafe{allocator.dealloc(p,layout);}
        assert_eq!(allocator.usage().used,0);assert_eq!(allocator.usage().free,HEAP_BYTES);
    }
}
