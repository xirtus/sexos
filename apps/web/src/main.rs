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
const MAX_SCRIPTS: usize = 32;
/// Largest body a script's `fetch` may load (a game's data file, say).
const FETCH_MAX: usize = 32 << 20;

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

struct Browser {
    window: Window,
    fonts: Fonts,
    page: Page,
    doc: Doc,
    sheets: Vec<String>,
    /// When the page's next timer or animation frame is due, in ticks.
    timer: Option<u64>,
    /// The page's Canvas 2D surfaces.
    canvases: sex_web::canvas::Canvases,
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
    /// The scroll offset the page script last heard (`window.scrollY`).
    script_scroll: i32,
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

impl Browser {
    fn draw(&mut self) {
        let start = sex_pdx::get_ticks();
        let pixels = &mut self.window.pixels()[..(W * H) as usize];
        let bar = match (&self.editing, self.status) {
            (Some(t), _) => t.clone(),
            (None, Some(s)) => alloc::format!("{}  {}", s, self.url),
            (None, None) => if self.page.title.is_empty() { self.url.clone() } else { alloc::format!("{} — {}", self.page.title, self.url) },
        };
        sex_web::frame(&self.page, &mut self.fonts, pixels, W, H, self.scroll, self.focus, &bar, self.editing.is_some());
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
        let mut url = String::from(sex_html::url::fragment(url).0);
        let mut method = String::from(if method.is_empty() { "GET" } else { method });
        let mut body = body;
        for _ in 0..MAX_REDIRECTS {
            let target = sex_fetch::Url::parse(&url).map_err(|_| net::message(net::Error::Address))?;
            let cross = script.filter(|o| *o != target.origin());
            let cookie = if cross.is_some() { None } else { self.jar.borrow().header(&target, wall()) };
            let mut sent: Vec<(&str, &str)> = headers.iter().filter(|(k, _)| !sex_fetch::forbidden_header(k)).copied().collect();
            if let Some(o) = cross { sent.push(("Origin", o)); }
            let req = sex_fetch::Request { method: &method, url: &url, headers: &sent, body, trusted_origin: cross.is_some() };
            let begun = sex_pdx::get_ticks();
            let r = net::request(&mut self.window, &req, cookie.as_deref(), max).map_err(net::message)?;
            if cross.is_none() {
                let mut jar = self.jar.borrow_mut();
                for c in r.head.all("set-cookie") { jar.store(&target, c, wall()); }
            }
            sex_pdx::serial_record!("[web.http] method={} status={} bytes={} encoding={} chunked={} cookies={} ticks={} url={}", method, r.head.status, r.body.len(),
                r.head.get("content-encoding").unwrap_or("none"), r.head.get("transfer-encoding").is_some_and(|t| t.contains("chunked")) as u8, self.jar.borrow().len(), sex_pdx::get_ticks() - begun, url);
            if r.head.is_redirect() {
                if let Some(next) = r.head.get("location").and_then(|l| sex_fetch::join(&url, l)) {
                    // 303, and 301/302 after a POST, continue as a GET without the body.
                    if r.head.status == 303 || (matches!(r.head.status, 301 | 302) && method == "POST") { method = String::from("GET"); body = &[]; }
                    url = String::from(sex_html::url::fragment(&next).0);
                    continue;
                }
            }
            if let Some(o) = cross {
                let allowed = r.head.get("access-control-allow-origin").map(str::trim);
                if allowed != Some("*") && allowed != Some(o) {
                    sex_pdx::serial_record!("[web.cors] blocked origin={} url={}", o, url);
                    return Err(b"The site doesn't let this page read it.");
                }
            }
            let lines = r.head.lines(true);
            return Ok(Loaded { url, status: r.head.status, headers: lines, body: r.body });
        }
        Err(b"Too many redirects.")
    }

