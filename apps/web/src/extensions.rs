// SPDX-License-Identifier: GPL-3.0-only
//! Extensions (slice N7a): Chrome and Firefox extensions installed from a `.crx` or
//! `.zip` link, kept unpacked in Files under `Web/extensions/<id>/`.
//!
//! What runs: content scripts (style sheets and scripts injected into matching pages,
//! at `document_start` before the page's scripts, otherwise after them, from the
//! manifest or `chrome.scripting.registerContentScripts`), declarativeNetRequest
//! rulesets and dynamic rules, which block through the same engine as Shields, and
//! (slice N7b) each extension's background page or service worker in a script realm
//! of its own, started with the browser. The `chrome.*` API is sex-js's
//! `extensions.js`; this routes its messages between realms (`ext_pump`), serves
//! `kaleidoscope-extension://ID/PATH` from the extension's files, and keeps its
//! storage with the profile's. Content scripts run in the page's own script realm,
//! not an isolated one, so a page can see what they define (but can't get at the
//! `chrome` API, which takes a key only the browser's code has).
//!
//! `about:extensions` lists them, turns each off or on, removes them, and installs
//! one from an address (`about:extensions?install=URL`).

use super::{clock, sex_html, text_of, wall, Browser, Doc, PageScript};
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use serde_json::{json, Value};
use sex_app::{files, Window};
use sex_extensions::{clean_path, dnr, package, Manifest, Pattern, RunAt};
use sex_js::{Document, Script};
use sex_shields::{Engine, Kind, Request, Verdict};

/// Where an extension's own files are served from.
pub const SCHEME: &str = "kaleidoscope-extension://";
/// What extensions remember besides `chrome.storage`: rulesets turned on or off,
/// dynamic rules, registered content scripts. `<id>.json` each.
const STATE: &str = "Web/extensions-state";
/// Rounds of message passing after one piece of work, so two realms answering each
/// other forever can't hold the browser.
const ROUNDS: usize = 64;

const DIR: &str = "Web/extensions";
/// Extensions turned off, one id a line.
const OFF: &[u8] = b"Web/extensions-off.tsv";
/// Largest package downloaded, and largest file of one read back.
const PACKAGE_MAX: usize = 64 << 20;
const FILE_MAX: usize = 16 << 20;

pub struct Ext {
    pub id: String,
    pub manifest: Manifest,
    pub enabled: bool,
    rules: Option<Box<Engine>>,
    pub rule_count: usize,
    pub skipped: usize,
    /// Content script files read so far, by path.
    cache: Vec<(String, String)>,
    /// `manifest.json` as written, for `chrome.runtime.getManifest`.
    raw: String,
    /// The default locale's `messages.json`, for `chrome.i18n`.
    messages: String,
    /// Ruleset ids turned on, when `updateEnabledRulesets` changed the manifest's choice.
    rulesets_on: Option<Vec<String>>,
    /// declarativeNetRequest rules added at run time: dynamic (kept) and session.
    dynamic: Vec<Value>,
    session: Vec<Value>,
    /// Content scripts from `chrome.scripting.registerContentScripts`.
    registered: Vec<Value>,
    /// Installed since the browser started: the background hears `onInstalled`.
    fresh: bool,
}

/// An extension's background page or service worker, running.
pub struct Background {
    pub id: String,
    script: Box<Script<Document>>,
    timer: Option<u64>,
}

#[derive(Default)]
pub struct Extensions {
    pub list: Vec<Ext>,
    pub backgrounds: Vec<Background>,
    /// What lets the browser's own code (and nothing a page wrote) make a `chrome` object.
    pub key: String,
}

/// Where a message to or from an extension realm goes: a page's document (by its
/// [`Document::owner`], so an answer never lands in another tab's page) or a background.
#[derive(Clone, PartialEq, Eq)]
enum Realm { Page(u64), Background(String) }

impl Realm {
    fn tag(&self) -> String { match self { Realm::Page(d) => alloc::format!("page:{}", d), Realm::Background(id) => alloc::format!("bg:{}", id) } }
    fn from_tag(t: &str) -> Option<Realm> {
        if let Some(id) = t.strip_prefix("bg:") { return Some(Realm::Background(String::from(id))); }
        t.strip_prefix("page:")?.parse().ok().map(Realm::Page)
    }
}

