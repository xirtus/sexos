// SPDX-License-Identifier: GPL-3.0-only
//! Batch rename (Ctrl+Shift+R): every file in the folder gets a new name from one mask,
//! with the new names shown as it is typed and any conflict named before anything moves.
//! Enter renames in an order that never overwrites (swaps go through a temporary name);
//! Ctrl+Z takes the whole batch back in one step.
//!
//! The mask is the name, then `.` and the extension: `[N]` the old name, `[E]` the old
//! extension, `[C]` a counter (`[C10+5:3]` starts at 10, steps by 5, 3 digits), `[Y]`
//! `[M]` `[D]` `[h]` `[m]` `[s]` the modification time, `[U]` `[L]` `[F]` change case.
//! Without a `.`, the extension stays as it was.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use loom_core::rename::{self as rn, Item, Problem, Rules};
use sex_app::files::{self, direct};
use sex_app::Window;

use crate::{at, join, shown, App, Undo, BACKSPACE, ENTER, ESC};

/// Rows of the preview shown.
const SHOWN: usize = 14;

pub struct State {
    pub dir: Vec<u8>,
    pub items: Vec<Item>,
    /// Every name in the folder, folders included: what new names must not collide with.
    pub existing: Vec<Vec<u8>>,
    pub mask: Vec<u8>,
}

/// The name mask and the extension mask of what was typed.
fn rules(mask: &[u8]) -> Rules {
    // The last `.` outside brackets splits name from extension.
    let mut depth = 0i32;
    let mut split = None;
    for (i, b) in mask.iter().enumerate() {
        match b { b'[' => depth += 1, b']' => depth -= 1, b'.' if depth <= 0 => split = Some(i), _ => {} }
    }
    let (name, ext) = match split { Some(i) => (mask[..i].to_vec(), mask[i + 1..].to_vec()), None => (mask.to_vec(), b"[E]".to_vec()) };
    Rules { name, ext, ..Rules::default() }
}
fn problem(p: &Problem) -> String {
    match p {
        Problem::Empty => "empty".into(),
        Problem::BadChar(b) => format!("can't hold {}", if *b == 0 { "NUL" } else { "/" }),
        Problem::Dots => "can't be . or ..".into(),
        Problem::TooLong => "too long".into(),
        Problem::Duplicate => "two would get this name".into(),
        Problem::Exists => "already taken".into(),
    }
}

