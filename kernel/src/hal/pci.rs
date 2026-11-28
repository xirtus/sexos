use crate::serial_println;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_id: u8,
    pub subclass_id: u8,
    pub prog_if: u8,
    /// 0: reached through the legacy config ports. Otherwise the kernel address
    /// of this function's 4 KiB ECAM page (a device behind Intel VMD).
    pub ecam: u64,
}

static NET_DIAG_HTTP_STATUS: AtomicU32 = AtomicU32::new(0);
static NET_DIAG_HTTP_BYTES: AtomicU32 = AtomicU32::new(0);
static NET_DIAG_HTTP_SOURCE: AtomicU8 = AtomicU8::new(0);
static NET_DIAG_HTTP_BODY_LEN: AtomicU32 = AtomicU32::new(0);
static mut NET_DIAG_HTTP_BODY: [u8; 64] = [0u8; 64];

pub fn get_net_diag() -> (u32, u32, u8) {
    (
        NET_DIAG_HTTP_STATUS.load(Ordering::Acquire),
        NET_DIAG_HTTP_BYTES.load(Ordering::Acquire),
        NET_DIAG_HTTP_SOURCE.load(Ordering::Acquire),
    )
}

pub fn get_net_diag_body_len() -> u32 {
    NET_DIAG_HTTP_BODY_LEN.load(Ordering::Acquire)
}

pub fn get_net_diag_body_chunk(idx: usize) -> u64 {
    let len = NET_DIAG_HTTP_BODY_LEN.load(Ordering::Acquire) as usize;
    let start = idx * 8;
    if start >= len {
        return u64::MAX;
    }

    let end = core::cmp::min(start + 8, len);
    let mut packed = 0u64;
    let mut i = 0usize;
    while i < (end - start) {
        packed |= unsafe { (NET_DIAG_HTTP_BODY[start + i] as u64) << (i * 8) };
        i += 1;
    }
    packed
}

impl PciDevice {
    /// Boot discovery only. Restore both BAR words and the command register;
    /// upper status bits are W1C and must never be echoed into a config write.
    pub fn probe_boot_bar0(&self) -> Option<(u64, u64)> { self.probe_boot_bar(0) }

