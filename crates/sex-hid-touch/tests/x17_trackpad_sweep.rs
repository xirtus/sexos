// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Sweep Test 3: I2C-HID & Precision Touchpad / Mouse-collection protocol sweep
//! covering Dell/Synaptics/ELAN/Goodix-style descriptors, slow/missing reset sentinels,
//! Mouse fallback when SET_REPORT is rejected, 5-finger heavy-padding descriptors,
//! hybrid 1-slot continuation packets, malformed packets, sleep/wake re-init, and
//! Pace IRQ-to-poll fallback.
use sex_hid_touch::descriptor::{self, PAGE_BUTTON, PAGE_DESKTOP, PAGE_DIGITIZER, USAGE_CONTACT_COUNT, USAGE_X, USAGE_Y};
use sex_hid_touch::gesture::{Config, Event, Recognizer};
use sex_hid_touch::i2c_hid::{self as hid, HidDescriptor};
use sex_hid_touch::pace::{Mode, Next, Pace, Read, CHECK_TICKS, SILENT_AFTER, STUCK_AFTER};
use sex_hid_touch::session::{Bus, ProbeError, Report, Touchpad, MAX_REPORT_DESCRIPTOR};
use sex_hid_touch::touch::{self, MouseReport};
use std::collections::VecDeque;

/// Realistic 5-finger Dell/Synaptics/ELAN Precision Touchpad descriptor WITH:
/// - Mouse Application Collection (Report ID 1: 3 buttons, 5-bit pad, 8-bit rel X/Y, 8-bit wheel)
/// - 5-Finger Touch Pad Collection (Report ID 4: heavy 1-bit constant padding per finger that
///   would overflow a 96-field parser if padding were stored, plus Scan Time, Contact Count, Button 1)
/// - Device Configuration Collection (Feature Report ID 5: 2-byte Input Mode + Surface/Button switch)
fn dell_5finger_ptp_with_mouse_descriptor() -> Vec<u8> {
    let mut d = vec![
        // Collection 1: Mouse (Report ID 1)
        0x05, 0x01,       // Usage Page (Generic Desktop)
        0x09, 0x02,       // Usage (Mouse)
        0xA1, 0x01,       // Collection (Application)
        0x85, 0x01,       //   Report ID (1)
        0x09, 0x01,       //   Usage (Pointer)
        0xA1, 0x00,       //   Collection (Physical)
        0x05, 0x09,       //     Usage Page (Button)
        0x19, 0x01,       //     Usage Minimum (1)
        0x29, 0x03,       //     Usage Maximum (3)
        0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x03, 0x81, 0x02, // 3 buttons
        0x75, 0x01, 0x95, 0x05, 0x81, 0x03, // 5 bits constant padding
        0x05, 0x01,       //     Usage Page (Generic Desktop)
        0x09, 0x30, 0x09, 0x31, 0x09, 0x38, // X, Y, Wheel
        0x15, 0x81, 0x25, 0x7F, 0x75, 0x08, 0x95, 0x03, 0x81, 0x06, // rel X, Y, Wheel
        0xC0,             //   End Physical
        0xC0,             // End Mouse Application

        // Collection 2: Touch Pad (Report ID 4)
        0x05, 0x0D,       // Usage Page (Digitizer)
        0x09, 0x05,       // Usage (Touch Pad)
        0xA1, 0x01,       // Collection (Application)
        0x85, 0x04,       //   Report ID (4)
    ];
    for _ in 0..5 {
        d.extend_from_slice(&[
            0x05, 0x0D, 0x09, 0x22, 0xA1, 0x02, // Finger, Logical
            0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x01,
            0x09, 0x47, 0x81, 0x02,             // Confidence (1 bit)
            0x09, 0x42, 0x81, 0x02,             // Tip switch (1 bit)
            0x75, 0x01, 0x95, 0x06, 0x81, 0x03, // 6 x 1-bit constant padding (would eat 30 slots across 5 fingers!)
            0x75, 0x08, 0x95, 0x01, 0x25, 0x0F, 0x09, 0x51, 0x81, 0x02, // Contact ID (8 bits)
            0x05, 0x01, 0x15, 0x00, 0x26, 0x80, 0x0C, 0x75, 0x10, 0x95, 0x01, // X 0..3200 (16 bits)
            0x46, 0xD4, 0x04, 0x09, 0x30, 0x81, 0x02,
            0x26, 0xD0, 0x07, 0x46, 0x16, 0x03, 0x09, 0x31, 0x81, 0x02, // Y 0..2000 (16 bits)
            0x75, 0x01, 0x95, 0x10, 0x81, 0x03, // 16 x 1-bit padding (would eat 80 more slots!)
            0xC0,                               // End Finger (64 bits = 8 bytes per finger)
        ]);
    }
    d.extend_from_slice(&[
        0x05, 0x0D, 0x75, 0x10, 0x95, 0x01, 0x27, 0xFF, 0xFF, 0x00, 0x00, 0x09, 0x56, 0x81, 0x02, // Scan time (16 bits)
        0x09, 0x54, 0x25, 0x05, 0x75, 0x08, 0x95, 0x01, 0x81, 0x02, // Contact count (8 bits)
        0x05, 0x09, 0x09, 0x01, 0x25, 0x01, 0x75, 0x01, 0x95, 0x01, 0x81, 0x02, // Clickpad Button 1
        0x75, 0x01, 0x95, 0x07, 0x81, 0x03, // 7 x 1-bit padding
        0xC0,                               // End Touch Pad Application

        // Collection 3: Device Configuration (Feature Report ID 5, 2 bytes)
        0x05, 0x0D, 0x09, 0x0E, 0xA1, 0x01,
        0x85, 0x05,
        0x09, 0x52, 0x15, 0x00, 0x25, 0x0A, 0x75, 0x08, 0x95, 0x01, 0xB1, 0x02, // Input Mode
        0x09, 0x57, 0x75, 0x08, 0x95, 0x01, 0xB1, 0x02, // Surface/Button switch byte
        0xC0,
    ]);
    d
}

