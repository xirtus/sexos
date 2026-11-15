// SPDX-License-Identifier: GPL-3.0-only
//! The node editor's Add menu (Shift+A): a Search entry at the top, then the
//! tree's node categories as submenus, in Blender's order. Typing anything
//! while the menu is open searches every node type (and the operations of
//! Math, Vector Math and Mix, as "Math > Multiply"), like Blender 4's menus.
use crate::draw::{Align, Rect};
use crate::input::{Button, Input, Key};
use crate::theme as t;
use crate::ui::Ui;
use sex_3d::nodes::{edit, enums, registry, Category, NodeTree, TreeKind};

/// One thing the menu can add: a node type, maybe with a property preset.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub label: String,
    /// Node type, or "group:<i>" for a node group.
    pub idname: String,
    pub preset: Option<(String, String)>,
}

impl Entry {
    fn new(label: &str, idname: &str) -> Entry {
        Entry { label: label.into(), idname: idname.into(), preset: None }
    }
    /// The `node.add_node` argument for this entry.
    pub fn arg(&self) -> String {
        match &self.preset {
            Some((p, v)) => format!("{}\t{p}\t{v}", self.idname),
            None => self.idname.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AddMenu {
    pub x: i32,
    pub y: i32,
    /// The search text once typing started.
    pub search: Option<String>,
    /// Open category submenu.
    pub sub: Option<usize>,
    /// Highlighted search result.
    pub hl: usize,
}

pub enum Outcome {
    Open,
    Close,
    Pick(Entry),
}

const ITEM: i32 = 22;
const W: i32 = 190;

fn order(kind: TreeKind) -> &'static [Category] {
    use Category as C;
    match kind {
        TreeKind::Shader => &[C::Input, C::Output, C::Shader, C::Displacement, C::Color, C::Texture, C::Vector, C::Converter, C::Script, C::Group, C::Layout],
        TreeKind::Compositor => &[C::Input, C::Output, C::Color, C::Filter, C::Converter, C::Matte, C::Distort, C::Transform, C::Utilities, C::Group, C::Layout],
        TreeKind::Geometry => &[
            C::Attribute,
            C::Input,
            C::Output,
            C::Geometry,
            C::Curve,
            C::Instances,
            C::Mesh,
            C::Point,
            C::Volume,
            C::SimulationZone,
            C::Material,
            C::Texture,
            C::Utilities,
            C::Group,
            C::Layout,
        ],
    }
}

/// The menu's categories with their entries. `current` is the group being
/// edited (it cannot contain itself; its Group Input/Output are offered).
pub fn categories(kind: TreeKind, groups: &[NodeTree], current: Option<usize>) -> Vec<(String, Vec<Entry>)> {
    let reg = registry::categories(kind);
    let mut cats: Vec<(Category, Vec<Entry>)> = Vec::new();
    for c in order(kind) {
        let mut v: Vec<Entry> = reg.iter().filter(|x| x.0 == *c).flat_map(|x| x.1.iter().map(|t| Entry::new(t.label, t.idname))).collect();
        match c {
            Category::Group => {
                if current.is_some() {
                    for (id, l) in [(edit::GROUP_INPUT, "Group Input"), (edit::GROUP_OUTPUT, "Group Output")] {
                        if !v.iter().any(|e| e.idname == id) {
                            v.push(Entry::new(l, id));
                        }
                    }
                }
                for (i, g) in groups.iter().enumerate() {
                    if g.kind == kind && Some(i) != current && !uses_group(groups, i, current) {
                        v.push(Entry::new(&g.name, &format!("group:{i}")));
                    }
                }
            }
            Category::Layout => {
                for (id, l) in [(edit::FRAME, "Frame"), (edit::REROUTE, "Reroute")] {
                    if !v.iter().any(|e| e.idname == id) {
                        v.push(Entry::new(l, id));
                    }
                }
            }
            _ => {}
        }
        if !v.is_empty() || matches!(c, Category::Group) {
            cats.push((*c, v));
        }
    }
    // Categories the registry has that the list above does not name.
    for (c, types) in reg {
        if !cats.iter().any(|x| x.0 == c) {
            cats.push((c, types.iter().map(|t| Entry::new(t.label, t.idname)).collect()));
        }
    }
    cats.into_iter().map(|(c, v)| (c.label().to_string(), v)).collect()
}

/// Does group `g` (transitively) contain group `target`?
fn uses_group(groups: &[NodeTree], g: usize, target: Option<usize>) -> bool {
    let Some(target) = target else { return false };
    let mut stack = vec![g];
    let mut seen = vec![false; groups.len()];
    while let Some(i) = stack.pop() {
        if i == target {
            return true;
        }
        if i >= groups.len() || std::mem::replace(&mut seen[i], true) {
            continue;
        }
        stack.extend(groups[i].nodes.iter().filter_map(edit::group_of));
    }
    false
}

/// Search results for `q` (every word must match), best first.
pub fn search(cats: &[(String, Vec<Entry>)], q: &str) -> Vec<Entry> {
    let words: Vec<String> = q.to_lowercase().split_whitespace().map(String::from).collect();
    let mut all: Vec<Entry> = cats.iter().flat_map(|c| c.1.iter().cloned()).collect();
    // Operations of the math-like nodes.
    let mut ops = Vec::new();
    for e in &all {
        for prop in ["operation", "blend_type"] {
            let items = enums::items(&e.idname, prop);
            if prop == "blend_type" && e.idname != "ShaderNodeMix" {
                continue;
            }
            for (id, lab) in items {
                ops.push(Entry { label: format!("{} \u{25B8} {lab}", e.label), idname: e.idname.clone(), preset: Some((prop.into(), (*id).into())) });
            }
        }
    }
    all.extend(ops);
    let score = |e: &Entry| -> Option<i32> {
        let l = e.label.to_lowercase();
        if !words.iter().all(|w| l.contains(w.as_str())) {
            return None;
        }
        let first = words.first().map(|w| w.as_str()).unwrap_or("");
        Some(if l.starts_with(first) { 0 } else if l.split_whitespace().any(|x| x.starts_with(first)) { 1 } else { 2 } + if e.preset.is_some() { 3 } else { 0 })
    };
    let mut hits: Vec<(i32, Entry)> = all.into_iter().filter_map(|e| score(&e).map(|s| (s, e))).collect();
    hits.sort_by_key(|h| h.0);
    hits.into_iter().map(|h| h.1).take(14).collect()
}

impl AddMenu {
    pub fn new(x: i32, y: i32, search: bool) -> AddMenu {
        AddMenu { x, y, search: search.then(String::new), sub: None, hl: 0 }
    }

    fn main_rect(&self, n: usize, area: Rect) -> Rect {
        let h = (n as i32 + 1) * ITEM + 7 + 8 + ITEM;
        let x = self.x.min(area.right() - W - 4).max(area.x + 2);
        let y = self.y.min(area.bottom() - h - 2).max(area.y + 2);
        Rect::new(x, y, W, h)
    }

    /// Rows of the main menu: (rect, index) with index 0 = Search and
    /// i + 1 = category i.
    fn rows(&self, n: usize, area: Rect) -> Vec<(Rect, usize)> {
        let r = self.main_rect(n, area);
        let mut y = r.y + ITEM + 4;
        let mut v = vec![(Rect::new(r.x + 4, y, r.w - 8, ITEM), 0)];
        y += ITEM + 7;
        for i in 0..n {
            v.push((Rect::new(r.x + 4, y, r.w - 8, ITEM), i + 1));
            y += ITEM;
        }
        v
    }

    fn sub_rect(&self, cats: &[(String, Vec<Entry>)], area: Rect) -> Option<(Rect, usize)> {
        let i = self.sub?;
        let rows = self.rows(cats.len(), area);
        let (rr, _) = rows.iter().find(|r| r.1 == i + 1)?;
        let n = cats.get(i)?.1.len().max(1) as i32;
        let h = n * ITEM + 8;
        let mut x = rr.right() + 6;
        if x + W > area.right() {
            x = rr.x - W - 6;
        }
        let y = (rr.y - 4).min(area.bottom() - h - 2).max(area.y + 2);
        Some((Rect::new(x, y, W, h), i))
    }

    fn search_rect(&self, n: usize, area: Rect) -> Rect {
        let h = (n.max(1) as i32 + 1) * ITEM + 14;
        let x = self.x.min(area.right() - W - 60).max(area.x + 2);
        let y = self.y.min(area.bottom() - h - 2).max(area.y + 2);
        Rect::new(x, y, W + 60, h)
    }

    /// Handle this frame's event (all of it while open).
    pub fn handle(&mut self, ui: &mut Ui, cats: &[(String, Vec<Entry>)], area: Rect) -> Outcome {
        let Some(input) = ui.input else { return Outcome::Open };
        let pos = input.pos();
        let mut out = Outcome::Open;
        let mut took = true;
        match (&mut self.search, input) {
            (_, Input::Key { key: Key::Esc, .. }) => out = Outcome::Close,
            (Some(q), Input::Key { key, text, mods }) => {
                let results = search(cats, q);
                match key {
                    Key::Enter | Key::NumEnter => {
                        out = match results.get(self.hl) {
                            Some(e) => Outcome::Pick(e.clone()),
                            None => Outcome::Close,
                        }
                    }
                    Key::Backspace => {
                        if q.pop().is_none() {
                            self.search = None;
                        }
                        self.hl = 0;
                    }
                    Key::Down => self.hl = (self.hl + 1).min(results.len().saturating_sub(1)),
                    Key::Up => self.hl = self.hl.saturating_sub(1),
                    _ => {
                        let c = text.or(match key {
                            Key::Char(c) => Some(if mods.shift { c.to_ascii_uppercase() } else { c }),
                            Key::Space => Some(' '),
                            _ => None,
                        });
                        if let Some(c) = c.filter(|c| !c.is_control()) {
                            q.push(c);
                            self.hl = 0;
                        }
                    }
                }
            }
            (None, Input::Key { key, text, mods }) => {
                // Typing in the menu starts a search.
                let c = text.or(match key {
                    Key::Char(c) if !mods.ctrl && !mods.alt => Some(c),
                    _ => None,
                });
                if let Some(c) = c.filter(|c| c.is_alphanumeric()) {
                    self.search = Some(c.to_string());
                    self.hl = 0;
                } else {
                    took = false;
                }
            }
            (Some(q), Input::Move { x, y }) => {
                let results = search(cats, q);
                let r = self.search_rect(results.len(), area);
                for k in 0..results.len() {
                    if Rect::new(r.x + 4, r.y + ITEM + 10 + k as i32 * ITEM, r.w - 8, ITEM).contains(x, y) {
                        self.hl = k;
                    }
                }
            }
            (Some(q), Input::Down { x, y, button: Button::Left, .. }) => {
                let results = search(cats, q);
                let r = self.search_rect(results.len(), area);
                out = Outcome::Close;
                for (k, e) in results.iter().enumerate() {
                    if Rect::new(r.x + 4, r.y + ITEM + 10 + k as i32 * ITEM, r.w - 8, ITEM).contains(x, y) {
                        out = Outcome::Pick(e.clone());
                    }
                }
                if r.contains(x, y) && matches!(out, Outcome::Close) {
                    out = Outcome::Open;
                }
            }
            (None, Input::Move { x, y }) => {
                if let Some((sr, _)) = self.sub_rect(cats, area) {
                    if sr.contains(x, y) {
                        ui.input = None;
                        return Outcome::Open;
                    }
                }
                for (rr, i) in self.rows(cats.len(), area) {
                    if rr.contains(x, y) {
                        self.sub = if i == 0 { None } else { Some(i - 1) };
                    }
                }
            }
            (None, Input::Down { x, y, button: Button::Left, .. }) => {
                out = Outcome::Close;
                if let Some((sr, ci)) = self.sub_rect(cats, area) {
                    if sr.contains(x, y) {
                        out = Outcome::Open;
                        for (k, e) in cats[ci].1.iter().enumerate() {
                            if Rect::new(sr.x + 4, sr.y + 4 + k as i32 * ITEM, sr.w - 8, ITEM).contains(x, y) {
                                out = Outcome::Pick(e.clone());
                            }
                        }
                    }
                }
                if self.main_rect(cats.len(), area).contains(x, y) {
                    out = Outcome::Open;
                    for (rr, i) in self.rows(cats.len(), area) {
                        if rr.contains(x, y) {
                            if i == 0 {
                                self.search = Some(String::new());
                            } else {
                                self.sub = Some(i - 1);
                            }
                        }
                    }
                }
            }
            (_, Input::Down { .. }) => out = Outcome::Close,
            (_, Input::Up { .. }) | (_, Input::Wheel { .. }) => {}
            _ => took = false,
        }
        if took || pos.is_some() {
            ui.input = None;
        }
        out
    }

    pub fn draw(&self, ui: &mut Ui, cats: &[(String, Vec<Entry>)], area: Rect) {
        let mouse = ui.mouse();
        let panel = |ui: &mut Ui, r: Rect| {
            ui.p.round_rect_alpha(Rect::new(r.x + 2, r.y + 3, r.w, r.h), 6, 0x000000, 90);
            ui.p.round_rect(r, 6, t::MENU_BG);
            ui.p.round_frame(r, 6, 0x2a2a2a);
        };
        if let Some(q) = &self.search {
            let results = search(cats, q);
            let r = self.search_rect(results.len(), area);
            panel(ui, r);
            let sr = Rect::new(r.x + 6, r.y + 5, r.w - 12, ITEM);
            ui.p.round_rect(sr, 4, t::BUTTON_DARK);
            magnifier(ui, sr.x + 10, sr.y + ITEM / 2);
            ui.p.label(Rect::new(sr.x + 20, sr.y, sr.w - 24, sr.h), &format!("{q}|"), t::TEXT_HI, Align::Left);
            if results.is_empty() {
                ui.p.label(Rect::new(r.x + 12, r.y + ITEM + 10, r.w - 24, ITEM), "No results found", t::TEXT_DIM, Align::Left);
            }
            for (k, e) in results.iter().enumerate() {
                let ir = Rect::new(r.x + 4, r.y + ITEM + 10 + k as i32 * ITEM, r.w - 8, ITEM);
                if k == self.hl {
                    ui.p.round_rect(ir, 4, t::MENU_ITEM_HOVER);
                }
                ui.p.label(Rect::new(ir.x + 8, ir.y, ir.w - 16, ir.h), &e.label, if k == self.hl { t::TEXT_HI } else { t::TEXT }, Align::Left);
            }
            return;
        }
        let r = self.main_rect(cats.len(), area);
        panel(ui, r);
        ui.p.label(Rect::new(r.x + 12, r.y + 2, r.w - 24, ITEM), "Add", t::TEXT_DIM, Align::Left);
        ui.p.fill(Rect::new(r.x + 6, r.y + ITEM + 1, r.w - 12, 1), t::MENU_SEP);
        for (rr, i) in self.rows(cats.len(), area) {
            let hover = rr.contains(mouse.0, mouse.1) || (i > 0 && self.sub == Some(i - 1));
            if hover {
                ui.p.round_rect(rr, 4, t::MENU_ITEM_HOVER);
            }
            if i == 0 {
                magnifier(ui, rr.x + 12, rr.y + ITEM / 2);
                ui.p.label(Rect::new(rr.x + 24, rr.y, rr.w - 32, rr.h), "Search...", if hover { t::TEXT_HI } else { t::TEXT }, Align::Left);
                ui.p.fill(Rect::new(rr.x + 2, rr.bottom() + 3, rr.w - 4, 1), t::MENU_SEP);
            } else {
                ui.p.label(Rect::new(rr.x + 24, rr.y, rr.w - 32, rr.h), &cats[i - 1].0, if hover { t::TEXT_HI } else { t::TEXT }, Align::Left);
                ui.p.arrow((rr.right() - 10) as f32, (rr.y + rr.h / 2) as f32, 8.0, false, t::TEXT_DIM);
            }
        }
        if let Some((sr, ci)) = self.sub_rect(cats, area) {
            panel(ui, sr);
            if cats[ci].1.is_empty() {
                ui.p.label(Rect::new(sr.x + 12, sr.y + 4, sr.w - 24, ITEM), "No node groups", t::TEXT_DIM, Align::Left);
            }
            for (k, e) in cats[ci].1.iter().enumerate() {
                let ir = Rect::new(sr.x + 4, sr.y + 4 + k as i32 * ITEM, sr.w - 8, ITEM);
                let hover = ir.contains(mouse.0, mouse.1);
                if hover {
                    ui.p.round_rect(ir, 4, t::MENU_ITEM_HOVER);
                }
                ui.p.label(Rect::new(ir.x + 12, ir.y, ir.w - 20, ir.h), &e.label, if hover { t::TEXT_HI } else { t::TEXT }, Align::Left);
            }
        }
    }

    /// Where a category's submenu entry `k` is (for tests and scripted input).
    pub fn entry_rect(&self, cats: &[(String, Vec<Entry>)], area: Rect, k: usize) -> Option<Rect> {
        let (sr, _) = self.sub_rect(cats, area)?;
        Some(Rect::new(sr.x + 4, sr.y + 4 + k as i32 * ITEM, sr.w - 8, ITEM))
    }
    /// Where main-menu row `i` is (0 = Search, 1.. = categories).
    pub fn row_rect(&self, cats: &[(String, Vec<Entry>)], area: Rect, i: usize) -> Option<Rect> {
        self.rows(cats.len(), area).into_iter().find(|r| r.1 == i).map(|r| r.0)
    }
}

/// Our own magnifying-glass icon.
fn magnifier(ui: &mut Ui, x: i32, y: i32) {
    ui.p.ring(x as f32 - 1.0, y as f32 - 1.0, 3.5, t::TEXT_DIM);
    ui.p.line(x as f32 + 1.5, y as f32 + 1.5, x as f32 + 4.5, y as f32 + 4.5, t::TEXT_DIM);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_and_search() {
        let cats = categories(TreeKind::Shader, &[], None);
        let names: Vec<&str> = cats.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names[0], "Input");
        assert!(names.contains(&"Layout") && names.contains(&"Group"));
        let r = search(&cats, "princ");
        assert_eq!(r[0].idname, "ShaderNodeBsdfPrincipled");
        let r = search(&cats, "multiply");
        assert!(r.iter().any(|e| e.idname == "ShaderNodeMath" && e.preset == Some(("operation".into(), "MULTIPLY".into()))));
    }
}
