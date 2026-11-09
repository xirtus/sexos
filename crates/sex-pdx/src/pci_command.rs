// SPDX-License-Identifier: GPL-3.0-only
//! Immutable BOOT device-command authority. Request integers select bits, never
//! a BDF, register offset, mapping, DMA address or capability to create.
pub const SYS_COMMAND: u64 = 68;
pub const SYS_IDENTITY: u64 = 69;
/// SYS_IDENTITY with this first argument asks where the registers sit in the view.
pub const QUERY_WINDOW: u64 = 1;
pub const MEMORY: u16 = 1 << 1;
pub const BUS_MASTER: u16 = 1 << 2;
pub const NIC_BITS: u16 = MEMORY | BUS_MASTER;
pub const DMA_REGION_ID: u32 = 2;
pub const DMA_BYTES: u64 = 256 * 1024;
/// Bell's pool when BOOT leased it an Intel HD Audio controller.
pub const HDA_DMA_REGION_ID: u32 = 0x4300;
pub const HDA_DMA_BYTES: u64 = 256 * 1024;
/// The drive service's pool (the same one its NVMe path uses).
pub const AHCI_DMA_REGION_ID: u32 = 3;
pub const AHCI_DMA_BYTES: u64 = 512 * 1024;
/// Where an AHCI controller's lease sits in the drive service (NVMe is slot 16).
pub const SLOT_AHCI_HOST: u64 = 21;
pub const DENIED: u64 = u64::MAX;
/// A hardware write/readback failure is uncertain; do not retry a mutation.
pub const IO: u64 = u64::MAX - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant {
    pub subject: u32,
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub allowed: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Update { grant: Grant, set: u16, clear: u16 }
impl Update {
    pub fn grant(&self) -> Grant { self.grant }
    pub fn needs_dma(&self) -> bool { self.set & BUS_MASTER != 0 }
    pub fn next(&self, previous: u16) -> u16 { (previous | self.set) & !self.clear }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply { pub before: u16, pub after: u16, pub changed: bool }

/// Immutable BOOT metadata about the held device; never a selector or grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity { pub vendor: u16, pub product: u16 }
/// Wired Ethernet controllers SexNet drives. Families share a register map:
/// e1000 (82540), e1000e (82574, I217/I218/I219 on Intel desktop and laptop
/// chipsets), igb (82576, I210/I211, I350), igc (I225/I226 2.5 GbE), and Realtek
/// r8169 (RTL8111/8168/8101 gigabit) and r8125 (RTL8125/8126 2.5/5 GbE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model { I82540, I82574, I219, I82576, I210, I350, I225, Rtl8168, Rtl8125 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family { E1000, E1000e, Igb, Igc, Realtek }
impl Model {
    pub fn family(self) -> Family {
        match self {
            Model::I82540 => Family::E1000,
            Model::I82574 | Model::I219 => Family::E1000e,
            Model::I82576 | Model::I210 | Model::I350 => Family::Igb,
            Model::I225 => Family::Igc,
            Model::Rtl8168 | Model::Rtl8125 => Family::Realtek,
        }
    }
    /// Advanced (igb/igc) descriptors and queue registers at 0xC000/0xE000.
    pub fn advanced(self) -> bool { matches!(self.family(), Family::Igb | Family::Igc) }
    /// The registers a driver touches end before this offset in the leased BAR.
    pub fn registers_end(self) -> u64 {
        match self.family() {
            Family::Realtek => 0x100,
            Family::Igb | Family::Igc => 0xE030,
            _ => 0x5408,
        }
    }
}
impl Identity {
    pub fn model(self) -> Option<Model> {
        let p = self.product;
        match self.vendor {
            0x8086 => Some(match p {
                0x100e => Model::I82540,
                0x10d3 => Model::I82574,
                // I217/I218/I219 (LM and V) on 8th-series to Arrow Lake chipsets.
                0x153a | 0x153b | 0x155a | 0x1559 | 0x15a0..=0x15a3 | 0x156f | 0x1570
                | 0x15b7..=0x15b9 | 0x15bb..=0x15be | 0x15d6..=0x15d8 | 0x15df..=0x15e3
                | 0x15f4 | 0x15f5 | 0x15f9..=0x15fc | 0x0d4c..=0x0d4f | 0x0d53 | 0x0d55
                | 0x1a1c..=0x1a1f | 0x0dc5..=0x0dc8 | 0x550a..=0x5511 | 0x57a0 | 0x57a1
                | 0x57b3..=0x57ba => Model::I219,
                0x10c9 | 0x10e6 | 0x10e7 | 0x10e8 | 0x1526 | 0x150a | 0x1518 | 0x150d => Model::I82576,
                0x1533 | 0x1536..=0x1539 | 0x157b | 0x157c => Model::I210,
                0x1521..=0x1524 | 0x1516 => Model::I350,
                0x15f2 | 0x15f3 | 0x15f7 | 0x15f8 | 0x15fd | 0x0d9f | 0x3100..=0x3102
                | 0x125b..=0x125f | 0x5502 | 0x5503 => Model::I225,
                _ => return None,
            }),
            0x10ec => Some(match p {
                0x8161 | 0x8167..=0x8169 | 0x8136 | 0x2502 | 0x2600 => Model::Rtl8168,
                0x8125 | 0x8126 | 0x3000 | 0x5000 => Model::Rtl8125,
                _ => return None,
            }),
            // D-Link and other boards built on the RTL8168/8169.
            0x1186 if matches!(p, 0x4300 | 0x4302) => Some(Model::Rtl8168),
            _ => None,
        }
    }
}

/// The grant must be obtained from the current domain's typed capability table.
pub fn authorize(grant: Option<&Grant>, caller: u32, set: u64, clear: u64) -> Result<Update,u64> {
    let g = grant.ok_or(DENIED)?;
    if caller == 0 || g.subject != caller || g.device > 31 || g.function > 7
        || matches!(g.vendor_id, 0 | 0xffff) || g.allowed == 0 || g.allowed & !NIC_BITS != 0
        || set > u16::MAX as u64 || clear > u16::MAX as u64
    { return Err(DENIED); }
    let (set,clear) = (set as u16, clear as u16);
    if (set | clear) & !g.allowed != 0 || set & clear != 0 { return Err(DENIED); }
    Ok(Update { grant: *g, set, clear })
}
