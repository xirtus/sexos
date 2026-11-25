// SPDX-License-Identifier: GPL-3.0-only
//! The workbook behind the window: GridCraft's engine (storytold/gridcraft, MIT OR
//! Apache-2.0) holds the cells, recalculates, keeps undo, and reads and writes the
//! files. Nothing here draws or touches SexOS, so the host tests drive it directly.
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::FilterCriterion;
use serde_json::{Value as Json, json};
use std::io::Write;

/// How a cell reads in the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub text: String,
    /// Numbers sit at the right of their cell, as in every spreadsheet.
    pub right: bool,
    pub error: bool,
}

/// The active AutoFilter criterion for one column, for the native value menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterState { None, Values { values: Vec<String>, blanks: bool }, Other }

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

/// Lines that fit a cell's text width, capped by the visible row height. Explicit
/// line breaks stay separate; a long word breaks at a character boundary.
pub fn wrap_lines(text: &str, width: i32, max_lines: usize, measure: impl Fn(&str) -> i32) -> Vec<String> {
    let mut lines = Vec::new();
    let max_lines = max_lines.max(1);
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let joined = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if measure(&joined) <= width { line = joined; continue; }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                if lines.len() == max_lines { return lines; }
            }
            for ch in word.chars() {
                let mut next = line.clone(); next.push(ch);
                if !line.is_empty() && measure(&next) > width {
                    lines.push(std::mem::take(&mut line));
                    if lines.len() == max_lines { return lines; }
                }
                line.push(ch);
            }
        }
        if !line.is_empty() || paragraph.is_empty() {
            lines.push(line);
            if lines.len() == max_lines { return lines; }
        }
    }
    lines
}

/// A chart's place on the grid: its anchor cell, the offset into it and its size, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed { pub id: u32, pub row: u32, pub col: u32, pub dx: i32, pub dy: i32, pub width: i32, pub height: i32 }

/// One piece of a chart's text, in the chart's pixels; `align` and `valign` are -1, 0 or 1
/// for left/top, centre and right/bottom of the anchor point.
pub struct Label { pub x: f32, pub y: f32, pub text: String, pub size: f32, pub color: u32, pub align: i8, pub valign: i8 }

/// A drawn chart: 0x00RRGGBB pixels at the asked size, with the text still to draw over them.
pub struct ChartPicture { pub pixels: Vec<u32>, pub text: Vec<Label> }

/// One entry from GridCraft's built-in worksheet function catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function { pub name: String, pub category: String, pub signature: String, pub description: String }

impl Function {
    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_ascii_lowercase();
        self.name.to_ascii_lowercase().contains(&q)
            || self.category.to_ascii_lowercase().contains(&q)
            || self.description.to_ascii_lowercase().contains(&q)
    }
}

pub struct Book {
    session: Session,
    /// The file the workbook is saved to.
    pub path: String,
}

impl Book {
    /// The current engine's functions, not a hand-maintained shortcut list.
    pub fn functions(&mut self) -> Result<Vec<Function>, String> {
        let catalog = self.run("formulas.functions", json!({}))?;
        let entries = catalog.as_array().ok_or("Invalid function catalog")?;
        Ok(entries.iter().filter_map(|entry| Some(Function {
            name: entry["name"].as_str()?.to_string(),
            category: entry["category"].as_str().unwrap_or("").to_string(),
            signature: entry["signature"].as_str().unwrap_or("").to_string(),
            description: entry["description"].as_str().unwrap_or("").to_string(),
        })).collect())
    }
    /// An empty workbook that Ctrl+S saves to `path`.
    pub fn blank(path: &str) -> Book {
        let mut session = Session::new();
        session.new_workbook();
        Book { session, path: path.to_string() }
    }

    /// Opens an xlsx, xls, ods, csv, tsv or GridCraft json file.
    pub fn open(path: &str) -> Result<(Book, Vec<String>), String> {
        let mut session = Session::new();
        let r = session.execute_headless("file.open", json!({ "path": path })).map_err(|e| e.to_string())?;
        let mut warnings: Vec<String> = r["warnings"].as_array().into_iter().flatten().filter_map(Json::as_str).map(str::to_string).collect();
        let mut book = Book { session, path: path.to_string() };
        if path.to_ascii_lowercase().ends_with(".ods") {
            match book.restore_ods_extras(path) {
                Ok(n) => if n > 0 { warnings.retain(|w| !w.contains("formulas become their saved results")); },
                Err(e) => warnings.push(format!("ODS formulas were kept as values: {e}")),
            }
        }
        Ok((book, warnings))
    }

