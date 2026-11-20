//! Cut edges that land between words, not inside them.
//!
//! Adapted from Scarlet's dialogue join checks (`dialogue-join.ts`, MIT,
//! github.com/Forward-Future/scarlet). A transcript's word times come from speech
//! recognition and are loose by tens of milliseconds. Where there is a real pause between
//! two words that does not matter: a cut anywhere in the pause sounds clean. Where two
//! words run together in one breath, a cut placed on the transcript's boundary often
//! clips the end of one word or the first sound of the next, and the editor hears a
//! leftover "s" or a changed word. Scarlet learnt on real passes to put such an edge on the
//! waveform: the stretch at room tone between the words, or failing that a trough well
//! under the sound either side. This module does the same, on integer microseconds and
//! integer tenths of a dB so every run places the same edge.
//!
//! The levels are RMS loudness in fixed buckets ([`Levels`], 20 ms in Scarlet), measured
//! by the program from the decoded sound.

use crate::transcript::{SourceCut, Transcript, Word};
use crate::{Micros, Range};
use alloc::vec::Vec;

/// A level this low (tenths of dBFS) between two words is room tone: the instant between
/// two words said in one breath. It is far under silence's own bar because a sibilant's
/// hiss sits just below that; an editor heard a leftover "s" at a cut placed at -47 dBFS.
pub const DIP_DECIBELS: i16 = -550;
/// A trough above room tone can still take an edge when it is under -48 dBFS and at least
/// 10 dB under the sound either side, as it shows on a waveform.
pub const TROUGH_DECIBELS: i16 = -480;
pub const TROUGH_DEPTH: i16 = 100;
/// A shallower low point, as deep under its neighbours, takes an edge within 50 ms of the
/// transcript's own boundary, never further off: the stop before a final "s" reads the same.
pub const LOW_POINT_DECIBELS: i16 = -440;
pub const LOW_POINT_REACH: Micros = 50_000;
/// How far either side of a trough the sound it lies under is looked for.
pub const TROUGH_REACH: Micros = 80_000;
/// Two words timed this close are said as one, and their edge goes where the sound parts them.
pub const RUN_ON_GAP: Micros = 60_000;
/// Below this a word's loudest moment cannot locate it inside its timing (silence's bar, -40 dBFS).
pub const QUIET_DECIBELS: i16 = -400;
/// The floor [`Levels`] reports for digital silence.
pub const SILENT_DECIBELS: i16 = -1200;

/// Loudness of a source in buckets of `step`, the first starting at `from`. Each entry is
/// RMS level in tenths of a dB under full scale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Levels {
    pub from: Micros,
    pub step: Micros,
    pub db: Vec<i16>,
}

/// log2(x) in 16.16 fixed point, for x >= 1.
fn log2_q16(x: u64) -> i64 {
    let int = 63 - x.leading_zeros() as i64;
    // Mantissa in [1, 2) as 1.31 fixed point; each squaring yields one fraction bit.
    let mut m: u64 = if int >= 31 { x >> (int - 31) } else { x << (31 - int) };
    let mut frac = 0i64;
    for bit in (0..16).rev() {
        m = (m * m) >> 31;
        if m >= 1 << 32 {
            m >>= 1;
            frac |= 1 << bit;
        }
    }
    (int << 16) | frac
}

/// Mean square of 16-bit samples as tenths of a dB under full scale.
pub fn decibels(mean_square: u64) -> i16 {
    if mean_square == 0 {
        return SILENT_DECIBELS;
    }
    // Full scale is 32768², 2^30. 10·log10(2) dB per doubling = 30.103 tenths per bit.
    let tenths = (log2_q16(mean_square) - (30 << 16)) * 30_103 / (1000 << 16);
    tenths.clamp(SILENT_DECIBELS as i64, 0) as i16
}

impl Levels {
    /// Levels of stereo samples at `rate` starting at source time `from`, in buckets of `step`.
    pub fn from_pcm(from: Micros, pcm: &[[i16; 2]], rate: u32, step: Micros) -> Levels {
        let mut db = Vec::new();
        if rate > 0 && step > 0 {
            let per = ((step as i128 * rate as i128) / 1_000_000).max(1) as usize;
            for chunk in pcm.chunks(per) {
                let sum: u64 = chunk.iter().map(|s| ((s[0] as i64).pow(2) + (s[1] as i64).pow(2)) as u64).sum();
                db.push(decibels(sum / (2 * chunk.len() as u64)));
            }
        }
        Levels { from, step, db }
    }

