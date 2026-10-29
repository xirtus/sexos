#![allow(dead_code)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY: u64 = u64::MAX - 1;
#[path = "../crates/sex-pdx/src/text_batch.rs"]
pub mod text_batch;
#[path = "../servers/sexdisplay/src/text_transaction.rs"]
mod transaction;
use text_batch::{hash, Request, CAPACITY};
use transaction::Stage;
fn begin(s: &mut Stage, seq: u32, len: usize) {
    assert!(s
        .receive(
            9,
            156,
            Request::Begin {
                sequence: seq,
                len,
                color: 0xffe0f0ff
            }
        )
        .is_none());
}
fn chunk(s: &mut Stage, seq: u32, offset: usize, input: &[u8]) {
    let mut bytes = [0; 8];
    bytes[..input.len()].copy_from_slice(input);
    assert!(s
        .receive(
            9,
            156,
            Request::Chunk {
                sequence: seq,
                offset,
                count: input.len(),
                bytes
            }
        )
        .is_none());
}
#[test]
fn every_partial_prefix_is_private_and_empty_or_full_commit_is_exact() {
    for len in [0, 1, 7, 8, 9, 511, 512] {
        let text = vec![b'a'; len];
        let mut s = Stage::EMPTY;
        begin(&mut s, 1, len);
        for (i, c) in text.chunks(8).enumerate() {
            chunk(&mut s, 1, i * 8, c);
        }
        let (actual, color, bytes) = s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(&text),
                },
            )
            .unwrap();
        assert_eq!(actual, len);
        assert_eq!(color, 0xffe0f0ff);
        assert_eq!(&bytes[..len], &text);
        assert!(bytes[len..].iter().all(|&v| v == 0));
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(&text)
                }
            )
            .is_none());
    }
}
#[test]
fn incomplete_bad_hash_and_contradictory_duplicate_poison_until_fresh_sequence() {
    for mode in 0..3 {
        let mut s = Stage::EMPTY;
        begin(&mut s, 1, 9);
        chunk(&mut s, 1, 0, b"abcdefgh");
        match mode {
            0 => {}
            1 => chunk(&mut s, 1, 8, b"i"),
            _ => {
                chunk(&mut s, 1, 0, b"Abcdefgh");
                chunk(&mut s, 1, 8, b"i");
            }
        }
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: if mode == 1 { 0 } else { hash(b"abcdefghi") }
                }
            )
            .is_none());
        chunk(&mut s, 1, 8, b"i");
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(b"abcdefghi")
                }
            )
            .is_none());
        begin(&mut s, 2, 1);
        chunk(&mut s, 2, 0, b"z");
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 2,
                    hash: hash(b"z")
                }
            )
            .is_some());
        begin(&mut s, 1, 0);
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(b"")
                }
            )
            .is_none());
    }
}
#[test]
fn actual_owner_surface_retirement_and_slot_rebinding_fence_partial_data() {
    let mut s = Stage::EMPTY;
    begin(&mut s, 1, 1);
    chunk(&mut s, 1, 0, b"a");
    for (owner, sid) in [(16, 156), (9, 301), (0, 156)] {
        assert!(s
            .receive(
                owner,
                sid,
                Request::Commit {
                    sequence: 1,
                    hash: hash(b"a")
                }
            )
            .is_none());
    }
    s.retire(16);
    assert!(s
        .receive(
            9,
            156,
            Request::Commit {
                sequence: 1,
                hash: hash(b"a")
            }
        )
        .is_some());
    s.retire(9);
    assert!(s
        .receive(
            9,
            156,
            Request::Commit {
                sequence: 1,
                hash: hash(b"a")
            }
        )
        .is_none());
    begin(&mut s, 2, 1);
    chunk(&mut s, 2, 0, b"a");
    s.receive(
        16,
        301,
        Request::Begin {
            sequence: 1,
            len: 1,
            color: 0,
        },
    );
    assert!(s
        .receive(
            16,
            301,
            Request::Commit {
                sequence: 1,
                hash: hash(b"a")
            }
        )
        .is_none());
}
#[test]
fn wire_rejects_reserved_bits_zero_wrapping_sequences_bounds_padding_and_controls() {
    for len in [0, 1, 512] {
        let r = Request::Begin {
            sequence: u32::MAX,
            len,
            color: 0xffe0f0ff,
        };
        let (op, a, b) = r.wire();
        assert_eq!(Request::decode(op, a, b), Some(r));
    }
    for (op, a, b) in [
        (text_batch::BEGIN, 1 << 48, 0),
        (text_batch::BEGIN, 513 << 32 | 1, 0),
        (text_batch::BEGIN, 1, 1 << 32),
        (text_batch::BEGIN, 0, 0),
        (text_batch::COMMIT, 1 << 32, 0),
        (text_batch::CHUNK, 0, 1 << 32 | 1 << 24 | 1),
        (text_batch::CHUNK, 0, 1 << 32 | 512 << 8 | 1),
        (text_batch::CHUNK, 0, 1 << 32 | 1 << 8 | 1),
        (text_batch::CHUNK, 0, 1 << 32 | 9),
        (text_batch::CHUNK, 0x6161, 1 << 32 | 1),
        (text_batch::CHUNK, 1, 1 << 32 | 1),
    ] {
        assert!(Request::decode(op, a, b).is_none(), "{op:x} {a:x} {b:x}");
    }
    let mut s = Stage::EMPTY;
    begin(&mut s, 1, CAPACITY);
    assert!(s
        .receive(
            9,
            156,
            Request::Chunk {
                sequence: 1,
                offset: 0,
                count: usize::MAX,
                bytes: [0; 8]
            }
        )
        .is_none());
}
#[test]
fn identical_duplicates_and_reverse_delivery_are_idempotent() {
    let mut s = Stage::EMPTY;
    begin(&mut s, 1, 9);
    chunk(&mut s, 1, 8, b"i");
    begin(&mut s, 1, 9);
    chunk(&mut s, 1, 0, b"abcdefgh");
    chunk(&mut s, 1, 0, b"abcdefgh");
    assert!(s
        .receive(
            9,
            156,
            Request::Commit {
                sequence: 1,
                hash: hash(b"abcdefghi")
            }
        )
        .is_some());
}
#[test]
fn completion_retained_on_legacy_full_once_per_pass_and_scrubbed_on_exit() {
    for retire in [false, true] {
        let mut s = Stage::EMPTY;
        begin(&mut s, 7, 1);
        chunk(&mut s, 7, 0, b"x");
        s.receive(
            9,
            156,
            Request::Commit {
                sequence: 7,
                hash: hash(b"x"),
            },
        )
        .unwrap();
        let mut original = None;
        for _ in 0..8 {
            let mut attempts = 0;
            s.poll_reply(|owner, value| {
                assert_eq!(owner, 9);
                attempts += 1;
                if let Some(old) = original {
                    assert_eq!(old, value)
                } else {
                    original = Some(value)
                }
                1 // Actual syscall-29 Full status, unlike enqueue Full.
            });
            assert_eq!(attempts, 1);
        }
        if retire {
            s.retire(9);
            s.poll_reply(|_, _| panic!());
        } else {
            s.poll_reply(|owner, value| {
                assert_eq!(owner, 9);
                assert_eq!(Some(value), original);
                0
            });
            s.poll_reply(|_, _| panic!());
        }
    }
    let mut s = Stage::EMPTY;
    begin(&mut s, 1, 0);
    s.receive(
        9,
        156,
        Request::Commit {
            sequence: 1,
            hash: hash(b""),
        },
    )
    .unwrap();
    s.poll_reply(|_, _| 7);
    s.poll_reply(|_, _| 1);
    s.reset();
    s.poll_reply(|_, _| panic!("authoritative cleanup must cancel retry"));
}
#[test]
fn surface_destruction_and_vacant_rebinding_scrub_partial_bytes_and_pending_receipt() {
    for complete in [false, true] {
        let mut s = Stage::EMPTY;
        begin(&mut s, 1, 1);
        chunk(&mut s, 1, 0, b"a");
        if complete {
            s.receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(b"a"),
                },
            )
            .unwrap();
        }
        s.reset(); // Actual operation used by authorized destruction/vacant binding.
        s.poll_reply(|_, _| panic!("destroyed surface receipt escaped"));
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 1,
                    hash: hash(b"a")
                }
            )
            .is_none());
        begin(&mut s, 2, 1);
        assert!(s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 2,
                    hash: hash(b"a")
                }
            )
            .is_none());
        begin(&mut s, 3, 1);
        chunk(&mut s, 3, 0, b"z");
        let (n, _, bytes) = s
            .receive(
                9,
                156,
                Request::Commit {
                    sequence: 3,
                    hash: hash(b"z"),
                },
            )
            .unwrap();
        assert_eq!(&bytes[..n], b"z");
    }
}
