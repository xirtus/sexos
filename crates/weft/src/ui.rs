// SPDX-License-Identifier: GPL-3.0-only
//! The widget tree: building it, laying it out with taffy, finding what is under the
//! pointer, and the keyboard focus chain.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use taffy::prelude::*;

use crate::edit::LineEdit;
use crate::geom::{Point, Rect};
use crate::i18n::Strings;
use crate::icons::Icons;
use crate::menu::{Command, Commands, Menu};
use crate::text::{Face, Fonts};
use crate::theme::{Color, Theme};
use crate::widget::*;
use crate::widget::Layout;
use crate::Response;

pub(crate) struct Node {
    pub kind: Kind,
    pub taffy: NodeId,
    pub parent: Option<Id>,
    pub children: Vec<Id>,
    /// Where it is, in window layout units, after the last layout.
    pub rect: Rect,
    pub enabled: bool,
    pub visible: bool,
    pub tooltip: Option<String>,
    /// The accessible name when the visible text isn't one (icon buttons).
    pub label: Option<String>,
    pub context: Option<Menu>,
    /// The layout the program asked for; Weft adds per-kind defaults under it.
    pub layout: Layout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerKind { Main, Menu, Dialog, Palette, Toast, Tooltip }

pub(crate) struct Layer {
    pub root: Id,
    pub kind: LayerKind,
    /// For popups: the point or rectangle they open from.
    pub anchor: Rect,
    /// Below the anchor (menus from a bar) or to its right (submenus); else at the point.
    pub below: bool,
}

/// What the pointer is dragging.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Drag {
    None,
    Slider(Id),
    Split(Id),
    ScrollBar { id: Id, grab: f32 },
    ListSelect(Id),
    FieldSelect(Id),
    Tab { id: Id, from: usize, x0: f32, moved: bool },
}

pub(crate) struct OpenMenu {
    pub layer: Id,
    pub menu: Menu,
    pub hot: Option<usize>,
    pub rows: Vec<Id>,
    /// The widget the menu is about (context menus) or the menu bar and its index.
    pub target: Option<Id>,
    pub bar: Option<(Id, usize)>,
    /// For a tab strip's context menu, the tab it was opened on.
    pub tab: Option<usize>,
}

/// A dialog's handle, returned when it opens and given back with its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DialogId(pub u32);

pub(crate) enum DialogKind {
    Message,
    Prompt { field: Id },
    Color { sliders: [Id; 3], hex: Id, preview: Id },
    Font { list: Id, size: Id, preview: Id, faces: Vec<String> },
}

pub(crate) struct DialogState {
    pub id: DialogId,
    pub layer: Id,
    pub buttons: Vec<Id>,
    pub cancel: usize,
    pub default: usize,
    pub kind: DialogKind,
    /// Where focus goes back to when the dialog closes.
    pub return_focus: Option<Id>,
}

pub(crate) struct PaletteState { pub layer: Id, pub field: Id, pub list: Id, pub results: Vec<usize>, pub return_focus: Option<Id> }

pub(crate) struct Toast { pub layer: Id, pub until: u64, pub action: Option<(Id, String)> }

/// A window's interface: the widgets, their layout and state, and the theme, strings,
/// icons and commands they use. One per window.
pub struct Ui {
    pub(crate) nodes: Vec<Option<Node>>,
    free: Vec<u32>,
    pub(crate) tree: TaffyTree<Id>,
    pub(crate) layers: Vec<Layer>,
    pub(crate) theme: Theme,
    pub fonts: Fonts,
    pub icons: Icons,
    pub(crate) strings: Strings,
    pub commands: Commands,
    pub(crate) focus: Option<Id>,
    pub(crate) focus_visible: bool,
    pub(crate) hover: Option<Id>,
    pub(crate) pressed: Option<Id>,
    pub(crate) drag: Drag,
    /// The window in device pixels.
    pub(crate) size: (u32, u32),
    pub(crate) layout_dirty: bool,
    pub(crate) full_damage: bool,
    pub(crate) damage: Rect,
    pub(crate) out: Vec<Response>,
    pub(crate) now: u64,
    pub(crate) last_click: (u64, Point, u8),
    pub(crate) clipboard: String,
    pub(crate) menus: Vec<OpenMenu>,
    pub(crate) palette: Option<PaletteState>,
    pub(crate) dialogs: Vec<DialogState>,
    pub(crate) toasts: Vec<Toast>,
    pub(crate) tooltip: Option<(Id, u64, Option<Id>)>,
    pub(crate) next_dialog: u32,
    pub(crate) pointer: Point,
    pub(crate) a11y_dirty: bool,
    /// The widget the last context menu was opened on.
    pub(crate) context_target: Option<Id>,
    /// Where every widget was at the last layout, to damage only what changed.
    pub(crate) placed: Vec<Rect>,
    /// Nodes the last accessibility update described (to send only changes).
    pub(crate) a11y_sent: alloc::collections::BTreeMap<u64, accesskit::Node>,
}

