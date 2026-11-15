// SPDX-License-Identifier: GPL-3.0-only
//! Immediate-mode widgets in Blender's style.
//!
//! Every input event runs one frame: the whole window is drawn, and each widget
//! looks at the frame's event as it draws. The first widget that uses the event
//! takes it, so nothing behind it sees it. Hotkeys go to the area under the
//! pointer afterwards, as in Blender.
use crate::draw::{Align, Painter, Rect};
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;

/// A widget's identity across frames (hash of a path such as "props/obj/loc.x").
pub type Id = u64;

pub fn id(s: &str) -> Id {
    // FNV-1a.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// An operator call, named like Blender's (`object.delete`, `mesh.primitive_cube_add`),
/// with an optional argument string. Menus, buttons and hotkeys all produce these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op {
    pub id: String,
    pub arg: String,
}

impl Op {
    pub fn new(id: &str) -> Op {
        Op { id: id.into(), arg: String::new() }
    }
    pub fn with(id: &str, arg: &str) -> Op {
        Op { id: id.into(), arg: arg.into() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    /// Label, shortcut hint, operator.
    Op(String, String, Op),
    /// A submenu opened on hover.
    Sub(String, Vec<MenuItem>),
    /// Checkbox-style entry.
    Check(String, bool, Op),
    Separator,
    /// A heading line (greyed).
    Heading(String),
}

impl MenuItem {
    pub fn op(label: &str, shortcut: &str, op: Op) -> MenuItem {
        MenuItem::Op(label.into(), shortcut.into(), op)
    }
}

/// An open popup menu (header menus, dropdowns, Shift+A, right-click).
#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    pub title: String,
    pub items: Vec<MenuItem>,
    pub x: i32,
    pub y: i32,
    /// Open submenu index and its popup.
    pub sub: Option<(usize, Box<Popup>)>,
    /// Where the opening button was, so a second click on it closes the menu.
    pub owner: Rect,
}

impl Popup {
    pub fn new(title: &str, items: Vec<MenuItem>, x: i32, y: i32) -> Popup {
        Popup { title: title.into(), items, x, y, sub: None, owner: Rect::default() }
    }
}

const ITEM_H: i32 = 22;
const SEP_H: i32 = 7;

impl Popup {
    fn size(&self, p: &mut Painter) -> (i32, i32) {
        let mut w = 120.max(p.text_width(&self.title) + 24);
        let mut h = if self.title.is_empty() { 4 } else { ITEM_H + 4 };
        for it in &self.items {
            match it {
                MenuItem::Op(l, s, _) => {
                    w = w.max(p.text_width(l) + p.text_width(s) + 56);
                    h += ITEM_H
                }
                MenuItem::Sub(l, _) | MenuItem::Check(l, _, _) | MenuItem::Heading(l) => {
                    w = w.max(p.text_width(l) + 56);
                    h += ITEM_H
                }
                MenuItem::Separator => h += SEP_H,
            }
        }
        (w, h + 4)
    }
    /// The popup's rectangle, kept inside the window.
    pub fn rect(&self, p: &mut Painter) -> Rect {
        let (w, h) = self.size(p);
        let x = self.x.min(p.width as i32 - w).max(0);
        let y = self.y.min(p.height as i32 - h).max(0);
        Rect::new(x, y, w, h)
    }
    fn item_rects(&self, p: &mut Painter) -> Vec<(usize, Rect)> {
        let r = self.rect(p);
        let mut y = r.y + 2 + if self.title.is_empty() { 0 } else { ITEM_H + 2 };
        let mut out = Vec::new();
        for (i, it) in self.items.iter().enumerate() {
            let h = if matches!(it, MenuItem::Separator) { SEP_H } else { ITEM_H };
            out.push((i, Rect::new(r.x + 4, y, r.w - 8, h)));
            y += h;
        }
        out
    }

