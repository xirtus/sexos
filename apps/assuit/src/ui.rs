// SPDX-License-Identifier: GPL-3.0-only
//! The page view: WordCraft draws the pages, this stacks them on a grey desk and
//! scrolls through them, with the caret or the selection's highlight over them and a
//! status line, and turns SexOS keys into WordCraft's shortcuts. Only pages on screen
//! are drawn, and only again after an edit. WordCraft's egui chrome (ribbon, rulers,
//! panes) follows with M11.
use crate::book::{Book, Chord, Picture, TableStyleChoice};
use sex_office::{Canvas, Pen};
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx::app_key::Key;
use sex_std::sex_pdx::app_pointer::{Kind, Pointer};
use sex_theme::Tokens;

#[derive(Clone, Copy)]
struct Palette {
    desk: u32, shadow: u32, caret: u32, bar_bg: u32,
    dim: u32, ink: u32, line: u32, selected: u32, on_selected: u32,
}

impl Palette {
    fn from_tokens(t: Tokens) -> Self {
        let rgb = |c: u32| c & 0x00FF_FFFF;
        Self {
            desk: rgb(t.window), shadow: rgb(t.border),
            // Document pages are rendered on white paper in every UI theme.
            caret: 0x00000000,
            bar_bg: rgb(t.surface), dim: rgb(t.text_dim), ink: rgb(t.text), line: rgb(t.border),
            selected: rgb(t.accent), on_selected: rgb(t.on_accent),
        }
    }
}
const STATUS_H: i32 = 26;
const MARGIN: i32 = 16;

pub struct Ui {
    palette: Palette,
    theme_generation: u64,
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
    surface_size: (i32, i32),
    /// The Ctrl+F or Ctrl+H prompt, while it is open.
    find: Option<Prompt>,
    /// The Ctrl+Shift+S style list, while it is open: (id, name) and the highlighted row.
    picker: Option<(Vec<(String, String)>, usize)>,
    /// Native samples for the named table styles in this DOCX document.
    table_gallery: Option<(Vec<TableStyleChoice>, usize)>,
    /// The Save As or Go To prompt, while it is open.
    ask: Option<Ask>,
    /// Ctrl+Shift+Y's selected issue and its suggestion list.
    proof: Option<Proof>,
    /// The app-owned custom word list and optional word being added.
    dictionary: Option<Dictionary>,
    /// An in-app shortcut reference, opened with Ctrl+Shift+/.
    help: bool,
    /// The document came back with last time's unsaved changes: Esc throws them away until
    /// the next edit.
    recovered: bool,
    /// When the document last changed without its changes being kept (ticks).
    unkept: Option<u64>,
    /// The page Go To asked for, to scroll to on the next draw.
    scroll_to: Option<usize>,
    /// One read-only page fitted to the window before printing.
    preview_page: Option<usize>,
}

/// A one-line prompt in the status line.
enum Ask {
    /// Ctrl+Shift+E: a file name; its extension picks the format.
    SaveAs(String),
    /// Ctrl+G: a page number.
    GoTo(String),
    /// Ctrl+T: dimensions such as 2x3.
    Table(String),
    /// Ctrl+Shift+T: edit the table cell containing the caret.
    TableEdit(String),
    /// Ctrl+Alt+L: choose a list kind, remove it or restart numbering.
    List(String),
    /// Ctrl+Shift+I: image path in the program's granted folder.
    Picture(String),
    /// Ctrl+Shift+R: alternative text for the selected image.
    PictureAlt(String),
    /// Ctrl+Shift+O: selected picture width in inches.
    PictureWidth(String),
    /// Ctrl+Shift+C: four crop percentages for the selected picture.
    PictureCrop(String),
    /// Ctrl+Shift+P: a 1-based PDF page range such as `1-3,5`.
    PrintRange(String),
    /// Ctrl+Shift+L: paper size and orientation, for example `A4 landscape`.
    Paper(String),
    /// Ctrl+Shift+M: a margin preset or four inch values.
    Margins(String),
    /// Ctrl+Alt+N: page number location and alignment.
    PageNumbers(String),
    /// Ctrl+Alt+S: line multiple, space before and after in points.
    Spacing(String),
    /// Ctrl+Shift+K: document proofing language.
    Language(String),
}

/// Unsaved changes are copied aside after this long without a key (milliseconds).
const KEEP_AFTER_MS: u64 = 15_000;

/// Ctrl+F asks what to find; Ctrl+H also asks what to put in its place.
struct Prompt {
    what: String,
    with: Option<String>,
    /// Typing goes into `with` (Tab or Enter moves there from `what`).
    on_with: bool,
}

struct Proof { word: String, suggestions: Vec<String>, at: usize }
struct Dictionary { words: Vec<String>, at: usize, adding: Option<String> }

impl Prompt {
    /// The text being typed into.
    fn field(&mut self) -> &mut String {
        match &mut self.with { Some(with) if self.on_with => with, _ => &mut self.what }
    }
}

const GAP: i32 = 12;
const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+S", "Save"), ("Ctrl+Shift+E", "Save as / choose format"),
    ("Ctrl+P", "Export PDF"), ("Ctrl+Shift+P", "Print selected pages"),
    ("Ctrl+Alt+P", "Preview pages before printing"),
    ("Ctrl+Shift+L", "Paper size and orientation"),
    ("Ctrl+Shift+M", "Page margins"),
    ("Ctrl+Alt+N", "Add page numbers to header or footer"),
    ("Ctrl+Alt+S", "Paragraph line and before/after spacing"),
    ("Ctrl+T", "Insert table"),
    ("Ctrl+Shift+T", "Edit table; enter style to browse DOCX table styles"),
    ("Ctrl+Alt+L", "Lists: formats, level 1–9, off or restart"),
    ("Ctrl+Alt+Right / Left", "Move one list level deeper / shallower"),
    ("Ctrl+Shift+I", "Insert picture"), ("Ctrl+Shift+R", "Picture alt text"),
    ("Ctrl+Shift+O", "Picture width"),
    ("Ctrl+Shift+C", "Crop selected picture"),
    ("Ctrl+Shift+S", "Apply paragraph style"),
    ("Ctrl+B / I / U", "Bold / italic / underline"),
    ("Ctrl+Z / Y", "Undo / redo"), ("Ctrl+A", "Select all"),
    ("Ctrl+C / X / V", "Shared clipboard copy / cut / paste"),
    ("Ctrl+F / H", "Find / replace"),
    ("Ctrl+G", "Go to page"), ("Ctrl+Shift+Y", "Spelling suggestions"),
    ("Ctrl+Shift+K", "Proofing language"),
    ("Ctrl+Shift+U", "Show or hide spelling marks"),
    ("Ctrl+Shift+D", "Manage custom dictionary"),
    ("Ctrl+= / - / 0", "Zoom in / out / reset"),
    ("Ctrl+Shift+/", "Show or hide this reference"),
];

