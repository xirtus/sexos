// SPDX-License-Identifier: GPL-3.0-only
//! The grid, the formula bar and the sheet tabs, painted into the window with
//! sex-office until GridCraft's own egui interface runs on SexOS (workstream M11).
use crate::book::{Book, column_name};
use sex_office::{Canvas, Pen};
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx::app_pointer::{Kind, Pointer};

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

const BAR_H: i32 = 30;
const HEAD_H: i32 = 22;
const ROW_HEAD_W: i32 = 48;
const TABS_H: i32 = 28;
const PAD: i32 = 4;
/// Rows a Page Up or Page Down moves.
const PAGE: u32 = 20;

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
    top: u32,
    left: u32,
    edit: Option<Edit>,
    message: String,
    pen: Pen,
    bold: Pen,
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
    pub fn page_up(c: u8) -> bool { matches!(c, 0x49 | 0x68) }
    pub fn page_down(c: u8) -> bool { matches!(c, 0x51 | 0x6D) }
}

impl Ui {
    pub fn new(book: Book, name: String) -> Ui {
        Ui { book, name, row: 0, col: 0, top: 0, left: 0, edit: None, message: String::new(), pen: Pen::sans(13), bold: Pen::sans(13) }
    }

    fn cell_name(&self) -> String { format!("{}{}", column_name(self.col), self.row + 1) }

    /// The serial record of the cursor's column, so a gate can watch recalculation.
    pub fn record_column(&self) {
        let sheets = self.book.sheets();
        let sheet = sheets.get(self.book.active_sheet()).cloned().unwrap_or_default();
        let cells: Vec<String> = (0..8).map(|r| self.book.shown(r, self.col).text).collect();
        sex_std::record(&format!("[gingham.column] sheet={sheet} col={} text={}", column_name(self.col), cells.join("|")));
    }

    fn commit(&mut self) -> bool {
        let Some(edit) = self.edit.take() else { return false };
        let text: String = edit.text.iter().collect();
        match self.book.set(self.row, self.col, &text) {
            Ok(()) => {
                sex_std::record(&format!("[gingham.set] cell={} input={text}", self.cell_name()));
                self.message.clear();
            }
            Err(e) => self.message = format!("{}: {e}", self.cell_name()),
        }
        true
    }

    fn move_to(&mut self, row: i64, col: i64) {
        self.row = row.clamp(0, 1_048_575) as u32;
        self.col = col.clamp(0, 16_383) as u32;
    }

    /// Keeps the cursor on screen for a grid area `w` x `h`.
    fn scroll_into_view(&mut self, w: i32, h: i32) {
        if self.row < self.top { self.top = self.row; }
        if self.col < self.left { self.left = self.col; }
        loop {
            let mut y = 0;
            for r in self.top..=self.row { y += self.book.row_height(r) as i32; }
            if y <= h || self.top >= self.row { break; }
            self.top += 1;
        }
        loop {
            let mut x = 0;
            for c in self.left..=self.col { x += self.book.col_width(c) as i32; }
            if x <= w || self.left >= self.col { break; }
            self.left += 1;
        }
    }