/// A label's or button's text size for the theme.
pub(crate) fn text_px(theme: &Theme, style: TextStyle) -> f32 { style.size(theme.font_size) }

impl Ui {
    /// An empty window of `width` by `height` device pixels.
    pub fn new(width: u32, height: u32, theme: Theme, strings: Strings) -> Ui {
        let mut tree = TaffyTree::new();
        tree.disable_rounding();
        let mut ui = Ui {
            nodes: Vec::new(), free: Vec::new(), tree, layers: Vec::new(), theme, fonts: Fonts::new(), icons: Icons::new(), strings,
            commands: Commands::default(), focus: None, focus_visible: false, hover: None, pressed: None, drag: Drag::None,
            size: (width.max(1), height.max(1)), layout_dirty: true, full_damage: true, damage: Rect::ZERO, out: Vec::new(), now: 0,
            last_click: (0, Point::default(), 0), clipboard: String::new(), menus: Vec::new(), palette: None, dialogs: Vec::new(),
            toasts: Vec::new(), tooltip: None, next_dialog: 1, pointer: Point::default(), a11y_dirty: true,
            context_target: None, placed: Vec::new(), a11y_sent: alloc::collections::BTreeMap::new(),
        };
        let root = ui.alloc(Kind::Panel { fill: Fill::Window, border: false, radius: 0.0 }, None, Layout::column());
        ui.layers.push(Layer { root, kind: LayerKind::Main, anchor: Rect::ZERO, below: false });
        ui.builtin_commands();
        ui
    }

    fn builtin_commands(&mut self) {
        let s = &self.strings;
        let list = [
            Command::new("weft.palette", &s.get("weft-palette")).shortcut("Ctrl+Shift+P").icon("command").hidden(),
            Command::new("weft.tab-next", &s.get("weft-next-tab")).shortcut("Ctrl+Tab"),
            Command::new("weft.tab-previous", &s.get("weft-previous-tab")).shortcut("Ctrl+Shift+Tab"),
            Command::new("weft.tab-close", &s.get("weft-close-tab")).shortcut("Ctrl+W").icon("close"),
            Command::new("weft.tab-new", &s.get("weft-new-tab")).shortcut("Ctrl+T").icon("plus"),
            Command::new("weft.tab-close-others", &s.get("weft-close-other-tabs")),
            Command::new("weft.tab-pin", &s.get("weft-pin-tab")).icon("pin").hidden(),
            Command::new("weft.cut", &s.get("weft-cut")).icon("cut").hidden(),
            Command::new("weft.copy", &s.get("weft-copy")).icon("copy").hidden(),
            Command::new("weft.paste", &s.get("weft-paste")).icon("paste").hidden(),
            Command::new("weft.select-all", &s.get("weft-select-all")).hidden(),
            Command::new("weft.undo", &s.get("weft-undo")).icon("undo").hidden(),
            Command::new("weft.redo", &s.get("weft-redo")).icon("redo").hidden(),
        ];
        for c in list { self.commands.add(c); }
        self.update_tab_commands();
    }

    /// The main layer's root: build the window's widgets under it.
    pub fn root(&self) -> Id { self.layers[0].root }
    pub fn theme(&self) -> &Theme { &self.theme }
    pub fn strings(&self) -> &Strings { &self.strings }
    /// A translated string (see [`Strings::format`]).
    pub fn tr(&self, id: &str) -> String { self.strings.get(id) }

    /// Changes the theme and repaints everything.
    pub fn set_theme(&mut self, theme: Theme) { self.theme = theme; self.relayout(); self.full_damage = true; }
    /// Changes the language: Weft's own strings follow; the program re-sets its texts.
    pub fn set_strings(&mut self, strings: Strings) {
        self.strings = strings;
        let keep: Vec<Command> = self.commands.all().iter().filter(|c| !c.id.starts_with("weft.")).cloned().collect();
        self.commands = Commands::default();
        self.builtin_commands();
        for c in keep { self.commands.add(c); }
        self.relayout();
    }
    /// The window's pixel area changed.
    pub fn resize(&mut self, width: u32, height: u32) { self.size = (width.max(1), height.max(1)); self.relayout(); self.full_damage = true; }
    pub fn size(&self) -> (u32, u32) { self.size }
    /// The window's size in layout units.
    pub fn logical_size(&self) -> (f32, f32) { (self.size.0 as f32 / self.theme.scale, self.size.1 as f32 / self.theme.scale) }

