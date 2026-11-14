// SPDX-License-Identifier: GPL-3.0-only
//! Painting: every widget kind, focus rings, scroll bars, and the layers over the
//! window. Only the damaged part of the window is repainted.
use alloc::string::String;
use alloc::vec::Vec;

use crate::geom::Rect;
use crate::icons::Icons;
use crate::paint::Painter;
use crate::text::{Face, Fonts};
use crate::theme::{Color, Mode, Theme};
use crate::ui::{fill_color, tab_strip_height, text_px, Drag, Layer, LayerKind, Node, OpenMenu, Ui};
use crate::widget::*;

/// Everything painting reads, borrowed apart from the fonts it caches glyphs in.
struct Draw<'a> {
    nodes: &'a [Option<Node>],
    fonts: &'a mut Fonts,
    icons: &'a Icons,
    theme: &'a Theme,
    focus: Option<Id>,
    focus_visible: bool,
    hover: Option<Id>,
    pressed: Option<Id>,
    drag: Drag,
    now: u64,
    menus: &'a [OpenMenu],
    /// List geometry and scroll bars computed before borrowing apart.
    lists: &'a [(Id, ListBits)],
    bars: &'a [(Id, Rect)],
    dividers: &'a [(Id, Rect)],
    tab_new: &'a [(Id, Rect)],
    titles: &'a [(Id, Vec<Rect>)],
    fields: &'a [(Id, FieldBits)],
}

#[derive(Clone, Copy)]
struct ListBits { header: Rect, rows: Rect, row_h: f32 }

#[derive(Clone)]
struct FieldBits { shown: String, sel: (f32, f32), cursor: f32 }

impl Ui {
    /// Paints what changed into `pixels` (`width * height` words of the window, as
    /// [`Ui::size`]). Returns the device-pixel rectangle painted, if any.
    pub fn paint(&mut self, pixels: &mut [u32]) -> Option<Rect> {
        self.layout();
        if !self.full_damage && self.damage.is_empty() { return None; }
        let (w, h) = (self.size.0 as usize, self.size.1 as usize);
        if pixels.len() < w * h { return None; }
        let s = self.theme.scale;
        let whole = Rect::new(0.0, 0.0, w as f32, h as f32);
        let damage = if self.full_damage { whole } else { self.damage.scale(s).round_out().intersect(&whole) };
        self.full_damage = false;
        self.damage = Rect::ZERO;
        if damage.is_empty() { return None; }

        // Geometry that needs `&mut self` or helper methods, worked out first.
        let mut lists = Vec::new();
        let mut bars = Vec::new();
        let mut dividers = Vec::new();
        let mut tab_new = Vec::new();
        let mut titles = Vec::new();
        let mut fields = Vec::new();
        for i in 0..self.nodes.len() {
            let id = Id(i as u32);
            let Some(n) = self.nodes[i].as_ref() else { continue };
            if n.rect.is_empty() { continue; }
            match &n.kind {
                Kind::List(_) => {
                    let g = self.list_geom(id);
                    lists.push((id, ListBits { header: g.header, rows: g.rows, row_h: g.row_h }));
                }
                Kind::Tabs(_) => { if let (_, Some(nb)) = self.tab_geom(id) { tab_new.push((id, nb)); } }
                Kind::Split { .. } => dividers.push((id, self.split_divider(id))),
                Kind::MenuBar { .. } => titles.push((id, self.bar_titles(id))),
                Kind::Field(f) => {
                    let (a, b) = f.edit.selection();
                    fields.push((id, FieldBits { shown: Ui::field_shown(f), sel: (self.field_x(f, a), self.field_x(f, b)), cursor: self.field_x(f, f.edit.cursor) }));
                }
                _ => {}
            }
            if let Some((thumb, ..)) = self.scrollbar(id) { bars.push((id, thumb)); }
        }

        let layers: Vec<(Id, LayerKind)> = self.layers.iter().map(|l: &Layer| (l.root, l.kind)).collect();
        let Ui { nodes, fonts, icons, theme, focus, focus_visible, hover, pressed, drag, now, menus, .. } = self;
        let mut d = Draw {
            nodes, fonts, icons, theme, focus: *focus, focus_visible: *focus_visible, hover: *hover, pressed: *pressed, drag: *drag, now: *now,
            menus, lists: &lists, bars: &bars, dividers: &dividers, tab_new: &tab_new, titles: &titles, fields: &fields,
        };
        let mut p = Painter::new(pixels, w, h, s);
        p.push_clip_device(damage);
        let (lw, lh) = (w as f32 / s, h as f32 / s);
        for (root, kind) in layers {
            let r = d.node(root).rect;
            match kind {
                LayerKind::Main => {}
                LayerKind::Dialog | LayerKind::Palette => {
                    p.fill_rect(Rect::new(0.0, 0.0, lw, lh), d.theme.palette.scrim);
                    p.shadow(r, d.theme.radius + 2.0, 24.0, d.theme.palette.shadow);
                }
                _ => p.shadow(r, d.theme.radius + 2.0, 10.0, d.theme.palette.shadow),
            }
            d.node_tree(&mut p, root);
        }
        Some(damage)
    }
}

