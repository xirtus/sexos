// SPDX-License-Identifier: GPL-3.0-only
//! The [`Page`] for a parsed document: scripts read and change the [`Dom`], and the
//! browser lays the page out again when [`Document::dirty`] says they did.
use crate::{op, Page};
use alloc::{format, string::String, vec::Vec};
use sex_html::dom::{Dom, Kind, NodeId};

pub struct Document {
    pub dom: Dom,
    /// The page's address, for `location`.
    pub url: String,
    pub viewport: (i32, i32),
    /// Set by any change to the tree; the browser clears it after laying out again.
    pub dirty: bool,
    /// `console` output not yet taken: (level, text).
    pub log: Vec<(u8, String)>,
    /// Where a script asked to go, once it returns.
    pub navigate: Option<String>,
    /// Element boxes from the last layout, by node: (x, y, w, h).
    pub rects: Vec<Option<(i32, i32, i32, i32)>>,
    /// Supplies the monotonic clock.
    pub clock: fn() -> u64,
}

impl Document {
    pub fn new(dom: Dom, url: &str, viewport: (i32, i32), clock: fn() -> u64) -> Self {
        Self { dom, url: String::from(url), viewport, dirty: false, log: Vec::new(), navigate: None, rects: Vec::new(), clock }
    }

    fn node(&self, s: Option<&&str>) -> Option<NodeId> {
        let id: NodeId = s?.parse().ok()?;
        (id < self.dom.nodes.len()).then_some(id)
    }

    fn element(&self, s: Option<&&str>) -> Option<NodeId> {
        self.node(s).filter(|&n| matches!(self.dom.nodes[n].kind, Kind::Element { .. }))
    }

    fn ids(list: impl Iterator<Item = NodeId>) -> String {
        let mut out = String::new();
        for (i, n) in list.enumerate() {
            if i > 0 { out.push(','); }
            out.push_str(&format!("{n}"));
        }
        out
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
}

impl Page for Document {
    fn now_ns(&mut self) -> u64 { (self.clock)() }

    fn call(&mut self, operation: u32, a: &[&str]) -> Option<String> {
        let none = String::new;
        match operation {
            op::LOG => {
                let level = a.first().and_then(|l| l.parse().ok()).unwrap_or(0u8).min(2);
                let text = a.get(1).copied().unwrap_or("");
                // Keep the log bounded: a page that logs in a loop must not fill memory.
                if self.log.len() < 256 { self.log.push((level, String::from(&text[..floor(text, 1024)]))); }
                None
            }
            op::BY_ID => {
                let want = *a.first()?;
                self.dom.descendants(Dom::ROOT).into_iter().find(|&n| self.dom.attr(n, "id") == Some(want)).map(|n| format!("{n}"))
            }
            op::QUERY => {
                let root = self.node(a.first())?;
                let list = sex_html::css::parse_selector_list(a.get(1)?);
                if list.is_empty() { return Some(String::from("!")); }
                let all = a.get(2) == Some(&"1");
                let mut found = self.dom.descendants(root).into_iter().skip(1)
                    .filter(|&n| matches!(self.dom.nodes[n].kind, Kind::Element { .. }))
                    .filter(|&n| list.iter().any(|s| sex_html::css::matches(&self.dom, n, s)));
                if all { Some(Self::ids(found)) } else { found.next().map(|n| format!("{n}")) }
            }
            op::INFO => {
                let n = self.node(a.first())?;
                Some(match &self.dom.nodes[n].kind {
                    Kind::Document => String::from("9,#document"),
                    Kind::Text(_) => String::from("3,#text"),
                    Kind::Element { tag, .. } => format!("1,{}", tag.to_ascii_uppercase()),
                })
            }
            op::GET_ATTR => {
                let n = self.element(a.first())?;
                self.dom.attr(n, &a.get(1)?.to_ascii_lowercase()).map(String::from)
            }
            op::SET_ATTR => {
                let n = self.element(a.first())?;
                let name = a.get(1)?.to_ascii_lowercase();
                if !valid_name(&name) { return Some(String::from("!")); }
                self.dom.set_attr(n, &name, a.get(2)?);
                self.dirty = true;
                Some(none())
            }
            op::REMOVE_ATTR => {
                let n = self.element(a.first())?;
                self.dom.remove_attr(n, &a.get(1)?.to_ascii_lowercase());
                self.dirty = true;
                Some(none())
            }
            op::GET_TEXT => { let n = self.node(a.first())?; Some(self.dom.text_content(n)) }
            op::SET_TEXT => {
                let n = self.node(a.first())?;
                self.dom.set_text(n, a.get(1)?);
                self.dirty = true;
                Some(none())
            }
            op::GET_HTML => {
                let n = self.node(a.first())?;
                Some(if a.get(1) == Some(&"1") { self.dom.outer_html(n) } else { self.dom.inner_html(n) })
            }
            op::SET_HTML => {
                let n = self.element(a.first())?;
                self.dom.clear(n);
                sex_html::parse::parse_into(&mut self.dom, n, a.get(1)?);
                self.dirty = true;
                Some(none())
            }
            op::CREATE_ELEMENT => {
                let tag = a.first()?.to_ascii_lowercase();
                if !valid_name(&tag) { return Some(String::from("!")); }
                Some(format!("{}", self.dom.create(Kind::Element { tag, attrs: Vec::new() })))
            }
            op::CREATE_TEXT => Some(format!("{}", self.dom.create(Kind::Text(String::from(*a.first()?))))),
            op::INSERT => {
                let parent = self.node(a.first())?;
                let child = self.node(a.get(1))?;
                let before = self.node(a.get(2));
                if !self.dom.insert(parent, child, before) { return Some(String::from("!")); }
                self.dirty = true;
                Some(none())
            }
            op::DETACH => {
                let n = self.node(a.first())?;
                self.dom.detach(n);
                self.dirty = true;
                Some(none())
            }
            op::PARENT => { let n = self.node(a.first())?; self.dom.nodes[n].parent.map(|p| format!("{p}")) }
            op::CHILDREN => { let n = self.node(a.first())?; Some(Self::ids(self.dom.nodes[n].children.iter().copied())) }
            op::ATTR_NAMES => {
                let n = self.element(a.first())?;
                let Kind::Element { attrs, .. } = &self.dom.nodes[n].kind else { return None };
                Some(attrs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join("\n"))
            }
            op::VIEWPORT => Some(format!("{},{}", self.viewport.0, self.viewport.1)),
            op::NAVIGATE => { self.navigate = Some(String::from(*a.first()?)); None }
            op::CLICKABLE => {
                let n = self.element(a.first())?;
                if self.dom.clickable.insert(n) { self.dirty = true; }
                None
            }
            op::RECT => {
                let n = self.node(a.first())?;
                let (x, y, w, h) = self.rects.get(n).copied().flatten().unwrap_or((0, 0, 0, 0));
                Some(format!("{x},{y},{w},{h}"))
            }
            op::LOCATION => Some(self.url.clone()),
            _ => None,
        }
    }
}

/// The largest char boundary at or below `max`.
fn floor(s: &str, max: usize) -> usize {
    let mut i = max.min(s.len());
    while !s.is_char_boundary(i) { i -= 1; }
    i
}
