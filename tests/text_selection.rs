//! Actual bounded selection state; no clipboard, IPC, layout or authority mocks.
#[path = "../crates/sex-pdx/src/text_selection.rs"]
mod text_selection;
use text_selection::{replacement_len, Error, Selection, Snapshot, Source, MAX_BYTES};

const A: Source = Source {
    generation: 7,
    revision: 0,
};
const B: Source = Source {
    generation: 8,
    revision: 0,
};
const EDITED: Source = Source {
    generation: 7,
    revision: 1,
};

fn bound(len: usize) -> Selection {
    let mut selection = Selection::new();
    selection.bind(A, len).unwrap();
    selection
}
fn state(selection: &Selection) -> (Source, usize, usize, usize) {
    let s = selection.snapshot().unwrap();
    (s.source(), s.len(), s.anchor(), s.caret())
}
fn unchanged(selection: &Selection, before: Snapshot) {
    assert!(selection.unchanged(before));
    assert_eq!(selection.snapshot(), Some(before));
}

#[test]
fn explicit_binding_and_clear_have_no_retained_body_or_implicit_document() {
    let mut selection = Selection::new();
    assert_eq!(selection.snapshot(), None);
    assert_eq!(selection.range(), None);
    assert_eq!(selection.left(A, false), Err(Error::Unbound));
    selection.bind(A, 5).unwrap();
    assert_eq!(state(&selection), (A, 5, 0, 0));
    selection.end(A, true).unwrap();
    let pending = selection.snapshot().unwrap();
    selection.clear();
    assert_eq!(selection.range(), None);
    assert!(!selection.unchanged(pending));
    selection.bind(B, 5).unwrap();
    assert_eq!(state(&selection), (B, 5, 0, 0));
    assert!(core::mem::size_of::<Selection>() < 128);
}

#[test]
fn generation_zero_is_invalid_but_revision_zero_and_maximal_ids_are_data() {
    let mut selection = bound(6);
    let before = selection.snapshot().unwrap();
    for revision in [0, 1, u64::MAX] {
        let invalid = Source {
            generation: 0,
            revision,
        };
        assert_eq!(selection.bind(invalid, 6), Err(Error::InvalidSource));
        unchanged(&selection, before);
        assert_eq!(
            selection.move_to(invalid, 1, true),
            Err(Error::InvalidSource)
        );
        unchanged(&selection, before);
    }
    selection
        .bind(
            Source {
                generation: u64::MAX,
                revision: u64::MAX,
            },
            MAX_BYTES,
        )
        .unwrap();
    assert_eq!(selection.snapshot().unwrap().source().generation, u64::MAX);
    selection.rebind(A, 0).unwrap();
    assert_eq!(state(&selection), (A, 0, 0, 0));
}

#[test]
fn forward_shift_extension_reversal_and_crossing_preserve_original_anchor() {
    let mut selection = bound(10);
    selection.move_to(A, 4, false).unwrap();
    for (move_right, range, caret) in [
        (true, (4, 5), 5),
        (true, (4, 6), 6),
        (false, (4, 5), 5),
        (false, (4, 4), 4),
        (false, (3, 4), 3),
        (false, (2, 4), 2),
    ] {
        if move_right {
            selection.right(A, true).unwrap();
        } else {
            selection.left(A, true).unwrap();
        }
        assert_eq!(selection.range(), Some(range));
        assert_eq!(state(&selection).2, 4);
        assert_eq!(state(&selection).3, caret);
    }
    selection.right(A, true).unwrap();
    assert_eq!(selection.range(), Some((3, 4)));
}

#[test]
fn unshifted_left_and_right_collapse_forward_and_reverse_ranges_without_extra_step() {
    for reverse in [false, true] {
        for right in [false, true] {
            let mut selection = bound(10);
            selection
                .move_to(A, if reverse { 8 } else { 2 }, false)
                .unwrap();
            selection
                .move_to(A, if reverse { 2 } else { 8 }, true)
                .unwrap();
            if right {
                selection.right(A, false).unwrap();
            } else {
                selection.left(A, false).unwrap();
            }
            let expected = if right { 8 } else { 2 };
            assert_eq!(state(&selection), (A, 10, expected, expected));
        }
    }
}

#[test]
fn home_end_and_direct_moves_extend_or_collapse_and_boundaries_clamp() {
    let mut selection = bound(6);
    selection.left(A, true).unwrap();
    assert_eq!(selection.range(), Some((0, 0)));
    selection.move_to(A, 3, false).unwrap();
    selection.home(A, true).unwrap();
    assert_eq!(state(&selection), (A, 6, 3, 0));
    selection.end(A, true).unwrap();
    assert_eq!(state(&selection), (A, 6, 3, 6));
    selection.right(A, true).unwrap();
    assert_eq!(state(&selection), (A, 6, 3, 6));
    selection.home(A, false).unwrap();
    assert_eq!(state(&selection), (A, 6, 0, 0));
    selection.end(A, false).unwrap();
    assert_eq!(state(&selection), (A, 6, 6, 6));
    selection.move_to(A, 2, false).unwrap();
    assert_eq!(state(&selection), (A, 6, 2, 2));
}

