// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
// Host tests for sex-lanes. Run by scripts/lanes_host_gate.sh.
use sex_lanes::{nvme, plan::Plan, poll::{Action, Adaptive, Config, Mode}, ring::*, rss};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[repr(C, align(64))]
struct Mem([u8; 8192]);
fn mem() -> Box<Mem> { Box::new(Mem([0; 8192])) }

#[test]
fn ring_round_trip_and_wrap() {
    let mut m = mem();
    let base = m.0.as_mut_ptr();
    unsafe { assert!(init::<BlockRequest>(base, 8)); }
    let mut p = unsafe { Producer::<BlockRequest>::attach(base, 8).unwrap() };
    let mut c = unsafe { Consumer::<BlockRequest>::attach(base, 8).unwrap() };
    for round in 0..10u16 {
        for i in 0..8u16 {
            let r = BlockRequest { op: BLOCK_READ, tag: round * 8 + i, len: 4096, offset: i as u64 * 4096, ..Default::default() };
            assert_eq!(p.push(r), Ok(false));
        }
        assert_eq!(p.push(BlockRequest::default()), Err(PushError::Full));
        let mut out = [BlockRequest::default(); 5];
        assert_eq!(c.pop_many(&mut out), Ok(5));
        assert_eq!(out[4].tag, round * 8 + 4);
        assert_eq!(p.free(), Ok(5));
        while let Some(r) = c.pop().unwrap() { assert!(r.tag >= round * 8 + 5); }
    }
}

#[test]
fn bad_sizes_refused() {
    let mut m = mem();
    unsafe {
        assert!(!init::<BlockRequest>(m.0.as_mut_ptr(), 6));
        assert!(!init::<BlockRequest>(m.0.as_mut_ptr(), 0));
        assert!(!init::<BlockRequest>(m.0.as_mut_ptr().add(8), 8), "misaligned base");
        assert!(init::<BlockRequest>(m.0.as_mut_ptr(), 8));
        assert!(Consumer::<BlockRequest>::attach(m.0.as_mut_ptr(), 16).is_none(), "entries must match");
    }
    assert!(bytes_for::<BlockRequest>(64) <= 8192);
}

#[test]
fn a_lying_peer_is_corrupt_not_a_crash() {
    let mut m = mem();
    let base = m.0.as_mut_ptr();
    unsafe { init::<Packet>(base, 16); }
    let mut c = unsafe { Consumer::<Packet>::attach(base, 16).unwrap() };
    let mut p = unsafe { Producer::<Packet>::attach(base, 16).unwrap() };
    let h = unsafe { &*(base as *const Header) };
    h.tail.value.store(1000, Ordering::Release);
    assert_eq!(c.ready(), Err(Corrupt));
    assert_eq!(c.pop(), Err(Corrupt));
    h.tail.value.store(0, Ordering::Release);
    h.head.value.store(5, Ordering::Release);
    assert_eq!(p.free(), Err(Corrupt), "consumer claims it read more than was written");
    assert_eq!(p.push(Packet::default()), Err(PushError::Corrupt));
}

#[test]
fn wakeup_only_when_asked() {
    let mut m = mem();
    let base = m.0.as_mut_ptr();
    unsafe { init::<u64>(base, 4); }
    let mut p = unsafe { Producer::<u64>::attach(base, 4).unwrap() };
    let mut c = unsafe { Consumer::<u64>::attach(base, 4).unwrap() };
    assert_eq!(p.push(1), Ok(false), "busy consumer: no signal");
    assert_eq!(c.prepare_sleep(), Ok(false), "entries waiting: do not sleep");
    assert_eq!(c.pop(), Ok(Some(1)));
    assert_eq!(c.prepare_sleep(), Ok(true));
    assert_eq!(p.push(2), Ok(true), "sleeping consumer: signal");
    assert_eq!(p.push_many(&[]), Ok((0, false)), "nothing published, nothing to signal");
    c.awake();
    assert_eq!(p.push_many(&[3, 4]), Ok((2, false)));
    assert_eq!(p.push_many(&[5, 6]), Ok((1, false)), "only one slot left");
}

