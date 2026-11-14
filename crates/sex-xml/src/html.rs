// SPDX-License-Identifier: GPL-3.0-only
//! HTML as plain text in blocks: paragraphs, headings, list items, quotes and
//! preformatted runs, white space collapsed outside `pre`. Enough for a reader that
//! reflows text itself; layout, styles and scripts are ignored.
use crate::{parse, Child, Node};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Para,
    /// `h1` is 1 ... `h6` is 6.
    Heading(u8),
    Item,
    Quote,
    Pre,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    pub text: String,
}

const SKIP: &[&str] = &["head", "script", "style", "title", "noscript", "template", "svg", "math", "iframe", "object"];
const BLOCK: &[&str] = &["p", "div", "section", "article", "aside", "header", "footer", "main", "nav", "figure", "figcaption",
    "table", "tr", "ul", "ol", "dl", "dt", "dd", "form", "fieldset", "address", "center", "body", "html", "hr", "caption", "details", "summary"];

struct Walk { blocks: Vec<Block>, cur: String, kind: Kind, space: bool }

impl Walk {
    fn flush(&mut self) {
        let text = if self.kind == Kind::Pre { String::from(self.cur.trim_end_matches(['\n', ' '])) } else { String::from(self.cur.trim_end()) };
        if !text.trim().is_empty() { self.blocks.push(Block { kind: self.kind, text }); }
        self.cur.clear();
        self.space = false;
    }
    fn text(&mut self, t: &str) {
        if self.kind == Kind::Pre { self.cur.push_str(t); return; }
        for c in t.chars() {
            if c.is_whitespace() && c != '\u{A0}' {
                self.space = true;
            } else {
                if self.space && !self.cur.is_empty() { self.cur.push(' '); }
                self.space = false;
                self.cur.push(if c == '\u{A0}' { ' ' } else { c });
            }
        }
    }
    fn node(&mut self, n: &Node) {
        let name = crate::local(&n.name).to_ascii_lowercase();
        let name = name.as_str();
        if SKIP.contains(&name) { return; }
        let kind = match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Some(Kind::Heading(name.as_bytes()[1] - b'0')),
            "li" => Some(Kind::Item),
            "blockquote" => Some(Kind::Quote),
            "pre" => Some(Kind::Pre),
            _ if BLOCK.contains(&name) => Some(Kind::Para),
            _ => None,
        };
        match name {
            "br" => { self.flush(); return; }
            "img" | "image" => {
                if let Some(alt) = n.attr("alt").filter(|a| !a.trim().is_empty()) {
                    self.space = true;
                    self.text("[");
                    self.cur.push_str(alt.trim());
                    self.cur.push(']');
                }
                return;
            }
            _ => {}
        }
        let outer = self.kind;
        if let Some(k) = kind {
            self.flush();
            // A paragraph inside a quote or list item stays a quote or item.
            self.kind = if k == Kind::Para && outer != Kind::Para { outer } else { k };
        }
        if matches!(name, "td" | "th") && !self.cur.is_empty() { self.cur.push_str("  "); }
        for c in &n.children {
            match c { Child::Text(t) => self.text(t), Child::Element(e) => self.node(e) }
        }
        if kind.is_some() {
            self.flush();
            self.kind = outer;
        }
    }
}

/// The blocks of text in an HTML document or fragment.
pub fn blocks(html: &str) -> Vec<Block> { blocks_of(&parse(html)) }

/// The blocks of text under an already parsed node.
pub fn blocks_of(root: &Node) -> Vec<Block> {
    let mut w = Walk { blocks: Vec::new(), cur: String::new(), kind: Kind::Para, space: false };
    for c in &root.children {
        match c { Child::Text(t) => w.text(t), Child::Element(e) => w.node(e) }
    }
    w.flush();
    w.blocks
}

/// The document as plain text, blocks separated by blank lines.
pub fn to_text(html: &str) -> String {
    let mut out = String::new();
    for b in blocks(html) {
        if !out.is_empty() { out.push_str("\n\n"); }
        if b.kind == Kind::Item { out.push_str("* "); }
        out.push_str(&b.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_headings_lists() {
        let b = blocks("<html><head><title>x</title><style>p{}</style></head><body><h2>Chapter  One</h2><p>Hello <em>big</em>\n world.</p><ul><li><p>a</p></li><li>b</li></ul><pre>  x\n  y\n</pre><p>a<br/>b <img alt='cat' src='c.png'/></p></body></html>");
        let got: Vec<(Kind, &str)> = b.iter().map(|b| (b.kind, b.text.as_str())).collect();
        assert_eq!(got, [
            (Kind::Heading(2), "Chapter One"), (Kind::Para, "Hello big world."), (Kind::Item, "a"), (Kind::Item, "b"),
            (Kind::Pre, "  x\n  y"), (Kind::Para, "a"), (Kind::Para, "b [cat]"),
        ]);
    }

    #[test]
    fn soup_to_text() {
        assert_eq!(to_text("Some <b>bold</b> &amp; <a href=x>linked</a> text<p>Next&nbsp;one"), "Some bold & linked text\n\nNext one");
    }
}
