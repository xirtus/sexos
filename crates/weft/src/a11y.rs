// SPDX-License-Identifier: GPL-3.0-only
//! The accessibility tree: what a screen reader (Whisper) reads and does.
//!
//! Weft describes the window as AccessKit nodes. Node ids are the widget's index + 1,
//! so the window's root is node 1; menus, dialogs, the palette, toasts and tooltips
//! hang under it. List rows, tabs and menu-bar titles are nodes of their own with ids
//! `(widget + 1) << 32 | kind << 28 | (index + 1)`, made only for the rows on screen.
//! The first update holds the whole tree; later ones only what changed, and always
//! the focus. [`encode`] turns an update into bytes for the `sex_pdx::a11y` transport
//! and [`decode`] reads them back.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Role, Toggled, TreeId, TreeInfo, TreeUpdate};

use crate::input::Key;
use crate::ui::{tab_strip_height, LayerKind, Ui};
use crate::widget::*;
use crate::Response;

const ROW: u64 = 1;
const TAB: u64 = 2;
const BAR: u64 = 3;

/// The node id of a widget.
pub fn node_id(id: Id) -> NodeId { NodeId(id.0 as u64 + 1) }
fn part_id(id: Id, kind: u64, index: usize) -> NodeId { NodeId(((id.0 as u64 + 1) << 32) | kind << 28 | (index as u64 + 1)) }
/// The widget and part (kind, index) a node id names.
fn parse_id(n: NodeId) -> (Id, Option<(u64, usize)>) {
    let v = n.0;
    if v >> 32 == 0 { return (Id((v as u32).wrapping_sub(1)), None); }
    (Id(((v >> 32) - 1) as u32), Some(((v >> 28) & 0xF, ((v & 0x0FFF_FFFF) - 1) as usize)))
}

impl Ui {
    /// The whole tree, as a screen reader needs when it starts.
    pub fn a11y_tree(&mut self) -> TreeUpdate {
        self.a11y_sent.clear();
        self.a11y_update().expect("a full tree")
    }

    /// What changed since the last update, or `None` if nothing did.
    pub fn a11y_update(&mut self) -> Option<TreeUpdate> {
        self.layout();
        let first = self.a11y_sent.is_empty();
        if !first && !self.a11y_dirty { return None; }
        self.a11y_dirty = false;
        let all = self.a11y_nodes();
        let mut nodes = Vec::new();
        let mut keep = BTreeMap::new();
        for (id, node) in all {
            if self.a11y_sent.get(&id.0) != Some(&node) { nodes.push((id, node.clone())); }
            keep.insert(id.0, node);
        }
        self.a11y_sent = keep;
        let tree = first.then(|| TreeInfo { root: NodeId(1), toolkit_name: Some("Weft".into()), toolkit_version: Some(env!("CARGO_PKG_VERSION").into()) });
        Some(TreeUpdate { nodes, tree, tree_id: TreeId::ROOT, focus: self.a11y_focus() })
    }

    fn a11y_focus(&self) -> NodeId {
        if let Some(m) = self.menus.last() { if let Some(h) = m.hot { return node_id(m.rows[h]); } }
        match self.focus.filter(|f| self.exists(*f)) {
            Some(f) => match &self.node(f).kind {
                Kind::List(l) if l.model.len() > 0 => part_id(f, ROW, l.cursor),
                Kind::Tabs(t) if !t.tabs.is_empty() => part_id(f, TAB, t.active),
                _ => node_id(f),
            },
            None => NodeId(1),
        }
    }

