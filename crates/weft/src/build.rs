// SPDX-License-Identifier: GPL-3.0-only
//! Making widgets and reading or changing their state.
//!
//! Every maker takes the parent and returns the new widget's [`Id`]; state changes go
//! through setters so Weft knows what to repaint. What the person does comes back as
//! [`crate::Response`]s from [`Ui::handle`].
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::menu::Menu;
use crate::theme::Color;
use crate::ui::Ui;
use crate::widget::*;

impl Ui {
    // ---- containers ------------------------------------------------------------

    /// A container with no look of its own, laid out by `layout`.
    pub fn group(&mut self, parent: Id, layout: Layout) -> Id {
        let id = self.add(parent, Kind::Panel { fill: Fill::None, border: false, radius: 0.0 });
        self.set_layout(id, layout);
        id
    }
    pub fn column(&mut self, parent: Id) -> Id { let g = self.theme.spacing(); self.group(parent, Layout::column().gap(g)) }
    pub fn row(&mut self, parent: Id) -> Id { let g = self.theme.spacing(); self.group(parent, Layout::row().gap(g)) }
    /// A raised surface (a card): rounded, bordered, padded.
    pub fn card(&mut self, parent: Id, layout: Layout) -> Id {
        let r = self.theme.radius;
        let id = self.add(parent, Kind::Panel { fill: Fill::Surface, border: true, radius: r });
        let p = self.theme.padding();
        self.set_layout(id, layout.padding(p));
        id
    }
    /// A container filled with one of the theme's colours.
    pub fn panel(&mut self, parent: Id, fill: Fill, layout: Layout) -> Id {
        let id = self.add(parent, Kind::Panel { fill, border: false, radius: 0.0 });
        self.set_layout(id, layout);
        id
    }
    /// A vertical scroll view: its children may be taller than it.
    pub fn scroll(&mut self, parent: Id) -> Id {
        let id = self.add(parent, Kind::Scroll { y: 0.0, content: 0.0 });
        let g = self.theme.spacing();
        self.set_layout(id, Layout::column().gap(g));
        id
    }
    /// Two panes side by side (or stacked when `vertical`), with a draggable divider.
    /// Add exactly two children. `ratio` is the first pane's share (0..1).
    pub fn split(&mut self, parent: Id, vertical: bool, ratio: f32) -> Id {
        self.add(parent, Kind::Split { vertical, ratio: ratio.clamp(0.05, 0.95) })
    }
    pub fn split_ratio(&self, id: Id) -> f32 { match &self.node(id).kind { Kind::Split { ratio, .. } => *ratio, _ => 0.0 } }

    // ---- text ------------------------------------------------------------------

    pub fn label(&mut self, parent: Id, text: &str) -> Id {
        self.add(parent, Kind::Label { text: text.into(), style: TextStyle::Body, wrap: false, align: Align::Start, color: None })
    }
    /// A label in another size or face.
    pub fn text(&mut self, parent: Id, text: &str, style: TextStyle) -> Id {
        self.add(parent, Kind::Label { text: text.into(), style, wrap: false, align: Align::Start, color: None })
    }
    /// A paragraph that wraps to its width.
    pub fn paragraph(&mut self, parent: Id, text: &str) -> Id {
        self.add(parent, Kind::Label { text: text.into(), style: TextStyle::Body, wrap: true, align: Align::Start, color: None })
    }
    pub fn set_align(&mut self, id: Id, a: Align) {
        if let Kind::Label { align, .. } = &mut self.node_mut(id).kind { *align = a; }
        self.damage_node(id);
    }
    pub fn set_color(&mut self, id: Id, c: Option<Color>) {
        if let Kind::Label { color, .. } = &mut self.node_mut(id).kind { *color = c; }
        self.damage_node(id);
    }

