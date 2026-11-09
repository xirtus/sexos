// SPDX-License-Identifier: GPL-3.0-only
//! HID report descriptors: where a pointer's buttons, X, Y and wheel sit in its reports.
//!
//! Real mice don't all send the 3-byte boot report: most add a wheel, many use 12- or
//! 16-bit motion, side buttons and report IDs, and tablets send absolute positions with
//! their own ranges. This reads the descriptor the device gives and finds the fields a
//! pointer needs. Keyboards are read in the boot protocol, which every keyboard has.

/// One field inside a report: its bit position, width, and whether it is signed.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Field { pub bit: u16, pub size: u8, pub signed: bool, pub min: i32, pub max: i32 }

impl Field {
    pub fn present(&self) -> bool { self.size != 0 }
    /// The value in `report` (which starts after any report ID byte).
    pub fn read(&self, report: &[u8]) -> Option<i32> {
        if self.size == 0 || self.size > 32 { return None; }
        let end = self.bit as usize + self.size as usize;
        if end > report.len() * 8 { return None; }
        let mut value: u64 = 0;
        for i in 0..self.size as usize {
            let at = self.bit as usize + i;
            if report[at / 8] & (1 << (at % 8)) != 0 { value |= 1 << i; }
        }
        Some(if self.signed && self.size < 64 && value & (1 << (self.size - 1)) != 0 {
            (value | (!0u64 << self.size)) as i64 as i32
        } else { value as i64 as i32 })
    }
}

/// The fields of one pointer report.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct PointerLayout {
    /// Report ID that carries the pointer, or 0 when the device uses none.
    pub report_id: u8,
    pub buttons: Field,
    pub x: Field,
    pub y: Field,
    pub wheel: Field,
    pub absolute: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pointer { pub buttons: u8, pub x: i32, pub y: i32, pub wheel: i32, pub absolute: bool }

impl PointerLayout {
    /// Decode one received report (including its report ID byte if the device has IDs).
    pub fn decode(&self, data: &[u8]) -> Option<Pointer> {
        let body = if self.report_id != 0 {
            if data.first() != Some(&self.report_id) { return None; }
            &data[1..]
        } else { data };
        let x = self.x.read(body)?;
        let y = self.y.read(body)?;
        let buttons = if self.buttons.present() { self.buttons.read(body).unwrap_or(0) as u32 } else { 0 };
        let wheel = if self.wheel.present() { self.wheel.read(body).unwrap_or(0) } else { 0 };
        Some(Pointer { buttons: (buttons & 0xff) as u8, x, y, wheel, absolute: self.absolute })
    }
    /// Scale an absolute field to 0..=32767, the range Input expects.
    pub fn scale(field: &Field, value: i32) -> u16 {
        let (min, max) = (field.min as i64, field.max as i64);
        if max <= min { return 0; }
        let v = (value as i64).clamp(min, max) - min;
        ((v * 32767) / (max - min)) as u16
    }
}

