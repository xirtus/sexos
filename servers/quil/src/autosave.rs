// SPDX-License-Identifier: GPL-3.0-only
//! Idle autosave of an unsaved Quil buffer into a separate draft object.
//!
//! The draft never replaces the user's document and never clears its dirty
//! state: it is a recovery copy named `<title>.draft` that Linen lists like any
//! other object. One draft object is reused for the life of a document
//! source; a name already taken (for example by an older session's draft)
//! moves to `<title>NN.draft`. Storage authority is Quil's existing held document
//! route; this module only decides when and under which name to save.

/// Milliseconds of no edits before a dirty buffer is drafted.
pub const IDLE_MS: u64 = 30_000;
/// sex_pdx::document::EXISTS
const EXISTS: u32 = 10;
const MAX_SUFFIX: u8 = 99;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done { Saved { draft: u8, revision: u64 }, NameTaken { next: u8 }, Failed { error: u32 }, GaveUp }

pub struct Autosave {
    draft: Option<u8>,
    drafted: Option<u64>,
    seen: Option<(u64, u64)>,
    suffix: u8,
    pending: bool,
    /// An explicit Save (create=true for Save As) that arrived while a draft
    /// save was in flight; it runs as soon as the draft settles.
    pub queued_save: Option<bool>,
}

impl Autosave {
    pub const fn new() -> Self { Self { draft: None, drafted: None, seen: None, suffix: 0, pending: false, queued_save: None } }
    /// A new document source (load, new buffer): the old draft no longer applies.
    pub fn reset(&mut self) { let queued = self.queued_save; let pending = self.pending; *self = Self::new(); self.queued_save = queued; self.pending = pending; }
    pub fn pending(&self) -> bool { self.pending }
    /// A dirty revision is waiting out its idle period: the caller must keep
    /// polling (with a bounded timed wait) instead of blocking for input.
    pub fn armed(&self) -> bool { self.seen.is_some() && !self.pending }

    /// Whether a draft save should start now. `ready` means storage is idle
    /// and the editor is not in a modal state.
    pub fn due(&mut self, now: u64, dirty: bool, revision: u64, ready: bool) -> bool {
        if !dirty || self.pending || self.suffix > MAX_SUFFIX || self.drafted == Some(revision) { self.seen = None; return false; }
        match self.seen {
            Some((rev, _)) if rev == revision => {}
            _ => { self.seen = Some((revision, now)); return false; }
        }
        let (_, at) = self.seen.unwrap();
        if !ready || now.saturating_sub(at) < IDLE_MS { return false; }
        self.pending = true;
        true
    }

    /// Draft name: `<title>.draft` (`<title>NN.draft` after a collision),
    /// within the storage name rules (16 bytes of [A-Za-z0-9._-]).
    pub fn name(&self, title: &[u8; 16]) -> [u8; 16] {
        let len = title.iter().position(|b| *b == 0).unwrap_or(16);
        let base: &[u8] = if len == 0 { b"untitled" } else { &title[..len] };
        let room = if self.suffix == 0 { 10 } else { 8 };
        let mut out = [0u8; 16];
        let mut n = 0;
        for &b in base.iter().take(room) { out[n] = if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') { b } else { b'_' }; n += 1; }
        if self.suffix > 0 { out[n] = b'0' + self.suffix / 10; out[n + 1] = b'0' + self.suffix % 10; n += 2; }
        out[n..n + 6].copy_from_slice(b".draft");
        out
    }
    pub fn target(&self) -> Option<u8> { self.draft }

    /// Settle the in-flight draft save. Failure waits for a fresh idle period.
    pub fn finish(&mut self, saved: bool, object: u8, revision: u64, error: u32) -> Done {
        self.pending = false;
        self.seen = None;
        if saved {
            self.draft = Some(object);
            self.drafted = Some(revision);
            return Done::Saved { draft: object, revision };
        }
        if error == EXISTS && self.draft.is_none() {
            self.suffix += 1;
            return if self.suffix > MAX_SUFFIX { Done::GaveUp } else { Done::NameTaken { next: self.suffix } };
        }
        // The draft object may have vanished; create a fresh one next time.
        if self.draft.is_some() { self.draft = None; }
        Done::Failed { error }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn title(s: &[u8]) -> [u8; 16] { let mut t = [0; 16]; t[..s.len()].copy_from_slice(s); t }

    #[test]
    fn drafts_only_after_idle_on_a_dirty_new_revision() {
        let mut a = Autosave::new();
        assert!(!a.due(0, false, 1, true));
        assert!(!a.armed());
        assert!(!a.due(0, true, 1, true));
        assert!(a.armed());
        assert!(!a.due(IDLE_MS - 1, true, 1, true));
        assert!(!a.due(IDLE_MS + 5, true, 2, true), "an edit restarts the idle period");
        assert!(!a.due(2 * IDLE_MS, true, 2, false), "busy storage defers");
        assert!(a.due(2 * IDLE_MS + 5, true, 2, true));
        assert!(a.pending() && !a.armed());
        assert!(!a.due(9 * IDLE_MS, true, 2, true), "one save in flight");
        assert_eq!(a.finish(true, 7, 2, 0), Done::Saved { draft: 7, revision: 2 });
        assert!(!a.due(20 * IDLE_MS, true, 2, true) && !a.due(30 * IDLE_MS, true, 2, true), "same revision is not drafted twice");
        assert_eq!(a.target(), Some(7));
    }

    #[test]
    fn names_are_bounded_and_collisions_move_to_a_suffix() {
        let mut a = Autosave::new();
        assert_eq!(&a.name(&title(b"notes")), b"notes.draft\0\0\0\0\0");
        assert_eq!(&a.name(&title(b"")), b"untitled.draft\0\0");
        assert_eq!(&a.name(&title(b"abcdefghijklmnop")), b"abcdefghij.draft");
        assert_eq!(&a.name(&title(b"a b~c")), b"a_b_c.draft\0\0\0\0\0");
        a.pending = true;
        assert_eq!(a.finish(false, 0, 1, EXISTS), Done::NameTaken { next: 1 });
        assert_eq!(&a.name(&title(b"abcdefghijklmnop")), b"abcdefgh01.draft");
        assert_eq!(&a.name(&title(b"notes")), b"notes01.draft\0\0\0");
        for _ in 1..MAX_SUFFIX { a.finish(false, 0, 1, EXISTS); }
        assert_eq!(a.finish(false, 0, 1, EXISTS), Done::GaveUp);
        assert!(!a.due(0, true, 5, true) && !a.due(10 * IDLE_MS, true, 5, true), "gave up");
    }

    #[test]
    fn reset_forgets_the_draft_but_keeps_inflight_state() {
        let mut a = Autosave::new();
        a.due(0, true, 1, true); assert!(a.due(IDLE_MS, true, 1, true));
        a.queued_save = Some(false);
        a.reset();
        assert!(a.pending() && a.queued_save == Some(false) && a.target().is_none());
        assert_eq!(a.finish(true, 3, 1, 0), Done::Saved { draft: 3, revision: 1 });
    }

    #[test]
    fn lost_draft_object_is_recreated() {
        let mut a = Autosave::new();
        a.pending = true; a.finish(true, 4, 1, 0);
        a.pending = true;
        assert_eq!(a.finish(false, 0, 2, 9), Done::Failed { error: 9 });
        assert_eq!(a.target(), None);
    }
}
