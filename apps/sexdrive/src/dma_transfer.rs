// SPDX-License-Identifier: GPL-3.0-only
//! A driver submission owns its reusable DMA backing until validated completion.
use sex_pdx::dma_pool::{DmaBuffer, DmaError, DmaLease};

pub struct DmaTransfer<'a> {
    lease: &'a mut DmaLease,
    tag: u64,
    submitted: bool,
    finished: bool,
    on_uncertain: fn(),
    pub buffer: DmaBuffer,
}

impl<'a> DmaTransfer<'a> {
    pub fn prepare(lease: &'a mut DmaLease, tag: u64, on_uncertain: fn()) -> Result<Self, DmaError> {
        let buffer = lease.prepare(tag)?;
        Ok(Self { lease, tag, submitted: false, finished: false, on_uncertain, buffer })
    }

    /// Must run immediately before the device-visible doorbell write.
    pub fn submit(&mut self) -> Result<(), DmaError> {
        self.lease.mark_submitted(self.tag)?;
        self.submitted = true;
        Ok(())
    }

    /// The actual CQE must match both CID and phase. Status errors still prove
    /// DMA completion; absence of a matching CQE never permits reuse.
    pub fn complete(&mut self, observed_cid: u16, observed_phase: u32, expected_phase: u32) -> Result<(), DmaError> {
        if observed_cid as u64 != self.tag || observed_phase != expected_phase {
            return Err(DmaError::InvalidTransition);
        }
        self.lease.complete(self.tag)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for DmaTransfer<'_> {
    fn drop(&mut self) {
        if self.finished { return; }
        if self.submitted {
            self.lease.quarantine();
            (self.on_uncertain)();
        } else if self.lease.abort_before_submission(self.tag).is_err() {
            self.lease.quarantine();
            (self.on_uncertain)();
        }
    }
}
