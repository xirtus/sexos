// SPDX-License-Identifier: GPL-3.0-only
//! ACPI-WMI devices (`PNP0C14`) and the vendor services built on them: Alienware Command
//! Center fans, temperatures and thermal profiles, and Dell hotkey events.
//!
//! `_WDG` parsing and the vendor protocols live in `sex_laptop::{wmi, awcc, dell_wmi}`;
//! this file only runs the AML: `WMxx(instance, method, buffer)` and `_WED(notify)`.
use crate::{bounded, eval, Aml};
use acpi::aml::namespace::AmlName;
use acpi::aml::object::{Object, WrappedObject};
use alloc::vec::Vec;
use sex_laptop::profile::Profile;
use sex_laptop::wmi::{self as wmi_data, guids, Block};
use sex_laptop::{awcc, dell_smbios, dell_wmi, smbios};
use sex_pdx::serial_println;

pub struct Device { pub path: AmlName, wdg: Vec<u8> }

impl Device {
    fn block(&self, guid: &[u8; 16]) -> Option<Block> { wmi_data::find(&self.wdg, guid) }
}

/// Every WMI device and its blocks, logged once.
pub fn find(aml: &Aml, all: &[AmlName]) -> Vec<Device> {
    let mut out = Vec::new();
    for dev in all.iter().filter(|p| crate::ids_of(aml, p).iter().any(|i| i == "PNP0C14")).take(8) {
        let wdg = match eval(aml, dev, "_WDG", Vec::new()).as_deref() {
            Some(Object::Buffer(b)) => b.clone(),
            _ => continue,
        };
        for b in wmi_data::blocks(&wdg) {
            let mut text = [0u8; 36];
            wmi_data::guid_text(&b.guid, &mut text);
            serial_println!("[sexacpi.wmi] path={} guid={} name={} id={:02x}{:02x} instances={} flags={:#x}",
                dev, core::str::from_utf8(&text).unwrap_or("?"), wmi_data::known(&b.guid).unwrap_or("unknown"),
                b.id[0], b.id[1], b.instances, b.flags);
        }
        out.push(Device { path: dev.clone(), wdg });
    }
    out
}

fn word_of(o: &Object) -> Option<u32> {
    match o {
        Object::Integer(v) => Some(*v as u32),
        Object::Buffer(b) if b.len() >= 4 => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        _ => None,
    }
}

/// `WMAX` on one device.
struct Wmax<'a> { aml: &'a Aml, dev: &'a AmlName, method: [u8; 4] }

impl awcc::Firmware for Wmax<'_> {
    fn call(&mut self, method: u32, a: u32) -> Option<u32> {
        let name = core::str::from_utf8(&self.method).ok()?;
        let args = alloc::vec![
            WrappedObject::new(Object::Integer(0)),
            WrappedObject::new(Object::Integer(method as u64)),
            WrappedObject::new(Object::Buffer(a.to_le_bytes().to_vec())),
        ];
        bounded(|| eval(self.aml, self.dev, name, args)).as_deref().and_then(word_of)
    }
}

/// The Alienware fan controller, when the machine has one.
pub struct Awcc { dev: AmlName, method: [u8; 4], pub inventory: awcc::Inventory, pub last: awcc::Reading, reads: u32 }

impl Awcc {
    pub fn find(aml: &Aml, devices: &[Device]) -> Option<Self> {
        let (dev, block) = devices.iter().find_map(|d| d.block(&guids::ALIENWARE_WMAX).map(|b| (d, b)))?;
        let method = block.method_name()?;
        let mut fw = Wmax { aml, dev: &dev.path, method };
        let Some(inventory) = awcc::probe(&mut fw) else {
            serial_println!("[sexacpi.awcc] path={} method={} answered=0", dev.path, core::str::from_utf8(&method).unwrap_or("?"));
            return None;
        };
        let last = awcc::read(&mut fw, &inventory);
        let current = last.profile.and_then(awcc::kind).map_or("unknown", awcc::kind_name);
        serial_println!("[sexacpi.awcc] path={} method={} description={:#010x} fans={} sensors={} profiles={} current={}",
            dev.path, core::str::from_utf8(&method).unwrap_or("?"), inventory.description.word(),
            inventory.fan_count, inventory.sensor_count, inventory.profile_count, current);
        for (i, f) in inventory.fans().iter().enumerate() {
            serial_println!("[sexacpi.awcc.fan] id={:#04x} name={} rpm={} min={} max={}", f.id, awcc::fan_name(f.id), last.rpm[i].map_or(-1, |v| v as i32), f.min_rpm, f.max_rpm);
        }
        for (i, s) in inventory.sensors().iter().enumerate() {
            serial_println!("[sexacpi.awcc.temp] id={:#04x} name={} celsius={}", s, awcc::sensor_name(*s), last.celsius[i].map_or(-1, |c| c as i32));
        }
        for p in inventory.profiles() {
            serial_println!("[sexacpi.awcc.profile] id={:#04x} kind={}", p, awcc::kind(*p).map_or("unknown", awcc::kind_name));
        }
        Some(Self { dev: dev.path.clone(), method, inventory, last, reads: 0 })
    }