#[test]
fn select_all_and_empty_document_remain_canonical() {
    for len in [0, 1, MAX_BYTES] {
        let mut selection = bound(len);
        selection.select_all(A).unwrap();
        assert_eq!(state(&selection), (A, len, 0, len));
        assert_eq!(selection.range(), Some((0, len)));
        selection.select_all(A).unwrap();
        assert_eq!(selection.range(), Some((0, len)));
        selection.clear();
        assert_eq!(selection.select_all(A), Err(Error::Unbound));
    }
}

#[test]
fn wrong_source_revision_or_out_of_range_move_never_partially_updates_selection() {
    let mut selection = bound(6);
    selection.move_to(A, 3, false).unwrap();
    selection.end(A, true).unwrap();
    let before = selection.snapshot().unwrap();
    for source in [B, EDITED] {
        assert_eq!(selection.left(source, true), Err(Error::WrongSource));
        unchanged(&selection, before);
        assert_eq!(selection.right(source, false), Err(Error::WrongSource));
        unchanged(&selection, before);
        assert_eq!(selection.home(source, true), Err(Error::WrongSource));
        unchanged(&selection, before);
        assert_eq!(selection.end(source, false), Err(Error::WrongSource));
        unchanged(&selection, before);
        assert_eq!(selection.select_all(source), Err(Error::WrongSource));
        unchanged(&selection, before);
    }
    for position in [7, usize::MAX] {
        assert_eq!(
            selection.move_to(A, position, true),
            Err(Error::InvalidPosition)
        );
        unchanged(&selection, before);
        assert_eq!(
            selection.move_to(A, position, false),
            Err(Error::InvalidPosition)
        );
        unchanged(&selection, before);
    }
    assert_eq!(selection.rebind(B, MAX_BYTES + 1), Err(Error::BodyTooLarge));
    unchanged(&selection, before);
}

#[test]
fn new_document_generation_and_revision_race_invalidate_pending_paste() {
    let mut selection = bound(6);
    selection.select_all(A).unwrap();
    let pending = selection.snapshot().unwrap();
    selection.rebind(B, 6).unwrap();
    assert!(!selection.unchanged(pending));
    assert_eq!(
        selection.replacement_len(pending, 2),
        Err(Error::SnapshotChanged)
    );
    assert_eq!(
        selection.copy(A, b"abcdef", &mut [0; 6]),
        Err(Error::WrongSource)
    );
    selection.rebind(A, 6).unwrap();
    selection.select_all(A).unwrap();
    let pending = selection.snapshot().unwrap();
    selection.rebind(EDITED, 6).unwrap();
    selection.select_all(EDITED).unwrap();
    assert!(!selection.unchanged(pending));
    assert_eq!(
        selection.replacement_len(pending, 2),
        Err(Error::SnapshotChanged)
    );
}

#[test]
fn changing_length_anchor_caret_or_clearing_invalidates_exact_snapshot() {
    let mut selection = bound(6);
    selection.move_to(A, 2, false).unwrap();
    selection.move_to(A, 4, true).unwrap();
    let pending = selection.snapshot().unwrap();
    selection.move_to(A, 3, true).unwrap();
    assert!(!selection.unchanged(pending));
    selection.move_to(A, 4, false).unwrap();
    assert!(!selection.unchanged(pending));
    selection.rebind(A, 7).unwrap();
    assert!(!selection.unchanged(pending));
    selection.clear();
    assert_eq!(
        selection.replacement_len(pending, 0),
        Err(Error::SnapshotChanged)
    );
}

#[test]
fn copy_forward_and_reverse_is_exact_opaque_bytes_with_no_truncation() {
    let body = [b'a', 0, b'\n', 0xff, b'z', b'!'];
    for reverse in [false, true] {
        let mut selection = bound(body.len());
        selection
            .move_to(A, if reverse { 5 } else { 1 }, false)
            .unwrap();
        selection
            .move_to(A, if reverse { 1 } else { 5 }, true)
            .unwrap();
        let mut output = [0xa5; 8];
        assert_eq!(selection.copy(A, &body, &mut output), Ok(4));
        assert_eq!(&output[..4], &[0, b'\n', 0xff, b'z']);
        assert_eq!(&output[4..], &[0xa5; 4]);
        let mut small = [0xa5; 3];
        let pending = selection.snapshot().unwrap();
        assert_eq!(
            selection.copy(A, &body, &mut small),
            Err(Error::DestinationTooSmall)
        );
        assert_eq!(small, [0xa5; 3]);
        unchanged(&selection, pending);
    }
}

