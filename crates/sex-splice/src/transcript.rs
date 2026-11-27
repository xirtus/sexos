// SPDX-License-Identifier: GPL-3.0-only
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

fn leading_punct(text: &str, keep_leading_apos: bool) -> &str {
    let pred = |c: char| c.is_alphanumeric() || (keep_leading_apos && c == '\'');
    let idx = text.find(pred).unwrap_or(text.len());
    &text[..idx]
}

fn trailing_punct(text: &str, keep_trailing_apos: bool) -> &str {
    let pred = |c: char| c.is_alphanumeric() || (keep_trailing_apos && c == '\'');
    let idx = text
        .rfind(pred)
        .map(|i| i + text[i..].chars().next().map_or(0, char::len_utf8))
        .unwrap_or(0);
    &text[idx..]
}

fn match_word_token(word_text: &str, target_norm: &str, allow_possessive: bool) -> Option<bool> {
    let raw = norm(word_text);
    if raw == target_norm {
        return Some(false);
    }
    let stripped = if !target_norm.starts_with('\'') && !target_norm.ends_with('\'') {
        raw.trim_matches('\'')
    } else {
        raw.as_str()
    };
    if stripped == target_norm {
        return Some(false);
    }
    if allow_possessive
        && !target_norm.ends_with("'s")
        && stripped.strip_prefix(target_norm) == Some("'s")
    {
        return Some(true);
    }
    None
}

impl Transcript {
    pub fn is_filler(&self, i: usize) -> bool {
        let n = norm(&self.words[i].text);
        FILLERS.iter().any(|f| *f == n)
    }

