// SPDX-License-Identifier: GPL-3.0-only
//! The document behind the window: WordCraft's engine (storytold/wordcraft, MIT OR
//! Apache-2.0) holds the text and styles, runs every edit as one of its commands (with
//! undo), lays the pages out as Word does and draws them. Nothing here touches SexOS,
//! so the host tests drive it directly.
use std::path::Path;
use std::sync::Arc;
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
}

impl Book {
    /// An empty document that Ctrl+S saves to `path`.
    pub fn blank(path: &str) -> Book {
        Book { session: Session::new(Document::new()), path: path.to_string() }
    }

    /// Opens a docx, doc, odt, rtf, html, md or txt file.
    pub fn open(path: &str) -> Result<Book, String> {
        let (doc, _) = wordcraft_engine::io::open_path_with(Path::new(path), None)?;
        let mut session = Session::new(doc);
        session.path = Some(path.into());
        session.dirty = false;
        Ok(Book { session, path: path.to_string() })
    }

    /// Runs one WordCraft command, such as `text.insert` or `format.bold`.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.session.run(id, &params).map_err(|e| format!("{e:?}"))
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

    /// The document as the bytes of its file, in the format `path`'s extension names.
    pub fn file_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        wordcraft_engine::io::save_bytes(path, &self.session.doc)
    }

    /// Writes the document to its file with std::fs::write (the engine's safe save
    /// creates and renames a temporary file). Returns the size.
    pub fn save(&mut self) -> Result<usize, String> {
        let bytes = self.file_bytes(&self.path)?;
        std::fs::write(&self.path, &bytes).map_err(|e| format!("Could not save {}: {e}", self.path))?;
        self.session.dirty = false;
        Ok(bytes.len())
    }
}

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
