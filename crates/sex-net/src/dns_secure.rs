// SPDX-License-Identifier: GPL-3.0-only
//! Encrypted name lookups: DNS over TLS (RFC 7858, port 853) and DNS over HTTPS
//! (RFC 8484, POST of `application/dns-message`). This module turns a lookup into the
//! bytes to send inside an already-verified TLS session and reads the answers back;
//! the service owns the flow and the session. When a secure resolver is configured,
//! lookups never fall back to plain DNS (strict mode, RFC 8310 section 5), so the
//! network can't see or change the names asked.

use crate::dns_wire;
use crate::Answers;
use alloc::vec::Vec;

/// Who answers encrypted lookups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The resolver the network gave, in plain DNS.
    Plain,
    /// DNS over TLS to `address` port 853; the certificate must name `name`.
    Tls { address: [u8; 4], name: &'static str },
    /// DNS over HTTPS to `address` port 443, `host` and `path` (usually "/dns-query").
    Https { address: [u8; 4], host: &'static str, path: &'static str },
}

/// Well-known public resolvers a person can pick in Settings.
pub const PRESETS: &[(&str, Mode, Mode)] = &[
    ("Cloudflare", Mode::Tls { address: [1, 1, 1, 1], name: "one.one.one.one" }, Mode::Https { address: [1, 1, 1, 1], host: "cloudflare-dns.com", path: "/dns-query" }),
    ("Quad9", Mode::Tls { address: [9, 9, 9, 9], name: "dns.quad9.net" }, Mode::Https { address: [9, 9, 9, 9], host: "dns.quad9.net", path: "/dns-query" }),
    ("Google", Mode::Tls { address: [8, 8, 8, 8], name: "dns.google" }, Mode::Https { address: [8, 8, 8, 8], host: "dns.google", path: "/dns-query" }),
];

impl Mode {
    pub fn port(&self) -> u16 { match self { Mode::Plain => 53, Mode::Tls { .. } => 853, Mode::Https { .. } => 443 } }
    /// The name the TLS certificate must carry.
    pub fn server_name(&self) -> Option<&'static str> { match self { Mode::Plain => None, Mode::Tls { name, .. } => Some(name), Mode::Https { host, .. } => Some(host) } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Name, Reply, Status(u16), NoAddress }

/// One lookup over a secure transport: the A question and, when `six`, the AAAA one.
pub struct Lookup { mode: Mode, name: [u8; 255], len: usize, id: u16, six: bool, input: Vec<u8>, v4: Option<Option<[u8; 4]>>, v6: Option<Option<[u8; 16]>>, ttl: u32 }

impl Lookup {
    /// `id` should be fresh per lookup (odd; the AAAA question takes `id + 1`). DoH sends
    /// id 0 as RFC 8484 asks, so answers can be cached by HTTP; then `id` is ignored.
    pub fn new(mode: Mode, host: &[u8], six: bool, id: u16) -> Result<Self, Error> {
        let mut name = [0u8; 255];
        let len = dns_wire::encode_name(host, &mut name).ok_or(Error::Name)?;
        let id = if matches!(mode, Mode::Https { .. }) { 1 } else { id | 1 };
        Ok(Self { mode, name, len, id, six, input: Vec::new(), v4: None, v6: if six { None } else { Some(None) }, ttl: u32::MAX })
    }

    fn question(&self, kind: u16) -> Vec<u8> {
        let mut q = [0u8; dns_wire::MAX_BYTES];
        let id = if kind == dns_wire::A { self.id } else { self.id.wrapping_add(1) };
        let n = dns_wire::query_type(id, &self.name[..self.len], kind, &mut q).unwrap_or(0);
        let mut v = q[..n].to_vec();
        if matches!(self.mode, Mode::Https { .. }) && n >= 2 { v[0] = 0; v[1] = 0; }
        v
    }

