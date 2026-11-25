// SPDX-License-Identifier: GPL-3.0-only
//! Read-only Git index facts for a project inside Quil's Files grant.
//! Version 2/3 SHA-1 indexes are understood; unsupported formats are reported.
use alloc::vec::Vec;
use alloc::{format, vec};
use alloc::string::{String, ToString};
use sha1::{Digest, Sha1};

const HEADER: usize = 12;
const ENTRY: usize = 62;
const CHECKSUM: usize = 20;
pub const MAX_INDEX: usize = 8 << 20;
pub const MAX_FILE: u64 = 8 << 20;
pub const MAX_DIFF_BYTES: usize = 64 << 10;
pub const MAX_DIFF_LINES: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffError { Blob, Limit, Binary }

/// A loose object is untrusted file content. Bound decompression, verify Git's
/// object ID and size, and accept only a blob before using it as a diff base.
pub fn loose_blob(compressed: &[u8], id: &[u8; 20]) -> Result<Vec<u8>, DiffError> {
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(compressed, MAX_FILE as usize + 64)
        .map_err(|_| DiffError::Blob)?;
    if Sha1::digest(&raw).as_slice() != id { return Err(DiffError::Blob); }
    let nul = raw.iter().position(|b| *b == 0).ok_or(DiffError::Blob)?;
    let size = core::str::from_utf8(raw.get(5..nul).ok_or(DiffError::Blob)?)
        .ok().and_then(|s| s.parse::<usize>().ok()).ok_or(DiffError::Blob)?;
    if !raw.starts_with(b"blob ") || size > MAX_FILE as usize || raw.len() - nul - 1 != size { return Err(DiffError::Blob); }
    Ok(raw[nul + 1..].to_vec())
}

