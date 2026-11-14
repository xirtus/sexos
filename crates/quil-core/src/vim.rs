// SPDX-License-Identifier: GPL-3.0-only
//! Vim mode: normal, insert, visual (char, line, block) and command-line modes over the
//! same [`Editor`], so every Vim command also works with several cursors. Covers counts,
//! operators with motions and text objects, registers for yank and put, `.` repeat,
//! `/` and `?` search, `f`/`t` finds and the everyday `:` commands.

use crate::buffer::Buffer;
use crate::editor::{is_word, After, Editor, Sel};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VKey { Char(char), Esc, Enter, Backspace, Delete, Tab, Left, Right, Up, Down, Ctrl(char) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Normal, Insert, Visual, VisualLine, VisualBlock, Command, Search }

/// What the editor around Vim must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action { None, Save, Quit, ForceQuit, SaveQuit, Message(String) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind { Exclusive, Inclusive, Linewise }

enum Parse { Need, Bad, Done }

pub struct Vim {
    pub mode: Mode,
    pending: Vec<VKey>,
    /// What is typed after `:`, `/` or `?`.
    pub cmdline: String,
    search_forward: bool,
    register: Vec<u8>,
    linewise: bool,
    last_change: Vec<VKey>,
    recording: Option<Vec<VKey>>,
    replaying: bool,
    pub search: Vec<u8>,
    last_find: Option<(char, char)>,
    /// Visual mode's fixed end and moving end.
    vanchor: usize,
    vhead: usize,
    /// Lines a Ctrl+D or Ctrl+U moves; the window sets it from its height.
    pub half_page: usize,
}

impl Default for Vim { fn default() -> Self { Vim::new() } }

fn line_last_char(b: &Buffer, at: usize) -> usize {
    let (s, e) = (b.line_start(at), b.line_end(at));
    if e > s { b.prev_char(e) } else { s }
}
fn first_non_blank(b: &Buffer, at: usize) -> usize {
    let mut a = b.line_start(at);
    while matches!(b.byte(a), Some(b' ') | Some(b'\t')) { a += 1; }
    a
}
/// 0 blank, 1 word, 2 punctuation; `big` makes every non-blank one class.
fn class(c: Option<char>, big: bool) -> u8 {
    match c { None => 0, Some(c) if c.is_whitespace() => 0, Some(_) if big => 1, Some(c) if is_word(c) => 1, Some(_) => 2 }
}
fn word_fwd(b: &Buffer, mut at: usize, big: bool) -> usize {
    let n = b.len();
    let c0 = class(b.char_at(at), big);
    if c0 != 0 { while at < n && class(b.char_at(at), big) == c0 { at = b.next_char(at); } }
    // Blanks, but an empty line is a word of its own.
    while at < n && class(b.char_at(at), big) == 0 {
        if b.byte(at) == Some(b'\n') && b.byte(at + 1) == Some(b'\n') { return at + 1; }
        at = b.next_char(at);
    }
    at
}
fn word_back(b: &Buffer, mut at: usize, big: bool) -> usize {
    while at > 0 && class(b.char_at(b.prev_char(at)), big) == 0 { at = b.prev_char(at); }
    if at == 0 { return 0; }
    let c = class(b.char_at(b.prev_char(at)), big);
    while at > 0 && class(b.char_at(b.prev_char(at)), big) == c { at = b.prev_char(at); }
    at
}
fn word_end(b: &Buffer, at: usize, big: bool) -> usize {
    let n = b.len();
    let mut a = b.next_char(at);
    while a < n && class(b.char_at(a), big) == 0 { a = b.next_char(a); }
    let c = class(b.char_at(a), big);
    while a < n { let nx = b.next_char(a); if nx < n && class(b.char_at(nx), big) == c { a = nx } else { break } }
    a.min(n.saturating_sub(1))
}
fn matching(b: &Buffer, at: usize) -> Option<usize> {
    // The first bracket at or after the cursor on its line, then its partner.
    let end = b.line_end(at);
    let mut a = at;
    let (open, close, fwd) = loop {
        if a >= end { return None; }
        match b.byte(a)? { b'(' => break (b'(', b')', true), b'[' => break (b'[', b']', true), b'{' => break (b'{', b'}', true),
            b')' => break (b'(', b')', false), b']' => break (b'[', b']', false), b'}' => break (b'{', b'}', false), _ => a += 1 }
    };
    let mut depth = 0i32;
    let mut i = a;
    loop {
        let c = b.byte(i)?;
        if c == open { depth += if fwd { 1 } else { -1 }; }
        if c == close { depth += if fwd { -1 } else { 1 }; }
        if depth == 0 { return Some(i); }
        if fwd { i += 1; if i >= b.len() { return None; } } else { if i == 0 { return None; } i -= 1; }
    }
}
/// The innermost `open`…`close` pair around `at`: offsets of the two brackets.
fn enclosing(b: &Buffer, at: usize, open: u8, close: u8) -> Option<(usize, usize)> {
    let mut depth = 0;
    let mut i = at;
    let start = loop {
        let c = b.byte(i);
        if c == Some(close) && i != at { depth += 1; }
        if c == Some(open) { if depth == 0 { break i; } depth -= 1; }
        if i == 0 { return None; }
        i -= 1;
    };
    let mut depth = 0;
    let mut j = start + 1;
    loop {
        let c = b.byte(j)?;
        if c == open { depth += 1; }
        if c == close { if depth == 0 { return Some((start, j)); } depth -= 1; }
        j += 1;
    }
}

