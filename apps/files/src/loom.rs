// SPDX-License-Identifier: GPL-3.0-only
//! Where the Loom engines (`sex-archive`, `loom-core`) meet Files: the state they keep,
//! and the few calls Files makes into them for keys, drawing and background work.
//!
//! Keys: Enter on an archive browses into it, Ctrl+Shift+X extracts, Ctrl+Shift+A packs
//! a zip; Ctrl+0..Ctrl+5 rate, Ctrl+Shift+G tags; Ctrl+Shift+R batch renames;
//! Ctrl+Shift+D finds duplicates; Ctrl+U maps disk usage; Ctrl+Shift+C compares folders.
//! Enter on a disk or CD image (`.iso`, `.img`, `.qcow2`, `.vhd(x)`, `.vmdk`, ...) browses it
//! the same way, its partitions as folders (see `diskimg`). Ctrl+Shift+P lists network
//! places (SFTP, WebDAV, S3; see `places`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::files::Pick;
use sex_app::Window;

use crate::{archive, compare, diskimg, places, dupes, meta, rename, shown, usage, App, BACKSPACE, ENTER, ESC};

// Letters and digits, PS/2 set 1, used with Ctrl (and Shift).
pub const KEY_A: u8 = 0x1E;
pub const KEY_X: u8 = 0x2D;
const KEY_R: u8 = 0x13;
const KEY_D: u8 = 0x20;
const KEY_C: u8 = 0x2E;
const KEY_G: u8 = 0x22;
const KEY_U: u8 = 0x16;
const KEY_P: u8 = 0x19;
/// `1`..`5` are 0x02..0x06; `0` is 0x0B.
const KEY_0: u8 = 0x0B;

pub enum Mode { None, Rename(rename::State), Dupes(dupes::State), Usage(usage::State) }
pub enum Prompt { Tags(Vec<u8>), Compare }

pub struct Loom {
    pub meta: meta::Meta,
    pub inside: Option<archive::Inside>,
    pub image: Option<diskimg::Inside>,
    pub places: places::Places,
    pub mode: Mode,
    pub prompt: Option<(Prompt, Vec<u8>)>,
    pub marks: Option<compare::Marks>,
}
impl Loom {
    pub fn new() -> Self { Loom { meta: meta::Meta::new(), inside: None, image: None, places: places::Places::default(), mode: Mode::None, prompt: None, marks: None } }
    /// Work is waiting between keys.
    pub fn busy(&self) -> bool {
        match &self.mode { Mode::Dupes(d) => d.busy(), Mode::Usage(u) => u.busy(), _ => false }
    }
}

impl App {
    /// A key, before Files' own handling. True when it was taken here.
    pub(crate) fn loom_key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) -> bool {
        if let Some((_, text)) = self.loom.prompt.as_mut() {
            match key.code {
                ENTER => {
                    let (p, text) = self.loom.prompt.take().unwrap();
                    self.status.clear();
                    match p { Prompt::Tags(path) => self.set_tags(window, &path, &text), Prompt::Compare => self.compare_with(window, &text) }
                    return true;
                }
                ESC => { self.loom.prompt = None; self.status.clear(); return true; }
                BACKSPACE => { while let Some(b) = text.pop() { if b & 0xC0 != 0x80 { break; } } }
                _ => if let Some(c) = sex_app::text(key) { if text.len() + c.len_utf8() <= 255 { text.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()); } },
            }
            self.prompt_status();
            return true;
        }
        match self.loom.mode {
            Mode::Rename(_) => { self.rename_key(window, key); return true; }
            Mode::Dupes(_) => { self.dupes_key(window, key); return true; }
            Mode::Usage(_) => { self.usage_key(window, key); return true; }
            Mode::None => {}
        }
        if self.loom.inside.is_some() { return self.archive_key(window, key); }
        if self.loom.image.is_some() { return self.image_key(window, key); }
        if self.loom.places.active() { return self.places_key(window, key); }
        if self.job.is_some() || self.search.is_some() { return false; }
        if key.control && key.shift {
            match key.code {
                KEY_R => self.rename_start(window),
                KEY_D => self.dupes_start(),
                KEY_C => { self.loom.prompt = Some((Prompt::Compare, Vec::new())); self.prompt_status(); }
                KEY_G => match self.selected() {
                    Some(p) => { let t = self.tags_text(&p); self.loom.prompt = Some((Prompt::Tags(p), t)); self.prompt_status(); }
                    None => self.status = "Select something to tag.".into(),
                },
                KEY_A => self.make_zip(window),
                KEY_X => self.extract_any(window),
                KEY_P => self.places_open(window),
                _ => return false,
            }
            return true;
        }
        if key.control {
            match key.code {
                KEY_U => self.usage_start(),
                0x02..=0x06 => self.rate(window, key.code - 1),
                KEY_0 => self.rate(window, 0),
                _ => return false,
            }
            return true;
        }
        // Enter on an archive goes into it (not while another program waits for a file to be picked).
        if key.code == ENTER && self.found.is_none() && !matches!(self.question.as_ref().map(|q| q.mode), Some(Pick::Open)) {
            if let Some(e) = self.entries.get(self.cursor) {
                if !e.dir && archive::is_archive(&e.name) {
                    let p = self.selected().unwrap();
                    self.archive_open(window, p);
                    return true;
                }
                if !e.dir && self.image_enter(window, &e.name.clone()) { return true; }
            }
        }
        false
    }
    fn prompt_status(&mut self) {
        let Some((p, text)) = &self.loom.prompt else { return };
        let label = match p {
            Prompt::Tags(path) => format!("Tags for {} (commas between)", shown(crate::base(path))),
            Prompt::Compare => String::from("Compare this folder with (a path)"),
        };
        self.status = format!("{}: {}|   (Enter confirms, Esc cancels)", label, shown(text));
    }
    /// Draws a Loom view in place of the listing. True when it did.
    pub(crate) fn loom_render(&mut self, window: &mut Window) -> bool {
        if let Mode::Usage(_) = self.loom.mode { return self.usage_render(window); }
        let text = match self.loom.mode {
            Mode::Rename(_) => self.rename_render(),
            Mode::Dupes(_) => {
                let mut t = self.dupes_render().unwrap_or_default();
                if !self.status.is_empty() { t.push_str(&self.status); t.push('\n'); }
                t.push_str("Arrows move  Del moves the selected copy to the trash  Esc back to the folder\n");
                Some(t)
            }
            _ => None,
        };
        let Some(mut t) = text else { return false };
        if self.showing_pixels { window.show_pixels(0, 0); self.showing_pixels = false; }
        if matches!(self.loom.mode, Mode::Rename(_)) && !self.status.is_empty() { t.push('\n'); t.push_str(&self.status); t.push('\n'); }
        window.show(t.as_bytes());
        true
    }
    /// What a row shows after its name: rating, tags, compare mark.
    pub(crate) fn loom_badge(&self, name: &[u8]) -> String {
        if self.loom.inside.is_some() || self.loom.image.is_some() || self.loom.places.active() { return String::new(); }
        let mut s = self.meta_badge(name);
        s.push_str(&self.compare_badge(name));
        s
    }
    /// One piece of background work. True when the window should be drawn again.
    pub(crate) fn loom_work(&mut self) -> bool {
        match self.loom.mode { Mode::Dupes(_) => self.dupes_step(), Mode::Usage(_) => self.usage_step(), _ => false }
    }
}
