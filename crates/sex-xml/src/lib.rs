// SPDX-License-Identifier: GPL-3.0-only
//! A small, forgiving XML reader for the documents SexOS readers meet: EPUB packages
//! and chapters (XHTML), FictionBook, RSS, Atom and OPML. It builds a tree of
//! [`Node`]s, decodes entities (the XML five, numeric ones and the common HTML names),
//! keeps CDATA verbatim, skips comments, processing instructions and the doctype, and
//! never fails: a stray end tag is dropped, an unclosed element is closed at the end,
//! and HTML's void elements (`br`, `img`, ...) need no `/`.
//!
//! Names are kept as written (`dc:title`); [`Node::is`] and [`Node::attr`] match a
//! local name too, so callers need not care which prefix a file chose.
//!
//! [`html`] turns XHTML or tag soup into paragraphs and headings of plain text.
#![no_std]

extern crate alloc;

pub mod html;

use alloc::string::String;
use alloc::vec::Vec;

/// One element.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    /// The name as written, prefix included.
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Child>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Child {
    Element(Node),
    Text(String),
}

/// The part of `name` after any `prefix:`.
pub fn local(name: &str) -> &str { name.rsplit(':').next().unwrap_or(name) }

impl Node {
    /// True when the element's name, or its local name, is `name` (ASCII case-insensitive).
    pub fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name) || local(&self.name).eq_ignore_ascii_case(name)
    }
    /// The attribute `name`: an exact match first, then one whose local name matches.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name))
            .or_else(|| self.attrs.iter().find(|(k, _)| local(k).eq_ignore_ascii_case(local(name))))
            .map(|(_, v)| v.as_str())
    }
    /// The element children.
    pub fn elements(&self) -> impl Iterator<Item = &Node> {
        self.children.iter().filter_map(|c| match c { Child::Element(e) => Some(e), _ => None })
    }
    /// The first element child called `name`.
    pub fn child(&self, name: &str) -> Option<&Node> { self.elements().find(|e| e.is(name)) }
    /// Every element child called `name`.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.elements().filter(move |e| e.is(name))
    }
    /// The first element called `name` at any depth below this one, in document order.
    pub fn find(&self, name: &str) -> Option<&Node> {
        for e in self.elements() {
            if e.is(name) { return Some(e); }
            if let Some(f) = e.find(name) { return Some(f); }
        }
        None
    }
    /// Every element called `name` below this one, in document order (not below a match).
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        for e in self.elements() {
            if e.is(name) { out.push(e); } else { e.find_all(name, out); }
        }
    }
    /// All the text inside, concatenated.
    pub fn text(&self) -> String {
        let mut s = String::new();
        self.text_into(&mut s);
        s
    }
    fn text_into(&self, s: &mut String) {
        for c in &self.children {
            match c { Child::Text(t) => s.push_str(t), Child::Element(e) => e.text_into(s) }
        }
    }
    /// The text inside with runs of white space made one space and the ends trimmed.
    pub fn text_trim(&self) -> String { collapse(&self.text()) }
    /// The trimmed text of the child `name`, or "".
    pub fn child_text(&self, name: &str) -> String { self.child(name).map(Node::text_trim).unwrap_or_default() }
}

/// White space runs made one space; the ends trimmed.
pub fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split(|c: char| c.is_whitespace()).filter(|w| !w.is_empty()) {
        if !out.is_empty() { out.push(' '); }
        out.push_str(word);
    }
    out
}

/// HTML's elements without content. `link` and `meta` only with attributes: RSS has a
/// `<link>` holding text.
const VOID: &[&str] = &["br", "img", "hr", "input", "area", "base", "col", "embed", "wbr"];

