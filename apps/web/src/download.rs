// SPDX-License-Identifier: GPL-3.0-only
//! Downloads. A page load whose answer is not a page (`Content-Disposition:
//! attachment`, or a type the browser doesn't show) goes to `Downloads/` in Files as
//! it arrives, with its progress in the bar, instead of into memory. `about:downloads`
//! lists them; each opens from there.

use super::{net, state_file, Browser, Pending};
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::files;
use sex_web::download_history::{self, Record};
use sex_web::download_pause::Transfer;

const DIR: &str = "Downloads";
/// Largest download.
const DOWNLOAD_MAX: usize = 4 << 30;

/// Types the browser shows itself; anything else is saved.
fn shown(content_type: &str) -> bool {
    let t = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    t.is_empty() || t.starts_with("text/") || t.starts_with("image/") || t.ends_with("+xml") || t.ends_with("/xml")
        || matches!(t.as_str(), "application/xhtml+xml" | "application/json" | "application/javascript")
}

/// Whether an answer with these headers is a download.
pub fn is_download(head: &sex_fetch::Head) -> bool {
    head.status < 300 && (head.get("content-disposition").is_some_and(|d| d.trim().to_ascii_lowercase().starts_with("attachment")) || !shown(head.get("content-type").unwrap_or("")))
}

/// A safe file name for the download: the header's `filename`, else the address's
/// last part, else `download`.
pub fn file_name(head: &sex_fetch::Head, url: &str) -> String {
    let from_header = head.get("content-disposition").and_then(|d| {
        d.split(';').filter_map(|p| p.trim().strip_prefix("filename=")).next().map(|f| String::from(f.trim().trim_matches('"')))
    });
    let from_url = || {
        let path = sex_html::url::fragment(url).0.split('?').next().unwrap_or("");
        path.rsplit('/').next().map(String::from)
    };
    let raw = from_header.or_else(from_url).unwrap_or_default();
    let mut name: String = raw.chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '\t') { '_' } else { c }).collect();
    name = String::from(name.trim_start_matches('.').trim());
    if name.is_empty() { name = String::from("download"); }
    name.chars().take(120).collect()
}

use sex_web::sex_html;

/// A finished download: its file and size.
pub use sex_web::download_history::Record as Done;
/// A failed transfer retained only for this browser window, with an explicit retry.
pub struct Failure { pub name: String, pub url: String, pub why: String }

enum Control { Continue, TogglePause, Cancel }

impl Browser {
    /// Escape stops a transfer; Ctrl+Shift+J pauses or resumes the live flow.
    /// Close reaches the normal loop after cleanup; other input is held in order.
    fn download_control(&mut self) -> Control {
        let Some(event) = self.window.poll() else { return Control::Continue };
        match event {
            sex_app::Event::Key(key) if key.code == 0x01 && !key.control => Control::Cancel,
            sex_app::Event::Key(key) if key.code == 0x24 && key.control && key.shift => Control::TogglePause,
            sex_app::Event::Close => {
                self.download_events.clear();
                self.download_events.push(sex_app::Event::Close);
                Control::Cancel
            }
            sex_app::Event::Pointer(p) if matches!(p.kind, sex_pdx::app_pointer::Kind::Move) => Control::Continue,
            other @ (sex_app::Event::Key(_) | sex_app::Event::Pointer(_)) if self.download_events.len() < 32 => {
                self.download_events.push(other);
                Control::Continue
            }
            _ => Control::Continue,
        }
    }

    /// Load the person's finished transfers before the first page opens. A private
    /// window never calls this, and private-tab records are excluded from saves.
    pub(super) fn load_downloads(&mut self) {
        const PATH: &str = "Kaleidoscope/downloads.tsv";
        let loaded = state_file::read(&mut self.window, PATH, download_history::MAX_FILE)
            .and_then(|s| download_history::decode(&s).ok())
            .or_else(|| state_file::read(&mut self.window, "Kaleidoscope/downloads.tsv.previous", download_history::MAX_FILE)
                .and_then(|s| download_history::decode(&s).ok()))
            .unwrap_or_default();
        sex_pdx::serial_record!("[web.download.history.loaded] count={}", loaded.len());
        self.downloads = loaded;
    }

