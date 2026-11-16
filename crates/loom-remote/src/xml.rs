// SPDX-License-Identifier: GPL-3.0-only
//! Just enough XML for WebDAV multistatus and S3 listings: elements by local name
//! (namespace prefixes dropped), text with entities and CDATA decoded. Attributes,
//! comments, processing instructions and doctypes are skipped.
use alloc::{string::String, vec::Vec};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node { pub name: String, pub children: Vec<Node>, pub text: String }

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> { self.children.iter().find(|c| c.name == name) }
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a { self.children.iter().filter(move |c| c.name == name) }
    /// The text of a direct child, if there is one.
    pub fn text_of(&self, name: &str) -> Option<&str> { self.child(name).map(|c| c.text.as_str()) }
    /// The first element named `name` at any depth (self included).
    pub fn find(&self, name: &str) -> Option<&Node> {
        if self.name == name { return Some(self); }
        self.children.iter().find_map(|c| c.find(name))
    }
}

const DEPTH: usize = 64;

/// Parses a document into its root element.
pub fn parse(s: &str) -> Option<Node> {
    let b = s.as_bytes();
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let end = b[i..].iter().position(|c| *c == b'<').map_or(b.len(), |p| i + p);
            if let Some(top) = stack.last_mut() { unescape(&s[i..end], &mut top.text); }
            i = end;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with("<!--") { i += rest.find("-->")? + 3; continue; }
        if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let e = r.find("]]>")?;
            if let Some(top) = stack.last_mut() { top.text.push_str(&r[..e]); }
            i += 9 + e + 3;
            continue;
        }
        if rest.starts_with("<?") { i += rest.find("?>")? + 2; continue; }
        if rest.starts_with("<!") { i += rest.find('>')? + 1; continue; }
        let close = tag_end(&b[i..])?;
        let inner = &s[i + 1..i + close];
        i += close + 1;
        if let Some(name) = inner.strip_prefix('/') {
            let node = stack.pop()?;
            if local(name.trim()) != node.name { return None; }
            match stack.last_mut() { Some(p) => p.children.push(node), None => { root = Some(node); break; } }
            continue;
        }
        let selfclose = inner.ends_with('/');
        let inner = inner.trim_end_matches('/');
        let name = inner.split(|c: char| c.is_ascii_whitespace()).next()?;
        if name.is_empty() || stack.len() >= DEPTH { return None; }
        let node = Node { name: local(name).into(), ..Default::default() };
        if selfclose {
            match stack.last_mut() { Some(p) => p.children.push(node), None => { root = Some(node); break; } }
        } else { stack.push(node); }
    }
    root
}

/// The `>` that ends a tag, outside quoted attribute values.
fn tag_end(b: &[u8]) -> Option<usize> {
    let mut q = 0u8;
    for (k, &c) in b.iter().enumerate() {
        if q != 0 { if c == q { q = 0; } } else if c == b'"' || c == b'\'' { q = c; } else if c == b'>' { return Some(k); }
    }
    None
}
fn local(name: &str) -> &str { name.rsplit(':').next().unwrap_or(name) }

/// Appends text with the five named entities and numeric references decoded.
pub fn unescape(s: &str, out: &mut String) {
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(e) = rest.find(';').filter(|e| *e < 12) else { out.push('&'); rest = &rest[1..]; continue };
        let ent = &rest[1..e];
        let c = match ent {
            "amp" => Some('&'), "lt" => Some('<'), "gt" => Some('>'), "quot" => Some('"'), "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c { Some(c) => { out.push(c); rest = &rest[e + 1..]; } None => { out.push('&'); rest = &rest[1..]; } }
    }
    out.push_str(rest);
}

/// Escapes text for an element body.
pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() { match c { '&' => o.push_str("&amp;"), '<' => o.push_str("&lt;"), '>' => o.push_str("&gt;"), '"' => o.push_str("&quot;"), c => o.push(c) } }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_multistatus_and_listings() {
        let doc = r#"<?xml version="1.0"?><!-- hi --><d:multistatus xmlns:d="DAV:"><d:response><d:href>/a%20b/</d:href>
            <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><d:x a="1>2"/></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
            <d:response><d:href>/q&amp;a&#x21;&#33;<![CDATA[<x>]]></d:href></d:response></d:multistatus>"#;
        let n = parse(doc).unwrap();
        assert_eq!(n.name, "multistatus");
        let r: Vec<_> = n.all("response").collect();
        assert_eq!(r.len(), 2);
        assert!(r[0].find("collection").is_some());
        assert!(r[0].find("x").is_some());
        assert_eq!(r[1].text_of("href"), Some("/q&a!!<x>"));
        assert!(parse("<a><b></a>").is_none());
        assert!(parse("<a>").is_none());
        assert_eq!(escape("<&>"), "&lt;&amp;&gt;");
    }
}
