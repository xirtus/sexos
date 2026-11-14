// SPDX-License-Identifier: GPL-3.0-only
//! Bodice: e-books. The library is the books in the program's own folder (EPUB,
//! FictionBook, comic book archives); `run bodice @file.epub` copies a book in and
//! opens it. Each book opens where it was left (`positions.txt`); the font size and
//! night mode are kept in `settings.txt`. Reading, parsing and reflow are sex-ebook's,
//! tested on the host; this file is the window around them.
//!
//! Keys in a book: Right, Down, PageDown or Space turn forward; Left, Up, PageUp or
//! Backspace turn back; Home and End go to the first and last page; `+` and `-` change
//! the font size; `n` switches night mode; `t` opens the contents, where Enter jumps;
//! Esc goes back to the library. The mouse turns pages by clicking the right or left
//! part of the page, and the wheel turns them too. In the library Up/Down choose and
//! Enter (or a click) opens.
#![no_std]
#![no_main]

extern crate alloc;

mod draw;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use draw::{clip, Canvas, Face};
use sex_app::{files, Event, Nav, Window};
use sex_ebook::layout::{chapter_page, page_of, paginate, Page, Position, Style};
use sex_ebook::store::{Positions, Settings};
use sex_ebook::{Book, Format};
use sex_pdx::app_pointer::Kind;

const POSITIONS: &[u8] = b"positions.txt";
const SETTINGS: &[u8] = b"settings.txt";
/// The largest book file Bodice opens.
const BOOK_MAX: u64 = 64 << 20;
/// Font cell sizes, smallest first.
const SIZES: [(usize, usize); 5] = [(8, 16), (10, 20), (12, 24), (14, 28), (16, 32)];
const MARGIN: i32 = 24;
const TOP: i32 = 34;
const BOTTOM: i32 = 30;

