// SPDX-License-Identifier: GPL-3.0-only
//! What the browser keeps between runs, in Files under `Web/`: history, bookmarks,
//! cookies that outlast the session, and each origin's `localStorage`. Changes are
//! saved a moment after they happen (pages may write storage every frame), and at once
//! when the window closes.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use sex_app::{files, Window};
use sex_js::storage::WebStorage;
use sex_web::sex_html;

const DIR: &[u8] = b"Kaleidoscope";
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

/// A visited page: its address, title and how often it was opened.
pub struct Visit { pub url: String, pub title: String, pub visits: u32 }

#[derive(Default)]
pub struct Profile {
    /// Least recently visited first.
    pub history: Vec<Visit>,
    /// (address, title), oldest first.
    pub bookmarks: Vec<(String, String)>,
    pub storage: Rc<RefCell<WebStorage>>,
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
    /// Reads the profile back, and the cookies into `jar`.
    pub fn load(window: &mut Window, jar: &mut sex_fetch::Jar, now: u64) -> Profile {
        let mut p = Profile::default();
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

    /// Forgets what the site at `url` kept: its cookies and its storage.
    pub fn clear_site(&mut self, jar: &mut sex_fetch::Jar, url: &str) {
        let Ok(u) = sex_fetch::Url::parse(url) else { return };
        jar.clear_site(&u.host);
        self.storage.borrow_mut().clear(&u.origin());
        self.later();
    }

    /// Notes that storage or cookies may have changed.
    pub fn touched(&mut self, jar: &sex_fetch::Jar) {
        if self.storage.borrow().changes != self.saved_storage || jar.changes() != self.saved_jar { self.later(); }
    }

    /// Saves what changed, when it is due (or now with `force`).
    pub fn save(&mut self, window: &mut Window, jar: &sex_fetch::Jar, now: u64, force: bool) {
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
        if wrote > 0 { let _ = files::sync(window); }
        sex_pdx::serial_record!("[web.profile.saved] files={} history={} bookmarks={} cookies={}", wrote, self.history.len(), self.bookmarks.len(), jar.len());
    }
}
