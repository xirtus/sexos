// SPDX-License-Identifier: GPL-3.0-only
//! LRC Timed Lyrics Parser and Generator mapped to 48 kHz Moan timeline frames.
//!
//! Converts between standard `[mm:ss.xx]` lyric timestamps and sample-accurate
//! 48 kHz PCM frame offsets (1 ms = 48 frames at Bell's standard 48 kHz capture rate).

use alloc::{format, string::String, vec::Vec};
use moan_core::RATE;

/// A single timed lyric line with millisecond timestamp and 48 kHz frame offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LrcLine {
    /// Timestamp in milliseconds from the start of the audio.
    pub timestamp_ms: u32,
    /// Exact timeline offset in stereo PCM frames at 48 kHz.
    pub frame_offset: usize,
    /// Text of the lyric line.
    pub text: String,
}

impl LrcLine {
    /// Construct a new timed lyric line from a millisecond timestamp and text.
    pub fn new(timestamp_ms: u32, text: &str) -> Self {
        let frame_offset = ms_to_frame(timestamp_ms);
        Self {
            timestamp_ms,
            frame_offset,
            text: String::from(text),
        }
    }

    /// Construct from an exact frame offset and text.
    pub fn from_frame(frame_offset: usize, text: &str) -> Self {
        let timestamp_ms = frame_to_ms(frame_offset);
        Self {
            timestamp_ms,
            frame_offset,
            text: String::from(text),
        }
    }
}

/// Convert milliseconds to 48 kHz PCM frame count.
#[inline]
pub fn ms_to_frame(ms: u32) -> usize {
    (ms as usize).saturating_mul(RATE as usize / 1000)
}

/// Convert 48 kHz PCM frame count to milliseconds.
#[inline]
pub fn frame_to_ms(frame: usize) -> u32 {
    (frame / (RATE as usize / 1000)) as u32
}

/// Parse an LRC format string into a list of timed `LrcLine`s sorted by timestamp.
pub fn parse_lrc(content: &str) -> Vec<LrcLine> {
    let mut lines = Vec::new();

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Collect all timestamp tags [mm:ss.xx] on this line
        let mut timestamps = Vec::new();
        let mut idx = 0;
        let bytes = trimmed.as_bytes();

        while idx < bytes.len() && bytes[idx] == b'[' {
            if let Some(close_bracket) = trimmed[idx..].find(']') {
                let tag = &trimmed[idx + 1..idx + close_bracket];
                if let Some(ms) = parse_timestamp_tag(tag) {
                    timestamps.push(ms);
                    idx += close_bracket + 1;
                } else {
                    // Non-timestamp tag like [ar:Artist], skip
                    idx += close_bracket + 1;
                }
            } else {
                break;
            }
        }

        let lyric_text = trimmed[idx..].trim();
        for ts in timestamps {
            lines.push(LrcLine::new(ts, lyric_text));
        }
    }

    // Sort by timestamp
    lines.sort_by_key(|l| l.timestamp_ms);
    lines
}

/// Parse `mm:ss.xx` or `mm:ss.xxx` or `mm:ss` timestamp into milliseconds.
fn parse_timestamp_tag(tag: &str) -> Option<u32> {
    let parts: Vec<&str> = tag.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let minutes: u32 = parts[0].trim().parse().ok()?;
    let sec_parts: Vec<&str> = parts[1].split('.').collect();

    let seconds: u32 = sec_parts[0].trim().parse().ok()?;
    let millis: u32 = if sec_parts.len() == 2 {
        let frac_str = sec_parts[1].trim();
        match frac_str.len() {
            1 => frac_str.parse::<u32>().ok()? * 100,
            2 => frac_str.parse::<u32>().ok()? * 10,
            3 => frac_str.parse::<u32>().ok()?,
            _ => frac_str[..3].parse::<u32>().ok()?,
        }
    } else {
        0
    };

    let total_ms = minutes
        .checked_mul(60_000)?
        .checked_add(seconds.checked_mul(1000)?)?
        .checked_add(millis)?;

    Some(total_ms)
}

/// Generate a standard `[mm:ss.xx] lyric text` string from `LrcLine`s.
pub fn generate_lrc(lines: &[LrcLine]) -> String {
    let mut out = String::new();
    for line in lines {
        let total_sec = line.timestamp_ms / 1000;
        let mm = total_sec / 60;
        let ss = total_sec % 60;
        let hundredths = (line.timestamp_ms % 1000) / 10;
        out.push_str(&format!("[{:02}:{:02}.{:02}] {}\n", mm, ss, hundredths, line.text));
    }
    out
}
