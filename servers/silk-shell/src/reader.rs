// SPDX-License-Identifier: GPL-3.0-only
//! Whisper, the screen reader (slice S7): what the Shell says, and when.
//!
//! Super+Alt+R turns it on. The voice is a confined program (`apps/whisper-voice`)
//! that Shell starts and sends text to (`sex_pdx::speech_voice`); this module only
//! decides the words. While on, the Shell speaks:
//!
//! * the focused window's title, each time focus moves (Alt+Tab steps move focus, so
//!   each switcher selection is spoken);
//! * the launcher's selected entry as it opens and as the selection moves;
//! * each notification as it is posted, summary then body;
//! * inside a program built on Weft, the focused control's name and role, from the
//!   tree the program sends while the reader is on (`sex_pdx::a11y`).
//!
//! Newer speech interrupts older: each utterance starts with the voice's FIRST piece,
//! and what is still queued for the voice is dropped. Pieces leave from a poll, one
//! call at a time, so a busy voice never stalls the Shell.
//!
//! It builds alone for host tests (`tests/screen_reader.rs`).
use alloc::vec::Vec;
use sex_pdx::{a11y, speech_voice as voice};

/// Pieces of one utterance at most.
pub const MAX_PIECES: usize = voice::MAX_TEXT / voice::PIECE_BYTES;

/// Speech waiting for the voice: the pieces of the newest utterance not yet sent.
pub struct Speech { pieces: [[u64; 3]; MAX_PIECES], len: usize, next: usize }

impl Speech {
    pub const fn new() -> Self { Self { pieces: [[0; 3]; MAX_PIECES], len: 0, next: 0 } }
    /// Replaces whatever is still waiting with `text`, which interrupts the voice.
    pub fn say(&mut self, text: &[u8]) {
        self.len = 0;
        self.next = 0;
        for p in voice::pieces(text, 0) {
            if self.len == MAX_PIECES { break; }
            self.pieces[self.len] = p;
            self.len += 1;
        }
    }
    /// Asks the voice to stop talking.
    pub fn hush(&mut self) { self.say(b""); }
    /// Drops what is waiting (the voice is gone).
    pub fn clear(&mut self) { self.len = 0; self.next = 0; }
    /// The next piece to send.
    pub fn front(&self) -> Option<[u64; 3]> { (self.next < self.len).then(|| self.pieces[self.next]) }
    /// The front piece reached the voice.
    pub fn sent(&mut self) { if self.next < self.len { self.next += 1; } }
    pub fn pending(&self) -> bool { self.next < self.len }
}

/// What the Shell shows now, as the reader looks at it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot<'a> {
    /// The focused surface (0: none).
    pub focus: u64,
    /// Its title, if it is a window with one.
    pub title: Option<&'a [u8]>,
    /// The launcher's selected row and its name, while the launcher is open.
    pub launcher: Option<(u8, &'a [u8])>,
    /// Notifications posted since the Shell started.
    pub notices: usize,
}

