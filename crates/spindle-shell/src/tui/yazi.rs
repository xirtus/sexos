// SPDX-License-Identifier: GPL-3.0-only
//! Interactive 3-pane Ratatui `yazi` / `ranger` file manager (`\x1b[?1049h` alt-screen).
//! Mirrors Void `.zshrc`'s `function y()`: pressing `q` exits and changes Spindle's cwd
//! to the directory you navigated to (`TuiExit::Cd`), while `Esc` cancels cleanly.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::widgets::{
    self, AMBER, BLUE, BORDER, CORAL, CYAN, EMERALD, FROST, LAVENDER, MUTE, ROSE, SEL_BG,
};
use super::{TuiExit, TuiKey, TuiOutput};
use crate::ergonomics::{file_glyph, human_size, tint_source_line};
use crate::host::{DirEntry, EntryKind, Host};
use crate::path;
use crate::width::truncate;

enum Preview {
    Empty,
    Dir(Vec<DirEntry>),
    Text { lines: Vec<String>, ext: String, total_bytes: usize },
    Binary { hex_lines: Vec<String>, total_bytes: usize },
    Err(String),
}

/// Interactive 3-pane Yazi file browser & previewer.
pub struct YaziApp {
    cwd: String,
    show_hidden: bool,
    selected: usize,
    scroll: usize,
    parent_entries: Vec<DirEntry>,
    entries: Vec<DirEntry>,
    preview: Preview,
    frame: usize,
}