    pub fn draw(&self, p: &mut Painter, mouse: (i32, i32)) {
        let r = self.rect(p);
        p.round_rect_alpha(Rect::new(r.x + 2, r.y + 3, r.w, r.h), 6, 0x000000, 90);
        p.round_rect(r, 6, t::MENU_BG);
        p.round_frame(r, 6, 0x2a2a2a);
        if !self.title.is_empty() {
            p.label(Rect::new(r.x + 12, r.y + 2, r.w - 24, ITEM_H), &self.title, t::TEXT_DIM, Align::Left);
            p.fill(Rect::new(r.x + 6, r.y + ITEM_H + 2, r.w - 12, 1), t::MENU_SEP);
        }
        let open_sub = self.sub.as_ref().map(|s| s.0);
        for (i, ir) in self.item_rects(p) {
            let hover = ir.contains(mouse.0, mouse.1) || open_sub == Some(i);
            match &self.items[i] {
                MenuItem::Separator => p.fill(Rect::new(ir.x + 2, ir.y + SEP_H / 2, ir.w - 4, 1), t::MENU_SEP),
                MenuItem::Heading(l) => p.label(Rect::new(ir.x + 8, ir.y, ir.w - 16, ir.h), l, t::TEXT_DIM, Align::Left),
                MenuItem::Op(l, s, _) => {
                    if hover {
                        p.round_rect(ir, 4, t::MENU_ITEM_HOVER);
                    }
                    p.label(Rect::new(ir.x + 24, ir.y, ir.w - 32, ir.h), l, if hover { t::TEXT_HI } else { t::TEXT }, Align::Left);
                    p.label(Rect::new(ir.x + 24, ir.y, ir.w - 32, ir.h), s, t::TEXT_DIM, Align::Right);
                }
                MenuItem::Check(l, on, _) => {
                    if hover {
                        p.round_rect(ir, 4, t::MENU_ITEM_HOVER);
                    }
                    let b = Rect::new(ir.x + 6, ir.y + 5, 12, 12);
                    p.round_rect(b, 3, if *on { t::SELECTED } else { t::BUTTON_DARK });
                    if *on {
                        check_mark(p, b);
                    }
                    p.label(Rect::new(ir.x + 24, ir.y, ir.w - 32, ir.h), l, t::TEXT, Align::Left);
                }
                MenuItem::Sub(l, _) => {
                    if hover {
                        p.round_rect(ir, 4, t::MENU_ITEM_HOVER);
                    }
                    p.label(Rect::new(ir.x + 24, ir.y, ir.w - 32, ir.h), l, t::TEXT, Align::Left);
                    p.arrow((ir.right() - 10) as f32, (ir.y + ir.h / 2) as f32, 8.0, false, t::TEXT_DIM);
                }
            }
        }
        if let Some((_, sub)) = &self.sub {
            sub.draw(p, mouse);
        }
    }

    /// Feed an event. Returns (still open, chosen operator).
    pub fn handle(&mut self, p: &mut Painter, input: &Input) -> (bool, Option<Op>) {
        if let Some((_, sub)) = &mut self.sub {
            let sr = sub.rect(p);
            if input.pos().map_or(true, |(x, y)| sr.contains(x, y)) || matches!(input, Input::Key { .. }) {
                let (open, op) = sub.handle(p, input);
                if op.is_some() || !open {
                    return (op.is_none() && !matches!(input, Input::Key { key: Key::Esc, .. }), op);
                }
                if input.pos().is_some() {
                    return (true, None);
                }
            }
        }
        let r = self.rect(p);
        match *input {
            Input::Key { key: Key::Esc, .. } => (false, None),
            Input::Move { x, y } => {
                for (i, ir) in self.item_rects(p) {
                    if ir.contains(x, y) {
                        if let MenuItem::Sub(_, items) = &self.items[i] {
                            if self.sub.as_ref().map(|s| s.0) != Some(i) {
                                let sp = Popup::new("", items.clone(), ir.right() + 2, ir.y - 4);
                                self.sub = Some((i, Box::new(sp)));
                            }
                        } else {
                            self.sub = None;
                        }
                    }
                }
                (true, None)
            }
            Input::Down { x, y, .. } => {
                if !r.contains(x, y) {
                    return (false, None);
                }
                for (i, ir) in self.item_rects(p) {
                    if ir.contains(x, y) {
                        match &self.items[i] {
                            MenuItem::Op(_, _, op) | MenuItem::Check(_, _, op) => return (false, Some(op.clone())),
                            _ => return (true, None),
                        }
                    }
                }
                (true, None)
            }
            _ => (true, None),
        }
    }
}

fn check_mark(p: &mut Painter, b: Rect) {
    let (x, y) = (b.x as f32, b.y as f32);
    p.line(x + 2.5, y + 6.0, x + 5.0, y + 9.0, t::TEXT_HI);
    p.line(x + 5.0, y + 9.0, x + 10.0, y + 3.0, t::TEXT_HI);
}

/// State that outlives a frame: what is being dragged or typed into.
#[derive(Default)]
pub struct UiState {
    pub mouse: (i32, i32),
    /// Widget being dragged (number fields, sliders, scrollbars).
    pub active: Option<Id>,
    pub drag_origin: (i32, i32),
    pub drag_start_value: f32,
    pub drag_moved: bool,
    /// Text being typed into a field: (widget, text).
    pub editing: Option<(Id, String)>,
    /// The editing text is "selected": the first typed character replaces
    /// it, as Blender selects a number field's text when typing starts.
    pub edit_select_all: bool,
    /// A menu a widget asked to open this frame; the app shows it.
    pub open_popup: Option<Popup>,
    /// Status-bar hint for what is under the pointer.
    pub hint: String,
}

/// One frame's widget context.
pub struct Ui<'a> {
    pub p: &'a mut Painter,
    pub st: &'a mut UiState,
    /// This frame's event; `None` once a widget has taken it.
    pub input: Option<Input>,
}

