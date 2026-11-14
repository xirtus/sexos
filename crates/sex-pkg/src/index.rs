// SPDX-License-Identifier: GPL-3.0-only
//! A store's catalog: one line per package, signed by a key the system trusts.
//!
//! ```text
//! # SexOS store
//! name=lua version=5.4.7 file=lua-5.4.7.sexpkg bytes=301234 sha256=HEX grants=none summary=The Lua language
//! ```
//!
//! `summary` comes last and runs to the end of the line. `file` is relative to the
//! catalog's own URL. The catalog says what exists; each package still carries its own
//! signature, and Files checks that one before installing anything.
//!
//! Optional fields for the store's app pages (older readers skip them, and a catalog
//! without them still parses):
//!
//! ```text
//! category=games featured=1 icon=icons/lua.png:HEX screenshot=shots/lua1.png:HEX
//! description=Percent%20encoded%20longer%20text%0Aon%20more%20lines
//! ```
//!
//! `icon` and each `screenshot` (up to [`SCREENSHOTS_MAX`]) name a picture relative to the
//! catalog's URL and its SHA-256: the signature over the catalog covers the hashes, so a
//! picture is checked before it is shown. `description` is percent-encoded (`%20` a space,
//! `%0A` a new line, `%25` a percent sign) so a line stays one line.
use crate::{valid_name, valid_rel, Error, Version, GRANTS};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;

pub const DOMAIN: &[u8] = b"SexOS store catalog v1\0";
pub const INDEX_FILE: &str = "sexos-store.txt";
pub const SIG_FILE: &str = "sexos-store.sig";
pub const MAX_BYTES: usize = 192 * 1024;
pub const SCREENSHOTS_MAX: usize = 4;
pub const DESCRIPTION_MAX: usize = 2000;
/// The category a package without one is listed under.
pub const OTHER: &str = "other";

/// A picture the catalog names: where it is, relative to the catalog, and its hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset { pub file: String, pub sha256: [u8; 32] }
impl Asset {
    fn parse(v: &str) -> Option<Self> {
        let (file, hash) = v.rsplit_once(':')?;
        if !valid_rel(file) || !is_picture(file) { return None; }
        Some(Asset { file: file.into(), sha256: crate::unhex::<32>(hash.as_bytes())? })
    }
    fn text(&self) -> String { alloc::format!("{}:{}", self.file, crate::hex(&self.sha256)) }
}
/// The picture kinds the store shows (what `sex-image` decodes).
pub fn is_picture(file: &str) -> bool {
    let lower = file.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".webp", ".gif"].iter().any(|x| lower.ends_with(x))
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Item {
    pub name: String,
    pub version: Version,
    pub file: String,
    pub bytes: u64,
    pub sha256: [u8; 32],
    pub grants: u32,
    pub summary: String,
    /// Lowercase word; empty means [`OTHER`].
    pub category: String,
    pub featured: bool,
    pub description: String,
    pub icon: Option<Asset>,
    pub screenshots: Vec<Asset>,
}
impl Item {
    pub fn category(&self) -> &str { if self.category.is_empty() { OTHER } else { &self.category } }
}

/// `description` as it goes into a line: every byte that is not plain printable ASCII
/// (and `%`, `=`, space) as `%XX`.
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_graphic() && b != b'%' && b != b'=' { out.push(b as char); } else { let _ = write!(out, "%{b:02X}"); }
    }
    out
}
pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(x) = crate::unhex::<1>(&b[i + 1..i + 3]) { out.push(x[0]); i += 3; continue; }
        }
        out.push(b[i]);
        i += 1;
    }
    let text = String::from_utf8_lossy(&out);
    text.chars().filter(|c| *c == '\n' || !c.is_control()).take(DESCRIPTION_MAX).collect()
}
fn category_ok(v: &str) -> bool { valid_name(v) }

