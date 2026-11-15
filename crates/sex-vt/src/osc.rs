// SPDX-License-Identifier: GPL-3.0-only
//! The text protocols carried in OSC strings: OSC 8 hyperlinks, OSC 52 clipboard, OSC 7
//! working directory, OSC 9 and OSC 777 notifications (and ConEmu's OSC 9;4 progress), and
//! OSC 133 shell-integration marks.
//!
//! None of them acts on the system: what the program asked for is queued as [`Event`]s
//! for the terminal window to decide on ([`Screen::take_events`]), so a program cannot
//! read the clipboard or pop notifications unless the window agrees.

use crate::{Point, Screen};
use alloc::string::String;
use alloc::vec::Vec;

/// Something a program asked the terminal window for. The screen only reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// OSC 52 set: put these bytes on the clipboard. `target` is the selection the
    /// program named: `c` clipboard, `p` primary, `s` selection (`c` when it named none).
    Clipboard { target: char, set: Vec<u8> },
    /// OSC 52 query: the program wants to read the clipboard; answer with
    /// [`Screen::reply_clipboard`] if the window allows it.
    ClipboardQuery { target: char },
    /// OSC 9 (iTerm2) or OSC 777;notify (urxvt, foot): a desktop notification.
    Notify { title: String, body: String },
    /// OSC 7 (`file://host/path`) or ConEmu's OSC 9;9: the shell's working directory.
    Cwd(String),
    /// ConEmu's OSC 9;4: a progress indicator for the window or task bar.
    Progress(Progress),
}

/// ConEmu/Windows Terminal progress states (OSC 9;4;state;percent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// State 0: hide the indicator.
    Clear,
    /// State 1: normal, at this percentage.
    Set(u8),
    /// State 2: failed, at this percentage.
    Error(u8),
    /// State 3: busy without a known amount.
    Indeterminate,
    /// State 4: paused, at this percentage.
    Paused(u8),
}

/// One command as the shell marked it with OSC 133, in absolute rows (oldest history
/// line is row 0). `prompt` is where the prompt starts (A), `input` where the typed
/// command starts (B), `output` where its output starts (C), and `end` where it finished
/// (D), with the exit status the shell reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandMark {
    pub prompt: Point,
    pub input: Option<Point>,
    pub output: Option<Point>,
    pub end: Option<Point>,
    pub exit: Option<i32>,
}

/// A mark in global rows (absolute row plus the lines ever dropped), so trimming the
/// history never has to touch it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mark {
    pub prompt: (usize, usize),
    pub input: Option<(usize, usize)>,
    pub output: Option<(usize, usize)>,
    pub end: Option<(usize, usize)>,
    pub exit: Option<i32>,
}

/// One OSC 8 hyperlink in the link table.
#[derive(Clone, Debug)]
pub(crate) struct Link {
    pub uri: String,
    /// The program's `id=` parameter: cells far apart with the same id are one link.
    pub id: Option<Vec<u8>>,
}

const MAX_URI: usize = 4096;

impl Screen {
    /// Every OSC string not already handled by the title code.
    pub(crate) fn osc(&mut self, params: &[&[u8]]) {
        let Some(kind) = params.first() else { return };
        let rest = &params[1..];
        match *kind {
            b"8" => self.osc_hyperlink(rest),
            b"52" => self.osc_clipboard(params),
            b"7" => { if let Some(uri) = rest.first() { if let Some(path) = file_url_path(uri) { self.cwd_event(path); } } }
            b"9" => self.osc_9(rest),
            b"777" => {
                if rest.first() == Some(&&b"notify"[..]) {
                    let title = lossy(rest.get(1).copied().unwrap_or_default());
                    let body = join(rest.get(2..).unwrap_or_default());
                    self.ext.events.push(Event::Notify { title, body });
                }
            }
            b"133" => self.osc_133(rest),
            _ => {}
        }
    }

    // ---- Events ----

