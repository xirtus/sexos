// SPDX-License-Identifier: GPL-3.0-only
//! Text subtitles: SubRip (`.srt`) and the dialogue of SubStation Alpha (`.ssa`/`.ass`).
//! Parsing only, no drawing: a [`Track`] answers which lines show at a time. Styling
//! is dropped: ASS override blocks (`{\i1}`), SRT tags (`<i>`) and positions. Bytes are
//! read as UTF-8 (a BOM is skipped); malformed cues are skipped, never fatal.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Lines joined by '\n'.
    pub text: String,
}

#[derive(Default, Debug)]
pub struct Track {
    /// Sorted by start.
    pub cues: Vec<Cue>,
}

impl Track {
    /// SRT or ASS, told apart by their content.
    pub fn parse(bytes: &[u8]) -> Track {
        let text = String::from_utf8_lossy(bytes);
        let text = text.trim_start_matches('\u{feff}');
        let mut cues = if text.contains("[Events]") || text.contains("Dialogue:") { parse_ass(text) } else { parse_srt(text) };
        cues.retain(|c| c.end_ms > c.start_ms && !c.text.is_empty());
        cues.sort_by_key(|c| c.start_ms);
        Track { cues }
    }

    /// The text showing at `ms`, every cue then on screen joined by '\n', or None.
    pub fn at(&self, ms: u64, out: &mut String) -> bool {
        out.clear();
        // Cues are short and few overlap: look back from the first that starts later.
        let end = self.cues.partition_point(|c| c.start_ms <= ms);
        for c in self.cues[..end].iter().rev().take(16).collect::<Vec<_>>().into_iter().rev() {
            if ms < c.end_ms {
                if !out.is_empty() { out.push('\n'); }
                out.push_str(&c.text);
            }
        }
        !out.is_empty()
    }
}

/// "HH:MM:SS,mmm" (SRT, also with '.') or "H:MM:SS.cc" (ASS centiseconds).
fn time(t: &str) -> Option<u64> {
    let t = t.trim();
    let (hms, frac) = match t.find([',', '.']) { Some(i) => (&t[..i], &t[i + 1..]), None => (t, "") };
    let mut parts = hms.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() { return None; }
    let n = |v: &str| v.trim().parse::<u64>().ok();
    let ms = match frac.len() {
        0 => 0,
        1 => n(frac)? * 100,
        2 => n(frac)? * 10,
        _ => n(&frac[..3])?,
    };
    Some((n(h)? * 3600 + n(m)? * 60 + n(s)?) * 1000 + ms)
}

/// Drops `<...>` tags (SRT) and `{...}` override blocks (ASS).
fn strip(line: &str) -> String {
    let mut out = String::new();
    let mut depth = None;
    for ch in line.chars() {
        match (depth, ch) {
            (None, '<') => depth = Some('>'),
            (None, '{') => depth = Some('}'),
            (Some(close), c) if c == close => depth = None,
            (Some(_), _) => {}
            (None, c) => out.push(c),
        }
    }
    String::from(out.trim())
}

fn parse_srt(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r')).peekable();
    while let Some(line) = lines.next() {
        let Some((a, b)) = line.split_once("-->") else { continue };
        let (Some(start_ms), Some(end_ms)) = (time(a), time(b.split_whitespace().next().unwrap_or(""))) else { continue };
        let mut body = String::new();
        while let Some(l) = lines.peek() {
            if l.trim().is_empty() { break; }
            let l = strip(lines.next().unwrap());
            if l.is_empty() { continue; }
            if !body.is_empty() { body.push('\n'); }
            body.push_str(&l);
        }
        cues.push(Cue { start_ms, end_ms, text: body });
    }
    cues
}

fn parse_ass(text: &str) -> Vec<Cue> {
    // The [Events] Format line names the columns; Text is always last.
    let mut columns: Vec<String> = ["Layer", "Start", "End", "Style", "Name", "MarginL", "MarginR", "MarginV", "Effect", "Text"]
        .iter().map(|s| String::from(*s)).collect();
    let mut cues = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(f) = line.strip_prefix("Format:") {
            let names: Vec<String> = f.split(',').map(|s| String::from(s.trim())).collect();
            if names.iter().any(|n| n == "Text") { columns = names; }
            continue;
        }
        let Some(rest) = line.strip_prefix("Dialogue:") else { continue };
        let fields: Vec<&str> = rest.splitn(columns.len(), ',').collect();
        if fields.len() != columns.len() { continue; }
        let col = |name: &str| columns.iter().position(|c| c == name).map(|i| fields[i]);
        let (Some(start_ms), Some(end_ms)) = (col("Start").and_then(time), col("End").and_then(time)) else { continue };
        let raw = col("Text").unwrap_or("").replace("\\N", "\n").replace("\\n", "\n").replace("\\h", " ");
        let body: Vec<String> = raw.split('\n').map(strip).filter(|l| !l.is_empty()).collect();
        cues.push(Cue { start_ms, end_ms, text: body.join("\n") });
    }
    cues
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(t: &Track, ms: u64) -> String { let mut s = String::new(); t.at(ms, &mut s); s }

    #[test]
    fn srt_cues_show_at_their_times() {
        let t = Track::parse(b"\xef\xbb\xbf1\r\n00:00:01,000 --> 00:00:02,500\r\nHello <i>there</i>\r\nsecond line\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000 X1:10\r\nBye\r\n\r\nbroken\n00:00:xx --> 1\nskip\n");
        assert_eq!(t.cues.len(), 2);
        assert_eq!(at(&t, 999), "");
        assert_eq!(at(&t, 1000), "Hello there\nsecond line");
        assert_eq!(at(&t, 2499), "Hello there\nsecond line");
        assert_eq!(at(&t, 2500), "");
        assert_eq!(at(&t, 3500), "Bye");
    }

    #[test]
    fn ass_dialogue_with_overrides_and_custom_format() {
        let t = Track::parse(b"[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Text\nDialogue: 0,0:00:01.50,0:00:03.00,Default,{\\i1}Hi,{\\b1} you\\Nthere\nDialogue: 0,0:00:02.00,0:00:02.20,Default,Over\nComment: 0,0:00:00.00,0:00:09.00,Default,no\n");
        assert_eq!(t.cues.len(), 2);
        assert_eq!(at(&t, 1600), "Hi, you\nthere");
        assert_eq!(at(&t, 2100), "Hi, you\nthere\nOver");
        assert_eq!(at(&t, 3000), "");
    }

    #[test]
    fn times() {
        assert_eq!(time("01:02:03,004"), Some(3_723_004));
        assert_eq!(time("0:00:01.5"), Some(1_500));
        assert_eq!(time("1:2"), None);
    }
}
