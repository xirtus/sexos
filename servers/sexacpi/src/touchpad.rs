// SPDX-License-Identifier: GPL-3.0-only
//! Finds laptop touchpads (HID over I2C, `PNP0C50`) and tells the Input service where
//! each one is.
//!
//! The Input service holds the Serial IO controllers' registers but cannot read
//! firmware; this service reads firmware but holds no bus. For each present touchpad:
//! `_CRS` gives its I2C address, speed and controller, plus its GPIO interrupt pin;
//! `_DSM` (function 1) gives the HID descriptor register; the controller's `_ADR` gives
//! its PCI function and `FMCN`/`SSCN` its clock counts. Both devices' `_PS0` run first,
//! since some firmware only powers the pad there.
use crate::{eval, eval_int, int_of, name, Aml};
use acpi::aml::namespace::AmlName;
use acpi::aml::object::{Object, WrappedObject};
use alloc::vec::Vec;
use sex_laptop::resources::{self, Resource};
use sex_pdx::serial_println;
use sex_pdx::touchpad::{self as t, Found};

fn path_from(bytes: &[u8], scope: &AmlName) -> Option<AmlName> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let text = core::str::from_utf8(&bytes[..end]).ok()?;
    let n = name(text)?;
    if text.starts_with('\\') { Some(n) } else { n.resolve(&scope.parent().ok()?).ok() }
}

fn hid_register(aml: &Aml, dev: &AmlName) -> Option<u16> {
    let args = alloc::vec![
        WrappedObject::new(Object::Buffer(dsm_uuid().to_vec())),
        WrappedObject::new(Object::Integer(1)),
        WrappedObject::new(Object::Integer(1)),
        WrappedObject::new(Object::Package(Vec::new())),
    ];
    let o = eval(aml, dev, "_DSM", args)?;
    match &*o {
        Object::Integer(v) => Some(*v as u16),
        Object::Buffer(b) if b.len() >= 2 => Some(u16::from_le_bytes([b[0], b[1]])),
        _ => None,
    }
}

/// `3cdff6f7-4267-4555-ad05-b30a3d8938de` in ACPI's byte order (HID over I2C 1.0 §5.1).
fn dsm_uuid() -> [u8; 16] {
    [0xF7, 0xF6, 0xDF, 0x3C, 0x67, 0x42, 0x55, 0x45, 0xAD, 0x05, 0xB3, 0x0A, 0x3D, 0x89, 0x38, 0xDE]
}

fn counts(aml: &Aml, controller: &AmlName, khz: u32) -> Option<(u16, u16, u16)> {
    let method = if khz >= 1000 { "FPCN" } else if khz >= 400 { "FMCN" } else { "SSCN" };
    let o = eval(aml, controller, method, Vec::new())?;
    let Object::Package(items) = &*o else { return None };
    let v: Vec<u64> = items.iter().filter_map(int_of).collect();
    (v.len() >= 2).then(|| (v[0] as u16, v[1] as u16, v.get(2).copied().unwrap_or(0) as u16))
}

/// One touchpad's description, or why there is none.
pub fn describe(aml: &Aml, dev: &AmlName) -> Result<Found, &'static str> {
    let crs = eval(aml, dev, "_CRS", Vec::new()).ok_or("no_crs")?;
    let Object::Buffer(bytes) = &*crs else { return Err("crs_not_buffer") };
    let (mut bus, mut gpio) = (None, None);
    resources::parse(bytes, |r| match r {
        Resource::I2c { address, ten_bit, hz, controller } if bus.is_none() => bus = Some((address, ten_bit, hz, controller.to_vec())),
        Resource::GpioInt { pin, .. } if gpio.is_none() => gpio = Some(pin),
        _ => {}
    });
    let (address, ten_bit, hz, controller) = bus.ok_or("no_i2c")?;
    let controller = path_from(&controller, dev).ok_or("bad_controller_path")?;
    let adr = eval_int(aml, &controller, "_ADR").ok_or("controller_no_adr")?;
    let hid = hid_register(aml, dev).ok_or("no_dsm")?;
    for p in [&controller, dev] { let _ = eval(aml, p, "_PS0", Vec::new()); }
    let khz = (hz / 1000).clamp(100, 1000);
    Ok(Found {
        bus: 0, device: (adr >> 16) as u8 & 31, function: (adr & 7) as u8, address, ten_bit, khz, hid_register: hid,
        counts: counts(aml, &controller, khz), gpio_pin: gpio,
    })
}

/// Describes every touchpad; the caller sends them.
pub fn find(aml: &Aml, devices: &[AmlName]) -> Vec<Found> {
    let mut out = Vec::new();
    for dev in devices {
        match describe(aml, dev) {
            Ok(f) => {
                serial_println!("[sexacpi.touchpad] path={} pci={:02x}:{:02x}.{} address={:#x} ten_bit={} khz={} hid_register={:#x} gpio={} counts={}",
                    dev, f.bus, f.device, f.function, f.address, f.ten_bit as u8, f.khz, f.hid_register,
                    f.gpio_pin.map_or(-1, |p| p as i32), f.counts.map_or(alloc::string::String::from("none"), |(h, l, s)| alloc::format!("{h}/{l}/{s}")));
                out.push(f);
            }
            Err(why) => serial_println!("[sexacpi.touchpad.skip] path={} reason={}", dev, why),
        }
    }
    out
}

/// Sends what Input has not taken yet; true once all are delivered.
pub fn send(pending: &mut Vec<Found>) -> bool {
    while let Some(f) = pending.first() {
        let [a0, a1, a2] = f.encode();
        let (status, _) = sex_pdx::pdx_call(t::ROUTE_SLOT, t::OP_FOUND, a0, a1, a2);
        if status == sex_pdx::ERR_SERVICE_NOT_READY { return false; }
        serial_println!("[sexacpi.touchpad.sent] pci={:02x}:{:02x}.{} status={:#x}", f.bus, f.device, f.function, status);
        pending.remove(0);
    }
    true
}