/// Two threads, millions of items, the consumer sleeping whenever empty: nothing is
/// lost or duplicated, and no wake-up is missed (the test would hang).
#[test]
fn threads_never_lose_a_wakeup() {
    const N: u64 = 2_000_000;
    let mut m = mem();
    let base = m.0.as_mut_ptr() as usize;
    unsafe { init::<u64>(base as *mut u8, 64); }
    let signal = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let done = Arc::new(AtomicBool::new(false));
    let (s2, d2) = (signal.clone(), done.clone());
    let consumer = std::thread::spawn(move || {
        let mut c = unsafe { Consumer::<u64>::attach(base as *mut u8, 64).unwrap() };
        let (mut next, mut sleeps) = (0u64, 0u64);
        let mut buf = [0u64; 16];
        while next < N {
            let n = c.pop_many(&mut buf).unwrap();
            for v in &buf[..n] { assert_eq!(*v, next); next += 1; }
            if n == 0 && c.prepare_sleep().unwrap() {
                let (lock, cv) = &*s2;
                let mut flag = lock.lock().unwrap();
                while !*flag { flag = cv.wait(flag).unwrap(); }
                *flag = false;
                sleeps += 1;
                c.awake();
            }
        }
        d2.store(true, Ordering::Release);
        sleeps
    });
    let mut p = unsafe { Producer::<u64>::attach(base as *mut u8, 64).unwrap() };
    let mut v = 0u64;
    let mut signals = 0;
    while v < N {
        let batch: Vec<u64> = (v..(v + 7).min(N)).collect();
        let (n, wake) = p.push_many(&batch).unwrap();
        v += n as u64;
        if wake {
            signals += 1;
            let (lock, cv) = &*signal;
            *lock.lock().unwrap() = true;
            cv.notify_one();
        }
        if n == 0 { std::thread::yield_now(); }
    }
    let sleeps = consumer.join().unwrap();
    assert!(done.load(Ordering::Acquire));
    assert!(signals >= sleeps, "every sleep was woken by a signal ({signals} signals, {sleeps} sleeps)");
}

#[test]
fn polling_starts_on_a_burst_and_stops_when_idle() {
    let cfg = Config { min_batch: 2, max_batch: 32, idle_rounds: 4, max_rounds: 100 };
    let mut a = Adaptive::new(cfg);
    assert_eq!(a.enter_batch(), 8);
    assert_eq!(a.round(1), Action::Poll);
    assert_eq!(a.round(0), Action::Sleep);
    assert_eq!(a.round(8), Action::MaskAndPoll);
    assert_eq!(a.mode(), Mode::Polling);
    for _ in 0..3 { assert_eq!(a.round(0), Action::Poll); }
    assert_eq!(a.round(5), Action::Poll, "work resets the idle count");
    for _ in 0..3 { assert_eq!(a.round(0), Action::Poll); }
    assert_eq!(a.round(0), Action::UnmaskAndSleep);
    assert_eq!(a.mode(), Mode::Interrupt);
}

#[test]
fn polling_yields_under_endless_load() {
    let cfg = Config { min_batch: 2, max_batch: 32, idle_rounds: 4, max_rounds: 10 };
    let mut a = Adaptive::new(cfg);
    a.round(100);
    let yields = (0..100).filter(|_| a.round(3) == Action::Yield).count();
    assert_eq!(yields, 10);
    assert_eq!(a.mode(), Mode::Polling);
}

#[test]
fn entry_threshold_adapts() {
    let cfg = Config { min_batch: 2, max_batch: 32, idle_rounds: 2, max_rounds: 1000 };
    let mut a = Adaptive::new(cfg);
    // A burst followed by nothing: polling was wasted, so the bar goes up.
    a.round(8); a.round(0); assert_eq!(a.round(0), Action::UnmaskAndSleep);
    assert_eq!(a.enter_batch(), 16);
    a.round(16); a.round(0); a.round(0);
    assert_eq!(a.enter_batch(), 32);
    a.round(32); a.round(0); a.round(0);
    assert_eq!(a.enter_batch(), 32, "capped at max_batch");
    // A stretch full of work: the bar comes down.
    a.round(32);
    for _ in 0..10 { a.round(40); }
    a.round(0); a.round(0);
    assert_eq!(a.enter_batch(), 16);
    assert!(a.polled_items >= 400);
}