    fn a11y_nodes(&self) -> Vec<(NodeId, Node)> {
        let mut out = Vec::new();
        let main = self.root();
        self.a11y_walk(main, &mut out);
        // Overlays hang under the window.
        let extra: Vec<NodeId> = self.layers.iter().skip(1).map(|l| node_id(l.root)).collect();
        for l in self.layers.iter().skip(1) { self.a11y_walk(l.root, &mut out); }
        if let Some((_, root)) = out.iter_mut().find(|(id, _)| *id == NodeId(1)) {
            let mut kids: Vec<NodeId> = root.children().to_vec();
            kids.extend(extra);
            root.set_children(kids);
            root.set_role(Role::Window);
        }
        for l in self.layers.iter().skip(1) {
            if let Some((_, n)) = out.iter_mut().find(|(id, _)| *id == node_id(l.root)) {
                match l.kind {
                    LayerKind::Menu => n.set_role(Role::Menu),
                    LayerKind::Dialog => { n.set_role(Role::Dialog); n.set_modal(); }
                    LayerKind::Palette => { n.set_role(Role::Dialog); n.set_modal(); n.set_label(self.tr("weft-palette")); }
                    LayerKind::Toast => n.set_role(Role::Status),
                    LayerKind::Tooltip => n.set_role(Role::Tooltip),
                    LayerKind::Main => {}
                }
            }
        }
        out
    }

    fn bounds(&self, r: crate::geom::Rect) -> accesskit::Rect {
        let s = self.theme.scale as f64;
        accesskit::Rect { x0: r.x as f64 * s, y0: r.y as f64 * s, x1: r.right() as f64 * s, y1: r.bottom() as f64 * s }
    }

