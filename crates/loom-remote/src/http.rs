// SPDX-License-Identifier: GPL-3.0-only
//! The HTTP exchange WebDAV and S3 run on, with two carriers:
//! [`H2`] — sex-http's HTTP/2 client (chosen when TLS ALPN says `h2`), and
//! [`Http1`] — a small HTTP/1.1 client (keep-alive, Content-Length or chunked bodies,
//! streamed uploads) for servers that only speak 1.1, as S3 and many WebDAV hosts do.
//! sex-http's own HTTP/1.1 codec reads fixed boot resources only, so 1.1 lives here.
use crate::{Connect, Error, Result, Source, Stream};
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};

pub struct Request<'a> {
    pub method: &'a str,
    /// Origin-form target: the encoded path and query.
    pub target: &'a str,
    /// Extra headers (names in lower case). Host, Content-Length and framing are added.
    pub headers: Vec<(String, String)>,
}

pub enum Body<'a> {
    Empty,
    Bytes(&'a [u8]),
    /// Streamed from `src`; `len` when known (else HTTP/1.1 sends it chunked).
    Stream { src: &'a mut dyn Source, len: Option<u64> },
}

#[derive(Clone, Debug, Default)]
pub struct Response {
    pub status: u16,
    /// Names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The body went past the caller's limit; the rest was dropped (and the connection with it).
    pub truncated: bool,
}
impl Response {
    pub fn header(&self, name: &str) -> Option<&str> { self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()) }
    pub fn ok(&self) -> bool { (200..300).contains(&self.status) }
    pub fn text(&self) -> &str { core::str::from_utf8(&self.body).unwrap_or("") }
}

pub trait Http {
    /// `host[:port]`, as sent in Host / :authority (and signed by SigV4).
    fn authority(&self) -> &str;
    /// Sends one request; keeps at most `limit` body bytes of the response.
    fn send(&mut self, req: &Request<'_>, body: Body<'_>, limit: usize) -> Result<Response>;
}

const HEAD_MAX: usize = 64 << 10;
const CHUNK: usize = 64 << 10;

/// HTTP/1.1 over connections from `C`, one at a time, reused while the server allows.
pub struct Http1<C: Connect> {
    connect: C,
    authority: String,
    conn: Option<Box<dyn Stream>>,
    buf: Vec<u8>,
}

impl<C: Connect> Http1<C> {
    pub fn new(connect: C, authority: &str) -> Self { Self { connect, authority: authority.into(), conn: None, buf: Vec::new() } }

