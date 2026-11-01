//! Browser outbox against the actual register ABI and display-owned Stage.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"] pub mod app_lifecycle;
#[path="../servers/sexdisplay/src/bitmap_binding.rs"] mod bitmap_binding;

// Explicit host-only bindings; no future Browser or Display authority is
// created. The actual Stage below continues to admit only current15/300.
mod incarnation {
    #[derive(Clone,Copy,Debug,PartialEq,Eq)]
    pub struct Identity { target:u32, task:u32, surface:u64 }
    impl Identity {
        pub const fn new(target:u32,task:u32,surface:u64)->Self { Self{target,task,surface} }
        pub const fn target(self)->u32 { self.target }
        pub const fn task(self)->u32 { self.task }
        pub const fn surface(self)->u64 { self.surface }
    }
}
const ORIGINAL:incarnation::Identity=incarnation::Identity::new(15,150,300);
const FUTURE:incarnation::Identity=incarnation::Identity::new(30,300,343);
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]
pub mod bitmap_batch;
#[path="../apps/kaleidoscope/src/bitmap_delivery.rs"]
mod production;
#[path="../servers/sexdisplay/src/bitmap_transaction.rs"]
mod transaction;
use bitmap_batch::{hash, Applied, Geometry, Request};
use production::{Admission, Metadata, Operation, Outbox, Poll, QueueError, Receipt};
use transaction::Stage;

// Explicit current held-catalog fixture. Client incarnation doubles above are
// separate: future-client tests never initialize the actual Stage for30/31.
fn bound_stage()->Stage {
    let mut stage=Stage::UNBOUND;
    let catalog=app_lifecycle::CatalogInfo {
        target_pd:15,task_id:150,surface_id:300,coordinator_pd:3,
    };
    assert!(stage.initialize_once(bitmap_binding::Binding::from_catalog(40,catalog).unwrap()));
    stage
}


fn geometry(width: u8) -> Geometry {
    Geometry {
        x: 8,
        y: 40,
        width,
        height: 1,
        scale: 2,
    }
}
fn metadata(n: u64) -> Metadata {
    Metadata {
        viewport: n,
        source: n + 10,
    }
}
fn request(op: Operation) -> Request {
    assert_eq!(op.surface, 300);
    Request::decode(op.opcode, op.packed, op.control).unwrap()
}
fn deliver(stage: &mut Stage, op: Operation) -> Option<transaction::Change> {
    stage.receive(15, 150, op.surface, request(op))
}
// Explicit simulated Full boundary, separate from packet delivery. These
// callbacks prove receipt custody only, not native rendering or pixel output.
fn publication_boundary(stage: &mut Stage) {
    if stage.needs_publication() {
        stage.poll_reply(|_, _| panic!("Applied before simulated Full publication"));
        assert!(stage.release_after_publication());
    } else {
        assert!(!stage.release_after_publication());
    }
}
fn receipt(stage: &mut Stage, out: &mut Outbox) -> Receipt {
    publication_boundary(stage);
    let mut done = None;
    stage.poll_reply(|owner, value| {
        assert_eq!(owner, 15);
        done = out.receive(1, value);
        0
    });
    done.expect("actual retained display Applied")
}
fn submit(stage: &mut Stage, out: &mut Outbox, pixels: &[u32]) -> Vec<Operation> {
    let mut ops = Vec::new();
    for _ in 0..600 {
        let before = ops.len();
        assert_eq!(
            out.poll(pixels, 4, |op| {
                ops.push(op);
                deliver(stage, op);
                Admission::Accepted
            }),
            Poll::Pending
        );
        assert!(ops.len() - before <= 4);
        if ops.len() == before {
            return ops;
        }
    }
    panic!("finite transaction did not reach retained completion");
}

#[test]
fn actual_odd_pixel_transaction_publishes_once_and_only_exact_ack_completes() {
    let pixels = [0xff123456, 0x80010203, 0x00010203];
    let mut out = Outbox::new(ORIGINAL);
    assert_eq!(out.queue(&pixels, geometry(3), metadata(1)), Ok(1));
    let mut stage = bound_stage();
    let mut visible = 0;
    let ops = submit(&mut stage, &mut out, &pixels);
    for op in &ops {
        if matches!(request(*op), Request::Commit { .. }) {
            visible += 1;
        }
    }
    assert_eq!(visible, 1);
    assert_eq!(ops.len(), 4);
    assert_eq!(
        request(ops[2]),
        Request::Chunk {
            sequence: 1,
            offset: 2,
            count: 1,
            pixels: [pixels[2], 0]
        }
    );
    assert_eq!(stage.sample(15, 300, 12, 40), Some(pixels[2]));
    let word = Applied {
        surface: 300,
        sequence: 1,
    }
    .encode()
    .unwrap();
    for (caller, value) in [
        (0, word),
        (15, word),
        (1, word | 1 << 56),
        (1, word ^ 1),
        (
            1,
            Applied {
                surface: 301,
                sequence: 1,
            }
            .encode()
            .unwrap(),
        ),
        (
            1,
            Applied {
                surface: 300,
                sequence: 2,
            }
            .encode()
            .unwrap(),
        ),
    ] {
        assert_eq!(out.receive(caller, value), None);
    }
    assert!(out.pending());
    let result = receipt(&mut stage, &mut out);
    assert_eq!(
        result,
        Receipt {
            sequence: 1,
            geometry: Some(geometry(3)),
            hash: hash(&pixels),
            metadata: metadata(1)
        }
    );
    assert_eq!(out.receive(1, word), None);
    assert!(!out.pending());
    assert_eq!(out.poll(&[], 4, |_| panic!()), Poll::Idle);
}

