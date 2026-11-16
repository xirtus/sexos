// SPDX-License-Identifier: GPL-3.0-only
//! Generative Job definitions, MuseMode requests, Stem roles, and 128-bit Nonce validation.
//!
//! Modeled on Moan's offline plugin supervisor with `try_reserve_exact` buffer allocations,
//! 128-bit job nonce validation, and non-destructive `Status::WaitingForMemory` recovery.

use alloc::{string::String, sync::Arc, vec::Vec};
use moan_core::Frame;
use crate::lrc::LrcLine;
use crate::tier::RamTier;

/// 128-bit cryptographic job nonce.
pub type Nonce = [u8; 16];

/// Validate that a job nonce is non-zero and valid.
pub fn validate_nonce(nonce: Nonce) -> Result<(), MuseError> {
    if nonce == [0; 16] {
        Err(MuseError::InvalidNonce)
    } else {
        Ok(())
    }
}

/// Generation job execution lifecycle status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Memory allocation failed; completed job inputs & buffers kept in memory for non-destructive retry.
    WaitingForMemory,
    /// Engine is idle, ready for new jobs.
    Idle,
    /// Generative synthesis or analysis in progress.
    Rendering,
    /// Stems, takes, and LRC lyrics are being finalized.
    Finishing,
    /// Cancellation requested; awaiting retirement.
    Cancelling,
    /// Generation complete; takes and audio ready to audition and insert.
    Ready,
    /// Job failed; original timeline audio remains 100% untouched.
    Failed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::WaitingForMemory => "Completed generation kept; free memory and retry",
            Self::Idle => "Muse generative engine idle",
            Self::Rendering => "Generating audio with Muse engine",
            Self::Finishing => "Finishing Muse stems and takes",
            Self::Cancelling => "Cancelling Muse generation",
            Self::Ready => "Muse audio ready to audition and insert",
            Self::Failed => "Muse generation failed; original region unchanged",
        }
    }
}

/// Errors occurring during job validation, buffer allocation, or execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuseError {
    /// Insufficient memory for PCM or HTTP buffer; triggers Status::WaitingForMemory.
    Memory,
    /// Nonce is all zeros or invalid.
    InvalidNonce,
    /// Another generation job is currently running.
    Busy,
    /// Input parameters out of range or invalid.
    InvalidParameters(&'static str),
    /// Job was cancelled by the user.
    Cancelled,
    /// Remote bridge or sidecar network error.
    Network(String),
    /// Upstream protocol encoding or decoding error.
    Protocol(&'static str),
    /// Audio WAV format or decoding error.
    WavFormat(&'static str),
    /// Internal execution failure.
    Failed(&'static str),
}

impl MuseError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Memory => "Not enough memory for generative audio buffer; free memory and retry",
            Self::InvalidNonce => "Job nonce is invalid or empty",
            Self::Busy => "A generative job is already running",
            Self::InvalidParameters(msg) => msg,
            Self::Cancelled => "Generation job cancelled",
            Self::Network(_) => "Remote bridge connection failed",
            Self::Protocol(msg) => msg,
            Self::WavFormat(msg) => msg,
            Self::Failed(msg) => msg,
        }
    }
}

/// Suno Studio 1.1 / 2.0 stem categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StemRole {
    Vocals,
    Drums,
    Bass,
    Chords,
    Fx,
    Other,
}

impl StemRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Vocals => "Vocals",
            Self::Drums => "Drums",
            Self::Bass => "Bass",
            Self::Chords => "Chords",
            Self::Fx => "FX",
            Self::Other => "Other",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        let lower = s.to_ascii_lowercase();
        if lower.contains("vocal") || lower.contains("voice") || lower.contains("sing") {
            Some(Self::Vocals)
        } else if lower.contains("drum") || lower.contains("beat") || lower.contains("perc") {
            Some(Self::Drums)
        } else if lower.contains("bass") || lower.contains("sub") {
            Some(Self::Bass)
        } else if lower.contains("chord") || lower.contains("pad") || lower.contains("harmony") || lower.contains("key") {
            Some(Self::Chords)
        } else if lower.contains("fx") || lower.contains("effect") || lower.contains("sweep") {
            Some(Self::Fx)
        } else if lower.contains("other") || lower.contains("lead") || lower.contains("inst") {
            Some(Self::Other)
        } else {
            None
        }
    }

    pub fn all() -> [Self; 6] {
        [
            Self::Vocals,
            Self::Drums,
            Self::Bass,
            Self::Chords,
            Self::Fx,
            Self::Other,
        ]
    }
}

