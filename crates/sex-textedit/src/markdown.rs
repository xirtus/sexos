// SPDX-License-Identifier: GPL-3.0-only
//! Markdown preview: CommonMark's everyday blocks (headings, paragraphs joined across
//! lines, lists, numbered lists, task boxes, quotes, fenced and indented code, rules,
//! tables kept as text) and inline styles (strong, emphasis, code, links, strikeout),
//! laid out as styled runs for a text grid. Jot wraps and draws them.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style { Plain, Strong, Emphasis, Code, Link, Strike, Heading(u8), Quote, Rule, Bullet }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// Columns of indent before the first run (lists, quotes).
    pub indent: usize,
    /// What starts the block (`• `, `1. `, `☐ `, `│ `), drawn in the indent.
    pub marker: String,
    pub runs: Vec<(String, Style)>,
    /// Code keeps its lines; everything else wraps.
    pub code: bool,
    /// Source line the block starts on, so the preview can follow the cursor.
    pub line: usize,
}

fn heading(l: &str) -> Option<(u8, &str)> {
    let n = l.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&n) && (l.len() == n || l.as_bytes()[n] == b' ') { Some((n as u8, l[n..].trim().trim_end_matches('#').trim_end())) } else { None }
}
fn is_rule(l: &str) -> bool {
    let t: String = l.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_'))
}
/// A list item: (indent, marker, rest).
fn item(l: &str) -> Option<(usize, String, &str)> {
    let indent = l.len() - l.trim_start().len();
    let t = l.trim_start();
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(m) {
            if let Some(r) = rest.strip_prefix("[ ] ") { return Some((indent, String::from("☐ "), r)); }
            if let Some(r) = rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")) { return Some((indent, String::from("☑ "), r)); }
            return Some((indent, String::from("• "), rest));
        }
    }
    let digits = t.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 && digits < 10 {
        let after = &t[digits..];
        if let Some(rest) = after.strip_prefix(". ").or_else(|| after.strip_prefix(") ")) { return Some((indent, alloc::format!("{}. ", &t[..digits]), rest)); }
    }
    None
}

/// Inline styles of one paragraph's text.
pub fn inline(s: &str, base: Style) -> Vec<(String, Style)> {
    let mut out: Vec<(String, Style)> = Vec::new();
    let push = |out: &mut Vec<(String, Style)>, t: &str, st: Style| {
        if t.is_empty() { return; }
        if let Some(last) = out.last_mut() { if last.1 == st { last.0.push_str(t); return; } }
        out.push((String::from(t), st));
    };
    let b = s.as_bytes();
    let mut i = 0;
    let mut plain = 0;
    while i < b.len() {
        let rest = &s[i..];
        let mut hit: Option<(usize, &str, Style)> = None; // (consumed, inner text, style)
        if rest.starts_with('`') {
            if let Some(e) = rest[1..].find('`') { hit = Some((e + 2, &rest[1..1 + e], Style::Code)); }
        } else if rest.starts_with("**") || rest.starts_with("__") {
            let m = &rest[..2];
            if let Some(e) = rest[2..].find(m) { if e > 0 { hit = Some((e + 4, &rest[2..2 + e], Style::Strong)); } }
        } else if rest.starts_with("~~") {
            if let Some(e) = rest[2..].find("~~") { if e > 0 { hit = Some((e + 4, &rest[2..2 + e], Style::Strike)); } }
        } else if (rest.starts_with('*') || rest.starts_with('_')) && rest.len() > 1 && !rest.as_bytes()[1].is_ascii_whitespace() {
            let m = &rest[..1];
            // `_` inside a word (snake_case) is not emphasis.
            let in_word = m == "_" && i > 0 && b[i - 1].is_ascii_alphanumeric();
            if !in_word { if let Some(e) = rest[1..].find(m) { if e > 0 { hit = Some((e + 2, &rest[1..1 + e], Style::Emphasis)); } } }
        } else if rest.starts_with('[') {
            if let Some(close) = rest.find("](") {
                if let Some(end) = rest[close + 2..].find(')') { hit = Some((close + 3 + end, &rest[1..close], Style::Link)); }
            }
        } else if rest.starts_with("![") {
            if let Some(close) = rest.find("](") {
                if let Some(end) = rest[close + 2..].find(')') { hit = Some((close + 3 + end, &rest[2..close], Style::Link)); }
            }
        }
        match hit {
            Some((n, inner, st)) => {
                push(&mut out, &s[plain..i], base);
                let st = if base != Style::Plain && st == Style::Emphasis { base } else { st };
                if st == Style::Code { push(&mut out, inner, st) } else { for (t, s2) in inline(inner, st) { push(&mut out, &t, s2); } }
                i += n;
                plain = i;
            }
            None => { i += s[i..].chars().next().map_or(1, |c| c.len_utf8()); }
        }
    }
    push(&mut out, &s[plain..], base);
    out
}

