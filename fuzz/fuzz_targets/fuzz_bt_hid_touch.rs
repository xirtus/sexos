// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_hid_touch::descriptor::{self, Field};
use sex_hid_touch::gesture::{Config, Recognizer};
use sex_hid_touch::i2c_hid::{self, HidDescriptor};
use sex_hid_touch::session::{Bus, Touchpad};
use sex_hid_touch::touch;

/// Canonical Windows Precision Touchpad HID report descriptor (Report ID 1 touch, Report ID 3 input mode).
const PTP_DESCRIPTOR: &[u8] = &[
    0x05, 0x0D, // Usage Page (Digitizers)
    0x09, 0x05, // Usage (Touch Pad)
    0xA1, 0x01, // Collection (Application)
    0x85, 0x01, //   Report ID (1)
    0x09, 0x22, //   Usage (Finger)
    0xA1, 0x02, //   Collection (Logical)
    0x09, 0x42, //     Usage (Tip Switch)
    0x09, 0x32, //     Usage (In Range)
    0x15, 0x00, //     Logical Minimum (0)
    0x25, 0x01, //     Logical Maximum (1)
    0x75, 0x01, //     Report Size (1)
    0x95, 0x02, //     Report Count (2)
    0x81, 0x02, //     Input (Data,Var,Abs)
    0x95, 0x06, //     Report Count (6)
    0x81, 0x03, //     Input (Const,Var,Abs)
    0x75, 0x08, //     Report Size (8)
    0x09, 0x51, //     Usage (Contact Identifier)
    0x25, 0x0F, //     Logical Maximum (15)
    0x95, 0x01, //     Report Count (1)
    0x81, 0x02, //     Input (Data,Var,Abs)
    0x05, 0x01, //     Usage Page (Generic Desktop)
    0x26, 0xDC, 0x05, // Logical Maximum (1500)
    0x75, 0x10, //     Report Size (16)
    0x09, 0x30, //     Usage (X)
    0x81, 0x02, //     Input (Data,Var,Abs)
    0x26, 0x84, 0x03, // Logical Maximum (900)
    0x09, 0x31, //     Usage (Y)
    0x81, 0x02, //     Input (Data,Var,Abs)
    0xC0,       //   End Collection
    0x05, 0x0D, //   Usage Page (Digitizers)
    0x09, 0x22, //   Usage (Finger 2)
    0xA1, 0x02, //   Collection (Logical)
    0x09, 0x42,
    0x09, 0x32,
    0x15, 0x00,
    0x25, 0x01,
    0x75, 0x01,
    0x95, 0x02,
    0x81, 0x02,
    0x95, 0x06,
    0x81, 0x03,
    0x75, 0x08,
    0x09, 0x51,
    0x25, 0x0F,
    0x95, 0x01,
    0x81, 0x02,
    0x05, 0x01,
    0x26, 0xDC, 0x05,
    0x75, 0x10,
    0x09, 0x30,
    0x81, 0x02,
    0x26, 0x84, 0x03,
    0x09, 0x31,
    0x81, 0x02,
    0xC0,       //   End Collection
    0x05, 0x0D, //   Usage Page (Digitizers)
    0x27, 0xFF, 0xFF, 0x00, 0x00, // Logical Maximum (65535)
    0x75, 0x10, //   Report Size (16)
    0x95, 0x01, //   Report Count (1)
    0x09, 0x56, //   Usage (Scan Time)
    0x81, 0x02, //   Input (Data,Var,Abs)
    0x09, 0x54, //   Usage (Contact Count)
    0x25, 0x05, //   Logical Maximum (5)
    0x75, 0x08, //   Report Size (8)
    0x81, 0x02, //   Input (Data,Var,Abs)
    0x05, 0x09, //   Usage Page (Button)
    0x09, 0x01, //   Usage (Button 1)
    0x25, 0x01, //   Logical Maximum (1)
    0x75, 0x01, //   Report Size (1)
    0x81, 0x02, //   Input (Data,Var,Abs)
    0x95, 0x07, //   Report Count (7)
    0x81, 0x03, //   Input (Const,Var,Abs)
    0x05, 0x0D, //   Usage Page (Digitizers)
    0x85, 0x03, //   Report ID (3)
    0x09, 0x52, //   Usage (Input Mode)
    0x25, 0x0F, //   Logical Maximum (15)
    0x75, 0x08, //   Report Size (8)
    0x95, 0x01, //   Report Count (1)
    0xB1, 0x02, //   Feature (Data,Var,Abs)
    0xC0,       // End Collection
];

struct FuzzBus<'a> {
    hid_desc: [u8; 30],
    report_desc: &'a [u8],
    reports: &'a [u8],
    pos: usize,
    reset_ack_pending: bool,
}

impl<'a> Bus for FuzzBus<'a> {
    fn transfer(&mut self, write: &[u8], read: &mut [u8]) -> bool {
        if write.len() == 2 {
            let reg = u16::from_le_bytes([write[0], write[1]]);
            if reg == 0x0001 && read.len() == 30 {
                read.copy_from_slice(&self.hid_desc);
                return true;
            }
            if reg == 0x0002 && read.len() == self.report_desc.len() {
                read.copy_from_slice(self.report_desc);
                return true;
            }
        }
        if !write.is_empty() && read.is_empty() {
            // Check for reset command (opcode 0x01 in upper nibble of 4th byte)
            if write.len() >= 4 && (write[3] & 0x0F) == i2c_hid::OP_RESET {
                self.reset_ack_pending = true;
            }
            return true;
        }
        if write.is_empty() && !read.is_empty() {
            read.fill(0);
            if self.reset_ack_pending {
                self.reset_ack_pending = false;
                return true;
            }
            if self.pos >= self.reports.len() {
                return false;
            }
            let chunk_len = (self.reports[self.pos] as usize)
                .min(self.reports.len() - (self.pos + 1))
                .min(read.len().saturating_sub(2));
            self.pos += 1;
            let total = (chunk_len + 2) as u16;
            if read.len() >= 2 {
                read[..2].copy_from_slice(&total.to_le_bytes());
                read[2..2 + chunk_len].copy_from_slice(&self.reports[self.pos..self.pos + chunk_len]);
            }
            self.pos += chunk_len;
            return true;
        }
        false
    }
}

