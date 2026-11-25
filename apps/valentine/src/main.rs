// SPDX-License-Identifier: GPL-3.0-only
//! Valentine: a local mail viewer. It reads `.eml` files within its Files grant;
//! account-linked IMAP/SMTP flows will be supplied by Multipass in a later slice.
//! Enter opens a message, typing filters sender/subject/preview, Esc clears search,
//! Ctrl+R reloads the folder, Ctrl+G cycles bounded local folders, Up/Down chooses a message
//! and wheel scrolls its body.
//! Ctrl+N starts a draft, Ctrl+E edits an unsent draft, Tab moves fields and
//! Ctrl+S saves it; Ctrl+Z/Y undo and redo field edits. A quiet draft autosaves
//! after three seconds.
//! Ctrl+Shift+/ opens the offline keyboard reference; Escape closes it.
#![no_std]
#![no_main]
extern crate alloc;
mod draw;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use draw::{clip, Canvas, Face};
use sex_app::{files, settings, Event, Window};
use sex_mail::inbox::Summary;
use sex_mail::local_folders as folders;
use sex_mail::draft::{DraftEdit, DraftHistory, LocalDraft};
use sex_mail::message::{self, Message};
use sex_pdx::app_pointer::Kind;
use sex_theme::Tokens;

const MAX_MAIL: u64 = 4 << 20;
const MAX_LIST: usize = 200;
const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+Shift+/", "Show or close this keyboard reference"),
    ("Up / Down", "Choose a message in the local folder"),
    ("Enter", "Open the chosen message"),
    ("Type", "Filter sender, subject and preview"),
    ("Escape", "Clear search or leave a draft, saving changes"),
    ("Ctrl+R", "Reload local mail"),
    ("Ctrl+G", "Next local folder, including nested folders"),
    ("Ctrl+N", "Start an unsent local draft"),
    ("Ctrl+E", "Edit an opened unsent draft"),
    ("Tab / Enter", "Move between draft fields / insert a body line"),
    ("Ctrl+S", "Save the draft in Valentine's folder"),
    ("Ctrl+Z / Ctrl+Y", "Undo / redo a draft edit"),
    ("Ctrl+M", "Show mail account status"),
    ("Mouse wheel", "Scroll the opened message"),
    ("F11", "Close the Valentine window"),
];
static mut STORE: [u8; MAX_MAIL as usize] = [0; MAX_MAIL as usize];

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[valentine.fatal] {}", info);
    sex_pdx::task_finish(1);
}

#[derive(Clone, Copy)]
struct Palette { back: u32, panel: u32, field: u32, text: u32, dim: u32, accent: u32, select: u32, border: u32 }
impl Palette {
    fn from_tokens(t: Tokens) -> Self {
        let rgb = |c: u32| c & 0x00ff_ffff;
        Self { back: rgb(t.window), panel: rgb(t.surface), field: rgb(t.field),
            text: rgb(t.text), dim: rgb(t.text_dim), accent: rgb(t.accent),
            select: rgb(sex_theme::mix(t.surface, t.accent, 0.22)), border: rgb(t.border) }
    }
}

