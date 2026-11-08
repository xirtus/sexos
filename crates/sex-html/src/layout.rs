// SPDX-License-Identifier: GPL-3.0-only
//! Layout: styled elements into positioned boxes, then a display list.
//!
//! Normal flow follows CSS 2: block boxes stack with collapsing sibling margins,
//! inline content fills line boxes (greedy breaking, baseline alignment, half-leading,
//! `text-align`), `white-space` is honoured. Tables use the automatic table layout
//! (column widths from min-/max-content), flex containers lay rows with grow, wrap,
//! gap and `justify-content`, and inline blocks, images and form controls are atomic
//! boxes on the line. Floats and positioned boxes stay in normal flow for now.
//! All coordinates are integer pixels.
use crate::dom::{Dom, Kind, NodeId};
use crate::style::{self, Align, Display, Justify, Len, ListStyle, Style, Transform, WhiteSpace, BOTTOM, LEFT, RIGHT, TOP};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontKey { pub size: i32, pub bold: bool, pub italic: bool, pub mono: bool }

/// What layout asks of the system: text measurement and image sizes.
pub trait Host {
    /// Advance width of `text` in pixels.
    fn text_width(&self, text: &str, font: FontKey) -> i32;
    /// Ascent above and descent below the baseline, both positive pixels.
    fn metrics(&self, font: FontKey) -> (i32, i32);
    /// Natural size of the image at `src`, when known.
    fn image_size(&self, _src: &str) -> Option<(i32, i32)> { None }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bullet { Disc, Circle, Square }

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Rect { x: i32, y: i32, w: i32, h: i32, color: u32, radius: i32 },
    Text { x: i32, baseline: i32, font: FontKey, color: u32, text: String, underline: bool, strike: bool },
    Image { x: i32, y: i32, w: i32, h: i32, src: String, alt: String },
    Bullet { x: i32, y: i32, size: i32, color: u32, kind: Bullet },
}

