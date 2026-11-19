// SPDX-License-Identifier: GPL-3.0-only
//! An undo tree. Undoing and then changing something does not throw the undone changes
//! away: they stay on another branch, reachable with [`UndoTree::switch_branch`]. Redo
//! follows the branch made most recently, which is what a linear undo would do.
//!
//! Typing merges into one step: characters added at the end of the previous insert, or
//! removed just before or at the previous delete, join that step until a word ends, the
//! cursor jumps, or [`UndoTree::seal`] is called.

use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub at: usize,
    pub removed: Vec<u8>,
    pub inserted: Vec<u8>,
    pub cursor_before: usize,
}

struct Node { parent: usize, edit: Edit, children: Vec<usize>, /* which child redo takes */ redo: usize, sealed: bool }

pub struct UndoTree {
    /// Node 0 is the root: the document as opened. It holds no edit.
    nodes: Vec<Node>,
    current: usize,
}

impl Default for UndoTree { fn default() -> Self { Self::new() } }

impl UndoTree {
    pub fn new() -> Self {
        let root = Node { parent: 0, edit: Edit { at: 0, removed: Vec::new(), inserted: Vec::new(), cursor_before: 0 }, children: Vec::new(), redo: 0, sealed: true };
        UndoTree { nodes: alloc::vec![root], current: 0 }
    }
    /// The node the document is at; equal ids mean equal text.
    pub fn current(&self) -> usize { self.current }
    pub fn len(&self) -> usize { self.nodes.len() - 1 }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    /// How many steps back to the opened document.
    pub fn depth(&self) -> usize { let (mut n, mut d) = (self.current, 0); while n != 0 { n = self.nodes[n].parent; d += 1; } d }
    pub fn seal(&mut self) { self.nodes[self.current].sealed = true; }

    pub fn record(&mut self, edit: Edit) {
        let cur = self.current;
        if cur != 0 && !self.nodes[cur].sealed && self.nodes[cur].children.is_empty() {
            let last = &mut self.nodes[cur].edit;
            let word_end = edit.inserted.iter().any(|b| matches!(b, b' ' | b'\n' | b'\t'));
            // Typing on: inserts right after the previous insert.
            if edit.removed.is_empty() && last.removed.is_empty() && edit.at == last.at + last.inserted.len() {
                last.inserted.extend_from_slice(&edit.inserted);
                if word_end { self.nodes[cur].sealed = true; }
                return;
            }
            // Backspacing on: deletes ending where the previous one began.
            if edit.inserted.is_empty() && last.inserted.is_empty() && edit.at + edit.removed.len() == last.at {
                let mut removed = edit.removed.clone();
                removed.extend_from_slice(&last.removed);
                last.removed = removed;
                last.at = edit.at;
                return;
            }
            // Delete key on: deletes at the same place.
            if edit.inserted.is_empty() && last.inserted.is_empty() && edit.at == last.at {
                last.removed.extend_from_slice(&edit.removed);
                return;
            }
        }
        let word_end = edit.inserted.iter().any(|b| matches!(b, b' ' | b'\n' | b'\t'));
        let big = edit.inserted.len() > 1 || edit.removed.len() > 4;
        let id = self.nodes.len();
        self.nodes.push(Node { parent: cur, edit, children: Vec::new(), redo: 0, sealed: word_end || big });
        self.nodes[cur].children.push(id);
        self.nodes[cur].redo = self.nodes[cur].children.len() - 1;
        self.nodes[cur].sealed = true;
        self.current = id;
    }
    /// The edit to reverse, moving to its parent.
    pub fn undo(&mut self) -> Option<Edit> {
        if self.current == 0 { return None; }
        let n = self.current;
        self.nodes[n].sealed = true;
        let parent = self.nodes[n].parent;
        // Redo comes back here.
        if let Some(i) = self.nodes[parent].children.iter().position(|c| *c == n) { self.nodes[parent].redo = i; }
        self.current = parent;
        Some(self.nodes[n].edit.clone())
    }
    pub fn redo(&mut self) -> Option<Edit> {
        let node = &self.nodes[self.current];
        let child = *node.children.get(node.redo)?;
        self.current = child;
        Some(self.nodes[child].edit.clone())
    }
    /// After an undo, makes redo take the next branch from here instead. True if there
    /// was another branch.
    pub fn switch_branch(&mut self) -> bool {
        let node = &mut self.nodes[self.current];
        if node.children.len() < 2 { return false; }
        node.redo = (node.redo + 1) % node.children.len();
        true
    }
    pub fn branches(&self) -> usize { self.nodes[self.current].children.len() }
}
