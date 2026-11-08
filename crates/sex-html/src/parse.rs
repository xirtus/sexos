// SPDX-License-Identifier: GPL-3.0-only
//! A forgiving HTML parser: a tokenizer plus a tree builder that follows the parts of
//! the HTML5 tree-construction rules ordinary pages lean on (implied `html`/`head`/
//! `body`, void elements, `<p>`/`<li>`/table cells closing themselves, raw text in
//! `script`/`style`/`textarea`/`title`, stray end tags ignored). It never fails: any
//! input becomes some tree.
use crate::dom::{Dom, Kind, NodeId};
use alloc::string::String;
use alloc::vec::Vec;

const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
const RAW: &[&str] = &["script", "style", "textarea", "title", "xmp", "noembed", "noframes"];
const HEAD_ONLY: &[&str] = &["base", "link", "meta", "title", "style", "script", "noscript"];
/// Start tags that close an open `<p>`.
const CLOSES_P: &[&str] = &[
    "address", "article", "aside", "blockquote", "details", "div", "dl", "fieldset", "figcaption", "figure", "footer",
    "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hgroup", "hr", "main", "menu", "nav", "ol", "p", "pre",
    "section", "table", "ul", "li", "dd", "dt", "figure", "center", "dir", "summary", "search",
];

pub fn is_void(tag: &str) -> bool { VOID.contains(&tag) }

pub fn parse(src: &str) -> Dom {
    let mut b = Builder { dom: Dom::new(), stack: Vec::new(), head: None, body: None };
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' { i += 1; continue; }
        let rest = &bytes[i + 1..];
        let starts_tag = rest.first().map_or(false, |c| c.is_ascii_alphabetic() || matches!(c, b'/' | b'!' | b'?'));
        if !starts_tag { i += 1; continue; }
        b.text(&src[text_start..i]);
        if rest.starts_with(b"!--") {
            let end = find(bytes, i + 4, b"-->").map_or(bytes.len(), |e| e + 3);
            i = end;
        } else if rest[0] == b'!' || rest[0] == b'?' {
            i = find(bytes, i, b">").map_or(bytes.len(), |e| e + 1);
        } else if rest[0] == b'/' {
            let (name, end) = tag_name(bytes, i + 2);
            i = find(bytes, end, b">").map_or(bytes.len(), |e| e + 1);
            if !name.is_empty() { b.end(&name); }
        } else {
            let (name, end) = tag_name(bytes, i + 1);
            let (attrs, end, _self_closing) = attributes(src, end);
            i = end;
            if RAW.contains(&name.as_str()) {
                let close = find_close(bytes, i, &name);
                let raw = &src[i..close.0];
                b.start(&name, attrs);
                if !raw.is_empty() {
                    let parent = *b.stack.last().unwrap();
                    if name == "textarea" || name == "title" { b.dom.add_text(parent, &decode(raw)); } else { b.dom.add_text(parent, raw); }
                }
                b.end(&name);
                i = close.1;
            } else {
                b.start(&name, attrs);
            }
        }
        text_start = i;
    }
    b.text(&src[text_start.min(src.len())..]);
    b.ensure_body();
    b.dom
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > hay.len() { return None; }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// Where `</name` begins (ASCII case-insensitive) and where its `>` ends.
fn find_close(bytes: &[u8], from: usize, name: &str) -> (usize, usize) {
    let n = name.len();
    let mut i = from;
    while i + 2 + n <= bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'/' && bytes[i + 2..i + 2 + n].eq_ignore_ascii_case(name.as_bytes()) {
            let after = bytes.get(i + 2 + n).copied();
            if matches!(after, None | Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'/')) {
                let end = find(bytes, i, b">").map_or(bytes.len(), |e| e + 1);
                return (i, end);
            }
        }
        i += 1;
    }
    (bytes.len(), bytes.len())
}

fn tag_name(bytes: &[u8], mut i: usize) -> (String, usize) {
    let mut name = String::new();
    while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'/' | b'>') {
        name.push(bytes[i].to_ascii_lowercase() as char);
        i += 1;
    }
    (name, i)
}

