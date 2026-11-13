// SPDX-License-Identifier: GPL-3.0-only
//! HTTP/1.1 for SexOS programs, without any I/O: addresses, requests, reply heads,
//! body framing (Content-Length, chunked, until close), content decoding (gzip,
//! deflate) and a cookie jar. `sex_app::net::request` moves the bytes; this crate
//! decides what they mean, so all of it is tested on the host.
//!
//! What a hostile server can do here is bounded: heads are capped at [`HEAD_MAX`],
//! decoded bodies at the caller's limit, and cookies by count and size.
#![no_std]

extern crate alloc;

pub mod cookie;
pub mod pieces;
pub mod script;
pub mod ws;
pub mod woff;
mod date;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
pub use cookie::Jar;
pub use date::parse_http_date;

/// The largest reply head we read.
pub const HEAD_MAX: usize = 64 * 1024;
/// The user agent we send unless a request names its own.
pub const USER_AGENT: &str = "Mozilla/5.0 (X11; SexOS x86_64) AppleWebKit/537.36 (KHTML, like Gecko) SexOSWeb/0.2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The address can't be read, or isn't http(s).
    Address,
    /// A header name or value would break the request (CR, LF, NUL, or not a token).
    Header,
    /// The reply isn't HTTP we can read.
    Malformed,
    /// The reply head is larger than [`HEAD_MAX`].
    HeadTooLarge,
    /// The body is larger than the caller allowed.
    TooLarge,
    /// The body ended before its framing said it would.
    Truncated,
    /// A content encoding we did not ask for (brotli, zstd).
    Encoding,
    /// The compressed body is damaged.
    Corrupt,
}

/// An absolute http or https address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`; never the fragment.
    pub target: String,
}

impl Url {
    pub fn parse(url: &str) -> Result<Url, Error> {
        let (secure, rest) = if let Some(r) = strip_prefix_ci(url, "https://") { (true, r) } else if let Some(r) = strip_prefix_ci(url, "http://") { (false, r) } else { return Err(Error::Address) };
        let rest = rest.split('#').next().unwrap_or("");
        let end = rest.find(|c| c == '/' || c == '?').unwrap_or(rest.len());
        let (authority, target) = rest.split_at(end);
        // Credentials in addresses are not sent anywhere.
        let authority = authority.rsplit('@').next().unwrap_or("");
        let (host, port) = match authority.rfind(':') {
            Some(i) if !authority.ends_with(']') => {
                let p = &authority[i + 1..];
                let port = if p.is_empty() { None } else { Some(p.parse::<u16>().map_err(|_| Error::Address)?) };
                (&authority[..i], port)
            }
            _ => (authority, None),
        };
        // A name, an IPv4 address, or an IPv6 address in brackets (kept, as the Host header wants).
        let literal6 = host.len() > 2 && host.starts_with('[') && host.ends_with(']') && host[1..host.len() - 1].bytes().all(|b| b.is_ascii_hexdigit() || b":.".contains(&b));
        if host.is_empty() || host.len() > 253 || !literal6 && !host.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b)) { return Err(Error::Address); }
        let port = port.unwrap_or(if secure { 443 } else { 80 });
        if port == 0 { return Err(Error::Address); }
        let mut target = if target.is_empty() { String::from("/") } else if target.starts_with('?') { alloc::format!("/{}", target) } else { String::from(target) };
        // Spaces and controls are percent-encoded rather than sent raw.
        if target.bytes().any(|b| b <= b' ' || b == 0x7f) {
            let mut t = String::new();
            for b in target.bytes() { if b <= b' ' || b == 0x7f { t.push_str(&alloc::format!("%{:02X}", b)); } else { t.push(b as char); } }
            target = t;
        }
        Ok(Url { secure, host: host.to_ascii_lowercase(), port, target })
    }
    fn default_port(&self) -> bool { self.port == if self.secure { 443 } else { 80 } }
    /// The Host header's value.
    pub fn authority(&self) -> String { if self.default_port() { self.host.clone() } else { alloc::format!("{}:{}", self.host, self.port) } }
    pub fn path(&self) -> &str { self.target.split('?').next().unwrap_or("/") }
    pub fn origin(&self) -> String { alloc::format!("{}://{}", if self.secure { "https" } else { "http" }, self.authority()) }
}

