// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! EDID 1.3/1.4 with CTA-861 extensions: what a monitor says about itself.
//! Written from the VESA E-EDID and CTA-861 layouts; drivers read the bytes
//! (GMBUS, DP AUX, virtio-gpu GET_EDID) and hand them here.

use alloc::string::String;
use alloc::vec::Vec;
use crate::mode::{cvt_rb, cta_vic, Mode, ESTABLISHED, FLAG_HSYNC_POSITIVE, FLAG_INTERLACED, FLAG_PREFERRED, FLAG_VSYNC_POSITIVE};

pub const BLOCK: usize = 128;
pub const HEADER: [u8; 8] = [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdidError { TooShort, BadHeader, BadChecksum }

/// Refresh and clock limits from the range descriptor (tag 0xFD).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Range { pub min_hz: u8, pub max_hz: u8, pub max_clock_khz: u32 }

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Edid {
    /// Three-letter PNP id, e.g. "SHP", "AUO", "DEL".
    pub vendor: [u8; 3],
    pub product: u16,
    pub serial: u32,
    pub year: u16,
    pub version: (u8, u8),
    pub digital: bool,
    /// Bits per colour from EDID 1.4 digital input, when stated.
    pub bpc: Option<u8>,
    pub width_mm: u16,
    pub height_mm: u16,
    pub name: String,
    pub range: Option<Range>,
    /// Every timing the monitor offers, preferred first, duplicates removed.
    pub modes: Vec<Mode>,
    pub hdmi: bool,
    pub audio: bool,
    /// Extension blocks the header announced.
    pub extensions: u8,
    /// Extension blocks skipped because their checksum failed.
    pub bad_extensions: u8,
}

pub fn checksum_ok(block: &[u8]) -> bool {
    block.len() >= BLOCK && block[..BLOCK].iter().fold(0u8, |a, b| a.wrapping_add(*b)) == 0
}

/// Bytes a reader must fetch for the whole EDID, from the base block alone.
pub fn total_len(base: &[u8]) -> usize {
    if base.len() < BLOCK { BLOCK } else { BLOCK * (1 + base[126] as usize) }
}

impl Edid {
    pub fn parse(bytes: &[u8]) -> Result<Edid, EdidError> {
        if bytes.len() < BLOCK { return Err(EdidError::TooShort); }
        if bytes[..8] != HEADER { return Err(EdidError::BadHeader); }
        if !checksum_ok(bytes) { return Err(EdidError::BadChecksum); }
        let b = &bytes[..BLOCK];
        let id = u16::from_be_bytes([b[8], b[9]]);
        let letter = |s: u16| b'A' - 1 + (s & 0x1f) as u8;
        let mut e = Edid {
            vendor: [letter(id >> 10), letter(id >> 5), letter(id)],
            product: u16::from_le_bytes([b[10], b[11]]),
            serial: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
            year: if b[17] == 0 { 0 } else { 1990 + b[17] as u16 },
            version: (b[18], b[19]),
            digital: b[20] & 0x80 != 0,
            width_mm: b[21] as u16 * 10,
            height_mm: b[22] as u16 * 10,
            extensions: b[126],
            ..Edid::default()
        };
        if e.digital && e.version >= (1, 4) {
            e.bpc = match (b[20] >> 4) & 7 { 1 => Some(6), 2 => Some(8), 3 => Some(10), 4 => Some(12), 5 => Some(14), 6 => Some(16), _ => None };
        }
        let mut sized = false;
        for (i, d) in b[54..126].chunks(18).enumerate() {
            if let Some((mut m, wmm, hmm)) = detailed(d) {
                if i == 0 { m.flags |= FLAG_PREFERRED; }
                if !sized && wmm > 0 && hmm > 0 { e.width_mm = wmm; e.height_mm = hmm; sized = true; }
                e.push(m);
            } else if d[0] == 0 && d[1] == 0 {
                match d[3] {
                    0xFC => e.name = text(&d[5..18]),
                    0xFD => e.range = Some(Range { min_hz: d[5], max_hz: d[6], max_clock_khz: d[9] as u32 * 10_000 }),
                    _ => {}
                }
            }
        }
        for bit in 0..16 {
            let byte = b[35 + bit / 8];
            if byte & (0x80 >> (bit % 8)) != 0 {
                if let Some(m) = ESTABLISHED[bit] { e.push(m); }
            }
        }
        for s in b[38..54].chunks(2) {
            if (s[0] == 1 && s[1] == 1) || s[0] == 0 { continue; }
            let w = (s[0] as u16 + 31) * 8;
            let h = match s[1] >> 6 {
                0 => if e.version >= (1, 3) { w * 10 / 16 } else { w },
                1 => w * 3 / 4, 2 => w * 4 / 5, _ => w * 9 / 16,
            };
            if let Some(m) = cvt_rb(w, h, (s[1] & 0x3f) as u16 + 60) { e.push(m); }
        }
        let blocks = (bytes.len() / BLOCK).min(1 + e.extensions as usize);
        for n in 1..blocks {
            let x = &bytes[n * BLOCK..(n + 1) * BLOCK];
            if !checksum_ok(x) { e.bad_extensions += 1; continue; }
            if x[0] == 0x02 { e.cta(x); }
        }
        Ok(e)
    }

    fn push(&mut self, m: Mode) {
        if !m.valid() || self.modes.iter().any(|o| o.same_timing(&m)) { return; }
        self.modes.push(m);
    }

    fn cta(&mut self, x: &[u8]) {
        let dtd = (x[2] as usize).min(127);
        if x[1] >= 2 && x[3] & 0x40 != 0 { self.audio = true; }
        let mut i = 4;
        while i < dtd {
            let tag = x[i] >> 5;
            let len = (x[i] & 0x1f) as usize;
            let body = &x[(i + 1).min(dtd)..(i + 1 + len).min(dtd)];
            match tag {
                1 => if !body.is_empty() { self.audio = true; },
                2 => for &v in body {
                    let vic = if (129..=192).contains(&v) { v & 0x7f } else { v };
                    if let Some(m) = cta_vic(vic) { self.push(m); }
                },
                3 if body.len() >= 3 => {
                    let oui = body[0] as u32 | (body[1] as u32) << 8 | (body[2] as u32) << 16;
                    if oui == 0x000C03 || oui == 0xC45DD8 { self.hdmi = true; }
                }
                _ => {}
            }
            i += 1 + len;
        }
        if dtd >= 4 {
            let mut o = dtd;
            while o + 18 <= 127 {
                match detailed(&x[o..o + 18]) { Some((m, _, _)) => self.push(m), None => break }
                o += 18;
            }
        }
    }

    pub fn preferred(&self) -> Option<Mode> {
        self.modes.iter().copied().find(|m| m.preferred()).or_else(|| self.modes.first().copied())
    }
    pub fn vendor_str(&self) -> &str { core::str::from_utf8(&self.vendor).unwrap_or("???") }
}

/// A detailed timing descriptor: the mode and its image size in millimetres.
pub fn detailed(d: &[u8]) -> Option<(Mode, u16, u16)> {
    if d.len() < 18 { return None; }
    let clock = u16::from_le_bytes([d[0], d[1]]) as u32 * 10;
    if clock == 0 { return None; }
    let ha = d[2] as u16 | ((d[4] >> 4) as u16) << 8;
    let hb = d[3] as u16 | ((d[4] & 0xf) as u16) << 8;
    let va = d[5] as u16 | ((d[7] >> 4) as u16) << 8;
    let vb = d[6] as u16 | ((d[7] & 0xf) as u16) << 8;
    let hso = d[8] as u16 | (((d[11] >> 6) & 3) as u16) << 8;
    let hsw = d[9] as u16 | (((d[11] >> 4) & 3) as u16) << 8;
    let vso = (d[10] >> 4) as u16 | (((d[11] >> 2) & 3) as u16) << 4;
    let vsw = (d[10] & 0xf) as u16 | ((d[11] & 3) as u16) << 4;
    let wmm = d[12] as u16 | ((d[14] >> 4) as u16) << 8;
    let hmm = d[13] as u16 | ((d[14] & 0xf) as u16) << 8;
    let mut flags = 0;
    if d[17] & 0x80 != 0 { flags |= FLAG_INTERLACED; }
    if (d[17] >> 3) & 3 == 3 {
        if d[17] & 4 != 0 { flags |= FLAG_VSYNC_POSITIVE; }
        if d[17] & 2 != 0 { flags |= FLAG_HSYNC_POSITIVE; }
    }
    let m = Mode::new(clock, [ha, ha + hso, ha + hso + hsw, ha + hb], [va, va + vso, va + vso + vsw, va + vb], flags);
    Some((m, wmm, hmm))
}

fn text(b: &[u8]) -> String {
    let mut s = String::new();
    for &c in b { if c == 0x0a || c == 0 { break; } if (0x20..0x7f).contains(&c) { s.push(c as char); } }
    String::from(s.trim_end())
}
