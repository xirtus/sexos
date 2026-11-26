// SPDX-License-Identifier: GPL-3.0-only
//! Bounded paths inside Valentine's single Files grant.
use alloc::vec::Vec;

pub const MAX_DEPTH: usize = 4;
pub const MAX_FOLDERS: usize = 64;

pub fn valid_name(name: &[u8]) -> bool {
    !name.is_empty() && name.len() <= 80 && name != b"." && name != b".."
        && name.iter().all(|&b| b != b'/' && b != b'\\' && b >= 0x20 && b != 0x7f)
}

pub fn valid_folder(path: &[u8]) -> bool {
    if path.is_empty() { return true; }
    let mut depth = 0;
    for part in path.split(|&b| b == b'/') {
        depth += 1;
        if depth > MAX_DEPTH || !valid_name(part) { return false; }
    }
    true
}

pub fn child(parent: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    if !valid_folder(parent) || !valid_name(name) || parent.split(|&b| b == b'/').count() >= MAX_DEPTH && !parent.is_empty() { return None; }
    let mut path = Vec::with_capacity(parent.len() + name.len() + 1);
    if !parent.is_empty() { path.extend_from_slice(parent); path.push(b'/'); }
    path.extend_from_slice(name);
    Some(path)
}

pub fn file(folder: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    if !valid_name(name) || !valid_folder(folder) { return None; }
    let mut path = Vec::with_capacity(folder.len() + name.len() + 1);
    if !folder.is_empty() { path.extend_from_slice(folder); path.push(b'/'); }
    path.extend_from_slice(name);
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_inside_valentines_granted_folder() {
        assert_eq!(file(b"Travel", b"ticket.eml"), Some(b"Travel/ticket.eml".to_vec()));
        assert_eq!(file(b"Travel/2026", b"ticket.eml"), Some(b"Travel/2026/ticket.eml".to_vec()));
        assert_eq!(child(b"Travel", b"2026"), Some(b"Travel/2026".to_vec()));
        assert_eq!(child(b"Travel/2026/Asia/May", b"too-deep"), None);
        assert_eq!(file(b"", b"inbox.eml"), Some(b"inbox.eml".to_vec()));
        for bad in [b"../out.eml".as_slice(), b"..", b"a/b", b"a\\b", b"\0", b"\x7f"] {
            assert_eq!(file(b"Travel", bad), None);
        }
        assert_eq!(file(b"../escape", b"mail.eml"), None);
        assert_eq!(file(b"Travel/../escape", b"mail.eml"), None);
        assert_eq!(file(b"Travel//escape", b"mail.eml"), None);
        assert_eq!(child(b"Travel/../escape", b"mail"), None);
    }
}
