// SPDX-License-Identifier: GPL-3.0-only
//! 4-Tier Adaptive RAM Supervisor & Non-Destructive Memory Recovery.
//!
//! Models Moan's offline plugin supervisor:
//! - Validates 128-bit job nonces (`validate_nonce`)
//! - Selects optimal tier via `select_tier` probe
//! - Pre-allocates large PCM/WAV buffers using `try_reserve_exact`
//! - On memory pressure, preserves job parameters in `Status::WaitingForMemory`
//! - Non-destructive `retry()` resumes execution once memory is freed

use crate::job::{validate_nonce, MuseError, MuseJobRequest, MuseJobResult, Status, StemRole};
use crate::pack::{ModelPackState, MuseConfig};
use crate::stems::TakeCrate;
use crate::tier::{select_tier, TierSelection};
use crate::tier0::Tier0Engine;

/// Adaptive supervisor managing generative Muse jobs across the 4 hardware tiers.
pub struct MuseSupervisor {
    config: MuseConfig,
    pack_state: ModelPackState,
    tier_selection: TierSelection,
    engine0: Tier0Engine,
    status: Status,
    active_request: Option<MuseJobRequest>,
    last_result: Option<MuseJobResult>,
    active_take: Option<TakeCrate>,
}

impl MuseSupervisor {
    /// Initialize supervisor with configuration and detected hardware RAM.
    pub fn new(config: MuseConfig, detected_ram_mb: u64, pack_state: ModelPackState) -> Self {
        let endpoint_healthy = config.endpoint_url.is_some();
        let tier_selection = select_tier(
            detected_ram_mb,
            &pack_state,
            endpoint_healthy,
            config.preferred_tier,
        );

        Self {
            config,
            pack_state,
            tier_selection,
            engine0: Tier0Engine::new(),
            status: Status::Idle,
            active_request: None,
            last_result: None,
            active_take: None,
        }
    }

    pub fn config(&self) -> &MuseConfig {
        &self.config
    }

    pub fn pack_state(&self) -> &ModelPackState {
        &self.pack_state
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn tier_selection(&self) -> &TierSelection {
        &self.tier_selection
    }

    pub fn active_take(&self) -> Option<&TakeCrate> {
        self.active_take.as_ref()
    }

    pub fn active_take_mut(&mut self) -> Option<&mut TakeCrate> {
        self.active_take.as_mut()
    }

    /// Submit a new generative job request with 128-bit nonce validation.
    pub fn submit_job(&mut self, req: MuseJobRequest) -> Result<(), MuseError> {
        if self.status == Status::Rendering || self.status == Status::Finishing {
            return Err(MuseError::Busy);
        }

        validate_nonce(req.nonce)?;

        self.active_request = Some(req);
        self.status = Status::Rendering;
        self.execute_current_job()
    }

    /// Execute or resume the active job request.
    fn execute_current_job(&mut self) -> Result<(), MuseError> {
        let Some(req) = self.active_request.as_ref() else {
            self.status = Status::Idle;
            return Ok(());
        };

        // Attempt execution (Tier 0 engine handles offline pure Rust)
        match self.engine0.execute(req) {
            Ok(result) => {
                self.status = Status::Finishing;

                // Build or update TakeCrate
                let title = match &req.mode {
                    crate::job::MuseMode::CreateSong { prompt, .. } => prompt.as_str(),
                    _ => "Muse Generation",
                };

                let mut take = TakeCrate::new(
                    u64::from_le_bytes([req.nonce[0], req.nonce[1], req.nonce[2], req.nonce[3], req.nonce[4], req.nonce[5], req.nonce[6], req.nonce[7]]),
                    title,
                    title,
                    "",
                    120,
                    "C Major",
                    StemRole::Other,
                );

                let lrc = result.understand.as_ref().map(|u| u.lyrics.clone()).unwrap_or_default();
                if let Err(e) = take.add_version(result.frames.clone(), result.stems.clone(), lrc, 0) {
                    if e == MuseError::Memory {
                        self.status = Status::WaitingForMemory;
                        return Ok(());
                    }
                }

                self.active_take = Some(take);
                self.last_result = Some(result);
                self.status = Status::Ready;
                Ok(())
            }
            Err(MuseError::Memory) => {
                // Non-destructive memory wait: retain request so user can free RAM and retry!
                self.status = Status::WaitingForMemory;
                Ok(())
            }
            Err(err) => {
                self.status = Status::Failed;
                Err(err)
            }
        }
    }

    /// Non-destructive retry after freeing system memory.
    pub fn retry(&mut self) -> Result<(), MuseError> {
        if self.status == Status::WaitingForMemory {
            self.status = Status::Rendering;
            self.execute_current_job()
        } else {
            Ok(())
        }
    }

    /// Cancel the active job.
    pub fn cancel(&mut self) {
        if self.status == Status::Rendering || self.status == Status::WaitingForMemory {
            self.status = Status::Cancelling;
            self.active_request = None;
            self.status = Status::Idle;
        }
    }

    /// Audit or consume completed generation result.
    pub fn take_result(&mut self) -> Option<MuseJobResult> {
        let res = self.last_result.take();
        if res.is_some() && self.status == Status::Ready {
            self.status = Status::Idle;
            self.active_request = None;
        }
        res
    }
}
