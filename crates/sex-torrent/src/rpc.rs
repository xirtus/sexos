// SPDX-License-Identifier: GPL-3.0-only
//! A Transmission-compatible RPC server, without I/O, so that existing remotes
//! (transmission-remote, Transmissionic, Tremotesf, Transmission's own web UI, phone
//! apps) can drive Swingers.
//!
//! The program reads bytes from its listening socket and passes them to
//! [`RpcServer::handle_http`], which parses the HTTP request (pipelining, `Content-Length`
//! and chunked bodies), and answers with the response bytes to write back. Along the way:
//! - Host check against a whitelist (DNS-rebinding protection; IP literals and
//!   `localhost` always pass), answered with 421 like Transmission.
//! - HTTP Basic authentication against the configured user and password, compared in
//!   constant time; 401 with `WWW-Authenticate` otherwise.
//! - The CSRF session-id protocol: a request to the RPC path without the right
//!   `X-Transmission-Session-Id` gets 409 carrying the id; clients retry with it.
//! - JSON requests `{method, arguments, tag}` and responses `{result, arguments, tag}`
//!   (spec: transmission/docs/rpc-spec.md, rpc-version 17). Argument keys are matched
//!   ignoring case, `-` and `_`, so kebab-case, camelCase and snake_case all work. A
//!   request carrying `"jsonrpc": "2.0"` (Transmission 4.1 style) is answered in kind,
//!   with snake_case keys, best effort.
//! - Other paths go to the embedded web UI ([`crate::webui`]).
//!
//! The program implements [`RpcBackend`]: it lists torrents as [`TorrentInfo`] structs
//! (the fields Transmission clients actually read) and carries out adds, actions and
//! settings; this module does all of the JSON. The [`json`] submodule is a small bounded
//! parser and encoder (depth limit, strict UTF-8, `\u` escapes with surrogate pairs).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// A small JSON value type, parser and encoder.
pub mod json {
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Nesting deeper than this is refused.
    pub const MAX_DEPTH: usize = 32;

    /// A JSON value. Objects keep their order (and duplicates; lookups take the first).
    #[derive(Clone, Debug, PartialEq)]
    pub enum Json { Null, Bool(bool), Num(f64), Str(String), Arr(Vec<Json>), Obj(Vec<(String, Json)>) }

    /// Why JSON was refused.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Error { Utf8, Syntax(usize), Depth, Trailing(usize) }

    impl Json {
        /// The first member with this exact key.
        pub fn get(&self, k: &str) -> Option<&Json> {
            if let Json::Obj(m) = self { m.iter().find(|(x, _)| x == k).map(|(_, v)| v) } else { None }
        }
        pub fn as_str(&self) -> Option<&str> { if let Json::Str(s) = self { Some(s) } else { None } }
        pub fn as_f64(&self) -> Option<f64> { if let Json::Num(n) = self { Some(*n) } else { None } }
        /// An integral number in `i64` range (and within 2^53, where f64 is exact).
        pub fn as_i64(&self) -> Option<i64> {
            let n = self.as_f64()?;
            (n >= -9.007e15 && n <= 9.007e15 && (n as i64) as f64 == n).then_some(n as i64)
        }
        pub fn as_u64(&self) -> Option<u64> { self.as_i64().and_then(|n| u64::try_from(n).ok()) }
        /// A boolean; `0`/`1` are accepted because some clients send them.
        pub fn as_bool(&self) -> Option<bool> {
            match self { Json::Bool(b) => Some(*b), Json::Num(n) if *n == 0.0 || *n == 1.0 => Some(*n == 1.0), _ => None }
        }
        pub fn as_arr(&self) -> Option<&[Json]> { if let Json::Arr(a) = self { Some(a) } else { None } }
        pub fn as_obj(&self) -> Option<&[(String, Json)]> { if let Json::Obj(o) = self { Some(o) } else { None } }
        /// Builds an object from `(&str, Json)` pairs.
        pub fn obj<const N: usize>(pairs: [(&str, Json); N]) -> Json {
            Json::Obj(pairs.into_iter().map(|(k, v)| (String::from(k), v)).collect())
        }
        /// Encodes compactly.
        pub fn encode(&self) -> String { let mut s = String::new(); self.write(&mut s); s }
        /// Appends the encoding to `out`.
        pub fn write(&self, out: &mut String) {
            match self {
                Json::Null => out.push_str("null"),
                Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
                Json::Num(n) => write_num(*n, out),
                Json::Str(s) => write_str(s, out),
                Json::Arr(a) => {
                    out.push('[');
                    for (i, v) in a.iter().enumerate() { if i > 0 { out.push(','); } v.write(out); }
                    out.push(']');
                }
                Json::Obj(o) => {
                    out.push('{');
                    for (i, (k, v)) in o.iter().enumerate() { if i > 0 { out.push(','); } write_str(k, out); out.push(':'); v.write(out); }
                    out.push('}');
                }
            }
        }
    }

    impl From<&str> for Json { fn from(s: &str) -> Json { Json::Str(String::from(s)) } }
    impl From<String> for Json { fn from(s: String) -> Json { Json::Str(s) } }
    impl From<bool> for Json { fn from(b: bool) -> Json { Json::Bool(b) } }
    impl From<f64> for Json { fn from(n: f64) -> Json { Json::Num(n) } }
    impl From<u64> for Json { fn from(n: u64) -> Json { Json::Num(n as f64) } }
    impl From<u32> for Json { fn from(n: u32) -> Json { Json::Num(n as f64) } }
    impl From<i64> for Json { fn from(n: i64) -> Json { Json::Num(n as f64) } }
    impl From<i32> for Json { fn from(n: i32) -> Json { Json::Num(n as f64) } }
    impl From<Vec<Json>> for Json { fn from(v: Vec<Json>) -> Json { Json::Arr(v) } }

    fn write_num(n: f64, out: &mut String) {
        use core::fmt::Write;
        if !n.is_finite() { out.push_str("null"); }
        else if n >= -9.007e15 && n <= 9.007e15 && (n as i64) as f64 == n { let _ = write!(out, "{}", n as i64); }
        else { let _ = write!(out, "{}", n); }
    }

    fn write_str(s: &str, out: &mut String) {
        use core::fmt::Write;
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""), '\\' => out.push_str("\\\\"), '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"), '\t' => out.push_str("\\t"), '\u{8}' => out.push_str("\\b"), '\u{c}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 => { let _ = write!(out, "\\u{:04x}", c as u32); }
                c => out.push(c),
            }
        }
        out.push('"');
    }

    struct P<'a> { b: &'a [u8], i: usize }

    impl<'a> P<'a> {
        fn ws(&mut self) { while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') { self.i += 1; } }
        fn err(&self) -> Error { Error::Syntax(self.i) }
        fn lit(&mut self, s: &[u8], v: Json) -> Result<Json, Error> {
            if self.b[self.i..].starts_with(s) { self.i += s.len(); Ok(v) } else { Err(self.err()) }
        }
        fn value(&mut self, depth: usize) -> Result<Json, Error> {
            if depth > MAX_DEPTH { return Err(Error::Depth); }
            self.ws();
            match *self.b.get(self.i).ok_or(self.err())? {
                b'n' => self.lit(b"null", Json::Null),
                b't' => self.lit(b"true", Json::Bool(true)),
                b'f' => self.lit(b"false", Json::Bool(false)),
                b'"' => self.string().map(Json::Str),
                b'[' => {
                    self.i += 1;
                    let mut a = Vec::new();
                    self.ws();
                    if self.b.get(self.i) == Some(&b']') { self.i += 1; return Ok(Json::Arr(a)); }
                    loop {
                        a.push(self.value(depth + 1)?);
                        self.ws();
                        match self.b.get(self.i) { Some(b',') => self.i += 1, Some(b']') => { self.i += 1; return Ok(Json::Arr(a)); } _ => return Err(self.err()) }
                    }
                }
                b'{' => {
                    self.i += 1;
                    let mut o = Vec::new();
                    self.ws();
                    if self.b.get(self.i) == Some(&b'}') { self.i += 1; return Ok(Json::Obj(o)); }
                    loop {
                        self.ws();
                        if self.b.get(self.i) != Some(&b'"') { return Err(self.err()); }
                        let k = self.string()?;
                        self.ws();
                        if self.b.get(self.i) != Some(&b':') { return Err(self.err()); }
                        self.i += 1;
                        let v = self.value(depth + 1)?;
                        o.push((k, v));
                        self.ws();
                        match self.b.get(self.i) { Some(b',') => self.i += 1, Some(b'}') => { self.i += 1; return Ok(Json::Obj(o)); } _ => return Err(self.err()) }
                    }
                }
                b'-' | b'0'..=b'9' => self.number(),
                _ => Err(self.err()),
            }
        }
        fn number(&mut self) -> Result<Json, Error> {
            let s = self.i;
            if self.b[self.i] == b'-' { self.i += 1; }
            let digits = |p: &mut Self| { let st = p.i; while p.i < p.b.len() && p.b[p.i].is_ascii_digit() { p.i += 1; } p.i - st };
            let int_start = self.i;
            let n = digits(self);
            if n == 0 || (n > 1 && self.b[int_start] == b'0') { return Err(Error::Syntax(int_start)); }
            if self.b.get(self.i) == Some(&b'.') { self.i += 1; if digits(self) == 0 { return Err(self.err()); } }
            if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
                self.i += 1;
                if matches!(self.b.get(self.i), Some(b'+' | b'-')) { self.i += 1; }
                if digits(self) == 0 { return Err(self.err()); }
            }
            let txt = core::str::from_utf8(&self.b[s..self.i]).map_err(|_| Error::Syntax(s))?;
            txt.parse::<f64>().map(Json::Num).map_err(|_| Error::Syntax(s))
        }
        fn hex4(&mut self) -> Result<u32, Error> {
            let h = self.b.get(self.i..self.i + 4).ok_or(self.err())?;
            let s = core::str::from_utf8(h).map_err(|_| self.err())?;
            let v = u32::from_str_radix(s, 16).map_err(|_| self.err())?;
            if !h.iter().all(|c| c.is_ascii_hexdigit()) { return Err(self.err()); }
            self.i += 4;
            Ok(v)
        }
        fn string(&mut self) -> Result<String, Error> {
            self.i += 1;
            let mut out = String::new();
            loop {
                let start = self.i;
                while self.i < self.b.len() && self.b[self.i] != b'"' && self.b[self.i] != b'\\' && self.b[self.i] >= 0x20 { self.i += 1; }
                // The whole input was checked as UTF-8 and we stop only at ASCII bytes.
                out.push_str(core::str::from_utf8(&self.b[start..self.i]).map_err(|_| Error::Utf8)?);
                match self.b.get(self.i) {
                    Some(b'"') => { self.i += 1; return Ok(out); }
                    Some(b'\\') => {
                        self.i += 1;
                        let c = *self.b.get(self.i).ok_or(self.err())?;
                        self.i += 1;
                        match c {
                            b'"' => out.push('"'), b'\\' => out.push('\\'), b'/' => out.push('/'), b'b' => out.push('\u{8}'),
                            b'f' => out.push('\u{c}'), b'n' => out.push('\n'), b'r' => out.push('\r'), b't' => out.push('\t'),
                            b'u' => {
                                let hi = self.hex4()?;
                                let cp = if (0xD800..0xDC00).contains(&hi) {
                                    if self.b[self.i..].starts_with(b"\\u") {
                                        let save = self.i;
                                        self.i += 2;
                                        let lo = self.hex4()?;
                                        if (0xDC00..0xE000).contains(&lo) { 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00) } else { self.i = save; 0xFFFD }
                                    } else { 0xFFFD }
                                } else if (0xDC00..0xE000).contains(&hi) { 0xFFFD } else { hi };
                                out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                            }
                            _ => return Err(Error::Syntax(self.i - 1)),
                        }
                    }
                    _ => return Err(self.err()),
                }
            }
        }
    }

    /// Parses one JSON document (UTF-8, optional BOM).
    pub fn parse(input: &[u8]) -> Result<Json, Error> {
        let input = input.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(input);
        core::str::from_utf8(input).map_err(|_| Error::Utf8)?;
        let mut p = P { b: input, i: 0 };
        let v = p.value(0)?;
        p.ws();
        if p.i != input.len() { return Err(Error::Trailing(p.i)); }
        Ok(v)
    }
}