    pub(crate) fn relayout(&mut self) { self.layout_dirty = true; self.a11y_dirty = true; }

    pub(crate) fn node(&self, id: Id) -> &Node { self.nodes[id.0 as usize].as_ref().expect("live widget") }
    pub(crate) fn node_mut(&mut self, id: Id) -> &mut Node { self.nodes[id.0 as usize].as_mut().expect("live widget") }
    pub(crate) fn get(&self, id: Id) -> Option<&Node> { self.nodes.get(id.0 as usize).and_then(|n| n.as_ref()) }
    pub fn exists(&self, id: Id) -> bool { self.get(id).is_some() }

    fn alloc(&mut self, kind: Kind, parent: Option<Id>, layout: Layout) -> Id {
        let index = match self.free.pop() { Some(i) => i, None => { self.nodes.push(None); self.nodes.len() as u32 - 1 } };
        let id = Id(index);
        let style = self.style_for(&kind, &layout);
        let taffy = if leaf(&kind) { self.tree.new_leaf_with_context(style, id).unwrap() } else { self.tree.new_leaf(style).unwrap() };
        self.nodes[index as usize] = Some(Node { kind, taffy, parent, children: Vec::new(), rect: Rect::ZERO, enabled: true, visible: true, tooltip: None, label: None, context: None, layout });
        if let Some(p) = parent {
            let pt = self.node(p).taffy;
            self.tree.add_child(pt, taffy).unwrap();
            self.node_mut(p).children.push(id);
            self.after_child_added(p);
        }
        self.relayout();
        id
    }

    /// Adds a widget of `kind` under `parent`.
    pub(crate) fn add(&mut self, parent: Id, kind: Kind) -> Id { self.alloc(kind, Some(parent), Layout::default()) }
    pub(crate) fn add_detached(&mut self, kind: Kind, layout: Layout) -> Id { self.alloc(kind, None, layout) }

    /// Removes a widget and everything under it.
    pub fn remove(&mut self, id: Id) {
        if id == self.root() { return; }
        let Some(node) = self.nodes.get_mut(id.0 as usize).and_then(|n| n.take()) else { return };
        for c in node.children.iter().rev() {
            if let Some(n) = self.nodes.get_mut(c.0 as usize).and_then(|n| n.as_mut()) { n.parent = None; }
            self.remove(*c);
        }
        let _ = self.tree.remove(node.taffy);
        if let Some(p) = node.parent {
            if let Some(pn) = self.nodes.get_mut(p.0 as usize).and_then(|n| n.as_mut()) { pn.children.retain(|c| *c != id); }
        }
        self.free.push(id.0);
        for slot in [&mut self.focus, &mut self.hover, &mut self.pressed] { if *slot == Some(id) { *slot = None; } }
        if matches!(self.drag, Drag::Slider(x) | Drag::Split(x) | Drag::ListSelect(x) | Drag::FieldSelect(x) if x == id) { self.drag = Drag::None; }
        self.relayout();
    }

    /// Removes everything under `id`, keeping `id`.
    pub fn clear(&mut self, id: Id) {
        let children = self.node(id).children.clone();
        for c in children { self.remove(c); }
    }

    pub fn children(&self, id: Id) -> &[Id] { &self.node(id).children }
    pub fn parent(&self, id: Id) -> Option<Id> { self.node(id).parent }

    /// Where `id` is, in layout units from the window's top-left, after the last paint.
    pub fn rect(&self, id: Id) -> Rect { self.node(id).rect }

    // ---- per-widget settings -------------------------------------------------

