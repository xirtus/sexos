// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Bonds: the keys that let a paired device come back without pairing again. LE bonds
//! keep the long-term key (LE Secure Connections: EDIV and Rand are zero) and, when the
//! device gave one, its identity resolving key and identity address, so a device that
//! changes its private address is still recognised. Classic bonds keep the link key.
//!
//! [`Bonds::encode`] / [`Bonds::decode`] give the bytes Cufflinks keeps through Files
//! (`sex_pdx::bt_bonds`, file `/system/bluetooth.bonds`): a magic, a version, fixed-size
//! records and a CRC-32 over all of it. A damaged file decodes to nothing rather than to
//! wrong keys.

use crate::crypto;
use crate::hci::Addr;
use crate::ssp;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeBond {
    /// The address it paired from, and its type (0 public, 1 random).
    pub addr: Addr,
    pub addr_type: u8,
    pub ltk: [u8; 16],
    /// Identity resolving key and identity address (type, address), if it gave them.
    pub irk: Option<[u8; 16]>,
    pub identity: Option<(u8, Addr)>,
}

impl LeBond {
    /// Whether an address seen now is this device: its address, its identity address, or
    /// a private address its IRK resolves.
    pub fn matches(&self, addr: &Addr) -> bool {
        self.addr == *addr || self.identity.is_some_and(|(_, a)| a == *addr)
            || self.irk.is_some_and(|k| crypto::resolves(&k, &addr.0))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bonds { pub le: Vec<LeBond>, pub classic: Vec<ssp::Bond> }

pub const MAGIC: &[u8; 4] = b"CUFB";
pub const VERSION: u8 = 1;
const RECORD: usize = 48;
/// Most bonds kept: the oldest goes when a new one would pass this.
pub const MAX: usize = 32;
/// The largest encoded store.
pub const MAX_BYTES: usize = 8 + MAX * RECORD + 4;

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 { c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 }; }
    }
    !c
}

impl Bonds {
    pub fn le_for(&self, addr: &Addr) -> Option<&LeBond> { self.le.iter().find(|b| b.matches(addr)) }
    pub fn classic_for(&self, addr: &Addr) -> Option<&ssp::Bond> { self.classic.iter().find(|b| b.addr == *addr) }
    pub fn len(&self) -> usize { self.le.len() + self.classic.len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Keep an LE bond (replacing one for the same device). False if nothing changed.
    pub fn put_le(&mut self, b: LeBond) -> bool {
        if let Some(old) = self.le.iter_mut().find(|o| o.addr == b.addr || o.matches(&b.addr)) {
            if *old == b { return false; }
            *old = b;
            return true;
        }
        if self.len() >= MAX { if !self.le.is_empty() { self.le.remove(0); } else { self.classic.remove(0); } }
        self.le.push(b);
        true
    }
    pub fn put_classic(&mut self, b: ssp::Bond) -> bool {
        if let Some(old) = self.classic.iter_mut().find(|o| o.addr == b.addr) {
            if *old == b { return false; }
            *old = b;
            return true;
        }
        if self.len() >= MAX { if !self.classic.is_empty() { self.classic.remove(0); } else { self.le.remove(0); } }
        self.classic.push(b);
        true
    }
    pub fn forget(&mut self, addr: &Addr) -> bool {
        let n = self.len();
        self.le.retain(|b| !b.matches(addr));
        self.classic.retain(|b| b.addr != *addr);
        n != self.len()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAX_BYTES);
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(self.len().min(MAX) as u8);
        out.extend_from_slice(&[0, 0]);
        for b in self.le.iter() {
            let mut r = [0u8; RECORD];
            r[0] = 1;
            r[1] = b.addr_type;
            r[2] = b.irk.is_some() as u8 | (b.identity.is_some() as u8) << 1;
            r[3..9].copy_from_slice(&b.addr.0);
            r[9..25].copy_from_slice(&b.ltk);
            r[25..41].copy_from_slice(&b.irk.unwrap_or([0; 16]));
            if let Some((t, a)) = b.identity { r[41] = t; r[42..48].copy_from_slice(&a.0); }
            out.extend_from_slice(&r);
        }
        for b in self.classic.iter() {
            let mut r = [0u8; RECORD];
            r[0] = 2;
            r[1] = b.key_type;
            r[3..9].copy_from_slice(&b.addr.0);
            r[9..25].copy_from_slice(&b.key);
            out.extend_from_slice(&r);
        }
        let crc = crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    /// The store in `bytes`; None if it is not one, or is damaged.
    pub fn decode(bytes: &[u8]) -> Option<Bonds> {
        if bytes.len() < 12 || &bytes[..4] != MAGIC || bytes[4] != VERSION { return None; }
        let n = bytes[5] as usize;
        let end = 8 + n * RECORD;
        if n > MAX || bytes.len() < end + 4 { return None; }
        if crc32(&bytes[..end]) != u32::from_le_bytes(bytes[end..end + 4].try_into().ok()?) { return None; }
        let mut out = Bonds::default();
        for r in bytes[8..end].chunks(RECORD) {
            let addr = Addr(r[3..9].try_into().ok()?);
            let key: [u8; 16] = r[9..25].try_into().ok()?;
            match r[0] {
                1 => out.le.push(LeBond { addr, addr_type: r[1] & 1, ltk: key,
                    irk: (r[2] & 1 != 0).then(|| r[25..41].try_into().unwrap()),
                    identity: (r[2] & 2 != 0).then(|| (r[41] & 1, Addr(r[42..48].try_into().unwrap()))) }),
                2 => out.classic.push(ssp::Bond { addr, key, key_type: r[1] }),
                _ => return None,
            }
        }
        Some(out)
    }
}