fn make_hid_desc(report_desc_len: u16, max_input_len: u16) -> [u8; 30] {
    let mut d = [0u8; 30];
    d[0..2].copy_from_slice(&30u16.to_le_bytes());
    d[2..4].copy_from_slice(&0x0100u16.to_le_bytes());
    d[4..6].copy_from_slice(&report_desc_len.to_le_bytes());
    d[6..8].copy_from_slice(&0x0002u16.to_le_bytes()); // wReportDescRegister
    d[8..10].copy_from_slice(&0x0003u16.to_le_bytes()); // wInputRegister
    d[10..12].copy_from_slice(&max_input_len.to_le_bytes());
    d[12..14].copy_from_slice(&0x0004u16.to_le_bytes()); // wOutputRegister
    d[14..16].copy_from_slice(&4u16.to_le_bytes());
    d[16..18].copy_from_slice(&0x0005u16.to_le_bytes()); // wCommandRegister
    d[18..20].copy_from_slice(&0x0006u16.to_le_bytes()); // wDataRegister
    d[20..22].copy_from_slice(&0x04F3u16.to_le_bytes());
    d[22..24].copy_from_slice(&0x3100u16.to_le_bytes());
    d
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Direct I2C/BT HID framing & helper codecs.
    let _ = HidDescriptor::parse(data);
    let _ = i2c_hid::input_payload(data);
    if data.len() >= 6 {
        let mut cmd_buf = [0u8; 5];
        let reg = u16::from_le_bytes([data[0], data[1]]);
        let _ = i2c_hid::set_power(reg, data[2] & 1 != 0, &mut cmd_buf);
        let _ = i2c_hid::command(reg, data[2] & 0x0F, data[3] & 0x03, data[4], &mut cmd_buf);
        let mut feat_buf = [0u8; 64];
        let _ = i2c_hid::set_feature(reg, reg.wrapping_add(1), data[4], &data[5..data.len().min(32)], &mut feat_buf);

        let f = Field {
            bit: u16::from_le_bytes([data[0], data[1]]) & 0x03FF,
            size: data[2] % 33,
            logical_min: i16::from_le_bytes([data[4], data[5]]) as i32,
            logical_max: i32::MAX,
            ..Field::default()
        };
        let _ = descriptor::value(&f, &data[6..]);
    }

    // 2. Arbitrary HID report descriptor + sequence of HID input reports -> decode -> gesture Recognizer.
    let split = ((data[0] as usize) + 1).min(data.len());
    let (desc_bytes, report_stream) = data[1.min(data.len())..].split_at(
        split.saturating_sub(1).min(data.len().saturating_sub(1)),
    );

    if let Some(layout) = descriptor::parse(desc_bytes) {
        let ext = touch::extent(&layout);
        let mut rec = Recognizer::new(Config::for_extent(ext.0));
        let mut ms = 0u64;
        let mut at = 0usize;
        while at < report_stream.len() {
            let rlen = (report_stream[at] as usize)
                .min(report_stream.len() - (at + 1));
            at += 1;
            let rep = &report_stream[at..at + rlen];
            at += rlen;
            ms = ms.wrapping_add((rep.first().copied().unwrap_or(8) as u64) + 1);
            if let Some(frame) = touch::decode(&layout, rep) {
                rec.feed(&frame, ms, |_| {});
            }
        }
    }
    // Also parse `data` directly as a pure HID report descriptor so short reproducers map 1:1.
    if let Some(layout) = descriptor::parse(data) {
        let ext = touch::extent(&layout);
        let mut rec = Recognizer::new(Config::for_extent(ext.0));
        if let Some(frame) = touch::decode(&layout, data) {
            rec.feed(&frame, 10, |_| {});
        }
    }

    // 3. Full `Touchpad::probe` + `Touchpad::poll` + `Recognizer` session over simulated HID bus.
    let use_custom = (data[0] & 0x80) != 0 && desc_bytes.len() <= 2048;
    let chosen_desc = if use_custom { desc_bytes } else { PTP_DESCRIPTOR };
    let mut bus = FuzzBus {
        hid_desc: make_hid_desc(chosen_desc.len() as u16, 64),
        report_desc: chosen_desc,
        reports: if use_custom { report_stream } else { &data[1..] },
        pos: 0,
        reset_ack_pending: false,
    };
    let mut scratch = [0u8; 2048];
    if let Ok(mut pad) = Touchpad::probe(&mut bus, 0x0001, &mut scratch) {
        let mut rec = Recognizer::new(Config::for_extent(pad.extent.0));
        let mut ms = 100u64;
        for _ in 0..32 {
            match pad.poll(&mut bus) {
                Ok(Some(frame)) => {
                    ms += 12;
                    rec.feed(&frame, ms, |_| {});
                }
                Ok(None) => {
                    ms += 12;
                }
                Err(()) => break,
            }
        }
    }
});
