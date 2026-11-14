// SPDX-License-Identifier: GPL-3.0-only
//! sexshop (formerly Store): the package catalog in a window. `sexshop` in Spindle starts it as
//! `run +net +pkg sexshop`: the network to download, and packages to install and remove.
//! Files still checks every package's signature and asks nothing of the person here: a
//! package that asks for the network is asked about on its program's first run, in Spindle.
//!
//! The window lists the store's catalog, marking what is installed and what is newer.
//! Up and Down choose, Enter installs or upgrades, Delete removes, R downloads the catalog
//! again from the saved store, U upgrades everything, `/` types words to narrow the list.
//!
//! Given a command it does that one thing and ends, for scripts and gates:
//! `store refresh`, `store search WORDS`, `store get NAME`, `store upgrade`,
//! `store remove NAME`.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::{files, nav, net, typed, Event, Nav, Window};
use sex_pdx::fs as w;
use sex_pdx::package as p;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[store.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const ENTER: u8 = 0x1C;
const ESC: u8 = 0x01;
const BACKSPACE: u8 = 0x0E;
const SHOWN: usize = 14;
const STATUS_LINES: usize = 6;
/// The most a catalog and its signature may be (Files refuses bigger ones anyway).
const CATALOG_BYTES: usize = 256 * 1024;
const SIG_BYTES: usize = 4096;
/// A package whose size the store did not say.
const PACKAGE_BYTES: usize = 16 * 1024 * 1024;

fn shown(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("?") }
fn word(status: u8) -> &'static str {
    match status { w::OK => "ok", w::DENIED => "denied", w::NOT_FOUND => "not_found", w::INVALID => "invalid", _ => "failed" }
}

/// What Files said: its status, the lines for the person, and the URLs for the program.
struct Said { status: u8, lines: Vec<Vec<u8>>, urls: Vec<Vec<u8>> }

fn ask(window: &mut Window, op: u64, arg: &[u8]) -> Said {
    let (status, report) = files::package(window, op, arg, 0);
    let mut said = Said { status, lines: Vec::new(), urls: Vec::new() };
    for line in report.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        match line.strip_prefix(&[p::MACHINE]) {
            Some(url) => said.urls.push(url.to_vec()),
            None => { sex_pdx::serial_record!("[store.line] {}", shown(line)); said.lines.push(line.to_vec()); }
        }
    }
    if status == w::DENIED && said.lines.is_empty() {
        said.lines.push(b"Not allowed: start sexshop with `sexshop` (run +net +pkg sexshop).".to_vec());
        sex_pdx::serial_record!("[store.line] not allowed op={}", op);
    }
    said
}

/// Downloads `url` into the file `to` in this program's folder.
fn download(window: &mut Window, url: &[u8], to: &[u8], max: usize) -> Result<(), Vec<u8>> {
    let mut buf = vec![0u8; max];
    match net::get(window, url, &mut buf) {
        Ok((200, n)) => files::write(window, to, &buf[..n]).map_err(|e| format!("Could not save the download: {}", shown(w::message(e))).into_bytes()),
        Ok((status, _)) => Err(format!("The store answered {} for {}.", status, shown(url)).into_bytes()),
        Err(e) => Err(net::message(e).to_vec()),
    }
}

/// The result of one job: the last status and every line said on the way.
struct Done { status: u8, lines: Vec<Vec<u8>> }
impl Done {
    fn failed(&mut self, line: Vec<u8>) { sex_pdx::serial_record!("[store.line] {}", shown(&line)); self.lines.push(line); self.status = w::IO; }
    fn take(&mut self, said: Said) -> Said { self.status = said.status; self.lines.extend(said.lines.iter().cloned()); said }
}

/// The catalog again, from the store Files remembers.
fn refresh(window: &mut Window) -> Done {
    let mut done = Done { status: w::OK, lines: Vec::new() };
    let source = done.take(ask(window, p::SOURCE, b""));
    let Some(url) = source.urls.first().filter(|_| source.status == w::OK) else { return done };
    for (leaf, to, max) in [(&b"sexos-store.txt"[..], p::CATALOG_DOWNLOAD, CATALOG_BYTES), (b"sexos-store.sig", p::SIG_DOWNLOAD, SIG_BYTES)] {
        let mut at = url.clone();
        at.extend_from_slice(leaf);
        if let Err(why) = download(window, &at, to, max) { done.failed(why); return done; }
    }
    done.take(ask(window, p::CATALOG, url));
    done
}

