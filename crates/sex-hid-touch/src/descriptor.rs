// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! HID report descriptors (HID 1.11 section 6.2.2), as far as a touchpad needs: the input
//! fields of the Precision Touchpad application collection, with their usages, sizes and
//! the finger collection each belongs to.

pub const PAGE_DESKTOP: u16 = 0x01;
pub const PAGE_BUTTON: u16 = 0x09;
pub const PAGE_DIGITIZER: u16 = 0x0D;
pub const USAGE_X: u16 = 0x30;
pub const USAGE_Y: u16 = 0x31;
pub const USAGE_TOUCH_PAD: u16 = 0x05;
pub const USAGE_FINGER: u16 = 0x22;
pub const USAGE_TIP_SWITCH: u16 = 0x42;
pub const USAGE_CONFIDENCE: u16 = 0x47;
pub const USAGE_CONTACT_ID: u16 = 0x51;
pub const USAGE_CONTACT_COUNT: u16 = 0x54;
pub const USAGE_SCAN_TIME: u16 = 0x56;
pub const USAGE_INPUT_MODE: u16 = 0x52;

pub const MAX_FIELDS: usize = 96;
pub const MAX_FINGERS: usize = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Field {
    pub report_id: u8,
    /// Bit offset within the report (after the report id byte, when there is one).
    pub bit: u16,
    pub size: u8,
    pub page: u16,
    pub usage: u16,
    pub logical_min: i32,
    pub logical_max: i32,
    pub physical_max: i32,
    /// Which finger collection (0-based) holds it, or None outside one.
    pub finger: Option<u8>,
    /// Constant (padding) fields carry no value.
    pub constant: bool,
}

/// The input fields of one touchpad, and the feature report that switches it into
/// touchpad mode.
#[derive(Clone, Debug)]
pub struct Layout {
    pub fields: [Field; MAX_FIELDS],
    pub count: usize,
    /// The input report the touchpad's contacts arrive in.
    pub touch_report: Option<u8>,
    /// Feature report id carrying Input Mode (write 3 for touchpad mode).
    pub input_mode_report: Option<u8>,
    pub fingers: u8,
    pub uses_report_ids: bool,
}

impl Layout {
    pub fn fields(&self) -> &[Field] { &self.fields[..self.count] }
    /// The first field of a finger with a usage.
    pub fn finger_field(&self, finger: u8, page: u16, usage: u16) -> Option<&Field> {
        self.fields().iter().find(|f| f.finger == Some(finger) && f.page == page && f.usage == usage && f.report_id == self.touch_report.unwrap_or(0))
    }
    pub fn field(&self, page: u16, usage: u16) -> Option<&Field> {
        self.fields().iter().find(|f| f.page == page && f.usage == usage && f.report_id == self.touch_report.unwrap_or(0))
    }
}

fn signed(v: u32, bytes: usize) -> i32 {
    match bytes { 1 => v as u8 as i8 as i32, 2 => v as u16 as i16 as i32, _ => v as i32 }
}

#[derive(Clone, Copy, Default)]
struct Globals { page: u16, lmin: i32, lmax: i32, pmax: i32, size: u8, count: u16, id: u8 }

