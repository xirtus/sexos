// SPDX-License-Identifier: GPL-3.0-only
//! The [`Page`] for a parsed document: scripts read and change the [`Dom`], and the
//! browser lays the page out again when [`Document::dirty`] says they did.
use crate::{op, Page};
use core::sync::atomic::{AtomicU64, Ordering};
static NEXT_RESOURCE_OWNER: AtomicU64 = AtomicU64::new(1);
use alloc::{format, string::String, vec::Vec};
use sex_html::dom::{Dom, Kind, NodeId};

pub struct Document {
    resource_owner: u64,
    source_policies: alloc::collections::BTreeMap<String, bool>,
    source_policy_overflow: bool,
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
    /// The POST body's `Content-Type` (empty: `application/x-www-form-urlencoded`).
    pub navigate_type: String,
    /// `document.referrer`: the page this one was opened from, as its request said.
    pub referrer: String,
    /// For a page in an iframe, the origin of the page around it (`window.parent`).
    pub parent_origin: Option<String>,
    /// `postMessage` calls for other pages, once the script returns: (to: -1 for the
    /// page around this one, else an iframe element of this page; data as JSON;
    /// target origin, `*` for any).
    pub outbox: Vec<(i64, String, String)>,
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
    /// Classic script resources: loaded/evaluated by the browser, not fetch().
    pub classic_scripts: Vec<ClassicScript>,
    /// Loaded bodies the script has not taken yet, by request id.
    pub bodies: Vec<(u32, Vec<u8>)>,
    /// Video and audio requests not yet taken by the browser: (element, verb, argument).
    pub media: Vec<(NodeId, String, String)>,
    /// Media Source appends not taken yet: (element, SourceBuffer, bytes).
    pub media_bytes: Vec<(NodeId, u32, Vec<u8>)>,
    /// Loaded replies' final address and headers, by request id.
    pub heads: Vec<(u32, String)>,
    next_fetch: u32,
    /// The window's cookies, shared with the browser's own requests.
    pub cookies: Option<alloc::rc::Rc<core::cell::RefCell<sex_fetch::Jar>>>,
    /// Seconds since 1970, for cookie expiry.
    pub wall: fn() -> u64,
    /// `localStorage` of every origin, kept by the browser.
    pub storage: Option<alloc::rc::Rc<core::cell::RefCell<crate::storage::WebStorage>>>,
    /// Loaded ES module sources by address, for `import`.
    pub modules: alloc::collections::BTreeMap<String, String>,
    /// Module addresses asked for and not in `modules`.
    pub modules_wanted: Vec<String>,
    /// The page's import map: (bare specifier, or a prefix ending in `/`, address).
    pub import_map: Vec<(String, String)>,
}

/// A connected script element requesting classic execution. The host derives
/// its URL and CORS mode from the actual DOM, never a page-supplied fetch option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassicScript {
    pub node: NodeId,
    pub url: String,
    pub cors: bool,
    owner: u64,
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
    /// Bind requested module names to the fetched final URL, including redirects.
    /// This records provenance only; it grants no fetch/CORS access.
    pub fn keep_source_policy(&mut self, requested: &str, final_url: &str) {
        let requested = strip_fragment(requested);
        if self.source_policies.len() < 128 || self.source_policies.contains_key(requested) {
            self.source_policies.insert(String::from(requested), self.source_url_muted(final_url));
        } else { self.source_policy_overflow = true; }
    }
    fn source_muted(&self, source: &str) -> bool {
        let source = strip_fragment(source);
        if let Some(&muted) = self.source_policies.get(source) { return muted; }
        if self.source_policy_overflow { return true; }
        self.source_url_muted(source)
    }
    fn source_url_muted(&self, source: &str) -> bool {
        let Some(resolved) = sex_html::url::resolve(&self.url, source) else {
            return sex_fetch::Url::parse(&self.url).is_ok() || source.contains(':');
        };
        match (sex_fetch::Url::parse(&self.url), sex_fetch::Url::parse(&resolved)) {
            (Ok(owner), Ok(src)) => owner.origin() != src.origin(),
            (_, Ok(_)) | (Ok(_), Err(_)) => true,
            _ => false, // host-owned local/about sources
        }
    }
    /// Whether a completion belongs to this document, independent of repainting
    /// or a script intentionally removing its own node after evaluation.
    pub fn classic_script_owner(&self, ask: &ClassicScript) -> bool { ask.owner == self.resource_owner }

    /// A delayed script is still bound to this document and the unchanged,
    /// connected DOM element. Reloading the same URL creates a new owner token.
    pub fn classic_script_current(&self, ask: &ClassicScript) -> bool {
        self.classic_script_owner(ask) && ask.node < self.dom.nodes.len()
            && self.dom.tag(ask.node) == Some("script")
            && self.dom.is_inclusive_ancestor(Dom::ROOT, ask.node)
            && self.dom.attr(ask.node, "src").map(str::trim) == Some(ask.url.as_str())
            && self.dom.attr(ask.node, "crossorigin").is_some() == ask.cors
            && !self.dom.attr(ask.node, "crossorigin").is_some_and(|v| v.eq_ignore_ascii_case("use-credentials"))
            && self.dom.attr(ask.node, "type").is_none_or(|v| {
                let ty = v.trim().to_ascii_lowercase();
                ty.is_empty() || ty.contains("javascript") || ty.contains("ecmascript")
            })
    }

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
        Self { resource_owner: NEXT_RESOURCE_OWNER.fetch_add(1, Ordering::Relaxed), source_policies: Default::default(), source_policy_overflow: false, dom, url: String::from(url), viewport, dirty: false, log: Vec::new(), navigate: None, navigate_post: None, navigate_type: String::new(), referrer: String::new(), parent_origin: None, outbox: Vec::new(), rects: Vec::new(), layers: Vec::new(), clock, canvas_queue: Vec::new(), measure: rough_measure, gl: Default::default(), pictures: Vec::new(), fetches: Vec::new(), classic_scripts: Vec::new(), bodies: Vec::new(), media: Vec::new(), media_bytes: Vec::new(), heads: Vec::new(), next_fetch: 0, cookies: None, wall: || 0, storage: None, modules: Default::default(), modules_wanted: Vec::new(), import_map: Vec::new() }
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