    /// Case-insensitively find single-word or multi-word phrases via [`norm`] and replace them in
    /// place while preserving leading/trailing punctuation on edge words and keeping exact
    /// [`Word::source`] timings (splitting proportionally across the matched span when the
    /// replacement word count differs; adapted from Scarlet's `fixWord`, MIT).
    ///
    /// Returns the number of phrase occurrences replaced.
    pub fn fix_word(&mut self, find: &str, replace: &str) -> usize {
        let find_norm: Vec<String> = find
            .split_whitespace()
            .map(norm)
            .filter(|s| !s.is_empty())
            .collect();
        let rep_tokens: Vec<&str> = replace.split_whitespace().collect();
        if find_norm.is_empty() || rep_tokens.is_empty() {
            return 0;
        }

        let m = find_norm.len();
        let r = rep_tokens.len();
        let keep_lead_apos = find_norm[0].starts_with('\'');
        let keep_trail_apos = find_norm[m - 1].ends_with('\'');
        let mut replaced = 0usize;
        let mut i = 0usize;

        while i + m <= self.words.len() {
            let mut possessive_tail = false;
            let mut all_match = true;
            for k in 0..m {
                match match_word_token(&self.words[i + k].text, &find_norm[k], k + 1 == m) {
                    Some(poss) => {
                        if k + 1 == m {
                            possessive_tail = poss;
                        }
                    }
                    None => {
                        all_match = false;
                        break;
                    }
                }
            }
            if !all_match {
                i += 1;
                continue;
            }

            let lead = String::from(leading_punct(&self.words[i].text, keep_lead_apos));
            let trail = String::from(trailing_punct(&self.words[i + m - 1].text, keep_trail_apos));
            let span_start = self.words[i].source.start;
            let span_end = self.words[i + m - 1].source.end;
            let total = span_end - span_start;

            let mut new_words = Vec::with_capacity(r);
            for (k, &tok) in rep_tokens.iter().enumerate() {
                let mut text = String::new();
                if k == 0 && !lead.is_empty() && !tok.starts_with(lead.as_str()) {
                    text.push_str(&lead);
                }
                text.push_str(tok);
                if k + 1 == r {
                    if possessive_tail && !tok.ends_with("'s") && !tok.ends_with("'S") {
                        text.push_str("'s");
                    }
                    if !trail.is_empty() && !text.ends_with(trail.as_str()) {
                        text.push_str(&trail);
                    }
                }

                let source = if r == m {
                    self.words[i + k].source
                } else {
                    let s = span_start + ((total as i128 * k as i128) / r as i128) as Micros;
                    let e = span_start + ((total as i128 * (k + 1) as i128) / r as i128) as Micros;
                    Range::new(s, e)
                };
                new_words.push(Word { text, source });
            }

            self.words.splice(i..i + m, new_words);
            replaced += 1;
            i += r;
        }

        replaced
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn w(text: &str, start: Micros, end: Micros) -> Word {
        Word { text: String::from(text), source: Range::new(start, end) }
    }

    #[test]
    fn fix_word_preserves_edge_punctuation_and_exact_timings() {
        let mut tr = Transcript {
            media: 1,
            words: vec![
                w("\"Scarlett,\"", 100_000, 500_000),
                w("meets", 550_000, 800_000),
                w("scarlett!", 850_000, 1_300_000),
                w("(SCARLETT).", 1_400_000, 1_900_000),
            ],
        };

        let n = tr.fix_word("scarlett", "Splicy");
        assert_eq!(n, 3);
        assert_eq!(
            tr.words,
            vec![
                w("\"Splicy,\"", 100_000, 500_000),
                w("meets", 550_000, 800_000),
                w("Splicy!", 850_000, 1_300_000),
                w("(Splicy).", 1_400_000, 1_900_000),
            ]
        );
    }

    #[test]
    fn fix_word_splits_and_merges_multi_word_phrases_proportionally() {
        // 2 words -> 1 word merges source span and combines outer edge punctuation.
        let mut tr = Transcript {
            media: 1,
            words: vec![
                w("“sex", 200_000, 450_000),
                w("os,”", 500_000, 800_000),
                w("runs", 900_000, 1_200_000),
            ],
        };
        assert_eq!(tr.fix_word("Sex OS", "SexOS"), 1);
        assert_eq!(
            tr.words,
            vec![
                w("“SexOS,”", 200_000, 800_000),
                w("runs", 900_000, 1_200_000),
            ]
        );

        // 1 word -> 3 words splits 200_000..800_000 into 3 equal 200_000us slices, keeping edge quotes!
        assert_eq!(tr.fix_word("sexos", "Splicy Video Editor"), 1);
        assert_eq!(
            tr.words,
            vec![
                w("“Splicy", 200_000, 400_000),
                w("Video", 400_000, 600_000),
                w("Editor,”", 600_000, 800_000),
                w("runs", 900_000, 1_200_000),
            ]
        );

        // 2 words -> 2 words preserves each original word's exact Range (including inter-word gap).
        let mut same_len = Transcript {
            media: 1,
            words: vec![w("old", 100, 200), w("name,", 250, 400)],
        };
        assert_eq!(same_len.fix_word("old name", "Splicy Core"), 1);
        assert_eq!(
            same_len.words,
            vec![w("Splicy", 100, 200), w("Core,", 250, 400)]
        );

        // Self-expanding replacement ("Splicy" -> "Splicy Pro") terminates cleanly without re-matching.
        assert_eq!(same_len.fix_word("splicy", "Splicy Pro"), 1);
        assert_eq!(same_len.words.len(), 3);
        assert_eq!(same_len.fix_word("", "anything"), 0);
        assert_eq!(same_len.fix_word("splicy", "   "), 0);
    }

    #[test]
    fn fix_word_preserves_possessives_and_single_quotes() {
        let mut tr = Transcript {
            media: 1,
            words: vec![
                w("'Scarlett',", 0, 400_000),
                w("Scarlett's.", 500_000, 900_000),
                w("'90s!", 1_000_000, 1_400_000),
            ],
        };
        assert_eq!(tr.fix_word("scarlett", "Splicy"), 2);
        assert_eq!(tr.fix_word("'90s", "1990s"), 1);
        assert_eq!(
            tr.words,
            vec![
                w("'Splicy',", 0, 400_000),
                w("Splicy's.", 500_000, 900_000),
                w("1990s!", 1_000_000, 1_400_000),
            ]
        );
    }
}