fn read_bytes(window: &mut Window, path: &str) -> Option<Vec<u8>> {
    let size = files::size(window, path.as_bytes()).ok()?;
    if size as usize > FILE_MAX { return None; }
    let mut buf = alloc::vec![0u8; size as usize];
    let n = files::read(window, path.as_bytes(), &mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

/// A secret for this run: what the clock and the processor's counter say now.
fn new_key() -> String {
    // SAFETY: rdtsc only reads the time stamp counter.
    let t = unsafe { core::arch::x86_64::_rdtsc() };
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ sex_pdx::get_ticks();
    for b in t.to_le_bytes() { h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3); }
    let mut h2 = h.rotate_left(29) ^ t.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h2 ^= h2 >> 31;
    alloc::format!("{:016x}{:016x}", h, h2)
}

/// (extension id, path) of a `kaleidoscope-extension://` address.
pub fn split_url(url: &str) -> Option<(&str, String)> {
    let rest = url.strip_prefix(SCHEME)?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let (id, path) = rest.split_once('/').unwrap_or((rest, ""));
    Some((id, clean_path(path)?))
}

fn read(window: &mut Window, path: &str) -> Option<String> {
    let size = files::size(window, path.as_bytes()).ok()?;
    if size as usize > FILE_MAX { return None; }
    let mut buf = alloc::vec![0u8; size as usize];
    let n = files::read(window, path.as_bytes(), &mut buf).ok()?;
    buf.truncate(n);
    Some(text_of(&buf))
}

/// A folder name for an extension: its name's letters and digits.
fn make_id(name: &str) -> String {
    let id: String = name.chars().filter_map(|c| if c.is_ascii_alphanumeric() { Some(c.to_ascii_lowercase()) } else if c == ' ' || c == '-' || c == '_' { Some('-') } else { None }).take(40).collect();
    let id = String::from(id.trim_matches('-'));
    if id.is_empty() { String::from("extension") } else { id }
}

/// `%xx` and `+` in a query value.
pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(core::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) { Ok(v) => { out.push(v); i += 2; } Err(_) => out.push(b'%') }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('\'', "&#39;").replace('"', "&quot;") }

impl Ext {
    fn load(window: &mut Window, id: &str, off: &[String]) -> Option<Ext> {
        let base = alloc::format!("{}/{}", DIR, id);
        let raw = read(window, &alloc::format!("{}/manifest.json", base))?;
        let mut manifest = Manifest::parse(&raw).ok()?;
        let messages = read(window, &alloc::format!("{}/_locales/{}/messages.json", base, manifest.default_locale.as_deref().unwrap_or("en"))).unwrap_or_else(|| String::from("{}"));
        if manifest.locale_file().is_some() { manifest.localize(&messages); }
        let mut ext = Ext { id: String::from(id), manifest, enabled: !off.iter().any(|o| o == id), rules: None, rule_count: 0, skipped: 0, cache: Vec::new(),
            raw, messages, rulesets_on: None, dynamic: Vec::new(), session: Vec::new(), registered: Vec::new(), fresh: false };
        if let Some(Value::Object(st)) = read(window, &alloc::format!("{}/{}.json", STATE, id)).and_then(|t| serde_json::from_str(&t).ok()) {
            ext.rulesets_on = st.get("rulesets").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect());
            if let Some(Value::Array(a)) = st.get("dynamic") { ext.dynamic = a.clone(); }
            if let Some(Value::Array(a)) = st.get("scripts") { ext.registered = a.clone(); }
        }
        ext.rebuild(window);
        sex_pdx::serial_record!("[web.extension.loaded] id={} mv={} enabled={} rules={} skipped={} content_scripts={} background={}",
            ext.id, ext.manifest.version, ext.enabled as u8, ext.rule_count, ext.skipped, ext.manifest.content_scripts.len(), ext.manifest.background.len());
        Some(ext)
    }

    /// The rulesets turned on now.
    fn rulesets_enabled(&self) -> Vec<String> {
        match &self.rulesets_on {
            Some(on) => self.manifest.rulesets.iter().filter(|r| on.contains(&r.id)).map(|r| r.id.clone()).collect(),
            None => self.manifest.rulesets.iter().filter(|r| r.enabled).map(|r| r.id.clone()).collect(),
        }
    }

    /// Builds the blocking engine again from the rulesets turned on and the dynamic and
    /// session rules.
    fn rebuild(&mut self, window: &mut Window) {
        let (mut engine, mut count, mut skipped) = (Box::new(Engine::new()), 0, 0);
        let mut add = |json: &str, count: &mut usize, skipped: &mut usize| match dnr::convert(json) {
            Ok(c) => { *skipped += c.skipped; for f in &c.filters { if engine.add_filter(f) { *count += 1 } else { *skipped += 1 } } }
            Err(_) => *skipped += 1,
        };
        for id in self.rulesets_enabled() {
            let Some(r) = self.manifest.rulesets.iter().find(|r| r.id == id) else { continue };
            let Some(json) = read(window, &alloc::format!("{}/{}/{}", DIR, self.id, r.path)) else { continue };
            add(&json, &mut count, &mut skipped);
        }
        for rules in [&self.dynamic, &self.session] {
            if !rules.is_empty() { add(&Value::Array(rules.clone()).to_string(), &mut count, &mut skipped); }
        }
        self.rule_count = count;
        self.skipped = skipped;
        self.rules = (count > 0).then_some(engine);
    }

    /// Saves what the extension changed at run time.
    fn save_state(&self, window: &mut Window) {
        let st = json!({ "rulesets": self.rulesets_on, "dynamic": self.dynamic, "scripts": self.registered });
        let _ = files::mkdir(window, b"Web");
        let _ = files::mkdir(window, STATE.as_bytes());
        let _ = files::write(window, alloc::format!("{}/{}.json", STATE, self.id).as_bytes(), st.to_string().as_bytes());
    }

    /// The code that gives a script of this extension its `chrome` and `browser`.
    fn api(&self, key: &str, realm: &str) -> String {
        let id = self.id.replace(['"', '\\'], "");
        let manifest = if serde_json::from_str::<Value>(&self.raw).is_ok() { self.raw.as_str() } else { "{}" };
        let messages = if serde_json::from_str::<Value>(&self.messages).is_ok() { self.messages.as_str() } else { "{}" };
        alloc::format!("globalThis.__kaleidoscopeExt&&__kaleidoscopeExt(\"{id}\",\"{key}\",{manifest},{messages},\"{realm}\")")
    }

    fn file(&mut self, window: &mut Window, path: &str) -> Option<String> {
        if let Some((_, t)) = self.cache.iter().find(|(p, _)| p == path) { return Some(t.clone()); }
        let t = read(window, &alloc::format!("{}/{}/{}", DIR, self.id, path))?;
        self.cache.push((String::from(path), t.clone()));
        Some(t)
    }

    /// A content script wrapped so it sees `chrome` and `browser`.
    fn wrap(&self, key: &str, source: &str) -> String {
        alloc::format!("(function(){{const chrome={};const browser=chrome;\n{}\n}})();", self.api(key, "content"), source)
    }

    /// Registered content scripts for a page at `url`: (css files, js files, run_at).
    fn registered_for(&self, url: &str, in_frame: bool) -> Vec<(Vec<String>, Vec<String>, RunAt)> {
        let strings = |v: &Value| -> Vec<String> { v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default() };
        let mut out = Vec::new();
        for r in &self.registered {
            let matches = strings(&r["matches"]);
            let exclude = strings(&r["excludeMatches"]);
            if in_frame && r["allFrames"].as_bool() != Some(true) { continue; }
            if !matches.iter().filter_map(|p| Pattern::parse(p)).any(|p| p.matches(url)) || exclude.iter().filter_map(|p| Pattern::parse(p)).any(|p| p.matches(url)) { continue; }
            let run_at = match r["runAt"].as_str() { Some("document_start") => RunAt::Start, Some("document_end") => RunAt::End, _ => RunAt::Idle };
            let clean = |v: Vec<String>| v.iter().filter_map(|p| clean_path(p)).collect::<Vec<_>>();
            out.push((clean(strings(&r["css"])), clean(strings(&r["js"])), run_at));
        }
        out
    }
}