/// The `Referer` a request to `to` carries from the page at `from`, under the
/// default policy (`strict-origin-when-cross-origin`): the page's address without
/// its fragment within its own origin, only its origin elsewhere, and nothing from
/// an https page to an http address or from a page that isn't http(s). The address
/// is the page's real one: the browser never makes one up.
pub fn referrer(from: &str, to: &Url) -> Option<String> {
    let page = Url::parse(from).ok()?;
    if page.secure && !to.secure { return None; }
    if page.origin() == to.origin() { Some(alloc::format!("{}{}", page.origin(), page.target)) } else { Some(alloc::format!("{}/", page.origin())) }
}

fn strip_prefix_ci<'a>(s: &'a str, p: &str) -> Option<&'a str> {
    (s.len() >= p.len() && s.as_bytes()[..p.len()].eq_ignore_ascii_case(p.as_bytes())).then(|| &s[p.len()..])
}

/// One request. Headers the request names replace the defaults of the same name.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
    /// The `Origin` among `headers` is the browser's own, not a page's.
    pub trusted_origin: bool,
    /// The browser's `Referer`: the page this request is for, as [`referrer`] trims it.
    pub referrer: Option<&'a str>,
}
impl<'a> Request<'a> {
    pub const fn get(url: &'a str) -> Self { Request { method: "GET", url, headers: &[], body: &[], trusted_origin: false, referrer: None } }
}

fn token(s: &str) -> bool { !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)) }
fn value_ok(s: &str) -> bool { !s.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) }

/// Headers a page's script may not set (the browser owns them).
pub fn forbidden_header(name: &str) -> bool {
    const NAMES: &[&str] = &["host", "connection", "content-length", "transfer-encoding", "te", "trailer", "upgrade", "keep-alive", "cookie", "cookie2", "accept-encoding", "expect", "date", "via", "proxy-authorization", "origin", "referer"];
    let n = name.to_ascii_lowercase();
    NAMES.contains(&n.as_str()) || n.starts_with("proxy-") || n.starts_with("sec-")
}

/// The request's bytes: HTTP/1.1, which keeps the connection for further requests
/// unless the site says otherwise (see `Head::keeps_connection`).
/// `cookie` is the jar's `Cookie` value for this address, if any.
pub fn encode(req: &Request, url: &Url, cookie: Option<&str>) -> Result<Vec<u8>, Error> {
    let method = if req.method.is_empty() { "GET" } else { req.method };
    if !token(method) { return Err(Error::Header); }
    let mut out = Vec::with_capacity(256 + req.body.len());
    let mut put = |s: &str| out.extend_from_slice(s.as_bytes());
    put(&method.to_ascii_uppercase()); put(" "); put(&url.target); put(" HTTP/1.1\r\n");
    put("Host: "); put(&url.authority()); put("\r\n");
    let named = |n: &str| req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(n));
    if !named("user-agent") { put("User-Agent: "); put(USER_AGENT); put("\r\n"); }
    if !named("accept") { put("Accept: */*\r\n"); }
    if let Some(r) = req.referrer.filter(|r| value_ok(r)) { put("Referer: "); put(r); put("\r\n"); }
    if !named("accept-language") { put("Accept-Language: en-US,en;q=0.8\r\n"); }
    put("Accept-Encoding: gzip, deflate\r\n");
    for (k, v) in req.headers {
        if !token(k) || !value_ok(v) { return Err(Error::Header); }
        // The browser's own Origin goes through; a page's Origin or Host never does.
        if forbidden_header(k) && !(k.eq_ignore_ascii_case("origin") && req.trusted_origin) { continue; }
        put(k); put(": "); put(v.trim()); put("\r\n");
    }
    if let Some(c) = cookie.filter(|c| !c.is_empty()) {
        if !value_ok(c) { return Err(Error::Header); }
        put("Cookie: "); put(c); put("\r\n");
    }
    let m = method.to_ascii_uppercase();
    if !req.body.is_empty() || matches!(m.as_str(), "POST" | "PUT" | "PATCH") { put(&alloc::format!("Content-Length: {}\r\n", req.body.len())); }
    put("\r\n");
    out.extend_from_slice(req.body);
    Ok(out)
}

