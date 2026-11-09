use crate::hal::{HAL, HardwareAbstractionLayer};
use crate::serial_println;
use crate::ipc::DOMAIN_REGISTRY;
use crate::capability::{CapabilityData, PciCapData, InterruptCapData};

// SLOT_NVME_HOST = 16 — slotted BAR lease for sexdrive NVMe probe.
// Not added to sex-pdx to avoid ABI hash churn; canonical slot number only here.
const SLOT_NVME_HOST: u64 = 16;

pub fn init(sexdrive_pd_id: u32, sexdisplay_pd_id: u32, sexusb_pd_id: u32, sexnet_pd_id: u32, sexbell_pd_id: u32) {
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
                // Firmware may leave the controller cold, with DMA off. BOOT turns it on
                // here; the status half of this register is write-one-to-clear.
                let command = unsafe { crate::hal::pci::pci_config_read(dev.bus, dev.dev, dev.func, 0x04) } & 0xFFFF;
                unsafe { crate::hal::pci::pci_config_write(dev.bus, dev.dev, dev.func, 0x04, command | 0x0006); }
                if let Some(pd) = DOMAIN_REGISTRY.get(sexdrive_pd_id) {
                    // Grant at SLOT_NVME_HOST (16) so sexdrive can call MAP_PCI_BAR(16, ...).
                    pd.grant_capability(SLOT_NVME_HOST, CapabilityData::Pci(PciCapData {
                        bus: dev.bus,
                        dev: dev.dev,
                        func: dev.func,
                        vendor_id: dev.vendor_id,
                        device_id: dev.device_id,
                        bar0_view: Some(crate::boot_memory::pci_bar0(pd, &dev, SLOT_NVME_HOST)),
                        command: None,
                        virtio: None, class: 0,
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
                        command: None,
                        virtio: None, class: 0,
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
                        command: None,
                        virtio: None, class: 0,
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
                        if pd.find_capability(sex_pdx::SLOT_NIC as u32).is_some(){
                            serial_println!("[kernel.cap.nic.skip] reason=slot_occupied vendor={:04x} device={:04x}",dev.vendor_id,dev.device_id);
                            continue;
                        }
                        let Some(view)=crate::boot_memory::nic_bar0(pd,&dev,sex_pdx::SLOT_NIC)else{
                            serial_println!("[kernel.cap.nic.skip] reason=unsupported_or_invalid_bar vendor={:04x} device={:04x}",dev.vendor_id,dev.device_id);
                            continue;
                        };
                        pd.grant_capability(sex_pdx::SLOT_NIC, CapabilityData::Pci(PciCapData {
                            bus: dev.bus,
                            dev: dev.dev,
                            func: dev.func,
                            vendor_id: dev.vendor_id,
                            device_id: dev.device_id,
                            bar0_view: Some(view),
                            command: Some(crate::boot_memory::nic_command(pd,&dev,sex_pdx::SLOT_NIC,view)),
                            virtio: None, class: 0,
                        }));
                        serial_println!("[kernel.cap.nic.grant] pd={} slot=19 vendor={:04x} device={:04x} model={:?}", sexnet_pd_id,
                            dev.vendor_id, dev.device_id, crate::nic_boot::model(dev.vendor_id, dev.device_id));
                    }
                }
            }
            (0x04, 0x03) => class_lease(&dev, crate::device_lease::Kind::Hda, sexbell_pd_id, sex_pdx::virtio::SLOT_SOUND),
            (0x01, 0x06) if dev.prog_if == 0x01 => class_lease(&dev, crate::device_lease::Kind::Ahci, sexdrive_pd_id, sex_pdx::pci_command::SLOT_AHCI_HOST),
            _ if dev.vendor_id == sex_pdx::virtio::VENDOR => virtio(&dev, sexbell_pd_id),
            _ => {}
        }
    }

    if !nvme_found {
        serial_println!("[kernel.pci.nvme.absent]");
    }
    serial_println!("DevMgr: Hardware discovery complete.");
}

