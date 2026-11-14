// SPDX-License-Identifier: GPL-3.0-only
//! Gossip: feeds and podcasts. Subscriptions live in `feeds.txt` in the program's own
//! folder and the items read in `read.txt`; each feed's last reply is kept beside them
//! so the lists show without the network. `run +net gossip` fetches every feed on
//! start; `run +net gossip URL` subscribes to a feed, `run +net gossip @list.opml`
//! imports an OPML list. Parsing, OPML and the read state are sex-feed's, tested on the
//! host; this file is the window around them.
//!
//! Keys: Up/Down choose, Enter opens (a feed's items, then an item, which marks it
//! read), Esc goes back. In the feeds: `r` fetches all again, `a` subscribes to a typed
//! address, Delete unsubscribes, `x` writes `subscriptions.opml`. In items: `m` marks
//! read or unread, `d` downloads a podcast episode into the folder. The mouse picks and
//! opens rows; the wheel scrolls an item.
//!
//! Podcast episodes are downloaded into the folder. There is no way yet to ask
//! another program (Qupid) to open a file, so the episode is not played from here.
// TODO: hand the episode to Qupid once programs can ask Shell to open a file with another program.
#![no_std]
#![no_main]

extern crate alloc;

mod draw;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use draw::{clip, Canvas, Face};
use sex_app::{files, net, Event, Nav, Window};
use sex_feed::store::{ReadState, Subscriptions};
use sex_feed::{opml, Item};
use sex_pdx::app_pointer::Kind;

const FEEDS: &[u8] = b"feeds.txt";
const READ: &[u8] = b"read.txt";
const EXPORT: &[u8] = b"subscriptions.opml";
/// The largest feed and the largest episode Gossip downloads.
const FEED_MAX: usize = 4 << 20;
const EPISODE_MAX: usize = 64 << 20;

