use crate::hal::{HAL, HardwareAbstractionLayer};
use crate::serial_println;
use crate::ipc::DOMAIN_REGISTRY;
use crate::capability::{CapabilityData, PciCapData, InterruptCapData, ProtectionDomain};
use sex_hwdb::manifest::Lease;

// Which device goes to which service is data: the driver manifests in
// `sex_hwdb::manifest`. This file only builds leases of each kind.

pub fn init(sexdrive_pd_id: u32, sexdisplay_pd_id: u32, sexusb_pd_id: u32, sexnet_pd_id: u32, sexbell_pd_id: u32) {
    serial_println!("DevMgr: Starting hardware discovery...");

    let devices = HAL.enumerate_pci();
    serial_println!("DevMgr: Enumerated {} PCI devices.", devices.len());

    let mut census = sex_hwdb::Census::default();
    for dev in &devices { census_line(&mut census, dev); }
    serial_println!("[hw.census] devices={} working={} in_progress={} planned={} unsupported={} warnings={}",
        census.devices, census.working, census.in_progress, census.planned, census.unsupported, census.warnings);

    let behind_vmd = devices.iter().any(|d| sex_hwdb::is_vmd(id(d)));
    let services = Services { drive: sexdrive_pd_id, display: sexdisplay_pd_id, usb: sexusb_pd_id, net: sexnet_pd_id, bell: sexbell_pd_id };
    let mut held = [0u8; sex_hwdb::manifest::ALL.len()];
    let mut nvme = 0u32;
    for dev in devices {
        let Some((manifest, entry)) = sex_hwdb::manifest::claim(id(&dev)) else { continue };
        let index = sex_hwdb::manifest::ALL.iter().position(|m| m.service == manifest.service).unwrap_or(0);
        if entry.lease == Lease::Nvme { nvme += 1; }
        let owner = services.owner(manifest.service);
        let Some(pd) = DOMAIN_REGISTRY.get(owner).filter(|_| owner != 0) else {
            serial_println!("[devmgr.lease.skip] {:02x}:{:02x}.{} {:04x}:{:04x} service={} reason=service_absent",
                dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id, manifest.service.name());
            continue;
        };
        if manifest.max_devices != 0 && held[index] >= manifest.max_devices {
            serial_println!("[devmgr.lease.skip] {:02x}:{:02x}.{} {:04x}:{:04x} service={} reason=manifest_limit max={}",
                dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id, manifest.service.name(), manifest.max_devices);
            continue;
        }
        // The first device keeps the slot its driver always read; later ones of the same
        // service take the next free extra slot, so none ever replaces another.
        let slot = if entry.slot == 0 { 0 }
            else if pd.find_capability(entry.slot as u32).is_none() { entry.slot }
            else {
                let base = sex_pdx::device::EXTRA_SLOT_BASE;
                match (base..base + sex_pdx::device::EXTRA_SLOTS).find(|s| pd.find_capability(*s as u32).is_none()) {
                    Some(s) => s,
                    None => {
                        serial_println!("[devmgr.lease.skip] {:02x}:{:02x}.{} service={} reason=no_free_slot", dev.bus, dev.dev, dev.func, manifest.service.name());
                        continue;
                    }
                }
            };
        let first = slot == entry.slot;
        let leased = match entry.lease {
            Lease::Nvme => nvme_lease(&dev, pd, slot),
            Lease::Xhci => xhci_lease(&dev, pd, slot),
            Lease::Nic => nic_lease(&dev, pd, slot),
            // HDMI/DisplayPort-only audio on a graphics card plays nothing from the
            // speakers; it must not take the sound slot.
            Lease::Hda if crate::device_lease::display_audio(&dev) => {
                serial_println!("[kernel.cap.class.skip] kind=Hda reason=display_audio vendor={:04x} device={:04x}", dev.vendor_id, dev.device_id);
                false
            }
            Lease::Hda => class_lease(&dev, crate::device_lease::Kind::Hda, pd, slot, first),
            Lease::Ahci => class_lease(&dev, crate::device_lease::Kind::Ahci, pd, slot, first),
            Lease::Wifi => class_lease(&dev, crate::device_lease::Kind::Wifi, pd, slot, first),
            Lease::Virtio => virtio(&dev, pd, slot, first),
            Lease::Display => display_lease(&dev, pd),
        };
        if !leased { continue; }
        held[index] += 1;
        serial_println!("[devmgr.lease] {:02x}:{:02x}.{} {:04x}:{:04x} service={} pd={} slot={} kind={:?} nth={}",
            dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id, manifest.service.name(), owner, slot, entry.lease, held[index]);
        if slot != 0 { crate::device_irq::record(owner, slot, &dev, manifest.vectors); }
    }
    serial_println!("[devmgr.manifest] services={} drive={} usb={} net={} bell={} display={}",
        sex_hwdb::manifest::ALL.len(), held[0], held[1], held[2], held[3], held[4]);

    if nvme == 0 {
        serial_println!("[kernel.pci.nvme.absent] behind_vmd={}", behind_vmd as u8);
    }
    serial_println!("DevMgr: Hardware discovery complete.");
}

