// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Register access. A driver is written against [`Regs`], never against a raw
//! pointer, so a test can hand it a device model and a fuzzer a hostile device.

/// What a register reads when the device is gone (removed, powered down, or a
/// bus error): all ones.
pub const GONE: u32 = 0xffff_ffff;

/// True when a register read says the device is not there any more.
pub const fn is_gone(value: u32) -> bool { value == GONE }

/// Bits `shift..shift + width` of `value`.
pub const fn field(value: u32, shift: u32, width: u32) -> u32 {
    if width == 0 { return 0; }
    let v = if shift >= 32 { 0 } else { value >> shift };
    if width >= 32 { v } else { v & ((1u32 << width) - 1) }
}

/// 32-bit register reads and writes at byte offsets in one device's window.
///
/// 64-bit registers are two 32-bit accesses, low half first (what most PCI
/// devices require and what a split read on a 32-bit bus does).
pub trait Regs {
    fn read32(&mut self, offset: u32) -> u32;
    fn write32(&mut self, offset: u32, value: u32);

    fn read64(&mut self, offset: u32) -> u64 {
        let lo = self.read32(offset) as u64;
        let hi = self.read32(offset.wrapping_add(4)) as u64;
        lo | hi << 32
    }
    fn write64(&mut self, offset: u32, value: u64) {
        self.write32(offset, value as u32);
        self.write32(offset.wrapping_add(4), (value >> 32) as u32);
    }
    /// Read, clear `clear`, set `set`, write back; returns the value written.
    fn modify32(&mut self, offset: u32, clear: u32, set: u32) -> u32 {
        let v = (self.read32(offset) & !clear) | set;
        self.write32(offset, v);
        v
    }
}

impl<R: Regs + ?Sized> Regs for &mut R {
    fn read32(&mut self, offset: u32) -> u32 { (**self).read32(offset) }
    fn write32(&mut self, offset: u32, value: u32) { (**self).write32(offset, value) }
    fn read64(&mut self, offset: u32) -> u64 { (**self).read64(offset) }
    fn write64(&mut self, offset: u32, value: u64) { (**self).write64(offset, value) }
}

/// Memory-mapped registers: a window the driver's service mapped (a leased BAR
/// view). Accesses outside the window or not 4-byte aligned never touch memory:
/// reads return [`GONE`], writes are dropped, and both are counted, so a driver
/// bug that computes a bad offset from device data is a record, not a fault.
pub struct Mmio {
    base: *mut u8,
    len: usize,
    stray: u32,
}

impl Mmio {
    /// # Safety
    /// `base..base + len` must be a device register mapping that stays mapped
    /// for the life of this value and is not accessed through another `Mmio`.
    pub const unsafe fn new(base: *mut u8, len: usize) -> Mmio { Mmio { base, len, stray: 0 } }
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    /// Accesses refused because they fell outside the window or were unaligned.
    pub fn stray_accesses(&self) -> u32 { self.stray }
    fn ok(&mut self, offset: u32, width: usize) -> bool {
        let o = offset as usize;
        if o % width != 0 || o.checked_add(width).map_or(true, |e| e > self.len) {
            self.stray = self.stray.saturating_add(1);
            return false;
        }
        true
    }
}

impl Regs for Mmio {
    fn read32(&mut self, offset: u32) -> u32 {
        if !self.ok(offset, 4) { return GONE; }
        // SAFETY: in bounds and aligned (checked above); `new`'s contract covers the mapping.
        unsafe { core::ptr::read_volatile(self.base.add(offset as usize) as *const u32) }
    }
    fn write32(&mut self, offset: u32, value: u32) {
        if !self.ok(offset, 4) { return; }
        // SAFETY: as above.
        unsafe { core::ptr::write_volatile(self.base.add(offset as usize) as *mut u32, value) }
    }
}
