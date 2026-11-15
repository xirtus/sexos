// SPDX-License-Identifier: GPL-3.0-only
//! Input: keys and the pointer turned into widget behaviour and [`Response`]s.
//!
//! The keyboard model is the same in every program: Tab and Shift+Tab move focus,
//! arrows move inside a widget, Space and Enter press, Escape backs out, F10 opens the
//! menu bar, Shift+F10 or the Menu key opens a context menu, Ctrl+Shift+P opens the
//! command palette, and document tabs answer Ctrl+Tab, Ctrl+W, Ctrl+T and Ctrl+1..9.
use alloc::string::String;
use alloc::vec::Vec;

use crate::geom::{Point, Rect};
use crate::input::{code, Button, Event, Key, Nav, Pointer, PointerKind};
use crate::menu::Menu;
use crate::text::Face;
use crate::ui::{tab_strip_height, Drag, LayerKind, Ui, SPLIT_GAP};
use crate::widget::*;
use crate::{Response, TabEvent};

/// Two clicks closer than this (ms) are a double click.
const DOUBLE_MS: u64 = 450;
/// Width of the scroll bars of lists and scroll views.
pub(crate) const BAR_W: f32 = 10.0;
/// A tab dragged this far above or below its strip comes off it.
const TEAR_OFF: f32 = 48.0;

/// PS/2 set 1 codes of the letters editing shortcuts use.
mod letter {
    pub const A: u8 = 0x1E;
    pub const C: u8 = 0x2E;
    pub const V: u8 = 0x2F;
    pub const X: u8 = 0x2D;
    pub const Y: u8 = 0x15;
    pub const Z: u8 = 0x2C;
}

/// Where a list's parts are.
pub(crate) struct ListGeom {
    pub header: Rect,
    pub rows: Rect,
    pub row_h: f32,
    pub content: f32,
    pub bar: Option<Rect>,
}

impl Ui {
    // ---- entry points ------------------------------------------------------------

    /// Feeds one event in; returns what the program should act on.
    pub fn handle(&mut self, ev: Event) -> Vec<Response> {
        self.layout();
        match ev {
            Event::Key(k) => self.on_key(k),
            Event::Pointer(p) => self.on_pointer(p),
            Event::Resize { width, height } => self.resize(width, height),
            Event::Close => self.out.push(Response::Close),
        }
        self.layout();
        core::mem::take(&mut self.out)
    }

    /// Runs a command as if its shortcut were pressed (Weft's own `weft.*` commands
    /// act here; others come back as [`Response::Command`]).
    pub fn run(&mut self, command: &str) -> Vec<Response> {
        self.run_command(command, None);
        self.layout();
        core::mem::take(&mut self.out)
    }

    /// What the system clipboard holds, for Paste (the glue keeps it current).
    pub fn set_clipboard(&mut self, text: &str) { self.clipboard.clear(); self.clipboard.push_str(text); }

    /// Routes a response: those about Weft's own overlays are handled here.
    pub(crate) fn emit(&mut self, r: Response) {
        let id = match &r { Response::Clicked(i) | Response::Changed(i) | Response::Submitted(i) | Response::Selected(i) | Response::Activated(i, _) | Response::Toggled(i, _) => Some(*i), _ => None };
        if let Some(id) = id {
            if self.layer_root(id) != self.root() && self.overlay_response(&r) { return; }
        }
        if let Response::Toggled(..) | Response::Changed(_) | Response::Selected(_) = r { self.a11y_dirty = true; }
        self.out.push(r);
    }

    /// The root of the layer `id` is in.
    pub(crate) fn layer_root(&self, mut id: Id) -> Id {
        while let Some(p) = self.get(id).and_then(|n| n.parent) { id = p; }
        id
    }

    // ---- commands ----------------------------------------------------------------

    /// Runs `id`; `tab` names the tab a tab strip's context menu was opened on.
    pub(crate) fn run_command(&mut self, id: &str, tab: Option<(Id, usize)>) {
        if let Some(c) = self.commands.get(id) { if !c.enabled { return; } }
        let doc = tab.map(|t| t.0).or_else(|| self.doc_tabs());
        let active = |ui: &Ui, t: Id| tab.map(|x| x.1).unwrap_or_else(|| ui.active_tab(t));
        match id {
            "weft.palette" => self.open_palette(),
            "weft.tab-next" | "weft.tab-previous" => if let Some(t) = doc {
                let n = self.tab_list(t).len();
                if n > 1 {
                    let a = self.active_tab(t);
                    let to = if id == "weft.tab-next" { (a + 1) % n } else { (a + n - 1) % n };
                    self.select_tab(t, to);
                }
            },
            "weft.tab-close" => if let Some(t) = doc {
                let i = active(self, t);
                if i < self.tab_list(t).len() { self.out.push(Response::Tab(t, TabEvent::Closed(i))); }
            },
            "weft.tab-close-others" => if let Some(t) = doc { let i = active(self, t); self.out.push(Response::Tab(t, TabEvent::CloseOthers(i))); },
            "weft.tab-new" => if let Some(t) = doc { self.out.push(Response::Tab(t, TabEvent::New)); },
            "weft.tab-pin" => if let Some(t) = doc { let i = active(self, t); self.toggle_pin(t, i); },
            "weft.cut" | "weft.copy" | "weft.paste" | "weft.select-all" | "weft.undo" | "weft.redo" => {
                let Some(f) = self.focus.filter(|f| matches!(self.node(*f).kind, Kind::Field(_))) else { return };
                let code = match id { "weft.cut" => letter::X, "weft.copy" => letter::C, "weft.paste" => letter::V, "weft.select-all" => letter::A, "weft.undo" => letter::Z, _ => letter::Y };
                self.field_key(f, Key::ctrl(code));
            }
            _ => self.out.push(Response::Command(id.into())),
        }
    }

