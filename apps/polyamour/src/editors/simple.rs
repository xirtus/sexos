// SPDX-License-Identifier: GPL-3.0-only
//! The Python Console, the Info editor, Preferences (Add-ons) and the editors
//! that only show a note for now (File Browser, Spreadsheet).
//!
//! The console looks and keys like Blender's: `>>> ` prompt and `... ` for the
//! rest of a block (with auto-indent), Up/Down history, Tab or Ctrl+Space
//! completion from the live namespace, `C = bpy.context`, `D = bpy.data`,
//! Ctrl+C/V. Each line runs in the document's Python kernel (python.rs) and is
//! one undo step when it changed the scene. Where Python cannot run live
//! (SexOS has no pipes between programs yet) lines run as polyops commands.
//! Info lists the report history (`Doc::info.reports`) with Blender's icons.
//! Preferences > Add-ons lists the add-ons folder with enable checkboxes; the
//! choice is kept in Polyamour's data folder.
use super::{header_menus, note, EditorKind};
use crate::anim::{self, ReportKind};
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::input::{Input, Key, Mods};
use crate::theme as t;
use crate::ui::{MenuItem, Op, Ui};
use sex_3d::script::Session;

// Blender's default console theme.
const C_OUTPUT: u32 = 0x71a8ff;
const C_INPUT: u32 = 0xf2f2f2;
const C_INFO: u32 = 0x95d600;
const C_ERROR: u32 = 0xff3333;
const C_CURSOR: u32 = 0xdc6060;

const PROMPT: &str = ">>> ";
const MORE: &str = "... ";

/// polyops command words, for Tab completion and `help`.
pub const COMMANDS: &[&str] = &[
    "active", "active_camera", "add_empty", "add_mesh", "add_object", "add_primitive", "camera_add", "collection_new", "collection_set", "delete", "deselect", "exists",
    "get", "light_add", "link", "list", "material_assign", "material_clear", "material_new", "material_set", "mesh_edit", "mesh_new", "mesh_set", "modifier_add",
    "modifier_clear", "modifier_remove", "modifier_set", "parent", "ping", "render", "scene", "select", "select_add", "select_all", "set", "world",
];

pub struct State {
    pub kind: EditorKind,
    id: u64,
    /// Console scrollback: (colour, line).
    pub lines: Vec<(u32, String)>,
    pub input: String,
    /// Caret position in `input`, in chars.
    pub cursor: usize,
    pub history: Vec<String>,
    hist_pos: Option<usize>,
    session: Option<Session>,
    /// Lines scrolled up from the bottom.
    scroll: usize,
    /// Python runs live (decided when the console first shows); false: polyops lines.
    pub live: Option<bool>,
    /// Inside a block: the prompt is `... `.
    more: bool,
    /// Preferences: the add-ons folder was read.
    scanned: bool,
}

