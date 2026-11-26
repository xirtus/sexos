// SPDX-License-Identifier: GPL-3.0-only
//! Thumbnails in Files: Ctrl+G shows the folder as a grid of pictures instead of a list.
//!
//! Pictures (PNG, JPEG, GIF, WebP) are made small in the background, one per piece of
//! work between keys, and kept in Files' own folder (`thumbs/`), named by path, size and
//! modification time; the folder opened again draws from there without decoding anything.
//! Other files and folders show their icon from the SexOS icon set (`sex_icons`).
//!
//! Documents are drawn by their own program, one at a time, run with no window:
//! `NAME --thumb @/in @/out 112` writes the thumbnail straight into the same cache, in
//! the same format (u16 LE width, u16 LE height, RGB rows). A helper that is missing,
//! fails or takes too long leaves the kind tile.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::files::{self, Entry};
use sex_app::Window;
use sex_font::Font;

use crate::{at, join};

use sex_theme::candy_ui::{
    draw_candy_button, draw_candy_panel, draw_candy_selection_row, draw_candy_tab,
    draw_candy_toolbar, ButtonState, ButtonStyle, CandyPalette,
};

/// Largest side of a thumbnail, in pixels.
pub const SIDE: usize = 112;
const CELL_W: usize = 136;
const CELL_H: usize = SIDE + 30;
const TOOLBAR_H: usize = 30;
const TAB_BAR_H: usize = 22;
const HEAD: usize = TOOLBAR_H + TAB_BAR_H + 4;
const FOOT: usize = 24;
const SIDEBAR_W: usize = 128;
const DIR: &[u8] = b"thumbs";
/// Pictures larger than this are not read for a thumbnail.
const FILE_MAX: u64 = 48 * 1024 * 1024;

const PAL: CandyPalette = CandyPalette::dark();
const BG: u32 = PAL.bg;
const TILE: u32 = PAL.card;
const TEXT: u32 = PAL.text;
const DIM: u32 = PAL.text_dim;
/// Side of a file or folder icon in a tile.
const ICON: usize = 80;

const PLACES: [(&str, &[u8]); 5] = [
    ("Home", b"/"),
    ("Documents", b"/documents"),
    ("Downloads", b"/downloads"),
    ("Pictures", b"/pictures"),
    ("Devices / Media", b"/media"),
];

