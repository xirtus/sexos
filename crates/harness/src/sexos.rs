// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The real system, for a driver service: a leased device's registers, the
//! service's DMA pool, its interrupt bits and the clock, all through `sex_pdx`.
//! Only the service's own `main` touches this module; the driver crate stays
//! generic over [`Regs`], [`Clock`], [`Irq`] and [`DmaPool`], so the same code
//! is tested on the host.
//!
//! ```ignore
//! let lease = harness::sexos::lease(SLOT)?;            // what BOOT leased at SLOT
//! let mut regs = harness::sexos::LeasedRegs::open(SLOT)?; // its BAR view
//! let mut dma = harness::sexos::dma_pool()?;            // the service's DMA pool
//! let mut irq = harness::sexos::LeaseIrq::new(lease);
//! let mut clock = harness::sexos::SystemClock;
//! let mut out = harness::sexos::Serial;
//! let dev = my_driver::Device::start(&mut regs, &mut clock, &mut irq, &mut out)?;
//! ```

use sex_pdx::device::Lease;
use sex_pdx::memory_region as m;

use crate::dma::DmaPool;
use crate::error::Error;
use crate::irq::Irq;
use crate::record::Sink;
use crate::regs::{Mmio, Regs};
use crate::wait::Clock;

/// The lease BOOT made at `slot` for this service (identity, location, vectors).
pub fn lease(slot: u64) -> Result<Lease, Error> { sex_pdx::device::at_slot(slot).ok_or(Error::Denied) }

/// The register window of the device leased at `slot` (read and write view of
/// the BAR BOOT chose for its lease kind).
pub struct LeasedRegs { mmio: Mmio, pub slot: u64 }

impl LeasedRegs {
    pub fn open(slot: u64) -> Result<LeasedRegs, Error> {
        let view = sex_pdx::memory_region_full_view(slot, m::QUERY_VIEW | m::READ | m::WRITE).map_err(|_| Error::Denied)?;
        if view.virtual_address == 0 || view.length == 0 { return Err(Error::Denied); }
        // SAFETY: the kernel mapped this view for this service and keeps it
        // mapped while the lease lasts; nothing else in the service opens it.
        let mmio = unsafe { Mmio::new(view.virtual_address as *mut u8, view.length as usize) };
        Ok(LeasedRegs { mmio, slot })
    }
    pub fn stray_accesses(&self) -> u32 { self.mmio.stray_accesses() }
    /// Turn on memory decoding and bus mastering (PCI command bits) for this
    /// lease, as the lease kind allows.
    pub fn enable(&self, bus_master: bool) -> Result<(), Error> {
        let bits = 0x2 | if bus_master { 0x4 } else { 0 };
        sex_pdx::device_command(self.slot, bits, 0).map(|_| ()).map_err(|_| Error::Denied)
    }
}

impl Regs for LeasedRegs {
    fn read32(&mut self, offset: u32) -> u32 { self.mmio.read32(offset) }
    fn write32(&mut self, offset: u32, value: u32) { self.mmio.write32(offset, value) }
}

/// The service's DMA pool (`SLOT_DMA_REGION`): memory its devices may reach.
pub fn dma_pool() -> Result<DmaPool<'static>, Error> {
    let info = sex_pdx::memory_region_info(sex_pdx::SLOT_DMA_REGION).map_err(|_| Error::Denied)?;
    let view = sex_pdx::memory_region_full_view(sex_pdx::SLOT_DMA_REGION, info.rights).map_err(|_| Error::Denied)?;
    let bus = view.dma_address.ok_or(Error::Denied)?;
    // SAFETY: the pool is this service's for its whole life; call this once.
    let s = unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, view.length as usize) };
    Ok(DmaPool::new(s, bus))
}

/// Milliseconds since boot; a sleep also ends when an interrupt bit arrives.
pub struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&mut self) -> u64 { sex_pdx::get_ticks() }
    fn sleep_ms(&mut self, ms: u64) {
        if ms == 0 { sex_pdx::sys_yield() } else { let _ = sex_pdx::sys_wait_self_ticks(ms); }
    }
}

/// The interrupt vectors BOOT programmed for one lease (syscall 86). Bits of
/// other leases of the same service are kept for them, not dropped.
pub struct LeaseIrq { lease: Lease, others: u64 }
impl LeaseIrq {
    pub fn new(lease: Lease) -> Self { LeaseIrq { lease, others: 0 } }
    /// Pending bits that belonged to other leases of this service.
    pub fn others(&mut self) -> u64 { core::mem::take(&mut self.others) }
}
impl Irq for LeaseIrq {
    fn unmask(&mut self, vector: u8) -> Result<(), Error> {
        if vector >= self.lease.vectors { return Err(Error::Denied); }
        sex_pdx::device::irq_unmask(self.lease.slot, vector as u64).map_err(|_| Error::Denied)
    }
    fn mask(&mut self, vector: u8) -> Result<(), Error> {
        if vector >= self.lease.vectors { return Err(Error::Denied); }
        sex_pdx::device::irq_mask(self.lease.slot, vector as u64).map_err(|_| Error::Denied)
    }
    fn take(&mut self) -> u64 {
        let all = sex_pdx::device::irq_take();
        let mine = self.lease.bits();
        self.others |= all & !mine;
        (all & mine) >> self.lease.first_bit
    }
}

/// The serial record stream (what gates and `hw save` read).
pub struct Serial;
impl Sink for Serial {
    fn line(&mut self, line: &str) { sex_pdx::serial_record!("{}", line); }
}