pub use json::Json;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() { s.push(B64[(n >> (18 - 6 * k)) as usize & 63] as char); } else { s.push('='); }
        }
    }
    s
}

/// Decodes base64 (standard or URL alphabet); whitespace is skipped, padding optional.
pub fn base64_decode(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut pad = false;
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62, b'/' | b'_' => 63,
            b'=' => { pad = true; continue; }
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        if pad { return None; }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); acc &= (1 << bits) - 1; }
    }
    Some(out)
}

/// Compares secrets without an early exit on the first difference.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let n = a.len().max(b.len());
    let mut d = (a.len() ^ b.len()) as u64;
    for i in 0..n { d |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as u64; }
    core::hint::black_box(d) == 0
}

// ---------------------------------------------------------------------------------
// HTTP

/// Header section limit.
pub const MAX_HEAD: usize = 16 * 1024;
/// Body limit (a base64 .torrent can be large).
pub const MAX_BODY: usize = 32 << 20;
const MAX_HEADERS: usize = 64;

/// A parsed HTTP request.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Request {
    pub method: String,
    /// The path, without the query.
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// HTTP/1.0, or `Connection: close`.
    pub close: bool,
}

impl Request {
    /// A header value by name (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Why bytes are not (yet) a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpError { Incomplete, BadRequest, TooLarge }

fn find(b: &[u8], pat: &[u8]) -> Option<usize> { b.windows(pat.len()).position(|w| w == pat) }

/// Parses one request from the front of `buf`; returns it and the bytes it used.
pub fn parse_request(buf: &[u8]) -> Result<(Request, usize), HttpError> {
    let head_end = match find(&buf[..buf.len().min(MAX_HEAD + 4)], b"\r\n\r\n") {
        Some(p) => p,
        None => return Err(if buf.len() > MAX_HEAD { HttpError::TooLarge } else { HttpError::Incomplete }),
    };
    let head = core::str::from_utf8(&buf[..head_end]).map_err(|_| HttpError::BadRequest)?;
    let mut lines = head.split("\r\n");
    let mut rl = lines.next().ok_or(HttpError::BadRequest)?.split(' ');
    let (method, target, version) = (rl.next().unwrap_or(""), rl.next().unwrap_or(""), rl.next().unwrap_or(""));
    if method.is_empty() || !method.bytes().all(|c| c.is_ascii_uppercase()) || !target.starts_with('/') || !version.starts_with("HTTP/1.") || rl.next().is_some() {
        return Err(HttpError::BadRequest);
    }
    let mut req = Request { method: String::from(method), close: version == "HTTP/1.0", ..Request::default() };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    req.path = String::from(path);
    req.query = String::from(query);
    for line in lines {
        let (k, v) = line.split_once(':').ok_or(HttpError::BadRequest)?;
        if k.is_empty() || k.contains(' ') { return Err(HttpError::BadRequest); }
        if req.headers.len() >= MAX_HEADERS { return Err(HttpError::TooLarge); }
        req.headers.push((String::from(k), String::from(v.trim())));
    }
    match req.header("Connection").map(|c| c.to_ascii_lowercase()) {
        Some(c) if c == "close" => req.close = true,
        Some(c) if c == "keep-alive" => req.close = false,
        _ => {}
    }
    let mut at = head_end + 4;
    if req.header("Transfer-Encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
        loop {
            let rest = &buf[at..];
            let le = find(&rest[..rest.len().min(64)], b"\r\n").ok_or(if rest.len() >= 64 { HttpError::BadRequest } else { HttpError::Incomplete })?;
            let size_s = core::str::from_utf8(&rest[..le]).map_err(|_| HttpError::BadRequest)?;
            let size = usize::from_str_radix(size_s.split(';').next().unwrap_or("").trim(), 16).map_err(|_| HttpError::BadRequest)?;
            if size > MAX_BODY || req.body.len() + size > MAX_BODY { return Err(HttpError::TooLarge); }
            at += le + 2;
            if size == 0 {
                // Trailers until an empty line.
                loop {
                    let rest = &buf[at..];
                    let le = find(rest, b"\r\n").ok_or(if rest.len() > MAX_HEAD { HttpError::TooLarge } else { HttpError::Incomplete })?;
                    at += le + 2;
                    if le == 0 { break; }
                }
                break;
            }
            if buf.len() < at + size + 2 { return Err(HttpError::Incomplete); }
            req.body.extend_from_slice(&buf[at..at + size]);
            if &buf[at + size..at + size + 2] != b"\r\n" { return Err(HttpError::BadRequest); }
            at += size + 2;
        }
    } else if let Some(cl) = req.header("Content-Length") {
        let n: usize = cl.trim().parse().map_err(|_| HttpError::BadRequest)?;
        if n > MAX_BODY { return Err(HttpError::TooLarge); }
        if buf.len() < at + n { return Err(HttpError::Incomplete); }
        req.body = buf[at..at + n].to_vec();
        at += n;
    }
    Ok((req, at))
}

/// An HTTP response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK", 204 => "No Content", 301 => "Moved Permanently", 304 => "Not Modified", 400 => "Bad Request",
        401 => "Unauthorized", 403 => "Forbidden", 404 => "Not Found", 405 => "Method Not Allowed", 409 => "Conflict",
        413 => "Payload Too Large", 421 => "Misdirected Request", 500 => "Internal Server Error", _ => "Unknown",
    }
}

impl Response {
    /// A response with a content type and body.
    pub fn new(status: u16, content_type: &str, body: Vec<u8>) -> Response {
        Response { status, headers: alloc::vec![(String::from("Content-Type"), String::from(content_type))], body }
    }
    /// A small HTML message page.
    pub fn message(status: u16, text: &str) -> Response {
        Response::new(status, "text/html; charset=UTF-8", format!("<h1>{}: {}</h1><p>{}</p>", status, reason(status), text).into_bytes())
    }
    /// Adds a header.
    pub fn with(mut self, k: &str, v: &str) -> Response { self.headers.push((String::from(k), String::from(v))); self }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    /// The bytes to write: status line, headers (plus `Content-Length`), body.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut h = format!("HTTP/1.1 {} {}\r\n", self.status, reason(self.status));
        for (k, v) in &self.headers {
            // Header injection guard: values never contain line breaks.
            let v: String = v.chars().filter(|c| *c != '\r' && *c != '\n').collect();
            h.push_str(&format!("{}: {}\r\n", k, v));
        }
        h.push_str(&format!("Content-Length: {}\r\n\r\n", self.body.len()));
        let mut out = h.into_bytes();
        out.extend_from_slice(&self.body);
        out
    }
}

// ---------------------------------------------------------------------------------
// What the backend provides

/// Transmission's torrent status codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TorrentStatus { #[default] Stopped = 0, CheckWait = 1, Check = 2, DownloadWait = 3, Download = 4, SeedWait = 5, Seed = 6 }

/// One file of a torrent.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FileInfo {
    /// Path including the torrent's folder, `/`-separated.
    pub name: String,
    pub length: u64,
    pub bytes_completed: u64,
    pub wanted: bool,
    /// -1 low, 0 normal, 1 high.
    pub priority: i8,
    pub begin_piece: u32,
    pub end_piece: u32,
}

/// One tracker and its state.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TrackerInfo {
    pub id: u32,
    pub announce: String,
    pub scrape: String,
    pub sitename: String,
    pub host: String,
    pub tier: u32,
    pub is_backup: bool,
    /// 0 inactive, 1 waiting, 2 queued, 3 active.
    pub announce_state: u8,
    pub scrape_state: u8,
    pub has_announced: bool,
    pub has_scraped: bool,
    pub last_announce_time: i64,
    pub last_announce_start_time: i64,
    pub last_announce_succeeded: bool,
    pub last_announce_timed_out: bool,
    pub last_announce_result: String,
    pub last_announce_peer_count: i64,
    pub last_scrape_time: i64,
    pub last_scrape_start_time: i64,
    pub last_scrape_succeeded: bool,
    pub last_scrape_timed_out: bool,
    pub last_scrape_result: String,
    pub next_announce_time: i64,
    pub next_scrape_time: i64,
    /// -1 when unknown.
    pub seeder_count: i64,
    pub leecher_count: i64,
    pub download_count: i64,
}

/// One connected peer.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PeerInfo {
    pub address: String,
    pub port: u16,
    pub client_name: String,
    pub flag_str: String,
    pub client_is_choked: bool,
    pub client_is_interested: bool,
    pub peer_is_choked: bool,
    pub peer_is_interested: bool,
    pub is_downloading_from: bool,
    pub is_uploading_to: bool,
    pub is_encrypted: bool,
    pub is_incoming: bool,
    pub is_utp: bool,
    /// 0..=1000 (per mille).
    pub progress_permille: u16,
    pub rate_to_client: u64,
    pub rate_to_peer: u64,
}

/// Peer source counts (`peersFrom`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PeersFrom { pub cache: u32, pub dht: u32, pub incoming: u32, pub lpd: u32, pub ltep: u32, pub pex: u32, pub tracker: u32 }

/// A torrent as Transmission clients see it. Rates in bytes/s, sizes in bytes, times in
/// unix seconds (0 = never), ratios as f64 (-1 = not available, -2 = infinite).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TorrentInfo {
    pub id: u64,
    pub hash_string: String,
    pub name: String,
    pub status: TorrentStatus,
    pub error: u8,
    pub error_string: String,
    pub percent_done: f64,
    pub percent_complete: f64,
    pub metadata_percent_complete: f64,
    pub recheck_progress: f64,
    pub rate_download: u64,
    pub rate_upload: u64,
    /// Seconds; -1 not available, -2 unknown.
    pub eta: i64,
    pub eta_idle: i64,
    pub total_size: u64,
    pub size_when_done: u64,
    pub left_until_done: u64,
    pub desired_available: u64,
    pub have_valid: u64,
    pub have_unchecked: u64,
    pub corrupt_ever: u64,
    pub uploaded_ever: u64,
    pub downloaded_ever: u64,
    pub upload_ratio: f64,
    pub peers_connected: u32,
    pub peers_sending_to_us: u32,
    pub peers_getting_from_us: u32,
    pub webseeds_sending_to_us: u32,
    pub peers_from: PeersFrom,
    pub added_date: i64,
    pub done_date: i64,
    pub start_date: i64,
    pub activity_date: i64,
    pub date_created: i64,
    pub edit_date: i64,
    pub seconds_downloading: i64,
    pub seconds_seeding: i64,
    pub download_dir: String,
    pub labels: Vec<String>,
    pub group: String,
    pub comment: String,
    pub creator: String,
    pub is_private: bool,
    pub is_finished: bool,
    pub is_stalled: bool,
    pub queue_position: u32,
    pub magnet_link: String,
    pub piece_count: u32,
    pub piece_size: u64,
    /// Bitfield of pieces we have (sent base64-encoded).
    pub pieces: Vec<u8>,
    pub files: Vec<FileInfo>,
    pub trackers: Vec<TrackerInfo>,
    pub peers: Vec<PeerInfo>,
    pub webseeds: Vec<String>,
    pub bandwidth_priority: i8,
    /// KB/s.
    pub download_limit: u64,
    pub download_limited: bool,
    pub upload_limit: u64,
    pub upload_limited: bool,
    pub honors_session_limits: bool,
    pub max_connected_peers: u32,
    pub seed_ratio_limit: f64,
    /// 0 global, 1 single, 2 unlimited.
    pub seed_ratio_mode: u8,
    pub seed_idle_limit: u32,
    pub seed_idle_mode: u8,
    pub sequential_download: bool,
    pub torrent_file: String,
    pub primary_mime_type: String,
    pub manual_announce_time: i64,
}