    /// Boot discovery only. Restore both BAR words and the command register;
    /// upper status bits are W1C and must never be echoed into a config write.
    pub fn probe_boot_bar(&self, index: u8) -> Option<(u64, u64)> {
        assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),
            crate::ipc::BootPhase::RegistryBuild as u8, "PCI_PROBE_AFTER_GENESIS");
        if index > 5 { return None; }
        let at = 0x10 + index * 4;
        let low = self.read_u32(at);
        if low & 1 != 0 || !matches!((low >> 1) & 3, 0 | 2) { return None; }
        let wide = ((low >> 1) & 3) == 2;
        if wide && index == 5 { return None; }
        let high = if wide { self.read_u32(at + 4) } else { 0 };
        let command = self.read_u32(0x04) & 0xffff;
        self.write_u32(0x04, command & !7);
        self.write_u32(at, u32::MAX);
        if wide { self.write_u32(at + 4, u32::MAX); }
        let mask_low = self.read_u32(at);
        let mask_high = if wide { self.read_u32(at + 4) } else { 0 };
        self.write_u32(at, low);
        if wide { self.write_u32(at + 4, high); }
        self.write_u32(0x04, command);
        crate::pci_bar_bounds::decode_bar0(low, high, mask_low, mask_high)
    }

    /// Boot discovery only. Gives an unassigned 64-bit memory BAR (base 0, as the
    /// Alienware x17's firmware leaves its touchpad's Serial IO controller) an address:
    /// naturally aligned, at least 1 MiB above the end of every 64-bit window that
    /// firmware already placed above 4 GiB on this bus (function BARs, bridge prefetch windows),
    /// below 2^39 (Tiger Lake's physical address width). Linux would take the host
    /// bridge's `_CRS` window, which only the ACPI service can read; firmware packs its
    /// own assignments at the low end of that window, so the space above them is free.
    /// Returns the new base, or None when the BAR is not an empty 64-bit memory BAR.
    pub fn assign_boot_bar64(&self, index: u8) -> Option<u64> {
        assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),
            crate::ipc::BootPhase::RegistryBuild as u8, "PCI_ASSIGN_AFTER_GENESIS");
        if index > 4 { return None; }
        let at = 0x10 + index * 4;
        let low = self.read_u32(at);
        if low & 1 != 0 || (low >> 1) & 3 != 2 || low & !15 != 0 || self.read_u32(at + 4) != 0 { return None; }
        let command = self.read_u32(0x04) & 0xffff;
        self.write_u32(0x04, command & !7);
        self.write_u32(at, u32::MAX);
        self.write_u32(at + 4, u32::MAX);
        let mask = ((self.read_u32(at + 4) as u64) << 32) | (self.read_u32(at) & !15) as u64;
        let length = (!mask).wrapping_add(1);
        if length < 16 || !length.is_power_of_two() || length > (1 << 30) {
            self.write_u32(at, low); self.write_u32(at + 4, 0); self.write_u32(0x04, command);
            return None;
        }
        let mut top = 0u64;
        // Only this bus is walked: whatever sits behind a bridge lies inside that
        // bridge's window, so nothing behind one is probed.
        for (b, d, f, _) in present_functions().into_iter().filter(|p| p.0 == self.bus) {
            let header = (unsafe { pci_config_read(b, d, f, 0x0c) } >> 16) as u8 & 0x7f;
            let bars = match header { 0 => 6u8, _ => 0 };
            let mut i = 0u8;
            while i < bars {
                let lo = unsafe { pci_config_read(b, d, f, 0x10 + i * 4) };
                let wide = lo & 1 == 0 && (lo >> 1) & 3 == 2 && i + 1 < bars;
                if wide && (b, d, f, i) != (self.bus, self.dev, self.func, index) {
                    let base = (lo & !15) as u64 | (unsafe { pci_config_read(b, d, f, 0x14 + i * 4) } as u64) << 32;
                    if base >= 1 << 32 {
                        // Size it the way `probe_boot_bar` does: no decode while probing.
                        let cmd = unsafe { pci_config_read(b, d, f, 0x04) } & 0xffff;
                        unsafe {
                            pci_config_write(b, d, f, 0x04, cmd & !7);
                            pci_config_write(b, d, f, 0x10 + i * 4, u32::MAX);
                            pci_config_write(b, d, f, 0x14 + i * 4, u32::MAX);
                        }
                        let m = ((unsafe { pci_config_read(b, d, f, 0x14 + i * 4) } as u64) << 32)
                            | (unsafe { pci_config_read(b, d, f, 0x10 + i * 4) } & !15) as u64;
                        unsafe {
                            pci_config_write(b, d, f, 0x10 + i * 4, lo);
                            pci_config_write(b, d, f, 0x14 + i * 4, (base >> 32) as u32);
                            pci_config_write(b, d, f, 0x04, cmd);
                        }
                        top = top.max(base.saturating_add((!m).wrapping_add(1)));
                    }
                }
                i += if wide { 2 } else { 1 };
            }
            if header == 1 {
                // Prefetchable window: base/limit at 0x24, upper words at 0x28/0x2c.
                let pl = unsafe { pci_config_read(b, d, f, 0x24) };
                if pl & 0xf == 1 {
                    let limit = (((unsafe { pci_config_read(b, d, f, 0x2c) } as u64) << 32) | ((pl >> 16) as u64 & 0xfff0) << 16) | 0xf_ffff;
                    let base = ((unsafe { pci_config_read(b, d, f, 0x28) } as u64) << 32) | ((pl as u64 & 0xfff0) << 16);
                    if base >= 1 << 32 && limit > base { top = top.max(limit + 1); }
                }
            }
        }
        let align = length.max(1 << 20);
        let base = (top.max(1 << 32) + (1 << 20) + align - 1) & !(align - 1);
        if top == 0 || base + length > 1 << 39 {
            self.write_u32(at, low); self.write_u32(at + 4, 0); self.write_u32(0x04, command);
            serial_println!("[pci.bar.assign.skip] {:02x}:{:02x}.{} bar={} length={:#x} top={:#x}", self.bus, self.dev, self.func, index, length, top);
            return None;
        }
        self.write_u32(at, (base as u32 & !15) | (low & 15));
        self.write_u32(at + 4, (base >> 32) as u32);
        self.write_u32(0x04, command);
        serial_println!("[pci.bar.assign] {:02x}:{:02x}.{} bar={} base={:#x} length={:#x} top={:#x}", self.bus, self.dev, self.func, index, base, length, top);
        Some(base)
    }

    /// Like `probe_boot_bar`, but a BAR shorter than a page is accepted too.
    pub fn probe_boot_bar_any(&self, index: u8) -> Option<(u64, u64)> {
        if let Some(found) = self.probe_boot_bar(index) { return Some(found); }
        if index > 5 { return None; }
        let at = 0x10 + index * 4;
        let low = self.read_u32(at);
        if low & 1 != 0 || !matches!((low >> 1) & 3, 0 | 2) { return None; }
        let wide = ((low >> 1) & 3) == 2;
        if wide && index == 5 { return None; }
        let high = if wide { self.read_u32(at + 4) } else { 0 };
        let command = self.read_u32(0x04) & 0xffff;
        self.write_u32(0x04, command & !7);
        self.write_u32(at, u32::MAX);
        if wide { self.write_u32(at + 4, u32::MAX); }
        let mask_low = self.read_u32(at);
        let mask_high = if wide { self.read_u32(at + 4) } else { 0 };
        self.write_u32(at, low);
        if wide { self.write_u32(at + 4, high); }
        self.write_u32(0x04, command);
        crate::pci_bar_bounds::decode_subpage(low, high, mask_low, mask_high)
    }

    pub fn read_u32(&self, offset: u8) -> u32 {
        if self.ecam != 0 {
            return unsafe { core::ptr::read_volatile((self.ecam + (offset as u64 & 0xFC)) as *const u32) };
        }
        unsafe { pci_config_read(self.bus, self.dev, self.func, offset) }
    }

    pub fn write_u32(&self, offset: u8, value: u32) {
        if self.ecam != 0 {
            return unsafe { core::ptr::write_volatile((self.ecam + (offset as u64 & 0xFC)) as *mut u32, value) };
        }
        unsafe { pci_config_write(self.bus, self.dev, self.func, offset, value) }
    }

    pub fn get_bar(&self, index: u8) -> u64 {
        let offset = 0x10 + (index * 4);
        let bar = self.read_u32(offset);
        if bar & 0x1 != 0 {
            (bar & 0xFFFF_FFFC) as u64
        } else {
            let type_bits = (bar >> 1) & 0x3;
            if type_bits == 0x2 { // 64-bit
                let bar_high = self.read_u32(offset + 4);
                ((bar_high as u64) << 32) | (bar as u64 & 0xFFFF_FFF0)
            } else {
                (bar & 0xFFFF_FFF0) as u64
            }
        }
    }
}

