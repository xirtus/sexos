// SPDX-License-Identifier: GPL-3.0-only
//! The grid, the formula bar and the sheet tabs, painted into the window with
//! sex-office until GridCraft's own egui interface runs on SexOS (workstream M11).
//! Frozen rows and columns stay put while the rest scrolls; filtered-out rows are
//! skipped; charts float over the cells at their anchors.
use crate::book::{Book, Function, Range, column_name};
use crate::filter_menu::FilterMenu;
use sex_office::{Canvas, Pen};
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx::app_pointer::{Kind, Pointer};
use std::cell::RefCell;
use sex_theme::Tokens;

#[derive(Clone, Copy)]
struct Palette {
    bg: u32, grid: u32, head_bg: u32, head_on: u32, ink: u32,
    dim: u32, error: u32, cursor: u32, bar_bg: u32, tab_on: u32,
    accent: u32, frozen: u32, chart_edge: u32,
}

impl Palette {
    fn from_tokens(t: Tokens) -> Self {
        let rgb = |c: u32| c & 0x00FF_FFFF;
        Self {
            bg: rgb(t.field), grid: rgb(t.border), head_bg: rgb(t.surface),
            head_on: rgb(sex_theme::mix(t.surface, t.accent, 0.25)),
            ink: rgb(t.text), dim: rgb(t.text_dim), error: rgb(t.danger),
            cursor: rgb(t.focus), bar_bg: rgb(t.surface), tab_on: rgb(t.field),
            accent: rgb(t.accent), frozen: rgb(t.text_dim), chart_edge: rgb(t.border),
        }
    }
}

const BAR_H: i32 = 30;
const HEAD_H: i32 = 22;
const ROW_HEAD_W: i32 = 48;
const TABS_H: i32 = 28;
const PAD: i32 = 4;
/// Rows a Page Up or Page Down moves.
const PAGE: u32 = 20;
const LAST_ROW: u32 = 1_048_575;
const LAST_COL: u32 = 16_383;

/// How a cell is being typed into.
struct Edit {
    text: Vec<char>,
    caret: usize,
    /// Begun by typing over the cell: arrows commit and move, as in Excel's Enter mode.
    /// Begun with Enter: arrows move the caret.
    replace: bool,
}

pub struct Ui {
    palette: Palette,
    theme_generation: u64,
    book: Book,
    name: String,
    row: u32,
    col: u32,
    /// The other corner of the selection, when Shift has stretched it.
    anchor: Option<(u32, u32)>,
    /// First scrolling row and column (frozen ones are always shown before them).
    top: u32,
    left: u32,
    edit: Option<Edit>,
    message: String,
    pen: Pen,
    /// Pens for chart text, by pixel size.
    small: RefCell<Vec<(u32, Pen)>>,
    /// What the last draw put where, for clicks: (row, y, height) and (col, x, width).
    rows: Vec<(u32, i32, i32)>,
    cols: Vec<(u32, i32, i32)>,
    /// The grid area's size in the last draw.
    grid: (i32, i32),
    /// The cells last copied or cut, outlined until pasted over or Esc.
    copied: Option<(Range, bool)>,
    /// The Ctrl+F or Ctrl+H prompt, while it is open.
    find: Option<Prompt>,
    /// A column whose right edge is being dragged: (column, pointer x at the start, width then,
    /// width now).
    resizing: Option<(u32, i32, i32, i32)>,
    /// A row whose bottom edge is being dragged: (row, pointer y at start, height then, height now).
    resizing_row: Option<(u32, i32, i32, i32)>,
    /// The Save As, Go To or insert/delete prompt, while it is open.
    ask: Option<Ask>,
    /// Value picker opened from an AutoFilter header button or Ctrl+Shift+O.
    filter_menu: Option<FilterMenu>,
    /// Searchable worksheet function catalog.
    function_picker: Option<FunctionPicker>,
    /// An in-app shortcut reference, opened with Ctrl+Shift+/.
    help: bool,
    import_warnings: Vec<String>,
    warning_open: bool,
    warning_index: usize,
    /// The workbook came back with last time's unsaved changes: Esc throws them away until
    /// the next edit.
    recovered: bool,
    /// When the workbook last changed without its changes being kept (ticks), for the
    /// quiet-moment copy.
    unkept: Option<u64>,
}

/// A one-line prompt in the status line.
enum Ask {
    /// Ctrl+Shift+S: a file name; its extension picks the format.
    SaveAs(Vec<char>),
    /// Ctrl+G: a cell, range or Sheet!cell.
    GoTo(Vec<char>),
    /// Ctrl+Shift+= (insert) or Ctrl+- (delete): R for rows, C for columns.
    Lines { insert: bool },
    /// Ctrl+Shift+M: C, L, B or P picks a chart for the current selection.
    Chart,
    /// Ctrl+Shift+Q: edit the chart anchored at the selected cell.
    ChartEdit { id: u32, text: Vec<char> },
    /// Ctrl+Shift+J: sort the selected table by several columns in priority order.
    MultiSort { range: crate::book::Range, text: Vec<char> },
    /// Ctrl+Shift+K or C in the value menu: a comparison for this column.
    FilterCondition { col: u32, text: Vec<char> },
}


struct FunctionPicker { entries: Vec<Function>, query: String, selected: usize }

impl FunctionPicker {
    fn matches(&self) -> Vec<usize> {
        let mut matches: Vec<usize> = self.entries.iter().enumerate().filter_map(|(i, f)| f.matches(&self.query).then_some(i)).collect();
        if !self.query.is_empty() {
            let q = self.query.to_ascii_lowercase();
            matches.sort_by_key(|i| {
                let name = self.entries[*i].name.to_ascii_lowercase();
                if name == q { 0 } else if name.starts_with(&q) { 1 } else { 2 }
            });
        }
        matches
    }
}

const FILTER_MENU_W: i32 = 230;
const FILTER_MENU_ROW: i32 = 23;
const FILTER_MENU_VISIBLE: usize = 8;

/// Unsaved changes are copied aside after this long without a key (milliseconds).
const KEEP_AFTER_MS: u64 = 15_000;
fn warning_lines(message: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in message.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() { line.push(' '); }
        line.push_str(word);
    }
    if !line.is_empty() { lines.push(line); }
    lines
}
const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+S", "Save"), ("Ctrl+Shift+S", "Save as / choose format"),
    ("Ctrl+P", "Export PDF"), ("Ctrl+G", "Go to cell or range"),
    ("F2", "Edit selected cell"), ("Ctrl+Z / Y", "Undo / redo"),
    ("Ctrl+C / X / V", "Copy / cut / paste"),
    ("Ctrl+F / H", "Find / replace"),
    ("Ctrl+Shift+= / -", "Insert / delete rows or columns"),
    ("Ctrl+Shift+A / D", "Sort ascending / descending"),
    ("Ctrl+Shift+J", "Sort selected table by several columns"),
    ("Ctrl+Shift+L / E / O", "Filter / by cell / choose values; Space marks"),
    ("Ctrl+Shift+K", "Filter current column by condition"),
    ("Ctrl+Shift+F / C", "Freeze panes / column chart"),
    ("Ctrl+Shift+M", "Choose column, line, bar or pie chart"),
    ("Ctrl+Shift+Q", "Edit selected chart layout"),
    ("Ctrl+Shift+I", "Search and insert a worksheet function"),
    ("Ctrl+Shift+U", "Wrap or unwrap selected cells"),
    ("Ctrl+Shift+W", "Review import warnings"),
    ("Ctrl+PageUp / Down", "Previous / next sheet"),
    ("Ctrl+Shift+/", "Show or hide this reference"),
];

/// Ctrl+F asks what to find; Ctrl+H also asks what to put in its place.
struct Prompt {
    what: Vec<char>,
    with: Option<Vec<char>>,
    /// Typing goes into `with` (Tab or Enter moves there from `what`).
    on_with: bool,
}

impl Prompt {
    /// The text being typed into.
    fn field(&mut self) -> &mut Vec<char> {
        match &mut self.with { Some(with) if self.on_with => with, _ => &mut self.what }
    }
}

/// Set-1 scan codes Shell passes (with the normalized codes some keyboards send).
mod code {
    pub const ESC: u8 = 0x01;
    pub const BACKSPACE: u8 = 0x0E;
    pub const SPACE: u8 = 0x39;
    pub const TAB: u8 = 0x0F;
    pub const ENTER: u8 = 0x1C;
    pub const S: u8 = 0x1F;
    pub const Z: u8 = 0x2C;
    pub const Y: u8 = 0x15;
    pub const A: u8 = 0x1E;
    pub const D: u8 = 0x20;
    pub const E: u8 = 0x12;
    pub const F: u8 = 0x21;
    pub const L: u8 = 0x26;
    pub const M: u8 = 0x32;
    pub const C: u8 = 0x2E;
    pub const X: u8 = 0x2D;
    pub const V: u8 = 0x2F;
    pub const R: u8 = 0x13;
    pub const T: u8 = 0x14;
    pub const B: u8 = 0x30;
    pub const I: u8 = 0x17;
    pub const U: u8 = 0x16;
    pub const O: u8 = 0x18;
    pub const K: u8 = 0x25;
    pub const J: u8 = 0x24;
    pub const Q: u8 = 0x10;
    pub const W: u8 = 0x11;
    pub const F2: u8 = 0x3C;
    pub const SLASH: u8 = 0x35;
    pub const H: u8 = 0x23;
    pub const G: u8 = 0x22;
    pub const P: u8 = 0x19;
    pub const MINUS: u8 = 0x0C;
    pub const EQUALS: u8 = 0x0D;
    /// The digit row: 1 to 6, and the key left of 1 (` and ~).
    pub fn digit(c: u8) -> Option<u8> { (0x02..=0x07).contains(&c).then(|| c - 1) }
    pub const GRAVE: u8 = 0x29;
    pub fn page_up(c: u8) -> bool { matches!(c, 0x49 | 0x68) }
    pub fn page_down(c: u8) -> bool { matches!(c, 0x51 | 0x6D) }
}

