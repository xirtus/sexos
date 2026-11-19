// SPDX-License-Identifier: GPL-3.0-only
//! Commands, the shortcut map and menus.
//!
//! A program registers each thing it can do once, as a [`Command`] with an id, a label,
//! an optional shortcut and icon. Menus, the menu bar, context menus and the command
//! palette (Ctrl+Shift+P) all show commands, so a label or shortcut changes in one
//! place and every surface agrees. Running one, by any route, gives the program
//! [`crate::Response::Command`] with its id. People can rebind shortcuts with a keymap
//! text (`app.copy = Ctrl+Shift+C`).
use alloc::string::String;
use alloc::vec::Vec;

use crate::input::{Key, Shortcut};

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub shortcut: Option<Shortcut>,
    pub icon: Option<String>,
    pub enabled: bool,
    /// Shown with a check mark in menus when `Some(true)`.
    pub checked: Option<bool>,
    /// Extra words the palette matches ("preferences" for Settings).
    pub keywords: String,
    pub in_palette: bool,
}

impl Command {
    pub fn new(id: &str, label: &str) -> Command {
        Command { id: id.into(), label: label.into(), shortcut: None, icon: None, enabled: true, checked: None, keywords: String::new(), in_palette: true }
    }
    /// `Ctrl+S`, `F2`, `Ctrl+Shift+P`. A chord that doesn't parse is ignored.
    pub fn shortcut(mut self, chord: &str) -> Command { self.shortcut = Shortcut::parse(chord); self }
    pub fn icon(mut self, name: &str) -> Command { self.icon = Some(name.into()); self }
    pub fn keywords(mut self, words: &str) -> Command { self.keywords = words.into(); self }
    pub fn checked(mut self, on: bool) -> Command { self.checked = Some(on); self }
    pub fn hidden(mut self) -> Command { self.in_palette = false; self }
}

#[derive(Clone, Debug, Default)]
pub struct Commands { list: Vec<Command> }

impl Commands {
    /// Adds or replaces the command with this id.
    pub fn add(&mut self, c: Command) {
        match self.list.iter_mut().find(|x| x.id == c.id) { Some(x) => *x = c, None => self.list.push(c) }
    }
    pub fn get(&self, id: &str) -> Option<&Command> { self.list.iter().find(|c| c.id == id) }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Command> { self.list.iter_mut().find(|c| c.id == id) }
    pub fn all(&self) -> &[Command] { &self.list }
    pub fn set_enabled(&mut self, id: &str, on: bool) { if let Some(c) = self.get_mut(id) { c.enabled = on; } }
    pub fn set_checked(&mut self, id: &str, on: bool) { if let Some(c) = self.get_mut(id) { c.checked = Some(on); } }
    /// Gives `id` this shortcut and takes it from any command that had it.
    pub fn rebind(&mut self, id: &str, s: Option<Shortcut>) {
        if let Some(s) = s { for c in &mut self.list { if c.shortcut == Some(s) { c.shortcut = None; } } }
        if let Some(c) = self.get_mut(id) { c.shortcut = s; }
    }
    /// Reads `command.id = Chord` lines (`none` unbinds); returns the lines it couldn't use.
    pub fn load_keymap(&mut self, text: &str) -> Vec<usize> {
        let mut bad = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() { continue; }
            let Some((id, chord)) = line.split_once('=') else { bad.push(n + 1); continue };
            let (id, chord) = (id.trim(), chord.trim());
            if self.get(id).is_none() { bad.push(n + 1); continue; }
            if chord.eq_ignore_ascii_case("none") { self.rebind(id, None); continue; }
            match Shortcut::parse(chord) { Some(s) => self.rebind(id, Some(s)), None => bad.push(n + 1) }
        }
        bad
    }
    /// The enabled command a key runs.
    pub fn for_key(&self, key: &Key) -> Option<&Command> {
        self.list.iter().find(|c| c.enabled && c.shortcut.map_or(false, |s| s.matches(key)))
    }
    /// Palette matches for `query`, best first.
    pub fn search(&self, query: &str) -> Vec<usize> {
        let mut scored: Vec<(i32, usize)> = self.list.iter().enumerate()
            .filter(|(_, c)| c.in_palette)
            .filter_map(|(i, c)| {
                let label = fuzzy(query, &c.label);
                let words = if c.keywords.is_empty() { None } else { fuzzy(query, &c.keywords).map(|s| s - 5) };
                label.max(words).map(|s| (s - if c.enabled { 0 } else { 50 }, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, i)| i).collect()
    }
}

/// How well `query` matches `text` as a subsequence, ignoring case: higher is better,
/// with bonuses for letters at word starts and letters in a row. `None`: no match.
pub fn fuzzy(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() { return Some(0); }
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let mut score = 0;
    let mut ti = 0;
    let mut last: Option<usize> = None;
    for qc in query.chars().flat_map(char::to_lowercase) {
        if qc == ' ' { continue; }
        let found = (ti..t.len()).find(|&i| t[i] == qc)?;
        let start = found == 0 || !t[found - 1].is_alphanumeric();
        score += 1 + if start { 8 } else { 0 } + if last == Some(found.wrapping_sub(1)) { 5 } else { 0 };
        last = Some(found);
        ti = found + 1;
    }
    // Shorter labels win ties.
    Some(score * 4 - t.len() as i32 / 4)
}

#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    Command(String),
    Submenu(Menu),
    Separator,
}

/// A menu: a title (for the menu bar or a submenu) and items.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Menu {
    pub title: String,
    pub items: Vec<MenuItem>,
}

impl Menu {
    pub fn new(title: &str) -> Menu { Menu { title: title.into(), items: Vec::new() } }
    pub fn command(mut self, id: &str) -> Menu { self.items.push(MenuItem::Command(id.into())); self }
    pub fn submenu(mut self, m: Menu) -> Menu { self.items.push(MenuItem::Submenu(m)); self }
    pub fn separator(mut self) -> Menu { self.items.push(MenuItem::Separator); self }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_find_commands_and_keymaps_rebind() {
        let mut c = Commands::default();
        c.add(Command::new("app.save", "Save").shortcut("Ctrl+S"));
        c.add(Command::new("app.palette", "Palette").shortcut("Ctrl+Shift+P"));
        assert_eq!(c.for_key(&Key::ctrl(0x1F)).map(|c| c.id.as_str()), Some("app.save"));
        assert!(c.for_key(&Key::ctrl(0x19)).is_none());
        assert_eq!(c.load_keymap("app.save = Ctrl+Shift+S\nnope = F1\napp.palette = Ctrl+Bogus\n"), alloc::vec![2, 3]);
        assert!(c.for_key(&Key::ctrl(0x1F)).is_none());
        assert_eq!(c.for_key(&Key::ctrl(0x1F).shift()).map(|c| c.id.as_str()), Some("app.save"));
        c.set_enabled("app.save", false);
        assert!(c.for_key(&Key::ctrl(0x1F).shift()).is_none());
    }
    #[test]
    fn palette_search_prefers_word_starts() {
        let mut c = Commands::default();
        for (id, l) in [("a", "Close Tab"), ("b", "Copy"), ("c", "Toggle Comment"), ("d", "Close Other Tabs")] { c.add(Command::new(id, l)); }
        let r = c.search("ct");
        assert_eq!(c.all()[r[0]].label, "Close Tab");
        assert!(c.search("zzz").is_empty());
        assert_eq!(c.search("").len(), 4);
        c.add(Command::new("s", "Settings").keywords("preferences options"));
        assert_eq!(c.all()[c.search("pref")[0]].id, "s");
    }
}
