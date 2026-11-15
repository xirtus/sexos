// SPDX-License-Identifier: GPL-3.0-only
//! Web seeds (BEP 19, "GetRight style"): plain HTTP(S) servers holding the torrent's
//! files, listed in the torrent's `url-list`.
//!
//! A piece maps to byte ranges of one or more files. For each range this module builds an
//! HTTP `Range` request (the program sends it with its own HTTP client), checks the
//! response (206 with a matching `Content-Range`, or a 200 carrying the whole file, from
//! which the range is cut), follows redirects per file (at most [`MAX_REDIRECTS`] hops),
//! and decides when a seed is rested (exponential back-off, `Retry-After`) or banned
//! (too many failures, or data that failed the piece hash).
//!
//! URL rules (BEP 19): for a single-file torrent, a URL ending in `/` gets the torrent
//! name appended, otherwise it names the file itself; for a multi-file torrent the URL is
//! a directory to which `name/path/to/file` is appended. Every path component is
//! percent-encoded.
//!
//! The piece-to-file mapping comes from the caller as `(path, file_offset, len)` triples,
//! where `path` is the torrent `name` followed by the file's path components (just
//! `[name]` for a single-file torrent).
//!
//! BEP 17 (Hoffman-style "httpseeds", a server script answering `?info_hash=&piece=`) is
//! obsolete and deliberately not supported.
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ops::Range;

use crate::tracker::{parse_url, url_encode, HttpTarget, Scheme, TrackerUrl};
use crate::Millis;

/// Redirect hops followed per file.
pub const MAX_REDIRECTS: u8 = 5;
/// Consecutive failures after which a seed is banned.
pub const MAX_FAILURES: u32 = 10;
/// Pieces failing their hash after which a seed is banned (it serves wrong data).
pub const MAX_HASH_FAILURES: u32 = 3;
/// Back-off: 30 s doubling per consecutive failure, at most an hour.
pub const BACKOFF_BASE: Millis = 30_000;
pub const BACKOFF_MAX: Millis = 3_600_000;
/// A server's `Retry-After` is honoured up to a day.
pub const RETRY_AFTER_MAX: Millis = 86_400_000;
/// Ranges in one piece request (a piece spanning more files than this is refused).
pub const MAX_RANGES: usize = 1024;

/// One HTTP request for part of a piece.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RangeRequest {
    /// Index into the slices the request was built from.
    pub slice: usize,
    /// The file's full URL (after any redirect remembered for it).
    pub url: String,
    pub target: HttpTarget,
    /// Byte range in the file, inclusive (as HTTP writes it).
    pub start: u64,
    pub end: u64,
    /// Where these bytes go in the piece.
    pub piece_offset: u64,
}

impl RangeRequest {
    /// The `Range:` header value.
    pub fn range_header(&self) -> String { format!("bytes={}-{}", self.start, self.end) }
    /// Bytes expected.
    pub fn len(&self) -> u64 { self.end - self.start + 1 }
    /// Never true (a request has at least one byte); for clippy's sake.
    pub fn is_empty(&self) -> bool { false }
}

/// Why a request could not be built or a response not used.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Failure {
    /// The seed's URL (or a redirect) is unusable.
    BadUrl,
    /// 404 / 410: the server does not have this file.
    NotFound,
    /// 429 / 503 and friends: try later (`Retry-After` seconds when given).
    Busy(Option<u32>),
    /// Wrong status, a range that does not match, a short body, too many redirects.
    Bad,
}

/// What a response means.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// These bytes of the body are the requested range.
    Data(Range<usize>),
    /// Follow this absolute URL (call [`WebSeed::on_redirect`] then rebuild requests).
    Redirect(String),
    Failed(Failure),
}

