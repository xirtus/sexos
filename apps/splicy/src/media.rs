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

/// Filmstrips and waveforms by media id.
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub thumbs: BTreeMap<u32, Vec<Thumb>>,
    pub peaks: BTreeMap<u32, Peaks>,
}

impl Library {
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