impl<'a> Ui<'a> {
    pub fn mouse(&self) -> (i32, i32) {
        self.st.mouse
    }
    pub fn hovered(&self, r: Rect) -> bool {
        r.contains(self.st.mouse.0, self.st.mouse.1) && self.p.clip().contains(self.st.mouse.0, self.st.mouse.1)
    }
    /// Take the event if it is a left press inside `r`.
    pub fn clicked(&mut self, r: Rect) -> bool {
        if let Some(Input::Down { x, y, button: Button::Left, .. }) = self.input {
            if r.contains(x, y) && self.p.clip().contains(x, y) {
                self.input = None;
                return true;
            }
        }
        false
    }
    pub fn right_clicked(&mut self, r: Rect) -> bool {
        if let Some(Input::Down { x, y, button: Button::Right, .. }) = self.input {
            if r.contains(x, y) && self.p.clip().contains(x, y) {
                self.input = None;
                return true;
            }
        }
        false
    }
    pub fn take(&mut self) -> Option<Input> {
        self.input.take()
    }

    /// Blender's regular push button.
    pub fn button(&mut self, r: Rect, label: &str) -> bool {
        let hover = self.hovered(r);
        self.p.round_rect(r, 4, if hover { t::BUTTON_HOVER } else { t::BUTTON });
        self.p.label(r.shrink(4), label, t::TEXT, Align::Center);
        self.clicked(r)
    }
    /// Flat header-style button (menus, tool buttons). `on` draws it pressed.
    pub fn flat_button(&mut self, r: Rect, label: &str, on: bool) -> bool {
        let hover = self.hovered(r);
        if on {
            self.p.round_rect(r, 4, t::SELECTED);
        } else if hover {
            self.p.round_rect(r, 4, t::BUTTON_HOVER);
        }
        self.p.label(r.shrink(4), label, if on || hover { t::TEXT_HI } else { t::TEXT }, Align::Center);
        self.clicked(r)
    }
    /// A header menu title ("File", "View", ...). Returns true when it should open.
    pub fn menu_title(&mut self, r: Rect, label: &str) -> bool {
        let hover = self.hovered(r);
        if hover {
            self.p.round_rect(r, 4, t::BUTTON_HOVER);
        }
        self.p.label(r, label, if hover { t::TEXT_HI } else { t::TEXT }, Align::Center);
        self.clicked(r)
    }
    /// Open `items` as a popup under `r`.
    pub fn open_menu(&mut self, r: Rect, title: &str, items: Vec<MenuItem>) {
        let mut pop = Popup::new(title, items, r.x, r.bottom() + 2);
        pop.owner = r;
        self.st.open_popup = Some(pop);
    }
    /// A dropdown showing `value`; returns true when it was clicked (caller opens the menu).
    pub fn dropdown(&mut self, r: Rect, value: &str) -> bool {
        let hover = self.hovered(r);
        self.p.round_rect(r, 4, if hover { t::BUTTON_DARK_HOVER } else { t::BUTTON_DARK });
        self.p.label(Rect::new(r.x + 6, r.y, r.w - 20, r.h), value, t::TEXT, Align::Left);
        self.p.arrow((r.right() - 9) as f32, (r.y + r.h / 2) as f32, 7.0, true, t::TEXT_DIM);
        self.clicked(r)
    }
    pub fn checkbox(&mut self, r: Rect, label: &str, value: &mut bool) -> bool {
        let b = Rect::new(r.x, r.y + (r.h - 14) / 2, 14, 14);
        let hover = self.hovered(r);
        self.p.round_rect(b, 3, if *value { t::SELECTED } else if hover { t::BUTTON_DARK_HOVER } else { t::BUTTON_DARK });
        if *value {
            check_mark(self.p, Rect::new(b.x + 1, b.y + 1, 12, 12));
        }
        self.p.label(Rect::new(r.x + 20, r.y, r.w - 20, r.h), label, t::TEXT, Align::Left);
        if self.clicked(r) {
            *value = !*value;
            return true;
        }
        false
    }
    /// A row of toggle buttons joined together (e.g. X Y Z, select modes).
    pub fn segmented(&mut self, r: Rect, labels: &[&str], selected: &mut usize) -> bool {
        let n = labels.len().max(1) as i32;
        let w = r.w / n;
        let mut changed = false;
        for (i, l) in labels.iter().enumerate() {
            let br = Rect::new(r.x + i as i32 * w, r.y, if i as i32 == n - 1 { r.w - w * (n - 1) } else { w - 1 }, r.h);
            let on = *selected == i;
            let hover = self.hovered(br);
            self.p.fill(br, if on { t::SELECTED } else if hover { t::BUTTON_HOVER } else { t::BUTTON });
            self.p.label(br, l, if on { t::TEXT_HI } else { t::TEXT }, Align::Center);
            if self.clicked(br) {
                *selected = i;
                changed = true;
            }
        }
        changed
    }