#[test]
fn nvme_queue_commands() {
    let q = nvme::request_queues(8);
    assert_eq!(q.opcode, 0x09);
    assert_eq!(q.cdw10, 0x07);
    assert_eq!(q.cdw11, 0x0007_0007);
    assert_eq!(nvme::request_queues(0).cdw11, 0, "at least one");
    assert_eq!(nvme::granted_queues(0x0003_0007, 8), 4, "fewer completion queues granted");
    assert_eq!(nvme::granted_queues(0x001f_001f, 8), 8, "more granted than asked");
    let cq = nvme::create_cq(3, 64, 0x1234_5000, Some(3));
    assert_eq!((cq.opcode, cq.cdw10, cq.cdw11, cq.prp1), (0x05, 63 << 16 | 3, 3 << 16 | 2 | 1, 0x1234_5000));
    assert_eq!(nvme::create_cq(2, 64, 0, None).cdw11, 1, "polled queue: IEN clear");
    let sq = nvme::create_sq(3, 64, 0x9_0000_0000, 3);
    assert_eq!((sq.opcode, sq.cdw10, sq.cdw11), (0x01, 63 << 16 | 3, 3 << 16 | 1));
    let e = sq.entry(0xBEEF);
    assert_eq!(e[0], 0x01 | 0xBEEF << 16);
    assert_eq!((e[6], e[7]), (0, 9));
    assert_eq!((nvme::delete_sq(4).opcode, nvme::delete_sq(4).cdw10, nvme::delete_cq(4).opcode), (0, 4, 4));
}

#[test]
fn nvme_doorbells_follow_the_stride() {
    let cap = 0x0000_0000_0000_07ffu64; // DSTRD 0
    assert_eq!(nvme::sq_tail_doorbell(0, cap), 0x1000);
    assert_eq!(nvme::cq_head_doorbell(0, cap), 0x1004);
    assert_eq!(nvme::sq_tail_doorbell(3, cap), 0x1018);
    assert_eq!(nvme::cq_head_doorbell(3, cap), 0x101C);
    let wide = cap | 2 << 32; // 16-byte stride
    assert_eq!(nvme::sq_tail_doorbell(1, wide), 0x1020);
    assert_eq!(nvme::cq_head_doorbell(1, wide), 0x1030);
    assert_eq!(nvme::max_entries(cap), 2048);
}

#[test]
fn plan_one_queue_per_core_when_possible() {
    let p = Plan::new(8, 8, 9, true);
    for c in 0..8 {
        assert_eq!(p.queue_of(c), c);
        assert_eq!(p.home_of(c), c);
        assert_eq!(p.vector_of(c), c + 1, "vector 0 stays the admin queue's");
        assert!(p.vector_private(c));
    }
}

#[test]
fn plan_shares_when_the_device_is_small() {
    // A drive with 4 queue pairs and 2 MSI-X vectors on a 16-thread x17.
    let p = Plan::new(16, 4, 2, true);
    assert_eq!(p.queues, 4);
    assert_eq!((p.queue_of(0), p.queue_of(5), p.queue_of(15)), (0, 1, 3));
    assert_eq!(p.vector_of(0), 1);
    assert_eq!(p.vector_of(3), 1, "one I/O vector for every queue");
    assert!(!p.vector_private(0));
    // More queues than cores: only as many as cores are used.
    let p = Plan::new(2, 8, 8, false);
    assert_eq!(p.queues, 2);
    assert_eq!(p.vector_of(1), 1);
    // One vector, not reserved for admin: everything shares it.
    let p = Plan::new(4, 4, 1, true);
    assert_eq!((p.vector_base, p.vector_of(3)), (0, 0));
}

#[test]
fn toeplitz_matches_the_microsoft_verification_suite() {
    let k = &rss::DEFAULT_KEY;
    let cases = [
        ([66, 9, 149, 187], [161, 142, 100, 80], 2794, 1766, 0x323e8fc2u32, 0x51ccc178u32),
        ([199, 92, 111, 2], [65, 69, 140, 83], 14230, 4739, 0xd718262a, 0xc626b0ea),
        ([24, 19, 198, 95], [12, 22, 207, 184], 12898, 38024, 0xd2d0a5de, 0x5c2b394a),
        ([38, 27, 205, 30], [209, 142, 163, 6], 48228, 2217, 0x82989176, 0xafc7327f),
        ([153, 39, 163, 191], [202, 188, 127, 2], 44251, 1303, 0x5d1809c5, 0x10e828a2),
    ];
    for (src, dst, sport, dport, addr, flow) in cases {
        assert_eq!(rss::ipv4_addresses(k, src, dst), addr, "{src:?} -> {dst:?}");
        assert_eq!(rss::ipv4_flow(k, src, dst, sport, dport), flow, "{src:?}:{sport} -> {dst:?}:{dport}");
    }
}