pub enum Thumb { Picture { w: usize, h: usize, px: Vec<u32> }, Kind(&'static str) }

/// What one folder's thumbnails took.
#[derive(Default, Clone, Copy)]
pub struct Tally { pub pictures: usize, pub made: usize, pub cached: usize, pub failed: usize, pub started: u64,
                   /// Files a helper program draws: how many, drawn now, from the cache.
                   pub documents: usize, pub drawn: usize, pub from_cache: usize,
                   /// When the last picture was done: `ms` counts pictures only.
                   pub pictures_done: u64 }

/// Kinds a program draws a thumbnail of with `--thumb`, and that program.
const HELPERS: &[(&str, &[u8])] = &[("PDF", b"vellum"), ("RAW", b"gaze")];
/// A helper run that has not ended by now is ended.
const HELPER_MS: u64 = 20_000;
static HELPER_RUNS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0x7B00_0000);

/// The helper drawing one document: (cookie, document path, cache name, started at).
struct Helping { cookie: u32, program: &'static [u8], path: Vec<u8>, name: Vec<u8>, started: u64 }

pub struct Thumbs {
    pub on: bool,
    made: BTreeMap<Vec<u8>, Thumb>,
    queue: VecDeque<(Vec<u8>, u64)>,
    /// Documents waiting for their helper, after the pictures.
    documents: VecDeque<(Vec<u8>, u64, &'static [u8])>,
    helping: Option<Helping>,
    /// Helpers found missing this run, so they are not asked again.
    missing: Vec<&'static [u8]>,
    /// Where Files' own folder is as an absolute path, if helpers may write there.
    own: Option<Vec<u8>>,
    pub tally: Tally,
    /// The folder whose thumbnails are being made, logged when the queue empties.
    folder: Vec<u8>,
    dir_ready: bool,
    font: Option<Font<'static>>,
    acc: Vec<i32>,
    cover: Vec<u8>,
    /// File icons drawn so far, by icon name: `ICON` square `0xAARRGGBB` pixels.
    icons: BTreeMap<&'static str, Vec<u32>>,
    /// The picked tile: Settings' accent in the dark theme, read when a folder is listed.
    picked: u32,
}

pub fn kind(name: &[u8], dir: bool) -> Option<&'static str> {
    if dir { return Some("FOLDER"); }
    let ext = match name.iter().rposition(|b| *b == b'.') { Some(i) => &name[i + 1..], None => return Some("FILE") };
    let mut e = [0u8; 5];
    if ext.len() > e.len() { return Some("FILE"); }
    for (o, b) in e.iter_mut().zip(ext) { *o = b.to_ascii_lowercase(); }
    Some(match &e[..ext.len()] {
        b"png" | b"jpg" | b"jpeg" | b"gif" | b"webp" => return None,
        b"pdf" => "PDF",
        // Camera RAW: Gaze draws the embedded preview.
        b"dng" | b"cr2" | b"cr3" | b"nef" | b"arw" | b"orf" | b"rw2" | b"pef" | b"raf" => "RAW",
        b"mp4" | b"webm" | b"mkv" | b"mov" | b"avi" | b"m4v" => "VIDEO",
        b"mp3" | b"ogg" | b"opus" | b"flac" | b"wav" | b"m4a" => "AUDIO",
        b"txt" | b"md" | b"rs" | b"toml" | b"json" | b"csv" | b"log" | b"html" | b"css" | b"js" => "TEXT",
        b"zip" | b"tar" | b"gz" | b"xz" | b"zst" => "ARCHIVE",
        _ => "FILE",
    })
}

fn fnv(bytes: &[u8]) -> u64 { bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3)) }