/// GridCraft's charts measure their text with the grid's own font.
struct Measure<'a>(&'a Ui);
impl gridcraft_chart::Measure for Measure<'_> {
    fn text_width(&self, text: &str, size: f32, _bold: bool) -> f32 {
        self.0.with_pen(size, |pen| pen.measure(text)) as f32
    }
}

impl Ui {
    pub fn new(book: Book, name: String) -> Ui {
        let theme = sex_app::settings::theme();
        let palette = Palette::from_tokens(theme);
        let theme_generation = sex_app::settings::generation().unwrap_or(0);
        sex_std::record(&format!("[spreadeagle.theme] mode={:?} accent={:06x}", theme.mode, palette.accent));
        Ui {
            palette, theme_generation,
            book, name, row: 0, col: 0, anchor: None, top: 0, left: 0, edit: None, message: String::new(),
            pen: Pen::sans(13), small: RefCell::new(Vec::new()), rows: Vec::new(), cols: Vec::new(), grid: (i32::MAX, i32::MAX), copied: None, find: None, resizing: None, resizing_row: None,
            ask: None, filter_menu: None, function_picker: None, help: false,
            import_warnings: Vec::new(), warning_open: false, warning_index: 0, recovered: false, unkept: None,
        }
    }

    pub fn with_import_warnings(mut self, warnings: Vec<String>) -> Ui {
        self.import_warnings = warnings.into_iter().map(|w| w.replace("GridCraft", "Spreadeagle")).collect();
        self.warning_open = !self.import_warnings.is_empty();
        if self.warning_open {
            sex_std::record(&format!("[spreadeagle.warnings.shown] count={}", self.import_warnings.len()));
        }
        self
    }

    fn with_pen<T>(&self, size: f32, f: impl FnOnce(&mut Pen) -> T) -> T {
        let px = (size.round() as u32).clamp(8, 32);
        let mut pens = self.small.borrow_mut();
        let i = match pens.iter().position(|(s, _)| *s == px) {
            Some(i) => i,
            None => { pens.push((px, Pen::sans(px))); pens.len() - 1 }
        };
        f(&mut pens[i].1)
    }

    fn cell_name(&self) -> String { format!("{}{}", column_name(self.col), u64::from(self.row) + 1) }

    /// The cells the next command works on: the stretched selection, or the cursor's cell.
    fn selection(&self) -> Range {
        let (r, c) = self.anchor.unwrap_or((self.row, self.col));
        Range { r0: r, c0: c, r1: self.row, c1: self.col }
    }

    /// The frozen rows and columns the view keeps still: the sheet's, unless they would
    /// take more than half the grid (a small window, or panes frozen far down), when that
    /// direction scrolls as a whole so the cursor's cells stay in sight.
    fn frozen(&self) -> (u32, u32) {
        let (fr, fc) = self.book.frozen().unwrap_or((0, 0));
        let (w, h) = self.grid;
        let rows: i32 = (0..fr).map(|r| self.book.row_height(r) as i32).sum();
        let cols: i32 = (0..fc).map(|c| self.book.col_width(c) as i32).sum();
        (if rows * 2 > h { 0 } else { fr }, if cols * 2 > w { 0 } else { fc })
    }

    /// The serial record of the cursor's column, so a gate can watch recalculation,
    /// sorting and filtering. Hidden (filtered) rows read as `~`.
    pub fn record_column(&self) {
        let sheets = self.book.sheets();
        let sheet = sheets.get(self.book.active_sheet()).cloned().unwrap_or_default();
        let cells: Vec<String> = (0..8).map(|r| if self.book.row_height(r) == 0 { "~".into() } else { self.book.shown(r, self.col).text }).collect();
        sex_std::record(&format!("[spreadeagle.column] sheet={sheet} col={} text={}", column_name(self.col), cells.join("|")));
    }

    fn commit(&mut self) -> bool {
        let Some(edit) = self.edit.take() else { return false };
        let text: String = edit.text.iter().collect();
        match self.book.set(self.row, self.col, &text) {
            Ok(()) => {
                sex_std::record(&format!("[spreadeagle.set] cell={} input={text}", self.cell_name()));
                self.message.clear();
            }
            Err(e) => self.message = format!("{}: {e}", self.cell_name()),
        }
        true
    }

    fn move_to(&mut self, row: i64, col: i64) {
        self.row = row.clamp(0, LAST_ROW as i64) as u32;
        self.col = col.clamp(0, LAST_COL as i64) as u32;
    }

    /// Keeps the cursor on screen for a grid area `w` x `h`, below and right of the
    /// frozen rows and columns.
    fn scroll_into_view(&mut self, w: i32, h: i32) {
        let (fr, fc) = self.frozen();
        let fh: i32 = (0..fr).map(|r| self.book.row_height(r) as i32).sum();
        let fw: i32 = (0..fc).map(|c| self.book.col_width(c) as i32).sum();
        self.top = self.top.max(fr);
        self.left = self.left.max(fc);
        if self.row >= fr {
            if self.row < self.top { self.top = self.row; }
            loop {
                let y: i32 = (self.top..=self.row).map(|r| self.book.row_height(r) as i32).sum();
                if y <= h - fh || self.top >= self.row { break; }
                self.top += 1;
            }
        }
        if self.col >= fc {
            if self.col < self.left { self.left = self.col; }
            loop {
                let x: i32 = (self.left..=self.col).map(|c| self.book.col_width(c) as i32).sum();
                if x <= w - fw || self.left >= self.col { break; }
                self.left += 1;
            }
        }
    }

    /// The grid's left edge of column `c`, frozen columns first (may be off screen).
    fn col_x(&self, c: u32) -> i32 {
        let (_, fc) = self.frozen();
        let width = |a: u32, b: u32| (a..b).map(|i| self.book.col_width(i) as i32).sum::<i32>();
        if c < fc { return ROW_HEAD_W + width(0, c); }
        let base = ROW_HEAD_W + width(0, fc);
        if c >= self.left { base + width(self.left, c) } else { base - width(c, self.left) }
    }
    fn row_y(&self, r: u32) -> i32 {
        let (fr, _) = self.frozen();
        let height = |a: u32, b: u32| (a..b).map(|i| self.book.row_height(i) as i32).sum::<i32>();
        let base = BAR_H + HEAD_H;
        if r < fr { return base + height(0, r); }
        let base = base + height(0, fr);
        if r >= self.top { base + height(self.top, r) } else { base - height(r, self.top) }
    }