/// Joins a web seed URL and a file path per BEP 19.
pub fn file_url(base: &str, multi_file: bool, path: &[String]) -> Option<String> {
    let u = parse_url(base)?;
    if u.scheme == Scheme::Udp || path.is_empty() || path.len() > 256 { return None; }
    if path.iter().any(|c| c.is_empty() || c == "." || c == "..") { return None; }
    let (p, q) = match u.path_and_query.find('?') { Some(i) => u.path_and_query.split_at(i), None => (u.path_and_query.as_str(), "") };
    let mut p = p.to_string();
    if multi_file {
        if !p.ends_with('/') { p.push('/'); }
        let enc: Vec<String> = path.iter().map(|c| url_encode(c.as_bytes())).collect();
        p.push_str(&enc.join("/"));
    } else if p.ends_with('/') {
        p.push_str(&url_encode(path.last()?.as_bytes()));
    }
    let scheme = if u.scheme == Scheme::Https { "https" } else { "http" };
    Some(format!("{}://{}:{}{}{}", scheme, u.host_for_header(), u.port, p, q))
}

fn target(u: &TrackerUrl) -> HttpTarget {
    let default = if u.tls() { 443 } else { 80 };
    let host_header = if u.port == default { u.host_for_header() } else { format!("{}:{}", u.host_for_header(), u.port) };
    HttpTarget { host: u.host.clone(), port: u.port, tls: u.tls(), path_and_query: u.path_and_query.clone(), host_header }
}

/// Resolves a `Location` header against the URL that was requested.
pub fn resolve_location(base: &str, location: &str) -> Option<String> {
    let location = location.trim();
    if location.is_empty() || location.len() > crate::tracker::MAX_URL { return None; }
    let b = parse_url(base)?;
    let scheme = if b.tls() { "https" } else { "http" };
    let abs = if location.contains("://") {
        location.to_string()
    } else if let Some(rest) = location.strip_prefix("//") {
        format!("{}://{}", scheme, rest)
    } else if location.starts_with('/') {
        format!("{}://{}:{}{}", scheme, b.host_for_header(), b.port, location)
    } else {
        let path = b.path_and_query.split('?').next().unwrap_or("/");
        let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
        format!("{}://{}:{}{}{}", scheme, b.host_for_header(), b.port, if dir.is_empty() { "/" } else { dir }, location)
    };
    let u = parse_url(&abs)?;
    if u.scheme == Scheme::Udp { return None; }
    Some(abs)
}

/// Parses `bytes a-b/total` (total may be `*`).
pub fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let v = v.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = v.split_once('/')?;
    let (a, b) = range.trim().split_once('-')?;
    let num = |s: &str| -> Option<u64> { let s = s.trim(); if s.is_empty() || s.len() > 19 || !s.bytes().all(|c| c.is_ascii_digit()) { None } else { s.parse().ok() } };
    let (a, b) = (num(a)?, num(b)?);
    if a > b { return None; }
    let total = if total.trim() == "*" { None } else { Some(num(total)?) };
    if matches!(total, Some(t) if b >= t) { return None; }
    Some((a, b, total))
}

/// Judges an HTTP response to `req`: status, the `Content-Range`, `Location` and
/// `Retry-After` headers (when present), and how many body bytes arrived.
pub fn check_response(req: &RangeRequest, status: u16, content_range: Option<&str>, location: Option<&str>, retry_after: Option<&str>, body_len: usize) -> Outcome {
    let want = req.len();
    match status {
        206 => {
            let ok = match content_range.map(parse_content_range) {
                Some(Some((a, b, _))) => a == req.start && b == req.end,
                Some(None) => false,
                None => true,
            };
            if ok && body_len as u64 == want { Outcome::Data(0..body_len) } else { Outcome::Failed(Failure::Bad) }
        }
        200 => {
            // The server ignored Range and sent the whole file: cut our part out.
            let (Ok(s), Ok(e)) = (usize::try_from(req.start), usize::try_from(req.end)) else { return Outcome::Failed(Failure::Bad) };
            if e < body_len { Outcome::Data(s..e + 1) } else { Outcome::Failed(Failure::Bad) }
        }
        301 | 302 | 303 | 307 | 308 => match location.and_then(|l| resolve_location(&req.url, l)) {
            Some(u) => Outcome::Redirect(u),
            None => Outcome::Failed(Failure::Bad),
        },
        404 | 410 => Outcome::Failed(Failure::NotFound),
        408 | 429 | 500 | 502 | 503 | 504 => {
            let secs = retry_after.and_then(|r| { let r = r.trim(); if r.len() <= 9 && !r.is_empty() && r.bytes().all(|c| c.is_ascii_digit()) { r.parse().ok() } else { None } });
            Outcome::Failed(Failure::Busy(secs))
        }
        _ => Outcome::Failed(Failure::Bad),
    }
}