fn id(dev: &crate::hal::pci::PciDevice) -> sex_hwdb::Id {
    sex_hwdb::Id { vendor: dev.vendor_id, device: dev.device_id, class: dev.class_id, subclass: dev.subclass_id, prog_if: dev.prog_if }
}

/// The domains BOOT started for each driver service.
struct Services { drive: u32, display: u32, usb: u32, net: u32, bell: u32 }
impl Services {
    fn owner(&self, service: sex_hwdb::Service) -> u32 {
        use sex_hwdb::Service::*;
        match service {
            Drive => self.drive,
            // Older boot graphs ran the xHCI probe inside the drive service.
            Usb => if self.usb != 0 { self.usb } else { self.drive },
            Net => self.net,
            Bell => self.bell,
            Display => self.display,
            _ => 0,
        }
    }
}

/// NVMe: BAR0 view; firmware may leave the controller cold with DMA off, so BOOT
/// turns memory space and bus mastering on (the status half is write-one-to-clear).
fn nvme_lease(dev: &crate::hal::pci::PciDevice, pd: &ProtectionDomain, slot: u64) -> bool {
    serial_println!("[kernel.pci.nvme.found] {:02x}:{:02x}.{} vendor={:04x} device={:04x}", dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id);
    dev.write_u32(0x04, (dev.read_u32(0x04) & 0xFFFF) | 0x0006);
    pd.grant_capability(slot, CapabilityData::Pci(PciCapData {
        bus: dev.bus, dev: dev.dev, func: dev.func, vendor_id: dev.vendor_id, device_id: dev.device_id,
        bar0_view: Some(crate::boot_memory::pci_bar0(pd, dev, slot)),
        command: None,
        virtio: None, class: 0, window: (0, 0),
    }));
    serial_println!("[kernel.cap.nvme_bar.grant] pd={} slot={}", pd.id, slot);
    true
}

fn xhci_lease(dev: &crate::hal::pci::PciDevice, pd: &ProtectionDomain, slot: u64) -> bool {
    let irq_line = (dev.read_u32(0x3C) & 0xFF) as u8;
    serial_println!("DevMgr: Discovered USB XHCI ({:02x}:{:02x}.{}) vendor={:04x} device={:04x} bar0={:#x} irq_line={:#x}",
        dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id, dev.get_bar(0), irq_line);
    // Strict lease boundary: one xHCI PCI capability per driver slot.
    pd.grant_capability(slot, CapabilityData::Pci(PciCapData {
        bus: dev.bus, dev: dev.dev, func: dev.func, vendor_id: dev.vendor_id, device_id: dev.device_id,
        bar0_view: Some(crate::boot_memory::pci_bar0(pd, dev, slot)),
        command: None,
        virtio: None, class: 0, window: (0, 0),
    }));
    pd.grant(CapabilityData::Interrupt(InterruptCapData { irq: irq_line }));
    serial_println!("DevMgr: Leased XHCI to pd={} slot={}", pd.id, slot);
    true
}

/// Wired Ethernet: only parts whose register layout the driver ABI knows.
fn nic_lease(dev: &crate::hal::pci::PciDevice, pd: &ProtectionDomain, slot: u64) -> bool {
    serial_println!("[kernel.pci.nic.found] {:02x}:{:02x}.{} vendor={:04x} device={:04x}", dev.bus, dev.dev, dev.func, dev.vendor_id, dev.device_id);
    let Some((view, window)) = crate::boot_memory::nic_lease(pd, dev, slot) else {
        serial_println!("[kernel.cap.nic.skip] reason=unsupported_or_invalid_bar vendor={:04x} device={:04x}", dev.vendor_id, dev.device_id);
        return false;
    };
    pd.grant_capability(slot, CapabilityData::Pci(PciCapData {
        bus: dev.bus, dev: dev.dev, func: dev.func, vendor_id: dev.vendor_id, device_id: dev.device_id,
        bar0_view: Some(view),
        command: Some(crate::boot_memory::nic_command(pd, dev, slot, view)),
        virtio: None, class: 0, window,
    }));
    serial_println!("[kernel.cap.nic.grant] pd={} slot={} vendor={:04x} device={:04x} model={:?}", pd.id, slot,
        dev.vendor_id, dev.device_id, crate::nic_boot::model(dev.vendor_id, dev.device_id));
    true
}

