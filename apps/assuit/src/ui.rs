// SPDX-License-Identifier: GPL-3.0-only
//! The page view: WordCraft draws the caret's page, this paints it centred on a grey
//! desk with the caret over it and a status line, and turns SexOS keys into WordCraft's
//! shortcuts. GridCraft-style egui chrome (ribbon, rulers, panes) follows with M11.
use crate::book::{Book, Chord, Picture};
use sex_office::{Canvas, Pen};
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx::app_key::Key;
use sex_std::sex_pdx::app_pointer::{Kind, Pointer};

const DESK: u32 = 0x00e8eaed;
const SHADOW: u32 = 0x00c4c7cc;
const CARET: u32 = 0x00000000;
const BAR_BG: u32 = 0x00f8f9fa;
const DIM: u32 = 0x005f6368;
const LINE: u32 = 0x00dadce0;
const STATUS_H: i32 = 26;
const MARGIN: i32 = 16;

pub struct Ui {
    book: Book,
    name: String,
    /// 1.0 fits the page to the window's width.
    zoom: f32,
    scroll: i32,
    /// The drawn page: (page, scale, picture).
    shown: Option<(usize, f32, Picture)>,
    /// The page being looked at, when it is not the caret's (Page Up/Down without moving).
    message: String,
    pen: Pen,
}

/// Set-1 scan codes Shell passes.
mod code {
    pub const ESC: u8 = 0x01;
    pub const BACKSPACE: u8 = 0x0E;
    pub const TAB: u8 = 0x0F;
    pub const ENTER: u8 = 0x1C;
    pub const S: u8 = 0x1F;
    pub fn page_up(c: u8) -> bool { matches!(c, 0x49 | 0x68) }
    pub fn page_down(c: u8) -> bool { matches!(c, 0x51 | 0x6D) }
}

/// The name WordCraft's shortcuts give `key`, with its character when it types one.
fn key_name(key: Key) -> Option<String> {
    if let Some(nav) = sex_app::nav(key) {
        return Some(match nav { Nav::Left => "Left", Nav::Right => "Right", Nav::Up => "Up", Nav::Down => "Down", Nav::Home => "Home", Nav::End => "End", Nav::Delete => "Delete" }.into());
    }
    Some(match key.code {
        code::ESC => "Escape".into(),
        code::BACKSPACE => "Backspace".into(),
        code::TAB => "Tab".into(),
        code::ENTER => "Enter".into(),
        c if code::page_up(c) => "PageUp".into(),
        c if code::page_down(c) => "PageDown".into(),
        // Ctrl+letter: the letter as a US keyboard has it.
        _ => {
            let plain = sex_std::sex_pdx::app_key::us_char(Key { control: false, shift: false, ..key })?;
            (plain as char).to_ascii_uppercase().to_string()
        }
    })
}

impl Ui {
    pub fn new(book: Book, name: String) -> Ui {
        Ui { book, name, zoom: 1.0, scroll: 0, shown: None, message: String::new(), pen: Pen::sans(13) }
    }

    fn scale(&mut self, page: usize, width: i32) -> f32 {
        let (pw, _) = self.book.page_size(page);
        ((width - 2 * MARGIN).max(64) as f32 / pw.max(1.0) * self.zoom).clamp(0.1, 6.0)
    }

    /// Draws the caret's page again: after any change, the picture is stale.
    fn invalidate(&mut self) { self.shown = None; }

