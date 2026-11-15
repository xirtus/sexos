// SPDX-License-Identifier: GPL-3.0-only
//! Bounded candidate indexes. Matching always reads the current DOM, not cached results.
//!
//! Two parts, kept apart because pages change them at different rates:
//! - `Keys`: the elements carrying each id, class and tag. DOM mutation methods update
//!   it in place (an attribute change touches one node), so it survives the
//!   set-attribute/create/insert churn of script start-up.
//! - `Order`: preorder ranks for tree order and subtree ranges. Any move of a node
//!   makes it stale; it is rebuilt only when a query needs it. A query whose
//!   candidates are few checks ancestry and sorts them directly instead.
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use crate::{css::{self, Selector}, dom::{Dom, Kind, NodeId}};

/// Most candidates checked by ancestor walks before the tree order is rebuilt.
const FEW: usize = 64;
const MAX_NODES: usize = 32768;
const MAX_TOKENS: usize = 131072;
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Cache { keys: Option<Keys>, attempted: bool, order: Option<Order> }

/// Candidate lists per key. Lists are in no particular order.
struct Keys {
    ids: BTreeMap<String, Vec<NodeId>>,
    classes: BTreeMap<String, Vec<NodeId>>,
    tags: BTreeMap<String, Vec<NodeId>>,
    tokens: usize,
    bytes: usize,
}

struct Order { rank: Vec<usize>, end: Vec<usize>, order: Vec<NodeId> }

fn put(map: &mut BTreeMap<String, Vec<NodeId>>, key: &str, n: NodeId) {
    match map.get_mut(key) {
        Some(v) => if !v.contains(&n) { v.push(n) },
        None => { map.insert(String::from(key), vec![n]); }
    }
}

fn take(map: &mut BTreeMap<String, Vec<NodeId>>, key: &str, n: NodeId) {
    if let Some(v) = map.get_mut(key) {
        if let Some(i) = v.iter().position(|&x| x == n) { v.swap_remove(i); }
        if v.is_empty() { map.remove(key); }
    }
}

impl Keys {
    fn build(dom: &Dom) -> Option<Self> {
        if dom.nodes.len() > MAX_NODES { return None; }
        let mut keys = Self { ids: BTreeMap::new(), classes: BTreeMap::new(), tags: BTreeMap::new(), tokens: 0, bytes: 0 };
        for (n, node) in dom.nodes.iter().enumerate() {
            if let Kind::Element { tag, attrs } = &node.kind {
                keys.add(n, tag, attrs);
                if keys.tokens > MAX_TOKENS || keys.bytes > MAX_BYTES { return None; }
            }
        }
        Some(keys)
    }

    fn add(&mut self, n: NodeId, tag: &str, attrs: &[(String, String)]) {
        self.tokens += 1; self.bytes = self.bytes.saturating_add(tag.len());
        put(&mut self.tags, tag, n);
        for (name, value) in attrs { self.attr(n, name, value, true); }
    }

    fn attr(&mut self, n: NodeId, name: &str, value: &str, add: bool) {
        let each = |map: &mut BTreeMap<String, Vec<NodeId>>, key: &str, tokens: &mut usize, bytes: &mut usize| {
            if add { *tokens += 1; *bytes = bytes.saturating_add(key.len()); put(map, key, n) } else { take(map, key, n) }
        };
        match name {
            "id" => each(&mut self.ids, value, &mut self.tokens, &mut self.bytes),
            "class" => for c in value.split_ascii_whitespace() { each(&mut self.classes, c, &mut self.tokens, &mut self.bytes) },
            _ => {}
        }
    }

