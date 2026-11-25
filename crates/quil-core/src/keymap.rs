// SPDX-License-Identifier: GPL-3.0-only
//! Commands and the keys that run them. Every command has a name, so the command
//! palette can list it; keymaps (VS Code by default, Emacs) map key presses to commands,
//! and Vim mode lives in [`crate::vim`]. SexOS keys carry Ctrl and Shift but no Alt, so
//! VS Code's Alt chords move to Ctrl+Shift, and Emacs Meta is Escape-then-key, as Emacs
//! itself allows.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named { Enter, Esc, Backspace, Delete, Tab, Left, Right, Up, Down, Home, End, PageUp, PageDown, F(u8) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum K { Char(char), Named(Named) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press { pub k: K, pub ctrl: bool, pub shift: bool }
impl Press {
    pub const fn ch(c: char) -> Press { Press { k: K::Char(c), ctrl: false, shift: false } }
    pub const fn ctrl(c: char) -> Press { Press { k: K::Char(c), ctrl: true, shift: false } }
    pub const fn ctrl_shift(c: char) -> Press { Press { k: K::Char(c), ctrl: true, shift: true } }
    pub const fn named(n: Named) -> Press { Press { k: K::Named(n), ctrl: false, shift: false } }
}

macro_rules! commands {
    ($($id:ident => $name:expr),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Command { $($id),* }
        impl Command {
            pub const ALL: &'static [Command] = &[$(Command::$id),*];
            pub fn name(self) -> &'static str { match self { $(Command::$id => $name),* } }
        }
    };
}

