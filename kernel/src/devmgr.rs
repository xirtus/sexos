use crate::hal::{HAL, HardwareAbstractionLayer};
use crate::serial_println;
use crate::ipc::DOMAIN_REGISTRY;
use crate::capability::{CapabilityData, PciCapData, InterruptCapData};

// SLOT_NVME_HOST = 16 — slotted BAR lease for sexdrive NVMe probe.
// Not added to sex-pdx to avoid ABI hash churn; canonical slot number only here.
const SLOT_NVME_HOST: u64 = 16;

pub fn init(sexdrive_pd_id: u32, sexdisplay_pd_id: u32, sexusb_pd_id: u32, sexnet_pd_id: u32) {
    serial_println!("DevMgr: Starting hardware discovery...");

    let devices = HAL.enumerate_pci();
    serial_println!("DevMgr: Enumerated {} PCI devices.", devices.len());

    let mut nvme_found = false;
    for dev in devices {
        match (dev.class_id, dev.subclass_id) {
            (0x01, 0x08) => { // NVMe
                serial_println!("[kernel.pci.nvme.found] {:02x}:{:02x}.{} vendor={:04x} device={:04x}", dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id);
                let bar0_pa = dev.get_bar(0);
                serial_println!("[kernel.pci.nvme.bar0] pa={:#x}", bar0_pa);
                if let Some(pd) = DOMAIN_REGISTRY.get(sexdrive_pd_id) {
                    // Grant at SLOT_NVME_HOST (16) so sexdrive can call MAP_PCI_BAR(16, ...).
                    pd.grant_capability(SLOT_NVME_HOST, CapabilityData::Pci(PciCapData {
                        bus: dev.bus,
                        dev: dev.dev,
                        func: dev.func,
                        vendor_id: dev.vendor_id,
                        device_id: dev.device_id,
                        bar0_view: Some(crate::boot_memory::pci_bar0(pd, &dev, SLOT_NVME_HOST)),
                    }));
                    serial_println!("[kernel.cap.nvme_bar.grant] pd={} slot={}", sexdrive_pd_id, SLOT_NVME_HOST);
                    // Route IRQ (Simplified vector mapping)
                    crate::interrupts::register_irq_route(0x22, sexdrive_pd_id);
                }
                nvme_found = true;
            }
            (0x03, _) => { // GPU
                serial_println!("DevMgr: Assigning GPU ({:02x}:{:02x}.{}) to sexdisplay", dev.bus, dev.dev, dev.func);
                if let Some(pd) = DOMAIN_REGISTRY.get(sexdisplay_pd_id) {
                    pd.grant(CapabilityData::Pci(PciCapData {
                        bus: dev.bus,
                        dev: dev.dev,
                        func: dev.func,
                        vendor_id: dev.vendor_id,
                        device_id: dev.device_id,
                        bar0_view: None,
                    }));
                    crate::interrupts::register_irq_route(0x23, sexdisplay_pd_id);
                }
            }
            (0x0c, 0x03) if dev.prog_if == 0x30 => { // USB XHCI
                let bar0 = dev.get_bar(0);
                let irq_line = ((dev.read_u32(0x3C) >> 0) & 0xFF) as u8;
                serial_println!(
                    "DevMgr: Discovered USB XHCI ({:02x}:{:02x}.{}) vendor={:04x} device={:04x} bar0={:#x} irq_line={:#x}",
                    dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id, bar0, irq_line
                );
                let usb_owner_pd_id = if sexusb_pd_id != 0 { sexusb_pd_id } else { sexdrive_pd_id };
                if let Some(pd) = DOMAIN_REGISTRY.get(usb_owner_pd_id) {
                    // Strict lease boundary: one XHCI PCI capability to one driver PD slot.
                    pd.grant_capability(sex_pdx::SLOT_USB_HOST, CapabilityData::Pci(PciCapData {
                        bus: dev.bus,
                        dev: dev.dev,
                        func: dev.func,
                        vendor_id: dev.vendor_id,
                        device_id: dev.device_id,
                        bar0_view: Some(crate::boot_memory::pci_bar0(pd, &dev, sex_pdx::SLOT_USB_HOST)),
                    }));
                    pd.grant(CapabilityData::Interrupt(InterruptCapData { irq: irq_line }));
                    serial_println!(
                        "DevMgr: Leased XHCI to pd={} slot={}",
                        usb_owner_pd_id,
                        sex_pdx::SLOT_USB_HOST
                    );
                }
            }
            (0x0c, _) => { // USB (other)
                serial_println!("DevMgr: Discovered USB controller class={:02x}:{:02x} ({:02x}:{:02x}.{}) vendor={:04x} device={:04x}", dev.class_id, dev.subclass_id, dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id);
            }
            (0x02, 0x00) => { // Ethernet NIC
                serial_println!(
                    "[kernel.pci.nic.found] {:02x}:{:02x}.{} vendor={:04x} device={:04x}",
                    dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id
                );
                if sexnet_pd_id != 0 {
                    if let Some(pd) = DOMAIN_REGISTRY.get(sexnet_pd_id) {
                        pd.grant_capability(sex_pdx::SLOT_NIC, CapabilityData::Pci(PciCapData {
                            bus: dev.bus,
                            dev: dev.dev,
                            func: dev.func,
                            vendor_id: dev.vendor_id,
                            device_id: dev.device_id,
                            bar0_view: Some(crate::boot_memory::pci_bar0(pd, &dev, sex_pdx::SLOT_NIC)),
                        }));
                        serial_println!("[kernel.cap.nic.grant] pd={} slot=19", sexnet_pd_id);
                    }
                }
            }
            _ => {}
        }
    }

    if !nvme_found {
        serial_println!("[kernel.pci.nvme.absent]");
    }
    serial_println!("DevMgr: Hardware discovery complete.");
}
