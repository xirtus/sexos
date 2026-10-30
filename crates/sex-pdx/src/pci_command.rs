// SPDX-License-Identifier: GPL-3.0-only
//! Immutable BOOT device-command authority. Request integers select bits, never
//! a BDF, register offset, mapping, DMA address or capability to create.
pub const SYS_COMMAND: u64 = 68;
pub const SYS_IDENTITY: u64 = 69;
pub const MEMORY: u16 = 1 << 1;
pub const BUS_MASTER: u16 = 1 << 2;
pub const NIC_BITS: u16 = MEMORY | BUS_MASTER;
pub const DMA_REGION_ID: u32 = 2;
pub const DMA_BYTES: u64 = 256 * 1024;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model { I82540, I82574 }
impl Identity {
    pub fn model(self) -> Option<Model> {
        match (self.vendor,self.product) { (0x8086,0x100e)=>Some(Model::I82540),
            (0x8086,0x10d3)=>Some(Model::I82574),_=>None }
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