/// One web seed and its health.
#[derive(Clone, Debug)]
pub struct WebSeed {
    pub url: String,
    /// Per-file redirects: path → (url, hops).
    redirects: BTreeMap<Vec<String>, (String, u8)>,
    /// Files the server said it does not have.
    missing: BTreeSet<Vec<String>>,
    pub fails: u32,
    pub hash_fails: u32,
    pub retry_at: Millis,
    pub banned: bool,
    /// Bytes received in good responses.
    pub downloaded: u64,
}

impl WebSeed {
    /// `None` if the URL is not http(s).
    pub fn new(url: &str) -> Option<Self> {
        let u = parse_url(url)?;
        if u.scheme == Scheme::Udp { return None; }
        Some(WebSeed { url: url.trim().to_string(), redirects: BTreeMap::new(), missing: BTreeSet::new(), fails: 0, hash_fails: 0, retry_at: 0, banned: false, downloaded: 0 })
    }

    /// Whether to send requests now.
    pub fn usable(&self, now: Millis) -> bool { !self.banned && now >= self.retry_at }

    /// Whether the seed may have this file (it has not answered 404 for it).
    pub fn has_file(&self, path: &[String]) -> bool { !self.missing.contains(path) }

    /// The URL for a file, after redirects.
    pub fn url_for(&self, multi_file: bool, path: &[String]) -> Option<String> {
        match self.redirects.get(path) { Some((u, _)) => Some(u.clone()), None => file_url(&self.url, multi_file, path) }
    }

    /// The requests for one piece: `slices` are `(path, file_offset, len)` in piece order.
    /// Zero-length slices are skipped (they still advance nothing).
    pub fn requests(&self, multi_file: bool, slices: &[(&[String], u64, u64)]) -> Result<Vec<RangeRequest>, Failure> {
        if slices.len() > MAX_RANGES { return Err(Failure::Bad); }
        let mut out = Vec::new();
        let mut piece_offset: u64 = 0;
        for (i, &(path, off, len)) in slices.iter().enumerate() {
            if len == 0 { continue; }
            if !self.has_file(path) { return Err(Failure::NotFound); }
            let url = self.url_for(multi_file, path).ok_or(Failure::BadUrl)?;
            let u = parse_url(&url).ok_or(Failure::BadUrl)?;
            let end = off.checked_add(len - 1).ok_or(Failure::Bad)?;
            out.push(RangeRequest { slice: i, target: target(&u), url, start: off, end, piece_offset });
            piece_offset = piece_offset.checked_add(len).ok_or(Failure::Bad)?;
        }
        Ok(out)
    }

    /// Remember a redirect for the file `req` asked for. `false` (and a failure counted)
    /// if the chain got too long.
    pub fn on_redirect(&mut self, path: &[String], new_url: &str, now: Millis) -> bool {
        let hops = self.redirects.get(path).map_or(0, |r| r.1) + 1;
        if hops > MAX_REDIRECTS || parse_url(new_url).map_or(true, |u| u.scheme == Scheme::Udp) {
            self.on_failure(path, Failure::Bad, now);
            return false;
        }
        if self.redirects.len() >= 4096 && !self.redirects.contains_key(path) { return false; }
        self.redirects.insert(path.to_vec(), (new_url.to_string(), hops));
        true
    }

    /// A range arrived intact.
    pub fn on_data(&mut self, bytes: u64) {
        self.fails = 0;
        self.downloaded = self.downloaded.saturating_add(bytes);
    }

