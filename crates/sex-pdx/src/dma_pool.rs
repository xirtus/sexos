// SPDX-License-Identifier: GPL-3.0-only
//! Userland byte suballocation inside an already-assigned boot DMA region.
//! This owns no physical frame allocator and cannot extend its original region.

use crate::memory_region::{RegionView, PAGE_BYTES, PHYSICAL_END, USER_VA_END};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaBuffer {
    pub virtual_address: u64,
    pub physical_address: u64,
    pub length: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmaError { InvalidRegion, InvalidSize, Exhausted, Busy, InvalidTransition }

pub struct DmaRegionPool {
    view: RegionView,
    used: u64,
}

impl DmaRegionPool {
    /// `view` must come from successful capability-authorized DMA resolution.
    /// Local arithmetic checks do not authenticate a fabricated descriptor.
    pub fn new(view: RegionView) -> Result<Self, DmaError> {
        let physical = view.dma_address.ok_or(DmaError::InvalidRegion)?;
        if view.region_id == 0 || view.length == 0 || view.length % PAGE_BYTES != 0
            || view.virtual_address == 0 || view.virtual_address % PAGE_BYTES != 0
            || physical == 0 || physical % PAGE_BYTES != 0
            || !view.virtual_address.checked_add(view.length).is_some_and(|end| end <= USER_VA_END)
            || !physical.checked_add(view.length).is_some_and(|end| end <= PHYSICAL_END)
        {
            return Err(DmaError::InvalidRegion);
        }
        Ok(Self { view, used: 0 })
    }

    /// Permanent initialization buffers and reusable bounce-buffer backing.
    /// Never call for each request: a reusable lease below owns request lifetime.
    pub fn reserve(&mut self, bytes: u64) -> Result<DmaBuffer, DmaError> {
        if bytes == 0 { return Err(DmaError::InvalidSize); }
        let length = bytes.checked_add(PAGE_BYTES - 1).ok_or(DmaError::InvalidSize)?
            & !(PAGE_BYTES - 1);
        let end = self.used.checked_add(length).ok_or(DmaError::Exhausted)?;
        if end > self.view.length { return Err(DmaError::Exhausted); }
        let buffer = DmaBuffer {
            virtual_address: self.view.virtual_address + self.used,
            physical_address: self.view.dma_address.ok_or(DmaError::InvalidRegion)? + self.used,
            length,
        };
        self.used = end;
        Ok(buffer)
    }

    pub fn used(&self) -> u64 { self.used }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeaseState { Available, Prepared(u64), InFlight(u64), Quarantined }

/// Single-owner driver transaction. A timeout never makes DMA memory reusable.
/// There is deliberately no reset/unquarantine API: recovery requires a real
/// controller-reset design and evidence that old DMA can no longer complete.
pub struct DmaLease {
    buffer: DmaBuffer,
    state: LeaseState,
}

impl DmaLease {
    pub fn new(buffer: DmaBuffer) -> Self { Self { buffer, state: LeaseState::Available } }

    pub fn prepare(&mut self, tag: u64) -> Result<DmaBuffer, DmaError> {
        if self.state != LeaseState::Available { return Err(DmaError::Busy); }
        self.state = LeaseState::Prepared(tag);
        Ok(self.buffer)
    }

    /// Call immediately before making the device-visible submission.
    pub fn mark_submitted(&mut self, tag: u64) -> Result<(), DmaError> {
        if self.state != LeaseState::Prepared(tag) { return Err(DmaError::InvalidTransition); }
        self.state = LeaseState::InFlight(tag);
        Ok(())
    }

    pub fn abort_before_submission(&mut self, tag: u64) -> Result<(), DmaError> {
        if self.state != LeaseState::Prepared(tag) { return Err(DmaError::InvalidTransition); }
        self.state = LeaseState::Available;
        Ok(())
    }

    /// Caller must validate the actual queue's completion identity and phase.
    pub fn complete(&mut self, tag: u64) -> Result<(), DmaError> {
        if self.state != LeaseState::InFlight(tag) { return Err(DmaError::InvalidTransition); }
        self.state = LeaseState::Available;
        Ok(())
    }

    pub fn quarantine(&mut self) { self.state = LeaseState::Quarantined; }

    pub fn is_quarantined(&self) -> bool { self.state == LeaseState::Quarantined }
}
