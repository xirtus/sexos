// SPDX-License-Identifier: GPL-3.0-only
//! Contiguous list selection for Loom's native folder view.
use core::ops::RangeInclusive;
use alloc::vec::Vec;

/// A shift-extended selection, clamped to the current listing. Without an
/// anchor the focused row alone is selected.
pub fn rows(anchor: Option<usize>, focus: usize, len: usize) -> Option<RangeInclusive<usize>> {
    if len == 0 { return None; }
    let focus = focus.min(len - 1);
    let anchor = anchor.unwrap_or(focus).min(len - 1);
    Some(anchor.min(focus)..=anchor.max(focus))
}

/// A cut group can move only to a different folder outside every selected
/// source. Comparing a slash boundary keeps `/docs2` distinct from `/docs`.
pub fn can_move_group_into(paths: &[Vec<u8>], destination: &[u8]) -> bool {
    !paths.is_empty() && paths.iter().all(|source| {
        let parent = source.iter().rposition(|b| *b == b'/').map_or(&b"/"[..], |at| {
            if at == 0 { &b"/"[..] } else { &source[..at] }
        });
        parent != destination && !(destination.starts_with(source)
            && destination.get(source.len()) == Some(&b'/')) && destination != source
    })
}

/// Candidate for a destination already containing `name`. Extensions stay at
/// the end, as in Loom's single-item copy naming.
pub fn collision_name(name: &[u8], tag: &str, number: usize) -> Vec<u8> {
    let (stem, extension) = match name.iter().rposition(|b| *b == b'.') {
        Some(at) if at > 0 => (&name[..at], &name[at..]),
        _ => (name, &name[name.len()..]),
    };
    let mut candidate = stem.to_vec();
    if number == 1 { candidate.extend_from_slice(alloc::format!(" ({tag})").as_bytes()); }
    else { candidate.extend_from_slice(alloc::format!(" ({tag} {number})").as_bytes()); }
    candidate.extend_from_slice(extension);
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reversed_shift_range_and_single_focus_stay_in_listing() {
        assert_eq!(rows(None, 2, 4).unwrap().collect::<alloc::vec::Vec<_>>(), alloc::vec![2]);
        assert_eq!(rows(Some(3), 1, 5).unwrap().collect::<alloc::vec::Vec<_>>(), alloc::vec![1, 2, 3]);
        assert_eq!(rows(Some(1), 4, 3).unwrap().collect::<alloc::vec::Vec<_>>(), alloc::vec![1, 2]);
        assert!(rows(Some(0), 0, 0).is_none());
    }

    #[test]
    fn batch_move_refuses_self_and_descendants_but_not_similar_names() {
        let paths = alloc::vec![b"/docs/one.txt".to_vec(), b"/docs/folder".to_vec()];
        assert!(!can_move_group_into(&paths, b"/docs"));
        assert!(!can_move_group_into(&paths, b"/docs/folder/sub"));
        assert!(can_move_group_into(&paths, b"/docs2"));
        assert!(can_move_group_into(&paths, b"/target"));
        assert!(!can_move_group_into(&[], b"/target"));
    }

    #[test]
    fn collision_candidates_keep_extensions_and_number_each_copy() {
        assert_eq!(collision_name(b"report.pdf", "copy", 1), b"report (copy).pdf");
        assert_eq!(collision_name(b"report.pdf", "copy", 2), b"report (copy 2).pdf");
        assert_eq!(collision_name(b".secret", "copy", 1), b".secret (copy)");
    }
}
