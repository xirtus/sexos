//! PL011 UART — bounded early output only (Phase 2).
//!
//! Platform-fixed to QEMU `virt` (UART0 @ PL011_BASE). No DTB discovery,
//! no baud/divisor programming: QEMU's PL011 model accepts data without
//! clock setup. Real-hardware init is deferred to a later phase.
//!
//! Ordering: with the MMU off (Phase 2), data accesses are to
//! Device-nGnRnE memory. Accesses to the SAME peripheral are observed in
//! program order, and volatile ops prevent compiler reordering — so no
//! DMB/DSB is required per character. A barrier before first use after
//! enabling the MMU belongs to Phase 3, not here.

use super::registers::PL011_BASE;

const DR: usize = 0x00; // Data register
const FR: usize = 0x18; // Flag register
const FR_TXFF: u32 = 1 << 5; // Transmit FIFO full

#[inline(always)]
fn read_reg(off: usize) -> u32 {
    // SAFETY: PL011_BASE..+0xFFF is the UART0 MMIO window on QEMU virt;
    // reads are idempotent and bounded to Phase-2 fixed platform.
    unsafe { core::ptr::read_volatile((PL011_BASE + off) as *const u32) }
}

#[inline(always)]
fn write_reg(off: usize, v: u32) {
    // SAFETY: see read_reg. Writes only target the data register.
    unsafe { core::ptr::write_volatile((PL011_BASE + off) as *mut u32, v) }
}

/// Wait for TX FIFO space and emit one byte.
pub fn write_byte(b: u8) {
    while read_reg(FR) & FR_TXFF != 0 {
        core::hint::spin_loop();
    }
    write_reg(DR, b as u32);
}

/// Emit a static string slice. No allocation, no formatting framework.
pub fn write_str(s: &str) {
    for &b in s.as_bytes() {
        write_byte(b);
    }
}

/// Emit a u64 as exactly 16 lowercase hex digits (no leading trimming).
pub fn write_hex(v: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut shift = 60i32;
    while shift >= 0 {
        let nib = ((v >> shift) & 0xF) as usize;
        write_byte(HEX[nib]);
        shift -= 4;
    }
}

/// Emit a u64 in decimal (bounded: max 20 digits, no allocation).
pub fn write_dec(v: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    let mut n = v;
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &b in &buf[i..] {
        write_byte(b);
    }
}
