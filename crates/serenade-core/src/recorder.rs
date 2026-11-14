//! Capture session and bounded take buffer with VOX (voice-activated recording)
//! and strict memory limits.

use alloc::string::ToString;
use alloc::vec::Vec;

use crate::edit::{AudioClip, Marker};
use crate::meter::{dbfs_to_linear, LevelMeter};
use crate::waveform::WaveformMipmap;
use crate::SerenadeError;

/// Operational state of the audio recorder session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderState {
    /// Initial idle state before recording starts.
    Idle,
    /// Actively recording incoming audio streams.
    Recording,
    /// Temporarily paused; incoming audio is discarded.
    Paused,
    /// Finalized and stopped; session is closed.
    Stopped,
}

/// Dynamic VOX gate status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxStatus {
    /// VOX is not enabled; all recording frames are captured unconditionally.
    Disabled,
    /// Below threshold silence; audio frames are gated out.
    Gated,
    /// Voice detected above threshold; capturing audio.
    Active,
    /// Voice level dropped below threshold; hangover hold period is active.
    Hold,
}

/// Voice-Activated Recording (VOX) configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct VoxConfig {
    /// Gate trigger threshold in dBFS (e.g. -40.0 dBFS).
    pub threshold_dbfs: f32,
    /// Hangover duration in milliseconds after audio drops below threshold (e.g. 400 ms).
    pub hold_ms: u32,
    /// Pre-roll duration in milliseconds preserved prior to gate trigger (e.g. 50 ms).
    pub pre_roll_ms: u32,
}

/// Statistics reported after feeding an audio block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedStats {
    /// Number of multi-channel frames committed to the take buffer.
    pub frames_recorded: usize,
    /// Number of frames discarded by VOX silence gating.
    pub frames_gated: usize,
    /// Current status of the VOX engine.
    pub vox_status: VoxStatus,
}

/// Audio recorder session configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct RecorderConfig {
    /// Number of audio channels (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Sample rate in Hz (e.g. 48000).
    pub sample_rate: u32,
    /// Maximum allowed total samples in memory buffer before rejecting additions.
    pub max_samples: usize,
    /// Optional voice-activated recording configuration.
    pub vox: Option<VoxConfig>,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            channels: 2,
            sample_rate: 48000,
            // Default 10 minutes of stereo 48 kHz (48000 * 2 * 600 = 57,600,000 samples ~ 230 MB)
            max_samples: 57_600_000,
            vox: None,
        }
    }
}

/// Circular pre-roll buffer to prevent cutting off voice attacks when VOX triggers.
#[derive(Clone, Debug)]
struct PreRollBuffer {
    buffer: Vec<f32>,
    capacity_samples: usize,
    write_pos: usize,
    count: usize,
}

impl PreRollBuffer {
    fn new(capacity_samples: usize) -> Result<Self, SerenadeError> {
        let mut b = Vec::new();
        if capacity_samples > 0 {
            b.try_reserve_exact(capacity_samples)
                .map_err(|_| SerenadeError::AllocationFailed)?;
            b.resize(capacity_samples, 0.0);
        }
        Ok(Self {
            buffer: b,
            capacity_samples,
            write_pos: 0,
            count: 0,
        })
    }

    fn push_frame(&mut self, frame: &[f32]) {
        if self.capacity_samples == 0 {
            return;
        }
        for &s in frame {
            self.buffer[self.write_pos] = s;
            self.write_pos = (self.write_pos + 1) % self.capacity_samples;
            if self.count < self.capacity_samples {
                self.count += 1;
            }
        }
    }

    fn drain_into(&mut self, out: &mut Vec<f32>) {
        if self.count == 0 || self.capacity_samples == 0 {
            return;
        }
        let start = if self.count < self.capacity_samples {
            0
        } else {
            self.write_pos
        };

        for i in 0..self.count {
            let idx = (start + i) % self.capacity_samples;
            out.push(self.buffer[idx]);
        }
        self.count = 0;
        self.write_pos = 0;
    }

    fn clear(&mut self) {
        self.count = 0;
        self.write_pos = 0;
    }
}

/// Capture session and bounded take buffer.
#[derive(Clone, Debug)]
pub struct CaptureSession {
    channels: u16,
    sample_rate: u32,
    max_samples: usize,
    state: RecorderState,

    // VOX parameters
    vox_config: Option<VoxConfig>,
    vox_status: VoxStatus,
    vox_threshold_linear: f32,
    vox_hold_frames: u32,
    vox_hold_remaining: u32,
    pre_roll: PreRollBuffer,

    // Recorded data
    samples: Vec<f32>,
    markers: Vec<Marker>,
    next_marker_id: u32,

    // Level metering & waveform mipmap
    meter: LevelMeter,
    waveform: WaveformMipmap,
}