    /// The text of a label, button, field, check box or radio button.
    pub fn text_of(&self, id: Id) -> &str {
        match &self.node(id).kind {
            Kind::Label { text, .. } | Kind::Button { text, .. } | Kind::Check { text, .. } | Kind::Radio { text, .. } => text,
            Kind::Field(f) => &f.edit.text,
            _ => "",
        }
    }
    /// Changes the text of a label, button, field, check box or radio button.
    pub fn set_text(&mut self, id: Id, new: &str) {
        let resize = match &mut self.node_mut(id).kind {
            Kind::Label { text, .. } | Kind::Button { text, .. } | Kind::Check { text, .. } | Kind::Radio { text, .. } => {
                if text == new { return; }
                text.clear();
                text.push_str(new);
                true
            }
            Kind::Field(f) => { if f.edit.text == new { return; } f.edit.set_text(new); false }
            _ => return,
        };
        if resize {
            let t = self.node(id).taffy;
            let _ = self.tree.mark_dirty(t);
            self.layout_dirty = true;
        }
        self.damage_node(id);
    }

    // ---- controls --------------------------------------------------------------

    pub fn button(&mut self, parent: Id, text: &str) -> Id {
        self.add(parent, Kind::Button { text: text.into(), icon: None, style: ButtonStyle::Normal })
    }
    /// A button with an icon, and text if `text` isn't empty (give icon-only buttons a
    /// tooltip; it is also their accessible name).
    pub fn icon_button(&mut self, parent: Id, icon: &str, text: &str) -> Id {
        self.add(parent, Kind::Button { text: text.into(), icon: Some(icon.into()), style: ButtonStyle::Normal })
    }
    pub fn set_button_style(&mut self, id: Id, s: ButtonStyle) {
        if let Kind::Button { style, .. } = &mut self.node_mut(id).kind { *style = s; }
        self.damage_node(id);
    }
    /// A one-line text field.
    pub fn field(&mut self, parent: Id, text: &str, placeholder: &str) -> Id {
        self.add(parent, Ui::new_field(text, placeholder))
    }
    /// A field that shows dots instead of what is typed.
    pub fn password(&mut self, parent: Id, placeholder: &str) -> Id {
        let id = self.field(parent, "", placeholder);
        if let Some(f) = self.field_mut(id) { f.password = true; }
        id
    }
    pub fn set_max_length(&mut self, id: Id, n: Option<usize>) { if let Some(f) = self.field_mut(id) { f.edit.max_len = n; } }
    /// The selected range of a field (byte offsets).
    pub fn field_selection(&self, id: Id) -> (usize, usize) { match &self.node(id).kind { Kind::Field(f) => f.edit.selection(), _ => (0, 0) } }

    pub fn checkbox(&mut self, parent: Id, text: &str, on: bool) -> Id { self.add(parent, Kind::Check { text: text.into(), on, switch: false }) }
    pub fn switch(&mut self, parent: Id, text: &str, on: bool) -> Id { self.add(parent, Kind::Check { text: text.into(), on, switch: true }) }
    /// A radio button; buttons with the same `group` (in one window) exclude each other.
    pub fn radio(&mut self, parent: Id, text: &str, group: u32, on: bool) -> Id {
        let id = self.add(parent, Kind::Radio { text: text.into(), group, on });
        if on { self.set_checked(id, true); }
        id
    }
    pub fn is_checked(&self, id: Id) -> bool { matches!(self.node(id).kind, Kind::Check { on: true, .. } | Kind::Radio { on: true, .. }) }
    pub fn set_checked(&mut self, id: Id, value: bool) {
        let group = match &mut self.node_mut(id).kind {
            Kind::Check { on, .. } => { *on = value; None }
            Kind::Radio { on, group, .. } => { *on = value; value.then_some(*group) }
            _ => None,
        };
        self.damage_node(id);
        if let Some(g) = group {
            let others: Vec<Id> = (0..self.nodes.len() as u32).map(Id).filter(|o| *o != id && matches!(self.get(*o).map(|n| &n.kind), Some(Kind::Radio { group, on: true, .. }) if *group == g)).collect();
            for o in others { if let Kind::Radio { on, .. } = &mut self.node_mut(o).kind { *on = false; } self.damage_node(o); }
        }
    }