impl<'a> Draw<'a> {
    fn node(&self, id: Id) -> &'a Node { self.nodes[id.0 as usize].as_ref().expect("live widget") }

    fn width(&self, face: Face, size: f32, text: &str) -> f32 {
        let s = self.theme.scale;
        self.fonts.measure(face, size * s, text) / s
    }

    /// Draws one line of text vertically centred in `y..y+h`; returns its advance.
    fn text(&mut self, p: &mut Painter, face: Face, size: f32, x: f32, y: f32, h: f32, text: &str, c: Color) -> f32 {
        let s = self.theme.scale;
        let (a, de) = self.fonts.metrics(face, size * s);
        let base = y + (h - (a + de) / s) / 2.0 + a / s;
        p.text(self.fonts, face, size, x, libm::roundf(base * s) / s, text, c)
    }

    fn icon(&self, p: &mut Painter, name: &str, r: Rect, c: Color) { self.icons.draw(p, name, r, c); }

    fn node_tree(&mut self, p: &mut Painter, id: Id) {
        let n = self.node(id);
        if !n.visible || n.rect.is_empty() || !p.clip_visible(n.rect.inset(-6.0)) { return; }
        self.widget(p, id, n);
        let clip = matches!(n.kind, Kind::Scroll { .. } | Kind::Tabs(_) | Kind::Split { .. }) || matches!(n.kind, Kind::Panel { radius, .. } if radius > 0.0);
        if clip {
            let mut c = n.rect;
            if let Kind::Tabs(_) = n.kind { let sh = tab_strip_height(self.theme); c = Rect::new(c.x, c.y + sh, c.w, (c.h - sh).max(0.0)); }
            p.push_clip(c);
        }
        for ch in &n.children { self.node_tree(p, *ch); }
        if clip { p.pop_clip(); }
        if let Kind::Scroll { .. } = n.kind { self.scrollbar(p, id); }
        self.focus_ring(p, id, n);
    }

    fn focus_ring(&mut self, p: &mut Painter, id: Id, n: &Node) {
        if !self.focus_visible || self.focus != Some(id) { return; }
        if matches!(n.kind, Kind::Field(_) | Kind::MenuRow(_) | Kind::List(_)) { return; }
        let r = match &n.kind {
            // A tab strip shows focus on its active tab.
            Kind::Tabs(t) => match t.spans.get(t.active) { Some((x, w)) => Rect::new(n.rect.x + x - t.scroll, n.rect.y, *w, tab_strip_height(self.theme)).inset(2.0), None => return },
            Kind::Check { .. } | Kind::Radio { .. } => {
                let w = self.check_width(n) + 8.0 + self.width(Face::Sans, self.theme.font_size, label_of(&n.kind));
                Rect::new(n.rect.x, n.rect.y, w.min(n.rect.w), n.rect.h)
            }
            _ => n.rect,
        };
        p.stroke_round(r.inset(-2.0), self.theme.radius + 2.0, 2.0, self.theme.palette.focus);
    }

    fn check_width(&self, n: &Node) -> f32 { if matches!(n.kind, Kind::Check { switch: true, .. }) { 36.0 } else { 18.0 } }

