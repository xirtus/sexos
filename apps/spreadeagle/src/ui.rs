// SPDX-License-Identifier: GPL-3.0-only
//! The grid, the formula bar and the sheet tabs, painted into the window with
//! sex-office until GridCraft's own egui interface runs on SexOS (workstream M11).
//! Frozen rows and columns stay put while the rest scrolls; filtered-out rows are
//! skipped; charts float over the cells at their anchors.
use crate::book::{Book, Range, column_name};
use sex_office::{Canvas, Pen};
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx::app_pointer::{Kind, Pointer};
use std::cell::RefCell;

const BG: u32 = 0x00ffffff;
const GRID: u32 = 0x00dadce0;
const HEAD_BG: u32 = 0x00f1f3f4;
const HEAD_ON: u32 = 0x00d3e3fd;
const INK: u32 = 0x00202124;
const DIM: u32 = 0x005f6368;
const ERROR: u32 = 0x00c5221f;
const CURSOR: u32 = 0x001a73e8;
const BAR_BG: u32 = 0x00f8f9fa;
const TAB_ON: u32 = 0x00ffffff;
const ACCENT: u32 = 0x000b8043;
const FROZEN: u32 = 0x00bdc1c6;
const CHART_EDGE: u32 = 0x00c4c7c5;

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
}

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
    pub const C: u8 = 0x2E;
    pub const X: u8 = 0x2D;
    pub const V: u8 = 0x2F;
    pub const R: u8 = 0x13;
    pub const T: u8 = 0x14;
    pub const B: u8 = 0x30;
    pub const I: u8 = 0x17;
    pub const U: u8 = 0x16;
    pub const F2: u8 = 0x3C;
    pub const H: u8 = 0x23;
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
        Ui {
            book, name, row: 0, col: 0, anchor: None, top: 0, left: 0, edit: None, message: String::new(),
            pen: Pen::sans(13), small: RefCell::new(Vec::new()), rows: Vec::new(), cols: Vec::new(), grid: (i32::MAX, i32::MAX), copied: None, find: None, resizing: None,
        }
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
        canvas.fill(0, 0, w, h, BG);

        // Formula bar: the cell's name (or the selection's), then what it holds or what is being typed.
        canvas.fill(0, 0, w, BAR_H, BAR_BG);
        canvas.fill(0, BAR_H - 1, w, 1, GRID);
        let name = if self.anchor.is_some() { self.selection().a1() } else { self.cell_name() };
        self.pen.text(&mut canvas, PAD + 2, (BAR_H - self.pen.line_height()) / 2, &name, INK, Some((0, 90)));
        canvas.fill(96, 4, 1, BAR_H - 8, GRID);
        let (content, caret) = match &self.edit {
            Some(e) => (e.text.iter().collect::<String>(), Some(e.text[..e.caret].iter().collect::<String>())),
            None => (self.book.input(self.row, self.col), None),
        };
        let ty = (BAR_H - self.pen.line_height()) / 2;
        self.pen.text(&mut canvas, 104, ty, &content, INK, Some((104, w - 108)));
        if let Some(before) = caret {
            let x = 104 + self.pen.measure(&before);
            canvas.fill(x, 6, 2, BAR_H - 12, CURSOR);
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
        canvas.fill(0, top_y, w, HEAD_H, HEAD_BG);
        canvas.fill(0, top_y, ROW_HEAD_W, h - top_y - TABS_H, HEAD_BG);
        let (st, sl, sb, sr) = sel.bounds();
        for &(c, x, cw) in &cols {
            if (sl..=sr).contains(&c) { canvas.fill(x, top_y, cw, HEAD_H, HEAD_ON); }
            let label = column_name(c);
            let lw = self.pen.measure(&label);
            self.pen.text(&mut canvas, x + (cw - lw) / 2, top_y + (HEAD_H - self.pen.line_height()) / 2, &label, DIM, Some((x, cw)));
        }
        for &(r, y, rh) in &rows {
            if (st..=sb).contains(&r) { canvas.fill(0, y, ROW_HEAD_W, rh, HEAD_ON); }
            let label = format!("{}", r + 1);
            let lw = self.pen.measure(&label);
            self.pen.text(&mut canvas, ROW_HEAD_W - PAD - lw, y + (rh - self.pen.line_height()) / 2, &label, DIM, Some((0, ROW_HEAD_W)));
        }
        if !sel.single() {
            for &(r, y, rh) in &rows {
                for &(c, x, cw) in &cols {
                    if sel.contains(r, c) && (r, c) != (self.row, self.col) { canvas.tint(x, y, cw, rh, CURSOR, 30); }
                }
            }
        }
        for &(c, x, cw) in &cols {
            let color = if fc > 0 && c + 1 == fc { FROZEN } else { GRID };
            canvas.fill(x + cw - 1, top_y, if color == FROZEN { 2 } else { 1 }, grid_bottom - top_y, color);
        }
        for &(r, y, rh) in &rows {
            let color = if fr > 0 && r + 1 == fr { FROZEN } else { GRID };
            canvas.fill(0, y + rh - 1, w, if color == FROZEN { 2 } else { 1 }, color);
        }
        canvas.fill(ROW_HEAD_W - 1, top_y, 1, h - top_y - TABS_H, GRID);
        canvas.fill(0, top_y + HEAD_H - 1, w, 1, GRID);

        // Cells. Text runs into empty neighbours on its right, as in every spreadsheet.
        for &(r, y, rh) in &rows {
            for (i, &(c, x, cw)) in cols.iter().enumerate() {
                let shown = self.book.shown(r, c);
                if shown.text.is_empty() { continue; }
                let tw = self.pen.measure(&shown.text);
                let mut room = cw;
                if !shown.right {
                    for &(c2, x2, w2) in &cols[i + 1..] {
                        if room >= tw + 2 * PAD || x2 != x + room || !self.book.shown(r, c2).text.is_empty() { break; }
                        room += w2;
                    }
                }
                let tx = if shown.right { x + cw - PAD - tw } else { x + PAD };
                let color = if shown.error { ERROR } else { INK };
                let ty = y + (rh - self.pen.line_height()) / 2;
                let clip = Some((x + 1, room - 2));
                let (bold, _, underline) = self.book.font(r, c);
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
                    canvas.fill(bx, by, s, s, BG);
                    canvas.frame(bx, by, s, s, FROZEN);
                    for i in 0..4 { canvas.fill(bx + 3 + i, by + 5 + i, 8 - 2 * i, 1, DIM); }
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
                for i in 0..2 { canvas.frame(x0 - 1 + i, y0 - 1 + i, x1 - x0 + 2 - 2 * i, y1 - y0 + 2 - 2 * i, ACCENT); }
            }
        }

        // The edge being dragged.
        if let Some((c, _, _, w)) = self.resizing {
            if let Some(&(_, cx, _)) = cols.iter().find(|k| k.0 == c) { canvas.fill(cx + w - 1, top_y, 2, h - top_y - TABS_H, CURSOR); }
        }

        // The cursor cell.
        if let (Some(&(_, cx, cw)), Some(&(_, cy, ch))) = (cols.iter().find(|c| c.0 == self.col), rows.iter().find(|r| r.0 == self.row)) {
            canvas.frame(cx - 1, cy - 1, cw + 1, ch + 1, CURSOR);
            canvas.frame(cx, cy, cw - 1, ch - 1, CURSOR);
            canvas.fill(cx + cw - 4, cy + ch - 4, 5, 5, CURSOR);
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
            canvas.frame(cx, cy, placed.width, placed.height, CHART_EDGE);
        }

        // Sheet tabs and the status line.
        let ty0 = h - TABS_H;
        canvas.fill(0, ty0, w, TABS_H, HEAD_BG);
        canvas.fill(0, ty0, w, 1, GRID);
        let mut tx = 8;
        let active = self.book.active_sheet();
        for (i, sheet) in self.book.sheets().iter().enumerate() {
            let tw = self.pen.measure(sheet) + 24;
            if i == active {
                canvas.fill(tx, ty0, tw, TABS_H - 3, TAB_ON);
                canvas.fill(tx, ty0 + TABS_H - 5, tw, 2, ACCENT);
            }
            self.pen.text(&mut canvas, tx + 12, ty0 + (TABS_H - self.pen.line_height()) / 2, sheet, if i == active { ACCENT } else { DIM }, None);
            tx += tw + 2;
        }
        let status = if let Some(p) = &self.find {
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
        } else if self.message.is_empty() {
            format!("{}{}  Ctrl+S save  Ctrl+Z undo  Ctrl+C/X/V copy  Ctrl+F find  Ctrl+H replace  Ctrl+D fill  Ctrl+Shift+T sum  Ctrl+Shift+A/D sort  L filter  F freeze  C chart",
                self.name, if self.book.dirty() { " (changed)" } else { "" })
        } else {
            self.message.clone()
        };
        let sw = self.pen.measure(&status);
        let sx = (w - sw - 10).max(tx + 16);
        self.pen.text(&mut canvas, sx, ty0 + (TABS_H - self.pen.line_height()) / 2, &status, DIM, Some((sx, w - sx)));

        self.rows = rows;
        self.cols = cols;
        window.show_pixels(width, height);
        let short: String = self.name.chars().take(15).collect();
        window.title(format!("{short}{}", if self.book.dirty() { "*" } else { "" }).as_bytes());
    }

    fn save(&mut self) {
        match self.book.save() {
            Ok(bytes) => {
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
                let on = self.book.filter_range().is_some();
                self.command("data.filter", done, if on { "Filter on: Ctrl+Shift+E shows only rows like the cursor's cell".into() } else { "Filter off".into() })
            }
            code::E => {
                let value = self.book.shown(row, col).text;
                let done = self.book.filter_by(sel, col, Some(&value));
                self.command("data.filterBy", done, format!("Showing rows where {} is {value}", column_name(col)))
            }
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

    /// One key. Returns whether the cells (or the sheet) changed: worth a record of the column.
    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        let (row, col) = (self.row as i64, self.col as i64);
        if self.find.is_some() { return self.find_key(key); }
        if key.control {
            if key.shift && self.edit.is_none() && matches!(key.code, code::A | code::D | code::E | code::F | code::L | code::C) {
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
                let (Some(&(c, ..)), Some(&(r, ..))) = (self.cols.iter().find(|c| x >= c.1 && x < c.1 + c.2), self.rows.iter().find(|r| y >= r.1 && y < r.1 + r.2)) else { return };
                self.commit();
                self.anchor = None;
                self.row = r;
                self.col = c;
            }
            Kind::Move => {
                if let Some((c, x0, w0, _)) = self.resizing { self.resizing = Some((c, x0, w0, (w0 + x - x0).clamp(8, 1800))); }
            }
            Kind::Up { button: 0 } => {
                if let Some((c, _, w0, w)) = self.resizing.take() {
                    if w != w0 {
                        let done = self.book.set_col_width(c, w as u32);
                        sex_std::record(&format!("[spreadeagle.width] col={} width={}", column_name(c), self.book.col_width(c)));
                        self.command("home.columnWidth", done, format!("Column {} is {w} pixels wide", column_name(c)));
                    }
                }
            }
            _ => {}
        }
    }

    pub fn run(mut self, window: &mut sex_app::Window) {
        self.draw(window);
        let frozen = self.book.frozen().map_or("none".into(), |(r, c)| format!("{r},{c}"));
        sex_std::record(&format!("[spreadeagle.view] charts={} frozen={frozen}", self.book.charts().len()));
        sex_std::record(&format!("[spreadeagle.columns] A={} B={}", self.book.col_width(0), self.book.col_width(1)));
        self.record_column();
        loop {
            match window.next() {
                Event::Close => break,
                Event::Resize { .. } => self.draw(window),
                Event::Key(key) => {
                    let col = self.col;
                    let changed = self.key(key);
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