    fn a11y_walk(&self, id: Id, out: &mut Vec<(NodeId, Node)>) {
        let n = self.node(id);
        if !n.visible || (n.rect.is_empty() && !self.layout_dirty) { return; }
        let mut node = Node::new(Role::GenericContainer);
        node.set_bounds(self.bounds(n.rect));
        if !n.enabled { node.set_disabled(); }
        if let Some(t) = &n.tooltip { node.set_description(t.as_str()); }
        let mut kids: Vec<NodeId> = Vec::new();
        let mut walk_children = true;
        match &n.kind {
            Kind::Panel { .. } => {}
            Kind::Label { text, .. } => { node.set_role(Role::Label); node.set_value(text.as_str()); }
            Kind::Button { text, style, .. } => {
                node.set_role(if *style == ButtonStyle::Primary { Role::DefaultButton } else { Role::Button });
                if !text.is_empty() { node.set_label(text.as_str()); }
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
            }
            Kind::Field(f) => {
                node.set_role(if f.password { Role::PasswordInput } else { Role::TextInput });
                if !f.password { node.set_value(f.edit.text.as_str()); }
                if !f.placeholder.is_empty() { node.set_placeholder(f.placeholder.as_str()); }
                node.add_action(Action::Focus);
                node.add_action(Action::SetValue);
            }
            Kind::Check { text, on, switch } => {
                node.set_role(if *switch { Role::Switch } else { Role::CheckBox });
                node.set_label(text.as_str());
                node.set_toggled(if *on { Toggled::True } else { Toggled::False });
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
            }
            Kind::Radio { text, on, .. } => {
                node.set_role(Role::RadioButton);
                node.set_label(text.as_str());
                node.set_toggled(if *on { Toggled::True } else { Toggled::False });
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
            }
            Kind::Slider { value, min, max, step } => {
                node.set_role(Role::Slider);
                node.set_numeric_value(*value as f64);
                node.set_min_numeric_value(*min as f64);
                node.set_max_numeric_value(*max as f64);
                if *step > 0.0 { node.set_numeric_value_step(*step as f64); }
                for a in [Action::Focus, Action::Increment, Action::Decrement, Action::SetValue] { node.add_action(a); }
            }
            Kind::Progress { value } => {
                node.set_role(Role::ProgressIndicator);
                if let Some(v) = value { node.set_numeric_value(*v as f64); node.set_min_numeric_value(0.0); node.set_max_numeric_value(1.0); }
            }
            Kind::Scroll { y, content } => {
                node.set_role(Role::ScrollView);
                node.set_scroll_y(*y as f64);
                node.set_scroll_y_max((*content - n.rect.h).max(0.0) as f64);
            }
            Kind::List(l) => {
                let tree = (0..l.model.len().min(64)).any(|r| l.model.expandable(r).is_some() || l.model.depth(r) > 0);
                node.set_role(if tree { Role::Tree } else if l.columns.is_empty() { Role::ListBox } else { Role::Table });
                if l.multi { node.set_multiselectable(); }
                node.add_action(Action::Focus);
                let g = self.list_geom(id);
                let len = l.model.len();
                let first = (l.scroll / g.row_h) as usize;
                let count = (g.rows.h / g.row_h) as usize + 1;
                let mut cell = String::new();
                for row in first..(first + count).min(len) {
                    let mut rn = Node::new(if tree { Role::TreeItem } else if l.columns.is_empty() { Role::ListBoxOption } else { Role::Row });
                    let mut label = String::new();
                    for c in 0..l.columns.len().max(1) {
                        cell.clear();
                        l.model.cell(row, c, &mut cell);
                        if cell.is_empty() { continue; }
                        if !label.is_empty() { label.push_str(", "); }
                        label.push_str(&cell);
                    }
                    rn.set_label(label);
                    rn.set_selected(l.selection.contains(row));
                    rn.set_position_in_set(row + 1);
                    rn.set_size_of_set(len);
                    if tree { rn.set_level(l.model.depth(row) as usize + 1); }
                    if let Some(open) = l.model.expandable(row) { rn.set_expanded(open); rn.add_action(if open { Action::Collapse } else { Action::Expand }); }
                    let y = g.rows.y + row as f32 * g.row_h - l.scroll;
                    rn.set_bounds(self.bounds(crate::geom::Rect::new(n.rect.x, y, n.rect.w, g.row_h)));
                    rn.add_action(Action::Click);
                    rn.add_action(Action::ScrollIntoView);
                    let rid = part_id(id, ROW, row);
                    kids.push(rid);
                    out.push((rid, rn));
                }
            }
            Kind::Tabs(t) => {
                node.set_role(Role::TabList);
                node.add_action(Action::Focus);
                let sh = tab_strip_height(&self.theme);
                for (i, tab) in t.tabs.iter().enumerate() {
                    let mut tn = Node::new(Role::Tab);
                    let mut label = tab.title.clone();
                    if tab.modified { label.push_str(" •"); }
                    tn.set_label(label);
                    tn.set_selected(i == t.active);
                    tn.set_position_in_set(i + 1);
                    tn.set_size_of_set(t.tabs.len());
                    if let Some(tip) = &tab.tooltip { tn.set_description(tip.as_str()); }
                    if let Some(&(x, w)) = t.spans.get(i) { tn.set_bounds(self.bounds(crate::geom::Rect::new(n.rect.x + x - t.scroll, n.rect.y, w, sh))); }
                    tn.add_action(Action::Click);
                    let tid = part_id(id, TAB, i);
                    kids.push(tid);
                    out.push((tid, tn));
                }
                // The active page is the tab panel.
                if let Some(page) = n.children.get(t.active) {
                    let before = out.len();
                    self.a11y_walk(*page, out);
                    if out.len() > before {
                        if let Some((_, p)) = out.iter_mut().find(|(i, _)| *i == node_id(*page)) {
                            p.set_role(Role::TabPanel);
                            if let Some(tab) = t.tabs.get(t.active) { p.set_label(tab.title.as_str()); }
                        }
                        kids.push(node_id(*page));
                    }
                }
                walk_children = false;
            }
            Kind::Split { .. } => {}
            Kind::Icon { .. } | Kind::Image { .. } => { node.set_role(Role::Image); if n.label.is_none() { node.set_hidden(); } }
            Kind::Separator => return,
            Kind::MenuBar { menus, open } => {
                node.set_role(Role::MenuBar);
                let titles = self.bar_titles(id);
                for (i, m) in menus.iter().enumerate() {
                    let mut mn = Node::new(Role::MenuItem);
                    mn.set_label(m.title.as_str());
                    mn.set_expanded(*open == Some(i));
                    if let Some(r) = titles.get(i) { mn.set_bounds(self.bounds(*r)); }
                    mn.add_action(Action::Click);
                    let mid = part_id(id, BAR, i);
                    kids.push(mid);
                    out.push((mid, mn));
                }
            }
            Kind::MenuRow(m) => {
                if m.separator { return; }
                node.set_role(match m.check { Some(_) => Role::MenuItemCheckBox, None => Role::MenuItem });
                node.set_label(m.label.as_str());
                if let Some(c) = m.check { node.set_toggled(if c { Toggled::True } else { Toggled::False }); }
                if !m.shortcut.is_empty() { node.set_keyboard_shortcut(m.shortcut.as_str()); }
                if m.submenu { node.set_expanded(self.menus.len() > m.menu + 1 && self.menus[m.menu].hot == Some(m.index)); }
                node.add_action(Action::Click);
            }
        }
        if let Some(l) = &n.label { node.set_label(l.as_str()); }
        let me = node_id(id);
        let at = out.len();
        out.push((me, Node::default()));
        if walk_children {
            for c in &n.children {
                let before = out.len();
                self.a11y_walk(*c, out);
                if out.len() > before { kids.push(node_id(*c)); }
            }
        }
        node.set_children(kids);
        out[at].1 = node;
    }

