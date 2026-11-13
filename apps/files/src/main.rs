// SPDX-License-Identifier: GPL-3.0-only
//! Files: the file manager, and the open/save picker other programs ask through.
//!
//! It holds only what the person handed it when starting it (`files` in Spindle gives
//! the whole filesystem, `files DIR` one folder), so it is a program like any other.
//! Copy, cut and paste work between the SexOS disk and USB sticks or other disks under
//! `/media`; Delete moves to the trash of the same disk; Ctrl+Z undoes, step by step.
//! When another program asks the person for a file, Files shows the question and hands
//! that program the one path chosen, nothing more.
#![no_std]
#![no_main]
extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::files::{self, Entry, Pick, Question};
use sex_app::{Event, Nav, Window};
use sex_pdx::fs as w;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[files.app.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const ROWS: usize = 18;
const TRASH: &[u8] = b".Trash";
const INFO: &[u8] = b".Trash/.info";
const CHUNK: usize = 192 * 1024;

const ENTER: u8 = 0x1C;
const BACKSPACE: u8 = 0x0E;
const ESC: u8 = 0x01;
const F2: u8 = 0x3C;
const F5: u8 = 0x3F;
// Letters, PS/2 set 1, used with Ctrl.
const KEY_C: u8 = 0x2E;
const KEY_X: u8 = 0x2D;
const KEY_V: u8 = 0x2F;
const KEY_Z: u8 = 0x2C;
const KEY_N: u8 = 0x31;
const KEY_T: u8 = 0x14;
const KEY_R: u8 = 0x13;
const KEY_H: u8 = 0x23;
const KEY_D: u8 = 0x20;
const KEY_S: u8 = 0x1F;

/// One step Ctrl+Z can take back.
enum Undo { Moved { from: Vec<u8>, to: Vec<u8> }, Trashed { from: Vec<u8>, to: Vec<u8>, info: Vec<u8> }, Copied { to: Vec<u8> }, Made { path: Vec<u8> } }
enum Input { Rename(Vec<u8>), NewFolder, SaveAs }
struct App {
    root: Vec<u8>,
    cwd: Vec<u8>,
    entries: Vec<Entry>,
    cursor: usize,
    top: usize,
    clip: Option<(Vec<u8>, bool)>,
    undo: Vec<Undo>,
    input: Option<(Input, Vec<u8>)>,
    status: String,
    question: Option<Question>,
    /// This run can see questions: it holds the whole filesystem.
    portal: bool,
}

/// The path as Files knows a granted one: `@/absolute/path`.
fn at(abs: &[u8]) -> Vec<u8> { let mut p = vec![b'@']; p.extend_from_slice(abs); p }
fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut p = dir.to_vec();
    if p.last() != Some(&b'/') { p.push(b'/'); }
    p.extend_from_slice(name);
    p
}
fn parent(abs: &[u8]) -> Vec<u8> {
    match abs.iter().rposition(|b| *b == b'/') { Some(0) | None => b"/".to_vec(), Some(i) => abs[..i].to_vec() }
}
fn base(abs: &[u8]) -> &[u8] { abs.rsplit(|b| *b == b'/').next().unwrap_or(abs) }
fn shown(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("?") }
/// The disk a path is on: a volume under /media, or the SexOS filesystem.
fn disk_root(abs: &[u8]) -> Vec<u8> {
    if let Some(rest) = abs.strip_prefix(b"/media/") {
        let name = rest.split(|b| *b == b'/').next().unwrap_or(rest);
        return join(b"/media", name);
    }
    b"/".to_vec()
}
fn human(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{} B", b),
        b if b < 1024 * 1024 => format!("{} KiB", b / 1024),
        b if b < 1024 * 1024 * 1024 => format!("{} MiB", b / (1024 * 1024)),
        b => format!("{}.{} GiB", b / (1024 * 1024 * 1024), b % (1024 * 1024 * 1024) * 10 / (1024 * 1024 * 1024)),
    }
}

