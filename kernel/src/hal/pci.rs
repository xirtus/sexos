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

pub fn enumerate_bus() -> Vec<PciDevice> {
    assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),
        crate::ipc::BootPhase::RegistryBuild as u8, "PCI_ENUMERATION_AFTER_GENESIS");
    let mut devices = Vec::new();
    for bus in 0..256 {
        for slot in 0..32 {
            // Check if device exists
            let vendor_id = unsafe { pci_config_read(bus as u8, slot as u8, 0, 0) } as u16;
            if vendor_id == 0xFFFF { continue; }

            for func in 0..8 {
                let vendor_id = unsafe { pci_config_read(bus as u8, slot as u8, func as u8, 0) } as u16;
                if vendor_id != 0xFFFF {
                    let device_id = (unsafe { pci_config_read(bus as u8, slot as u8, func as u8, 0) } >> 16) as u16;
                    let class_rev = unsafe { pci_config_read(bus as u8, slot as u8, func as u8, 8) };
                    let class_id = (class_rev >> 24) as u8;
                    let subclass_id = (class_rev >> 16) as u8;
                    let prog_if = (class_rev >> 8) as u8;

                    devices.push(PciDevice {
                        bus: bus as u8,
                        dev: slot as u8,
                        func: func as u8,
                        vendor_id,
                        device_id,
                        class_id,
                        subclass_id,
                        prog_if,
                        ecam: 0,
                    });
                }
            }
        }
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
    for b in 0..256u16 {
        for d in 0..32u8 {
            for f in 0..8u8 {
                let id = unsafe { pci_config_read(b as u8, d, f, 0) };
                if id & 0xffff == 0xffff { if f == 0 { break; } continue; }
                // Bridges (header type 1) have two BARs, ordinary functions six.
                let header = (unsafe { pci_config_read(b as u8, d, f, 0x0c) } >> 16) as u8;
                let bars = match header & 0x7f { 0 => 6u8, 1 => 2, _ => 0 };
                let mut i = 0u8;
                while i < bars {
                    let low = unsafe { pci_config_read(b as u8, d, f, 0x10 + i * 4) };
                    let wide = low & 1 == 0 && (low >> 1) & 3 == 2 && i + 1 < bars;
                    let base = if low & 1 != 0 { 0 } else {
                        (low & !15) as u64 | if wide { (unsafe { pci_config_read(b as u8, d, f, 0x14 + i * 4) } as u64) << 32 } else { 0 }
                    };
                    let own = (b as u8, d, f, i) == (bus, dev, func, index);
                    if !own && base != 0 && base >= page && base < page + 4096 { return false; }
                    i += if wide { 2 } else { 1 };
                }
                if f == 0 && header & 0x80 == 0 { break; }
            }
        }
    }
    true
}
