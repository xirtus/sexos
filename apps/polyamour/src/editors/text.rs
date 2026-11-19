// SPDX-License-Identifier: GPL-3.0-only
//! Blender's Text Editor: text datablocks (`Doc::texts`), a monospace view with
//! a line-number gutter and Python syntax highlighting in Blender's default
//! text-editor colours, a caret with Shift selection, the usual editing keys,
//! an in-program clipboard, per-text undo, and Run Script.
//!
//! Run Script (the header button, Text > Run Script, Alt+P, or Ctrl+Enter)
//! runs a text that starts with `polyops 1` in-process; anything else runs as
//! Python in the document's kernel (python.rs): one undo step, and on an
//! error nothing changes and the error with its line goes to the Info editor
//! and the status bar. Templates > Python offers our own starting points.
use super::header_menus;
use crate::anim::{self, ReportKind};
use crate::doc::Doc;
use crate::draw::{Align, Painter, Rect};
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{MenuItem, Op, Ui};
use std::collections::HashMap;

// Blender's default text-editor theme (Preferences > Themes > Text Editor).
pub const C_TEXT: u32 = 0xdddddd;
pub const C_KEYWORD: u32 = 0xeb8f45;
pub const C_SPECIAL: u32 = 0x7ec6ff;
pub const C_BUILTIN: u32 = 0xd5a3f5;
pub const C_STRING: u32 = 0xd6e08a;
pub const C_NUMBER: u32 = 0x50dbff;
pub const C_COMMENT: u32 = 0x8c8c8c;
pub const C_DECORATOR: u32 = 0xf06c6c;
const C_GUTTER: u32 = 0x262626;
const C_LINE_NO: u32 = 0x7a7a7a;
const C_SELECTION: u32 = 0x2b4a7d;
const C_CARET: u32 = 0x71a8ff;
const C_CURRENT_LINE: u32 = 0x242424;

const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in",
    "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "match", "case",
];
const BUILTINS: &[&str] = &[
    "True", "False", "None", "self", "print", "range", "len", "int", "float", "str", "list", "dict", "set", "tuple", "abs", "min", "max", "sum", "enumerate", "zip", "map",
    "filter", "sorted", "reversed", "isinstance", "getattr", "setattr", "hasattr", "type", "object", "super", "round", "open", "any", "all", "bool", "repr", "iter", "next",
];

/// Example scripts offered in Templates.
pub const TEMPLATES: &[(&str, &str, &str)] = &[
    ("Operator Simple", "operator_simple.py", include_str!("../../python/templates/operator_simple.py")),
    ("UI Panel Simple", "ui_panel_simple.py", include_str!("../../python/templates/ui_panel_simple.py")),
    ("Add-on Add Object", "addon_add_object.py", include_str!("../../python/templates/addon_add_object.py")),
    ("Bmesh Simple", "bmesh_simple.py", include_str!("../../python/templates/bmesh_simple.py")),
    ("Handlers and Timers", "driver_handler.py", include_str!("../../python/templates/driver_handler.py")),
    ("Pyramid", "blender_pyramid.py", include_str!("../../python/examples/blender_pyramid.py")),
    ("Spiral Staircase", "spiral_staircase.py", include_str!("../../python/examples/spiral_staircase.py")),
    (
        "Polyops: Add Shapes",
        "add_shapes.poly",
        "polyops 1\n# polyops commands run in-process (no Python needed).\nadd_primitive uv_sphere name=Ball location=0,0,2 radius=0.8\nmaterial_new Red base_color=0.8,0.1,0.1 roughness=0.3\nmaterial_assign Ball Red\nadd_primitive torus location=0,0,0.5\nend\n",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum EditKind {
    #[default]
    Other,
    Type,
    Delete,
}

#[derive(Default)]
struct History {
    undo: Vec<(String, (usize, usize))>,
    redo: Vec<(String, (usize, usize))>,
    last: EditKind,
}

#[derive(Default)]
pub struct State {
    id: u64,
    /// Name of the text shown (a `Doc::texts` entry).
    pub text: Option<String>,
    /// Caret as (line, column in chars), and the other end of the selection.
    pub caret: (usize, usize),
    pub anchor: Option<(usize, usize)>,
    /// First visible line.
    pub scroll: usize,
    /// Column the caret wants when moving up/down through shorter lines.
    want_col: Option<usize>,
    follow: bool,
    dragging: bool,
    hist: HashMap<String, History>,
    char_w: i32,
    line_h: i32,
    visible: usize,
    pub line_numbers: bool,
    pub syntax: bool,
    initialised: bool,
    /// Where the header's "New" button was drawn (tests click it).
    pub new_button: Rect,
}

// ---------------------------------------------------------------- text helpers

fn line_starts(s: &str) -> Vec<usize> {
    let mut v = vec![0];
    v.extend(s.match_indices('\n').map(|(i, _)| i + 1));
    v
}

fn line_of<'a>(s: &'a str, starts: &[usize], l: usize) -> &'a str {
    let a = starts[l];
    let b = starts.get(l + 1).map_or(s.len(), |&e| e - 1);
    &s[a..b]
}

fn to_byte(s: &str, pos: (usize, usize)) -> usize {
    let starts = line_starts(s);
    let l = pos.0.min(starts.len() - 1);
    let line = line_of(s, &starts, l);
    starts[l] + line.char_indices().nth(pos.1).map_or(line.len(), |(i, _)| i)
}

fn from_byte(s: &str, b: usize) -> (usize, usize) {
    let starts = line_starts(s);
    let l = starts.partition_point(|&x| x <= b) - 1;
    (l, s[starts[l]..b.min(s.len())].chars().count())
}

fn line_len(s: &str, l: usize) -> usize {
    let starts = line_starts(s);
    if l >= starts.len() {
        return 0;
    }
    line_of(s, &starts, l).chars().count()
}

