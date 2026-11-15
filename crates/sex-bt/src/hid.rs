// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! HID reports from Bluetooth keyboards and mice, in the shape SexUSB already sends to
//! Input: a boot-keyboard snapshot (modifier bits and up to six usage ids) or relative
//! pointer motion.
//!
//! Two sources: boot protocol reports (classic HID after SET_PROTOCOL boot, and the HOGP
//! Boot Keyboard/Mouse Input Report characteristics), whose layout is fixed; and report
//! protocol reports, laid out by the device's report map (HOGP's Report Map
//! characteristic), which [`ReportMap::parse`] reads. Only what a keyboard or a mouse
//! needs is kept: modifier bits, the key array, buttons, X, Y and the wheel.

use alloc::vec::Vec;

/// One decoded input report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// Modifier bits (bit 0 left control .. bit 7 right GUI) and up to six pressed usage ids.
    Keys { modifiers: u8, keys: [u8; 6] },
    /// Buttons (bit 0 left, 1 right, 2 middle), relative motion and wheel.
    Pointer { buttons: u8, dx: i16, dy: i16, wheel: i8 },
}

impl Input {
    /// The six keys packed one per byte, first in the low byte (SexUSB's encoding).
    pub fn packed_keys(keys: &[u8; 6]) -> u64 {
        keys.iter().enumerate().fold(0u64, |a, (i, k)| a | (*k as u64) << (8 * i))
    }
}

/// Boot keyboard report: modifiers, reserved, six keys. Phantom state (rollover: every
/// key reads 0x01) gives None, so the last state stays.
pub fn boot_keyboard(r: &[u8]) -> Option<Input> {
    if r.len() < 3 { return None; }
    let body = &r[2..r.len().min(8)];
    if body.iter().any(|&k| k == 1) { return None; }
    let mut keys = [0u8; 6];
    let mut n = 0;
    for &k in body { if k > 3 && n < 6 { keys[n] = k; n += 1; } }
    Some(Input::Keys { modifiers: r[0], keys })
}

/// Boot mouse report: buttons, dx, dy and (often) a wheel byte.
pub fn boot_mouse(r: &[u8]) -> Option<Input> {
    if r.len() < 3 { return None; }
    Some(Input::Pointer { buttons: r[0] & 7, dx: r[1] as i8 as i16, dy: r[2] as i8 as i16, wheel: r.get(3).map(|&w| w as i8).unwrap_or(0) })
}

/// One field: bit offset and width inside its report (after the report id), sign.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Field { pub bit: u16, pub size: u8, pub signed: bool }

impl Field {
    pub fn present(&self) -> bool { self.size != 0 }
    pub fn read(&self, r: &[u8]) -> Option<i32> {
        if self.size == 0 || self.size > 32 || (self.bit as usize + self.size as usize) > r.len() * 8 { return None; }
        let mut v: u64 = 0;
        for i in 0..self.size as usize {
            let at = self.bit as usize + i;
            if r[at / 8] >> (at % 8) & 1 != 0 { v |= 1 << i; }
        }
        Some(if self.signed && v >> (self.size - 1) & 1 != 0 { (v | !0u64 << self.size) as i64 as i32 } else { v as i32 })
    }
}

/// Where a keyboard's or a mouse's data sits in one report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    /// The report id (0 when the device uses none).
    pub id: u8,
    /// Keyboard: eight modifier bits (usages 0xE0..0xE7) and the key array.
    pub modifiers: Field,
    pub keys: Field,
    pub key_count: u8,
    /// Mouse: buttons (one bit each, up to 8), X, Y, wheel; relative only.
    pub buttons: Field,
    pub x: Field,
    pub y: Field,
    pub wheel: Field,
}

impl Layout {
    pub fn keyboard(&self) -> bool { self.keys.present() || self.modifiers.present() }
    pub fn pointer(&self) -> bool { self.x.present() && self.y.present() }

    /// Decode one report body (without its id byte).
    pub fn decode(&self, r: &[u8]) -> Option<Input> {
        if self.keyboard() {
            let modifiers = self.modifiers.read(r).unwrap_or(0) as u8;
            let mut keys = [0u8; 6];
            let mut n = 0;
            for i in 0..self.key_count as u16 {
                let f = Field { bit: self.keys.bit + i * self.keys.size as u16, size: self.keys.size, signed: false };
                let k = f.read(r)? as u32;
                if k == 1 { return None; } // rollover
                if k > 3 && k < 256 && n < 6 { keys[n] = k as u8; n += 1; }
            }
            return Some(Input::Keys { modifiers, keys });
        }
        if self.pointer() {
            let clamp = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            return Some(Input::Pointer {
                buttons: self.buttons.read(r).unwrap_or(0) as u8 & 7,
                dx: clamp(self.x.read(r)?),
                dy: clamp(self.y.read(r)?),
                wheel: self.wheel.read(r).unwrap_or(0).clamp(-127, 127) as i8,
            });
        }
        None
    }
}