struct Item { name: Vec<u8>, summary: Summary }
struct Compose { draft: LocalDraft, path: Vec<u8>, field: u8, history: DraftHistory, changed_at: u64 }
impl Compose {
    fn new(draft: LocalDraft, path: Vec<u8>, saved: bool) -> Self {
        let history = DraftHistory::new(saved.then(|| draft.clone()));
        Self { draft, path, field: 0, history, changed_at: sex_pdx::get_ticks() }
    }
    fn dirty(&self) -> bool { self.history.dirty(&self.draft) }
    fn field_mut(&mut self) -> &mut String {
        match self.field { 0 => &mut self.draft.to, 1 => &mut self.draft.subject, _ => &mut self.draft.body }
    }
}
struct App {
    items: Vec<Item>, shown: Vec<usize>, picked: usize, opened: Option<usize>,
    folders: Vec<Vec<u8>>, folder: usize,
    mail: Option<Message>, query: String, status: String, scroll: usize,
    palette: Palette, generation: u64,
    compose: Option<Compose>, next_draft: u32, help: bool,
}
impl App {
    fn new() -> Self {
        let theme = settings::theme();
        let palette = Palette::from_tokens(theme);
        sex_pdx::serial_record!("[valentine.theme] mode={:?}", theme.mode);
        Self { items: Vec::new(), shown: Vec::new(), picked: 0, opened: None, mail: None,
            folders: vec![Vec::new()], folder: 0,
            query: String::new(), status: String::new(), scroll: 0, palette,
            generation: settings::generation().unwrap_or(0), compose: None, next_draft: 0, help: false }
    }
    fn filter(&mut self) {
        self.shown = self.items.iter().enumerate().filter_map(|(i, item)| item.summary.matches(&self.query).then_some(i)).collect();
        self.picked = self.picked.min(self.shown.len().saturating_sub(1));
        sex_pdx::serial_record!("[valentine.search] query={} matches={}", self.query, self.shown.len());
    }
    fn reload(&mut self, window: &mut Window) {
        let selected = self.folders.get(self.folder).cloned().unwrap_or_default();
        let previous = self.opened.and_then(|i| self.items.get(i)).map(|item| item.name.clone());
        self.items.clear(); self.opened = None; self.mail = None;
        let mut discovered = vec![Vec::new()];
        let mut scan = 0;
        while scan < discovered.len() && discovered.len() < folders::MAX_FOLDERS {
            let parent = discovered[scan].clone();
            if let Ok(entries) = files::list(window, &parent) {
                let mut children: Vec<_> = entries.into_iter().filter(|e| e.dir)
                    .filter_map(|e| folders::child(&parent, &e.name)).collect();
                children.sort();
                children.truncate(folders::MAX_FOLDERS - discovered.len());
                discovered.extend(children);
            }
            scan += 1;
        }
        discovered[1..].sort();
        self.folders = discovered;
        self.folder = self.folders.iter().position(|name| *name == selected).unwrap_or(0);
        let folder = self.folders.get(self.folder).cloned().unwrap_or_default();
        let entries = match files::list(window, &folder) {
            Ok(entries) => entries,
            Err(_) => { self.status = "Could not list this mail folder.".into(); return; }
        };
        let store = unsafe { &mut *core::ptr::addr_of_mut!(STORE) };
        for entry in entries.into_iter().filter(|e| !e.dir && e.name.to_ascii_lowercase().ends_with(b".eml")).take(MAX_LIST) {
            if entry.size > MAX_MAIL { continue; }
            let Some(path) = folders::file(&folder, &entry.name) else { continue };
            if let Ok(n) = files::read(window, &path, &mut store[..entry.size as usize]) {
                self.items.push(Item { name: path, summary: Summary::from_raw(&store[..n]) });
            }
        }
        self.items.sort_by(|a, b| a.name.cmp(&b.name));
        self.filter();
        if let Some(name) = previous {
            if let Some(i) = self.items.iter().position(|item| item.name == name) {
                self.open(i, window);
            }
        }
        if self.opened.is_none() && !self.shown.is_empty() { self.open(self.shown[0], window); }
        sex_pdx::serial_record!("[valentine.loaded] messages={} folder={}", self.items.len(), String::from_utf8_lossy(&folder));
    }
    fn open(&mut self, i: usize, window: &mut Window) {
        let Some(item) = self.items.get(i) else { return };
        let Ok(size) = files::size(window, &item.name) else { return };
        if size > MAX_MAIL { self.status = "Message exceeds the 4 MiB viewer limit.".into(); return; }
        let store = unsafe { &mut *core::ptr::addr_of_mut!(STORE) };
        match files::read(window, &item.name, &mut store[..size as usize]) {
            Ok(n) if n == size as usize => {
                self.mail = Some(message::parse(&store[..n])); self.opened = Some(i); self.scroll = 0;
                sex_pdx::serial_record!("[valentine.opened] file={} bytes={} subject={}",
                    String::from_utf8_lossy(&item.name), n, item.summary.subject);
            }
            _ => self.status = "Could not read the full message.".into(),
        }
    }
    fn new_draft(&mut self, window: &mut Window) {
        let stamp = sex_pdx::civil_time_ms().unwrap_or(sex_pdx::get_ticks());
        let path = loop {
            self.next_draft = self.next_draft.wrapping_add(1);
            let name = format!("draft-{}-{}-{}.eml", stamp, window.task, self.next_draft).into_bytes();
            let path = folders::file(self.folders.get(self.folder).map(Vec::as_slice).unwrap_or(b""), &name).unwrap();
            if files::size(window, &path).is_err() { break path; }
        };
        self.compose = Some(Compose::new(LocalDraft::default(), path, false));
        self.status = "New local draft. No message is sent.".into();
    }
    fn edit_draft(&mut self) {
        let Some(i) = self.opened else { return };
        let Some(draft) = self.mail.as_ref().and_then(LocalDraft::from_message) else {
            self.status = "Only unsent drafts can be edited.".into(); return;
        };
        self.compose = Some(Compose::new(draft, self.items[i].name.clone(), true));
    }
    fn save_draft(&mut self, window: &mut Window) -> bool {
        let Some(compose) = &self.compose else { return true };
        let Some(raw) = compose.draft.encode() else { self.status = "Header text contains a control character.".into(); return false };
        let path = compose.path.clone();
        let mut stage = path.clone(); stage.extend_from_slice(b".pending");
        let _ = files::remove(window, &stage);
        let result = (|| {
            files::create(window, &stage)?;
            if files::write_at(window, &stage, 0, &raw)? != raw.len() { return Err(sex_pdx::fs::NO_SPACE); }
            files::sync(window)?;
            files::rename(window, &stage, &path)?;
            files::sync(window)
        })();
        match result {
            Ok(()) => {
                if let Some(compose) = &mut self.compose { compose.history.mark_saved(&compose.draft); }
                self.status = "Draft saved in Valentine's folder. It was not sent.".into();
                sex_pdx::serial_record!("[valentine.draft.saved] path={} bytes={}", String::from_utf8_lossy(&path), raw.len());
                self.reload(window);
                if let Some(i) = self.items.iter().position(|item| item.name == path) {
                    self.open(i, window);
                    if let Some(row) = self.shown.iter().position(|&shown| shown == i) { self.picked = row; }
                }
                true
            }
            Err(e) => {
                self.status = format!("Draft not saved: {}", String::from_utf8_lossy(files::message(e)));
                sex_pdx::serial_record!("[valentine.draft.failed] status={}", e);
                false
            }
        }
    }
    fn compose_key(&mut self, window: &mut Window, k: sex_pdx::app_key::Key) -> bool {
        if self.compose.is_none() { return false; }
        if k.control && k.code == 0x1f { self.save_draft(window); return true; } // Ctrl+S
        if k.code == 0x01 { // Escape keeps a changed draft before leaving.
            if self.compose.as_ref().is_some_and(Compose::dirty) && !self.save_draft(window) { return true; }
            self.compose = None; return true;
        }
        let c = self.compose.as_mut().unwrap();
        if k.control && (k.code == 0x2c || k.code == 0x15) {
            let (action, changed) = if k.code == 0x2c {
                ("undo", c.history.undo(&mut c.draft))
            } else { ("redo", c.history.redo(&mut c.draft)) };
            if changed { c.changed_at = sex_pdx::get_ticks(); }
            sex_pdx::serial_record!("[valentine.draft.{}] changed={} dirty={}", action, changed as u8, c.dirty() as u8);
            return true;
        }
        let mut changed = false;
        match k.code {
            0x0f => c.field = (c.field + 1) % 3,
            0x1c if c.field < 2 => c.field += 1,
            0x1c => if c.draft.body.len() < 128 << 10 {
                changed = c.history.edit(&mut c.draft, DraftEdit::Insert { field: c.field, ch: '\n' });
            },
            0x0e => if let Some(ch) = c.field_mut().chars().last() {
                changed = c.history.edit(&mut c.draft, DraftEdit::Delete { field: c.field, ch });
            },
            _ if !k.control => if let Some(ch) = k.char().filter(|ch| !ch.is_control()) {
                let max = if c.field == 2 { 128 << 10 } else { 1024 };
                if c.field_mut().len() + ch.len_utf8() <= max {
                    changed = c.history.edit(&mut c.draft, DraftEdit::Insert { field: c.field, ch });
                }
            },
            _ => {},
        }
        if changed { c.changed_at = sex_pdx::get_ticks(); }
        true
    }
    fn key(&mut self, window: &mut Window, k: sex_pdx::app_key::Key) {
        if k.control && k.shift && k.code == 0x35 {
            self.help = !self.help;
            sex_pdx::serial_record!("[valentine.shortcuts] open={}", self.help as u8);
            return;
        }
        if self.help {
            if k.code == 0x01 {
                self.help = false;
                sex_pdx::serial_record!("[valentine.shortcuts] open=0");
            }
            return;
        }
        if self.compose_key(window, k) { return; }
        match (k.control, k.code) {
            (true, 0x32) => { // Ctrl+M: account linking needs the boot-routed Multipass service.
                self.status = "Mail accounts are unavailable in this build; local mail and drafts remain available.".into();
                sex_pdx::serial_record!("[valentine.account.unavailable] reason=multipass-service-not-booted");
                return;
            }
            (true, 0x31) => { self.new_draft(window); return; } // Ctrl+N
            (true, 0x12) => { self.edit_draft(); return; } // Ctrl+E
            (true, 0x13) => { self.reload(window); return; } // Ctrl+R
            (true, 0x22) => { // Ctrl+G: next bounded local folder, wrapping through root.
                if self.folders.len() > 1 {
                    self.folder = (self.folder + 1) % self.folders.len();
                    self.query.clear(); self.picked = 0;
                    self.reload(window);
                    let folder = self.folders.get(self.folder).map(Vec::as_slice).unwrap_or(b"");
                    sex_pdx::serial_record!("[valentine.folder] name={} messages={}", String::from_utf8_lossy(folder), self.items.len());
                }
                return;
            }
            (_, 0x48 | 0x67) => self.picked = self.picked.saturating_sub(1),
            (_, 0x50 | 0x6c) => self.picked = (self.picked + 1).min(self.shown.len().saturating_sub(1)),
            (_, 0x1c) => { if let Some(&i) = self.shown.get(self.picked) { self.open(i, window); } },
            (_, 0x01) => { self.query.clear(); self.picked = 0; self.filter(); },
            (_, 0x0e) => { self.query.pop(); self.picked = 0; self.filter(); },
            _ if !k.control => if let Some(c) = k.char().filter(|c| !c.is_control()) {
                self.query.push(c); self.picked = 0; self.filter();
            },
            _ => {},
        }
    }
    fn theme_poll(&mut self) -> bool {
        let Ok(generation) = settings::generation() else { return false };
        if generation == self.generation { return false; }
        self.generation = generation;
        let theme = settings::theme(); self.palette = Palette::from_tokens(theme);
        sex_pdx::serial_record!("[valentine.theme] mode={:?}", theme.mode);
        true
    }
}