fn line_count(s: &str) -> usize {
    s.matches('\n').count() + 1
}

impl State {
    fn sel(&self) -> Option<((usize, usize), (usize, usize))> {
        let a = self.anchor?;
        if a == self.caret {
            return None;
        }
        Some(if a < self.caret { (a, self.caret) } else { (self.caret, a) })
    }
    fn text_index(&self, d: &Doc) -> Option<usize> {
        let name = self.text.as_ref()?;
        d.texts.iter().position(|t| &t.0 == name)
    }
    fn snapshot(&mut self, d: &Doc, kind: EditKind) {
        let Some(i) = self.text_index(d) else { return };
        let h = self.hist.entry(d.texts[i].0.clone()).or_default();
        if kind == EditKind::Other || kind != h.last || h.undo.is_empty() {
            h.undo.push((d.texts[i].1.clone(), self.caret));
            if h.undo.len() > 200 {
                h.undo.remove(0);
            }
        }
        h.redo.clear();
        h.last = kind;
    }
    fn break_group(&mut self, d: &Doc) {
        if let Some(name) = &self.text {
            if let Some(h) = self.hist.get_mut(name) {
                h.last = EditKind::Other;
            }
        }
        let _ = d;
    }
    /// Replace the selection (or insert at the caret) with `ins`.
    fn insert(&mut self, d: &mut Doc, ins: &str, kind: EditKind) {
        let Some(i) = self.text_index(d) else { return };
        if kind == EditKind::Type && ins.chars().any(|c| c.is_whitespace()) {
            self.break_group(d);
        }
        self.snapshot(d, kind);
        let s = &mut d.texts[i].1;
        let (a, b) = match self.sel() {
            Some((a, b)) => (to_byte(s, a), to_byte(s, b)),
            None => {
                let c = to_byte(s, self.caret);
                (c, c)
            }
        };
        s.replace_range(a..b, ins);
        self.caret = from_byte(s, a + ins.len());
        self.anchor = None;
        self.want_col = None;
        self.follow = true;
    }
    fn delete(&mut self, d: &mut Doc, forward: bool) {
        let Some(i) = self.text_index(d) else { return };
        if self.sel().is_some() {
            self.insert(d, "", EditKind::Delete);
            return;
        }
        let s = &d.texts[i].1;
        let c = to_byte(s, self.caret);
        let range = if forward {
            let n = s[c..].chars().next().map_or(0, |ch| ch.len_utf8());
            c..c + n
        } else {
            // Backspace over indentation removes up to the previous 4-column stop.
            let starts = line_starts(s);
            let line = line_of(s, &starts, self.caret.0);
            let before: String = line.chars().take(self.caret.1).collect();
            if !before.is_empty() && before.chars().all(|c| c == ' ') {
                let n = (before.len() - 1) % 4 + 1;
                c - n..c
            } else {
                let n = s[..c].chars().next_back().map_or(0, |ch| ch.len_utf8());
                c - n..c
            }
        };
        if range.is_empty() {
            return;
        }
        self.snapshot(d, EditKind::Delete);
        let s = &mut d.texts[i].1;
        let start = range.start;
        s.replace_range(range, "");
        self.caret = from_byte(s, start);
        self.anchor = None;
        self.want_col = None;
        self.follow = true;
    }
    fn selected_text(&self, d: &Doc) -> Option<String> {
        let i = self.text_index(d)?;
        let (a, b) = self.sel()?;
        let s = &d.texts[i].1;
        Some(s[to_byte(s, a)..to_byte(s, b)].to_string())
    }
    fn move_caret(&mut self, d: &Doc, to: (usize, usize), extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.caret);
            }
        } else {
            self.anchor = None;
        }
        self.caret = to;
        self.follow = true;
        let _ = d;
    }
    fn undo(&mut self, d: &mut Doc, redo: bool) -> bool {
        let Some(i) = self.text_index(d) else { return false };
        let name = d.texts[i].0.clone();
        let h = self.hist.entry(name).or_default();
        let (from, to) = if redo { (&mut h.redo, &mut h.undo) } else { (&mut h.undo, &mut h.redo) };
        let Some((text, caret)) = from.pop() else { return false };
        to.push((core::mem::replace(&mut d.texts[i].1, text), self.caret));
        h.last = EditKind::Other;
        self.caret = caret;
        self.anchor = None;
        self.follow = true;
        true
    }
}

// ---------------------------------------------------------------- syntax

