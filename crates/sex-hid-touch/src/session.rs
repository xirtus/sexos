// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! One touchpad on an I2C bus: bring-up and polling.
//!
//! Bring-up follows HID over I2C 1.0 §7: read the HID descriptor from the register
//! firmware names, power on, reset, read the report descriptor, then switch a Precision
//! Touchpad from mouse emulation into touchpad mode (Input Mode feature = 3).
//!
//! Polling reads the input register directly. A pad with nothing new answers a zero
//! length, or repeats its last report; a repeat carries the same scan time, so identical
//! bytes are dropped.
use crate::descriptor::{self, Layout};
use crate::i2c_hid::{self as hid, HidDescriptor};
use crate::touch::{self, Frame};

/// The bus side: one device address on one controller. `write` first, a repeated start,
/// then `read`; either may be empty. False when the transfer failed.
pub trait Bus {
    fn transfer(&mut self, write: &[u8], read: &mut [u8]) -> bool;
}

/// The longest input report kept for de-duplication (Precision Touchpads send ~30-60 bytes).
pub const MAX_INPUT: usize = 256;
/// The longest report descriptor read.
pub const MAX_REPORT_DESCRIPTOR: usize = 4096;
/// Input Mode value that turns on multi-touch reports.
pub const INPUT_MODE_TOUCHPAD: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// The HID descriptor could not be read (nothing at that address or register).
    NoAnswer,
    /// The HID descriptor is not HID over I2C 1.00.
    BadDescriptor,
    /// A bring-up command failed.
    Command(&'static str),
    /// The report descriptor is too long, unreadable, or unparseable.
    ReportDescriptor,
    /// A HID device without touchpad contacts (a keyboard, a sensor hub).
    NotTouchpad,
}

pub struct Touchpad {
    pub hid: HidDescriptor,
    pub layout: Layout,
    pub extent: (i32, i32),
    /// Whether the Input Mode feature was written (false: the pad has none and is
    /// already in touchpad mode, or only emulates a mouse).
    pub touch_mode: bool,
    last: [u8; MAX_INPUT],
    last_len: usize,
    pub reports: u64,
    pub repeats: u64,
}

fn read_register<B: Bus>(bus: &mut B, register: u16, out: &mut [u8]) -> bool {
    bus.transfer(&register.to_le_bytes(), out)
}

impl Touchpad {
    /// Brings up the pad whose HID descriptor sits at `descriptor_register` (from `_DSM`).
    /// `scratch` holds the report descriptor while it is parsed.
    pub fn probe<B: Bus>(bus: &mut B, descriptor_register: u16, scratch: &mut [u8]) -> Result<Self, ProbeError> {
        let mut raw = [0u8; hid::DESCRIPTOR_LEN];
        if !read_register(bus, descriptor_register, &mut raw) { return Err(ProbeError::NoAnswer); }
        let d = HidDescriptor::parse(&raw).ok_or(ProbeError::BadDescriptor)?;
        let mut cmd = [0u8; 5];
        let n = hid::set_power(d.command_register, true, &mut cmd);
        if !bus.transfer(&cmd[..n], &mut []) { return Err(ProbeError::Command("set_power")); }
        let n = hid::command(d.command_register, hid::OP_RESET, 0, 0, &mut cmd);
        if !bus.transfer(&cmd[..n], &mut []) { return Err(ProbeError::Command("reset")); }
        // The reset answer is an empty input report; read it so it is not taken for data.
        let mut ack = [0u8; 2];
        let _ = bus.transfer(&[], &mut ack);
        let len = d.report_desc_length as usize;
        if len > scratch.len().min(MAX_REPORT_DESCRIPTOR) { return Err(ProbeError::ReportDescriptor); }
        if !read_register(bus, d.report_desc_register, &mut scratch[..len]) { return Err(ProbeError::ReportDescriptor); }
        let layout = descriptor::parse(&scratch[..len]).ok_or(ProbeError::ReportDescriptor)?;
        if layout.touch_report.is_none() && layout.fingers == 0 { return Err(ProbeError::NotTouchpad); }
        let mut touch_mode = false;
        if let Some(id) = layout.input_mode_report {
            let mut out = [0u8; 16];
            let n = hid::set_feature(d.command_register, d.data_register, id, &[INPUT_MODE_TOUCHPAD], &mut out)
                .ok_or(ProbeError::Command("input_mode"))?;
            if !bus.transfer(&out[..n], &mut []) { return Err(ProbeError::Command("input_mode")); }
            touch_mode = true;
        }
        let extent = touch::extent(&layout);
        Ok(Self { hid: d, layout, extent, touch_mode, last: [0; MAX_INPUT], last_len: 0, reports: 0, repeats: 0 })
    }

