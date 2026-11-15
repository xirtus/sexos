//! Version history: every applied edit is a version with a one-line summary. Undo and redo
//! move between versions; a new edit after an undo drops the redo branch. Earlier versions
//! stay whole, so reverting is choosing one.

use crate::Timeline;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub struct Version {
    pub summary: String,
    pub timeline: Timeline,
}

#[derive(Clone, Debug)]
pub struct History {
    versions: Vec<Version>,
    head: usize,
}

impl History {
    pub fn new(original: Timeline) -> Self {
        History { versions: alloc::vec![Version { summary: "Original".into(), timeline: original }], head: 0 }
    }

    pub fn current(&self) -> &Timeline {
        &self.versions[self.head].timeline
    }

    /// The first version: the footage as imported.
    pub fn original(&self) -> &Timeline {
        &self.versions[0].timeline
    }

    pub fn commit(&mut self, summary: &str, timeline: Timeline) {
        self.versions.truncate(self.head + 1);
        self.versions.push(Version { summary: summary.into(), timeline });
        self.head += 1;
    }

    pub fn undo(&mut self) -> bool {
        if self.head == 0 {
            return false;
        }
        self.head -= 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.head + 1 >= self.versions.len() {
            return false;
        }
        self.head += 1;
        true
    }

    /// Go back to version `index` as a new version, so the versions after it stay in history.
    pub fn revert_to(&mut self, index: usize) -> bool {
        let Some(v) = self.versions.get(index) else { return false };
        let summary = alloc::format!("Back to: {}", v.summary);
        let tl = v.timeline.clone();
        self.versions.truncate(self.head + 1);
        self.versions.push(Version { summary, timeline: tl });
        self.head = self.versions.len() - 1;
        true
    }

    pub fn versions(&self) -> &[Version] {
        &self.versions
    }

    pub fn head(&self) -> usize {
        self.head
    }
}
