//! Pure-Rust ports of **obs-time-warp-scan** and **dynamic-delay** for Cabaret.
//!
//! - [`TimeWarpScan`]: horizontal, vertical, and radial sweeping slit-scan effect freezing
//!   pixels over time with a glowing scan-bar preview.
//! - [`DynamicDelayBuffer`]: ring-buffered live slow-motion, reverse playback, instant replay,
//!   and frame-accurate scrubbing on a live video source.

use alloc::collections::VecDeque;
use cabaret_core::compositor::{blend_pixel, BlendMode, FrameBuffer, LinearRgba, MixColorSpace};

use crate::util::{absf, clamp01, floorf, lerp, roundf, smoothstep, sqrtf};

/// Sweep geometry for [`TimeWarpScan`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScanDirection {
    #[default]
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
    RadialOutward,
    RadialInward,
}

/// Configuration for [`TimeWarpScan`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeWarpConfig {
    pub direction: ScanDirection,
    pub duration_ms: f32,
    pub show_scan_bar: bool,
    pub scan_bar_thickness_px: f32,
    pub scan_bar_color: LinearRgba,
    pub loop_scan: bool,
}

impl Default for TimeWarpConfig {
    fn default() -> Self {
        Self {
            direction: ScanDirection::LeftToRight,
            duration_ms: 3000.0,
            show_scan_bar: true,
            scan_bar_thickness_px: 4.0,
            scan_bar_color: [0.1, 0.95, 1.0, 0.9],
            loop_scan: false,
        }
    }
}

/// Sweeping slit-scan filter freezing pixels progressively across time (`obs-time-warp-scan`).
#[derive(Clone, Debug)]
pub struct TimeWarpScan {
    pub config: TimeWarpConfig,
    pub progress: f32,
    pub finished: bool,
    frozen_canvas: Option<FrameBuffer>,
}

impl TimeWarpScan {
    pub fn new(config: TimeWarpConfig) -> Self {
        Self {
            config,
            progress: 0.0,
            finished: false,
            frozen_canvas: None,
        }
    }

    pub fn reset(&mut self) {
        self.progress = 0.0;
        self.finished = false;
        self.frozen_canvas = None;
    }

    /// Processes one live frame, advancing the scan sweep by `dt_ms`.
    pub fn process(&mut self, live_frame: &FrameBuffer, dt_ms: f32) -> FrameBuffer {
        let w = live_frame.width;
        let h = live_frame.height;

        let reinit = match &self.frozen_canvas {
            Some(fc) => fc.width != w || fc.height != h,
            None => true,
        };
        if reinit {
            self.frozen_canvas = Some(live_frame.clone());
        }

        let Some(ref mut frozen) = self.frozen_canvas else {
            return live_frame.clone();
        };

        if self.finished {
            return frozen.clone();
        }

        let dur = self.config.duration_ms.max(1.0);
        let prev_p = self.progress;
        let next_p = clamp01(prev_p + dt_ms.max(0.0) / dur);

        let wf = w as f32;
        let hf = h as f32;
        let span_px = match self.config.direction {
            ScanDirection::LeftToRight | ScanDirection::RightToLeft => wf,
            ScanDirection::TopToBottom | ScanDirection::BottomToTop => hf,
            ScanDirection::RadialOutward | ScanDirection::RadialInward => {
                0.5 * sqrtf(wf * wf + hf * hf)
            }
        };

        let mut out = live_frame.clone();
        for y in 0..h {
            let v = (y as f32 + 0.5) / hf;
            for x in 0..w {
                let u = (x as f32 + 0.5) / wf;
                let s = scan_coord(self.config.direction, u, v);

                let mut px = if s <= next_p {
                    let newly_swept = s > prev_p || (prev_p <= 0.0 && next_p > 0.0 && s <= next_p);
                    if newly_swept {
                        let live_px = live_frame.get_pixel(x, y).unwrap_or([0.0; 4]);
                        frozen.set_pixel(x, y, live_px);
                        live_px
                    } else {
                        frozen.get_pixel(x, y).unwrap_or([0.0; 4])
                    }
                } else {
                    live_frame.get_pixel(x, y).unwrap_or([0.0; 4])
                };

                if self.config.show_scan_bar && next_p > 0.0 && next_p < 1.0 {
                    let dist_px = absf(s - next_p) * span_px;
                    let half_t = (self.config.scan_bar_thickness_px * 0.5).max(0.5);
                    let glow = smoothstep(half_t + 1.0, 0.0, dist_px);
                    if glow > 1e-4 {
                        let mut bar = self.config.scan_bar_color;
                        bar[3] = clamp01(bar[3] * glow);
                        px = blend_pixel(px, bar, BlendMode::Normal, 1.0, MixColorSpace::Linear);
                    }
                }

                out.set_pixel(x, y, px);
            }
        }

        self.progress = next_p;
        if self.progress >= 1.0 {
            if self.config.loop_scan {
                self.progress = 0.0;
            } else {
                self.finished = true;
            }
        }
        out
    }
}

#[inline]
fn scan_coord(dir: ScanDirection, u: f32, v: f32) -> f32 {
    match dir {
        ScanDirection::LeftToRight => u,
        ScanDirection::RightToLeft => 1.0 - u,
        ScanDirection::TopToBottom => v,
        ScanDirection::BottomToTop => 1.0 - v,
        ScanDirection::RadialOutward => {
            let dx = u - 0.5;
            let dy = v - 0.5;
            clamp01(sqrtf(dx * dx + dy * dy) / 0.707_106_77)
        }
        ScanDirection::RadialInward => {
            let dx = u - 0.5;
            let dy = v - 0.5;
            1.0 - clamp01(sqrtf(dx * dx + dy * dy) / 0.707_106_77)
        }
    }
}

