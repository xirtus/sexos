// SPDX-License-Identifier: GPL-3.0-only
//! The window, the keys and the rendered page, with the find bar, the thumbnail strip
//! and the outline panel around it.
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx::app_pointer::Kind;
use sex_std::sex_app::pdf_sign;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Instant;
use sex_theme::Tokens;
use vellum_edit::{AnnotInfo, AnnotKind, AnnotStyle, ContentIntegrity, Editor, FieldInfo, FieldKind, FieldValue, NewAnnot, NoteIcon, PageInfo};

use crate::marks;
use crate::history::{AnnotationHistory, FieldHistory};
use crate::search::{self, Hit};
use crate::text::{self, PageText};
use crate::zoom::{self, Zoom};
use crate::{outline, recovery, sample, thumb};

#[derive(Clone, Copy)]
struct Palette {
    background: u32, panel_back: u32, bar_back: u32,
    ink: u32, dim: u32, accent: u32, selected: u32, danger: u32,
}

impl Palette {
    fn from_tokens(t: Tokens) -> Self {
        let rgb = |c: u32| c & 0x00FF_FFFF;
        Self {
            background: rgb(t.window), panel_back: rgb(t.surface), bar_back: rgb(t.field),
            ink: rgb(t.text), dim: rgb(t.text_dim), accent: rgb(t.accent),
            selected: rgb(sex_theme::mix(t.surface, t.accent, 0.25)), danger: rgb(t.danger),
        }
    }
}
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
const CHOICE_MENU_ROWS: usize = 8;
const CHOICE_MENU_WIDTH: usize = 360;
/// Two pages besides the shown page, never more than 48 MiB of retired pixels.
const CACHE_PAGES: usize = 2;
const CACHE_BYTES: usize = 48 << 20;

/// Keyboard codes Shell passes through.
const ESCAPE: u8 = 0x01;
const BACKSPACE: u8 = 0x0E;
const ENTER: u8 = 0x1C;
const KEY_F: u8 = 0x21;
const KEY_G: u8 = 0x22;
const KEY_D: u8 = 0x20;
/// Shell keeps F3 for itself today; it is honoured if it ever arrives.
const F3: u8 = 0x3D;
const TAB: u8 = 0x0F;
const KEY_E: u8 = 0x12;
const KEY_R: u8 = 0x13;
const KEY_S: u8 = 0x1F;
const KEY_H: u8 = 0x23;
const KEY_O: u8 = 0x18;
const HIGHLIGHT_COLORS: [(&str, [f32; 3]); 4] = [
    ("yellow", [1.0, 0.92, 0.0]),
    ("green", [0.45, 0.9, 0.45]),
    ("pink", [1.0, 0.55, 0.75]),
    ("blue", [0.45, 0.7, 1.0]),
];
const HIGHLIGHT_OPACITIES: [(&str, f32); 4] = [
    ("100%", 1.0), ("75%", 0.75), ("50%", 0.5), ("25%", 0.25),
];
const KEY_L: u8 = 0x26;
const KEY_N: u8 = 0x31;
const KEY_Z: u8 = 0x2C;
const KEY_V: u8 = 0x2F;
const KEY_B: u8 = 0x30;
const SLASH: u8 = 0x35;
const SHORTCUTS: &[&str] = &[
    "Pages: Right / Space / N next; Left / P previous",
    "Home / End first / last; Up / Down scroll",
    "+ / - zoom; 0 / W fit width; F fit page; 1 actual size",
    "T thumbnails; O outline (Up / Down / Enter choose)",
    "Search: Ctrl+F find; Enter / Ctrl+G next; spaces cross lines",
    "Escape closes search",
    "Markup: Ctrl+R region; drag on page or use arrow keys",
    "Shift+arrows size the region; Enter highlights it",
    "Ctrl+H highlight a found word or selected region",
    "Ctrl+Shift+H choose the next highlight color",
    "Ctrl+Shift+O choose the next highlight opacity",
    "Ctrl+N note; Ctrl+L annotations; Delete removes selected",
    "Forms: Ctrl+E; Ctrl+Down / Ctrl+Up change field",
    "Text: arrows/Home/End move; Shift selects; Ctrl+A selects all",
    "Backspace/Delete or typing replaces selected form text",
    "Click the focused text value in the bar to place its caret",
    "Space toggles boxes or sets a radio choice; Up/Down chooses",
    "Click a choice value for its pointer menu; wheel scrolls options",
    "Click a PDF field to focus; checkbox/radio clicks set it",
    "Ctrl+Z / Ctrl+Shift+Z undo / redo fields or markup",
    "Ctrl+Shift+G places a typed visual signature",
    "Visual signatures are markup, not digital signatures",
    "Ctrl+Shift+V inspects stored digital signature fields",
    "Ctrl+Shift+D places an empty digital signature field",
    "Ctrl+Shift+B signs an empty field; Enter confirms, Escape cancels",
    "Escape also cancels while Shibari approval is pending",
    "Shibari asks separately; content integrity does not establish trust",
    "Ctrl+S saves edits; Ctrl+Shift+/ opens this help",
];
/// The chosen region's outline, and a focused form field's.
const REGION: u32 = 0x00e0457b;
/// How far one arrow press moves or sizes the region, in points.
const NUDGE: f64 = 12.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel { None, Thumbs, Outline, Annots }

/// Form filling: the fields the keyboard reaches, which one, and its text so far.
struct Form {
    fields: Vec<(FieldInfo, usize, [f64; 4])>, at: usize, text: String,
    caret: usize, anchor: Option<usize>, choice: Option<usize>, popup: bool, popup_top: usize,
}

fn text_selection(anchor: Option<usize>, caret: usize) -> Option<(usize, usize)> {
    anchor.filter(|at| *at != caret).map(|at| (at.min(caret), at.max(caret)))
}

fn replace_text_selection(text: &mut String, caret: &mut usize, anchor: &mut Option<usize>) -> bool {
    let Some((start, end)) = text_selection(*anchor, *caret) else { *anchor = None; return false };
    let first = text_byte(text, start);
    let last = text_byte(text, end);
    text.replace_range(first..last, "");
    *caret = start;
    *anchor = None;
    true
}

fn text_byte(text: &str, caret: usize) -> usize {
    text.char_indices().nth(caret).map_or(text.len(), |(at, _)| at)
}

fn text_with_caret(text: &str, caret: usize) -> String {
    let at = text_byte(text, caret);
    format!("{}|{}", &text[..at], &text[at..])
}

fn text_display(text: &str, caret: usize, anchor: Option<usize>) -> String {
    if text_selection(anchor, caret).is_some() { text.to_owned() }
    else { text_with_caret(text, caret) }
}

fn text_caret_at_x(x: usize, value_x: usize, chars: usize) -> usize {
    ((x.saturating_sub(value_x).saturating_add(CW / 2)) / CW).min(chars)
}

fn dragged_region(start: (f64, f64), end: (f64, f64), bounds: (f64, f64)) -> [f64; 4] {
    let (w, h) = (bounds.0.max(0.0), bounds.1.max(0.0));
    let (min_w, min_h) = (w.min(6.0), h.min(6.0));
    let x0 = start.0.min(end.0).clamp(0.0, w - min_w);
    let y0 = start.1.min(end.1).clamp(0.0, h - min_h);
    let x1 = start.0.max(end.0).clamp(x0 + min_w, w);
    let y1 = start.1.max(end.1).clamp(y0 + min_h, h);
    [x0, y0, x1, y1]
}

fn text_backspace(text: &mut String, caret: &mut usize) {
    if *caret == 0 { return; }
    let end = text_byte(text, *caret);
    *caret -= 1;
    let start = text_byte(text, *caret);
    text.replace_range(start..end, "");
}

fn text_delete(text: &mut String, caret: usize) {
    let at = text_byte(text, caret);
    if at < text.len() {
        let end = text_byte(text, caret + 1);
        text.replace_range(at..end, "");
    }
}

fn text_insert(text: &mut String, caret: &mut usize, c: char) {
    if text.chars().count() >= 200 { return; }
    text.insert(text_byte(text, *caret), c);
    *caret += 1;
}

/// The bounded menu fits in the client area even for a PDF with many choices.
fn choice_menu(width: usize, height: usize, bar: usize, count: usize) -> Option<(usize, usize, usize, usize)> {
    let rows = count.min(CHOICE_MENU_ROWS).min(height.saturating_sub(bar + 4) / ROW);
    let w = width.saturating_sub(16).min(CHOICE_MENU_WIDTH);
    (rows > 0 && w >= 80).then_some((8, bar, w, rows))
}

fn choice_menu_hit(menu: (usize, usize, usize, usize), x: usize, y: usize, top: usize, count: usize) -> Option<usize> {
    let (mx, my, mw, rows) = menu;
    if x < mx || x >= mx + mw || y < my || y >= my + rows * ROW { return None; }
    let index = top + (y - my) / ROW;
    (index < count).then_some(index)
}

fn scroll_choice_top(top: usize, count: usize, rows: usize, steps: i8) -> usize {
    if steps > 0 { top.saturating_sub(steps as usize) }
    else { top.saturating_add(steps.unsigned_abs() as usize).min(count.saturating_sub(rows)) }
}

struct SigningChoice { signer: pdf_sign::Signer, field: String, certificate_hash: [u8; 32] }

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
struct Rendered { pixels: Vec<u32>, w: usize, h: usize, page: usize, scale: f32, ink: usize }

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
    let cache = RenderCache::new();
    render_page_cached(pdf, &cache, index, scale)
}

/// Share Hayro's glyph and interpreter work while drawing a batch of thumbnails.
fn render_page_cached<'a>(pdf: &'a Pdf, cache: &RenderCache<'a>, index: usize, scale: f32) -> Option<(Vec<u32>, usize, usize)> {
    let pages = pdf.pages();
    let page = pages.get(index)?;
    let pixmap = render(page, cache, &InterpreterSettings::default(), &RenderSettings::default(),
        &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE });
    let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
    let pixels = pixmap.data_as_u8_slice().chunks_exact(4).map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect();
    Some((pixels, w, h))
}

