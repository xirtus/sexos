// SPDX-License-Identifier: GPL-3.0-only
//! The File Browser dialog behind File > Open, Save As, Import and Export,
//! laid out like Blender's: a path bar, the folder's entries (folders first,
//! files filtered by the formats the action takes), a file name field and the
//! action and Cancel buttons.
//!
//! Keys: Up/Down move the highlight, Enter opens a folder or confirms,
//! Backspace edits the name (on an empty name it goes to the parent folder),
//! Ctrl+H shows hidden files, Esc cancels. A double click on a file confirms.
use crate::draw::{Align, Painter, Rect};
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use std::path::{Path, PathBuf};

/// What the dialog is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    Open,
    SaveAs,
    /// Import files of these extensions into the current scene.
    Import(Vec<String>),
    /// Export with this extension.
    Export(String),
}

impl Purpose {
    pub fn title(&self) -> String {
        match self {
            Purpose::Open => "Open".into(),
            Purpose::SaveAs => "Save As".into(),
            Purpose::Import(_) => "Import".into(),
            Purpose::Export(e) => format!("Export .{e}"),
        }
    }
    fn button(&self) -> &'static str {
        match self {
            Purpose::Open => "Open",
            Purpose::SaveAs => "Save As",
            Purpose::Import(_) => "Import",
            Purpose::Export(_) => "Export",
        }
    }
    fn saves(&self) -> bool {
        matches!(self, Purpose::SaveAs | Purpose::Export(_))
    }
    /// Extensions listed (empty: everything the suite opens).
    fn filter(&self) -> Vec<String> {
        match self {
            Purpose::Open => ["poly", "blend", "fbx", "glb", "gltf", "obj", "stl", "ply"].map(String::from).to_vec(),
            Purpose::SaveAs => ["poly", "blend"].map(String::from).to_vec(),
            Purpose::Import(v) => v.clone(),
            Purpose::Export(e) => vec![e.clone()],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

/// The confirmed choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
    pub purpose: Purpose,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct FileBrowser {
    pub purpose: Purpose,
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    /// Index into `entries` of the highlighted row.
    pub highlight: Option<usize>,
    pub name: String,
    pub show_hidden: bool,
    pub scroll: usize,
    pub error: Option<String>,
    last_click: Option<(usize, u32)>,
    clicks: u32,
}

const ROW: i32 = 22;

impl FileBrowser {
    /// A dialog in `dir`, the file name prefilled with `name`.
    pub fn new(purpose: Purpose, dir: &Path, name: &str) -> FileBrowser {
        let mut b = FileBrowser { purpose, dir: dir.to_path_buf(), entries: Vec::new(), highlight: None, name: name.into(), show_hidden: false, scroll: 0, error: None, last_click: None, clicks: 0 };
        b.refresh();
        b
    }

    pub fn refresh(&mut self) {
        let filter = self.purpose.filter();
        self.entries.clear();
        self.error = None;
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if !self.show_hidden && name.starts_with('.') {
                        continue;
                    }
                    let meta = e.metadata().ok();
                    let dir = meta.as_ref().is_some_and(|m| m.is_dir());
                    let ext = name.rsplit_once('.').map(|x| x.1.to_ascii_lowercase()).unwrap_or_default();
                    if !dir && !filter.is_empty() && !filter.contains(&ext) {
                        continue;
                    }
                    self.entries.push(Entry { name, dir, size: meta.map_or(0, |m| m.len()) });
                }
            }
            Err(e) => self.error = Some(format!("{}: {e}", self.dir.display())),
        }
        self.entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        self.highlight = None;
        self.scroll = 0;
    }

    fn enter(&mut self, name: &str) {
        self.dir = if name == ".." { self.dir.parent().map(Path::to_path_buf).unwrap_or_else(|| self.dir.clone()) } else { self.dir.join(name) };
        self.refresh();
    }

    /// The path the name field names (with the export extension added).
    fn target(&self) -> Option<PathBuf> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        let mut n = name.to_string();
        if let Purpose::Export(e) = &self.purpose {
            if !n.to_ascii_lowercase().ends_with(&format!(".{e}")) {
                n = format!("{n}.{e}");
            }
        }
        if self.purpose == Purpose::SaveAs && !n.contains('.') {
            n.push_str(".poly");
        }
        Some(if Path::new(&n).is_absolute() { PathBuf::from(n) } else { self.dir.join(n) })
    }

    fn confirm(&mut self) -> Option<Chosen> {
        if let Some(i) = self.highlight {
            let e = self.entries[i].clone();
            if e.dir {
                self.enter(&e.name);
                return None;
            }
        }
        let path = self.target()?;
        if path.is_dir() {
            self.dir = path;
            self.name.clear();
            self.refresh();
            return None;
        }
        if !self.purpose.saves() && !path.exists() {
            self.error = Some(format!("{} does not exist", path.display()));
            return None;
        }
        Some(Chosen { purpose: self.purpose.clone(), path })
    }

    fn select(&mut self, i: usize) {
        self.highlight = Some(i);
        if !self.entries[i].dir {
            self.name = self.entries[i].name.clone();
        }
    }

    fn layout(&self, p: &Painter) -> (Rect, Rect, Rect, Rect, Rect, Rect) {
        let b = p.bounds();
        let w = (b.w - 80).clamp(320, 900);
        let h = (b.h - 80).clamp(240, 640);
        let dlg = Rect::new(b.x + (b.w - w) / 2, b.y + (b.h - h) / 2, w, h);
        let inner = dlg.shrink(8);
        let (_title, rest) = inner.take_top(24);
        let (path, rest) = rest.take_top(26);
        let (bottom, rest) = rest.take_bottom(34);
        let list = Rect::new(rest.x, rest.y + 4, rest.w, rest.h - 8);
        let (buttons, field) = bottom.take_right(200);
        let field = Rect::new(field.x, bottom.y + 5, field.w - 8, 24);
        (dlg, path, list, field, Rect::new(buttons.x, buttons.y + 5, 96, 24), Rect::new(buttons.x + 104, buttons.y + 5, 96, 24))
    }

    fn visible_rows(list: &Rect) -> usize {
        (list.h / ROW).max(1) as usize
    }

    /// Feed an event. Returns (still open, the confirmed choice).
    pub fn handle(&mut self, p: &mut Painter, input: &Input) -> (bool, Option<Chosen>) {
        let (_, _, list, _, ok, cancel) = self.layout(p);
        let rows = Self::visible_rows(&list);
        let n = self.entries.len() + 1; // row 0 is ".."
        let row_entry = |r: usize| r.checked_sub(1);
        match *input {
            Input::Key { key: Key::Esc, .. } => return (false, None),
            Input::Key { key: Key::Enter | Key::NumEnter, .. } => {
                if self.highlight.is_none() && self.name.trim().is_empty() {
                    return (true, None);
                }
                if let Some(c) = self.confirm() {
                    return (false, Some(c));
                }
            }
            Input::Key { key: Key::Up, .. } => {
                let cur = self.highlight.map_or(0, |i| i + 1);
                let next = cur.saturating_sub(1);
                match row_entry(next) {
                    Some(i) => self.select(i),
                    None => self.highlight = None,
                }
                self.scroll = self.scroll.min(next);
            }
            Input::Key { key: Key::Down, .. } => {
                let cur = self.highlight.map_or(0, |i| i + 1);
                let next = (cur + 1).min(n - 1);
                if let Some(i) = row_entry(next) {
                    self.select(i);
                }
                if next >= self.scroll + rows {
                    self.scroll = next + 1 - rows;
                }
            }
            Input::Key { key: Key::Backspace, .. } => {
                if self.name.is_empty() {
                    self.enter("..");
                } else {
                    self.name.pop();
                }
            }
            Input::Key { key: Key::Char('h'), mods: Mods::CTRL, .. } => {
                self.show_hidden = !self.show_hidden;
                self.refresh();
            }
            Input::Key { text: Some(c), mods, .. } if !c.is_control() && !mods.ctrl => {
                self.name.push(c);
                self.highlight = None;
            }
            Input::Wheel { steps, .. } => {
                let max = n.saturating_sub(rows);
                self.scroll = (self.scroll as i64 - steps as i64 * 3).clamp(0, max as i64) as usize;
            }
            Input::Down { x, y, button: Button::Left, .. } => {
                if ok.contains(x, y) {
                    if let Some(c) = self.confirm() {
                        return (false, Some(c));
                    }
                } else if cancel.contains(x, y) {
                    return (false, None);
                } else if list.contains(x, y) {
                    let r = self.scroll + ((y - list.y) / ROW) as usize;
                    if r < n {
                        self.clicks += 1;
                        let double = self.last_click.is_some_and(|(lr, c)| lr == r && c + 1 == self.clicks);
                        self.last_click = Some((r, self.clicks));
                        match row_entry(r) {
                            None => self.enter(".."),
                            Some(i) if self.entries[i].dir => self.enter(&self.entries[i].name.clone()),
                            Some(i) => {
                                self.select(i);
                                if double {
                                    if let Some(c) = self.confirm() {
                                        return (false, Some(c));
                                    }
                                }
                            }
                        }
                    }
                } else {
                    self.last_click = None;
                }
            }
            _ => {}
        }
        (true, None)
    }

    pub fn draw(&self, p: &mut Painter, mouse: (i32, i32)) {
        let (dlg, path, list, field, ok, cancel) = self.layout(p);
        let all = p.bounds();
        p.fill_alpha(all, 0x000000, 110);
        p.round_rect(dlg, 6, t::MENU_BG);
        p.round_frame(dlg, 6, t::OUTLINE);
        p.label(Rect::new(dlg.x + 10, dlg.y + 6, dlg.w - 20, 22), &format!("Polyamour File View: {}", self.purpose.title()), t::TEXT_HI, Align::Left);
        p.round_rect(path, 3, t::BUTTON_DARK);
        p.label(path.shrink(4), &self.dir.display().to_string(), t::TEXT, Align::Left);
        p.fill(list, t::OUTLINER);
        p.push_clip(list);
        let rows = Self::visible_rows(&list);
        let n = self.entries.len() + 1;
        for r in self.scroll..n.min(self.scroll + rows + 1) {
            let y = list.y + (r - self.scroll) as i32 * ROW;
            let row = Rect::new(list.x, y, list.w, ROW);
            let (name, dir, size) = match r.checked_sub(1) {
                None => ("..".to_string(), true, 0),
                Some(i) => (self.entries[i].name.clone(), self.entries[i].dir, self.entries[i].size),
            };
            let hi = r.checked_sub(1) == self.highlight && r > 0;
            let bg = if hi {
                t::OUTLINER_SEL
            } else if row.contains(mouse.0, mouse.1) {
                t::BUTTON_DARK_HOVER
            } else if r % 2 == 1 {
                t::OUTLINER_ALT
            } else {
                t::OUTLINER
            };
            p.fill(row, bg);
            // Folder or file glyph.
            let gx = row.x + 8;
            let gy = row.y + 5;
            if dir {
                p.round_rect(Rect::new(gx, gy + 2, 14, 10), 2, 0xc8a15a);
                p.fill(Rect::new(gx, gy, 6, 3), 0xc8a15a);
            } else {
                p.round_rect(Rect::new(gx + 2, gy, 10, 13), 1, 0xb8b8b8);
            }
            p.label(Rect::new(row.x + 30, row.y, row.w - 130, ROW), &name, if dir { t::TEXT_HI } else { t::TEXT }, Align::Left);
            if !dir {
                p.label(Rect::new(row.right() - 100, row.y, 92, ROW), &human_size(size), t::TEXT_DIM, Align::Right);
            }
        }
        if let Some(e) = &self.error {
            p.label(Rect::new(list.x + 8, list.bottom() - ROW, list.w - 16, ROW), e, 0xff6060, Align::Left);
        } else if self.entries.is_empty() {
            p.label(Rect::new(list.x + 30, list.y + ROW, list.w - 40, ROW), "No matching files", t::TEXT_DIM, Align::Left);
        }
        p.pop_clip();
        p.round_rect(field, 3, t::BUTTON_DARK);
        let end = p.text(field.x + 6, field.y + (field.h - p.line_height()) / 2, &self.name, t::TEXT);
        p.fill(Rect::new(end + 1, field.y + 4, 1, field.h - 8), t::TEXT);
        for (r, label, color) in [(ok, self.purpose.button(), t::SELECTED), (cancel, "Cancel", t::BUTTON)] {
            let c = if r.contains(mouse.0, mouse.1) { if color == t::SELECTED { t::SELECTED_HOVER } else { t::BUTTON_HOVER } } else { color };
            p.round_rect(r, 4, c);
            p.label(r, label, t::TEXT_HI, Align::Center);
        }
    }
}

