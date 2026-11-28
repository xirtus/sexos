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
                // The descriptor comes from the device: bound the fields walked (a report
                // holds at most 64 Ki bits) and saturate offsets. Offsets keep the declared
                // width; a field wider than 32 bits is never read (`Field::read`).
                let walk = count.min(u16::MAX as u32 / size.max(1));
                if !constant && variable && (found.is_none() || cur.report_id == id) {
                    for n in 0..walk {
                        let usage = if range {
                            let u = umin.saturating_add(n);
                            if u > umax { break; } u
                        } else if nusages == 0 { 0 } else { usages[(n as usize).min(nusages - 1)] };
                        let field = Field { bit: base.saturating_add((n * size) as u16), size: size.min(255) as u8,
                            signed: lmin < 0, min: lmin, max: lmax };
                        match usage {
                            0x0009_0001 => {
                                if !cur.buttons.present() || cur.report_id != id {
                                    let buttons = if range { umax.saturating_sub(umin).saturating_add(1).min(count).min(8) } else { 1 };
                                    cur.buttons = Field { bit: field.bit, size: buttons.saturating_mul(size).min(32) as u8, signed: false, min: 0, max: 1 };
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
                offsets[id as usize] = base.saturating_add(size.saturating_mul(count).min(u16::MAX as u32) as u16);
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

// ── Media buttons ─────────────────────────────────────────────────────
// A headset's volume, mute and call buttons, a remote's player keys, and a keyboard's
// media keys come on the Consumer (0x0C) and Telephony (0x0B) pages. Each becomes the
// keyboard usage sexinput already gives a key code: Mute 0x7F, Volume Up 0x80,
// Volume Down 0x81, the player keys 0xE8-0xEB (Play/Pause, Stop, Previous, Next),
// and Microphone Mute 0xEC (Telephony Phone Mute 0x0B/0x2F, Consumer 0x0C/0x02F0, 0x0C/0x0109).

/// The keys, one bit each in a held mask: Mute, Volume Up, Volume Down, Play/Pause,
/// Stop, Previous, Next, Microphone Mute.
pub const MEDIA_USAGES: [u8; 8] = [0x7F, 0x80, 0x81, 0xE8, 0xE9, 0xEA, 0xEB, 0xEC];

/// The key bit (into [`MEDIA_USAGES`]) of a full usage (page << 16 | id), if it has one.
/// A headset's hook switch answers and hangs up, which is Play/Pause everywhere else.
pub fn media_key(usage: u32) -> Option<u8> {
    Some(match usage {
        0x000C_00E2 => 0,
        0x000C_00E9 => 1,
        0x000C_00EA => 2,
        0x000C_00CD | 0x000C_00B0 | 0x000C_00B1 | 0x000B_0020 => 3,
        0x000C_00B7 => 4,
        0x000C_00B6 => 5,
        0x000C_00B5 => 6,
        0x000B_002F | 0x000C_02F0 | 0x000C_0109 => 7,
        _ => return None,
    })
}

/// One field that can press a key: a one-bit (variable) button, or an array slot
/// whose value names the usage (`umin` + value - `lmin`, up to `umax`).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct MediaField {
    pub report_id: u8,
    pub bit: u16,
    pub size: u8,
    /// Variable: the key bit it presses. Array: unused.
    pub key: u8,
    pub array: bool,
    pub umin: u32,
    pub umax: u32,
    pub lmin: i32,
}

pub const MEDIA_FIELDS: usize = 12;

/// Where the media buttons sit in a device's reports.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct MediaLayout { pub fields: [MediaField; MEDIA_FIELDS], pub n: u8, pub ids: bool }

impl MediaLayout {
    /// Which key bits a field can press.
    fn reach(f: &MediaField) -> u8 {
        if !f.array { return 1 << f.key; }
        let mut m = 0u8;
        // Usage ranges in practice are small; look at most at the first 1 Ki.
        for u in f.umin..=f.umax.min(f.umin.saturating_add(1024)) { if let Some(k) = media_key(u) { m |= 1 << k; } }
        m
    }
    /// The held mask after `report` (with its report ID byte when the device uses
    /// IDs), from the one before: keys this report cannot press stay as they were.
    /// None when the report carries none of the buttons.
    pub fn decode(&self, report: &[u8], held: u8) -> Option<u8> {
        let (id, body) = if self.ids { (*report.first()?, report.get(1..)?) } else { (0, report) };
        let mut reach = 0u8;
        let mut pressed = 0u8;
        for f in self.fields[..self.n as usize].iter().filter(|f| f.report_id == id) {
            let field = Field { bit: f.bit, size: f.size, signed: false, min: 0, max: 0 };
            let Some(v) = field.read(body) else { continue };
            reach |= Self::reach(f);
            if !f.array {
                if v != 0 { pressed |= 1 << f.key; }
            } else if v != 0 || f.lmin != 0 {
                let u = f.umin as i64 + v as i64 - f.lmin as i64;
                if u >= f.umin as i64 && u <= f.umax as i64 {
                    if let Some(k) = media_key(u as u32) { pressed |= 1 << k; }
                }
            }
        }
        (reach != 0).then_some(held & !reach | pressed & reach)
    }
    /// The held keys as up to six keyboard usages, one per byte.
    pub fn usages(held: u8) -> u64 {
        let mut keys = 0u64;
        let mut n = 0;
        for (k, u) in MEDIA_USAGES.iter().enumerate() {
            if held & (1 << k) != 0 && n < 6 { keys |= (*u as u64) << (8 * n); n += 1; }
        }
        keys
    }
}

/// Find media buttons in a report descriptor: None when it has none.
pub fn parse_media(desc: &[u8]) -> Option<MediaLayout> {
    let (mut page, mut lmin, mut size, mut count, mut id) = (0u32, 0i32, 0u32, 0u32, 0u8);
    let mut usages = [0u32; 16];
    let mut nusages = 0usize;
    let (mut umin, mut umax, mut range) = (0u32, 0u32, false);
    let mut offsets = [0u16; 256];
    let mut stack = [(0u32, 0i32, 0u32, 0u32, 0u8); 4];
    let mut depth = 0usize;
    let mut out = MediaLayout::default();
    let mut i = 0usize;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE { i += 3 + *desc.get(i + 1)? as usize; continue; }
        let len = match prefix & 3 { 3 => 4, n => n as usize };
        if i + 1 + len > desc.len() { break; }
        let raw = desc[i + 1..i + 1 + len].iter().rev().fold(0u32, |a, b| (a << 8) | *b as u32);
        let signed = match len { 1 => raw as u8 as i8 as i32, 2 => raw as u16 as i16 as i32, _ => raw as i32 };
        let (kind, tag) = ((prefix >> 2) & 3, prefix >> 4);
        i += 1 + len;
        let full = |raw: u32| if len == 4 { raw } else { (page << 16) | raw };
        match (kind, tag) {
            (1, 0) => page = raw,
            (1, 1) => lmin = signed,
            (1, 7) => size = raw,
            (1, 8) => { id = raw as u8; out.ids = true; }
            (1, 9) => count = raw,
            (1, 10) => if depth < stack.len() { stack[depth] = (page, lmin, size, count, id); depth += 1; },
            (1, 11) => if depth > 0 { depth -= 1; (page, lmin, size, count, id) = stack[depth]; },
            (2, 0) => if nusages < usages.len() { usages[nusages] = full(raw); nusages += 1; },
            (2, 1) => { umin = full(raw); range = true; }
            (2, 2) => { umax = full(raw); range = true; }
            (0, 8) => {
                let (constant, variable) = (raw & 1 != 0, raw & 2 != 0);
                let base = offsets[id as usize];
                let walk = count.min(u16::MAX as u32 / size.max(1));
                if !constant && size > 0 && size <= 32 {
                    for n in 0..walk {
                        if out.n as usize == MEDIA_FIELDS { break; }
                        let bit = base.saturating_add((n * size) as u16);
                        let f = if variable {
                            let usage = if range { let u = umin.saturating_add(n); if u > umax { break; } u }
                                else if nusages == 0 { break } else { usages[(n as usize).min(nusages - 1)] };
                            let Some(key) = media_key(usage) else { continue };
                            MediaField { report_id: id, bit, size: size as u8, key, ..MediaField::default() }
                        } else {
                            // An array names its usages by range (a listed set: its first
                            // and last, which is how consumer arrays are written).
                            let (lo, hi) = if range { (umin, umax) } else if nusages > 0 { (usages[0], usages[nusages - 1]) } else { break };
                            MediaField { report_id: id, bit, size: size as u8, array: true, umin: lo, umax: hi.max(lo), lmin, ..MediaField::default() }
                        };
                        if MediaLayout::reach(&f) == 0 { if f.array { break; } continue; }
                        out.fields[out.n as usize] = f;
                        out.n += 1;
                    }
                }
                offsets[id as usize] = base.saturating_add(size.saturating_mul(count).min(u16::MAX as u32) as u16);
            }
            _ => {}
        }
        if kind == 0 { nusages = 0; range = false; umin = 0; umax = 0; }
    }
    (out.n > 0).then_some(out)
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

    #[test]
    fn hostile_counts_and_sizes_are_bounded() {
        // Report Size and Report Count u32::MAX, then X as an input: must end at once.
        let d = [0x05, 0x01, 0x09, 0x30, 0x77, 0xff, 0xff, 0xff, 0xff, 0x97, 0xff, 0xff, 0xff, 0xff, 0x81, 0x02,
            0x09, 0x31, 0x81, 0x02];
        let _ = parse_pointer(&d);
        // Buttons over the whole 32-bit usage range with 255-bit fields.
        let d = [0x05, 0x09, 0x1b, 0x00, 0x00, 0x00, 0x00, 0x2b, 0xff, 0xff, 0xff, 0xff, 0x75, 0xff, 0x95, 0x08, 0x81, 0x02,
            0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x75, 0x08, 0x95, 0x02, 0x81, 0x06];
        if let Some(l) = parse_pointer(&d) { assert!(l.buttons.size <= 8); let _ = l.decode(&[0xff; 64]); }
    }

    // A typical USB headset: report 1 is Consumer Volume Up, Volume Down, Mute and
    // Play/Pause as bits (4 bits of padding); report 2 is Telephony Hook Switch and
    // Phone Mute as bits.
    const HEADSET: &[u8] = &[0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x01, 0x15, 0x00, 0x25, 0x01,
        0x09, 0xe9, 0x09, 0xea, 0x09, 0xe2, 0x09, 0xcd, 0x75, 0x01, 0x95, 0x04, 0x81, 0x02,
        0x95, 0x04, 0x81, 0x01, 0xc0,
        0x05, 0x0b, 0x09, 0x05, 0xa1, 0x01, 0x85, 0x02, 0x09, 0x20, 0x09, 0x2f, 0x75, 0x01, 0x95, 0x02,
        0x81, 0x06, 0x95, 0x06, 0x81, 0x01, 0xc0];
    // A keyboard's media interface: a 16-bit Consumer array, usages 0..0x29c.
    const CONSUMER_ARRAY: &[u8] = &[0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x03, 0x15, 0x00, 0x26, 0x9c, 0x02,
        0x19, 0x00, 0x2a, 0x9c, 0x02, 0x75, 0x10, 0x95, 0x01, 0x81, 0x00, 0xc0];

    #[test] fn headset_buttons() {
        let l = parse_media(HEADSET).unwrap();
        assert!(l.ids);
        assert_eq!(l.n, 6);
        // Volume Up down, then up; Mute held while the hook switch is pressed and let go.
        let h = l.decode(&[1, 0b0001], 0).unwrap();
        assert_eq!(MediaLayout::usages(h), 0x80);
        let h = l.decode(&[1, 0b0100], h).unwrap();
        assert_eq!(MediaLayout::usages(h), 0x7F);
        let h = l.decode(&[2, 0b01], h).unwrap();
        assert_eq!(MediaLayout::usages(h), 0x7F | 0xE8 << 8);
        let h = l.decode(&[2, 0b00], h).unwrap();
        assert_eq!(h, 1);
        let h = l.decode(&[1, 0], h).unwrap();
        assert_eq!(h, 0);
        // Report 9 is not a button report; Phone Mute presses bit 7 (0xEC -> KEY_MIC_MUTE).
        assert_eq!(l.decode(&[9, 0xff], 0), None);
        let h = l.decode(&[2, 0b10], 0).unwrap();
        assert_eq!(h, 1 << 7);
        assert_eq!(MediaLayout::usages(h), 0xEC);
        assert_eq!(l.decode(&[2, 0], h), Some(0));
    }
    #[test] fn composite_pointer_and_media_usages() {
        // Composite HID interface: Report ID 1 mouse, Report ID 2 Consumer array (Volume Up / Mic Mute).
        let composite: &[u8] = &[
            0x05,0x01,0x09,0x02,0xa1,0x01,0x85,0x01,0x09,0x01,0xa1,0x00,0x05,0x09,0x19,0x01,
            0x29,0x03,0x15,0x00,0x25,0x01,0x95,0x03,0x75,0x01,0x81,0x02,0x95,0x01,0x75,0x05,
            0x81,0x01,0x05,0x01,0x09,0x30,0x09,0x31,0x09,0x38,0x15,0x81,0x25,0x7f,0x75,0x08,
            0x95,0x03,0x81,0x06,0xc0,0xc0,
            0x05,0x0c,0x09,0x01,0xa1,0x01,0x85,0x02,0x15,0x00,0x26,0x9c,0x03,0x19,0x00,0x2a,
            0x9c,0x03,0x75,0x10,0x95,0x01,0x81,0x00,0xc0,
        ];
        let ptr = parse_pointer(composite).unwrap();
        let med = parse_media(composite).unwrap();
        assert_eq!(ptr.report_id, 1);
        assert!(med.ids);
        // Mouse report decodes as pointer, ignored by media.
        assert_eq!(med.decode(&[1, 0x01, 2, 3, 0], 0), None);
        let p = ptr.decode(&[1, 0x01, 2, 3, 0]).unwrap();
        assert_eq!((p.buttons, p.x, p.y), (1, 2, 3));
        // Consumer 0x02F0 & 0x0109 Mic Mute array values decode to bit 7 (0xEC), ignored by pointer.
        assert!(ptr.decode(&[2, 0xf0, 0x02]).is_none());
        let h = med.decode(&[2, 0xf0, 0x02], 0).unwrap();
        assert_eq!(MediaLayout::usages(h), 0xEC);
        let h = med.decode(&[2, 0x09, 0x01], 0).unwrap();
        assert_eq!(MediaLayout::usages(h), 0xEC);
    }
    #[test] fn consumer_array_keys() {
        let l = parse_media(CONSUMER_ARRAY).unwrap();
        assert_eq!(l.n, 1);
        assert_eq!(l.decode(&[3, 0xcd, 0x00], 0), Some(1 << 3));
        assert_eq!(l.decode(&[3, 0xb5, 0x00], 1 << 3), Some(1 << 6));
        assert_eq!(l.decode(&[3, 0x00, 0x00], 1 << 6), Some(0));
        assert_eq!(l.decode(&[3, 0x23, 0x02], 0), Some(0), "AC Home is not one of ours");
    }
    #[test] fn pointers_and_hostile_media_descriptors() {
        assert_eq!(parse_media(QEMU_MOUSE), None);
        let d = [0x05, 0x0c, 0x1b, 0x00, 0x00, 0x00, 0x00, 0x2b, 0xff, 0xff, 0xff, 0xff, 0x77, 0xff, 0xff, 0xff, 0xff,
            0x97, 0xff, 0xff, 0xff, 0xff, 0x81, 0x00, 0x09, 0xe9, 0x75, 0x01, 0x95, 0xff, 0x81, 0x02];
        if let Some(l) = parse_media(&d) { let _ = l.decode(&[0xff; 64], 0xff); }
    }

    #[test]
    fn wide_padding_keeps_later_fields_in_place() {
        // 64-bit constant padding, then relative 8-bit X and Y.
        let d = [0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x75, 0x40, 0x95, 0x01, 0x81, 0x01,
            0x09, 0x30, 0x09, 0x31, 0x15, 0x81, 0x25, 0x7f, 0x75, 0x08, 0x95, 0x02, 0x81, 0x06, 0xc0];
        let l = parse_pointer(&d).unwrap();
        let p = l.decode(&[0, 0, 0, 0, 0, 0, 0, 0, 5, 6]).unwrap();
        assert_eq!((p.x, p.y), (5, 6));
    }
}
