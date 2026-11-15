// SPDX-License-Identifier: GPL-3.0-only
//! What opens over the window: menus (bar, context, submenus), the command palette,
//! dialogs (message, confirm, save changes, prompt, colour, font), toasts and tooltips.
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::geom::{Point, Rect};
use crate::input::{Key, Nav};
use crate::menu::{Menu, MenuItem};
use crate::text::Face;
use crate::theme::Color;
use crate::ui::{DialogId, DialogKind, DialogState, LayerKind, OpenMenu, PaletteState, Toast, Ui};
use crate::widget::*;
use crate::{DialogValue, Response};

impl Ui {
    // ---- menus -------------------------------------------------------------------

    /// Opens `menu` as a context menu at window point `at` (layout units).
    pub fn popup_menu(&mut self, menu: Menu, at: Point) {
        self.close_menus(0);
        self.open_menu(menu, Rect::new(at.x, at.y, 0.0, 0.0), false, None, None, None);
    }

    /// The widget the last context menu was opened on, so a command knows what it is for.
    pub fn context_target(&self) -> Option<Id> { self.context_target }

    pub(crate) fn open_menu(&mut self, menu: Menu, anchor: Rect, below: bool, target: Option<Id>, bar: Option<(Id, usize)>, tab: Option<usize>) {
        if menu.items.is_empty() { return; }
        if self.menus.is_empty() { self.context_target = target; }
        let layer = self.open_layer(LayerKind::Menu, anchor, below, Layout::column().padding(4.0).min_width(180.0), Fill::Surface);
        let index = self.menus.len();
        let mut rows = Vec::new();
        for (i, item) in menu.items.iter().enumerate() {
            let mut st = MenuRowState { menu: index, index: i, label: String::new(), shortcut: String::new(), icon: None, check: None, submenu: false, separator: false, enabled: true };
            match item {
                MenuItem::Separator => { st.separator = true; st.enabled = false; }
                MenuItem::Submenu(m) => { st.label = m.title.clone(); st.submenu = true; }
                MenuItem::Command(id) => match self.commands.get(id) {
                    Some(c) => {
                        st.label = c.label.clone();
                        st.shortcut = c.shortcut.map(|s| s.label()).unwrap_or_default();
                        st.icon = c.icon.clone();
                        st.check = c.checked;
                        st.enabled = c.enabled;
                    }
                    None => { st.label = id.clone(); st.enabled = false; }
                },
            }
            rows.push(self.add(layer, Kind::MenuRow(Box::new(st))));
        }
        self.menus.push(OpenMenu { layer, menu, hot: None, rows, target, bar, tab });
        self.hide_tooltip();
    }

    /// Closes open menus from `from` up.
    pub(crate) fn close_menus(&mut self, from: usize) {
        while self.menus.len() > from {
            let m = self.menus.pop().unwrap();
            if let Some((bar, _)) = m.bar { if self.exists(bar) { if let Kind::MenuBar { open, .. } = &mut self.node_mut(bar).kind { *open = None; } self.damage_node(bar); } }
            self.close_layer(m.layer);
        }
        self.full_damage = true;
    }

    fn row_menu(&self, row: Id) -> Option<(usize, usize)> {
        match &self.get(row)?.kind { Kind::MenuRow(m) => Some((m.menu, m.index)), _ => None }
    }

    fn set_hot(&mut self, mi: usize, hot: Option<usize>) {
        let Some(m) = self.menus.get(mi) else { return };
        if m.hot == hot { return; }
        let (old, rows) = (m.hot, m.rows.clone());
        self.menus[mi].hot = hot;
        for i in [old, hot].into_iter().flatten() { if let Some(r) = rows.get(i) { self.damage_node(*r); } }
        self.a11y_dirty = true;
    }