#[test]
fn full_at_every_operation_boundary_preserves_exact_request_without_publication_replay() {
    let pixels = [1, 2, 3, 4, 5];
    for boundary in 0..5 {
        let mut out = Outbox::new(ORIGINAL);
        let mut stage = bound_stage();
        out.queue(&pixels, geometry(5), metadata(1)).unwrap();
        let mut ops = Vec::new();
        let mut blocked = None;
        let mut changes = 0;
        for _ in 0..20 {
            let result = out.poll(&pixels, 4, |op| {
                if ops.len() == boundary && blocked.is_none() {
                    blocked = Some(op);
                    return Admission::Full;
                }
                if ops.len() == boundary {
                    assert_eq!(Some(op), blocked);
                }
                ops.push(op);
                changes += usize::from(deliver(&mut stage, op).is_some());
                Admission::Accepted
            });
            if result == Poll::Pending && ops.len() == 5 {
                break;
            }
        }
        assert_eq!(ops.len(), 5);
        assert_eq!(changes, 1);
        assert!(blocked.is_some());
        for _ in 0..4 {
            assert_eq!(
                out.poll(&[], 4, |_| panic!("accepted Commit replay")),
                Poll::Pending
            );
        }
        publication_boundary(&mut stage);
        let mut actual = Vec::new();
        for _ in 0..3 {
            stage.poll_reply(|owner, value| {
                actual.push((owner, value));
                99
            });
        }
        assert_eq!(actual.len(), 3);
        assert!(actual.iter().all(|r| *r == actual[0]));
        assert_eq!(receipt(&mut stage, &mut out).sequence, 1);
        assert_eq!(out.poll(&[], 4, |_| panic!()), Poll::Idle);
    }
}

#[test]
fn budget_is_hard_capped_and_full_uses_one_attempt() {
    let pixels = [1; 48];
    let mut out = Outbox::new(ORIGINAL);
    out.queue(&pixels, geometry(48), metadata(1)).unwrap();
    assert_eq!(out.poll(&pixels, 0, |_| panic!()), Poll::Pending);
    let mut calls = 0;
    assert_eq!(
        out.poll(&pixels, usize::MAX, |_| {
            calls += 1;
            Admission::Accepted
        }),
        Poll::Pending
    );
    assert_eq!(calls, 4);
    calls = 0;
    assert_eq!(
        out.poll(&pixels, 4, |_| {
            calls += 1;
            Admission::Full
        }),
        Poll::Deferred
    );
    assert_eq!(calls, 1);
    assert!(core::mem::size_of::<Outbox>() < 512);
}

#[test]
fn clear_abandons_every_unsent_image_prefix_and_never_reads_scrubbed_pixels() {
    let pixels = [1, 2, 3, 4, 5];
    for accepted_prefix in 0..5 {
        let mut out = Outbox::new(ORIGINAL);
        let mut stage = bound_stage();
        out.queue(&pixels, geometry(5), metadata(1)).unwrap();
        for _ in 0..accepted_prefix {
            out.poll(&pixels, 1, |op| {
                deliver(&mut stage, op);
                Admission::Accepted
            });
        }
        out.poll(&pixels, 1, |_| Admission::Full);
        assert_eq!(out.clear(metadata(2)), Ok(2));
        assert_eq!(
            out.poll(&[], 4, |op| {
                assert_eq!(request(op), Request::Clear { sequence: 2 });
                deliver(&mut stage, op);
                Admission::Accepted
            }),
            Poll::Pending
        );
        assert_eq!(
            out.poll(&[], 4, |_| panic!("accepted Clear replay")),
            Poll::Pending
        );
        assert_eq!(
            receipt(&mut stage, &mut out),
            Receipt {
                sequence: 2,
                geometry: None,
                hash: 0,
                metadata: metadata(2)
            }
        );
        assert!(stage.scrubbed());
        assert!(!out.pending());
    }
}

#[test]
fn clear_after_accepted_commit_keeps_old_correlation_and_then_erases() {
    let pixels = [0xff112233];
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    out.queue(&pixels, geometry(1), metadata(1)).unwrap();
    submit(&mut stage, &mut out, &pixels);
    assert_eq!(out.clear(metadata(2)), Ok(2));
    assert_eq!(
        out.queue(&pixels, geometry(1), metadata(3)),
        Err(QueueError::ClearPending)
    );
    assert_eq!(
        out.poll(&[], 4, |_| panic!("cannot overtake old Applied")),
        Poll::Pending
    );
    let old = receipt(&mut stage, &mut out);
    assert_eq!(old.metadata, metadata(1));
    assert_eq!(old.geometry, Some(geometry(1)));
    assert_eq!(old.hash, hash(&pixels));
    assert!(out.pending());
    assert_eq!(stage.geometry(), Some(geometry(1)));
    out.poll(&[], 4, |op| {
        assert_eq!(request(op), Request::Clear { sequence: 2 });
        deliver(&mut stage, op);
        Admission::Accepted
    });
    assert_eq!(stage.geometry(), None);
    let clear = receipt(&mut stage, &mut out);
    assert_eq!(clear.metadata, metadata(2));
    assert_eq!(clear.geometry, None);
    assert!(stage.scrubbed());
    assert!(!out.pending());
}

