// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! What a PCI device is, which SexOS service drives it, and how far that support goes.
//!
//! BOOT reads this table once while it enumerates PCI, and prints one census line per
//! device, so the first boot on a new machine says plainly what works and what does not.
//! The table is data, not authority: it never leases a device. Leases stay in devmgr.
//! Later a userland device manager will match drivers from the same table.
//!
//! Exact ids come first; when an id is not listed the class decides. Support levels
//! describe the state of master, not a promise; `InProgress` names work in an open branch.
#![no_std]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id { pub vendor: u16, pub device: u16, pub class: u8, pub subclass: u8, pub prog_if: u8 }

/// The SexOS service that owns a device kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service { Kernel, Drive, Net, Bell, Usb, Display, Input, Nothing }

impl Service {
    pub fn name(self) -> &'static str {
        match self {
            Service::Kernel => "kernel",
            Service::Drive => "sexdrive",
            Service::Net => "sexnet",
            Service::Bell => "sexbell",
            Service::Usb => "sexusb",
            Service::Display => "sexdisplay",
            Service::Input => "sexinput",
            Service::Nothing => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// A SexOS service drives the device today.
    Driven,
    /// Usable only through what firmware set up (the boot framebuffer); no native driver.
    Firmware,
    /// Platform plumbing: bridges, host bridge, memory and system peripherals.
    Platform,
    /// A driver is in an open branch.
    InProgress,
    /// In the driver plan, not started.
    Planned,
    /// Not driven and not planned.
    Unsupported,
}

impl Support {
    pub fn name(self) -> &'static str {
        match self {
            Support::Driven => "driven",
            Support::Firmware => "firmware",
            Support::Platform => "platform",
            Support::InProgress => "in_progress",
            Support::Planned => "planned",
            Support::Unsupported => "none",
        }
    }
    /// Whether the device does its job on this build.
    pub fn works(self) -> bool { matches!(self, Support::Driven | Support::Firmware | Support::Platform) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    /// The model when the id is known, otherwise empty (use `vendor_name` + `class_name`).
    pub model: &'static str,
    pub service: Service,
    pub support: Support,
}

pub fn vendor_name(vendor: u16) -> &'static str {
    match vendor {
        0x8086 => "Intel",
        0x10de => "NVIDIA",
        0x1002 => "AMD/ATI",
        0x1022 => "AMD",
        0x10ec => "Realtek",
        0x144d => "Samsung",
        0x15b7 => "Western Digital",
        0x1c5c => "SK hynix",
        0x1344 => "Micron",
        0x1e0f => "Kioxia",
        0x1987 => "Phison",
        0x126f => "Silicon Motion",
        0x1cc1 => "ADATA",
        0x1e4b => "Maxio",
        0x1d79 => "Transcend",
        0x2646 => "Kingston",
        0x14e4 => "Broadcom",
        0x168c | 0x17cb => "Qualcomm",
        0x1969 => "Qualcomm Atheros (Killer)",
        0x14c3 => "MediaTek",
        0x1b21 => "ASMedia",
        0x1033 => "NEC/Renesas",
        0x1912 => "Renesas",
        0x1d6a => "Aquantia",
        0x1af4 => "virtio",
        0x1b36 => "QEMU",
        0x1234 => "QEMU/Bochs",
        0x15ad => "VMware",
        0x80ee => "VirtualBox",
        _ => "Unknown vendor",
    }
}