    pub fn set_layout(&mut self, id: Id, layout: Layout) {
        let style = self.style_for(&self.node(id).kind, &layout);
        let t = self.node(id).taffy;
        self.tree.set_style(t, style).unwrap();
        self.node_mut(id).layout = layout;
        self.relayout();
    }
    pub fn set_enabled(&mut self, id: Id, on: bool) {
        if self.node(id).enabled != on { self.node_mut(id).enabled = on; self.damage_node(id); self.a11y_dirty = true; }
        if !on && self.focus == Some(id) { self.focus = None; }
    }
    pub fn is_enabled(&self, id: Id) -> bool { self.node(id).enabled }
    /// Hides or shows a widget; hidden widgets take no space.
    pub fn set_visible(&mut self, id: Id, on: bool) {
        if self.node(id).visible == on { return; }
        self.node_mut(id).visible = on;
        let style = self.style_for(&self.node(id).kind, &self.node(id).layout.clone());
        let t = self.node(id).taffy;
        self.tree.set_style(t, style).unwrap();
        if !on && self.focus.map_or(false, |f| self.is_ancestor(id, f)) { self.focus = None; }
        self.relayout();
    }
    pub fn set_tooltip(&mut self, id: Id, text: &str) { self.node_mut(id).tooltip = Some(text.into()); }
    /// The name a screen reader says, when the visible text isn't one.
    pub fn set_label(&mut self, id: Id, text: &str) { self.node_mut(id).label = Some(text.into()); self.a11y_dirty = true; }
    /// A right-click (or Shift+F10, or the Menu key) on `id` or anything inside it opens `menu`.
    pub fn set_context_menu(&mut self, id: Id, menu: Menu) { self.node_mut(id).context = Some(menu); }

    pub(crate) fn is_ancestor(&self, a: Id, mut b: Id) -> bool {
        loop {
            if a == b { return true; }
            match self.get(b).and_then(|n| n.parent) { Some(p) => b = p, None => return false }
        }
    }
    /// Whether `id` and all its ancestors are visible.
    pub(crate) fn shown(&self, mut id: Id) -> bool {
        loop {
            let Some(n) = self.get(id) else { return false };
            if !n.visible { return false; }
            match n.parent { Some(p) => id = p, None => return true }
        }
    }

    // ---- damage ----------------------------------------------------------------

    pub(crate) fn damage_rect(&mut self, r: Rect) {
        let r = r.inset(-4.0);
        self.damage = self.damage.union(&r);
    }
    pub(crate) fn damage_node(&mut self, id: Id) {
        if let Some(n) = self.get(id) { let r = n.rect; self.damage_rect(r); }
        self.a11y_dirty = true;
    }
    /// Whether anything needs painting.
    pub fn needs_paint(&self) -> bool { self.layout_dirty || self.full_damage || !self.damage.is_empty() }
    /// Repaint everything next time (after drawing over the window yourself).
    pub fn invalidate(&mut self) { self.full_damage = true; }

    // ---- layout --------------------------------------------------------------

    /// The taffy style for a widget: per-kind defaults, then the program's layout.
    fn style_for(&self, kind: &Kind, layout: &Layout) -> Style {
        let mut s = layout.0.clone();
        let t = &self.theme;
        match kind {
            Kind::List(_) => { if s.flex_grow == 0.0 { s.flex_grow = 1.0; } if s.min_size.height == <Style>::DEFAULT.min_size.height { s.min_size.height = length(t.row_height() * 3.0); } }
            Kind::Scroll { .. } => { s.overflow = taffy::Point { x: taffy::style::Overflow::Hidden, y: taffy::style::Overflow::Scroll }; if s.flex_grow == 0.0 { s.flex_grow = 1.0; } s.min_size.height = length(0.0); if s.display == Display::Block { s.display = Display::Flex; s.flex_direction = FlexDirection::Column; } }
            Kind::Tabs(st) => {
                s.display = Display::Flex;
                s.flex_direction = FlexDirection::Column;
                s.padding.top = length(tab_strip_height(t));
                if st.document && s.flex_grow == 0.0 { s.flex_grow = 1.0; }
            }
            Kind::Split { vertical, .. } => {
                s.display = Display::Flex;
                s.flex_direction = if *vertical { FlexDirection::Column } else { FlexDirection::Row };
                s.gap = Size { width: length(SPLIT_GAP), height: length(SPLIT_GAP) };
                if s.flex_grow == 0.0 { s.flex_grow = 1.0; }
                s.min_size = Size { width: length(0.0), height: length(0.0) };
            }
            Kind::Separator => { s.align_self = Some(AlignSelf::STRETCH); s.flex_shrink = 0.0; }
            Kind::MenuBar { .. } => { s.flex_shrink = 0.0; }
            Kind::Panel { .. } if layout.0.display == Display::Block && layout.0.flex_direction == FlexDirection::Row => {
                s.display = Display::Flex;
                s.flex_direction = FlexDirection::Column;
            }
            _ => {}
        }
        s
    }

    pub(crate) fn after_child_added(&mut self, parent: Id) {
        // Tab pages: only the active one shows. Split panes: sized by the ratio.
        match &self.node(parent).kind {
            Kind::Tabs(st) => {
                let active = st.active;
                let kids = self.node(parent).children.clone();
                for (i, c) in kids.iter().enumerate() { self.page_style(*c, i == active); }
            }
            Kind::Split { ratio, .. } => { let r = *ratio; self.split_styles(parent, r); }
            _ => {}
        }
    }

