// SPDX-License-Identifier: GPL-3.0-only
//! `malloc` and friends over the program's Rust heap.
//!
//! C `free` gets only a pointer, while Rust's allocator needs the layout back, so
//! every block carries a 16-byte header just before the pointer: the offset from
//! the real allocation start and the full allocation size. Alignment up to 4 KiB
//! is honoured (`posix_memalign`, FFmpeg asks for 64).
use crate::{c_int, c_void, set_errno, EINVAL, ENOMEM};
use alloc::alloc::{alloc, dealloc, Layout};
use core::ptr::null_mut;

const HEADER: usize = 16;
const MIN_ALIGN: usize = 16;
const MAX_ALIGN: usize = 4096;

/// Allocates `size` bytes aligned to `align` (a power of two, at least 16).
unsafe fn allocate(size: usize, align: usize) -> *mut u8 {
    let Some(total) = size.checked_add(align).and_then(|t| t.checked_add(HEADER)) else { return null_mut() };
    let Ok(layout) = Layout::from_size_align(total, align) else { return null_mut() };
    let base = unsafe { alloc(layout) };
    if base.is_null() {
        return null_mut();
    }
    // First aligned address with room for the header before it.
    let user = (base as usize + HEADER).next_multiple_of(align);
    let offset = user - base as usize;
    let header = (user - HEADER) as *mut usize;
    unsafe {
        header.write(offset | (align.trailing_zeros() as usize) << 48);
        header.add(1).write(total);
    }
    user as *mut u8
}

/// The user size a block can hold and its alignment.
unsafe fn block(ptr: *mut u8) -> (*mut u8, Layout, usize) {
    let header = (ptr as usize - HEADER) as *const usize;
    let word = unsafe { header.read() };
    let total = unsafe { header.add(1).read() };
    let offset = word & ((1 << 48) - 1);
    let align = 1usize << (word >> 48);
    let base = unsafe { ptr.sub(offset) };
    (base, unsafe { Layout::from_size_align_unchecked(total, align) }, total - offset)
}

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    let p = unsafe { allocate(size.max(1), MIN_ALIGN) };
    if p.is_null() {
        set_errno(ENOMEM);
    }
    p.cast()
}

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    let Some(bytes) = count.checked_mul(size) else {
        set_errno(ENOMEM);
        return null_mut();
    };
    let p = unsafe { malloc(bytes) };
    if !p.is_null() {
        unsafe { core::ptr::write_bytes(p.cast::<u8>(), 0, bytes) };
    }
    p
}

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    let (base, layout, _) = unsafe { block(ptr.cast()) };
    unsafe { dealloc(base, layout) };
}

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if ptr.is_null() {
        return unsafe { malloc(size) };
    }
    if size == 0 {
        unsafe { free(ptr) };
        return null_mut();
    }
    let (_, layout, room) = unsafe { block(ptr.cast()) };
    if size <= room {
        return ptr;
    }
    let fresh = unsafe { allocate(size, layout.align().max(MIN_ALIGN)) };
    if fresh.is_null() {
        set_errno(ENOMEM);
        return null_mut();
    }
    unsafe {
        core::ptr::copy_nonoverlapping(ptr.cast::<u8>(), fresh, room.min(size));
        free(ptr);
    }
    fresh.cast()
}

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn posix_memalign(out: *mut *mut c_void, align: usize, size: usize) -> c_int {
    if !align.is_power_of_two() || align % core::mem::size_of::<usize>() != 0 || align > MAX_ALIGN {
        return EINVAL;
    }
    let p = unsafe { allocate(size.max(1), align.max(MIN_ALIGN)) };
    if p.is_null() {
        return ENOMEM;
    }
    unsafe { *out = p.cast() };
    0
}

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn aligned_alloc(align: usize, size: usize) -> *mut c_void {
    let mut p = null_mut();
    match unsafe { posix_memalign(&mut p, align.max(core::mem::size_of::<usize>()), size) } {
        0 => p,
        e => {
            set_errno(e);
            null_mut()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_reuse_and_realloc() {
        unsafe {
            for align in [16usize, 32, 64, 4096] {
                let mut p = null_mut();
                assert_eq!(posix_memalign(&mut p, align, 100), 0);
                assert_eq!(p as usize % align, 0);
                core::ptr::write_bytes(p.cast::<u8>(), 0xab, 100);
                let q = realloc(p, 5000);
                assert!(!q.is_null());
                assert_eq!(q as usize % align, 0, "realloc keeps the alignment");
                assert!((0..100).all(|i| *q.cast::<u8>().add(i) == 0xab));
                free(q);
            }
            let z = calloc(10, 10).cast::<u8>();
            assert!((0..100).all(|i| *z.add(i) == 0));
            free(z.cast());
            assert!(calloc(usize::MAX, 2).is_null());
            assert_eq!(posix_memalign(&mut null_mut(), 24, 8), EINVAL);
            free(null_mut());
        }
    }
}
