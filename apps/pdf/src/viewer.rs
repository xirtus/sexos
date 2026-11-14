// SPDX-License-Identifier: GPL-3.0-only
//! The window, the keys and the rendered page, with the find bar, the thumbnail strip
//! and the outline panel around it.
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx::app_pointer::Kind;
use std::collections::HashMap;
use std::time::Instant;

use crate::search::{self, Hit};
use crate::text::{self, PageText};
use crate::zoom::{self, Zoom};
use crate::{outline, sample};

const BACKGROUND: u32 = 0x003a3a3e;
const PANEL_BACK: u32 = 0x002a2a2e;
const BAR_BACK: u32 = 0x00202024;
const INK: u32 = 0x00e8e8ea;
const DIM: u32 = 0x009a9aa0;
const ACCENT: u32 = 0x004f8fe8;
/// Search hits are multiplied into the page, so text under them stays legible.
const HIT_NOW: u32 = 0x00ffd23a;
const HIT_OTHER: u32 = 0x00fff2b0;
const MARGIN: usize = 12;
/// The thumbnail strip and the outline share one panel on the left.
const PANEL: usize = 168;
const BAR: usize = 30;
/// One character of the bar's and the outline's text.
const CW: usize = 9;
const CH: usize = 16;
const ROW: usize = 20;

/// Keyboard codes Shell passes through.
const ESCAPE: u8 = 0x01;
const BACKSPACE: u8 = 0x0E;
const ENTER: u8 = 0x1C;
const KEY_F: u8 = 0x21;
const KEY_G: u8 = 0x22;
/// Shell keeps F3 for itself today; it is honoured if it ever arrives.
const F3: u8 = 0x3D;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel { None, Thumbs, Outline }

/// A rendered page: pixels 0x00RRGGBB, width, height, which page, at what scale.
struct Rendered { pixels: Vec<u32>, w: usize, h: usize, page: usize, scale: f32 }

/// Text drawn with Geist Mono, each character rasterised once.
struct Letters { font: sex_font::Font<'static>, acc: Vec<i32>, cells: HashMap<char, Vec<u8>> }

impl Letters {
    fn new() -> Option<Self> {
        Some(Self { font: sex_font::Font::new(sex_font::GEIST_MONO)?, acc: vec![0; sex_font::raster::scratch_len(CW, CH)], cells: HashMap::new() })
    }
    /// Draws `s` from (x, y) in a `stride` wide buffer, cut at `right`.
    fn draw(&mut self, out: &mut [u32], stride: usize, (x, y): (usize, usize), right: usize, s: &str, color: u32) {
        for (i, c) in s.chars().enumerate() {
            let cx = x + i * CW;
            if cx + CW > right { break; }
            let (font, acc) = (&self.font, &mut self.acc);
            let cell = self.cells.entry(c).or_insert_with(|| {
                let mut out = vec![0u8; CW * CH];
                if !sex_font::draw_cell(font, c as u32, CW, CH, acc, &mut out) { out.fill(0); }
                out
            });
            for (row, line) in cell.chunks_exact(CW).enumerate() {
                let at = (y + row) * stride + cx;
                let Some(dst) = out.get_mut(at..at + CW) else { return };
                for (p, a) in dst.iter_mut().zip(line) {
                    if *a != 0 { *p = mix(*p, color, *a as u32); }
                }
            }
        }
    }
}

/// `over` laid on `under` with coverage `a` of 255.
fn mix(under: u32, over: u32, a: u32) -> u32 {
    let ch = |s: u32| ((((under >> s) & 0xFF) * (255 - a) + ((over >> s) & 0xFF) * a) / 255) << s;
    ch(16) | ch(8) | ch(0)
}

/// `p` seen through a coloured filter: white becomes the filter's colour, black stays.
fn multiply(p: u32, filter: u32) -> u32 {
    let ch = |s: u32| ((((p >> s) & 0xFF) * ((filter >> s) & 0xFF)) / 255) << s;
    ch(16) | ch(8) | ch(0)
}

fn fill(out: &mut [u32], stride: usize, (x, y, w, h): (usize, usize, usize, usize), color: u32) {
    for row in y..y + h {
        if let Some(line) = out.get_mut(row * stride + x..row * stride + x + w) { line.fill(color); }
    }
}

fn render_page(pdf: &Pdf, index: usize, scale: f32) -> Option<(Vec<u32>, usize, usize)> {
    let pages = pdf.pages();
    let page = pages.get(index)?;
    let cache = RenderCache::new();
    let pixmap = render(page, &cache, &InterpreterSettings::default(), &RenderSettings::default(),
        &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE });
    let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
    let pixels = pixmap.data_as_u8_slice().chunks_exact(4).map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect();
    Some((pixels, w, h))
}