pub fn class_name(class: u8, subclass: u8, prog_if: u8) -> &'static str {
    match (class, subclass, prog_if) {
        (0x01, 0x01, _) => "IDE controller",
        (0x01, 0x04, _) => "RAID controller",
        (0x01, 0x06, 0x01) => "SATA controller (AHCI)",
        (0x01, 0x06, _) => "SATA controller",
        (0x01, 0x08, _) => "NVMe SSD",
        (0x01, _, _) => "storage controller",
        (0x02, 0x00, _) => "Ethernet controller",
        (0x02, 0x80, _) => "wireless network controller",
        (0x02, _, _) => "network controller",
        (0x03, 0x00, _) => "VGA display controller",
        (0x03, 0x02, _) => "3D controller",
        (0x03, _, _) => "display controller",
        (0x04, 0x03, _) => "HD Audio controller",
        (0x04, 0x01, _) => "audio device",
        (0x04, _, _) => "multimedia controller",
        (0x05, _, _) => "memory controller",
        (0x06, 0x00, _) => "host bridge",
        (0x06, 0x01, _) => "ISA/LPC bridge",
        (0x06, 0x04, _) => "PCI bridge",
        (0x06, _, _) => "bridge",
        (0x07, _, _) => "communication controller",
        (0x08, _, _) => "system peripheral",
        (0x09, _, _) => "input controller",
        (0x0c, 0x03, 0x30) => "USB 3 controller (xHCI)",
        (0x0c, 0x03, 0x40) => "USB4 host interface",
        (0x0c, 0x03, _) => "USB controller",
        (0x0c, 0x05, _) => "SMBus controller",
        (0x0c, 0x80, _) => "serial bus controller",
        (0x0c, _, _) => "serial bus controller",
        (0x0d, _, _) => "wireless controller",
        (0x10, _, _) => "encryption controller",
        (0x11, _, _) => "signal processing controller",
        (0x12, _, _) => "processing accelerator",
        _ => "device",
    }
}