commands! {
    Save => "File: Save",
    SaveAs => "File: Save As",
    Open => "File: Open",
    OpenFolder => "File: Open Folder",
    NewTab => "File: New Tab",
    CloseTab => "View: Close Tab",
    NextTab => "View: Next Tab",
    PrevTab => "View: Previous Tab",
    Quit => "Quil: Quit",
    Undo => "Edit: Undo",
    Redo => "Edit: Redo",
    Copy => "Edit: Copy",
    Cut => "Edit: Cut",
    Paste => "Edit: Paste",
    SelectAll => "Selection: Select All",
    Find => "Find: Find in File",
    FindNext => "Find: Next Match",
    FindPrev => "Find: Previous Match",
    Replace => "Find: Replace in File",
    GotoLine => "Go: Go to Line",
    Palette => "View: Command Palette",
    QuickOpen => "Go: Go to File",
    ProjectSearch => "Search: Find in Folder",
    ToggleTree => "View: Toggle File Tree",
    ToggleNumbers => "View: Toggle Line Numbers",
    AddCursorAbove => "Selection: Add Cursor Above",
    AddCursorBelow => "Selection: Add Cursor Below",
    AddNextOccurrence => "Selection: Add Next Occurrence",
    SelectAllOccurrences => "Selection: Select All Occurrences",
    SplitIntoLines => "Selection: Cursor at Each Line",
    Collapse => "Selection: Single Cursor",
    Indent => "Edit: Indent Lines",
    Outdent => "Edit: Outdent Lines",
    ToggleComment => "Edit: Toggle Line Comment",
    DuplicateLine => "Edit: Duplicate Line",
    DeleteLine => "Edit: Delete Line",
    MoveLineUp => "Edit: Move Line Up",
    MoveLineDown => "Edit: Move Line Down",
    JoinLines => "Edit: Join Lines",
    DeleteWordBack => "Edit: Delete Word Backwards",
    KeymapVsCode => "Preferences: VS Code Keys",
    KeymapVim => "Preferences: Vim Mode",
    KeymapEmacs => "Preferences: Emacs Keys",
    // Movement, mostly for Emacs keys.
    Left => "Cursor: Left", Right => "Cursor: Right", Up => "Cursor: Up", Down => "Cursor: Down",
    WordLeft => "Cursor: Word Left", WordRight => "Cursor: Word Right",
    LineStart => "Cursor: Line Start", LineEnd => "Cursor: Line End",
    DocStart => "Cursor: Top", DocEnd => "Cursor: Bottom",
    PageUp => "Cursor: Page Up", PageDown => "Cursor: Page Down",
    KillLine => "Edit: Kill to Line End",
    DeleteForward => "Edit: Delete Forward",
    SetMark => "Selection: Set Mark",
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keymap { VsCode, Vim, Emacs }

/// VS Code's default keys (Alt chords as Ctrl+Shift).
const VSCODE: &[(Press, Command)] = &[
    (Press::ctrl('s'), Command::Save), (Press::ctrl_shift('s'), Command::SaveAs), (Press::ctrl('o'), Command::Open),
    (Press::ctrl_shift('o'), Command::OpenFolder), (Press::ctrl('n'), Command::NewTab), (Press::ctrl('w'), Command::CloseTab),
    (Press { k: K::Named(Named::PageDown), ctrl: true, shift: false }, Command::NextTab),
    (Press { k: K::Named(Named::PageUp), ctrl: true, shift: false }, Command::PrevTab),
    (Press::ctrl('q'), Command::Quit),
    (Press::ctrl('z'), Command::Undo), (Press::ctrl('y'), Command::Redo), (Press::ctrl_shift('z'), Command::Redo),
    (Press::ctrl('c'), Command::Copy), (Press::ctrl('x'), Command::Cut), (Press::ctrl('v'), Command::Paste),
    (Press::ctrl('a'), Command::SelectAll), (Press::ctrl('f'), Command::Find), (Press::ctrl('g'), Command::FindNext),
    (Press::ctrl_shift('g'), Command::FindPrev), (Press::ctrl('h'), Command::Replace), (Press::ctrl('l'), Command::GotoLine),
    (Press::ctrl_shift('p'), Command::Palette), (Press::ctrl('p'), Command::QuickOpen), (Press::ctrl_shift('f'), Command::ProjectSearch),
    (Press::ctrl('b'), Command::ToggleTree),
    (Press { k: K::Named(Named::Up), ctrl: true, shift: true }, Command::AddCursorAbove),
    (Press { k: K::Named(Named::Down), ctrl: true, shift: true }, Command::AddCursorBelow),
    (Press::ctrl('d'), Command::AddNextOccurrence), (Press::ctrl_shift('l'), Command::SelectAllOccurrences),
    (Press::ctrl_shift('i'), Command::SplitIntoLines),
    (Press::ctrl(']'), Command::Indent), (Press::ctrl('['), Command::Outdent), (Press::ctrl('/'), Command::ToggleComment),
    (Press::ctrl_shift('d'), Command::DuplicateLine), (Press::ctrl_shift('k'), Command::DeleteLine),
    (Press { k: K::Named(Named::Up), ctrl: true, shift: false }, Command::MoveLineUp),
    (Press { k: K::Named(Named::Down), ctrl: true, shift: false }, Command::MoveLineDown),
    (Press::ctrl('j'), Command::JoinLines),
    (Press { k: K::Named(Named::Backspace), ctrl: true, shift: false }, Command::DeleteWordBack),
    (Press { k: K::Named(Named::Home), ctrl: true, shift: false }, Command::DocStart),
    (Press { k: K::Named(Named::End), ctrl: true, shift: false }, Command::DocEnd),
];

/// Emacs keys; `M-x` is Escape then `x`.
const EMACS: &[(Press, Command)] = &[
    (Press::ctrl('f'), Command::Right), (Press::ctrl('b'), Command::Left), (Press::ctrl('n'), Command::Down), (Press::ctrl('p'), Command::Up),
    (Press::ctrl('a'), Command::LineStart), (Press::ctrl('e'), Command::LineEnd), (Press::ctrl('v'), Command::PageDown),
    (Press::ctrl('d'), Command::DeleteForward), (Press::ctrl('k'), Command::KillLine), (Press::ctrl('y'), Command::Paste),
    (Press::ctrl('w'), Command::Cut), (Press::ctrl('s'), Command::Find), (Press::ctrl('r'), Command::FindPrev),
    (Press::ctrl('/'), Command::Undo), (Press::ctrl('_'), Command::Undo), (Press::ctrl(' '), Command::SetMark), (Press::ctrl('g'), Command::Collapse),
    (Press::ctrl('l'), Command::GotoLine),
];
const EMACS_META: &[(char, Command)] = &[
    ('f', Command::WordRight), ('b', Command::WordLeft), ('w', Command::Copy), ('x', Command::Palette), ('v', Command::PageUp),
    ('<', Command::DocStart), ('>', Command::DocEnd), ('%', Command::Replace), ('g', Command::GotoLine), (';', Command::ToggleComment),
];
const EMACS_CX: &[(Press, Command)] = &[
    (Press::ctrl('s'), Command::Save), (Press::ctrl('w'), Command::SaveAs), (Press::ctrl('f'), Command::Open), (Press::ctrl('c'), Command::Quit),
    (Press::ch('k'), Command::CloseTab), (Press::ch('b'), Command::QuickOpen), (Press::ch('o'), Command::NextTab), (Press::ch('u'), Command::Undo),
    (Press::ch('h'), Command::SelectAll), (Press::ctrl('d'), Command::OpenFolder),
];

/// Turns presses into commands, holding Emacs prefixes (`C-x`, Escape) between presses.
#[derive(Default)]
pub struct Keys { prefix: Option<Prefix> }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Prefix { CtrlX, Meta }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup { Run(Command), Pending, Unbound }

impl Keys {
    pub fn pending(&self) -> Option<&'static str> {
        self.prefix.map(|p| match p { Prefix::CtrlX => "C-x-", Prefix::Meta => "M-" })
    }
    pub fn lookup(&mut self, map: Keymap, p: Press) -> Lookup {
        fn same(a: &Press, b: &Press) -> bool {
            let fold = |k: K| if let K::Char(c) = k { K::Char(c.to_ascii_lowercase()) } else { k };
            fold(a.k) == fold(b.k) && a.ctrl == b.ctrl && a.shift == b.shift
        }
        match map {
            Keymap::VsCode | Keymap::Vim => {
                // Vim mode passes only the keys it doesn't use (Ctrl chords) to here.
                VSCODE.iter().find(|(k, _)| same(k, &p)).map_or(Lookup::Unbound, |(_, c)| Lookup::Run(*c))
            }
            Keymap::Emacs => {
                if let Some(pre) = self.prefix.take() {
                    return match pre {
                        Prefix::CtrlX => EMACS_CX.iter().find(|(k, _)| same(k, &p)).map_or(Lookup::Unbound, |(_, c)| Lookup::Run(*c)),
                        Prefix::Meta => match p.k { K::Char(ch) => EMACS_META.iter().find(|(k, _)| *k == ch).map_or(Lookup::Unbound, |(_, c)| Lookup::Run(*c)), _ => Lookup::Unbound },
                    };
                }
                if p == Press::ctrl('x') { self.prefix = Some(Prefix::CtrlX); return Lookup::Pending; }
                if p.k == K::Named(Named::Esc) && !p.ctrl { self.prefix = Some(Prefix::Meta); return Lookup::Pending; }
                if let Some((_, c)) = EMACS.iter().find(|(k, _)| same(k, &p)) { return Lookup::Run(*c); }
                // Ctrl+Shift chords keep their VS Code meaning (palette, multi-cursor).
                if p.ctrl && p.shift { return VSCODE.iter().find(|(k, _)| same(k, &p)).map_or(Lookup::Unbound, |(_, c)| Lookup::Run(*c)); }
                Lookup::Unbound
            }
        }
    }
    /// The key shown beside a command in the palette.
    pub fn binding(map: Keymap, c: Command) -> Option<Press> {
        let table = if map == Keymap::Emacs { EMACS } else { VSCODE };
        table.iter().find(|(_, x)| *x == c).map(|(k, _)| *k)
    }
}

