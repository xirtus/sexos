// SPDX-License-Identifier: GPL-3.0-only
//! Quick-select hints, as kitty's and WezTerm's: everything on the visible screen that
//! looks worth copying or opening (URLs, file paths with an optional `:line:col`, git
//! hashes, IP addresses, email addresses, long numbers) gets a short label to type.
//!
//! The detectors are small hand-written scanners rather than regular expressions, so
//! they cost nothing to build and work without the `regex` feature.

use crate::Screen;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKind {
    Url,
    Email,
    Ip,
    /// A file path, maybe with `:line` or `:line:col` after it.
    Path,
    /// 7 to 40 hex digits: a git commit.
    Hash,
    /// Four digits or more: a PID, a port, an issue number.
    Number,
}

/// One hint: its label, what it found, and where on the visible screen it is, from
/// (`col`, `row`) to (`end_col`, `end_row`) inclusive (a hint can continue on a
/// soft-wrapped row).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub label: String,
    pub kind: HintKind,
    pub text: String,
    pub col: usize,
    pub row: usize,
    pub end_col: usize,
    pub end_row: usize,
}

/// The order labels take their letters from: the home row first.
pub const ALPHABET: &str = "asdfghjklqwertyuiopzxcvbnm";

/// The hints on the visible screen, top to bottom, labelled.
pub fn find(screen: &Screen) -> Vec<Hint> {
    let mut found: Vec<(HintKind, String, (usize, usize), (usize, usize))> = Vec::new();
    let rows = screen.rows();
    let mut row = 0;
    while row < rows {
        // A logical line: this row and the visible rows it soft-wraps onto.
        let first = row;
        let mut last = row;
        while last + 1 < rows && screen.visible_line(last).is_some_and(|l| l.wrapped) { last += 1; }
        let mut chars: Vec<char> = Vec::new();
        let mut at: Vec<(usize, usize)> = Vec::new();
        for r in first..=last {
            for c in 0..screen.cols() { chars.push(screen.visible(c, r).ch); at.push((c, r)); }
        }
        for (kind, a, b) in scan(&chars) { found.push((kind, chars[a..b].iter().collect(), at[a], at[b - 1])); }
        row = last + 1;
    }
    let labels = labels(found.len());
    found.into_iter().zip(labels).map(|((kind, text, (col, row), (end_col, end_row)), label)| Hint { label, kind, text, col, row, end_col, end_row }).collect()
}

/// `n` labels, none a prefix of another, as short as they can be: single letters while
/// they last, then the remaining letters start two-letter labels (kitty's scheme).
pub fn labels(n: usize) -> Vec<String> {
    let abc: Vec<char> = ALPHABET.chars().collect();
    let k = abc.len();
    let mut out = Vec::with_capacity(n);
    if n <= k { out.extend(abc[..n].iter().map(|c| String::from(*c))); return out; }
    if n <= k * k {
        // s singles leave k - s prefixes of k labels each: s + (k - s) * k >= n.
        let singles = (k * k - n) / (k - 1);
        out.extend(abc[..singles].iter().map(|c| String::from(*c)));
        'outer: for p in &abc[singles..] {
            for c in &abc { if out.len() == n { break 'outer; } let mut s = String::from(*p); s.push(*c); out.push(s); }
        }
        return out;
    }
    'three: for a in &abc { for b in &abc { for c in &abc {
        if out.len() == n { break 'three; }
        let mut s = String::from(*a); s.push(*b); s.push(*c); out.push(s);
    } } }
    out
}

/// What the letters typed so far pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolve {
    /// This hint's label was typed in full.
    Exact(usize),
    /// Some labels start with what was typed: keep reading.
    Prefix,
    /// No label starts with it.
    None,
}
pub fn resolve(hints: &[Hint], typed: &str) -> Resolve {
    if let Some(i) = hints.iter().position(|h| h.label == typed) { return Resolve::Exact(i); }
    if hints.iter().any(|h| h.label.starts_with(typed)) { Resolve::Prefix } else { Resolve::None }
}

// ---- The detectors ----

fn word(c: char) -> bool { c.is_alphanumeric() || c == '_' }
fn at_word_end(s: &[char], i: usize) -> bool { s.get(i).is_none_or(|&c| !word(c)) }

/// Every hint in one logical line, as (kind, start, end exclusive), left to right.
fn scan(s: &[char]) -> Vec<(HintKind, usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let boundary = i == 0 || !word(s[i - 1]);
        if boundary && !s[i].is_whitespace() {
            let hit = url(s, i).map(|e| (HintKind::Url, e))
                .or_else(|| email(s, i).map(|e| (HintKind::Email, e)))
                .or_else(|| ipv6(s, i).map(|e| (HintKind::Ip, e)))
                .or_else(|| ipv4(s, i).map(|e| (HintKind::Ip, e)))
                .or_else(|| path(s, i).map(|e| (HintKind::Path, e)))
                .or_else(|| hash(s, i).map(|e| (HintKind::Hash, e)))
                .or_else(|| number(s, i).map(|e| (HintKind::Number, e)));
            if let Some((kind, end)) = hit { out.push((kind, i, end)); i = end; continue; }
        }
        i += 1;
    }
    out
}

fn starts_with(s: &[char], i: usize, p: &str) -> bool {
    let mut j = i;
    for c in p.chars() { if s.get(j) != Some(&c) { return false; } j += 1; }
    true
}

