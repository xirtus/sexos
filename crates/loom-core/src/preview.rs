// SPDX-License-Identifier: GPL-3.0-only
//! Bounded, inert text for Loom's quick preview. No file content becomes a key
//! or a display control sequence.
use alloc::string::String;
use alloc::vec::Vec;

pub const MAX_BYTES: usize = 32 * 1024;
pub const PAGE_ROWS: usize = 18;
const COLS: usize = 120;

pub struct TextPreview { rows: Vec<String>, pub offset: usize, pub truncated: bool }

impl TextPreview {
    pub fn parse(bytes: &[u8], truncated: bool) -> Option<Self> {
        let source = match core::str::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) if truncated && e.error_len().is_none() => core::str::from_utf8(&bytes[..e.valid_up_to()]).ok()?,
            Err(_) => return None,
        };
        if source.chars().any(|c| c == '\0' || c.is_control() && !matches!(c, '\n' | '\r' | '\t')) { return None; }
        let mut rows = Vec::new();
        for line in source.lines() {
            let mut row = String::new();
            for c in line.trim_end_matches('\r').chars().take(COLS) { row.push(if matches!(c, '\t' | '\r') { ' ' } else { c }); }
            rows.push(row);
        }
        if rows.is_empty() { rows.push(String::new()); }
        Some(Self { rows, offset: 0, truncated })
    }

    pub fn len(&self) -> usize { self.rows.len() }
    pub fn page(&self) -> &[String] {
        &self.rows[self.offset..self.rows.len().min(self.offset + PAGE_ROWS)]
    }
    pub fn down(&mut self) { self.offset = (self.offset + 1).min(self.rows.len().saturating_sub(PAGE_ROWS)); }
    pub fn up(&mut self) { self.offset = self.offset.saturating_sub(1); }
    pub fn first(&mut self) { self.offset = 0; }
    pub fn last(&mut self) { self.offset = self.rows.len().saturating_sub(PAGE_ROWS); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_preview_pages_and_refuses_display_controls() {
        let mut data = String::new();
        for n in 0..25 { data.push_str(&alloc::format!("line {n}\r\n")); }
        let mut p = TextPreview::parse(data.as_bytes(), false).unwrap();
        assert_eq!(p.page().len(), PAGE_ROWS);
        assert_eq!(p.page()[0], "line 0");
        p.last(); assert_eq!(p.offset, 7);
        p.down(); assert_eq!(p.offset, 7);
        p.up(); assert_eq!(p.offset, 6);
        p.first(); assert_eq!(p.offset, 0);
        assert_eq!(TextPreview::parse(b"hello\x1b[2J", false).map(|p| p.len()), None);
        assert!(TextPreview::parse(b"hello\0there", false).is_none());
        assert!(TextPreview::parse(b"\xff", false).is_none());
        assert_eq!(TextPreview::parse(b"a\tb", true).unwrap().page()[0], "a b");
        assert_eq!(TextPreview::parse(b"a\rb", false).unwrap().page()[0], "a b");
        assert_eq!(TextPreview::parse(b"ok\xc3", true).unwrap().page()[0], "ok");
        assert!(TextPreview::parse(b"ok\xc3", false).is_none());
    }
}