    /// What programs asked the window for since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<Event> { core::mem::take(&mut self.ext.events) }
    /// The working directory the shell last reported (OSC 7).
    pub fn cwd(&self) -> Option<&str> { self.ext.cwd.as_deref() }
    /// Answers an OSC 52 query with the clipboard's bytes (the window decides whether to).
    pub fn reply_clipboard(&mut self, target: char, data: &[u8]) {
        self.replies.extend_from_slice(b"\x1b]52;");
        let mut buf = [0u8; 4];
        self.replies.extend_from_slice(target.encode_utf8(&mut buf).as_bytes());
        self.replies.push(b';');
        base64_encode(data, &mut self.replies);
        self.replies.extend_from_slice(b"\x1b\\");
    }
    fn cwd_event(&mut self, path: String) {
        self.ext.cwd = Some(path.clone());
        self.ext.events.push(Event::Cwd(path));
    }

    fn osc_clipboard(&mut self, params: &[&[u8]]) {
        // A clipboard set longer than the parser keeps arrives cut off: drop it rather
        // than put half of it on the clipboard.
        let raw: usize = params.iter().map(|p| p.len() + 1).sum();
        if raw >= crate::OSC_MAX { return; }
        let target = params.get(1).and_then(|p| p.iter().find(|b| b"cpsq01234567".contains(b))).map(|&b| b as char).unwrap_or('c');
        let target = if target.is_ascii_digit() || target == 'q' { 's' } else { target };
        let data = params.get(2).copied().unwrap_or_default();
        if data == b"?" { self.ext.events.push(Event::ClipboardQuery { target }); return; }
        if let Some(set) = base64_decode(data) { self.ext.events.push(Event::Clipboard { target, set }); }
    }

    fn osc_9(&mut self, rest: &[&[u8]]) {
        match rest.first().copied() {
            None => {}
            Some(b"4") => {
                let num = |i: usize| rest.get(i).and_then(|p| parse_num(p)).unwrap_or(0);
                let pct = num(2).min(100) as u8;
                let p = match num(1) { 0 => Progress::Clear, 1 => Progress::Set(pct), 2 => Progress::Error(pct), 3 => Progress::Indeterminate, 4 => Progress::Paused(pct), _ => return };
                self.ext.events.push(Event::Progress(p));
            }
            Some(b"9") => {
                if let Some(p) = rest.get(1) {
                    let p = p.strip_prefix(b"\"").and_then(|p| p.strip_suffix(b"\"")).unwrap_or(p);
                    if !p.is_empty() { self.cwd_event(lossy(p)); }
                }
            }
            // ConEmu's other numbered codes (sleep, message boxes, ...) are not notifications.
            Some(code) if !code.is_empty() && code.len() <= 2 && code.iter().all(u8::is_ascii_digit) => {}
            Some(_) => self.ext.events.push(Event::Notify { title: String::new(), body: join(rest) }),
        }
    }

    // ---- OSC 8 hyperlinks ----