/// Session-wide statistics block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StatsBlock { pub uploaded_bytes: u64, pub downloaded_bytes: u64, pub files_added: u64, pub session_count: u64, pub seconds_active: u64 }

/// `session-stats`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SessionStats {
    pub active_torrent_count: u32,
    pub paused_torrent_count: u32,
    pub torrent_count: u32,
    pub download_speed: u64,
    pub upload_speed: u64,
    pub cumulative: StatsBlock,
    pub current: StatsBlock,
}

/// A typed field that converts to and from JSON.
pub trait Field: Sized {
    fn to_json(&self) -> Json;
    fn from_json(j: &Json) -> Option<Self>;
}
impl Field for bool { fn to_json(&self) -> Json { Json::Bool(*self) } fn from_json(j: &Json) -> Option<bool> { j.as_bool() } }
impl Field for u64 { fn to_json(&self) -> Json { Json::from(*self) } fn from_json(j: &Json) -> Option<u64> { j.as_u64() } }
impl Field for i64 { fn to_json(&self) -> Json { Json::from(*self) } fn from_json(j: &Json) -> Option<i64> { j.as_i64() } }
impl Field for f64 { fn to_json(&self) -> Json { Json::Num(*self) } fn from_json(j: &Json) -> Option<f64> { j.as_f64().filter(|n| n.is_finite()) } }
impl Field for String { fn to_json(&self) -> Json { Json::Str(self.clone()) } fn from_json(j: &Json) -> Option<String> { j.as_str().map(String::from) } }
impl Field for Vec<u32> {
    fn to_json(&self) -> Json { Json::Arr(self.iter().map(|&n| Json::from(n)).collect()) }
    fn from_json(j: &Json) -> Option<Vec<u32>> {
        match j { Json::Arr(a) => a.iter().map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok())).collect(), Json::Num(_) => j.as_u64().and_then(|n| u32::try_from(n).ok()).map(|n| alloc::vec![n]), _ => None }
    }
}
impl Field for Vec<u64> {
    fn to_json(&self) -> Json { Json::Arr(self.iter().map(|&n| Json::from(n)).collect()) }
    fn from_json(j: &Json) -> Option<Vec<u64>> { j.as_arr()?.iter().map(|v| v.as_u64()).collect() }
}
impl Field for Vec<String> {
    fn to_json(&self) -> Json { Json::Arr(self.iter().map(|s| Json::from(s.as_str())).collect()) }
    fn from_json(j: &Json) -> Option<Vec<String>> { j.as_arr()?.iter().map(|v| v.as_str().map(String::from)).collect() }
}

/// Normalizes a key: lower case without `-` and `_`.
fn nk(k: &str) -> String { k.chars().filter(|c| *c != '-' && *c != '_').map(|c| c.to_ascii_lowercase()).collect() }

/// An argument by key, ignoring case, `-` and `_`.
fn arg<'a>(args: &'a Json, key: &str) -> Option<&'a Json> {
    let want = nk(key);
    args.as_obj()?.iter().find(|(k, _)| nk(k) == want).map(|(_, v)| v)
}

macro_rules! settings {
    ($(#[$m:meta])* $name:ident, $changes:ident { $($(#[$fm:meta])* $f:ident : $t:ty = $k:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Default)]
        pub struct $name { $($(#[$fm])* pub $f: $t,)* }
        /// The settable subset sent by a client: `None` = unchanged.
        #[derive(Clone, Debug, PartialEq, Default)]
        pub struct $changes { $(pub $f: Option<$t>,)* }
        impl $name {
            /// Keys and values in Transmission's names.
            pub fn to_pairs(&self) -> Vec<(String, Json)> { alloc::vec![$((String::from($k), Field::to_json(&self.$f)),)*] }
            /// Applies changes.
            pub fn apply(&mut self, c: &$changes) { $(if let Some(v) = &c.$f { self.$f = v.clone(); })* }
        }
        impl $changes {
            /// Reads changes from request arguments.
            pub fn from_args(args: &Json) -> $changes { $changes { $($f: arg(args, $k).and_then(Field::from_json),)* } }
            pub fn is_empty(&self) -> bool { true $(&& self.$f.is_none())* }
        }
    };
}

settings!(
    /// Session settings (`session-get` / `session-set`). Speeds in KB/s like Transmission.
    SessionSettings, SessionChanges {
    alt_speed_down: u64 = "alt-speed-down",
    alt_speed_enabled: bool = "alt-speed-enabled",
    alt_speed_time_begin: u64 = "alt-speed-time-begin",
    alt_speed_time_enabled: bool = "alt-speed-time-enabled",
    alt_speed_time_end: u64 = "alt-speed-time-end",
    alt_speed_time_day: u64 = "alt-speed-time-day",
    alt_speed_up: u64 = "alt-speed-up",
    blocklist_enabled: bool = "blocklist-enabled",
    blocklist_size: u64 = "blocklist-size",
    blocklist_url: String = "blocklist-url",
    cache_size_mb: u64 = "cache-size-mb",
    config_dir: String = "config-dir",
    default_trackers: String = "default-trackers",
    dht_enabled: bool = "dht-enabled",
    download_dir: String = "download-dir",
    download_queue_enabled: bool = "download-queue-enabled",
    download_queue_size: u64 = "download-queue-size",
    encryption: String = "encryption",
    idle_seeding_limit: u64 = "idle-seeding-limit",
    idle_seeding_limit_enabled: bool = "idle-seeding-limit-enabled",
    incomplete_dir: String = "incomplete-dir",
    incomplete_dir_enabled: bool = "incomplete-dir-enabled",
    lpd_enabled: bool = "lpd-enabled",
    peer_limit_global: u64 = "peer-limit-global",
    peer_limit_per_torrent: u64 = "peer-limit-per-torrent",
    peer_port: u64 = "peer-port",
    peer_port_random_on_start: bool = "peer-port-random-on-start",
    pex_enabled: bool = "pex-enabled",
    port_forwarding_enabled: bool = "port-forwarding-enabled",
    queue_stalled_enabled: bool = "queue-stalled-enabled",
    queue_stalled_minutes: u64 = "queue-stalled-minutes",
    rename_partial_files: bool = "rename-partial-files",
    script_torrent_done_enabled: bool = "script-torrent-done-enabled",
    script_torrent_done_filename: String = "script-torrent-done-filename",
    seed_queue_enabled: bool = "seed-queue-enabled",
    seed_queue_size: u64 = "seed-queue-size",
    seed_ratio_limit: f64 = "seedRatioLimit",
    seed_ratio_limited: bool = "seedRatioLimited",
    speed_limit_down: u64 = "speed-limit-down",
    speed_limit_down_enabled: bool = "speed-limit-down-enabled",
    speed_limit_up: u64 = "speed-limit-up",
    speed_limit_up_enabled: bool = "speed-limit-up-enabled",
    start_added_torrents: bool = "start-added-torrents",
    trash_original_torrent_files: bool = "trash-original-torrent-files",
    utp_enabled: bool = "utp-enabled",
});

settings!(
    /// Per-torrent settable values, as stored (used for `torrent-set`).
    TorrentSettings, TorrentChanges {
    bandwidth_priority: i64 = "bandwidthPriority",
    download_limit: u64 = "downloadLimit",
    download_limited: bool = "downloadLimited",
    /// File indices; an empty list means all files.
    files_wanted: Vec<u32> = "files-wanted",
    files_unwanted: Vec<u32> = "files-unwanted",
    honors_session_limits: bool = "honorsSessionLimits",
    labels: Vec<String> = "labels",
    group: String = "group",
    location: String = "location",
    peer_limit: u64 = "peer-limit",
    priority_high: Vec<u32> = "priority-high",
    priority_low: Vec<u32> = "priority-low",
    priority_normal: Vec<u32> = "priority-normal",
    queue_position: u64 = "queuePosition",
    seed_idle_limit: u64 = "seedIdleLimit",
    seed_idle_mode: i64 = "seedIdleMode",
    seed_ratio_limit: f64 = "seedRatioLimit",
    seed_ratio_mode: i64 = "seedRatioMode",
    sequential_download: bool = "sequentialDownload",
    tracker_add: Vec<String> = "trackerAdd",
    tracker_remove: Vec<u64> = "trackerRemove",
    tracker_list: String = "trackerList",
    upload_limit: u64 = "uploadLimit",
    upload_limited: bool = "uploadLimited",
});

/// Where a torrent to add comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddSource {
    /// A magnet link (a bare 40-hex info-hash is turned into one).
    Magnet(String),
    /// An http(s) URL of a .torrent.
    Url(String),
    /// A path on the server.
    Path(String),
    /// The .torrent bytes.
    Metainfo(Vec<u8>),
}

/// `torrent-add`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddRequest {
    pub source: AddSource,
    pub paused: Option<bool>,
    pub download_dir: Option<String>,
    pub labels: Vec<String>,
    pub files_wanted: Vec<u32>,
    pub files_unwanted: Vec<u32>,
    pub priority_high: Vec<u32>,
    pub priority_low: Vec<u32>,
    pub priority_normal: Vec<u32>,
    pub bandwidth_priority: Option<i64>,
    pub peer_limit: Option<u64>,
    pub cookies: Option<String>,
    pub sequential_download: Option<bool>,
}

/// The result of an add.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddOutcome {
    Added { id: u64, name: String, hash: String },
    Duplicate { id: u64, name: String, hash: String },
}

/// Torrent actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorrentAction {
    Start, StartNow, Stop, Verify, Reannounce,
    Remove { delete_local_data: bool },
    QueueTop, QueueUp, QueueDown, QueueBottom,
}

/// What the program provides to the RPC server.
pub trait RpcBackend {
    fn session(&self) -> SessionSettings;
    fn set_session(&mut self, changes: &SessionChanges) -> Result<(), String>;
    fn stats(&self) -> SessionStats;
    /// Torrents by id (`None` = all). With `detail` false the program may leave files,
    /// trackers, peers and pieces empty.
    fn torrents(&self, ids: Option<&[u64]>, detail: bool) -> Vec<TorrentInfo>;
    /// Torrents active recently (Transmission: in the last 60 seconds).
    fn recently_active(&self) -> Vec<u64>;
    /// Ids removed recently.
    fn recently_removed(&self) -> Vec<u64>;
    fn add(&mut self, req: AddRequest) -> Result<AddOutcome, String>;
    fn action(&mut self, ids: &[u64], action: TorrentAction) -> Result<(), String>;
    fn set(&mut self, ids: &[u64], changes: &TorrentChanges) -> Result<(), String>;
    fn set_location(&mut self, ids: &[u64], location: &str, move_data: bool) -> Result<(), String>;
    fn rename_path(&mut self, _id: u64, _path: &str, _name: &str) -> Result<(), String> { Err(String::from("not supported")) }
    /// Free and total bytes at a path.
    fn free_space(&self, path: &str) -> Option<(u64, u64)>;
    /// Whether the peer port is reachable, if known.
    fn port_test(&mut self) -> Option<bool> { None }
    /// Updates the blocklist; returns its size.
    fn blocklist_update(&mut self) -> Result<u64, String> { Ok(0) }
}