#[test]
fn reflow_while_waiting_coalesces_only_desired_and_old_ack_is_historical() {
    let pixels = [1, 2];
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    let old = geometry(2);
    out.queue(&pixels, old, metadata(1)).unwrap();
    submit(&mut stage, &mut out, &pixels);
    let middle = Geometry { x: 20, ..old };
    let latest = Geometry {
        x: 30,
        scale: 3,
        ..old
    };
    assert_eq!(out.queue(&pixels, middle, metadata(2)), Ok(2));
    assert_eq!(out.queue(&pixels, latest, metadata(3)), Ok(3));
    assert_eq!(out.queue(&pixels, latest, metadata(3)), Ok(3));
    assert_eq!(out.poll(&[], 4, |_| panic!()), Poll::Pending);
    let prior = receipt(&mut stage, &mut out);
    assert_eq!((prior.geometry, prior.metadata), (Some(old), metadata(1)));
    assert_eq!(stage.geometry(), Some(old));
    let operations = submit(&mut stage, &mut out, &pixels);
    assert!(operations.iter().all(|op| match request(*op) {
        Request::Begin { sequence, .. }
        | Request::Chunk { sequence, .. }
        | Request::Commit { sequence, .. }
        | Request::Clear { sequence } => sequence == 3,
    }));
    let current = receipt(&mut stage, &mut out);
    assert_eq!(
        (current.geometry, current.metadata),
        (Some(latest), metadata(3))
    );
}

#[test]
fn new_queue_after_old_ack_replaces_unstarted_desired_without_stale_replay() {
    let pixels = [1];
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    out.queue(&pixels, geometry(1), metadata(1)).unwrap();
    submit(&mut stage, &mut out, &pixels);
    out.queue(
        &pixels,
        Geometry {
            x: 20,
            ..geometry(1)
        },
        metadata(2),
    )
    .unwrap();
    receipt(&mut stage, &mut out);
    let latest = Geometry {
        x: 30,
        ..geometry(1)
    };
    assert_eq!(out.queue(&pixels, latest, metadata(3)), Ok(3));
    submit(&mut stage, &mut out, &pixels);
    assert_eq!(receipt(&mut stage, &mut out).sequence, 3);
    assert_eq!(stage.geometry(), Some(latest));
    assert!(!out.pending());
    assert_eq!(
        out.poll(&[], 4, |_| panic!("must not replay superseded sequence2")),
        Poll::Idle
    );
}

#[test]
fn clear_overrides_queued_reflow_before_or_after_old_commit_ack() {
    for ack_first in [false, true] {
        let pixels = [1];
        let mut out = Outbox::new(ORIGINAL);
        let mut stage = bound_stage();
        out.queue(&pixels, geometry(1), metadata(1)).unwrap();
        submit(&mut stage, &mut out, &pixels);
        out.queue(
            &pixels,
            Geometry {
                x: 20,
                ..geometry(1)
            },
            metadata(2),
        )
        .unwrap();
        if ack_first {
            receipt(&mut stage, &mut out);
        }
        assert_eq!(out.clear(metadata(3)), Ok(3));
        if !ack_first {
            receipt(&mut stage, &mut out);
        }
        let mut sent = 0;
        out.poll(&[], 4, |op| {
            sent += 1;
            assert_eq!(request(op), Request::Clear { sequence: 3 });
            deliver(&mut stage, op);
            Admission::Accepted
        });
        assert_eq!(sent, 1);
        assert_eq!(receipt(&mut stage, &mut out).sequence, 3);
    }
}

#[test]
fn reset_erases_display_pixels_but_does_not_reinterpret_retained_ack() {
    let pixels = [0xffabcdef];
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    out.queue(&pixels, geometry(1), metadata(1)).unwrap();
    submit(&mut stage, &mut out, &pixels);
    stage.reset_surface(15, 300);
    assert!(stage.pixels_scrubbed());
    assert_eq!(stage.geometry(), None);
    out.clear(metadata(2)).unwrap();
    assert_eq!(out.poll(&[], 4, |_| panic!()), Poll::Pending);
    let historical = receipt(&mut stage, &mut out);
    assert_eq!(historical.geometry, Some(geometry(1)));
    assert_eq!(historical.metadata, metadata(1));
    assert_eq!(stage.geometry(), None);
    assert!(stage.scrubbed());
    submit(&mut stage, &mut out, &[]);
    assert_eq!(receipt(&mut stage, &mut out).geometry, None);
    out.queue(&pixels, geometry(1), metadata(3)).unwrap();
    submit(&mut stage, &mut out, &pixels);
    assert_eq!(receipt(&mut stage, &mut out).sequence, 3);
    assert_eq!(stage.geometry(), Some(geometry(1)));
}

