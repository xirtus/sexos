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
use vellum_edit::{AnnotInfo, AnnotKind, AnnotStyle, Editor, FieldInfo, FieldKind, FieldValue, NewAnnot, NoteIcon, PageInfo};

use crate::marks;
use crate::search::{self, Hit};
use crate::text::{self, PageText};
use crate::zoom::{self, Zoom};
use crate::{outline, sample, thumb};

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
const TAB: u8 = 0x0F;
const KEY_E: u8 = 0x12;
const KEY_R: u8 = 0x13;
const KEY_S: u8 = 0x1F;
const KEY_H: u8 = 0x23;
const KEY_L: u8 = 0x26;
const KEY_N: u8 = 0x31;
/// The chosen region's outline, and a focused form field's.
const REGION: u32 = 0x00e0457b;
/// How far one arrow press moves or sizes the region, in points.
const NUDGE: f64 = 12.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel { None, Thumbs, Outline, Annots }

/// Form filling: the fields the keyboard reaches, which one, and its text so far.
struct Form { fields: Vec<(FieldInfo, usize, [f64; 4])>, at: usize, text: String }

/// Annotations Ctrl+L lists: markup, not form widgets, links or popups.
fn listed(a: &AnnotInfo) -> bool { !matches!(a.subtype.as_str(), "Widget" | "Popup" | "Link") }

fn describe(a: &AnnotInfo) -> String {
    let kind = if a.subtype == "Text" { "Note" } else { a.subtype.as_str() };
    match a.contents.as_deref().filter(|c| !c.is_empty()) {
        Some(c) => format!("{kind}: {c}"),
        None => kind.to_string(),
    }
}

