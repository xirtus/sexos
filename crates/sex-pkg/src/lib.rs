// SPDX-License-Identifier: GPL-3.0-only
//! SexOS packages: one signed file holding a program, its data and the grants it asks for.
//!
//! ```text
//! "SEXPKG01"  u32 manifest bytes  u32 entry count          (16 byte head)
//! manifest: UTF-8 lines `key=value`
//! entry * count: u16 name bytes, u64 data bytes, name, data
//! "SIG1"  publisher key (32)  Ed25519 signature (64)        (100 byte trailer)
//! ```
//!
//! The signature covers SHA-256 of [`DOMAIN`] followed by every byte before the trailer,
//! so a reader checks it in one pass without holding the file. Only a key the system
//! trusts counts: the release key it was built with, or a key the person added. A package
//! is data until Files checks it; nothing in it grants anything by being there. Its
//! manifest only *asks* for grants (network, microphone), and the person answers.
//!
//! Entry names say where each file goes: `bin/PROGRAM` is the one program (installed as
//! `/bin/PROGRAM`), `data/PATH` lands in the program's own folder `/data/PROGRAM/PATH`.
#![no_std]
extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;
use sha2::{Digest, Sha256};

pub const MAGIC: [u8; 8] = *b"SEXPKG01";
pub const HEAD: usize = 16;
pub const TRAILER: usize = 100;
pub const TRAILER_TAG: [u8; 4] = *b"SIG1";
/// Signed before the body, so a package signature can never pass as any other signature.
pub const DOMAIN: &[u8] = b"SexOS package v1\0";
pub const MANIFEST_MAX: usize = 4096;
pub const ENTRIES_MAX: u32 = 4096;
pub const ENTRY_HEAD: usize = 10;
pub const ENTRY_NAME_MAX: usize = 200;
/// A program and its names fit the program store's 16 bytes.
pub const NAME_MAX: usize = 16;
/// The largest single file in a package.
pub const FILE_MAX: u64 = 512 * 1024 * 1024;
/// The installed program's largest image (the program store's limit).
pub const PROGRAM_MAX: u64 = 32 * 1024 * 1024;

/// What a package may ask the person for. Bits, as stored in a consent record.
pub const GRANT_NETWORK: u32 = 1;
pub const GRANT_MICROPHONE: u32 = 2;
pub const GRANTS: [(u32, &str); 2] = [(GRANT_NETWORK, "network"), (GRANT_MICROPHONE, "microphone")];
pub fn grant_name(bit: u32) -> &'static str { GRANTS.iter().find(|g| g.0 == bit).map(|g| g.1).unwrap_or("?") }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The file ends before its parts do.
    Short,
    /// Not a package.
    Magic,
    Manifest(&'static str),
    Entry(&'static str),
    /// The signature does not match the contents.
    Signature,
    /// Signed, but by a key this system does not trust.
    Untrusted,
    TooLarge,
}
impl Error {
    pub fn why(self) -> &'static str {
        match self {
            Error::Short => "The package is cut short.",
            Error::Magic => "That file is not a SexOS package.",
            Error::Manifest(m) | Error::Entry(m) => m,
            Error::Signature => "The package was changed after it was signed.",
            Error::Untrusted => "The package is signed by a publisher this system does not trust. Trust it first with: pkg trust KEY",
            Error::TooLarge => "The package is too large.",
        }
    }
}

/// A dotted version, up to four numbers: 5.4.7 < 5.10.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub [u32; 4]);
impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let mut v = [0u32; 4];
        let mut n = 0;
        for part in s.split('.') {
            if n == 4 || part.is_empty() || part.len() > 9 || !part.bytes().all(|b| b.is_ascii_digit()) { return None; }
            v[n] = part.parse().ok()?;
            n += 1;
        }
        Some(Version(v))
    }
}
impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let last = self.0.iter().rposition(|x| *x != 0).unwrap_or(0).max(1);
        for (i, x) in self.0[..=last].iter().enumerate() { if i > 0 { f.write_char('.')?; } write!(f, "{x}")?; }
        Ok(())
    }
}