/// Colour spans (start char, end char, colour) of one line. `in_string` is the
/// open triple quote carried from earlier lines; it is updated for the next.
pub fn highlight(line: &str, in_string: &mut Option<char>) -> Vec<(usize, usize, u32)> {
    let c: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut prev_word = String::new();
    if let Some(q) = *in_string {
        let mut j = 0;
        let mut closed = false;
        while j + 2 < c.len() {
            if c[j] == q && c[j + 1] == q && c[j + 2] == q {
                closed = true;
                j += 3;
                break;
            }
            j += 1;
        }
        if !closed {
            out.push((0, c.len(), C_STRING));
            return out;
        }
        out.push((0, j, C_STRING));
        *in_string = None;
        i = j;
    }
    while i < c.len() {
        let ch = c[i];
        if ch == '#' {
            out.push((i, c.len(), C_COMMENT));
            break;
        }
        if ch == '"' || ch == '\'' {
            let triple = i + 2 < c.len() && c[i + 1] == ch && c[i + 2] == ch;
            let start = i;
            if triple {
                i += 3;
                let mut closed = false;
                while i + 2 < c.len() {
                    if c[i] == ch && c[i + 1] == ch && c[i + 2] == ch {
                        i += 3;
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    *in_string = Some(ch);
                    i = c.len();
                }
            } else {
                i += 1;
                while i < c.len() && c[i] != ch {
                    if c[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(c.len());
            }
            out.push((start, i, C_STRING));
            prev_word.clear();
            continue;
        }
        if ch.is_ascii_digit() || (ch == '.' && i + 1 < c.len() && c[i + 1].is_ascii_digit() && (i == 0 || !c[i - 1].is_alphanumeric())) {
            if i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_') {
                i += 1;
                continue;
            }
            let start = i;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '.' || c[i] == '_') {
                i += 1;
            }
            out.push((start, i, C_NUMBER));
            continue;
        }
        if ch == '@' && c[..i].iter().all(|c| c.is_whitespace()) {
            out.push((i, c.len(), C_DECORATOR));
            break;
        }
        if ch.is_alphabetic() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            let w: String = c[start..i].iter().collect();
            if prev_word == "def" || prev_word == "class" {
                out.push((start, i, C_SPECIAL));
            } else if KEYWORDS.contains(&w.as_str()) {
                out.push((start, i, C_KEYWORD));
            } else if BUILTINS.contains(&w.as_str()) && (start == 0 || c[start - 1] != '.') {
                out.push((start, i, C_BUILTIN));
            }
            prev_word = w;
            continue;
        }
        if !ch.is_whitespace() {
            prev_word.clear();
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------- menus

fn m(s: &State, label: &str, key: &str, op: &str) -> MenuItem {
    MenuItem::op(label, key, anim::mail(op, s.id, ""))
}

fn view_menu(s: &State) -> Vec<MenuItem> {
    vec![
        MenuItem::Check("Line Numbers".into(), s.line_numbers, anim::mail("text.toggle_line_numbers", s.id, "")),
        MenuItem::Check("Syntax Highlight".into(), s.syntax, anim::mail("text.toggle_syntax", s.id, "")),
        MenuItem::Separator,
        m(s, "Top of File", "Ctrl Home", "text.move_top"),
        m(s, "Bottom of File", "Ctrl End", "text.move_bottom"),
        MenuItem::Separator,
        MenuItem::op("Toggle Maximize Area", "Ctrl Space", Op::new("screen.screen_full_area")),
    ]
}

fn text_menu(s: &State) -> Vec<MenuItem> {
    vec![
        m(s, "New", "", "text.new"),
        m(s, "Open...", "", "text.open"),
        MenuItem::Separator,
        m(s, "Save", "", "text.save"),
        MenuItem::Separator,
        m(s, "Run Script", "Alt P", "text.run_script"),
    ]
}

fn edit_menu(s: &State) -> Vec<MenuItem> {
    vec![
        m(s, "Undo", "Ctrl Z", "text.undo"),
        m(s, "Redo", "Shift Ctrl Z", "text.redo"),
        MenuItem::Separator,
        m(s, "Cut", "Ctrl X", "text.cut"),
        m(s, "Copy", "Ctrl C", "text.copy"),
        m(s, "Paste", "Ctrl V", "text.paste"),
        m(s, "Duplicate Line", "Shift Ctrl D", "text.duplicate_line"),
        MenuItem::Separator,
        m(s, "Toggle Comments", "Ctrl /", "text.comment_toggle"),
    ]
}

fn select_menu(s: &State) -> Vec<MenuItem> {
    vec![m(s, "All", "Ctrl A", "text.select_all"), m(s, "Line", "Shift Ctrl A", "text.select_line")]
}

fn format_menu(s: &State) -> Vec<MenuItem> {
    vec![m(s, "Indent", "Tab", "text.indent"), m(s, "Unindent", "Shift Tab", "text.unindent"), MenuItem::Separator, m(s, "Toggle Comments", "Ctrl /", "text.comment_toggle")]
}

fn templates_menu(s: &State) -> Vec<MenuItem> {
    let items = TEMPLATES.iter().enumerate().map(|(i, t)| MenuItem::op(t.0, "", anim::mail("text.template", s.id, &i.to_string()))).collect();
    vec![MenuItem::Sub("Python".into(), items), MenuItem::Sub("Open Shading Language".into(), vec![MenuItem::Heading("Not available yet".into())])]
}

// ---------------------------------------------------------------- header

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    init(s, d);
    let (v, tx, e, se, f, te) = (view_menu(s), text_menu(s), edit_menu(s), select_menu(s), format_menu(s), templates_menu(s));
    let view = move |_: &Doc| v.clone();
    let text = move |_: &Doc| tx.clone();
    let edit = move |_: &Doc| e.clone();
    let sel = move |_: &Doc| se.clone();
    let fmt = move |_: &Doc| f.clone();
    let tpl = move |_: &Doc| te.clone();
    let menus: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 6] = [("View", &view), ("Text", &text), ("Edit", &edit), ("Select", &sel), ("Format", &fmt), ("Templates", &tpl)];
    let mut x = header_menus(ui, hr, hr.x, &menus, d) + 10;
    let y = hr.y + 3;
    let h = hr.h - 6;
    // The text datablock: browse dropdown, then New / Open (or its name).
    let br = Rect::new(x, y, 36, h);
    if ui.dropdown(br, "") {
        let mut items: Vec<MenuItem> = d.texts.iter().map(|t| MenuItem::op(&t.0, "", anim::mail("text.select", s.id, &t.0))).collect();
        if items.is_empty() {
            items.push(MenuItem::Heading("No texts".into()));
        }
        ui.open_menu(br, "", items);
    }
    text_icon(ui.p, Rect::new(br.x + 2, br.y, 18, br.h));
    x = br.right() + 2;
    match s.text.clone() {
        Some(name) => {
            let nr = Rect::new(x, y, 150, h);
            let mut n = name.clone();
            if ui.text_field(crate::ui::id(&format!("text{}/name", s.id)), nr, &mut n) && !n.is_empty() && !d.texts.iter().any(|t| t.0 == n) {
                if let Some(i) = s.text_index(d) {
                    d.texts[i].0 = n.clone();
                    if let Some(h) = s.hist.remove(&name) {
                        s.hist.insert(n.clone(), h);
                    }
                    s.text = Some(n);
                }
            }
            let nb = Rect::new(nr.right() + 2, y, 24, h);
            if ui.button(nb, "+") {
                new_text(s, d, "", "Text");
            }
            let xb = Rect::new(nb.right() + 2, y, 24, h);
            if ui.button(xb, "X") {
                s.text = None;
            }
            x = xb.right();
        }
        None => {
            let nb = Rect::new(x, y, 60, h);
            s.new_button = nb;
            if ui.button(nb, "+ New") {
                new_text(s, d, "", "Text");
            }
            let ob = Rect::new(nb.right() + 2, y, 60, h);
            if ui.button(ob, "Open") {
                d.queue.push(Op::new("text.open"));
            }
            x = ob.right();
        }
    }
    // Right: Run Script, then the line-number / syntax toggles.
    let tg = Rect::new(hr.right() - 56, y, 50, h);
    let mut on = [s.line_numbers, s.syntax];
    for (i, lbl) in ["#", "Py"].iter().enumerate() {
        let r = Rect::new(tg.x + i as i32 * 25, y, 24, h);
        if ui.flat_button(r, lbl, on[i]) {
            on[i] = !on[i];
        }
    }
    s.line_numbers = on[0];
    s.syntax = on[1];
    let rr = Rect::new(tg.x - 104, y, 96, h);
    if rr.x > x + 8 && s.text.is_some() {
        let hover = ui.hovered(rr);
        ui.p.round_rect(rr, 4, if hover { t::BUTTON_HOVER } else { t::BUTTON });
        let (cx, cy) = (rr.x as f32 + 14.0, rr.y as f32 + rr.h as f32 / 2.0);
        play_tri(ui.p, cx, cy, t::TEXT);
        ui.p.label(Rect::new(rr.x + 24, rr.y, rr.w - 26, rr.h), "Run Script", t::TEXT, Align::Left);
        if ui.clicked(rr) {
            run_current(s, d);
        }
    }
}

fn play_tri(p: &mut Painter, cx: f32, cy: f32, color: u32) {
    for i in 0..10 {
        let y = cy - 5.0 + i as f32;
        let w = (5.0 - (i as f32 - 4.5).abs()) * 1.6;
        p.fill(Rect::new((cx - 3.0) as i32, y as i32, w.round().max(1.0) as i32, 1), color);
    }
}

/// A small page icon for the text datablock.
fn text_icon(p: &mut Painter, r: Rect) {
    let b = Rect::new(r.x + 4, r.y + r.h / 2 - 6, 10, 12);
    p.frame(b, t::TEXT_DIM);
    for i in 0..3 {
        p.fill(Rect::new(b.x + 2, b.y + 3 + i * 3, 6, 1), t::TEXT_DIM);
    }
}

fn init(s: &mut State, d: &Doc) {
    if !s.initialised {
        s.initialised = true;
        s.id = anim::editor_id();
        s.line_numbers = true;
        s.syntax = true;
    }
    if s.text.as_ref().is_some_and(|n| !d.texts.iter().any(|t| &t.0 == n)) {
        s.text = None;
    }
    if s.text.is_none() {
        s.text = d.texts.last().map(|t| t.0.clone());
        s.caret = (0, 0);
        s.anchor = None;
        s.scroll = 0;
    }
}

/// Add a text datablock named like Blender's ("Text", "Text.001") and show it.
pub fn new_text(s: &mut State, d: &mut Doc, body: &str, base: &str) {
    let name = sex_3d::scene::unique_name(base, |n| d.texts.iter().any(|t| t.0 == n));
    d.texts.push((name.clone(), body.to_string()));
    s.text = Some(name);
    s.caret = (0, 0);
    s.anchor = None;
    s.scroll = 0;
}

// ---------------------------------------------------------------- running

fn run_current(s: &mut State, d: &mut Doc) {
    let Some(i) = s.text_index(d) else {
        anim::report(d, ReportKind::Warning, "No text to run");
        return;
    };
    let (name, src) = d.texts[i].clone();
    run_script(d, &name, &src);
}

/// Run a text: polyops in-process, otherwise Python (python.rs).
pub fn run_script(d: &mut Doc, name: &str, src: &str) {
    let first = src.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).unwrap_or("");
    if first.starts_with("polyops") {
        apply_polyops(d, name, src);
        return;
    }
    crate::python::run_script(d, name, src);
}

