// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Native NVIDIA GPU driver logic: bringing up GSP-RM on Ampere laptops.
//!
//! The first target is the RTX 3070 Laptop GPU (GA104, chipset 0x174) in the
//! Alienware x17 R1. Every Turing-and-later NVIDIA GPU runs its own management
//! processor (GSP, a RISC-V core) with NVIDIA's signed firmware ("GSP-RM").
//! Clocks, memory, display and 3D all go through it, so booting it is step one.
//!
//! The boot, in the order [`boot::GspBoot`] runs it:
//! 1. Read the chip (`PMC_BOOT_0`) and the size of video memory.
//! 2. Read the VBIOS out of the GPU's ROM window and find the FWSEC ucode.
//! 3. Lay out the top of video memory: FRTS, bootloader, GSP-RM ELF, heap, and
//!    the 256-byte `GspFwWprMeta` that tells Booter where everything goes.
//! 4. Run FWSEC-FRTS on the GSP falcon (heavy secure, through the boot ROM):
//!    it carves the write-protected region WPR2.
//! 5. Reset the GSP into RISC-V mode, point it at the LibOS arguments, queue
//!    the first RPCs (system info, registry) in shared memory.
//! 6. Run Booter on the SEC2 falcon: it copies GSP-RM into WPR2 through the
//!    radix-3 page table and starts the RISC-V core.
//! 7. Wait for the `GSP_INIT_DONE` event on the message queue, running any
//!    CPU sequencer scripts GSP-RM sends meanwhile.
//!
//! Sources (no code is linked, everything is re-implemented in Rust):
//! - nouveau's GSP-RM path (`nvkm/subdev/gsp`, `nvkm/falcon`, `nvkm/subdev/bios`),
//!   MIT licensed, Copyright Red Hat Inc. and NVIDIA CORPORATION;
//! - NVIDIA's open-gpu-kernel-modules 535.113.01 (dual MIT/GPL), for the RM
//!   structure layouts (`GspFwWprMeta`, message queues, RPC headers);
//! - the nova-core VBIOS documentation (GPL-2.0+ OR MIT).
//! `docs/BRIDGE_LEDGER.md` table 2 lists them. Struct offsets were checked by
//! compiling the MIT headers with a C compiler; the tests pin them.
//!
//! Everything touches hardware through [`Mmio`] (BAR0) and plain byte slices
//! the caller has made visible to the GPU (DMA), so the same code runs in the
//! `sexgpu-nv` service and against a simulated GPU in the host tests. No heap.
#![no_std]

pub mod boot;
pub mod chip;
pub mod elf;
pub mod falcon;
pub mod fw;
pub mod layout;
pub mod libos;
pub mod msgq;
pub mod radix3;
pub mod regs;
pub mod rpc;
pub mod sequencer;
pub mod vbios;

/// 32-bit register access to BAR0 (the 16 MiB register window).
pub trait Mmio {
    fn read(&mut self, offset: u32) -> u32;
    fn write(&mut self, offset: u32, value: u32);
    fn modify(&mut self, offset: u32, clear: u32, set: u32) -> u32 {
        let v = self.read(offset);
        self.write(offset, (v & !clear) | set);
        v
    }
}

/// Time, for the waits the hardware needs. Tests count instead of sleeping.
pub trait Clock {
    fn delay_us(&mut self, us: u32);
}

/// A buffer the GPU can reach by DMA: the CPU's view and the bus address the
/// GPU uses for its first byte. The buffer is contiguous on the bus.
pub struct Dma<'a> {
    pub cpu: &'a mut [u8],
    pub bus: u64,
}

impl<'a> Dma<'a> {
    pub fn new(cpu: &'a mut [u8], bus: u64) -> Self { Dma { cpu, bus } }
    pub fn len(&self) -> usize { self.cpu.len() }
    pub fn is_empty(&self) -> bool { self.cpu.is_empty() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A register never reached the wanted value.
    Timeout { reg: u32, last: u32 },
    /// The chip is not one this driver boots.
    UnsupportedChip(u32),
    /// A firmware file or VBIOS structure is malformed.
    BadFirmware(&'static str),
    /// A buffer the caller provided is too small.
    BufferTooSmall { need: usize, have: usize },
    /// A falcon finished with a nonzero mailbox (or an error code register).
    FalconFailed { falcon: &'static str, mbox0: u32, mbox1: u32 },
    /// The fuses rule out every signature in the image.
    NoSignature(&'static str),
    /// WPR2 was already set up by a previous driver; it must be unloaded first.
    Wpr2Busy(u32),
    /// GSP-RM reported an error or sent something unexpected.
    Gsp(&'static str),
    /// The GSP message queue is full or a message is too large.
    QueueFull,
}

/// Poll until `reg & mask == want`. `budget` counts reads, `step_us` is the
/// pause between them, so a dead device can never hang the caller.
pub fn wait_for(m: &mut impl Mmio, c: &mut impl Clock, reg: u32, mask: u32, want: u32, budget: u32, step_us: u32) -> Result<u32, Error> {
    let mut last = 0;
    for i in 0..budget.max(1) {
        last = m.read(reg);
        if last & mask == want { return Ok(last); }
        if i + 1 < budget { c.delay_us(step_us); }
    }
    Err(Error::Timeout { reg, last })
}

// Little-endian access to byte slices, bounds-checked.
pub(crate) fn rd8(b: &[u8], o: usize) -> Option<u8> { b.get(o).copied() }
pub(crate) fn rd16(b: &[u8], o: usize) -> Option<u16> { Some(u16::from_le_bytes(b.get(o..o.checked_add(2)?)?.try_into().ok()?)) }
pub(crate) fn rd32(b: &[u8], o: usize) -> Option<u32> { Some(u32::from_le_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?)) }
pub(crate) fn rd64(b: &[u8], o: usize) -> Option<u64> { Some(u64::from_le_bytes(b.get(o..o.checked_add(8)?)?.try_into().ok()?)) }
pub(crate) fn wr16(b: &mut [u8], o: usize, v: u16) { b[o..o + 2].copy_from_slice(&v.to_le_bytes()); }
pub(crate) fn wr32(b: &mut [u8], o: usize, v: u32) { b[o..o + 4].copy_from_slice(&v.to_le_bytes()); }
pub(crate) fn wr64(b: &mut [u8], o: usize, v: u64) { b[o..o + 8].copy_from_slice(&v.to_le_bytes()); }

pub(crate) const fn align_up(v: u64, a: u64) -> u64 { (v + a - 1) & !(a - 1) }
pub(crate) const fn align_down(v: u64, a: u64) -> u64 { v & !(a - 1) }

/// Index of the highest set bit, counting from 1 (`fls` in C); 0 for 0.
pub(crate) fn fls(v: u32) -> u32 { 32 - v.leading_zeros() }

/// Read a little-endian u32 at `o` (public helper for services and tests).
pub fn le32(b: &[u8], o: usize) -> Option<u32> { rd32(b, o) }
/// Read a little-endian u64 at `o`.
pub fn le64(b: &[u8], o: usize) -> Option<u64> { rd64(b, o) }