/// IPv6 inputs are 36 bytes, which needs every byte of the 40-byte key: checked
/// against the definition bit by bit (key bits i..i+32 for input bit i).
#[test]
fn toeplitz_long_inputs_match_the_definition() {
    fn key_bit(k: &[u8], i: usize) -> u32 { k.get(i / 8).map_or(0, |b| (b >> (7 - i % 8)) as u32 & 1) }
    fn reference(k: &[u8], input: &[u8]) -> u32 {
        let mut r = 0u32;
        for i in 0..input.len() * 8 {
            if input[i / 8] >> (7 - i % 8) & 1 != 0 { r ^= (0..32).fold(0, |w, j| w << 1 | key_bit(k, i + j)); }
        }
        r
    }
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..200 {
        let mut src = [0u8; 16]; let mut dst = [0u8; 16];
        for b in src.iter_mut().chain(dst.iter_mut()) { x ^= x << 13; x ^= x >> 7; x ^= x << 17; *b = x as u8; }
        let (sp, dp) = (x as u16, (x >> 16) as u16);
        let mut input = [0u8; 36];
        input[..16].copy_from_slice(&src); input[16..32].copy_from_slice(&dst);
        input[32..34].copy_from_slice(&sp.to_be_bytes()); input[34..].copy_from_slice(&dp.to_be_bytes());
        assert_eq!(rss::ipv6_flow(&rss::DEFAULT_KEY, src, dst, sp, dp), reference(&rss::DEFAULT_KEY, &input));
    }
}

#[test]
fn indirection_table_spreads_and_keeps_flows_together() {
    let t = rss::spread(4);
    assert_eq!(&t[..6], &[0, 1, 2, 3, 0, 1]);
    let mut seen = [0u32; 4];
    for port in 1000..5000u16 {
        let h = rss::ipv4_flow(&rss::DEFAULT_KEY, [10, 0, 2, 2], [10, 0, 2, 15], port, 443);
        let q = rss::queue_of(h, &t);
        assert_eq!(q, rss::queue_of(h, &t));
        seen[q as usize] += 1;
    }
    assert!(seen.iter().all(|&n| n > 700), "flows spread over all queues: {seen:?}");
    let w = rss::intel::reta_words(&t);
    assert_eq!(w[0], 0x0302_0100);
    assert_eq!(rss::intel::key_words(&rss::DEFAULT_KEY)[0], 0xda56_5a6d);
    assert_eq!((rss::intel::rdt(1), rss::intel::tdt(3)), (0xC058, 0xE0D8));
}

#[test]
fn hybrid_spins_about_as_long_as_the_device_takes() {
    use sex_lanes::poll::Hybrid;
    let mut h = Hybrid::new(1_000, 100_000);
    assert_eq!(h.spin_budget(), 1_000, "no history: only the floor");
    for _ in 0..32 { h.record(20_000, true); }
    let b = h.spin_budget();
    assert!((25_000..=32_000).contains(&b), "about 1.5x a 20k average, got {b}");
    // The device turns slow (a flush, a sleeping disk): spinning drops to the floor.
    for _ in 0..64 { h.record(5_000_000, false); }
    assert_eq!(h.spin_budget(), 1_000);
    // And recovers when it is quick again.
    for _ in 0..64 { h.record(10_000, true); }
    assert!((10_000..=20_000).contains(&h.spin_budget()), "{}", h.spin_budget());
    assert_eq!((h.spun_hits, h.slept), (96, 64));
    // Never past the cap.
    let mut h = Hybrid::new(1_000, 50_000);
    for _ in 0..32 { h.record(45_000, true); }
    assert_eq!(h.spin_budget(), 50_000);
}
