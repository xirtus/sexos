//! Serenade Core — High-performance `#![no_std] + alloc` audio recording,
//! sample-accurate level metering, progressive multi-resolution waveform mipmaps,
//! non-destructive clip editing, and bounds-checked RIFF/WAVE container engine.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod edit;
pub mod meter;
pub mod polish;
pub mod recorder;
pub mod wav;
pub mod waveform;

pub use edit::{AudioClip, FadeCurve, Marker};
pub use meter::{
    dbfs_to_linear, linear_to_dbfs, linear_to_dbfs_approx, ChannelMeter, LevelMeter, MIN_DBFS,
    MIN_DBFS_LINEAR,
};
pub use polish::{
    clean_hum, de_ess, measure_loudness, normalize_podcast_lufs, polish_podcast_voice,
    suppress_background_noise, LoudnessReadout, PODCAST_TARGET_LUFS, TRUE_PEAK_CEILING_DBTP,
};
pub use recorder::{
    CaptureSession, FeedStats, RecorderConfig, RecorderState, VoxConfig, VoxStatus,
};
pub use wav::{decode_wav, encode_wav, DecodedWav, WavError, WavFormat, WavMetadata};
pub use waveform::{WaveformBin, WaveformMipmap, LEVEL_SIZES, MIPMAP_LEVEL_COUNT};

/// Top-level error type for Serenade operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SerenadeError {
    /// Attempted an operation on a finalized or stopped session.
    SessionClosed,
    /// Invalid state machine transition.
    InvalidState(&'static str),
    /// Memory allocation cap exceeded.
    BufferFull {
        current_samples: usize,
        max_samples: usize,
    },
    /// Memory allocation failed (OOM protection).
    AllocationFailed,
    /// Sample rate must be non-zero.
    InvalidSampleRate,
    /// Invalid operational parameter.
    InvalidParameter(&'static str),
    /// Invalid audio sample or channel format.
    InvalidFormat(&'static str),
    /// Invalid slice or trim boundary range.
    InvalidEditRange {
        start: usize,
        end: usize,
        total: usize,
    },
    /// WAV decoding or encoding error.
    Wav(WavError),
}
