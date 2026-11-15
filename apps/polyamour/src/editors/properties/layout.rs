// SPDX-License-Identifier: GPL-3.0-only
//! The Properties editor's layout engine: collapsible panels and sub-panels
//! stacked down the region, and Blender's "property split" rows (label right-
//! aligned in the left column, the button in the right one).
//!
//! Property search runs every panel once "dry" (nothing drawn, no input) to
//! count matching labels; panels without a match are hidden and the others
//! are shown open with the matching labels highlighted, as Blender does.
use super::icons::{self, Icon};
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::input::Input;
use crate::theme as t;
use crate::ui::{id, MenuItem, NumOpts, Op, Ui};
use std::collections::HashMap;

pub const ROW: i32 = 20;
pub const GAP: i32 = 2;
pub const PANEL_HEAD: i32 = 26;
pub const PANEL_BG: u32 = 0x3d3d3d;
const SUB_LINE: u32 = 0x333333;

pub struct L<'a, 'b> {
    pub ui: &'a mut Ui<'b>,
    /// Panel open states by key.
    pub open: &'a mut HashMap<String, bool>,
    /// The panel column.
    pub x: i32,
    pub w: i32,
    pub y: i32,
    /// Lower-case search text; empty when not searching.
    pub query: String,
    pub dry: bool,
    pub hits: usize,
    /// Sub-panel nesting (indents the header).
    pub depth: i32,
    /// Grey out and ignore input (Blender's inactive buttons).
    pub inactive: bool,
    /// Widget-id prefix (per tab).
    pub prefix: String,
    /// The dragged widget that already pushed its undo step.
    pub undo_for: Option<Option<u64>>,
    /// Where each property's button was drawn (by key and by label), for
    /// tests and for the editor to find widgets.
    pub fields: Vec<(String, Rect)>,
}

impl<'a, 'b> L<'a, 'b> {
    /// Content area inside a panel.
    pub fn inner(&self) -> (i32, i32) {
        (self.x + 10, self.w - 20)
    }
    fn split(&self) -> (Rect, Rect) {
        let (x, w) = self.inner();
        let lw = (w as f32 * 0.4) as i32;
        (Rect::new(x, self.y, lw - 8, ROW), Rect::new(x + lw, self.y, w - lw, ROW))
    }
    /// A property changed: one undo step per edit (a whole drag is one edit),
    /// and the viewports refresh.
    pub fn commit(&mut self, d: &mut Doc, label: &str) {
        let active = self.ui.st.active;
        if active.is_none() || self.undo_for != Some(active) {
            d.push_undo(label);
            self.undo_for = Some(active);
        }
        d.touch();
    }
    pub fn wid(&self, key: &str) -> u64 {
        id(&format!("{}/{key}", self.prefix))
    }

    /// Does `label` match the search? Counts hits in the dry pass.
    fn matches(&mut self, label: &str) -> bool {
        if self.query.is_empty() || label.is_empty() {
            return false;
        }
        let m = label.to_lowercase().contains(&self.query);
        if m && self.dry {
            self.hits += 1;
        }
        m
    }

    /// Reserve a row of height `h` with the panel background behind it.
    /// Returns false in the dry pass (nothing to draw).
    pub fn band(&mut self, h: i32) -> bool {
        if self.dry {
            return false;
        }
        if self.depth > 0 {
            self.ui.p.fill(Rect::new(self.x, self.y, self.w, h + GAP), PANEL_BG);
        }
        true
    }
    /// A data-block row outside the panels: icon and name, renamable when
    /// `key` is given. Returns the new name after Enter.
    pub fn id_block(&mut self, icon: Icon, name: &str, key: Option<&str>) -> Option<String> {
        if !self.band(ROW) {
            return None;
        }
        let (x, w) = (self.x + 4, self.w - 8);
        let r = Rect::new(x + 22, self.y, w - 22, ROW);
        icons::draw(self.ui.p, icon, x + 10, self.y + ROW / 2);
        let out = match key {
            Some(k) => {
                let wid = self.wid(k);
                let mut n = name.to_string();
                self.fields.push((k.to_string(), r));
                self.ui.text_field(wid, r, &mut n).then_some(n)
            }
            None => {
                self.ui.p.round_rect(r, 4, t::BUTTON_DARK);
                self.ui.p.label(Rect::new(r.x + 6, r.y, r.w - 8, r.h), name, t::TEXT, Align::Left);
                None
            }
        };
        self.y += ROW + 8;
        out
    }
    fn advance(&mut self, h: i32) {
        self.y += h + GAP;
    }