fn render(window: &mut Window, app: &App, tiny: &mut Face, body: &mut Face, title: &mut Face) {
    let (w, h) = window.size();
    if w == 0 || h == 0 { return; }
    let (w, h) = (w as i32, h as i32);
    let mut c = Canvas { px: window.pixels(), w: w as usize, h: h as usize };
    let p = app.palette;
    let list_w = (w / 3).clamp(260, 420).min(w / 2);
    c.fill(0, 0, w, h, p.back);
    c.fill(0, 0, w, 42, p.panel);
    c.fill(0, 41, w, 1, p.border);
    c.text(title, 16, 6, "Valentine", p.accent, list_w - 24);
    let folder = app.folders.get(app.folder).map(Vec::as_slice).unwrap_or(b"");
    let folder_name = if folder.is_empty() { String::from("Home") } else { String::from_utf8_lossy(folder).into_owned() };
    c.text(tiny, list_w + 16, 12, &format!("Local mail: {folder_name} | No account linked"), p.dim, w - list_w - 24);
    if app.help {
        c.text(title, 24, 58, "Valentine shortcuts", p.text, w - 48);
        c.text(tiny, 24, 93, "Local mail and drafts. No message is sent by these controls.", p.dim, w - 48);
        for (i, (key, action)) in SHORTCUTS.iter().enumerate() {
            let y = 132 + i as i32 * 31;
            if y + 25 >= h - 32 { break; }
            c.text(body, 24, y, key, p.accent, 210);
            c.text(body, 240, y, action, p.text, w - 260);
        }
        c.fill(0, h - 28, w, 28, p.panel);
        c.fill(0, h - 28, w, 1, p.border);
        c.text(tiny, 12, h - 22, "Escape or Ctrl+Shift+/ closes this reference", p.dim, w - 24);
        window.show_pixels(w as usize, h as usize);
        return;
    }
    c.fill(0, 42, list_w, h - 70, p.panel);
    c.fill(list_w - 1, 42, 1, h - 42, p.border);
    c.text(tiny, 12, 50, &format!("Find: {}_   {} messages", app.query, app.shown.len()), p.dim, list_w - 20);
    let row_h = 58;
    let visible = ((h - 116) / row_h).max(1) as usize;
    let first = app.picked.saturating_sub(visible.saturating_sub(1));
    for (row, &i) in app.shown.iter().enumerate().skip(first).take(visible) {
        let y = 78 + (row - first) as i32 * row_h;
        if row == app.picked { c.fill(4, y, list_w - 8, row_h - 3, p.select); }
        let m = &app.items[i].summary;
        c.text(body, 12, y + 3, clip(&m.from, ((list_w - 28) / body.w as i32) as usize), p.text, list_w - 24);
        c.text(tiny, 12, y + 26, clip(&m.subject, ((list_w - 28) / tiny.w as i32) as usize), p.dim, list_w - 24);
        c.fill(12, y + row_h - 2, list_w - 24, 1, p.border);
    }
    let x = list_w + 20; let max = w - x - 16;
    if let Some(compose) = &app.compose {
        c.text(title, x, 56, "Unsent draft", p.accent, max);
        for (field, (label, value)) in [("To", &compose.draft.to), ("Subject", &compose.draft.subject)].iter().enumerate() {
            let y = 102 + field as i32 * 62;
            c.text(tiny, x, y, label, p.dim, max);
            c.fill(x, y + 18, max, 32, if compose.field == field as u8 { p.select } else { p.field });
            c.text(body, x + 8, y + 23, &format!("{}{}", clip(value, ((max / body.w as i32).max(1) as usize).saturating_sub(2)), if compose.field == field as u8 { "_" } else { "" }), p.text, max - 16);
        }
        c.text(tiny, x, 232, "Message", p.dim, max);
        c.fill(x, 250, max, h - 290, if compose.field == 2 { p.select } else { p.field });
        let lines: Vec<&str> = compose.draft.body.lines().collect();
        for (row, line) in lines.iter().take(((h - 320) / body.h as i32).max(0) as usize).enumerate() {
            c.text(body, x + 8, 258 + row as i32 * body.h as i32, clip(line, ((max - 16) / body.w as i32).max(1) as usize), p.text, max - 16);
        }
        if lines.is_empty() { c.text(body, x + 8, 258, "_", p.text, max - 16); }
        c.text(tiny, x, h - 54, "Tab next field  Enter newline  Ctrl+S save  Esc keep and close", p.dim, max);
    } else if let Some(mail) = &app.mail {
        c.text(title, x, 56, clip(&mail.subject(), (max / title.w as i32) as usize), p.text, max);
        c.text(body, x, 94, &format!("From: {}", mail.from()), p.dim, max);
        c.text(tiny, x, 121, &format!("Date: {}", mail.headers.text("date").unwrap_or_default()), p.dim, max);
        if !mail.attachments.is_empty() {
            c.text(tiny, x, 146, &format!("{} attachment(s) kept with the original message", mail.attachments.len()), p.accent, max);
        }
        let lines: Vec<&str> = mail.text.as_deref().unwrap_or("").lines().collect();
        let first = app.scroll.min(lines.len());
        if lines.is_empty() {
            let note = if mail.html.is_some() { "HTML mail is closed here; remote content is blocked." } else { "No plain-text body." };
            c.text(body, x, 178, note, p.dim, max);
        } else {
            for (row, line) in lines.iter().skip(first).take(((h - 220) / body.h as i32).max(0) as usize).enumerate() {
                c.text(body, x, 178 + row as i32 * body.h as i32, clip(line, (max / body.w as i32) as usize), p.text, max);
            }
        }
    } else {
        c.text(body, x, 80, "No local mail yet.", p.text, max);
        c.text(tiny, x, 110, "Put .eml files in Valentine's folder, then Ctrl+R.", p.dim, max);
        c.text(tiny, x, 132, "Account sync will arrive through Multipass.", p.dim, max);
    }
    c.fill(0, h - 28, w, 28, p.panel);
    c.fill(0, h - 28, w, 1, p.border);
    let status = if app.status.is_empty() { "Ctrl+G folder  Ctrl+N draft  Ctrl+E edit  Ctrl+M account  Enter open  Ctrl+R reload" } else { &app.status };
    c.text(tiny, 12, h - 22, status, p.dim, w - 24);
    window.show_pixels(w as usize, h as usize);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open(); window.title(b"Valentine");
    let (mut tiny, mut body, mut title) = (Face::new(8, 16), Face::new(10, 20), Face::new(16, 30));
    let mut app = App::new(); app.reload(&mut window);
    render(&mut window, &app, &mut tiny, &mut body, &mut title);
    sex_pdx::serial_record!("[valentine.ready] target={} task={}", window.target, window.task);
    loop {
        match window.next_within(1000) {
            Some(Event::Close) => {
                if app.compose.as_ref().is_some_and(Compose::dirty) { app.save_draft(&mut window); }
                sex_pdx::serial_record!("[valentine.closed] target={} task={}", window.target, window.task); window.close()
            },
            Some(Event::Key(k)) => { app.status.clear(); app.key(&mut window, k); render(&mut window, &app, &mut tiny, &mut body, &mut title); },
            Some(Event::Pointer(ptr)) => {
                match ptr.kind {
                    Kind::Wheel { steps } => { app.scroll = (app.scroll as i32 - steps as i32 * 3).max(0) as usize; },
                    Kind::Down { button: 0 } if i32::from(ptr.x) < (window.size().0 as i32 / 3).clamp(260, 420) && ptr.y >= 78 => {
                        let row = ((ptr.y - 78) / 58).max(0) as usize;
                        let first = app.picked.saturating_sub(((window.size().1 as i32 - 116) / 58).max(1) as usize - 1);
                        if let Some(&i) = app.shown.get(first + row) { app.picked = first + row; app.open(i, &mut window); }
                    },
                    _ => continue,
                }
                render(&mut window, &app, &mut tiny, &mut body, &mut title);
            },
            Some(Event::Resize { .. }) => render(&mut window, &app, &mut tiny, &mut body, &mut title),
            None => {
                let due = app.compose.as_ref().is_some_and(|c| c.dirty() && sex_pdx::get_ticks().saturating_sub(c.changed_at) >= 3000);
                if due { app.save_draft(&mut window); render(&mut window, &app, &mut tiny, &mut body, &mut title); }
                if app.theme_poll() { render(&mut window, &app, &mut tiny, &mut body, &mut title); }
            },
            _ => {},
        }
    }
}