fn encode_5finger_report(fingers: &[(u8, bool, u16, u16)], button: bool, scan_time: u16) -> Vec<u8> {
    let mut r = vec![4u8]; // Report ID 4
    let mut body = [0u8; 44]; // 5 * 8 bytes + 2 scan_time + 1 count + 1 button
    for (i, &(id, tip, x, y)) in fingers.iter().enumerate().take(5) {
        let base = i * 8;
        body[base] = 0x01 | if tip { 0x02 } else { 0 }; // confidence=1, tip
        body[base + 1] = id;
        body[base + 2..base + 4].copy_from_slice(&x.to_le_bytes());
        body[base + 4..base + 6].copy_from_slice(&y.to_le_bytes());
    }
    body[40..42].copy_from_slice(&scan_time.to_le_bytes());
    body[42] = fingers.len() as u8;
    body[43] = u8::from(button);
    r.extend_from_slice(&body);
    r
}

struct MockPad {
    hid_reg: u16,
    rdesc: Vec<u8>,
    reports: VecDeque<Vec<u8>>,
    powered: bool,
    reset_nacks_left: u32,
    send_reset_sentinel: bool,
    rdesc_nacks_left: u32,
    reject_input_mode: bool,
    input_mode_sets: u32,
    last_input_mode_len: usize,
    total_delay_ms: u32,
}

impl MockPad {
    fn new(rdesc: Vec<u8>) -> Self {
        Self {
            hid_reg: 0x0020,
            rdesc,
            reports: VecDeque::new(),
            powered: false,
            reset_nacks_left: 0,
            send_reset_sentinel: true,
            rdesc_nacks_left: 0,
            reject_input_mode: false,
            input_mode_sets: 0,
            last_input_mode_len: 0,
            total_delay_ms: 0,
        }
    }
    fn hid_desc(&self) -> [u8; 30] {
        let mut d = [0u8; 30];
        let words: [u16; 13] = [
            30, 0x0100, self.rdesc.len() as u16, 0x0002, 0x0003, 0x0040,
            0x0004, 0x0010, 0x0005, 0x0006, 0x06CB, 0xCE26, 0x0001,
        ];
        for (i, w) in words.iter().enumerate() {
            d[i * 2..i * 2 + 2].copy_from_slice(&w.to_le_bytes());
        }
        d
    }
}

impl Bus for MockPad {
    fn transfer(&mut self, write: &[u8], read: &mut [u8]) -> bool {
        let reg = (write.len() >= 2).then(|| u16::from_le_bytes([write[0], write[1]]));
        match (reg, write.len()) {
            (Some(r), 2) if r == self.hid_reg => {
                let d = self.hid_desc();
                let n = d.len().min(read.len());
                read[..n].copy_from_slice(&d[..n]);
                true
            }
            (Some(0x0002), 2) => {
                if self.rdesc_nacks_left > 0 {
                    self.rdesc_nacks_left -= 1;
                    return false;
                }
                if read.len() < self.rdesc.len() { return false; }
                read[..self.rdesc.len()].copy_from_slice(&self.rdesc);
                true
            }
            (Some(0x0005), len) if len >= 4 => {
                let opcode = write[3] & 0xF;
                match opcode {
                    hid::OP_SET_POWER => { self.powered = write[2] == 0; true }
                    hid::OP_RESET => true,
                    hid::OP_SET_REPORT => {
                        if self.reject_input_mode { return false; }
                        self.input_mode_sets += 1;
                        self.last_input_mode_len = len;
                        true
                    }
                    _ => false,
                }
            }
            (None, 0) => {
                if self.reset_nacks_left > 0 {
                    self.reset_nacks_left -= 1;
                    return false;
                }
                read.fill(0);
                if let Some(r) = self.reports.pop_front() {
                    let total = (r.len() + 2) as u16;
                    read[..2].copy_from_slice(&total.to_le_bytes());
                    read[2..2 + r.len()].copy_from_slice(&r);
                } else if !self.send_reset_sentinel {
                    // Pad stays non-zero / busy during reset poll if sentinel absent.
                    read[0] = 0xFF;
                    read[1] = 0xFF;
                }
                true
            }
            _ => false,
        }
    }
    fn delay_ms(&mut self, ms: u32) { self.total_delay_ms += ms; }
}

