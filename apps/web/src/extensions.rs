// SPDX-License-Identifier: GPL-3.0-only
//! Extensions (slice N7a): Chrome and Firefox extensions installed from a `.crx` or
//! `.zip` link, kept unpacked in Files under `Web/extensions/<id>/`.
//!
//! What runs so far: content scripts (style sheets and scripts injected into matching
//! pages, at `document_start` before the page's scripts, otherwise after them) and
//! declarativeNetRequest rulesets, which block through the same engine as Shields.
//! Background pages and service workers, `chrome.storage`, messaging and the other
//! `chrome.*` APIs are slice N7b. Content scripts run in the page's own script realm
//! for now, not an isolated one, so a page can see what they define.
//!
//! `about:extensions` lists them, turns each off or on, removes them, and installs
//! one from an address (`about:extensions?install=URL`).

use super::{text_of, Browser, PageScript};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::{files, Window};
use sex_extensions::{dnr, package, Manifest, RunAt};
use sex_shields::{Engine, Kind, Request, Verdict};

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
}

#[derive(Default)]
pub struct Extensions { pub list: Vec<Ext> }

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
        let mut manifest = Manifest::parse(&read(window, &alloc::format!("{}/manifest.json", base))?).ok()?;
        if let Some(loc) = manifest.locale_file() { if let Some(m) = read(window, &alloc::format!("{}/{}", base, loc)) { manifest.localize(&m); } }
        let mut ext = Ext { id: String::from(id), manifest, enabled: !off.iter().any(|o| o == id), rules: None, rule_count: 0, skipped: 0, cache: Vec::new() };
        let sets: Vec<String> = ext.manifest.rulesets.iter().filter(|r| r.enabled).map(|r| r.path.clone()).collect();
        if !sets.is_empty() {
            let mut engine = Box::new(Engine::new());
            for path in sets {
                let Some(json) = read(window, &alloc::format!("{}/{}", base, path)) else { continue };
                match dnr::convert(&json) {
                    Ok(c) => {
                        ext.skipped += c.skipped;
                        for f in &c.filters { if engine.add_filter(f) { ext.rule_count += 1 } else { ext.skipped += 1 } }
                    }
                    Err(_) => ext.skipped += 1,
                }
            }
            ext.rules = Some(engine);
        }
        sex_pdx::serial_record!("[web.extension.loaded] id={} mv={} enabled={} rules={} skipped={} content_scripts={} background={}",
            ext.id, ext.manifest.version, ext.enabled as u8, ext.rule_count, ext.skipped, ext.manifest.content_scripts.len(), ext.manifest.background.len());
        Some(ext)
    }

    fn file(&mut self, window: &mut Window, path: &str) -> Option<String> {
        if let Some((_, t)) = self.cache.iter().find(|(p, _)| p == path) { return Some(t.clone()); }
        let t = read(window, &alloc::format!("{}/{}/{}", DIR, self.id, path))?;
        self.cache.push((String::from(path), t.clone()));
        Some(t)
    }

    /// A content script wrapped so it sees `chrome` and `browser` with the runtime
    /// basics (`id`, `getURL`, `getManifest`).
    fn wrap(&self, source: &str) -> String {
        let id = self.id.replace(['"', '\\'], "");
        alloc::format!(
            "(function(){{const __id=\"{id}\";const runtime={{id:__id,getURL:p=>'kaleidoscope-extension://'+__id+'/'+String(p).replace(/^\\//,''),getManifest:()=>({{name:{name},version:{version},manifest_version:{mv}}})}};\
             const chrome={{runtime}};const browser=chrome;\n{source}\n}})();",
            name = json_str(&self.manifest.name), version = json_str(&self.manifest.extension_version), mv = self.manifest.version,
        )
    }
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() { match c { '"' => o.push_str("\\\""), '\\' => o.push_str("\\\\"), c if (c as u32) < 0x20 => o.push(' '), c => o.push(c) } }
    o.push('"');
    o
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
        Extensions { list }
    }

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
        for e in self.list.iter_mut().filter(|e| e.enabled) {
            let wanted: Vec<(Vec<String>, Vec<String>, RunAt)> = e.manifest.content_scripts.iter().filter(|c| c.applies(url, in_frame)).map(|c| (c.css.clone(), c.js.clone(), c.run_at)).collect();
            for (sheets, scripts, run_at) in wanted {
                for p in sheets { if let Some(t) = e.file(window, &p) { css.push(t); } }
                for p in scripts {
                    let Some(t) = e.file(window, &p) else { continue };
                    let s = PageScript { source: e.wrap(&t), name: alloc::format!("kaleidoscope-extension://{}/{}", e.id, p), module: false, node: None, external: false };
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
        if let Some(ext) = Ext::load(&mut self.window, &id, &[]) { self.extensions.list.push(ext); } else { id = String::new(); }
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
                }
                "remove" => {
                    if let Some(i) = self.extensions.list.iter().position(|e| e.id == v) {
                        let e = self.extensions.list.remove(i);
                        // Files removes a folder once it is empty; the files go first.
                        remove_tree(&mut self.window, &alloc::format!("{}/{}", DIR, e.id));
                        sex_pdx::serial_record!("[web.extension.removed] id={}", e.id);
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
                if m.background.is_empty() { String::new() } else { String::from(", background script (not run yet)") });
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
