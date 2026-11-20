// SPDX-License-Identifier: GPL-3.0-only
//! PCIe power mechanism: BOOT turns on link power states (ASPM, L1 substates, LTR,
//! clock PM), drivers move the devices BOOT leased them between D0 and D3hot, and
//! drivers hear when the machine goes to sleep. The policy (which states, latency
//! checks, quirks, what a D-state change must save) is `sex_pcipm`, tested on the
//! host; this file is config access and records.
//!
//! Config space: through each function's ECAM page (mapped at BOOT, needed for the
//! L1 substates and LTR capabilities above 0x100), else the legacy ports.
use crate::hal::pci::{pci_config_read, pci_config_write, PciDevice};
use crate::serial_println;
use core::sync::atomic::{AtomicBool, Ordering};
use sex_pcipm::{aspm, pm, Bdf, Config};

const MAX_PAGES: usize = 128;
const MAX_SAVED: usize = 16;
/// MSI-X table entries kept across a reset (16 bytes each).
const MSIX_KEPT: usize = 16;

static PAGES: spin::Mutex<[(u32, u64); MAX_PAGES]> = spin::Mutex::new([(0, 0); MAX_PAGES]);
static SAVED: spin::Mutex<[Option<(Bdf, pm::Saved, [u32; MSIX_KEPT * 4])>; MAX_SAVED]> = spin::Mutex::new([None; MAX_SAVED]);
/// The FADT says the OS must not enable ASPM (`IAPC_BOOT_ARCH` bit 4).
pub static ASPM_FORBIDDEN: AtomicBool = AtomicBool::new(false);

fn key(f: Bdf) -> u32 { 1 << 16 | (f.bus as u32) << 8 | (f.dev as u32) << 3 | f.func as u32 }

struct Hw;
impl Hw {
    fn page(f: Bdf) -> Option<u64> { PAGES.lock().iter().find(|p| p.0 == key(f)).map(|p| p.1) }
}
impl Config for Hw {
    fn read(&mut self, f: Bdf, at: u16) -> u32 {
        match Self::page(f) {
            Some(va) => unsafe { ((va + (at as u64 & 0xFFC)) as *const u32).read_volatile() },
            None if at < 256 => unsafe { pci_config_read(f.bus, f.dev, f.func, at as u8 & 0xFC) },
            None => u32::MAX,
        }
    }
    fn write(&mut self, f: Bdf, at: u16, value: u32) {
        match Self::page(f) {
            Some(va) => unsafe { ((va + (at as u64 & 0xFFC)) as *mut u32).write_volatile(value) },
            None if at < 256 => unsafe { pci_config_write(f.bus, f.dev, f.func, at as u8 & 0xFC, value) },
            None => {}
        }
    }
    fn delay_us(&mut self, us: u32) { crate::hal::acpi::stall_us(us as u64) }
}

/// BOOT: map each function's ECAM page (segment 0, from MCFG).
fn map_ecam(devices: &[PciDevice]) -> usize {
    let regions = crate::apic::PCI_REGIONS.lock();
    let mut pages = PAGES.lock();
    let mut n = 0;
    for d in devices.iter().filter(|d| d.ecam == 0) {
        let Some(r) = regions.iter().find(|r| r.segment == 0 && (r.bus_start..=r.bus_end).contains(&d.bus)) else { continue };
        if n == MAX_PAGES { break; }
        let pa = r.phys_addr as u64 + (((d.bus - r.bus_start) as u64) << 20 | (d.dev as u64) << 15 | (d.func as u64) << 12);
        let va = crate::boot_memory::kernel_mmio(pa, 4096);
        // A window that does not answer for this function is not used.
        let id = unsafe { (va as *const u32).read_volatile() };
        if id & 0xFFFF != d.vendor_id as u32 { continue; }
        pages[n] = (key(Bdf::new(d.bus, d.dev, d.func)), va);
        n += 1;
    }
    n
}