    pub(crate) fn menu_hover(&mut self, row: Id) {
        let Some((mi, i)) = self.row_menu(row) else { return };
        let Kind::MenuRow(st) = &self.node(row).kind else { return };
        if st.separator { return; }
        let sub = st.submenu && st.enabled;
        let already = self.menus.get(mi).map_or(false, |m| m.hot == Some(i)) && self.menus.len() > mi + 1;
        self.set_hot(mi, Some(i));
        if already { return; }
        self.close_menus(mi + 1);
        if sub { self.open_submenu(mi, i, false); }
    }

    fn open_submenu(&mut self, mi: usize, i: usize, hot_first: bool) {
        let Some(MenuItem::Submenu(m)) = self.menus[mi].menu.items.get(i).cloned() else { return };
        let row = self.menus[mi].rows[i];
        let anchor = self.node(row).rect;
        let (target, tab) = (self.menus[mi].target, self.menus[mi].tab);
        self.close_menus(mi + 1);
        self.open_menu(m, anchor, false, target, None, tab);
        if hot_first { let n = self.menus.len() - 1; let f = self.next_hot(n, None, true); self.set_hot(n, f); }
    }

    /// The next row that can be chosen, going down (or up), wrapping.
    fn next_hot(&self, mi: usize, from: Option<usize>, down: bool) -> Option<usize> {
        let m = &self.menus[mi];
        let n = m.rows.len();
        let ok = |i: usize| matches!(&self.node(m.rows[i]).kind, Kind::MenuRow(r) if !r.separator && r.enabled);
        let start = match from { Some(f) => f, None => if down { n - 1 } else { 0 } };
        (1..=n).map(|k| if down { (start + k) % n } else { (start + n * 2 - k) % n }).find(|i| ok(*i))
    }

    pub(crate) fn menu_key(&mut self, k: Key) {
        let top = self.menus.len() - 1;
        let hot = self.menus[top].hot;
        match k.nav() {
            Some(Nav::Up) => { let h = self.next_hot(top, hot, false); self.set_hot(top, h); }
            Some(Nav::Down) => { let h = self.next_hot(top, hot, true); self.set_hot(top, h); }
            Some(Nav::Home) => { let h = self.next_hot(top, None, true); self.set_hot(top, h); }
            Some(Nav::End) => { let h = self.next_hot(top, None, false); self.set_hot(top, h); }
            Some(Nav::Right) => {
                let sub = hot.map_or(false, |h| matches!(&self.node(self.menus[top].rows[h]).kind, Kind::MenuRow(r) if r.submenu && r.enabled));
                if sub { self.open_submenu(top, hot.unwrap(), true); }
                else if let Some((bar, i)) = self.menus[0].bar { self.open_bar_menu(bar, i + 1); }
            }
            Some(Nav::Left) => {
                if top > 0 { self.close_menus(top); }
                else if let Some((bar, i)) = self.menus[0].bar {
                    let n = match &self.node(bar).kind { Kind::MenuBar { menus, .. } => menus.len(), _ => 1 };
                    self.open_bar_menu(bar, (i + n - 1) % n);
                }
            }
            Some(Nav::Enter | Nav::Space) => { if let Some(h) = hot { let row = self.menus[top].rows[h]; self.menu_activate_row(row); } }
            Some(Nav::Escape) => self.close_menus(top),
            Some(Nav::Tab) => self.close_menus(0),
            _ => {
                // A letter picks the first item it starts.
                let Some(c) = k.insert() else { return };
                let c = c.to_lowercase().next().unwrap_or(c);
                let rows = self.menus[top].rows.clone();
                let found = rows.iter().position(|r| matches!(&self.node(*r).kind, Kind::MenuRow(m) if m.enabled && !m.separator && m.label.chars().next().and_then(|x| x.to_lowercase().next()) == Some(c)));
                if let Some(i) = found { self.set_hot(top, Some(i)); self.menu_activate_row(rows[i]); }
            }
        }
    }