impl State {
    pub fn new(kind: EditorKind) -> State {
        State { kind, id: anim::editor_id(), lines: Vec::new(), input: String::new(), cursor: 0, history: Vec::new(), hist_pos: None, session: None, scroll: 0, live: None, more: false, scanned: false }
    }
    /// Start the console: Python's banner, or the polyops one where Python
    /// cannot run live.
    fn start(&mut self, d: &mut Doc) {
        if self.live.is_some() {
            return;
        }
        let banner = crate::python::banner(d);
        self.live = Some(d.python.live());
        let mut lines: Vec<(u32, String)> = banner.into_iter().map(|l| (C_OUTPUT, l)).collect();
        lines.append(&mut self.lines);
        self.lines = lines;
    }
    fn prompt(&self) -> &'static str {
        if self.more {
            MORE
        } else {
            PROMPT
        }
    }
    fn session(&mut self) -> &mut Session {
        self.session.get_or_insert_with(|| {
            let mut s = Session::new();
            let mut scratch = sex_3d::scene::Scene::new();
            let _ = s.exec_line(&mut scratch, "polyops 1");
            s
        })
    }
    /// Run one console line against the document.
    pub fn exec(&mut self, d: &mut Doc, line: &str) {
        self.start(d);
        if self.live == Some(true) {
            self.exec_python(d, line);
            return;
        }
        self.lines.push((C_INPUT, format!("{PROMPT}{line}")));
        let cmd = line.trim();
        if !cmd.is_empty() && self.history.last().map(String::as_str) != Some(line) {
            self.history.push(line.to_string());
        }
        self.hist_pos = None;
        self.scroll = 0;
        match cmd {
            "" => {}
            "help" | "help()" => {
                self.lines.push((C_OUTPUT, "polyops commands (see docs/handoff/POLYAMOUR_PYTHON_2026_11_14.md):".into()));
                for chunk in COMMANDS.chunks(8) {
                    self.lines.push((C_OUTPUT, format!("    {}", chunk.join(" "))));
                }
            }
            "clear" => self.lines.clear(),
            "end" | "polyops 1" => {}
            _ => {
                let mut work = d.scene.clone();
                let res = self.session().exec_line(&mut work, cmd);
                let replies = self.session().take_replies();
                match res {
                    Ok(()) => {
                        if work != d.scene {
                            d.push_undo("Console");
                            d.scene = work;
                            d.touch();
                        }
                        for r in replies {
                            self.lines.push((C_OUTPUT, r));
                        }
                    }
                    Err(e) => {
                        self.lines.push((C_ERROR, e.message.clone()));
                        anim::report(d, ReportKind::Error, &e.message);
                    }
                }
            }
        }
    }
    fn exec_python(&mut self, d: &mut Doc, line: &str) {
        self.lines.push((C_INPUT, format!("{}{line}", self.prompt())));
        if !line.trim().is_empty() && self.history.last().map(String::as_str) != Some(line) {
            self.history.push(line.to_string());
        }
        self.hist_pos = None;
        self.scroll = 0;
        // An indent-only line (left by auto-indent) ends a block like an empty one.
        let line = if self.more && line.trim().is_empty() { "" } else { line };
        match crate::python::console(d, line) {
            Some(crate::python::ConsoleResult::More) => {
                self.more = true;
                // Blender's auto-indent: keep the block's indentation, one
                // more level after a colon.
                let mut indent: String = line.chars().take_while(|c| *c == ' ').collect();
                if line.trim_end().ends_with(':') {
                    indent.push_str("    ");
                }
                self.input = indent;
                self.cursor = self.input.chars().count();
            }
            Some(crate::python::ConsoleResult::Done { printed, .. }) => {
                self.more = false;
                self.print(&printed);
            }
            None => {
                self.more = false;
                self.live = Some(false);
                let why = d.python.unavailable.clone().unwrap_or_else(|| "the Python kernel stopped".into());
                self.lines.push((C_ERROR, format!("Python is not available: {why}; lines now run as polyops commands")));
            }
        }
    }
    /// Printed text into the scrollback: output blue, errors red.
    fn print(&mut self, printed: &[crate::python::Printed]) {
        use crate::python::Printed;
        let mut cur: Option<(bool, String)> = None;
        let mut chunks = Vec::new();
        for p in printed {
            let (err, s) = match p {
                Printed::Out(s) => (false, s),
                Printed::Err(s) => (true, s),
            };
            match &mut cur {
                Some((e, buf)) if *e == err => buf.push_str(s),
                _ => {
                    if let Some(c) = cur.take() {
                        chunks.push(c);
                    }
                    cur = Some((err, s.clone()));
                }
            }
        }
        chunks.extend(cur);
        for (err, text) in chunks {
            for l in text.strip_suffix('\n').unwrap_or(&text).split('\n') {
                self.lines.push((if err { C_ERROR } else { C_OUTPUT }, l.replace('\t', "    ")));
            }
        }
    }
    /// Python completion of the input up to the caret (Tab, Ctrl+Space).
    fn complete_python(&mut self, d: &mut Doc) {
        let at = byte_at(&self.input, self.cursor);
        let (head, tail) = (self.input[..at].to_string(), self.input[at..].to_string());
        let Some(hits) = crate::python::complete(d, &head) else { return };
        match hits.as_slice() {
            [] => {}
            [one] => {
                self.input = format!("{one}{tail}");
                self.cursor = one.chars().count();
            }
            many => {
                let mut prefix = many[0].clone();
                for h in many {
                    while !h.starts_with(&prefix) {
                        prefix.pop();
                    }
                }
                self.lines.push((C_INPUT, format!("{}{}", self.prompt(), self.input)));
                // Blender lists the candidates' last parts.
                let cut = head.rfind(['.', '[', ' ', '(']).map_or(0, |i| i + 1);
                let names: Vec<String> = many.iter().map(|m| m.get(cut..).unwrap_or(m).to_string()).collect();
                for chunk in names.chunks(6) {
                    self.lines.push((C_INFO, chunk.join("  ")));
                }
                if prefix.len() > head.len() {
                    self.input = format!("{prefix}{tail}");
                    self.cursor = prefix.chars().count();
                }
            }
        }
    }
    fn complete(&mut self) {
        let word = self.input.trim_start();
        if word.contains(' ') {
            return;
        }
        let hits: Vec<&str> = COMMANDS.iter().copied().filter(|c| c.starts_with(word)).collect();
        match hits.as_slice() {
            [] => {}
            [one] => {
                self.input = format!("{one} ");
                self.cursor = self.input.chars().count();
            }
            many => {
                // Extend to the common prefix, list the candidates like Blender.
                let mut prefix = many[0].to_string();
                for h in many {
                    while !h.starts_with(&prefix) {
                        prefix.pop();
                    }
                }
                self.lines.push((C_INPUT, format!("{PROMPT}{}", self.input)));
                self.lines.push((C_INFO, many.join("  ")));
                self.input = prefix;
                self.cursor = self.input.chars().count();
            }
        }
    }
}

