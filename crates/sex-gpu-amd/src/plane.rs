// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Page flips on the primary surface: HUBP latches a new surface address at the
//! next vsync. The high half is written first; writing the low half arms the
//! flip (DC's hubp1_program_surface_flip_and_addr), and SURFACE_FLIP_PENDING
//! stays set until the new address is scanned.

use crate::asic::Asic;
use crate::regs::*;
use crate::{Error, Mmio};

/// Arm a vsync flip of pipe `p` to `address` (display address space, 256-byte aligned).
pub fn flip(m: &mut impl Mmio, p: u32, address: u64) -> Result<(), Error> {
    if address & 0xFF != 0 || address >> 48 != 0 { return Err(Error::BadArgument); }
    m.modify(hubp(p, HUBPREQ_FLIP_CONTROL), SURFACE_FLIP_TYPE | SURFACE_UPDATE_LOCK, 0);
    m.write(hubp(p, HUBPREQ_PRIMARY_SURFACE_ADDRESS_HIGH), (address >> 32) as u32 & ADDRESS_HIGH_MASK);
    m.write(hubp(p, HUBPREQ_PRIMARY_SURFACE_ADDRESS), address as u32);
    Ok(())
}

/// The last flip has been scanned out.
pub fn flip_done(m: &mut impl Mmio, p: u32) -> bool { m.read(hubp(p, HUBPREQ_FLIP_CONTROL)) & SURFACE_FLIP_PENDING == 0 }

pub fn frame_count(m: &mut impl Mmio, asic: &Asic, otg_index: u32) -> u32 {
    m.read(otg(otg_index, otg_late(asic.layout).frame_count)) & FRAME_COUNT_MASK
}

/// Wait for the frame counter to move (one vblank). `budget` counts reads.
pub fn wait_vblank(m: &mut impl Mmio, asic: &Asic, otg_index: u32, budget: u32) -> Result<u32, Error> {
    let start = frame_count(m, asic, otg_index);
    for _ in 0..budget.max(1) {
        let now = frame_count(m, asic, otg_index);
        if now != start { return Ok(now); }
    }
    Err(Error::Timeout(otg(otg_index, otg_late(asic.layout).frame_count)))
}

/// Current scanline of an OTG.
pub fn scanline(m: &mut impl Mmio, otg_index: u32) -> u32 { m.read(otg(otg_index, OTG_STATUS_POSITION)) & FIELD15 }