impl App {
    fn load(&mut self, window: &mut Window) {
        match files::list(window, &at(&self.cwd)) {
            Ok(mut list) => {
                list.retain(|e| e.name != TRASH);
                list.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase())));
                self.entries = list;
            }
            Err(e) => { self.entries.clear(); self.status = format!("Can't open {}: {}", shown(&self.cwd), shown(files::message(e))); }
        }
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
        sex_pdx::serial_record!("[files.app.list] dir={} count={}", shown(&self.cwd), self.entries.len());
    }
    fn go(&mut self, window: &mut Window, dir: Vec<u8>) { self.cwd = dir; self.cursor = 0; self.top = 0; self.load(window); }
    fn selected(&self) -> Option<Vec<u8>> { self.entries.get(self.cursor).map(|e| join(&self.cwd, &e.name)) }
    fn select(&mut self, name: &[u8]) { if let Some(i) = self.entries.iter().position(|e| e.name == name) { self.cursor = i; } }

    fn render(&mut self, window: &mut Window) {
        let mut t = String::new();
        let space = files::space(window, &at(&self.cwd)).ok().filter(|s| s.0 > 0).map(|(total, free)| format!("  {} free of {}", human(free), human(total))).unwrap_or_default();
        t.push_str(&format!("Files  {}{}\n", shown(&self.cwd), space));
        if let Some(q) = &self.question {
            let what = match q.mode { Pick::Open => "open a file: Enter chooses it", Pick::Save => "save a file: Ctrl+S names it here", Pick::Folder => "pick a folder: Ctrl+S chooses this one" };
            t.push_str(&format!(">> {} asks to {}. Esc says no.\n", shown(&q.program), what));
        }
        t.push('\n');
        if self.entries.is_empty() { t.push_str("  (empty)\n"); }
        if self.cursor < self.top { self.top = self.cursor; }
        if self.cursor >= self.top + ROWS { self.top = self.cursor + 1 - ROWS; }
        for (i, e) in self.entries.iter().enumerate().skip(self.top).take(ROWS) {
            let mark = if i == self.cursor { "> " } else { "  " };
            if e.dir { t.push_str(&format!("{}{}/\n", mark, shown(&e.name))); } else { t.push_str(&format!("{}{}   {}\n", mark, shown(&e.name), human(e.size))); }
        }
        if self.entries.len() > self.top + ROWS { t.push_str(&format!("  ... {} more\n", self.entries.len() - self.top - ROWS)); }
        t.push('\n');
        if let Some((kind, text)) = &self.input {
            let label = match kind { Input::Rename(_) => "Rename to", Input::NewFolder => "New folder", Input::SaveAs => "Save as" };
            t.push_str(&format!("{}: {}|   (Enter confirms, Esc cancels)\n", label, shown(text)));
        } else if !self.status.is_empty() { t.push_str(&self.status); t.push('\n'); }
        t.push_str("Enter open  Backspace up  F2 rename\nCtrl+C/X/V copy, cut, paste  Ctrl+N folder\nDel trash  Ctrl+T show trash  Ctrl+R restore\nCtrl+Z undo  Ctrl+D disks  Ctrl+H home\n");
        window.show(t.as_bytes());
    }

    /// Copies a file or a whole folder, across disks too. Partial copies are removed.
    fn copy_all(&mut self, window: &mut Window, from: &[u8], to: &[u8]) -> Result<(), u8> {
        let st = files::stat(window, &at(from))?;
        if st.dir {
            files::mkdir(window, &at(to))?;
            for e in files::list(window, &at(from))? {
                if let Err(err) = self.copy_all(window, &join(from, &e.name), &join(to, &e.name)) { let _ = self.remove_all(window, to); return Err(err); }
            }
            return Ok(());
        }
        files::create(window, &at(to))?;
        let mut buf = vec![0u8; CHUNK];
        let mut offset = 0u64;
        let result = loop {
            let n = match files::read_at(window, &at(from), offset, &mut buf) { Ok(n) => n, Err(e) => break Err(e) };
            if n == 0 { break Ok(()); }
            if let Err(e) = files::write_at(window, &at(to), offset, &buf[..n]) { break Err(e); }
            offset += n as u64;
            if n < buf.len() { break Ok(()); }
        };
        if result.is_err() { let _ = files::remove(window, &at(to)); }
        result
    }
    fn remove_all(&mut self, window: &mut Window, path: &[u8]) -> Result<(), u8> {
        if files::stat(window, &at(path))?.dir {
            for e in files::list(window, &at(path))? { self.remove_all(window, &join(path, &e.name))?; }
        }
        files::remove(window, &at(path))
    }
    /// Rename where the disk allows it, else copy and remove.
    fn move_path(&mut self, window: &mut Window, from: &[u8], to: &[u8]) -> Result<(), u8> {
        match files::rename(window, &at(from), &at(to)) {
            Err(w::OTHER_DISK) => { self.copy_all(window, from, to)?; self.remove_all(window, from) }
            other => other,
        }
    }
    /// A name not yet taken in `dir`: `name`, `name (copy)`, `name (copy 2)`...
    fn free_name(&mut self, window: &mut Window, dir: &[u8], name: &[u8], tag: &str) -> Vec<u8> {
        let try_name = join(dir, name);
        if files::stat(window, &at(&try_name)).is_err() { return try_name; }
        let (stem, ext) = match name.iter().rposition(|b| *b == b'.') { Some(i) if i > 0 => (&name[..i], &name[i..]), _ => (name, &name[name.len()..]) };
        for k in 1..1000 {
            let mut n = stem.to_vec();
            n.extend_from_slice(if k == 1 { format!(" ({})", tag) } else { format!(" ({} {})", tag, k) }.as_bytes());
            n.extend_from_slice(ext);
            let p = join(dir, &n);
            if files::stat(window, &at(&p)).is_err() { return p; }
        }
        try_name
    }

    fn paste(&mut self, window: &mut Window) {
        let Some((from, cut)) = self.clip.clone() else { self.status = "Nothing to paste: Ctrl+C or Ctrl+X a file first.".into(); return };
        if self.cwd.starts_with(&from) && (self.cwd.len() == from.len() || self.cwd[from.len()] == b'/') { self.status = "A folder can't go inside itself.".into(); return; }
        let to = self.free_name(window, &self.cwd.clone(), base(&from), "copy");
        self.status = format!("{} {}...", if cut { "Moving" } else { "Copying" }, shown(base(&from)));
        self.render(window);
        let result = if cut { self.move_path(window, &from, &to) } else { self.copy_all(window, &from, &to) };
        match result {
            Ok(()) => {
                sex_pdx::serial_record!("[files.app.pasted] from={} to={} cut={}", shown(&from), shown(&to), cut as u8);
                self.status = format!("{} {}.", if cut { "Moved" } else { "Copied" }, shown(base(&to)));
                self.undo.push(if cut { Undo::Moved { from: from.clone(), to: to.clone() } } else { Undo::Copied { to: to.clone() } });
                if cut { self.clip = None; }
                let _ = files::sync(window);
                self.load(window);
                self.select(base(&to));
            }
            Err(e) => self.status = format!("Couldn't paste: {}.", shown(files::message(e))),
        }
    }
    fn trash(&mut self, window: &mut Window) {
        let Some(path) = self.selected() else { return };
        let root = disk_root(&path);
        let (trash, info) = (join(&root, TRASH), join(&root, INFO));
        let _ = files::mkdir(window, &at(&trash));
        let _ = files::mkdir(window, &at(&info));
        let to = self.free_name(window, &trash, base(&path), "trashed");
        match files::rename(window, &at(&path), &at(&to)) {
            Ok(()) => {
                // Where it came from, for Ctrl+R.
                let note = join(&info, base(&to));
                let _ = files::write(window, &at(&note), &path);
                sex_pdx::serial_record!("[files.app.trashed] from={} to={}", shown(&path), shown(&to));
                self.status = format!("Moved {} to the trash. Ctrl+Z puts it back.", shown(base(&path)));
                self.undo.push(Undo::Trashed { from: path, to, info: note });
                let _ = files::sync(window);
                self.load(window);
            }
            Err(e) => self.status = format!("Couldn't move to the trash: {}.", shown(files::message(e))),
        }
    }
    /// Puts the selected item of a trash folder back where it was.
    fn restore(&mut self, window: &mut Window) {
        let Some(path) = self.selected() else { return };
        if base(&self.cwd) != TRASH { self.status = "Ctrl+T opens the trash; Ctrl+R there puts the item back.".into(); return; }
        let note = join(&join(&self.cwd, b".info"), base(&path));
        let mut buf = [0u8; w::PATH_BYTES];
        let Ok(n) = files::read(window, &at(&note), &mut buf) else { self.status = "Where this came from isn't known.".into(); return };
        let back = buf[..n].to_vec();
        match files::rename(window, &at(&path), &at(&back)) {
            Ok(()) => { let _ = files::remove(window, &at(&note)); self.status = format!("Restored {}.", shown(&back)); self.undo.push(Undo::Moved { from: path, to: back }); let _ = files::sync(window); self.load(window); }
            Err(e) => self.status = format!("Couldn't restore: {}.", shown(files::message(e))),
        }
    }
    fn undo(&mut self, window: &mut Window) {
        let Some(step) = self.undo.pop() else { self.status = "Nothing to undo.".into(); return };
        let result = match &step {
            Undo::Moved { from, to } => self.move_path(window, to, from),
            Undo::Trashed { from, to, info } => self.move_path(window, to, from).map(|_| { let _ = files::remove(window, &at(info)); }),
            Undo::Copied { to } => self.remove_all(window, to),
            Undo::Made { path } => files::remove(window, &at(path)),
        };
        match result {
            Ok(()) => { sex_pdx::serial_record!("[files.app.undone] left={}", self.undo.len()); self.status = "Undone.".into(); let _ = files::sync(window); self.load(window); }
            Err(e) => { self.status = format!("Couldn't undo: {}.", shown(files::message(e))); self.undo.push(step); }
        }
    }
    fn finish_input(&mut self, window: &mut Window) {
        let Some((kind, text)) = self.input.take() else { return };
        if text.is_empty() || text.contains(&b'/') || text == b"." || text == b".." { self.status = "That name can't be used.".into(); return; }
        match kind {
            Input::Rename(from) => {
                let to = join(&parent(&from), &text);
                match files::rename(window, &at(&from), &at(&to)) {
                    Ok(()) => { self.status = format!("Renamed to {}.", shown(&text)); self.undo.push(Undo::Moved { from, to }); self.load(window); self.select(&text); }
                    Err(e) => self.status = format!("Couldn't rename: {}.", shown(files::message(e))),
                }
            }
            Input::NewFolder => {
                let path = join(&self.cwd, &text);
                match files::mkdir(window, &at(&path)) {
                    Ok(()) => { self.status = format!("Made {}.", shown(&text)); self.undo.push(Undo::Made { path }); self.load(window); self.select(&text); }
                    Err(e) => self.status = format!("Couldn't make the folder: {}.", shown(files::message(e))),
                }
            }
            Input::SaveAs => { let path = join(&self.cwd, &text); self.answer(window, Some(path)); }
        }
        let _ = files::sync(window);
    }
    /// Answers the waiting question with one path, or no.
    fn answer(&mut self, window: &mut Window, path: Option<Vec<u8>>) {
        let Some(q) = self.question.take() else { return };
        let token = path.as_ref().map(|p| at(p));
        match files::answer(window, &q, token.as_deref()) {
            Ok(()) => {
                sex_pdx::serial_record!("[files.app.answered] program={} path={}", shown(&q.program), path.as_deref().map(shown).unwrap_or("-"));
                self.status = match &path { Some(p) => format!("Gave {} to {}.", shown(p), shown(&q.program)), None => format!("Told {} no.", shown(&q.program)) };
            }
            Err(e) => self.status = format!("Couldn't answer: {}.", shown(files::message(e))),
        }
    }
    fn poll_questions(&mut self, window: &mut Window) -> bool {
        if !self.portal || self.question.is_some() { return false; }
        match files::questions(window) {
            Ok(mut qs) if !qs.is_empty() => {
                let q = qs.remove(0);
                sex_pdx::serial_record!("[files.app.question] program={} mode={:?}", shown(&q.program), q.mode);
                self.question = Some(q);
                true
            }
            Ok(_) => false,
            Err(_) => { self.portal = false; false }
        }
    }

    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        if let Some((_, text)) = self.input.as_mut() {
            match key.code {
                ENTER => self.finish_input(window),
                ESC => { self.input = None; }
                BACKSPACE => { while let Some(b) = text.pop() { if b & 0xC0 != 0x80 { break; } } }
                _ => if let Some(c) = sex_app::text(key) { if text.len() + c.len_utf8() <= 255 { text.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()); } },
            }
            return;
        }
        self.status.clear();
        if key.control {
            match key.code {
                KEY_C | KEY_X => if let Some(p) = self.selected() {
                    let cut = key.code == KEY_X;
                    self.status = format!("{} {}: Ctrl+V pastes it.", if cut { "Cut" } else { "Copied" }, shown(base(&p)));
                    self.clip = Some((p, cut));
                },
                KEY_V => self.paste(window),
                KEY_Z => self.undo(window),
                KEY_N => self.input = Some((Input::NewFolder, Vec::new())),
                KEY_T => { let t = join(&disk_root(&self.cwd), TRASH); self.go(window, t); }
                KEY_R => self.restore(window),
                KEY_H => { let r = self.root.clone(); self.go(window, r); }
                KEY_D => self.go(window, w::MEDIA.to_vec()),
                KEY_S => match self.question.as_ref().map(|q| q.mode) {
                    Some(Pick::Save) => self.input = Some((Input::SaveAs, self.question.as_ref().unwrap().suggestion.clone())),
                    Some(Pick::Folder) => { let c = self.cwd.clone(); self.answer(window, Some(c)); }
                    _ => {}
                },
                _ => {}
            }
            return;
        }
        match (key.code, sex_app::nav(key)) {
            (_, Some(Nav::Up)) => self.cursor = self.cursor.saturating_sub(1),
            (_, Some(Nav::Down)) => self.cursor = (self.cursor + 1).min(self.entries.len().saturating_sub(1)),
            (_, Some(Nav::Home)) => self.cursor = 0,
            (_, Some(Nav::End)) => self.cursor = self.entries.len().saturating_sub(1),
            (_, Some(Nav::Delete)) => self.trash(window),
            (ENTER, _) => {
                let Some(e) = self.entries.get(self.cursor) else { return };
                let path = join(&self.cwd, &e.name);
                if e.dir { self.go(window, path); }
                else if matches!(self.question.as_ref().map(|q| q.mode), Some(Pick::Open)) { self.answer(window, Some(path)); }
                else { self.status = format!("{}: {}. Programs open it with run NAME @{}.", shown(&e.name), human(e.size), shown(&path)); }
            }
            (BACKSPACE, _) => {
                // Never above what this run was given.
                if self.cwd != self.root && self.cwd.len() > 1 {
                    let here = base(&self.cwd).to_vec();
                    let up = parent(&self.cwd);
                    self.go(window, up);
                    self.select(&here);
                }
            }
            (ESC, _) => if self.question.is_some() { self.answer(window, None); },
            (F2, _) => if let Some(p) = self.selected() { let name = base(&p).to_vec(); self.input = Some((Input::Rename(p), name)); },
            (F5, _) => self.load(window),
            _ => {}
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Files");
    // The folder the person gave: `@/` is everything.
    let mut args = [0u8; w::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let root = given.split(|b| *b == b' ').find_map(|t| t.strip_prefix(b"@")).filter(|p| p.first() == Some(&b'/')).map(|p| p.to_vec());
    let Some(root) = root else {
        window.show(b"Files needs a folder to look after. Start it from Spindle with:  files  (everything)  or  files DIR\n\nF11 closes.");
        sex_pdx::serial_record!("[files.app.ready] root=none");
        loop { if let Event::Close = window.next() { window.close() } }
    };
    let portal = root == b"/";
    let mut app = App { cwd: root.clone(), root, entries: Vec::new(), cursor: 0, top: 0, clip: None, undo: Vec::new(), input: None, status: String::new(), question: None, portal };
    app.load(&mut window);
    app.poll_questions(&mut window);
    app.render(&mut window);
    sex_pdx::serial_record!("[files.app.ready] target={} task={} root={} portal={}", window.target, window.task, shown(&app.root), portal as u8);
    loop {
        match window.next_within(400) {
            Some(Event::Close) => { let _ = files::sync(&mut window); sex_pdx::serial_record!("[files.app.closed]"); window.close() }
            Some(Event::Key(key)) => { app.key(&mut window, key); app.render(&mut window); }
            Some(_) => {}
            // Quiet: another program may be asking the person for a file.
            None => if app.poll_questions(&mut window) { app.render(&mut window); },
        }
    }
}
