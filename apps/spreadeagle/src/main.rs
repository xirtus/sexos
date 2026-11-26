// SPDX-License-Identifier: GPL-3.0-only
//! Spreadeagle: the SexOS spreadsheet. `run spreadeagle NAME.xlsx` opens a workbook in the
//! program's folder (xlsx, xls, ods, csv or tsv; a name that does not exist yet starts an
//! empty workbook saved there). With no name it opens `welcome.xlsx`, writing it first
//! if it is missing.
//!
//! The engine is GridCraft's (github.com/storytold/gridcraft, MIT OR Apache-2.0): its
//! formulas, number formats, sheets, undo and xlsx reader and writer run unchanged. Its
//! egui interface follows when egui runs on sex-app; until then [`ui`] paints the grid.
//!
//! Keys: arrows, Tab, Page Up/Down and Ctrl+Home move; typing replaces the cell, Enter
//! edits it (arrows then move the caret), Enter or Tab commits, Esc cancels; Delete
//! clears; Ctrl+Z / Ctrl+Y undo and redo; Ctrl+Page Up/Down change sheet; Ctrl+S saves,
//! Ctrl+Shift+S saves as (the extension picks xlsx, ods, csv, tsv, html or pdf), Ctrl+P
//! writes a PDF beside the file, Ctrl+G goes to a cell, Ctrl+Shift+= and Ctrl+- insert and
//! delete rows or columns. Unsaved changes are kept aside on close and after a quiet
//! moment, and come back the next time the workbook opens.
//! The mouse picks cells and sheet tabs and the wheel scrolls.
#![cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod book;
mod filter_menu;
mod ods;
#[cfg(target_os = "hermit")]
mod ui;

/// The first workbook, hand-written SpreadsheetML (apps/spreadeagle/make_welcome.py).
#[cfg_attr(not(target_os = "hermit"), allow(dead_code))]
const WELCOME: &[u8] = include_bytes!("../assets/welcome.xlsx");

#[cfg(target_os = "hermit")]
fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| {
        if std::fs::metadata("welcome.xlsx").is_err() {
            if let Err(e) = std::fs::write("welcome.xlsx", WELCOME) { eprintln!("Could not write welcome.xlsx: {e}"); }
        }
        "welcome.xlsx".to_string()
    });
    let recovered = book::Book::open_recovered(&name);
    let was_recovered = recovered.is_some();
    let opened = if let Some(book) = recovered {
        Ok((book, Vec::new()))
    } else if std::fs::metadata(&name).is_ok() {
        book::Book::open(&name)
    } else {
        Ok((book::Book::blank(&name), Vec::new()))
    };
    let (book, warnings) = match opened {
        Ok(pair) => pair,
        Err(message) => {
            println!("Spreadeagle could not open {name}: {message}\n\nPut a workbook in this program's folder and run: spreadeagle NAME.xlsx");
            std::process::exit(1);
        }
    };
    let (target, task) = sex_std::identity();
    for w in &warnings { sex_std::record(&format!("[spreadeagle.warning] {w}")); }
    sex_std::record(&format!("[spreadeagle.open] target={target} task={task} name={name} sheets={}", book.sheets().join(",")));
    let window = sex_std::window();
    let ui = ui::Ui::new(book, name).with_import_warnings(warnings);
    let ui = if was_recovered { sex_std::record("[spreadeagle.recovered]"); ui.recovered() } else { ui };
    ui.run(window);
    std::process::exit(0);
}

/// On the host the engine runs only under `cargo test`.
#[cfg(not(target_os = "hermit"))]
fn main() {}
