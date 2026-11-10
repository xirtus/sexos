// SPDX-License-Identifier: GPL-3.0-only
//! Discrete GPUs that nothing drives yet: name them, then power them off (D3cold).
//!
//! A gaming laptop's NVIDIA GPU (the Alienware x17 R1's RTX 30 laptop part, for one)
//! draws power and makes heat from the moment the machine starts, even when the Intel
//! GPU drives the panel. Until a SexOS driver owns it (GSP firmware boot, stage N2 in
//! docs/handoff/NVIDIA_GPU_PLAN_2026_11_09.md), BOOT turns it off the way Linux runtime
//! power management does:
//!
//! 1. every function in the GPU's slot goes to D3hot through its PCI PM registers (here,
//!    at boot);
//! 2. the GPU's ACPI node runs `_PS3`, then `_OFF` for the power resources its
//!    `_PR0`..`_PR3` name (skipping any resource another device needs in D0);
//! 3. the root port above it, when the GPU is all that is behind it, does the same.
//!
//! Steps 2 and 3 run firmware AML, so they happen in the ring-3 ACPI service
//! (`servers/sexacpi`), which drives this module through `sex_pdx::power::FW_DGPU`. That
//! op only reaches the GPUs listed here: the service never names a PCI function itself,
//! and the header save/restore (which writes BARs) stays in the kernel.
//!
//! What stays on: a GPU on the root bus (an integrated GPU) and a GPU that holds the boot
//! framebuffer (BIOS "discrete only" mode or a mux set to the dGPU). Without the ACPI
//! service the GPU stays in D3hot, which every PCI device must accept.

use super::acpi::stall_us;
use crate::hal::pci::{pci_config_read, pci_config_write};
use crate::serial_println;
use alloc::string::String;
use alloc::vec::Vec;
use sex_pdx::power as w;

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
// ── State kept for `power_on` ─────────────────────────────────────────────

struct Off {
    gpu: Bdf,
    vendor: u16,
    device: u16,
    /// The bridges above the GPU, root port first.
    chain: Vec<Bdf>,
    port: Bdf,
    /// The root port goes cold too only when this GPU is all that is behind it.
    port_alone: bool,
    port_d3: bool,
    /// Each function's configuration header (0x04..0x40), restored after power returns.
    saved: Vec<(Bdf, [u32; 16])>,
    port_saved: Option<[u32; 16]>,
    cold: bool,
    on: bool,
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

pub fn powered_off(b: Bdf) -> bool { OFF.lock().iter().any(|o| !o.on && o.gpu.bus == b.bus && o.gpu.dev == b.dev) }

/// BOOT: find discrete GPUs and park the ones nothing will drive in D3hot.
pub fn init() {
    let Some(hhdm) = crate::HHDM_REQUEST.response().map(|r| r.offset) else { return };
    let fb = crate::FB_REQUEST.response().and_then(|r| r.framebuffers().iter().next().map(|f| (f.address() as u64).wrapping_sub(hhdm)));
    let all = scan();
    for d in all.iter().filter(|d| d.class >> 8 == 0x03 && d.at.func == 0) {
        let gpu = d.at;
        let name = w::gpu_product(d.vendor, d.device);
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
        let port_alone = (0..32u8).all(|dev| dev == gpu.dev || !exists(Bdf { bus: gpu.bus, dev, func: 0 }));
        let mut off = Off { gpu, vendor: d.vendor, device: d.device, chain, port, port_alone, port_d3: false,
            saved: Vec::new(), port_saved: None, cold: false, on: true };
        prepare(&mut off);
        serial_println!("[gpu.power] at={} name=\"{}\" action=off state=d3hot acpi=pending", gpu, name);
        OFF.lock().push(off);
    }
}

/// Saves every function's header (and the root port's) and puts the functions in D3hot.
fn prepare(o: &mut Off) {
    let funcs = functions(o.gpu.bus, o.gpu.dev);
    o.saved = funcs.iter().map(|f| (*f, save(*f))).collect();
    o.port_saved = o.port_alone.then(|| save(o.port));
    for f in &funcs { set_pm_state(*f, 3); }
    stall_us(10_000); // D0 -> D3hot needs 10 ms before the next access.
    o.port_d3 = false;
    o.cold = false;
    o.on = false;
}

/// Back to D0 with the configuration restored, once the ACPI service has turned the
/// power resources back on. False if it does not come back.
fn power_on(o: &mut Off) -> bool {
    if o.port_d3 {
        if let Some(h) = o.port_saved.as_ref() { set_pm_state(o.port, 0); stall_us(10_000); restore(o.port, h); }
        o.port_d3 = false;
    }
    // After D3cold the link retrains: up to a second before the device answers.
    let mut waited = 0;
    while !exists(o.gpu) && waited < 1_000 { stall_us(1_000); waited += 1; }
    if !exists(o.gpu) { serial_println!("[gpu.power] at={} action=on ok=0 reason=no_answer", o.gpu); return false; }
    for (f, h) in &o.saved {
        set_pm_state(*f, 0);
        stall_us(10_000);
        restore(*f, h);
    }
    o.on = true;
    serial_println!("[gpu.power] at={} action=on ok=1 cold={} waited_ms={}", o.gpu, o.cold as u8, waited);
    true
}

fn pack(b: Bdf) -> u64 { w::bdf(b.bus, b.dev, b.func) as u64 }

/// `FW_DGPU`, for the ACPI service (FIRMWARE right).
pub fn firmware_op(arg: u64, arg2: u64, arg3: u64) -> [u64; 5] {
    let (op, index) = (arg & 0xFF, (arg >> 8) as usize);
    let mut list = OFF.lock();
    let Some(o) = list.get_mut(index) else { return [w::INVALID, 0, 0, 0, 0] };
    match op {
        w::DGPU_INFO => {
            let mut words = [0u64; 2];
            for (i, b) in o.chain.iter().take(8).enumerate() { words[i / 4] |= pack(*b) << ((i % 4) * 16); }
            [w::OK, pack(o.gpu) | (o.port_alone as u64) << 16 | (o.chain.len().min(8) as u64) << 20 | (o.vendor as u64) << 32,
             words[0], words[1], o.device as u64]
        }
        w::DGPU_PORT_D3 if o.port_alone && !o.on => {
            set_pm_state(o.port, 3);
            stall_us(10_000);
            o.port_d3 = true;
            [w::OK, 0, 0, 0, 0]
        }
        w::DGPU_DONE => { o.cold = arg2 > 0 && arg3 != 0; [w::OK, 0, 0, 0, 0] }
        w::DGPU_RESTORE if !o.on => {
            let on = power_on(o);
            let back = on && o.saved.iter().all(|(f, _)| pm_state(*f).map_or(true, |s| s == 0))
                && o.saved.first().map_or(false, |(f, h)| rd(*f, 0x10) == h[4]);
            [w::OK, on as u64, back as u64, 0, 0]
        }
        w::DGPU_PREPARE if o.on => { prepare(o); [w::OK, 0, 0, 0, 0] }
        _ => [w::INVALID, 0, 0, 0, 0],
    }
}