    pub(crate) fn page_style(&mut self, page: Id, on: bool) {
        let mut s = self.style_for(&self.node(page).kind, &self.node(page).layout.clone());
        s.flex_grow = 1.0;
        s.min_size.height = length(0.0);
        if !on { s.display = Display::None; }
        let t = self.node(page).taffy;
        self.tree.set_style(t, s).unwrap();
        self.relayout();
    }

    pub(crate) fn split_styles(&mut self, split: Id, ratio: f32) {
        let kids = self.node(split).children.clone();
        for (i, c) in kids.iter().enumerate() {
            let mut s = self.style_for(&self.node(*c).kind, &self.node(*c).layout.clone());
            if i == 0 { s.flex_basis = percent(ratio); s.flex_grow = 0.0; s.flex_shrink = 1.0; } else { s.flex_basis = length(0.0); s.flex_grow = 1.0; }
            s.min_size = Size { width: length(0.0), height: length(0.0) };
            let t = self.node(*c).taffy;
            self.tree.set_style(t, s).unwrap();
        }
        self.relayout();
    }

    /// Lays everything out if anything changed.
    pub fn layout(&mut self) {
        if !self.layout_dirty { return; }
        self.layout_dirty = false;
        let (w, h) = self.logical_size();
        for li in 0..self.layers.len() {
            let (root, kind) = (self.layers[li].root, self.layers[li].kind);
            let rt = self.node(root).taffy;
            let avail = match kind {
                LayerKind::Main => Size { width: AvailableSpace::Definite(w), height: AvailableSpace::Definite(h) },
                _ => Size { width: AvailableSpace::Definite((w - 32.0).max(80.0)), height: AvailableSpace::Definite((h - 32.0).max(60.0)) },
            };
            if kind == LayerKind::Main {
                let mut st = self.tree.style(rt).unwrap().clone();
                st.size = Size { width: length(w), height: length(h) };
                self.tree.set_style(rt, st).unwrap();
            }
            let Ui { tree, nodes, fonts, theme, .. } = self;
            let _ = tree.compute_layout_with_measure(rt, avail, |inputs, _nid, ctx, style| {
                let kind = ctx.and_then(|id| nodes.get(id.0 as usize)).and_then(|n| n.as_ref()).map(|n| &n.kind);
                taffy::compute_leaf_layout(inputs, style, |_, _| 0.0, |known, avail| match kind {
                    Some(k) => measure(k, fonts, theme, known, avail),
                    None => Size::ZERO,
                })
            });
            let size = self.tree.layout(rt).map(|l| l.size).unwrap_or(Size::ZERO);
            let origin = self.layer_origin(li, size.width, size.height, w, h);
            self.place(root, origin, false);
        }
        // Repaint only what moved, grew, shrank, appeared or went away.
        if !self.full_damage {
            let n = self.nodes.len().max(self.placed.len());
            for i in 0..n {
                let old = self.placed.get(i).copied().unwrap_or(Rect::ZERO);
                let new = self.nodes.get(i).and_then(|n| n.as_ref()).map_or(Rect::ZERO, |n| n.rect);
                if old != new {
                    if !old.is_empty() { self.damage_rect(old); }
                    if !new.is_empty() { self.damage_rect(new); }
                }
            }
        }
        self.placed = self.nodes.iter().map(|n| n.as_ref().map_or(Rect::ZERO, |n| n.rect)).collect();
        self.a11y_dirty = true;
        self.update_tab_commands();
        self.after_layout();
    }

    fn layer_origin(&self, li: usize, lw: f32, lh: f32, w: f32, h: f32) -> Point {
        let l = &self.layers[li];
        let clamp = |x: f32, y: f32| Point::new(x.min(w - lw - 4.0).max(4.0), y.min(h - lh - 4.0).max(4.0));
        match l.kind {
            LayerKind::Main => Point::new(0.0, 0.0),
            LayerKind::Menu if l.below => clamp(l.anchor.x, l.anchor.bottom()),
            LayerKind::Menu => {
                // A submenu opens to the right of its row, or to the left if no room.
                if l.anchor.w > 0.0 {
                    let x = if l.anchor.right() + lw <= w { l.anchor.right() - 2.0 } else { l.anchor.x - lw + 2.0 };
                    clamp(x, l.anchor.y - 4.0)
                } else { clamp(l.anchor.x, l.anchor.y) }
            }
            LayerKind::Tooltip => clamp(l.anchor.x, l.anchor.y + 20.0),
            LayerKind::Dialog => Point::new(((w - lw) / 2.0).max(4.0), ((h - lh) / 2.5).max(4.0)),
            LayerKind::Palette => Point::new(((w - lw) / 2.0).max(4.0), (h * 0.12).min(80.0)),
            LayerKind::Toast => {
                let below = self.layers[li + 1..].iter().filter(|x| x.kind == LayerKind::Toast).count() as f32;
                Point::new(((w - lw) / 2.0).max(4.0), h - lh - 16.0 - below * (lh + 8.0))
            }
        }
    }

