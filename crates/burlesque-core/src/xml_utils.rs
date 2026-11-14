// SPDX-License-Identifier: GPL-3.0-only
//! Robust, memory-bounded, panic-free XML parser for OOXML and ODF presentation files.

use alloc::string::String;
use alloc::vec::Vec;

/// XML parsing errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XmlError {
    MalformedTag,
    UnexpectedEof,
    UnclosedTag,
    DepthLimitExceeded,
    NodeLimitExceeded,
    InvalidEntity,
}

/// A parsed XML node tree.
#[derive(Clone, Debug, PartialEq)]
pub struct XmlNode {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<XmlNode>,
}

impl XmlNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            attributes: Vec::new(),
            text: String::new(),
            children: Vec::new(),
        }
    }

    /// Retrieve attribute value by exact name or namespace-stripped local name.
    pub fn attr(&self, key: &str) -> Option<&str> {
        for (k, v) in &self.attributes {
            if k == key {
                return Some(v.as_str());
            }
            if let Some(pos) = k.rfind(':') {
                if &k[pos + 1..] == key {
                    return Some(v.as_str());
                }
            }
        }
        None
    }

    /// Retrieve first direct child matching exact name or namespace-stripped name.
    pub fn child(&self, name: &str) -> Option<&XmlNode> {
        self.children.iter().find(|c| node_name_matches(&c.name, name))
    }

    /// Retrieve all direct children matching name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a XmlNode> {
        self.children.iter().filter(move |c| node_name_matches(&c.name, name))
    }

    /// Recursively find all descendant nodes matching name.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a XmlNode>) {
        if node_name_matches(&self.name, name) {
            out.push(self);
        }
        for child in &self.children {
            child.find_all(name, out);
        }
    }

    /// Parse XML string into node tree with defensive depth and node limits.
    pub fn parse(xml: &str) -> Result<Self, XmlError> {
        let mut parser = XmlParser::new(xml);
        parser.parse_root()
    }
}

fn node_name_matches(node_name: &str, target: &str) -> bool {
    if node_name == target {
        return true;
    }
    if let Some(pos) = node_name.rfind(':') {
        &node_name[pos + 1..] == target
    } else {
        false
    }
}

/// Tokenizer and recursive-descent builder.
struct XmlParser<'a> {
    chars: &'a str,
    bytes: &'a [u8],
    pos: usize,
    node_count: usize,
}

const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 50_000;

