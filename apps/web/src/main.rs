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
const SOURCE_MAX: usize = 1536 * 1024;
const MAX_SHEETS: usize = 8;
const MAX_IMAGES: usize = 24;
const MAX_REDIRECTS: usize = 6;
const MAX_SCRIPTS: usize = 32;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[web.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn clock() -> u64 { sex_pdx::get_ticks().saturating_mul(1_000_000) }

/// The page's tree, with the script engine that owns it when the page has scripts.
enum Doc {
    Plain(Document),
    Scripted(Script<Document>),
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
    url: String,
    history: Vec<String>,
    scroll: i32,
    focus: Option<usize>,
    editing: Option<String>,
    status: Option<&'static str>,
    buf: Vec<u8>,
}

/// Text from bytes: UTF-8 when valid, otherwise read as Latin-1.
fn text_of(bytes: &[u8]) -> String {
    match core::str::from_utf8(bytes) {
        Ok(s) => String::from(s),
        Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 > bytes.len() => String::from(core::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or("")),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
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
        sex_pdx::serial_record!("[web.shown] w={} h={} scroll={} focus={} items={} ticks={}", W, H, self.scroll, self.focus.map_or(-1, |f| f as i64), self.page.items.len(), sex_pdx::get_ticks() - start);
    }

    /// Fetches `url` into `self.buf`, following redirects. Returns the final address
    /// and the body's range, or a message.
    fn fetch(&mut self, url: &str) -> Result<(String, core::ops::Range<usize>), &'static [u8]> {
        let mut url = String::from(sex_html::url::fragment(url).0);
        for _ in 0..MAX_REDIRECTS {
            self.buf.resize(SOURCE_MAX, 0);
            let r = net::fetch(&mut self.window, url.as_bytes(), &mut self.buf).map_err(net::message)?;
            if (300..400).contains(&r.status) {
                if let Some(next) = r.header(&self.buf, b"location").and_then(|l| core::str::from_utf8(l).ok()).and_then(|l| sex_html::url::resolve(&url, l)) {
                    url = String::from(sex_html::url::fragment(&next).0);
                    continue;
                }
            }
            if r.status >= 400 { return Err(b"The site answered with an error."); }
            return Ok((url, r.body..r.end));
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
        let (html, base) = if url == "about:welcome" || url.is_empty() {
            (String::from(WELCOME), String::from("about:welcome"))
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
                    if let Some(img) = sex_web::decode_image(&self.buf[body], (W - 16) as usize, 2000) { self.fonts.images.push((src.clone(), img)); }
                }
            }
            self.buf = Vec::new();
        }
        let scripts = self.scripts(&dom, &base);
        self.sheets = sheets;
        self.timer = None;
        self.doc = Doc::Plain(Document::new(dom, &base, (W, H - BAR), clock));
        let script_begun = sex_pdx::get_ticks();
        if !scripts.is_empty() { self.run_scripts(scripts); }
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
    fn scripts(&mut self, dom: &sex_html::dom::Dom, base: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for n in dom.descendants(sex_html::dom::Dom::ROOT) {
            if out.len() >= MAX_SCRIPTS { break; }
            if dom.tag(n) != Some("script") { continue; }
            let ty = dom.attr(n, "type").unwrap_or("").trim().to_ascii_lowercase();
            if !(ty.is_empty() || ty.contains("javascript") || ty == "module" || ty.contains("ecmascript")) { continue; }
            match dom.attr(n, "src") {
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
        self.doc = Doc::Scripted(script);
        self.after_script(false);
    }

    /// After any script ran: print its console, take a navigation, and lay out again
    /// when it changed the page. Returns true if it drew.
    fn after_script(&mut self, draw: bool) -> bool {
        let doc = self.doc.get();
        for (level, text) in doc.log.drain(..) {
            for line in text.lines().take(8) { sex_pdx::serial_record!("[web.console] level={} {}", level, line); }
        }
        if let Some(url) = doc.navigate.take() {
            self.open(&url, true);
            return true;
        }
        let dirty = core::mem::take(&mut doc.dirty);
        if dirty {
            self.relayout();
            if draw { self.draw(); return true; }
        }
        false
    }

    /// Styles and lays the page out again from the tree.
    fn relayout(&mut self) {
        let sheet_refs: Vec<&str> = self.sheets.iter().map(|s| s.as_str()).collect();
        let doc = match &mut self.doc { Doc::Plain(d) => d, Doc::Scripted(s) => s.page_mut() };
        let styles = sex_html::styles(&doc.dom, &sheet_refs, W, H - BAR);
        self.page = sex_html::layout::layout(&doc.dom, &styles, W, &self.fonts);
        self.scroll = self.scroll.min(self.max_scroll());
        if self.focus.map_or(false, |f| f >= self.page.links.len()) { self.focus = None; }
        // Ask for the next tick soon: the change may have started timers.
        if matches!(self.doc, Doc::Scripted(_)) && self.timer.is_none() { self.timer = Some(sex_pdx::get_ticks()); }
    }

    /// Runs due timers and animation frames.
    fn tick(&mut self) {
        let Some(script) = self.doc.script() else { self.timer = None; return };
        let next = script.dispatch("tick");
        self.timer = match next { Ok(ms) if ms >= 0 => Some(sex_pdx::get_ticks() + ms as u64), _ => None };
        if let Err(sex_js::Error::Trapped(why)) = next { sex_pdx::serial_record!("[web.script] trapped={}", why); self.doc = Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", (W, H - BAR), clock)); }
        self.after_script(true);
    }

    /// Gives the page a key first. True if a handler kept it.
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
        let Some(script) = self.doc.script() else { return false };
        let kept = script.dispatch(&alloc::format!("key\tkeydown\t{}\t\t{}", name, control)).map_or(false, |r| r == 1);
        let _ = script.dispatch(&alloc::format!("key\tkeyup\t{}\t\t{}", name, control));
        self.after_script(true);
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
        if let Some(script) = self.doc.script() {
            let mut kept = script.dispatch(&alloc::format!("click\t{}", node)).map_or(false, |r| r == 1);
            if let (false, Some(code)) = (kept, href.strip_prefix("javascript:")) {
                let _ = script.eval(&sex_html::parse::decode(code), "javascript:");
                kept = true;
            }
            let navigated = self.doc.get().navigate.is_some();
            self.after_script(true);
            if kept || navigated || href.is_empty() { return; }
        }
        if href.is_empty() || href.starts_with("javascript:") { return; }
        let target = if href == "about:welcome" { Some(href) } else if self.url.starts_with('@') || self.url == "about:welcome" {
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

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Web");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = String::from(core::str::from_utf8(files::args(&mut window, &mut args)).unwrap_or("").trim());
    let Some(fonts) = Fonts::new() else { window.show(b"Web: the fonts could not be loaded."); loop { if let Event::Close = window.next() { window.close() } } };
    let page = sex_html::render("", W, H, &fonts as &dyn Host);
    let doc = Doc::Plain(Document::new(sex_html::dom::Dom::new(), "", (W, H - BAR), clock));
    let mut b = Browser { window, fonts, page, doc, sheets: Vec::new(), timer: None, url: String::new(), history: Vec::new(), scroll: 0, focus: None, editing: None, status: None, buf: Vec::new() };
    let first = given.split_ascii_whitespace().next().unwrap_or("about:welcome").to_string();
    b.open(&first, false);
    sex_pdx::serial_record!("[web.ready] target={} task={}", b.window.target, b.window.task);
    loop {
        let event = match b.timer {
            Some(due) => {
                let now = sex_pdx::get_ticks();
                match if due > now { b.window.next_within(due - now) } else { b.window.poll() } {
                    Some(e) => e,
                    None => { b.tick(); continue; }
                }
            }
            None => b.window.next(),
        };
        match event {
            Event::Close => { sex_pdx::serial_record!("[web.closed] target={} task={}", b.window.target, b.window.task); b.window.close() }
            Event::Key(key) => b.key(key),
        }
    }
}
