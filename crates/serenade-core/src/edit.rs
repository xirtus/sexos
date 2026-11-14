//! Non-destructive audio clip editing operations:
//! trimming, marker splitting, peak normalization, gain, fade curves,
//! DC-offset removal, and silence trimming.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::f32::consts::PI;

use crate::meter::dbfs_to_linear;
use crate::SerenadeError;

/// Fade curve characteristic for fade-in / fade-out operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeCurve {
    /// Linear ramp: $y = t$.
    Linear,
    /// Smooth cosine S-curve: $y = \frac{1 - \cos(\pi t)}{2}$.
    Cosine,
    /// Smooth cubic step: $y = 3t^2 - 2t^3$.
    SmoothStep,
}

impl FadeCurve {
    /// Evaluates the curve factor at normalized progress $t \in [0.0, 1.0]$.
    pub fn factor(&self, t: f32) -> f32 {
        let clamped = if t < 0.0 {
            0.0
        } else if t > 1.0 {
            1.0
        } else {
            t
        };

        match self {
            Self::Linear => clamped,
            Self::Cosine => 0.5 * (1.0 - libm::cosf(PI * clamped)),
            Self::SmoothStep => clamped * clamped * (3.0 - 2.0 * clamped),
        }
    }
}

/// A named, timestamped cue marker placed along an audio clip timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    /// Unique marker identifier.
    pub id: u32,
    /// Descriptive name or label.
    pub name: String,
    /// Absolute frame offset within the clip.
    pub frame_index: u64,
    /// Timestamp offset in milliseconds.
    pub timestamp_ms: u64,
}

/// An in-memory audio clip representation storing interleaved PCM frames
/// and associated timeline markers.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioClip {
    /// Channel count (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Sample rate in Hz (e.g. 48000).
    pub sample_rate: u32,
    /// Interleaved 32-bit floating point audio samples in range [-1.0, 1.0].
    pub samples: Vec<f32>,
    /// Timeline cue markers.
    pub markers: Vec<Marker>,
}

impl AudioClip {
    /// Creates a new audio clip.
    pub fn new(
        channels: u16,
        sample_rate: u32,
        samples: Vec<f32>,
        markers: Vec<Marker>,
    ) -> Result<Self, SerenadeError> {
        if channels == 0 || channels > 2 {
            return Err(SerenadeError::InvalidFormat("channels must be 1 (mono) or 2 (stereo)"));
        }
        let ch = channels;
        if sample_rate == 0 {
            return Err(SerenadeError::InvalidSampleRate);
        }
        if samples.len() % (ch as usize) != 0 {
            return Err(SerenadeError::InvalidFormat("samples len not aligned to channels"));
        }
        Ok(Self {
            channels: ch,
            sample_rate,
            samples,
            markers,
        })
    }

    /// Total number of multi-channel frames in the clip.
    pub fn frame_count(&self) -> usize {
        let ch = (self.channels as usize).max(1);
        self.samples.len() / ch
    }

