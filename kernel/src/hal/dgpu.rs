// SPDX-License-Identifier: GPL-3.0-only
//! Discrete GPUs that nothing drives yet: name them, then power them off (D3cold).
//!
//! A gaming laptop's NVIDIA GPU (the Alienware x17 R1's RTX 30 laptop part, for one)
//! draws power and makes heat from the moment the machine starts, even when the Intel
//! GPU drives the panel. Until a SexOS driver owns it (GSP firmware boot, stage N2 in
//! docs/handoff/NVIDIA_GPU_PLAN_2026_11_09.md), BOOT turns it off the way Linux runtime
//! power management does:
//!
//! 1. every function in the GPU's slot goes to D3hot through its PCI PM registers;
//! 2. the GPU's ACPI node runs `_PS3`, then `_OFF` for the power resources its
//!    `_PR0`..`_PR3` name (skipping any resource another device needs in D0);
//! 3. the root port above it, when the GPU is all that is behind it, does the same.
//!
//! On Ampere and later laptops step 3 is what cuts power: the firmware's power resource
//! lives on the root port (`\_SB.PC00.PEG1` with `_PR3 = {PG01}` and the like). Afterwards
//! the GPU no longer answers configuration reads.
//!
//! What stays on: a GPU on the root bus (an integrated GPU), a GPU that holds the boot
//! framebuffer (BIOS "discrete only" mode or a mux set to the dGPU), and anything when
//! AML is unusable (then only D3hot, which every PCI device must accept).
//!
//! Firmware AML runs quarantined and with a work budget: a `_OFF` that waits forever
//! on a link bit ends the attempt instead of the boot.
//!
//! `power_on` reverses it and restores each function's configuration header, for the
//! driver that will come later; a QEMU gate exercises it (see `GATE_NAME`).

use super::acpi::{eval, eval_int, ids_of, int_of, stall_us, with_aml, Hw};
use crate::hal::pci::{pci_config_read, pci_config_write};
use crate::serial_println;
use acpi::aml::namespace::{AmlName, NamespaceLevelKind};
use acpi::aml::object::Object;
use acpi::aml::Interpreter;
use alloc::string::String;
use alloc::vec::Vec;
use core::str::FromStr;

/// AML work allowed for one power change: about three seconds of stalls and register access.
const BUDGET: u64 = 3_000_000;
/// A firmware name that marks a test GPU: when the GPU's ACPI node has it, BOOT also
/// powers the GPU back on and off again, so the gate checks `power_on` too. Real firmware
/// never defines it.
const GATE_NAME: &str = "SXGT";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bdf { pub bus: u8, pub dev: u8, pub func: u8 }
impl core::fmt::Display for Bdf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, "{:02x}:{:02x}.{}", self.bus, self.dev, self.func) }
}
fn rd(b: Bdf, at: u8) -> u32 { unsafe { pci_config_read(b.bus, b.dev, b.func, at & 0xFC) } }
fn wr(b: Bdf, at: u8, v: u32) { unsafe { pci_config_write(b.bus, b.dev, b.func, at & 0xFC, v) } }
fn exists(b: Bdf) -> bool { rd(b, 0) & 0xFFFF != 0xFFFF }

/// The PCI power management capability's offset.
fn pm_cap(b: Bdf) -> Option<u8> {
    if (rd(b, 0x04) >> 16) & 0x10 == 0 { return None; }
    let mut at = (rd(b, 0x34) & 0xFC) as u8;
    for _ in 0..48 {
        if at < 0x40 { return None; }
        let v = rd(b, at);
        if v & 0xFF == 0x01 { return Some(at); }
        at = ((v >> 8) & 0xFC) as u8;
    }
    None
}
/// Sets D0 (0) or D3hot (3); returns the state it was in. PME status (bit 15) is
/// write-one-to-clear, so it is never written back.
fn set_pm_state(b: Bdf, state: u32) -> Option<u32> {
    let cap = pm_cap(b)?;
    let csr = rd(b, cap + 4);
    wr(b, cap + 4, (csr & 0x7FFC) | state);
    Some(csr & 3)
}
fn pm_state(b: Bdf) -> Option<u32> { pm_cap(b).map(|c| rd(b, c + 4) & 3) }

