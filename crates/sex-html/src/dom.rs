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
}

impl Dom {
    pub const ROOT: NodeId = 0;

    pub fn new() -> Self {
        Self { nodes: alloc::vec![Node { kind: Kind::Document, parent: None, children: Vec::new() }] }
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
        (0..self.nodes.len()).find(|&i| self.tag(i) == Some(tag))
    }

    /// All text below `id`, concatenated.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out);
        out
    }

    fn collect_text(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].kind {
            Kind::Text(t) => out.push_str(t),
            _ => for &c in &self.nodes[id].children { self.collect_text(c, out) },
        }
    }
}

impl Default for Dom {
    fn default() -> Self { Self::new() }
}