    fn scrollbar(&mut self, p: &mut Painter, id: Id) {
        if let Some((_, thumb)) = self.bars.iter().find(|b| b.0 == id) {
            let c = self.theme.palette.text_dim.with_alpha(if self.hover == Some(id) || matches!(self.drag, Drag::ScrollBar { id: x, .. } if x == id) { 0xA0 } else { 0x60 });
            p.fill_round(*thumb, thumb.w / 2.0, c);
        }
    }

    fn widget(&mut self, p: &mut Painter, id: Id, n: &'a Node) {
        let t = self.theme;
        let pal = &t.palette;
        let r = n.rect;
        let hot = self.hover == Some(id);
        let down = self.pressed == Some(id) && hot;
        let fg = if n.enabled { pal.text } else { pal.text_disabled };
        let body = t.font_size;
        match &n.kind {
            Kind::Panel { fill, border, radius } => {
                let c = fill_color(t, *fill);
                if c.a() > 0 { if *radius > 0.0 { p.fill_round(r, *radius, c); } else { p.fill_rect(r, c); } }
                if *border { p.stroke_round(r, *radius, t.border_width(), pal.border); }
            }
            Kind::Label { text, style, wrap, align, color } => {
                let face = if *style == TextStyle::Mono { Face::Mono } else { Face::Sans };
                let size = text_px(t, *style);
                let c = if !n.enabled { pal.text_disabled } else { color.unwrap_or(if *style == TextStyle::Caption { pal.text_dim } else { pal.text }) };
                let s = t.scale;
                let lh = self.fonts.line_height(face, size * s) / s;
                let lines: Vec<(usize, usize)> = if *wrap { self.fonts.wrap(face, size * s, text, r.w * s) } else {
                    let mut v = Vec::new();
                    let mut start = 0;
                    for (i, ch) in text.char_indices() { if ch == '\n' { v.push((start, i)); start = i + 1; } }
                    v.push((start, text.len()));
                    v
                };
                for (i, (a, b)) in lines.iter().enumerate() {
                    let line = text[*a..*b].trim_end_matches('\n');
                    let lw = self.width(face, size, line);
                    let x = match align { Align::Start => r.x, Align::Center => r.x + (r.w - lw) / 2.0, Align::End => r.right() - lw };
                    self.text(p, face, size, x, r.y + i as f32 * lh, lh, line, c);
                }
            }
            Kind::Button { text, icon, style } => {
                let (bg, mut fgc, border) = match style {
                    ButtonStyle::Primary => (if down { pal.accent.mix(Color::hex(0), 0.2) } else if hot { pal.accent.mix(pal.on_accent, 0.12) } else { pal.accent }, pal.on_accent, false),
                    ButtonStyle::Danger => (if down { pal.danger.mix(Color::hex(0), 0.2) } else if hot { pal.danger.mix(pal.on_danger, 0.12) } else { pal.danger }, pal.on_danger, false),
                    ButtonStyle::Flat => (if down { pal.control_pressed } else if hot { pal.control_hover } else { Color::TRANSPARENT }, pal.text, false),
                    ButtonStyle::Normal => (if down { pal.control_pressed } else if hot { pal.control_hover } else { pal.control }, pal.text, true),
                };
                let bg = if n.enabled { bg } else { bg.mix(pal.window, 0.5) };
                if !n.enabled { fgc = pal.text_disabled; }
                if bg.a() > 0 { p.fill_round(r, t.radius, bg); }
                if border || t.mode == Mode::HighContrast { p.stroke_round(r, t.radius, t.border_width(), pal.border); }
                let tw = if text.is_empty() { 0.0 } else { self.width(Face::Sans, body, text) };
                let iw = if icon.is_some() { 18.0 } else { 0.0 };
                let gap = if iw > 0.0 && tw > 0.0 { 6.0 } else { 0.0 };
                let x0 = r.x + (r.w - tw - iw - gap) / 2.0;
                if let Some(i) = icon { self.icon(p, i, Rect::new(x0, r.y + (r.h - 18.0) / 2.0, 18.0, 18.0), fgc); }
                if !text.is_empty() { self.text(p, Face::Sans, body, x0 + iw + gap, r.y, r.h, text, fgc); }
            }
            Kind::Field(f) => {
                let focused = self.focus == Some(id);
                p.fill_round(r, t.radius, if n.enabled { pal.field } else { pal.field.mix(pal.window, 0.5) });
                let (bc, bw) = if focused { (pal.accent, 2.0) } else { (if hot { pal.text_dim } else { pal.border }, t.border_width()) };
                p.stroke_round(r, t.radius, bw, bc);
                let pad = t.padding();
                let inner = Rect::new(r.x + pad, r.y, (r.w - pad * 2.0).max(0.0), r.h);
                p.push_clip(inner.inset_xy(-1.0, 2.0));
                let bits = self.fields.iter().find(|x| x.0 == id).map(|x| x.1.clone());
                if let Some(b) = bits {
                    let x0 = inner.x - f.scroll;
                    if focused && b.sel.1 > b.sel.0 {
                        let sel = Rect::new(x0 + b.sel.0, r.y + 5.0, b.sel.1 - b.sel.0, r.h - 10.0);
                        p.fill_rect(sel, pal.selection);
                    }
                    if b.shown.is_empty() {
                        if !f.placeholder.is_empty() { self.text(p, Face::Sans, body, inner.x, r.y, r.h, &f.placeholder, pal.text_dim); }
                    } else {
                        self.text(p, Face::Sans, body, x0, r.y, r.h, &b.shown, fg);
                    }
                    if focused && n.enabled {
                        let cx = libm::roundf((x0 + b.cursor) * t.scale) / t.scale;
                        p.fill_rect(Rect::new(cx, r.y + 6.0, 1.5, r.h - 12.0), pal.text);
                    }
                }
                p.pop_clip();
            }
            Kind::Check { text, on, switch } => {
                let cy = r.y + r.h / 2.0;
                if *switch {
                    let track = Rect::new(r.x, cy - 10.0, 36.0, 20.0);
                    p.fill_round(track, 10.0, if *on { pal.accent } else if hot { pal.text_dim } else { pal.border });
                    let kx = if *on { track.right() - 10.0 } else { track.x + 10.0 };
                    p.fill_circle(kx, cy, 7.0, if *on { pal.on_accent } else { pal.field });
                } else {
                    let b = Rect::new(r.x, cy - 9.0, 18.0, 18.0);
                    if *on {
                        p.fill_round(b, 4.0, if n.enabled { pal.accent } else { pal.text_disabled });
                        p.line(b.x + 4.0, cy, b.x + 7.5, cy + 3.5, 2.0, pal.on_accent);
                        p.line(b.x + 7.5, cy + 3.5, b.x + 14.0, cy - 4.0, 2.0, pal.on_accent);
                    } else {
                        p.fill_round(b, 4.0, pal.field);
                        p.stroke_round(b, 4.0, t.border_width().max(1.5), if hot { pal.accent } else { pal.text_dim });
                    }
                }
                let x = r.x + self.check_width(n) + 8.0;
                self.text(p, Face::Sans, body, x, r.y, r.h, text, fg);
            }
            Kind::Radio { text, on, .. } => {
                let (cx, cy) = (r.x + 9.0, r.y + r.h / 2.0);
                if *on {
                    p.fill_circle(cx, cy, 9.0, if n.enabled { pal.accent } else { pal.text_disabled });
                    p.fill_circle(cx, cy, 4.0, pal.on_accent);
                } else {
                    p.fill_circle(cx, cy, 9.0, if hot { pal.accent } else { pal.text_dim });
                    p.fill_circle(cx, cy, 7.5, pal.field);
                }
                self.text(p, Face::Sans, body, r.x + 26.0, r.y, r.h, text, fg);
            }
            Kind::Slider { value, min, max, .. } => {
                let f = if max > min { (value - min) / (max - min) } else { 0.0 };
                let (x0, x1, cy) = (r.x + 8.0, r.right() - 8.0, r.y + r.h / 2.0);
                p.fill_round(Rect::new(x0, cy - 2.0, x1 - x0, 4.0), 2.0, pal.border);
                let kx = x0 + (x1 - x0) * f;
                let ac = if n.enabled { pal.accent } else { pal.text_disabled };
                p.fill_round(Rect::new(x0, cy - 2.0, kx - x0, 4.0), 2.0, ac);
                p.fill_circle(kx, cy, if hot || down { 9.0 } else { 8.0 }, ac);
                p.fill_circle(kx, cy, 3.0, pal.on_accent);
            }
            Kind::Progress { value } => {
                let track = Rect::new(r.x, r.y + (r.h - 6.0) / 2.0, r.w, 6.0);
                p.fill_round(track, 3.0, pal.border);
                match value {
                    Some(v) => p.fill_round(Rect::new(track.x, track.y, track.w * v.clamp(0.0, 1.0), track.h), 3.0, pal.accent),
                    None => {
                        let phase = (self.now % 1500) as f32 / 1500.0;
                        let w = track.w * 0.3;
                        let x = track.x - w + (track.w + w) * phase;
                        p.push_clip(track);
                        p.fill_round(Rect::new(x, track.y, w, track.h), 3.0, pal.accent);
                        p.pop_clip();
                    }
                }
            }
            Kind::Scroll { .. } => {}
            Kind::Split { .. } => {
                if let Some((_, d)) = self.dividers.iter().find(|x| x.0 == id) {
                    let active = hot || self.drag == Drag::Split(id);
                    let c = if active { pal.accent } else { pal.border };
                    if d.w < d.h { p.fill_rect(Rect::new(d.x + d.w / 2.0 - 0.5, d.y, 1.0, d.h), c); } else { p.fill_rect(Rect::new(d.x, d.y + d.h / 2.0 - 0.5, d.w, 1.0), c); }
                }
            }
            Kind::List(l) => self.list(p, id, n, l),
            Kind::Tabs(tb) => self.tabs(p, id, n, tb),
            Kind::Icon { name, .. } => self.icon(p, name, r, fg),
            Kind::Image { w, h, pixels } => p.image(r, *w, *h, pixels),
            Kind::Separator => {
                if r.w >= r.h { p.fill_rect(Rect::new(r.x, r.y + r.h / 2.0 - 0.5, r.w, 1.0), pal.border); }
                else { p.fill_rect(Rect::new(r.x + r.w / 2.0 - 0.5, r.y, 1.0, r.h), pal.border); }
            }
            Kind::MenuBar { menus, open } => {
                let titles = self.titles.iter().find(|x| x.0 == id).map(|x| x.1.clone()).unwrap_or_default();
                for (i, (m, tr)) in menus.iter().zip(titles).enumerate() {
                    if *open == Some(i) { p.fill_round(tr.inset_xy(1.0, 3.0), t.radius, pal.control_pressed); }
                    self.text(p, Face::Sans, body, tr.x + t.padding(), tr.y, tr.h, &m.title, fg);
                }
            }
            Kind::MenuRow(m) => {
                if m.separator { p.fill_rect(Rect::new(r.x + 6.0, r.y + r.h / 2.0, r.w - 12.0, 1.0), pal.border); return; }
                let lit = self.menus.get(m.menu).map_or(false, |om| om.hot == Some(m.index));
                let c = if !m.enabled { pal.text_disabled } else if lit { pal.on_accent } else { pal.text };
                if lit { p.fill_round(r, t.radius, pal.accent); }
                if m.check == Some(true) { self.icon(p, "check", Rect::new(r.x + 6.0, r.y + (r.h - 16.0) / 2.0, 16.0, 16.0), c); }
                else if let Some(i) = &m.icon { self.icon(p, i, Rect::new(r.x + 6.0, r.y + (r.h - 16.0) / 2.0, 16.0, 16.0), c); }
                self.text(p, Face::Sans, body, r.x + 30.0, r.y, r.h, &m.label, c);
                if m.submenu { self.icon(p, "chevron-right", Rect::new(r.right() - 22.0, r.y + (r.h - 14.0) / 2.0, 14.0, 14.0), c); }
                else if !m.shortcut.is_empty() {
                    let size = body * 0.92;
                    let w = self.width(Face::Sans, size, &m.shortcut);
                    self.text(p, Face::Sans, size, r.right() - 10.0 - w, r.y, r.h, &m.shortcut, if lit { c } else { pal.text_dim });
                }
            }
        }
    }