impl Thumbs {
    pub const fn new() -> Self {
        Thumbs { on: false, made: BTreeMap::new(), queue: VecDeque::new(), documents: VecDeque::new(), helping: None, missing: Vec::new(), own: None,
                 tally: Tally { pictures: 0, made: 0, cached: 0, failed: 0, started: 0, documents: 0, drawn: 0, from_cache: 0, pictures_done: 0 }, folder: Vec::new(),
                 dir_ready: false, font: None, acc: Vec::new(), cover: Vec::new(), icons: BTreeMap::new(), picked: 0xDB4B7E }
    }
    pub fn busy(&self) -> bool { self.on && (!self.queue.is_empty() || !self.documents.is_empty() || self.helping.is_some()) }
    /// Queues the pictures of a freshly listed folder that have no thumbnail yet.
    pub fn folder(&mut self, cwd: &[u8], entries: &[Entry]) {
        let accent = sex_app::settings::appearance().map_or(sex_theme::DEFAULT_ACCENT, |(_, a)| a as u32);
        self.picked = sex_theme::tokens(sex_theme::Mode::Dark, accent).accent & 0xFF_FFFF;
        self.queue.clear();
        self.documents.clear();
        // A helper still drawing for the folder left behind is ended.
        if let Some(h) = self.helping.take() { sex_app::launch::kill(h.cookie); }
        self.folder = cwd.to_vec();
        self.tally = Tally { started: sex_pdx::get_ticks(), ..Tally::default() };
        for e in entries {
            let path = join(cwd, &e.name);
            match kind(&e.name, e.dir) {
                None => {
                    self.tally.pictures += 1;
                    if !self.made.contains_key(&path) { self.queue.push_back((path, e.size)); }
                }
                Some(k) => if let Some((_, program)) = HELPERS.iter().find(|(h, _)| *h == k) {
                    if e.size == 0 { continue; }
                    self.tally.documents += 1;
                    if !self.made.contains_key(&path) { self.documents.push_back((path, e.size, program)); }
                },
            }
        }
        if self.on && !self.busy() && self.tally.pictures + self.tally.documents > 0 { self.report(); }
    }
    fn report(&self) {
        let t = &self.tally;
        sex_pdx::serial_record!("[files.app.thumbs] dir={} pictures={} made={} cached={} failed={} ms={} documents={} drawn={} from_cache={}",
            crate::shown(&self.folder), t.pictures, t.made, t.cached, t.failed,
            if t.pictures_done != 0 { t.pictures_done } else { sex_pdx::get_ticks() }.saturating_sub(t.started), t.documents, t.drawn, t.from_cache);
    }
    fn cache_name(window: &mut Window, path: &[u8], size: u64) -> Vec<u8> {
        let mtime = files::stat(window, &at(path)).map(|s| s.mtime).unwrap_or(0);
        alloc::format!("thumbs/{:016x}-{}-{}", fnv(path), size, mtime).into_bytes()
    }
    fn ready(&mut self, window: &mut Window) {
        if self.dir_ready { return; }
        let _ = files::mkdir(window, DIR);
        self.dir_ready = true;
        // Files' own folder, as a helper is handed it: `/data/NAME/thumbs` is the same
        // folder only if Files runs as `loom` and holds the whole filesystem.
        let mut own = sex_pdx::program_store::DATA.to_vec();
        own.extend_from_slice(b"loom");
        let mut probe = own.clone();
        probe.extend_from_slice(b"/thumbs");
        self.own = files::stat(window, &at(&probe)).ok().filter(|s| s.dir).map(|_| own);
    }
    /// Makes one thumbnail. True when the grid should be drawn again.
    pub fn step(&mut self, window: &mut Window) -> bool {
        let Some((path, size)) = self.queue.pop_front() else { return self.document_step(window) };
        let name = Self::cache_name(window, &path, size);
        let thumb = match self.cached(window, &name) {
            Some(t) => { self.tally.cached += 1; t }
            None => match picture(window, &path, size) {
                Some((w, h, px)) => {
                    self.tally.made += 1;
                    self.keep(window, &name, w, h, &px);
                    Thumb::Picture { w, h, px }
                }
                None => { self.tally.failed += 1; Thumb::Kind("PICTURE?") }
            },
        };
        self.made.insert(path, thumb);
        if self.queue.is_empty() { self.tally.pictures_done = sex_pdx::get_ticks(); }
        if !self.busy() { self.report(); }
        true
    }
    /// Starts the next document's helper, or looks at the one running.
    fn document_step(&mut self, window: &mut Window) -> bool {
        use sex_app::launch::{self, Event};
        if let Some(h) = &self.helping {
            let (cookie, program) = (h.cookie, h.program);
            let ended = match launch::poll(cookie) {
                Some(Event::Started { .. }) => return false,
                Some(Event::Refused { code, .. }) => {
                    // Not installed: the kind tile, and no more asking this run.
                    if code == sex_pdx::program_store::NOT_FOUND as u32 && !self.missing.contains(&program) { self.missing.push(program); }
                    Some(alloc::format!("refused code={}", code))
                }
                Some(Event::Done { status: Some(0), .. }) => None,
                Some(Event::Done { status, .. }) => Some(alloc::format!("status={}", status.map_or(-1, |s| s as i64))),
                None if sex_pdx::get_ticks().saturating_sub(h.started) > HELPER_MS => { launch::kill(cookie); Some(String::from("timeout")) }
                None => { let _ = sex_pdx::sys_wait_self_ticks(2); return false; }
            };
            let h = self.helping.take().unwrap();
            let ms = sex_pdx::get_ticks().saturating_sub(h.started);
            let thumb = match ended {
                None => self.cached(window, &h.name),
                Some(why) => {
                    sex_pdx::serial_record!("[files.app.thumbs.helper.failed] program={} path={} {} ms={}", crate::shown(program), crate::shown(&h.path), why, ms);
                    None
                }
            };
            if thumb.is_some() {
                self.tally.drawn += 1;
                sex_pdx::serial_record!("[files.app.thumbs.helper] program={} path={} ms={}", crate::shown(program), crate::shown(&h.path), ms);
            }
            let label = HELPERS.iter().find(|(_, p)| *p == program).map_or("FILE", |(k, _)| *k);
            self.made.insert(h.path, thumb.unwrap_or(Thumb::Kind(label)));
            if !self.busy() { self.report(); }
            return true;
        }
        let Some((path, size, program)) = self.documents.pop_front() else { return false };
        self.ready(window);
        let name = Self::cache_name(window, &path, size);
        if let Some(t) = self.cached(window, &name) {
            self.tally.from_cache += 1;
            self.made.insert(path, t);
            if !self.busy() { self.report(); }
            return true;
        }
        let label = HELPERS.iter().find(|(_, p)| *p == program).map_or("FILE", |(k, _)| *k);
        let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
        let mut line = Vec::new();
        let started = match (&self.own, self.missing.contains(&program)) {
            (Some(own), false) => {
                let mut out = own.clone();
                out.push(b'/');
                out.extend_from_slice(&name);
                let input = sex_pdx::launch::open_args(&path, &mut args).map(|n| args[..n].to_vec());
                let output = sex_pdx::launch::open_args(&out, &mut args).map(|n| args[..n].to_vec());
                match (input, output) {
                    (Some(i), Some(o)) => {
                        line.extend_from_slice(b"--thumb ");
                        line.extend_from_slice(&i);
                        line.push(b' ');
                        line.extend_from_slice(&o);
                        line.extend_from_slice(alloc::format!(" {}", SIDE).as_bytes());
                        let cookie = HELPER_RUNS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        (line.len() <= sex_pdx::fs::ARGS_MAX && launch::run_hidden(program, &line, cookie).is_ok()).then_some(cookie)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        match started {
            Some(cookie) => self.helping = Some(Helping { cookie, program, path, name, started: sex_pdx::get_ticks() }),
            None => { self.made.insert(path, Thumb::Kind(label)); if !self.busy() { self.report(); } }
        }
        true
    }
    fn cached(&self, window: &mut Window, name: &[u8]) -> Option<Thumb> {
        let mut raw = vec![0u8; 4 + SIDE * SIDE * 3];
        let n = files::read(window, name, &mut raw).ok()?;
        let (w, h) = (u16::from_le_bytes([raw[0], raw[1]]) as usize, u16::from_le_bytes([raw[2], raw[3]]) as usize);
        if w == 0 || h == 0 || w > SIDE || h > SIDE || n != 4 + w * h * 3 { return None; }
        let px = raw[4..n].chunks_exact(3).map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32).collect();
        Some(Thumb::Picture { w, h, px })
    }
    fn keep(&mut self, window: &mut Window, name: &[u8], w: usize, h: usize, px: &[u32]) {
        self.ready(window);
        let mut out = Vec::with_capacity(4 + px.len() * 3);
        out.extend_from_slice(&(w as u16).to_le_bytes());
        out.extend_from_slice(&(h as u16).to_le_bytes());
        for p in px { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]); }
        let _ = files::write(window, name, &out);
    }

    /// Width of the left Places sidebar when the window is wide enough.
    fn sidebar_w(width: usize) -> usize { if width >= 560 { SIDEBAR_W } else { 0 } }
    /// How many tiles fit across a window this wide.
    pub fn columns(width: usize) -> usize { (width.saturating_sub(Self::sidebar_w(width) + 12) / CELL_W).max(1) }
    /// How many rows of tiles fit.
    fn rows(height: usize) -> usize { (height.saturating_sub(HEAD + FOOT) / CELL_H).max(1) }

    /// Draws the Dolphin/Finder-style Candy Lavender grid into the window's pixels;
    /// `top` is moved to keep the cursor seen.
    pub fn draw(&mut self, window: &mut Window, head: &str, foot: &str, cwd: &[u8], entries: &[Entry], cursor: usize, top: &mut usize) {
        let (width, height) = window.size();
        let (width, height) = (width.max(CELL_W + 8), height.max(HEAD + FOOT + CELL_H));
        let px = window.pixels();
        let px = &mut px[..width * height];
        px.fill(BG);

        let sb = Self::sidebar_w(width);
        let content_x = sb + 8;

        // 1. Top frosted Candy Lavender toolbar: <- -> ^ nav pills, breadcrumb path bar, Icons/List switcher, + tab.
        draw_candy_toolbar(px, width, height, 0, 0, width as i32, TOOLBAR_H as i32, &PAL);
        let nav_labels = ["<", ">", "^"];
        let mut bx = 10i32;
        for sym in nav_labels {
            draw_candy_button(px, width, height, bx, 6, 26, 24, 8, ButtonStyle::Normal, ButtonState::Rest, &PAL);
            self.text(px, width, (bx + 9) as usize, 10, 14, sym, TEXT);
            bx += 30;
        }
        // Right-side view switcher & new-tab pill.
        let plus_x = (width as i32 - 34).max(bx + 120);
        let view_x = (plus_x - 104).max(bx + 80);
        let path_x = bx + 6;
        let path_w = (view_x - path_x - 10).max(60);
        draw_candy_panel(px, width, height, path_x, 6, path_w, 24, 8, &PAL);
        self.text(px, width, (path_x + 10) as usize, 10, (path_w - 18).max(8) as usize, head, TEXT);

        draw_candy_tab(px, width, height, view_x, 6, 48, 24, true, ButtonState::Rest, &PAL);
        self.text(px, width, (view_x + 8) as usize, 10, 38, "Icons", TEXT);
        draw_candy_tab(px, width, height, view_x + 50, 6, 46, 24, false, ButtonState::Rest, &PAL);
        self.text(px, width, (view_x + 59) as usize, 10, 34, "List", DIM);

        draw_candy_button(px, width, height, plus_x, 6, 24, 24, 12, ButtonStyle::Normal, ButtonState::Rest, &PAL);
        self.text(px, width, (plus_x + 8) as usize, 10, 12, "+", TEXT);

        // 2. Folder Tab Bar right below the toolbar.
        fill(px, width, 0, TOOLBAR_H, width, TAB_BAR_H, PAL.panel);
        fill(px, width, 0, TOOLBAR_H + TAB_BAR_H - 1, width, 1, PAL.hairline);
        let cur_name = crate::shown(cwd.rsplit(|&b| b == b'/').find(|s| !s.is_empty()).unwrap_or(b"Home"));
        let mut tx = (sb + 8) as i32;
        let tab1_w = (cur_name.len() as i32 * 8 + 38).clamp(84, 180);
        draw_candy_tab(px, width, height, tx, TOOLBAR_H as i32 + 2, tab1_w, TAB_BAR_H as i32 - 4, true, ButtonState::Rest, &PAL);
        self.text(px, width, (tx + 10) as usize, TOOLBAR_H + 5, (tab1_w - 28).max(16) as usize, cur_name, TEXT);
        self.text(px, width, (tx + tab1_w - 16) as usize, TOOLBAR_H + 5, 12, "x", DIM);
        tx += tab1_w + 6;
        if tx + 86 < width as i32 {
            draw_candy_tab(px, width, height, tx, TOOLBAR_H as i32 + 2, 76, TAB_BAR_H as i32 - 4, false, ButtonState::Rest, &PAL);
            self.text(px, width, (tx + 10) as usize, TOOLBAR_H + 5, 58, "Home", DIM);
            tx += 82;
        }
        if tx + 28 < width as i32 {
            draw_candy_button(px, width, height, tx, TOOLBAR_H as i32 + 3, 22, TAB_BAR_H as i32 - 6, 8, ButtonStyle::Subtle, ButtonState::Rest, &PAL);
            self.text(px, width, (tx + 7) as usize, TOOLBAR_H + 5, 12, "+", DIM);
        }

        // 3. Left frosted Places / Favorites sidebar.
        if sb > 0 {
            fill(px, width, 0, TOOLBAR_H + TAB_BAR_H, sb, height - TOOLBAR_H - TAB_BAR_H - FOOT, PAL.panel);
            fill(px, width, sb - 1, TOOLBAR_H + TAB_BAR_H, 1, height - TOOLBAR_H - TAB_BAR_H - FOOT, PAL.hairline);
            self.text(px, width, 12, HEAD + 2, sb - 20, "PLACES", DIM);
            for (idx, (label, path)) in PLACES.iter().enumerate() {
                let py = HEAD + 24 + idx * 28;
                if py + 24 >= height - FOOT { break; }
                let active = cwd == *path || (idx == 0 && cwd.is_empty());
                if active {
                    draw_candy_selection_row(px, width, height, 8, py as i32, (sb - 16) as i32, 24, 8, &PAL);
                }
                self.text(px, width, 18, py + 4, sb - 28, label, if active { TEXT } else { DIM });
            }
        }

        // 4. Rounded Candy File Grid.
        let cols = Self::columns(width);
        let rows = Self::rows(height);
        let row = cursor / cols;
        if row < *top { *top = row; }
        if row >= *top + rows { *top = row + 1 - rows; }
        let first = *top * cols;
        for (i, e) in entries.iter().enumerate().skip(first).take(rows * cols) {
            let (c, r) = ((i - first) % cols, (i - first) / cols);
            let (x0, y0) = (content_x + c * CELL_W, HEAD + r * CELL_H);
            let box_w = CELL_W - 8;
            if i == cursor {
                draw_candy_button(px, width, height, x0 as i32, y0 as i32, box_w as i32, (CELL_H - 6) as i32, 10, ButtonStyle::Active, ButtonState::Rest, &PAL);
            } else {
                draw_candy_panel(px, width, height, x0 as i32, y0 as i32, box_w as i32, (CELL_H - 6) as i32, 10, &PAL);
            }
            let inner = (x0 + (box_w - SIDE) / 2, y0 + 4);
            match (kind(&e.name, e.dir), self.made.get(&join(cwd, &e.name))) {
                (_, Some(Thumb::Picture { w, h, px: t })) => {
                    let (ox, oy) = (inner.0 + (SIDE - w) / 2, inner.1 + (SIDE - h) / 2);
                    for y in 0..*h { px[(oy + y) * width + ox..(oy + y) * width + ox + w].copy_from_slice(&t[y * w..y * w + w]); }
                }
                (_, made) => {
                    let tile = if i == cursor { PAL.selection } else { TILE };
                    // A picture that would not decode says so under its icon.
                    let failed = match made { Some(Thumb::Kind(k)) if k.ends_with('?') => Some(*k), _ => None };
                    let icon = self.icon(sex_icons::file_for(&e.name, e.dir));
                    let (ox, oy) = (inner.0 + (SIDE - ICON) / 2, inner.1 + (SIDE - ICON) / 2);
                    for y in 0..ICON {
                        for x in 0..ICON {
                            let p = icon[y * ICON + x];
                            if p >> 24 != 0 { px[(oy + y) * width + ox + x] = blend(tile, p, p >> 24); }
                        }
                    }
                    if let Some(k) = failed { self.text(px, width, inner.0 + 8, inner.1 + SIDE - 18, SIDE - 16, k, DIM); }
                }
            }
            let mut name = String::from(crate::shown(&e.name));
            if e.dir { name.push('/'); }
            self.text(px, width, x0 + 6, y0 + SIDE + 6, box_w - 12, &name, TEXT);
        }

        // 5. Clean frosted Candy bottom status bar ("N Folders, M Files").
        let foot_y = height - FOOT;
        draw_candy_toolbar(px, width, height, 0, foot_y as i32, width as i32, FOOT as i32, &PAL);
        let folders = entries.iter().filter(|e| e.dir).count();
        let files_n = entries.len().saturating_sub(folders);
        let more = entries.len().saturating_sub(first + rows * cols);
        let summary = if more > 0 {
            alloc::format!("{folders} Folders, {files_n} Files  ({more} more below)  {foot}")
        } else {
            alloc::format!("{folders} Folders, {files_n} Files  {foot}")
        };
        self.text(px, width, 12, foot_y + 8, width - 24, &summary, DIM);
        window.show_pixels(width, height);
    }

    /// File icon `name` at `ICON` pixels, drawn on first use.
    fn icon(&mut self, name: &'static str) -> &[u32] {
        self.icons.entry(name).or_insert_with(|| {
            let (a, m) = sex_icons::scratch(ICON);
            let (mut acc, mut mask, mut px) = (vec![0; a], vec![0; m], vec![0; ICON * ICON]);
            sex_icons::draw(sex_icons::Kind::File, name, ICON, &sex_icons::Palette::BRAND, &mut acc, &mut mask, &mut px);
            px
        })
    }

    fn font(&mut self) -> Option<&Font<'static>> {
        if self.font.is_none() {
            self.font = Font::new(sex_font::GEIST_SANS);
            self.acc = vec![0; sex_font::raster::scratch_len(GLYPH_W, GLYPH_H)];
            self.cover = vec![0; GLYPH_W * GLYPH_H];
        }
        self.font.as_ref()
    }
    /// Draws one line of text at (`x`, `y`) top left, cut at `max` pixels wide.
    fn text(&mut self, px: &mut [u32], stride: usize, x: usize, y: usize, max: usize, s: &str, color: u32) {
        if self.font().is_none() { return; }
        let (font, acc, cover) = (self.font.as_ref().unwrap(), &mut self.acc, &mut self.cover);
        let rows = px.len() / stride;
        let mut pen = 0i32; // 1/256 pixel
        for c in s.chars() {
            let Some(g) = font.glyph(c as u32).or_else(|| font.glyph('?' as u32)) else { continue };
            let adv = font.advance(g, SIZE);
            if (pen + adv) / 256 > max as i32 { break; }
            let (whole, frac) = (pen / 256, pen % 256);
            if c != ' ' && font.draw(g, SIZE, 2 * 256 + frac, 15 * 256, GLYPH_W, GLYPH_H, acc, cover) {
                for gy in 0..GLYPH_H {
                    let py = y + gy;
                    if py >= rows { break; }
                    for gx in 0..GLYPH_W {
                        let a = cover[gy * GLYPH_W + gx] as u32;
                        if a == 0 { continue; }
                        let Some(pxx) = (x as i32 + whole + gx as i32 - 2).try_into().ok().filter(|v: &usize| *v < stride) else { continue };
                        let d = &mut px[py * stride + pxx];
                        *d = blend(*d, color, a);
                    }
                }
            }
            pen += adv;
        }
    }
}