    /// Duration of the audio clip in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        let frames = self.frame_count() as u64;
        let sr = (self.sample_rate as u64).max(1);
        frames.saturating_mul(1000) / sr
    }

    /// Non-destructively trims the clip to `[start_frame, end_frame)`.
    ///
    /// Markers within the trimmed window are retained and re-indexed.
    pub fn trim(&self, start_frame: usize, end_frame: usize) -> Result<Self, SerenadeError> {
        let total = self.frame_count();
        if start_frame > end_frame || end_frame > total {
            return Err(SerenadeError::InvalidEditRange {
                start: start_frame,
                end: end_frame,
                total,
            });
        }

        let ch = self.channels as usize;
        let sample_start = start_frame * ch;
        let sample_end = end_frame * ch;
        let trimmed_samples = self.samples[sample_start..sample_end].to_vec();

        let mut trimmed_markers = Vec::new();
        let start_u64 = start_frame as u64;
        let end_u64 = end_frame as u64;

        for m in &self.markers {
            if m.frame_index >= start_u64 && m.frame_index < end_u64 {
                let new_frame = m.frame_index - start_u64;
                let new_ts = (new_frame * 1000) / (self.sample_rate as u64);
                trimmed_markers.push(Marker {
                    id: m.id,
                    name: m.name.clone(),
                    frame_index: new_frame,
                    timestamp_ms: new_ts,
                });
            }
        }

        Ok(Self {
            channels: self.channels,
            sample_rate: self.sample_rate,
            samples: trimmed_samples,
            markers: trimmed_markers,
        })
    }

    /// Splits the clip into multiple clips at the locations of all markers.
    ///
    /// The clips are returned in chronological order.
    pub fn split_at_markers(&self) -> Result<Vec<Self>, SerenadeError> {
        let total = self.frame_count();
        if self.markers.is_empty() || total == 0 {
            return Ok(vec![self.clone()]);
        }

        // Collect and sort marker frame offsets
        let mut split_points: Vec<usize> = self
            .markers
            .iter()
            .map(|m| (m.frame_index as usize).min(total))
            .collect();
        split_points.sort_unstable();
        split_points.dedup();

        let mut result = Vec::new();
        let mut cur = 0;

        for &pt in &split_points {
            if pt > cur {
                result.push(self.trim(cur, pt)?);
                cur = pt;
            }
        }

        if cur < total {
            result.push(self.trim(cur, total)?);
        }

        Ok(result)
    }

    /// Normalizes the peak level to `target_linear` (e.g. 1.0 for 0 dBFS or 0.988 for -0.1 dBFS).
    ///
    /// Returns the applied linear gain factor.
    pub fn normalize_peak(&mut self, target_linear: f32) -> Result<f32, SerenadeError> {
        if target_linear <= 0.0 || !target_linear.is_finite() {
            return Err(SerenadeError::InvalidParameter("target_linear must be positive and finite"));
        }

        let mut max_abs = 0.0f32;
        for &s in &self.samples {
            let a = if s.is_finite() {
                if s < 0.0 { -s } else { s }
            } else {
                0.0
            };
            if a > max_abs {
                max_abs = a;
            }
        }

        if max_abs <= 1e-9 || !max_abs.is_finite() {
            return Ok(1.0);
        }

        let gain = target_linear / max_abs;
        if !gain.is_finite() {
            return Ok(1.0);
        }
        self.apply_gain(gain);
        Ok(gain)
    }

    /// Applies a constant linear gain multiplier to all samples.
    pub fn apply_gain(&mut self, gain_linear: f32) {
        for s in &mut self.samples {
            *s *= gain_linear;
        }
    }

    /// Applies gain specified in dB.
    pub fn apply_gain_db(&mut self, gain_db: f32) {
        let linear = dbfs_to_linear(gain_db);
        self.apply_gain(linear);
    }

    /// Applies a fade-in curve to the first `duration_frames` of the clip.
    pub fn fade_in(&mut self, duration_frames: usize, curve: FadeCurve) -> Result<(), SerenadeError> {
        let total = self.frame_count();
        let n = duration_frames.min(total);
        if n == 0 {
            return Ok(());
        }

        let ch = self.channels as usize;
        let denom = if n > 1 { (n - 1) as f32 } else { 1.0 };
        for frame in 0..n {
            let t = frame as f32 / denom;
            let factor = curve.factor(t);
            let idx = frame * ch;
            for c in 0..ch {
                self.samples[idx + c] *= factor;
            }
        }
        Ok(())
    }

    /// Applies a fade-out curve to the last `duration_frames` of the clip.
    pub fn fade_out(&mut self, duration_frames: usize, curve: FadeCurve) -> Result<(), SerenadeError> {
        let total = self.frame_count();
        let n = duration_frames.min(total);
        if n == 0 {
            return Ok(());
        }

        let ch = self.channels as usize;
        let start_frame = total - n;
        let denom = if n > 1 { (n - 1) as f32 } else { 1.0 };

        for step in 0..n {
            let t = 1.0 - (step as f32 / denom);
            let factor = curve.factor(t);
            let idx = (start_frame + step) * ch;
            for c in 0..ch {
                self.samples[idx + c] *= factor;
            }
        }
        Ok(())
    }

    /// Computes and removes DC offset independently from each audio channel.
    pub fn remove_dc_offset(&mut self) {
        let total_frames = self.frame_count();
        if total_frames == 0 {
            return;
        }

        let ch = self.channels as usize;
        let mut sums = [0.0f64; 2];

        for frame in 0..total_frames {
            let base = frame * ch;
            for c in 0..ch {
                sums[c] += self.samples[base + c] as f64;
            }
        }

        let means = [
            (sums[0] / total_frames as f64) as f32,
            if ch > 1 { (sums[1] / total_frames as f64) as f32 } else { 0.0 },
        ];

        for frame in 0..total_frames {
            let base = frame * ch;
            for c in 0..ch {
                self.samples[base + c] -= means[c];
            }
        }
    }

    /// Trims silence from the beginning and end of the clip based on a dBFS threshold.
    pub fn trim_silence(
        &self,
        threshold_dbfs: f32,
        _min_silence_frames: usize,
    ) -> Result<Self, SerenadeError> {
        let total = self.frame_count();
        if total == 0 {
            return Ok(self.clone());
        }

        let threshold_linear = dbfs_to_linear(threshold_dbfs);
        let ch = self.channels as usize;

        // Find first audible frame
        let mut first_audible = None;
        for frame in 0..total {
            let base = frame * ch;
            let mut is_audible = false;
            for c in 0..ch {
                let s = self.samples[base + c];
                let a = if s < 0.0 { -s } else { s };
                if a >= threshold_linear {
                    is_audible = true;
                    break;
                }
            }
            if is_audible {
                first_audible = Some(frame);
                break;
            }
        }

        let start = match first_audible {
            Some(f) => f,
            None => {
                // Entire clip is silent
                return Ok(Self {
                    channels: self.channels,
                    sample_rate: self.sample_rate,
                    samples: Vec::new(),
                    markers: Vec::new(),
                });
            }
        };

        // Find last audible frame
        let mut last_audible = start;
        for frame in (start..total).rev() {
            let base = frame * ch;
            let mut is_audible = false;
            for c in 0..ch {
                let s = self.samples[base + c];
                let a = if s < 0.0 { -s } else { s };
                if a >= threshold_linear {
                    is_audible = true;
                    break;
                }
            }
            if is_audible {
                last_audible = frame;
                break;
            }
        }

        self.trim(start, last_audible + 1)
    }
}
