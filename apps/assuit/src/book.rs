// SPDX-License-Identifier: GPL-3.0-only
//! The document behind the window: WordCraft's engine (storytold/wordcraft, MIT OR
//! Apache-2.0) holds the text and styles, runs every edit as one of its commands (with
//! undo), lays the pages out as Word does and draws them. Nothing here touches SexOS,
//! so the host tests drive it directly.
use std::path::Path;
use std::sync::Arc;
use std::io::{Read, Write};
use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_engine::doc::{Document, StoryRef, StyleKind};
use wordcraft_engine::layout::DocLayout;
use wordcraft_engine::render::{RenderOptions, render_page};

/// A drawn page: 0x00RRGGBB pixels, width, height.
pub struct Picture { pub pixels: Vec<u32>, pub width: usize, pub height: usize }

/// One real table style in the open document, with colours for a native sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableStyleChoice {
    pub name: String,
    pub header: u32,
    pub body: u32,
    pub band: u32,
    pub borders: bool,
}

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
    /// The app-owned proofing word list. The document's exact-file grant never
    /// authorizes writing a sibling next to someone else's document.
    pub const DICTIONARY: &'static str = "assuit-custom-dictionary.txt";

    pub fn load_dictionary(&mut self, path: &Path) {
        wordcraft_engine::proof::load_user_dictionary(path);
        self.session.user_dict_path = Some(path.to_path_buf());
    }

    /// An empty document that Ctrl+S saves to `path`.
    pub fn blank(path: &str) -> Book {
        #[allow(unused_mut)] // the SexOS build loads the app-owned dictionary below
        let mut book = Book { session: Session::new(Document::new()), path: path.to_string(), edits: 0 };
        #[cfg(target_os = "hermit")]
        book.load_dictionary(Path::new(Self::DICTIONARY));
        book
    }

    /// Opens a docx, doc, odt, rtf, html, md or txt file.
    pub fn open(path: &str) -> Result<Book, String> {
        let (doc, _) = wordcraft_engine::io::open_path_with(Path::new(path), None)?;
        let mut session = Session::new(doc);
        session.path = Some(path.into());
        session.dirty = false;
        #[allow(unused_mut)] // the SexOS build loads the app-owned dictionary below
        let mut book = Book { session, path: path.to_string(), edits: 0 };
        #[cfg(target_os = "hermit")]
        book.load_dictionary(Path::new(Self::DICTIONARY));
        Ok(book)
    }

    /// Runs one WordCraft command, such as `text.insert` or `format.bold`.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let r = self.session.run(id, &params).map_err(|e| format!("{e:?}"))?;
        if self.session.registry.get(id).is_some_and(|spec| spec.mutates) { self.edits += 1; }
        Ok(r)
    }
    pub fn type_text(&mut self, text: &str) -> Result<(), String> { self.run("text.insert", json!({ "text": text })).map(|_| ()) }

    /// Plain text for SexOS's shared clipboard. WordCraft also retains its
    /// formatted fragment so a same-document paste can preserve styling.
    pub fn copy_text(&mut self) -> Result<String, String> {
        self.run("edit.copy", json!({}))?["text"].as_str()
            .map(str::to_string).ok_or_else(|| "Copy returned no text".into())
    }

    /// Pass the actual system clipboard text to WordCraft. It uses its own
    /// formatted fragment only when that text still matches the last copy.
    pub fn paste_text(&mut self, text: &str) -> Result<(), String> {
        self.run("edit.paste", json!({ "text": text })).map(|_| ())
    }

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

    /// Available DOCX table styles, including inherited conditional fills.
    /// ODT cannot keep a named table style through WordCraft's writer yet.
    pub fn table_styles(&self) -> Result<Vec<TableStyleChoice>, String> {
        if self.save_path().to_ascii_lowercase().ends_with(".odt") {
            return Err("ODT named table styles do not round-trip yet; save as DOCX for this command".into());
        }
        let sheet = &self.session.doc.styles;
        let rgb = |c: wordcraft_engine::doc::Rgb| (u32::from(c.0) << 16) | (u32::from(c.1) << 8) | u32::from(c.2);
        let mut styles: Vec<_> = sheet.styles.iter()
            .filter(|style| style.kind == StyleKind::Table && !style.hidden)
            .filter_map(|style| {
                let parts = sheet.table_style(&style.id)?.parts;
                let body = parts.fill.map(rgb).unwrap_or(0xFFFFFF);
                Some((style.priority.unwrap_or(u32::MAX), TableStyleChoice {
                    name: style.name.clone(),
                    header: parts.header_fill.map(rgb).unwrap_or(body),
                    body,
                    band: parts.band_fill.map(rgb).unwrap_or(body),
                    borders: parts.borders.is_some() || parts.header_borders.is_some(),
                }))
            }).collect();
        styles.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
        Ok(styles.into_iter().map(|(_, style)| style).collect())
    }

    /// The style name of the caret's paragraph.
    pub fn paragraph_style(&mut self) -> String {
        self.run("styles.inspect", json!({})).ok()
            .and_then(|r| r["paragraph"]["styleName"].as_str().map(str::to_string))
            .unwrap_or_default()
    }

    /// Gives the caret's (or the selection's) paragraphs the style `id`.
    pub fn set_style(&mut self, id: &str) -> Result<(), String> { self.run("para.style", json!({ "style": id })).map(|_| ()) }

    /// Edit an actual WordCraft list at the caret or across selected paragraphs.
    /// The names stay deliberately finite so a typo cannot pick a different list.
    pub fn edit_list(&mut self, input: &str) -> Result<&'static str, String> {
        let action = input.trim().to_ascii_lowercase();
        if let Some(level) = action.strip_prefix("level ") {
            let level = level.parse::<u8>().map_err(|_| "Enter level 1 through level 9".to_string())?;
            if !(1..=9).contains(&level) { return Err("Enter level 1 through level 9".into()); }
            self.run("list.get", json!({}))
                .map_err(|_| "Place the caret in a list first".to_string())?;
            self.run("para.listLevel", json!({ "level": level - 1 }))
                .map_err(|_| "Could not change the list level".to_string())?;
            return Ok("Changed list level");
        }
        let (command, params, result) = match action.as_str() {
            "bullet" | "bullets" => ("para.bullets", json!({}), "Toggled bullets"),
            "numbered" | "numbers" => ("para.numbering", json!({}), "Toggled numbering"),
            "numbered paren" => ("para.numbering", json!({ "kind": "numberedParen" }), "Toggled parenthesized numbering"),
            "upper letter" => ("para.numbering", json!({ "kind": "upperLetter" }), "Toggled upper-letter numbering"),
            "lower letter" => ("para.numbering", json!({ "kind": "lowerLetter" }), "Toggled lower-letter numbering"),
            "lower roman" => ("para.numbering", json!({ "kind": "lowerRoman" }), "Toggled lower-roman numbering"),
            "legal" => ("para.multilevel", json!({ "kind": "legal" }), "Toggled legal numbering"),
            "off" => ("para.numbering", json!({ "off": true }), "Removed list formatting"),
            "restart" => ("para.restartNumbering", json!({}), "Restarted numbering at one"),
            _ => return Err("Choose bullet, numbered, numbered paren, upper letter, lower letter, lower roman, legal, off, restart or level 1–9".into()),
        };
        self.run(command, params).map_err(|_| "Place the caret in a paragraph that can take this list action".to_string())?;
        Ok(result)
    }

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
        if matches!(action.as_str(), "header on" | "header off") {
            let on = action == "header on";
            self.run("table.repeatHeader", json!({ "value": on }))
                .map_err(|_| "Place the caret in the table row to change".to_string())?;
            return Ok(if on { "Repeating table header enabled" } else { "Repeating table header disabled" });
        }
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
            _ => return Err("Choose row/column edits, merge cells, split 2–10, header on/off, shade RRGGBB, border all/outside/inside/none, or style NAME".into()),
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

    /// Apply a line-height multiple and points before/after the current paragraph
    /// (or every selected paragraph). These are document properties, not view zoom.
    pub fn set_paragraph_spacing(&mut self, input: &str) -> Result<(), String> {
        const HELP: &str = "Enter line multiple (0.5–3), space before and after (0–72 pt), for example 1.5 0 8";
        let words: Vec<_> = input.split_whitespace().collect();
        if words.len() != 3 { return Err(HELP.into()); }
        let mut values = [0.0_f32; 3];
        for (at, word) in words.iter().enumerate() {
            values[at] = word.parse::<f32>().map_err(|_| HELP.to_string())?;
            if !values[at].is_finite() { return Err(HELP.into()); }
        }
        if !(0.5..=3.0).contains(&values[0]) || !(0.0..=72.0).contains(&values[1])
            || !(0.0..=72.0).contains(&values[2]) { return Err(HELP.into()); }
        self.run("para.lineSpacing", json!({ "value": values[0] }))?;
        self.run("para.spacing", json!({ "before": values[1], "after": values[2] }))?;
        Ok(())
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

    /// WordCraft lays spelling issues into the editable page as red proofing
    /// marks. This view choice leaves document bytes and undo history alone.
    pub fn proofing_marks(&self) -> bool { self.session.view.proofing }

    pub fn set_proofing_marks(&mut self, enabled: bool) -> Result<(), String> {
        let result = self.run("file.options", json!({ "checkSpelling": enabled }))?;
        if result["checkSpelling"] != enabled { return Err("Proofing view did not change".into()); }
        Ok(())
    }

    /// Set the document's default proofing language and the current selection.
    pub fn proofing_language(&mut self, input: &str) -> Result<&'static str, String> {
        let tag = match input.trim().to_ascii_lowercase().as_str() {
            "en-us" | "us" => "en-US",
            "en-gb" | "uk" => "en-GB",
            _ => return Err("Choose en-US or en-GB".into()),
        };
        self.run("review.language", json!({ "lang": tag, "default": true }))?;
        Ok(tag)
    }

    /// Keep the selected issue in Assuit's app-owned dictionary.
    pub fn add_spelling_word(&mut self, word: &str) -> Result<(), String> {
        let path = self.session.user_dict_path.clone().ok_or("Dictionary is unavailable")?;
        self.run("review.addToDictionary", json!({ "word": word }))?;
        let saved = std::fs::read_to_string(&path)
            .map_err(|e| format!("Could not keep dictionary: {e}"))?;
        if !saved.lines().any(|line| line.eq_ignore_ascii_case(word)) {
            return Err("Could not keep dictionary word".into());
        }
        Ok(())
    }

    /// Only the words in Assuit's own persisted dictionary, sorted for the UI.
    pub fn dictionary_words(&self) -> Vec<String> {
        let Some(path) = &self.session.user_dict_path else { return Vec::new() };
        let mut bytes = Vec::new();
        if let Ok(file) = std::fs::File::open(path) { let _ = file.take(1_000_000).read_to_end(&mut bytes); }
        let mut words: Vec<String> = String::from_utf8_lossy(&bytes).lines()
            .filter_map(wordcraft_engine::proof::custom_word).take(100_000).collect();
        words.sort();
        words.dedup();
        words
    }

    /// Remove one saved word and make the change durable. The proofing engine's
    /// save merges concurrent additions while keeping this removal out.
    pub fn remove_spelling_word(&mut self, word: &str) -> Result<(), String> {
        let path = self.session.user_dict_path.clone().ok_or("Dictionary is unavailable")?;
        let key = wordcraft_engine::proof::custom_word(word).ok_or("Choose one dictionary word")?;
        if !self.dictionary_words().contains(&key) { return Err("Word is not in Assuit's dictionary".into()); }
        wordcraft_engine::proof::remove_word(&key);
        if let Err(e) = wordcraft_engine::proof::save_user_dictionary(&path, &[key.clone()]) {
            wordcraft_engine::proof::add_word(&key);
            return Err(format!("Could not keep dictionary: {e}"));
        }
        self.session.relayout();
        Ok(())
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
    let result = write_result.and_then(|()| rename_staged(&name, path, bytes));
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
fn rename_staged(from: &str, to: &str, bytes: &[u8]) -> std::io::Result<()> {
    // Emitted after the complete staged file was synced, immediately before
    // asking Files to promote it. CI cuts guest power on this receipt.
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
    sex_std::record(&format!("[assuit.promote.begin] from={from} to={to} bytes={} fnv={hash:016x}", bytes.len()));
    let result = sex_std::fs::rename(from, to);
    if result.is_ok() { sex_std::record(&format!("[assuit.promote.done] to={to}")); }
    result
}

#[cfg(not(target_os = "hermit"))]
fn rename_staged(from: &str, to: &str, _bytes: &[u8]) -> std::io::Result<()> { std::fs::rename(from, to) }

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
    fn numbered_lists_round_trip_in_docx_and_odt() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("list-edit.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("First item").unwrap();
            assert!(book.edit_list("number maybe").is_err());
            assert_eq!(book.edit_list("numbered").unwrap(), "Toggled numbering");
            book.key(&Chord { name: "Enter", ctrl: false, shift: false });
            book.type_text("Second item").unwrap();
            let numbered = |b: &Book| b.session.doc.body.iter().filter_map(|block| block.as_para())
                .filter(|para| para.props.numbering.is_some_and(|n| n.num != 0)).count();
            assert_eq!(numbered(&book), 2, "{ext} did not continue the list");
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert_eq!(numbered(&reopened), 2, "{ext} lost the list on save");
            assert!(reopened.text().contains("First item\nSecond item"));
            let before_restart = reopened.session.doc.body[1].as_para().unwrap().props.numbering.unwrap().num;
            reopened.edit_list("restart").unwrap();
            let after_restart = reopened.session.doc.body[1].as_para().unwrap().props.numbering.unwrap().num;
            assert_ne!(before_restart, after_restart, "{ext} did not restart numbering");
            reopened.edit_list("off").unwrap();
            assert_eq!(numbered(&reopened), 1, "off should change the caret paragraph");
            reopened.edit_list("bullet").unwrap();
            assert_eq!(numbered(&reopened), 2, "bullet should create a list again");
        }
    }

    #[test]
    fn list_format_choices_round_trip() {
        let choices = [
            ("bullet", "bullet", "•"),
            ("numbered paren", "decimal", "%1)"),
            ("upper letter", "upperLetter", "%1."),
            ("lower letter", "lowerLetter", "%1)"),
            ("lower roman", "lowerRoman", "%1."),
            ("legal", "decimal", "%1."),
        ];
        for ext in ["docx", "odt"] {
            for (action, format, marker) in choices {
                let path = temp(&format!("list-{}.{ext}", action.replace(' ', "-")));
                let mut book = Book::blank(&path);
                book.type_text("An item").unwrap();
                book.edit_list(action).unwrap();
                let check = |book: &Book| {
                    let para = book.session.doc.body[0].as_para().unwrap();
                    let num = para.props.numbering.unwrap().num;
                    let level = book.session.doc.numbering.level(num, 0).unwrap();
                    assert_eq!(level.format.ooxml(), format, "{ext} {action}");
                    assert_eq!(level.text, marker, "{ext} {action}");
                };
                check(&book);
                book.save().unwrap();
                check(&Book::open(&path).unwrap());
            }
        }
    }

    #[test]
    fn nested_list_levels_round_trip_in_docx_and_odt() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("nested-list.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("Parent item").unwrap();
            assert!(book.edit_list("level 2").is_err(), "level requires an existing list");
            book.edit_list("numbered").unwrap();
            book.key(&Chord { name: "Enter", ctrl: false, shift: false });
            book.type_text("Child item").unwrap();
            assert!(book.edit_list("level 0").is_err());
            assert!(book.edit_list("level 10").is_err());
            assert_eq!(book.edit_list("level 2").unwrap(), "Changed list level");
            let levels = |b: &Book| b.session.doc.body.iter().filter_map(|block| block.as_para())
                .map(|para| para.props.numbering.unwrap().level).collect::<Vec<_>>();
            assert_eq!(levels(&book), [0, 1]);
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert_eq!(levels(&reopened), [0, 1], "{ext} lost the nested level");
            reopened.key(&Chord { name: "End", ctrl: true, shift: false });
            reopened.edit_list("level 1").unwrap();
            assert_eq!(levels(&reopened), [0, 0]);
        }
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
    fn shared_clipboard_text_moves_between_documents_without_stale_fragments() {
        let mut source = Book::blank(&temp("clipboard-source.docx"));
        source.type_text("Assuit handoff").unwrap();
        source.key(&Chord { name: "A", ctrl: true, shift: false });
        assert_eq!(source.copy_text().unwrap(), "Assuit handoff");
        let mut destination = Book::blank(&temp("clipboard-destination.docx"));
        destination.type_text("Old").unwrap();
        destination.key(&Chord { name: "A", ctrl: true, shift: false });
        destination.copy_text().unwrap();
        destination.paste_text("Assuit handoff").unwrap();
        assert_eq!(destination.text(), "Assuit handoff");
        destination.key(&Chord { name: "Z", ctrl: true, shift: false });
        assert_eq!(destination.text(), "Old");
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
    fn paragraph_spacing_round_trips_in_docx_and_odt() {
        use wordcraft_engine::doc::props::LineSpacing;
        for ext in ["docx", "odt"] {
            let path = temp(&format!("paragraph-spacing.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("A paragraph with deliberate spacing.").unwrap();
            for bad in ["1.5 8", "NaN 0 8", "4 0 8", "1.5 -1 8", "1.5 0 100"] {
                assert!(book.set_paragraph_spacing(bad).is_err(), "{bad}");
            }
            book.set_paragraph_spacing("1.5 6 12").unwrap();
            let spacing = |b: &Book| {
                let p = b.session.doc.body.iter().find_map(|block| block.as_para()).unwrap();
                (p.props.line_spacing, p.props.space_before, p.props.space_after)
            };
            assert_eq!(spacing(&book), (Some(LineSpacing::Multiple(1.5)), Some(6.0), Some(12.0)));
            book.save().unwrap();
            let reopened = Book::open(&path).unwrap();
            assert_eq!(spacing(&reopened), spacing(&book), "{ext} lost paragraph spacing");
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
    fn table_gallery_lists_real_docx_styles_and_applies_choice() {
        let path = temp("table-gallery.docx");
        let mut book = Book::blank(&path);
        book.insert_table(2, 2).unwrap();
        let choices = book.table_styles().unwrap();
        assert!(!choices.is_empty());
        let chosen = choices.iter().find(|s| s.name == "Grid Table 1 Light").unwrap();
        assert!(chosen.borders || chosen.header != chosen.body || chosen.band != chosen.body,
            "sample should show a real style feature");
        book.edit_table(&format!("style {}", chosen.name)).unwrap();
        book.save().unwrap();
        let reopened = Book::open(&path).unwrap();
        let table = reopened.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        assert_eq!(table.props.style.as_deref(), Some("GridTable1Light"));
        let odt = Book::blank(&temp("table-gallery.odt"));
        assert!(odt.table_styles().unwrap_err().contains("ODT named table styles"));
    }

    #[test]
    fn repeating_table_header_round_trips_in_docx_and_odt() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("table-header.{ext}"));
            let mut book = Book::blank(&path);
            assert!(book.edit_table("header on").is_err(), "requires a table row");
            book.insert_table(2, 2).unwrap();
            let header = |b: &Book| b.session.doc.body.iter().find_map(|block| block.as_table())
                .unwrap().rows.iter().map(|row| row.props.header).collect::<Vec<_>>();
            assert_eq!(book.edit_table("header on").unwrap(), "Repeating table header enabled");
            assert_eq!(header(&book), [true, false]);
            book.edit_table("header off").unwrap();
            assert_eq!(header(&book), [false, false]);
            book.edit_table("header on").unwrap();
            book.save().unwrap();
            assert_eq!(header(&Book::open(&path).unwrap()), [true, false], "{ext} lost repeating header");
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

    #[test]
    fn proofing_toggle_changes_page_marks_without_changing_document_or_undo() {
        let mut book = Book::blank(&temp("proofing-marks.docx"));
        book.type_text("Hello helo world").unwrap();
        book.key(&Chord { name: "Home", ctrl: true, shift: false });
        assert!(book.proofing_marks());
        let with_marks = book.draw(0, 2.0).unwrap().pixels;
        let text = book.text();
        let edits = book.edits;
        book.set_proofing_marks(false).unwrap();
        assert!(!book.proofing_marks());
        let without_marks = book.draw(0, 2.0).unwrap().pixels;
        assert_ne!(with_marks, without_marks, "a misspelling must visibly lose its page mark");
        book.set_proofing_marks(true).unwrap();
        assert_eq!(book.draw(0, 2.0).unwrap().pixels, with_marks);
        assert_eq!(book.text(), text);
        assert_eq!(book.edits, edits);
    }

    #[test]
    fn proofing_language_survives_docx_save_and_reopen() {
        let path = temp("proofing-language.docx");
        let mut book = Book::blank(&path);
        assert!(book.proofing_language("de-DE").is_err());
        assert_eq!(book.proofing_language("uk").unwrap(), "en-GB");
        assert_eq!(book.session.doc.styles.default_chr.lang.as_deref(), Some("en-GB"));
        book.type_text("Colour and centre.").unwrap();
        book.save().unwrap();
        let reopened = Book::open(&path).unwrap();
        assert_eq!(reopened.session.doc.styles.default_chr.lang.as_deref(), Some("en-GB"));
    }

    #[test]
    fn added_word_is_kept_and_loadable_in_new_session() {
        let path = temp("assuit-proofing-words.txt");
        let word = "zxqassuitproofword";
        let mut book = Book::blank(&temp("proofing-dictionary.docx"));
        book.load_dictionary(Path::new(&path));
        book.type_text(&format!("Hello {word} world")).unwrap();
        book.key(&Chord { name: "Home", ctrl: true, shift: false });
        assert_eq!(book.next_spelling_issue().unwrap().unwrap().0, word);
        book.add_spelling_word(word).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().lines().any(|line| line == word));
        assert!(wordcraft_engine::proof::remove_word(word));
        assert!(!wordcraft_engine::proof::user_dictionary().contains(&word.to_string()));
        let mut reopened = Book::blank(&temp("proofing-dictionary-reopen.docx"));
        reopened.load_dictionary(Path::new(&path));
        assert!(wordcraft_engine::proof::user_dictionary().contains(&word.to_string()));
        reopened.type_text(&format!("Hello {word} world")).unwrap();
        reopened.key(&Chord { name: "Home", ctrl: true, shift: false });
        let next = reopened.next_spelling_issue().unwrap();
        assert!(next.is_none(), "{next:?}");
        wordcraft_engine::proof::remove_word(word);
    }

    #[test]
    fn dictionary_manager_lists_and_removes_saved_words() {
        let path = temp("assuit-manager-words.txt");
        let word = "zxqassuitmanagerword";
        let mut book = Book::blank(&temp("manager.docx"));
        book.load_dictionary(Path::new(&path));
        book.add_spelling_word(word).unwrap();
        assert!(book.dictionary_words().contains(&word.to_string()));
        book.remove_spelling_word(word).unwrap();
        assert!(!book.dictionary_words().contains(&word.to_string()));
        assert!(!wordcraft_engine::proof::user_dictionary().contains(&word.to_string()));
        let mut again = Book::blank(&temp("manager-reopen.docx"));
        again.load_dictionary(Path::new(&path));
        assert!(!again.dictionary_words().contains(&word.to_string()));
    }

}
