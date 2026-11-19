// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
// Host tests for SexDrive's NVMe queue bookkeeping: many commands in flight, the
// ring wrapping, completions in any order, a full SQ holding submissions back and
// one failed command in a batch. Run by scripts/sexdrive_dma_host_gate.sh.
#[path = "../apps/sexdrive/src/ioq.rs"]
mod ioq;
use ioq::*;

fn cqe(cid: u16, phase: u32, status: u32) -> u32 { cid as u32 | (phase << 16) | (status << 17) }

/// Submit everything the batch allows; returns (piece, slot, cid, sq index) per command.
fn fill(batch: &mut Batch, next_cid: &mut u16) -> Vec<(u32, usize, u16, u32)> {
    let mut out = vec![];
    while let Some((piece, slot)) = batch.next_submit() {
        let cid = *next_cid;
        *next_cid = next_cid.wrapping_add(1);
        let at = batch.submitted(slot, cid).expect("room was offered");
        out.push((piece, slot, cid, at));
    }
    out
}

#[test]
fn queue_entries_follow_mqes() {
    assert_eq!(queue_entries(0), 2);
    assert_eq!(queue_entries(15), 16);
    assert_eq!(queue_entries(63), 64);
    assert_eq!(queue_entries(2047), QUEUE_ENTRIES);
    assert_eq!(queue_entries(u16::MAX), QUEUE_ENTRIES);
    assert_eq!(QUEUE_ENTRIES as u64 * 64, 4096, "the SQ still fits its one page");
}

#[test]
fn a_whole_run_is_cut_into_commands() {
    assert_eq!(pieces(512 * 1024, 128 * 1024), 4);
    assert_eq!(pieces(512 * 1024 + 512, 128 * 1024), 5);
    assert_eq!(pieces(512 * 1024, 16 * 1024), 32);
    assert_eq!(pieces(4096, 0), 0);
    assert_eq!(piece(4, 512 * 1024 + 512, 128 * 1024), (512 * 1024, 512));
    assert_eq!(piece(1, 200 * 1024, 128 * 1024), (128 * 1024, 72 * 1024));
}

#[test]
fn thirty_two_commands_go_out_before_one_completion() {
    let mut batch = Batch::new(Ring::idle(64, 0, 0, 1), 32, MAX_IN_FLIGHT);
    let mut next = 0x500;
    let sent = fill(&mut batch, &mut next);
    assert_eq!(sent.len(), 32);
    assert_eq!(batch.flight.len(), 32);
    assert_eq!(batch.ring.sq_tail, 32);
    assert!(batch.next_submit().is_none());
    for (i, &(piece, slot, cid, at)) in sent.iter().enumerate() {
        assert_eq!((piece, slot, cid, at), (i as u32, i, 0x500 + i as u16, i as u32));
    }
    // Reverse order: the last command finishes first.
    for (n, &(piece, slot, cid, _)) in sent.iter().rev().enumerate() {
        let done = batch.complete(n as u32 + 1, cqe(cid, 1, 0)).unwrap();
        assert_eq!((done.slot, done.cid, done.piece, done.status, done.phase), (slot, cid, piece, 0, 1));
    }
    assert!(batch.finished() && batch.succeeded());
    assert_eq!(batch.most_in_flight, 32);
    assert_eq!(batch.ring.cq_head, 32);
}

#[test]
fn out_of_order_completions_free_the_right_slot_and_refill_it() {
    let mut batch = Batch::new(Ring::idle(64, 0, 0, 1), 6, 3);
    let mut next = 10;
    let first = fill(&mut batch, &mut next);
    assert_eq!(first.iter().map(|s| s.2).collect::<Vec<_>>(), vec![10, 11, 12]);
    // CID 11 (slot 1) finishes first: only slot 1 is free, and piece 3 goes there.
    let done = batch.complete(2, cqe(11, 1, 0)).unwrap();
    assert_eq!((done.slot, done.piece), (1, 1));
    let refill = fill(&mut batch, &mut next);
    assert_eq!(refill, vec![(3, 1, 13, 3)]);
    for cid in [12, 10, 13] { batch.complete(4, cqe(cid, 1, 0)).unwrap(); }
    let rest = fill(&mut batch, &mut next);
    assert_eq!(rest.iter().map(|s| (s.0, s.1)).collect::<Vec<_>>(), vec![(4, 0), (5, 1)]);
    for cid in [15, 14] { batch.complete(6, cqe(cid, 1, 0)).unwrap(); }
    assert!(batch.succeeded());
    assert_eq!(batch.submitted_pieces(), 6);
}

