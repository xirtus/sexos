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
        let warnings = r["warnings"].as_array().into_iter().flatten().filter_map(Json::as_str).map(str::to_string).collect();
        Ok((Book { session, path: path.to_string() }, warnings))
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
    pub fn undo(&mut self) -> Result<(), String> { self.run("edit.undo", json!({})).map(|_| ()) }
    pub fn redo(&mut self) -> Result<(), String> { self.run("edit.redo", json!({})).map(|_| ()) }

    /// Column width and row height in pixels at 100%.
    pub fn col_width(&self, col: u32) -> u32 {
        self.session.doc().ok().and_then(|d| d.wb.active().map(|s| s.col_width(col))).map_or(64, |w| w.round() as u32)
    }
    pub fn row_height(&self, row: u32) -> u32 {
        self.session.doc().ok().and_then(|d| d.wb.active().map(|s| s.row_height(row))).map_or(20, |h| h.round() as u32)
    }

    /// The workbook as the bytes of its file, in the format `path`'s extension names
    /// (xlsx when it names none the engine writes).
    pub fn file_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        let d = self.session.doc().map_err(|e| e.to_string())?;
        gridcraft_engine::io::save_file_bytes(&d.wb, path).map_err(|e| e.to_string())
    }

    /// Writes the workbook to its file. std::fs::write, not the engine's
    /// write-then-rename, so a save works on every SexOS volume. Returns the size.
    pub fn save(&mut self) -> Result<usize, String> {
        let path = self.path.clone();
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
}