fn apply_polyops(d: &mut Doc, name: &str, ops: &str) {
    let mut work = d.scene.clone();
    match sex_3d::script::run(&mut work, ops) {
        Ok(rep) => {
            if work != d.scene {
                d.push_undo("Run Script");
                d.scene = work;
                crate::edit::sanitize(d);
                d.touch();
                d.anim.dirty = true;
            }
            if !rep.panels.is_empty() {
                crate::python::register_polyops_panels(d, &rep.panels, None);
            }
            for r in &rep.replies {
                anim::report(d, ReportKind::Output, r);
            }
            let made = if rep.created.is_empty() { String::new() } else { format!(", added {}", rep.created.join(", ")) };
            anim::report(d, ReportKind::Info, &format!("Ran {name}: {} command(s){made}", rep.commands));
        }
        Err(e) => anim::report(d, ReportKind::Error, &format!("{name}: {e}")),
    }
}

// ---------------------------------------------------------------- body

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    init(s, d);
    for (op, arg) in anim::take_mail(d, s.id) {
        command(s, d, &op, &arg);
    }
    let p = &mut *ui.p;
    s.char_w = ((p.mono.measure(&"M".repeat(32)) + 16) / 32).max(1);
    s.line_h = p.mono.line_height() + 3;
    p.fill(r, t::TEXT_EDITOR);
    let Some(ti) = s.text_index(d) else {
        return;
    };
    let text = d.texts[ti].1.clone();
    let starts = line_starts(&text);
    let n = starts.len();
    let digits = format!("{n}").len().max(2) as i32;
    let gutter = if s.line_numbers { digits * s.char_w + 16 } else { 4 };
    let tx = r.x + gutter + 6;
    s.visible = ((r.h - 6) / s.line_h).max(1) as usize;
    if s.follow {
        s.follow = false;
        if s.caret.0 < s.scroll {
            s.scroll = s.caret.0;
        } else if s.caret.0 >= s.scroll + s.visible {
            s.scroll = s.caret.0 + 1 - s.visible;
        }
    }
    s.scroll = s.scroll.min(n.saturating_sub(1));

    // Mouse: place the caret, drag to select, wheel to scroll.
    let pos_at = |s: &State, x: i32, y: i32| {
        let l = (s.scroll as i32 + ((y - r.y - 3) / s.line_h).max(0)) as usize;
        let l = l.min(n - 1);
        let c = ((x - tx + s.char_w / 2) / s.char_w).max(0) as usize;
        (l, c.min(line_of(&text, &starts, l).chars().count()))
    };
    let clip = p.clip();
    match ui.input {
        Some(Input::Down { x, y, button: Button::Left, mods }) if r.contains(x, y) && clip.contains(x, y) => {
            let to = pos_at(s, x, y);
            if mods.shift {
                if s.anchor.is_none() {
                    s.anchor = Some(s.caret);
                }
            } else {
                s.anchor = Some(to);
            }
            s.caret = to;
            s.dragging = true;
            s.want_col = None;
            s.break_group(d);
            ui.input = None;
        }
        Some(Input::Move { x, y }) if s.dragging => {
            s.caret = pos_at(s, x, y);
            ui.input = None;
        }
        Some(Input::Up { button: Button::Left, .. }) if s.dragging => {
            s.dragging = false;
            ui.input = None;
        }
        Some(Input::Wheel { x, y, steps, .. }) if r.contains(x, y) && clip.contains(x, y) => {
            s.scroll = (s.scroll as i32 - steps * 3).clamp(0, n as i32 - 1) as usize;
            ui.input = None;
        }
        _ => {}
    }

    let p = &mut *ui.p;
    if s.line_numbers {
        p.fill(Rect::new(r.x, r.y, gutter, r.h), C_GUTTER);
    }
    // Triple-quoted strings carry over lines: scan the lines above the view.
    let mut in_str = None;
    if s.syntax {
        for l in 0..s.scroll.min(n) {
            highlight(line_of(&text, &starts, l), &mut in_str);
        }
    }
    let sel = s.sel();
    for row in 0..=s.visible {
        let l = s.scroll + row;
        if l >= n {
            break;
        }
        let y = r.y + 3 + row as i32 * s.line_h;
        let line = line_of(&text, &starts, l);
        let len = line.chars().count();
        if l == s.caret.0 && sel.is_none() {
            p.fill(Rect::new(r.x + gutter, y, r.w - gutter, s.line_h), C_CURRENT_LINE);
        }
        if let Some((a, b)) = sel {
            if l >= a.0 && l <= b.0 {
                let c0 = if l == a.0 { a.1 } else { 0 };
                let c1 = if l == b.0 { b.1 } else { len + 1 };
                p.fill(Rect::new(tx + c0 as i32 * s.char_w, y, ((c1 - c0.min(c1)) as i32 * s.char_w).max(0), s.line_h), C_SELECTION);
            }
        }
        if s.line_numbers {
            let num = format!("{:>w$}", l + 1, w = digits as usize);
            p.mono_text(r.x + 8, y + 1, &num, if l == s.caret.0 { t::TEXT_HI } else { C_LINE_NO });
        }
        p.push_clip(Rect::new(r.x + gutter, r.y, r.w - gutter, r.h));
        // One glyph per cell, so columns line up exactly with the caret grid.
        let mut colors = vec![C_TEXT; len];
        if s.syntax {
            for (a, b, col) in highlight(line, &mut in_str) {
                for c in colors.iter_mut().take(b.min(len)).skip(a) {
                    *c = col;
                }
            }
        }
        let first = ((clip.x - tx) / s.char_w).max(0) as usize;
        let last = (((clip.right() - tx) / s.char_w) + 1).max(0) as usize;
        let mut buf = [0u8; 4];
        for (ci, ch) in line.chars().enumerate().skip(first).take(last.saturating_sub(first)) {
            if ch != ' ' {
                p.mono_text(tx + ci as i32 * s.char_w, y + 1, ch.encode_utf8(&mut buf), colors[ci]);
            }
        }
        if l == s.caret.0 {
            let cx = tx + s.caret.1.min(len) as i32 * s.char_w;
            p.fill(Rect::new(cx - 1, y, 2, s.line_h), C_CARET);
        }
        p.pop_clip();
    }
}

