#[path = "../servers/silk-shell/src/document_policy.rs"]
mod policy;
#[test]
fn published_new_document_beyond_seed_ids_is_covered_by_the_same_workspace() {
    for id in 7..=15 {
        for op in [0, 6] {
            assert!(policy::allows(200, op, id, id, id as u8, true, true, false));
        }
    }
}
#[test]
fn foreign_protected_missing_unpublished_or_revoked_references_are_denied() {
    for subject in [0, 9, 16, 202, u64::MAX] {
        assert!(!policy::allows(subject, 6, 7, 7, 7, true, true, false));
    }
    for op in [1, 2, 3, 4, 5, 7, 8, 9, 10, 255] {
        assert!(!policy::allows(200, op, 7, 7, 7, true, true, false));
    }
    for disk in [0, 1, 2, 16, 255] {
        assert!(!policy::allows(200, 6, 7, 7, disk, true, true, false));
    }
    for (id, published, ready, active, deny) in [
        (0, 0, true, true, false),
        (u64::MAX, u64::MAX, true, true, false),
        (7, 8, true, true, false),
        (7, 7, false, true, false),
        (7, 7, true, false, false),
        (7, 7, true, true, true),
    ] {
        assert!(!policy::allows(
            200, 6, id, published, 7, ready, active, deny
        ));
    }
}
