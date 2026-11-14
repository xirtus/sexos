// SPDX-License-Identifier: GPL-3.0-only
//! Browser extensions for Kaleidoscope, without any I/O: what an extension is and what
//! it asks for, so the browser only has to load files and run scripts.
//!
//! * [`Manifest`]: `manifest.json` of Manifest V2 and V3 (name, content scripts,
//!   declarativeNetRequest rulesets, background, permissions), with `__MSG_x__` names
//!   read from `_locales`.
//! * [`Pattern`]: Chrome match patterns (`*://*.example.com/*`, `<all_urls>`).
//! * [`dnr`]: declarativeNetRequest rules turned into Shields filters, so an MV3
//!   blocker (uBlock Origin Lite, for one) blocks through the same engine as Shields.
//! * [`package`]: `.crx` and `.zip` packages unpacked into (path, bytes).
#![no_std]

extern crate alloc;

pub mod dnr;
pub mod package;
#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;
use serde_json::Value;

/// When a content script runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunAt { Start, End, Idle }

/// A `content_scripts` entry.
#[derive(Clone, Debug)]
pub struct ContentScript {
    pub matches: Vec<Pattern>,
    pub exclude: Vec<Pattern>,
    pub js: Vec<String>,
    pub css: Vec<String>,
    pub run_at: RunAt,
    pub all_frames: bool,
}

impl ContentScript {
    /// Whether this script goes into a page at `url` (in a frame or not).
    pub fn applies(&self, url: &str, in_frame: bool) -> bool {
        (self.all_frames || !in_frame) && self.matches.iter().any(|p| p.matches(url)) && !self.exclude.iter().any(|p| p.matches(url))
    }
}

/// A declarativeNetRequest ruleset the manifest names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ruleset { pub id: String, pub path: String, pub enabled: bool }

/// What `manifest.json` says.
#[derive(Clone, Debug)]
pub struct Manifest {
    pub version: u32,
    pub name: String,
    pub extension_version: String,
    pub description: String,
    pub default_locale: Option<String>,
    pub content_scripts: Vec<ContentScript>,
    pub rulesets: Vec<Ruleset>,
    /// MV3 `background.service_worker`, or MV2 `background.scripts` / `page`.
    pub background: Vec<String>,
    pub permissions: Vec<String>,
    pub host_permissions: Vec<Pattern>,
    /// The page the toolbar button opens (`action`/`browser_action` `default_popup`).
    pub popup: Option<String>,
    pub options_page: Option<String>,
}

/// Why a manifest can't be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error { Json, NotObject, Version, NoName, Package(&'static str) }

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|s| s.as_str()).map(String::from).collect(),
        Some(Value::String(s)) => alloc::vec![s.clone()],
        _ => Vec::new(),
    }
}

/// A safe relative path inside the extension (no `..`, no leading `/`).
pub fn clean_path(p: &str) -> Option<String> {
    let p = p.trim().trim_start_matches("./").trim_start_matches('/');
    if p.is_empty() || p.contains('\\') || p.contains('\0') || p.split('/').any(|s| s == ".." || s == ".") { return None; }
    Some(String::from(p))
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, Error> {
        // Chrome accepts comments in manifest.json; strip whole-line ones.
        let cleaned: String = text.trim_start_matches('\u{feff}').lines().filter(|l| !l.trim_start().starts_with("//")).flat_map(|l| l.chars().chain(core::iter::once('\n'))).collect();
        let v: Value = serde_json::from_str(&cleaned).map_err(|_| Error::Json)?;
        let o = v.as_object().ok_or(Error::NotObject)?;
        let version = o.get("manifest_version").and_then(Value::as_u64).ok_or(Error::Version)? as u32;
        if !(2..=3).contains(&version) { return Err(Error::Version); }
        let name = String::from(o.get("name").and_then(Value::as_str).ok_or(Error::NoName)?);
        let mut content_scripts = Vec::new();
        for cs in o.get("content_scripts").and_then(Value::as_array).into_iter().flatten() {
            let pats = |k: &str| strings(cs.get(k)).iter().filter_map(|p| Pattern::parse(p)).collect::<Vec<_>>();
            let run_at = match cs.get("run_at").and_then(Value::as_str) { Some("document_start") => RunAt::Start, Some("document_end") => RunAt::End, _ => RunAt::Idle };
            content_scripts.push(ContentScript {
                matches: pats("matches"), exclude: pats("exclude_matches"),
                js: strings(cs.get("js")).iter().filter_map(|p| clean_path(p)).collect(),
                css: strings(cs.get("css")).iter().filter_map(|p| clean_path(p)).collect(),
                run_at, all_frames: cs.get("all_frames").and_then(Value::as_bool).unwrap_or(false),
            });
        }
        let mut rulesets = Vec::new();
        if let Some(r) = o.get("declarative_net_request").and_then(|d| d.get("rule_resources")).and_then(Value::as_array) {
            for r in r {
                let (Some(id), Some(path)) = (r.get("id").and_then(Value::as_str), r.get("path").and_then(Value::as_str).and_then(clean_path)) else { continue };
                rulesets.push(Ruleset { id: String::from(id), path, enabled: r.get("enabled").and_then(Value::as_bool).unwrap_or(false) });
            }
        }
        let bg = o.get("background");
        let mut background = Vec::new();
        if let Some(b) = bg {
            background.extend(strings(b.get("service_worker")));
            background.extend(strings(b.get("scripts")));
            background.extend(strings(b.get("page")));
        }
        let background = background.iter().filter_map(|p| clean_path(p)).collect();
        let mut permissions = strings(o.get("permissions"));
        let mut host_permissions: Vec<Pattern> = strings(o.get("host_permissions")).iter().filter_map(|p| Pattern::parse(p)).collect();
        // MV2 lists hosts among the permissions.
        permissions.retain(|p| match Pattern::parse(p) { Some(pat) if p.contains("://") || p == "<all_urls>" => { host_permissions.push(pat); false } _ => true });
        let action = o.get("action").or_else(|| o.get("browser_action")).or_else(|| o.get("page_action"));
        let popup = action.and_then(|a| a.get("default_popup")).and_then(Value::as_str).and_then(clean_path);
        let options_page = o.get("options_page").and_then(Value::as_str).or_else(|| o.get("options_ui").and_then(|u| u.get("page")).and_then(Value::as_str)).and_then(clean_path);
        Ok(Manifest {
            version, name,
            extension_version: String::from(o.get("version").and_then(Value::as_str).unwrap_or("")),
            description: String::from(o.get("description").and_then(Value::as_str).unwrap_or("")),
            default_locale: o.get("default_locale").and_then(Value::as_str).map(String::from),
            content_scripts, rulesets, background, permissions, host_permissions, popup, options_page,
        })
    }

    /// Replaces `__MSG_key__` in the name and description from a locale's
    /// `messages.json` (keys match without regard to case, as in Chrome).
    pub fn localize(&mut self, messages: &str) {
        let Ok(Value::Object(m)) = serde_json::from_str::<Value>(messages) else { return };
        let look = |s: &str| -> Option<String> {
            let key = s.strip_prefix("__MSG_")?.strip_suffix("__")?;
            m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).and_then(|(_, v)| v.get("message")).and_then(Value::as_str).map(String::from)
        };
        if let Some(n) = look(&self.name) { self.name = n; }
        if let Some(d) = look(&self.description) { self.description = d; }
    }

    /// The path of the locale file `localize` wants, if the name needs one.
    pub fn locale_file(&self) -> Option<String> {
        if !self.name.starts_with("__MSG_") && !self.description.starts_with("__MSG_") { return None; }
        Some(alloc::format!("_locales/{}/messages.json", self.default_locale.as_deref().unwrap_or("en")))
    }
}

