// SPDX-License-Identifier: GPL-3.0-only
//! sexshop (formerly Store): the package catalogs in a window. `sexshop` in Spindle starts it as
//! `run +net +pkg sexshop`: the network to download, and packages to install and remove.
//! Files still checks every package's signature and asks nothing of the person here: a
//! package that asks for the network is asked about on its program's first run, in Spindle.
//!
//! The window lists every catalog Files keeps (the main store and each third-party
//! repository), with a row of categories, the featured packages, and a count of updates.
//! Each package has a page: its icon, screenshots, description, size and the grants it
//! asks for. Pictures come from the catalog's folder and are checked against the SHA-256
//! the signed catalog gives before they are shown.
//!
//! Keys: Up and Down choose, Enter opens a package's page (Enter there installs or
//! upgrades, Esc goes back), Tab or Left/Right change category, Delete removes, R
//! downloads every catalog again, U upgrades everything, P lists repositories (Delete
//! removes one there), `/` types words to narrow the list. The mouse clicks categories,
//! featured tiles, rows and the page's buttons; the wheel scrolls.
//!
//! Adding a repository and rolling a package back are the person's decisions, taken in
//! Spindle (`pkg repo add NAME URL KEY`, `pkg rollback NAME`); the Store says so.
//!
//! Given a command it does that one thing and ends, for scripts and gates:
//! `store refresh`, `store search WORDS`, `store get [REPO/]NAME`, `store upgrade`,
//! `store remove NAME`, `store page [REPO/]NAME`, `store repos`, `store repo remove NAME`.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sex_app::{files, nav, net, typed, Event, Nav, Window};
use sex_pdx::app_pointer::Kind as PointerKind;
use sex_pdx::fs as w;
use sex_pdx::package as p;
use sex_pkg::index::{Asset, Listing};
use sha2::{Digest, Sha256};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[store.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const ENTER: u8 = 0x1C;
const ESC: u8 = 0x01;
const BACKSPACE: u8 = 0x0E;
const TAB: u8 = 0x0F;
const STATUS_LINES: usize = 3;
/// The most a catalog and its signature may be (Files refuses bigger ones anyway).
const CATALOG_BYTES: usize = 256 * 1024;
const SIG_BYTES: usize = 4096;
/// A package whose size the store did not say.
const PACKAGE_BYTES: usize = 16 * 1024 * 1024;
/// The largest icon or screenshot file the store downloads.
const PICTURE_BYTES: usize = 4 * 1024 * 1024;

fn shown(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("?") }
fn word(status: u8) -> &'static str {
    match status { w::OK => "ok", w::DENIED => "denied", w::NOT_FOUND => "not_found", w::INVALID => "invalid", _ => "failed" }
}

/// What Files said: its status, the lines for the person, and the lines for the program.
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
    fn new() -> Self { Done { status: w::OK, lines: Vec::new() } }
    fn failed(&mut self, line: Vec<u8>) { sex_pdx::serial_record!("[store.line] {}", shown(&line)); self.lines.push(line); self.status = w::IO; }
    fn take(&mut self, said: Said) -> Said { self.status = said.status; self.lines.extend(said.lines.iter().cloned()); said }
}

/// Every catalog again: the main store's and each repository's, from what Files keeps.
fn refresh(window: &mut Window) -> Done {
    let mut done = Done::new();
    let source = done.take(ask(window, p::SOURCE, b""));
    if source.status != w::OK { return done; }
    let mut worst = w::OK;
    for line in &source.urls {
        // `URL` is the main store, `URL NAME` a repository: Files checks each with its own key.
        let url = line.split(|b| *b == b' ').next().unwrap_or(line);
        let mut ok = true;
        for (leaf, to, max) in [(&b"sexos-store.txt"[..], p::CATALOG_DOWNLOAD, CATALOG_BYTES), (b"sexos-store.sig", p::SIG_DOWNLOAD, SIG_BYTES)] {
            let mut at = url.to_vec();
            at.extend_from_slice(leaf);
            if let Err(why) = download(window, &at, to, max) { done.failed(why); ok = false; break; }
        }
        if ok { done.take(ask(window, p::CATALOG, line)); }
        if done.status != w::OK { worst = done.status; }
    }
    done.status = worst;
    let (all, _) = catalog(window, b"");
    sex_pdx::serial_record!("[store.updates] count={}", all.iter().filter(|l| l.newer).count());
    done
}