struct Colours { back: u32, text: u32, dim: u32, heading: u32, quote: u32, select: u32, bar: u32 }
const DAY: Colours = Colours { back: 0x00F4_EFE6, text: 0x0022_2222, dim: 0x0080_7A70, heading: 0x0080_2A3A, quote: 0x0045_4A60, select: 0x00E2_D6C2, bar: 0x00E9_E2D6 };
const NIGHT: Colours = Colours { back: 0x0010_1114, text: 0x00C8_C8C0, dim: 0x0070_7070, heading: 0x00E0_9A70, quote: 0x0098_A8C8, select: 0x0028_2C36, bar: 0x0018_1A1F };

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[bodice.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn shown(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("?") }

#[derive(Clone, Copy, PartialEq)]
enum Mode { Library, Reading, Contents }

struct Open {
    name: String,
    book: Book,
    /// The file, kept for a comic whose pages are decoded as they are shown.
    data: Vec<u8>,
    pages: Vec<Page>,
    /// What `pages` was laid out for: columns, rows.
    laid: (usize, usize),
    page: usize,
    /// The decoded comic page: its index, size and pixels.
    picture: Option<(usize, usize, usize, Vec<u32>)>,
}

impl Open {
    fn count(&self) -> usize { if self.book.format == Format::Cbz { self.book.images.len() } else { self.pages.len() } }
    fn position(&self) -> Position {
        if self.book.format == Format::Cbz { return Position { chapter: self.page, offset: 0 }; }
        self.pages.get(self.page).map_or(Position::default(), Page::position)
    }
}

struct App {
    mode: Mode,
    books: Vec<String>,
    chosen: usize,
    open: Option<Open>,
    toc_chosen: usize,
    positions: Positions,
    settings: Settings,
    status: String,
    faces: Vec<Option<Face>>,
    small: Face,
}

impl App {
    fn colours(&self) -> &'static Colours { if self.settings.night { &NIGHT } else { &DAY } }
    fn face(&mut self) -> &mut Face {
        let i = self.settings.font.min(SIZES.len() - 1);
        self.faces[i].get_or_insert_with(|| Face::new(SIZES[i].0, SIZES[i].1))
    }
    fn library(&mut self, window: &mut Window) {
        self.books = files::list(window, b"").unwrap_or_default().into_iter()
            .filter(|e| !e.dir)
            .filter_map(|e| String::from_utf8(e.name).ok())
            .filter(|n| sex_ebook::is_book_name(n))
            .collect();
        self.books.sort();
        self.chosen = self.chosen.min(self.books.len().saturating_sub(1));
        sex_pdx::serial_record!("[bodice.library] books={}", self.books.len());
        for b in &self.books { sex_pdx::serial_record!("[bodice.library.book] name={}", b); }
    }
    fn save_settings(&mut self, window: &mut Window) {
        let _ = files::write(window, SETTINGS, self.settings.serialize().as_bytes());
    }
    fn save_position(&mut self, window: &mut Window) {
        let Some(o) = &self.open else { return };
        let pos = o.position();
        self.positions.set(&o.name, pos);
        let page = o.page + 1;
        let name = o.name.clone();
        match files::write(window, POSITIONS, self.positions.serialize().as_bytes()) {
            Ok(()) => sex_pdx::serial_record!("[bodice.saved] book={} page={} chapter={} offset={}", name, page, pos.chapter, pos.offset),
            Err(e) => sex_pdx::serial_record!("[bodice.save.failed] status={}", e),
        }
    }
    /// The text area in characters for the current face and window size.
    fn grid(&mut self, (w, h): (usize, usize)) -> (usize, usize) {
        let f = self.face();
        let (fw, fh) = (f.w as i32, f.h as i32);
        (((w as i32 - 2 * MARGIN) / fw).max(8) as usize, ((h as i32 - TOP - BOTTOM) / fh).max(2) as usize)
    }
    /// Lays the open book out again if the window or font changed, keeping its place.
    fn relayout(&mut self, size: (usize, usize)) {
        let grid = self.grid(size);
        let Some(o) = &mut self.open else { return };
        if o.book.format == Format::Cbz || o.laid == grid { return; }
        let pos = o.position();
        let first = o.pages.is_empty();
        o.pages = paginate(&o.book, grid.0, grid.1);
        o.laid = grid;
        if !first { o.page = page_of(&o.pages, pos); }
        sex_pdx::serial_record!("[bodice.layout] book={} cols={} rows={} pages={} page={}", o.name, grid.0, grid.1, o.pages.len(), o.page + 1);
    }
    fn read_book(&mut self, window: &mut Window, path: &[u8]) -> Result<Vec<u8>, String> {
        let size = files::stat(window, path).map_err(|e| String::from(shown(files::message(e))))?.size;
        if size > BOOK_MAX { return Err(String::from("That book is larger than 64 MiB.")); }
        let mut data = alloc::vec![0u8; size as usize];
        let n = files::read(window, path, &mut data).map_err(|e| String::from(shown(files::message(e))))?;
        data.truncate(n);
        Ok(data)
    }
    fn open_book(&mut self, window: &mut Window, name: &str) {
        let data = match self.read_book(window, name.as_bytes()) {
            Ok(d) => d,
            Err(why) => { sex_pdx::serial_record!("[bodice.open.failed] book={} why={}", name, why); self.status = format!("Could not read {name}: {why}"); return; }
        };
        let book = match sex_ebook::open(name, &data) {
            Ok(b) => b,
            Err(e) => { sex_pdx::serial_record!("[bodice.open.failed] book={} why={:?}", name, e); self.status = format!("{name} is not a book Bodice reads."); return; }
        };
        let keep = if book.format == Format::Cbz { data } else { Vec::new() };
        let saved = self.positions.get(name);
        self.open = Some(Open { name: String::from(name), book, data: keep, pages: Vec::new(), laid: (0, 0), page: 0, picture: None });
        self.relayout(window.size());
        let o = self.open.as_mut().unwrap();
        if let Some(pos) = saved {
            o.page = if o.book.format == Format::Cbz { pos.chapter.min(o.count().saturating_sub(1)) } else { page_of(&o.pages, pos) };
        }
        sex_pdx::serial_record!("[bodice.opened] book={} format={} title={} author={} chapters={} toc={} pages={} page={} saved={}",
            o.name, o.book.format.name(), o.book.title, o.book.author, o.book.parts(), o.book.toc.len(), o.count(), o.page + 1, saved.is_some() as u8);
        self.mode = Mode::Reading;
        self.status.clear();
    }
    /// Copies `@path` into the library and opens it.
    fn import(&mut self, window: &mut Window, path: &[u8]) {
        let data = match self.read_book(window, path) {
            Ok(d) => d,
            Err(why) => { sex_pdx::serial_record!("[bodice.import.failed] path={} why={}", shown(path), why); self.status = format!("Could not read {}: {why}", shown(path)); return; }
        };
        let name = shown(path).trim_start_matches('@').rsplit('/').next().unwrap_or("book");
        let name = String::from(name);
        if !sex_ebook::is_book_name(&name) || sex_ebook::open(&name, &data).is_err() {
            sex_pdx::serial_record!("[bodice.import.failed] path={} why=not-a-book", shown(path));
            self.status = format!("{name} is not a book Bodice reads.");
            return;
        }
        match files::write(window, name.as_bytes(), &data) {
            Ok(()) => sex_pdx::serial_record!("[bodice.imported] book={} bytes={}", name, data.len()),
            Err(e) => { self.status = format!("Could not keep {name}: {}", shown(files::message(e))); return; }
        }
        self.library(window);
        self.chosen = self.books.iter().position(|b| *b == name).unwrap_or(0);
        self.open_book(window, &name);
    }
    fn turn(&mut self, window: &mut Window, to: isize) {
        let Some(o) = &mut self.open else { return };
        let last = o.count().saturating_sub(1) as isize;
        let next = to.clamp(0, last) as usize;
        if next == o.page { return; }
        o.page = next;
        let pos = o.position();
        sex_pdx::serial_record!("[bodice.page] book={} page={} of={} chapter={} offset={}", o.name, o.page + 1, o.count(), pos.chapter, pos.offset);
        self.save_position(window);
    }
    fn close_book(&mut self, window: &mut Window) {
        if self.open.is_none() { return; }
        self.save_position(window);
        self.open = None;
        self.mode = Mode::Library;
        self.library(window);
    }
    fn jump(&mut self, window: &mut Window) {
        let Some(o) = &mut self.open else { return };
        let Some(entry) = o.book.toc.get(self.toc_chosen) else { return };
        let page = if o.book.format == Format::Cbz { entry.chapter } else { chapter_page(&o.pages, entry.chapter) };
        sex_pdx::serial_record!("[bodice.jump] book={} entry={} chapter={} page={}", o.name, entry.label, entry.chapter, page + 1);
        self.mode = Mode::Reading;
        self.turn(window, page as isize);
    }
    fn set_font(&mut self, window: &mut Window, font: usize) {
        if font == self.settings.font { return; }
        self.settings.font = font;
        sex_pdx::serial_record!("[bodice.font] size={}", SIZES[font].1);
        self.save_settings(window);
        self.relayout(window.size());
    }
    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let typed = sex_app::typed(key);
        let nav = sex_app::nav(key);
        match self.mode {
            Mode::Library => match (key.code, nav) {
                (0x1C, _) => if let Some(name) = self.books.get(self.chosen).cloned() { self.open_book(window, &name); },
                (_, Some(Nav::Up)) => self.chosen = self.chosen.saturating_sub(1),
                (_, Some(Nav::Down)) => self.chosen = (self.chosen + 1).min(self.books.len().saturating_sub(1)),
                _ => {}
            },
            Mode::Contents => {
                let n = self.open.as_ref().map_or(0, |o| o.book.toc.len());
                match (key.code, nav) {
                    (0x1C, _) => self.jump(window),
                    (0x01, _) => self.mode = Mode::Reading,
                    (_, Some(Nav::Up)) => self.toc_chosen = self.toc_chosen.saturating_sub(1),
                    (_, Some(Nav::Down)) => self.toc_chosen = (self.toc_chosen + 1).min(n.saturating_sub(1)),
                    _ => if typed == Some(b't') { self.mode = Mode::Reading; },
                }
            }
            Mode::Reading => {
                let page = self.open.as_ref().map_or(0, |o| o.page) as isize;
                let last = self.open.as_ref().map_or(0, |o| o.count()) as isize;
                match (key.code, nav, typed) {
                    (0x01, _, _) => self.close_book(window),
                    (0x51 | 0x6D, _, _) | (_, Some(Nav::Right | Nav::Down), _) | (_, _, Some(b' ')) => self.turn(window, page + 1),
                    (0x49 | 0x68 | 0x0E, _, _) | (_, Some(Nav::Left | Nav::Up), _) => self.turn(window, page - 1),
                    (_, Some(Nav::Home), _) => self.turn(window, 0),
                    (_, Some(Nav::End), _) => self.turn(window, last - 1),
                    (_, _, Some(b'+' | b'=')) => self.set_font(window, (self.settings.font + 1).min(SIZES.len() - 1)),
                    (_, _, Some(b'-')) => self.set_font(window, self.settings.font.saturating_sub(1)),
                    (_, _, Some(b'n')) => {
                        self.settings.night = !self.settings.night;
                        sex_pdx::serial_record!("[bodice.night] on={}", self.settings.night as u8);
                        self.save_settings(window);
                    }
                    (_, _, Some(b't')) => {
                        let Some(o) = &self.open else { return };
                        // The contents open on the entry for the chapter being read.
                        let here = o.position().chapter;
                        self.toc_chosen = o.book.toc.iter().rposition(|t| t.chapter <= here).unwrap_or(0);
                        sex_pdx::serial_record!("[bodice.contents] entries={}", o.book.toc.len());
                        for t in &o.book.toc { sex_pdx::serial_record!("[bodice.contents.entry] chapter={} depth={} label={}", t.chapter, t.depth, t.label); }
                        self.mode = Mode::Contents;
                    }
                    _ => {}
                }
            }
        }
    }
    fn pointer(&mut self, window: &mut Window, p: sex_pdx::app_pointer::Pointer) {
        let (w, _) = window.size();
        let row = |y: i32, h: i32| ((y - TOP - 8) / h).max(0) as usize;
        match (self.mode, p.kind) {
            (Mode::Reading, Kind::Down { button: 0 }) => {
                let page = self.open.as_ref().map_or(0, |o| o.page) as isize;
                if (p.x as usize) > w * 2 / 3 { self.turn(window, page + 1); } else if (p.x as usize) < w / 3 { self.turn(window, page - 1); }
            }
            (Mode::Reading, Kind::Wheel { steps }) => {
                let page = self.open.as_ref().map_or(0, |o| o.page) as isize;
                self.turn(window, page - steps as isize);
            }
            (Mode::Library, Kind::Down { button: 0 }) => {
                let i = row(p.y as i32, self.small.h as i32 * 2);
                if i < self.books.len() {
                    if i == self.chosen { let n = self.books[i].clone(); self.open_book(window, &n); } else { self.chosen = i; }
                }
            }
            (Mode::Contents, Kind::Down { button: 0 }) => {
                let i = row(p.y as i32, self.small.h as i32 + 4);
                if self.open.as_ref().is_some_and(|o| i < o.book.toc.len()) { self.toc_chosen = i; self.jump(window); }
            }
            _ => {}
        }
    }
}