    /// Does what a screen reader asks: focus, click, set a value, step a slider,
    /// open or close a tree row, scroll a row into view.
    pub fn a11y_action(&mut self, req: &ActionRequest) -> Vec<Response> {
        self.layout();
        let (id, part) = parse_id(req.target_node);
        if !self.exists(id) { return Vec::new(); }
        match (part, req.action) {
            (None, Action::Focus) => { self.focus_visible = true; self.focus(Some(id)); }
            (None, Action::Click) => {
                if let Kind::MenuRow(_) = self.node(id).kind { self.menu_activate_row(id); } else { self.activate(id); }
            }
            (None, Action::Increment | Action::Decrement) => {
                self.focus(Some(id));
                let k = Key::new(if req.action == Action::Increment { crate::input::code::RIGHT } else { crate::input::code::LEFT });
                return self.handle(crate::Event::Key(k));
            }
            (None, Action::SetValue) => match (&self.node(id).kind, &req.data) {
                (Kind::Field(_), Some(ActionData::Value(v))) => { let v = String::from(&**v); self.set_text(id, &v); self.emit(Response::Changed(id)); }
                (Kind::Slider { .. }, Some(ActionData::NumericValue(v))) => { self.set_value(id, *v as f32); self.emit(Response::Changed(id)); }
                _ => {}
            },
            (Some((ROW, row)), a) => {
                let Kind::List(l) = &mut self.node_mut(id).kind else { return Vec::new() };
                if row >= l.model.len() { return Vec::new(); }
                match a {
                    Action::Click | Action::Focus => {
                        l.cursor = row; l.anchor = row; l.selection.set(row);
                        self.focus(Some(id));
                        self.reveal_cursor(id);
                        self.emit(Response::Selected(id));
                    }
                    Action::Expand => self.emit(Response::Expand(id, row, true)),
                    Action::Collapse => self.emit(Response::Expand(id, row, false)),
                    Action::ScrollIntoView => { l.cursor = row; self.reveal_cursor(id); }
                    _ => {}
                }
            }
            (Some((TAB, i)), Action::Click | Action::Focus) => { self.select_tab(id, i); self.focus(Some(id)); }
            (Some((BAR, i)), Action::Click) => self.open_bar_menu(id, i),
            _ => {}
        }
        self.damage_node(id);
        self.layout();
        core::mem::take(&mut self.out)
    }
}

// ---- the wire format ---------------------------------------------------------------

/// Bytes on the wire start with this.
pub const MAGIC: &[u8; 4] = b"WA11";
pub const VERSION: u8 = 1;

