//! Intel VMD bus driver (BOOT only). With Intel RST "RAID On", NVMe drives sit
//! in VMD's own PCI domain and the normal bus scan never sees them. For every
//! VMD controller found, map its config window (CFGBAR), let `sex_vmd` number
//! the buses and place every BAR inside VMD's memory windows, and add the
//! devices behind it to the device list. From then on they are ordinary PCI
//! devices whose config space is reached through `PciDevice::ecam`, and the
//! normal leases (NVMe to sexdrive) apply to them unchanged.
//!
//! Interrupts behind VMD go through VMD's own MSI-X table (VMCONFIG MSI remap). Each
//! domain's table is set up here, every entry masked; `route` aims one entry at a CPU
//! vector when `device_irq` gives a device behind VMD its vectors.

use crate::hal::pci::PciDevice;
use crate::serial_println;
use alloc::vec::Vec;
use x86_64::{structures::paging::PageTableFlags, VirtAddr};

/// Devices kept per VMD domain; a laptop has two or three root ports.
const MAX_DEVICES: usize = 32;

/// One VMD domain's interrupt table: the ECAM range of its devices, VMD's MSI-X
/// table (kernel mapping) and the vectors not yet given out.
#[derive(Clone, Copy)]
struct Remap { ecam: u64, ecam_end: u64, table: u64, vectors: sex_vmd::Vectors }
static REMAPS: spin::Mutex<[Option<Remap>; 4]> = spin::Mutex::new([None; 4]);

/// BOOT: `n` VMD vectors for `dev` (a device behind VMD), each aimed at CPU vector
/// `cpu_vector + i` on `apic`. Returns the first VMD vector, for the device's MSI-X.
pub fn route(dev: &PciDevice, n: u16, cpu_vector: u8, apic: u32) -> Option<u16> {
    let mut remaps = REMAPS.lock();
    let r = remaps.iter_mut().flatten().find(|r| (r.ecam..r.ecam_end).contains(&dev.ecam))?;
    let first = r.vectors.take(n)?;
    for i in 0..n as u64 {
        let entry = (r.table + (first as u64 + i) * 16) as *mut u32;
        unsafe {
            entry.write_volatile(crate::device_irq::address(apic));
            entry.add(1).write_volatile(0);
            entry.add(2).write_volatile(cpu_vector as u32 + i as u32);
            entry.add(3).write_volatile(0);
        }
    }
    Some(first)
}

/// BOOT: VMD's own MSI-X on, every entry masked, MSI remapping on.
fn msi_remap(vmd: &PciDevice, ecam: u64, bytes: u64) {
    if crate::safe_start() { serial_println!("[kernel.vmd.msi] remap=0 reason=safe_start"); return; }
    let Some(cap) = crate::device_irq::capabilities(vmd).1 else {
        serial_println!("[kernel.vmd.msi] remap=0 reason=no_msix"); return;
    };
    let size = ((vmd.read_u32(cap) >> 16) & 0x7ff) + 1;
    let place = vmd.read_u32(cap + 4);
    let (bir, offset) = ((place & 7) as u8, (place & !7) as u64);
    let Some((physical, _)) = vmd.probe_boot_bar(bir).filter(|&(_, l)| offset + size as u64 * 16 <= l) else {
        serial_println!("[kernel.vmd.msi] remap=0 reason=table_outside_bar bir={} offset={:#x}", bir, offset); return;
    };
    let table_pa = physical + offset;
    let page = table_pa & !0xfff;
    let table = crate::boot_memory::kernel_mmio(page, ((table_pa - page) + size as u64 * 16 + 0xfff) & !0xfff) + (table_pa - page);
    vmd.write_u32(cap, vmd.read_u32(cap) | 0xC000 << 16);
    for i in 0..size as u64 { unsafe { ((table + i * 16 + 12) as *mut u32).write_volatile(1); } }
    vmd.write_u32(cap, (vmd.read_u32(cap) | 0x8000 << 16) & !(0x4000 << 16));
    vmd.write_u32(0x04, (vmd.read_u32(0x04) & 0xffff) | 1 << 10);
    let config = vmd.read_u32(sex_vmd::VMCONFIG as u8);
    vmd.write_u32(sex_vmd::VMCONFIG as u8, config | sex_vmd::VMCONFIG_MSI_REMAP);
    let vectors = sex_vmd::Vectors::new(size as u16);
    let mut remaps = REMAPS.lock();
    match remaps.iter_mut().find(|r| r.is_none()) {
        Some(free) => *free = Some(Remap { ecam, ecam_end: ecam + bytes, table, vectors }),
        None => { serial_println!("[kernel.vmd.msi] remap=0 reason=too_many_vmd"); return; }
    }
    serial_println!("[kernel.vmd.msi] remap=1 table_size={} child_vectors={} bir={} offset={:#x} vmconfig={:#x}",
        size, vectors.free(), bir, offset, config | sex_vmd::VMCONFIG_MSI_REMAP);
}

struct Ecam { base: u64, buses: u16 }