#[test]
fn ring_wraps_and_the_phase_flips_at_the_end_of_the_cq() {
    // Start four entries before the end of a 64-entry queue.
    let mut batch = Batch::new(Ring::idle(64, 60, 60, 1), 8, 8);
    let mut next = 0xfffc; // the CID counter wraps too
    let sent = fill(&mut batch, &mut next);
    assert_eq!(sent.iter().map(|s| s.3).collect::<Vec<_>>(), vec![60, 61, 62, 63, 0, 1, 2, 3]);
    assert_eq!(sent.iter().map(|s| s.2).collect::<Vec<_>>(), vec![0xfffc, 0xfffd, 0xfffe, 0xffff, 0, 1, 2, 3]);
    // Four CQEs in the old phase fill entries 60..63, the next four carry phase 0.
    for (n, cid) in [0xffffu16, 0xfffc, 0xfffe, 0xfffd].into_iter().enumerate() {
        let done = batch.complete(61 + n as u32, cqe(cid, 1, 0)).unwrap();
        assert_eq!(done.phase, 1);
    }
    assert_eq!((batch.ring.cq_head, batch.ring.phase), (0, 0));
    // A CQE still carrying phase 1 at entry 0 is last lap's: not new.
    assert_eq!(batch.complete(0, cqe(2, 1, 0)), Err(Refused::Stale));
    for cid in [3u16, 0, 2, 1] {
        assert_eq!(batch.complete(4, cqe(cid, 0, 0)).unwrap().phase, 0);
    }
    assert!(batch.succeeded());
    assert_eq!((batch.ring.cq_head, batch.ring.phase, batch.ring.sq_head), (4, 0, 4));
}

#[test]
fn a_full_sq_holds_submissions_back_until_the_device_reads_entries() {
    // A 4-entry queue holds 3 commands; a request of 6 must wait for SQHD to move.
    let mut batch = Batch::new(Ring::idle(4, 0, 0, 1), 6, MAX_IN_FLIGHT);
    let mut next = 1;
    assert_eq!(fill(&mut batch, &mut next).len(), 3);
    assert_eq!(batch.ring.sq_free(), 0);
    assert!(batch.next_submit().is_none(), "full SQ: nothing more until a CQE");
    // The device read only one SQE so far (SQHD=1): one entry comes free.
    batch.complete(1, cqe(1, 1, 0)).unwrap();
    assert_eq!(batch.ring.sq_free(), 1);
    assert_eq!(fill(&mut batch, &mut next).len(), 1);
    assert!(batch.next_submit().is_none());
    // The device read two more: two entries free (the CQ head wraps in a 4-entry queue).
    batch.complete(3, cqe(2, 1, 0)).unwrap();
    assert_eq!(batch.ring.sq_head, 3);
    batch.complete(0, cqe(3, 1, 0)).unwrap();
    assert_eq!(batch.ring.sq_head, 0);
    assert_eq!(fill(&mut batch, &mut next).len(), 2);
    while !batch.flight.is_empty() {
        let head = batch.ring.cq_head;
        let cid = (1..=6).find(|&c| batch.accepts(c)).unwrap();
        batch.complete(head, cqe(cid, batch.ring.phase, 0)).unwrap();
    }
    assert!(batch.succeeded());
}

#[test]
fn bogus_sq_head_is_ignored() {
    let mut ring = Ring::idle(16, 5, 0, 1);
    ring.push(); ring.push();
    ring.consume(12); // far outside 5..7
    assert_eq!(ring.sq_head, 5);
    ring.consume(0x20); // not even an index
    assert_eq!(ring.sq_head, 5);
    ring.consume(7);
    assert_eq!(ring.sq_head, 7);
    assert_eq!(ring.sq_free(), 15);
}