impl Vim {
    pub fn new() -> Vim {
        Vim { mode: Mode::Normal, pending: Vec::new(), cmdline: String::new(), search_forward: true, register: Vec::new(), linewise: false,
              last_change: Vec::new(), recording: None, replaying: false, search: Vec::new(), last_find: None, vanchor: 0, vhead: 0, half_page: 20 }
    }
    /// What the status line shows.
    pub fn label(&self) -> &'static str {
        match self.mode { Mode::Normal => "NORMAL", Mode::Insert => "INSERT", Mode::Visual => "VISUAL", Mode::VisualLine => "V-LINE",
            Mode::VisualBlock => "V-BLOCK", Mode::Command => "COMMAND", Mode::Search => "SEARCH" }
    }
    pub fn pending_text(&self) -> String {
        let mut s = String::new();
        for k in &self.pending { if let VKey::Char(c) = k { s.push(*c) } }
        s
    }
    /// Normal and visual modes draw a block cursor.
    pub fn block_cursor(&self) -> bool { !matches!(self.mode, Mode::Insert) }

    pub fn key(&mut self, ed: &mut Editor, k: VKey) -> Action {
        if let Some(r) = self.recording.as_mut() { if !self.replaying { r.push(k); } }
        match self.mode {
            Mode::Insert => { self.insert_key(ed, k); Action::None }
            Mode::Command | Mode::Search => self.cmdline_key(ed, k),
            _ => {
                if k == VKey::Esc && self.pending.is_empty() {
                    if self.mode == Mode::VisualBlock { self.mode = Mode::Normal; } else if self.mode != Mode::Normal { self.leave_visual(ed); } else { ed.collapse(); }
                    self.clamp(ed);
                    return Action::None;
                }
                if k == VKey::Esc { self.pending.clear(); return Action::None; }
                self.pending.push(k);
                let keys = self.pending.clone();
                let mut act = Action::None;
                match self.run(ed, &keys, &mut act) {
                    Parse::Need => {}
                    Parse::Bad | Parse::Done => self.pending.clear(),
                }
                if matches!(self.mode, Mode::Normal) { self.clamp(ed); }
                act
            }
        }
    }

    fn insert_key(&mut self, ed: &mut Editor, k: VKey) {
        match k {
            VKey::Esc => {
                self.mode = Mode::Normal;
                ed.end_change();
                if let Some(r) = self.recording.take() { if !self.replaying { self.last_change = r; } }
                let sels: Vec<Sel> = ed.sels.iter().map(|s| { let ls = ed.buf.line_start(s.head); Sel::cursor(if s.head > ls { ed.buf.prev_char(s.head) } else { s.head }) }).collect();
                ed.sels = sels;
                ed.normalize();
            }
            VKey::Char(c) => { let mut b = [0u8; 4]; ed.type_text(c.encode_utf8(&mut b).as_bytes()); }
            VKey::Enter => ed.type_text(b"\n"),
            VKey::Backspace => ed.backspace(),
            VKey::Delete => ed.delete_forward(),
            VKey::Tab => ed.tab(),
            VKey::Left => ed.move_all(crate::editor::Motion::Left, false),
            VKey::Right => ed.move_all(crate::editor::Motion::Right, false),
            VKey::Up => ed.move_all(crate::editor::Motion::Up, false),
            VKey::Down => ed.move_all(crate::editor::Motion::Down, false),
            VKey::Ctrl('w') => ed.delete_word_back(),
            VKey::Ctrl(_) => {}
        }
    }

    /// Keeps normal-mode cursors on a character, never on a line's newline.
    fn clamp(&self, ed: &mut Editor) {
        for s in &mut ed.sels {
            let h = s.head;
            let e = ed.buf.line_end(h);
            let h = if h >= e && e > ed.buf.line_start(h) { ed.buf.prev_char(e) } else { h };
            *s = Sel { anchor: h, head: h, col: s.col };
        }
        ed.normalize();
    }

    fn start_record(&mut self, keys: &[VKey]) {
        if !self.replaying { self.recording = Some(keys.to_vec()); }
    }
    fn finish_record(&mut self) {
        if let Some(r) = self.recording.take() { if !self.replaying { self.last_change = r; } }
    }

    fn run(&mut self, ed: &mut Editor, keys: &[VKey], act: &mut Action) -> Parse {
        let mut i = 0;
        let mut count = 0usize;
        while let Some(VKey::Char(c @ '0'..='9')) = keys.get(i) {
            if *c == '0' && count == 0 { break; }
            count = count * 10 + (*c as usize - '0' as usize);
            i += 1;
        }
        let Some(&k) = keys.get(i) else { return Parse::Need };
        i += 1;
        let n = count.max(1);
        let visual = matches!(self.mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock);

        // Operators.
        if let VKey::Char(op @ ('d' | 'c' | 'y' | '>' | '<')) = k {
            if visual && self.mode != Mode::VisualBlock { self.visual_op(ed, op, keys); return Parse::Done; }
            if self.mode == Mode::VisualBlock && op == 'd' { self.mode = Mode::Normal; return self.run(ed, &[VKey::Char('x')], act); }
            let mut count2 = 0usize;
            while let Some(VKey::Char(c @ '0'..='9')) = keys.get(i) {
                if *c == '0' && count2 == 0 { break; }
                count2 = count2 * 10 + (*c as usize - '0' as usize);
                i += 1;
            }
            let total = if count == 0 && count2 == 0 { 0 } else { n * count2.max(1) };
            let Some(&m) = keys.get(i) else { return Parse::Need };
            i += 1;
            if m == VKey::Char(op) {
                // dd, cc, yy, >>, <<: whole lines.
                self.apply_op(ed, op, keys, &mut |b, h| {
                    let mut e = h;
                    for _ in 1..total.max(1) { let ne = b.line_end(e); if ne < b.len() { e = ne + 1 } }
                    Some((b.line_start(h), e, Kind::Linewise))
                });
                return Parse::Done;
            }
            if let VKey::Char(obj @ ('i' | 'a')) = m {
                let Some(&VKey::Char(t)) = keys.get(i) else { return if keys.len() <= i { Parse::Need } else { Parse::Bad } };
                self.apply_op(ed, op, keys, &mut |b, h| text_object(b, h, obj == 'a', t).map(|(a, e)| (a, e, Kind::Exclusive)));
                return Parse::Done;
            }
            let rest = &keys[i - 1..];
            let mut need = false;
            let search = self.search.clone();
            let last_find = self.last_find;
            let mut found = None;
            self.apply_op(ed, op, keys, &mut |b, h| {
                match motion(b, h, rest, total, true, op == 'c', &search, last_find) {
                    M::Need => { need = true; None }
                    M::Bad => None,
                    M::To(t, kind, f) => { if f.is_some() { found = f; } Some((h, t, kind)) }
                }
            });
            if found.is_some() { self.last_find = found; }
            return if need { Parse::Need } else { Parse::Done };
        }

        let ch = if let VKey::Char(c) = k { Some(c) } else { None };
        match (k, ch) {
            (VKey::Ctrl('r'), _) => { for _ in 0..n { ed.redo(); } return Parse::Done; }
            (VKey::Ctrl('d'), _) | (VKey::Ctrl('u'), _) => {
                let m = if k == VKey::Ctrl('d') { crate::editor::Motion::PageDown(self.half_page) } else { crate::editor::Motion::PageUp(self.half_page) };
                ed.move_all(m, false);
                return Parse::Done;
            }
            (VKey::Ctrl('v'), _) => { ed.collapse(); self.mode = Mode::VisualBlock; return Parse::Done; }
            _ => {}
        }
        if self.mode == Mode::VisualBlock {
            match ch {
                Some('j') => { for _ in 0..n { ed.add_cursor(true); } return Parse::Done; }
                Some('k') => { for _ in 0..n { ed.add_cursor(false); } return Parse::Done; }
                Some('I' | 'A' | 'i' | 'a' | 'x' | 'c' | 's' | 'r' | '~') => { self.mode = Mode::Normal; }
                _ => {}
            }
        }
        if matches!(self.mode, Mode::Visual | Mode::VisualLine) {
            match ch {
                Some('o') => { core::mem::swap(&mut self.vanchor, &mut self.vhead); self.show_visual(ed); return Parse::Done; }
                Some('x') => { self.visual_op(ed, 'd', keys); return Parse::Done; }
                Some('s') => { self.visual_op(ed, 'c', keys); return Parse::Done; }
                Some('J') => { ed.join_lines(); self.leave_visual(ed); return Parse::Done; }
                Some('v') | Some('V') => {
                    let to = if ch == Some('v') { Mode::Visual } else { Mode::VisualLine };
                    if self.mode == to { self.leave_visual(ed); } else { self.mode = to; self.show_visual(ed); }
                    return Parse::Done;
                }
                Some('~') | Some('u') | Some('U') => {
                    let (a, e) = (ed.primary_sel().start(), ed.primary_sel().end());
                    let t = ed.buf.slice(a..e);
                    let s: String = String::from_utf8_lossy(&t).chars().map(|c| match ch { Some('u') => c.to_lowercase().next().unwrap_or(c), Some('U') => c.to_uppercase().next().unwrap_or(c), _ => toggle(c) }).collect();
                    ed.set_selection(a, e);
                    ed.edit(|_, _| (a..e, s.clone().into_bytes(), After::Start));
                    self.mode = Mode::Normal;
                    ed.set_cursor(a);
                    return Parse::Done;
                }
                Some('p') | Some('P') => {
                    let (a, e) = (ed.primary_sel().start(), ed.primary_sel().end());
                    let reg = self.register.clone();
                    ed.edit(|_, _| (a..e, reg.clone(), After::Start));
                    self.mode = Mode::Normal;
                    return Parse::Done;
                }
                Some(':') => { self.leave_visual(ed); self.mode = Mode::Command; self.cmdline.clear(); return Parse::Done; }
                _ => {}
            }
            // Motions move the visual head.
            let search = self.search.clone();
            match motion(&ed.buf, self.vhead, &keys[i - 1..], count, false, false, &search, self.last_find) {
                M::Need => return Parse::Need,
                M::Bad => return Parse::Bad,
                M::To(t, _, f) => { if f.is_some() { self.last_find = f; } self.vhead = t; self.show_visual(ed); return Parse::Done; }
            }
        }

        match ch {
            Some('i') | Some('a') | Some('I') | Some('A') | Some('o') | Some('O') => {
                self.start_record(keys);
                ed.begin_change();
                let c = ch.unwrap();
                if c == 'o' || c == 'O' {
                    ed.edit(|b, s| {
                        let ls = b.line_start(s.head);
                        let mut ind: Vec<u8> = b.slice(ls..b.line_end(s.head)).into_iter().take_while(|c| *c == b' ' || *c == b'\t').collect();
                        if c == 'o' { let e = b.line_end(s.head); let mut t = vec![b'\n']; t.append(&mut ind); (e..e, t, After::End) }
                        else { let n = ind.len(); ind.push(b'\n'); (ls..ls, ind, After::At(n)) }
                    });
                } else {
                    let b = &ed.buf;
                    let v: Vec<Sel> = ed.sels.iter().map(|s| Sel::cursor(match c {
                        'a' => if b.byte(s.head) == Some(b'\n') || s.head >= b.len() { s.head } else { b.next_char(s.head) },
                        'I' => first_non_blank(b, s.head), 'A' => b.line_end(s.head), _ => s.head,
                    })).collect();
                    ed.sels = v;
                    ed.normalize();
                }
                self.mode = Mode::Insert;
                Parse::Done
            }
            Some('v') => { ed.collapse(); self.mode = Mode::Visual; self.vanchor = ed.cursor(); self.vhead = ed.cursor(); self.show_visual(ed); Parse::Done }
            Some('V') => { ed.collapse(); self.mode = Mode::VisualLine; self.vanchor = ed.cursor(); self.vhead = ed.cursor(); self.show_visual(ed); Parse::Done }
            Some(':') => { self.mode = Mode::Command; self.cmdline.clear(); Parse::Done }
            Some('/') | Some('?') => { self.mode = Mode::Search; self.search_forward = ch == Some('/'); self.cmdline.clear(); Parse::Done }
            Some('u') => { for _ in 0..n { ed.undo(); } Parse::Done }
            Some('.') => {
                let keys = self.last_change.clone();
                self.pending.clear();
                self.replaying = true;
                for k in keys { let _ = self.key(ed, k); }
                if self.mode == Mode::Insert { let _ = self.key(ed, VKey::Esc); }
                self.replaying = false;
                Parse::Done
            }
            Some('x') | Some('X') | Some('D') | Some('C') | Some('s') | Some('S') | Some('Y') => {
                let map: &[VKey] = match ch.unwrap() {
                    'x' => &[VKey::Char('d'), VKey::Char('l')], 'X' => &[VKey::Char('d'), VKey::Char('h')],
                    'D' => &[VKey::Char('d'), VKey::Char('$')], 'C' => &[VKey::Char('c'), VKey::Char('$')],
                    's' => &[VKey::Char('c'), VKey::Char('l')], 'S' => &[VKey::Char('c'), VKey::Char('c')], _ => &[VKey::Char('y'), VKey::Char('y')],
                };
                let mut v: Vec<VKey> = Vec::new();
                if count > 0 { for d in alloc::format!("{count}").chars() { v.push(VKey::Char(d)); } }
                v.extend_from_slice(map);
                // Record the keys as typed, so `.` repeats `x`, not its expansion.
                let typed = keys.to_vec();
                let r = self.run(ed, &v, act);
                if ch != Some('C') && ch != Some('s') && ch != Some('S') && ch != Some('Y') { self.last_change = typed; }
                else if let Some(rec) = self.recording.as_mut() { *rec = typed; }
                r
            }
            Some('p') | Some('P') => {
                self.start_record(keys);
                let reg = self.register.clone();
                let line = self.linewise;
                let after = ch == Some('p');
                for _ in 0..n {
                    ed.edit(|b, s| {
                        if line {
                            let at = if after { b.next_line(s.head) } else { b.line_start(s.head) };
                            if after && at == b.len() && b.line_end(s.head) == b.len() {
                                let mut t = vec![b'\n']; t.extend_from_slice(reg.strip_suffix(b"\n").unwrap_or(&reg));
                                (at..at, t, After::At(1))
                            } else { (at..at, reg.clone(), After::Start) }
                        } else {
                            let at = if after && b.byte(s.head).is_some_and(|c| c != b'\n') { b.next_char(s.head) } else { s.head };
                            (at..at, reg.clone(), After::At(reg.len().saturating_sub(1)))
                        }
                    });
                }
                if line { let v: Vec<Sel> = ed.sels.iter().map(|s| Sel::cursor(first_non_blank(&ed.buf, s.head))).collect(); ed.sels = v; }
                self.finish_record();
                Parse::Done
            }
            Some('J') => { self.start_record(keys); for _ in 0..n.max(2) - 1 { ed.join_lines(); } self.finish_record(); Parse::Done }
            Some('r') => {
                let Some(&VKey::Char(c)) = keys.get(i) else { return if keys.len() <= i { Parse::Need } else { Parse::Bad } };
                self.start_record(keys);
                let mut t = [0u8; 4];
                let rep = c.encode_utf8(&mut t).as_bytes().to_vec();
                ed.edit(|b, s| {
                    let mut e = s.head;
                    for _ in 0..n { if b.byte(e).is_some_and(|c| c != b'\n') { e = b.next_char(e) } }
                    let k = b.slice(s.head..e).iter().filter(|c| **c & 0xC0 != 0x80).count();
                    let mut out = Vec::new();
                    for _ in 0..k { out.extend_from_slice(&rep); }
                    let l = out.len();
                    (s.head..e, out, After::At(l.saturating_sub(rep.len())))
                });
                self.finish_record();
                Parse::Done
            }
            Some('~') => {
                self.start_record(keys);
                ed.edit(|b, s| {
                    let mut e = s.head;
                    for _ in 0..n { if b.byte(e).is_some_and(|c| c != b'\n') { e = b.next_char(e) } }
                    let t: String = String::from_utf8_lossy(&b.slice(s.head..e)).chars().map(toggle).collect();
                    (s.head..e, t.into_bytes(), After::End)
                });
                self.finish_record();
                Parse::Done
            }
            Some('*') | Some('#') => {
                if let Some((a, e)) = ed.word_at(ed.cursor()) {
                    self.search = ed.buf.slice(a..e);
                    let fwd = ch == Some('*');
                    self.search_forward = fwd;
                    self.jump_search(ed, fwd, n);
                }
                Parse::Done
            }
            Some('z') => if keys.len() > i { Parse::Done } else { Parse::Need },
            _ => {
                // A motion.
                let search = self.search.clone();
                let lf = self.last_find;
                let mut need = false;
                let mut bad = false;
                let mut found = None;
                let sels: Vec<Sel> = ed.sels.clone();
                let mut out = Vec::new();
                for s in &sels {
                    match motion(&ed.buf, s.head, &keys[i - 1..], count, false, false, &search, lf) {
                        M::Need => need = true,
                        M::Bad => bad = true,
                        M::To(t, _, f) => { if f.is_some() { found = f; } out.push(Sel { anchor: t, head: t, col: None }); }
                    }
                }
                if need { return Parse::Need; }
                if bad { return Parse::Bad; }
                if found.is_some() { self.last_find = found; }
                // j and k keep the column.
                let vertical = matches!(keys.get(i - 1), Some(VKey::Char('j' | 'k')) | Some(VKey::Up | VKey::Down));
                if vertical {
                    let down = matches!(keys.get(i - 1), Some(VKey::Char('j')) | Some(VKey::Down));
                    let m = if down { crate::editor::Motion::PageDown(n) } else { crate::editor::Motion::PageUp(n) };
                    ed.move_all(m, false);
                } else {
                    ed.sels = out;
                    ed.normalize();
                }
                Parse::Done
            }
        }
    }

    /// Runs operator `op` over the range each selection's motion gives.
    fn apply_op(&mut self, ed: &mut Editor, op: char, keys: &[VKey], range: &mut dyn FnMut(&Buffer, usize) -> Option<(usize, usize, Kind)>) {
        let mut spans = Vec::new();
        for s in &ed.sels {
            let Some((a, b, kind)) = range(&ed.buf, s.head) else { continue };
            let (mut a, mut b) = (a.min(b), a.max(b));
            match kind {
                Kind::Inclusive => b = ed.buf.next_char(b).min(ed.buf.len()),
                Kind::Linewise => { a = ed.buf.line_start(a); b = ed.buf.next_line(b); }
                Kind::Exclusive => {}
            }
            spans.push((s.head, a, b, kind == Kind::Linewise));
        }
        if spans.is_empty() { return; }
        self.do_op(ed, op, keys, spans);
    }

    fn do_op(&mut self, ed: &mut Editor, op: char, keys: &[VKey], spans: Vec<(usize, usize, usize, bool)>) {
        let lw = spans[0].3;
        let (_, a0, b0, _) = spans[0];
        let mut reg = ed.buf.slice(a0..b0);
        if lw && !reg.ends_with(b"\n") { reg.push(b'\n'); }
        if op != '>' && op != '<' { self.register = reg; self.linewise = lw; }
        let find = |s: &Sel| spans.iter().find(|p| p.0 == s.head).copied();
        match op {
            'y' => {
                let v: Vec<Sel> = ed.sels.iter().map(|s| Sel::cursor(find(s).map_or(s.head, |p| if p.3 { s.head.min(p.1.max(ed.buf.line_start(s.head))) } else { p.1 }))).collect();
                ed.sels = v;
                ed.normalize();
            }
            'd' => {
                self.start_record(keys);
                ed.edit(|b, s| match find(s) {
                    Some((_, a, e, true)) => {
                        // Deleting the last lines takes the newline before them.
                        if e == b.len() && a > 0 && !b.slice(a..e).ends_with(b"\n") { (a - 1..e, vec![], After::Start) }
                        else { (a..e, vec![], After::Start) }
                    }
                    Some((_, a, e, false)) => (a..e, vec![], After::Start),
                    None => (s.head..s.head, vec![], After::Start),
                });
                if lw { let v: Vec<Sel> = ed.sels.iter().map(|s| Sel::cursor(first_non_blank(&ed.buf, s.head))).collect(); ed.sels = v; }
                self.finish_record();
            }
            'c' => {
                self.start_record(keys);
                ed.begin_change();
                ed.edit(|b, s| match find(s) {
                    Some((_, a, e, true)) => {
                        // cc keeps the indentation and one empty line.
                        let ind: Vec<u8> = b.slice(a..e).into_iter().take_while(|c| *c == b' ' || *c == b'\t').collect();
                        let keep_nl = b.slice(a..e).ends_with(b"\n");
                        let mut t = ind.clone();
                        if keep_nl { t.push(b'\n'); }
                        (a..e, t, After::At(ind.len()))
                    }
                    Some((_, a, e, false)) => (a..e, vec![], After::Start),
                    None => (s.head..s.head, vec![], After::Start),
                });
                self.mode = Mode::Insert;
            }
            '>' | '<' => {
                self.start_record(keys);
                let v: Vec<Sel> = spans.iter().map(|p| Sel { anchor: p.1, head: if p.2 > p.1 { p.2 - 1 } else { p.1 }, col: None }).collect();
                ed.sels = v;
                if op == '>' { ed.indent_lines() } else { ed.outdent_lines() }
                let v: Vec<Sel> = ed.sels.iter().map(|s| Sel::cursor(first_non_blank(&ed.buf, s.start()))).collect();
                ed.sels = v;
                ed.normalize();
                self.finish_record();
            }
            _ => {}
        }
    }

    fn visual_range(&self, ed: &Editor) -> (usize, usize, bool) {
        let (a, h) = (self.vanchor.min(self.vhead), self.vanchor.max(self.vhead));
        if self.mode == Mode::VisualLine { (ed.buf.line_start(a), ed.buf.next_line(h), true) } else { (a, ed.buf.next_char(h).min(ed.buf.len()), false) }
    }
    fn show_visual(&self, ed: &mut Editor) {
        let (a, b, _) = self.visual_range(ed);
        if self.vhead >= self.vanchor { ed.set_selection(a, b) } else { ed.set_selection(b, a) }
    }
    fn leave_visual(&mut self, ed: &mut Editor) {
        self.mode = Mode::Normal;
        let h = self.vhead.min(ed.buf.len());
        ed.set_cursor(h);
    }
    fn visual_op(&mut self, ed: &mut Editor, op: char, keys: &[VKey]) {
        let (a, b, lw) = self.visual_range(ed);
        self.mode = Mode::Normal;
        ed.set_cursor(a);
        self.do_op(ed, op, keys, vec![(a, a, b, lw)]);
    }

    fn jump_search(&mut self, ed: &mut Editor, forward: bool, n: usize) -> bool {
        if self.search.is_empty() { return false; }
        let pat = self.search.clone();
        let mut ok = false;
        let v: Vec<Sel> = ed.sels.iter().map(|s| {
            let mut at = s.head;
            for _ in 0..n {
                let hit = if forward { ed.buf.find(&pat, at + 1, false).or_else(|| ed.buf.find(&pat, 0, false)) }
                          else { ed.buf.rfind(&pat, at, false).or_else(|| ed.buf.rfind(&pat, ed.buf.len(), false)) };
                if let Some(h) = hit { at = h; ok = true; }
            }
            Sel::cursor(at)
        }).collect();
        ed.sels = v;
        ed.normalize();
        ok
    }

    fn cmdline_key(&mut self, ed: &mut Editor, k: VKey) -> Action {
        match k {
            VKey::Esc => { self.mode = Mode::Normal; self.cmdline.clear(); Action::None }
            VKey::Backspace => { if self.cmdline.pop().is_none() { self.mode = Mode::Normal; } Action::None }
            VKey::Char(c) => { self.cmdline.push(c); Action::None }
            VKey::Enter => {
                let line = core::mem::take(&mut self.cmdline);
                if self.mode == Mode::Search {
                    self.mode = Mode::Normal;
                    if !line.is_empty() { self.search = line.into_bytes(); }
                    let fwd = self.search_forward;
                    if !self.jump_search(ed, fwd, 1) { return Action::Message(String::from("Pattern not found")); }
                    self.clamp(ed);
                    return Action::None;
                }
                self.mode = Mode::Normal;
                let a = self.ex(ed, line.trim());
                self.clamp(ed);
                a
            }
            _ => Action::None,
        }
    }

    /// The `:` commands.
    pub fn ex(&mut self, ed: &mut Editor, cmd: &str) -> Action {
        match cmd {
            "w" => return Action::Save,
            "q" => return if ed.modified() { Action::Message(String::from("No write since last change (add ! to override)")) } else { Action::Quit },
            "q!" => return Action::ForceQuit,
            "wq" | "x" => return Action::SaveQuit,
            "noh" | "nohlsearch" => { self.search.clear(); return Action::None; }
            "" => return Action::None,
            _ => {}
        }
        if let Ok(n) = cmd.parse::<usize>() {
            let at = ed.buf.offset_of_line(n.saturating_sub(1)).unwrap_or(0);
            ed.set_cursor(first_non_blank(&ed.buf, at));
            return Action::None;
        }
        if cmd == "$" { let l = ed.buf.line_start(ed.buf.len()); ed.set_cursor(l); return Action::None; }
        // :s/pat/rep/[g] on the cursor's line, :%s on every line.
        let (all, rest) = if let Some(r) = cmd.strip_prefix("%s") { (true, r) } else if let Some(r) = cmd.strip_prefix('s') { (false, r) } else { return Action::Message(alloc::format!("Not an editor command: {cmd}")) };
        let Some(sep) = rest.chars().next() else { return Action::None };
        let parts: Vec<&str> = rest[sep.len_utf8()..].split(sep).collect();
        if parts.len() < 2 || parts[0].is_empty() { return Action::Message(String::from("Usage: :s/pattern/replacement/g")); }
        let (pat, rep, global) = (parts[0].as_bytes(), parts[1].as_bytes(), parts.get(2).is_some_and(|f| f.contains('g')));
        let (from, to) = if all { (0, ed.buf.len()) } else { let c = ed.cursor(); (ed.buf.line_start(c), ed.buf.line_end(c)) };
        let mut hits = Vec::new();
        let mut at = from;
        let mut last_line = usize::MAX;
        while let Some(h) = ed.buf.find(pat, at, false) {
            if h + pat.len() > to { break; }
            let l = ed.buf.line_start(h);
            if global || l != last_line { hits.push(h); last_line = l; }
            at = h + pat.len();
        }
        if hits.is_empty() { return Action::Message(String::from("Pattern not found")); }
        let n = hits.len();
        ed.sels = hits.iter().map(|h| Sel { anchor: *h, head: h + pat.len(), col: None }).collect();
        ed.primary = 0;
        let rep = rep.to_vec();
        ed.edit(|_, s| (s.start()..s.end(), rep.clone(), After::Start));
        ed.collapse();
        Action::Message(alloc::format!("{n} substitution{}", if n == 1 { "" } else { "s" }))
    }
}