fn attributes(src: &str, mut i: usize) -> (Vec<(String, String)>, usize, bool) {
    let bytes = src.as_bytes();
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;
    loop {
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'\x0c') { i += 1; }
        if i >= bytes.len() { return (attrs, i, self_closing); }
        match bytes[i] {
            b'>' => return (attrs, i + 1, self_closing),
            b'/' => { self_closing = true; i += 1; continue; }
            _ => {}
        }
        self_closing = false;
        let start = i;
        while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'/' | b'>' | b'=') { i += 1; }
        if i == start { i += 1; continue; }
        let name = src[start..i].to_ascii_lowercase();
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') { i += 1; }
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') { i += 1; }
            match bytes.get(i) {
                Some(&q) if q == b'"' || q == b'\'' => {
                    let end = bytes[i + 1..].iter().position(|&c| c == q).map_or(bytes.len(), |p| p + i + 1);
                    value = decode(&src[i + 1..end]);
                    i = (end + 1).min(bytes.len());
                }
                _ => {
                    let start = i;
                    while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'>') { i += 1; }
                    value = decode(&src[start..i]);
                }
            }
        }
        if !attrs.iter().any(|(n, _)| *n == name) { attrs.push((name, value)); }
    }
}

struct Builder {
    dom: Dom,
    stack: Vec<NodeId>,
    head: Option<NodeId>,
    body: Option<NodeId>,
}

impl Builder {
    fn element(&mut self, parent: NodeId, tag: &str, attrs: Vec<(String, String)>) -> NodeId {
        self.dom.add(parent, Kind::Element { tag: String::from(tag), attrs })
    }

    fn html(&mut self) -> NodeId {
        if self.stack.is_empty() {
            let html = self.element(Dom::ROOT, "html", Vec::new());
            self.stack.push(html);
        }
        self.stack[0]
    }

    fn ensure_head(&mut self) -> NodeId {
        if let Some(h) = self.head { return h; }
        let html = self.html();
        let head = self.element(html, "head", Vec::new());
        self.head = Some(head);
        head
    }

    fn ensure_body(&mut self) -> NodeId {
        if let Some(b) = self.body { return b; }
        self.ensure_head();
        let html = self.html();
        let body = self.element(html, "body", Vec::new());
        self.body = Some(body);
        self.stack.truncate(1);
        self.stack.push(body);
        body
    }

    fn current_tag(&self) -> Option<&str> { self.stack.last().and_then(|&n| self.dom.tag(n)) }

    fn text(&mut self, raw: &str) {
        if raw.is_empty() { return; }
        if self.body.is_none() {
            if raw.bytes().all(|c| c.is_ascii_whitespace()) { return; }
            self.ensure_body();
        }
        let parent = *self.stack.last().unwrap();
        if matches!(self.current_tag(), Some("table" | "tbody" | "thead" | "tfoot" | "tr")) && raw.bytes().all(|c| c.is_ascii_whitespace()) { return; }
        self.dom.add_text(parent, &decode(raw));
    }

    fn open(&self, tag: &str) -> Option<usize> {
        self.stack.iter().rposition(|&n| self.dom.tag(n) == Some(tag))
    }

    /// Closes `tag` if it is open without crossing one of `barriers`.
    fn close_within(&mut self, tag: &str, barriers: &[&str]) {
        for pos in (0..self.stack.len()).rev() {
            let t = self.dom.tag(self.stack[pos]).unwrap_or("");
            if t == tag { self.stack.truncate(pos); return; }
            if barriers.contains(&t) { return; }
        }
    }

