// SPDX-License-Identifier: GPL-3.0-only
//! Laptop embedded controller and firmware answers (sexacpi + sex-laptop): an EC
//! whose status and data ports answer from the input (stuck busy, events that
//! never clear, garbage data), the `_CRS` resource buffers firmware hands out for
//! the EC, touchpad and I2C devices, and `_BST`/`_BIF`/`_BIX` battery packages.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
use sex_laptop::{battery, ec, resources, thermal};
use sexos_driver_fuzz::{tick, Src};

struct Ports<'a>(Src<'a>, u8);
impl ec::Ports for Ports<'_> {
    fn status(&mut self) -> u8 { tick(); if self.0.is_empty() { self.1 } else { self.0.u8() } }
    fn command(&mut self, _value: u8) {}
    fn read_data(&mut self) -> u8 { tick(); self.0.u8() }
    fn write_data(&mut self, _value: u8) {}
    fn pause(&mut self) {}
}

fn run(d: &[u8]) {
    let mut s = Src::new(d);
    let crs = s.chunk();
    resources::parse(crs, |_| {});
    let _ = resources::ec_ports(crs);
    let words: Vec<Option<u64>> = (0..s.below(24)).map(|_| if s.bool() { Some(s.u64()) } else { None }).collect();
    let info: Vec<Option<u64>> = (0..s.below(24)).map(|_| if s.bool() { Some(s.u32() as u64) } else { None }).collect();
    let bix = s.bool();
    if let Some(r) = battery::Raw::from_packages(&words, &info, bix) {
        if let Some(reading) = battery::combine(&[r, r]) {
            let mut out = [0u8; 6];
            let _ = battery::hm(reading.minutes.unwrap_or(0), &mut out);
            let _ = battery::must_shut_down(&reading, Some(s.bool()));
        }
    }
    let k = s.u32();
    let _ = (thermal::plausible(k as u64), thermal::celsius(k));
    let gone = if s.bool() { 0xff } else { 0x00 };
    let mut e = ec::Ec::new(Ports(Src::new(s.rest()), gone));
    for a in 0..8u8 {
        let _ = e.read(a.wrapping_mul(37));
        let _ = e.write(a, a);
        let _ = e.read_wide(a, 1 + a as usize % 8);
        let _ = e.write_wide(a, 1 + a as usize % 8, 0x1234_5678);
        if e.event_pending() { let _ = e.query(); }
    }
}

fn seeds() -> Vec<Vec<u8>> {
    // _CRS: two I/O ports (0x62, 0x66), I2C serial bus to \_SB.I2C1 address 0x2c, end tag.
    let mut crs = vec![0x47, 1, 0x62, 0, 0x62, 0, 1, 1, 0x47, 1, 0x66, 0, 0x66, 0, 1, 1];
    let path = b"\\_SB.PCI0.I2C1\0";
    let mut i2c = vec![0x8e];
    i2c.extend_from_slice(&((12 + path.len()) as u16).to_le_bytes());
    i2c.extend_from_slice(&[2, 0, 1, 0, 0, 0, 1, 6, 0]);
    i2c.extend_from_slice(&400_000u32.to_le_bytes());
    i2c.extend_from_slice(&0x2cu16.to_le_bytes());
    i2c.extend_from_slice(path);
    crs.extend_from_slice(&i2c);
    crs.extend_from_slice(&[0x79, 0]);
    let mut v = (crs.len() as u16).to_le_bytes().to_vec();
    v.extend_from_slice(&crs);
    v.extend_from_slice(&[4, 0, 0, 0]);
    for x in [1u64, 1500, 4000, 11_400] { v.push(1); v.extend_from_slice(&x.to_le_bytes()); }
    v.extend_from_slice(&[0; 4]);
    // EC: input buffer empty / output full, then data bytes.
    for i in 0..200u8 { v.extend_from_slice(&[0x01, i]); }
    vec![v]
}

sexos_driver_fuzz::driver_target!(run, seeds);