    fn osc_hyperlink(&mut self, rest: &[&[u8]]) {
        let uri = join(rest.get(1..).unwrap_or_default());
        if uri.is_empty() || uri.len() > MAX_URI { self.ext.link = 0; return; }
        let id = rest.first().and_then(|p| p.split(|&b| b == b':').find_map(|kv| kv.strip_prefix(b"id="))).filter(|id| !id.is_empty()).map(<[u8]>::to_vec);
        self.ext.link = self.intern_link(uri, id);
    }
    /// A link table index for this link, reusing an equal live entry.
    fn intern_link(&mut self, uri: String, id: Option<Vec<u8>>) -> u16 {
        let same = |l: &Option<Link>| l.as_ref().is_some_and(|l| l.uri == uri && l.id == id);
        // Most links are written once, right after one another: look at the newest few.
        if let Some(i) = self.ext.links.iter().rev().take(32).position(same) { return (self.ext.links.len() - i) as u16; }
        if id.is_some() { if let Some(i) = self.ext.links.iter().position(same) { return i as u16 + 1; } }
        if !self.ext.links.iter().any(Option::is_none) && self.ext.links.len() >= self.ext.link_gc_at.max(256) { self.collect_links(); }
        let entry = Some(Link { uri, id });
        if let Some(i) = self.ext.links.iter().position(Option::is_none) { self.ext.links[i] = entry; return i as u16 + 1; }
        if self.ext.links.len() >= u16::MAX as usize { return 0; }
        self.ext.links.push(entry);
        self.ext.links.len() as u16
    }
    /// Frees link table entries no cell refers to any more (their lines left the history
    /// or were overwritten). Runs when the table fills up, so it costs one sweep per
    /// doubling.
    fn collect_links(&mut self) {
        let mut live = alloc::vec![false; self.ext.links.len() + 1];
        let mut mark = |cells: &[crate::Cell]| for c in cells { if c.link != 0 { live[c.link as usize] = true; } };
        for l in self.history.iter().chain(self.lines.iter()) { mark(&l.cells); }
        if let Some((lines, _)) = &self.saved_primary { for l in lines { mark(&l.cells); } }
        live[self.ext.link as usize] = true;
        let mut kept = 0;
        for (i, l) in self.ext.links.iter_mut().enumerate() {
            if live[i + 1] { kept += 1; } else { *l = None; }
        }
        while self.ext.links.last().is_some_and(Option::is_none) { self.ext.links.pop(); }
        self.ext.link_gc_at = (kept * 2).max(256);
    }
    /// The URI of a link table index, as cells carry it.
    pub fn link(&self, id: u16) -> Option<&str> {
        if id == 0 { return None; }
        self.ext.links.get(id as usize - 1)?.as_ref().map(|l| l.uri.as_str())
    }
    /// The hyperlink under a visible cell, if the program wrote one there.
    pub fn link_at(&self, col: usize, row: usize) -> Option<&str> { self.link(self.visible(col, row).link) }
    /// How many link table entries are in use (tests and diagnostics).
    pub fn links_live(&self) -> usize { self.ext.links.iter().filter(|l| l.is_some()).count() }

    // ---- OSC 133 shell integration ----

