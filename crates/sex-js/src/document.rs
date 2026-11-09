// SPDX-License-Identifier: GPL-3.0-only
//! The [`Page`] for a parsed document: scripts read and change the [`Dom`], and the
//! browser lays the page out again when [`Document::dirty`] says they did.
use crate::{op, Page};
use alloc::{format, string::String, vec::Vec};
use sex_html::dom::{Dom, Kind, NodeId};

pub struct Document {
    pub dom: Dom,
    /// The page's address, for `location`.
    pub url: String,
    pub viewport: (i32, i32),
    /// Set by any change to the tree; the browser clears it after laying out again.
    pub dirty: bool,
    /// `console` output not yet taken: (level, text).
    pub log: Vec<(u8, String)>,
    /// Where a script asked to go, once it returns.
    pub navigate: Option<String>,
    /// The body of a form POST that `navigate` loads (form-urlencoded).
    pub navigate_post: Option<String>,
    /// Element boxes from the last layout, by node: (x, y, w, h).
    pub rects: Vec<Option<(i32, i32, i32, i32)>>,
    /// Each node's paint layer (higher paints on top), from `Page::layers`.
    pub layers: Vec<u32>,
    /// Supplies the monotonic clock.
    pub clock: fn() -> u64,
    /// Canvas 2D command batches not yet drawn, by canvas element.
    pub canvas_queue: Vec<(NodeId, String)>,
    /// Text width for `measureText`: (CSS font, text) to pixels.
    pub measure: fn(&str, &str) -> i32,
    /// WebGL contexts, by canvas element.
    pub gl: crate::webgl::GlCanvases,
    /// The page's decoded pictures by `src`, shared with the browser, so WebGL can
    /// upload one the moment a script asks.
    pub pictures: Vec<(String, alloc::rc::Rc<sex_image::Image>)>,
    /// `fetch`/XHR requests the browser has not loaded yet.
    pub fetches: Vec<FetchAsk>,
    /// Loaded bodies the script has not taken yet, by request id.
    pub bodies: Vec<(u32, Vec<u8>)>,
    /// Video and audio requests not yet taken by the browser: (element, verb, argument).
    pub media: Vec<(NodeId, String, String)>,
    /// Loaded replies' final address and headers, by request id.
    pub heads: Vec<(u32, String)>,
    next_fetch: u32,
    /// The window's cookies, shared with the browser's own requests.
    pub cookies: Option<alloc::rc::Rc<core::cell::RefCell<sex_fetch::Jar>>>,
    /// Seconds since 1970, for cookie expiry.
    pub wall: fn() -> u64,
}

/// One request a script made with `fetch` or `XMLHttpRequest`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchAsk {
    pub id: u32,
    /// As the script gave it (relative addresses are the browser's to resolve).
    pub url: String,
    /// Uppercase; empty means GET.
    pub method: String,
    /// `name: value` lines.
    pub headers: String,
    pub body: Vec<u8>,
}
impl FetchAsk {
    /// The request headers as pairs, skipping malformed lines.
    pub fn header_pairs(&self) -> Vec<(&str, &str)> {
        self.headers.lines().filter_map(|l| { let i = l.find(':')?; Some((l[..i].trim(), l[i + 1..].trim())) }).filter(|(k, _)| !k.is_empty()).collect()
    }
}

impl Document {
    /// Keeps a loaded body until the script takes it. Bodies nobody reads (a failed
    /// request, or a response whose body was never asked for) give way to newer ones
    /// once 64 are waiting.
    pub fn keep_body(&mut self, id: u32, status: u16, body: Vec<u8>) {
        if status == 0 { return; }
        if self.bodies.len() >= 64 { self.bodies.remove(0); }
        self.bodies.push((id, body));
    }
    /// Keeps a reply's final address and header lines until the script asks.
    pub fn keep_head(&mut self, id: u32, url: &str, headers: &str) {
        if self.heads.len() >= 64 { self.heads.remove(0); }
        self.heads.push((id, format!("{}\n{}", url, headers)));
    }
    fn page_url(&self) -> Option<sex_fetch::Url> { sex_fetch::Url::parse(crate::document::strip_fragment(&self.url)).ok() }
}

/// A rough width when the browser gives no measure: 0.55 em per character.
fn rough_measure(font: &str, text: &str) -> i32 {
    let size = font.split_ascii_whitespace().find_map(|t| t.strip_suffix("px")?.parse::<i32>().ok()).unwrap_or(10);
    text.chars().count() as i32 * size * 55 / 100
}