#[test]
fn known_send_rejection_never_completes_and_new_higher_begin_recovers() {
    let pixels = [1, 2, 3];
    for boundary in 0..4 {
        let mut out = Outbox::new(ORIGINAL);
        let mut stage = bound_stage();
        out.queue(&pixels, geometry(3), metadata(1)).unwrap();
        for _ in 0..boundary {
            out.poll(&pixels, 1, |op| {
                deliver(&mut stage, op);
                Admission::Accepted
            });
        }
        assert_eq!(
            out.poll(&pixels, 1, |_| Admission::Rejected(99)),
            Poll::Rejected(99)
        );
        assert!(!out.pending());
        stage.poll_reply(|_, _| panic!("rejection is not Applied"));
        assert_eq!(
            out.receive(
                1,
                Applied {
                    surface: 300,
                    sequence: 1
                }
                .encode()
                .unwrap()
            ),
            None
        );
        assert_eq!(out.queue(&pixels, geometry(3), metadata(2)), Ok(2));
        submit(&mut stage, &mut out, &pixels);
        assert_eq!(receipt(&mut stage, &mut out).sequence, 2);
    }
}

#[test]
fn rejected_active_preserves_latest_reflow_for_explicit_recovery() {
    let pixels = [1];
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    out.queue(&pixels, geometry(1), metadata(1)).unwrap();
    out.queue(
        &pixels,
        Geometry {
            x: 30,
            ..geometry(1)
        },
        metadata(2),
    )
    .unwrap();
    assert_eq!(
        out.poll(&pixels, 1, |_| Admission::Rejected(7)),
        Poll::Rejected(7)
    );
    assert!(out.pending());
    submit(&mut stage, &mut out, &pixels);
    assert_eq!(receipt(&mut stage, &mut out).metadata, metadata(2));
}

#[test]
fn malformed_pixels_are_never_sent_and_do_not_destroy_valid_queued_batch() {
    let pixels = [1, 2];
    let mut out = Outbox::new(ORIGINAL);
    assert_eq!(
        out.queue(&[], geometry(2), metadata(1)),
        Err(QueueError::InvalidPixels)
    );
    assert_eq!(
        out.queue(
            &pixels,
            Geometry {
                width: 0,
                ..geometry(2)
            },
            metadata(1)
        ),
        Err(QueueError::InvalidGeometry)
    );
    assert_eq!(out.sequence(), 0);
    assert!(!out.pending());
    out.queue(&pixels, geometry(2), metadata(1)).unwrap();
    assert_eq!(
        out.queue(&[1], geometry(2), metadata(2)),
        Err(QueueError::InvalidPixels)
    );
    let mut stage = bound_stage();
    submit(&mut stage, &mut out, &pixels);
    assert_eq!(receipt(&mut stage, &mut out).metadata, metadata(1));
}

#[test]
fn changed_pixels_after_full_are_rejected_before_retry_and_can_be_cleared() {
    let pixels = [1, 2, 3];
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    out.queue(&pixels, geometry(3), metadata(1)).unwrap();
    out.poll(&pixels, 1, |op| {
        deliver(&mut stage, op);
        Admission::Accepted
    });
    assert_eq!(out.poll(&pixels, 4, |_| Admission::Full), Poll::Deferred);
    assert_eq!(
        out.poll(&[1, 2, 9], 4, |_| panic!("changed chunk must not send")),
        Poll::InvalidPixels
    );
    assert!(!out.pending());
    assert_eq!(stage.geometry(), None);
    out.clear(metadata(2)).unwrap();
    submit(&mut stage, &mut out, &[]);
    assert_eq!(receipt(&mut stage, &mut out).geometry, None);
    assert!(stage.scrubbed());
}

#[test]
fn completed_chunks_allow_drop_before_commit_or_receipt() {
    let pixels = [1];
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    out.queue(&pixels, geometry(1), metadata(1)).unwrap();
    out.poll(&pixels, 2, |op| {
        deliver(&mut stage, op);
        Admission::Accepted
    });
    assert_eq!(
        out.poll(&[], 1, |op| {
            assert!(matches!(request(op), Request::Commit { .. }));
            deliver(&mut stage, op);
            Admission::Accepted
        }),
        Poll::Pending
    );
    assert_eq!(receipt(&mut stage, &mut out).hash, hash(&pixels));
}

#[test]
fn close_fences_partial_full_waiting_and_clear_without_false_completion() {
    let pixels = [1];
    for prefix in 0..=4 {
        let mut out = Outbox::new(ORIGINAL);
        let mut stage = bound_stage();
        out.queue(&pixels, geometry(1), metadata(1)).unwrap();
        for _ in 0..prefix.min(3) {
            out.poll(&pixels, 1, |op| {
                deliver(&mut stage, op);
                Admission::Accepted
            });
        }
        if prefix == 4 {
            out.clear(metadata(2)).unwrap();
        }
        out.close();
        assert!(!out.pending());
        assert_eq!(out.sequence(), if prefix == 4 { 2 } else { 1 });
        assert_eq!(out.poll(&[], 4, |_| panic!()), Poll::Closed);
        assert_eq!(
            out.queue(&pixels, geometry(1), metadata(3)),
            Err(QueueError::Closed)
        );
        assert_eq!(out.clear(metadata(3)), Err(QueueError::Closed));
        publication_boundary(&mut stage);
        stage.poll_reply(|_, value| {
            assert_eq!(out.receive(1, value), None);
            0
        });
        // Local close does not forge display erasure; lifecycle retirement does.
        stage.retire(15,150);
        assert!(stage.scrubbed());
    }
}