    /// Blender's number field: drag left/right to change by `step` per 10 px, click
    /// without dragging to type a value, Enter to accept, Esc to cancel.
    /// Returns true when the value changed this frame.
    pub fn number(&mut self, wid: Id, r: Rect, label: &str, value: &mut f32, step: f32, unit: &str) -> bool {
        self.number_ex(wid, r, label, value, NumOpts { step, unit, ..NumOpts::default() })
    }

    /// [`Ui::number`] with limits, integer rounding and Blender's slider look
    /// (a blue bar filled to the value, for 0..1 factors like Roughness).
    pub fn number_ex(&mut self, wid: Id, r: Rect, label: &str, value: &mut f32, o: NumOpts<'_>) -> bool {
        let before = *value;
        let changed = self.number_raw(wid, r, label, value, &o);
        if changed {
            let mut v = value.clamp(o.min, o.max);
            if o.int {
                v = v.round();
            }
            *value = if v.is_finite() { v } else { before };
        }
        changed && *value != before
    }

    fn number_raw(&mut self, wid: Id, r: Rect, label: &str, value: &mut f32, o: &NumOpts<'_>) -> bool {
        let (step, unit) = (o.step, o.unit);
        let hover = self.hovered(r);
        let editing = matches!(&self.st.editing, Some((e, _)) if *e == wid);
        let dragging = self.st.active == Some(wid);
        if o.slider && !editing {
            self.p.round_rect(r, 4, if hover || dragging { t::BUTTON_DARK_HOVER } else { t::BUTTON_DARK });
            let f = ((*value - o.min) / (o.max - o.min)).clamp(0.0, 1.0);
            let fw = (r.w as f32 * f).round() as i32;
            if fw > 0 {
                self.p.push_clip(Rect::new(r.x, r.y, fw, r.h));
                self.p.round_rect(r, 4, if hover || dragging { t::SELECTED_HOVER } else { t::SELECTED });
                self.p.pop_clip();
            }
        } else {
            self.p.round_rect(r, 4, if editing { t::BUTTON_DARK } else if dragging { t::SELECTED } else if hover { t::BUTTON_HOVER } else { t::BUTTON });
        }
        let mut changed = false;
        if editing {
            let text = self.st.editing.as_ref().map(|e| e.1.clone()).unwrap_or_default();
            self.p.label(Rect::new(r.x + 6, r.y, r.w - 12, r.h), &format!("{text}|"), t::TEXT_HI, Align::Left);
            if let Some(Input::Key { key, text: ch, .. }) = self.input {
                self.input = None;
                match key {
                    Key::Enter | Key::NumEnter => {
                        if let Some(v) = eval_number(&text) {
                            *value = v;
                            changed = true;
                        }
                        self.st.editing = None;
                    }
                    Key::Esc => self.st.editing = None,
                    Key::Backspace => {
                        if let Some(e) = &mut self.st.editing {
                            if self.st.edit_select_all {
                                e.1.clear();
                            } else {
                                e.1.pop();
                            }
                        }
                    }
                    _ => {
                        if let (Some(c), Some(e)) = (ch, &mut self.st.editing) {
                            if c.is_ascii_graphic() {
                                if self.st.edit_select_all {
                                    e.1.clear();
                                }
                                e.1.push(c);
                            }
                        }
                    }
                }
                self.st.edit_select_all = false;
            } else if let Some(Input::Down { x, y, .. }) = self.input {
                if !r.contains(x, y) {
                    if let Some(v) = eval_number(&text) {
                        *value = v;
                        changed = true;
                    }
                    self.st.editing = None;
                }
            }
            return changed;
        }
        let shown = if o.int { format!("{}{}", value.round() as i64, if unit.is_empty() { String::new() } else { format!(" {unit}") }) } else { format_number(*value, unit) };
        if label.is_empty() {
            self.p.label(r.shrink(4), &shown, t::TEXT, Align::Center);
        } else {
            self.p.label(Rect::new(r.x + 6, r.y, r.w - 12, r.h), label, t::TEXT, Align::Left);
            self.p.label(Rect::new(r.x + 6, r.y, r.w - 12, r.h), &shown, t::TEXT, Align::Right);
        }
        match self.input {
            Some(Input::Down { x, y, button: Button::Left, .. }) if r.contains(x, y) && self.p.clip().contains(x, y) => {
                self.input = None;
                self.st.active = Some(wid);
                self.st.drag_origin = (x, y);
                self.st.drag_start_value = *value;
                self.st.drag_moved = false;
            }
            Some(Input::Move { x, .. }) if dragging => {
                self.input = None;
                let dx = x - self.st.drag_origin.0;
                if dx.abs() > 2 {
                    self.st.drag_moved = true;
                }
                if self.st.drag_moved {
                    *value = if o.slider && o.max.is_finite() && o.min.is_finite() {
                        // Sliders follow the pointer across their width.
                        self.st.drag_start_value + dx as f32 / r.w.max(1) as f32 * (o.max - o.min)
                    } else {
                        self.st.drag_start_value + (dx as f32 / 10.0) * step
                    };
                    changed = true;
                }
            }
            Some(Input::Up { button: Button::Left, .. }) if dragging => {
                self.input = None;
                self.st.active = None;
                if !self.st.drag_moved {
                    self.st.editing = Some((wid, trim_number(*value)));
                    self.st.edit_select_all = true;
                }
            }
            _ => {}
        }
        changed
    }