/// Find a pointer (mouse, tablet, touch screen in mouse mode) in a report descriptor.
pub fn parse_pointer(desc: &[u8]) -> Option<PointerLayout> {
    // Global state.
    let (mut page, mut lmin, mut lmax, mut size, mut count, mut id) = (0u32, 0i32, 0i32, 0u32, 0u32, 0u8);
    // Local state: listed usages, or a min..max range.
    let mut usages = [0u32; 16];
    let mut nusages = 0usize;
    let (mut umin, mut umax, mut range) = (0u32, 0u32, false);
    // Bit offsets per report ID (IDs are 0..=255).
    let mut offsets = [0u16; 256];
    let mut stack = [(0u32, 0i32, 0i32, 0u32, 0u32, 0u8); 4];
    let mut depth = 0usize;
    let mut found: Option<PointerLayout> = None;
    let mut cur = PointerLayout::default();
    let mut i = 0usize;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE { // long item
            let len = *desc.get(i + 1)? as usize;
            i += 3 + len;
            continue;
        }
        let len = match prefix & 3 { 3 => 4, n => n as usize };
        if i + 1 + len > desc.len() { break; }
        let raw = desc[i + 1..i + 1 + len].iter().rev().fold(0u32, |a, b| (a << 8) | *b as u32);
        let signed = match len { 1 => raw as u8 as i8 as i32, 2 => raw as u16 as i16 as i32, _ => raw as i32 };
        let kind = (prefix >> 2) & 3;
        let tag = prefix >> 4;
        i += 1 + len;
        match (kind, tag) {
            (1, 0) => page = raw,
            (1, 1) => lmin = signed,
            (1, 2) => lmax = if lmin >= 0 && len < 4 { raw as i32 } else { signed },
            (1, 7) => size = raw,
            (1, 8) => id = raw as u8,
            (1, 9) => count = raw,
            (1, 10) => if depth < stack.len() { stack[depth] = (page, lmin, lmax, size, count, id); depth += 1; },
            (1, 11) => if depth > 0 { depth -= 1; (page, lmin, lmax, size, count, id) = stack[depth]; },
            (2, 0) => {
                let usage = if len == 4 { raw } else { (page << 16) | raw };
                if nusages < usages.len() { usages[nusages] = usage; nusages += 1; }
            }
            (2, 1) => { umin = if len == 4 { raw } else { (page << 16) | raw }; range = true; }
            (2, 2) => { umax = if len == 4 { raw } else { (page << 16) | raw }; range = true; }
            (0, 8) => { // Input
                let constant = raw & 1 != 0;
                let variable = raw & 2 != 0;
                let relative = raw & 4 != 0;
                let base = offsets[id as usize];
                if !constant && variable && (found.is_none() || cur.report_id == id) {
                    for n in 0..count {
                        let usage = if range {
                            let u = umin.saturating_add(n);
                            if u > umax { break; } u
                        } else if nusages == 0 { 0 } else { usages[(n as usize).min(nusages - 1)] };
                        let field = Field { bit: base + (n * size) as u16, size: size as u8,
                            signed: lmin < 0, min: lmin, max: lmax };
                        match usage {
                            0x0009_0001 => {
                                if !cur.buttons.present() || cur.report_id != id {
                                    let buttons = if range { (umax - umin + 1).min(count).min(8) } else { 1 };
                                    cur.buttons = Field { bit: field.bit, size: (buttons as u8).min(8) * size as u8, signed: false, min: 0, max: 1 };
                                }
                            }
                            0x0001_0030 => { cur.x = field; cur.report_id = id; cur.absolute = !relative; }
                            0x0001_0031 => { cur.y = field; }
                            0x0001_0038 => { cur.wheel = field; }
                            _ => {}
                        }
                    }
                    if cur.x.present() && cur.y.present() && found.is_none() {
                        found = Some(cur);
                    }
                    if let Some(f) = found.as_mut() { if f.report_id == cur.report_id { *f = cur; } }
                }
                offsets[id as usize] = base.saturating_add((size * count) as u16);
            }
            (0, 9) | (0, 11) => { // Output, Feature: their bits are in other reports.
            }
            _ => {}
        }
        if kind == 0 { nusages = 0; range = false; umin = 0; umax = 0; }
    }
    let mut layout = found?;
    // Size of the buttons field covers one bit per button.
    if layout.buttons.present() { layout.buttons.size = layout.buttons.size.min(8); }
    Some(layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    // QEMU usb-mouse: 3 buttons, X/Y/wheel relative 8-bit.
    const QEMU_MOUSE: &[u8] = &[0x05,0x01,0x09,0x02,0xa1,0x01,0x09,0x01,0xa1,0x00,0x05,0x09,0x19,0x01,0x29,0x03,
        0x15,0x00,0x25,0x01,0x95,0x03,0x75,0x01,0x81,0x02,0x95,0x01,0x75,0x05,0x81,0x01,0x05,0x01,0x09,0x30,
        0x09,0x31,0x09,0x38,0x15,0x81,0x25,0x7f,0x75,0x08,0x95,0x03,0x81,0x06,0xc0,0xc0];
    // QEMU usb-tablet: 3 buttons, absolute 16-bit X/Y 0..0x7fff, relative wheel.
    const QEMU_TABLET: &[u8] = &[0x05,0x01,0x09,0x02,0xa1,0x01,0x09,0x01,0xa1,0x00,0x05,0x09,0x19,0x01,0x29,0x03,
        0x15,0x00,0x25,0x01,0x95,0x03,0x75,0x01,0x81,0x02,0x95,0x01,0x75,0x05,0x81,0x01,0x05,0x01,0x09,0x30,
        0x09,0x31,0x15,0x00,0x26,0xff,0x7f,0x35,0x00,0x46,0xff,0x7f,0x75,0x10,0x95,0x02,0x81,0x02,0x05,0x01,
        0x09,0x38,0x15,0x81,0x25,0x7f,0x35,0x00,0x45,0x00,0x75,0x08,0x95,0x01,0x81,0x06,0xc0,0xc0];
    // A gaming mouse with report ID 1, 5 buttons, 16-bit X/Y.
    const ID_MOUSE: &[u8] = &[0x05,0x01,0x09,0x02,0xa1,0x01,0x85,0x01,0x09,0x01,0xa1,0x00,0x05,0x09,0x19,0x01,
        0x29,0x05,0x15,0x00,0x25,0x01,0x95,0x05,0x75,0x01,0x81,0x02,0x95,0x01,0x75,0x03,0x81,0x01,0x05,0x01,
        0x09,0x30,0x09,0x31,0x16,0x01,0x80,0x26,0xff,0x7f,0x75,0x10,0x95,0x02,0x81,0x06,0x09,0x38,0x15,0x81,
        0x25,0x7f,0x75,0x08,0x95,0x01,0x81,0x06,0xc0,0xc0];

    #[test] fn qemu_mouse() {
        let l = parse_pointer(QEMU_MOUSE).unwrap();
        assert!(!l.absolute); assert_eq!(l.report_id, 0);
        let p = l.decode(&[0x05, 0xfe, 0x03, 0xff]).unwrap();
        assert_eq!((p.buttons, p.x, p.y, p.wheel), (5, -2, 3, -1));
    }
    #[test] fn qemu_tablet() {
        let l = parse_pointer(QEMU_TABLET).unwrap();
        assert!(l.absolute);
        let p = l.decode(&[0x01, 0x34, 0x12, 0xff, 0x7f, 0x01]).unwrap();
        assert_eq!((p.buttons, p.x, p.y, p.wheel), (1, 0x1234, 0x7fff, 1));
        assert_eq!(PointerLayout::scale(&l.x, 0x7fff), 32767);
    }
    #[test] fn report_id_mouse() {
        let l = parse_pointer(ID_MOUSE).unwrap();
        assert_eq!(l.report_id, 1);
        let p = l.decode(&[0x01, 0x11, 0x00, 0x01, 0xfe, 0xff, 0x02]).unwrap();
        assert_eq!((p.buttons, p.x, p.y, p.wheel, p.absolute), (0x11, 256, -2, 2, false));
        assert!(l.decode(&[0x02, 0, 0, 0, 0, 0, 0]).is_none());
    }
}
