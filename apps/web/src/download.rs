// SPDX-License-Identifier: GPL-3.0-only
//! Downloads. A page load whose answer is not a page (`Content-Disposition:
//! attachment`, or a type the browser doesn't show) goes to `Downloads/` in Files as
//! it arrives, with its progress in the bar, instead of into memory. `about:downloads`
//! lists them; each opens from there.

use super::{net, state_file, Browser, Pending};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::files;
use sex_web::download_history::{self, Record};
use sex_web::download_pause::Transfer;
use sex_web::download_promotion::{self, Step};
use sex_web::download_resume::{self, Record as Resume};

const DIR: &str = "Downloads";
/// Largest download.
const DOWNLOAD_MAX: usize = 4 << 30;
fn part_name(path: &str) -> String { alloc::format!("{DIR}/.{}.part", &path[DIR.len() + 1..]) }
fn part_offset(window: &mut sex_app::Window, r: &Resume) -> Option<u64> {
    let part = part_name(&r.path);
    let n = files::size(window, part.as_bytes()).ok()?;
    if n == 0 || n >= r.total { return None; }
    let mut byte = [0];
    if files::read_at(window, part.as_bytes(), 0, &mut byte).ok() != Some(1)
        || files::read_at(window, part.as_bytes(), n - 1, &mut byte).ok() != Some(1) { return None; }
    Some(n)
}

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

