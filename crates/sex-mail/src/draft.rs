// SPDX-License-Identifier: GPL-3.0-only
//! A local, unsent RFC 5322 draft. Account identity and delivery are added only
//! after Multipass gives Valentine an authenticated submission flow.
use crate::message::{self, Message};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalDraft { pub to: String, pub subject: String, pub body: String }

/// A tail edit in one composer field. Each step stores one character, so even a
/// long body does not copy into the undo history on every keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftEdit { Insert { field: u8, ch: char }, Delete { field: u8, ch: char } }

pub struct DraftHistory {
    saved: Option<LocalDraft>,
    undo: Vec<DraftEdit>,
    redo: Vec<DraftEdit>,
}

impl DraftHistory {
    pub fn new(saved: Option<LocalDraft>) -> Self { Self { saved, undo: Vec::new(), redo: Vec::new() } }
    pub fn dirty(&self, draft: &LocalDraft) -> bool { self.saved.as_ref() != Some(draft) }
    pub fn mark_saved(&mut self, draft: &LocalDraft) { self.saved = Some(draft.clone()); }

    pub fn edit(&mut self, draft: &mut LocalDraft, change: DraftEdit) -> bool {
        if !apply(draft, change, false) { return false; }
        if self.undo.len() == 1024 { self.undo.remove(0); }
        self.undo.push(change);
        self.redo.clear();
        true
    }
    pub fn undo(&mut self, draft: &mut LocalDraft) -> bool {
        let Some(&change) = self.undo.last() else { return false };
        if !apply(draft, change, true) { return false; }
        self.undo.pop(); self.redo.push(change); true
    }
    pub fn redo(&mut self, draft: &mut LocalDraft) -> bool {
        let Some(&change) = self.redo.last() else { return false };
        if !apply(draft, change, false) { return false; }
        self.redo.pop(); self.undo.push(change); true
    }
}

fn apply(draft: &mut LocalDraft, change: DraftEdit, reverse: bool) -> bool {
    let field = match change { DraftEdit::Insert { field, .. } | DraftEdit::Delete { field, .. } => field };
    let value = match field { 0 => &mut draft.to, 1 => &mut draft.subject, 2 => &mut draft.body, _ => return false };
    let (ch, insert) = match change {
        DraftEdit::Insert { ch, .. } => (ch, !reverse),
        DraftEdit::Delete { ch, .. } => (ch, reverse),
    };
    if insert { value.push(ch); return true; }
    if !value.ends_with(ch) { return false; }
    value.pop(); true
}

impl LocalDraft {
    /// Reject controls in header fields so an imported or edited draft cannot
    /// inject an extra recipient or message header.
    pub fn encode(&self) -> Option<Vec<u8>> {
        if self.to.chars().any(char::is_control) || self.subject.chars().any(char::is_control) { return None; }
        let mut out = alloc::format!("X-Unsent: 1\r\nTo: {}\r\nSubject: {}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n",
            self.to.trim(), message::encode_word(self.subject.trim())).into_bytes();
        for line in self.body.split('\n') { out.extend_from_slice(line.trim_end_matches('\r').as_bytes()); out.extend_from_slice(b"\r\n"); }
        Some(out)
    }

    pub fn from_message(message: &Message) -> Option<Self> {
        if message.headers.get("X-Unsent") != Some("1") { return None; }
        let text = message.text.as_deref().unwrap_or("");
        let body = text.strip_suffix("\r\n").unwrap_or(text).replace("\r\n", "\n");
        Some(Self { to: message.headers.text("To").unwrap_or_default(),
            subject: message.subject(), body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_round_trip_and_header_injection_refused() {
        let d = LocalDraft { to: "amy@example.org".into(), subject: "Café plans".into(), body: "Hello\nSecond line".into() };
        let raw = d.encode().unwrap();
        assert_eq!(crate::inbox::Summary::from_raw(&raw).from, "Draft");
        let parsed = message::parse(&raw);
        assert_eq!(LocalDraft::from_message(&parsed), Some(d.clone()));
        assert!(LocalDraft::from_message(&message::parse(b"From: a@b\r\n\r\nHi")).is_none());
        assert!(LocalDraft { to: "amy@example.org\r\nBcc: spy@example.org".into(), ..d.clone() }.encode().is_none());
        assert!(LocalDraft { subject: "Hi\nX-Evil: yes".into(), ..d }.encode().is_none());
    }

    #[test]
    fn draft_history_tracks_edits_across_save_and_unicode() {
        let mut draft = LocalDraft::default();
        let mut history = DraftHistory::new(None);
        assert!(history.dirty(&draft));
        for (field, ch) in [(0, 'é'), (1, 'P'), (2, 'α'), (2, '\n')] {
            assert!(history.edit(&mut draft, DraftEdit::Insert { field, ch }));
        }
        assert_eq!((&*draft.to, &*draft.subject, &*draft.body), ("é", "P", "α\n"));
        history.mark_saved(&draft);
        assert!(!history.dirty(&draft));
        assert!(history.undo(&mut draft));
        assert_eq!(draft.body, "α");
        assert!(history.dirty(&draft));
        assert!(history.redo(&mut draft));
        assert_eq!(draft.body, "α\n");
        assert!(!history.dirty(&draft));
        assert!(history.edit(&mut draft, DraftEdit::Delete { field: 0, ch: 'é' }));
        assert!(history.dirty(&draft));
        assert!(!history.redo(&mut draft), "new edit clears redo");
        assert!(history.undo(&mut draft));
        assert_eq!(draft.to, "é");
        assert!(!history.dirty(&draft));
    }
}