impl Ecam {
    fn at(&self, bus: u8, dev: u8, func: u8, offset: u16) -> Option<u64> {
        if bus as u16 >= self.buses || dev >= 32 || func >= 8 || offset >= 4096 { return None; }
        Some(self.base + ((bus as u64) << 20 | (dev as u64) << 15 | (func as u64) << 12 | (offset as u64 & 0xFFC)))
    }
}

impl sex_vmd::ConfigSpace for Ecam {
    fn read(&mut self, bus: u8, dev: u8, func: u8, offset: u16) -> u32 {
        match self.at(bus, dev, func, offset) {
            Some(va) => unsafe { core::ptr::read_volatile(va as *const u32) },
            None => u32::MAX,
        }
    }
    fn write(&mut self, bus: u8, dev: u8, func: u8, offset: u16, value: u32) {
        if let Some(va) = self.at(bus, dev, func, offset) {
            unsafe { core::ptr::write_volatile(va as *mut u32, value) }
        }
    }
}

/// Add the devices behind every VMD controller in `devices` to `devices`.
pub fn expand(devices: &mut Vec<PciDevice>) {
    let controllers: Vec<PciDevice> = devices.iter().copied()
        .filter(|d| d.ecam == 0 && sex_vmd::is_vmd(d.vendor_id, d.device_id)).collect();
    for vmd in controllers {
        let found = domain(&vmd, devices);
        serial_println!("[kernel.vmd.done] {:02x}:{:02x}.{} device={:04x} added={}", vmd.bus, vmd.dev, vmd.func, vmd.device_id, found);
    }
}

fn domain(vmd: &PciDevice, devices: &mut Vec<PciDevice>) -> usize {
    let (Some((cfg, cfg_len)), Some((m1, m1_len)), Some((m2, m2_len))) =
        (vmd.probe_boot_bar(0), vmd.probe_boot_bar(2), vmd.probe_boot_bar(4)) else {
        serial_println!("[kernel.vmd.skip] reason=bars_unassigned cfgbar={:#x} membar1={:#x} membar2={:#x}",
            vmd.get_bar(0), vmd.get_bar(2), vmd.get_bar(4));
        return 0;
    };
    let vmcap = vmd.read_u32(sex_vmd::VMCAP as u8);
    let vmconfig = vmd.read_u32(sex_vmd::VMCONFIG as u8);
    let start = sex_vmd::bus_start(vmcap, vmconfig);
    let buses = ((cfg_len >> 20) as u16).min(256 - start as u16);
    serial_println!("[kernel.vmd.found] {:02x}:{:02x}.{} device={:04x} cfgbar={:#x} buses={} first_bus={} membar1={:#x}+{:#x} membar2={:#x}+{:#x}",
        vmd.bus, vmd.dev, vmd.func, vmd.device_id, cfg, buses, start, m1, m1_len, m2, m2_len);
    if buses == 0 { serial_println!("[kernel.vmd.skip] reason=cfgbar_too_small bytes={:#x}", cfg_len); return 0; }

    // VMD itself must decode its windows and pass DMA through.
    let command = vmd.read_u32(0x04) & 0xFFFF;
    vmd.write_u32(0x04, command | 0x0006);

    let bytes = (buses as u64) << 20;
    let Some(va) = crate::memory::va_allocator::allocate_va(bytes as usize) else {
        serial_println!("[kernel.vmd.skip] reason=no_address_space bytes={:#x}", bytes); return 0;
    };
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_CACHE
        | PageTableFlags::WRITE_THROUGH | PageTableFlags::NO_EXECUTE;
    let mapped = crate::memory::manager::GLOBAL_VAS.lock().as_mut()
        .map(|vas| vas.map_physical_range(VirtAddr::new(va), cfg, bytes, flags, 0).is_ok()).unwrap_or(false);
    if !mapped { serial_println!("[kernel.vmd.skip] reason=cfgbar_map_failed"); return 0; }

    let mut ecam = Ecam { base: va, buses };
    let mut out = [sex_vmd::Device::default(); MAX_DEVICES];
    let count = match sex_vmd::configure(&mut ecam, start, buses,
        sex_vmd::Window { base: m1, length: m1_len }, sex_vmd::Window { base: m2, length: m2_len }, &mut out) {
        Ok(n) => n,
        Err(e) => { serial_println!("[kernel.vmd.err] {:?}", e); return 0; }
    };
    msi_remap(vmd, va, bytes);
    let mut added = 0;
    for d in &out[..count] {
        let bar0 = d.bar(0).map(|b| b.address).unwrap_or(0);
        serial_println!("[kernel.vmd.device] bus={:02x} dev={:02x} fn={} vendor={:04x} device={:04x} class={:02x}:{:02x}:{:02x} bridge={} bar0={:#x}",
            start as u16 + d.bus as u16, d.dev, d.func, d.vendor, d.device, d.class, d.subclass, d.prog_if, d.bridge as u8, bar0);
        if d.bridge { continue; }
        devices.push(PciDevice {
            bus: start.wrapping_add(d.bus), dev: d.dev, func: d.func,
            vendor_id: d.vendor, device_id: d.device,
            class_id: d.class, subclass_id: d.subclass, prog_if: d.prog_if,
            ecam: ecam.at(d.bus, d.dev, d.func, 0).unwrap_or(0),
        });
        added += 1;
    }
    added
}