    fn keep_downloads(&mut self) {
        let _ = files::mkdir(&mut self.window, b"Kaleidoscope");
        let text = download_history::encode(&self.downloads);
        let kept = state_file::replace(&mut self.window, "Kaleidoscope/downloads.tsv", text.as_bytes());
        sex_pdx::serial_record!("[web.download.history.saved] count={} kept={}", self.downloads.iter().filter(|d| !d.private).count(), kept as u8);
    }

    /// Loads `url` for showing as a page: like `fetch`, but an answer that is a download
    /// streams to Files instead (then `Ok(None)`).
    pub(super) fn fetch_or_download(&mut self, url: &str) -> Result<Option<(String, core::ops::Range<usize>)>, &'static [u8]> {
        let mut p = Pending::new(0, "GET", url, &[], &[], DOWNLOAD_MAX, None);
        loop {
            let mut ex = self.send(&mut p, true)?;
            // Wait for the head.
            let head = loop {
                let done = ex.poll(&mut self.window).map_err(net::message)?;
                if let Some(h) = ex.head() { break h; }
                if done { return Err(net::message(net::Error::Failed)); }
                if ex.idle() > 64 { super::pause() } else if ex.idle() > 0 { sex_pdx::sys_yield() }
            };
            if head.is_redirect() || !is_download(&head) {
                // A page (or a hop): read it whole, as before, within the page limit.
                while !ex.poll(&mut self.window).map_err(net::message)? {
                    if ex.received() > super::SOURCE_MAX + sex_fetch::HEAD_MAX + (super::SOURCE_MAX >> 3) + 4096 { ex.cancel(&mut self.window); return Err(net::message(net::Error::TooLarge)); }
                    if ex.idle() > 64 { super::pause() } else if ex.idle() > 0 { sex_pdx::sys_yield() }
                }
                let r = ex.finish().map_err(net::message)?;
                match self.landed(&mut p, r) {
                    None => continue,
                    Some(Ok(l)) => {
                        if l.status >= 400 { return Err(b"The site answered with an error."); }
                        self.buf = l.body;
                        return Ok(Some((l.url, 0..self.buf.len())));
                    }
                    Some(Err(e)) => return Err(e),
                }
            }
            // A download: its cookies count, then the body goes to a file.
            if let Ok(t) = sex_fetch::Url::parse(&p.url) {
                let mut jar = self.jar.borrow_mut();
                for c in head.all("set-cookie") { jar.store(&t, c, super::wall()); }
            }
            let name = file_name(&head, &p.url);
            self.save_stream(ex, &name, &p.url)?;
            return Ok(None);
        }
    }

    /// Streams the rest of `ex` into `Downloads/NAME` (a free name), showing progress.
    /// The bytes go to `Downloads/.NAME.part` and take the real name only once whole, so
    /// nothing (the Organizer, Loom, another program) ever sees half a download as a file.
    fn save_stream(&mut self, mut ex: net::Exchange, name: &str, url: &str) -> Result<(), &'static [u8]> {
        let _ = files::mkdir(&mut self.window, DIR.as_bytes());
        let path = self.free_name(name);
        let part = alloc::format!("{DIR}/.{}.part", &path[DIR.len() + 1..]);
        let _ = files::remove(&mut self.window, part.as_bytes());
        if files::write(&mut self.window, part.as_bytes(), &[]).is_err() {
            ex.cancel(&mut self.window);
            let _ = files::remove(&mut self.window, part.as_bytes());
            self.failed_download(name, url, b"Can't save in Downloads.");
            return Err(b"Can't save in Downloads.");
        }
        sex_pdx::serial_record!("[web.download.start] path={} url={}", path, url);
        let mut cancelled = false;
        let result = (|| -> Result<(u64, u64), &'static [u8]> {
        let began = sex_pdx::get_ticks();
        let mut transfer = Transfer::new(DOWNLOAD_MAX as u64);
        let mut total: Option<u64> = None;
        let mut chunk: Vec<u8> = Vec::new();
        let mut shown_at = 0u64;
        let mut streamed = false;
        loop {
            match self.download_control() {
                Control::Cancel => {
                    ex.cancel(&mut self.window);
                    cancelled = true;
                    return Err(b"Download cancelled.");
                }
                Control::TogglePause => {
                    let paused = transfer.toggle();
                    if !paused { ex.resume_after_pause(); }
                    self.download_status = Some(if paused {
                        alloc::format!("Paused at {} KB  {}  ·  Ctrl+Shift+J resumes  ·  Esc cancels", transfer.written() >> 10, name)
                    } else {
                        alloc::format!("Resuming {} at {} KB", name, transfer.written() >> 10)
                    });
                    self.draw();
                    sex_pdx::serial_record!("[web.download.pause] paused={} bytes={}", paused as u8, transfer.written());
                }
                Control::Continue => {}
            }
            if transfer.paused() { let _ = sex_pdx::sys_wait_self_ticks(16); continue; }
            let done = match ex.poll(&mut self.window) { Ok(d) => d, Err(e) => return Err(net::message(e)) };
            if let Some(t) = ex.stream(&mut chunk) {
                streamed = true;
                total = t;
                if !chunk.is_empty() {
                    let Some(offset) = transfer.write_offset(chunk.len()) else { ex.cancel(&mut self.window); return Err(b"Download exceeds its size limit."); };
                    if files::write_at(&mut self.window, part.as_bytes(), offset, &chunk).map_or(true, |n| n < chunk.len()) {
                        ex.cancel(&mut self.window);
                        return Err(b"Files is full: the download stopped.");
                    }
                    transfer.committed(chunk.len());
                    chunk.clear();
                }
            }
            let now = sex_pdx::get_ticks();
            if now >= shown_at + 250 {
                shown_at = now;
                self.download_status = Some(match total { Some(t) if t > 0 => alloc::format!("Downloading {}%  {}  ·  Ctrl+Shift+J pauses  ·  Esc cancels", transfer.written() * 100 / t, name), _ => alloc::format!("Downloading {} KB  {}  ·  Ctrl+Shift+J pauses  ·  Esc cancels", transfer.written() >> 10, name) });
                self.draw();
                sex_pdx::serial_record!("[web.download.progress] bytes={} total={}", transfer.written(), total.map_or(-1, |t| t as i64));
            }
            if done { break; }
            if ex.idle() > 64 { super::pause() } else if ex.idle() > 0 { sex_pdx::sys_yield() }
        }
        if !streamed {
            // Compressed or chunked: the body comes whole.
            let r = ex.finish().map_err(net::message)?;
            if transfer.write_offset(r.body.len()).is_none() { return Err(b"Download exceeds its size limit."); }
            if files::write(&mut self.window, part.as_bytes(), &r.body).is_err() { return Err(b"Files is full: the download stopped."); }
            transfer.committed(r.body.len());
        }
        if total.is_some_and(|expected| transfer.written() != expected) {
            return Err(b"Download ended before all bytes arrived.");
        }
        let check = self.check_file(&part, transfer.written());
        if files::rename(&mut self.window, part.as_bytes(), path.as_bytes()).is_err() { return Err(b"Can't save in Downloads."); }
        let _ = files::sync(&mut self.window);
        sex_pdx::serial_record!("[web.download.done] path={} bytes={} ticks={} fnv={:016x}", path, transfer.written(), sex_pdx::get_ticks() - began, check);
        Ok((transfer.written(), check))
        })();
        self.download_status = None;
        match result {
            Ok((wrote, _)) => {
                self.download_failures.retain(|f| f.url != url);
                let private = self.jarbox == super::profile::Jarbox::Private;
                self.downloads.push(Record { path, bytes: wrote, url: String::from(url), private });
                if self.downloads.len() > download_history::MAX_RECORDS { self.downloads.remove(0); }
                if !private { self.keep_downloads(); }
                Ok(())
            }
            Err(why) => {
                let _ = files::remove(&mut self.window, part.as_bytes());
                if cancelled {
                    let clean = files::size(&mut self.window, part.as_bytes()).is_err();
                    sex_pdx::serial_record!("[web.download.cancelled] path={} cleaned={}", path, clean as u8);
                    self.failed_download(name, url, why);
                    return Ok(());
                }
                self.failed_download(name, url, why);
                Err(why)
            }
        }
    }

    fn failed_download(&mut self, name: &str, url: &str, why: &'static [u8]) {
        self.download_status = None;
        let why = core::str::from_utf8(why).unwrap_or("Download stopped");
        sex_pdx::serial_record!("[web.download.failed] name={} reason={}", name, why);
        if self.download_failures.len() == 32 { self.download_failures.remove(0); }
        self.download_failures.push(Failure {
            name: name.chars().take(120).collect(),
            url: if url.len() <= 2048 && (url.starts_with("https://") || url.starts_with("http://")) { String::from(url) } else { String::new() },
            why: why.chars().take(120).collect(),
        });
    }

    /// Reads the saved file back and gives its FNV-1a 64 hash, so what landed on disk
    /// can be checked against what was sent (files up to 64 MB; 0 for larger ones).
    fn check_file(&mut self, path: &str, size: u64) -> u64 {
        if size > 64 << 20 { return 0; }
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut buf = alloc::vec![0u8; 1 << 20];
        let mut at = 0u64;
        while at < size {
            let n = match files::read_at(&mut self.window, path.as_bytes(), at, &mut buf) { Ok(0) | Err(_) => break, Ok(n) => n };
            for b in &buf[..n] { h ^= *b as u64; h = h.wrapping_mul(0x0100_0000_01b3); }
            at += n as u64;
        }
        if at != size { return 0; }
        h
    }

    /// `Downloads/name`, or `name (2)` and on when taken.
    fn free_name(&mut self, name: &str) -> String {
        let (stem, ext) = match name.rfind('.') { Some(i) if i > 0 => (&name[..i], &name[i..]), _ => (name, "") };
        for n in 1..1000 {
            let p = if n == 1 { alloc::format!("{DIR}/{name}") } else { alloc::format!("{DIR}/{stem} ({n}){ext}") };
            if files::size(&mut self.window, p.as_bytes()).is_err() { return p; }
        }
        alloc::format!("{DIR}/{stem}-{}{ext}", sex_pdx::get_ticks())
    }

    /// The `about:downloads` page: this run's downloads, then what else is in the folder.
    pub(super) fn downloads_page(&mut self) -> String {
        sex_pdx::serial_record!("[web.download.panel] done={} failed={}", self.downloads.len(), self.download_failures.len());
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
        let mut html = String::from("<title>Downloads</title><body style='margin:24px;font-size:15px'><h2>Downloads</h2>");
        if self.downloads.is_empty() { html.push_str("<p>No completed downloads in history yet.</p>"); }
        html.push_str("<ul>");
        let mut here = Vec::with_capacity(self.downloads.len());
        for d in self.downloads.iter() { here.push(files::size(&mut self.window, d.path.as_bytes()).is_ok()); }
        for (d, there) in self.downloads.iter().zip(here).rev() {
            let name = esc(d.path.trim_start_matches("Downloads/"));
            // Files' Organizer may have placed it in the person's folders (Documents,
            // Pictures...), outside this program's own: say so rather than link nowhere.
            if there {
                html.push_str(&alloc::format!("<li><a href=\"@{}\">{}</a> — {} KB <span style='color:#5f6368'>from {}</span></li>", esc(&d.path), name, d.bytes >> 10, esc(&d.url)));
            } else {
                html.push_str(&alloc::format!("<li>{} — {} KB <span style='color:#5f6368'>from {}; moved out of Downloads (Files' organize log says where)</span></li>", name, d.bytes >> 10, esc(&d.url)));
            }
        }
        html.push_str("</ul>");
        if !self.download_failures.is_empty() {
            html.push_str("<h3>Needs attention</h3><ul>");
            for f in self.download_failures.iter().rev() {
                if f.url.is_empty() {
                    html.push_str(&alloc::format!("<li>{} — {}</li>", esc(&f.name), esc(&f.why)));
                } else {
                    html.push_str(&alloc::format!("<li>{} — {} <a href=\"{}\">Try again</a></li>", esc(&f.name), esc(&f.why), esc(&f.url)));
                }
            }
            html.push_str("</ul>");
        }
        if let Ok(list) = files::list(&mut self.window, DIR.as_bytes()) {
            if list.iter().any(|e| !e.name.starts_with(b".")) {
                html.push_str("<h3>In Downloads</h3><ul>");
                // Hidden names are downloads still arriving (`.NAME.part`).
                for e in list.iter().filter(|e| !e.name.starts_with(b".")) {
                    let n = String::from_utf8_lossy(&e.name).into_owned();
                    html.push_str(&alloc::format!("<li><a href=\"@{DIR}/{}\">{}</a></li>", esc(&n), esc(&n)));
                }
                html.push_str("</ul>");
            }
        }
        html
    }
}