fn byte_at(s: &str, c: usize) -> usize {
    s.char_indices().nth(c).map_or(s.len(), |(i, _)| i)
}

fn console_menus(s: &State) -> Vec<(&'static str, Vec<MenuItem>)> {
    let m = |l: &str, k: &str, op: &str| MenuItem::op(l, k, anim::mail(op, s.id, ""));
    vec![
        ("View", vec![MenuItem::op("Toggle Maximize Area", "Ctrl Space", Op::new("screen.screen_full_area"))]),
        (
            "Console",
            vec![
                m("Clear All", "", "console.clear"),
                m("Clear Line", "Shift Enter", "console.clear_line"),
                MenuItem::Separator,
                m("Copy", "Ctrl C", "console.copy"),
                m("Paste", "Ctrl V", "console.paste"),
                MenuItem::Separator,
                m("Autocomplete", "Ctrl Space", "console.autocomplete"),
            ],
        ),
    ]
}

fn info_menus(s: &State) -> Vec<(&'static str, Vec<MenuItem>)> {
    let m = |l: &str, k: &str, op: &str| MenuItem::op(l, k, anim::mail(op, s.id, ""));
    vec![
        ("View", vec![MenuItem::op("Toggle Maximize Area", "Ctrl Space", Op::new("screen.screen_full_area"))]),
        ("Edit", vec![m("Copy", "Ctrl C", "info.report_copy"), m("Delete", "X", "info.report_delete")]),
    ]
}

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let menus = match s.kind {
        EditorKind::PythonConsole => console_menus(s),
        EditorKind::Info => info_menus(s),
        _ => return,
    };
    let fns: Vec<Box<dyn Fn(&Doc) -> Vec<MenuItem>>> = menus.iter().map(|(_, items)| {
        let items = items.clone();
        Box::new(move |_: &Doc| items.clone()) as Box<dyn Fn(&Doc) -> Vec<MenuItem>>
    }).collect();
    let refs: Vec<(&str, &dyn Fn(&Doc) -> Vec<MenuItem>)> = menus.iter().zip(fns.iter()).map(|((n, _), f)| (*n, f.as_ref())).collect();
    header_menus(ui, hr, hr.x, &refs, d);
}

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    for (op, _) in anim::take_mail(d, s.id) {
        match op.as_str() {
            "console.clear" => s.lines.clear(),
            "console.clear_line" => {
                s.input.clear();
                s.cursor = 0;
            }
            "console.copy" | "info.report_copy" => {
                d.info.clipboard = if s.kind == EditorKind::Info {
                    d.info.reports.iter().map(|r| r.text.clone()).collect::<Vec<_>>().join("\n")
                } else {
                    s.input.clone()
                }
            }
            "console.paste" => paste(s, d),
            "console.autocomplete" => {
                if s.live == Some(true) {
                    s.complete_python(d)
                } else {
                    s.complete()
                }
            }
            "info.report_delete" => d.info.reports.clear(),
            _ => {}
        }
    }
    match s.kind {
        EditorKind::PythonConsole => {
            s.start(d);
            console_body(s, ui, r)
        }
        EditorKind::Info => info_body(s, d, ui, r),
        EditorKind::Preferences => prefs_body(s, d, ui, r),
        _ => note(ui, r, s.kind.label()),
    }
}

