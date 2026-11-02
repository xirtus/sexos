// SPDX-License-Identifier: GPL-3.0-only
//! Bounded HTML-to-readable-text conversion for the Browser reader.
//!
//! This is a reading view, not a web engine: there is no DOM, CSS, layout,
//! network fetch or form. Markup becomes plain UTF-8 text:
//! headings and block elements start new lines, list items get a bullet,
//! links keep their text plus a numbered reference listed at the end, and
//! images show their alt text, inline scripts run only through the bounded
//! subset in `script.rs` (their writes become text), and `style`, `head`,
//! `template` and comments contribute nothing. The
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
    /// Inside `<pre>`: whitespace is kept as written (tabs become spaces).
    pre: bool,
    /// `<blockquote>` nesting; each line inside starts with `> ` per level.
    quote: u8,
}

impl<'a> Output<'a> {
    fn new(buf: &'a mut [u8]) -> Self { Self { buf, len: 0, truncated: false, space: true, newlines: 2, pre: false, quote: 0 } }
    fn room(&self) -> usize { self.buf.len().saturating_sub(TRUNCATED.len()) }
    fn byte(&mut self, b: u8) {
        if self.len < self.room() { self.buf[self.len] = b; self.len += 1; } else { self.truncated = true; }
    }
    fn text(&mut self, b: u8) {
        if self.pre {
            match b {
                b'\r' => {}
                b'\n' => { self.byte(b'\n'); self.newlines = self.newlines.saturating_add(1); }
                b'\t' | 0x0c => { self.byte(b' '); self.newlines = 0; }
                _ => { self.byte(b); self.newlines = 0; }
            }
            self.space = false;
            return;
        }
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c) {
            self.space = true;
            return;
        }
        if self.space && self.newlines == 0 { self.byte(b' '); }
        if self.newlines != 0 { for _ in 0..self.quote { self.byte(b'>'); self.byte(b' '); } }
        self.space = false;
        self.newlines = 0;
        self.byte(b);
    }
    fn bytes(&mut self, s: &[u8]) { for &b in s { self.text(b); } }
    /// Literal bytes at the start of a line (indentation, list markers).
    fn raw(&mut self, s: &[u8]) {
        if self.newlines != 0 { for _ in 0..self.quote { self.byte(b'>'); self.byte(b' '); } }
        for &b in s { self.byte(b); }
        self.newlines = 0;
        self.space = false;
    }
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
pub struct Converted {
    pub len: usize, pub links: usize, pub truncated: bool,
    /// Output ranges of each listed link target; empty when the list was cut.
    pub targets: [(usize, usize); MAX_LINKS],
    /// Inline scripts run by the bounded subset, and scripts refused
    /// (external `src`, other types, or outside the subset).
    pub scripts_run: u8,
    pub scripts_refused: u8,
}

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

const SKIP: [&[u8]; 5] = [b"style", b"head", b"template", b"noscript", b"svg"];
const BLOCK: [&[u8]; 15] = [b"p", b"div", b"section", b"article", b"header", b"footer", b"main", b"nav", b"ul", b"ol", b"table", b"blockquote", b"pre", b"hr", b"dl"];

/// Convert `html` into readable text in `out`. Link targets are appended as a
/// numbered list; at most MAX_LINKS are kept.
pub fn convert(html: &[u8], out: &mut [u8]) -> Converted {
    let mut o = Output::new(out);
    let mut links: [(usize, usize); MAX_LINKS] = [(0, 0); MAX_LINKS];
    let mut link_count = 0usize;
    let mut open_link: Option<usize> = None;
    let mut skip_until: Option<&[u8]> = None;
    let (mut scripts_run, mut scripts_refused) = (0u8, 0u8);
    // Open lists: 0 = bulleted, n = next number of an ordered list.
    let mut lists = [0u16; 4];
    let mut depth = 0usize;
    if let Some(title) = title(html) {
        o.bytes(b"Title:");
        o.space = true;
        let mut buf = [0u8; 4];
        let mut k = 0;
        while k < title.len() {
            if title[k] == b'&' { if let Some((n, used)) = entity(&title[k + 1..], &mut buf) { for &c in &buf[..n] { o.text(c); } k += 1 + used; continue; } }
            o.text(title[k]); k += 1;
        }
        o.line(true);
    }
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
            if !end_tag && name_is(name, b"script") {
                // Find the raw end tag; script text is never markup.
                let rest = &html[i..];
                let close = rest.windows(9).position(|w| name_is(w, b"</script>"));
                let source = &rest[..close.unwrap_or(rest.len())];
                i += close.map_or(rest.len(), |c| c + 9);
                let attrs = &body[name_end..];
                let plain = attribute(attrs, b"src").is_none()
                    && attribute(attrs, b"type").is_none_or(|t| name_is(t, b"text/javascript"));
                match plain.then(|| crate::script::run(source)) {
                    Some(Ok(written)) => { scripts_run = scripts_run.saturating_add(1); for &c in &written { o.text(c); } }
                    _ => scripts_refused = scripts_refused.saturating_add(1),
                }
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
                for _ in 1..depth.max(1) { o.raw(b"  "); }
                match lists.get(depth.wrapping_sub(1)).copied().filter(|n| *n > 0) {
                    Some(n) => {
                        let mut digits = [0u8; 6];
                        let len = number(n as usize, &mut digits);
                        o.raw(&digits[..len]);
                        o.raw(b". ");
                        lists[depth - 1] = n.saturating_add(1);
                    }
                    None => o.raw(b"- "),
                }
            } else if (name_is(name, b"dt") || name_is(name, b"dd")) && !end_tag {
                o.line(false);
                if name_is(name, b"dd") { o.raw(b"    "); }
            } else if name_is(name, b"tr") && end_tag {
                o.line(false);
            } else if (name_is(name, b"td") || name_is(name, b"th")) && !end_tag {
                if o.newlines == 0 { o.bytes(b" |"); o.space = true; }
            } else if BLOCK.iter().any(|n| name_is(name, n)) {
                o.pre = false;
                let list = name_is(name, b"ol") || name_is(name, b"ul");
                // A list inside a list item continues that item's lines.
                o.line(!(list && depth > usize::from(end_tag)));
                if name_is(name, b"ol") || name_is(name, b"ul") {
                    if end_tag { depth = depth.saturating_sub(1); }
                    else if depth < lists.len() {
                        let start = attribute(&body[name_end..], b"start")
                            .and_then(|v| core::str::from_utf8(v).ok()?.trim().parse::<u16>().ok()).unwrap_or(1).max(1);
                        lists[depth] = if name_is(name, b"ol") { start } else { 0 };
                        depth += 1;
                    } else { depth += 1; }
                } else if name_is(name, b"blockquote") {
                    o.quote = if end_tag { o.quote.saturating_sub(1) } else { (o.quote + 1).min(4) };
                } else if name_is(name, b"hr") {
                    o.bytes(b"----");
                    o.line(true);
                }
                if name_is(name, b"pre") && !end_tag {
                    o.pre = true;
                    // A newline right after <pre> is not content.
                    if html.get(i) == Some(&b'\n') { i += 1; } else if html[i..].starts_with(b"\r\n") { i += 2; }
                }
            } else if name_is(name, b"img") && !end_tag {
                // Images are not fetched; their alternative text is shown.
                o.bytes(b"[image");
                if let Some(alt) = attribute(&body[name_end..], b"alt").filter(|a| a.iter().any(|b| !b.is_ascii_whitespace())) {
                    o.text(b':');
                    o.space = true;
                    for &c in alt { o.text(c); }
                }
                o.text(b']');
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
    let mut targets = [(0, 0); MAX_LINKS];
    o.pre = false;
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
            let from = o.len;
            for &c in &html[start..start + len] { o.text(c); }
            if !o.truncated { targets[n] = (from, o.len); }
        }
    }
    let (len, truncated) = o.finish();
    Converted { len, links: link_count, truncated, targets, scripts_run, scripts_refused }
}

/// Text of the first `<title>` element, if any (it sits in the skipped head).
fn title(html: &[u8]) -> Option<&[u8]> {
    let open = html.windows(6).position(|w| name_is(&w[..6], b"<title"))?;
    let start = open + html[open..].iter().position(|&b| b == b'>')? + 1;
    let end = start + html[start..].windows(8).position(|w| name_is(w, b"</title>"))?;
    let t = &html[start..end];
    t.iter().any(|b| !b.is_ascii_whitespace()).then_some(t)
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