/// Reads `text` into a tree. The returned node is a nameless root holding the
/// document's top-level elements (normally one).
pub fn parse(text: &str) -> Node {
    let b = text.as_bytes();
    let mut stack: Vec<Node> = Vec::new();
    stack.push(Node::default());
    let mut i = 0;
    let push_text = |stack: &mut Vec<Node>, t: String| {
        if t.is_empty() { return; }
        let top = stack.last_mut().unwrap();
        if let Some(Child::Text(prev)) = top.children.last_mut() { prev.push_str(&t); } else { top.children.push(Child::Text(t)); }
    };
    while i < b.len() {
        if b[i] != b'<' {
            let end = memchr(b'<', &b[i..]).map_or(b.len(), |n| i + n);
            push_text(&mut stack, decode(&text[i..end]));
            i = end;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map_or(b.len(), |n| i + n + 3);
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").map_or(rest.len(), |n| n);
            push_text(&mut stack, String::from(&rest[9..end.max(9)]));
            i += (end + 3).min(rest.len());
        } else if rest.starts_with("<?") {
            i = rest.find("?>").map_or(b.len(), |n| i + n + 2);
        } else if rest.starts_with("<!") {
            // A doctype, perhaps with an internal subset in brackets.
            let mut depth = 0i32;
            let mut j = 2;
            let rb = rest.as_bytes();
            while j < rb.len() {
                match rb[j] { b'[' => depth += 1, b']' => depth -= 1, b'>' if depth <= 0 => break, _ => {} }
                j += 1;
            }
            i += (j + 1).min(rest.len());
        } else if rest.starts_with("</") {
            let end = memchr(b'>', &b[i..]).map_or(b.len(), |n| i + n);
            let name = text[i + 2..end].trim();
            i = (end + 1).min(b.len());
            // Close up to the matching element, if it is open at all.
            if let Some(at) = stack.iter().rposition(|n| n.name.eq_ignore_ascii_case(name)).filter(|at| *at > 0) {
                while stack.len() > at {
                    let done = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(Child::Element(done));
                }
            }
        } else {
            // A start tag: its name, attributes (quotes may hold `>`), and `/>`.
            let mut j = i + 1;
            while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' { j += 1; }
            let name = &text[i + 1..j];
            if name.is_empty() || !(b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_' || b[i + 1] >= 0x80) {
                push_text(&mut stack, String::from("<"));
                i += 1;
                continue;
            }
            let mut attrs = Vec::new();
            let mut empty = false;
            loop {
                while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
                if j >= b.len() { break; }
                if b[j] == b'>' { j += 1; break; }
                if b[j] == b'/' { empty = true; j += 1; continue; }
                let k0 = j;
                while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'=' && b[j] != b'>' && b[j] != b'/' { j += 1; }
                let key = &text[k0..j];
                while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
                let mut value = String::new();
                if j < b.len() && b[j] == b'=' {
                    j += 1;
                    while j < b.len() && b[j].is_ascii_whitespace() { j += 1; }
                    if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                        let q = b[j];
                        let v0 = j + 1;
                        let v1 = memchr(q, &b[v0..]).map_or(b.len(), |n| v0 + n);
                        value = decode(&text[v0..v1]);
                        j = (v1 + 1).min(b.len());
                    } else {
                        let v0 = j;
                        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' { j += 1; }
                        value = decode(&text[v0..j]);
                    }
                }
                if !key.is_empty() {
                    empty = false;
                    attrs.push((String::from(key), value));
                } else {
                    // Skip the stray character whole: stepping one byte could land inside
                    // a UTF-8 sequence and the next slice would panic (issue #261).
                    j += text[j.min(text.len())..].chars().next().map_or(1, char::len_utf8);
                }
            }
            i = j;
            let node = Node { name: String::from(name), attrs, children: Vec::new() };
            let lower = local(name);
            let void = VOID.iter().any(|v| v.eq_ignore_ascii_case(lower))
                || (!node.attrs.is_empty() && (lower.eq_ignore_ascii_case("link") || lower.eq_ignore_ascii_case("meta")));
            if empty || void {
                stack.last_mut().unwrap().children.push(Child::Element(node));
            } else if lower.eq_ignore_ascii_case("script") || lower.eq_ignore_ascii_case("style") {
                // Raw text up to the end tag, kept so a caller may look but never parsed.
                let close = alloc::format!("</{name}");
                let end = find_ci(&text[i..], &close).map_or(b.len(), |n| i + n);
                let mut node = node;
                if end > i { node.children.push(Child::Text(String::from(&text[i..end]))); }
                stack.last_mut().unwrap().children.push(Child::Element(node));
                i = memchr(b'>', &b[end.min(b.len())..]).map_or(b.len(), |n| end + n + 1);
            } else {
                stack.push(node);
            }
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(Child::Element(done));
    }
    stack.pop().unwrap()
}