#[test]
fn five_finger_dell_descriptor_parses_all_fingers_button_and_mouse_collection() {
    let rdesc = dell_5finger_ptp_with_mouse_descriptor();
    let l = descriptor::parse(&rdesc).expect("must parse 5-finger PTP + Mouse descriptor");
    assert_eq!(l.touch_report, Some(4));
    assert_eq!(l.mouse_report, Some(1));
    assert_eq!(l.input_mode_report, Some(5));
    assert_eq!(l.input_mode_bytes, 2);
    assert_eq!(l.fingers, 5);
    // Verify finger 4 and Button 1 at the end were NOT truncated by padding!
    assert_eq!(l.finger_field(4, PAGE_DESKTOP, USAGE_X).unwrap().bit, 4 * 64 + 16);
    assert_eq!(l.finger_field(4, PAGE_DESKTOP, USAGE_Y).unwrap().bit, 4 * 64 + 32);
    assert_eq!(l.field(PAGE_DIGITIZER, USAGE_CONTACT_COUNT).unwrap().bit, 5 * 64 + 16);
    assert_eq!(l.field(PAGE_BUTTON, 1).unwrap().bit, 5 * 64 + 24);
    assert!(l.mouse_field(PAGE_DESKTOP, USAGE_X).is_some());
    assert!(l.mouse_field(PAGE_DESKTOP, USAGE_Y).is_some());
}

#[test]
fn slow_reset_and_report_descriptor_retry_bring_up_and_gestures_work() {
    let mut pad = MockPad::new(dell_5finger_ptp_with_mouse_descriptor());
    pad.reset_nacks_left = 3;
    pad.rdesc_nacks_left = 2;
    let mut scratch = [0u8; MAX_REPORT_DESCRIPTOR];
    let mut tp = Touchpad::probe(&mut pad, 0x0020, &mut scratch).expect("probe must succeed after reset settle + retry");
    assert!(pad.powered);
    assert!(tp.touch_mode);
    assert_eq!(pad.input_mode_sets, 1);
    assert!(pad.total_delay_ms >= 25, "must settle after SET_POWER and RESET");
    assert_eq!(tp.extent, (3200, 2000));

    // Feed 1-finger motion, clickpad press, and 2-finger scroll through Recognizer.
    let mut rec = Recognizer::new(Config::for_extent(tp.extent.0));
    let mut dx_sum = 0i32;
    let mut clicks = 0u32;
    let mut scrolls = 0i32;

    for i in 0..8u16 {
        pad.reports.push_back(encode_5finger_report(&[(1, true, 1000 + i * 50, 800)], false, i * 80));
    }
    pad.reports.push_back(encode_5finger_report(&[(1, true, 1350, 800)], true, 700));
    pad.reports.push_back(encode_5finger_report(&[], false, 800));
    // Two-finger vertical scroll.
    for i in 0..8u16 {
        pad.reports.push_back(encode_5finger_report(&[(1, true, 1000, 800 + i * 60), (2, true, 1400, 800 + i * 60)], false, 900 + i * 80));
    }

    let mut t = 0u64;
    while let Ok(Some(Report::Touch(f))) = tp.poll_report(&mut pad) {
        rec.feed(&f, t, |e| match e {
            Event::Move(dx, _) => dx_sum += dx,
            Event::Button(1, true) => clicks += 1,
            Event::Scroll(s) => scrolls += s as i32,
            _ => {}
        });
        t += 8;
    }
    assert!(dx_sum > 50, "expected pointer motion, got {dx_sum}");
    assert_eq!(clicks, 1, "expected clickpad press");
    assert!(scrolls != 0, "expected 2-finger scroll");

    // Sleep & wake must re-apply Input Mode.
    assert!(tp.sleep(&mut pad));
    assert!(!pad.powered);
    assert!(tp.wake(&mut pad));
    assert!(pad.powered);
    assert_eq!(pad.input_mode_sets, 2);
}