    /// A colour swatch (linear RGB shown in sRGB, like Blender's colour
    /// buttons). Returns true when clicked; the caller opens its picker.
    pub fn swatch(&mut self, r: Rect, rgb: [f32; 3]) -> bool {
        let enc = |c: f32| (sex_3d::render::srgb_encode(c.clamp(0.0, 1.0)) * 255.0).round() as u32;
        let c = enc(rgb[0]) << 16 | enc(rgb[1]) << 8 | enc(rgb[2]);
        self.p.round_rect(r, 4, c);
        self.p.round_frame(r, 4, if self.hovered(r) { t::BUTTON_HOVER } else { 0x1d1d1d });
        self.clicked(r)
    }

    /// Single-line text field. Returns true when Enter committed a change.
    pub fn text_field(&mut self, wid: Id, r: Rect, value: &mut String) -> bool {
        let editing = matches!(&self.st.editing, Some((e, _)) if *e == wid);
        let hover = self.hovered(r);
        self.p.round_rect(r, 4, if editing || hover { t::BUTTON_DARK_HOVER } else { t::BUTTON_DARK });
        if !editing {
            self.p.label(Rect::new(r.x + 6, r.y, r.w - 12, r.h), value, t::TEXT, Align::Left);
            if self.clicked(r) {
                // Blender enters text editing with everything selected.
                self.st.editing = Some((wid, value.clone()));
                self.st.edit_select_all = true;
            }
            return false;
        }
        let text = self.st.editing.as_ref().map(|e| e.1.clone()).unwrap_or_default();
        self.p.label(Rect::new(r.x + 6, r.y, r.w - 12, r.h), &format!("{text}|"), t::TEXT_HI, Align::Left);
        match self.input {
            Some(Input::Key { key, text: ch, .. }) => {
                self.input = None;
                match key {
                    Key::Enter | Key::NumEnter => {
                        let changed = *value != text;
                        *value = text;
                        self.st.editing = None;
                        return changed;
                    }
                    Key::Esc => self.st.editing = None,
                    Key::Backspace => {
                        if let Some(e) = &mut self.st.editing {
                            if self.st.edit_select_all {
                                e.1.clear();
                            } else {
                                e.1.pop();
                            }
                        }
                    }
                    _ => {
                        if let (Some(c), Some(e)) = (ch, &mut self.st.editing) {
                            if !c.is_control() {
                                if self.st.edit_select_all {
                                    e.1.clear();
                                }
                                e.1.push(c);
                            }
                        }
                    }
                }
                self.st.edit_select_all = false;
            }
            Some(Input::Down { x, y, .. }) if !r.contains(x, y) => {
                let changed = *value != text;
                *value = text;
                self.st.editing = None;
                return changed;
            }
            _ => {}
        }
        false
    }