impl CaptureSession {
    /// Creates a new capture session with explicit memory limits.
    pub fn new(config: RecorderConfig) -> Result<Self, SerenadeError> {
        if config.channels == 0 || config.channels > 2 {
            return Err(SerenadeError::InvalidParameter("channels must be 1 (mono) or 2 (stereo)"));
        }
        let ch = config.channels;
        if config.sample_rate == 0 {
            return Err(SerenadeError::InvalidSampleRate);
        }
        if config.max_samples == 0 {
            return Err(SerenadeError::InvalidParameter("max_samples must be greater than zero"));
        }

        let (vox_thresh_lin, vox_hold_fr, pre_roll_buf, status) = if let Some(ref v) = config.vox {
            let thresh = dbfs_to_linear(v.threshold_dbfs);
            let hold_fr = (config.sample_rate as u64 * v.hold_ms as u64 / 1000) as u32;
            let pre_roll_fr = (config.sample_rate as u64 * v.pre_roll_ms as u64 / 1000) as usize;
            let pre_roll_smp = (pre_roll_fr * (ch as usize)).min(config.max_samples);
            (
                thresh,
                hold_fr,
                PreRollBuffer::new(pre_roll_smp)?,
                VoxStatus::Gated,
            )
        } else {
            (0.0, 0, PreRollBuffer::new(0)?, VoxStatus::Disabled)
        };

        let meter = LevelMeter::new(ch, config.sample_rate, 1500, 20.0);
        let waveform = WaveformMipmap::new(ch);

        Ok(Self {
            channels: ch,
            sample_rate: config.sample_rate,
            max_samples: config.max_samples,
            state: RecorderState::Idle,
            vox_config: config.vox,
            vox_status: status,
            vox_threshold_linear: vox_thresh_lin,
            vox_hold_frames: vox_hold_fr,
            vox_hold_remaining: 0,
            pre_roll: pre_roll_buf,
            samples: Vec::new(),
            markers: Vec::new(),
            next_marker_id: 1,
            meter,
            waveform,
        })
    }

    /// Current recorder state.
    pub fn state(&self) -> RecorderState {
        self.state
    }

    /// Current VOX status.
    pub fn vox_status(&self) -> VoxStatus {
        self.vox_status
    }

    /// Starts recording.
    pub fn start(&mut self) -> Result<(), SerenadeError> {
        match self.state {
            RecorderState::Idle => {
                self.state = RecorderState::Recording;
                Ok(())
            }
            RecorderState::Recording => Ok(()),
            RecorderState::Paused => {
                self.state = RecorderState::Recording;
                Ok(())
            }
            RecorderState::Stopped => Err(SerenadeError::SessionClosed),
        }
    }

    /// Pauses recording.
    pub fn pause(&mut self) -> Result<(), SerenadeError> {
        match self.state {
            RecorderState::Recording => {
                self.state = RecorderState::Paused;
                Ok(())
            }
            RecorderState::Paused => Ok(()),
            RecorderState::Idle => Err(SerenadeError::InvalidState("cannot pause before start")),
            RecorderState::Stopped => Err(SerenadeError::SessionClosed),
        }
    }

    /// Resumes recording after pause.
    pub fn resume(&mut self) -> Result<(), SerenadeError> {
        self.start()
    }

    /// Stops recording and extracts the recorded audio clip.
    pub fn stop(&mut self) -> Result<AudioClip, SerenadeError> {
        if self.state == RecorderState::Stopped {
            return Err(SerenadeError::SessionClosed);
        }
        self.state = RecorderState::Stopped;

        AudioClip::new(
            self.channels,
            self.sample_rate,
            core::mem::take(&mut self.samples),
            core::mem::take(&mut self.markers),
        )
    }

    /// Appends a named marker at the current recorded frame position.
    pub fn add_marker(&mut self, name: &str) -> Result<Marker, SerenadeError> {
        if self.state == RecorderState::Stopped {
            return Err(SerenadeError::SessionClosed);
        }

        let ch = (self.channels as usize).max(1);
        let frame_index = (self.samples.len() / ch) as u64;
        let rate = (self.sample_rate as u64).max(1);
        let timestamp_ms = (frame_index.saturating_mul(1000)) / rate;

        let marker = Marker {
            id: self.next_marker_id,
            name: name.to_string(),
            frame_index,
            timestamp_ms,
        };
        self.next_marker_id = self.next_marker_id.saturating_add(1);
        self.markers.push(marker.clone());
        Ok(marker)
    }

    /// Access all markers added to the session.
    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    /// Total frames recorded into the take buffer so far.
    pub fn frame_count(&self) -> usize {
        let ch = (self.channels as usize).max(1);
        self.samples.len() / ch
    }

    /// Reference to the real-time level meter.
    pub fn meter(&mut self) -> &mut LevelMeter {
        &mut self.meter
    }

    /// Reference to the progressive waveform mipmap pyramid.
    pub fn waveform(&self) -> &WaveformMipmap {
        &self.waveform
    }