impl<'a> XmlParser<'a> {
    fn new(xml: &'a str) -> Self {
        Self {
            chars: xml,
            bytes: xml.as_bytes(),
            pos: 0,
            node_count: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        if self.pos < self.bytes.len() {
            Some(self.bytes[self.pos])
        } else {
            None
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_root(&mut self) -> Result<XmlNode, XmlError> {
        self.skip_whitespace();
        // Skip XML declarations, DTDs, and comments before root
        loop {
            if self.pos >= self.bytes.len() {
                return Err(XmlError::UnexpectedEof);
            }
            if self.bytes[self.pos..].starts_with(b"<?") {
                self.skip_until(b"?>")?;
                self.skip_whitespace();
            } else if self.bytes[self.pos..].starts_with(b"<!--") {
                self.skip_until(b"-->")?;
                self.skip_whitespace();
            } else if self.bytes[self.pos..].starts_with(b"<!DOCTYPE") {
                self.skip_until(b">")?;
                self.skip_whitespace();
            } else {
                break;
            }
        }

        self.parse_element(0)
    }

    fn skip_until(&mut self, pat: &[u8]) -> Result<(), XmlError> {
        while self.pos + pat.len() <= self.bytes.len() {
            if &self.bytes[self.pos..self.pos + pat.len()] == pat {
                self.pos += pat.len();
                return Ok(());
            }
            self.pos += 1;
        }
        Err(XmlError::UnexpectedEof)
    }

    fn parse_element(&mut self, depth: usize) -> Result<XmlNode, XmlError> {
        if depth > MAX_DEPTH {
            return Err(XmlError::DepthLimitExceeded);
        }
        self.node_count += 1;
        if self.node_count > MAX_NODES {
            return Err(XmlError::NodeLimitExceeded);
        }

        if self.peek() != Some(b'<') {
            return Err(XmlError::MalformedTag);
        }
        self.pos += 1; // skip '<'

        // Read tag name
        let name_start = self.pos;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' || b == b'/' || b == b'>' {
                break;
            }
            self.pos += 1;
        }
        let tag_name_slice = &self.chars[name_start..self.pos];
        if tag_name_slice.is_empty() {
            return Err(XmlError::MalformedTag);
        }
        let tag_name = String::from(tag_name_slice);

        // Read attributes
        let mut attributes = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.bytes.len() {
                return Err(XmlError::UnexpectedEof);
            }
            let b = self.bytes[self.pos];
            if b == b'>' || (b == b'/' && self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'>') {
                break;
            }

            // Attribute key
            let key_start = self.pos;
            while self.pos < self.bytes.len() {
                let kb = self.bytes[self.pos];
                if kb == b'=' || kb == b' ' || kb == b'\t' || kb == b'\r' || kb == b'\n' || kb == b'/' || kb == b'>' {
                    break;
                }
                self.pos += 1;
            }
            let key = String::from(&self.chars[key_start..self.pos]);
            self.skip_whitespace();

            let mut val = String::new();
            if self.peek() == Some(b'=') {
                self.pos += 1;
                self.skip_whitespace();
                if self.pos < self.bytes.len() {
                    let quote = self.bytes[self.pos];
                    if quote == b'"' || quote == b'\'' {
                        self.pos += 1;
                        let val_start = self.pos;
                        while self.pos < self.bytes.len() && self.bytes[self.pos] != quote {
                            self.pos += 1;
                        }
                        if self.pos >= self.bytes.len() {
                            return Err(XmlError::UnexpectedEof);
                        }
                        val = decode_entities(&self.chars[val_start..self.pos]);
                        self.pos += 1; // skip quote
                    }
                }
            }
            if !key.is_empty() {
                attributes.push((key, val));
            }
        }

        // Check self-closing `/>`
        self.skip_whitespace();
        if self.pos + 1 < self.bytes.len() && self.bytes[self.pos] == b'/' && self.bytes[self.pos + 1] == b'>' {
            self.pos += 2;
            return Ok(XmlNode {
                name: tag_name,
                attributes,
                text: String::new(),
                children: Vec::new(),
            });
        }

        if self.peek() == Some(b'>') {
            self.pos += 1;
        } else {
            return Err(XmlError::MalformedTag);
        }

        // Parse content: text and children until closing tag
        let mut children = Vec::new();
        let mut text_acc = String::new();

        loop {
            // Read text up to next '<'
            let text_start = self.pos;
            while self.pos < self.bytes.len() && self.bytes[self.pos] != b'<' {
                self.pos += 1;
            }
            if text_start < self.pos {
                let piece = &self.chars[text_start..self.pos];
                text_acc.push_str(&decode_entities(piece));
            }

            if self.pos >= self.bytes.len() {
                return Err(XmlError::UnclosedTag);
            }

            // Check if comment, CDATA, closing tag, or child element
            if self.bytes[self.pos..].starts_with(b"<!--") {
                self.skip_until(b"-->")?;
            } else if self.bytes[self.pos..].starts_with(b"<![CDATA[") {
                self.pos += 9;
                let cdata_start = self.pos;
                self.skip_until(b"]]>")?;
                let cdata_end = self.pos - 3;
                text_acc.push_str(&self.chars[cdata_start..cdata_end]);
            } else if self.bytes[self.pos..].starts_with(b"</") {
                // Closing tag
                self.pos += 2;
                let close_name_start = self.pos;
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'>' {
                    self.pos += 1;
                }
                if self.pos >= self.bytes.len() {
                    return Err(XmlError::UnexpectedEof);
                }
                let close_name = self.chars[close_name_start..self.pos].trim();
                self.pos += 1; // skip '>'

                // Allow matching by full name or prefix
                if node_name_matches(&tag_name, close_name) || node_name_matches(close_name, &tag_name) {
                    break;
                }
                // Graceful fallback for slightly misnested tags: continue or finish
                break;
            } else {
                // Child element
                let child = self.parse_element(depth + 1)?;
                children.push(child);
            }
        }

        Ok(XmlNode {
            name: tag_name,
            attributes,
            text: text_acc,
            children,
        })
    }
}

/// Decodes standard XML entities safely.
pub fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return String::from(input);
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp_idx) = rest.find('&') {
        out.push_str(&rest[..amp_idx]);
        let rem = &rest[amp_idx + 1..];
        if let Some(semi_idx) = rem.find(';') {
            let entity = &rem[..semi_idx];
            match entity {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                    if let Ok(code) = u32::from_str_radix(&entity[2..], 16) {
                        if let Some(ch) = core::char::from_u32(code) {
                            out.push(ch);
                        }
                    }
                }
                _ if entity.starts_with('#') => {
                    if let Ok(code) = entity[1..].parse::<u32>() {
                        if let Some(ch) = core::char::from_u32(code) {
                            out.push(ch);
                        }
                    }
                }
                _ => {
                    out.push('&');
                    out.push_str(entity);
                    out.push(';');
                }
            }
            rest = &rem[semi_idx + 1..];
        } else {
            out.push('&');
            rest = rem;
        }
    }
    out.push_str(rest);
    out
}
