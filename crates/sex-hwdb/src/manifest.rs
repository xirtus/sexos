// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Driver manifests: which devices each driver service asks BOOT to lease to it.
//!
//! This is the lease authority's input, kept beside the census table so that adding
//! support for a new controller is a data change here, not a kernel edit. The kernel's
//! `devmgr` holds only the mechanisms: how a lease of each `Lease` kind is built (which
//! BAR, how long the register view, which command bits, which DMA pool). It walks PCI
//! once, finds the first manifest entry that matches each device, and leases it to that
//! service, many devices per service.
//!
//! A manifest names, per kind of device, the slot the driver reads its first one from;
//! how many devices the service may hold; and how many interrupt vectors it wants each.

use crate::{Id, Service};

/// How BOOT builds the lease. Each kind is one mechanism in the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lease {
    /// NVMe: BAR0 view (64 KiB prefix), memory + bus master turned on.
    Nvme,
    /// xHCI: BAR0 view, plus its legacy interrupt line.
    Xhci,
    /// Wired Ethernet whose register layout `sex_pdx::pci_command::Model` knows;
    /// the register BAR the model names, command grant, network DMA pool.
    Nic,
    /// Intel High Definition Audio: BAR0, command grant, sound DMA pool.
    Hda,
    /// AHCI SATA: BAR5 (ABAR), command grant; the drive pool is already the drive's.
    Ahci,
    /// Modern virtio PCI: the one BAR holding all four register blocks, command grant,
    /// virtio DMA pool.
    Virtio,
    /// Display controller: identity only, no BAR view yet (firmware framebuffer).
    Display,
    /// Intel display engine (Gen12 Tiger Lake): a view of BAR0's display registers
    /// (`sex_pdx::display_frames::INTEL_REGISTERS` bytes, not the GTT half), no command
    /// grant and no DMA pool. Display reads what firmware lit (workstream L, slice L3).
    IntelDisplay,
    /// Intel Wi-Fi: BAR0, command grant, its own 4 MiB DMA pool and the firmware pack.
    Wifi,
    /// Intel Serial IO I2C (DesignWare block plus Intel's private page): BAR0, memory
    /// command bit, the function woken to D0, no DMA. Only product ids in
    /// [`SERIAL_IO_I2C`] match, so Serial IO SPI and UART functions stay unleased.
    SerialIo,
}

/// Intel Serial IO I2C functions, Skylake to Meteor Lake PCHs.
pub const SERIAL_IO_I2C: &[u16] = &[
    0x9d60, 0x9d61, 0x9d62, 0x9d63, 0x9d64, 0x9d65, 0xa160, 0xa161, 0xa162, 0xa163, 0xa2e0, 0xa2e1, 0xa2e2, 0xa2e3,
    0x9de8, 0x9de9, 0x9dea, 0x9deb, 0x9dc5, 0x9dc6, 0xa368, 0xa369, 0xa36a, 0xa36b,
    0x02e8, 0x02e9, 0x02ea, 0x02eb, 0x02c5, 0x02c6, 0x06e8, 0x06e9, 0x06ea, 0x06eb,
    0x34e8, 0x34e9, 0x34ea, 0x34eb, 0x34c5, 0x34c6,
    0xa0e8, 0xa0e9, 0xa0ea, 0xa0eb, 0xa0c5, 0xa0c6, 0xa0d8, 0xa0d9,
    0x43e8, 0x43e9, 0x43ea, 0x43eb, 0x43ad, 0x43ae, 0x43d8,
    0x51e8, 0x51e9, 0x51ea, 0x51eb, 0x51c5, 0x51c6, 0x51d8, 0x51d9,
    0x7acc, 0x7acd, 0x7ace, 0x7acf, 0x7afc, 0x7afd, 0x7a4c, 0x7a4d, 0x7a4e, 0x7a4f, 0x7a7c, 0x7a7d,
    0x7e50, 0x7e51, 0x7e78, 0x7e79, 0x7e7a, 0x7e7b,
];
pub fn serial_io_i2c(vendor: u16, device: u16) -> bool { vendor == 0x8086 && SERIAL_IO_I2C.contains(&device) }

/// One kind of device a service drives. `class` is `class << 16 | subclass << 8 | prog_if`
/// compared under `class_mask`; `vendor`/`device` of 0 match any. `slot` is where the
/// driver finds its first device of this kind (0 = no slot: identity only); further
/// devices get the next free slot from `sex_pdx::device::EXTRA_SLOT_BASE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub class: u32,
    pub class_mask: u32,
    pub vendor: u16,
    pub device: u16,
    pub lease: Lease,
    pub slot: u64,
}