    fn start(&mut self, tag: &str, attrs: Vec<(String, String)>) {
        match tag {
            "html" => {
                let html = self.html();
                if let Kind::Element { attrs: have, .. } = &mut self.dom.nodes[html].kind {
                    for a in attrs { if !have.iter().any(|(n, _)| *n == a.0) { have.push(a); } }
                }
                return;
            }
            "head" => { if self.body.is_none() { self.ensure_head(); } return; }
            "body" => {
                let body = self.ensure_body();
                if let Kind::Element { attrs: have, .. } = &mut self.dom.nodes[body].kind {
                    for a in attrs { if !have.iter().any(|(n, _)| *n == a.0) { have.push(a); } }
                }
                return;
            }
            _ => {}
        }
        if self.body.is_none() && HEAD_ONLY.contains(&tag) {
            let head = self.ensure_head();
            let id = self.element(head, tag, attrs);
            if !is_void(tag) { self.stack.truncate(1); self.stack.push(head); self.stack.push(id); }
            return;
        }
        if self.body.is_none() { self.ensure_body(); }
        // Head-only elements after the body has started (common) still go in place.
        if self.stack.len() >= 2 && self.dom.tag(self.stack[self.stack.len() - 1]) == Some("head") { self.stack.pop(); self.ensure_body(); }
        const BLOCK_BARRIERS: &[&str] = &["table", "td", "th", "button", "object", "body"];
        if CLOSES_P.contains(&tag) { self.close_within("p", BLOCK_BARRIERS); }
        match tag {
            "li" => { self.close_within("li", &["ul", "ol", "menu", "table", "body"]); }
            "dt" | "dd" => { self.close_within("dt", &["dl", "body"]); self.close_within("dd", &["dl", "body"]); }
            "option" => { self.close_within("option", &["select", "body"]); }
            "tr" => {
                self.close_within("td", &["table"]); self.close_within("th", &["table"]); self.close_within("tr", &["table"]);
                if !matches!(self.current_tag(), Some("tbody" | "thead" | "tfoot" | "table")) && self.open("table").is_none() {}
            }
            "td" | "th" => {
                self.close_within("td", &["tr", "table"]);
                self.close_within("th", &["tr", "table"]);
                if matches!(self.current_tag(), Some("table")) {
                    let parent = *self.stack.last().unwrap();
                    let tbody = self.element(parent, "tbody", Vec::new());
                    self.stack.push(tbody);
                }
                if matches!(self.current_tag(), Some("tbody" | "thead" | "tfoot")) {
                    let parent = *self.stack.last().unwrap();
                    let tr = self.element(parent, "tr", Vec::new());
                    self.stack.push(tr);
                }
            }
            "thead" | "tbody" | "tfoot" => {
                for t in ["td", "th", "tr", "thead", "tbody", "tfoot"] { self.close_within(t, &["table"]); }
            }
            "a" => { self.close_within("a", &["body", "td", "th", "table"]); }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if matches!(self.current_tag(), Some("h1" | "h2" | "h3" | "h4" | "h5" | "h6")) { self.stack.pop(); }
            }
            _ => {}
        }
        if tag == "tr" && matches!(self.current_tag(), Some("table")) {
            let parent = *self.stack.last().unwrap();
            let tbody = self.element(parent, "tbody", Vec::new());
            self.stack.push(tbody);
        }
        let parent = *self.stack.last().unwrap();
        let id = self.element(parent, tag, attrs);
        if !is_void(tag) { self.stack.push(id); }
    }

    fn end(&mut self, tag: &str) {
        match tag {
            "html" | "body" | "head" => {
                if tag == "head" && self.body.is_none() && self.current_tag() == Some("head") { self.stack.pop(); }
                return;
            }
            "br" => { self.start("br", Vec::new()); return; }
            "p" if self.open("p").is_none() => { self.start("p", Vec::new()); }
            _ => {}
        }
        if let Some(pos) = self.open(tag) {
            if pos >= 1 { self.stack.truncate(pos); }
        }
        if self.stack.is_empty() { self.html(); }
    }
}

/// Replaces character references (`&amp;`, `&#38;`, `&#x26;` and the common named ones).
pub fn decode(raw: &str) -> String {
    if !raw.contains('&') { return String::from(raw); }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let (ch, used) = reference(rest);
        match ch {
            Some(c) => { out.push(c); rest = &rest[used..]; }
            None => { out.push('&'); rest = &rest[1..]; }
        }
    }
    out.push_str(rest);
    out
}