fn memchr(c: u8, b: &[u8]) -> Option<usize> { b.iter().position(|x| *x == c) }
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.len() > h.len() { return None; }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// Entities replaced by their characters; an unknown one stays as written.
pub fn decode(s: &str) -> String {
    if !s.contains('&') { return String::from(s); }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest[1..].find(|c: char| c == ';' || c == '&' || c.is_whitespace() || c == '<').map(|n| n + 1);
        match end.filter(|e| rest.as_bytes().get(*e) == Some(&b';')) {
            Some(e) => {
                match entity(&rest[1..e]) {
                    Some(c) => { out.push(c); rest = &rest[e + 1..]; }
                    None => { out.push('&'); rest = &rest[1..]; }
                }
            }
            None => { out.push('&'); rest = &rest[1..]; }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let v = match num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse::<u32>().ok()?,
        };
        return Some(char::from_u32(v).filter(|c| *c != '\0').unwrap_or('\u{FFFD}'));
    }
    Some(match name {
        "amp" => '&', "lt" => '<', "gt" => '>', "quot" => '"', "apos" => '\'',
        "nbsp" => '\u{A0}', "shy" => '\u{AD}', "copy" => '\u{A9}', "reg" => '\u{AE}', "trade" => '\u{2122}',
        "mdash" => '\u{2014}', "ndash" => '\u{2013}', "hellip" => '\u{2026}', "bull" => '\u{2022}', "middot" => '\u{B7}',
        "lsquo" => '\u{2018}', "rsquo" => '\u{2019}', "ldquo" => '\u{201C}', "rdquo" => '\u{201D}', "sbquo" => '\u{201A}', "bdquo" => '\u{201E}',
        "laquo" => '\u{AB}', "raquo" => '\u{BB}', "lsaquo" => '\u{2039}', "rsaquo" => '\u{203A}',
        "deg" => '\u{B0}', "times" => '\u{D7}', "divide" => '\u{F7}', "minus" => '\u{2212}', "plusmn" => '\u{B1}',
        "euro" => '\u{20AC}', "pound" => '\u{A3}', "yen" => '\u{A5}', "cent" => '\u{A2}', "sect" => '\u{A7}', "para" => '\u{B6}',
        "eacute" => '\u{E9}', "egrave" => '\u{E8}', "ecirc" => '\u{EA}', "euml" => '\u{EB}', "aacute" => '\u{E1}', "agrave" => '\u{E0}',
        "acirc" => '\u{E2}', "auml" => '\u{E4}', "aring" => '\u{E5}', "ccedil" => '\u{E7}', "iacute" => '\u{ED}', "iuml" => '\u{EF}',
        "oacute" => '\u{F3}', "ocirc" => '\u{F4}', "ouml" => '\u{F6}', "uacute" => '\u{FA}', "uuml" => '\u{FC}', "ntilde" => '\u{F1}',
        "szlig" => '\u{DF}', "Auml" => '\u{C4}', "Ouml" => '\u{D6}', "Uuml" => '\u{DC}', "Eacute" => '\u{C9}', "thinsp" => '\u{2009}',
        "ensp" => '\u{2002}', "emsp" => '\u{2003}', "zwnj" => '\u{200C}', "zwj" => '\u{200D}',
        _ => return None,
    })
}