fn wheel(s: &mut State, ui: &mut Ui, r: Rect, total: usize) {
    if let Some(Input::Wheel { x, y, steps, .. }) = ui.input {
        if r.contains(x, y) && ui.p.clip().contains(x, y) {
            s.scroll = (s.scroll as i32 + steps * 3).clamp(0, total.saturating_sub(1) as i32) as usize;
            ui.input = None;
        }
    }
}

fn console_body(s: &mut State, ui: &mut Ui, r: Rect) {
    let total = s.lines.len() + 1;
    wheel(s, ui, r, total);
    let p = &mut *ui.p;
    let lh = p.mono.line_height() + 3;
    let cw = ((p.mono.measure(&"M".repeat(32)) + 16) / 32).max(1);
    let x = r.x + 8;
    // Bottom-up: the input line, then the scrollback above it.
    let mut y = r.bottom() - lh - 4 + s.scroll as i32 * lh;
    let input_line = format!("{}{}", s.prompt(), s.input);
    p.mono_text(x, y, &input_line, C_INPUT);
    let cx = x + (s.prompt().len() + s.cursor) as i32 * cw;
    p.fill(Rect::new(cx, y, 2, lh - 2), C_CURSOR);
    for (col, line) in s.lines.iter().rev() {
        y -= lh;
        if y + lh < r.y {
            break;
        }
        if y >= r.y && y < r.bottom() {
            p.mono_text(x, y, line, *col);
        }
    }
}

fn info_body(s: &mut State, d: &Doc, ui: &mut Ui, r: Rect) {
    let total = d.info.reports.len();
    wheel(s, ui, r, total);
    let p = &mut *ui.p;
    let lh = 22;
    let mut y = r.bottom() - lh - 2 + s.scroll as i32 * lh;
    for (i, rep) in d.info.reports.iter().enumerate().rev() {
        if y + lh < r.y {
            break;
        }
        if i % 2 == 1 {
            p.fill(Rect::new(r.x, y, r.w, lh), 0x212121);
        }
        let (icon, col) = match rep.kind {
            ReportKind::Info => (Some(0x4a7fd0), t::TEXT),
            ReportKind::Warning => (Some(0xd99a1e), t::TEXT),
            ReportKind::Error => (Some(0xd03a3a), t::TEXT),
            ReportKind::Output => (None, t::TEXT_DIM),
        };
        if let Some(ic) = icon {
            let b = Rect::new(r.x + 8, y + 4, 14, 14);
            p.round_rect(b, 3, ic);
            let mark = match rep.kind {
                ReportKind::Info => "i",
                _ => "!",
            };
            p.label(b, mark, 0xffffff, Align::Center);
        }
        p.label(Rect::new(r.x + 30, y, r.w - 36, lh), &rep.text, col, Align::Left);
        y -= lh;
    }
}

