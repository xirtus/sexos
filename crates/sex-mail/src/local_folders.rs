// SPDX-License-Identifier: GPL-3.0-only
//! Paths inside Valentine's single Files grant. Only immediate child folders are shown.
use alloc::vec::Vec;

pub fn valid_name(name: &[u8]) -> bool {
    !name.is_empty() && name.len() <= 80 && name != b"." && name != b".."
        && name.iter().all(|&b| b != b'/' && b != b'\\' && b != 0 && b >= 0x20)
}

pub fn file(folder: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    if !valid_name(name) || (!folder.is_empty() && !valid_name(folder)) { return None; }
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
        assert_eq!(file(b"", b"inbox.eml"), Some(b"inbox.eml".to_vec()));
        for bad in [b"../out.eml".as_slice(), b"..", b"a/b", b"a\\b", b"\0"] {
            assert_eq!(file(b"Travel", bad), None);
        }
        assert_eq!(file(b"../escape", b"mail.eml"), None);
    }
}
