// SPDX-License-Identifier: GPL-3.0-only
//! CSS: style sheets into rules, selectors and declarations, and selector matching.
//!
//! Declarations stay as text until the cascade needs them (see `style`). `@media`
//! blocks are kept when their query fits the viewport width; `@supports` content is
//! kept; other at-rules (`@font-face`, `@keyframes`, `@import`, ...) are skipped.
use crate::dom::{Dom, Kind, NodeId};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub struct Declaration {
    pub name: String,
    pub value: String,
    pub important: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttrOp { Exists, Equals(String), Word(String), Prefix(String), Suffix(String), Contains(String), Dash(String) }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pseudo { FirstChild, LastChild, OnlyChild, Link, Root, Empty, NthChild(i32, i32), Not(Vec<Compound>),
    /// Under the pointer (or an ancestor of what is).
    Hover, Focus, FocusWithin, Checked, Disabled, Enabled }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, AttrOp)>,
    pub pseudo: Vec<Pseudo>,
    /// A pseudo-class or element this engine never matches (`:hover`, `::before`, ...).
    pub never: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combinator { Descendant, Child, Next, Subsequent }

/// Compounds left to right; `links[i]` joins `parts[i]` to `parts[i + 1]`.
#[derive(Clone, Debug)]
pub struct Selector {
    pub parts: Vec<Compound>,
    pub links: Vec<Combinator>,
    pub specificity: u32,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

#[derive(Default, Debug)]
pub struct Sheet {
    pub rules: Vec<Rule>,
}

fn skip_comments_and_space(b: &[u8], mut i: usize) -> usize {
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() { i += 1; }
        if b[i..].starts_with(b"/*") {
            i = b[i + 2..].windows(2).position(|w| w == b"*/").map_or(b.len(), |p| i + 2 + p + 2);
        } else if b[i..].starts_with(b"<!--") {
            i += 4;
        } else if b[i..].starts_with(b"-->") {
            i += 3;
        } else {
            return i;
        }
    }
}

