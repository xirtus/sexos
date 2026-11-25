// SPDX-License-Identifier: GPL-3.0-only
//! The workbook behind the window: GridCraft's engine (storytold/gridcraft, MIT OR
//! Apache-2.0) holds the cells, recalculates, keeps undo, and reads and writes the
//! files. Nothing here draws or touches SexOS, so the host tests drive it directly.
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, RangeRef, Value};
use gridcraft_engine::model::{ErrorStyle, FilterCriterion};
use std::collections::BTreeSet;
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

/// GridCraft persists Excel chart style numbers but currently renders their
/// default colours alike. Give each supported style a native palette while
/// retaining any explicit series colours from the workbook.
fn styled_color(base: [u8; 4], style: u32) -> [u8; 4] {
    const TINTS: [[u8; 3]; 16] = [
        [0, 0, 0], [156, 110, 94], [46, 42, 43], [44, 109, 158],
        [45, 132, 124], [159, 85, 125], [176, 111, 43], [99, 91, 153],
        [52, 126, 72], [184, 73, 72], [82, 108, 162], [151, 91, 61],
        [81, 132, 145], [123, 104, 68], [122, 91, 139], [67, 67, 67],
    ];
    if !(2..=16).contains(&style) { return base; }
    let tint = TINTS[(style - 1) as usize];
    let strength = 40u16 + ((style - 2) % 4) as u16 * 10;
    let mix = |a: u8, b: u8| ((a as u16 * (100 - strength) + b as u16 * strength) / 100) as u8;
    [mix(base[0], tint[0]), mix(base[1], tint[1]), mix(base[2], tint[2]), base[3]]
}

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
    unreadable_ods_charts: bool,
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
        Book { session, path: path.to_string(), unreadable_ods_charts: false }
    }

    /// Opens an xlsx, xls, ods, csv, tsv or GridCraft json file.
    pub fn open(path: &str) -> Result<(Book, Vec<String>), String> {
        let mut session = Session::new();
        let r = session.execute_headless("file.open", json!({ "path": path })).map_err(|e| e.to_string())?;
        let mut warnings: Vec<String> = r["warnings"].as_array().into_iter().flatten().filter_map(Json::as_str).map(str::to_string).collect();
        let mut book = Book { session, path: path.to_string(), unreadable_ods_charts: false };
        if path.to_ascii_lowercase().ends_with(".ods") {
            match book.restore_ods_extras(path, &mut warnings) {
                Ok(n) => if n > 0 { warnings.retain(|w| !w.contains("formulas become their saved results")); },
                Err(e) => warnings.push(format!("ODS formulas were kept as values: {e}")),
            }
        }
        Ok((book, warnings))
    }

    /// Puts the file's formulas back over the values GridCraft imported, and its fonts and
    /// column widths, as part of the opened file: no undo step, not a change. Returns how
    /// many formulas.
    fn restore_ods_extras(&mut self, path: &str, warnings: &mut Vec<String>) -> Result<usize, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let extras = crate::ods::extras(&bytes)?;
        if extras == crate::ods::Extras::default() { return Ok(0); }
        self.unreadable_ods_charts = extras.chart_warnings.iter().any(|w| w.contains("could not") || w.contains("cannot preserve"));
        if !extras.charts.is_empty() {
            for warning in warnings.iter_mut() {
                if warning.starts_with("ODS data-only import:") {
                    *warning = "ODS import: charts were restored; images and advanced formatting still need review before replacing the original file.".into();
                }
            }
        }
        warnings.extend(extras.filter_warnings.iter().cloned());
        warnings.extend(extras.chart_warnings.iter().cloned());
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
        if !extras.validations.is_empty() || !extras.freezes.is_empty() || !extras.charts.is_empty() {
            let d = self.session.doc_mut().map_err(|e| e.to_string())?;
            let mut wb = (*d.wb).clone();
            for &(s, row, col) in &extras.freezes {
                wb.sheet_mut(s).ok_or("Freeze refers to a missing sheet")?.freeze = Some((row, col));
            }
            for (s, range, mut rule) in extras.validations {
                let formula = format!("={}", rule.f1.trim_start_matches('='));
                rule.f1 = crate::ods::remap_sheet_names(&formula, &extras.sheet_names, &imported_names)
                    .trim_start_matches('=').to_string();
                rule.ranges = vec![range];
                wb.sheet_mut(s).ok_or("Validation refers to a missing sheet")?.validations.push(rule);
            }
            for found in extras.charts {
                let mut chart = found.chart;
                chart.id = wb.next_object_id();
                let sh = wb.sheet_mut(found.sheet).ok_or("Chart refers to a missing sheet")?;
                let mut x = chart.anchor.dx;
                let mut col = 0;
                while col < 16_384 && x + 0.002 >= sh.col_width(col) * 0.75 {
                    x -= sh.col_width(col) * 0.75;
                    col += 1;
                }
                let mut y = chart.anchor.dy;
                let mut row = 0;
                while row < 1_048_576 && y + 0.002 >= sh.row_height(row) * 0.75 {
                    y -= sh.row_height(row) * 0.75;
                    row += 1;
                }
                chart.anchor.cell = CellRef::new(row, col);
                chart.anchor.dx = x.max(0.0);
                chart.anchor.dy = y.max(0.0);
                sh.charts.push(chart);
            }
            d.wb = std::sync::Arc::new(wb);
        }
        // Use GridCraft's own filter commands so hidden rows, SUBTOTAL results,
        // and the visible header buttons agree with the restored criteria.
        for filter in &extras.filters {
            let result = (|| -> Result<(), String> {
                self.activate(filter.sheet)?;
                self.run("data.filter", json!({ "range": filter.range.a1() }))?;
                for (offset, criterion) in &filter.criteria {
                    let column = column_name(filter.range.start.col + offset);
                    let params = match criterion {
                        FilterCriterion::Values { values, blanks } => json!({ "column": column, "values": values, "blanks": blanks }),
                        FilterCriterion::Custom { a, b, and } => {
                            if let Some(b) = b { json!({ "column": column, "custom": { "op": a.0, "value": a.1,
                                "op2": b.0, "value2": b.1, "and": and } }) }
                            else { json!({ "column": column, "custom": { "op": a.0, "value": a.1 } }) }
                        }
                        FilterCriterion::Top10 { bottom, percent, count } => json!({ "column": column,
                            "top": { "bottom": bottom, "percent": percent, "count": count } }),
                        _ => return Err("unsupported ODS filter condition".into()),
                    };
                    self.run("data.filterBy", params)?;
                }
                Ok(())
            })();
            if let Err(e) = result { warnings.push(format!("ODS AutoFilter on {} could not be restored: {e}",
                extras.sheet_names.get(filter.sheet).map(String::as_str).unwrap_or("sheet"))); }
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
        let check = self.run("data.validate", json!({ "cell": cell, "input": input }))?;
        if check.get("ok").and_then(Json::as_bool) == Some(false)
            && check.get("style").and_then(Json::as_str) == Some("Stop") {
            return Err(check.get("message").and_then(Json::as_str).unwrap_or("This value is not allowed in this cell.").into());
        }
        self.run("cell.set", json!({ "cell": cell, "input": input })).map(|_| ())
    }
    /// Restrict a selected range to a comma-separated list or a current-sheet
    /// source range (`=D2:D20`). The source is fixed for every selected cell.
    /// An empty entry clears validation from that range.
    pub fn list_validation(&mut self, range: Range, input: &str) -> Result<(), String> {
        let input = input.trim();
        if input.is_empty() {
            return self.run("data.validation", json!({ "range": range.a1(), "clear": true })).map(|_| ());
        }
        if let Some(source) = input.strip_prefix('=') {
            let source = gridcraft_engine::core::RangeRef::parse(source.trim())
                .filter(|r| r.count() >= 2 && r.count() <= 1000 && (r.height() == 1 || r.width() == 1))
                .ok_or("Enter a current-sheet row or column of 2–1,000 cells, such as =D2:D20.")?;
            let fixed = |cell: CellRef| {
                let a1 = cell.a1();
                let split = a1.find(|c: char| c.is_ascii_digit()).unwrap();
                format!("${}${}", &a1[..split], &a1[split..])
            };
            let formula = format!("={}:{}", fixed(source.start), fixed(source.end));
            return self.run("data.validation", json!({ "range": range.a1(), "type": "list", "formula1": formula,
                "errorStyle": "stop", "errorTitle": "Choose a listed value" })).map(|_| ());
        }
        let values: Vec<_> = input.split(',').map(str::trim).collect();
        if values.len() < 2 || values.len() > 30 || values.iter().any(|v| v.is_empty()) || input.chars().count() > 255 {
            return Err("Enter 2–30 choices separated by commas (255 characters maximum).".into());
        }
        self.run("data.validation", json!({ "range": range.a1(), "type": "list", "formula1": values.join(","),
            "errorStyle": "stop", "errorTitle": "Choose a listed value" })).map(|_| ())
    }
    /// The live choices for an in-cell dropdown, including lists imported from XLSX.
    /// A workbook may suppress its dropdown even while validation stays active.
    pub fn validation_choices(&self, row: u32, col: u32) -> Option<Vec<String>> {
        let d = self.session.doc().ok()?;
        let at = CellRef::new(row, col);
        let rule = d.wb.active()?.validations.iter().find(|rule| rule.ranges.iter().any(|r| r.contains(at)))?;
        if rule.kind != gridcraft_engine::model::ValidationKind::List || !rule.in_cell_dropdown { return None; }
        Some(gridcraft_engine::cmd::data::list_items(&d.wb, d.wb.active_sheet(), at, rule))
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
    /// Preview a bulk edit against existing stop-style rules before touching the
    /// live workbook. The engine's paste may replace rules, so the checks use
    /// the rules that were present at the destination before the edit.
    fn run_validated_on(&mut self, range: Range, id: &str, params: Json) -> Result<Json, String> {
        let original = self.session.doc().map_err(|e| e.to_string())?.clone();
        let sheet_index = original.wb.active_sheet();
        let has_stop = original.wb.active().is_some_and(|s| s.validations.iter().any(|v| v.show_error && v.error_style == ErrorStyle::Stop));
        // GridCraft skips plain-text cells past the sheet edge. A cheap
        // delimiter bound keeps ordinary pastes fast; near an edge, the probe
        // below checks the parsed shape before anything changes in the book.
        let may_cross_edge = params.get("text").and_then(Json::as_str).is_some_and(|text| {
            let rows_left = gridcraft_engine::core::MAX_ROWS.saturating_sub(range.r0) as usize;
            let cols_left = gridcraft_engine::core::MAX_COLS.saturating_sub(range.c0) as usize;
            text.bytes().filter(|b| matches!(b, b'\n' | b'\r')).count() >= rows_left
                || text.bytes().filter(|b| *b == b'\t').count() >= cols_left
        });
        if !has_stop && !may_cross_edge { return self.run_on(range, id, params); }
        let mut probe = Session::new();
        probe.add_document(original.clone());
        probe.clipboard = self.session.clipboard.clone();
        probe.execute_headless("selection.set", json!({ "range": range.a1() })).map_err(|e| e.to_string())?;
        let result = probe.execute_headless(id, params.clone()).map_err(|e| e.to_string())?;
        let area = if id == "edit.paste" {
            Self::paste_area(range, &result)?
        } else {
            gridcraft_engine::core::RangeRef::parse(&range.a1()).unwrap()
        };
        if !has_stop { return self.run_on(range, id, params); }
        let before = original.wb.active().ok_or("No active sheet")?;
        let after_doc = probe.doc().map_err(|e| e.to_string())?;
        let after = after_doc.wb.active().ok_or("No active sheet")?;
        let mut changed = BTreeSet::new();
        changed.extend(before.cells.iter_range(area).map(|(at, _)| at));
        changed.extend(after.cells.iter_range(area).map(|(at, _)| at));
        let mut checked = (*after_doc.wb).clone();
        checked.sheet_mut(sheet_index).ok_or("No active sheet")?.validations = before.validations.clone();
        for at in changed {
            let input = after.input_text(at);
            if before.input_text(at) == input { continue; }
            if let Some((rule, message)) = gridcraft_engine::cmd::data::check_validation(&checked, sheet_index, at, &input) {
                if rule.error_style == ErrorStyle::Stop { return Err(format!("{}: {message}", at.a1())); }
            }
        }
        self.run_on(range, id, params)
    }
    fn paste_area(at: Range, result: &Json) -> Result<gridcraft_engine::core::RangeRef, String> {
        if let Some(range) = result.get("pasted").and_then(Json::as_str).and_then(gridcraft_engine::core::RangeRef::parse) {
            return Ok(range);
        }
        let rows = result.get("rows").and_then(Json::as_u64).ok_or("Paste did not report its rows")?;
        let cols = result.get("cols").and_then(Json::as_u64).ok_or("Paste did not report its columns")?;
        if rows == 0 || cols == 0 { return Err("Clipboard has no cells".into()); }
        let start = CellRef::new(at.r0, at.c0);
        let end = start.offset(rows as i64 - 1, cols as i64 - 1).ok_or("Pasted text does not fit on the sheet")?;
        Ok(gridcraft_engine::core::RangeRef::new(start, end))
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
        let r = self.run_validated_on(Range::cell(row, col), "edit.paste", json!({}))?;
        Ok(Self::paste_area(Range::cell(row, col), &r)?.a1())
    }
    /// Pastes tab-separated text from another program, with the same stop-rule
    /// preflight as an in-workbook paste.
    pub fn paste_text(&mut self, row: u32, col: u32, text: &str) -> Result<String, String> {
        // Force the text path even when its bytes equal an earlier internal copy.
        let clipboard = self.session.clipboard.take();
        let result = self.run_validated_on(Range::cell(row, col), "edit.paste", json!({ "text": text }));
        self.session.clipboard = clipboard;
        let r = result?;
        Ok(Self::paste_area(Range::cell(row, col), &r)?.a1())
    }
    /// Empties every cell of `range` (values and formulas; formats stay).
    pub fn clear_range(&mut self, range: Range) -> Result<(), String> {
        self.run_on(range, "edit.clearContents", json!({})).map(|_| ())
    }
    /// Copies the top row of `range` down through it (Ctrl+D), or its left column right (Ctrl+R).
    pub fn fill(&mut self, range: Range, down: bool) -> Result<(), String> {
        self.run_validated_on(range, if down { "edit.fillDown" } else { "edit.fillRight" }, json!({})).map(|_| ())
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
    /// Re-measure selected columns from their displayed cell text.
    pub fn autofit_cols(&mut self, first: u32, last: u32) -> Result<(), String> {
        if last < first || last - first >= 256 { return Err("Select at most 256 columns to fit.".into()); }
        self.run("home.autofitColumnWidth", json!({ "cols": format!("{}:{}", column_name(first), column_name(last)) })).map(|_| ())
    }
    /// Clear custom row heights and fit their current content in one undo step.
    pub fn autofit_rows(&mut self, first: u32, last: u32) -> Result<(), String> {
        if last < first || last - first >= 10_000 { return Err("Select at most 10,000 rows to fit.".into()); }
        // GridCraft's AutoFit command reveals hidden rows. Do not silently
        // undo an active AutoFilter while measuring a row.
        if self.filter_range().is_some() { return Err("Turn off AutoFilter before fitting rows.".into()); }
        self.run("home.autofitRowHeight", json!({ "rows": format!("{}:{}", first + 1, last + 1) })).map(|_| ())
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

    /// Search the full AutoFilter column, including sparse cells and array spills,
    /// instead of only the menu's first 1,000 values or 10,000 rows. Return a
    /// bounded set of distinct displayed matches so the menu cannot grow without
    /// limit; the scan follows sparse stored cells and dynamic-array spills.
    pub fn filter_search_choices(&self, col: u32, query: &str) -> Vec<String> {
        const MAX_MATCHES: usize = 128;
        let Some(range) = self.filter_range() else { return Vec::new() };
        let (top, left, bottom, right) = range.bounds();
        if query.is_empty() || !(left..=right).contains(&col) || bottom <= top { return Vec::new() }
        let needle = query.to_lowercase();
        let Ok(d) = self.session.doc() else { return Vec::new() };
        let Some(sheet) = d.wb.active() else { return Vec::new() };
        let mut matches = BTreeSet::new();
        let wants_blank = "(blank)".contains(&needle);
        // A full column can contain over a million occupied rows. Track row
        // coverage in one bit per row instead of allocating a tree node for
        // every value whenever the query also matches "(blank)".
        let row_count = (bottom - top) as usize;
        let mut occupied_rows = wants_blank.then(|| vec![0u8; row_count.div_ceil(8)]);
        let mut occupied_count = 0usize;
        let cells = sheet.cells.iter_range(RangeRef::new(CellRef::new(top + 1, col), CellRef::new(bottom, col)))
            .map(|(at, _)| at).chain(sheet.spill.keys().copied().filter(|at| at.col == col && at.row > top && at.row <= bottom));
        for at in cells {
            let value = gridcraft_engine::display::cell_text(&d.wb, sheet, at);
            if let Some(rows) = &mut occupied_rows {
                if !value.is_empty() {
                    let index = (at.row - top - 1) as usize;
                    let (byte, mask) = (index / 8, 1u8 << (index % 8));
                    if rows[byte] & mask == 0 { rows[byte] |= mask; occupied_count += 1; }
                }
            }
            let label = if value.is_empty() { "(blank)" } else { value.as_str() };
            if label.to_lowercase().contains(&needle)
                && (value.is_empty() || matches.len() < MAX_MATCHES) { matches.insert(value); }
            if matches.len() >= MAX_MATCHES && !wants_blank { break; }
        }
        if wants_blank && occupied_count < row_count { matches.insert(String::new()); }
        matches.into_iter().take(MAX_MATCHES).collect()
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
            let blanks = values.iter().any(String::is_empty);
            let nonblank: Vec<&str> = values.iter().filter(|value| !value.is_empty()).map(String::as_str).collect();
            json!({ "column": column_name(col), "values": nonblank, "blanks": blanks })
        };
        self.run("data.filterBy", params).map(|_| ())
    }

    /// Apply one or two AutoFilter comparisons to a column. GridCraft evaluates
    /// the criteria against cells, including numeric values and formula results.
    pub fn filter_by_condition(&mut self, at: Range, col: u32, input: &str) -> Result<(), String> {
        const HELP: &str = "Enter above/below average, top/bottom N or N%, =, <>, text or numeric comparison";
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
        let mut words = input.split_whitespace();
        let direction = words.next().unwrap_or("").to_ascii_lowercase();
        if matches!(direction.as_str(), "top" | "bottom") {
            let amount = words.next().ok_or(HELP)?;
            if words.next().is_some() { return Err(HELP.into()); }
            let percent = amount.ends_with('%');
            let count = amount.trim_end_matches('%').parse::<u32>().map_err(|_| HELP)?;
            if count == 0 || count > if percent { 100 } else { 1_000_000 } { return Err(HELP.into()); }
            if self.filter_range().is_none() { self.toggle_filter(at)?; }
            self.run("data.filterBy", json!({ "column": column_name(col),
                "top": { "count": count, "bottom": direction == "bottom", "percent": percent } }))?;
            return Ok(());
        }
        if matches!(direction.as_str(), "above" | "below") {
            if !words.next().is_some_and(|word| word.eq_ignore_ascii_case("average")) || words.next().is_some() {
                return Err(HELP.into());
            }
            if self.filter_range().is_none() { self.toggle_filter(at)?; }
            self.run("data.filterBy", json!({ "column": column_name(col), "aboveAverage": direction == "above" }))?;
            return Ok(());
        }
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
        if let Some(value) = input.strip_prefix("style ") {
            let style = value.parse::<u32>().map_err(|_| "Choose chart style 1–16")?;
            if !(1..=16).contains(&style) { return Err("Choose chart style 1–16".into()); }
            self.run("chart.set", json!({ "chart": id, "style": style }))?;
            return Ok(format!("Chart style: {style}"));
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
        Err("Enter remove 2, data A1:B4, switch, type, style, title, legend, table, hole or angle".into())
    }

    /// Chart `index` of the active sheet laid out at `width` x `height` pixels: shapes
    /// drawn as 0x00RRGGBB pixels on white, and its text left for the caller's font.
    pub fn chart_picture(&self, index: usize, width: u32, height: u32, measure: &dyn gridcraft_chart::Measure) -> Option<ChartPicture> {
        let d = self.session.doc().ok()?;
        let si = d.wb.active_sheet();
        let chart = d.wb.sheets.get(si)?.charts.get(index)?;
        let mut data = gridcraft_chart::resolve(&d.wb, si, chart);
        if chart.style > 1 {
            for color in &mut data.palette { *color = styled_color(*color, chart.style); }
            for (series, drawn) in chart.series.iter().zip(&mut data.series) {
                if series.color.is_none() { drawn.color = styled_color(drawn.color, chart.style); }
            }
        }
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
        if lower.ends_with(".ods") {
            if self.unreadable_ods_charts { return Err("ODS contains charts Spreadeagle could not read; keep the original file and save a reviewed copy as XLSX".into()); }
            return crate::ods::write(&d.wb);
        }
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
        assert_eq!(book.filter_range().unwrap().bounds().2, 1_102);
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
    fn filter_blank_choice_includes_empty_cells_and_empty_formula_results() {
        let mut book = Book::blank(&temp("filter-blanks.xlsx"));
        book.set(0, 0, "Item").unwrap();
        book.set(1, 0, "Food").unwrap();
        book.set(3, 0, "=\"\"").unwrap();
        book.set(4, 0, "Rent").unwrap();
        book.toggle_filter(Range { r0: 0, c0: 0, r1: 4, c1: 0 }).unwrap();
        assert_eq!(book.filter_choices(0), ["", "Food", "Rent"]);
        let visible = |book: &Book| (1..=4).map(|row| book.row_height(row) > 0).collect::<Vec<_>>();
        book.filter_by_values(Range::cell(0, 0), 0, &[String::new()]).unwrap();
        assert_eq!(visible(&book), [false, true, true, false]);
        assert_eq!(book.filter_state(0), FilterState::Values { values: vec![], blanks: true });
        let path = book.path.clone();
        book.save().unwrap();
        let reopened = Book::open(&path).unwrap().0;
        assert_eq!(visible(&reopened), [false, true, true, false]);
        assert_eq!(reopened.filter_state(0), FilterState::Values { values: vec![], blanks: true });
        book.filter_by_values(Range::cell(0, 0), 0, &[String::new(), "Food".into()]).unwrap();
        assert_eq!(visible(&book), [true, true, true, false]);
        assert_eq!(book.filter_state(0), FilterState::Values { values: vec!["Food".into()], blanks: true });
        book.filter_by_values(Range::cell(0, 0), 0, &[]).unwrap();
        assert_eq!(visible(&book), [true, true, true, true]);
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
    fn top_bottom_items_and_percent_include_ties_and_reopen() {
        let mut book = Book::blank(&temp("filter-top-bottom.xlsx"));
        book.set(0, 0, "Score").unwrap();
        for (row, value) in ["5", "1", "9", "3", "9", "7", "x"].iter().enumerate() {
            book.set(row as u32 + 1, 0, value).unwrap();
        }
        let at = Range::cell(0, 0);
        for bad in ["top 0", "bottom 101%", "top 2 extra", "top many"] {
            assert!(book.filter_by_condition(at, 0, bad).is_err(), "{bad}");
        }
        assert!(book.filter_range().is_none());
        let visible = |book: &Book| (1..=7).map(|row| book.row_height(row) > 0).collect::<Vec<_>>();
        book.filter_by_condition(at, 0, "top 2").unwrap();
        assert_eq!(visible(&book), [false, false, true, false, true, false, false]);
        book.filter_by_condition(at, 0, "bottom 2").unwrap();
        assert_eq!(visible(&book), [false, true, false, true, false, false, false]);
        book.filter_by_condition(at, 0, "top 50%").unwrap();
        assert_eq!(visible(&book), [false, false, true, false, true, true, false]);
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(visible(&Book::open(&path).unwrap().0), visible(&book));
    }

    #[test]
    fn above_below_average_filter_numeric_and_formula_values() {
        let mut book = Book::blank(&temp("filter-average.xlsx"));
        book.set(0, 0, "Score").unwrap();
        for (row, value) in [(1, "1"), (2, "2"), (3, "3"), (4, "=AVERAGE(A2:A4)"), (5, "x")] {
            book.set(row, 0, value).unwrap();
        }
        assert_eq!(book.shown(4, 0).text, "2");
        let at = Range::cell(0, 0);
        for bad in ["above", "below median", "above average and > 2"] {
            assert!(book.filter_by_condition(at, 0, bad).is_err(), "{bad}");
        }
        assert!(book.filter_range().is_none());
        let visible = |book: &Book| (1..=5).map(|row| book.row_height(row) > 0).collect::<Vec<_>>();
        book.filter_by_condition(at, 0, "above average").unwrap();
        assert_eq!(visible(&book), [false, false, true, false, false]);
        book.filter_by_condition(at, 0, "below average").unwrap();
        assert_eq!(visible(&book), [true, false, false, false, false]);
        let path = book.path.clone();
        book.save().unwrap();
        assert_eq!(visible(&Book::open(&path).unwrap().0), visible(&book));
    }

    #[test]
    fn selected_columns_and_custom_rows_autofit_undo_and_reopen() {
        let mut book = Book::blank(&temp("autofit.xlsx"));
        book.set(0, 0, "A longer heading").unwrap();
        book.set_col_width(0, 400).unwrap();
        book.set_row_height(0, 100).unwrap();
        assert_eq!((book.col_width(0), book.row_height(0)), (400, 100));
        assert!(book.autofit_cols(2, 1).is_err());
        book.autofit_cols(0, 0).unwrap();
        let fitted_col = book.col_width(0);
        assert!(fitted_col > 64 && fitted_col < 400, "fitted width {fitted_col}");
        book.autofit_rows(0, 0).unwrap();
        let fitted_row = book.row_height(0);
        assert!(fitted_row < 100, "fitted height {fitted_row}");
        book.undo().unwrap();
        assert_eq!(book.row_height(0), 100);
        book.redo().unwrap();
        assert_eq!(book.row_height(0), fitted_row);
        book.toggle_filter(Range::cell(0, 0)).unwrap();
        assert!(book.autofit_rows(0, 0).is_err(), "fitting must not reveal filtered rows");
        assert_eq!(book.row_height(0), fitted_row);
        book.toggle_filter(Range::cell(0, 0)).unwrap();
        let path = book.path.clone();
        book.save().unwrap();
        let reopened = Book::open(&path).unwrap().0;
        assert_eq!((reopened.col_width(0), reopened.row_height(0)), (fitted_col, fitted_row));
    }

    #[test]
    fn literal_list_validation_rejects_bad_entries_and_round_trips() {
        let mut book = Book::blank(&temp("validation-list.xlsx"));
        let area = Range { r0: 1, c0: 1, r1: 2, c1: 1 };
        for bad in ["Food", "Food,,Fun", "Food,Fun,"] {
            assert!(book.list_validation(area, bad).is_err(), "{bad}");
        }
        book.list_validation(area, "Food, Fun").unwrap();
        book.undo().unwrap();
        assert!(book.session.doc().unwrap().wb.active().unwrap().validations.is_empty());
        book.redo().unwrap();
        assert!(book.set(1, 1, "Other").is_err());
        assert!(book.input(1, 1).is_empty(), "invalid input must not replace a cell");
        book.set(1, 1, "Food").unwrap();
        book.set(2, 1, "fun").unwrap();
        let path = book.path.clone();
        book.save().unwrap();
        let mut reopened = Book::open(&path).unwrap().0;
        assert!(reopened.set(2, 1, "Other").is_err());
        assert_eq!(reopened.input(2, 1).to_ascii_lowercase(), "fun");
        reopened.list_validation(Range::cell(1, 1), "").unwrap();
        reopened.set(1, 1, "Other").unwrap();
        assert!(reopened.set(2, 1, "Other").is_err(), "clearing B2 must keep B3's rule");
    }

    #[test]
    fn range_list_validation_tracks_source_and_round_trips() {
        let mut book = Book::blank(&temp("validation-range.xlsx"));
        for (row, value) in [(1, "Food"), (2, "Fun"), (3, "Rent")] {
            book.set(row, 3, value).unwrap();
        }
        let area = Range { r0: 1, c0: 1, r1: 2, c1: 1 };
        for bad in ["=D2", "=D2:C4", "=D2:E4", "=D1:D1002", "=missing"] {
            assert!(book.list_validation(area, bad).is_err(), "{bad}");
        }
        book.list_validation(area, "=D2:D4").unwrap();
        assert_eq!(book.session.doc().unwrap().wb.active().unwrap().validations[0].f1, "$D$2:$D$4");
        book.set(1, 1, "Food").unwrap();
        book.set(2, 1, "Rent").unwrap();
        assert!(book.set(1, 1, "Other").is_err());
        book.set(1, 3, "Meals").unwrap();
        assert!(book.set(1, 1, "Food").is_err(), "source edits must update choices");
        book.set(1, 1, "Meals").unwrap();
        book.set(2, 1, "Fun").unwrap();
        let path = book.path.clone();
        book.save().unwrap();
        let mut reopened = Book::open(&path).unwrap().0;
        assert_eq!(reopened.input(1, 1), "Meals");
        assert!(reopened.set(2, 1, "Food").is_err());
        reopened.set(2, 1, "Meals").unwrap();
    }

    #[test]
    fn ods_list_validation_keeps_blank_cells_and_live_sources() {
        let mut book = Book::blank(&temp("validation-ods.xlsx"));
        book.set(1, 3, "Food").unwrap();
        book.set(2, 3, "Fun").unwrap();
        book.list_validation(Range { r0: 1, c0: 0, r1: 3, c1: 0 }, "Food,Fun").unwrap();
        book.list_validation(Range { r0: 1, c0: 1, r1: 3, c1: 1 }, "=D2:D3").unwrap();
        let (path, _) = book.save_as(&temp("validation-ods.ods")).unwrap();
        let xml = {
            let bytes = std::fs::read(&path).unwrap();
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            let mut xml = String::new();
            std::io::Read::read_to_string(&mut zip.by_name("content.xml").unwrap(), &mut xml).unwrap();
            xml
        };
        assert!(xml.contains("table:content-validations"));
        assert!(xml.contains("cell-content-is-in-list"));
        assert!(xml.contains("table:content-validation-name"));
        let mut reopened = Book::open(&path).unwrap().0;
        assert_eq!(reopened.validation_choices(3, 0), Some(vec!["Food".into(), "Fun".into()]));
        assert_eq!(reopened.validation_choices(3, 1), Some(vec!["Food".into(), "Fun".into()]));
        assert!(reopened.set(3, 0, "Other").is_err());
        reopened.set(1, 3, "Rent").unwrap();
        assert_eq!(reopened.validation_choices(3, 1), Some(vec!["Rent".into(), "Fun".into()]));
        assert!(reopened.set(3, 1, "Food").is_err());
    }

    #[test]
    fn ods_validation_source_follows_renamed_sheet() {
        use std::io::{Read, Write};
        let path = temp("validation-renamed-sheet.ods");
        let mut book = Book::blank(&path);
        book.run("home.insertSheet", json!({ "name": "Source" })).unwrap();
        book.activate(1).unwrap();
        book.set(1, 3, "Food").unwrap();
        book.set(2, 3, "Fun").unwrap();
        book.activate(0).unwrap();
        book.list_validation(Range::cell(1, 0), "Food,Fun").unwrap();
        {
            let d = book.session.doc_mut().unwrap();
            let mut wb = (*d.wb).clone();
            wb.sheet_mut(0).unwrap().validations[0].f1 = "'Source'!$D$2:$D$3".into();
            d.wb = std::sync::Arc::new(wb);
        }
        assert_eq!(book.validation_choices(1, 0), Some(vec!["Food".into(), "Fun".into()]));
        book.save().unwrap();

        // ODF permits this sheet name; GridCraft renames it on import to obey its
        // workbook name rules. Change both the table name and its list source.
        let bytes = std::fs::read(&path).unwrap();
        let mut source = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut target = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for i in 0..source.len() {
            let mut file = source.by_index(i).unwrap();
            let name = file.name().to_string();
            let method = file.compression();
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            if name == "content.xml" {
                data = String::from_utf8(data).unwrap().replace("Source", "Q1:Q2").into_bytes();
            }
            target.start_file(name, zip::write::SimpleFileOptions::default().compression_method(method)).unwrap();
            target.write_all(&data).unwrap();
        }
        std::fs::write(&path, target.finish().unwrap().into_inner()).unwrap();

        let (mut reopened, _) = Book::open(&path).unwrap();
        assert_ne!(reopened.sheets()[1], "Q1:Q2", "GridCraft must normalize the ODF-only sheet name");
        assert_eq!(reopened.validation_choices(1, 0), Some(vec!["Food".into(), "Fun".into()]));
        assert!(reopened.set(1, 0, "Other").is_err());
    }

    #[test]
    fn validation_dropdown_reads_literal_and_live_choices() {
        let mut book = Book::blank(&temp("validation-dropdown.xlsx"));
        assert_eq!(book.validation_choices(1, 1), None);
        book.list_validation(Range::cell(1, 1), "Food,Fun").unwrap();
        assert_eq!(book.validation_choices(1, 1), Some(vec!["Food".into(), "Fun".into()]));
        book.set(1, 3, "Meals").unwrap();
        book.set(2, 3, "Fun").unwrap();
        book.list_validation(Range::cell(1, 2), "=D2:D3").unwrap();
        assert_eq!(book.validation_choices(1, 2), Some(vec!["Meals".into(), "Fun".into()]));
        book.set(1, 3, "Rent").unwrap();
        assert_eq!(book.validation_choices(1, 2), Some(vec!["Rent".into(), "Fun".into()]));
        let path = book.path.clone();
        book.save().unwrap();
        let reopened = Book::open(&path).unwrap().0;
        assert_eq!(reopened.validation_choices(1, 1), Some(vec!["Food".into(), "Fun".into()]));
        assert_eq!(reopened.validation_choices(1, 2), Some(vec!["Rent".into(), "Fun".into()]));
    }

    #[test]
    fn paste_and_fill_reject_unlisted_values_without_partial_edits() {
        let mut book = Book::blank(&temp("validation-bulk.xlsx"));
        book.list_validation(Range { r0: 1, c0: 0, r1: 2, c1: 0 }, "Food,Fun").unwrap();
        book.set(1, 0, "Food").unwrap();
        book.set(2, 0, "Fun").unwrap();
        book.set(1, 1, "Other").unwrap();
        book.copy(Range::cell(1, 1), false).unwrap();
        let history = book.session.doc().unwrap().undo.len();
        assert!(book.paste(1, 0).is_err());
        assert_eq!(book.input(1, 0), "Food");
        assert_eq!(book.session.doc().unwrap().undo.len(), history, "rejected paste must leave undo history intact");
        book.set(1, 1, "Fun").unwrap();
        book.set(2, 1, "Other").unwrap();
        book.copy(Range { r0: 1, c0: 1, r1: 2, c1: 1 }, false).unwrap();
        assert!(book.paste(1, 0).is_err(), "one invalid value rejects the whole block");
        assert_eq!((book.input(1, 0), book.input(2, 0)), ("Food".into(), "Fun".into()));
        book.copy(Range::cell(1, 1), false).unwrap();
        book.paste(1, 0).unwrap();
        assert_eq!(book.input(1, 0), "Fun");
        book.undo().unwrap();
        assert_eq!(book.input(1, 0), "Food");

        book.set(1, 1, "Other").unwrap();
        book.set(2, 1, "Fun").unwrap();
        let area = Range { r0: 1, c0: 1, r1: 2, c1: 1 };
        book.list_validation(Range::cell(2, 1), "Food,Fun").unwrap();
        assert!(book.fill(area, true).is_err());
        assert_eq!(book.input(2, 1), "Fun");
        book.set(1, 1, "Food").unwrap();
        book.fill(area, true).unwrap();
        assert_eq!(book.input(2, 1), "Food");
    }

    #[test]
    fn external_text_paste_obeys_stop_rules_for_every_cell() {
        let mut book = Book::blank(&temp("validation-text-paste.xlsx"));
        let area = Range { r0: 1, c0: 0, r1: 2, c1: 0 };
        book.list_validation(area, "Food,Fun").unwrap();
        book.set(1, 0, "Fun").unwrap();
        book.set(2, 0, "Food").unwrap();
        let before = book.session.doc().unwrap().undo.len();
        assert!(book.paste_text(1, 0, "Food\nOther").is_err());
        assert_eq!((book.input(1, 0), book.input(2, 0)), ("Fun".into(), "Food".into()));
        assert_eq!(book.session.doc().unwrap().undo.len(), before);
        assert_eq!(book.paste_text(1, 0, "Food\nFun").unwrap(), "A2:A3");
        assert_eq!((book.input(1, 0), book.input(2, 0)), ("Food".into(), "Fun".into()));
        book.copy(Range::cell(1, 0), false).unwrap();
        book.paste_text(2, 0, "Food\n").unwrap();
        assert_eq!(book.input(2, 0), "Food");
        assert!(book.validation_choices(2, 0).is_some(), "plain paste keeps destination validation");
    }

    #[test]
    fn external_text_paste_refuses_sheet_edge_without_partial_edit() {
        let mut book = Book::blank(&temp("text-paste-edge.xlsx"));
        let last_row = gridcraft_engine::core::MAX_ROWS - 1;
        let last_col = gridcraft_engine::core::MAX_COLS - 1;
        book.set(last_row, 0, "old row").unwrap();
        book.set(0, last_col, "old col").unwrap();
        let history = book.session.doc().unwrap().undo.len();
        assert!(book.paste_text(last_row, 0, "new row\nextra").is_err());
        assert_eq!(book.input(last_row, 0), "old row");
        assert_eq!(book.session.doc().unwrap().undo.len(), history);
        assert!(book.paste_text(0, last_col, "new col\textra").is_err());
        assert_eq!(book.input(0, last_col), "old col");
        assert_eq!(book.session.doc().unwrap().undo.len(), history);
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
    fn ods_frozen_panes_round_trip() {
        let path = temp("freeze.ods");
        let mut book = Book::blank(&path);
        book.freeze(Some((3, 2))).unwrap();
        book.save().unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(crate::ods::extras(&raw).unwrap().freezes, vec![(0, 3, 2)]);
        assert_eq!(Book::open(&path).unwrap().0.frozen(), Some((3, 2)));
        book.freeze(None).unwrap();
        book.save().unwrap();
        assert_eq!(Book::open(&path).unwrap().0.frozen(), None);
    }

    #[test]
    fn ods_autofilter_keeps_values_and_ranked_rows() {
        let path = temp("filter-roundtrip.ods");
        let mut book = Book::blank(&path);
        for (row, region, score) in [(1, "East", "9"), (2, "West", "3"), (3, "East", "6"), (4, "", "1")] {
            book.set(row, 0, region).unwrap();
            book.set(row, 1, score).unwrap();
        }
        book.set(0, 0, "Region").unwrap(); book.set(0, 1, "Score").unwrap();
        let area = Range { r0: 0, c0: 0, r1: 4, c1: 1 };
        book.toggle_filter(area).unwrap();
        book.filter_by_values(area, 0, &["East".into(), String::new()]).unwrap();
        book.filter_by_condition(area, 1, "top 2").unwrap();
        book.save().unwrap();
        let raw = std::fs::read(&path).unwrap();
        let filters = crate::ods::extras(&raw).unwrap().filters;
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].range.a1(), "A1:B5");
        assert_eq!(filters[0].criteria.len(), 2);
        let (again, warnings) = Book::open(&path).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("AutoFilter")), "{warnings:?}");
        assert_eq!(again.filter_range(), Some(area));
        assert_eq!(again.filter_state(0), FilterState::Values { values: vec!["East".into()], blanks: true });
        let sheet = again.session.doc().unwrap().wb.active().unwrap();
        assert!(sheet.is_row_filtered(2)); // West is excluded by the value choice.
        assert!(!sheet.is_row_filtered(1));
    }

    #[test]
    fn ods_autofilter_keeps_two_numeric_conditions() {
        let path = temp("filter-condition-ods.ods");
        let mut book = Book::blank(&path);
        book.set(0, 0, "Score").unwrap();
        for row in 1..=5 { book.set(row, 0, &row.to_string()).unwrap(); }
        let area = Range { r0: 0, c0: 0, r1: 5, c1: 0 };
        book.toggle_filter(area).unwrap();
        book.filter_by_condition(area, 0, ">= 2 and <= 4").unwrap();
        book.save().unwrap();
        let (again, warnings) = Book::open(&path).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("AutoFilter")), "{warnings:?}");
        let sheet = again.session.doc().unwrap().wb.active().unwrap();
        assert_eq!(sheet.autofilter.as_ref().unwrap().range.a1(), "A1:A6");
        assert_eq!(sheet.autofilter.as_ref().unwrap().criteria.len(), 1);
        assert!(sheet.is_row_filtered(1));
        assert!(!sheet.is_row_filtered(3));
        assert!(sheet.is_row_filtered(5));
    }

    #[test]
    fn ods_refuses_filter_it_cannot_preserve() {
        let path = temp("unsupported-filter.ods");
        let mut book = Book::blank(&path);
        book.set(0, 0, "Score").unwrap();
        book.set(1, 0, "1").unwrap();
        book.set(2, 0, "2").unwrap();
        let area = Range { r0: 0, c0: 0, r1: 2, c1: 0 };
        book.toggle_filter(area).unwrap();
        book.filter_by_condition(area, 0, "above average").unwrap();
        assert!(book.save().unwrap_err().contains("save as XLSX"));
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
    fn chart_style_changes_native_pixels_and_survives_xlsx() {
        let mut book = budget("chart-style.xlsx");
        book.add_chart(Range { r0: 0, c0: 0, r1: 3, c1: 1 }, 0, 3).unwrap();
        let id = book.chart_at_anchor(0, 3).unwrap().id;
        book.edit_chart(id, "type doughnut").unwrap();
        let picture = |b: &Book| b.chart_picture(0, 480, 288, &gridcraft_chart::ApproxMeasure).unwrap().pixels;
        let default_pixels = picture(&book);
        for bad in ["style 0", "style 17", "style rose"] {
            assert!(book.edit_chart(id, bad).is_err(), "{bad}");
        }
        book.edit_chart(id, "style 5").unwrap();
        let styled_pixels = picture(&book);
        assert_ne!(styled_pixels, default_pixels, "style must affect the chart actually shown");
        assert_eq!(book.session.doc().unwrap().wb.active().unwrap().charts[0].style, 5);
        book.undo().unwrap();
        assert_eq!(picture(&book), default_pixels);
        book.redo().unwrap();
        assert_eq!(picture(&book), styled_pixels);
        let path = book.path.clone();
        book.save().unwrap();
        let (reopened, _) = Book::open(&path).unwrap();
        assert_eq!(reopened.session.doc().unwrap().wb.active().unwrap().charts[0].style, 5);
        assert_eq!(picture(&reopened), styled_pixels);
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
    fn filter_partial_search_reaches_sparse_rows_beyond_preview_and_reopens() {
        let path = temp("filter-late-search.xlsx");
        let mut book = Book::blank(&path);
        book.set(0, 0, "Code").unwrap();
        for row in 1..=1_001 { book.set(row, 0, &format!("Item {row:04}")).unwrap(); }
        book.set(10_050, 0, "Late amber").unwrap();
        book.set(10_051, 0, "Late azure").unwrap();
        let area = Range { r0: 0, c0: 0, r1: 10_051, c1: 0 };
        book.toggle_filter(area).unwrap();
        assert_eq!(book.filter_choices(0).len(), 1_000);
        assert_eq!(book.filter_search_choices(0, "late a"), ["Late amber", "Late azure"]);
        book.filter_by_values(area, 0, &["Late amber".into(), "Late azure".into()]).unwrap();
        book.save().unwrap();
        let (again, warnings) = Book::open(&path).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.filter_state(0), FilterState::Values { values: vec!["Late amber".into(), "Late azure".into()], blanks: false });
    }

    #[test]
    fn filter_search_includes_spill_results_and_sparse_blanks() {
        let mut book = Book::blank(&temp("filter-spill.xlsx"));
        book.set(0, 0, "Code").unwrap();
        book.set(0, 1, "Generated").unwrap();
        book.set(1, 0, "filled").unwrap();
        book.set(3, 0, "other").unwrap();
        book.set(1, 1, "=SEQUENCE(3)").unwrap();
        book.toggle_filter(Range { r0: 0, c0: 0, r1: 3, c1: 1 }).unwrap();
        assert_eq!(book.filter_search_choices(0, "blank"), [""]);
        assert_eq!(book.filter_search_choices(1, "3"), ["3"]);
    }

    #[test]
    fn filter_blank_search_handles_dense_rows_without_false_blanks() {
        let path = temp("filter-dense-blank.xlsx");
        std::fs::write(&path, include_bytes!("../assets/filter-overflow.xlsx")).unwrap();
        let mut book = Book::open(&path).unwrap().0;
        book.toggle_filter(Range::cell(0, 0)).unwrap();
        // Every data row is occupied, including those beyond the menu preview.
        assert!(!book.filter_search_choices(0, "a").contains(&String::new()));
        book.set(500, 0, "").unwrap();
        assert!(book.filter_search_choices(0, "a").contains(&String::new()));

        let mut many = Book::blank(&temp("filter-many-blank-matches.xlsx"));
        many.set(0, 0, "Code").unwrap();
        for row in 1..=300 { if row != 150 { many.set(row, 0, &format!("alpha {row}")).unwrap(); } }
        many.toggle_filter(Range { r0: 0, c0: 0, r1: 300, c1: 0 }).unwrap();
        let matches = many.filter_search_choices(0, "a");
        assert_eq!(matches.len(), 128);
        assert!(matches.contains(&String::new()));
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
