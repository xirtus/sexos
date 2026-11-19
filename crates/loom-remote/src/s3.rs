// SPDX-License-Identifier: GPL-3.0-only
//! S3-compatible object storage (AWS S3, MinIO, Ceph RGW, Garage, R2, B2, ...) with
//! Signature Version 4. Folders are key prefixes: ListObjectsV2 with delimiter `/`
//! lists one level; an empty folder is a zero-byte `name/` marker object, as the
//! S3 consoles make them. Uploads up to `part_size` are one PUT; bigger ones use a
//! multipart upload that is aborted on failure. S3 writes are all-or-nothing by
//! nature. Rename is CopyObject then DeleteObject (per object for a folder).
use crate::http::{Body, Http, Request, Response};
use crate::sigv4::{self, Credentials, Scope};
use crate::text::{amz_date, parse_http_date, parse_iso8601, percent_encode};
use crate::{components, Abilities, Entry, Error, Kind, Place, Result, Source};
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};

/// S3 copies at most 5 GiB in one CopyObject.
const COPY_MAX: u64 = 5 << 30;
const LISTING_MAX: usize = 4 << 20;

pub struct Config {
    pub bucket: String,
    pub region: String,
    /// `/bucket/key` on the endpoint (MinIO, most self-hosted stores) rather than the
    /// bucket in the host name (then the Http's authority is `bucket.s3.<region>...`).
    pub path_style: bool,
    /// Key prefix the place starts at ("" for the whole bucket, or "photos/").
    pub prefix: String,
    /// Uploads bigger than this go in parts of this size (S3 needs at least 5 MiB).
    pub part_size: usize,
}
impl Config {
    pub fn new(bucket: &str, region: &str) -> Self { Self { bucket: bucket.into(), region: region.into(), path_style: true, prefix: String::new(), part_size: 16 << 20 } }
}

pub struct S3<H: Http> {
    http: H,
    cfg: Config,
    creds: Credentials,
    /// Seconds since 1970 (UTC), for signing.
    clock: Box<dyn FnMut() -> u64>,
}

struct Listing { files: Vec<(String, u64, Option<u64>)>, dirs: Vec<String>, next: Option<String> }

impl<H: Http> S3<H> {
    pub fn new(http: H, mut cfg: Config, creds: Credentials, clock: Box<dyn FnMut() -> u64>) -> Self {
        if !cfg.prefix.is_empty() && !cfg.prefix.ends_with('/') { cfg.prefix.push('/'); }
        cfg.part_size = cfg.part_size.max(5 << 20);
        Self { http, cfg, creds, clock }
    }
    pub fn http(&mut self) -> &mut H { &mut self.http }

    fn key(&self, parts: &[&str], dir: bool) -> String {
        let mut k = self.cfg.prefix.clone();
        k.push_str(&parts.join("/"));
        if dir && !parts.is_empty() { k.push('/'); }
        k
    }
    fn uri(&self, key: &str) -> String {
        let k = percent_encode(key, true);
        if self.cfg.path_style { format!("/{}/{k}", percent_encode(&self.cfg.bucket, false)) } else { format!("/{k}") }
    }

    /// One signed request. `key` None means the bucket itself.
    fn call(&mut self, method: &str, key: Option<&str>, query: &[(&str, &str)], extra: Vec<(String, String)>, body: &[u8], limit: usize) -> Result<Response> {
        let uri = match key { Some(k) => self.uri(k), None => if self.cfg.path_style { format!("/{}", percent_encode(&self.cfg.bucket, false)) } else { "/".into() } };
        let q = sigv4::canonical_query(query);
        let date = amz_date((self.clock)());
        let hash = if body.is_empty() { sigv4::EMPTY_SHA256.into() } else { sigv4::sha256_hex(body) };
        let mut headers = extra;
        headers.push(("x-amz-date".into(), date.clone()));
        headers.push(("x-amz-content-sha256".into(), hash.clone()));
        if let Some(t) = &self.creds.session_token { headers.push(("x-amz-security-token".into(), t.clone())); }
        let host = self.http.authority().to_string();
        let mut signed: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        signed.push(("host", &host));
        let auth = sigv4::authorization(&self.creds, &Scope { amz_date: &date, region: &self.cfg.region, service: "s3" }, method, &uri, &q, &signed, &hash);
        headers.push(("authorization".into(), auth));
        let target = if q.is_empty() { uri } else { format!("{uri}?{q}") };
        let body = if body.is_empty() { Body::Empty } else { Body::Bytes(body) };
        self.http.send(&Request { method, target: &target, headers }, body, limit)
    }
    fn check(&self, r: Response) -> Result<Response> {
        // CopyObject and CompleteMultipartUpload may fail inside a 200.
        if r.ok() && !r.text().contains("<Error>") { Ok(r) } else { Err(s3_error(&r)) }
    }