/// Downloads and installs one package (`[REPO/]NAME`), or every newer one (empty).
fn get(window: &mut Window, name: &[u8]) -> Done {
    let mut done = Done::new();
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
fn just(said: Said) -> Done { Done { status: said.status, lines: said.lines } }

/// Every catalog entry matching `words`, as Files describes them.
fn catalog(window: &mut Window, words: &[u8]) -> (Vec<Listing>, Done) {
    let said = ask(window, p::DETAILS, words);
    if said.status != w::OK { return (Vec::new(), just(said)); }
    let items = said.urls.iter().filter_map(|l| Listing::parse(shown(l))).collect();
    (items, Done::new())
}

/// A checked, decoded picture, its pixels blended over the background.
struct Pic { w: usize, h: usize, px: Vec<u32> }
/// Downloads a catalog picture, checks its hash, decodes it and fits it in `max_w` x `max_h`.
fn picture(window: &mut Window, base: &str, asset: &Asset, max_w: usize, max_h: usize) -> Result<Pic, &'static str> {
    let url = format!("{}{}", base, asset.file);
    let mut buf = Vec::new();
    buf.try_reserve_exact(PICTURE_BYTES + 16 * 1024).map_err(|_| "no memory")?;
    buf.resize(PICTURE_BYTES + 16 * 1024, 0);
    let n = match net::get(window, url.as_bytes(), &mut buf) { Ok((200, n)) => n, Ok(_) => return Err("not served"), Err(_) => return Err("no network") };
    buf.truncate(n);
    if Sha256::digest(&buf).as_slice() != asset.sha256 { return Err("hash differs from the signed catalog"); }
    let limits = sex_image::Limits { max_width: 4096, max_height: 4096, max_pixels: 16_000_000, max_input_bytes: PICTURE_BYTES };
    let image = sex_image::decode(&buf, &limits).map_err(|_| "not a picture")?;
    drop(buf);
    let small = image.fit_within(max_w, max_h).map_err(|_| "too large")?;
    let px = small.pixels.iter().map(|p| blend(BG, p & 0xFF_FFFF, p >> 24)).collect();
    Ok(Pic { w: small.width, h: small.height, px })
}

// ---- Drawing ----------------------------------------------------------------------

const BG: u32 = 0x1B1D22;
const PANEL: u32 = 0x262930;
const PICKED: u32 = 0x3D6FD6;
const TEXT: u32 = 0xE6E8EC;
const DIM: u32 = 0x9AA0AA;
const ACCENT: u32 = 0xE0569B;
const GOOD: u32 = 0x4CAF7A;
const GLYPH_W: usize = 22;
const GLYPH_H: usize = 20;
/// 13 px, in 1/64 pixel.
const SIZE: u32 = 13 * 64;
const ROW: usize = 22;
const ICON: usize = 64;
const TILE_ICON: usize = 40;
const SHOT_W: usize = 240;
const SHOT_H: usize = 150;

fn blend(under: u32, over: u32, a: u32) -> u32 {
    let mix = |s: u32| ((((under >> s) & 0xFF) * (255 - a) + ((over >> s) & 0xFF) * a) / 255) << s;
    mix(16) | mix(8) | mix(0)
}

