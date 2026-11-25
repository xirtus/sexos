// SPDX-License-Identifier: GPL-3.0-only
//! Read-only Git index facts for a project inside Quil's Files grant.
//! Version 2/3 SHA-1 indexes are understood; unsupported formats are reported.
use alloc::vec::Vec;
use alloc::string::ToString;
use sha1::{Digest, Sha1};

const HEADER: usize = 12;
const ENTRY: usize = 62;
const CHECKSUM: usize = 20;
pub const MAX_INDEX: usize = 8 << 20;
pub const MAX_FILE: u64 = 8 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tracked { pub path: Vec<u8>, pub id: [u8; 20], pub size: u32, pub stage: u8 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Header, Version, Truncated, Checksum, Path, Limit }

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
}
