#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8, pub dev: u8, pub func: u8, pub vendor_id: u16, pub device_id: u16, pub class_id: u8, pub subclass_id: u8,
}

impl PciDevice {
    pub fn read_u32(&self, offset: u8) -> u32 {
        unsafe { crate::hal::pci::pci_config_read(self.bus,self.dev,self.func,offset) }
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