    pub(crate) fn menu_activate_row(&mut self, row: Id) {
        let Some((mi, i)) = self.row_menu(row) else { return };
        let Kind::MenuRow(st) = &self.node(row).kind else { return };
        if st.separator || !st.enabled || mi >= self.menus.len() { return; }
        match self.menus[mi].menu.items.get(i).cloned() {
            Some(MenuItem::Submenu(_)) => { self.set_hot(mi, Some(i)); self.open_submenu(mi, i, true); }
            Some(MenuItem::Command(id)) => {
                let tab = match (self.menus[0].target, self.menus[0].tab) { (Some(t), Some(i)) => Some((t, i)), _ => None };
                self.close_menus(0);
                self.run_command(&id, tab);
            }
            _ => {}
        }
    }

    /// Where a menu bar's titles are.
    pub(crate) fn bar_titles(&self, bar: Id) -> Vec<Rect> {
        let r = self.node(bar).rect;
        let pad = self.theme.padding();
        let Kind::MenuBar { menus, .. } = &self.node(bar).kind else { return Vec::new() };
        let mut x = r.x;
        menus.iter().map(|m| {
            let w = self.text_width(Face::Sans, self.theme.font_size, &m.title) + pad * 2.0;
            let rect = Rect::new(x, r.y, w, r.h);
            x += w;
            rect
        }).collect()
    }

    pub(crate) fn bar_index_at(&self, bar: Id, p: Point) -> Option<usize> { self.bar_titles(bar).iter().position(|r| r.contains(p)) }

    pub(crate) fn open_bar_menu(&mut self, bar: Id, i: usize) {
        let Kind::MenuBar { menus, open } = &self.node(bar).kind else { return };
        if menus.is_empty() { return; }
        let i = i % menus.len();
        if *open == Some(i) && !self.menus.is_empty() { return; }
        let m = menus[i].clone();
        self.close_menus(0);
        let title = self.bar_titles(bar)[i];
        if let Kind::MenuBar { open, .. } = &mut self.node_mut(bar).kind { *open = Some(i); }
        self.damage_node(bar);
        self.open_menu(m, title, true, Some(bar), Some((bar, i)), None);
        if !self.menus.is_empty() { let f = self.next_hot(0, None, true); self.set_hot(0, f); }
    }

    // ---- command palette -------------------------------------------------------------

    pub(crate) fn open_palette(&mut self) {
        if self.palette.is_some() { return; }
        self.close_menus(0);
        let return_focus = self.focus;
        let layer = self.open_layer(LayerKind::Palette, Rect::ZERO, false, Layout::column().width(560.0).padding(8.0).gap(6.0), Fill::Surface);
        let ph = self.tr("weft-palette-placeholder");
        let field = self.field(layer, "", &ph);
        let fl = Layout::default().fill_width();
        self.set_layout(field, fl);
        let list = self.list_with(layer, Box::new(Rows::default()), vec![Column::new("", 380.0), Column::new("", 140.0).right()]);
        self.set_layout(list, Layout::default().height(320.0).fill_width());
        if let Kind::List(l) = &mut self.node_mut(list).kind { l.columns.iter_mut().for_each(|c| c.title.clear()); }
        self.palette = Some(PaletteState { layer, field, list, results: Vec::new(), return_focus });
        self.refresh_palette();
        self.focus_visible = true;
        self.focus(Some(field));
    }

    fn refresh_palette(&mut self) {
        let Some(p) = &self.palette else { return };
        let (field, list) = (p.field, p.list);
        let q = String::from(self.text_of(field));
        let results = self.commands.search(&q);
        let rows = Rows(results.iter().map(|i| { let c = &self.commands.all()[*i]; vec![c.label.clone(), c.shortcut.map(|s| s.label()).unwrap_or_default()] }).collect());
        let empty = results.is_empty();
        self.palette.as_mut().unwrap().results = results;
        self.set_rows(list, rows);
        self.select_row(list, if empty { None } else { Some(0) });
    }