    fn fw<'a>(&'a self, aml: &'a Aml) -> Wmax<'a> { Wmax { aml, dev: &self.dev, method: self.method } }

    /// New fan speeds and temperatures; a summary line about once a minute.
    pub fn refresh(&mut self, aml: &Aml) {
        let r = awcc::read(&mut self.fw(aml), &self.inventory);
        self.reads = self.reads.wrapping_add(1);
        if self.reads % 60 == 1 || r.profile != self.last.profile {
            let mut fans = alloc::string::String::new();
            for (i, f) in self.inventory.fans().iter().enumerate() { fans.push_str(&alloc::format!("{}:{} ", awcc::fan_name(f.id), r.rpm[i].map_or(-1, |v| v as i32))); }
            let mut temps = alloc::string::String::new();
            for (i, s) in self.inventory.sensors().iter().enumerate() {
                temps.push_str(&alloc::format!("{}:{} ", awcc::sensor_name(*s), r.celsius[i].map_or(-1, |c| c as i32)));
            }
            serial_println!("[sexacpi.awcc.reading] rpm={} celsius={} profile={}", fans.trim_end(), temps.trim_end(),
                r.profile.and_then(awcc::kind).map_or("unknown", awcc::kind_name));
        }
        self.last = r;
    }

    /// Follows the desktop's power profile with the matching firmware thermal profile.
    pub fn follow(&mut self, aml: &Aml, p: Profile) {
        let Some(id) = awcc::for_profile(p, self.inventory.profiles()) else {
            serial_println!("[sexacpi.awcc.set] desktop={} firmware=none", p.name());
            return;
        };
        let inv = self.inventory;
        let ok = awcc::activate(&mut self.fw(aml), &inv, id);
        serial_println!("[sexacpi.awcc.set] desktop={} firmware={:#04x} kind={} ok={}", p.name(), id, awcc::kind(id).map_or("unknown", awcc::kind_name), ok as u8);
    }

    /// Directly activates a specific AWCC thermal profile kind (from Spindle `profile <name>`).
    pub fn set_kind(&mut self, aml: &Aml, wanted: awcc::Kind) {
        let id = self.inventory.profiles().iter().copied().find(|&p| awcc::kind(p) == Some(wanted))
            .or_else(|| (wanted == awcc::Kind::GMode).then_some(awcc::PROFILE_GMODE));
        let Some(id) = id else {
            serial_println!("[sexacpi.awcc.set] kind={} firmware=none", awcc::kind_name(wanted));
            return;
        };
        let inv = self.inventory;
        let ok = awcc::activate(&mut self.fw(aml), &inv, id);
        serial_println!("[sexacpi.awcc.set] kind={} firmware={:#04x} ok={}", awcc::kind_name(wanted), id, ok as u8);
        self.reads = 0;
        self.refresh(aml);
    }
}

/// Dell hotkey events: which device and notify value carry them.
pub struct Hotkeys { dev: AmlName, notify: u8 }

impl Hotkeys {
    pub fn find(devices: &[Device]) -> Option<Self> {
        devices.iter().find_map(|d| d.block(&guids::DELL_EVENT).and_then(|b| b.notify_id()).map(|n| Self { dev: d.path.clone(), notify: n }))
    }
    /// For a `Notify(path, value)`: the keys it carried, when it is ours.
    pub fn take(&self, aml: &Aml, path: &AmlName, value: u64) -> Option<Vec<dell_wmi::Key>> {
        if path != &self.dev || value != self.notify as u64 { return None; }
        let data = bounded(|| eval(aml, &self.dev, "_WED", alloc::vec![WrappedObject::new(Object::Integer(value))]));
        let mut keys = [dell_wmi::Key::Other(0); 8];
        let n = match data.as_deref() {
            Some(Object::Buffer(b)) => dell_wmi::decode(b, &mut keys),
            _ => 0,
        };
        for k in &keys[..n] {
            let code = match k { dell_wmi::Key::Other(c) => *c as u32, dell_wmi::Key::Program(p) => *p as u32, _ => 0 };
            serial_println!("[sexacpi.hotkey] key={} code={:#06x} act={}", k.name(), code, k.actionable() as u8);
        }
        Some(keys[..n].to_vec())
    }
}

/// Dell battery charge modes through the SMBIOS calling interface (WMI method 1 of the
/// Dell SMBIOS GUID), with tokens from the SMBIOS table.
pub struct Charging { dev: AmlName, method: [u8; 4], size: usize, tokens: Vec<dell_smbios::Token>, pub state: dell_smbios::Charge }