fn render(window: &mut Window, app: &mut App) {
    let (w, h) = window.size();
    let (w, h) = (w.max(200), h.max(160));
    let col = app.colours();
    let mut c = Canvas { px: window.pixels(), w, h };
    c.fill(0, 0, w as i32, h as i32, col.back);
    c.fill(0, 0, w as i32, TOP - 6, col.bar);
    let (wi, hi) = (w as i32, h as i32);
    let small = &mut app.small;
    match app.mode {
        Mode::Library => {
            c.text(small, MARGIN, 6, "Bodice library", col.heading, wi - 2 * MARGIN);
            if app.books.is_empty() {
                c.text(small, MARGIN, TOP + 8, "No books yet. Start Bodice with run bodice @book.epub to add one.", col.dim, wi - 2 * MARGIN);
            }
            let rh = small.h as i32 * 2;
            let cols = ((wi - 2 * MARGIN) / small.w as i32).max(4) as usize;
            for (i, b) in app.books.iter().enumerate() {
                let y = TOP + 8 + i as i32 * rh;
                if y + rh > hi - BOTTOM { break; }
                if i == app.chosen { c.fill(MARGIN - 8, y - 4, wi - 2 * MARGIN + 16, rh, col.select); }
                let place = app.positions.get(b).map_or(String::from("new"), |p| format!("left in part {}", p.chapter + 1));
                c.text(small, MARGIN, y, clip(b, cols.saturating_sub(24)), col.text, wi - 2 * MARGIN);
                c.text(small, wi - MARGIN - (place.len() * small.w) as i32, y, &place, col.dim, wi);
            }
        }
        Mode::Contents => {
            let Some(o) = &app.open else { return };
            c.text(small, MARGIN, 6, &format!("Contents of {}", if o.book.title.is_empty() { &o.name } else { &o.book.title }), col.heading, wi - 2 * MARGIN);
            let rh = small.h as i32 + 4;
            let cols = ((wi - 2 * MARGIN) / small.w as i32).max(4) as usize;
            for (i, t) in o.book.toc.iter().enumerate() {
                let y = TOP + 8 + i as i32 * rh;
                if y + rh > hi - BOTTOM { break; }
                if i == app.toc_chosen { c.fill(MARGIN - 8, y - 2, wi - 2 * MARGIN + 16, rh, col.select); }
                let indent = t.depth.min(6) as i32 * 2 * small.w as i32;
                c.text(small, MARGIN + indent, y, clip(&t.label, cols), col.text, wi - 2 * MARGIN - indent);
            }
        }
        Mode::Reading => {
            let Some(o) = &mut app.open else { return };
            let title = if o.book.title.is_empty() { o.name.clone() } else { o.book.title.clone() };
            let where_ = format!("{} / {}", o.page + 1, o.count());
            let room = wi - 2 * MARGIN - (where_.len() as i32 + 2) * small.w as i32;
            c.text(small, MARGIN, 6, clip(&title, (room / small.w as i32).max(0) as usize), col.dim, room);
            c.text(small, wi - MARGIN - (where_.len() * small.w) as i32, 6, &where_, col.dim, wi);
            if o.book.format == Format::Cbz {
                if o.picture.as_ref().map(|p| p.0) != Some(o.page) {
                    o.picture = None;
                    if let Ok(z) = sex_ebook::zip::Zip::open(&o.data) {
                        if let Some(bytes) = o.book.images.get(o.page).and_then(|n| z.read(n, 32 << 20).ok()) {
                            if let Ok(img) = sex_image::decode(&bytes, &sex_image::Limits::PHOTO) { o.picture = Some((o.page, img.width, img.height, img.pixels)); }
                        }
                    }
                }
                match &o.picture {
                    Some((_, pw, ph, px)) => c.picture(MARGIN, TOP, wi - 2 * MARGIN, hi - TOP - BOTTOM, *pw, *ph, px),
                    None => { c.text(small, MARGIN, TOP + 8, "This page's picture could not be read.", col.dim, wi); }
                }
            } else if let Some(page) = o.pages.get(o.page) {
                let i = app.settings.font.min(SIZES.len() - 1);
                let face = app.faces[i].get_or_insert_with(|| Face::new(SIZES[i].0, SIZES[i].1));
                for (k, line) in page.lines.iter().enumerate() {
                    let colour = match line.style { Style::Heading => col.heading, Style::Quote => col.quote, Style::Pre => col.text, Style::Body => col.text };
                    c.text(face, MARGIN, TOP + k as i32 * face.h as i32, &line.text, colour, wi - 2 * MARGIN);
                }
            }
        }
    }
    let bottom = if !app.status.is_empty() { app.status.clone() } else {
        String::from(match app.mode {
            Mode::Library => "Up/Down choose, Enter opens",
            Mode::Contents => "Up/Down choose, Enter jumps, Esc back",
            Mode::Reading => "Arrows or clicks turn pages, t contents, + - size, n night, Esc library",
        })
    };
    c.text(&mut app.small, MARGIN, hi - BOTTOM + 6, &bottom, col.dim, wi - 2 * MARGIN);
    window.show_pixels(w, h);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Bodice");
    let mut buf = alloc::vec![0u8; 256 << 10];
    let positions = match files::read(&mut window, POSITIONS, &mut buf) { Ok(n) => Positions::parse(&String::from_utf8_lossy(&buf[..n])), Err(_) => Positions::default() };
    let mut settings = match files::read(&mut window, SETTINGS, &mut buf) { Ok(n) => Settings::parse(&String::from_utf8_lossy(&buf[..n])), Err(_) => Settings::default() };
    settings.font = settings.font.min(SIZES.len() - 1);
    drop(buf);
    let mut app = App {
        mode: Mode::Library, books: Vec::new(), chosen: 0, open: None, toc_chosen: 0, positions, settings,
        status: String::new(), faces: (0..SIZES.len()).map(|_| None).collect(), small: Face::new(9, 18),
    };
    app.library(&mut window);
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args).to_vec();
    if let Some(path) = given.split(|b| *b == b' ').find(|w| w.first() == Some(&b'@')) {
        app.import(&mut window, path);
    } else if let Some(name) = given.split(|b| *b == b' ').find(|w| !w.is_empty()).map(|w| String::from(shown(w))) {
        app.open_book(&mut window, &name);
    }
    render(&mut window, &mut app);
    sex_pdx::serial_record!("[bodice.ready] target={} task={} books={} night={} font={}", window.target, window.task, app.books.len(), app.settings.night as u8, SIZES[app.settings.font].1);
    loop {
        match window.next() {
            Event::Close => {
                app.close_book(&mut window);
                sex_pdx::serial_record!("[bodice.closed] target={} task={}", window.target, window.task);
                window.close()
            }
            Event::Key(key) => { app.status.clear(); app.key(&mut window, key); }
            Event::Pointer(p) if matches!(p.kind, Kind::Down { button: 0 } | Kind::Wheel { .. }) => app.pointer(&mut window, p),
            Event::Pointer(_) => continue,
            Event::Resize { .. } => app.relayout(window.size()),
        }
        render(&mut window, &mut app);
    }
}
