// SPDX-License-Identifier: GPL-3.0-only
//! The laptop half of the ACPI service: the embedded controller, firmware events,
//! batteries, backlight and thermal trips.
//!
//! Laptop firmware keeps the battery, the AC adapter and often the lid behind the embedded
//! controller (EC). Without an `EmbeddedControl` region handler every one of those reads
//! fails, so the battery looks absent. Here the EC is driven through the kernel's firmware
//! port window, `_REG` tells the firmware it is usable, and EC events (`SCI_EVT` in the
//! status register) run the firmware's `_Qxx` methods. Those methods `Notify` the lid,
//! battery, AC adapter or the panel (brightness keys); the notifications land here.
use crate::{bounded, eval, eval_int, int_of, io, name, Aml, Devices};
use acpi::aml::namespace::AmlName;
use acpi::aml::object::{Object, WrappedObject};
use acpi::aml::op_region::{OpRegion, RegionHandler, RegionSpace};
use acpi::aml::AmlError;
use alloc::vec::Vec;
use core::cell::RefCell;
use sex_laptop::{backlight, battery, ec, thermal};
use sex_pdx::power as w;
use sex_pdx::serial_println;

// ── Embedded controller ───────────────────────────────────────────────────

struct FwPorts { data: u16, command: u16 }
impl ec::Ports for FwPorts {
    fn status(&mut self) -> u8 { io(self.command, 1, None) as u8 }
    fn command(&mut self, v: u8) { io(self.command, 1, Some(v as u64)); }
    fn read_data(&mut self) -> u8 { io(self.data, 1, None) as u8 }
    fn write_data(&mut self, v: u8) { io(self.data, 1, Some(v as u64)); }
    fn pause(&mut self) { crate::stall_us(10) }
}

/// One task runs AML, so the controller is never shared across threads.
struct Single<T>(RefCell<Option<T>>);
unsafe impl<T> Sync for Single<T> {}
static EC: Single<ec::Ec<FwPorts>> = Single(RefCell::new(None));
static EC_ERRORS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn with_ec<R>(f: impl FnOnce(&mut ec::Ec<FwPorts>) -> Result<R, ec::Error>) -> Result<R, AmlError> {
    let mut slot = EC.0.borrow_mut();
    let Some(ec) = slot.as_mut() else { return Err(AmlError::NoHandlerForRegionAccess(RegionSpace::EmbeddedControl)) };
    f(ec).map_err(|e| {
        if EC_ERRORS.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 8 { serial_println!("[sexacpi.ec.error] {:?}", e); }
        AmlError::NoHandlerForRegionAccess(RegionSpace::EmbeddedControl)
    })
}

struct EcRegion;
impl EcRegion {
    fn read(region: &OpRegion, bytes: usize) -> Result<u64, AmlError> {
        crate::spend(bytes as u64 * 4);
        with_ec(|ec| ec.read_wide(region.base as u8, bytes))
    }
    fn write(region: &OpRegion, bytes: usize, v: u64) -> Result<(), AmlError> {
        crate::spend(bytes as u64 * 4);
        with_ec(|ec| ec.write_wide(region.base as u8, bytes, v))
    }
}
impl RegionHandler for EcRegion {
    fn read_u8(&self, r: &OpRegion) -> Result<u8, AmlError> { Self::read(r, 1).map(|v| v as u8) }
    fn read_u16(&self, r: &OpRegion) -> Result<u16, AmlError> { Self::read(r, 2).map(|v| v as u16) }
    fn read_u32(&self, r: &OpRegion) -> Result<u32, AmlError> { Self::read(r, 4).map(|v| v as u32) }
    fn read_u64(&self, r: &OpRegion) -> Result<u64, AmlError> { Self::read(r, 8) }
    fn write_u8(&self, r: &OpRegion, v: u8) -> Result<(), AmlError> { Self::write(r, 1, v as u64) }
    fn write_u16(&self, r: &OpRegion, v: u16) -> Result<(), AmlError> { Self::write(r, 2, v as u64) }
    fn write_u32(&self, r: &OpRegion, v: u32) -> Result<(), AmlError> { Self::write(r, 4, v as u64) }
    fn write_u64(&self, r: &OpRegion, v: u64) -> Result<(), AmlError> { Self::write(r, 8, v) }
}