/// The keyboard and mouse reports a report map describes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportMap { pub layouts: Vec<Layout> }

impl ReportMap {
    /// Read a report descriptor (HID 1.11, 6.2.2). Unknown items are skipped; absolute
    /// pointers (tablets) are left out, as they are not keyboards or mice.
    pub fn parse(desc: &[u8]) -> ReportMap {
        #[derive(Clone, Copy, Default)]
        struct Global { page: u32, min: i32, size: u32, count: u32, id: u8 }
        let mut g = Global::default();
        let mut stack: Vec<Global> = Vec::new();
        let mut usages: Vec<u32> = Vec::new();
        let (mut umin, mut umax) = (None::<u32>, None::<u32>);
        let mut offsets = [0u16; 256];
        let mut out: Vec<Layout> = Vec::new();
        let mut i = 0;
        while i < desc.len() {
            let prefix = desc[i];
            if prefix == 0xFE { i += 3 + *desc.get(i + 1).unwrap_or(&0) as usize; continue; }
            let len = match prefix & 3 { 3 => 4, n => n as usize };
            let Some(data) = desc.get(i + 1..i + 1 + len) else { break };
            let raw = data.iter().rev().fold(0u32, |a, b| a << 8 | *b as u32);
            let signed = match len { 1 => raw as u8 as i8 as i32, 2 => raw as u16 as i16 as i32, _ => raw as i32 };
            let page = g.page;
            let usage = move |raw: u32| if len == 4 { raw } else { page << 16 | raw };
            i += 1 + len;
            match (prefix >> 2 & 3, prefix >> 4) {
                (1, 0) => g.page = raw,
                (1, 1) => g.min = signed,
                (1, 7) => g.size = raw,
                (1, 8) => g.id = raw as u8,
                (1, 9) => g.count = raw,
                (1, 10) => stack.push(g),
                (1, 11) => { if let Some(s) = stack.pop() { g = s; } }
                (2, 0) => usages.push(usage(raw)),
                (2, 1) => umin = Some(usage(raw)),
                (2, 2) => umax = Some(usage(raw)),
                (0, 8) => { // Input
                    let (constant, variable, relative) = (raw & 1 != 0, raw & 2 != 0, raw & 4 != 0);
                    let base = offsets[g.id as usize];
                    let at = match out.iter().position(|l| l.id == g.id) {
                        Some(at) => at,
                        None => { out.push(Layout { id: g.id, ..Layout::default() }); out.len() - 1 }
                    };
                    let l = &mut out[at];
                    let field = |n: u32| Field { bit: base + (n * g.size) as u16, size: g.size as u8, signed: g.min < 0 };
                    let usage_of = |n: u32| match (umin, umax) {
                        (Some(a), Some(b)) => a.checked_add(n).filter(|u| *u <= b),
                        _ => usages.get((n as usize).min(usages.len().saturating_sub(1))).copied(),
                    };
                    if !constant && !variable && (g.page == 7 || umin.is_some_and(|u| u >> 16 == 7)) {
                        // The key array: each slot holds a usage id.
                        l.keys = field(0);
                        l.key_count = g.count.min(16) as u8;
                    } else if !constant && variable {
                        for n in 0..g.count {
                            let Some(u) = usage_of(n) else { break };
                            match u {
                                0x0007_00E0 if g.size == 1 => l.modifiers = Field { bit: field(n).bit, size: 8, signed: false },
                                0x0007_00E1..=0x0007_00E7 => {}
                                0x0009_0001 if g.size == 1 => {
                                    let span = match (umin, umax) { (Some(a), Some(b)) => (b - a + 1).min(g.count), _ => 1 };
                                    l.buttons = Field { bit: field(n).bit, size: span.min(8) as u8, signed: false };
                                }
                                0x0001_0030 if relative => l.x = field(n),
                                0x0001_0031 if relative => l.y = field(n),
                                0x0001_0038 => l.wheel = field(n),
                                _ => {}
                            }
                        }
                    }
                    offsets[g.id as usize] = base.saturating_add((g.size * g.count) as u16);
                }
                _ => {}
            }
            if prefix >> 2 & 3 == 0 { usages.clear(); umin = None; umax = None; }
        }
        out.retain(|l| l.keyboard() || l.pointer());
        ReportMap { layouts: out }
    }

    /// Decode a report: `id` is its report id (0 when the map has none), `body` the bytes after it.
    pub fn decode(&self, id: u8, body: &[u8]) -> Option<Input> {
        self.layouts.iter().find(|l| l.id == id).and_then(|l| l.decode(body))
    }
}
