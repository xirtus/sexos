// SPDX-License-Identifier: GPL-3.0-only
//! Paths as a program writes them, turned into paths in its own folder.
//!
//! The program's folder is `/` to the program, and a path without a leading `/` is
//! from there too (a program has no other current folder). `.` and empty parts drop
//! out; `..` goes up one part and never above the folder.

/// Why a path cannot name anything in the program's folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused { AboveFolder, NotUtf8OrNul }

/// `path` as Files names it: parts joined by `/`, no leading `/`; empty for the folder.
pub fn resolve(path: &[u8]) -> Result<Vec<u8>, Refused> {
    if path.contains(&0) { return Err(Refused::NotUtf8OrNul); }
    let mut parts: Vec<&[u8]> = Vec::new();
    for part in path.split(|b| *b == b'/') {
        match part {
            b"" | b"." => {}
            b".." => { if parts.pop().is_none() { return Err(Refused::AboveFolder); } }
            p => parts.push(p),
        }
    }
    Ok(parts.join(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_is_root_and_dots_drop_out() {
        assert_eq!(resolve(b"/").unwrap(), b"");
        assert_eq!(resolve(b"").unwrap(), b"");
        assert_eq!(resolve(b"/notes/./a.txt").unwrap(), b"notes/a.txt");
        assert_eq!(resolve(b"notes//a.txt").unwrap(), b"notes/a.txt");
        assert_eq!(resolve(b"notes/x/../a.txt").unwrap(), b"notes/a.txt");
    }
    #[test]
    fn never_leaves_the_folder() {
        assert_eq!(resolve(b"../etc/passwd"), Err(Refused::AboveFolder));
        assert_eq!(resolve(b"/a/../../b"), Err(Refused::AboveFolder));
        assert_eq!(resolve(b"a\0b"), Err(Refused::NotUtf8OrNul));
    }
}
