// SPDX-License-Identifier: GPL-3.0-only
//! `<setjmp.h>` for x86-64: non-local jumps inside one thread of the program, which
//! interpreters such as Lua use for their errors. Nothing here leaves the program: a
//! jump restores the callee-saved registers, the stack pointer and the return address
//! that `setjmp` saved in the caller's `jmp_buf`.
//!
//! The layout is musl's `__jmp_buf` (eight words: rbx, rbp, r12, r13, r14, r15, rsp,
//! rip), the type the program compiles against. Signal masks do not exist here, so
//! `sigsetjmp`/`siglongjmp` are the same jumps.

#[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "hermit")))]
mod x86_64 {
    use crate::{c_int, c_void};

    #[unsafe(naked)]
    #[no_mangle]
    pub unsafe extern "C" fn setjmp(_env: *mut c_void) -> c_int {
        core::arch::naked_asm!(
            "mov [rdi], rbx",
            "mov [rdi + 8], rbp",
            "mov [rdi + 16], r12",
            "mov [rdi + 24], r13",
            "mov [rdi + 32], r14",
            "mov [rdi + 40], r15",
            "lea rdx, [rsp + 8]",
            "mov [rdi + 48], rdx",
            "mov rdx, [rsp]",
            "mov [rdi + 56], rdx",
            "xor eax, eax",
            "ret",
        );
    }

    /// Returns `value` from the `setjmp` that filled `env`, or 1 if `value` is 0.
    #[unsafe(naked)]
    #[no_mangle]
    pub unsafe extern "C" fn longjmp(_env: *mut c_void, _value: c_int) -> ! {
        core::arch::naked_asm!(
            "xor eax, eax",
            "cmp esi, 1",
            "adc eax, esi",
            "mov rbx, [rdi]",
            "mov rbp, [rdi + 8]",
            "mov r12, [rdi + 16]",
            "mov r13, [rdi + 24]",
            "mov r14, [rdi + 32]",
            "mov r15, [rdi + 40]",
            "mov rsp, [rdi + 48]",
            "jmp qword ptr [rdi + 56]",
        );
    }

    #[unsafe(naked)]
    #[no_mangle]
    pub unsafe extern "C" fn _setjmp(_env: *mut c_void) -> c_int {
        core::arch::naked_asm!("jmp {f}", f = sym setjmp);
    }
    #[unsafe(naked)]
    #[no_mangle]
    pub unsafe extern "C" fn sigsetjmp(_env: *mut c_void, _save: c_int) -> c_int {
        core::arch::naked_asm!("jmp {f}", f = sym setjmp);
    }
    #[unsafe(naked)]
    #[no_mangle]
    pub unsafe extern "C" fn _longjmp(_env: *mut c_void, _value: c_int) -> ! {
        core::arch::naked_asm!("jmp {f}", f = sym longjmp);
    }
    #[unsafe(naked)]
    #[no_mangle]
    pub unsafe extern "C" fn siglongjmp(_env: *mut c_void, _value: c_int) -> ! {
        core::arch::naked_asm!("jmp {f}", f = sym longjmp);
    }
}