const fn known(model: &'static str, service: Service, support: Support) -> Device { Device { model, service, support } }

/// Exact ids worth naming. Laptop parts of the bare-metal test machine (Alienware x17 R1:
/// Tiger Lake-H, HM570, RTX 30 laptop GPU, Killer AX1675 and E3100G) come first.
fn exact(vendor: u16, device: u16) -> Option<Device> {
    use Service::*;
    use Support::*;
    Some(match (vendor, device) {
        // Intel Tiger Lake-H platform (x17 R1, XPS 17 9710).
        (0x8086, 0x9a60) => known("UHD Graphics (Tiger Lake-H GT1)", Display, Firmware),
        (0x8086, 0x9a68) => known("UHD Graphics (Tiger Lake-H)", Display, Firmware),
        (0x8086, 0x9a49) => known("Iris Xe Graphics (Tiger Lake GT2)", Display, Firmware),
        (0x8086, 0x9a0b) => known("Volume Management Device (Tiger Lake, RST RAID)", Drive, Planned),
        (0x8086, 0x467f) => known("Volume Management Device (Alder Lake, RST RAID)", Drive, Planned),
        (0x8086, 0xa77f) => known("Volume Management Device (Raptor Lake, RST RAID)", Drive, Planned),
        (0x8086, 0x7d0b) => known("Volume Management Device (Meteor Lake, RST RAID)", Drive, Planned),
        (0x8086, 0xad0b) => known("Volume Management Device (Arrow Lake, RST RAID)", Drive, Planned),
        (0x8086, 0x43c8) => known("Tiger Lake-H HD Audio (cAVS)", Bell, Driven),
        (0x8086, 0xa0c8) => known("Tiger Lake-LP HD Audio (cAVS)", Bell, Driven),
        (0x8086, 0x43ed) => known("Tiger Lake-H USB 3.2 xHCI", Usb, Driven),
        (0x8086, 0xa0ed) => known("Tiger Lake-LP USB 3.2 xHCI", Usb, Driven),
        (0x8086, 0x9a13) => known("Tiger Lake Thunderbolt 4 USB xHCI", Usb, Driven),
        (0x8086, 0x9a1b) | (0x8086, 0x9a1d) => known("Tiger Lake Thunderbolt 4 NHI", Usb, Planned),
        (0x8086, 0x1137) => known("Maple Ridge Thunderbolt 4 NHI", Usb, Planned),
        // Intel Wi-Fi (iwlwifi family). Killer AX1675 is an AX210 or AX211 module.
        (0x8086, 0x2725) => known("Wi-Fi 6E AX210 / Killer AX1675x", Net, Planned),
        (0x8086, 0x2723) => known("Wi-Fi 6 AX200 / Killer AX1650x", Net, Planned),
        (0x8086, 0x43f0) => known("Wi-Fi 6 AX201 (Tiger Lake-H CNVi) / Killer AX1650i", Net, Planned),
        (0x8086, 0xa0f0) => known("Wi-Fi 6 AX201 (Tiger Lake CNVi)", Net, Planned),
        (0x8086, 0x51f0) | (0x8086, 0x51f1) => known("Wi-Fi 6E AX211 (Alder Lake CNVi) / Killer AX1675i", Net, Planned),
        (0x8086, 0x7af0) => known("Wi-Fi 6E AX211 (Alder Lake-S CNVi)", Net, Planned),
        (0x8086, 0x272b) => known("Wi-Fi 7 BE200", Net, Planned),
        // Intel Ethernet (e1000, e1000e/I219, igb, igc families are driven by sexnet).
        (0x8086, 0x100e) => known("82540EM Gigabit (e1000)", Net, Driven),
        (0x8086, 0x10d3) => known("82574L Gigabit (e1000e)", Net, Driven),
        (0x8086, 0x15f3) => known("I225-V 2.5GbE", Net, Driven),
        (0x8086, 0x125c) => known("I226-V 2.5GbE", Net, Driven),
        (0x8086, 0x1533) => known("I210 Gigabit", Net, Driven),
        (0x8086, 0x10c9) => known("82576 Gigabit (igb)", Net, Driven),
        // QEMU q35 platform.
        (0x8086, 0x29c0) => known("Q35 host bridge (QEMU)", Kernel, Platform),
        (0x8086, 0x2918) => known("ICH9 LPC bridge (QEMU)", Kernel, Platform),
        (0x8086, 0x2922) => known("ICH9 SATA AHCI (QEMU)", Drive, Driven),
        (0x8086, 0x2930) => known("ICH9 SMBus (QEMU)", Kernel, Platform),
        (0x8086, 0x293e) => known("ICH9 HD Audio (QEMU)", Bell, Driven),
        (0x1b36, 0x0010) => known("NVMe controller (QEMU)", Drive, Driven),
        (0x1b36, 0x000d) => known("xHCI controller (QEMU)", Usb, Driven),
        (0x1033, 0x0194) => known("uPD720200 xHCI (QEMU nec-usb-xhci)", Usb, Driven),
        (0x1234, 0x1111) => known("Standard VGA (QEMU)", Display, Firmware),
        // Realtek.
        (0x10ec, 0x8125) => known("RTL8125 2.5GbE", Net, Driven),
        (0x10ec, 0x3000) => known("Killer E3000/E3100G 2.5GbE (RTL8125 family)", Net, Driven),
        (0x10ec, 0x8168) => known("RTL8111/8168 Gigabit", Net, Driven),
        (0x10ec, 0x8139) => known("RTL8139 Fast Ethernet", Net, Unsupported),
        (0x10ec, 0x525a) => known("RTS525A PCIe card reader", Drive, Planned),
        (0x10ec, 0x522a) => known("RTS522A PCIe card reader", Drive, Planned),
        // NVIDIA Ampere laptop GPUs (the x17 R1 sold the 3060, 3070 and 3080).
        (0x10de, 0x249c) | (0x10de, 0x24dc) => known("GeForce RTX 3080 Laptop (GA104)", Display, Firmware),
        (0x10de, 0x249d) | (0x10de, 0x24dd) => known("GeForce RTX 3070 Laptop (GA104)", Display, Firmware),
        (0x10de, 0x2520) | (0x10de, 0x2560) => known("GeForce RTX 3060 Laptop (GA106)", Display, Firmware),
        (0x10de, 0x2420) | (0x10de, 0x2460) => known("GeForce RTX 3080 Ti Laptop (GA103)", Display, Firmware),
        (0x10de, 0x2204) => known("GeForce RTX 3090 (GA102)", Display, Firmware),
        (0x10de, 0x2206) => known("GeForce RTX 3080 (GA102)", Display, Firmware),
        (0x10de, 0x228b) => known("GA104 HD Audio (HDMI/DP)", Bell, Planned),
        (0x10de, 0x228e) => known("GA106 HD Audio (HDMI/DP)", Bell, Planned),
        (0x10de, 0x1aef) => known("GA102 HD Audio (HDMI/DP)", Bell, Planned),
        // NVMe SSDs commonly shipped in laptops; any NVMe class device is driven.
        (0x144d, 0xa808) => known("NVMe SSD (970 EVO/PM981)", Drive, Driven),
        (0x144d, 0xa809) => known("NVMe SSD (980)", Drive, Driven),
        (0x144d, 0xa80a) => known("NVMe SSD (980 PRO/PM9A1)", Drive, Driven),
        (0x15b7, 0x5006) => known("NVMe SSD (PC SN730/SN750)", Drive, Driven),
        (0x1c5c, 0x174a) => known("NVMe SSD (PC711/Gold P31)", Drive, Driven),
        // virtio.
        (0x1af4, 0x1059) => known("sound device", Bell, Driven),
        (0x1af4, 0x1041) | (0x1af4, 0x1000) => known("network device", Net, Planned),
        (0x1af4, 0x1042) | (0x1af4, 0x1001) => known("block device", Drive, Planned),
        (0x1af4, 0x1052) => known("input device", Input, Planned),
        (0x1af4, 0x1050) => known("GPU", Display, Planned),
        _ => return None,
    })
}

/// Which service owns a device kind on master, by class alone.
fn by_class(id: Id) -> (Service, Support) {
    use Service::*;
    use Support::*;
    match (id.class, id.subclass, id.prog_if) {
        (0x01, 0x08, _) => (Drive, Driven),
        (0x01, 0x06, 0x01) => (Drive, Driven),
        (0x01, 0x04, _) => (Drive, Unsupported),
        (0x01, _, _) => (Nothing, Unsupported),
        // Most Intel and Realtek Ethernet parts are driven; an unlisted one may not be.
        (0x02, 0x00, _) if matches!(id.vendor, 0x8086 | 0x10ec) => (Net, Planned),
        (0x02, 0x80, _) if id.vendor == 0x8086 => (Net, Planned),
        (0x02, _, _) => (Net, Unsupported),
        (0x03, _, _) => (Display, Firmware),
        (0x04, 0x03, _) if id.vendor != 0x10de && id.vendor != 0x1002 => (Bell, Driven),
        (0x04, 0x03, _) => (Bell, Planned),
        (0x04, _, _) => (Nothing, Unsupported),
        (0x05, _, _) | (0x06, _, _) | (0x08, _, _) | (0x11, _, _) => (Kernel, Platform),
        (0x0c, 0x03, 0x30) => (Usb, Driven),
        (0x0c, 0x03, _) => (Usb, Unsupported),
        (0x0c, 0x05, _) => (Kernel, Platform),
        // Intel Serial IO (I2C/SPI/UART): the bus under laptop touchpads (I2C HID).
        (0x0c, 0x80, _) if id.vendor == 0x8086 => (Input, Planned),
        _ => (Nothing, Unsupported),
    }
}

pub fn describe(id: Id) -> Device {
    exact(id.vendor, id.device).unwrap_or_else(|| {
        let (service, support) = by_class(id);
        Device { model: "", service, support }
    })
}

/// True for Intel Volume Management Device controllers (RST "RAID On").
pub fn is_vmd(id: Id) -> bool {
    id.vendor == 0x8086 && matches!(id.device, 0x9a0b | 0x467f | 0x4c3d | 0xa77f | 0x7d0b | 0xad0b | 0xb60b | 0x201d | 0x28c0)
}

/// What the person at the machine needs to know about this device, if anything.
pub fn advice(id: Id) -> Option<&'static str> {
    if is_vmd(id) {
        return Some("Intel VMD/RST RAID is on: NVMe drives behind it are hidden. In the BIOS set Storage to AHCI/NVMe (not RAID On), or wait for the VMD driver");
    }
    match (id.class, id.subclass) {
        (0x01, 0x04) => Some("RAID controller without a driver: disks behind it are hidden"),
        (0x03, _) if id.vendor == 0x10de => Some("NVIDIA GPU has no native driver yet: the picture comes from the firmware framebuffer. On a laptop, Hybrid mode lets the Intel GPU drive the panel"),
        (0x02, 0x80) => Some("no Wi-Fi driver yet: use Ethernet"),
        _ => None,
    }
}

/// Running totals for the census summary line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census { pub devices: u32, pub working: u32, pub in_progress: u32, pub planned: u32, pub unsupported: u32, pub warnings: u32 }

impl Census {
    pub fn add(&mut self, id: Id) -> Device {
        let device = describe(id);
        self.devices += 1;
        match device.support {
            s if s.works() => self.working += 1,
            Support::InProgress => self.in_progress += 1,
            Support::Planned => self.planned += 1,
            _ => self.unsupported += 1,
        }
        if advice(id).is_some() { self.warnings += 1; }
        device
    }
}