    pub fn draw(&mut self, window: &mut sex_app::Window) {
        let palette = self.palette;
        let (width, height) = window.size();
        let (w, h) = (width as i32, height as i32);
        let grid_w = w - ROW_HEAD_W;
        let grid_h = h - BAR_H - HEAD_H - TABS_H;
        self.grid = (grid_w, grid_h);
        self.scroll_into_view(grid_w, grid_h);
        let (fr, fc) = self.frozen();
        let mut px = std::mem::take(&mut self.rows);
        px.clear();
        let mut canvas = Canvas::new(window.pixels(), width, height);
        canvas.fill(0, 0, w, h, palette.bg);

        // Formula bar: the cell's name (or the selection's), then what it holds or what is being typed.
        canvas.fill(0, 0, w, BAR_H, palette.bar_bg);
        canvas.fill(0, BAR_H - 1, w, 1, palette.grid);
        let name = if self.anchor.is_some() { self.selection().a1() } else { self.cell_name() };
        self.pen.text(&mut canvas, PAD + 2, (BAR_H - self.pen.line_height()) / 2, &name, palette.ink, Some((0, 90)));
        canvas.fill(96, 4, 1, BAR_H - 8, palette.grid);
        let (content, caret) = match &self.edit {
            Some(e) => (e.text.iter().collect::<String>(), Some(e.text[..e.caret].iter().collect::<String>())),
            None => (self.book.input(self.row, self.col), None),
        };
        let ty = (BAR_H - self.pen.line_height()) / 2;
        self.pen.text(&mut canvas, 104, ty, &content, palette.ink, Some((104, w - 108)));
        if let Some(before) = caret {
            let x = 104 + self.pen.measure(&before);
            canvas.fill(x, 6, 2, BAR_H - 12, palette.cursor);
        }

        // Which rows and columns show: the frozen ones, then from the scroll position on.
        let top_y = BAR_H;
        let grid_bottom = h - TABS_H;
        let mut cols = std::mem::take(&mut self.cols);
        cols.clear();
        let mut x = ROW_HEAD_W;
        for c in (0..fc).chain(self.left..=LAST_COL) {
            if x >= w { break; }
            let cw = self.book.col_width(c) as i32;
            if cw > 0 { cols.push((c, x, cw)); }
            x += cw;
        }
        let mut y = top_y + HEAD_H;
        for r in (0..fr).chain(self.top..=LAST_ROW) {
            if y >= grid_bottom { break; }
            let rh = self.book.row_height(r) as i32;
            if rh > 0 { px.push((r, y, rh)); }
            y += rh;
        }
        let rows = px;

        // Headings, the selection's tint and the grid lines.
        let sel = self.selection();
        canvas.fill(0, top_y, w, HEAD_H, palette.head_bg);
        canvas.fill(0, top_y, ROW_HEAD_W, h - top_y - TABS_H, palette.head_bg);
        let (st, sl, sb, sr) = sel.bounds();
        for &(c, x, cw) in &cols {
            if (sl..=sr).contains(&c) { canvas.fill(x, top_y, cw, HEAD_H, palette.head_on); }
            let label = column_name(c);
            let lw = self.pen.measure(&label);
            self.pen.text(&mut canvas, x + (cw - lw) / 2, top_y + (HEAD_H - self.pen.line_height()) / 2, &label, palette.dim, Some((x, cw)));
        }
        for &(r, y, rh) in &rows {
            if (st..=sb).contains(&r) { canvas.fill(0, y, ROW_HEAD_W, rh, palette.head_on); }
            let label = format!("{}", r + 1);
            let lw = self.pen.measure(&label);
            self.pen.text(&mut canvas, ROW_HEAD_W - PAD - lw, y + (rh - self.pen.line_height()) / 2, &label, palette.dim, Some((0, ROW_HEAD_W)));
        }
        if !sel.single() {
            for &(r, y, rh) in &rows {
                for &(c, x, cw) in &cols {
                    if sel.contains(r, c) && (r, c) != (self.row, self.col) { canvas.tint(x, y, cw, rh, palette.cursor, 30); }
                }
            }
        }
        for &(c, x, cw) in &cols {
            let color = if fc > 0 && c + 1 == fc { palette.frozen } else { palette.grid };
            canvas.fill(x + cw - 1, top_y, if color == palette.frozen { 2 } else { 1 }, grid_bottom - top_y, color);
        }
        for &(r, y, rh) in &rows {
            let color = if fr > 0 && r + 1 == fr { palette.frozen } else { palette.grid };
            canvas.fill(0, y + rh - 1, w, if color == palette.frozen { 2 } else { 1 }, color);
        }
        canvas.fill(ROW_HEAD_W - 1, top_y, 1, h - top_y - TABS_H, palette.grid);
        canvas.fill(0, top_y + HEAD_H - 1, w, 1, palette.grid);

        // Cells. Text runs into empty neighbours on its right, as in every spreadsheet.
        for &(r, y, rh) in &rows {
            for (i, &(c, x, cw)) in cols.iter().enumerate() {
                let shown = self.book.shown(r, c);
                if shown.text.is_empty() { continue; }
                let color = if shown.error { palette.error } else { palette.ink };
                let (bold, _, underline) = self.book.font(r, c);
                if self.book.wrapped(r, c) {
                    let height = self.pen.line_height();
                    let lines = crate::book::wrap_lines(&shown.text, cw - 2 * PAD,
                        ((rh - 2 * PAD) / height).max(1) as usize, |line| self.pen.measure(line));
                    let clip = Some((x + 1, cw - 2));
                    for (n, line) in lines.iter().enumerate() {
                        let tw = self.pen.measure(line);
                        let tx = if shown.right { x + cw - PAD - tw } else { x + PAD };
                        let ty = y + PAD + n as i32 * height;
                        self.pen.text(&mut canvas, tx, ty, line, color, clip);
                        if bold { self.pen.text(&mut canvas, tx + 1, ty, line, color, clip); }
                        if underline {
                            let ux = tx.max(x + 1);
                            canvas.fill(ux, ty + height - 2, ((tx + tw + bold as i32).min(x + cw - 1) - ux).max(0), 1, color);
                        }
                    }
                    continue;
                }
                let tw = self.pen.measure(&shown.text);
                let mut room = cw;
                if !shown.right {
                    for &(c2, x2, w2) in &cols[i + 1..] {
                        if room >= tw + 2 * PAD || x2 != x + room || !self.book.shown(r, c2).text.is_empty() { break; }
                        room += w2;
                    }
                }
                let tx = if shown.right { x + cw - PAD - tw } else { x + PAD };
                let ty = y + (rh - self.pen.line_height()) / 2;
                let clip = Some((x + 1, room - 2));
                self.pen.text(&mut canvas, tx, ty, &shown.text, color, clip);
                // One face for now (sex-font): bold is the text drawn again a pixel right.
                if bold { self.pen.text(&mut canvas, tx + 1, ty, &shown.text, color, clip); }
                if underline {
                    let ux = tx.max(x + 1);
                    canvas.fill(ux, ty + self.pen.line_height() - 2, (tx + tw + bold as i32).min(x + room - 1) - ux, 1, color);
                }
            }
        }

        // AutoFilter buttons on the filter's header row.
        if let Some(f) = self.book.filter_range() {
            let (ft, fl, _, fr2) = f.bounds();
            if let Some(&(_, y, rh)) = rows.iter().find(|r| r.0 == ft) {
                for &(_, x, cw) in cols.iter().filter(|c| (fl..=fr2).contains(&c.0)) {
                    let (bx, by, s) = (x + cw - 17, y + (rh - 14) / 2, 14);
                    canvas.fill(bx, by, s, s, palette.bg);
                    canvas.frame(bx, by, s, s, palette.frozen);
                    for i in 0..4 { canvas.fill(bx + 3 + i, by + 5 + i, 8 - 2 * i, 1, palette.dim); }
                }
            }
        }

        // The copied cells, outlined as Excel's marching ants are (still: no timer here).
        if let Some((range, _)) = self.copied {
            let (t, l, b, r) = range.bounds();
            let (x0, y0) = (self.col_x(l), self.row_y(t));
            let (x1, y1) = (self.col_x(r) + self.book.col_width(r) as i32, self.row_y(b) + self.book.row_height(b) as i32);
            if x1 > ROW_HEAD_W && y1 > top_y + HEAD_H {
                let (x0, y0) = (x0.max(ROW_HEAD_W - 1), y0.max(top_y + HEAD_H - 1));
                for i in 0..2 { canvas.frame(x0 - 1 + i, y0 - 1 + i, x1 - x0 + 2 - 2 * i, y1 - y0 + 2 - 2 * i, palette.accent); }
            }
        }

        // The edge being dragged.
        if let Some((c, _, _, w)) = self.resizing {
            if let Some(&(_, cx, _)) = cols.iter().find(|k| k.0 == c) { canvas.fill(cx + w - 1, top_y, 2, h - top_y - TABS_H, palette.cursor); }
        }

        // The cursor cell.
        if let (Some(&(_, cx, cw)), Some(&(_, cy, ch))) = (cols.iter().find(|c| c.0 == self.col), rows.iter().find(|r| r.0 == self.row)) {
            canvas.frame(cx - 1, cy - 1, cw + 1, ch + 1, palette.cursor);
            canvas.frame(cx, cy, cw - 1, ch - 1, palette.cursor);
            canvas.fill(cx + cw - 4, cy + ch - 4, 5, 5, palette.cursor);
        }

        // Charts, over the cells at their anchors.
        let clip = (ROW_HEAD_W, top_y + HEAD_H, w - ROW_HEAD_W, grid_bottom - top_y - HEAD_H);
        for (i, placed) in self.book.charts().into_iter().enumerate() {
            let (cx, cy) = (self.col_x(placed.col) + placed.dx, self.row_y(placed.row) + placed.dy);
            if cx >= w || cy >= grid_bottom || cx + placed.width < ROW_HEAD_W || cy + placed.height < clip.1 { continue; }
            let (cw, ch) = (placed.width as u32, placed.height as u32);
            let Some(picture) = self.book.chart_picture(i, cw, ch, &Measure(self)) else { continue };
            canvas.blit(cx, cy, &picture.pixels, cw as usize, clip);
            let inside = |x: i32, y: i32| x >= clip.0 && y >= clip.1 && y < clip.1 + clip.3;
            for label in &picture.text {
                let (tw, lh) = self.with_pen(label.size, |pen| (pen.measure(&label.text), pen.line_height()));
                let lx = cx + label.x as i32 - match label.align { -1 => 0, 0 => tw / 2, _ => tw };
                let ly = cy + label.y as i32 - match label.valign { -1 => 0, 0 => lh / 2, _ => lh };
                if !inside(lx, ly) || ly + lh > clip.1 + clip.3 { continue; }
                let right = (cx + cw as i32).min(w);
                self.with_pen(label.size, |pen| pen.text(&mut canvas, lx, ly, &label.text, label.color, Some((cx.max(clip.0), right - cx.max(clip.0)))));
            }
            canvas.frame(cx, cy, placed.width, placed.height, palette.chart_edge);
        }

        // Sheet tabs and the status line.
        let ty0 = h - TABS_H;
        canvas.fill(0, ty0, w, TABS_H, palette.head_bg);
        canvas.fill(0, ty0, w, 1, palette.grid);
        let mut tx = 8;
        let active = self.book.active_sheet();
        for (i, sheet) in self.book.sheets().iter().enumerate() {
            let tw = self.pen.measure(sheet) + 24;
            if i == active {
                canvas.fill(tx, ty0, tw, TABS_H - 3, palette.tab_on);
                canvas.fill(tx, ty0 + TABS_H - 5, tw, 2, palette.accent);
            }
            self.pen.text(&mut canvas, tx + 12, ty0 + (TABS_H - self.pen.line_height()) / 2, sheet, if i == active { palette.accent } else { palette.dim }, None);
            tx += tw + 2;
        }
        let status = if let Some(ask) = &self.ask {
            let said = |hint: &str| if self.message.is_empty() { hint.to_string() } else { self.message.clone() };
            match ask {
                Ask::SaveAs(name) => format!("Save as: {}_   {}", name.iter().collect::<String>(), said("xlsx, ods, csv, tsv, html or pdf; Enter saves, Esc closes")),
                Ask::GoTo(place) => format!("Go to: {}_   {}", place.iter().collect::<String>(), said("a cell like B7, B7:D9 or Sheet2!B7; Enter goes, Esc closes")),
                Ask::Lines { insert: true } => format!("Insert {}: R rows above, C columns to the left, Esc cancels", self.selection().a1()),
                Ask::Lines { insert: false } => format!("Delete {}: R its rows, C its columns, Esc cancels", self.selection().a1()),
                Ask::Chart => format!("Chart of {}: C column, L line, B bar, P pie, D doughnut, Esc cancels", self.selection().a1()),
                Ask::ChartEdit { text, .. } => format!("Edit chart: {}_   {}", text.iter().collect::<String>(), said("remove 2, data A1:B4, switch, type/title, hole, angle")),
                Ask::MultiSort { range, text } => format!("Sort {}: {}_   {}", range.a1(), text.iter().collect::<String>(), said("columns in priority order: A asc, C desc; Enter sorts")),
                Ask::FilterCondition { col, text } => format!("Filter {}: {}_   {}", column_name(*col), text.iter().collect::<String>(), said("= or <> text (*, ? wildcards); numeric; join two with and/or")),
            }
        } else if let Some(p) = &self.find {
            let what: String = p.what.iter().collect();
            match &p.with {
                None => {
                    let said = if self.message.is_empty() { "Enter finds the next, Esc closes" } else { &self.message };
                    format!("Find: {what}_   {said}")
                }
                Some(with) => {
                    let with: String = with.iter().collect();
                    let said = if self.message.is_empty() { "Tab switches, Enter replaces all, Esc closes" } else { &self.message };
                    let (a, b) = if p.on_with { ("", "_") } else { ("_", "") };
                    format!("Find: {what}{a}   Replace with: {with}{b}   {said}")
                }
            }
        } else if let Some(menu) = &self.filter_menu {
            format!("Filter {}: Up/Down chooses, Space marks, C condition, Enter applies, Esc closes ({} values)", column_name(menu.col), menu.values.len())
        } else if let Some(picker) = &self.function_picker {
            format!("Function: {}_   Type to search names, categories or descriptions; Up/Down chooses, Enter inserts, Esc closes ({} matches)", picker.query, picker.matches().len())
        } else if self.message.is_empty() {
            format!("{}{}  Ctrl+S save  Ctrl+Shift+S save as  Ctrl+P PDF  Ctrl+Z undo  Ctrl+C/X/V copy  Ctrl+F find  Ctrl+H replace  Ctrl+G go to  Ctrl+Shift+=/- insert/delete  Ctrl+D fill  Ctrl+Shift+T sum  Ctrl+Shift+A/D sort  L filter  F freeze  C chart",
                self.name, if self.book.dirty() { " (changed)" } else { "" })
        } else {
            self.message.clone()
        };
        let sw = self.pen.measure(&status);
        let sx = (w - sw - 10).max(tx + 16);
        self.pen.text(&mut canvas, sx, ty0 + (TABS_H - self.pen.line_height()) / 2, &status, palette.dim, Some((sx, w - sx)));
        if let Some(menu) = &self.filter_menu {
            let (mx, my) = self.filter_menu_rect(menu.col, w, h);
            let count = menu.visible_len().min(FILTER_MENU_VISIBLE);
            let mh = 26 + count as i32 * FILTER_MENU_ROW;
            canvas.fill(mx + 3, my + 3, FILTER_MENU_W, mh, palette.grid);
            canvas.fill(mx, my, FILTER_MENU_W, mh, palette.bar_bg);
            canvas.frame(mx, my, FILTER_MENU_W, mh, palette.grid);
            let title = if menu.condition { "condition active" } else if menu.active { "values active" } else { "choose values" };
            let heading = if menu.searching || !menu.query.is_empty() {
                format!("{} / {}_ ({})", column_name(menu.col), menu.query, menu.visible_len() - 1)
            } else { format!("{}: {title}  / search", column_name(menu.col)) };
            self.pen.text(&mut canvas, mx + 8, my + 5, &heading, palette.ink, Some((mx + 8, FILTER_MENU_W - 16)));
            let first = menu.selected.saturating_sub(FILTER_MENU_VISIBLE - 1);
            for slot in 0..count {
                let index = first + slot;
                let value = menu.value_index(index);
                let label = if index == 0 { "All rows" } else if let Some(i) = value {
                    menu.values[i].as_str()
                } else { menu.exact.as_deref().unwrap_or("All rows") };
                let y = my + 25 + slot as i32 * FILTER_MENU_ROW;
                if index == menu.selected { canvas.fill(mx + 3, y, FILTER_MENU_W - 6, FILTER_MENU_ROW, palette.head_on); }
                let mark = if value.is_none() { "    " } else if value.is_some_and(|i| menu.checked[i]) { "[x] " } else { "[ ] " };
                let shown = format!("{mark}{}", if label.is_empty() { "(blank)" } else { label });
                self.pen.text(&mut canvas, mx + 9, y + 3, &shown, if index == menu.selected { palette.accent } else { palette.ink }, Some((mx + 9, FILTER_MENU_W - 18)));
            }
        }
        if let Some(picker) = &self.function_picker {
            let matches = picker.matches();
            let visible = matches.len().min(8);
            let row_h = self.pen.line_height() + 7;
            let (bw, bh) = ((w - 2 * PAD).min(620).max(1), 64 + visible as i32 * row_h);
            let (bx, by) = ((w - bw) / 2, ((h - bh) / 2).max(0));
            canvas.fill(bx + 3, by + 3, bw, bh, palette.grid);
            canvas.fill(bx, by, bw, bh, palette.bar_bg);
            canvas.frame(bx, by, bw, bh, palette.grid);
            self.pen.text(&mut canvas, bx + 10, by + 7, &format!("Worksheet functions: {}", picker.query), palette.ink, Some((bx, bw - 20)));
            let first = picker.selected.saturating_sub(visible.saturating_sub(1));
            for (slot, &index) in matches.iter().skip(first).take(visible).enumerate() {
                let f = &picker.entries[index];
                let y = by + 31 + slot as i32 * row_h;
                if first + slot == picker.selected { canvas.fill(bx + 4, y - 2, bw - 8, row_h, palette.head_on); }
                self.pen.text(&mut canvas, bx + 10, y, &format!("{}  ·  {}", f.name, f.category), palette.ink, Some((bx + 10, bw - 20)));
            }
            if let Some(&index) = matches.get(picker.selected) {
                let f = &picker.entries[index];
                self.pen.text(&mut canvas, bx + 10, by + bh - 29, &f.signature, palette.accent, Some((bx + 10, bw - 20)));
                self.pen.text(&mut canvas, bx + 10, by + bh - 15, &f.description, palette.dim, Some((bx + 10, bw - 20)));
            }
        }
        if self.help {
            let row = self.pen.line_height() + 6;
            let bw = (w - 2 * PAD).min(540).max(0);
            let bh = (row * (SHORTCUTS.len() as i32 + 2)).min(h - 2 * PAD);
            if bw > 0 && bh > 0 {
                let bx = (w - bw) / 2;
                let by = ((h - bh) / 2).max(0);
                canvas.fill(bx + 3, by + 3, bw, bh, palette.grid);
                canvas.fill(bx, by, bw, bh, palette.bar_bg);
                canvas.frame(bx, by, bw, bh, palette.grid);
                self.pen.text(&mut canvas, bx + 12, by + 8, "Spreadeagle keyboard shortcuts (Esc closes)", palette.ink, Some((bx, bw - 16)));
                for (i, (shortcut, action)) in SHORTCUTS.iter().enumerate() {
                    let y = by + row * (i as i32 + 1) + 8;
                    if y + row > by + bh { break; }
                    self.pen.text(&mut canvas, bx + 12, y, shortcut, palette.accent, Some((bx, 190)));
                    self.pen.text(&mut canvas, bx + 200, y, action, palette.ink, Some((bx + 200, (bw - 212).max(0))));
                }
            }
        }
        if self.warning_open {
            let bw = (w - 2 * PAD).min(680).max(1);
            let lines = warning_lines(&self.import_warnings[self.warning_index], 75);
            let row = self.pen.line_height() + 8;
            let bh = (84 + row * lines.len() as i32).min(h - 2 * PAD).max(1);
            let (bx, by) = ((w - bw) / 2, (h - bh) / 2);
            canvas.fill(bx + 4, by + 4, bw, bh, palette.grid);
            canvas.fill(bx, by, bw, bh, palette.bar_bg);
            canvas.frame(bx, by, bw, bh, palette.error);
            self.pen.text(&mut canvas, bx + 14, by + 12,
                &format!("Import warning {} of {}", self.warning_index + 1, self.import_warnings.len()),
                palette.error, Some((bx + 14, bw - 28)));
            for (i, line) in lines.iter().enumerate() {
                self.pen.text(&mut canvas, bx + 14, by + 41 + i as i32 * row, line,
                    palette.ink, Some((bx + 14, bw - 28)));
            }
            self.pen.text(&mut canvas, bx + 14, by + bh - 25,
                "Up/Down reviews warnings  ·  Enter or Esc closes  ·  Ctrl+Shift+W reopens",
                palette.dim, Some((bx + 14, bw - 28)));
        }

        self.rows = rows;
        self.cols = cols;
        window.show_pixels(width, height);
        let short: String = self.name.chars().take(15).collect();
        window.title(format!("{short}{}", if self.book.dirty() { "*" } else { "" }).as_bytes());
    }

