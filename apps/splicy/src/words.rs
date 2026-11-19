// SPDX-License-Identifier: GPL-3.0-only
//! The transcript beside a clip: `talk.webm` has `talk.words`. One word per line, as
//! `START END WORD` with times in milliseconds from the clip's start; blank lines and
//! lines starting with `#` are skipped. Splicy's transcriber (S4) writes it, and anyone
//! can write one by hand or convert one from another tool.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_splice::transcript::{Transcript, Word};
use sex_splice::Range;

/// A line that does not read as `START END WORD`, counted from 1.
#[derive(Debug, PartialEq, Eq)]
pub struct BadLine(pub usize);

/// Read a transcript. Words must be in order and must not overlap.
pub fn parse(text: &str) -> Result<Transcript, BadLine> {
    let mut words: Vec<Word> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = BadLine(n + 1);
        let mut parts = line.splitn(3, char::is_whitespace);
        let start: i64 = parts.next().and_then(|s| s.parse().ok()).ok_or(BadLine(n + 1))?;
        let end: i64 = parts.next().and_then(|s| s.trim().parse().ok()).ok_or(BadLine(n + 1))?;
        let word = parts.next().map(str::trim).filter(|w| !w.is_empty()).ok_or(BadLine(n + 1))?;
        let (start, end) = (start.checked_mul(1000).ok_or(BadLine(n + 1))?, end.checked_mul(1000).ok_or(BadLine(n + 1))?);
        if start < 0 || end <= start || words.last().is_some_and(|w| w.source.end > start) {
            return Err(bad);
        }
        words.push(Word { text: word.into(), source: Range::new(start, end) });
    }
    Ok(Transcript { media: 0, words })
}

/// Write a transcript in the same form.
pub fn write(t: &Transcript) -> String {
    let mut out = String::from("# Splicy transcript: START_MS END_MS WORD\n");
    for w in &t.words {
        out.push_str(&format!("{} {} {}\n", w.source.start / 1000, w.source.end / 1000, w.text));
    }
    out
}

/// `talk.webm` -> `talk.words`.
pub fn beside(clip: &str) -> String {
    let stem = match clip.rfind('.') {
        Some(dot) if !clip[dot..].contains('/') => &clip[..dot],
        _ => clip,
    };
    format!("{stem}.words")
}