static PCI_CONFIG_LOCK: spin::Mutex<()> = spin::Mutex::new(());

fn config_address(bus:u8,slot:u8,func:u8,offset:u8)->u32 {
    ((bus as u32)<<16)|((slot as u32)<<11)|((func as u32)<<8)|(offset as u32&0xfc)|0x8000_0000
}

fn config_locked<R>(operation:impl FnOnce()->R)->R {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let _guard=PCI_CONFIG_LOCK.lock();operation()
    })
}

unsafe fn config_read_unlocked(bus:u8,slot:u8,func:u8,offset:u8)->u32 {
    x86_64::instructions::port::Port::new(0xCF8).write(config_address(bus,slot,func,offset));
    x86_64::instructions::port::Port::new(0xCFC).read()
}

unsafe fn config_write_unlocked(bus:u8,slot:u8,func:u8,offset:u8,value:u32) {
    x86_64::instructions::port::Port::new(0xCF8).write(config_address(bus,slot,func,offset));
    x86_64::instructions::port::Port::new(0xCFC).write(value);
}

pub unsafe fn pci_config_read(bus:u8,slot:u8,func:u8,offset:u8)->u32 {
    config_locked(||config_read_unlocked(bus,slot,func,offset))
}

pub unsafe fn pci_config_write(bus:u8,slot:u8,func:u8,offset:u8,value:u32) {
    config_locked(||config_write_unlocked(bus,slot,func,offset,value))
}

/// Capability authorization precedes this bounded mechanism. One config lock
/// covers identity/read/optional16-bit command write/readback. Status at+6 is
/// never echoed through a32-bit write (its error fields are write-one-to-clear).
pub(crate) fn apply_command(update:sex_pdx::pci_command::Update)->Result<sex_pdx::pci_command::Reply,u64> {
    use sex_pdx::pci_command::{Reply,IO};
    config_locked(||unsafe {
        let g=update.grant();
        if config_read_unlocked(g.bus,g.device,g.function,0) != (g.vendor_id as u32 | ((g.device_id as u32)<<16)) {
            return Err(IO);
        }
        let before=config_read_unlocked(g.bus,g.device,g.function,4) as u16;
        let expected=update.next(before);
        if expected != before {
            x86_64::instructions::port::Port::new(0xCF8).write(config_address(g.bus,g.device,g.function,4));
            x86_64::instructions::port::Port::<u16>::new(0xCFC).write(expected);
        }
        let after=config_read_unlocked(g.bus,g.device,g.function,4) as u16;
        if after != expected { return Err(IO); }
        Ok(Reply {before,after,changed:before!=after})
    })
}

