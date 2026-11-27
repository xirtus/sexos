// SPDX-License-Identifier: GPL-3.0-only
//! What the timeline draws on its clips: small pictures along a source (the filmstrip)
//! and the loudness of its sound (the waveform). The program fills a [`Library`] from
//! the decoder; tests fill it with made-up pictures and sound.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use sex_splice::Micros;

/// One small picture of a source at a source time.
#[derive(Clone, Debug)]
pub struct Thumb {
    pub at: Micros,
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

/// The loudest sample in each bucket of `per` microseconds, 0 to 255.
#[derive(Clone, Debug, Default)]
pub struct Peaks {
    pub per: Micros,
    pub v: Vec<u8>,
}

impl Peaks {
    /// Peaks of stereo samples at `rate`, one per `per` microseconds.
    pub fn from_pcm(pcm: &[[i16; 2]], rate: u32, per: Micros) -> Peaks {
        let mut p = Peaks { per, v: Vec::new() };
        p.add(0, pcm, rate);
        p
    }

    /// Add samples starting at source time `at`.
    pub fn add(&mut self, at: Micros, pcm: &[[i16; 2]], rate: u32) {
        if self.per <= 0 || rate == 0 {
            return;
        }
        for (i, s) in pcm.iter().enumerate() {
            let t = at + (i as i128 * 1_000_000 / rate as i128) as Micros;
            if t < 0 {
                continue;
            }
            let b = (t / self.per) as usize;
            if b >= self.v.len() {
                self.v.resize(b + 1, 0);
            }
            let loud = (s[0].unsigned_abs().max(s[1].unsigned_abs()) >> 7).min(255) as u8;
            self.v[b] = self.v[b].max(loud);
        }
    }

    /// The loudest bucket in source times `from..to`, 0 to 255.
    pub fn over(&self, from: Micros, to: Micros) -> u8 {
        if self.per <= 0 || self.v.is_empty() {
            return 0;
        }
        let a = (from.max(0) / self.per) as usize;
        let b = ((to.max(0) + self.per - 1) / self.per) as usize;
        self.v.get(a..b.min(self.v.len()).max(a)).map_or(0, |s| s.iter().copied().max().unwrap_or(0))
    }
}

/// Max 48 kHz stereo PCM frames cached in RAM per clip (~5 minutes = 57.6 MiB).
pub const MAX_PCM_FRAMES: usize = 48_000 * 300;

/// Filmstrips, waveforms, and decoded 48 kHz stereo PCM by media id.
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub thumbs: BTreeMap<u32, Vec<Thumb>>,
    pub peaks: BTreeMap<u32, Peaks>,
    /// RMS loudness for placing cut edges between words (20 ms buckets).
    pub levels: BTreeMap<u32, sex_splice::edges::Levels>,
    /// Sample-exact 48 kHz stereo PCM scanned on open for zero-seek real-time playback.
    pub pcm: BTreeMap<u32, Vec<[i16; 2]>>,
}

impl Library {
    /// Append decoded stereo PCM starting at `at_us` (`rate` Hz, normally 48_000).
    pub fn append_pcm(&mut self, media: u32, at_us: Micros, samples: &[[i16; 2]], rate: u32) {
        if samples.is_empty() || rate == 0 {
            return;
        }
        let buf = self.pcm.entry(media).or_default();
        if buf.len() >= MAX_PCM_FRAMES {
            return;
        }
        let start_idx = ((at_us.max(0) as u64 * rate as u64) / 1_000_000) as usize;
        if start_idx > MAX_PCM_FRAMES {
            return;
        }
        if start_idx > buf.len() {
            let pad_to = start_idx.min(MAX_PCM_FRAMES);
            if buf.try_reserve(pad_to - buf.len()).is_err() {
                return;
            }
            buf.resize(pad_to, [0, 0]);
        }
        let end_idx = (start_idx + samples.len()).min(MAX_PCM_FRAMES);
        let copy_n = end_idx.saturating_sub(start_idx);
        if copy_n == 0 {
            return;
        }
        if end_idx > buf.len() {
            let need = end_idx - buf.len();
            if buf.try_reserve(need).is_err() {
                return;
            }
            let overlap = buf.len().saturating_sub(start_idx);
            if overlap > 0 {
                buf[start_idx..start_idx + overlap].copy_from_slice(&samples[..overlap]);
            }
            buf.extend_from_slice(&samples[overlap..copy_n]);
        } else {
            buf[start_idx..start_idx + copy_n].copy_from_slice(&samples[..copy_n]);
        }
    }
    /// Keep a picture, in time order.
    pub fn add_thumb(&mut self, media: u32, t: Thumb) {
        let list = self.thumbs.entry(media).or_default();
        let i = list.partition_point(|x| x.at < t.at);
        list.insert(i, t);
    }

    /// The picture taken nearest to source time `at`.
    pub fn thumb(&self, media: u32, at: Micros) -> Option<&Thumb> {
        let list = self.thumbs.get(&media)?;
        let i = list.partition_point(|x| x.at < at);
        match (i.checked_sub(1).and_then(|j| list.get(j)), list.get(i)) {
            (Some(a), Some(b)) => Some(if at - a.at <= b.at - at { a } else { b }),
            (a, b) => a.or(b),
        }
    }
}
