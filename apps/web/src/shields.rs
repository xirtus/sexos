// SPDX-License-Identifier: GPL-3.0-only
//! Shields: the browser's ad and tracker blocking, on by default as in Brave.
//!
//! Every request a page makes (pictures, style sheets, scripts, frames, `fetch`,
//! media) is judged by [`sex_shields::Engine`] before it leaves; a blocked one never
//! reaches the network and is counted in the bar's shield badge. Element hiding rules
//! go into the page's style sheets. The filters are Shields' small starter list and,
//! once downloaded from `about:shields`, EasyList and EasyPrivacy, kept in Files under
//! `Web/`. Ctrl+Shift+S turns Shields off (and on again) for the shown site.

use super::{text_of, Browser};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::{files, Window};
use sex_shields::{Engine, Kind, Request, Verdict};
use sex_web::sex_html;

/// Sites the person turned Shields off for, one host a line.
const OFF: &[u8] = b"Web/shields-off.tsv";
/// Largest list file read or downloaded (EasyList is about 2 MB).
const LIST_MAX: usize = 16 << 20;
/// Requests blocked lately, for `about:shields`.
const RECENT: usize = 40;

/// The word `send` gives for a request Shields stopped.
pub const BLOCKED: &[u8] = b"Shields blocked this request.";

fn list_path(name: &str) -> String { alloc::format!("Web/shields-{}", name) }

fn host_of(url: &str) -> Option<String> { sex_fetch::Url::parse(url).ok().map(|u| u.host.to_ascii_lowercase()) }

pub struct Shields {
    engine: Box<Engine>,
    /// Hosts with Shields down.
    off: Vec<String>,
    /// Lists loaded besides the starter list: (file name, bytes).
    pub lists: Vec<(&'static str, usize)>,
    /// Requests blocked since the program started.
    pub total: u64,
    /// The latest blocked requests: (kind, address, filter).
    pub recent: Vec<(&'static str, String, String)>,
}

impl Shields {
    /// No filters at all (until `load`).
    pub fn empty() -> Shields { Shields { engine: Box::new(Engine::new()), off: Vec::new(), lists: Vec::new(), total: 0, recent: Vec::new() } }

    /// The starter list, the downloaded lists in Files, and the sites turned off.
    pub fn load(window: &mut Window) -> Shields {
        let begun = sex_pdx::get_ticks();
        let mut s = Shields::empty();
        s.engine.add_list(sex_shields::STARTER_LIST);
        for (name, _) in sex_shields::LISTS {
            if let Some(text) = read(window, list_path(name).as_bytes()) {
                s.engine.add_list(&text);
                s.lists.push((name, text.len()));
            }
        }
        if let Some(text) = read(window, OFF) { s.off = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect(); }
        let c = s.engine.counts();
        sex_pdx::serial_record!("[web.shields.loaded] lists={} network={} cosmetic={} skipped={} off={} ticks={}", s.lists.len() + 1, c.network, c.cosmetic, c.skipped, s.off.len(), sex_pdx::get_ticks() - begun);
        s
    }

    /// Whether Shields are up for pages of `page`'s site.
    pub fn up(&self, page: &str) -> bool { host_of(page).is_none_or(|h| !self.off.contains(&h)) }

    /// Judges a request from `page`. True when it must not go out.
    pub fn blocks(&mut self, url: &str, page: &str, kind: Kind) -> bool {
        if !page.starts_with("http") || !self.up(page) { return false; }
        match self.engine.check(&Request { url, page, kind }) {
            Verdict::Block(id) => {
                let filter = String::from(self.engine.filter_text(id));
                sex_pdx::serial_record!("[web.shields.blocked] kind={} filter={} url={}", kind.name(), filter, url);
                self.total += 1;
                if self.recent.len() >= RECENT { self.recent.remove(0); }
                self.recent.push((kind.name(), String::from(url), filter));
                true
            }
            _ => false,
        }
    }

    /// The hiding style sheet for `page`, from the classes and ids in `dom`.
    pub fn css(&self, page: &str, dom: &sex_html::dom::Dom) -> Option<String> {
        if !page.starts_with("http") || !self.up(page) { return None; }
        let (mut classes, mut ids): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
        for node in &dom.nodes {
            let sex_html::dom::Kind::Element { attrs, .. } = &node.kind else { continue };
            for (k, v) in attrs {
                match k.as_str() { "class" => classes.extend(v.split_ascii_whitespace()), "id" => ids.push(v.trim()), _ => {} }
            }
        }
        classes.sort_unstable();
        classes.dedup();
        ids.sort_unstable();
        ids.dedup();
        let css = self.engine.css(page, &|c| classes.binary_search(&c).is_ok(), &|i| ids.binary_search(&i).is_ok());
        sex_pdx::serial_record!("[web.shields.hide] rules={} classes={} ids={} url={}", css.lines().count(), classes.len(), ids.len(), page);
        (!css.is_empty()).then_some(css)
    }

    /// Turns Shields off for `page`'s site, or on again. Returns whether they are up.
    pub fn toggle(&mut self, window: &mut Window, page: &str) -> bool {
        let Some(host) = host_of(page) else { return true };
        let up = match self.off.iter().position(|h| *h == host) {
            Some(i) => { self.off.remove(i); true }
            None => { self.off.push(host.clone()); false }
        };
        let mut text = String::new();
        for h in &self.off { text.push_str(h); text.push('\n'); }
        let _ = files::mkdir(window, b"Web");
        let _ = files::write(window, OFF, text.as_bytes());
        sex_pdx::serial_record!("[web.shields.toggle] up={} host={}", up as u8, host);
        up
    }
}

fn read(window: &mut Window, path: &[u8]) -> Option<String> {
    let size = files::size(window, path).ok()?;
    if size == 0 || size as usize > LIST_MAX { return None; }
    let mut buf = alloc::vec![0u8; size as usize];
    let n = files::read(window, path, &mut buf).ok()?;
    buf.truncate(n);
    Some(text_of(&buf))
}

fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('\'', "&#39;") }

impl Browser {
    /// Downloads EasyList and EasyPrivacy into Files and starts using them.
    pub(super) fn update_shield_lists(&mut self) -> String {
        let mut report = String::new();
        let _ = files::mkdir(&mut self.window, b"Web");
        for (name, url) in sex_shields::LISTS {
            self.status = Some("Downloading filter lists…");
            self.draw();
            let got = self.fetch_up_to(url, LIST_MAX).map(|(_, body)| self.buf[body].to_vec());
            self.buf = Vec::new();
            match got {
                Ok(bytes) if bytes.starts_with(b"[Adblock") || bytes.starts_with(b"!") => {
                    let ok = files::write(&mut self.window, list_path(name).as_bytes(), &bytes).is_ok();
                    sex_pdx::serial_record!("[web.shields.list] name={} bytes={} saved={}", name, bytes.len(), ok as u8);
                    report.push_str(&alloc::format!("<li>{}: {} KB {}</li>", name, bytes.len() / 1024, if ok { "saved" } else { "could not be saved" }));
                }
                Ok(_) => report.push_str(&alloc::format!("<li>{}: not a filter list</li>", name)),
                Err(why) => report.push_str(&alloc::format!("<li>{}: {}</li>", name, esc(core::str::from_utf8(why).unwrap_or("failed")))),
            }
        }
        let total = self.shields.total;
        let recent = core::mem::take(&mut self.shields.recent);
        self.shields = Shields::load(&mut self.window);
        self.shields.total = total;
        self.shields.recent = recent;
        report
    }

