// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The pcode mailbox: questions to the GPU's power-management firmware. The
//! display needs two answers from it: the memory latency of each watermark
//! level, and on Gen12 how long SAGV (system agent voltage/frequency
//! changes) can block memory. Layout per the PRM's "GT/Display pcode
//! mailbox" (MAILBOX, DATA0, DATA1; command in bits 7:0, ready in bit 31).

use crate::{wait_for, Error, Mmio};

pub const MAILBOX: u32 = 0x138124;
pub const DATA0: u32 = 0x138128;
pub const DATA1: u32 = 0x13812c;
pub const READY: u32 = 1 << 31;
pub const ERROR_MASK: u32 = 0xff;
pub const READ_MEM_LATENCY: u32 = 0x6;
pub const READ_SAGV_BLOCK_TIME_US: u32 = 0x23;

/// One read: DATA0 in, DATA0 out.
pub fn read(m: &mut impl Mmio, cmd: u32, data0: u32, budget: u32) -> Result<u32, Error> {
    if m.read(MAILBOX) & READY != 0 { return Err(Error::Timeout(MAILBOX)); }
    m.write(DATA0, data0);
    m.write(DATA1, 0);
    m.write(MAILBOX, READY | cmd);
    let v = wait_for(m, MAILBOX, READY, 0, budget)?;
    if v & ERROR_MASK != 0 { return Err(Error::NotSupported); }
    Ok(m.read(DATA0))
}

/// Watermark latencies (µs) for levels 0..=7, adjusted as the PRM says: a
/// zero level disables every deeper one, levels never drop below the one
/// before, and a level 0 of zero means the read latency was left out (add
/// 3 µs on Gen12 to every valid level).
pub fn wm_latency(m: &mut impl Mmio, budget: u32) -> Result<[u16; 8], Error> {
    let lo = read(m, READ_MEM_LATENCY, 0, budget)?;
    let hi = read(m, READ_MEM_LATENCY, 1, budget)?;
    let mut w = [0u16; 8];
    for i in 0..4 {
        w[i] = ((lo >> (8 * i)) & 0xff) as u16;
        w[i + 4] = ((hi >> (8 * i)) & 0xff) as u16;
    }
    Ok(adjust(w))
}

pub fn adjust(mut w: [u16; 8]) -> [u16; 8] {
    if let Some(first_zero) = (1..8).find(|&l| w[l] == 0) { for l in first_zero..8 { w[l] = 0; } }
    for l in 1..8 { if w[l] == 0 { break; } w[l] = w[l].max(w[l - 1]); }
    if w[0] == 0 {
        w[0] += 3;
        for l in 1..8 { if w[l] == 0 { break; } w[l] += 3; }
    }
    w
}

/// Gen12 SAGV block time in µs (0 when pcode does not say).
pub fn sagv_block_us(m: &mut impl Mmio, budget: u32) -> u32 {
    read(m, READ_SAGV_BLOCK_TIME_US, 0, budget).unwrap_or(0)
}
