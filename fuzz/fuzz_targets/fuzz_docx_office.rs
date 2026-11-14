// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_archive::{no_progress, Archive, EntryKind, IoError, Sink};

struct MemorySink {
    buf: Vec<u8>,
    limit: usize,
}

impl MemorySink {
    fn new(limit: usize) -> Self {
        Self { buf: Vec::new(), limit }
    }
}

impl Sink for MemorySink {
    fn write_all(&mut self, data: &[u8]) -> Result<(), IoError> {
        if self.buf.len().saturating_add(data.len()) > self.limit {
            return Err(IoError("docx extract limit exceeded"));
        }
        self.buf.extend_from_slice(data);
        Ok(())
    }
}

fn safe_pen_layout(x: i32, y: i32, text: &str, clip: Option<(i32, i32)>) -> Option<i32> {
    // Defends against the Bug 1 integer overflow: `cx + cw` and `x * 256`
    let (cx0, cx1) = clip.map_or((i32::MIN, i32::MAX), |(cx, cw)| {
        (cx, cx.checked_add(cw).unwrap_or(i32::MAX))
    });

    let _baseline = y.checked_add(12)?;
    let mut pen = x.checked_mul(256)?;

    for c in text.chars() {
        let at = pen / 256;
        if at >= cx1 {
            break;
        }
        // Advance simulation (e.g. 8 pixels * 256 per glyph)
        let advance = if c.is_ascii() { 8 * 256 } else { 12 * 256 };
        if at >= cx0 && at < cx1 {
            let _px = at;
        }
        pen = pen.checked_add(advance)?;
    }
    Some(pen / 256)
}

fn safe_mix(under: u32, over: u32, alpha: u32) -> u32 {
    let alpha = alpha.min(255);
    let ch = |s: u32| {
        let (u, o) = ((under >> s) & 0xff, (over >> s) & 0xff);
        ((o * alpha + u * (255 - alpha) + 127) / 255) << s
    };
    ch(16) | ch(8) | ch(0)
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. DOCX OOXML ZIP archive inspection & extraction
    if let Ok(archive) = Archive::open(data) {
        let entries = archive.entries().to_vec();
        for (i, entry) in entries.iter().enumerate().take(32) {
            if entry.kind != EntryKind::File || !entry.safe {
                continue;
            }
            if entry.path.starts_with("word/") && entry.path.ends_with(".xml") {
                let mut sink = MemorySink::new(128 * 1024);
                if archive.extract(i, &mut sink, &mut no_progress).is_ok() {
                    if let Ok(xml_str) = core::str::from_utf8(&sink.buf) {
                        let _ = sex_html::parse::parse(xml_str);
                    }
                }
            }
        }
    }

    // 2. Office Pen / Canvas rendering layout robustness
    if data.len() >= 8 {
        let x = i32::from_le_bytes(data[0..4].try_into().unwrap());
        let y = i32::from_le_bytes(data[4..8].try_into().unwrap());
        if let Ok(text) = core::str::from_utf8(&data[8..]) {
            let _ = safe_pen_layout(x, y, text, Some((0, 1024)));
            let _ = safe_pen_layout(x, y, text, None);
        }
    }

    // 3. Alpha blend channel mixing
    if data.len() >= 12 {
        let under = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let over = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let alpha = u32::from_le_bytes(data[8..12].try_into().unwrap());
        let _ = safe_mix(under, over, alpha);
    }
});
