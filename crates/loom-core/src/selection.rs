// SPDX-License-Identifier: GPL-3.0-only
//! Contiguous list selection for Loom's native folder view.
use core::ops::RangeInclusive;
use alloc::vec::Vec;

/// Bound one clipboard group so planning and rollback remain predictable.
pub const MAX_BATCH_COPY: usize = 64;

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

/// Reserve collision-free basenames for a copy group before any file changes.
/// `occupied` checks existing destinations; names reserved earlier in this
/// group also count as taken.
pub fn copy_names(names: &[Vec<u8>], mut occupied: impl FnMut(&[u8]) -> bool) -> Option<Vec<Vec<u8>>> {
    if names.is_empty() || names.len() > MAX_BATCH_COPY { return None; }
    let mut planned: Vec<Vec<u8>> = Vec::with_capacity(names.len());
    for name in names {
        let mut chosen = None;
        for number in 0..1000 {
            let candidate = if number == 0 { name.clone() } else { collision_name(name, "copy", number) };
            if !occupied(&candidate) && !planned.contains(&candidate) { chosen = Some(candidate); break; }
        }
        planned.push(chosen?);
    }
    Some(planned)
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

    #[test]
    fn batch_copy_reserves_every_name_before_writing() {
        let names = alloc::vec![b"one.txt".to_vec(), b"two.txt".to_vec(), b"one.txt".to_vec()];
        let chosen = copy_names(&names, |n| n == b"one.txt" || n == b"two.txt").unwrap();
        assert_eq!(chosen, alloc::vec![b"one (copy).txt".to_vec(), b"two (copy).txt".to_vec(), b"one (copy 2).txt".to_vec()]);
        assert!(copy_names(&[], |_| false).is_none());
        assert!(copy_names(&alloc::vec![b"x".to_vec(); MAX_BATCH_COPY + 1], |_| false).is_none());
    }
}