    fn centre(&self, i: usize) -> Micros {
        self.from + i as Micros * self.step + self.step / 2
    }

    /// How far bucket `i` lies under the loudest level within [`TROUGH_REACH`] on each side:
    /// the shallower of the two.
    pub fn trough_depth(&self, i: usize) -> i16 {
        let k = ((TROUGH_REACH + self.step / 2) / self.step).max(1) as usize;
        let left = self.db[i.saturating_sub(k)..i].iter().copied().max();
        let right = self.db.get(i + 1..(i + k + 1).min(self.db.len())).and_then(|s| s.iter().copied().max());
        match (left, right) {
            (Some(l), Some(r)) => l.min(r).saturating_sub(self.db[i]),
            _ => i16::MIN,
        }
    }

    /// A level an edge can sit at between words: room tone, or a trough deep under the
    /// sound either side.
    pub fn is_dip(&self, i: usize, bar: i16) -> bool {
        let db = self.db[i];
        db <= DIP_DECIBELS || (db <= bar && self.trough_depth(i) >= TROUGH_DEPTH)
    }

    /// The lowest (or loudest) bucket whose centre lies in `from..=to`.
    fn extreme(&self, from: Micros, to: Micros, lowest: bool) -> Option<usize> {
        let mut best: Option<usize> = None;
        for i in 0..self.db.len() {
            let at = self.centre(i);
            if at < from {
                continue;
            }
            if at > to {
                break;
            }
            let better = match best {
                None => true,
                Some(b) if lowest => self.db[i] < self.db[b],
                Some(b) => self.db[i] > self.db[b],
            };
            if better {
                best = Some(i);
            }
        }
        best
    }

    /// Where an edge sits between two words that run together, searching `from..=to`:
    /// the middle of the room-tone stretch around the lowest level, or a trough's floor.
    /// `boundary` is the transcript's own edge; a shallower low point within
    /// [`LOW_POINT_REACH`] of it counts. `None` when the stretch holds no dip.
    pub fn dip_between(&self, from: Micros, to: Micros, boundary: Option<Micros>) -> Option<Dip> {
        let at = self.extreme(from, to, true)?;
        let low = self.db[at];
        let room_tone = low <= DIP_DECIBELS;
        // A trough above room tone is a dip only when its floor is inside the stretch: the
        // lowest level at the stretch's end is the wall of a trough beyond it, still in sound.
        let inside = at > 0
            && at + 1 < self.db.len()
            && self.centre(at - 1) >= from
            && self.centre(at + 1) <= to
            && self.db[at - 1] >= low
            && self.db[at + 1] >= low;
        let near = boundary.is_some_and(|b| (self.centre(at) - b).abs() <= LOW_POINT_REACH);
        let bar = if near { LOW_POINT_DECIBELS } else { TROUGH_DECIBELS };
        if !room_tone && !(inside && self.is_dip(at, bar)) {
            return None;
        }
        // The stretch still down in it: at room tone, or within 6 dB of the trough's floor.
        let floor = if room_tone { DIP_DECIBELS } else { low.saturating_add(60) };
        let (mut first, mut last) = (at, at);
        while first > 0 && self.centre(first - 1) >= from && self.db[first - 1] <= floor {
            first -= 1;
        }
        while last + 1 < self.db.len() && self.centre(last + 1) <= to && self.db[last + 1] <= floor {
            last += 1;
        }
        let room = Range::new(self.from + first as Micros * self.step, self.from + (last as Micros + 1) * self.step);
        let place = if room_tone { (self.centre(first) + self.centre(last)) / 2 } else { self.centre(at) };
        Some(Dip { at: place, db: low, room_tone, room })
    }

    /// The moment a word is loudest, if the recording is loud enough there to locate it.
    pub fn peak_of(&self, word: &Word) -> Option<Micros> {
        let i = self.extreme(word.source.start, word.source.end, false)?;
        (self.db[i] > QUIET_DECIBELS).then(|| self.centre(i))
    }
}

/// Builds [`Levels`] from sound that arrives in pieces, each with its own start time.
#[derive(Clone, Debug, Default)]
pub struct LevelMeter {
    pub step: Micros,
    sums: Vec<u64>,
    counts: Vec<u32>,
}