fn command(s: &mut State, d: &mut Doc, op: &str, arg: &str) {
    match op {
        "text.new" => new_text(s, d, "", "Text"),
        "text.select" => {
            s.text = Some(arg.to_string());
            s.caret = (0, 0);
            s.anchor = None;
            s.scroll = 0;
        }
        "text.template" => {
            if let Some(tp) = arg.parse::<usize>().ok().and_then(|i| TEMPLATES.get(i)) {
                new_text(s, d, tp.2, tp.1);
            }
        }
        "text.open" => anim::report(d, ReportKind::Warning, "text.open is not available yet (no file browser): use Templates or paste"),
        "text.save" => {
            if let Some(i) = s.text_index(d) {
                let (name, body) = d.texts[i].clone();
                match std::fs::write(&name, body) {
                    Ok(()) => anim::report(d, ReportKind::Info, &format!("Saved text \"{name}\"")),
                    Err(e) => anim::report(d, ReportKind::Error, &format!("Cannot write {name}: {e}")),
                }
            }
        }
        "text.run_script" => run_current(s, d),
        "text.toggle_line_numbers" => s.line_numbers = !s.line_numbers,
        "text.toggle_syntax" => s.syntax = !s.syntax,
        _ => {
            let key = match op {
                "text.undo" => (Key::Char('z'), Mods::CTRL),
                "text.redo" => (Key::Char('z'), Mods::CTRL_SHIFT),
                "text.cut" => (Key::Char('x'), Mods::CTRL),
                "text.copy" => (Key::Char('c'), Mods::CTRL),
                "text.paste" => (Key::Char('v'), Mods::CTRL),
                "text.duplicate_line" => (Key::Char('d'), Mods::CTRL_SHIFT),
                "text.comment_toggle" => (Key::Char('/'), Mods::CTRL),
                "text.select_all" => (Key::Char('a'), Mods::CTRL),
                "text.select_line" => (Key::Char('a'), Mods::CTRL_SHIFT),
                "text.indent" => (Key::Tab, Mods::NONE),
                "text.unindent" => (Key::Tab, Mods::SHIFT),
                "text.move_top" => (Key::Home, Mods::CTRL),
                "text.move_bottom" => (Key::End, Mods::CTRL),
                _ => return,
            };
            // Indent from the menu acts on the selection's lines.
            if op == "text.indent" && s.sel().is_none() {
                s.anchor = Some((s.caret.0, 0));
                s.caret = (s.caret.0, line_len(&d.texts[s.text_index(d).unwrap_or(0)].1, s.caret.0));
            }
            hotkey(s, d, &Input::Key { key: key.0, mods: key.1, text: None }, Rect::default());
        }
    }
}

