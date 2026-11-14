// SPDX-License-Identifier: GPL-3.0-only
//! Loom's engines, kept apart from the window so they can be tested on the host.
//!
//! Loom (`apps/files`) lists folders, reads files and draws; this crate decides. It does
//! no I/O of its own: entries come in as plain values and file contents through the small
//! [`ReadAt`] trait, so every engine runs the same here and in tests.
//!
//! - [`rename`]: batch rename with a pattern language, live preview, conflicts and a safe order of moves.
//! - [`dupes`]: duplicate files by size, then a partial hash, then a full hash; stepwise.
//! - [`usage`]: a disk-usage tree laid out as a squarified treemap and as sunburst rings.
//! - [`tags`]: tags, ratings, colour labels and comments; smart folders; recent files; view memory.
//! - [`compare`]: two folder trees side by side, sync plans, and `.sha256` sum files.
//!
//! Paths and names are bytes (`Vec<u8>`), as Loom keeps them; `/` separates path parts.
//! Times are whole seconds since 1970-01-01 UTC.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod compare;
pub mod date;
pub mod dupes;
pub mod glob;
pub mod hash;
pub mod rename;
pub mod tags;
mod text;
pub mod usage;

/// Where file contents come from: Loom implements it over its own file calls, tests over memory.
pub trait ReadAt {
    /// Reads from `path` at `offset` into `buf`; returns how many bytes were read (0 at the end),
    /// or `None` when the file can't be read.
    fn read_at(&mut self, path: &[u8], offset: u64, buf: &mut [u8]) -> Option<usize>;
}

/// The last part of a path: `a/b/c.txt` -> `c.txt`.
pub fn base_name(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|b| *b == b'/') { Some(i) => &path[i + 1..], None => path }
}

/// The folder a path is in: `a/b/c.txt` -> `a/b`, `c.txt` -> empty.
pub fn parent(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|b| *b == b'/') { Some(i) => &path[..i], None => &[] }
}

/// True when `path` is `dir` itself or lies inside it.
pub fn is_within(path: &[u8], dir: &[u8]) -> bool {
    if dir.is_empty() { return true; }
    path.starts_with(dir) && (path.len() == dir.len() || path[dir.len()] == b'/' || dir.ends_with(b"/"))
}

/// `path` with its leading `from` replaced by `to` (for keys that follow a move).
pub fn rebase(path: &[u8], from: &[u8], to: &[u8]) -> alloc::vec::Vec<u8> {
    let mut out = to.to_vec();
    out.extend_from_slice(&path[from.len()..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths() {
        assert_eq!(base_name(b"a/b/c.txt"), b"c.txt");
        assert_eq!(base_name(b"c"), b"c");
        assert_eq!(parent(b"a/b/c"), b"a/b");
        assert_eq!(parent(b"c"), b"");
        assert!(is_within(b"a/b/c", b"a/b"));
        assert!(is_within(b"a/b", b"a/b"));
        assert!(!is_within(b"a/bc", b"a/b"));
        assert_eq!(rebase(b"a/b/c", b"a/b", b"x"), b"x/c");
    }
}
