// SPDX-License-Identifier: GPL-3.0-only
//! Crush, the contacts: a list to search, a card to read and edit, vCard files in and
//! out, and two-way CardDAV sync. The cards live in the program's own folder
//! (`contacts.vcf`, see sex_vcard::Book); the CardDAV account in `dav.txt`.
//! `run crush @cards.vcf` imports cards; `run +net crush @account.dav` takes an
//! account. The model is sex-vcard's and the sync sex-dav's, both tested on the host.
//!
//! Keys in the list: typing searches (Backspace, Esc clears), Up/Down pick, Enter
//! edits, Delete removes, Ctrl+N a new card, Ctrl+S syncs, Ctrl+E exports every card
//! to `crush-export.vcf`. Editing: Tab or Up/Down move between fields, Enter saves,
//! Esc leaves without saving. The mouse picks a card or a field.
#![no_std]
#![no_main]

extern crate alloc;

mod draw;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use draw::{clip, Canvas, Face};
use sex_app::{files, Event, Nav, Window};
use sex_dav::dav::Kind as DavKind;
use sex_dav::sync::{self, Collection, Item, State};
use sex_dav::{DavAccount, DavResponse, DavTransport, FileAccount};
use sex_pdx::app_pointer::Kind;
use sex_vcard::{Address, Book, Contact, SyncState, Typed};

const FILE: &[u8] = b"contacts.vcf";
const ACCOUNT: &[u8] = b"dav.txt";
static mut STORE: [u8; 2 << 20] = [0; 2 << 20];

const BACK: u32 = 0x0017_191E;
const PANEL: u32 = 0x000F_1014;
const ACCENT: u32 = 0x00E5_7BA5;
const TEXT: u32 = 0x00E8_EAED;
const DIM: u32 = 0x0080_868E;
const SELECT: u32 = 0x003A_2C3F;
const FIELD: u32 = 0x0022_252C;

const KEY_N: u8 = 0x31;
const KEY_S: u8 = 0x1F;
const KEY_E: u8 = 0x12;
const FIELDS: [&str; 7] = ["Name", "Email", "Phone", "Organization", "Address", "Birthday", "Note"];

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[crush.fatal] {}", info);
    sex_pdx::task_finish(1);
}

struct Edit { index: Option<usize>, fields: [String; 7], at: usize }

struct App { book: Book, query: String, shown: Vec<usize>, picked: usize, edit: Option<Edit>, status: String, counter: u32 }

/// Requests over plain HTTP(S) with the account file's credentials.
// TODO(Multipass): a MultipassTransport over sex_pdx::accounts replaces this and dav.txt.
struct DirectHttp<'a> { window: &'a mut Window, origin: String, auth: Option<String> }

impl DavTransport for DirectHttp<'_> {
    fn request(&mut self, method: &str, path: &str, content_type: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<DavResponse, sex_dav::Error> {
        let url = format!("{}{}", self.origin, path);
        let mut all: Vec<(&str, &str)> = headers.to_vec();
        if let Some(a) = &self.auth { all.push(("Authorization", a.as_str())); }
        if !content_type.is_empty() { all.push(("Content-Type", content_type)); }
        let req = sex_fetch::Request { method, url: &url, headers: &all, body, trusted_origin: false, referrer: None };
        let r = sex_app::net::request(self.window, &req, None, 4 << 20).map_err(|e| sex_dav::Error::Transport(e as u32))?;
        sex_pdx::serial_record!("[crush.dav.request] method={} path={} status={}", method, path, r.head.status);
        Ok(DavResponse { status: r.head.status, etag: r.head.get("etag").map(String::from), body: r.body })
    }
}

fn first(list: &[Typed]) -> String { list.first().map(|t| t.value.clone()).unwrap_or_default() }

