// SPDX-License-Identifier: GPL-3.0-only
//! Animation frame pacing at 60 Hz.
//!
//! While a window animates, Display draws one frame per 1/60 s (16, 17, 17
//! ms, so three frames take exactly 50 ms) instead of as fast as its loop
//! spins, and sleeps in between. With no vblank interrupt from virtio-gpu,
//! the timer is the clock; a display engine that reports vblank can call
//! `presented` from its flip instead. Each paced frame's interval is kept
//! for the frame-time gate (`[display.frame.time]`).

/// Three frames per 50 ms = 60 Hz.
const STEP_X3: u64 = 50;
/// Intervals kept for percentiles.
const KEEP: usize = 128;

pub struct Pacer {
    /// Next frame due, in ms times three (exact 16.67 ms steps).
    next3: u64,
    last: u64,
    running: bool,
    intervals: [u16; KEEP],
    count: usize,
}

impl Pacer {
    pub const fn new() -> Self { Self { next3: 0, last: 0, running: false, intervals: [0; KEEP], count: 0 } }
    /// True when the next animation frame may be drawn at `now` (ms).
    pub fn due(&self, now: u64) -> bool { !self.running || now * 3 >= self.next3 }
    /// Ms until the next frame is due (0 when due or not animating).
    pub fn wait(&self, now: u64) -> u64 {
        if !self.running || now * 3 >= self.next3 { 0 } else { (self.next3 - now * 3 + 2) / 3 }
    }
    /// An animation frame was presented at `now`. Returns the interval since
    /// the previous one (None for the first frame of an animation).
    pub fn presented(&mut self, now: u64) -> Option<u64> {
        let interval = if self.running { Some(now.saturating_sub(self.last)) } else { None };
        // Stay on the 60 Hz grid; after a slow frame start again from now.
        self.next3 = if self.running && self.next3 + STEP_X3 > now * 3 { self.next3 + STEP_X3 } else { now * 3 + STEP_X3 };
        self.last = now;
        self.running = true;
        if let Some(i) = interval { self.intervals[self.count % KEEP] = i.min(u16::MAX as u64) as u16; self.count += 1; }
        interval
    }
    /// The animation ended: the next one starts a fresh grid.
    pub fn stop(&mut self) { self.running = false; }
    /// The `pct` percentile of kept intervals (ms), if any.
    pub fn percentile(&self, pct: usize) -> Option<u16> {
        let n = self.count.min(KEEP);
        if n == 0 { return None; }
        let mut v = self.intervals;
        let v = &mut v[..n];
        v.sort_unstable();
        Some(v[((n - 1) * pct.min(100)) / 100])
    }
}

/// Lightweight ring-buffered frame-time meter for telemetry and optional HUD.
const METER_KEEP: usize = 64;
/// Frames taking longer than a 60 Hz budget (17 ms) count as slow.
pub const SLOW_THRESHOLD_MS: u16 = 17;

pub struct FrameMeter {
    samples: [u16; METER_KEEP],
    count: usize,
    total_frames: u64,
    slow_frames: u64,
    last_ms: u16,
    max_ms: u16,
    hud: bool,
}

impl FrameMeter {
    pub const fn new() -> Self {
        Self { samples: [0; METER_KEEP], count: 0, total_frames: 0, slow_frames: 0, last_ms: 0, max_ms: 0, hud: false }
    }
    pub fn set_hud(&mut self, on: bool) -> bool {
        let changed = self.hud != on;
        self.hud = on;
        changed
    }
    pub fn hud_enabled(&self) -> bool { self.hud }
    pub fn record(&mut self, render_ms: u64) {
        let ms = render_ms.min(u16::MAX as u64) as u16;
        self.samples[self.count % METER_KEEP] = ms;
        self.count = self.count.wrapping_add(1);
        self.total_frames = self.total_frames.wrapping_add(1);
        if ms >= SLOW_THRESHOLD_MS { self.slow_frames = self.slow_frames.wrapping_add(1); }
        self.last_ms = ms;
        if ms > self.max_ms { self.max_ms = ms; }
    }
    pub fn last_ms(&self) -> u16 { self.last_ms }
    pub fn total_frames(&self) -> u64 { self.total_frames }
    pub fn slow_frames(&self) -> u64 { self.slow_frames }
    pub fn max_ms(&self) -> u16 { self.max_ms }
    pub fn percentile(&self, pct: usize) -> u16 {
        let n = self.count.min(METER_KEEP);
        if n == 0 { return 0; }
        let mut v = self.samples;
        let v = &mut v[..n];
        v.sort_unstable();
        v[((n - 1) * pct.min(100)) / 100]
    }
    /// Copies up to `out.len()` most-recent frame times in chronological order.
    pub fn recent(&self, out: &mut [u16]) -> usize {
        let n = self.count.min(METER_KEEP).min(out.len());
        let start = self.count.saturating_sub(n);
        for i in 0..n {
            out[i] = self.samples[(start + i) % METER_KEEP];
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paces_at_sixty_hertz() {
        let mut p = Pacer::new();
        assert!(p.due(1000) && p.wait(1000) == 0);
        assert_eq!(p.presented(1000), None);
        assert!(!p.due(1010));
        assert_eq!(p.wait(1010), 7);
        assert!(p.due(1017));
        let mut t = 1017;
        let mut frames = 0;
        while t < 1050 { if p.due(t) { p.presented(t); frames += 1; } t += 1; }
        assert_eq!(frames, 2, "three frames per 50 ms");
        assert!(p.percentile(99).unwrap() <= 17);
    }
    #[test]
    fn slow_frame_restarts_the_grid() {
        let mut p = Pacer::new();
        p.presented(0);
        assert_eq!(p.presented(100), Some(100));
        assert!(!p.due(110) && p.due(117));
        p.stop();
        assert!(p.due(118));
        assert_eq!(p.presented(118), None);
    }
    #[test]
    fn frame_meter_percentiles_and_slow_count() {
        let mut m = FrameMeter::new();
        assert!(!m.hud_enabled());
        assert!(m.set_hud(true));
        assert!(!m.set_hud(true));
        for ms in [4u64, 5, 6, 8, 10, 12, 14, 16, 18, 25] { m.record(ms); }
        assert_eq!(m.total_frames(), 10);
        assert_eq!(m.slow_frames(), 2);
        assert_eq!(m.last_ms(), 25);
        assert_eq!(m.max_ms(), 25);
        assert_eq!(m.percentile(50), 10);
        assert_eq!(m.percentile(95), 18);
        assert_eq!(m.percentile(99), 18);
        let mut buf = [0u16; 4];
        assert_eq!(m.recent(&mut buf), 4);
        assert_eq!(buf, [14, 16, 18, 25]);
    }
}