    pub(crate) fn close_palette(&mut self) {
        let Some(p) = self.palette.take() else { return };
        self.close_layer(p.layer);
        if let Some(f) = p.return_focus.filter(|f| self.exists(*f)) { self.focus(Some(f)); }
        self.full_damage = true;
    }

    fn palette_run(&mut self, row: Option<usize>) {
        let Some(p) = &self.palette else { return };
        let Some(i) = row.or_else(|| self.selected_row(p.list)).and_then(|r| p.results.get(r).copied()) else { return };
        let id = self.commands.all()[i].id.clone();
        if !self.commands.all()[i].enabled { return; }
        self.close_palette();
        self.run_command(&id, None);
    }

    pub(crate) fn palette_key(&mut self, k: Key) {
        let Some(p) = &self.palette else { return };
        let (field, list) = (p.field, p.list);
        match k.nav() {
            Some(Nav::Escape) => self.close_palette(),
            Some(Nav::Enter) => self.palette_run(None),
            Some(Nav::Up | Nav::Down | Nav::PageUp | Nav::PageDown) => self.list_key(list, k),
            Some(Nav::Tab) => {}
            _ => { self.field_key(field, k); }
        }
    }

    // ---- overlay responses -----------------------------------------------------------

    /// Handles a response from a widget in an overlay; `false` passes it to the program.
    pub(crate) fn overlay_response(&mut self, r: &Response) -> bool {
        if let Some(p) = &self.palette {
            let (field, list) = (p.field, p.list);
            match r {
                Response::Changed(i) if *i == field => { self.refresh_palette(); return true; }
                Response::Submitted(i) if *i == field => { self.palette_run(None); return true; }
                Response::Activated(i, row) if *i == list => { self.palette_run(Some(*row)); return true; }
                Response::Selected(i) if *i == list => return true,
                _ => {}
            }
        }
        let id = match r { Response::Clicked(i) | Response::Changed(i) | Response::Submitted(i) | Response::Selected(i) | Response::Activated(i, _) | Response::Toggled(i, _) => *i, _ => return false };
        let root = self.layer_root(id);
        if let Some(di) = self.dialogs.iter().position(|d| d.layer == root) {
            let d = &self.dialogs[di];
            match r {
                Response::Clicked(b) => {
                    let Some(bi) = d.buttons.iter().position(|x| x == b) else { return true };
                    let value = if bi == d.cancel { DialogValue::Cancelled } else {
                        match &d.kind {
                            DialogKind::Message => DialogValue::Button(bi),
                            DialogKind::Prompt { field } => DialogValue::Text(String::from(self.text_of(*field))),
                            DialogKind::Color { hex, .. } => DialogValue::Color(Color::parse(self.text_of(*hex)).unwrap_or(Color::TRANSPARENT)),
                            DialogKind::Font { list, size, faces, .. } => DialogValue::Font { family: self.selected_row(*list).and_then(|i| faces.get(i)).cloned().unwrap_or_default(), size: self.value(*size) },
                        }
                    };
                    self.answer_dialog_at(di, value);
                }
                Response::Submitted(_) => { let b = d.buttons[d.default]; self.emit(Response::Clicked(b)); }
                Response::Changed(_) | Response::Selected(_) => self.dialog_changed(di, id),
                _ => {}
            }
            return true;
        }
        if let Some(ti) = self.toasts.iter().position(|t| t.layer == root) {
            if let (Response::Clicked(b), Some((ab, cmd))) = (r, self.toasts[ti].action.clone()) {
                if *b == ab { let t = self.toasts.remove(ti); self.close_layer(t.layer); self.run_command(&cmd, None); }
            }
            return true;
        }
        false
    }

    // ---- dialogs -----------------------------------------------------------------

    fn dialog_frame(&mut self, title: &str, text: &str) -> (Id, Id) {
        let layer = self.open_layer(LayerKind::Dialog, Rect::ZERO, false, Layout::column().width(440.0).padding(20.0).gap(12.0), Fill::Surface);
        if !title.is_empty() { self.text(layer, title, TextStyle::Heading); }
        if !text.is_empty() { let p = self.paragraph(layer, text); self.set_layout(p, Layout::default().fill_width()); }
        (layer, layer)
    }

