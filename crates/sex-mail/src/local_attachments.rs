// SPDX-License-Identifier: GPL-3.0-only
//! Names and picked paths for exporting a decoded local MIME attachment.
use alloc::vec::Vec;

/// MIME filenames are untrusted display text. The picker gets a single safe
/// basename, never a path supplied by the sender.
pub fn suggested_name(raw: &str) -> Vec<u8> {
    let base = raw.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(raw);
    let mut name = Vec::new();
    for ch in base.chars().take(96) {
        let b = ch as u32;
        name.push(if b <= 0x7f && (ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_')) { b as u8 } else { b'_' });
    }
    while name.first() == Some(&b'.') { name.remove(0); }
    while name.last() == Some(&b'.') { name.pop(); }
    if name.is_empty() { b"attachment".to_vec() } else { name }
}

/// Files returns an exact `@/` grant for the person's Save choice. Reject a
/// malformed answer before creating anything, even though Files also checks it.
pub fn picked_file(path: &[u8]) -> bool {
    path.starts_with(b"@/") && path.len() > 2 && path[2..].split(|b| *b == b'/').all(|part| {
        !part.is_empty() && part != b"." && part != b".."
            && part.iter().all(|b| *b >= 0x20 && *b != 0x7f && *b != b'\\')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_sender_cannot_choose_an_export_path() {
        assert_eq!(suggested_name("../../Private/plan.pdf"), b"plan.pdf");
        assert_eq!(suggested_name("C:\\Users\\Me\\photo.jpg"), b"photo.jpg");
        assert_eq!(suggested_name("\u{202e}gpj.exe"), b"_gpj.exe");
        assert_eq!(suggested_name("..."), b"attachment");
        assert!(picked_file(b"@/Documents/plan.pdf"));
        for bad in [b"/Documents/plan.pdf".as_slice(), b"@/Documents/../plan.pdf", b"@/Documents/", b"@/Documents/x\0.pdf"] {
            assert!(!picked_file(bad));
        }
    }
}