impl Extensions {
    pub fn load(window: &mut Window) -> Extensions {
        let off: Vec<String> = read(window, core::str::from_utf8(OFF).unwrap_or("")).map(|t| t.lines().map(|l| String::from(l.trim())).filter(|l| !l.is_empty()).collect()).unwrap_or_default();
        let mut list = Vec::new();
        if let Ok(entries) = files::list(window, DIR.as_bytes()) {
            for e in entries.into_iter().filter(|e| e.dir) {
                let Ok(id) = core::str::from_utf8(&e.name) else { continue };
                if let Some(ext) = Ext::load(window, id, &off) { list.push(ext); }
            }
        }
        Extensions { list, backgrounds: Vec::new(), key: new_key() }
    }

    /// Whether any extension is on (pages then get the key for `chrome`).
    pub fn any(&self) -> bool { self.list.iter().any(|e| e.enabled) }

    /// The extension whose rules block this request, if one does.
    pub fn blocks(&self, url: &str, page: &str, kind: Kind) -> Option<&str> {
        if !page.starts_with("http") { return None; }
        for e in self.list.iter().filter(|e| e.enabled) {
            let Some(rules) = &e.rules else { continue };
            if let Verdict::Block(id) = rules.check(&Request { url, page, kind }) {
                sex_pdx::serial_record!("[web.extension.blocked] id={} kind={} filter={} url={}", e.id, kind.name(), rules.filter_text(id), url);
                return Some(&e.id);
            }
        }
        None
    }

    /// The style sheets and scripts extensions put into a page at `url`: sheets, then
    /// scripts to run before the page's own, then after.
    pub fn content(&mut self, window: &mut Window, url: &str, in_frame: bool) -> (Vec<String>, Vec<PageScript>, Vec<PageScript>) {
        let (mut css, mut early, mut late) = (Vec::new(), Vec::new(), Vec::new());
        if !url.starts_with("http") { return (css, early, late); }
        let key = self.key.clone();
        for e in self.list.iter_mut().filter(|e| e.enabled) {
            let mut wanted: Vec<(Vec<String>, Vec<String>, RunAt)> = e.manifest.content_scripts.iter().filter(|c| c.applies(url, in_frame)).map(|c| (c.css.clone(), c.js.clone(), c.run_at)).collect();
            wanted.extend(e.registered_for(url, in_frame));
            for (sheets, scripts, run_at) in wanted {
                for p in sheets { if let Some(t) = e.file(window, &p) { css.push(t); } }
                for p in scripts {
                    let Some(t) = e.file(window, &p) else { continue };
                    let s = PageScript { source: e.wrap(&key, &t), name: alloc::format!("kaleidoscope-extension://{}/{}", e.id, p), module: false, node: None, external: false };
                    if run_at == RunAt::Start { early.push(s) } else { late.push(s) }
                }
            }
        }
        if !css.is_empty() || !early.is_empty() || !late.is_empty() {
            sex_pdx::serial_record!("[web.extension.content] css={} early={} late={} url={}", css.len(), early.len(), late.len(), url);
        }
        (css, early, late)
    }