    /// Everything to send in the TLS session, at once (both questions are pipelined).
    pub fn request(&self) -> Vec<u8> {
        let kinds: &[u16] = if self.six { &[dns_wire::A, dns_wire::AAAA] } else { &[dns_wire::A] };
        let mut out = Vec::new();
        for &kind in kinds {
            let q = self.question(kind);
            match self.mode {
                Mode::Https { host, path, .. } => {
                    out.extend_from_slice(b"POST ");
                    out.extend_from_slice(path.as_bytes());
                    out.extend_from_slice(b" HTTP/1.1\r\nHost: ");
                    out.extend_from_slice(host.as_bytes());
                    out.extend_from_slice(b"\r\nAccept: application/dns-message\r\nContent-Type: application/dns-message\r\nContent-Length: ");
                    out.extend_from_slice(alloc::format!("{}", q.len()).as_bytes());
                    out.extend_from_slice(b"\r\n\r\n");
                }
                _ => out.extend_from_slice(&(q.len() as u16).to_be_bytes()),
            }
            out.extend_from_slice(&q);
        }
        out
    }

    /// Bytes that came back. Gives the answers once every question has its reply;
    /// `Err(NoAddress)` when neither family has an address.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<Option<Answers>, Error> {
        self.input.extend_from_slice(bytes);
        loop {
            let Some((message, used)) = self.next_message()? else { break };
            self.input.drain(..used);
            self.answer(&message)?;
        }
        match (self.v4, self.v6) {
            (Some(v4), Some(v6)) if v4.is_none() && v6.is_none() => Err(Error::NoAddress),
            (Some(v4), Some(v6)) => Ok(Some(Answers { v4, v6 })),
            _ => Ok(None),
        }
    }

    /// The shortest TTL of the answers, for the cache.
    pub fn ttl(&self) -> u32 { self.ttl }

