// SPDX-License-Identifier: GPL-3.0-only
//! Signed updates into the system slot that is not running, with one try before the new
//! system becomes the default.
//!
//! A release is three files: the boot image, an index naming it (version, size, SHA-256)
//! and an Ed25519 signature over the index. Only an index signed by the release key this
//! system was built with, newer than the running system, is used, and only an image whose
//! size and hash are the ones the index names.
//!
//! The boot menu carries the slot state in a comment line, rewritten in place:
//!
//! ```text
//! # sexos-state version=7 candidate=8 current=b trial=pending prev=3
//! DEFAULT_ENTRY=1
//! ```
//!
//! `current` is the slot whose entries come first; `prev` is the number of the entry that
//! starts the other slot. After an update the new slot is current and the trial pending,
//! so the next start runs it. As soon as Files starts there it marks the trial started and
//! makes the previous system the default, so a crash or hang from then on falls back on
//! the next start. When the desktop answers (Spindle's first request) the trial passes and
//! the new system becomes the default for good. A start in the previous slot during a
//! trial marks it failed. Limine cannot count tries itself, so a failure before Files
//! starts repeats until someone picks "SexOS previous system" in the menu, which the menu
//! always shows.
use crate::fat::{self, Fat};
use crate::gpt::{Extent, Gpt, Guid, TYPE_ESP, TYPE_SEXOS_BOOT};
use crate::install::{compose, pad, MENU_BYTES, MENU_MARK, MENU_PATH};
use crate::{BlockDev, Error, Window, SECTOR};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sha2::{Digest, Sha256};

pub const STATE_TAG: &str = "# sexos-state ";
pub const PREVIOUS_TITLE: &str = ":SexOS previous system";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot { A, B }
impl Slot {
    pub fn other(self) -> Self { match self { Slot::A => Slot::B, Slot::B => Slot::A } }
    fn letter(self) -> &'static str { match self { Slot::A => "a", Slot::B => "b" } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trial { None, Pending, Started, Failed }
impl Trial {
    fn word(self) -> &'static str { match self { Trial::None => "none", Trial::Pending => "pending", Trial::Started => "started", Trial::Failed => "failed" } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    /// The version of the last system that passed its trial (0: as installed).
    pub version: u64,
    /// The version being tried, or last tried.
    pub candidate: u64,
    pub current: Slot,
    pub trial: Trial,
    /// The number of the entry that starts the other slot (0: it holds no system).
    pub prev: u32,
}
impl State {
    pub fn fresh() -> Self { Self { version: 0, candidate: 0, current: Slot::A, trial: Trial::None, prev: 0 } }
    pub fn line(&self) -> String {
        format!("{STATE_TAG}version={} candidate={} current={} trial={} prev={}\n", self.version, self.candidate, self.current.letter(), self.trial.word(), self.prev)
    }
    /// The entry Limine starts unasked.
    pub fn default_entry(&self) -> u32 {
        match self.trial { Trial::Started | Trial::Failed if self.prev != 0 => self.prev, _ => 1 }
    }
    pub fn parse(menu: &[u8]) -> Result<Self, Error> {
        if !menu.starts_with(MENU_MARK.as_bytes()) { return Err(Error::NotFound); }
        let text = core::str::from_utf8(menu).map_err(|_| Error::Corrupt)?;
        let line = text.lines().find_map(|l| l.strip_prefix(STATE_TAG)).ok_or(Error::NotFound)?;
        let mut st = State::fresh();
        let (mut cur, mut tri) = (false, false);
        for kv in line.split_ascii_whitespace() {
            let (k, v) = kv.split_once('=').ok_or(Error::Corrupt)?;
            let num = || v.parse::<u64>().map_err(|_| Error::Corrupt);
            match k {
                "version" => st.version = num()?,
                "candidate" => st.candidate = num()?,
                "prev" => st.prev = num()? as u32,
                "current" => { st.current = match v { "a" => Slot::A, "b" => Slot::B, _ => return Err(Error::Corrupt) }; cur = true; }
                "trial" => { st.trial = match v { "none" => Trial::None, "pending" => Trial::Pending, "started" => Trial::Started, "failed" => Trial::Failed, _ => return Err(Error::Corrupt) }; tri = true; }
                _ => {}
            }
        }
        if !cur || !tri { return Err(Error::Corrupt); }
        Ok(st)
    }
    /// What a start in `booted` changes, if anything.
    pub fn on_start(&self, booted: Slot) -> Option<State> {
        let mut next = *self;
        match self.trial {
            Trial::Pending if booted == self.current => next.trial = Trial::Started,
            Trial::Pending | Trial::Started if booted != self.current => next.trial = Trial::Failed,
            _ => return None,
        }
        Some(next)
    }
    /// What a working desktop in `booted` changes, if anything.
    pub fn on_healthy(&self, booted: Slot) -> Option<State> {
        if !matches!(self.trial, Trial::Started | Trial::Pending) || booted != self.current { return None; }
        Some(State { version: self.version.max(self.candidate), trial: Trial::None, ..*self })
    }
}

/// `menu` with its state line and default entry replaced by `state`'s, same size.
pub fn restate(menu: &[u8], state: &State) -> Result<Vec<u8>, Error> {
    State::parse(menu)?;
    let text = core::str::from_utf8(menu).map_err(|_| Error::Corrupt)?;
    let mut out = String::new();
    for line in text.lines() {
        if line.starts_with(STATE_TAG) { out.push_str(&state.line()); continue; }
        if line.starts_with("DEFAULT_ENTRY=") { out.push_str(&format!("DEFAULT_ENTRY={}\n", state.default_entry())); continue; }
        // The padding line goes; `pad` puts it back at the new length.
        if line.starts_with('#') && line.len() > 64 && line[1..].bytes().all(|b| b == b' ') { continue; }
        out.push_str(line);
        out.push('\n');
    }
    pad(out.into_bytes())
}

/// A signed index: which image, how big, its hash, and its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index { pub version: u64, pub image: String, pub sha256: [u8; 32], pub bytes: u64 }

fn hex_nibble(b: u8) -> Option<u8> {
    match b { b'0'..=b'9' => Some(b - b'0'), b'a'..=b'f' => Some(b - b'a' + 10), b'A'..=b'F' => Some(b - b'A' + 10), _ => None }
}
pub fn unhex<const N: usize>(text: &[u8]) -> Option<[u8; N]> {
    let text = text.trim_ascii();
    if text.len() != 2 * N { return None; }
    let mut out = [0u8; N];
    for (i, pair) in text.chunks(2).enumerate() { out[i] = hex_nibble(pair[0])? << 4 | hex_nibble(pair[1])?; }
    Some(out)
}

impl Index {
    pub fn parse(text: &[u8]) -> Result<Self, Error> {
        let text = core::str::from_utf8(text).map_err(|_| Error::Corrupt)?;
        let (mut version, mut image, mut sha256, mut bytes) = (None, None, None, None);
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let (k, v) = line.split_once('=').ok_or(Error::Corrupt)?;
            match k.trim() {
                "version" => version = v.trim().parse().ok(),
                "image" => image = Some(String::from(v.trim())),
                "sha256" => sha256 = unhex::<32>(v.as_bytes()),
                "bytes" => bytes = v.trim().parse().ok(),
                _ => {}
            }
        }
        let bytes: u64 = bytes.ok_or(Error::Corrupt)?;
        if bytes == 0 || bytes % SECTOR as u64 != 0 { return Err(Error::Corrupt); }
        Ok(Index { version: version.ok_or(Error::Corrupt)?, image: image.ok_or(Error::Corrupt)?, sha256: sha256.ok_or(Error::Corrupt)?, bytes })
    }
}