    fn save_off(&self, window: &mut Window) {
        let mut t = String::new();
        for e in self.list.iter().filter(|e| !e.enabled) { t.push_str(&e.id); t.push('\n'); }
        let _ = files::mkdir(window, b"Web");
        let _ = files::write(window, OFF, t.as_bytes());
    }
}

impl Browser {
    /// Downloads a package from `url` and installs it. Returns the new extension's id.
    fn install_extension(&mut self, url: &str) -> Result<String, String> {
        self.status = Some("Installing extension…");
        self.draw();
        let bytes = self.fetch_up_to(url, PACKAGE_MAX).map(|(_, b)| self.buf[b].to_vec()).map_err(|w| String::from(core::str::from_utf8(w).unwrap_or("download failed")));
        self.buf = Vec::new();
        let unpacked = package::unpack(&bytes?).map_err(|e| alloc::format!("{:?}", e))?;
        let manifest_text = unpacked.iter().find(|(p, _)| p == "manifest.json").map(|(_, b)| text_of(b)).unwrap_or_default();
        let mut manifest = Manifest::parse(&manifest_text).map_err(|e| alloc::format!("manifest.json: {:?}", e))?;
        if let Some(loc) = manifest.locale_file() { if let Some((_, m)) = unpacked.iter().find(|(p, _)| *p == loc) { manifest.localize(&text_of(m)); } }
        let mut id = make_id(&manifest.name);
        // Installing again replaces the same extension.
        self.extensions.list.retain(|e| e.id != id);
        let w = &mut self.window;
        let _ = files::mkdir(w, b"Web");
        let _ = files::mkdir(w, DIR.as_bytes());
        let base = alloc::format!("{}/{}", DIR, id);
        let _ = files::mkdir(w, base.as_bytes());
        let mut made: Vec<String> = Vec::new();
        for (path, data) in &unpacked {
            // Folders on the way to the file.
            let mut at = 0;
            while let Some(i) = path[at..].find('/') {
                let dir = alloc::format!("{}/{}", base, &path[..at + i]);
                if !made.contains(&dir) { let _ = files::mkdir(w, dir.as_bytes()); made.push(dir); }
                at += i + 1;
            }
            files::write(w, alloc::format!("{}/{}", base, path).as_bytes(), data).map_err(|e| String::from(core::str::from_utf8(files::message(e)).unwrap_or("could not save")))?;
        }
        let _ = files::sync(w);
        sex_pdx::serial_record!("[web.extension.installed] id={} files={} name={}", id, unpacked.len(), manifest.name);
        if let Some(mut ext) = Ext::load(&mut self.window, &id, &[]) { ext.fresh = true; self.extensions.list.push(ext); } else { id = String::new(); }
        // A new version starts its background again.
        self.extensions.backgrounds.retain(|b| b.id != id);
        self.start_backgrounds();
        Ok(id)
    }

