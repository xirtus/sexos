// SPDX-License-Identifier: GPL-3.0-only
//! Assuit: the SexOS word processor. `run assuit NAME.docx` opens a document in the
//! program's folder (docx, doc, odt, rtf, html, md or txt; a name that does not exist
//! yet starts an empty document saved there). With no name it opens `welcome.docx`,
//! writing it first if it is missing.
//!
//! The engine is WordCraft's (github.com/storytold/wordcraft, MIT OR Apache-2.0): its
//! document model, Word-compatible layout, file formats, undo and every command run
//! unchanged, and its renderer draws the pages. Its egui interface follows when egui
//! runs on sex-app; until then [`ui`] shows the caret's page.
//!
//! Keys are WordCraft's (Word's): arrows, Home/End, Ctrl+arrows, Shift to select,
//! Enter, Backspace, Delete, Ctrl+B/I/U, Ctrl+Z/Y, Ctrl+A. Ctrl+S saves (a .doc beside
//! itself as .docx), Ctrl+Shift+E saves as (the extension picks docx, odt, rtf, html, md,
//! txt or pdf), Ctrl+P writes a PDF beside the file, Ctrl+G goes to a page, Ctrl+= and
//! Ctrl+- zoom. A click places the caret; the wheel scrolls. Unsaved changes are kept
//! aside on close and after a quiet moment, and come back the next time the file opens.
#![cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod book;
#[cfg(target_os = "hermit")]
mod ui;

/// The first document, hand-written WordprocessingML (apps/assuit/make_welcome.py).
const WELCOME: &[u8] = include_bytes!("../assets/welcome.docx");
/// A small first-run picture for trying the insert command, using SexOS's own mark.
#[cfg(target_os = "hermit")]
const MARK: &[u8] = include_bytes!("../../../assets/brand/sexos-lips-badge-64.png");

#[cfg(target_os = "hermit")]
fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| {
        if std::fs::metadata("welcome.docx").is_err() {
            if let Err(e) = std::fs::write("welcome.docx", WELCOME) { eprintln!("Could not write welcome.docx: {e}"); }
        }
        if std::fs::metadata("assuitmark.png").is_err() {
            if let Err(e) = std::fs::write("assuitmark.png", MARK) { eprintln!("Could not write assuitmark.png: {e}"); }
        }
        "welcome.docx".to_string()
    });
    let recovered = book::Book::open_recovered(&name);
    let was_recovered = recovered.is_some();
    let opened = match recovered {
        Some(book) => Ok(book),
        None if std::fs::metadata(&name).is_ok() => book::Book::open(&name),
        None => Ok(book::Book::blank(&name)),
    };
    let mut book = match opened {
        Ok(book) => book,
        Err(message) => {
            println!("Assuit could not open {name}: {message}\n\nPut a document in this program's folder and run: assuit NAME.docx");
            std::process::exit(1);
        }
    };
    match book.restore_proofing_preference(book::Book::PROOFING_PREFERENCE) {
        Ok(saved) => sex_std::record(&format!("[assuit.proofing.preference] enabled={} source={}",
            book.proofing_marks() as u8, if saved.is_some() { "saved" } else { "default" })),
        Err(e) => sex_std::record(&format!("[assuit.proofing.preference.error] {e}")),
    }
    let (target, task) = sex_std::identity();
    sex_std::record(&format!("[assuit.open] target={target} task={task} name={name} pages={}", book.pages()));
    let window = sex_std::window();
    let ui = ui::Ui::new(book, name);
    let ui = if was_recovered { sex_std::record("[assuit.recovered]"); ui.recovered() } else { ui };
    ui.run(window);
    std::process::exit(0);
}

/// On the host the engine runs only under `cargo test`.
#[cfg(not(target_os = "hermit"))]
fn main() {}
