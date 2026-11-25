// SPDX-License-Identifier: GPL-3.0-only
//! The document behind the window: WordCraft's engine (storytold/wordcraft, MIT OR
//! Apache-2.0) holds the text and styles, runs every edit as one of its commands (with
//! undo), lays the pages out as Word does and draws them. Nothing here touches SexOS,
//! so the host tests drive it directly.
use std::path::Path;
use std::sync::Arc;
use std::io::Write;
use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_engine::doc::{Document, StoryRef};
use wordcraft_engine::layout::DocLayout;
use wordcraft_engine::render::{RenderOptions, render_page};

/// A drawn page: 0x00RRGGBB pixels, width, height.
pub struct Picture { pub pixels: Vec<u32>, pub width: usize, pub height: usize }

/// Where the caret is: page, and its top-left and height in page points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret { pub page: usize, pub x: f32, pub top: f32, pub height: f32 }

/// A key as WordCraft's shortcuts write it, with its modifiers.
pub struct Chord<'a> { pub name: &'a str, pub ctrl: bool, pub shift: bool }

pub struct Book {
    session: Session,
    pub path: String,
    /// Commands run that change the document (WordCraft marks them `mutates`): typing,
    /// formatting, styles, undo. The window watches it to keep unsaved work.
    pub edits: u64,
}

impl Book {
    /// An empty document that Ctrl+S saves to `path`.
    pub fn blank(path: &str) -> Book {
        Book { session: Session::new(Document::new()), path: path.to_string(), edits: 0 }
    }

    /// Opens a docx, doc, odt, rtf, html, md or txt file.
    pub fn open(path: &str) -> Result<Book, String> {
        let (doc, _) = wordcraft_engine::io::open_path_with(Path::new(path), None)?;
        let mut session = Session::new(doc);
        session.path = Some(path.into());
        session.dirty = false;
        Ok(Book { session, path: path.to_string(), edits: 0 })
    }