fn now_date() -> vellum_edit::PdfDate {
    marks::date(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}

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

/// A two-pixel frame around `(x0, y0, x1, y1)`, cut to `clip` (left, top, right, bottom).
fn outline(out: &mut [u32], stride: usize, (x0, y0, x1, y1): (isize, isize, isize, isize), clip: (isize, isize, isize, isize), color: u32) {
    let mut dot = |x: isize, y: isize| {
        if x >= clip.0 && x < clip.2 && y >= clip.1 && y < clip.3 {
            if let Some(p) = out.get_mut(y as usize * stride + x as usize) { *p = color; }
        }
    };
    for t in 0..2 {
        for x in x0..=x1 { dot(x, y0 + t); dot(x, y1 - t); }
        for y in y0..=y1 { dot(x0 + t, y); dot(x1 - t, y); }
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
    /// The file as given, where Ctrl+S writes.
    path: String,
    /// The editing engine over the file's bytes, or why this file cannot be edited.
    editor: Result<Editor, String>,
    /// The region Ctrl+R chose: shown points `[left, top, right, bottom]` on the page.
    region: Option<[f64; 4]>,
    /// A sticky note being typed.
    note: Option<String>,
    form: Option<Form>,
    /// The current page's annotations, while Ctrl+L lists them.
    annots: Vec<AnnotInfo>,
    /// A line of news in the bar until the next key: saved, refused, ...
    status: Option<String>,
}

/// Where each part of the window is.
struct Layout { panel: usize, bar: usize, x0: usize, y0: usize, aw: usize, ah: usize }

impl Viewer {
    fn pages(&self) -> usize { self.pdf.pages().len() }

    fn layout(&self, (width, height): (usize, usize)) -> Layout {
        let panel = if self.panel == Panel::None || width < PANEL * 2 { 0 } else { PANEL };
        let bar = if self.bar_shown() { BAR.min(height / 2) } else { 0 };
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
            // The chosen region and the focused field, outlined where they sit on the page.
            let skip = self.hscroll.saturating_sub(MARGIN).min(r.w) as f64;
            let to_screen = |s: [f64; 4]| {
                let k = r.scale as f64;
                let x = |v: f64| (left as f64 + v * k - skip) as isize;
                let y = |v: f64| (l.y0 as f64 + MARGIN as f64 + v * k - self.scroll as f64) as isize;
                (x(s[0]), y(s[1]), x(s[2]), y(s[3]))
            };
            let clip = (l.x0 as isize, l.y0 as isize, width as isize, height as isize);
            if let Some(region) = self.region { outline(out, width, to_screen(region), clip, REGION); }
            if let (Some(form), Ok(editor)) = (&self.form, &self.editor) {
                if let Some((_, page, rect)) = form.fields.get(form.at).filter(|f| f.1 == r.page) {
                    if let Ok(info) = editor.page_info(*page) { outline(out, width, to_screen(marks::rect_to_shown(&info, *rect)), clip, REGION); }
                }
            }
        }
        if l.panel > 0 { self.draw_panel(out, width, height); }
        if l.bar > 0 { self.draw_bar(out, width, l.bar); }
        window.show_pixels(width, height);
        let short: String = self.name.chars().take(9).collect();
        let changed = if self.editor.as_ref().is_ok_and(|e| e.is_modified()) { "*" } else { "" };
        window.title(format!("{short}{changed} {}/{}", self.page + 1, self.pages()).as_bytes());
    }

    fn bar_shown(&self) -> bool {
        self.finding || self.note.is_some() || self.form.is_some() || self.region.is_some() || self.status.is_some()
    }

    fn draw_bar(&mut self, out: &mut [u32], width: usize, bar: usize) {
        fill(out, width, (0, 0, width, bar), BAR_BACK);
        fill(out, width, (0, bar - 1, width, 1), ACCENT);
        let y = (bar.saturating_sub(CH)) / 2;
        // Editing has the bar before find: what is typed goes where the bar says.
        let editing = if let Some(note) = &self.note {
            Some(("Note:".to_string(), format!("{note}_"), "Enter adds it, Escape cancels".to_string()))
        } else if let Some(form) = &self.form {
            form.fields.get(form.at).map(|(f, _, _)| {
                let value = match f.kind {
                    FieldKind::Checkbox => if marks::toggled(f) == "Off" { "[x]  Space unticks".into() } else { "[ ]  Space ticks".into() },
                    _ => format!("{}_", form.text),
                };
                (format!("{}:", f.name), value, format!("field {} of {}, Tab next, Escape ends", form.at + 1, form.fields.len()))
            })
        } else if self.region.is_some() && !self.finding {
            Some(("Region:".into(), "arrows move, Shift+arrows size".into(), "Ctrl+H or Enter highlights, Ctrl+N notes".into()))
        } else if !self.finding {
            Some((String::new(), String::new(), self.status.clone().unwrap_or_default()))
        } else { None };
        if let Some((label, value, right)) = editing {
            let right = self.status.clone().unwrap_or(right);
            let Some(letters) = self.letters.as_mut() else { return };
            letters.draw(out, width, (8, y), width, &label, DIM);
            let vx = 8 + (label.chars().count() + 1) * CW;
            letters.draw(out, width, (vx, y), width.saturating_sub((right.len() + 2) * CW), &value, INK);
            let sx = width.saturating_sub(8 + right.len() * CW);
            letters.draw(out, width, (sx, y), width, &right, DIM);
            return;
        }
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
            Panel::Annots => self.draw_annots(out, width, height),
            Panel::None => {}
        }
    }

    fn draw_annots(&mut self, out: &mut [u32], width: usize, height: usize) {
        let top = if self.bar_shown() { BAR } else { 0 } + 6;
        let rows: Vec<String> = self.annots.iter().enumerate().map(|(i, a)| format!("{} {}", i + 1, describe(a))).collect();
        let Some(letters) = self.letters.as_mut() else { return };
        if rows.is_empty() {
            letters.draw(out, width, (8, top), PANEL - 4, "No annotations", DIM);
            return;
        }
        let fits = ((height - top) / ROW).max(1);
        let first = self.chosen.saturating_sub(fits - 1);
        for (row, (i, text)) in rows.iter().enumerate().skip(first).take(fits).enumerate() {
            let y = top + row * ROW;
            if i == self.chosen { fill(out, width, (0, y, PANEL, ROW), 0x00384a66); }
            letters.draw(out, width, (8, y + (ROW - CH) / 2), PANEL - 4, text, if i == self.chosen { INK } else { DIM });
        }
    }

    /// One slot of the strip: the thumbnail, its border and its number below it.
    fn thumb_slot(&self) -> usize {
        let (pw, ph) = self.pdf.pages().first().map_or((612.0, 792.0), |p| p.render_dimensions());
        ((PANEL - 24) as f32 * ph / pw.max(1.0)) as usize + 8 + CH + 8
    }

    fn draw_thumbs(&mut self, out: &mut [u32], width: usize, height: usize) {
        let top = if self.bar_shown() { BAR } else { 0 };
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
        let top = if self.bar_shown() { BAR } else { 0 } + 6;
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
            Panel::None | Panel::Annots => sex_std::record("[pdf.panel] shown=0"),
        }
    }

    /// A key; true when something changed and the window should be drawn again.
    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key, size: (usize, usize)) -> bool {
        let had_status = self.status.take().is_some();
        if self.edit_key(key) { return true; }
        self.view_key(key, size) || had_status
    }

    fn view_key(&mut self, key: sex_std::sex_pdx::app_key::Key, size: (usize, usize)) -> bool {
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
                    Panel::Annots => {
                        let fits = ((size.1 - l.bar - 6) / ROW).max(1);
                        let first = self.chosen.saturating_sub(fits - 1);
                        self.chosen = (first + (y - l.bar).saturating_sub(6) / ROW).min(self.annots.len().saturating_sub(1));
                    }
                    Panel::None => {}
                }
                before != self.page || matches!(self.panel, Panel::Outline | Panel::Annots)
            }
            _ => false,
        }
    }
}