pub fn describe(p: Press) -> alloc::string::String {
    let mut s = alloc::string::String::new();
    if p.ctrl { s.push_str("Ctrl+"); }
    if p.shift { s.push_str("Shift+"); }
    match p.k {
        K::Char(' ') => s.push_str("Space"),
        K::Char(c) => s.push(c.to_ascii_uppercase()),
        K::Named(n) => s.push_str(match n {
            Named::Enter => "Enter", Named::Esc => "Esc", Named::Backspace => "Backspace", Named::Delete => "Delete", Named::Tab => "Tab",
            Named::Left => "Left", Named::Right => "Right", Named::Up => "Up", Named::Down => "Down", Named::Home => "Home", Named::End => "End",
            Named::PageUp => "PageUp", Named::PageDown => "PageDown", Named::F(_) => "F",
        }),
    }
    s
}

/// The compact, scrollable reference shown inside Quil. Keep bindings sourced from
/// the active keymap rather than maintaining a second list of key spellings.
pub fn shortcut_rows(map: Keymap) -> alloc::vec::Vec<alloc::string::String> {
    use alloc::{format, string::String, vec::Vec};
    let mut rows = Vec::new();
    rows.push(String::from("Ctrl+Shift+/  Help   Up/Down scroll   Esc close"));
    for command in [
        Command::Save, Command::SaveAs, Command::Open, Command::OpenFolder,
        Command::NewTab, Command::CloseTab, Command::NextTab, Command::PrevTab,
        Command::Undo, Command::Redo, Command::Find, Command::Replace,
        Command::QuickOpen, Command::ProjectSearch, Command::Palette,
        Command::ToggleTree, Command::AddCursorBelow, Command::AddNextOccurrence,
        Command::SelectAllOccurrences, Command::ToggleComment,
    ] {
        let key = if map == Keymap::Emacs {
            EMACS_CX.iter().find(|(_, c)| *c == command).map(|(p, _)| format!("Ctrl+X {}", describe(*p)))
                .or_else(|| EMACS_META.iter().find(|(_, c)| *c == command).map(|(c, _)| format!("Esc {}", c.to_ascii_uppercase())))
                .or_else(|| EMACS.iter().find(|(_, c)| *c == command).map(|(p, _)| describe(*p)))
                .or_else(|| VSCODE.iter().find(|(p, c)| p.ctrl && p.shift && *c == command).map(|(p, _)| describe(*p)))
        } else {
            VSCODE.iter().find(|(_, c)| *c == command).map(|(p, _)| describe(*p))
        };
        if let Some(key) = key { rows.push(format!("{key:<20} {}", command.name())); }
    }
    if map == Keymap::Vim {
        rows.push(String::from("Esc  Normal mode   i  Insert   :w  Save"));
        rows.push(String::from("dd  Delete line   /  Search   u  Undo"));
    }
    rows
}

#[cfg(test)]
mod shortcut_help_tests {
    use super::*;
    #[test]
    fn help_uses_active_bindings_and_includes_close_hint() {
        let vs = shortcut_rows(Keymap::VsCode).join("\n");
        assert!(vs.contains("Ctrl+S"));
        assert!(vs.contains("File: Save"));
        assert!(vs.contains("Ctrl+Shift+/  Help"));
        let emacs = shortcut_rows(Keymap::Emacs).join("\n");
        assert!(emacs.contains("Ctrl+X Ctrl+S"));
        assert!(emacs.contains("Esc X"));
        assert!(!emacs.contains("Ctrl+P                Go: Go to File"));
        let vim = shortcut_rows(Keymap::Vim).join("\n");
        assert!(vim.contains(":w  Save"));
    }
}
