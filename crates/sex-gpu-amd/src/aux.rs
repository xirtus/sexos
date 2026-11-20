// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! DisplayPort AUX through the DCN AUX engine: native DPCD reads and writes and
//! I2C-over-AUX (EDID) for eDP panels and DP monitors. A request is written
//! byte by byte into AUX_SW_DATA (header, length, payload), AUX_SW_GO starts
//! it, and the reply (status byte first) is read back the same way. The
//! engine is shared with the display microcontroller: software asks the
//! arbiter first and gives it back after every transaction.
//!
//! The DDC/AUX pad must already be in AUX mode; firmware leaves it so for the
//! panel it lit. Switching pads (DC_GPIO_*) is a later slice.

use crate::regs::*;
use crate::{wait_for, Error, Mmio};

pub const NATIVE_WRITE: u8 = 0x8;
pub const NATIVE_READ: u8 = 0x9;
pub const I2C_WRITE: u8 = 0x0;
pub const I2C_READ: u8 = 0x1;
/// Middle-of-transaction: keep the I2C bus claimed after this request.
pub const I2C_MOT: u8 = 0x4;
pub const EDID_ADDR: u8 = 0x50;
pub const MAX_PAYLOAD: usize = 16;
const BUDGET: u32 = 10_000;
const RETRIES: u32 = 7;