    fn dialog_buttons(&mut self, layer: Id, labels: &[&str], default: usize) -> Vec<Id> {
        let row = self.group(layer, Layout::row().gap(8.0).justify(taffy::JustifyContent::FLEX_END).fill_width());
        let mut out = Vec::new();
        for (i, l) in labels.iter().enumerate() {
            let b = self.button(row, l);
            if i == default { self.set_button_style(b, ButtonStyle::Primary); }
            out.push(b);
        }
        out
    }

    fn push_dialog(&mut self, layer: Id, buttons: Vec<Id>, default: usize, cancel: usize, kind: DialogKind, focus: Id) -> DialogId {
        self.close_menus(0);
        self.close_palette();
        let id = DialogId(self.next_dialog);
        self.next_dialog += 1;
        let return_focus = self.focus;
        self.dialogs.push(DialogState { id, layer, buttons, cancel, default, kind, return_focus });
        self.focus_visible = true;
        self.focus(Some(focus));
        if let Some(f) = self.field_mut(focus) { f.edit.select_all(); }
        id
    }

    /// A message with buttons; the answer is [`DialogValue::Button`] with the index
    /// pressed, or [`DialogValue::Cancelled`] for Escape or the `cancel` button.
    pub fn message(&mut self, title: &str, text: &str, buttons: &[&str], default: usize, cancel: usize) -> DialogId {
        let (layer, _) = self.dialog_frame(title, text);
        let b = self.dialog_buttons(layer, buttons, default);
        let f = b[default.min(b.len() - 1)];
        self.push_dialog(layer, b, default.min(buttons.len() - 1), cancel, DialogKind::Message, f)
    }
    /// OK and Cancel.
    pub fn confirm(&mut self, title: &str, text: &str) -> DialogId {
        let (ok, cancel) = (self.tr("weft-ok"), self.tr("weft-cancel"));
        self.message(title, text, &[&cancel, &ok], 1, 0)
    }
    /// Save, Don't Save and Cancel: answers `Button(0)` for Save, `Button(1)` for
    /// Don't Save, or `Cancelled`.
    pub fn ask_save(&mut self, title: &str, text: &str) -> DialogId {
        let (save, dont, cancel) = (self.tr("weft-save"), self.tr("weft-dont-save"), self.tr("weft-cancel"));
        self.message(title, text, &[&save, &dont, &cancel], 0, 2)
    }
    /// Asks for a line of text; answers [`DialogValue::Text`].
    pub fn prompt(&mut self, title: &str, text: &str, initial: &str) -> DialogId {
        let (layer, _) = self.dialog_frame(title, text);
        let field = self.field(layer, initial, "");
        self.set_layout(field, Layout::default().fill_width());
        let (ok, cancel) = (self.tr("weft-ok"), self.tr("weft-cancel"));
        let b = self.dialog_buttons(layer, &[&cancel, &ok], 1);
        self.push_dialog(layer, b, 1, 0, DialogKind::Prompt { field }, field)
    }
    /// A colour picker (red, green, blue and hex); answers [`DialogValue::Color`].
    pub fn pick_color(&mut self, title: &str, initial: Color) -> DialogId {
        let (layer, _) = self.dialog_frame(title, "");
        let grid = self.group(layer, Layout::grid(2).gap(8.0).fill_width());
        if let Some(st) = self.tree.style(self.node(grid).taffy).ok().cloned() {
            let mut st = st;
            st.grid_template_columns = vec![taffy::prelude::length(90.0), taffy::prelude::fr(1.0)];
            let t = self.node(grid).taffy;
            self.tree.set_style(t, st).unwrap();
        }
        let names = [self.tr("weft-color-red"), self.tr("weft-color-green"), self.tr("weft-color-blue")];
        let vals = [initial.r(), initial.g(), initial.b()];
        let mut sliders = [Id(0); 3];
        for i in 0..3 {
            self.label(grid, &names[i]);
            let s = self.slider(grid, 0.0, 255.0, vals[i] as f32);
            self.set_step(s, 1.0);
            self.set_layout(s, Layout::default().fill_width());
            self.set_label(s, &names[i]);
            sliders[i] = s;
        }
        let hex_label = self.tr("weft-color-hex");
        self.label(grid, &hex_label);
        let hex = self.field(grid, &format!("#{:06X}", initial.0 & 0xFF_FFFF), "#RRGGBB");
        self.set_label(hex, &hex_label);
        let preview = self.image(layer, 96, 32, vec![initial.0 & 0xFF_FFFF; 96 * 32]);
        let (ok, cancel) = (self.tr("weft-ok"), self.tr("weft-cancel"));
        let b = self.dialog_buttons(layer, &[&cancel, &ok], 1);
        self.push_dialog(layer, b, 1, 0, DialogKind::Color { sliders, hex, preview }, sliders[0])
    }
    /// A font picker over `faces`; answers [`DialogValue::Font`].
    pub fn pick_font(&mut self, title: &str, faces: &[&str], current: &str, size: f32) -> DialogId {
        let (layer, _) = self.dialog_frame(title, "");
        let list = self.list(layer, faces);
        self.set_layout(list, Layout::default().height(200.0).fill_width());
        if let Some(i) = faces.iter().position(|f| *f == current) { self.select_row(list, Some(i)); }
        let row = self.row(layer);
        let size_label = self.tr("weft-font-size");
        self.label(row, &size_label);
        let s = self.slider(row, 6.0, 96.0, size);
        self.set_step(s, 1.0);
        self.set_label(s, &size_label);
        let preview = self.label(layer, "");
        let (ok, cancel) = (self.tr("weft-ok"), self.tr("weft-cancel"));
        let b = self.dialog_buttons(layer, &[&cancel, &ok], 1);
        let faces: Vec<String> = faces.iter().map(|f| String::from(*f)).collect();
        let id = self.push_dialog(layer, b, 1, 0, DialogKind::Font { list, size: s, preview, faces }, list);
        let di = self.dialogs.len() - 1;
        self.dialog_changed(di, s);
        id
    }

