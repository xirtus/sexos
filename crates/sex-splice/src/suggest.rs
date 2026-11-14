//! Suggestions wait for the editor.
//!
//! An assistant (or a rule such as "shorten long pauses") can apply an edit straight to the
//! working timeline, or stage it here. A staged edit is kept as source cuts, not timeline
//! ops, so it still lands on the right words after the editor makes other changes. The
//! editor applies it, skips it, or asks for a revision; nothing is lost either way.

use crate::transcript::{ripple_ops, SourceCut};
use crate::{EditError, Op, Timeline};
use alloc::string::String;
use alloc::vec::Vec;

/// Who proposed an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Author {
    Editor,
    /// A rule built into Splicy, such as the pause or filler pass.
    Rule,
    /// An assistant, which runs confined and off by default.
    Assistant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Applied,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub id: u32,
    pub by: Author,
    /// One sentence the editor reads on the suggestion's card.
    pub summary: String,
    pub cuts: Vec<SourceCut>,
    pub status: Status,
}

#[derive(Clone, Debug, Default)]
pub struct Suggestions {
    pub items: Vec<Suggestion>,
    next: u32,
}

impl Suggestions {
    pub fn stage(&mut self, by: Author, summary: &str, cuts: Vec<SourceCut>) -> u32 {
        self.next += 1;
        self.items.push(Suggestion { id: self.next, by, summary: summary.into(), cuts, status: Status::Pending });
        self.next
    }

    pub fn get(&self, id: u32) -> Option<&Suggestion> {
        self.items.iter().find(|s| s.id == id)
    }

    pub fn pending(&self) -> impl Iterator<Item = &Suggestion> {
        self.items.iter().filter(|s| s.status == Status::Pending)
    }

    /// The ops a suggestion would make on `tl` now, for preview before applying.
    pub fn ops_for(&self, id: u32, tl: &Timeline) -> Option<Vec<Op>> {
        self.get(id).map(|s| ripple_ops(tl, &s.cuts))
    }

    /// Apply a pending suggestion to `tl`, returning the new timeline.
    pub fn apply(&mut self, id: u32, tl: &Timeline) -> Result<Option<Timeline>, EditError> {
        let Some(s) = self.items.iter_mut().find(|s| s.id == id && s.status == Status::Pending) else { return Ok(None) };
        let next = tl.apply_all(&ripple_ops(tl, &s.cuts))?;
        s.status = Status::Applied;
        Ok(Some(next))
    }

    pub fn skip(&mut self, id: u32) -> bool {
        match self.items.iter_mut().find(|s| s.id == id && s.status == Status::Pending) {
            Some(s) => {
                s.status = Status::Skipped;
                true
            }
            None => false,
        }
    }
}
