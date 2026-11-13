// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! GMBUS: the display engine's I2C master on the DDC pins. HDMI and DVI
//! monitors answer EDID here (address 0x50). DP and eDP sinks use AUX instead.

use crate::regs::*;
use crate::{wait_any, wait_for, Error, Mmio};

pub const EDID_ADDR: u8 = 0x50;
const BUDGET: u32 = 50_000;

/// Read `buf.len()` (1..=256) bytes from an I2C device starting at `offset`,
/// using one indexed read cycle (offset write + repeated start + read).
pub fn read(m: &mut impl Mmio, port: u32, addr: u8, offset: u8, buf: &mut [u8]) -> Result<(), Error> {
    let pin = gmbus_pin(port).ok_or(Error::NotSupported)?;
    if buf.is_empty() || buf.len() > 256 { return Err(Error::BadArgument); }
    m.write(GMBUS0, GMBUS_RATE_100K | pin);
    let r = transfer(m, addr, offset, buf);
    if r.is_err() {
        // Clear a NAK or a stuck cycle so the next user starts clean.
        m.write(GMBUS1, GMBUS_SW_CLR_INT);
        m.write(GMBUS1, 0);
    }
    m.write(GMBUS0, 0);
    r
}

fn transfer(m: &mut impl Mmio, addr: u8, offset: u8, buf: &mut [u8]) -> Result<(), Error> {
    m.write(GMBUS1, GMBUS_SW_RDY | GMBUS_CYCLE_INDEX | GMBUS_CYCLE_WAIT
        | (buf.len() as u32) << GMBUS_BYTE_COUNT_SHIFT | (offset as u32) << GMBUS_INDEX_SHIFT
        | (addr as u32) << 1 | GMBUS_READ);
    for chunk in buf.chunks_mut(4) {
        let s = wait_any(m, GMBUS2, GMBUS_HW_RDY | GMBUS_SATOER, BUDGET)?;
        if s & GMBUS_SATOER != 0 { return Err(Error::Nak); }
        let v = m.read(GMBUS3).to_le_bytes();
        chunk.copy_from_slice(&v[..chunk.len()]);
    }
    wait_for(m, GMBUS2, GMBUS_HW_WAIT_PHASE, GMBUS_HW_WAIT_PHASE, BUDGET)?;
    m.write(GMBUS1, GMBUS_SW_RDY | GMBUS_CYCLE_STOP);
    wait_for(m, GMBUS2, GMBUS_ACTIVE, 0, BUDGET)?;
    Ok(())
}

/// The whole EDID (base block and up to one extension) from an HDMI/DVI port.
pub fn read_edid(m: &mut impl Mmio, port: u32, out: &mut [u8; 256]) -> Result<usize, Error> {
    read(m, port, EDID_ADDR, 0, &mut out[..128])?;
    if out[126] == 0 { return Ok(128); }
    read(m, port, EDID_ADDR, 128, &mut out[128..])?;
    Ok(256)
}