    /// Puts the file's formulas back over the values GridCraft imported, and its fonts and
    /// column widths, as part of the opened file: no undo step, not a change. Returns how
    /// many formulas.
    fn restore_ods_extras(&mut self, path: &str) -> Result<usize, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let extras = crate::ods::extras(&bytes)?;
        if extras == crate::ods::Extras::default() { return Ok(0); }
        let imported_names = self.sheets();
        let active = self.active_sheet();
        let mut sheet = usize::MAX;
        for f in &extras.formulas {
            if f.sheet != sheet { self.activate(f.sheet)?; sheet = f.sheet; }
            self.set(f.row, f.col, &crate::ods::remap_sheet_names(&f.formula, &extras.sheet_names, &imported_names))?;
        }
        // Fonts by runs of a row, one command per run.
        let mut fonts = extras.fonts.clone();
        fonts.sort_by_key(|f| (f.sheet, f.row, f.col));
        let mut i = 0;
        while i < fonts.len() {
            let f = &fonts[i];
            let mut end = i + 1;
            while end < fonts.len() && fonts[end].sheet == f.sheet && fonts[end].row == f.row && fonts[end].col == fonts[end - 1].col + 1 && fonts[end].font == f.font { end += 1; }
            if f.sheet != sheet { self.activate(f.sheet)?; sheet = f.sheet; }
            let range = Range { r0: f.row, c0: f.col, r1: f.row, c1: fonts[end - 1].col };
            for (on, style) in [(f.font.0, "bold"), (f.font.1, "italic"), (f.font.2, "underline")] {
                if on { self.run(&format!("home.{style}"), json!({ "range": range.a1(), "on": true }))?; }
            }
            i = end;
        }
        for &(s, row, col) in &extras.wraps {
            if s != sheet { self.activate(s)?; sheet = s; }
            self.set_wrap(Range::cell(row, col), true)?;
        }
        for (s, row, col, code) in &extras.percents {
            if *s != sheet { self.activate(*s)?; sheet = *s; }
            self.run("home.numberFormat", json!({ "range": Range::cell(*row, *col).a1(), "format": code }))?;
        }
        for &(s, col, width) in &extras.widths {
            if s != sheet { self.activate(s)?; sheet = s; }
            if (width.round() as u32) != self.col_width(col) { self.set_col_width(col, width.round() as u32)?; }
        }
        self.activate(active)?;
        if let Ok(d) = self.session.doc_mut() {
            d.undo.clear();
            d.redo.clear();
            d.saved = d.wb.clone();
        }
        Ok(extras.formulas.len())
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
    pub fn wrapped(&self, row: u32, col: u32) -> bool {
        let Ok(d) = self.session.doc() else { return false };
        let Some(sheet) = d.wb.active() else { return false };
        d.wb.styles.get(sheet.style_id(CellRef::new(row, col))).align.wrap
    }
    /// Wrap every cell in `range`; GridCraft also grows automatic row heights.
    pub fn set_wrap(&mut self, range: Range, on: bool) -> Result<(), String> {
        self.run_on(range, "home.wrapText", json!({ "range": range.a1(), "on": on })).map(|_| ())
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
    /// Sets one row's height in pixels at 100% (0 hides it).
    pub fn set_row_height(&mut self, row: u32, height: u32) -> Result<(), String> {
        let number = row.saturating_add(1);
        self.run("home.rowHeight", json!({ "rows": format!("{number}:{number}"), "height": height.min(546) })).map(|_| ())
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

    /// Sort an explicitly selected block by two or more columns, in priority order.
    /// A header row is detected by GridCraft and kept at the top.
    pub fn sort_multi(&mut self, range: Range, input: &str) -> Result<(), String> {
        if range.single() { return Err("Select the whole table before sorting by several columns".into()); }
        let (_, left, _, right) = range.bounds();
        let mut keys = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for part in input.split(',') {
            let words: Vec<_> = part.split_whitespace().collect();
            if words.len() != 2 { return Err("Enter column and order, for example A asc, C desc".into()); }
            let name = words[0].to_ascii_uppercase();
            if !name.bytes().all(|c| c.is_ascii_uppercase()) { return Err("Enter a column letter such as A or BC".into()); }
            let col = CellRef::parse(&format!("{name}1")).ok_or("Enter a valid column letter")?.col;
            if !(left..=right).contains(&col) { return Err(format!("Column {name} is outside {}", range.a1())); }
            if !seen.insert(col) { return Err(format!("Column {name} appears more than once")); }
            let order = match words[1].to_ascii_lowercase().as_str() {
                "asc" => "asc", "desc" => "desc",
                _ => return Err("Choose asc or desc for each column".into()),
            };
            keys.push(json!({ "column": name, "order": order }));
        }
        if !(2..=8).contains(&keys.len()) { return Err("Choose 2–8 different sort columns".into()); }
        self.select(range)?;
        self.run("data.sort", json!({ "range": range.a1(), "keys": keys })).map(|_| ())
    }

    /// The AutoFilter's range on the active sheet, if it has one.
    pub fn filter_range(&self) -> Option<Range> {
        let d = self.session.doc().ok()?;
        let r = d.wb.active()?.autofilter.as_ref()?.range;
        Some(Range { r0: r.start.row, c0: r.start.col, r1: r.end.row, c1: r.end.col })
    }

    pub fn filter_state(&self, col: u32) -> FilterState {
        let Ok(d) = self.session.doc() else { return FilterState::None };
        let Some(af) = d.wb.active().and_then(|s| s.autofilter.as_ref()) else { return FilterState::None };
        if !(af.range.start.col..=af.range.end.col).contains(&col) { return FilterState::None; }
        let Some((_, criterion)) = af.criteria.iter().find(|(off, _)| *off == col - af.range.start.col) else { return FilterState::None };
        match criterion {
            FilterCriterion::Values { values, blanks } => FilterState::Values { values: values.clone(), blanks: *blanks },
            _ => FilterState::Other,
        }
    }

    /// Distinct displayed values under an AutoFilter header, including rows currently hidden
    /// by that filter. The cap keeps opening a menu bounded on very large sheets.
    pub fn filter_choices(&self, col: u32) -> Vec<String> {
        let Some(range) = self.filter_range() else { return Vec::new() };
        let (top, left, bottom, right) = range.bounds();
        if !(left..=right).contains(&col) { return Vec::new() }
        let mut values = std::collections::BTreeSet::new();
        for row in top.saturating_add(1)..=bottom.min(top.saturating_add(10_000)) {
            values.insert(self.shown(row, col).text);
            if values.len() >= 1_000 { break; }
        }
        values.into_iter().collect()
    }

    /// Resolve an exact displayed value across the full filter range. The menu preview is
    /// deliberately bounded, but a value beyond that preview must still be reachable.
    pub fn filter_exact_choice(&self, col: u32, query: &str) -> Option<String> {
        let range = self.filter_range()?;
        let (top, left, bottom, right) = range.bounds();
        if query.is_empty() || !(left..=right).contains(&col) { return None; }
        let needle = query.to_lowercase();
        let d = self.session.doc().ok()?;
        let sheet = d.wb.active()?;
        for row in top.saturating_add(1)..=bottom {
            let value = gridcraft_engine::display::cell_text(&d.wb, sheet, CellRef::new(row, col));
            if value.to_lowercase() == needle { return Some(value); }
        }
        None
    }

    /// Turns the AutoFilter on (over the region around `at`) or off.
    pub fn toggle_filter(&mut self, at: Range) -> Result<(), String> {
        self.select(at)?;
        self.run("data.filter", json!({})).map(|_| ())
    }

    /// Shows only the rows whose column `col` holds `value`, turning the AutoFilter on
    /// first if needed. `None` shows that column's rows again.
    pub fn filter_by(&mut self, at: Range, col: u32, value: Option<&str>) -> Result<(), String> {
        let values: Vec<String> = value.into_iter().map(str::to_string).collect();
        self.filter_by_values(at, col, &values)
    }

    /// Shows all selected values in one column. An empty choice clears that
    /// column's filter while keeping AutoFilter available on the range.
    pub fn filter_by_values(&mut self, at: Range, col: u32, values: &[String]) -> Result<(), String> {
        if self.filter_range().is_none() { self.toggle_filter(at)?; }
        let params = if values.is_empty() {
            json!({ "column": column_name(col), "clear": true })
        } else {
            json!({ "column": column_name(col), "values": values })
        };
        self.run("data.filterBy", params).map(|_| ())
    }

    /// Apply one or two AutoFilter comparisons to a column. GridCraft evaluates
    /// the criteria against cells, including numeric values and formula results.
    pub fn filter_by_condition(&mut self, at: Range, col: u32, input: &str) -> Result<(), String> {
        const HELP: &str = "Enter =, <>, contains/starts/ends/notcontains text or >, >=, <, <= number; join two with and/or";
        fn criterion(input: &str) -> Result<(&'static str, &str), String> {
            let (name, value) = input.trim().split_once(' ').ok_or(HELP)?;
            let value = value.trim();
            if value.is_empty() || value.chars().count() > 128 { return Err(HELP.into()); }
            let op = match name.to_ascii_lowercase().as_str() {
                "contains" => "contains", "notcontains" => "notContains",
                "starts" => "beginsWith", "ends" => "endsWith",
                "=" => "=", "<>" | "!=" => "<>",
                ">" => ">", ">=" => ">=", "<" => "<", "<=" => "<=",
                _ => return Err(HELP.into()),
            };
            if matches!(op, ">" | ">=" | "<" | "<=")
                && !value.parse::<f64>().is_ok_and(f64::is_finite) { return Err(HELP.into()); }
            Ok((op, value))
        }
        let input = input.trim();
        // Ordinary text may contain "and" or "or". A join exists only when
        // its right side starts a second comparison. Reject three comparisons.
        let mut joins = Vec::new();
        for (delimiter, and) in [(" and ", true), (" or ", false)] {
            for (at, _) in input.match_indices(delimiter) {
                let right = &input[at + delimiter.len()..];
                if right.split_once(' ').is_some_and(|(op, _)| matches!(op.to_ascii_lowercase().as_str(),
                    "contains" | "notcontains" | "starts" | "ends" | "=" | "<>" | "!=" | ">" | ">=" | "<" | "<=")) {
                    joins.push((at, delimiter.len(), and));
                }
            }
        }
        if joins.len() > 1 { return Err(HELP.into()); }
        let (first, second) = match joins.pop() {
            Some((at, len, and)) => (&input[..at], Some((&input[at + len..], and))),
            None => (input, None),
        };
        let (op, value) = criterion(first)?;
        let second = second.map(|(input, and)| criterion(input).map(|(op, value)| (op, value, and))).transpose()?;
        if self.filter_range().is_none() { self.toggle_filter(at)?; }
        let custom = if let Some((op2, value2, and)) = second {
            json!({ "op": op, "value": value, "op2": op2, "value2": value2, "and": and })
        } else { json!({ "op": op, "value": value }) };
        self.run("data.filterBy", json!({ "column": column_name(col), "custom": custom })).map(|_| ())
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
        self.add_chart_kind(range, row, col, "column")
    }

    /// Insert one of the chart types offered by the native grid.
    pub fn add_chart_kind(&mut self, range: Range, row: u32, col: u32, kind: &str) -> Result<(), String> {
        if !matches!(kind, "column" | "line" | "bar" | "pie" | "doughnut") {
            return Err("Choose column, line, bar, pie or doughnut".into());
        }
        self.select(range)?;
        let at = CellRef::new(row, col).a1();
        self.run("insert.chart", json!({ "range": range.a1(), "type": kind, "at": at })).map(|_| ())
    }

    /// The active sheet's charts: where each sits and how big it is.
    pub fn charts(&self) -> Vec<Placed> {
        let Ok(d) = self.session.doc() else { return Vec::new() };
        let Some(sheet) = d.wb.active() else { return Vec::new() };
        sheet.charts.iter().map(|c| {
            let a = c.anchor;
            // Anchors are in points; the grid is in pixels at 96 dpi.
            let px = |pt: f32| (pt * 4.0 / 3.0).round() as i32;
            Placed { id: c.id, row: a.cell.row, col: a.cell.col, dx: px(a.dx), dy: px(a.dy), width: px(a.width).max(1), height: px(a.height).max(1) }
        }).collect()
    }

    /// The topmost chart anchored at the current cell.
    pub fn chart_at_anchor(&self, row: u32, col: u32) -> Option<Placed> {
        self.charts().into_iter().rev().find(|c| c.row == row && c.col == col)
    }

    /// Change an existing chart through GridCraft's undoable command.
    pub fn edit_chart(&mut self, id: u32, input: &str) -> Result<String, String> {
        let input = input.trim();
        let chart = self.session.doc().ok().and_then(|d| d.wb.active()
            .and_then(|sheet| sheet.charts.iter().find(|chart| chart.id == id).map(|chart| (chart.kind, chart.by_rows, chart.series.len()))));
        let Some((kind, by_rows, series_count)) = chart else { return Err("Choose a chart on this sheet first".into()); };
        if let Some(number) = input.strip_prefix("remove ") {
            let number = number.parse::<usize>().map_err(|_| "Enter a series number from the legend")?;
            if series_count < 2 || number == 0 || number > series_count {
                return Err(format!("Choose series 1–{series_count}; keep at least one"));
            }
            self.run("chart.selectData", json!({ "chart": id, "removeSeries": [number - 1] }))?;
            let remaining = self.session.doc().ok().and_then(|d| d.wb.active()
                .and_then(|sheet| sheet.charts.iter().find(|chart| chart.id == id).map(|chart| chart.series.len()))).unwrap_or(0);
            if remaining != series_count - 1 { return Err("Could not remove that series".into()); }
            return Ok(format!("Removed chart series {number}"));
        }
        if input == "switch" {
            self.run("chart.switchRowColumn", json!({ "chart": id }))?;
            let switched = self.session.doc().ok().and_then(|d| d.wb.active()
                .and_then(|sheet| sheet.charts.iter().find(|chart| chart.id == id).map(|chart| chart.by_rows != by_rows))).unwrap_or(false);
            if !switched { return Err("This chart has no selectable source range".into()); }
            return Ok("Chart series switched between rows and columns".into());
        }
        if let Some(range) = input.strip_prefix("data ") {
            let range = range.trim();
            if range.is_empty() || range.chars().count() > 120 {
                return Err("Enter a chart data range such as A1:B4".into());
            }
            self.run("chart.selectData", json!({ "chart": id, "range": range }))?;
            return Ok(format!("Chart data: {range}"));
        }
        if let Some(title) = input.strip_prefix("title ") {
            if title.is_empty() || title.chars().count() > 120 { return Err("Enter title followed by 1–120 characters".into()); }
            self.run("chart.set", json!({ "chart": id, "title": title }))?;
            return Ok(format!("Chart title: {title}"));
        }
        if input == "clear title" {
            self.run("chart.set", json!({ "chart": id, "title": null }))?;
            return Ok("Chart title cleared".into());
        }
        if let Some(kind) = input.strip_prefix("type ") {
            if !matches!(kind, "column" | "line" | "bar" | "pie" | "doughnut") {
                return Err("Choose type column, line, bar, pie or doughnut".into());
            }
            self.run("chart.set", json!({ "chart": id, "type": kind }))?;
            return Ok(format!("Chart type: {kind}"));
        }
        if let Some(position) = input.strip_prefix("legend ") {
            if !matches!(position, "none" | "top" | "bottom" | "left" | "right") {
                return Err("Choose legend none, top, bottom, left or right".into());
            }
            self.run("chart.set", json!({ "chart": id, "legend": position }))?;
            return Ok(format!("Chart legend: {position}"));
        }
        if let Some(choice) = input.strip_prefix("table ") {
            let on = match choice { "on" => true, "off" => false, _ => return Err("Choose table on or off".into()) };
            self.run("chart.set", json!({ "chart": id, "dataTable": on }))?;
            return Ok(format!("Chart data table: {}", if on { "on" } else { "off" }));
        }
        if let Some(value) = input.strip_prefix("hole ") {
            if kind != gridcraft_engine::model::ChartKind::Doughnut { return Err("Hole size needs a doughnut chart".into()); }
            let hole = value.parse::<u8>().map_err(|_| "Choose hole size 10–90")?;
            if !(10..=90).contains(&hole) { return Err("Choose hole size 10–90".into()); }
            self.run("chart.set", json!({ "chart": id, "holeSize": hole }))?;
            return Ok(format!("Doughnut hole: {hole}%"));
        }
        if let Some(value) = input.strip_prefix("angle ") {
            if !matches!(kind, gridcraft_engine::model::ChartKind::Pie | gridcraft_engine::model::ChartKind::Doughnut) {
                return Err("Slice angle needs a pie or doughnut chart".into());
            }
            let angle = value.parse::<u16>().map_err(|_| "Choose slice angle 0–359")?;
            if angle > 359 { return Err("Choose slice angle 0–359".into()); }
            self.run("chart.set", json!({ "chart": id, "firstSliceAngle": angle }))?;
            return Ok(format!("First slice angle: {angle}°"));
        }
        for (prefix, field, label) in [("x title ", "xTitle", "horizontal axis"), ("y title ", "yTitle", "vertical axis")] {
            if let Some(title) = input.strip_prefix(prefix) {
                if title.is_empty() || title.chars().count() > 120 {
                    return Err("Enter an axis title of 1–120 characters".into());
                }
                self.run("chart.set", json!({ "chart": id, (field): title }))?;
                return Ok(format!("Chart {label}: {title}"));
            }
        }
        for (command, field, label) in [("clear x title", "xTitle", "horizontal axis"), ("clear y title", "yTitle", "vertical axis")] {
            if input == command {
                self.run("chart.set", json!({ "chart": id, (field): null }))?;
                return Ok(format!("Chart {label} title cleared"));
            }
        }
        for (prefix, field, label) in [("labels ", "dataLabels", "data labels"), ("gridlines ", "gridlines", "gridlines")] {
            if let Some(choice) = input.strip_prefix(prefix) {
                let on = match choice { "on" => true, "off" => false, _ => return Err(format!("Choose {prefix}on or off")) };
                self.run("chart.set", json!({ "chart": id, (field): on }))?;
                return Ok(format!("Chart {label}: {}", if on { "on" } else { "off" }));
            }
        }
        Err("Enter remove 2, data A1:B4, switch, type, title, legend, table, hole or angle".into())
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

    /// Where Ctrl+S writes: the file itself, or beside it as .xlsx when nothing here writes
    /// its format (xls, xlsb, slk).
    pub fn save_path(&self) -> String {
        let lower = self.path.to_ascii_lowercase();
        match [".xls", ".xlsb", ".slk"].iter().find(|e| lower.ends_with(*e)) {
            Some(_) => self.beside("xlsx"),
            None => self.path.clone(),
        }
    }

    /// The workbook as the bytes of its file, in the format `path`'s extension names: xlsx,
    /// xlsm, ods, csv, tsv, html, json, or pdf (the active sheet, as printed).
    pub fn file_bytes(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".pdf") {
            let r = self.run("file.exportPdf", json!({}))?;
            let data = r.get("base64").and_then(Json::as_str).ok_or("no PDF")?;
            return gridcraft_engine::io::base64_decode(data).ok_or_else(|| "bad PDF data".to_string());
        }
        let d = self.session.doc().map_err(|e| e.to_string())?;
        if lower.ends_with(".ods") { return crate::ods::write(&d.wb); }
        gridcraft_engine::io::save_file_bytes(&d.wb, path).map_err(|e| e.to_string())
    }

    /// The file beside this one with extension `ext` (`report.xlsx` -> `report.pdf`).
    pub fn beside(&self, ext: &str) -> String {
        let name_at = self.path.rfind('/').map_or(0, |i| i + 1);
        let own = self.path.starts_with("@/");
        let stem = match self.path[name_at..].rfind('.') {
            Some(dot) if dot > 0 && own => &self.path[name_at..name_at + dot],
            Some(dot) if dot > 0 => &self.path[..name_at + dot],
            _ if own => &self.path[name_at..],
            _ => &self.path[..],
        };
        format!("{stem}.{ext}")
    }

    /// Writes the active sheet as a PDF beside the workbook (Ctrl+P: SexOS prints to PDF).
    /// Returns (its name, size).
    pub fn export_pdf(&mut self) -> Result<(String, usize), String> {
        let path = self.beside("pdf");
        let bytes = self.file_bytes(&path)?;
        replace_file(&path, &bytes).map_err(|e| format!("Could not write {path}: {e}"))?;
        Ok((path, bytes.len()))
    }

    /// Save As: writes the workbook as `path` in the format its extension names and, unless
    /// that is a copy-only format (pdf, html, csv or tsv of one sheet, json), goes on
    /// working in it. A name with no extension keeps this file's. Returns (path, size).
    pub fn save_as(&mut self, path: &str) -> Result<(String, usize), String> {
        let name_at = path.rfind('/').map_or(0, |i| i + 1);
        let path = if path[name_at..].contains('.') { path.to_string() } else {
            let ext = self.save_path().rsplit_once('.').map_or("xlsx".to_string(), |(_, e)| e.to_string());
            format!("{path}.{ext}")
        };
        let lower = path.to_ascii_lowercase();
        let copy = [".pdf", ".html", ".htm", ".json"].iter().any(|e| lower.ends_with(e))
            || ([".csv", ".tsv"].iter().any(|e| lower.ends_with(e)) && self.sheets().len() > 1);
        let bytes = self.file_bytes(&path)?;
        replace_file(&path, &bytes).map_err(|e| format!("Could not save {path}: {e}"))?;
        if !copy {
            let old = std::mem::replace(&mut self.path, path.clone());
            let _ = std::fs::remove_file(recovery_path(&old));
            if let Ok(d) = self.session.doc_mut() {
                d.saved = d.wb.clone();
                d.path = Some(path.clone());
            }
        }
        Ok((path, bytes.len()))
    }

    /// Keeps unsaved changes safe in [`recovery_path`] (on close, and after a quiet moment),
    /// as GridCraft's own xlsx so nothing is lost.
    pub fn write_recovery(&self) -> Result<(), String> {
        let d = self.session.doc().map_err(|e| e.to_string())?;
        let path = recovery_path(&self.path);
        let bytes = gridcraft_engine::io::save_bytes_as(&d.wb, &path, gridcraft_engine::io::FileKind::Xlsx).map_err(|e| e.to_string())?;
        replace_file(&path, &bytes).map_err(|e| format!("Could not keep the changes in {path}: {e}"))
    }

    /// Opens `path` with the unsaved changes kept from last time, if there are any: the
    /// workbook is then changed (not saved) and Ctrl+S writes it to `path`.
    pub fn open_recovered(path: &str) -> Option<Book> {
        let kept = recovery_path(path);
        std::fs::metadata(&kept).ok()?;
        let (mut book, _) = Book::open(&kept).ok()?;
        book.path = path.to_string();
        if let Ok(d) = book.session.doc_mut() {
            d.path = Some(path.to_string());
            // Not the saved file: a fresh copy reads as changed.
            d.saved = std::sync::Arc::new((*d.wb).clone());
        }
        Some(book)
    }

    /// Throws the kept changes away.
    pub fn forget_recovery(&self) { let _ = std::fs::remove_file(recovery_path(&self.path)); }

    /// Ctrl+G: `B7`, `B7:D9`, `Sheet2!B7` or `'My sheet'!B7`. Activates the sheet it names
    /// and returns the range.
    pub fn go_to(&mut self, place: &str) -> Result<Range, String> {
        let place = place.trim();
        let (sheet, cells) = match place.rsplit_once('!') {
            Some((sheet, cells)) => (Some(sheet.trim_matches('\'').replace("''", "'")), cells),
            None => (None, place),
        };
        let bad = || format!("{place} is not a cell: try B7, B7:D9 or Sheet2!B7");
        let r = gridcraft_engine::core::RangeRef::parse(cells).ok_or_else(bad)?;
        if let Some(name) = sheet {
            let at = self.sheets().iter().position(|s| s.eq_ignore_ascii_case(&name)).ok_or_else(|| format!("There is no sheet called {name}"))?;
            self.activate(at)?;
        }
        Ok(Range { r0: r.start.row, c0: r.start.col, r1: r.end.row.min(LAST_ROW), c1: r.end.col.min(LAST_COL) })
    }

    /// Inserts as many rows above `range` (or columns left of it) as it spans.
    pub fn insert_lines(&mut self, range: Range, rows: bool) -> Result<(), String> {
        let (t, l, b, r) = range.bounds();
        if rows { self.run("home.insertRows", json!({ "rows": format!("{}:{}", t + 1, b + 1) })) }
        else { self.run("home.insertColumns", json!({ "cols": format!("{}:{}", column_name(l), column_name(r)) })) }.map(|_| ())
    }

    /// Deletes the rows (or columns) `range` spans.
    pub fn delete_lines(&mut self, range: Range, rows: bool) -> Result<(), String> {
        let (t, l, b, r) = range.bounds();
        if rows { self.run("home.deleteRows", json!({ "rows": format!("{}:{}", t + 1, b + 1) })) }
        else { self.run("home.deleteColumns", json!({ "cols": format!("{}:{}", column_name(l), column_name(r)) })) }.map(|_| ())
    }

    /// Writes the workbook to its file (see [`Book::save_path`]). Returns the size.
    pub fn save(&mut self) -> Result<usize, String> {
        let path = self.save_path();
        let bytes = self.file_bytes(&path)?;
        replace_file(&path, &bytes).map_err(|e| format!("Could not save {path}: {e}"))?;
        self.forget_recovery();
        self.path = path.clone();
        if let Ok(d) = self.session.doc_mut() {
            d.saved = d.wb.clone();
            d.path = Some(path);
        }
        Ok(bytes.len())
    }
}

/// The last row and column Excel's grid has, counted from 0.
pub const LAST_ROW: u32 = 1_048_575;
pub const LAST_COL: u32 = 16_383;

/// Where unsaved changes to `path` are kept: `report.ods` -> `report.ods.recovered.xlsx`.
pub fn recovery_path(path: &str) -> String {
    if path.starts_with("@/") {
        let hash = path.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        format!("spreadeagle-recovered-{hash:016x}.xlsx")
    } else { format!("{path}.recovered.xlsx") }
}

/// Write a complete file beside the destination before Files replaces its name.
fn replace_file(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::{Error, ErrorKind};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut staged = None;
    for _ in 0..16 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = if path.starts_with("@/") { format!("spreadeagle-{n}.tmp") }
            else { format!("{path}.spreadeagle-{n}.tmp") };
        match OpenOptions::new().write(true).create_new(true).open(&name) {
            Ok(file) => { staged = Some((name, file)); break; }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let (name, mut file) = staged.ok_or_else(|| Error::new(ErrorKind::AlreadyExists, "no free Spreadeagle staging name"))?;
    let write_result = (|| {
        file.write_all(bytes)?;
        file.flush()?;
        sync_staged(&file)
    })();
    drop(file);
    let result = write_result.and_then(|()| rename_staged(&name, path));
    if result.is_err() { let _ = std::fs::remove_file(&name); }
    result
}

#[cfg(target_os = "hermit")]
fn sync_staged(file: &std::fs::File) -> std::io::Result<()> {
    use sex_std::fs::FileExt;
    file.fsync()
}

#[cfg(not(target_os = "hermit"))]
fn sync_staged(file: &std::fs::File) -> std::io::Result<()> { file.sync_all() }

#[cfg(target_os = "hermit")]
fn rename_staged(from: &str, to: &str) -> std::io::Result<()> { sex_std::fs::rename(from, to) }

#[cfg(not(target_os = "hermit"))]
fn rename_staged(from: &str, to: &str) -> std::io::Result<()> { std::fs::rename(from, to) }

/// `A`, `B`, … `Z`, `AA`, … for column `col` counted from 0.
pub fn column_name(col: u32) -> String {
    let mut n = u64::from(col) + 1;
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
    const WARNING_ODS: &[u8] = include_bytes!("../assets/import-warning.ods");

    fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("spreadeagle-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn warns_when_ods_import_loses_source_formatting() {
        let path = temp("import-warning.ods");
        std::fs::write(&path, WARNING_ODS).unwrap();
        let (book, warnings) = Book::open(&path).unwrap();
        assert_eq!(book.shown(0, 0).text, "Fruit");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("Source formatting")), "{warnings:?}");
    }

    #[test]
    fn column_names() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
        // Issue #282: the last u32 column must not overflow.
        assert_eq!(column_name(u32::MAX), "MWLQKWV");
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
    fn sorts_by_two_columns_and_preserves_header_after_save() {
        let mut book = Book::blank(&temp("multi-sort.xlsx"));
        for (row, group, score) in [(1, "B", "2"), (2, "A", "1"), (3, "B", "5"), (4, "A", "3")] {
            book.set(row, 0, group).unwrap();
            book.set(row, 1, score).unwrap();
        }
        book.set(0, 0, "Group").unwrap();
        book.set(0, 1, "Score").unwrap();
        let range = Range { r0: 0, c0: 0, r1: 4, c1: 1 };
        for bad in ["A asc", "A asc, A desc", "C asc, B desc", "A up, B desc", "A asc, B"] {
            assert!(book.sort_multi(range, bad).is_err(), "{bad}");
        }
        book.sort_multi(range, "A asc, B desc").unwrap();
        let rows = |book: &Book| (0..=4).map(|r| (book.shown(r, 0).text, book.shown(r, 1).text)).collect::<Vec<_>>();
        assert_eq!(rows(&book), [("Group", "Score"), ("A", "3"), ("A", "1"), ("B", "5"), ("B", "2")].map(|(a, b)| (a.to_string(), b.to_string())));
        book.undo().unwrap();
        assert_eq!(rows(&book)[1], ("B".into(), "2".into()));
        book.redo().unwrap();
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(rows(&Book::open(&path).unwrap().0), rows(&book));
    }

    #[test]
    fn filter_hides_other_rows() {
        let mut book = budget("filter.xlsx");
        assert!(book.filter_range().is_none());
        book.filter_by(Range::cell(2, 0), 0, Some("Food")).unwrap();
        assert_eq!(book.filter_choices(0), ["Food", "Fun", "Rent"]);
        assert_eq!(book.filter_range(), Some(Range { r0: 0, c0: 0, r1: 3, c1: 1 }));
        let heights: Vec<u32> = (0..4).map(|r| book.row_height(r)).collect();
        assert!(heights[0] > 0 && heights[2] > 0, "{heights:?}");
        assert_eq!((heights[1], heights[3]), (0, 0), "{heights:?}");
        let choice = book.filter_choices(0)[0].clone();
        book.filter_by(Range::cell(2, 0), 0, Some(&choice)).unwrap();
        assert_eq!((book.row_height(1), book.row_height(2) > 0), (0, true));
        book.filter_by(Range::cell(2, 0), 0, None).unwrap();
        assert!((0..4).all(|r| book.row_height(r) > 0));
        book.filter_by_values(Range::cell(2, 0), 0, &["Food".into(), "Fun".into()]).unwrap();
        assert_eq!((book.row_height(1), book.row_height(2) > 0, book.row_height(3) > 0), (0, true, true));
        book.filter_by_values(Range::cell(2, 0), 0, &[]).unwrap();
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
    fn exact_filter_lookup_reaches_beyond_menu_preview() {
        let path = temp("filter-large.xlsx");
        std::fs::write(&path, include_bytes!("../assets/filter-overflow.xlsx")).unwrap();
        let mut book = Book::open(&path).unwrap().0;
        book.toggle_filter(Range::cell(0, 0)).unwrap();
        assert_eq!(book.filter_range().unwrap().bounds().2, 1_100);
        assert_eq!(book.filter_choices(0).len(), 1_000);
        assert_eq!(book.filter_exact_choice(0, "item 1100").as_deref(), Some("Item 1100"));
        assert_eq!(book.filter_exact_choice(0, "not present"), None);
        book.filter_by_values(Range::cell(1, 0), 0, &["Item 1100".into()]).unwrap();
        assert!(book.row_height(1_100) > 0);
        assert_eq!(book.row_height(1), 0);
    }

    #[test]
    fn filter_state_reports_selected_values_and_condition_after_reopen() {
        let mut book = budget("filter-state.xlsx");
        let at = Range::cell(2, 0);
        assert_eq!(book.filter_state(0), FilterState::None);
        book.filter_by_values(at, 0, &["Food".into(), "Fun".into()]).unwrap();
        assert_eq!(book.filter_state(0), FilterState::Values { values: vec!["Food".into(), "Fun".into()], blanks: false });
        assert_eq!(book.filter_state(1), FilterState::None);
        let path = book.path.clone();
        book.save().unwrap();
        let (mut reopened, _) = Book::open(&path).unwrap();
        assert_eq!(reopened.filter_state(0), book.filter_state(0));
        reopened.filter_by_condition(at, 0, "contains oo").unwrap();
        assert_eq!(reopened.filter_state(0), FilterState::Other);
        reopened.filter_by_values(at, 0, &[]).unwrap();
        assert_eq!(reopened.filter_state(0), FilterState::None);
        reopened.filter_by_values(at, 1, &["200".into()]).unwrap();
        assert_eq!(reopened.filter_state(1), FilterState::Values { values: vec!["200".into()], blanks: false });
        assert_eq!(reopened.filter_state(0), FilterState::None);
    }

    #[test]
    fn filter_conditions_match_text_and_numeric_cells() {
        let mut book = budget("condition.xlsx");
        assert!(book.filter_by_condition(Range::cell(1, 0), 0, "contains").is_err());
        assert!(book.filter_by_condition(Range::cell(1, 0), 1, "> many").is_err());
        assert!(book.filter_range().is_none());
        book.filter_by_condition(Range::cell(1, 0), 0, "contains oo").unwrap();
        assert_eq!((book.row_height(1), book.row_height(2) > 0, book.row_height(3)), (0, true, 0));
        book.filter_by_condition(Range::cell(1, 0), 0, "starts f").unwrap();
        assert_eq!((book.row_height(1), book.row_height(2) > 0, book.row_height(3) > 0), (0, true, true));
        book.filter_by_condition(Range::cell(1, 0), 0, "contains impossible").unwrap();
        assert_eq!((book.row_height(1), book.row_height(2), book.row_height(3)), (0, 0, 0));
        book.filter_by_values(Range::cell(1, 0), 0, &[]).unwrap();
        book.filter_by_condition(Range::cell(1, 1), 1, "> 200").unwrap();
        assert_eq!((book.row_height(1) > 0, book.row_height(2) > 0, book.row_height(3)), (true, true, 0));
    }

    #[test]
    fn compound_filter_conditions_match_and_or_and_reopen() {
        let mut book = budget("compound-condition.xlsx");
        let at = Range::cell(1, 0);
        for bad in ["starts f or > many", "> 1 and <= infinity", "starts F or contains oo and notcontains Food"] {
            assert!(book.filter_by_condition(at, 0, bad).is_err(), "{bad}");
        }
        assert!(book.filter_range().is_none());
        book.filter_by_condition(at, 0, "contains and").unwrap();
        book.filter_by_condition(at, 0, "contains or").unwrap();
        book.filter_by_condition(at, 0, "contains Food and Rent").unwrap();
        assert_eq!((book.row_height(1), book.row_height(2), book.row_height(3)), (0, 0, 0));
        book.filter_by_condition(at, 0, "contains oo or starts R").unwrap();
        assert_eq!((book.row_height(1) > 0, book.row_height(2) > 0, book.row_height(3) > 0), (true, true, false));
        book.filter_by_condition(at, 0, "starts F and notcontains oo").unwrap();
        assert_eq!((book.row_height(1) > 0, book.row_height(2) > 0, book.row_height(3) > 0), (false, false, true));
        book.filter_by_values(at, 0, &[]).unwrap();
        book.filter_by_condition(at, 1, ">= 150 and <= 300").unwrap();
        assert_eq!((book.row_height(1) > 0, book.row_height(2) > 0, book.row_height(3) > 0), (false, true, true));
        let path = book.path.clone();
        book.save().unwrap();
        let reopened = Book::open(&path).unwrap().0;
        assert_eq!((reopened.row_height(1) > 0, reopened.row_height(2) > 0, reopened.row_height(3) > 0), (false, true, true));
    }

    #[test]
    fn equality_inequality_and_wildcards_filter_and_reopen() {
        let mut book = budget("filter-equality.xlsx");
        let at = Range::cell(1, 0);
        for bad in ["=", "<>", "!=", "=  "] {
            assert!(book.filter_by_condition(at, 0, bad).is_err(), "{bad}");
        }
        book.filter_by_condition(at, 0, "= Food").unwrap();
        assert_eq!([1, 2, 3].map(|r| book.row_height(r) > 0), [false, true, false]);
        book.filter_by_condition(at, 0, "<> Food").unwrap();
        assert_eq!([1, 2, 3].map(|r| book.row_height(r) > 0), [true, false, true]);
        book.filter_by_condition(at, 0, "= F*").unwrap();
        assert_eq!([1, 2, 3].map(|r| book.row_height(r) > 0), [false, true, true]);
        book.filter_by_condition(at, 0, "= F* or = Rent").unwrap();
        assert_eq!([1, 2, 3].map(|r| book.row_height(r) > 0), [true, true, true]);
        book.filter_by_condition(at, 0, "!= F*").unwrap();
        assert_eq!([1, 2, 3].map(|r| book.row_height(r) > 0), [true, false, false]);
        let path = book.path.clone();
        book.save().unwrap();
        let reopened = Book::open(&path).unwrap().0;
        assert_eq!([1, 2, 3].map(|r| reopened.row_height(r) > 0), [true, false, false]);
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
    fn row_height_drag_result_undoes_and_survives_xlsx() {
        let mut book = budget("row-height.xlsx");
        let original = book.row_height(0);
        book.set_row_height(0, 72).unwrap();
        assert_eq!(book.row_height(0), 72);
        assert_eq!(book.row_height(1), 20);
        book.undo().unwrap();
        assert_eq!(book.row_height(0), original);
        book.redo().unwrap();
        assert_eq!(book.row_height(0), 72);
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(Book::open(&path).unwrap().0.row_height(0), 72);
    }

    #[test]
    fn wrapped_cell_grows_row_undoes_and_round_trips() {
        assert_eq!(wrap_lines("one two three", 7, 3, |s| s.len() as i32), ["one two", "three"]);
        assert_eq!(wrap_lines("abcdefgh", 3, 3, |s| s.len() as i32), ["abc", "def", "gh"]);
        assert_eq!(wrap_lines("alpha\nbeta", 8, 3, |s| s.len() as i32), ["alpha", "beta"]);
        assert_eq!(wrap_lines("abcdefgh", 2, 1, |s| s.len() as i32), ["ab"]);
        for ext in ["xlsx", "ods"] {
            let mut book = Book::blank(&temp(&format!("wrap.{ext}")));
            book.set(0, 0, "Quarterly planning notes for our team").unwrap();
            book.set_col_width(0, 72).unwrap();
            let original = book.row_height(0);
            book.set_wrap(Range::cell(0, 0), true).unwrap();
            assert!(book.wrapped(0, 0), "{ext}");
            assert!(book.row_height(0) > original, "{ext}: {} <= {original}", book.row_height(0));
            book.undo().unwrap();
            assert!(!book.wrapped(0, 0), "{ext}");
            book.redo().unwrap();
            assert!(book.wrapped(0, 0), "{ext}");
            let path = book.path.clone();
            book.save().unwrap();
            let (again, _) = Book::open(&path).unwrap();
            assert!(again.wrapped(0, 0), "{ext}");
            assert_eq!(again.input(0, 0), "Quarterly planning notes for our team");
            if ext == "ods" {
                let raw = std::fs::read(&path).unwrap();
                assert!(crate::ods::extras(&raw).unwrap().wraps.contains(&(0, 0, 0)));
            }
        }
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
    fn chart_types_round_trip_in_xlsx() {
        let mut book = budget("chart-types.xlsx");
        let range = Range { r0: 0, c0: 0, r1: 3, c1: 1 };
        assert!(book.add_chart_kind(range, 1, 3, "unsupported").is_err());
        for kind in ["column", "line", "bar", "pie"] {
            book.add_chart_kind(range, 1, 3, kind).unwrap();
        }
        let kinds = |book: &Book| -> Vec<String> {
            book.session.doc().unwrap().wb.active().unwrap().charts.iter()
                .map(|chart| format!("{:?}", chart.kind)).collect()
        };
        assert_eq!(kinds(&book).join(","), "ColumnClustered,Line,BarClustered,Pie");
        let path = book.path.clone();
        book.save().unwrap();
        let (reopened, _) = Book::open(&path).unwrap();
        assert_eq!(kinds(&reopened).join(","), "ColumnClustered,Line,BarClustered,Pie");
    }

    #[test]
    fn chart_title_and_type_edit_round_trip_in_xlsx() {
        let mut book = budget("chart-edit.xlsx");
        let range = Range { r0: 0, c0: 0, r1: 3, c1: 1 };
        book.add_chart(range, 0, 3).unwrap();
        let chart = book.chart_at_anchor(0, 3).unwrap();
        assert!(book.edit_chart(chart.id, "type donut").is_err());
        assert!(book.edit_chart(chart.id + 100, "type line").is_err());
        book.edit_chart(chart.id, "title Budget by category").unwrap();
        book.edit_chart(chart.id, "type line").unwrap();
        let state = |book: &Book| {
            let chart = &book.session.doc().unwrap().wb.active().unwrap().charts[0];
            (chart.title.clone(), format!("{:?}", chart.kind))
        };
        assert_eq!(state(&book), (Some("Budget by category".into()), "Line".into()));
        book.edit_chart(chart.id, "clear title").unwrap();
        assert_eq!(state(&book).0, None);
        book.undo().unwrap();
        assert_eq!(state(&book).0, Some("Budget by category".into()));
        book.undo().unwrap();
        assert_eq!(state(&book).1, "ColumnClustered");
        book.redo().unwrap();
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(state(&Book::open(&path).unwrap().0), (Some("Budget by category".into()), "Line".into()));
    }

    #[test]
    fn chart_layout_controls_undo_and_round_trip_in_xlsx() {
        let mut book = budget("chart-layout.xlsx");
        book.add_chart(Range { r0: 0, c0: 0, r1: 3, c1: 1 }, 0, 3).unwrap();
        let id = book.chart_at_anchor(0, 3).unwrap().id;
        for bad in ["legend diagonal", "x title ", "y title ", "labels maybe", "gridlines maybe"] {
            assert!(book.edit_chart(id, bad).is_err(), "{bad}");
        }
        book.edit_chart(id, "legend right").unwrap();
        book.edit_chart(id, "x title Category").unwrap();
        book.edit_chart(id, "y title Annual cost").unwrap();
        book.edit_chart(id, "labels on").unwrap();
        book.edit_chart(id, "gridlines off").unwrap();
        let state = |book: &Book| {
            let c = &book.session.doc().unwrap().wb.active().unwrap().charts[0];
            (format!("{:?}", c.legend), c.x_title.clone(), c.y_title.clone(), c.data_labels, c.gridlines)
        };
        assert_eq!(state(&book), ("Right".into(), Some("Category".into()), Some("Annual cost".into()), true, false));
        book.undo().unwrap();
        assert!(state(&book).4);
        book.redo().unwrap();
        book.edit_chart(id, "clear x title").unwrap();
        assert_eq!(state(&book).1, None);
        book.undo().unwrap();
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(state(&Book::open(&path).unwrap().0), ("Right".into(), Some("Category".into()), Some("Annual cost".into()), true, false));
    }

    #[test]
    fn doughnut_design_controls_render_undo_and_round_trip() {
        let mut book = budget("chart-design.xlsx");
        book.add_chart(Range { r0: 0, c0: 0, r1: 3, c1: 1 }, 0, 3).unwrap();
        let id = book.chart_at_anchor(0, 3).unwrap().id;
        for bad in ["hole 50", "angle 90", "type radar"] {
            assert!(book.edit_chart(id, bad).is_err(), "{bad}");
        }
        book.edit_chart(id, "type doughnut").unwrap();
        let before = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        for bad in ["hole 9", "hole 91", "angle 360", "angle -1"] {
            assert!(book.edit_chart(id, bad).is_err(), "{bad}");
        }
        book.edit_chart(id, "hole 50").unwrap();
        let after_hole = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        assert_ne!(before, after_hole);
        book.edit_chart(id, "angle 90").unwrap();
        let after_angle = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        assert_ne!(after_hole, after_angle);
        let state = |book: &Book| {
            let chart = &book.session.doc().unwrap().wb.active().unwrap().charts[0];
            (chart.kind, chart.hole_size, chart.first_slice_angle)
        };
        assert_eq!(state(&book), (gridcraft_engine::model::ChartKind::Doughnut, 50, 90));
        book.undo().unwrap();
        assert_ne!(state(&book).2, 90);
        book.redo().unwrap();
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(state(&Book::open(&path).unwrap().0), state(&book));
        let mut inserted = budget("doughnut-insert.xlsx");
        inserted.add_chart_kind(Range { r0: 0, c0: 0, r1: 3, c1: 1 }, 0, 3, "doughnut").unwrap();
        assert_eq!(state(&inserted).0, gridcraft_engine::model::ChartKind::Doughnut);
        assert!(inserted.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).is_some());
    }

    #[test]
    fn chart_switch_rebuilds_series_and_survives_xlsx() {
        let mut book = budget("chart-switch.xlsx");
        book.add_chart(Range { r0: 0, c0: 0, r1: 3, c1: 1 }, 0, 3).unwrap();
        let id = book.chart_at_anchor(0, 3).unwrap().id;
        book.edit_chart(id, "type line").unwrap();
        let state = |book: &Book| {
            let chart = &book.session.doc().unwrap().wb.active().unwrap().charts[0];
            (chart.by_rows, chart.series.len())
        };
        let before = state(&book);
        let pixels_before = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        assert_eq!(before, (false, 1));
        book.edit_chart(id, "switch").unwrap();
        assert_eq!(state(&book), (true, 3));
        let pixels_after = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        assert_ne!(pixels_before, pixels_after);
        book.undo().unwrap();
        assert_eq!(state(&book), before);
        book.redo().unwrap();
        assert_eq!(state(&book), (true, 3));
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(state(&Book::open(&path).unwrap().0).1, 3);
    }

    #[test]
    fn chart_data_range_expands_series_and_round_trips() {
        let mut book = Book::blank(&temp("chart-data.xlsx"));
        book.set(0, 0, "Month").unwrap();
        book.set(0, 1, "Sales").unwrap();
        for (row, month, sales) in [(1, "Jan", "10"), (2, "Feb", "20"), (3, "Mar", "30")] {
            book.set(row, 0, month).unwrap();
            book.set(row, 1, sales).unwrap();
        }
        book.add_chart(Range { r0: 0, c0: 0, r1: 2, c1: 1 }, 0, 3).unwrap();
        let id = book.chart_at_anchor(0, 3).unwrap().id;
        let values = |book: &Book| book.session.doc().unwrap().wb.active().unwrap().charts[0].series[0].values.clone();
        assert!(values(&book).contains("$B$2:$B$3"));
        let before = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        for bad in ["data ", "data nowhere", "data A1:XFD1048576"] {
            assert!(book.edit_chart(id, bad).is_err(), "{bad}");
        }
        book.edit_chart(id, "data A1:B4").unwrap();
        assert!(values(&book).contains("$B$2:$B$4"));
        let after = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        assert_ne!(before, after);
        book.undo().unwrap();
        assert!(values(&book).contains("$B$2:$B$3"));
        book.redo().unwrap();
        assert!(values(&book).contains("$B$2:$B$4"));
        let path = book.path.clone();
        book.save().unwrap();
        assert!(values(&Book::open(&path).unwrap().0).contains("$B$2:$B$4"));
    }

    #[test]
    fn chart_remove_series_keeps_remaining_data_and_round_trips() {
        let mut book = Book::blank(&temp("chart-remove-series.xlsx"));
        for (col, title) in [(0, "Month"), (1, "North"), (2, "South")] {
            book.set(0, col, title).unwrap();
        }
        for (row, month, north, south) in [(1, "Jan", "10", "60"), (2, "Feb", "20", "50"), (3, "Mar", "30", "40")] {
            for (col, value) in [(0, month), (1, north), (2, south)] { book.set(row, col, value).unwrap(); }
        }
        book.add_chart(Range { r0: 0, c0: 0, r1: 3, c1: 2 }, 0, 4).unwrap();
        let id = book.chart_at_anchor(0, 4).unwrap().id;
        let series = |book: &Book| book.session.doc().unwrap().wb.active().unwrap().charts[0].series.clone();
        let original = series(&book);
        assert_eq!(original.len(), 2);
        for bad in ["remove 0", "remove 3", "remove x"] { assert!(book.edit_chart(id, bad).is_err(), "{bad}"); }
        let before = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        book.edit_chart(id, "remove 2").unwrap();
        assert_eq!(series(&book), original[..1]);
        assert!(book.edit_chart(id, "remove 1").is_err());
        let after = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        assert_ne!(before, after);
        book.undo().unwrap();
        assert_eq!(series(&book), original);
        book.redo().unwrap();
        assert_eq!(series(&book), original[..1]);
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(series(&Book::open(&path).unwrap().0).len(), 1);
    }

    #[test]
    fn chart_data_table_changes_preview_and_round_trips() {
        let mut book = budget("chart-presentation.xlsx");
        book.add_chart(Range { r0: 0, c0: 0, r1: 3, c1: 1 }, 0, 3).unwrap();
        let id = book.chart_at_anchor(0, 3).unwrap().id;
        for bad in ["table maybe", "table enable"] {
            assert!(book.edit_chart(id, bad).is_err(), "{bad}");
        }
        let before = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        book.edit_chart(id, "table on").unwrap();
        let state = |book: &Book| {
            let c = &book.session.doc().unwrap().wb.active().unwrap().charts[0];
            c.data_table.is_some()
        };
        assert!(state(&book));
        let after = book.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        assert_ne!(before, after, "the chart's native pixels must show the data table");
        book.undo().unwrap();
        assert!(!state(&book));
        book.redo().unwrap();
        let path = book.path.clone();
        book.save().unwrap();
        assert!(state(&Book::open(&path).unwrap().0));
    }

    #[test]
    fn ods_opens_and_saves_as_ods() {
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
        assert!(book.save_path().ends_with("sample.ods"));
        book.save().unwrap();
        assert!(book.path.ends_with("sample.ods"));
        let (again, _) = Book::open(&book.path).unwrap();
        assert_eq!(again.shown(4, 1).text, "50");
        assert_eq!(again.input(4, 1), "=SUM(B2:B4)");
    }

    #[test]
    fn imported_ods_formula_arguments_recalculate() {
        let path = temp("formula-compat.ods");
        std::fs::write(&path, include_bytes!("../assets/formula-compat.ods")).unwrap();
        let (mut book, warnings) = Book::open(&path).unwrap();
        assert_eq!(book.sheets(), ["Formula checks", "Q1Q2"], "{warnings:?}");
        assert_eq!(book.input(0, 1), "=IF(A1>0,SUM(A1,A3),0)");
        assert_eq!(book.input(1, 1), "=IFERROR(A1/A2,99)");
        assert_eq!(book.input(2, 1), "=ROUND(AVERAGE(A1,A3),1)");
        assert_eq!(book.input(3, 1), "=COUNTIF(A1:A3,\">0\")");
        assert_eq!(book.input(4, 1), "=Q1Q2!A1+1");
        assert_eq!(book.shown(4, 1).text, "5");
        assert_eq!(column(&book, 1, 4), ["15", "99", "7.5", "2"]);
        book.set(2, 0, "7").unwrap();
        assert_eq!(column(&book, 1, 4), ["17", "99", "8.5", "2"]);
        book.activate(1).unwrap();
        book.set(0, 0, "8").unwrap();
        book.activate(0).unwrap();
        assert_eq!(book.shown(4, 1).text, "9");
        let xlsx = temp("formula-compat.xlsx");
        book.save_as(&xlsx).unwrap();
        let (again, _) = Book::open(&xlsx).unwrap();
        assert_eq!(column(&again, 1, 4), ["17", "99", "8.5", "2"]);
        assert_eq!(again.shown(4, 1).text, "9");

        let csv = temp("formula-compat.csv");
        std::fs::write(&csv, "Amount,Rate\n10,2\n7,3\n").unwrap();
        let (mut imported, _) = Book::open(&csv).unwrap();
        imported.set(1, 2, "=SUM(A2*B2,A3*B3)").unwrap();
        assert_eq!(imported.shown(1, 2).text, "41");
        imported.set(2, 0, "8").unwrap();
        assert_eq!(imported.shown(1, 2).text, "44");
    }

    #[test]
    fn ods_keeps_sheets_formulas_dates_fonts_and_widths() {
        let path = temp("kept.ods");
        let mut book = Book::blank(&path);
        book.set(0, 0, "Item").unwrap();
        book.set(0, 1, "When").unwrap();
        book.set(0, 2, "Share").unwrap();
        book.toggle_style(Range { r0: 0, c0: 0, r1: 0, c1: 2 }, "bold").unwrap();
        book.set(1, 0, "Tea & \"cakes\" <b>").unwrap();
        book.set(1, 1, "2026-11-19").unwrap();
        book.set(1, 2, "25%").unwrap();
        book.set(2, 0, "Two\nlines").unwrap();
        book.set(2, 1, "1.5").unwrap();
        book.set(2, 2, "TRUE").unwrap();
        book.toggle_style(Range::cell(2, 0), "italic").unwrap();
        book.set(4, 1, "=SUM(B3,10)*2").unwrap();
        book.set(5, 1, "=IF(B5>10,\"big\",\"small\")").unwrap();
        book.set_col_width(0, 150).unwrap();
        book.run("home.insertSheet", json!({ "name": "Other sheet" })).unwrap();
        let other = book.sheets().iter().position(|s| s == "Other sheet").unwrap();
        book.activate(other).unwrap();
        book.set(0, 0, "='Sheet1'!B5+1").unwrap();
        let first = book.sheets()[0].clone();
        book.activate(0).unwrap();
        let shown: Vec<_> = (0..6).flat_map(|r| (0..3).map(move |c| (r, c))).map(|(r, c)| (book.input(r, c), book.shown(r, c).text)).collect();
        book.save().unwrap();
        let (mut again, warnings) = Book::open(&path).unwrap();
        assert_eq!(again.sheets(), book.sheets(), "{warnings:?}");
        let back: Vec<_> = (0..6).flat_map(|r| (0..3).map(move |c| (r, c))).map(|(r, c)| (again.input(r, c), again.shown(r, c).text)).collect();
        assert_eq!(back, shown);
        assert_eq!(again.font(0, 1), (true, false, false));
        assert_eq!(again.font(2, 0), (false, true, false));
        assert_eq!(again.col_width(0), 150);
        assert!(!again.dirty());
        again.activate(other).unwrap();
        assert_eq!(again.shown(0, 0).text, "24", "sheet {first}");
        assert_eq!(again.input(0, 0), "=Sheet1!B5+1");
    }

    #[test]
    fn exports_pdf_and_saves_as_other_formats() {
        let path = temp("export.xlsx");
        std::fs::write(&path, WELCOME).unwrap();
        let (mut book, _) = Book::open(&path).unwrap();
        let (pdf, n) = book.export_pdf().unwrap();
        assert!(pdf.ends_with("export.pdf") && n > 500);
        assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
        assert_eq!(book.path, path, "a PDF is a copy");
        book.set(1, 1, "7").unwrap();
        let (csv, _) = book.save_as(&temp("copy.csv")).unwrap();
        assert!(std::fs::read_to_string(&csv).unwrap().contains("Rent"));
        assert_eq!(book.path, path, "one sheet of two is a copy");
        assert!(book.dirty());
        let (ods, _) = book.save_as(&temp("moved")).unwrap();
        assert!(ods.ends_with("moved.xlsx"));
        assert_eq!(book.path, ods);
        assert!(!book.dirty());
        assert!(book.save_as(&temp("bad.cvs")).is_err());
    }

    #[test]
    fn unsaved_changes_are_recovered() {
        let path = temp("recover.xlsx");
        std::fs::write(&path, WELCOME).unwrap();
        let (mut book, _) = Book::open(&path).unwrap();
        assert!(Book::open_recovered(&path).is_none());
        book.set(1, 1, "999").unwrap();
        book.write_recovery().unwrap();
        let mut back = Book::open_recovered(&path).unwrap();
        assert_eq!(back.path, path);
        assert!(back.dirty());
        assert_eq!(back.shown(1, 1).text, "999");
        back.save().unwrap();
        assert!(Book::open_recovered(&path).is_none(), "saving forgets the kept copy");
        assert_eq!(Book::open(&path).unwrap().0.shown(1, 1).text, "999");
    }

    #[test]
    fn replacement_keeps_a_previous_workbook_until_complete() {
        let path = temp("replacement.xlsx");
        std::fs::write(&path, b"previous workbook").unwrap();
        let stale = format!("{path}.spreadeagle-0.tmp");
        std::fs::write(&stale, b"stale staging data").unwrap();
        replace_file(&path, b"new complete workbook").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new complete workbook");
        assert_eq!(std::fs::read(&stale).unwrap(), b"stale staging data");
        std::fs::remove_file(stale).unwrap();
    }

    #[test]
    fn exact_file_grant_uses_own_folder_for_export_and_recovery() {
        let book = Book::blank("@/documents/budget.xls");
        assert_eq!(book.save_path(), "budget.xlsx");
        assert_eq!(book.beside("pdf"), "budget.pdf");
        let kept = recovery_path("@/documents/budget.ods");
        assert!(kept.starts_with("spreadeagle-recovered-") && kept.ends_with(".xlsx"));
        assert_eq!(kept, recovery_path("@/documents/budget.ods"));
    }

    #[test]
    fn exposes_more_than_two_hundred_distinct_formula_functions() {
        let mut book = Book::blank(&temp("functions.xlsx"));
        let catalog = book.run("formulas.functions", json!({})).unwrap();
        let entries = catalog.as_array().unwrap();
        let names: std::collections::BTreeSet<_> = entries.iter()
            .filter_map(|entry| entry["name"].as_str()).collect();
        assert!(names.len() >= 200, "only {} distinct functions", names.len());
        for required in ["SUM", "IF", "XLOOKUP", "FILTER", "LET"] {
            assert!(names.contains(required), "missing {required}");
        }
        let functions = book.functions().unwrap();
        assert!(functions.len() >= 200);
        let sum = functions.iter().find(|f| f.name == "SUM").unwrap();
        assert!(!sum.signature.is_empty() && !sum.description.is_empty());
        assert!(sum.matches("sum") && sum.matches(&sum.category) && !sum.matches("not-a-function"));
    }

    #[test]
    fn go_to_and_insert_delete_lines() {
        let mut book = budget("lines.xlsx");
        assert_eq!(book.go_to("c3").unwrap(), Range { r0: 2, c0: 2, r1: 2, c1: 2 });
        assert_eq!(book.go_to("B2:D4").unwrap(), Range { r0: 1, c0: 1, r1: 3, c1: 3 });
        assert_eq!(book.go_to("Notes!A2").unwrap(), Range::cell(1, 0));
        assert_eq!(book.active_sheet(), 1);
        assert!(book.go_to("Nowhere!A1").is_err());
        assert!(book.go_to("hello").is_err());
        book.activate(0).unwrap();
        let before = column(&book, 0, 4);
        book.insert_lines(Range { r0: 1, c0: 0, r1: 2, c1: 0 }, true).unwrap();
        assert_eq!(column(&book, 0, 6), [before[0].as_str(), "", "", before[1].as_str(), before[2].as_str(), before[3].as_str()]);
        book.delete_lines(Range { r0: 1, c0: 0, r1: 2, c1: 0 }, true).unwrap();
        assert_eq!(column(&book, 0, 4), before);
        let costs = column(&book, 1, 4);
        book.insert_lines(Range::cell(0, 0), false).unwrap();
        assert_eq!(column(&book, 2, 4), costs);
        book.delete_lines(Range::cell(0, 0), false).unwrap();
        assert_eq!(column(&book, 1, 4), costs);
        book.undo().unwrap();
        assert_eq!(column(&book, 2, 4), costs);
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