    /// Run `f` with input withheld when inactive, then grey the row out.
    fn guarded<R>(&mut self, h: i32, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = if self.inactive { self.ui.input.take() } else { None };
        let y = self.y;
        let r = f(self);
        if self.inactive {
            self.ui.input = saved;
            self.ui.p.fill_alpha(Rect::new(self.x, y, self.w, h), PANEL_BG, 140);
        }
        r
    }

    fn draw_label(&mut self, r: Rect, label: &str, hit: bool) {
        if hit {
            let w = self.ui.p.text_width(label) + 6;
            self.ui.p.round_rect(Rect::new(r.right() - w + 2, r.y + 2, w, r.h - 4), 3, 0x2f4a73);
        }
        self.ui.p.label(r, label, if hit { t::TEXT_HI } else { t::TEXT }, Align::Right);
    }

    /// A number property in the split layout.
    pub fn number(&mut self, label: &str, key: &str, v: &mut f32, o: NumOpts<'_>) -> bool {
        let hit = self.matches(label);
        if !self.band(ROW) {
            return false;
        }
        let (lr, fr) = self.split();
        self.fields.push((key.to_string(), fr));
        self.fields.push((label.to_string(), fr));
        let wid = self.wid(key);
        let changed = self.guarded(ROW, |l| {
            l.draw_label(lr, label, hit);
            l.ui.number_ex(wid, fr, "", v, o)
        });
        self.advance(ROW);
        changed
    }
    pub fn int(&mut self, label: &str, key: &str, v: &mut i64, min: i64, max: i64) -> bool {
        let mut f = *v as f32;
        let c = self.number(label, key, &mut f, NumOpts { step: 1.0, int: true, min: min as f32, max: max as f32, ..NumOpts::default() });
        if c {
            *v = f as i64;
        }
        c
    }
    pub fn uint(&mut self, label: &str, key: &str, v: &mut u32, min: u32, max: u32) -> bool {
        let mut i = *v as i64;
        let c = self.int(label, key, &mut i, min as i64, max as i64);
        if c {
            *v = i as u32;
        }
        c
    }
    /// A whole-number field with a unit ("1920 px").
    pub fn number_u32(&mut self, label: &str, key: &str, v: &mut u32, unit: &str) -> bool {
        let mut f = *v as f32;
        let c = self.number(label, key, &mut f, NumOpts { step: 1.0, unit, min: 1.0, max: 65536.0, int: true, ..NumOpts::default() });
        if c {
            *v = f as u32;
        }
        c
    }
    pub fn slider(&mut self, label: &str, key: &str, v: &mut f32, min: f32, max: f32) -> bool {
        self.number(label, key, v, NumOpts { step: (max - min) / 100.0, min, max, slider: true, ..NumOpts::default() })
    }
    /// Blender's checkbox in the split layout: an optional heading on the
    /// left, the box and its text on the right.
    pub fn check(&mut self, heading: &str, label: &str, v: &mut bool) -> bool {
        let hit = self.matches(label) | self.matches(heading);
        if !self.band(ROW) {
            return false;
        }
        let (lr, fr) = self.split();
        self.fields.push((label.to_string(), fr));
        let changed = self.guarded(ROW, |l| {
            l.draw_label(lr, heading, hit && !heading.is_empty());
            l.ui.checkbox(fr, label, v)
        });
        self.advance(ROW);
        changed
    }
    /// A dropdown showing `value`; picking an item queues `properties.set`
    /// with `key=ITEM` for the editor to apply.
    pub fn enumeration(&mut self, label: &str, key: &str, value: &str, items: &[&str]) {
        let hit = self.matches(label);
        if !self.band(ROW) {
            return;
        }
        let (lr, fr) = self.split();
        let fr = if label.is_empty() { Rect::new(self.inner().0, fr.y, self.inner().1, fr.h) } else { fr };
        self.fields.push((key.to_string(), fr));
        self.fields.push((label.to_string(), fr));
        self.guarded(ROW, |l| {
            l.draw_label(lr, label, hit);
            if l.ui.dropdown(fr, value) {
                let menu = items.iter().map(|it| if it.is_empty() { MenuItem::Separator } else { MenuItem::op(it, "", Op::with("properties.set", &format!("{key}={it}"))) }).collect();
                l.ui.open_menu(fr, label, menu);
            }
        });
        self.advance(ROW);
    }
    /// A dropdown showing `value` whose menu is `items` (operators of the
    /// caller's choosing). Returns the field when drawn.
    pub fn dropdown_row(&mut self, label: &str, key: &str, value: &str, items: &dyn Fn() -> Vec<MenuItem>) -> Option<Rect> {
        let hit = self.matches(label);
        if !self.band(ROW) {
            return None;
        }
        let (lr, fr) = self.split();
        let fr = if label.is_empty() { Rect::new(self.inner().0, fr.y, self.inner().1, fr.h) } else { fr };
        self.fields.push((key.to_string(), fr));
        self.fields.push((label.to_string(), fr));
        self.guarded(ROW, |l| {
            l.draw_label(lr, label, hit);
            if l.ui.dropdown(fr, value) {
                l.ui.open_menu(fr, label, items());
            }
        });
        self.advance(ROW);
        Some(fr)
    }
    /// A row of joined toggle buttons (Point / Sun / Spot / Area). Returns the
    /// newly picked index.
    pub fn expand(&mut self, label: &str, items: &[&str], current: usize) -> Option<usize> {
        let hit = self.matches(label) | items.iter().any(|i| self.query.len() > 0 && i.to_lowercase().contains(&self.query));
        if hit && self.dry && label.is_empty() {
            self.hits += 1;
        }
        if !self.band(ROW) {
            return None;
        }
        let (lr, fr) = self.split();
        let fr = if label.is_empty() { Rect::new(self.inner().0, fr.y, self.inner().1, fr.h) } else { fr };
        self.fields.push((label.to_string(), fr));
        let mut sel = current;
        let changed = self.guarded(ROW, |l| {
            l.draw_label(lr, label, hit && !label.is_empty());
            l.ui.segmented(fr, items, &mut sel)
        });
        self.advance(ROW);
        (changed && sel != current).then_some(sel)
    }
    pub fn text(&mut self, label: &str, key: &str, v: &mut String) -> bool {
        let hit = self.matches(label);
        if !self.band(ROW) {
            return false;
        }
        let (lr, fr) = self.split();
        self.fields.push((key.to_string(), fr));
        self.fields.push((label.to_string(), fr));
        let wid = self.wid(key);
        let c = self.guarded(ROW, |l| {
            l.draw_label(lr, label, hit);
            l.ui.text_field(wid, fr, v)
        });
        self.advance(ROW);
        c
    }
    /// A colour swatch property; returns its rectangle when clicked.
    pub fn color(&mut self, label: &str, rgb: [f32; 3]) -> Option<Rect> {
        let hit = self.matches(label);
        if !self.band(ROW) {
            return None;
        }
        let (lr, fr) = self.split();
        self.fields.push((label.to_string(), fr));
        let c = self.guarded(ROW, |l| {
            l.draw_label(lr, label, hit);
            l.ui.swatch(fr, rgb)
        });
        self.advance(ROW);
        c.then_some(fr)
    }
    /// A full-width push button.
    pub fn button(&mut self, label: &str) -> bool {
        let _ = self.matches(label);
        if !self.band(ROW) {
            return false;
        }
        let (x, w) = self.inner();
        let r = Rect::new(x, self.y, w, ROW);
        self.fields.push((label.to_string(), r));
        let c = self.guarded(ROW, |l| l.ui.button(r, label));
        self.advance(ROW);
        c
    }
    /// A push button in the field column.
    pub fn field_button(&mut self, label: &str, text: &str) -> bool {
        let hit = self.matches(label);
        if !self.band(ROW) {
            return false;
        }
        let (lr, fr) = self.split();
        self.fields.push((label.to_string(), fr));
        let c = self.guarded(ROW, |l| {
            l.draw_label(lr, label, hit);
            l.ui.button(fr, text)
        });
        self.advance(ROW);
        c
    }
    /// Read-only text (label: value).
    pub fn info(&mut self, label: &str, value: &str) {
        let hit = self.matches(label);
        if !self.band(ROW) {
            return;
        }
        let (lr, fr) = self.split();
        self.fields.push((label.to_string(), fr));
        self.draw_label(lr, label, hit);
        self.ui.p.label(Rect::new(fr.x + 4, fr.y, fr.w - 4, fr.h), value, t::TEXT_DIM, Align::Left);
        self.advance(ROW);
    }
    /// A grey note across the panel.
    pub fn note(&mut self, s: &str) {
        if !self.band(ROW) {
            return;
        }
        let (x, w) = self.inner();
        self.ui.p.label(Rect::new(x, self.y, w, ROW), s, t::TEXT_DIM, Align::Left);
        self.advance(ROW);
    }
    pub fn space(&mut self, h: i32) {
        if self.band(h - GAP) {
            self.y += h;
        }
    }
    /// An empty Blender list box with + / - at its right (vertex groups,
    /// shape keys, particle systems). Returns 1 for +, 2 for -.
    pub fn list_box(&mut self, rows: &[(Icon, String)], active: usize, min_rows: usize) -> (u8, Option<usize>) {
        let n = rows.len().max(min_rows) as i32;
        let h = n * ROW + 8;
        if !self.band(h) {
            return (0, None);
        }
        let (x, w) = self.inner();
        let bx = Rect::new(x, self.y, w - 28, h);
        self.ui.p.round_rect(bx, 4, 0x282828);
        let mut picked = None;
        for (i, (ic, name)) in rows.iter().enumerate() {
            let rr = Rect::new(bx.x + 4, bx.y + 4 + i as i32 * ROW, bx.w - 8, ROW);
            if i == active {
                self.ui.p.round_rect(rr, 3, t::OUTLINER_ACTIVE);
            }
            icons::draw(self.ui.p, *ic, rr.x + 10, rr.y + ROW / 2);
            self.ui.p.label(Rect::new(rr.x + 22, rr.y, rr.w - 24, ROW), name, t::TEXT, Align::Left);
            if !self.inactive && self.ui.clicked(rr) {
                picked = Some(i);
            }
        }
        let mut res = 0;
        let plus = Rect::new(bx.right() + 4, self.y, 24, ROW);
        let minus = Rect::new(bx.right() + 4, self.y + ROW + 1, 24, ROW);
        for (r, ic, v) in [(plus, Icon::Plus, 1u8), (minus, Icon::Minus, 2)] {
            let hov = self.ui.hovered(r);
            self.ui.p.round_rect(r, 4, if hov { t::BUTTON_HOVER } else { t::BUTTON });
            icons::draw(self.ui.p, ic, r.x + r.w / 2, r.y + r.h / 2);
            if self.ui.clicked(r) {
                res = v;
            }
        }
        self.advance(h);
        (res, picked)
    }