impl YaziApp {
    pub fn new(host: &dyn Host, start_arg: Option<&str>) -> Result<Self, String> {
        let start = match start_arg {
            Some(arg) if !arg.is_empty() => path::resolve(&host.cwd(), &host.home(), arg),
            _ => host.cwd(),
        };
        let st = host.stat(&start)?;
        if st.kind != EntryKind::Dir && start != "/" {
            return Err(format!("`{}` is not a folder", start));
        }
        let mut app = Self {
            cwd: start,
            show_hidden: false,
            selected: 0,
            scroll: 0,
            parent_entries: Vec::new(),
            entries: Vec::new(),
            preview: Preview::Empty,
            frame: 0,
        };
        app.reload(host, None);
        Ok(app)
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn init(&mut self, cols: usize, rows: usize) -> TuiOutput {
        let mut s = String::from("\x1b[?1049h\x1b[?25l");
        s.push_str(&self.render(cols, rows));
        TuiOutput::frame(s)
    }

    pub fn tick(&mut self, _cols: usize, _rows: usize, _now_ms: u64) -> TuiOutput {
        self.frame = self.frame.wrapping_add(1);
        TuiOutput::none()
    }

    pub fn key(&mut self, key: TuiKey, host: &dyn Host, cols: usize, rows: usize) -> TuiOutput {
        match key {
            TuiKey::Char('q') => {
                return TuiOutput::finish(
                    String::from("\x1b[?25h\x1b[?1049l"),
                    TuiExit::Cd(self.cwd.clone()),
                );
            }
            TuiKey::Escape | TuiKey::CtrlC => {
                return TuiOutput::finish(String::from("\x1b[?25h\x1b[?1049l"), TuiExit::Cancel);
            }
            TuiKey::Char('j') | TuiKey::Down => {
                if !self.entries.is_empty() && self.selected + 1 < self.entries.len() {
                    self.selected += 1;
                    self.refresh_preview(host);
                }
            }
            TuiKey::Char('k') | TuiKey::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                    self.refresh_preview(host);
                }
            }
            TuiKey::Char('g') | TuiKey::Home => {
                self.selected = 0;
                self.scroll = 0;
                self.refresh_preview(host);
            }
            TuiKey::Char('G') | TuiKey::End => {
                if !self.entries.is_empty() {
                    self.selected = self.entries.len() - 1;
                    self.refresh_preview(host);
                }
            }
            TuiKey::PageDown => {
                if !self.entries.is_empty() {
                    self.selected = (self.selected + 10).min(self.entries.len() - 1);
                    self.refresh_preview(host);
                }
            }
            TuiKey::PageUp => {
                self.selected = self.selected.saturating_sub(10);
                self.refresh_preview(host);
            }
            TuiKey::Char('h') | TuiKey::Left => {
                if self.cwd != "/" {
                    let (parent, leaf) = path::split(&self.cwd);
                    let keep = String::from(leaf);
                    self.cwd = if parent.is_empty() { String::from("/") } else { String::from(parent) };
                    self.reload(host, Some(&keep));
                }
            }
            TuiKey::Char('l') | TuiKey::Right | TuiKey::Enter => {
                if let Some(entry) = self.entries.get(self.selected).cloned() {
                    if entry.kind == EntryKind::Dir {
                        self.cwd = path::join(&self.cwd, &entry.name);
                        self.reload(host, None);
                    }
                }
            }
            TuiKey::Char('.') | TuiKey::Char('H') => {
                let keep = self.entries.get(self.selected).map(|e| e.name.clone());
                self.show_hidden = !self.show_hidden;
                self.reload(host, keep.as_deref());
            }
            TuiKey::Char('r') => {
                let keep = self.entries.get(self.selected).map(|e| e.name.clone());
                self.reload(host, keep.as_deref());
            }
            _ => return TuiOutput::none(),
        }
        TuiOutput::frame(self.render(cols, rows))
    }

    fn reload(&mut self, host: &dyn Host, select_name: Option<&str>) {
        self.entries = load_sorted_dir(&self.cwd, self.show_hidden, host).unwrap_or_default();
        self.parent_entries = if self.cwd == "/" {
            Vec::new()
        } else {
            let (parent, _) = path::split(&self.cwd);
            let parent_abs = if parent.is_empty() { "/" } else { parent };
            load_sorted_dir(parent_abs, self.show_hidden, host).unwrap_or_default()
        };
        self.selected = select_name
            .and_then(|name| self.entries.iter().position(|e| e.name == name))
            .unwrap_or(0);
        self.scroll = 0;
        self.refresh_preview(host);
    }

    fn refresh_preview(&mut self, host: &dyn Host) {
        let Some(entry) = self.entries.get(self.selected) else {
            self.preview = Preview::Empty;
            return;
        };
        let target = path::join(&self.cwd, &entry.name);
        if entry.kind == EntryKind::Dir {
            match load_sorted_dir(&target, self.show_hidden, host) {
                Ok(items) => self.preview = Preview::Dir(items),
                Err(e) => self.preview = Preview::Err(e),
            }
            return;
        }
        match host.read_file(&target) {
            Ok(bytes) => {
                let total_bytes = bytes.len();
                let slice = &bytes[..total_bytes.min(32 * 1024)];
                if let Ok(text) = core::str::from_utf8(slice) {
                    let ext = path::extension(&entry.name);
                    let lines: Vec<String> = text.lines().take(60).map(String::from).collect();
                    self.preview = Preview::Text { lines, ext, total_bytes };
                } else {
                    self.preview = Preview::Binary { hex_lines: format_hex(&bytes[..total_bytes.min(256)]), total_bytes };
                }
            }
            Err(e) => self.preview = Preview::Err(e),
        }
    }

    pub fn render(&mut self, cols: usize, rows: usize) -> String {
        let w = cols.max(40);
        let h = rows.max(12);
        let body_h = h.saturating_sub(4).max(3);
        let three_pane = w >= 72;

        if !self.entries.is_empty() {
            if self.selected < self.scroll {
                self.scroll = self.selected;
            } else if self.selected >= self.scroll + body_h {
                self.scroll = self.selected + 1 - body_h;
            }
        }

        let (left_w, mid_w, right_w) = if three_pane {
            let l = (w * 20 / 100).clamp(14, 24);
            let m = (w * 36 / 100).clamp(20, 38);
            (l, m, w - l - m)
        } else {
            let m = (w * 42 / 100).clamp(18, 28);
            (0, m, w - m)
        };

        let (_, cur_leaf) = path::split(&self.cwd);
        let sel_info = self
            .entries
            .get(self.selected)
            .map(|e| format!("{} · {}", e.name, if e.kind == EntryKind::Dir { String::from("dir") } else { human_size(e.size) }))
            .unwrap_or_else(|| String::from("empty"));
        let hidden_badge = if self.show_hidden { "hidden:on" } else { "hidden:off" };
        let top_hdr = format!(
            " {}◈ Yazi{}  {}{}{}  {}({}){}  {}[{}]{}",
            widgets::fg(ROSE), "\x1b[0m",
            widgets::fg(BLUE), self.cwd, "\x1b[0m",
            widgets::fg(MUTE), sel_info, "\x1b[0m",
            widgets::fg(CYAN), hidden_badge, "\x1b[0m"
        );

        let mut out = String::from("\x1b[H");
        out.push_str(&widgets::pad_ansi(&top_hdr, w));
        out.push_str("\x1b[K\r\n");

        // Top border row of panes.
        if three_pane {
            out.push_str(&widgets::box_top("Parent", "", left_w, BORDER, MUTE, MUTE));
        }
        let mid_badge = format!("{}/{}", if self.entries.is_empty() { 0 } else { self.selected + 1 }, self.entries.len());
        out.push_str(&widgets::box_top("Files", &mid_badge, mid_w, CYAN, FROST, CYAN));
        let prev_title = self.entries.get(self.selected).map(|e| e.name.as_str()).unwrap_or("Preview");
        out.push_str(&widgets::box_top(prev_title, "", right_w, BORDER, LAVENDER, MUTE));
        out.push_str("\x1b[K\r\n");

        for row in 0..body_h {
            if three_pane {
                let l_txt = if self.cwd == "/" && row == 0 {
                    format!("{}/ (root){}", widgets::fg(MUTE), "\x1b[0m")
                } else if let Some(pe) = self.parent_entries.get(row) {
                    let active = pe.name == cur_leaf;
                    let col = if active { CYAN } else if pe.kind == EntryKind::Dir { BLUE } else { MUTE };
                    let suffix = if pe.kind == EntryKind::Dir { "/" } else { "" };
                    let lbl = truncate(&format!("{} {}{}", file_glyph(pe), pe.name, suffix), left_w.saturating_sub(4));
                    if active {
                        format!("{}{}{}\x1b[0m", widgets::bg(SEL_BG), widgets::fg(col), lbl)
                    } else {
                        format!("{}{}\x1b[0m", widgets::fg(col), lbl)
                    }
                } else {
                    String::new()
                };
                out.push_str(&widgets::box_row(&l_txt, left_w, BORDER));
            }

            let mid_txt = if let Some(e) = self.entries.get(self.scroll + row) {
                let is_sel = self.scroll + row == self.selected;
                let glyph = file_glyph(e);
                let sz = if e.kind == EntryKind::Dir { String::from("dir") } else { human_size(e.size) };
                let name_cap = mid_w.saturating_sub(4 + sz.len() + 4);
                let name = truncate(&format!("{}{}", e.name, if e.kind == EntryKind::Dir { "/" } else { "" }), name_cap);
                let n_col = if is_sel {
                    FROST
                } else if e.kind == EntryKind::Dir {
                    BLUE
                } else if e.name.ends_with(".wasm") {
                    EMERALD
                } else {
                    FROST
                };
                if is_sel {
                    format!(
                        "{}{}▶ {} {:<nw$} {}{}\x1b[0m",
                        widgets::bg(SEL_BG), widgets::fg(ROSE), glyph, name,
                        widgets::fg(AMBER), sz,
                        nw = name_cap
                    )
                } else {
                    format!(
                        "  {}{} {}{:<nw$} {}{}\x1b[0m",
                        widgets::fg(CYAN), glyph,
                        widgets::fg(n_col), name,
                        widgets::fg(MUTE), sz,
                        nw = name_cap
                    )
                }
            } else if row == 0 && self.entries.is_empty() {
                format!("{}(empty folder)\x1b[0m", widgets::fg(MUTE))
            } else {
                String::new()
            };
            out.push_str(&widgets::box_row(&mid_txt, mid_w, CYAN));

            let right_txt = render_preview_row(&self.preview, row, right_w.saturating_sub(4));
            out.push_str(&widgets::box_row(&right_txt, right_w, BORDER));
            out.push_str("\x1b[K\r\n");
        }

        // Bottom border row of panes.
        if three_pane {
            out.push_str(&widgets::box_bot("", left_w, BORDER, MUTE));
        }
        out.push_str(&widgets::box_bot("", mid_w, CYAN, MUTE));
        out.push_str(&widgets::box_bot("", right_w, BORDER, MUTE));
        out.push_str("\x1b[K\r\n");

        let help = format!(
            " {}jk/↑↓{} move · {}h/←{} parent · {}l/→/Enter{} open · {}.{} hidden · {}q{} cd&quit · {}Esc{} cancel",
            widgets::fg(CYAN), "\x1b[0m",
            widgets::fg(CYAN), "\x1b[0m",
            widgets::fg(CYAN), "\x1b[0m",
            widgets::fg(CYAN), "\x1b[0m",
            widgets::fg(EMERALD), "\x1b[0m",
            widgets::fg(CORAL), "\x1b[0m",
        );
        out.push_str(&widgets::pad_ansi(&help, w));
        out.push_str("\x1b[K\x1b[J");
        out
    }
}

