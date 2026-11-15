// SPDX-License-Identifier: GPL-3.0-only
//! The page view: WordCraft draws the pages, this stacks them on a grey desk and
//! scrolls through them, with the caret or the selection's highlight over them and a
//! status line, and turns SexOS keys into WordCraft's shortcuts. Only pages on screen
//! are drawn, and only again after an edit. WordCraft's egui chrome (ribbon, rulers,
//! panes) follows with M11.
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
    /// How far down the stack of pages the view is, in pixels.
    scroll: i32,
    /// Drawn pages by number: (scale, picture). Dropped after any edit.
    shown: Vec<Option<(f32, Picture)>>,
    message: String,
    pen: Pen,
    /// Where each page sat in the last draw: (top in the stack, left, width, height, scale).
    places: Vec<(i32, i32, i32, i32, f32)>,
    /// What Ctrl+F is being asked to find, while its prompt is open.
    find: Option<String>,
}

const GAP: i32 = 12;
const SELECTED: u32 = 0x001a73e8;

/// Set-1 scan codes Shell passes.
mod code {
    pub const ESC: u8 = 0x01;
    pub const BACKSPACE: u8 = 0x0E;
    pub const TAB: u8 = 0x0F;
    pub const ENTER: u8 = 0x1C;
    pub const S: u8 = 0x1F;
    pub const F: u8 = 0x21;
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
        Ui { book, name, zoom: 1.0, scroll: 0, shown: Vec::new(), message: String::new(), pen: Pen::sans(13), places: Vec::new(), find: None }
    }

    fn scale(&mut self, page: usize, width: i32) -> f32 {
        let (pw, _) = self.book.page_size(page);
        ((width - 2 * MARGIN).max(64) as f32 / pw.max(1.0) * self.zoom).clamp(0.1, 6.0)
    }

    /// The pages change after an edit: draw them again.
    fn invalidate(&mut self) { self.shown.clear(); }

    /// Lays the pages out one under another for a window `w` wide.
    fn place(&mut self, w: i32) {
        let pages = self.book.pages();
        let mut places = Vec::with_capacity(pages);
        let mut top = MARGIN;
        for page in 0..pages {
            let scale = self.scale(page, w);
            let (pw, ph) = self.book.page_size(page);
            let (width, height) = ((pw * scale).ceil() as i32, (ph * scale).ceil() as i32);
            places.push((top, ((w - width) / 2).max(0), width, height, scale));
            top += height + GAP;
        }
        self.places = places;
    }

    fn picture(&mut self, page: usize, scale: f32) -> Option<&Picture> {
        if self.shown.len() <= page { self.shown.resize_with(page + 1, || None); }
        if !matches!(&self.shown[page], Some((s, _)) if *s == scale) {
            let started = std::time::Instant::now();
            let picture = self.book.draw(page, scale)?;
            let ink = picture.pixels.iter().filter(|p| **p != 0x00ffffff).count();
            sex_std::record(&format!("[assuit.page] page={} pages={} width={} height={} ink={ink} ms={}",
                page + 1, self.book.pages(), picture.width, picture.height, started.elapsed().as_millis()));
            self.shown[page] = Some((scale, picture));
        }
        self.shown[page].as_ref().map(|(_, p)| p)
    }

    pub fn draw(&mut self, window: &mut sex_app::Window) {
        let (width, height) = window.size();
        let (w, h) = (width as i32, height as i32);
        let view_h = h - STATUS_H;
        self.place(w);
        let caret = self.book.caret();
        // Keep the caret on screen.
        if let Some(c) = caret {
            if let Some(&(top, _, _, _, scale)) = self.places.get(c.page) {
                let (y0, y1) = (top + (c.top * scale) as i32, top + ((c.top + c.height) * scale) as i32);
                if y0 < self.scroll + MARGIN { self.scroll = y0 - MARGIN; }
                if y1 > self.scroll + view_h - MARGIN { self.scroll = y1 - view_h + MARGIN; }
            }
        }
        let total = self.places.last().map_or(0, |p| p.0 + p.3 + MARGIN);
        self.scroll = self.scroll.clamp(0, (total - view_h).max(0));
        let selection = self.book.selection_rects();
        let mut drawn = Vec::new();
        for page in 0..self.places.len() {
            let (top, left, pw, ph, scale) = self.places[page];
            let y = top - self.scroll;
            if y >= view_h || y + ph <= 0 { continue; }
            if self.picture(page, scale).is_some() { drawn.push((page, left, y, pw, ph, scale)); }
        }
        let mut canvas = Canvas::new(window.pixels(), width, height);
        canvas.fill(0, 0, w, h, DESK);
        let clip = (0, 0, w, view_h);
        for (page, left, y, pw, ph, scale) in drawn {
            canvas.fill(left + 2, y + 2, pw, ph, SHADOW);
            if let Some((_, picture)) = &self.shown[page] { canvas.blit(left, y, &picture.pixels, picture.width, clip); }
            for &(_, rx, ry, rw, rh) in selection.iter().filter(|r| r.0 == page) {
                let (x0, y0) = (left + (rx * scale) as i32, y + (ry * scale) as i32);
                let (x1, y1) = (left + ((rx + rw) * scale).ceil() as i32, (y + ((ry + rh) * scale).ceil() as i32).min(view_h));
                canvas.tint(x0, y0, x1 - x0, y1 - y0, SELECTED, 70);
            }
            if let Some(c) = caret.filter(|c| c.page == page && selection.is_empty()) {
                let cx = left + (c.x * scale) as i32;
                let cy = y + (c.top * scale) as i32;
                let ch = ((c.height * scale) as i32).max(8);
                let (top, bottom) = (cy.max(0), (cy + ch).min(view_h));
                if bottom > top { canvas.fill(cx, top, 2, bottom - top, CARET); }
            }
        }
        canvas.fill(0, view_h, w, STATUS_H, BAR_BG);
        canvas.fill(0, view_h, w, 1, LINE);
        let pages = self.places.len();
        let page = caret.map_or(0, |c| c.page);
        let status = if let Some(find) = &self.find {
            let said = if self.message.is_empty() { "Enter finds the next, Esc closes" } else { &self.message };
            format!("Find: {find}_   {said}")
        } else if self.message.is_empty() {
            format!("{}{}   Page {} of {}   {} words   Ctrl+S save  Ctrl+F find  Ctrl+C/X/V copy  Ctrl+B/I/U style  Ctrl+Z undo  Ctrl+=/- zoom",
                self.name, if self.book.dirty() { " (changed)" } else { "" }, page + 1, pages, self.book.words())
        } else {
            self.message.clone()
        };
        self.pen.text(&mut canvas, 10, view_h + (STATUS_H - self.pen.line_height()) / 2, &status, DIM, Some((0, w - 10)));
        window.show_pixels(width, height);
        let short: String = self.name.chars().take(15).collect();
        window.title(format!("{short}{}", if self.book.dirty() { "*" } else { "" }).as_bytes());
        if !selection.is_empty() {
            sex_std::record(&format!("[assuit.selection] rects={} pages={}", selection.len(), {
                let mut p: Vec<usize> = selection.iter().map(|r| r.0 + 1).collect(); p.dedup(); p.len() }));
        }
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
        if self.find.is_some() { return self.find_key(key); }
        self.message.clear();
        if key.control {
            match (key.code, sex_app::typed(Key { control: false, ..key })) {
                (code::S, _) => return self.save(),
                (code::F, _) if !key.shift => { self.find = Some(String::new()); return; }
                (_, Some(b'=' | b'+')) => { self.zoom = (self.zoom * 1.25).min(4.0); self.invalidate(); return; }
                (_, Some(b'-')) => { self.zoom = (self.zoom / 1.25).max(0.25); self.invalidate(); return; }
                (_, Some(b'0')) => { self.zoom = 1.0; self.invalidate(); return; }
                _ => {}
            }
        }
        if let Some(name) = key_name(key) {
            let typed = !key.control && name.chars().count() == 1;
            if !typed {
                if let Some(id) = self.book.key(&Chord { name: &name, ctrl: key.control, shift: key.shift }) {
                    if !id.starts_with("caret.") {
                        sex_std::record(&format!("[assuit.command] {id}"));
                        self.invalidate();
                    }
                    return;
                }
            }
        }
        if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control() && !key.control) {
            let mut s = [0u8; 4];
            if self.book.type_text(ch.encode_utf8(&mut s)).is_ok() { self.invalidate(); }
        }
    }

    /// A key while the find prompt is open: Enter selects the next match.
    fn find_key(&mut self, key: Key) {
        let Some(find) = &mut self.find else { return };
        match key.code {
            code::ESC => self.find = None,
            code::BACKSPACE => { find.pop(); self.message.clear(); }
            code::ENTER if !find.is_empty() => {
                let what = find.clone();
                match self.book.find_next(&what) {
                    Ok((at, count)) => {
                        sex_std::record(&format!("[assuit.find] what={what} match={at} of={count}"));
                        self.message = format!("\"{what}\": match {at} of {count}");
                    }
                    Err(e) => {
                        sex_std::record(&format!("[assuit.find] what={what} none"));
                        self.message = e;
                    }
                }
                // The prompt stays open: Enter again goes on to the next match.
            }
            _ => if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) { find.push(ch); self.message.clear(); },
        }
    }

    /// A click puts the caret where it lands; dragging with the button held selects;
    /// the wheel scrolls.
    fn pointer(&mut self, p: Pointer) -> bool {
        let extend = match p.kind {
            Kind::Wheel { steps } => { self.scroll -= steps as i32 * 60; return true; }
            Kind::Down { button: 0 } => false,
            Kind::Move if p.buttons & 1 != 0 => true,
            _ => return false,
        };
        let (x, y) = (p.x as i32, p.y as i32 + self.scroll);
        let Some(page) = self.places.iter().position(|&(top, _, _, ph, _)| y >= top && y < top + ph + GAP) else { return false };
        let (top, left, _, _, scale) = self.places[page];
        self.book.click(page, (x - left) as f32 / scale, (y - top) as f32 / scale, extend)
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
                    if self.pointer(p) { self.draw(window); }
                }
                // Drag and drop: nothing to take yet.
                _ => {}
            }
        }
        sex_std::record("[assuit.closed]");
    }
}