struct Viewer {
    palette: Palette,
    theme_generation: u64,
    pdf: Pdf,
    name: String,
    page: usize,
    zoom: Zoom,
    scroll: usize,
    hscroll: usize,
    shown: Option<Rendered>,
    /// Oldest first. These are owned pixels, not references into Hayro's cache.
    page_cache: Vec<Rendered>,
    cache_bytes: usize,
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
    /// The recovered PDF has not yet been committed to its source by Ctrl+S.
    recovered_pending: bool,
    /// Last complete copy kept for this open document; unchanged redraws do not
    /// write it again.
    recovery_fingerprint: Option<(usize, u64)>,
    field_history: FieldHistory,
    annotation_history: AnnotationHistory,
    highlight_color: usize,
    highlight_opacity: usize,
    /// The region Ctrl+R chose: shown points `[left, top, right, bottom]` on the page.
    region: Option<[f64; 4]>,
    region_drag: Option<(usize, (f64, f64))>,
    /// A sticky note being typed.
    note: Option<String>,
    /// Comment being edited for a highlight in the annotation list.
    comment: Option<(usize, String, &'static str)>,
    /// A typed visual signature; it is markup, not a cryptographic signature.
    signature: Option<String>,
    signing: Option<SigningChoice>,
    close_after_sign: bool,
    form: Option<Form>,
    /// The focused text field whose form-bar drag began with the primary button.
    text_drag: Option<usize>,
    /// The current page's annotations, while Ctrl+L lists them.
    annots: Vec<AnnotInfo>,
    /// A line of news in the bar until the next key: saved, refused, ...
    status: Option<String>,
    shortcuts: bool,
    shortcuts_top: usize,
}

/// Where each part of the window is.
struct Layout { panel: usize, bar: usize, x0: usize, y0: usize, aw: usize, ah: usize }

fn shown_at_click((x, y): (usize, usize), l: &Layout, r: &Rendered, scroll: usize, hscroll: usize) -> Option<(f64, f64)> {
    if x < l.x0 || y < l.y0 || x >= l.x0 + l.aw || y >= l.y0 + l.ah || r.scale <= 0.0 { return None; }
    let left = l.x0 + if r.w + 2 * MARGIN <= l.aw { (l.aw - r.w) / 2 } else { MARGIN };
    let skip = hscroll.saturating_sub(MARGIN).min(r.w);
    let px = x as f64 - left as f64 + skip as f64;
    let py = y as f64 - l.y0 as f64 - MARGIN as f64 + scroll as f64;
    (px >= 0.0 && py >= 0.0 && px < r.w as f64 && py < r.h as f64)
        .then_some((px / r.scale as f64, py / r.scale as f64))
}

fn focus_destination(first_widget_page: usize, pointer_page: Option<usize>) -> (usize, bool) {
    (pointer_page.unwrap_or(first_widget_page), pointer_page.is_none())
}

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
        let hit = self.page_cache.iter().position(|s| s.page == self.page && (s.scale - scale).abs() < 1e-4)
            .map(|i| {
                let page = self.page_cache.remove(i);
                self.cache_bytes = self.cache_bytes.saturating_sub(page.pixels.len().saturating_mul(4));
                page
            });
        if let Some(old) = self.shown.take() { self.remember(old); }
        if let Some(page) = hit {
            sex_std::record(&format!("[pdf.page] page={} pages={} width={} height={} ink={} ms={}",
                page.page + 1, self.pages(), page.w, page.h, page.ink, started.elapsed().as_millis()));
            sex_std::record(&format!("[pdf.page.cache] hit=1 page={} bytes={}", page.page + 1, page.pixels.len().saturating_mul(4)));
            self.shown = Some(page);
            return;
        }
        let Some((pixels, w, h)) = render_page(&self.pdf, self.page, scale) else { return };
        let ink = pixels.iter().filter(|p| **p != 0x00ffffff).count();
        sex_std::record(&format!("[pdf.page] page={} pages={} width={w} height={h} ink={ink} ms={}",
            self.page + 1, self.pages(), started.elapsed().as_millis()));
        self.shown = Some(Rendered { pixels, w, h, page: self.page, scale, ink });
    }

    fn remember(&mut self, page: Rendered) {
        let bytes = page.pixels.len().saturating_mul(4);
        if bytes > CACHE_BYTES { return; }
        while !self.page_cache.is_empty() && (self.page_cache.len() >= CACHE_PAGES || self.cache_bytes.saturating_add(bytes) > CACHE_BYTES) {
            let old = self.page_cache.remove(0);
            self.cache_bytes = self.cache_bytes.saturating_sub(old.pixels.len().saturating_mul(4));
        }
        self.cache_bytes += bytes;
        self.page_cache.push(page);
    }

    fn clear_page_cache(&mut self) {
        self.shown = None;
        self.page_cache.clear();
        self.cache_bytes = 0;
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
        if page != self.page {
            self.page = page; self.scroll = 0; self.hscroll = 0;
            self.region = None; self.region_drag = None;
        }
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
        let palette = self.palette;
        let size = window.size();
        let out = window.pixels();
        let (width, height) = (size.0.min(out.len() / size.1.max(1)), size.1);
        if width == 0 || height == 0 { return; }
        let l = self.layout((width, height));
        let scale = self.scale(&l);
        self.render(scale);
        out[..width * height].fill(palette.background);
        if let Some(r) = &self.shown {
            let max_scroll = (r.h + 2 * MARGIN).saturating_sub(l.ah);
            if let Some(y) = self.reveal.take() { self.scroll = zoom::scroll_to(y, r.scale, r.h, l.ah, MARGIN); }
            self.scroll = self.scroll.min(max_scroll);
            self.hscroll = self.hscroll.min((r.w + 2 * MARGIN).saturating_sub(l.aw));
            let left = l.x0 + if r.w + 2 * MARGIN <= l.aw { (l.aw - r.w) / 2 } else { MARGIN };
            let hits: Vec<(Hit, bool)> = self.hits.iter().filter(|h| h.page == r.page && self.finding).map(|h| (*h, Some(*h) == self.hit)).collect();
            let boxes = self.texts.as_ref().map(|t| &t[r.page].boxes);
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
                        for b in boxes.iter().skip(hit.at).take(hit.len) {
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
                if let Some((field, _, _)) = form.fields.get(form.at) {
                    if let Ok(info) = editor.page_info(r.page) {
                        for (_, page, rect) in &field.widgets {
                            if *page == Some(r.page) { outline(out, width, to_screen(marks::rect_to_shown(&info, *rect)), clip, REGION); }
                        }
                    }
                }
            }
        }
        if l.panel > 0 { self.draw_panel(out, width, height); }
        if l.bar > 0 { self.draw_bar(out, width, l.bar); }
        self.draw_choice_menu(out, width, height, l.bar);
        if self.shortcuts { self.draw_shortcuts(out, width, height); }
        window.show_pixels(width, height);
        let short: String = self.name.chars().take(9).collect();
        let changed = if self.editor.as_ref().is_ok_and(|e| e.is_modified()) { "*" } else { "" };
        window.title(format!("{short}{changed} {}/{}", self.page + 1, self.pages()).as_bytes());
    }

    fn bar_shown(&self) -> bool {
        self.finding || self.note.is_some() || self.comment.is_some() || self.signature.is_some() || self.signing.is_some() || self.form.is_some() || self.region.is_some() || self.status.is_some()
    }

    fn draw_shortcuts(&mut self, out: &mut [u32], width: usize, height: usize) {
        if width < 220 || height < 100 { return; }
        let palette = self.palette;
        let (x, y, w, h) = (16, 16, width - 32, height - 32);
        fill(out, width, (x, y, w, h), palette.panel_back);
        fill(out, width, (x, y, w, 2), palette.accent);
        let Some(letters) = self.letters.as_mut() else { return };
        letters.draw(out, width, (x + 12, y + 12), width - 16, "Vellum shortcuts", palette.ink);
        let fits = (h.saturating_sub(64) / ROW).max(1);
        let top = self.shortcuts_top.min(SHORTCUTS.len().saturating_sub(fits));
        for (i, line) in SHORTCUTS.iter().skip(top).take(fits).enumerate() {
            letters.draw(out, width, (x + 12, y + 40 + i * ROW), width - 16, line, palette.dim);
        }
        letters.draw(out, width, (x + 12, height - 36), width - 16, "Up / Down scroll; Escape closes", palette.accent);
    }

    fn draw_bar(&mut self, out: &mut [u32], width: usize, bar: usize) {
        let palette = self.palette;
        fill(out, width, (0, 0, width, bar), palette.bar_back);
        fill(out, width, (0, bar - 1, width, 1), palette.accent);
        let y = (bar.saturating_sub(CH)) / 2;
        // Editing has the bar before find: what is typed goes where the bar says.
        let editing = if let Some(choice) = &self.signing {
            Some(("Sole Shibari cert:".into(), format!("{} / {}", choice.field, choice.certificate_hash[..8].iter().map(|b| format!("{b:02X}")).collect::<String>()), "Enter requests approval; Escape cancels".into()))
        } else if let Some(signature) = &self.signature {
            Some(("Visual signature:".to_string(), format!("{signature}_"), "Enter places, Escape cancels; not digital".to_string()))
        } else if let Some(note) = &self.note {
            Some(("Note:".to_string(), format!("{note}_"), "Enter adds it, Escape cancels".to_string()))
        } else if let Some((_, comment, kind)) = &self.comment {
            let label = if *kind == "Note" { "Note text:" } else { "Highlight comment:" };
            Some((label.to_string(), format!("{comment}_"), "Enter saves, Escape cancels".to_string()))
        } else if let Some(form) = &self.form {
            form.fields.get(form.at).map(|(f, _, _)| {
                let value = match f.kind {
                    FieldKind::Checkbox => if marks::toggled(f) == "Off" { "[x]  Space unticks".into() } else { "[ ]  Space ticks".into() },
                    FieldKind::Radio => form.choice.and_then(|i| f.states.get(i)).map(|state|
                        format!("{state}  (Up/Down choose, Space sets)")).unwrap_or_else(|| "Up/Down choose, Space sets".into()),
                    FieldKind::ComboBox { editable: false } | FieldKind::ListBox { multi_select: false } =>
                        form.choice.and_then(|i| f.options.get(i)).map(|(_, shown)| format!("{shown}  (Up/Down choose, Enter saves)"))
                            .unwrap_or_else(|| "Up/Down choose, Enter saves".into()),
                    FieldKind::ComboBox { editable: true } => form.choice.and_then(|i| f.options.get(i))
                        .map(|(_, shown)| format!("{shown}  (type or Up/Down choose)"))
                        .unwrap_or_else(|| format!("{}  (type or Up/Down choose)", text_display(&form.text, form.caret, form.anchor))),
                    FieldKind::ListBox { multi_select: true } => form.choice.and_then(|i| f.options.get(i))
                        .map(|(_, shown)| format!("{} {shown}  (Space toggles, Up/Down moves)",
                            if marks::choice_indices(f).contains(&i) { "[x]" } else { "[ ]" }))
                        .unwrap_or_else(|| "Up/Down moves, Space toggles".into()),
                    _ => text_display(&form.text, form.caret, form.anchor),
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
            let vx = 8 + (label.chars().count() + 1) * CW;
            let value_width = width.saturating_sub((right.len() + 2) * CW);
            if let Some(form) = self.form.as_ref() {
                if let Some((start, end)) = text_selection(form.anchor, form.caret) {
                    let left = vx.saturating_add(start.saturating_mul(CW));
                    let right_edge = vx.saturating_add(end.saturating_mul(CW)).min(vx.saturating_add(value_width)).min(width);
                    if left < right_edge { fill(out, width, (left, y, right_edge - left, CH), palette.selected); }
                }
            }
            let Some(letters) = self.letters.as_mut() else { return };
            letters.draw(out, width, (8, y), width, &label, palette.dim);
            letters.draw(out, width, (vx, y), value_width, &value, palette.ink);
            let sx = width.saturating_sub(8 + right.len() * CW);
            letters.draw(out, width, (sx, y), width, &right, palette.dim);
            return;
        }
        let status = if self.term != self.searched || self.term.is_empty() { String::new() }
            else if self.found_none { "not found".into() }
            else { self.hit.and_then(|h| self.hits.iter().position(|x| *x == h)).map(|i| format!("{} of {}", i + 1, self.hits.len())).unwrap_or_default() };
        let y = (bar.saturating_sub(CH)) / 2;
        let Some(letters) = self.letters.as_mut() else { return };
        letters.draw(out, width, (8, y), width, "Find:", palette.dim);
        letters.draw(out, width, (8 + 6 * CW, y), width.saturating_sub(16 * CW), &format!("{}_", self.term), palette.ink);
        let sx = width.saturating_sub(8 + status.len() * CW);
        letters.draw(out, width, (sx, y), width, &status, if self.found_none { palette.danger } else { palette.dim });
    }

    fn draw_choice_menu(&mut self, out: &mut [u32], width: usize, height: usize, bar: usize) {
        let Some(form) = self.form.as_ref().filter(|f| f.popup) else { return };
        let Some((field, _, _)) = form.fields.get(form.at) else { return };
        let Some((x, y, w, rows)) = choice_menu(width, height, bar, field.options.len()) else { return };
        let selected = marks::choice_indices(field);
        let lines: Vec<(usize, String)> = field.options.iter().enumerate().skip(form.popup_top).take(rows)
            .map(|(i, (_, shown))| (i, format!("{} {shown}", if selected.contains(&i) { "[x]" } else { "[ ]" }))).collect();
        fill(out, width, (x, y, w, rows * ROW), self.palette.panel_back);
        let Some(letters) = self.letters.as_mut() else { return };
        for (row, (index, label)) in lines.iter().enumerate() {
            if Some(*index) == form.choice {
                fill(out, width, (x, y + row * ROW, w, ROW), self.palette.selected);
            }
            letters.draw(out, width, (x + 6, y + row * ROW + 2), w.saturating_sub(12), label,
                if Some(*index) == form.choice { self.palette.accent } else { self.palette.ink });
        }
    }

    fn draw_panel(&mut self, out: &mut [u32], width: usize, height: usize) {
        let palette = self.palette;
        fill(out, width, (0, 0, PANEL, height), palette.panel_back);
        match self.panel {
            Panel::Thumbs => self.draw_thumbs(out, width, height),
            Panel::Outline => self.draw_outline(out, width, height),
            Panel::Annots => self.draw_annots(out, width, height),
            Panel::None => {}
        }
    }

    fn draw_annots(&mut self, out: &mut [u32], width: usize, height: usize) {
        let palette = self.palette;
        let top = if self.bar_shown() { BAR } else { 0 } + 6;
        let rows: Vec<String> = self.annots.iter().enumerate().map(|(i, a)| format!("{} {}", i + 1, describe(a))).collect();
        let Some(letters) = self.letters.as_mut() else { return };
        if rows.is_empty() {
            letters.draw(out, width, (8, top), PANEL - 4, "No annotations", palette.dim);
            return;
        }
        let fits = ((height - top) / ROW).max(1);
        let first = self.chosen.saturating_sub(fits - 1);
        for (row, (i, text)) in rows.iter().enumerate().skip(first).take(fits).enumerate() {
            let y = top + row * ROW;
            if i == self.chosen { fill(out, width, (0, y, PANEL, ROW), palette.selected); }
            letters.draw(out, width, (8, y + (ROW - CH) / 2), PANEL - 4, text, if i == self.chosen { palette.ink } else { palette.dim });
        }
    }

    /// One slot of the strip: the thumbnail, its border and its number below it.
    fn thumb_slot(&self) -> usize {
        let (pw, ph) = self.pdf.pages().first().map_or((612.0, 792.0), |p| p.render_dimensions());
        ((PANEL - 24) as f32 * ph / pw.max(1.0)) as usize + 8 + CH + 8
    }

    fn draw_thumbs(&mut self, out: &mut [u32], width: usize, height: usize) {
        let started = Instant::now();
        // Keep temporary decoded resources bounded even on a very tall display.
        let mut cache = RenderCache::new();
        let palette = self.palette;
        let top = if self.bar_shown() { BAR } else { 0 };
        let slot = self.thumb_slot().max(1);
        let fits = ((height - top) / slot).max(1);
        if self.page < self.thumb_top { self.thumb_top = self.page; }
        if self.page >= self.thumb_top + fits { self.thumb_top = self.page + 1 - fits; }
        let mut rendered = 0;
        for i in self.thumb_top..(self.thumb_top + fits + 1).min(self.pages()) {
            if self.thumbs[i].is_none() {
                if rendered != 0 && rendered % 2 == 0 { cache = RenderCache::new(); }
                let pw = self.pdf.pages()[i].render_dimensions().0.max(1.0);
                self.thumbs[i] = render_page_cached(&self.pdf, &cache, i, (PANEL - 24) as f32 / pw);
                rendered += 1;
            }
            let y = top + (i - self.thumb_top) * slot + 8;
            let Some((pixels, w, h)) = &self.thumbs[i] else { continue };
            let x = (PANEL - w) / 2;
            if i == self.page { fill(out, width, (x - 3, y.saturating_sub(3), w + 6, (*h + 6).min(height.saturating_sub(y.saturating_sub(3)))), palette.accent); }
            for row in 0..*h {
                if y + row >= height { break; }
                out[(y + row) * width + x..(y + row) * width + x + w].copy_from_slice(&pixels[row * w..row * w + w]);
            }
            let label = format!("{}", i + 1);
            if y + h + 4 + CH <= height {
                let lx = (PANEL - label.len() * CW) / 2;
                if let Some(letters) = self.letters.as_mut() { letters.draw(out, width, (lx, y + h + 4), PANEL, &label, if i == self.page { palette.ink } else { palette.dim }); }
            }
        }
        if rendered > 0 {
            sex_std::record(&format!("[pdf.thumbs] rendered={rendered} cached={} first={} ms={}", self.thumbs.iter().filter(|t| t.is_some()).count(), self.thumb_top + 1, started.elapsed().as_millis()));
        }
    }

    fn draw_outline(&mut self, out: &mut [u32], width: usize, height: usize) {
        let palette = self.palette;
        let top = if self.bar_shown() { BAR } else { 0 } + 6;
        let entries = self.outline.clone().unwrap_or_default();
        let Some(letters) = self.letters.as_mut() else { return };
        if entries.is_empty() {
            letters.draw(out, width, (8, top), PANEL - 4, "No outline", palette.dim);
            return;
        }
        let fits = ((height - top) / ROW).max(1);
        let first = self.chosen.saturating_sub(fits - 1);
        for (row, (i, e)) in entries.iter().enumerate().skip(first).take(fits).enumerate() {
            let y = top + row * ROW;
            if i == self.chosen { fill(out, width, (0, y, PANEL, ROW), palette.selected); }
            let indent = 8 + e.depth.min(4) * CW;
            let color = if e.page == Some(self.page) { palette.ink } else { palette.dim };
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
    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key, window: &mut sex_app::Window) -> bool {
        let size = window.size();
        if key.control && key.shift && key.code == SLASH {
            self.shortcuts = !self.shortcuts;
            self.shortcuts_top = 0;
            sex_std::record(&format!("[pdf.shortcuts] shown={} rows={}", self.shortcuts as u8, SHORTCUTS.len()));
            return true;
        }
        if self.shortcuts {
            match sex_app::nav(key) {
                Some(Nav::Down) => self.shortcuts_top = (self.shortcuts_top + 1).min(SHORTCUTS.len().saturating_sub(1)),
                Some(Nav::Up) => self.shortcuts_top = self.shortcuts_top.saturating_sub(1),
                _ if key.code == ESCAPE => self.shortcuts = false,
                _ => return false,
            }
            sex_std::record(&format!("[pdf.shortcuts] shown={} top={}", self.shortcuts as u8, self.shortcuts_top));
            return true;
        }
        let had_status = self.status.take().is_some();
        if self.edit_key(key, window) { return true; }
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
        let popup_was_open = self.form.as_ref().is_some_and(|f| f.popup);
        if self.choice_pointer(p.kind, (x, y), size, l.bar) { return true; }
        if self.text_pointer(p.kind, p.buttons, (x, y), l.bar) { return true; }
        if self.region_pointer(p.kind, p.buttons, (x, y), &l) { return true; }
        if self.widget_pointer(p.kind, (x, y), &l, size) { return true; }
        if popup_was_open && self.form.as_ref().is_some_and(|f| !f.popup) { return true; }
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

    fn region_pointer(&mut self, kind: Kind, buttons: u8, point: (usize, usize), l: &Layout) -> bool {
        if self.shortcuts || self.region.is_none() { self.region_drag = None; return false; }
        let shown = self.shown.as_ref().and_then(|r| shown_at_click(point, l, r, self.scroll, self.hscroll));
        match kind {
            Kind::Down { button: 0 } => {
                let Some(start) = shown else { return false };
                self.region_drag = Some((self.page, start));
                self.update_drag_region(start, start);
                true
            }
            Kind::Move if self.region_drag.is_some() => {
                if buttons & 1 == 0 { self.region_drag = None; return true; }
                if self.region_drag.is_some_and(|(page, _)| page != self.page) { self.region_drag = None; return true; }
                if let Some(end) = shown {
                    let (_, start) = self.region_drag.unwrap();
                    self.update_drag_region(start, end);
                }
                true
            }
            Kind::Up { button: 0 } if self.region_drag.is_some() => {
                let (page, start) = self.region_drag.take().unwrap();
                if page != self.page { return true; }
                if let Some(end) = shown { self.update_drag_region(start, end); }
                if let Some(r) = self.region {
                    sex_std::record(&format!("[pdf.region.drag] page={} x={:.0} y={:.0} w={:.0} h={:.0}",
                        page + 1, r[0], r[1], r[2] - r[0], r[3] - r[1]));
                }
                true
            }
            _ => false,
        }
    }

    fn update_drag_region(&mut self, start: (f64, f64), end: (f64, f64)) {
        let Some(page) = self.pdf.pages().get(self.page) else { return };
        let (w, h) = page.render_dimensions();
        self.region = Some(dragged_region(start, end, (w as f64, h as f64)));
    }

    fn text_pointer(&mut self, kind: Kind, buttons: u8, (x, y): (usize, usize), bar: usize) -> bool {
        if self.shortcuts { self.text_drag = None; return false; }
        let Some(form) = self.form.as_mut() else { self.text_drag = None; return false };
        let Some((field, _, _)) = form.fields.get(form.at) else { self.text_drag = None; return false };
        if !matches!(field.kind, FieldKind::Text { .. }) { self.text_drag = None; return false; }
        let value_x = 8 + (field.name.chars().count() + 2) * CW;
        let at = text_caret_at_x(x, value_x, form.text.chars().count());
        match kind {
            Kind::Down { button: 0 } if y < bar => {
                form.caret = at;
                form.anchor = Some(at);
                self.text_drag = Some(form.at);
                sex_std::record(&format!("[pdf.form.pointer] name={} at={} chars={}", field.name, at, form.text.chars().count()));
                true
            }
            Kind::Move if self.text_drag == Some(form.at) => {
                if buttons & 1 == 0 { self.text_drag = None; form.anchor = None; return true; }
                form.caret = at;
                if let Some((start, end)) = text_selection(form.anchor, form.caret) {
                    sex_std::record(&format!("[pdf.form.drag.selection] name={} start={} end={} chars={}", field.name, start, end, form.text.chars().count()));
                }
                true
            }
            Kind::Up { button: 0 } if self.text_drag == Some(form.at) => {
                form.caret = at;
                self.text_drag = None;
                let selected = text_selection(form.anchor, form.caret);
                if let Some((start, end)) = selected {
                    sex_std::record(&format!("[pdf.form.drag.selection] name={} start={} end={} chars={}", field.name, start, end, form.text.chars().count()));
                } else { form.anchor = None; }
                true
            }
            Kind::Leave if self.text_drag.is_some() => { self.text_drag = None; true }
            _ => false,
        }
    }

    fn widget_pointer(&mut self, kind: Kind, point: (usize, usize), l: &Layout, size: (usize, usize)) -> bool {
        if !matches!(kind, Kind::Down { button: 0 }) || self.shortcuts || self.region.is_some() || self.signing.is_some() || self.note.is_some() || self.comment.is_some() || self.signature.is_some() {
            return false;
        }
        let Some(shown) = self.shown.as_ref().and_then(|r| shown_at_click(point, l, r, self.scroll, self.hscroll)) else { return false };
        let Some(info) = self.page_info(self.page) else { return false };
        let fields = if let Some(form) = &self.form { form.fields.clone() }
            else { self.editor.as_ref().ok().map(|ed| marks::fillable(&ed.fields().unwrap_or_default())).unwrap_or_default() };
        let Some((index, widget)) = marks::widget_at_shown(&fields, self.page, &info, shown) else { return false };
        let clicked_page = self.page;
        if self.form.is_some() { self.commit_field(); }
        else {
            sex_std::record(&format!("[pdf.form] fields={} first={}", fields.len(), fields[0].0.name));
            self.form = Some(Form { fields, at: 0, text: String::new(), caret: 0, anchor: None, choice: None, popup: false, popup_top: 0 });
        }
        self.focus_at(index, Some(clicked_page));
        let Some((field, _, _)) = self.form.as_ref().and_then(|f| f.fields.get(index)).cloned() else { return true };
        let kind = match &field.kind {
            FieldKind::Checkbox => {
                let to = marks::toggled(&field);
                self.set_field(&field.name, FieldValue::State(to.clone()), "checkbox", &to);
                "checkbox"
            }
            FieldKind::Radio => {
                if let Some(state) = marks::radio_state_at(&field, widget) {
                    let state = state.to_string();
                    self.set_field(&field.name, FieldValue::State(state.clone()), "radio", &state);
                } else { self.status = Some("This radio option has no PDF on-state".into()); }
                "radio"
            }
            FieldKind::ComboBox { .. } | FieldKind::ListBox { .. } => {
                if let Some(form) = self.form.as_mut() {
                    let menu = choice_menu(size.0, size.1, l.bar, field.options.len());
                    form.popup = menu.is_some();
                    form.popup_top = form.choice.unwrap_or(0).min(field.options.len().saturating_sub(menu.map_or(0, |m| m.3)));
                }
                "choice"
            }
            _ => "text",
        };
        sex_std::record(&format!("[pdf.widget.pointer] name={} kind={kind} page={}", field.name, self.page + 1));
        true
    }

    fn choice_pointer(&mut self, kind: Kind, (x, y): (usize, usize), (width, height): (usize, usize), bar: usize) -> bool {
        let Some(form) = self.form.as_ref() else { return false };
        let Some((field, _, _)) = form.fields.get(form.at) else { return false };
        if !matches!(field.kind, FieldKind::ComboBox { .. } | FieldKind::ListBox { .. }) { return false; }
        let count = field.options.len();
        let Some(menu) = choice_menu(width, height, bar, count) else { return false };
        if let Kind::Wheel { steps } = kind {
            if !form.popup || choice_menu_hit(menu, x, y, form.popup_top, count).is_none() { return false; }
            let top = scroll_choice_top(form.popup_top, count, menu.3, steps);
            if let Some(form) = self.form.as_mut() { form.popup_top = top; }
            return true;
        }
        if !matches!(kind, Kind::Down { button: 0 }) { return false; }
        if form.popup {
            if let Some(index) = choice_menu_hit(menu, x, y, form.popup_top, count) {
                let (field, _, _) = form.fields[form.at].clone();
                if let Some(form) = self.form.as_mut() {
                    form.choice = Some(index);
                    if let FieldKind::ComboBox { editable: true } = &field.kind {
                        form.text = field.options[index].0.clone();
                        form.caret = form.text.chars().count();
                        form.anchor = None;
                    }
                    form.popup = matches!(&field.kind, FieldKind::ListBox { multi_select: true });
                }
                if let Some(value) = marks::clicked_choice(&field, index).filter(|v| *v != field.value) {
                    let (kind, shown) = if let FieldValue::Choices(v) = &value {
                        ("multi-choice", format!("{} selected", v.len()))
                    } else { ("choice", field.options[index].1.clone()) };
                    self.set_field(&field.name, value, kind, &shown);
                }
                sex_std::record(&format!("[pdf.choice.pointer] name={} selected={} count={}", field.name, index + 1, field.options.len()));
                return true;
            }
            if let Some(form) = self.form.as_mut() { form.popup = false; }
            return false;
        }
        if y < bar && x < width.saturating_mul(2) / 3 {
            if let Some(form) = self.form.as_mut() {
                form.popup = true;
                form.popup_top = form.choice.unwrap_or(0).min(count.saturating_sub(menu.3));
            }
            sex_std::record(&format!("[pdf.choice.menu] name={} count={count}", field.name));
            return true;
        }
        false
    }
}

/// Annotating, form filling and saving.
impl Viewer {
    /// An editing key; true when it was one (and the window should be drawn again).
    fn edit_key(&mut self, key: sex_std::sex_pdx::app_key::Key, window: &mut sex_app::Window) -> bool {
        let ctrl = |code: u8| key.control && key.code == code;
        if let Some(choice) = self.signing.take() {
            if key.code == ENTER && !key.control { self.sign_field(choice, window); }
            else if key.code == ESCAPE { self.status = Some("Digital signing cancelled".into()); sex_std::record("[pdf.sign.cancelled] stage=viewer"); }
            else { self.signing = Some(choice); }
            return true;
        }
        if self.signature.is_some() { self.signature_key(key); return true; }
        if self.comment.is_some() { self.comment_key(key); return true; }
        if ctrl(KEY_Z) {
            self.commit_field();
            if self.annotation_history.has_step(key.shift) { self.annotation_history_step(key.shift); }
            else { self.field_history_step(key.shift); }
            return true;
        }
        if ctrl(KEY_G) && key.shift {
            if self.editor().is_some() { self.signature = Some(String::new()); }
            return true;
        }
        if ctrl(KEY_V) && key.shift { self.inspect_signatures(); return true; }
        if ctrl(KEY_D) && key.shift { self.place_signature_field(); return true; }
        if ctrl(KEY_B) && key.shift { self.choose_signature_field(); return true; }
        if ctrl(KEY_S) { self.save(); return true; }
        if ctrl(KEY_H) && key.shift {
            self.highlight_color = (self.highlight_color + 1) % HIGHLIGHT_COLORS.len();
            let name = HIGHLIGHT_COLORS[self.highlight_color].0;
            self.status = Some(format!("Highlight color: {name}. Ctrl+H marks text or a region."));
            sex_std::record(&format!("[pdf.highlight.color] name={name}"));
            return true;
        }
        if ctrl(KEY_O) && key.shift {
            self.highlight_opacity = (self.highlight_opacity + 1) % HIGHLIGHT_OPACITIES.len();
            let name = HIGHLIGHT_OPACITIES[self.highlight_opacity].0;
            self.status = Some(format!("Highlight opacity: {name}. Ctrl+H marks text or a region."));
            sex_std::record(&format!("[pdf.highlight.opacity] value={name}"));
            return true;
        }
        if ctrl(KEY_H) { self.highlight(); return true; }
        if ctrl(KEY_R) {
            self.region = match self.region { Some(_) => None, None => Some(self.first_region()) };
            self.region_drag = None;
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

    fn inspect_signatures(&mut self) {
        match self.editor.as_ref().ok().and_then(|editor| editor.signatures().ok()) {
            Some(signatures) => {
                let signed = signatures.iter().filter(|s| s.signed).count();
                let malformed = signatures.iter().filter(|s| s.signed && !s.byte_range_well_formed).count();
                let mut intact = 0;
                let mut changed = 0;
                let mut unsupported = 0;
                if let Ok(editor) = &self.editor {
                    for signature in signatures.iter().filter(|s| s.signed) {
                        match editor.signature_content_integrity(&signature.field_name) {
                            Ok(ContentIntegrity::Intact) => intact += 1,
                            Ok(ContentIntegrity::Changed) => changed += 1,
                            _ => unsupported += 1,
                        }
                    }
                }
                self.status = Some(if signatures.is_empty() {
                    "No digital signature fields in this PDF".into()
                } else if malformed > 0 {
                    format!("{signed} stored signature(s), {malformed} with invalid byte ranges; trust unverified")
                } else if changed > 0 {
                    format!("{changed} signature content check(s) failed; signer trust unverified")
                } else if intact > 0 {
                    format!("{intact} signature content intact, {unsupported} unsupported; signer trust unverified")
                } else {
                    format!("{signed} stored signature(s); content check unsupported, signer trust unverified")
                });
                sex_std::record(&format!("[pdf.signatures] fields={} signed={signed} malformed={malformed} verified=0 content_intact={intact} content_changed={changed} unsupported={unsupported}", signatures.len()));
            }
            None => self.status = Some("Cannot inspect signatures in this PDF".into()),
        }
    }

    fn place_signature_field(&mut self) {
        let page = self.page;
        let Some(info) = self.page_info(page) else { let _ = self.editor(); return };
        let (w, h) = self.pdf.pages().get(page).map_or((612.0, 792.0), |p| p.render_dimensions());
        let shown = self.region.unwrap_or([w as f64 * 0.2, h as f64 * 0.78, w as f64 * 0.7, h as f64 * 0.84]);
        let rect = marks::rect_to_user(&info, shown);
        let Some(editor) = self.editor() else { return };
        let names: Vec<String> = match editor.fields() {
            Ok(fields) => fields.into_iter().map(|f| f.name).collect(),
            Err(e) => { self.status = Some(format!("Cannot list fields: {e}")); return; }
        };
        let name = (1..=999).map(|i| format!("Vellum signature {i}"))
            .find(|n| !names.contains(n));
        let Some(name) = name else { self.status = Some("Too many signature fields".into()); return };
        match editor.add_signature_field(page, &name, rect) {
            Ok(_) => {
                self.field_history.clear();
                self.annotation_history.clear();
                self.region = None;
                self.region_drag = None;
                self.refresh();
                self.status = Some(format!("{name} added. Ctrl+S saves this empty field."));
                sex_std::record(&format!("[pdf.sign.field] page={} name={name} empty=1", page + 1));
            }
            Err(e) => self.status = Some(format!("Could not add signature field: {e}")),
        }
    }

    fn choose_signature_field(&mut self) {
        self.commit_field();
        let recovered_pending = self.recovered_pending;
        let Some(editor) = self.editor() else { return };
        if editor.is_modified() || recovered_pending {
            self.status = Some("Save pending edits with Ctrl+S before signing".into());
            return;
        }
        let fields = match editor.signatures() {
            Ok(fields) => fields,
            Err(e) => { self.status = Some(format!("Cannot list signature fields: {e}")); return; }
        };
        if fields.iter().any(|field| field.signed) {
            self.status = Some("Already signed PDF: adding another signature is not supported".into());
            return;
        }
        let Some(field) = fields.iter().find(|field| !field.signed) else {
            self.status = Some("Place and save an empty signature field first".into());
            return;
        };
        let field_name = field.field_name.clone();
        let signer = match pdf_sign::selected() {
            Ok(Some(signer)) => signer,
            Ok(None) => { self.status = Some("Open Shibari and load a signing certificate first".into()); return; }
            Err(_) => { self.status = Some("Shibari signer unavailable".into()); return; }
        };
        let mut certificate = vec![0; signer.certificate_bytes];
        if pdf_sign::certificate(signer, &mut certificate).is_err() {
            self.status = Some("Could not read Shibari's certificate".into());
            return;
        }
        let certificate_hash: [u8; 32] = Sha256::digest(&certificate).into();
        self.signing = Some(SigningChoice { signer, field: field_name.clone(), certificate_hash });
        self.status = None;
        sex_std::record(&format!("[pdf.sign.choice] field={field_name} certificate_bytes={} confirmed=0", signer.certificate_bytes));
    }

    fn sign_field(&mut self, choice: SigningChoice, window: &mut sex_app::Window) {
        let source_path = self.path.clone();
        let sign_field = choice.field.clone();
        let mut certificate = vec![0; choice.signer.certificate_bytes];
        let mut point = [0; pdf_sign::POINT_BYTES];
        if pdf_sign::certificate(choice.signer, &mut certificate).is_err()
            || pdf_sign::public_point(choice.signer, &mut point).is_err() {
            self.status = Some("Could not read Shibari's certificate".into());
            sex_std::record("[pdf.sign.failed] stage=certificate");
            return;
        }
        if <[u8; 32]>::from(Sha256::digest(&certificate)) != choice.certificate_hash {
            self.status = Some("Shibari's certificate changed; choose it again".into());
            sex_std::record("[pdf.sign.failed] stage=certificate_changed");
            return;
        }
        let mut canceled_by_viewer = false;
        let mut close_after_sign = false;
        let result = self.editor.as_ref().ok().and_then(|editor| {
            editor.embed_detached_signature_with(&choice.field, 4096, |first, second| {
                vellum_edit::cms::detached_p256_sha256(first, second, &certificate, &point, |attrs| {
                    pdf_sign::request(choice.signer, &source_path, &sign_field, attrs)
                        .map_err(|_| vellum_edit::cms::CmsError::SignerRefused)?;
                    let deadline = sex_std::sex_pdx::get_ticks().saturating_add(90_000);
                    loop {
                        let mut signature = [0; pdf_sign::MAX_SIGNATURE];
                        match pdf_sign::poll(choice.signer, &mut signature) {
                            Ok(Some(len)) => return Ok(signature[..len].to_vec()),
                            Ok(None) if sex_std::sex_pdx::get_ticks() < deadline => {
                                match window.poll() {
                                    Some(Event::Key(key)) if key.code == ESCAPE => canceled_by_viewer = true,
                                    Some(Event::Close) => { canceled_by_viewer = true; close_after_sign = true; },
                                    _ => {},
                                }
                                if canceled_by_viewer {
                                    pdf_sign::cancel(choice.signer);
                                    return Err(vellum_edit::cms::CmsError::SignerRefused);
                                }
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                            _ => { pdf_sign::cancel(choice.signer); return Err(vellum_edit::cms::CmsError::SignerRefused); }
                        }
                    }
                }).map_err(|e| vellum_edit::Error::BadValue(format!("CMS signing failed: {e:?}")))
            }).ok()
        });
        self.close_after_sign = close_after_sign;
        let Some(bytes) = result else {
            if canceled_by_viewer {
                self.status = Some("Digital signing cancelled; PDF unchanged".into());
                sex_std::record("[pdf.sign.cancelled] stage=approval");
            } else {
                self.status = Some("Signature declined or could not be created; PDF unchanged".into());
                sex_std::record("[pdf.sign.failed] stage=approval_or_cms");
            }
            return;
        };
        if !Self::signature_intact(&bytes, &choice.field) {
            self.status = Some("Signature content check failed; PDF unchanged".into());
            sex_std::record("[pdf.sign.failed] stage=content_check");
            return;
        }
        let path = self.path.clone();
        let Some(original) = self.editor.as_ref().ok().map(Editor::original) else { return };
        if let Err(e) = thumb::write_whole_if_unchanged(&path, &bytes, original) {
            self.status = Some(format!("Could not save signed PDF: {e}"));
            sex_std::record("[pdf.sign.failed] stage=save");
            return;
        }
        let sync_result = sex_std::sex_app::files::direct::sync();
        let back = match std::fs::read(&path) {
            Ok(back) => back,
            Err(e) => { self.status = Some(format!("Signed PDF saved but could not be read: {e}")); return; }
        };
        if back != bytes || !Self::signature_intact(&back, &choice.field) {
            self.status = Some("Saved signature failed content check".into());
            sex_std::record("[pdf.sign.failed] stage=readback");
            return;
        }
        let Ok(pdf) = Pdf::new(back.clone()) else { self.status = Some("Saved signed PDF could not reopen".into()); return; };
        self.editor = Editor::open(back).map_err(|e| e.to_string());
        self.pdf = pdf;
        self.clear_page_cache();
        self.thumbs = vec![None; self.pages()];
        self.texts = None;
        self.field_history.clear();
        self.annotation_history.clear();
        self.recovered_pending = false;
        self.recovery_fingerprint = None;
        let _ = std::fs::remove_file(recovery::path(&path));
        if let Err(error) = sync_result {
            self.status = Some(format!("Signed PDF reopened, but Files could not confirm storage sync: {error:?}"));
            sex_std::record("[pdf.sign.failed] stage=sync");
        } else {
            self.status = Some("Signature content intact; signer trust unverified".into());
            sex_std::record(&format!("[pdf.sign.saved] field={} bytes={} content_intact=1 verified=0", choice.field, bytes.len()));
        }
    }

    fn signature_intact(bytes: &[u8], field: &str) -> bool {
        Editor::open(bytes.to_vec()).ok()
            .and_then(|editor| editor.signature_content_integrity(field).ok())
            == Some(ContentIntegrity::Intact)
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
                self.clear_page_cache();
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
                    ESCAPE => { self.region = None; self.region_drag = None; sex_std::record("[pdf.region] shown=0"); }
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
            let boxes: Vec<[f64; 4]> = texts[hit.page].boxes.iter().skip(hit.at).take(hit.len).map(|b| [b.x0, b.y0, b.x1, b.y1]).collect();
            (hit.page, marks::line_rects(&boxes), "find")
        } else {
            self.status = Some("Find a word (Ctrl+F) or choose a region (Ctrl+R) first".into());
            return;
        };
        let Some(info) = self.page_info(page) else { let _ = self.editor(); return };
        let rects: Vec<[f64; 4]> = rects.iter().map(|r| marks::rect_to_user(&info, *r)).collect();
        let n = rects.len();
        let (color_name, color) = HIGHLIGHT_COLORS[self.highlight_color];
        let (opacity_name, opacity) = HIGHLIGHT_OPACITIES[self.highlight_opacity];
        let annot = NewAnnot { kind: AnnotKind::Highlight { rects }, style: AnnotStyle { color, opacity, date: Some(now_date()), ..AnnotStyle::default() } };
        let Some(editor) = self.editor() else { return };
        match editor.add_annotation(page, &annot) {
            Ok(id) => {
                self.field_history.clear();
                self.annotation_history.record_add(page, annot, id);
                self.region = None;
                self.region_drag = None;
                self.refresh();
                self.status = Some(format!("Highlighted in {color_name} at {opacity_name}. Ctrl+S saves."));
                sex_std::record(&format!("[pdf.annotate] kind=highlight page={} rects={n} from={from} annots={}", page + 1, self.annotation_count()));
            }
            Err(e) => {
                sex_std::record(&format!("[pdf.annotate.refused] action=highlight reason={e}"));
                self.status = Some(format!("Could not highlight: {e}"));
            }
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

    fn comment_key(&mut self, key: sex_std::sex_pdx::app_key::Key) {
        let Some((_, text, _)) = self.comment.as_mut() else { return };
        match key.code {
            ESCAPE => { self.comment = None; self.status = Some("Comment edit cancelled".into()); },
            ENTER => {
                let (index, text, kind) = self.comment.take().unwrap();
                let page = self.page;
                let Some(editor) = self.editor() else { return };
                match editor.set_annotation_text(page, index, &text) {
                    Ok(before) => {
                        self.field_history.clear();
                        self.annotation_history.record_comment(page, index, before.unwrap_or_default(), text.clone());
                        self.load_annots(false);
                        self.status = Some(format!("{kind} text saved. Ctrl+S saves the PDF."));
                        if kind == "Note" {
                            sex_std::record(&format!("[pdf.annotate.note_edit] page={} index={index} chars={}", page + 1, text.chars().count()));
                        } else {
                            sex_std::record(&format!("[pdf.annotate.comment] page={} index={index} chars={}", page + 1, text.chars().count()));
                        }
                    }
                    Err(e) => self.status = Some(format!("Could not edit annotation text: {e}")),
                }
            }
            BACKSPACE => { text.pop(); }
            _ => if let Some(c) = sex_app::text(key).filter(|c| !c.is_control() && !key.control) {
                if text.chars().count() < 400 { text.push(c); }
            },
        }
    }

    fn signature_key(&mut self, key: sex_std::sex_pdx::app_key::Key) {
        let Some(name) = self.signature.as_mut() else { return };
        match key.code {
            ESCAPE => self.signature = None,
            ENTER => {
                let name = self.signature.take().unwrap_or_default();
                if !name.trim().is_empty() { self.add_signature(name.trim().to_string()); }
            }
            BACKSPACE => { name.pop(); }
            _ => if let Some(c) = sex_app::text(key).filter(|c| !c.is_control() && !key.control) {
                if name.chars().count() < 80 { name.push(c); }
            },
        }
    }

    /// Places clearly labelled visible markup. It does not make a CMS signature.
    fn add_signature(&mut self, name: String) {
        let Some(info) = self.page_info(self.page) else { let _ = self.editor(); return };
        let (w, h) = self.pdf.pages().get(self.page).map_or((612.0, 792.0), |p| p.render_dimensions());
        let shown = self.region.unwrap_or([w as f64 * 0.2, h as f64 * 0.78, w as f64 * 0.7, h as f64 * 0.84]);
        let rect = marks::rect_to_user(&info, shown);
        let annot = NewAnnot {
            kind: AnnotKind::FreeText { rect, text: name.clone(), font_size: 20.0 },
            style: AnnotStyle {
                color: [0.08, 0.18, 0.42],
                author: Some(name.clone()),
                contents: Some(format!("Visual signature: {name}")),
                date: Some(now_date()),
                ..AnnotStyle::default()
            },
        };
        let page = self.page;
        let Some(editor) = self.editor() else { return };
        match editor.add_annotation(page, &annot) {
            Ok(id) => {
                self.field_history.clear();
                self.annotation_history.record_add(page, annot, id);
                self.region = None;
                self.region_drag = None;
                self.refresh();
                self.status = Some("Visual signature placed. Ctrl+S saves; no digital signature".into());
                sex_std::record(&format!("[pdf.sign.visual] page={} chars={} annots={}", page + 1, name.chars().count(), self.annotation_count()));
            }
            Err(e) => self.status = Some(format!("Could not place visual signature: {e}")),
        }
    }

    /// A sticky note at the region's top left, else beside the word found, else near the
    /// page's top left.
    fn add_note(&mut self, text: String) {
        let (page, at) = if let Some(r) = self.region {
            (self.page, (r[0], r[1]))
        } else if let (Some(hit), Some(texts)) = (self.hit, &self.texts) {
            let b = texts[hit.page].boxes.get(hit.at + hit.len.saturating_sub(1)).copied().unwrap_or_default();
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
            Ok(id) => {
                self.field_history.clear();
                self.annotation_history.record_add(page, annot, id);
                self.region = None;
                self.region_drag = None;
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
            (_, ENTER) => {
                let Some(annot) = self.annots.get(self.chosen) else { return true };
                let kind = match annot.subtype.as_str() { "Highlight" => Some("Highlight"), "Text" => Some("Note"), _ => None };
                if let Some(kind) = kind {
                    self.comment = Some((annot.index, annot.contents.clone().unwrap_or_default(), kind));
                } else { self.status = Some("Select a highlight or note to edit its text".into()); }
            }
            (Some(Nav::Delete), _) | (_, BACKSPACE) => {
                let Some(a) = self.annots.get(self.chosen).cloned() else { return true };
                let page = self.page;
                let Some(editor) = self.editor() else { return true };
                match editor.remove_annotation(page, a.index) {
                    Ok(removed) => {
                        self.field_history.clear();
                        self.annotation_history.record_delete(page, removed);
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
        match editor.signed_edit_restricted() {
            Ok(true) => {
                self.status = Some("This signed PDF needs permission review before forms can change".into());
                sex_std::record("[pdf.form.refused] reason=signed_revision");
                return;
            }
            Err(e) => {
                self.status = Some(format!("Cannot check form permissions: {e}"));
                sex_std::record("[pdf.form.refused] reason=permissions_unreadable");
                return;
            }
            Ok(false) => {}
        }
        let fields = marks::fillable(&editor.fields().unwrap_or_default());
        if fields.is_empty() {
            self.status = Some("This document has no form fields to fill".into());
            sex_std::record("[pdf.form] fields=0");
            return;
        }
        sex_std::record(&format!("[pdf.form] fields={} first={}", fields.len(), fields[0].0.name));
        self.form = Some(Form { fields, at: 0, text: String::new(), caret: 0, anchor: None, choice: None, popup: false, popup_top: 0 });
        self.focus(0);
    }

    /// Moves to field `at`: its page, scrolled to it, its text ready to edit.
    fn focus(&mut self, at: usize) {
        self.focus_at(at, None);
    }

    /// A pointer already sits on a widget's shown page; keyboard focus uses the first widget.
    fn focus_at(&mut self, at: usize, pointer_page: Option<usize>) {
        self.text_drag = None;
        let Some(form) = self.form.as_mut() else { return };
        form.at = at;
        form.popup = false;
        form.popup_top = 0;
        let Some((f, page, rect)) = form.fields.get(at).cloned() else { return };
        form.text = match &f.value { FieldValue::Text(s) => s.clone(), _ => String::new() };
        form.caret = form.text.chars().count();
        form.anchor = None;
        form.choice = if f.kind == FieldKind::Radio {
            marks::radio_choice(&f)
        } else if matches!(f.kind, FieldKind::ListBox { multi_select: true }) {
            marks::choice_indices(&f).first().copied().or(Some(0))
        } else { marks::choice_index(&f) };
        let choice = form.choice;
        let (shown_page, navigate) = focus_destination(page, pointer_page);
        if navigate {
            self.go(page);
            if let Some(info) = self.page_info(page) { self.reveal = Some(marks::rect_to_shown(&info, rect)[1] as f32); }
        } else { self.reveal = None; }
        sex_std::record(&format!("[pdf.field.focus] name={} index={} page={}", f.name, at + 1, shown_page + 1));
        if matches!(f.kind, FieldKind::ComboBox { .. } | FieldKind::ListBox { .. }) {
            sex_std::record(&format!("[pdf.choice.focus] name={} selected={} count={}", f.name, choice.map_or(0, |i| i + 1), f.options.len()));
        } else if f.kind == FieldKind::Radio {
            sex_std::record(&format!("[pdf.radio.focus] name={} selected={} count={}", f.name, choice.map_or(0, |i| i + 1), f.states.len()));
        }
    }

    /// Sets the focused field to what was typed, if that changed it.
    fn commit_field(&mut self) {
        let Some(form) = self.form.as_ref() else { return };
        let Some((f, _, _)) = form.fields.get(form.at) else { return };
        if matches!(f.kind, FieldKind::ComboBox { editable: false } | FieldKind::ListBox { multi_select: false }) {
            let Some((export, shown)) = form.choice.and_then(|i| f.options.get(i)).cloned() else { return };
            if marks::choice_index(f) == form.choice { return; }
            let name = f.name.clone();
            self.set_field(&name, FieldValue::Text(export), "choice", &shown);
            return;
        }
        if matches!(f.kind, FieldKind::ComboBox { editable: true }) {
            let selected = form.choice.and_then(|i| f.options.get(i)).map(|(export, _)| export.clone());
            let value = selected.unwrap_or_else(|| form.text.clone());
            if f.value == FieldValue::Text(value.clone()) { return; }
            let shown = form.choice.and_then(|i| f.options.get(i)).map(|(_, shown)| shown.clone()).unwrap_or_else(|| value.clone());
            let name = f.name.clone();
            self.set_field(&name, FieldValue::Text(value), "editable-choice", &shown);
            return;
        }
        if !matches!(f.kind, FieldKind::Text { .. }) { return; }
        let now = match &f.value { FieldValue::Text(s) => s.as_str(), _ => "" };
        if now == form.text { return; }
        let (name, text) = (f.name.clone(), form.text.clone());
        self.set_field(&name, FieldValue::Text(text.clone()), "text", &text);
    }

    fn set_field(&mut self, name: &str, value: FieldValue, kind: &str, shown: &str) {
        let result = {
            let Some(editor) = self.editor() else { return };
            let before = editor.fields().ok().and_then(|fields| fields.into_iter().find(|f| f.name == name)).map(|f| f.value);
            editor.set_field(name, value.clone()).map(|()| (before, marks::fillable(&editor.fields().unwrap_or_default())))
        };
        match result {
            Ok((before, fields)) => {
                if let Some(before) = before { self.field_history.record(name, before, value); }
                self.annotation_history.clear();
                if let Some(form) = self.form.as_mut() {
                    form.fields = fields;
                    if let Some((field, _, _)) = form.fields.get(form.at) {
                        if field.kind == FieldKind::Radio {
                            form.choice = marks::radio_choice(field);
                            sex_std::record(&format!("[pdf.radio.current] name={} selected={}", field.name, form.choice.map_or(0, |i| i + 1)));
                        }
                    }
                }
                self.refresh();
                sex_std::record(&format!("[pdf.field] name={name} kind={kind} value={shown}"));
            }
            Err(e) => self.status = Some(format!("Could not fill {name}: {e}")),
        }
    }

    fn field_history_step(&mut self, redo: bool) {
        self.commit_field();
        let result = match self.editor.as_mut() {
            Ok(editor) => self.field_history.step(editor, redo),
            Err(why) => { self.status = Some(format!("This file cannot be edited: {why}")); return; }
        };
        match result {
            Ok(Some(name)) => {
                let fields = self.editor.as_ref().ok().map(|e| marks::fillable(&e.fields().unwrap_or_default())).unwrap_or_default();
                if let Some(form) = self.form.as_mut() {
                    form.fields = fields;
                    if let Some((field, _, _)) = form.fields.get(form.at) {
                        form.text = match &field.value { FieldValue::Text(s) => s.clone(), _ => String::new() };
                        form.caret = form.text.chars().count();
                        form.anchor = None;
                        form.choice = if field.kind == FieldKind::Radio {
                            marks::radio_choice(field)
                        } else if matches!(field.kind, FieldKind::ListBox { multi_select: true }) {
                            marks::choice_indices(field).first().copied().or(Some(0))
                        } else { marks::choice_index(field) };
                        if field.kind == FieldKind::Radio {
                            sex_std::record(&format!("[pdf.radio.history] name={} selected={}", field.name, form.choice.map_or(0, |i| i + 1)));
                        }
                    }
                }
                self.refresh();
                self.status = Some(format!("{} form field {name}. Ctrl+S saves.", if redo { "Redid" } else { "Undid" }));
                sex_std::record(&format!("[pdf.field.history] action={} name={name}", if redo { "redo" } else { "undo" }));
            }
            Ok(None) => self.status = Some(format!("No form field edit to {}", if redo { "redo" } else { "undo" })),
            Err(e) => self.status = Some(format!("Could not {} form field: {e}", if redo { "redo" } else { "undo" })),
        }
    }

    fn annotation_history_step(&mut self, redo: bool) {
        let result = match self.editor.as_mut() {
            Ok(editor) => self.annotation_history.step(editor, redo),
            Err(why) => { self.status = Some(format!("This file cannot be edited: {why}")); return; }
        };
        match result {
            Ok(Some(page)) => {
                self.refresh();
                if self.panel == Panel::Annots { self.load_annots(false); }
                self.status = Some(format!("{} markup change. Ctrl+S saves.", if redo { "Redid" } else { "Undid" }));
                sex_std::record(&format!("[pdf.annotate.history] action={} page={} annots={}",
                    if redo { "redo" } else { "undo" }, page + 1, self.annotation_count()));
            }
            Ok(None) => self.status = Some(format!("No markup change to {}", if redo { "redo" } else { "undo" })),
            Err(e) => self.status = Some(format!("Could not {} markup: {e}", if redo { "redo" } else { "undo" })),
        }
    }

    fn end_form(&mut self) {
        self.commit_field();
        self.form = None;
        self.text_drag = None;
        sex_std::record("[pdf.form] done=1");
    }

    fn form_key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        let Some(form) = self.form.as_mut() else { return false };
        let n = form.fields.len().max(1);
        let checkbox = form.fields.get(form.at).is_some_and(|f| f.0.kind == FieldKind::Checkbox);
        let radio = form.fields.get(form.at).is_some_and(|f| f.0.kind == FieldKind::Radio);
        let choice_kind = form.fields.get(form.at).map(|f| f.0.kind.clone());
        let choices = form.fields.get(form.at).and_then(|f| matches!(f.0.kind,
            FieldKind::ComboBox { .. } | FieldKind::ListBox { .. }).then_some(f.0.options.len()));
        if radio && !key.control {
            if let Some(backward) = match sex_app::nav(key) { Some(Nav::Up) => Some(true), Some(Nav::Down) => Some(false), _ => None } {
                if let Some((f, _, _)) = form.fields.get(form.at) {
                    form.choice = marks::step_choice(f.states.len(), form.choice, backward);
                    sex_std::record(&format!("[pdf.radio.choice] name={} selected={} count={}", f.name, form.choice.map_or(0, |i| i + 1), f.states.len()));
                }
                return true;
            }
        }
        if let Some(count) = choices {
            if !key.control {
                if let Some(backward) = match sex_app::nav(key) { Some(Nav::Up) => Some(true), Some(Nav::Down) => Some(false), _ => None } {
                    form.choice = marks::step_choice(count, form.choice, backward);
                    if let Some((f, _, _)) = form.fields.get(form.at) {
                        if let Some((export, shown)) = form.choice.and_then(|i| f.options.get(i)) {
                            if matches!(choice_kind, Some(FieldKind::ComboBox { editable: true })) { form.text = export.clone(); }
                            form.caret = form.text.chars().count();
                            form.anchor = None;
                            sex_std::record(&format!("[pdf.choice] name={} selected={} count={} display={}", f.name, form.choice.unwrap_or(0) + 1, count, shown));
                        }
                    }
                    return true;
                }
            }
        }
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
        let text_field = !checkbox && !radio && (choices.is_none() || matches!(choice_kind, Some(FieldKind::ComboBox { editable: true })));
        if text_field && !key.control {
            let length = form.text.chars().count();
            let selected = text_selection(form.anchor, form.caret);
            let moved = match sex_app::nav(key) {
                Some(Nav::Left) => {
                    if key.shift { form.anchor.get_or_insert(form.caret); }
                    form.caret = if key.shift { form.caret.saturating_sub(1) }
                        else { selected.map_or(form.caret.saturating_sub(1), |(start, _)| start) };
                    if !key.shift { form.anchor = None; }
                    true
                }
                Some(Nav::Right) => {
                    if key.shift { form.anchor.get_or_insert(form.caret); }
                    form.caret = if key.shift { (form.caret + 1).min(length) }
                        else { selected.map_or((form.caret + 1).min(length), |(_, end)| end) };
                    if !key.shift { form.anchor = None; }
                    true
                }
                Some(Nav::Home) => { if key.shift { form.anchor.get_or_insert(form.caret); } else { form.anchor = None; } form.caret = 0; true }
                Some(Nav::End) => { if key.shift { form.anchor.get_or_insert(form.caret); } else { form.anchor = None; } form.caret = length; true }
                Some(Nav::Delete) => {
                    if !replace_text_selection(&mut form.text, &mut form.caret, &mut form.anchor) {
                        text_delete(&mut form.text, form.caret);
                    }
                    form.choice = None;
                    true
                }
                _ => false,
            };
            if moved {
                if let Some((start, end)) = text_selection(form.anchor, form.caret) {
                    sex_std::record(&format!("[pdf.form.selection] start={start} end={end} chars={}", form.text.chars().count()));
                }
                sex_std::record(&format!("[pdf.form.caret] at={} chars={}", form.caret, form.text.chars().count()));
                return true;
            }
        }
        match key.code {
            ESCAPE => { self.end_form(); return true; }
            BACKSPACE if text_field => {
                if !replace_text_selection(&mut form.text, &mut form.caret, &mut form.anchor) {
                    text_backspace(&mut form.text, &mut form.caret);
                }
                form.choice = None; return true;
            }
            BACKSPACE if choices.is_some() => return true,
            _ => {}
        }
        if matches!(choice_kind, Some(FieldKind::ComboBox { editable: true })) && key.control && key.code == sex_std::sex_pdx::app_key::A {
            form.text.clear(); form.caret = 0; form.anchor = None; form.choice = None; return true;
        }
        if text_field && key.control && key.code == sex_std::sex_pdx::app_key::A {
            form.anchor = Some(0); form.caret = form.text.chars().count();
            sex_std::record(&format!("[pdf.form.selection] start=0 end={} chars={}", form.caret, form.caret));
            return true;
        }
        let Some(c) = sex_app::text(key).filter(|c| !c.is_control() && !key.control) else { return false };
        if radio {
            if c == ' ' {
                let Some((f, _, _)) = form.fields.get(form.at).cloned() else { return true };
                let Some(state) = form.choice.and_then(|i| f.states.get(i)).cloned() else { return true };
                self.set_field(&f.name, FieldValue::State(state.clone()), "radio", &state);
            }
        } else if checkbox {
            if c == ' ' {
                let Some((f, _, _)) = form.fields.get(form.at).cloned() else { return true };
                let to = marks::toggled(&f);
                self.set_field(&f.name, FieldValue::State(to.clone()), "checkbox", &to);
            }
        } else if matches!(choice_kind, Some(FieldKind::ListBox { multi_select: true })) {
            if c == ' ' {
                let Some((f, _, _)) = form.fields.get(form.at).cloned() else { return true };
                let Some(i) = form.choice else { return true };
                let Some(values) = marks::toggle_choice(&f, i) else { return true };
                let count = values.len();
                self.set_field(&f.name, FieldValue::Choices(values), "multi-choice", &format!("{count} selected"));
            }
            return true;
        } else if choices.is_some() && !matches!(choice_kind, Some(FieldKind::ComboBox { editable: true })) {
            return true;
        } else if text_field {
            form.choice = None;
            replace_text_selection(&mut form.text, &mut form.caret, &mut form.anchor);
            text_insert(&mut form.text, &mut form.caret, c);
        }
        true
    }

    /// Ctrl+S: every change appended to the file as one incremental update, then the
    /// file read back and shown as it now is.
    fn save(&mut self) {
        let started = Instant::now();
        self.commit_field();
        let path = self.path.clone();
        let recovered_pending = self.recovered_pending;
        let Some(editor) = self.editor() else { return };
        if !editor.is_modified() && !recovered_pending {
            self.status = Some("Nothing to save".into());
            sex_std::record("[pdf.saved] changed=0");
            return;
        }
        let original = editor.original().to_vec();
        let bytes = match editor.save_incremental() {
            Ok(b) => b,
            Err(e) => { self.status = Some(format!("Could not save: {e}")); sex_std::record(&format!("[pdf.save.failed] {e}")); return; }
        };
        if let Err(e) = thumb::write_confirmed(&path, &bytes, sex_std::sex_app::files::direct::sync) {
            self.status = Some(format!("Could not save: {e}"));
            sex_std::record(&format!("[pdf.save.failed] {e}"));
            return;
        }
        sex_std::record(&format!("[pdf.save.synced] path={path} bytes={}", bytes.len()));
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
        self.recovered_pending = false;
        self.recovery_fingerprint = None;
        let _ = std::fs::remove_file(recovery::path(&path));
        let _ = sex_std::sex_app::files::direct::sync();
        self.field_history.clear();
        self.annotation_history.clear();
        self.pdf = pdf;
        self.clear_page_cache();
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

    /// Keep unsaved markup and committed form values as a complete incremental
    /// PDF after a quiet interval. The source file is left intact until Ctrl+S.
    fn keep_recovery(&mut self) {
        let Ok(editor) = &self.editor else { return };
        if !editor.is_modified() { return; }
        let bytes = match editor.save_incremental() {
            Ok(bytes) => bytes,
            Err(e) => { sex_std::record(&format!("[pdf.recovery.failed] {e}")); return; }
        };
        let fingerprint = recovery::fingerprint(&bytes);
        if self.recovery_fingerprint == Some(fingerprint) { return; }
        let name = recovery::path(&self.path);
        match thumb::write_whole(&name, &bytes) {
            Ok(()) => {
                match sex_std::sex_app::files::direct::sync() {
                    Ok(()) => {
                        self.recovery_fingerprint = Some(fingerprint);
                        sex_std::record(&format!("[pdf.recovery.kept] name={name} bytes={}", bytes.len()));
                    }
                    Err(e) => sex_std::record(&format!("[pdf.recovery.failed] sync={e:?}")),
                }
            }
            Err(e) => sex_std::record(&format!("[pdf.recovery.failed] {e}")),
        }
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

#[cfg(test)]
mod render_cache_tests {
    use super::*;

    #[test]
    fn form_caret_edits_unicode_at_character_boundaries() {
        let mut text = String::from("naïve");
        let mut caret = 2;
        assert_eq!(text_with_caret(&text, caret), "na|ïve");
        text_delete(&mut text, caret);
        assert_eq!(text, "nave");
        text_insert(&mut text, &mut caret, 'é');
        assert_eq!(text, "naéve");
        assert_eq!(caret, 3);
        text_backspace(&mut text, &mut caret);
        assert_eq!(text, "nave");
        assert_eq!(caret, 2);
        text_backspace(&mut text, &mut caret);
        assert_eq!(text, "nve");
        assert_eq!(caret, 1);
        let mut full = "x".repeat(200);
        text_insert(&mut full, &mut caret, 'z');
        assert_eq!(full.chars().count(), 200);
    }

    #[test]
    fn form_selection_replaces_forward_and_reverse_unicode_spans() {
        let mut text = String::from("Ada naïve");
        let mut caret = 4;
        let mut anchor = Some(9);
        assert_eq!(text_selection(anchor, caret), Some((4, 9)));
        assert_eq!(text_display(&text, caret, anchor), "Ada naïve");
        assert!(replace_text_selection(&mut text, &mut caret, &mut anchor));
        assert_eq!((text.as_str(), caret, anchor), ("Ada ", 4, None));
        text_insert(&mut text, &mut caret, 'é');
        assert_eq!((text.as_str(), caret), ("Ada é", 5));
        anchor = Some(0);
        assert!(replace_text_selection(&mut text, &mut caret, &mut anchor));
        assert_eq!((text.as_str(), caret, anchor), ("", 0, None));
    }

    #[test]
    fn form_bar_click_places_caret_at_nearest_character_boundary() {
        let origin = 62;
        assert_eq!(text_caret_at_x(0, origin, 12), 0);
        assert_eq!(text_caret_at_x(origin + 5 * CW + 2, origin, 12), 5);
        assert_eq!(text_caret_at_x(origin + 5 * CW + 5, origin, 12), 6);
        assert_eq!(text_caret_at_x(usize::MAX, origin, 12), 12);
    }

    #[test]
    fn form_bar_reverse_drag_selects_unicode_characters_for_replacement() {
        let mut text = String::from("Ada naïve");
        let vx = 62;
        let anchor = text_caret_at_x(vx + 9 * CW, vx, text.chars().count());
        let mut caret = text_caret_at_x(vx + 4 * CW, vx, text.chars().count());
        let mut selected = Some(anchor);
        assert_eq!(text_selection(selected, caret), Some((4, 9)));
        assert!(replace_text_selection(&mut text, &mut caret, &mut selected));
        text_insert(&mut text, &mut caret, 'é');
        assert_eq!((text.as_str(), caret, selected), ("Ada é", 5, None));
    }

    #[test]
    fn dragged_region_normalizes_reverse_drag_and_stays_on_page() {
        assert_eq!(dragged_region((160.0, 140.0), (40.0, 70.0), (200.0, 180.0)),
            [40.0, 70.0, 160.0, 140.0]);
        assert_eq!(dragged_region((199.0, 179.0), (199.0, 179.0), (200.0, 180.0)),
            [194.0, 174.0, 200.0, 180.0]);
        assert_eq!(dragged_region((1.0, 1.0), (1.0, 1.0), (4.0, 3.0)),
            [0.0, 0.0, 4.0, 3.0]);
    }

    #[test]
    fn shared_cache_keeps_thumbnail_pixels_identical() {
        let pdf = Pdf::new(include_bytes!("../../../tests/fixtures/pdf/find10.pdf").to_vec()).unwrap();
        let cache = RenderCache::new();
        for page in 0..2 {
            let separate = render_page(&pdf, page, 0.2).unwrap();
            let shared = render_page_cached(&pdf, &cache, page, 0.2).unwrap();
            assert_eq!(shared, separate, "page {page}");
        }
    }

    #[test]
    fn choice_menu_clips_rows_and_rejects_outside_clicks() {
        let menu = choice_menu(640, 180, BAR, 100).unwrap();
        assert_eq!(menu, (8, BAR, CHOICE_MENU_WIDTH, 7));
        assert_eq!(choice_menu_hit(menu, 20, BAR + 2, 91, 100), Some(91));
        assert_eq!(choice_menu_hit(menu, 20, BAR + 6 * ROW + 2, 91, 100), Some(97));
        assert_eq!(choice_menu_hit(menu, 20, BAR + 7 * ROW, 91, 100), None);
        assert_eq!(choice_menu_hit(menu, 400, BAR + 2, 91, 100), None);
        assert!(choice_menu(70, 180, BAR, 3).is_none());
        assert_eq!(scroll_choice_top(0, 100, 7, -100), 93);
        assert_eq!(scroll_choice_top(93, 100, 7, 100), 0);
    }

    #[test]
    fn widget_click_converts_centering_zoom_and_scroll() {
        let l = Layout { panel: PANEL, bar: BAR, x0: PANEL, y0: BAR, aw: 600, ah: 500 };
        let r = Rendered { pixels: Vec::new(), w: 400, h: 600, page: 0, scale: 2.0, ink: 0 };
        let left = PANEL + (600 - 400) / 2;
        assert_eq!(shown_at_click((left + 200, BAR + MARGIN + 100), &l, &r, 0, 0), Some((100.0, 50.0)));
        assert_eq!(shown_at_click((left + 200, BAR + MARGIN + 20), &l, &r, 80, 0), Some((100.0, 50.0)));
        let wide = Rendered { pixels: Vec::new(), w: 800, h: 600, page: 0, scale: 2.0, ink: 0 };
        assert_eq!(shown_at_click((PANEL + MARGIN + 152, BAR + MARGIN + 20), &l, &wide, 80, 60), Some((100.0, 50.0)));
        assert_eq!(shown_at_click((PANEL - 1, BAR + MARGIN + 100), &l, &r, 0, 0), None);
        assert_eq!(shown_at_click((left + 200, BAR - 1), &l, &r, 0, 0), None);
        assert_eq!(focus_destination(0, Some(1)), (1, false), "a repeated widget keeps its clicked page");
        assert_eq!(focus_destination(0, None), (0, true), "keyboard focus navigates to the first widget");
    }
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
    let (source_bytes, _) = match open(&name) {
        Ok(opened) => opened,
        Err(message) => {
            // Said in the window as text; closing it ends the program.
            println!("{message}\n\nPut a PDF in this program's folder and run: pdf NAME.pdf");
            std::process::exit(1);
        }
    };
    let kept_name = recovery::path(&name);
    let recovered = std::fs::read(&kept_name).ok().filter(|kept| {
        recovery::belongs_to(&source_bytes, kept)
            && Pdf::new(kept.clone()).is_ok()
            && Editor::open(kept.clone()).is_ok()
    });
    let recovered_pending = recovered.is_some();
    let bytes = recovered.unwrap_or(source_bytes);
    let pdf = Pdf::new(bytes.clone()).expect("opened or validated recovery PDF");
    // A file the person gave (`@/home/x.pdf`) is named by its last part.
    let short = name.rsplit('/').next().unwrap_or(&name).to_string();
    let (target, task) = sex_std::identity();
    let editor = Editor::open(bytes).map_err(|e| e.to_string());
    sex_std::record(&format!("[pdf.open] target={target} task={task} name={short} pages={}", pdf.pages().len()));
    if let Err(why) = &editor { sex_std::record(&format!("[pdf.edit.refused] {why}")); }
    let window = sex_std::window();
    let theme = sex_app::settings::theme();
    let palette = Palette::from_tokens(theme);
    let theme_generation = sex_app::settings::generation().unwrap_or(0);
    sex_std::record(&format!("[pdf.theme] mode={:?} accent={:06x}", theme.mode, palette.accent));
    let pages = pdf.pages().len();
    let mut viewer = Viewer {
        palette, theme_generation,
        pdf, name: short, page: 0, zoom: Zoom::FitWidth, scroll: 0, hscroll: 0, shown: None, page_cache: Vec::new(), cache_bytes: 0,
        panel: Panel::None, thumbs: vec![None; pages], thumb_top: 0, outline: None, chosen: 0,
        finding: false, term: String::new(), searched: String::new(), hits: Vec::new(), hit: None, found_none: false,
        texts: None, reveal: None, letters: Letters::new(),
        path: name, editor, recovered_pending, recovery_fingerprint: None,
        field_history: FieldHistory::default(), annotation_history: AnnotationHistory::default(), highlight_color: 0, highlight_opacity: 0, region: None, region_drag: None, note: None, comment: None, signature: None, signing: None, close_after_sign: false, form: None, text_drag: None, annots: Vec::new(),
        status: recovered_pending.then(|| "Brought back unsaved changes. Ctrl+S saves them to the PDF.".into()),
        shortcuts: false, shortcuts_top: 0,
    };
    if recovered_pending { sex_std::record(&format!("[pdf.recovery.restored] name={kept_name} bytes={}", viewer.editor.as_ref().map_or(0, |e| e.original().len()))); }
    viewer.draw(window);
    let mut theme_next = sex_std::sex_pdx::get_ticks().saturating_add(1000);
    let mut recovery_next = sex_std::sex_pdx::get_ticks().saturating_add(3000);
    loop {
        let now = sex_std::sex_pdx::get_ticks();
        if now >= recovery_next {
            recovery_next = now.saturating_add(3000);
            viewer.keep_recovery();
        }
        if now >= theme_next {
            theme_next = now.saturating_add(1000);
            if let Ok(generation) = sex_app::settings::generation() {
                if generation != viewer.theme_generation {
                    viewer.theme_generation = generation;
                    let theme = sex_app::settings::theme();
                    viewer.palette = Palette::from_tokens(theme);
                    sex_std::record(&format!("[pdf.theme] mode={:?} accent={:06x}", theme.mode, viewer.palette.accent));
                    viewer.draw(window);
                }
            }
        }
        let changed = match window.next_within(1000) {
            Some(Event::Close) => { viewer.keep_recovery(); break },
            Some(Event::Resize { .. }) => true,
            Some(Event::Key(key)) => {
                let changed = viewer.key(key, window);
                if viewer.close_after_sign { viewer.keep_recovery(); break; }
                if viewer.panel == Panel::Annots { viewer.load_annots(false); }
                changed
            }
            Some(Event::Pointer(p)) => viewer.pointer(p, window.size()),
            Some(Event::Drop { .. } | Event::DragEnded { .. } | Event::KeyUp(_)) | None => false,
        };
        if changed { viewer.draw(window); }
    }
    sex_std::record("[pdf.closed]");
    std::process::exit(0);
}