#[test]
fn sequences_exhaust_without_wrap_or_replaying_last_accepted_mutation() {
    let pixels = [1];
    let mut out = Outbox::with_sequence_floor(ORIGINAL, u32::MAX - 1);
    let mut stage = bound_stage();
    assert_eq!(out.queue(&pixels, geometry(1), metadata(1)), Ok(u32::MAX));
    submit(&mut stage, &mut out, &pixels);
    assert_eq!(out.clear(metadata(2)), Err(QueueError::SequenceExhausted));
    assert_eq!(
        out.poll(&[], 4, |_| panic!("exhaustion cannot replay Commit")),
        Poll::Pending
    );
    assert_eq!(receipt(&mut stage, &mut out).sequence, u32::MAX);
    assert_eq!(
        out.queue(&pixels, geometry(1), metadata(2)),
        Err(QueueError::SequenceExhausted)
    );
    assert_eq!(out.clear(metadata(2)), Err(QueueError::SequenceExhausted));
    assert_eq!(out.sequence(), u32::MAX);
    assert!(!out.pending());
}

#[test]
fn sequence_exhaustion_preserves_reserved_clear_but_stops_unsent_image() {
    let pixels = [1];
    let mut out = Outbox::with_sequence_floor(ORIGINAL, u32::MAX - 2);
    let mut stage = bound_stage();
    out.queue(&pixels, geometry(1), metadata(1)).unwrap();
    submit(&mut stage, &mut out, &pixels);
    assert_eq!(out.clear(metadata(2)), Ok(u32::MAX));
    assert_eq!(out.clear(metadata(3)), Err(QueueError::SequenceExhausted));
    assert_eq!(receipt(&mut stage, &mut out).sequence, u32::MAX - 1);
    submit(&mut stage, &mut out, &[]);
    assert_eq!(
        receipt(&mut stage, &mut out),
        Receipt {
            sequence: u32::MAX,
            geometry: None,
            hash: 0,
            metadata: metadata(2)
        }
    );
    assert!(stage.scrubbed());
    let mut out = Outbox::with_sequence_floor(ORIGINAL, u32::MAX - 1);
    out.queue(&pixels, geometry(1), metadata(1)).unwrap();
    out.poll(&pixels, 1, |_| Admission::Accepted);
    out.poll(&pixels, 1, |_| Admission::Full);
    assert_eq!(out.clear(metadata(2)), Err(QueueError::SequenceExhausted));
    assert!(!out.pending());
    assert_eq!(
        out.poll(&[], 4, |_| panic!("exhaustion cannot send unsent image")),
        Poll::Idle
    );
}

#[test]
fn repeated_same_clear_is_coalesced_and_full_clear_is_retained_exactly() {
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    assert_eq!(out.clear(metadata(1)), Ok(1));
    assert_eq!(out.clear(metadata(1)), Ok(1));
    let mut held = None;
    assert_eq!(
        out.poll(&[], 4, |op| {
            held = Some(op);
            Admission::Full
        }),
        Poll::Deferred
    );
    assert_eq!(out.clear(metadata(1)), Ok(1));
    out.poll(&[], 4, |op| {
        assert_eq!(Some(op), held);
        deliver(&mut stage, op);
        Admission::Accepted
    });
    assert_eq!(out.clear(metadata(1)), Ok(1));
    assert_eq!(receipt(&mut stage, &mut out).geometry, None);
    assert_eq!(out.sequence(), 1);
}

#[test]
fn early_duplicate_old_and_future_receipts_cannot_complete_active_work() {
    let pixels = [1];
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    out.queue(&pixels, geometry(1), metadata(1)).unwrap();
    let word = |sequence| {
        Applied {
            surface: 300,
            sequence,
        }
        .encode()
        .unwrap()
    };
    for prefix in 0..3 {
        assert_eq!(out.receive(1, word(1)), None);
        if prefix < 2 {
            out.poll(&pixels, 1, |op| {
                deliver(&mut stage, op);
                Admission::Accepted
            });
        }
    }
    submit(&mut stage, &mut out, &pixels);
    receipt(&mut stage, &mut out);
    out.queue(&pixels, geometry(1), metadata(2)).unwrap();
    submit(&mut stage, &mut out, &pixels);
    assert_eq!(out.receive(1, word(1)), None);
    assert_eq!(out.receive(1, word(3)), None);
    assert_eq!(receipt(&mut stage, &mut out).sequence, 2);
}

#[test]
fn clear_while_clear_ack_is_held_keeps_both_original_correlations() {
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    out.clear(metadata(1)).unwrap();
    submit(&mut stage, &mut out, &[]);
    assert_eq!(out.clear(metadata(2)), Ok(2));
    assert_eq!(
        out.poll(&[], 4, |_| panic!("must wait originalClearACK")),
        Poll::Pending
    );
    assert_eq!(
        receipt(&mut stage, &mut out),
        Receipt {
            sequence: 1,
            geometry: None,
            hash: 0,
            metadata: metadata(1)
        }
    );
    submit(&mut stage, &mut out, &[]);
    assert_eq!(
        receipt(&mut stage, &mut out),
        Receipt {
            sequence: 2,
            geometry: None,
            hash: 0,
            metadata: metadata(2)
        }
    );
    assert!(!out.pending());
}