/// Generative mode corresponding to Suno Studio 1.1 + 2.0 workflows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuseMode {
    /// Full song creation from prompt and optional lyrics.
    CreateSong {
        prompt: String,
        lyrics: String,
        vocal_language: String,
        bpm: u16,
        key_scale: String,
        time_sig: [u8; 2],
        duration_frames: usize,
        thinking_cot: bool,
        batch_count: u8,
    },
    /// Suno Studio context-aware overdub conditioned on Moan's rendered reference mix window.
    AddLayer {
        role: StemRole,
    },
    /// Suno Studio timeline Replace inpainting with smooth equal-power crossfade splicing.
    ReplaceSection {
        from_frame: usize,
        to_frame: usize,
    },
    /// Extend timeline past from_frame for extra_frames.
    Extend {
        from_frame: usize,
        extra_frames: usize,
    },
    /// Suno Studio "Mouth Beat" / hummed mic-to-instrument transformation or style cover.
    Cover {
        style_strength_pct: u8,
    },
    /// Generates full backing arrangement around dry/Ariel vocals.
    Vocal2Bgm,
    /// Splits audio into StemRole::{Vocals, Drums, Bass, Chords, Fx, Other}.
    ExtractStems,
    /// Suno Studio 2.0 MIDI-as-Prompt: bakes MIDI guide and renders studio audio.
    MidiToAudio,
    /// Extracts detected_bpm, detected_key, caption, and timed LRC lyrics.
    UnderstandAudio,
}

/// Request to execute a generative job.
#[derive(Clone, Debug)]
pub struct MuseJobRequest {
    pub nonce: Nonce,
    pub mode: MuseMode,
    pub reference_pcm: Option<Arc<Vec<Frame>>>,
    pub midi_pattern: Option<Arc<moan_core::midi::Pattern>>,
    pub target_tier: RamTier,
    pub created_at_ms: u64,
}

impl MuseJobRequest {
    pub fn new(nonce: Nonce, mode: MuseMode, target_tier: RamTier) -> Result<Self, MuseError> {
        validate_nonce(nonce)?;
        Ok(Self {
            nonce,
            mode,
            reference_pcm: None,
            midi_pattern: None,
            target_tier,
            created_at_ms: 0,
        })
    }
}

/// Result of audio understanding / transcription.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnderstandResult {
    pub detected_bpm: u16,
    pub detected_key: String,
    pub caption: String,
    pub lyrics: Vec<LrcLine>,
}

/// Completed audio and metadata from a generative job.
#[derive(Clone, Debug)]
pub struct MuseJobResult {
    pub nonce: Nonce,
    pub frames: Arc<Vec<Frame>>,
    pub stems: Vec<(StemRole, Arc<Vec<Frame>>)>,
    pub understand: Option<UnderstandResult>,
    pub clipped_samples: u32,
}

/// Safely allocate a buffer of `Frame`s using `try_reserve_exact`.
pub fn try_alloc_frames(len: usize) -> Result<Vec<Frame>, MuseError> {
    let mut vec = Vec::new();
    vec.try_reserve_exact(len).map_err(|_| MuseError::Memory)?;
    vec.resize(len, [0, 0]);
    Ok(vec)
}

/// Safely allocate a buffer of bytes using `try_reserve_exact`.
pub fn try_alloc_bytes(len: usize) -> Result<Vec<u8>, MuseError> {
    let mut vec = Vec::new();
    vec.try_reserve_exact(len).map_err(|_| MuseError::Memory)?;
    vec.resize(len, 0);
    Ok(vec)
}