/// Set-1 scan codes Shell passes.
mod code {
    pub const ESC: u8 = 0x01;
    pub const BACKSPACE: u8 = 0x0E;
    pub const TAB: u8 = 0x0F;
    pub const ENTER: u8 = 0x1C;
    pub const S: u8 = 0x1F;
    pub const F: u8 = 0x21;
    pub const H: u8 = 0x23;
    pub const E: u8 = 0x12;
    pub const G: u8 = 0x22;
    pub const P: u8 = 0x19;
    pub const T: u8 = 0x14;
    pub const I: u8 = 0x17;
    pub const R: u8 = 0x13;
    pub const O: u8 = 0x18;
    pub const C: u8 = 0x2E;
    pub const X: u8 = 0x2D;
    pub const V: u8 = 0x2F;
    pub const Y: u8 = 0x15;
    pub const U: u8 = 0x16;
    pub const L: u8 = 0x26;
    pub const M: u8 = 0x32;
    pub const N: u8 = 0x31;
    pub const K: u8 = 0x25;
    pub const D: u8 = 0x20;
    pub const A: u8 = 0x1E;
    pub const SLASH: u8 = 0x35;
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
        let theme = sex_app::settings::theme();
        let palette = Palette::from_tokens(theme);
        let theme_generation = sex_app::settings::generation().unwrap_or(0);
        sex_std::record(&format!("[assuit.theme] mode={:?} accent={:06x}", theme.mode, palette.selected));
        Ui { palette, theme_generation, book, name, zoom: 1.0, scroll: 0, shown: Vec::new(), message: String::new(), pen: Pen::sans(13), places: Vec::new(), surface_size: (0, 0), find: None, picker: None, table_gallery: None, ask: None, proof: None, dictionary: None, help: false, recovered: false, unkept: None, scroll_to: None, preview_page: None }
    }

    fn scale(&mut self, page: usize, width: i32) -> f32 {
        let (pw, _) = self.book.page_size(page);
        ((width - 2 * MARGIN).max(64) as f32 / pw.max(1.0) * self.zoom).clamp(0.1, 6.0)
    }

    /// The pages change after an edit: draw them again.
    fn invalidate(&mut self) { self.shown.clear(); }

    fn gallery_bounds(w: i32, view_h: i32, count: usize, at: usize) -> (i32, i32, i32, usize, usize, i32) {
        let row = 44;
        let shown = count.min(((view_h - 2 * MARGIN - 63) / row).max(1) as usize);
        let first = (at + 1).saturating_sub(shown);
        let bw = (w - 2 * MARGIN).min(450).max(1);
        let bh = 63 + shown as i32 * row;
        ((w - bw) / 2, ((view_h - bh) / 2).max(0), bw, shown, first, row)
    }

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
        if let Some(page) = self.preview_page { return self.draw_preview(window, page); }
        let palette = self.palette;
        let (width, height) = window.size();
        let (w, h) = (width as i32, height as i32);
        self.surface_size = (w, h);
        let view_h = h - STATUS_H;
        self.place(w);
        // Go To: the page's top at the top of the view.
        if let Some(&(top, ..)) = self.scroll_to.take().and_then(|p| self.places.get(p)) { self.scroll = top - MARGIN; }
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
        canvas.fill(0, 0, w, h, palette.desk);
        let clip = (0, 0, w, view_h);
        for (page, left, y, pw, ph, scale) in drawn {
            canvas.fill(left + 2, y + 2, pw, ph, palette.shadow);
            if let Some((_, picture)) = &self.shown[page] { canvas.blit(left, y, &picture.pixels, picture.width, clip); }
            for &(_, rx, ry, rw, rh) in selection.iter().filter(|r| r.0 == page) {
                let (x0, y0) = (left + (rx * scale) as i32, y + (ry * scale) as i32);
                let (x1, y1) = (left + ((rx + rw) * scale).ceil() as i32, (y + ((ry + rh) * scale).ceil() as i32).min(view_h));
                canvas.tint(x0, y0, x1 - x0, y1 - y0, palette.selected, 70);
            }
            if let Some(c) = caret.filter(|c| c.page == page && selection.is_empty()) {
                let cx = left + (c.x * scale) as i32;
                let cy = y + (c.top * scale) as i32;
                let ch = ((c.height * scale) as i32).max(8);
                let (top, bottom) = (cy.max(0), (cy + ch).min(view_h));
                if bottom > top { canvas.fill(cx, top, 2, bottom - top, palette.caret); }
            }
        }
        if let Some((styles, at)) = &self.picker {
            let row = self.pen.line_height() + 8;
            // As many rows as fit, scrolled so the highlighted one shows.
            let fit = ((view_h - 2 * MARGIN - 34) / row).max(1) as usize;
            let shown = styles.len().min(fit);
            let first = (at + 1).saturating_sub(shown);
            let (bw, bh) = (240, row * shown as i32 + 34);
            let (bx, by) = (MARGIN, (view_h - bh - MARGIN).max(MARGIN.min(view_h - bh)));
            canvas.fill(bx + 3, by + 3, bw, bh, palette.shadow);
            canvas.fill(bx, by, bw, bh, palette.bar_bg);
            canvas.frame(bx, by, bw, bh, palette.line);
            self.pen.text(&mut canvas, bx + 10, by + 8, "Apply a style (Enter, Esc)", palette.dim, Some((bx, bw - 10)));
            for (i, (_, name)) in styles.iter().enumerate().skip(first).take(shown) {
                let y = by + 30 + (i - first) as i32 * row;
                if i == *at { canvas.fill(bx + 4, y - 2, bw - 8, row, palette.selected); }
                self.pen.text(&mut canvas, bx + 12, y + 2, name, if i == *at { palette.on_selected } else { palette.ink }, Some((bx, bw - 12)));
            }
        }
        if let Some((styles, at)) = &self.table_gallery {
            let (bx, by, bw, shown, first, row) = Self::gallery_bounds(w, view_h, styles.len(), *at);
            let bh = 63 + shown as i32 * row;
            canvas.fill(bx + 3, by + 3, bw, bh, palette.shadow);
            canvas.fill(bx, by, bw, bh, palette.bar_bg);
            canvas.frame(bx, by, bw, bh, palette.line);
            self.pen.text(&mut canvas, bx + 12, by + 8, "Assuit table styles", palette.ink, Some((bx, bw - 24)));
            for (slot, style) in styles.iter().skip(first).take(shown).enumerate() {
                let y = by + 33 + slot as i32 * row;
                if first + slot == *at { canvas.fill(bx + 5, y - 3, bw - 10, row - 2, palette.selected); }
                for r in 0..3 {
                    for c in 0..3 {
                        let x = bx + 14 + c * 22;
                        let cy = y + r * 11;
                        let color = if r == 0 { style.header } else if r == 1 { style.band } else { style.body };
                        canvas.fill(x, cy, 21, 10, color);
                        if style.borders { canvas.frame(x, cy, 21, 10, palette.line); }
                    }
                }
                self.pen.text(&mut canvas, bx + 92, y + 8, &style.name,
                    if first + slot == *at { palette.on_selected } else { palette.ink }, Some((bx + 92, bw - 105)));
            }
            self.pen.text(&mut canvas, bx + 12, by + bh - 25,
                "Up/Down choose  ·  Enter or click apply  ·  Esc close", palette.dim, Some((bx, bw - 24)));
        }
        if self.help {
            let row = self.pen.line_height() + 6;
            let bw = (w - 2 * MARGIN).min(520).max(0);
            let bh = (row * (SHORTCUTS.len() as i32 + 2)).min(view_h - 2 * MARGIN);
            if bw > 0 && bh > 0 {
                let bx = (w - bw) / 2;
                let by = ((view_h - bh) / 2).max(0);
                canvas.fill(bx + 3, by + 3, bw, bh, palette.shadow);
                canvas.fill(bx, by, bw, bh, palette.bar_bg);
                canvas.frame(bx, by, bw, bh, palette.line);
                self.pen.text(&mut canvas, bx + 12, by + 8, "Assuit keyboard shortcuts (Esc closes)", palette.ink, Some((bx, bw - 16)));
                for (i, (shortcut, action)) in SHORTCUTS.iter().enumerate() {
                    let y = by + row * (i as i32 + 1) + 8;
                    if y + row > by + bh { break; }
                    self.pen.text(&mut canvas, bx + 12, y, shortcut, palette.selected, Some((bx, 170)));
                    self.pen.text(&mut canvas, bx + 180, y, action, palette.ink, Some((bx + 180, (bw - 192).max(0))));
                }
            }
        }
        if let Some(dict) = &self.dictionary {
            let row = self.pen.line_height() + 8;
            let shown = dict.words.len().min(8);
            let (bw, bh) = ((w - 2 * MARGIN).min(420).max(1), 67 + row * shown as i32);
            let (bx, by) = ((w - bw) / 2, ((view_h - bh) / 2).max(0));
            canvas.fill(bx + 3, by + 3, bw, bh, palette.shadow);
            canvas.fill(bx, by, bw, bh, palette.bar_bg);
            canvas.frame(bx, by, bw, bh, palette.line);
            self.pen.text(&mut canvas, bx + 10, by + 8, "Assuit custom dictionary", palette.ink, Some((bx, bw - 20)));
            let first = dict.at.saturating_sub(shown.saturating_sub(1));
            for (slot, word) in dict.words.iter().skip(first).take(shown).enumerate() {
                let y = by + 31 + slot as i32 * row;
                if first + slot == dict.at { canvas.fill(bx + 4, y - 2, bw - 8, row, palette.selected); }
                self.pen.text(&mut canvas, bx + 10, y, word, if first + slot == dict.at { palette.on_selected } else { palette.ink }, Some((bx, bw - 20)));
            }
            let footer = match &dict.adding { Some(word) => format!("Add: {word}_   Enter saves, Esc cancels"), None => String::from("A add  ·  Delete remove  ·  Esc close") };
            self.pen.text(&mut canvas, bx + 10, by + bh - 26, &footer, palette.dim, Some((bx, bw - 20)));
        }
        canvas.fill(0, view_h, w, STATUS_H, palette.bar_bg);
        canvas.fill(0, view_h, w, 1, palette.line);
        let pages = self.places.len();
        let page = caret.map_or(0, |c| c.page);
        let status = if let Some(ask) = &self.ask {
            let said = |hint: &str| if self.message.is_empty() { hint.to_string() } else { self.message.clone() };
            match ask {
                Ask::SaveAs(name) => format!("Save as: {name}_   {}", said("docx, odt, rtf, html, md, txt or pdf; Enter saves, Esc closes")),
                Ask::GoTo(page) => format!("Go to page: {page}_   {}", said(&format!("1 to {pages}; Enter goes, Esc closes"))),
                Ask::Table(size) => format!("Insert table: {size}_   {}", said("rows x columns, for example 2x3; Enter inserts, Esc closes")),
                Ask::TableEdit(action) => format!("Table: {action}_   {}", said("row/column edits, merge/split, header on/off, shade, border, style")),
                Ask::List(kind) => format!("List: {kind}_   {}", said("bullet, numbered, formats, level 1–9, off, restart")),
                Ask::Picture(path) => format!("Insert picture: {path}_   {}", said("PNG, JPEG, GIF, WebP or BMP in this program's folder; Enter inserts")),
                Ask::PictureAlt(alt) => format!("Picture description: {alt}_   {}", said("Describe the selected picture for readers who cannot see it")),
                Ask::PictureWidth(width) => format!("Picture width: {width}_   {}", said("Inches, 0.1 to 20; proportions stay the same")),
                Ask::PictureCrop(crop) => format!("Picture crop: {crop}_   {}", said("Left top right bottom, whole percentages; 0 0 0 0 resets")),
                Ask::PrintRange(pages) => format!("Print pages: {pages}_   {}", said("Enter a range such as 1-3,5; PDF is saved beside the document")),
                Ask::Paper(paper) => format!("Paper: {paper}_   {}", said("Letter, Legal or A4 then portrait or landscape; Enter applies")),
                Ask::Margins(margins) => format!("Margins: {margins}_   {}", said("normal/narrow/moderate/wide/mirrored or top bottom left right in inches")),
                Ask::PageNumbers(place) => format!("Page numbers: {place}_   {}", said("top/bottom left/center/right, optional 'of'; or remove top/bottom")),
                Ask::Spacing(spacing) => format!("Paragraph spacing: {spacing}_   {}", said("line multiple, before pt, after pt; for example 1.5 0 8")),
                Ask::Language(lang) => format!("Default proofing language: {lang}_   {}", said("en-US or en-GB; also applies to selected text")),
            }
        } else if let Some(p) = &self.find {
            match &p.with {
                None => {
                    let said = if self.message.is_empty() { "Enter finds the next, Esc closes" } else { &self.message };
                    format!("Find: {}_   {said}", p.what)
                }
                Some(with) => {
                    let said = if self.message.is_empty() { "Tab switches, Enter replaces all, Esc closes" } else { &self.message };
                    let (a, b) = if p.on_with { ("", "_") } else { ("_", "") };
                    format!("Find: {}{a}   Replace with: {with}{b}   {said}", p.what)
                }
            }
        } else if let Some((styles, at)) = &self.table_gallery {
            format!("Table styles: {} of {}  ·  {}", at + 1, styles.len(), styles[*at].name)
        } else if let Some(dict) = &self.dictionary {
            format!("Custom dictionary: {} words   {}", dict.words.len(), self.message)
        } else if let Some(proof) = &self.proof {
            if proof.suggestions.is_empty() {
                format!("Spelling: {}. No suggestion; A adds to dictionary, Ctrl+Shift+Y next, Esc closes", proof.word)
            } else {
                format!("Spelling: {} → {} ({}/{})   Enter changes, A adds to dictionary, Ctrl+Shift+Y next, Esc closes",
                    proof.word, proof.suggestions[proof.at], proof.at + 1, proof.suggestions.len())
            }
        } else if self.message.is_empty() {
            format!("{}{}   Page {} of {}   {}   {} words   Ctrl+S save  Ctrl+Shift+E save as  Ctrl+P PDF  Ctrl+T table  Ctrl+Shift+I picture  Ctrl+Shift+Y spelling  Ctrl+F find  Ctrl+H replace  Ctrl+G go to  Ctrl+Shift+S styles  Ctrl+B/I/U  Ctrl+Z undo  Ctrl+=/- zoom",
                self.name, if self.book.dirty() { " (changed)" } else { "" }, page + 1, pages, self.book.paragraph_style(), self.book.words())
        } else {
            self.message.clone()
        };
        self.pen.text(&mut canvas, 10, view_h + (STATUS_H - self.pen.line_height()) / 2, &status, palette.dim, Some((0, w - 10)));
        window.show_pixels(width, height);
        let short: String = self.name.chars().take(15).collect();
        window.title(format!("{short}{}", if self.book.dirty() { "*" } else { "" }).as_bytes());
        if !selection.is_empty() {
            sex_std::record(&format!("[assuit.selection] rects={} pages={}", selection.len(), {
                let mut p: Vec<usize> = selection.iter().map(|r| r.0 + 1).collect(); p.dedup(); p.len() }));
        }
    }

    fn draw_preview(&mut self, window: &mut sex_app::Window, requested: usize) {
        let (width, height) = window.size();
        let (w, h) = (width as i32, height as i32);
        let view_h = (h - STATUS_H).max(1);
        let pages = self.book.pages().max(1);
        let page = requested.min(pages - 1);
        self.preview_page = Some(page);
        let (pw, ph) = self.book.page_size(page);
        let scale = (((w - 2 * MARGIN).max(1) as f32 / pw.max(1.0))
            .min((view_h - 2 * MARGIN).max(1) as f32 / ph.max(1.0))).clamp(0.1, 6.0);
        let (paper_w, paper_h) = ((pw * scale).ceil() as i32, (ph * scale).ceil() as i32);
        let (left, top) = ((w - paper_w) / 2, (view_h - paper_h) / 2);
        let ready = self.picture(page, scale).is_some();
        let mut canvas = Canvas::new(window.pixels(), width, height);
        canvas.fill(0, 0, w, h, self.palette.desk);
        canvas.fill(left + 2, top + 2, paper_w, paper_h, self.palette.shadow);
        if ready {
            if let Some((_, picture)) = &self.shown[page] {
                canvas.blit(left, top, &picture.pixels, picture.width, (0, 0, w, view_h));
            }
        }
        canvas.fill(0, view_h, w, STATUS_H, self.palette.bar_bg);
        canvas.fill(0, view_h, w, 1, self.palette.line);
        let status = if let Some(Ask::PrintRange(typed)) = &self.ask {
            format!("Print pages: {typed}_   Enter exports range, Esc cancels")
        } else if !self.message.is_empty() {
            self.message.clone()
        } else {
            format!("Print preview  ·  Page {} of {}  ·  Left/Right pages  ·  Ctrl+P PDF  ·  Ctrl+Shift+P range  ·  Esc edit", page + 1, pages)
        };
        self.pen.text(&mut canvas, 10, view_h + (STATUS_H - self.pen.line_height()) / 2, &status, self.palette.dim, Some((0, w - 10)));
        window.show_pixels(width, height);
        window.title(format!("{} — Print preview", self.name).as_bytes());
        sex_std::record(&format!("[assuit.preview] page={} pages={} width={} height={}", page + 1, pages, paper_w, paper_h));
    }

    fn save(&mut self) {
        match self.book.save() {
            Ok(bytes) => {
                self.recovered = false;
                self.unkept = None;
                self.name = self.book.path.rsplit('/').next().unwrap_or(&self.book.path).to_string();
                sex_std::record(&format!("[assuit.saved] name={} bytes={bytes}", self.book.path));
                self.message = format!("Saved {} ({bytes} bytes)", self.name);
            }
            Err(e) => {
                sex_std::record(&format!("[assuit.save-failed] {e}"));
                self.message = e;
            }
        }
    }

    /// Keeps unsaved changes aside (on close, and after a quiet moment), so a crash or a
    /// closed window loses nothing: the next start opens them.
    fn keep(&mut self) {
        self.unkept = None;
        if !self.book.dirty() { return; }
        match self.book.write_recovery() {
            Ok(()) => sex_std::record(&format!("[assuit.kept] name={}", crate::book::recovery_path(&self.book.path))),
            Err(e) => sex_std::record(&format!("[assuit.keep-failed] {e}")),
        }
    }

    /// Opens with last time's unsaved changes: says so, and Esc throws them away.
    pub fn recovered(mut self) -> Ui {
        self.recovered = true;
        self.message = "Brought back the changes not saved last time: Ctrl+S keeps them, Esc throws them away".into();
        self
    }

    /// Esc on a document that came back with unsaved changes: goes back to the saved file.
    fn discard_recovered(&mut self) {
        self.recovered = false;
        self.book.forget_recovery();
        let path = self.book.path.clone();
        let opened = if std::fs::metadata(&path).is_ok() { Book::open(&path) } else { Ok(Book::blank(&path)) };
        match opened {
            Ok(book) => {
                self.book = book;
                self.scroll = 0;
                self.invalidate();
                sex_std::record(&format!("[assuit.discarded] name={path}"));
                self.message = format!("Threw the kept changes away: {} is as last saved", self.name);
            }
            Err(e) => self.message = e,
        }
    }

    /// A key while the Save As or Go To prompt is open.
    fn ask_key(&mut self, key: Key) {
        let Some(ask) = &mut self.ask else { return };
        let text = match ask { Ask::SaveAs(t) | Ask::GoTo(t) | Ask::Table(t) | Ask::TableEdit(t) | Ask::List(t) | Ask::Picture(t) | Ask::PictureAlt(t) | Ask::PictureWidth(t) | Ask::PictureCrop(t) | Ask::PrintRange(t) | Ask::Paper(t) | Ask::Margins(t) | Ask::PageNumbers(t) | Ask::Spacing(t) | Ask::Language(t) => t };
        match key.code {
            code::ESC => { self.ask = None; self.message.clear(); return; }
            code::BACKSPACE => { text.pop(); self.message.clear(); return; }
            code::ENTER => {}
            _ => {
                if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) { text.push(ch); self.message.clear(); }
                return;
            }
        }
        let typed = text.trim().to_string();
        if typed.is_empty() { return; }
        if matches!(ask, Ask::GoTo(_)) {
            let Ok(n) = typed.parse::<usize>() else { self.message = format!("{typed} is not a page number"); return };
            self.ask = None;
            let page = self.book.go_to_page(n);
            self.scroll_to = Some(page);
            sex_std::record(&format!("[assuit.goto] page={}", page + 1));
            self.message = format!("Page {} of {}", page + 1, self.book.pages());
            return;
        }
        if matches!(ask, Ask::Table(_)) {
            let Some((rows, cols)) = typed.split_once(|c| matches!(c, 'x' | 'X' | '×')) else {
                self.message = "Enter rows x columns, for example 2x3".into();
                return;
            };
            let dimensions = (rows.trim().parse::<usize>(), cols.trim().parse::<usize>());
            let (Ok(rows), Ok(cols)) = dimensions else {
                self.message = "Enter rows x columns, for example 2x3".into();
                return;
            };
            match self.book.insert_table(rows, cols) {
                Ok(()) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.table] rows={rows} cols={cols}"));
                    self.message = format!("Inserted {rows} × {cols} table");
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::TableEdit(_)) {
            if typed.eq_ignore_ascii_case("style") {
                match self.book.table_styles() {
                    Ok(styles) if !styles.is_empty() => {
                        sex_std::record(&format!("[assuit.table-gallery.open] styles={}", styles.len()));
                        self.ask = None;
                        self.table_gallery = Some((styles, 0));
                        self.message.clear();
                    }
                    Ok(_) => self.message = "This document has no table styles".into(),
                    Err(e) => self.message = e,
                }
                return;
            }
            match self.book.edit_table(&typed) {
                Ok(description) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.table-edit] action={typed}"));
                    self.message = description.into();
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::List(_)) {
            match self.book.edit_list(&typed) {
                Ok(description) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.list] action={typed}"));
                    self.message = description.into();
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::Picture(_)) {
            match self.book.insert_picture(&typed) {
                Ok(()) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.picture] path={typed}"));
                    self.message = format!("Inserted picture from {typed}");
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::PictureAlt(_)) {
            match self.book.picture_alt_text(&typed) {
                Ok(()) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.picture-alt] chars={}", typed.chars().count()));
                    self.message = "Updated picture description".into();
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::PictureWidth(_)) {
            match self.book.picture_width(&typed) {
                Ok(()) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.picture-width] inches={typed}"));
                    self.message = format!("Picture width set to {typed} inches");
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::PictureCrop(_)) {
            match self.book.picture_crop(&typed) {
                Ok(()) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.picture-crop] values={typed}"));
                    self.message = "Cropped picture".into();
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::PrintRange(_)) {
            match self.book.export_pdf_range(&typed) {
                Ok((path, bytes)) => {
                    self.ask = None;
                    sex_std::record(&format!("[assuit.pdf-range] pages={typed} name={path} bytes={bytes}"));
                    self.message = format!("Printed {typed} to {} ({bytes} bytes)", path.rsplit('/').next().unwrap_or(&path));
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::Paper(_)) {
            match self.book.set_paper(&typed) {
                Ok(()) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.paper] {typed}"));
                    self.message = format!("Paper set to {typed}");
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::Margins(_)) {
            match self.book.set_margins(&typed) {
                Ok(()) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.margins] {typed}"));
                    self.message = format!("Margins set to {typed}");
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::PageNumbers(_)) {
            match self.book.insert_page_numbers(&typed) {
                Ok(()) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.page-numbers] {typed}"));
                    self.message = format!("Page numbers: {typed}");
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::Spacing(_)) {
            match self.book.set_paragraph_spacing(&typed) {
                Ok(()) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.paragraph-spacing] {typed}"));
                    self.message = format!("Paragraph spacing set to {typed}");
                }
                Err(e) => self.message = e,
            }
            return;
        }
        if matches!(ask, Ask::Language(_)) {
            match self.book.proofing_language(&typed) {
                Ok(tag) => {
                    self.ask = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.proofing-language] {tag}"));
                    self.message = format!("Default proofing language: {tag}");
                }
                Err(e) => self.message = e,
            }
            return;
        }
        match self.book.save_as(&typed) {
            Ok((path, bytes)) => {
                self.ask = None;
                sex_std::record(&format!("[assuit.saved-as] name={path} bytes={bytes}"));
                let short = path.rsplit('/').next().unwrap_or(&path).to_string();
                if path == self.book.path { self.name = short.clone(); self.recovered = false; self.unkept = None; }
                self.message = format!("Saved {short} ({bytes} bytes)");
            }
            Err(e) => self.message = e,
        }
    }

    /// Writes every page as a PDF beside the document.
    fn export_pdf(&mut self) {
        match self.book.export_pdf() {
            Ok((path, bytes)) => {
                sex_std::record(&format!("[assuit.pdf] name={path} bytes={bytes}"));
                self.message = format!("Wrote {} ({bytes} bytes)", path.rsplit('/').next().unwrap_or(&path));
            }
            Err(e) => {
                sex_std::record(&format!("[assuit.pdf-failed] {e}"));
                self.message = e;
            }
        }
    }

    /// One key: Assuit's own (save, zoom), then WordCraft's shortcut, then typed text.
    fn key(&mut self, key: Key) {
        if key.control && key.alt && key.code == code::P {
            let closing = self.preview_page.is_some();
            self.preview_page = if self.preview_page.is_some() { None } else { Some(self.book.caret().map_or(0, |c| c.page)) };
            self.message.clear();
            if closing { sex_std::record("[assuit.preview.closed]"); }
            return;
        }
        if let Some(page) = self.preview_page {
            if self.ask.is_some() { return self.ask_key(key); }
            let last = self.book.pages().saturating_sub(1);
            match (key.code, sex_app::nav(key)) {
                (code::ESC, _) => { self.preview_page = None; sex_std::record("[assuit.preview.closed]"); },
                (_, Some(Nav::Left)) | (_, Some(Nav::Up)) => self.preview_page = Some(page.saturating_sub(1)),
                (_, Some(Nav::Right)) | (_, Some(Nav::Down)) => self.preview_page = Some((page + 1).min(last)),
                (c, _) if code::page_up(c) => self.preview_page = Some(page.saturating_sub(1)),
                (c, _) if code::page_down(c) => self.preview_page = Some((page + 1).min(last)),
                (code::P, _) if key.control && !key.shift => self.export_pdf(),
                (code::P, _) if key.control && key.shift => self.ask = Some(Ask::PrintRange(String::new())),
                _ => {}
            }
            return;
        }
        if key.control && key.shift && key.code == code::SLASH {
            self.help = !self.help;
            sex_std::record(&format!("[assuit.shortcuts] shown={}", self.help as u8));
            return;
        }
        if self.help {
            if key.code == code::ESC { self.help = false; }
            return;
        }
        if key.control && key.shift && key.code == code::D && self.ask.is_none() && self.find.is_none() && self.picker.is_none() && self.proof.is_none() {
            if self.dictionary.is_some() { self.dictionary = None; }
            else {
                let words = self.book.dictionary_words();
                sex_std::record(&format!("[assuit.dictionary.open] words={}", words.len()));
                self.dictionary = Some(Dictionary { words, at: 0, adding: None });
            }
            self.message.clear();
            return;
        }
        if self.dictionary.is_some() { return self.dictionary_key(key); }
        if self.ask.is_some() { return self.ask_key(key); }
        if self.find.is_some() { return self.find_key(key); }
        if self.picker.is_some() { return self.picker_key(key); }
        if self.table_gallery.is_some() { return self.table_gallery_key(key); }
        if self.proof.is_some() { return self.proof_key(key); }
        if self.recovered && key.code == code::ESC && !key.control { return self.discard_recovered(); }
        self.message.clear();
        if key.code == code::U && key.control && key.shift {
            let enabled = !self.book.proofing_marks();
            match self.book.set_proofing_preference(Book::PROOFING_PREFERENCE, enabled) {
                Ok(()) => {
                    self.invalidate();
                    self.message = format!("Spelling marks {}", if enabled { "shown" } else { "hidden" });
                    sex_std::record(&format!("[assuit.proofing.marks] enabled={}", enabled as u8));
                }
                Err(e) => self.message = format!("Could not change spelling marks: {e}"),
            }
            return;
        }
        if key.code == code::Y && key.control && key.shift { return self.check_spelling(); }
        if key.control && key.alt && !key.shift {
            let deeper = match sex_app::nav(key) {
                Some(Nav::Right) => Some(true),
                Some(Nav::Left) => Some(false),
                _ => None,
            };
            if let Some(deeper) = deeper {
                match self.book.step_list_level(deeper) {
                    Ok(level) => {
                        self.invalidate();
                        self.message = format!("List level {level}");
                        sex_std::record(&format!("[assuit.list-level] level={level}"));
                    }
                    Err(e) => self.message = e,
                }
                return;
            }
        }
        if key.control {
            match (key.code, sex_app::typed(Key { control: false, ..key })) {
                (code::E, _) if key.shift => { self.ask = Some(Ask::SaveAs(self.name.clone())); return; }
                (code::G, _) if !key.shift => { self.ask = Some(Ask::GoTo(String::new())); return; }
                (code::P, _) if !key.shift => return self.export_pdf(),
                (code::P, _) if key.shift => { self.ask = Some(Ask::PrintRange(String::new())); return; }
                (code::L, _) if key.shift => { self.ask = Some(Ask::Paper(String::new())); return; }
                (code::L, _) if key.alt => { self.ask = Some(Ask::List(String::new())); return; }
                (code::M, _) if key.shift => { self.ask = Some(Ask::Margins(String::new())); return; }
                (code::N, _) if key.alt => { self.ask = Some(Ask::PageNumbers(String::new())); return; }
                (code::S, _) if key.alt => { self.ask = Some(Ask::Spacing(String::new())); return; }
                (code::K, _) if key.shift => { self.ask = Some(Ask::Language(String::new())); return; }
                (code::T, _) if !key.shift => { self.ask = Some(Ask::Table(String::new())); return; }
                (code::T, _) if key.shift => { self.ask = Some(Ask::TableEdit(String::new())); return; }
                (code::I, _) if key.shift => { self.ask = Some(Ask::Picture(String::new())); return; }
                (code::R, _) if key.shift => { self.ask = Some(Ask::PictureAlt(String::new())); return; }
                (code::O, _) if key.shift => { self.ask = Some(Ask::PictureWidth(String::new())); return; }
                (code::C, _) if key.shift => { self.ask = Some(Ask::PictureCrop(String::new())); return; }
                (code::S, _) if key.shift => {
                    let styles = self.book.styles();
                    let current = self.book.paragraph_style();
                    let at = styles.iter().position(|s| s.1 == current).unwrap_or(0);
                    sex_std::record(&format!("[assuit.styles] count={} current={current}", styles.len()));
                    if !styles.is_empty() { self.picker = Some((styles, at)); }
                    return;
                }
                (code::S, _) => return self.save(),
                (code::F, _) if !key.shift => { self.find = Some(Prompt { what: String::new(), with: None, on_with: false }); return; }
                (code::H, _) if !key.shift => { self.find = Some(Prompt { what: String::new(), with: Some(String::new()), on_with: false }); return; }
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

    /// A key-originated transfer through Shell's focused-window clipboard.
    /// Prompts keep their own text editing and never publish document text.
    fn clipboard_key(&mut self, key: Key, window: &mut sex_app::Window) -> bool {
        if !key.control || key.shift || key.alt
            || self.preview_page.is_some() || self.help || self.dictionary.is_some()
            || self.ask.is_some() || self.find.is_some() || self.picker.is_some()
            || self.table_gallery.is_some() || self.proof.is_some() { return false; }
        match key.code {
            code::C | code::X => {
                let cut = key.code == code::X;
                let text = match self.book.copy_text() {
                    Ok(text) if !text.is_empty() => text,
                    Ok(_) => { self.message = "Select text before copying".into(); return true; }
                    Err(e) => { self.message = format!("Could not copy: {e}"); return true; }
                };
                let result = sex_app::clipboard::copy(window, &text);
                sex_std::record(&format!("[assuit.clipboard.copy] bytes={} cut={} ok={}", text.len(), cut as u8, result.is_ok() as u8));
                match result {
                    Ok(()) => {
                        sex_std::record("[assuit.command] edit.copy");
                        if cut {
                            match self.book.run("edit.cut", serde_json::json!({})) {
                                Ok(_) => { self.invalidate(); sex_std::record("[assuit.command] edit.cut"); },
                                Err(e) => { self.message = format!("Copied, but could not cut: {e}"); return true; },
                            }
                        }
                        self.message = if cut { "Cut to shared clipboard" } else { "Copied to shared clipboard" }.into();
                    }
                    Err(e) => self.message = format!("Shared clipboard unavailable: {e:?}"),
                }
                true
            }
            code::V => {
                let text = match sex_app::clipboard::paste(window) {
                    Ok(text) => text.to_string(),
                    Err(sex_app::clipboard::Error::Empty) => {
                        self.message = "Shared clipboard is empty".into(); return true;
                    }
                    Err(e) => { self.message = format!("Shared clipboard unavailable: {e:?}"); return true; }
                };
                match self.book.paste_text(&text) {
                    Ok(()) => {
                        self.invalidate();
                        sex_std::record(&format!("[assuit.clipboard.paste] bytes={} ok=1", text.len()));
                        sex_std::record("[assuit.command] edit.paste");
                        self.message = "Pasted from shared clipboard".into();
                    }
                    Err(e) => self.message = format!("Could not paste: {e}"),
                }
                true
            }
            _ => false,
        }
    }

    fn check_spelling(&mut self) {
        match self.book.next_spelling_issue() {
            Ok(Some((word, suggestions))) => {
                sex_std::record(&format!("[assuit.spelling] word={word} suggestions={}", suggestions.len()));
                self.proof = Some(Proof { word, suggestions, at: 0 });
            }
            Ok(None) => self.message = "Spelling and grammar check complete".into(),
            Err(e) => self.message = e,
        }
    }

    fn dictionary_key(&mut self, key: Key) {
        let Some(mut dict) = self.dictionary.take() else { return };
        if let Some(typed) = &mut dict.adding {
            match key.code {
                code::ESC => { dict.adding = None; self.message.clear(); }
                code::BACKSPACE => { typed.pop(); self.message.clear(); }
                code::ENTER => {
                    let word = typed.trim().to_string();
                    if !word.is_empty() {
                        match self.book.add_spelling_word(&word) {
                            Ok(()) => {
                                dict.words = self.book.dictionary_words();
                                dict.at = dict.words.iter().position(|w| w.eq_ignore_ascii_case(&word)).unwrap_or(0);
                                dict.adding = None;
                                self.invalidate();
                                sex_std::record(&format!("[assuit.dictionary.manager.added] word={word}"));
                                self.message = format!("Added {word}");
                            }
                            Err(e) => self.message = e,
                        }
                    }
                }
                _ => if !key.control && typed.chars().count() < 64 {
                    if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) { typed.push(ch); self.message.clear(); }
                },
            }
        } else {
            match (key.code, sex_app::nav(key)) {
                (code::ESC, _) => { self.message.clear(); return; }
                (code::A, _) if !key.control => { dict.adding = Some(String::new()); self.message.clear(); }
                (_, Some(Nav::Up)) => dict.at = dict.at.saturating_sub(1),
                (_, Some(Nav::Down)) => dict.at = (dict.at + 1).min(dict.words.len().saturating_sub(1)),
                (_, Some(Nav::Delete)) => if let Some(word) = dict.words.get(dict.at).cloned() {
                    match self.book.remove_spelling_word(&word) {
                        Ok(()) => {
                            dict.words = self.book.dictionary_words();
                            dict.at = dict.at.min(dict.words.len().saturating_sub(1));
                            self.invalidate();
                            sex_std::record(&format!("[assuit.dictionary.manager.removed] word={word}"));
                            self.message = format!("Removed {word}");
                        }
                        Err(e) => self.message = e,
                    }
                },
                _ => {}
            }
        }
        self.dictionary = Some(dict);
    }

    fn proof_key(&mut self, key: Key) {
        if key.code == code::ESC { self.proof = None; return; }
        if key.code == code::Y && key.control && key.shift { self.proof = None; return self.check_spelling(); }
        let Some(proof) = &mut self.proof else { return };
        if key.code == code::A && !key.control {
            let word = proof.word.clone();
            match self.book.add_spelling_word(&word) {
                Ok(()) => {
                    self.proof = None;
                    self.invalidate();
                    sex_std::record(&format!("[assuit.dictionary.added] word={word}"));
                    self.message = format!("Added {word} to Assuit's dictionary");
                }
                Err(e) => self.message = e,
            }
            return;
        }
        match sex_app::nav(key) {
            Some(Nav::Up) => proof.at = proof.at.saturating_sub(1),
            Some(Nav::Down) => proof.at = (proof.at + 1).min(proof.suggestions.len().saturating_sub(1)),
            _ if key.code == code::ENTER => {
                let Some(suggestion) = proof.suggestions.get(proof.at).cloned() else { return };
                match self.book.apply_spelling(&suggestion) {
                    Ok(()) => {
                        sex_std::record(&format!("[assuit.spelling.change] to={suggestion}"));
                        self.proof = None;
                        self.invalidate();
                        self.message = format!("Changed spelling to {suggestion}");
                    }
                    Err(e) => self.message = e,
                }
            }
            _ => {}
        }
    }

    /// A key while the find (or find and replace) prompt is open: Enter selects the next
    /// match, or with a replacement replaces them all.
    fn find_key(&mut self, key: Key) {
        let Some(p) = &mut self.find else { return };
        match key.code {
            code::ESC => self.find = None,
            code::BACKSPACE => { p.field().pop(); self.message.clear(); }
            code::TAB if p.with.is_some() => p.on_with = !p.on_with,
            code::ENTER if p.with.is_some() && !p.on_with => p.on_with = true,
            code::ENTER if !p.what.is_empty() => {
                let what = p.what.clone();
                if let Some(with) = p.with.clone() {
                    match self.book.replace_all(&what, &with) {
                        Ok(n) => {
                            sex_std::record(&format!("[assuit.replace] what={what} with={with} count={n}"));
                            self.message = format!("Replaced {n}");
                            if n > 0 { self.invalidate(); }
                        }
                        Err(e) => self.message = e,
                    }
                    return;
                }
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
            _ => if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) { p.field().push(ch); self.message.clear(); },
        }
    }

    /// A key while the style list is open: arrows choose, Enter applies, Esc closes.
    fn picker_key(&mut self, key: Key) {
        let Some((styles, at)) = &mut self.picker else { return };
        match (key.code, sex_app::nav(key)) {
            (code::ESC, _) => self.picker = None,
            (_, Some(Nav::Up)) => *at = at.saturating_sub(1),
            (_, Some(Nav::Down)) => *at = (*at + 1).min(styles.len() - 1),
            (code::ENTER, _) => {
                let (id, name) = styles[*at].clone();
                self.picker = None;
                match self.book.set_style(&id) {
                    Ok(()) => {
                        sex_std::record(&format!("[assuit.style] style={name}"));
                        self.message = format!("Paragraph style: {name}");
                        self.invalidate();
                    }
                    Err(e) => self.message = e,
                }
            }
            _ => {}
        }
    }

    fn apply_table_gallery(&mut self, at: usize) {
        let Some((styles, _)) = self.table_gallery.take() else { return };
        let Some(style) = styles.get(at) else { return };
        match self.book.edit_table(&format!("style {}", style.name)) {
            Ok(_) => {
                sex_std::record(&format!("[assuit.table-gallery.applied] name={}", style.name));
                self.message = format!("Table style: {}", style.name);
                self.invalidate();
            }
            Err(e) => { self.message = e; self.table_gallery = Some((styles, at)); }
        }
    }

    fn table_gallery_key(&mut self, key: Key) {
        let Some((styles, at)) = &mut self.table_gallery else { return };
        match (key.code, sex_app::nav(key)) {
            (code::ESC, _) => { self.table_gallery = None; sex_std::record("[assuit.table-gallery.closed]"); },
            (_, Some(Nav::Up)) => *at = at.saturating_sub(1),
            (_, Some(Nav::Down)) => *at = (*at + 1).min(styles.len() - 1),
            (c, _) if code::page_up(c) => *at = at.saturating_sub(6),
            (c, _) if code::page_down(c) => *at = (*at + 6).min(styles.len() - 1),
            (code::ENTER, _) => { let at = *at; self.apply_table_gallery(at); },
            _ => {}
        }
    }

    /// A click puts the caret where it lands; dragging with the button held selects;
    /// the wheel scrolls.
    fn pointer(&mut self, p: Pointer) -> bool {
        if let Some(page) = self.preview_page {
            if let Kind::Wheel { steps } = p.kind {
                self.preview_page = Some(if steps > 0 { page.saturating_sub(1) } else { (page + 1).min(self.book.pages().saturating_sub(1)) });
                return true;
            }
            return false;
        }
        if let Some((styles, at)) = &self.table_gallery {
            let Kind::Down { button: 0 } = p.kind else { return false };
            let (width, height) = self.surface_size;
            let (bx, by, bw, shown, first, row) = Self::gallery_bounds(width, height - STATUS_H, styles.len(), *at);
            let (x, y) = (p.x as i32, p.y as i32);
            if x < bx || x >= bx + bw || y < by + 33 || y >= by + 33 + shown as i32 * row { return false; }
            let index = first + ((y - by - 33) / row) as usize;
            self.apply_table_gallery(index);
            return true;
        }
        // The style list is modal: the pages under it do not take clicks.
        if self.picker.is_some() || self.dictionary.is_some() { return false; }
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
        let mut theme_next = sex_std::sex_pdx::get_ticks().saturating_add(1000);
        loop {
            let now = sex_std::sex_pdx::get_ticks();
            if now >= theme_next {
                theme_next = now.saturating_add(1000);
                if let Ok(generation) = sex_app::settings::generation() {
                    if generation != self.theme_generation {
                        self.theme_generation = generation;
                        let theme = sex_app::settings::theme();
                        self.palette = Palette::from_tokens(theme);
                        sex_std::record(&format!("[assuit.theme] mode={:?} accent={:06x}", theme.mode, self.palette.selected));
                        self.draw(window);
                    }
                }
            }
            let Some(event) = window.next_within(1000) else {
                // A quiet moment after a change: keep the unsaved work safe.
                if self.unkept.is_some_and(|t| sex_std::sex_pdx::get_ticks().saturating_sub(t) >= KEEP_AFTER_MS) { self.keep(); }
                continue;
            };
            match event {
                Event::Close => { self.keep(); break; }
                Event::Resize { .. } => self.draw(window),
                Event::Key(key) => {
                    let dirty = self.book.dirty();
                    let edits = self.book.edits;
                    if !self.clipboard_key(key, window) { self.key(key); }
                    // An edit (typing, formatting, a style, undo): keep it soon, and the
                    // brought-back changes become the user's own.
                    if self.book.edits != edits && self.book.dirty() {
                        self.unkept.get_or_insert(sex_std::sex_pdx::get_ticks());
                        if key.code != code::ESC { self.recovered = false; }
                    }
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