    fn list_page(&mut self, prefix: &str, delimiter: bool, token: Option<&str>, max: Option<&str>) -> Result<Listing> {
        let mut q = vec![("list-type", "2"), ("prefix", prefix)];
        if delimiter { q.push(("delimiter", "/")); }
        if let Some(t) = token { q.push(("continuation-token", t)); }
        if let Some(m) = max { q.push(("max-keys", m)); }
        let r = self.call("GET", None, &q, Vec::new(), &[], LISTING_MAX)?;
        let r = self.check(r)?;
        let doc = crate::xml::parse(r.text()).ok_or(Error::Protocol("bad ListObjectsV2 XML"))?;
        let mut l = Listing { files: Vec::new(), dirs: Vec::new(), next: None };
        for c in doc.all("Contents") {
            let Some(k) = c.text_of("Key") else { continue };
            l.files.push((k.into(), c.text_of("Size").and_then(|s| s.trim().parse().ok()).unwrap_or(0), c.text_of("LastModified").and_then(parse_iso8601)));
        }
        for c in doc.all("CommonPrefixes") { if let Some(p) = c.text_of("Prefix") { l.dirs.push(p.into()); } }
        if doc.text_of("IsTruncated").is_some_and(|t| t.trim() == "true") {
            l.next = Some(doc.text_of("NextContinuationToken").ok_or(Error::Protocol("truncated listing without token"))?.into());
        }
        Ok(l)
    }
    /// Every key under `prefix` (all depths).
    fn all_keys(&mut self, prefix: &str) -> Result<Vec<(String, u64)>> {
        let mut out = Vec::new();
        let mut token = None;
        loop {
            let l = self.list_page(prefix, false, token.as_deref(), None)?;
            out.extend(l.files.into_iter().map(|(k, s, _)| (k, s)));
            match l.next { Some(t) => token = Some(t), None => return Ok(out) }
        }
    }
    fn head(&mut self, key: &str) -> Result<Option<(u64, Option<u64>)>> {
        let r = self.call("HEAD", Some(key), &[], Vec::new(), &[], 0)?;
        match r.status {
            200 => Ok(Some((r.header("content-length").and_then(|s| s.parse().ok()).unwrap_or(0), r.header("last-modified").and_then(parse_http_date)))),
            404 => Ok(None),
            _ => Err(s3_error(&r)),
        }
    }
    /// Is there anything at all under this folder key (marker included)?
    fn dir_exists(&mut self, dirkey: &str) -> Result<bool> {
        let l = self.list_page(dirkey, false, None, Some("1"))?;
        Ok(!l.files.is_empty())
    }
    fn delete_key(&mut self, key: &str) -> Result<()> {
        let r = self.call("DELETE", Some(key), &[], Vec::new(), &[], 4096)?;
        self.check(r).map(|_| ())
    }
    fn copy_key(&mut self, from: &str, to: &str) -> Result<()> {
        let src = format!("/{}/{}", percent_encode(&self.cfg.bucket, false), percent_encode(from, true));
        let r = self.call("PUT", Some(to), &[], vec![("x-amz-copy-source".into(), src)], &[], 64 << 10)?;
        self.check(r).map(|_| ())
    }
    fn multipart(&mut self, key: &str, first: Vec<u8>, src: &mut dyn Source) -> Result<u64> {
        let r = self.call("POST", Some(key), &[("uploads", "")], vec![("content-type".into(), "application/octet-stream".into())], &[], 64 << 10)?;
        let r = self.check(r)?;
        let doc = crate::xml::parse(r.text()).ok_or(Error::Protocol("bad InitiateMultipartUpload XML"))?;
        let id: String = doc.find("UploadId").map(|n| n.text.clone()).ok_or(Error::Protocol("no UploadId"))?;
        let res = self.parts(key, &id, first, src);
        if res.is_err() { let _ = self.call("DELETE", Some(key), &[("uploadId", &id)], Vec::new(), &[], 4096); }
        res
    }
    fn parts(&mut self, key: &str, id: &str, first: Vec<u8>, src: &mut dyn Source) -> Result<u64> {
        let mut etags = Vec::new();
        let mut buf = first;
        let mut total = 0u64;
        loop {
            let n = (etags.len() + 1).to_string();
            let r = self.call("PUT", Some(key), &[("partNumber", &n), ("uploadId", id)], Vec::new(), &buf, 4096)?;
            let r = self.check(r)?;
            etags.push(r.header("etag").ok_or(Error::Protocol("part without ETag"))?.to_string());
            total += buf.len() as u64;
            buf.resize(self.cfg.part_size, 0);
            let k = crate::read_full(src, &mut buf)?;
            if k == 0 { break; }
            buf.truncate(k);
        }
        let mut x = String::from("<CompleteMultipartUpload xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">");
        for (i, e) in etags.iter().enumerate() { x.push_str(&format!("<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>", i + 1, crate::xml::escape(e))); }
        x.push_str("</CompleteMultipartUpload>");
        let r = self.call("POST", Some(key), &[("uploadId", id)], vec![("content-type".into(), "application/xml".into())], x.as_bytes(), 64 << 10)?;
        self.check(r)?;
        Ok(total)
    }
}

fn s3_error(r: &Response) -> Error {
    let doc = crate::xml::parse(r.text());
    let code = doc.as_ref().and_then(|d| d.find("Code")).map(|c| c.text.clone()).unwrap_or_default();
    match (r.status, code.as_str()) {
        (404, _) | (_, "NoSuchKey") | (_, "NoSuchBucket") | (_, "NoSuchUpload") => Error::NotFound,
        (_, "AccessDenied") | (401, _) => Error::Denied,
        (s, c) => {
            let msg = doc.as_ref().and_then(|d| d.find("Message")).map(|m| m.text.clone()).unwrap_or_default();
            Error::Remote(s as u32, if c.is_empty() { "S3 error".into() } else { format!("{c}: {msg}") })
        }
    }
}

impl<H: Http> Place for S3<H> {
    fn abilities(&self) -> Abilities { Abilities { atomic_write: true, atomic_dir_rename: false } }
    fn list(&mut self, dir: &str) -> Result<Vec<Entry>> {
        let parts = components(dir)?;
        let dk = self.key(&parts, true);
        let mut out = Vec::new();
        let mut token = None;
        let mut seen = parts.is_empty();
        loop {
            let l = self.list_page(&dk, true, token.as_deref(), None)?;
            for (k, size, mtime) in l.files {
                seen = true;
                let name = &k[dk.len()..];
                if name.is_empty() { continue; }
                out.push(Entry { name: name.into(), kind: Kind::File, size, mtime });
            }
            for p in l.dirs {
                seen = true;
                let name = p[dk.len()..].trim_end_matches('/');
                if !name.is_empty() { out.push(Entry { name: name.into(), kind: Kind::Dir, size: 0, mtime: None }); }
            }
            match l.next { Some(t) => token = Some(t), None => break }
        }
        if !seen { return Err(Error::NotFound); }
        crate::sort_entries(&mut out);
        Ok(out)
    }
    fn stat(&mut self, path: &str) -> Result<Entry> {
        let parts = components(path)?;
        let name = parts.last().unwrap_or(&"").to_string();
        if parts.is_empty() { return Ok(Entry { name, kind: Kind::Dir, size: 0, mtime: None }); }
        let k = self.key(&parts, false);
        if let Some((size, mtime)) = self.head(&k)? { return Ok(Entry { name, kind: Kind::File, size, mtime }); }
        if self.dir_exists(&self.key(&parts, true))? { return Ok(Entry { name, kind: Kind::Dir, size: 0, mtime: None }); }
        Err(Error::NotFound)
    }
    fn read(&mut self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        let parts = components(path)?;
        if parts.is_empty() { return Err(Error::BadPath); }
        if len == 0 { return Ok(Vec::new()); }
        let k = self.key(&parts, false);
        let limit = usize::try_from(offset.saturating_add(len)).unwrap_or(usize::MAX);
        let r = self.call("GET", Some(&k), &[], vec![("range".into(), format!("bytes={offset}-{}", offset.saturating_add(len - 1)))], &[], limit)?;
        match r.status {
            206 => { let mut b = r.body; b.truncate(len as usize); Ok(b) }
            200 => Ok(r.body.get(offset as usize..).unwrap_or(&[]).iter().copied().take(len as usize).collect()),
            416 => Ok(Vec::new()),
            _ => Err(s3_error(&r)),
        }
    }
    fn write(&mut self, path: &str, src: &mut dyn Source, _size: Option<u64>) -> Result<u64> {
        let parts = components(path)?;
        if parts.is_empty() { return Err(Error::BadPath); }
        let k = self.key(&parts, false);
        let mut buf = vec![0u8; self.cfg.part_size];
        let n = crate::read_full(src, &mut buf)?;
        buf.truncate(n);
        if n == self.cfg.part_size {
            // Is there more? Then this is a multipart upload.
            let mut next = vec![0u8; self.cfg.part_size];
            let m = crate::read_full(src, &mut next)?;
            if m > 0 {
                next.truncate(m);
                let mut rest: &[u8] = &next;
                let mut chained = Chain { a: &mut rest, b: src };
                return self.multipart(&k, buf, &mut chained);
            }
        }
        let r = self.call("PUT", Some(&k), &[], vec![("content-type".into(), "application/octet-stream".into())], &buf, 4096)?;
        self.check(r)?;
        Ok(n as u64)
    }
    fn mkdir(&mut self, path: &str) -> Result<()> {
        let parts = components(path)?;
        if parts.is_empty() { return Err(Error::BadPath); }
        if self.stat(path).is_ok() { return Err(Error::Exists); }
        let k = self.key(&parts, true);
        let r = self.call("PUT", Some(&k), &[], Vec::new(), &[], 4096)?;
        self.check(r).map(|_| ())
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let (f, t) = (components(from)?, components(to)?);
        if f.is_empty() || t.is_empty() { return Err(Error::BadPath); }
        if f == t { return Ok(()); }
        if self.stat(to).is_ok() { return Err(Error::Exists); }
        let e = self.stat(from)?;
        if e.kind == Kind::File {
            if e.size > COPY_MAX { return Err(Error::Unsupported); }
            let (fk, tk) = (self.key(&f, false), self.key(&t, false));
            self.copy_key(&fk, &tk)?;
            return self.delete_key(&fk);
        }
        let (fd, td) = (self.key(&f, true), self.key(&t, true));
        if td.starts_with(&fd) { return Err(Error::BadPath); }
        let keys = self.all_keys(&fd)?;
        if keys.iter().any(|(_, s)| *s > COPY_MAX) { return Err(Error::Unsupported); }
        // Copy all first, so a failure leaves the folder whole where it was.
        for (k, _) in &keys { let nk = format!("{td}{}", &k[fd.len()..]); self.copy_key(k, &nk)?; }
        for (k, _) in &keys { self.delete_key(k)?; }
        Ok(())
    }
    fn delete(&mut self, path: &str) -> Result<()> {
        let parts = components(path)?;
        if parts.is_empty() { return Err(Error::BadPath); }
        let e = self.stat(path)?;
        if e.kind == Kind::File { let k = self.key(&parts, false); return self.delete_key(&k); }
        let dk = self.key(&parts, true);
        let l = self.list_page(&dk, false, None, Some("2"))?;
        if l.files.iter().any(|(k, _, _)| *k != dk) { return Err(Error::NotEmpty); }
        self.delete_key(&dk)
    }
}

struct Chain<'a, 'b> { a: &'a mut dyn Source, b: &'b mut dyn Source }
impl Source for Chain<'_, '_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> { match self.a.read(buf)? { 0 => self.b.read(buf), n => Ok(n) } }
}
