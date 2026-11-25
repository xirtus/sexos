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

    /// Inserts an editable table at the caret. Keep the UI's dimensions bounded so
    /// one accidental input cannot lay out thousands of cells at once.
    pub fn insert_table(&mut self, rows: usize, cols: usize) -> Result<(), String> {
        if !(1..=20).contains(&rows) || !(1..=10).contains(&cols) {
            return Err("Table size must be 1–20 rows by 1–10 columns".into());
        }
        self.run("insert.table", json!({ "rows": rows, "cols": cols })).map(|_| ())
    }

    /// Edit the table containing the caret through WordCraft's undoable commands.
    /// Keep the accepted phrases explicit so a typo cannot delete a different part.
    pub fn edit_table(&mut self, action: &str) -> Result<&'static str, String> {
        let action = action.trim().to_ascii_lowercase();
        if let Some(columns) = action.strip_prefix("split ") {
            let columns = columns.parse::<usize>().map_err(|_| "Enter split 2 through split 10".to_string())?;
            if !(2..=10).contains(&columns) { return Err("Enter split 2 through split 10".into()); }
            self.run("table.split", json!({ "columns": columns }))
                .map_err(|_| "Place the caret in a table cell first".to_string())?;
            return Ok("Split cell");
        }
        if let Some(color) = action.strip_prefix("shade ") {
            if color.len() != 6 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Enter shade RRGGBB, for example shade FFE8B0".into());
            }
            self.run("table.shading", json!({ "color": color }))
                .map_err(|_| "Place the caret in a table cell first".to_string())?;
            return Ok("Shaded selected cells");
        }
        if let Some(kind) = action.strip_prefix("border ") {
            if !matches!(kind, "all" | "outside" | "inside" | "none") {
                return Err("Enter border all, outside, inside, or none".into());
            }
            if self.save_path().to_ascii_lowercase().ends_with(".odt") {
                return Err("ODT table borders do not round-trip yet; save as DOCX for this command".into());
            }
            self.run("table.borders", json!({ "kind": kind }))
                .map_err(|_| "Place the caret in a table cell first".to_string())?;
            return Ok("Changed table borders");
        }
        if let Some(name) = action.strip_prefix("style ") {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 80 {
                return Err("Enter style followed by a table style name, for example style Grid Table 1 Light".into());
            }
            if self.save_path().to_ascii_lowercase().ends_with(".odt") {
                return Err("ODT named table styles do not round-trip yet; save as DOCX for this command".into());
            }
            self.run("table.style", json!({ "style": name }))
                .map_err(|_| "Place the caret in a table and enter one of its available table style names".to_string())?;
            return Ok("Applied table style");
        }
        let (command, description) = match action.as_str() {
            "select row" => ("table.selectRow", "Selected row"),
            "select column" => ("table.selectColumn", "Selected column"),
            "row above" => ("table.insertRowAbove", "Inserted row above"),
            "row below" => ("table.insertRowBelow", "Inserted row below"),
            "column left" => ("table.insertColumnLeft", "Inserted column left"),
            "column right" => ("table.insertColumnRight", "Inserted column right"),
            "delete row" => ("table.deleteRow", "Deleted row"),
            "delete column" => ("table.deleteColumn", "Deleted column"),
            "merge cells" => ("table.merge", "Merged selected cells"),
            _ => return Err("Choose row/column edits, merge cells, split 2–10, shade RRGGBB, border all/outside/inside/none, or style NAME".into()),
        };
        self.run(command, json!({})).map_err(|_| "Place the caret in a table cell first".to_string())?;
        Ok(description)
    }

    /// Set the current section's paper and orientation. WordCraft stores this in the
    /// document, so DOCX and ODT preserve it after Save As and reopen.
    pub fn set_paper(&mut self, input: &str) -> Result<(), String> {
        let mut words = input.split_whitespace();
        let size = match words.next().unwrap_or("").to_ascii_lowercase().as_str() {
            "letter" => "Letter", "legal" => "Legal", "a4" => "A4",
            _ => return Err("Choose Letter, Legal or A4, then portrait or landscape".into()),
        };
        let orientation = match words.next().unwrap_or("").to_ascii_lowercase().as_str() {
            "portrait" => "portrait", "landscape" => "landscape",
            _ => return Err("Choose Letter, Legal or A4, then portrait or landscape".into()),
        };
        if words.next().is_some() { return Err("Choose Letter, Legal or A4, then portrait or landscape".into()); }
        self.run("layout.size", json!({ "name": size }))?;
        self.run("layout.orientation", json!({ "value": orientation }))?;
        Ok(())
    }

    /// Set page margins with a named preset or top, bottom, left, right in inches.
    /// WordCraft checks that the resulting section leaves room for text.
    pub fn set_margins(&mut self, input: &str) -> Result<(), String> {
        const HELP: &str = "Choose normal, narrow, moderate, wide or mirrored; or four inch values: top bottom left right";
        let words: Vec<_> = input.split_whitespace().collect();
        let params = if words.len() == 1 {
            let preset = words[0].to_ascii_lowercase();
            if !matches!(preset.as_str(), "normal" | "narrow" | "moderate" | "wide" | "mirrored") {
                return Err(HELP.into());
            }
            json!({ "preset": preset })
        } else if words.len() == 4 {
            let mut points = [0.0_f32; 4];
            for (at, word) in words.iter().enumerate() {
                let inches = word.parse::<f32>().map_err(|_| HELP.to_string())?;
                if !inches.is_finite() || !(0.0..=10.0).contains(&inches) { return Err(HELP.into()); }
                points[at] = inches * 72.0;
            }
            json!({ "top": points[0], "bottom": points[1], "left": points[2], "right": points[3] })
        } else { return Err(HELP.into()); };
        self.run("layout.margins", params).map(|_| ())
    }

    /// Inserts a picture from a file the program can read in its granted folder.
    pub fn insert_picture(&mut self, path: &str) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("Could not read {path}: {e}"))?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("Picture is larger than 16 MiB".into());
        }
        let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
        self.run("insert.picture", json!({ "data": data, "alt": path.rsplit('/').next().unwrap_or(path) })).map(|_| ())
    }

    /// Set useful alternative text on the selected picture. This is stored in
    /// the document and travels with the image on DOCX and ODT save.
    pub fn picture_alt_text(&mut self, text: &str) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() || text.chars().count() > 500 {
            return Err("Enter 1–500 characters describing the selected picture".into());
        }
        self.run("picture.altText", json!({ "text": text }))
            .map_err(|_| "Select a picture first".to_string())?;
        Ok(())
    }

    /// Resize the selected picture by its width in inches while preserving its
    /// original proportions. WordCraft stores the size in document points.
    pub fn picture_width(&mut self, inches: &str) -> Result<(), String> {
        let inches = inches.trim().parse::<f32>().map_err(|_| "Enter a picture width from 0.1 to 20 inches".to_string())?;
        if !inches.is_finite() || !(0.1..=20.0).contains(&inches) {
            return Err("Enter a picture width from 0.1 to 20 inches".into());
        }
        self.run("picture.size", json!({ "width": inches * 72.0, "lockAspect": true }))
            .map_err(|_| "Select a picture first".to_string())?;
        Ok(())
    }

    /// Crop the selected picture's left, top, right and bottom edges by whole
    /// percentages of the original image, leaving at least five percent visible.
    pub fn picture_crop(&mut self, input: &str) -> Result<(), String> {
        const HELP: &str = "Enter four crop percentages: left top right bottom (0–95)";
        let values: Vec<u8> = input.split_whitespace().map(str::parse::<u8>)
            .collect::<Result<_, _>>().map_err(|_| HELP.to_string())?;
        if values.len() != 4 || values.iter().any(|v| *v > 95)
            || u16::from(values[0]) + u16::from(values[2]) > 95
            || u16::from(values[1]) + u16::from(values[3]) > 95 {
            return Err(HELP.into());
        }
        let fractions: Vec<f32> = values.into_iter().map(|v| f32::from(v) / 100.0).collect();
        self.run("picture.crop", json!({ "left": fractions[0], "top": fractions[1],
            "right": fractions[2], "bottom": fractions[3] }))
            .map_err(|_| "Select a picture first".to_string())?;
        Ok(())
    }

    /// Selects the next proofing issue and returns its word and suggestions.
    pub fn next_spelling_issue(&mut self) -> Result<Option<(String, Vec<String>)>, String> {
        let issue = self.run("review.spelling", json!({}))?;
        if issue["done"] == true { return Ok(None); }
        let word = issue["text"].as_str().ok_or("Proofing returned no word")?.to_string();
        let suggestions = issue["suggestions"].as_array().into_iter().flatten()
            .filter_map(|s| s.as_str().map(str::to_string)).collect();
        Ok(Some((word, suggestions)))
    }

    /// Replaces the selected issue with one of the engine's suggestions.
    pub fn apply_spelling(&mut self, suggestion: &str) -> Result<(), String> {
        self.run("review.applySuggestion", json!({ "text": suggestion })).map(|_| ())
    }

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

    /// Prints a chosen 1-based page range (`1-3,5`) beside the document. The
    /// engine keeps PDF links and validates the range against laid-out pages.
    pub fn export_pdf_range(&self, pages: &str) -> Result<(String, usize), String> {
        if pages.len() > 80 { return Err("Page range is too long".into()); }
        let (bytes, _) = self.session.print_pdf(&json!({ "pages": pages }))
            .map_err(|e| format!("Could not print pages: {e:?}"))?;
        let path = self.beside("pages.pdf");
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
    fn paper_setup_round_trips_in_docx_and_odt() {
        for ext in ["docx", "odt"] {
            let mut book = welcome("paper.docx");
            assert!(book.set_paper("A4 sideways").is_err());
            assert!(book.set_paper("unknown landscape").is_err());
            book.set_paper("A4 landscape").unwrap();
            let (w, h) = book.page_size(0);
            assert!(w > h && (w - 842.0).abs() < 2.0 && (h - 595.0).abs() < 2.0, "{w} × {h}");
            let (path, _) = book.save_as(&temp(&format!("paper.{ext}"))).unwrap();
            let mut reopened = Book::open(&path).unwrap();
            let (w, h) = reopened.page_size(0);
            assert!(w > h && (w - 842.0).abs() < 2.0 && (h - 595.0).abs() < 2.0, "{ext}: {w} × {h}");
        }
    }

    #[test]
    fn margins_round_trip_in_docx_and_odt() {
        for ext in ["docx", "odt"] {
            let mut book = welcome("margins.docx");
            assert!(book.set_margins("NaN 1 1 1").is_err());
            assert!(book.set_margins("wide extra").is_err());
            book.set_margins("narrow").unwrap();
            assert!((book.session.doc.last_section.margin_left - 36.0).abs() < 0.1);
            book.set_margins("0.5 0.75 1 1.25").unwrap();
            let section = &book.session.doc.last_section;
            assert!((section.margin_top - 36.0).abs() < 0.1);
            assert!((section.margin_bottom - 54.0).abs() < 0.1);
            assert!((section.margin_left - 72.0).abs() < 0.1);
            assert!((section.margin_right - 90.0).abs() < 0.1);
            let (path, _) = book.save_as(&temp(&format!("margins.{ext}"))).unwrap();
            let reopened = Book::open(&path).unwrap();
            let section = &reopened.session.doc.last_section;
            assert!((section.margin_top - 36.0).abs() < 0.1, "{ext}");
            assert!((section.margin_bottom - 54.0).abs() < 0.1, "{ext}");
            assert!((section.margin_left - 72.0).abs() < 0.1, "{ext}");
            assert!((section.margin_right - 90.0).abs() < 0.1, "{ext}");
        }
    }

    #[test]
    fn exports_pdf_and_keeps_unsaved_changes() {
        let mut book = welcome("letter.docx");
        let (pdf, n) = book.export_pdf().unwrap();
        assert!(pdf.ends_with("letter.pdf") && n > 1000);
        assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
        let (pages, n) = book.export_pdf_range("1").unwrap();
        assert!(pages.ends_with("letter.pages.pdf") && n > 1000);
        assert!(std::fs::read(&pages).unwrap().starts_with(b"%PDF-"));
        assert!(book.export_pdf_range("2").is_err(), "out-of-range pages must be refused");
        assert_eq!(book.path, temp("letter.docx"), "printing does not switch documents");
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

    #[test]
    fn inserted_table_round_trips_in_docx_and_odt() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("table.{ext}"));
            let mut book = Book::blank(&path);
            assert!(book.insert_table(0, 2).is_err());
            assert!(book.insert_table(21, 2).is_err());
            book.insert_table(2, 2).unwrap();
            book.type_text("First cell").unwrap();
            book.key(&Chord { name: "Tab", ctrl: false, shift: false });
            book.type_text("Second cell").unwrap();
            book.save().unwrap();
            let again = Book::open(&path).unwrap();
            assert!(again.text().contains("First cell"), "{ext}: {}", again.text());
            assert!(again.text().contains("Second cell"), "{ext}: {}", again.text());
        }
    }

    #[test]
    fn edited_table_round_trips_in_docx_and_odt() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("table-edit.{ext}"));
            let mut book = Book::blank(&path);
            assert!(book.edit_table("row below").is_err());
            assert!(book.edit_table("remove all").is_err());
            book.insert_table(2, 2).unwrap();
            book.type_text("Anchor").unwrap();
            book.edit_table("row below").unwrap();
            book.edit_table("column right").unwrap();
            let dimensions = |book: &Book| {
                let table = book.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
                (table.rows.len(), table.cols())
            };
            assert_eq!(dimensions(&book), (3, 3));
            book.save().unwrap();
            let mut again = Book::open(&path).unwrap();
            assert_eq!(dimensions(&again), (3, 3), "{ext} lost inserted cells");
            assert!(again.text().contains("Anchor"));
            let at = again.session.doc.body.iter().position(|b| b.as_table().is_some()).unwrap();
            again.run("caret.set", json!({"pos": {"story": "body", "path": [at, 1, 1, 0], "off": 0}})).unwrap();
            again.edit_table("delete row").unwrap();
            again.edit_table("delete column").unwrap();
            assert_eq!(dimensions(&again), (2, 2));
            again.save().unwrap();
            assert_eq!(dimensions(&Book::open(&path).unwrap()), (2, 2), "{ext} lost deleted cells");
        }
    }

    #[test]
    fn merged_and_split_cells_round_trip_in_docx_and_odt() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("table-merge.{ext}"));
            let mut book = Book::blank(&path);
            book.insert_table(2, 2).unwrap();
            assert!(book.edit_table("split 11").is_err());
            let at = book.session.doc.body.iter().position(|b| b.as_table().is_some()).unwrap();
            let cell = |column| json!({ "story": "body", "path": [at, 0, column, 0], "off": 0 });
            book.run("select.range", json!({ "anchor": cell(0), "focus": cell(1) })).unwrap();
            book.edit_table("merge cells").unwrap();
            let span = |book: &Book| book.session.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows[0].cells[0].span();
            assert_eq!(span(&book), 2);
            book.save().unwrap();
            let mut again = Book::open(&path).unwrap();
            assert_eq!(span(&again), 2, "{ext} lost merged cell");
            let at = again.session.doc.body.iter().position(|b| b.as_table().is_some()).unwrap();
            again.run("caret.set", json!({ "pos": { "story": "body", "path": [at, 0, 0, 0], "off": 0 } })).unwrap();
            again.edit_table("split 2").unwrap();
            assert_eq!(again.session.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows[0].cells.len(), 2);
            again.save().unwrap();
            assert_eq!(Book::open(&path).unwrap().session.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows[0].cells.len(), 2, "{ext} lost split cells");
        }
    }

    #[test]
    fn table_shading_and_borders_round_trip_in_docx_and_odt() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("table-style.{ext}"));
            let mut book = Book::blank(&path);
            book.insert_table(2, 2).unwrap();
            assert!(book.edit_table("shade blue").is_err());
            assert!(book.edit_table("border maybe").is_err());
            book.edit_table("shade FFE8B0").unwrap();
            if ext == "docx" { book.edit_table("border all").unwrap(); }
            else { assert!(book.edit_table("border all").unwrap_err().contains("ODT table borders")); }
            let styled = |book: &Book| {
                let table = book.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
                (table.rows[0].cells[0].props.shading.map(|color| color.hex()), table.props.borders.is_some())
            };
            assert_eq!(styled(&book), (Some("FFE8B0".into()), ext == "docx"));
            book.save().unwrap();
            assert_eq!(styled(&Book::open(&path).unwrap()), (Some("FFE8B0".into()), ext == "docx"), "{ext} lost table styling");
        }
    }

    #[test]
    fn named_table_style_round_trips_in_docx_and_refuses_odt() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("table-named-style.{ext}"));
            let mut book = Book::blank(&path);
            assert!(book.edit_table("style Grid Table 1 Light").is_err());
            book.insert_table(2, 2).unwrap();
            assert!(book.edit_table("style no such table style").is_err());
            if ext == "odt" {
                assert!(book.edit_table("style Grid Table 1 Light").unwrap_err().contains("ODT named table styles"));
                continue;
            }
            book.edit_table("style Grid Table 1 Light").unwrap();
            let style = |book: &Book| book.session.doc.body.iter().find_map(|b| b.as_table())
                .unwrap().props.style.clone();
            assert_eq!(style(&book).as_deref(), Some("GridTable1Light"));
            book.save().unwrap();
            assert_eq!(style(&Book::open(&path).unwrap()).as_deref(), Some("GridTable1Light"), "{ext} lost named table style");
        }
    }

    #[test]
    fn inserted_picture_round_trips_in_docx_and_odt() {
        let png = include_bytes!("../../../assets/brand/sexos-lips-badge-64.png");
        let picture = temp("mark.png");
        std::fs::write(&picture, png).unwrap();
        for ext in ["docx", "odt"] {
            let path = temp(&format!("picture.{ext}"));
            let mut book = Book::blank(&path);
            book.insert_picture(&picture).unwrap();
            assert_eq!(book.session.doc.media.len(), 1);
            book.save().unwrap();
            let again = Book::open(&path).unwrap();
            assert_eq!(again.session.doc.media.len(), 1, "{ext} lost the picture");
            assert!(again.session.doc.media.values().next().unwrap().starts_with(b"\x89PNG"));
        }
    }

    #[test]
    fn picture_alt_text_round_trips_in_docx_and_odt() {
        let png = include_bytes!("../../../assets/brand/sexos-lips-badge-64.png");
        let picture = temp("alt-mark.png");
        std::fs::write(&picture, png).unwrap();
        for ext in ["docx", "odt"] {
            let path = temp(&format!("picture-alt.{ext}"));
            let mut book = Book::blank(&path);
            assert!(book.picture_alt_text("An image").is_err());
            book.insert_picture(&picture).unwrap();
            book.picture_alt_text("SexOS lips badge").unwrap();
            let alt = |book: &Book| book.session.doc.body.iter().filter_map(|b| b.as_para())
                .flat_map(|p| p.object_offsets().into_iter().filter_map(|off| p.object_at(off)))
                .find_map(|o| match o { wordcraft_engine::doc::para::InlineObject::Image { alt, .. } => Some(alt.clone()), _ => None });
            assert_eq!(alt(&book).as_deref(), Some("SexOS lips badge"));
            book.save().unwrap();
            assert_eq!(alt(&Book::open(&path).unwrap()).as_deref(), Some("SexOS lips badge"), "{ext} lost alternative text");
        }
    }

    #[test]
    fn picture_width_preserves_aspect_and_round_trips() {
        let picture = temp("size-mark.png");
        std::fs::write(&picture, include_bytes!("../../../assets/brand/sexos-lips-badge-64.png")).unwrap();
        for ext in ["docx", "odt"] {
            let path = temp(&format!("picture-size.{ext}"));
            let mut book = Book::blank(&path);
            assert!(book.picture_width("2").is_err());
            book.insert_picture(&picture).unwrap();
            assert!(book.picture_width("22").is_err());
            let size = |book: &Book| book.session.doc.body.iter().filter_map(|b| b.as_para())
                .flat_map(|p| p.object_offsets().into_iter().filter_map(|off| p.object_at(off)))
                .find_map(|o| match o { wordcraft_engine::doc::para::InlineObject::Image { w, h, .. } => Some((*w, *h)), _ => None }).unwrap();
            let before = size(&book);
            book.picture_width("2").unwrap();
            let after = size(&book);
            assert!((after.0 - 144.0).abs() < 0.1);
            assert!((after.1 / after.0 - before.1 / before.0).abs() < 0.01);
            book.save().unwrap();
            let saved = size(&Book::open(&path).unwrap());
            assert!((saved.0 - 144.0).abs() < 0.1, "{ext} lost picture width: {saved:?}");
            assert!((saved.1 / saved.0 - before.1 / before.0).abs() < 0.01, "{ext} lost picture proportions");
        }
    }

    #[test]
    fn picture_crop_round_trips_in_docx_and_odt() {
        let picture = temp("crop-mark.png");
        std::fs::write(&picture, include_bytes!("../../../assets/brand/sexos-lips-badge-64.png")).unwrap();
        for ext in ["docx", "odt"] {
            let path = temp(&format!("picture-crop.{ext}"));
            let mut book = Book::blank(&path);
            assert!(book.picture_crop("10 0 5 0").is_err());
            book.insert_picture(&picture).unwrap();
            assert!(book.picture_crop("80 0 20 0").is_err());
            book.picture_crop("10 0 5 0").unwrap();
            let crop = |book: &Book| book.session.doc.body.iter().filter_map(|b| b.as_para())
                .flat_map(|p| p.object_offsets().into_iter().filter_map(|off| p.object_at(off)))
                .find_map(|o| match o { wordcraft_engine::doc::para::InlineObject::Image { crop, .. } => Some(*crop), _ => None }).unwrap();
            let want = [0.1, 0.0, 0.05, 0.0];
            assert_eq!(crop(&book), want);
            book.save().unwrap();
            let kept = crop(&Book::open(&path).unwrap());
            assert!(kept.iter().zip(want).all(|(got, want)| (*got - want).abs() < 0.001), "{ext} lost crop: {kept:?}");
        }
    }

    #[test]
    fn spelling_selects_and_corrects_a_word() {
        let mut book = Book::blank(&temp("spelling.docx"));
        book.type_text("Hello helo world").unwrap();
        book.key(&Chord { name: "Home", ctrl: true, shift: false });
        let (word, suggestions) = book.next_spelling_issue().unwrap().unwrap();
        assert_eq!(word, "helo");
        assert!(!suggestions.is_empty(), "{suggestions:?}");
        let correction = suggestions[0].clone();
        book.apply_spelling(&correction).unwrap();
        let corrected = format!("Hello {correction} world");
        assert_eq!(book.text().trim(), corrected);
        book.save().unwrap();
        assert_eq!(Book::open(&book.path).unwrap().text().trim(), corrected);
    }

}