/// Downloads and installs one package (`name`), or every newer one (empty).
fn get(window: &mut Window, name: &[u8]) -> Done {
    let mut done = Done { status: w::OK, lines: Vec::new() };
    let found = done.take(ask(window, p::LOCATE, name));
    if found.status != w::OK { return done; }
    // Each URL follows its "Getting NAME VERSION (N KiB)..." line.
    let sizes: Vec<usize> = found.lines.iter().filter(|l| l.starts_with(b"Getting ")).map(|l| kib(l).map_or(PACKAGE_BYTES, |k| k * 1024 + 64 * 1024)).collect();
    for (i, url) in found.urls.iter().enumerate() {
        if let Err(why) = download(window, url, p::DOWNLOAD, sizes.get(i).copied().unwrap_or(PACKAGE_BYTES)) { done.failed(why); return done; }
        let installed = done.take(ask(window, p::INSTALL, p::DOWNLOAD));
        if installed.status != w::OK { return done; }
    }
    done
}
fn kib(line: &[u8]) -> Option<usize> {
    let open = line.iter().rposition(|b| *b == b'(')?;
    let digits = line[open + 1..].iter().take_while(|b| b.is_ascii_digit()).fold(0usize, |v, b| v * 10 + (*b - b'0') as usize);
    (digits > 0).then_some(digits)
}

/// One package in the list.
struct Item { name: Vec<u8>, line: Vec<u8>, installed: bool, newer: bool }
fn catalog(window: &mut Window, words: &[u8]) -> (Vec<Item>, Done) {
    let mut done = Done { status: w::OK, lines: Vec::new() };
    let said = ask(window, p::SEARCH, words);
    done.status = said.status;
    if said.status != w::OK { done.lines = said.lines; return (Vec::new(), done); }
    let items = said.lines.iter().filter(|l| l.as_slice() != b"Nothing matches.").map(|l| Item {
        name: l.split(|b| *b == b' ').next().unwrap_or(b"").to_vec(),
        line: l.clone(),
        installed: l.windows(11).any(|w| w == b"(installed "),
        newer: l.windows(10).any(|w| w == b"newer here"),
    }).map(|mut i| { i.installed |= i.line.ends_with(b"(installed)"); i }).collect();
    (items, done)
}

struct View { items: Vec<Item>, selected: usize, filter: Vec<u8>, typing: bool, status: Vec<Vec<u8>>, renders: u32 }
impl View {
    fn say(&mut self, lines: Vec<Vec<u8>>) {
        self.status.extend(lines);
        let extra = self.status.len().saturating_sub(STATUS_LINES);
        self.status.drain(..extra);
    }
    fn reload(&mut self, window: &mut Window) {
        let (items, done) = catalog(window, &self.filter);
        self.items = items;
        if done.status != w::OK { self.say(done.lines); }
        self.selected = self.selected.min(self.items.len().saturating_sub(1));
    }
    fn render(&mut self, window: &mut Window, working: Option<&str>) {
        let mut t = String::new();
        t.push_str(&format!("sexshop: {} packages", self.items.len()));
        if !self.filter.is_empty() || self.typing { t.push_str(&format!("   search: {}{}", shown(&self.filter), if self.typing { "_" } else { "" })); }
        t.push_str("\n\n");
        let top = self.selected.saturating_sub(SHOWN - 1);
        for (i, item) in self.items.iter().enumerate().skip(top).take(SHOWN) {
            t.push_str(if i == self.selected { "> " } else { "  " });
            t.push_str(shown(&item.line));
            t.push('\n');
        }
        if self.items.is_empty() { t.push_str("  Nothing to show. R downloads the catalog from the store.\n"); }
        t.push('\n');
        match working {
            Some(what) => { t.push_str(what); t.push('\n'); }
            None => for line in &self.status { t.push_str(shown(line)); t.push('\n'); },
        }
        t.push_str("\nEnter install or upgrade  Delete remove  R refresh  U upgrade all  / search\n");
        window.show(t.as_bytes());
        self.renders += 1;
        let installed = self.items.iter().filter(|i| i.installed).count();
        let newer = self.items.iter().filter(|i| i.newer).count();
        sex_pdx::serial_record!("[store.list] items={} installed={} newer={} selected={} render={}", self.items.len(), installed, newer, self.selected, self.renders);
    }
    fn job(&mut self, window: &mut Window, what: &str, run: impl FnOnce(&mut Window) -> Done) {
        self.render(window, Some(what));
        let done = run(window);
        self.say(done.lines);
        self.reload(window);
        self.render(window, None);
    }
}