/// Finds the EC device (`PNP0C09`), takes its ports from `_CRS`, and installs the region
/// handler. Runs after the tables load and before `_INI`, like ACPICA's early EC.
pub fn ec_install(aml: &Aml, all: &[AmlName]) -> Option<AmlName> {
    let dev = all.iter().find(|p| crate::ids_of(aml, p).iter().any(|i| i == "PNP0C09"))?.clone();
    let crs = eval(aml, &dev, "_CRS", Vec::new());
    let ports = crs.as_ref().and_then(|o| if let Object::Buffer(b) = &**o { sex_laptop::resources::ec_ports(b) } else { None });
    let (data, command) = ports.unwrap_or((ec::DATA_PORT, ec::COMMAND_PORT));
    *EC.0.borrow_mut() = Some(ec::Ec::new(FwPorts { data, command }));
    aml.install_region_handler(RegionSpace::EmbeddedControl, EcRegion);
    serial_println!("[sexacpi.ec] path={} data={:#x} command={:#x} from_crs={}", dev, data, command, ports.is_some() as u8);
    Some(dev)
}

/// `_REG(EmbeddedControl, connect)`: most laptop firmware reads the battery from the EC
/// only once this ran.
pub fn ec_connect(aml: &Aml, dev: &AmlName) {
    let args = alloc::vec![WrappedObject::new(Object::Integer(3)), WrappedObject::new(Object::Integer(1))];
    let ran = eval(aml, dev, "_REG", args).is_some();
    serial_println!("[sexacpi.ec.reg] path={} ran={}", dev, ran as u8);
}

/// Runs the `_Qxx` method for each event the EC has waiting (at most a few per pass).
pub fn ec_events(aml: &Aml, dev: &AmlName) -> usize {
    let mut handled = 0;
    for _ in 0..8 {
        let pending = { let mut slot = EC.0.borrow_mut(); slot.as_mut().map_or(false, |ec| ec.event_pending()) };
        if !pending { break; }
        let Ok(q) = with_ec(|ec| ec.query()) else { break };
        if q == 0 { break; }
        let method = ec::query_method(q);
        let method = core::str::from_utf8(&method).unwrap_or("_Q00");
        let ran = bounded(|| eval(aml, dev, method, Vec::new()).is_some());
        serial_println!("[sexacpi.ec.event] query={:#04x} method={} ran={}", q, method, ran as u8);
        handled += 1;
    }
    handled
}

// ── Notify ────────────────────────────────────────────────────────────────

static NOTIFIED: Single<Vec<(WrappedObject, u64)>> = Single(RefCell::new(None));
/// Called by the interpreter for `Notify(object, value)`.
pub fn on_notify(target: &WrappedObject, value: u64) {
    let mut q = NOTIFIED.0.borrow_mut();
    let q = q.get_or_insert_with(Vec::new);
    if q.len() < 32 { q.push((target.clone(), value)); }
}
fn take_notified() -> Vec<(WrappedObject, u64)> { NOTIFIED.0.borrow_mut().take().unwrap_or_default() }

/// The namespace path of a device object, by address.
fn path_of(aml: &Aml, target: &WrappedObject) -> Option<AmlName> {
    let want = &**target as *const Object;
    let mut found = None;
    let mut ns = aml.namespace.lock().clone();
    let _ = ns.traverse(|path, level| {
        for (seg, (_, obj)) in level.values.iter() {
            if &**obj as *const Object == want { found = AmlName::from_name_seg(*seg).resolve(path).ok(); }
        }
        Ok(found.is_none())
    });
    found
}

/// What a firmware notification asks of us.
pub enum Event { Refresh, Brightness(bool), None }

pub fn notifications(aml: &Aml, d: &Devices, light: &Backlight) -> Vec<Event> {
    let mut out = Vec::new();
    for (target, value) in take_notified() {
        let path = path_of(aml, &target);
        let kind = match &path {
            Some(p) if d.lids.contains(p) => "lid",
            Some(p) if d.batteries.contains(p) => "battery",
            Some(p) if d.ac.contains(p) => "ac",
            Some(p) if d.thermal.contains(p) => "thermal",
            Some(p) if light.output.as_ref() == Some(p) || light.output.as_ref().and_then(|o| o.parent().ok()).as_ref() == Some(p) => "backlight",
            _ => "other",
        };
        serial_println!("[sexacpi.notify] path={} value={:#x} kind={}", path.as_ref().map_or(alloc::string::String::from("?"), |p| p.as_string()), value, kind);
        out.push(match (kind, value) {
            ("backlight", backlight::NOTIFY_UP) => Event::Brightness(true),
            ("backlight", backlight::NOTIFY_DOWN) => Event::Brightness(false),
            ("lid" | "battery" | "ac" | "thermal", _) => Event::Refresh,
            _ => Event::None,
        });
    }
    out
}

// ── Backlight (ACPI video `_BCL`/`_BCM`/`_BQC`) ─────────────────────────────

