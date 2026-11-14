// SPDX-License-Identifier: GPL-3.0-only
//! WebDAV (RFC 4918): PROPFIND (depth 0 and 1), GET with Range, PUT, MKCOL, MOVE and
//! DELETE. Nextcloud and ownCloud serve files at `/remote.php/dav/files/<user>`.
//! A write goes to a temporary name in the same folder and is moved over the target
//! (`Overwrite: T`), so a failed upload leaves the old file.
use crate::http::{Body, Http, Request, Response};
use crate::text::{base64, parse_http_date, percent_decode, percent_encode};
use crate::{components, split_last, temp_name, Abilities, Entry, Error, Kind, Place, Result, Source};
use alloc::{format, string::{String, ToString}, vec, vec::Vec};

/// How to log in. `Debug` never shows the secret.
#[derive(Clone)]
pub enum Auth { None, Basic { user: String, password: String }, Bearer(String) }
impl core::fmt::Debug for Auth {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self { Auth::None => f.write_str("None"), Auth::Basic { user, .. } => write!(f, "Basic({user}, ..)"), Auth::Bearer(_) => f.write_str("Bearer(..)") }
    }
}

const PROPFIND: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;
/// Largest PROPFIND reply we take (a folder of ~20 000 items).
const LISTING_MAX: usize = 16 << 20;

pub struct WebDav<H: Http> {
    http: H,
    scheme: String,
    /// The URL path of the place's root, without a trailing `/`.
    base: String,
    auth: Option<String>,
    temp_counter: u32,
}

impl<H: Http> WebDav<H> {
    /// `scheme` is "https" or "http" (for MOVE's Destination); `base` the root's URL
    /// path, e.g. "/remote.php/dav/files/alice".
    pub fn new(http: H, scheme: &str, base: &str, auth: Auth) -> Self {
        let auth = match auth {
            Auth::None => None,
            Auth::Basic { user, password } => Some(format!("Basic {}", base64(format!("{user}:{password}").as_bytes()))),
            Auth::Bearer(t) => Some(format!("Bearer {t}")),
        };
        let base = base.trim_end_matches('/');
        let base = if base.starts_with('/') || base.is_empty() { base.into() } else { format!("/{base}") };
        Self { http, scheme: scheme.into(), base, auth, temp_counter: 0x5eed }
    }
    pub fn http(&mut self) -> &mut H { &mut self.http }

    fn target(&self, parts: &[&str], dir: bool) -> String {
        let mut t = self.base.clone();
        for p in parts { t.push('/'); t.push_str(&percent_encode(p, false)); }
        if dir || parts.is_empty() { t.push('/'); }
        t
    }
    fn send(&mut self, method: &str, target: &str, mut headers: Vec<(String, String)>, body: Body<'_>, limit: usize) -> Result<Response> {
        if let Some(a) = &self.auth { headers.push(("authorization".into(), a.clone())); }
        self.http.send(&Request { method, target, headers }, body, limit)
    }

    fn propfind(&mut self, parts: &[&str], depth: u8) -> Result<Vec<(String, Entry)>> {
        let t = self.target(parts, depth == 1);
        let h = vec![("depth".into(), depth.to_string()), ("content-type".into(), "application/xml; charset=utf-8".into())];
        let r = self.send("PROPFIND", &t, h, Body::Bytes(PROPFIND.as_bytes()), LISTING_MAX)?;
        if r.status != 207 { return Err(status_error(&r)); }
        if r.truncated { return Err(Error::Protocol("listing too large")); }
        let doc = crate::xml::parse(r.text()).ok_or(Error::Protocol("bad multistatus XML"))?;
        let mut out = Vec::new();
        for resp in doc.all("response") {
            let Some(href) = resp.text_of("href") else { continue };
            let path = href_path(href.trim());
            let mut e = Entry { name: path.rsplit('/').next().unwrap_or("").to_string(), kind: Kind::File, size: 0, mtime: None };
            for ps in resp.all("propstat") {
                if !ps.text_of("status").is_some_and(|s| s.split_whitespace().nth(1) == Some("200")) { continue; }
                let Some(prop) = ps.child("prop") else { continue };
                if prop.child("resourcetype").is_some_and(|t| t.child("collection").is_some()) { e.kind = Kind::Dir; }
                if let Some(n) = prop.text_of("getcontentlength") { e.size = n.trim().parse().unwrap_or(0); }
                if let Some(d) = prop.text_of("getlastmodified") { e.mtime = parse_http_date(d); }
            }
            if e.kind == Kind::Dir { e.size = 0; }
            out.push((path, e));
        }
        Ok(out)
    }
    fn stat_parts(&mut self, parts: &[&str]) -> Result<Entry> {
        let mut v = self.propfind(parts, 0)?;
        if v.is_empty() { return Err(Error::Protocol("empty multistatus")); }
        let (_, mut e) = v.swap_remove(0);
        e.name = parts.last().unwrap_or(&"").to_string();
        Ok(e)
    }
    fn delete_target(&mut self, t: &str) -> Result<()> {
        let r = self.send("DELETE", t, Vec::new(), Body::Empty, 4096)?;
        if r.ok() { Ok(()) } else { Err(status_error(&r)) }
    }
    fn move_target(&mut self, from: &str, to: &str, overwrite: bool) -> Result<()> {
        let dest = format!("{}://{}{}", self.scheme, self.http.authority(), to);
        let h = vec![("destination".into(), dest), ("overwrite".into(), if overwrite { "T" } else { "F" }.into())];
        let r = self.send("MOVE", from, h, Body::Empty, 4096)?;
        if r.ok() { Ok(()) } else if r.status == 412 { Err(Error::Exists) } else { Err(status_error(&r)) }
    }
}