struct Viewer {
    pdf: Pdf,
    name: String,
    page: usize,
    zoom: Zoom,
    scroll: usize,
    hscroll: usize,
    shown: Option<Rendered>,
    panel: Panel,
    thumbs: Vec<Option<(Vec<u32>, usize, usize)>>,
    /// The first thumbnail slot shown, so the current page stays in view.
    thumb_top: usize,
    outline: Option<Vec<outline::Entry>>,
    chosen: usize,
    /// The find bar: open, what is typed, the hits for `searched`, the hit shown.
    finding: bool,
    term: String,
    searched: String,
    hits: Vec<Hit>,
    hit: Option<Hit>,
    found_none: bool,
    /// Every page's text, read the first time anything is searched.
    texts: Option<Vec<PageText>>,
    /// Where a hit that was just chosen sits, in points from the page's top.
    reveal: Option<f32>,
    letters: Option<Letters>,
}

/// Where each part of the window is.
struct Layout { panel: usize, bar: usize, x0: usize, y0: usize, aw: usize, ah: usize }

impl Viewer {
    fn pages(&self) -> usize { self.pdf.pages().len() }

    fn layout(&self, (width, height): (usize, usize)) -> Layout {
        let panel = if self.panel == Panel::None || width < PANEL * 2 { 0 } else { PANEL };
        let bar = if self.finding { BAR.min(height / 2) } else { 0 };
        Layout { panel, bar, x0: panel, y0: bar, aw: width - panel, ah: height - bar }
    }

    fn scale(&self, l: &Layout) -> f32 {
        let dims = self.pdf.pages().get(self.page).map_or((612.0, 792.0), |p| p.render_dimensions());
        zoom::scale(self.zoom, dims, (l.aw.saturating_sub(2 * MARGIN), l.ah.saturating_sub(2 * MARGIN)))
    }

    /// Renders the current page at `scale`, unless it already is.
    fn render(&mut self, scale: f32) {
        if self.shown.as_ref().is_some_and(|s| s.page == self.page && (s.scale - scale).abs() < 1e-4) { return; }
        let started = Instant::now();
        let Some((pixels, w, h)) = render_page(&self.pdf, self.page, scale) else { return };
        let ink = pixels.iter().filter(|p| **p != 0x00ffffff).count();
        sex_std::record(&format!("[pdf.page] page={} pages={} width={w} height={h} ink={ink} ms={}",
            self.page + 1, self.pages(), started.elapsed().as_millis()));
        self.shown = Some(Rendered { pixels, w, h, page: self.page, scale });
    }

    fn texts(&mut self) -> &[PageText] {
        if self.texts.is_none() {
            let started = Instant::now();
            let texts: Vec<PageText> = self.pdf.pages().iter().map(|p| text::page_text(p)).collect();
            sex_std::record(&format!("[pdf.text] pages={} chars={} ms={}", texts.len(),
                texts.iter().map(|t| t.chars.len()).sum::<usize>(), started.elapsed().as_millis()));
            self.texts = Some(texts);
        }
        self.texts.as_deref().unwrap_or(&[])
    }

    /// Enter or F3 in the bar: the next hit (or the one before), turning to its page.
    fn find(&mut self, back: bool) {
        if self.term.is_empty() { return; }
        if self.term != self.searched {
            let term = self.term.clone();
            let pages: Vec<Vec<char>> = self.texts().iter().map(|t| t.chars.clone()).collect();
            self.hits = search::hits(&pages, &term);
            self.searched = term;
            self.hit = None;
        }
        let hit = if back { search::previous(&self.hits, self.hit, self.page) } else { search::next(&self.hits, self.hit, self.page) };
        self.found_none = hit.is_none();
        let Some(hit) = hit else {
            sex_std::record(&format!("[pdf.find] term={} hits=0", self.searched));
            return;
        };
        let index = self.hits.iter().position(|h| *h == hit).unwrap_or(0);
        sex_std::record(&format!("[pdf.find] term={} page={} hits={} index={}", self.searched, hit.page + 1, self.hits.len(), index + 1));
        self.hit = Some(hit);
        self.go(hit.page);
        self.reveal = self.texts.as_ref().and_then(|t| t[hit.page].boxes.get(hit.at)).map(|b| b.y0 as f32);
    }

