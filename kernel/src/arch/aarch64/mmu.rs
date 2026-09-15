//! AArch64 MMU bring-up — ARM64_PHASE_3_MMU_GLOBAL_VAS_V1.
//!
//! ONE global virtual address space (Sex SAS model; mirrors the x86
//! single-CR3 design). TTBR0_EL1 holds the kernel image (EL1-only), the
//! MMIO identity window, and ALL user mappings (never per-PD switched —
//! per-PD isolation is a Phase 6/7 POE concern, NOT page-table switching).
//! TTBR1_EL1 holds the HHDM window (EL1-only).
//!
//! Scope limits (gate-enforced): NO POE/TCR2/POR register access, NO
//! GIC/timer, NO scheduler. W^X via per-descriptor UXN/PXN + SCTLR.WXN.
//!
//! Platform-fixed to QEMU virt (see registers.rs). Boot CPU only.

use super::registers::*;
use super::serial;
use core::ptr::{read_volatile, write_volatile};

/// One 4 KiB page-table (512 × u64), 4 KiB aligned.
#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct Table {
    e: [u64; 512],
}

/// Static table pool (bump-allocated at boot, before any heap exists).
/// TTBR0 needs: L0, L1, L2(0-1GiB), L2(1-2GiB), L3(kernel), L3(user) = 6.
/// TTBR1 needs: L0, L1 = 2. Pool of 16 leaves Phase-4 headroom.
const POOL: usize = 16;
static mut TABLES: [Table; POOL] = [Table { e: [0; 512] }; POOL];
static mut POOL_NEXT: usize = 0;
/// TTBR0 root, set once during init (== &TABLES[0] by construction order).
static mut T0_ROOT: *mut Table = core::ptr::null_mut();
/// Published TTBR physical addresses (AP cores reuse the SHARED tables —
/// single global VAS across all cores). Written pre-CPU_ON; the BSP
/// cache-clean covers them (see smp.rs).
static mut TTBR0_ADDR: u64 = 0;
static mut TTBR1_ADDR: u64 = 0;

/// TTBR0 table physical address (valid after init()).
pub fn ttbr0_pa() -> u64 { unsafe { TTBR0_ADDR } }
/// TTBR1 table physical address (valid after init()).
pub fn ttbr1_pa() -> u64 { unsafe { TTBR1_ADDR } }

// Linker symbols (kernel image bounds — identity VAs, VMA==LMA).
unsafe extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
    static __bss_start: u8;
}

#[inline]
fn idx0(va: u64) -> usize { ((va >> 39) & 0x1FF) as usize }
#[inline]
fn idx1(va: u64) -> usize { ((va >> 30) & 0x1FF) as usize }
#[inline]
fn idx2(va: u64) -> usize { ((va >> 21) & 0x1FF) as usize }
#[inline]
fn idx3(va: u64) -> usize { ((va >> 12) & 0x1FF) as usize }

/// Allocate a zeroed table from the static pool.
/// SAFETY: single-core boot, before any concurrency; pool never freed.
unsafe fn alloc_table() -> *mut Table {
    unsafe {
        if POOL_NEXT >= POOL {
            serial::write_str("[sex.arm.mmu] FATAL table_pool_exhausted\n");
            loop { core::hint::spin_loop() }
        }
        let t = &raw mut TABLES[POOL_NEXT];
        POOL_NEXT += 1;
        for i in 0..512 { (*t).e[i] = 0; }
        t
    }
}

/// Return the next-level table for entry `i`, creating it if absent.
/// SAFETY: `t` must be a valid table; only used on table-descriptor paths
/// (never walks through block entries).
unsafe fn next_table(t: *mut Table, i: usize) -> *mut Table {
    unsafe {
        let e = read_volatile(&(*t).e[i]);
        if (e & 0b11) == 0b11 && (e & 0x0000_FFFF_FFFF_F000) != 0 {
            return (e & 0x0000_FFFF_FFFF_F000) as *mut Table;
        }
        let nt = alloc_table();
        write_volatile(&mut (*t).e[i], (nt as u64) | D_VALID_TABLE);
        nt
    }
}

/// Map one 4 KiB page under `root` (creating intermediate tables).
/// SAFETY: boot-only, single core.
unsafe fn map_page(root: *mut Table, va: u64, pa: u64, attrs: u64) {
    unsafe {
        let l1 = next_table(root, idx0(va));
        let l2 = next_table(l1, idx1(va));
        let l3 = next_table(l2, idx2(va));
        write_volatile(&mut (*l3).e[idx3(va)], (pa & 0x0000_FFFF_FFFF_F000) | attrs | D_VALID_TABLE);
    }
}

/// Map a page into the TTBR0 (global lower) space. Used by el0_smoke.
/// SAFETY: only valid after init() has installed T0_ROOT; boot-only.
pub(crate) unsafe fn map_page_t0(va: u64, pa: u64, attrs: u64) {
    unsafe { map_page(T0_ROOT, va, pa, attrs) }
}

/// Set a 1 GiB L1 block descriptor. `l1` is an L1 table.
/// SAFETY: boot-only.
unsafe fn map_block_1g(l1: *mut Table, va: u64, pa: u64, attrs: u64) {
    unsafe {
        write_volatile(&mut (*l1).e[idx1(va)], (pa & 0x0000_FFFF_C000_0000) | attrs | D_VALID_BLOCK);
    }
}