pub fn parse(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i].trim_end_matches('\r');
        let start = i;
        if l.trim().is_empty() { i += 1; continue; }
        if let Some(fence) = ["```", "~~~"].iter().find(|f| l.trim_start().starts_with(**f)) {
            let mut runs = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(*fence) { runs.push((String::from(lines[i]), Style::Code)); i += 1; }
            i += 1;
            out.push(Block { indent: 2, marker: String::new(), runs, code: true, line: start });
            continue;
        }
        if l.starts_with("    ") || l.starts_with('\t') {
            let mut runs = Vec::new();
            while i < lines.len() && (lines[i].starts_with("    ") || lines[i].starts_with('\t')) { runs.push((String::from(lines[i].trim_start_matches('\t').get(4.min(lines[i].len())..).unwrap_or("")), Style::Code)); i += 1; }
            out.push(Block { indent: 2, marker: String::new(), runs, code: true, line: start });
            continue;
        }
        if let Some((n, t)) = heading(l) {
            out.push(Block { indent: 0, marker: String::new(), runs: inline(t, Style::Heading(n)), code: false, line: start });
            i += 1;
            continue;
        }
        // Setext headings: a line underlined with === or ---.
        if i + 1 < lines.len() && !lines[i + 1].trim().is_empty() && (lines[i + 1].trim().chars().all(|c| c == '=') || (lines[i + 1].trim().chars().all(|c| c == '-') && lines[i + 1].trim().len() >= 2)) && item(l).is_none() {
            let n = if lines[i + 1].trim().starts_with('=') { 1 } else { 2 };
            out.push(Block { indent: 0, marker: String::new(), runs: inline(l.trim(), Style::Heading(n)), code: false, line: start });
            i += 2;
            continue;
        }
        if is_rule(l) { out.push(Block { indent: 0, marker: String::new(), runs: alloc::vec![(String::new(), Style::Rule)], code: false, line: start }); i += 1; continue; }
        if l.trim_start().starts_with('>') {
            let mut para = String::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let t = lines[i].trim_start()[1..].trim();
                if !para.is_empty() { para.push(' '); }
                para.push_str(t);
                i += 1;
            }
            out.push(Block { indent: 2, marker: String::from("│ "), runs: inline(&para, Style::Quote), code: false, line: start });
            continue;
        }
        if let Some((ind, marker, rest)) = item(l) {
            let mut para = String::from(rest.trim());
            i += 1;
            // Lazy continuation lines belong to the item.
            while i < lines.len() && !lines[i].trim().is_empty() && item(lines[i]).is_none() && heading(lines[i]).is_none() && lines[i].starts_with(' ') {
                para.push(' ');
                para.push_str(lines[i].trim());
                i += 1;
            }
            let width = marker.chars().count();
            out.push(Block { indent: ind / 2 * 2 + width, marker, runs: inline(&para, Style::Plain), code: false, line: start });
            continue;
        }
        // A paragraph: lines until a blank line or another block begins.
        let mut para = String::new();
        while i < lines.len() {
            let t = lines[i].trim_end_matches('\r');
            if t.trim().is_empty() || heading(t).is_some() || item(t).is_some() || t.trim_start().starts_with('>') || t.trim_start().starts_with("```") { break; }
            if !para.is_empty() && i != start && is_rule(t) { break; }
            // Tables keep their own lines.
            if t.trim_start().starts_with('|') {
                if !para.is_empty() { break; }
                let mut runs = Vec::new();
                while i < lines.len() && lines[i].trim_start().starts_with('|') { runs.push((String::from(lines[i].trim()), Style::Plain)); i += 1; }
                out.push(Block { indent: 0, marker: String::new(), runs, code: true, line: start });
                para.clear();
                break;
            }
            let hard = t.ends_with("  ");
            if !para.is_empty() { para.push(' '); }
            para.push_str(t.trim());
            if hard { para.push('\n'); }
            i += 1;
        }
        if !para.is_empty() { out.push(Block { indent: 0, marker: String::new(), runs: inline(&para, Style::Plain), code: false, line: start }); }
    }
    out
}
