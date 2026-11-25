// SPDX-License-Identifier: GPL-3.0-only
//! Candidate folders for Loom's compact local Places list. Files checks that
//! each candidate exists and is a folder using its own grant before showing it.
use alloc::vec::Vec;

use crate::is_within;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place { pub label: &'static str, pub path: Vec<u8> }

pub fn candidates(root: &[u8], cwd: &[u8], previous: Option<&[u8]>, home: &[u8]) -> Vec<Place> {
    let mut out = Vec::new();
    let mut add = |label, path: &[u8]| {
        if is_within(path, root) && !out.iter().any(|p: &Place| p.path == path) {
            out.push(Place { label, path: path.to_vec() });
        }
    };
    add("Start", root);
    if let Some(path) = previous.filter(|path| *path != cwd) { add("Previous", path); }
    add("Home", home);
    add("Downloads", b"/Downloads");
    add("Documents", b"/Documents");
    add("Disks", b"/media");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_stay_inside_the_granted_root_and_deduplicate() {
        let narrow = candidates(b"/media/LOOM", b"/media/LOOM/docs", Some(b"/media/LOOM"), b"/home");
        assert_eq!(narrow, [Place { label: "Start", path: b"/media/LOOM".to_vec() }]);
        let full = candidates(b"/", b"/Documents/tax", Some(b"/Documents"), b"/home");
        assert_eq!(full.iter().map(|p| p.label).collect::<Vec<_>>(),
                   ["Start", "Previous", "Home", "Downloads", "Disks"]);
        assert!(!full.iter().any(|p| p.path == b"/Documents" && p.label == "Documents"));
    }
}