/// One line's fields: everything before ` summary=` as key/value pairs, and the summary.
pub fn fields(line: &str) -> Result<(Vec<(&str, &str)>, &str), Error> {
    let (head, summary) = match line.find(" summary=") { Some(i) => (&line[..i], line[i + 9..].trim()), None => match line.strip_prefix("summary=") { Some(s) => ("", s.trim()), None => (line, "") } };
    let mut out = Vec::new();
    for kv in head.split_ascii_whitespace() {
        out.push(kv.split_once('=').ok_or(Error::Manifest("A catalog field has no '='."))?);
    }
    Ok((out, summary))
}

/// One catalog line, already split by [`fields`].
pub fn item(kvs: &[(&str, &str)], summary: &str) -> Result<Item, Error> {
    let (mut name, mut version, mut file, mut bytes, mut sha) = (None, None, None, None, None);
    let mut it = Item::default();
    for &(k, v) in kvs {
        match k {
            "name" if valid_name(v) => name = Some(v.to_string()),
            "version" => version = Version::parse(v),
            "file" if valid_rel(v) && v.ends_with(".sexpkg") => file = Some(v.to_string()),
            "bytes" => bytes = v.parse::<u64>().ok(),
            "sha256" => sha = crate::unhex::<32>(v.as_bytes()),
            "grants" => {
                for g in v.split(',').filter(|g| *g != "none") {
                    it.grants |= GRANTS.iter().find(|x| x.1 == g).ok_or(Error::Manifest("A catalog entry asks for an unknown grant."))?.0;
                }
            }
            "category" if category_ok(v) => it.category = v.into(),
            "featured" => it.featured = v == "1" || v == "yes",
            "description" => it.description = decode(v),
            "icon" => it.icon = Some(Asset::parse(v).ok_or(Error::Manifest("A catalog icon is not FILE:SHA256 of a picture."))?),
            "screenshot" => {
                let a = Asset::parse(v).ok_or(Error::Manifest("A catalog screenshot is not FILE:SHA256 of a picture."))?;
                if it.screenshots.len() < SCREENSHOTS_MAX { it.screenshots.push(a); }
            }
            _ => {}
        }
    }
    let (Some(name), Some(version), Some(file), Some(bytes), Some(sha256)) = (name, version, file, bytes, sha) else {
        return Err(Error::Manifest("A catalog line lacks name, version, file, bytes or sha256."));
    };
    it.name = name; it.version = version; it.file = file; it.bytes = bytes; it.sha256 = sha256;
    it.summary = summary.chars().filter(|c| !c.is_control()).take(120).collect();
    Ok(it)
}

pub fn parse(text: &str) -> Result<Vec<Item>, Error> {
    let mut items: Vec<Item> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (kvs, summary) = fields(line)?;
        let i = item(&kvs, summary)?;
        if items.iter().any(|o| o.name == i.name) { return Err(Error::Manifest("The catalog names one package twice.")); }
        items.push(i);
    }
    Ok(items)
}

