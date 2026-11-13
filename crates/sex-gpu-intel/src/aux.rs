// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! DisplayPort AUX channel: native DPCD reads/writes and I2C-over-AUX (EDID)
//! for eDP panels and DP monitors. One transaction carries at most 16 data
//! bytes; requests and replies travel in the five AUX data registers, MSB first.

use crate::regs::*;
use crate::{wait_for, Error, Mmio};

pub const NATIVE_WRITE: u8 = 0x8;
pub const NATIVE_READ: u8 = 0x9;
pub const I2C_WRITE: u8 = 0x0;
pub const I2C_READ: u8 = 0x1;
/// Middle-of-transaction: keep the I2C bus claimed after this request.
pub const I2C_MOT: u8 = 0x4;
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
    pub const EDP_BACKLIGHT_CAP: u32 = 0x701;
    pub const SET_POWER: u32 = 0x600;
}

/// What a sink replied, in its status nibble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply { Ack, Nak, Defer }
fn reply(byte: u8, i2c: bool) -> Reply {
    let v = if i2c { (byte >> 6) & 3 } else { (byte >> 4) & 3 };
    let native = (byte >> 4) & 3;
    if native == 1 { return Reply::Nak; }
    if native == 2 { return Reply::Defer; }
    match v { 0 => Reply::Ack, 1 => Reply::Nak, _ => Reply::Defer }
}

pub fn pack(src: &[u8]) -> u32 {
    let mut v = 0u32;
    for (i, b) in src.iter().take(4).enumerate() { v |= (*b as u32) << ((3 - i) * 8); }
    v
}
pub fn unpack(v: u32, dst: &mut [u8]) {
    for (i, b) in dst.iter_mut().take(4).enumerate() { *b = (v >> ((3 - i) * 8)) as u8; }
}

/// Send one raw AUX message (header + data, up to 20 bytes) and collect the
/// raw reply (status byte first). Returns the reply length.
pub fn raw(m: &mut impl Mmio, port: u32, msg: &[u8], out: &mut [u8; 20]) -> Result<usize, Error> {
    if msg.is_empty() || msg.len() > 20 { return Err(Error::BadArgument); }
    let ctl = aux_ctl(port);
    wait_for(m, ctl, AUX_SEND_BUSY, 0, BUDGET)?;
    for (i, c) in msg.chunks(4).enumerate() { m.write(aux_data(port, i as u32), pack(c)); }
    m.write(ctl, AUX_SEND_BUSY | AUX_DONE | AUX_TIME_OUT_ERROR | AUX_RECEIVE_ERROR | AUX_TIME_OUT_MAX
        | (msg.len() as u32) << AUX_SIZE_SHIFT | aux_fw_sync(32) | aux_sync(32));
    let st = wait_for(m, ctl, AUX_SEND_BUSY, 0, BUDGET)?;
    // Write-one-to-clear the status bits we saw.
    m.write(ctl, st & (AUX_DONE | AUX_TIME_OUT_ERROR | AUX_RECEIVE_ERROR));
    if st & AUX_TIME_OUT_ERROR != 0 { return Err(Error::Nak); }
    if st & AUX_RECEIVE_ERROR != 0 { return Err(Error::Garbled); }
    let n = ((st & AUX_SIZE_MASK) >> AUX_SIZE_SHIFT) as usize;
    if n == 0 || n > 20 { return Err(Error::Garbled); }
    for i in 0..n.div_ceil(4) {
        let v = m.read(aux_data(port, i as u32));
        let end = (i * 4 + 4).min(n);
        unpack(v, &mut out[i * 4..end]);
    }
    Ok(n)
}

fn header(cmd: u8, addr: u32, len: usize) -> [u8; 4] {
    [cmd << 4 | ((addr >> 16) & 0xf) as u8, (addr >> 8) as u8, addr as u8, len.saturating_sub(1) as u8]
}