    /// `about:shields`: what Shields hold and what they blocked lately.
    pub(super) fn shields_page(&mut self, url: &str) -> String {
        let note = if url == "about:shields?update" { alloc::format!("<ul>{}</ul>", self.update_shield_lists()) } else { String::new() };
        let s = &self.shields;
        let c = s.engine.counts();
        let mut lists = String::from("<li>Shields starter list (built in)</li>");
        for (name, bytes) in &s.lists { lists.push_str(&alloc::format!("<li>{} ({} KB)</li>", name, bytes / 1024)); }
        let mut off = String::new();
        for h in &s.off { off.push_str(&alloc::format!("<li>{}</li>", esc(h))); }
        if off.is_empty() { off.push_str("<li>None: Shields are up everywhere.</li>"); }
        let mut recent = String::new();
        for (kind, u, f) in s.recent.iter().rev() {
            let shown: String = u.chars().take(90).collect();
            recent.push_str(&alloc::format!("<tr><td>{}</td><td>{}</td><td><code>{}</code></td></tr>", kind, esc(&shown), esc(f)));
        }
        alloc::format!(
            "<title>Shields</title><body style='margin:24px;font-size:15px;font-family:sans-serif'>\
             <h1 style='color:#fb542b'>Shields</h1>\
             <p>Kaleidoscope blocks ads and trackers before they load. <b>{}</b> requests blocked since it started.</p>{}\
             <h2>Filters</h2><p>{} network filters, {} hiding rules ({} filters this engine can't honour exactly are left out).</p><ul>{}</ul>\
             <p><a href='about:shields?update'>Download EasyList and EasyPrivacy</a> (again, to update them).</p>\
             <h2>Sites with Shields down</h2><p>Ctrl+Shift+S turns Shields down for the site you are on, and up again.</p><ul>{}</ul>\
             <h2>Blocked lately</h2><table><tr><th>Kind</th><th>Address</th><th>Filter</th></tr>{}</table>",
            s.total, note, c.network, c.cosmetic, c.skipped, lists, off, recent
        )
    }
}