fn is_bridge(b: Bdf) -> bool { (rd(b, 0x0C) >> 16) & 0x7F == 1 }
fn secondary_bus(b: Bdf) -> u8 { (rd(b, 0x18) >> 8) as u8 }
fn functions(bus: u8, dev: u8) -> Vec<Bdf> {
    if !exists(Bdf { bus, dev, func: 0 }) { return Vec::new(); }
    let multi = (rd(Bdf { bus, dev, func: 0 }, 0x0C) >> 16) & 0x80 != 0;
    (0..if multi { 8 } else { 1 }).map(|func| Bdf { bus, dev, func }).filter(|b| exists(*b)).collect()
}

/// Is physical address `pa` inside the memory a bridge forwards to its secondary side?
fn bridge_forwards(b: Bdf, pa: u64) -> bool {
    let m = rd(b, 0x20);
    let (base, limit) = (((m & 0xFFF0) as u64) << 16, (m & 0xFFF0_0000) as u64 | 0xF_FFFF);
    if base <= limit && (base..=limit).contains(&pa) { return true; }
    let p = rd(b, 0x24);
    let wide = p & 0xF == 1;
    let base = ((p & 0xFFF0) as u64) << 16 | if wide { (rd(b, 0x28) as u64) << 32 } else { 0 };
    let limit = ((p & 0xFFF0_0000) as u64 | 0xF_FFFF) | if wide { (rd(b, 0x2C) as u64) << 32 } else { 0 };
    base <= limit && (base..=limit).contains(&pa)
}

/// The GPU's chip, from its own PMC_BOOT_0 register (BAR0 + 0). Read only when BAR0 is
/// below 4 GiB, memory decode is on and the kernel's direct map covers it.
fn nvidia_chip(gpu: Bdf, hhdm: u64) -> Option<(u32, &'static str)> {
    let bar = rd(gpu, 0x10);
    if bar & 1 != 0 || (bar >> 1) & 3 == 2 && rd(gpu, 0x14) != 0 { return None; }
    let pa = (bar & 0xFFFF_FFF0) as u64;
    if pa == 0 || rd(gpu, 0x04) & 2 == 0 { return None; }
    let va = hhdm + pa;
    let flags = crate::memory::manager::read_pte_flags(x86_64::VirtAddr::new(va)).ok()?;
    if flags & 1 == 0 { return None; }
    let boot0 = unsafe { core::ptr::read_volatile(va as *const u32) };
    if boot0 == u32::MAX || boot0 == 0 { return None; }
    let chip = (boot0 >> 20) & 0x1FF;
    let label = match chip {
        0x162 => "TU102", 0x164 => "TU104", 0x166 => "TU106", 0x167 => "TU117", 0x168 => "TU116",
        0x170 => "GA100", 0x172 => "GA102", 0x173 => "GA103", 0x174 => "GA104", 0x176 => "GA106", 0x177 => "GA107",
        0x192 => "AD102", 0x193 => "AD103", 0x194 => "AD104", 0x196 => "AD106", 0x197 => "AD107",
        0x1B2 => "GB202", 0x1B3 => "GB203", 0x1B5 => "GB205", 0x1B6 => "GB206", 0x1B7 => "GB207",
        _ => "unknown",
    };
    Some((chip, label))
}
fn product(vendor: u16, device: u16) -> &'static str {
    match (vendor, device) {
        (0x10DE, 0x2520 | 0x2560 | 0x2563) => "GeForce RTX 3060 Laptop",
        (0x10DE, 0x249D | 0x24DD) => "GeForce RTX 3070 Laptop",
        (0x10DE, 0x249C | 0x24DC) => "GeForce RTX 3080 Laptop",
        (0x10DE, 0x2420 | 0x2460) => "GeForce RTX 3080 Ti Laptop",
        (0x10DE, 0x2204) => "GeForce RTX 3090",
        (0x10DE, _) => "NVIDIA GPU",
        (0x1002, _) => "AMD GPU",
        (0x8086, _) => "Intel GPU",
        _ => "display controller",
    }
}

// ── The ACPI side ─────────────────────────────────────────────────────────

/// ACPI `_ADR` of a PCI function: device in the high word, function in the low.
fn adr_matches(adr: u64, b: Bdf) -> bool {
    (adr >> 16) as u8 == b.dev && (adr & 0xFFFF == b.func as u64 || adr & 0xFFFF == 0xFFFF)
}
/// Every ACPI device, with the parent's path beside it.
fn devices(aml: &Interpreter<Hw>) -> Vec<AmlName> {
    let mut found = Vec::new();
    let mut ns = aml.namespace.lock().clone();
    let _ = ns.traverse(|path, level| { if matches!(level.kind, NamespaceLevelKind::Device) { found.push(path.clone()); } Ok(true) });
    found.truncate(4096);
    found
}
fn parent_is(path: &AmlName, parent: &AmlName) -> bool { path.parent().map_or(false, |p| &p == parent) }

