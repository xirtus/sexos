// SPDX-License-Identifier: GPL-3.0-only
//! A small XML reader for WebDAV replies: elements with resolved namespaces, their
//! attributes and text. It reads what servers send (declarations, comments,
//! processing instructions, CDATA, the five named entities and character
//! references) and refuses what it cannot read rather than guessing. A DOCTYPE is
//! refused, so no entity can expand. Depth and size are bounded by the caller's body
//! limit and [`MAX_DEPTH`].
use alloc::string::String;
use alloc::vec::Vec;

pub const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Syntax, Unbalanced, Doctype, TooDeep }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node { Element(Element), Text(String) }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    /// The namespace URI ("" when none) and the local name.
    pub ns: String,
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn is(&self, ns: &str, name: &str) -> bool { self.ns == ns && self.name == name }
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n { Node::Element(e) => Some(e), _ => None })
    }
    /// The first child element named so.
    pub fn child(&self, ns: &str, name: &str) -> Option<&Element> { self.elements().find(|e| e.is(ns, name)) }
    pub fn children_named<'a>(&'a self, ns: &'a str, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.is(ns, name))
    }
    /// The first element named so at any depth below this one.
    pub fn find(&self, ns: &str, name: &str) -> Option<&Element> {
        for e in self.elements() {
            if e.is(ns, name) { return Some(e); }
            if let Some(f) = e.find(ns, name) { return Some(f); }
        }
        None
    }
    /// All the text inside, children's included.
    pub fn text(&self) -> String {
        let mut out = String::new();
        fn walk(e: &Element, out: &mut String) {
            for n in &e.children { match n { Node::Text(t) => out.push_str(t), Node::Element(c) => walk(c, out) } }
        }
        walk(self, &mut out);
        out
    }
}

/// Text with `&`, `<`, `>` and `"` escaped, for element text and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c { '&' => out.push_str("&amp;"), '<' => out.push_str("&lt;"), '>' => out.push_str("&gt;"), '"' => out.push_str("&quot;"), c => out.push(c) }
    }
    out
}

fn decode(raw: &str) -> Result<String, Error> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let end = rest[i..].find(';').ok_or(Error::Syntax)? + i;
        let name = &rest[i + 1..end];
        match name {
            "amp" => out.push('&'), "lt" => out.push('<'), "gt" => out.push('>'), "quot" => out.push('"'), "apos" => out.push('\''),
            n if n.starts_with("#x") || n.starts_with("#X") => out.push(u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32).ok_or(Error::Syntax)?),
            n if n.starts_with('#') => out.push(n[1..].parse::<u32>().ok().and_then(char::from_u32).ok_or(Error::Syntax)?),
            _ => return Err(Error::Syntax),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// An element as read, before its prefixes are resolved.
struct Open { prefixed: String, attrs: Vec<(String, String)>, scope: Vec<(String, String)>, children: Vec<Node> }

fn resolve(scopes: &[Open], extra: &[(String, String)], prefix: &str) -> Option<String> {
    for (p, uri) in extra.iter().rev() { if p == prefix { return Some(uri.clone()); } }
    for o in scopes.iter().rev() {
        for (p, uri) in o.scope.iter().rev() { if p == prefix { return Some(uri.clone()); } }
    }
    match prefix { "" => Some(String::new()), "xml" => Some("http://www.w3.org/XML/1998/namespace".into()), _ => None }
}

fn split(name: &str) -> (&str, &str) { name.split_once(':').unwrap_or(("", name)) }