/// Every PCI function present, as (bus, device, function, vendor|device id), found by
/// one walk of all 256 buses while the boot registry is built and reused by every boot
/// caller after it (device manager, IOMMU, GPU power, BAR page checks). Each config read
/// is two port accesses, two exits to the hypervisor under KVM: one walk is ~16k exits,
/// which nested CI virtualisation turned into a third of a second per walk, and boot
/// used to walk the bus several times. Functions 1-7 are read only on multi-function
/// devices (header type bit 7), as the GPU and BAR walks already did. Outside the boot
/// registry phase nothing is cached: devices may come and go.
static PRESENT: spin::Mutex<Option<Vec<(u8, u8, u8, u32)>>> = spin::Mutex::new(None);
pub fn present_functions() -> Vec<(u8, u8, u8, u32)> {
    let boot = crate::ipc::BOOT_PHASE.load(Ordering::Acquire) == crate::ipc::BootPhase::RegistryBuild as u8;
    if boot { if let Some(found) = PRESENT.lock().as_ref() { return found.clone(); } }
    let t = crate::boot_cost::now();
    let mut found = Vec::new();
    for bus in 0..256u16 {
        for dev in 0..32u8 {
            let id = unsafe { pci_config_read(bus as u8, dev, 0, 0) };
            if id & 0xFFFF == 0xFFFF { continue; }
            found.push((bus as u8, dev, 0, id));
            let multi = (unsafe { pci_config_read(bus as u8, dev, 0, 0x0C) } >> 16) & 0x80 != 0;
            if !multi { continue; }
            for func in 1..8u8 {
                let id = unsafe { pci_config_read(bus as u8, dev, func, 0) };
                if id & 0xFFFF != 0xFFFF { found.push((bus as u8, dev, func, id)); }
            }
        }
    }
    crate::boot_cost::add(crate::boot_cost::Bucket::PciWalk, t, 1);
    if boot {
        serial_println!("[pci.walk] functions={} cached=1", found.len());
        *PRESENT.lock() = Some(found.clone());
    }
    found
}

pub fn enumerate_bus() -> Vec<PciDevice> {
    assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),
        crate::ipc::BootPhase::RegistryBuild as u8, "PCI_ENUMERATION_AFTER_GENESIS");
    let mut devices = Vec::new();
    for (bus, slot, func, id) in present_functions() {
        let class_rev = unsafe { pci_config_read(bus, slot, func, 8) };
        devices.push(PciDevice {
            bus,
            dev: slot,
            func,
            vendor_id: id as u16,
            device_id: (id >> 16) as u16,
            class_id: (class_rev >> 24) as u8,
            subclass_id: (class_rev >> 16) as u8,
            prog_if: (class_rev >> 8) as u8,
            ecam: 0,
        });
    }
    
    // Log discovery for XPS 17 target
    for dev in &devices {
        match (dev.vendor_id, dev.device_id) {
            (0x8086, 0x9a60) => { serial_println!("HAL: Detected Intel Iris Xe (XPS 17)"); }
            (0x144d, _) => { serial_println!("HAL: Detected Samsung NVMe (XPS 17)"); }
            _ => {}
        }
        // Network Controller (class 0x02) detection — e1000, virtio-net, etc.
        // Marker only: no BAR mapping, no IRQ routing, no driver attach.
        if dev.class_id == 0x02 {
            // Discovery only. The BOOT-assigned SexNet domain owns the controller; the kernel
            // never programs NIC registers, rings or frames (DESIGN_GUARDRAILS: no protocol
            // stacks in the kernel).
            serial_println!("[pci.net.device] vendor=0x{:04X} device=0x{:04X} class=0x02 subclass=0x{:02X} prog_if=0x{:02X} ok=1 reason=network_controller_detected",
                dev.vendor_id, dev.device_id, dev.subclass_id, dev.prog_if);
            serial_println!("[pci.net.passive] vendor=0x{:04X} device=0x{:04X} config_writes=0 mmio=0 rings=0 dma=0 packets=0 owner=boot_assigned_sexnet ok=1",
                dev.vendor_id,dev.device_id);
            continue;
        }
    }

    devices
}

/// True when no memory BAR of any PCI function other than `(bus,dev,func,index)`
/// starts inside the 4 KiB page at `page`. BARs are naturally aligned powers of
/// two, so another window overlapping that page must start inside it. Reads BAR
/// registers only; nothing is probed or written.
pub fn page_exclusive(page: u64, bus: u8, dev: u8, func: u8, index: u8) -> bool {
    for (b, d, f, _) in present_functions() {
        // Bridges (header type 1) have two BARs, ordinary functions six.
        let header = (unsafe { pci_config_read(b, d, f, 0x0c) } >> 16) as u8;
        let bars = match header & 0x7f { 0 => 6u8, 1 => 2, _ => 0 };
        let mut i = 0u8;
        while i < bars {
            let low = unsafe { pci_config_read(b, d, f, 0x10 + i * 4) };
            let wide = low & 1 == 0 && (low >> 1) & 3 == 2 && i + 1 < bars;
            let base = if low & 1 != 0 { 0 } else {
                (low & !15) as u64 | if wide { (unsafe { pci_config_read(b, d, f, 0x14 + i * 4) } as u64) << 32 } else { 0 }
            };
            let own = (b, d, f, i) == (bus, dev, func, index);
            if !own && base != 0 && base >= page && base < page + 4096 { return false; }
            i += if wide { 2 } else { 1 };
        }
    }
    true
}