/// The ACPI nodes for a chain of PCI functions from the root bus down (root port first).
/// Firmware (and QEMU) may describe one slot twice; the description that reaches the GPU
/// and names power resources wins.
fn nodes(aml: &Interpreter<Hw>, all: &[AmlName], root_bus: u8, chain: &[Bdf]) -> Option<Vec<AmlName>> {
    fn walk(aml: &Interpreter<Hw>, all: &[AmlName], at: &AmlName, chain: &[Bdf]) -> Option<Vec<AmlName>> {
        let Some((hop, rest)) = chain.split_first() else { return Some(Vec::new()) };
        let mut best: Option<Vec<AmlName>> = None;
        for p in all.iter().filter(|p| parent_is(p, at)) {
            if !eval_int(aml, p, "_ADR").map_or(false, |a| adr_matches(a, *hop)) { continue; }
            let Some(tail) = walk(aml, all, p, rest) else { continue };
            let mut path = alloc::vec![p.clone()];
            path.extend(tail);
            let powered = path.iter().any(|n| !resources(aml, n).is_empty());
            if powered { return Some(path); }
            best.get_or_insert(path);
        }
        best
    }
    all.iter().filter(|p| {
        let ids = ids_of(aml, p);
        ids.iter().any(|i| i == "PNP0A08" || i == "PNP0A03") && eval_int(aml, p, "_BBN").unwrap_or(0) as u8 == root_bus
    }).find_map(|host| walk(aml, all, host, chain))
}

/// The power resources a device's `_PR0`..`_PR3` name, as full paths, first-listed first.
fn resources(aml: &Interpreter<Hw>, dev: &AmlName) -> Vec<AmlName> {
    let mut out: Vec<AmlName> = Vec::new();
    for m in ["_PR0", "_PR1", "_PR2", "_PR3"] {
        let Some(list) = eval(aml, dev, m, Vec::new()) else { continue };
        let Object::Package(items) = &*list else { continue };
        for item in items.iter() {
            let Object::String(s) = &**item else { continue };
            let Ok(n) = AmlName::from_str(s) else { continue };
            let Ok((full, obj)) = aml.namespace.lock().search(&n, dev) else { continue };
            if matches!(*obj, Object::PowerResource { .. }) && !out.contains(&full) { out.push(full); }
        }
    }
    out
}
/// Resources some other device (outside `ours`) needs in D0.
fn needed_elsewhere(aml: &Interpreter<Hw>, all: &[AmlName], ours: &[AmlName], res: &AmlName) -> bool {
    all.iter().filter(|d| !ours.iter().any(|o| *d == o || d.as_string().starts_with(&(o.as_string() + "."))))
        .any(|d| {
            let Some(list) = eval(aml, d, "_PR0", Vec::new()) else { return false };
            let Object::Package(items) = &*list else { return false };
            items.iter().any(|i| match &**i {
                Object::String(s) => AmlName::from_str(s).ok().and_then(|n| aml.namespace.lock().search(&n, d).ok()).map_or(false, |(f, _)| &f == res),
                _ => false,
            })
        })
}
fn resource_on(aml: &Interpreter<Hw>, r: &AmlName) -> Option<bool> { eval_int(aml, r, "_STA").map(|s| s & 1 != 0) }

/// One device's ACPI part of going to D3cold: `_PS3`, then its unshared resources off.
fn acpi_off(aml: &Interpreter<Hw>, all: &[AmlName], ours: &[AmlName], dev: &AmlName, log: &mut String) -> usize {
    use core::fmt::Write;
    if eval(aml, dev, "_PS3", Vec::new()).is_some() { let _ = write!(log, " {}._PS3", dev); }
    let mut off = 0;
    for r in resources(aml, dev).iter().rev() {
        if needed_elsewhere(aml, all, ours, r) { let _ = write!(log, " {}=shared", r); continue; }
        if resource_on(aml, r) == Some(false) { let _ = write!(log, " {}=already_off", r); off += 1; continue; }
        eval(aml, r, "_OFF", Vec::new());
        let sta = resource_on(aml, r);
        let _ = write!(log, " {}._OFF sta={}", r, match sta { Some(true) => "1", Some(false) => "0", None => "none" });
        if sta != Some(true) { off += 1; }
    }
    off
}
fn acpi_on(aml: &Interpreter<Hw>, dev: &AmlName) {
    for r in resources(aml, dev) { if resource_on(aml, &r) != Some(true) { eval(aml, &r, "_ON", Vec::new()); } }
    eval(aml, dev, "_PS0", Vec::new());
}