struct DellCall<'a> { aml: &'a Aml, dev: &'a AmlName, method: [u8; 4], size: usize }

impl dell_smbios::Firmware for DellCall<'_> {
    fn call(&mut self, class: u16, select: u16, input: [u32; 4]) -> Option<[u32; 4]> {
        let name = core::str::from_utf8(&self.method).ok()?;
        let mut buf = alloc::vec![0u8; self.size];
        let n = dell_smbios::request(class, select, input, self.size, &mut buf);
        buf.truncate(n);
        let args = alloc::vec![
            WrappedObject::new(Object::Integer(0)),
            WrappedObject::new(Object::Integer(1)),
            WrappedObject::new(Object::Buffer(buf)),
        ];
        match bounded(|| eval(self.aml, self.dev, name, args)).as_deref() {
            Some(Object::Buffer(b)) => dell_smbios::answer(b),
            _ => None,
        }
    }
}

impl Charging {
    /// Logs the machine (SMBIOS type 0/1) and finds the charge-mode tokens and the Dell
    /// SMBIOS WMI method. `None` on machines without them.
    pub fn find(aml: &Aml, devices: &[Device], table: Option<&[u8]>) -> Option<Self> {
        let table = table?;
        let sys = smbios::system(table);
        let mut tokens = Vec::new();
        for st in smbios::structures(table).filter(|s| s.kind == 0xDA) { tokens.extend(dell_smbios::tokens(&st)); }
        serial_println!("[sexacpi.smbios] vendor={} product={} sku={} bios={} dell_tokens={} x17r1={}",
            sys.vendor.unwrap_or("?"), sys.product.unwrap_or("?"), sys.sku.unwrap_or("?"), sys.bios_version.unwrap_or("?"),
            tokens.len(), smbios::is_alienware_x17_r1(&sys) as u8);
        if !dell_smbios::MODES.iter().any(|(_, id)| tokens.iter().any(|t| t.id == *id)) { return None; }
        let (dev, block) = devices.iter().find_map(|d| d.block(&guids::DELL_SMBIOS).map(|b| (d, b)))?;
        let method = block.method_name()?;
        // The descriptor block names the buffer size; 4096 bytes is what Dell firmware uses.
        let size = devices.iter().find_map(|d| {
            let q = d.block(&guids::DELL_DESCRIPTOR)?.query_name()?;
            let name = core::str::from_utf8(&q).ok()?;
            match bounded(|| eval(aml, &d.path, name, alloc::vec![WrappedObject::new(Object::Integer(0))])).as_deref() {
                Some(Object::Buffer(b)) => dell_smbios::descriptor(b).map(|(_, size)| size as usize),
                _ => None,
            }
        });
        let found = size.is_some();
        let size = size.unwrap_or(4096).saturating_sub(8).max(36);
        let mut c = Self { dev: dev.path.clone(), method, size, tokens, state: dell_smbios::Charge::default() };
        c.refresh(aml);
        serial_println!("[sexacpi.charge] path={} method={} buffer={} descriptor={}", c.dev, core::str::from_utf8(&method).unwrap_or("?"), size, found as u8);
        Some(c)
    }

    fn fw<'a>(&'a self, aml: &'a Aml) -> DellCall<'a> { DellCall { aml, dev: &self.dev, method: self.method, size: self.size } }

    pub fn refresh(&mut self, aml: &Aml) {
        let tokens = core::mem::take(&mut self.tokens);
        self.state = dell_smbios::charge(&mut self.fw(aml), &tokens);
        self.tokens = tokens;
        let mut modes = alloc::string::String::new();
        for (m, _) in dell_smbios::MODES.iter().filter(|(m, _)| self.state.supports(*m)) { modes.push_str(m.name()); modes.push(','); }
        serial_println!("[sexacpi.charge.state] modes={} mode={} start={} end={}", modes.trim_end_matches(','),
            self.state.mode.map_or("unknown", |m| m.name()), self.state.start.map_or(-1, |v| v as i32), self.state.end.map_or(-1, |v| v as i32));
    }

    /// A `CTL_CHARGE` value: a mode, or 55..=100 for custom stopping at that percent.
    pub fn set(&mut self, aml: &Aml, value: u8) {
        let (mode, limits) = match value {
            55..=100 => (dell_smbios::Mode::Custom, Some((value.saturating_sub(10), value))),
            v => match dell_smbios::Mode::from_u8(v) { Some(m) => (m, None), None => return },
        };
        let tokens = core::mem::take(&mut self.tokens);
        let ok = self.state.supports(mode) && dell_smbios::set_mode(&mut self.fw(aml), &tokens, mode, limits);
        self.tokens = tokens;
        serial_println!("[sexacpi.charge.set] value={} mode={} ok={}", value, mode.name(), ok as u8);
        self.refresh(aml);
    }
}