// ---------------------------------------------------------------------------------
// Torrent fields

/// Every `torrent-get` field this server knows, in Transmission's names.
pub const TORRENT_FIELDS: &[&str] = &[
    "activityDate", "addedDate", "bandwidthPriority", "comment", "corruptEver", "creator", "dateCreated",
    "desiredAvailable", "doneDate", "downloadDir", "downloadedEver", "downloadLimit", "downloadLimited",
    "editDate", "error", "errorString", "eta", "etaIdle", "file-count", "files", "fileStats", "group",
    "hashString", "haveUnchecked", "haveValid", "honorsSessionLimits", "id", "isFinished", "isPrivate",
    "isStalled", "labels", "leftUntilDone", "magnetLink", "manualAnnounceTime", "maxConnectedPeers",
    "metadataPercentComplete", "name", "peer-limit", "peers", "peersConnected", "peersFrom",
    "peersGettingFromUs", "peersSendingToUs", "percentComplete", "percentDone", "pieces", "pieceCount",
    "pieceSize", "priorities", "primary-mime-type", "queuePosition", "rateDownload", "rateUpload",
    "recheckProgress", "secondsDownloading", "secondsSeeding", "seedIdleLimit", "seedIdleMode",
    "seedRatioLimit", "seedRatioMode", "sequentialDownload", "sizeWhenDone", "startDate", "status",
    "torrentFile", "totalSize", "trackerList", "trackers", "trackerStats", "uploadedEver", "uploadLimit",
    "uploadLimited", "uploadRatio", "wanted", "webseeds", "webseedsSendingToUs",
];

const DETAIL_FIELDS: &[&str] = &["files", "fileStats", "priorities", "wanted", "peers", "peersFrom", "pieces", "trackers", "trackerStats", "trackerList", "webseeds"];

fn canonical(name: &str) -> Option<&'static str> {
    let n = nk(name);
    TORRENT_FIELDS.iter().copied().find(|f| nk(f) == n)
}

fn strs(v: &[String]) -> Json { Json::Arr(v.iter().map(|s| Json::from(s.as_str())).collect()) }

fn tracker_json(t: &TrackerInfo, stats: bool) -> Json {
    let mut o = alloc::vec![
        (String::from("announce"), Json::from(t.announce.as_str())),
        (String::from("id"), Json::from(t.id)),
        (String::from("scrape"), Json::from(t.scrape.as_str())),
        (String::from("sitename"), Json::from(t.sitename.as_str())),
        (String::from("tier"), Json::from(t.tier)),
    ];
    if stats {
        let more: [(&str, Json); 22] = [
            ("announceState", Json::from(t.announce_state as u32)), ("downloadCount", Json::from(t.download_count)),
            ("hasAnnounced", Json::from(t.has_announced)), ("hasScraped", Json::from(t.has_scraped)), ("host", Json::from(t.host.as_str())),
            ("isBackup", Json::from(t.is_backup)), ("lastAnnouncePeerCount", Json::from(t.last_announce_peer_count)),
            ("lastAnnounceResult", Json::from(t.last_announce_result.as_str())), ("lastAnnounceStartTime", Json::from(t.last_announce_start_time)),
            ("lastAnnounceSucceeded", Json::from(t.last_announce_succeeded)), ("lastAnnounceTime", Json::from(t.last_announce_time)),
            ("lastAnnounceTimedOut", Json::from(t.last_announce_timed_out)), ("lastScrapeResult", Json::from(t.last_scrape_result.as_str())),
            ("lastScrapeStartTime", Json::from(t.last_scrape_start_time)), ("lastScrapeSucceeded", Json::from(t.last_scrape_succeeded)),
            ("lastScrapeTime", Json::from(t.last_scrape_time)), ("lastScrapeTimedOut", Json::from(t.last_scrape_timed_out)),
            ("leecherCount", Json::from(t.leecher_count)), ("nextAnnounceTime", Json::from(t.next_announce_time)),
            ("nextScrapeTime", Json::from(t.next_scrape_time)), ("scrapeState", Json::from(t.scrape_state as u32)),
            ("seederCount", Json::from(t.seeder_count)),
        ];
        o.extend(more.into_iter().map(|(k, v)| (String::from(k), v)));
    }
    Json::Obj(o)
}

fn peer_json(p: &PeerInfo) -> Json {
    Json::obj([
        ("address", Json::from(p.address.as_str())), ("clientName", Json::from(p.client_name.as_str())),
        ("clientIsChoked", Json::from(p.client_is_choked)), ("clientIsInterested", Json::from(p.client_is_interested)),
        ("flagStr", Json::from(p.flag_str.as_str())), ("isDownloadingFrom", Json::from(p.is_downloading_from)),
        ("isEncrypted", Json::from(p.is_encrypted)), ("isIncoming", Json::from(p.is_incoming)),
        ("isUploadingTo", Json::from(p.is_uploading_to)), ("isUTP", Json::from(p.is_utp)),
        ("peerIsChoked", Json::from(p.peer_is_choked)), ("peerIsInterested", Json::from(p.peer_is_interested)),
        ("port", Json::from(p.port as u32)), ("progress", Json::from(p.progress_permille.min(1000) as f64 / 1000.0)),
        ("rateToClient", Json::from(p.rate_to_client)), ("rateToPeer", Json::from(p.rate_to_peer)),
    ])
}

/// The value of one field (by its canonical name).
fn torrent_field(t: &TorrentInfo, f: &str) -> Json {
    match f {
        "activityDate" => t.activity_date.into(), "addedDate" => t.added_date.into(),
        "bandwidthPriority" => (t.bandwidth_priority as i64).into(), "comment" => t.comment.as_str().into(),
        "corruptEver" => t.corrupt_ever.into(), "creator" => t.creator.as_str().into(), "dateCreated" => t.date_created.into(),
        "desiredAvailable" => t.desired_available.into(), "doneDate" => t.done_date.into(), "downloadDir" => t.download_dir.as_str().into(),
        "downloadedEver" => t.downloaded_ever.into(), "downloadLimit" => t.download_limit.into(), "downloadLimited" => t.download_limited.into(),
        "editDate" => t.edit_date.into(), "error" => (t.error as u32).into(), "errorString" => t.error_string.as_str().into(),
        "eta" => t.eta.into(), "etaIdle" => t.eta_idle.into(), "file-count" => (t.files.len() as u64).into(),
        "files" => Json::Arr(t.files.iter().map(|f| Json::obj([
            ("bytesCompleted", f.bytes_completed.into()), ("length", f.length.into()), ("name", f.name.as_str().into()),
            ("begin_piece", f.begin_piece.into()), ("end_piece", f.end_piece.into()),
        ])).collect()),
        "fileStats" => Json::Arr(t.files.iter().map(|f| Json::obj([
            ("bytesCompleted", f.bytes_completed.into()), ("wanted", f.wanted.into()), ("priority", (f.priority as i64).into()),
        ])).collect()),
        "group" => t.group.as_str().into(), "hashString" => t.hash_string.as_str().into(),
        "haveUnchecked" => t.have_unchecked.into(), "haveValid" => t.have_valid.into(),
        "honorsSessionLimits" => t.honors_session_limits.into(), "id" => t.id.into(), "isFinished" => t.is_finished.into(),
        "isPrivate" => t.is_private.into(), "isStalled" => t.is_stalled.into(), "labels" => strs(&t.labels),
        "leftUntilDone" => t.left_until_done.into(), "magnetLink" => t.magnet_link.as_str().into(),
        "manualAnnounceTime" => t.manual_announce_time.into(), "maxConnectedPeers" => t.max_connected_peers.into(),
        "metadataPercentComplete" => t.metadata_percent_complete.into(), "name" => t.name.as_str().into(),
        "peer-limit" => t.max_connected_peers.into(), "peers" => Json::Arr(t.peers.iter().map(peer_json).collect()),
        "peersConnected" => t.peers_connected.into(),
        "peersFrom" => Json::obj([
            ("fromCache", t.peers_from.cache.into()), ("fromDht", t.peers_from.dht.into()), ("fromIncoming", t.peers_from.incoming.into()),
            ("fromLpd", t.peers_from.lpd.into()), ("fromLtep", t.peers_from.ltep.into()), ("fromPex", t.peers_from.pex.into()),
            ("fromTracker", t.peers_from.tracker.into()),
        ]),
        "peersGettingFromUs" => t.peers_getting_from_us.into(), "peersSendingToUs" => t.peers_sending_to_us.into(),
        "percentComplete" => t.percent_complete.into(), "percentDone" => t.percent_done.into(),
        "pieces" => base64_encode(&t.pieces).into(), "pieceCount" => t.piece_count.into(), "pieceSize" => t.piece_size.into(),
        "priorities" => Json::Arr(t.files.iter().map(|f| Json::from(f.priority as i64)).collect()),
        "primary-mime-type" => t.primary_mime_type.as_str().into(), "queuePosition" => t.queue_position.into(),
        "rateDownload" => t.rate_download.into(), "rateUpload" => t.rate_upload.into(), "recheckProgress" => t.recheck_progress.into(),
        "secondsDownloading" => t.seconds_downloading.into(), "secondsSeeding" => t.seconds_seeding.into(),
        "seedIdleLimit" => t.seed_idle_limit.into(), "seedIdleMode" => (t.seed_idle_mode as u32).into(),
        "seedRatioLimit" => t.seed_ratio_limit.into(), "seedRatioMode" => (t.seed_ratio_mode as u32).into(),
        "sequentialDownload" => t.sequential_download.into(), "sizeWhenDone" => t.size_when_done.into(),
        "startDate" => t.start_date.into(), "status" => (t.status as u32).into(), "torrentFile" => t.torrent_file.as_str().into(),
        "totalSize" => t.total_size.into(),
        "trackerList" => {
            let mut s = String::new();
            let mut tier = None;
            for tr in &t.trackers {
                if tier.is_some() { s.push('\n'); if tier != Some(tr.tier) { s.push('\n'); } }
                s.push_str(&tr.announce);
                tier = Some(tr.tier);
            }
            s.into()
        }
        "trackers" => Json::Arr(t.trackers.iter().map(|x| tracker_json(x, false)).collect()),
        "trackerStats" => Json::Arr(t.trackers.iter().map(|x| tracker_json(x, true)).collect()),
        "uploadedEver" => t.uploaded_ever.into(), "uploadLimit" => t.upload_limit.into(), "uploadLimited" => t.upload_limited.into(),
        "uploadRatio" => t.upload_ratio.into(),
        "wanted" => Json::Arr(t.files.iter().map(|f| Json::from(f.wanted as u32)).collect()),
        "webseeds" => strs(&t.webseeds), "webseedsSendingToUs" => t.webseeds_sending_to_us.into(),
        _ => Json::Null,
    }
}

// ---------------------------------------------------------------------------------
// The server

