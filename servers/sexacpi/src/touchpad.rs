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
//!
//! The interrupt: an `Interrupt` resource is already an I/O APIC input. A `GpioInt`
//! pin is looked up on the GPIO controller it names (Intel Tiger Lake / Alder Lake,
//! `sex_laptop::intel_gpio`): when firmware routed that pad to the I/O APIC, its
//! `INTSEL` is the input. The pad is only read, never changed. Anything else leaves
//! `irq` empty and Input polls.
use crate::{eval, eval_int, int_of, name, Aml};
use acpi::aml::namespace::AmlName;
use acpi::aml::object::{Object, WrappedObject};
use alloc::vec::Vec;
use sex_laptop::intel_gpio::{self as gpio, Chip, Pad};
use sex_laptop::resources::{self, Resource};
use sex_pdx::power as w;
use sex_pdx::serial_println;
use sex_pdx::touchpad::{self as t, Found, Irq};

/// One 32-bit register read through the kernel's firmware window.
fn read32(address: u64) -> Option<u32> {
    crate::spend(1);
    let r = w::call4(w::FW_MEM, address, 4, 0);
    (r[0] == w::OK).then_some(r[1] as u32)
}

fn hid_of(aml: &Aml, dev: &AmlName) -> Option<alloc::string::String> {
    let o = eval(aml, dev, "_HID", Vec::new())?;
    match &*o {
        Object::Integer(v) => Some(alloc::string::String::from_utf8_lossy(&w::eisa_id(*v as u32)).into_owned()),
        Object::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// The I/O APIC input behind a `GpioInt` pin, or why there is none.
fn gpio_line(aml: &Aml, dev: &AmlName, pin: u16, active_low: bool, controller: &[u8]) -> Result<Irq, &'static str> {
    let path = path_from(controller, dev).ok_or("gpio_bad_path")?;
    let chip = hid_of(aml, &path).as_deref().and_then(Chip::from_hid).ok_or("gpio_chip_unknown")?;
    let (community, pad) = chip.locate(pin).ok_or("gpio_pin_unknown")?;
    let crs = eval(aml, &path, "_CRS", Vec::new()).ok_or("gpio_no_crs")?;
    let Object::Buffer(bytes) = &*crs else { return Err("gpio_crs_not_buffer") };
    let mut bases = Vec::new();
    resources::parse(bytes, |r| if let Resource::Memory { base, length } = r { bases.push((base as u64, length)); });
    let &(base, length) = bases.get(community).ok_or("gpio_no_community")?;
    let revid = read32(base + gpio::REVID as u64).ok_or("gpio_unreadable")?;
    let padbar = read32(base + gpio::PADBAR as u64).ok_or("gpio_unreadable")?;
    let at = gpio::pad_offset(revid, padbar, pad);
    if at as u64 + 8 > length as u64 { return Err("gpio_pad_outside"); }
    let p = Pad { dw0: read32(base + at as u64).ok_or("gpio_unreadable")?, dw1: read32(base + at as u64 + 4).ok_or("gpio_unreadable")? };
    serial_println!("[sexacpi.touchpad.gpio] chip={} pin={} community={} pad={} dw0={:#010x} dw1={:#010x}", chip.name, pin, community, pad, p.dw0, p.dw1);
    let l = p.apic_line(active_low).map_err(|e| match e {
        gpio::NotRouted::NotGpioInput => "gpio_not_input",
        gpio::NotRouted::NotApicRouted => "gpio_not_apic_routed",
        gpio::NotRouted::NoLine => "gpio_no_line",
    })?;
    Ok(Irq { gsi: l.gsi, edge: l.edge, active_low: l.active_low })
}

fn path_from(bytes: &[u8], scope: &AmlName) -> Option<AmlName> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let text = core::str::from_utf8(&bytes[..end]).ok()?;
    let n = name(text)?;
    if text.starts_with('\\') { Some(n) } else { n.resolve(&scope.parent().ok()?).ok() }
}

