// SPDX-License-Identifier: GPL-3.0-only
//! Lending memory to a driver's devices through the IOMMU (syscall 90).
//!
//! With an IOMMU each driver service's devices have their own DMA domain: by
//! default they reach only the service's BOOT DMA pool (at its physical
//! address) and firmware-reserved memory. A service lends more of the memory
//! it already holds, a capability region such as a shared block buffer, and
//! gets back a device address to program into its hardware. Returning the
//! loan unmaps it and flushes the IOMMU, after which the device faults instead
//! of reaching the memory. Without an IOMMU the kernel refuses every loan, so
//! drivers keep bounce-copying through their pool.
//!
//! Registers: rax = 90, rdi = op, rsi/rdx/r10/r8 = arguments.
//! Reply: rax = status, rsi/rdx/r10/r8 = values.

pub const SYS_DMA: u64 = 90;

/// rsi = 1 when this service's devices are behind an active IOMMU,
/// rdx = domain id, r10 = faults seen, r8 = pages on loan.
pub const OP_INFO: u64 = 0;
/// rsi = region slot, rdx = byte offset, r10 = bytes, r8 = DEVICE_READ|DEVICE_WRITE.
/// Reply rsi = device address of the first byte, rdx = pages.
pub const OP_LEND: u64 = 1;
/// rsi = device address a LEND returned. Reply rsi = pages returned.
pub const OP_RETURN: u64 = 2;

/// The device may read the memory (the service sends it data).
pub const DEVICE_READ: u64 = 1;
/// The device may write the memory (the service receives data). Needs the
/// region's WRITE right.
pub const DEVICE_WRITE: u64 = 2;

pub const DENIED: u64 = u64::MAX;
/// No IOMMU translates this service's devices; keep using the BOOT pool.
pub const NO_IOMMU: u64 = u64::MAX - 1;
/// The service's device-address window or loan table is full.
pub const EXHAUSTED: u64 = u64::MAX - 2;
/// RETURN: the IOMMU did not confirm its flush. The device may still reach
/// the memory; treat it as still lent (the device address is never reused).
pub const UNCERTAIN: u64 = u64::MAX - 3;

/// Most loans a service may hold at once.
pub const MAX_LOANS: usize = 64;
/// Device-address window per service, in bytes.
pub const WINDOW_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info { pub active: bool, pub domain: u64, pub faults: u64, pub pages_lent: u64 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loan { pub device_address: u64, pub pages: u64 }

fn call(op: u64, a: u64, b: u64, c: u64, d: u64) -> [u64; 5] {
    let (status, r1, r2, r3, r4): (u64, u64, u64, u64, u64);
    unsafe { core::arch::asm!("syscall",
        inlateout("rax") SYS_DMA => status, in("rdi") op,
        inlateout("rsi") a => r1, inlateout("rdx") b => r2,
        inlateout("r10") c => r3, inlateout("r8") d => r4,
        out("rcx") _, out("r11") _, options(nostack)); }
    [status, r1, r2, r3, r4]
}

pub fn info() -> Info {
    let r = call(OP_INFO, 0, 0, 0, 0);
    if r[0] != 0 { return Info { active: false, domain: 0, faults: 0, pages_lent: 0 }; }
    Info { active: r[1] == 1, domain: r[2], faults: r[3], pages_lent: r[4] }
}

/// Lend `bytes` at `offset` of the region held at `slot` to this service's devices.
pub fn lend(slot: u64, offset: u64, bytes: u64, access: u64) -> Result<Loan, u64> {
    let r = call(OP_LEND, slot, offset, bytes, access);
    if r[0] != 0 { return Err(r[0]); }
    Ok(Loan { device_address: r[1], pages: r[2] })
}

/// End a loan. The device must be done with it: later DMA there faults.
pub fn give_back(device_address: u64) -> Result<u64, u64> {
    let r = call(OP_RETURN, device_address, 0, 0, 0);
    if r[0] != 0 { return Err(r[0]); }
    Ok(r[1])
}