/// Checks `sig` (64 bytes, or 128 hex digits) over `index` with the release key, then reads it.
pub fn verify_index(index: &[u8], sig: &[u8], key: &[u8; 32]) -> Result<Index, Error> {
    let sig: [u8; 64] = if sig.len() == 64 { sig.try_into().unwrap() } else { unhex::<64>(sig).ok_or(Error::BadSignature)? };
    let pk = ed25519_compact::PublicKey::new(*key);
    pk.verify(index, &ed25519_compact::Signature::new(sig)).map_err(|_| Error::BadSignature)?;
    Index::parse(index)
}

/// Where an installed SexOS lives on its disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct System { pub a: (Extent, Guid), pub b: (Extent, Guid), pub esp: Extent, pub menu: fat::Extent }
impl System {
    pub fn find<D: BlockDev + ?Sized>(dev: &mut D) -> Result<Self, Error> {
        let gpt = Gpt::read(dev)?;
        let mut slots: Vec<(Extent, Guid)> = gpt.find_type(TYPE_SEXOS_BOOT).map(|e| (Extent { first: e.first(), sectors: e.sectors() }, e.part_guid())).collect();
        if slots.len() != 2 { return Err(Error::NotFound); }
        slots.sort_by_key(|s| s.0.first);
        for e in gpt.find_type(TYPE_ESP) {
            let esp = Extent { first: e.first(), sectors: e.sectors() };
            let mut w = Window::new(dev, esp.first, esp.sectors)?;
            let Ok(f) = Fat::open(&mut w) else { continue };
            let Ok(at) = f.extent(&mut w, MENU_PATH) else { continue };
            if at.sectors as usize * SECTOR != MENU_BYTES { continue; }
            let menu = fat::Extent { first_lba: at.first_lba + esp.first, sectors: at.sectors };
            return Ok(System { a: slots[0], b: slots[1], esp, menu });
        }
        Err(Error::NotFound)
    }
    pub fn slot_of(&self, part: Guid) -> Option<Slot> {
        if part == self.a.1 { Some(Slot::A) } else if part == self.b.1 { Some(Slot::B) } else { None }
    }
    pub fn slot(&self, s: Slot) -> (Extent, Guid) { match s { Slot::A => self.a, Slot::B => self.b } }
    pub fn read_menu<D: BlockDev + ?Sized>(&self, dev: &mut D) -> Result<Vec<u8>, Error> {
        let mut m = vec![0u8; MENU_BYTES];
        dev.read(self.menu.first_lba, &mut m)?;
        Ok(m)
    }
    /// Rewrites the menu in place: the same sectors, so the ESP's FAT never changes.
    pub fn write_menu<D: BlockDev + ?Sized>(&self, dev: &mut D, menu: &[u8]) -> Result<(), Error> {
        if menu.len() != MENU_BYTES { return Err(Error::Invalid); }
        dev.write(self.menu.first_lba, menu)
    }
}