    /// A panel (or sub-panel when called inside one). `header` draws extra
    /// header widgets into the rectangle right of the title. Returns true if
    /// the body was shown.
    pub fn panel(&mut self, d: &mut Doc, key: &str, title: &str, default_open: bool, body: &mut dyn FnMut(&mut L, &mut Doc)) -> bool {
        self.panel_with(d, key, title, default_open, &mut |_, _, _| {}, body)
    }

    pub fn panel_with(
        &mut self,
        d: &mut Doc,
        key: &str,
        title: &str,
        default_open: bool,
        header: &mut dyn FnMut(&mut L, &mut Doc, Rect),
        body: &mut dyn FnMut(&mut L, &mut Doc),
    ) -> bool {
        let searching = !self.query.is_empty();
        let title_hit = searching && title.to_lowercase().contains(&self.query);
        if self.dry {
            // Counting pass of an enclosing panel.
            if title_hit {
                self.hits += 1;
            }
            body(self, d);
            return false;
        }
        if searching {
            // Count this panel's matches without drawing.
            let saved_y = self.y;
            let saved_input = self.ui.input.take();
            self.dry = true;
            self.hits = 0;
            body(self, d);
            self.dry = false;
            self.ui.input = saved_input;
            self.y = saved_y;
            if self.hits == 0 && !title_hit {
                return false;
            }
        }
        let sub = self.depth > 0;
        let open_now = *self.open.entry(key.to_string()).or_insert(default_open) || searching;
        let hh = if sub { 24 } else { PANEL_HEAD };
        let hr = Rect::new(self.x, self.y, self.w, hh);
        if sub {
            self.ui.p.fill(Rect::new(self.x, self.y, self.w, hh), PANEL_BG);
            self.ui.p.fill(Rect::new(self.x + 6, self.y, self.w - 12, 1), SUB_LINE);
        } else {
            self.ui.p.round_rect(Rect::new(hr.x, hr.y, hr.w, if open_now { hh + 8 } else { hh }), 5, PANEL_BG);
        }
        let indent = if sub { 10 + (self.depth - 1) * 12 } else { 0 };
        let ax = hr.x + 12 + indent;
        self.ui.p.arrow(ax as f32, (hr.y + hh / 2) as f32, 8.0, open_now, t::TEXT_DIM);
        let tw = self.ui.p.text_width(title);
        let title_r = Rect::new(ax + 10, hr.y, tw + 4, hh);
        self.ui.p.label(title_r, title, if title_hit { t::TEXT_HI } else { t::TEXT }, Align::Left);
        let extra = Rect::new(title_r.right() + 6, hr.y + 3, (hr.right() - 8 - title_r.right() - 6).max(0), hh - 6);
        header(self, d, extra);
        // Ctrl+click on a header collapses every other panel, like Blender.
        if let Some(Input::Down { x, y, button: crate::input::Button::Left, mods }) = self.ui.input {
            if hr.contains(x, y) && self.ui.p.clip().contains(x, y) {
                self.ui.input = None;
                if mods.ctrl {
                    for v in self.open.values_mut() {
                        *v = false;
                    }
                    self.open.insert(key.to_string(), true);
                } else {
                    let e = self.open.entry(key.to_string()).or_insert(default_open);
                    *e = !*e;
                }
            }
        }
        self.y += hh;
        if !open_now {
            if !sub {
                self.y += 4;
            }
            return false;
        }
        if !sub {
            self.space(4);
        }
        self.depth += 1;
        body(self, d);
        self.depth -= 1;
        if !sub {
            // Rounded bottom.
            let clip = Rect::new(self.x, self.y, self.w, 6);
            self.ui.p.push_clip(clip);
            self.ui.p.round_rect(Rect::new(self.x, self.y - 8, self.w, 14), 5, PANEL_BG);
            self.ui.p.pop_clip();
            self.y += 10;
        } else {
            self.space(2);
        }
        true
    }
}
