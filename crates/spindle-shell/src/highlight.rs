// SPDX-License-Identifier: GPL-3.0-only
//! Colours for the line as it is typed.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;

use crate::ast::Op;
use crate::ergonomics;
use crate::host::Host;
use crate::lexer::{lex, TokKind};
use crate::path;
use crate::signature::{self, NAMESPACES};
use crate::units;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// A built-in command name.
    Command,
    /// A defined shell alias (`..`, `ll`, `fetch`, ...).
    Alias,
    /// A program name.
    External,
    /// An unknown command head (not builtin, alias, or program).
    Unknown,
    /// `let`, `true`, `null`.
    Keyword,
    Flag,
    String,
    /// Numbers, sizes, durations, dates.
    Number,
    Variable,
    Operator,
    /// `|`, `;`, `&`, brackets.
    Punctuation,
    /// An existing file or folder path on the host.
    Path,
    Comment,
    /// Something that will not parse.
    Error,
    /// Bare words: paths, column names.
    Text,
}

impl Style {
    /// An ANSI SGR sequence for the style ("" for plain text).
    pub fn ansi(self) -> &'static str {
        match self {
            Style::Command => "\x1b[1;36m",
            Style::Alias => "\x1b[1;32m",
            Style::External => "\x1b[1;32m",
            Style::Unknown => "\x1b[4;31m",
            Style::Keyword => "\x1b[35m",
            Style::Flag => "\x1b[34m",
            Style::String => "\x1b[32m",
            Style::Number => "\x1b[35m",
            Style::Variable => "\x1b[1;35m",
            Style::Operator => "\x1b[33m",
            Style::Punctuation => "\x1b[1;37m",
            Style::Path => "\x1b[4m",
            Style::Comment => "\x1b[2m",
            Style::Error => "\x1b[4;31m",
            Style::Text => "",
        }
    }

    /// Rich 24-bit SexOS Noir SGR sequence for live command-line highlighting.
    pub fn ansi_truecolor(self) -> &'static str {
        match self {
            Style::Command => "\x1b[1;38;2;34;211;238m",
            Style::Alias => "\x1b[1;38;2;52;211;153m",
            Style::External => "\x1b[1;38;2;16;185;129m",
            Style::Unknown => "\x1b[4;38;2;244;63;94m",
            Style::Keyword => "\x1b[38;2;192;132;252m",
            Style::Flag => "\x1b[38;2;125;211;252m",
            Style::String => "\x1b[38;2;245;158;11m",
            Style::Number => "\x1b[38;2;216;180;254m",
            Style::Variable => "\x1b[1;38;2;192;132;252m",
            Style::Operator => "\x1b[38;2;167;139;250m",
            Style::Punctuation => "\x1b[1;38;2;167;139;250m",
            Style::Path => "\x1b[4;38;2;226;232;240m",
            Style::Comment => "\x1b[38;2;100;116;139m",
            Style::Error => "\x1b[4;38;2;244;63;94m",
            Style::Text => "",
        }
    }
}

/// Styled byte ranges of `line`, in order; bytes outside them are whitespace.
pub fn highlight(line: &str) -> Vec<(Range<usize>, Style)> {
    highlight_inner(line, None, &[], None)
}

/// Live host- and alias-aware syntax highlighting (`Unknown` underlined rose, existing paths underlined).
pub fn highlight_with(
    line: &str,
    host: &dyn Host,
    extra_cmds: &[&str],
    aliases: &BTreeMap<String, String>,
) -> Vec<(Range<usize>, Style)> {
    highlight_inner(line, Some(host), extra_cmds, Some(aliases))
}

fn highlight_inner(
    line: &str,
    host: Option<&dyn Host>,
    extra_cmds: &[&str],
    aliases: Option<&BTreeMap<String, String>>,
) -> Vec<(Range<usize>, Style)> {
    let toks = lex(line);
    let mut out = Vec::new();
    let mut head = true;
    let mut after_let = 0u8;
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        let style = match t.kind {
            TokKind::Word if after_let == 1 => { after_let = 2; Style::Variable }
            TokKind::Word if head => {
                head = false;
                if t.text == "let" { after_let = 1; Style::Keyword }
                else if t.text.starts_with('^') { Style::External }
                else if units::parse_literal(&t.text).is_some() { Style::Number }
                else {
                    let two = toks.get(i + 1).filter(|n| n.kind == TokKind::Word).and_then(|n| {
                        let mut s = t.text.clone();
                        s.push(' ');
                        s.push_str(&n.text);
                        signature::lookup(&s)
                    });
                    if two.is_some() {
                        out.push((t.span.range(), Style::Command));
                        i += 1;
                        out.push((toks[i].span.range(), Style::Command));
                        i += 1;
                        continue;
                    }
                    if signature::lookup(&t.text).is_some()
                        || NAMESPACES.contains(&t.text.as_str())
                        || extra_cmds.contains(&t.text.as_str())
                    {
                        Style::Command
                    } else if aliases.is_some_and(|a| a.contains_key(&t.text)) {
                        Style::Alias
                    } else if let Some(h) = host {
                        if ergonomics::is_known_command(&t.text, h, aliases.unwrap_or(&BTreeMap::new()), extra_cmds) {
                            Style::External
                        } else {
                            Style::Unknown
                        }
                    } else {
                        Style::External
                    }
                }
            }
            TokKind::Word => {
                let w = t.text.as_str();
                if matches!(w, "true" | "false" | "null") { Style::Keyword }
                else if units::parse_literal(w).is_some() { Style::Number }
                else if w.starts_with('-') && w.len() > 1 && w.as_bytes()[1].is_ascii_alphabetic() || w.starts_with("--") { Style::Flag }
                else if Op::from_token(w).is_some() || w == "not" { Style::Operator }
                else if let Some(h) = host {
                    let abs = path::resolve(&h.cwd(), &h.home(), w);
                    if h.stat(&abs).is_ok() { Style::Path } else { Style::Text }
                } else {
                    Style::Text
                }
            }
            TokKind::Backtick if head => { head = false; Style::External }
            TokKind::Str | TokKind::Backtick => { head = false; Style::String }
            TokKind::Var => { head = false; Style::Variable }
            TokKind::Op => {
                if after_let == 2 && t.text == "=" { after_let = 0; head = true; }
                Style::Operator
            }
            TokKind::Pipe | TokKind::Semi | TokKind::Newline | TokKind::LParen => { head = true; after_let = 0; Style::Punctuation }
            TokKind::Amp | TokKind::RParen | TokKind::LBracket | TokKind::RBracket | TokKind::LBrace | TokKind::RBrace | TokKind::Colon | TokKind::Comma => { head = false; Style::Punctuation }
            TokKind::Comment => Style::Comment,
            TokKind::Error => Style::Error,
        };
        if t.kind != TokKind::Newline { out.push((t.span.range(), style)); }
        i += 1;
    }
    out
}