    /// Absolute rectangles from taffy's relative ones.
    fn place(&mut self, id: Id, origin: Point, hidden: bool) {
        let t = self.node(id).taffy;
        let l = *self.tree.layout(t).unwrap();
        let rect = Rect::new(origin.x + l.location.x, origin.y + l.location.y, l.size.width, l.size.height);
        let shown = !hidden && self.tree.style(t).map_or(true, |s| s.display != Display::None);
        let node = self.node_mut(id);
        node.rect = if shown { rect } else { Rect::new(rect.x, rect.y, 0.0, 0.0) };
        let mut child_origin = Point::new(rect.x, rect.y);
        let mut content_bottom: f32 = 0.0;
        if let Kind::Scroll { y, .. } = node.kind { child_origin.y -= y; }
        let kids = node.children.clone();
        for c in kids {
            self.place(c, child_origin, !shown);
            let cr = self.node(c).rect;
            content_bottom = content_bottom.max(cr.bottom() - child_origin.y);
        }
        if let Kind::Scroll { y, content } = &mut self.node_mut(id).kind {
            let pad = l.padding.bottom;
            *content = content_bottom + pad;
            let max = (*content - rect.h).max(0.0);
            if *y > max {
                *y = max;
                // Clamped: place the children again at the right offset.
                self.place(id, origin, hidden);
            }
        }
    }

    // ---- hit testing -------------------------------------------------------------

    /// The topmost layer that takes the pointer at `p`, and the deepest widget there.
    pub(crate) fn hit(&self, p: Point) -> (Option<usize>, Option<Id>) {
        for li in (0..self.layers.len()).rev() {
            let l = &self.layers[li];
            if l.kind == LayerKind::Tooltip || l.kind == LayerKind::Toast && !self.node(l.root).rect.contains(p) { continue; }
            if self.node(l.root).rect.contains(p) { return (Some(li), self.deepest(l.root, p)); }
            // Modal layers swallow the pointer outside themselves.
            if matches!(l.kind, LayerKind::Dialog | LayerKind::Palette | LayerKind::Menu) { return (Some(li), None); }
        }
        (None, None)
    }

    fn deepest(&self, id: Id, p: Point) -> Option<Id> {
        let n = self.node(id);
        if !n.visible || !n.rect.contains(p) { return None; }
        for c in n.children.iter().rev() {
            if let Some(d) = self.deepest(*c, p) { return Some(d); }
        }
        Some(id)
    }

    /// Focusable widgets of the layer that has the keyboard, in order.
    pub(crate) fn focus_chain(&self) -> Vec<Id> {
        let root = self.key_layer_root();
        let mut out = Vec::new();
        self.collect_focusable(root, &mut out);
        out
    }
    fn collect_focusable(&self, id: Id, out: &mut Vec<Id>) {
        let n = self.node(id);
        if !n.visible || !n.enabled { return; }
        if let Kind::Tabs(_) = n.kind { out.push(id); }
        else if n.kind.focusable() { out.push(id); }
        // Inactive tab pages are display:none: their rects are empty.
        for c in &n.children {
            if self.node(*c).rect.w > 0.0 || self.node(*c).rect.h > 0.0 || self.layout_dirty { self.collect_focusable(*c, out); }
        }
    }
    pub(crate) fn key_layer_root(&self) -> Id {
        self.layers.iter().rev().find(|l| matches!(l.kind, LayerKind::Dialog | LayerKind::Palette | LayerKind::Main)).map(|l| l.root).unwrap_or(self.root())
    }

    /// Moves keyboard focus to `id` (or nowhere).
    pub fn focus(&mut self, id: Option<Id>) {
        if self.focus == id { return; }
        if let Some(old) = self.focus { if self.exists(old) { self.damage_node(old); } }
        self.focus = id;
        if let Some(new) = id { self.damage_node(new); self.scroll_into_view(new); }
        self.a11y_dirty = true;
    }
    pub fn focused(&self) -> Option<Id> { self.focus }

