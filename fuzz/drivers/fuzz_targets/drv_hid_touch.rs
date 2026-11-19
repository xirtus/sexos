// SPDX-License-Identifier: GPL-3.0-only
//! I2C-HID touchpads (the x17's Precision Touchpad): the whole bring-up and input
//! path against a pad that answers every I2C read with fuzz bytes: HID descriptor,
//! report descriptor, Input Mode feature write, then input reports through the
//! contact decoder and the gesture recognizer.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
use sex_hid_touch::{descriptor, gesture, i2c_hid, session, touch};
use sexos_driver_fuzz::{tick, Src};

struct Pad<'a>(Src<'a>);
impl session::Bus for Pad<'_> {
    fn transfer(&mut self, _write: &[u8], read: &mut [u8]) -> bool {
        tick();
        if self.0.u8() == 0xee { return false; } // NAK now and then
        for b in read.iter_mut() { *b = self.0.u8(); }
        true
    }
}

fn run(d: &[u8]) {
    let _ = i2c_hid::HidDescriptor::parse(d);
    let _ = i2c_hid::input_payload(d);
    if let Some(l) = descriptor::parse(d) {
        let _ = touch::extent(&l);
        for f in l.fields() { let _ = descriptor::value(f, d); }
        let _ = touch::decode(&l, d);
    }
    let mut bus = Pad(Src::new(d));
    let mut scratch = vec![0u8; session::MAX_REPORT_DESCRIPTOR];
    let Ok(mut pad) = session::Touchpad::probe(&mut bus, 1, &mut scratch) else { return };
    let _ = pad.input_len();
    let mut g = gesture::Recognizer::new(gesture::Config::for_extent(3000));
    for t in 0..64u64 {
        if bus.0.is_empty() { break; }
        match pad.poll(&mut bus) {
            Ok(Some(frame)) => g.feed(&frame, t * 8, |_| {}),
            Ok(None) => {}
            Err(()) => break,
        }
    }
    let _ = pad.sleep(&mut bus);
}

fn seeds() -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../seeds/fuzz_bt_hid_touch"))
        .map(|r| r.filter_map(|e| std::fs::read(e.ok()?.path()).ok()).collect()).unwrap_or_default();
    // A whole bring-up as the bus sees it: each transfer is one NAK byte (0 = ack)
    // then the bytes read. HID descriptor (report descriptor 86 bytes at register
    // 2, input register 3, 16-byte reports), set power, reset, the empty reset
    // report, the report descriptor, Input Mode, then finger reports.
    let Ok(rd) = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../seeds/fuzz_bt_hid_touch/01_ptp_descriptor.bin")) else { return v };
    let w = |x: u16| x.to_le_bytes();
    let mut hd = Vec::new();
    for x in [30u16, 0x0100, rd.len() as u16, 2, 3, 16, 4, 0, 5, 6, 0x045e, 0x0001, 1] { hd.extend_from_slice(&w(x)); }
    hd.extend_from_slice(&[0, 0, 0, 0]);
    let mut b = vec![0u8]; b.extend_from_slice(&hd);
    b.extend_from_slice(&[0, 0, 0, 0, 0]);
    b.push(0); b.extend_from_slice(&rd);
    b.push(0);
    for i in 0..48u8 { b.push(0); b.extend_from_slice(&[16, 0, 1, 0x03, i, 0x10, i, 0x08, 0, 0, 0x10, 0, 1, 0, 0, 0]); }
    v.push(b);
    v
}

sexos_driver_fuzz::driver_target!(run, seeds);