fn toggle(c: char) -> char {
    if c.is_lowercase() { c.to_uppercase().next().unwrap_or(c) } else if c.is_uppercase() { c.to_lowercase().next().unwrap_or(c) } else { c }
}

enum M { Need, Bad, To(usize, Kind, Option<(char, char)>) }

/// Where motion `keys[0..]` goes from `h`, `n` times. `op` is true under an operator
/// (where `l` may reach the line's end and `w` stops at it); `change` makes `cw` act as `ce`.
#[allow(clippy::too_many_arguments)]
fn motion(b: &Buffer, h: usize, keys: &[VKey], count: usize, op: bool, change: bool, search: &[u8], last_find: Option<(char, char)>) -> M {
    let n = count.max(1);
    let Some(&k) = keys.first() else { return M::Need };
    let ch = match k {
        VKey::Char(c) => c,
        VKey::Left => 'h', VKey::Right => 'l', VKey::Up => 'k', VKey::Down => 'j', VKey::Backspace => 'h',
        VKey::Enter => '+',
        _ => return M::Bad,
    };
    let mut at = h;
    let kind = match ch {
        'h' => { let ls = b.line_start(h); for _ in 0..n { if at > ls { at = b.prev_char(at) } } Kind::Exclusive }
        'l' | ' ' => {
            let end = b.line_end(h);
            for _ in 0..n { let nx = b.next_char(at); if nx < end || (op && nx <= end) { at = nx } }
            Kind::Exclusive
        }
        'j' | '+' => { for _ in 0..n { let e = b.line_end(at); if e < b.len() { at = e + 1 } } if ch == '+' { at = first_non_blank(b, at) } else { at = b.at_column(at, b.column(h)) } Kind::Linewise }
        'k' | '-' => { for _ in 0..n { if let Some(p) = b.prev_line(at) { at = p } } if ch == '-' { at = first_non_blank(b, at) } else { at = b.at_column(at, b.column(h)) } Kind::Linewise }
        'w' | 'W' => {
            let big = ch == 'W';
            if change && class(b.char_at(h), big) != 0 {
                for i in 0..n {
                    // On a word's last character, `cw` changes just that character.
                    if i == 0 && class(b.char_at(b.next_char(at)), big) != class(b.char_at(at), big) { continue; }
                    at = word_end(b, at, big);
                }
                return M::To(at, Kind::Inclusive, None);
            }
            for _ in 0..n { at = word_fwd(b, at, big); }
            // Under an operator, `w` stops at the end of the line it started on.
            if op && at > b.line_end(h) && b.line_end(h) > h { at = b.line_end(h); }
            Kind::Exclusive
        }
        'b' | 'B' => { for _ in 0..n { at = word_back(b, at, ch == 'B') } Kind::Exclusive }
        'e' | 'E' => { for _ in 0..n { at = word_end(b, at, ch == 'E') } Kind::Inclusive }
        '0' => { at = b.line_start(h); Kind::Exclusive }
        '^' | '_' => { at = first_non_blank(b, h); Kind::Exclusive }
        '$' => {
            for _ in 1..n { let e = b.line_end(at); if e < b.len() { at = e + 1 } }
            at = if op { let e = b.line_end(at); if e > b.line_start(at) { b.prev_char(e) } else { e } } else { line_last_char(b, at) };
            if op && b.line_end(at) == b.line_start(at) { return M::To(at, Kind::Exclusive, None); }
            Kind::Inclusive
        }
        'G' => { at = if count > 0 { b.offset_of_line(n - 1).unwrap_or(0) } else { b.line_start(b.len()) }; at = first_non_blank(b, at); Kind::Linewise }
        'g' => {
            let Some(&k2) = keys.get(1) else { return M::Need };
            match k2 { VKey::Char('g') => { at = first_non_blank(b, b.offset_of_line(n - 1).unwrap_or(0)); Kind::Linewise }
                VKey::Char('_') => { at = line_last_char(b, h); Kind::Inclusive }
                _ => return M::Bad }
        }
        'f' | 't' | 'F' | 'T' | ';' | ',' => {
            let (kind_c, target) = if ch == ';' || ch == ',' {
                let Some((kc, t)) = last_find else { return M::Bad };
                let kc = if ch == ',' { match kc { 'f' => 'F', 'F' => 'f', 't' => 'T', _ => 't' } } else { kc };
                (kc, t)
            } else {
                let Some(&k2) = keys.get(1) else { return M::Need };
                let VKey::Char(t) = k2 else { return M::Bad };
                (ch, t)
            };
            let (ls, le) = (b.line_start(h), b.line_end(h));
            for i in 0..n {
                let mut p = at;
                let fwd = kind_c == 'f' || kind_c == 't';
                // t/T repeated with ; skip the character they stopped next to.
                if (kind_c == 't' || kind_c == 'T') && (i > 0 || ch == ';' || ch == ',') { p = if fwd { b.next_char(p) } else { b.prev_char(p) }; }
                let mut hit = None;
                loop {
                    p = if fwd { b.next_char(p) } else { if p <= ls { break } b.prev_char(p) };
                    if fwd && p >= le { break; }
                    if b.char_at(p) == Some(target) { hit = Some(p); break; }
                }
                let Some(p) = hit else { return M::Bad };
                at = match kind_c { 't' => b.prev_char(p), 'T' => b.next_char(p), _ => p };
            }
            let kind = if kind_c == 'f' || kind_c == 't' { Kind::Inclusive } else { Kind::Exclusive };
            let rec = if ch == ';' || ch == ',' { None } else { Some((kind_c, target)) };
            return M::To(at, kind, rec);
        }
        '}' | '{' => {
            for _ in 0..n {
                if ch == '}' {
                    let mut l = b.next_line(at);
                    while l < b.len() && b.line_end(l) != l { l = b.next_line(l); }
                    at = l;
                } else {
                    let mut l = b.line_start(at);
                    while let Some(p) = b.prev_line(l) { l = p; if b.line_end(l) == l { break; } }
                    at = l;
                }
            }
            Kind::Exclusive
        }
        '%' => { match matching(b, h) { Some(p) => at = p, None => return M::Bad } Kind::Inclusive }
        'n' | 'N' => {
            if search.is_empty() { return M::Bad; }
            for _ in 0..n {
                let hit = if ch == 'n' { b.find(search, at + 1, false).or_else(|| b.find(search, 0, false)) } else { b.rfind(search, at, false).or_else(|| b.rfind(search, b.len(), false)) };
                match hit { Some(p) => at = p, None => return M::Bad }
            }
            Kind::Exclusive
        }
        _ => return M::Bad,
    };
    M::To(at, kind, None)
}

