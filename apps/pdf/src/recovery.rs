// SPDX-License-Identifier: GPL-3.0-only
//! Complete PDF recovery copies. The source remains the authoritative file until
//! the person saves. An exact `@/file.pdf` grant writes the copy in Vellum's own
//! folder, never beside a file for which it has no sibling authority.

pub fn path(source: &str) -> String {
    if source.starts_with("@/") {
        let hash = source.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        format!("vellum-recovered-{hash:016x}.pdf")
    } else { format!("{source}.vellum-recovered.pdf") }
}

/// An incremental edit of exactly this source is longer and starts with every
/// original byte. Parsing the candidate PDF remains the caller's job.
pub fn belongs_to(source: &[u8], kept: &[u8]) -> bool {
    kept.len() > source.len() && kept.starts_with(source)
}

pub fn fingerprint(bytes: &[u8]) -> (usize, u64) {
    (bytes.len(), bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_requires_the_exact_source_prefix() {
        let source = b"%PDF-1.7\noriginal";
        assert!(belongs_to(source, b"%PDF-1.7\noriginal\nupdate"));
        assert!(!belongs_to(source, b"%PDF-1.7\nchanged\nupdate"));
        assert!(!belongs_to(source, source));
        assert_ne!(fingerprint(b"old"), fingerprint(b"new"));
        assert_eq!(path("@/documents/form.pdf"), path("@/documents/form.pdf"));
        assert_ne!(path("@/documents/form.pdf"), path("@/documents/other.pdf"));
        assert!(!path("@/documents/form.pdf").starts_with("@/"));
    }
}