/// The run's own words: what Spindle added (`+packages`) and given paths are not commands.
fn command_words(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for word in raw.split(|b| *b == b' ').filter(|w| !w.is_empty() && *w != p::GRANT_TOKEN && !w.starts_with(b"@")) {
        if !out.is_empty() { out.push(b' '); }
        out.extend_from_slice(word);
    }
    out
}

fn command(mut window: Window, line: &[u8]) -> ! {
    let window = &mut window;
    let mut it = line.splitn(2, |b| *b == b' ');
    let (op, rest) = (it.next().unwrap_or(b""), it.next().unwrap_or(b""));
    let done = match op {
        b"refresh" => refresh(window),
        b"search" => { let s = ask(window, p::SEARCH, rest); Done { status: s.status, lines: s.lines } }
        b"get" | b"install" if !rest.is_empty() => get(window, rest),
        b"upgrade" => get(window, b""),
        b"remove" if !rest.is_empty() => { let s = ask(window, p::REMOVE, rest); Done { status: s.status, lines: s.lines } }
        _ => {
            sex_pdx::serial_record!("[store.line] Usage: store [refresh | search WORDS | get NAME | upgrade | remove NAME]");
            Done { status: w::INVALID, lines: Vec::new() }
        }
    };
    let mut t = String::new();
    for l in &done.lines { t.push_str(shown(l)); t.push('\n'); }
    window.show(t.as_bytes());
    sex_pdx::serial_record!("[store.done] op={} status={} lines={}", shown(op), word(done.status), done.lines.len());
    sex_pdx::task_finish(0)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"sexshop");
    let mut raw = [0u8; w::ARGS_MAX];
    let args = files::args(&mut window, &mut raw);
    let packages = args.split(|b| *b == b' ').any(|t| t == p::GRANT_TOKEN);
    let words = command_words(args);
    sex_pdx::serial_record!("[store.ready] target={} task={} packages={} command={}", window.target, window.task, packages as u8, shown(&words));
    if !words.is_empty() { command(window, &words); }
    let mut view = View { items: Vec::new(), selected: 0, filter: Vec::new(), typing: false, status: Vec::new(), renders: 0 };
    if !packages { view.say(vec![b"This run may not install packages. Start sexshop with: sexshop".to_vec()]); }
    view.reload(&mut window);
    view.render(&mut window, None);
    loop {
        let key = match window.next() {
            Event::Key(key) => key,
            Event::Close => window.close(),
            _ => continue,
        };
        if view.typing {
            match key.code {
                ENTER | ESC => view.typing = false,
                BACKSPACE => { view.filter.pop(); }
                _ => if let Some(c) = typed(key).filter(|c| *c >= b' ') { if view.filter.len() < 64 { view.filter.push(c); } },
            }
            view.selected = 0;
            view.reload(&mut window);
            view.render(&mut window, None);
            continue;
        }
        let chosen = view.items.get(view.selected).map(|i| (i.name.clone(), i.installed, i.newer));
        match (nav(key), key.code, typed(key)) {
            (Some(Nav::Up), _, _) => view.selected = view.selected.saturating_sub(1),
            (Some(Nav::Down), _, _) => view.selected = (view.selected + 1).min(view.items.len().saturating_sub(1)),
            (Some(Nav::Home), _, _) => view.selected = 0,
            (Some(Nav::End), _, _) => view.selected = view.items.len().saturating_sub(1),
            (Some(Nav::Delete), _, _) => if let Some((name, true, _)) = chosen {
                view.job(&mut window, &format!("Removing {}...", shown(&name)), |win| { let s = ask(win, p::REMOVE, &name); Done { status: s.status, lines: s.lines } });
                continue;
            },
            (_, ENTER, _) => if let Some((name, installed, newer)) = chosen {
                if installed && !newer { view.say(vec![format!("{} is installed and up to date.", shown(&name)).into_bytes()]); }
                else { view.job(&mut window, &format!("Getting {}...", shown(&name)), |win| get(win, &name)); continue; }
            },
            (_, _, Some(b'r' | b'R')) => { view.job(&mut window, "Downloading the catalog...", refresh); continue; }
            (_, _, Some(b'u' | b'U')) => { view.job(&mut window, "Upgrading...", |win| get(win, b"")); continue; }
            (_, _, Some(b'/')) => { view.typing = true; }
            _ => continue,
        }
        view.render(&mut window, None);
    }
}
