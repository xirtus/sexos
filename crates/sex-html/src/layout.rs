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
    /// Each laid-out element's border box (x, y, width, height), by node; an element
    /// split over lines (a link) has the box around all its parts.
    pub boxes: Vec<Option<[i32; 4]>>,
    /// Each node's paint layer: 0 for normal flow, higher for positioned boxes painted
    /// later (on top), so hit testing can prefer what is drawn on top.
    pub layers: Vec<u32>,
    /// Clip rectangles (x, y, width, height) for `overflow` other than `visible`; entry 0
    /// is unused.
    pub clips: Vec<[i32; 4]>,
    /// Each item's clip (an index into `clips`, 0 for none); empty when nothing clips.
    pub item_clips: Vec<u16>,
}

impl Page {
    /// The rectangle item `i` is clipped to, if any.
    pub fn clip_of(&self, i: usize) -> Option<[i32; 4]> {
        match self.item_clips.get(i) { Some(&c) if c > 0 => self.clips.get(c as usize).copied(), _ => None }
    }
    /// The link under a point, if any.
    pub fn link_at(&self, x: i32, y: i32) -> Option<&Link> {
        self.links.iter().find(|l| x >= l.x && x < l.x + l.w && y >= l.y && y < l.y + l.h)
    }
}

/// Lays out `dom` (already styled by `styles`) for a viewport `width` pixels wide.
pub fn layout(dom: &Dom, styles: &[Option<Style>], width: i32, host: &dyn Host) -> Page {
    layout_in(dom, styles, width, 0, host)
}

/// Lays out for a viewport `width` by `viewport_height` pixels: the box that
/// absolutely positioned and fixed elements without a positioned ancestor place
/// themselves in (0 takes the page's own height).
pub fn layout_in(dom: &Dom, styles: &[Option<Style>], width: i32, viewport_height: i32, host: &dyn Host) -> Page {
    let html = dom.find("html");
    let mut l = Layout { dom, styles, host, items: Vec::new(), links: Vec::new(), anchors: Vec::new(), intrinsic: alloc::vec![None; dom.nodes.len()], link_stack: Vec::new(), boxes: Vec::new(),
        abs: Vec::new(), positioned: Vec::new(), force_height: None, def_h: (viewport_height > 0).then_some(viewport_height), root: html, viewport: (width, viewport_height), floats: Vec::new(), float_base: 0 };
    let body = dom.find("body");
    let mut height = 0;
    if let Some(html) = html {
        let r = l.block(html, 0, 0, width, None);
        height = r.margin_top + r.height + r.margin_bottom;
    }
    let (mut tagged, clips) = resolve_clips(core::mem::take(&mut l.items));
    let (order, below) = layer_order(&mut tagged);
    let item_clips: Vec<u16> = if clips.len() > 1 { tagged.iter().map(|t| t.1).collect() } else { Vec::new() };
    l.items = tagged.into_iter().map(|t| t.0).collect();
    // Ranks in paint order: layers under normal flow, normal flow itself, then the rest.
    let flow_rank = below as u32;
    let mut rank: Vec<Option<u32>> = alloc::vec![None; dom.nodes.len()];
    for &(n, seq) in &l.positioned {
        if let Some(i) = order.iter().position(|&q| q == seq) { rank[n] = Some(if i < below { i as u32 } else { i as u32 + 1 }); }
    }
    let layers: Vec<u32> = (0..dom.nodes.len()).map(|n| {
        let mut p = Some(n);
        while let Some(q) = p { if let Some(r) = rank[q] { return r; } p = dom.nodes[q].parent; }
        flow_rank
    }).collect();
    let bg = |n: Option<NodeId>| n.and_then(|n| styles[n].as_ref()).map_or(0, |s| s.background);
    let background = match (bg(html), bg(body)) { (h, _) if h >> 24 != 0 => h, (_, b) if b >> 24 != 0 => b, _ => 0xFFFF_FFFF };
    // The root's background paints the whole canvas, not just its box.
    if let Some(Item::Rect { color, .. }) = l.items.iter_mut().find(|i| matches!(i, Item::Rect { .. })) { if *color == background { *color = 0; } }
    let title = dom.find("title").map(|t| collapse(&dom.text_content(t))).unwrap_or_default();
    let mut boxes: Vec<Option<[i32; 4]>> = alloc::vec![None; dom.nodes.len()];
    let mut put = |n: NodeId, r: [i32; 4]| if let Some(b) = boxes.get_mut(n) {
        *b = Some(match *b {
            None => r,
            Some(o) => { let (x0, y0) = (o[0].min(r[0]), o[1].min(r[1])); [x0, y0, (o[0] + o[2]).max(r[0] + r[2]) - x0, (o[1] + o[3]).max(r[1] + r[3]) - y0] }
        });
    };
    for (n, r) in l.boxes { put(n, r); }
    for k in &l.links { put(k.node, [k.x, k.y, k.w, k.h]); }
    Page { width, height, background: background | 0xFF00_0000, title, items: l.items, links: l.links, anchors: l.anchors, boxes, layers, clips, item_clips }
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
const REPLACED: &[&str] = &["img", "svg", "canvas", "video", "audio", "iframe", "object", "embed", "input", "select", "textarea"];

/// A 1-based (negative: from the end) grid line as a 0-based column index.
fn line_index(line: i32, n: usize) -> usize {
    let n = n as i32;
    (if line > 0 { line - 1 } else { n + 1 + line }).clamp(0, n) as usize
}

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
    boxes: Vec<(NodeId, [i32; 4])>,
    /// Absolutely positioned and fixed boxes waiting for their containing block, with
    /// their static position.
    abs: Vec<(NodeId, i32, i32)>,
    /// Positioned elements and their layer sequence numbers.
    positioned: Vec<(NodeId, u32)>,
    /// The border-box height the next block takes (an absolute box between top and bottom).
    force_height: Option<i32>,
    /// The definite height percentage heights resolve against: the containing block's
    /// content height when it has one (the viewport for the root), else none.
    def_h: Option<i32>,
    root: Option<NodeId>,
    viewport: (i32, i32),
    /// Floats placed so far (margin boxes); those from `float_base` on belong to the
    /// block formatting context being laid out.
    floats: Vec<FloatBox>,
    float_base: usize,
}

#[derive(Clone, Copy)]
struct FloatBox { left: bool, x: i32, y: i32, w: i32, h: i32 }

/// Layer markers: positioned boxes paint in their own layer, ordered by `z-index`.
/// A marker is a zero-size rectangle with `w` set to one of these; `h` holds the
/// z-index and `color` the sequence number. Markers move with the items between them
/// (into inline blocks, flex and table shifts) and are removed before painting.
const LAYER_BEGIN: i32 = i32::MIN;
const LAYER_BEGIN_AUTO: i32 = i32::MIN + 1;
const LAYER_END: i32 = i32::MIN + 2;
/// Clip markers, the same way: a box with `overflow` other than `visible` clips what is
/// between them to its padding box (x, y, `radius` wide, `h` high).
const CLIP_BEGIN: i32 = i32::MIN + 3;
const CLIP_END: i32 = i32::MIN + 4;