impl Item {
    fn offset(&mut self, dx: i32, dy: i32) {
        match self {
            Item::Rect { x, y, .. } | Item::Image { x, y, .. } | Item::Bullet { x, y, .. } => { *x += dx; *y += dy; }
            Item::Text { x, baseline, .. } => { *x += dx; *baseline += dy; }
        }
    }
    /// Top and bottom pixel rows this item can touch.
    pub fn vertical_span(&self) -> (i32, i32) {
        match self {
            Item::Rect { y, h, .. } | Item::Image { y, h, .. } => (*y, *y + *h),
            Item::Bullet { y, size, .. } => (*y, *y + *size),
            Item::Text { baseline, font, .. } => (*baseline - font.size * 5 / 4, *baseline + font.size / 2),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Something the person can activate: a link, a button, or an element a script listens to.
/// `href` is empty when it is not a link.
pub struct Link { pub x: i32, pub y: i32, pub w: i32, pub h: i32, pub href: String, pub node: NodeId }

pub struct Page {
    pub width: i32,
    pub height: i32,
    pub background: u32,
    pub title: String,
    pub items: Vec<Item>,
    pub links: Vec<Link>,
    /// `id` attributes and the y of their box, for `#fragment` links.
    pub anchors: Vec<(String, i32)>,
}

impl Page {
    /// The link under a point, if any.
    pub fn link_at(&self, x: i32, y: i32) -> Option<&Link> {
        self.links.iter().find(|l| x >= l.x && x < l.x + l.w && y >= l.y && y < l.y + l.h)
    }
}

/// Lays out `dom` (already styled by `styles`) for a viewport `width` pixels wide.
pub fn layout(dom: &Dom, styles: &[Option<Style>], width: i32, host: &dyn Host) -> Page {
    let mut l = Layout { dom, styles, host, items: Vec::new(), links: Vec::new(), anchors: Vec::new(), intrinsic: alloc::vec![None; dom.nodes.len()], link_stack: Vec::new() };
    let html = dom.find("html");
    let body = dom.find("body");
    let mut height = 0;
    if let Some(html) = html {
        let r = l.block(html, 0, 0, width, None);
        height = r.margin_top + r.height + r.margin_bottom;
    }
    let bg = |n: Option<NodeId>| n.and_then(|n| styles[n].as_ref()).map_or(0, |s| s.background);
    let background = match (bg(html), bg(body)) { (h, _) if h >> 24 != 0 => h, (_, b) if b >> 24 != 0 => b, _ => 0xFFFF_FFFF };
    // The root's background paints the whole canvas, not just its box.
    if let Some(Item::Rect { color, .. }) = l.items.iter_mut().find(|i| matches!(i, Item::Rect { .. })) { if *color == background { *color = 0; } }
    let title = dom.find("title").map(|t| collapse(&dom.text_content(t))).unwrap_or_default();
    Page { width, height, background: background | 0xFF00_0000, title, items: l.items, links: l.links, anchors: l.anchors }
}

/// Parses, styles and lays out a whole page.
pub fn render(html: &str, width: i32, viewport_height: i32, host: &dyn Host) -> Page {
    let dom = crate::parse::parse(html);
    let styles = crate::styles(&dom, &[], width, viewport_height);
    layout(&dom, &styles, width, host)
}

fn collapse(s: &str) -> String {
    let mut out = String::new();
    for w in s.split_ascii_whitespace() { if !out.is_empty() { out.push(' '); } out.push_str(w); }
    out
}

/// Elements whose content is not their children: drawn at their own (or intrinsic) size.
const REPLACED: &[&str] = &["img", "svg", "canvas", "video", "iframe", "object", "embed", "input", "select", "textarea"];

struct BlockResult { height: i32, margin_top: i32, margin_bottom: i32, first_baseline: Option<i32>, width: i32 }

struct Layout<'a> {
    dom: &'a Dom,
    styles: &'a [Option<Style>],
    host: &'a dyn Host,
    items: Vec<Item>,
    links: Vec<Link>,
    anchors: Vec<(String, i32)>,
    intrinsic: Vec<Option<(i32, i32)>>,
    link_stack: Vec<usize>,
}

/// One unbreakable thing on a line.
enum Piece {
    Word { text: String, font: FontKey, color: u32, underline: bool, strike: bool, link: Option<NodeId>, bg: u32, width: i32, shift: i32 },
    Space { width: i32, breakable: bool, font: FontKey, color: u32, underline: bool, link: Option<NodeId>, bg: u32 },
    Break,
    Atomic { items: Vec<Item>, links: Vec<Link>, width: i32, height: i32, link: Option<NodeId> },
    Gap(i32),
    /// An `id` (or `<a name>`) inside the line: where a `#fragment` link scrolls to.
    Anchor(String),
}

impl Piece {
    fn width(&self) -> i32 {
        match self { Piece::Word { width, .. } | Piece::Space { width, .. } | Piece::Atomic { width, .. } | Piece::Gap(width) => *width, Piece::Break | Piece::Anchor(_) => 0 }
    }
}

fn font_of(s: &Style) -> FontKey { FontKey { size: s.font_size.max(1), bold: s.bold, italic: s.italic, mono: s.mono } }

fn sum_h(s: &Style, cw: i32) -> (i32, i32, i32) {
    // margins, border+padding (left + right)
    let m = s.margin[LEFT].or_zero(cw) + s.margin[RIGHT].or_zero(cw);
    let bp = s.border[LEFT] + s.border[RIGHT] + s.padding[LEFT].or_zero(cw) + s.padding[RIGHT].or_zero(cw);
    (m, bp, m + bp)
}

impl<'a> Layout<'a> {
    fn style(&self, n: NodeId) -> Option<&'a Style> { self.styles.get(n).and_then(|s| s.as_ref()) }

    fn display(&self, n: NodeId) -> Display {
        match &self.dom.nodes[n].kind {
            Kind::Text(_) => Display::Inline,
            Kind::Element { .. } => self.style(n).map_or(Display::None, |s| s.display),
            Kind::Document => Display::Block,
        }
    }

    fn is_block(&self, n: NodeId) -> bool {
        let d = self.display(n);
        if d == Display::Contents { return self.dom.nodes[n].children.iter().any(|&c| self.is_block(c)); }
        self.style(n).map_or(false, |s| s.is_block_level())
    }

    /// Lays out a block-level box whose margin box starts at (`x`, `y`) inside a
    /// containing block `cw` wide. `force_width` sets the border-box width (flex items, cells).
    fn block(&mut self, n: NodeId, x: i32, y: i32, cw: i32, force_width: Option<i32>) -> BlockResult {
        let s = self.style(n).unwrap();
        if let Some(tag) = self.dom.tag(n).filter(|t| REPLACED.contains(t)) {
            // Pictures and controls keep their own size in block flow too.
            let mt = s.margin[TOP].or_zero(cw);
            let mb = s.margin[BOTTOM].or_zero(cw);
            let pushed = self.push_link(n);
            let piece = self.atomic(n, s, tag);
            if pushed { self.link_stack.pop(); }
            if let Some(Piece::Atomic { mut items, links, width, height, link }) = piece {
                for it in &mut items { it.offset(x, y - mt); }
                self.items.extend(items);
                for mut l in links { l.x += x; l.y += y - mt; self.links.push(l); }
                if let Some(a) = link.or_else(|| self.link_stack.last().copied()) { self.add_link(a, x, y, width, height - mt - mb); }
                return BlockResult { height: (height - mt - mb).max(0), margin_top: mt, margin_bottom: mb, first_baseline: None, width };
            }
        }
        let (_, bp, _) = sum_h(s, cw);
        let mut ml = s.margin[LEFT].or_zero(cw);
        let mut mr = s.margin[RIGHT].or_zero(cw);
        let mt = s.margin[TOP].or_zero(cw);
        let mb = s.margin[BOTTOM].or_zero(cw);
        let mut content_w = match (force_width, s.width.resolve(cw)) {
            (Some(f), _) => f - bp,
            (None, Some(w)) => if s.box_sizing_border { w - bp } else { w },
            (None, None) => {
                if matches!(s.display, Display::Table) || s.float != style::Float::None || matches!(s.position, style::Position::Absolute | style::Position::Fixed) {
                    let (_, max) = self.intrinsic(n);
                    max.min(cw - ml - mr) - bp
                } else {
                    cw - ml - mr - bp
                }
            }
        };
        if let Some(max) = s.max_width.resolve(cw) { content_w = content_w.min(if s.box_sizing_border { max - bp } else { max }); }
        if let Some(min) = s.min_width.resolve(cw) { content_w = content_w.max(if s.box_sizing_border { min - bp } else { min }); }
        content_w = content_w.max(0);
        let used = content_w + bp;
        if force_width.is_none() {
            let free = cw - used - (if s.margin[LEFT] == Len::Auto { 0 } else { ml }) - (if s.margin[RIGHT] == Len::Auto { 0 } else { mr });
            match (s.margin[LEFT] == Len::Auto, s.margin[RIGHT] == Len::Auto) {
                (true, true) => { ml = (free / 2).max(0); mr = free - ml; }
                (true, false) => ml = free.max(0),
                _ => {}
            }
            if s.float == style::Float::Right && s.margin[LEFT] != Len::Auto { ml = (cw - used - mr).max(ml); }
        }
        let _ = mr;
        let bx = x + ml;
        let by = y;
        if let Some(id) = self.dom.attr(n, "id") { self.anchors.push((String::from(id), by)); }
        if let Some(name) = self.dom.attr(n, "name").filter(|_| self.dom.tag(n) == Some("a")) { self.anchors.push((String::from(name), by)); }
        let bg_index = self.items.len();
        let paints = s.background >> 24 != 0 || s.border.iter().any(|b| *b > 0);
        if paints { self.items.push(Item::Rect { x: 0, y: 0, w: 0, h: 0, color: 0, radius: 0 }); }
        let cx = bx + s.border[LEFT] + s.padding[LEFT].or_zero(cw);
        let cy = by + s.border[TOP] + s.padding[TOP].or_zero(cw);
        let pushed_link = self.push_link(n);
        let (content_h, first_baseline) = match s.display {
            Display::Table => self.table(n, cx, cy, content_w),
            Display::TableRow => self.row(n, cx, cy, content_w),
            Display::Flex if !s.flex_column => self.flex_row(n, cx, cy, content_w),
            _ => self.flow(n, cx, cy, content_w),
        };
        if pushed_link { self.link_stack.pop(); }
        let mut inner_h = match s.height.resolve(0) {
            Some(h) if !matches!(s.height, Len::Pct(_)) => if s.box_sizing_border { h - (s.border[TOP] + s.border[BOTTOM] + s.padding[TOP].or_zero(cw) + s.padding[BOTTOM].or_zero(cw)) } else { h },
            _ => content_h,
        };
        if let Some(min) = s.min_height.resolve(0) { inner_h = inner_h.max(min); }
        inner_h = inner_h.max(0);
        let height = inner_h + s.border[TOP] + s.border[BOTTOM] + s.padding[TOP].or_zero(cw) + s.padding[BOTTOM].or_zero(cw);
        if paints {
            self.box_paint(bg_index, s, bx, by, used, height);
        }
        if s.display == Display::ListItem {
            if let Some(base) = first_baseline { self.marker(n, s, cx, base); }
        }
        if s.hidden { self.items.truncate(bg_index); }
        BlockResult { height, margin_top: mt, margin_bottom: mb, first_baseline, width: used + ml + mr }
    }

    /// Fills the placeholder at `index` with the background and pushes borders after it.
    fn box_paint(&mut self, index: usize, s: &Style, x: i32, y: i32, w: i32, h: i32) {
        self.items[index] = Item::Rect { x, y, w, h, color: s.background, radius: s.radius };
        let b = s.border;
        let c = s.border_color;
        let uniform = b.iter().all(|v| *v == b[0]) && c.iter().all(|v| *v == c[0]);
        if s.radius > 0 && uniform && b[0] > 0 && s.background >> 24 == 0xFF {
            // A rounded border: the border colour behind, the background inset over it.
            self.items[index] = Item::Rect { x, y, w, h, color: c[0], radius: s.radius };
            let inner = Item::Rect { x: x + b[0], y: y + b[0], w: w - 2 * b[0], h: h - 2 * b[0], color: s.background, radius: (s.radius - b[0]).max(0) };
            self.items.insert(index + 1, inner);
            return;
        }
        let mut borders = Vec::new();
        if b[TOP] > 0 { borders.push(Item::Rect { x, y, w, h: b[TOP], color: c[TOP], radius: 0 }); }
        if b[BOTTOM] > 0 { borders.push(Item::Rect { x, y: y + h - b[BOTTOM], w, h: b[BOTTOM], color: c[BOTTOM], radius: 0 }); }
        if b[LEFT] > 0 { borders.push(Item::Rect { x, y, w: b[LEFT], h, color: c[LEFT], radius: 0 }); }
        if b[RIGHT] > 0 { borders.push(Item::Rect { x: x + w - b[RIGHT], y, w: b[RIGHT], h, color: c[RIGHT], radius: 0 }); }
        let at = index + 1;
        for (i, item) in borders.into_iter().enumerate() { self.items.insert(at + i, item); }
    }

    fn marker(&mut self, n: NodeId, s: &Style, cx: i32, baseline: i32) {
        let font = font_of(s);
        let gap = s.font_size / 2;
        let kind = match s.list_style {
            ListStyle::None => return,
            ListStyle::Disc => Some(Bullet::Disc),
            ListStyle::Circle => Some(Bullet::Circle),
            ListStyle::Square => Some(Bullet::Square),
            _ => None,
        };
        if let Some(kind) = kind {
            let size = (s.font_size * 3 / 8).max(4);
            let (asc, _) = self.host.metrics(font);
            let mid = baseline - asc * 3 / 8;
            self.items.push(Item::Bullet { x: cx - gap - size, y: mid - size / 2, size, color: s.color, kind });
            return;
        }
        let index = self.list_index(n);
        let mut text = match s.list_style {
            ListStyle::LowerAlpha | ListStyle::UpperAlpha => alpha(index, s.list_style == ListStyle::UpperAlpha),
            ListStyle::LowerRoman | ListStyle::UpperRoman => roman(index, s.list_style == ListStyle::UpperRoman),
            _ => itoa(index),
        };
        text.push('.');
        let w = self.host.text_width(&text, font);
        self.items.push(Item::Text { x: cx - gap - w, baseline, font, color: s.color, text, underline: false, strike: false });
    }

    fn list_index(&self, n: NodeId) -> i32 {
        if let Some(v) = self.dom.attr(n, "value").and_then(|v| v.trim().parse().ok()) { return v; }
        let parent = self.dom.nodes[n].parent.unwrap_or(0);
        let mut i: i32 = self.dom.attr(parent, "start").and_then(|v| v.trim().parse().ok()).unwrap_or(1);
        for c in self.dom.element_children(parent) {
            if c == n { break; }
            if self.display(c) == Display::ListItem { i += 1; }
        }
        i
    }

    fn push_link(&mut self, n: NodeId) -> bool {
        if self.dom.activatable(n) { self.link_stack.push(n); true } else { false }
    }

    /// Normal flow of a block container's children. Returns content height and first baseline.
    fn flow(&mut self, n: NodeId, cx: i32, cy: i32, cw: i32) -> (i32, Option<i32>) {
        let s = self.style(n).unwrap();
        let gap = if s.display == Display::Flex { s.gap } else { 0 };
        let mut y = cy;
        let mut pending_margin = 0;
        let mut first_baseline = None;
        let mut inline_run: Vec<NodeId> = Vec::new();
        let children = self.flat_children(n);
        let mut first_block = true;
        for c in children.iter().copied().chain(core::iter::once(usize::MAX)) {
            let block = c != usize::MAX && self.is_block(c);
            if c != usize::MAX && !block {
                if self.display(c) != Display::None { inline_run.push(c); }
                continue;
            }
            if !inline_run.is_empty() {
                let run = core::mem::take(&mut inline_run);
                if let Some((h, base)) = self.inline_context(n, &run, cx, y + pending_margin, cw) {
                    y += pending_margin;
                    pending_margin = 0;
                    if first_baseline.is_none() { first_baseline = base; }
                    y += h;
                    first_block = false;
                }
            }
            if c == usize::MAX { break; }
            let cs = self.style(c).unwrap();
            let mt = cs.margin[TOP].or_zero(cw);
            let collapsed = if first_block && pending_margin == 0 { mt } else if mt >= 0 && pending_margin >= 0 { mt.max(pending_margin) } else { mt + pending_margin };
            let top = y + collapsed + if first_block { 0 } else { gap };
            let r = self.block(c, cx, top, cw, None);
            if first_baseline.is_none() { first_baseline = r.first_baseline; }
            y = top + r.height;
            pending_margin = r.margin_bottom;
            first_block = false;
        }
        y += pending_margin.max(0);
        (y - cy, first_baseline)
    }

    /// Children with `display: contents` unwrapped.
    fn flat_children(&self, n: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        for &c in &self.dom.nodes[n].children {
            if matches!(self.dom.nodes[c].kind, Kind::Element { .. }) && self.display(c) == Display::Contents && self.dom.nodes[c].children.iter().any(|&g| self.is_block(g)) {
                out.extend(self.flat_children(c));
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Lays out a run of inline-level nodes as line boxes. `None` when it holds only collapsible space.
    fn inline_context(&mut self, container: NodeId, run: &[NodeId], x: i32, y: i32, w: i32) -> Option<(i32, Option<i32>)> {
        let cs = self.style(container).unwrap();
        let mut pieces = Vec::new();
        let mut space_before = true;
        for &n in run { self.collect(n, cs, &mut pieces, &mut space_before); }
        while matches!(pieces.last(), Some(Piece::Space { breakable: true, .. })) { pieces.pop(); }
        if pieces.iter().all(|p| matches!(p, Piece::Space { .. } | Piece::Anchor(_))) {
            for p in pieces { if let Piece::Anchor(id) = p { self.anchors.push((id, y)); } }
            return None;
        }
        Some(self.lines(cs, pieces, x, y, w))
    }

    fn collect(&mut self, n: NodeId, parent: &Style, out: &mut Vec<Piece>, space_before: &mut bool) {
        match &self.dom.nodes[n].kind {
            Kind::Text(t) => { let s = parent; self.text_pieces(t, s, out, space_before); }
            Kind::Document => {}
            Kind::Element { tag, .. } => {
                let Some(s) = self.style(n) else { return };
                if s.display == Display::None { return; }
                let tag = tag.as_str();
                if tag == "br" { out.push(Piece::Break); *space_before = true; return; }
                let atomic = matches!(tag, "img" | "input" | "select" | "textarea" | "button" | "svg" | "canvas" | "video" | "iframe" | "object" | "embed" | "meter" | "progress")
                    || s.display == Display::InlineBlock || s.is_block_level();
                if atomic {
                    if tag == "img" && s.width == Len::Auto && s.height == Len::Auto {
                        let src = self.dom.attr(n, "src").unwrap_or("");
                        if self.host.image_size(src).is_none() {
                            // Unknown size: the alt text stands in, as text.
                            if let Some(alt) = self.dom.attr(n, "alt").filter(|a| !a.trim().is_empty()) {
                                let alt = alloc::format!("[{}]", alt.trim());
                                self.text_pieces(&alt, s, out, space_before);
                            }
                            return;
                        }
                    }
                    if let Some(p) = self.atomic(n, s, tag) { out.push(p); *space_before = false; }
                    return;
                }
                for name in ["id", "name"] {
                    if name == "name" && tag != "a" { continue; }
                    if let Some(id) = self.dom.attr(n, name) { out.push(Piece::Anchor(String::from(id))); }
                }
                let pushed = self.push_link(n);
                let left = s.margin[LEFT].or_zero(0) + s.padding[LEFT].or_zero(0) + s.border[LEFT];
                if left > 0 { out.push(Piece::Gap(left)); }
                for &c in &self.dom.nodes[n].children { self.collect(c, s, out, space_before); }
                let right = s.margin[RIGHT].or_zero(0) + s.padding[RIGHT].or_zero(0) + s.border[RIGHT];
                if right > 0 { out.push(Piece::Gap(right)); }
                if pushed { self.link_stack.pop(); }
            }
        }
    }

    fn text_pieces(&mut self, text: &str, s: &Style, out: &mut Vec<Piece>, space_before: &mut bool) {
        if s.display == Display::None { return; }
        let font = font_of(s);
        let link = self.link_stack.last().copied();
        let bg = s.background;
        let color = if s.hidden { 0 } else { s.color };
        let shift = match s.vertical_super { 1 => -(s.font_size / 2), -1 => s.font_size / 4, _ => 0 };
        let space = |this: &Self, breakable: bool| Piece::Space { width: this.host.text_width(" ", font), breakable, font, color, underline: s.underline, link, bg };
        let word = |this: &Self, w: &str, prev_cap: &mut bool| {
            let text = transform(w, s.transform, prev_cap);
            let width = this.host.text_width(&text, font) + s.letter_spacing * text.chars().count() as i32;
            Piece::Word { text, font, color, underline: s.underline, strike: s.strike, link, bg, width, shift }
        };
        let mut cap = true;
        match s.white_space {
            WhiteSpace::Pre | WhiteSpace::PreWrap => {
                for (i, line) in text.split('\n').enumerate() {
                    if i > 0 { out.push(Piece::Break); }
                    let line = expand_tabs(line.trim_end_matches('\r'));
                    if line.is_empty() { continue; }
                    if s.white_space == WhiteSpace::Pre {
                        out.push(word(self, &line, &mut cap));
                    } else {
                        for (j, part) in line.split(' ').enumerate() {
                            if j > 0 { out.push(space(self, true)); }
                            if !part.is_empty() { out.push(word(self, part, &mut cap)); }
                        }
                    }
                }
                *space_before = text.ends_with('\n');
            }
            _ => {
                let breakable = s.white_space != WhiteSpace::NoWrap;
                let lines: Vec<&str> = if s.white_space == WhiteSpace::PreLine { text.split('\n').collect() } else { alloc::vec![text] };
                for (li, line) in lines.iter().enumerate() {
                    if li > 0 { out.push(Piece::Break); *space_before = true; }
                    let mut at = 0;
                    let b = line.as_bytes();
                    while at < b.len() {
                        if is_space(b[at]) {
                            while at < b.len() && is_space(b[at]) { at += 1; }
                            if !*space_before { out.push(space(self, breakable)); *space_before = true; }
                            continue;
                        }
                        let start = at;
                        while at < b.len() && !is_space(b[at]) { at += 1; }
                        let w = &line[start..at];
                        // Long words may break after hyphens and slashes.
                        let mut rest = w;
                        while let Some(p) = rest.char_indices().find(|(i, c)| (*c == '-' || *c == '/' || *c == '\u{200B}') && *i > 0 && *i + 1 < rest.len()).map(|(i, c)| i + c.len_utf8()) {
                            if !breakable { break; }
                            out.push(word(self, &rest[..p], &mut cap));
                            out.push(Piece::Space { width: 0, breakable: true, font, color, underline: false, link, bg: 0 });
                            rest = &rest[p..];
                        }
                        out.push(word(self, rest, &mut cap));
                        *space_before = false;
                    }
                }
            }
        }
    }

    /// An atomic inline box (image, control, inline-block), laid out at the origin.
    fn atomic(&mut self, n: NodeId, s: &Style, tag: &str) -> Option<Piece> {
        let link = self.link_stack.last().copied();
        let (m_l, m_r) = (s.margin[LEFT].or_zero(0), s.margin[RIGHT].or_zero(0));
        let (m_t, m_b) = (s.margin[TOP].or_zero(0), s.margin[BOTTOM].or_zero(0));
        let saved_items = core::mem::take(&mut self.items);
        let saved_links = core::mem::take(&mut self.links);
        let (w, h) = match tag {
            "img" | "svg" | "canvas" | "video" | "iframe" | "object" | "embed" => {
                let src = String::from(self.dom.attr(n, "src").unwrap_or(""));
                let natural = self.host.image_size(&src);
                let (w, h) = match (s.width.resolve(0), s.height.resolve(0), natural) {
                    (Some(w), Some(h), _) => (w, h),
                    (Some(w), None, Some((nw, nh))) if nw > 0 => (w, w * nh / nw),
                    (None, Some(h), Some((nw, nh))) if nh > 0 => (h * nw / nh, h),
                    (None, None, Some(size)) => size,
                    (Some(w), None, _) => (w, if tag == "img" { w } else { 150 }),
                    (None, Some(h), _) => (if tag == "img" { h } else { 300 }, h),
                    _ => if tag == "img" { (0, 0) } else { (300, 150) },
                };
                let alt = String::from(self.dom.attr(n, "alt").unwrap_or(""));
                if w > 0 && h > 0 { self.items.push(Item::Image { x: s.border[LEFT], y: s.border[TOP], w, h, src, alt }); }
                (w + s.border[LEFT] + s.border[RIGHT], h + s.border[TOP] + s.border[BOTTOM])
            }
            "input" | "select" | "textarea" if s.display != Display::None => self.control(n, s, tag),
            _ => {
                let (min, max) = self.intrinsic(n);
                let (_, bp, _) = sum_h(s, 0);
                let width = match s.width.resolve(0) { Some(w) => if s.box_sizing_border { w } else { w + bp }, None => max.clamp(min, max.max(min)) };
                let r = self.block(n, 0, 0, width + m_l + m_r, Some(width));
                (r.width - m_l - m_r, r.height)
            }
        };
        let mut items = core::mem::replace(&mut self.items, saved_items);
        let links = core::mem::replace(&mut self.links, saved_links);
        for i in &mut items { i.offset(m_l, m_t); }
        let mut links = links;
        for l in &mut links { l.x += m_l; l.y += m_t; }
        if s.hidden { items.clear(); }
        Some(Piece::Atomic { items, links, width: w + m_l + m_r, height: h + m_t + m_b, link })
    }

    fn control(&mut self, n: NodeId, s: &Style, tag: &str) -> (i32, i32) {
        let font = font_of(s);
        let kind = self.dom.attr(n, "type").unwrap_or("text").to_ascii_lowercase();
        if matches!(kind.as_str(), "checkbox" | "radio") {
            let size = 13;
            self.items.push(Item::Rect { x: 0, y: 0, w: size, h: size, color: 0xFF8E_918F, radius: if kind == "radio" { 7 } else { 2 } });
            self.items.push(Item::Rect { x: 1, y: 1, w: size - 2, h: size - 2, color: 0xFFFF_FFFF, radius: if kind == "radio" { 6 } else { 1 } });
            if self.dom.attr(n, "checked").is_some() {
                self.items.push(Item::Rect { x: 3, y: 3, w: size - 6, h: size - 6, color: 0xFF0B_57D0, radius: if kind == "radio" { 4 } else { 1 } });
            }
            return (size, size);
        }
        let (label, faint) = match tag {
            "select" => {
                let opt = self.dom.element_children(n).find(|&o| self.dom.attr(o, "selected").is_some()).or_else(|| self.dom.element_children(n).next());
                (opt.map(|o| collapse(&self.dom.text_content(o))).unwrap_or_default() + " ▾", false)
            }
            "textarea" => (self.dom.text_content(n), false),
            _ => match kind.as_str() {
                "submit" | "button" | "reset" => (String::from(self.dom.attr(n, "value").unwrap_or(if kind == "reset" { "Reset" } else { "Submit" })), false),
                "password" => (self.dom.attr(n, "value").map(|v| v.chars().map(|_| '•').collect()).unwrap_or_default(), false),
                _ => match self.dom.attr(n, "value").filter(|v| !v.is_empty()) {
                    Some(v) => (String::from(v), false),
                    None => (String::from(self.dom.attr(n, "placeholder").unwrap_or("")), true),
                },
            },
        };
        let (asc, desc) = self.host.metrics(font);
        let pad = s.padding[LEFT].or_zero(0).max(2);
        let button = matches!(kind.as_str(), "submit" | "button" | "reset") || tag == "select";
        let chars: i32 = self.dom.attr(n, "size").and_then(|v| v.trim().parse().ok()).unwrap_or(20);
        let text_w = self.host.text_width(&label, font);
        let inner_w = match s.width.resolve(0) { Some(w) => w, None => if button { text_w } else { (chars * s.font_size / 2).max(text_w.min(400)) } };
        let rows = if tag == "textarea" { self.dom.attr(n, "rows").and_then(|v| v.trim().parse().ok()).unwrap_or(2) } else { 1 };
        let inner_h = match s.height.resolve(0) { Some(h) => h, None => (asc + desc + 2) * rows };
        let (w, h) = (inner_w + 2 * pad + 2, inner_h + 6);
        self.items.push(Item::Rect { x: 0, y: 0, w, h, color: s.border_color[TOP] | 0xFF00_0000, radius: s.radius });
        self.items.push(Item::Rect { x: 1, y: 1, w: w - 2, h: h - 2, color: if s.background >> 24 != 0 { s.background } else if button { 0xFFF1_F3F4 } else { 0xFFFF_FFFF }, radius: (s.radius - 1).max(0) });
        if !label.is_empty() {
            let color = if faint { 0xFF80_8080 } else { s.color };
            let first = label.lines().next().unwrap_or("");
            self.items.push(Item::Text { x: 1 + pad, baseline: 3 + asc + 1, font, color, text: String::from(first), underline: false, strike: false });
        }
        (w, h)
    }

    /// Breaks pieces into lines, positions them and emits items. Returns height and first baseline.
    fn lines(&mut self, cs: &Style, pieces: Vec<Piece>, x0: i32, y0: i32, width: i32) -> (i32, Option<i32>) {
        let strut_font = font_of(cs);
        let (sa, sd) = self.host.metrics(strut_font);
        let strut_lh = cs.line_height();
        let wraps = !matches!(cs.white_space, WhiteSpace::Pre | WhiteSpace::NoWrap);
        let mut y = y0;
        let mut first_baseline = None;
        let mut start = 0;
        let mut pieces: Vec<Option<Piece>> = pieces.into_iter().map(Some).collect();
        while start < pieces.len() {
            // Skip breakable spaces at the start of a line.
            while start < pieces.len() && matches!(pieces[start], Some(Piece::Space { breakable: true, .. })) { start += 1; }
            if start >= pieces.len() { break; }
            let mut end = start;
            let mut used = 0;
            let mut last_break: Option<usize> = None;
            while end < pieces.len() {
                let p = pieces[end].as_ref().unwrap();
                if matches!(p, Piece::Break) { break; }
                let pw = p.width();
                if let Piece::Space { breakable: true, .. } = p { last_break = Some(end); }
                if wraps && used + pw > width && end > start && !matches!(p, Piece::Space { .. }) {
                    if let Some(b) = last_break { end = b; }
                    break;
                }
                used += pw;
                end += 1;
            }
            let forced = end < pieces.len() && matches!(pieces[end], Some(Piece::Break));
            // Trailing spaces do not count.
            let mut trim = end;
            while trim > start && matches!(pieces[trim - 1], Some(Piece::Space { .. })) { trim -= 1; }
            let line_w: i32 = pieces[start..trim].iter().map(|p| p.as_ref().unwrap().width()).sum();
            // Vertical metrics: half-leading around each piece.
            let mut above = (strut_lh - (sa + sd)) / 2 + sa;
            let mut below = strut_lh - above;
            if start == trim { above = 0; below = 0; if forced { above = (strut_lh - (sa + sd)) / 2 + sa; below = strut_lh - above; } }
            for p in pieces[start..trim].iter().flatten() {
                match p {
                    Piece::Word { font, shift, .. } => {
                        let (a, d) = self.host.metrics(*font);
                        let lh = if cs.line_height > 0 && font.size == cs.font_size { cs.line_height } else { font.size * cs.line_height().max(1) / cs.font_size.max(1) };
                        let lh = lh.max(a + d);
                        let hl = (lh - (a + d)) / 2;
                        above = above.max(a + hl - shift);
                        below = below.max(d + hl + shift);
                    }
                    Piece::Atomic { height, .. } => above = above.max(*height),
                    _ => {}
                }
            }
            let baseline = y + above;
            if first_baseline.is_none() { first_baseline = Some(baseline); }
            let mut x = x0 + match cs.align {
                Align::Center => (width - line_w) / 2,
                Align::Right => width - line_w,
                _ => 0,
            }.max(if cs.align == Align::Left || cs.align == Align::Justify { 0 } else { i32::MIN / 2 });
            if cs.align != Align::Left && line_w > width { x = x0; }
            let line_top = y;
            let line_h = above + below;
            let mut run: Option<(i32, String, FontKey, u32, bool, bool, Option<NodeId>, u32, i32, i32)> = None;
            let flush = |this: &mut Self, run: &mut Option<(i32, String, FontKey, u32, bool, bool, Option<NodeId>, u32, i32, i32)>| {
                if let Some((rx, text, font, color, ul, st, link, bg, w, shift)) = run.take() {
                    if bg >> 24 != 0 { this.items.push(Item::Rect { x: rx, y: line_top + (line_h - font.size * 5 / 4) / 2, w, h: font.size * 5 / 4, color: bg, radius: 2 }); }
                    if color >> 24 != 0 { this.items.push(Item::Text { x: rx, baseline: baseline + shift, font, color, text, underline: ul, strike: st }); }
                    if let Some(l) = link { this.add_link(l, rx, line_top, w, line_h); }
                }
            };
            for i in start..trim {
                let p = pieces[i].take().unwrap();
                match p {
                    Piece::Word { text, font, color, underline, strike, link, bg, width: pw, shift } => {
                        let same = run.as_ref().map_or(false, |r| r.2 == font && r.3 == color && r.4 == underline && r.5 == strike && r.6 == link && r.7 == bg && r.9 == shift && r.0 + r.8 == x);
                        if same {
                            let r = run.as_mut().unwrap();
                            r.1.push_str(&text);
                            r.8 += pw;
                        } else {
                            flush(self, &mut run);
                            run = Some((x, text, font, color, underline, strike, link, bg, pw, shift));
                        }
                        x += pw;
                    }
                    Piece::Space { width: sw, font, color, underline, link, bg, .. } => {
                        let same = run.as_ref().map_or(false, |r| r.2 == font && r.3 == color && r.6 == link && r.7 == bg && r.0 + r.8 == x);
                        if same && sw > 0 {
                            let r = run.as_mut().unwrap();
                            r.1.push(' ');
                            r.8 += sw;
                            r.4 &= underline;
                        } else {
                            flush(self, &mut run);
                        }
                        x += sw;
                    }
                    Piece::Gap(g) => { flush(self, &mut run); x += g; }
                    Piece::Anchor(id) => self.anchors.push((id, line_top)),
                    Piece::Atomic { mut items, links, width: aw, height: ah, link } => {
                        flush(self, &mut run);
                        let top = baseline - ah;
                        for it in &mut items { it.offset(x, top); }
                        self.items.extend(items);
                        for mut l in links { l.x += x; l.y += top; self.links.push(l); }
                        if let Some(l) = link { self.add_link(l, x, top, aw, ah); }
                        x += aw;
                    }
                    Piece::Break => {}
                }
            }
            flush(self, &mut run);
            y += line_h;
            start = if forced { end + 1 } else { end.max(start + 1) };
        }
        (y - y0, first_baseline)
    }

    fn add_link(&mut self, a: NodeId, x: i32, y: i32, w: i32, h: i32) {
        let href = String::from(self.dom.attr(a, "href").unwrap_or(""));
        // Join with the previous rectangle of the same link on the same line.
        if let Some(last) = self.links.last_mut() {
            if last.node == a && last.y == y && last.h == h && last.x + last.w >= x - 8 && last.x <= x { last.w = x + w - last.x; return; }
        }
        self.links.push(Link { x, y, w, h, href, node: a });
    }

    fn table_rows(&self, table: NodeId) -> Vec<NodeId> {
        let mut rows = Vec::new();
        for c in self.dom.element_children(table) {
            match self.display(c) {
                Display::TableRow => rows.push(c),
                Display::None => {}
                _ if matches!(self.dom.tag(c), Some("thead" | "tbody" | "tfoot")) => {
                    for r in self.dom.element_children(c) { if self.display(r) == Display::TableRow { rows.push(r); } }
                }
                _ => {}
            }
        }
        rows
    }

    fn row_cells(&self, row: NodeId) -> Vec<NodeId> {
        self.dom.element_children(row).filter(|&c| self.display(c) != Display::None).collect()
    }

    fn colspan(&self, cell: NodeId) -> usize {
        self.dom.attr(cell, "colspan").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 64)
    }

    /// Column widths for the automatic table layout.
    fn columns(&mut self, rows: &[NodeId], width: i32, spacing: i32, fixed_width: bool) -> Vec<i32> {
        let mut mins: Vec<i32> = Vec::new();
        let mut maxs: Vec<i32> = Vec::new();
        let mut fixed: Vec<Option<i32>> = Vec::new();
        let mut spans = Vec::new();
        for &r in rows {
            let mut col = 0;
            for c in self.row_cells(r) {
                let span = self.colspan(c);
                let (mn, mx) = self.intrinsic(c);
                while mins.len() < col + span { mins.push(0); maxs.push(0); fixed.push(None); }
                if span == 1 {
                    mins[col] = mins[col].max(mn);
                    maxs[col] = maxs[col].max(mx);
                    if let Some(Len::Px(w)) = self.style(c).map(|s| s.width) {
                        let (_, bp, _) = sum_h(self.style(c).unwrap(), 0);
                        fixed[col] = Some(fixed[col].unwrap_or(0).max(w + bp));
                    }
                } else {
                    spans.push((col, span, mn, mx));
                }
                col += span;
            }
        }
        for (col, span, mn, mx) in spans {
            let have_min: i32 = mins[col..col + span].iter().sum();
            let have_max: i32 = maxs[col..col + span].iter().sum();
            if mn > have_min { let add = (mn - have_min) / span as i32; for m in &mut mins[col..col + span] { *m += add; } }
            if mx > have_max { let add = (mx - have_max) / span as i32; for m in &mut maxs[col..col + span] { *m += add; } }
        }
        let n = mins.len();
        if n == 0 { return Vec::new(); }
        for i in 0..n { if let Some(f) = fixed[i] { mins[i] = mins[i].max(f.min(mins[i].max(f))); maxs[i] = f.max(mins[i]); } maxs[i] = maxs[i].max(mins[i]); }
        let avail = width - spacing * (n as i32 + 1);
        let sum_min: i32 = mins.iter().sum();
        let sum_max: i32 = maxs.iter().sum();
        let mut cols: Vec<i32> = if sum_max <= avail {
            let mut c = maxs.clone();
            if fixed_width && sum_max > 0 {
                // Spread the spare width over the auto columns, proportionally to max-content.
                let spare = avail - sum_max;
                let auto: i32 = (0..n).filter(|&i| fixed[i].is_none()).map(|i| maxs[i]).sum();
                for i in 0..n {
                    if auto > 0 { if fixed[i].is_none() { c[i] += (spare as i64 * maxs[i] as i64 / auto as i64) as i32; } }
                    else { c[i] += (spare as i64 * maxs[i] as i64 / sum_max as i64) as i32; }
                }
            }
            c
        } else if sum_min >= avail {
            mins.clone()
        } else {
            let spare = avail - sum_min;
            let range = (sum_max - sum_min).max(1);
            (0..n).map(|i| mins[i] + (spare as i64 * (maxs[i] - mins[i]) as i64 / range as i64) as i32).collect()
        };
        for c in &mut cols { *c = (*c).max(0); }
        cols
    }

    fn table(&mut self, n: NodeId, cx: i32, cy: i32, cw: i32) -> (i32, Option<i32>) {
        let s = self.style(n).unwrap();
        let spacing = if s.border_collapse { 0 } else {
            s.border_spacing.or_else(|| self.dom.attr(n, "cellspacing").and_then(|v| v.trim().parse().ok())).unwrap_or(2)
        };
        let mut y = cy;
        let mut first = None;
        // Captions first.
        for c in self.dom.element_children(n).collect::<Vec<_>>() {
            if self.dom.tag(c) == Some("caption") && self.display(c) != Display::None {
                let r = self.block(c, cx, y, cw, None);
                y += r.height + r.margin_top + r.margin_bottom;
            }
        }
        let rows = self.table_rows(n);
        let fixed_width = s.width != Len::Auto;
        let cols = self.columns(&rows, cw, spacing, fixed_width);
        y += spacing;
        for r in rows {
            let bg_index = self.items.len();
            let rs = self.style(r).unwrap();
            if rs.background >> 24 != 0 { self.items.push(Item::Rect { x: 0, y: 0, w: 0, h: 0, color: 0, radius: 0 }); }
            let cells = self.row_cells(r);
            let mut x = cx + spacing;
            let mut col = 0;
            let mut row_h = 0;
            let mut placed = Vec::new();
            for c in cells {
                let span = self.colspan(c);
                let end = (col + span).min(cols.len());
                let w: i32 = cols[col.min(cols.len())..end].iter().sum::<i32>() + spacing * (end.saturating_sub(col) as i32 - 1).max(0);
                let start_items = self.items.len();
                let start_links = self.links.len();
                let res = self.block(c, x, y, w, Some(w));
                if first.is_none() { first = res.first_baseline; }
                row_h = row_h.max(res.height);
                placed.push((c, x, w, res.height, start_items..self.items.len(), start_links..self.links.len()));
                x += w + spacing;
                col = end;
            }
            // Cells stretch to the row height; middle vertical alignment by default.
            for (c, x, w, h, item_range, link_range) in placed {
                let start_items = item_range.start;
                let cs = self.style(c).unwrap();
                let valign = self.dom.attr(c, "valign").map(|v| v.to_ascii_lowercase()).or_else(|| self.dom.attr(r, "valign").map(|v| v.to_ascii_lowercase()));
                let dy = match valign.as_deref() { Some("top") => 0, Some("bottom") => row_h - h, _ => (row_h - h) / 2 };
                if dy != 0 {
                    for it in &mut self.items[item_range.clone()] { it.offset(0, dy); }
                    for l in &mut self.links[link_range] { l.y += dy; }
                }
                // Grow the cell's own background to the full row.
                if dy != 0 || h != row_h {
                    if let Some(Item::Rect { x: rx, y: ry, w: rw, h: rh, .. }) = self.items.get_mut(start_items) {
                        if *rx == x && *rw == w && cs.background >> 24 != 0 { *ry = y; *rh = row_h; }
                    }
                }
            }
            if rs.background >> 24 != 0 { self.items[bg_index] = Item::Rect { x: cx, y, w: cw, h: row_h, color: rs.background, radius: 0 }; }
            y += row_h + spacing;
        }
        (y - cy, first)
    }

    /// A lone `display: table-row` outside a table: cells side by side.
    fn row(&mut self, n: NodeId, cx: i32, cy: i32, cw: i32) -> (i32, Option<i32>) {
        let cols = self.columns(&[n], cw, 0, true);
        let mut x = cx;
        let mut h = 0;
        let mut first = None;
        for (i, c) in self.row_cells(n).into_iter().enumerate() {
            let w = *cols.get(i).unwrap_or(&0);
            let r = self.block(c, x, cy, w, Some(w));
            if first.is_none() { first = r.first_baseline; }
            h = h.max(r.height);
            x += w;
        }
        (h, first)
    }

    fn flex_row(&mut self, n: NodeId, cx: i32, cy: i32, cw: i32) -> (i32, Option<i32>) {
        let s = self.style(n).unwrap();
        let gap = s.gap;
        // Each child becomes a flex item; loose text becomes an anonymous one.
        let mut items: Vec<(NodeId, i32, i32, i32)> = Vec::new(); // node, base width (margin box), grow, min
        for c in self.flat_children(n) {
            match &self.dom.nodes[c].kind {
                Kind::Text(t) if t.trim().is_empty() => continue,
                Kind::Element { .. } if self.display(c) == Display::None => continue,
                _ => {}
            }
            let (min, max) = self.intrinsic(c);
            let (grow, base) = match self.style(c).filter(|_| matches!(self.dom.nodes[c].kind, Kind::Element { .. })) {
                Some(cs) => {
                    let (m, bp, _) = sum_h(cs, cw);
                    let size = if cs.flex_basis != Len::Auto { cs.flex_basis } else { cs.width };
                    let basis = size.resolve(cw).map(|w| if cs.box_sizing_border { w.max(bp) + m } else { w + bp + m });
                    let mut base = basis.unwrap_or(max);
                    if let Some(mw) = cs.min_width.resolve(cw) { base = base.max(if cs.box_sizing_border { mw + m } else { mw + bp + m }); }
                    (cs.flex_grow, base)
                }
                None => (0, max),
            };
            items.push((c, base, grow, min));
        }
        // Lines: wrap when allowed, otherwise one line.
        let mut lines: Vec<Vec<(NodeId, i32, i32, i32)>> = Vec::new();
        let mut line = Vec::new();
        let mut used = 0;
        for it in items {
            if s.flex_wrap && !line.is_empty() && used + gap + it.1 > cw { lines.push(core::mem::take(&mut line)); used = 0; }
            used += if line.is_empty() { it.1 } else { gap + it.1 };
            line.push(it);
        }
        if !line.is_empty() { lines.push(line); }
        let mut y = cy;
        let mut first = None;
        for mut line in lines {
            let count = line.len() as i32;
            let total: i32 = line.iter().map(|i| i.1).sum::<i32>() + gap * (count - 1).max(0);
            let mut free = cw - total;
            if free < 0 {
                // Shrink towards min-content, proportionally to base size.
                let shrinkable: i32 = line.iter().map(|i| (i.1 - i.3).max(0)).sum();
                if shrinkable > 0 {
                    let cut = (-free).min(shrinkable);
                    for it in &mut line { let room = (it.1 - it.3).max(0); it.1 -= (cut as i64 * room as i64 / shrinkable as i64) as i32; }
                }
                free = 0;
            }
            let grow_total: i32 = line.iter().map(|i| i.2).sum();
            if grow_total > 0 && free > 0 {
                for it in &mut line { it.1 += (free as i64 * it.2 as i64 / grow_total as i64) as i32; }
                free = 0;
            }
            let (mut x, between) = match s.justify {
                Justify::Start => (cx, gap),
                Justify::End => (cx + free, gap),
                Justify::Center => (cx + free / 2, gap),
                Justify::SpaceBetween => (cx, gap + if count > 1 { free / (count - 1) } else { 0 }),
                Justify::SpaceAround => (cx + free / (2 * count.max(1)), gap + free / count.max(1)),
                Justify::SpaceEvenly => (cx + free / (count + 1), gap + free / (count + 1)),
            };
            let mut line_h = 0;
            let mut placed = Vec::new();
            for (c, w, _, _) in line {
                let start_items = self.items.len();
                let start_links = self.links.len();
                let h = if matches!(self.dom.nodes[c].kind, Kind::Text(_)) {
                    self.inline_context(n, &[c], x, y, w).map_or(0, |(h, b)| { if first.is_none() { first = b; } h })
                } else {
                    let cs = self.style(c).unwrap();
                    let (m, _, _) = sum_h(cs, cw);
                    let mt = cs.margin[TOP].or_zero(cw);
                    let r = self.block(c, x, y + mt, w, Some(w - m));
                    if first.is_none() { first = r.first_baseline; }
                    r.height + mt + r.margin_bottom
                };
                line_h = line_h.max(h);
                placed.push((h, start_items..self.items.len(), start_links..self.links.len()));
                x += w + between;
            }
            if s.align_center {
                for (h, si, sl) in placed {
                    let dy = (line_h - h) / 2;
                    if dy > 0 { for it in &mut self.items[si] { it.offset(0, dy); } for l in &mut self.links[sl] { l.y += dy; } }
                }
            }
            y += line_h + if s.flex_wrap { gap } else { 0 };
        }
        (y - cy, first)
    }

    /// (min-content, max-content) border-box widths, margins included.
    fn intrinsic(&mut self, n: NodeId) -> (i32, i32) {
        if let Some(v) = self.intrinsic[n] { return v; }
        let v = self.compute_intrinsic(n);
        self.intrinsic[n] = Some(v);
        v
    }

    fn compute_intrinsic(&mut self, n: NodeId) -> (i32, i32) {
        let parent_style = |this: &Self, n: NodeId| { let mut p = this.dom.nodes[n].parent; while let Some(x) = p { if let Some(s) = this.style(x) { return s; } p = this.dom.nodes[x].parent; } unreachable!() };
        match &self.dom.nodes[n].kind {
            Kind::Document => (0, 0),
            Kind::Text(t) => {
                let s = parent_style(self, n);
                let font = font_of(s);
                if matches!(s.white_space, WhiteSpace::Pre | WhiteSpace::PreWrap) {
                    let w = t.split('\n').map(|l| self.host.text_width(&expand_tabs(l), font)).max().unwrap_or(0);
                    return (if s.white_space == WhiteSpace::Pre { w } else { 0 }, w);
                }
                let space = self.host.text_width(" ", font);
                let mut min = 0;
                let mut max = 0;
                let mut cap = true;
                let mut count = 0;
                for w in t.split_ascii_whitespace() {
                    let w = transform(w, s.transform, &mut cap);
                    let ww = self.host.text_width(&w, font);
                    min = min.max(ww);
                    max += ww + if count > 0 { space } else { 0 };
                    count += 1;
                }
                if count > 0 && t.starts_with(|c: char| c.is_ascii_whitespace()) { max += space; }
                if s.white_space == WhiteSpace::NoWrap { min = max; }
                (min, max)
            }
            Kind::Element { tag, .. } => {
                let Some(s) = self.style(n) else { return (0, 0) };
                if s.display == Display::None { return (0, 0); }
                let (m, bp, _) = sum_h(s, 0);
                if let Some(w) = s.width.resolve(0).filter(|_| matches!(s.width, Len::Px(_))) {
                    let w = if s.box_sizing_border { w + m } else { w + bp + m };
                    return (w, w);
                }
                let tag = tag.as_str();
                if matches!(tag, "img" | "svg" | "canvas" | "video" | "iframe" | "object" | "embed") {
                    let src = self.dom.attr(n, "src").unwrap_or("");
                    let w = match (s.height.resolve(0), self.host.image_size(src)) {
                        (Some(h), Some((nw, nh))) if nh > 0 => h * nw / nh,
                        (_, Some((nw, _))) => nw,
                        _ => if tag == "img" { 0 } else { 300 },
                    };
                    let w = w + m + bp;
                    return (w, w);
                }
                if matches!(tag, "input" | "select" | "textarea") {
                    let saved = core::mem::take(&mut self.items);
                    let (w, _) = self.control(n, s, tag);
                    self.items = saved;
                    return (w + m, w + m);
                }
                if tag == "br" { return (0, 0); }
                let mut min = 0;
                let mut max: i32 = 0;
                let mut run = 0;
                let children: Vec<NodeId> = self.dom.nodes[n].children.clone();
                let row_like = matches!(s.display, Display::TableRow) || (s.display == Display::Flex && !s.flex_column);
                if s.display == Display::Table {
                    let rows = self.table_rows(n);
                    for r in rows {
                        let (mut rmin, mut rmax) = (0, 0);
                        for c in self.row_cells(r) { let (a, b) = self.intrinsic(c); rmin += a + 2; rmax += b + 2; }
                        min = min.max(rmin);
                        max = max.max(rmax);
                    }
                    return (min + bp + m + 2, max + bp + m + 2);
                }
                let mut count = 0;
                for c in children {
                    let (cmin, cmax) = self.intrinsic(c);
                    if row_like {
                        if matches!(&self.dom.nodes[c].kind, Kind::Text(t) if t.trim().is_empty()) { continue; }
                        min += cmin;
                        max += cmax + if count > 0 { s.gap } else { 0 };
                        count += 1;
                        continue;
                    }
                    min = min.max(cmin);
                    if self.is_block(c) {
                        max = max.max(run).max(cmax);
                        run = 0;
                    } else {
                        run += cmax;
                    }
                }
                max = max.max(run);
                if s.white_space == WhiteSpace::NoWrap && !row_like { min = max; }
                (min + bp + m, max + bp + m)
            }
        }
    }
}

fn is_space(b: u8) -> bool { matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c') }

fn expand_tabs(s: &str) -> String {
    let mut out = String::new();
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' { let n = 8 - col % 8; for _ in 0..n { out.push(' '); } col += n; } else { out.push(c); col += 1; }
    }
    out
}

fn transform(w: &str, t: Transform, cap: &mut bool) -> String {
    let w = w.replace('\u{A0}', " ");
    match t {
        Transform::None => w,
        Transform::Upper => w.to_uppercase(),
        Transform::Lower => w.to_lowercase(),
        Transform::Capitalize => {
            let mut out = String::new();
            for c in w.chars() {
                if *cap && c.is_alphabetic() { out.extend(c.to_uppercase()); *cap = false; } else { out.push(c); }
            }
            *cap = true;
            out
        }
    }
}

fn itoa(n: i32) -> String { alloc::format!("{}", n) }

fn alpha(mut n: i32, upper: bool) -> String {
    if n <= 0 { return itoa(n); }
    let mut s = Vec::new();
    while n > 0 { n -= 1; s.push((if upper { b'A' } else { b'a' }) + (n % 26) as u8); n /= 26; }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

fn roman(mut n: i32, upper: bool) -> String {
    if !(1..4000).contains(&n) { return itoa(n); }
    let table = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut s = String::new();
    for (v, r) in table { while n >= v { s.push_str(r); n -= v; } }
    if upper { s.to_uppercase() } else { s }
}

/// Splits a [`Page`] item list into the ones that touch rows `top..bottom`.
pub fn visible(page: &Page, top: i32, bottom: i32) -> impl Iterator<Item = &Item> {
    page.items.iter().filter(move |i| { let (a, b) = i.vertical_span(); b > top && a < bottom })
}