    pub fn slider(&mut self, parent: Id, min: f32, max: f32, value: f32) -> Id {
        self.add(parent, Kind::Slider { value: value.clamp(min, max), min, max, step: 0.0 })
    }
    /// Values snap to multiples of `step` (0: continuous). Arrow keys move one step
    /// (or a hundredth of the range).
    pub fn set_step(&mut self, id: Id, s: f32) { if let Kind::Slider { step, .. } = &mut self.node_mut(id).kind { *step = s.max(0.0); } }
    pub fn progress(&mut self, parent: Id, value: Option<f32>) -> Id { self.add(parent, Kind::Progress { value }) }
    /// A slider's value or a progress bar's fraction (0..1; -1 when indeterminate).
    pub fn value(&self, id: Id) -> f32 {
        match &self.node(id).kind { Kind::Slider { value, .. } => *value, Kind::Progress { value } => value.unwrap_or(-1.0), _ => 0.0 }
    }
    pub fn set_value(&mut self, id: Id, v: f32) {
        match &mut self.node_mut(id).kind {
            Kind::Slider { value, min, max, step } => { let mut v = v.clamp(*min, *max); if *step > 0.0 { v = *min + libm::roundf((v - *min) / *step) * *step; } *value = v.clamp(*min, *max); }
            Kind::Progress { value } => *value = if v < 0.0 { None } else { Some(v.clamp(0.0, 1.0)) },
            _ => return,
        }
        self.damage_node(id);
    }

    pub fn icon(&mut self, parent: Id, name: &str, size: f32) -> Id { self.add(parent, Kind::Icon { name: name.into(), size }) }
    /// A picture of `w` by `h` `0x00RRGGBB` pixels, shown at that size in layout units.
    pub fn image(&mut self, parent: Id, w: usize, h: usize, pixels: Vec<u32>) -> Id { self.add(parent, Kind::Image { w, h, pixels }) }
    pub fn set_image(&mut self, id: Id, w2: usize, h2: usize, px: Vec<u32>) {
        if let Kind::Image { w, h, pixels } = &mut self.node_mut(id).kind { *w = w2; *h = h2; *pixels = px; }
        self.relayout();
    }
    pub fn separator(&mut self, parent: Id) -> Id { self.add(parent, Kind::Separator) }

    // ---- lists, tables and trees --------------------------------------------------

    /// A list of `items`, one selectable at a time.
    pub fn list(&mut self, parent: Id, items: &[&str]) -> Id { self.list_with(parent, Box::new(Rows::from_strs(items)), Vec::new()) }
    /// A list, table (with `columns`) or tree over any model. Only visible rows are
    /// asked for, so it scrolls a million rows as fast as ten.
    pub fn list_with(&mut self, parent: Id, model: Box<dyn ListModel>, columns: Vec<Column>) -> Id {
        self.add(parent, Kind::List(Box::new(ListState {
            model, columns, selection: Selection::default(), cursor: 0, anchor: 0, multi: false, scroll: 0.0, sort: None, find: String::new(), find_at: 0, reveal: false,
        })))
    }
    /// Lets Ctrl and Shift clicks (and Shift+arrows) select many rows.
    pub fn set_multi_select(&mut self, id: Id, on: bool) { if let Kind::List(l) = &mut self.node_mut(id).kind { l.multi = on; } }
    pub fn selection(&self, id: Id) -> Option<&Selection> { match &self.node(id).kind { Kind::List(l) => Some(&l.selection), _ => None } }
    pub fn selected_row(&self, id: Id) -> Option<usize> { self.selection(id).and_then(|s| s.first()) }
    pub fn select_row(&mut self, id: Id, row: Option<usize>) {
        if let Kind::List(l) = &mut self.node_mut(id).kind {
            match row { Some(r) if r < l.model.len() => { l.selection.set(r); l.cursor = r; l.anchor = r; } _ => l.selection.clear() }
        }
        self.reveal_cursor(id);
        self.damage_node(id);
    }
    /// Replaces the rows of a list made with [`Ui::list`] (or any [`Rows`] model).
    pub fn set_rows(&mut self, id: Id, rows: Rows) { self.set_model(id, Box::new(rows)); }
    pub fn set_model(&mut self, id: Id, model: Box<dyn ListModel>) {
        if let Kind::List(l) = &mut self.node_mut(id).kind {
            l.model = model;
            let n = l.model.len();
            l.selection.clamp(n);
            l.cursor = l.cursor.min(n.saturating_sub(1));
        }
        let t = self.node(id).taffy;
        let _ = self.tree.mark_dirty(t);
        self.relayout();
    }
    /// The model, to change it in place (call [`Ui::model_changed`] after).
    pub fn model_mut<T: ListModel>(&mut self, id: Id) -> Option<&mut T> {
        match &mut self.node_mut(id).kind { Kind::List(l) => (l.model.as_mut() as &mut dyn core::any::Any).downcast_mut::<T>(), _ => None }
    }
    pub fn model_changed(&mut self, id: Id) {
        if let Kind::List(l) = &mut self.node_mut(id).kind { let n = l.model.len(); l.selection.clamp(n); }
        self.damage_node(id);
    }
    /// The column a table is sorted by (shown with an arrow), and the direction.
    pub fn set_sort(&mut self, id: Id, sort: Option<(usize, bool)>) { if let Kind::List(l) = &mut self.node_mut(id).kind { l.sort = sort; } self.damage_node(id); }
    pub fn set_columns(&mut self, id: Id, cols: Vec<Column>) { if let Kind::List(l) = &mut self.node_mut(id).kind { l.columns = cols; } self.relayout(); }