/// Where an `import` specifier points from module `base`: the import map's entry
/// (exact, or the longest matching prefix ending in `/`) first; then relative and
/// absolute addresses resolve against `base`; another bare name (`lit`) is left as it is.
pub fn resolve_module(map: &[(String, String)], base: &str, spec: &str) -> String {
    if let Some((_, to)) = map.iter().find(|(k, _)| k == spec) { return to.clone(); }
    if let Some((k, to)) = map.iter().filter(|(k, _)| k.ends_with('/') && spec.starts_with(k.as_str())).max_by_key(|(k, _)| k.len()) {
        return format!("{}{}", to, &spec[k.len()..]);
    }
    let relative = spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/');
    if !relative && !spec.contains("://") { return String::from(spec); }
    sex_html::url::resolve(base, spec).unwrap_or_else(|| String::from(spec))
}

impl Page for Document {
    fn now_ns(&mut self) -> u64 { (self.clock)() }
    fn take_wanted_modules(&mut self) -> Vec<String> { core::mem::take(&mut self.modules_wanted) }

    fn call_bytes(&mut self, operation: u32, a: &[&[u8]]) -> Option<Vec<u8>> {
        let text = |i: usize| a.get(i).and_then(|b| core::str::from_utf8(b).ok()).unwrap_or("");
        match operation {
            op::SOURCE_MUTED => return Some(if self.source_muted(text(0)) { b"1".to_vec() } else { b"0".to_vec() }),
            op::CLASSIC_SCRIPT => {
                if self.classic_scripts.len() >= 64 { return None; }
                let node = self.element(Some(&text(0)))?;
                if self.dom.tag(node) != Some("script") || !self.dom.is_inclusive_ancestor(Dom::ROOT, node) { return None; }
                let ty = self.dom.attr(node, "type").unwrap_or("").trim().to_ascii_lowercase();
                if !ty.is_empty() && !ty.contains("javascript") && !ty.contains("ecmascript") { return None; }
                let src = self.dom.attr(node, "src")?.trim();
                if src.is_empty() { return None; }
                // Credentialed cross-origin script mode needs a complete ACAC
                // policy; refuse until supported instead of silently bypassing it.
                if self.dom.attr(node, "crossorigin").is_some_and(|v| v.eq_ignore_ascii_case("use-credentials")) { return None; }
                self.classic_scripts.push(ClassicScript { node, url: String::from(src), cors: self.dom.attr(node, "crossorigin").is_some(), owner: self.resource_owner });
                return Some(b"1".to_vec());
            }
            op::FETCH => {
                // A page may not have more than 64 requests waiting at once.
                if self.fetches.len() >= 64 { return None; }
                self.next_fetch += 1;
                let method = text(1).trim().to_ascii_uppercase();
                self.fetches.push(FetchAsk { id: self.next_fetch, url: String::from(text(0)), method, headers: String::from(text(2)), body: a.get(3).map(|b| b.to_vec()).unwrap_or_default() });
                return Some(format!("{}", self.next_fetch).into_bytes());
            }
            op::MEDIA_APPEND => {
                let n = self.element(Some(&text(0)))?;
                if !matches!(self.dom.tag(n), Some("video" | "audio")) { return None; }
                // Appends wait here until the browser's next tick: keep that bounded.
                let queued: usize = self.media_bytes.iter().map(|(_, _, b)| b.len()).sum();
                let bytes = a.get(2).copied().unwrap_or(&[]);
                if queued + bytes.len() > 64 << 20 { return None; }
                self.media_bytes.push((n, text(1).parse().ok()?, bytes.to_vec()));
                return Some(Vec::new());
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
            op::STORAGE => {
                let store = self.storage.clone()?;
                let url = self.page_url()?;
                let origin = url.origin();
                let key = a.get(1).copied().unwrap_or("");
                return match *a.first()? {
                    "get" => store.borrow().get(&origin, key).map(String::from),
                    "set" => Some(String::from(if store.borrow_mut().set(&origin, key, a.get(2).copied().unwrap_or("")) { "" } else { "!" })),
                    "remove" => { store.borrow_mut().remove(&origin, key); Some(none()) }
                    "clear" => { store.borrow_mut().clear(&origin); Some(none()) }
                    "len" => Some(format!("{}", store.borrow().len(&origin))),
                    "key" => store.borrow().key(&origin, key.parse().ok()?).map(String::from),
                    _ => None,
                };
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
            op::NAVIGATE => { self.navigate = Some(String::from(*a.first()?)); self.navigate_post = a.get(1).filter(|m| m.eq_ignore_ascii_case("post")).map(|_| String::from(a.get(2).copied().unwrap_or("")));
                // Only the form encodings a page may choose.
                let ty = a.get(3).copied().unwrap_or("");
                self.navigate_type = if ty.starts_with("multipart/form-data; boundary=") || ty == "text/plain" { String::from(ty) } else { String::new() };
                None }
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
            op::MODULE_RESOLVE => Some(resolve_module(&self.import_map, strip_fragment(a.first()?), a.get(1)?)),
            op::MODULE_SOURCE => {
                // The fragment names nothing to fetch: x.js#a and x.js share one source.
                let url = &strip_fragment(a.first()?);
                match self.modules.get(*url) {
                    Some(src) => Some(src.clone()),
                    None => {
                        if !self.modules_wanted.iter().any(|w| w == url) { self.modules_wanted.push(String::from(*url)); }
                        None
                    }
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
            op::FRAME => Some(alloc::format!("{}\t{}", self.referrer, self.parent_origin.as_deref().unwrap_or(""))),
            op::POST_MESSAGE => {
                let to: i64 = a.first()?.parse().ok()?;
                // A page can't flood the browser with messages it never lets go of.
                if self.outbox.len() < 1024 { self.outbox.push((to, String::from(*a.get(1)?), String::from(a.get(2).copied().unwrap_or("*")))); }
                None
            }
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
/// The `imports` of an import map (`<script type="importmap">`), addresses resolved
/// against the page: (specifier or prefix ending in `/`, address).
/// The source the page gets for a module that could not be loaded: importing it fails,
/// as a failed fetch does in browsers, instead of giving an empty module.
pub fn failed_module(url: &str) -> String {
    let mut quoted = String::new();
    for c in url.chars() {
        match c { '\\' | '\'' => { quoted.push('\\'); quoted.push(c) } '\n' | '\r' => quoted.push(' '), c => quoted.push(c) }
    }
    format!("throw new TypeError('Failed to fetch module: {quoted}');")
}

pub fn import_map(base: &str, json: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(at) = json.find("\"imports\"") else { return out };
    let rest = &json[at + 9..];
    let Some(open) = rest.find('{') else { return out };
    let mut chars = rest[open + 1..].chars().peekable();
    // Reads one JSON string, after its opening quote.
    fn string(chars: &mut core::iter::Peekable<core::str::Chars>) -> Option<String> {
        let mut s = String::new();
        loop {
            match chars.next()? {
                '"' => return Some(s),
                '\\' => { let c = chars.next()?; s.push(match c { 'n' => '\n', 't' => '\t', other => other }); }
                c => s.push(c),
            }
        }
    }
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') { chars.next(); }
        match chars.next() {
            Some('"') => {}
            _ => break,
        }
        let Some(key) = string(&mut chars) else { break };
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ':') { chars.next(); }
        if chars.next() != Some('"') { break; }
        let Some(value) = string(&mut chars) else { break };
        let url = sex_html::url::resolve(base, &value).unwrap_or(value);
        out.push((key, url));
    }
    out
}

pub(crate) fn strip_fragment(url: &str) -> &str { url.split('#').next().unwrap_or("") }