pub fn parse(desc: &[u8]) -> Option<Layout> {
    let mut out = Layout { fields: [Field::default(); MAX_FIELDS], count: 0, touch_report: None, input_mode_report: None, fingers: 0, uses_report_ids: false };
    let mut g = Globals::default();
    let mut stack: [Globals; 8] = [Globals::default(); 8];
    let mut sp = 0;
    let mut usages: [(u16, u16); 16] = [(0, 0); 16];
    let mut n_usages = 0;
    let mut usage_range: Option<(u32, u32)> = None;
    // Collection nesting: (is_touchpad_app, finger index)
    let mut depth = 0usize;
    let mut touchpad_depth: Option<usize> = None;
    let mut finger_depth: Option<usize> = None;
    let mut finger: Option<u8> = None;
    let mut bits = [0u16; 256]; // bit offset per input report id
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE { // long item: skip
            let len = *desc.get(i + 1)? as usize;
            i += 3 + len;
            continue;
        }
        let size = [0usize, 1, 2, 4][(prefix & 3) as usize];
        let data = desc.get(i + 1..i + 1 + size)?;
        let mut v = 0u32;
        for (k, b) in data.iter().enumerate() { v |= (*b as u32) << (8 * k); }
        let (kind, tag) = ((prefix >> 2) & 3, prefix >> 4);
        let usage_of = |v: u32, g: &Globals| -> (u16, u16) { if size == 4 { ((v >> 16) as u16, v as u16) } else { (g.page, v as u16) } };
        match (kind, tag) {
            // Main items
            (0, 0x8) | (0, 0xB) => { // Input / Feature
                let input = tag == 0x8;
                let constant = v & 1 != 0;
                if input {
                    let base = &mut bits[g.id as usize];
                    for k in 0..g.count {
                        let (page, usage) = if let Some((lo, hi)) = usage_range {
                            let u = (lo + k as u32).min(hi);
                            (if lo > 0xFFFF { (lo >> 16) as u16 } else { g.page }, u as u16)
                        } else if n_usages > 0 { usages[(k as usize).min(n_usages - 1)] } else { (g.page, 0) };
                        if out.count < MAX_FIELDS && touchpad_depth.is_some() {
                            out.fields[out.count] = Field { report_id: g.id, bit: *base, size: g.size, page, usage,
                                logical_min: g.lmin, logical_max: g.lmax, physical_max: g.pmax, finger, constant };
                            out.count += 1;
                        }
                        *base = base.saturating_add(g.size as u16);
                    }
                    if touchpad_depth.is_some() && out.touch_report.is_none() && !constant { out.touch_report = Some(g.id); }
                } else if usages[..n_usages].iter().any(|&(p, u)| p == PAGE_DIGITIZER && u == USAGE_INPUT_MODE) {
                    out.input_mode_report = Some(g.id);
                }
                n_usages = 0; usage_range = None;
            }
            (0, 0x9) => { n_usages = 0; usage_range = None; } // Output
            (0, 0xA) => { // Collection
                depth += 1;
                let (page, usage) = if n_usages > 0 { usages[0] } else { (g.page, 0) };
                if page == PAGE_DIGITIZER && usage == USAGE_TOUCH_PAD && v == 1 && touchpad_depth.is_none() { touchpad_depth = Some(depth); }
                if touchpad_depth.is_some() && page == PAGE_DIGITIZER && usage == USAGE_FINGER && finger_depth.is_none() {
                    finger_depth = Some(depth);
                    finger = Some(out.fingers);
                    out.fingers = out.fingers.saturating_add(1);
                }
                n_usages = 0; usage_range = None;
            }
            (0, 0xC) => { // End Collection
                if finger_depth == Some(depth) { finger_depth = None; finger = None; }
                if touchpad_depth == Some(depth) { touchpad_depth = None; }
                depth = depth.saturating_sub(1);
                n_usages = 0; usage_range = None;
            }
            // Global items
            (1, 0x0) => g.page = v as u16,
            (1, 0x1) => g.lmin = signed(v, size),
            (1, 0x2) => g.lmax = if g.lmin < 0 { signed(v, size) } else { v as i32 },
            (1, 0x4) => g.pmax = v as i32,
            (1, 0x7) => g.size = v.min(32) as u8,
            (1, 0x8) => { g.id = v as u8; out.uses_report_ids = true; }
            (1, 0x9) => g.count = v.min(256) as u16,
            (1, 0xA) => { if sp < stack.len() { stack[sp] = g; sp += 1; } }
            (1, 0xB) => { if sp > 0 { sp -= 1; g = stack[sp]; } }
            // Local items
            (2, 0x0) => { if n_usages < usages.len() { usages[n_usages] = usage_of(v, &g); n_usages += 1; } }
            (2, 0x1) => usage_range = Some((if size == 4 { v } else { v }, usage_range.map_or(v, |r| r.1))),
            (2, 0x2) => usage_range = Some((usage_range.map_or(0, |r| r.0), v)),
            _ => {}
        }
        i += 1 + size;
    }
    (out.touch_report.is_some() || out.count > 0).then_some(out)
}