#[derive(Default)]
pub struct Backlight { pub output: Option<AmlName>, levels: Vec<u8>, pub current: Option<u8> }

impl Backlight {
    /// The first display output whose `_BCL` lists levels.
    pub fn find(aml: &Aml, all: &[AmlName]) -> Self {
        for dev in all {
            let Some(list) = eval(aml, dev, "_BCL", Vec::new()) else { continue };
            let Object::Package(items) = &*list else { continue };
            let raw: Vec<u64> = items.iter().filter_map(int_of).collect();
            let mut buf = [0u8; 101];
            let n = backlight::levels(&raw, &mut buf);
            if n == 0 { continue; }
            let current = eval_int(aml, dev, "_BQC").filter(|v| *v <= 100).map(|v| v as u8);
            serial_println!("[sexacpi.backlight] path={} levels={} current={}", dev, n, current.map_or(-1, |c| c as i32));
            return Self { output: Some(dev.clone()), levels: buf[..n].to_vec(), current };
        }
        Self::default()
    }
    fn set(&mut self, aml: &Aml, level: u8, why: &str) {
        let Some(dev) = self.output.clone() else { return };
        // `_BCM` returns nothing, so success is "the method exists and ran".
        let ok = bounded(|| name("_BCM").and_then(|n| n.resolve(&dev).ok())
            .map_or(false, |p| aml.evaluate_if_present(p, alloc::vec![WrappedObject::new(Object::Integer(level as u64))]).map_or(false, |r| r.is_some())));
        let read = bounded(|| eval_int(aml, &dev, "_BQC")).filter(|v| *v <= 100).map(|v| v as u8);
        self.current = read.or(Some(level));
        serial_println!("[sexacpi.backlight.set] level={} by={} bcm={} bqc={}", level, why, ok as u8, read.map_or(-1, |v| v as i32));
    }
    pub fn step(&mut self, aml: &Aml, up: bool, why: &str) {
        let current = self.current.unwrap_or(*self.levels.last().unwrap_or(&100));
        if let Some(l) = backlight::step(&self.levels, current, up) { self.set(aml, l, why); }
    }
    pub fn to_percent(&mut self, aml: &Aml, percent: u8) {
        if let Some(l) = backlight::nearest(&self.levels, percent) { self.set(aml, l, "settings"); }
    }
    pub fn refresh(&mut self, aml: &Aml) {
        if let Some(dev) = &self.output {
            if let Some(v) = eval_int(aml, dev, "_BQC").filter(|v| *v <= 100) { self.current = Some(v as u8); }
        }
    }
}

/// Drains brightness requests the desktop queued through the kernel.
pub fn controls(aml: &Aml, light: &mut Backlight) -> bool {
    let mut any = false;
    for _ in 0..8 {
        let r = w::call4(w::FW_CONTROL, 0, 0, 0);
        if r[0] != w::OK { break; }
        let value = (r[1] >> 8) as u8;
        match r[1] & 0xFF {
            w::CTL_BRIGHTNESS_STEP => light.step(aml, value == 1, "desktop"),
            w::CTL_BRIGHTNESS_SET => light.to_percent(aml, value),
            w::CTL_SLEEP => sleep(aml, light, value == 1),
            _ => {}
        }
        any = true;
    }
    any
}

// ── Suspend to idle ────────────────────────────────────────────────────────

/// Intel's and Microsoft's low-power S0 idle `_DSM`s on the power engine plug-in device
/// (`INT33A1` / `PNP0D80`). Functions: 3 screen off, 4 screen on, 5 entry, 6 exit (and
/// Microsoft's 7/8 modern-standby entry/exit). Firmware uses them to turn off the keyboard
/// light, slow the fans and let the platform reach its deepest idle state.
const LPS0_INTEL: [u8; 16] = [0xA0, 0x40, 0xEB, 0xC4, 0xD2, 0x6C, 0xE2, 0x11, 0xBC, 0xFD, 0x08, 0x00, 0x20, 0x0C, 0x9A, 0x66];
const LPS0_MICROSOFT: [u8; 16] = [0x56, 0x0D, 0xE0, 0x11, 0x64, 0xCE, 0xCE, 0x47, 0x83, 0x7B, 0x1F, 0x89, 0x8F, 0x9A, 0xA4, 0x61];

