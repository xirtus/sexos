// SPDX-License-Identifier: GPL-3.0-only
//! SMTP submission client (RFC 5321, RFC 6409) with ESMTP AUTH (RFC 4954) and
//! dot-stuffing. TLS (implicit on 465 or STARTTLS on 587) is the flow's business.
use crate::message::address;
use crate::{Auth, Conn, Error, Lines, Result};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub struct Reply { pub code: u16, pub lines: Vec<String> }

pub struct Client<C: Conn> { io: Lines<C>, pub extensions: Vec<String> }

impl<C: Conn> Client<C> {
    /// Read the 220 greeting and say EHLO.
    pub fn connect(conn: C, client_name: &str) -> Result<Self> {
        let mut c = Self { io: Lines::new(conn), extensions: Vec::new() };
        c.expect(220)?;
        let r = c.command(&format!("EHLO {client_name}"), 250)?;
        c.extensions = r.lines.iter().skip(1).map(|l| l.to_ascii_uppercase()).collect();
        Ok(c)
    }

    pub fn into_conn(self) -> C { self.io.conn }

    pub fn reply(&mut self) -> Result<Reply> {
        let mut lines = Vec::new();
        loop {
            let l = self.io.line()?;
            if l.len() < 3 { return Err(Error::Protocol("short SMTP reply".into())); }
            let code: u16 = core::str::from_utf8(&l[..3]).ok().and_then(|c| c.parse().ok()).ok_or_else(|| Error::Protocol("bad SMTP code".into()))?;
            lines.push(String::from_utf8_lossy(l.get(4..).unwrap_or(&[])).into_owned());
            if l.get(3) != Some(&b'-') { return Ok(Reply { code, lines }); }
        }
    }

    fn expect(&mut self, code: u16) -> Result<Reply> {
        let r = self.reply()?;
        if r.code != code { return Err(Error::Refused(format!("{} {}", r.code, r.lines.join(" ")))); }
        Ok(r)
    }

    pub fn command(&mut self, cmd: &str, code: u16) -> Result<Reply> {
        self.io.send(format!("{cmd}\r\n").as_bytes())?;
        self.expect(code)
    }

    /// Run by Multipass with the secret from Shibari.
    pub fn authenticate(&mut self, auth: &Auth) -> Result<()> {
        let (mech, ir) = auth.sasl();
        self.command(&format!("AUTH {mech} {ir}"), 235).map(|_| ())
    }

    /// Send one message. Recipients come from the caller (To, Cc and Bcc), not the headers.
    pub fn send(&mut self, from: &str, to: &[&str], message: &[u8]) -> Result<()> {
        self.command(&format!("MAIL FROM:<{}>", address(from)), 250)?;
        for r in to { self.command(&format!("RCPT TO:<{}>", address(r)), 250)?; }
        self.command("DATA", 354)?;
        self.io.send(&dot_stuff(message))?;
        self.io.send(b".\r\n")?;
        self.expect(250).map(|_| ())
    }

    pub fn quit(&mut self) -> Result<()> { self.command("QUIT", 221).map(|_| ()) }
}

/// Normalise line ends to CRLF, double leading dots, and end with CRLF.
pub fn dot_stuff(m: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(m.len() + 16);
    let mut at_line_start = true;
    let mut i = 0;
    while i < m.len() {
        let b = m[i];
        if at_line_start && b == b'.' { out.push(b'.'); }
        match b {
            b'\r' if m.get(i + 1) == Some(&b'\n') => { out.extend_from_slice(b"\r\n"); i += 1; at_line_start = true; }
            b'\n' | b'\r' => { out.extend_from_slice(b"\r\n"); at_line_start = true; }
            _ => { out.push(b); at_line_start = false; }
        }
        i += 1;
    }
    if !out.ends_with(b"\r\n") { out.extend_from_slice(b"\r\n"); }
    out
}

/// The reverse, for servers (and the fixture).
pub fn dot_unstuff(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for line in data.split_inclusive(|&b| b == b'\n') {
        out.extend_from_slice(line.strip_prefix(b".").filter(|_| line.starts_with(b"..")).unwrap_or(line));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn stuffing_round_trips() {
        let m = b"a\n.b\r\n..c\rd";
        let s = super::dot_stuff(m);
        assert_eq!(s, b"a\r\n..b\r\n...c\r\nd\r\n");
        assert_eq!(super::dot_unstuff(&s), b"a\r\n.b\r\n..c\r\nd\r\n");
    }
}
