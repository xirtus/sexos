//! Exercise actual output custody without FFmpeg or guest allocator startup.
extern crate alloc;
#[path = "../crates/sex-media/src/recording/output.rs"]
mod output;
use output::{Failure, Output, MAX_FILE_BYTES, TURN_BYTES};

#[test]
fn container_patches_stay_ordered_and_reconstruct_exact_bytes() {
    let mut out = Output::new(128);
    out.write(b"HEADER____").unwrap();
    out.seek(16, 0).unwrap();
    out.write(b"frame").unwrap();
    out.seek(6, 0).unwrap();
    out.write(b"size").unwrap();
    assert_eq!(out.len(), 21);
    let mut file = vec![0; 21];
    let mut offsets = Vec::new();
    while let Some(w) = out.pending() {
        let (token, offset, n) = (w.token, w.offset as usize, w.bytes.len());
        file[offset..offset + n].copy_from_slice(w.bytes);
        offsets.push(offset);
        out.consume(token, n).unwrap();
    }
    assert_eq!(offsets, [0, 16, 6]);
    assert_eq!(&file[..10], b"HEADERsize");
    assert_eq!(&file[10..16], &[0; 6]);
    assert_eq!(&file[16..], b"frame");
    assert_eq!(out.pending_bytes(), 0);
}
#[test]
fn full_and_partial_acknowledgements_fence_replay_and_bound_each_turn() {
    let mut out = Output::new(TURN_BYTES * 3);
    out.write(&vec![7; TURN_BYTES * 2 + 3]).unwrap();
    let first = out.pending().unwrap().token;
    assert_eq!(out.pending().unwrap().bytes.len(), TURN_BYTES);
    assert_eq!(out.consume(first, TURN_BYTES + 1), Err(Failure::Invalid));
    out.consume(first, 17).unwrap();
    assert_eq!(out.consume(first, 17), Err(Failure::Invalid));
    assert_eq!(out.pending().unwrap().offset, 17);
    while let Some(w) = out.pending() {
        let (token, n) = (w.token, w.bytes.len());
        assert!(n <= TURN_BYTES);
        out.consume(token, n).unwrap();
    }
    assert!(out.empty());
    assert_eq!(out.consume(first, 1), Err(Failure::Invalid));
}
#[test]
fn limits_fail_before_mutating_existing_output_or_cursor() {
    let mut out = Output::new(8);
    out.write(b"1234").unwrap();
    assert_eq!(out.write(b"56789"), Err(Failure::Limit));
    assert_eq!(out.seek(0, 1), Ok(4));
    assert_eq!(out.len(), 4);
    assert_eq!(out.pending_bytes(), 4);
    assert_eq!(out.seek(-5, 1), Err(Failure::Invalid));
    assert_eq!(out.seek(0, 1), Ok(4));
    assert_eq!(out.seek(MAX_FILE_BYTES as i64, 0), Ok(MAX_FILE_BYTES));
    assert_eq!(out.write(b"x"), Err(Failure::Limit));
    assert_eq!(out.seek(0, 0x10000), Ok(4));
    assert_eq!(out.len(), 4);
    assert_eq!(out.pending().unwrap().bytes, b"1234");
}