    fn fill(&mut self) -> Result<usize> {
        let conn = self.conn.as_mut().ok_or_else(|| Error::Transport("closed".into()))?;
        let mut tmp = vec![0u8; CHUNK];
        let n = conn.read(&mut tmp)?;
        self.buf.extend_from_slice(&tmp[..n]);
        Ok(n)
    }
    fn line(&mut self) -> Result<String> {
        loop {
            if let Some(p) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let l = String::from_utf8_lossy(&self.buf[..p]).into_owned();
                self.buf.drain(..p + 2);
                return Ok(l);
            }
            if self.buf.len() > HEAD_MAX { return Err(Error::Protocol("header line too long")); }
            if self.fill()? == 0 { return Err(Error::Transport("connection closed".into())); }
        }
    }
    /// Moves up to `n` body bytes to `out` (kept while under `limit`).
    fn body_bytes(&mut self, mut n: u64, out: &mut Vec<u8>, limit: usize, truncated: &mut bool) -> Result<()> {
        while n > 0 {
            if self.buf.is_empty() && self.fill()? == 0 { return Err(Error::Transport("body cut short".into())); }
            let k = (self.buf.len() as u64).min(n) as usize;
            keep(out, &self.buf[..k], limit, truncated);
            self.buf.drain(..k);
            n -= k as u64;
            if *truncated { return Ok(()); }
        }
        Ok(())
    }

    fn write_request(&mut self, req: &Request<'_>, body: &mut Body<'_>) -> Result<()> {
        let mut head = format!("{} {} HTTP/1.1\r\nhost: {}\r\n", req.method, req.target, self.authority);
        for (n, v) in &req.headers { head.push_str(n); head.push_str(": "); head.push_str(v); head.push_str("\r\n"); }
        match body {
            Body::Empty => if matches!(req.method, "PUT" | "POST") { head.push_str("content-length: 0\r\n"); },
            Body::Bytes(b) => head.push_str(&format!("content-length: {}\r\n", b.len())),
            Body::Stream { len: Some(n), .. } => head.push_str(&format!("content-length: {n}\r\n")),
            Body::Stream { len: None, .. } => head.push_str("transfer-encoding: chunked\r\n"),
        }
        head.push_str("\r\n");
        let conn = self.conn.as_mut().ok_or_else(|| Error::Transport("closed".into()))?;
        match body {
            Body::Bytes(b) if b.len() <= CHUNK => { let mut m = head.into_bytes(); m.extend_from_slice(b); conn.write_all(&m)?; }
            Body::Bytes(b) => { conn.write_all(head.as_bytes())?; conn.write_all(b)?; }
            Body::Empty => conn.write_all(head.as_bytes())?,
            Body::Stream { src, len } => {
                conn.write_all(head.as_bytes())?;
                let mut tmp = vec![0u8; CHUNK];
                let mut sent = 0u64;
                loop {
                    let want = match len { Some(n) => ((*n - sent) as usize).min(CHUNK), None => CHUNK };
                    if want == 0 { break; }
                    let k = crate::read_full(*src, &mut tmp[..want])?;
                    if k == 0 { break; }
                    sent += k as u64;
                    if len.is_none() { conn.write_all(format!("{k:x}\r\n").as_bytes())?; }
                    conn.write_all(&tmp[..k])?;
                    if len.is_none() { conn.write_all(b"\r\n")?; }
                }
                match len {
                    // The server waits for bytes we do not have: this connection is useless.
                    Some(n) if sent != *n => return Err(Error::Source),
                    Some(_) => {}
                    None => conn.write_all(b"0\r\n\r\n")?,
                }
            }
        }
        Ok(())
    }

    fn read_response(&mut self, method: &str, limit: usize) -> Result<(Response, bool)> {
        let (mut r, mut close) = loop {
            let status_line = self.line()?;
            let mut p = status_line.splitn(3, ' ');
            let version = p.next().unwrap_or("");
            if !version.starts_with("HTTP/1.") { return Err(Error::Protocol("not an HTTP/1 response")); }
            let status: u16 = p.next().and_then(|s| s.parse().ok()).ok_or(Error::Protocol("bad status"))?;
            let mut headers = Vec::new();
            let mut size = 0;
            loop {
                let l = self.line()?;
                if l.is_empty() { break; }
                size += l.len();
                if size > HEAD_MAX { return Err(Error::Protocol("headers too long")); }
                let Some((n, v)) = l.split_once(':') else { return Err(Error::Protocol("bad header")) };
                headers.push((n.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
            if (100..200).contains(&status) { continue; }
            let r = Response { status, headers, body: Vec::new(), truncated: false };
            let close = version == "HTTP/1.0" && r.header("connection").is_none_or(|c| !c.eq_ignore_ascii_case("keep-alive"))
                || r.header("connection").is_some_and(|c| c.eq_ignore_ascii_case("close"));
            break (r, close);
        };
        if method == "HEAD" || r.status == 204 || r.status == 304 { return Ok((r, close)); }
        let mut body = Vec::new();
        let mut truncated = false;
        if r.header("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
            loop {
                let l = self.line()?;
                let n = u64::from_str_radix(l.split(';').next().unwrap_or("").trim(), 16).map_err(|_| Error::Protocol("bad chunk size"))?;
                if n == 0 { while !self.line()?.is_empty() {} break; }
                self.body_bytes(n, &mut body, limit, &mut truncated)?;
                if truncated { break; }
                if !self.line()?.is_empty() { return Err(Error::Protocol("bad chunk end")); }
            }
        } else if let Some(n) = r.header("content-length") {
            let n: u64 = n.parse().map_err(|_| Error::Protocol("bad content-length"))?;
            self.body_bytes(n, &mut body, limit, &mut truncated)?;
        } else {
            close = true;
            loop {
                if !self.buf.is_empty() { let b = core::mem::take(&mut self.buf); keep(&mut body, &b, limit, &mut truncated); if truncated { break; } }
                if self.fill()? == 0 { break; }
            }
        }
        r.body = body;
        r.truncated = truncated;
        Ok((r, close || truncated))
    }
}

fn keep(out: &mut Vec<u8>, b: &[u8], limit: usize, truncated: &mut bool) {
    let room = limit.saturating_sub(out.len());
    if b.len() > room { out.extend_from_slice(&b[..room]); *truncated = true; } else { out.extend_from_slice(b); }
}

impl<C: Connect> Http for Http1<C> {
    fn authority(&self) -> &str { &self.authority }
    fn send(&mut self, req: &Request<'_>, mut body: Body<'_>, limit: usize) -> Result<Response> {
        // A kept-alive connection may have been closed by the server: retry once on a
        // fresh one when nothing of the response came and the body can be sent again.
        let reused = self.conn.is_some();
        let replayable = !matches!(body, Body::Stream { .. });
        for attempt in 0..2 {
            if self.conn.is_none() { self.conn = Some(self.connect.connect()?); self.buf.clear(); }
            let r = self.write_request(req, &mut body).and_then(|_| self.read_response(req.method, limit));
            match r {
                Ok((resp, close)) => { if close { self.conn = None; } return Ok(resp); }
                Err(e) => {
                    self.conn = None;
                    let early = matches!(e, Error::Transport(_)) && self.buf.is_empty();
                    if !(attempt == 0 && reused && replayable && early) { return Err(e); }
                }
            }
        }
        Err(Error::Transport("retry failed".into()))
    }
}

/// HTTP/2 over one connection with sex-http's client. Request bodies are gathered
/// before sending (sex-http takes a whole body); keep uploads chunked above this.
pub struct H2 {
    stream: Box<dyn Stream>,
    client: sex_http::h2::Client,
    authority: String,
    scheme: String,
}

impl H2 {
    /// `stream` is connected (TLS with ALPN `h2`, or cleartext by prior knowledge).
    pub fn new(stream: Box<dyn Stream>, scheme: &str, authority: &str) -> Self {
        Self { stream, client: sex_http::h2::Client::new(), authority: authority.into(), scheme: scheme.into() }
    }
    fn flush(&mut self) -> Result<()> { let o = self.client.output(); if !o.is_empty() { self.stream.write_all(&o)?; } Ok(()) }
}

impl Http for H2 {
    fn authority(&self) -> &str { &self.authority }
    fn send(&mut self, req: &Request<'_>, body: Body<'_>, limit: usize) -> Result<Response> {
        use sex_http::h2::{Error as E, Event};
        let owned;
        let data: Option<&[u8]> = match body {
            Body::Empty => None,
            Body::Bytes(b) => Some(b),
            Body::Stream { src, len } => {
                let mut v = Vec::new();
                let mut tmp = vec![0u8; CHUNK];
                loop { let k = src.read(&mut tmp)?; if k == 0 { break; } v.extend_from_slice(&tmp[..k]); }
                if len.is_some_and(|n| n != v.len() as u64) { return Err(Error::Source); }
                owned = v;
                Some(&owned)
            }
        };
        let mut headers: Vec<(String, String)> = req.headers.iter().map(|(n, v)| (n.to_ascii_lowercase(), v.clone())).collect();
        if let Some(d) = data { headers.push(("content-length".into(), d.len().to_string())); }
        let hs: Vec<(&[u8], &[u8])> = headers.iter().map(|(n, v)| (n.as_bytes(), v.as_bytes())).collect();
        let id = self.client.request(req.method.as_bytes(), self.scheme.as_bytes(), self.authority.as_bytes(), req.target.as_bytes(), &hs, data)
            .map_err(|e| match e { E::GoingAway => Error::Transport("server going away".into()), _ => Error::Transport("connection broken".into()) })?;
        let mut r = Response::default();
        let mut buf = vec![0u8; CHUNK];
        loop {
            self.flush()?;
            while let Some(ev) = self.client.event() {
                match ev {
                    Event::Headers { stream, status, headers, trailers } if stream == id => {
                        if !trailers && status >= 200 {
                            r.status = status;
                            r.headers = headers.into_iter().map(|(n, v)| (String::from_utf8_lossy(&n).into_owned(), String::from_utf8_lossy(&v).into_owned())).collect();
                        }
                    }
                    Event::Data { stream, bytes } if stream == id => {
                        keep(&mut r.body, &bytes, limit, &mut r.truncated);
                        if r.truncated { self.client.cancel(id); self.flush()?; return Ok(r); }
                    }
                    Event::End { stream } if stream == id => { self.flush()?; return Ok(r); }
                    Event::Reset { stream, code } if stream == id => return Err(Error::Transport(format!("stream reset ({code})"))),
                    Event::GoAway { last, code } if last < id => return Err(Error::Transport(format!("server went away ({code})"))),
                    _ => {}
                }
            }
            let n = self.stream.read(&mut buf)?;
            if n == 0 { return Err(Error::Transport("connection closed".into())); }
            self.client.receive(&buf[..n]).map_err(|_| Error::Protocol("HTTP/2"))?;
        }
    }
}