/// An href's path, decoded, without scheme, host or trailing `/`.
fn href_path(href: &str) -> String {
    let p = match href.find("://") { Some(i) => href[i + 3..].find('/').map_or("/", |j| &href[i + 3 + j..]), None => href };
    percent_decode(p.trim_end_matches('/'))
}

fn status_error(r: &Response) -> Error {
    match r.status {
        401 | 403 => Error::Denied,
        404 | 409 => Error::NotFound,
        405 => Error::Exists,
        s => { let mut m: String = r.text().chars().take(200).collect(); if m.is_empty() { m = "HTTP error".into(); } Error::Remote(s as u32, m) }
    }
}

impl<H: Http> Place for WebDav<H> {
    fn abilities(&self) -> Abilities { Abilities { atomic_write: true, atomic_dir_rename: true } }
    fn list(&mut self, dir: &str) -> Result<Vec<Entry>> {
        let parts = components(dir)?;
        let all = self.propfind(&parts, 1)?;
        let me = percent_decode(self.target(&parts, false).trim_end_matches('/'));
        let mut out: Vec<Entry> = Vec::new();
        let mut is_dir = parts.is_empty();
        for (path, e) in all {
            if path == me { is_dir |= e.kind == Kind::Dir; continue; }
            if !e.name.is_empty() { out.push(e); }
        }
        if !is_dir && out.is_empty() { return Err(Error::BadPath); }
        crate::sort_entries(&mut out);
        Ok(out)
    }
    fn stat(&mut self, path: &str) -> Result<Entry> { let parts = components(path)?; self.stat_parts(&parts) }
    fn read(&mut self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        let parts = components(path)?;
        if parts.is_empty() { return Err(Error::BadPath); }
        if len == 0 { return Ok(Vec::new()); }
        let t = self.target(&parts, false);
        let last = offset.saturating_add(len - 1);
        let limit = usize::try_from(offset.saturating_add(len)).unwrap_or(usize::MAX);
        let r = self.send("GET", &t, vec![("range".into(), format!("bytes={offset}-{last}"))], Body::Empty, limit)?;
        match r.status {
            206 => { let mut b = r.body; b.truncate(len as usize); Ok(b) }
            // The server ignored Range: skip to the offset.
            200 => Ok(r.body.get(offset as usize..).unwrap_or(&[]).iter().copied().take(len as usize).collect()),
            416 => Ok(Vec::new()),
            _ => Err(status_error(&r)),
        }
    }
    fn write(&mut self, path: &str, src: &mut dyn Source, size: Option<u64>) -> Result<u64> {
        let (parent, name) = split_last(path)?;
        let mut parts = parent.clone();
        parts.push(name);
        match self.stat_parts(&parts) { Ok(e) if e.kind == Kind::Dir => return Err(Error::Exists), Ok(_) | Err(Error::NotFound) => {} Err(e) => return Err(e) }
        self.temp_counter = self.temp_counter.wrapping_mul(1664525).wrapping_add(1013904223);
        let tmp_name = temp_name(name, self.temp_counter);
        let mut tparts = parent.clone();
        tparts.push(&tmp_name);
        let tmp = self.target(&tparts, false);
        let target = self.target(&parts, false);
        let mut counted = Counting { src, n: 0 };
        let r = self.send("PUT", &tmp, vec![("content-type".into(), "application/octet-stream".into())], Body::Stream { src: &mut counted, len: size }, 4096);
        let n = counted.n;
        let done = match r { Ok(r) if r.ok() => self.move_target(&tmp, &target, true), Ok(r) => Err(status_error(&r)), Err(e) => Err(e) };
        if done.is_err() { let _ = self.delete_target(&tmp); }
        done.map(|_| n)
    }
    fn mkdir(&mut self, path: &str) -> Result<()> {
        let parts = components(path)?;
        if parts.is_empty() { return Err(Error::BadPath); }
        let t = self.target(&parts, true);
        let r = self.send("MKCOL", &t, Vec::new(), Body::Empty, 4096)?;
        if r.ok() { Ok(()) } else { Err(status_error(&r)) }
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let (f, t) = (components(from)?, components(to)?);
        if f.is_empty() || t.is_empty() { return Err(Error::BadPath); }
        let dir = self.stat_parts(&f)?.kind == Kind::Dir;
        let (ft, tt) = (self.target(&f, dir), self.target(&t, dir));
        self.move_target(&ft, &tt, false)
    }
    fn delete(&mut self, path: &str) -> Result<()> {
        let parts = components(path)?;
        if parts.is_empty() { return Err(Error::BadPath); }
        let e = self.stat_parts(&parts)?;
        // DELETE on a collection removes everything under it: check it is empty first.
        if e.kind == Kind::Dir && !self.list(path)?.is_empty() { return Err(Error::NotEmpty); }
        let t = self.target(&parts, e.kind == Kind::Dir);
        self.delete_target(&t)
    }
}

/// Counts bytes as an upload reads them.
struct Counting<'a> { src: &'a mut dyn Source, n: u64 }
impl Source for Counting<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> { let k = self.src.read(buf)?; self.n += k as u64; Ok(k) }
}
