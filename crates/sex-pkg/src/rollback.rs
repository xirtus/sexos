// SPDX-License-Identifier: GPL-3.0-only
//! Per-package rollback: an upgrade keeps the version it replaced, and a rollback puts it
//! back (keeping the one it replaced in turn, so a second rollback undoes the first).
//!
//! An installed package's files are `/bin/PROGRAM`, its data files `/data/PROGRAM/REL`
//! (listed in its record's `files`) and its record `ROOT/PROGRAM/{manifest,files,repo}`.
//! The kept version lives whole under `ROOT/PROGRAM/prev/`: `bin`, `data/REL`, and its
//! own `manifest`, `files` and `repo`. Everything moves by rename, inside one filesystem
//! transaction, so nothing is copied and a failure part way changes nothing. Files the
//! program saved itself in `/data/PROGRAM` are not the package's and never move.
use crate::{valid_rel, Version};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The record files that travel with a version. `repo` is missing for packages installed
/// before repositories existed (they came from the main store).
pub const RECORDS: [&str; 3] = ["manifest", "files", "repo"];
pub const PREV: &str = "prev";
pub const SWAP: &str = "swap";

/// One rename. `optional`: the source may be missing (a record older installs lack).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move { pub from: String, pub to: String, pub optional: bool }

/// Where one version's parts are: installed (`None`) or under the record's `prev`/`swap`.
fn place(root: &str, program: &str, at: Option<&str>) -> (String, String, String) {
    match at {
        None => (format!("/bin/{program}"), format!("/data/{program}/"), format!("{root}/{program}/")),
        Some(d) => (format!("{root}/{program}/{d}/bin"), format!("{root}/{program}/{d}/data/"), format!("{root}/{program}/{d}/")),
    }
}
fn moves(root: &str, program: &str, files: &[u8], from: Option<&str>, to: Option<&str>, out: &mut Vec<Move>) {
    let (fb, fd, fr) = place(root, program, from);
    let (tb, td, tr) = place(root, program, to);
    out.push(Move { from: fb, to: tb, optional: false });
    for rel in data_files(files) { out.push(Move { from: format!("{fd}{rel}"), to: format!("{td}{rel}"), optional: true }); }
    for leaf in RECORDS { out.push(Move { from: format!("{fr}{leaf}"), to: format!("{tr}{leaf}"), optional: leaf != "manifest" }); }
}
/// The data files a `files` record lists (plain relative paths only).
pub fn data_files(files: &[u8]) -> Vec<&str> {
    files.split(|b| *b == b'\n').filter_map(|l| core::str::from_utf8(l).ok()).filter(|l| valid_rel(l)).collect()
}

/// Keeps the installed version as the previous one, before an upgrade writes the new one.
/// The old `prev` must be gone first.
pub fn keep(root: &str, program: &str, files: &[u8]) -> Vec<Move> {
    let mut out = Vec::new();
    moves(root, program, files, None, Some(PREV), &mut out);
    out
}
/// Swaps the installed version and the kept one: installed to `swap`, kept to installed,
/// `swap` to kept. `files` and `prev_files` are the two versions' `files` records.
pub fn swap(root: &str, program: &str, files: &[u8], prev_files: &[u8]) -> Vec<Move> {
    let mut out = Vec::new();
    moves(root, program, files, None, Some(SWAP), &mut out);
    moves(root, program, prev_files, Some(PREV), None, &mut out);
    moves(root, program, files, Some(SWAP), Some(PREV), &mut out);
    out
}
/// An install keeps the old version only when it is another version (a reinstall of the
/// same one has nothing to go back to).
pub fn keeps_previous(old: Version, new: Version) -> bool { old != new }
/// The person's answers stay when the version going in asks for nothing more than the
/// one going out; otherwise they are asked again.
pub fn consent_kept(out_grants: u32, in_grants: u32) -> bool { in_grants & !out_grants == 0 }
