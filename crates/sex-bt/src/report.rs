// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Devices found by scanning: classic inquiry results (plain, with RSSI, extended with
//! EIR data) and LE advertising reports (legacy and extended), with the AD structures
//! inside them (Core Supplement Part A): name, class of device, appearance, flags.

use crate::hci::{ev, Addr};
use alloc::string::String;
use alloc::vec::Vec;

/// AD / EIR data types.
pub mod ad {
    pub const FLAGS: u8 = 0x01;
    pub const UUID16_SOME: u8 = 0x02;
    pub const UUID16_ALL: u8 = 0x03;
    pub const SHORT_NAME: u8 = 0x08;
    pub const COMPLETE_NAME: u8 = 0x09;
    pub const TX_POWER: u8 = 0x0A;
    pub const CLASS_OF_DEVICE: u8 = 0x0D;
    pub const APPEARANCE: u8 = 0x19;
    pub const MANUFACTURER: u8 = 0xFF;
}

/// One AD structure: (type, data).
pub fn structures(data: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut at = 0usize;
    core::iter::from_fn(move || {
        while at < data.len() {
            let len = data[at] as usize;
            if len == 0 { return None; } // zero padding ends EIR data
            if at + 1 + len > data.len() { return None; }
            let item = (data[at + 1], &data[at + 2..at + 1 + len]);
            at += 1 + len;
            return Some(item);
        }
        None
    })
}

/// Encode AD structures (for tests and for our own advertising later).
pub fn encode(items: &[(u8, &[u8])]) -> Vec<u8> {
    let mut v = Vec::new();
    for (t, d) in items { v.push(d.len() as u8 + 1); v.push(*t); v.extend_from_slice(d); }
    v
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport { Classic, Le }

/// A device seen while scanning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub addr: Addr,
    pub transport: Transport,
    /// LE address type (0 public, 1 random); 0 for classic.
    pub addr_type: u8,
    pub rssi: Option<i8>,
    pub class: Option<u32>,
    pub name: Option<String>,
    pub appearance: Option<u16>,
    pub flags: Option<u8>,
    pub connectable: bool,
}

impl Found {
    fn new(addr: Addr, transport: Transport) -> Found {
        Found { addr, transport, addr_type: 0, rssi: None, class: None, name: None, appearance: None, flags: None, connectable: transport == Transport::Classic }
    }
    /// Fill name, class, appearance and flags from AD/EIR data. A complete name wins
    /// over a shortened one.
    pub fn absorb(&mut self, data: &[u8]) {
        let mut short = None;
        for (t, d) in structures(data) {
            match t {
                ad::COMPLETE_NAME => self.name = Some(String::from_utf8_lossy(d).into_owned()),
                ad::SHORT_NAME => short = Some(String::from_utf8_lossy(d).into_owned()),
                ad::CLASS_OF_DEVICE if d.len() == 3 => self.class = Some(u24(d)),
                ad::APPEARANCE if d.len() == 2 => self.appearance = Some(u16::from_le_bytes([d[0], d[1]])),
                ad::FLAGS if !d.is_empty() => self.flags = Some(d[0]),
                _ => {}
            }
        }
        if self.name.is_none() { self.name = short; }
    }
}

fn u24(b: &[u8]) -> u32 { b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16 }
fn addr(b: &[u8]) -> Addr { let mut a = [0; 6]; a.copy_from_slice(&b[..6]); Addr(a) }

/// The devices in one report event (`hci::Event::Report`). Malformed reports give
/// nothing rather than half a device.
pub fn reports(code: u8, sub: u8, p: &[u8]) -> Vec<Found> {
    let mut out = Vec::new();
    match (code, sub) {
        // Inquiry Result: num, then per-field arrays: addr[6], psrm, reserved[2], class[3], clock[2].
        (ev::INQUIRY_RESULT, _) => {
            let Some(&n) = p.first() else { return out };
            let n = n as usize;
            if p.len() < 1 + n * 14 { return out; }
            for i in 0..n {
                let mut f = Found::new(addr(&p[1 + 6 * i..]), Transport::Classic);
                f.class = Some(u24(&p[1 + n * 9 + 3 * i..]));
                out.push(f);
            }
        }
        // With RSSI: addr[6], psrm, reserved, class[3], clock[2], rssi.
        (ev::INQUIRY_RESULT_WITH_RSSI, _) => {
            let Some(&n) = p.first() else { return out };
            let n = n as usize;
            if p.len() < 1 + n * 14 { return out; }
            for i in 0..n {
                let mut f = Found::new(addr(&p[1 + 6 * i..]), Transport::Classic);
                f.class = Some(u24(&p[1 + n * 8 + 3 * i..]));
                f.rssi = Some(p[1 + n * 13 + i] as i8);
                out.push(f);
            }
        }
        // Extended: one response, addr, psrm, reserved, class, clock, rssi, EIR[240].
        (ev::EXTENDED_INQUIRY_RESULT, _) => {
            if p.len() < 15 || p[0] != 1 { return out; }
            let mut f = Found::new(addr(&p[1..]), Transport::Classic);
            f.class = Some(u24(&p[9..]));
            f.rssi = Some(p[14] as i8);
            f.absorb(&p[15..]);
            out.push(f);
        }
        // LE Advertising Report: num, event_type[n], addr_type[n], addr[n][6], len[n], data..., rssi[n].
        (ev::LE_META, ev::LE_ADVERTISING_REPORT) => {
            let Some(&n) = p.first() else { return out };
            let n = n as usize;
            let lens_at = 1 + n * 8;
            if p.len() < lens_at + n { return out; }
            let total: usize = p[lens_at..lens_at + n].iter().map(|&l| l as usize).sum();
            let data_at = lens_at + n;
            if p.len() < data_at + total + n { return out; }
            let mut at = data_at;
            for i in 0..n {
                let kind = p[1 + i];
                let mut f = Found::new(addr(&p[1 + 2 * n + 6 * i..]), Transport::Le);
                f.addr_type = p[1 + n + i];
                // ADV_IND (0) and ADV_DIRECT_IND (1) are connectable.
                f.connectable = kind <= 1;
                let len = p[lens_at + i] as usize;
                f.absorb(&p[at..at + len]);
                at += len;
                f.rssi = Some(p[data_at + total + i] as i8);
                out.push(f);
            }
        }
        // LE Extended Advertising Report: num, then per report 24 fixed bytes + data.
        (ev::LE_META, ev::LE_EXTENDED_ADVERTISING_REPORT) => {
            let Some(&n) = p.first() else { return out };
            let mut at = 1;
            for _ in 0..n {
                if p.len() < at + 24 { return out; }
                let r = &p[at..];
                let len = r[23] as usize;
                if r.len() < 24 + len { return out; }
                let kind = u16::from_le_bytes([r[0], r[1]]);
                let mut f = Found::new(addr(&r[3..]), Transport::Le);
                f.addr_type = r[2];
                f.connectable = kind & 1 != 0;
                f.rssi = Some(r[13] as i8);
                f.absorb(&r[24..24 + len]);
                out.push(f);
                at += 24 + len;
            }
        }
        _ => {}
    }
    out
}
