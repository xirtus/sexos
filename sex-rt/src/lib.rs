#![no_std]

use core::sync::atomic::{AtomicBool, Ordering};
pub use sex_pdx::{SYSCALL_PDX_CALL, SYSCALL_PDX_LISTEN, SYSCALL_YIELD, MessageType, PdxEvent, pdx_call, pdx_listen, pdx_reply, sys_yield, sched_yield};

mod heap;

// This .bss storage is part of each domain's BOOT_DAG-loaded ELF. The runtime
// subdivides its own bytes; it has no frame allocation or mapping authority.
static HEAP_ARENA: heap::Arena = heap::Arena::new();
#[global_allocator]
static ALLOCATOR: heap::BoundedAllocator = heap::BoundedAllocator::growing(&HEAP_ARENA, grow_pages);
/// Pages a service with a growable arena may add once the ELF arena is full.
fn grow_pages(pages: u64) -> Result<(u64, u64), u64> {
    sex_pdx::program_memory::grow(pages).map(|m| (m.base, m.pages))
}
static HEAP_INIT_REPORTED: AtomicBool = AtomicBool::new(false);

pub fn sys_exit(status: usize) -> ! {
    sex_pdx::task_finish(status as u64)
}

pub fn heap_init() {
    ALLOCATOR.init();
    if !HEAP_INIT_REPORTED.swap(true, Ordering::AcqRel) {
        sex_pdx::serial_record!("[runtime.heap.ready] backing=elf_owned bytes={} runtime_mapping=0", heap::HEAP_BYTES);
    }
}

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    sex_pdx::mem::copy_forward(dest, src, n);
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8 {
    sex_pdx::mem::fill(s, c as u8, n);
    s
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let a = *s1.add(i);
        let b = *s2.add(i);
        if a != b { return a as i32 - b as i32; }
        i += 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    sex_pdx::mem::move_bytes(dest, src, n);
    dest
}