    fn save(&mut self) {
        match self.book.save() {
            Ok(bytes) => {
                self.recovered = false;
                self.unkept = None;
                sex_std::record(&format!("[spreadeagle.saved] name={} bytes={bytes}", self.book.path));
                let saved = self.book.path.rsplit('/').next().unwrap_or(&self.book.path).to_string();
                if saved != self.name { self.name = saved; }
                self.message = format!("Saved {} ({bytes} bytes)", self.name);
            }
            Err(e) => {
                sex_std::record(&format!("[spreadeagle.save-failed] {e}"));
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
            Ok(()) => sex_std::record(&format!("[spreadeagle.kept] name={}", crate::book::recovery_path(&self.book.path))),
            Err(e) => sex_std::record(&format!("[spreadeagle.keep-failed] {e}")),
        }
    }

    /// Writes the active sheet as a PDF beside the workbook.
    fn export_pdf(&mut self) {
        match self.book.export_pdf() {
            Ok((path, bytes)) => {
                sex_std::record(&format!("[spreadeagle.pdf] name={path} bytes={bytes}"));
                self.message = format!("Wrote {} ({bytes} bytes)", path.rsplit('/').next().unwrap_or(&path));
            }
            Err(e) => {
                sex_std::record(&format!("[spreadeagle.pdf-failed] {e}"));
                self.message = e;
            }
        }
    }

    /// A key while a Save As, Go To or insert/delete prompt is open. Returns whether
    /// cells (or the sheet) changed.
    fn ask_key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        let Some(ask) = &mut self.ask else { return false };
        if key.code == code::ESC { self.ask = None; self.message.clear(); return false; }
        match ask {
            Ask::Lines { insert } => {
                let insert = *insert;
                let rows = match key.code { code::R => true, code::C => false, _ => return false };
                self.ask = None;
                let sel = self.selection();
                let what = if rows { "rows" } else { "columns" };
                let done = if insert { self.book.insert_lines(sel, rows) } else { self.book.delete_lines(sel, rows) };
                let id = format!("home.{}{}", if insert { "insert" } else { "delete" }, if rows { "Rows" } else { "Columns" });
                self.command(&id, done, format!("{} the {what} of {}", if insert { "Inserted" } else { "Deleted" }, sel.a1()))
            }
            Ask::Chart => {
                if key.control { return false; }
                let kind = match key.code {
                    code::C => "column", code::L => "line", code::B => "bar", code::P => "pie", code::D => "doughnut",
                    _ => return false,
                };
                self.ask = None;
                let sel = self.selection();
                let (top, _, _, right) = sel.bounds();
                let done = self.book.add_chart_kind(sel, top, right + 2, kind);
                let added = done.is_ok();
                self.command("insert.chart", done, format!("{} chart of {}", kind, sel.a1()));
                if let Some(chart) = self.book.charts().last().filter(|_| added) {
                    sex_std::record(&format!("[spreadeagle.chart.type] kind={kind} at={}{} width={} height={}",
                        column_name(chart.col), u64::from(chart.row) + 1, chart.width, chart.height));
                }
                false
            }
            Ask::ChartEdit { id, text } => {
                let id = *id;
                match key.code {
                    code::BACKSPACE => { text.pop(); self.message.clear(); return false; }
                    code::ENTER => {}
                    _ => {
                        if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) { text.push(ch); self.message.clear(); }
                        return false;
                    }
                }
                let typed = text.iter().collect::<String>();
                match self.book.edit_chart(id, &typed) {
                    Ok(description) => {
                        self.ask = None;
                        sex_std::record(&format!("[spreadeagle.chart.edit] id={id} action={typed}"));
                        self.message = description;
                        true
                    }
                    Err(e) => { self.message = e; false }
                }
            }
            Ask::MultiSort { range, text } => {
                let range = *range;
                match key.code {
                    code::BACKSPACE => { text.pop(); self.message.clear(); return false; }
                    code::ENTER => {}
                    _ => {
                        if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) { text.push(ch); self.message.clear(); }
                        return false;
                    }
                }
                let typed = text.iter().collect::<String>();
                let done = self.book.sort_multi(range, &typed);
                if done.is_ok() { self.ask = None; }
                let changed = self.command("data.sort", done, format!("Sorted {} by {typed}", range.a1()));
                if changed { sex_std::record(&format!("[spreadeagle.sort.multi] range={} keys={typed}", range.a1())); }
                changed
            }
            Ask::FilterCondition { col, text } => {
                let col = *col;
                match key.code {
                    code::BACKSPACE => { text.pop(); self.message.clear(); return false; }
                    code::ENTER => {}
                    _ => {
                        if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) { text.push(ch); self.message.clear(); }
                        return false;
                    }
                }
                let typed = text.iter().collect::<String>();
                let at = self.selection();
                let done = self.book.filter_by_condition(at, col, &typed);
                if done.is_ok() { self.ask = None; }
                let changed = self.command("data.filterBy", done, format!("Filter {}: {typed}", column_name(col)));
                if changed { sex_std::record(&format!("[spreadeagle.filter.condition] col={} rule={typed}", column_name(col))); }
                changed
            }
            Ask::SaveAs(text) | Ask::GoTo(text) => {
                match key.code {
                    code::BACKSPACE => { text.pop(); self.message.clear(); return false; }
                    code::ENTER => {}
                    _ => {
                        if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) { text.push(ch); self.message.clear(); }
                        return false;
                    }
                }
                let typed: String = text.iter().collect::<String>().trim().to_string();
                if typed.is_empty() { return false; }
                if matches!(ask, Ask::SaveAs(_)) {
                    match self.book.save_as(&typed) {
                        Ok((path, bytes)) => {
                            self.ask = None;
                            sex_std::record(&format!("[spreadeagle.saved-as] name={path} bytes={bytes}"));
                            let short = path.rsplit('/').next().unwrap_or(&path).to_string();
                            if path == self.book.path { self.name = short.clone(); self.recovered = false; }
                            self.message = format!("Saved {short} ({bytes} bytes)");
                        }
                        Err(e) => self.message = e,
                    }
                    return false;
                }
                match self.book.go_to(&typed) {
                    Ok(range) => {
                        self.ask = None;
                        self.commit();
                        self.anchor = if range.single() { None } else { Some((range.r0, range.c0)) };
                        self.row = range.r1;
                        self.col = range.c1;
                        sex_std::record(&format!("[spreadeagle.goto] place={typed} cell={}", self.cell_name()));
                        self.message = format!("Went to {}", range.a1());
                        true
                    }
                    Err(e) => { self.message = e; false }
                }
            }
        }
    }

    fn switch_sheet(&mut self, by: i64) {
        let n = self.book.sheets().len() as i64;
        if n == 0 { return; }
        let to = (self.book.active_sheet() as i64 + by).rem_euclid(n) as usize;
        if self.book.activate(to).is_ok() { self.home(); }
    }

    fn home(&mut self) { self.row = 0; self.col = 0; self.top = 0; self.left = 0; self.anchor = None; }

    /// Runs one of the data commands and says how it went. Returns whether cells changed.
    fn command(&mut self, id: &str, done: Result<(), String>, said: String) -> bool {
        match done {
            Ok(()) => {
                sex_std::record(&format!("[spreadeagle.command] {id}"));
                self.message = said;
                true
            }
            Err(e) => {
                sex_std::record(&format!("[spreadeagle.command-failed] {id} {e}"));
                self.message = e;
                false
            }
        }
    }

    fn filter_menu_rect(&self, col: u32, width: i32, height: i32) -> (i32, i32) {
        let header = self.book.filter_range().map_or(0, |r| r.bounds().0);
        let x = (self.col_x(col) + self.book.col_width(col) as i32 - 17)
            .clamp(ROW_HEAD_W, (width - FILTER_MENU_W).max(ROW_HEAD_W));
        let y = self.row_y(header) + self.book.row_height(header) as i32;
        (x, y.clamp(BAR_H + HEAD_H, (height - TABS_H - 26 - FILTER_MENU_ROW * FILTER_MENU_VISIBLE as i32).max(BAR_H + HEAD_H)))
    }

    fn open_filter_menu(&mut self, col: u32) {
        let Some(range) = self.book.filter_range() else {
            self.message = "Turn the filter on with Ctrl+Shift+L first".into();
            return;
        };
        let (_, left, _, right) = range.bounds();
        if !(left..=right).contains(&col) { return; }
        let values = self.book.filter_choices(col);
        sex_std::record(&format!("[spreadeagle.filter.menu] col={} values={}", column_name(col), values.len()));
        let menu = FilterMenu::new(col, values, self.book.filter_state(col));
        sex_std::record(&format!("[spreadeagle.filter.menu.state] col={} active={} selected={}", column_name(col),
            if menu.condition { "condition" } else if menu.active { "values" } else { "none" }, menu.checked.iter().filter(|&&on| on).count()));
        self.filter_menu = Some(menu);
    }

    fn apply_filter_menu(&mut self) -> bool {
        let Some(menu) = self.filter_menu.take() else { return false };
        if menu.untouched_active() {
            sex_std::record(&format!("[spreadeagle.filter.menu.unchanged] col={}", column_name(menu.col)));
            return false;
        }
        let chosen = menu.selected_value_index();
        let exact = menu.selected_exact().map(str::to_string);
        let values: Vec<String> = if let Some(value) = exact {
            vec![value]
        } else if menu.toggled {
            menu.values.into_iter().zip(menu.checked).filter_map(|(value, checked)| checked.then_some(value)).collect()
        } else {
            chosen.map(|i| menu.values[i].clone()).into_iter().collect()
        };
        let label = match values.as_slice() { [] => "all".to_string(), [one] => one.clone(), _ => format!("{}-values", values.len()) };
        let done = self.book.filter_by_values(self.selection(), menu.col, &values);
        let changed = self.command("data.filterBy", done, format!("Filter {}: {label}", column_name(menu.col)));
        if changed { sex_std::record(&format!("[spreadeagle.filter.choice] col={} value={label}", column_name(menu.col))); }
        changed
    }

    fn filter_menu_key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        if key.code == code::ESC {
            if let Some(menu) = self.filter_menu.as_mut().filter(|m| !m.query.is_empty()) {
                menu.clear_query(); menu.searching = false; return false;
            }
            self.filter_menu = None; return false;
        }
        if key.code == code::ENTER { return self.apply_filter_menu(); }
        if (key.code == code::SLASH && !key.control) || (key.code == code::F && key.control && !key.shift) {
            if let Some(menu) = self.filter_menu.as_mut() { menu.searching = true; }
            return false;
        }
        if key.code == code::C && !key.control && self.filter_menu.as_ref().is_some_and(|m| !m.searching) {
            let col = self.filter_menu.take().unwrap().col;
            self.ask = Some(Ask::FilterCondition { col, text: Vec::new() });
            sex_std::record(&format!("[spreadeagle.filter.condition-prompt] col={}", column_name(col)));
            return false;
        }
        if let Some(menu) = &mut self.filter_menu {
            if key.code == code::BACKSPACE && menu.searching {
                menu.pop_query();
                if menu.results.is_empty() { menu.offer_exact(self.book.filter_exact_choice(menu.col, &menu.query)); }
                return false;
            }
            if key.code == code::SPACE && menu.searching && !key.control {
                menu.push_query(' ');
                if menu.results.is_empty() { menu.offer_exact(self.book.filter_exact_choice(menu.col, &menu.query)); }
                return false;
            }
            if key.code == code::SPACE && (!key.control || menu.searching) {
                if menu.selected_exact().is_some() { return false; }
                menu.toggled = true;
                if menu.selected == 0 { menu.checked.fill(false); }
                else if let Some(i) = menu.selected_value_index() { menu.checked[i] = !menu.checked[i]; }
                return false;
            }
            match sex_app::nav(key) {
                Some(Nav::Up) => menu.selected = menu.selected.saturating_sub(1),
                Some(Nav::Down) => menu.selected = (menu.selected + 1).min(menu.visible_len().saturating_sub(1)),
                _ => if menu.searching {
                    if let Some(c) = sex_app::text(key).filter(|c| !c.is_control() && !key.control) {
                        menu.push_query(c);
                        if menu.results.is_empty() {
                            menu.offer_exact(self.book.filter_exact_choice(menu.col, &menu.query));
                        }
                        sex_std::record(&format!("[spreadeagle.filter.search] col={} query={} matches={} selected={}",
                            column_name(menu.col), menu.query, menu.visible_len() - 1, menu.selected));
                    }
                },
            }
        }
        false
    }

    /// Ctrl+Shift shortcuts: sort, filter, freeze, chart.
    fn data_key(&mut self, code: u8) -> bool {
        let sel = self.selection();
        let (row, col) = (self.row, self.col);
        match code {
            code::A | code::D => {
                let asc = code == code::A;
                let done = self.book.sort(sel, col, asc);
                self.command(if asc { "data.sortAscending" } else { "data.sortDescending" }, done, format!("Sorted by column {}, {}", column_name(col), if asc { "A to Z" } else { "Z to A" }))
            }
            code::L => {
                let done = self.book.toggle_filter(sel);
                self.filter_menu = None;
                let on = self.book.filter_range().is_some();
                self.command("data.filter", done, if on { "Filter on: Ctrl+Shift+E shows only rows like the cursor's cell".into() } else { "Filter off".into() })
            }
            code::E => {
                let value = self.book.shown(row, col).text;
                let done = self.book.filter_by(sel, col, Some(&value));
                self.command("data.filterBy", done, format!("Showing rows where {} is {value}", column_name(col)))
            }
            code::O => { self.open_filter_menu(col); false }
            code::F => {
                let at = if self.book.frozen().is_none() && (row, col) != (0, 0) { Some((row, col)) } else { None };
                let done = self.book.freeze(at);
                let said = match at { Some(_) => format!("Froze the rows above and the columns left of {}", self.cell_name()), None => "Panes unfrozen".into() };
                self.command("view.freezePanes", done, said);
                false
            }
            code::C => {
                let (t, _, _, r) = sel.bounds();
                let done = self.book.add_chart(sel, t, r + 2);
                self.command("insert.chart", done, format!("Chart of {}", sel.a1()));
                if let Some(c) = self.book.charts().last() { sex_std::record(&format!("[spreadeagle.chart] at={}{} width={} height={}", column_name(c.col), u64::from(c.row) + 1, c.width, c.height)); }
                false
            }
            _ => false,
        }
    }

    fn function_picker_key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        if key.code == code::ESC { self.function_picker = None; return false; }
        let Some(picker) = &mut self.function_picker else { return false };
        match (key.code, sex_app::nav(key)) {
            (code::BACKSPACE, _) => { picker.query.pop(); picker.selected = 0; }
            (_, Some(Nav::Up)) => picker.selected = picker.selected.saturating_sub(1),
            (_, Some(Nav::Down)) => picker.selected = (picker.selected + 1).min(picker.matches().len().saturating_sub(1)),
            (code::ENTER, _) => {
                let Some(&index) = picker.matches().get(picker.selected) else { return false };
                let name = picker.entries[index].name.clone();
                let formula = format!("={name}()");
                self.edit = Some(Edit { text: formula.chars().collect(), caret: name.chars().count() + 2, replace: false });
                self.anchor = None;
                self.function_picker = None;
                self.message = format!("{}: type arguments, then Enter", name);
                sex_std::record(&format!("[spreadeagle.functions.insert] name={name} at={}", self.cell_name()));
            }
            _ => if !key.control && picker.query.chars().count() < 60 {
                if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) { picker.query.push(ch); picker.selected = 0; }
            },
        }
        false
    }

    /// One key. Returns whether the cells (or the sheet) changed: worth a record of the column.
    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        if key.control && key.shift && key.code == code::W && !self.import_warnings.is_empty() {
            self.warning_open = true;
            self.warning_index = 0;
            sex_std::record(&format!("[spreadeagle.warnings.shown] count={}", self.import_warnings.len()));
            return false;
        }
        if self.warning_open {
            match sex_app::nav(key) {
                Some(Nav::Up) => self.warning_index = self.warning_index.saturating_sub(1),
                Some(Nav::Down) => self.warning_index = (self.warning_index + 1).min(self.import_warnings.len() - 1),
                _ if matches!(key.code, code::ENTER | code::ESC) => {
                    self.warning_open = false;
                    self.message = format!("Imported with {} warning(s). Ctrl+Shift+W reviews them.", self.import_warnings.len());
                    sex_std::record("[spreadeagle.warnings.closed]");
                }
                _ => {}
            }
            return false;
        }
        if key.control && key.shift && key.code == code::SLASH {
            self.help = !self.help;
            sex_std::record(&format!("[spreadeagle.shortcuts] shown={}", self.help as u8));
            return false;
        }
        if self.help {
            if key.code == code::ESC { self.help = false; }
            return false;
        }
        if self.function_picker.is_some() { return self.function_picker_key(key); }
        let (row, col) = (self.row as i64, self.col as i64);
        if self.ask.is_some() { return self.ask_key(key); }
        if self.find.is_some() { return self.find_key(key); }
        if self.filter_menu.is_some() { return self.filter_menu_key(key); }
        if self.recovered && key.code == code::ESC && !key.control && self.edit.is_none() {
            return self.discard_recovered();
        }
        if key.control && self.edit.is_none() {
            let ask = match key.code {
                code::I if key.shift => {
                    match self.book.functions() {
                        Ok(entries) => {
                            sex_std::record(&format!("[spreadeagle.functions.open] count={}", entries.len()));
                            self.function_picker = Some(FunctionPicker { entries, query: String::new(), selected: 0 });
                        }
                        Err(e) => self.message = e,
                    }
                    return false;
                }
                code::U if key.shift => {
                    let range = self.selection();
                    let on = !self.book.wrapped(self.row, self.col);
                    let done = self.book.set_wrap(range, on);
                    if done.is_ok() {
                        sex_std::record(&format!("[spreadeagle.wrap] range={} on={} row_height={} cell_y={}", range.a1(), on as u8, self.book.row_height(self.row), self.row_y(self.row)));
                    }
                    self.command("home.wrapText", done, format!("{} {}", range.a1(), if on { "wrapped" } else { "unwrapped" }));
                    return true;
                }
                code::S if key.shift => Some(Ask::SaveAs(self.name.chars().collect())),
                code::G => Some(Ask::GoTo(Vec::new())),
                code::M if key.shift => { sex_std::record("[spreadeagle.chart.choose]"); Some(Ask::Chart) },
                code::Q if key.shift => {
                    if let Some(chart) = self.book.chart_at_anchor(self.row, self.col) {
                        sex_std::record(&format!("[spreadeagle.chart.edit-prompt] id={} at={}{}", chart.id, column_name(chart.col), u64::from(chart.row) + 1));
                        Some(Ask::ChartEdit { id: chart.id, text: Vec::new() })
                    } else { self.message = "Select a chart or go to its anchor cell first".into(); None }
                }
                code::K if key.shift => { sex_std::record(&format!("[spreadeagle.filter.condition-prompt] col={}", column_name(self.col))); Some(Ask::FilterCondition { col: self.col, text: Vec::new() }) },
                code::J if key.shift => {
                    let range = self.selection();
                    sex_std::record(&format!("[spreadeagle.sort.multi-prompt] range={}", range.a1()));
                    Some(Ask::MultiSort { range, text: Vec::new() })
                }
                code::EQUALS if key.shift => Some(Ask::Lines { insert: true }),
                code::MINUS => Some(Ask::Lines { insert: false }),
                code::P => { self.export_pdf(); return false; }
                _ => None,
            };
            if let Some(ask) = ask { self.message.clear(); self.ask = Some(ask); return false; }
        }
        if key.control {
            if key.shift && self.edit.is_none() && matches!(key.code, code::A | code::D | code::E | code::F | code::L | code::C | code::O) {
                return self.data_key(key.code);
            }
            if key.shift && self.edit.is_none() && (code::digit(key.code).is_some() || key.code == code::GRAVE) {
                return self.format_key(key.code);
            }
            if self.edit.is_none() && matches!(key.code, code::C | code::X | code::V | code::D | code::R | code::B | code::I | code::U | code::F | code::H | code::T) {
                return self.edit_key(key.code, key.shift);
            }
            match key.code {
                code::S => { self.commit(); self.save(); return false; }
                code::Z => { self.edit = None; let _ = self.book.undo(); return true; }
                code::Y => { self.edit = None; let _ = self.book.redo(); return true; }
                code::A if self.edit.is_some() => { if let Some(e) = &mut self.edit { e.caret = 0; } return false; }
                c if code::page_up(c) => { self.commit(); self.switch_sheet(-1); return true; }
                c if code::page_down(c) => { self.commit(); self.switch_sheet(1); return true; }
                _ => {}
            }
            if let Some(Nav::Home) = sex_app::nav(key) { self.commit(); self.anchor = None; self.move_to(0, 0); }
            return false;
        }
        if let Some(edit) = &mut self.edit {
            match (key.code, sex_app::nav(key)) {
                (code::ESC, _) => { self.edit = None; return false; }
                (code::ENTER, _) => { let changed = self.commit(); self.step_rows(if key.shift { -1 } else { 1 }); return changed; }
                (code::TAB, _) => { let changed = self.commit(); self.move_to(row, col + if key.shift { -1 } else { 1 }); return changed; }
                (code::BACKSPACE, _) => { if edit.caret > 0 { edit.caret -= 1; edit.text.remove(edit.caret); } return false; }
                (_, Some(Nav::Delete)) => { if edit.caret < edit.text.len() { edit.text.remove(edit.caret); } return false; }
                (_, Some(nav)) if !edit.replace => {
                    match nav {
                        Nav::Left => edit.caret = edit.caret.saturating_sub(1),
                        Nav::Right => edit.caret = (edit.caret + 1).min(edit.text.len()),
                        Nav::Home => edit.caret = 0,
                        Nav::End => edit.caret = edit.text.len(),
                        _ => {}
                    }
                    return false;
                }
                (_, Some(nav)) => {
                    let changed = self.commit();
                    self.step(nav, false);
                    return changed;
                }
                _ => {}
            }
            if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) {
                edit.text.insert(edit.caret, ch);
                edit.caret += 1;
            }
            return false;
        }
        match (key.code, sex_app::nav(key)) {
            (code::ENTER | code::F2, _) => {
                let text: Vec<char> = self.book.input(self.row, self.col).chars().collect();
                self.edit = Some(Edit { caret: text.len(), text, replace: false });
            }
            (code::ESC, _) => { self.anchor = None; self.copied = None; }
            (code::TAB, _) => { self.anchor = None; self.move_to(row, col + if key.shift { -1 } else { 1 }); }
            (code::BACKSPACE, _) | (_, Some(Nav::Delete)) => {
                let sel = self.selection();
                let done = if sel.single() { self.book.clear(self.row, self.col) } else { self.book.clear_range(sel) };
                if done.is_ok() { return true; }
            }
            (c, _) if code::page_up(c) => { self.anchor = None; self.step_rows(-(PAGE as i64)); }
            (c, _) if code::page_down(c) => { self.anchor = None; self.step_rows(PAGE as i64); }
            (_, Some(nav)) => self.step(nav, key.shift),
            _ => {
                if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) {
                    self.anchor = None;
                    self.edit = Some(Edit { text: vec![ch], caret: 1, replace: true });
                }
            }
        }
        false
    }

    /// Esc on a workbook that came back with unsaved changes: goes back to the saved file.
    fn discard_recovered(&mut self) -> bool {
        self.recovered = false;
        self.book.forget_recovery();
        let path = self.book.path.clone();
        let opened = if std::fs::metadata(&path).is_ok() { Book::open(&path).map(|(b, _)| b) } else { Ok(Book::blank(&path)) };
        match opened {
            Ok(book) => {
                self.book = book;
                self.home();
                sex_std::record(&format!("[spreadeagle.discarded] name={path}"));
                self.message = format!("Threw the kept changes away: {} is as last saved", self.name);
                true
            }
            Err(e) => { self.message = e; false }
        }
    }

    /// Ctrl shortcuts on the cells: clipboard, fill, styles, AutoSum and find, on GridCraft's
    /// keys (Excel's). Returns whether cells changed.
    fn edit_key(&mut self, code: u8, shift: bool) -> bool {
        let sel = self.selection();
        match code {
            code::C | code::X => {
                let cut = code == code::X;
                let id = if cut { "edit.cut" } else { "edit.copy" };
                let done = self.book.copy(sel, cut);
                let n = done.as_ref().map_or(0, |t| t.lines().count());
                if done.is_ok() { self.copied = Some((sel, cut)); }
                self.command(id, done.map(|_| ()), format!("{} {} ({n} rows). Ctrl+V pastes at the cursor", if cut { "Cut" } else { "Copied" }, sel.a1()));
                false
            }
            code::V => {
                let done = self.book.paste(self.row, self.col);
                let to = done.clone().unwrap_or_default();
                if done.is_ok() && self.copied.is_some_and(|(_, cut)| cut) { self.copied = None; }
                if let Ok(at) = &done { sex_std::record(&format!("[spreadeagle.paste] at={at}")); }
                self.command("edit.paste", done.map(|_| ()), format!("Pasted into {to}"))
            }
            code::D | code::R => {
                let down = code == code::D;
                let done = self.book.fill(sel, down);
                self.command(if down { "edit.fillDown" } else { "edit.fillRight" }, done, format!("Filled {} {}", sel.a1(), if down { "down" } else { "right" }))
            }
            code::B | code::I | code::U => {
                let style = match code { code::B => "bold", code::I => "italic", _ => "underline" };
                let done = self.book.toggle_style(sel, style);
                let on = *done.as_ref().unwrap_or(&false);
                let said = format!("{} {} {}", sel.a1(), if on { "now" } else { "no longer" }, style);
                self.command(&format!("home.{style}"), done.map(|_| ()), said);
                true
            }
            code::T if shift => {
                let done = self.book.auto_sum(self.row, self.col);
                let said = format!("{} = {}", self.cell_name(), done.clone().unwrap_or_default());
                self.command("formulas.autoSum", done.map(|_| ()), said)
            }
            code::F => { self.find = Some(Prompt { what: Vec::new(), with: None, on_with: false }); false }
            code::H => { self.find = Some(Prompt { what: Vec::new(), with: Some(Vec::new()), on_with: false }); false }
            _ => false,
        }
    }

    /// Ctrl+Shift+~ and Ctrl+Shift+1..6: Excel's number formats on the selection.
    fn format_key(&mut self, code: u8) -> bool {
        let format = match code::digit(code) {
            Some(1) => "Number",
            Some(2) => "Time",
            Some(3) => "Short Date",
            Some(4) => "Currency",
            Some(5) => "Percentage",
            Some(6) => "Scientific",
            _ => "General",
        };
        let sel = self.selection();
        let done = self.book.number_format(sel, format);
        self.command("home.numberFormat", done, format!("{} as {format}", sel.a1()))
    }

    /// A key while the find (or find and replace) prompt is open.
    fn find_key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        let Some(p) = &mut self.find else { return false };
        match key.code {
            code::ESC => { self.find = None; }
            code::BACKSPACE => { p.field().pop(); self.message.clear(); }
            code::TAB if p.with.is_some() => { p.on_with = !p.on_with; }
            code::ENTER if p.with.is_some() && !p.on_with => { p.on_with = true; }
            code::ENTER => {
                let what: String = p.what.iter().collect();
                if what.is_empty() { return false; }
                if let Some(with) = &p.with {
                    let with: String = with.iter().collect();
                    let done = self.book.replace_all(&what, &with);
                    let n = *done.as_ref().unwrap_or(&0);
                    sex_std::record(&format!("[spreadeagle.replace] what={what} with={with} cells={n}"));
                    self.message = match &done { Ok(_) => format!("Replaced \"{what}\" in {n} cells"), Err(e) => e.clone() };
                    return n > 0;
                }
                match self.book.find(self.row, self.col, &what) {
                    Ok((sheet, row, col, n)) => {
                        self.anchor = None;
                        self.row = row;
                        self.col = col;
                        let sheets = self.book.sheets();
                        let name = sheets.get(sheet).cloned().unwrap_or_default();
                        sex_std::record(&format!("[spreadeagle.find] what={what} sheet={name} cell={}", self.cell_name()));
                        self.message = format!("Found \"{what}\" in {name}!{} ({n} cells match)", self.cell_name());
                        // The cursor moved, maybe to another sheet: worth a record.
                        return true;
                    }
                    Err(e) => { sex_std::record(&format!("[spreadeagle.find] what={what} none")); self.message = e; }
                }
            }
            _ => {
                if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) { p.field().push(ch); self.message.clear(); }
            }
        }
        false
    }

    /// Moves `by` visible rows: filtered-out rows are stepped over.
    fn step_rows(&mut self, by: i64) {
        let mut row = self.row as i64;
        for _ in 0..by.unsigned_abs() {
            let mut next = row + by.signum();
            while (0..=LAST_ROW as i64).contains(&next) && self.book.row_height(next as u32) == 0 { next += by.signum(); }
            if !(0..=LAST_ROW as i64).contains(&next) { break; }
            row = next;
        }
        self.move_to(row, self.col as i64);
    }

    /// An arrow: moves the cursor, or with Shift stretches the selection.
    fn step(&mut self, nav: Nav, extend: bool) {
        if extend { self.anchor.get_or_insert((self.row, self.col)); } else { self.anchor = None; }
        let (row, col) = (self.row as i64, self.col as i64);
        match nav {
            Nav::Left => self.move_to(row, col - 1),
            Nav::Right => self.move_to(row, col + 1),
            Nav::Up => self.step_rows(-1),
            Nav::Down => self.step_rows(1),
            Nav::Home => self.move_to(row, 0),
            Nav::End | Nav::Delete => {}
        }
    }

    /// A click picks a cell or a sheet tab; the wheel scrolls.
    fn pointer(&mut self, p: Pointer, window: &sex_app::Window) {
        if self.function_picker.is_some() { return; }
        let (_, height) = window.size();
        let (x, y) = (p.x as i32, p.y as i32);
        match p.kind {
            Kind::Wheel { steps } => {
                let (fr, _) = self.frozen();
                self.top = (self.top as i64 - steps as i64 * 3).max(fr as i64) as u32;
                // Keep the cursor where the rows are, as scrolling a sheet does.
                if self.row >= fr && self.row < self.top { self.row = self.top; }
            }
            Kind::Down { button: 0 } => {
                if let Some(col) = self.filter_menu.as_ref().map(|menu| menu.col) {
                    let (mx, my) = self.filter_menu_rect(col, window.size().0 as i32, height as i32);
                    if x >= mx && x < mx + FILTER_MENU_W && y >= my + 25 {
                        let slot = ((y - my - 25) / FILTER_MENU_ROW) as usize;
                        let menu = self.filter_menu.as_mut().unwrap();
                        let first = menu.selected.saturating_sub(FILTER_MENU_VISIBLE - 1);
                        let index = first + slot;
                        if slot < FILTER_MENU_VISIBLE && index < menu.visible_len() {
                            menu.selected = index;
                            if self.apply_filter_menu() { self.record_column(); }
                        }
                    } else { self.filter_menu = None; }
                    return;
                }
                if y >= height as i32 - TABS_H {
                    let mut tx = 8;
                    for (i, sheet) in self.book.sheets().iter().enumerate() {
                        let tw = self.pen.measure(sheet) + 24;
                        if x >= tx && x < tx + tw { self.commit(); if self.book.activate(i).is_ok() { self.home(); } return; }
                        tx += tw + 2;
                    }
                    return;
                }
                // A column header's right edge: drag it to change the width.
                if (BAR_H..BAR_H + HEAD_H).contains(&y) {
                    if let Some(&(c, _, cw)) = self.cols.iter().find(|c| (x - (c.1 + c.2)).abs() <= 3) {
                        self.resizing = Some((c, x, cw, cw));
                        return;
                    }
                }
                // A row header's bottom edge changes that row's height.
                if (0..ROW_HEAD_W).contains(&x) {
                    if let Some(&(r, _, rh)) = self.rows.iter().find(|r| (y - (r.1 + r.2)).abs() <= 3) {
                        self.resizing_row = Some((r, y, rh, rh));
                        return;
                    }
                }
                if let Some(range) = self.book.filter_range() {
                    let (header, left, _, right) = range.bounds();
                    if let Some(&(_, ry, rh)) = self.rows.iter().find(|r| r.0 == header) {
                        if let Some(&(c, cx, cw)) = self.cols.iter().find(|c| (left..=right).contains(&c.0) && x >= c.1 + c.2 - 17 && x < c.1 + c.2 - 3) {
                            let by = ry + (rh - 14) / 2;
                            if y >= by && y < by + 14 && x >= cx + cw - 17 {
                                self.open_filter_menu(c);
                                return;
                            }
                        }
                    }
                }
                if let Some(chart) = self.book.charts().into_iter().rev().find(|chart| {
                    let cx = self.col_x(chart.col) + chart.dx;
                    let cy = self.row_y(chart.row) + chart.dy;
                    x >= cx && x < cx + chart.width && y >= cy && y < cy + chart.height
                        && y < height as i32 - TABS_H
                }) {
                    self.commit();
                    self.anchor = None;
                    self.row = chart.row;
                    self.col = chart.col;
                    self.message = "Chart selected. Ctrl+Shift+Q edits its title or type".into();
                    sex_std::record(&format!("[spreadeagle.chart.selected] id={} at={}{}", chart.id, column_name(chart.col), u64::from(chart.row) + 1));
                    return;
                }
                let (Some(&(c, ..)), Some(&(r, ..))) = (self.cols.iter().find(|c| x >= c.1 && x < c.1 + c.2), self.rows.iter().find(|r| y >= r.1 && y < r.1 + r.2)) else { return };
                self.commit();
                self.anchor = None;
                self.row = r;
                self.col = c;
            }
            Kind::Move => {
                if let Some((c, x0, w0, _)) = self.resizing { self.resizing = Some((c, x0, w0, (w0 + x - x0).clamp(8, 1800))); }
                if let Some((r, y0, h0, _)) = self.resizing_row { self.resizing_row = Some((r, y0, h0, (h0 + y - y0).clamp(8, 546))); }
            }
            Kind::Up { button: 0 } => {
                if let Some((c, _, w0, w)) = self.resizing.take() {
                    if w != w0 {
                        let done = self.book.set_col_width(c, w as u32);
                        sex_std::record(&format!("[spreadeagle.width] col={} width={}", column_name(c), self.book.col_width(c)));
                        self.command("home.columnWidth", done, format!("Column {} is {w} pixels wide", column_name(c)));
                    }
                }
                if let Some((r, _, h0, h)) = self.resizing_row.take() {
                    if h != h0 {
                        let done = self.book.set_row_height(r, h as u32);
                        sex_std::record(&format!("[spreadeagle.height] row={} height={}", r + 1, self.book.row_height(r)));
                        self.command("home.rowHeight", done, format!("Row {} is {h} pixels high", r + 1));
                    }
                }
            }
            _ => {}
        }
    }

    /// Opens with last time's unsaved changes: says so, and Esc throws them away.
    pub fn recovered(mut self) -> Ui {
        self.recovered = true;
        self.message = "Brought back the changes not saved last time: Ctrl+S keeps them, Esc throws them away".into();
        self
    }

    pub fn run(mut self, window: &mut sex_app::Window) {
        self.draw(window);
        let mut theme_next = sex_std::sex_pdx::get_ticks().saturating_add(1000);
        let frozen = self.book.frozen().map_or("none".into(), |(r, c)| format!("{r},{c}"));
        sex_std::record(&format!("[spreadeagle.view] charts={} frozen={frozen}", self.book.charts().len()));
        sex_std::record(&format!("[spreadeagle.columns] A={} B={}", self.book.col_width(0), self.book.col_width(1)));
        sex_std::record(&format!("[spreadeagle.rows] 1={} 2={}", self.book.row_height(0), self.book.row_height(1)));
        self.record_column();
        loop {
            let now = sex_std::sex_pdx::get_ticks();
            if now >= theme_next {
                theme_next = now.saturating_add(1000);
                if let Ok(generation) = sex_app::settings::generation() {
                    if generation != self.theme_generation {
                        self.theme_generation = generation;
                        let theme = sex_app::settings::theme();
                        self.palette = Palette::from_tokens(theme);
                        sex_std::record(&format!("[spreadeagle.theme] mode={:?} accent={:06x}", theme.mode, self.palette.accent));
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
                // A cell being typed into counts: commit it, then keep.
                Event::Close => { self.commit(); self.keep(); break; }
                Event::Resize { .. } => self.draw(window),
                Event::Key(key) => {
                    let col = self.col;
                    let changed = self.key(key);
                    if changed && self.book.dirty() {
                        self.unkept.get_or_insert(sex_std::sex_pdx::get_ticks());
                        // An edit makes the brought-back changes the user's own: Esc no longer throws them away.
                        if !matches!(key.code, code::ESC) { self.recovered = false; }
                    }
                    self.draw(window);
                    if changed || col != self.col { self.record_column(); }
                }
                Event::Pointer(p) => {
                    if matches!(p.kind, Kind::Down { .. } | Kind::Wheel { .. }) || self.resizing.is_some() {
                        let col = self.col;
                        self.pointer(p, window);
                        self.draw(window);
                        if col != self.col { self.record_column(); }
                    }
                }
                // Drag and drop: nothing to take yet.
                _ => {}
            }
        }
        sex_std::record("[spreadeagle.closed]");
    }
}
