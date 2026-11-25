// SPDX-License-Identifier: GPL-3.0-only
//! The document behind the window: WordCraft's engine (storytold/wordcraft, MIT OR
//! Apache-2.0) holds the text and styles, runs every edit as one of its commands (with
//! undo), lays the pages out as Word does and draws them. Nothing here touches SexOS,
//! so the host tests drive it directly.
use std::path::Path;
use std::sync::Arc;
use std::io::{Read, Write};
use serde_json::{Value, json};
use wordcraft_engine::{Selection, Session};
use wordcraft_engine::doc::{Document, InlineObject, PartKind, Pos, StoryRef, StyleKind};
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
    /// Body caret saved when Assuit enters a newly inserted footnote.
    note_return: Option<Pos>,
}

impl Book {
    /// The app-owned proofing word list. The document's exact-file grant never
    /// authorizes writing a sibling next to someone else's document.
    pub const DICTIONARY: &'static str = "assuit-custom-dictionary.txt";
    pub const PROOFING_PREFERENCE: &'static str = "assuit-proofing-marks.txt";
    pub const SPELL_DICTIONARIES: &'static str = "assuit-dictionaries";

    pub fn load_dictionary(&mut self, path: &Path) {
        wordcraft_engine::proof::load_user_dictionary(path);
        self.session.user_dict_path = Some(path.to_path_buf());
    }

    /// Search only Assuit's granted folder for optional Hunspell pairs. People
    /// can place a licensed `<locale>.aff` and `<locale>.dic` pair here in Loom.
    pub fn use_spell_dictionaries(&mut self, folder: &Path) -> Result<(), String> {
        std::fs::create_dir_all(folder).map_err(|e| format!("Could not create Assuit dictionaries folder: {e}"))?;
        // Hermit's std backend refuses canonicalize before reaching SexOS's
        // file calls; use the app's granted-path implementation in the guest.
        #[cfg(target_os = "hermit")]
        let absolute = sex_std::fs::canonicalize(folder)
            .map_err(|e| format!("Could not open Assuit dictionaries folder: {e}"))?;
        #[cfg(not(target_os = "hermit"))]
        let absolute = std::fs::canonicalize(folder)
            .map_err(|e| format!("Could not open Assuit dictionaries folder: {e}"))?;
        let path = absolute.to_str().ok_or("Dictionaries folder name is not UTF-8")?;
        self.session.run("file.options", &json!({ "dictionariesFolder": path }))
            .map_err(|e| format!("Could not use Assuit dictionaries folder: {e:?}"))?;
        Ok(())
    }

    /// An empty document that Ctrl+S saves to `path`.
    pub fn blank(path: &str) -> Book {
        #[allow(unused_mut)] // the SexOS build loads the app-owned dictionary below
        let mut book = Book { session: Session::new(Document::new()), path: path.to_string(), edits: 0, note_return: None };
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
        let mut book = Book { session, path: path.to_string(), edits: 0, note_return: None };
        #[cfg(target_os = "hermit")]
        book.load_dictionary(Path::new(Self::DICTIONARY));
        Ok(book)
    }