/// The entry blocks of a menu: each from its `:` line to the next.
fn blocks(menu: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in menu.lines() {
        if line.trim_start().starts_with(':') { out.push(String::new()); }
        if line.starts_with('#') && line[1..].bytes().all(|b| b == b' ') { continue; }
        if let Some(b) = out.last_mut() { b.push_str(line); b.push('\n'); }
    }
    out
}

/// The menu after an update into `target`: the new image's entries, then the running
/// system's first entry as "SexOS previous system", then the other systems' entries.
pub fn update_menu(old: &[u8], image_cfg: &str, target: (Slot, Guid), booted: Guid, version: u64) -> Result<Vec<u8>, Error> {
    let st = State::parse(old)?;
    let old = core::str::from_utf8(old).map_err(|_| Error::Corrupt)?;
    let mark = format!("guid://{}/", core::str::from_utf8(&booted.text()).unwrap());
    let all = blocks(old);
    let mut prev = all.iter().find(|b| b.contains(&mark)).cloned().ok_or(Error::NotFound)?;
    let first_line = prev.find('\n').unwrap_or(prev.len());
    prev.replace_range(..first_line, PREVIOUS_TITLE);
    let mut extra = vec![prev];
    extra.extend(all.into_iter().filter(|b| b.contains("PROTOCOL=efi_chainload")));
    let entries = image_cfg.lines().filter(|l| { let t = l.trim_start(); t.starts_with(':') && !t.starts_with("::") }).count() as u32;
    let state = State { version: st.version, candidate: version, current: target.0, trial: Trial::Pending, prev: entries + 1 };
    compose(image_cfg, target.1, &extra, &state)
}

const CHUNK: usize = 256;
/// SHA-256 of the first `bytes` of `dev`.
pub fn hash<D: BlockDev + ?Sized>(dev: &mut D, at: u64, bytes: u64) -> Result<[u8; 32], Error> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK * SECTOR];
    let sectors = bytes / SECTOR as u64;
    let mut done = 0;
    while done < sectors {
        let n = (sectors - done).min(CHUNK as u64) as usize;
        dev.read(at + done, &mut buf[..n * SECTOR])?;
        h.update(&buf[..n * SECTOR]);
        done += n as u64;
    }
    Ok(h.finalize().into())
}

/// Writes a verified image into the slot that is not running and points the menu at it
/// for one try. `booted` is the partition the running kernel came from. Returns the slot
/// written. Until the menu's last write the running system's menu is untouched, so a
/// failure anywhere before leaves every start as it was.
pub fn apply<D: BlockDev + ?Sized, I: BlockDev + ?Sized>(dev: &mut D, sys: &System, booted: Guid, index: &Index, image: &mut I, progress: &mut dyn FnMut(&str)) -> Result<Slot, Error> {
    let running = sys.slot_of(booted).ok_or(Error::Unsupported)?;
    let old = sys.read_menu(dev)?;
    let st = State::parse(&old)?;
    if index.version <= st.version { return Err(Error::Older); }
    let target = running.other();
    let (extent, guid) = sys.slot(target);
    let sectors = index.bytes / SECTOR as u64;
    if image.sectors() < sectors { return Err(Error::BadSignature); }
    if sectors > extent.sectors { return Err(Error::NoSpace); }
    progress("checking the image");
    if hash(image, 0, index.bytes)? != index.sha256 { return Err(Error::BadSignature); }
    let cfg = {
        let img = Fat::open(image)?;
        String::from_utf8(img.read_file(image, "/limine.cfg")?).map_err(|_| Error::Corrupt)?
    };
    let menu = update_menu(&old, &cfg, (target, guid), booted, index.version)?;
    progress("writing the other system slot");
    let mut buf = vec![0u8; CHUNK * SECTOR];
    let mut done = 0;
    while done < sectors {
        let n = (sectors - done).min(CHUNK as u64) as usize;
        image.read(done, &mut buf[..n * SECTOR])?;
        dev.write(extent.first + done, &buf[..n * SECTOR])?;
        done += n as u64;
    }
    progress("reading the slot back");
    if hash(dev, extent.first, index.bytes)? != index.sha256 { return Err(Error::Io); }
    progress("pointing the boot menu at it");
    sys.write_menu(dev, &menu)?;
    Ok(target)
}

#[cfg(test)]
mod tests;
