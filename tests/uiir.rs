//! Native tests of the production UiIR stream over host-allocated pages.
#[path = "../crates/sex-pdx/src/uiir.rs"]
#[allow(dead_code)]
mod uiir;
use uiir::{Consumer, Error, Producer, Record, CAPACITY, CURSOR_BYTES, RING_BYTES};

#[repr(C, align(4096))]
struct Page([u8; 4096]);
fn pages() -> (&'static mut Page, &'static mut Page) {
    assert_eq!((RING_BYTES, CURSOR_BYTES), (4096, 4096));
    (Box::leak(Box::new(Page([0; 4096]))), Box::leak(Box::new(Page([0; 4096]))))
}
fn pair() -> (Producer, Consumer, *mut u8) {
    let (ring, cursor) = pages();
    let ring = ring.0.as_mut_ptr();
    let cursor = cursor.0.as_mut_ptr();
    unsafe { (Producer::attach(ring, cursor), Consumer::attach(ring, cursor), ring) }
}

#[test]
fn records_are_invisible_until_their_epoch_commits() {
    let (mut p, mut c, _) = pair();
    assert_eq!(c.poll(), Ok(None));
    assert_eq!(p.push(7, 70), Ok(1));
    assert_eq!(p.push(8, 80), Ok(2));
    assert_eq!(c.poll(), Ok(None), "staged, not committed");
    assert_eq!(p.commit(), 1);
    assert_eq!(c.poll(), Ok(Some(Record { sequence: 1, epoch: 1, kind: 7, payload: 70 })));
    assert_eq!(c.poll(), Ok(Some(Record { sequence: 2, epoch: 1, kind: 8, payload: 80 })));
    assert_eq!(c.poll(), Ok(None));
    assert_eq!(p.commit(), 1, "an empty commit opens no epoch");
    p.push(9, 90).unwrap();
    assert_eq!(p.commit(), 2);
    assert_eq!(c.poll().unwrap().unwrap().epoch, 2);
}

#[test]
fn producer_never_overwrites_unconsumed_records() {
    let (mut p, mut c, _) = pair();
    for i in 0..CAPACITY { p.push(1, i).unwrap(); }
    assert_eq!(p.push(1, 0), Err(Error::Full));
    p.commit();
    // Consuming frees exactly one slot at a time.
    c.poll().unwrap().unwrap();
    assert!(p.push(1, 999).is_ok());
    assert_eq!(p.push(1, 0), Err(Error::Full));
}

#[test]
fn sequences_stay_contiguous_across_many_wraps() {
    let (mut p, mut c, _) = pair();
    let mut expected = 1;
    for round in 0..50u64 {
        for i in 0..(round % 17 + 1) { p.push(round, i).unwrap(); }
        p.commit();
        while let Some(r) = c.poll().unwrap() {
            assert_eq!(r.sequence, expected);
            expected += 1;
        }
    }
    assert!(expected - 1 > 3 * CAPACITY);
}

#[test]
fn consumer_rejects_a_record_that_breaks_the_stream() {
    let (mut p, mut c, ring) = pair();
    p.push(1, 1).unwrap();
    p.push(1, 2).unwrap();
    p.commit();
    // Corrupt slot 2's sequence: the consumer stops there, not past it.
    unsafe { *(ring.add(uiir::HEADER_BYTES + uiir::RECORD_BYTES) as *mut u64) = 7; }
    assert!(c.poll().unwrap().is_some());
    assert_eq!(c.poll(), Err(Error::Order { expected: 2, got: 7 }));
    assert_eq!(c.poll(), Err(Error::Order { expected: 2, got: 7 }), "not consumed");
    assert_eq!(c.consumed(), 1);
}

#[test]
fn an_inflated_consumer_cursor_cannot_make_the_producer_overwrite() {
    let (ring, cursor) = pages();
    let (ring, cursor) = (ring.0.as_mut_ptr(), cursor.0.as_mut_ptr());
    let mut p = unsafe { Producer::attach(ring, cursor) };
    for i in 0..CAPACITY { p.push(1, i).unwrap(); }
    // A faulty consumer claims to have consumed far more than was committed.
    unsafe { *(cursor as *mut u64) = u64::MAX / 2; }
    assert_eq!(p.push(1, 0), Err(Error::Full));
}

#[test]
fn concurrent_producer_and_consumer_agree_on_order() {
    let (ring, cursor) = pages();
    let ring = ring.0.as_mut_ptr() as usize;
    let cursor = cursor.0.as_mut_ptr() as usize;
    let total = 200_000u64;
    let producer = std::thread::spawn(move || {
        let mut p = unsafe { Producer::attach(ring as *mut u8, cursor as *const u8) };
        let mut sent = 0;
        while sent < total {
            match p.push(3, sent) { Ok(_) => sent += 1, Err(Error::Full) => { p.commit(); std::thread::yield_now(); } Err(e) => panic!("{e:?}") }
            if sent % 13 == 0 { p.commit(); }
        }
        p.commit();
    });
    let mut c = unsafe { Consumer::attach(ring as *const u8, cursor as *mut u8) };
    let mut next = 0;
    let mut last_epoch = 0;
    while next < total {
        match c.poll().unwrap() {
            Some(r) => { assert_eq!((r.sequence, r.payload), (next + 1, next)); assert!(r.epoch >= last_epoch); last_epoch = r.epoch; next += 1; }
            None => std::thread::yield_now(),
        }
    }
    producer.join().unwrap();
}

#[test]
fn boot_grants_one_writer_per_page_and_both_sides_check_exact_peer_and_rights() {
    let boot = include_str!("../kernel/src/boot_memory.rs");
    let init = boot.split("pub fn init_uiir(").nth(1).unwrap().split("\n}\n").next().unwrap();
    // Ring: producer writable, consumer read-only. Cursor: the reverse.
    assert!(init.contains("paired_at(producer, consumer, RING_SLOT, RING_SLOT, RING_REGION_ID, RING_BYTES as u64, false);"));
    assert!(init.contains("paired_at(consumer, producer, CURSOR_SLOT, CURSOR_SLOT, CURSOR_REGION_ID, CURSOR_BYTES as u64, false);"));
    assert!(include_str!("../kernel/src/init.rs").contains("crate::boot_memory::init_uiir(silkshell_id, sexdisp_id);"));
    let pdx = include_str!("../crates/sex-pdx/src/lib.rs");
    let views = pdx.split("fn uiir_views(").nth(1).unwrap().split("\npub fn uiir_producer").next().unwrap();
    assert!(views.contains("info.region_id != region || info.peer_pd != Some(peer) || info.rights != rights"));
    assert_eq!((uiir::RING_REGION_ID, uiir::CURSOR_REGION_ID), (9, 10));
}