/// A reply's status line and headers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Head {
    pub status: u16,
    pub reason: String,
    /// Names lowercased, values trimmed, in arrival order.
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub fn get(&self, name: &str) -> Option<&str> { self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()) }
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a { self.headers.iter().filter(move |(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()) }
    pub fn content_type(&self) -> &str { self.get("content-type").unwrap_or("") }
    pub fn is_redirect(&self) -> bool { matches!(self.status, 301 | 302 | 303 | 307 | 308) && self.get("location").is_some() }
    fn chunked(&self) -> bool { self.all("transfer-encoding").any(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case("chunked"))) }
    /// How the body is delimited, for a reply to a request with `method`.
    pub fn framing(&self, method: &str) -> Result<Framing, Error> {
        if method.eq_ignore_ascii_case("HEAD") || (100..200).contains(&self.status) || self.status == 204 || self.status == 304 { return Ok(Framing::Empty); }
        if self.chunked() { return Ok(Framing::Chunked); }
        let mut lengths = self.all("content-length");
        match lengths.next() {
            None => Ok(Framing::UntilClose),
            Some(first) => {
                let n: u64 = first.parse().map_err(|_| Error::Malformed)?;
                // Two different lengths is how request smuggling starts: refuse.
                if lengths.any(|v| v.parse::<u64>().ok() != Some(n)) { return Err(Error::Malformed); }
                Ok(Framing::Length(n))
            }
        }
    }
    /// The headers as `name: value` lines, for scripts (`getAllResponseHeaders`).
    pub fn lines(&self, skip_cookies: bool) -> String {
        let mut s = String::new();
        for (k, v) in &self.headers {
            if skip_cookies && (k == "set-cookie" || k == "set-cookie2") { continue; }
            s.push_str(k); s.push_str(": "); s.push_str(v); s.push_str("\r\n");
        }
        s
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing { Empty, Length(u64), Chunked, UntilClose }

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> { hay.windows(needle.len()).position(|w| w == needle) }

/// Reads the head at the start of `bytes`. `Ok(None)` while it is incomplete; on
/// success, the head and where the body starts. Interim 1xx replies are skipped.
impl Head {
    /// The site keeps the connection open after this reply (HTTP/1.1 without
    /// `Connection: close`). `http10` is whether the status line said HTTP/1.0.
    pub fn keeps_connection(&self, http10: bool) -> bool {
        let says = |word: &str| self.all("connection").any(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(word)));
        if http10 { says("keep-alive") } else { !says("close") }
    }
}

pub fn parse_head(bytes: &[u8]) -> Result<Option<(Head, usize)>, Error> {
    let mut at = 0;
    loop {
        let rest = &bytes[at..];
        let Some(end) = find(&rest[..rest.len().min(HEAD_MAX + 4)], b"\r\n\r\n") else {
            return if rest.len() > HEAD_MAX { Err(Error::HeadTooLarge) } else { Ok(None) };
        };
        let text = core::str::from_utf8(&rest[..end]).map_err(|_| Error::Malformed)?;
        let mut lines = text.split("\r\n");
        let status_line = lines.next().ok_or(Error::Malformed)?;
        let mut parts = status_line.splitn(3, ' ');
        let version = parts.next().unwrap_or("");
        if !version.starts_with("HTTP/1.") { return Err(Error::Malformed); }
        let status: u16 = parts.next().and_then(|s| s.parse().ok()).filter(|s| (100..1000).contains(s)).ok_or(Error::Malformed)?;
        let reason = parts.next().unwrap_or("").to_string();
        let mut headers = Vec::new();
        for line in lines {
            if line.starts_with(' ') || line.starts_with('\t') { return Err(Error::Malformed); }
            let colon = line.find(':').ok_or(Error::Malformed)?;
            let name = &line[..colon];
            if !token(name) { return Err(Error::Malformed); }
            headers.push((name.to_ascii_lowercase(), line[colon + 1..].trim().to_string()));
        }
        at += end + 4;
        if (100..200).contains(&status) && status != 101 { continue; }
        return Ok(Some((Head { status, reason, headers }, at)));
    }
}

