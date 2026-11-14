// SPDX-License-Identifier: GPL-3.0-only
//! Physical intervals held by boot data owners, excluded from page-table frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range { pub start: u64, pub end: u64 }
impl Range {
    pub fn overlaps(self, other: Self) -> bool { self.start < other.end && other.start < self.end }
}

/// Pure bootstrap placement: skip the consumed usable-page prefix, carve metadata
/// in the first region that actually fits, and return remaining free intervals.
pub struct BootstrapPlanner {
    remaining_consumed: u64,
    metadata_bytes: u64,
    pub metadata: Option<Range>,
}
impl BootstrapPlanner {
    pub fn new(consumed_pages: u64, metadata_bytes: u64) -> Self {
        Self { remaining_consumed: consumed_pages, metadata_bytes, metadata: None }
    }
    /// Pages the boot allocator walked past but that nobody else may have.
    pub fn skip(&mut self, length: u64) {
        self.remaining_consumed -= self.remaining_consumed.min(length / 4096);
    }
    pub fn region(&mut self, base: u64, length: u64) -> Result<Option<Range>, &'static str> {
        if base % 4096 != 0 || length % 4096 != 0 || self.metadata_bytes == 0 || self.metadata_bytes % 4096 != 0 {
            return Err("BOOT_MEMORY_REGION_ALIGNMENT");
        }
        let end = base.checked_add(length).ok_or("BOOT_MEMORY_REGION_OVERFLOW")?;
        let skip = self.remaining_consumed.min(length / 4096);
        self.remaining_consumed -= skip;
        let mut start = base + skip * 4096;
        if self.metadata.is_none() && end - start >= self.metadata_bytes {
            self.metadata = Some(Range { start, end: start + self.metadata_bytes });
            start += self.metadata_bytes;
        }
        Ok(if start < end { Some(Range {start,end}) } else { None })
    }
}
/// Boot regions BOOT may reserve. 128 was full on a QEMU desktop once Display also
/// got a virtio-gpu scanout buffer.
pub const CAPACITY: usize = 192;
pub struct BootFrameReservations { ranges: [Range; CAPACITY], count: usize }
impl BootFrameReservations {
    pub const fn new() -> Self { Self { ranges: [Range {start:0,end:0};CAPACITY], count:0 } }
    pub fn reserve(&mut self, start: u64, length: u64, mut already_consumed: impl FnMut(Range)->bool) -> Result<(), &'static str> {
        let end = start.checked_add(length).ok_or("BOOT_FRAME_RANGE_OVERFLOW")?;
        if start == 0 || length == 0 || start % 4096 != 0 || length % 4096 != 0 { return Err("BOOT_FRAME_RANGE_ALIGNMENT"); }
        let range = Range {start,end};
        if already_consumed(range) || self.ranges[..self.count].iter().any(|r| r.overlaps(range)) {
            return Err("BOOT_FRAME_OWNERSHIP_OVERLAP");
        }
        // A range touching one already held joins it, so neighbours take one entry.
        if let Some(r) = self.ranges[..self.count].iter_mut().find(|r| r.end == start || r.start == end) {
            (r.start, r.end) = (r.start.min(start), r.end.max(end));
            return Ok(());
        }
        if self.count == self.ranges.len() { return Err("BOOT_FRAME_RESERVATIONS_FULL"); }
        self.ranges[self.count] = range;
        self.count += 1;
        Ok(())
    }
    pub fn contains(&self, physical: u64) -> bool {
        self.ranges[..self.count].iter().any(|r| physical >= r.start && physical < r.end)
    }
}