fn marker(item: &Item) -> Option<(i32, i32, u32)> {
    match *item { Item::Rect { w, h, color, .. } if w <= LAYER_END => Some((w, h, color)), _ => None }
}

/// Turns clip markers into a clip index on each item, and the clip rectangles (nested
/// clips intersect).
fn resolve_clips(items: Vec<Item>) -> (Vec<(Item, u16)>, Vec<[i32; 4]>) {
    let mut clips = alloc::vec![[0; 4]];
    let mut stack: Vec<u16> = Vec::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Item::Rect { x, y, w: CLIP_BEGIN, h, radius, .. } => {
                let mut r = [x, y, radius.max(0), h.max(0)];
                if let Some(&top) = stack.last() {
                    let o = clips[top as usize];
                    let (x0, y0) = (r[0].max(o[0]), r[1].max(o[1]));
                    let (x1, y1) = ((r[0] + r[2]).min(o[0] + o[2]), (r[1] + r[3]).min(o[1] + o[3]));
                    r = [x0, y0, (x1 - x0).max(0), (y1 - y0).max(0)];
                }
                if clips.len() < u16::MAX as usize { clips.push(r); }
                stack.push((clips.len() - 1) as u16);
            }
            Item::Rect { w: CLIP_END, .. } => { stack.pop(); }
            item => out.push((item, stack.last().copied().unwrap_or(0))),
        }
    }
    (out, clips)
}

/// Moves each layer's items after normal flow, ordered by stacking context and
/// z-index (negative z-indexes under normal flow), and drops the markers. Returns
/// the layers' sequence numbers in paint order and how many paint under normal flow.
fn layer_order(items: &mut Vec<(Item, u16)>) -> (Vec<u32>, usize) {
    if !items.iter().any(|i| marker(&i.0).is_some()) { return (Vec::new(), 0); }
    // Each layer: its sort key (the path of (z, seq) through enclosing stacking
    // contexts) and its items.
    let mut layers: Vec<(Vec<(i32, u32)>, Vec<(Item, u16)>)> = Vec::new();
    let mut flow: Vec<(Item, u16)> = Vec::new();
    // Open layers: index into `layers`, and whether it starts a stacking context.
    let mut open: Vec<(usize, bool)> = Vec::new();
    for item in core::mem::take(items) {
        match marker(&item.0) {
            Some((kind, z, seq)) if kind != LAYER_END => {
                let context = open.iter().rev().find(|o| o.1).map(|o| layers[o.0].0.clone()).unwrap_or_default();
                let mut key = context;
                key.push((if kind == LAYER_BEGIN { z } else { 0 }, seq));
                layers.push((key, Vec::new()));
                open.push((layers.len() - 1, kind == LAYER_BEGIN));
            }
            Some(_) => { open.pop(); }
            None => match open.last() { Some(&(i, _)) => layers[i].1.push(item), None => flow.push(item) },
        }
    }
    let mut order: Vec<usize> = (0..layers.len()).collect();
    order.sort_by(|&a, &b| layers[a].0.cmp(&layers[b].0));
    let seqs: Vec<u32> = order.iter().map(|&i| layers[i].0.last().unwrap().1).collect();
    let (below, above): (Vec<usize>, Vec<usize>) = order.into_iter().partition(|&i| layers[i].0[0].0 < 0);
    let below_count = below.len();
    let mut out = Vec::new();
    // The root's background stays first.
    let mut flow = flow.into_iter();
    if let Some(first) = flow.next() { out.push(first); }
    for i in below { out.append(&mut layers[i].1); }
    out.extend(flow);
    for i in above { out.append(&mut layers[i].1); }
    *items = out;
    (seqs, below_count)
}

/// One unbreakable thing on a line.
enum Piece {
    Word { text: String, font: FontKey, color: u32, underline: bool, strike: bool, link: Option<NodeId>, bg: u32, width: i32, shift: i32 },
    Space { width: i32, breakable: bool, font: FontKey, color: u32, underline: bool, link: Option<NodeId>, bg: u32 },
    Break,
    Atomic { items: Vec<Item>, links: Vec<Link>, boxes: Vec<(NodeId, [i32; 4])>, width: i32, height: i32, link: Option<NodeId> },
    Gap(i32),
    /// An `id` (or `<a name>`) inside the line: where a `#fragment` link scrolls to.
    Anchor(String),
    /// Where an inline element (a `span`, `em`...) starts and ends: its fragments
    /// on each line become its box, for hit testing.
    Open(NodeId),
    Close(NodeId),
    /// An absolutely positioned or fixed element: out of the line, but placed from here
    /// when it gives no offsets.
    Abs(NodeId),
}