/// Index just past the block that opens at `open` (`b[open] == b'{'`), honouring strings and comments.
fn block_end(b: &[u8], open: usize) -> usize {
    let mut depth = 0;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => { depth -= 1; if depth == 0 { return i + 1; } }
            q @ (b'"' | b'\'') => { i += 1; while i < b.len() && b[i] != q { if b[i] == b'\\' { i += 1; } i += 1; } }
            b'/' if b.get(i + 1) == Some(&b'*') => { i = b[i + 2..].windows(2).position(|w| w == b"*/").map_or(b.len(), |p| i + 2 + p + 1); }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

/// Where the next `{` or `;` at depth zero is (outside strings and parentheses).
fn prelude_end(b: &[u8], mut i: usize) -> usize {
    let mut paren = 0;
    while i < b.len() {
        match b[i] {
            b'(' => paren += 1,
            b')' => paren -= 1,
            b'{' | b';' if paren <= 0 => return i,
            q @ (b'"' | b'\'') => { i += 1; while i < b.len() && b[i] != q { if b[i] == b'\\' { i += 1; } i += 1; } }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

pub fn parse_sheet(css: &str, viewport_width: i32) -> Sheet {
    let mut sheet = Sheet::default();
    parse_rules(css, viewport_width, &mut sheet);
    sheet
}

fn parse_rules(css: &str, vw: i32, sheet: &mut Sheet) {
    let b = css.as_bytes();
    let mut i = 0;
    while i < b.len() {
        i = skip_comments_and_space(b, i);
        if i >= b.len() { break; }
        let end = prelude_end(b, i);
        let prelude = css[i..end].trim();
        if end >= b.len() { break; }
        if b[end] == b';' { i = end + 1; continue; }
        let close = block_end(b, end);
        let inner = &css[end + 1..close.saturating_sub(1).max(end + 1)];
        if let Some(at) = prelude.strip_prefix('@') {
            let name = at.split(|c: char| c.is_whitespace() || c == '(').next().unwrap_or("").to_ascii_lowercase();
            match name.as_str() {
                "media" => if media_matches(&at[5..], vw) { parse_rules(inner, vw, sheet) },
                "supports" | "layer" | "document" | "-moz-document" | "container" => parse_rules(inner, vw, sheet),
                _ => {}
            }
        } else {
            let selectors = parse_selector_list(prelude);
            if !selectors.is_empty() {
                let declarations = parse_declarations(inner);
                if !declarations.is_empty() { sheet.rules.push(Rule { selectors, declarations }); }
            }
        }
        i = close;
    }
}

/// A media query list against a screen `vw` pixels wide.
pub fn media_matches(query: &str, vw: i32) -> bool {
    let q = query.to_ascii_lowercase();
    q.split(',').any(|one| {
        let one = one.trim();
        let (negated, one) = match one.strip_prefix("not ") { Some(r) => (true, r), None => (false, one) };
        let one = one.strip_prefix("only ").unwrap_or(one);
        let mut ok = true;
        for (n, part) in one.split(" and ").enumerate() {
            let part = part.trim();
            if part.starts_with('(') {
                let inner = part.trim_start_matches('(').trim_end_matches(')');
                let (feature, value) = match inner.split_once(':') { Some((f, v)) => (f.trim(), v.trim()), None => (inner.trim(), "") };
                let px = |v: &str| parse_px(v, 16);
                ok &= match feature {
                    "min-width" => px(value).map_or(false, |w| vw >= w),
                    "max-width" => px(value).map_or(false, |w| vw <= w),
                    "prefers-color-scheme" => value == "light",
                    "prefers-reduced-motion" => value == "no-preference",
                    "orientation" => value == "landscape",
                    "hover" | "any-hover" => value != "none",
                    "pointer" | "any-pointer" => value == "fine",
                    "min-resolution" | "-webkit-min-device-pixel-ratio" | "min--moz-device-pixel-ratio" => false,
                    "max-resolution" | "-webkit-max-device-pixel-ratio" => true,
                    _ => false,
                };
            } else if n == 0 {
                ok &= matches!(part, "" | "all" | "screen");
            }
        }
        ok != negated
    })
}

fn parse_px(v: &str, em: i32) -> Option<i32> {
    let v = v.trim();
    let num_end = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(v.len());
    let n = parse_number(&v[..num_end])?;
    let unit = &v[num_end..];
    Some(match unit { "" | "px" => n / 100, "em" | "rem" => n * em / 100, _ => return None })
}

/// A decimal number scaled by 100 (`1.5` -> 150).
pub fn parse_number(s: &str) -> Option<i32> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') { Some(r) => (true, r), None => (false, s.strip_prefix('+').unwrap_or(s)) };
    if s.is_empty() { return None; }
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if !int.bytes().all(|c| c.is_ascii_digit()) || !frac.bytes().all(|c| c.is_ascii_digit()) || (int.is_empty() && frac.is_empty()) { return None; }
    let mut v: i64 = 0;
    for c in int.bytes() { v = (v * 10 + (c - b'0') as i64).min(1 << 40); }
    v *= 100;
    let f = frac.as_bytes();
    if !f.is_empty() { v += (f[0] - b'0') as i64 * 10; }
    if f.len() > 1 { v += (f[1] - b'0') as i64; }
    if f.len() > 2 && f[2] >= b'5' { v += 1; }
    let v = v.min(i32::MAX as i64) as i32;
    Some(if neg { -v } else { v })
}

pub fn parse_declarations(block: &str) -> Vec<Declaration> {
    let mut out = Vec::new();
    let b = block.as_bytes();
    let mut i = 0;
    while i < b.len() {
        i = skip_comments_and_space(b, i);
        let end = prelude_end(b, i);
        // A nested block inside declarations (CSS nesting) is skipped whole.
        let stop = if end < b.len() && b[end] == b'{' { block_end(b, end) } else { end };
        let text = &block[i..end.min(block.len())];
        if end >= b.len() || b[end] == b';' {
            if let Some((name, value)) = text.split_once(':') {
                let name = name.trim().to_ascii_lowercase();
                let mut value = value.trim();
                let mut important = false;
                if let Some(at) = value.to_ascii_lowercase().rfind("!important") {
                    important = true;
                    value = value[..at].trim();
                }
                if !name.is_empty() && !value.is_empty() {
                    out.push(Declaration { name, value: String::from(strip_comments(value).trim()), important });
                }
            }
        }
        i = if stop >= b.len() { b.len() } else { stop + 1 };
    }
    out
}

fn strip_comments(v: &str) -> String {
    let mut out = String::new();
    let mut rest = v;
    while let Some(a) = rest.find("/*") {
        out.push_str(&rest[..a]);
        rest = rest[a + 2..].find("*/").map_or("", |e| &rest[a + 2 + e + 2..]);
    }
    out.push_str(rest);
    out
}

pub fn parse_selector_list(text: &str) -> Vec<Selector> {
    split_top(text, ',').into_iter().filter_map(|s| parse_selector(s.trim())).collect()
}

pub(crate) fn split_top(text: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            c if c == sep && depth == 0 => { out.push(&text[start..i]); start = i + c.len_utf8(); }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

fn ident_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b'_' || b[i] >= 0x80 || b[i] == b'\\') {
        if b[i] == b'\\' { i += 1; }
        i += 1;
    }
    i.min(b.len())
}

fn unescape(s: &str) -> String { s.replace('\\', "") }

pub fn parse_selector(text: &str) -> Option<Selector> {
    if text.is_empty() { return None; }
    let b = text.as_bytes();
    let mut parts = Vec::new();
    let mut links = Vec::new();
    let mut i = 0;
    let mut pending: Option<Combinator> = None;
    loop {
        let start = i;
        while i < b.len() && b[i].is_ascii_whitespace() { i += 1; }
        if i >= b.len() { break; }
        let combinator = match b[i] {
            b'>' => { i += 1; Some(Combinator::Child) }
            b'+' => { i += 1; Some(Combinator::Next) }
            b'~' => { i += 1; Some(Combinator::Subsequent) }
            _ => None,
        };
        if let Some(c) = combinator {
            pending = Some(c);
            continue;
        }
        if !parts.is_empty() {
            let c = pending.take().unwrap_or(Combinator::Descendant);
            if pending.is_none() && start == i && c == Combinator::Descendant { return None; }
            links.push(c);
        } else if pending.is_some() {
            return None;
        }
        let (compound, next) = parse_compound(text, i)?;
        parts.push(compound);
        i = next;
    }
    if parts.is_empty() || pending.is_some() { return None; }
    let specificity = parts.iter().map(compound_specificity).sum();
    Some(Selector { parts, links, specificity })
}

fn compound_specificity(c: &Compound) -> u32 {
    let mut s = 0;
    if c.id.is_some() { s += 1 << 16; }
    s += (c.classes.len() as u32 + c.attrs.len() as u32) << 8;
    for p in &c.pseudo {
        s += match p { Pseudo::Not(inner) => inner.iter().map(compound_specificity).max().unwrap_or(0), _ => 1 << 8 };
    }
    if c.tag.is_some() { s += 1; }
    s
}

fn parse_compound(text: &str, mut i: usize) -> Option<(Compound, usize)> {
    let b = text.as_bytes();
    let mut c = Compound::default();
    let start = i;
    if i < b.len() && b[i] == b'*' {
        i += 1;
    } else if i < b.len() && (b[i].is_ascii_alphabetic() || b[i] >= 0x80) {
        let e = ident_end(b, i);
        c.tag = Some(text[i..e].to_ascii_lowercase());
        i = e;
    }
    while i < b.len() {
        match b[i] {
            b'#' => { let e = ident_end(b, i + 1); if e == i + 1 { return None; } c.id = Some(unescape(&text[i + 1..e])); i = e; }
            b'.' => { let e = ident_end(b, i + 1); if e == i + 1 { return None; } c.classes.push(unescape(&text[i + 1..e])); i = e; }
            b'[' => {
                let close = text[i..].find(']')? + i;
                c.attrs.push(parse_attr(&text[i + 1..close])?);
                i = close + 1;
            }
            b':' => {
                let element = b.get(i + 1) == Some(&b':');
                let s = if element { i + 2 } else { i + 1 };
                let e = ident_end(b, s);
                let name = text[s..e].to_ascii_lowercase();
                i = e;
                let mut arg = "";
                if b.get(i) == Some(&b'(') {
                    let mut depth = 0;
                    let mut j = i;
                    while j < b.len() { if b[j] == b'(' { depth += 1 } else if b[j] == b')' { depth -= 1; if depth == 0 { break; } } j += 1; }
                    arg = &text[i + 1..j.min(b.len())];
                    i = (j + 1).min(b.len());
                }
                if element { c.never = true; continue; }
                match name.as_str() {
                    "first-child" => c.pseudo.push(Pseudo::FirstChild),
                    "last-child" => c.pseudo.push(Pseudo::LastChild),
                    "only-child" => c.pseudo.push(Pseudo::OnlyChild),
                    "link" | "any-link" => c.pseudo.push(Pseudo::Link),
                    "root" => c.pseudo.push(Pseudo::Root),
                    "empty" => c.pseudo.push(Pseudo::Empty),
                    "nth-child" => { let (a, bb) = parse_nth(arg)?; c.pseudo.push(Pseudo::NthChild(a, bb)); }
                    "not" => {
                        let mut inner = Vec::new();
                        for part in split_top(arg, ',') {
                            let (comp, end) = parse_compound(part.trim(), 0)?;
                            if end != part.trim().len() { return None; }
                            inner.push(comp);
                        }
                        c.pseudo.push(Pseudo::Not(inner));
                    }
                    "is" | "where" | "matches" | "-webkit-any" => {
                        // Single-compound arguments only; the first alternative that parses stands in.
                        let mut alts = split_top(arg, ',').into_iter().filter_map(|p| parse_compound(p.trim(), 0).filter(|(_, e)| *e == p.trim().len()));
                        let (comp, _) = alts.next()?;
                        merge(&mut c, comp);
                    }
                    "hover" => c.pseudo.push(Pseudo::Hover),
                    "focus" | "focus-visible" => c.pseudo.push(Pseudo::Focus),
                    "focus-within" => c.pseudo.push(Pseudo::FocusWithin),
                    "checked" => c.pseudo.push(Pseudo::Checked),
                    "disabled" => c.pseudo.push(Pseudo::Disabled),
                    "enabled" => c.pseudo.push(Pseudo::Enabled),
                    "visited" | "active" | "target"
                    | "before" | "after" | "first-line" | "first-letter" | "placeholder" | "selection" | "marker" => c.never = true,
                    "lang" | "dir" => {}
                    _ => return None,
                }
            }
            _ => break,
        }
    }
    if i == start { return None; }
    Some((c, i))
}

fn merge(into: &mut Compound, from: Compound) {
    if into.tag.is_none() { into.tag = from.tag; }
    if into.id.is_none() { into.id = from.id; }
    into.classes.extend(from.classes);
    into.attrs.extend(from.attrs);
    into.pseudo.extend(from.pseudo);
    into.never |= from.never;
}

fn parse_nth(arg: &str) -> Option<(i32, i32)> {
    let a = arg.trim().to_ascii_lowercase();
    match a.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    let a: String = a.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(n) = a.find('n') {
        let coef = match &a[..n] { "" | "+" => 1, "-" => -1, s => s.parse().ok()? };
        let rest = &a[n + 1..];
        let off = if rest.is_empty() { 0 } else { rest.parse().ok()? };
        Some((coef, off))
    } else {
        Some((0, a.parse().ok()?))
    }
}

fn parse_attr(inner: &str) -> Option<(String, AttrOp)> {
    let ops = ["~=", "^=", "$=", "*=", "|=", "="];
    for op in ops {
        if let Some(at) = inner.find(op) {
            let name = inner[..at].trim().to_ascii_lowercase();
            let mut value = inner[at + op.len()..].trim();
            if value.ends_with(" i") || value.ends_with(" s") { value = value[..value.len() - 2].trim(); }
            let value = String::from(value.trim_matches(|c| c == '"' || c == '\''));
            let op = match op {
                "~=" => AttrOp::Word(value), "^=" => AttrOp::Prefix(value), "$=" => AttrOp::Suffix(value),
                "*=" => AttrOp::Contains(value), "|=" => AttrOp::Dash(value), _ => AttrOp::Equals(value),
            };
            return Some((name, op));
        }
    }
    let name = inner.trim().to_ascii_lowercase();
    (!name.is_empty()).then_some((name, AttrOp::Exists))
}

pub fn matches(dom: &Dom, node: NodeId, sel: &Selector) -> bool {
    matches_from(dom, node, sel, sel.parts.len() - 1)
}

fn matches_from(dom: &Dom, node: NodeId, sel: &Selector, at: usize) -> bool {
    if !matches_compound(dom, node, &sel.parts[at]) { return false; }
    if at == 0 { return true; }
    match sel.links[at - 1] {
        Combinator::Child => parent_element(dom, node).map_or(false, |p| matches_from(dom, p, sel, at - 1)),
        Combinator::Descendant => {
            let mut p = parent_element(dom, node);
            while let Some(n) = p {
                if matches_from(dom, n, sel, at - 1) { return true; }
                p = parent_element(dom, n);
            }
            false
        }
        Combinator::Next => dom.prev_element(node).map_or(false, |p| matches_from(dom, p, sel, at - 1)),
        Combinator::Subsequent => {
            let mut p = dom.prev_element(node);
            while let Some(n) = p {
                if matches_from(dom, n, sel, at - 1) { return true; }
                p = dom.prev_element(n);
            }
            false
        }
    }
}

fn parent_element(dom: &Dom, node: NodeId) -> Option<NodeId> {
    dom.nodes[node].parent.filter(|p| matches!(dom.nodes[*p].kind, Kind::Element { .. }))
}

pub fn matches_compound(dom: &Dom, node: NodeId, c: &Compound) -> bool {
    if c.never { return false; }
    let Kind::Element { tag, .. } = &dom.nodes[node].kind else { return false };
    if let Some(t) = &c.tag { if t != tag { return false; } }
    if let Some(id) = &c.id { if dom.attr(node, "id") != Some(id.as_str()) { return false; } }
    if !c.classes.iter().all(|k| dom.has_class(node, k)) { return false; }
    for (name, op) in &c.attrs {
        let Some(v) = dom.attr(node, name) else { return false };
        let ok = match op {
            AttrOp::Exists => true,
            AttrOp::Equals(x) => v == x,
            AttrOp::Word(x) => v.split_ascii_whitespace().any(|w| w == x),
            AttrOp::Prefix(x) => !x.is_empty() && v.starts_with(x.as_str()),
            AttrOp::Suffix(x) => !x.is_empty() && v.ends_with(x.as_str()),
            AttrOp::Contains(x) => !x.is_empty() && v.contains(x.as_str()),
            AttrOp::Dash(x) => v == x || (v.starts_with(x.as_str()) && v.as_bytes().get(x.len()) == Some(&b'-')),
        };
        if !ok { return false; }
    }
    for p in &c.pseudo {
        let ok = match p {
            Pseudo::FirstChild => dom.prev_element(node).is_none(),
            Pseudo::LastChild => next_element(dom, node).is_none(),
            Pseudo::OnlyChild => dom.prev_element(node).is_none() && next_element(dom, node).is_none(),
            Pseudo::Link => tag == "a" && dom.attr(node, "href").is_some(),
            Pseudo::Root => tag == "html",
            Pseudo::Empty => dom.nodes[node].children.is_empty(),
            Pseudo::NthChild(a, b) => {
                let mut index = 1;
                let mut p = dom.prev_element(node);
                while let Some(n) = p { index += 1; p = dom.prev_element(n); }
                if *a == 0 { index == *b } else { let d = index - b; d % a == 0 && d / a >= 0 }
            }
            Pseudo::Not(inner) => !inner.iter().any(|c| matches_compound(dom, node, c)),
            Pseudo::Hover => dom.hover.is_some_and(|h| dom.is_inclusive_ancestor(node, h)),
            Pseudo::Focus => dom.caret.is_some_and(|(f, _)| f == node),
            Pseudo::FocusWithin => dom.caret.is_some_and(|(f, _)| dom.is_inclusive_ancestor(node, f)),
            Pseudo::Checked => dom.attr(node, "checked").is_some() || (tag == "option" && dom.attr(node, "selected").is_some()),
            Pseudo::Disabled => matches!(tag.as_str(), "input" | "button" | "select" | "textarea" | "option" | "fieldset") && dom.attr(node, "disabled").is_some(),
            Pseudo::Enabled => matches!(tag.as_str(), "input" | "button" | "select" | "textarea" | "option" | "fieldset") && dom.attr(node, "disabled").is_none(),
        };
        if !ok { return false; }
    }
    true
}

fn next_element(dom: &Dom, node: NodeId) -> Option<NodeId> {
    let parent = dom.nodes[node].parent?;
    let mut found = false;
    for c in dom.element_children(parent) {
        if found { return Some(c); }
        if c == node { found = true; }
    }
    None
}