    fn toggle_pin(&mut self, tabs: Id, i: usize) {
        let Kind::Tabs(t) = &mut self.node_mut(tabs).kind else { return };
        let Some(tab) = t.tabs.get_mut(i) else { return };
        tab.pinned = !tab.pinned;
        let on = tab.pinned;
        let pinned = t.tabs.iter().filter(|x| x.pinned).count();
        // Pinned tabs go to the end of the pinned run; unpinned ones just after it.
        let to = if on { pinned - 1 } else { pinned };
        self.move_tab(tabs, i, to.min(self.tab_list(tabs).len() - 1));
        self.out.push(Response::Tab(tabs, TabEvent::Pinned(to, on)));
        self.damage_node(tabs);
    }

    pub(crate) fn select_tab(&mut self, tabs: Id, i: usize) {
        if i >= self.tab_list(tabs).len() { return; }
        if self.active_tab(tabs) != i {
            self.set_active_tab(tabs, i);
            self.out.push(Response::Tab(tabs, TabEvent::Selected(i)));
        }
    }

    // ---- keys --------------------------------------------------------------------

    fn on_key(&mut self, k: Key) {
        if !self.focus_visible { self.focus_visible = true; if let Some(f) = self.focus { self.damage_node(f); } }
        self.hide_tooltip();
        if !self.menus.is_empty() { self.menu_key(k); return; }
        if self.palette.is_some() { self.palette_key(k); return; }
        let nav = k.nav();
        let focus = self.focus.filter(|f| self.exists(*f) && self.shown(*f) && self.layer_root(*f) == self.key_layer_root());
        if let Some(d) = self.dialogs.last() {
            if nav == Some(Nav::Escape) { self.answer_dialog(crate::DialogValue::Cancelled); return; }
            let on_button = focus.map_or(false, |f| matches!(self.node(f).kind, Kind::Button { .. } | Kind::List(_)));
            if nav == Some(Nav::Enter) && !k.control && !on_button {
                let b = d.buttons[d.default];
                self.emit(Response::Clicked(b));
                return;
            }
        }
        if let Some(f) = focus { if matches!(self.node(f).kind, Kind::Field(_)) && self.field_key(f, k) { return; } }
        // Ctrl+1..9: the first eight tabs, and the last.
        if k.control && !k.shift && (0x02..=0x0A).contains(&k.code) {
            if let Some(t) = self.doc_tabs() {
                let n = self.tab_list(t).len();
                let i = if k.code == 0x0A { n.saturating_sub(1) } else { (k.code - 0x02) as usize };
                if i < n { self.select_tab(t, i); }
                return;
            }
        }
        if let Some(c) = self.commands.for_key(&k) { let id = c.id.clone(); self.run_command(&id, None); return; }
        if nav == Some(Nav::Tab) && !k.control { self.move_focus(!k.shift); return; }
        if k.code == code::F10 && !k.control {
            if k.shift { self.context_menu_for_focus(); } else if let Some(bar) = self.find_kind("menubar").into_iter().find(|b| self.shown(*b)) { self.open_bar_menu(bar, 0); }
            return;
        }
        if k.code == code::MENU { self.context_menu_for_focus(); return; }
        if let Some(f) = focus { if self.node(f).enabled { self.widget_key(f, k); } }
    }

