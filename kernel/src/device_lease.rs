// SPDX-License-Identifier: GPL-3.0-only
//! BOOT policy for controllers leased by PCI class rather than by product id:
//! Intel High Definition Audio (to the Bell service), AHCI SATA (to the drive
//! service) and Intel Wi-Fi (to the network service). Like the NIC and virtio leases, a driver sees only one BAR view, the
//! command bits BOOT granted, and its own DMA pool; it never names an address.
use crate::hal::pci::PciDevice;

/// Intel HD Audio controller: multimedia, HD audio, any interface.
pub const CLASS_HDA: u32 = 0x04_03_00;
/// AHCI 1.0 SATA controller.
pub const CLASS_AHCI: u32 = 0x01_06_01;
/// "Other" network controller: Wi-Fi. The manifest only claims Intel's.
pub const CLASS_WIFI: u32 = 0x02_80_00;
/// "Other" serial bus controller: Intel Serial IO. The manifest only claims its I2C functions.
pub const CLASS_SERIAL_IO: u32 = 0x0c_80_00;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Hda, Ahci, Wifi, SerialIo }
impl Kind {
    pub fn of(dev: &PciDevice) -> Option<Kind> {
        Self::of_class((dev.class_id as u32) << 16 | (dev.subclass_id as u32) << 8 | dev.prog_if as u32)
    }
    /// HD Audio ignores the programming interface; AHCI must say AHCI, not IDE or RAID.
    pub fn of_class(class: u32) -> Option<Kind> {
        match class {
            c if c & 0xffff00 == CLASS_HDA => Some(Kind::Hda),
            CLASS_AHCI => Some(Kind::Ahci),
            c if c & 0xffff00 == CLASS_WIFI => Some(Kind::Wifi),
            c if c & 0xffff00 == CLASS_SERIAL_IO => Some(Kind::SerialIo),
            _ => None,
        }
    }
    pub fn class(self, dev: &PciDevice) -> u32 {
        match self { Kind::Hda => CLASS_HDA | dev.prog_if as u32, Kind::Ahci => CLASS_AHCI, Kind::Wifi => CLASS_WIFI | dev.prog_if as u32, Kind::SerialIo => CLASS_SERIAL_IO | dev.prog_if as u32 }
    }
    /// HDA and Intel Wi-Fi keep their registers in BAR0; AHCI's ABAR is BAR5.
    pub fn bar(self) -> u8 { match self { Kind::Hda | Kind::Wifi | Kind::SerialIo => 0, Kind::Ahci => 5 } }
    /// Global registers plus every stream (HDA: up to 30 descriptors) or port
    /// (AHCI: 32 ports at 0x100 + n*0x80) a driver may touch.
    /// Intel Wi-Fi: the CSRs up to the RX free-ring index at 0x1c80 (8 KiB BAR minimum).
    /// Serial IO: the DesignWare block and Intel's private page (resets) at 0x200.
    pub fn registers_end(self) -> u64 { match self { Kind::Hda => 0x180, Kind::Ahci => 0x180, Kind::Wifi => 0x2000, Kind::SerialIo => 0x300 } }
    pub fn view_length(self, actual: u64) -> Option<u64> {
        if actual < self.registers_end() { None } else { Some(actual.min(64 * 1024)) }
    }
    /// The DMA pool region a driver of this kind holds at SLOT_DMA_REGION.
    pub fn dma(self) -> (u32, u64) {
        match self {
            Kind::Hda => (sex_pdx::pci_command::HDA_DMA_REGION_ID, sex_pdx::pci_command::HDA_DMA_BYTES),
            Kind::Ahci => (sex_pdx::pci_command::AHCI_DMA_REGION_ID, sex_pdx::pci_command::AHCI_DMA_BYTES),
            Kind::Wifi => (sex_pdx::pci_command::WIFI_DMA_REGION_ID, sex_pdx::pci_command::WIFI_DMA_BYTES),
            Kind::SerialIo => (0, 0),
        }
    }
    /// Where that pool sits. Wi-Fi shares the network service with the wired NIC,
    /// whose pool already holds SLOT_DMA_REGION.
    pub fn dma_slot(self) -> u64 {
        match self { Kind::Wifi => sex_pdx::pci_command::SLOT_WIFI_DMA, _ => sex_pdx::SLOT_DMA_REGION }
    }
}

/// PCI power management: firmware may leave a Serial IO function in D3hot, where its
/// registers read all ones. Puts it in D0 (the driver waits out the 10 ms recovery).
pub fn wake_d0(dev: &PciDevice) -> bool {
    if dev.read_u32(0x04) >> 16 & 0x10 == 0 { return false; }
    let mut at = (dev.read_u32(0x34) & 0xfc) as u8;
    for _ in 0..48 {
        if at < 0x40 { return false; }
        let head = dev.read_u32(at);
        if head & 0xff == 0x01 {
            let pmcsr = dev.read_u32(at + 4);
            if pmcsr & 3 != 0 { dev.write_u32(at + 4, pmcsr & !3); return true; }
            return false;
        }
        at = (head >> 8 & 0xfc) as u8;
    }
    false
}

/// HD Audio that only feeds a monitor: function 1+ of a graphics card (AMD and
/// NVIDIA put it beside the GPU) or Intel's Haswell/Broadwell HDMI controller.
/// Analog sound lives on the chipset's own controller.
pub fn display_audio(dev: &PciDevice) -> bool {
    if dev.vendor_id == 0x8086 && matches!(dev.device_id, 0x0a0c | 0x0c0c | 0x0d0c | 0x160c) { return true; }
    if dev.func == 0 { return false; }
    // Function 0 of the same device: through its ECAM page behind VMD, else the ports.
    let function0 = PciDevice { func: 0, ecam: if dev.ecam != 0 { dev.ecam - ((dev.func as u64) << 12) } else { 0 }, ..*dev };
    let class = function0.read_u32(0x08) >> 24;
    class == 0x03
}