/// Annotating, form filling and saving.
impl Viewer {
    /// An editing key; true when it was one (and the window should be drawn again).
    fn edit_key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        let ctrl = |code: u8| key.control && key.code == code;
        if ctrl(KEY_S) { self.save(); return true; }
        if ctrl(KEY_H) { self.highlight(); return true; }
        if ctrl(KEY_R) {
            self.region = match self.region { Some(_) => None, None => Some(self.first_region()) };
            sex_std::record(&format!("[pdf.region] shown={}", self.region.is_some() as u8));
            return true;
        }
        if ctrl(KEY_N) {
            if self.editor().is_some() { self.note = Some(String::new()); }
            return true;
        }
        if ctrl(KEY_L) {
            if self.panel == Panel::Annots { self.panel = Panel::None; } else { self.panel = Panel::Annots; self.chosen = 0; self.load_annots(true); }
            return true;
        }
        if ctrl(KEY_E) {
            if self.form.is_some() { self.end_form(); } else { self.start_form(); }
            return true;
        }
        if self.note.is_some() { self.note_key(key); return true; }
        if self.form.is_some() && self.form_key(key) { return true; }
        if self.region.is_some() && !self.finding && self.region_key(key) { return true; }
        if self.panel == Panel::Annots && self.annots_key(key) { return true; }
        false
    }

    /// The editing engine, or `None` with the reason in the bar.
    fn editor(&mut self) -> Option<&mut Editor> {
        if let Err(why) = &self.editor {
            self.status = Some(format!("This file cannot be edited: {why}"));
            sex_std::record(&format!("[pdf.edit.refused] {why}"));
            return None;
        }
        self.editor.as_mut().ok()
    }

    fn page_info(&self, page: usize) -> Option<PageInfo> { self.editor.as_ref().ok()?.page_info(page).ok() }

    /// Shows the edits so far: the page drawn again from the bytes a save would write.
    fn refresh(&mut self) {
        let Ok(editor) = &self.editor else { return };
        match editor.save_incremental().map_err(|e| e.to_string()).and_then(|b| Pdf::new(b).map_err(|e| format!("{e:?}"))) {
            Ok(pdf) => {
                self.pdf = pdf;
                self.shown = None;
                self.thumbs = vec![None; self.pages()];
            }
            Err(e) => self.status = Some(format!("Could not show the change: {e}")),
        }
    }

    /// Markup annotations in the whole document.
    fn annotation_count(&self) -> usize {
        let Ok(editor) = &self.editor else { return 0 };
        (0..editor.page_count()).map(|p| editor.annotations(p).map_or(0, |a| a.iter().filter(|a| listed(a)).count())).sum()
    }

    /// A region to start from: the middle of the page's upper part.
    fn first_region(&self) -> [f64; 4] {
        let (w, h) = self.pdf.pages().get(self.page).map_or((612.0, 792.0), |p| p.render_dimensions());
        let (w, h) = (w as f64, h as f64);
        [w * 0.2, h * 0.15, w * 0.6, h * 0.15 + 48.0]
    }

    fn region_key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        let Some(mut r) = self.region else { return false };
        let (w, h) = self.pdf.pages().get(self.page).map_or((612.0, 792.0), |p| p.render_dimensions());
        let (w, h) = (w as f64, h as f64);
        let (dx, dy) = match sex_app::nav(key) {
            Some(Nav::Left) => (-NUDGE, 0.0),
            Some(Nav::Right) => (NUDGE, 0.0),
            Some(Nav::Up) => (0.0, -NUDGE),
            Some(Nav::Down) => (0.0, NUDGE),
            _ => {
                match key.code {
                    ENTER => self.highlight(),
                    ESCAPE => { self.region = None; sex_std::record("[pdf.region] shown=0"); }
                    _ => return false,
                }
                return true;
            }
        };
        if key.shift {
            // Shift sizes it from its bottom right corner.
            r[2] = (r[2] + dx).clamp(r[0] + 6.0, w);
            r[3] = (r[3] + dy).clamp(r[1] + 6.0, h);
        } else {
            let (dx, dy) = (dx.clamp(-r[0], w - r[2]), dy.clamp(-r[1], h - r[3]));
            r = [r[0] + dx, r[1] + dy, r[2] + dx, r[3] + dy];
        }
        self.region = Some(r);
        sex_std::record(&format!("[pdf.region] shown=1 x={:.0} y={:.0} w={:.0} h={:.0}", r[0], r[1], r[2] - r[0], r[3] - r[1]));
        true
    }

    /// Ctrl+H: the chosen region, else the word found, highlighted.
    fn highlight(&mut self) {
        let (page, rects, from) = if let Some(r) = self.region {
            (self.page, vec![r], "region")
        } else if let (Some(hit), Some(texts)) = (self.hit, &self.texts) {
            let len = self.searched.chars().count();
            let boxes: Vec<[f64; 4]> = texts[hit.page].boxes.iter().skip(hit.at).take(len).map(|b| [b.x0, b.y0, b.x1, b.y1]).collect();
            (hit.page, marks::line_rects(&boxes), "find")
        } else {
            self.status = Some("Find a word (Ctrl+F) or choose a region (Ctrl+R) first".into());
            return;
        };
        let Some(info) = self.page_info(page) else { let _ = self.editor(); return };
        let rects: Vec<[f64; 4]> = rects.iter().map(|r| marks::rect_to_user(&info, *r)).collect();
        let n = rects.len();
        let annot = NewAnnot { kind: AnnotKind::Highlight { rects }, style: AnnotStyle { date: Some(now_date()), ..AnnotStyle::default() } };
        let Some(editor) = self.editor() else { return };
        match editor.add_annotation(page, &annot) {
            Ok(_) => {
                self.region = None;
                self.refresh();
                self.status = Some("Highlighted. Ctrl+S saves.".into());
                sex_std::record(&format!("[pdf.annotate] kind=highlight page={} rects={n} from={from} annots={}", page + 1, self.annotation_count()));
            }
            Err(e) => self.status = Some(format!("Could not highlight: {e}")),
        }
    }

    fn note_key(&mut self, key: sex_std::sex_pdx::app_key::Key) {
        let Some(text) = self.note.as_mut() else { return };
        match key.code {
            ESCAPE => self.note = None,
            ENTER => { let text = self.note.take().unwrap_or_default(); self.add_note(text); }
            BACKSPACE => { text.pop(); }
            _ => if let Some(c) = sex_app::text(key).filter(|c| !c.is_control() && !key.control) {
                if text.chars().count() < 400 { text.push(c); }
            },
        }
    }

    /// A sticky note at the region's top left, else beside the word found, else near the
    /// page's top left.
    fn add_note(&mut self, text: String) {
        let (page, at) = if let Some(r) = self.region {
            (self.page, (r[0], r[1]))
        } else if let (Some(hit), Some(texts)) = (self.hit, &self.texts) {
            let len = self.searched.chars().count().max(1);
            let b = texts[hit.page].boxes.get(hit.at + len - 1).copied().unwrap_or_default();
            (hit.page, (b.x1 + 2.0, (b.y0 - 4.0).max(0.0)))
        } else {
            (self.page, (36.0, 36.0))
        };
        let Some(info) = self.page_info(page) else { let _ = self.editor(); return };
        let at = marks::to_user(&info, at);
        let chars = text.chars().count();
        let annot = NewAnnot {
            kind: AnnotKind::Note { at, icon: NoteIcon::Comment },
            style: AnnotStyle { contents: Some(text), date: Some(now_date()), ..AnnotStyle::default() },
        };
        let Some(editor) = self.editor() else { return };
        match editor.add_annotation(page, &annot) {
            Ok(_) => {
                self.region = None;
                self.refresh();
                self.status = Some("Note added. Ctrl+S saves.".into());
                sex_std::record(&format!("[pdf.annotate] kind=note page={} chars={chars} annots={}", page + 1, self.annotation_count()));
            }
            Err(e) => self.status = Some(format!("Could not add the note: {e}")),
        }
    }

    /// The current page's annotations for the Ctrl+L list.
    fn load_annots(&mut self, record: bool) {
        let page = self.page;
        let Some(editor) = self.editor() else { self.annots.clear(); return };
        self.annots = editor.annotations(page).unwrap_or_default().into_iter().filter(listed).collect();
        self.chosen = self.chosen.min(self.annots.len().saturating_sub(1));
        if record {
            let names: Vec<String> = self.annots.iter().map(|a| a.subtype.clone()).collect();
            sex_std::record(&format!("[pdf.annots] page={} count={} kinds={}", page + 1, self.annots.len(), names.join(",")));
        }
    }

    fn annots_key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        match (sex_app::nav(key), key.code) {
            (Some(Nav::Up), _) => self.chosen = self.chosen.saturating_sub(1),
            (Some(Nav::Down), _) => self.chosen = (self.chosen + 1).min(self.annots.len().saturating_sub(1)),
            (_, ESCAPE) => self.panel = Panel::None,
            (Some(Nav::Delete), _) | (_, BACKSPACE) => {
                let Some(a) = self.annots.get(self.chosen).cloned() else { return true };
                let page = self.page;
                let Some(editor) = self.editor() else { return true };
                match editor.delete_annotation(page, a.index) {
                    Ok(()) => {
                        self.refresh();
                        self.load_annots(false);
                        sex_std::record(&format!("[pdf.annotate] kind=delete page={} was={} annots={}", page + 1, a.subtype, self.annotation_count()));
                        self.status = Some(format!("Deleted {}. Ctrl+S saves.", describe(&a)));
                    }
                    Err(e) => self.status = Some(format!("Could not delete it: {e}")),
                }
            }
            _ => return false,
        }
        true
    }

    fn start_form(&mut self) {
        let Some(editor) = self.editor() else { return };
        let fields = marks::fillable(&editor.fields().unwrap_or_default());
        if fields.is_empty() {
            self.status = Some("This document has no form fields to fill".into());
            sex_std::record("[pdf.form] fields=0");
            return;
        }
        sex_std::record(&format!("[pdf.form] fields={} first={}", fields.len(), fields[0].0.name));
        self.form = Some(Form { fields, at: 0, text: String::new() });
        self.focus(0);
    }

    /// Moves to field `at`: its page, scrolled to it, its text ready to edit.
    fn focus(&mut self, at: usize) {
        let Some(form) = self.form.as_mut() else { return };
        form.at = at;
        let Some((f, page, rect)) = form.fields.get(at).cloned() else { return };
        form.text = match &f.value { FieldValue::Text(s) => s.clone(), _ => String::new() };
        self.go(page);
        if let Some(info) = self.page_info(page) { self.reveal = Some(marks::rect_to_shown(&info, rect)[1] as f32); }
        sex_std::record(&format!("[pdf.field.focus] name={} index={} page={}", f.name, at + 1, page + 1));
    }

    /// Sets the focused field to what was typed, if that changed it.
    fn commit_field(&mut self) {
        let Some(form) = self.form.as_ref() else { return };
        let Some((f, _, _)) = form.fields.get(form.at) else { return };
        if !matches!(f.kind, FieldKind::Text { .. }) { return; }
        let now = match &f.value { FieldValue::Text(s) => s.as_str(), _ => "" };
        if now == form.text { return; }
        let (name, text) = (f.name.clone(), form.text.clone());
        self.set_field(&name, FieldValue::Text(text.clone()), "text", &text);
    }

    fn set_field(&mut self, name: &str, value: FieldValue, kind: &str, shown: &str) {
        let Some(editor) = self.editor() else { return };
        match editor.set_field(name, value) {
            Ok(()) => {
                let fields = marks::fillable(&editor.fields().unwrap_or_default());
                if let Some(form) = self.form.as_mut() { form.fields = fields; }
                self.refresh();
                sex_std::record(&format!("[pdf.field] name={name} kind={kind} value={shown}"));
            }
            Err(e) => self.status = Some(format!("Could not fill {name}: {e}")),
        }
    }

    fn end_form(&mut self) {
        self.commit_field();
        self.form = None;
        sex_std::record("[pdf.form] done=1");
    }

    fn form_key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        let Some(form) = self.form.as_mut() else { return false };
        let n = form.fields.len().max(1);
        let checkbox = form.fields.get(form.at).is_some_and(|f| f.0.kind == FieldKind::Checkbox);
        let step = match (sex_app::nav(key), key.code) {
            (_, TAB) => Some(if key.shift { n - 1 } else { 1 }),
            (Some(Nav::Down), _) if key.control => Some(1),
            (Some(Nav::Up), _) if key.control => Some(n - 1),
            (_, ENTER) => Some(1),
            _ => None,
        };
        if let Some(step) = step {
            let next = (form.at + step) % n;
            self.commit_field();
            self.focus(next);
            return true;
        }
        match key.code {
            ESCAPE => { self.end_form(); return true; }
            BACKSPACE if !checkbox => { form.text.pop(); return true; }
            _ => {}
        }
        let Some(c) = sex_app::text(key).filter(|c| !c.is_control() && !key.control) else { return false };
        if checkbox {
            if c == ' ' {
                let Some((f, _, _)) = form.fields.get(form.at).cloned() else { return true };
                let to = marks::toggled(&f);
                self.set_field(&f.name, FieldValue::State(to.clone()), "checkbox", &to);
            }
        } else if form.text.chars().count() < 200 {
            form.text.push(c);
        }
        true
    }

    /// Ctrl+S: every change appended to the file as one incremental update, then the
    /// file read back and shown as it now is.
    fn save(&mut self) {
        let started = Instant::now();
        self.commit_field();
        let path = self.path.clone();
        let Some(editor) = self.editor() else { return };
        if !editor.is_modified() {
            self.status = Some("Nothing to save".into());
            sex_std::record("[pdf.saved] changed=0");
            return;
        }
        let original = editor.original().to_vec();
        let bytes = match editor.save_incremental() {
            Ok(b) => b,
            Err(e) => { self.status = Some(format!("Could not save: {e}")); sex_std::record(&format!("[pdf.save.failed] {e}")); return; }
        };
        if let Err(e) = std::fs::write(&path, &bytes) {
            self.status = Some(format!("Could not save: {e}"));
            sex_std::record(&format!("[pdf.save.failed] {e}"));
            return;
        }
        let _ = sex_std::sex_app::files::direct::sync();
        // Show the file as it is on the disk now, not as it was meant to be.
        let back = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => { self.status = Some(format!("Saved, but could not read it back: {e}")); return; }
        };
        let pdf = match Pdf::new(back.clone()) {
            Ok(p) => p,
            Err(e) => { self.status = Some(format!("Saved, but it does not open: {e:?}")); sex_std::record("[pdf.save.failed] reopen"); return; }
        };
        self.editor = Editor::open(back.clone()).map_err(|e| e.to_string());
        self.pdf = pdf;
        self.shown = None;
        self.thumbs = vec![None; self.pages()];
        self.texts = None;
        if self.panel == Panel::Annots { self.load_annots(false); }
        if let (Some(form), Ok(editor)) = (self.form.as_mut(), &self.editor) { form.fields = marks::fillable(&editor.fields().unwrap_or_default()); }
        let filled = self.editor.as_ref().ok().and_then(|e| e.fields().ok()).map_or(0, |fs| fs.iter().filter(|f| match &f.value {
            FieldValue::Text(s) => !s.is_empty(),
            FieldValue::State(s) => s != "Off",
            FieldValue::Choices(c) => !c.is_empty(),
            FieldValue::Empty => false,
        }).count());
        let prefix = back.starts_with(&original);
        sex_std::record(&format!("[pdf.saved] path={path} bytes={} original={} prefix={} same={} annots={} filled={filled} ms={}",
            back.len(), original.len(), prefix as u8, (back == bytes) as u8, self.annotation_count(), started.elapsed().as_millis()));
        self.status = Some(format!("Saved {} bytes (an update after the original {})", back.len(), original.len()));
    }
}

