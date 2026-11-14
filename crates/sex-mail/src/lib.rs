// SPDX-License-Identifier: GPL-3.0-only
//! Mail for Valentine, SexOS's inbox: IMAP (with IDLE) and SMTP clients, RFC 5322 and
//! MIME parsing, conversation threading, and [`fixture`], a small in-memory IMAP+SMTP
//! server the host tests and the QEMU gate talk to.
//!
//! Protocols run over a [`Conn`]: on SexOS that is a sex-net TCP flow with TLS; in tests
//! it is the fixture. Authentication is a separate step ([`imap::Client::authenticate`],
//! [`smtp::Client::authenticate`]) so Multipass can run it with the secret from Shibari
//! and hand Valentine the already-authenticated flow: Valentine never sees a password.
#![no_std]
extern crate alloc;

pub mod b64;
pub mod fixture;
pub mod imap;
pub mod message;
pub mod smtp;
pub mod thread;

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The connection closed or failed.
    Io,
    /// The server said something we could not parse.
    Protocol(String),
    /// The server refused (IMAP NO/BAD, SMTP 4xx/5xx), with its text.
    Refused(String),
}

pub type Result<T> = core::result::Result<T, Error>;

/// A byte stream: a TLS flow on SexOS, an in-memory server in tests.
pub trait Conn {
    fn write_all(&mut self, data: &[u8]) -> Result<()>;
    /// Read at least one byte, blocking until some arrive. Returns 0 at end of stream.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
}

/// Line reader over a [`Conn`], keeping leftovers between calls.
pub struct Lines<C: Conn> { pub conn: C, buf: Vec<u8> }

impl<C: Conn> Lines<C> {
    pub fn new(conn: C) -> Self { Self { conn, buf: Vec::new() } }

    fn fill(&mut self) -> Result<()> {
        let mut tmp = [0u8; 4096];
        let n = self.conn.read(&mut tmp)?;
        if n == 0 { return Err(Error::Io); }
        self.buf.extend_from_slice(&tmp[..n]);
        Ok(())
    }

    /// One line without its CRLF.
    pub fn line(&mut self) -> Result<Vec<u8>> {
        loop {
            if let Some(i) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let line = self.buf[..i].to_vec();
                self.buf.drain(..i + 2);
                return Ok(line);
            }
            if self.buf.len() > 1 << 20 { return Err(Error::Protocol("line too long".into())); }
            self.fill()?;
        }
    }

    /// Exactly `n` bytes (an IMAP literal).
    pub fn bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        while self.buf.len() < n { self.fill()?; }
        Ok(self.buf.drain(..n).collect())
    }

    pub fn send(&mut self, data: &[u8]) -> Result<()> { self.conn.write_all(data) }
}

/// Authentication material, supplied by Multipass from Shibari.
pub enum Auth<'a> {
    Plain { user: &'a str, password: &'a str },
    /// OAuth 2 bearer (Gmail, Outlook): SASL XOAUTH2.
    XOAuth2 { user: &'a str, token: &'a str },
}

impl Auth<'_> {
    pub(crate) fn sasl(&self) -> (&'static str, String) {
        match self {
            Auth::Plain { user, password } => {
                let mut raw = Vec::new();
                raw.push(0); raw.extend_from_slice(user.as_bytes()); raw.push(0); raw.extend_from_slice(password.as_bytes());
                ("PLAIN", b64::encode(&raw))
            }
            Auth::XOAuth2 { user, token } => ("XOAUTH2", b64::encode(alloc::format!("user={user}\x01auth=Bearer {token}\x01\x01").as_bytes())),
        }
    }
}