enum Control { Continue, TogglePause, Cancel, Close }

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
                Control::Close
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
        self.download_resumes = state_file::read(&mut self.window, "Kaleidoscope/resumable.tsv", download_resume::MAX_FILE)
            .and_then(|s| download_resume::decode(&s).ok()).unwrap_or_default();
        self.download_resumes.retain(|r| part_offset(&mut self.window, r).is_some());
        sex_pdx::serial_record!("[web.download.resume.loaded] count={}", self.download_resumes.len());
    }

    fn keep_resumes(&mut self) -> bool {
        let _ = files::mkdir(&mut self.window, b"Kaleidoscope");
        state_file::replace(&mut self.window, "Kaleidoscope/resumable.tsv", download_resume::encode(&self.download_resumes).as_bytes())
    }

    fn drop_resume(&mut self, url: &str) {
        if !self.download_resumes.iter().any(|r| r.url == url) { return; }
        let old: Vec<_> = self.download_resumes.iter().filter(|r| r.url == url).map(|r| part_name(&r.path)).collect();
        self.download_resumes.retain(|r| r.url != url);
        for part in old { let _ = files::remove(&mut self.window, part.as_bytes()); }
        let _ = self.keep_resumes();
    }

    fn keep_downloads(&mut self) {
        let _ = files::mkdir(&mut self.window, b"Kaleidoscope");
        let text = download_history::encode(&self.downloads);
        let kept = state_file::replace(&mut self.window, "Kaleidoscope/downloads.tsv", text.as_bytes());
        sex_pdx::serial_record!("[web.download.history.saved] count={} kept={}", self.downloads.iter().filter(|d| !d.private).count(), kept as u8);
    }

    /// Clear the list, not the person's downloaded files or resumable transfers.
    pub(super) fn clear_download_history(&mut self) {
        if self.url != "about:downloads" { return; }
        let old = self.downloads.len();
        let replaced = state_file::replace(&mut self.window, "Kaleidoscope/downloads.tsv", b"v1\n");
        // The normal recovery copy contains the old list. A deliberate clear
        // must retire it too, or a later damaged live file could restore it.
        let previous = b"Kaleidoscope/downloads.tsv.previous";
        let retired = replaced && (files::size(&mut self.window, previous).is_err()
            || files::remove(&mut self.window, previous).is_ok() && files::sync(&mut self.window).is_ok());
        if retired {
            self.downloads.clear();
            self.download_failures.clear();
            sex_pdx::serial_record!("[web.download.history.cleared] count={} kept=1", old);
            self.open("about:downloads", false);
        } else {
            sex_pdx::serial_record!("[web.download.history.cleared] count={} kept=0", old);
            self.download_status = Some(String::from("Could not clear download history; try again."));
            self.draw();
        }
    }

    /// Loads `url` for showing as a page: like `fetch`, but an answer that is a download
    /// streams to Files instead (then `Ok(None)`).
    pub(super) fn fetch_or_download(&mut self, url: &str) -> Result<Option<(String, core::ops::Range<usize>)>, &'static [u8]> {
        let mut resume = if self.jarbox == super::profile::Jarbox::Private { None }
            else { self.download_resumes.iter().find(|r| r.url == url).cloned() };
        'request: loop {
            let range = resume.as_ref().and_then(|r| part_offset(&mut self.window, r).map(|n| alloc::format!("bytes={n}-")));
            if resume.is_some() && range.is_none() { self.drop_resume(url); resume = None; }
            let headers: Vec<(&str, &str)> = match (&resume, &range) {
                (Some(r), Some(range)) => vec![("Range", range.as_str()), ("If-Range", r.etag.as_str())],
                _ => Vec::new(),
            };
            let mut p = Pending::new(0, "GET", url, &headers, &[], DOWNLOAD_MAX, None);
            loop {
            let mut ex = self.send(&mut p, true)?;
            // Wait for the head.
            let head = loop {
                let done = ex.poll(&mut self.window).map_err(net::message)?;
                if let Some(h) = ex.head() { break h; }
                if done { return Err(net::message(net::Error::Failed)); }
                if ex.idle() > 64 { super::pause() } else if ex.idle() > 0 { sex_pdx::sys_yield() }
            };
            if let Some(r) = &resume {
                let offset = range.as_ref().and_then(|s| s.strip_prefix("bytes=")).and_then(|s| s.strip_suffix('-')).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                if head.status == 206 && download_resume::appendable(&head, r, offset) && p.url == r.url {
                    if let Ok(t) = sex_fetch::Url::parse(&p.url) {
                        let mut jar = self.jar.borrow_mut();
                        for c in head.all("set-cookie") { jar.store(&t, c, super::wall()); }
                    }
                    sex_pdx::serial_record!("[web.download.resume] path={} offset={} total={}", r.path, offset, r.total);
                    let name = file_name(&head, &p.url);
                    self.save_stream(ex, &head, &name, &p.url, Some(r.clone()))?;
                    return Ok(None);
                }
                self.drop_resume(url);
                resume = None;
                if head.status != 200 || p.url != url {
                    ex.cancel(&mut self.window);
                    sex_pdx::serial_record!("[web.download.resume.restart] reason=range_response");
                    continue 'request;
                }
                sex_pdx::serial_record!("[web.download.resume.restart] reason=full_response");
            }
            if head.status == 206 { ex.cancel(&mut self.window); return Err(b"The server sent an unexpected partial response."); }
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
            self.save_stream(ex, &head, &name, &p.url, None)?;
            return Ok(None);
            }
        }
    }

    /// Streams the rest of `ex` into `Downloads/NAME` (a free name), showing progress.
    /// The bytes go to `Downloads/.NAME.part` and take the real name only once whole, so
    /// nothing (the Organizer, Loom, another program) ever sees half a download as a file.
    fn save_stream(&mut self, mut ex: net::Exchange, head: &sex_fetch::Head, name: &str, url: &str, resume: Option<Resume>) -> Result<(), &'static [u8]> {
        let _ = files::mkdir(&mut self.window, DIR.as_bytes());
        let path = resume.as_ref().map(|r| r.path.clone()).unwrap_or_else(|| self.free_name(name));
        let part = part_name(&path);
        if resume.is_none() { let _ = files::remove(&mut self.window, part.as_bytes()); }
        if resume.is_none() && files::write(&mut self.window, part.as_bytes(), &[]).is_err() {
            ex.cancel(&mut self.window);
            let _ = files::remove(&mut self.window, part.as_bytes());
            self.failed_download(name, url, b"Can't save in Downloads.");
            return Err(b"Can't save in Downloads.");
        }
        let start = resume.as_ref().and_then(|_| files::size(&mut self.window, part.as_bytes()).ok()).unwrap_or(0);
        let resumable = if self.jarbox == super::profile::Jarbox::Private { None }
            else { resume.or_else(|| download_resume::eligible(head, url, &path)) };
        sex_pdx::serial_record!("[web.download.start] path={} url={} offset={}", path, url, start);
        let mut cancelled = false;
        let mut interrupted = false;
        let mut written = start;
        let result = (|| -> Result<(u64, u64), &'static [u8]> {
        let began = sex_pdx::get_ticks();
        let mut transfer = Transfer::from_offset(DOWNLOAD_MAX as u64, start).ok_or(b"Download exceeds its size limit." as &'static [u8])?;
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
                Control::Close => {
                    ex.cancel(&mut self.window);
                    interrupted = true;
                    return Err(b"Download interrupted.");
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
            let done = match ex.poll(&mut self.window) { Ok(d) => d, Err(e) => { interrupted = true; return Err(net::message(e)); } };
            if let Some(t) = ex.stream(&mut chunk) {
                streamed = true;
                total = resumable.as_ref().map(|r| r.total).or(t);
                if !chunk.is_empty() {
                    let Some(offset) = transfer.write_offset(chunk.len()) else { ex.cancel(&mut self.window); return Err(b"Download exceeds its size limit."); };
                    if files::write_at(&mut self.window, part.as_bytes(), offset, &chunk).map_or(true, |n| n < chunk.len()) {
                        ex.cancel(&mut self.window);
                        return Err(b"Files is full: the download stopped.");
                    }
                    transfer.committed(chunk.len());
                    written = transfer.written();
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
            interrupted = true;
            return Err(b"Download ended before all bytes arrived.");
        }
        let check = self.check_file(&part, transfer.written());
        let promoted = download_promotion::promote(|step| {
            let result = match step {
                Step::SyncPart | Step::SyncName => files::sync(&mut self.window),
                Step::Rename => files::rename(&mut self.window, part.as_bytes(), path.as_bytes()),
            };
            if result.is_ok() {
                let stage = match step { Step::SyncPart => "part", Step::Rename => "rename", Step::SyncName => "name" };
                sex_pdx::serial_record!("[web.download.promote] step={} path={}", stage, path);
            }
            result
        });
        match promoted {
            Ok(()) => {}
            Err((Step::SyncPart, _)) => return Err(b"Could not make the download bytes durable."),
            Err((Step::Rename, _)) => return Err(b"Can't save in Downloads."),
            Err((Step::SyncName, _)) => return Err(b"The file may be in Downloads, but its save could not be confirmed."),
        }
        if files::size(&mut self.window, path.as_bytes()).ok() != Some(transfer.written())
            || (transfer.written() <= 64 << 20 && (check == 0 || self.check_file(&path, transfer.written()) != check)) {
            return Err(b"The saved download did not match its staged bytes.");
        }
        sex_pdx::serial_record!("[web.download.done] path={} bytes={} ticks={} fnv={:016x}", path, transfer.written(), sex_pdx::get_ticks() - began, check);
        Ok((transfer.written(), check))
        })();
        self.download_status = None;
        match result {
            Ok((wrote, _)) => {
                if resumable.is_some() { self.drop_resume(url); }
                self.download_failures.retain(|f| f.url != url);
                let private = self.jarbox == super::profile::Jarbox::Private;
                self.downloads.push(Record { path, bytes: wrote, url: String::from(url), private });
                if self.downloads.len() > download_history::MAX_RECORDS { self.downloads.remove(0); }
                if !private { self.keep_downloads(); }
                Ok(())
            }
            Err(why) => {
                let kept = !cancelled && interrupted && written > 0 && resumable.as_ref().is_some_and(|r| written < r.total)
                    && files::sync(&mut self.window).is_ok() && {
                        self.download_resumes.retain(|r| r.url != url);
                        self.download_resumes.push(resumable.clone().unwrap());
                        if self.download_resumes.len() > download_resume::MAX_RECORDS { self.download_resumes.remove(0); }
                        self.keep_resumes()
                    };
                if !kept { let _ = files::remove(&mut self.window, part.as_bytes()); self.drop_resume(url); }
                else { sex_pdx::serial_record!("[web.download.resume.kept] path={} bytes={}", path, written); }
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
            if files::size(&mut self.window, p.as_bytes()).is_err()
                && files::size(&mut self.window, part_name(&p).as_bytes()).is_err() { return p; }
        }
        alloc::format!("{DIR}/{stem}-{}{ext}", sex_pdx::get_ticks())
    }

    /// The `about:downloads` page: this run's downloads, then what else is in the folder.
    pub(super) fn downloads_page(&mut self) -> String {
        sex_pdx::serial_record!("[web.download.panel] done={} failed={}", self.downloads.len(), self.download_failures.len());
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
        let mut html = String::from("<title>Downloads</title><body style='margin:24px;font-size:15px'><h2>Downloads</h2><p>Ctrl+Alt+J clears this history; downloaded files and resumable transfers stay.</p>");
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
        if self.jarbox != super::profile::Jarbox::Private && !self.download_resumes.is_empty() {
            html.push_str("<h3>Resume downloads</h3><ul>");
            for r in &self.download_resumes {
                html.push_str(&alloc::format!("<li>{} — <a href=\"{}\">Resume verified download</a></li>", esc(&r.path), esc(&r.url)));
            }
            html.push_str("</ul>");
        }
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