/// Reads a field's value from a report (report id byte stripped).
pub fn value(f: &Field, report: &[u8]) -> Option<i32> {
    let (start, size) = (f.bit as usize, f.size as usize);
    if size == 0 || size > 32 || (start + size + 7) / 8 > report.len() { return None; }
    let mut v: u64 = 0;
    for k in 0..(size + (start % 8) + 7) / 8 { v |= (report[start / 8 + k] as u64) << (8 * k); }
    let raw = ((v >> (start % 8)) & ((1u64 << size) - 1)) as u32;
    Some(if f.logical_min < 0 && size < 32 && raw & (1 << (size - 1)) != 0 { (raw | !((1u32 << size) - 1)) as i32 } else { raw as i32 })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A Precision Touchpad descriptor in the shape Microsoft's sample and most I2C pads use:
    /// report 1 = two fingers (tip, confidence, 3-bit contact id, 12-bit X 0..3000 and Y
    /// 0..2000), scan time, contact count, one button; feature report 3 = Input Mode.
    pub fn ptp_descriptor() -> Vec<u8> {
        let mut d = vec![
            0x05, 0x0D,       // Usage Page (Digitizer)
            0x09, 0x05,       // Usage (Touch Pad)
            0xA1, 0x01,       // Collection (Application)
            0x85, 0x01,       //   Report ID (1)
        ];
        for _ in 0..2 {
            d.extend_from_slice(&[
                0x05, 0x0D, 0x09, 0x22, 0xA1, 0x02,       // Finger, Logical collection
                0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x01,
                0x09, 0x47, 0x81, 0x02,                   // Confidence
                0x09, 0x42, 0x81, 0x02,                   // Tip switch
                0x95, 0x01, 0x75, 0x03, 0x25, 0x05, 0x09, 0x51, 0x81, 0x02, // Contact id (3 bits)
                0x75, 0x03, 0x95, 0x01, 0x81, 0x03,       // 3 bits padding
                0x05, 0x01, 0x15, 0x00, 0x26, 0xB8, 0x0B, 0x75, 0x0C, 0x95, 0x01, // X 0..3000, 12 bits
                0x46, 0x9C, 0x04, 0x09, 0x30, 0x81, 0x02,  //   physical max 1180 (0.1 mm)
                0x26, 0xD0, 0x07, 0x46, 0x16, 0x03, 0x09, 0x31, 0x81, 0x02, // Y 0..2000, physical 790
                0xC0,                                     // End finger
            ]);
        }
        d.extend_from_slice(&[
            0x05, 0x0D, 0x55, 0x0C, 0x66, 0x01, 0x10, 0x47, 0xFF, 0xFF, 0x00, 0x00, 0x27, 0xFF, 0xFF, 0x00, 0x00,
            0x75, 0x10, 0x95, 0x01, 0x09, 0x56, 0x81, 0x02, // Scan time
            0x09, 0x54, 0x25, 0x7F, 0x95, 0x01, 0x75, 0x08, 0x81, 0x02, // Contact count
            0x05, 0x09, 0x09, 0x01, 0x25, 0x01, 0x75, 0x01, 0x95, 0x01, 0x81, 0x02, // Button 1
            0x95, 0x07, 0x81, 0x03,                         // padding
            0xC0,                                           // End application
            0x05, 0x0D, 0x09, 0x0E, 0xA1, 0x01,             // Device configuration
            0x85, 0x03, 0x09, 0x22, 0xA1, 0x02,
            0x09, 0x52, 0x15, 0x00, 0x25, 0x0A, 0x75, 0x08, 0x95, 0x01, 0xB1, 0x02, // Input Mode feature
            0xC0, 0xC0,
        ]);
        d
    }

    #[test]
    fn finds_fingers_fields_and_input_mode() {
        let l = parse(&ptp_descriptor()).unwrap();
        assert_eq!(l.touch_report, Some(1));
        assert_eq!(l.input_mode_report, Some(3));
        assert_eq!(l.fingers, 2);
        let x1 = l.finger_field(1, PAGE_DESKTOP, USAGE_X).unwrap();
        // Finger 0 is 1+1+3+3+12+12 = 32 bits, so finger 1's X starts at 32 + 8.
        assert_eq!((x1.bit, x1.size, x1.logical_max, x1.physical_max), (40, 12, 3000, 1180));
        let y0 = l.finger_field(0, PAGE_DESKTOP, USAGE_Y).unwrap();
        assert_eq!((y0.bit, y0.logical_max), (20, 2000));
        let cc = l.field(PAGE_DIGITIZER, USAGE_CONTACT_COUNT).unwrap();
        assert_eq!((cc.bit, cc.size, cc.finger), (80, 8, None));
        assert_eq!(l.field(PAGE_BUTTON, 1).unwrap().bit, 88);
    }

    #[test]
    fn values_unpack_across_bytes_and_sign() {
        let f = Field { bit: 4, size: 12, logical_max: 4095, ..Default::default() };
        assert_eq!(value(&f, &[0xA0, 0xBC]), Some(0xBCA));
        let s = Field { bit: 0, size: 8, logical_min: -127, logical_max: 127, ..Default::default() };
        assert_eq!(value(&s, &[0xFE]), Some(-2));
        assert_eq!(value(&f, &[0xA0]), None);
    }

    #[test]
    fn hostile_descriptors_do_not_panic() {
        let d = ptp_descriptor();
        for cut in 0..d.len() { let _ = parse(&d[..cut]); }
        let _ = parse(&[0xFE, 0xFF]);
        let _ = parse(&[0xC0; 64]);
        let _ = parse(&[0xA1u8, 0x01].repeat(32));
        let mut many = vec![0x05, 0x0D, 0x09, 0x05, 0xA1, 0x01, 0x75, 0x20, 0x96, 0xFF, 0x00];
        for _ in 0..40 { many.extend_from_slice(&[0x81, 0x02]); }
        let l = parse(&many).unwrap();
        assert_eq!(l.count, MAX_FIELDS);
    }
}