#[test]
fn maximum_asset_stays_finite_without_internal_image_copy() {
    let pixels: Vec<_> = (0..bitmap_batch::CAPACITY)
        .map(|i| 0xff000000 | i as u32)
        .collect();
    let g = Geometry {
        width: 48,
        height: 48,
        scale: 4,
        ..geometry(1)
    };
    let mut out = Outbox::new(ORIGINAL);
    let mut stage = bound_stage();
    out.queue(&pixels, g, metadata(1)).unwrap();
    let ops = submit(&mut stage, &mut out, &pixels);
    assert_eq!(ops.len(), 1154);
    assert!(core::mem::size_of::<Outbox>() < 512);
    assert_eq!(receipt(&mut stage, &mut out).hash, hash(&pixels));
    assert_eq!(
        stage.sample(15, 300, g.x as i64 + 47 * 4, g.y as i64 + 47 * 4),
        Some(pixels[2303])
    );
}

#[test]
fn viewport_reset_clear_barrier_restarts_partial_and_accepted_transactions() {
    // BEGIN only, partial CHUNK, Full unsent COMMIT, accepted COMMIT awaiting
    // Applied, and accepted CLEAR awaiting Applied all exercise the real Stage.
    for mode in 0..5 {
        let pixels = [0xff123456, 0x80123456, 0x00123456];
        let mut out = Outbox::new(ORIGINAL);
        let mut stage = bound_stage();
        out.queue(&pixels, geometry(3), metadata(1)).unwrap();
        submit(&mut stage, &mut out, &pixels);
        receipt(&mut stage, &mut out);
        let in_flight_geometry = Geometry {
            x: 20,
            ..geometry(3)
        };
        if mode == 4 {
            assert_eq!(out.clear(metadata(2)), Ok(2));
            submit(&mut stage, &mut out, &[]);
        } else {
            assert_eq!(out.queue(&pixels, in_flight_geometry, metadata(2)), Ok(2));
            if mode == 3 {
                submit(&mut stage, &mut out, &pixels);
            } else {
                for _ in 0..=mode {
                    assert_eq!(
                        out.poll(&pixels, 1, |op| {
                            assert!(!matches!(request(op), Request::Commit { .. }));
                            deliver(&mut stage, op);
                            Admission::Accepted
                        }),
                        Poll::Pending
                    );
                }
                if mode == 2 {
                    assert_eq!(
                        out.poll(&pixels, 1, |op| {
                            assert_eq!(
                                request(op),
                                Request::Commit {
                                    sequence: 2,
                                    hash: hash(&pixels)
                                }
                            );
                            Admission::Full
                        }),
                        Poll::Deferred
                    );
                }
            }
        }
        publication_boundary(&mut stage);
        let mut historical_word = None;
        stage.poll_reply(|owner, value| {
            assert_eq!(owner, 15);
            historical_word = Some(value);
            99 // Actual display retains the original publication receipt.
        });
        assert_eq!(historical_word.is_some(), mode >= 3);
        stage.reset_surface(15, 300);
        assert!(stage.pixels_scrubbed());
        assert_eq!(stage.geometry(), None);
        let reset_meta = Metadata {
            viewport: 77,
            source: 0,
        };
        assert_eq!(out.clear(reset_meta), Ok(3));
        assert_eq!(
            out.queue(&pixels, geometry(3), metadata(3)),
            Err(QueueError::ClearPending)
        );
        if let Some(word) = historical_word {
            assert_eq!(
                out.poll(&[], 4, |_| panic!(
                    "reset must retain accepted historical ACK"
                )),
                Poll::Pending
            );
            stage.poll_reply(|owner, value| {
                assert_eq!((owner, value), (15, word));
                99
            });
            let old = receipt(&mut stage, &mut out);
            assert_eq!(old.sequence, 2);
            assert_eq!(old.metadata, metadata(2));
            assert_eq!(
                old.geometry,
                if mode == 3 {
                    Some(in_flight_geometry)
                } else {
                    None
                }
            );
            assert_eq!(stage.geometry(), None); // Historical ACK is not paint.
        } else {
            stage.poll_reply(|_, _| panic!("uncommitted prefix has no historical ACK"));
            assert_eq!(
                out.receive(
                    1,
                    Applied {
                        surface: 300,
                        sequence: 2
                    }
                    .encode()
                    .unwrap()
                ),
                None
            );
        }
        let mut held_clear = None;
        assert_eq!(
            out.poll(&[], 4, |op| {
                assert_eq!(request(op), Request::Clear { sequence: 3 });
                held_clear = Some(op);
                Admission::Full
            }),
            Poll::Deferred
        );
        assert_eq!(
            out.poll(&[], 4, |op| {
                assert_eq!(Some(op), held_clear);
                let change = deliver(&mut stage, op).unwrap();
                assert_eq!((change.old, change.new), (None, None));
                Admission::Accepted
            }),
            Poll::Pending
        );
        assert_eq!(
            out.queue(&pixels, geometry(3), metadata(3)),
            Err(QueueError::ClearPending)
        );
        assert_eq!(
            receipt(&mut stage, &mut out),
            Receipt {
                sequence: 3,
                geometry: None,
                hash: 0,
                metadata: reset_meta
            }
        );
        assert!(!out.pending());
        assert!(stage.scrubbed());
        let fresh_geometry = Geometry {
            x: 30,
            scale: 3,
            ..geometry(3)
        };
        let fresh_meta = Metadata {
            viewport: 77,
            source: 42,
        };
        assert_eq!(out.queue(&pixels, fresh_geometry, fresh_meta), Ok(4));
        let sent = submit(&mut stage, &mut out, &pixels);
        assert_eq!(
            request(sent[0]),
            Request::Begin {
                sequence: 4,
                geometry: fresh_geometry
            }
        );
        assert_eq!(
            request(*sent.last().unwrap()),
            Request::Commit {
                sequence: 4,
                hash: hash(&pixels)
            }
        );
        assert_eq!(
            receipt(&mut stage, &mut out),
            Receipt {
                sequence: 4,
                geometry: Some(fresh_geometry),
                hash: hash(&pixels),
                metadata: fresh_meta
            }
        );
        assert_eq!(stage.geometry(), Some(fresh_geometry));
        for (index, &pixel) in pixels.iter().enumerate() {
            assert_eq!(
                stage.sample(15, 300, 30 + index as i64 * 3, 40),
                Some(pixel)
            );
        }
        assert!(!out.pending());
    }
}

