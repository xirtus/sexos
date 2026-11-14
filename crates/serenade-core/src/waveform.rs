//! Progressive multi-resolution min/max waveform peak pyramid (`WaveformMipmap`).
//!
//! Enables constant O(1)-per-column UI timeline rendering at arbitrary zoom levels.

use alloc::vec;
use alloc::vec::Vec;

/// Number of mipmap pyramid levels.
pub const MIPMAP_LEVEL_COUNT: usize = 6;

/// Frame bucket size per mipmap level.
pub const LEVEL_SIZES: [usize; MIPMAP_LEVEL_COUNT] = [
    64,      // Level 0: 64 frames
    256,     // Level 1: 256 frames (4x L0)
    1024,    // Level 2: 1024 frames (4x L1)
    4096,    // Level 3: 4096 frames (4x L2)
    16384,   // Level 4: 16384 frames (4x L3)
    65536,   // Level 5: 65536 frames (4x L4)
];

/// Peak and RMS summary within a time bin for stereo or mono audio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveformBin {
    /// Minimum sample value on left/mono channel (-1.0..=1.0).
    pub min_left: f32,
    /// Maximum sample value on left/mono channel (-1.0..=1.0).
    pub max_left: f32,
    /// RMS energy on left/mono channel (0.0..=1.0).
    pub rms_left: f32,
    /// Minimum sample value on right channel.
    pub min_right: f32,
    /// Maximum sample value on right channel.
    pub max_right: f32,
    /// RMS energy on right channel.
    pub rms_right: f32,
}

impl Default for WaveformBin {
    fn default() -> Self {
        Self {
            min_left: 0.0,
            max_left: 0.0,
            rms_left: 0.0,
            min_right: 0.0,
            max_right: 0.0,
            rms_right: 0.0,
        }
    }
}

impl WaveformBin {
    /// Combines multiple bins into a single summary bin.
    pub fn combine(bins: &[Self]) -> Self {
        if bins.is_empty() {
            return Self::default();
        }

        let mut min_l = bins[0].min_left;
        let mut max_l = bins[0].max_left;
        let mut sum_sq_l = 0.0f64;

        let mut min_r = bins[0].min_right;
        let mut max_r = bins[0].max_right;
        let mut sum_sq_r = 0.0f64;

        for b in bins {
            if b.min_left < min_l {
                min_l = b.min_left;
            }
            if b.max_left > max_l {
                max_l = b.max_left;
            }
            sum_sq_l += (b.rms_left as f64) * (b.rms_left as f64);

            if b.min_right < min_r {
                min_r = b.min_right;
            }
            if b.max_right > max_r {
                max_r = b.max_right;
            }
            sum_sq_r += (b.rms_right as f64) * (b.rms_right as f64);
        }

        let len = bins.len() as f64;
        let rms_l = libm::sqrt(sum_sq_l / len) as f32;
        let rms_r = libm::sqrt(sum_sq_r / len) as f32;

        Self {
            min_left: min_l,
            max_left: max_l,
            rms_left: rms_l,
            min_right: min_r,
            max_right: max_r,
            rms_right: rms_r,
        }
    }
}

/// Accumulator for progressive frame feeding into Level 0.
#[derive(Clone, Debug)]
struct BinAccumulator {
    count: usize,
    min_left: f32,
    max_left: f32,
    sum_sq_left: f64,
    min_right: f32,
    max_right: f32,
    sum_sq_right: f64,
}

impl BinAccumulator {
    fn new() -> Self {
        Self {
            count: 0,
            min_left: 0.0,
            max_left: 0.0,
            sum_sq_left: 0.0,
            min_right: 0.0,
            max_right: 0.0,
            sum_sq_right: 0.0,
        }
    }

    fn push(&mut self, left: f32, right: f32) {
        if self.count == 0 {
            self.min_left = left;
            self.max_left = left;
            self.min_right = right;
            self.max_right = right;
        } else {
            if left < self.min_left {
                self.min_left = left;
            }
            if left > self.max_left {
                self.max_left = left;
            }
            if right < self.min_right {
                self.min_right = right;
            }
            if right > self.max_right {
                self.max_right = right;
            }
        }
        self.sum_sq_left += (left as f64) * (left as f64);
        self.sum_sq_right += (right as f64) * (right as f64);
        self.count += 1;
    }

    fn to_bin(&self) -> WaveformBin {
        if self.count == 0 {
            return WaveformBin::default();
        }
        let len = self.count as f64;
        let rms_l = libm::sqrt(self.sum_sq_left / len) as f32;
        let rms_r = libm::sqrt(self.sum_sq_right / len) as f32;

        WaveformBin {
            min_left: self.min_left,
            max_left: self.max_left,
            rms_left: rms_l,
            min_right: self.min_right,
            max_right: self.max_right,
            rms_right: rms_r,
        }
    }

