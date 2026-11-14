// SPDX-License-Identifier: GPL-3.0-only
//! WebDAV (RFC 4918), CalDAV (RFC 4791) and CardDAV (RFC 6352) for SexOS programs,
//! without any I/O, so all of it is tested on the host.
//!
//! - [`xml`]: a small namespace-aware XML reader.
//! - [`dav`]: PROPFIND and REPORT bodies (calendar-query, addressbook-query,
//!   sync-collection) and the multistatus replies they get.
//! - [`sync`]: two-way sync of one collection: ctag and sync-token to see whether
//!   anything changed, etags to see what, PUT with If-Match / If-None-Match, DELETE,
//!   and the conflict rule: the server wins and the local copy is kept as a note.
//!
//! A program moves the bytes through a [`DavTransport`]. Paths are relative to the
//! account's origin and no credentials pass through this crate: the transport adds
//! them. Today the transport is direct HTTP with a credentials file in the program's
//! own folder ([`FileAccount`]); the Multipass account service will take its place.
#![no_std]

extern crate alloc;

pub mod dav;
pub mod sync;
pub mod xml;

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The transport could not reach the server (its own error kept as a number).
    Transport(u32),
    /// The server answered with this status where success was needed.
    Status(u16),
    /// A reply that is not XML we can read.
    Xml(xml::Error),
    /// An address that is not `http(s)://host[:port]/path`.
    Address,
}

impl From<xml::Error> for Error { fn from(e: xml::Error) -> Error { Error::Xml(e) } }

/// What the server answered: the status, the ETag header when it sent one, the body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DavResponse { pub status: u16, pub etag: Option<String>, pub body: Vec<u8> }

/// The way requests reach the server. `path` is relative to the account's origin
/// (`/dav/calendars/me/home/`); the only headers given are the ones a program may set
/// (Depth, If-Match, If-None-Match, Prefer, Accept); credentials are the transport's.
pub trait DavTransport {
    fn request(&mut self, method: &str, path: &str, content_type: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<DavResponse, Error>;
}

/// A DAV account as a direct-HTTP transport needs it.
// TODO(Multipass): accounts move to the Multipass account service (sex_pdx::accounts,
// OP 0x2B0: the program asks for a grant and sends "method\npath\ncontent-type\n
// headers\n\nbody" through it; Multipass adds the credentials). A MultipassTransport
// implementing DavTransport replaces DirectHttp + FileAccount then.
pub trait DavAccount {
    /// `http://host:port/dav/` or `https://...`: where discovery starts.
    fn base_url(&self) -> &str;
    /// The Authorization header's value, if the account has credentials.
    fn auth_header(&self) -> Option<String>;
}

/// An account written in the program's own folder:
///
/// ```text
/// url=http://10.0.2.2:8080/dav/calendars/me/
/// user=me
/// password=secret
/// email=me@example.org
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileAccount { pub url: String, pub user: String, pub password: String, pub email: String }

impl FileAccount {
    pub fn parse(text: &str) -> Option<FileAccount> {
        let mut a = FileAccount::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = String::from(v.trim());
            match k.trim() { "url" => a.url = v, "user" => a.user = v, "password" => a.password = v, "email" => a.email = v, _ => {} }
        }
        if split_url(&a.url).is_some() { Some(a) } else { None }
    }
}

impl DavAccount for FileAccount {
    fn base_url(&self) -> &str { &self.url }
    fn auth_header(&self) -> Option<String> {
        if self.user.is_empty() { return None; }
        let mut pair = String::from(&self.user);
        pair.push(':');
        pair.push_str(&self.password);
        let mut out = String::from("Basic ");
        out.push_str(&base64(pair.as_bytes()));
        Some(out)
    }
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() { out.push(T[(n >> (18 - 6 * k) & 63) as usize] as char); } else { out.push('='); }
        }
    }
    out
}

/// `http://host:port/a/b` as its origin (`http://host:port`) and path (`/a/b`).
pub fn split_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://"))?;
    let host_end = rest.find('/').unwrap_or(rest.len());
    if host_end == 0 { return None; }
    let at = url.len() - rest.len() + host_end;
    Some((&url[..at], if at == url.len() { "/" } else { &url[at..] }))
}

/// An href from a reply as a path on the same origin: an absolute path stays, a full
/// URL loses its origin, a relative one is joined to `base` (a collection path).
pub fn resolve(base: &str, href: &str) -> String {
    let href = href.trim();
    if let Some((_, path)) = split_url(href) { return String::from(path); }
    if href.starts_with('/') { return String::from(href); }
    let mut out = String::from(&base[..base.rfind('/').map_or(0, |i| i + 1)]);
    if out.is_empty() { out.push('/'); }
    out.push_str(href);
    out
}

/// Percent-decoding for comparing hrefs (`%40` and `@` are the same resource).
pub fn unpercent(path: &str) -> String {
    let b = path.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(core::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("x"), 16) { out.push(v); i += 3; continue; }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| String::from(path))
}

/// A resource name made from a UID: letters, digits and `-_.@` kept, the rest `_`.
pub fn resource_name(uid: &str, ext: &str) -> String {
    let mut out: String = uid.chars().take(120).map(|c| if c.is_ascii_alphanumeric() || "-_.@".contains(c) { c } else { '_' }).collect();
    if out.is_empty() { out.push_str("item"); }
    out.push_str(ext);
    out
}