    /// Scrolls every scroll view holding `id` so it is visible.
    pub(crate) fn scroll_into_view(&mut self, id: Id) {
        let r = self.node(id).rect;
        let mut cur = self.node(id).parent;
        while let Some(p) = cur {
            let pr = self.node(p).rect;
            if let Kind::Scroll { y, .. } = &mut self.node_mut(p).kind {
                let old = *y;
                if r.y < pr.y { *y -= pr.y - r.y + 4.0; }
                else if r.bottom() > pr.bottom() { *y += r.bottom() - pr.bottom() + 4.0; }
                *y = y.max(0.0);
                if *y != old { self.relayout(); }
            }
            cur = self.node(p).parent;
        }
    }

    /// Ids of widgets whose kind name is `kind` (tests and tools).
    pub fn find_kind(&self, kind: &str) -> Vec<Id> {
        self.nodes.iter().enumerate().filter_map(|(i, n)| n.as_ref().filter(|n| n.kind.name() == kind).map(|_| Id(i as u32))).collect()
    }
    /// The first widget showing `text` (button, label, check, radio), for tests and tools.
    pub fn find_text(&self, text: &str) -> Option<Id> {
        self.nodes.iter().enumerate().find_map(|(i, n)| {
            let n = n.as_ref()?;
            let t = match &n.kind { Kind::Label { text, .. } | Kind::Button { text, .. } | Kind::Check { text, .. } | Kind::Radio { text, .. } => text.as_str(), _ => return None };
            (t == text && self.shown(Id(i as u32)) && (self.layout_dirty || !n.rect.is_empty())).then_some(Id(i as u32))
        })
    }

    pub(crate) fn open_layer(&mut self, kind: LayerKind, anchor: Rect, below: bool, layout: Layout, fill: Fill) -> Id {
        let root = self.add_detached(Kind::Panel { fill, border: true, radius: self.theme.radius + 2.0 }, layout);
        self.layers.push(Layer { root, kind, anchor, below });
        // Shadows and the dialog scrim reach past the layer: repaint everything.
        self.relayout();
        self.full_damage = true;
        root
    }
    pub(crate) fn close_layer(&mut self, root: Id) {
        self.layers.retain(|l| l.root != root);
        // Removing the root takes its widgets; focus inside it goes back to the window.
        let had_focus = self.focus.map_or(false, |f| self.exists(f) && self.is_ancestor(root, f));
        self.remove_detached(root);
        if had_focus { self.focus = None; }
        self.relayout();
        self.full_damage = true;
    }
    fn remove_detached(&mut self, root: Id) {
        let Some(node) = self.nodes.get_mut(root.0 as usize).and_then(|n| n.take()) else { return };
        for c in node.children.iter() {
            if let Some(n) = self.nodes.get_mut(c.0 as usize).and_then(|n| n.as_mut()) { n.parent = None; }
            self.remove(*c);
        }
        let _ = self.tree.remove(node.taffy);
        self.free.push(root.0);
    }

    /// Edit state of a field (for the editing code).
    pub(crate) fn field_mut(&mut self, id: Id) -> Option<&mut FieldState> {
        match &mut self.node_mut(id).kind { Kind::Field(f) => Some(f), _ => None }
    }

    pub(crate) fn new_field(text: &str, placeholder: &str) -> Kind {
        Kind::Field(Box::new(FieldState { edit: LineEdit::new(text), placeholder: placeholder.into(), password: false, scroll: 0.0 }))
    }

    /// Keeps the tab commands enabled only when a document tab strip exists.
    pub(crate) fn update_tab_commands(&mut self) {
        let has = self.doc_tabs().is_some();
        for id in ["weft.tab-next", "weft.tab-previous", "weft.tab-close", "weft.tab-new", "weft.tab-close-others"] {
            self.commands.set_enabled(id, has);
            if let Some(c) = self.commands.get_mut(id) { c.in_palette = has; }
        }
    }
    pub(crate) fn doc_tabs(&self) -> Option<Id> {
        self.nodes.iter().enumerate().find_map(|(i, n)| match n.as_ref().map(|n| &n.kind) { Some(Kind::Tabs(t)) if t.document && self.shown(Id(i as u32)) => Some(Id(i as u32)), _ => None })
    }

}

/// Space between split panes (the divider).
pub(crate) const SPLIT_GAP: f32 = 6.0;

pub(crate) fn tab_strip_height(t: &Theme) -> f32 { t.control_height() + 4.0 }