fn hid_register(aml: &Aml, dev: &AmlName) -> Option<u16> {
    for rev in [1u64, 0u64] {
        let args = alloc::vec![
            WrappedObject::new(Object::Buffer(dsm_uuid().to_vec())),
            WrappedObject::new(Object::Integer(rev)),
            WrappedObject::new(Object::Integer(1)),
            WrappedObject::new(Object::Package(Vec::new())),
        ];
        let Some(o) = eval(aml, dev, "_DSM", args) else { continue };
        match &*o {
            Object::Integer(v) => return Some(*v as u16),
            Object::Buffer(b) if b.len() >= 2 => return Some(u16::from_le_bytes([b[0], b[1]])),
            Object::Buffer(b) if b.len() == 1 => return Some(b[0] as u16),
            _ => {}
        }
    }
    None
}

/// `3cdff6f7-4267-4555-ad05-b30a3d8938de` in ACPI's byte order (HID over I2C 1.0 §5.1).
fn dsm_uuid() -> [u8; 16] {
    [0xF7, 0xF6, 0xDF, 0x3C, 0x67, 0x42, 0x55, 0x45, 0xAD, 0x05, 0xB3, 0x0A, 0x3D, 0x89, 0x38, 0xDE]
}

fn power_on(aml: &Aml, dev: &AmlName) {
    if let Some(list) = eval(aml, dev, "_PR0", Vec::new()) {
        if let Object::Package(items) = &*list {
            for item in items.iter() {
                let Object::String(s) = &**item else { continue };
                let Some(n) = name(s) else { continue };
                let found = aml.namespace.lock().search(&n, dev).ok()
                    .and_then(|(full, obj)| matches!(*obj, Object::PowerResource { .. }).then_some(full));
                if let Some(full) = found { let _ = eval(aml, &full, "_ON", Vec::new()); }
            }
        }
    }
    let _ = eval(aml, dev, "_PS0", Vec::new());
}

fn counts(aml: &Aml, controller: &AmlName, khz: u32) -> Option<(u16, u16, u16)> {
    let method = if khz >= 1000 { "FPCN" } else if khz >= 400 { "FMCN" } else { "SSCN" };
    let o = eval(aml, controller, method, Vec::new())?;
    let Object::Package(items) = &*o else { return None };
    let v: Vec<u64> = items.iter().filter_map(int_of).collect();
    (v.len() >= 2 && v[0] > 0 && v[1] > 0).then(|| (v[0] as u16, v[1] as u16, v.get(2).copied().unwrap_or(0) as u16))
}

/// One touchpad's description, or why there is none.
pub fn describe(aml: &Aml, dev: &AmlName) -> Result<Found, &'static str> {
    let crs = match crate::eval_result(aml, dev, "_CRS", Vec::new()) {
        Ok(Some(o)) => o,
        Ok(None) => return Err("no_crs"),
        Err(e) => {
            serial_println!("[sexacpi.touchpad.crs.err] path={} err={:?}", dev, e);
            dump_methods(aml, dev);
            // Power the pad and its controller anyway: Input's fallback scan finds the pad
            // on the bus without `_CRS`, but only if firmware has switched it on.
            if let Ok(parent) = dev.parent() { power_on(aml, &parent); }
            power_on(aml, dev);
            return Err("crs_failed");
        }
    };
    let Object::Buffer(bytes) = &*crs else { return Err("crs_not_buffer") };
    let (mut bus, mut gpio, mut apic) = (None, None, None);
    resources::parse(bytes, |r| match r {
        Resource::I2c { address, ten_bit, hz, controller } if bus.is_none() => bus = Some((address, ten_bit, hz, controller.to_vec())),
        Resource::GpioInt { pin, active_low, controller, .. } if gpio.is_none() => gpio = Some((pin, active_low, controller.to_vec())),
        Resource::Interrupt { number, edge, active_low } if apic.is_none() && number <= 0x3ff => apic = Some(Irq { gsi: number as u16, edge, active_low }),
        _ => {}
    });
    let (address, ten_bit, hz, controller) = bus.ok_or("no_i2c")?;
    let controller = path_from(&controller, dev).ok_or("bad_controller_path")?;
    let adr = eval_int(aml, &controller, "_ADR").ok_or("controller_no_adr")?;
    let hid = hid_register(aml, dev).ok_or("no_dsm")?;
    for p in [&controller, dev] { power_on(aml, p); }
    let khz = (hz / 1000).clamp(100, 1000);
    let irq = match (apic, &gpio) {
        (Some(i), _) => Ok(i),
        (None, Some((pin, active_low, source))) => gpio_line(aml, dev, *pin, *active_low, source),
        (None, None) => Err("none_named"),
    };
    if let Err(why) = irq { serial_println!("[sexacpi.touchpad.irq] path={} line=none reason={}", dev, why); }
    Ok(Found {
        bus: 0, device: (adr >> 16) as u8 & 31, function: (adr & 7) as u8, address, ten_bit, khz, hid_register: hid,
        counts: counts(aml, &controller, khz), gpio_pin: gpio.map(|g| g.0), irq: irq.ok(),
    })
}

