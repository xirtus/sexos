// SPDX-License-Identifier: GPL-3.0-only
//! A small XML reader for Kodi's files (`addon.xml`, `addons.xml`, settings): elements,
//! attributes, text, CDATA and the five named entities plus numeric ones. Comments,
//! processing instructions and a DOCTYPE are skipped. Namespaces are kept as part of
//! the name. Nesting deeper than [`MAX_DEPTH`] is refused, so a hostile repository
//! cannot exhaust the stack.

use alloc::string::String;
use alloc::vec::Vec;

pub const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The document ended inside markup.
    Truncated,
    /// Markup that is not XML, at this byte offset.
    Syntax(usize),
    /// A closing tag that does not match its opening tag, at this byte offset.
    Mismatch(usize),
    TooDeep,
    /// No root element.
    Empty,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> + '_ {
        self.children.iter().filter_map(|n| match n { Node::Element(e) => Some(e), _ => None })
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter_map(move |n| match n { Node::Element(e) if e.name == name => Some(e), _ => None })
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    /// The element's own text (not its children's), trimmed.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for n in &self.children {
            if let Node::Text(t) = n { out.push_str(t); }
        }
        String::from(out.trim())
    }
}

struct Reader<'a> {
    s: &'a str,
    at: usize,
}

impl<'a> Reader<'a> {
    fn rest(&self) -> &'a str { &self.s[self.at..] }
    fn starts(&self, p: &str) -> bool { self.rest().starts_with(p) }
    fn skip_ws(&mut self) {
        let r = self.rest();
        self.at += r.len() - r.trim_start().len();
    }
    /// Moves past `end`, returning what came before it.
    fn until(&mut self, end: &str) -> Result<&'a str, Error> {
        let r = self.rest();
        let i = r.find(end).ok_or(Error::Truncated)?;
        self.at += i + end.len();
        Ok(&r[..i])
    }
    fn name(&mut self) -> Result<&'a str, Error> {
        let r = self.rest();
        let n = r.find(|c: char| c.is_whitespace() || matches!(c, '/' | '>' | '=' | '<')).unwrap_or(r.len());
        if n == 0 { return Err(Error::Syntax(self.at)); }
        self.at += n;
        Ok(&r[..n])
    }
    /// Skips comments, processing instructions and a DOCTYPE. True when it skipped one.
    fn skip_misc(&mut self) -> Result<bool, Error> {
        if self.starts("<!--") { self.at += 4; self.until("-->")?; return Ok(true); }
        if self.starts("<?") { self.at += 2; self.until("?>")?; return Ok(true); }
        if self.starts("<!DOCTYPE") || self.starts("<!doctype") {
            // An internal subset is bracketed; skip it whole.
            let r = self.rest();
            let close = match (r.find('['), r.find('>')) {
                (Some(b), Some(g)) if b < g => r[b..].find("]>").map(|i| b + i + 2),
                (_, g) => g.map(|g| g + 1),
            };
            self.at += close.ok_or(Error::Truncated)?;
            return Ok(true);
        }
        Ok(false)
    }
}

/// Text with entities replaced. Unknown entities are kept as written, as Kodi does.
pub fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.as_bytes()[..rest.len().min(12)].iter().position(|&b| b == b';') else { out.push('&'); rest = &rest[1..]; continue };
        let ent = &rest[1..end];
        let c = match ent {
            "lt" => Some('<'), "gt" => Some('>'), "amp" => Some('&'), "quot" => Some('"'), "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => { out.push(c); rest = &rest[end + 1..]; }
            None => { out.push('&'); rest = &rest[1..]; }
        }
    }
    out.push_str(rest);
    out
}

/// Parses a document and returns its root element.
pub fn parse(doc: &str) -> Result<Element, Error> {
    let doc = doc.strip_prefix('\u{feff}').unwrap_or(doc);
    let mut r = Reader { s: doc, at: 0 };
    loop {
        r.skip_ws();
        if r.at >= doc.len() { return Err(Error::Empty); }
        if !r.skip_misc()? { break; }
    }
    if !r.starts("<") { return Err(Error::Syntax(r.at)); }
    // An explicit stack instead of recursion: depth is bounded by MAX_DEPTH.
    let mut stack: Vec<Element> = Vec::new();
    loop {
        if r.at >= doc.len() { return Err(Error::Truncated); }
        if r.starts("<![CDATA[") {
            r.at += 9;
            let t = r.until("]]>")?;
            push_text(&mut stack, String::from(t));
        } else if r.skip_misc()? {
        } else if r.starts("</") {
            let at = r.at;
            r.at += 2;
            let name = r.name()?;
            r.skip_ws();
            if !r.starts(">") { return Err(Error::Syntax(r.at)); }
            r.at += 1;
            let done = stack.pop().ok_or(Error::Syntax(at))?;
            if done.name != name { return Err(Error::Mismatch(at)); }
            if let Some(root) = close(&mut stack, done) { return Ok(root); }
        } else if r.starts("<") {
            r.at += 1;
            let mut el = Element { name: String::from(r.name()?), attrs: Vec::new(), children: Vec::new() };
            let self_closing = loop {
                r.skip_ws();
                if r.starts("/>") { r.at += 2; break true; }
                if r.starts(">") { r.at += 1; break false; }
                if r.at >= doc.len() { return Err(Error::Truncated); }
                let key = r.name()?;
                r.skip_ws();
                if !r.starts("=") { return Err(Error::Syntax(r.at)); }
                r.at += 1;
                r.skip_ws();
                let q = r.rest().chars().next().ok_or(Error::Truncated)?;
                if q != '"' && q != '\'' { return Err(Error::Syntax(r.at)); }
                r.at += 1;
                let v = r.until(if q == '"' { "\"" } else { "'" })?;
                el.attrs.push((String::from(key), unescape(v)));
            };
            if self_closing {
                if let Some(root) = close(&mut stack, el) { return Ok(root); }
            } else {
                if stack.len() >= MAX_DEPTH { return Err(Error::TooDeep); }
                stack.push(el);
            }
        } else {
            let r0 = r.rest();
            let n = r0.find('<').unwrap_or(r0.len());
            r.at += n;
            if stack.is_empty() { return Err(Error::Syntax(r.at)); }
            push_text(&mut stack, unescape(&r0[..n]));
        }
    }
}

fn push_text(stack: &mut [Element], t: String) {
    if let Some(top) = stack.last_mut() {
        if let Some(Node::Text(prev)) = top.children.last_mut() { prev.push_str(&t); } else { top.children.push(Node::Text(t)); }
    }
}

/// Attaches a finished element to its parent; returns it when it was the root.
fn close(stack: &mut [Element], el: Element) -> Option<Element> {
    match stack.last_mut() {
        Some(parent) => { parent.children.push(Node::Element(el)); None }
        None => Some(el),
    }
}