#[test]
fn rejected_input_mode_falls_back_to_mouse_collection_cleanly() {
    let mut pad = MockPad::new(dell_5finger_ptp_with_mouse_descriptor());
    pad.reject_input_mode = true;
    pad.send_reset_sentinel = false; // Also test missing reset sentinel!
    let mut scratch = [0u8; MAX_REPORT_DESCRIPTOR];
    let mut tp = Touchpad::probe(&mut pad, 0x0020, &mut scratch).expect("must fall back to Mouse mode");
    assert!(!tp.touch_mode);
    assert_eq!(tp.layout.mouse_report, Some(1));

    // Report ID 1: buttons=0x01 (left), dx=+12, dy=-7, wheel=-1
    pad.reports.push_back(vec![1, 0x01, 12, (-7i8) as u8, (-1i8) as u8]);
    let got = tp.poll_report(&mut pad).unwrap();
    assert_eq!(got, Some(Report::Mouse(MouseReport { dx: 12, dy: -7, wheel: -1, buttons: 0x01 })));
}

#[test]
fn hybrid_single_slot_continuation_packet_is_decoded() {
    let rdesc = vec![
        0x05, 0x0D, 0x09, 0x05, 0xA1, 0x01,
        0x85, 0x01,
        0x05, 0x0D, 0x09, 0x22, 0xA1, 0x02,
        0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x01,
        0x09, 0x42, 0x81, 0x02, // Tip
        0x75, 0x07, 0x95, 0x01, 0x81, 0x03, // Pad
        0x05, 0x01, 0x26, 0xB8, 0x0B, 0x75, 0x10, 0x95, 0x01,
        0x09, 0x30, 0x81, 0x02, // X
        0x09, 0x31, 0x81, 0x02, // Y
        0xC0,
        0x05, 0x0D, 0x09, 0x54, 0x25, 0x05, 0x75, 0x08, 0x95, 0x01, 0x81, 0x02, // Contact count
        0xC0,
    ];
    let l = descriptor::parse(&rdesc).unwrap();
    // Continuation packet: Report ID 1, Tip=1, X=500, Y=600, Contact Count=0.
    let mut pkt = vec![1u8, 0x01];
    pkt.extend_from_slice(&500u16.to_le_bytes());
    pkt.extend_from_slice(&600u16.to_le_bytes());
    pkt.push(0);
    let f = touch::decode(&l, &pkt).expect("continuation packet must decode");
    assert_eq!(f.count, 1);
    assert_eq!((f.contacts[0].x, f.contacts[0].y, f.contacts[0].down), (500, 600, true));
}

#[test]
fn malformed_and_glitch_packets_never_panic_and_pace_recovers_from_bad_irq() {
    assert_eq!(hid::input_payload(&[]), None);
    assert_eq!(hid::input_payload(&[0, 0]), None);
    assert_eq!(hid::input_payload(&[2, 0]), None);
    assert_eq!(hid::input_payload(&[0xFF, 0xFF, 1, 2, 3]), None);
    assert_eq!(HidDescriptor::parse(&[0xFF; 30]), None);

    // Silent IRQ line falls back to polling after SILENT_AFTER check reads with reports.
    let mut p = Pace::line(false, 0);
    for i in 1..=SILENT_AFTER {
        let t = i as u64 * CHECK_TICKS;
        assert!(p.due(t, false));
        let next = p.after(t, Read::Report { touch: true });
        if i == SILENT_AFTER { assert_eq!(next, Next::Fallback("line_silent")); }
    }
    assert_eq!(p.mode, Mode::Poll);

    // Stuck IRQ line falls back to polling after STUCK_AFTER empty reads.
    let mut p = Pace::line(false, 0);
    for i in 1..=STUCK_AFTER {
        assert!(p.due(i as u64, true));
        let next = p.after(i as u64, Read::Empty);
        if i == STUCK_AFTER { assert_eq!(next, Next::Fallback("line_stuck")); }
    }
    assert_eq!(p.mode, Mode::Poll);

    // Non-touchpad HID descriptor is cleanly rejected as NotTouchpad.
    let kbd_only = vec![
        0x05, 0x01, 0x09, 0x06, 0xA1, 0x01,
        0x05, 0x07, 0x19, 0x00, 0x29, 0x65, 0x75, 0x08, 0x95, 0x06, 0x81, 0x00,
        0xC0,
    ];
    let mut pad = MockPad::new(kbd_only);
    let mut scratch = [0u8; MAX_REPORT_DESCRIPTOR];
    assert_eq!(Touchpad::probe(&mut pad, 0x0020, &mut scratch).err(), Some(ProbeError::ReportDescriptor));
}
