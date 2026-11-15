// SPDX-License-Identifier: GPL-3.0-only
//! Bounded candidate indexes. Matching always reads the current DOM, not cached results.
use alloc::{collections::{BTreeMap, BTreeSet}, string::String, vec, vec::Vec};
use crate::{css::{self, Selector}, dom::{Dom, Kind, NodeId}};

#[derive(Default)]
pub(crate) struct Cache { index: Option<Index>, attempted: bool }
pub(crate) struct Index {
    ids: BTreeMap<String, Vec<NodeId>>,
    classes: BTreeMap<String, Vec<NodeId>>,
    tags: BTreeMap<String, Vec<NodeId>>,
    rank: Vec<usize>,
    end: Vec<usize>,
    order: Vec<NodeId>,
}
impl Index {
    fn build(dom: &Dom) -> Option<Self> {
        if dom.nodes.len() > 32768 { return None; }
        let mut tokens = 0usize;
        let mut bytes = 0usize;
        for n in &dom.nodes {
            if let Kind::Element { tag, attrs } = &n.kind {
                tokens += 1; bytes = bytes.saturating_add(tag.len());
                for (name, value) in attrs {
                    if name == "id" { tokens += 1; bytes = bytes.saturating_add(value.len()); }
                    if name == "class" {
                        for c in value.split_ascii_whitespace() { tokens += 1; bytes = bytes.saturating_add(c.len()); }
                    }
                }
                if tokens > 131072 || bytes > 2*1024*1024 { return None; }
            }
        }
        let mut ix = Self { ids: BTreeMap::new(), classes: BTreeMap::new(), tags: BTreeMap::new(),
            rank: vec![0; dom.nodes.len()], end: vec![0; dom.nodes.len()], order: Vec::new() };
        // Each detached subtree is also queryable. Preorder intervals preserve
        // tree order after reparenting; arena allocation order is not tree order.
        for root in 0..dom.nodes.len() {
            if dom.nodes[root].parent.is_some() { continue; }
            let mut stack = vec![(root, false)];
            while let Some((n, leave)) = stack.pop() {
                if leave { ix.end[n] = ix.order.len(); continue; }
                ix.rank[n] = ix.order.len(); ix.order.push(n);
                stack.push((n, true));
                stack.extend(dom.nodes[n].children.iter().rev().map(|&c| (c, false)));
                if let Some(tag) = dom.tag(n) {
                    ix.tags.entry(String::from(tag)).or_default().push(n);
                    if let Some(id) = dom.attr(n, "id") { ix.ids.entry(String::from(id)).or_default().push(n); }
                    if let Some(classes) = dom.attr(n, "class") {
                        for c in classes.split_ascii_whitespace() {
                            let entries = ix.classes.entry(String::from(c)).or_default();
                            if entries.last() != Some(&n) { entries.push(n); }
                        }
                    }
                }
            }
        }
        Some(ix)
    }
    fn candidates<'a>(&'a self, selector: &Selector) -> Option<&'a [NodeId]> {
        let right = selector.parts.last()?;
        let mut best: Option<&[NodeId]> = None;
        let mut choose = |v: Option<&'a Vec<NodeId>>| {
            let v = v.map(Vec::as_slice).unwrap_or(&[]);
            if best.is_none_or(|b| v.len() < b.len()) { best = Some(v); }
        };
        if let Some(id) = &right.id { choose(self.ids.get(id)); }
        if let Some(tag) = &right.tag { choose(self.tags.get(tag)); }
        for c in &right.classes { choose(self.classes.get(c)); }
        best
    }
}
impl Cache {
    pub(crate) fn query(&mut self, dom: &Dom, root: NodeId, selectors: &[Selector], all: bool) -> Vec<NodeId> {
        if !self.attempted { self.index = Index::build(dom); self.attempted = true; }
        let Some(ix) = &self.index else {
            let found = dom.descendants(root).into_iter().skip(1).filter(|&n|
                selectors.iter().any(|s| css::matches(dom, n, s)));
            return if all { found.collect() } else { found.take(1).collect() };
        };
        let lo = ix.rank[root] + 1; let hi = ix.end[root];
        let mut found = BTreeSet::new();
        for selector in selectors {
            let candidates = ix.candidates(selector).unwrap_or(&ix.order[lo..hi]);
            for &n in candidates {
                let rank = ix.rank[n];
                if rank < lo || rank >= hi { continue; }
                if !all && found.first().is_some_and(|&(r, _)| rank >= r) { break; }
                if css::matches(dom, n, selector) {
                    found.insert((rank, n));
                    if !all { break; }
                }
            }
        }
        if all { found.into_iter().map(|(_, n)| n).collect() }
        else { found.into_iter().take(1).map(|(_, n)| n).collect() }
    }
}