const BACK: u32 = 0x0016_1820;
const PANEL: u32 = 0x0020_232C;
const ACCENT: u32 = 0x00E0_7A5F;
const TEXT: u32 = 0x00E8_EAED;
const DIM: u32 = 0x0080_868E;
const SELECT: u32 = 0x002E_3442;
const UNREAD: u32 = 0x0072_B7F2;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[gossip.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn shown(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("?") }

#[derive(Clone, Copy, PartialEq)]
enum Mode { Feeds, Items, Item, Adding }

struct Loaded { items: Vec<Item>, error: Option<String> }

struct App {
    subs: Subscriptions,
    read: ReadState,
    loaded: Vec<Loaded>,
    mode: Mode,
    feed: usize,
    item: usize,
    scroll: usize,
    input: String,
    status: String,
}

fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let mut word = word;
            while word.chars().count() > cols {
                if !line.is_empty() { out.push(core::mem::take(&mut line)); }
                let cut = word.char_indices().nth(cols).map_or(word.len(), |(i, _)| i);
                out.push(String::from(&word[..cut]));
                word = &word[cut..];
            }
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > cols { out.push(core::mem::take(&mut line)); }
            if !line.is_empty() { line.push(' '); }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

impl App {
    fn url(&self, feed: usize) -> &str { self.subs.feeds.get(feed).map_or("", |f| f.url.as_str()) }
    fn unread(&self, feed: usize) -> usize {
        let url = self.url(feed);
        self.loaded.get(feed).map_or(0, |l| l.items.iter().filter(|i| !self.read.is_read(url, &i.id)).count())
    }
    fn save_feeds(&mut self, window: &mut Window) {
        if let Err(e) = files::write(window, FEEDS, self.subs.serialize().as_bytes()) {
            sex_pdx::serial_record!("[gossip.save.failed] file=feeds status={}", e);
            self.status = format!("Not saved: {}", shown(files::message(e)));
        }
    }
    fn save_read(&mut self, window: &mut Window) {
        match files::write(window, READ, self.read.serialize().as_bytes()) {
            Ok(()) => sex_pdx::serial_record!("[gossip.saved.read] count={}", self.read.len()),
            Err(e) => sex_pdx::serial_record!("[gossip.save.failed] file=read status={}", e),
        }
    }
    /// The kept copy of each feed, so the lists show before (or without) the network.
    fn load_caches(&mut self, window: &mut Window, buf: &mut [u8]) {
        self.loaded = (0..self.subs.feeds.len()).map(|_| Loaded { items: Vec::new(), error: None }).collect();
        for i in 0..self.subs.feeds.len() {
            let name = sex_feed::cache_name(self.url(i));
            if let Ok(n) = files::read(window, name.as_bytes(), buf) {
                if let Ok(f) = sex_feed::parse_bytes(&buf[..n]) { self.loaded[i].items = f.items; }
            }
        }
    }
    fn fetch(&mut self, window: &mut Window, i: usize, buf: &mut [u8]) {
        let url = String::from(self.url(i));
        let error = match net::get(window, url.as_bytes(), buf) {
            Ok((200, n)) if n < buf.len() => match sex_feed::parse_bytes(&buf[..n]) {
                Ok(feed) => {
                    let _ = files::write(window, sex_feed::cache_name(&url).as_bytes(), &buf[..n]);
                    if self.subs.retitle(&url, &feed.title) { self.save_feeds(window); }
                    self.loaded[i].items = feed.items;
                    sex_pdx::serial_record!("[gossip.fetched] feed={} status=200 items={} unread={}", url, self.loaded[i].items.len(), self.unread(i));
                    None
                }
                Err(_) => Some(String::from("not an RSS or Atom feed")),
            },
            Ok((200, _)) => Some(String::from("the feed is larger than 4 MiB")),
            Ok((status, _)) => Some(format!("the server answered {status}")),
            Err(e) => Some(String::from(shown(net::message(e)))),
        };
        if let Some(why) = &error { sex_pdx::serial_record!("[gossip.fetch.failed] feed={} why={}", url, why); }
        self.loaded[i].error = error;
    }
    fn fetch_all(&mut self, window: &mut Window) {
        let mut buf = alloc::vec![0u8; FEED_MAX];
        for i in 0..self.subs.feeds.len() { self.fetch(window, i, &mut buf); }
        let failed = self.loaded.iter().filter(|l| l.error.is_some()).count();
        self.status = if failed == 0 { format!("{} feeds fetched.", self.subs.feeds.len()) } else { format!("{failed} of {} feeds could not be fetched.", self.subs.feeds.len()) };
    }
    fn subscribe(&mut self, window: &mut Window, url: &str) {
        if !self.subs.add(url, "") { self.status = String::from("Already subscribed."); return; }
        sex_pdx::serial_record!("[gossip.subscribed] feed={}", url);
        self.save_feeds(window);
        self.loaded.push(Loaded { items: Vec::new(), error: None });
        let mut buf = alloc::vec![0u8; FEED_MAX];
        let i = self.subs.feeds.len() - 1;
        self.fetch(window, i, &mut buf);
    }
    fn import(&mut self, window: &mut Window, path: &[u8]) {
        let mut buf = alloc::vec![0u8; 1 << 20];
        let n = match files::read(window, path, &mut buf) {
            Ok(n) => n,
            Err(e) => { self.status = format!("Could not read {}: {}", shown(path), shown(files::message(e))); sex_pdx::serial_record!("[gossip.import.failed] path={} status={}", shown(path), e); return; }
        };
        let subs = opml::import(&String::from_utf8_lossy(&buf[..n]));
        let added = self.subs.merge(&subs);
        sex_pdx::serial_record!("[gossip.imported] feeds={} added={}", subs.len(), added);
        self.status = format!("Imported {} feeds ({added} new).", subs.len());
        if added > 0 { self.save_feeds(window); }
    }
    fn export(&mut self, window: &mut Window) {
        let text = opml::export(&self.subs.feeds);
        match files::write(window, EXPORT, text.as_bytes()) {
            Ok(()) => { sex_pdx::serial_record!("[gossip.exported] feeds={} bytes={}", self.subs.feeds.len(), text.len()); self.status = format!("Wrote {} feeds to subscriptions.opml.", self.subs.feeds.len()); }
            Err(e) => self.status = format!("Not written: {}", shown(files::message(e))),
        }
    }
    fn open_feed(&mut self, i: usize) {
        if i >= self.subs.feeds.len() { return; }
        self.feed = i;
        self.item = 0;
        self.mode = Mode::Items;
        let url = self.url(i);
        let items = &self.loaded[i].items;
        sex_pdx::serial_record!("[gossip.feed] feed={} title={} items={} unread={}", url, self.subs.feeds[i].title, items.len(), self.unread(i));
        for (k, it) in items.iter().enumerate() {
            sex_pdx::serial_record!("[gossip.item] index={} read={} enclosure={} title={}", k, self.read.is_read(url, &it.id) as u8, it.enclosure.is_some() as u8, it.title);
        }
    }
    fn set_read(&mut self, window: &mut Window, read: bool) {
        let url = String::from(self.url(self.feed));
        let Some(it) = self.loaded.get(self.feed).and_then(|l| l.items.get(self.item)) else { return };
        let (id, title) = (it.id.clone(), it.title.clone());
        if self.read.set(&url, &id, read) {
            sex_pdx::serial_record!("[gossip.{}] index={} id={} title={}", if read { "read" } else { "unread" }, self.item, id, title);
            self.save_read(window);
        }
    }
    fn open_item(&mut self, window: &mut Window) {
        if self.loaded.get(self.feed).is_none_or(|l| self.item >= l.items.len()) { return; }
        self.mode = Mode::Item;
        self.scroll = 0;
        self.set_read(window, true);
    }
    fn download(&mut self, window: &mut Window) {
        let Some(enc) = self.loaded.get(self.feed).and_then(|l| l.items.get(self.item)).and_then(|i| i.enclosure.clone()) else {
            self.status = String::from("This item has no episode to download.");
            return;
        };
        let name = sex_feed::enclosure_name(&enc.url);
        let size = if enc.length > 0 { (enc.length as usize).saturating_add(64 << 10).min(EPISODE_MAX) } else { EPISODE_MAX };
        let mut buf = Vec::new();
        if buf.try_reserve_exact(size).is_err() { self.status = String::from("Not enough memory for that episode."); return; }
        buf.resize(size, 0);
        self.status = match net::get(window, enc.url.as_bytes(), &mut buf) {
            Ok((200, n)) if n < buf.len() || enc.length as usize == n => match files::write(window, name.as_bytes(), &buf[..n]) {
                Ok(()) => {
                    sex_pdx::serial_record!("[gossip.enclosure.saved] file={} bytes={} type={}", name, n, enc.mime);
                    // TODO: ask Shell to open it in Qupid once programs can open files with another program.
                    sex_pdx::serial_record!("[gossip.enclosure.kept] file={} player=none", name);
                    format!("Saved {name} ({n} bytes) in Gossip's folder.")
                }
                Err(e) => format!("Could not save {name}: {}", shown(files::message(e))),
            },
            Ok((200, _)) => String::from("The episode is larger than Gossip downloads."),
            Ok((status, _)) => format!("The server answered {status}."),
            Err(e) => String::from(shown(net::message(e))),
        };
        if !self.status.starts_with("Saved") { sex_pdx::serial_record!("[gossip.enclosure.failed] url={} why={}", enc.url, self.status); }
    }
    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let typed = sex_app::typed(key);
        let nav = sex_app::nav(key);
        match self.mode {
            Mode::Adding => match key.code {
                0x1C => {
                    self.mode = Mode::Feeds;
                    let url = String::from(self.input.trim());
                    self.input.clear();
                    if !url.is_empty() { self.subscribe(window, &url); }
                }
                0x01 => { self.mode = Mode::Feeds; self.input.clear(); }
                0x0E => { self.input.pop(); }
                _ => if let Some(c) = typed { if self.input.len() < 400 { self.input.push(c as char); } },
            },
            Mode::Feeds => match (key.code, nav, typed) {
                (0x1C, _, _) => self.open_feed(self.feed),
                (_, Some(Nav::Up), _) => self.feed = self.feed.saturating_sub(1),
                (_, Some(Nav::Down), _) => self.feed = (self.feed + 1).min(self.subs.feeds.len().saturating_sub(1)),
                (_, Some(Nav::Delete), _) => {
                    let url = String::from(self.url(self.feed));
                    if url.is_empty() { return; }
                    self.subs.remove(&url);
                    self.read.forget(&url);
                    self.loaded.remove(self.feed);
                    self.feed = self.feed.min(self.subs.feeds.len().saturating_sub(1));
                    sex_pdx::serial_record!("[gossip.unsubscribed] feed={}", url);
                    self.save_feeds(window);
                    self.save_read(window);
                }
                (_, _, Some(b'r')) => self.fetch_all(window),
                (_, _, Some(b'a')) => { self.mode = Mode::Adding; self.input.clear(); }
                (_, _, Some(b'x')) => self.export(window),
                _ => {}
            },
            Mode::Items => {
                let n = self.loaded.get(self.feed).map_or(0, |l| l.items.len());
                match (key.code, nav, typed) {
                    (0x1C, _, _) => self.open_item(window),
                    (0x01, _, _) => self.mode = Mode::Feeds,
                    (_, Some(Nav::Up), _) => self.item = self.item.saturating_sub(1),
                    (_, Some(Nav::Down), _) => self.item = (self.item + 1).min(n.saturating_sub(1)),
                    (_, _, Some(b'm')) => {
                        let url = String::from(self.url(self.feed));
                        let read = self.loaded[self.feed].items.get(self.item).is_some_and(|i| self.read.is_read(&url, &i.id));
                        self.set_read(window, !read);
                    }
                    (_, _, Some(b'd')) => self.download(window),
                    (_, _, Some(b'r')) => { let mut buf = alloc::vec![0u8; FEED_MAX]; self.fetch(window, self.feed, &mut buf); }
                    _ => {}
                }
            }
            Mode::Item => match (key.code, nav, typed) {
                (0x01 | 0x1C, _, _) => self.mode = Mode::Items,
                (_, Some(Nav::Up), _) | (0x49 | 0x68, _, _) => self.scroll = self.scroll.saturating_sub(if nav.is_some() { 1 } else { 10 }),
                (_, Some(Nav::Down), _) | (0x51 | 0x6D, _, _) => self.scroll += if nav.is_some() { 1 } else { 10 },
                (_, _, Some(b'm')) => self.set_read(window, false),
                (_, _, Some(b'd')) => self.download(window),
                _ => {}
            },
        }
    }
}