fn load_sorted_dir(abs: &str, show_hidden: bool, host: &dyn Host) -> Result<Vec<DirEntry>, String> {
    let mut items = host.list_dir(abs)?;
    items.retain(|e| show_hidden || !e.name.starts_with('.'));
    items.sort_by(|a, b| (a.kind != EntryKind::Dir, &a.name).cmp(&(b.kind != EntryKind::Dir, &b.name)));
    Ok(items)
}

fn render_preview_row(preview: &Preview, row: usize, inner_w: usize) -> String {
    match preview {
        Preview::Empty => {
            if row == 0 { format!("{}(nothing selected)\x1b[0m", widgets::fg(MUTE)) } else { String::new() }
        }
        Preview::Err(e) => {
            if row == 0 { format!("{}error: {}\x1b[0m", widgets::fg(ROSE), e) } else { String::new() }
        }
        Preview::Dir(items) => {
            if items.is_empty() && row == 0 {
                return format!("{}(empty directory)\x1b[0m", widgets::fg(MUTE));
            }
            if let Some(e) = items.get(row) {
                let col = if e.kind == EntryKind::Dir { BLUE } else { FROST };
                let suf = if e.kind == EntryKind::Dir { "/" } else { "" };
                let sz = if e.kind == EntryKind::Dir { String::new() } else { format!(" ({})", human_size(e.size)) };
                format!("{}{} {}{}{}{}\x1b[0m", widgets::fg(col), file_glyph(e), e.name, suf, widgets::fg(MUTE), sz)
            } else {
                String::new()
            }
        }
        Preview::Text { lines, ext, total_bytes } => {
            if row == 0 {
                return format!("{}UTF-8 text · {} ({} lines)\x1b[0m", widgets::fg(MUTE), human_size(*total_bytes as u64), lines.len());
            }
            if let Some(line) = lines.get(row - 1) {
                let num = format!("{}{:>3} │\x1b[0m ", widgets::fg(MUTE), row);
                let code_cap = inner_w.saturating_sub(6);
                let clipped = truncate(line, code_cap);
                let tinted = tint_source_line(&clipped, ext);
                format!("{}{}", num, tinted)
            } else {
                String::new()
            }
        }
        Preview::Binary { hex_lines, total_bytes } => {
            if row == 0 {
                return format!("{}binary · {} (hex preview)\x1b[0m", widgets::fg(AMBER), human_size(*total_bytes as u64));
            }
            hex_lines.get(row - 1).map(|l| format!("{}{}\x1b[0m", widgets::fg(FROST), l)).unwrap_or_default()
        }
    }
}

fn format_hex(bytes: &[u8]) -> Vec<String> {
    let mut rows = Vec::new();
    for (idx, chunk) in bytes.chunks(8).enumerate() {
        let mut hex = String::new();
        let mut asc = String::new();
        for b in chunk {
            hex.push_str(&format!("{:02x} ", b));
            asc.push(if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '.' });
        }
        rows.push(format!("{:04x}: {:<24} {}", idx * 8, hex, asc));
    }
    rows
}