/// Virtio devices go to the service that drives that kind of device: sound to
/// the Bell service. One device per kind; later ones are left alone.
fn virtio(dev: &crate::hal::pci::PciDevice, sexbell_pd_id: u32) {
    serial_println!("[kernel.pci.virtio.found] {:02x}:{:02x}.{} product={:#06x}", dev.bus, dev.dev, dev.func, dev.device_id);
    let (owner, slot) = match dev.device_id {
        sex_pdx::virtio::SOUND => (sexbell_pd_id, sex_pdx::virtio::SLOT_SOUND),
        _ => { serial_println!("[kernel.cap.virtio.skip] product={:#06x} reason=no_driver", dev.device_id); return; }
    };
    let Some(pd) = DOMAIN_REGISTRY.get(owner).filter(|_| owner != 0) else {
        serial_println!("[kernel.cap.virtio.skip] product={:#06x} reason=service_absent", dev.device_id); return;
    };
    if pd.find_capability(slot as u32).is_some() {
        serial_println!("[kernel.cap.virtio.skip] product={:#06x} reason=slot_occupied", dev.device_id); return;
    }
    let Some((bar, layout)) = crate::virtio_boot::layout(dev) else {
        serial_println!("[kernel.cap.virtio.skip] product={:#06x} reason=no_modern_layout", dev.device_id); return;
    };
    let Some(view) = crate::boot_memory::virtio_bar(pd, dev, slot, bar, &layout) else {
        serial_println!("[kernel.cap.virtio.skip] product={:#06x} reason=invalid_bar bar={}", dev.device_id, bar); return;
    };
    let command = crate::boot_memory::virtio_command(pd, dev, slot, view, &layout);
    pd.grant_capability(slot, CapabilityData::Pci(PciCapData {
        bus: dev.bus, dev: dev.dev, func: dev.func, vendor_id: dev.vendor_id, device_id: dev.device_id,
        bar0_view: Some(view), command: Some(command), virtio: Some(layout), class: 0,
    }));
    crate::boot_memory::virtio_dma(owner);
    serial_println!("[kernel.cap.virtio.grant] pd={} slot={} product={:#06x} bar={} common={:#x} notify={:#x} isr={:#x} device={:#x} multiplier={}",
        owner, slot, dev.device_id, bar, layout.common.offset, layout.notify.offset, layout.isr.offset, layout.device.offset, layout.notify_multiplier);
}

/// HD Audio goes to the Bell service, AHCI to the drive service: one BAR view,
/// memory and bus-master command bits, and a DMA pool. One controller per kind;
/// later ones (and a sound card when virtio sound came first) are left alone.
fn class_lease(dev: &crate::hal::pci::PciDevice, kind: crate::device_lease::Kind, owner: u32, slot: u64) {
    serial_println!("[kernel.pci.class.found] {:02x}:{:02x}.{} kind={:?} vendor={:04x} device={:04x}",
        dev.bus, dev.dev, dev.func, kind, dev.vendor_id, dev.device_id);
    let Some(pd) = DOMAIN_REGISTRY.get(owner).filter(|_| owner != 0) else {
        serial_println!("[kernel.cap.class.skip] kind={:?} reason=service_absent", kind); return;
    };
    if pd.find_capability(slot as u32).is_some() {
        serial_println!("[kernel.cap.class.skip] kind={:?} reason=slot_occupied", kind); return;
    }
    let Some(view) = crate::boot_memory::class_bar(pd, dev, slot, kind) else {
        serial_println!("[kernel.cap.class.skip] kind={:?} reason=invalid_bar bar={}", kind, kind.bar()); return;
    };
    let command = crate::boot_memory::class_command(pd, dev, slot, view, kind);
    pd.grant_capability(slot, CapabilityData::Pci(PciCapData {
        bus: dev.bus, dev: dev.dev, func: dev.func, vendor_id: dev.vendor_id, device_id: dev.device_id,
        bar0_view: Some(view), command: Some(command), virtio: None, class: kind.class(dev),
    }));
    crate::boot_memory::class_dma(owner, kind);
    serial_println!("[kernel.cap.class.grant] pd={} slot={} kind={:?} bytes={}", owner, slot, kind, view.length);
}
