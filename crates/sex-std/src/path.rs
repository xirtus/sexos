// SPDX-License-Identifier: GPL-3.0-only
//! Paths as a program writes them, turned into paths in its own folder.
//!
//! The program's folder is `/` to the program. A path without a leading `/` starts
//! from the working folder: the program's folder, unless the program that started this
//! run gave it another (`std::process::Command::current_dir`, handed on as a grant
//! `@/absolute/path`) or the program changed it (`std::env::set_current_dir`). `.` and
//! empty parts drop out; `..` goes up one part and never above the folder, nor out of
//! a granted path.

/// Why a path cannot name anything in the program's folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused { AboveFolder, NotUtf8OrNul }

/// The working folder as [`resolve`] spells it: empty for the program's own folder.
static CWD: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

/// The working folder, as [`resolve`] spells paths: empty for the program's folder,
/// `notes/2026` inside it, or `@/abs/path` for a granted folder.
pub fn cwd() -> Vec<u8> { CWD.lock().map(|c| c.clone()).unwrap_or_default() }
/// The working folder becomes `path` (resolved from the one before); its spelling.
pub fn set_cwd(path: &[u8]) -> Result<Vec<u8>, Refused> {
    let to = resolve(path)?;
    if let Ok(mut c) = CWD.lock() { *c = to.clone(); }
    Ok(to)
}
/// The working folder as the program sees it: `/` and the parts, or `@/abs/path`.
pub fn cwd_shown() -> Vec<u8> {
    let c = cwd();
    if c.starts_with(b"@") { c } else { let mut s = b"/".to_vec(); s.extend_from_slice(&c); s }
}

/// `path` as Files names it: parts joined by `/`, no leading `/`; empty for the folder.
/// A granted path keeps its `@` as its first part.
pub fn resolve(path: &[u8]) -> Result<Vec<u8>, Refused> {
    if path.contains(&0) { return Err(Refused::NotUtf8OrNul); }
    resolve_from(&cwd(), path)
}

/// `path` from the working folder `base` (as [`resolve`] spells it).
pub fn resolve_from(base: &[u8], path: &[u8]) -> Result<Vec<u8>, Refused> {
    if path.contains(&0) { return Err(Refused::NotUtf8OrNul); }
    let absolute = path.first() == Some(&b'/') || path.starts_with(b"@/") || path == b"@";
    let mut parts: Vec<&[u8]> = if absolute { Vec::new() } else { base.split(|b| *b == b'/').filter(|p| !p.is_empty()).collect() };
    let mut granted = parts.first() == Some(&&b"@"[..]);
    for (i, part) in path.split(|b| *b == b'/').enumerate() {
        match part {
            b"" | b"." => {}
            b"@" if i == 0 => { parts.push(part); granted = true; }
            b".." => {
                // `@` alone is the granted path's mark, not a part to climb out of.
                if parts.is_empty() || granted && parts.len() == 1 { return Err(Refused::AboveFolder); }
                parts.pop();
            }
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
    fn granted_paths_keep_their_mark() {
        assert_eq!(resolve_from(b"", b"@/data/a/x.txt").unwrap(), b"@/data/a/x.txt");
        assert_eq!(resolve_from(b"", b"@/data/a/../b").unwrap(), b"@/data/b");
        assert_eq!(resolve_from(b"@/data/a", b"../../..").err(), Some(Refused::AboveFolder));
    }
    #[test]
    fn relative_paths_start_at_the_working_folder() {
        assert_eq!(resolve_from(b"notes", b"a.txt").unwrap(), b"notes/a.txt");
        assert_eq!(resolve_from(b"notes", b"/a.txt").unwrap(), b"a.txt");
        assert_eq!(resolve_from(b"notes", b"../a.txt").unwrap(), b"a.txt");
        assert_eq!(resolve_from(b"@/data/a/work", b"out/x.txt").unwrap(), b"@/data/a/work/out/x.txt");
        assert_eq!(resolve_from(b"@/data/a/work", b"../x").unwrap(), b"@/data/a/x");
        assert_eq!(resolve_from(b"@/data/a/work", b"/x").unwrap(), b"x");
        assert_eq!(resolve_from(b"@/w", b".."), Ok(b"@".to_vec()));
        assert_eq!(resolve_from(b"@/w", b"../..").err(), Some(Refused::AboveFolder));
    }
    #[test]
    fn never_leaves_the_folder() {
        assert_eq!(resolve(b"../etc/passwd"), Err(Refused::AboveFolder));
        assert_eq!(resolve(b"/a/../../b"), Err(Refused::AboveFolder));
        assert_eq!(resolve(b"a\0b"), Err(Refused::NotUtf8OrNul));
    }
}
