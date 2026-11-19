// SPDX-License-Identifier: GPL-3.0-only
//! The HTTP/1.1 Needle needs: one request per connection (`Connection: close`),
//! bodies by `Content-Length`, `chunked`, or the end of the connection.
use crate::url::Url;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A request's bytes. `body` with a `content_type` gets a Content-Length.
pub fn request(method: &str, url: &Url, content_type: Option<&str>, body: &[u8]) -> Vec<u8> {
    let mut head = format!("{method} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Needle/0.1 (SexOS)\r\nConnection: close\r\n", url.path, url.host_header());
    if let Some(t) = content_type { head.push_str(&format!("Content-Type: {t}\r\n")); }
    if content_type.is_some() || !body.is_empty() || method == "POST" || method == "PUT" {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Malformed, Truncated }

/// A whole response from `data`, or `Ok(None)` while more bytes are needed.
/// `eof` says the connection has closed (which ends a body with no length).
pub fn parse_response(data: &[u8], eof: bool) -> Result<Option<Response>, Error> {
    let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") else {
        return if eof { Err(Error::Truncated) } else { Ok(None) };
    };
    let head = core::str::from_utf8(&data[..end]).map_err(|_| Error::Malformed)?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or(Error::Malformed)?;
    let mut parts = status_line.splitn(3, ' ');
    if !parts.next().is_some_and(|v| v.starts_with("HTTP/1.")) { return Err(Error::Malformed); }
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or(Error::Malformed)?;
    let mut headers = Vec::new();
    for l in lines {
        let (k, v) = l.split_once(':').ok_or(Error::Malformed)?;
        headers.push((k.trim().to_string(), v.trim().to_string()));
    }
    let mut r = Response { status, headers, body: Vec::new() };
    let rest = &data[end + 4..];
    // 1xx (100 Continue) carries no body: skip it and read what follows.
    if (100..200).contains(&status) { return parse_response(rest, eof); }
    if r.header("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
        let mut at = 0;
        loop {
            let Some(nl) = rest[at..].windows(2).position(|w| w == b"\r\n") else { return if eof { Err(Error::Truncated) } else { Ok(None) } };
            let size_text = core::str::from_utf8(&rest[at..at + nl]).map_err(|_| Error::Malformed)?;
            let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| Error::Malformed)?;
            at += nl + 2;
            if size == 0 { break; }
            if rest.len() < at + size + 2 { return if eof { Err(Error::Truncated) } else { Ok(None) }; }
            r.body.extend_from_slice(&rest[at..at + size]);
            at += size + 2;
        }
        return Ok(Some(r));
    }
    if let Some(n) = r.header("content-length") {
        let n: usize = n.parse().map_err(|_| Error::Malformed)?;
        if rest.len() < n { return if eof { Err(Error::Truncated) } else { Ok(None) }; }
        r.body = rest[..n].to_vec();
        return Ok(Some(r));
    }
    if status == 204 || status == 304 { return Ok(Some(r)); }
    if !eof { return Ok(None); }
    r.body = rest.to_vec();
    Ok(Some(r))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bodies() {
        let u = Url::parse("http://h:8080/eSCL/ScanJobs").unwrap();
        assert_eq!(request("POST", &u, Some("text/xml"), b"<x/>"),
            b"POST /eSCL/ScanJobs HTTP/1.1\r\nHost: h:8080\r\nUser-Agent: Needle/0.1 (SexOS)\r\nConnection: close\r\nContent-Type: text/xml\r\nContent-Length: 4\r\n\r\n<x/>".to_vec());
        let full = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc";
        assert_eq!(parse_response(&full[..full.len() - 1], false), Ok(None));
        assert_eq!(parse_response(full, false).unwrap().unwrap().body, b"abc");
        let ch = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n";
        assert_eq!(parse_response(ch, false).unwrap().unwrap().body, b"abcde");
        let close = b"HTTP/1.0 201 Created\r\nLocation: /eSCL/ScanJobs/7\r\n\r\nxy";
        assert_eq!(parse_response(close, false), Ok(None));
        let r = parse_response(close, true).unwrap().unwrap();
        assert_eq!((r.status, r.header("location"), r.body.as_slice()), (201, Some("/eSCL/ScanJobs/7"), &b"xy"[..]));
        let cont = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(parse_response(cont, false).unwrap().unwrap().status, 404);
    }
}