    /// Tab (forward) or Shift+Tab through the focusable widgets.
    pub(crate) fn move_focus(&mut self, forward: bool) {
        let chain = self.focus_chain();
        if chain.is_empty() { return; }
        let at = self.focus.and_then(|f| chain.iter().position(|c| *c == f));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => chain.len() - 1,
            (Some(i), true) => (i + 1) % chain.len(),
            (Some(i), false) => (i + chain.len() - 1) % chain.len(),
        };
        let id = chain[next];
        self.focus(Some(id));
        // Tabbing into a field selects its text, as everywhere else.
        if let Some(f) = self.field_mut(id) { f.edit.select_all(); }
    }

    fn widget_key(&mut self, id: Id, k: Key) {
        let nav = k.nav();
        match &self.node(id).kind {
            Kind::Button { .. } if matches!(nav, Some(Nav::Space | Nav::Enter)) => self.activate(id),
            Kind::Check { .. } if nav == Some(Nav::Space) => self.activate(id),
            Kind::Radio { group, .. } => {
                let group = *group;
                match nav {
                    Some(Nav::Space) => self.activate(id),
                    Some(Nav::Up | Nav::Left | Nav::Down | Nav::Right) => {
                        let fwd = matches!(nav, Some(Nav::Down | Nav::Right));
                        let peers: Vec<Id> = self.focus_chain().into_iter().filter(|o| matches!(self.node(*o).kind, Kind::Radio { group: g, .. } if g == group)).collect();
                        if let Some(i) = peers.iter().position(|p| *p == id) {
                            let to = peers[if fwd { (i + 1) % peers.len() } else { (i + peers.len() - 1) % peers.len() }];
                            self.focus(Some(to));
                            self.activate(to);
                        }
                    }
                    _ => {}
                }
            }
            Kind::Slider { value, min, max, step } => {
                let (v, lo, hi) = (*value, *min, *max);
                let unit = if *step > 0.0 { *step } else { (hi - lo) / 100.0 };
                let to = match nav {
                    Some(Nav::Left | Nav::Down) => v - unit,
                    Some(Nav::Right | Nav::Up) => v + unit,
                    Some(Nav::PageDown) => v - unit * 10.0,
                    Some(Nav::PageUp) => v + unit * 10.0,
                    Some(Nav::Home) => lo,
                    Some(Nav::End) => hi,
                    _ => return,
                };
                self.set_value(id, to);
                if self.value(id) != v { self.emit(Response::Changed(id)); }
            }
            Kind::List(_) => self.list_key(id, k),
            Kind::Tabs(t) => {
                let (n, a, closable) = (t.tabs.len(), t.active, t.closable);
                if n == 0 { return; }
                match nav {
                    Some(Nav::Left) => self.select_tab(id, (a + n - 1) % n),
                    Some(Nav::Right) => self.select_tab(id, (a + 1) % n),
                    Some(Nav::Home) => self.select_tab(id, 0),
                    Some(Nav::End) => self.select_tab(id, n - 1),
                    Some(Nav::Delete) if closable => self.out.push(Response::Tab(id, TabEvent::Closed(a))),
                    _ => {}
                }
            }
            Kind::MenuBar { .. } if matches!(nav, Some(Nav::Space | Nav::Enter | Nav::Down)) => self.open_bar_menu(id, 0),
            _ => {}
        }
    }

    /// Presses a widget: the same as a click, Space, or a screen reader's Click.
    pub(crate) fn activate(&mut self, id: Id) {
        if !self.node(id).enabled { return; }
        match &self.node(id).kind {
            Kind::Button { .. } => self.emit(Response::Clicked(id)),
            Kind::Check { on, .. } => { let v = !*on; self.set_checked(id, v); self.emit(Response::Toggled(id, v)); }
            Kind::Radio { on, .. } => { if !*on { self.set_checked(id, true); self.emit(Response::Toggled(id, true)); } }
            Kind::MenuRow(_) => self.menu_activate_row(id),
            _ => {}
        }
    }

    /// Editing keys for a field; `false` if the key isn't one it uses.
    pub(crate) fn field_key(&mut self, id: Id, k: Key) -> bool {
        let clip = self.clipboard.clone();
        let Some(f) = self.field_mut(id) else { return false };
        let before = f.edit.text.clone();
        let e = &mut f.edit;
        let mut copy = None;
        let mut submit = false;
        if k.control && !k.shift && k.code == letter::A { e.select_all(); }
        else if k.control && k.code == letter::C { if !f.password && e.has_selection() { copy = Some(String::from(e.selected())); } }
        else if k.control && k.code == letter::X {
            if !f.password && e.has_selection() { copy = Some(String::from(e.selected())); e.backspace(false); }
        }
        else if k.control && k.code == letter::V { e.insert(&clip); }
        else if k.control && k.code == letter::Z && !k.shift { e.undo(); }
        else if k.control && (k.code == letter::Y || k.code == letter::Z) { e.redo(); }
        else {
            match k.nav() {
                Some(Nav::Left) => e.left(k.control, k.shift),
                Some(Nav::Right) => e.right(k.control, k.shift),
                Some(Nav::Home) => e.home(k.shift),
                Some(Nav::End) => e.end(k.shift),
                Some(Nav::Backspace) => { e.backspace(k.control); }
                Some(Nav::Delete) => { e.delete(k.control); }
                Some(Nav::Enter) => submit = true,
                Some(Nav::Space) if !k.control => { e.insert(" "); }
                Some(_) => return false,
                None => match k.insert() {
                    Some(c) => { let mut b = [0u8; 4]; e.insert(c.encode_utf8(&mut b)); }
                    None => return false,
                },
            }
        }
        let changed = f.edit.text != before;
        if let Some(c) = copy { self.clipboard = c.clone(); self.out.push(Response::Copy(c)); }
        self.reveal_field_cursor(id);
        self.damage_node(id);
        if changed { self.emit(Response::Changed(id)); }
        if submit { self.emit(Response::Submitted(id)); }
        true
    }

    // ---- lists -------------------------------------------------------------------

    pub(crate) fn list_geom(&self, id: Id) -> ListGeom {
        let r = self.node(id).rect;
        let row_h = self.theme.row_height();
        let Kind::List(l) = &self.node(id).kind else { return ListGeom { header: Rect::ZERO, rows: r, row_h, content: 0.0, bar: None } };
        let hh = if l.columns.is_empty() { 0.0 } else { row_h };
        let header = Rect::new(r.x, r.y, r.w, hh);
        let rows = Rect::new(r.x, r.y + hh, r.w, (r.h - hh).max(0.0));
        let content = l.model.len() as f32 * row_h;
        let bar = (content > rows.h + 0.5).then(|| Rect::new(rows.right() - BAR_W, rows.y, BAR_W, rows.h));
        ListGeom { header, rows, row_h, content, bar }
    }

    fn list_scroll_to(&mut self, id: Id, y: f32) {
        let g = self.list_geom(id);
        let max = (g.content - g.rows.h).max(0.0);
        if let Kind::List(l) = &mut self.node_mut(id).kind {
            let y = y.clamp(0.0, max);
            if l.scroll != y { l.scroll = y; }
        }
        self.damage_node(id);
    }

    /// Scrolls a list so its cursor row shows (now if laid out, else after layout).
    pub(crate) fn reveal_cursor(&mut self, id: Id) {
        if self.node(id).rect.h <= 0.0 { if let Kind::List(l) = &mut self.node_mut(id).kind { l.reveal = true; } return; }
        let g = self.list_geom(id);
        let Kind::List(l) = &mut self.node_mut(id).kind else { return };
        l.reveal = false;
        let top = l.cursor as f32 * g.row_h;
        let mut y = l.scroll;
        if top < y { y = top; } else if top + g.row_h > y + g.rows.h { y = top + g.row_h - g.rows.h; }
        self.list_scroll_to(id, y);
    }

    fn list_row_at(&self, id: Id, p: Point) -> Option<usize> {
        let g = self.list_geom(id);
        let Kind::List(l) = &self.node(id).kind else { return None };
        if !g.rows.contains(p) { return None; }
        let row = ((p.y - g.rows.y + l.scroll) / g.row_h) as usize;
        (row < l.model.len()).then_some(row)
    }

    pub(crate) fn list_key(&mut self, id: Id, k: Key) {
        let g = self.list_geom(id);
        let now = self.now;
        let Kind::List(l) = &mut self.node_mut(id).kind else { return };
        let n = l.model.len();
        if n == 0 { return; }
        let page = ((g.rows.h / g.row_h) as usize).max(1);
        let c = l.cursor.min(n - 1);
        let nav = k.nav();
        let mut moved = None;
        match nav {
            Some(Nav::Up) => moved = Some(c.saturating_sub(1)),
            Some(Nav::Down) => moved = Some((c + 1).min(n - 1)),
            Some(Nav::PageUp) => moved = Some(c.saturating_sub(page)),
            Some(Nav::PageDown) => moved = Some((c + page).min(n - 1)),
            Some(Nav::Home) => moved = Some(0),
            Some(Nav::End) => moved = Some(n - 1),
            Some(Nav::Enter) => { self.emit(Response::Activated(id, c)); return; }
            Some(Nav::Space) => {
                if k.control && l.multi { l.selection.toggle(c); } else { l.selection.set(c); l.anchor = c; }
                self.damage_node(id);
                self.emit(Response::Selected(id));
                return;
            }
            Some(Nav::Left | Nav::Right) => {
                let right = nav == Some(Nav::Right);
                match (l.model.expandable(c), right) {
                    (Some(false), true) => { self.emit(Response::Expand(id, c, true)); return; }
                    (Some(true), false) => { self.emit(Response::Expand(id, c, false)); return; }
                    (Some(true), true) => moved = Some((c + 1).min(n - 1)),
                    (_, false) => {
                        let d = l.model.depth(c);
                        if d > 0 { moved = (0..c).rev().find(|r| l.model.depth(*r) < d); }
                    }
                    _ => {}
                }
            }
            _ => {
                if k.control && k.code == letter::A && l.multi {
                    l.selection.set_range(0, n - 1);
                    self.damage_node(id);
                    self.emit(Response::Selected(id));
                    return;
                }
                // Type to find: rows starting with what was typed lately.
                let Some(ch) = k.insert() else { return };
                if now.saturating_sub(l.find_at) > 1000 { l.find.clear(); }
                l.find_at = now;
                l.find.extend(ch.to_lowercase());
                let from = if l.find.chars().count() == 1 { c + 1 } else { c };
                let mut cell = String::new();
                for i in 0..n {
                    let row = (from + i) % n;
                    cell.clear();
                    l.model.cell(row, 0, &mut cell);
                    if cell.to_lowercase().starts_with(l.find.as_str()) { moved = Some(row); break; }
                }
            }
        }
        let Some(to) = moved else { return };
        let Kind::List(l) = &mut self.node_mut(id).kind else { return };
        l.cursor = to;
        if k.shift && l.multi { l.selection.set_range(l.anchor, to); }
        else if k.control && l.multi { /* moves the cursor only */ }
        else { l.selection.set(to); l.anchor = to; }
        self.reveal_cursor(id);
        self.damage_node(id);
        self.emit(Response::Selected(id));
    }

    fn list_click(&mut self, id: Id, p: Point, clicks: u8) {
        let g = self.list_geom(id);
        if let Some(bar) = g.bar { if bar.contains(p) { self.scrollbar_down(id, p); return; } }
        if g.header.contains(p) {
            // Which column header.
            let Kind::List(l) = &self.node(id).kind else { return };
            let mut x = g.header.x;
            for (i, c) in l.columns.iter().enumerate() {
                let last = i + 1 == l.columns.len();
                if p.x < x + c.width || last { self.emit(Response::Sort(id, i)); return; }
                x += c.width;
            }
            return;
        }
        let Some(row) = self.list_row_at(id, p) else { return };
        let Kind::List(l) = &mut self.node_mut(id).kind else { return };
        // A click on a tree row's arrow opens or closes it.
        if let Some(open) = l.model.expandable(row) {
            let x0 = g.rows.x + 4.0 + l.model.depth(row) as f32 * 16.0;
            if p.x >= x0 && p.x < x0 + 16.0 { self.emit(Response::Expand(id, row, !open)); return; }
        }
        let changed = !(l.selection.count() == 1 && l.selection.contains(row));
        l.cursor = row;
        l.anchor = row;
        l.selection.set(row);
        self.damage_node(id);
        if changed { self.emit(Response::Selected(id)); }
        if clicks >= 2 { self.emit(Response::Activated(id, row)); }
        self.drag = Drag::ListSelect(id);
    }

    // ---- fields --------------------------------------------------------------------

    /// Width in layout units of `text` in the body face.
    pub(crate) fn text_width(&self, face: Face, size: f32, text: &str) -> f32 {
        let s = self.theme.scale;
        self.fonts.measure(face, size * s, text) / s
    }

    /// What a field shows: its text, or dots for a password.
    pub(crate) fn field_shown(f: &FieldState) -> String {
        if f.password { f.edit.text.chars().map(|_| '•').collect() } else { f.edit.text.clone() }
    }

    /// The x offset (from the text start) of byte `at` of a field's text.
    pub(crate) fn field_x(&self, f: &FieldState, at: usize) -> f32 {
        let size = self.theme.font_size;
        if f.password { return self.text_width(Face::Sans, size, "•") * f.edit.text[..at].chars().count() as f32; }
        self.text_width(Face::Sans, size, &f.edit.text[..at])
    }

    /// The byte offset nearest `x` (from the text start).
    fn field_at(&self, f: &FieldState, x: f32) -> usize {
        let mut best = (0usize, f32::MAX);
        let ends = f.edit.text.char_indices().map(|(i, _)| i).chain(core::iter::once(f.edit.text.len()));
        for i in ends {
            let d = (self.field_x(f, i) - x).abs();
            if d < best.1 { best = (i, d); } else { break; }
        }
        best.0
    }

    pub(crate) fn reveal_field_cursor(&mut self, id: Id) {
        let r = self.node(id).rect;
        let pad = self.theme.padding();
        let Kind::Field(f) = &self.node(id).kind else { return };
        let x = self.field_x(f, f.edit.cursor);
        let room = (r.w - pad * 2.0).max(1.0);
        let total = self.field_x(f, f.edit.text.len());
        let mut scroll = f.scroll;
        if x < scroll { scroll = (x - room / 3.0).max(0.0); }
        if x > scroll + room { scroll = x - room + 2.0; }
        scroll = scroll.min((total - room + 2.0).max(0.0)).max(0.0);
        if let Some(f) = self.field_mut(id) { f.scroll = scroll; }
    }

    fn field_pointer(&mut self, id: Id, p: Point, clicks: u8, extend: bool) {
        let r = self.node(id).rect;
        let pad = self.theme.padding();
        let Kind::Field(f) = &self.node(id).kind else { return };
        let at = self.field_at(f, p.x - r.x - pad + f.scroll);
        let f = self.field_mut(id).unwrap();
        match clicks { 2 => f.edit.select_word(at), 3.. => f.edit.select_all(), _ => f.edit.set_cursor(at, extend) }
        self.reveal_field_cursor(id);
        self.damage_node(id);
    }

    // ---- tabs --------------------------------------------------------------------

    /// Lays a strip's tabs out: each tab's span, the New Tab button and the scroll.
    pub(crate) fn tab_geom(&mut self, id: Id) -> (Rect, Option<Rect>) {
        let r = self.node(id).rect;
        let t_ = &self.theme;
        let (ch, pad, body) = (t_.control_height(), t_.padding(), t_.font_size);
        let strip = Rect::new(r.x, r.y, r.w, tab_strip_height(t_));
        let Kind::Tabs(t) = &self.node(id).kind else { return (strip, None) };
        let nb = if t.new_button { ch } else { 0.0 };
        let room = (strip.w - nb - 4.0).max(0.0);
        let mut widths: Vec<f32> = t.tabs.iter().map(|tab| {
            if tab.pinned { return ch + 8.0; }
            let tw = self.text_width(Face::Sans, body, &tab.title);
            (tw + pad * 2.0 + if tab.icon.is_some() { 22.0 } else { 0.0 } + if t.closable { 24.0 } else { 0.0 }).clamp(96.0, 240.0)
        }).collect();
        let pinned: f32 = t.tabs.iter().zip(&widths).filter(|(x, _)| x.pinned).map(|(_, w)| *w).sum();
        let unpinned = t.tabs.iter().filter(|x| !x.pinned).count();
        let mut total: f32 = widths.iter().sum();
        if total > room && unpinned > 0 {
            let each = ((room - pinned) / unpinned as f32).max(72.0);
            for (w, tab) in widths.iter_mut().zip(&t.tabs) { if !tab.pinned { *w = w.min(each); } }
            total = widths.iter().sum();
        }
        let reveal = t.reveal;
        let active = t.active;
        let Kind::Tabs(t) = &mut self.node_mut(id).kind else { unreachable!() };
        t.spans.clear();
        let mut x = 0.0;
        for w in &widths { t.spans.push((x, *w)); x += w; }
        t.total = total;
        t.room = room;
        if reveal {
            t.reveal = false;
            if let Some(&(x, w)) = t.spans.get(active) {
                if x < t.scroll { t.scroll = x; } else if x + w > t.scroll + room { t.scroll = x + w - room; }
            }
        }
        t.scroll = t.scroll.clamp(0.0, (total - room).max(0.0));
        let new = t.new_button.then(|| Rect::new(strip.x + total.min(room) - t.scroll.min(0.0) + 2.0, strip.y + 2.0, ch, ch));
        (strip, new)
    }

    /// The tab under `p` and whether `p` is on its close button.
    fn tab_at(&self, id: Id, p: Point) -> Option<(usize, bool)> {
        let r = self.node(id).rect;
        let h = tab_strip_height(&self.theme);
        let Kind::Tabs(t) = &self.node(id).kind else { return None };
        if p.y < r.y || p.y >= r.y + h || p.x >= r.x + t.room { return None; }
        let x = p.x - r.x + t.scroll;
        let i = t.spans.iter().position(|(sx, w)| x >= *sx && x < sx + w)?;
        let (sx, w) = t.spans[i];
        let close = t.closable && !t.tabs[i].pinned && x >= sx + w - 26.0;
        Some((i, close))
    }

    /// Scrolls a tab strip so tab `i` shows.
    pub(crate) fn reveal_tab(&mut self, id: Id, _i: usize) {
        if let Kind::Tabs(t) = &mut self.node_mut(id).kind { t.reveal = true; }
        if self.node(id).rect.w > 0.0 { self.tab_geom(id); }
    }

    fn tabs_down(&mut self, id: Id, p: Point, button: Button) {
        let (_, new) = self.tab_geom(id);
        if let Some(nb) = new { if nb.contains(p) && button == Button::Left { self.out.push(Response::Tab(id, TabEvent::New)); return; } }
        let Some((i, close)) = self.tab_at(id, p) else { return };
        match button {
            Button::Middle => { if matches!(&self.node(id).kind, Kind::Tabs(t) if t.closable && !t.tabs[i].pinned) { self.out.push(Response::Tab(id, TabEvent::Closed(i))); } }
            Button::Left if close => self.out.push(Response::Tab(id, TabEvent::Closed(i))),
            Button::Left => {
                self.select_tab(id, i);
                let reorder = matches!(&self.node(id).kind, Kind::Tabs(t) if t.reorderable);
                if reorder { self.drag = Drag::Tab { id, from: i, x0: p.x, moved: false }; }
            }
            Button::Right => {}
        }
    }

    fn tab_drag(&mut self, p: Point) {
        let Drag::Tab { id, from, x0, moved } = self.drag else { return };
        let moved = moved || (p.x - x0).abs() > 6.0;
        self.drag = Drag::Tab { id, from, x0, moved };
        if !moved { return; }
        let r = self.node(id).rect;
        let Kind::Tabs(t) = &self.node(id).kind else { return };
        let cur = t.active;
        let pinned = t.tabs[cur].pinned;
        let x = p.x - r.x + t.scroll;
        // Drop slot: the tab whose middle the pointer passed, within the same run.
        let mut to = cur;
        for (i, (sx, w)) in t.spans.iter().enumerate() {
            if t.tabs[i].pinned != pinned { continue; }
            if (i < cur && x < sx + w / 2.0) || (i > cur && x > sx + w / 2.0) { if i < cur && to == cur { to = i; } else if i > cur { to = i; } }
        }
        if to != cur { self.move_tab(id, cur, to); self.tab_geom(id); }
    }

    fn tab_drop(&mut self, p: Point) {
        let Drag::Tab { id, from, moved, .. } = self.drag else { return };
        if !moved || !self.exists(id) { return; }
        let r = self.node(id).rect;
        let strip = tab_strip_height(&self.theme);
        let cur = self.active_tab(id);
        if p.y < r.y - TEAR_OFF || p.y > r.y + strip + TEAR_OFF {
            self.out.push(Response::Tab(id, TabEvent::TornOff(cur, p)));
        } else if cur != from {
            self.out.push(Response::Tab(id, TabEvent::Moved { from, to: cur }));
        }
    }

    fn tab_menu(&mut self, id: Id, i: usize, at: Point) {
        let (pinned, closable) = match &self.node(id).kind { Kind::Tabs(t) => (t.tabs[i].pinned, t.closable), _ => return };
        let pin_label = self.tr(if pinned { "weft-unpin-tab" } else { "weft-pin-tab" });
        if let Some(c) = self.commands.get_mut("weft.tab-pin") { c.label = pin_label; c.enabled = true; }
        let mut m = Menu::new("");
        if closable {
            m = m.command("weft.tab-close").command("weft.tab-close-others").separator();
            for c in ["weft.tab-close", "weft.tab-close-others"] { self.commands.set_enabled(c, true); }
        }
        m = m.command("weft.tab-pin");
        if closable { m = m.separator().command("weft.tab-new"); self.commands.set_enabled("weft.tab-new", true); }
        self.open_menu(m, Rect::new(at.x, at.y, 0.0, 0.0), false, Some(id), None, Some(i));
        self.update_tab_commands();
    }

    // ---- pointer -------------------------------------------------------------------

    fn on_pointer(&mut self, p: Pointer) {
        let s = self.theme.scale;
        let pt = Point::new(p.x / s, p.y / s);
        self.pointer = pt;
        match p.kind {
            PointerKind::Move => self.on_move(pt),
            PointerKind::Down(b) => self.on_down(pt, b),
            PointerKind::Up(b) => self.on_up(pt, b),
            PointerKind::Wheel(n) => self.on_wheel(pt, n),
            PointerKind::Leave => { if self.drag == Drag::None { self.set_hover(None); } self.hide_tooltip(); }
        }
    }

    /// The widget a pointer event at `hit` is for: the nearest one that does something.
    pub(crate) fn interactive(&self, hit: Option<Id>) -> Option<Id> {
        let mut cur = hit;
        while let Some(id) = cur {
            let n = self.node(id);
            if matches!(n.kind, Kind::Button { .. } | Kind::Field(_) | Kind::Check { .. } | Kind::Radio { .. } | Kind::Slider { .. } | Kind::List(_) | Kind::MenuBar { .. } | Kind::MenuRow(_)) { return Some(id); }
            if let Kind::Tabs(_) = n.kind { if self.pointer.y < n.rect.y + tab_strip_height(&self.theme) { return Some(id); } }
            if let Kind::Split { .. } = n.kind { if self.split_divider(id).contains(self.pointer) { return Some(id); } }
            if let Kind::Scroll { content, .. } = n.kind { if content > n.rect.h && self.pointer.x >= n.rect.right() - BAR_W { return Some(id); } }
            cur = n.parent;
        }
        None
    }

    pub(crate) fn split_divider(&self, id: Id) -> Rect {
        let n = self.node(id);
        let (Kind::Split { vertical, .. }, Some(first)) = (&n.kind, n.children.first()) else { return Rect::ZERO };
        let a = self.node(*first).rect;
        if *vertical { Rect::new(n.rect.x, a.bottom(), n.rect.w, SPLIT_GAP) } else { Rect::new(a.right(), n.rect.y, SPLIT_GAP, n.rect.h) }
    }

    pub(crate) fn set_hover(&mut self, id: Option<Id>) {
        if self.hover == id { return; }
        if let Some(old) = self.hover { if self.exists(old) { self.damage_node(old); } }
        self.hover = id;
        if let Some(new) = id { self.damage_node(new); }
    }

    fn on_move(&mut self, p: Point) {
        match self.drag {
            Drag::None => {}
            Drag::Slider(id) => { self.slider_to(id, p); return; }
            Drag::Split(id) => {
                let n = self.node(id);
                let Kind::Split { vertical, .. } = n.kind else { return };
                let r = n.rect;
                let ratio = if vertical { (p.y - r.y) / r.h.max(1.0) } else { (p.x - r.x) / r.w.max(1.0) };
                let ratio = ratio.clamp(0.05, 0.95);
                if let Kind::Split { ratio: old, .. } = &mut self.node_mut(id).kind { *old = ratio; }
                self.split_styles(id, ratio);
                return;
            }
            Drag::ScrollBar { id, grab } => { self.scrollbar_to(id, p.y - grab); return; }
            Drag::ListSelect(id) => {
                let r = self.list_geom(id).rows;
                let y = p.y.clamp(r.y, r.bottom() - 1.0);
                if let Some(row) = self.list_row_at(id, Point::new(r.x + 1.0, y)) {
                    if let Kind::List(l) = &mut self.node_mut(id).kind {
                        if l.cursor != row {
                            l.cursor = row;
                            if l.multi { l.selection.set_range(l.anchor, row); } else { l.selection.set(row); l.anchor = row; }
                            self.reveal_cursor(id);
                            self.emit(Response::Selected(id));
                        }
                    }
                }
                return;
            }
            Drag::FieldSelect(id) => { self.field_pointer(id, p, 1, true); return; }
            Drag::Tab { .. } => { self.tab_drag(p); return; }
        }
        let (_, hit) = self.hit(p);
        let target = self.interactive(hit);
        self.set_hover(target);
        // Menus follow the pointer: rows light up, submenus open, and the bar
        // switches menus while one is open.
        if let Some(t) = target {
            if let Kind::MenuRow(_) = self.node(t).kind { self.menu_hover(t); }
            if let Kind::MenuBar { .. } = self.node(t).kind { if !self.menus.is_empty() { if let Some(i) = self.bar_index_at(t, p) { self.open_bar_menu(t, i); } } }
            if let Kind::Tabs(_) = self.node(t).kind {
                let at = self.tab_at(t, p);
                if let Kind::Tabs(st) = &mut self.node_mut(t).kind {
                    let (h, c) = (at.map(|a| a.0), at.and_then(|a| a.1.then_some(a.0)));
                    if st.hover != h || st.hover_close != c { st.hover = h; st.hover_close = c; self.damage_node(t); }
                }
            }
        }
        if let Some(old) = self.tooltip.map(|t| t.0) {
            if Some(old) != target { self.hide_tooltip(); }
        }
        if let Some(t) = target.or(hit) {
            let tip = self.tooltip_owner(t);
            if let Some(owner) = tip { if self.tooltip.map(|x| x.0) != Some(owner) { self.tooltip = Some((owner, self.now + 600, None)); } }
        }
    }

    fn on_down(&mut self, p: Point, b: Button) {
        self.hide_tooltip();
        let (layer, hit) = self.hit(p);
        if !self.menus.is_empty() {
            let in_menu = layer.map_or(false, |li| self.layers[li].kind == LayerKind::Menu);
            if in_menu {
                if let Some(t) = self.interactive(hit) { self.pressed = Some(t); }
                return;
            }
            // Outside: close menus; a press on the bar's open title just closes it.
            let target = self.interactive(hit);
            let bar_title = target.and_then(|t| matches!(self.node(t).kind, Kind::MenuBar { .. }).then_some(t));
            let open_bar = self.menus.first().and_then(|m| m.bar);
            self.close_menus(0);
            if let (Some(bar), Some((ob, oi))) = (bar_title, open_bar) {
                if bar == ob && self.bar_index_at(bar, p) == Some(oi) { return; }
            } else { return; }
        }
        let Some(li) = layer else { return };
        if self.palette.is_some() && self.layers[li].kind != LayerKind::Palette { self.close_palette(); return; }
        let Some(target) = self.interactive(hit) else {
            if b == Button::Right { self.context_menu_at(hit, p); }
            return;
        };
        if !self.node(target).enabled { return; }
        let clicks = if self.now.saturating_sub(self.last_click.0) < DOUBLE_MS && (self.last_click.1.x - p.x).abs() < 5.0 && (self.last_click.1.y - p.y).abs() < 5.0 && b == Button::Left {
            self.last_click.2.saturating_add(1)
        } else { 1 };
        if b == Button::Left { self.last_click = (self.now, p, clicks); }
        if self.node(target).kind.focusable() && !matches!(self.node(target).kind, Kind::MenuBar { .. }) {
            self.focus_visible = false;
            self.focus(Some(target));
        }
        match b {
            Button::Left => {
                self.pressed = Some(target);
                self.damage_node(target);
                match &self.node(target).kind {
                    Kind::Field(_) => { self.field_pointer(target, p, clicks, false); self.drag = Drag::FieldSelect(target); }
                    Kind::Slider { .. } => { self.slider_to(target, p); self.drag = Drag::Slider(target); }
                    Kind::List(_) => self.list_click(target, p, clicks),
                    Kind::Tabs(_) => self.tabs_down(target, p, b),
                    Kind::Split { .. } => self.drag = Drag::Split(target),
                    Kind::Scroll { .. } => self.scrollbar_down(target, p),
                    Kind::MenuBar { .. } => if let Some(i) = self.bar_index_at(target, p) { self.open_bar_menu(target, i); },
                    _ => {}
                }
            }
            Button::Middle => if let Kind::Tabs(_) = self.node(target).kind { self.tabs_down(target, p, b); },
            Button::Right => {
                match &self.node(target).kind {
                    Kind::Tabs(_) => { if let Some((i, _)) = self.tab_at(target, p) { self.tab_menu(target, i, p); return; } }
                    Kind::List(_) => {
                        if let Some(row) = self.list_row_at(target, p) {
                            if let Kind::List(l) = &mut self.node_mut(target).kind {
                                if !l.selection.contains(row) { l.selection.set(row); l.cursor = row; l.anchor = row; self.emit(Response::Selected(target)); }
                            }
                            self.damage_node(target);
                        }
                    }
                    _ => {}
                }
                self.context_menu_at(Some(target), p);
            }
        }
    }

    fn on_up(&mut self, p: Point, b: Button) {
        if b != Button::Left { return; }
        let drag = core::mem::replace(&mut self.drag, Drag::None);
        if let Drag::Tab { .. } = drag { self.drag = drag; self.tab_drop(p); self.drag = Drag::None; }
        let pressed = self.pressed.take();
        if let Some(pr) = pressed { if self.exists(pr) { self.damage_node(pr); } }
        let (_, hit) = self.hit(p);
        let target = self.interactive(hit);
        let Some(t) = target else { return };
        // Menu rows take a release even after a press elsewhere (press on the bar,
        // release on an item); other widgets only when pressed and released on.
        let row = matches!(self.node(t).kind, Kind::MenuRow(_));
        if row && !self.menus.is_empty() { self.activate(t); return; }
        if pressed == Some(t) && matches!(self.node(t).kind, Kind::Button { .. } | Kind::Check { .. } | Kind::Radio { .. }) { self.activate(t); }
    }

    fn on_wheel(&mut self, p: Point, steps: i8) {
        let (_, hit) = self.hit(p);
        let delta = -(steps as f32) * self.theme.row_height() * 3.0;
        let mut cur = hit;
        while let Some(id) = cur {
            match &self.node(id).kind {
                Kind::List(l) => { let y = l.scroll + delta; let before = l.scroll; self.list_scroll_to(id, y); if self.list_scroll(id) != before { return; } }
                Kind::Scroll { y, content } => {
                    let (y0, max) = (*y, (*content - self.node(id).rect.h).max(0.0));
                    let ny = (y0 + delta).clamp(0.0, max);
                    if ny != y0 { if let Kind::Scroll { y, .. } = &mut self.node_mut(id).kind { *y = ny; } self.relayout(); return; }
                }
                Kind::Tabs(t) if p.y < self.node(id).rect.y + tab_strip_height(&self.theme) => {
                    let ns = (t.scroll + delta).clamp(0.0, (t.total - t.room).max(0.0));
                    if let Kind::Tabs(t) = &mut self.node_mut(id).kind { t.scroll = ns; }
                    self.damage_node(id);
                    return;
                }
                _ => {}
            }
            cur = self.node(id).parent;
        }
    }

    fn list_scroll(&self, id: Id) -> f32 { match &self.node(id).kind { Kind::List(l) => l.scroll, _ => 0.0 } }

    fn slider_to(&mut self, id: Id, p: Point) {
        let r = self.node(id).rect;
        let Kind::Slider { min, max, value, .. } = self.node(id).kind else { return };
        let f = ((p.x - r.x - 8.0) / (r.w - 16.0).max(1.0)).clamp(0.0, 1.0);
        self.set_value(id, min + f * (max - min));
        if self.value(id) != value { self.emit(Response::Changed(id)); }
    }

    /// The scroll bar of a list or scroll view: thumb rectangle and the range it moves in.
    pub(crate) fn scrollbar(&self, id: Id) -> Option<(Rect, Rect, f32, f32)> {
        let n = self.node(id);
        let (track, view, content, y) = match &n.kind {
            Kind::List(l) => { let g = self.list_geom(id); (g.bar?, g.rows.h, g.content, l.scroll) }
            Kind::Scroll { y, content } if *content > n.rect.h + 0.5 => (Rect::new(n.rect.right() - BAR_W, n.rect.y, BAR_W, n.rect.h), n.rect.h, *content, *y),
            _ => return None,
        };
        let th = (track.h * view / content).max(24.0).min(track.h);
        let ty = track.y + (track.h - th) * (y / (content - view).max(1.0));
        Some((Rect::new(track.x + 2.0, ty, track.w - 4.0, th), track, content - view, th))
    }

    fn scrollbar_down(&mut self, id: Id, p: Point) {
        let Some((thumb, track, _, th)) = self.scrollbar(id) else { return };
        if !track.contains(p) { return; }
        let grab = if thumb.contains(p) { p.y - thumb.y } else { th / 2.0 };
        self.drag = Drag::ScrollBar { id, grab };
        self.scrollbar_to(id, p.y - grab);
    }

    fn scrollbar_to(&mut self, id: Id, top: f32) {
        let Some((_, track, range, th)) = self.scrollbar(id) else { return };
        let f = ((top - track.y) / (track.h - th).max(1.0)).clamp(0.0, 1.0);
        let y = f * range;
        match &mut self.node_mut(id).kind {
            Kind::List(_) => self.list_scroll_to(id, y),
            Kind::Scroll { y: sy, .. } => { *sy = y; self.relayout(); }
            _ => {}
        }
    }

    /// Opens the context menu of what was right-clicked (or its nearest ancestor with one).
    fn context_menu_at(&mut self, hit: Option<Id>, p: Point) {
        let mut cur = hit;
        while let Some(id) = cur {
            if let Some(m) = self.node(id).context.clone() {
                self.open_menu(m, Rect::new(p.x, p.y, 0.0, 0.0), false, Some(id), None, None);
                return;
            }
            if let Kind::Field(f) = &self.node(id).kind {
                let m = if f.password { Menu::new("").command("weft.paste").command("weft.select-all") } else {
                    Menu::new("").command("weft.undo").command("weft.redo").separator().command("weft.cut").command("weft.copy").command("weft.paste").separator().command("weft.select-all")
                };
                self.focus(Some(id));
                self.open_menu(m, Rect::new(p.x, p.y, 0.0, 0.0), false, Some(id), None, None);
                return;
            }
            cur = self.node(id).parent;
        }
    }

    /// Shift+F10 or the Menu key: the focused widget's context menu, at the widget.
    fn context_menu_for_focus(&mut self) {
        let Some(f) = self.focus else { return };
        let r = self.node(f).rect;
        if let Kind::Tabs(t) = &self.node(f).kind {
            let i = t.active;
            let (x, w) = t.spans.get(i).copied().unwrap_or((0.0, 0.0));
            let at = Point::new(r.x + x - t.scroll + w / 2.0, r.y + tab_strip_height(&self.theme));
            self.tab_menu(f, i, at);
            return;
        }
        let at = match &self.node(f).kind {
            Kind::List(l) => { let g = self.list_geom(f); Point::new(r.x + 24.0, g.rows.y + (l.cursor as f32 + 1.0) * g.row_h - l.scroll) }
            _ => Point::new(r.x + 8.0, r.bottom()),
        };
        self.context_menu_at(Some(f), at);
    }

    /// Lays out pending reveals after layout (rows and tabs asked for before layout).
    pub(crate) fn after_layout(&mut self) {
        let pending: Vec<Id> = self.nodes.iter().enumerate().filter_map(|(i, n)| match n.as_ref().map(|n| &n.kind) {
            Some(Kind::List(l)) if l.reveal => Some(Id(i as u32)),
            Some(Kind::Tabs(_)) => Some(Id(i as u32)),
            _ => None,
        }).collect();
        for id in pending {
            match self.node(id).kind { Kind::List(_) => self.reveal_cursor(id), _ => { self.tab_geom(id); } }
        }
    }
}