/// Preferences: the Add-ons section (the only one with settings so far).
fn prefs_body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    ui.p.fill(r, t::PROPERTIES);
    let (nav, content) = r.take_left(160.min(r.w / 3));
    ui.p.fill(nav, 0x232323);
    let mut ny = nav.y + 8;
    for (name, on) in [("Interface", false), ("Themes", false), ("Viewport", false), ("Editing", false), ("Animation", false), ("Add-ons", true), ("Input", false), ("System", false), ("File Paths", false)] {
        let br = Rect::new(nav.x + 6, ny, nav.w - 12, 22);
        if on {
            ui.p.round_rect(br, 4, t::SELECTED);
        }
        ui.p.label(br.shrink(6), name, if on { t::TEXT_HI } else { t::TEXT_DIM }, Align::Left);
        ny += 24;
    }
    let x = content.x + 14;
    let w = content.w - 28;
    let mut y = content.y + 10;
    if !s.scanned {
        s.scanned = true;
        crate::python::scan_addons(d);
    }
    if let Some(why) = d.python.unavailable.clone() {
        ui.p.label(Rect::new(x, y, w, 20), &format!("Python add-ons unavailable ({why}); .wasm add-ons run in-process."), t::TEXT_DIM, Align::Left);
        y += 24;
    }
    let folder = crate::python::addons_dir(d).display().to_string();
    if ui.button(Rect::new(x + w - 90, y, 90, 22), "Refresh") {
        d.queue.push(Op::new("script.addons_refresh"));
    }
    ui.p.label(Rect::new(x, y, w - 100, 22), &format!("Add-ons folder: {folder}"), t::TEXT_DIM, Align::Left);
    y += 32;
    if d.python.addons.is_empty() {
        ui.p.label(Rect::new(x, y, w, 22), "No add-ons found. Put a .py file with bl_info, a package folder, or a .wasm add-on into the add-ons folder.", t::TEXT_DIM, Align::Left);
        return;
    }
    let addons = d.python.addons.clone();
    for a in &addons {
        let h = 66;
        let br = Rect::new(x, y, w, h);
        ui.p.round_rect(br, 5, 0x3d3d3d);
        let mut on = d.python.enabled.contains(&a.module);
        let title = if a.category.is_empty() { a.name.clone() } else { format!("{}: {}", a.category, a.name) };
        if ui.checkbox(Rect::new(x + 10, y + 6, w - 20, 22), &title, &mut on) {
            d.queue.push(Op::with("script.addon_toggle", &a.module));
        }
        ui.p.label(Rect::new(x + 30, y + 26, w - 40, 18), &a.description, t::TEXT_DIM, Align::Left);
        let mut meta = Vec::new();
        if !a.version.is_empty() {
            meta.push(format!("Version {}", a.version));
        }
        if !a.author.is_empty() {
            meta.push(format!("by {}", a.author));
        }
        if !a.location.is_empty() {
            meta.push(a.location.clone());
        }
        meta.push(a.file.rsplit(['/', '\\']).next().unwrap_or(&a.file).to_string());
        ui.p.label(Rect::new(x + 30, y + 44, w - 40, 18), &meta.join("   "), t::TEXT_DIM, Align::Left);
        y += h + 6;
        if y > content.bottom() {
            break;
        }
    }
}

fn paste(s: &mut State, d: &mut Doc) {
    let clip = d.info.clipboard.clone();
    let mut lines = clip.split('\n').peekable();
    while let Some(l) = lines.next() {
        let at = byte_at(&s.input, s.cursor);
        s.input.insert_str(at, l);
        s.cursor += l.chars().count();
        if lines.peek().is_some() {
            let line = core::mem::take(&mut s.input);
            s.cursor = 0;
            s.exec(d, &line);
        }
    }
}

