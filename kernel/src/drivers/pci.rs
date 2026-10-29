use crate::serial_println;
use crate::ipc::DOMAIN_REGISTRY;
use crate::capability::CapabilityData;

#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8, pub dev: u8, pub func: u8, pub vendor_id: u16, pub device_id: u16, pub class_id: u8, pub subclass_id: u8,
}

impl PciDevice {
    pub fn read_u32(&self, offset: u8) -> u32 {
        let address: u32 = 0x80000000 | ((self.bus as u32) << 16) | ((self.dev as u32) << 11) | ((self.func as u32) << 8) | (offset as u32 & 0xFC);
        unsafe {
            x86_64::instructions::port::Port::new(0xCF8).write(address);
            x86_64::instructions::port::Port::new(0xCFC).read()
        }
    }
}

pub fn enumerate_display_devices() -> alloc::vec::Vec<PciDevice> {
    let mut devices = alloc::vec::Vec::new();
    for bus in 0..8 {
        for dev in 0..32 {
            let d = PciDevice { bus, dev, func: 0, vendor_id: 0, device_id: 0, class_id: 0, subclass_id: 0 };
            let config0 = d.read_u32(0);
            let vendor = (config0 & 0xFFFF) as u16;
            if vendor == 0xFFFF { continue; }

            let config8 = d.read_u32(0x08);
            let class = (config8 >> 24) as u8;
            let subclass = (config8 >> 16) as u8;
            let device_id = (config0 >> 16) as u16;

            if class == 0x03 { // Display Controller
                devices.push(PciDevice {
                    bus, dev, func: 0,
                    vendor_id: vendor,
                    device_id,
                    class_id: class,
                    subclass_id: subclass
                });
            }
        }
    }
    devices
}

pub fn bootstrap_drivers(sexdrive_pd_id: u32, sexdisplay_pd_id: u32) {
    serial_println!("pci: Starting hardware enumeration...");
    for bus in 0..8 {
        for dev in 0..32 {
            let d = PciDevice { bus, dev, func: 0, vendor_id: 0, device_id: 0, class_id: 0, subclass_id: 0 };
            let config0 = d.read_u32(0);
            let vendor = (config0 & 0xFFFF) as u16;
            if vendor == 0xFFFF { continue; }
            
            let config8 = d.read_u32(0x08);
            let class = (config8 >> 24) as u8;
            let subclass = (config8 >> 16) as u8;

            if class == 0x01 && subclass == 0x08 { // NVMe
                serial_println!("pci: Found NVMe at {:02x}:{:02x}.0", bus, dev);
                if let Some(pd) = DOMAIN_REGISTRY.get(sexdrive_pd_id) {
                    pd.grant(CapabilityData::Pci(crate::capability::PciCapData { bus, dev, func: 0, vendor_id: vendor, device_id: 0, bar0_view: None }));
                    crate::interrupts::register_irq_route(0x22, sexdrive_pd_id);
                }
            } else if class == 0x03 { // GPU
                serial_println!("pci: Found GPU at {:02x}:{:02x}.0", bus, dev);
                if let Some(pd) = DOMAIN_REGISTRY.get(sexdisplay_pd_id) {
                    pd.grant(CapabilityData::Pci(crate::capability::PciCapData { bus, dev, func: 0, vendor_id: vendor, device_id: 0, bar0_view: None }));
                    crate::interrupts::register_irq_route(0x23, sexdisplay_pd_id);
                }
            }
        }
    }
}