/// `--thumb`: the first page on white, fitted, written whole; no window.
fn make_thumb(r: &thumb::Request) -> Result<(usize, usize), String> {
    let started = Instant::now();
    let bytes = std::fs::read(&r.input).map_err(|e| format!("cannot read {}: {e}", r.input))?;
    let pdf = Pdf::new(bytes).map_err(|e| format!("not a PDF this viewer can read ({e:?})"))?;
    let pages = pdf.pages();
    let page = pages.first().ok_or("the PDF has no pages")?;
    let (pw, ph) = page.render_dimensions();
    let (tw, th) = thumb::fit(pw.ceil().max(1.0) as usize, ph.ceil().max(1.0) as usize, r.size);
    let scale = (tw as f32 / pw.max(1.0)).min(th as f32 / ph.max(1.0)).min(1.0);
    let (pixels, w, h) = render_page(&pdf, 0, scale).ok_or("the page could not be drawn")?;
    let image = sex_image::Image { width: w, height: h, pixels: pixels.into_iter().map(|p| p | 0xFF00_0000).collect() };
    // The renderer rounds the page's sides its own way: the fitted size is the promise.
    let small = thumb::resize(image, tw, th).map_err(|e| format!("cannot shrink: {e:?}"))?;
    let out = thumb::encode(&small).ok_or("cannot encode")?;
    thumb::write_whole(&r.output, &out).map_err(|e| format!("cannot write {}: {e}", r.output))?;
    let _ = sex_std::sex_app::files::direct::sync();
    sex_std::record(&format!("[pdf.thumb] status=0 w={} h={} bytes={} ms={} out={}", small.width, small.height, out.len(), started.elapsed().as_millis(), r.output));
    Ok((small.width, small.height))
}

