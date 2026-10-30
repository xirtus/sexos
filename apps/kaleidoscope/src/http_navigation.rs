// SPDX-License-Identifier: GPL-3.0-only
//! Finite private navigation intent, never endpoint or communication authority.
//! Before each follow, the caller must obtain positive preceding job cleanup
//! and fresh DNS provenance through its existing held BOOT route. This pure
//! controller cannot authorize or perform either operation.
use crate::http_wire::Path;

pub struct Navigation {
    visited: [Path; 3],
    count: u8,
    active: bool,
}

impl Navigation {
    pub const fn new() -> Self {
        Self { visited: [Path::initial(); 3], count: 0, active: false }
    }

    /// A new explicit user intent starts at the fixed initial document.
    pub fn begin(&mut self) {
        self.visited = [Path::initial(); 3];
        self.count = 1;
        self.active = true;
    }

    /// Before begin or after cancellation, this is the fixed initial path.
    pub fn current(&self) -> Path {
        if self.count == 0 { Path::initial() } else { self.visited[self.count as usize - 1] }
    }

    pub fn active(&self) -> bool { self.active }

    pub fn hops(&self) -> usize { self.count.saturating_sub(1) as usize }

    /// Record only bounded path data after the caller's positive cleanup and
    /// fresh resolution. Rejection ends this intent without accepting the path.
    pub fn follow(&mut self, path: Path) -> bool {
        let count = self.count as usize;
        if !self.active || count == self.visited.len()
            || self.visited[..count].contains(&path) {
            self.active = false;
            return false;
        }
        self.visited[count] = path;
        self.count += 1;
        true
    }

    /// Final body completion preserves its path for presentation metadata.
    pub fn complete(&mut self) { self.active = false; }

    /// Discard all retained redirect intent on cancellation or supersession.
    pub fn cancel(&mut self) {
        self.visited = [Path::initial(); 3];
        self.count = 0;
        self.active = false;
    }
}
