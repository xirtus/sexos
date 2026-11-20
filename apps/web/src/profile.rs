// SPDX-License-Identifier: GPL-3.0-only
//! What the browser keeps between runs, in Files under `Web/`: history, bookmarks,
//! cookies that outlast the session, and each origin's `localStorage`. Changes are
//! saved a moment after they happen (pages may write storage every frame), and at once
//! when the window closes.
//!
//! Each tab takes its cookies and storage from a [`Jarbox`]: the person's own, one of
//! the container boxes (Personal, Work, Banking, Shopping; each with its own files
//! under `Kaleidoscope/containers/<n>/`, so sites can't follow the person from one to
//! another), or the private box, which lives in memory only and is forgotten when its
//! last tab closes. Private tabs leave no history either.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use sex_app::{files, Window};
use sex_js::storage::WebStorage;
use sex_web::sex_html;

const DIR_STR: &str = "Kaleidoscope";
const DIR: &[u8] = DIR_STR.as_bytes();
const HISTORY: &[u8] = b"Kaleidoscope/history.tsv";
const BOOKMARKS: &[u8] = b"Kaleidoscope/bookmarks.tsv";
const COOKIES: &[u8] = b"Kaleidoscope/cookies.tsv";
const STORAGE: &[u8] = b"Kaleidoscope/storage.tsv";
/// Most history entries kept; the least recently visited go first.
const MAX_HISTORY: usize = 5000;
/// Biggest profile file read back.
const MAX_FILE: u64 = 64 << 20;
/// Changes wait this long (ticks, ms) before going to Files, so a burst saves once.
const SAVE_AFTER: u64 = 1500;

/// Where a tab keeps its cookies and storage.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Jarbox {
    #[default]
    Own,
    /// One of [`CONTAINERS`], numbered from 1.
    Container(u8),
    /// Memory only, forgotten when the last private tab closes.
    Private,
}

/// The container boxes: their names and colours (the tab's underline).
pub const CONTAINERS: &[(&str, u32)] = &[("Personal", 0xFF37_ADFF), ("Work", 0xFFFF_9F00), ("Banking", 0xFF51_CD00), ("Shopping", 0xFFFF_4BDA)];

impl Jarbox {
    /// The box's name for the bar, `None` for the person's own.
    pub fn name(self) -> Option<&'static str> {
        match self { Jarbox::Own => None, Jarbox::Private => Some("Private"), Jarbox::Container(n) => CONTAINERS.get(n as usize - 1).map(|c| c.0) }
    }
    /// The colour under the box's tabs, 0 for none.
    pub fn colour(self) -> u32 {
        match self { Jarbox::Own => 0, Jarbox::Private => 0xFF9A_4DFF, Jarbox::Container(n) => CONTAINERS.get(n as usize - 1).map_or(0, |c| c.1) }
    }
    /// For logs: own, private, or the container's number.
    pub fn tag(self) -> String {
        match self { Jarbox::Own => String::from("own"), Jarbox::Private => String::from("private"), Jarbox::Container(n) => alloc::format!("{n}") }
    }
}

/// A container's or the private box's cookies and storage.
pub struct Store {
    pub kind: Jarbox,
    pub jar: Rc<RefCell<sex_fetch::Jar>>,
    pub storage: Rc<RefCell<WebStorage>>,
    saved_jar: u64,
    saved_storage: u64,
}

/// A visited page: its address, title and how often it was opened.
pub struct Visit { pub url: String, pub title: String, pub visits: u32 }

#[derive(Default)]
pub struct Profile {
    /// Least recently visited first.
    pub history: Vec<Visit>,
    /// (address, title), oldest first.
    pub bookmarks: Vec<(String, String)>,
    pub storage: Rc<RefCell<WebStorage>>,
    /// The person's own cookies.
    pub jar: Rc<RefCell<sex_fetch::Jar>>,
    /// The container boxes opened this run, and the private box while it has tabs.
    pub boxes: Vec<Store>,
    history_dirty: bool,
    bookmarks_dirty: bool,
    saved_storage: u64,
    saved_jar: u64,
    /// When the pending changes go to Files.
    pub due: Option<u64>,
}

fn clean(s: &str) -> String { s.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect() }

fn read(window: &mut Window, path: &[u8]) -> Option<String> {
    let size = files::size(window, path).ok()?;
    if size == 0 || size > MAX_FILE { return None; }
    let mut buf = alloc::vec![0u8; size as usize];
    let n = files::read(window, path, &mut buf).ok()?;
    buf.truncate(n);
    String::from_utf8(buf).ok()
}