impl App {
    fn refresh(&mut self) {
        self.shown = self.book.search(&self.query);
        self.picked = self.picked.min(self.shown.len().saturating_sub(1));
    }
    fn save(&mut self, window: &mut Window) {
        let text = self.book.serialize();
        match files::write(window, FILE, text.as_bytes()) {
            Ok(()) => sex_pdx::serial_record!("[crush.saved] contacts={} bytes={}", self.book.contacts.len(), text.len()),
            Err(e) => self.status = format!("Not saved: {}", core::str::from_utf8(files::message(e)).unwrap_or("?")),
        }
    }
    fn read(&mut self, window: &mut Window, path: &[u8]) -> Option<String> {
        let store = unsafe { &mut *core::ptr::addr_of_mut!(STORE) };
        match files::read(window, path, store) {
            Ok(n) if n < store.len() => Some(String::from_utf8_lossy(&store[..n]).into_owned()),
            Ok(_) => { self.status = String::from("That file is too large."); None }
            Err(_) => None,
        }
    }
    fn import(&mut self, window: &mut Window, path: &[u8]) {
        let name = String::from(core::str::from_utf8(path).unwrap_or("?"));
        let Some(text) = self.read(window, path) else { self.status = format!("Could not read {name}."); return };
        if name.ends_with(".dav") {
            match FileAccount::parse(&text) {
                Some(a) => {
                    let _ = files::write(window, ACCOUNT, text.as_bytes());
                    sex_pdx::serial_record!("[crush.account] url={} user={}", a.url, a.user);
                    self.status = format!("CardDAV account {} kept; Ctrl+S syncs.", a.url);
                }
                None => self.status = format!("{name} is not an account."),
            }
            return;
        }
        match sex_vcard::parse(&text) {
            Ok(mut cards) => {
                let found = cards.len();
                for c in &mut cards { if c.uid.is_empty() { c.uid = self.new_uid(); } if !self.book.url.is_empty() { c.state = SyncState::New; } }
                let added = self.book.merge(cards);
                sex_pdx::serial_record!("[crush.imported] cards={} added={}", found, added);
                self.status = format!("Imported {found} cards from {name} ({added} new).");
                self.refresh();
                if added > 0 { self.save(window); }
            }
            Err(_) => self.status = format!("{name} is not a vCard file."),
        }
    }
    fn new_uid(&mut self) -> String {
        self.counter += 1;
        format!("{}-{}-{}@crush.sexos", sex_pdx::civil_time_ms().unwrap_or(0), self.counter, sex_pdx::get_ticks() % 100_000)
    }
    fn search_changed(&mut self) {
        self.picked = 0;
        self.refresh();
        sex_pdx::serial_record!("[crush.search] query={} matches={}", self.query, self.shown.len());
    }
    fn start_edit(&mut self, index: Option<usize>) {
        let c = index.map(|i| self.book.contacts[i].clone()).unwrap_or_default();
        let fields = [c.display(), first(&c.emails), first(&c.phones), c.org.clone(), c.addresses.first().map(|a| a.line()).unwrap_or_default(), c.birthday.clone(), c.note.clone()];
        let fields = if index.is_none() { Default::default() } else { fields };
        self.edit = Some(Edit { index, fields, at: 0 });
    }
    fn finish_edit(&mut self, window: &mut Window) {
        let Some(e) = self.edit.take() else { return };
        let f = &e.fields;
        if f.iter().all(|s| s.trim().is_empty()) { return; }
        let mut c = match e.index { Some(i) => self.book.contacts[i].clone(), None => Contact { uid: self.new_uid(), ..Contact::new("") } };
        let set_first = |list: &mut Vec<Typed>, v: &str| {
            let v = v.trim();
            match (list.first_mut(), v.is_empty()) { (Some(_), true) => { list.remove(0); } (Some(t), false) => t.value = v.into(), (None, false) => list.push(Typed::new("", v)), (None, true) => {} }
        };
        if c.display() != f[0].trim() { c.name = f[0].trim().into(); c.n = Default::default(); let mut w = c.name.rsplitn(2, ' '); c.n[0] = w.next().unwrap_or("").into(); c.n[1] = w.next().unwrap_or("").into(); }
        set_first(&mut c.emails, &f[1]);
        set_first(&mut c.phones, &f[2]);
        c.org = f[3].trim().into();
        let address = f[4].trim();
        if c.addresses.first().map(|a| a.line()).unwrap_or_default() != address {
            if address.is_empty() { c.addresses.clear(); } else {
                let a = Address { street: address.into(), ..Address::default() };
                if c.addresses.is_empty() { c.addresses.push(a); } else { c.addresses[0] = a; }
            }
        }
        c.birthday = f[5].trim().into();
        c.note = f[6].clone();
        sex_pdx::serial_record!("[crush.contact.saved] name={} email={} phone={} new={}", c.display(), first(&c.emails), first(&c.phones), e.index.is_none() as u8);
        self.status = format!("Saved {}.", c.display());
        match e.index {
            Some(i) => { if self.book.contacts[i] != c { c.touch(); self.book.contacts[i] = c; } }
            None => { if !self.book.url.is_empty() { c.state = SyncState::New; } self.book.contacts.push(c); }
        }
        self.refresh();
        self.save(window);
    }
    fn delete(&mut self, window: &mut Window) {
        let Some(&i) = self.shown.get(self.picked) else { return };
        let name = self.book.contacts[i].display();
        if self.book.contacts[i].href.is_empty() { self.book.contacts.remove(i); } else { self.book.contacts[i].state = SyncState::Deleted; }
        sex_pdx::serial_record!("[crush.removed] name={}", name);
        self.status = format!("Removed {name}.");
        self.refresh();
        self.save(window);
    }
    fn export(&mut self, window: &mut Window) {
        let cards: Vec<&Contact> = self.book.contacts.iter().filter(|c| c.state != SyncState::Deleted).collect();
        let text = sex_vcard::export(cards.iter().copied());
        match files::write(window, b"crush-export.vcf", text.as_bytes()) {
            Ok(()) => { sex_pdx::serial_record!("[crush.exported] cards={} bytes={}", cards.len(), text.len()); self.status = format!("Wrote {} cards to crush-export.vcf.", cards.len()); }
            Err(e) => self.status = format!("Not written: {}", core::str::from_utf8(files::message(e)).unwrap_or("?")),
        }
    }
    fn sync(&mut self, window: &mut Window) {
        let account = self.read(window, ACCOUNT).and_then(|t| FileAccount::parse(&t));
        let Some(account) = account else {
            self.status = String::from("No CardDAV account: run +net crush @account.dav (url=, user=, password=).");
            sex_pdx::serial_record!("[crush.sync.failed] reason=no-account");
            return;
        };
        let Some((origin, base)) = sex_dav::split_url(account.base_url()) else { return };
        let mut t = DirectHttp { window, origin: origin.into(), auth: account.auth_header() };
        let book = &mut self.book;
        let result = (|| -> Result<sync::Report, sex_dav::Error> {
            if book.url.is_empty() {
                let found = sync::discover(&mut t, base, DavKind::Contacts)?;
                book.url = found.first().map_or_else(|| String::from(base), |f| f.path.clone());
                sex_pdx::serial_record!("[crush.sync.found] books={} path={}", found.len(), book.url);
                // Cards made before there was a server go up.
                for c in &mut book.contacts { if c.href.is_empty() { c.state = SyncState::New; } }
            }
            let mut items: Vec<Item> = book.contacts.iter().map(|c| Item {
                href: c.href.clone(), etag: c.etag.clone(), uid: c.uid.clone(), data: sex_vcard::export([c]),
                state: match c.state { SyncState::Clean => State::Clean, SyncState::New => State::New, SyncState::Modified => State::Modified, SyncState::Deleted => State::Deleted },
            }).collect();
            let mut col = Collection { path: book.url.clone(), ctag: book.ctag.clone(), sync_token: book.sync_token.clone() };
            let report = sync::sync(&mut t, DavKind::Contacts, &mut col, &mut items)?;
            book.ctag = col.ctag;
            book.sync_token = col.sync_token;
            if !report.unchanged {
                book.contacts.clear();
                for item in &items {
                    for mut c in sex_vcard::parse(&item.data).unwrap_or_default() {
                        c.href = item.href.clone();
                        c.etag = item.etag.clone();
                        c.state = match item.state { State::Clean => SyncState::Clean, State::New => SyncState::New, State::Modified => SyncState::Modified, State::Deleted => SyncState::Deleted };
                        book.contacts.push(c);
                    }
                }
                // The server won; our copies stay here as notes, not sent anywhere.
                for note in &report.conflicts {
                    for mut c in sex_vcard::parse(note).unwrap_or_default() {
                        c.name = format!("Conflict: {}", c.display());
                        c.uid = format!("{}-conflict", c.uid);
                        book.contacts.push(c);
                    }
                }
            }
            Ok(report)
        })();
        match result {
            Ok(r) => {
                sex_pdx::serial_record!("[crush.sync] unchanged={} incremental={} pulled={} removed={} pushed={} deleted={} conflicts={} failed={} contacts={}",
                    r.unchanged as u8, r.incremental as u8, r.pulled, r.removed, r.pushed, r.deleted, r.conflicts.len(), r.failed, self.book.contacts.len());
                self.status = format!("Synced: {} from the server, {} to it.", r.pulled + r.removed, r.pushed + r.deleted);
            }
            Err(e) => { sex_pdx::serial_record!("[crush.sync.failed] error={:?}", e); self.status = format!("Sync failed: {e:?}"); }
        }
        self.refresh();
        self.save(window);
    }
    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        if let Some(e) = &mut self.edit {
            match (key.code, sex_app::nav(key)) {
                (0x1C, _) => self.finish_edit(window),
                (0x01, _) => self.edit = None,
                (0x0F, _) | (_, Some(Nav::Down)) => e.at = (e.at + if key.shift && key.code == 0x0F { FIELDS.len() - 1 } else { 1 }) % FIELDS.len(),
                (_, Some(Nav::Up)) => e.at = (e.at + FIELDS.len() - 1) % FIELDS.len(),
                (0x0E, _) => { e.fields[e.at].pop(); }
                _ => if let Some(c) = sex_app::text(key).filter(|c| !c.is_control()) { if e.fields[e.at].len() < 200 { e.fields[e.at].push(c); } },
            }
            return;
        }
        if key.control {
            match key.code { KEY_N => self.start_edit(None), KEY_S => self.sync(window), KEY_E => self.export(window), _ => {} }
            return;
        }
        match (key.code, sex_app::nav(key)) {
            (0x1C, _) => if let Some(&i) = self.shown.get(self.picked) { self.start_edit(Some(i)) },
            (0x01, _) => { if !self.query.is_empty() { self.query.clear(); self.search_changed(); } }
            (0x0E, _) => { if self.query.pop().is_some() { self.search_changed(); } }
            (_, Some(Nav::Up)) => self.picked = self.picked.saturating_sub(1),
            (_, Some(Nav::Down)) => self.picked = (self.picked + 1).min(self.shown.len().saturating_sub(1)),
            (_, Some(Nav::Delete)) => self.delete(window),
            _ => if let Some(c) = sex_app::text(key).filter(|c| !c.is_control()) { if self.query.len() < 60 { self.query.push(c); self.search_changed(); } },
        }
    }
}