// Future cases exercise the actual outbox/register ABI only. Display's real
// Stage remains fixed15/300 above; these tests do not claim future admission.
fn bound_word(identity:incarnation::Identity, sequence:u32)->u64 {
    Applied{surface:u16::try_from(identity.surface()).unwrap(),sequence}.encode().unwrap()
}
fn bound_operations(out:&mut Outbox,pixels:&[u32],identity:incarnation::Identity)->Vec<Operation> {
    let mut operations=Vec::new();
    for _ in 0..16 {
        let before=operations.len();
        assert_eq!(out.poll(pixels,4,|op|{
            assert_eq!(op.surface,identity.surface());
            assert!(Request::decode(op.opcode,op.packed,op.control).is_some());
            operations.push(op);Admission::Accepted
        }),Poll::Pending);
        if before==operations.len(){return operations;}
    }
    panic!("small finite bound transaction did not reach Awaiting");
}

#[test]
fn immutable_constructor_binds_begin_chunks_commit_clear_and_exact_applied() {
    let pixels=[0xff123456,0x80010203,0x00010203];let before=pixels;
    for (identity,foreign) in [(ORIGINAL,FUTURE),(FUTURE,ORIGINAL)] {
        let mut out=Outbox::new(identity);
        // Neither a first foreign receipt nor another client's construction
        // can choose this outbox's destination.
        assert_eq!(out.receive(1,bound_word(foreign,1)),None);
        assert_eq!(out.queue(&pixels,geometry(3),metadata(1)),Ok(1));
        let operations=bound_operations(&mut out,&pixels,identity);
        let requests:Vec<_>=operations.iter().map(|op|Request::decode(op.opcode,op.packed,op.control).unwrap()).collect();
        assert_eq!(requests,vec![Request::Begin{sequence:1,geometry:geometry(3)},
            Request::Chunk{sequence:1,offset:0,count:2,pixels:[pixels[0],pixels[1]]},
            Request::Chunk{sequence:1,offset:2,count:1,pixels:[pixels[2],0]},
            Request::Commit{sequence:1,hash:hash(&pixels)}]);
        assert_eq!(out.receive(1,bound_word(foreign,1)),None);
        for caller in [0,identity.target(),foreign.target()] {assert_eq!(out.receive(caller,bound_word(identity,1)),None);}
        assert_eq!(out.receive(1,bound_word(identity,1)|(1<<56)),None);
        assert_eq!(out.receive(1,bound_word(identity,2)),None);
        assert_eq!(out.receive(1,bound_word(identity,1)),Some(Receipt{
            sequence:1,geometry:Some(geometry(3)),hash:hash(&pixels),metadata:metadata(1)}));
        assert_eq!(out.receive(1,bound_word(identity,1)),None);
        assert_eq!(out.clear(metadata(2)),Ok(2));
        let clear=bound_operations(&mut out,&[],identity);assert_eq!(clear.len(),1);
        assert_eq!(Request::decode(clear[0].opcode,clear[0].packed,clear[0].control),Some(Request::Clear{sequence:2}));
        assert_eq!(out.receive(1,bound_word(foreign,2)),None);
        assert_eq!(out.receive(1,bound_word(identity,2)),Some(Receipt{sequence:2,geometry:None,hash:0,metadata:metadata(2)}));
        assert!(!out.pending());assert_eq!(pixels,before);
    }
}