#[test]
fn copy_wrong_source_length_and_oversize_preserve_entire_destination() {
    let mut selection = bound(6);
    selection.select_all(A).unwrap();
    let pending = selection.snapshot().unwrap();
    let mut output = [0xa5; MAX_BYTES];
    for source in [B, EDITED] {
        assert_eq!(
            selection.copy(source, b"abcdef", &mut output),
            Err(Error::WrongSource)
        );
        assert!(output.iter().all(|&b| b == 0xa5));
    }
    for body in [b"abcde".as_slice(), b"abcdefg"] {
        assert_eq!(
            selection.copy(A, body, &mut output),
            Err(Error::WrongBodyLength)
        );
        assert!(output.iter().all(|&b| b == 0xa5));
    }
    assert_eq!(
        selection.copy(A, &[1; MAX_BYTES + 1], &mut output),
        Err(Error::BodyTooLarge)
    );
    assert!(output.iter().all(|&b| b == 0xa5));
    unchanged(&selection, pending);
}

#[test]
fn whole_twelve_kib_copy_is_exact_and_allocation_free_model_does_not_retain_it() {
    let mut body = [0; MAX_BYTES];
    for (index, byte) in body.iter_mut().enumerate() {
        *byte = (index % 256) as u8;
    }
    let mut selection = bound(body.len());
    selection.select_all(A).unwrap();
    let pending = selection.snapshot().unwrap();
    let mut output = [0; MAX_BYTES];
    assert_eq!(selection.copy(A, &body, &mut output), Ok(MAX_BYTES));
    assert_eq!(output, body);
    unchanged(&selection, pending);
    selection.clear();
    body.fill(0);
    assert_eq!(selection.range(), None);
    assert_eq!(output[255], 255);
}

#[test]
fn empty_copy_never_writes_but_still_checks_exact_source_and_body_length() {
    let selection = bound(0);
    let mut output = [0xa5; 3];
    assert_eq!(selection.copy(A, b"", &mut output), Ok(0));
    assert_eq!(output, [0xa5; 3]);
    assert_eq!(selection.copy(B, b"", &mut output), Err(Error::WrongSource));
    assert_eq!(
        selection.copy(A, b"x", &mut output),
        Err(Error::WrongBodyLength)
    );
    assert_eq!(output, [0xa5; 3]);
    let unbound = Selection::new();
    assert_eq!(unbound.copy(A, b"", &mut output), Err(Error::Unbound));
}

#[test]
fn full_replacement_capacity_uses_selected_length_and_checked_arithmetic() {
    for (len, selected, incoming, expected) in [
        (0, 0, 0, 0),
        (0, 0, MAX_BYTES, MAX_BYTES),
        (MAX_BYTES, MAX_BYTES, MAX_BYTES, MAX_BYTES),
        (MAX_BYTES, 1, 1, MAX_BYTES),
        (MAX_BYTES, MAX_BYTES, 0, 0),
        (100, 20, 30, 110),
    ] {
        assert_eq!(replacement_len(len, selected, incoming), Ok(expected));
    }
    assert_eq!(
        replacement_len(MAX_BYTES, 0, 1),
        Err(Error::CapacityExceeded)
    );
    assert_eq!(replacement_len(5, 6, 0), Err(Error::InvalidSelectionLength));
    assert_eq!(
        replacement_len(MAX_BYTES + 1, 0, 0),
        Err(Error::BodyTooLarge)
    );
    assert_eq!(
        replacement_len(1, 0, usize::MAX),
        Err(Error::CapacityExceeded)
    );
    assert_eq!(
        replacement_len(0, 0, usize::MAX),
        Err(Error::CapacityExceeded)
    );
}

#[test]
fn overfull_pending_paste_and_stale_selection_fail_without_mutation() {
    let mut selection = bound(MAX_BYTES);
    selection.move_to(A, 4, false).unwrap();
    selection.move_to(A, 8, true).unwrap();
    let pending = selection.snapshot().unwrap();
    assert_eq!(selection.replacement_len(pending, 4), Ok(MAX_BYTES));
    unchanged(&selection, pending);
    assert_eq!(
        selection.replacement_len(pending, 5),
        Err(Error::CapacityExceeded)
    );
    unchanged(&selection, pending);
    assert_eq!(
        selection.replacement_len(pending, usize::MAX),
        Err(Error::CapacityExceeded)
    );
    unchanged(&selection, pending);
    selection.left(A, true).unwrap();
    let changed = selection.snapshot().unwrap();
    assert_eq!(
        selection.replacement_len(pending, 0),
        Err(Error::SnapshotChanged)
    );
    unchanged(&selection, changed);
}

#[test]
fn finite_movement_sequences_preserve_every_source_and_range_invariant() {
    for len in 0..=32 {
        let mut selection = bound(len);
        for step in 0..512 {
            match step % 8 {
                0 => selection.left(A, true),
                1 => selection.right(A, true),
                2 => selection.end(A, true),
                3 => selection.home(A, true),
                4 => selection.move_to(A, (step / 8) % (len + 1), false),
                5 => selection.left(A, false),
                6 => selection.right(A, false),
                _ => selection.select_all(A),
            }
            .unwrap();
            let snapshot = selection.snapshot().unwrap();
            let (first, end) = snapshot.range();
            assert_eq!(snapshot.source(), A);
            assert_eq!(snapshot.len(), len);
            assert!(
                snapshot.anchor() <= len && snapshot.caret() <= len && first <= end && end <= len
            );
        }
    }
}