    /// `about:extensions`, after doing what its address asks (`?install=`, `?toggle=`,
    /// `?remove=`).
    pub(super) fn extensions_page(&mut self, url: &str) -> String {
        let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut note = String::new();
        for pair in query.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let v = decode(v);
            match k {
                "install" if !v.is_empty() => match self.install_extension(&v) {
                    Ok(id) => note = alloc::format!("<p style='color:#137333'>Installed <b>{}</b>.</p>", esc(&id)),
                    Err(why) => { sex_pdx::serial_record!("[web.extension.install_failed] why={} url={}", why, v); note = alloc::format!("<p style='color:#b3261e'>Could not install: {}</p>", esc(&why)) }
                },
                "toggle" => {
                    if let Some(e) = self.extensions.list.iter_mut().find(|e| e.id == v) { e.enabled = !e.enabled; sex_pdx::serial_record!("[web.extension.toggle] id={} enabled={}", e.id, e.enabled as u8); }
                    self.extensions.save_off(&mut self.window);
                    self.start_backgrounds();
                }
                "remove" => {
                    if let Some(i) = self.extensions.list.iter().position(|e| e.id == v) {
                        let e = self.extensions.list.remove(i);
                        // Files removes a folder once it is empty; the files go first.
                        remove_tree(&mut self.window, &alloc::format!("{}/{}", DIR, e.id));
                        let _ = files::remove(&mut self.window, alloc::format!("{}/{}.json", STATE, e.id).as_bytes());
                        sex_pdx::serial_record!("[web.extension.removed] id={}", e.id);
                        self.start_backgrounds();
                    }
                }
                _ => {}
            }
        }
        let mut rows = String::new();
        for e in &self.extensions.list {
            let m = &e.manifest;
            let what = alloc::format!("{} content script(s), {} blocking rule(s){}{}", m.content_scripts.len(), e.rule_count,
                if e.skipped > 0 { alloc::format!(", {} rule(s) not supported yet", e.skipped) } else { String::new() },
                if m.background.is_empty() { String::new() } else if self.extensions.backgrounds.iter().any(|b| b.id == e.id) { String::from(", background running") } else { String::from(", background stopped") });
            rows.push_str(&alloc::format!(
                "<li style='margin:10px 0'><b>{}</b> {} <span style='color:#5f6368'>MV{}</span><br>{}<br><span style='color:#5f6368'>{}</span><br>\
                 <a href='about:extensions?toggle={}'>{}</a> · <a href='about:extensions?remove={}'>Remove</a></li>",
                esc(&m.name), esc(&m.extension_version), m.version, esc(&m.description), what, esc(&e.id), if e.enabled { "Turn off" } else { "Turn on" }, esc(&e.id)));
        }
        if rows.is_empty() { rows.push_str("<li>None yet.</li>"); }
        alloc::format!(
            "<title>Extensions</title><body style='margin:24px;font-size:15px;font-family:sans-serif'><h1>Extensions</h1>{}\
             <form action='about:extensions'><p>Install from a .crx or .zip address:<br><input name='install' size='50'> <button>Install</button></p></form>\
             <ul style='list-style:none;padding:0'>{}</ul>",
            note, rows)
    }
}

fn remove_tree(window: &mut Window, path: &str) {
    if let Ok(entries) = files::list(window, path.as_bytes()) {
        for e in entries {
            let Ok(name) = core::str::from_utf8(&e.name) else { continue };
            let child = alloc::format!("{}/{}", path, name);
            if e.dir { remove_tree(window, &child) } else { let _ = files::remove(window, child.as_bytes()); }
        }
    }
    let _ = files::remove(window, path.as_bytes());
}

impl Browser {
    /// The bytes of `kaleidoscope-extension://ID/PATH`, from an extension that is on.
    pub(super) fn extension_file(&mut self, url: &str) -> Option<Vec<u8>> {
        let (id, path) = split_url(url)?;
        if !self.extensions.list.iter().any(|e| e.enabled && e.id == id) { return None; }
        read_bytes(&mut self.window, &alloc::format!("{}/{}/{}", DIR, id, path))
    }

    /// The tab extensions see, as JSON.
    pub(super) fn ext_tab_json(&self) -> String { self.ext_tab() }
    fn ext_tab(&self) -> String {
        json!({ "id": 1, "index": 0, "windowId": 1, "active": true, "highlighted": true, "incognito": false, "status": "complete", "url": self.url, "title": self.page.title }).to_string()
    }

    /// Starts the background of every extension that is on and has one, and isn't
    /// running yet; stops those of extensions turned off or removed.
    pub(super) fn start_backgrounds(&mut self) {
        let list = &self.extensions.list;
        self.extensions.backgrounds.retain(|b| list.iter().any(|e| e.enabled && e.id == b.id));
        let ids: Vec<String> = self.extensions.list.iter()
            .filter(|e| e.enabled && !e.manifest.background.is_empty() && !self.extensions.backgrounds.iter().any(|b| b.id == e.id))
            .map(|e| e.id.clone()).collect();
        for id in ids { self.start_background(&id); }
        self.ext_pump();
    }