/// Drops sentence punctuation and closing brackets that do not belong to the match.
fn trim_end(s: &[char], start: usize, mut end: usize) -> usize {
    loop {
        let Some(&c) = s.get(end.wrapping_sub(1)).filter(|_| end > start) else { return end };
        let unbalanced = |open: char, close: char| c == close && s[start..end].iter().filter(|&&x| x == open).count() < s[start..end].iter().filter(|&&x| x == close).count();
        if ".,;:!?'\"".contains(c) || unbalanced('(', ')') || unbalanced('[', ']') || unbalanced('{', '}') { end -= 1; } else { return end; }
    }
}

fn url(s: &[char], i: usize) -> Option<usize> {
    const SCHEMES: [&str; 9] = ["https://", "http://", "ftp://", "sftp://", "file://", "ssh://", "git://", "mailto:", "www."];
    let scheme = SCHEMES.iter().find(|p| starts_with(s, i, p))?;
    let mut end = i + scheme.chars().count();
    while end < s.len() && !s[end].is_whitespace() && !s[end].is_control() && !"<>\"`|\\^".contains(s[end]) { end += 1; }
    let end = trim_end(s, i, end);
    (end > i + scheme.chars().count()).then_some(end)
}

fn email(s: &[char], i: usize) -> Option<usize> {
    let local = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let mut j = i;
    while j < s.len() && local(s[j]) { j += 1; }
    if j == i || s.get(j) != Some(&'@') { return None; }
    let d0 = j + 1;
    let mut k = d0;
    while k < s.len() && (s[k].is_ascii_alphanumeric() || s[k] == '-' || s[k] == '.') { k += 1; }
    while k > d0 && s[k - 1] == '.' { k -= 1; }
    let domain = &s[d0..k];
    let dot = domain.iter().rposition(|&c| c == '.')?;
    let tld = &domain[dot + 1..];
    (dot > 0 && tld.len() >= 2 && tld.iter().all(|c| c.is_ascii_alphabetic())).then_some(k)
}

fn ipv4(s: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    for part in 0..4 {
        if part > 0 { if s.get(j) != Some(&'.') { return None; } j += 1; }
        let d0 = j;
        let mut v = 0u32;
        while j < s.len() && s[j].is_ascii_digit() && j - d0 < 3 { v = v * 10 + s[j].to_digit(10)?; j += 1; }
        if j == d0 || v > 255 { return None; }
    }
    if s.get(j).is_some_and(|c| c.is_ascii_digit()) || (s.get(j) == Some(&'.') && s.get(j + 1).is_some_and(|c| c.is_ascii_digit())) { return None; }
    // An optional port.
    if s.get(j) == Some(&':') && s.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
        let mut k = j + 1;
        while k < s.len() && s[k].is_ascii_digit() { k += 1; }
        if at_word_end(s, k) { j = k; }
    }
    at_word_end(s, j).then_some(j)
}

fn ipv6(s: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    while j < s.len() && (s[j].is_ascii_hexdigit() || s[j] == ':') { j += 1; }
    // A trailing lone colon is punctuation ("addr fe80::1: up").
    while j > i && s[j - 1] == ':' && !(j >= i + 2 && s[j - 2] == ':') { j -= 1; }
    if !at_word_end(s, j) && s.get(j) != Some(&'%') { return None; }
    let t: String = s[i..j].iter().collect();
    let colons = t.matches(':').count();
    let doubles = t.matches("::").count();
    if colons < 2 || doubles > 1 || t.contains(":::") || !t.chars().any(|c| c.is_ascii_digit()) { return None; }
    let groups: Vec<&str> = t.split(':').filter(|g| !g.is_empty()).collect();
    if groups.iter().any(|g| g.len() > 4) { return None; }
    if !(groups.len() == 8 && doubles == 0 || doubles == 1 && groups.len() < 8) { return None; }
    Some(j)
}

fn path(s: &[char], i: usize) -> Option<usize> {
    let pc = |c: char| c.is_alphanumeric() || "._-/~+%@=,#".contains(c);
    let mut j = i;
    while j < s.len() && pc(s[j]) { j += 1; }
    let mut j = trim_end(s, i, j);
    let t = &s[i..j];
    let has_slash = t.contains(&'/');
    let has_letter = t.iter().any(|c| c.is_alphabetic());
    // `name.ext` without a slash counts only with a `:line` after it.
    let dotted = !has_slash && t.iter().skip(1).any(|&c| c == '.') && t.last().is_some_and(|c| c.is_alphanumeric());
    let line_at = |j: usize| s.get(j) == Some(&':') && s.get(j + 1).is_some_and(|c| c.is_ascii_digit());
    if !has_letter || !(has_slash || dotted && line_at(j)) { return None; }
    if t.iter().all(|&c| c == '/' || c == '.' || c == '~') { return None; }
    // `:line` and `:line:col`.
    for _ in 0..2 {
        if !line_at(j) { break; }
        let mut k = j + 1;
        while k < s.len() && s[k].is_ascii_digit() { k += 1; }
        j = k;
    }
    Some(j)
}

fn hash(s: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    while j < s.len() && matches!(s[j], '0'..='9' | 'a'..='f') { j += 1; }
    let t = &s[i..j];
    let ok = (7..=40).contains(&t.len()) && at_word_end(s, j) && t.iter().any(|c| c.is_ascii_digit()) && t.iter().any(|c| c.is_ascii_alphabetic());
    ok.then_some(j)
}

fn number(s: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    while j < s.len() && s[j].is_ascii_digit() { j += 1; }
    (j - i >= 4 && at_word_end(s, j)).then_some(j)
}