// ── State kept for `power_on` ─────────────────────────────────────────────

struct Off {
    gpu: Bdf,
    port: Option<Bdf>,
    /// ACPI nodes: the GPU's, and the root port's when it was powered off too.
    gpu_node: Option<AmlName>,
    port_node: Option<AmlName>,
    /// Each function's configuration header (0x04..0x40), restored after power returns.
    saved: Vec<(Bdf, [u32; 16])>,
    port_saved: Option<[u32; 16]>,
    cold: bool,
}
static OFF: spin::Mutex<Vec<Off>> = spin::Mutex::new(Vec::new());

fn save(b: Bdf) -> [u32; 16] { let mut h = [0u32; 16]; for i in 1..16 { h[i] = rd(b, (i * 4) as u8); } h }
fn restore(b: Bdf, h: &[u32; 16]) {
    // BARs and the rest first, the command register (with its W1C status half cleared) last.
    for i in 2..16 { if i != 3 { wr(b, (i * 4) as u8, h[i]); } }
    wr(b, 0x0C, h[3] & 0x0000_FFFF);
    wr(b, 0x04, h[1] & 0xFFFF);
}

struct Found { at: Bdf, vendor: u16, device: u16, class: u16 }
/// Config space, read only: the HAL's enumeration also runs boot NIC diagnostics.
fn scan() -> Vec<Found> {
    let mut out = Vec::new();
    for bus in 0..=255u8 {
        for dev in 0..32u8 {
            for at in functions(bus, dev) {
                let id = rd(at, 0);
                out.push(Found { at, vendor: id as u16, device: (id >> 16) as u16, class: (rd(at, 0x08) >> 16) as u16 });
            }
        }
    }
    out
}

pub fn powered_off(b: Bdf) -> bool { OFF.lock().iter().any(|o| o.gpu.bus == b.bus && o.gpu.dev == b.dev) }

/// BOOT: find discrete GPUs and power off the ones nothing will drive.
pub fn init() {
    let Some(hhdm) = crate::HHDM_REQUEST.response().map(|r| r.offset) else { return };
    let fb = crate::FB_REQUEST.response().and_then(|r| r.framebuffers().iter().next().map(|f| (f.address() as u64).wrapping_sub(hhdm)));
    let all = scan();
    for d in all.iter().filter(|d| d.class >> 8 == 0x03 && d.at.func == 0) {
        let gpu = d.at;
        let name = product(d.vendor, d.device);
        let chip = if d.vendor == 0x10DE { nvidia_chip(gpu, hhdm) } else { None };
        serial_println!("[gpu.found] at={} vendor={:04x} device={:04x} class={:04x} name=\"{}\" chip={}", gpu, d.vendor, d.device, d.class,
            name, chip.map_or(String::from("unread"), |(c, l)| alloc::format!("{}:{:#x}", l, c)));
        // The bridges above it, root port first.
        let mut chain = Vec::new();
        let mut bus = gpu.bus;
        while chain.len() < 8 {
            let Some(up) = all.iter().find(|b| b.class == 0x0604 && is_bridge(b.at) && secondary_bus(b.at) == bus).map(|b| b.at) else { break };
            chain.insert(0, up);
            bus = up.bus;
        }
        if chain.is_empty() { serial_println!("[gpu.power] at={} action=keep reason=integrated", gpu); continue; }
        let port = *chain.last().unwrap();
        if let Some(pa) = fb.filter(|pa| bridge_forwards(port, *pa)) {
            serial_println!("[gpu.power] at={} action=keep reason=boot_framebuffer fb={:#x}", gpu, pa); continue;
        }
        power_off(gpu, chain, port, name, true);
    }
}