    fn start_background(&mut self, id: &str) {
        let Some(i) = self.extensions.list.iter().position(|e| e.id == id) else { return };
        let started = sex_pdx::get_ticks();
        let base = alloc::format!("{}{}/", SCHEME, id);
        let mut doc = Document::new(sex_html::dom::Dom::new(), &alloc::format!("{}_generated_background_page.html", base), self.view, clock);
        doc.measure = sex_web::canvas::measure_css;
        doc.storage = Some(self.profile.storage.clone());
        doc.wall = wall;
        doc.ext_key = self.extensions.key.clone();
        doc.ext_tab = self.ext_tab();
        let mut script = match Script::new(doc, sex_js::Limits::default()) {
            Ok(s) => Box::new(s),
            Err(e) => { sex_pdx::serial_record!("[web.extension.background] id={} engine={:?}", id, e); return; }
        };
        let api = self.extensions.list[i].api(&self.extensions.key, "bg");
        let _ = script.eval(&alloc::format!("globalThis.chrome=globalThis.browser={};globalThis.self=globalThis;globalThis.importScripts=function(){{}};", api), "kaleidoscope-extension-api.js");
        let (files, module, fresh) = { let e = &mut self.extensions.list[i]; (e.manifest.background.clone(), e.manifest.background_module, core::mem::take(&mut e.fresh)) };
        let mut ran = 0;
        for path in files {
            let name = alloc::format!("{}{}", base, path);
            let Some(source) = self.extension_file(&name).map(|b| text_of(&b)) else { continue };
            // An MV2 background page names its scripts in HTML.
            if path.ends_with(".html") {
                let dom = sex_html::parse::parse(&source);
                for n in dom.descendants(sex_html::dom::Dom::ROOT) {
                    if dom.tag(n) != Some("script") { continue; }
                    let Some(src) = dom.attr(n, "src").and_then(clean_path) else { continue };
                    let Some(code) = self.extension_file(&alloc::format!("{}{}", base, src)).map(|b| text_of(&b)) else { continue };
                    let is_module = dom.attr(n, "type") == Some("module");
                    if self.run_background_source(&mut script, code, alloc::format!("{}{}", base, src), is_module) { ran += 1; }
                }
                continue;
            }
            if self.run_background_source(&mut script, source, name, module) { ran += 1; }
        }
        let e = if fresh { json!({ "k": "installed", "id": id, "reason": "install" }) } else { json!({ "k": "startup", "id": id }) };
        let _ = script.dispatch(&alloc::format!("ext\t{}", e));
        if fresh { let _ = script.dispatch(&alloc::format!("ext\t{}", json!({ "k": "startup", "id": id }))); }
        sex_pdx::serial_record!("[web.extension.background] id={} scripts={} module={} ticks={} memory={}", id, ran, module as u8, sex_pdx::get_ticks() - started, script.memory_bytes());
        self.extensions.backgrounds.push(Background { id: String::from(id), script, timer: Some(sex_pdx::get_ticks()) });
        self.background_log(self.extensions.backgrounds.len() - 1);
    }

    fn run_background_source(&mut self, script: &mut Script<Document>, source: String, name: String, module: bool) -> bool {
        let r = if module {
            let s = PageScript { source, name, module: true, node: None, external: true };
            self.load_modules(script, &s);
            script.eval_module(&s.source, &s.name)
        } else { script.eval(&source, &name) };
        if let Err(e) = &r { sex_pdx::serial_record!("[web.extension.background.error] {:?}", e); }
        r.is_ok()
    }

    /// Prints a background's console, and serves its requests.
    fn background_log(&mut self, i: usize) {
        let Some(b) = self.extensions.backgrounds.get_mut(i) else { return };
        let id = b.id.clone();
        for (level, text) in b.script.page_mut().log.drain(..) {
            for line in text.lines().take(8) { sex_pdx::serial_record!("[web.extension.console] id={} level={} {}", id, level, line); }
        }
        // Its requests are answered in turn: a background rarely asks for much.
        for _ in 0..8 {
            let Some(b) = self.extensions.backgrounds.get_mut(i) else { return };
            let asked = core::mem::take(&mut b.script.page_mut().fetches);
            if asked.is_empty() { break; }
            for mut ask in asked {
                if !ask.url.contains("://") { ask.url = alloc::format!("{}{}/{}", SCHEME, id, ask.url.trim_start_matches("./").trim_start_matches('/')); }
                let r = match self.extension_file(&ask.url) {
                    Some(body) => super::Loaded { url: ask.url.clone(), status: 200, headers: String::new(), body },
                    None if ask.url.starts_with(SCHEME) => super::Loaded { url: ask.url.clone(), status: 404, headers: String::new(), body: Vec::new() },
                    None => {
                        let headers: Vec<(String, String)> = ask.headers.lines().filter_map(|l| l.split_once(':')).map(|(k, v)| (String::from(k.trim()), String::from(v.trim()))).collect();
                        let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                        let method = if ask.method.is_empty() { "GET" } else { ask.method.as_str() };
                        match self.http(method, &ask.url, &h, &ask.body, super::FETCH_MAX, None) {
                            Ok(r) => r,
                            Err(_) => super::Loaded { url: ask.url.clone(), status: 0, headers: String::new(), body: Vec::new() },
                        }
                    }
                };
                sex_pdx::serial_record!("[web.extension.fetch] id={} status={} bytes={} url={}", id, r.status, r.body.len(), ask.url);
                let Some(b) = self.extensions.backgrounds.get_mut(i) else { return };
                let n = r.body.len();
                b.script.page_mut().keep_head(ask.id, &r.url, &r.headers);
                b.script.page_mut().keep_body(ask.id, r.status, r.body);
                let _ = b.script.dispatch(&alloc::format!("fetched\t{}\t{}\t{}", ask.id, r.status, n));
                b.timer = Some(sex_pdx::get_ticks());
            }
        }
    }

    /// When a background wants its timers run next.
    pub(super) fn backgrounds_due(&self) -> Option<u64> { self.extensions.backgrounds.iter().filter_map(|b| b.timer).min() }