    /// Closes a dialog without an answer (the program changed its mind).
    pub fn close_dialog(&mut self, id: DialogId) {
        if let Some(di) = self.dialogs.iter().position(|d| d.id == id) {
            let d = self.dialogs.remove(di);
            self.close_layer(d.layer);
            if let Some(f) = d.return_focus.filter(|f| self.exists(*f)) { self.focus(Some(f)); }
        }
    }

    pub(crate) fn answer_dialog(&mut self, value: DialogValue) {
        if !self.dialogs.is_empty() { let di = self.dialogs.len() - 1; self.answer_dialog_at(di, value); }
    }

    fn answer_dialog_at(&mut self, di: usize, value: DialogValue) {
        let d = self.dialogs.remove(di);
        self.close_layer(d.layer);
        if let Some(f) = d.return_focus.filter(|f| self.exists(*f)) { self.focus(Some(f)); }
        self.full_damage = true;
        self.out.push(Response::Dialog(d.id, value));
    }

    fn dialog_changed(&mut self, di: usize, from: Id) {
        match &self.dialogs[di].kind {
            DialogKind::Color { sliders, hex, preview } => {
                let (sliders, hex, preview) = (*sliders, *hex, *preview);
                let c = if from == hex {
                    let Some(c) = Color::parse(self.text_of(hex)) else { return };
                    for (s, v) in sliders.iter().zip([c.r(), c.g(), c.b()]) { self.set_value(*s, v as f32); }
                    c
                } else {
                    let v: Vec<u8> = sliders.iter().map(|s| self.value(*s) as u8).collect();
                    let c = Color::rgb(v[0], v[1], v[2]);
                    self.set_text(hex, &format!("#{:06X}", c.0 & 0xFF_FFFF));
                    c
                };
                self.set_image(preview, 96, 32, vec![c.0 & 0xFF_FFFF; 96 * 32]);
            }
            DialogKind::Font { list, size, preview, faces } => {
                let face = self.selected_row(*list).and_then(|i| faces.get(i)).cloned().unwrap_or_default();
                let text = format!("{}  {} pt  AaBbCc 0123", face, self.value(*size) as u32);
                let p = *preview;
                self.set_text(p, &text);
            }
            _ => {}
        }
    }