/// Reads a document and returns its root element.
pub fn parse(text: &str) -> Result<Element, Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut stack: Vec<Open> = Vec::new();
    let mut root: Option<Element> = None;
    let mut i = 0;
    let b = text.as_bytes();
    while i < b.len() {
        if b[i] != b'<' {
            let end = text[i..].find('<').map_or(b.len(), |j| i + j);
            let t = decode(&text[i..end])?;
            match stack.last_mut() {
                Some(o) => if !t.is_empty() { o.children.push(Node::Text(t)) },
                None => if !t.trim().is_empty() { return Err(Error::Syntax) },
            }
            i = end;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<?") {
            i += rest.find("?>").ok_or(Error::Syntax)? + 2;
        } else if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or(Error::Syntax)? + 3;
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").ok_or(Error::Syntax)?;
            stack.last_mut().ok_or(Error::Syntax)?.children.push(Node::Text(String::from(&rest[9..end])));
            i += end + 3;
        } else if rest.starts_with("<!") {
            return Err(Error::Doctype);
        } else if rest.starts_with("</") {
            let end = rest.find('>').ok_or(Error::Syntax)?;
            let name = rest[2..end].trim();
            let open = stack.pop().ok_or(Error::Unbalanced)?;
            if open.prefixed != name { return Err(Error::Unbalanced); }
            let e = close(&stack, open)?;
            match stack.last_mut() { Some(p) => p.children.push(Node::Element(e)), None => { if root.is_some() { return Err(Error::Syntax); } root = Some(e) } }
            i += end + 1;
        } else {
            // A start tag: name, attributes, then `>` or `/>`, quotes respected.
            let mut j = 1;
            let mut quote = 0u8;
            while j < rest.len() {
                let c = rest.as_bytes()[j];
                if quote != 0 { if c == quote { quote = 0; } } else if c == b'"' || c == b'\'' { quote = c; } else if c == b'>' { break; }
                j += 1;
            }
            if j >= rest.len() { return Err(Error::Syntax); }
            let inner = &rest[1..j];
            let (inner, empty) = match inner.strip_suffix('/') { Some(s) => (s, true), None => (inner, false) };
            let name_end = inner.find(|c: char| c.is_ascii_whitespace()).unwrap_or(inner.len());
            let prefixed = String::from(&inner[..name_end]);
            if prefixed.is_empty() { return Err(Error::Syntax); }
            let mut attrs = Vec::new();
            let mut scope = Vec::new();
            let mut a = inner[name_end..].trim_start();
            while !a.is_empty() {
                let eq = a.find('=').ok_or(Error::Syntax)?;
                let key = a[..eq].trim();
                let v = a[eq + 1..].trim_start();
                let q = *v.as_bytes().first().ok_or(Error::Syntax)?;
                if q != b'"' && q != b'\'' { return Err(Error::Syntax); }
                let close_at = v[1..].find(q as char).ok_or(Error::Syntax)? + 1;
                let value = decode(&v[1..close_at])?;
                if key == "xmlns" { scope.push((String::new(), value)); }
                else if let Some(p) = key.strip_prefix("xmlns:") { scope.push((String::from(p), value)); }
                else { attrs.push((String::from(key), value)); }
                a = v[close_at + 1..].trim_start();
            }
            if stack.len() >= MAX_DEPTH { return Err(Error::TooDeep); }
            let open = Open { prefixed, attrs, scope, children: Vec::new() };
            if empty {
                let e = close(&stack, open)?;
                match stack.last_mut() { Some(p) => p.children.push(Node::Element(e)), None => { if root.is_some() { return Err(Error::Syntax); } root = Some(e) } }
            } else {
                stack.push(open);
            }
            i += j + 1;
        }
    }
    if !stack.is_empty() { return Err(Error::Unbalanced); }
    root.ok_or(Error::Syntax)
}

fn close(stack: &[Open], open: Open) -> Result<Element, Error> {
    let (prefix, name) = split(&open.prefixed);
    let ns = resolve(stack, &open.scope, prefix).ok_or(Error::Syntax)?;
    let attrs = open.attrs.into_iter().map(|(k, v)| (String::from(split(&k).1), v)).collect();
    Ok(Element { ns, name: String::from(name), attrs, children: open.children })
}

#[cfg(test)]
mod boundary_tests {
    /// Server replies are untrusted: no prefix of odd, multibyte markup may panic
    /// (the bug fuzzing found in sex-xml, issue #261).
    #[test]
    fn multibyte_and_stray_quotes_never_panic() {
        let bytes: &[u8] = &[38, 60, 75, 10, 61, 34, 34, 255];
        let _ = super::parse(&alloc::string::String::from_utf8_lossy(bytes));
        let messy = "<d:a =\"\"\"é ='x'ü \"ä\" b=ö xmlns:d=\"DAV:\"/>текст<d:p 日本=\"語\" \"\"\"€>x</d:p><![CDATA[ü]]><!--é-->&#xé;&é;";
        for (n, _) in messy.char_indices() { let _ = super::parse(&messy[..n]); }
        let _ = super::parse(messy);
    }
}