impl Document {
    pub fn new(dom: Dom, url: &str, viewport: (i32, i32), clock: fn() -> u64) -> Self {
        Self { dom, url: String::from(url), viewport, dirty: false, log: Vec::new(), navigate: None, navigate_post: None, rects: Vec::new(), layers: Vec::new(), clock, canvas_queue: Vec::new(), measure: rough_measure, gl: Default::default(), pictures: Vec::new(), fetches: Vec::new(), bodies: Vec::new(), media: Vec::new(), heads: Vec::new(), next_fetch: 0, cookies: None, wall: || 0 }
    }

    /// Takes each element's laid-out box from the browser's layout, for
    /// `getBoundingClientRect` and hit testing.
    pub fn set_boxes(&mut self, boxes: &[Option<[i32; 4]>]) {
        self.rects.clear();
        self.rects.extend(boxes.iter().map(|b| b.map(|r| (r[0], r[1], r[2], r[3]))));
    }
    pub fn set_layers(&mut self, layers: &[u32]) {
        self.layers.clear();
        self.layers.extend_from_slice(layers);
    }
    /// The topmost, then deepest, element whose laid-out box holds the page point
    /// (`x`, `y`): what a click there lands on. Later elements win ties, as they paint on top.
    pub fn node_at(&self, x: i32, y: i32) -> Option<NodeId> {
        let mut best: Option<((u32, usize), NodeId)> = None;
        for (n, rect) in self.rects.iter().enumerate() {
            let Some((rx, ry, rw, rh)) = *rect else { continue };
            if x < rx || y < ry || x >= rx + rw || y >= ry + rh { continue; }
            if self.dom.tag(n).is_none() { continue; }
            let mut depth = 0;
            let mut p = self.dom.nodes[n].parent;
            while let Some(q) = p { depth += 1; p = self.dom.nodes[q].parent; }
            let key = (self.layers.get(n).copied().unwrap_or(0), depth);
            if best.map_or(true, |(d, _)| key >= d) { best = Some((key, n)); }
        }
        best.map(|(_, n)| n)
    }
    fn node(&self, s: Option<&&str>) -> Option<NodeId> {
        let id: NodeId = s?.parse().ok()?;
        (id < self.dom.nodes.len()).then_some(id)
    }

    fn element(&self, s: Option<&&str>) -> Option<NodeId> {
        self.node(s).filter(|&n| matches!(self.dom.nodes[n].kind, Kind::Element { .. }))
    }

    fn ids(list: impl Iterator<Item = NodeId>) -> String {
        let mut out = String::new();
        for (i, n) in list.enumerate() {
            if i > 0 { out.push(','); }
            out.push_str(&format!("{n}"));
        }
        out
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
}

impl Page for Document {
    fn now_ns(&mut self) -> u64 { (self.clock)() }

    fn call_bytes(&mut self, operation: u32, a: &[&[u8]]) -> Option<Vec<u8>> {
        let text = |i: usize| a.get(i).and_then(|b| core::str::from_utf8(b).ok()).unwrap_or("");
        match operation {
            op::FETCH => {
                // A page may not have more than 64 requests waiting at once.
                if self.fetches.len() >= 64 { return None; }
                self.next_fetch += 1;
                let method = text(1).trim().to_ascii_uppercase();
                self.fetches.push(FetchAsk { id: self.next_fetch, url: String::from(text(0)), method, headers: String::from(text(2)), body: a.get(3).map(|b| b.to_vec()).unwrap_or_default() });
                return Some(format!("{}", self.next_fetch).into_bytes());
            }
            op::FETCH_HEAD => {
                let id: u32 = text(0).parse().ok()?;
                let i = self.heads.iter().position(|(n, _)| *n == id)?;
                return Some(self.heads.swap_remove(i).1.into_bytes());
            }
            op::COOKIE => {
                let jar = self.cookies.clone()?;
                let url = self.page_url()?;
                let now = (self.wall)();
                if let Some(v) = a.first() { jar.borrow_mut().store_from_script(&url, core::str::from_utf8(v).ok()?, now); return Some(Vec::new()); }
                return Some(jar.borrow().script_view(&url, now).into_bytes());
            }
            op::FETCH_BODY => {
                let id: u32 = text(0).parse().ok()?;
                let i = self.bodies.iter().position(|(n, _)| *n == id)?;
                return Some(self.bodies.swap_remove(i).1);
            }
            _ => {}
        }
        let node: NodeId = text(0).parse().ok()?;
        if node >= self.dom.nodes.len() { return None; }
        match operation {
            op::GL => {
                let canvas = self.gl.by_node.get_mut(&node)?;
                canvas.run(text(1), a.get(2).copied().unwrap_or(&[]), &self.pictures);
                Some(Vec::new())
            }
            op::GL_QUERY => {
                if text(1) == "init" {
                    // init W H ALPHA: makes (or keeps) the canvas's context.
                    let (w, h) = (text(2).parse().unwrap_or(300), text(3).parse().unwrap_or(150));
                    let alpha = text(4) != "0";
                    self.gl.by_node.entry(node).or_insert_with(|| crate::webgl::GlCanvas::new(w, h, alpha));
                    return Some(Vec::from(&b"1"[..]));
                }
                self.gl.by_node.get_mut(&node)?.query(text(1), &a[2..])
            }
            _ => None,
        }
    }