    fn go(&mut self, page: usize) {
        let page = page.min(self.pages().saturating_sub(1));
        if page != self.page { self.page = page; self.scroll = 0; self.hscroll = 0; }
    }

    fn set_zoom(&mut self, zoom: Zoom, width: f32) {
        self.zoom = zoom;
        let (mode, scale) = match zoom {
            Zoom::FitWidth => ("width", width),
            Zoom::FitPage => ("page", width),
            Zoom::Scale(s) => ("fixed", s),
        };
        sex_std::record(&format!("[pdf.zoom] mode={mode} pct={}", zoom::percent(scale)));
    }

    fn draw(&mut self, window: &mut sex_app::Window) {
        let size = window.size();
        let out = window.pixels();
        let (width, height) = (size.0.min(out.len() / size.1.max(1)), size.1);
        if width == 0 || height == 0 { return; }
        let l = self.layout((width, height));
        let scale = self.scale(&l);
        self.render(scale);
        out[..width * height].fill(BACKGROUND);
        if let Some(r) = &self.shown {
            let max_scroll = (r.h + 2 * MARGIN).saturating_sub(l.ah);
            if let Some(y) = self.reveal.take() { self.scroll = zoom::scroll_to(y, r.scale, r.h, l.ah, MARGIN); }
            self.scroll = self.scroll.min(max_scroll);
            self.hscroll = self.hscroll.min((r.w + 2 * MARGIN).saturating_sub(l.aw));
            let left = l.x0 + if r.w + 2 * MARGIN <= l.aw { (l.aw - r.w) / 2 } else { MARGIN };
            let hits: Vec<(Hit, bool)> = self.hits.iter().filter(|h| h.page == r.page && self.finding).map(|h| (*h, Some(*h) == self.hit)).collect();
            let boxes = self.texts.as_ref().map(|t| &t[r.page].boxes);
            let len = self.searched.chars().count();
            for y in 0..l.ah {
                let py = (y + self.scroll).wrapping_sub(MARGIN);
                if py >= r.h { continue; }
                let row = &mut out[(l.y0 + y) * width..(l.y0 + y + 1) * width];
                let from = &r.pixels[py * r.w..py * r.w + r.w];
                let skip = self.hscroll.saturating_sub(MARGIN).min(r.w);
                let n = (r.w - skip).min(width - left);
                row[left..left + n].copy_from_slice(&from[skip..skip + n]);
                // Light up the hits on this row: each character's box, scaled.
                if let Some(boxes) = boxes {
                    for (hit, now) in &hits {
                        for b in boxes.iter().skip(hit.at).take(len) {
                            let (top, bottom) = ((b.y0 * r.scale as f64) as usize, (b.y1 * r.scale as f64) as usize);
                            if py < top || py >= bottom { continue; }
                            let (x0, x1) = ((b.x0 * r.scale as f64) as usize, (b.x1 * r.scale as f64) as usize);
                            for x in x0.max(skip)..x1.min(skip + n) {
                                let p = &mut row[left + x - skip];
                                *p = multiply(*p, if *now { HIT_NOW } else { HIT_OTHER });
                            }
                        }
                    }
                }
            }
        }
        if l.panel > 0 { self.draw_panel(out, width, height); }
        if l.bar > 0 { self.draw_bar(out, width, l.bar); }
        window.show_pixels(width, height);
        let short: String = self.name.chars().take(9).collect();
        window.title(format!("{short} {}/{}", self.page + 1, self.pages()).as_bytes());
    }