/// The file's bytes, and the PDF they make.
fn open(name: &str) -> Result<(Vec<u8>, Pdf), String> {
    let bytes = std::fs::read(name).map_err(|e| format!("Could not read {name}: {e}"))?;
    let pdf = Pdf::new(bytes.clone()).map_err(|e| format!("{name} is not a PDF this viewer can read ({e:?})"))?;
    Ok((bytes, pdf))
}

pub fn main() {
    let words: Vec<String> = std::env::args().skip(1).collect();
    if let Some(request) = thumb::request(&words) {
        let status = match request.map_err(String::from).and_then(|r| make_thumb(&r)) {
            Ok(_) => 0,
            Err(why) => { sex_std::record(&format!("[pdf.thumb] status=1 {why}")); 1 }
        };
        std::process::exit(status);
    }
    let name = words.first().cloned().unwrap_or_else(|| {
        if std::fs::metadata("welcome.pdf").is_err() {
            if let Err(e) = std::fs::write("welcome.pdf", sample::document()) { eprintln!("Could not write welcome.pdf: {e}"); }
        }
        "welcome.pdf".to_string()
    });
    let (bytes, pdf) = match open(&name) {
        Ok(opened) => opened,
        Err(message) => {
            // Said in the window as text; closing it ends the program.
            println!("{message}\n\nPut a PDF in this program's folder and run: pdf NAME.pdf");
            std::process::exit(1);
        }
    };
    // A file the person gave (`@/home/x.pdf`) is named by its last part.
    let short = name.rsplit('/').next().unwrap_or(&name).to_string();
    let (target, task) = sex_std::identity();
    let editor = Editor::open(bytes).map_err(|e| e.to_string());
    sex_std::record(&format!("[pdf.open] target={target} task={task} name={short} pages={}", pdf.pages().len()));
    if let Err(why) = &editor { sex_std::record(&format!("[pdf.edit.refused] {why}")); }
    let window = sex_std::window();
    let pages = pdf.pages().len();
    let mut viewer = Viewer {
        pdf, name: short, page: 0, zoom: Zoom::FitWidth, scroll: 0, hscroll: 0, shown: None,
        panel: Panel::None, thumbs: vec![None; pages], thumb_top: 0, outline: None, chosen: 0,
        finding: false, term: String::new(), searched: String::new(), hits: Vec::new(), hit: None, found_none: false,
        texts: None, reveal: None, letters: Letters::new(),
        path: name, editor, region: None, note: None, form: None, annots: Vec::new(), status: None,
    };
    viewer.draw(window);
    loop {
        let changed = match window.next() {
            Event::Close => break,
            Event::Resize { .. } => true,
            Event::Key(key) => {
                let changed = viewer.key(key, window.size());
                if viewer.panel == Panel::Annots { viewer.load_annots(false); }
                changed
            }
            Event::Pointer(p) => viewer.pointer(p, window.size()),
        };
        if changed { viewer.draw(window); }
    }
    sex_std::record("[pdf.closed]");
    std::process::exit(0);
}