impl LevelMeter {
    pub fn new(step: Micros) -> LevelMeter {
        LevelMeter { step, sums: Vec::new(), counts: Vec::new() }
    }

    /// Add stereo samples at `rate` starting at source time `at`.
    pub fn add(&mut self, at: Micros, pcm: &[[i16; 2]], rate: u32) {
        if self.step <= 0 || rate == 0 {
            return;
        }
        for (i, s) in pcm.iter().enumerate() {
            let t = at + (i as i128 * 1_000_000 / rate as i128) as Micros;
            if t < 0 {
                continue;
            }
            let b = (t / self.step) as usize;
            if b >= self.sums.len() {
                self.sums.resize(b + 1, 0);
                self.counts.resize(b + 1, 0);
            }
            self.sums[b] += ((s[0] as i64).pow(2) + (s[1] as i64).pow(2)) as u64;
            self.counts[b] += 2;
        }
    }

    pub fn levels(&self) -> Levels {
        let db = self.sums.iter().zip(&self.counts).map(|(s, c)| if *c == 0 { SILENT_DECIBELS } else { decibels(s / *c as u64) }).collect();
        Levels { from: 0, step: self.step, db }
    }
}

/// A place between two words an edge can go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dip {
    pub at: Micros,
    /// Its lowest level, tenths of dBFS.
    pub db: i16,
    /// At room tone rather than in a trough above it.
    pub room_tone: bool,
    /// The whole stretch down in the dip.
    pub room: Range,
}

/// How one edge of a cut was placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// A pause between the words, or the end of the material: the transcript's edge is safe.
    Pause,
    /// Words said in one breath; the edge moved onto the dip between them.
    Dip(Dip),
    /// Words said in one breath and no dip between them: the edge stays on the transcript's
    /// boundary, in sound, for the editor to hear before taking.
    InSound,
}

/// A word cut with each edge placed against the sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacedCut {
    pub cut: SourceCut,
    pub start: Edge,
    pub end: Edge,
}

impl PlacedCut {
    /// Both edges sit where no word sounds.
    pub fn clean(&self) -> bool {
        !matches!(self.start, Edge::InSound) && !matches!(self.end, Edge::InSound)
    }
}

impl Transcript {
    /// Where the edge between kept and removed words `left` and `right` goes, given the
    /// transcript's own `edge` there.
    fn place_edge(&self, levels: &Levels, left: usize, right: usize, edge: Micros) -> (Micros, Edge) {
        let (l, r) = (&self.words[left], &self.words[right]);
        if r.source.start - l.source.end >= RUN_ON_GAP {
            return (edge, Edge::Pause);
        }
        let from = levels.peak_of(l).unwrap_or(l.source.start);
        let to = levels.peak_of(r).unwrap_or(r.source.end);
        let boundary = (l.source.end + r.source.start) / 2;
        match levels.dip_between(from, to, Some(boundary)) {
            Some(d) => (d.at, Edge::Dip(d)),
            None => (edge, Edge::InSound),
        }
    }