fn dsm(aml: &Aml, dev: &AmlName, uuid: &[u8; 16], revision: u64, function: u64) -> Option<WrappedObject> {
    let args = alloc::vec![
        WrappedObject::new(Object::Buffer(uuid.to_vec())),
        WrappedObject::new(Object::Integer(revision)),
        WrappedObject::new(Object::Integer(function)),
        WrappedObject::new(Object::Package(Vec::new())),
    ];
    eval(aml, dev, "_DSM", args)
}
/// Function bits a `_DSM` supports (function 0's answer).
fn dsm_mask(aml: &Aml, dev: &AmlName, uuid: &[u8; 16], revision: u64) -> u64 {
    match dsm(aml, dev, uuid, revision, 0).as_deref() {
        Some(Object::Buffer(b)) => b.iter().take(8).enumerate().fold(0, |m, (i, v)| m | (*v as u64) << (8 * i)),
        Some(Object::Integer(v)) => *v,
        _ => 0,
    }
}

static SLEEP_SAVED: Single<u8> = Single(RefCell::new(None));

fn sleep(aml: &Aml, light: &mut Backlight, enter: bool) {
    let all = crate::acpi_devices(aml);
    let dev = all.iter().find(|p| crate::ids_of(aml, p).iter().any(|i| i == "INT33A1" || i == "PNP0D80")).cloned();
    let mut ran = alloc::string::String::new();
    if let Some(dev) = &dev {
        for (uuid, revision, name, steps) in [(&LPS0_INTEL, 0u64, "intel", if enter { [3u64, 5] } else { [6, 4] }),
                                              (&LPS0_MICROSOFT, 0, "microsoft", if enter { [3, 7] } else { [8, 4] })] {
            let mask = dsm_mask(aml, dev, uuid, revision);
            for f in steps {
                if mask & 1 << f != 0 && dsm(aml, dev, uuid, revision, f).is_some() {
                    ran.push_str(&alloc::format!("{name}:{f} "));
                }
            }
        }
    }
    // The panel: down to its lowest level while asleep, back after.
    if enter {
        *SLEEP_SAVED.0.borrow_mut() = light.current;
        if light.output.is_some() { light.to_percent(aml, 0); }
    } else if let Some(level) = SLEEP_SAVED.0.borrow_mut().take() {
        light.to_percent(aml, level);
    }
    serial_println!("[sexacpi.sleep] enter={} lps0={} ran={} panel={}", enter as u8, dev.as_ref().map_or(alloc::string::String::from("none"), |d| alloc::format!("{d}")),
        ran.trim_end(), light.current.map_or(-1, |v| v as i32));
}

// ── Sensors ────────────────────────────────────────────────────────────────

fn integers(o: &Option<WrappedObject>) -> Option<Vec<Option<u64>>> {
    let o = o.as_ref()?;
    let Object::Package(items) = &**o else { return None };
    Some(items.iter().map(int_of).collect())
}

pub fn read(aml: &Aml, d: &Devices, light: &Backlight) -> w::Sensors {
    let mut s = w::Sensors::default();
    let mut raws = Vec::new();
    for b in d.batteries.iter() {
        if eval_int(aml, b, "_STA").map_or(false, |v| v & 0x10 == 0) { continue; }
        let bst = integers(&eval(aml, b, "_BST", Vec::new()));
        let (info, bix) = match integers(&eval(aml, b, "_BIX", Vec::new())) { Some(p) => (Some(p), true), None => (integers(&eval(aml, b, "_BIF", Vec::new())), false) };
        if let (Some(bst), Some(info)) = (bst, info) {
            if let Some(r) = battery::Raw::from_packages(&bst, &info, bix) { raws.push(r); }
        }
    }
    if let Some(ac) = d.ac.first() { s.on_ac = eval_int(aml, ac, "_PSR").map(|v| v != 0); }
    if let Some(r) = battery::combine(&raws) {
        s.battery_percent = Some(r.percent);
        s.charging = r.charging;
        s.battery_minutes = r.minutes;
        s.battery_health = r.health;
        s.battery_critical = battery::must_shut_down(&r, s.on_ac);
    }
    if let Some(lid) = d.lids.first() { s.lid_open = eval_int(aml, lid, "_LID").map(|v| v != 0); }
    let mut hottest: Option<(u64, Option<u64>)> = None;
    for t in d.thermal.iter() {
        let Some(tmp) = eval_int(aml, t, "_TMP").filter(|v| thermal::plausible(*v)) else { continue };
        let crt = eval_int(aml, t, "_CRT").filter(|v| thermal::plausible(*v));
        if thermal::critical(Some(tmp), crt) { s.thermal_critical = true; }
        if hottest.map_or(true, |(h, _)| tmp > h) { hottest = Some((tmp, crt)); }
    }
    if let Some((t, crt)) = hottest {
        s.hottest_decikelvin = Some(t as u32);
        s.critical_decikelvin = crt.map(|c| c as u16);
    }
    s.brightness = light.current;
    s
}
