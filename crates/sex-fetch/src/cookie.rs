// SPDX-License-Identifier: GPL-3.0-only
//! Cookies (RFC 6265): kept per window by the browser, sent back to the site that set
//! them. `HttpOnly` cookies never reach page scripts; `Secure` ones only travel over
//! TLS. Sizes and counts are capped so a site can't fill the program's memory.

use crate::{parse_http_date, Url};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const MAX_COOKIES: usize = 3000;
const MAX_PER_DOMAIN: usize = 180;
const MAX_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Lowercase, without a leading dot.
    pub domain: String,
    /// Sent only to exactly `domain` (no `Domain` attribute was given).
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Seconds since 1970; `None` lasts for the session.
    pub expires: Option<u64>,
    created: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Jar {
    cookies: Vec<Cookie>,
    serial: u64,
}

fn is_ip(host: &str) -> bool { host.bytes().all(|b| b.is_ascii_digit() || b == b'.') || host.contains(':') }

fn domain_match(host: &str, domain: &str) -> bool {
    host == domain || (!is_ip(host) && host.len() > domain.len() && host.ends_with(domain) && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}

fn path_match(request: &str, cookie: &str) -> bool {
    request == cookie || (request.starts_with(cookie) && (cookie.ends_with('/') || request.as_bytes().get(cookie.len()) == Some(&b'/')))
}

fn default_path(url: &Url) -> String {
    let p = url.path();
    match p.rfind('/') { Some(0) | None => String::from("/"), Some(i) => String::from(&p[..i]) }
}

impl Jar {
    pub const fn new() -> Self { Jar { cookies: Vec::new(), serial: 0 } }
    pub fn len(&self) -> usize { self.cookies.len() }
    pub fn is_empty(&self) -> bool { self.cookies.is_empty() }
    pub fn iter(&self) -> impl Iterator<Item = &Cookie> { self.cookies.iter() }

    /// Takes one `Set-Cookie` header from a reply for `url`.
    pub fn store(&mut self, url: &Url, header: &str, now: u64) { self.set(url, header, now, true) }
    /// Takes a `document.cookie = ...` assignment from a page at `url`.
    pub fn store_from_script(&mut self, url: &Url, text: &str, now: u64) { self.set(url, text, now, false) }

    fn set(&mut self, url: &Url, header: &str, now: u64, from_http: bool) {
        if header.len() > MAX_BYTES { return; }
        let mut parts = header.split(';');
        let pair = parts.next().unwrap_or("");
        let (name, value) = match pair.find('=') { Some(i) => (pair[..i].trim(), pair[i + 1..].trim()), None => ("", pair.trim()) };
        if name.is_empty() && value.is_empty() { return; }
        if name.bytes().chain(value.bytes()).any(|b| b < 0x20 || b == 0x7f) { return; }
        let mut c = Cookie { name: name.to_string(), value: value.to_string(), domain: url.host.clone(), host_only: true, path: default_path(url), secure: false, http_only: false, expires: None, created: 0 };
        let mut max_age: Option<i64> = None;
        for attr in parts {
            let (k, v) = match attr.find('=') { Some(i) => (attr[..i].trim(), attr[i + 1..].trim()), None => (attr.trim(), "") };
            match k.to_ascii_lowercase().as_str() {
                "expires" => { if max_age.is_none() { c.expires = parse_http_date(v).or(Some(0)); } }
                "max-age" => {
                    if let Ok(n) = v.parse::<i64>() { max_age = Some(n); c.expires = Some(if n <= 0 { 0 } else { now.saturating_add(n as u64) }); }
                }
                "domain" => {
                    let d = v.trim_start_matches('.').to_ascii_lowercase();
                    if d.is_empty() { continue; }
                    // A cookie for someone else is refused.
                    if !domain_match(&url.host, &d) { return; }
                    // So is one for a whole public suffix (com, co.uk, github.io), which
                    // would reach every site under it; on that very host it stays host-only.
                    if !d.contains('.') || psl::suffix_str(&d) == Some(d.as_str()) {
                        if d == url.host { continue; }
                        return;
                    }
                    c.domain = d;
                    c.host_only = false;
                }
                "path" => { if v.starts_with('/') { c.path = v.to_string(); } }
                "secure" => c.secure = true,
                "httponly" => c.http_only = true,
                _ => {}
            }
        }
        if c.http_only && !from_http { return; }
        // __Secure- and __Host- prefixes promise more; hold sites to them.
        if (c.name.starts_with("__Secure-") || c.name.starts_with("__Host-")) && (!c.secure || !url.secure) { return; }
        if c.name.starts_with("__Host-") && (!c.host_only || c.path != "/") { return; }
        if c.secure && !url.secure { return; }
        let old = self.cookies.iter().position(|o| o.name == c.name && o.domain == c.domain && o.path == c.path);
        if let Some(i) = old {
            if self.cookies[i].http_only && !from_http { return; }
            c.created = self.cookies[i].created;
            self.cookies.remove(i);
        } else {
            self.serial += 1;
            c.created = self.serial;
        }
        if c.expires.is_some_and(|e| e <= now) { return; }
        self.cookies.push(c);
        self.trim(now);
    }

    fn trim(&mut self, now: u64) {
        self.cookies.retain(|c| c.expires.map_or(true, |e| e > now));
        let newest = self.cookies.last().map(|c| c.domain.clone());
        if let Some(d) = newest {
            while self.cookies.iter().filter(|c| c.domain == d).count() > MAX_PER_DOMAIN {
                let oldest = self.cookies.iter().position(|c| c.domain == d).unwrap();
                self.cookies.remove(oldest);
            }
        }
        while self.cookies.len() > MAX_COOKIES { self.cookies.remove(0); }
    }

    fn matching(&self, url: &Url, now: u64, scripts: bool) -> Vec<&Cookie> {
        let path = url.path();
        let mut v: Vec<&Cookie> = self.cookies.iter().filter(|c| {
            c.expires.map_or(true, |e| e > now)
                && (if c.host_only { url.host == c.domain } else { domain_match(&url.host, &c.domain) })
                && path_match(path, &c.path)
                && (!c.secure || url.secure)
                && !(scripts && c.http_only)
        }).collect();
        // Longer paths first, then older cookies first.
        v.sort_by(|a, b| b.path.len().cmp(&a.path.len()).then(a.created.cmp(&b.created)));
        v
    }

    fn join(list: Vec<&Cookie>) -> String {
        let mut s = String::new();
        for c in list {
            if !s.is_empty() { s.push_str("; "); }
            if !c.name.is_empty() { s.push_str(&c.name); s.push('='); }
            s.push_str(&c.value);
        }
        s
    }
    /// The `Cookie` header value for a request to `url`, or `None` when nothing applies.
    pub fn header(&self, url: &Url, now: u64) -> Option<String> {
        let s = Self::join(self.matching(url, now, false));
        (!s.is_empty()).then_some(s)
    }
    /// What `document.cookie` reads on a page at `url`.
    pub fn script_view(&self, url: &Url, now: u64) -> String { Self::join(self.matching(url, now, true)) }
}