/// A document's bytes as text: UTF-16 by its byte-order mark, windows-1251 or
/// ISO-8859-1 when the XML declaration says so, otherwise UTF-8 (bad bytes replaced).
pub fn text_of(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) { return String::from_utf8_lossy(rest).into_owned(); }
    let utf16 = |rest: &[u8], be: bool| {
        let units = rest.chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) });
        char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect::<String>()
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) { return utf16(rest, false); }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) { return utf16(rest, true); }
    let head = &bytes[..bytes.len().min(200)];
    let decl_end = head.windows(2).position(|w| w == b"?>").unwrap_or(0);
    let decl = String::from_utf8_lossy(&head[..decl_end]).to_ascii_lowercase();
    let encoding = decl.split("encoding=").nth(1).map(|v| v.trim_start_matches(['"', '\'']).split(['"', '\'']).next().unwrap_or("")).unwrap_or("");
    match encoding {
        "windows-1251" | "cp1251" => bytes.iter().map(|&b| cp1251(b)).collect(),
        "iso-8859-1" | "latin1" | "latin-1" | "windows-1252" => bytes.iter().map(|&b| char::from(b)).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn cp1251(b: u8) -> char {
    const HIGH: [u16; 64] = [
        0x0402, 0x0403, 0x201A, 0x0453, 0x201E, 0x2026, 0x2020, 0x2021, 0x20AC, 0x2030, 0x0409, 0x2039, 0x040A, 0x040C, 0x040B, 0x040F,
        0x0452, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0xFFFD, 0x2122, 0x0459, 0x203A, 0x045A, 0x045C, 0x045B, 0x045F,
        0x00A0, 0x040E, 0x045E, 0x0408, 0x00A4, 0x0490, 0x00A6, 0x00A7, 0x0401, 0x00A9, 0x0404, 0x00AB, 0x00AC, 0x00AD, 0x00AE, 0x0407,
        0x00B0, 0x00B1, 0x0406, 0x0456, 0x0491, 0x00B5, 0x00B6, 0x00B7, 0x0451, 0x2116, 0x0454, 0x00BB, 0x0458, 0x0405, 0x0455, 0x0457,
    ];
    match b {
        0..=0x7F => char::from(b),
        0x80..=0xBF => char::from_u32(HIGH[(b - 0x80) as usize] as u32).unwrap_or('\u{FFFD}'),
        _ => char::from_u32(0x0410 + (b - 0xC0) as u32).unwrap_or('\u{FFFD}'),
    }
}

/// `s` with `&`, `<`, `>`, `"` written as entities, for attribute values and text.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c { '&' => out.push_str("&amp;"), '<' => out.push_str("&lt;"), '>' => out.push_str("&gt;"), '"' => out.push_str("&quot;"), _ => out.push(c) }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_entities_and_cdata() {
        let doc = parse("<?xml version='1.0'?><!DOCTYPE x [<!ENTITY a 'b'>]><!-- hi --><a x=\"1 &amp; 2\" y='q>'><dc:b>t&lt;&#65;&#x42;&mdash;&bogus;</dc:b><c/><![CDATA[<raw>&amp;]]></a>");
        let a = doc.child("a").unwrap();
        assert_eq!(a.attr("x"), Some("1 & 2"));
        assert_eq!(a.attr("y"), Some("q>"));
        assert_eq!(a.child("b").unwrap().text(), "t<AB\u{2014}&bogus;");
        assert!(a.child("c").unwrap().children.is_empty());
        assert_eq!(a.text(), "t<AB\u{2014}&bogus;<raw>&amp;");
    }

    #[test]
    fn forgiving() {
        let doc = parse("<p>one<br>two</b><img src=x alt=\"pic\">three<p>four");
        let p = doc.child("p").unwrap();
        assert_eq!(p.text(), "onetwothreefour");
        assert!(p.find("img").is_some());
        assert_eq!(p.find("img").unwrap().attr("src"), Some("x"));
        let rss = parse("<channel><link>http://a/</link><link rel=x href=y><title>t</title></channel>");
        let ch = rss.child("channel").unwrap();
        assert_eq!(ch.child("link").unwrap().text(), "http://a/");
        assert_eq!(ch.child("title").unwrap().text(), "t");
        let s = parse("<script>if (a<b) x='</p>'</script><p>z</p>");
        assert_eq!(s.child("p").unwrap().text(), "z");
    }

    #[test]
    fn prefixed_attrs_and_escape() {
        let doc = parse("<nav epub:type=\"toc\" xml:lang=en></nav>");
        let n = doc.child("nav").unwrap();
        assert_eq!(n.attr("epub:type"), Some("toc"));
        assert_eq!(n.attr("type"), Some("toc"));
        assert_eq!(escape("a<b & \"c\""), "a&lt;b &amp; &quot;c&quot;");
        assert_eq!(collapse("  a \n\t b  "), "a b");
    }

    #[test]
    fn encodings() {
        assert_eq!(text_of(b"\xEF\xBB\xBFhi"), "hi");
        assert_eq!(text_of(&[0xFF, 0xFE, b'h', 0, b'i', 0]), "hi");
        assert_eq!(text_of(b"<?xml version=\"1.0\" encoding=\"windows-1251\"?>\xCF\xF0\xE8\xE2\xE5\xF2"), "<?xml version=\"1.0\" encoding=\"windows-1251\"?>\u{41F}\u{440}\u{438}\u{432}\u{435}\u{442}");
        assert_eq!(text_of(b"<?xml encoding='ISO-8859-1'?>caf\xE9"), "<?xml encoding='ISO-8859-1'?>caf\u{E9}");
    }
    #[test]
    fn empty_attribute_key_before_multibyte_text() {
        // Issue #261: `="""` then a non-ASCII character used to panic on a char boundary.
        let bytes: &[u8] = &[38, 60, 75, 10, 61, 34, 34, 255];
        let text = alloc::string::String::from_utf8_lossy(bytes);
        let _ = parse(&text);
        // Every prefix of markup full of multibyte characters and stray quotes parses.
        let messy = "<a =\"\"\"é ='x'ü \"ä\" b=ö/>текст<p 日本=\"語\" \"\"\"€>x</p>";
        for (n, _) in messy.char_indices() { let _ = parse(&messy[..n]); }
        let _ = parse(messy);
    }

}