    // ---- toasts and tooltips -------------------------------------------------------

    /// A short message at the bottom of the window for `ms` milliseconds, with an
    /// optional button that runs a command (`("Undo", "app.undo")`).
    pub fn toast(&mut self, text: &str, action: Option<(&str, &str)>, ms: u64) {
        let layer = self.open_layer(LayerKind::Toast, Rect::ZERO, false, Layout::row().padding_xy(14.0, 8.0).gap(12.0), Fill::Surface);
        self.label(layer, text);
        let action = action.map(|(label, cmd)| {
            let b = self.button(layer, label);
            self.set_button_style(b, ButtonStyle::Flat);
            (b, String::from(cmd))
        });
        let until = self.now + ms;
        self.toasts.push(Toast { layer, until, action });
        self.a11y_dirty = true;
    }

    pub(crate) fn tooltip_owner(&self, mut id: Id) -> Option<Id> {
        loop {
            let n = self.get(id)?;
            if n.tooltip.is_some() { return Some(id); }
            id = n.parent?;
        }
    }

    pub(crate) fn hide_tooltip(&mut self) {
        if let Some((_, _, Some(layer))) = self.tooltip { self.close_layer(layer); self.full_damage = true; }
        self.tooltip = None;
    }

    /// Advances Weft's clock to `now` (milliseconds, any start): shows tooltips, ends
    /// toasts and animates busy bars. Returns how long until it wants another tick.
    pub fn tick(&mut self, now: u64) -> Option<u64> {
        self.now = now;
        let mut next: Option<u64> = None;
        let mut want = |at: u64| { let d = at.saturating_sub(now).max(1); next = Some(next.map_or(d, |n: u64| n.min(d))); };
        if let Some((owner, at, layer)) = self.tooltip {
            if layer.is_none() {
                if now >= at && self.exists(owner) && self.drag == crate::ui::Drag::None && self.menus.is_empty() {
                    let text = self.node(owner).tooltip.clone().unwrap_or_default();
                    let p = self.pointer;
                    let l = self.open_layer(LayerKind::Tooltip, Rect::new(p.x, p.y, 0.0, 0.0), false, Layout::column().padding_xy(8.0, 4.0).max_width(360.0), Fill::Surface);
                    let lab = self.text(l, &text, TextStyle::Caption);
                    if let Kind::Label { wrap, .. } = &mut self.node_mut(lab).kind { *wrap = true; }
                    self.tooltip = Some((owner, at, Some(l)));
                } else { want(at); }
            }
        }
        let (gone, keep): (Vec<Toast>, Vec<Toast>) = core::mem::take(&mut self.toasts).into_iter().partition(|t| t.until <= now);
        self.toasts = keep;
        for t in gone { self.close_layer(t.layer); self.full_damage = true; }
        for t in &self.toasts { want(t.until); }
        // Busy bars move.
        let busy: Vec<Id> = self.nodes.iter().enumerate().filter_map(|(i, n)| n.as_ref().filter(|n| matches!(n.kind, Kind::Progress { value: None })).map(|_| Id(i as u32))).filter(|i| self.shown(*i)).collect();
        if !busy.is_empty() { for b in busy { self.damage_node(b); } want(now + 33); }
        next
    }

}
