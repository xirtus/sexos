// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! DMA memory and rings. The service hands a driver its DMA pool (memory the
//! device may reach, behind the IOMMU); the driver carves buffers from it with
//! [`DmaPool`] and checks every index and length the device writes back.

use crate::error::Error;

/// One buffer the device can reach: the CPU's view and the bus address the
/// device uses for the same bytes. Every accessor is bounds checked.
pub struct DmaBuf<'a> {
    pub cpu: &'a mut [u8],
    pub bus: u64,
}

impl<'a> DmaBuf<'a> {
    pub fn new(cpu: &'a mut [u8], bus: u64) -> Self { DmaBuf { cpu, bus } }
    pub fn len(&self) -> usize { self.cpu.len() }
    pub fn is_empty(&self) -> bool { self.cpu.is_empty() }
    /// The bus address of byte `offset`, if it is inside the buffer.
    pub fn bus_at(&self, offset: usize) -> Option<u64> {
        (offset < self.cpu.len()).then(|| self.bus.wrapping_add(offset as u64))
    }
    /// Whether a device-supplied bus range lies wholly inside this buffer.
    pub fn contains_bus(&self, bus: u64, len: u64) -> bool {
        bus >= self.bus && bus.checked_add(len).is_some_and(|e| e <= self.bus.wrapping_add(self.cpu.len() as u64))
    }
    pub fn bytes(&self, offset: usize, len: usize) -> Option<&[u8]> { self.cpu.get(offset..offset.checked_add(len)?) }
    pub fn bytes_mut(&mut self, offset: usize, len: usize) -> Option<&mut [u8]> { self.cpu.get_mut(offset..offset.checked_add(len)?) }
    pub fn read_u16(&self, offset: usize) -> Option<u16> { self.bytes(offset, 2).map(|b| u16::from_le_bytes([b[0], b[1]])) }
    pub fn read_u32(&self, offset: usize) -> Option<u32> { self.bytes(offset, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) }
    pub fn read_u64(&self, offset: usize) -> Option<u64> {
        Some(self.read_u32(offset)? as u64 | (self.read_u32(offset + 4)? as u64) << 32)
    }
    pub fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), Error> {
        self.bytes_mut(offset, 4).ok_or(Error::BadRequest)?.copy_from_slice(&value.to_le_bytes());
        Ok(())
    }
    pub fn write_u64(&mut self, offset: usize, value: u64) -> Result<(), Error> {
        self.bytes_mut(offset, 8).ok_or(Error::BadRequest)?.copy_from_slice(&value.to_le_bytes());
        Ok(())
    }
}

/// Carves aligned buffers out of one DMA region, front to back. Buffers borrow
/// the region, so none outlives it and no two overlap.
pub struct DmaPool<'a> {
    rest: &'a mut [u8],
    bus: u64,
}

impl<'a> DmaPool<'a> {
    pub fn new(region: &'a mut [u8], bus: u64) -> Self { DmaPool { rest: region, bus } }
    /// Bytes left (before alignment).
    pub fn remaining(&self) -> usize { self.rest.len() }
    /// A zeroed buffer of `len` bytes whose bus address is a multiple of `align`
    /// (a power of two), or `None` when the pool is out.
    pub fn take(&mut self, len: usize, align: usize) -> Option<DmaBuf<'a>> {
        if len == 0 || !align.is_power_of_two() { return None; }
        let a = align as u64;
        let pad = ((a - (self.bus % a)) % a) as usize;
        if pad.checked_add(len)? > self.rest.len() { return None; }
        let rest = core::mem::take(&mut self.rest);
        let (_, rest) = rest.split_at_mut(pad);
        let (buf, rest) = rest.split_at_mut(len);
        self.rest = rest;
        let bus = self.bus + pad as u64;
        self.bus = bus + len as u64;
        buf.fill(0);
        Some(DmaBuf { cpu: buf, bus })
    }
}

/// Index arithmetic for a ring of `size` entries shared with a device. The
/// device writes heads and tails; [`RingIndex::checked`] is the only way such
/// a value becomes an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingIndex { size: u32 }

impl RingIndex {
    /// `size` must be at least 1.
    pub const fn new(size: u32) -> Option<RingIndex> { if size == 0 { None } else { Some(RingIndex { size }) } }
    pub const fn size(self) -> u32 { self.size }
    /// A device-reported index, refused when it is past the ring.
    pub fn checked(self, device_value: u32) -> Result<u32, Error> {
        if device_value < self.size { Ok(device_value) } else { Err(Error::BadValue) }
    }
    pub fn next(self, i: u32) -> u32 { if i + 1 >= self.size { 0 } else { i + 1 } }
    /// Entries from `from` up to (not including) `to`, going forward.
    pub fn distance(self, from: u32, to: u32) -> u32 {
        let (f, t) = (from % self.size, to % self.size);
        if t >= f { t - f } else { self.size - f + t }
    }
}