    fn draw_bar(&mut self, out: &mut [u32], width: usize, bar: usize) {
        fill(out, width, (0, 0, width, bar), BAR_BACK);
        fill(out, width, (0, bar - 1, width, 1), ACCENT);
        let status = if self.term != self.searched || self.term.is_empty() { String::new() }
            else if self.found_none { "not found".into() }
            else { self.hit.and_then(|h| self.hits.iter().position(|x| *x == h)).map(|i| format!("{} of {}", i + 1, self.hits.len())).unwrap_or_default() };
        let y = (bar.saturating_sub(CH)) / 2;
        let Some(letters) = self.letters.as_mut() else { return };
        letters.draw(out, width, (8, y), width, "Find:", DIM);
        letters.draw(out, width, (8 + 6 * CW, y), width.saturating_sub(16 * CW), &format!("{}_", self.term), INK);
        let sx = width.saturating_sub(8 + status.len() * CW);
        letters.draw(out, width, (sx, y), width, &status, if self.found_none { 0x00ff8a7a } else { DIM });
    }

    fn draw_panel(&mut self, out: &mut [u32], width: usize, height: usize) {
        fill(out, width, (0, 0, PANEL, height), PANEL_BACK);
        match self.panel {
            Panel::Thumbs => self.draw_thumbs(out, width, height),
            Panel::Outline => self.draw_outline(out, width, height),
            Panel::None => {}
        }
    }

    /// One slot of the strip: the thumbnail, its border and its number below it.
    fn thumb_slot(&self) -> usize {
        let (pw, ph) = self.pdf.pages().first().map_or((612.0, 792.0), |p| p.render_dimensions());
        ((PANEL - 24) as f32 * ph / pw.max(1.0)) as usize + 8 + CH + 8
    }

    fn draw_thumbs(&mut self, out: &mut [u32], width: usize, height: usize) {
        let top = if self.finding { BAR } else { 0 };
        let slot = self.thumb_slot().max(1);
        let fits = ((height - top) / slot).max(1);
        if self.page < self.thumb_top { self.thumb_top = self.page; }
        if self.page >= self.thumb_top + fits { self.thumb_top = self.page + 1 - fits; }
        let mut rendered = 0;
        for i in self.thumb_top..(self.thumb_top + fits + 1).min(self.pages()) {
            if self.thumbs[i].is_none() {
                let pw = self.pdf.pages()[i].render_dimensions().0.max(1.0);
                self.thumbs[i] = render_page(&self.pdf, i, (PANEL - 24) as f32 / pw);
                rendered += 1;
            }
            let y = top + (i - self.thumb_top) * slot + 8;
            let Some((pixels, w, h)) = &self.thumbs[i] else { continue };
            let x = (PANEL - w) / 2;
            if i == self.page { fill(out, width, (x - 3, y.saturating_sub(3), w + 6, (*h + 6).min(height.saturating_sub(y.saturating_sub(3)))), ACCENT); }
            for row in 0..*h {
                if y + row >= height { break; }
                out[(y + row) * width + x..(y + row) * width + x + w].copy_from_slice(&pixels[row * w..row * w + w]);
            }
            let label = format!("{}", i + 1);
            if y + h + 4 + CH <= height {
                let lx = (PANEL - label.len() * CW) / 2;
                if let Some(letters) = self.letters.as_mut() { letters.draw(out, width, (lx, y + h + 4), PANEL, &label, if i == self.page { INK } else { DIM }); }
            }
        }
        if rendered > 0 {
            sex_std::record(&format!("[pdf.thumbs] rendered={rendered} cached={} first={}", self.thumbs.iter().filter(|t| t.is_some()).count(), self.thumb_top + 1));
        }
    }

    fn draw_outline(&mut self, out: &mut [u32], width: usize, height: usize) {
        let top = if self.finding { BAR } else { 0 } + 6;
        let entries = self.outline.clone().unwrap_or_default();
        let Some(letters) = self.letters.as_mut() else { return };
        if entries.is_empty() {
            letters.draw(out, width, (8, top), PANEL - 4, "No outline", DIM);
            return;
        }
        let fits = ((height - top) / ROW).max(1);
        let first = self.chosen.saturating_sub(fits - 1);
        for (row, (i, e)) in entries.iter().enumerate().skip(first).take(fits).enumerate() {
            let y = top + row * ROW;
            if i == self.chosen { fill(out, width, (0, y, PANEL, ROW), 0x00384a66); }
            let indent = 8 + e.depth.min(4) * CW;
            let color = if e.page == Some(self.page) { INK } else { DIM };
            letters.draw(out, width, (indent, y + (ROW - CH) / 2), PANEL - 4, &e.title, color);
        }
    }