    fn list(&mut self, p: &mut Painter, id: Id, n: &Node, l: &ListState) {
        let t = self.theme;
        let pal = &t.palette;
        let r = n.rect;
        let Some((_, g)) = self.lists.iter().find(|x| x.0 == id).copied() else { return };
        let focused = self.focus == Some(id);
        p.fill_round(r, t.radius, pal.field);
        p.push_clip(r.inset(1.0));
        let body = t.font_size;
        let pad = t.padding();
        let bar = if self.bars.iter().any(|b| b.0 == id) { crate::events::BAR_W } else { 0.0 };
        // Column x ranges; the last column takes what is left.
        let mut cols: Vec<(f32, f32, Align)> = Vec::new();
        if l.columns.is_empty() { cols.push((r.x, r.w - bar, Align::Start)); } else {
            let mut x = r.x;
            for (i, c) in l.columns.iter().enumerate() {
                let w = if i + 1 == l.columns.len() { (r.right() - bar - x).max(0.0) } else { c.width };
                cols.push((x, w, c.align));
                x += w;
            }
        }
        if g.header.h > 0.0 {
            p.fill_rect(g.header, pal.control);
            p.fill_rect(Rect::new(g.header.x, g.header.bottom() - 1.0, g.header.w, 1.0), pal.border);
            for (i, c) in l.columns.iter().enumerate() {
                let (x, w, align) = cols[i];
                p.push_clip(Rect::new(x, g.header.y, w, g.header.h));
                let tw = self.width(Face::Sans, body, &c.title);
                let tx = if align == Align::End { x + w - pad - tw } else { x + pad };
                self.text(p, Face::Sans, body, tx, g.header.y, g.header.h, &c.title, pal.text_dim);
                if let Some((sc, asc)) = l.sort { if sc == i {
                    let ax = if align == Align::End { tx - 16.0 } else { tx + tw + 4.0 };
                    self.icon(p, if asc { "chevron-up" } else { "chevron-down" }, Rect::new(ax, g.header.y + (g.header.h - 12.0) / 2.0, 12.0, 12.0), pal.text_dim);
                } }
                p.pop_clip();
                if i > 0 { p.fill_rect(Rect::new(x, g.header.y + 4.0, 1.0, g.header.h - 8.0), pal.border); }
            }
        }
        p.push_clip(g.rows);
        let len = l.model.len();
        let first = (l.scroll / g.row_h) as usize;
        let count = (g.rows.h / g.row_h) as usize + 2;
        let tree = (0..len.min(64)).any(|r| l.model.expandable(r).is_some() || l.model.depth(r) > 0);
        let mut cell = String::new();
        for row in first..(first + count).min(len) {
            let y = g.rows.y + row as f32 * g.row_h - l.scroll;
            let rr = Rect::new(r.x + 2.0, y, r.w - 4.0 - bar, g.row_h);
            let sel = l.selection.contains(row);
            if sel { p.fill_round(rr, t.radius - 2.0, if focused { pal.selection } else { pal.selection.mix(pal.field, 0.4) }); }
            if focused && self.focus_visible && row == l.cursor { p.stroke_round(rr.inset(0.5), t.radius - 2.0, 1.0, pal.focus); }
            let c = if n.enabled { pal.text } else { pal.text_disabled };
            for (ci, (x, w, align)) in cols.iter().enumerate() {
                let mut tx = *x + pad;
                if ci == 0 {
                    let depth = l.model.depth(row) as f32;
                    if tree {
                        tx = *x + 4.0 + depth * 16.0;
                        if let Some(open) = l.model.expandable(row) {
                            self.icon(p, if open { "chevron-down" } else { "chevron-right" }, Rect::new(tx + 1.0, y + (g.row_h - 14.0) / 2.0, 14.0, 14.0), pal.text_dim);
                        }
                        tx += 18.0;
                    }
                    if let Some(icon) = l.model.icon(row) {
                        self.icon(p, icon, Rect::new(tx, y + (g.row_h - 16.0) / 2.0, 16.0, 16.0), c);
                        tx += 22.0;
                    }
                }
                cell.clear();
                l.model.cell(row, ci, &mut cell);
                if cell.is_empty() { continue; }
                p.push_clip(Rect::new(*x, y, (*w - 2.0).max(0.0), g.row_h));
                if *align == Align::End { let tw = self.width(Face::Sans, t.font_size, &cell); tx = x + w - pad - tw; }
                self.text(p, Face::Sans, t.font_size, tx, y, g.row_h, &cell, c);
                p.pop_clip();
            }
        }
        p.pop_clip();
        p.pop_clip();
        self.scrollbar(p, id);
        p.stroke_round(r, t.radius, if focused { 2.0 } else { t.border_width() }, if focused { pal.accent } else { pal.border });
    }

