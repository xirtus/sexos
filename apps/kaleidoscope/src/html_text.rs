// SPDX-License-Identifier: GPL-3.0-only
//! Bounded HTML-to-readable-text conversion for the Browser reader.
//!
//! This is a reading view, not a web engine: there is no DOM, CSS, layout,
//! script execution, network fetch or form. Markup becomes plain UTF-8 text:
//! headings and block elements start new lines, list items get a bullet,
//! links keep their text plus a numbered reference listed at the end, and
//! `script`, `style`, `head`, `template` and comments contribute nothing. The
//! output never exceeds the caller's buffer; a cut is marked, never silent.

pub const MAX_LINKS: usize = 16;
const TRUNCATED: &[u8] = b"\n[page truncated]";

pub struct Output<'a> {
    buf: &'a mut [u8],
    len: usize,
    truncated: bool,
    /// The last emitted byte was whitespace or nothing has been emitted.
    space: bool,
    newlines: u8,
}

impl<'a> Output<'a> {
    fn new(buf: &'a mut [u8]) -> Self { Self { buf, len: 0, truncated: false, space: true, newlines: 2 } }
    fn room(&self) -> usize { self.buf.len().saturating_sub(TRUNCATED.len()) }
    fn byte(&mut self, b: u8) {
        if self.len < self.room() { self.buf[self.len] = b; self.len += 1; } else { self.truncated = true; }
    }
    fn text(&mut self, b: u8) {
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c) {
            self.space = true;
            return;
        }
        if self.space && self.newlines == 0 { self.byte(b' '); }
        self.space = false;
        self.newlines = 0;
        self.byte(b);
    }
    fn bytes(&mut self, s: &[u8]) { for &b in s { self.text(b); } }
    /// End the current line; `blank` asks for an empty line before the next block.
    fn line(&mut self, blank: bool) {
        let want = if blank { 2 } else { 1 };
        while self.newlines < want { self.byte(b'\n'); self.newlines += 1; }
        self.space = true;
    }
    fn finish(mut self) -> (usize, bool) {
        while self.len > 0 && self.buf[self.len - 1] == b'\n' { self.len -= 1; }
        if self.truncated {
            for &b in TRUNCATED { self.buf[self.len] = b; self.len += 1; }
        }
        (self.len, self.truncated)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Converted { pub len: usize, pub links: usize, pub truncated: bool }

fn lower(b: u8) -> u8 { b.to_ascii_lowercase() }
fn name_is(name: &[u8], want: &[u8]) -> bool { name.len() == want.len() && name.iter().zip(want).all(|(a, b)| lower(*a) == *b) }

/// Value of attribute `want` inside a tag's attribute bytes, unquoted.
fn attribute<'b>(attrs: &'b [u8], want: &[u8]) -> Option<&'b [u8]> {
    let mut i = 0;
    while i < attrs.len() {
        while i < attrs.len() && attrs[i].is_ascii_whitespace() { i += 1; }
        let start = i;
        while i < attrs.len() && !attrs[i].is_ascii_whitespace() && attrs[i] != b'=' { i += 1; }
        let name = &attrs[start..i];
        while i < attrs.len() && attrs[i].is_ascii_whitespace() { i += 1; }
        if i < attrs.len() && attrs[i] == b'=' {
            i += 1;
            while i < attrs.len() && attrs[i].is_ascii_whitespace() { i += 1; }
            let (value, next) = match attrs.get(i) {
                Some(&q @ (b'"' | b'\'')) => {
                    let end = attrs[i + 1..].iter().position(|&b| b == q).map_or(attrs.len(), |p| i + 1 + p);
                    (&attrs[(i + 1).min(attrs.len())..end], end + 1)
                }
                _ => {
                    let end = attrs[i..].iter().position(|b| b.is_ascii_whitespace()).map_or(attrs.len(), |p| i + p);
                    (&attrs[i..end], end)
                }
            };
            if name_is(name, want) { return Some(value); }
            i = next;
        } else if name.is_empty() {
            i += 1;
        }
    }
    None
}

/// Decode one character reference starting after `&`; returns the bytes to
/// emit and how many input bytes (after `&`) it consumed.
fn entity(input: &[u8], out: &mut [u8; 4]) -> Option<(usize, usize)> {
    let end = input.iter().take(12).position(|&b| b == b';')?;
    let name = &input[..end];
    let code: u32 = if let Some(num) = name.strip_prefix(b"#") {
        let (digits, radix) = match num.first() { Some(b'x' | b'X') => (&num[1..], 16), _ => (num, 10) };
        if digits.is_empty() { return None; }
        let mut v: u32 = 0;
        for &d in digits { v = v.checked_mul(radix)?.checked_add((d as char).to_digit(radix)?)?; }
        v
    } else {
        match name {
            b"amp" => b'&' as u32, b"lt" => b'<' as u32, b"gt" => b'>' as u32, b"quot" => b'"' as u32,
            b"apos" => b'\'' as u32, b"nbsp" => b' ' as u32, b"copy" => 0xA9, b"mdash" => 0x2014,
            b"ndash" => 0x2013, b"hellip" => 0x2026, _ => return None,
        }
    };
    // Controls and surrogates are not readable text.
    let ch = char::from_u32(code).filter(|c| !c.is_control() || *c == '\n')?;
    Some((ch.encode_utf8(out).len(), end + 1))
}

