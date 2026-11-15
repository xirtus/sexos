// SPDX-License-Identifier: GPL-3.0-only
//! Just enough HTTP/1.1 for tiles and JSON: `http://` URLs, a GET request, and a
//! response read whole (Content-Length, chunked, or to the close).
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url { pub host: String, pub port: u16, pub path: String }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error { NotHttp, BadUrl, BadResponse, Truncated }

pub fn parse_url(url: &str) -> Result<Url, Error> {
    let rest = url.strip_prefix("http://").ok_or(if url.starts_with("https://") { Error::NotHttp } else { Error::BadUrl })?;
    let (authority, path) = match rest.find('/') { Some(i) => (&rest[..i], &rest[i..]), None => (rest, "/") };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() => (h, p.parse().map_err(|_| Error::BadUrl)?),
        _ => (authority, 80),
    };
    if host.is_empty() || host.contains(['@', ' ']) { return Err(Error::BadUrl); }
    Ok(Url { host: host.to_string(), port, path: path.to_string() })
}

pub fn request(u: &Url) -> String {
    let host = if u.port == 80 { u.host.clone() } else { format!("{}:{}", u.host, u.port) };
    format!("GET {} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: Rendezvous/0.1 (SexOS)\r\nAccept: */*\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n", u.path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response { pub status: u16, pub headers: Vec<(String, String)>, pub body: Vec<u8> }

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> { self.headers.iter().find(|h| h.0.eq_ignore_ascii_case(name)).map(|h| h.1.as_str()) }
}

/// Parses a whole response (the connection has closed). A gzip Content-Encoding is
/// left for the caller ([`crate::inflate`]).
pub fn parse_response(raw: &[u8]) -> Result<Response, Error> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or(Error::Truncated)?;
    let head = core::str::from_utf8(&raw[..end]).map_err(|_| Error::BadResponse)?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or(Error::BadResponse)?;
    let mut parts = status_line.splitn(3, ' ');
    if !parts.next().unwrap_or("").starts_with("HTTP/1.") { return Err(Error::BadResponse); }
    let status = parts.next().and_then(|s| s.parse().ok()).ok_or(Error::BadResponse)?;
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
    let rest = &raw[end + 4..];
    let mut r = Response { status, headers, body: Vec::new() };
    if r.header("Transfer-Encoding").is_some_and(|t| t.eq_ignore_ascii_case("chunked")) {
        r.body = dechunk(rest)?;
    } else if let Some(n) = r.header("Content-Length").and_then(|n| n.parse::<usize>().ok()) {
        if rest.len() < n { return Err(Error::Truncated); }
        r.body = rest[..n].to_vec();
    } else {
        r.body = rest.to_vec();
    }
    Ok(r)
}

fn dechunk(mut b: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n").ok_or(Error::Truncated)?;
        let size_text = core::str::from_utf8(&b[..eol]).map_err(|_| Error::BadResponse)?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| Error::BadResponse)?;
        b = &b[eol + 2..];
        if size == 0 { return Ok(out); }
        if b.len() < size + 2 { return Err(Error::Truncated); }
        out.extend_from_slice(&b[..size]);
        b = &b[size + 2..];
    }
}