    fn call(&mut self, operation: u32, a: &[&str]) -> Option<String> {
        let none = String::new;
        match operation {
            op::LOG => {
                let level = a.first().and_then(|l| l.parse().ok()).unwrap_or(0u8).min(2);
                let text = a.get(1).copied().unwrap_or("");
                // QuickJS throws null when it has no memory left even for the error.
                let text = if level == 2 && text == "null" { "InternalError: out of memory (the page filled its script heap)" } else { text };
                // Keep the log bounded: a page that logs in a loop must not fill memory.
                if self.log.len() < 256 { self.log.push((level, String::from(&text[..floor(text, 1024)]))); }
                None
            }
            op::BY_ID => {
                let want = *a.first()?;
                self.dom.descendants(Dom::ROOT).into_iter().find(|&n| self.dom.attr(n, "id") == Some(want)).map(|n| format!("{n}"))
            }
            op::QUERY => {
                let root = self.node(a.first())?;
                let list = sex_html::css::parse_selector_list(a.get(1)?);
                if list.is_empty() { return Some(String::from("!")); }
                let all = a.get(2) == Some(&"1");
                let mut found = self.dom.descendants(root).into_iter().skip(1)
                    .filter(|&n| matches!(self.dom.nodes[n].kind, Kind::Element { .. }))
                    .filter(|&n| list.iter().any(|s| sex_html::css::matches(&self.dom, n, s)));
                if all { Some(Self::ids(found)) } else { found.next().map(|n| format!("{n}")) }
            }
            op::INFO => {
                let n = self.node(a.first())?;
                Some(match &self.dom.nodes[n].kind {
                    Kind::Document => String::from("9,#document"),
                    Kind::Text(_) => String::from("3,#text"),
                    Kind::Element { tag, .. } => format!("1,{}", tag.to_ascii_uppercase()),
                })
            }
            op::GET_ATTR => {
                let n = self.element(a.first())?;
                self.dom.attr(n, &a.get(1)?.to_ascii_lowercase()).map(String::from)
            }
            op::SET_ATTR => {
                let n = self.element(a.first())?;
                let name = a.get(1)?.to_ascii_lowercase();
                if !valid_name(&name) { return Some(String::from("!")); }
                self.dom.set_attr(n, &name, a.get(2)?);
                self.dirty = true;
                Some(none())
            }
            op::REMOVE_ATTR => {
                let n = self.element(a.first())?;
                self.dom.remove_attr(n, &a.get(1)?.to_ascii_lowercase());
                self.dirty = true;
                Some(none())
            }
            op::GET_TEXT => { let n = self.node(a.first())?; Some(self.dom.text_content(n)) }
            op::SET_TEXT => {
                let n = self.node(a.first())?;
                self.dom.set_text(n, a.get(1)?);
                self.dirty = true;
                Some(none())
            }
            op::GET_HTML => {
                let n = self.node(a.first())?;
                Some(if a.get(1) == Some(&"1") { self.dom.outer_html(n) } else { self.dom.inner_html(n) })
            }
            op::SET_HTML => {
                let n = self.element(a.first())?;
                self.dom.clear(n);
                sex_html::parse::parse_into(&mut self.dom, n, a.get(1)?);
                self.dirty = true;
                Some(none())
            }
            op::CREATE_ELEMENT => {
                let tag = a.first()?.to_ascii_lowercase();
                if !valid_name(&tag) { return Some(String::from("!")); }
                Some(format!("{}", self.dom.create(Kind::Element { tag, attrs: Vec::new() })))
            }
            op::CREATE_TEXT => Some(format!("{}", self.dom.create(Kind::Text(String::from(*a.first()?))))),
            op::INSERT => {
                let parent = self.node(a.first())?;
                let child = self.node(a.get(1))?;
                let before = self.node(a.get(2));
                if !self.dom.insert(parent, child, before) { return Some(String::from("!")); }
                self.dirty = true;
                Some(none())
            }
            op::DETACH => {
                let n = self.node(a.first())?;
                self.dom.detach(n);
                self.dirty = true;
                Some(none())
            }
            op::PARENT => { let n = self.node(a.first())?; self.dom.nodes[n].parent.map(|p| format!("{p}")) }
            op::CHILDREN => { let n = self.node(a.first())?; Some(Self::ids(self.dom.nodes[n].children.iter().copied())) }
            op::ATTR_NAMES => {
                let n = self.element(a.first())?;
                let Kind::Element { attrs, .. } = &self.dom.nodes[n].kind else { return None };
                Some(attrs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join("\n"))
            }
            op::VIEWPORT => Some(format!("{},{}", self.viewport.0, self.viewport.1)),
            op::NAVIGATE => { self.navigate = Some(String::from(*a.first()?)); self.navigate_post = a.get(1).filter(|m| m.eq_ignore_ascii_case("post")).map(|_| String::from(a.get(2).copied().unwrap_or(""))); None }
            op::CLICKABLE => {
                let n = self.element(a.first())?;
                if self.dom.clickable.insert(n) { self.dirty = true; }
                None
            }
            op::RECT => {
                let n = self.node(a.first())?;
                let (x, y, w, h) = self.rects.get(n).copied().flatten().unwrap_or((0, 0, 0, 0));
                Some(format!("{x},{y},{w},{h}"))
            }
            op::HIT => {
                let x = a.first()?.parse().ok()?;
                let y = a.get(1)?.parse().ok()?;
                self.node_at(x, y).map(|n| format!("{n}"))
            }
            op::VALUE => {
                let n = self.element(a.first())?;
                match a.get(1) {
                    Some(v) => { self.dom.values.insert(n, String::from(*v)); self.dirty = true; None }
                    None => Some(self.dom.control_value(n)),
                }
            }
            op::CARET => {
                let caret = match a.first().filter(|s| !s.is_empty()) {
                    Some(_) => Some((self.element(a.first())?, a.get(1).and_then(|p| p.parse().ok()).unwrap_or(0))),
                    None => None,
                };
                if caret != self.dom.caret { self.dom.caret = caret; self.dirty = true; }
                None
            }
            op::LOCATION => Some(self.url.clone()),
            op::CANVAS => {
                let n = self.element(a.first())?;
                if self.dom.tag(n) != Some("canvas") { return None; }
                let batch = *a.get(1)?;
                // A page that draws without end must not fill memory before the browser runs it.
                let queued: usize = self.canvas_queue.iter().map(|(_, b)| b.len()).sum();
                if queued + batch.len() > 8 << 20 { return None; }
                match self.canvas_queue.last_mut() {
                    Some((last, b)) if *last == n => { b.push('\u{1e}'); b.push_str(batch); }
                    _ => self.canvas_queue.push((n, String::from(batch))),
                }
                None
            }
            op::MEASURE => Some(format!("{}", (self.measure)(a.first()?, a.get(1)?))),
            op::MEDIA => {
                let n = self.element(a.first())?;
                if !matches!(self.dom.tag(n), Some("video" | "audio")) { return None; }
                // A page calling in a loop must not fill memory before the browser looks.
                if self.media.len() >= 256 { return None; }
                self.media.push((n, String::from(*a.get(1)?), String::from(a.get(2).copied().unwrap_or(""))));
                None
            }
            _ => None,
        }
    }
}

/// The largest char boundary at or below `max`.
fn floor(s: &str, max: usize) -> usize {
    let mut i = max.min(s.len());
    while !s.is_char_boundary(i) { i -= 1; }
    i
}

/// An address without its `#fragment`.
pub(crate) fn strip_fragment(url: &str) -> &str { url.split('#').next().unwrap_or("") }
