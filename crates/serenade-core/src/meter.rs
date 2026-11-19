//! Real-time level metering: Sample-accurate Peak & RMS meters,
//! dBFS conversions, peak-hold decay, and clipping detection.

use core::f32;

/// Minimum linear threshold floor for dBFS calculation (-120 dBFS).
pub const MIN_DBFS_LINEAR: f32 = 0.000001; // 1e-6 -> -120 dBFS
/// Clamped floor for dBFS output.
pub const MIN_DBFS: f32 = -120.0;

/// Converts a linear amplitude (0.0 ..= 1.0+) to dBFS.
///
/// 1.0 corresponds to 0.0 dBFS. Values below 1e-6 or non-finite are clamped to -120.0 dBFS.
pub fn linear_to_dbfs(linear: f32) -> f32 {
    let abs_val = if linear.is_finite() {
        if linear < 0.0 { -linear } else { linear }
    } else {
        0.0
    };
    if abs_val <= MIN_DBFS_LINEAR {
        return MIN_DBFS;
    }
    20.0 * libm::log10f(abs_val)
}

/// Converts a dBFS level to linear amplitude.
///
/// 0.0 dBFS corresponds to 1.0. -6.02 dBFS corresponds to ~0.5.
pub fn dbfs_to_linear(dbfs: f32) -> f32 {
    if dbfs <= MIN_DBFS || !dbfs.is_finite() {
        return 0.0;
    }
    libm::powf(10.0, dbfs / 20.0)
}

/// Fast integer dBFS approximation in pure `no_std` without transcendental functions.
///
/// Uses IEEE-754 binary floating-point representation (`linear.to_bits()`)
/// to approximate $20 \log_{10}(\text{amplitude}) \approx 6.0206 \log_2(\text{amplitude})$.
///
/// Returns an integer dBFS value clamped between -120 and +24 dBFS.
pub fn linear_to_dbfs_approx(linear: f32) -> i16 {
    let abs_val = if linear < 0.0 { -linear } else { linear };
    if abs_val <= MIN_DBFS_LINEAR || !abs_val.is_finite() {
        return -120;
    }

    let bits = abs_val.to_bits();
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127;
    // Mantissa in [0.0, 1.0)
    let mantissa_bits = bits & 0x007F_FFFF;
    // Fast linear approximation of log2(1 + m) ~ m * (1.442695 - 0.442695 * m)
    // For integer precision, m itself is already within 0.08 dB of log2(1+m).
    let m = mantissa_bits as f32 / 8388608.0;
    let log2_val = exponent as f32 + m * (1.34 - 0.34 * m);

    // 20 * log10(2) = 6.0205999
    let db = (log2_val * 6.0206) as i32;

    if db < -120 {
        -120
    } else if db > 24 {
        24
    } else {
        db as i16
    }
}

/// Per-channel metering status snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelMeter {
    /// Instantaneous peak amplitude in linear scale (0.0..=1.0+).
    pub peak: f32,
    /// Instantaneous peak in dBFS.
    pub peak_dbfs: f32,
    /// Integer approximation of peak in dBFS.
    pub peak_dbfs_approx: i16,
    /// Held peak amplitude in linear scale with decay.
    pub held_peak: f32,
    /// Held peak in dBFS.
    pub held_peak_dbfs: f32,
    /// RMS amplitude in linear scale.
    pub rms: f32,
    /// RMS level in dBFS.
    pub rms_dbfs: f32,
    /// Cumulative count of samples that clipped (>= 0.9999 or <= -0.9999).
    pub clip_count: u64,
}

impl Default for ChannelMeter {
    fn default() -> Self {
        Self {
            peak: 0.0,
            peak_dbfs: MIN_DBFS,
            peak_dbfs_approx: -120,
            held_peak: 0.0,
            held_peak_dbfs: MIN_DBFS,
            rms: 0.0,
            rms_dbfs: MIN_DBFS,
            clip_count: 0,
        }
    }
}

/// Sample-accurate stereo/mono Peak and RMS level meter with peak-hold decay
/// and clipping detection.
#[derive(Clone, Debug)]
pub struct LevelMeter {
    channels: u16,
    sample_rate: u32,
    peak_hold_frames: u32,
    decay_rate_per_frame: f32,

    // Per-channel state (up to 2 channels supported natively)
    current_peak: [f32; 2],
    held_peak: [f32; 2],
    hold_remaining_frames: [u32; 2],
    rms_sum_squares: [f64; 2],
    rms_sample_count: [usize; 2],
    last_rms: [f32; 2],
    clip_count: [u64; 2],
}