/// One node as the wire carries it: what a screen reader speaks and acts on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WireNode {
    pub id: u64,
    /// [`accesskit::Role`] as its `u8` value.
    pub role: u8,
    pub label: String,
    pub value: String,
    pub description: String,
    pub placeholder: String,
    pub shortcut: String,
    /// x0, y0, x1, y1 in window pixels.
    pub bounds: Option<[f32; 4]>,
    pub disabled: bool,
    pub hidden: bool,
    pub modal: bool,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    /// 0 off, 1 on, 2 mixed.
    pub toggled: Option<u8>,
    /// value, min, max, step (NaN when absent).
    pub numeric: Option<[f64; 4]>,
    pub position: u32,
    pub set_size: u32,
    pub level: u32,
    /// Bit per [`ACTIONS`] entry.
    pub actions: u32,
    pub children: Vec<u64>,
}

/// The actions the wire carries, in bit order.
pub const ACTIONS: [Action; 9] = [Action::Click, Action::Focus, Action::Increment, Action::Decrement, Action::SetValue, Action::Expand, Action::Collapse, Action::ScrollIntoView, Action::ShowContextMenu];

/// A decoded update.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wire {
    /// Set on a full tree: the root node.
    pub root: Option<u64>,
    pub focus: u64,
    pub nodes: Vec<WireNode>,
}

fn put_str(out: &mut Vec<u8>, s: &str) { out.extend_from_slice(&(s.len() as u32).to_le_bytes()); out.extend_from_slice(s.as_bytes()); }

/// An update as bytes: magic, version, flags (bit 0: full tree), root?, focus, node
/// count, then each node (see [`WireNode`]), all little-endian.
pub fn encode(u: &TreeUpdate) -> Vec<u8> {
    let mut o = Vec::with_capacity(64 + u.nodes.len() * 64);
    o.extend_from_slice(MAGIC);
    o.push(VERSION);
    o.push(u.tree.is_some() as u8);
    if let Some(t) = &u.tree { o.extend_from_slice(&t.root.0.to_le_bytes()); }
    o.extend_from_slice(&u.focus.0.to_le_bytes());
    o.extend_from_slice(&(u.nodes.len() as u32).to_le_bytes());
    for (id, n) in &u.nodes {
        o.extend_from_slice(&id.0.to_le_bytes());
        o.push(n.role() as u8);
        let toggled = n.toggled().map(|t| match t { Toggled::False => 0u8, Toggled::True => 1, Toggled::Mixed => 2 });
        let num = n.numeric_value().is_some() || n.min_numeric_value().is_some();
        let flags: u32 = n.is_disabled() as u32 | (n.is_hidden() as u32) << 1 | (n.is_modal() as u32) << 2
            | (n.is_selected().is_some() as u32) << 3 | (n.is_selected() == Some(true)) as u32 * 16
            | (n.is_expanded().is_some() as u32) << 5 | ((n.is_expanded() == Some(true)) as u32) << 6
            | (toggled.is_some() as u32) << 7 | (toggled.unwrap_or(0) as u32) << 8
            | (n.bounds().is_some() as u32) << 10 | (num as u32) << 11;
        o.extend_from_slice(&flags.to_le_bytes());
        if let Some(b) = n.bounds() { for v in [b.x0, b.y0, b.x1, b.y1] { o.extend_from_slice(&(v as f32).to_le_bytes()); } }
        for s in [n.label(), n.value(), n.description(), n.placeholder(), n.keyboard_shortcut()] { put_str(&mut o, s.unwrap_or("")); }
        if num {
            for v in [n.numeric_value(), n.min_numeric_value(), n.max_numeric_value(), n.numeric_value_step()] { o.extend_from_slice(&v.unwrap_or(f64::NAN).to_le_bytes()); }
        }
        for v in [n.position_in_set(), n.size_of_set(), n.level()] { o.extend_from_slice(&(v.unwrap_or(0) as u32).to_le_bytes()); }
        let actions: u32 = ACTIONS.iter().enumerate().map(|(i, a)| (n.supports_action(*a) as u32) << i).sum();
        o.extend_from_slice(&actions.to_le_bytes());
        o.extend_from_slice(&(n.children().len() as u32).to_le_bytes());
        for c in n.children() { o.extend_from_slice(&c.0.to_le_bytes()); }
    }
    o
}

