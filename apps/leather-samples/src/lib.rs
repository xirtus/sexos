// SPDX-License-Identifier: GPL-3.0-only
//! Leather SDK examples: state + Weft widgets + a separate capability-I/O layer.
#![no_std]
extern crate alloc;
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use weft::{Id, Layout, Response, TextStyle, Ui};

#[cfg(feature = "sexos")]
pub mod program;

pub const TEXT_LIMIT: usize = 64 * 1024;
/// A small line-at-a-time editor. Newlines live in the model, not in a custom
/// widget; Weft retains selection, keyboard editing, undo and accessibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub lines: Vec<String>,
    pub line: usize,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            lines: alloc::vec![String::new()],
            line: 0,
        }
    }
}
impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > TEXT_LIMIT {
            return Err("Document exceeds 64 KiB");
        }
        let text = core::str::from_utf8(bytes).map_err(|_| "Document is not UTF-8")?;
        if text.contains('\0') {
            return Err("Document contains NUL bytes");
        }
        Ok(Self {
            lines: text
                .split('\n')
                .map(|s| s.strip_suffix('\r').unwrap_or(s).to_string())
                .collect(),
            line: 0,
        })
    }
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
    pub fn update(&mut self, text: &str) -> bool {
        if text.contains(['\n', '\r', '\0'])
            || self.text().len() - self.lines[self.line].len() + text.len() > TEXT_LIMIT
        {
            return false;
        }
        self.lines[self.line] = text.into();
        true
    }
    pub fn next(&mut self) {
        self.line = (self.line + 1).min(self.lines.len() - 1);
    }
    pub fn previous(&mut self) {
        self.line = self.line.saturating_sub(1);
    }
    pub fn add(&mut self) -> bool {
        if self.text().len() == TEXT_LIMIT {
            return false;
        }
        self.line += 1;
        self.lines.insert(self.line, String::new());
        true
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hello,
    Stitch,
    Lantern,
    Postcard,
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hello => "hello",
            Self::Stitch => "stitch",
            Self::Lantern => "lantern",
            Self::Postcard => "postcard",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Hello => "Leather Greeting",
            Self::Stitch => "Leather Stitch",
            Self::Lantern => "Leather Lantern",
            Self::Postcard => "Leather Postcard",
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Load(String),
    Save(String, String),
    Image(String),
    Fetch(String),
}
pub struct Sample {
    pub kind: Kind,
    pub path: Id,
    pub content: Id,
    pub status: Id,
    pub primary: Id,
    pub secondary: Id,
    pub previous: Id,
    pub next: Id,
    pub add: Id,
    pub image: Id,
    pub document: Document,
    count: u32,
}
impl Sample {
    pub fn build(ui: &mut Ui, kind: Kind) -> Self {
        let root = ui.root();
        ui.set_layout(root, Layout::column().padding(12.0).gap(8.0));
        ui.text(root, kind.title(), TextStyle::Title);
        let path = ui.field(
            root,
            match kind {
                Kind::Stitch => "draft.txt",
                Kind::Postcard => "https://example.com/",
                _ => "",
            },
            "Filename or URL",
        );
        ui.set_layout(path, Layout::default().fill_width());
        ui.set_max_length(path, Some(1024));
        let row = ui.row(root);
        let primary = ui.button(
            row,
            match kind {
                Kind::Hello => "Greet",
                Kind::Stitch => "Save",
                Kind::Lantern => "Open picture",
                Kind::Postcard => "Fetch",
            },
        );
        let secondary = ui.button(row, "Load text");
        let previous = ui.button(row, "Previous line");
        let next = ui.button(row, "Next line");
        let add = ui.button(row, "Add line");
        for id in [secondary, previous, next, add] {
            ui.set_visible(id, kind == Kind::Stitch);
        }
        ui.set_visible(path, kind != Kind::Hello);
        let content = ui.field(root, "Hello from Leather", "Edit this line");
        ui.set_layout(content, Layout::default().fill_width());
        ui.set_max_length(content, Some(TEXT_LIMIT));
        ui.set_visible(content, kind == Kind::Stitch);
        let image = ui.image(root, 1, 1, alloc::vec![0x202020]);
        ui.set_visible(image, kind == Kind::Lantern);
        let status = ui.paragraph(
            root,
            match kind {
                Kind::Hello => "Press Greet to meet Weft.",
                Kind::Stitch => {
                    "Line 1. Save creates a new file; load an existing document before replacing it."
                }
                Kind::Lantern => "An original sample picture; enter a filename to open another.",
                Kind::Postcard => {
                    "Fetch up to 16 KiB of text. Start with run +net leather-postcard."
                }
            },
        );
        ui.set_layout(status, Layout::default().fill_width());
        ui.focus(Some(if kind == Kind::Hello { primary } else { path }));
        Self {
            kind,
            path,
            content,
            status,
            primary,
            secondary,
            previous,
            next,
            add,
            image,
            document: Document::parse(b"Hello from Leather").unwrap(),
            count: 0,
        }
    }
    pub fn loaded(&mut self, ui: &mut Ui, document: Document) {
        self.document = document;
        self.show_line(ui);
    }
    fn show_line(&self, ui: &mut Ui) {
        ui.set_text(self.content, &self.document.lines[self.document.line]);
        ui.set_text(
            self.status,
            &format!(
                "Line {} of {}",
                self.document.line + 1,
                self.document.lines.len()
            ),
        );
    }
    pub fn on(&mut self, ui: &mut Ui, response: &Response) -> Option<Action> {
        if *response == Response::Changed(self.content) {
            if !self.document.update(ui.text_of(self.content)) {
                self.show_line(ui);
                ui.set_text(self.status, "Document exceeds limits");
            }
        }
        if *response == Response::Clicked(self.primary)
            || *response == Response::Submitted(self.path)
        {
            let path = ui.text_of(self.path).trim().to_string();
            return match self.kind {
                Kind::Hello => {
                    self.count += 1;
                    ui.set_text(
                        self.status,
                        &format!("Hello, SexOS! Greeting {}", self.count),
                    );
                    None
                }
                Kind::Stitch => Some(Action::Save(path, self.document.text())),
                Kind::Lantern => Some(Action::Image(path)),
                Kind::Postcard => Some(Action::Fetch(path)),
            };
        }
        if *response == Response::Clicked(self.secondary) {
            return Some(Action::Load(ui.text_of(self.path).trim().into()));
        }
        if *response == Response::Clicked(self.previous) {
            self.document.previous();
            self.show_line(ui);
        }
        if *response == Response::Clicked(self.next) {
            self.document.next();
            self.show_line(ui);
        }
        if *response == Response::Clicked(self.add) {
            self.document.add();
            self.show_line(ui);
            ui.focus(Some(self.content));
        }
        None
    }
}

/// File operations used by the editor save policy. CREATE must atomically refuse
/// an existing name; only a loaded or just-created route may be replaced.
pub trait Store {
    fn create(&mut self, path: &str) -> Result<(), &'static str>;
    fn replace(&mut self, path: &str, text: &[u8]) -> Result<(), &'static str>;
    fn sync(&mut self) -> Result<(), &'static str>;
}
#[derive(Default)]
pub struct SaveState {
    loaded_path: Option<String>,
}
impl SaveState {
    /// Call only after the complete file has been read and parsed successfully.
    pub fn loaded(&mut self, path: &str) {
        self.loaded_path = Some(path.into());
    }
    pub fn save(
        &mut self,
        store: &mut impl Store,
        path: &str,
        text: &[u8],
    ) -> Result<(), &'static str> {
        if self.loaded_path.as_deref() != Some(path) {
            store.create(path)?;
            // A failed write/sync may leave our new partial file; a subsequent
            // save may retry it, but no unrelated existing file was truncated.
            self.loaded(path);
        }
        store.replace(path, text)?;
        store.sync()
    }
}
