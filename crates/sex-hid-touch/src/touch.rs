// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! One input report as a frame of contacts.
use crate::descriptor::{self as d, Layout};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    pub id: u8,
    pub x: i32,
    pub y: i32,
    /// Finger on the surface (tip switch), and not a palm (confidence, when reported).
    pub down: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub contacts: [Contact; d::MAX_FINGERS],
    pub count: usize,
    /// The pad's own click (clickpads press the whole surface).
    pub button: bool,
}
impl Frame {
    pub fn down(&self) -> impl Iterator<Item = &Contact> { self.contacts[..self.count].iter().filter(|c| c.down) }
    pub fn fingers(&self) -> usize { self.down().count() }
}

/// A relative pointer report from a touchpad's fallback Mouse collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseReport {
    pub dx: i32,
    pub dy: i32,
    pub wheel: i32,
    pub buttons: u8,
}

/// Logical size of the surface (X, Y maxima) for scaling.
pub fn extent(l: &Layout) -> (i32, i32) {
    let x = l.finger_field(0, d::PAGE_DESKTOP, d::USAGE_X).map_or(4095, |f| f.logical_max.max(1));
    let y = l.finger_field(0, d::PAGE_DESKTOP, d::USAGE_Y).map_or(4095, |f| f.logical_max.max(1));
    (x, y)
}

/// Decodes a report (as read, report id first when the layout uses ids). Hybrid pads
/// send one finger per report: then `count` is the slots this report carries.
pub fn decode(l: &Layout, raw: &[u8]) -> Option<Frame> {
    let report = if l.uses_report_ids {
        let first = *raw.first()?;
        if Some(first) != l.touch_report { return None; }
        &raw[1..]
    } else {
        if l.touch_report.is_none() { return None; }
        raw
    };
    let mut f = Frame::default();
    let reported = l.field(d::PAGE_DIGITIZER, d::USAGE_CONTACT_COUNT).and_then(|c| d::value(c, report)).map(|v| v.max(0) as usize);
    let slots = (l.fingers as usize).min(d::MAX_FINGERS);
    let mut n = reported.map_or(slots, |c| c.min(slots));
    // Hybrid single-slot pads send Contact Count = 0 on continuation packets while tip is down.
    if n == 0 && slots == 1 {
        let tip0 = l.finger_field(0, d::PAGE_DIGITIZER, d::USAGE_TIP_SWITCH)
            .and_then(|fd| d::value(fd, report)).unwrap_or(0) != 0;
        if tip0 { n = 1; }
    }
    for i in 0..n {
        let i8 = i as u8;
        let get = |page, usage| l.finger_field(i8, page, usage).and_then(|fd| d::value(fd, report));
        let tip = get(d::PAGE_DIGITIZER, d::USAGE_TIP_SWITCH).unwrap_or(0) != 0;
        let confident = get(d::PAGE_DIGITIZER, d::USAGE_CONFIDENCE).map_or(true, |v| v != 0);
        f.contacts[i] = Contact {
            id: get(d::PAGE_DIGITIZER, d::USAGE_CONTACT_ID).unwrap_or(i as i32) as u8,
            x: get(d::PAGE_DESKTOP, d::USAGE_X)?,
            y: get(d::PAGE_DESKTOP, d::USAGE_Y)?,
            down: tip && confident,
        };
    }
    f.count = n;
    f.button = l.field(d::PAGE_BUTTON, 1).and_then(|b| d::value(b, report)).unwrap_or(0) != 0;
    Some(f)
}

/// Decodes a Mouse-collection report when the touchpad is in mouse emulation mode.
pub fn decode_mouse(l: &Layout, raw: &[u8]) -> Option<MouseReport> {
    let report = if l.uses_report_ids {
        let first = *raw.first()?;
        if Some(first) != l.mouse_report { return None; }
        &raw[1..]
    } else {
        if l.mouse_report.is_none() { return None; }
        raw
    };
    let dx = l.mouse_field(d::PAGE_DESKTOP, d::USAGE_X).and_then(|f| d::value(f, report)).unwrap_or(0);
    let dy = l.mouse_field(d::PAGE_DESKTOP, d::USAGE_Y).and_then(|f| d::value(f, report)).unwrap_or(0);
    let wheel = l.mouse_field(d::PAGE_DESKTOP, d::USAGE_WHEEL).and_then(|f| d::value(f, report)).unwrap_or(0);
    let mut buttons = 0u8;
    for b in 0..3u8 {
        if l.mouse_field(d::PAGE_BUTTON, (b + 1) as u16).and_then(|f| d::value(f, report)).unwrap_or(0) != 0 {
            buttons |= 1 << b;
        }
    }
    Some(MouseReport { dx, dy, wheel, buttons })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::descriptor::{parse, tests::ptp_descriptor};

    /// Builds report 1 of the test descriptor.
    pub fn report(fingers: &[(u8, bool, i32, i32)], button: bool) -> Vec<u8> {
        let mut bits = [0u8; 12];
        let mut put = |at: usize, size: usize, v: u32| for k in 0..size { if v >> k & 1 != 0 { bits[(at + k) / 8] |= 1 << ((at + k) % 8); } };
        for (i, &(id, tip, x, y)) in fingers.iter().enumerate().take(2) {
            let b = i * 32;
            put(b, 1, 1);
            put(b + 1, 1, tip as u32);
            put(b + 2, 3, id as u32);
            put(b + 8, 12, x as u32);
            put(b + 20, 12, y as u32);
        }
        put(80, 8, fingers.len() as u32);
        put(88, 1, button as u32);
        let mut r = vec![1u8];
        r.extend_from_slice(&bits);
        r
    }

    #[test]
    fn decodes_two_fingers_and_button() {
        let l = parse(&ptp_descriptor()).unwrap();
        let f = decode(&l, &report(&[(1, true, 1500, 1000), (2, true, 300, 1999)], true)).unwrap();
        assert_eq!(f.count, 2);
        assert_eq!(f.contacts[0], Contact { id: 1, x: 1500, y: 1000, down: true });
        assert_eq!(f.contacts[1], Contact { id: 2, x: 300, y: 1999, down: true });
        assert!(f.button);
        assert_eq!(f.fingers(), 2);
        let f = decode(&l, &report(&[(1, false, 1500, 1000)], false)).unwrap();
        assert_eq!((f.count, f.fingers(), f.button), (1, 0, false));
        assert_eq!(extent(&l), (3000, 2000));
        // Another report id is not ours.
        assert_eq!(decode(&l, &[2, 0, 0]), None);
    }

    #[test]
    fn empty_report_with_report_ids_is_ignored() {
        let l = parse(&ptp_descriptor()).unwrap();
        assert!(l.uses_report_ids);
        assert!(decode(&l, &[]).is_none());
    }
}