/// One thing to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Say<'a> {
    Title(&'a [u8]),
    Launcher(&'a [u8]),
    /// Notifications posted since the last look (the newest is the one to read).
    Notice(usize),
}

/// Remembers what was last spoken about, so only changes are spoken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watch { focus: u64, launcher: Option<u8>, notices: usize, primed: bool }

impl Watch {
    pub const fn new() -> Self { Self { focus: 0, launcher: None, notices: 0, primed: false } }
    /// Starts over: the next look takes the screen as it is and says nothing of it.
    pub fn reset(&mut self) { self.primed = false; }
    /// What changed since the last look, newest last. The launcher's selection wins
    /// over the focus move it causes (the launcher is not a window to name).
    pub fn look<'a>(&mut self, s: &Snapshot<'a>, out: &mut [Option<Say<'a>>; 2]) {
        *out = [None, None];
        if !self.primed {
            *self = Self { focus: s.focus, launcher: s.launcher.map(|l| l.0), notices: s.notices, primed: true };
            return;
        }
        let mut n = 0;
        if s.notices != self.notices {
            out[n] = Some(Say::Notice(s.notices.wrapping_sub(self.notices)));
            n += 1;
            self.notices = s.notices;
        }
        let selected = s.launcher.map(|l| l.0);
        if let Some((_, name)) = s.launcher {
            if selected != self.launcher && !name.is_empty() { out[n] = Some(Say::Launcher(name)); }
            self.launcher = selected;
            self.focus = s.focus;
            return;
        }
        self.launcher = None;
        if s.focus != self.focus {
            self.focus = s.focus;
            if let Some(title) = s.title.filter(|t| !t.is_empty()) { out[n] = Some(Say::Title(title)); }
        }
    }
}

/// Appends `parts` into `out`, cutting what does not fit. Returns the length.
pub fn join(parts: &[&[u8]], out: &mut [u8]) -> usize {
    let mut n = 0;
    for p in parts {
        let t = p.len().min(out.len() - n);
        out[n..n + t].copy_from_slice(&p[..t]);
        n += t;
    }
    n
}

/// A notification read out: "summary. body" (newlines become pauses).
pub fn notice_text(summary: &[u8], body: &[u8], out: &mut [u8]) -> usize {
    let mut n = if body.is_empty() { join(&[summary], out) } else { join(&[summary, b". ", body], out) };
    for b in &mut out[..n] { if *b == b'\n' { *b = b' '; } }
    while n > 0 && out[n - 1] == b' ' { n -= 1; }
    n
}

/// A program's accessibility tree as the Shell keeps it: each node's id, role, name
/// and value, and which node has focus. Only what speech needs.
pub struct Tree {
    nodes: Vec<(u64, u8, bool, Vec<u8>, Vec<u8>)>,
    focus: u64,
    gather: Vec<u8>,
    gathering: bool,
    /// Nodes kept at most, so a program cannot fill the Shell's memory.
    limit: usize,
}

/// What an update did to the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied { Refused, Same, FocusMoved }

impl Tree {
    pub const MAX_NODES: usize = 2048;
    pub const fn new() -> Self { Self { nodes: Vec::new(), focus: 0, gather: Vec::new(), gathering: false, limit: Self::MAX_NODES } }
    pub fn focus(&self) -> u64 { self.focus }
    pub fn len(&self) -> usize { self.nodes.len() }

    /// One `TREE` piece; a whole update is applied when its last piece arrives.
    pub fn piece(&mut self, p: &a11y::Piece) -> Option<Applied> {
        if p.first { self.gather.clear(); self.gathering = true; }
        if !self.gathering { return None; }
        if self.gather.len() + p.len > a11y::MAX_UPDATE { self.gather.clear(); self.gathering = false; return Some(Applied::Refused); }
        self.gather.extend_from_slice(p.data());
        if !p.last { return None; }
        self.gathering = false;
        let bytes = core::mem::take(&mut self.gather);
        let applied = self.apply(&bytes);
        self.gather = bytes;
        self.gather.clear();
        Some(applied)
    }

    /// One whole encoded update.
    pub fn apply(&mut self, bytes: &[u8]) -> Applied {
        let Some(head) = a11y::scan(bytes, |_| {}) else { return Applied::Refused };
        if head.full { self.nodes.clear(); }
        let limit = self.limit;
        let nodes = &mut self.nodes;
        a11y::scan(bytes, |n| {
            let entry = (n.id, n.role, n.disabled, n.label.to_vec(), n.value.to_vec());
            if let Some(e) = nodes.iter_mut().find(|e| e.0 == n.id) { *e = entry; }
            else if nodes.len() < limit { nodes.push(entry); }
        });
        let moved = head.focus != self.focus;
        self.focus = head.focus;
        if moved { Applied::FocusMoved } else { Applied::Same }
    }

    /// What to say for the focused node ("7 button"); 0 if there is nothing to name.
    pub fn focused_text(&self, out: &mut [u8]) -> usize {
        let Some(e) = self.nodes.iter().find(|e| e.0 == self.focus) else { return 0 };
        a11y::speak(&a11y::NodeText { id: e.0, role: e.1, hidden: false, disabled: e.2, label: &e.3, value: &e.4 }, out)
    }
}
