// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Packages through Files: install, remove, list, trust, and the person's answers to the
//! grants a package asks for. One more request kind on the Files protocol (`fs`), with the
//! sub-operation in `offset`. Replies carry a report in the data area.
//!
//! Spindle may use every operation. A program may use it only when the person started it
//! with `run +pkg` (its run's arguments then hold [`GRANT_TOKEN`], which only Spindle can
//! put there), and then only INSTALL (never [`ALLOW_DOWNGRADE`]), CHECK, REMOVE, LIST,
//! CATALOG, SEARCH, LOCATE, SOURCE, REPO_REMOVE, REPOS and DETAILS. TRUST, CONSENT,
//! REPO_ADD and ROLLBACK stay Spindle's: they are the person's answers (whom to trust,
//! what to allow, going back to an older version). A program's package paths, [`DOWNLOAD`], [`CATALOG_DOWNLOAD`] and
//! [`SIG_DOWNLOAD`] are inside its own folder, as all its paths are.
//!
//! Installed packages are recorded under `/system/packages/PROGRAM/`: `manifest` (as
//! signed), `files` (what install wrote, for remove), `consent` (the person's answers),
//! `repo` (the repository it came from; missing means the main store) and `prev/` (the
//! version the last upgrade replaced, for ROLLBACK). Repositories are listed in [`REPOS_FILE`].
//! Programs never reach `/system`: their paths are confined to their own folder.

/// The request kind (`fs::Request::kind`). Far from the core kinds on purpose.
pub const KIND: u8 = 40;
/// Install or upgrade the package at the path. `length` bit 0 allows a downgrade.
pub const INSTALL: u64 = 0;
pub const ALLOW_DOWNGRADE: u64 = 1;
/// Remove the package or program named by the path.
pub const REMOVE: u64 = 1;
/// One line per installed package: `program name version grants`.
pub const LIST: u64 = 2;
/// Trust a publisher: the path is its 64-digit hex public key.
pub const TRUST: u64 = 3;
/// Record the person's answer for a program (the path): `length` holds the grants allowed
/// in bits 0..16 and the grants answered in bits 16..32.
pub const CONSENT: u64 = 4;
/// Check a package without installing it: the report says what it is and asks for.
pub const CHECK: u64 = 5;
/// A store catalog was downloaded to [`CATALOG_DOWNLOAD`] and [`SIG_DOWNLOAD`]: check its
/// signature, keep it, and remember the path (the store's folder URL) as the source.
pub const CATALOG: u64 = 6;
/// Packages in the kept catalog matching every word of the path.
pub const SEARCH: u64 = 7;
/// Where to download packages from: the path names one package, or is empty for every
/// installed package the catalog has newer. Report lines starting with [`MACHINE`] hold URLs.
pub const LOCATE: u64 = 8;
/// The saved store folder URL, as one [`MACHINE`] line, then one `URL NAME` line for each
/// third-party repository. CATALOG takes such a line: a bare URL is the main store, checked
/// against the trusted keys; `URL NAME` is that repository, checked against its own key.
pub const SOURCE: u64 = 9;
/// Add a third-party repository: the path is `NAME URL KEY`. Its key then signs that
/// repository's catalog and packages. The person's trust decision: Spindle only.
pub const REPO_ADD: u64 = 10;
/// Remove the repository named by the path (its installed packages stay).
pub const REPO_REMOVE: u64 = 11;
/// One line per repository: `NAME URL KEY16 packages=N`, and the updates waiting.
pub const REPOS: u64 = 12;
/// Put back the version the last upgrade of the package (the path) replaced, keeping the
/// current one in its place. A downgrade, so Spindle only.
pub const ROLLBACK: u64 = 13;
/// For the store's window: every catalog entry matching the path's words, one [`MACHINE`]
/// line each in `sex_pkg::index::Listing` form (repository, installed and kept versions,
/// category, pictures and their hashes, description).
pub const DETAILS: u64 = 14;
/// The run argument Spindle adds for `run +pkg`: this run may use package requests.
pub const GRANT_TOKEN: &[u8] = b"+packages";
/// Report lines for Spindle rather than the person start with this byte.
pub const MACHINE: u8 = 1;
pub const CATALOG_DOWNLOAD: &[u8] = b"/download.catalog";
pub const SIG_DOWNLOAD: &[u8] = b"/download.catalog.sig";

pub const ROOT: &[u8] = b"/system/packages";
pub const TRUSTED: &[u8] = b"/system/packages/trusted";
/// Third-party repositories, `NAME URL KEY` per line (`sex_pkg::repo`).
pub const REPOS_FILE: &[u8] = b"/system/packages/repos.txt";
/// Where `pkg install URL` downloads to. Files removes it once it has looked at it.
pub const DOWNLOAD: &[u8] = b"/download.sexpkg";

/// Grant bits (the same as `sex_pkg::GRANT_*`).
pub const NETWORK: u32 = 1;
pub const MICROPHONE: u32 = 2;

/// After a PREPARE of `/bin/PROGRAM` that a package installed, the data area starts with
/// this note: magic, then u32 grants asked, u32 grants answered, u32 grants allowed.
pub const NOTE_MAGIC: [u8; 8] = *b"PKGNOTE1";
pub const NOTE_BYTES: usize = 20;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Note { pub asked: u32, pub answered: u32, pub allowed: u32 }
impl Note {
    pub fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&NOTE_MAGIC);
        out[8..12].copy_from_slice(&self.asked.to_le_bytes());
        out[12..16].copy_from_slice(&self.answered.to_le_bytes());
        out[16..20].copy_from_slice(&self.allowed.to_le_bytes());
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < NOTE_BYTES || b[..8] != NOTE_MAGIC { return None; }
        let w = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        Some(Note { asked: w(8), answered: w(12), allowed: w(16) })
    }
    /// Grants asked and not yet answered: the person has to be asked.
    pub fn open(&self) -> u32 { self.asked & !self.answered }
}
/// The consent file's text and back: `network=yes` lines.
pub fn consent_text(answered: u32, allowed: u32, out: &mut [u8; 64]) -> usize {
    let mut n = 0;
    for (bit, word) in [(NETWORK, &b"network"[..]), (MICROPHONE, &b"microphone"[..])] {
        if answered & bit == 0 { continue; }
        let yes: &[u8] = if allowed & bit != 0 { b"=yes\n" } else { b"=no\n" };
        out[n..n + word.len()].copy_from_slice(word); n += word.len();
        out[n..n + yes.len()].copy_from_slice(yes); n += yes.len();
    }
    n
}
pub fn parse_consent(text: &[u8]) -> (u32, u32) {
    let (mut answered, mut allowed) = (0, 0);
    for line in text.split(|b| *b == b'\n') {
        let bit = if line.starts_with(b"network=") { NETWORK } else if line.starts_with(b"microphone=") { MICROPHONE } else { continue };
        answered |= bit;
        if line.ends_with(b"=yes") { allowed |= bit; }
    }
    (answered, allowed)
}
