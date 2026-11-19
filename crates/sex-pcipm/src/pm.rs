// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Device power states (PCI Bus Power Management 1.2, PCIe 5.3.1).
//!
//! D3hot is the state every PCI function must accept: its link drops to L1 (L1.2
//! with substates) and the device stops nearly everything. Coming back to D0 a
//! function that does not set No_Soft_Reset is reset internally and forgets its
//! configuration, so [`save`] keeps what Linux's pci_save_state keeps (the header,
//! PCIe control registers, MSI, MSI-X control, L1 substates and LTR) and
//! [`set_state`] puts it back. Transition times: 10 ms after D3hot entry and exit.
use crate::{find_cap, find_ext_cap, pcie, reg, Bdf, Config};

pub const D0: u8 = 0;
pub const D3HOT: u8 = 3;

const PMCSR: u16 = 4;
const PMCSR_STATE: u32 = 3;
const PMCSR_NO_SOFT_RESET: u32 = 1 << 3;
const PMCSR_PME_STATUS: u32 = 1 << 15;
/// D0 <-> D3hot recovery time (PCI PM 1.2, table 18).
pub const D3HOT_DELAY_US: u32 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The function is absent or reads all ones.
    Absent,
    /// It has no power management capability (it can only be in D0).
    NoPm,
    /// Only D0 and D3hot are asked for.
    BadState,
    /// The state read back is not the one written.
    Stuck(u8),
}

/// Configuration a function may lose in D3hot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    pub header: [u32; 16],
    exp_at: u16,
    exp: [u32; 4],
    msi_at: u16,
    msi: [u32; 6],
    msi_len: u8,
    msix_at: u16,
    msix: u32,
    l1ss_at: u16,
    l1ss: [u32; 2],
    ltr_at: u16,
    ltr: u32,
}

const EXP_SAVED: [u16; 4] = [reg::EXP_DEVCTL, reg::EXP_LNKCTL, reg::EXP_DEVCTL2, reg::EXP_LNKCTL2];

pub fn save<C: Config + ?Sized>(c: &mut C, f: Bdf) -> Saved {
    let mut s = Saved::default();
    for (i, w) in s.header.iter_mut().enumerate() { *w = c.read(f, i as u16 * 4); }
    if let Some((at, _)) = pcie(c, f) {
        s.exp_at = at;
        for (i, off) in EXP_SAVED.iter().enumerate() { s.exp[i] = c.read(f, at + off) & 0xFFFF; }
    }
    if let Some(at) = find_cap(c, f, reg::CAP_MSI) {
        s.msi_at = at;
        let control = c.read(f, at) >> 16;
        // Header, address (two dwords when 64-bit), data, then mask and pending bits.
        s.msi_len = 3 + (control >> 7 & 1) as u8 + if control >> 8 & 1 != 0 { 2 } else { 0 };
        for i in 0..s.msi_len as usize { s.msi[i] = c.read(f, at + i as u16 * 4); }
    }
    if let Some(at) = find_cap(c, f, reg::CAP_MSIX) { s.msix_at = at; s.msix = c.read(f, at); }
    if let Some(at) = find_ext_cap(c, f, reg::EXT_L1SS) {
        s.l1ss_at = at;
        s.l1ss = [c.read(f, at + reg::L1SS_CTL1), c.read(f, at + reg::L1SS_CTL2)];
    }
    if let Some(at) = find_ext_cap(c, f, reg::EXT_LTR) { s.ltr_at = at; s.ltr = c.read(f, at + 4); }
    s
}

/// Writes back what `save` read, in Linux's order: header from the top down with the
/// command register last, then PCIe controls, LTR, L1 substates (enables last), MSI
/// (enable last), MSI-X control.
pub fn restore<C: Config + ?Sized>(c: &mut C, f: Bdf, s: &Saved) {
    for i in (2..16).rev() {
        let at = i as u16 * 4;
        if c.read(f, at) != s.header[i] { c.write(f, at, s.header[i]); }
    }
    // Command only: the status half is write-one-to-clear.
    c.write(f, reg::COMMAND, s.header[1] & 0xFFFF);
    if s.exp_at != 0 {
        for (i, off) in EXP_SAVED.iter().enumerate() { c.write(f, s.exp_at + off, s.exp[i]); }
    }
    if s.ltr_at != 0 { c.write(f, s.ltr_at + 4, s.ltr); }
    if s.l1ss_at != 0 {
        c.write(f, s.l1ss_at + reg::L1SS_CTL2, s.l1ss[1]);
        c.write(f, s.l1ss_at + reg::L1SS_CTL1, s.l1ss[0] & !reg::L1SS_ENABLES);
        c.write(f, s.l1ss_at + reg::L1SS_CTL1, s.l1ss[0]);
    }
    if s.msi_at != 0 {
        let control = s.msi[0] & 0xFFFF_0000;
        c.write(f, s.msi_at, control & !(1 << 16));
        for i in 1..s.msi_len as usize { c.write(f, s.msi_at + i as u16 * 4, s.msi[i]); }
        c.write(f, s.msi_at, control);
    }
    if s.msix_at != 0 { c.write(f, s.msix_at, s.msix & 0xFFFF_0000); }
}

/// The current D-state (0..3), or None without a PM capability.
pub fn state<C: Config + ?Sized>(c: &mut C, f: Bdf) -> Option<u8> {
    let at = find_cap(c, f, reg::CAP_PM)?;
    Some((c.read(f, at + PMCSR) & PMCSR_STATE) as u8)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    pub from: u8,
    pub to: u8,
    /// The function keeps its configuration in D3hot (No_Soft_Reset).
    pub no_soft_reset: bool,
    /// Saved configuration was written back on the way to D0.
    pub restored: bool,
}

/// Moves `f` to D0 or D3hot. Going down saves into `saved`; coming up restores from it
/// when the function was reset.
pub fn set_state<C: Config + ?Sized>(c: &mut C, f: Bdf, to: u8, saved: &mut Option<Saved>) -> Result<Transition, Error> {
    if to != D0 && to != D3HOT { return Err(Error::BadState); }
    if c.read(f, 0) & 0xFFFF == 0xFFFF { return Err(Error::Absent); }
    let at = find_cap(c, f, reg::CAP_PM).ok_or(Error::NoPm)?;
    let csr = c.read(f, at + PMCSR);
    let from = (csr & PMCSR_STATE) as u8;
    let no_soft_reset = csr & PMCSR_NO_SOFT_RESET != 0;
    let mut t = Transition { from, to, no_soft_reset, restored: false };
    if from == to { return Ok(t); }
    if to == D3HOT && from == D0 { *saved = Some(save(c, f)); }
    // PME status is write-one-to-clear: never write it back.
    c.write(f, at + PMCSR, (csr & !(PMCSR_STATE | PMCSR_PME_STATUS)) | to as u32);
    c.delay_us(D3HOT_DELAY_US);
    let now = (c.read(f, at + PMCSR) & PMCSR_STATE) as u8;
    if now != to { return Err(Error::Stuck(now)); }
    if to == D0 && !no_soft_reset {
        if let Some(s) = saved.as_ref() { restore(c, f, s); t.restored = true; }
    }
    Ok(t)
}
