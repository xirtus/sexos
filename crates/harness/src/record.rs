// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Record lines: `[tag] key=value key="a value with spaces"`.
//!
//! This is how a SexOS driver reports: one line per fact, on the serial record
//! stream. Gates read them in QEMU, Spindle `hw save` collects them on a real
//! machine, and `scripts/hw_report.py` and `scripts/hardware_list.py` turn them
//! into verdicts and the public hardware list. [`Record`] makes lines those tools
//! can always parse: tags of `[a-z0-9_.]`, keys of `[A-Za-z0-9_]`, and values
//! quoted when they hold a space (inner quotes become `'`). Lines are built in a
//! fixed buffer (no allocation) and cut at [`MAX_LINE`] bytes.

use core::fmt::Write;

pub const MAX_LINE: usize = 240;

/// Where finished lines go: the serial record stream on SexOS, a `Vec` in tests.
pub trait Sink {
    fn line(&mut self, line: &str);
}

impl<S: Sink + ?Sized> Sink for &mut S {
    fn line(&mut self, line: &str) { (**self).line(line) }
}

/// A sink that drops everything (a driver used where nobody listens).
pub struct Quiet;
impl Sink for Quiet { fn line(&mut self, _: &str) {} }

/// Whether `tag` is a valid record tag: `[a-z0-9_]` words joined by single dots.
pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.split('.').all(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
}

struct Line { buf: [u8; MAX_LINE], len: usize }
impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.len < MAX_LINE { self.buf[self.len] = b; self.len += 1; }
        }
        Ok(())
    }
}
impl Line {
    fn as_str(&self) -> &str {
        // Cut back to a char boundary if a multi-byte char was split at the end.
        let mut n = self.len;
        while core::str::from_utf8(&self.buf[..n]).is_err() { n -= 1; }
        core::str::from_utf8(&self.buf[..n]).unwrap_or("")
    }
}

/// One record line under construction. Built with chained calls and sent with
/// [`Record::emit`]:
///
/// ```
/// # use harness::record::{Record, Sink};
/// # struct V(Vec<String>); impl Sink for V { fn line(&mut self, l: &str) { self.0.push(l.into()) } }
/// # let mut out = V(Vec::new());
/// Record::new("edu.ready").hex("id", 0x0100_00ed).num("irq", 1).text("mode", "msi x").emit(&mut out);
/// assert_eq!(out.0[0], "[edu.ready] id=0x10000ed irq=1 mode=\"msi x\"");
/// ```
pub struct Record { line: Line }

impl Record {
    /// Starts a line. An invalid tag is replaced by `harness.bad_tag` (and the
    /// line still goes out), so a typo shows up instead of vanishing.
    pub fn new(tag: &str) -> Record {
        let mut line = Line { buf: [0; MAX_LINE], len: 0 };
        let tag = if valid_tag(tag) { tag } else { "harness.bad_tag" };
        let _ = write!(line, "[{tag}]");
        Record { line }
    }
    fn key(&mut self, key: &str) {
        let _ = self.line.write_char(' ');
        let mut any = false;
        for c in key.chars() {
            let c = if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' };
            let _ = self.line.write_char(c);
            any = true;
        }
        if !any { let _ = self.line.write_char('_'); }
        let _ = self.line.write_char('=');
    }
    pub fn num(mut self, key: &str, value: u64) -> Record { self.key(key); let _ = write!(self.line, "{value}"); self }
    pub fn int(mut self, key: &str, value: i64) -> Record { self.key(key); let _ = write!(self.line, "{value}"); self }
    pub fn hex(mut self, key: &str, value: u64) -> Record { self.key(key); let _ = write!(self.line, "{value:#x}"); self }
    pub fn flag(mut self, key: &str, value: bool) -> Record { self.key(key); let _ = self.line.write_char(if value { '1' } else { '0' }); self }
    /// Text; quoted when it holds whitespace, `"` or nothing. Control characters
    /// become `?` so one record is always one line.
    pub fn text(mut self, key: &str, value: &str) -> Record {
        self.key(key);
        let quote = value.is_empty() || value.chars().any(|c| c.is_whitespace() || c == '"');
        if quote { let _ = self.line.write_char('"'); }
        for c in value.chars() {
            let c = match c { '"' => '\'', c if c.is_control() => '?', c if quote || !c.is_whitespace() => c, _ => '_' };
            let _ = self.line.write_char(c);
        }
        if quote { let _ = self.line.write_char('"'); }
        self
    }
    /// Bytes as unbroken hex (an EDID, a descriptor). `hw_report.py --seeds`
    /// turns such fields into fuzz seeds.
    pub fn bytes(mut self, key: &str, value: &[u8]) -> Record {
        self.key(key);
        for b in value { let _ = write!(self.line, "{b:02x}"); }
        self
    }
    pub fn as_str(&self) -> &str { self.line.as_str() }
    pub fn emit<S: Sink>(self, sink: &mut S) { sink.line(self.line.as_str()) }
}