    /// Runs one WordCraft command, such as `text.insert` or `format.bold`.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let r = self.session.run(id, &params).map_err(|e| format!("{e:?}"))?;
        if self.session.registry.get(id).is_some_and(|spec| spec.mutates) { self.edits += 1; }
        Ok(r)
    }
    pub fn type_text(&mut self, text: &str) -> Result<(), String> { self.run("text.insert", json!({ "text": text })).map(|_| ()) }

    /// Runs what a key does in WordCraft: its shortcut's command, Shift extending a
    /// caret move. Returns the command run, if the key has one.
    pub fn key(&mut self, chord: &Chord) -> Option<String> {
        let mods = |shift: bool| format!("{}{}{}", if chord.ctrl { "Mod+" } else { "" }, if shift { "Shift+" } else { "" }, chord.name);
        let registry = self.session.registry.clone();
        let (spec, extend) = match registry.by_shortcut(&mods(chord.shift)) {
            Some(spec) => (spec, false),
            None if chord.shift => (registry.by_shortcut(&mods(false))?, true),
            None => return None,
        };
        let id = spec.id.to_string();
        let params = if extend && id.starts_with("caret.") { json!({ "extend": true }) } else { json!({}) };
        let _ = self.run(&id, params);
        Some(id)
    }

    /// Selects the next place the text holds `what`, after the caret and wrapping round.
    /// Returns (which match, counted from 1; how many there are).
    pub fn find_next(&mut self, what: &str) -> Result<(usize, usize), String> {
        let missing = || format!("\"{what}\" is not in the document");
        let next = self.run("edit.findNext", json!({ "text": what })).map_err(|_| missing())?;
        let count = next.get("count").and_then(Value::as_u64).unwrap_or(0) as usize;
        if count == 0 { return Err(missing()); }
        Ok((next.get("index").and_then(Value::as_u64).unwrap_or(0) as usize + 1, count))
    }

    /// Replaces every `what` with `with` (Word's Replace All). Returns how many were replaced.
    pub fn replace_all(&mut self, what: &str, with: &str) -> Result<usize, String> {
        let r = self.run("edit.replaceAll", json!({ "text": what, "with": with }))?;
        Ok(r.get("replaced").and_then(Value::as_u64).unwrap_or(0) as usize)
    }

    /// The paragraph styles of the Styles gallery, in its order: (id, name).
    pub fn styles(&mut self) -> Vec<(String, String)> {
        let Ok(list) = self.run("styles.manage", json!({})) else { return Vec::new() };
        list.as_array().into_iter().flatten()
            .filter(|s| s["inGallery"] == true && s["hidden"] != true && matches!(s["type"].as_str(), Some("paragraph" | "linked")))
            .filter_map(|s| Some((s["id"].as_str()?.to_string(), s["name"].as_str()?.to_string())))
            .collect()
    }

    /// The style name of the caret's paragraph.
    pub fn paragraph_style(&mut self) -> String {
        self.run("styles.inspect", json!({})).ok()
            .and_then(|r| r["paragraph"]["styleName"].as_str().map(str::to_string))
            .unwrap_or_default()
    }

    /// Gives the caret's (or the selection's) paragraphs the style `id`.
    pub fn set_style(&mut self, id: &str) -> Result<(), String> { self.run("para.style", json!({ "style": id })).map(|_| ()) }

    /// Words in the body, as a word processor's status bar counts them.
    pub fn words(&self) -> usize { self.text().split_whitespace().count() }

    pub fn dirty(&self) -> bool { self.session.dirty }
    pub fn text(&self) -> String { self.session.doc.plain_text(StoryRef::Body) }
    pub fn layout(&mut self) -> Arc<DocLayout> { self.session.layout() }
    pub fn pages(&mut self) -> usize { self.layout().pages.len() }
    /// A page's size in points.
    pub fn page_size(&mut self, page: usize) -> (f32, f32) {
        self.layout().pages.get(page).map_or((595.0, 842.0), |p| (p.w, p.h))
    }

    pub fn caret(&mut self) -> Option<Caret> {
        let focus = self.session.sel.focus.clone();
        let hint = self.session.page_hint;
        let c = self.layout().caret_on(&focus, hint)?;
        Some(Caret { page: c.page, x: c.x, top: c.top, height: c.height })
    }

    /// Puts the caret at (`x`, `y`) points on `page`, as a click does; with `extend`
    /// (Shift+click) the selection runs from where it began to there.
    pub fn click(&mut self, page: usize, x: f32, y: f32, extend: bool) -> bool {
        let Some(pos) = self.layout().hit(page, x, y, StoryRef::Body) else { return false };
        if extend { self.session.sel.focus = pos; } else { self.session.sel = wordcraft_engine::Selection::caret(pos); }
        self.session.goal_x = None;
        self.session.page_hint = page;
        true
    }

    /// The selection's highlight: (page, x, y, width, height) in page points, none for a caret.
    pub fn selection_rects(&mut self) -> Vec<(usize, f32, f32, f32, f32)> {
        let sel = self.session.sel.clone();
        if sel.is_collapsed() { return Vec::new(); }
        let hint = self.session.page_hint;
        let layout = self.layout();
        layout.selection_rects(&self.session.doc, &sel.anchor, &sel.focus, hint).into_iter().map(|(p, r)| (p, r.x, r.y, r.w, r.h)).collect()
    }

    /// Draws `page` at `scale` pixels per point.
    pub fn draw(&mut self, page: usize, scale: f32) -> Option<Picture> {
        let layout = self.layout();
        let p = layout.pages.get(page)?;
        let r = render_page(&self.session.doc, p, scale, &RenderOptions::default());
        // Premultiplied RGBA on an opaque white page: the colour channels are the pixel.
        let pixels = r.pixels.chunks_exact(4).map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32).collect();
        Some(Picture { pixels, width: r.width as usize, height: r.height as usize })
    }

    /// The document as the bytes of its file, in the format `path`'s extension names: docx,
    /// odt, rtf, html, md, txt, or pdf (every page, as printed).
    pub fn file_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        wordcraft_engine::io::save_bytes(path, &self.session.doc)
    }

    /// Where Ctrl+S writes: the file itself, or beside it as .docx when WordCraft only
    /// reads its format (Word 97-2003 .doc and .dot, or a PDF).
    pub fn save_path(&self) -> String {
        let lower = self.path.to_ascii_lowercase();
        if [".doc", ".dot", ".pdf"].iter().any(|e| lower.ends_with(e)) { self.beside("docx") } else { self.path.clone() }
    }

    /// Writes the document to its file (see [`Book::save_path`]). Returns the size.
    pub fn save(&mut self) -> Result<usize, String> {
        let path = self.save_path();
        let bytes = self.file_bytes(&path)?;
        replace_file(&path, &bytes).map_err(|e| format!("Could not save {path}: {e}"))?;
        self.forget_recovery();
        self.path = path.clone();
        self.session.path = Some(path.into());
        self.session.dirty = false;
        Ok(bytes.len())
    }

    /// The file beside this one with extension `ext` (`letter.docx` -> `letter.pdf`).
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

    /// Writes every page as a PDF beside the document (Ctrl+P: SexOS prints to PDF).
    /// Returns (its name, size).
    pub fn export_pdf(&self) -> Result<(String, usize), String> {
        let path = self.beside("pdf");
        let bytes = self.file_bytes(&path)?;
        replace_file(&path, &bytes).map_err(|e| format!("Could not write {path}: {e}"))?;
        Ok((path, bytes.len()))
    }

    /// Save As: writes the document as `path` in the format its extension names and, unless
    /// that is an export (pdf, png), goes on working in it. A name with no extension keeps
    /// this file's. Returns (path, size).
    pub fn save_as(&mut self, path: &str) -> Result<(String, usize), String> {
        let name_at = path.rfind('/').map_or(0, |i| i + 1);
        let path = if path[name_at..].contains('.') { path.to_string() } else {
            let ext = self.save_path().rsplit_once('.').map_or("docx".to_string(), |(_, e)| e.to_string());
            format!("{path}.{ext}")
        };
        let lower = path.to_ascii_lowercase();
        if [".doc", ".dot"].iter().any(|e| lower.ends_with(e)) {
            return Err("Word 97-2003 files open but do not save: save as .docx, .odt or .rtf".into());
        }
        let bytes = self.file_bytes(&path)?;
        replace_file(&path, &bytes).map_err(|e| format!("Could not save {path}: {e}"))?;
        if ![".pdf", ".png"].iter().any(|e| lower.ends_with(e)) {
            self.forget_recovery();
            self.path = path.clone();
            self.session.path = Some(path.clone().into());
            self.session.dirty = false;
        }
        Ok((path, bytes.len()))
    }

    /// Keeps unsaved changes safe in [`recovery_path`] (on close, and after a quiet moment),
    /// as WordCraft's own docx so nothing is lost.
    pub fn write_recovery(&self) -> Result<(), String> {
        let path = recovery_path(&self.path);
        let bytes = self.file_bytes(&path)?;
        replace_file(&path, &bytes).map_err(|e| format!("Could not keep the changes in {path}: {e}"))
    }

    /// Opens `path` with the unsaved changes kept from last time, if there are any: the
    /// document is then changed (not saved) and Ctrl+S writes it to `path`.
    pub fn open_recovered(path: &str) -> Option<Book> {
        let kept = recovery_path(path);
        std::fs::metadata(&kept).ok()?;
        let mut book = Book::open(&kept).ok()?;
        book.path = path.to_string();
        book.session.path = Some(path.into());
        book.session.dirty = true;
        Some(book)
    }

    /// Throws the kept changes away.
    pub fn forget_recovery(&self) { let _ = std::fs::remove_file(recovery_path(&self.path)); }

    /// Ctrl+G: puts the caret at the top of page `page` (counted from 1, clamped to the
    /// pages there are). Returns the page, counted from 0.
    pub fn go_to_page(&mut self, page: usize) -> usize {
        let page = page.clamp(1, self.pages().max(1)) - 1;
        // The first line: a click just inside the body's top-left corner lands on it.
        let (x, y) = self.layout().pages.get(page).map_or((72.0, 72.0), |p| (p.body.x + 1.0, p.body.y + 1.0));
        self.click(page, x, y, false);
        page
    }
}

