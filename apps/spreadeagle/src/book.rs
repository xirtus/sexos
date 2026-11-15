// SPDX-License-Identifier: GPL-3.0-only
//! The workbook behind the window: GridCraft's engine (storytold/gridcraft, MIT OR
//! Apache-2.0) holds the cells, recalculates, keeps undo, and reads and writes the
//! files. Nothing here draws or touches SexOS, so the host tests drive it directly.
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use serde_json::{Value as Json, json};

/// How a cell reads in the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub text: String,
    /// Numbers sit at the right of their cell, as in every spreadsheet.
    pub right: bool,
    pub error: bool,
}

/// Cells from (`r0`, `c0`) to (`r1`, `c1`), counted from 0, corners in any order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range { pub r0: u32, pub c0: u32, pub r1: u32, pub c1: u32 }

impl Range {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cell(row: u32, col: u32) -> Range { Range { r0: row, c0: col, r1: row, c1: col } }
    /// Top, left, bottom, right.
    pub fn bounds(&self) -> (u32, u32, u32, u32) { (self.r0.min(self.r1), self.c0.min(self.c1), self.r0.max(self.r1), self.c0.max(self.c1)) }
    pub fn single(&self) -> bool { self.r0 == self.r1 && self.c0 == self.c1 }
    pub fn contains(&self, row: u32, col: u32) -> bool {
        let (t, l, b, r) = self.bounds();
        (t..=b).contains(&row) && (l..=r).contains(&col)
    }
    pub fn a1(&self) -> String {
        let (t, l, b, r) = self.bounds();
        let one = CellRef::new(t, l).a1();
        if self.single() { one } else { format!("{one}:{}", CellRef::new(b, r).a1()) }
    }
}

/// A chart's place on the grid: its anchor cell, the offset into it and its size, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed { pub row: u32, pub col: u32, pub dx: i32, pub dy: i32, pub width: i32, pub height: i32 }

/// One piece of a chart's text, in the chart's pixels; `align` and `valign` are -1, 0 or 1
/// for left/top, centre and right/bottom of the anchor point.
pub struct Label { pub x: f32, pub y: f32, pub text: String, pub size: f32, pub color: u32, pub align: i8, pub valign: i8 }

/// A drawn chart: 0x00RRGGBB pixels at the asked size, with the text still to draw over them.
pub struct ChartPicture { pub pixels: Vec<u32>, pub text: Vec<Label> }

pub struct Book {
    session: Session,
    /// The file the workbook is saved to.
    pub path: String,
}

impl Book {
    /// An empty workbook that Ctrl+S saves to `path`.
    pub fn blank(path: &str) -> Book {
        let mut session = Session::new();
        session.new_workbook();
        Book { session, path: path.to_string() }
    }

    /// Opens an xlsx, xls, csv, tsv or GridCraft json file.
    pub fn open(path: &str) -> Result<(Book, Vec<String>), String> {
        let mut session = Session::new();
        let r = session.execute_headless("file.open", json!({ "path": path })).map_err(|e| e.to_string())?;
        let mut warnings: Vec<String> = r["warnings"].as_array().into_iter().flatten().filter_map(Json::as_str).map(str::to_string).collect();
        let mut book = Book { session, path: path.to_string() };
        if path.to_ascii_lowercase().ends_with(".ods") {
            match book.restore_ods_formulas(path) {
                Ok(0) => {}
                Ok(n) => {
                    warnings.retain(|w| !w.contains("formulas become their saved results"));
                    warnings.push(format!("{n} formulas read from the ODS file; Ctrl+S saves an .xlsx beside it."));
                }
                Err(e) => warnings.push(format!("ODS formulas were kept as values: {e}")),
            }
        }
        Ok((book, warnings))
    }