struct Canvas { px: &'static mut [u32], w: usize, h: usize, font: Option<sex_font::Font<'static>>, acc: Vec<i32>, cover: Vec<u8> }
impl Canvas {
    fn new(window: &Window) -> Self {
        let px = window.pixels();
        Canvas { px, w: 0, h: 0, font: sex_font::Font::new(sex_font::GEIST_SANS), acc: vec![0; sex_font::raster::scratch_len(GLYPH_W, GLYPH_H)], cover: vec![0; GLYPH_W * GLYPH_H] }
    }
    fn size(&mut self, window: &Window) {
        let (w, h) = window.size();
        let w = w.clamp(320, 4096);
        let h = h.max(240).min(self.px.len() / w);
        self.w = w; self.h = h;
    }
    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        for row in y..(y + h).min(self.h) { let s = row * self.w; self.px[s + x.min(self.w)..s + (x + w).min(self.w)].fill(color); }
    }
    fn image(&mut self, x: usize, y: usize, pic: &Pic) {
        for row in 0..pic.h {
            if y + row >= self.h || x >= self.w { break; }
            let n = pic.w.min(self.w - x);
            let s = (y + row) * self.w + x;
            self.px[s..s + n].copy_from_slice(&pic.px[row * pic.w..row * pic.w + n]);
        }
    }
    fn measure(&self, s: &str) -> usize {
        let Some(font) = &self.font else { return s.len() * 7 };
        s.chars().filter_map(|c| font.glyph(c as u32)).map(|g| font.advance(g, SIZE) as usize).sum::<usize>() / 256
    }
    /// One line of text at (`x`, `y`) top left, cut at `max` pixels wide.
    fn text(&mut self, x: usize, y: usize, max: usize, s: &str, color: u32) {
        let Some(font) = &self.font else { return };
        let (acc, cover) = (&mut self.acc, &mut self.cover);
        let mut pen = 0i32;
        for c in s.chars() {
            let Some(g) = font.glyph(c as u32).or_else(|| font.glyph('?' as u32)) else { continue };
            let adv = font.advance(g, SIZE);
            if (pen + adv) / 256 > max as i32 { break; }
            let (whole, frac) = (pen / 256, pen % 256);
            if c != ' ' && font.draw(g, SIZE, 2 * 256 + frac, 15 * 256, GLYPH_W, GLYPH_H, acc, cover) {
                for gy in 0..GLYPH_H {
                    let py = y + gy;
                    if py >= self.h { break; }
                    for gx in 0..GLYPH_W {
                        let a = cover[gy * GLYPH_W + gx] as u32;
                        if a == 0 { continue; }
                        let Some(pxx) = (x as i32 + whole + gx as i32 - 2).try_into().ok().filter(|v: &usize| *v < self.w) else { continue };
                        let d = &mut self.px[py * self.w + pxx];
                        *d = blend(*d, color, a);
                    }
                }
            }
            pen += adv;
        }
    }
    /// Words wrapped to `max` pixels; returns the y after the last line.
    fn wrapped(&mut self, x: usize, mut y: usize, max: usize, s: &str, color: u32, bottom: usize) -> usize {
        for para in s.split('\n') {
            let mut line = String::new();
            for word in para.split(' ') {
                let trial = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if self.measure(&trial) > max && !line.is_empty() {
                    if y + ROW > bottom { return y; }
                    self.text(x, y, max, &line, color);
                    y += 18;
                    line = word.to_string();
                } else { line = trial; }
            }
            if y + ROW > bottom { return y; }
            self.text(x, y, max, &line, color);
            y += 18;
        }
        y
    }
}

// ---- The window --------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { List, Page(usize), Repos }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit { Tab(usize), Open(usize), Install, Remove, Back, RepoRow(usize) }

struct View {
    all: Vec<Listing>,
    /// Indexes into `all` that the current category shows.
    shown: Vec<usize>,
    categories: Vec<String>,
    category: usize,
    selected: usize,
    top: usize,
    mode: Mode,
    filter: Vec<u8>,
    typing: bool,
    status: Vec<Vec<u8>>,
    renders: u32,
    hits: Vec<(usize, usize, usize, usize, Hit)>,
    icons: Vec<(String, Option<Pic>)>,
    shots: Vec<Pic>,
    repos: Vec<Vec<u8>>,
    repo_selected: usize,
    canvas: Canvas,
}
impl View {
    fn say(&mut self, lines: Vec<Vec<u8>>) {
        self.status.extend(lines);
        let extra = self.status.len().saturating_sub(STATUS_LINES);
        self.status.drain(..extra);
    }
    fn updates(&self) -> usize { self.all.iter().filter(|l| l.newer).count() }
    fn reload(&mut self, window: &mut Window) {
        let (all, done) = catalog(window, &self.filter);
        self.all = all;
        if done.status != w::OK { self.say(done.lines); }
        let mut cats: Vec<String> = Vec::new();
        for l in &self.all { let c = l.item.category().to_string(); if !cats.contains(&c) { cats.push(c); } }
        let current = self.categories.get(self.category.wrapping_sub(1)).cloned();
        self.categories = cats;
        self.category = current.and_then(|c| self.categories.iter().position(|x| *x == c)).map_or(0, |i| i + 1);
        self.pick_category();
    }
    fn pick_category(&mut self) {
        let cat = self.categories.get(self.category.wrapping_sub(1)).cloned();
        self.shown = (0..self.all.len()).filter(|i| cat.as_deref().is_none_or(|c| self.all[*i].item.category() == c)).collect();
        self.selected = self.selected.min(self.shown.len().saturating_sub(1));
        if let Mode::Page(i) = self.mode { if i >= self.all.len() { self.mode = Mode::List; } }
    }
    fn icon(&mut self, window: &mut Window, i: usize, side: usize) -> Option<usize> {
        let l = &self.all[i];
        let key = format!("{}@{}", l.full_name(), side);
        if let Some(k) = self.icons.iter().position(|(n, _)| *n == key) { return self.icons[k].1.as_ref().map(|_| k); }
        let pic = l.item.icon.clone().and_then(|a| picture(window, &l.base.clone(), &a, side, side).ok());
        self.icons.push((key, pic));
        let k = self.icons.len() - 1;
        self.icons[k].1.as_ref().map(|_| k)
    }