/// Checks a catalog's signature (raw 64 bytes or 128 hex digits) against trusted keys.
pub fn verify(text: &[u8], sig: &[u8], trusted: &[[u8; 32]]) -> Result<Vec<Item>, Error> {
    if text.len() > MAX_BYTES { return Err(Error::TooLarge); }
    let sig: [u8; 64] = if sig.len() == 64 { sig.try_into().unwrap() } else { crate::unhex::<64>(sig).ok_or(Error::Signature)? };
    let mut msg = Vec::with_capacity(DOMAIN.len() + text.len());
    msg.extend_from_slice(DOMAIN);
    msg.extend_from_slice(text);
    let sig = ed25519_compact::Signature::new(sig);
    if !trusted.iter().any(|k| ed25519_compact::PublicKey::new(*k).verify(&msg, &sig).is_ok()) { return Err(Error::Untrusted); }
    parse(core::str::from_utf8(text).map_err(|_| Error::Manifest("The catalog is not UTF-8."))?)
}
pub fn sign(text: &[u8], seed: [u8; 32]) -> [u8; 64] {
    let kp = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(seed));
    let mut msg = Vec::with_capacity(DOMAIN.len() + text.len());
    msg.extend_from_slice(DOMAIN);
    msg.extend_from_slice(text);
    let mut out = [0u8; 64];
    out.copy_from_slice(kp.sk.sign(&msg, None).as_ref());
    out
}
pub fn grant_words(grants: u32) -> String {
    let words: Vec<&str> = GRANTS.iter().filter(|g| grants & g.0 != 0).map(|g| g.1).collect();
    if words.is_empty() { "none".to_string() } else { words.join(",") }
}
/// The fields of an entry before `summary=`, without a trailing space.
pub fn head(item: &Item) -> String {
    let mut s = alloc::format!("name={} version={} file={} bytes={} sha256={} grants={}", item.name, item.version, item.file, item.bytes, crate::hex(&item.sha256), grant_words(item.grants));
    if !item.category.is_empty() { let _ = write!(s, " category={}", item.category); }
    if item.featured { s.push_str(" featured=1"); }
    if let Some(i) = &item.icon { let _ = write!(s, " icon={}", i.text()); }
    for a in &item.screenshots { let _ = write!(s, " screenshot={}", a.text()); }
    if !item.description.is_empty() { let _ = write!(s, " description={}", encode(&item.description)); }
    s
}
/// Lines for a catalog entry, as `sexpkg index` writes them. An entry with only the
/// original fields is written exactly as before.
pub fn line(item: &Item) -> String { alloc::format!("{} summary={}\n", head(item), item.summary) }
/// Words matching every word of `query` (case-insensitive) in the name, summary,
/// category or description. `category:WORD` keeps one category; `is:featured` keeps the
/// featured ones.
pub fn search<'a>(items: &'a [Item], query: &str) -> Vec<&'a Item> {
    let q: Vec<String> = query.split_ascii_whitespace().map(|w| w.to_ascii_lowercase()).collect();
    items.iter().filter(|i| matches(i, &q)).collect()
}
pub fn matches(i: &Item, q: &[String]) -> bool {
    let hay = alloc::format!("{} {} {} {}", i.name, i.summary, i.category(), i.description).to_ascii_lowercase();
    q.iter().all(|w| match w.strip_prefix("category:") {
        Some(c) => i.category() == c,
        None if w == "is:featured" => i.featured,
        None => hay.contains(w.as_str()),
    })
}
/// The categories present, in first-seen order, each with its count.
pub fn categories(items: &[Item]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for i in items {
        match out.iter_mut().find(|c| c.0 == i.category()) { Some(c) => c.1 += 1, None => out.push((i.category().into(), 1)) }
    }
    out
}

/// One package as Files describes it to the store's window: where it comes from, what is
/// installed and kept, and its catalog entry. Its own line format, written by Files and
/// read by the Store:
///
/// `repo=R base=URL installed=VER|- previous=VER|- newer=0|1 <catalog fields> summary=...`
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Listing {
    pub repo: String,
    /// The repository's folder URL, ending in `/`: pictures are `base` + their file.
    pub base: String,
    pub installed: Option<Version>,
    pub previous: Option<Version>,
    pub newer: bool,
    pub item: Item,
}
impl Listing {
    pub fn line(&self) -> String {
        let v = |v: &Option<Version>| v.map(|v| v.to_string()).unwrap_or_else(|| "-".into());
        alloc::format!("repo={} base={} installed={} previous={} newer={} {} summary={}", self.repo, self.base, v(&self.installed), v(&self.previous), self.newer as u8, head(&self.item), self.item.summary)
    }
    pub fn parse(line: &str) -> Option<Self> {
        let (kvs, summary) = fields(line.trim()).ok()?;
        let mut l = Listing { item: item(&kvs, summary).ok()?, ..Default::default() };
        for &(k, v) in &kvs {
            match k {
                "repo" if valid_name(v) => l.repo = v.into(),
                "base" if !v.is_empty() => l.base = v.into(),
                "installed" => l.installed = Version::parse(v),
                "previous" => l.previous = Version::parse(v),
                "newer" => l.newer = v == "1",
                _ => {}
            }
        }
        (!l.repo.is_empty()).then_some(l)
    }
    /// The name the store and `pkg get` use: `NAME` from the main store, `REPO/NAME` otherwise.
    pub fn full_name(&self) -> String {
        if self.repo == crate::repo::MAIN { self.item.name.clone() } else { alloc::format!("{}/{}", self.repo, self.item.name) }
    }
}