/// Timestamped video frame entry stored inside [`DynamicDelayBuffer`].
#[derive(Clone, Debug)]
pub struct DelayedFrame {
    pub seq: u64,
    pub timestamp_us: i64,
    pub frame: FrameBuffer,
}

/// Ring-buffered live slow-motion, reverse playback, and instant scrubbing (`dynamic-delay`).
#[derive(Clone, Debug)]
pub struct DynamicDelayBuffer {
    pub capacity_frames: usize,
    pub speed: f32,
    pub live_follow: bool,
    pub interpolate_frames: bool,
    ring: VecDeque<DelayedFrame>,
    next_seq: u64,
    playhead: f64,
}

impl DynamicDelayBuffer {
    pub fn new(capacity_frames: usize) -> Self {
        Self {
            capacity_frames: capacity_frames.max(2),
            speed: 1.0,
            live_follow: true,
            interpolate_frames: false,
            ring: VecDeque::with_capacity(capacity_frames.max(2)),
            next_seq: 0,
            playhead: 0.0,
        }
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    pub fn oldest_seq(&self) -> Option<u64> {
        self.ring.front().map(|f| f.seq)
    }

    pub fn newest_seq(&self) -> Option<u64> {
        self.ring.back().map(|f| f.seq)
    }

    pub fn current_playhead(&self) -> f64 {
        self.playhead
    }

    /// Pushes a live incoming frame into the circular delay ring.
    pub fn push_frame(&mut self, timestamp_us: i64, frame: FrameBuffer) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.ring.push_back(DelayedFrame {
            seq,
            timestamp_us,
            frame,
        });
        while self.ring.len() > self.capacity_frames {
            self.ring.pop_front();
        }
        if self.live_follow || self.ring.len() == 1 {
            self.playhead = seq as f64;
        } else if let Some(oldest) = self.oldest_seq() {
            if self.playhead < oldest as f64 {
                self.playhead = oldest as f64;
            }
        }
    }

    /// Sets playback speed (`1.0` normal, `0.5` half-speed slow-mo, `-1.0` reverse, `0.0` pause).
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(-8.0, 8.0);
        self.live_follow = false;
    }

    /// Snaps playhead back to the live edge (`speed = 1.0`, `live_follow = true`).
    pub fn go_live(&mut self) {
        self.speed = 1.0;
        self.live_follow = true;
        if let Some(newest) = self.newest_seq() {
            self.playhead = newest as f64;
        }
    }

    /// Rewinds by `rewind_frames` from the newest buffered frame and plays at `speed`.
    pub fn trigger_instant_replay(&mut self, rewind_frames: usize, speed: f32) {
        self.live_follow = false;
        self.speed = speed.clamp(-8.0, 8.0);
        if let (Some(oldest), Some(newest)) = (self.oldest_seq(), self.newest_seq()) {
            let target = newest.saturating_sub(rewind_frames as u64).max(oldest);
            self.playhead = target as f64;
        }
    }

    /// Scrubs playhead to normalized ring position `norm in [0.0 (oldest) .. 1.0 (newest)]`.
    pub fn scrub_to_normalized(&mut self, norm: f32) {
        self.live_follow = false;
        if let (Some(oldest), Some(newest)) = (self.oldest_seq(), self.newest_seq()) {
            let n = clamp01(norm) as f64;
            self.playhead = oldest as f64 + (newest - oldest) as f64 * n;
        }
    }

    /// Scrubs playhead by `delta_frames` relative to current position and pauses follow.
    pub fn scrub_frames(&mut self, delta_frames: f32) {
        self.live_follow = false;
        if let (Some(oldest), Some(newest)) = (self.oldest_seq(), self.newest_seq()) {
            self.playhead = (self.playhead + delta_frames as f64).clamp(oldest as f64, newest as f64);
        }
    }

    /// Advances the delay playhead by `dt_frames * speed` and samples the resulting [`FrameBuffer`].
    pub fn step_and_sample(&mut self, dt_frames: f32) -> Option<FrameBuffer> {
        let (oldest, newest) = (self.oldest_seq()?, self.newest_seq()?);
        if self.live_follow {
            self.playhead = newest as f64;
            return self.ring.back().map(|d| d.frame.clone());
        }

        self.playhead =
            (self.playhead + (dt_frames * self.speed) as f64).clamp(oldest as f64, newest as f64);

        if !self.interpolate_frames {
            let target_seq = roundf(self.playhead as f32) as u64;
            let idx = target_seq.saturating_sub(oldest) as usize;
            return self
                .ring
                .get(idx.min(self.ring.len() - 1))
                .map(|d| d.frame.clone());
        }

        let f0 = floorf(self.playhead as f32) as u64;
        let frac = (self.playhead - f0 as f64) as f32;
        let idx0 = f0.saturating_sub(oldest) as usize;
        let idx1 = (idx0 + 1).min(self.ring.len() - 1);
        let a = &self.ring[idx0].frame;
        let b = &self.ring[idx1].frame;
        if frac <= 1e-4 || a.width != b.width || a.height != b.height {
            return Some(a.clone());
        }
        let mut out = a.clone();
        for (dst, (&pa, &pb)) in out.pixels.iter_mut().zip(a.pixels.iter().zip(b.pixels.iter())) {
            *dst = [
                lerp(pa[0], pb[0], frac),
                lerp(pa[1], pb[1], frac),
                lerp(pa[2], pb[2], frac),
                lerp(pa[3], pb[3], frac),
            ];
        }
        Some(out)
    }
}