struct Reader<'a> { b: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> { let s = self.b.get(self.at..self.at + n)?; self.at += n; Some(s) }
    fn u8(&mut self) -> Option<u8> { self.take(1).map(|s| s[0]) }
    fn u32(&mut self) -> Option<u32> { self.take(4).map(|s| u32::from_le_bytes(s.try_into().unwrap())) }
    fn u64(&mut self) -> Option<u64> { self.take(8).map(|s| u64::from_le_bytes(s.try_into().unwrap())) }
    fn f32(&mut self) -> Option<f32> { self.take(4).map(|s| f32::from_le_bytes(s.try_into().unwrap())) }
    fn f64(&mut self) -> Option<f64> { self.take(8).map(|s| f64::from_le_bytes(s.try_into().unwrap())) }
    fn str(&mut self) -> Option<String> { let n = self.u32()? as usize; core::str::from_utf8(self.take(n)?).ok().map(String::from) }
}

/// Reads bytes [`encode`] made; `None` if they are not a whole, valid update.
pub fn decode(bytes: &[u8]) -> Option<Wire> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.take(4)? != MAGIC || r.u8()? != VERSION { return None; }
    let full = r.u8()? & 1 != 0;
    let root = if full { Some(r.u64()?) } else { None };
    let focus = r.u64()?;
    let count = r.u32()? as usize;
    let mut nodes = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let mut n = WireNode { id: r.u64()?, role: r.u8()?, ..Default::default() };
        let f = r.u32()?;
        n.disabled = f & 1 != 0;
        n.hidden = f & 2 != 0;
        n.modal = f & 4 != 0;
        if f & 8 != 0 { n.selected = Some(f & 16 != 0); }
        if f & 32 != 0 { n.expanded = Some(f & 64 != 0); }
        if f & 128 != 0 { n.toggled = Some(((f >> 8) & 3) as u8); }
        if f & (1 << 10) != 0 { n.bounds = Some([r.f32()?, r.f32()?, r.f32()?, r.f32()?]); }
        n.label = r.str()?;
        n.value = r.str()?;
        n.description = r.str()?;
        n.placeholder = r.str()?;
        n.shortcut = r.str()?;
        if f & (1 << 11) != 0 { n.numeric = Some([r.f64()?, r.f64()?, r.f64()?, r.f64()?]); }
        n.position = r.u32()?;
        n.set_size = r.u32()?;
        n.level = r.u32()?;
        n.actions = r.u32()?;
        let kids = r.u32()? as usize;
        n.children = (0..kids).map(|_| r.u64()).collect::<Option<Vec<_>>>()?;
        nodes.push(n);
    }
    (r.at == bytes.len()).then_some(Wire { root, focus, nodes })
}

/// The tree as indented text (one node a line, focus marked), for tests and logs.
pub fn describe(u: &TreeUpdate) -> String {
    let map: BTreeMap<u64, &Node> = u.nodes.iter().map(|(i, n)| (i.0, n)).collect();
    let mut out = String::new();
    let root = u.tree.as_ref().map_or(1, |t| t.root.0);
    let mut stack = vec![(root, 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(n) = map.get(&id) else { continue };
        if n.is_hidden() { continue; }
        for _ in 0..depth { out.push_str("  "); }
        out.push_str(&alloc::format!("{:?}", n.role()));
        if let Some(l) = n.label() { out.push_str(&alloc::format!(" {:?}", l)); }
        if let Some(v) = n.value() { out.push_str(&alloc::format!(" = {:?}", v)); }
        if let Some(t) = n.toggled() { out.push_str(if t == Toggled::True { " [on]" } else { " [off]" }); }
        if let Some(v) = n.numeric_value() { out.push_str(&alloc::format!(" = {}", v)); }
        if n.is_selected() == Some(true) { out.push_str(" [selected]"); }
        if let Some(e) = n.is_expanded() { out.push_str(if e { " [open]" } else { " [closed]" }); }
        if n.is_disabled() { out.push_str(" [disabled]"); }
        if u.focus.0 == id { out.push_str(" [focus]"); }
        out.push('\n');
        for c in n.children().iter().rev() { stack.push((c.0, depth + 1)); }
    }
    out
}

