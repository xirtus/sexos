// SPDX-License-Identifier: GPL-3.0-only
//! The document tree: one arena of nodes, each knowing its parent and children.
use alloc::string::String;
use alloc::vec::Vec;

pub type NodeId = usize;

pub enum Kind {
    Document,
    Element { tag: String, attrs: Vec<(String, String)> },
    Text(String),
}

pub struct Node {
    pub kind: Kind,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

pub struct Dom {
    pub nodes: Vec<Node>,
    /// Elements a script listens to for clicks, so the person can reach them.
    pub clickable: alloc::collections::BTreeSet<NodeId>,
}

impl Dom {
    pub const ROOT: NodeId = 0;

    pub fn new() -> Self {
        Self { nodes: alloc::vec![Node { kind: Kind::Document, parent: None, children: Vec::new() }], clickable: Default::default() }
    }

    pub fn add(&mut self, parent: NodeId, kind: Kind) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node { kind, parent: Some(parent), children: Vec::new() });
        self.nodes[parent].children.push(id);
        id
    }

    /// Adds text, joined to the previous text child when there is one.
    pub fn add_text(&mut self, parent: NodeId, text: &str) {
        if let Some(&last) = self.nodes[parent].children.last() {
            if let Kind::Text(s) = &mut self.nodes[last].kind {
                s.push_str(text);
                return;
            }
        }
        self.add(parent, Kind::Text(String::from(text)));
    }

    pub fn tag(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id].kind { Kind::Element { tag, .. } => Some(tag), _ => None }
    }

    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        match &self.nodes[id].kind {
            Kind::Element { attrs, .. } => attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()),
            _ => None,
        }
    }

    /// Can the person activate this element: a link, a button, or one with a click handler?
    pub fn activatable(&self, id: NodeId) -> bool {
        match self.tag(id) {
            Some("a") if self.attr(id, "href").is_some() => true,
            Some("button") | Some("summary") => true,
            Some("input") => matches!(self.attr(id, "type").map(|t| t.to_ascii_lowercase()).as_deref(), Some("button" | "submit" | "reset" | "checkbox" | "radio")),
            Some(_) => self.attr(id, "onclick").is_some() || self.clickable.contains(&id),
            None => false,
        }
    }

    pub fn has_class(&self, id: NodeId, class: &str) -> bool {
        self.attr(id, "class").map_or(false, |c| c.split_ascii_whitespace().any(|c| c == class))
    }

    /// Element children only, in order.
    pub fn element_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes[id].children.iter().copied().filter(move |c| matches!(self.nodes[*c].kind, Kind::Element { .. }))
    }

    pub fn prev_element(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.nodes[id].parent?;
        let mut prev = None;
        for c in self.element_children(parent) {
            if c == id { return prev; }
            prev = Some(c);
        }
        None
    }

    /// First element with this tag in document order.
    pub fn find(&self, tag: &str) -> Option<NodeId> {
        self.descendants(Self::ROOT).into_iter().find(|&i| self.tag(i) == Some(tag))
    }

    /// All text below `id`, concatenated.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out);
        out
    }

    /// A node in no tree yet, for scripts to place with [`Dom::insert`].
    pub fn create(&mut self, kind: Kind) -> NodeId {
        self.nodes.push(Node { kind, parent: None, children: Vec::new() });
        self.nodes.len() - 1
    }

    /// Is `node` inside `ancestor` (or the same node)?
    pub fn contains(&self, ancestor: NodeId, mut node: NodeId) -> bool {
        loop {
            if node == ancestor { return true; }
            match self.nodes[node].parent { Some(p) => node = p, None => return false }
        }
    }

    /// Takes a node out of its parent; it and its subtree stay in the arena, detached.
    pub fn detach(&mut self, node: NodeId) {
        if let Some(p) = self.nodes[node].parent.take() { self.nodes[p].children.retain(|&c| c != node); }
    }

    /// Puts `child` under `parent`, before `before` when that is one of its children,
    /// otherwise last. Refuses (false) to put a node inside itself.
    pub fn insert(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) -> bool {
        if child == Self::ROOT || self.contains(child, parent) || matches!(self.nodes[parent].kind, Kind::Text(_)) { return false; }
        self.detach(child);
        let at = before.and_then(|b| self.nodes[parent].children.iter().position(|&c| c == b)).unwrap_or(self.nodes[parent].children.len());
        self.nodes[parent].children.insert(at, child);
        self.nodes[child].parent = Some(parent);
        true
    }

    /// Detaches every child of `node`.
    pub fn clear(&mut self, node: NodeId) {
        for c in core::mem::take(&mut self.nodes[node].children) { self.nodes[c].parent = None; }
    }

    pub fn set_attr(&mut self, id: NodeId, name: &str, value: &str) {
        if let Kind::Element { attrs, .. } = &mut self.nodes[id].kind {
            match attrs.iter_mut().find(|(n, _)| n == name) {
                Some(a) => a.1 = String::from(value),
                None => attrs.push((String::from(name), String::from(value))),
            }
        }
    }

    pub fn remove_attr(&mut self, id: NodeId, name: &str) {
        if let Kind::Element { attrs, .. } = &mut self.nodes[id].kind { attrs.retain(|(n, _)| n != name); }
    }

    /// Replaces a text node's text, or an element's children with one text node.
    pub fn set_text(&mut self, id: NodeId, text: &str) {
        if let Kind::Text(t) = &mut self.nodes[id].kind { *t = String::from(text); return; }
        if let Kind::Document = self.nodes[id].kind { return; }
        self.clear(id);
        if !text.is_empty() {
            let t = self.create(Kind::Text(String::from(text)));
            self.insert(id, t, None);
        }
    }

    /// Nodes in document order from `id`, `id` first.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = alloc::vec![id];
        while let Some(n) = stack.pop() {
            out.push(n);
            stack.extend(self.nodes[n].children.iter().rev().copied());
        }
        out
    }

    /// The node's children as HTML.
    pub fn inner_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        for &c in &self.nodes[id].children { self.write_html(c, &mut out); }
        out
    }

    /// The node itself as HTML.
    pub fn outer_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.write_html(id, &mut out);
        out
    }

    fn write_html(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].kind {
            Kind::Document => for &c in &self.nodes[id].children { self.write_html(c, out) },
            Kind::Text(t) => {
                let raw = self.nodes[id].parent.and_then(|p| self.tag(p)).map_or(false, |t| matches!(t, "script" | "style"));
                if raw { out.push_str(t) } else { escape(t, false, out) }
            }
            Kind::Element { tag, attrs } => {
                out.push('<');
                out.push_str(tag);
                for (n, v) in attrs {
                    out.push(' ');
                    out.push_str(n);
                    out.push_str("=\"");
                    escape(v, true, out);
                    out.push('"');
                }
                out.push('>');
                if crate::parse::is_void(tag) { return; }
                for &c in &self.nodes[id].children { self.write_html(c, out); }
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
        }
    }

    fn collect_text(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].kind {
            Kind::Text(t) => out.push_str(t),
            _ => for &c in &self.nodes[id].children { self.collect_text(c, out) },
        }
    }
}

fn escape(text: &str, attr: bool, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' if !attr => out.push_str("&lt;"),
            '>' if !attr => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
}

impl Default for Dom {
    fn default() -> Self { Self::new() }
}