/// A name a program, package or store entry may carry: lowercase letters, digits, `-`, `_`.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= NAME_MAX && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && s.as_bytes()[0].is_ascii_alphanumeric()
}
/// A relative path inside a program's folder: plain parts, no `.`/`..`, no spaces.
pub fn valid_rel(s: &str) -> bool {
    !s.is_empty() && s.len() <= ENTRY_NAME_MAX && !s.starts_with('/') && !s.ends_with('/')
        && s.split('/').all(|p| !p.is_empty() && p != "." && p != ".." && p.bytes().all(|b| b.is_ascii_graphic() && b != b'\\'))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: Version,
    pub summary: String,
    /// The `/bin` name it installs; the package's name unless it says otherwise.
    pub program: String,
    pub grants: u32,
    pub license: String,
}
impl Manifest {
    pub fn new(name: &str, version: &str) -> Self {
        Manifest { name: name.into(), version: Version::parse(version).unwrap_or_default(), summary: String::new(), program: name.into(), grants: 0, license: String::new() }
    }
    pub fn parse(text: &str) -> Result<Self, Error> {
        let (mut name, mut version, mut summary, mut program, mut grants, mut license) = (None, None, String::new(), None, 0u32, String::new());
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let (k, v) = line.split_once('=').ok_or(Error::Manifest("A manifest line has no '='."))?;
            let v = v.trim();
            match k.trim() {
                "name" => { if !valid_name(v) { return Err(Error::Manifest("The package name must be 1 to 16 lowercase letters, digits, '-' or '_'.")); } name = Some(v.to_string()) }
                "version" => version = Some(Version::parse(v).ok_or(Error::Manifest("The version must be numbers separated by dots."))?),
                "summary" => { summary = v.chars().filter(|c| !c.is_control()).take(120).collect() }
                "program" => { if !valid_name(v) { return Err(Error::Manifest("The program name must be 1 to 16 lowercase letters, digits, '-' or '_'.")); } program = Some(v.to_string()) }
                "license" => { license = v.chars().filter(|c| !c.is_control()).take(60).collect() }
                "grant" => grants |= GRANTS.iter().find(|g| g.1 == v).ok_or(Error::Manifest("The package asks for a grant SexOS does not know."))?.0,
                // Unknown keys are for newer tools; they grant nothing.
                _ => {}
            }
        }
        let name = name.ok_or(Error::Manifest("The manifest has no name."))?;
        let version = version.ok_or(Error::Manifest("The manifest has no version."))?;
        let program = program.unwrap_or_else(|| name.clone());
        Ok(Manifest { name, version, summary, program, grants, license })
    }
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let _ = write!(s, "name={}\nversion={}\nprogram={}\n", self.name, self.version, self.program);
        if !self.summary.is_empty() { let _ = writeln!(s, "summary={}", self.summary); }
        if !self.license.is_empty() { let _ = writeln!(s, "license={}", self.license); }
        for (bit, word) in GRANTS { if self.grants & bit != 0 { let _ = writeln!(s, "grant={word}"); } }
        s
    }
    pub fn grant_words(&self) -> String {
        let words: Vec<&str> = GRANTS.iter().filter(|g| self.grants & g.0 != 0).map(|g| g.1).collect();
        if words.is_empty() { "none".into() } else { words.join(",") }
    }
}

/// Where an entry goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `/bin/PROGRAM`.
    Program,
    /// `/data/PROGRAM/PATH`.
    Data(String),
}
pub fn target(name: &str, manifest: &Manifest) -> Result<Target, Error> {
    if let Some(p) = name.strip_prefix("bin/") {
        return if p == manifest.program { Ok(Target::Program) } else { Err(Error::Entry("A package installs only its own program into /bin.")) };
    }
    match name.strip_prefix("data/") {
        Some(rel) if valid_rel(rel) => Ok(Target::Data(rel.into())),
        _ => Err(Error::Entry("A package file must be bin/PROGRAM or data/PATH with plain path parts.")),
    }
}