    /// The outline entry for a page: the last one at or before it.
    fn entry_for_page(&self) -> usize {
        self.outline.as_ref().and_then(|o| o.iter().rposition(|e| e.page.is_some_and(|p| p <= self.page))).unwrap_or(0)
    }

    fn toggle(&mut self, panel: Panel) {
        self.panel = if self.panel == panel { Panel::None } else { panel };
        match self.panel {
            Panel::Thumbs => sex_std::record(&format!("[pdf.thumbs] shown=1 pages={}", self.pages())),
            Panel::Outline => {
                if self.outline.is_none() { self.outline = Some(outline::read(&self.pdf)); }
                self.chosen = self.entry_for_page();
                let entries = self.outline.as_ref().map_or(0, |o| o.len());
                sex_std::record(&format!("[pdf.outline] shown=1 entries={entries}"));
            }
            Panel::None => sex_std::record("[pdf.panel] shown=0"),
        }
    }

    /// A key; true when something changed and the window should be drawn again.
    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key, size: (usize, usize)) -> bool {
        let before = (self.page, self.zoom, self.scroll, self.hscroll, self.panel, self.chosen, self.finding, self.term.clone(), self.hit);
        let l = self.layout(size);
        let fit_width = zoom::scale(Zoom::FitWidth, self.pdf.pages().get(self.page).map_or((612.0, 792.0), |p| p.render_dimensions()), (l.aw.saturating_sub(2 * MARGIN), l.ah.saturating_sub(2 * MARGIN)));
        let now = self.scale(&l);
        let last = self.pages().saturating_sub(1);
        let step = size.1 * 3 / 4;
        if key.control && key.code == KEY_F {
            self.finding = true;
        } else if key.code == F3 || key.control && key.code == KEY_G {
            self.finding = true;
            self.find(key.shift);
        } else if self.finding && key.code == ESCAPE {
            self.finding = false;
            self.hit = None;
        } else if self.finding && key.code == ENTER {
            self.find(key.shift);
        } else if self.finding && key.code == BACKSPACE {
            self.term.pop();
        } else if let (true, Some(c), None) = (self.finding, sex_app::typed(key), sex_app::nav(key)) {
            if self.term.len() < 64 { self.term.push(c as char); }
        } else if self.panel == Panel::Outline && matches!(sex_app::nav(key), Some(Nav::Up | Nav::Down)) || self.panel == Panel::Outline && key.code == ENTER {
            let n = self.outline.as_ref().map_or(0, |o| o.len());
            match sex_app::nav(key) {
                Some(Nav::Up) => self.chosen = self.chosen.saturating_sub(1),
                Some(Nav::Down) => self.chosen = (self.chosen + 1).min(n.saturating_sub(1)),
                _ => if let Some(e) = self.outline.as_ref().and_then(|o| o.get(self.chosen)).cloned() {
                    if let Some(p) = e.page {
                        sex_std::record(&format!("[pdf.goto] page={} via=outline title={}", p + 1, e.title));
                        self.go(p);
                    }
                },
            }
        } else {
            match (sex_app::nav(key), sex_app::typed(key), key.code) {
                (Some(Nav::Right), ..) if key.shift => self.hscroll += size.0 / 4,
                (Some(Nav::Left), ..) if key.shift => self.hscroll = self.hscroll.saturating_sub(size.0 / 4),
                (Some(Nav::Right), ..) | (_, Some(b' ' | b'n'), _) | (_, _, 0x51 | 0x6D) => self.go(self.page + 1),
                (Some(Nav::Left), ..) | (_, Some(b'p'), _) | (_, _, 0x49 | 0x68) => self.go(self.page.saturating_sub(1)),
                (Some(Nav::Home), ..) => self.go(0),
                (Some(Nav::End), ..) => self.go(last),
                (Some(Nav::Down), ..) => self.scroll += step,
                (Some(Nav::Up), ..) => self.scroll = self.scroll.saturating_sub(step),
                (_, Some(b'+' | b'='), _) => self.set_zoom(zoom::zoom_in(now), fit_width),
                (_, Some(b'-'), _) => self.set_zoom(zoom::zoom_out(now), fit_width),
                (_, Some(b'0' | b'w'), _) => self.set_zoom(Zoom::FitWidth, fit_width),
                (_, Some(b'f'), _) => {
                    let dims = self.pdf.pages().get(self.page).map_or((612.0, 792.0), |p| p.render_dimensions());
                    let s = zoom::scale(Zoom::FitPage, dims, (l.aw.saturating_sub(2 * MARGIN), l.ah.saturating_sub(2 * MARGIN)));
                    self.set_zoom(Zoom::FitPage, s);
                }
                (_, Some(b'1'), _) => self.set_zoom(Zoom::Scale(1.0), fit_width),
                (_, Some(b't'), _) => self.toggle(Panel::Thumbs),
                (_, Some(b'o'), _) => self.toggle(Panel::Outline),
                _ => {}
            }
        }
        before != (self.page, self.zoom, self.scroll, self.hscroll, self.panel, self.chosen, self.finding, self.term.clone(), self.hit)
    }

    /// A click in the panel chooses a page; the wheel scrolls the page.
    fn pointer(&mut self, p: sex_std::sex_pdx::app_pointer::Pointer, size: (usize, usize)) -> bool {
        let l = self.layout(size);
        let (x, y) = (p.x.max(0) as usize, p.y.max(0) as usize);
        match p.kind {
            Kind::Wheel { steps } => {
                let by = steps.unsigned_abs() as usize * 48;
                let before = self.scroll;
                self.scroll = if steps > 0 { self.scroll.saturating_sub(by) } else { self.scroll + by };
                before != self.scroll
            }
            Kind::Down { button: 0 } if x < l.panel && y >= l.bar => {
                let before = self.page;
                match self.panel {
                    Panel::Thumbs => self.go(self.thumb_top + (y - l.bar) / self.thumb_slot().max(1)),
                    Panel::Outline => {
                        let fits = ((size.1 - l.bar - 6) / ROW).max(1);
                        let first = self.chosen.saturating_sub(fits - 1);
                        let i = first + (y - l.bar).saturating_sub(6) / ROW;
                        if let Some(e) = self.outline.as_ref().and_then(|o| o.get(i)).cloned() {
                            self.chosen = i;
                            if let Some(p) = e.page { sex_std::record(&format!("[pdf.goto] page={} via=outline title={}", p + 1, e.title)); self.go(p); }
                        }
                    }
                    Panel::None => {}
                }
                before != self.page || self.panel == Panel::Outline
            }
            _ => false,
        }
    }
}