/// Build both address-space halves and enable the MMU. Returns only on
/// success (fatal errors print + halt — no recovery at this stage).
pub fn init() {
    unsafe {
        // ── TTBR0 (lower 48-bit): kernel + MMIO identity + user smoke ──
        let t0_l0 = alloc_table();
        T0_ROOT = t0_l0;
        // Kernel image [__kernel_start, __kernel_end): identity map.
        // Below __bss_start: RO+X (EL1). From __bss_start: RW+NX (EL1).
        // __bss_start is 4 KiB aligned by the linker script; .text/.rodata
        // precede it, the 64 KiB boot stack and smoke statics live in it.
        let kstart = &raw const __kernel_start as u64;
        let kend = &raw const __kernel_end as u64;
        let text_end = &raw const __bss_start as u64;
        let mut va = kstart;
        while va < kend {
            let attrs = if va < text_end {
                D_AF | D_SH_INNER | D_AP_RO_EL1 | D_UXN | ATTR_NORMAL // PXN=0: EL1-executable
            } else {
                D_AF | D_SH_INNER | D_AP_RW_EL1 | D_UXN | D_PXN | ATTR_NORMAL
            };
            map_page(t0_l0, va, va, attrs); // identity: VA == PA (VMA==LMA)
            va += 0x1000;
        }

        // MMIO identity window: one 2 MiB block at 0x0900_0000 covers the
        // PL011 (Device-nGnRnE, EL1-only, XN). GIC comes in Phase 4 via
        // the HHDM device alias (or an added identity block there).
        let l1 = next_table(t0_l0, idx0(PL011_BASE as u64));
        let l2 = next_table(l1, idx1(PL011_BASE as u64));
        write_volatile(
            &mut (*l2).e[idx2(PL011_BASE as u64)],
            (PL011_BASE as u64 & 0x0000_FFFF_FFE0_0000)
                | D_AF | D_AP_RW_EL1 | D_UXN | D_PXN | ATTR_DEVICE
                | D_VALID_BLOCK,
        );

        // User smoke pages (code RX/EL0, data+stack RW/EL0+UXN+PXN).
        super::el0_smoke::map_user_pages();
        // Phase-5 SVC-test code page (RX/EL0).
        super::phase5::map_user_pages();

        // ── TTBR1 (upper 48-bit): HHDM ──
        let t1_l0 = alloc_table();
        TTBR0_ADDR = t0_l0 as u64;
        TTBR1_ADDR = t1_l0 as u64;
        let t1_l1 = next_table(t1_l0, idx0(HHDM_BASE));
        // [0, 1 GiB): device alias (GIC/UART/virtio accessible via HHDM).
        map_block_1g(
            t1_l1, HHDM_BASE + MMIO_BLOCK_BASE, MMIO_BLOCK_BASE,
            D_AF | D_AP_RW_EL1 | D_UXN | D_PXN | ATTR_DEVICE,
        );
        // [0x40000000, 0x80000000): DRAM alias (Normal WB, NX).
        map_block_1g(
            t1_l1, HHDM_BASE + DRAM_BASE, DRAM_BASE,
            D_AF | D_SH_INNER | D_AP_RW_EL1 | D_UXN | D_PXN | ATTR_NORMAL,
        );

        serial::write_str("[sex.arm.mmu] tables=ok\n");

        // ── Program system registers and enable ──
        // DSB: table writes must be visible to the walker before TTBRs.
        core::arch::asm!("dsb sy", options(nostack));
        core::arch::asm!(
            "msr MAIR_EL1, {m}",
            "msr TCR_EL1, {t}",
            "msr TTBR0_EL1, {b0}",
            "msr TTBR1_EL1, {b1}",
            "isb",
            m = in(reg) MAIR_EL1_V,
            t = in(reg) TCR_EL1_V,
            b0 = in(reg) (t0_l0 as u64),
            b1 = in(reg) (t1_l0 as u64),
            options(nostack),
        );
        core::arch::asm!(
            "msr SCTLR_EL1, {s}",
            "isb",
            "ic iallu",
            "dsb sy",
            "isb",
            s = in(reg) SCTLR_EL1_MMU,
            options(nostack),
        );

        // Verify M + WXN actually set (readback — no claim without proof).
        let rb: u64;
        core::arch::asm!("mrs {}, SCTLR_EL1", out(reg) rb, options(nomem, nostack));
        if rb & 0x1 == 0 {
            serial::write_str("[sex.arm.mmu] FATAL mmu_not_enabled\n");
            loop { core::hint::spin_loop() }
        }
        serial::write_str("[sex.arm.mmu] on=1\n");
        if rb & (1 << 19) != 0 {
            serial::write_str("[sex.arm.mmu] wx=1\n");
        } else {
            serial::write_str("[sex.arm.mmu] FATAL wxn_clear\n");
            loop { core::hint::spin_loop() }
        }

        // HHDM live proof: probe magic was written pre-MMU into the user
        // data page (offset 8); read it back through the TTBR1 HHDM alias.
        let probe_pa = super::el0_smoke::user_data_pa() + 8;
        let got = read_volatile((HHDM_BASE + probe_pa) as *const u64);
        if got == super::el0_smoke::HHDM_PROBE_MAGIC {
            serial::write_str("[sex.arm.mmu] hhdm=ok\n");
        } else {
            serial::write_str("[sex.arm.mmu] FATAL hhdm_mismatch got=0x");
            serial::write_hex(got);
            serial::write_str("\n");
            loop { core::hint::spin_loop() }
        }
    }
}