/// The 16 byte head: manifest length and entry count.
pub fn parse_head(head: &[u8]) -> Result<(usize, u32), Error> {
    if head.len() < HEAD { return Err(Error::Short); }
    if head[..8] != MAGIC { return Err(Error::Magic); }
    let m = u32::from_le_bytes(head[8..12].try_into().unwrap()) as usize;
    let n = u32::from_le_bytes(head[12..16].try_into().unwrap());
    if m == 0 || m > MANIFEST_MAX { return Err(Error::Manifest("The manifest is empty or over 4 KiB.")); }
    if n == 0 || n > ENTRIES_MAX { return Err(Error::Entry("A package holds 1 to 4096 files.")); }
    Ok((m, n))
}
pub fn parse_entry_head(b: &[u8]) -> Result<(usize, u64), Error> {
    if b.len() < ENTRY_HEAD { return Err(Error::Short); }
    let n = u16::from_le_bytes([b[0], b[1]]) as usize;
    let size = u64::from_le_bytes(b[2..10].try_into().unwrap());
    if n == 0 || n > ENTRY_NAME_MAX + 5 { return Err(Error::Entry("A file name in the package is empty or too long.")); }
    if size > FILE_MAX { return Err(Error::TooLarge); }
    Ok((n, size))
}

/// The signature check, fed the file in order: everything before the trailer, then the trailer.
pub struct Verifier(Sha256);
impl Default for Verifier { fn default() -> Self { Self::new() } }
impl Verifier {
    pub fn new() -> Self { let mut h = Sha256::new(); h.update(DOMAIN); Verifier(h) }
    pub fn update(&mut self, body: &[u8]) { self.0.update(body); }
    /// The publisher key, if the trailer's signature matches and that key is trusted.
    pub fn finish(self, trailer: &[u8], trusted: &[[u8; 32]]) -> Result<[u8; 32], Error> {
        if trailer.len() != TRAILER { return Err(Error::Short); }
        if trailer[..4] != TRAILER_TAG { return Err(Error::Magic); }
        let key: [u8; 32] = trailer[4..36].try_into().unwrap();
        let sig: [u8; 64] = trailer[36..100].try_into().unwrap();
        let digest = self.0.finalize();
        ed25519_compact::PublicKey::new(key).verify(digest, &ed25519_compact::Signature::new(sig)).map_err(|_| Error::Signature)?;
        if !trusted.contains(&key) { return Err(Error::Untrusted); }
        Ok(key)
    }
}

/// One file inside a package: where it goes, and where its bytes are in the package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry { pub target: Target, pub offset: u64, pub size: u64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout { pub manifest: Manifest, pub entries: Vec<Entry> }

/// Reads a package's layout through `read(offset, buf)` (which fills `buf` or fails), for
/// a file `total` bytes long. Checks every rule except the signature: the files exactly
/// fill the body, exactly one is the program, no two land on the same path.
pub fn layout<E>(total: u64, mut read: impl FnMut(u64, &mut [u8]) -> Result<(), E>) -> Result<Layout, Result<Error, E>> {
    if total < (HEAD + TRAILER) as u64 { return Err(Ok(Error::Short)); }
    let body = total - TRAILER as u64;
    let mut head = [0u8; HEAD];
    read(0, &mut head).map_err(Err)?;
    let (mlen, count) = parse_head(&head).map_err(Ok)?;
    if (HEAD + mlen) as u64 > body { return Err(Ok(Error::Short)); }
    let mut text = alloc::vec![0u8; mlen];
    read(HEAD as u64, &mut text).map_err(Err)?;
    let text = core::str::from_utf8(&text).map_err(|_| Ok(Error::Manifest("The manifest is not UTF-8.")))?;
    let manifest = Manifest::parse(text).map_err(Ok)?;
    let mut at = (HEAD + mlen) as u64;
    let mut entries: Vec<Entry> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..count {
        if at + ENTRY_HEAD as u64 > body { return Err(Ok(Error::Short)); }
        let mut eh = [0u8; ENTRY_HEAD];
        read(at, &mut eh).map_err(Err)?;
        let (nlen, size) = parse_entry_head(&eh).map_err(Ok)?;
        at += ENTRY_HEAD as u64;
        if at + nlen as u64 > body { return Err(Ok(Error::Short)); }
        let mut name = alloc::vec![0u8; nlen];
        read(at, &mut name).map_err(Err)?;
        at += nlen as u64;
        let name = String::from_utf8(name).map_err(|_| Ok(Error::Entry("A file name in the package is not UTF-8.")))?;
        if seen.contains(&name) { return Err(Ok(Error::Entry("The package names one file twice."))); }
        let t = target(&name, &manifest).map_err(Ok)?;
        if t == Target::Program && size > PROGRAM_MAX { return Err(Ok(Error::TooLarge)); }
        if at.checked_add(size).is_none_or(|end| end > body) { return Err(Ok(Error::Short)); }
        entries.push(Entry { target: t, offset: at, size });
        seen.push(name);
        at += size;
    }
    if at != body { return Err(Ok(Error::Entry("The package has bytes no file accounts for."))); }
    if entries.iter().filter(|e| e.target == Target::Program).count() != 1 { return Err(Ok(Error::Entry("A package holds exactly one program, bin/PROGRAM."))); }
    // A data file may not sit where another needs a folder: `a` and `a/b`.
    for e in &entries {
        if let Target::Data(p) = &e.target {
            if entries.iter().any(|o| matches!(&o.target, Target::Data(q) if q.len() > p.len() && q.starts_with(p.as_str()) && q.as_bytes()[p.len()] == b'/')) {
                return Err(Ok(Error::Entry("A package file is also another file's folder.")));
            }
        }
    }
    Ok(Layout { manifest, entries })
}

