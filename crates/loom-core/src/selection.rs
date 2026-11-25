// SPDX-License-Identifier: GPL-3.0-only
//! Contiguous list selection for Loom's native folder view.
use core::ops::RangeInclusive;

/// A shift-extended selection, clamped to the current listing. Without an
/// anchor the focused row alone is selected.
pub fn rows(anchor: Option<usize>, focus: usize, len: usize) -> Option<RangeInclusive<usize>> {
    if len == 0 { return None; }
    let focus = focus.min(len - 1);
    let anchor = anchor.unwrap_or(focus).min(len - 1);
    Some(anchor.min(focus)..=anchor.max(focus))
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
}