    fn render(&mut self, window: &mut Window, working: Option<&str>) {
        self.canvas.size(window);
        let (w, h) = (self.canvas.w, self.canvas.h);
        self.hits.clear();
        self.canvas.fill(0, 0, w, h, BG);
        match self.mode {
            Mode::List => self.render_list(window),
            Mode::Page(i) => self.render_page(window, i),
            Mode::Repos => self.render_repos(),
        }
        // The bottom: what happened last, and the keys.
        let foot = h.saturating_sub(18 * (STATUS_LINES + 1) + 8);
        self.canvas.fill(0, foot, w, h - foot, PANEL);
        let lines: Vec<String> = match working { Some(s) => vec![s.to_string()], None => self.status.iter().map(|l| shown(l).to_string()).collect() };
        for (k, l) in lines.iter().enumerate() { self.canvas.text(10, foot + 4 + 18 * k, w - 20, l, TEXT); }
        let keys = match self.mode {
            Mode::List => "Enter page  Tab category  Delete remove  R refresh  U upgrade all  P repositories  / search",
            Mode::Page(_) => "Enter install or upgrade  Delete remove  Esc back",
            Mode::Repos => "Delete removes the chosen repository  Esc back",
        };
        self.canvas.text(10, h - 22, w - 20, keys, DIM);
        window.show_pixels(w, h);
        self.renders += 1;
        if self.mode == Mode::List {
            let installed = self.shown.iter().filter(|i| self.all[**i].installed.is_some()).count();
            let newer = self.shown.iter().filter(|i| self.all[**i].newer).count();
            sex_pdx::serial_record!("[store.list] items={} installed={} newer={} selected={} render={}", self.shown.len(), installed, newer, self.selected, self.renders);
        }
    }
    fn render_list(&mut self, window: &mut Window) {
        let w = self.canvas.w;
        let mut head = format!("sexshop: {} packages", self.all.len());
        let updates = self.updates();
        if !self.filter.is_empty() || self.typing { head.push_str(&format!("   search: {}{}", shown(&self.filter), if self.typing { "_" } else { "" })); }
        self.canvas.text(10, 8, w - 140, &head, TEXT);
        if updates > 0 {
            let badge = format!("{updates} update{}", if updates == 1 { "" } else { "s" });
            let bw = self.canvas.measure(&badge) + 16;
            self.canvas.fill(w - bw - 10, 6, bw, 22, ACCENT);
            self.canvas.text(w - bw - 2, 8, bw, &badge, TEXT);
        }
        // Categories.
        let mut x = 10;
        let names: Vec<String> = core::iter::once("All".to_string()).chain(self.categories.iter().cloned()).collect();
        for (k, name) in names.iter().enumerate() {
            let tw = self.canvas.measure(name) + 20;
            if x + tw > w { break; }
            self.canvas.fill(x, 36, tw, 22, if k == self.category { PICKED } else { PANEL });
            self.canvas.text(x + 10, 37, tw, name, TEXT);
            self.hits.push((x, 36, tw, 22, Hit::Tab(k)));
            x += tw + 6;
        }
        let mut y = 66;
        // Featured: the first four featured packages the category shows.
        let featured: Vec<usize> = self.shown.iter().copied().filter(|i| self.all[*i].item.featured).take(4).collect();
        if !featured.is_empty() {
            self.canvas.text(10, y, 200, "Featured", DIM);
            y += 20;
            let tile_w = ((w - 20) / 4).min(220);
            for (k, i) in featured.iter().enumerate() {
                let x = 10 + k * (tile_w + 6);
                self.canvas.fill(x, y, tile_w, TILE_ICON + 12, PANEL);
                if let Some(ic) = self.icon(window, *i, TILE_ICON) { let pic = self.icons[ic].1.take().unwrap(); self.canvas.image(x + 6, y + 6, &pic); self.icons[ic].1 = Some(pic); }
                let (name, summary) = (self.all[*i].full_name(), self.all[*i].item.summary.clone());
                self.canvas.text(x + TILE_ICON + 12, y + 6, tile_w - TILE_ICON - 16, &name, TEXT);
                self.canvas.text(x + TILE_ICON + 12, y + 26, tile_w - TILE_ICON - 16, &summary, DIM);
                self.hits.push((x, y, tile_w, TILE_ICON + 12, Hit::Open(*i)));
            }
            y += TILE_ICON + 20;
        }
        // The list.
        let bottom = self.canvas.h.saturating_sub(18 * (STATUS_LINES + 1) + 12);
        let rows = ((bottom.saturating_sub(y)) / ROW).max(1);
        if self.selected < self.top { self.top = self.selected; }
        if self.selected >= self.top + rows { self.top = self.selected + 1 - rows; }
        if self.shown.is_empty() { self.canvas.text(10, y, w - 20, "Nothing to show. R downloads the catalogs from the store and its repositories.", DIM); }
        for (k, i) in self.shown.iter().copied().enumerate().skip(self.top).take(rows) {
            let l = &self.all[i];
            let mut line = format!("{} {}  {}", l.full_name(), l.item.version, l.item.summary);
            if l.item.grants != 0 { line.push_str(&format!("  asks for: {}", sex_pkg::index::grant_words(l.item.grants))); }
            let mark = match (l.installed, l.newer) { (Some(v), true) => format!("installed {v}, update"), (Some(_), false) => "installed".into(), _ => String::new() };
            let ry = y + (k - self.top) * ROW;
            if k == self.selected { self.canvas.fill(6, ry, w - 12, ROW, PICKED); }
            let mw = self.canvas.measure(&mark);
            self.canvas.text(12, ry + 1, w - 30 - mw, &line, TEXT);
            let newer = l.newer;
            self.canvas.text(w - 14 - mw, ry + 1, mw + 4, &mark, if newer { ACCENT } else { GOOD });
            self.hits.push((6, ry, w - 12, ROW, Hit::Open(i)));
        }
    }
    fn render_page(&mut self, window: &mut Window, i: usize) {
        let w = self.canvas.w;
        let bottom = self.canvas.h.saturating_sub(18 * (STATUS_LINES + 1) + 12);
        self.canvas.fill(10, 6, 70, 22, PANEL);
        self.canvas.text(18, 7, 60, "< Back", TEXT);
        self.hits.push((10, 6, 70, 22, Hit::Back));
        if let Some(ic) = self.icon(window, i, ICON) { let pic = self.icons[ic].1.take().unwrap(); self.canvas.image(16, 40, &pic); self.icons[ic].1 = Some(pic); }
        else { self.canvas.fill(16, 40, ICON, ICON, PANEL); }
        let l = self.all[i].clone();
        let x = 16 + ICON + 16;
        self.canvas.text(x, 40, w - x - 10, &format!("{} {}", l.full_name(), l.item.version), TEXT);
        self.canvas.text(x, 58, w - x - 10, &l.item.summary, DIM);
        let from = if l.repo == sex_pkg::repo::MAIN { "the main store".to_string() } else { format!("the repository {}", l.repo) };
        self.canvas.text(x, 76, w - x - 10, &format!("{}  |  {} KiB  |  from {}", l.item.category(), l.item.bytes.div_ceil(1024), from), DIM);
        let grants = if l.item.grants == 0 { "Asks for: nothing".to_string() } else { format!("Asks for: {} (you are asked on its first run)", sex_pkg::index::grant_words(l.item.grants)) };
        self.canvas.text(x, 94, w - x - 10, &grants, if l.item.grants == 0 { DIM } else { ACCENT });
        // Buttons.
        let label = match (l.installed, l.newer) { (None, _) => "Install", (Some(_), true) => "Upgrade", (Some(_), false) => "Installed" };
        let mut bx = 16;
        let by = 118;
        let bw = self.canvas.measure(label) + 24;
        self.canvas.fill(bx, by, bw, 24, if label == "Installed" { PANEL } else { PICKED });
        self.canvas.text(bx + 12, by + 2, bw, label, TEXT);
        if label != "Installed" { self.hits.push((bx, by, bw, 24, Hit::Install)); }
        bx += bw + 8;
        if l.installed.is_some() {
            let rw = self.canvas.measure("Remove") + 24;
            self.canvas.fill(bx, by, rw, 24, PANEL);
            self.canvas.text(bx + 12, by + 2, rw, "Remove", TEXT);
            self.hits.push((bx, by, rw, 24, Hit::Remove));
            bx += rw + 8;
        }
        if let Some(prev) = l.previous {
            self.canvas.text(bx, by + 2, w.saturating_sub(bx + 10), &format!("{prev} is kept. Roll back in Spindle: pkg rollback {}", l.item.name), DIM);
        }
        let mut y = by + 36;
        if !l.item.description.is_empty() { y = self.canvas.wrapped(16, y, w - 32, &l.item.description, TEXT, bottom.saturating_sub(SHOT_H + 10)); y += 8; }
        // Screenshots, side by side.
        let mut sx = 16;
        for pic in core::mem::take(&mut self.shots).into_iter() {
            if sx + pic.w <= w && y + pic.h <= bottom { self.canvas.image(sx, y, &pic); }
            sx += pic.w + 10;
            self.shots.push(pic);
        }
        if l.item.screenshots.len() > self.shots.len() && y + 18 < bottom {
            let names: Vec<&str> = l.item.screenshots.iter().skip(self.shots.len()).map(|a| a.file.as_str()).collect();
            self.canvas.text(16, y.max(1) + if self.shots.is_empty() { 0 } else { SHOT_H + 6 }, w - 32, &format!("Screenshots not shown: {}", names.join(", ")), DIM);
        }
    }
    fn render_repos(&mut self) {
        let w = self.canvas.w;
        self.canvas.text(10, 8, w - 20, "Repositories", TEXT);
        self.canvas.text(10, 28, w - 20, "Add one in Spindle (it is your trust decision): pkg repo add NAME URL KEY", DIM);
        for (k, line) in self.repos.clone().iter().enumerate() {
            let y = 56 + k * ROW;
            if k == self.repo_selected { self.canvas.fill(6, y, w - 12, ROW, PICKED); }
            self.canvas.text(12, y + 1, w - 24, shown(line), TEXT);
            self.hits.push((6, y, w - 12, ROW, Hit::RepoRow(k)));
        }
    }
    fn job(&mut self, window: &mut Window, what: &str, run: impl FnOnce(&mut Window) -> Done) {
        self.render(window, Some(what));
        let done = run(window);
        self.say(done.lines);
        self.reload(window);
        self.render(window, None);
    }
    fn open_page(&mut self, window: &mut Window, i: usize) {
        self.mode = Mode::Page(i);
        self.render(window, Some("Getting the pictures..."));
        let l = self.all[i].clone();
        self.shots.clear();
        let mut bad = 0;
        for a in &l.item.screenshots {
            match picture(window, &l.base, a, SHOT_W, SHOT_H) { Ok(pic) => self.shots.push(pic), Err(why) => { bad += 1; self.say(vec![format!("{}: {}", a.file, why).into_bytes()]); } }
        }
        sex_pdx::serial_record!("[store.page] name={} repo={} shots={}/{} bad={}", l.item.name, l.repo, self.shots.len(), l.item.screenshots.len(), bad);
        self.render(window, None);
    }
    fn load_repos(&mut self, window: &mut Window) {
        let said = ask(window, p::REPOS, b"");
        self.repos = said.lines;
        self.repo_selected = self.repo_selected.min(self.repos.len().saturating_sub(1));
    }
    fn install(&mut self, window: &mut Window, i: usize) {
        let name = self.all[i].full_name();
        self.job(window, &format!("Getting {name}..."), |win| get(win, name.as_bytes()));
        self.repage(&name);
    }
    fn remove(&mut self, window: &mut Window, i: usize) {
        let name = self.all[i].item.name.clone();
        self.job(window, &format!("Removing {name}..."), |win| just(ask(win, p::REMOVE, name.as_bytes())));
        let full = self.all.get(i).map(|l| l.full_name()).unwrap_or_default();
        self.repage(&full);
    }
    /// After a reload, the page shows the same package again.
    fn repage(&mut self, full: &str) {
        if let Mode::Page(_) = self.mode { self.mode = self.all.iter().position(|l| l.full_name() == full).map_or(Mode::List, Mode::Page); }
    }
    fn act(&mut self, window: &mut Window, hit: Hit) {
        match (hit, self.mode) {
            (Hit::Tab(k), _) => { self.category = k; self.selected = 0; self.top = 0; self.pick_category(); }
            (Hit::Open(i), _) => { if let Some(k) = self.shown.iter().position(|x| *x == i) { self.selected = k; } self.open_page(window, i); return; }
            (Hit::Install, Mode::Page(i)) => { self.install(window, i); return; }
            (Hit::Remove, Mode::Page(i)) => { self.remove(window, i); return; }
            (Hit::Back, _) => self.mode = Mode::List,
            (Hit::RepoRow(k), _) => self.repo_selected = k,
            _ => {}
        }
        self.render(window, None);
    }
    fn remove_repo(&mut self, window: &mut Window) {
        let Some(line) = self.repos.get(self.repo_selected) else { return };
        let name = line.split(|b| *b == b' ').next().unwrap_or(b"").to_vec();
        if name == b"main" || name.is_empty() { self.say(vec![b"The main store is set in Spindle: pkg refresh URL".to_vec()]); self.render(window, None); return; }
        self.job(window, &format!("Removing the repository {}...", shown(&name)), |win| just(ask(win, p::REPO_REMOVE, &name)));
        self.load_repos(window);
        self.mode = Mode::Repos;
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

/// `store page NAME`: the page, its pictures checked and drawn, for scripts and gates.
fn page(window: &mut Window, name: &[u8]) -> Done {
    let (all, done) = catalog(window, b"");
    if done.status != w::OK { return done; }
    let name = shown(name);
    let Some(i) = all.iter().position(|l| l.full_name() == name).or_else(|| all.iter().position(|l| l.item.name == name)) else {
        let mut d = Done::new();
        d.status = w::NOT_FOUND;
        d.lines.push(format!("No package {name} in the catalogs.").into_bytes());
        return d;
    };
    let l = all[i].clone();
    let icon = match &l.item.icon { None => "none", Some(a) => match picture(window, &l.base, a, ICON, ICON) { Ok(_) => "ok", Err(why) => why } };
    let mut ok = 0;
    for a in &l.item.screenshots { if picture(window, &l.base, a, SHOT_W, SHOT_H).is_ok() { ok += 1; } }
    sex_pdx::serial_record!("[store.page] name={} repo={} version={} bytes={} grants={} category={} featured={} icon={} shots={}/{} description={} installed={} previous={}",
        l.item.name, l.repo, l.item.version, l.item.bytes, sex_pkg::index::grant_words(l.item.grants), l.item.category(), l.item.featured as u8, icon, ok, l.item.screenshots.len(), l.item.description.len(),
        l.installed.map(|v| v.to_string()).unwrap_or_else(|| "-".into()), l.previous.map(|v| v.to_string()).unwrap_or_else(|| "-".into()));
    let mut d = Done::new();
    d.lines.push(format!("{} {} from {}: {} KiB, asks for {}", l.full_name(), l.item.version, l.repo, l.item.bytes.div_ceil(1024), sex_pkg::index::grant_words(l.item.grants)).into_bytes());
    if !l.item.description.is_empty() { d.lines.push(l.item.description.replace('\n', " ").into_bytes()); }
    if icon != "ok" && icon != "none" { d.status = w::INVALID; }
    if ok != l.item.screenshots.len() { d.status = w::INVALID; }
    d
}

fn command(mut window: Window, line: &[u8]) -> ! {
    let window = &mut window;
    let mut it = line.splitn(2, |b| *b == b' ');
    let (op, rest) = (it.next().unwrap_or(b""), it.next().unwrap_or(b""));
    let done = match op {
        b"refresh" => refresh(window),
        b"search" => just(ask(window, p::SEARCH, rest)),
        b"get" | b"install" if !rest.is_empty() => get(window, rest),
        b"upgrade" => get(window, b""),
        b"remove" if !rest.is_empty() => just(ask(window, p::REMOVE, rest)),
        b"page" if !rest.is_empty() => page(window, rest),
        b"repos" => just(ask(window, p::REPOS, b"")),
        b"repo" if rest.starts_with(b"remove ") => just(ask(window, p::REPO_REMOVE, rest[7..].trim_ascii())),
        b"repo" | b"rollback" => {
            let line = if op == b"repo" { "Adding a repository is your trust decision: in Spindle, pkg repo add NAME URL KEY" } else { "Going back a version is done in Spindle: pkg rollback NAME" };
            sex_pdx::serial_record!("[store.line] {}", line);
            Done { status: w::DENIED, lines: vec![line.as_bytes().to_vec()] }
        }
        _ => {
            sex_pdx::serial_record!("[store.line] Usage: store [refresh | search WORDS | get [REPO/]NAME | upgrade | remove NAME | page NAME | repos | repo remove NAME]");
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
    let canvas = Canvas::new(&window);
    let mut view = View { all: Vec::new(), shown: Vec::new(), categories: Vec::new(), category: 0, selected: 0, top: 0, mode: Mode::List, filter: Vec::new(), typing: false, status: Vec::new(), renders: 0, hits: Vec::new(), icons: Vec::new(), shots: Vec::new(), repos: Vec::new(), repo_selected: 0, canvas };
    if !packages { view.say(vec![b"This run may not install packages. Start sexshop with: sexshop".to_vec()]); }
    view.reload(&mut window);
    sex_pdx::serial_record!("[store.updates] count={}", view.updates());
    view.render(&mut window, None);
    loop {
        let key = match window.next() {
            Event::Key(key) => key,
            Event::Close => window.close(),
            Event::Resize { .. } => { view.render(&mut window, None); continue; }
            Event::Pointer(pt) => {
                match pt.kind {
                    PointerKind::Down { button: 0 } => {
                        let (x, y) = (pt.x.max(0) as usize, pt.y.max(0) as usize);
                        if let Some(hit) = view.hits.iter().find(|(hx, hy, hw, hh, _)| x >= *hx && x < hx + hw && y >= *hy && y < hy + hh).map(|h| h.4) { view.act(&mut window, hit); }
                    }
                    PointerKind::Wheel { steps } if view.mode == Mode::List => {
                        view.selected = if steps > 0 { view.selected.saturating_sub(steps as usize) } else { (view.selected + (-steps) as usize).min(view.shown.len().saturating_sub(1)) };
                        view.render(&mut window, None);
                    }
                    _ => {}
                }
                continue;
            }
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
        match view.mode {
            Mode::Page(i) => match (nav(key), key.code) {
                (_, ESC | BACKSPACE) => view.mode = Mode::List,
                (Some(Nav::Delete), _) if view.all[i].installed.is_some() => { view.remove(&mut window, i); continue; }
                (_, ENTER) => {
                    if view.all[i].installed.is_some() && !view.all[i].newer { view.say(vec![format!("{} is installed and up to date.", view.all[i].full_name()).into_bytes()]); }
                    else { view.install(&mut window, i); continue; }
                }
                _ => continue,
            },
            Mode::Repos => match (nav(key), key.code) {
                (_, ESC | BACKSPACE) => view.mode = Mode::List,
                (Some(Nav::Up), _) => view.repo_selected = view.repo_selected.saturating_sub(1),
                (Some(Nav::Down), _) => view.repo_selected = (view.repo_selected + 1).min(view.repos.len().saturating_sub(1)),
                (Some(Nav::Delete), _) => { view.remove_repo(&mut window); continue; }
                _ => continue,
            },
            Mode::List => {
                let chosen = view.shown.get(view.selected).copied();
                match (nav(key), key.code, typed(key)) {
                    (Some(Nav::Up), _, _) => view.selected = view.selected.saturating_sub(1),
                    (Some(Nav::Down), _, _) => view.selected = (view.selected + 1).min(view.shown.len().saturating_sub(1)),
                    (Some(Nav::Home), _, _) => view.selected = 0,
                    (Some(Nav::End), _, _) => view.selected = view.shown.len().saturating_sub(1),
                    (Some(Nav::Left), _, _) => { view.category = view.category.checked_sub(1).unwrap_or(view.categories.len()); view.selected = 0; view.pick_category(); }
                    (Some(Nav::Right), _, _) | (_, TAB, _) => { view.category = (view.category + 1) % (view.categories.len() + 1); view.selected = 0; view.pick_category(); }
                    (Some(Nav::Delete), _, _) => if let Some(i) = chosen.filter(|i| view.all[*i].installed.is_some()) { view.remove(&mut window, i); continue; },
                    (_, ENTER, _) => if let Some(i) = chosen { view.open_page(&mut window, i); continue; },
                    (_, _, Some(b'r' | b'R')) => { view.job(&mut window, "Downloading the catalogs...", refresh); continue; }
                    (_, _, Some(b'u' | b'U')) => { view.job(&mut window, "Upgrading...", |win| get(win, b"")); continue; }
                    (_, _, Some(b'p' | b'P')) => { view.load_repos(&mut window); view.mode = Mode::Repos; }
                    (_, _, Some(b'/')) => { view.typing = true; }
                    _ => continue,
                }
            }
        }
        view.render(&mut window, None);
    }
}