/// Read up to 16 DPCD bytes.
pub fn dpcd_read(m: &mut impl Mmio, port: u32, addr: u32, buf: &mut [u8]) -> Result<(), Error> {
    if buf.is_empty() || buf.len() > MAX_PAYLOAD { return Err(Error::BadArgument); }
    let msg = header(NATIVE_READ, addr, buf.len());
    let mut out = [0u8; 20];
    for _ in 0..RETRIES {
        let n = raw(m, port, &msg, &mut out)?;
        match reply(out[0], false) {
            Reply::Ack if n - 1 >= buf.len() => { buf.copy_from_slice(&out[1..1 + buf.len()]); return Ok(()); }
            Reply::Ack => return Err(Error::Garbled),
            Reply::Nak => return Err(Error::Nak),
            Reply::Defer => continue,
        }
    }
    Err(Error::Defer)
}

pub fn dpcd_write(m: &mut impl Mmio, port: u32, addr: u32, data: &[u8]) -> Result<(), Error> {
    if data.is_empty() || data.len() > MAX_PAYLOAD { return Err(Error::BadArgument); }
    let mut msg = [0u8; 20];
    msg[..4].copy_from_slice(&header(NATIVE_WRITE, addr, data.len()));
    msg[4..4 + data.len()].copy_from_slice(data);
    let mut out = [0u8; 20];
    for _ in 0..RETRIES {
        raw(m, port, &msg[..4 + data.len()], &mut out)?;
        match reply(out[0], false) { Reply::Ack => return Ok(()), Reply::Nak => return Err(Error::Nak), Reply::Defer => continue }
    }
    Err(Error::Defer)
}

fn i2c(m: &mut impl Mmio, port: u32, cmd: u8, addr: u8, data: &mut [u8]) -> Result<(), Error> {
    let mut out = [0u8; 20];
    let write = cmd & 0x3 == I2C_WRITE;
    let mut msg = [0u8; 20];
    let len = if data.is_empty() { 3 } else { 4 };
    msg[..4].copy_from_slice(&header(cmd, addr as u32, data.len()));
    let mut n_msg = len;
    if write && !data.is_empty() { msg[4..4 + data.len()].copy_from_slice(data); n_msg += data.len(); }
    for _ in 0..RETRIES {
        let n = raw(m, port, &msg[..n_msg], &mut out)?;
        match reply(out[0], true) {
            Reply::Ack => {
                if !write && !data.is_empty() {
                    if n - 1 < data.len() { return Err(Error::Garbled); }
                    data.copy_from_slice(&out[1..1 + data.len()]);
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
pub fn read_edid(m: &mut impl Mmio, port: u32, out: &mut [u8; 256]) -> Result<usize, Error> {
    let mut total = 128;
    let mut off = 0;
    let r = (|| {
        i2c(m, port, I2C_WRITE | I2C_MOT, crate::gmbus::EDID_ADDR, &mut [0u8])?;
        while off < total {
            i2c(m, port, I2C_READ | I2C_MOT, crate::gmbus::EDID_ADDR, &mut out[off..off + 16])?;
            off += 16;
            if off == 128 && out[126] > 0 { total = 256; }
        }
        Ok(total)
    })();
    // Address-only read without MOT releases the bus, success or not.
    let _ = i2c(m, port, I2C_READ, crate::gmbus::EDID_ADDR, &mut []);
    r
}

/// DisplayPort receiver capabilities from the first DPCD bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Caps { pub rev: u8, pub max_link_khz: u32, pub max_lanes: u8, pub psr: u8, pub edp_rev: u8 }
pub fn caps(m: &mut impl Mmio, port: u32, edp: bool) -> Result<Caps, Error> {
    let mut b = [0u8; 3];
    dpcd_read(m, port, dpcd::REV, &mut b)?;
    let mut c = Caps { rev: b[0], max_link_khz: b[1] as u32 * 270_000, max_lanes: b[2] & 0x1f, ..Caps::default() };
    if edp {
        let mut p = [0u8; 1];
        if dpcd_read(m, port, dpcd::PSR_SUPPORT, &mut p).is_ok() { c.psr = p[0]; }
        if dpcd_read(m, port, dpcd::EDP_REV, &mut p).is_ok() { c.edp_rev = p[0]; }
    }
    Ok(c)
}
