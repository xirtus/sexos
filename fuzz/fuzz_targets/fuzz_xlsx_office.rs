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
            return Err(IoError("xlsx extract limit exceeded"));
        }
        self.buf.extend_from_slice(data);
        Ok(())
    }
}

/// Checked column name calculation defending against Bug 2 (`col + 1` overflow when col == u32::MAX)
fn safe_column_name(col: u32) -> Option<String> {
    let mut n = col.checked_add(1)?;
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).ok()
}

/// Parse column name back to index
fn parse_column_name(name: &str) -> Option<u32> {
    if name.is_empty() || name.len() > 7 {
        return None;
    }
    let mut col: u32 = 0;
    for b in name.bytes() {
        if !b.is_ascii_uppercase() {
            return None;
        }
        col = col.checked_mul(26)?.checked_add((b - b'A' + 1) as u32)?;
    }
    col.checked_sub(1)
}

fn tokenize_formula(formula: &str) {
    let mut chars = formula.chars().peekable();
    if chars.peek() == Some(&'=') {
        chars.next();
    }
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\r' | '\n' => {
                chars.next();
            }
            '+' | '-' | '*' | '/' | '(' | ')' | ',' | ':' => {
                chars.next();
            }
            '0'..='9' => {
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() || d == '.' {
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            'A'..='Z' | 'a'..='z' | '_' => {
                let mut ident = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_alphanumeric() || d == '_' {
                        ident.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let _ = parse_column_name(&ident);
            }
            '"' => {
                chars.next();
                while let Some(d) = chars.next() {
                    if d == '"' {
                        break;
                    }
                }
            }
            _ => {
                chars.next();
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. XLSX OOXML ZIP archive inspection & extraction
    if let Ok(archive) = Archive::open(data) {
        let entries = archive.entries().to_vec();
        for (i, entry) in entries.iter().enumerate().take(32) {
            if entry.kind != EntryKind::File || !entry.safe {
                continue;
            }
            if entry.path.starts_with("xl/") && entry.path.ends_with(".xml") {
                let mut sink = MemorySink::new(128 * 1024);
                if archive.extract(i, &mut sink, &mut no_progress).is_ok() {
                    if let Ok(xml_str) = core::str::from_utf8(&sink.buf) {
                        let _ = sex_html::parse::parse(xml_str);
                    }
                }
            }
        }
    }

    // 2. Column coordinate arithmetic (testing boundary cases & u32::MAX)
    if data.len() >= 4 {
        let col = u32::from_le_bytes(data[0..4].try_into().unwrap());
        if let Some(name) = safe_column_name(col) {
            let _ = parse_column_name(&name);
        }
    }

    // 3. Formula tokenization
    if let Ok(formula) = core::str::from_utf8(data) {
        if formula.len() < 2048 {
            tokenize_formula(formula);
        }
    }
});