/// Display controllers: identity only; the picture still comes from firmware.
fn display_lease(dev: &crate::hal::pci::PciDevice, pd: &ProtectionDomain) -> bool {
    serial_println!("DevMgr: Assigning GPU ({:02x}:{:02x}.{}) to sexdisplay", dev.bus, dev.dev, dev.func);
    pd.grant(CapabilityData::Pci(PciCapData {
        bus: dev.bus, dev: dev.dev, func: dev.func, vendor_id: dev.vendor_id, device_id: dev.device_id,
        bar0_view: None,
        command: None,
        virtio: None, class: 0, window: (0, 0),
    }));
    true
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

/// Modern virtio PCI: the one BAR holding all four register blocks.
fn virtio(dev: &crate::hal::pci::PciDevice, pd: &ProtectionDomain, slot: u64, first: bool) -> bool {
    serial_println!("[kernel.pci.virtio.found] {:02x}:{:02x}.{} product={:#06x}", dev.bus, dev.dev, dev.func, dev.device_id);
    let Some((bar, layout)) = crate::virtio_boot::layout(dev) else {
        serial_println!("[kernel.cap.virtio.skip] product={:#06x} reason=no_modern_layout", dev.device_id); return false;
    };
    let Some(view) = crate::boot_memory::virtio_bar(pd, dev, slot, bar, &layout) else {
        serial_println!("[kernel.cap.virtio.skip] product={:#06x} reason=invalid_bar bar={}", dev.device_id, bar); return false;
    };
    let command = crate::boot_memory::virtio_command(pd, dev, slot, view, &layout);
    pd.grant_capability(slot, CapabilityData::Pci(PciCapData {
        bus: dev.bus, dev: dev.dev, func: dev.func, vendor_id: dev.vendor_id, device_id: dev.device_id,
        bar0_view: Some(view), command: Some(command), virtio: Some(layout), class: 0, window: (0, 0),
    }));
    // One DMA pool per service: a second device shares the first one's.
    if first { crate::boot_memory::virtio_dma(pd.id); }
    serial_println!("[kernel.cap.virtio.grant] pd={} slot={} product={:#06x} bar={} common={:#x} notify={:#x} isr={:#x} device={:#x} multiplier={}",
        pd.id, slot, dev.device_id, bar, layout.common.offset, layout.notify.offset, layout.isr.offset, layout.device.offset, layout.notify_multiplier);
    true
}

/// HD Audio goes to the Bell service, AHCI to the drive service, Intel Wi-Fi to
/// the network service: one BAR view, memory and bus-master command bits, and
/// (for the first device) a DMA pool.
fn class_lease(dev: &crate::hal::pci::PciDevice, kind: crate::device_lease::Kind, pd: &ProtectionDomain, slot: u64, first: bool) -> bool {
    serial_println!("[kernel.pci.class.found] {:02x}:{:02x}.{} kind={:?} vendor={:04x} device={:04x}",
        dev.bus, dev.dev, dev.func, kind, dev.vendor_id, dev.device_id);
    let Some((view, window)) = crate::boot_memory::class_bar(pd, dev, slot, kind) else {
        serial_println!("[kernel.cap.class.skip] kind={:?} reason=invalid_bar bar={}", kind, kind.bar()); return false;
    };
    let command = crate::boot_memory::class_command(pd, dev, slot, view, kind);
    pd.grant_capability(slot, CapabilityData::Pci(PciCapData {
        bus: dev.bus, dev: dev.dev, func: dev.func, vendor_id: dev.vendor_id, device_id: dev.device_id,
        bar0_view: Some(view), command: Some(command), virtio: None, class: kind.class(dev), window,
    }));
    if first { crate::boot_memory::class_dma(pd.id, kind); }
    serial_println!("[kernel.cap.class.grant] pd={} slot={} kind={:?} bytes={}", pd.id, slot, kind, view.length);
    true
}
