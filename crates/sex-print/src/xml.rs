// SPDX-License-Identifier: GPL-3.0-only
//! The XML eSCL needs: elements, text and the five predefined entities. Namespace
//! prefixes are dropped (`scan:ColorMode` is `ColorMode`), attributes are skipped,
//! and declarations, comments and CDATA are handled. Small and strict enough for
//! scanner capability documents; not a general XML library.
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    /// Local name, without the namespace prefix.
    pub name: String,
    pub text: String,
    pub children: Vec<Element>,
}

impl Element {
    pub fn child(&self, name: &str) -> Option<&Element> { self.children.iter().find(|c| c.name == name) }
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    /// The element at a path of child names.
    pub fn path(&self, names: &[&str]) -> Option<&Element> {
        names.iter().try_fold(self, |e, n| e.child(n))
    }
    /// Every descendant (depth first) with this name.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for c in &self.children {
            if c.name == name { out.push(c); }
            c.find_all(name, out);
        }
    }
    pub fn text_of(&self, name: &str) -> Option<&str> { self.child(name).map(|c| c.text.trim()) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error;

fn local(name: &str) -> &str { name.rsplit(':').next().unwrap_or(name) }

fn unescape(s: &str, out: &mut String) {
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let Some(end) = after.find(';') else { out.push_str(after); return };
        let ent = &after[1..end];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let n = ent.strip_prefix("#x").and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()));
                match n.and_then(char::from_u32) { Some(c) => out.push(c), None => out.push_str(&after[..=end]) }
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
}

/// The document's root element.
pub fn parse(doc: &str) -> Result<Element, Error> {
    let mut stack: Vec<Element> = Vec::new();
    let mut root = None;
    let mut rest = doc;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if let Some(top) = stack.last_mut() { unescape(rest, &mut top.text); }
            break;
        };
        if lt > 0 {
            if let Some(top) = stack.last_mut() { unescape(&rest[..lt], &mut top.text); }
        }
        rest = &rest[lt..];
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = &r[r.find("-->").ok_or(Error)? + 3..];
        } else if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let end = r.find("]]>").ok_or(Error)?;
            if let Some(top) = stack.last_mut() { top.text.push_str(&r[..end]); }
            rest = &r[end + 3..];
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = &rest[rest.find('>').ok_or(Error)? + 1..];
        } else if let Some(r) = rest.strip_prefix("</") {
            let end = r.find('>').ok_or(Error)?;
            let e = stack.pop().ok_or(Error)?;
            if e.name != local(r[..end].trim()) { return Err(Error); }
            match stack.last_mut() { Some(p) => p.children.push(e), None => root = Some(e) }
            rest = &r[end + 1..];
        } else {
            // A start tag; attribute values may hold '>' inside quotes.
            let b = rest.as_bytes();
            let mut i = 1;
            let mut quote = 0u8;
            while i < b.len() {
                match b[i] {
                    q @ (b'"' | b'\'') if quote == 0 => quote = q,
                    q if q == quote => quote = 0,
                    b'>' if quote == 0 => break,
                    _ => {}
                }
                i += 1;
            }
            if i >= b.len() { return Err(Error); }
            let inner = &rest[1..i];
            let empty = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let name = inner.split(|c: char| c.is_whitespace()).next().unwrap_or("");
            if name.is_empty() { return Err(Error); }
            let e = Element { name: local(name).to_string(), ..Default::default() };
            if empty {
                match stack.last_mut() { Some(p) => p.children.push(e), None => root = Some(e) }
            } else {
                stack.push(e);
            }
            rest = &rest[i + 1..];
        }
        if root.is_some() && stack.is_empty() { break; }
    }
    if !stack.is_empty() { return Err(Error); }
    root.ok_or(Error)
}

/// Text escaped for an element body.
pub fn escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c { '&' => o.push_str("&amp;"), '<' => o.push_str("&lt;"), '>' => o.push_str("&gt;"), c => o.push(c) }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_document() {
        let e = parse("<?xml version=\"1.0\"?><!-- c --><a:Root xmlns:a=\"x>y\"><a:B>1 &amp; 2</a:B><C/><![CDATA[<z>]]></a:Root>").unwrap();
        assert_eq!(e.name, "Root");
        assert_eq!(e.text_of("B"), Some("1 & 2"));
        assert!(e.child("C").is_some());
        assert_eq!(e.text, "<z>");
        assert!(parse("<a><b></a>").is_err());
    }
}