    /// Runs the backgrounds' timers that are due.
    pub(super) fn backgrounds_tick(&mut self) {
        let now = sex_pdx::get_ticks();
        for i in 0..self.extensions.backgrounds.len() {
            let b = &mut self.extensions.backgrounds[i];
            if !b.timer.is_some_and(|t| t <= now) { continue; }
            let next = b.script.dispatch("tick");
            b.timer = match next { Ok(ms) if ms >= 0 => Some(sex_pdx::get_ticks() + ms as u64), _ => None };
            self.background_log(i);
        }
    }

    /// Delivers what extension code posted (messages, answers, storage changes, asks of
    /// the browser) until every realm is quiet.
    pub(super) fn ext_pump(&mut self) {
        if self.extensions.backgrounds.is_empty() && self.doc.get().ext_outbox.is_empty() { return; }
        let tab = self.ext_tab();
        for b in self.extensions.backgrounds.iter_mut() { b.script.page_mut().ext_tab = tab.clone(); }
        for _ in 0..ROUNDS {
            let page = Realm::Page(self.doc.get().owner());
            let mut posted: Vec<(Realm, String)> = self.doc.get().ext_outbox.drain(..).map(|t| (page.clone(), t)).collect();
            for b in self.extensions.backgrounds.iter_mut() {
                let id = b.id.clone();
                posted.extend(b.script.page_mut().ext_outbox.drain(..).map(|t| (Realm::Background(id.clone()), t)));
            }
            if posted.is_empty() { return; }
            for (from, text) in posted {
                let Ok(m) = serde_json::from_str::<Value>(&text) else { continue };
                self.ext_route(from, &m);
            }
            for i in 0..self.extensions.backgrounds.len() { self.background_log(i); }
            if matches!(self.doc, Doc::Scripted(_)) { self.after_script(true); }
        }
        sex_pdx::serial_record!("[web.extension.busy] rounds={}", ROUNDS);
    }

    /// Dispatches `e` into a realm. False when there's no such realm (or it fell over).
    fn ext_deliver(&mut self, to: &Realm, e: &Value) -> bool {
        let text = alloc::format!("ext\t{}", e);
        match to {
            Realm::Page(owner) => {
                // Only the document that asked: after a tab switch or a new page it is gone.
                if self.doc.get().owner() != *owner { sex_pdx::serial_record!("[web.extension.undelivered] page={}", owner); return false; }
                let ok = match self.doc.script() { Some(s) => s.dispatch(&text).is_ok(), None => false };
                // A listener may have set a timer and changed nothing else.
                self.timers_changed();
                ok
            }
            Realm::Background(id) => match self.extensions.backgrounds.iter_mut().find(|b| b.id == *id) {
                Some(b) => { let ok = b.script.dispatch(&text).is_ok(); b.timer = Some(sex_pdx::get_ticks()); ok }
                None => false,
            },
        }
    }

    /// An answer from the browser itself (its value as JSON text, as extension code sends them).
    fn ext_answer(&mut self, to: &Realm, n: &Value, r: Value) { self.ext_deliver(to, &json!({ "k": "reply", "n": n, "r": r.to_string() })); }