    /// Bytes one input read takes.
    pub fn input_len(&self) -> usize { (self.hid.max_input_length as usize).clamp(2, MAX_INPUT) }

    /// Reads the input register once. `Ok(None)`: nothing new. `Err(())`: the bus failed.
    pub fn poll<B: Bus>(&mut self, bus: &mut B) -> Result<Option<Frame>, ()> {
        let mut raw = [0u8; MAX_INPUT];
        let n = self.input_len();
        if !bus.transfer(&[], &mut raw[..n]) { return Err(()); }
        let Some(payload) = hid::input_payload(&raw[..n]) else { return Ok(None) };
        if payload.len() == self.last_len && payload == &self.last[..self.last_len] {
            self.repeats += 1;
            return Ok(None);
        }
        self.last[..payload.len()].copy_from_slice(payload);
        self.last_len = payload.len();
        let frame = touch::decode(&self.layout, payload);
        if frame.is_some() { self.reports += 1; }
        Ok(frame)
    }

    /// Puts the pad to sleep (before s2idle or shutdown).
    pub fn sleep<B: Bus>(&self, bus: &mut B) -> bool {
        let mut cmd = [0u8; 5];
        let n = hid::set_power(self.hid.command_register, false, &mut cmd);
        bus.transfer(&cmd[..n], &mut [])
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::descriptor::tests::ptp_descriptor;
    use crate::gesture::{Config, Event, Recognizer};
    use crate::touch::tests::report;
    use std::collections::VecDeque;

    /// An I2C HID Precision Touchpad: HID descriptor at register 0x0001, report descriptor
    /// at 0x0002, input at 0x0003, command 0x0005, data 0x0006.
    pub struct Pad {
        pub reports: VecDeque<Vec<u8>>,
        pub last: Vec<u8>,
        pub powered: bool,
        pub reset: bool,
        pub reset_pending: bool,
        pub input_mode: Option<u8>,
        pub repeat_last: bool,
        pub writes: Vec<Vec<u8>>,
    }
    impl Pad {
        pub fn new() -> Self {
            Self { reports: VecDeque::new(), last: vec![], powered: false, reset: false, reset_pending: false, input_mode: None, repeat_last: true, writes: vec![] }
        }
        fn descriptor() -> Vec<u8> {
            let rd = ptp_descriptor().len() as u16;
            let mut d = vec![];
            for w in [30u16, 0x0100, rd, 0x0002, 0x0003, 64, 0x0004, 0, 0x0005, 0x0006, 0x04f3, 0x3195, 1, 0, 0] { d.extend_from_slice(&w.to_le_bytes()); }
            d
        }
        pub fn push(&mut self, report: Vec<u8>) { self.reports.push_back(report); }
    }
    impl Bus for Pad {
        fn transfer(&mut self, write: &[u8], read: &mut [u8]) -> bool {
            if !write.is_empty() { self.writes.push(write.to_vec()); }
            let register = if write.len() >= 2 { Some(u16::from_le_bytes([write[0], write[1]])) } else { None };
            match (register, write.len()) {
                (Some(0x0001), 2) => { let d = Self::descriptor(); let n = d.len().min(read.len()); read[..n].copy_from_slice(&d[..n]); }
                (Some(0x0002), 2) => { let d = ptp_descriptor(); let n = read.len(); read.copy_from_slice(&d[..n]); }
                (Some(0x0005), _) => {
                    let (kind, opcode) = (write[2], write[3] & 0xf);
                    match opcode {
                        hid::OP_SET_POWER => self.powered = kind == 0,
                        hid::OP_RESET => { self.reset = true; self.reset_pending = true; }
                        hid::OP_SET_REPORT => {
                            // command, data register, length, report id, value
                            let at = 4;
                            assert_eq!(u16::from_le_bytes([write[at], write[at + 1]]), 0x0006);
                            self.input_mode = Some(write[at + 5]);
                            assert_eq!(write[at + 4], 3, "Input Mode is feature report 3");
                        }
                        _ => return false,
                    }
                }
                (None, 0) => {
                    read.fill(0);
                    if self.reset_pending { self.reset_pending = false; return true; }
                    let r = match self.reports.pop_front() { Some(r) => { self.last = r.clone(); r } None if self.repeat_last && !self.last.is_empty() => self.last.clone(), None => return true };
                    let len = (r.len() + 2) as u16;
                    read[..2].copy_from_slice(&len.to_le_bytes());
                    read[2..2 + r.len()].copy_from_slice(&r);
                }
                _ => return false,
            }
            true
        }
    }

    fn up(pad: &mut Pad) -> Touchpad {
        let mut scratch = [0u8; MAX_REPORT_DESCRIPTOR];
        Touchpad::probe(pad, 0x0001, &mut scratch).unwrap()
    }

    #[test]
    fn brings_a_precision_touchpad_into_touchpad_mode() {
        let mut pad = Pad::new();
        let t = up(&mut pad);
        assert!(pad.powered && pad.reset);
        assert_eq!(pad.input_mode, Some(INPUT_MODE_TOUCHPAD));
        assert!(t.touch_mode);
        assert_eq!(t.extent, (3000, 2000));
        assert_eq!((t.hid.vendor_id, t.hid.product_id), (0x04f3, 0x3195));
        assert_eq!(t.input_len(), 64);
    }

    #[test]
    fn repeated_reports_from_polling_are_dropped() {
        let mut pad = Pad::new();
        let mut t = up(&mut pad);
        assert_eq!(t.poll(&mut pad), Ok(None));
        pad.push(report(&[(1, true, 100, 100)], false));
        assert!(t.poll(&mut pad).unwrap().is_some());
        assert_eq!(t.poll(&mut pad), Ok(None));
        assert_eq!(t.poll(&mut pad), Ok(None));
        assert_eq!((t.reports, t.repeats), (1, 2));
        pad.push(report(&[(1, true, 140, 100)], false));
        assert_eq!(t.poll(&mut pad).unwrap().unwrap().contacts[0].x, 140);
    }

    #[test]
    fn a_silent_address_and_a_bad_descriptor_fail_cleanly() {
        struct Nobody;
        impl Bus for Nobody { fn transfer(&mut self, _: &[u8], _: &mut [u8]) -> bool { false } }
        let mut scratch = [0u8; 64];
        assert_eq!(Touchpad::probe(&mut Nobody, 1, &mut scratch).err(), Some(ProbeError::NoAnswer));
        struct Garbage;
        impl Bus for Garbage { fn transfer(&mut self, _: &[u8], r: &mut [u8]) -> bool { r.fill(0xff); true } }
        assert_eq!(Touchpad::probe(&mut Garbage, 1, &mut scratch).err(), Some(ProbeError::BadDescriptor));
    }

    #[test]
    fn a_finger_sliding_moves_the_pointer_end_to_end() {
        let mut pad = Pad::new();
        let mut t = up(&mut pad);
        let mut g = Recognizer::new(Config::for_extent(t.extent.0));
        let mut moved = 0;
        for (i, x) in (1000..1600).step_by(40).enumerate() {
            pad.push(report(&[(1, true, x, 800)], false));
            if let Some(f) = t.poll(&mut pad).unwrap() {
                g.feed(&f, i as u64 * 8, |e| if let Event::Move(dx, _) = e { moved += dx; });
            }
        }
        assert!(moved > 100, "moved {moved}");
    }

    #[test]
    fn sleeps_the_pad() {
        let mut pad = Pad::new();
        let t = up(&mut pad);
        assert!(t.sleep(&mut pad));
        assert!(!pad.powered);
    }
}
