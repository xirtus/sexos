// SPDX-License-Identifier: GPL-3.0-only
//! Spans and errors with a caret under the bad part of the line.

use alloc::format;
use alloc::string::{String, ToString};

use crate::width;

/// A byte range of the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Span { Span { start, end } }
    /// The span covering both.
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
    pub fn range(self) -> core::ops::Range<usize> { self.start..self.end }
}

/// An error: what went wrong, where on the line, and what to do about it.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellError {
    pub message: String,
    pub span: Option<Span>,
    /// Text written after the carets.
    pub label: String,
    pub help: Option<String>,
}

impl ShellError {
    pub fn new(message: impl Into<String>) -> ShellError {
        ShellError { message: message.into(), span: None, label: String::new(), help: None }
    }
    pub fn at(message: impl Into<String>, span: Span, label: impl Into<String>) -> ShellError {
        ShellError { message: message.into(), span: Some(span), label: label.into(), help: None }
    }
    pub fn help(mut self, help: impl Into<String>) -> ShellError {
        self.help = Some(help.into());
        self
    }
    /// Gives the error a span when it has none yet.
    pub fn or_at(mut self, span: Span) -> ShellError {
        if self.span.is_none() { self.span = Some(span); }
        self
    }

    /// The error as terminal text: the message, the line holding the span with carets
    /// under it, and the help. `color` adds ANSI red.
    ///
    /// ```text
    /// error: unknown command `lss`
    ///   ls | lss
    ///        ^^^ not a built-in command or a program
    ///   help: did you mean `ls`?
    /// ```
    pub fn render(&self, source: &str, color: bool) -> String {
        let (red, bold, reset) = if color { ("\x1b[31m", "\x1b[1;31m", "\x1b[0m") } else { ("", "", "") };
        let mut out = format!("{}error:{} {}\n", bold, reset, self.message);
        if let Some(span) = self.span {
            let start = span.start.min(source.len());
            let line_start = source[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_end = source[start..].find('\n').map(|i| start + i).unwrap_or(source.len());
            let line = &source[line_start..line_end];
            let lead = width::str_width(&source[line_start..start]);
            let end = span.end.clamp(start, line_end);
            let carets = width::str_width(&source[start..end]).max(1);
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
            out.push_str("  ");
            for _ in 0..lead { out.push(' '); }
            out.push_str(red);
            for _ in 0..carets { out.push('^'); }
            if !self.label.is_empty() {
                out.push(' ');
                out.push_str(&self.label);
            }
            out.push_str(reset);
            out.push('\n');
        }
        if let Some(help) = &self.help {
            out.push_str("  help: ");
            out.push_str(help);
            out.push('\n');
        }
        out
    }
}

impl core::fmt::Display for ShellError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str(&self.message) }
}

impl From<String> for ShellError {
    fn from(s: String) -> ShellError { ShellError::new(s) }
}
impl From<&str> for ShellError {
    fn from(s: &str) -> ShellError { ShellError::new(s.to_string()) }
}