/// Where unsaved changes to `path` are kept. An exact external file grant does not
/// authorize a sibling, so its recovery copy lives in Assuit's own folder.
pub fn recovery_path(path: &str) -> String {
    if path.starts_with("@/") {
        let hash = path.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        format!("assuit-recovered-{hash:016x}.docx")
    } else { format!("{path}.recovered.docx") }
}

/// Stage bytes beside the destination, then ask Files to replace its name in one
/// rename. A failed encode or write leaves the previous document untouched.
fn replace_file(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::{Error, ErrorKind};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut staged = None;
    for _ in 0..16 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // An exact `@/file.docx` grant does not authorize a temporary sibling.
        let name = if path.starts_with("@/") { format!("assuit-{n}.tmp") }
            else { format!("{path}.assuit-{n}.tmp") };
        match OpenOptions::new().write(true).create_new(true).open(&name) {
            Ok(file) => { staged = Some((name, file)); break; }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let (name, mut file) = staged.ok_or_else(|| Error::new(ErrorKind::AlreadyExists, "no free Assuit staging name"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const WELCOME: &[u8] = include_bytes!("../assets/welcome.docx");

    fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("assuit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name).to_string_lossy().into_owned()
    }

    fn welcome(name: &str) -> Book {
        let path = temp(name);
        std::fs::write(&path, WELCOME).unwrap();
        Book::open(&path).unwrap()
    }

    #[test]
    fn opens_the_hand_written_docx() {
        let mut book = welcome("open.docx");
        let text = book.text();
        assert!(text.starts_with("Welcome to Assuit"), "{text}");
        assert!(text.contains("Word documents"));
        assert_eq!(book.pages(), 1);
        let (w, h) = book.page_size(0);
        assert!((w - 595.3).abs() < 1.0 && (h - 841.9).abs() < 1.0, "A4: {w} x {h}");
        assert!(!book.dirty());
    }

    #[test]
    fn page_draws_with_ink() {
        let mut book = welcome("draw.docx");
        let p = book.draw(0, 1.0).unwrap();
        assert_eq!((p.width, p.height), (596, 842));
        let ink = p.pixels.iter().filter(|px| **px != 0x00ffffff).count();
        assert!(ink > 2000, "ink {ink}");
        // The title is blue (0B5394).
        assert!(p.pixels.iter().any(|px| { let (r, g, b) = (px >> 16 & 255, px >> 8 & 255, px & 255); b > 120 && r < 60 && g < 120 }));
    }

    #[test]
    fn keys_run_wordcraft_commands() {
        let mut book = welcome("keys.docx");
        let start = book.caret().unwrap();
        assert_eq!(book.key(&Chord { name: "End", ctrl: false, shift: false }).as_deref(), Some("caret.end"));
        let end = book.caret().unwrap();
        assert!(end.x > start.x);
        book.type_text("!").unwrap();
        assert!(book.text().starts_with("Welcome to Assuit!"));
        assert!(book.dirty());
        assert_eq!(book.key(&Chord { name: "Backspace", ctrl: false, shift: false }).as_deref(), Some("text.backspace"));
        assert!(book.text().starts_with("Welcome to Assuit\n"));
        book.key(&Chord { name: "Enter", ctrl: false, shift: false });
        book.type_text("Second line").unwrap();
        assert!(book.text().starts_with("Welcome to Assuit\nSecond line\n"), "{}", book.text());
        assert_eq!(book.key(&Chord { name: "Z", ctrl: true, shift: false }).as_deref(), Some("edit.undo"));
        assert!(!book.text().contains("Second line"));
        // Shift+Home selects the line, Ctrl+B makes it bold.
        assert_eq!(book.key(&Chord { name: "Home", ctrl: false, shift: true }).as_deref(), Some("caret.home"));
        assert_eq!(book.key(&Chord { name: "B", ctrl: true, shift: false }).as_deref(), Some("format.bold"));
    }

    #[test]
    fn click_places_the_caret() {
        let mut book = welcome("click.docx");
        let before = book.caret().unwrap();
        assert!(book.click(0, 300.0, 200.0, false));
        let after = book.caret().unwrap();
        assert_ne!(before, after);
    }

    #[test]
    fn selection_has_a_highlight() {
        let mut book = welcome("select.docx");
        assert!(book.selection_rects().is_empty());
        book.key(&Chord { name: "End", ctrl: false, shift: true });
        let rects = book.selection_rects();
        assert_eq!(rects.len(), 1, "{rects:?}");
        let (page, _, _, w, h) = rects[0];
        assert_eq!(page, 0);
        assert!(w > 50.0 && h > 10.0, "{rects:?}");
        // Shift+click stretches it to a later line.
        let caret = book.caret().unwrap();
        assert!(book.click(0, 200.0, caret.top + 120.0, true));
        assert!(book.selection_rects().len() > 1);
        book.click(0, 200.0, caret.top, false);
        assert!(book.selection_rects().is_empty());
    }

    #[test]
    fn long_documents_flow_onto_more_pages() {
        let mut book = welcome("long.docx");
        book.key(&Chord { name: "End", ctrl: true, shift: false });
        for i in 0..60 { book.type_text(&format!("Line {i} of a longer document")).unwrap(); book.key(&Chord { name: "Enter", ctrl: false, shift: false }); }
        assert!(book.pages() >= 2, "{} pages", book.pages());
        assert_eq!(book.caret().unwrap().page, book.pages() - 1);
        let p = book.draw(1, 0.5).unwrap();
        assert!(p.pixels.iter().filter(|px| **px != 0x00ffffff).count() > 500);
    }

    #[test]
    fn save_and_reopen_keeps_text_and_styles() {
        let mut book = welcome("round.docx");
        book.key(&Chord { name: "End", ctrl: false, shift: false });
        book.type_text(" today").unwrap();
        let bytes = book.save().unwrap();
        assert!(bytes > 1000);
        assert!(!book.dirty());
        let mut again = Book::open(&book.path).unwrap();
        assert_eq!(again.text(), book.text());
        assert!(again.text().starts_with("Welcome to Assuit today"));
        let a = book.draw(0, 0.5).unwrap();
        let b = again.draw(0, 0.5).unwrap();
        let differ = a.pixels.iter().zip(&b.pixels).filter(|(x, y)| x != y).count();
        assert!(differ < a.pixels.len() / 200, "{differ} pixels differ after a round trip");
    }

    #[test]
    fn blank_document_saves_as_docx() {
        let path = temp("blank.docx");
        let mut book = Book::blank(&path);
        book.type_text("Hello").unwrap();
        book.save().unwrap();
        assert!(Book::open(&path).unwrap().text().starts_with("Hello"));
    }

    #[test]
    fn find_selects_matches_and_copy_pastes() {
        let mut book = welcome("find.docx");
        let words = book.words();
        assert!(words > 20, "{words}");
        let (first, count) = book.find_next("Assuit").unwrap();
        assert_eq!((first, count), (1, count));
        assert!(count >= 2);
        assert_eq!(book.selection_rects().len(), 1, "the match is selected");
        let (second, _) = book.find_next("Assuit").unwrap();
        assert_eq!(second, 2);
        assert!(book.find_next("no such words here").is_err());
        // The selected match copies and pastes at the end.
        book.key(&Chord { name: "C", ctrl: true, shift: false });
        book.key(&Chord { name: "End", ctrl: true, shift: false });
        book.type_text(" ").unwrap();
        book.key(&Chord { name: "V", ctrl: true, shift: false });
        assert!(book.text().trim_end().ends_with("Assuit"), "{:?}", book.text().chars().rev().take(40).collect::<String>());
        assert_eq!(book.words(), words + 1);
    }


    #[test]
    fn round_trips_odt_rtf_and_markdown() {
        let mut book = welcome("formats.docx");
        book.key(&Chord { name: "End", ctrl: true, shift: false });
        book.type_text(" Last words.").unwrap();
        let text = book.text();
        for ext in ["odt", "rtf", "html", "md", "txt"] {
            let (path, n) = book.save_as(&temp(&format!("copy.{ext}"))).unwrap();
            assert!(n > 20, "{ext}");
            assert_eq!(book.path, path);
            let again = Book::open(&path).unwrap();
            let words = |t: &str| t.split_whitespace().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(words(&again.text()), words(&text), "{ext} keeps the words");
        }
        // The title keeps its style through ODT, Writer's own format.
        let mut odt = Book::open(&temp("copy.odt")).unwrap();
        assert_eq!(odt.paragraph_style(), "Title");
    }

    #[test]
    fn exports_pdf_and_keeps_unsaved_changes() {
        let mut book = welcome("letter.docx");
        let (pdf, n) = book.export_pdf().unwrap();
        assert!(pdf.ends_with("letter.pdf") && n > 1000);
        assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
        let (as_pdf, _) = book.save_as(&temp("printed.pdf")).unwrap();
        assert!(as_pdf.ends_with("printed.pdf"));
        assert!(book.path.ends_with("letter.docx"), "a PDF is an export");
        assert!(book.save_as(&temp("old.doc")).is_err());

        let path = book.path.clone();
        assert!(Book::open_recovered(&path).is_none());
        book.key(&Chord { name: "End", ctrl: true, shift: false });
        book.type_text(" Unsaved.").unwrap();
        book.write_recovery().unwrap();
        let mut back = Book::open_recovered(&path).unwrap();
        assert!(back.dirty());
        assert_eq!(back.path, path);
        assert!(back.text().trim_end().ends_with("Unsaved."));
        back.save().unwrap();
        assert!(Book::open_recovered(&path).is_none(), "saving forgets the kept copy");
        assert!(Book::open(&path).unwrap().text().trim_end().ends_with("Unsaved."));
    }

    #[test]
    fn go_to_page_moves_the_caret() {
        let mut book = welcome("pages.docx");
        for _ in 0..80 { book.type_text("A paragraph long enough to fill some of the page. ").unwrap(); book.key(&Chord { name: "Enter", ctrl: false, shift: false }); }
        let pages = book.pages();
        assert!(pages >= 2);
        assert_eq!(book.go_to_page(2), 1);
        assert_eq!(book.caret().unwrap().page, 1);
        assert_eq!(book.go_to_page(99), pages - 1);
        assert_eq!(book.go_to_page(0), 0);
        assert_eq!(book.caret().unwrap().page, 0);
    }

    #[test]
    fn edits_count_changes_not_moves() {
        let mut book = welcome("edits.docx");
        assert_eq!(book.edits, 0);
        book.key(&Chord { name: "End", ctrl: false, shift: false });
        book.key(&Chord { name: "Home", ctrl: false, shift: true });
        assert_eq!(book.edits, 0, "moving and selecting change nothing");
        book.key(&Chord { name: "B", ctrl: true, shift: false });
        assert_eq!(book.edits, 1, "bold is an edit with the same text");
        book.set_style("Heading1").unwrap();
        assert_eq!(book.edits, 2);
        book.type_text("x").unwrap();
        assert_eq!(book.edits, 3);
    }

    #[test]
    fn doc_saves_beside_as_docx() {
        let book = Book::blank(&temp("old.doc"));
        assert!(book.save_path().ends_with("old.docx"));
    }

    #[test]
    fn exact_file_grant_uses_own_folder_for_recovery() {
        let kept = recovery_path("@/documents/letter.docx");
        assert!(kept.starts_with("assuit-recovered-") && kept.ends_with(".docx"));
        assert_eq!(kept, recovery_path("@/documents/letter.docx"));
        assert_ne!(kept, recovery_path("@/documents/other.docx"));
        let book = Book::blank("@/documents/letter.docx");
        assert_eq!(book.beside("pdf"), "letter.pdf");
    }

    #[test]
    fn replacement_preserves_previous_file_until_staged_bytes_are_ready() {
        let path = temp("replace.docx");
        std::fs::write(&path, b"previous copy").unwrap();
        let staged_name = format!("{path}.assuit-0.tmp");
        // A stale staging name must not truncate either that file or the document.
        std::fs::write(&staged_name, b"stale staging data").unwrap();
        replace_file(&path, b"new complete copy").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new complete copy");
        assert_eq!(std::fs::read(&staged_name).unwrap(), b"stale staging data");
        std::fs::remove_file(staged_name).unwrap();
    }

    #[test]
    fn replace_and_paragraph_styles() {
        let mut book = welcome("styles.docx");
        assert_eq!(book.replace_all("Assuit", "Assuit Writer").unwrap(), book.text().matches("Assuit").count());
        assert!(book.text().starts_with("Welcome to Assuit Writer"));
        let styles = book.styles();
        let names: Vec<&str> = styles.iter().map(|s| s.1.as_str()).collect();
        assert!(names.contains(&"Normal") && names.contains(&"Heading 1"), "{names:?}");
        assert!(!names.contains(&"Strong"), "character styles stay out: {names:?}");
        assert_eq!(book.paragraph_style(), "Title");
        book.set_style("Heading1").unwrap();
        assert_eq!(book.paragraph_style(), "Heading 1");
        assert!(book.dirty());
    }

}