    /// A collapsible panel header like Blender's properties panels. Returns `open`.
    pub fn panel_header(&mut self, r: Rect, title: &str, open: &mut bool) -> bool {
        self.p.round_rect(r, 4, t::PANEL_HEADER);
        self.p.arrow((r.x + 12) as f32, (r.y + r.h / 2) as f32, 8.0, *open, t::TEXT_DIM);
        self.p.label(Rect::new(r.x + 22, r.y, r.w - 28, r.h), title, t::TEXT, Align::Left);
        if self.clicked(r) {
            *open = !*open;
        }
        *open
    }
}

/// Options for [`Ui::number_ex`].
#[derive(Clone, Copy, Debug)]
pub struct NumOpts<'u> {
    /// Change per 10 px of dragging.
    pub step: f32,
    pub unit: &'u str,
    pub min: f32,
    pub max: f32,
    /// Draw as a slider filled from `min` to the value.
    pub slider: bool,
    /// Whole numbers only.
    pub int: bool,
}

impl Default for NumOpts<'static> {
    fn default() -> Self {
        NumOpts { step: 0.1, unit: "", min: f32::NEG_INFINITY, max: f32::INFINITY, slider: false, int: false }
    }
}

/// Blender shows lengths like "1 m", "0.25 m", angles "90°".
pub fn format_number(v: f32, unit: &str) -> String {
    let s = trim_number(v);
    if unit.is_empty() {
        s
    } else if unit == "°" {
        format!("{s}°")
    } else {
        format!("{s} {unit}")
    }
}

pub fn trim_number(v: f32) -> String {
    let mut s = format!("{:.4}", v);
    while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
        s.pop();
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

/// Numbers typed into fields may be small expressions, as in Blender ("2*3", "1/4", "-0.5+1").
pub fn eval_number(s: &str) -> Option<f32> {
    let s: String = s.chars().filter(|c| !c.is_whitespace() && *c != '°' && *c != 'm').collect();
    fn expr(b: &[u8], i: &mut usize) -> Option<f64> {
        let mut v = term(b, i)?;
        while *i < b.len() && (b[*i] == b'+' || b[*i] == b'-') {
            let op = b[*i];
            *i += 1;
            let r = term(b, i)?;
            v = if op == b'+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn term(b: &[u8], i: &mut usize) -> Option<f64> {
        let mut v = factor(b, i)?;
        while *i < b.len() && (b[*i] == b'*' || b[*i] == b'/') {
            let op = b[*i];
            *i += 1;
            let r = factor(b, i)?;
            v = if op == b'*' { v * r } else { v / r };
        }
        Some(v)
    }
    fn factor(b: &[u8], i: &mut usize) -> Option<f64> {
        if *i < b.len() && b[*i] == b'-' {
            *i += 1;
            return factor(b, i).map(|v| -v);
        }
        if *i < b.len() && b[*i] == b'(' {
            *i += 1;
            let v = expr(b, i)?;
            if *i < b.len() && b[*i] == b')' {
                *i += 1;
            }
            return Some(v);
        }
        let start = *i;
        while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == b'.') {
            *i += 1;
        }
        core::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
    }
    let b = s.as_bytes();
    let mut i = 0;
    let v = expr(b, &mut i)?;
    (i == b.len() && v.is_finite()).then_some(v as f32)
}

/// Does `input` match a hotkey?
pub fn is_key(input: &Option<Input>, key: Key, mods: Mods) -> bool {
    matches!(input, Some(Input::Key { key: k, mods: m, .. }) if *k == key && *m == mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_format_and_evaluate_like_blender() {
        assert_eq!(format_number(1.0, "m"), "1 m");
        assert_eq!(format_number(0.25, "m"), "0.25 m");
        assert_eq!(format_number(90.0, "°"), "90°");
        assert_eq!(eval_number("2*3"), Some(6.0));
        assert_eq!(eval_number("1/4 m"), Some(0.25));
        assert_eq!(eval_number("-(1+1)"), Some(-2.0));
        assert_eq!(eval_number("abc"), None);
    }
}