    fn reset(&mut self) {
        self.count = 0;
        self.min_left = 0.0;
        self.max_left = 0.0;
        self.sum_sq_left = 0.0;
        self.min_right = 0.0;
        self.max_right = 0.0;
        self.sum_sq_right = 0.0;
    }
}

/// Progressive multi-resolution waveform peak pyramid.
#[derive(Clone, Debug)]
pub struct WaveformMipmap {
    channels: u16,
    total_frames: usize,
    levels: [Vec<WaveformBin>; MIPMAP_LEVEL_COUNT],
    accum: BinAccumulator,
}

impl WaveformMipmap {
    /// Creates a new empty waveform mipmap.
    pub fn new(channels: u16) -> Self {
        Self {
            channels: if channels <= 1 { 1 } else { 2 },
            total_frames: 0,
            levels: [
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            accum: BinAccumulator::new(),
        }
    }

    /// Feeds a single frame (mono or stereo).
    pub fn feed_frame(&mut self, frame: &[f32]) {
        let left = if !frame.is_empty() && frame[0].is_finite() {
            frame[0]
        } else {
            0.0
        };
        let right = if self.channels > 1 && frame.len() > 1 && frame[1].is_finite() {
            frame[1]
        } else {
            left
        };

        self.accum.push(left, right);
        self.total_frames += 1;

        if self.accum.count >= LEVEL_SIZES[0] {
            let bin = self.accum.to_bin();
            self.accum.reset();
            self.push_level0_bin(bin);
        }
    }

    /// Feeds an interleaved block of frames.
    pub fn feed_block(&mut self, samples: &[f32]) {
        let ch = self.channels as usize;
        let mut i = 0;
        while i + ch <= samples.len() {
            self.feed_frame(&samples[i..i + ch]);
            i += ch;
        }
    }

    fn push_level0_bin(&mut self, bin: WaveformBin) {
        self.levels[0].push(bin);

        // Propagate upstream across pyramid levels
        let mut curr_lvl = 0;
        while curr_lvl + 1 < MIPMAP_LEVEL_COUNT {
            let len = self.levels[curr_lvl].len();
            if len % 4 != 0 {
                break;
            }
            let start = len - 4;
            let group = &self.levels[curr_lvl][start..len];
            let higher_bin = WaveformBin::combine(group);
            self.levels[curr_lvl + 1].push(higher_bin);
            curr_lvl += 1;
        }
    }

    /// Total frames fed so far.
    pub fn total_frames(&self) -> usize {
        self.total_frames
    }

    /// Number of bins stored in a specific mipmap level.
    pub fn level_len(&self, level: usize) -> usize {
        if level < MIPMAP_LEVEL_COUNT {
            self.levels[level].len()
        } else {
            0
        }
    }

    /// Queries a range `[start_frame, end_frame)` and returns `target_columns`
    /// display bins for direct timeline rendering in O(target_columns) time.
    pub fn query_range(
        &self,
        start_frame: usize,
        end_frame: usize,
        target_columns: usize,
    ) -> Vec<WaveformBin> {
        if target_columns == 0 || start_frame >= end_frame || self.total_frames == 0 {
            return Vec::new();
        }

        let start = start_frame.min(self.total_frames);
        let end = end_frame.min(self.total_frames);
        let span = end - start;
        if span == 0 {
            return vec![WaveformBin::default(); target_columns];
        }

        let frames_per_col = span as f32 / target_columns as f32;

        // Choose the highest level where level bucket size <= frames_per_col
        let mut chosen_level = 0;
        for (lvl, &size) in LEVEL_SIZES.iter().enumerate() {
            if size as f32 <= frames_per_col {
                chosen_level = lvl;
            } else {
                break;
            }
        }

        let bucket_size = LEVEL_SIZES[chosen_level];
        let bins = &self.levels[chosen_level];
        let mut result = Vec::new();
        if result.try_reserve_exact(target_columns).is_err() {
            return Vec::new();
        }

        for col in 0..target_columns {
            let col_start = start + (col * span) / target_columns;
            let col_end = (start + ((col + 1) * span) / target_columns).max(col_start + 1);

            let bin_start_idx = col_start / bucket_size;
            let bin_end_idx = ((col_end + bucket_size - 1) / bucket_size).max(bin_start_idx + 1);

            if bin_start_idx < bins.len() {
                let actual_end = bin_end_idx.min(bins.len());
                if actual_end > bin_start_idx {
                    result.push(WaveformBin::combine(&bins[bin_start_idx..actual_end]));
                } else {
                    result.push(bins[bin_start_idx]);
                }
            } else if self.accum.count > 0 {
                // Audio is still actively being accumulated in the tail
                result.push(self.accum.to_bin());
            } else if !bins.is_empty() {
                result.push(*bins.last().unwrap());
            } else {
                result.push(WaveformBin::default());
            }
        }

        result
    }
}