    /// The shortest candidate list for the selector's rightmost compound, or None when
    /// it names no id, tag or class.
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

impl Order {
    fn build(dom: &Dom) -> Self {
        let mut o = Self { rank: vec![0; dom.nodes.len()], end: vec![0; dom.nodes.len()], order: Vec::with_capacity(dom.nodes.len()) };
        // Each detached subtree is also queryable. Preorder intervals preserve
        // tree order after reparenting; arena allocation order is not tree order.
        let mut stack = Vec::new();
        for root in 0..dom.nodes.len() {
            if dom.nodes[root].parent.is_some() { continue; }
            stack.push((root, false));
            while let Some((n, leave)) = stack.pop() {
                if leave { o.end[n] = o.order.len(); continue; }
                o.rank[n] = o.order.len(); o.order.push(n);
                stack.push((n, true));
                stack.extend(dom.nodes[n].children.iter().rev().map(|&c| (c, false)));
            }
        }
        o
    }
}

/// Tree-order key: the top of the node's tree, then its child index at each level.
fn position(dom: &Dom, mut n: NodeId) -> Vec<usize> {
    let mut path = Vec::new();
    while let Some(p) = dom.nodes[n].parent {
        path.push(dom.nodes[p].children.iter().position(|&c| c == n).unwrap_or(0));
        n = p;
    }
    path.push(n);
    path.reverse();
    path
}

impl Cache {
    /// Ordinary DOM methods report their changes here so the indexes stay usable.
    pub(crate) fn element_created(&mut self, n: NodeId, kind: &Kind, attached: bool) {
        if let (Some(keys), Kind::Element { tag, attrs }) = (&mut self.keys, kind) {
            keys.add(n, tag, attrs);
            if keys.tokens > MAX_TOKENS || keys.bytes > MAX_BYTES || n >= MAX_NODES { self.keys = None; self.attempted = false; }
        }
        match &mut self.order {
            // A new detached node is its own tree, after every existing one.
            Some(o) if !attached && o.rank.len() == n => { o.rank.push(o.order.len()); o.order.push(n); o.end.push(o.order.len()); }
            _ => self.order = None,
        }
    }

    pub(crate) fn attr_changed(&mut self, n: NodeId, name: &str, old: Option<&str>, new: Option<&str>) {
        if name != "id" && name != "class" { return; }
        let Some(keys) = &mut self.keys else { self.attempted = false; return };
        if let Some(old) = old { keys.attr(n, name, old, false); }
        if let Some(new) = new { keys.attr(n, name, new, true); }
        if keys.tokens > MAX_TOKENS || keys.bytes > MAX_BYTES { self.keys = None; self.attempted = false; }
    }

    pub(crate) fn tree_changed(&mut self) { self.order = None; }

    pub(crate) fn query(&mut self, dom: &Dom, root: NodeId, selectors: &[Selector], all: bool) -> Vec<NodeId> {
        if !self.attempted { self.keys = Keys::build(dom); self.attempted = true; }
        let Some(keys) = &self.keys else { return walk(dom, root, selectors, all) };
        let lists: Vec<Option<&[NodeId]>> = selectors.iter().map(|s| keys.candidates(s)).collect();
        if self.order.is_none() {
            if lists.iter().any(Option::is_none) {
                // Every element is a candidate: walking the subtree costs no more than
                // rebuilding the order, unless the subtree is a whole tree.
                if dom.nodes[root].parent.is_some() { return walk(dom, root, selectors, all); }
            } else if lists.iter().map(|l| l.map_or(0, <[_]>::len)).sum::<usize>() <= FEW {
                return few(dom, root, selectors, &lists, all);
            }
            self.order = Some(Order::build(dom));
        }
        let ix = self.order.as_ref().unwrap();
        let lo = ix.rank[root] + 1; let hi = ix.end[root];
        let mut found: Vec<(usize, NodeId)> = Vec::new();
        let mut best = usize::MAX;
        for (selector, list) in selectors.iter().zip(&lists) {
            let candidates = list.unwrap_or(&ix.order[lo..hi]);
            for &n in candidates {
                let rank = ix.rank[n];
                if rank < lo || rank >= hi || (!all && rank >= best) { continue; }
                if css::matches(dom, n, selector) {
                    if all { found.push((rank, n)) } else { best = rank; found = vec![(rank, n)]; }
                    // The subtree list is in tree order: its first match is the first.
                    if !all && list.is_none() { break; }
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found.into_iter().map(|(_, n)| n).collect()
    }
}

/// Candidates checked by ancestry, then put in tree order.
fn few(dom: &Dom, root: NodeId, selectors: &[Selector], lists: &[Option<&[NodeId]>], all: bool) -> Vec<NodeId> {
    let mut found: Vec<NodeId> = Vec::new();
    for (selector, list) in selectors.iter().zip(lists) {
        for &n in list.unwrap_or(&[]) {
            if n != root && !found.contains(&n) && dom.is_inclusive_ancestor(root, n) && css::matches(dom, n, selector) { found.push(n); }
        }
    }
    if found.len() > 1 {
        let mut keyed: Vec<(Vec<usize>, NodeId)> = found.into_iter().map(|n| (position(dom, n), n)).collect();
        keyed.sort_unstable();
        found = keyed.into_iter().map(|(_, n)| n).collect();
    }
    if !all { found.truncate(1); }
    found
}

fn walk(dom: &Dom, root: NodeId, selectors: &[Selector], all: bool) -> Vec<NodeId> {
    let found = dom.descendants(root).into_iter().skip(1).filter(|&n| selectors.iter().any(|s| css::matches(dom, n, s)));
    if all { found.collect() } else { found.take(1).collect() }
}