    fn tabs(&mut self, p: &mut Painter, id: Id, n: &Node, tb: &TabsState) {
        let t = self.theme;
        let pal = &t.palette;
        let r = n.rect;
        let sh = tab_strip_height(t);
        let strip = Rect::new(r.x, r.y, r.w, sh);
        let strip_bg = pal.window.mix(pal.text, 0.05);
        p.fill_rect(strip, strip_bg);
        p.fill_rect(Rect::new(r.x, strip.bottom() - 1.0, r.w, 1.0), pal.border);
        p.push_clip(Rect::new(r.x, r.y, tb.room.max(0.0), sh));
        let body = t.font_size;
        for (i, tab) in tb.tabs.iter().enumerate() {
            let Some(&(x, w)) = tb.spans.get(i) else { continue };
            let tr = Rect::new(r.x + x - tb.scroll, r.y + 3.0, w - 2.0, sh - 3.0);
            let active = i == tb.active;
            let hot = tb.hover == Some(i) && self.hover == Some(id);
            if active {
                p.fill_round(Rect::new(tr.x, tr.y, tr.w, tr.h + 6.0), t.radius, pal.window);
                p.stroke_round(Rect::new(tr.x, tr.y, tr.w, tr.h + 6.0), t.radius, 1.0, pal.border);
                p.fill_rect(Rect::new(tr.x, tr.bottom() - 1.0, tr.w, 2.0), pal.window);
                p.fill_round(Rect::new(tr.x + 6.0, tr.y, tr.w - 12.0, 2.5), 1.0, pal.accent);
            } else if hot {
                p.fill_round(tr.inset_xy(1.0, 2.0), t.radius, pal.control_hover);
            }
            if let Some(gc) = tab.group { p.fill_rect(Rect::new(tr.x + 4.0, tr.bottom() - 3.0, tr.w - 8.0, 3.0), gc); }
            let c = if active { pal.text } else { pal.text_dim };
            let mut x0 = tr.x + t.padding();
            if tab.pinned {
                let icon = tab.icon.as_deref().unwrap_or("pin");
                self.icon(p, icon, Rect::new(tr.x + (tr.w - 16.0) / 2.0, tr.y + (tr.h - 16.0) / 2.0, 16.0, 16.0), c);
                if tab.modified { p.fill_circle(tr.right() - 6.0, tr.y + 6.0, 3.0, pal.accent); }
                continue;
            }
            if let Some(icon) = &tab.icon { self.icon(p, icon, Rect::new(x0, tr.y + (tr.h - 16.0) / 2.0, 16.0, 16.0), c); x0 += 22.0; }
            let close_w = if tb.closable { 26.0 } else { 0.0 };
            p.push_clip(Rect::new(x0, tr.y, (tr.right() - close_w - x0 - 2.0).max(0.0), tr.h));
            self.text(p, Face::Sans, body, x0, tr.y, tr.h, &tab.title, c);
            p.pop_clip();
            if tb.closable {
                let cr = Rect::new(tr.right() - 24.0, tr.y + (tr.h - 18.0) / 2.0, 18.0, 18.0);
                let on_close = tb.hover_close == Some(i) && self.hover == Some(id);
                if tab.modified && !on_close && !hot { p.fill_circle(cr.center().x, cr.center().y, 4.0, pal.text_dim); }
                else if active || hot {
                    if on_close { p.fill_round(cr, 4.0, pal.control_pressed); }
                    self.icon(p, "close", cr.inset(3.0), c);
                }
            }
        }
        p.pop_clip();
        if let Some((_, nb)) = self.tab_new.iter().find(|x| x.0 == id) {
            self.icon(p, "plus", nb.inset(8.0), pal.text_dim);
        }
    }
}

fn label_of(k: &Kind) -> &str {
    match k { Kind::Check { text, .. } | Kind::Radio { text, .. } | Kind::Button { text, .. } | Kind::Label { text, .. } => text, _ => "" }
}