    fn osc_133(&mut self, rest: &[&[u8]]) {
        if self.alternate() { return; }
        let Some(&kind) = rest.first().and_then(|p| p.first()) else { return };
        let here = (self.ext.dropped + self.history.len() + self.y, self.x.min(self.cols - 1));
        let last = self.ext.commands.back_mut();
        match (kind, last) {
            (b'A', last) => {
                // A prompt drawn again on the same row (a resize, a shell's repaint) is the same prompt.
                if let Some(m) = last { if m.prompt.0 == here.0 && m.output.is_none() { m.prompt = here; return; } }
                self.ext.commands.push_back(Mark { prompt: here, input: None, output: None, end: None, exit: None });
                // Bounded like the history it points into.
                while self.ext.commands.len() > self.history_max + self.rows { self.ext.commands.pop_front(); }
            }
            (b'B', Some(m)) => m.input = Some(here),
            (b'C', Some(m)) => m.output = Some(here),
            (b'D', Some(m)) => {
                if m.end.is_none() {
                    m.end = Some(here);
                    m.exit = rest.get(1).and_then(|p| core::str::from_utf8(p).ok()).and_then(|s| s.parse().ok());
                }
            }
            _ => {}
        }
    }
    fn mark_point(&self, (row, col): (usize, usize)) -> Point { Point { row: row - self.ext.dropped, col } }
    /// The commands the shell marked, oldest first.
    pub fn commands(&self) -> Vec<CommandMark> {
        self.ext.commands.iter().map(|m| CommandMark {
            prompt: self.mark_point(m.prompt),
            input: m.input.map(|p| self.mark_point(p)),
            output: m.output.map(|p| self.mark_point(p)),
            end: m.end.map(|p| self.mark_point(p)),
            exit: m.exit,
        }).collect()
    }
    /// The absolute rows where prompts start, oldest first.
    pub fn prompt_rows(&self) -> Vec<usize> { self.ext.commands.iter().map(|m| m.prompt.0 - self.ext.dropped).collect() }
    /// Scrolls the view so the previous (`dir < 0`) or next (`dir > 0`) prompt is at the
    /// top, relative to the row now at the top. Returns the prompt's absolute row; the
    /// next prompt past the last one brings the view back to the bottom.
    pub fn jump_prompt(&mut self, dir: isize) -> Option<usize> {
        // At the bottom there is nothing further down to jump to.
        if dir > 0 && self.view == 0 { return None; }
        let top = self.point_at(0, 0).row;
        let rows = self.prompt_rows();
        let target = if dir < 0 { rows.iter().rev().copied().find(|&r| r < top) } else { rows.iter().copied().find(|&r| r > top) };
        match target {
            Some(row) => { self.view_top(row); Some(row) }
            None if dir > 0 => { self.scroll_view(-(self.view as isize)); None }
            None => None,
        }
    }
    /// Puts absolute row `row` at the top of the view, as far as the history allows.
    pub(crate) fn view_top(&mut self, row: usize) {
        let h = self.history.len();
        let max = if self.alternate() { 0 } else { h };
        let next = h.saturating_sub(row).min(max);
        if next != self.view { self.view = next; self.all_dirty = true; }
    }
    /// The output of the command whose prompt is at or above absolute row `row`: from
    /// where it started (OSC 133 C, or the line after the command line) to the line
    /// before it ended or the next prompt began. `None` when no command is marked there
    /// or it printed nothing.
    pub fn command_output(&self, row: usize) -> Option<(Point, Point)> {
        let cmds = self.commands();
        let i = cmds.iter().rposition(|c| c.prompt.row <= row)?;
        let c = cmds[i];
        let start = c.output.or_else(|| c.input.map(|p| Point { row: p.row + 1, col: 0 }))?;
        let stop = c.end.or_else(|| cmds.get(i + 1).map(|n| n.prompt)).unwrap_or_else(|| {
            let (x, y) = self.cursor();
            Point { row: self.history.len() + y, col: x }
        });
        // The output ends just before `stop`; trailing blank lines are not part of it.
        let mut end_row = if stop.col == 0 { stop.row.checked_sub(1)? } else { stop.row };
        while end_row > start.row && self.line_text(end_row).is_empty() { end_row -= 1; }
        if end_row < start.row || (end_row == start.row && self.line_text(end_row).len() <= start.col) { return None; }
        let end_col = if end_row == stop.row { stop.col.saturating_sub(1) } else { self.line_text(end_row).chars().count().max(1) - 1 };
        Some((start, Point { row: end_row, col: end_col }))
    }
    /// Selects [`Screen::command_output`] for the command at or above `row`.
    pub fn select_command_output(&mut self, row: usize) -> bool {
        match self.command_output(row) { Some((a, b)) => { self.select(a, b); true } None => false }
    }
}

fn lossy(b: &[u8]) -> String { String::from_utf8_lossy(b).into_owned() }
/// OSC parameters joined back with the `;` the parser split them on.
fn join(parts: &[&[u8]]) -> String {
    let mut out = Vec::new();
    for (i, p) in parts.iter().enumerate() { if i > 0 { out.push(b';'); } out.extend_from_slice(p); }
    String::from_utf8_lossy(&out).into_owned()
}
fn parse_num(p: &[u8]) -> Option<u32> { core::str::from_utf8(p).ok()?.parse().ok() }

/// The path of a `file://host/path` URL (any `scheme://host/path` is taken the same way),
/// percent-decoded.
fn file_url_path(uri: &[u8]) -> Option<String> {
    let at = uri.windows(3).position(|w| w == b"://")?;
    let after = &uri[at + 3..];
    let slash = after.iter().position(|&b| b == b'/')?;
    let path = &after[slash..];
    let mut out = Vec::with_capacity(path.len());
    let mut i = 0;
    while i < path.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if path[i] == b'%' && i + 2 < path.len() {
            if let (Some(h), Some(l)) = (hex(path[i + 1]), hex(path[i + 2])) { out.push((h * 16 + l) as u8); i += 3; continue; }
        }
        out.push(path[i]);
        i += 1;
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard (or URL-safe) base64, padding optional, whitespace ignored. `None` if it is
/// not base64.
pub(crate) fn base64_decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for &b in data {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); acc &= (1 << bits) - 1; }
    }
    Some(out)
}

pub(crate) fn base64_encode(data: &[u8], out: &mut Vec<u8>) {
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { B64[(n >> (18 - 6 * i) & 63) as usize] } else { b'=' });
        }
    }
}