struct Layout { w: i32, h: i32, list_w: i32 }
impl Layout {
    const TOP: i32 = 64;
    const ROW: i32 = 42;
    fn new((w, h): (usize, usize)) -> Layout { let (w, h) = (w.max(360) as i32, h.max(240) as i32); Layout { w, h, list_w: (w * 2 / 5).clamp(180, 420) } }
}

fn render(window: &mut Window, app: &App, small: &mut Face, tiny: &mut Face, big: &mut Face) {
    let l = Layout::new(window.size());
    let (w, h) = (l.w as usize, l.h as usize);
    let mut c = Canvas { px: window.pixels(), w, h };
    c.fill(0, 0, l.w, l.h, BACK);
    c.fill(0, 0, l.list_w, l.h, PANEL);
    c.text(big, 12, 6, "Crush", ACCENT, l.list_w - 20);
    let count = app.book.contacts.iter().filter(|c| c.state != SyncState::Deleted).count();
    c.text(tiny, 12 + 6 * big.w as i32, 16, &format!("{count} contacts"), DIM, l.list_w - 120);
    c.fill(8, 38, l.list_w - 16, 22, FIELD);
    let search = if app.query.is_empty() { String::from("Type to search") } else { format!("{}_", app.query) };
    c.text(small, 14, 39, &search, if app.query.is_empty() { DIM } else { TEXT }, l.list_w - 28);
    let chars = ((l.list_w - 24) / small.w as i32) as usize;
    for (row, &i) in app.shown.iter().enumerate() {
        let y = Layout::TOP + row as i32 * Layout::ROW;
        if y + Layout::ROW > l.h - 30 { break; }
        let card = &app.book.contacts[i];
        if row == app.picked { c.fill(4, y, l.list_w - 8, Layout::ROW - 2, SELECT); }
        c.text(small, 12, y + 2, clip(&card.display(), chars), TEXT, l.list_w - 20);
        let sub = [first(&card.emails), first(&card.phones), card.org.clone()].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
        let mark = match card.state { SyncState::New | SyncState::Modified => " (to sync)", _ => "" };
        c.text(tiny, 12, y + 2 + small.h as i32, &format!("{}{mark}", clip(&sub, chars)), DIM, l.list_w - 20);
    }
    // The card, or the form.
    let x = l.list_w + 16;
    let fw = l.w - x - 16;
    if let Some(e) = &app.edit {
        c.text(big, x, 6, if e.index.is_none() { "New contact" } else { "Edit contact" }, TEXT, fw);
        for (k, name) in FIELDS.iter().enumerate() {
            let y = Layout::TOP + k as i32 * 50;
            c.text(tiny, x, y, name, DIM, fw);
            c.fill(x, y + tiny.h as i32 + 2, fw, 26, if k == e.at { SELECT } else { FIELD });
            let v = if k == e.at { format!("{}_", e.fields[k]) } else { e.fields[k].clone() };
            let fit = ((fw - 12) / small.w as i32) as usize;
            let shown = if v.chars().count() > fit { &v[v.char_indices().rev().nth(fit - 1).map_or(0, |(i, _)| i)..] } else { &v[..] };
            c.text(small, x + 6, y + tiny.h as i32 + 5, shown, TEXT, fw - 12);
        }
    } else if let Some(card) = app.shown.get(app.picked).map(|&i| &app.book.contacts[i]) {
        c.text(big, x, 6, &card.display(), TEXT, fw);
        let mut y = Layout::TOP;
        let mut put = |c: &mut Canvas, label: &str, value: &str| {
            if value.is_empty() || y + 40 > l.h - 30 { return; }
            c.text(tiny, x, y, label, DIM, fw);
            c.text(small, x, y + tiny.h as i32, value, TEXT, fw);
            y += tiny.h as i32 + small.h as i32 + 8;
        };
        if !card.org.is_empty() || !card.title.is_empty() { put(&mut c, "Organization", &[card.title.as_str(), card.org.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ")); }
        for e in &card.emails { put(&mut c, &format!("Email {}", e.label()), &e.value); }
        for p in &card.phones { put(&mut c, &format!("Phone {}", p.label()), &p.value); }
        for a in &card.addresses { put(&mut c, "Address", &a.line()); }
        for u in &card.urls { put(&mut c, "Web", u); }
        put(&mut c, "Birthday", &card.birthday);
        put(&mut c, "Nickname", &card.nickname);
        for line in card.note.lines() { put(&mut c, "Note", line); }
        if !card.categories.is_empty() { put(&mut c, "Groups", &card.categories.join(", ")); }
        put(&mut c, "Server", &card.href);
    } else {
        c.text(small, x, Layout::TOP, "No contacts. Ctrl+N adds one; run crush @cards.vcf imports.", DIM, fw);
    }
    let bottom = if !app.status.is_empty() { app.status.clone() } else if app.edit.is_some() { String::from("Tab next field, Enter saves, Esc leaves") }
        else { String::from("Enter edits, Delete removes, Ctrl+N new, Ctrl+S sync, Ctrl+E export") };
    c.text(small, 12, l.h - 26, &bottom, DIM, l.w - 24);
    window.show_pixels(w, h);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Crush");
    let (mut small, mut tiny, mut big) = (Face::new(10, 20), Face::new(8, 16), Face::new(16, 30));
    let store = unsafe { &mut *core::ptr::addr_of_mut!(STORE) };
    let book = match files::read(&mut window, FILE, store) { Ok(n) => Book::parse(&String::from_utf8_lossy(&store[..n])), Err(_) => Book::default() };
    sex_pdx::serial_record!("[crush.loaded] contacts={}", book.contacts.len());
    let mut app = App { book, query: String::new(), shown: Vec::new(), picked: 0, edit: None, status: String::new(), counter: 0 };
    app.refresh();
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args).to_vec();
    for word in given.split(|b| *b == b' ').filter(|w| w.first() == Some(&b'@')) { app.import(&mut window, word); }
    render(&mut window, &app, &mut small, &mut tiny, &mut big);
    sex_pdx::serial_record!("[crush.ready] target={} task={} contacts={}", window.target, window.task, app.book.contacts.len());
    loop {
        match window.next() {
            Event::Close => { sex_pdx::serial_record!("[crush.closed] target={} task={}", window.target, window.task); window.close() }
            Event::Key(key) => { app.status.clear(); app.key(&mut window, key); }
            Event::Pointer(p) if matches!(p.kind, Kind::Down { button: 0 }) => {
                let l = Layout::new(window.size());
                let (x, y) = (p.x as i32, p.y as i32);
                if let Some(e) = &mut app.edit {
                    if x > l.list_w && y >= Layout::TOP { e.at = (((y - Layout::TOP) / 50) as usize).min(FIELDS.len() - 1); }
                } else if x < l.list_w && y >= Layout::TOP {
                    let row = ((y - Layout::TOP) / Layout::ROW) as usize;
                    if row < app.shown.len() { if row == app.picked { app.start_edit(Some(app.shown[row])); } else { app.picked = row; } }
                }
            }
            Event::Pointer(p) => match p.kind {
                Kind::Wheel { steps } if app.edit.is_none() => app.picked = (app.picked as i32 - steps as i32).clamp(0, app.shown.len().saturating_sub(1) as i32) as usize,
                _ => continue,
            },
            Event::Resize { .. } => {}
        }
        render(&mut window, &app, &mut small, &mut tiny, &mut big);
    }
}