const GLYPH_W: usize = 22;
const GLYPH_H: usize = 20;
/// 13 px, in 1/64 pixel.
const SIZE: u32 = 13 * 64;

fn blend(under: u32, over: u32, a: u32) -> u32 {
    let mix = |s: u32| ((((under >> s) & 0xFF) * (255 - a) + ((over >> s) & 0xFF) * a) / 255) << s;
    mix(16) | mix(8) | mix(0)
}
fn fill(px: &mut [u32], stride: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    for row in y..(y + h).min(px.len() / stride) { px[row * stride + x..row * stride + (x + w).min(stride)].fill(color); }
}

/// Reads and shrinks one picture: (width, height, 0x00RRGGBB pixels over the tile).
fn picture(window: &mut Window, path: &[u8], size: u64) -> Option<(usize, usize, Vec<u32>)> {
    if size == 0 || size > FILE_MAX { return None; }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size as usize).ok()?;
    bytes.resize(size as usize, 0);
    let n = files::read(window, &at(path), &mut bytes).ok()?;
    bytes.truncate(n);
    let image = sex_image::decode(&bytes, &sex_image::Limits::PHOTO).ok()?;
    drop(bytes);
    let small = image.fit_within(SIDE, SIDE).ok()?;
    let px = small.pixels.iter().map(|p| {
        let a = p >> 24;
        blend(TILE, p & 0xFF_FFFF, a)
    }).collect();
    Some((small.width, small.height, px))
}
