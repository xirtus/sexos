// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Fast freestanding memory copies for domains that supply their own `memcpy`,
//! `memset` and `memmove` (the kernel, sex-rt services, drivers). Eight bytes per step
//! with `rep movsq`/`rep stosq`, then the tail a byte at a time. On CPUs with fast
//! string operations (every recent x86_64) this is the fastest simple copy; byte
//! loops were 8x or more slower. Inline assembly, so LLVM cannot turn it back into
//! a call to the very symbol it implements.

/// Copies `n` bytes forward. The ranges may overlap only if `dest <= src`.
#[inline(always)]
pub unsafe fn copy_forward(dest: *mut u8, src: *const u8, n: usize) {
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!(
        "rep movsq",
        "mov rcx, {tail}",
        "rep movsb",
        tail = in(reg) n & 7,
        inout("rcx") n >> 3 => _,
        inout("rdi") dest => _,
        inout("rsi") src => _,
        options(nostack, preserves_flags),
    );
    #[cfg(not(target_arch = "x86_64"))]
    { let mut i = 0; while i < n { dest.add(i).write_volatile(src.add(i).read_volatile()); i += 1; } }
}

/// Copies `n` bytes backward, for overlapping moves with `dest > src`. Eight bytes
/// per step from the end, then the head a byte at a time. Plain moves, never `std`:
/// an interrupt taken with the direction flag set would run handler code (and its
/// string copies) backward.
#[inline(always)]
pub unsafe fn copy_backward(dest: *mut u8, src: *const u8, n: usize) {
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!(
        "2:",
        "cmp rcx, 8",
        "jb 3f",
        "sub rcx, 8",
        "mov {t}, qword ptr [rsi + rcx]",
        "mov qword ptr [rdi + rcx], {t}",
        "jmp 2b",
        "3:",
        "test rcx, rcx",
        "jz 4f",
        "dec rcx",
        "mov {t:l}, byte ptr [rsi + rcx]",
        "mov byte ptr [rdi + rcx], {t:l}",
        "jmp 3b",
        "4:",
        t = out(reg) _,
        inout("rcx") n => _,
        in("rdi") dest,
        in("rsi") src,
        options(nostack),
    );
    #[cfg(not(target_arch = "x86_64"))]
    { let mut i = n; while i > 0 { i -= 1; dest.add(i).write_volatile(src.add(i).read_volatile()); } }
}

/// Sets `n` bytes to `byte`.
#[inline(always)]
pub unsafe fn fill(dest: *mut u8, byte: u8, n: usize) {
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!(
        "rep stosq",
        "mov rcx, {tail}",
        "rep stosb",
        tail = in(reg) n & 7,
        inout("rcx") n >> 3 => _,
        inout("rdi") dest => _,
        in("rax") (byte as u64) * 0x0101_0101_0101_0101,
        options(nostack, preserves_flags),
    );
    #[cfg(not(target_arch = "x86_64"))]
    { let mut i = 0; while i < n { dest.add(i).write_volatile(byte); i += 1; } }
}

/// `memmove`: forward unless `dest` starts inside `src..src+n`.
#[inline(always)]
pub unsafe fn move_bytes(dest: *mut u8, src: *const u8, n: usize) {
    if (dest as usize).wrapping_sub(src as usize) >= n { copy_forward(dest, src, n) } else { copy_backward(dest, src, n) }
}