pub fn hotkey(s: &mut State, d: &mut Doc, i: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, text } = *i else { return false };
    if s.kind == EditorKind::Info {
        return match (key, mods) {
            (Key::Char('x') | Key::Delete, Mods::NONE) => {
                d.info.reports.clear();
                true
            }
            (Key::Char('c'), Mods::CTRL) => {
                d.info.clipboard = d.info.reports.iter().map(|r| r.text.clone()).collect::<Vec<_>>().join("\n");
                true
            }
            _ => false,
        };
    }
    if s.kind != EditorKind::PythonConsole {
        return false;
    }
    let n = s.input.chars().count();
    match (key, mods.ctrl) {
        (Key::Enter | Key::NumEnter, _) if mods.shift => {
            s.input.clear();
            s.cursor = 0;
        }
        (Key::Enter | Key::NumEnter, _) => {
            let line = core::mem::take(&mut s.input);
            s.cursor = 0;
            s.exec(d, &line);
        }
        (Key::Space, true) => {
            if s.live == Some(true) {
                s.complete_python(d)
            } else {
                s.complete()
            }
        }
        (Key::Backspace, _) => {
            if s.cursor > 0 {
                let a = byte_at(&s.input, s.cursor - 1);
                let b = byte_at(&s.input, s.cursor);
                s.input.replace_range(a..b, "");
                s.cursor -= 1;
            }
        }
        (Key::Delete, _) => {
            if s.cursor < n {
                let a = byte_at(&s.input, s.cursor);
                let b = byte_at(&s.input, s.cursor + 1);
                s.input.replace_range(a..b, "");
            }
        }
        (Key::Left, _) => s.cursor = s.cursor.saturating_sub(1),
        (Key::Right, _) => s.cursor = (s.cursor + 1).min(n),
        (Key::Home, _) => s.cursor = 0,
        (Key::End, _) => s.cursor = n,
        (Key::Up | Key::Down, _) => {
            if s.history.is_empty() {
                return true;
            }
            let last = s.history.len() - 1;
            s.hist_pos = match (key, s.hist_pos) {
                (Key::Up, None) => Some(last),
                (Key::Up, Some(p)) => Some(p.saturating_sub(1)),
                (_, Some(p)) if p < last => Some(p + 1),
                _ => None,
            };
            s.input = s.hist_pos.map(|p| s.history[p].clone()).unwrap_or_default();
            s.cursor = s.input.chars().count();
        }
        (Key::Tab, _) => {
            let before = &s.input[..byte_at(&s.input, s.cursor)];
            if s.live == Some(true) && before.trim().is_empty() {
                // Indent at the start of a line, as Blender's console does.
                let at = byte_at(&s.input, s.cursor);
                s.input.insert_str(at, "    ");
                s.cursor += 4;
            } else if s.live == Some(true) {
                s.complete_python(d)
            } else {
                s.complete()
            }
        }
        (Key::Char('v'), true) => paste(s, d),
        (Key::Char('c'), true) => d.info.clipboard = s.input.clone(),
        (Key::Space, false) => {
            let at = byte_at(&s.input, s.cursor);
            s.input.insert(at, ' ');
            s.cursor += 1;
        }
        (Key::Esc, _) => {}
        (_, false) => {
            let c = text.or(match key {
                Key::Char(c) => Some(c),
                Key::NumDot => Some('.'),
                Key::NumMinus => Some('-'),
                Key::NumPlus => Some('+'),
                Key::Num(n) => char::from_digit(n as u32, 10),
                _ => None,
            });
            match c {
                Some(c) if !c.is_control() => {
                    let at = byte_at(&s.input, s.cursor);
                    s.input.insert(at, c);
                    s.cursor += 1;
                }
                _ => return false,
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_runs_polyops_lines_with_history() {
        let mut d = Doc::default();
        let mut s = State::new(EditorKind::PythonConsole);
        // The polyops console (where Python cannot run live).
        s.live = Some(false);
        let n = d.scene.objects.len();
        for c in "add_primitive cube name=Box".chars() {
            let key = if c == ' ' { Key::Space } else { Key::Char(c) };
            hotkey(&mut s, &mut d, &Input::Key { key, mods: Mods::NONE, text: Some(c) }, Rect::default());
        }
        hotkey(&mut s, &mut d, &Input::key(Key::Enter), Rect::default());
        assert_eq!(d.scene.objects.len(), n + 1);
        assert!(d.scene.find_object("Box").is_some());
        s.exec(&mut d, "get Box.location");
        assert!(s.lines.last().unwrap().1.contains("ok 0 0 0"), "{:?}", s.lines.last());
        s.exec(&mut d, "add_primitive nonsense");
        assert_eq!(s.lines.last().unwrap().0, C_ERROR);
        hotkey(&mut s, &mut d, &Input::key(Key::Up), Rect::default());
        assert_eq!(s.input, "add_primitive nonsense");
        hotkey(&mut s, &mut d, &Input::key(Key::Up), Rect::default());
        assert_eq!(s.input, "get Box.location");
        s.input = "material_n".into();
        s.complete();
        assert_eq!(s.input, "material_new ");
        d.run(&Op::new("ed.undo"));
        assert!(d.scene.find_object("Box").is_none());
    }

    fn typed(s: &mut State, d: &mut Doc, text: &str) {
        for c in text.chars() {
            let key = if c == ' ' { Key::Space } else { Key::Char(c) };
            hotkey(s, d, &Input::Key { key, mods: Mods::NONE, text: Some(c) }, Rect::default());
        }
    }

    #[test]
    fn python_console_blocks_completion_and_undo() {
        if std::process::Command::new("python3").arg("--version").output().is_err() {
            return;
        }
        let mut d = Doc::default();
        let mut s = State::new(EditorKind::PythonConsole);
        s.start(&mut d);
        assert_eq!(s.live, Some(true));
        assert!(s.lines[0].1.starts_with("PYTHON INTERACTIVE CONSOLE 3."), "{:?}", s.lines[0]);
        let n = d.scene.objects.len();
        typed(&mut s, &mut d, "for i in range(2):");
        hotkey(&mut s, &mut d, &Input::key(Key::Enter), Rect::default());
        assert_eq!(s.prompt(), MORE);
        assert_eq!(s.input, "    ", "auto-indent");
        typed(&mut s, &mut d, "bpy.ops.mesh.primitive_cube_add(location=(i * 3, 5, 0))");
        hotkey(&mut s, &mut d, &Input::key(Key::Enter), Rect::default());
        hotkey(&mut s, &mut d, &Input::key(Key::Enter), Rect::default());
        assert_eq!(s.prompt(), PROMPT);
        assert_eq!(d.scene.objects.len(), n + 2);
        s.exec(&mut d, "len(D.objects)");
        assert_eq!(s.lines.last().unwrap(), &(C_OUTPUT, (n + 2).to_string()));
        s.exec(&mut d, "1/0");
        assert_eq!(s.lines.last().unwrap().0, C_ERROR);
        assert!(s.lines.iter().any(|l| l.1.contains("ZeroDivisionError")));
        // Ctrl+Space completes from the namespace.
        typed(&mut s, &mut d, "C.active_obj");
        hotkey(&mut s, &mut d, &Input::key_mods(Key::Space, Mods::CTRL), Rect::default());
        assert_eq!(s.input, "C.active_object");
        s.input = "D.objects['Cu".into();
        s.cursor = s.input.chars().count();
        hotkey(&mut s, &mut d, &Input::key(Key::Tab), Rect::default());
        assert!(s.input.starts_with("D.objects['Cube"), "{}", s.input);
        // Each line that changed the scene is one undo step.
        d.run(&Op::new("ed.undo"));
        assert_eq!(d.scene.objects.len(), n);
    }
}