#[test]
fn the_dma_limit_caps_commands_in_flight() {
    // Three bounce buffers: a four-piece request has three out, then the fourth.
    let mut batch = Batch::new(Ring::idle(64, 0, 0, 1), 4, 3);
    let mut next = 0;
    assert_eq!(fill(&mut batch, &mut next).len(), 3);
    batch.complete(1, cqe(0, 1, 0)).unwrap();
    assert_eq!(fill(&mut batch, &mut next), vec![(3, 0, 3, 3)]);
    for cid in [2, 1, 3] { batch.complete(4, cqe(cid, 1, 0)).unwrap(); }
    assert!(batch.succeeded());
    assert_eq!(batch.most_in_flight, 3);
}

#[test]
fn an_error_on_one_command_stops_submitting_but_reaps_every_other() {
    let mut batch = Batch::new(Ring::idle(64, 0, 0, 1), 8, 4);
    let mut next = 40;
    assert_eq!(fill(&mut batch, &mut next).len(), 4);
    let done = batch.complete(1, cqe(41, 1, 0x281)).unwrap();
    assert_eq!((done.slot, done.status), (1, 0x281));
    assert!(batch.next_submit().is_none(), "no new commands after an error");
    assert!(!batch.finished(), "three commands still own their buffers");
    for cid in [43, 40, 42] { assert_eq!(batch.complete(4, cqe(cid, 1, 0)).unwrap().status, 0); }
    assert!(batch.finished());
    assert!(!batch.succeeded());
    assert_eq!((batch.errors(), batch.first_error()), (1, Some((41, 0x281))));
    assert_eq!(batch.submitted_pieces(), 4);
}

#[test]
fn foreign_or_repeated_cids_are_refused_and_free_nothing() {
    let mut batch = Batch::new(Ring::idle(64, 0, 0, 1), 2, 2);
    let mut next = 7;
    fill(&mut batch, &mut next);
    assert_eq!(batch.complete(1, cqe(99, 1, 0)), Err(Refused::Foreign(99)));
    assert_eq!(batch.ring.cq_head, 0, "a refused CQE is not consumed");
    assert_eq!(batch.flight.len(), 2);
    batch.complete(1, cqe(7, 1, 0)).unwrap();
    // The same CID again (a replayed CQE) is no longer in flight.
    assert_eq!(batch.complete(2, cqe(7, 1, 0)), Err(Refused::Foreign(7)));
    batch.complete(2, cqe(8, 1, 0)).unwrap();
    assert!(batch.succeeded());
}

#[test]
fn in_flight_table_refuses_duplicates_used_slots_and_slots_past_the_limit() {
    let mut flight = InFlight::new(2);
    assert!(flight.insert(0, 5, 0));
    assert!(!flight.insert(0, 6, 1), "slot in use");
    assert!(!flight.insert(1, 5, 1), "CID in flight");
    assert!(!flight.insert(2, 6, 1), "past the limit");
    assert_eq!(flight.free_slot(), Some(1));
    assert_eq!(flight.take(0), Some((5, 0)));
    assert_eq!(flight.take(0), None);
    assert_eq!(InFlight::new(1000).limit(), MAX_IN_FLIGHT);
}

#[test]
fn an_explicit_stop_drains_without_new_commands() {
    let mut batch = Batch::new(Ring::idle(64, 0, 0, 1), 4, 4);
    let mut next = 0;
    let (piece, slot) = batch.next_submit().unwrap();
    assert_eq!((piece, slot), (0, 0));
    batch.submitted(slot, next).unwrap();
    next += 1;
    batch.stop(); // the next command's buffer could not be prepared
    assert!(batch.next_submit().is_none());
    assert!(batch.submitted(1, next).is_none());
    batch.complete(1, cqe(0, 1, 0)).unwrap();
    assert!(batch.finished() && !batch.succeeded());
}
