//! Editing by transcript: delete words and their frames and sound go with them.
//!
//! A transcript belongs to one source file and its times are source times, so it stays
//! valid however the timeline is cut. Every function here answers in source ranges
//! ([`SourceCut`]) and [`ripple_ops`] turns those into timeline ops against whatever the
//! timeline looks like now. That is what lets a suggestion made an hour ago still land on
//! the right words after other edits.

use crate::{Micros, Op, Range, Timeline};
use alloc::string::String;
use alloc::vec::Vec;

/// One recognised word, with where it is spoken in its source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub source: Range,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    pub media: u32,
    /// Words in spoken order, not overlapping.
    pub words: Vec<Word>,
}

/// A span of one source to leave out of the edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceCut {
    pub media: u32,
    pub range: Range,
}

/// Hesitation sounds a dialogue pass may remove. Words such as "like" or "so" carry
/// meaning too often to cut by rule; they are left to the editor or the assistant.
pub const FILLERS: &[&str] = &["um", "uh", "umm", "uhh", "er", "erm", "ah", "hmm", "mm"];

/// Lower-case letters and digits of a word, so "Um," and "um" compare equal.
pub fn norm(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric() || *c == '\'').flat_map(char::to_lowercase).collect()
}

impl Transcript {
    pub fn is_filler(&self, i: usize) -> bool {
        let n = norm(&self.words[i].text);
        FILLERS.iter().any(|f| *f == n)
    }

    /// Leave out words `first..=last`. The cut runs from the first word to the start of the
    /// word after the last, so the pause before the removed words becomes the pause before
    /// the next one and no double pause is left behind.
    pub fn cut_words(&self, first: usize, last: usize) -> Option<SourceCut> {
        if first > last || last >= self.words.len() {
            return None;
        }
        let start = self.words[first].source.start;
        let end = match self.words.get(last + 1) {
            Some(next) => next.source.start,
            None => self.words[last].source.end,
        };
        Some(SourceCut { media: self.media, range: Range::new(start, end) })
    }

    /// Cut every filler word on its own.
    pub fn filler_cuts(&self) -> Vec<SourceCut> {
        (0..self.words.len()).filter(|i| self.is_filler(*i)).filter_map(|i| self.cut_words(i, i)).collect()
    }

    /// Shorten every silence between words longer than `longest` down to `keep`, taking the
    /// time out of the middle so each side keeps its breath.
    pub fn pause_cuts(&self, longest: Micros, keep: Micros) -> Vec<SourceCut> {
        let keep = keep.min(longest);
        self.words
            .windows(2)
            .filter_map(|w| {
                let gap = Range::new(w[0].source.end, w[1].source.start);
                if gap.len() <= longest {
                    return None;
                }
                let lead = keep / 2;
                let range = Range::new(gap.start + lead, gap.end - (keep - lead));
                Some(SourceCut { media: self.media, range })
            })
            .collect()
    }

    /// Retakes: a phrase of at least `min_words` words said again straight away. Returns the
    /// earlier take's word span `(first, last)`, which a dialogue pass removes so the later,
    /// usually cleaner, take plays. Longest repeat wins at each position.
    pub fn repeated_takes(&self, min_words: usize) -> Vec<(usize, usize)> {
        let n: Vec<String> = self.words.iter().map(|w| norm(&w.text)).collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < n.len() {
            let mut best = 0;
            let mut len = (n.len() - i) / 2;
            while len >= min_words.max(1) {
                if n[i..i + len] == n[i + len..i + 2 * len] {
                    best = len;
                    break;
                }
                len -= 1;
            }
            if best > 0 {
                out.push((i, i + best - 1));
                i += best;
            } else {
                i += 1;
            }
        }
        out
    }

    /// The words as they play now: index into `words` and timeline start. A word whose
    /// start has been cut out is missing.
    pub fn playing(&self, tl: &Timeline) -> Vec<(usize, Micros)> {
        self.words.iter().enumerate().filter_map(|(i, w)| tl.place_of(self.media, w.source.start).map(|t| (i, t))).collect()
    }
}

/// Turn source cuts into ripple deletes on the timeline as it is now. Each cut is found on
/// the first track that plays its source; where earlier edits removed part of it, only the
/// part still playing is cut. Ops come latest first so each leaves the earlier ones valid.
pub fn ripple_ops(tl: &Timeline, cuts: &[SourceCut]) -> Vec<Op> {
    let mut spans: Vec<Range> = Vec::new();
    for cut in cuts {
        let Some(track) = tl.tracks.iter().find(|t| t.clips.iter().any(|c| c.media == cut.media)) else { continue };
        for c in track.clips.iter().filter(|c| c.media == cut.media) {
            let s = Range::new(cut.range.start.max(c.source.start), cut.range.end.min(c.source.end));
            if s.is_empty() {
                continue;
            }
            let (Some(a), Some(b)) = (c.timeline_at(s.start), c.timeline_at(s.end)) else { continue };
            if b > a {
                spans.push(Range::new(a, b));
            }
        }
    }
    spans.sort_by_key(|r| r.start);
    let mut merged: Vec<Range> = Vec::new();
    for r in spans {
        match merged.last_mut() {
            Some(m) if r.start <= m.end => m.end = m.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged.into_iter().rev().map(|range| Op::RippleDelete { range }).collect()
}