impl App {
    /// Ctrl+Shift+R: starts the mask prompt over the files of this folder.
    pub(crate) fn rename_start(&mut self, window: &mut Window) {
        if self.found.is_some() { self.status = "Batch rename works on a folder: Esc leaves the search results first.".into(); return; }
        let dir = self.cwd.clone();
        let items: Vec<Item> = self.entries.iter().filter(|e| !e.dir).map(|e| {
            let mtime = direct::stat(&at(&join(&dir, &e.name))).map(|s| s.modified).unwrap_or(0);
            Item { name: e.name.clone(), mtime, dir: false }
        }).collect();
        if items.is_empty() { self.status = "No files here to rename.".into(); return; }
        let existing = files::list(window, &at(&dir)).map(|l| l.into_iter().map(|e| e.name).collect()).unwrap_or_default();
        sex_pdx::serial_record!("[files.app.rename.start] dir={} items={}", shown(&dir), items.len());
        self.loom.mode = crate::loom::Mode::Rename(State { dir, items, existing, mask: b"[N]".to_vec() });
    }
    pub(crate) fn rename_key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let crate::loom::Mode::Rename(st) = &mut self.loom.mode else { return };
        match key.code {
            ESC => { self.loom.mode = crate::loom::Mode::None; self.status = "Nothing renamed.".into(); }
            BACKSPACE => { while let Some(b) = st.mask.pop() { if b & 0xC0 != 0x80 { break; } } }
            ENTER => self.rename_apply(window),
            _ => if let Some(c) = sex_app::text(key) { if st.mask.len() + c.len_utf8() <= 200 { st.mask.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()); } },
        }
    }
    fn rename_apply(&mut self, window: &mut Window) {
        let crate::loom::Mode::Rename(st) = &self.loom.mode else { return };
        let preview = match rn::preview(&rules(&st.mask), &st.items, &st.existing) {
            Ok(p) => p,
            Err(e) => { self.status = format!("The mask can't be read at {}: {}.", e.at + 1, e.why); return; }
        };
        let moves = match rn::plan(&preview, &st.existing) {
            Ok(m) => m,
            Err(b) => { self.status = format!("{} names have a problem: fix the mask first.", b.problems); return; }
        };
        let dir = st.dir.clone();
        let mask = st.mask.clone();
        self.loom.mode = crate::loom::Mode::None;
        if moves.is_empty() { self.status = "The mask gives every file its own name: nothing to rename.".into(); return; }
        let mut done: Vec<rn::Move> = Vec::new();
        for m in &moves {
            match files::rename(window, &at(&join(&dir, &m.from)), &at(&join(&dir, &m.to))) {
                Ok(()) => done.push(m.clone()),
                Err(e) => {
                    // Put back what already moved, so the folder is as it was.
                    for u in rn::undo(&done) { let _ = files::rename(window, &at(&join(&dir, &u.from)), &at(&join(&dir, &u.to))); }
                    let _ = files::sync(window);
                    sex_pdx::serial_record!("[files.app.rename.failed] dir={} at={} error={}", shown(&dir), shown(&m.from), e);
                    self.status = format!("Couldn't rename {}: {}. Nothing was renamed.", shown(&m.from), shown(files::message(e)));
                    self.load(window);
                    return;
                }
            }
        }
        let _ = files::sync(window);
        for m in &done { self.meta_moved(window, &join(&dir, &m.from), &join(&dir, &m.to)); }
        let changed = preview.changes();
        sex_pdx::serial_record!("[files.app.renamed] dir={} mask={} files={} moves={}", shown(&dir), shown(&mask), changed, done.len());
        for l in preview.lines.iter().filter(|l| l.changed()).take(20) { sex_pdx::serial_record!("[files.app.renamed.file] from={} to={}", shown(&l.from), shown(&l.to)); }
        let undo: Vec<(Vec<u8>, Vec<u8>)> = rn::undo(&done).into_iter().map(|m| (m.from, m.to)).collect();
        self.undo.push(Undo::Renamed { dir, moves: undo });
        self.load(window);
        self.status = format!("Renamed {} files. Ctrl+Z takes them all back.", changed);
    }
    /// Ctrl+Z after a batch rename: every move back, in an order that never overwrites.
    pub(crate) fn rename_undo(&mut self, window: &mut Window, dir: &[u8], moves: &[(Vec<u8>, Vec<u8>)]) -> Result<(), u8> {
        for (i, (f, t)) in moves.iter().enumerate() {
            if let Err(e) = files::rename(window, &at(&join(dir, f)), &at(&join(dir, t))) {
                // Redo what was already undone, so the batch stays whole either way.
                for (f2, t2) in moves[..i].iter().rev() { let _ = files::rename(window, &at(&join(dir, t2)), &at(&join(dir, f2))); }
                return Err(e);
            }
        }
        Ok(())
    }
    /// The prompt and the first rows of the preview, in place of the listing.
    pub(crate) fn rename_render(&self) -> Option<String> {
        let crate::loom::Mode::Rename(st) = &self.loom.mode else { return None };
        let mut t = format!("Rename {} files in {}\n\n", st.items.len(), shown(&st.dir));
        t.push_str(&format!("New name: {}|\n\n", shown(&st.mask)));
        match rn::preview(&rules(&st.mask), &st.items, &st.existing) {
            Err(e) => t.push_str(&format!("  The mask can't be read at {}: {}.\n", e.at + 1, e.why)),
            Ok(p) => {
                for l in p.lines.iter().take(SHOWN) {
                    let why: Vec<String> = l.problems.iter().map(problem).collect();
                    let mark = if l.ok() { "  " } else { "! " };
                    t.push_str(&format!("{}{}  ->  {}{}\n", mark, shown(&l.from), shown(&l.to), if why.is_empty() { String::new() } else { format!("   ({})", why.join(", ")) }));
                }
                if p.lines.len() > SHOWN { t.push_str(&format!("  ... {} more\n", p.lines.len() - SHOWN)); }
                t.push('\n');
                t.push_str(&if p.ok() { format!("{} of {} change. Enter renames them, Esc cancels.\n", p.changes(), p.lines.len()) } else { format!("{} names have a problem (marked !): Enter won't rename until they are fixed. Esc cancels.\n", p.problems()) });
            }
        }
        t.push_str("\n[N] name  [N2-5] its characters 2 to 5  [E] extension  [C] counter, [C10+5:3] from 10 by 5, 3 digits\n[Y] [M] [D] [h] [m] [s] modified date and time  [U] [L] [F] upper, lower, Title case\nText after the last . is the extension's mask; without one the extension stays.\n");
        Some(t)
    }
}