    pub fn draw(&mut self, window: &mut sex_app::Window) {
        let (width, height) = window.size();
        let (w, h) = (width as i32, height as i32);
        let grid_w = w - ROW_HEAD_W;
        let grid_h = h - BAR_H - HEAD_H - TABS_H;
        self.scroll_into_view(grid_w, grid_h);
        let mut canvas = Canvas::new(window.pixels(), width, height);
        canvas.fill(0, 0, w, h, BG);

        // Formula bar: the cell's name, then what it holds or what is being typed.
        canvas.fill(0, 0, w, BAR_H, BAR_BG);
        canvas.fill(0, BAR_H - 1, w, 1, GRID);
        let name = self.cell_name();
        self.bold.text(&mut canvas, PAD + 2, (BAR_H - self.pen.line_height()) / 2, &name, INK, Some((0, 70)));
        canvas.fill(76, 4, 1, BAR_H - 8, GRID);
        let (content, caret) = match &self.edit {
            Some(e) => (e.text.iter().collect::<String>(), Some(e.text[..e.caret].iter().collect::<String>())),
            None => (self.book.input(self.row, self.col), None),
        };
        let ty = (BAR_H - self.pen.line_height()) / 2;
        self.pen.text(&mut canvas, 84, ty, &content, INK, Some((84, w - 88)));
        if let Some(before) = caret {
            let x = 84 + self.pen.measure(&before);
            canvas.fill(x, 6, 2, BAR_H - 12, CURSOR);
        }

        // Column and row headings.
        let top_y = BAR_H;
        canvas.fill(0, top_y, w, HEAD_H, HEAD_BG);
        canvas.fill(0, top_y, ROW_HEAD_W, h - top_y - TABS_H, HEAD_BG);
        let mut cols = Vec::new();
        let mut x = ROW_HEAD_W;
        let mut c = self.left;
        while x < w {
            let cw = self.book.col_width(c) as i32;
            if cw > 0 {
                if c == self.col { canvas.fill(x, top_y, cw, HEAD_H, HEAD_ON); }
                let label = column_name(c);
                let lw = self.pen.measure(&label);
                self.pen.text(&mut canvas, x + (cw - lw) / 2, top_y + (HEAD_H - self.pen.line_height()) / 2, &label, DIM, Some((x, cw)));
                canvas.fill(x + cw - 1, top_y, 1, h - top_y - TABS_H, GRID);
                cols.push((c, x, cw));
            }
            x += cw;
            c += 1;
            if c > 16_383 { break; }
        }
        let mut rows = Vec::new();
        let mut y = top_y + HEAD_H;
        let mut r = self.top;
        while y < h - TABS_H {
            let rh = self.book.row_height(r) as i32;
            if rh > 0 {
                if r == self.row { canvas.fill(0, y, ROW_HEAD_W, rh, HEAD_ON); }
                let label = format!("{}", r + 1);
                let lw = self.pen.measure(&label);
                self.pen.text(&mut canvas, ROW_HEAD_W - PAD - lw, y + (rh - self.pen.line_height()) / 2, &label, DIM, Some((0, ROW_HEAD_W)));
                canvas.fill(0, y + rh - 1, w, 1, GRID);
                rows.push((r, y, rh));
            }
            y += rh;
            r += 1;
            if r > 1_048_575 { break; }
        }
        canvas.fill(ROW_HEAD_W - 1, top_y, 1, h - top_y - TABS_H, GRID);
        canvas.fill(0, top_y + HEAD_H - 1, w, 1, GRID);

        // Cells. Text runs into empty neighbours on its right, as in every spreadsheet.
        let grid_bottom = h - TABS_H;
        for &(r, y, rh) in &rows {
            if y >= grid_bottom { break; }
            for (i, &(c, x, cw)) in cols.iter().enumerate() {
                let shown = self.book.shown(r, c);
                if shown.text.is_empty() { continue; }
                let tw = self.pen.measure(&shown.text);
                let mut room = cw;
                if !shown.right {
                    for &(c2, _, w2) in &cols[i + 1..] {
                        if room >= tw + 2 * PAD || !self.book.shown(r, c2).text.is_empty() { break; }
                        room += w2;
                    }
                }
                let tx = if shown.right { x + cw - PAD - tw } else { x + PAD };
                let color = if shown.error { ERROR } else { INK };
                let clip = (x + 1, (room - 2).min(grid_bottom));
                self.pen.text(&mut canvas, tx, y + (rh - self.pen.line_height()) / 2, &shown.text, color, Some(clip));
            }
        }
        // The cursor cell.
        if let (Some(&(_, cx, cw)), Some(&(_, cy, ch))) = (cols.iter().find(|c| c.0 == self.col), rows.iter().find(|r| r.0 == self.row)) {
            canvas.frame(cx - 1, cy - 1, cw + 1, ch + 1, CURSOR);
            canvas.frame(cx, cy, cw - 1, ch - 1, CURSOR);
            canvas.fill(cx + cw - 4, cy + ch - 4, 5, 5, CURSOR);
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
        let status = if self.message.is_empty() {
            format!("{}{}  Ctrl+S save  Ctrl+Z undo  Ctrl+PgUp/PgDn sheets", self.name, if self.book.dirty() { " (changed)" } else { "" })
        } else {
            self.message.clone()
        };
        let sw = self.pen.measure(&status);
        let sx = (w - sw - 10).max(tx + 16);
        self.pen.text(&mut canvas, sx, ty0 + (TABS_H - self.pen.line_height()) / 2, &status, DIM, Some((sx, w - sx)));

        window.show_pixels(width, height);
        let short: String = self.name.chars().take(15).collect();
        window.title(format!("{short}{}", if self.book.dirty() { "*" } else { "" }).as_bytes());
    }

    fn save(&mut self) {
        match self.book.save() {
            Ok(bytes) => {
                sex_std::record(&format!("[gingham.saved] name={} bytes={bytes}", self.book.path));
                self.message = format!("Saved {} ({bytes} bytes)", self.name);
            }
            Err(e) => {
                sex_std::record(&format!("[gingham.save-failed] {e}"));
                self.message = e;
            }
        }
    }

    fn switch_sheet(&mut self, by: i64) {
        let n = self.book.sheets().len() as i64;
        if n == 0 { return; }
        let to = (self.book.active_sheet() as i64 + by).rem_euclid(n) as usize;
        if self.book.activate(to).is_ok() {
            self.row = 0; self.col = 0; self.top = 0; self.left = 0;
        }
    }

    /// One key. Returns whether the cells changed (worth a record of the column).
    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key) -> bool {
        let (row, col) = (self.row as i64, self.col as i64);
        if key.control {
            match key.code {
                code::S => { self.commit(); self.save(); return false; }
                code::Z => { self.edit = None; let _ = self.book.undo(); return true; }
                code::Y => { self.edit = None; let _ = self.book.redo(); return true; }
                code::A if self.edit.is_some() => { if let Some(e) = &mut self.edit { e.caret = 0; } return false; }
                c if code::page_up(c) => { self.commit(); self.switch_sheet(-1); return false; }
                c if code::page_down(c) => { self.commit(); self.switch_sheet(1); return false; }
                _ => {}
            }
            match sex_app::nav(key) {
                Some(Nav::Home) => { self.commit(); self.move_to(0, 0); }
                _ => {}
            }
            return false;
        }
        if let Some(edit) = &mut self.edit {
            match (key.code, sex_app::nav(key)) {
                (code::ESC, _) => { self.edit = None; return false; }
                (code::ENTER, _) => { let changed = self.commit(); self.move_to(row + if key.shift { -1 } else { 1 }, col); return changed; }
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
                    self.step(nav, row, col);
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
            (code::ENTER, _) => {
                let text: Vec<char> = self.book.input(self.row, self.col).chars().collect();
                self.edit = Some(Edit { caret: text.len(), text, replace: false });
            }
            (code::TAB, _) => self.move_to(row, col + if key.shift { -1 } else { 1 }),
            (code::BACKSPACE, _) | (_, Some(Nav::Delete)) => {
                if self.book.clear(self.row, self.col).is_ok() { return true; }
            }
            (c, _) if code::page_up(c) => self.move_to(row - PAGE as i64, col),
            (c, _) if code::page_down(c) => self.move_to(row + PAGE as i64, col),
            (_, Some(nav)) => self.step(nav, row, col),
            _ => {
                if let Some(ch) = sex_app::text(key).filter(|c| !c.is_control()) {
                    self.edit = Some(Edit { text: vec![ch], caret: 1, replace: true });
                }
            }
        }
        false
    }

    fn step(&mut self, nav: Nav, row: i64, col: i64) {
        match nav {
            Nav::Left => self.move_to(row, col - 1),
            Nav::Right => self.move_to(row, col + 1),
            Nav::Up => self.move_to(row - 1, col),
            Nav::Down => self.move_to(row + 1, col),
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
                self.top = (self.top as i64 - steps as i64 * 3).max(0) as u32;
                // Keep the cursor where the rows are, as scrolling a sheet does.
                if self.row < self.top { self.row = self.top; }
            }
            Kind::Down { button: 0 } => {
                if y >= height as i32 - TABS_H {
                    let mut tx = 8;
                    for (i, sheet) in self.book.sheets().iter().enumerate() {
                        let tw = self.pen.measure(sheet) + 24;
                        if x >= tx && x < tx + tw { self.commit(); let _ = self.book.activate(i); self.row = 0; self.col = 0; self.top = 0; self.left = 0; return; }
                        tx += tw + 2;
                    }
                    return;
                }
                if y < BAR_H + HEAD_H || x < ROW_HEAD_W { return; }
                self.commit();
                let mut cx = ROW_HEAD_W;
                let mut c = self.left;
                while cx + self.book.col_width(c) as i32 <= x { cx += self.book.col_width(c) as i32; c += 1; }
                let mut cy = BAR_H + HEAD_H;
                let mut r = self.top;
                while cy + self.book.row_height(r) as i32 <= y { cy += self.book.row_height(r) as i32; r += 1; }
                self.row = r;
                self.col = c;
            }
            _ => {}
        }
    }

    pub fn run(mut self, window: &mut sex_app::Window) {
        self.draw(window);
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
                    if matches!(p.kind, Kind::Down { .. } | Kind::Wheel { .. }) {
                        let col = self.col;
                        self.pointer(p, window);
                        self.draw(window);
                        if col != self.col { self.record_column(); }
                    }
                }
            }
        }
        sex_std::record("[gingham.closed]");
    }
}
