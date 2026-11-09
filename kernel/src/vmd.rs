//! Intel VMD bus driver (BOOT only). With Intel RST "RAID On", NVMe drives sit
//! in VMD's own PCI domain and the normal bus scan never sees them. For every
//! VMD controller found, map its config window (CFGBAR), let `sex_vmd` number
//! the buses and place every BAR inside VMD's memory windows, and add the
//! devices behind it to the device list. From then on they are ordinary PCI
//! devices whose config space is reached through `PciDevice::ecam`, and the
//! normal leases (NVMe to sexdrive) apply to them unchanged.
//!
//! Interrupts behind VMD are remapped through VMD's own MSI-X table; nothing
//! here sets that up, so drivers of these devices poll (sexdrive does).

use crate::hal::pci::PciDevice;
use crate::serial_println;
use alloc::vec::Vec;
use x86_64::{structures::paging::PageTableFlags, VirtAddr};

/// Devices kept per VMD domain; a laptop has two or three root ports.
const MAX_DEVICES: usize = 32;

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
    let count = match sex_vmd::configure(&mut ecam, buses,
        sex_vmd::Window { base: m1, length: m1_len }, sex_vmd::Window { base: m2, length: m2_len }, &mut out) {
        Ok(n) => n,
        Err(e) => { serial_println!("[kernel.vmd.err] {:?}", e); return 0; }
    };
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