#[test]
fn full_at_every_bound_operation_retains_identical_request_across_peer_activity() {
    let pixels=[1,2,3];
    for (identity,foreign) in [(ORIGINAL,FUTURE),(FUTURE,ORIGINAL)] {
        let mut out=Outbox::new(identity);out.queue(&pixels,geometry(3),metadata(1)).unwrap();
        for index in 0..4 {
            let mut held=None;
            for _ in 0..2 {
                assert_eq!(out.poll(&pixels,4,|op|{
                    assert_eq!(op.surface,identity.surface());
                    if let Some(previous)=held{assert_eq!(op,previous)}else{held=Some(op)}
                    Admission::Full
                }),Poll::Deferred);
                assert_eq!(out.receive(1,bound_word(foreign,1)),None);
                let mut peer=Outbox::new(foreign);peer.clear(metadata(90)).unwrap();
                let peer_ops=bound_operations(&mut peer,&[],foreign);assert_eq!(peer_ops.len(),1);
                assert!(peer.receive(1,bound_word(foreign,1)).is_some());
            }
            assert_eq!(out.poll(&pixels,1,|op|{
                assert_eq!(Some(op),held);assert_eq!(op.surface,identity.surface());
                let request=Request::decode(op.opcode,op.packed,op.control).unwrap();
                assert!(match index{0=>matches!(request,Request::Begin{..}),1|2=>matches!(request,Request::Chunk{..}),_=>matches!(request,Request::Commit{..})});
                Admission::Accepted
            }),Poll::Pending);
        }
        assert!(out.receive(1,bound_word(identity,1)).is_some());
        out.clear(metadata(2)).unwrap();let mut held=None;
        for _ in 0..2 {assert_eq!(out.poll(&[],4,|op|{
            assert_eq!(op.surface,identity.surface());if let Some(previous)=held{assert_eq!(op,previous)}else{held=Some(op)}
            Admission::Full
        }),Poll::Deferred);assert_eq!(out.receive(1,bound_word(foreign,2)),None);}
        out.poll(&[],4,|op|{assert_eq!(Some(op),held);assert_eq!(op.surface,identity.surface());Admission::Accepted});
        assert_eq!(out.receive(1,bound_word(identity,2)).unwrap().geometry,None);
    }
}

#[test]
fn bound_historical_commit_and_clear_keep_original_metadata_through_desired_replacement() {
    for (identity,foreign) in [(ORIGINAL,FUTURE),(FUTURE,ORIGINAL)] {
        let pixels=[1];let mut out=Outbox::new(identity);
        out.queue(&pixels,geometry(1),metadata(1)).unwrap();bound_operations(&mut out,&pixels,identity);
        let reflow=Geometry{x:20,..geometry(1)};
        assert_eq!(out.queue(&pixels,reflow,metadata(2)),Ok(2));
        assert_eq!(out.clear(metadata(3)),Ok(3));
        assert_eq!(out.poll(&[],4,|_|panic!("accepted historical Commit must finish before bound Clear")),Poll::Pending);
        assert_eq!(out.receive(1,bound_word(foreign,1)),None);
        assert_eq!(out.receive(1,bound_word(identity,1)),Some(Receipt{
            sequence:1,geometry:Some(geometry(1)),hash:hash(&pixels),metadata:metadata(1)}));
        let clear=bound_operations(&mut out,&[],identity);
        assert_eq!(clear.len(),1);assert_eq!(Request::decode(clear[0].opcode,clear[0].packed,clear[0].control),Some(Request::Clear{sequence:3}));
        assert_eq!(out.clear(metadata(4)),Ok(4));
        assert_eq!(out.poll(&[],4,|_|panic!("accepted historical Clear must retain its original receipt")),Poll::Pending);
        assert_eq!(out.receive(1,bound_word(foreign,3)),None);
        assert_eq!(out.receive(1,bound_word(identity,3)),Some(Receipt{sequence:3,geometry:None,hash:0,metadata:metadata(3)}));
        let next=bound_operations(&mut out,&[],identity);assert_eq!(next.len(),1);
        assert_eq!(out.receive(1,bound_word(identity,3)),None);
        assert_eq!(out.receive(1,bound_word(identity,4)),Some(Receipt{sequence:4,geometry:None,hash:0,metadata:metadata(4)}));
        assert_eq!(out.sequence(),4);assert!(!out.pending());
    }
}

#[test]
fn constructor_sequence_floor_and_terminal_close_cannot_be_reset_by_receipts() {
    for (identity,foreign) in [(ORIGINAL,FUTURE),(FUTURE,ORIGINAL)] {
        let mut out=Outbox::with_sequence_floor(identity,40);
        for word in [bound_word(identity,1),bound_word(identity,40),bound_word(foreign,41)] {
            assert_eq!(out.receive(1,word),None);assert_eq!(out.sequence(),40);
        }
        assert_eq!(out.queue(&[1],geometry(1),metadata(1)),Ok(41));bound_operations(&mut out,&[1],identity);
        assert_eq!(out.receive(1,bound_word(identity,40)),None);assert_eq!(out.receive(1,bound_word(foreign,41)),None);
        out.close();out.close();assert_eq!(out.sequence(),41);assert!(!out.pending());
        for word in [bound_word(identity,41),bound_word(identity,42),bound_word(foreign,41)] {assert_eq!(out.receive(1,word),None);}
        assert_eq!(out.queue(&[1],geometry(1),metadata(2)),Err(QueueError::Closed));
        assert_eq!(out.clear(metadata(2)),Err(QueueError::Closed));
        assert_eq!(out.poll(&[],4,|_|panic!("closed outbox cannot revive or rebind")),Poll::Closed);
    }
}