/// Lines touched by the selection (or the caret's line).
fn sel_lines(s: &State) -> (usize, usize) {
    match s.sel() {
        Some((a, b)) => (a.0, if b.1 == 0 && b.0 > a.0 { b.0 - 1 } else { b.0 }),
        None => (s.caret.0, s.caret.0),
    }
}

/// Apply `f` to each line of the selection as one undo step.
fn edit_lines(s: &mut State, d: &mut Doc, f: &dyn Fn(&str) -> String) {
    let Some(i) = s.text_index(d) else { return };
    let (l0, l1) = sel_lines(s);
    s.snapshot(d, EditKind::Other);
    let text = &mut d.texts[i].1;
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    for l in l0..=l1.min(lines.len() - 1) {
        lines[l] = f(&lines[l]);
    }
    *text = lines.join("\n");
    let len1 = line_len(text, l1);
    if s.sel().is_some() {
        s.anchor = Some((l0, 0));
        s.caret = (l1, len1);
    } else {
        s.caret.1 = s.caret.1.min(line_len(text, s.caret.0));
    }
    s.follow = true;
}

pub fn hotkey(s: &mut State, d: &mut Doc, i: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, text: ch } = *i else { return false };
    let Some(ti) = s.text_index(d) else {
        // With no text, only Ctrl+N style creation applies (Blender: Alt+N).
        return false;
    };
    let text = d.texts[ti].1.clone();
    let lines = line_count(&text);
    let shift = mods.shift;
    let (l, c) = s.caret;
    let cur_len = line_len(&text, l);
    let word_left = |l: usize, c: usize| -> (usize, usize) {
        if c == 0 {
            return if l > 0 { (l - 1, line_len(&text, l - 1)) } else { (0, 0) };
        }
        let starts = line_starts(&text);
        let chars: Vec<char> = line_of(&text, &starts, l).chars().collect();
        let mut j = c.min(chars.len());
        while j > 0 && !chars[j - 1].is_alphanumeric() {
            j -= 1;
        }
        while j > 0 && (chars[j - 1].is_alphanumeric() || chars[j - 1] == '_') {
            j -= 1;
        }
        (l, j)
    };
    let word_right = |l: usize, c: usize| -> (usize, usize) {
        let starts = line_starts(&text);
        let chars: Vec<char> = line_of(&text, &starts, l).chars().collect();
        if c >= chars.len() {
            return if l + 1 < lines { (l + 1, 0) } else { (l, c) };
        }
        let mut j = c;
        while j < chars.len() && !chars[j].is_alphanumeric() {
            j += 1;
        }
        while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
            j += 1;
        }
        (l, j)
    };
    let vert = |s: &mut State, dl: i32| {
        let want = *s.want_col.get_or_insert(s.caret.1);
        let nl = (s.caret.0 as i32 + dl).clamp(0, lines as i32 - 1) as usize;
        (nl, want.min(line_len(&text, nl)))
    };
    if key == Key::Char('p') && mods.alt && !mods.ctrl {
        run_current(s, d);
        return true;
    }
    match (key, mods.ctrl) {
        (Key::Enter | Key::NumEnter, true) => run_current(s, d),
        (Key::Enter | Key::NumEnter, false) => {
            let starts = line_starts(&text);
            let line = line_of(&text, &starts, l);
            let mut indent: String = line.chars().take_while(|c| *c == ' ').collect();
            let before: String = line.chars().take(c).collect();
            if before.trim_end().ends_with(':') {
                indent.push_str("    ");
            }
            s.break_group(d);
            s.insert(d, &format!("\n{indent}"), EditKind::Other);
        }
        (Key::Tab, false) => {
            if shift {
                edit_lines(s, d, &|l: &str| {
                    let n = l.chars().take(4).take_while(|c| *c == ' ').count();
                    l[n..].to_string()
                });
            } else if s.sel().is_some_and(|(a, b)| a.0 != b.0) {
                edit_lines(s, d, &|l: &str| format!("    {l}"));
            } else {
                let n = 4 - c % 4;
                s.insert(d, &" ".repeat(n), EditKind::Type);
            }
        }
        (Key::Backspace, false) => s.delete(d, false),
        (Key::Delete, false) => s.delete(d, true),
        (Key::Backspace, true) => {
            if s.sel().is_none() {
                s.anchor = Some(s.caret);
                s.caret = word_left(l, c);
            }
            s.insert(d, "", EditKind::Delete);
        }
        (Key::Delete, true) => {
            if s.sel().is_none() {
                s.anchor = Some(s.caret);
                s.caret = word_right(l, c);
            }
            s.insert(d, "", EditKind::Delete);
        }
        (Key::Left, ctrl) => {
            let to = if !shift && !ctrl && s.sel().is_some() {
                s.sel().unwrap().0
            } else if ctrl {
                word_left(l, c)
            } else if c > 0 {
                (l, c.min(cur_len) - 1)
            } else if l > 0 {
                (l - 1, line_len(&text, l - 1))
            } else {
                (0, 0)
            };
            s.want_col = None;
            s.move_caret(d, to, shift);
        }
        (Key::Right, ctrl) => {
            let to = if !shift && !ctrl && s.sel().is_some() {
                s.sel().unwrap().1
            } else if ctrl {
                word_right(l, c)
            } else if c < cur_len {
                (l, c + 1)
            } else if l + 1 < lines {
                (l + 1, 0)
            } else {
                (l, cur_len)
            };
            s.want_col = None;
            s.move_caret(d, to, shift);
        }
        (Key::Up, false) => {
            let to = vert(s, -1);
            s.move_caret(d, to, shift);
        }
        (Key::Down, false) => {
            let to = vert(s, 1);
            s.move_caret(d, to, shift);
        }
        (Key::PageUp, false) => {
            let n = s.visible.max(2) as i32 - 1;
            let to = vert(s, -n);
            s.scroll = s.scroll.saturating_sub(n as usize);
            s.move_caret(d, to, shift);
        }
        (Key::PageDown, false) => {
            let n = s.visible.max(2) as i32 - 1;
            let to = vert(s, n);
            s.scroll = (s.scroll + n as usize).min(lines - 1);
            s.move_caret(d, to, shift);
        }
        (Key::Home, false) => {
            s.want_col = None;
            s.move_caret(d, (l, 0), shift);
        }
        (Key::End, false) => {
            s.want_col = None;
            s.move_caret(d, (l, cur_len), shift);
        }
        (Key::Home, true) => s.move_caret(d, (0, 0), shift),
        (Key::End, true) => s.move_caret(d, (lines - 1, line_len(&text, lines - 1)), shift),
        (Key::Char('a'), true) if !shift => {
            s.anchor = Some((0, 0));
            s.caret = (lines - 1, line_len(&text, lines - 1));
        }
        (Key::Char('a'), true) => {
            s.anchor = Some((l, 0));
            s.caret = (l, cur_len);
        }
        (Key::Char('c'), true) => {
            if let Some(t) = s.selected_text(d) {
                d.info.clipboard = t;
            }
        }
        (Key::Char('x'), true) => {
            if let Some(t) = s.selected_text(d) {
                d.info.clipboard = t;
                s.insert(d, "", EditKind::Other);
            }
        }
        (Key::Char('v'), true) => {
            let clip = d.info.clipboard.clone();
            if !clip.is_empty() {
                s.break_group(d);
                s.insert(d, &clip, EditKind::Other);
            }
        }
        (Key::Char('z'), true) => {
            if !s.undo(d, shift) {
                anim::report(d, ReportKind::Warning, if shift { "Nothing to redo" } else { "Nothing to undo" });
            }
        }
        (Key::Char('y'), true) => {
            s.undo(d, true);
        }
        (Key::Char('d'), true) if shift => {
            let starts = line_starts(&text);
            let line = line_of(&text, &starts, l).to_string();
            let keep = s.caret;
            s.anchor = None;
            s.caret = (l, cur_len);
            s.insert(d, &format!("\n{line}"), EditKind::Other);
            s.caret = (keep.0 + 1, keep.1);
        }
        (Key::Char('/'), true) => {
            let starts = line_starts(&text);
            let (l0, l1) = sel_lines(s);
            let all = (l0..=l1.min(lines - 1)).all(|i| line_of(&text, &starts, i).trim_start().starts_with('#') || line_of(&text, &starts, i).trim().is_empty());
            if all {
                edit_lines(s, d, &|l: &str| {
                    let n = l.len() - l.trim_start().len();
                    let rest = &l[n..];
                    let rest = rest.strip_prefix("# ").or_else(|| rest.strip_prefix('#')).unwrap_or(rest);
                    format!("{}{rest}", &l[..n])
                });
            } else {
                edit_lines(s, d, &|l: &str| {
                    let n = l.len() - l.trim_start().len();
                    format!("{}# {}", &l[..n], &l[n..])
                });
            }
        }
        (Key::Esc, false) => s.anchor = None,
        (Key::Space, false) => s.insert(d, " ", EditKind::Type),
        (_, false) => {
            let c = ch.or(match key {
                Key::Char(c) => Some(if shift { shifted(c) } else { c }),
                Key::NumDot => Some('.'),
                Key::NumPlus => Some('+'),
                Key::NumMinus => Some('-'),
                Key::NumStar => Some('*'),
                Key::NumSlash => Some('/'),
                Key::Num(n) => char::from_digit(n as u32, 10),
                _ => None,
            });
            match c {
                Some(c) if !c.is_control() => s.insert(d, &c.to_string(), EditKind::Type),
                _ => return false,
            }
        }
        _ => return false,
    }
    true
}

