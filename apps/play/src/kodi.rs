// SPDX-License-Identifier: GPL-3.0-only
//! Qupid's Kodi add-ons (Q13b): repositories and installs, natively in Rust
//! (`sex_kodi::Manager`). Qupid keeps the addon folder in its own folder, under
//! `kodi/`; plugins run later in the confined Python helper.
//!
//! - `run +net qupid --add-repo URL`: install a repository addon zip ("Install from zip").
//! - `run +net qupid --addons [WORDS]`: the video plugins every repository offers (or
//!   those matching WORDS). Up/Down choose, Enter installs the chosen one with what it
//!   needs, R reads the repositories again, F11 closes.
//! - `run +net qupid --install ID`: install one addon and its dependencies.
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use sex_app::{files, Event, Window};
use sex_kodi::addon::content;
use sex_kodi::manager::{Error, Manager, Net, Store};

const ROOT: &str = "kodi";
const REDIRECTS: usize = 5;

struct WebNet { window: *mut Window }
impl Net for WebNet {
    fn get(&mut self, url: &str, max: usize) -> Result<Vec<u8>, String> {
        let window = unsafe { &mut *self.window };
        let mut url = url.to_string();
        for _ in 0..=REDIRECTS {
            let req = sex_fetch::Request { method: "GET", url: &url, headers: &[], body: &[], trusted_origin: false, referrer: None };
            let r = sex_app::net::request(window, &req, None, max).map_err(|e| format!("{e:?}"))?;
            sex_pdx::serial_record!("[qupid.kodi.get] url={} status={} bytes={}", url, r.head.status, r.body.len());
            if r.head.is_redirect() {
                url = sex_fetch::join(&url, r.head.get("location").unwrap_or("")).ok_or("bad redirect")?;
                continue;
            }
            if r.head.status != 200 { return Err(format!("HTTP {}", r.head.status)); }
            return Ok(r.body);
        }
        Err("too many redirects".to_string())
    }
}

struct OwnFolder { window: *mut Window }
impl Store for OwnFolder {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let window = unsafe { &mut *self.window };
        let p = format!("{ROOT}/{path}");
        let len = files::size(window, p.as_bytes()).ok()? as usize;
        let mut out = alloc::vec![0u8; len];
        let mut at = 0;
        while at < len {
            match files::read_at(window, p.as_bytes(), at as u64, &mut out[at..]) { Ok(n) if n > 0 => at += n, _ => return None }
        }
        Some(out)
    }
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        let window = unsafe { &mut *self.window };
        let p = format!("{ROOT}/{path}");
        // Make the folders on the way; one already there is fine.
        let mut at = 0;
        while let Some(k) = p[at..].find('/') {
            let _ = files::mkdir(window, p[..at + k].as_bytes());
            at += k + 1;
        }
        files::write(window, p.as_bytes(), data).map_err(|e| String::from(core::str::from_utf8(files::message(e)).unwrap_or("write failed")))
    }
}

fn why(e: &Error) -> String {
    match e {
        Error::Fetch { url, why } => format!("Could not fetch {url}: {why}"),
        Error::Package(p) => format!("Not a usable addon zip: {p:?}"),
        Error::NotRepository(id) => format!("{id} is an addon, not a repository"),
        Error::Checksum(url) => format!("Download does not match its checksum ({url})"),
        Error::Index(w) => format!("Cannot read the repository's list: {w}"),
        Error::Plan(p) => format!("Cannot install: {p:?}"),
        Error::NoZip(id) => format!("{id} is not offered as a zip"),
        Error::Store(w) => format!("Cannot save: {w}"),
    }
}

