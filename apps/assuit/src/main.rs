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
//! Enter, Backspace, Delete, Ctrl+B/I/U, Ctrl+Z/Y, Ctrl+A. Ctrl+S saves, Ctrl+= and
//! Ctrl+- zoom. A click places the caret; the wheel scrolls.
#![cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod book;
#[cfg(target_os = "hermit")]
mod ui;

/// The first document, hand-written WordprocessingML (apps/assuit/make_welcome.py).
const WELCOME: &[u8] = include_bytes!("../assets/welcome.docx");

#[cfg(target_os = "hermit")]
fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| {
        if std::fs::metadata("welcome.docx").is_err() {
            if let Err(e) = std::fs::write("welcome.docx", WELCOME) { eprintln!("Could not write welcome.docx: {e}"); }
        }
        "welcome.docx".to_string()
    });
    let opened = if std::fs::metadata(&name).is_ok() { book::Book::open(&name) } else { Ok(book::Book::blank(&name)) };
    let mut book = match opened {
        Ok(book) => book,
        Err(message) => {
            println!("Assuit could not open {name}: {message}\n\nPut a document in this program's folder and run: assuit NAME.docx");
            std::process::exit(1);
        }
    };
    let (target, task) = sex_std::identity();
    sex_std::record(&format!("[assuit.open] target={target} task={task} name={name} pages={}", book.pages()));
    let window = sex_std::window();
    ui::Ui::new(book, name).run(window);
    std::process::exit(0);
}

/// On the host the engine runs only under `cargo test`.
#[cfg(not(target_os = "hermit"))]
fn main() {}