/// Where a chunked body ends in `raw` (just past the last chunk and its trailers),
/// or `None` while more bytes are needed.
pub fn chunked_end(raw: &[u8]) -> Result<Option<usize>, Error> { walk_chunks(raw, None, usize::MAX) }

/// Finds the end of a chunked body as it arrives, without walking the chunks already
/// seen again (a long download stays linear).
#[derive(Clone, Copy, Debug, Default)]
pub struct ChunkScan { at: usize }
impl ChunkScan {
    pub const fn new() -> Self { ChunkScan { at: 0 } }
    /// `raw` is the whole body so far (it only grows between calls). `Some(end)` once
    /// the last chunk and trailers are in.
    pub fn advance(&mut self, raw: &[u8]) -> Result<Option<usize>, Error> {
        loop {
            let rest = &raw[self.at..];
            let Some(nl) = find(rest, b"\r\n") else { return if rest.len() > 1024 { Err(Error::Malformed) } else { Ok(None) } };
            let line = core::str::from_utf8(&rest[..nl]).map_err(|_| Error::Malformed)?;
            let hex = line.split(';').next().unwrap_or("").trim();
            if hex.is_empty() || hex.len() > 15 { return Err(Error::Malformed); }
            let size = usize::from_str_radix(hex, 16).map_err(|_| Error::Malformed)?;
            if size == 0 { return Ok(walk_chunks(rest, None, usize::MAX)?.map(|e| self.at + e)); }
            let next = nl + 2 + size + 2;
            if rest.len() < next { return Ok(None); }
            if &rest[next - 2..next] != b"\r\n" { return Err(Error::Malformed); }
            self.at += next;
        }
    }
}

/// Walks a chunked body, copying the data into `out` when given.
fn walk_chunks(raw: &[u8], mut out: Option<&mut Vec<u8>>, max: usize) -> Result<Option<usize>, Error> {
    let mut at = 0usize;
    let mut total = 0usize;
    loop {
        let Some(nl) = find(&raw[at..], b"\r\n") else { return if raw.len() - at > 1024 { Err(Error::Malformed) } else { Ok(None) } };
        let line = core::str::from_utf8(&raw[at..at + nl]).map_err(|_| Error::Malformed)?;
        let hex = line.split(';').next().unwrap_or("").trim();
        if hex.is_empty() || hex.len() > 15 { return Err(Error::Malformed); }
        let size = usize::from_str_radix(hex, 16).map_err(|_| Error::Malformed)?;
        at += nl + 2;
        if size == 0 {
            // Trailers until an empty line.
            loop {
                let Some(nl) = find(&raw[at..], b"\r\n") else { return Ok(None) };
                at += nl + 2;
                if nl == 0 { return Ok(Some(at)); }
            }
        }
        total = total.checked_add(size).ok_or(Error::TooLarge)?;
        if total > max { return Err(Error::TooLarge); }
        if raw.len() < at + size + 2 { return Ok(None); }
        if &raw[at + size..at + size + 2] != b"\r\n" { return Err(Error::Malformed); }
        if let Some(o) = out.as_deref_mut() { o.extend_from_slice(&raw[at..at + size]); }
        at += size + 2;
    }
}

/// Whether `raw` (the bytes after the head) holds the whole body. A reply framed by
/// the connection closing is complete only when the caller saw the close.
pub fn complete(framing: Framing, raw: &[u8]) -> Result<bool, Error> {
    Ok(match framing {
        Framing::Empty => true,
        Framing::Length(n) => raw.len() as u64 >= n,
        Framing::Chunked => chunked_end(raw)?.is_some(),
        Framing::UntilClose => false,
    })
}