/// Server configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcConfig {
    pub auth_required: bool,
    pub username: String,
    pub password: String,
    /// Host names accepted in the `Host` header (`*` wildcards); IP literals and
    /// `localhost` always pass.
    pub host_whitelist: Vec<String>,
    pub host_whitelist_enabled: bool,
    /// The RPC path (Transmission's default `/transmission/rpc`).
    pub rpc_path: String,
    /// Serve the embedded web UI on other paths.
    pub serve_webui: bool,
}

impl Default for RpcConfig {
    fn default() -> RpcConfig {
        RpcConfig { auth_required: false, username: String::new(), password: String::new(), host_whitelist: Vec::new(),
            host_whitelist_enabled: true, rpc_path: String::from("/transmission/rpc"), serve_webui: true }
    }
}

/// What to do with bytes read from a client connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpOutcome {
    /// Not a whole request yet: read more and call again with all the bytes.
    NeedMore,
    /// Write `bytes`, drop the first `consumed` input bytes, and close if asked.
    Reply { bytes: Vec<u8>, consumed: usize, close: bool },
}

/// The RPC version we speak (Transmission 4.0).
pub const RPC_VERSION: u64 = 17;
pub const RPC_VERSION_MINIMUM: u64 = 14;
pub const RPC_VERSION_SEMVER: &str = "5.3.0";
/// Version string reported in `session-get`.
pub const SERVER_VERSION: &str = "4.0.6 (Swingers 0.1.0)";

/// The Transmission RPC server state.
#[derive(Clone, Debug)]
pub struct RpcServer {
    pub config: RpcConfig,
    session_id: String,
}

const SID_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

fn host_ok(cfg: &RpcConfig, host: Option<&str>) -> bool {
    if !cfg.host_whitelist_enabled { return true; }
    let Some(h) = host else { return true }; // HTTP/1.0 without Host
    let h = h.trim();
    let name = if let Some(rest) = h.strip_prefix('[') { return rest.contains(']'); } else { h.rsplit_once(':').map(|(n, _)| n).unwrap_or(h) };
    let name = name.to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") { return true; }
    if !name.is_empty() && name.split('.').count() == 4 && name.split('.').all(|p| p.parse::<u8>().is_ok()) { return true; }
    let n: Vec<char> = name.chars().collect();
    cfg.host_whitelist.iter().any(|w| crate::rules::glob_chars(&w.to_ascii_lowercase().chars().collect::<Vec<char>>(), &n))
}

fn to_snake(k: &str) -> String {
    let mut s = String::with_capacity(k.len() + 4);
    for (i, c) in k.chars().enumerate() {
        if c == '-' { s.push('_'); }
        else if c.is_ascii_uppercase() { if i > 0 { s.push('_'); } s.push(c.to_ascii_lowercase()); }
        else { s.push(c); }
    }
    s
}

fn snake_keys(j: Json, depth: usize) -> Json {
    if depth > json::MAX_DEPTH { return j; }
    match j {
        Json::Obj(o) => Json::Obj(o.into_iter().map(|(k, v)| (to_snake(&k), snake_keys(v, depth + 1))).collect()),
        Json::Arr(a) => Json::Arr(a.into_iter().map(|v| snake_keys(v, depth + 1)).collect()),
        v => v,
    }
}

impl RpcServer {
    /// A server with a fresh session id from `random`.
    pub fn new(config: RpcConfig, random: &mut dyn FnMut(&mut [u8])) -> RpcServer {
        let mut s = RpcServer { config, session_id: String::new() };
        s.rotate_session_id(random);
        s
    }
    /// The current CSRF session id.
    pub fn session_id(&self) -> &str { &self.session_id }
    /// Picks a new session id (clients will get one 409 and retry).
    pub fn rotate_session_id(&mut self, random: &mut dyn FnMut(&mut [u8])) {
        let mut b = [0u8; 48];
        random(&mut b);
        self.session_id = b.iter().map(|&x| SID_CHARS[x as usize % SID_CHARS.len()] as char).collect();
    }

    /// Handles bytes from a connection (see [`HttpOutcome`]).
    pub fn handle_http(&mut self, buf: &[u8], backend: &mut dyn RpcBackend) -> HttpOutcome {
        match parse_request(buf) {
            Err(HttpError::Incomplete) => HttpOutcome::NeedMore,
            Err(e) => {
                let r = if e == HttpError::TooLarge { Response::message(413, "Request too large.") } else { Response::message(400, "Malformed request.") };
                HttpOutcome::Reply { bytes: r.with("Connection", "close").to_bytes(), consumed: buf.len(), close: true }
            }
            Ok((req, consumed)) => {
                let mut r = self.handle(&req, backend);
                if req.close { r = r.with("Connection", "close"); }
                HttpOutcome::Reply { bytes: r.to_bytes(), consumed, close: req.close }
            }
        }
    }

    /// Handles one parsed request.
    pub fn handle(&mut self, req: &Request, backend: &mut dyn RpcBackend) -> Response {
        if !host_ok(&self.config, req.header("Host")) {
            return Response::message(421, "Transmission received your request, but the hostname was unrecognized. Add it to the RPC host whitelist.");
        }
        if self.config.auth_required && !self.authorized(req) {
            return Response::message(401, "Unauthorized User").with("WWW-Authenticate", "Basic realm=\"Swingers\"");
        }
        let path = req.path.trim_end_matches('/');
        if path == self.config.rpc_path.trim_end_matches('/') {
            let sid_ok = req.header("X-Transmission-Session-Id").is_some_and(|s| ct_eq(s.as_bytes(), self.session_id.as_bytes()));
            if !sid_ok {
                return Response::message(409, "Your request had an invalid session-id header. To fix this, retry with the X-Transmission-Session-Id header from this response.")
                    .with("X-Transmission-Session-Id", &self.session_id);
            }
            if req.method != "POST" { return Response::message(405, "Use POST.").with("Allow", "POST"); }
            return match self.handle_rpc(&req.body, backend) {
                Ok(j) => Response::new(200, "application/json; charset=UTF-8", j.encode().into_bytes()).with("X-Transmission-Session-Id", &self.session_id),
                Err(e) => Response::message(400, e),
            };
        }
        if self.config.serve_webui {
            if let Some(r) = crate::webui::serve(&req.method, &req.path) { return r; }
        }
        Response::message(404, "Not found.")
    }

    fn authorized(&self, req: &Request) -> bool {
        let Some(h) = req.header("Authorization") else { return false };
        let Some(b64) = h.strip_prefix("Basic ").or_else(|| h.strip_prefix("basic ")) else { return false };
        let Some(raw) = base64_decode(b64.trim().as_bytes()) else { return false };
        let (u, p) = match raw.iter().position(|&c| c == b':') { Some(i) => (&raw[..i], &raw[i + 1..]), None => (&raw[..], &b""[..]) };
        let a = ct_eq(u, self.config.username.as_bytes());
        let b = ct_eq(p, self.config.password.as_bytes());
        a & b
    }