    /// [`Transcript::cut_words`], with each edge between words said in one breath moved onto
    /// the waveform's dip between them, so the cut never clips a kept word.
    pub fn placed_cut(&self, first: usize, last: usize, levels: &Levels) -> Option<PlacedCut> {
        let cut = self.cut_words(first, last)?;
        let (start, start_edge) = match first.checked_sub(1) {
            Some(prev) => self.place_edge(levels, prev, first, cut.range.start),
            None => (cut.range.start, Edge::Pause),
        };
        let (end, end_edge) = if last + 1 < self.words.len() {
            self.place_edge(levels, last, last + 1, cut.range.end)
        } else {
            (cut.range.end, Edge::Pause)
        };
        if end <= start {
            return None;
        }
        Some(PlacedCut { cut: SourceCut { media: cut.media, range: Range::new(start, end) }, start: start_edge, end: end_edge })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use std::vec;

    const STEP: Micros = 20_000;

    /// Levels from a list of (milliseconds, tenths of dBFS) stretches.
    fn levels(stretches: &[(Micros, i16)]) -> Levels {
        let mut db = Vec::new();
        for &(ms, level) in stretches {
            db.extend(core::iter::repeat(level).take((ms * 1000 / STEP) as usize));
        }
        Levels { from: 0, step: STEP, db }
    }

    fn word(text: &str, start_ms: Micros, end_ms: Micros) -> Word {
        Word { text: String::from(text), source: Range::new(start_ms * 1000, end_ms * 1000) }
    }

    #[test]
    fn decibels_of_full_scale_half_scale_and_silence() {
        assert_eq!(decibels(1 << 30), 0);
        // Half amplitude is a quarter of the power: -6.02 dB.
        assert_eq!(decibels(1 << 28), -60);
        assert_eq!(decibels(1), -903);
        assert_eq!(decibels(0), SILENT_DECIBELS);
        let tone: Vec<[i16; 2]> = (0..4800).map(|i| if i % 2 == 0 { [16384, 16384] } else { [-16384, -16384] }).collect();
        let l = Levels::from_pcm(0, &tone, 48_000, STEP);
        assert_eq!(l.db.len(), 5);
        assert!(l.db.iter().all(|d| *d == -60), "{:?}", l.db);
        let mut m = LevelMeter::new(STEP);
        m.add(80_000, &tone[..960], 48_000);
        m.add(0, &tone[960..], 48_000);
        assert_eq!(m.levels().db, l.db);
    }

    #[test]
    fn room_tone_between_run_together_words_takes_its_middle() {
        // "game" 0-300 ms loud, 60 ms of room tone, "let" loud.
        let l = levels(&[(300, -200), (60, -600), (300, -200)]);
        let d = l.dip_between(100_000, 500_000, None).unwrap();
        assert!(d.room_tone);
        assert_eq!(d.room, Range::new(300_000, 360_000));
        assert_eq!(d.at, 330_000);
    }

    #[test]
    fn a_deep_trough_counts_and_a_shallow_one_only_near_the_boundary() {
        // A trough at -50 dBFS, 30 dB under its neighbours.
        let l = levels(&[(200, -200), (20, -500), (200, -200)]);
        let d = l.dip_between(0, 420_000, None).unwrap();
        assert!(!d.room_tone);
        assert_eq!(d.at, 210_000);
        // At -45 dBFS it is too shallow on its own ...
        let l = levels(&[(200, -200), (20, -450), (200, -200)]);
        assert_eq!(l.dip_between(0, 420_000, None), None);
        // ... but takes the edge when the transcript put the boundary within 50 ms of it.
        assert_eq!(l.dip_between(0, 420_000, Some(240_000)).map(|d| d.at), Some(210_000));
        assert_eq!(l.dip_between(0, 420_000, Some(300_000)), None);
    }

    #[test]
    fn the_lowest_level_at_the_search_end_is_a_wall_not_a_trough() {
        // Level falls steadily into the next stretch: the search's lowest bucket is its last one.
        let l = levels(&[(200, -200), (40, -300), (40, -400), (40, -470), (200, -490)]);
        assert_eq!(l.dip_between(0, 300_000, None), None);
    }

    #[test]
    fn a_cut_between_run_together_words_moves_onto_the_dip() {
        // "so" 100-200, "um" 210-350, "the" 360-600 by the transcript, run together; the
        // sound has room tone at 200-220 and 380-440 ms. The transcript's end edge at 360 ms
        // would clip the tail of "um"; the placed one waits for the room tone.
        let l = levels(&[(100, -600), (100, -200), (20, -600), (160, -250), (60, -620), (160, -200), (100, -600)]);
        let t = Transcript { media: 3, words: vec![word("so", 100, 200), word("um", 210, 350), word("the", 360, 600)] };
        let p = t.placed_cut(1, 1, &l).unwrap();
        assert_eq!(t.cut_words(1, 1).unwrap().range, Range::new(210_000, 360_000));
        assert!(p.clean(), "{p:?}");
        assert_eq!(p.cut.range, Range::new(210_000, 410_000));
        assert!(matches!(p.start, Edge::Dip(d) if d.room_tone));
        assert!(matches!(p.end, Edge::Dip(d) if d.room_tone));
        assert_eq!(p.cut.media, 3);
    }

    #[test]
    fn a_pause_keeps_the_transcript_edge_and_no_dip_is_reported_in_sound() {
        let l = levels(&[(1000, -200)]);
        let t = Transcript { media: 1, words: vec![word("one", 0, 200), word("two", 400, 600), word("three", 610, 800)] };
        let p = t.placed_cut(1, 1, &l).unwrap();
        assert_eq!(p.start, Edge::Pause);
        assert_eq!(p.end, Edge::InSound);
        assert!(!p.clean());
        assert_eq!(p.cut.range, Range::new(400_000, 610_000));
    }
}