/// The body as the page should see it: unframed and decompressed, at most `max`
/// bytes. `closed` says the connection ended (so an until-close body is whole).
pub fn decode(head: &Head, method: &str, raw: &[u8], max: usize) -> Result<Vec<u8>, Error> {
    let framed = match head.framing(method)? {
        // HEAD and 304 replies may name the encoding of a body they don't carry.
        Framing::Empty => return Ok(Vec::new()),
        Framing::Length(n) => {
            if (raw.len() as u64) < n { return Err(Error::Truncated); }
            if n > max as u64 && head.get("content-encoding").is_none() { return Err(Error::TooLarge); }
            Vec::from(&raw[..n as usize])
        }
        Framing::Chunked => {
            let mut out = Vec::new();
            if walk_chunks(raw, Some(&mut out), max.saturating_mul(4).max(max))?.is_none() { return Err(Error::Truncated); }
            out
        }
        Framing::UntilClose => Vec::from(raw),
    };
    let mut body = framed;
    // Encodings apply in the order listed, so undo them last-first.
    let encodings: Vec<String> = head.all("content-encoding").flat_map(|v| v.split(',')).map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty() && t != "identity").collect();
    for e in encodings.iter().rev() {
        body = inflate(e, &body, max)?;
    }
    if body.len() > max { return Err(Error::TooLarge); }
    Ok(body)
}

fn inflate(encoding: &str, data: &[u8], max: usize) -> Result<Vec<u8>, Error> {
    use zune_inflate::{errors::DecodeErrorStatus, DeflateDecoder, DeflateOptions};
    let options = || DeflateOptions::default().set_limit(max).set_size_hint(data.len().saturating_mul(4).min(max));
    let map = |e: zune_inflate::errors::InflateDecodeErrors| match e.error { DecodeErrorStatus::OutputLimitExceeded(..) => Error::TooLarge, _ => Error::Corrupt };
    match encoding {
        "gzip" | "x-gzip" => DeflateDecoder::new_with_options(data, options()).decode_gzip().map_err(map),
        // "deflate" is meant to be zlib-wrapped, but some servers send it raw.
        "deflate" => DeflateDecoder::new_with_options(data, options()).decode_zlib().or_else(|_| DeflateDecoder::new_with_options(data, options()).decode_deflate()).map_err(map),
        _ => Err(Error::Encoding),
    }
}

/// `base` joined with a relative reference, the way a `Location` header or a link is
/// read. Returns `None` for addresses that aren't http(s).
pub fn join(base: &str, reference: &str) -> Option<String> {
    let r = reference.trim();
    if strip_prefix_ci(r, "http://").is_some() || strip_prefix_ci(r, "https://").is_some() { return Some(String::from(r)); }
    let b = Url::parse(base).ok()?;
    let scheme = if b.secure { "https:" } else { "http:" };
    if let Some(rest) = r.strip_prefix("//") { return Some(alloc::format!("{}//{}", scheme, rest)); }
    if r.contains(':') && r.find(':') < r.find(|c| c == '/' || c == '?' || c == '#') .or(Some(r.len())) { return None; }
    let origin = b.origin();
    if r.starts_with('/') { return Some(alloc::format!("{}{}", origin, r)); }
    if r.is_empty() { return Some(alloc::format!("{}{}", origin, b.target)); }
    if r.starts_with('?') { return Some(alloc::format!("{}{}{}", origin, b.path(), r)); }
    if r.starts_with('#') { return Some(alloc::format!("{}{}{}", origin, b.target, r)); }
    let dir = &b.path()[..b.path().rfind('/').map_or(0, |i| i + 1)];
    // Remove dot segments.
    let joined = alloc::format!("{}{}", dir, r);
    let (path, tail) = match joined.find(|c| c == '?' || c == '#') { Some(i) => joined.split_at(i), None => (joined.as_str(), "") };
    let mut segs: Vec<&str> = Vec::new();
    let parts: Vec<&str> = path.split('/').collect();
    for (i, s) in parts.iter().enumerate() {
        match *s {
            "." => { if i == parts.len() - 1 { segs.push(""); } }
            ".." => { if segs.len() > 1 { segs.pop(); } if i == parts.len() - 1 { segs.push(""); } }
            _ => segs.push(s),
        }
    }
    let mut p = segs.join("/");
    if !p.starts_with('/') { p.insert(0, '/'); }
    Some(alloc::format!("{}{}{}", origin, p, tail))
}

#[cfg(test)]
mod tests;