/// Describes every touchpad; the caller sends them.
pub fn find(aml: &Aml, devices: &[AmlName]) -> Vec<Found> {
    let mut out = Vec::new();
    for dev in devices {
        match describe(aml, dev) {
            Ok(f) => {
                serial_println!("[sexacpi.touchpad] path={} pci={:02x}:{:02x}.{} address={:#x} ten_bit={} khz={} hid_register={:#x} gpio={} counts={} irq={}",
                    dev, f.bus, f.device, f.function, f.address, f.ten_bit as u8, f.khz, f.hid_register,
                    f.gpio_pin.map_or(-1, |p| p as i32), f.counts.map_or(alloc::string::String::from("none"), |(h, l, s)| alloc::format!("{h}/{l}/{s}")),
                    f.irq.map_or(alloc::string::String::from("none"), |i| alloc::format!("{}/{}/{}", i.gsi, if i.edge { "edge" } else { "level" }, if i.active_low { "low" } else { "high" })));
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

/// When a touchpad's `_CRS` fails: its AML and that of every method it names (one
/// level, at most 6 KiB), so the failing opcode can be found from the log alone.
fn dump_methods(aml: &Aml, dev: &AmlName) {
    let mut seen: Vec<AmlName> = Vec::new();
    let mut queue: Vec<AmlName> = Vec::new();
    if let Some(n) = name("_CRS").and_then(|n| n.resolve(dev).ok()) { queue.push(n); }
    let mut budget = 6144usize;
    let mut level = 0;
    while !queue.is_empty() && level < 2 {
        let mut next = Vec::new();
        for path in core::mem::take(&mut queue) {
            if seen.contains(&path) { continue; }
            seen.push(path.clone());
            let Ok(obj) = aml.namespace.lock().get(path.clone()) else { continue };
            let Object::Method { code, flags } = &*obj else { continue };
            let take = code.len().min(budget);
            budget -= take;
            serial_println!("[sexacpi.aml.method] path={} flags={:#x} bytes={}", path, flags.0, code.len());
            for (i, chunk) in code[..take].chunks(48).enumerate() {
                let mut hex = alloc::string::String::new();
                for b in chunk { let _ = core::fmt::Write::write_fmt(&mut hex, format_args!("{b:02x}")); }
                serial_println!("[sexacpi.aml.hex] path={} at={:#x} {}", path, i * 48, hex);
            }
            for w in code.windows(4) {
                let ok = (w[0].is_ascii_uppercase() || w[0] == b'_') && w[1..].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_');
                if !ok { continue; }
                let Ok(text) = core::str::from_utf8(w) else { continue };
                let Some(n) = name(text) else { continue };
                if let Ok((full, o)) = aml.namespace.lock().search(&n, dev) {
                    if matches!(*o, Object::Method { .. }) && !seen.contains(&full) && !next.contains(&full) { next.push(full); }
                }
            }
            if budget == 0 { return; }
        }
        queue = next;
        level += 1;
    }
}