    fn ext_route(&mut self, from: Realm, m: &Value) {
        let (t, n) = (m["t"].as_str().unwrap_or(""), &m["n"]);
        let Some(id) = m["id"].as_str().map(String::from) else { return };
        // Code may speak only for its own extension, and only one that is on.
        let Some(ei) = self.extensions.list.iter().position(|e| e.enabled && e.id == id) else { return };
        if matches!(&from, Realm::Background(b) if *b != id) { return; }
        sex_pdx::serial_record!("[web.extension.message] id={} t={} from={}", id, t, from.tag());
        match t {
            "send" | "tabsend" => {
                let to = if t == "send" { Realm::Background(id.clone()) } else { Realm::Page(self.doc.get().owner()) };
                let sender = match &from {
                    Realm::Page(_) => json!({ "id": id, "url": self.url, "origin": super::origin_of(&self.url), "frameId": 0, "tab": serde_json::from_str::<Value>(&self.ext_tab()).unwrap_or(Value::Null) }),
                    Realm::Background(_) => json!({ "id": id, "url": alloc::format!("{}{}/_generated_background_page.html", SCHEME, id) }),
                };
                let e = json!({ "k": "msg", "id": id, "n": n, "m": m["m"], "to": from.tag(), "sender": sender });
                if to == from || !self.ext_deliver(&to, &e) {
                    self.ext_deliver(&from, &json!({ "k": "reply", "n": n, "e": "Could not establish connection. Receiving end does not exist." }));
                }
            }
            "reply" => {
                let Some(to) = Realm::from_tag(m["to"].as_str().unwrap_or("")) else { return };
                let r = if m["none"].as_bool() == Some(true) { json!({ "k": "reply", "n": n }) } else { json!({ "k": "reply", "n": n, "r": m["r"] }) };
                self.ext_deliver(&to, &r);
            }
            "changed" => {
                let e = json!({ "k": "changed", "id": id, "area": m["area"], "changes": m["changes"] });
                let page = Realm::Page(self.doc.get().owner());
                if from != page { self.ext_deliver(&page, &e); }
                let bg = Realm::Background(id.clone());
                if from != bg { self.ext_deliver(&bg, &e); }
            }
            "exec" | "css" => {
                let mut code = String::new();
                let mut css = String::from(m["css"].as_str().unwrap_or(""));
                for f in m["files"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    let Some(path) = clean_path(f) else { continue };
                    let Some(text) = self.extensions.list[ei].file(&mut self.window, &path) else { continue };
                    if t == "css" { css.push('\n'); css.push_str(&text); } else { code.push_str(&text); code.push_str("\n;\n"); }
                }
                code.push_str(m["code"].as_str().unwrap_or(""));
                if t == "css" && !css.is_empty() {
                    self.sheets.push(css);
                    self.relayout();
                    self.draw();
                } else if !code.is_empty() {
                    let wrapped = self.extensions.list[ei].wrap(&self.extensions.key, &code);
                    let name = alloc::format!("{}{}/executeScript.js", SCHEME, id);
                    match self.doc.script() {
                        Some(s) => { let _ = s.eval(&wrapped, &name); }
                        None => self.run_scripts(alloc::vec![PageScript { source: wrapped, name, module: false, node: None, external: false }]),
                    }
                }
                self.ext_answer(&from, n, Value::Null);
            }
            "register" => {
                let e = &mut self.extensions.list[ei];
                for s in m["scripts"].as_array().into_iter().flatten() {
                    let sid = s["id"].clone();
                    match e.registered.iter_mut().find(|r| r["id"] == sid) {
                        // `updateContentScripts` changes only what it names.
                        Some(r) => if let (Some(old), Some(new)) = (r.as_object_mut(), s.as_object()) { for (k, v) in new { old.insert(k.clone(), v.clone()); } },
                        None => e.registered.push(s.clone()),
                    }
                }
                e.save_state(&mut self.window);
                sex_pdx::serial_record!("[web.extension.registered] id={} scripts={}", id, self.extensions.list[ei].registered.len());
                self.ext_answer(&from, n, Value::Null);
            }
            "unregister" => {
                let e = &mut self.extensions.list[ei];
                match m["ids"].as_array() { Some(ids) => e.registered.retain(|r| !ids.contains(&r["id"])), None => e.registered.clear() }
                e.save_state(&mut self.window);
                self.ext_answer(&from, n, Value::Null);
            }
            "registered" => {
                let e = &self.extensions.list[ei];
                let list: Vec<Value> = e.registered.iter().filter(|r| m["ids"].as_array().is_none_or(|ids| ids.contains(&r["id"]))).cloned().collect();
                self.ext_answer(&from, n, Value::Array(list));
            }
            "rulesets" => {
                let e = &mut self.extensions.list[ei];
                if m.get("enable").is_some() || m.get("disable").is_some() {
                    let mut on = e.rulesets_enabled();
                    for d in m["disable"].as_array().into_iter().flatten().filter_map(Value::as_str) { on.retain(|x| x != d); }
                    for a in m["enable"].as_array().into_iter().flatten().filter_map(Value::as_str) { if !on.iter().any(|x| x == a) && e.manifest.rulesets.iter().any(|r| r.id == a) { on.push(String::from(a)); } }
                    e.rulesets_on = Some(on);
                    e.rebuild(&mut self.window);
                    e.save_state(&mut self.window);
                    sex_pdx::serial_record!("[web.extension.rulesets] id={} on={} rules={}", id, e.rulesets_enabled().join(","), e.rule_count);
                }
                let on = self.extensions.list[ei].rulesets_enabled();
                self.ext_answer(&from, n, json!(on));
            }
            "rules" => {
                let e = &mut self.extensions.list[ei];
                let dynamic = m["which"].as_str() != Some("session");
                if m.get("add").is_some() || m.get("remove").is_some() {
                    let list = if dynamic { &mut e.dynamic } else { &mut e.session };
                    let remove: Vec<Value> = m["remove"].as_array().cloned().unwrap_or_default();
                    let add: Vec<Value> = m["add"].as_array().cloned().unwrap_or_default();
                    list.retain(|r| !remove.contains(&r["id"]) && !add.iter().any(|a| a["id"] == r["id"]));
                    list.extend(add);
                    e.rebuild(&mut self.window);
                    if dynamic { e.save_state(&mut self.window); }
                    sex_pdx::serial_record!("[web.extension.rules] id={} dynamic={} session={} rules={}", id, e.dynamic.len(), e.session.len(), e.rule_count);
                }
                let e = &self.extensions.list[ei];
                self.ext_answer(&from, n, Value::Array(if dynamic { e.dynamic.clone() } else { e.session.clone() }));
            }
            "open" | "reload-tab" => {
                let url = if t == "open" { String::from(m["url"].as_str().unwrap_or("")) } else { self.url.clone() };
                if !url.is_empty() { self.doc.get().navigate = Some(url); }
            }
            "reload" => { self.extensions.backgrounds.retain(|b| b.id != id); self.start_background(&id); }
            _ => {}
        }
    }
}