fn leaf(kind: &Kind) -> bool {
    !matches!(kind, Kind::Panel { .. } | Kind::Scroll { .. } | Kind::Tabs(_) | Kind::Split { .. })
}

/// The natural size of a leaf widget in layout units.
fn measure(kind: &Kind, fonts: &Fonts, theme: &Theme, known: Size<Option<f32>>, avail: Size<AvailableSpace>) -> Size<f32> {
    let s = theme.scale;
    let width_of = |face: Face, size: f32, text: &str| fonts.measure(face, size * s, text) / s;
    let line = |face: Face, size: f32| fonts.line_height(face, size * s) / s;
    let ch = theme.control_height();
    let pad = theme.padding();
    let body = theme.font_size;
    let r = match kind {
        Kind::Label { text, style, wrap, .. } => {
            let face = if *style == TextStyle::Mono { Face::Mono } else { Face::Sans };
            let size = text_px(theme, *style);
            let limit = known.width.or(match avail.width { AvailableSpace::Definite(w) => Some(w), _ => None });
            if *wrap {
                if let Some(w) = limit {
                    let lines = fonts.wrap(face, size * s, text, w * s).len().max(1);
                    let widest = fonts.wrap(face, size * s, text, w * s).iter().map(|(a, b)| width_of(face, size, &text[*a..*b])).fold(0.0, f32::max);
                    return Size { width: known.width.unwrap_or(widest.min(w)), height: lines as f32 * line(face, size) };
                }
            }
            let n = text.split('\n').count().max(1);
            let w = text.split('\n').map(|l| width_of(face, size, l)).fold(0.0, f32::max);
            Size { width: libm::ceilf(w), height: n as f32 * line(face, size) }
        }
        Kind::Button { text, icon, .. } => {
            let tw = if text.is_empty() { 0.0 } else { width_of(Face::Sans, body, text) };
            let iw = if icon.is_some() { 18.0 + if text.is_empty() { 0.0 } else { 6.0 } } else { 0.0 };
            let w = if text.is_empty() { ch } else { (tw + iw + pad * 2.0).max(ch * 2.0) };
            Size { width: libm::ceilf(w), height: ch }
        }
        Kind::Field(_) => Size { width: 200.0, height: ch },
        Kind::Check { text, switch, .. } => Size { width: if *switch { 36.0 } else { 18.0 } + 8.0 + width_of(Face::Sans, body, text), height: ch },
        Kind::Radio { text, .. } => Size { width: 18.0 + 8.0 + width_of(Face::Sans, body, text), height: ch },
        Kind::Slider { .. } => Size { width: 160.0, height: ch },
        Kind::Progress { .. } => Size { width: 160.0, height: 8.0 },
        Kind::List(l) => {
            let rows = l.model.len().min(6).max(3) as f32;
            let header = if l.columns.is_empty() { 0.0 } else { theme.row_height() };
            let w = if l.columns.is_empty() { 200.0 } else { l.columns.iter().map(|c| c.width).sum::<f32>().max(200.0) };
            Size { width: w, height: rows * theme.row_height() + header }
        }
        Kind::Tabs(_) => Size { width: 200.0, height: tab_strip_height(theme) },
        Kind::Icon { size, .. } => Size { width: *size, height: *size },
        Kind::Image { w, h, .. } => Size { width: *w as f32, height: *h as f32 },
        Kind::Separator => Size { width: 1.0, height: 1.0 },
        Kind::MenuBar { menus, .. } => Size { width: menus.iter().map(|m| width_of(Face::Sans, body, &m.title) + pad * 2.0).sum(), height: ch },
        Kind::MenuRow(m) if m.separator => Size { width: 0.0, height: 9.0 },
        Kind::MenuRow(m) => {
            let sw = if m.shortcut.is_empty() { 0.0 } else { width_of(Face::Sans, body * 0.92, &m.shortcut) + 24.0 };
            Size { width: libm::ceilf(28.0 + width_of(Face::Sans, body, &m.label) + sw + 28.0).max(160.0), height: theme.row_height() }
        }
        _ => Size::ZERO,
    };
    Size { width: known.width.unwrap_or(r.width), height: known.height.unwrap_or(r.height) }
}

/// The colour a fill names in this theme.
pub(crate) fn fill_color(t: &Theme, f: Fill) -> Color {
    let p = &t.palette;
    match f { Fill::None => Color::TRANSPARENT, Fill::Window => p.window, Fill::Surface => p.surface, Fill::Field => p.field, Fill::Control => p.control, Fill::Accent => p.accent }
}