    fn open(&mut self, url: &str, remember: bool) {
        if remember && !self.url.is_empty() { self.history.push(self.url.clone()); }
        // Same page, other fragment: just scroll.
        let (here, _) = sex_html::url::fragment(&self.url);
        let (there, frag) = sex_html::url::fragment(url);
        if !here.is_empty() && here == there && frag.is_some() {
            self.url = String::from(url);
            self.scroll_to(frag);
            return;
        }
        self.status = Some("Loading…");
        self.url = String::from(url);
        self.draw();
        let begun = sex_pdx::get_ticks();
        let (html, base) = if let Some(page) = built_in(if url.is_empty() { "about:welcome" } else { url }) {
            (String::from(page), String::from(if url.is_empty() { "about:welcome" } else { url }))
        } else if let Some(file) = url.strip_prefix('@') {
            self.buf.resize(SOURCE_MAX, 0);
            match files::read(&mut self.window, file.as_bytes(), &mut self.buf) {
                Ok(n) => (text_of(&self.buf[..n]), String::from(url)),
                Err(e) => (message_page("Can't open that file", core::str::from_utf8(files::message(e)).unwrap_or(""), url), String::from(url)),
            }
        } else if url.starts_with("http://") || url.starts_with("https://") {
            match self.fetch(url) {
                Ok((final_url, body)) => { let t = text_of(&self.buf[body]); (t, final_url) }
                Err(why) => (message_page("Can't open this page", core::str::from_utf8(why).unwrap_or(""), url), String::from(url)),
            }
        } else {
            (message_page("Can't open that address", "Addresses start with https://, http://, or @ for a file.", url), String::from(url))
        };
        self.buf = Vec::new();
        if base.starts_with("http") { self.url = base.clone() + frag.map(|f| alloc::format!("#{}", f)).as_deref().unwrap_or(""); }
        let dom = sex_html::parse::parse(&html);
        drop(html);
        let fetched = sex_pdx::get_ticks();
        // Style sheets and pictures from the network, within limits.
        let mut sheets: Vec<String> = Vec::new();
        self.fonts.images.clear();
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
                    if let Some(img) = sex_web::decode_image(&self.buf[body], (W - 16) as usize, 2000) { self.fonts.images.push((src.clone(), alloc::rc::Rc::new(img))); }
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
        let mut doc = Document::new(dom, &base, (W, H - BAR), clock);
        doc.measure = sex_web::canvas::measure_css;
        doc.cookies = Some(self.jar.clone());
        doc.wall = wall;
        doc.pictures = self.fonts.images.clone();
        self.doc = Doc::Plain(doc);
        self.canvases = sex_web::canvas::Canvases::new();
        let script_begun = sex_pdx::get_ticks();
        if !scripts.is_empty() || handlers { self.run_scripts(scripts); }
        let laid = sex_pdx::get_ticks();
        self.relayout();
        sex_pdx::serial_record!("[web.loaded] bytes_ok=1 items={} links={} height={} sheets={} images={} fetch_ticks={} layout_ticks={} script_ticks={} scripted={}",
            self.page.items.len(), self.page.links.len(), self.page.height, self.sheets.len(), self.fonts.images.len(), script_begun - begun,
            sex_pdx::get_ticks() - laid, laid - script_begun, matches!(self.doc, Doc::Scripted(_)) as u8);
        let _ = fetched;
        self.status = None;
        self.scroll = 0;
        self.focus = None;
        let title: Vec<u8> = self.page.title.bytes().take(16).collect();
        self.window.title(if title.is_empty() { b"Web" } else { &title });
        self.scroll_to(frag);
    }

    /// The page's classic scripts in document order: inline text, or fetched from `src`.
    #[inline(never)]
    fn scripts(&mut self, dom: &sex_html::dom::Dom, base: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for n in dom.descendants(sex_html::dom::Dom::ROOT) {
            if out.len() >= MAX_SCRIPTS { break; }
            if dom.tag(n) != Some("script") { continue; }
            let ty = dom.attr(n, "type").unwrap_or("").trim().to_ascii_lowercase();
            if !(ty.is_empty() || ty.contains("javascript") || ty == "module" || ty.contains("ecmascript")) { continue; }
            match dom.attr(n, "src") {
                // A built-in page's scripts are built in too.
                Some(src) if base.starts_with("about:") => {
                    if let Some(code) = built_in(&alloc::format!("about:{}", src.trim_start_matches("./"))) { out.push((String::from(code), alloc::format!("about:{}", src))); }
                }
                Some(src) if base.starts_with('@') => {
                    let Some(path) = local_path(base, src) else { continue };
                    self.buf.resize(SOURCE_MAX, 0);
                    if let Ok(n) = files::read(&mut self.window, path.as_bytes(), &mut self.buf) { out.push((text_of(&self.buf[..n]), alloc::format!("@{}", path))); }
                    self.buf = Vec::new();
                }
                Some(src) => {
                    let Some(abs) = sex_html::url::resolve(base, src) else { continue };
                    if !abs.starts_with("http") { continue; }
                    self.status = Some("Loading scripts…");
                    if let Ok((_, body)) = self.fetch(&abs) { out.push((text_of(&self.buf[body]), abs)); }
                    self.buf = Vec::new();
                }
                None => out.push((dom.text_content(n), alloc::format!("{}#script{}", base, out.len() + 1))),
            }
        }
        out
    }