fn reference(s: &str) -> (Option<char>, usize) {
    let b = s.as_bytes();
    if b.get(1) == Some(&b'#') {
        let (hex, start) = if matches!(b.get(2), Some(b'x' | b'X')) { (true, 3) } else { (false, 2) };
        let mut end = start;
        while end < b.len() && (if hex { b[end].is_ascii_hexdigit() } else { b[end].is_ascii_digit() }) && end - start < 8 { end += 1; }
        if end == start { return (None, 0); }
        let v = u32::from_str_radix(&s[start..end], if hex { 16 } else { 10 }).unwrap_or(0xFFFD);
        let v = match v { 0 | 0xD800..=0xDFFF => 0xFFFD, 0x80..=0x9F => windows_1252(v), v => v };
        let c = char::from_u32(v).unwrap_or('\u{FFFD}');
        let used = if b.get(end) == Some(&b';') { end + 1 } else { end };
        return (Some(c), used);
    }
    let end = b[1..].iter().position(|c| !c.is_ascii_alphanumeric()).map_or(b.len(), |p| p + 1);
    if end > 33 || end == 1 { return (None, 0); }
    let name = &s[1..end];
    let semi = b.get(end) == Some(&b';');
    match named(name) {
        Some(c) => (Some(c), if semi { end + 1 } else { end }),
        None => (None, 0),
    }
}

fn windows_1252(v: u32) -> u32 {
    const T: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F,
        0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    T[(v - 0x80) as usize]
}

fn named(name: &str) -> Option<char> {
    let c = match name {
        "amp" | "AMP" => '&', "lt" | "LT" => '<', "gt" | "GT" => '>', "quot" | "QUOT" => '"', "apos" => '\'',
        "nbsp" => '\u{A0}', "copy" | "COPY" => '©', "reg" | "REG" => '®', "trade" => '™', "hellip" => '…',
        "mdash" => '—', "ndash" => '–', "lsquo" => '‘', "rsquo" => '’', "ldquo" => '“', "rdquo" => '”',
        "laquo" => '«', "raquo" => '»', "bull" => '•', "middot" => '·', "deg" => '°', "plusmn" => '±',
        "times" => '×', "divide" => '÷', "euro" => '€', "pound" => '£', "yen" => '¥', "cent" => '¢',
        "sect" => '§', "para" => '¶', "larr" => '←', "rarr" => '→', "uarr" => '↑', "darr" => '↓',
        "harr" => '↔', "rArr" => '⇒', "lArr" => '⇐', "hearts" => '♥', "check" => '✓', "star" => '☆',
        "iexcl" => '¡', "iquest" => '¿', "shy" => '\u{AD}', "zwj" => '\u{200D}', "zwnj" => '\u{200C}',
        "thinsp" => '\u{2009}', "ensp" => '\u{2002}', "emsp" => '\u{2003}', "frac12" => '½', "frac14" => '¼',
        "frac34" => '¾', "sup2" => '²', "sup3" => '³', "micro" => 'µ', "le" => '≤', "ge" => '≥', "ne" => '≠',
        "infin" => '∞', "minus" => '−', "prime" => '′', "Prime" => '″', "dagger" => '†', "Dagger" => '‡',
        "permil" => '‰', "alpha" => 'α', "beta" => 'β', "gamma" => 'γ', "delta" => 'δ', "pi" => 'π',
        "sigma" => 'σ', "omega" => 'ω', "lambda" => 'λ', "mu" => 'μ', "theta" => 'θ', "Delta" => 'Δ',
        "Omega" => 'Ω', "Sigma" => 'Σ', "auml" => 'ä', "ouml" => 'ö', "uuml" => 'ü', "Auml" => 'Ä',
        "Ouml" => 'Ö', "Uuml" => 'Ü', "szlig" => 'ß', "eacute" => 'é', "egrave" => 'è', "ecirc" => 'ê',
        "aacute" => 'á', "agrave" => 'à', "acirc" => 'â', "atilde" => 'ã', "aring" => 'å', "aelig" => 'æ',
        "ccedil" => 'ç', "iacute" => 'í', "oacute" => 'ó', "ograve" => 'ò', "ocirc" => 'ô', "otilde" => 'õ',
        "oslash" => 'ø', "uacute" => 'ú', "ntilde" => 'ñ', "Eacute" => 'É', "Ntilde" => 'Ñ', "Ccedil" => 'Ç',
        _ => return None,
    };
    Some(c)
}

