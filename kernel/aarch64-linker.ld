/* ==========================================================================
 * Sex Microkernel — AArch64 linker script (Phase 2: entry + serial only)
 *
 * Address model: PHYSICAL, identity-addressed. MMU is OFF in Phase 2.
 *   VMA == LMA == physical address. No higher-half mapping yet (Phase 3).
 *
 * Load address: 0x40080000 — QEMU `virt` direct-kernel (-kernel) convention:
 *   DRAM starts at 0x40000000; QEMU loads an ELF kernel per its program
 *   headers and jumps to e_entry; 0x40080000 leaves the first 512 KiB of
 *   DRAM free (QEMU places the DTB at the start of DRAM; x0 = DTB pointer).
 *
 * No Limine request sections: the x86 boot contract does not apply here.
 * No collision with kernel/linker.ld (x86_64) — separate file, separate
 * build path (scripts/build_arm64_phase2.sh).
 * ========================================================================== */
ENTRY(_start)

PHYS_BASE = 0x40080000;

SECTIONS
{
    . = PHYS_BASE;
    __kernel_start = .;

    .text : ALIGN(4K) {
        KEEP(*(.text.boot))      /* early entry must survive */
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
        *(.data.rel.ro .data.rel.ro.*)
    }

    .bss : ALIGN(4K) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        __boot_stack_bottom = .;
        . = . + 0x10000;         /* 64 KiB boot stack (zeroed with BSS) */
        __boot_stack_top = .;
        __bss_end = .;
    }

    __kernel_end = .;

    /DISCARD/ : {
        *(.comment)
        *(.note*)
        *(.eh_frame*)
    }
}
