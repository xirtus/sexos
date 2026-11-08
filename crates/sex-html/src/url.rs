// SPDX-License-Identifier: GPL-3.0-only
//! Addresses: resolving a link against the page it is on (RFC 3986 reference
//! resolution for the `http`/`https` cases pages use).
use alloc::string::String;
use alloc::vec::Vec;

/// (scheme with "://", authority, path, query-and-fragment) of an absolute address.
fn split(url: &str) -> Option<(&str, &str, &str, &str)> {
    let s = url.find("://")? + 3;
    let rest = &url[s..];
    let a = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let after = &rest[a..];
    let p = after.find(['?', '#']).unwrap_or(after.len());
    Some((&url[..s], &rest[..a], &after[..p], &after[p..]))
}

fn remove_dots(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let segs: Vec<&str> = path.split('/').collect();
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        match *seg {
            "." => { if last { out.push(""); } }
            ".." => { if out.len() > 1 { out.pop(); } if last { out.push(""); } }
            s => out.push(s),
        }
    }
    let mut p = out.join("/");
    if !p.starts_with('/') { p.insert(0, '/'); }
    p
}

/// `href` as an absolute address, seen from the page at `base`. `None` for schemes
/// the browser does not open (`mailto:`, `javascript:`, ...).
pub fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") { return Some(String::from(href)); }
    if let Some(colon) = href.find(':') {
        if href[..colon].bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.') && !href[..colon].is_empty()
            && !href[..colon].contains('/') { return None; }
    }
    let (scheme, auth, path, tail) = split(base)?;
    if let Some(rest) = href.strip_prefix("//") { return Some(alloc::format!("{}{}", scheme, rest)); }
    if href.is_empty() { return Some(alloc::format!("{}{}{}{}", scheme, auth, path, tail.split('#').next().unwrap_or(""))); }
    if href.starts_with('#') {
        let q = tail.split('#').next().unwrap_or("");
        return Some(alloc::format!("{}{}{}{}{}", scheme, auth, path, q, href));
    }
    if href.starts_with('?') { return Some(alloc::format!("{}{}{}{}", scheme, auth, if path.is_empty() { "/" } else { path }, href)); }
    let (hpath, htail) = href.split_at(href.find(['?', '#']).unwrap_or(href.len()));
    let merged = if hpath.starts_with('/') {
        remove_dots(hpath)
    } else {
        let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
        let dir = if dir.is_empty() { "/" } else { dir };
        remove_dots(&alloc::format!("{}{}", dir, hpath))
    };
    Some(alloc::format!("{}{}{}{}", scheme, auth, merged, htail))
}

/// The address without its `#fragment`, and the fragment.
pub fn fragment(url: &str) -> (&str, Option<&str>) {
    match url.find('#') { Some(i) => (&url[..i], Some(&url[i + 1..])), None => (url, None) }
}