const ROW: i32 = 22;
const LIST_Y: i32 = 44;

fn render(window: &mut Window, app: &App, small: &mut Face, big: &mut Face) {
    let (w, h) = window.size();
    let (w, h) = (w.max(240), h.max(160));
    let (wi, hi) = (w as i32, h as i32);
    let mut c = Canvas { px: window.pixels(), w, h };
    c.fill(0, 0, wi, hi, BACK);
    c.fill(0, 0, wi, 36, PANEL);
    let cols = ((wi - 32) / small.w as i32).max(4) as usize;
    let rows = ((hi - LIST_Y - 34) / ROW).max(1) as usize;
    match app.mode {
        Mode::Feeds | Mode::Adding => {
            c.text(big, 12, 4, "Gossip", ACCENT, 200);
            c.text(small, 140, 10, &format!("{} feeds", app.subs.feeds.len()), DIM, wi - 150);
            if app.subs.feeds.is_empty() { c.text(small, 16, LIST_Y, "No feeds yet: press a to subscribe to one.", DIM, wi - 32); }
            let first = app.feed.saturating_sub(rows.saturating_sub(1));
            for (k, f) in app.subs.feeds.iter().enumerate().skip(first).take(rows) {
                let y = LIST_Y + (k - first) as i32 * ROW;
                if k == app.feed { c.fill(8, y - 2, wi - 16, ROW, SELECT); }
                let unread = app.unread(k);
                let name = if f.title.is_empty() { f.url.as_str() } else { f.title.as_str() };
                let tail = match app.loaded.get(k).and_then(|l| l.error.as_ref()) { Some(e) => format!("({e})"), None => format!("{unread} unread") };
                c.text(small, 16, y, clip(name, cols.saturating_sub(tail.len() + 2)), if unread > 0 { TEXT } else { DIM }, wi - 32);
                c.text(small, wi - 16 - (tail.chars().count() * small.w) as i32, y, &tail, if unread > 0 { UNREAD } else { DIM }, wi);
            }
        }
        Mode::Items => {
            let f = &app.subs.feeds[app.feed];
            c.text(big, 12, 4, clip(if f.title.is_empty() { &f.url } else { &f.title }, ((wi - 24) / big.w as i32).max(1) as usize), ACCENT, wi - 24);
            let url = f.url.as_str();
            let items = &app.loaded[app.feed].items;
            if items.is_empty() { c.text(small, 16, LIST_Y, "No items.", DIM, wi - 32); }
            let first = app.item.saturating_sub(rows.saturating_sub(1));
            for (k, it) in items.iter().enumerate().skip(first).take(rows) {
                let y = LIST_Y + (k - first) as i32 * ROW;
                if k == app.item { c.fill(8, y - 2, wi - 16, ROW, SELECT); }
                let read = app.read.is_read(url, &it.id);
                c.text(small, 16, y, if read { " " } else { "*" }, UNREAD, 20);
                let mark = if it.enclosure.is_some() { " [episode]" } else { "" };
                let line = format!("{}{}", it.title, mark);
                c.text(small, 16 + 2 * small.w as i32, y, clip(&line, cols.saturating_sub(2)), if read { DIM } else { TEXT }, wi - 40);
            }
        }
        Mode::Item => {
            let it = &app.loaded[app.feed].items[app.item];
            c.text(big, 12, 4, clip(&it.title, ((wi - 24) / big.w as i32).max(1) as usize), ACCENT, wi - 24);
            let mut lines: Vec<(String, u32)> = Vec::new();
            if !it.date.is_empty() { lines.push((it.date.clone(), DIM)); }
            if !it.link.is_empty() { lines.push((it.link.clone(), DIM)); }
            if let Some(e) = &it.enclosure {
                lines.push((format!("Episode: {} ({}, {} bytes). Press d to download it.", e.url, if e.mime.is_empty() { "?" } else { &e.mime }, e.length), UNREAD));
            }
            lines.push((String::new(), TEXT));
            for l in wrap(&it.text(), cols) { lines.push((l, TEXT)); }
            for (k, (l, colour)) in lines.iter().skip(app.scroll).take(((hi - LIST_Y - 34) / small.h as i32).max(1) as usize).enumerate() {
                c.text(small, 16, LIST_Y + k as i32 * small.h as i32, clip(l, cols), *colour, wi - 32);
            }
        }
    }
    let bottom = match app.mode {
        Mode::Adding => format!("Feed address: {}_", app.input),
        _ if !app.status.is_empty() => app.status.clone(),
        Mode::Feeds => String::from("Enter opens, r fetches, a adds, Delete removes, x exports OPML"),
        Mode::Items => String::from("Enter reads, m marks read/unread, d downloads the episode, Esc back"),
        Mode::Item => String::from("Up/Down scroll, m marks unread, d downloads the episode, Esc back"),
    };
    c.text(small, 12, hi - 26, &bottom, if app.mode == Mode::Adding { TEXT } else { DIM }, wi - 24);
    window.show_pixels(w, h);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Gossip");
    let mut small = Face::new(9, 18);
    let mut big = Face::new(14, 28);
    let mut buf = alloc::vec![0u8; FEED_MAX];
    let subs = match files::read(&mut window, FEEDS, &mut buf) { Ok(n) => Subscriptions::parse(&String::from_utf8_lossy(&buf[..n])), Err(_) => Subscriptions::default() };
    let read = match files::read(&mut window, READ, &mut buf) { Ok(n) => ReadState::parse(&String::from_utf8_lossy(&buf[..n])), Err(_) => ReadState::default() };
    sex_pdx::serial_record!("[gossip.loaded] feeds={} read={}", subs.feeds.len(), read.len());
    let mut app = App { subs, read, loaded: Vec::new(), mode: Mode::Feeds, feed: 0, item: 0, scroll: 0, input: String::new(), status: String::new() };
    // `@list.opml` imports a list; an address subscribes to it.
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args).to_vec();
    let mut new_urls = Vec::new();
    for word in given.split(|b| *b == b' ').filter(|w| !w.is_empty()) {
        if word.first() == Some(&b'@') { app.import(&mut window, word); }
        else if word.starts_with(b"http://") || word.starts_with(b"https://") { new_urls.push(String::from(shown(word))); }
    }
    for url in &new_urls { if app.subs.add(url, "") { sex_pdx::serial_record!("[gossip.subscribed] feed={}", url); } }
    if !new_urls.is_empty() { app.save_feeds(&mut window); }
    app.load_caches(&mut window, &mut buf);
    drop(buf);
    render(&mut window, &app, &mut small, &mut big);
    app.fetch_all(&mut window);
    render(&mut window, &app, &mut small, &mut big);
    sex_pdx::serial_record!("[gossip.ready] target={} task={} feeds={}", window.target, window.task, app.subs.feeds.len());
    loop {
        match window.next() {
            Event::Close => { sex_pdx::serial_record!("[gossip.closed] target={} task={}", window.target, window.task); window.close() }
            Event::Key(key) => { app.status.clear(); app.key(&mut window, key); }
            Event::Pointer(p) => match p.kind {
                Kind::Down { button: 0 } => {
                    let row = ((p.y as i32 - LIST_Y + 2) / ROW).max(0) as usize;
                    let rows = ((window.size().1 as i32 - LIST_Y - 34) / ROW).max(1) as usize;
                    match app.mode {
                        Mode::Feeds => {
                            let k = app.feed.saturating_sub(rows.saturating_sub(1)) + row;
                            if k < app.subs.feeds.len() { if k == app.feed { app.open_feed(k); } else { app.feed = k; } }
                        }
                        Mode::Items => {
                            let k = app.item.saturating_sub(rows.saturating_sub(1)) + row;
                            if k < app.loaded[app.feed].items.len() { if k == app.item { app.open_item(&mut window); } else { app.item = k; } }
                        }
                        _ => continue,
                    }
                }
                Kind::Wheel { steps } if app.mode == Mode::Item => app.scroll = (app.scroll as isize - steps as isize * 3).max(0) as usize,
                _ => continue,
            },
            Event::Resize { .. } => {}
        }
        render(&mut window, &app, &mut small, &mut big);
    }
}