/// BOOT, before any lease: link power states for every PCIe link in segment 0
/// (devices behind VMD are on VMD's own links and are left as firmware set them).
pub fn boot(devices: &[PciDevice]) {
    let ecam = map_ecam(devices);
    let functions: alloc::vec::Vec<Bdf> = devices.iter().filter(|d| d.ecam == 0).map(|d| Bdf::new(d.bus, d.dev, d.func)).collect();
    let forbidden = ASPM_FORBIDDEN.load(Ordering::Acquire);
    let r = aspm::configure(&mut Hw, &functions, aspm::Policy::POWERSAVE, forbidden);
    let mut changed = 0;
    for l in r.iter() {
        let (mut a, mut b, mut c) = ([0u8; 48], [0u8; 48], [0u8; 48]);
        if l.after != l.before { changed += 1; }
        let child = l.children().first().copied().unwrap_or(l.port);
        // What the hardware kept (ASPM control bits, port then device).
        let readback = |f: Bdf| sex_pcipm::pcie(&mut Hw, f).map_or(0xFF, |(at, _)| Hw.read(f, at + sex_pcipm::reg::EXP_LNKCTL) & 3);
        serial_println!("[pci.aspm] link={}->{} id={:04x}:{:04x} support={} before={} after={} l12_threshold_us={} skip={} lnkctl={}/{}",
            l.port, child, l.vendor(), l.device(), aspm::describe(l.support, &mut a),
            aspm::describe(l.before, &mut b), aspm::describe(l.after, &mut c), l.threshold_us, l.skip.map_or("none", |s| s.name()),
            readback(l.port), readback(child));
    }
    serial_println!("[pci.aspm.summary] links={} changed={} ltr_enabled={} ltr_already={} ecam_pages={} fadt_no_aspm={} dropped={}",
        r.count, changed, r.ltr_enabled, r.ltr_already, ecam, forbidden as u8, r.dropped);
}

/// A driver's D0 / D3hot request for a device BOOT leased it (syscall 86 `OP_POWER`).
/// `table`/`vectors`: its MSI-X table, kept across a reset. Returns the state it was
/// in, or an error from `sex_pdx::device`.
pub fn set_power(lease: &sex_pdx::device::Lease, table: u64, to: u64) -> u64 {
    use sex_pdx::device as d;
    let f = Bdf::new(lease.bus, lease.dev, lease.func);
    let mut hw = Hw;
    // The record must still name the function there (VMD devices sit in another domain).
    if hw.read(f, 0) != lease.vendor as u32 | (lease.device as u32) << 16 { return d::ERR_POWER; }
    let mut saved_table = SAVED.lock();
    let slot = saved_table.iter().position(|s| s.is_some_and(|s| s.0 == f));
    let mut saved = slot.and_then(|i| saved_table[i].map(|s| s.1));
    let mut msix = slot.and_then(|i| saved_table[i].map(|s| s.2)).unwrap_or([0; MSIX_KEPT * 4]);
    let kept = if lease.mode == d::Mode::MsiX { (lease.vectors as usize).min(MSIX_KEPT) } else { 0 };
    // Only a device still in D0 has a readable table; a repeated D3hot request keeps
    // the snapshot taken on the way down.
    if to == pm::D3HOT as u64 && table != 0 && pm::state(&mut hw, f) == Some(pm::D0) {
        for (i, w) in msix.iter_mut().enumerate().take(kept * 4) { *w = unsafe { ((table + i as u64 * 4) as *const u32).read_volatile() }; }
    }
    let result = pm::set_state(&mut hw, f, to as u8, &mut saved);
    match result {
        Ok(t) => {
            if t.restored && table != 0 {
                for (i, w) in msix.iter().enumerate().take(kept * 4) { unsafe { ((table + i as u64 * 4) as *mut u32).write_volatile(*w) } }
            }
            if let Some(s) = saved {
                let at = slot.or_else(|| saved_table.iter().position(|s| s.is_none()));
                if let Some(at) = at { saved_table[at] = Some((f, s, msix)); }
            }
            serial_println!("[pci.power] {} id={:04x}:{:04x} from=d{} to=d{} no_soft_reset={} restored={} msix_kept={}",
                f, lease.vendor, lease.device, t.from, t.to, t.no_soft_reset as u8, t.restored as u8, kept);
            t.from as u64
        }
        Err(e) => {
            serial_println!("[pci.power.error] {} to=d{} error={:?}", f, to, e);
            d::ERR_POWER
        }
    }
}

/// Suspend to idle began (true) or ended: every service holding a device lease hears
/// `sex_pdx::power::OP_DEVICE_SLEEP` (arg0 = 1 asleep, 0 awake) from the kernel.
pub fn tell_drivers(asleep: bool) {
    let mut told = 0;
    for pd in crate::device_irq::holders().into_iter().filter(|p| *p != 0) {
        let Some(domain) = crate::ipc::DOMAIN_REGISTRY.get(pd) else { continue };
        let msg = crate::ipc::messages::MessageType::IpcCall { func_id: sex_pdx::power::OP_DEVICE_SLEEP, arg0: asleep as u64, arg1: 0, arg2: 0, caller_pd: 0, caller_task: 0 };
        if unsafe { (*domain.message_ring).enqueue(msg) }.is_ok() {
            crate::scheduler::unpark_thread(domain.main_task.load(Ordering::Acquire));
            told += 1;
        }
    }
    serial_println!("[pci.power.sleep] asleep={} drivers_told={}", asleep as u8, told);
}