/// A whole package in memory, checked: its layout and its publisher key.
pub fn verify(bytes: &[u8], trusted: &[[u8; 32]]) -> Result<(Layout, [u8; 32]), Error> {
    if bytes.len() < HEAD + TRAILER { return Err(Error::Short); }
    let (body, trailer) = bytes.split_at(bytes.len() - TRAILER);
    let layout = layout::<()>(bytes.len() as u64, |off, buf| {
        let s = off as usize;
        buf.copy_from_slice(bytes.get(s..s + buf.len()).ok_or(())?);
        Ok(())
    }).map_err(|e| e.unwrap_or(Error::Short))?;
    let mut v = Verifier::new();
    v.update(body);
    let key = v.finish(trailer, trusted)?;
    Ok((layout, key))
}

/// Builds and signs a package. `files` are (entry name, bytes).
pub fn build(manifest: &Manifest, files: &[(&str, &[u8])], seed: [u8; 32]) -> Result<Vec<u8>, Error> {
    let text = manifest.to_text();
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    for (name, data) in files {
        target(name, manifest)?;
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
    }
    sign(out, seed)
}
/// Appends the trailer for `body`.
pub fn sign(mut body: Vec<u8>, seed: [u8; 32]) -> Result<Vec<u8>, Error> {
    let kp = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(seed));
    let mut v = Verifier::new();
    v.update(&body);
    let digest = v.0.finalize();
    let sig = kp.sk.sign(digest, None);
    body.extend_from_slice(&TRAILER_TAG);
    body.extend_from_slice(kp.pk.as_ref());
    body.extend_from_slice(sig.as_ref());
    Ok(body)
}
pub fn public_key(seed: [u8; 32]) -> [u8; 32] {
    let kp = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(seed));
    let mut k = [0u8; 32];
    k.copy_from_slice(kp.pk.as_ref());
    k
}

pub fn hex(bytes: &[u8]) -> String { let mut s = String::new(); for b in bytes { let _ = write!(s, "{b:02x}"); } s }
pub fn unhex<const N: usize>(s: &[u8]) -> Option<[u8; N]> {
    let s = s.trim_ascii();
    if s.len() != 2 * N { return None; }
    let mut out = [0u8; N];
    for (i, pair) in s.chunks(2).enumerate() {
        let d = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        out[i] = d(pair[0])? << 4 | d(pair[1])?;
    }
    Some(out)
}
/// Keys from a trust file: one 64-digit hex key per line, `#` comments.
pub fn parse_keys(text: &[u8]) -> Vec<[u8; 32]> {
    text.split(|b| *b == b'\n').map(|l| l.split(|b| *b == b'#').next().unwrap_or(l)).filter_map(unhex::<32>).collect()
}

pub mod index;

#[cfg(test)]
mod tests;