impl Piece {
    fn width(&self) -> i32 {
        match self { Piece::Word { width, .. } | Piece::Space { width, .. } | Piece::Atomic { width, .. } | Piece::Gap(width) => *width, Piece::Break | Piece::Anchor(_) | Piece::Open(_) | Piece::Close(_) | Piece::Abs(_) => 0 }
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

    /// Absolutely positioned or fixed: out of normal flow.
    fn out_of_flow(&self, n: NodeId) -> bool {
        matches!(self.dom.nodes[n].kind, Kind::Element { .. }) && self.style(n).map_or(false, |s| s.display != Display::None && matches!(s.position, style::Position::Absolute | style::Position::Fixed))
    }

    /// Lays out a block-level box whose margin box starts at (`x`, `y`) inside a
    /// containing block `cw` wide. `force_width` sets the border-box width (flex items, cells).
    /// Positioned boxes get their own paint layer, relative ones are offset, and
    /// boxes positioned inside them are placed once their size is known.
    fn block(&mut self, n: NodeId, x: i32, y: i32, cw: i32, force_width: Option<i32>) -> BlockResult {
        let s = self.style(n).unwrap();
        if let Some((tx, ty)) = s.translate {
            let (i0, l0, b0, a0) = (self.items.len(), self.links.len(), self.boxes.len(), self.anchors.len());
            let r = self.block_faded(n, x, y, cw, force_width);
            let own_w = r.width - s.margin[LEFT].or_zero(cw) - s.margin[RIGHT].or_zero(cw);
            let (dx, dy) = (tx.or_zero(own_w), ty.or_zero(r.height));
            self.shift(i0, l0, b0, a0, dx, dy);
            return r;
        }
        self.block_faded(n, x, y, cw, force_width)
    }

    fn block_faded(&mut self, n: NodeId, x: i32, y: i32, cw: i32, force_width: Option<i32>) -> BlockResult {
        let s = self.style(n).unwrap();
        if s.opacity < 255 {
            // Everything the box paints fades with it (nested opacities multiply).
            let i0 = self.items.len();
            let r = self.block_positioned(n, x, y, cw, force_width);
            let a = s.opacity as u32;
            for it in &mut self.items[i0..] {
                match it {
                    Item::Rect { w, color, .. } if *w > CLIP_END => *color = (*color & 0x00FF_FFFF) | (((*color >> 24) * a / 255) << 24),
                    Item::Text { color, .. } | Item::Bullet { color, .. } => *color = (*color & 0x00FF_FFFF) | (((*color >> 24) * a / 255) << 24),
                    Item::Image { w, h, .. } if a < 128 => { *w = 0; *h = 0; }
                    _ => {}
                }
            }
            return r;
        }
        self.block_positioned(n, x, y, cw, force_width)
    }

    fn block_positioned(&mut self, n: NodeId, x: i32, y: i32, cw: i32, force_width: Option<i32>) -> BlockResult {
        let s = self.style(n).unwrap();
        let positioned = s.position != style::Position::Static;
        let is_root = self.root == Some(n);
        if !positioned && !is_root { return self.block_inner(n, x, y, cw, force_width); }
        let (i0, l0, b0, a0) = (self.items.len(), self.links.len(), self.boxes.len(), self.anchors.len());
        let abs_mark = self.abs.len();
        let seq = self.positioned.len() as u32;
        if positioned {
            self.positioned.push((n, seq));
            let (kind, z) = match s.z_index { Some(z) => (LAYER_BEGIN, z), None => (LAYER_BEGIN_AUTO, 0) };
            self.items.push(Item::Rect { x: 0, y: 0, w: kind, h: z, color: seq, radius: 0 });
        }
        let r = self.block_inner(n, x, y, cw, force_width);
        // Boxes positioned inside: this box's padding box holds them (the viewport, for
        // the root and for fixed boxes).
        let mut pending: Vec<(NodeId, i32, i32)> = self.abs.drain(abs_mark..).collect();
        let pad = self.boxes.iter().rev().find(|(b, _)| *b == n).map(|(_, b)| [b[0] + s.border[LEFT], b[1] + s.border[TOP], b[2] - s.border[LEFT] - s.border[RIGHT], b[3] - s.border[TOP] - s.border[BOTTOM]]);
        let view = [0, 0, self.viewport.0, if self.viewport.1 > 0 { self.viewport.1 } else { r.height }];
        // This box's overflow clip closed with its content; its absolute boxes (not fixed
        // ones) are clipped to the same padding box.
        let clip = (s.overflow_clip && !is_root).then_some(pad).flatten();
        let mut clipping = false;
        // Placing an absolute subtree can discover fixed descendants that bubble
        // back to the root. Visit them before the next sibling, rather than losing
        // the newly queued boxes after a one-shot drain. Each DOM node can be
        // placed at most once in a valid tree; cap even malformed work at that size.
        pending.reverse();
        for _ in 0..self.dom.nodes.len() {
            let Some((c, sx, sy)) = pending.pop() else { break };
            let fixed = self.style(c).map_or(false, |cs| cs.position == style::Position::Fixed);
            if is_root {
                self.place_abs(c, sx, sy, view);
                let next = pending.len();
                pending.extend(self.abs.drain(abs_mark..));
                pending[next..].reverse();
            }
            else if fixed { self.abs.push((c, sx, sy)); }
            else {
                if let (Some([px, py, pw, ph]), false) = (clip, clipping) {
                    self.items.push(Item::Rect { x: px, y: py, w: CLIP_BEGIN, h: ph, color: 0, radius: pw });
                    clipping = true;
                }
                self.place_abs(c, sx, sy, pad.unwrap_or(view));
            }
        }
        if clipping { self.items.push(Item::Rect { x: 0, y: 0, w: CLIP_END, h: 0, color: 0, radius: 0 }); }
        if matches!(s.position, style::Position::Relative | style::Position::Sticky) {
            let dx = s.inset[LEFT].resolve(cw).or_else(|| s.inset[RIGHT].resolve(cw).map(|v| -v)).unwrap_or(0);
            let dy = s.inset[TOP].resolve(0).or_else(|| s.inset[BOTTOM].resolve(0).map(|v| -v)).unwrap_or(0);
            if s.position == style::Position::Relative && (dx != 0 || dy != 0) { self.shift(i0, l0, b0, a0, dx, dy); }
        }
        if positioned { self.items.push(Item::Rect { x: 0, y: 0, w: LAYER_END, h: 0, color: seq, radius: 0 }); }
        r
    }

    /// Moves everything laid out since the given list lengths.
    fn shift(&mut self, i0: usize, l0: usize, b0: usize, a0: usize, dx: i32, dy: i32) {
        for it in &mut self.items[i0..] { it.offset(dx, dy); }
        for l in &mut self.links[l0..] { l.x += dx; l.y += dy; }
        for (_, b) in &mut self.boxes[b0..] { b[0] += dx; b[1] += dy; }
        for (_, y) in &mut self.anchors[a0..] { *y += dy; }
    }

    /// The free span [left, right) of the line from `x` to `x + w` between `y` and `y + h`,
    /// beside this formatting context's floats.
    fn band(&self, x: i32, w: i32, y: i32, h: i32) -> (i32, i32) {
        let (mut l, mut r) = (x, x + w);
        for f in &self.floats[self.float_base..] {
            if f.y >= y + h.max(1) || f.y + f.h <= y { continue; }
            if f.left { l = l.max(f.x + f.w); } else { r = r.min(f.x); }
        }
        (l, r)
    }

    /// Lays a float out and puts it against the left or right side as high as it fits,
    /// at `y` or below.
    fn place_float(&mut self, n: NodeId, cx: i32, cw: i32, y: i32) {
        let s = self.style(n).unwrap();
        let left = s.float == style::Float::Left;
        let (i0, l0, b0, a0) = (self.items.len(), self.links.len(), self.boxes.len(), self.anchors.len());
        let r = self.block(n, 0, 0, cw, None);
        let (fw, fh) = (r.width, r.margin_top + r.height + r.margin_bottom);
        // A float is no higher than the floats before it.
        let mut fy = self.floats[self.float_base..].iter().map(|f| f.y).fold(y, i32::max);
        if s.clear != 0 {
            if let Some(b) = self.floats[self.float_base..].iter().filter(|f| s.clear & if f.left { 1 } else { 2 } != 0).map(|f| f.y + f.h).max() { fy = fy.max(b); }
        }
        loop {
            let (l, rr) = self.band(cx, cw, fy, fh);
            if rr - l >= fw || (l == cx && rr == cx + cw) { break; }
            // Move below the first float in the way.
            let next = self.floats[self.float_base..].iter().filter(|f| f.y < fy + fh.max(1) && f.y + f.h > fy).map(|f| f.y + f.h).min();
            match next { Some(b) if b > fy => fy = b, _ => break }
        }
        let (l, rr) = self.band(cx, cw, fy, fh);
        let fx = if left { l } else { rr - fw };
        self.shift(i0, l0, b0, a0, fx, fy + r.margin_top);
        self.floats.push(FloatBox { left, x: fx, y: fy, w: fw, h: fh });
    }

    /// Places an absolutely positioned box in containing block `cb` (x, y, w, h) by its
    /// offsets, or at its static position (`sx`, `sy`) along an axis that has none.
    fn place_abs(&mut self, n: NodeId, sx: i32, sy: i32, cb: [i32; 4]) {
        let s = self.style(n).unwrap();
        let [cx, cy, cw, ch] = cb;
        let (ml, mr) = (s.margin[LEFT].or_zero(cw), s.margin[RIGHT].or_zero(cw));
        let (mt, mb) = (s.margin[TOP].or_zero(cw), s.margin[BOTTOM].or_zero(cw));
        let (l, r) = (s.inset[LEFT].resolve(cw), s.inset[RIGHT].resolve(cw));
        let (t, b) = (s.inset[TOP].resolve(ch), s.inset[BOTTOM].resolve(ch));
        let width = match (s.width, l, r) { (Len::Auto, Some(l), Some(r)) => Some((cw - l - r - ml - mr).max(0)), _ => None };
        if let (Len::Auto, Some(t), Some(b)) = (s.height, t, b) { self.force_height = Some((ch - t - b - mt - mb).max(0)); }
        let (i0, l0, b0, a0) = (self.items.len(), self.links.len(), self.boxes.len(), self.anchors.len());
        let x0 = l.map_or(sx, |l| cx + l);
        let saved_h = core::mem::replace(&mut self.def_h, Some(ch));
        let res = self.block(n, x0, 0, cw, width);
        self.def_h = saved_h;
        self.force_height = None;
        let dx = match (l, r) { (None, Some(r)) => cx + cw - r - res.width - x0, _ => 0 };
        let dy = match (t, b) {
            (Some(t), _) => cy + t + mt,
            (None, Some(b)) => cy + ch - b - mb - res.height,
            _ => sy + mt,
        };
        self.shift(i0, l0, b0, a0, dx, dy);
    }

    fn block_inner(&mut self, n: NodeId, x: i32, y: i32, cw: i32, force_width: Option<i32>) -> BlockResult {
        let forced_height = self.force_height.take();
        let s = self.style(n).unwrap();
        // A new block formatting context keeps its floats to itself and grows to hold them.
        let bfc = self.root == Some(n) || force_width.is_some() || s.float != style::Float::None || s.overflow_clip
            || matches!(s.position, style::Position::Absolute | style::Position::Fixed)
            || matches!(s.display, Display::InlineBlock | Display::Flex | Display::Grid | Display::Table | Display::TableCell);
        let saved_base = self.float_base;
        if bfc { self.float_base = self.floats.len(); }
        let r = self.block_box(n, x, y, cw, force_width, forced_height, bfc);
        if bfc { self.floats.truncate(self.float_base); self.float_base = saved_base; }
        r
    }

    fn block_box(&mut self, n: NodeId, x: i32, y: i32, cw: i32, force_width: Option<i32>, forced_height: Option<i32>, bfc: bool) -> BlockResult {
        let s = self.style(n).unwrap();
        if let Some(tag) = self.dom.tag(n).filter(|t| REPLACED.contains(t)) {
            // Pictures and controls keep their own size in block flow too.
            let mt = s.margin[TOP].or_zero(cw);
            let mb = s.margin[BOTTOM].or_zero(cw);
            let pushed = self.push_link(n);
            let piece = self.atomic(n, s, tag, cw);
            if pushed { self.link_stack.pop(); }
            if let Some(Piece::Atomic { mut items, links, boxes, width, height, link }) = piece {
                for it in &mut items { it.offset(x, y - mt); }
                self.items.extend(items);
                for (b, r) in boxes { self.boxes.push((b, [r[0] + x, r[1] + y - mt, r[2], r[3]])); }
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
        }
        let _ = mr;
        let bx = x + ml;
        let by = y;
        if let Some(id) = self.dom.attr(n, "id") { self.anchors.push((String::from(id), by)); }
        if let Some(name) = self.dom.attr(n, "name").filter(|_| self.dom.tag(n) == Some("a")) { self.anchors.push((String::from(name), by)); }
        let bg_index = self.items.len();
        let paints = s.background >> 24 != 0 || s.border.iter().any(|b| *b > 0) || s.shadow.is_some();
        if paints { self.items.push(Item::Rect { x: 0, y: 0, w: 0, h: 0, color: 0, radius: 0 }); }
        let clip_at = (s.overflow_clip && self.root != Some(n)).then(|| { self.items.push(Item::Rect { x: 0, y: 0, w: CLIP_BEGIN, h: 0, color: 0, radius: 0 }); self.items.len() - 1 });
        let cx = bx + s.border[LEFT] + s.padding[LEFT].or_zero(cw);
        let cy = by + s.border[TOP] + s.padding[TOP].or_zero(cw);
        // This box's own height when it is known before its content: forced, fixed, or a
        // percentage of a definite containing block. Its children's percentages use it.
        let vbp = s.border[TOP] + s.border[BOTTOM] + s.padding[TOP].or_zero(cw) + s.padding[BOTTOM].or_zero(cw);
        let own_h = match (forced_height, s.height, self.def_h) {
            (Some(h), _, _) => Some(h - vbp),
            (None, Len::Px(h), _) => Some(if s.box_sizing_border { h - vbp } else { h }),
            (None, Len::Pct(_), Some(ph)) => s.height.resolve(ph).map(|h| if s.box_sizing_border { h - vbp } else { h }),
            _ => None,
        }.map(|h| {
            let h = match s.min_height.resolve(0) { Some(min) => h.max(min), None => h };
            h.max(0)
        });
        // Items of flex, grid and table containers size in their own lines, areas and
        // rows, so a percentage height there stays auto rather than take the container's.
        let for_children = if matches!(s.display, Display::Flex | Display::Grid | Display::Table | Display::TableRow) { None } else { own_h };
        let saved_def_h = core::mem::replace(&mut self.def_h, for_children);
        let pushed_link = self.push_link(n);
        let (content_h, first_baseline) = match s.display {
            Display::Table => self.table(n, cx, cy, content_w),
            Display::TableRow => self.row(n, cx, cy, content_w),
            Display::Flex if !s.flex_column => self.flex_row(n, cx, cy, content_w),
            Display::Grid => self.grid(n, cx, cy, content_w),
            _ => self.flow(n, cx, cy, content_w),
        };
        let content_h = if bfc { self.floats[self.float_base..].iter().map(|f| f.y + f.h - cy).fold(content_h, i32::max) } else { content_h };
        if pushed_link { self.link_stack.pop(); }
        let vertical_bp = s.border[TOP] + s.border[BOTTOM] + s.padding[TOP].or_zero(cw) + s.padding[BOTTOM].or_zero(cw);
        self.def_h = saved_def_h;
        let mut inner_h = own_h.unwrap_or(content_h);
        if let Some(min) = s.min_height.resolve(0) { inner_h = inner_h.max(min); }
        inner_h = inner_h.max(0);
        let height = inner_h + s.border[TOP] + s.border[BOTTOM] + s.padding[TOP].or_zero(cw) + s.padding[BOTTOM].or_zero(cw);
        let before = self.items.len();
        if paints {
            self.box_paint(bg_index, s, bx, by, used, height);
        }
        if let Some(at) = clip_at {
            // The background and borders went in before the marker.
            let at = at + (self.items.len() - before);
            let (bl, bt) = (s.border[LEFT], s.border[TOP]);
            self.items[at] = Item::Rect { x: bx + bl, y: by + bt, w: CLIP_BEGIN, h: height - bt - s.border[BOTTOM], color: 0, radius: used - bl - s.border[RIGHT] };
            self.items.push(Item::Rect { x: 0, y: 0, w: CLIP_END, h: 0, color: 0, radius: 0 });
        }
        if s.display == Display::ListItem {
            if let Some(base) = first_baseline { self.marker(n, s, cx, base); }
        }
        if s.hidden { self.items.truncate(bg_index); }
        if !s.hidden { self.boxes.push((n, [bx, by, used, height])); }
        BlockResult { height, margin_top: mt, margin_bottom: mb, first_baseline, width: used + ml + mr }
    }

    /// Fills the placeholder at `index` with the background and pushes borders after it.
    fn box_paint(&mut self, index: usize, s: &Style, x: i32, y: i32, w: i32, h: i32) {
        let mut index = index;
        if let Some((dx, dy, blur, spread, color)) = s.shadow {
            // A blurred shadow as a few nested translucent layers, widest first.
            let layers = if blur > 0 { (blur / 3).clamp(2, 5) } else { 1 };
            let alpha = (color >> 24) / layers as u32;
            for k in 0..layers {
                let grow = spread + blur * (layers - k) / layers - blur / 2;
                let r = Item::Rect { x: x + dx - grow, y: y + dy - grow, w: w + 2 * grow, h: h + 2 * grow, color: (color & 0x00FF_FFFF) | (alpha.max(1) << 24), radius: (s.radius + grow.max(0)).max(0) };
                self.items.insert(index, r);
                index += 1;
            }
        }
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

    /// An inline `<svg>`'s own size: its viewBox (its width and height attributes are
    /// presentational hints already).
    fn svg_size(&self, n: NodeId) -> Option<(i32, i32)> {
        let v: Vec<f32> = self.dom.attr(n, "viewbox")?.split(|c: char| c == ',' || c.is_ascii_whitespace()).filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect();
        (v.len() == 4 && v[2] > 0.0 && v[3] > 0.0).then(|| (v[2] as i32, v[3] as i32))
    }

    /// A canvas's size from its `width` and `height` attributes (300 by 150 by default).
    fn canvas_size(&self, n: NodeId) -> (i32, i32) {
        let get = |name, default| self.dom.attr(n, name).and_then(|v| v.trim().parse::<i32>().ok()).filter(|v| *v >= 0).unwrap_or(default);
        (get("width", 300).min(4096), get("height", 150).min(4096))
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
            if c != usize::MAX && self.out_of_flow(c) {
                // Placed by its containing block; without offsets it sits where it would have.
                self.abs.push((c, cx, y + pending_margin.max(0)));
                continue;
            }
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
            if cs.float != style::Float::None {
                self.place_float(c, cx, cw, y + pending_margin.max(0));
                continue;
            }
            let mt = cs.margin[TOP].or_zero(cw);
            let collapsed = if first_block && pending_margin == 0 { mt } else if mt >= 0 && pending_margin >= 0 { mt.max(pending_margin) } else { mt + pending_margin };
            let mut top = y + collapsed + if first_block { 0 } else { gap };
            if cs.clear != 0 {
                let below = self.floats[self.float_base..].iter().filter(|f| cs.clear & if f.left { 1 } else { 2 } != 0).map(|f| f.y + f.h).max();
                if let Some(b) = below { if b > top { top = b; } }
            }
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
        for &n in run { self.collect(n, cs, &mut pieces, &mut space_before, w); }
        while matches!(pieces.last(), Some(Piece::Space { breakable: true, .. })) { pieces.pop(); }
        if pieces.iter().all(|p| matches!(p, Piece::Space { .. } | Piece::Anchor(_) | Piece::Open(_) | Piece::Close(_) | Piece::Abs(_))) {
            for p in pieces { match p { Piece::Anchor(id) => self.anchors.push((id, y)), Piece::Abs(n) => self.abs.push((n, x, y)), _ => {} } }
            return None;
        }
        Some(self.lines(cs, pieces, x, y, w))
    }

    fn collect(&mut self, n: NodeId, parent: &Style, out: &mut Vec<Piece>, space_before: &mut bool, cw: i32) {
        match &self.dom.nodes[n].kind {
            Kind::Text(t) => { let s = parent; self.text_pieces(t, s, out, space_before); }
            Kind::Document => {}
            Kind::Element { tag, .. } => {
                let Some(s) = self.style(n) else { return };
                if s.display == Display::None { return; }
                let tag = tag.as_str();
                if tag == "br" { out.push(Piece::Break); *space_before = true; return; }
                // Sound without controls takes no room; its children are fallback text.
                if tag == "audio" && self.dom.attr(n, "controls").is_none() { return; }
                if self.out_of_flow(n) { out.push(Piece::Abs(n)); return; }
                let atomic = matches!(tag, "img" | "input" | "select" | "textarea" | "button" | "svg" | "canvas" | "video" | "audio" | "iframe" | "object" | "embed" | "meter" | "progress")
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
                    if let Some(p) = self.atomic(n, s, tag, cw) { out.push(p); *space_before = false; }
                    return;
                }
                for name in ["id", "name"] {
                    if name == "name" && tag != "a" { continue; }
                    if let Some(id) = self.dom.attr(n, name) { out.push(Piece::Anchor(String::from(id))); }
                }
                let pushed = self.push_link(n);
                let boxed = !s.hidden;
                if boxed { out.push(Piece::Open(n)); }
                let left = s.margin[LEFT].or_zero(0) + s.padding[LEFT].or_zero(0) + s.border[LEFT];
                if left > 0 { out.push(Piece::Gap(left)); }
                for &c in &self.dom.nodes[n].children { self.collect(c, s, out, space_before, cw); }
                let right = s.margin[RIGHT].or_zero(0) + s.padding[RIGHT].or_zero(0) + s.border[RIGHT];
                if right > 0 { out.push(Piece::Gap(right)); }
                if boxed { out.push(Piece::Close(n)); }
                if pushed { self.link_stack.pop(); }
            }
        }
    }

    fn text_pieces(&mut self, text: &str, s: &Style, out: &mut Vec<Piece>, space_before: &mut bool) {
        if s.display == Display::None { return; }
        let font = font_of(s);
        let link = self.link_stack.last().copied();
        // Only inline boxes paint behind their text; a block's background is its box.
        let bg = if matches!(s.display, Display::Inline) { s.background } else { 0 };
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
    fn atomic(&mut self, n: NodeId, s: &Style, tag: &str, cw: i32) -> Option<Piece> {
        let link = self.link_stack.last().copied();
        let (m_l, m_r) = (s.margin[LEFT].or_zero(0), s.margin[RIGHT].or_zero(0));
        let (m_t, m_b) = (s.margin[TOP].or_zero(0), s.margin[BOTTOM].or_zero(0));
        let saved_items = core::mem::take(&mut self.items);
        let saved_links = core::mem::take(&mut self.links);
        let saved_boxes = core::mem::take(&mut self.boxes);
        let (w, h) = match tag {
            "img" | "svg" | "canvas" | "video" | "audio" | "iframe" | "object" | "embed" => {
                let inline_svg = tag == "svg" && self.dom.attr(n, "src").is_none();
                let src = if inline_svg { alloc::format!("svg:{n}") } else { self.picture_src(n, tag) };
                let natural = if tag == "canvas" { Some(self.canvas_size(n)) } else if inline_svg { self.svg_size(n) } else if tag == "audio" { Some((300, 32)) } else { self.host.image_size(&src) };
                // Percentages use the containing block, not the origin used to
                // prepare this atomic piece. An indefinite percentage height is
                // auto, so intrinsic aspect/default sizing still applies.
                let height = match s.height {
                    Len::Pct(_) => self.def_h.and_then(|h| s.height.resolve(h)),
                    _ => s.height.resolve(0),
                };
                let (w, h) = match (s.width.resolve(cw), height, natural) {
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
                // Atomic preparation happens at the origin, but percentages still
                // resolve against the real containing block's content width.
                let width = match s.width.resolve(cw) { Some(w) => if s.box_sizing_border { w } else { w + bp }, None => max.clamp(min, max.max(min)) };
                let r = self.block(n, 0, 0, width + m_l + m_r, Some(width));
                (r.width - m_l - m_r, r.height)
            }
        };
        let mut items = core::mem::replace(&mut self.items, saved_items);
        let links = core::mem::replace(&mut self.links, saved_links);
        let mut boxes = core::mem::replace(&mut self.boxes, saved_boxes);
        if !boxes.iter().any(|(b, _)| *b == n) { boxes.push((n, [0, 0, w, h])); }
        for (_, r) in &mut boxes { r[0] += m_l; r[1] += m_t; }
        if s.hidden { boxes.clear(); }
        for i in &mut items { i.offset(m_l, m_t); }
        let mut links = links;
        for l in &mut links { l.x += m_l; l.y += m_t; }
        if s.hidden { items.clear(); }
        Some(Piece::Atomic { items, links, boxes, width: w + m_l + m_r, height: h + m_t + m_b, link })
    }

    /// The picture a replaced element shows: canvases, videos and sound controls are
    /// drawn by the browser under `canvas:N`, `video:N` and `audio:N`.
    fn picture_src(&self, n: NodeId, tag: &str) -> String {
        match tag {
            "canvas" | "video" | "audio" => alloc::format!("{tag}:{n}"),
            _ => String::from(self.dom.attr(n, "src").unwrap_or("")),
        }
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
            _ => match kind.as_str() {
                "submit" | "button" | "reset" => (String::from(self.dom.attr(n, "value").unwrap_or(if kind == "reset" { "Reset" } else { "Submit" })), false),
                _ => match self.dom.control_value(n) {
                    v if v.is_empty() => (String::from(self.dom.attr(n, "placeholder").unwrap_or("")), true),
                    v if kind == "password" => (v.chars().map(|_| '•').collect(), false),
                    v => (v, false),
                },
            },
        };
        let caret = self.dom.caret.filter(|c| c.0 == n).map(|c| if faint { 0 } else { c.1 });
        let (asc, desc) = self.host.metrics(font);
        let pad = s.padding[LEFT].or_zero(0).max(2);
        let button = matches!(kind.as_str(), "submit" | "button" | "reset") || tag == "select";
        let chars: i32 = self.dom.attr(n, "size").and_then(|v| v.trim().parse().ok()).unwrap_or(20);
        let text_w = self.host.text_width(&label, font);
        let inner_w = match s.width.resolve(0) { Some(w) => w, None => if button { text_w } else { (chars * s.font_size / 2).max(text_w.min(400)) } };
        let rows = if tag == "textarea" { self.dom.attr(n, "rows").and_then(|v| v.trim().parse().ok()).unwrap_or(2) } else { 1 };
        let inner_h = match s.height.resolve(0) { Some(h) => h, None => (asc + desc + 2) * rows };
        let (w, h) = (inner_w + 2 * pad + 2, inner_h + 6);
        // A focused text control shows a blue ring.
        let border = if caret.is_some() { 0xFF0B_57D0 } else { s.border_color[TOP] | 0xFF00_0000 };
        self.items.push(Item::Rect { x: 0, y: 0, w, h, color: border, radius: s.radius });
        self.items.push(Item::Rect { x: 1, y: 1, w: w - 2, h: h - 2, color: if s.background >> 24 != 0 { s.background } else if button { 0xFFF1_F3F4 } else { 0xFFFF_FFFF }, radius: (s.radius - 1).max(0) });
        let color = if faint { 0xFF80_8080 } else { s.color };
        let line_h = asc + desc + 2;
        // Text controls show the lines around the caret, scrolled sideways to keep it in view.
        let (caret_line, caret_col) = match caret {
            Some(c) => {
                let before: String = label.chars().take(c).collect();
                (before.matches('\n').count(), before.rsplit('\n').next().map_or(0, |l| l.chars().count()))
            }
            None => (0, 0),
        };
        let shown = if tag == "textarea" { rows.max(1) as usize } else { 1 };
        let first_line = (caret_line + 1).saturating_sub(shown);
        for (i, line) in label.split('\n').enumerate().skip(first_line).take(shown) {
            let row = (i - first_line) as i32;
            let mut skip = 0;
            if caret.is_some() && i == caret_line {
                let upto: String = line.chars().take(caret_col).collect();
                let over = self.host.text_width(&upto, font) - (inner_w - 2);
                if over > 0 {
                    let mut cut = 0;
                    while cut < upto.len() && self.host.text_width(&upto[..cut], font) < over { cut += upto[cut..].chars().next().map_or(1, |c| c.len_utf8()); }
                    skip = cut;
                }
            }
            let text = &line[skip.min(line.len())..];
            let text: String = if self.host.text_width(text, font) <= inner_w { String::from(text) } else {
                let mut t = String::new();
                for ch in text.chars() { t.push(ch); if self.host.text_width(&t, font) > inner_w { t.pop(); break; } }
                t
            };
            if !text.is_empty() { self.items.push(Item::Text { x: 1 + pad, baseline: 3 + asc + 1 + row * line_h, font, color, text, underline: false, strike: false }); }
            if caret.is_some() && i == caret_line {
                let upto: String = line.chars().take(caret_col).collect();
                let cx = 1 + pad + self.host.text_width(&upto[skip.min(upto.len())..], font);
                self.items.push(Item::Rect { x: cx.min(w - 2), y: 3 + row * line_h, w: 1, h: asc + desc + 1, color: s.color | 0xFF00_0000, radius: 0 });
            }
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
        // Inline elements open on this line, and the x where their fragment starts.
        let mut open: Vec<(NodeId, i32)> = Vec::new();
        while start < pieces.len() {
            // Skip breakable spaces at the start of a line.
            while start < pieces.len() && matches!(pieces[start], Some(Piece::Space { breakable: true, .. })) { start += 1; }
            if start >= pieces.len() { break; }
            // Floats beside this line narrow it.
            let (x0, width) = { let (l, r) = self.band(x0, width, y, strut_lh); (l, (r - l).max(0)) };
            let mut end = start;
            let mut used = 0;
            let mut last_break: Option<usize> = None;
            while end < pieces.len() {
                let p = pieces[end].as_ref().unwrap();
                if matches!(p, Piece::Break) { break; }
                let pw = p.width();
                if let Piece::Space { breakable: true, .. } = p { last_break = Some(end); }
                if wraps && used + pw > width && end > start && !matches!(p, Piece::Space { .. } | Piece::Open(_) | Piece::Close(_) | Piece::Abs(_)) {
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
            let markers_only = pieces[start..trim].iter().flatten().all(|p| matches!(p, Piece::Open(_) | Piece::Close(_) | Piece::Abs(_)));
            if start == trim || (markers_only && !forced) { above = 0; below = 0; if forced { above = (strut_lh - (sa + sd)) / 2 + sa; below = strut_lh - above; } }
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
            for o in &mut open { o.1 = x; }
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
                    Piece::Open(n) => open.push((n, x)),
                    Piece::Abs(n) => self.abs.push((n, x, line_top)),
                    Piece::Close(n) => if let Some(i) = open.iter().rposition(|o| o.0 == n) {
                        let (_, sx) = open.remove(i);
                        if x > sx && line_h > 0 { self.boxes.push((n, [sx, line_top, x - sx, line_h])); }
                    },
                    Piece::Atomic { mut items, links, boxes, width: aw, height: ah, link } => {
                        flush(self, &mut run);
                        let top = baseline - ah;
                        for it in &mut items { it.offset(x, top); }
                        self.items.extend(items);
                        for (b, r) in boxes { self.boxes.push((b, [r[0] + x, r[1] + top, r[2], r[3]])); }
                        for mut l in links { l.x += x; l.y += top; self.links.push(l); }
                        if let Some(l) = link { self.add_link(l, x, top, aw, ah); }
                        x += aw;
                    }
                    Piece::Break => {}
                }
            }
            flush(self, &mut run);
            // Elements still open continue on the next line: this line holds a fragment.
            for &(n, sx) in &open { if x > sx && line_h > 0 { self.boxes.push((n, [sx, line_top, x - sx, line_h])); } }
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
                Kind::Element { .. } if self.out_of_flow(c) => { self.abs.push((c, cx, cy)); continue; }
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

    /// Column widths of a grid `cw` wide holding `count` items.
    fn grid_tracks(&mut self, s: &Style, cw: i32, items: &[(NodeId, style::GridPlace)]) -> Vec<i32> {
        use style::Track;
        let gap = s.gap;
        let mut tracks: Vec<Track> = s.grid_columns.clone();
        if let Some((t, fit)) = s.grid_auto {
            let min = match t { Track::Len(l) | Track::MinMax(l, _) => l.resolve(cw).unwrap_or(0), _ => 0 }.max(1);
            let fixed: i32 = tracks.iter().map(|t| match t { Track::Len(l) | Track::MinMax(l, _) => l.or_zero(cw) + gap, _ => gap }).sum();
            let mut n = ((cw - fixed + gap) / (min + gap)).clamp(1, 1000) as usize;
            if fit { n = n.min(items.len().saturating_sub(tracks.len()).max(1)); }
            for _ in 0..n { tracks.push(t); }
        }
        if tracks.is_empty() { tracks.push(Track::Fr(100)); }
        let n = tracks.len();
        // Auto tracks (and minmax(.., auto)) take the widest single-column item in them.
        let mut content = alloc::vec![(0i32, 0i32); n];
        let mut col = 0;
        for (c, g) in items {
            let span = g.span.max(1) as usize;
            let start = if g.start != 0 { line_index(g.start, n) } else { col };
            if span == 1 && g.end == 0 && start < n {
                let (min, max) = self.intrinsic(*c);
                content[start].0 = content[start].0.max(min);
                content[start].1 = content[start].1.max(max);
            }
            col = (start + span) % n.max(1);
        }
        let gaps = gap * (n as i32 - 1);
        let mut w: Vec<i32> = tracks.iter().enumerate().map(|(i, t)| match t {
            Track::Len(l) => l.or_zero(cw),
            Track::Auto => content[i].1,
            Track::MinMax(l, 0) => l.resolve(cw).unwrap_or(content[i].0).max(content[i].0),
            Track::MinMax(l, _) => l.resolve(cw).unwrap_or(content[i].0),
            Track::Fr(_) => 0,
        }).collect();
        let fr_total: i32 = tracks.iter().map(|t| match t { Track::Fr(f) | Track::MinMax(_, f) => *f, _ => 0 }).sum();
        let fixed: i32 = tracks.iter().zip(&w).filter(|(t, _)| !matches!(t, Track::Fr(_) | Track::MinMax(_, 1..))).map(|(_, w)| *w).sum();
        let auto: Vec<usize> = (0..n).filter(|&i| matches!(tracks[i], Track::Auto | Track::MinMax(_, 0))).collect();
        let mut free = cw - gaps - fixed;
        if fixed + gaps > cw && !auto.is_empty() {
            // Too wide: auto tracks give back down to their min-content.
            let over = fixed + gaps - cw;
            let room: i32 = auto.iter().map(|&i| (w[i] - content[i].0).max(0)).sum();
            if room > 0 { for &i in &auto { let r = (w[i] - content[i].0).max(0); w[i] -= (over.min(room) as i64 * r as i64 / room as i64) as i32; } }
            free = cw - gaps - w.iter().enumerate().filter(|(i, _)| !matches!(tracks[*i], Track::Fr(_) | Track::MinMax(_, 1..))).map(|(_, w)| *w).sum::<i32>();
        }
        if fr_total > 0 {
            // Flexible tracks share what is left, never below their minimum; a track held at
            // its minimum leaves the rest to the others.
            let mut flex: Vec<usize> = (0..n).filter(|&i| matches!(tracks[i], Track::Fr(_) | Track::MinMax(_, 1..))).collect();
            let mut left = free.max(0);
            loop {
                let total: i32 = flex.iter().map(|&i| match tracks[i] { Track::Fr(f) | Track::MinMax(_, f) => f, _ => 0 }).sum::<i32>().max(1);
                let share = |i: usize| (left as i64 * match tracks[i] { Track::Fr(f) | Track::MinMax(_, f) => f, _ => 0 } as i64 / total as i64) as i32;
                let pinned: Vec<usize> = flex.iter().copied().filter(|&i| share(i) < w[i]).collect();
                if pinned.is_empty() {
                    for &i in &flex { w[i] = share(i); }
                    // Rounding leftovers go to the last flexible track so the grid fills exactly.
                    if let Some(&last) = flex.last() { w[last] += left - flex.iter().map(|&i| w[i]).sum::<i32>(); }
                    break;
                }
                for i in pinned { left -= w[i]; flex.retain(|&j| j != i); }
                left = left.max(0);
                if flex.is_empty() { break; }
            }
        } else if free > 0 && !auto.is_empty() {
            // Auto tracks stretch to fill the grid.
            let each = free / auto.len() as i32;
            for &i in &auto { w[i] += each; }
        }
        for v in &mut w { *v = (*v).max(0); }
        w
    }

    /// `display: grid`: items placed in rows over the column tracks.
    fn grid(&mut self, n: NodeId, cx: i32, cy: i32, cw: i32) -> (i32, Option<i32>) {
        let s = self.style(n).unwrap();
        let mut items: Vec<(NodeId, style::GridPlace)> = Vec::new();
        for c in self.flat_children(n) {
            match &self.dom.nodes[c].kind {
                Kind::Text(t) if t.trim().is_empty() => continue,
                Kind::Element { .. } if self.display(c) == Display::None => continue,
                Kind::Element { .. } if self.out_of_flow(c) => { self.abs.push((c, cx, cy)); continue; }
                Kind::Element { .. } => items.push((c, self.style(c).unwrap().grid_column)),
                _ => items.push((c, style::GridPlace::default())),
            }
        }
        let cols = self.grid_tracks(s, cw, &items);
        let n_cols = cols.len();
        let mut xs = Vec::with_capacity(n_cols + 1);
        let mut x = cx;
        for w in &cols { xs.push(x); x += w + s.gap; }
        xs.push(x);
        // Row-major placement: (node, column, span) per row.
        let mut rows: Vec<Vec<(NodeId, usize, usize)>> = alloc::vec![Vec::new()];
        let mut col = 0;
        for (c, g) in items {
            let (start, span) = match (g.start, g.end) {
                (0, 0) => (None, g.span.max(1) as usize),
                (a, 0) => (Some(line_index(a, n_cols)), g.span.max(1) as usize),
                (0, b) => { let e = line_index(b, n_cols); let sp = g.span.max(1) as usize; (Some(e.saturating_sub(sp)), sp) }
                (a, b) => { let (a, b) = (line_index(a, n_cols), line_index(b, n_cols)); (Some(a.min(b)), a.max(b).saturating_sub(a.min(b)).max(1)) }
            };
            let span = span.min(n_cols);
            let start = start.map(|s| s.min(n_cols - span));
            let at = match start {
                Some(s0) => { if s0 < col { rows.push(Vec::new()); } s0 }
                None => { if col + span > n_cols { rows.push(Vec::new()); 0 } else { col } }
            };
            rows.last_mut().unwrap().push((c, at, span));
            col = at + span;
            if col >= n_cols { rows.push(Vec::new()); col = 0; }
        }
        let mut y = cy;
        let mut first = None;
        let mut any = false;
        for row in rows.into_iter().filter(|r| !r.is_empty()) {
            if any { y += s.row_gap; }
            any = true;
            let marks = (self.items.len(), self.links.len(), self.boxes.len(), self.anchors.len(), self.abs.len(), self.positioned.len(), self.floats.len());
            let heights = self.grid_row(n, &row, &xs, y, cw, None, &mut first);
            let row_h = heights.iter().copied().max().unwrap_or(0);
            // Items stretch to the row (or centre in it with align-items: center): lay the
            // row out again when one came up short.
            let short = heights.iter().any(|h| *h < row_h) && (s.align_center || row.iter().zip(&heights).any(|((c, _, _), h)| *h < row_h
                && matches!(self.dom.nodes[*c].kind, Kind::Element { .. })
                && self.style(*c).is_some_and(|cs| (cs.background >> 24 != 0 || cs.border.iter().any(|b| *b > 0)) && cs.height == Len::Auto)));
            if short {
                self.items.truncate(marks.0); self.links.truncate(marks.1); self.boxes.truncate(marks.2); self.anchors.truncate(marks.3);
                self.abs.truncate(marks.4); self.positioned.truncate(marks.5); self.floats.truncate(marks.6);
                self.grid_row(n, &row, &xs, y, cw, Some((&heights, row_h)), &mut first);
            }
            y += row_h;
        }
        (y - cy, first)
    }

    /// Lays out one grid row; returns each item's margin-box height.
    fn grid_row(&mut self, n: NodeId, row: &[(NodeId, usize, usize)], xs: &[i32], y: i32, cw: i32, again: Option<(&[i32], i32)>, first: &mut Option<i32>) -> Vec<i32> {
        let gap = self.style(n).unwrap().gap;
        let center = self.style(n).unwrap().align_center;
        let mut heights = Vec::with_capacity(row.len());
        for (k, &(c, at, span)) in row.iter().enumerate() {
            let x = xs[at];
            let y = match again { Some((hs, rh)) if center => y + (rh - hs[k]) / 2, _ => y };
            let w = xs[at + span] - x - gap;
            let h = if matches!(self.dom.nodes[c].kind, Kind::Text(_)) {
                self.inline_context(n, &[c], x, y, w).map_or(0, |(h, b)| { if first.is_none() { *first = b; } h })
            } else {
                let cs = self.style(c).unwrap();
                let (m, _, _) = sum_h(cs, cw);
                let (mt, mb) = (cs.margin[TOP].or_zero(cw), cs.margin[BOTTOM].or_zero(cw));
                if let Some((_, rh)) = again.filter(|_| !center) { if cs.height == Len::Auto { self.force_height = Some((rh - mt - mb).max(0)); } }
                let r = self.block(c, x, y + mt, w, Some(w - m));
                self.force_height = None;
                if first.is_none() { *first = r.first_baseline; }
                r.height + mt + r.margin_bottom
            };
            heights.push(h);
        }
        heights
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
                if matches!(tag, "img" | "svg" | "canvas" | "video" | "audio" | "iframe" | "object" | "embed") {
                    let src = self.picture_src(n, tag);
                    let natural = if tag == "canvas" { Some(self.canvas_size(n)) } else if tag == "svg" && self.dom.attr(n, "src").is_none() { self.svg_size(n) } else if tag == "audio" { Some((300, 32)) } else { self.host.image_size(&src) };
                    // A percentage is indefinite during intrinsic contribution sizing.
                    let height = s.height.resolve(0).filter(|_| matches!(s.height, Len::Px(_)));
                    let w = match (height, natural) {
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
                    if self.out_of_flow(c) { continue; }
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