/// Runs the add-ons view or command in `words` (they start with --add-repo, --addons
/// or --install); never returns.
pub fn run(window: &mut Window, words: &[&[u8]]) -> ! {
    window.title(b"Qupid Add-ons");
    let wp: *mut Window = window;
    let (mut net, mut store) = (WebNet { window: wp }, OwnFolder { window: wp });
    let mut m = Manager::load(&mut store);
    sex_pdx::serial_record!("[qupid.kodi.folder] installed={} sources={}", m.installed.len(), m.sources.len());
    let arg = |k: usize| words.get(k).and_then(|w| core::str::from_utf8(w).ok()).unwrap_or("");
    let mut status = String::new();
    match words.first().copied().unwrap_or(b"") {
        b"--add-repo" => {
            window.show(b"Qupid Add-ons\n\nAdding the repository...");
            match m.add_repository(&mut net, &mut store, arg(1)) {
                Ok(a) => {
                    sex_pdx::serial_record!("[qupid.kodi.repo] id={} version={} sources={}", a.id, a.version, m.sources.len());
                    status = format!("Added {} {}.", a.name, a.version);
                }
                Err(e) => { sex_pdx::serial_record!("[qupid.kodi.failed] {:?}", e); status = why(&e); }
            }
        }
        b"--install" => {
            refresh(window, &mut m, &mut net);
            install(window, &mut m, &mut net, &mut store, arg(1), &mut status);
        }
        _ => {}
    }
    let query: String = if words.first().copied() == Some(&b"--addons"[..]) {
        words[1..].iter().filter_map(|w| core::str::from_utf8(w).ok()).collect::<Vec<_>>().join(" ")
    } else { String::new() };
    if m.catalogue.entries.is_empty() { refresh(window, &mut m, &mut net); }
    let mut chosen = 0usize;
    loop {
        let list: Vec<(String, String, String)> = {
            let entries = if query.is_empty() { m.catalogue.plugins(content::VIDEO | content::AUDIO) } else { m.catalogue.search(&query).into_iter().filter(|e| e.addon.is_plugin()).collect() };
            entries.iter().map(|e| (e.addon.id.clone(), e.addon.name.clone(), e.addon.version.clone())).collect()
        };
        chosen = chosen.min(list.len().saturating_sub(1));
        let mut text = format!("Qupid Add-ons  {} repositories, {} plugins", m.sources.len(), list.len());
        if !status.is_empty() { text.push_str("\n"); text.push_str(&status); }
        text.push('\n');
        let top = chosen.saturating_sub(8);
        for (k, (id, name, version)) in list.iter().enumerate().skip(top).take(18) {
            let have = match m.version_of(id) { Some(v) if v == version => "  installed", Some(_) => "  update", None => "" };
            text.push_str(&format!("\n{} {}  {}{}", if k == chosen { "*" } else { " " }, name, version, have));
        }
        if list.is_empty() { text.push_str("\nNo plugins yet. Add a repository: run +net qupid --add-repo URL"); }
        text.push_str("\n\nUp/Down choose  Enter install  R reload  F11 close");
        window.show(text.as_bytes());
        match window.next() {
            Event::Close => sex_pdx::task_finish(0),
            Event::Key(k) => match k.code {
                0x48 => chosen = chosen.saturating_sub(1),
                0x50 => chosen += 1,
                0x13 => { status.clear(); refresh(window, &mut m, &mut net); }
                0x1C => if let Some((id, _, _)) = list.get(chosen) {
                    let id = id.clone();
                    install(window, &mut m, &mut net, &mut store, &id, &mut status);
                },
                _ => {}
            },
            _ => {}
        }
    }
}

fn refresh(window: &mut Window, m: &mut Manager, net: &mut WebNet) {
    window.show(b"Qupid Add-ons\n\nReading the repositories...");
    for r in m.refresh(net) {
        match &r.addons {
            Ok(n) => sex_pdx::serial_record!("[qupid.kodi.refresh] repository={} addons={}", m.sources[r.source].repository, n),
            Err(e) => sex_pdx::serial_record!("[qupid.kodi.refresh.failed] repository={} {:?}", m.sources[r.source].repository, e),
        }
    }
    sex_pdx::serial_record!("[qupid.kodi.catalogue] addons={} plugins={}", m.catalogue.entries.len(), m.catalogue.plugins(content::VIDEO | content::AUDIO).len());
}

fn install(window: &mut Window, m: &mut Manager, net: &mut WebNet, store: &mut OwnFolder, id: &str, status: &mut String) {
    window.show(format!("Qupid Add-ons\n\nInstalling {id}...").as_bytes());
    match m.install(net, store, id) {
        Ok(done) => {
            let names: Vec<String> = done.iter().map(|(i, v)| format!("{i} {v}")).collect();
            sex_pdx::serial_record!("[qupid.kodi.installed] id={} addons={} list={}", id, done.len(), names.join(","));
            *status = if done.is_empty() { format!("{id} is already installed.") } else { format!("Installed {}.", names.join(", ")) };
        }
        Err(e) => { sex_pdx::serial_record!("[qupid.kodi.failed] {:?}", e); *status = why(&e); }
    }
}