    /// Puts the file's formulas back over the values GridCraft imported, as part of the
    /// opened file: no undo step, not a change. Returns how many.
    fn restore_ods_formulas(&mut self, path: &str) -> Result<usize, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let found = crate::ods::formulas(&bytes)?;
        if found.is_empty() { return Ok(0); }
        let active = self.active_sheet();
        let mut sheet = usize::MAX;
        for f in &found {
            if f.sheet != sheet { self.activate(f.sheet)?; sheet = f.sheet; }
            self.set(f.row, f.col, &f.formula)?;
        }
        self.activate(active)?;
        if let Ok(d) = self.session.doc_mut() {
            d.undo.clear();
            d.redo.clear();
            d.saved = d.wb.clone();
        }
        Ok(found.len())
    }

    fn run(&mut self, id: &str, params: Json) -> Result<Json, String> {
        self.session.execute_headless(id, params).map_err(|e| e.to_string())
    }

    pub fn sheets(&self) -> Vec<String> {
        self.session.doc().map(|d| d.wb.sheets.iter().map(|s| s.name.clone()).collect()).unwrap_or_default()
    }
    pub fn active_sheet(&self) -> usize { self.session.doc().map_or(0, |d| d.wb.active_sheet()) }
    pub fn activate(&mut self, sheet: usize) -> Result<(), String> {
        self.run("sheet.activate", json!({ "sheet": sheet })).map(|_| ())
    }
    pub fn dirty(&self) -> bool { self.session.doc().is_ok_and(|d| d.is_dirty()) }

    /// What the grid shows at (`row`, `col`), counted from 0, on the active sheet.
    pub fn shown(&self, row: u32, col: u32) -> Shown {
        let Ok(d) = self.session.doc() else { return Shown { text: String::new(), right: false, error: false } };
        let Some(sheet) = d.wb.active() else { return Shown { text: String::new(), right: false, error: false } };
        let c = CellRef::new(row, col);
        let value = sheet.value(c);
        Shown {
            text: gridcraft_engine::display::cell_text(&d.wb, sheet, c),
            right: matches!(value, Value::Number(_)),
            error: matches!(value, Value::Error(_)),
        }
    }

    /// What was typed into the cell: its formula with `=`, or its constant.
    pub fn input(&self, row: u32, col: u32) -> String {
        self.session.doc().ok().and_then(|d| d.wb.active().map(|s| s.input_text(CellRef::new(row, col)))).unwrap_or_default()
    }

    /// Types `input` into the cell, as Enter does: a formula, a number, a date or text.
    pub fn set(&mut self, row: u32, col: u32, input: &str) -> Result<(), String> {
        let cell = CellRef::new(row, col).a1();
        self.run("cell.set", json!({ "cell": cell, "input": input })).map(|_| ())
    }
    pub fn clear(&mut self, row: u32, col: u32) -> Result<(), String> {
        let cell = CellRef::new(row, col).a1();
        self.run("selection.set", json!({ "range": cell }))?;
        self.run("edit.clearContents", json!({})).map(|_| ())
    }
    /// Bold, italic and underline of the cell's font.
    pub fn font(&self, row: u32, col: u32) -> (bool, bool, bool) {
        let Ok(d) = self.session.doc() else { return (false, false, false) };
        let Some(sheet) = d.wb.active() else { return (false, false, false) };
        let f = &d.wb.styles.get(sheet.style_id(CellRef::new(row, col))).font;
        (f.bold, f.italic, f.underline != gridcraft_engine::model::Underline::None)
    }

    /// Runs `id` on `range`, as the command would run on cells dragged over.
    fn run_on(&mut self, range: Range, id: &str, params: Json) -> Result<Json, String> {
        self.select(range)?;
        self.run(id, params)
    }
    /// Copies (or cuts) `range` to the workbook's clipboard; returns the copied text,
    /// tab-separated, as the system clipboard would get it.
    pub fn copy(&mut self, range: Range, cut: bool) -> Result<String, String> {
        let r = self.run_on(range, if cut { "edit.cut" } else { "edit.copy" }, json!({}))?;
        Ok(r.get("text").and_then(Json::as_str).unwrap_or_default().to_string())
    }
    /// Pastes the clipboard with its top-left corner at (`row`, `col`); a cut moves its cells.
    /// Returns the cells pasted, as A1:B2.
    pub fn paste(&mut self, row: u32, col: u32) -> Result<String, String> {
        let r = self.run_on(Range::cell(row, col), "edit.paste", json!({}))?;
        Ok(r.get("pasted").and_then(Json::as_str).unwrap_or_default().to_string())
    }
    /// Empties every cell of `range` (values and formulas; formats stay).
    pub fn clear_range(&mut self, range: Range) -> Result<(), String> {
        self.run_on(range, "edit.clearContents", json!({})).map(|_| ())
    }
    /// Copies the top row of `range` down through it (Ctrl+D), or its left column right (Ctrl+R).
    pub fn fill(&mut self, range: Range, down: bool) -> Result<(), String> {
        self.run_on(range, if down { "edit.fillDown" } else { "edit.fillRight" }, json!({})).map(|_| ())
    }
    /// AutoSum: a SUM of the numbers above (or left of) the cell. Returns the formula.
    pub fn auto_sum(&mut self, row: u32, col: u32) -> Result<String, String> {
        let r = self.run_on(Range::cell(row, col), "formulas.autoSum", json!({}))?;
        Ok(r.get("formula").and_then(Json::as_str).unwrap_or_default().to_string())
    }
    /// Turns bold, italic or underline on or off over `range` (`style` is "bold", "italic"
    /// or "underline"), as the first cell decides. Returns whether it is now on.
    pub fn toggle_style(&mut self, range: Range, style: &str) -> Result<bool, String> {
        let r = self.run_on(range, &format!("home.{style}"), json!({}))?;
        Ok(r.get("on").and_then(Json::as_bool).unwrap_or(false))
    }
    /// The next cell after (`row`, `col`) whose text holds `what`, on any sheet, wrapping
    /// round: (sheet, row, col, how many cells match). The found sheet becomes the active one.
    pub fn find(&mut self, row: u32, col: u32, what: &str) -> Result<(usize, u32, u32, usize), String> {
        self.select(Range::cell(row, col))?;
        let r = self.run("edit.find", json!({ "what": what, "within": "workbook" }))?;
        let cell = r.get("cell").and_then(Json::as_str).ok_or("no cell")?;
        let at = CellRef::parse(cell).ok_or("bad cell")?;
        let n = |k: &str| r.get(k).and_then(Json::as_u64).unwrap_or(0) as usize;
        Ok((n("sheet"), at.row, at.col, n("count")))
    }
    /// Replaces every `what` with `with` in the text and formulas of every sheet (Excel's
    /// Replace All). Returns how many cells changed.
    pub fn replace_all(&mut self, what: &str, with: &str) -> Result<usize, String> {
        let r = self.run("edit.replace", json!({ "what": what, "with": with, "all": true, "within": "workbook" }))?;
        Ok(r.get("replaced").and_then(Json::as_u64).unwrap_or(0) as usize)
    }
    /// Sets the width of column `col` in pixels at 100% (0 hides it).
    pub fn set_col_width(&mut self, col: u32, width: u32) -> Result<(), String> {
        let name = column_name(col);
        self.run("home.columnWidth", json!({ "cols": format!("{name}:{name}"), "width": width.min(1800) })).map(|_| ())
    }
    /// Gives `range` a number format: "General", "Number", "Currency", "Short Date",
    /// "Time", "Percentage", "Scientific" (GridCraft's names), or a format code.
    pub fn number_format(&mut self, range: Range, format: &str) -> Result<(), String> {
        self.run_on(range, "home.numberFormat", json!({ "range": range.a1(), "format": format })).map(|_| ())
    }
    pub fn undo(&mut self) -> Result<(), String> { self.run("edit.undo", json!({})).map(|_| ()) }
    pub fn redo(&mut self) -> Result<(), String> { self.run("edit.redo", json!({})).map(|_| ()) }

    /// Column width and row height in pixels at 100%.
    pub fn col_width(&self, col: u32) -> u32 {
        self.session.doc().ok().and_then(|d| d.wb.active().map(|s| s.col_width(col))).map_or(64, |w| w.round() as u32)
    }
    pub fn row_height(&self, row: u32) -> u32 {
        self.session.doc().ok().and_then(|d| d.wb.active().map(|s| s.row_height(row))).map_or(20, |h| h.round() as u32)
    }

    /// Selects `range` (rows and columns counted from 0, inclusive), as dragging over cells
    /// does: sort, filter and charts work on it.
    pub fn select(&mut self, range: Range) -> Result<(), String> {
        self.run("selection.set", json!({ "range": range.a1() })).map(|_| ())
    }

    /// Sorts `range` (or the region around it when it is one cell) by column `col`,
    /// A to Z or Z to A; a header row stays on top.
    pub fn sort(&mut self, range: Range, col: u32, ascending: bool) -> Result<(), String> {
        self.select(range)?;
        let id = if ascending { "data.sortAscending" } else { "data.sortDescending" };
        let mut params = json!({ "column": column_name(col) });
        if !range.single() { params["range"] = json!(range.a1()); }
        self.run(id, params).map(|_| ())
    }

    /// The AutoFilter's range on the active sheet, if it has one.
    pub fn filter_range(&self) -> Option<Range> {
        let d = self.session.doc().ok()?;
        let r = d.wb.active()?.autofilter.as_ref()?.range;
        Some(Range { r0: r.start.row, c0: r.start.col, r1: r.end.row, c1: r.end.col })
    }

    /// Turns the AutoFilter on (over the region around `at`) or off.
    pub fn toggle_filter(&mut self, at: Range) -> Result<(), String> {
        self.select(at)?;
        self.run("data.filter", json!({})).map(|_| ())
    }

    /// Shows only the rows whose column `col` holds `value`, turning the AutoFilter on
    /// first if needed. `None` shows that column's rows again.
    pub fn filter_by(&mut self, at: Range, col: u32, value: Option<&str>) -> Result<(), String> {
        if self.filter_range().is_none() { self.toggle_filter(at)?; }
        let params = match value {
            Some(v) => json!({ "column": column_name(col), "values": [v] }),
            None => json!({ "column": column_name(col), "clear": true }),
        };
        self.run("data.filterBy", params).map(|_| ())
    }

    /// Rows above and columns left of the frozen corner, if panes are frozen.
    pub fn frozen(&self) -> Option<(u32, u32)> {
        self.session.doc().ok().and_then(|d| d.wb.active().and_then(|s| s.freeze))
    }

    /// Freezes the rows above and the columns left of (`row`, `col`), or unfreezes.
    pub fn freeze(&mut self, at: Option<(u32, u32)>) -> Result<(), String> {
        let params = match at {
            Some((row, col)) => json!({ "cell": CellRef::new(row, col).a1() }),
            None => json!({}),
        };
        self.run("view.freezePanes", params).map(|_| ())
    }

    /// Adds a column chart of `range` (its first row and column as labels) with its
    /// top-left corner at (`row`, `col`).
    pub fn add_chart(&mut self, range: Range, row: u32, col: u32) -> Result<(), String> {
        self.select(range)?;
        let at = CellRef::new(row, col).a1();
        self.run("insert.chart", json!({ "range": range.a1(), "type": "column", "at": at })).map(|_| ())
    }

    /// The active sheet's charts: where each sits and how big it is.
    pub fn charts(&self) -> Vec<Placed> {
        let Ok(d) = self.session.doc() else { return Vec::new() };
        let Some(sheet) = d.wb.active() else { return Vec::new() };
        sheet.charts.iter().map(|c| {
            let a = c.anchor;
            // Anchors are in points; the grid is in pixels at 96 dpi.
            let px = |pt: f32| (pt * 4.0 / 3.0).round() as i32;
            Placed { row: a.cell.row, col: a.cell.col, dx: px(a.dx), dy: px(a.dy), width: px(a.width).max(1), height: px(a.height).max(1) }
        }).collect()
    }

    /// Chart `index` of the active sheet laid out at `width` x `height` pixels: shapes
    /// drawn as 0x00RRGGBB pixels on white, and its text left for the caller's font.
    pub fn chart_picture(&self, index: usize, width: u32, height: u32, measure: &dyn gridcraft_chart::Measure) -> Option<ChartPicture> {
        let d = self.session.doc().ok()?;
        let si = d.wb.active_sheet();
        let chart = d.wb.sheets.get(si)?.charts.get(index)?;
        let data = gridcraft_chart::resolve(&d.wb, si, chart);
        let prims = gridcraft_chart::render(chart, &data, width as f32, height as f32, measure);
        let rgba = gridcraft_chart::rasterize(&prims, width, height, [255, 255, 255, 255]);
        if rgba.len() != (width * height * 4) as usize { return None; }
        let pixels = rgba.chunks_exact(4).map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect();
        let text = prims.into_iter().filter_map(|p| match p {
            gridcraft_chart::Prim::Text { x, y, text, size, color, align, valign, .. } => Some(Label {
                x, y, text, size,
                color: (color[0] as u32) << 16 | (color[1] as u32) << 8 | color[2] as u32,
                align: match align { gridcraft_chart::HAlign::Left => -1, gridcraft_chart::HAlign::Center => 0, gridcraft_chart::HAlign::Right => 1 },
                valign: match valign { gridcraft_chart::VAlign::Top => -1, gridcraft_chart::VAlign::Middle => 0, _ => 1 },
            }),
            _ => None,
        }).collect();
        Some(ChartPicture { pixels, text })
    }

    /// Where Ctrl+S writes: the file itself, or beside it as .xlsx when the engine only
    /// reads its format (ods, xls, xlsb).
    pub fn save_path(&self) -> String {
        let lower = self.path.to_ascii_lowercase();
        match [".ods", ".xls", ".xlsb"].iter().find(|e| lower.ends_with(*e)) {
            Some(e) => format!("{}.xlsx", &self.path[..self.path.len() - e.len()]),
            None => self.path.clone(),
        }
    }

    /// The workbook as the bytes of its file, in the format `path`'s extension names
    /// (xlsx when it names none the engine writes).
    pub fn file_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        let d = self.session.doc().map_err(|e| e.to_string())?;
        gridcraft_engine::io::save_file_bytes(&d.wb, path).map_err(|e| e.to_string())
    }

    /// Writes the workbook to its file (see [`Book::save_path`]). std::fs::write, not the
    /// engine's write-then-rename, so a save works on every SexOS volume. Returns the size.
    pub fn save(&mut self) -> Result<usize, String> {
        let path = self.save_path();
        self.path = path.clone();
        let bytes = self.file_bytes(&path)?;
        std::fs::write(&path, &bytes).map_err(|e| format!("Could not save {path}: {e}"))?;
        if let Ok(d) = self.session.doc_mut() {
            d.saved = d.wb.clone();
            d.path = Some(path);
        }
        Ok(bytes.len())
    }
}

