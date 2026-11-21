// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Native AMD Radeon display engine (DCN) driver logic, for Ryzen APUs (Renoir,
//! Cezanne, Rembrandt, Phoenix, Hawk Point, Strix) and RDNA dGPUs (Navi 1x-3x).
//!
//! Written for SexOS from AMD's MIT-licensed register headers
//! (`drivers/gpu/drm/amd/include/asic_reg/dcn/dcn_*_offset.h` and `_sh_mask.h`)
//! and the behaviour of the MIT-licensed DC code as a reference
//! (docs/BRIDGE_LEDGER.md table 2). No Linux driver code runs here and no
//! Tuxedo bridge is involved.
//!
//! Everything goes through the [`Mmio`] trait, so the same code drives the
//! register BAR (BAR 5) in a display service and the simulated registers in
//! the host tests. Slice A1 (this crate's first goal) takes the panel over
//! from firmware without a modeset: read the OTG timing and HUBP surface that
//! GOP set up, read the panel's EDID over the DCN AUX engine, then flip our
//! own framebuffers at vblank through HUBP's surface address.
#![no_std]
#[cfg(feature = "kms")]
extern crate alloc;

pub mod asic;
pub mod aux;
pub mod plane;
pub mod readout;
pub mod regs;

pub use asic::{identify, Asic, Layout};

/// 32-bit register access to the MMIO BAR. Offsets are bytes.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Timeout(u32),
    /// The sink did not answer (no monitor, hot-plug low, wrong address).
    Nak,
    /// The sink kept deferring.
    Defer,
    /// AUX receive error or garbled reply.
    Garbled,
    /// The display microcontroller (DMCU/DMUB) holds the AUX engine.
    Busy,
    BadArgument,
    NotSupported,
}