/// A Chrome match pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    /// `None`: `*` (http and https, also ws and wss).
    scheme: Option<String>,
    /// `None`: any host; `Some((host, subdomains))`.
    host: Option<(String, bool)>,
    path: String,
    all: bool,
}

impl Pattern {
    pub fn parse(p: &str) -> Option<Pattern> {
        let p = p.trim();
        if p == "<all_urls>" { return Some(Pattern { scheme: None, host: None, path: String::from("/*"), all: true }); }
        let (scheme, rest) = p.split_once("://")?;
        let scheme = match scheme { "*" => None, "http" | "https" | "ws" | "wss" | "file" | "ftp" => Some(String::from(scheme)), _ => return None };
        let (host, path) = match rest.find('/') { Some(i) => (&rest[..i], &rest[i..]), None => return None };
        let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
        let host = if host == "*" { None }
            else if let Some(h) = host.strip_prefix("*.") { if h.contains('*') { return None; } Some((String::from(h), true)) }
            else if host.contains('*') { return None }
            else { Some((host, false)) };
        Some(Pattern { scheme, host, path: String::from(path), all: false })
    }

    pub fn matches(&self, url: &str) -> bool {
        let url = url.split('#').next().unwrap_or(url);
        let Some((scheme, rest)) = url.split_once("://") else { return false };
        let ok_scheme = match &self.scheme {
            Some(s) => s.eq_ignore_ascii_case(scheme),
            None if self.all => matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "file"),
            None => matches!(scheme, "http" | "https" | "ws" | "wss"),
        };
        if !ok_scheme { return false; }
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let auth = &rest[..end];
        let host = auth.rsplit('@').next().unwrap_or(auth);
        let host = if host.starts_with('[') { host } else { host.split(':').next().unwrap_or(host) }.to_ascii_lowercase();
        let path = if end < rest.len() { &rest[end..] } else { "/" };
        let path = if path.starts_with('?') { alloc::format!("/{}", path) } else { String::from(path) };
        let ok_host = match &self.host {
            None => true,
            Some((h, false)) => host == *h,
            Some((h, true)) => host == *h || host.ends_with(h.as_str()) && host.as_bytes().get(host.len() - h.len() - 1) == Some(&b'.'),
        };
        ok_host && glob(self.path.as_bytes(), path.as_bytes())
    }
}

/// `*` matches any run; everything else itself.
fn glob(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si, mut star) = (0, 0, None);
    while si < s.len() {
        if pi < p.len() && p[pi] == b'*' { star = Some((pi, si)); pi += 1; }
        else if pi < p.len() && p[pi] == s[si] { pi += 1; si += 1; }
        else if let Some((sp, ss)) = star { pi = sp + 1; si = ss + 1; star = Some((sp, ss + 1)); }
        else { return false; }
    }
    p[pi..].iter().all(|&c| c == b'*')
}