    // ---- tabs --------------------------------------------------------------------

    /// A tab strip. With pages ([`Ui::tab_page`]) it shows the active page below the
    /// strip; without, it is a strip alone (a browser's tabs over one view).
    pub fn tabs(&mut self, parent: Id) -> Id {
        self.add(parent, Kind::Tabs(Box::new(TabsState {
            tabs: Vec::new(), active: 0, scroll: 0.0, closable: false, reorderable: true, new_button: false, document: false,
            spans: Vec::new(), hover_close: None, hover: None, total: 0.0, room: 0.0, reveal: false,
        })))
    }
    /// Document tabs: closable, a New Tab button, and Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+W,
    /// Ctrl+T and Ctrl+1..9 work anywhere in the window. Loom, Quil, Kaleidoscope and
    /// Spindle use these; each change comes back as [`crate::TabEvent`].
    pub fn document_tabs(&mut self, parent: Id) -> Id {
        let id = self.tabs(parent);
        if let Kind::Tabs(t) = &mut self.node_mut(id).kind { t.closable = true; t.new_button = true; t.document = true; }
        let l = self.node(id).layout.clone();
        self.set_layout(id, l);
        self.update_tab_commands();
        id
    }
    /// Adds a tab and its page (a column); returns the page to fill.
    pub fn tab_page(&mut self, tabs: Id, tab: Tab) -> Id {
        let at = self.push_tab(tabs, tab);
        let g = self.theme.spacing();
        let page = self.add(tabs, Kind::Panel { fill: Fill::None, border: false, radius: 0.0 });
        self.set_layout(page, Layout::column().gap(g).padding(g));
        // A pinned tab goes before the unpinned ones; its page goes with it.
        let n = self.node(tabs).children.len();
        if at + 1 < n {
            let node = self.node_mut(tabs);
            let p = node.children.pop().unwrap();
            node.children.insert(at, p);
            let kids: Vec<_> = self.node(tabs).children.iter().map(|c| self.node(*c).taffy).collect();
            let t = self.node(tabs).taffy;
            self.tree.set_children(t, &kids).unwrap();
        }
        self.after_child_added(tabs);
        page
    }
    /// Adds a tab without a page (strip-only tabs).
    pub fn push_tab(&mut self, tabs: Id, tab: Tab) -> usize {
        let n = match &mut self.node_mut(tabs).kind {
            Kind::Tabs(t) => {
                // Pinned tabs stay before unpinned ones.
                let at = if tab.pinned { t.tabs.iter().take_while(|x| x.pinned).count() } else { t.tabs.len() };
                // The active tab stays the active one.
                if !t.tabs.is_empty() && at <= t.active { t.active += 1; }
                t.tabs.insert(at, tab);
                at
            }
            _ => return 0,
        };
        self.damage_node(tabs);
        self.a11y_dirty = true;
        n
    }
    pub fn tab_list(&self, tabs: Id) -> &[Tab] { match &self.node(tabs).kind { Kind::Tabs(t) => &t.tabs, _ => &[] } }
    pub fn tab_mut(&mut self, tabs: Id, index: usize) -> Option<&mut Tab> {
        self.damage_node(tabs);
        match &mut self.node_mut(tabs).kind { Kind::Tabs(t) => t.tabs.get_mut(index), _ => None }
    }
    pub fn active_tab(&self, tabs: Id) -> usize { match &self.node(tabs).kind { Kind::Tabs(t) => t.active, _ => 0 } }
    /// Shows tab `index` (and its page).
    pub fn set_active_tab(&mut self, tabs: Id, index: usize) {
        let pages = self.node(tabs).children.clone();
        let changed = match &mut self.node_mut(tabs).kind {
            Kind::Tabs(t) if index < t.tabs.len() && t.active != index => { t.active = index; true }
            _ => false,
        };
        if !changed { return; }
        if !pages.is_empty() { for (i, p) in pages.iter().enumerate() { self.page_style(*p, i == index); } }
        self.reveal_tab(tabs, index);
        self.damage_node(tabs);
        self.a11y_dirty = true;
    }
    /// Removes tab `index` and its page; the program decides when (after asking about
    /// unsaved changes, say) in answer to [`crate::TabEvent::Closed`].
    pub fn remove_tab(&mut self, tabs: Id, index: usize) {
        let page = self.node(tabs).children.get(index).copied();
        let pages = !self.node(tabs).children.is_empty();
        let active = match &mut self.node_mut(tabs).kind {
            Kind::Tabs(t) if index < t.tabs.len() => {
                t.tabs.remove(index);
                if t.active > index || t.active >= t.tabs.len() { t.active = t.active.saturating_sub(1); }
                t.active
            }
            _ => return,
        };
        if let Some(p) = page { if pages { self.remove(p); } }
        let pages = self.node(tabs).children.clone();
        for (i, p) in pages.iter().enumerate() { self.page_style(*p, i == active); }
        self.damage_node(tabs);
        self.relayout();
    }
    /// Moves tab `from` to `to`, its page with it.
    pub fn move_tab(&mut self, tabs: Id, from: usize, to: usize) {
        let ok = match &mut self.node_mut(tabs).kind {
            Kind::Tabs(t) if from < t.tabs.len() && to < t.tabs.len() && from != to => {
                let tab = t.tabs.remove(from);
                t.tabs.insert(to, tab);
                if t.active == from { t.active = to; } else if from < t.active && to >= t.active { t.active -= 1; } else if from > t.active && to <= t.active { t.active += 1; }
                true
            }
            _ => false,
        };
        if !ok { return; }
        if self.node(tabs).children.len() > from.max(to) {
            let node = self.node_mut(tabs);
            let p = node.children.remove(from);
            node.children.insert(to, p);
            let kids: Vec<_> = self.node(tabs).children.iter().map(|c| self.node(*c).taffy).collect();
            let t = self.node(tabs).taffy;
            self.tree.set_children(t, &kids).unwrap();
        }
        self.damage_node(tabs);
        self.relayout();
    }
    pub fn set_tabs_closable(&mut self, tabs: Id, on: bool) { if let Kind::Tabs(t) = &mut self.node_mut(tabs).kind { t.closable = on; } self.damage_node(tabs); }

    // ---- menus -------------------------------------------------------------------

    /// A menu bar with these menus (F10 opens the first).
    pub fn menubar(&mut self, parent: Id, menus: Vec<Menu>) -> Id { self.add(parent, Kind::MenuBar { menus, open: None }) }
}