fn human_size(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GiB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KiB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("polyamour-fb-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.fbx"), b"x").unwrap();
        std::fs::write(d.join("b.txt"), b"x").unwrap();
        std::fs::write(d.join("C.glb"), b"x").unwrap();
        d
    }

    #[test]
    fn lists_folders_first_and_filters() {
        let d = tmpdir("list");
        let b = FileBrowser::new(Purpose::Open, &d, "");
        let names: Vec<&str> = b.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", "a.fbx", "C.glb"]);
        let b = FileBrowser::new(Purpose::Import(vec!["fbx".into()]), &d, "");
        assert_eq!(b.entries.len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn keys_pick_and_confirm() {
        let d = tmpdir("keys");
        let mut p = Painter::new(800, 600);
        let mut b = FileBrowser::new(Purpose::Open, &d, "");
        // Down twice: "sub" then "a.fbx".
        b.handle(&mut p, &Input::key(Key::Down));
        b.handle(&mut p, &Input::key(Key::Down));
        assert_eq!(b.name, "a.fbx");
        let (open, chosen) = b.handle(&mut p, &Input::key(Key::Enter));
        assert!(!open);
        assert_eq!(chosen.unwrap().path, d.join("a.fbx"));
        // Export adds the extension; typing goes to the name field.
        let mut b = FileBrowser::new(Purpose::Export("fbx".into()), &d, "");
        for c in "out".chars() {
            b.handle(&mut p, &Input::Key { key: Key::Char(c), mods: Mods::NONE, text: Some(c) });
        }
        let (_, chosen) = b.handle(&mut p, &Input::key(Key::Enter));
        assert_eq!(chosen.unwrap().path, d.join("out.fbx"));
        // Enter on a folder goes into it; Backspace on an empty name goes back up.
        let mut b = FileBrowser::new(Purpose::Open, &d, "");
        b.handle(&mut p, &Input::key(Key::Down));
        assert_eq!(b.handle(&mut p, &Input::key(Key::Enter)), (true, None));
        assert_eq!(b.dir, d.join("sub"));
        b.name.clear();
        b.handle(&mut p, &Input::key(Key::Backspace));
        assert_eq!(b.dir, d);
        let _ = std::fs::remove_dir_all(&d);
    }
}