    #[inline(never)]
    fn run_scripts(&mut self, scripts: Vec<(String, String)>) {
        let Doc::Plain(doc) = core::mem::replace(&mut self.doc, Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", (W, H - BAR), clock))) else { return };
        let started = sex_pdx::get_ticks();
        let mut script = match Script::new(doc, sex_js::Limits::default()) {
            Ok(s) => s,
            Err(e) => {
                sex_pdx::serial_record!("[web.script] engine={:?}", e);
                return;
            }
        };
        sex_pdx::serial_record!("[web.script] engine_ticks={} memory={}", sex_pdx::get_ticks() - started, script.memory_bytes());
        for (source, name) in scripts.iter() {
            let r = script.eval(source, name);
            if let Err(sex_js::Error::Trapped(why)) = &r { sex_pdx::serial_record!("[web.script] trapped={}", why); break; }
        }
        let _ = script.dispatch("ready");
        self.script_scroll = 0;
        self.doc = Doc::Scripted(alloc::boxed::Box::new(script));
        self.after_script(false);
    }

    /// After any script ran: print its console, take a navigation, and lay out again
    /// when it changed the page. Returns true if it drew.
    #[inline(never)]
    fn after_script(&mut self, draw: bool) -> bool {
        let doc = self.doc.get();
        for (level, text) in doc.log.drain(..) {
            for line in text.lines().take(8) { sex_pdx::serial_record!("[web.console] level={} {}", level, line); }
        }
        if let Some(url) = doc.navigate.take() {
            self.open(&url, true);
            return true;
        }
        // Fetches are served on the next tick, once this script has returned.
        if !doc.fetches.is_empty() && matches!(self.doc, Doc::Scripted(_)) { self.timer = Some(sex_pdx::get_ticks()); }
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
        if !dirty && !self.full_redraw {
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
        let styles = sex_html::styles(&doc.dom, &sheet_refs, W, H - BAR);
        self.page = sex_html::layout::layout(&doc.dom, &styles, W, &self.fonts);
        doc.set_boxes(&self.page.boxes);
        self.scroll = self.scroll.min(self.max_scroll());
        if self.focus.map_or(false, |f| f >= self.page.links.len()) { self.focus = None; }
        // Ask for the next tick soon: the change may have started timers.
        if matches!(self.doc, Doc::Scripted(_)) && self.timer.is_none() { self.timer = Some(sex_pdx::get_ticks()); }
    }

    /// Runs due timers and animation frames.
    #[inline(never)]
    fn tick(&mut self) {
        self.serve_fetches();
        self.sync_scroll();
        let Some(script) = self.doc.script() else { self.timer = None; return };
        let next = script.dispatch("tick");
        self.timer = match next { Ok(ms) if ms >= 0 => Some(sex_pdx::get_ticks() + ms as u64), _ => None };
        if let Err(sex_js::Error::Trapped(why)) = next { sex_pdx::serial_record!("[web.script] trapped={}", why); self.doc = Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", (W, H - BAR), clock)); }
        self.after_script(true);
    }

    /// Loads what the page's scripts asked `fetch` and XHR for, and tells each script.
    #[inline(never)]
    fn serve_fetches(&mut self) {
        let asked = match &mut self.doc { Doc::Scripted(s) => core::mem::take(&mut s.page_mut().fetches), _ => return };
        for ask in asked {
            let begun = sex_pdx::get_ticks();
            let r = self.load(&ask);
            let n = r.body.len();
            sex_pdx::serial_record!("[web.fetch] id={} status={} bytes={} ticks={} method={} url={}", ask.id, r.status, n, sex_pdx::get_ticks() - begun, if ask.method.is_empty() { "GET" } else { &ask.method }, ask.url);
            let Doc::Scripted(s) = &mut self.doc else { return };
            s.page_mut().keep_head(ask.id, &r.url, &r.headers);
            s.page_mut().keep_body(ask.id, r.status, r.body);
            if let Err(sex_js::Error::Trapped(why)) = s.dispatch(&alloc::format!("fetched\t{}\t{}\t{}", ask.id, r.status, n)) {
                sex_pdx::serial_record!("[web.script] trapped={}", why);
                self.doc = Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", (W, H - BAR), clock));
                return;
            }
        }
    }

    /// A script's request: the reply (status 0 when it could not be loaded). Pages fetch
    /// from where they came from: built in, a file (`@`), or the network.
    fn load(&mut self, ask: &sex_js::FetchAsk) -> Loaded {
        let url = ask.url.as_str();
        let base = String::from(sex_html::url::fragment(&self.url).0);
        let done = |status: u16, body: Vec<u8>| Loaded { url: String::from(url), status, headers: String::new(), body };
        if url.starts_with("about:") || (base.starts_with("about:") && !url.contains(':')) {
            let name = if url.starts_with("about:") { String::from(url) } else { alloc::format!("about:{}", url.trim_start_matches("./")) };
            return match built_in(&name) { Some(text) => done(200, Vec::from(text.as_bytes())), None => done(404, Vec::new()) };
        }
        if let Some(path) = local_path(&base, url) {
            return match files::size(&mut self.window, path.as_bytes()) {
                Ok(n) if n as usize <= FETCH_MAX => {
                    let mut body = alloc::vec![0u8; n as usize];
                    match files::read(&mut self.window, path.as_bytes(), &mut body) { Ok(got) => { body.truncate(got); done(200, body) } Err(_) => done(0, Vec::new()) }
                }
                Err(files::NOT_FOUND) => done(404, Vec::new()),
                _ => done(0, Vec::new()),
            };
        }
        let Some(abs) = sex_html::url::resolve(&base, url) else { return done(0, Vec::new()) };
        if !abs.starts_with("http") { return done(0, Vec::new()); }
        let headers = ask.header_pairs();
        let origin = sex_fetch::Url::parse(&base).map(|u| u.origin()).unwrap_or_else(|_| String::from("null"));
        match self.http(&ask.method, &abs, &headers, &ask.body, FETCH_MAX, Some(&origin)) { Ok(r) => r, Err(_) => done(0, Vec::new()) }
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

    fn max_scroll(&self) -> i32 { (self.page.height - (H - BAR)).max(0) }

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
        // Links are named by node: a script's relayout rebuilds the list.
        let link = if y >= 0 { self.page.links.iter().find(|l| px >= l.x && px < l.x + l.w && py >= l.y && py < l.y + l.h).map(|l| l.node) } else { None };
        let pressed = match p.kind {
            Kind::Down { button: 0 } => { self.press_link = link; None }
            Kind::Up { button: 0 } => self.press_link.take(),
            Kind::Leave => { self.press_link = None; None }
            _ => None,
        };
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
            Some(t) => self.open(&t, true),
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
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = String::from(core::str::from_utf8(files::args(&mut window, &mut args)).unwrap_or("").trim());
    let Some(fonts) = Fonts::new() else { window.show(b"Web: the fonts could not be loaded."); loop { if let Event::Close = window.next() { window.close() } } };
    let page = sex_html::render("", W, H, &fonts as &dyn Host);
    let doc = Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", (W, H - BAR), clock));
    let mut b = Browser { window, fonts, page, doc, sheets: Vec::new(), timer: None, canvases: sex_web::canvas::Canvases::new(), url: String::new(), history: Vec::new(), scroll: 0, focus: None, editing: None, status: None, buf: Vec::new(), full_redraw: true, jar: Default::default(), press_link: None, script_scroll: 0 };
    let first = given.split_ascii_whitespace().next().unwrap_or(first_page()).to_string();
    b.open(&first, false);
    sex_pdx::serial_record!("[web.ready] target={} task={}", b.window.target, b.window.task);
    let mut queued: Option<Event> = None;
    loop {
        let event = if let Some(e) = queued.take() { e } else { match b.timer {
            Some(due) => {
                let now = sex_pdx::get_ticks();
                match if due > now { b.window.next_within(due - now) } else { b.window.poll() } {
                    Some(e) => e,
                    None => { b.tick(); continue; }
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

