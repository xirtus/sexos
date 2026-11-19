// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;

#[allow(dead_code)]
#[path = "../../servers/sexusb/src/hid.rs"]
mod usb_hid;

use usb_hid::{parse_pointer, Field, PointerLayout};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Direct HID report descriptor parsing (used for USB/BT boot & report-mode mice,
    // gaming pointers with report IDs, and absolute tablets).
    if let Some(layout) = parse_pointer(data) {
        if let Some(p) = layout.decode(data) {
            let _ = PointerLayout::scale(&layout.x, p.x);
            let _ = PointerLayout::scale(&layout.y, p.y);
            let _ = PointerLayout::scale(&layout.wheel, p.wheel);
        }
    }

    // 2. Split input into (descriptor, sequence of HID input reports) and decode/scale each.
    if data.len() >= 4 {
        let split = (data[0] as usize).min(data.len() - 1);
        let (desc, reports) = data[1..].split_at(split);
        if let Some(layout) = parse_pointer(desc) {
            let mut pos = 0usize;
            while pos < reports.len() {
                let rlen = (reports[pos] as usize).min(reports.len() - (pos + 1));
                pos += 1;
                let rep = &reports[pos..pos + rlen];
                pos += rlen;
                if let Some(p) = layout.decode(rep) {
                    let _ = PointerLayout::scale(&layout.x, p.x);
                    let _ = PointerLayout::scale(&layout.y, p.y);
                }
            }
        }
    }

    // 3. Direct bitfield reader & scaler edge cases.
    if data.len() >= 12 {
        let f = Field {
            bit: u16::from_le_bytes([data[0], data[1]]),
            size: data[2],
            signed: data[3] & 1 != 0,
            min: i32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            max: i32::from_le_bytes([data[8], data[9], data[10], data[11]]),
        };
        if let Some(val) = f.read(&data[12..]) {
            let _ = PointerLayout::scale(&f, val);
        }
    }
});