/// A bounded line diff against the indexed blob. Equal lines provide context;
/// insertions and deletions are prefixed with + and -. No shell or Git process.
pub fn diff_text(before: &[u8], after: &[u8]) -> Result<Vec<String>, DiffError> {
    if before.len() > MAX_DIFF_BYTES || after.len() > MAX_DIFF_BYTES { return Err(DiffError::Limit); }
    if before.contains(&0) || after.contains(&0) { return Err(DiffError::Binary); }
    let a = core::str::from_utf8(before).map_err(|_| DiffError::Binary)?;
    let b = core::str::from_utf8(after).map_err(|_| DiffError::Binary)?;
    let a: Vec<&str> = a.lines().collect();
    let b: Vec<&str> = b.lines().collect();
    if a.len() > MAX_DIFF_LINES || b.len() > MAX_DIFF_LINES { return Err(DiffError::Limit); }
    let width = b.len() + 1;
    let mut lcs = vec![0u16; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i * width + j] = if a[i] == b[j] { lcs[(i + 1) * width + j + 1] + 1 }
                else { lcs[(i + 1) * width + j].max(lcs[i * width + j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push(format!("  {}", a[i])); i += 1; j += 1;
        } else if i < a.len() && (j == b.len() || lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
            out.push(format!("- {}", a[i])); i += 1;
        } else {
            out.push(format!("+ {}", b[j])); j += 1;
        }
    }
    if before.ends_with(b"\n") != after.ends_with(b"\n") { out.push(String::from("\\ No newline state differs")); }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tracked { pub path: Vec<u8>, pub id: [u8; 20], pub size: u32, pub stage: u8 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Header, Version, Truncated, Checksum, Path, Limit }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageError { Index(Error), Path, Conflict, Extension, Limit }

/// Prepare one tracked, unconflicted v2/v3 index entry and its loose blob.
/// Refuse index extensions: a cached tree or resolve-undo extension cannot be
/// preserved correctly by this small writer. Caller stages both with Files.
pub fn stage_tracked(index: &[u8], path: &[u8], bytes: &[u8]) -> Result<(Vec<u8>, [u8; 20], Vec<u8>), StageError> {
    if bytes.len() > MAX_DIFF_BYTES || bytes.contains(&0) || core::str::from_utf8(bytes).is_err() { return Err(StageError::Limit); }
    let entries = parse_index(index).map_err(StageError::Index)?;
    let mut at = HEADER;
    let body = index.len() - CHECKSUM;
    let mut target = None;
    for entry in entries {
        let flags = u16::from_be_bytes(index[at + 60..at + ENTRY].try_into().unwrap());
        if entry.path == path {
            if entry.stage != 0 || target.is_some() { return Err(StageError::Conflict); }
            target = Some(at);
        }
        let name_at = at + ENTRY + if flags & 0x4000 != 0 { 2 } else { 0 };
        let nul = index[name_at..body].iter().position(|b| *b == 0).unwrap() + name_at;
        at += (nul + 1 - at + 7) & !7;
    }
    if at != body { return Err(StageError::Extension); }
    let target = target.ok_or(StageError::Path)?;
    let id = blob_id(bytes);
    let mut raw = format!("blob {}\0", bytes.len()).into_bytes();
    raw.extend_from_slice(bytes);
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    let mut updated = index.to_vec();
    updated[target + 36..target + 40].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
    updated[target + 40..target + 60].copy_from_slice(&id);
    // Reset timestamps/device/inode, but preserve the tracked file mode and
    // ownership metadata. Git must refresh the worktree stat after staging.
    updated[target..target + 24].fill(0);
    let checksum: [u8; 20] = Sha1::digest(&updated[..body]).into();
    updated[body..].copy_from_slice(&checksum);
    Ok((updated, id, compressed))
}

pub fn parse_index(data: &[u8]) -> Result<Vec<Tracked>, Error> {
    if data.len() > MAX_INDEX { return Err(Error::Limit); }
    if data.len() < HEADER + CHECKSUM || &data[..4] != b"DIRC" { return Err(Error::Header); }
    let version = u32::from_be_bytes(data[4..8].try_into().unwrap());
    if version != 2 && version != 3 { return Err(Error::Version); }
    let count = u32::from_be_bytes(data[8..12].try_into().unwrap()) as usize;
    if count > 100_000 { return Err(Error::Limit); }
    let body = data.len() - CHECKSUM;
    let digest = Sha1::digest(&data[..body]);
    if digest.as_slice() != &data[body..] { return Err(Error::Checksum); }
    let mut at = HEADER;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let end = at.checked_add(ENTRY).filter(|end| *end <= body).ok_or(Error::Truncated)?;
        let mut id = [0; 20];
        id.copy_from_slice(&data[at + 40..at + 60]);
        let size = u32::from_be_bytes(data[at + 36..at + 40].try_into().unwrap());
        let flags = u16::from_be_bytes(data[at + 60..end].try_into().unwrap());
        let stage = ((flags >> 12) & 3) as u8;
        let name_at = end + if flags & 0x4000 != 0 { 2 } else { 0 };
        if name_at >= body { return Err(Error::Truncated); }
        let nul = data[name_at..body].iter().position(|b| *b == 0).ok_or(Error::Truncated)? + name_at;
        let path = &data[name_at..nul];
        if path.is_empty() || path[0] == b'/' || path.split(|b| *b == b'/').any(|part| part.is_empty() || part == b"." || part == b"..") {
            return Err(Error::Path);
        }
        if flags & 0x0FFF != 0x0FFF && flags as usize & 0x0FFF != path.len() { return Err(Error::Path); }
        entries.push(Tracked { path: path.to_vec(), id, size, stage });
        at = at.checked_add((nul + 1 - at + 7) & !7).filter(|next| *next <= body).ok_or(Error::Truncated)?;
    }
    Ok(entries)
}

pub fn blob_id(bytes: &[u8]) -> [u8; 20] {
    let mut h = Sha1::new();
    h.update(b"blob ");
    let n = bytes.len().to_string();
    h.update(n.as_bytes());
    h.update([0]);
    h.update(bytes);
    let mut id = [0; 20];
    id.copy_from_slice(&h.finalize());
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    const INDEX: &[u8] = include_bytes!("../../../tests/fixtures/quil-git-index");
    const DEMO: &[u8] = b"fn main() {\n    let needle = 1;\n    println!(\"{}\", needle);\n}\n";

    #[test]
    fn reads_real_git_index_and_detects_worktree_change() {
        let entries = parse_index(INDEX).unwrap();
        let demo = entries.iter().find(|e| e.path == b"demo.rs").unwrap();
        assert_eq!(demo.stage, 0);
        assert_eq!(demo.id, blob_id(DEMO));
        assert_ne!(demo.id, blob_id(b"edited\n"));
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn refuses_corrupt_or_unsupported_index() {
        let mut bad = INDEX.to_vec();
        bad[20] ^= 1;
        assert_eq!(parse_index(&bad), Err(Error::Checksum));
        bad = INDEX.to_vec();
        bad[7] = 4;
        assert_eq!(parse_index(&bad), Err(Error::Version));
        assert_eq!(parse_index(b"DIRC"), Err(Error::Header));
    }

    #[test]
    fn verifies_loose_blob_and_shows_separate_hunks() {
        let before = b"one\ntwo\nthree\nfour\n";
        let after = b"one\nTWO\nthree\nFOUR\n";
        let mut raw = b"blob 19\0".to_vec();
        raw.extend_from_slice(before);
        let id: [u8; 20] = Sha1::digest(&raw).into();
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        assert_eq!(loose_blob(&compressed, &id).unwrap(), before);
        assert_eq!(loose_blob(&compressed, &[0; 20]), Err(DiffError::Blob));
        assert_eq!(diff_text(before, after).unwrap(), ["  one", "- two", "+ TWO", "  three", "- four", "+ FOUR"]);
    }

    #[test]
    fn limits_diff_size_and_rejects_binary() {
        assert_eq!(diff_text(&vec![b'a'; MAX_DIFF_BYTES + 1], b""), Err(DiffError::Limit));
        assert_eq!(diff_text(b"a\0b", b"a\nb"), Err(DiffError::Binary));
    }

    #[test]
    fn stages_one_tracked_text_blob_without_rebuilding_other_entries() {
        let replacement = b"fn main() { println!(\"staged\"); }\n";
        let (updated, id, compressed) = stage_tracked(INDEX, b"demo.rs", replacement).unwrap();
        assert_eq!(id, blob_id(replacement));
        assert_eq!(loose_blob(&compressed, &id).unwrap(), replacement);
        let before = parse_index(INDEX).unwrap();
        let after = parse_index(&updated).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(after[0].path, before[0].path);
        assert_eq!(after[1], before[1]);
        assert_eq!(after[0].id, id);
        assert_eq!(&updated[HEADER + 24..HEADER + 28], &INDEX[HEADER + 24..HEADER + 28], "tracked file mode changed");
        assert_eq!(stage_tracked(INDEX, b"absent.rs", replacement), Err(StageError::Path));
        assert_eq!(stage_tracked(INDEX, b"demo.rs", b"bad\0bytes"), Err(StageError::Limit));
        let mut extended = INDEX[..INDEX.len() - CHECKSUM].to_vec();
        extended.extend_from_slice(b"TREE\0\0\0\0");
        let digest: [u8; 20] = Sha1::digest(&extended).into();
        extended.extend_from_slice(&digest);
        assert_eq!(stage_tracked(&extended, b"demo.rs", replacement), Err(StageError::Extension));
    }
}