impl Profile {
    /// Reads the profile back, with the person's own cookies.
    pub fn load(window: &mut Window, now: u64) -> Profile {
        let mut p = Profile::default();
        let jar_rc = p.jar.clone();
        let mut jar_ref = jar_rc.borrow_mut();
        let jar = &mut *jar_ref;
        if let Some(t) = read(window, HISTORY) {
            for line in t.lines() {
                let mut f = line.splitn(3, '\t');
                let (Some(n), Some(url), title) = (f.next(), f.next(), f.next()) else { continue };
                let Ok(visits) = n.parse() else { continue };
                p.history.push(Visit { url: String::from(url), title: String::from(title.unwrap_or("")), visits });
            }
        }
        if let Some(t) = read(window, BOOKMARKS) {
            for line in t.lines() {
                let (url, title) = line.split_once('\t').unwrap_or((line, ""));
                if !url.is_empty() { p.bookmarks.push((String::from(url), String::from(title))); }
            }
        }
        if let Some(t) = read(window, COOKIES) { *jar = sex_fetch::Jar::load(&t, now); }
        if let Some(t) = read(window, STORAGE) { p.storage = Rc::new(RefCell::new(WebStorage::load(&t))); }
        p.saved_jar = jar.changes();
        sex_pdx::serial_record!("[web.profile.loaded] history={} bookmarks={} cookies={} origins={}", p.history.len(), p.bookmarks.len(), jar.len(), p.storage.borrow().origins().count());
        p
    }

    fn later(&mut self) { if self.due.is_none() { self.due = Some(sex_pdx::get_ticks() + SAVE_AFTER); } }

    /// Notes a page opened (`title` may come later through [`titled`](Self::titled)).
    pub fn visit(&mut self, url: &str) {
        if !(url.starts_with("http://") || url.starts_with("https://")) { return; }
        let url = sex_html::url::fragment(url).0;
        let visit = match self.history.iter().position(|v| v.url == url) {
            Some(i) => { let mut v = self.history.remove(i); v.visits = v.visits.saturating_add(1); v }
            None => Visit { url: clean(url), title: String::new(), visits: 1 },
        };
        self.history.push(visit);
        if self.history.len() > MAX_HISTORY { self.history.remove(0); }
        self.history_dirty = true;
        self.later();
    }

    /// The page at `url` has this title.
    pub fn titled(&mut self, url: &str, title: &str) {
        let url = sex_html::url::fragment(url).0;
        let title = clean(title.trim());
        if let Some(v) = self.history.iter_mut().rev().find(|v| v.url == url) {
            if v.title != title { v.title = title.clone(); self.history_dirty = true; self.later(); }
        }
        if let Some(b) = self.bookmarks.iter_mut().find(|b| b.0 == url) {
            if b.1.is_empty() && !title.is_empty() { b.1 = title; self.bookmarks_dirty = true; self.later(); }
        }
    }

    pub fn bookmarked(&self, url: &str) -> bool { self.bookmarks.iter().any(|b| b.0 == url) }

    /// Bookmarks the page, or takes its bookmark away. True when it is bookmarked now.
    pub fn toggle_bookmark(&mut self, url: &str, title: &str) -> bool {
        let now = match self.bookmarks.iter().position(|b| b.0 == url) {
            Some(i) => { self.bookmarks.remove(i); false }
            None => { self.bookmarks.push((clean(url), clean(title.trim()))); true }
        };
        self.bookmarks_dirty = true;
        self.later();
        now
    }

    /// Addresses to offer for what is typed in the bar: bookmarks first, then history
    /// by how often it was visited. A match is the typed text at the start of the
    /// address (after its scheme and `www.`), or anywhere in the title.
    pub fn suggest(&self, typed: &str, most: usize) -> Vec<String> {
        let typed = typed.trim().to_ascii_lowercase();
        if typed.is_empty() { return Vec::new(); }
        let fits = |url: &str, title: &str| {
            let u = url.to_ascii_lowercase();
            let bare = u.trim_start_matches("https://").trim_start_matches("http://");
            u.starts_with(&typed) || bare.starts_with(&typed) || bare.trim_start_matches("www.").starts_with(&typed)
                || (typed.len() >= 3 && title.to_ascii_lowercase().contains(&typed))
        };
        let mut out: Vec<String> = Vec::new();
        for (url, title) in self.bookmarks.iter().rev() {
            if out.len() < most && fits(url, title) && !out.contains(url) { out.push(url.clone()); }
        }
        let mut seen: Vec<&Visit> = self.history.iter().filter(|v| fits(&v.url, &v.title)).collect();
        seen.sort_by(|a, b| b.visits.cmp(&a.visits));
        for v in seen {
            if out.len() >= most { break; }
            if !out.contains(&v.url) { out.push(v.url.clone()); }
        }
        out
    }