/// `iw`, `aw`, `i"`, `a(`, …: the range the object covers around `h`.
fn text_object(b: &Buffer, h: usize, around: bool, t: char) -> Option<(usize, usize)> {
    match t {
        'w' | 'W' => {
            let big = t == 'W';
            let c = class(b.char_at(h), big);
            let mut a = h;
            while a > b.line_start(h) && class(b.char_at(b.prev_char(a)), big) == c { a = b.prev_char(a); }
            let mut e = h;
            while e < b.line_end(h) && class(b.char_at(e), big) == c { e = b.next_char(e); }
            if around {
                let e0 = e;
                while e < b.line_end(h) && class(b.char_at(e), big) == 0 && b.byte(e) != Some(b'\n') { e = b.next_char(e); }
                if e == e0 { while a > b.line_start(h) && class(b.char_at(b.prev_char(a)), big) == 0 { a = b.prev_char(a); } }
            }
            Some((a, e))
        }
        '"' | '\'' | '`' => {
            let q = t as u8;
            let (ls, le) = (b.line_start(h), b.line_end(h));
            let line = b.slice(ls..le);
            let pos = h - ls;
            let quotes: Vec<usize> = line.iter().enumerate().filter(|(_, c)| **c == q).map(|(i, _)| i).collect();
            let mut pair = None;
            for w in quotes.chunks(2) { if w.len() == 2 && w[0] <= pos && pos <= w[1] { pair = Some((w[0], w[1])); } }
            if pair.is_none() { if let Some(w) = quotes.chunks(2).find(|w| w.len() == 2 && w[0] > pos) { pair = Some((w[0], w[1])); } }
            let (s, e) = pair?;
            Some(if around { (ls + s, ls + e + 1) } else { (ls + s + 1, ls + e) })
        }
        '(' | ')' | 'b' | '[' | ']' | '{' | '}' | 'B' | '<' | '>' => {
            let (o, c) = match t { '(' | ')' | 'b' => (b'(', b')'), '[' | ']' => (b'[', b']'), '<' | '>' => (b'<', b'>'), _ => (b'{', b'}') };
            let (s, e) = enclosing(b, h, o, c)?;
            Some(if around { (s, e + 1) } else { (s + 1, e) })
        }
        'p' => {
            let mut a = b.line_start(h);
            while let Some(p) = b.prev_line(a) { if b.line_end(p) == p { break } a = p; }
            let mut e = b.next_line(h);
            while e < b.len() && b.line_end(e) != e { e = b.next_line(e); }
            if around { while e < b.len() && b.line_end(e) == e { e = b.next_line(e); } }
            Some((a, e))
        }
        _ => None,
    }
}

