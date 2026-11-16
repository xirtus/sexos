// SPDX-License-Identifier: GPL-3.0-only
//! Fixed storage decouples queued render samples from actually consumed meters.
//! Allocated before transport starts; writing and advancing never allocate.
use alloc::{vec, vec::Vec};
use moan_core::Frame;
pub struct History {
    tracks: [Vec<Frame>; crate::MAX_TRACKS + 1],
    capacity: usize,
    consumed: usize,
}
impl History {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            tracks: core::array::from_fn(|_| vec![[0; 2]; capacity]),
            capacity,
            consumed: 0,
        }
    }
    pub fn reset(&mut self, start: usize) {
        self.consumed = start;
        for track in &mut self.tracks {
            track.fill([0; 2]);
        }
    }
    pub fn write(&mut self, track: usize, frame: usize, sample: Frame) {
        self.tracks[track][frame % self.capacity] = sample;
    }
    /// Only samples both written and reported consumed by Bell may affect meters.
    /// On an overrun, reset stale meter state and retain only available history.
    pub fn advance(
        &mut self,
        heard: usize,
        produced: usize,
        meters: &mut [serenade_core::LevelMeter; crate::MAX_TRACKS],
    ) -> usize {
        self.advance_inner(heard, produced, meters, None)
    }
    pub fn advance_master(
        &mut self,
        heard: usize,
        produced: usize,
        meters: &mut [serenade_core::LevelMeter; crate::MAX_TRACKS],
        master: &mut serenade_core::LevelMeter,
    ) -> usize {
        self.advance_inner(heard, produced, meters, Some(master))
    }
    fn advance_inner(
        &mut self,
        heard: usize,
        produced: usize,
        meters: &mut [serenade_core::LevelMeter; crate::MAX_TRACKS],
        mut master: Option<&mut serenade_core::LevelMeter>,
    ) -> usize {
        let end = heard.min(produced);
        if end < self.consumed {
            return 0;
        }
        let earliest = produced.saturating_sub(self.capacity);
        let lost = earliest.saturating_sub(self.consumed);
        if lost > 0 {
            self.consumed = earliest;
            for meter in meters.iter_mut() {
                meter.reset();
            }
            if let Some(master) = master.as_deref_mut() {
                master.reset();
            }
        }
        for at in self.consumed..end {
            if let Some(master) = master.as_deref_mut() {
                let frame = self.tracks[crate::MAX_TRACKS][at % self.capacity];
                master.process_frame(&[frame[0] as f32 / 32768.0, frame[1] as f32 / 32768.0]);
            }
            for (track, meter) in self.tracks.iter().zip(meters.iter_mut()) {
                let frame = track[at % self.capacity];
                meter.process_frame(&[frame[0] as f32 / 32768.0, frame[1] as f32 / 32768.0]);
            }
        }
        self.consumed = end;
        lost
    }
}
/// Snapshot once, never chase a concurrently draining device indefinitely.
pub fn fill_budget(room_snapshot: usize, remaining: usize, block_frames: usize) -> usize {
    room_snapshot
        .min(remaining)
        .min(block_frames.saturating_mul(64))
}
