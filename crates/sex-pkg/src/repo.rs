// SPDX-License-Identifier: GPL-3.0-only
//! Package repositories: the main store and the third-party ones the person added.
//!
//! The main store (`main`) is the one `pkg refresh URL` set; its catalog and packages are
//! checked against the keys the system trusts (the release key and `pkg trust` keys). A
//! third-party repository has its own name, folder URL and Ed25519 key, kept one per line
//! in the repository list:
//!
//! ```text
//! # name url key
//! extra https://example.org/sexos/ 64HEX
//! ```
//!
//! Its catalog counts only when signed by its own key, and its packages only when signed
//! by that key. Adding one is the person's trust decision (Spindle's `pkg repo add`).
//! An installed package remembers the repository it came from, and an upgrade from
//! another repository is refused: one repository cannot take over another's package.
use crate::index::Item;
use crate::{hex, unhex, valid_name, Version};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub const MAIN: &str = "main";
/// The most repositories a system keeps besides the main one.
pub const REPOS_MAX: usize = 8;
pub const URL_MAX: usize = 400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo { pub name: String, pub url: String, pub key: [u8; 32] }

/// A store folder URL: http(s), ends in `/`, no spaces.
pub fn valid_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://")) && url.ends_with('/') && url.len() <= URL_MAX
        && url.bytes().all(|b| b.is_ascii_graphic())
}

/// The repository list's lines; lines that do not parse are skipped.
pub fn parse(text: &[u8]) -> Vec<Repo> {
    let text = core::str::from_utf8(text).unwrap_or("");
    let mut out: Vec<Repo> = Vec::new();
    for line in text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()) {
        let mut w = line.split_ascii_whitespace();
        let (Some(name), Some(url), Some(key)) = (w.next(), w.next(), w.next()) else { continue };
        let Some(key) = unhex::<32>(key.as_bytes()) else { continue };
        if !valid_name(name) || name == MAIN || !valid_url(url) || out.iter().any(|r| r.name == name) { continue; }
        out.push(Repo { name: name.into(), url: url.into(), key });
    }
    out
}
pub fn text(repos: &[Repo]) -> String {
    let mut s = String::from("# SexOS package repositories: name url key\n");
    for r in repos { s.push_str(&alloc::format!("{} {} {}\n", r.name, r.url, hex(&r.key))); }
    s
}
/// Parses `NAME URL KEY` as typed (`pkg repo add`); a URL without its final `/` gets one.
pub fn parse_add(arg: &str) -> Result<Repo, &'static str> {
    let mut w = arg.split_ascii_whitespace();
    let (Some(name), Some(url), Some(key), None) = (w.next(), w.next(), w.next(), w.next()) else { return Err("Usage: pkg repo add NAME URL KEY") };
    if !valid_name(name) { return Err("A repository name is 1 to 16 lowercase letters, digits, '-' or '_'."); }
    if name == MAIN { return Err("'main' is the main store; set it with: pkg refresh URL"); }
    let mut url = url.to_string();
    if !url.ends_with('/') { url.push('/'); }
    if !valid_url(&url) { return Err("A repository is a folder URL starting with https:// or http://."); }
    let key = unhex::<32>(key.as_bytes()).ok_or("A repository key is 64 hex digits.")?;
    Ok(Repo { name: name.into(), url, key })
}
/// The list with `repo` added. The same repository again is refused, as is a name or URL
/// another one already has.
pub fn add(repos: &[Repo], repo: Repo) -> Result<Vec<Repo>, &'static str> {
    if repos.iter().any(|r| r.name == repo.name) { return Err("A repository by that name exists. Remove it first."); }
    if repos.iter().any(|r| r.url == repo.url) { return Err("A repository with that URL exists."); }
    if repos.len() >= REPOS_MAX { return Err("A system keeps at most 8 repositories."); }
    let mut out = repos.to_vec();
    out.push(repo);
    Ok(out)
}
pub fn remove(repos: &[Repo], name: &str) -> Option<Vec<Repo>> {
    let out: Vec<Repo> = repos.iter().filter(|r| r.name != name).cloned().collect();
    (out.len() != repos.len()).then_some(out)
}

/// The keys a repository's catalog and packages must be signed with.
pub fn keys_for(repo: &str, repos: &[Repo], trusted: &[[u8; 32]]) -> Vec<[u8; 32]> {
    if repo == MAIN { return trusted.to_vec(); }
    repos.iter().filter(|r| r.name == repo).map(|r| r.key).collect()
}
/// Every key a package may be signed with: the trusted ones and each repository's.
pub fn all_keys(repos: &[Repo], trusted: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let mut k = trusted.to_vec();
    for r in repos { if !k.contains(&r.key) { k.push(r.key); } }
    k
}
/// The repository a package signed by `key` comes from: the main store when the system
/// trusts the key, otherwise the first repository with that key.
pub fn origin(key: &[u8; 32], repos: &[Repo], trusted: &[[u8; 32]]) -> Option<String> {
    if trusted.contains(key) { return Some(MAIN.into()); }
    repos.iter().find(|r| &r.key == key).map(|r| r.name.clone())
}
/// May a package from `new` replace the installed one from `old`?
pub fn may_replace(old: &str, new: &str) -> bool { old == new }
/// `REPO/NAME` or `NAME`.
pub fn split_name(s: &str) -> (Option<&str>, &str) {
    match s.split_once('/') { Some((r, n)) => (Some(r), n), None => (None, s) }
}

/// One repository's kept catalog.
#[derive(Clone, Debug)]
pub struct Source { pub repo: String, pub url: String, pub items: Vec<Item> }
/// An installed package: its name, version and the repository it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Have { pub name: String, pub version: Version, pub repo: String }
/// What to download: the catalog entry and the repository's URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick { pub repo: String, pub url: String, pub item: Item }

/// The entry for `want` (`NAME` or `REPO/NAME`): the named repository's; else the one the
/// installed package came from; else the main store's; else the first repository's.
pub fn select(sources: &[Source], have: &[Have], want: &str) -> Option<Pick> {
    let (repo, name) = split_name(want);
    let in_repo = |r: &str| sources.iter().find(|s| s.repo == r).and_then(|s| s.items.iter().find(|i| i.name == name).map(|i| Pick { repo: s.repo.clone(), url: s.url.clone(), item: i.clone() }));
    if let Some(r) = repo { return in_repo(r); }
    if let Some(h) = have.iter().find(|h| h.name == name) { return in_repo(&h.repo); }
    in_repo(MAIN).or_else(|| sources.iter().find_map(|s| in_repo(&s.repo)))
}
/// Every installed package whose own repository has a newer version.
pub fn upgrades(sources: &[Source], have: &[Have]) -> Vec<Pick> {
    have.iter().filter_map(|h| {
        let s = sources.iter().find(|s| s.repo == h.repo)?;
        let i = s.items.iter().find(|i| i.name == h.name && i.version > h.version)?;
        Some(Pick { repo: s.repo.clone(), url: s.url.clone(), item: i.clone() })
    }).collect()
}