/// DPCD addresses SexOS reads.
pub mod dpcd {
    pub const REV: u32 = 0x000;
    pub const MAX_LINK_RATE: u32 = 0x001;
    pub const MAX_LANE_COUNT: u32 = 0x002;
    pub const PSR_SUPPORT: u32 = 0x070;
    pub const EDP_REV: u32 = 0x700;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply { Ack, Nak, Defer }
/// The sink's status byte: native reply in bits 5:4, I2C reply in bits 7:6.
pub fn reply(byte: u8, i2c: bool) -> Reply {
    match (byte >> 4) & 3 { 1 => return Reply::Nak, 2 => return Reply::Defer, _ => {} }
    if !i2c { return Reply::Ack; }
    match (byte >> 6) & 3 { 0 => Reply::Ack, 1 => Reply::Nak, _ => Reply::Defer }
}

/// AUX request header: command and 20-bit address, then length - 1 when there is data.
pub fn header(cmd: u8, addr: u32, len: usize) -> [u8; 4] {
    [(cmd << 4) | ((addr >> 16) & 0xF) as u8, (addr >> 8) as u8, addr as u8, len.saturating_sub(1) as u8]
}

fn acquire(m: &mut impl Mmio, ch: u32) -> Result<(), Error> {
    let status = |v: u32| (v & AUX_REG_RW_CNTL_STATUS_MASK) >> AUX_REG_RW_CNTL_STATUS_SHIFT;
    if status(m.read(aux(ch, AUX_ARB_CONTROL))) == ARB_DMCU { return Err(Error::Busy); }
    let control = m.read(aux(ch, AUX_CONTROL));
    if control & AUX_EN == 0 {
        m.write(aux(ch, AUX_CONTROL), control | AUX_EN | AUX_RESET);
        wait_for(m, aux(ch, AUX_CONTROL), AUX_RESET_DONE, AUX_RESET_DONE, BUDGET)?;
        m.write(aux(ch, AUX_CONTROL), (control | AUX_EN) & !AUX_RESET);
        wait_for(m, aux(ch, AUX_CONTROL), AUX_RESET_DONE, 0, BUDGET)?;
    }
    m.modify(aux(ch, AUX_ARB_CONTROL), 0, AUX_SW_USE_AUX_REG_REQ);
    if status(m.read(aux(ch, AUX_ARB_CONTROL))) != ARB_SW { release(m, ch); return Err(Error::Busy); }
    Ok(())
}

fn release(m: &mut impl Mmio, ch: u32) {
    m.modify(aux(ch, AUX_ARB_CONTROL), AUX_SW_USE_AUX_REG_REQ, AUX_SW_DONE_USING_AUX_REG);
}

fn data(byte: u8) -> u32 { (byte as u32) << AUX_SW_DATA_SHIFT }

/// Send one raw AUX message (header + data, up to 20 bytes) and collect the raw
/// reply (status byte first). Returns the reply length.
pub fn raw(m: &mut impl Mmio, ch: u32, msg: &[u8], out: &mut [u8; 20]) -> Result<usize, Error> {
    if msg.len() < 3 || msg.len() > 20 { return Err(Error::BadArgument); }
    acquire(m, ch)?;
    let r = transact(m, ch, msg, out);
    release(m, ch);
    r
}

fn transact(m: &mut impl Mmio, ch: u32, msg: &[u8], out: &mut [u8; 20]) -> Result<usize, Error> {
    m.modify(aux(ch, AUX_INTERRUPT_CONTROL), 0, AUX_SW_DONE_ACK);
    wait_for(m, aux(ch, AUX_SW_STATUS), AUX_SW_DONE, 0, BUDGET)?;
    m.modify(aux(ch, AUX_SW_CONTROL), AUX_SW_START_DELAY_MASK | AUX_SW_WR_BYTES_MASK, (msg.len() as u32) << AUX_SW_WR_BYTES_SHIFT);
    // The first byte sets the index (0); the rest auto-increment.
    m.write(aux(ch, AUX_SW_DATA), AUX_SW_AUTOINCREMENT_DISABLE | data(msg[0]));
    for &b in &msg[1..] { m.write(aux(ch, AUX_SW_DATA), data(b)); }
    m.modify(aux(ch, AUX_SW_CONTROL), 0, AUX_SW_GO);
    let st = wait_for(m, aux(ch, AUX_SW_STATUS), AUX_SW_DONE, AUX_SW_DONE, BUDGET)?;
    if st & AUX_SW_HPD_DISCON != 0 { return Err(Error::Nak); }
    if st & (AUX_SW_RX_TIMEOUT | AUX_SW_RX_TIMEOUT_STATE) != 0 { return Err(Error::Timeout(aux(ch, AUX_SW_STATUS))); }
    if st & AUX_SW_RX_INVALID != 0 { return Err(Error::Garbled); }
    let n = ((st & AUX_SW_REPLY_BYTE_COUNT_MASK) >> AUX_SW_REPLY_BYTE_COUNT_SHIFT) as usize;
    if n == 0 || n > out.len() { return Err(Error::Garbled); }
    m.write(aux(ch, AUX_SW_DATA), AUX_SW_AUTOINCREMENT_DISABLE | AUX_SW_DATA_RW);
    for b in out.iter_mut().take(n) { *b = ((m.read(aux(ch, AUX_SW_DATA)) & AUX_SW_DATA_MASK) >> AUX_SW_DATA_SHIFT) as u8; }
    Ok(n)
}

/// Read up to 16 DPCD bytes.
pub fn dpcd_read(m: &mut impl Mmio, ch: u32, addr: u32, buf: &mut [u8]) -> Result<(), Error> {
    if buf.is_empty() || buf.len() > MAX_PAYLOAD { return Err(Error::BadArgument); }
    let msg = header(NATIVE_READ, addr, buf.len());
    let mut out = [0u8; 20];
    for _ in 0..RETRIES {
        let n = raw(m, ch, &msg, &mut out)?;
        match reply(out[0], false) {
            Reply::Ack if n - 1 >= buf.len() => { buf.copy_from_slice(&out[1..1 + buf.len()]); return Ok(()); }
            Reply::Ack => return Err(Error::Garbled),
            Reply::Nak => return Err(Error::Nak),
            Reply::Defer => continue,
        }
    }
    Err(Error::Defer)
}

pub fn dpcd_write(m: &mut impl Mmio, ch: u32, addr: u32, payload: &[u8]) -> Result<(), Error> {
    if payload.is_empty() || payload.len() > MAX_PAYLOAD { return Err(Error::BadArgument); }
    let mut msg = [0u8; 20];
    msg[..4].copy_from_slice(&header(NATIVE_WRITE, addr, payload.len()));
    msg[4..4 + payload.len()].copy_from_slice(payload);
    let mut out = [0u8; 20];
    for _ in 0..RETRIES {
        raw(m, ch, &msg[..4 + payload.len()], &mut out)?;
        match reply(out[0], false) { Reply::Ack => return Ok(()), Reply::Nak => return Err(Error::Nak), Reply::Defer => continue }
    }
    Err(Error::Defer)
}

fn i2c(m: &mut impl Mmio, ch: u32, cmd: u8, addr: u8, buf: &mut [u8]) -> Result<(), Error> {
    let write = cmd & 0x3 == I2C_WRITE;
    let mut msg = [0u8; 20];
    msg[..4].copy_from_slice(&header(cmd, addr as u32, buf.len()));
    let mut len = if buf.is_empty() { 3 } else { 4 };
    if write && !buf.is_empty() { msg[4..4 + buf.len()].copy_from_slice(buf); len += buf.len(); }
    let mut out = [0u8; 20];
    for _ in 0..RETRIES {
        let n = raw(m, ch, &msg[..len], &mut out)?;
        match reply(out[0], true) {
            Reply::Ack => {
                if !write && !buf.is_empty() {
                    if n - 1 < buf.len() { return Err(Error::Garbled); }
                    buf.copy_from_slice(&out[1..1 + buf.len()]);
                }
                return Ok(());
            }
            Reply::Nak => return Err(Error::Nak),
            Reply::Defer => continue,
        }
    }
    Err(Error::Defer)
}

/// EDID over I2C-over-AUX: set the offset, read 16 bytes at a time, then stop.
/// Reads the base block and, when it says so, the first extension.
pub fn read_edid(m: &mut impl Mmio, ch: u32, out: &mut [u8; 256]) -> Result<usize, Error> {
    let mut total = 128;
    let mut off = 0;
    let r = (|| {
        i2c(m, ch, I2C_WRITE | I2C_MOT, EDID_ADDR, &mut [0u8])?;
        while off < total {
            i2c(m, ch, I2C_READ | I2C_MOT, EDID_ADDR, &mut out[off..off + 16])?;
            off += 16;
            if off == 128 && out[126] > 0 { total = 256; }
        }
        Ok(total)
    })();
    // Address-only read without MOT releases the bus, success or not.
    let _ = i2c(m, ch, I2C_READ, EDID_ADDR, &mut []);
    r
}

/// DisplayPort receiver capabilities from the first DPCD bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Caps { pub rev: u8, pub max_link_khz: u32, pub max_lanes: u8, pub psr: u8, pub edp_rev: u8 }
pub fn caps(m: &mut impl Mmio, ch: u32) -> Result<Caps, Error> {
    let mut b = [0u8; 3];
    dpcd_read(m, ch, dpcd::REV, &mut b)?;
    let mut c = Caps { rev: b[0], max_link_khz: b[1] as u32 * 270_000, max_lanes: b[2] & 0x1F, ..Caps::default() };
    let mut p = [0u8; 1];
    if dpcd_read(m, ch, dpcd::PSR_SUPPORT, &mut p).is_ok() { c.psr = p[0]; }
    if dpcd_read(m, ch, dpcd::EDP_REV, &mut p).is_ok() { c.edp_rev = p[0]; }
    Ok(c)
}