    /// Feeds signed 16-bit PCM frames into the capture session.
    pub fn feed_i16(&mut self, samples_i16: &[i16]) -> Result<FeedStats, SerenadeError> {
        let ch = (self.channels as usize).max(1);
        if samples_i16.len() % ch != 0 {
            return Err(SerenadeError::InvalidFormat("samples length not aligned to channels"));
        }

        // Convert chunk to f32 on stack in small batches to preserve memory
        let mut buf = [0.0f32; 256];
        let mut i = 0;
        let mut total_recorded = 0;
        let mut total_gated = 0;

        let step = (buf.len() / ch) * ch;

        while i < samples_i16.len() {
            let chunk_len = (samples_i16.len() - i).min(step);
            for k in 0..chunk_len {
                buf[k] = samples_i16[i + k] as f32 / 32768.0;
            }
            let stats = self.feed_f32(&buf[..chunk_len])?;
            total_recorded += stats.frames_recorded;
            total_gated += stats.frames_gated;
            i += chunk_len;
        }

        Ok(FeedStats {
            frames_recorded: total_recorded,
            frames_gated: total_gated,
            vox_status: self.vox_status,
        })
    }

    /// Feeds 32-bit floating point PCM frames into the capture session.
    pub fn feed_f32(&mut self, frames: &[f32]) -> Result<FeedStats, SerenadeError> {
        if self.state == RecorderState::Stopped {
            return Err(SerenadeError::SessionClosed);
        }
        let ch = (self.channels as usize).max(1);
        if frames.len() % ch != 0 {
            return Err(SerenadeError::InvalidFormat("frames length not aligned to channels"));
        }
        if frames.is_empty() {
            return Ok(FeedStats {
                frames_recorded: 0,
                frames_gated: 0,
                vox_status: self.vox_status,
            });
        }
        if self.state != RecorderState::Recording {
            // Idle or Paused: update meter display but discard audio data
            self.meter.process_block(frames);
            return Ok(FeedStats {
                frames_recorded: 0,
                frames_gated: frames.len() / ch,
                vox_status: self.vox_status,
            });
        }

        let num_frames = frames.len() / ch;
        let mut frames_recorded = 0;
        let mut frames_gated = 0;

        for frame_idx in 0..num_frames {
            let start = frame_idx * ch;
            let frame = &frames[start..start + ch];

            // Always update real-time level meter
            self.meter.process_frame(frame);

            // Compute frame peak for VOX gate decision
            let mut frame_peak = 0.0f32;
            for &s in frame {
                let a = if s.is_finite() {
                    if s < 0.0 { -s } else { s }
                } else {
                    0.0
                };
                if a > frame_peak {
                    frame_peak = a;
                }
            }

            let should_record = if self.vox_config.is_some() {
                if frame_peak >= self.vox_threshold_linear {
                    // Audio is loud enough: trigger VOX
                    if self.vox_status == VoxStatus::Gated {
                        // Flushed pre-roll to avoid chopping the attack
                        self.flush_pre_roll()?;
                    }
                    self.vox_status = VoxStatus::Active;
                    self.vox_hold_remaining = self.vox_hold_frames;
                    true
                } else if self.vox_status == VoxStatus::Active || self.vox_status == VoxStatus::Hold {
                    if self.vox_hold_remaining > 0 {
                        self.vox_hold_remaining -= 1;
                        self.vox_status = VoxStatus::Hold;
                        true
                    } else {
                        // Hold duration expired: transition to Gated
                        self.vox_status = VoxStatus::Gated;
                        self.pre_roll.clear();
                        self.pre_roll.push_frame(frame);
                        false
                    }
                } else {
                    // Gated silence
                    self.pre_roll.push_frame(frame);
                    false
                }
            } else {
                true
            };

            if should_record {
                // Enforce explicit memory cap before appending
                let needed = ch;
                let new_len = self.samples.len().checked_add(needed).ok_or(SerenadeError::AllocationFailed)?;
                if new_len > self.max_samples {
                    return Err(SerenadeError::BufferFull {
                        current_samples: self.samples.len(),
                        max_samples: self.max_samples,
                    });
                }

                // Explicit try_reserve to guarantee no-OOM failure
                self.samples
                    .try_reserve(needed)
                    .map_err(|_| SerenadeError::AllocationFailed)?;

                self.samples.extend_from_slice(frame);
                self.waveform.feed_frame(frame);
                frames_recorded += 1;
            } else {
                frames_gated += 1;
            }
        }

        Ok(FeedStats {
            frames_recorded,
            frames_gated,
            vox_status: self.vox_status,
        })
    }

    fn flush_pre_roll(&mut self) -> Result<(), SerenadeError> {
        let count = self.pre_roll.count;
        if count == 0 {
            return Ok(());
        }

        let new_len = self.samples.len().checked_add(count).ok_or(SerenadeError::AllocationFailed)?;
        if new_len > self.max_samples {
            return Err(SerenadeError::BufferFull {
                current_samples: self.samples.len(),
                max_samples: self.max_samples,
            });
        }

        self.samples
            .try_reserve(count)
            .map_err(|_| SerenadeError::AllocationFailed)?;

        let mut drained = Vec::new();
        drained.try_reserve_exact(count).map_err(|_| SerenadeError::AllocationFailed)?;
        self.pre_roll.drain_into(&mut drained);

        // Feed drained frames to waveform mipmap and buffer
        self.waveform.feed_block(&drained);
        self.samples.extend_from_slice(&drained);

        Ok(())
    }
}