/// `A`, `B`, … `Z`, `AA`, … for column `col` counted from 0.
pub fn column_name(col: u32) -> String {
    let mut n = col + 1;
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WELCOME: &[u8] = include_bytes!("../assets/welcome.xlsx");

    fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("spreadeagle-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn column_names() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
    }

    #[test]
    fn opens_the_hand_written_xlsx() {
        let path = temp("welcome.xlsx");
        std::fs::write(&path, WELCOME).unwrap();
        let (book, _) = Book::open(&path).unwrap();
        assert_eq!(book.sheets(), ["Budget", "Notes"]);
        assert_eq!(book.shown(0, 0).text, "Item");
        assert_eq!(book.shown(1, 1), Shown { text: "1200".into(), right: true, error: false });
        assert_eq!(book.input(4, 1), "=SUM(B2:B4)");
        assert_eq!(book.shown(4, 1).text, "1650");
        assert_eq!(book.shown(5, 1).text, "19800");
    }

    #[test]
    fn edits_recalculate_and_undo() {
        let path = temp("edit.xlsx");
        std::fs::write(&path, WELCOME).unwrap();
        let (mut book, _) = Book::open(&path).unwrap();
        assert!(!book.dirty());
        book.set(1, 1, "1500").unwrap();
        assert!(book.dirty());
        assert_eq!(book.shown(4, 1).text, "1950");
        assert_eq!(book.shown(5, 1).text, "23400");
        book.undo().unwrap();
        assert_eq!(book.shown(4, 1).text, "1650");
        book.redo().unwrap();
        assert_eq!(book.shown(4, 1).text, "1950");
        book.clear(3, 1).unwrap();
        assert_eq!(book.shown(4, 1).text, "1800");
        book.set(6, 1, "=AVERAGE(B2:B3)").unwrap();
        assert_eq!(book.shown(6, 1).text, "900");
        book.set(7, 1, "=1/0").unwrap();
        assert!(book.shown(7, 1).error);
    }

    #[test]
    fn save_and_reopen_keeps_every_cell() {
        let path = temp("round.xlsx");
        std::fs::write(&path, WELCOME).unwrap();
        let (mut book, _) = Book::open(&path).unwrap();
        book.set(1, 1, "40").unwrap();
        book.set(0, 2, "Note").unwrap();
        book.activate(1).unwrap();
        book.set(2, 0, "third line").unwrap();
        book.activate(0).unwrap();
        let bytes = book.save().unwrap();
        assert!(bytes > 1000);
        assert!(!book.dirty());
        let (mut again, _) = Book::open(&path).unwrap();
        assert_eq!(again.sheets(), ["Budget", "Notes"]);
        for row in 0..8 {
            for col in 0..4 {
                assert_eq!(again.input(row, col), book.input(row, col), "input at {row},{col}");
                assert_eq!(again.shown(row, col), book.shown(row, col), "shown at {row},{col}");
            }
        }
        assert_eq!(again.shown(4, 1).text, "490");
        again.activate(1).unwrap();
        assert_eq!(again.shown(2, 0).text, "third line");
    }

    #[test]
    fn csv_opens_and_saves_as_xlsx() {
        let csv = temp("plain.csv");
        std::fs::write(&csv, "a,b\n1,2\n3,4\n").unwrap();
        let (mut book, _) = Book::open(&csv).unwrap();
        book.set(3, 1, "=SUM(B2:B3)").unwrap();
        assert_eq!(book.shown(3, 1).text, "6");
        let out = temp("plain.xlsx");
        book.path = out.clone();
        book.save().unwrap();
        let (again, _) = Book::open(&out).unwrap();
        assert_eq!(again.shown(3, 1).text, "6");
        assert_eq!(again.input(3, 1), "=SUM(B2:B3)");
    }

    #[test]
    fn blank_book_saves() {
        let path = temp("blank.xlsx");
        let mut book = Book::blank(&path);
        book.set(0, 0, "=2^10").unwrap();
        book.save().unwrap();
        assert_eq!(Book::open(&path).unwrap().0.shown(0, 0).text, "1024");
    }
    /// Item/Cost rows of the Budget sheet, without its total rows.
    fn budget(name: &str) -> Book {
        let path = temp(name);
        std::fs::write(&path, WELCOME).unwrap();
        let (mut book, _) = Book::open(&path).unwrap();
        book.clear(4, 0).unwrap(); book.clear(4, 1).unwrap();
        book.clear(5, 0).unwrap(); book.clear(5, 1).unwrap();
        book
    }

    fn column(book: &Book, col: u32, rows: u32) -> Vec<String> { (0..rows).map(|r| book.shown(r, col).text).collect() }

    #[test]
    fn sorts_by_a_column_keeping_the_header() {
        let mut book = budget("sort.xlsx");
        book.sort(Range::cell(1, 1), 1, true).unwrap();
        assert_eq!(column(&book, 0, 4), ["Item", "Fun", "Food", "Rent"]);
        assert_eq!(column(&book, 1, 4), ["Cost", "150", "300", "1200"]);
        book.sort(Range { r0: 0, c0: 0, r1: 3, c1: 1 }, 0, false).unwrap();
        assert_eq!(column(&book, 0, 4), ["Item", "Rent", "Fun", "Food"]);
        book.undo().unwrap();
        assert_eq!(column(&book, 0, 4), ["Item", "Fun", "Food", "Rent"]);
    }

    #[test]
    fn filter_hides_other_rows() {
        let mut book = budget("filter.xlsx");
        assert!(book.filter_range().is_none());
        book.filter_by(Range::cell(2, 0), 0, Some("Food")).unwrap();
        assert_eq!(book.filter_range(), Some(Range { r0: 0, c0: 0, r1: 3, c1: 1 }));
        let heights: Vec<u32> = (0..4).map(|r| book.row_height(r)).collect();
        assert!(heights[0] > 0 && heights[2] > 0, "{heights:?}");
        assert_eq!((heights[1], heights[3]), (0, 0), "{heights:?}");
        book.filter_by(Range::cell(2, 0), 0, None).unwrap();
        assert!((0..4).all(|r| book.row_height(r) > 0));
        book.toggle_filter(Range::cell(2, 0)).unwrap();
        assert!(book.filter_range().is_none());
        // Turning the filter off while it hides rows shows them all again.
        book.filter_by(Range::cell(2, 0), 0, Some("Rent")).unwrap();
        assert_eq!(book.row_height(2), 0);
        book.toggle_filter(Range::cell(2, 0)).unwrap();
        assert!(book.filter_range().is_none());
        assert!((0..4).all(|r| book.row_height(r) > 0));
    }

    #[test]
    fn freezes_and_unfreezes_panes() {
        let mut book = budget("freeze.xlsx");
        assert_eq!(book.frozen(), None);
        book.freeze(Some((1, 1))).unwrap();
        assert_eq!(book.frozen(), Some((1, 1)));
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(Book::open(&path).unwrap().0.frozen(), Some((1, 1)));
        book.freeze(None).unwrap();
        assert_eq!(book.frozen(), None);
    }

    #[test]
    fn chart_of_a_range_draws_and_saves() {
        let mut book = budget("chart.xlsx");
        book.add_chart(Range { r0: 0, c0: 0, r1: 3, c1: 1 }, 1, 3).unwrap();
        let charts = book.charts();
        assert_eq!(charts.len(), 1);
        let c = charts[0];
        assert_eq!((c.row, c.col), (1, 3));
        assert!(c.width > 100 && c.height > 80, "{c:?}");
        let picture = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap();
        let ink = picture.pixels.iter().filter(|p| **p != 0x00ffffff).count();
        assert!(ink > 2000, "ink {ink}");
        assert!(picture.text.iter().any(|l| l.text == "Rent"), "labels: {:?}", picture.text.iter().map(|l| &l.text).collect::<Vec<_>>());
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(Book::open(&path).unwrap().0.charts().len(), 1);
    }

    #[test]
    fn ods_opens_and_saves_beside_it_as_xlsx() {
        let path = temp("sample.ods");
        std::fs::write(&path, include_bytes!("../assets/sample.ods")).unwrap();
        let (mut book, _) = Book::open(&path).unwrap();
        assert_eq!(book.sheets(), ["Harvest"]);
        assert_eq!(column(&book, 0, 5), ["Fruit", "Pears", "Apples", "Plums", "Total"]);
        assert_eq!(book.shown(4, 1).text, "49");
        assert_eq!(book.input(4, 1), "=SUM(B2:B4)");
        assert!(!book.dirty());
        assert!(book.undo().is_err(), "the formulas are part of the opened file, not an edit");
        book.set(1, 1, "13").unwrap();
        assert_eq!(book.shown(4, 1).text, "50");
        assert!(book.save_path().ends_with("sample.xlsx"));
        book.save().unwrap();
        assert!(book.path.ends_with("sample.xlsx"));
        assert_eq!(Book::open(&book.path).unwrap().0.shown(4, 1).text, "50");
    }

    #[test]
    fn ranges_name_their_cells() {
        assert_eq!(Range::cell(0, 0).a1(), "A1");
        assert_eq!(Range { r0: 3, c0: 1, r1: 0, c1: 0 }.a1(), "A1:B4");
        assert!(Range { r0: 3, c0: 1, r1: 0, c1: 0 }.contains(2, 1));
    }

    #[test]
    fn copy_paste_fill_sum_style_and_find() {
        let path = temp("clip.xlsx");
        std::fs::write(&path, WELCOME).unwrap();
        let (mut book, _) = Book::open(&path).unwrap();
        // Copy A1:B2 to D1: both rows arrive.
        assert_eq!(book.copy(Range { r0: 0, c0: 0, r1: 1, c1: 1 }, false).unwrap(), "Item\tCost\nRent\t1200\n");
        assert_eq!(book.paste(0, 3).unwrap(), "D1:E2");
        assert_eq!(book.shown(1, 4).text, "1200");
        // Cut moves: the source empties.
        book.copy(Range::cell(1, 4), true).unwrap();
        book.paste(9, 4).unwrap();
        assert_eq!(book.shown(1, 4).text, "");
        assert_eq!(book.shown(9, 4).text, "1200");
        // Clear a whole range.
        book.clear_range(Range { r0: 0, c0: 3, r1: 9, c1: 4 }).unwrap();
        assert!((0..10).all(|r| book.shown(r, 3).text.is_empty() && book.shown(r, 4).text.is_empty()));
        // Fill down copies the top cell.
        book.set(0, 6, "7").unwrap();
        book.fill(Range { r0: 0, c0: 6, r1: 2, c1: 6 }, true).unwrap();
        assert_eq!(book.shown(2, 6).text, "7");
        // AutoSum under a column of numbers.
        book.set(1, 7, "2").unwrap();
        book.set(2, 7, "3").unwrap();
        assert_eq!(book.auto_sum(3, 7).unwrap(), "=SUM(H2:H3)");
        assert_eq!(book.shown(3, 7).text, "5");
        // Bold on, then off again.
        assert!(book.toggle_style(Range { r0: 0, c0: 0, r1: 0, c1: 1 }, "bold").unwrap());
        assert_eq!(book.font(0, 1), (true, false, false));
        assert!(!book.toggle_style(Range::cell(0, 0), "bold").unwrap());
        assert!(book.toggle_style(Range::cell(0, 0), "underline").unwrap());
        assert_eq!(book.font(0, 0), (false, false, true));
        // Find moves on from the cursor and across sheets.
        let (sheet, row, col, n) = book.find(0, 0, "Fun").unwrap();
        assert_eq!((sheet, row, col), (0, 3, 0));
        assert!(n >= 1);
        assert!(book.find(0, 0, "no such text anywhere").is_err());
    }


    #[test]
    fn replace_widths_and_number_formats() {
        let path = temp("replace.xlsx");
        std::fs::write(&path, WELCOME).unwrap();
        let (mut book, _) = Book::open(&path).unwrap();
        assert_eq!(book.replace_all("Rent", "Lodging").unwrap(), 1);
        assert_eq!(book.shown(1, 0).text, "Lodging");
        assert_eq!(book.replace_all("nothing like this", "x").unwrap_or(0), 0);
        book.set_col_width(0, 150).unwrap();
        assert_eq!(book.col_width(0), 150);
        book.number_format(Range { r0: 1, c0: 1, r1: 3, c1: 1 }, "Currency").unwrap();
        assert!(book.shown(1, 1).text.contains("1,200.00"), "{}", book.shown(1, 1).text);
        book.number_format(Range::cell(1, 1), "Percentage").unwrap();
        assert!(book.shown(1, 1).text.ends_with('%'), "{}", book.shown(1, 1).text);
        book.undo().unwrap();
        assert!(book.shown(1, 1).text.contains("1,200.00"));
    }

}