const SKIP: [&[u8]; 6] = [b"script", b"style", b"head", b"template", b"noscript", b"svg"];
const BLOCK: [&[u8]; 14] = [b"p", b"div", b"section", b"article", b"header", b"footer", b"main", b"nav", b"ul", b"ol", b"table", b"blockquote", b"pre", b"hr"];

/// Convert `html` into readable text in `out`. Link targets are appended as a
/// numbered list; at most MAX_LINKS are kept.
pub fn convert(html: &[u8], out: &mut [u8]) -> Converted {
    let mut o = Output::new(out);
    let mut links: [(usize, usize); MAX_LINKS] = [(0, 0); MAX_LINKS];
    let mut link_count = 0usize;
    let mut open_link: Option<usize> = None;
    let mut skip_until: Option<&[u8]> = None;
    let mut i = 0;
    while i < html.len() {
        let b = html[i];
        if b == b'<' {
            if html[i..].starts_with(b"<!--") {
                i = html[i + 4..].windows(3).position(|w| w == b"-->").map_or(html.len(), |p| i + 4 + p + 3);
                continue;
            }
            let Some(close) = html[i..].iter().position(|&c| c == b'>') else { break };
            let tag = &html[i + 1..i + close];
            i += close + 1;
            let (end_tag, body) = match tag.first() { Some(b'/') => (true, &tag[1..]), _ => (false, tag) };
            let name_end = body.iter().position(|c| c.is_ascii_whitespace() || *c == b'/').unwrap_or(body.len());
            let name = &body[..name_end];
            if let Some(skip) = skip_until {
                if end_tag && name_is(name, skip) { skip_until = None; }
                continue;
            }
            if !end_tag {
                if let Some(s) = SKIP.iter().find(|s| name_is(name, s)) {
                    if !body.ends_with(b"/") { skip_until = Some(s); }
                    continue;
                }
            }
            let h = name.len() == 2 && lower(name[0]) == b'h' && (b'1'..=b'6').contains(&name[1]);
            if h {
                o.line(true);
                if !end_tag && name[1] <= b'2' { o.bytes(b"# "); }
            } else if name_is(name, b"br") {
                o.line(false);
            } else if name_is(name, b"li") && !end_tag {
                o.line(false);
                o.bytes(b"- ");
            } else if name_is(name, b"tr") && end_tag {
                o.line(false);
            } else if (name_is(name, b"td") || name_is(name, b"th")) && !end_tag {
                if o.newlines == 0 { o.bytes(b" |"); o.space = true; }
            } else if BLOCK.iter().any(|n| name_is(name, n)) {
                o.line(true);
            } else if name_is(name, b"a") {
                if !end_tag {
                    if let Some(href) = attribute(&body[name_end..], b"href") {
                        if link_count < MAX_LINKS {
                            let start = (href.as_ptr() as usize) - (html.as_ptr() as usize);
                            links[link_count] = (start, href.len());
                            open_link = Some(link_count);
                            link_count += 1;
                        }
                    }
                } else if let Some(n) = open_link.take() {
                    let mut tagbuf = [0u8; 6];
                    let len = number(n + 1, &mut tagbuf);
                    o.text(b'[');
                    o.space = false;
                    for &d in &tagbuf[..len] { o.byte(d); }
                    o.byte(b']');
                }
            }
            continue;
        }
        if skip_until.is_some() { i += 1; continue; }
        if b == b'&' {
            let mut buf = [0u8; 4];
            if let Some((n, used)) = entity(&html[i + 1..], &mut buf) {
                for &c in &buf[..n] { o.text(c); }
                i += 1 + used;
                continue;
            }
        }
        o.text(b);
        i += 1;
    }
    if link_count > 0 {
        o.line(true);
        o.bytes(b"Links:");
        for (n, &(start, len)) in links[..link_count].iter().enumerate() {
            o.line(false);
            let mut tagbuf = [0u8; 6];
            let digits = number(n + 1, &mut tagbuf);
            o.byte(b'[');
            for &d in &tagbuf[..digits] { o.byte(d); }
            o.byte(b']');
            o.byte(b' ');
            o.space = false;
            o.newlines = 0;
            for &c in &html[start..start + len] { o.text(c); }
        }
    }
    let (len, truncated) = o.finish();
    Converted { len, links: link_count, truncated }
}

fn number(mut n: usize, out: &mut [u8; 6]) -> usize {
    let mut tmp = [0u8; 6];
    let mut len = 0;
    loop { tmp[len] = b'0' + (n % 10) as u8; len += 1; n /= 10; if n == 0 || len == 6 { break; } }
    for i in 0..len { out[i] = tmp[len - 1 - i]; }
    len
}

/// Whether bytes declare themselves as an HTML document.
pub fn looks_like_html(bytes: &[u8]) -> bool {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(bytes.len());
    let head = &bytes[start..bytes.len().min(start + 15)];
    head.len() >= 5 && (name_is(&head[..head.len().min(14)], b"<!doctype html") || name_is(&head[..5], b"<html"))
}