    pub fn draw(&mut self, window: &mut sex_app::Window) {
        let (width, height) = window.size();
        let (w, h) = (width as i32, height as i32);
        let caret = self.book.caret();
        let page = caret.map_or(0, |c| c.page);
        let scale = self.scale(page, w);
        if !matches!(&self.shown, Some((p, s, _)) if *p == page && *s == scale) {
            let started = std::time::Instant::now();
            if let Some(picture) = self.book.draw(page, scale) {
                let ink = picture.pixels.iter().filter(|p| **p != 0x00ffffff).count();
                sex_std::record(&format!("[assuit.page] page={} pages={} width={} height={} ink={ink} ms={}",
                    page + 1, self.book.pages(), picture.width, picture.height, started.elapsed().as_millis()));
                self.shown = Some((page, scale, picture));
            }
        }
        let view_h = h - STATUS_H;
        // Keep the caret on screen.
        if let Some(c) = caret {
            let (top, bottom) = ((c.top * scale) as i32 + MARGIN, ((c.top + c.height) * scale) as i32 + MARGIN);
            if top < self.scroll { self.scroll = top - MARGIN; }
            if bottom > self.scroll + view_h { self.scroll = bottom - view_h + MARGIN; }
        }
        let mut canvas = Canvas::new(window.pixels(), width, height);
        canvas.fill(0, 0, w, h, DESK);
        if let Some((_, _, picture)) = &self.shown {
            let (pw, ph) = (picture.width as i32, picture.height as i32);
            self.scroll = self.scroll.clamp(0, (ph + 2 * MARGIN - view_h).max(0));
            let left = ((w - pw) / 2).max(0);
            let top = MARGIN - self.scroll;
            canvas.fill(left + 2, top + 2, pw, ph, SHADOW);
            for y in 0..ph {
                let sy = top + y;
                if sy < 0 { continue; }
                if sy >= view_h { break; }
                let n = pw.min(w - left) as usize;
                let row = (sy * w + left) as usize;
                canvas.px[row..row + n].copy_from_slice(&picture.pixels[y as usize * picture.width..y as usize * picture.width + n]);
            }
            if let Some(c) = caret {
                let x = left + (c.x * scale) as i32;
                let y = top + (c.top * scale) as i32;
                let ch = ((c.height * scale) as i32).max(8);
                canvas.fill(x, y.max(0), 2, (ch + y.min(0)).min(view_h - y.max(0)), CARET);
            }
        }
        canvas.fill(0, view_h, w, STATUS_H, BAR_BG);
        canvas.fill(0, view_h, w, 1, LINE);
        let pages = self.book.pages();
        let status = if self.message.is_empty() {
            format!("{}{}   Page {} of {}   Ctrl+S save  Ctrl+B/I/U style  Ctrl+Z undo  Ctrl+=/- zoom",
                self.name, if self.book.dirty() { " (changed)" } else { "" }, page + 1, pages)
        } else {
            self.message.clone()
        };
        self.pen.text(&mut canvas, 10, view_h + (STATUS_H - self.pen.line_height()) / 2, &status, DIM, Some((0, w - 10)));
        window.show_pixels(width, height);
        let short: String = self.name.chars().take(15).collect();
        window.title(format!("{short}{}", if self.book.dirty() { "*" } else { "" }).as_bytes());
    }

    fn save(&mut self) {
        match self.book.save() {
            Ok(bytes) => {
                sex_std::record(&format!("[assuit.saved] name={} bytes={bytes}", self.book.path));
                self.message = format!("Saved {} ({bytes} bytes)", self.name);
            }
            Err(e) => {
                sex_std::record(&format!("[assuit.save-failed] {e}"));
                self.message = e;
            }
        }
    }

    /// One key: Assuit's own (save, zoom), then WordCraft's shortcut, then typed text.
    fn key(&mut self, key: Key) {
        self.message.clear();
        if key.control {
            match (key.code, sex_app::typed(Key { control: false, ..key })) {
                (code::S, _) => return self.save(),
                (_, Some(b'=' | b'+')) => { self.zoom = (self.zoom * 1.25).min(4.0); return; }
                (_, Some(b'-')) => { self.zoom = (self.zoom / 1.25).max(0.25); return; }
                (_, Some(b'0')) => { self.zoom = 1.0; return; }
                _ => {}
            }
        }
        if let Some(name) = key_name(key) {
            let typed = !key.control && name.chars().count() == 1;
            if !typed {
                if let Some(id) = self.book.key(&Chord { name: &name, ctrl: key.control, shift: key.shift }) {
                    if !id.starts_with("caret.") { sex_std::record(&format!("[assuit.command] {id}")); }
                    self.invalidate();
                    return;
                }
            }
        }
        if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control() && !key.control) {
            let mut s = [0u8; 4];
            if self.book.type_text(ch.encode_utf8(&mut s)).is_ok() { self.invalidate(); }
        }
    }

    fn pointer(&mut self, p: Pointer, window: &sex_app::Window) {
        let (width, _) = window.size();
        match p.kind {
            Kind::Wheel { steps } => self.scroll -= steps as i32 * 60,
            Kind::Down { button: 0 } => {
                let Some((page, scale, picture)) = &self.shown else { return };
                let left = ((width as i32 - picture.width as i32) / 2).max(0);
                let (x, y) = ((p.x as i32 - left) as f32 / scale, (p.y as i32 + self.scroll - MARGIN) as f32 / scale);
                let page = *page;
                if self.book.click(page, x, y) { self.invalidate(); }
            }
            _ => {}
        }
    }

    pub fn run(mut self, window: &mut sex_app::Window) {
        self.draw(window);
        loop {
            match window.next() {
                Event::Close => break,
                Event::Resize { .. } => self.draw(window),
                Event::Key(key) => {
                    let dirty = self.book.dirty();
                    self.key(key);
                    self.draw(window);
                    if self.book.dirty() != dirty || self.book.dirty() {
                        let text = self.book.text();
                        let first: String = text.lines().next().unwrap_or("").chars().take(60).collect();
                        sex_std::record(&format!("[assuit.text] chars={} first={first}", text.chars().count()));
                    }
                }
                Event::Pointer(p) => {
                    if matches!(p.kind, Kind::Down { .. } | Kind::Wheel { .. }) {
                        self.pointer(p, window);
                        self.draw(window);
                    }
                }
            }
        }
        sex_std::record("[assuit.closed]");
    }
}
