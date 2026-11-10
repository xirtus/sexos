// SPDX-License-Identifier: GPL-3.0-only
//! Web: the graphical browser. `run web` shows its welcome page, `run +net web
//! https://example.com` opens a site, `run web @page.html` a file in its folder.
//!
//! Pages are parsed, styled and laid out here by sex-html and drawn by sex-web
//! (Geist text, boxes, PNG/JPEG pictures) into the window's pixel surface. All of it
//! runs inside this program's own confinement: a hostile page can only crash this
//! window. Page scripts run in QuickJS on wasmi (sex-js), confined again inside that
//! interpreter. Keys: Up/Down/Space/PgUp/PgDn/Home/End scroll, Right/Left pick a link
//! or button, Enter opens it, Backspace goes back, `/` types an address, F11 closes.
//! A page's own key handlers see each key first and may keep it.
#![no_std]
#![no_main]

extern crate alloc;

mod media;
mod mse;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use sex_app::{files, nav, net, Event, Nav, Window};
use sex_web::sex_html::{self, layout::Page, Host};
use sex_js::{Document, Script};
use sex_web::{Fonts, BAR};

/// The window's content area as Shell tiles it today (a program can't learn its size yet).
const W: i32 = 410;
const H: i32 = 330;
const WELCOME: &str = include_str!("welcome.html");
/// Pages built into the program, by `about:` address. The games are plain HTML and
/// JavaScript on Canvas 2D, so they exercise the same engine as any web page.
const BUILT_IN: &[(&str, &str)] = &[
    ("about:welcome", WELCOME),
    ("about:games", include_str!("games/index.html")),
    ("about:solitaire", include_str!("games/solitaire.html")),
    ("about:chess", include_str!("games/chess.html")),
    ("about:maze", include_str!("games/maze.html")),
    ("about:chess3d", include_str!("games/chess3d.html")),
    ("about:pinball", include_str!("games/pinball.html")),
    // Scripts the built-in pages load by relative `src`.
    ("about:chess-engine.js", include_str!("games/chess-engine.js")),
    ("about:three.cjs", include_str!("games/three.cjs")),
];
fn built_in(url: &str) -> Option<&'static str> { BUILT_IN.iter().find(|(a, _)| *a == url).map(|(_, p)| *p) }
/// The page a run without arguments opens: this program is also built as `solitaire`
/// `chess`, `maze`, `chess3d` and `pinball`, which start on their game.
fn first_page() -> &'static str {
    match env!("CARGO_BIN_NAME") { "solitaire" => "about:solitaire", "chess" => "about:chess", "maze" => "about:maze", "chess3d" => "about:chess3d", "pinball" => "about:pinball", _ => "about:welcome" }
}
const SOURCE_MAX: usize = 4 << 20;
const MAX_SHEETS: usize = 8;
const MAX_IMAGES: usize = 24;
const MAX_REDIRECTS: usize = 6;
const MAX_SCRIPTS: usize = 64;
/// Most ES modules one page may load.
const MAX_MODULES: usize = 256;
/// Largest body a script's `fetch` may load (a game's data file, say).
const FETCH_MAX: usize = 32 << 20;
/// Most bytes one script or module may have: app bundles (YouTube's is about 10 MB) run
/// past what an HTML page or style sheet needs.
const SCRIPT_MAX: usize = 32 << 20;
/// Largest video or sound file: it plays while it downloads, but is kept whole so
/// it can be seeked back through.
const MEDIA_MAX: usize = 256 << 20;
/// How long one tick may spend taking in media downloads.
const MEDIA_LOAD_BUDGET_MS: u64 = 4;
/// How often a running CSS animation is drawn (about 30 a second).
const ANIMATION_FRAME_MS: u64 = 33;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[web.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn clock() -> u64 { sex_pdx::get_ticks().saturating_mul(1_000_000) }
/// Seconds since 1970, for cookie expiry. Programs have no civil clock yet, so this
/// counts from the day this was written; cookies only need it to move forward.
fn wall() -> u64 { 1_794_182_400 + sex_pdx::get_ticks() / 1000 }

/// A loaded reply: final address, status, header lines for scripts, body.
struct Loaded { url: String, status: u16, headers: String, body: Vec<u8> }

/// Network requests a page's scripts may have going at once. The network service makes
/// flows as they are asked for; this keeps one page from asking for hundreds.
const PARALLEL: usize = 16;
/// `send`'s word that the service has no flow free right now.
const BUSY: &[u8] = b"busy";
fn pause() { let until = sex_pdx::get_ticks() + 5; while sex_pdx::get_ticks() < until { sex_pdx::sys_yield(); } }
fn failed(url: &str) -> Loaded { Loaded { url: String::from(url), status: 0, headers: String::new(), body: Vec::new() } }

/// A request on its way: where it is now (after any redirects) and what to send there.
/// A video or sound source on its way in.
struct MediaLoad {
    node: sex_html::dom::NodeId,
    p: Pending,
    ex: net::Exchange,
    body: alloc::rc::Rc<core::cell::RefCell<sex_media::Growing>>,
    /// The reply is a success that can be used as it comes (else it is read whole).
    streaming: bool,
    /// A first try to open before the end failed; try again at the end.
    retry_at_end: bool,
    opened: bool,
}

struct Pending {
    id: u32, asked: String, since: u64, begun: u64, hops: usize,
    method: String, url: String, headers: Vec<(String, String)>, body: Vec<u8>, max: usize, script: Option<String>,
}
impl Pending {
    fn new(id: u32, method: &str, url: &str, headers: &[(&str, &str)], body: &[u8], max: usize, script: Option<&str>) -> Pending {
        Pending {
            id, asked: String::from(url), since: sex_pdx::get_ticks(), begun: 0, hops: 0,
            method: String::from(if method.is_empty() { "GET" } else { method }), url: String::from(sex_html::url::fragment(url).0),
            headers: headers.iter().map(|(k, v)| (String::from(*k), String::from(*v))).collect(), body: Vec::from(body), max, script: script.map(String::from),
        }
    }
}

/// The page's tree, with the script engine that owns it when the page has scripts.
enum Doc {
    Plain(Document),
    /// Boxed: the engine is large, and programs have small stacks.
    Scripted(alloc::boxed::Box<Script<Document>>),
}
impl Doc {
    fn get(&mut self) -> &mut Document { match self { Doc::Plain(d) => d, Doc::Scripted(s) => s.page_mut() } }
    fn script(&mut self) -> Option<&mut Script<Document>> { match self { Doc::Scripted(s) => Some(s), _ => None } }
}

/// One script of a page, ready to run.
/// A script to run; `node` is its element, for `document.currentScript` (classic scripts only).
struct PageScript { source: String, name: String, module: bool, node: Option<usize> }

struct Browser {
    window: Window,
    fonts: Fonts,
    page: Page,
    doc: Doc,
    sheets: Vec<String>,
    /// When the page's next timer or animation frame is due, in ticks.
    timer: Option<u64>,
    /// When each CSS animation on the page started.
    timeline: sex_html::Timeline,
    /// A CSS animation is running: style and draw the page again soon.
    animating: bool,
    /// The page's Canvas 2D surfaces.
    canvases: sex_web::canvas::Canvases,
    /// The page's video and audio elements, each with its player once loaded.
    media: Vec<media::Media>,
    /// The page asked its media for something since the last tick.
    media_asked: bool,
    /// Media sources downloading; each plays as soon as enough has come.
    media_loads: Vec<MediaLoad>,
    url: String,
    history: Vec<String>,
    scroll: i32,
    focus: Option<usize>,
    editing: Option<String>,
    status: Option<&'static str>,
    buf: Vec<u8>,
    /// The window's cookies, shared with page scripts (`document.cookie`).
    jar: alloc::rc::Rc<core::cell::RefCell<sex_fetch::Jar>>,
    /// The window shows no full frame of the page yet, so pictures alone can't update it.
    full_redraw: bool,
    /// The link node a left press started on: a release follows a link only if it
    /// ends over that same link, as a click does.
    press_link: Option<usize>,
    /// The page's style sheets use `:hover`: pointer moves restyle it.
    hover_css: bool,
    /// A form's POST body for the next page `open` loads.
    post: Option<String>,
    /// The import map of the page being loaded, until its scripts start.
    import_map: Vec<(String, String)>,
    /// The scroll offset the page script last heard (`window.scrollY`).
    script_scroll: i32,
    /// The current page's viewport: the window below the bar, or an iframe's box.
    view: (i32, i32),
    /// The page around the current one when it is in an iframe (its address).
    parent: Option<String>,
    /// The shown page's iframes, each a page of its own (see [`Frame`]).
    frames: Vec<Frame>,
    /// The iframe element whose page is the current one, while the browser works on it.
    in_frame: Option<usize>,
    /// A page in a frame changed: draw the window again once back at the top.
    frame_dirty: bool,
    /// The page the next `open` comes from (its `Referer`).
    referrer_next: Option<String>,
    /// While a page loads (`Some`): the page it was opened from, for its requests'
    /// `Referer`, or `None` when it has none (a typed address, `no-referrer`).
    nav_from: Option<Option<String>>,
    /// The shown page was laid out again: its iframes may have come, gone or moved.
    frames_check: bool,
}

/// Most iframes a page loads; more show as empty boxes.
const MAX_FRAMES: usize = 4;

/// A page in an iframe. It has everything a page has (tree, scripts, timers, fetches,
/// media, pictures), kept here while the browser works on another page; to work on it,
/// the browser swaps it in as its current page (`enter`) and back (`leave`). Its
/// requests carry the page around it as `Referer`, and the two talk by `postMessage`.
struct Frame {
    node: usize,
    src: String,
    /// The iframe's box on the page around it (x, y, w, h).
    rect: [i32; 4],
    page: Page,
    doc: Doc,
    sheets: Vec<String>,
    timer: Option<u64>,
    timeline: sex_html::Timeline,
    animating: bool,
    canvases: sex_web::canvas::Canvases,
    media: Vec<media::Media>,
    media_asked: bool,
    media_loads: Vec<MediaLoad>,
    url: String,
    scroll: i32,
    focus: Option<usize>,
    hover_css: bool,
    import_map: Vec<(String, String)>,
    script_scroll: i32,
    view: (i32, i32),
    parent: Option<String>,
    images: Vec<(String, alloc::rc::Rc<sex_web::Image>)>,
    svgs: Vec<(String, alloc::rc::Rc<str>, (i32, i32))>,
    moving: Vec<sex_web::Moving>,
    frames_shown: u64,
}

/// Text from bytes: UTF-8 when valid, otherwise read as Latin-1.
fn text_of(bytes: &[u8]) -> String {
    match core::str::from_utf8(bytes) {
        Ok(s) => String::from(s),
        Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 > bytes.len() => String::from(core::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or("")),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// The file a relative address names on a page opened from a file (`@dir/page.html`).
fn local_path(base: &str, url: &str) -> Option<String> {
    if let Some(p) = url.strip_prefix('@') { return Some(String::from(p)); }
    let page = base.strip_prefix('@')?;
    if url.contains("://") || url.starts_with("about:") || url.starts_with('/') { return None; }
    let dir = &page[..page.rfind('/').map_or(0, |i| i + 1)];
    Some(alloc::format!("{}{}", dir, url.trim_start_matches("./")))
}

fn message_page(title: &str, text: &str, url: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;");
    alloc::format!(
        "<title>{}</title><body style='margin:24px;font-size:15px'><h2 style='color:#b3261e'>{}</h2><p>{}</p><p style='color:#5f6368'>{}</p><p><a href='about:welcome'>Welcome page</a></p>",
        esc(title), esc(title), esc(text), esc(url)
    )
}

/// The origin of an http(s) address, as `postMessage` and `ancestorOrigins` name it.
fn origin_of(url: &str) -> Option<String> { sex_fetch::Url::parse(url).ok().map(|u| u.origin()) }

impl Browser {
    /// Swaps frame `i`'s page with the current one.
    fn swap_frame(&mut self, i: usize) {
        use core::mem::swap;
        let f = &mut self.frames[i];
        swap(&mut self.page, &mut f.page); swap(&mut self.doc, &mut f.doc); swap(&mut self.sheets, &mut f.sheets);
        swap(&mut self.timer, &mut f.timer); swap(&mut self.timeline, &mut f.timeline); swap(&mut self.animating, &mut f.animating);
        swap(&mut self.canvases, &mut f.canvases); swap(&mut self.media, &mut f.media); swap(&mut self.media_asked, &mut f.media_asked);
        swap(&mut self.media_loads, &mut f.media_loads); swap(&mut self.url, &mut f.url); swap(&mut self.scroll, &mut f.scroll);
        swap(&mut self.focus, &mut f.focus); swap(&mut self.hover_css, &mut f.hover_css); swap(&mut self.import_map, &mut f.import_map);
        swap(&mut self.script_scroll, &mut f.script_scroll); swap(&mut self.view, &mut f.view); swap(&mut self.parent, &mut f.parent);
        swap(&mut self.fonts.images, &mut f.images); swap(&mut self.fonts.svgs, &mut f.svgs); swap(&mut self.fonts.moving, &mut f.moving);
        swap(&mut self.fonts.frames_shown, &mut f.frames_shown);
    }

    /// Makes frame `i`'s page the current one, until `leave`. Only the shown page has
    /// frames: frames inside frames are not loaded.
    fn enter(&mut self, i: usize) { self.swap_frame(i); self.in_frame = Some(self.frames[i].node); }
    fn leave(&mut self, i: usize) { self.swap_frame(i); self.in_frame = None; }

    /// Stops a frame's downloads as it goes.
    fn close_frame(&mut self, mut f: Frame) {
        for mut l in core::mem::take(&mut f.media_loads) { l.ex.cancel(&mut self.window); }
        sex_pdx::serial_record!("[web.frame.closed] node={}", f.node);
    }

    fn drop_frames(&mut self) {
        if self.in_frame.is_some() { return; }
        for f in core::mem::take(&mut self.frames) { self.close_frame(f); }
    }

    /// Brings the shown page's frames in line with its iframes: loads new ones (an
    /// iframe with an http(s) `src`, up to `MAX_FRAMES`), closes those whose element
    /// left or changed address, and follows their boxes.
    #[inline(never)]
    fn sync_frames(&mut self) {
        if self.in_frame.is_some() { return; }
        let base = self.url.clone();
        let doc = self.doc.get();
        let wanted: Vec<(usize, String, bool)> = doc.dom.descendants(sex_html::dom::Dom::ROOT).into_iter()
            .filter(|&n| doc.dom.tag(n) == Some("iframe"))
            .filter_map(|n| {
                let src = sex_html::url::resolve(&base, doc.dom.attr(n, "src")?.trim())?;
                src.starts_with("http").then(|| (n, src, doc.dom.attr(n, "referrerpolicy").is_some_and(|p| p.trim().eq_ignore_ascii_case("no-referrer"))))
            })
            .take(MAX_FRAMES).collect();
        let mut i = 0;
        while i < self.frames.len() {
            if wanted.iter().any(|(n, src, _)| *n == self.frames[i].node && *src == self.frames[i].src) { i += 1; continue; }
            let f = self.frames.remove(i);
            self.close_frame(f);
            self.frame_dirty = true;
        }
        for i in 0..self.frames.len() {
            // No box (hidden, `display: none`): nothing drawn, no pointer, until it has one.
            let b = self.page.boxes.get(self.frames[i].node).copied().flatten().unwrap_or([0; 4]);
            if b == self.frames[i].rect { continue; }
            self.frames[i].rect = b;
            self.frame_dirty = true;
            if (b[2], b[3]) != self.frames[i].view && b[2] > 0 && b[3] > 0 {
                self.enter(i);
                self.view = (b[2], b[3]);
                if let Doc::Scripted(s) = &mut self.doc { s.page_mut().viewport = (b[2], b[3]); }
                self.relayout();
                self.leave(i);
            }
        }
        for (node, src, no_referrer) in wanted {
            if self.frames.iter().any(|f| f.node == node) { continue; }
            let Some(Some(b)) = self.page.boxes.get(node).copied() else { continue };
            if b[2] <= 0 || b[3] <= 0 { continue; }
            let page = sex_html::render("", b[2], b[3], &self.fonts as &dyn Host);
            self.frames.push(Frame {
                node, src: src.clone(), rect: b, page, doc: Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", (b[2], b[3]), clock)),
                sheets: Vec::new(), timer: None, timeline: Default::default(), animating: false, canvases: sex_web::canvas::Canvases::new(),
                media: Vec::new(), media_asked: false, media_loads: Vec::new(), url: String::new(), scroll: 0, focus: None, hover_css: false,
                import_map: Vec::new(), script_scroll: 0, view: (b[2], b[3]), parent: Some(base.clone()),
                images: Vec::new(), svgs: Vec::new(), moving: Vec::new(), frames_shown: 0,
            });
            let i = self.frames.len() - 1;
            sex_pdx::serial_record!("[web.frame] node={} w={} h={} url={}", node, b[2], b[3], src);
            self.enter(i);
            // The iframe's request comes from the page around it.
            self.referrer_next = if no_referrer { None } else { Some(base.clone()) };
            self.open(&src, false);
            let (items, scripted, referrer) = (self.page.items.len(), matches!(self.doc, Doc::Scripted(_)), self.doc.get().referrer.clone());
            self.leave(i);
            sex_pdx::serial_record!("[web.frame.loaded] node={} items={} scripted={} referrer={}", node, items, scripted as u8, if referrer.is_empty() { "-" } else { &referrer });
            self.frame_dirty = true;
            if let Some(script) = self.doc.script() { let _ = script.dispatch(&alloc::format!("frameload\t{node}")); }
            // Its handler may have sent the page elsewhere, taking its frames.
            if self.after_script(true) && self.url != base { return; }
        }
    }

    /// Carries `postMessage` calls between the shown page and its frames. A message
    /// goes only to the origin its sender named (or any, for `*`).
    #[inline(never)]
    fn deliver(&mut self) {
        let top_url = self.url.clone();
        let top_origin = origin_of(&top_url).unwrap_or_else(|| String::from("null"));
        let fits = |target: &str, origin: &str| target == "*" || origin_of(target).as_deref() == Some(origin);
        for _ in 0..16 {
            let mut any = false;
            let out = core::mem::take(&mut self.doc.get().outbox);
            for (to, data, target) in out {
                let Some(i) = self.frames.iter().position(|f| f.node as i64 == to) else { continue };
                let origin = origin_of(&self.frames[i].url).unwrap_or_else(|| String::from("null"));
                if !fits(&target, &origin) { sex_pdx::serial_record!("[web.message] dropped to={} target={}", origin, target); continue; }
                self.enter(i);
                if let Some(script) = self.doc.script() { let _ = script.dispatch(&alloc::format!("message\tparent\t{top_origin}\t{data}")); }
                self.after_script(true);
                self.leave(i);
                sex_pdx::serial_record!("[web.message] down node={} bytes={}", to, data.len());
                any = true;
            }
            let mut i = 0;
            while i < self.frames.len() {
                let out = core::mem::take(&mut self.frames[i].doc.get().outbox);
                let (node, origin) = (self.frames[i].node, origin_of(&self.frames[i].url).unwrap_or_else(|| String::from("null")));
                i += 1;
                for (to, data, target) in out {
                    if to != -1 || !fits(&target, &top_origin) { continue; }
                    if let Some(script) = self.doc.script() { let _ = script.dispatch(&alloc::format!("message\t{node}\t{origin}\t{data}")); }
                    sex_pdx::serial_record!("[web.message] up node={} bytes={}", node, data.len());
                    any = true;
                    // The page may have gone elsewhere, taking its frames.
                    if self.after_script(true) && self.url != top_url { return; }
                }
            }
            if !any { break; }
        }
    }

    /// Paints the frames' pages over their iframes' boxes on the window.
    fn paint_frames(&mut self) {
        for i in 0..self.frames.len() {
            let [x, y, w, h] = self.frames[i].rect;
            let top = y - self.scroll + BAR;
            if w <= 0 || h <= 0 || top >= H || top + h <= BAR || x >= W || x + w <= 0 { continue; }
            let mut tmp = alloc::vec![0xFFFF_FFFFu32; (w * h) as usize];
            self.enter(i);
            sex_web::paint(&self.page, &mut self.fonts, &mut sex_web::Target { pixels: &mut tmp, width: w, height: h, scroll: self.scroll, top: 0, premul: false, clip: None }, self.focus);
            self.leave(i);
            let pixels = &mut self.window.pixels()[..(W * H) as usize];
            for row in 0..h {
                let sy = top + row;
                if sy < BAR || sy >= H { continue; }
                let (a, b) = (x.max(0), (x + w).min(W));
                let src = (row * w + a - x) as usize;
                pixels[(sy * W + a) as usize..(sy * W + b) as usize].copy_from_slice(&tmp[src..src + (b - a) as usize]);
            }
        }
    }

    /// When the next timer of any page (shown or framed) is due.
    fn next_due(&self) -> Option<u64> {
        self.frames.iter().filter_map(|f| f.timer).chain(self.timer).min()
    }

    /// Runs the timers that are due, on every page.
    fn tick_all(&mut self) {
        let now = sex_pdx::get_ticks();
        if self.timer.is_some_and(|t| t <= now) { self.tick(); }
        let mut i = 0;
        while i < self.frames.len() {
            if self.frames[i].timer.is_some_and(|t| t <= now) { self.enter(i); self.tick(); self.leave(i); }
            i += 1;
        }
        self.settle();
    }

    /// After any work: frames follow the page, messages are delivered, and a frame's
    /// change shows.
    fn settle(&mut self) {
        if core::mem::take(&mut self.frames_check) { self.sync_frames(); }
        if !self.frames.is_empty() { self.deliver(); }
        if core::mem::take(&mut self.frame_dirty) { self.draw(); }
    }

    fn draw(&mut self) {
        // A page in a frame is drawn as part of the page around it.
        if self.in_frame.is_some() { self.frame_dirty = true; return; }
        let start = sex_pdx::get_ticks();
        let pixels = &mut self.window.pixels()[..(W * H) as usize];
        let bar = match (&self.editing, self.status) {
            (Some(t), _) => t.clone(),
            (None, Some(s)) => alloc::format!("{}  {}", s, self.url),
            (None, None) => if self.page.title.is_empty() { self.url.clone() } else { alloc::format!("{} — {}", self.page.title, self.url) },
        };
        sex_web::frame(&self.page, &mut self.fonts, pixels, W, H, self.scroll, self.focus, &bar, self.editing.is_some());
        self.paint_frames();
        self.window.show_pixels(W as usize, H as usize);
        self.full_redraw = false;
        sex_pdx::serial_record!("[web.shown] w={} h={} scroll={} focus={} items={} ticks={}", W, H, self.scroll, self.focus.map_or(-1, |f| f as i64), self.page.items.len(), sex_pdx::get_ticks() - start);
    }

    /// Fetches `url` into `self.buf`, following redirects. Returns the final address
    /// and the body's range, or a message.
    fn fetch(&mut self, url: &str) -> Result<(String, core::ops::Range<usize>), &'static [u8]> { self.fetch_up_to(url, SOURCE_MAX) }

    fn fetch_up_to(&mut self, url: &str, max: usize) -> Result<(String, core::ops::Range<usize>), &'static [u8]> {
        let (url, status, body) = self.fetch_status(url, max)?;
        if status >= 400 { return Err(b"The site answered with an error."); }
        Ok((url, body))
    }

    /// Like `fetch`, also giving the status, and taking an error answer as an answer.
    fn fetch_status(&mut self, url: &str, max: usize) -> Result<(String, u16, core::ops::Range<usize>), &'static [u8]> {
        let r = self.http("GET", url, &[], &[], max, None)?;
        self.buf = r.body;
        Ok((r.url, r.status, 0..self.buf.len()))
    }

    /// One HTTP/1.1 exchange, following redirects, with the window's cookies.
    ///
    /// `script` is the requesting page's origin for a script's request. Then cookies
    /// go only to (and come only from) that same origin, a request elsewhere says
    /// where it comes from (`Origin`), and its reply reaches the script only if the
    /// site allows that origin (`Access-Control-Allow-Origin`). So a page can't read
    /// another site's private pages with the person's cookies.
    fn http(&mut self, method: &str, url: &str, headers: &[(&str, &str)], body: &[u8], max: usize, script: Option<&str>) -> Result<Loaded, &'static [u8]> {
        let mut p = Pending::new(0, method, url, headers, body, max, script);
        loop {
            let mut ex = self.send(&mut p, true)?;
            while !ex.poll(&mut self.window).map_err(net::message)? { if ex.idle() > 64 { pause() } else if ex.idle() > 0 { sex_pdx::sys_yield() } }
            let r = ex.finish().map_err(net::message)?;
            if let Some(done) = self.landed(&mut p, r) { return done; }
        }
    }

    /// Starts `p`'s current hop. `patient` waits while every flow is someone else's.
    fn send(&mut self, p: &mut Pending, patient: bool) -> Result<net::Exchange, &'static [u8]> {
        if p.hops >= MAX_REDIRECTS { return Err(b"Too many redirects."); }
        let target = sex_fetch::Url::parse(&p.url).map_err(|_| net::message(net::Error::Address))?;
        let cross = p.script.as_deref().filter(|o| *o != target.origin());
        let cookie = if cross.is_some() { None } else { self.jar.borrow().header(&target, wall()) };
        let mut sent: Vec<(&str, &str)> = p.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).filter(|(k, _)| !sex_fetch::forbidden_header(k)).collect();
        if let Some(o) = cross { sent.push(("Origin", o)); }
        // The page's own address, or while a page loads the one it was opened from.
        let from = match &self.nav_from { Some(loading) => loading.as_deref(), None => Some(self.url.as_str()) };
        let referrer = from.and_then(|f| sex_fetch::referrer(f, &target));
        let req = sex_fetch::Request { method: &p.method, url: &p.url, headers: &sent, body: &p.body, trusted_origin: cross.is_some(), referrer: referrer.as_deref() };
        p.begun = sex_pdx::get_ticks();
        let started = if patient { net::Exchange::start_patiently(&mut self.window, &req, cookie.as_deref(), p.max) } else { net::Exchange::start(&mut self.window, &req, cookie.as_deref(), p.max) };
        started.map_err(|e| if e == net::Error::Busy { BUSY } else { net::message(e) })
    }

    /// Takes the reply to `p`'s current hop: its cookies, then either the final answer
    /// or, for a redirect, `None` with `p` aimed at the next address.
    fn landed(&mut self, p: &mut Pending, r: net::Response) -> Option<Result<Loaded, &'static [u8]>> {
        let Ok(target) = sex_fetch::Url::parse(&p.url) else { return Some(Err(net::message(net::Error::Address))) };
        let cross = p.script.as_deref().filter(|o| *o != target.origin()).map(String::from);
        if cross.is_none() {
            let mut jar = self.jar.borrow_mut();
            for c in r.head.all("set-cookie") { jar.store(&target, c, wall()); }
        }
        sex_pdx::serial_record!("[web.http] method={} status={} bytes={} encoding={} chunked={} cookies={} ticks={} url={}", p.method, r.head.status, r.body.len(),
            r.head.get("content-encoding").unwrap_or("none"), r.head.get("transfer-encoding").is_some_and(|t| t.contains("chunked")) as u8, self.jar.borrow().len(), sex_pdx::get_ticks() - p.begun, p.url);
        if r.head.is_redirect() {
            if let Some(next) = r.head.get("location").and_then(|l| sex_fetch::join(&p.url, l)) {
                // 303, and 301/302 after a POST, continue as a GET without the body.
                if r.head.status == 303 || (matches!(r.head.status, 301 | 302) && p.method == "POST") { p.method = String::from("GET"); p.body.clear(); }
                p.url = String::from(sex_html::url::fragment(&next).0);
                p.hops += 1;
                return None;
            }
        }
        if let Some(o) = cross {
            let allowed = r.head.get("access-control-allow-origin").map(str::trim);
            if allowed != Some("*") && allowed != Some(o.as_str()) {
                sex_pdx::serial_record!("[web.cors] blocked origin={} url={}", o, p.url);
                return Some(Err(b"The site doesn't let this page read it."));
            }
        }
        let lines = r.head.lines(true);
        Some(Ok(Loaded { url: core::mem::take(&mut p.url), status: r.head.status, headers: lines, body: r.body }))
    }

    fn open(&mut self, url: &str, remember: bool) {
        let post = self.post.take();
        // The page this one is opened from, for its `Referer`: a link or script on it,
        // or the page around an iframe. A typed address has none.
        let from = self.referrer_next.take();
        if remember && !self.url.is_empty() && self.in_frame.is_none() { self.history.push(self.url.clone()); }
        // Same page, other fragment: just scroll.
        let (here, _) = sex_html::url::fragment(&self.url);
        let (there, frag) = sex_html::url::fragment(url);
        if post.is_none() && !here.is_empty() && here == there && frag.is_some() {
            self.url = String::from(url);
            self.scroll_to(frag);
            return;
        }
        if self.in_frame.is_none() { self.status = Some("Loading…"); }
        self.url = String::from(url);
        // Leaving the page stops its sound and pictures, and their downloads, and its frames.
        self.drop_frames();
        self.media.clear();
        for mut l in core::mem::take(&mut self.media_loads) { l.ex.cancel(&mut self.window); }
        self.draw();
        let begun = sex_pdx::get_ticks();
        let referrer = from.as_deref().zip(sex_fetch::Url::parse(url).ok()).and_then(|(f, t)| sex_fetch::referrer(f, &t)).unwrap_or_default();
        self.nav_from = Some(from);
        let (html, base) = if let Some(page) = built_in(if url.is_empty() { "about:welcome" } else { url }) {
            (String::from(page), String::from(if url.is_empty() { "about:welcome" } else { url }))
        } else if let Some(file) = url.strip_prefix('@') {
            self.buf.resize(SOURCE_MAX, 0);
            match files::read(&mut self.window, file.as_bytes(), &mut self.buf) {
                Ok(n) => (text_of(&self.buf[..n]), String::from(url)),
                Err(e) => (message_page("Can't open that file", core::str::from_utf8(files::message(e)).unwrap_or(""), url), String::from(url)),
            }
        } else if url.starts_with("http://") || url.starts_with("https://") {
            let got = match &post {
                // A form's POST: the answer is the next page.
                Some(body) => self.http("POST", url, &[("Content-Type", "application/x-www-form-urlencoded")], body.as_bytes(), SOURCE_MAX, None)
                    .and_then(|r| if r.status >= 400 { Err(&b"The site answered with an error."[..]) } else { self.buf = r.body; Ok((r.url, 0..self.buf.len())) }),
                None => self.fetch(url),
            };
            match got {
                Ok((final_url, body)) => { let t = text_of(&self.buf[body]); (t, final_url) }
                Err(why) => (message_page("Can't open this page", core::str::from_utf8(why).unwrap_or(""), url), String::from(url)),
            }
        } else {
            (message_page("Can't open that address", "Addresses start with https://, http://, or @ for a file.", url), String::from(url))
        };
        self.buf = Vec::new();
        self.nav_from = None;
        if base.starts_with("http") { self.url = base.clone() + frag.map(|f| alloc::format!("#{}", f)).as_deref().unwrap_or(""); }
        let dom = sex_html::parse::parse(&html);
        drop(html);
        let fetched = sex_pdx::get_ticks();
        // Style sheets and pictures from the network, within limits.
        let mut sheets: Vec<String> = Vec::new();
        self.fonts.images.clear();
        self.fonts.svgs.clear();
        self.fonts.moving.clear();
        self.fonts.frames_shown = 0;
        if base.starts_with("http") {
            let links: Vec<String> = sex_html::stylesheet_links(&dom).into_iter().take(MAX_SHEETS).filter_map(|h| sex_html::url::resolve(&base, h)).collect();
            for (i, link) in links.iter().enumerate() {
                self.status = Some("Loading style…");
                if i == 0 { self.draw(); }
                if let Ok((_, body)) = self.fetch(link) { sheets.push(text_of(&self.buf[body])); }
            }
            let sources: Vec<String> = sex_html::image_sources(&dom).into_iter().map(String::from).collect();
            let mut seen: Vec<&str> = Vec::new();
            for src in sources.iter() {
                if seen.len() >= MAX_IMAGES || seen.contains(&src.as_str()) { continue; }
                seen.push(src);
                let Some(abs) = sex_html::url::resolve(&base, src) else { continue };
                self.status = Some("Loading pictures…");
                if let Ok((_, body)) = self.fetch(&abs) {
                    let bytes = core::mem::take(&mut self.buf);
                    self.fonts.add_picture(src, &bytes[body], (W - 16) as usize, 2000);
                    self.buf = bytes;
                }
            }
            self.buf = Vec::new();
        }
        let scripts = self.scripts(&dom, &base);
        // Inline handlers (`onclick="..."`), `javascript:` links and forms need the engine too.
        let handlers = dom.descendants(sex_html::dom::Dom::ROOT).into_iter().any(|n| match &dom.nodes[n].kind {
            sex_html::dom::Kind::Element { tag, attrs, .. } => matches!(tag.as_str(), "input" | "textarea" | "select" | "form")
                || attrs.iter().any(|(k, v)| k.starts_with("on") || (k == "href" && v.trim_start().starts_with("javascript:"))),
            _ => false,
        });
        self.sheets = sheets;
        self.timer = None;
        self.timeline = sex_html::Timeline::default();
        let mut doc = Document::new(dom, &base, self.view, clock);
        doc.measure = sex_web::canvas::measure_css;
        doc.cookies = Some(self.jar.clone());
        doc.wall = wall;
        doc.referrer = referrer;
        doc.parent_origin = self.parent.as_deref().and_then(|p| sex_fetch::Url::parse(p).ok()).map(|u| u.origin());
        doc.pictures = self.fonts.images.clone();
        self.doc = Doc::Plain(doc);
        self.canvases = sex_web::canvas::Canvases::new();
        let script_begun = sex_pdx::get_ticks();
        if !scripts.is_empty() || handlers { self.run_scripts(scripts); }
        let laid = sex_pdx::get_ticks();
        self.media_scan();
        if !self.fonts.moving.is_empty() { self.timer = Some(sex_pdx::get_ticks()); }
        sex_pdx::serial_record!("[web.pictures] count={} moving={}", self.fonts.images.len(), self.fonts.moving.len());
        self.relayout();
        sex_pdx::serial_record!("[web.loaded] bytes_ok=1 items={} links={} height={} sheets={} images={} fetch_ticks={} layout_ticks={} script_ticks={} scripted={}",
            self.page.items.len(), self.page.links.len(), self.page.height, self.sheets.len(), self.fonts.images.len(), script_begun - begun,
            sex_pdx::get_ticks() - laid, laid - script_begun, matches!(self.doc, Doc::Scripted(_)) as u8);
        let _ = fetched;
        self.scroll = 0;
        self.focus = None;
        if self.in_frame.is_some() { return; }
        self.status = None;
        let title: Vec<u8> = self.page.title.bytes().take(16).collect();
        self.window.title(if title.is_empty() { b"Web" } else { &title });
        self.scroll_to(frag);
        self.sync_frames();
    }

    /// The page's scripts in the order they run: classic scripts as they appear, then
    /// deferred ones (`defer` and modules) in document order. Inline text, or fetched
    /// from `src`. Classic scripts marked `nomodule` are skipped, as module browsers do.
    #[inline(never)]
    fn scripts(&mut self, dom: &sex_html::dom::Dom, base: &str) -> Vec<PageScript> {
        let mut now = Vec::new();
        let mut later = Vec::new();
        for n in dom.descendants(sex_html::dom::Dom::ROOT) {
            if now.len() + later.len() >= MAX_SCRIPTS { break; }
            if dom.tag(n) != Some("script") { continue; }
            let ty = dom.attr(n, "type").unwrap_or("").trim().to_ascii_lowercase();
            let module = ty == "module";
            if ty == "importmap" { self.import_map.extend(sex_js::document::import_map(base, &dom.text_content(n))); continue; }
            if !(ty.is_empty() || ty.contains("javascript") || module || ty.contains("ecmascript")) { continue; }
            if !module && dom.attr(n, "nomodule").is_some() { continue; }
            let deferred = module || (dom.attr(n, "defer").is_some() && dom.attr(n, "src").is_some());
            let got = match dom.attr(n, "src") {
                // A built-in page's scripts are built in too.
                Some(src) if base.starts_with("about:") => built_in(&alloc::format!("about:{}", src.trim_start_matches("./"))).map(|code| (String::from(code), alloc::format!("about:{}", src))),
                Some(src) if base.starts_with('@') => {
                    let Some(path) = local_path(base, src) else { continue };
                    self.buf.resize(SOURCE_MAX, 0);
                    let r = files::read(&mut self.window, path.as_bytes(), &mut self.buf).ok().map(|n| (text_of(&self.buf[..n]), alloc::format!("@{}", path)));
                    self.buf = Vec::new();
                    r
                }
                Some(src) => {
                    let Some(abs) = sex_html::url::resolve(base, src) else { continue };
                    if !abs.starts_with("http") { continue; }
                    self.status = Some("Loading scripts…");
                    let r = self.fetch_up_to(&abs, SCRIPT_MAX).ok().map(|(url, body)| (text_of(&self.buf[body]), url));
                    self.buf = Vec::new();
                    r
                }
                None => Some((dom.text_content(n), alloc::format!("{}#script{}", base.split('#').next().unwrap_or(base), now.len() + later.len() + 1))),
            };
            let Some((source, name)) = got else { continue };
            let s = PageScript { source, name, module, node: (!module).then_some(n) };
            if deferred { later.push(s) } else { now.push(s) }
        }
        now.extend(later);
        now
    }

    #[inline(never)]
    fn run_scripts(&mut self, scripts: Vec<PageScript>) {
        let Doc::Plain(mut doc) = core::mem::replace(&mut self.doc, Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", self.view, clock))) else { return };
        doc.import_map = core::mem::take(&mut self.import_map);
        let started = sex_pdx::get_ticks();
        let mut script = match Script::new(doc, sex_js::Limits::default()) {
            Ok(s) => s,
            Err(e) => {
                sex_pdx::serial_record!("[web.script] engine={:?}", e);
                return;
            }
        };
        sex_pdx::serial_record!("[web.script] engine_ticks={} memory={}", sex_pdx::get_ticks() - started, script.memory_bytes());
        for s in scripts.iter() {
            let r = if s.module {
                self.load_modules(&mut script, s);
                script.eval_module(&s.source, &s.name)
            } else {
                if let Some(n) = s.node { let _ = script.dispatch(&alloc::format!("script\t{}", n)); }
                let r = script.eval(&s.source, &s.name);
                if s.node.is_some() { let _ = script.dispatch("script\t"); }
                r
            };
            if let Err(sex_js::Error::Trapped(why)) = &r { sex_pdx::serial_record!("[web.script] trapped={}", why); break; }
        }
        let _ = script.dispatch("ready");
        self.script_scroll = 0;
        self.doc = Doc::Scripted(alloc::boxed::Box::new(script));
        self.after_script(false);
    }

    /// Fetches every module a module script imports, a level of the graph at a time,
    /// until the engine is missing none (or `MAX_MODULES` are loaded). One that will not
    /// load is left out; importing it is then the script's error.
    #[inline(never)]
    fn load_modules(&mut self, script: &mut Script<Document>, s: &PageScript) {
        for _ in 0..MAX_MODULES {
            let Ok(need) = script.module_needs(&s.source, &s.name) else { return };
            if need.is_empty() { return; }
            for url in need {
                if script.page().modules.len() >= MAX_MODULES { return; }
                self.status = Some("Loading scripts…");
                let body = if url.starts_with("http") { self.fetch_up_to(&url, SCRIPT_MAX).ok().map(|(_, b)| text_of(&self.buf[b])) } else { None };
                self.buf = Vec::new();
                sex_pdx::serial_record!("[web.module] ok={} bytes={} url={}", body.is_some() as u8, body.as_ref().map_or(0, |b| b.len()), url);
                // A failed one is still recorded, so it is not asked for again.
                script.page_mut().modules.insert(url.clone(), body.unwrap_or_else(|| sex_js::document::failed_module(&url)));
            }
        }
    }

    /// After any script ran: print its console, take a navigation, and lay out again
    /// when it changed the page. Returns true if it drew.
    #[inline(never)]
    fn after_script(&mut self, draw: bool) -> bool {
        self.media_requests();
        let doc = self.doc.get();
        for (level, text) in doc.log.drain(..) {
            for line in text.lines().take(8) { sex_pdx::serial_record!("[web.console] level={} {}", level, line); }
        }
        if let Some(url) = doc.navigate.take() {
            self.post = doc.navigate_post.take();
            self.referrer_next = Some(self.url.clone());
            self.open(&url, true);
            return true;
        }
        // Fetches are served on the next tick, once this script has returned.
        if (!doc.fetches.is_empty() || !doc.modules_wanted.is_empty()) && matches!(self.doc, Doc::Scripted(_)) { self.timer = Some(sex_pdx::get_ticks()); }
        let doc = self.doc.get();
        let queue = core::mem::take(&mut doc.canvas_queue);
        let dirty = core::mem::take(&mut doc.dirty);
        let doc = match &mut self.doc { Doc::Plain(d) => d, Doc::Scripted(s) => s.page_mut() };
        self.canvases.run_queue(&mut self.fonts, &doc.dom, queue);
        let mut changed: Vec<usize> = core::mem::take(&mut self.canvases.drew);
        changed.extend(sex_web::canvas::present_gl_nodes(&mut self.fonts, &mut doc.gl));
        if dirty { self.relayout(); }
        if !draw || (!dirty && changed.is_empty()) { return false; }
        // Only canvases drew: put just them over the last frame when that is enough.
        if !dirty && !self.full_redraw && self.in_frame.is_none() {
            let names: Vec<String> = changed.iter().map(|n| alloc::format!("canvas:{n}")).collect();
            let start = sex_pdx::get_ticks();
            let pixels = &mut self.window.pixels()[..(W * H) as usize];
            if sex_web::repaint_pictures(&self.page, &mut self.fonts, pixels, W, H, self.scroll, &names) {
                self.window.show_pixels(W as usize, H as usize);
                sex_pdx::serial_record!("[web.shown] w={} h={} scroll={} focus={} items={} ticks={} canvases={}", W, H, self.scroll, self.focus.map_or(-1, |f| f as i64), self.page.items.len(), sex_pdx::get_ticks() - start, names.len());
                return true;
            }
        }
        self.draw();
        true
    }

    /// Styles and lays the page out again from the tree.
    #[inline(never)]
    fn relayout(&mut self) {
        let sheet_refs: Vec<&str> = self.sheets.iter().map(|s| s.as_str()).collect();
        let doc = match &mut self.doc { Doc::Plain(d) => d, Doc::Scripted(s) => s.page_mut() };
        let (styles, animating) = sex_html::styles_at(&doc.dom, &sheet_refs, self.view.0, self.view.1, sex_pdx::get_ticks(), &mut self.timeline);
        self.animating = animating;
        self.hover_css = sheet_refs.iter().any(|s| s.contains(":hover"))
            || doc.dom.descendants(sex_html::dom::Dom::ROOT).into_iter().any(|i| doc.dom.tag(i) == Some("style") && doc.dom.text_content(i).contains(":hover"));
        self.page = sex_html::layout::layout_in(&doc.dom, &styles, self.view.0, self.view.1, &self.fonts);
        sex_web::svg::draw_inline(&self.page, &doc.dom, &styles, &mut self.fonts);
        doc.set_boxes(&self.page.boxes);
        doc.set_layers(&self.page.layers);
        if self.in_frame.is_none() { self.frames_check = true; }
        self.scroll = self.scroll.min(self.max_scroll());
        if self.focus.map_or(false, |f| f >= self.page.links.len()) { self.focus = None; }
        // Ask for the next tick soon: the change may have started timers.
        if matches!(self.doc, Doc::Scripted(_)) && self.timer.is_none() { self.timer = Some(sex_pdx::get_ticks()); }
        self.next_frame();
    }

    /// While a CSS animation runs, the next tick is at most a frame away.
    fn next_frame(&mut self) {
        if self.animating {
            let frame = sex_pdx::get_ticks() + ANIMATION_FRAME_MS;
            self.timer = Some(self.timer.map_or(frame, |t| t.min(frame)));
        }
    }

    /// Runs due timers and animation frames.
    #[inline(never)]
    fn tick(&mut self) {
        self.serve_imports();
        self.serve_fetches();
        self.sync_scroll();
        let media_next = self.media_tick();
        let (moved, pictures_next) = self.fonts.step_pictures(sex_pdx::get_ticks());
        if moved && !self.animating { self.draw(); }
        if moved && self.fonts.frames_shown <= 8 { sex_pdx::serial_record!("[web.picture.frames] n={}", self.fonts.frames_shown); }
        if self.animating {
            self.timer = None;
            self.relayout();
            self.draw();
        }
        if let Some(script) = self.doc.script() {
            let next = script.dispatch("tick");
            self.timer = match next { Ok(ms) if ms >= 0 => Some(sex_pdx::get_ticks() + ms as u64), _ => None };
            self.next_frame();
            if let Err(sex_js::Error::Trapped(why)) = next { sex_pdx::serial_record!("[web.script] trapped={}", why); self.doc = Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", self.view, clock)); }
            self.after_script(true);
        } else {
            self.timer = None;
            self.next_frame();
        }
        // A request carried out during this tick (from a media event handler, or the
        // tick's own script) is played from the next pass.
        let asked = core::mem::take(&mut self.media_asked);
        let waiting = self.media.iter().any(|m| m.state == media::State::Waiting);
        // Downloads still coming are taken in every couple of milliseconds.
        let loading = !self.media_loads.is_empty();
        if let Some(ms) = if waiting || asked { Some(0) } else if loading { Some(media_next.map_or(2, |n| n.min(2))) } else { media_next } {
            let due = sex_pdx::get_ticks() + ms;
            self.timer = Some(self.timer.map_or(due, |t| t.min(due)));
        }
        // Animated GIF and WebP pictures.
        if let Some(due) = pictures_next { self.timer = Some(self.timer.map_or(due, |t| t.min(due))); }
    }

    /// Finds the page's video and audio elements and shows their posters.
    #[inline(never)]
    fn media_scan(&mut self) {
        let nodes = media::elements(&self.doc.get().dom);
        for node in nodes {
            if self.media.iter().any(|m| m.node == node) { continue; }
            let m = media::Media::new(&self.doc.get().dom, node);
            let poster = if m.audio { None } else { self.doc.get().dom.attr(node, "poster").map(String::from) };
            if let Some(poster) = poster.filter(|p| !p.trim().is_empty()) {
                let (status, body) = self.load_get(poster.trim());
                if (200..300).contains(&status) {
                    if let Some(img) = sex_web::decode_image(&body, media::MAX_W, media::MAX_H) { media::Media::poster(&mut self.fonts, &m.picture_name(), img); }
                }
            }
            self.media.push(m);
        }
        if self.media.iter().any(|m| m.state == media::State::Waiting) { self.timer = Some(sex_pdx::get_ticks()); }
    }

    /// Takes the page's requests to its video and audio elements.
    fn media_requests(&mut self) {
        let asked = core::mem::take(&mut self.doc.get().media);
        let appends = core::mem::take(&mut self.doc.get().media_bytes);
        if asked.is_empty() && appends.is_empty() { return; }
        for (node, verb, arg) in asked {
            let dom = &self.doc.get().dom;
            let i = match self.media.iter().position(|m| m.node == node) {
                Some(i) => i,
                None => { self.media.push(media::Media::new(dom, node)); self.media.len() - 1 }
            };
            if verb == "play" { self.hush_media(i); }
            let dom = &self.doc.get().dom;
            self.media[i].request(dom, &verb, &arg);
        }
        for (node, index, bytes) in appends {
            let Some(m) = self.media.iter_mut().find(|m| m.node == node) else { continue };
            let Some(mse) = m.mse.as_mut() else { continue };
            mse.append(node, index as usize, &bytes);
        }
        self.media_asked = true;
        self.timer = Some(sex_pdx::get_ticks());
    }

    /// Loads, plays and shows the page's media; tells the page what happened. Returns
    /// how soon to come back.
    #[inline(never)]
    fn media_tick(&mut self) -> Option<u64> {
        if self.media.is_empty() { return None; }
        let mut relayout = false;
        for i in 0..self.media.len() {
            if self.media[i].state != media::State::Waiting { continue; }
            let node = self.media[i].node;
            // A new source replaces one still coming.
            if let Some(k) = self.media_loads.iter().position(|l| l.node == node) { let mut old = self.media_loads.remove(k); old.ex.cancel(&mut self.window); }
            let src = self.media[i].src.clone();
            let begun = sex_pdx::get_ticks();
            let ask = sex_js::FetchAsk { id: 0, url: src.clone(), method: String::new(), headers: String::new(), body: Vec::new() };
            match self.local(&ask) {
                // Built-in pages and files are here at once.
                Ok(r) => {
                    sex_pdx::serial_record!("[web.media.load] node={} status={} bytes={} ticks={} src={}", node, r.status, r.body.len(), sex_pdx::get_ticks() - begun, src);
                    relayout |= self.media_loaded(i, r.status, r.body);
                }
                Err(mut p) => {
                    p.max = MEDIA_MAX;
                    // An element's load, not a script's: no CORS, and the site's cookies go.
                    p.script = None;
                    match self.send(&mut p, false) {
                        Ok(ex) => {
                            self.media[i].state = media::State::Loading;
                            self.media[i].events.push("loadstart");
                            self.media_loads.push(MediaLoad { node, p, ex, body: sex_media::Growing::shared(), streaming: false, retry_at_end: false, opened: false });
                        }
                        // Every flow is busy: try again on a later tick.
                        Err(e) if e == BUSY => {}
                        Err(_) => { self.media[i].state = media::State::Failed; self.media[i].events.push("error"); }
                    }
                }
            }
        }
        relayout |= self.media_downloads();
        let mut next: Option<u64> = None;
        let mut changed: Vec<String> = Vec::new();
        for m in self.media.iter_mut() {
            let (drew, wait) = m.step(&mut self.fonts);
            if drew { changed.push(m.picture_name()); }
            if let Some(w) = wait { next = Some(next.map_or(w, |n| n.min(w))); }
            relayout |= core::mem::take(&mut m.resized);
            // A MediaSource still filling: look again soon.
            if (m.player.is_none() || m.pending.is_some()) && m.mse.is_some() && m.state != media::State::Failed { next = Some(next.map_or(10, |n| n.min(10))); }
        }
        if relayout {
            self.relayout();
            self.draw();
        } else if !changed.is_empty() {
            let pixels = &mut self.window.pixels()[..(W * H) as usize];
            if !self.full_redraw && self.in_frame.is_none() && sex_web::repaint_pictures(&self.page, &mut self.fonts, pixels, W, H, self.scroll, &changed) {
                self.window.show_pixels(W as usize, H as usize);
            } else {
                self.draw();
            }
        }
        // Tell the page, which may answer by changing it.
        let mut reports: Vec<String> = Vec::new();
        for m in self.media.iter_mut() {
            if let Some(mse) = m.mse.as_mut() { reports.append(&mut mse.replies); }
            if let Some(r) = m.report() { reports.push(r); }
        }
        if !reports.is_empty() {
            if let Some(script) = self.doc.script() {
                for r in &reports {
                    if let Err(sex_js::Error::Trapped(why)) = script.dispatch(r) { sex_pdx::serial_record!("[web.script] trapped={}", why); break; }
                }
                self.after_script(true);
            }
        }
        next
    }

    /// Element `i` is about to play: every other one gives up the sound ring first,
    /// since a program has one.
    fn hush_media(&mut self, i: usize) {
        for (j, m) in self.media.iter_mut().enumerate() {
            if j != i { m.hush(); }
        }
    }

    /// K plays or pauses the page's first video or audio, J and L go back and forward
    /// 5 s. True if there was one.
    fn media_key(&mut self, code: u8) -> bool {
        let Some(i) = self.media.iter().position(|m| m.state == media::State::Ready) else { return false };
        if code == 0x25 && self.media[i].stopped() { self.hush_media(i); }
        let dom = &self.doc.get().dom;
        let m = &mut self.media[i];
        match code {
            0x25 => m.toggle(dom),
            0x24 => m.seek_by(dom, -5000),
            0x26 => m.seek_by(dom, 5000),
            _ => return false,
        }
        sex_pdx::serial_record!("[web.media.key] code={} node={}", code, m.node);
        self.timer = Some(sex_pdx::get_ticks());
        true
    }

    /// Loads the modules that `import()` calls wait for (and what those import), then
    /// lets the imports go on. A module that can't be loaded is empty, so its import fails.
    #[inline(never)]
    fn serve_imports(&mut self) {
        for _ in 0..MAX_MODULES {
            let Doc::Scripted(s) = &mut self.doc else { return };
            let wanted = s.modules_wanted();
            if wanted.is_empty() { return; }
            for url in wanted {
                let full = matches!(&self.doc, Doc::Scripted(s) if s.page().modules.len() >= MAX_MODULES);
                let body = if url.starts_with("http") && !full { self.fetch_up_to(&url, SCRIPT_MAX).ok().map(|(_, b)| text_of(&self.buf[b])) } else { None };
                self.buf = Vec::new();
                sex_pdx::serial_record!("[web.module] import=1 ok={} bytes={} url={}", body.is_some() as u8, body.as_ref().map_or(0, |b| b.len()), url);
                let Doc::Scripted(s) = &mut self.doc else { return };
                s.page_mut().modules.insert(url.clone(), body.unwrap_or_else(|| sex_js::document::failed_module(&url)));
            }
            let Doc::Scripted(s) = &mut self.doc else { return };
            if let Err(sex_js::Error::Trapped(why)) = s.modules_ready() {
                sex_pdx::serial_record!("[web.script] trapped={}", why);
                self.doc = Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", self.view, clock));
                return;
            }
        }
    }

    /// Loads what the page's scripts asked `fetch` and XHR for, and tells each script.
    /// Network requests run side by side, up to `PARALLEL` at once, each reported as it
    /// lands; requests that a script makes on hearing of one join in.
    #[inline(never)]
    fn serve_fetches(&mut self) {
        let mut waiting: Vec<Pending> = Vec::new();
        let mut going: Vec<(Pending, net::Exchange)> = Vec::new();
        loop {
            let asked = match &mut self.doc { Doc::Scripted(s) => core::mem::take(&mut s.page_mut().fetches), _ => { self.cancel_all(going); return } };
            for ask in asked {
                match self.local(&ask) {
                    Ok(r) => { if !self.answer(&ask.id, &ask.method, &ask.url, sex_pdx::get_ticks(), r) { self.cancel_all(going); return; } }
                    Err(p) => waiting.push(p),
                }
            }
            // Start what fits; one the service has no flow for waits its turn.
            while going.len() < PARALLEL && !waiting.is_empty() {
                let mut p = waiting.remove(0);
                match self.send(&mut p, going.is_empty()) {
                    Ok(ex) => going.push((p, ex)),
                    Err(BUSY) => { waiting.insert(0, p); break; }
                    Err(_) => { let (id, m, u, t) = (p.id, p.method.clone(), p.asked.clone(), p.since); if !self.answer(&id, &m, &u, t, failed(&p.url)) { self.cancel_all(going); return; } }
                }
            }
            if going.is_empty() && waiting.is_empty() { return; }
            let mut quiet = u32::MAX;
            // Media downloads keep moving too: one may hold the name resolver, which
            // answers this program one question at a time.
            for l in self.media_loads.iter_mut() { let _ = l.ex.poll(&mut self.window); }
            let mut i = 0;
            while i < going.len() {
                let step = going[i].1.poll(&mut self.window);
                quiet = quiet.min(going[i].1.idle());
                match step {
                    Ok(false) => { i += 1; continue; }
                    Ok(true) => {}
                    Err(_) => {}
                }
                let (mut p, ex) = going.remove(i);
                let landed = match step { Ok(_) => match ex.finish() { Ok(r) => self.landed(&mut p, r), Err(_) => Some(Err(&b""[..])) }, Err(_) => Some(Err(&b""[..])) };
                match landed {
                    None => waiting.insert(0, p),
                    Some(r) => {
                        let r = r.unwrap_or_else(|_| failed(&p.url));
                        if !self.answer(&p.id, &p.method, &p.asked, p.since, r) { self.cancel_all(going); return; }
                    }
                }
                quiet = 0;
            }
            if quiet > 64 { pause(); } else if quiet > 0 { sex_pdx::sys_yield(); }
        }
    }

    /// A whole body for media element `i`: open it, or fail it.
    fn media_loaded(&mut self, i: usize, status: u16, body: Vec<u8>) -> bool {
        if (200..300).contains(&status) && !body.is_empty() {
            if self.media[i].want_play { self.hush_media(i); }
            self.media[i].open(body)
        } else {
            self.media[i].state = media::State::Failed;
            self.media[i].events.push("error");
            false
        }
    }

    /// Takes in what the media downloads have brought, for a few milliseconds, and
    /// opens each element once enough of its source is here. True when a picture size
    /// became known (lay out again).
    #[inline(never)]
    fn media_downloads(&mut self) -> bool {
        let mut relayout = false;
        let begun = sex_pdx::get_ticks();
        let mut k = 0;
        while k < self.media_loads.len() {
            // An element gone, failed or given a new source no longer wants this one.
            let Some(i) = self.media.iter().position(|m| m.node == self.media_loads[k].node && matches!(m.state, media::State::Loading | media::State::Ready)) else {
                let mut gone = self.media_loads.remove(k); gone.ex.cancel(&mut self.window); continue;
            };
            let l = &mut self.media_loads[k];
            // Read while bytes keep coming, within the budget.
            let polled = loop {
                match l.ex.poll(&mut self.window) {
                    Err(e) => break Err(e),
                    Ok(true) => break Ok(true),
                    Ok(false) if l.ex.idle() > 0 || sex_pdx::get_ticks() - begun >= MEDIA_LOAD_BUDGET_MS => break Ok(false),
                    Ok(false) => {}
                }
            };
            if !l.streaming {
                if let Some(head) = l.ex.head() {
                    if (200..300).contains(&head.status) {
                        let mut g = l.body.borrow_mut();
                        if let Some(total) = l.ex.stream(&mut g.bytes) {
                            g.total = total;
                            l.streaming = true;
                            drop(g);
                            sex_pdx::serial_record!("[web.media.stream] node={} total={} url={}", l.node, total.map_or(-1, |t| t as i64), l.p.url);
                            if let Ok(target) = sex_fetch::Url::parse(&l.p.url) {
                                let mut jar = self.jar.borrow_mut();
                                for c in head.all("set-cookie") { jar.store(&target, c, wall()); }
                            }
                        }
                    }
                }
            }
            let l = &mut self.media_loads[k];
            if l.streaming {
                let ok = l.ex.stream(&mut l.body.borrow_mut().bytes).is_some();
                let done = matches!(polled, Ok(true)) || polled.is_err() || !ok;
                let have = l.body.borrow().bytes.len() as u64;
                if done {
                    l.body.borrow_mut().done = true;
                    sex_pdx::serial_record!("[web.media.load] node={} status=200 bytes={} ticks={} streamed=1 ok={} shown={} src={}", l.node, have, sex_pdx::get_ticks() - l.p.since, (polled.is_ok() && ok) as u8, self.media[i].frames, l.p.url);
                }
                let opened = l.opened;
                let ready = done || have >= sex_media::AHEAD + 64 * 1024 || l.body.borrow().total.is_some_and(|t| have >= t);
                if !opened && ready && (!l.retry_at_end || done) {
                    let body = l.body.clone();
                    let last = done;
                    if self.media[i].want_play { self.hush_media(i); }
                    relayout |= self.media[i].open_growing(body, last);
                    let l = &mut self.media_loads[k];
                    if self.media[i].state == media::State::Ready {
                        l.opened = true;
                        sex_pdx::serial_record!("[web.media.early] node={} have={} done={}", l.node, have, done as u8);
                    } else if !last {
                        // Not enough at the front (an MP4 indexed at its end): wait for all.
                        l.retry_at_end = true;
                        self.media[i].state = media::State::Loading;
                    }
                }
                if done { self.media_loads.remove(k); } else { k += 1; }
                continue;
            }
            // Read whole: a redirect, an error page, or a body that must be decoded first.
            match polled {
                Ok(false) => { k += 1; }
                Err(e) => {
                    let l = self.media_loads.remove(k);
                    sex_pdx::serial_record!("[web.media.load] node={} status=0 bytes=0 ticks={} why={} src={}", l.node, sex_pdx::get_ticks() - l.p.since, core::str::from_utf8(net::message(e)).unwrap_or("?"), l.p.url);
                    self.media[i].state = media::State::Failed; self.media[i].events.push("error");
                }
                Ok(true) => {
                    let mut l = self.media_loads.remove(k);
                    let r = match l.ex.finish() { Ok(r) => r, Err(_) => { self.media[i].state = media::State::Failed; self.media[i].events.push("error"); continue; } };
                    match self.landed(&mut l.p, r) {
                        // A redirect: on to the next address.
                        None => match self.send(&mut l.p, false) {
                            Ok(ex) => { self.media_loads.insert(k, MediaLoad { ex, body: sex_media::Growing::shared(), streaming: false, retry_at_end: false, opened: false, ..l }); k += 1; }
                            Err(_) => { self.media[i].state = media::State::Failed; self.media[i].events.push("error"); }
                        },
                        Some(Ok(done)) => {
                            sex_pdx::serial_record!("[web.media.load] node={} status={} bytes={} ticks={} streamed=0 src={}", l.node, done.status, done.body.len(), sex_pdx::get_ticks() - l.p.since, l.p.url);
                            self.media[i].state = media::State::Waiting;
                            relayout |= self.media_loaded(i, done.status, done.body);
                        }
                        Some(Err(_)) => { self.media[i].state = media::State::Failed; self.media[i].events.push("error"); }
                    }
                }
            }
        }
        relayout
    }

    /// A plain GET of a page's resource (a video, a poster): status and body. Like
    /// an element's load, it may come from another site without CORS, since the
    /// page's scripts never read the bytes.
    fn load_get(&mut self, url: &str) -> (u16, Vec<u8>) {
        let ask = sex_js::FetchAsk { id: 0, url: String::from(url), method: String::new(), headers: String::new(), body: Vec::new() };
        match self.local(&ask) {
            Ok(r) => (r.status, r.body),
            Err(p) => match self.http("GET", &p.url, &[], &[], FETCH_MAX, None) { Ok(r) => (r.status, r.body), Err(_) => (0, Vec::new()) },
        }
    }

    /// Lets go of every request still going, when the page that asked is gone.
    fn cancel_all(&mut self, going: Vec<(Pending, net::Exchange)>) { for (_, mut ex) in going { ex.cancel(&mut self.window); } }

    /// Tells the page one request's answer. False when the page's script fell over.
    fn answer(&mut self, id: &u32, method: &str, url: &str, since: u64, r: Loaded) -> bool {
        let n = r.body.len();
        sex_pdx::serial_record!("[web.fetch] id={} status={} bytes={} ticks={} method={} url={}", id, r.status, n, sex_pdx::get_ticks() - since, if method.is_empty() { "GET" } else { method }, url);
        let Doc::Scripted(s) = &mut self.doc else { return false };
        s.page_mut().keep_head(*id, &r.url, &r.headers);
        s.page_mut().keep_body(*id, r.status, r.body);
        if let Err(sex_js::Error::Trapped(why)) = s.dispatch(&alloc::format!("fetched\t{}\t{}\t{}", id, r.status, n)) {
            sex_pdx::serial_record!("[web.script] trapped={}", why);
            self.doc = Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", self.view, clock));
            return false;
        }
        true
    }

    /// A script's request that needs no network, answered now (status 0 when it could
    /// not be loaded), or the network request to make. Pages fetch from where they came
    /// from: built in, a file (`@`), or the network.
    fn local(&mut self, ask: &sex_js::FetchAsk) -> Result<Loaded, Pending> {
        let url = ask.url.as_str();
        let base = String::from(sex_html::url::fragment(&self.url).0);
        let done = |status: u16, body: Vec<u8>| Loaded { url: String::from(url), status, headers: String::new(), body };
        if url.starts_with("about:") || (base.starts_with("about:") && !url.contains(':')) {
            let name = if url.starts_with("about:") { String::from(url) } else { alloc::format!("about:{}", url.trim_start_matches("./")) };
            return Ok(match built_in(&name) { Some(text) => done(200, Vec::from(text.as_bytes())), None => done(404, Vec::new()) });
        }
        if let Some(path) = local_path(&base, url) {
            return Ok(match files::size(&mut self.window, path.as_bytes()) {
                Ok(n) if n as usize <= FETCH_MAX => {
                    let mut body = alloc::vec![0u8; n as usize];
                    match files::read(&mut self.window, path.as_bytes(), &mut body) { Ok(got) => { body.truncate(got); done(200, body) } Err(_) => done(0, Vec::new()) }
                }
                Err(files::NOT_FOUND) => done(404, Vec::new()),
                _ => done(0, Vec::new()),
            });
        }
        let Some(abs) = sex_html::url::resolve(&base, url) else { return Ok(done(0, Vec::new())) };
        if !abs.starts_with("http") { return Ok(done(0, Vec::new())); }
        let origin = sex_fetch::Url::parse(&base).map(|u| u.origin()).unwrap_or_else(|_| String::from("null"));
        let mut p = Pending::new(ask.id, &ask.method, &abs, &ask.header_pairs(), &ask.body, FETCH_MAX, Some(&origin));
        p.asked = ask.url.clone();
        Err(p)
    }

    /// Gives the page a key first. True if a handler kept it.
    /// A handler may have set a timer without changing the page: ask for a tick now,
    /// which reports when the next one is due.
    fn timers_changed(&mut self) {
        if matches!(self.doc, Doc::Scripted(_)) { self.timer = Some(sex_pdx::get_ticks()); }
    }

    #[inline(never)]
    fn page_key(&mut self, key: sex_pdx::app_key::Key) -> bool {
        let name: String = match (nav(key), key.code) {
            (Some(Nav::Up), _) => "ArrowUp".into(),
            (Some(Nav::Down), _) => "ArrowDown".into(),
            (Some(Nav::Left), _) => "ArrowLeft".into(),
            (Some(Nav::Right), _) => "ArrowRight".into(),
            (Some(Nav::Home), _) => "Home".into(),
            (Some(Nav::End), _) => "End".into(),
            (Some(Nav::Delete), _) => "Delete".into(),
            (_, 0x1C) => "Enter".into(),
            (_, 0x0E) => "Backspace".into(),
            (_, 0x01) => "Escape".into(),
            (_, 0x39) => " ".into(),
            (_, 0x49) => "PageUp".into(),
            (_, 0x51) => "PageDown".into(),
            _ => match sex_app::typed(key) { Some(c) => (c as char).to_string(), None => return false },
        };
        let control = if key.control { "1" } else { "0" };
        self.sync_scroll();
        let Some(script) = self.doc.script() else { return false };
        let kept = script.dispatch(&alloc::format!("key\tkeydown\t{}\t\t{}", name, control)).map_or(false, |r| r == 1);
        let _ = script.dispatch(&alloc::format!("key\tkeyup\t{}\t\t{}", name, control));
        self.after_script(true);
        self.timers_changed();
        kept
    }

    fn scroll_to(&mut self, frag: Option<&str>) {
        if let Some(f) = frag {
            if let Some((_, y)) = self.page.anchors.iter().find(|(id, _)| id == f) { self.scroll = (*y).min(self.max_scroll()).max(0); }
        }
        self.draw();
    }

    fn max_scroll(&self) -> i32 { (self.page.height - self.view.1).max(0) }

    fn scroll_by(&mut self, dy: i32) {
        let next = (self.scroll + dy).clamp(0, self.max_scroll());
        if next != self.scroll { self.scroll = next; self.draw(); }
    }

    /// Moves link focus forwards or back, scrolling it into view.
    fn tab(&mut self, back: bool) {
        let n = self.page.links.len();
        if n == 0 { return; }
        // Start from the first link in view when nothing is focused.
        let next = match self.focus {
            None => {
                let first = self.page.links.iter().position(|l| l.y >= self.scroll).unwrap_or(0);
                if back { first.checked_sub(1).unwrap_or(n - 1) } else { first }
            }
            Some(f) => if back { (f + n - 1) % n } else { (f + 1) % n },
        };
        self.focus = Some(next);
        let l = &self.page.links[next];
        let view = H - BAR;
        if l.y < self.scroll || l.y + l.h > self.scroll + view { self.scroll = (l.y - view / 3).clamp(0, self.max_scroll()); }
        self.draw();
    }

    fn follow(&mut self) {
        let Some(l) = self.focus.and_then(|f| self.page.links.get(f)) else { return };
        let (href, node) = (l.href.clone(), l.node);
        self.follow_link(href, node, true);
    }

    /// Mouse input over the window: the address bar takes a click; the page gets DOM
    /// mouse events, and a click it doesn't cancel follows the link under it.
    #[inline(never)]
    fn pointer(&mut self, p: sex_pdx::app_pointer::Pointer) {
        use sex_pdx::app_pointer::Kind;
        let (x, y) = (p.x as i32, p.y as i32 - BAR);
        if p.y >= 0 && (p.y as i32) < BAR && matches!(p.kind, Kind::Down { button: 0 }) {
            self.editing = Some(String::new());
            self.draw();
            return;
        }
        let (px, py) = (x, y + self.scroll);
        // Over a frame: the frame's page has the pointer, in its own coordinates.
        if y >= 0 && self.in_frame.is_none() {
            if let Some(i) = self.frames.iter().position(|f| px >= f.rect[0] && px < f.rect[0] + f.rect[2] && py >= f.rect[1] && py < f.rect[1] + f.rect[3]) {
                let [fx, fy, _, _] = self.frames[i].rect;
                let q = sex_pdx::app_pointer::Pointer { x: (px - fx) as i16, y: (py - fy + BAR) as i16, buttons: p.buttons, kind: p.kind };
                self.enter(i);
                self.pointer(q);
                self.leave(i);
                return;
            }
        }
        // Links are named by node: a script's relayout rebuilds the list.
        let link = if y >= 0 { self.page.links.iter().find(|l| px >= l.x && px < l.x + l.w && py >= l.y && py < l.y + l.h).map(|l| l.node) } else { None };
        let pressed = match p.kind {
            Kind::Down { button: 0 } => { self.press_link = link; None }
            Kind::Up { button: 0 } => self.press_link.take(),
            Kind::Leave => { self.press_link = None; None }
            _ => None,
        };
        // :hover follows the pointer.
        let hovered = if self.hover_css && y >= 0 && !matches!(p.kind, Kind::Leave) { self.doc.get().node_at(px, py) } else { None };
        let hover_changed = self.hover_css && self.doc.get().dom.hover != hovered;
        self.doc.get().dom.hover = hovered;
        let mut kept = false;
        if let Doc::Scripted(_) = self.doc {
            self.sync_scroll();
            let node = if y >= 0 { self.doc.get().node_at(px, py).map_or(String::new(), |n| alloc::format!("{n}")) } else { String::new() };
            let (kind, button, steps) = match p.kind {
                Kind::Move => ("move", 0, 0),
                Kind::Down { button } => ("down", button, 0),
                Kind::Up { button } => ("up", button, 0),
                Kind::Wheel { steps } => ("wheel", 0, steps),
                Kind::Leave => ("leave", 0, 0),
            };
            let script = self.doc.script().unwrap();
            kept = script.dispatch(&alloc::format!("mouse\t{kind}\t{node}\t{x}\t{y}\t{button}\t{}\t{steps}", p.buttons)).map_or(false, |r| r == 1);
            if self.doc.get().navigate.is_some() { self.after_script(true); return; }
            self.after_script(true);
            self.timers_changed();
        }
        if hover_changed { self.relayout(); self.draw(); }
        match p.kind {
            Kind::Wheel { steps } if !kept => self.scroll_by(-steps as i32 * 40),
            Kind::Up { button: 0 } if !kept && y >= 0 => {
                let i = match (link, pressed) { (Some(n), Some(m)) if n == m => self.page.links.iter().position(|l| l.node == n), _ => None };
                if let Some(i) = i {
                    self.focus = Some(i);
                    let l = &self.page.links[i];
                    let (href, node) = (l.href.clone(), l.node);
                    // The page already had its click.
                    self.follow_link(href, node, false);
                }
            }
            _ => {}
        }
    }

    /// Tells the page script the current scroll offset when it changed.
    fn sync_scroll(&mut self) {
        if self.scroll == self.script_scroll { return; }
        let y = self.scroll;
        if let Some(script) = self.doc.script() { let _ = script.dispatch(&alloc::format!("scroll\t{y}")); self.script_scroll = y; }
    }

    fn follow_link(&mut self, href: String, node: usize, click: bool) {
        if let Some(script) = self.doc.script() {
            let mut kept = click && script.dispatch(&alloc::format!("click\t{}", node)).map_or(false, |r| r == 1);
            if let (false, Some(code)) = (kept, href.strip_prefix("javascript:")) {
                let _ = script.eval(&sex_html::parse::decode(code), "javascript:");
                kept = true;
            }
            let navigated = self.doc.get().navigate.is_some();
            self.after_script(true);
            self.timers_changed();
            if kept || navigated || href.is_empty() { return; }
        }
        if href.is_empty() || href.starts_with("javascript:") { return; }
        let target = if built_in(&href).is_some() { Some(href) } else if self.url.starts_with('@') || self.url.starts_with("about:") {
            if href.starts_with("http") || href.starts_with('#') { Some(href) } else { None }
        } else { sex_html::url::resolve(&self.url, &href) };
        match target {
            Some(t) if t.starts_with('#') => { let frag = t[1..].to_string(); self.scroll_to(Some(&frag)); }
            Some(t) => { self.referrer_next = Some(self.url.clone()); self.open(&t, true) }
            None => {}
        }
    }

    fn key(&mut self, key: sex_pdx::app_key::Key) {
        let view = H - BAR;
        if self.editing.is_none() && self.page_key(key) { return; }
        if let Some(text) = self.editing.as_mut() {
            match key.code {
                0x01 => { self.editing = None; self.draw(); }
                0x1C => {
                    let mut url = self.editing.take().unwrap();
                    if !url.contains("://") && !url.starts_with('@') && !url.starts_with("about:") && !url.is_empty() { url.insert_str(0, "https://"); }
                    self.open(&url, true);
                }
                0x0E => { text.pop(); self.draw(); }
                _ => if let Some(c) = sex_app::typed(key) { text.push(c as char); self.draw(); },
            }
            return;
        }
        match key.code {
            0x1C => return self.follow(),
            0x0E => { if let Some(prev) = self.history.pop() { self.open(&prev, false); } return; }
            0x39 => return self.scroll_by(if key.shift { -view + 40 } else { view - 40 }),
            0x49 => return self.scroll_by(-view + 40),
            0x51 => return self.scroll_by(view - 40),
            0x35 if !key.shift => { self.editing = Some(String::new()); self.draw(); return; }
            0x13 if key.control => { let u = self.url.clone(); self.open(&u, false); return; }
            0x24..=0x26 if !key.control && self.media_key(key.code) => return,
            _ => {}
        }
        match nav(key) {
            Some(Nav::Up) => self.scroll_by(-40),
            Some(Nav::Down) => self.scroll_by(40),
            Some(Nav::Right) => self.tab(false),
            Some(Nav::Left) => self.tab(true),
            Some(Nav::Home) => { self.scroll = 0; self.draw(); }
            Some(Nav::End) => { self.scroll = self.max_scroll(); self.draw(); }
            _ => {}
        }
    }
}

// The stack size is linked in (build.rs): layout recurses per nesting level, and
// scripts call back into it.
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Web");
    sex_web::set_avif_decoder(media::decode_avif);
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = String::from(core::str::from_utf8(files::args(&mut window, &mut args)).unwrap_or("").trim());
    let Some(fonts) = Fonts::new() else { window.show(b"Web: the fonts could not be loaded."); loop { if let Event::Close = window.next() { window.close() } } };
    let page = sex_html::render("", W, H, &fonts as &dyn Host);
    let doc = Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", (W, H - BAR), clock));
    let mut b = Browser { window, fonts, page, doc, sheets: Vec::new(), timer: None, timeline: Default::default(), animating: false, canvases: sex_web::canvas::Canvases::new(), media: Vec::new(), media_asked: false, media_loads: Vec::new(), url: String::new(), history: Vec::new(), scroll: 0, focus: None, editing: None, status: None, buf: Vec::new(), full_redraw: true, jar: Default::default(), press_link: None, script_scroll: 0, hover_css: false, post: None, import_map: Vec::new(), view: (W, H - BAR), parent: None, frames: Vec::new(), in_frame: None, frame_dirty: false, referrer_next: None, nav_from: None, frames_check: false };
    // Big WebGL draws spread over the cores, a band of rows each.
    let cores = sex_app::thread::cores();
    sex_gl::set_parallel(sex_app::thread::parallel, cores);
    sex_pdx::serial_record!("[web.cores] cores={}", cores);
    let first = given.split_ascii_whitespace().next().unwrap_or(first_page()).to_string();
    b.open(&first, false);
    sex_pdx::serial_record!("[web.ready] target={} task={}", b.window.target, b.window.task);
    let mut queued: Option<Event> = None;
    loop {
        b.settle();
        let event = if let Some(e) = queued.take() { e } else { match b.next_due() {
            Some(due) => {
                let now = sex_pdx::get_ticks();
                match if due > now { b.window.next_within(due - now) } else { b.window.poll() } {
                    Some(e) => e,
                    None => { b.tick_all(); continue; }
                }
            }
            None => b.window.next(),
        } };
        match event {
            Event::Close => { sex_pdx::serial_record!("[web.closed] target={} task={}", b.window.target, b.window.task); b.window.close() }
            Event::Key(key) => b.key(key),
            Event::Resize { .. } => {}
            Event::Pointer(mut p) => {
                // Only the newest of a run of moves matters: a slow page must not lag
                // behind the mouse.
                while matches!(p.kind, sex_pdx::app_pointer::Kind::Move) {
                    match b.window.poll() {
                        Some(Event::Pointer(q)) if matches!(q.kind, sex_pdx::app_pointer::Kind::Move) => p = q,
                        other => { queued = other; break; }
                    }
                }
                b.pointer(p);
            }
        }
    }
}