fn power_off(gpu: Bdf, chain: Vec<Bdf>, port: Bdf, label: &str, first: bool) {
    let root_bus = chain[0].bus;
    let funcs = functions(gpu.bus, gpu.dev);
    let saved: Vec<(Bdf, [u32; 16])> = funcs.iter().map(|f| (*f, save(*f))).collect();
    // The root port goes cold too only when this GPU is all that is behind it.
    let port_alone = (0..32u8).all(|dev| dev == gpu.dev || !exists(Bdf { bus: gpu.bus, dev, func: 0 }));
    let port_saved = port_alone.then(|| save(port));
    for f in &funcs { set_pm_state(*f, 3); }
    stall_us(10_000); // D0 -> D3hot needs 10 ms before the next access.
    let mut path = chain.clone();
    path.push(gpu);
    let result = with_aml(BUDGET, |aml| {
        let all = devices(aml);
        let found = nodes(aml, &all, root_bus, &path)?;
        let gpu_node = found.last().cloned()?;
        let port_node = found.get(found.len().wrapping_sub(2)).cloned();
        let ours: Vec<AmlName> = [Some(gpu_node.clone()), if port_alone { port_node.clone() } else { None }].into_iter().flatten().collect();
        let mut log = String::new();
        let mut off = acpi_off(aml, &all, &ours, &gpu_node, &mut log);
        if port_alone {
            set_pm_state(port, 3);
            stall_us(10_000);
            if let Some(p) = &port_node { off += acpi_off(aml, &all, &ours, p, &mut log); }
        }
        let gate = eval(aml, &gpu_node, GATE_NAME, Vec::new()).and_then(|o| int_of(&o)).unwrap_or(0) != 0;
        Some((gpu_node, port_node.filter(|_| port_alone), off, log, gate))
    });
    let Some(Some((gpu_node, port_node, off, log, gate))) = result else {
        // No ACPI path to the power: D3hot is still a large saving and every device accepts it.
        serial_println!("[gpu.power] at={} name=\"{}\" action=off state=d3hot acpi={} resources_off=0", gpu, label,
            if result.is_none() { "unavailable" } else { "no_node" });
        OFF.lock().push(Off { gpu, port: None, gpu_node: None, port_node: None, saved, port_saved: None, cold: false });
        return;
    };
    let gone = !exists(gpu);
    let state = if gone { "d3cold" } else if off > 0 { "d3cold_requested" } else { "d3hot" };
    serial_println!("[gpu.power] at={} name=\"{}\" action=off state={} gpu_node={} port={} port_node={} resources_off={} config_gone={} steps:{}",
        gpu, label, state, gpu_node, port, port_node.as_ref().map_or(String::from("kept_on"), |p| p.as_string()), off, gone as u8, log);
    OFF.lock().push(Off { gpu, port: port_alone.then_some(port), gpu_node: Some(gpu_node), port_node, saved: saved.clone(), port_saved, cold: off > 0 });
    if gate && first {
        let on = power_on(gpu);
        let back = on && funcs.iter().all(|f| pm_state(*f).map_or(true, |s| s == 0)) && rd(gpu, 0x10) == saved[0].1[4];
        serial_println!("[gpu.power.cycle] at={} on={} restored={}", gpu, on as u8, back as u8);
        if on { power_off(gpu, chain, port, label, false); }
    }
}

/// Returns a GPU that `init` powered off to D0 with its configuration restored. For the
/// driver that will own it; false if it was never off or does not come back.
pub fn power_on(gpu: Bdf) -> bool {
    let Some(i) = OFF.lock().iter().position(|o| o.gpu == gpu) else { return false };
    let o = OFF.lock().remove(i);
    if o.gpu_node.is_some() || o.port_node.is_some() {
        with_aml(BUDGET, |aml| {
            if let Some(p) = &o.port_node { acpi_on(aml, p); }
            if let Some(g) = &o.gpu_node { acpi_on(aml, g); }
        });
    }
    if let (Some(port), Some(h)) = (o.port, o.port_saved.as_ref()) {
        set_pm_state(port, 0);
        stall_us(10_000);
        restore(port, h);
    }
    // After D3cold the link retrains: up to a second before the device answers.
    let mut waited = 0;
    while !exists(gpu) && waited < 1_000 { stall_us(1_000); waited += 1; }
    if !exists(gpu) { serial_println!("[gpu.power] at={} action=on ok=0 reason=no_answer", gpu); return false; }
    for (f, h) in &o.saved {
        set_pm_state(*f, 0);
        stall_us(10_000);
        restore(*f, h);
    }
    serial_println!("[gpu.power] at={} action=on ok=1 cold={} waited_ms={}", gpu, o.cold as u8, waited);
    true
}
