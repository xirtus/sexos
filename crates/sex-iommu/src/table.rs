// SPDX-License-Identifier: GPL-3.0-only
//! I/O page tables: 512 entries of 8 bytes per 4 KiB table, 3 or 4 levels,
//! 4 KiB leaves only. VT-d second-level and AMD-Vi host tables share this
//! shape and differ only in entry bits, which `Format` supplies.
use crate::PAGE;

/// Where tables live. The kernel backs this with frames it reserved at BOOT
/// and reads them through its direct map; host tests back it with a Vec.
pub trait Frames {
    /// A zeroed 4 KiB frame, or None when the table budget is spent.
    fn alloc(&mut self) -> Option<u64>;
    fn read(&self, table: u64, index: usize) -> u64;
    /// Must make the write visible to the IOMMU's page walker (clflush when
    /// the unit does not snoop CPU caches).
    fn write(&mut self, table: u64, index: usize, value: u64);
}

pub const READ: u8 = 1;
pub const WRITE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format { Vtd, AmdVi }

impl Format {
    /// An entry pointing at a table of `level` (1 = leaf table).
    fn table(self, frame: u64, level: u8) -> u64 {
        match self {
            // Non-leaf rights are ANDed with the leaf's, so grant both here.
            Format::Vtd => frame | 3,
            // PR, the next table's level in bits 11:9, IR and IW.
            Format::AmdVi => frame | 1 | ((level as u64) << 9) | (3 << 61),
        }
    }
    fn leaf(self, frame: u64, rights: u8) -> u64 {
        let (r, w) = (rights & READ != 0, rights & WRITE != 0);
        match self {
            Format::Vtd => frame | r as u64 | (w as u64) << 1,
            Format::AmdVi => frame | 1 | (r as u64) << 61 | (w as u64) << 62,
        }
    }
    fn present(self, entry: u64) -> bool {
        match self { Format::Vtd => entry & 3 != 0, Format::AmdVi => entry & 1 != 0 }
    }
    fn rights(self, entry: u64) -> u8 {
        match self {
            Format::Vtd => (entry & 3) as u8,
            Format::AmdVi => ((entry >> 61) & 1) as u8 | (((entry >> 62) & 1) as u8) << 1,
        }
    }
}

const ADDRESS: u64 = 0x000f_ffff_ffff_f000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError { Range, Unaligned, Present, NoTable, NoFrames, Rights }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Table { pub root: u64, pub levels: u8, pub format: Format }

impl Table {
    pub fn new(frames: &mut impl Frames, levels: u8, format: Format) -> Option<Self> {
        if !(3..=4).contains(&levels) { return None; }
        Some(Self { root: frames.alloc()?, levels, format })
    }

    /// Device addresses this table can name: [0, 2^(12 + 9 * levels)).
    pub fn limit(&self) -> u64 { 1u64 << (12 + 9 * self.levels as u32) }

    fn index(iova: u64, level: u8) -> usize { ((iova >> (12 + 9 * (level as u32 - 1))) & 511) as usize }

    /// The leaf table holding `iova`, creating missing tables when `grow`.
    fn leaf_table(&self, frames: &mut impl Frames, iova: u64, grow: bool) -> Result<u64, MapError> {
        let mut table = self.root;
        for level in (2..=self.levels).rev() {
            let i = Self::index(iova, level);
            let entry = frames.read(table, i);
            table = if self.format.present(entry) { entry & ADDRESS } else {
                if !grow { return Err(MapError::NoTable); }
                let next = frames.alloc().ok_or(MapError::NoFrames)?;
                frames.write(table, i, self.format.table(next, level - 1));
                next
            };
        }
        Ok(table)
    }

    fn check(&self, iova: u64, length: u64) -> Result<(), MapError> {
        if iova % PAGE != 0 || length % PAGE != 0 || length == 0 { return Err(MapError::Unaligned); }
        match iova.checked_add(length) { Some(end) if end <= self.limit() => Ok(()), _ => Err(MapError::Range) }
    }

    /// Build every table a later `map` of [iova, iova+length) needs, so that
    /// runtime mapping never allocates.
    pub fn prepare(&self, frames: &mut impl Frames, iova: u64, length: u64) -> Result<(), MapError> {
        self.check(iova, length)?;
        let mut at = iova;
        while at < iova + length {
            self.leaf_table(frames, at, true)?;
            at = (at | (PAGE * 512 - 1)) + 1;
        }
        Ok(())
    }

    /// Map one page. Never replaces a present entry: a device address names
    /// one frame until it is unmapped and the IOTLB flushed.
    pub fn map(&self, frames: &mut impl Frames, iova: u64, frame: u64, rights: u8, grow: bool) -> Result<(), MapError> {
        self.check(iova, PAGE)?;
        if frame % PAGE != 0 || frame & !ADDRESS != 0 { return Err(MapError::Unaligned); }
        if rights == 0 || rights & !(READ | WRITE) != 0 { return Err(MapError::Rights); }
        let table = self.leaf_table(frames, iova, grow)?;
        let i = Self::index(iova, 1);
        if self.format.present(frames.read(table, i)) { return Err(MapError::Present); }
        frames.write(table, i, self.format.leaf(frame, rights));
        Ok(())
    }

    /// Map a run of pages at consecutive device addresses (identity or not).
    pub fn map_range(&self, frames: &mut impl Frames, iova: u64, frame: u64, length: u64, rights: u8) -> Result<(), MapError> {
        self.check(iova, length)?;
        let mut done = 0;
        while done < length {
            if let Err(e) = self.map(frames, iova + done, frame + done, rights, true) {
                let mut undo = 0;
                while undo < done { self.unmap(frames, iova + undo); undo += PAGE; }
                return Err(e);
            }
            done += PAGE;
        }
        Ok(())
    }

    /// Clear one page; returns the frame it held. Caller flushes the IOTLB.
    pub fn unmap(&self, frames: &mut impl Frames, iova: u64) -> Option<u64> {
        self.check(iova, PAGE).ok()?;
        let table = self.leaf_table(frames, iova, false).ok()?;
        let i = Self::index(iova, 1);
        let entry = frames.read(table, i);
        if !self.format.present(entry) { return None; }
        frames.write(table, i, 0);
        Some(entry & ADDRESS)
    }

    /// What the device reaches at `iova`: frame and rights.
    pub fn translate(&self, frames: &mut impl Frames, iova: u64) -> Option<(u64, u8)> {
        if iova >= self.limit() { return None; }
        let table = self.leaf_table(frames, iova & !(PAGE - 1), false).ok()?;
        let entry = frames.read(table, Self::index(iova, 1));
        self.format.present(entry).then(|| ((entry & ADDRESS) | (iova & (PAGE - 1)), self.format.rights(entry)))
    }
}
