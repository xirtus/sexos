// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The narrow window onto the hardware that Bell implements.
//!
//! Bell owns the whole audio PCI function. It maps BAR0 (HD Audio, 16 KiB) and
//! BAR4 (DSP, 1 MiB) from its device lease and hands this crate plain offset-based
//! accessors, a microsecond delay and one DMA scratch area. Nothing here knows about
//! leases, page tables or interrupts, so the boot runs the same against the real
//! controller and against the register model in the tests.

/// What the DSP boot needs from its host.
///
/// Offsets are byte offsets from the start of the BAR. Accesses must be naturally
/// aligned and are little-endian, exactly as the MMIO registers are.
pub trait Hw {
    /// Reads an 8-bit HD Audio register (BAR0).
    fn hda_read8(&mut self, offset: u32) -> u8;
    /// Reads a 16-bit HD Audio register (BAR0).
    fn hda_read16(&mut self, offset: u32) -> u16;
    /// Reads a 32-bit HD Audio register (BAR0).
    fn hda_read32(&mut self, offset: u32) -> u32;
    /// Writes an 8-bit HD Audio register (BAR0).
    fn hda_write8(&mut self, offset: u32, value: u8);
    /// Writes a 16-bit HD Audio register (BAR0).
    fn hda_write16(&mut self, offset: u32, value: u16);
    /// Writes a 32-bit HD Audio register (BAR0).
    fn hda_write32(&mut self, offset: u32, value: u32);

    /// Reads a 32-bit DSP register or SRAM window word (BAR4).
    fn dsp_read32(&mut self, offset: u32) -> u32;
    /// Writes a 32-bit DSP register or SRAM window word (BAR4).
    fn dsp_write32(&mut self, offset: u32, value: u32);

    /// Busy-waits (or sleeps) at least `us` microseconds.
    fn delay_us(&mut self, us: u32);

    /// The DMA scratch area: a CPU view of physically contiguous memory and the bus
    /// address the controller sees for its first byte. The boot puts the buffer
    /// descriptor lists and the firmware copy here, so it must be at least
    /// [`crate::boot::scratch_needed`] bytes and 128-byte aligned on the bus.
    /// It must be DMA-coherent, or [`Hw::dma_sync`] must make it so.
    fn dma(&mut self) -> (&mut [u8], u64);

    /// Called after the CPU has written the scratch area and before the controller
    /// reads it. The default does nothing (coherent memory).
    fn dma_sync(&mut self) {}
}
