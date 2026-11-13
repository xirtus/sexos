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
use crate::{valid_name, valid_rel, Error, Version, GRANTS};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub const DOMAIN: &[u8] = b"SexOS store catalog v1\0";
pub const INDEX_FILE: &str = "sexos-store.txt";
pub const SIG_FILE: &str = "sexos-store.sig";
pub const MAX_BYTES: usize = 192 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item { pub name: String, pub version: Version, pub file: String, pub bytes: u64, pub sha256: [u8; 32], pub grants: u32, pub summary: String }

pub fn parse(text: &str) -> Result<Vec<Item>, Error> {
    let mut items: Vec<Item> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (fields, summary) = match line.find(" summary=") { Some(i) => (&line[..i], line[i + 9..].trim()), None => (line, "") };
        let (mut name, mut version, mut file, mut bytes, mut sha, mut grants) = (None, None, None, None, None, 0u32);
        for kv in fields.split_ascii_whitespace() {
            let (k, v) = kv.split_once('=').ok_or(Error::Manifest("A catalog field has no '='."))?;
            match k {
                "name" if valid_name(v) => name = Some(v.to_string()),
                "version" => version = Version::parse(v),
                "file" if valid_rel(v) && v.ends_with(".sexpkg") => file = Some(v.to_string()),
                "bytes" => bytes = v.parse::<u64>().ok(),
                "sha256" => sha = crate::unhex::<32>(v.as_bytes()),
                "grants" => {
                    for g in v.split(',').filter(|g| *g != "none") {
                        grants |= GRANTS.iter().find(|x| x.1 == g).ok_or(Error::Manifest("A catalog entry asks for an unknown grant."))?.0;
                    }
                }
                _ => {}
            }
        }
        let (Some(name), Some(version), Some(file), Some(bytes), Some(sha256)) = (name, version, file, bytes, sha) else {
            return Err(Error::Manifest("A catalog line lacks name, version, file, bytes or sha256."));
        };
        if items.iter().any(|i| i.name == name) { return Err(Error::Manifest("The catalog names one package twice.")); }
        items.push(Item { name, version, file, bytes, sha256, grants, summary: summary.chars().filter(|c| !c.is_control()).take(120).collect() });
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
/// Lines for a catalog entry, as `sexpkg index` writes them.
pub fn line(item: &Item) -> String {
    let grants: Vec<&str> = GRANTS.iter().filter(|g| item.grants & g.0 != 0).map(|g| g.1).collect();
    let grants = if grants.is_empty() { "none".to_string() } else { grants.join(",") };
    alloc::format!("name={} version={} file={} bytes={} sha256={} grants={} summary={}\n", item.name, item.version, item.file, item.bytes, crate::hex(&item.sha256), grants, item.summary)
}
/// Words matching every word of `query` (case-insensitive) in the name or summary.
pub fn search<'a>(items: &'a [Item], query: &str) -> Vec<&'a Item> {
    let q: Vec<String> = query.split_ascii_whitespace().map(|w| w.to_ascii_lowercase()).collect();
    items.iter().filter(|i| {
        let hay = alloc::format!("{} {}", i.name, i.summary).to_ascii_lowercase();
        q.iter().all(|w| hay.contains(w.as_str()))
    }).collect()
}