impl Match {
    pub const fn class(class: u32, class_mask: u32, lease: Lease, slot: u64) -> Match {
        Match { class, class_mask, vendor: 0, device: 0, lease, slot }
    }
    pub const fn vendor_class(vendor: u16, class: u32, class_mask: u32, lease: Lease, slot: u64) -> Match {
        Match { class, class_mask, vendor, device: 0, lease, slot }
    }
    pub const fn id(vendor: u16, device: u16, lease: Lease, slot: u64) -> Match {
        Match { class: 0, class_mask: 0, vendor, device, lease, slot }
    }
    pub fn matches(&self, id: Id) -> bool {
        let class = (id.class as u32) << 16 | (id.subclass as u32) << 8 | id.prog_if as u32;
        class & self.class_mask == self.class & self.class_mask
            && (self.vendor == 0 || self.vendor == id.vendor)
            && (self.device == 0 || self.device == id.device)
            && (self.lease != Lease::SerialIo || serial_io_i2c(id.vendor, id.device))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub service: Service,
    /// At most this many devices of this service (0 = no limit beyond the extra slots).
    pub max_devices: u8,
    /// MSI/MSI-X vectors BOOT programs per device (0 = none; the driver polls).
    pub vectors: u8,
    /// Ordered: the first matching entry decides the lease kind.
    pub matches: &'static [Match],
}

const CLASS: u32 = 0xff_ff00;
const EXACT: u32 = 0xff_ffff;
const BASE: u32 = 0xff_0000;

/// sexdrive: every NVMe controller, every AHCI SATA controller.
pub const DRIVE: Manifest = Manifest {
    service: Service::Drive, max_devices: 0, vectors: 1,
    matches: &[
        Match::class(0x01_08_00, CLASS, Lease::Nvme, 16),
        Match::class(0x01_06_01, EXACT, Lease::Ahci, 21),
    ],
};

/// sexusb: xHCI host controllers (USB 3; USB4 hosts expose xHCI too).
pub const USB: Manifest = Manifest {
    service: Service::Usb, max_devices: 0, vectors: 0,
    matches: &[Match::class(0x0c_03_30, EXACT, Lease::Xhci, 8)],
};

/// sexnet: Intel and Realtek wired Ethernet (the model table in `sex_pdx::pci_command`
/// decides which parts have a known register layout), and Intel Wi-Fi (the chip table
/// in `sex_wifi::devices` decides which ones it can start).
pub const NET: Manifest = Manifest {
    service: Service::Net, max_devices: 0, vectors: 0,
    matches: &[
        Match::vendor_class(0x8086, 0x02_00_00, CLASS, Lease::Nic, 19),
        Match::vendor_class(0x10ec, 0x02_00_00, CLASS, Lease::Nic, 19),
        Match::vendor_class(0x1186, 0x02_00_00, CLASS, Lease::Nic, 19),
        Match::vendor_class(0x8086, 0x02_80_00, CLASS, Lease::Wifi, 22),
    ],
};

/// sexbell: virtio sound, Intel HD Audio controllers.
pub const BELL: Manifest = Manifest {
    service: Service::Bell, max_devices: 0, vectors: 0,
    matches: &[
        // One sound card is driven: whichever comes first holds slot 20.
        Match::id(0x1af4, 0x1059, Lease::Virtio, 20),
        Match::class(0x04_03_00, CLASS, Lease::Hda, 20),
    ],
};

/// sexdisplay: virtio-gpu as a driven device (slice L2), Tiger Lake display
/// registers (slice L3), every other display controller as identity only.
pub const DISPLAY: Manifest = Manifest {
    service: Service::Display, max_devices: 0, vectors: 0,
    matches: &[
        Match::id(0x1af4, 0x1050, Lease::Virtio, 25),
        // Tiger Lake UHD/Iris Xe (sex-gpu-intel's TIGER_LAKE list).
        Match::id(0x8086, 0x9a40, Lease::IntelDisplay, 27),
        Match::id(0x8086, 0x9a49, Lease::IntelDisplay, 27),
        Match::id(0x8086, 0x9a60, Lease::IntelDisplay, 27),
        Match::id(0x8086, 0x9a68, Lease::IntelDisplay, 27),
        Match::id(0x8086, 0x9a70, Lease::IntelDisplay, 27),
        Match::id(0x8086, 0x9a78, Lease::IntelDisplay, 27),
        Match::class(0x03_00_00, BASE, Lease::Display, 0),
    ],
};

/// sexinput: the Serial IO I2C buses laptop touchpads sit on. Which one holds the
/// touchpad only firmware knows; the ACPI service tells Input (`sex_pdx::touchpad`).
pub const INPUT: Manifest = Manifest {
    service: Service::Input, max_devices: 8, vectors: 0,
    matches: &[Match::vendor_class(0x8086, 0x0c_80_00, CLASS, Lease::SerialIo, 28)],
};

/// Every manifest, in the order BOOT consults them.
pub const ALL: &[Manifest] = &[DRIVE, USB, NET, BELL, DISPLAY, INPUT];

/// The manifest and entry that claim this device, if any.
pub fn claim(id: Id) -> Option<(&'static Manifest, Match)> {
    ALL.iter().find_map(|m| m.matches.iter().find(|e| e.matches(id)).map(|e| (m, *e)))
}

/// The manifest of one service.
pub fn of(service: Service) -> Option<&'static Manifest> {
    ALL.iter().find(|m| m.service == service)
}