fn open(name: &str) -> Result<Pdf, String> {
    let bytes = std::fs::read(name).map_err(|e| format!("Could not read {name}: {e}"))?;
    Pdf::new(bytes).map_err(|e| format!("{name} is not a PDF this viewer can read ({e:?})"))
}

pub fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| {
        if std::fs::metadata("welcome.pdf").is_err() {
            if let Err(e) = std::fs::write("welcome.pdf", sample::document()) { eprintln!("Could not write welcome.pdf: {e}"); }
        }
        "welcome.pdf".to_string()
    });
    let pdf = match open(&name) {
        Ok(pdf) => pdf,
        Err(message) => {
            // Said in the window as text; closing it ends the program.
            println!("{message}\n\nPut a PDF in this program's folder and run: pdf NAME.pdf");
            std::process::exit(1);
        }
    };
    // A file the person gave (`@/home/x.pdf`) is named by its last part.
    let short = name.rsplit('/').next().unwrap_or(&name).to_string();
    let (target, task) = sex_std::identity();
    sex_std::record(&format!("[pdf.open] target={target} task={task} name={short} pages={}", pdf.pages().len()));
    let window = sex_std::window();
    let pages = pdf.pages().len();
    let mut viewer = Viewer {
        pdf, name: short, page: 0, zoom: Zoom::FitWidth, scroll: 0, hscroll: 0, shown: None,
        panel: Panel::None, thumbs: vec![None; pages], thumb_top: 0, outline: None, chosen: 0,
        finding: false, term: String::new(), searched: String::new(), hits: Vec::new(), hit: None, found_none: false,
        texts: None, reveal: None, letters: Letters::new(),
    };
    viewer.draw(window);
    loop {
        let changed = match window.next() {
            Event::Close => break,
            Event::Resize { .. } => true,
            Event::Key(key) => viewer.key(key, window.size()),
            Event::Pointer(p) => viewer.pointer(p, window.size()),
        };
        if changed { viewer.draw(window); }
    }
    sex_std::record("[pdf.closed]");
    std::process::exit(0);
}
