use sex_pdx::fs::snapshot_stage_chunk;
use std::cell::RefCell;

#[test]
fn client_changes_shared_region_after_snapshot() {
    let shared = RefCell::new(b"original".to_vec());
    let mut private = [0u8; 16];
    let snapshot = {
        let borrow = shared.borrow();
        snapshot_stage_chunk(&borrow, &mut private).unwrap()
    };
    // Model another task changing its shared request region after Files copied
    // it, before the stage's disk write and hash. Both must use the snapshot.
    shared.borrow_mut().copy_from_slice(b"tampered");
    let written = snapshot.to_vec();
    let hashed = snapshot.to_vec();
    assert_eq!(written, b"original");
    assert_eq!(hashed, written);
    assert_eq!(&*shared.borrow(), b"tampered");
}

#[test]
fn oversized_client_chunk_does_not_change_private_buffer() {
    let mut private = [7u8; 3];
    assert!(snapshot_stage_chunk(b"larger", &mut private).is_none());
    assert_eq!(private, [7; 3]);
}
