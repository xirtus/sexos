// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! PCI Express power management, free of hardware access so it runs on the host.
//!
//! * [`aspm`]: Active State Power Management on every link (L0s, L1, the L1.1/L1.2
//!   substates), Latency Tolerance Reporting and clock power management (CLKREQ#). An
//!   idle link drops to L1.2 by itself and the platform can reach its deepest package
//!   idle states (PC10 / S0ix on Intel laptops), which needs every link idle-capable.
//! * [`pm`]: a function's D-state (D0 / D3hot) with the configuration it loses on the
//!   way, saved and put back, for drivers that power their device down when idle and
//!   for suspend.
//!
//! The kernel supplies [`Config`] (ECAM or the legacy ports) and calls these at BOOT
//! and on a driver's request. Facts used: the PCI Express Base Specification (5.5
//! L1 PM Substates, 7.5.3 PCIe capability, 7.8.3 L1 PM Substates capability) and the
//! policy of Linux's drivers/pci/pcie/aspm.c, read only.
#![no_std]

pub mod aspm;
pub mod nhi;
pub mod pm;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bdf { pub bus: u8, pub dev: u8, pub func: u8 }
impl Bdf {
    pub const fn new(bus: u8, dev: u8, func: u8) -> Bdf { Bdf { bus, dev, func } }
}
impl core::fmt::Display for Bdf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, "{:02x}:{:02x}.{}", self.bus, self.dev, self.func) }
}

/// Configuration space of segment 0. Reads of absent functions or of extended
/// space without ECAM return all ones.
pub trait Config {
    fn read(&mut self, f: Bdf, at: u16) -> u32;
    fn write(&mut self, f: Bdf, at: u16, value: u32);
    /// Busy-waits; used for the D-state transition times.
    fn delay_us(&mut self, us: u32);
}

pub mod reg {
    pub const COMMAND: u16 = 0x04;
    pub const HEADER: u16 = 0x0C;
    pub const SECONDARY_BUS: u16 = 0x18;
    pub const CAP_PTR: u16 = 0x34;
    pub const STATUS_CAP_LIST: u32 = 1 << 20;

    pub const CAP_PM: u8 = 0x01;
    pub const CAP_MSI: u8 = 0x05;
    pub const CAP_EXP: u8 = 0x10;
    pub const CAP_MSIX: u8 = 0x11;
    pub const EXT_LTR: u16 = 0x18;
    pub const EXT_L1SS: u16 = 0x1E;

    // PCI Express capability, offsets from its start.
    pub const EXP_FLAGS: u16 = 0x00;
    pub const EXP_DEVCAP: u16 = 0x04;
    pub const EXP_DEVCTL: u16 = 0x08;
    pub const EXP_LNKCAP: u16 = 0x0C;
    pub const EXP_LNKCTL: u16 = 0x10;
    pub const EXP_DEVCAP2: u16 = 0x24;
    pub const EXP_DEVCTL2: u16 = 0x28;
    pub const EXP_LNKCTL2: u16 = 0x30;

    /// Device/Port type, PCIe capability register bits 7:4.
    pub const TYPE_ENDPOINT: u8 = 0x0;
    pub const TYPE_LEGACY_ENDPOINT: u8 = 0x1;
    pub const TYPE_ROOT_PORT: u8 = 0x4;
    pub const TYPE_UPSTREAM: u8 = 0x5;
    pub const TYPE_DOWNSTREAM: u8 = 0x6;
    pub const TYPE_PCIE_TO_PCI: u8 = 0x7;
    pub const TYPE_RC_INTEGRATED: u8 = 0x9;

    pub const DEVCAP_RBER: u32 = 1 << 15;
    pub const LNKCAP_ASPM_L0S: u32 = 1 << 10;
    pub const LNKCAP_ASPM_L1: u32 = 1 << 11;
    pub const LNKCAP_CLKPM: u32 = 1 << 18;
    pub const LNKCTL_ASPM_L0S: u32 = 1 << 0;
    pub const LNKCTL_ASPM_L1: u32 = 1 << 1;
    pub const LNKCTL_CLKPM: u32 = 1 << 8;
    pub const DEVCAP2_LTR: u32 = 1 << 11;
    pub const DEVCTL2_LTR: u32 = 1 << 10;

    // L1 PM Substates capability, offsets from its start.
    pub const L1SS_CAP: u16 = 0x04;
    pub const L1SS_CTL1: u16 = 0x08;
    pub const L1SS_CTL2: u16 = 0x0C;
    /// PCI-PM L1.2, PCI-PM L1.1, ASPM L1.2, ASPM L1.1 (same bits in CAP and CTL1).
    pub const L1SS_PCIPM_L1_2: u32 = 1 << 0;
    pub const L1SS_PCIPM_L1_1: u32 = 1 << 1;
    pub const L1SS_ASPM_L1_2: u32 = 1 << 2;
    pub const L1SS_ASPM_L1_1: u32 = 1 << 3;
    pub const L1SS_ENABLES: u32 = 0xF;
    pub const L1SS_L1_2_MASK: u32 = L1SS_PCIPM_L1_2 | L1SS_ASPM_L1_2;
    pub const L1SS_CAP_SUBSTATES: u32 = 1 << 4;
    pub const L1SS_CTL1_CM_RESTORE: u32 = 0xFF << 8;
    pub const L1SS_CTL1_LTR_THRESHOLD: u32 = 0x3FF << 16 | 0x7 << 29;
}

/// Offset of a standard capability, or None.
pub fn find_cap<C: Config + ?Sized>(c: &mut C, f: Bdf, id: u8) -> Option<u16> {
    if c.read(f, reg::COMMAND) & reg::STATUS_CAP_LIST == 0 { return None; }
    let mut at = (c.read(f, reg::CAP_PTR) & 0xFC) as u16;
    for _ in 0..48 {
        if at < 0x40 { return None; }
        let v = c.read(f, at);
        if v & 0xFF == id as u32 { return Some(at); }
        at = ((v >> 8) & 0xFC) as u16;
    }
    None
}

/// Offset of an extended capability (needs ECAM), or None.
pub fn find_ext_cap<C: Config + ?Sized>(c: &mut C, f: Bdf, id: u16) -> Option<u16> {
    let mut at = 0x100u16;
    for _ in 0..(4096 - 256) / 8 {
        let v = c.read(f, at);
        if v == 0 || v == u32::MAX { return None; }
        if v & 0xFFFF == id as u32 { return Some(at); }
        let next = ((v >> 20) & 0xFFC) as u16;
        if next < 0x100 { return None; }
        at = next;
    }
    None
}

pub fn exists<C: Config + ?Sized>(c: &mut C, f: Bdf) -> bool { c.read(f, 0) & 0xFFFF != 0xFFFF }

/// The PCIe capability's offset and the Device/Port type, for a PCIe function.
pub fn pcie<C: Config + ?Sized>(c: &mut C, f: Bdf) -> Option<(u16, u8)> {
    let at = find_cap(c, f, reg::CAP_EXP)?;
    Some((at, ((c.read(f, at) >> 20) & 0xF) as u8))
}

/// Read-modify-write of a 16-bit control register that shares its dword with a
/// status register whose bits are write-one-to-clear: the upper half is written as 0.
pub(crate) fn update16<C: Config + ?Sized>(c: &mut C, f: Bdf, at: u16, clear: u32, set: u32) -> (u32, u32) {
    let before = c.read(f, at) & 0xFFFF;
    let after = (before & !clear) | set;
    if after != before { c.write(f, at, after); }
    (before, after)
}