    /// Handles a JSON request body (no HTTP); the error is for unparsable input.
    pub fn handle_rpc(&mut self, body: &[u8], backend: &mut dyn RpcBackend) -> Result<Json, &'static str> {
        let req = json::parse(body).map_err(|_| "Invalid JSON.")?;
        if req.as_obj().is_none() { return Err("Expected a JSON object."); }
        let jsonrpc = req.get("jsonrpc").is_some();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("").replace('_', "-");
        let empty = Json::Obj(Vec::new());
        let args = req.get(if jsonrpc { "params" } else { "arguments" }).filter(|a| a.as_obj().is_some()).unwrap_or(&empty);
        let (result, out) = match self.dispatch(&method, args, backend) {
            Ok(o) => (String::from("success"), o),
            Err(e) => (e, Vec::new()),
        };
        if jsonrpc {
            let id = req.get("id").cloned().unwrap_or(Json::Null);
            return Ok(if result == "success" {
                Json::obj([("jsonrpc", "2.0".into()), ("result", snake_keys(Json::Obj(out), 0)), ("id", id)])
            } else {
                let code = if result.starts_with("method name not recognized") { -32601 } else { -32000 };
                Json::obj([("jsonrpc", "2.0".into()), ("error", Json::obj([("code", Json::from(code as i64)), ("message", result.into())])), ("id", id)])
            });
        }
        let mut resp = alloc::vec![(String::from("arguments"), Json::Obj(out)), (String::from("result"), Json::Str(result))];
        if let Some(tag) = req.get("tag") { resp.push((String::from("tag"), tag.clone())); }
        Ok(Json::Obj(resp))
    }

    fn resolve_args(&self, args: &Json, backend: &dyn RpcBackend) -> Result<Option<Vec<u64>>, String> {
        // Hash strings are looked up against the backend's list.
        let Some(v) = arg(args, "ids") else { return Ok(None) };
        if v.as_str().is_some_and(|s| s == "recently-active" || s == "recently_active") { return Ok(Some(backend.recently_active())); }
        let items: Vec<&Json> = match v { Json::Arr(a) => a.iter().take(100_000).collect(), v => alloc::vec![v] };
        let mut out = Vec::new();
        let mut index: Option<Vec<(u64, String)>> = None;
        for it in items {
            match it {
                Json::Num(_) => out.push(it.as_u64().ok_or_else(|| String::from("invalid ids"))?),
                Json::Str(s) => {
                    let idx = index.get_or_insert_with(|| backend.torrents(None, false).into_iter().map(|t| (t.id, t.hash_string.to_ascii_lowercase())).collect());
                    let s = s.to_ascii_lowercase();
                    if let Some((id, _)) = idx.iter().find(|(_, h)| *h == s) { out.push(*id); }
                    else if let Ok(n) = s.parse::<u64>() { out.push(n); }
                }
                _ => return Err(String::from("invalid ids")),
            }
        }
        Ok(Some(out))
    }

    fn dispatch(&mut self, method: &str, args: &Json, b: &mut dyn RpcBackend) -> Result<Vec<(String, Json)>, String> {
        let p = |k: &str, v: Json| (String::from(k), v);
        match method {
            "session-get" => {
                let mut pairs = b.session().to_pairs();
                pairs.push(p("rpc-version", RPC_VERSION.into()));
                pairs.push(p("rpc-version-minimum", RPC_VERSION_MINIMUM.into()));
                pairs.push(p("rpc-version-semver", RPC_VERSION_SEMVER.into()));
                pairs.push(p("version", SERVER_VERSION.into()));
                pairs.push(p("session-id", self.session_id.as_str().into()));
                pairs.push(p("units", Json::obj([
                    ("speed-units", Json::Arr(["kB/s", "MB/s", "GB/s", "TB/s"].iter().map(|s| Json::from(*s)).collect())),
                    ("speed-bytes", 1000u64.into()),
                    ("size-units", Json::Arr(["kB", "MB", "GB", "TB"].iter().map(|s| Json::from(*s)).collect())),
                    ("size-bytes", 1000u64.into()),
                    ("memory-units", Json::Arr(["KiB", "MiB", "GiB", "TiB"].iter().map(|s| Json::from(*s)).collect())),
                    ("memory-bytes", 1024u64.into()),
                ])));
                if let Some(fields) = arg(args, "fields").and_then(|f| f.as_arr()) {
                    let want: Vec<String> = fields.iter().filter_map(|f| f.as_str()).map(nk).collect();
                    pairs.retain(|(k, _)| want.contains(&nk(k)));
                }
                Ok(pairs)
            }
            "session-set" => {
                let c = SessionChanges::from_args(args);
                b.set_session(&c)?;
                Ok(Vec::new())
            }
            "session-stats" => {
                let s = b.stats();
                let block = |x: StatsBlock| Json::obj([
                    ("uploadedBytes", x.uploaded_bytes.into()), ("downloadedBytes", x.downloaded_bytes.into()),
                    ("filesAdded", x.files_added.into()), ("sessionCount", x.session_count.into()), ("secondsActive", x.seconds_active.into()),
                ]);
                Ok(alloc::vec![
                    p("activeTorrentCount", s.active_torrent_count.into()), p("downloadSpeed", s.download_speed.into()),
                    p("pausedTorrentCount", s.paused_torrent_count.into()), p("torrentCount", s.torrent_count.into()),
                    p("uploadSpeed", s.upload_speed.into()), p("cumulative-stats", block(s.cumulative)), p("current-stats", block(s.current)),
                ])
            }
            "session-close" | "group-set" => Ok(Vec::new()),
            "group-get" => Ok(alloc::vec![p("group", Json::Arr(Vec::new()))]),
            "torrent-get" => {
                let recent = arg(args, "ids").and_then(|v| v.as_str()).is_some_and(|s| s == "recently-active" || s == "recently_active");
                let ids = self.resolve_args(args, b)?;
                let fields: Vec<&'static str> = match arg(args, "fields").and_then(|f| f.as_arr()) {
                    Some(f) => { let mut v: Vec<&'static str> = f.iter().filter_map(|x| x.as_str()).filter_map(canonical).collect(); v.dedup(); v }
                    None => TORRENT_FIELDS.to_vec(),
                };
                let detail = fields.iter().any(|f| DETAIL_FIELDS.contains(f));
                let list = b.torrents(ids.as_deref(), detail);
                let table = arg(args, "format").and_then(|f| f.as_str()) == Some("table");
                let torrents = if table {
                    let mut rows = alloc::vec![Json::Arr(fields.iter().map(|f| Json::from(*f)).collect())];
                    rows.extend(list.iter().map(|t| Json::Arr(fields.iter().map(|f| torrent_field(t, f)).collect())));
                    Json::Arr(rows)
                } else {
                    Json::Arr(list.iter().map(|t| Json::Obj(fields.iter().map(|f| (String::from(*f), torrent_field(t, f))).collect())).collect())
                };
                let mut out = alloc::vec![p("torrents", torrents)];
                if recent { out.push(p("removed", Json::Arr(b.recently_removed().into_iter().map(Json::from).collect()))); }
                Ok(out)
            }
            "torrent-add" => {
                let source = if let Some(m) = arg(args, "metainfo").and_then(|m| m.as_str()) {
                    AddSource::Metainfo(base64_decode(m.as_bytes()).filter(|v| !v.is_empty()).ok_or_else(|| String::from("invalid or corrupt torrent file"))?)
                } else if let Some(f) = arg(args, "filename").and_then(|f| f.as_str()).map(str::trim).filter(|f| !f.is_empty()) {
                    let lower = f.to_ascii_lowercase();
                    if lower.starts_with("magnet:") { AddSource::Magnet(String::from(f)) }
                    else if lower.starts_with("http://") || lower.starts_with("https://") { AddSource::Url(String::from(f)) }
                    else if (f.len() == 40 || f.len() == 32) && crate::rss::parse_hash(f).is_some() { AddSource::Magnet(format!("magnet:?xt=urn:btih:{}", f)) }
                    else { AddSource::Path(String::from(f)) }
                } else {
                    return Err(String::from("no filename or metainfo specified"));
                };
                let v32 = |k: &str| arg(args, k).and_then(<Vec<u32> as Field>::from_json).unwrap_or_default();
                let req = AddRequest {
                    source,
                    paused: arg(args, "paused").and_then(|v| v.as_bool()),
                    download_dir: arg(args, "download-dir").and_then(|v| v.as_str()).map(String::from),
                    labels: arg(args, "labels").and_then(<Vec<String> as Field>::from_json).unwrap_or_default(),
                    files_wanted: v32("files-wanted"), files_unwanted: v32("files-unwanted"),
                    priority_high: v32("priority-high"), priority_low: v32("priority-low"), priority_normal: v32("priority-normal"),
                    bandwidth_priority: arg(args, "bandwidthPriority").and_then(|v| v.as_i64()),
                    peer_limit: arg(args, "peer-limit").and_then(|v| v.as_u64()),
                    cookies: arg(args, "cookies").and_then(|v| v.as_str()).map(String::from),
                    sequential_download: arg(args, "sequentialDownload").and_then(|v| v.as_bool()),
                };
                let obj = |id: u64, name: String, hash: String| Json::obj([("id", id.into()), ("name", name.into()), ("hashString", hash.into())]);
                Ok(match b.add(req)? {
                    AddOutcome::Added { id, name, hash } => alloc::vec![p("torrent-added", obj(id, name, hash))],
                    AddOutcome::Duplicate { id, name, hash } => alloc::vec![p("torrent-duplicate", obj(id, name, hash))],
                })
            }
            "torrent-start" | "torrent-start-now" | "torrent-stop" | "torrent-verify" | "torrent-reannounce" | "torrent-remove"
            | "queue-move-top" | "queue-move-up" | "queue-move-down" | "queue-move-bottom" => {
                let action = match method {
                    "torrent-start" => TorrentAction::Start, "torrent-start-now" => TorrentAction::StartNow,
                    "torrent-stop" => TorrentAction::Stop, "torrent-verify" => TorrentAction::Verify,
                    "torrent-reannounce" => TorrentAction::Reannounce,
                    "torrent-remove" => TorrentAction::Remove { delete_local_data: arg(args, "delete-local-data").and_then(|v| v.as_bool()).unwrap_or(false) },
                    "queue-move-top" => TorrentAction::QueueTop, "queue-move-up" => TorrentAction::QueueUp,
                    "queue-move-down" => TorrentAction::QueueDown, _ => TorrentAction::QueueBottom,
                };
                let ids = self.all_or(args, b)?;
                b.action(&ids, action)?;
                Ok(Vec::new())
            }
            "torrent-set" => {
                let ids = self.all_or(args, b)?;
                b.set(&ids, &TorrentChanges::from_args(args))?;
                Ok(Vec::new())
            }
            "torrent-set-location" => {
                let ids = self.all_or(args, b)?;
                let loc = arg(args, "location").and_then(|v| v.as_str()).ok_or_else(|| String::from("no location"))?;
                b.set_location(&ids, loc, arg(args, "move").and_then(|v| v.as_bool()).unwrap_or(false))?;
                Ok(Vec::new())
            }
            "torrent-rename-path" => {
                let ids = self.all_or(args, b)?;
                if ids.len() != 1 { return Err(String::from("torrent-rename-path requires 1 torrent")); }
                let path = arg(args, "path").and_then(|v| v.as_str()).unwrap_or("");
                let name = arg(args, "name").and_then(|v| v.as_str()).unwrap_or("");
                b.rename_path(ids[0], path, name)?;
                Ok(alloc::vec![p("path", path.into()), p("name", name.into()), p("id", ids[0].into())])
            }
            "free-space" => {
                let path = arg(args, "path").and_then(|v| v.as_str()).unwrap_or("");
                let (free, total) = b.free_space(path).ok_or_else(|| format!("free-space: cannot read {}", path))?;
                Ok(alloc::vec![p("path", path.into()), p("size-bytes", free.into()), p("total_size", total.into())])
            }
            "port-test" => {
                let mut out = alloc::vec![p("port-is-open", b.port_test().unwrap_or(false).into())];
                if let Some(ip) = arg(args, "ipProtocol").and_then(|v| v.as_str()) { out.push(p("ipProtocol", ip.into())); }
                Ok(out)
            }
            "blocklist-update" => Ok(alloc::vec![p("blocklist-size", b.blocklist_update()?.into())]),
            _ => Err(format!("method name not recognized: {}", method)),
        }
    }

    fn all_or(&self, args: &Json, b: &dyn RpcBackend) -> Result<Vec<u64>, String> {
        Ok(match self.resolve_args(args, b)? { Some(v) => v, None => b.torrents(None, false).into_iter().map(|t| t.id).collect() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[derive(Default)]
    struct Mock {
        session: SessionSettings,
        torrents: Vec<TorrentInfo>,
        log: Vec<String>,
        last_add: Option<AddRequest>,
        last_set: Option<TorrentChanges>,
    }

    impl Mock {
        fn new() -> Mock {
            let t1 = TorrentInfo {
                id: 1, hash_string: String::from("0123456789abcdef0123456789abcdef01234567"), name: String::from("Ubuntu \"24.04\" ISO"),
                status: TorrentStatus::Download, percent_done: 0.5, rate_download: 1_000_000, eta: 120, total_size: 6_000_000_000,
                size_when_done: 6_000_000_000, left_until_done: 3_000_000_000, upload_ratio: 0.25, labels: vec![String::from("linux")],
                download_dir: String::from("/data/dl"), pieces: vec![0xF0, 0x01], piece_count: 16,
                files: vec![
                    FileInfo { name: String::from("ubuntu/ubuntu.iso"), length: 6_000_000_000, bytes_completed: 3_000_000_000, wanted: true, priority: 1, begin_piece: 0, end_piece: 15 },
                    FileInfo { name: String::from("ubuntu/SHA256SUMS"), length: 100, bytes_completed: 0, wanted: false, priority: 0, begin_piece: 15, end_piece: 16 },
                ],
                trackers: vec![
                    TrackerInfo { id: 0, tier: 0, announce: String::from("https://t1/announce"), seeder_count: 10, leecher_count: -1, ..TrackerInfo::default() },
                    TrackerInfo { id: 1, tier: 0, announce: String::from("https://t2/announce"), ..TrackerInfo::default() },
                    TrackerInfo { id: 2, tier: 1, announce: String::from("udp://t3:80"), ..TrackerInfo::default() },
                ],
                ..TorrentInfo::default()
            };
            let t2 = TorrentInfo { id: 2, hash_string: String::from("ffffffffffffffffffffffffffffffffffffffff"), name: String::from("Seeded"), status: TorrentStatus::Seed, percent_done: 1.0, is_finished: true, ..TorrentInfo::default() };
            Mock { session: SessionSettings { download_dir: String::from("/data/dl"), peer_port: 51413, seed_ratio_limit: 2.0, dht_enabled: true, ..SessionSettings::default() }, torrents: vec![t1, t2], ..Mock::default() }
        }
    }

    impl RpcBackend for Mock {
        fn session(&self) -> SessionSettings { self.session.clone() }
        fn set_session(&mut self, c: &SessionChanges) -> Result<(), String> { self.session.apply(c); Ok(()) }
        fn stats(&self) -> SessionStats { SessionStats { torrent_count: 2, active_torrent_count: 1, download_speed: 1_000_000, ..SessionStats::default() } }
        fn torrents(&self, ids: Option<&[u64]>, detail: bool) -> Vec<TorrentInfo> {
            self.torrents.iter().filter(|t| ids.map_or(true, |i| i.contains(&t.id))).map(|t| {
                let mut t = t.clone();
                if !detail { t.files.clear(); t.trackers.clear(); }
                t
            }).collect()
        }
        fn recently_active(&self) -> Vec<u64> { vec![1] }
        fn recently_removed(&self) -> Vec<u64> { vec![7] }
        fn add(&mut self, req: AddRequest) -> Result<AddOutcome, String> {
            let dup = matches!(&req.source, AddSource::Magnet(m) if m.contains("0123456789abcdef"));
            self.last_add = Some(req);
            Ok(if dup { AddOutcome::Duplicate { id: 1, name: String::from("Ubuntu"), hash: String::from("0123") } } else { AddOutcome::Added { id: 3, name: String::from("New"), hash: String::from("abcd") } })
        }
        fn action(&mut self, ids: &[u64], a: TorrentAction) -> Result<(), String> { self.log.push(format!("{:?} {:?}", a, ids)); Ok(()) }
        fn set(&mut self, ids: &[u64], c: &TorrentChanges) -> Result<(), String> { self.log.push(format!("set {:?}", ids)); self.last_set = Some(c.clone()); Ok(()) }
        fn set_location(&mut self, ids: &[u64], loc: &str, mv: bool) -> Result<(), String> { self.log.push(format!("loc {:?} {} {}", ids, loc, mv)); Ok(()) }
        fn free_space(&self, path: &str) -> Option<(u64, u64)> { (path == "/data").then_some((1000, 5000)) }
        fn port_test(&mut self) -> Option<bool> { Some(true) }
    }

    fn server(auth: bool) -> RpcServer {
        let cfg = RpcConfig { auth_required: auth, username: String::from("admin"), password: String::from("s3cret"), ..RpcConfig::default() };
        RpcServer::new(cfg, &mut |b: &mut [u8]| for (i, x) in b.iter_mut().enumerate() { *x = i as u8; })
    }

    fn post(s: &RpcServer, body: &str, sid: bool) -> Vec<u8> {
        let mut r = format!("POST /transmission/rpc HTTP/1.1\r\nHost: 127.0.0.1:9091\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", body.len());
        if sid { r.push_str(&format!("X-Transmission-Session-Id: {}\r\n", s.session_id())); }
        r.push_str("\r\n");
        r.push_str(body);
        r.into_bytes()
    }

    fn call(s: &mut RpcServer, m: &mut Mock, body: &str) -> Json {
        match s.handle_http(&post(s, body, true), m) {
            HttpOutcome::Reply { bytes, .. } => {
                let p = find(&bytes, b"\r\n\r\n").unwrap();
                assert!(bytes.starts_with(b"HTTP/1.1 200 OK"), "{}", String::from_utf8_lossy(&bytes));
                json::parse(&bytes[p + 4..]).unwrap()
            }
            HttpOutcome::NeedMore => panic!("need more"),
        }
    }

    fn result(r: &Json) -> &str { r.get("result").unwrap().as_str().unwrap() }
    fn args(r: &Json) -> &Json { r.get("arguments").unwrap() }

    #[test]
    fn json_round_trip_and_escapes() {
        let j = json::parse(br#" {"a":[1,-2.5,1e3,true,false,null],"s":"q\"\\\/\b\f\n\r\t\u00e9\ud83d\ude00\ud800x","e":{}} "#).unwrap();
        assert_eq!(j.get("a").unwrap().as_arr().unwrap()[2].as_u64(), Some(1000));
        assert_eq!(j.get("s").unwrap().as_str(), Some("q\"\\/\u{8}\u{c}\n\r\té😀\u{FFFD}x"));
        assert_eq!(json::parse(j.encode().as_bytes()).unwrap(), j);
        assert_eq!(Json::from("\u{1}\"").encode(), "\"\\u0001\\\"\"");
        assert_eq!(Json::Num(0.5).encode(), "0.5");
        assert_eq!(Json::Num(f64::NAN).encode(), "null");
        assert_eq!(Json::from(6_000_000_000u64).encode(), "6000000000");
        for bad in [&b"{"[..], b"[1,]", b"01", b"1.", b"-", b"\"\\x\"", b"\"\x01\"", b"{1:2}", b"[1] 2", b"\"\\u12g4\"", b"\xff", b"tru", b"\"abc"] {
            assert!(json::parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        assert_eq!(json::parse("[".repeat(1000).as_bytes()), Err(json::Error::Depth));
        let mut seed = 99u32;
        let alpha = b"{}[]\",:0123456789.eE-+tfnul\\ uab";
        for _ in 0..500 {
            let v: Vec<u8> = (0..64).map(|_| { seed = seed.wrapping_mul(1103515245).wrapping_add(12345); alpha[(seed >> 16) as usize % alpha.len()] }).collect();
            let _ = json::parse(&v);
        }
    }

    #[test]
    fn base64_and_ct_eq() {
        for s in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            assert_eq!(base64_decode(base64_encode(s).as_bytes()).unwrap(), s);
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_decode(b"Zm9v\nYmE=").unwrap(), b"fooba");
        assert!(base64_decode(b"Zm=9").is_none());
        assert!(base64_decode(b"Zm*9").is_none());
        assert!(ct_eq(b"abc", b"abc") && !ct_eq(b"abc", b"abd") && !ct_eq(b"abc", b"abcd") && ct_eq(b"", b""));
    }

    #[test]
    fn http_parsing() {
        let req = b"POST /transmission/rpc?x=1 HTTP/1.1\r\nHost: h\r\nContent-Length: 2\r\n\r\n{}GET / HTTP/1.0\r\n\r\n";
        let (r, n) = parse_request(req).unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str(), r.query.as_str(), r.body.as_slice(), r.close), ("POST", "/transmission/rpc", "x=1", &b"{}"[..], false));
        let (r2, n2) = parse_request(&req[n..]).unwrap();
        assert_eq!((r2.method.as_str(), r2.close, n + n2), ("GET", true, req.len()));
        assert_eq!(parse_request(b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\nab"), Err(HttpError::Incomplete));
        assert_eq!(parse_request(b"POST / HTTP/1.1\r\nHost"), Err(HttpError::Incomplete));
        assert_eq!(parse_request(b"POST / HTTP/1.1\r\nContent-Length: 99999999999\r\n\r\n"), Err(HttpError::TooLarge));
        assert_eq!(parse_request(b"GARBAGE\r\n\r\n"), Err(HttpError::BadRequest));
        assert_eq!(parse_request(b"GET / HTTP/1.1\r\nNoColon\r\n\r\n"), Err(HttpError::BadRequest));
        assert_eq!(parse_request(&[b'a'; MAX_HEAD + 10]), Err(HttpError::TooLarge));
        let chunked = b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\nX-T: y\r\n\r\n";
        let (r, n) = parse_request(chunked).unwrap();
        assert_eq!((r.body.as_slice(), n), (&b"Wikipedia"[..], chunked.len()));
        assert_eq!(parse_request(&chunked[..chunked.len() - 3]), Err(HttpError::Incomplete));
        assert_eq!(parse_request(b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n"), Err(HttpError::BadRequest));
        let resp = Response::new(200, "text/plain", b"hi".to_vec()).with("X-Evil", "a\r\nSet-Cookie: x").to_bytes();
        assert_eq!(resp, b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Evil: aSet-Cookie: x\r\nContent-Length: 2\r\n\r\nhi".to_vec());
    }

    #[test]
    fn csrf_session_id_protocol() {
        let mut s = server(false);
        let mut m = Mock::new();
        assert_eq!(s.session_id().len(), 48);
        let body = r#"{"method":"session-get","tag":1}"#;
        let HttpOutcome::Reply { bytes, consumed, close } = s.handle_http(&post(&s, body, false), &mut m) else { panic!() };
        assert!(bytes.starts_with(b"HTTP/1.1 409 Conflict\r\n"));
        let txt = String::from_utf8(bytes).unwrap();
        assert!(txt.contains(&format!("X-Transmission-Session-Id: {}\r\n", s.session_id())));
        assert!(!close && consumed > 0);
        let r = call(&mut s, &mut m, body);
        assert_eq!(result(&r), "success");
        assert_eq!(r.get("tag").unwrap().as_u64(), Some(1));
        let a = args(&r);
        assert_eq!(a.get("rpc-version").unwrap().as_u64(), Some(RPC_VERSION));
        assert_eq!(a.get("download-dir").unwrap().as_str(), Some("/data/dl"));
        assert_eq!(a.get("peer-port").unwrap().as_u64(), Some(51413));
        assert_eq!(a.get("seedRatioLimit").unwrap().as_f64(), Some(2.0));
        assert_eq!(a.get("units").unwrap().get("speed-bytes").unwrap().as_u64(), Some(1000));
        // A rotated id refuses the old one.
        let old = post(&s, body, true);
        s.rotate_session_id(&mut |b: &mut [u8]| b.fill(3));
        let HttpOutcome::Reply { bytes, .. } = s.handle_http(&old, &mut m) else { panic!() };
        assert!(bytes.starts_with(b"HTTP/1.1 409"));
        let full = post(&s, body, true);
        assert_eq!(s.handle_http(&full[..full.len() - 3], &mut m), HttpOutcome::NeedMore);
        let get = format!("GET /transmission/rpc HTTP/1.1\r\nX-Transmission-Session-Id: {}\r\n\r\n", s.session_id());
        let HttpOutcome::Reply { bytes, .. } = s.handle_http(get.as_bytes(), &mut m) else { panic!() };
        assert!(bytes.starts_with(b"HTTP/1.1 405"));
    }

    #[test]
    fn basic_auth_and_host_whitelist() {
        let mut s = server(true);
        let mut m = Mock::new();
        let mk = |auth: Option<&str>, host: &str| {
            let mut r = format!("GET /swingers/ HTTP/1.1\r\nHost: {}\r\n", host);
            if let Some(a) = auth { r.push_str(&format!("Authorization: Basic {}\r\n", base64_encode(a.as_bytes()))); }
            r.push_str("\r\n");
            r
        };
        let status = |s: &mut RpcServer, m: &mut Mock, req: String| match s.handle_http(req.as_bytes(), m) {
            HttpOutcome::Reply { bytes, .. } => String::from_utf8_lossy(&bytes[9..12]).into_owned(),
            _ => panic!(),
        };
        assert_eq!(status(&mut s, &mut m, mk(None, "localhost:9091")), "401");
        assert_eq!(status(&mut s, &mut m, mk(Some("admin:wrong"), "localhost:9091")), "401");
        assert_eq!(status(&mut s, &mut m, mk(Some("admin"), "localhost:9091")), "401");
        assert_eq!(status(&mut s, &mut m, mk(Some("admin:s3cret"), "localhost:9091")), "200");
        assert_eq!(status(&mut s, &mut m, mk(Some("admin:s3cret"), "192.168.1.5:9091")), "200");
        assert_eq!(status(&mut s, &mut m, mk(Some("admin:s3cret"), "[::1]:9091")), "200");
        assert_eq!(status(&mut s, &mut m, mk(Some("admin:s3cret"), "evil.example:9091")), "421");
        s.config.host_whitelist = vec![String::from("*.home.arpa")];
        assert_eq!(status(&mut s, &mut m, mk(Some("admin:s3cret"), "nas.home.arpa")), "200");
        s.config.host_whitelist_enabled = false;
        assert_eq!(status(&mut s, &mut m, mk(Some("admin:s3cret"), "evil.example")), "200");
        let HttpOutcome::Reply { bytes, .. } = s.handle_http(mk(None, "localhost").as_bytes(), &mut m) else { panic!() };
        assert!(String::from_utf8_lossy(&bytes).contains("WWW-Authenticate: Basic realm=\"Swingers\""));
        assert_eq!(status(&mut s, &mut m, mk(Some("admin:s3cret"), "localhost").replace("/swingers/", "/nope")), "404");
    }

    #[test]
    fn transmission_remote_list() {
        let mut s = server(false);
        let mut m = Mock::new();
        // What `transmission-remote -l` sends.
        let r = call(&mut s, &mut m, r#"{"arguments":{"fields":["error","errorString","eta","id","isFinished","leftUntilDone","name","peersGettingFromUs","peersSendingToUs","rateDownload","rateUpload","sizeWhenDone","status","uploadRatio"]},"method":"torrent-get","tag":4}"#);
        let ts = args(&r).get("torrents").unwrap().as_arr().unwrap();
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].get("name").unwrap().as_str(), Some("Ubuntu \"24.04\" ISO"));
        assert_eq!(ts[0].get("status").unwrap().as_u64(), Some(4));
        assert_eq!(ts[0].get("eta").unwrap().as_i64(), Some(120));
        assert_eq!(ts[0].get("uploadRatio").unwrap().as_f64(), Some(0.25));
        assert_eq!(ts[0].as_obj().unwrap().len(), 14);
        assert!(ts[1].get("isFinished").unwrap().as_bool().unwrap());
    }

    #[test]
    fn details_table_format_and_recently_active() {
        let mut s = server(false);
        let mut m = Mock::new();
        let r = call(&mut s, &mut m, r#"{"method":"torrent-get","arguments":{"format":"table","ids":["0123456789ABCDEF0123456789ABCDEF01234567"],"fields":["id","files","fileStats","priorities","wanted","trackerStats","trackerList","pieces","labels","bogusField"]}}"#);
        let rows = args(&r).get("torrents").unwrap().as_arr().unwrap();
        assert_eq!(rows.len(), 2);
        let head: Vec<&str> = rows[0].as_arr().unwrap().iter().map(|j| j.as_str().unwrap()).collect();
        assert_eq!(head, vec!["id", "files", "fileStats", "priorities", "wanted", "trackerStats", "trackerList", "pieces", "labels"]);
        let row = rows[1].as_arr().unwrap();
        assert_eq!(row[0].as_u64(), Some(1));
        assert_eq!(row[1].as_arr().unwrap()[1].get("name").unwrap().as_str(), Some("ubuntu/SHA256SUMS"));
        assert_eq!(row[2].as_arr().unwrap()[1].get("wanted").unwrap().as_bool(), Some(false));
        assert_eq!(row[3].as_arr().unwrap()[0].as_i64(), Some(1));
        assert_eq!(row[4].as_arr().unwrap()[1].as_u64(), Some(0));
        assert_eq!(row[5].as_arr().unwrap()[0].get("seederCount").unwrap().as_i64(), Some(10));
        assert_eq!(row[6].as_str(), Some("https://t1/announce\nhttps://t2/announce\n\nudp://t3:80"));
        assert_eq!(row[7].as_str(), Some("8AE="));
        let r = call(&mut s, &mut m, r#"{"method":"torrent-get","arguments":{"ids":"recently-active","fields":["id","name"]},"tag":9}"#);
        assert_eq!(args(&r).get("torrents").unwrap().as_arr().unwrap().len(), 1);
        assert_eq!(args(&r).get("removed").unwrap().as_arr().unwrap()[0].as_u64(), Some(7));
        let r = call(&mut s, &mut m, r#"{"method":"torrent-get","arguments":{"ids":2,"fields":["name"]}}"#);
        assert_eq!(args(&r).get("torrents").unwrap().as_arr().unwrap()[0].get("name").unwrap().as_str(), Some("Seeded"));
    }

    #[test]
    fn add_variants() {
        let mut s = server(false);
        let mut m = Mock::new();
        let meta = base64_encode(b"d4:infod4:name1:xee");
        let r = call(&mut s, &mut m, &format!(r#"{{"method":"torrent-add","arguments":{{"metainfo":"{}","paused":true,"download-dir":"/data/x","labels":["a","b"],"files-unwanted":[1],"priority-high":[0]}}}}"#, meta));
        let added = args(&r).get("torrent-added").unwrap();
        assert_eq!((added.get("id").unwrap().as_u64(), added.get("hashString").unwrap().as_str()), (Some(3), Some("abcd")));
        let a = m.last_add.clone().unwrap();
        assert_eq!(a.source, AddSource::Metainfo(b"d4:infod4:name1:xee".to_vec()));
        assert_eq!((a.paused, a.download_dir.as_deref(), a.labels.len(), a.files_unwanted.clone(), a.priority_high.clone()), (Some(true), Some("/data/x"), 2, vec![1], vec![0]));
        let r = call(&mut s, &mut m, r#"{"method":"torrent-add","arguments":{"filename":"magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=u"}}"#);
        assert!(args(&r).get("torrent-duplicate").is_some());
        call(&mut s, &mut m, r#"{"method":"torrent-add","arguments":{"filename":"https://ex.org/a.torrent","cookies":"k=v"}}"#);
        assert_eq!(m.last_add.as_ref().unwrap().source, AddSource::Url(String::from("https://ex.org/a.torrent")));
        call(&mut s, &mut m, r#"{"method":"torrent-add","arguments":{"filename":"ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD"}}"#);
        assert_eq!(m.last_add.as_ref().unwrap().source, AddSource::Magnet(String::from("magnet:?xt=urn:btih:ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD")));
        call(&mut s, &mut m, r#"{"method":"torrent-add","arguments":{"filename":"/home/u/x.torrent"}}"#);
        assert_eq!(m.last_add.as_ref().unwrap().source, AddSource::Path(String::from("/home/u/x.torrent")));
        assert_eq!(result(&call(&mut s, &mut m, r#"{"method":"torrent-add","arguments":{}}"#)), "no filename or metainfo specified");
        assert_eq!(result(&call(&mut s, &mut m, r#"{"method":"torrent-add","arguments":{"metainfo":"!!!"}}"#)), "invalid or corrupt torrent file");
    }

    #[test]
    fn actions_set_and_misc() {
        let mut s = server(false);
        let mut m = Mock::new();
        call(&mut s, &mut m, r#"{"method":"torrent-stop","arguments":{"ids":[1,"ffffffffffffffffffffffffffffffffffffffff"]}}"#);
        call(&mut s, &mut m, r#"{"method":"torrent-start-now"}"#);
        call(&mut s, &mut m, r#"{"method":"torrent-remove","arguments":{"ids":[2],"delete-local-data":true}}"#);
        call(&mut s, &mut m, r#"{"method":"queue-move-top","arguments":{"ids":[2]}}"#);
        call(&mut s, &mut m, r#"{"method":"torrent-reannounce","arguments":{"ids":"recently-active"}}"#);
        call(&mut s, &mut m, r#"{"method":"torrent-verify","arguments":{"ids":[]}}"#);
        assert_eq!(m.log, vec!["Stop [1, 2]", "StartNow [1, 2]", "Remove { delete_local_data: true } [2]", "QueueTop [2]", "Reannounce [1]", "Verify []"]);
        call(&mut s, &mut m, r#"{"method":"torrent-set","arguments":{"ids":[1],"files-wanted":[1],"files_unwanted":[0],"priority-low":[],"labels":["x"],"downloadLimit":500,"download-limited":true,"seedRatioLimit":1.5,"seed_ratio_mode":1,"queuePosition":0,"uploadLimited":0}}"#);
        let c = m.last_set.clone().unwrap();
        assert_eq!((c.files_wanted, c.files_unwanted, c.priority_low), (Some(vec![1]), Some(vec![0]), Some(vec![])));
        assert_eq!((c.download_limit, c.download_limited, c.seed_ratio_limit, c.seed_ratio_mode), (Some(500), Some(true), Some(1.5), Some(1)));
        assert_eq!((c.queue_position, c.upload_limited, c.labels), (Some(0), Some(false), Some(vec![String::from("x")])));
        assert!(c.upload_limit.is_none());
        call(&mut s, &mut m, r#"{"method":"torrent-set-location","arguments":{"ids":[1],"location":"/new","move":true}}"#);
        assert_eq!(m.log.last().unwrap(), "loc [1] /new true");
        call(&mut s, &mut m, r#"{"method":"session-set","arguments":{"speed-limit-down":800,"speed-limit-down-enabled":true,"alt-speed-time-day":65,"encryption":"required","seedRatioLimit":3}}"#);
        assert_eq!((m.session.speed_limit_down, m.session.speed_limit_down_enabled, m.session.alt_speed_time_day, m.session.encryption.as_str(), m.session.seed_ratio_limit), (800, true, 65, "required", 3.0));
        let r = call(&mut s, &mut m, r#"{"method":"session-get","arguments":{"fields":["speed-limit-down","version"]}}"#);
        assert_eq!(args(&r).as_obj().unwrap().len(), 2);
        let r = call(&mut s, &mut m, r#"{"method":"session-stats"}"#);
        assert_eq!(args(&r).get("torrentCount").unwrap().as_u64(), Some(2));
        assert!(args(&r).get("cumulative-stats").unwrap().get("secondsActive").is_some());
        let r = call(&mut s, &mut m, r#"{"method":"free-space","arguments":{"path":"/data"}}"#);
        assert_eq!(args(&r).get("size-bytes").unwrap().as_u64(), Some(1000));
        assert!(result(&call(&mut s, &mut m, r#"{"method":"free-space","arguments":{"path":"/nope"}}"#)).starts_with("free-space"));
        let r = call(&mut s, &mut m, r#"{"method":"port-test","arguments":{"ipProtocol":"ipv4"}}"#);
        assert_eq!(args(&r).get("port-is-open").unwrap().as_bool(), Some(true));
        let r = call(&mut s, &mut m, r#"{"method":"blocklist-update"}"#);
        assert_eq!(args(&r).get("blocklist-size").unwrap().as_u64(), Some(0));
        assert_eq!(result(&call(&mut s, &mut m, r#"{"method":"torrent-rename-path","arguments":{"ids":[1],"path":"a","name":"b"}}"#)), "not supported");
        let r = call(&mut s, &mut m, r#"{"method":"frobnicate","tag":"t"}"#);
        assert!(result(&r).starts_with("method name not recognized"));
        assert_eq!(r.get("tag").unwrap().as_str(), Some("t"));
        assert_eq!(result(&call(&mut s, &mut m, r#"{"method":"torrent-stop","arguments":{"ids":[{"x":1}]}}"#)), "invalid ids");
    }

    #[test]
    fn bad_bodies_and_jsonrpc() {
        let mut s = server(false);
        let mut m = Mock::new();
        for body in ["", "nope", "[1,2]", "{\"method\":"] {
            let HttpOutcome::Reply { bytes, .. } = s.handle_http(&post(&s, body, true), &mut m) else { panic!() };
            assert!(bytes.starts_with(b"HTTP/1.1 400"), "{}", body);
        }
        let r = call(&mut s, &mut m, r#"{"jsonrpc":"2.0","method":"torrent_get","params":{"ids":[1],"fields":["hash_string","percent_done","rate_download"]},"id":42}"#);
        assert_eq!(r.get("id").unwrap().as_u64(), Some(42));
        let t = &r.get("result").unwrap().get("torrents").unwrap().as_arr().unwrap()[0];
        assert_eq!(t.get("percent_done").unwrap().as_f64(), Some(0.5));
        assert!(t.get("hash_string").is_some());
        let r = call(&mut s, &mut m, r#"{"jsonrpc":"2.0","method":"nope","id":1}"#);
        assert_eq!(r.get("error").unwrap().get("code").unwrap().as_i64(), Some(-32601));
        let HttpOutcome::Reply { bytes, .. } = s.handle_http(b"GET /transmission/web/ HTTP/1.1\r\nHost: localhost\r\n\r\n", &mut m) else { panic!() };
        assert!(bytes.starts_with(b"HTTP/1.1 200") && bytes.windows(8).any(|w| w == b"Swingers"));
        let HttpOutcome::Reply { close, consumed, .. } = s.handle_http(b"BAD\r\n\r\nmore", &mut m) else { panic!() };
        assert!(close && consumed == 11);
    }
}
