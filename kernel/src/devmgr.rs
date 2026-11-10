use crate::hal::{HAL, HardwareAbstractionLayer};
use crate::serial_println;
use crate::ipc::DOMAIN_REGISTRY;
use crate::capability::{CapabilityData, PciCapData, InterruptCapData};

// First NVMe at 16, second (the other half of an Intel RAID 0) at 160.
use sex_pdx::{SLOT_NVME_HOST, SLOT_NVME_HOST_2};

pub fn init(sexdrive_pd_id: u32, sexdisplay_pd_id: u32, sexusb_pd_id: u32, sexnet_pd_id: u32, sexbell_pd_id: u32) {
    serial_println!("DevMgr: Starting hardware discovery...");

    let mut devices = HAL.enumerate_pci();
    // Drives behind Intel VMD (RST "RAID On") join the list as ordinary devices.
    crate::vmd::expand(&mut devices);
    serial_println!("DevMgr: Enumerated {} PCI devices.", devices.len());

    let mut census = sex_hwdb::Census::default();
    for dev in &devices { census_line(&mut census, dev); }
    serial_println!("[hw.census] devices={} working={} in_progress={} planned={} unsupported={} warnings={}",
        census.devices, census.working, census.in_progress, census.planned, census.unsupported, census.warnings);

    let behind_vmd = devices.iter().any(|d| sex_hwdb::is_vmd(sex_hwdb::Id { vendor: d.vendor_id, device: d.device_id, class: d.class_id, subclass: d.subclass_id, prog_if: d.prog_if }));
    let mut nvme_found = false;
    let mut nvme_count = 0u32;
    for dev in devices {
        match (dev.class_id, dev.subclass_id) {
            (0x01, 0x08) => { // NVMe
                serial_println!("[kernel.pci.nvme.found] {:02x}:{:02x}.{} vendor={:04x} device={:04x} behind_vmd={}", dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id, (dev.ecam != 0) as u8);
                let slot = match nvme_count { 0 => SLOT_NVME_HOST, 1 => SLOT_NVME_HOST_2, _ => {
                    serial_println!("[kernel.pci.nvme.extra] {:02x}:{:02x}.{} leased=0", dev.bus, dev.dev, dev.func);
                    continue;
                } };
                nvme_count += 1;
                let bar0_pa = dev.get_bar(0);
                serial_println!("[kernel.pci.nvme.bar0] pa={:#x}", bar0_pa);
                // Firmware may leave the controller cold, with DMA off. BOOT turns it on
                // here; the status half of this register is write-one-to-clear.
                let command = dev.read_u32(0x04) & 0xFFFF;
                dev.write_u32(0x04, command | 0x0006);
                if let Some(pd) = DOMAIN_REGISTRY.get(sexdrive_pd_id) {
                    // Granted at its slot so sexdrive can call MAP_PCI_BAR(slot, ...).
                    pd.grant_capability(slot, CapabilityData::Pci(PciCapData {
                        bus: dev.bus,
                        dev: dev.dev,
                        func: dev.func,
                        vendor_id: dev.vendor_id,
                        device_id: dev.device_id,
                        bar0_view: Some(crate::boot_memory::pci_bar0(pd, &dev, slot)),
                        command: None,
                        virtio: None, class: 0, window: (0, 0),
                    }));
                    serial_println!("[kernel.cap.nvme_bar.grant] pd={} slot={}", sexdrive_pd_id, slot);
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
                        virtio: None, class: 0, window: (0, 0),
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
                        virtio: None, class: 0, window: (0, 0),
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
                        let Some((view,window))=crate::boot_memory::nic_lease(pd,&dev,sex_pdx::SLOT_NIC)else{
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
                            virtio: None, class: 0, window,
                        }));
                        serial_println!("[kernel.cap.nic.grant] pd={} slot=19 vendor={:04x} device={:04x} model={:?}", sexnet_pd_id,
                            dev.vendor_id, dev.device_id, crate::nic_boot::model(dev.vendor_id, dev.device_id));
                    }
                }
            }
            // HDMI/DisplayPort-only audio on a graphics card would hold Bell's one
            // sound slot and play nothing from the speakers; leave it alone.
            (0x04, 0x03) if crate::device_lease::display_audio(&dev) => {
                serial_println!("[kernel.cap.class.skip] kind=Hda reason=display_audio vendor={:04x} device={:04x}", dev.vendor_id, dev.device_id);
            }
            (0x04, 0x03) => class_lease(&dev, crate::device_lease::Kind::Hda, sexbell_pd_id, sex_pdx::virtio::SLOT_SOUND),
            (0x01, 0x06) if dev.prog_if == 0x01 => class_lease(&dev, crate::device_lease::Kind::Ahci, sexdrive_pd_id, sex_pdx::pci_command::SLOT_AHCI_HOST),
            _ if dev.vendor_id == sex_pdx::virtio::VENDOR => virtio(&dev, sexbell_pd_id),
            _ => {}
        }
    }

    if !nvme_found {
        serial_println!("[kernel.pci.nvme.absent] behind_vmd={}", behind_vmd as u8);
    }
    serial_println!("DevMgr: Hardware discovery complete.");
}

/// One line per device naming it, the service that owns its kind, and how far that
/// support goes, so a first boot on new hardware says what works. Description only:
/// leases are decided below, never by the table.
fn census_line(census: &mut sex_hwdb::Census, dev: &crate::hal::pci::PciDevice) {
    let id = sex_hwdb::Id { vendor: dev.vendor_id, device: dev.device_id, class: dev.class_id, subclass: dev.subclass_id, prog_if: dev.prog_if };
    let d = census.add(id);
    let model = if d.model.is_empty() { sex_hwdb::class_name(id.class, id.subclass, id.prog_if) } else { d.model };
    serial_println!("[hw.device] {:02x}:{:02x}.{} {:04x}:{:04x} class={:02x}{:02x}{:02x} {} {} service={} support={}",
        dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id, dev.class_id, dev.subclass_id, dev.prog_if,
        sex_hwdb::vendor_name(id.vendor), model, d.service.name(), d.support.name());
    if let Some(text) = sex_hwdb::advice(id) {
        serial_println!("[hw.warning] {:02x}:{:02x}.{} {}", dev.bus, dev.dev, dev.func, text);
    }
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
        bar0_view: Some(view), command: Some(command), virtio: Some(layout), class: 0, window: (0, 0),
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
    let Some((view, window)) = crate::boot_memory::class_bar(pd, dev, slot, kind) else {
        serial_println!("[kernel.cap.class.skip] kind={:?} reason=invalid_bar bar={}", kind, kind.bar()); return;
    };
    let command = crate::boot_memory::class_command(pd, dev, slot, view, kind);
    pd.grant_capability(slot, CapabilityData::Pci(PciCapData {
        bus: dev.bus, dev: dev.dev, func: dev.func, vendor_id: dev.vendor_id, device_id: dev.device_id,
        bar0_view: Some(view), command: Some(command), virtio: None, class: kind.class(dev), window,
    }));
    crate::boot_memory::class_dma(owner, kind);
    serial_println!("[kernel.cap.class.grant] pd={} slot={} kind={:?} bytes={}", owner, slot, kind, view.length);
}