    /// The cookies and storage of box `kind`, read from Files the first time a
    /// container is used this run.
    pub fn store(&mut self, window: &mut Window, kind: Jarbox, now: u64) -> (Rc<RefCell<sex_fetch::Jar>>, Rc<RefCell<WebStorage>>) {
        if kind == Jarbox::Own { return (self.jar.clone(), self.storage.clone()); }
        if let Some(s) = self.boxes.iter().find(|s| s.kind == kind) { return (s.jar.clone(), s.storage.clone()); }
        let (mut jar, mut storage) = (sex_fetch::Jar::default(), WebStorage::default());
        if let Jarbox::Container(n) = kind {
            if let Some(t) = read(window, alloc::format!("{}/containers/{n}/cookies.tsv", DIR_STR).as_bytes()) { jar = sex_fetch::Jar::load(&t, now); }
            if let Some(t) = read(window, alloc::format!("{}/containers/{n}/storage.tsv", DIR_STR).as_bytes()) { storage = WebStorage::load(&t); }
        }
        sex_pdx::serial_record!("[web.jarbox.opened] box={} cookies={} origins={}", kind.tag(), jar.len(), storage.origins().count());
        let s = Store { kind, saved_jar: jar.changes(), saved_storage: storage.changes, jar: Rc::new(RefCell::new(jar)), storage: Rc::new(RefCell::new(storage)) };
        let out = (s.jar.clone(), s.storage.clone());
        self.boxes.push(s);
        out
    }

    /// The last private tab closed: its cookies and storage go.
    pub fn forget_private(&mut self) {
        let Some(i) = self.boxes.iter().position(|s| s.kind == Jarbox::Private) else { return };
        let s = self.boxes.remove(i);
        sex_pdx::serial_record!("[web.private.forgotten] cookies={} origins={}", s.jar.borrow().len(), s.storage.borrow().origins().count());
    }

    /// Forgets what the site at `url` kept in one box: its cookies and its storage.
    pub fn clear_site(&mut self, jar: &mut sex_fetch::Jar, storage: &mut WebStorage, url: &str) {
        let Ok(u) = sex_fetch::Url::parse(url) else { return };
        jar.clear_site(&u.host);
        storage.clear(&u.origin());
        self.later();
    }

    /// Notes that storage or cookies may have changed.
    pub fn touched(&mut self) {
        let own = self.storage.borrow().changes != self.saved_storage || self.jar.borrow().changes() != self.saved_jar;
        let boxes = self.boxes.iter().any(|s| s.kind != Jarbox::Private && (s.storage.borrow().changes != s.saved_storage || s.jar.borrow().changes() != s.saved_jar));
        if own || boxes { self.later(); }
    }

    /// Saves what changed, when it is due (or now with `force`).
    pub fn save(&mut self, window: &mut Window, now: u64, force: bool) {
        let jar_rc = self.jar.clone();
        let jar_ref = jar_rc.borrow();
        let jar = &*jar_ref;
        if !force && !self.due.is_some_and(|d| sex_pdx::get_ticks() >= d) { return; }
        self.due = None;
        let _ = files::mkdir(window, DIR);
        let mut wrote = 0;
        if self.history_dirty {
            let mut t = String::new();
            for v in &self.history { t.push_str(&alloc::format!("{}\t{}\t{}\n", v.visits, v.url, v.title)); }
            if files::write(window, HISTORY, t.as_bytes()).is_ok() { self.history_dirty = false; wrote += 1; }
        }
        if self.bookmarks_dirty {
            let mut t = String::new();
            for (u, title) in &self.bookmarks { t.push_str(&alloc::format!("{}\t{}\n", u, title)); }
            if files::write(window, BOOKMARKS, t.as_bytes()).is_ok() { self.bookmarks_dirty = false; wrote += 1; }
        }
        if jar.changes() != self.saved_jar && files::write(window, COOKIES, jar.save(now).as_bytes()).is_ok() { self.saved_jar = jar.changes(); wrote += 1; }
        let changes = self.storage.borrow().changes;
        if changes != self.saved_storage {
            let text = self.storage.borrow().save();
            if files::write(window, STORAGE, text.as_bytes()).is_ok() { self.saved_storage = changes; wrote += 1; }
        }
        // Containers keep theirs in their own folders; the private box never reaches Files.
        for s in self.boxes.iter_mut() {
            let Jarbox::Container(n) = s.kind else { continue };
            let dir = alloc::format!("{}/containers/{n}", DIR_STR);
            let (jc, sc) = (s.jar.borrow().changes(), s.storage.borrow().changes);
            if jc == s.saved_jar && sc == s.saved_storage { continue; }
            let _ = files::mkdir(window, alloc::format!("{}/containers", DIR_STR).as_bytes());
            let _ = files::mkdir(window, dir.as_bytes());
            if jc != s.saved_jar && files::write(window, alloc::format!("{dir}/cookies.tsv").as_bytes(), s.jar.borrow().save(now).as_bytes()).is_ok() { s.saved_jar = jc; wrote += 1; }
            if sc != s.saved_storage && files::write(window, alloc::format!("{dir}/storage.tsv").as_bytes(), s.storage.borrow().save().as_bytes()).is_ok() { s.saved_storage = sc; wrote += 1; }
            sex_pdx::serial_record!("[web.jarbox.saved] box={} cookies={} origins={}", n, s.jar.borrow().len(), s.storage.borrow().origins().count());
        }
        if wrote > 0 { let _ = files::sync(window); }
        sex_pdx::serial_record!("[web.profile.saved] files={} history={} bookmarks={} cookies={}", wrote, self.history.len(), self.bookmarks.len(), jar.len());
    }
}
