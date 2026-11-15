//! Path sanitising and the virtual directory view.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::{Entry, EntryKind, Error, Result};

/// Turn an archive member name into a safe relative path.
///
/// Separators are `/`; empty and `.` components are dropped. Names with a
/// `..` component, an absolute root, a drive prefix (`C:`) or a NUL byte are
/// rejected with [`Error::UnsafePath`]. An empty result (for example `./`)
/// is returned as `""`.
pub fn sanitize_path(raw: &str) -> Result<String> {
    if raw.contains('\0') || raw.starts_with('/') {
        return Err(Error::UnsafePath(raw.to_string()));
    }
    let mut out = String::with_capacity(raw.len());
    let mut first = true;
    for c in raw.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." {
            return Err(Error::UnsafePath(raw.to_string()));
        }
        let b = c.as_bytes();
        if first && b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
            return Err(Error::UnsafePath(raw.to_string()));
        }
        if !first {
            out.push('/');
        }
        out.push_str(c);
        first = false;
    }
    Ok(out)
}

/// Normalised display path and whether it is safe to extract.
pub(crate) fn normalize(raw: &str) -> (String, bool) {
    match sanitize_path(raw) {
        Ok(p) => (p, true),
        Err(_) => (raw.trim_end_matches('/').to_string(), false),
    }
}

/// One child in a virtual directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirItem {
    /// Last path component.
    pub name: String,
    /// Full normalised path inside the archive.
    pub path: String,
    pub kind: EntryKind,
    /// Index into [`crate::Archive::entries`]; `None` for a folder that is
    /// only implied by deeper paths.
    pub entry: Option<usize>,
    pub size: u64,
    pub mtime: Option<i64>,
}

impl DirItem {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }
}

pub(crate) fn trim_dir(path: &str) -> &str {
    path.trim_matches('/')
}

/// List the children of `dir` ("" is the root), synthesising implied
/// folders. Folders sort first, then names in byte order.
pub(crate) fn list_dir(entries: &[Entry], dir: &str) -> Result<Vec<DirItem>> {
    let dir = trim_dir(dir);
    let prefix_len = if dir.is_empty() { 0 } else { dir.len() + 1 };
    let mut found = dir.is_empty();
    let mut kids: BTreeMap<&str, DirItem> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        if !e.safe || e.path.is_empty() {
            continue;
        }
        let p = e.path.as_str();
        if p == dir {
            if e.kind == EntryKind::Dir {
                found = true;
                continue;
            }
            return Err(Error::NotADirectory);
        }
        if !dir.is_empty() && !(p.len() > dir.len() && p.starts_with(dir) && p.as_bytes()[dir.len()] == b'/') {
            continue;
        }
        found = true;
        let rest = &p[prefix_len..];
        match rest.find('/') {
            Some(slash) => {
                let name = &rest[..slash];
                kids.entry(name).or_insert_with(|| DirItem {
                    name: name.to_string(),
                    path: p[..prefix_len + slash].to_string(),
                    kind: EntryKind::Dir,
                    entry: None,
                    size: 0,
                    mtime: None,
                });
            }
            None => {
                let item = DirItem {
                    name: rest.to_string(),
                    path: p.to_string(),
                    kind: e.kind,
                    entry: Some(i),
                    size: e.size,
                    mtime: e.mtime,
                };
                match kids.get(rest) {
                    // An implied folder is upgraded by its explicit entry;
                    // a later duplicate member replaces an earlier one.
                    Some(old) if old.entry.is_none() && e.kind != EntryKind::Dir => {
                        kids.insert(rest, item);
                    }
                    _ => {
                        kids.insert(rest, item);
                    }
                }
            }
        }
    }
    if !found {
        return Err(Error::NoSuchEntry);
    }
    let mut v: Vec<DirItem> = kids.into_values().collect();
    v.sort_by(|a, b| b.is_dir().cmp(&a.is_dir()).then_with(|| a.name.cmp(&b.name)));
    Ok(v)
}