    /// A request failed. 404 marks the file missing (a single-file seed without its only
    /// file is banned by the caller's check of [`has_file`](Self::has_file)); everything
    /// else backs off, and too many in a row ban the seed.
    pub fn on_failure(&mut self, path: &[String], f: Failure, now: Millis) {
        if f == Failure::NotFound && self.missing.len() < 65_536 {
            self.missing.insert(path.to_vec());
            self.redirects.remove(path);
        }
        self.fails = self.fails.saturating_add(1);
        let mut wait = (BACKOFF_BASE << (self.fails - 1).min(7)).min(BACKOFF_MAX);
        if let Failure::Busy(Some(s)) = f { wait = wait.max((s as Millis * 1000).min(RETRY_AFTER_MAX)); }
        self.retry_at = now.saturating_add(wait);
        if self.fails >= MAX_FAILURES || f == Failure::BadUrl { self.banned = true; }
    }

    /// A piece that came (partly) from this seed failed its hash check.
    pub fn on_hash_fail(&mut self, now: Millis) {
        self.hash_fails += 1;
        self.retry_at = now.saturating_add(BACKOFF_BASE);
        if self.hash_fails >= MAX_HASH_FAILURES { self.banned = true; }
    }

    /// Lifts a ban and forgets failures (user action).
    pub fn unban(&mut self) {
        self.banned = false;
        self.fails = 0;
        self.hash_fails = 0;
        self.retry_at = 0;
        self.missing.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn p(parts: &[&str]) -> Vec<String> { parts.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn url_joining() {
        assert_eq!(file_url("http://s/f.iso", false, &p(&["f.iso"])).unwrap(), "http://s:80/f.iso");
        assert_eq!(file_url("http://s/dir/", false, &p(&["my file.iso"])).unwrap(), "http://s:80/dir/my%20file.iso");
        assert_eq!(file_url("https://s/pub", true, &p(&["Name", "a b", "c#?.txt"])).unwrap(), "https://s:443/pub/Name/a%20b/c%23%3F.txt");
        assert_eq!(file_url("http://[::1]:8080/x/?k=1", true, &p(&["n", "f"])).unwrap(), "http://[::1]:8080/x/n/f?k=1");
        assert_eq!(file_url("http://s/", true, &p(&["n", ".."])), None);
        assert_eq!(file_url("udp://s:1/", true, &p(&["n"])), None);
        assert_eq!(file_url("http://s/", true, &[]), None);
    }

    #[test]
    fn piece_spanning_files() {
        let s = WebSeed::new("http://seed.example/files/").unwrap();
        let a = p(&["T", "a.bin"]);
        let b = p(&["T", "sub", "b.bin"]);
        let z = p(&["T", "empty"]);
        let r = s.requests(true, &[(&a, 900, 100), (&z, 0, 0), (&b, 0, 924)]).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].start, r[0].end, r[0].piece_offset, r[0].slice), (900, 999, 0, 0));
        assert_eq!(r[0].range_header(), "bytes=900-999");
        assert_eq!(r[0].target.path_and_query, "/files/T/a.bin");
        assert_eq!(r[0].target.host_header, "seed.example");
        assert_eq!((r[1].start, r[1].end, r[1].piece_offset, r[1].slice), (0, 923, 100, 2));
        assert_eq!(r[1].target.path_and_query, "/files/T/sub/b.bin");
        assert_eq!(s.requests(true, &[(&a, u64::MAX, 2)]), Err(Failure::Bad));
    }

    #[test]
    fn responses() {
        let s = WebSeed::new("http://h/f").unwrap();
        let f = p(&["f"]);
        let r = &s.requests(false, &[(&f, 10, 5)]).unwrap()[0];
        assert_eq!(check_response(r, 206, Some("bytes 10-14/100"), None, None, 5), Outcome::Data(0..5));
        assert_eq!(check_response(r, 206, None, None, None, 5), Outcome::Data(0..5));
        assert_eq!(check_response(r, 206, Some("bytes 11-15/100"), None, None, 5), Outcome::Failed(Failure::Bad));
        assert_eq!(check_response(r, 206, Some("bytes 10-14/100"), None, None, 4), Outcome::Failed(Failure::Bad));
        assert_eq!(check_response(r, 206, Some("garbage"), None, None, 5), Outcome::Failed(Failure::Bad));
        assert_eq!(check_response(r, 200, None, None, None, 100), Outcome::Data(10..15));
        assert_eq!(check_response(r, 200, None, None, None, 14), Outcome::Failed(Failure::Bad));
        assert_eq!(check_response(r, 302, None, Some("/mirror/f"), None, 0), Outcome::Redirect("http://h:80/mirror/f".into()));
        assert_eq!(check_response(r, 301, None, Some("other"), None, 0), Outcome::Redirect("http://h:80/other".into()));
        assert_eq!(check_response(r, 307, None, Some("https://m.example/f"), None, 0), Outcome::Redirect("https://m.example/f".into()));
        assert_eq!(check_response(r, 302, None, Some("udp://x:1/"), None, 0), Outcome::Failed(Failure::Bad));
        assert_eq!(check_response(r, 302, None, None, None, 0), Outcome::Failed(Failure::Bad));
        assert_eq!(check_response(r, 404, None, None, None, 0), Outcome::Failed(Failure::NotFound));
        assert_eq!(check_response(r, 503, None, None, Some("120"), 0), Outcome::Failed(Failure::Busy(Some(120))));
        assert_eq!(check_response(r, 503, None, None, Some("Wed, 21 Oct"), 0), Outcome::Failed(Failure::Busy(None)));
        assert_eq!(check_response(r, 418, None, None, None, 0), Outcome::Failed(Failure::Bad));
    }

    #[test]
    fn content_range_parsing() {
        assert_eq!(parse_content_range("bytes 0-0/1"), Some((0, 0, Some(1))));
        assert_eq!(parse_content_range("bytes 5-9/*"), Some((5, 9, None)));
        for bad in ["bytes 9-5/10", "bytes 0-10/10", "bytes -1-2/3", "bytes 0-99999999999999999999/1", "items 0-1/2", "bytes 0-1", ""] {
            assert_eq!(parse_content_range(bad), None, "{}", bad);
        }
    }

    #[test]
    fn redirects_backoff_and_bans() {
        let mut s = WebSeed::new("http://h/d/").unwrap();
        let f = p(&["T", "x"]);
        assert!(s.on_redirect(&f, "http://m/x", 0));
        assert_eq!(s.requests(true, &[(&f, 0, 1)]).unwrap()[0].url, "http://m/x");
        for i in 2..=MAX_REDIRECTS { assert!(s.on_redirect(&f, "http://m/x", 0), "{}", i); }
        assert!(!s.on_redirect(&f, "http://m/x", 0), "loop is cut");
        assert_eq!(s.fails, 1);
        assert!(!s.usable(29_999));
        assert!(s.usable(30_000));
        s.on_data(10);
        assert_eq!(s.fails, 0);
        s.on_failure(&f, Failure::Busy(Some(600)), 0);
        assert_eq!(s.retry_at, 600_000);
        s.on_failure(&f, Failure::NotFound, 0);
        assert!(!s.has_file(&f));
        assert_eq!(s.requests(true, &[(&f, 0, 1)]), Err(Failure::NotFound));
        for _ in 0..MAX_FAILURES { s.on_failure(&f, Failure::Bad, 0); }
        assert!(s.banned);
        s.unban();
        assert!(s.usable(0) && s.has_file(&f));
        for _ in 0..MAX_HASH_FAILURES { s.on_hash_fail(0); }
        assert!(s.banned);
        assert!(WebSeed::new("udp://x:1/").is_none());
        assert!(WebSeed::new("not a url").is_none());
    }

    #[test]
    fn hostile_locations() {
        let long = vec![b'a'; 5000];
        let long = core::str::from_utf8(&long).unwrap();
        assert_eq!(resolve_location("http://h/", long), None);
        assert_eq!(resolve_location("http://h/", "http://"), None);
        assert_eq!(resolve_location("http://h/", "//evil host/"), None);
        assert_eq!(resolve_location("http://h/a/b", "c").as_deref(), Some("http://h:80/a/c"));
    }
}
