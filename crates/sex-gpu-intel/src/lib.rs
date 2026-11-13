// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Native Intel Gen12 (Tiger Lake) display engine driver logic.
//!
//! Reverse-engineered from Intel's public display PRMs (Tiger Lake, Vol 2c
//! "Command Reference: Registers", Vol 12 "Display Engine") with the MIT
//! licensed i915 register headers as a cross-check. No Linux driver code runs
//! here and no Tuxedo bridge is involved (docs/BRIDGE_LEDGER.md table 2 lists
//! the references).
//!
//! Everything goes through the [`Mmio`] trait, so the same code drives the
//! real BAR in `sexgpu-intel` and the simulated registers in the host tests.
//! The first goal (slice L3) is to take the panel over from firmware without a
//! flicker: read what GOP set up, read the panel's EDID over DP AUX, check the
//! mode is native, then flip our own framebuffers at vblank.
#![no_std]
extern crate alloc;

pub mod aux;
pub mod backlight;
pub mod gmbus;
pub mod hotplug;
pub mod plane;
pub mod readout;
pub mod regs;

/// 32-bit register access to the display MMIO BAR (GTTMMADR, BAR 0).
pub trait Mmio {
    fn read(&mut self, offset: u32) -> u32;
    fn write(&mut self, offset: u32, value: u32);
    fn modify(&mut self, offset: u32, clear: u32, set: u32) {
        let v = self.read(offset);
        self.write(offset, (v & !clear) | set);
    }
}

/// Poll until `reg & mask == want`. `budget` counts reads, so tests are
/// deterministic and a dead device can never hang the service.
pub fn wait_for(m: &mut impl Mmio, reg: u32, mask: u32, want: u32, budget: u32) -> Result<u32, Error> {
    for _ in 0..budget.max(1) {
        let v = m.read(reg);
        if v & mask == want { return Ok(v); }
    }
    Err(Error::Timeout(reg))
}

/// Poll until any bit of `mask` is set.
pub fn wait_any(m: &mut impl Mmio, reg: u32, mask: u32, budget: u32) -> Result<u32, Error> {
    for _ in 0..budget.max(1) {
        let v = m.read(reg);
        if v & mask != 0 { return Ok(v); }
    }
    Err(Error::Timeout(reg))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Timeout(u32),
    /// The sink did not answer (no monitor, wrong address).
    Nak,
    /// The sink kept deferring.
    Defer,
    /// AUX receive error or garbled reply.
    Garbled,
    BadArgument,
    NotSupported,
}

/// PCI device ids of Gen12 Tiger Lake graphics (UP3/UP4/H).
pub const TIGER_LAKE: [u16; 6] = [0x9a40, 0x9a49, 0x9a60, 0x9a68, 0x9a70, 0x9a78];
pub fn supported(vendor: u16, device: u16) -> bool { vendor == 0x8086 && TIGER_LAKE.contains(&device) }