impl LevelMeter {
    /// Creates a new level meter.
    ///
    /// - `channels`: 1 (mono) or 2 (stereo).
    /// - `sample_rate`: e.g. 48000 Hz.
    /// - `hold_ms`: peak-hold duration in milliseconds before decay begins (e.g. 1500 ms).
    /// - `decay_db_per_sec`: decay rate in dB/sec once hold expires (e.g. 20.0 dB/s).
    pub fn new(channels: u16, sample_rate: u32, hold_ms: u32, decay_db_per_sec: f32) -> Self {
        let ch = if channels <= 1 { 1 } else { 2 };
        let sr = if sample_rate == 0 { 48000 } else { sample_rate };
        let hold_frames = (sr as u64 * hold_ms as u64 / 1000) as u32;

        let decay = if decay_db_per_sec.is_finite() && decay_db_per_sec >= 0.0 {
            decay_db_per_sec
        } else {
            20.0
        };
        let db_per_frame = decay / (sr as f32);
        let decay_rate_per_frame = libm::powf(10.0, -db_per_frame / 20.0);

        Self {
            channels: ch,
            sample_rate: sr,
            peak_hold_frames: hold_frames,
            decay_rate_per_frame,
            current_peak: [0.0; 2],
            held_peak: [0.0; 2],
            hold_remaining_frames: [0; 2],
            rms_sum_squares: [0.0; 2],
            rms_sample_count: [0; 2],
            last_rms: [0.0; 2],
            clip_count: [0; 2],
        }
    }

    /// Returns the sample rate configured for this meter.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Process a single frame of audio (1 sample for mono, 2 for stereo).
    pub fn process_frame(&mut self, frame: &[f32]) {
        let ch_count = self.channels as usize;
        let count = if frame.len() < ch_count { frame.len() } else { ch_count };

        for ch in 0..count {
            let sample = if frame[ch].is_finite() { frame[ch] } else { 0.0 };
            let abs_val = if sample < 0.0 { -sample } else { sample };

            // Clipping detection (standard threshold 0.9999 or 1.0)
            if abs_val >= 0.9999 {
                self.clip_count[ch] = self.clip_count[ch].saturating_add(1);
            }

            // Current peak
            if abs_val > self.current_peak[ch] {
                self.current_peak[ch] = abs_val;
            }

            // Peak hold & decay
            if abs_val >= self.held_peak[ch] {
                self.held_peak[ch] = abs_val;
                self.hold_remaining_frames[ch] = self.peak_hold_frames;
            } else if self.hold_remaining_frames[ch] > 0 {
                self.hold_remaining_frames[ch] -= 1;
            } else {
                self.held_peak[ch] *= self.decay_rate_per_frame;
                if self.held_peak[ch] < MIN_DBFS_LINEAR {
                    self.held_peak[ch] = 0.0;
                }
            }

            // RMS accumulation
            self.rms_sum_squares[ch] += (abs_val as f64) * (abs_val as f64);
            self.rms_sample_count[ch] += 1;
        }

        // For mono configured with 1 channel, copy channel 0 to channel 1 for consistent stereo readout
        if self.channels == 1 {
            self.current_peak[1] = self.current_peak[0];
            self.held_peak[1] = self.held_peak[0];
            self.hold_remaining_frames[1] = self.hold_remaining_frames[0];
            self.clip_count[1] = self.clip_count[0];
        }
    }

    /// Process a block of interleaved samples.
    pub fn process_block(&mut self, samples: &[f32]) {
        let ch = self.channels as usize;
        let mut i = 0;
        while i + ch <= samples.len() {
            self.process_frame(&samples[i..i + ch]);
            i += ch;
        }
    }

    /// Finalizes the RMS calculation for the current window and returns channel snapshots.
    pub fn snapshot(&mut self) -> [ChannelMeter; 2] {
        let mut result = [ChannelMeter::default(), ChannelMeter::default()];

        for ch in 0..2 {
            let rms = if self.rms_sample_count[ch] > 0 {
                let mean_sq = self.rms_sum_squares[ch] / (self.rms_sample_count[ch] as f64);
                let val = libm::sqrt(mean_sq) as f32;
                self.last_rms[ch] = val;
                // Reset RMS window accumulator
                self.rms_sum_squares[ch] = 0.0;
                self.rms_sample_count[ch] = 0;
                val
            } else {
                self.last_rms[ch]
            };

            let peak = self.current_peak[ch];
            let held = self.held_peak[ch];

            result[ch] = ChannelMeter {
                peak,
                peak_dbfs: linear_to_dbfs(peak),
                peak_dbfs_approx: linear_to_dbfs_approx(peak),
                held_peak: held,
                held_peak_dbfs: linear_to_dbfs(held),
                rms,
                rms_dbfs: linear_to_dbfs(rms),
                clip_count: self.clip_count[ch],
            };

            // Reset current instantaneous peak for the next block window
            self.current_peak[ch] = 0.0;
        }

        result
    }

    /// Resets the clipping counter.
    pub fn reset_clip_counts(&mut self) {
        self.clip_count = [0, 0];
    }

    /// Full meter reset.
    pub fn reset(&mut self) {
        self.current_peak = [0.0; 2];
        self.held_peak = [0.0; 2];
        self.hold_remaining_frames = [0; 2];
        self.rms_sum_squares = [0.0; 2];
        self.rms_sample_count = [0; 2];
        self.last_rms = [0.0; 2];
        self.clip_count = [0; 2];
    }
}