fn shifted(c: char) -> char {
    match c {
        'a'..='z' => c.to_ascii_uppercase(),
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '\\' => '|',
        '`' => '~',
        c => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editors::{Editor, EditorKind};
    use crate::App;

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            let inp = match c {
                '\n' => Input::key(Key::Enter),
                ' ' => Input::Key { key: Key::Space, mods: Mods::NONE, text: Some(' ') },
                c => Input::Key { key: Key::Char(c.to_ascii_lowercase()), mods: Mods::NONE, text: Some(c) },
            };
            app.frame(inp);
        }
    }

    fn text_area(app: &App) -> (u32, Rect) {
        let a = app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == EditorKind::TextEditor).unwrap();
        (a.id, a.rect)
    }

    fn text_state(app: &mut App) -> &mut State {
        let id = text_area(app).0;
        match &mut app.workspaces[app.active_ws].area_mut(id).unwrap().editor {
            Editor::Text(s) => s,
            _ => unreachable!(),
        }
    }

    #[test]
    fn highlight_python() {
        let mut st = None;
        let spans = highlight("def f(x): return 'a' + 12  # hi", &mut st);
        assert_eq!(spans[0], (0, 3, C_KEYWORD));
        assert_eq!(spans[1], (4, 5, C_SPECIAL));
        assert!(spans.contains(&(10, 16, C_KEYWORD)));
        assert!(spans.contains(&(17, 20, C_STRING)));
        assert!(spans.contains(&(23, 25, C_NUMBER)));
        assert!(spans.contains(&(27, 31, C_COMMENT)));
        highlight("x = \"\"\"doc", &mut st);
        assert_eq!(st, Some('"'));
        assert_eq!(highlight("more", &mut st), vec![(0, 4, C_STRING)]);
        highlight("end\"\"\" + 1", &mut st);
        assert_eq!(st, None);
    }

    #[test]
    fn typing_and_run_script_adds_an_object() {
        let mut app = App::new(1600, 900);
        app.doc.want_workspace = Some("Scripting".into());
        app.frame(Input::Tick);
        app.frame(Input::Tick);
        let (_, r) = text_area(&app);
        // "+ New" in the header, then type a polyops script.
        let hy = r.y + 13;
        app.frame(Input::Move { x: r.x + 400, y: r.y + 200 });
        let s = text_state(&mut app);
        assert!(s.text.is_none());
        let nb = text_state(&mut app).new_button;
        let _ = hy;
        app.frame(Input::Move { x: nb.x + 5, y: nb.y + 5 });
        app.frame(Input::Down { x: nb.x + 5, y: nb.y + 5, button: Button::Left, mods: Mods::NONE });
        app.frame(Input::Up { x: nb.x + 5, y: nb.y + 5, button: Button::Left });
        assert_eq!(app.doc.texts.len(), 1);
        app.frame(Input::Move { x: r.x + 400, y: r.y + 200 });
        type_str(&mut app, "polyops 1\nadd_primitive cube name=Typed location=3,0,0\nend");
        assert_eq!(app.doc.texts[0].1, "polyops 1\nadd_primitive cube name=Typed location=3,0,0\nend");
        let n = app.doc.scene.objects.len();
        app.frame(Input::key_mods(Key::Enter, Mods::CTRL));
        assert_eq!(app.doc.scene.objects.len(), n + 1, "{}", app.doc.report);
        assert!(app.doc.scene.find_object("Typed").is_some());
        // One undo step.
        app.frame(Input::Move { x: r.x - 300, y: r.y + 100 });
        app.frame(Input::key_mods(Key::Char('z'), Mods::CTRL));
        assert_eq!(app.doc.scene.objects.len(), n);
        // Editing: select all, copy, undo typing.
        app.frame(Input::Move { x: r.x + 400, y: r.y + 200 });
        app.frame(Input::key_mods(Key::Char('a'), Mods::CTRL));
        app.frame(Input::key_mods(Key::Char('c'), Mods::CTRL));
        assert!(app.doc.info.clipboard.starts_with("polyops 1"));
        app.frame(Input::key(Key::Backspace));
        assert_eq!(app.doc.texts[0].1, "");
        app.frame(Input::key_mods(Key::Char('z'), Mods::CTRL));
        assert!(app.doc.texts[0].1.starts_with("polyops 1"));
        // Auto-indent after a colon, Tab is 4 spaces.
        app.frame(Input::key_mods(Key::End, Mods::CTRL));
        type_str(&mut app, "\nif x:\nyz");
        assert!(app.doc.texts[0].1.ends_with("if x:\n    yz"), "{:?}", app.doc.texts[0].1);
        app.frame(Input::key(Key::Home));
        app.frame(Input::key(Key::Tab));
        assert!(app.doc.texts[0].1.ends_with("if x:\n        yz"));
        assert!(app.doc.info.reports.iter().any(|r| r.text.contains("Ran ")));
    }

    #[test]
    fn python_script_runs_when_python3_is_installed() {
        if std::process::Command::new("python3").arg("--version").output().is_err() {
            return;
        }
        let mut d = Doc::default();
        run_script(&mut d, "t.py", "import polyamour as pa\nm = pa.data.meshes.new('M')\nm.from_pydata([(0,0,0),(1,0,0),(0,1,0)], [], [(0,1,2)])\no = pa.data.objects.new('Tri', m)\npa.context.collection.objects.link(o)\nprint('made', o.name)\n");
        assert!(d.scene.find_object("Tri").is_some(), "{} / {:?}", d.report, d.info.reports);
        assert!(d.info.reports.iter().any(|r| r.kind == ReportKind::Output && r.text.contains("made Tri")), "{:?}", d.info.reports);
        run_script(&mut d, "bad.py", "raise KeyError('Missing')\n");
        assert!(d.info.reports.last().unwrap().kind == ReportKind::Error, "{:?}", d.info.reports);
    }
}