    fn next_message(&self) -> Result<Option<(Vec<u8>, usize)>, Error> {
        let b = &self.input;
        if !matches!(self.mode, Mode::Https { .. }) {
            if b.len() < 2 { return Ok(None); }
            let n = u16::from_be_bytes([b[0], b[1]]) as usize;
            return Ok(b.get(2..2 + n).map(|m| (m.to_vec(), 2 + n)));
        }
        // One HTTP/1.1 response with Content-Length.
        let Some(end) = b.windows(4).position(|w| w == b"\r\n\r\n") else {
            return if b.len() > 16 * 1024 { Err(Error::Reply) } else { Ok(None) };
        };
        let head = core::str::from_utf8(&b[..end]).map_err(|_| Error::Reply)?;
        let mut lines = head.split("\r\n");
        let status: u16 = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse().ok()).ok_or(Error::Reply)?;
        let mut length = None;
        let mut kind_ok = false;
        for l in lines {
            let Some((n, v)) = l.split_once(':') else { continue };
            let (n, v) = (n.trim(), v.trim());
            if n.eq_ignore_ascii_case("content-length") { length = v.parse::<usize>().ok(); }
            if n.eq_ignore_ascii_case("content-type") { kind_ok = v.eq_ignore_ascii_case("application/dns-message"); }
        }
        let length = length.ok_or(Error::Reply)?;
        if b.len() < end + 4 + length { return Ok(None); }
        if status != 200 { return Err(Error::Status(status)); }
        if !kind_ok || length > 65535 { return Err(Error::Reply); }
        Ok(Some((b[end + 4..end + 4 + length].to_vec(), end + 4 + length)))
    }

    fn answer(&mut self, message: &[u8]) -> Result<(), Error> {
        if message.len() < 12 { return Err(Error::Reply); }
        // DoH answers keep id 0; replies come in the order asked (HTTP/1.1 pipelining).
        let id = u16::from_be_bytes([message[0], message[1]]);
        let kind = match &self.mode {
            Mode::Https { .. } => if self.v4.is_none() { dns_wire::A } else { dns_wire::AAAA },
            _ if id == self.id => dns_wire::A,
            _ if id == self.id.wrapping_add(1) && self.six => dns_wire::AAAA,
            _ => return Err(Error::Reply),
        };
        let mut m = message.to_vec();
        let want = if kind == dns_wire::A { self.id } else { self.id.wrapping_add(1) };
        m[..2].copy_from_slice(&want.to_be_bytes());
        let found = match dns_wire::parse_type(want, &self.name[..self.len], kind, &m) {
            Ok(Some(f)) => { self.ttl = self.ttl.min(f.ttl); Some(f.address) }
            Ok(None) => return Err(Error::Reply),
            Err(_) => None,
        };
        if kind == dns_wire::A { self.v4 = Some(found.map(|a| [a[0], a[1], a[2], a[3]])); } else { self.v6 = Some(found); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reply(question: &[u8], data: &[u8]) -> Vec<u8> {
        let mut r = question.to_vec();
        r[2] = 0x81; r[3] = 0x80; r[7] = if data.is_empty() { 0 } else { 1 };
        if !data.is_empty() {
            let kind = &question[question.len() - 4..question.len() - 2];
            r.extend_from_slice(&[0xc0, 0x0c]); r.extend_from_slice(kind); r.extend_from_slice(&[0, 1, 0, 0, 1, 0x2c]);
            r.extend_from_slice(&(data.len() as u16).to_be_bytes()); r.extend_from_slice(data);
        }
        r
    }
    const SIX: [u8; 16] = [0x26, 6, 0x47, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x11, 0x11];
    #[test]
    fn dns_over_tls_pipelines_both_questions_and_reads_replies_in_any_order_and_pieces() {
        let mode = PRESETS[0].1;
        assert_eq!((mode.port(), mode.server_name()), (853, Some("one.one.one.one")));
        let mut l = Lookup::new(mode, b"example.org", true, 0x4242).unwrap();
        let req = l.request();
        let n = u16::from_be_bytes([req[0], req[1]]) as usize;
        let (a, rest) = req[2..].split_at(n);
        let aaaa = &rest[2..];
        assert_eq!(&a[..2], &0x4243u16.to_be_bytes());
        assert_eq!(&aaaa[..2], &0x4244u16.to_be_bytes());
        let mut wire = Vec::new();
        for m in [reply(aaaa, &SIX), reply(a, &[93, 184, 215, 14])] { wire.extend_from_slice(&(m.len() as u16).to_be_bytes()); wire.extend(m); }
        let mut got = Ok(None);
        for piece in wire.chunks(7) { got = l.receive(piece); }
        assert_eq!(got, Ok(Some(Answers { v4: Some([93, 184, 215, 14]), v6: Some(SIX) })));
        assert_eq!(l.ttl(), 300);
    }
    #[test]
    fn dns_over_https_posts_dns_messages_with_id_zero_and_reads_http_replies() {
        let mode = PRESETS[1].2;
        let mut l = Lookup::new(mode, b"example.org", false, 77).unwrap();
        let req = l.request();
        let text = core::str::from_utf8(&req[..req.windows(4).position(|w| w == b"\r\n\r\n").unwrap()]).unwrap();
        assert!(text.starts_with("POST /dns-query HTTP/1.1\r\nHost: dns.quad9.net\r\n") && text.contains("Content-Type: application/dns-message"));
        let q = &req[text.len() + 4..];
        assert_eq!(&q[..2], &[0, 0], "RFC 8484: id 0");
        let body = reply(q, &[192, 0, 2, 1]);
        let mut http = alloc::format!("HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        http.extend(body);
        assert_eq!(l.receive(&http[..30]), Ok(None));
        assert_eq!(l.receive(&http[30..]), Ok(Some(Answers { v4: Some([192, 0, 2, 1]), v6: None })));
        let mut l = Lookup::new(mode, b"example.org", false, 77).unwrap();
        assert_eq!(l.receive(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n"), Err(Error::Status(503)));
    }
    #[test]
    fn a_reply_to_another_question_or_with_no_address_is_not_accepted() {
        let mut l = Lookup::new(PRESETS[2].1, b"example.org", false, 9).unwrap();
        let req = l.request();
        let mut other = reply(&req[2..], &[1, 2, 3, 4]);
        other[1] ^= 0x40;
        let mut wire = (other.len() as u16).to_be_bytes().to_vec();
        wire.extend(other);
        assert_eq!(l.receive(&wire), Err(Error::Reply));
        let mut l = Lookup::new(PRESETS[2].1, b"nothing.example", false, 9).unwrap();
        let req = l.request();
        let none = reply(&req[2..], &[]);
        let mut wire = (none.len() as u16).to_be_bytes().to_vec();
        wire.extend(none);
        assert_eq!(l.receive(&wire), Err(Error::NoAddress));
        assert_eq!(Lookup::new(Mode::Plain, b"bad..name", false, 1).err(), Some(Error::Name));
    }
}