    /// Runs one WordCraft command, such as `text.insert` or `format.bold`.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let before = self.session.rev();
        let r = self.session.run(id, &params).map_err(|e| format!("{e:?}"))?;
        // Some navigation commands create/remove an empty header through a
        // nested mutating command. Count the actual revision for autosave.
        if self.session.rev() != before || self.session.registry.get(id).is_some_and(|spec| spec.mutates) {
            self.edits += 1;
        }
        Ok(r)
    }
    pub fn type_text(&mut self, text: &str) -> Result<(), String> { self.run("text.insert", json!({ "text": text })).map(|_| ()) }

    pub fn editing_header_footer(&self) -> bool { self.session.in_header_footer() }

    /// Enter the header or footer actually shown on the caret's page. WordCraft
    /// keeps its rich story, including any dynamic page-number field.
    pub fn edit_header_footer(&mut self, header: bool) -> Result<(), String> {
        if self.editing_header_footer() { return Err("Press Esc to leave the current header or footer".into()); }
        let page = self.caret().map_or(0, |caret| caret.page);
        self.run(if header { "insert.editHeader" } else { "insert.editFooter" }, json!({ "page": page }))?;
        Ok(())
    }

    pub fn close_header_footer(&mut self) -> Result<(), String> {
        if !self.editing_header_footer() { return Ok(()); }
        self.run("insert.closeHeader", json!({}))?;
        Ok(())
    }

    pub fn editing_inserted_note(&self) -> Option<&'static str> {
        let StoryRef::Part(id) = self.session.sel.focus.story else { return None };
        if self.note_return.is_none() { return None; }
        match self.session.doc.parts.get(&id)?.kind {
            PartKind::Footnote => Some("footnote"),
            PartKind::Endnote => Some("endnote"),
            _ => None,
        }
    }

    /// Insert a linked note at a collapsed body caret, then leave its rich
    /// story active so the writer can continue editing it.
    pub fn insert_footnote(&mut self, text: &str) -> Result<(), String> {
        self.insert_note(text, false)
    }

    pub fn insert_endnote(&mut self, text: &str) -> Result<(), String> {
        self.insert_note(text, true)
    }

    fn insert_note(&mut self, text: &str, endnote: bool) -> Result<(), String> {
        let kind = if endnote { "endnote" } else { "footnote" };
        let text = text.trim();
        if text.is_empty() || text.chars().count() > 500 || text.chars().any(char::is_control) {
            return Err(format!("Enter 1–500 characters for the {kind}"));
        }
        if !self.session.sel.is_collapsed() || self.session.sel.focus.story != StoryRef::Body {
            return Err(format!("Place the caret in body text before inserting {} {kind}", if endnote { "an" } else { "a" }));
        }
        let back = self.session.sel.focus.clone();
        self.run(if endnote { "references.endnote" } else { "references.footnote" }, json!({ "text": text }))?;
        self.note_return = Some(back);
        Ok(())
    }

    pub fn close_inserted_note(&mut self) {
        if let Some(back) = self.note_return.take() {
            self.session.sel = Selection::caret(back);
            self.session.clamp_selection();
        }
    }

    /// Reopen a linked footnote or endnote from its body reference. This also
    /// works after saving and reopening a document, when insert state is gone.
    pub fn edit_note_at_caret(&mut self) -> Result<&'static str, String> {
        if self.note_return.is_some() || !self.session.sel.is_collapsed()
            || self.session.sel.focus.story != StoryRef::Body {
            return Err("Place the body caret on a note reference".into());
        }
        let back = self.session.sel.focus.clone();
        let para = self.session.doc.para_at(&back)
            .ok_or("Place the body caret on a note reference")?;
        let id = para.objects_with_offsets().find_map(|(off, object)| {
            if off == back.off || off + '\u{FFFC}'.len_utf8() == back.off {
                if let InlineObject::NoteRef { id, .. } = object { return Some(*id); }
            }
            None
        }).ok_or("Place the body caret on a note reference")?;
        let kind = match self.session.doc.parts.get(&id).map(|part| &part.kind) {
            Some(PartKind::Footnote) => "footnote",
            Some(PartKind::Endnote) => "endnote",
            _ => return Err("The note reference has no editable note".into()),
        };
        self.session.sel = Selection::caret(Pos::new(StoryRef::Part(id),
            wordcraft_engine::doc::Path::top(0), 0));
        self.session.clamp_selection();
        self.note_return = Some(back);
        Ok(kind)
    }

    /// Convert linked notes throughout the document, keeping their references
    /// and rich text. A conversion is one undoable document edit.
    pub fn convert_notes(&mut self, input: &str) -> Result<usize, String> {
        let to = match input.trim().to_ascii_lowercase().as_str() {
            "endnote" | "endnotes" => "endnote",
            "footnote" | "footnotes" => "footnote",
            "swap" => "swap",
            _ => return Err("Enter endnote, footnote, or swap".into()),
        };
        if self.editing_inserted_note().is_some() {
            return Err("Press Esc to return to the body before converting notes".into());
        }
        let source_exists = self.session.doc.parts.values().any(|part| match to {
            "endnote" => part.kind == PartKind::Footnote,
            "footnote" => part.kind == PartKind::Endnote,
            _ => matches!(part.kind, PartKind::Footnote | PartKind::Endnote),
        });
        if !source_exists { return Err("No notes of that kind to convert".into()); }
        let result = self.run("references.convertNotes", json!({ "to": to }))?;
        Ok(result["converted"].as_u64().unwrap_or(0) as usize)
    }

    /// Change document-wide note placement or numbering through WordCraft's
    /// undoable note-options command. Values use the user's words, never XML.
    pub fn set_note_options(&mut self, input: &str) -> Result<(), String> {
        const HELP: &str = "Use footnote/endnote start N, restart continuous/section/page, format decimal/roman/letters, or position bottom/beneath/section/end";
        let words: Vec<_> = input.split_whitespace().collect();
        if words.len() != 3 { return Err(HELP.into()); }
        let kind = match words[0].to_ascii_lowercase().as_str() {
            "footnote" => "footnote", "endnote" => "endnote", _ => return Err(HELP.into()),
        };
        let mut params = json!({"kind": kind, "scope": "document"});
        let value = words[2].to_ascii_lowercase();
        match words[1].to_ascii_lowercase().as_str() {
            "start" => {
                let n: u32 = value.parse().map_err(|_| HELP.to_string())?;
                if !(1..=32767).contains(&n) { return Err(HELP.into()); }
                params["numStart"] = json!(n);
            }
            "restart" => {
                let restart = match value.as_str() {
                    "continuous" => "continuous", "section" => "eachSect",
                    "page" if kind == "footnote" => "eachPage", _ => return Err(HELP.into()),
                };
                params["numRestart"] = json!(restart);
            }
            "format" => {
                let format = match value.as_str() {
                    "decimal" => "decimal", "roman" => "lowerRoman", "romanupper" => "upperRoman",
                    "letters" => "lowerLetter", "lettersupper" => "upperLetter", _ => return Err(HELP.into()),
                };
                params["numFmt"] = json!(format);
            }
            "position" => {
                if self.path.to_ascii_lowercase().ends_with(".odt")
                    && (kind == "endnote" || value != "bottom") {
                    return Err("ODT saves only footnotes at the page bottom; use DOCX for other note placement".into());
                }
                let pos = match (kind, value.as_str()) {
                    ("footnote", "bottom") => "pageBottom", ("footnote", "beneath") => "beneathText",
                    ("endnote", "section") => "sectEnd", ("endnote", "end") => "docEnd",
                    _ => return Err(HELP.into()),
                };
                params["pos"] = json!(pos);
            }
            _ => return Err(HELP.into()),
        }
        self.run("references.noteOptions", params)?;
        Ok(())
    }

    /// Apply a web or mail link to selected text, or insert its URL (or an
    /// optional following label) at the caret. WordCraft keeps the link style.
    pub fn insert_hyperlink(&mut self, input: &str) -> Result<(), String> {
        const HELP: &str = "Enter an https://, http:// or mailto: address; optionally follow it with display text";
        let mut parts = input.trim().splitn(2, ' ');
        let url = parts.next().unwrap_or("");
        let label = parts.next().map(str::trim).filter(|s| !s.is_empty());
        let destination = ["https://", "http://", "mailto:"].iter().find_map(|scheme| url.strip_prefix(scheme));
        if url.len() > 2048 || destination.is_none_or(str::is_empty)
            || url.chars().any(|c| c.is_whitespace() || c.is_control())
            || label.is_some_and(|s| s.chars().count() > 200 || s.chars().any(char::is_control)) {
            return Err(HELP.into());
        }
        let mut params = json!({ "url": url });
        if let Some(label) = label { params["text"] = json!(label); }
        self.run("insert.link", params)?;
        Ok(())
    }

    /// Remove a link from selected text, leaving the text in place. A collapsed
    /// caret cannot identify a multi-word link safely, so require a selection.
    pub fn remove_hyperlink(&mut self) -> Result<(), String> {
        if self.session.sel.is_collapsed() { return Err("Select linked text to remove its link".into()); }
        let (a, b) = self.session.sel.ordered();
        let linked = self.session.doc.paths_between(&a, &b).into_iter().any(|path| {
            let Some(para) = self.session.doc.para(a.story, &path) else { return false };
            let from = if path == a.path { a.off } else { 0 };
            let to = if path == b.path { b.off } else { para.len() };
            para.run_ranges().any(|(range, props)| range.end > from && range.start < to && props.link.is_some())
        });
        if !linked { return Err("The selected text has no link".into()); }
        self.run("insert.removeLink", json!({}))?;
        Ok(())
    }

    /// Select the complete, single-paragraph link under a collapsed caret and
    /// return its address for Assuit's native edit prompt. Adjacent runs with
    /// the same target can have different character formatting.
    pub fn select_hyperlink_at_caret(&mut self) -> Option<String> {
        if !self.session.sel.is_collapsed() { return None; }
        let at = self.session.sel.focus.clone();
        let para = self.session.doc.para_at(&at)?;
        let runs = para.run_ranges().filter_map(|(range, props)|
            props.link.as_ref().map(|link| (range, link.clone()))).collect::<Vec<_>>();
        let i = runs.iter().position(|(range, _)| range.start < at.off && at.off <= range.end)
            .or_else(|| runs.iter().position(|(range, _)| range.start <= at.off && at.off < range.end))?;
        let url = runs[i].1.clone();
        let mut first = i;
        while first > 0 && runs[first - 1].1 == url && runs[first - 1].0.end == runs[first].0.start { first -= 1; }
        let mut last = i;
        while last + 1 < runs.len() && runs[last + 1].1 == url && runs[last].0.end == runs[last + 1].0.start { last += 1; }
        self.session.sel = Selection {
            anchor: Pos { off: runs[first].0.start, ..at.clone() },
            focus: Pos { off: runs[last].0.end, ..at },
        };
        self.session.clamp_selection();
        Some(url)
    }

    fn has_table_of_contents(&self) -> bool {
        let is_toc = |instr: &str| instr.split_whitespace().next() == Some("TOC") && !instr.contains("\\c");
        self.session.doc.body.iter().filter_map(|block| block.as_para())
            .flat_map(|para| &para.objects)
            .any(|obj| matches!(obj, InlineObject::Field { instr, .. } if is_toc(instr)))
            || self.session.doc.field_ranges(StoryRef::Body).iter().any(|field| is_toc(&field.instr))
    }

    /// Insert a generated table from heading styles, refresh it after edits,
    /// or remove it. WordCraft owns field updates and DOCX/ODT serialization.
    pub fn table_of_contents(&mut self, input: &str) -> Result<(), String> {
        const HELP: &str = "Enter insert 1–9 (heading levels), update, or remove";
        let words: Vec<_> = input.split_whitespace().collect();
        match words.as_slice() {
            ["insert", levels] => {
                let n = levels.parse::<u8>().map_err(|_| HELP.to_string())?;
                if !(1..=9).contains(&n) { return Err(HELP.into()); }
                if !self.has_table_of_contents()
                    && (!self.session.sel.is_collapsed() || self.session.sel.focus.story != StoryRef::Body) {
                    return Err("Place the caret in body text without a selection".into());
                }
                self.run("references.toc", json!({ "levels": n }))?;
            }
            ["update"] if self.has_table_of_contents() => { self.run("references.updateToc", json!({}))?; }
            ["remove"] if self.has_table_of_contents() => { self.run("references.removeToc", json!({}))?; }
            ["update" | "remove"] => return Err("This document has no table of contents".into()),
            _ => return Err(HELP.into()),
        }
        Ok(())
    }

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

    /// Move the list item at the caret exactly one of Word's nine levels.
    /// A collapsed caret avoids choosing a level for a mixed selection.
    pub fn step_list_level(&mut self, deeper: bool) -> Result<u8, String> {
        if !self.session.sel.is_collapsed() {
            return Err("Place the caret in one list item first".into());
        }
        let current = self.session.doc.para_at(&self.session.sel.focus)
            .and_then(|p| p.props.numbering)
            .ok_or_else(|| "Place the caret in a list item first".to_string())?.level;
        let next = if deeper { current.checked_add(1).filter(|n| *n <= 8) }
            else { current.checked_sub(1) }
            .ok_or_else(|| if deeper { "Already at the deepest list level" }
                else { "Already at the outermost list level" }.to_string())?;
        self.run("para.listLevel", json!({ "level": next }))?;
        Ok(next + 1)
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

    /// Edit dynamic page numbers in the section at the caret. The engine owns
    /// the field, numbering, layout and DOCX/ODT serialization.
    pub fn insert_page_numbers(&mut self, input: &str) -> Result<(), String> {
        const HELP: &str = "Enter top/bottom left/center/right, optional 'of'; remove top/bottom; start N; continue; format TYPE; or first/even on/off";
        let words: Vec<_> = input.split_whitespace().collect();
        if words.len() == 2 && matches!(words[0].to_ascii_lowercase().as_str(), "first" | "even") {
            let on = match words[1].to_ascii_lowercase().as_str() {
                "on" => true, "off" => false, _ => return Err(HELP.into()),
            };
            let command = if words[0].eq_ignore_ascii_case("first") {
                "layout.differentFirstPage"
            } else { "layout.differentOddEven" };
            return self.run(command, json!({ "value": on })).map(|_| ());
        }
        if words.first().is_some_and(|w| w.eq_ignore_ascii_case("format")) {
            let choice = words[1..].join(" ").to_ascii_lowercase();
            let format = match choice.as_str() {
                "decimal" => "decimal",
                "lower roman" => "lowerRoman",
                "upper roman" => "upperRoman",
                "lower letter" => "lowerLetter",
                "upper letter" => "upperLetter",
                _ => return Err(HELP.into()),
            };
            return self.run("layout.pageNumberFormat", json!({ "format": format })).map(|_| ());
        }
        if words.len() == 1 && words[0].eq_ignore_ascii_case("continue") {
            return self.run("layout.pageNumberFormat", json!({ "start": null })).map(|_| ());
        }
        if words.len() == 2 && words[0].eq_ignore_ascii_case("start") {
            let n = words[1].parse::<u32>().map_err(|_| HELP.to_string())?;
            if !(1..=32_767).contains(&n) { return Err(HELP.into()); }
            return self.run("layout.pageNumberFormat", json!({ "start": n })).map(|_| ());
        }
        if words.len() == 2 && words[0].eq_ignore_ascii_case("remove") {
            return self.remove_generated_page_numbers(words[1]);
        }
        if !(words.len() == 2 || words.len() == 3 && words[2].eq_ignore_ascii_case("of")) {
            return Err(HELP.into());
        }
        let position = words[0].to_ascii_lowercase();
        let align = words[1].to_ascii_lowercase();
        if !matches!(position.as_str(), "top" | "bottom")
            || !matches!(align.as_str(), "left" | "center" | "right") { return Err(HELP.into()); }
        let format = if words.len() == 3 { "x of y" } else { "" };
        self.run("insert.pageNumber", json!({ "position": position, "align": align, "format": format }))
            .map(|_| ())
    }

    /// WordCraft's removal commands clear the entire header/footer. Use them only
    /// when the single section's target contains exactly a generated page-number
    /// paragraph, so an author's other content can never be discarded here.
    fn remove_generated_page_numbers(&mut self, place: &str) -> Result<(), String> {
        let header = if place.eq_ignore_ascii_case("top") { true }
            else if place.eq_ignore_ascii_case("bottom") { false }
            else { return Err("Choose remove top or remove bottom".into()); };
        if self.session.doc.sections().len() != 1 {
            return Err("Page-number removal is available for single-section documents only".into());
        }
        let set = if header { self.session.doc.last_section.headers }
            else { self.session.doc.last_section.footers };
        if set.first.is_some() || set.even.is_some() {
            return Err("This document has other header or footer layouts".into());
        }
        let id = set.default.ok_or_else(|| "No page number at that position".to_string())?;
        let blocks = self.session.doc.story(StoryRef::Part(id))
            .ok_or_else(|| "Page-number content is unavailable".to_string())?;
        let [block] = blocks.as_slice() else {
            return Err("Header or footer contains other content".into());
        };
        let para = block.as_para().ok_or_else(|| "Header or footer contains other content".to_string())?;
        let fields = para.objects.iter().map(|object| match object {
            wordcraft_engine::doc::InlineObject::Field { instr, .. } => Some(instr.as_str()),
            _ => None,
        }).collect::<Option<Vec<_>>>()
            .ok_or_else(|| "Header or footer contains other content".to_string())?;
        let only_numbers = fields.as_slice() == ["PAGE"] || fields.as_slice() == ["PAGE", "NUMPAGES"];
        let other_text: String = para.text.chars().filter(|&c| c != '\u{FFFC}').collect();
        let expected = if fields.len() == 2 { "Page  of " } else { "" };
        if !only_numbers || other_text != expected {
            return Err("Header or footer contains other content".into());
        }
        self.run(if header { "insert.removeHeader" } else { "insert.removeFooter" }, json!({})).map(|_| ())
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

    /// Apply a line-height multiple or a point height and points before/after the current paragraph
    /// (or every selected paragraph). These are document properties, not view zoom.
    pub fn set_paragraph_spacing(&mut self, input: &str) -> Result<(), String> {
        const HELP: &str = "Enter a line multiple (0.5–3) or exact/minimum height (6–72 pt), then before and after (0–72 pt); e.g. exact 18 0 8";
        let words: Vec<_> = input.split_whitespace().collect();
        let (mode, numbers) = match words.as_slice() {
            [_, _, _] => ("multiple", words.as_slice()),
            [mode @ ("multiple" | "exact" | "minimum"), _, _, _] => (*mode, &words[1..]),
            _ => return Err(HELP.into()),
        };
        let mut values = [0.0_f32; 3];
        for (at, word) in numbers.iter().enumerate() {
            values[at] = word.parse::<f32>().map_err(|_| HELP.to_string())?;
            if !values[at].is_finite() { return Err(HELP.into()); }
        }
        let valid_height = if mode == "multiple" { (0.5..=3.0).contains(&values[0]) }
            else { (6.0..=72.0).contains(&values[0]) };
        if !valid_height || !(0.0..=72.0).contains(&values[1])
            || !(0.0..=72.0).contains(&values[2]) { return Err(HELP.into()); }
        let line = match mode {
            "exact" => json!({ "exactly": values[0] }),
            "minimum" => json!({ "atLeast": values[0] }),
            _ => json!({ "value": values[0] }),
        };
        self.run("para.lineSpacing", line)?;
        self.run("para.spacing", json!({ "before": values[1], "after": values[2] }))?;
        Ok(())
    }

    /// Set left, right and relative first-line indents for the current or
    /// selected paragraphs. A negative first-line value makes a hanging indent.
    pub fn set_paragraph_indents(&mut self, input: &str) -> Result<(), String> {
        const HELP: &str = "Enter left, right and first-line indents in inches, e.g. 0.5 0 -0.25 for hanging";
        let words: Vec<_> = input.split_whitespace().collect();
        if words.len() != 3 { return Err(HELP.into()); }
        let mut inch = [0.0_f32; 3];
        for (at, word) in words.iter().enumerate() {
            inch[at] = word.parse().map_err(|_| HELP.to_string())?;
            if !inch[at].is_finite() { return Err(HELP.into()); }
        }
        if !(0.0..=3.0).contains(&inch[0]) || !(0.0..=3.0).contains(&inch[1])
            || inch[0] + inch[1] > 4.0 || !(-3.0..=3.0).contains(&inch[2])
            || inch[0] + inch[2] < 0.0 { return Err(HELP.into()); }
        self.run("para.indents", json!({
            "left": inch[0] * 72.0, "right": inch[1] * 72.0, "firstLine": inch[2] * 72.0,
        }))?;
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

    /// An app-owned view choice, independent of the opened document and its undo history.
    /// Missing means WordCraft's default (marks visible); malformed data is refused.
    pub fn restore_proofing_preference(&mut self, path: &str) -> Result<Option<bool>, String> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("Could not read proofing preference: {e}")),
        };
        let mut bytes = Vec::new();
        file.take(8).read_to_end(&mut bytes).map_err(|e| format!("Could not read proofing preference: {e}"))?;
        let enabled = match bytes.as_slice() {
            b"on\n" => true,
            b"off\n" => false,
            _ => return Err("Proofing preference is invalid".into()),
        };
        self.set_proofing_marks(enabled)?;
        Ok(Some(enabled))
    }

    /// Commit the new choice before reporting success. A failed write restores the
    /// previous page view; the document bytes and undo history stay unchanged.
    pub fn set_proofing_preference(&mut self, path: &str, enabled: bool) -> Result<(), String> {
        let old = self.proofing_marks();
        self.set_proofing_marks(enabled)?;
        if let Err(e) = replace_file(path, if enabled { b"on\n" } else { b"off\n" }) {
            let _ = self.set_proofing_marks(old);
            return Err(format!("Could not keep proofing preference: {e}"));
        }
        Ok(())
    }

    /// Set the document's default proofing language and the current selection.
    pub fn proofing_language(&mut self, input: &str) -> Result<String, String> {
        let tag = match input.trim().to_ascii_lowercase().as_str() {
            "en-us" | "us" => "en-US".to_string(),
            "en-gb" | "uk" => "en-GB".to_string(),
            "en-au" => "en-AU".to_string(),
            "en-ca" => "en-CA".to_string(),
            "none" | "zxx" => "zxx".to_string(),
            other => {
                let (language, region) = other.split_once('-').ok_or("Choose an installed language such as de-DE")?;
                if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_lowercase())
                    || !(region.len() == 2 && region.bytes().all(|b| b.is_ascii_lowercase())
                        || region.len() == 3 && region.bytes().all(|b| b.is_ascii_digit())) {
                    return Err("Choose a language-region tag such as de-DE".into());
                }
                let region = if region.len() == 2 { region.to_ascii_uppercase() } else { region.to_string() };
                let tag = format!("{language}-{region}");
                if !wordcraft_engine::proof::has_dictionary(Some(&tag)) {
                    return Err("WordCraft cannot proof that language".into());
                }
                let folder = self.session.prefs.dictionaries_folder.clone()
                    .ok_or("Assuit dictionaries folder is unavailable")?;
                let names = [tag.replace('-', "_"), tag.clone(), language.to_string()];
                if wordcraft_engine::proof::hunspell::find_in(Path::new(&folder),
                    &[&names[0], &names[1], &names[2]]).is_none() {
                    return Err(format!("Add {tag}.aff and {tag}.dic to Assuit dictionaries, then retry"));
                }
                // A pair copied while this window was open must become visible now.
                wordcraft_engine::proof::set_dictionaries_folder(Some(folder.into()));
                if let Some((_, _, reason)) = wordcraft_engine::proof::unreadable_dictionary(Some(&tag)) {
                    return Err(format!("Dictionary for {tag} could not be read: {reason}"));
                }
                if let Some(name) = wordcraft_engine::proof::missing_dictionary(Some(&tag)) {
                    return Err(format!("Add {name}.aff and {name}.dic to Assuit dictionaries, then retry"));
                }
                tag
            }
        };
        if tag == "zxx" && self.path.to_ascii_lowercase().ends_with(".odt") {
            return Err("ODT cannot preserve the no-proofing language; use DOCX".into());
        }
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
    fn direct_list_level_steps_are_bounded_and_round_trip() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("list-level-keys.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("Parent").unwrap();
            assert!(book.step_list_level(true).is_err(), "plain paragraph is not a list");
            book.edit_list("numbered").unwrap();
            book.key(&Chord { name: "Enter", ctrl: false, shift: false });
            book.type_text("Child").unwrap();
            book.key(&Chord { name: "Home", ctrl: false, shift: true });
            assert!(book.step_list_level(true).is_err(), "selection has no single level");
            book.key(&Chord { name: "End", ctrl: false, shift: false });
            let levels = |b: &Book| b.session.doc.body.iter().filter_map(|block| block.as_para())
                .map(|p| p.props.numbering.unwrap().level).collect::<Vec<_>>();
            assert_eq!(levels(&book), [0, 0]);
            assert!(book.step_list_level(false).is_err(), "outermost level has no parent");
            assert_eq!(book.step_list_level(true).unwrap(), 2);
            assert_eq!(levels(&book), [0, 1]);
            book.key(&Chord { name: "Z", ctrl: true, shift: false });
            assert_eq!(levels(&book), [0, 0], "one undo restores the previous level");
            for level in 2..=9 { assert_eq!(book.step_list_level(true).unwrap(), level); }
            assert!(book.step_list_level(true).is_err(), "ninth level is the limit");
            assert_eq!(levels(&book), [0, 8]);
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert_eq!(levels(&reopened), [0, 8], "{ext} lost the nested level");
            reopened.key(&Chord { name: "End", ctrl: true, shift: false });
            assert_eq!(reopened.step_list_level(false).unwrap(), 8);
            assert_eq!(levels(&reopened), [0, 7]);
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
    fn page_numbers_are_dynamic_and_round_trip_in_docx_and_odt() {
        use wordcraft_engine::doc::InlineObject;
        for ext in ["docx", "odt"] {
            let path = temp(&format!("page-numbers.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("Numbered pages").unwrap();
            for bad in ["", "bottom", "middle center", "bottom middle", "bottom center junk"] {
                assert!(book.insert_page_numbers(bad).is_err(), "{bad}");
            }
            let unnumbered = book.draw(0, 2.0).unwrap().pixels;
            book.insert_page_numbers("bottom right of").unwrap();
            assert_ne!(book.draw(0, 2.0).unwrap().pixels, unnumbered,
                "page number must appear on the rendered page");
            let fields = |b: &Book| {
                let id = b.session.doc.last_section.footers.default.expect("footer exists");
                b.session.doc.story(StoryRef::Part(id)).unwrap().iter()
                    .filter_map(|block| block.as_para())
                    .flat_map(|para| &para.objects)
                    .filter_map(|object| match object {
                        InlineObject::Field { instr, .. } => Some(instr.as_str()),
                        _ => None,
                    }).map(str::to_string).collect::<Vec<_>>()
            };
            assert_eq!(fields(&book), ["PAGE", "NUMPAGES"]);
            book.insert_page_numbers("bottom left").unwrap();
            assert_eq!(fields(&book), ["PAGE"], "reformatting must replace generated fields");
            book.save().unwrap();
            let reopened = Book::open(&path).unwrap();
            assert_eq!(fields(&reopened), ["PAGE"], "{ext} lost its dynamic page field");
        }
    }

    #[test]
    fn edit_header_footer_keeps_page_field_and_round_trips() {
        use wordcraft_engine::doc::InlineObject;
        for ext in ["docx", "odt"] {
            let path = temp(&format!("header-footer-edit.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("Body").unwrap();
            book.insert_page_numbers("bottom right").unwrap();
            let before = book.draw(0, 2.0).unwrap().pixels;
            book.edit_header_footer(false).unwrap();
            assert!(book.editing_header_footer());
            assert!(book.edit_header_footer(true).is_err(), "nested editing must keep the body return point");
            book.type_text(" Confidential").unwrap();
            book.close_header_footer().unwrap();
            assert!(!book.editing_header_footer());
            assert!(book.draw(0, 2.0).unwrap().pixels != before, "footer text must render");
            let edits = book.edits;
            book.edit_header_footer(true).unwrap();
            assert!(book.edits > edits, "creating a header must schedule autosave");
            book.type_text("Draft").unwrap();
            book.close_header_footer().unwrap();
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            let footer = reopened.session.doc.last_section.footers.default.unwrap();
            let blocks = reopened.session.doc.story(StoryRef::Part(footer)).unwrap();
            let para = blocks[0].as_para().unwrap();
            assert!(para.text.contains("Confidential"), "{ext} lost footer text");
            assert!(para.objects.iter().any(|obj| matches!(obj, InlineObject::Field { instr, .. } if instr == "PAGE")),
                "{ext} lost dynamic page field");
            let header = reopened.session.doc.last_section.headers.default.unwrap();
            assert!(reopened.session.doc.plain_text(StoryRef::Part(header)).contains("Draft"), "{ext} lost header text");
            assert!(reopened.draw(0, 2.0).is_some());
        }
    }

    #[test]
    fn footnote_is_linked_editable_and_round_trips() {
        use wordcraft_engine::doc::InlineObject;
        for ext in ["docx", "odt"] {
            let path = temp(&format!("footnote.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("A documented claim").unwrap();
            let before = book.draw(0, 1.5).unwrap().pixels;
            assert!(book.insert_footnote("").is_err());
            assert!(book.insert_footnote(&"x".repeat(501)).is_err());
            book.insert_footnote("Source one").unwrap();
            assert_eq!(book.editing_inserted_note(), Some("footnote"));
            book.type_text(" updated").unwrap();
            book.close_inserted_note();
            assert_eq!(book.session.sel.focus.story, StoryRef::Body);
            assert!(book.draw(0, 1.5).unwrap().pixels != before, "note must render on the page");
            let note = |b: &Book| {
                let id = b.session.doc.body.iter().filter_map(|block| block.as_para())
                    .flat_map(|p| &p.objects)
                    .find_map(|obj| match obj { InlineObject::NoteRef { id, .. } => Some(*id), _ => None })
                    .expect("linked note reference");
                assert_eq!(b.session.doc.parts.get(&id).unwrap().kind, PartKind::Footnote);
                b.session.doc.plain_text(StoryRef::Part(id))
            };
            assert!(note(&book).contains("Source one updated"));
            book.save().unwrap();
            let reopened = Book::open(&path).unwrap();
            assert!(note(&reopened).contains("Source one updated"), "{ext} lost footnote text or link");
        }
    }

    #[test]
    fn endnote_is_linked_editable_and_round_trips() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("endnote.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("A claim for the back matter").unwrap();
            assert!(book.insert_endnote("").is_err());
            assert!(book.insert_endnote(&"x".repeat(501)).is_err());
            book.insert_endnote("Reference one").unwrap();
            assert_eq!(book.editing_inserted_note(), Some("endnote"));
            book.type_text(" updated").unwrap();
            book.close_inserted_note();
            assert_eq!(book.session.sel.focus.story, StoryRef::Body);
            let note = |b: &Book| {
                let id = b.session.doc.body.iter().filter_map(|block| block.as_para())
                    .flat_map(|p| &p.objects)
                    .find_map(|obj| match obj { InlineObject::NoteRef { id, .. } => Some(*id), _ => None })
                    .expect("linked endnote reference");
                assert_eq!(b.session.doc.parts.get(&id).unwrap().kind, PartKind::Endnote);
                b.session.doc.plain_text(StoryRef::Part(id))
            };
            assert!(note(&book).contains("Reference one updated"));
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert!(note(&reopened).contains("Reference one updated"), "{ext} lost the endnote text or link");
            let last = reopened.pages().saturating_sub(1);
            assert!(reopened.draw(last, 1.5).is_some());
        }
    }

    #[test]
    fn saved_note_reference_reopens_for_editing() {
        for (ext, endnote) in [("docx", false), ("odt", true)] {
            let path = temp(&format!("reopen-note.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("Claim").unwrap();
            if endnote { book.insert_endnote("First").unwrap(); }
            else { book.insert_footnote("First").unwrap(); }
            book.close_inserted_note();
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert!(reopened.edit_note_at_caret().is_err());
            let off = reopened.session.doc.body[0].as_para().unwrap()
                .objects_with_offsets().find_map(|(off, obj)|
                    matches!(obj, InlineObject::NoteRef { .. }).then_some(off)).unwrap();
            reopened.session.sel = Selection::caret(Pos::body(0, off));
            let kind = reopened.edit_note_at_caret().unwrap();
            assert_eq!(kind, if endnote { "endnote" } else { "footnote" });
            assert_eq!(reopened.editing_inserted_note(), Some(kind));
            reopened.type_text("Updated ").unwrap();
            reopened.close_inserted_note();
            assert_eq!(reopened.session.sel.focus.story, StoryRef::Body);
            reopened.save().unwrap();
            let saved = Book::open(&path).unwrap();
            let id = saved.session.doc.body[0].as_para().unwrap().objects.iter()
                .find_map(|obj| match obj { InlineObject::NoteRef { id, .. } => Some(*id), _ => None }).unwrap();
            let text = saved.session.doc.plain_text(StoryRef::Part(id));
            assert!(text.contains("Updated") && text.contains("First"), "saved note: {text}");
        }
    }

    #[test]
    fn note_placement_and_numbering_round_trip() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("note-options.{ext}"));
            let mut book = Book::blank(&path);
            for invalid in ["footnote start 0", "footnote start 32768", "endnote restart page",
                "footnote position end", "endnote format unknown"] {
                assert!(book.set_note_options(invalid).is_err(), "{invalid}");
            }
            book.type_text("Claim").unwrap();
            book.insert_footnote("Source").unwrap();
            book.close_inserted_note();
            book.set_note_options("footnote start 5").unwrap();
            book.set_note_options("footnote restart page").unwrap();
            book.set_note_options("footnote format roman").unwrap();
            if ext == "docx" { book.set_note_options("footnote position beneath").unwrap(); }
            else {
                assert!(book.set_note_options("footnote position beneath").is_err());
                assert!(book.set_note_options("endnote position section").is_err());
                book.set_note_options("footnote position bottom").unwrap();
            }
            book.set_note_options("endnote start 3").unwrap();
            book.set_note_options("endnote restart section").unwrap();
            if ext == "docx" { book.set_note_options("endnote position section").unwrap(); }
            book.save().unwrap();
            let reopened = Book::open(&path).unwrap();
            let foot = reopened.session.doc.settings.footnote_pr;
            let end = reopened.session.doc.settings.endnote_pr;
            use wordcraft_engine::doc::section::{NotePos, NoteRestart, NumFormat};
            assert_eq!(foot.num_start, Some(5), "{ext}");
            assert_eq!(foot.num_restart, Some(NoteRestart::EachPage), "{ext}");
            assert_eq!(foot.pos, Some(if ext == "docx" { NotePos::BeneathText } else { NotePos::PageBottom }), "{ext}");
            assert_eq!(reopened.session.doc.settings.footnote_format, NumFormat::LowerRoman, "{ext}");
            assert_eq!(end.num_start, Some(3), "{ext}");
            assert_eq!(end.num_restart, Some(NoteRestart::EachSect), "{ext}");
            if ext == "docx" { assert_eq!(end.pos, Some(NotePos::SectEnd)); }
        }
    }

    #[test]
    fn note_conversion_keeps_links_text_and_undo_in_docx_and_odt() {
        use wordcraft_engine::doc::InlineObject;
        for ext in ["docx", "odt"] {
            let path = temp(&format!("converted-notes.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("First claim").unwrap();
            book.insert_footnote("First source").unwrap();
            book.close_inserted_note();
            book.type_text(" Second claim").unwrap();
            book.insert_endnote("Second source").unwrap();
            book.close_inserted_note();
            let notes = |b: &Book| {
                b.session.doc.body.iter().filter_map(|block| block.as_para())
                    .flat_map(|p| &p.objects)
                    .filter_map(|obj| match obj { InlineObject::NoteRef { id, .. } => Some(*id), _ => None })
                    .map(|id| {
                        let part = b.session.doc.parts.get(&id).expect("linked note part");
                        (part.kind, b.session.doc.plain_text(StoryRef::Part(id)))
                    }).collect::<Vec<_>>()
            };
            assert_eq!(notes(&book).len(), 2);
            assert!(book.convert_notes("invalid").is_err());
            assert_eq!(book.convert_notes("endnotes").unwrap(), 1);
            assert!(notes(&book).iter().all(|(kind, _)| *kind == PartKind::Endnote));
            book.key(&Chord { name: "Z", ctrl: true, shift: false });
            assert_eq!(notes(&book).iter().filter(|(kind, _)| *kind == PartKind::Footnote).count(), 1,
                "{ext} undo must restore the original kinds in one step");
            assert_eq!(book.convert_notes("swap").unwrap(), 2);
            let swapped = notes(&book);
            assert!(swapped.iter().any(|(kind, text)| *kind == PartKind::Endnote && text.contains("First source")));
            assert!(swapped.iter().any(|(kind, text)| *kind == PartKind::Footnote && text.contains("Second source")));
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert_eq!(notes(&reopened), swapped, "{ext} lost converted notes or their links");
            assert_eq!(reopened.convert_notes("footnote").unwrap(), 1);
            assert!(notes(&reopened).iter().all(|(kind, _)| *kind == PartKind::Footnote));
            assert!(reopened.convert_notes("footnote").is_err(), "no-op must not alter the document");
        }
    }

    #[test]
    fn hyperlinks_style_selection_and_round_trip() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("hyperlinks.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("Visit Assuit").unwrap();
            for bad in ["", "https://", "file:///private", "javascript:alert", "https://example.org\nInjected"] {
                assert!(book.insert_hyperlink(bad).is_err(), "{bad}");
            }
            book.key(&Chord { name: "Home", ctrl: false, shift: true });
            let links = |b: &Book| b.session.doc.body.iter().filter_map(|block| block.as_para())
                .flat_map(|p| &p.runs).filter_map(|run| run.props.link.clone()).collect::<Vec<_>>();
            book.insert_hyperlink("https://example.org").unwrap();
            assert!(links(&book).contains(&"https://example.org".to_string()));
            assert_eq!(book.text(), "Visit Assuit", "selected text stays visible");
            book.key(&Chord { name: "Z", ctrl: true, shift: false });
            assert!(links(&book).is_empty(), "one undo removes the link styling");
            book.insert_hyperlink("https://example.org").unwrap();
            book.key(&Chord { name: "End", ctrl: false, shift: false });
            book.type_text(" ").unwrap();
            book.insert_hyperlink("mailto:help@example.org Contact").unwrap();
            assert!(book.text().contains("Contact"));
            assert!(links(&book).contains(&"mailto:help@example.org".to_string()));
            book.save().unwrap();
            let reopened = Book::open(&path).unwrap();
            assert!(links(&reopened).contains(&"https://example.org".to_string()), "{ext} lost web link");
            assert!(links(&reopened).contains(&"mailto:help@example.org".to_string()), "{ext} lost mail link");
            assert!(reopened.text().contains("Contact"), "{ext} lost display label");
        }
    }

    #[test]
    fn selected_hyperlink_removal_keeps_text_and_round_trips() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("remove-link.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("Visit Assuit").unwrap();
            assert!(book.remove_hyperlink().is_err(), "a caret is not a link selection");
            book.key(&Chord { name: "Home", ctrl: false, shift: true });
            assert!(book.remove_hyperlink().is_err(), "plain selected text has no link");
            book.insert_hyperlink("https://example.org").unwrap();
            assert!(book.session.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.runs)
                .any(|r| r.props.link.as_deref() == Some("https://example.org")));
            book.remove_hyperlink().unwrap();
            assert_eq!(book.text(), "Visit Assuit");
            assert!(book.session.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.runs)
                .all(|r| r.props.link.is_none()));
            book.key(&Chord { name: "Z", ctrl: true, shift: false });
            assert!(book.session.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.runs)
                .any(|r| r.props.link.is_some()), "one undo restores the link");
            book.key(&Chord { name: "Y", ctrl: true, shift: false });
            book.save().unwrap();
            let reopened = Book::open(&path).unwrap();
            assert_eq!(reopened.text(), "Visit Assuit");
            assert!(reopened.session.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.runs)
                .all(|r| r.props.link.is_none()), "{ext} restored a removed link");
        }
    }

    #[test]
    fn caret_link_edit_selects_full_label_and_round_trips() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("edit-link.{ext}"));
            let mut book = Book::blank(&path);
            book.insert_hyperlink("https://old.example Linked name").unwrap();
            book.session.sel = Selection::caret(Pos::body(0, 4));
            assert_eq!(book.select_hyperlink_at_caret().as_deref(), Some("https://old.example"));
            let (from, to) = book.session.sel.ordered();
            assert_eq!((from.off, to.off), (0, "Linked name".len()));
            book.insert_hyperlink("https://new.example Revised name").unwrap();
            assert!(book.text().contains("Revised name"));
            assert!(book.session.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.runs)
                .any(|r| r.props.link.as_deref() == Some("https://new.example")));
            book.key(&Chord { name: "Z", ctrl: true, shift: false });
            assert!(book.text().contains("Linked name"), "one undo restores the old label");
            book.key(&Chord { name: "Y", ctrl: true, shift: false });
            book.save().unwrap();
            let reopened = Book::open(&path).unwrap();
            assert!(reopened.text().contains("Revised name"));
            assert!(reopened.session.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.runs)
                .any(|r| r.props.link.as_deref() == Some("https://new.example")), "{ext} lost the edited target");
        }
        let mut plain = Book::blank("plain.docx");
        plain.type_text("ordinary text").unwrap();
        plain.session.sel = Selection::caret(Pos::body(0, 4));
        assert!(plain.select_hyperlink_at_caret().is_none());
        assert!(plain.session.sel.is_collapsed());
    }

    #[test]
    fn contents_table_inserts_updates_and_round_trips() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("contents.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("Chapter One").unwrap();
            book.set_style("Heading1").unwrap();
            book.key(&Chord { name: "Enter", ctrl: false, shift: false });
            book.type_text("Body paragraph").unwrap();
            book.key(&Chord { name: "Home", ctrl: true, shift: false });
            for bad in ["", "insert", "insert 0", "insert 10", "insert many"] {
                assert!(book.table_of_contents(bad).is_err(), "{bad}");
            }
            assert!(book.table_of_contents("update").is_err(), "no table yet");
            let before = book.draw(0, 1.0).unwrap().pixels;
            book.table_of_contents("insert 2").unwrap();
            assert!(book.has_table_of_contents());
            assert!(book.text().contains("Contents") && book.text().matches("Chapter One").count() >= 2);
            assert!(book.draw(0, 1.0).unwrap().pixels != before, "contents must render");
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert!(reopened.has_table_of_contents(), "{ext} lost contents field");
            reopened.key(&Chord { name: "End", ctrl: true, shift: false });
            reopened.key(&Chord { name: "Enter", ctrl: false, shift: false });
            reopened.type_text("Chapter Two").unwrap();
            reopened.set_style("Heading2").unwrap();
            reopened.table_of_contents("update").unwrap();
            assert!(reopened.text().matches("Chapter Two").count() >= 2, "refresh must add new heading");
            reopened.save().unwrap();
            let mut refreshed = Book::open(&path).unwrap();
            assert!(refreshed.text().matches("Chapter Two").count() >= 2, "{ext} lost refreshed entry");
            refreshed.table_of_contents("remove").unwrap();
            assert!(!refreshed.has_table_of_contents());
            assert!(refreshed.text().contains("Chapter One") && refreshed.text().contains("Chapter Two"));
        }
    }

    #[test]
    fn generated_page_numbers_can_be_removed_without_erasing_other_content() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("page-number-remove.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("A numbered document").unwrap();
            assert!(book.insert_page_numbers("remove bottom").is_err());
            assert!(book.insert_page_numbers("remove middle").is_err());
            book.insert_page_numbers("bottom center of").unwrap();
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert!(reopened.session.doc.last_section.footers.default.is_some(), "{ext}");
            reopened.insert_page_numbers("remove bottom").unwrap();
            assert!(reopened.session.doc.last_section.footers.default.is_none());
            reopened.key(&Chord { name: "Z", ctrl: true, shift: false });
            assert!(reopened.session.doc.last_section.footers.default.is_some(), "undo restores footer");
            reopened.insert_page_numbers("remove bottom").unwrap();
            reopened.save().unwrap();
            assert!(Book::open(&path).unwrap().session.doc.last_section.footers.default.is_none(), "{ext}");

            reopened.insert_page_numbers("top right").unwrap();
            reopened.save().unwrap();
            let mut top = Book::open(&path).unwrap();
            assert!(top.session.doc.last_section.headers.default.is_some(), "{ext}");
            top.insert_page_numbers("remove top").unwrap();
            assert!(top.session.doc.last_section.headers.default.is_none(), "{ext}");

            top.insert_page_numbers("bottom left").unwrap();
            top.run("insert.footer", json!({ "text": "Confidential" })).unwrap();
            let id = top.session.doc.last_section.footers.default.unwrap();
            assert!(top.insert_page_numbers("remove bottom").is_err(), "author's footer must survive");
            let text = top.session.doc.story(StoryRef::Part(id)).unwrap()[0].as_para().unwrap().plain_text();
            assert_eq!(text, "Confidential");
        }
    }

    #[test]
    fn page_number_start_is_section_scoped_and_round_trips() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("page-number-start.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("First section").unwrap();
            book.insert_page_numbers("bottom center").unwrap();
            book.run("layout.break", json!({ "kind": "nextPage" })).unwrap();
            book.type_text("Second section").unwrap();
            assert_eq!(book.session.doc.sections().len(), 2);
            for bad in ["start", "start zero", "start 0", "start 32768", "start 2 extra"] {
                assert!(book.insert_page_numbers(bad).is_err(), "{bad}");
            }
            book.insert_page_numbers("start 5").unwrap();
            let starts = |b: &Book| b.session.doc.sections().into_iter()
                .map(|(_, sec)| sec.page_num_start).collect::<Vec<_>>();
            assert_eq!(starts(&book), [None, Some(5)]);
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert_eq!(starts(&reopened), [None, Some(5)], "{ext} lost section restart");
            reopened.key(&Chord { name: "End", ctrl: true, shift: false });
            reopened.insert_page_numbers("continue").unwrap();
            assert_eq!(starts(&reopened), [None, None]);
            reopened.save().unwrap();
            assert_eq!(starts(&Book::open(&path).unwrap()), [None, None], "{ext} lost continuation");
        }
    }

    #[test]
    fn page_number_format_is_visible_section_scoped_and_round_trips() {
        use wordcraft_engine::doc::section::NumFormat;
        for ext in ["docx", "odt"] {
            let path = temp(&format!("page-number-format.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("First section").unwrap();
            book.insert_page_numbers("bottom center").unwrap();
            for bad in ["format", "format binary", "format decimal extra"] {
                assert!(book.insert_page_numbers(bad).is_err(), "{bad}");
            }
            let decimal = book.draw(0, 2.0).unwrap().pixels;
            book.insert_page_numbers("format lower roman").unwrap();
            assert_eq!(book.session.doc.last_section.page_num_format, NumFormat::LowerRoman);
            assert_ne!(book.draw(0, 2.0).unwrap().pixels, decimal,
                "Roman page number must change the rendered page");
            book.insert_page_numbers("format decimal").unwrap();
            book.run("layout.break", json!({ "kind": "nextPage" })).unwrap();
            book.type_text("Second section").unwrap();
            book.insert_page_numbers("format upper letter").unwrap();
            let formats = |b: &Book| b.session.doc.sections().into_iter()
                .map(|(_, sec)| sec.page_num_format).collect::<Vec<_>>();
            assert_eq!(formats(&book), [NumFormat::Decimal, NumFormat::UpperLetter]);
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert_eq!(formats(&reopened), formats(&book), "{ext} lost section number formats");
            reopened.key(&Chord { name: "End", ctrl: true, shift: false });
            reopened.insert_page_numbers("format upper roman").unwrap();
            assert_eq!(formats(&reopened), [NumFormat::Decimal, NumFormat::UpperRoman]);
        }
    }

    #[test]
    fn first_and_even_page_number_layouts_round_trip() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("page-number-variants.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text("First page").unwrap();
            book.insert_page_numbers("bottom center").unwrap();
            book.run("layout.break", json!({ "kind": "page" })).unwrap();
            book.type_text("Second page").unwrap();
            assert!(book.pages() >= 2, "page break should make page two");
            for bad in ["first maybe", "even maybe", "first", "even"] {
                assert!(book.insert_page_numbers(bad).is_err(), "{bad}");
            }
            let first_before = book.draw(0, 2.0).unwrap().pixels;
            let even_before = book.draw(1, 2.0).unwrap().pixels;
            book.insert_page_numbers("first on").unwrap();
            assert!(book.session.doc.last_section.title_page);
            assert!(book.draw(0, 2.0).unwrap().pixels != first_before,
                "first page must stop using the default footer");
            book.key(&Chord { name: "Z", ctrl: true, shift: false });
            assert!(!book.session.doc.last_section.title_page, "one undo restores the default first page");
            book.insert_page_numbers("first on").unwrap();
            book.insert_page_numbers("even on").unwrap();
            assert!(book.session.doc.settings.even_odd_headers);
            // WordCraft uses the default footer on even pages until an even
            // footer exists, so fill the new slot while the caret is on page 2.
            book.insert_page_numbers("bottom right").unwrap();
            assert!(book.session.doc.last_section.footers.even.is_some());
            assert!(book.draw(1, 2.0).unwrap().pixels != even_before,
                "even page must show its own right-aligned page number");
            book.save().unwrap();
            let mut reopened = Book::open(&path).unwrap();
            assert!(reopened.session.doc.last_section.title_page, "{ext} lost first-page setting");
            assert!(reopened.session.doc.settings.even_odd_headers, "{ext} lost odd/even setting");
            assert!(reopened.session.doc.last_section.footers.even.is_some(), "{ext} lost even-page footer");
            reopened.insert_page_numbers("even off").unwrap();
            reopened.insert_page_numbers("first off").unwrap();
            assert!(!reopened.session.doc.last_section.title_page);
            assert!(!reopened.session.doc.settings.even_odd_headers);
            reopened.save().unwrap();
            let again = Book::open(&path).unwrap();
            assert!(!again.session.doc.last_section.title_page, "{ext} lost first-page reset");
            assert!(!again.session.doc.settings.even_odd_headers, "{ext} lost odd/even reset");
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
    fn exact_and_minimum_line_heights_round_trip() {
        use wordcraft_engine::doc::props::LineSpacing;
        for ext in ["docx", "odt"] {
            let path = temp(&format!("paragraph-line-height.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text(&"Lines of text should wrap naturally on a document page. ".repeat(18)).unwrap();
            let before = book.draw(0, 1.0).unwrap().pixels;
            for bad in ["exact 0 0 0", "minimum 73 0 0", "exact NaN 0 0", "minimum 18 -1 0", "exact 18 0", "approx 18 0 8"] {
                assert!(book.set_paragraph_spacing(bad).is_err(), "{bad}");
            }
            let spacing = |b: &Book| {
                let p = b.session.doc.body.iter().find_map(|block| block.as_para()).unwrap();
                (p.props.line_spacing, p.props.space_before, p.props.space_after)
            };
            book.set_paragraph_spacing("multiple 1.5 0 8").unwrap();
            assert_eq!(spacing(&book).0, Some(LineSpacing::Multiple(1.5)));
            book.set_paragraph_spacing("exact 24 6 12").unwrap();
            assert_eq!(spacing(&book), (Some(LineSpacing::Exactly(24.0)), Some(6.0), Some(12.0)));
            assert!(book.draw(0, 1.0).unwrap().pixels != before, "exact height must change page rendering");
            book.save().unwrap();
            let exact = Book::open(&path).unwrap();
            assert_eq!(spacing(&exact), spacing(&book), "{ext} lost exact height");
            book.set_paragraph_spacing("minimum 28 0 8").unwrap();
            assert_eq!(spacing(&book), (Some(LineSpacing::AtLeast(28.0)), Some(0.0), Some(8.0)));
            book.save().unwrap();
            let minimum = Book::open(&path).unwrap();
            assert_eq!(spacing(&minimum), spacing(&book), "{ext} lost minimum height");
        }
    }

    #[test]
    fn paragraph_indents_round_trip_and_undo() {
        for ext in ["docx", "odt"] {
            let path = temp(&format!("paragraph-indents.{ext}"));
            let mut book = Book::blank(&path);
            book.type_text(&"An indented paragraph wraps over several lines on the page. ".repeat(20)).unwrap();
            let before = book.draw(0, 1.0).unwrap().pixels;
            for bad in ["0.5 0", "4 0 0", "0 0 -0.25", "0.5 0 -1", "0.5 NaN 0", "3 3 0"] {
                assert!(book.set_paragraph_indents(bad).is_err(), "{bad}");
            }
            let indents = |b: &Book| {
                let p = b.session.doc.body.iter().find_map(|block| block.as_para()).unwrap();
                (p.props.indent_left, p.props.indent_right, p.props.indent_first)
            };
            book.set_paragraph_indents("0.5 0.25 -0.25").unwrap();
            assert_eq!(indents(&book), (Some(36.0), Some(18.0), Some(-18.0)));
            assert!(book.draw(0, 1.0).unwrap().pixels != before, "indent must change page layout");
            book.key(&Chord { name: "Z", ctrl: true, shift: false });
            assert_ne!(indents(&book), (Some(36.0), Some(18.0), Some(-18.0)), "one undo must restore prior indents");
            book.set_paragraph_indents("0.5 0.25 -0.25").unwrap();
            book.save().unwrap();
            let reopened = Book::open(&path).unwrap();
            assert_eq!(indents(&reopened), indents(&book), "{ext} lost paragraph indents");
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
    fn proofing_preference_survives_restart_and_failed_write_rolls_back() {
        let path = temp("proofing-pref.txt");
        let _ = std::fs::remove_file(&path);
        let mut book = Book::blank(&temp("proofing-pref.docx"));
        book.type_text("Hello helo world").unwrap();
        let text = book.text();
        let edits = book.edits;
        assert_eq!(book.restore_proofing_preference(&path).unwrap(), None);
        book.set_proofing_preference(&path, false).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"off\n");
        assert!(!book.proofing_marks());
        assert_eq!(book.text(), text);
        assert_eq!(book.edits, edits);
        let mut restarted = Book::blank(&temp("proofing-restarted.docx"));
        assert_eq!(restarted.restore_proofing_preference(&path).unwrap(), Some(false));
        assert!(!restarted.proofing_marks());
        assert!(restarted.set_proofing_preference(&temp("missing/pref"), true).is_err());
        assert!(!restarted.proofing_marks(), "failed write must not change the view");
        restarted.set_proofing_preference(&path, true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"on\n");
        let mut again = Book::blank(&temp("proofing-again.docx"));
        assert_eq!(again.restore_proofing_preference(&path).unwrap(), Some(true));
        std::fs::write(&path, b"nope\n").unwrap();
        assert!(again.restore_proofing_preference(&path).is_err());
        assert!(again.proofing_marks(), "invalid saved value must keep the current view");
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
    fn english_dialects_and_no_proofing_round_trip() {
        use wordcraft_engine::proof::ProofLang;
        assert_eq!(ProofLang::from_tag(Some("en-AU")), ProofLang::EnAu);
        assert_eq!(ProofLang::from_tag(Some("en-CA")), ProofLang::EnCa);
        assert_eq!(ProofLang::from_tag(Some("zxx")), ProofLang::Other);
        for ext in ["docx", "odt"] {
            for (choice, tag) in [("en-AU", "en-AU"), ("en-CA", "en-CA"), ("none", "zxx")] {
                let path = temp(&format!("proofing-{choice}.{ext}"));
                let mut book = Book::blank(&path);
                if ext == "odt" && choice == "none" {
                    let before = book.session.doc.styles.default_chr.lang.clone();
                    assert!(book.proofing_language(choice).is_err());
                    assert_eq!(book.session.doc.styles.default_chr.lang, before);
                    continue;
                }
                assert_eq!(book.proofing_language(choice).unwrap(), tag);
                assert_eq!(book.session.doc.styles.default_chr.lang.as_deref(), Some(tag));
                book.type_text("Colour and centre.").unwrap();
                book.save().unwrap();
                let reopened = Book::open(&path).unwrap();
                assert_eq!(reopened.session.doc.styles.default_chr.lang.as_deref(), Some(tag), "{choice}.{ext}");
            }
        }
        let mut book = Book::blank(&temp("proofing-invalid.docx"));
        let before = book.session.doc.styles.default_chr.lang.clone();
        assert!(book.proofing_language("de-DE").is_err());
        assert_eq!(book.session.doc.styles.default_chr.lang, before);
    }

    #[test]
    fn installed_german_dictionary_checks_words_and_round_trips() {
        let folder = std::env::temp_dir().join(format!("assuit-german-dictionary-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let invalid = folder.join("invalid");
        std::fs::create_dir_all(&invalid).unwrap();
        std::fs::write(invalid.join("de_DE.aff"), b"SET UTF-16\n").unwrap();
        std::fs::write(invalid.join("de_DE.dic"), b"1\nHaus\n").unwrap();
        let mut rejected = Book::blank(&temp("assuit-invalid-german.docx"));
        rejected.use_spell_dictionaries(&invalid).unwrap();
        let original = rejected.session.doc.styles.default_chr.lang.clone();
        assert!(rejected.proofing_language("de-DE").is_err());
        assert_eq!(rejected.session.doc.styles.default_chr.lang, original);
        std::fs::write(folder.join("de-DE.aff"), b"SET UTF-8\nTRY aehuskzt\n").unwrap();
        std::fs::write(folder.join("de-DE.dic"), b"2\nHaus\nKatze\n").unwrap();
        for ext in ["docx", "odt"] {
            let path = temp(&format!("assuit-german-proofing.{ext}"));
            let mut book = Book::blank(&path);
            assert!(book.proofing_language("de-DE").is_err(), "missing dictionary must not claim proofing");
            book.use_spell_dictionaries(&folder).unwrap();
            assert_eq!(book.proofing_language("de-de").unwrap(), "de-DE");
            assert!(wordcraft_engine::proof::is_correct_for("Haus", Some("de-DE")));
            assert!(!wordcraft_engine::proof::is_correct_for("Huas", Some("de-DE")));
            book.type_text("Haus Huas").unwrap();
            book.key(&Chord { name: "Home", ctrl: true, shift: false });
            assert_eq!(book.next_spelling_issue().unwrap().unwrap().0, "Huas");
            book.save().unwrap();
            let reopened = Book::open(&path).unwrap();
            assert_eq!(reopened.session.doc.styles.default_chr.lang.as_deref(), Some("de-DE"));
        }
        wordcraft_engine::proof::set_dictionaries_folder(None);
        std::fs::remove_dir_all(folder).unwrap();
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
