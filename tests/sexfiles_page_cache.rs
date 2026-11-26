// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for `servers/sexfiles/src/page_cache.rs` and IPC v2 endpoint
//! request/reply dispatch (`crates/sex-pdx/src/endpoint.rs` + `servers/sexfiles/src/ipc_v2.rs`).

extern crate alloc;

#[path = "../servers/sexfiles/src/page_cache.rs"]
mod page_cache;

// Minimal host-compatible view of the pure IPC v2 encoding/decoding & dispatch logic.
// (Avoids x86_64 `asm!("syscall")` on non-x86_64 hosts while testing the exact
// production wire format and dispatch logic.)
#[allow(dead_code)]
mod ipc_v2_wire {
    include!(env!("SEXFILES_IPC_V2_HOST_SRC"));
}

use page_cache::{
    budget_for_heap, read_cached_run, HeapBudget, PageCache, ReadAheadTracker, BLOCK_SIZE,
    DEFAULT_CAPACITY, FIXED_HEAP_BYTES, GROWN_HEAP_BASE_CAPACITY, GROWN_HEAP_HIGH_CAPACITY,
    GROWN_HEAP_MID_CAPACITY, INITIAL_AHEAD_WINDOW, MAX_AHEAD_WINDOW, MAX_STREAMS,
    TIGHT_HEAP_MAX_BLOCKS, TIGHT_HEAP_MIN_FREE,
};

fn pattern_block(block: u64, gen: u8) -> [u8; BLOCK_SIZE] {
    let mut buf = [0u8; BLOCK_SIZE];
    for (i, b) in buf.iter_mut().enumerate() {
        *b = ((block as usize).wrapping_mul(31).wrapping_add(i).wrapping_add(gen as usize)) as u8;
    }
    buf
}

#[test]
fn page_cache_hit_miss_and_fallback() {
    let mut cache = PageCache::new_with_fallback(DEFAULT_CAPACITY);
    assert_eq!(cache.capacity(), DEFAULT_CAPACITY);
    assert!(cache.is_empty());

    let mut out = [0u8; BLOCK_SIZE];
    assert_eq!(cache.get(42, &mut out), None);
    assert_eq!(cache.stats().misses, 1);
    assert_eq!(cache.stats().hits, 0);

    let b42 = pattern_block(42, 1);
    cache.insert(42, &b42, false);
    assert_eq!(cache.len(), 1);
    assert!(cache.contains(42));

    assert_eq!(cache.get(42, &mut out), Some(false));
    assert_eq!(out, b42);
    let st = cache.stats();
    assert_eq!(st.hits, 1);
    assert_eq!(st.misses, 1);
    assert_eq!(st.readahead_hits, 0);
    assert_eq!(st.evictions, 0);

    // Prefetched block reports was_prefetched=true once, then false on subsequent hits.
    let b43 = pattern_block(43, 2);
    cache.insert(43, &b43, true);
    assert_eq!(cache.get(43, &mut out), Some(true));
    assert_eq!(out, b43);
    assert_eq!(cache.get(43, &mut out), Some(false));
    assert_eq!(cache.stats().readahead_hits, 1);
    assert_eq!(cache.stats().hits, 3);

    // Zero-capacity cache degrades gracefully to pass-through.
    let mut empty = PageCache::empty();
    assert_eq!(empty.capacity(), 0);
    empty.insert(1, &b42, false);
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.get(1, &mut out), None);
}

#[test]
fn page_cache_clock_eviction_and_hash_integrity() {
    let mut cache = PageCache::try_new(4).expect("4-block cache");
    let mut out = [0u8; BLOCK_SIZE];

    // Insert 2 hot demand pages (referenced=true) and 2 prefetched pages (referenced=false).
    cache.insert(10, &pattern_block(10, 0), false);
    cache.insert(11, &pattern_block(11, 0), true);
    cache.insert(12, &pattern_block(12, 0), false);
    cache.insert(13, &pattern_block(13, 0), true);
    assert_eq!(cache.len(), 4);

    // Next insertion should evict prefetched block 11 (referenced=false) before hot block 10.
    cache.insert(14, &pattern_block(14, 0), false);
    assert!(cache.contains(10), "hot demand block 10 should survive first eviction");
    assert!(!cache.contains(11), "unconsumed prefetched block 11 should be evicted first");
    assert_eq!(cache.stats().evictions, 1);

    // Stress backward-shift deletion across hundreds of evictions.
    for blk in 0..500u64 {
        cache.insert(blk, &pattern_block(blk, 7), false);
        assert!(cache.len() <= 4);
    }
    assert_eq!(cache.len(), 4);
    // The last 4 inserted blocks must all be intact and readable.
    for blk in 496..500u64 {
        assert_eq!(cache.get(blk, &mut out), Some(false));
        assert_eq!(out, pattern_block(blk, 7));
    }
}

#[test]
fn page_cache_write_through_invalidation_and_remount_clear() {
    let mut cache = PageCache::try_new(16).expect("16-block cache");
    let mut out = [0u8; BLOCK_SIZE];

    for blk in 0..8u64 {
        cache.insert(blk, &pattern_block(blk, 1), false);
    }

    // Write-through updates blocks 2..4 coherently.
    let mut write_buf = vec![0u8; 2 * BLOCK_SIZE];
    write_buf[..BLOCK_SIZE].copy_from_slice(&pattern_block(2, 9));
    write_buf[BLOCK_SIZE..].copy_from_slice(&pattern_block(3, 9));
    cache.write_through(2, &write_buf);

    assert_eq!(cache.get(2, &mut out), Some(false));
    assert_eq!(out, pattern_block(2, 9));
    assert_eq!(cache.get(3, &mut out), Some(false));
    assert_eq!(out, pattern_block(3, 9));
    assert_eq!(cache.stats().writes_updated, 2);

    // Surgical invalidation of range [3, 6).
    cache.invalidate_range(3, 3);
    assert!(cache.contains(2));
    assert!(!cache.contains(3));
    assert!(!cache.contains(4));
    assert!(!cache.contains(5));
    assert!(cache.contains(6));
    assert_eq!(cache.stats().invalidations, 3);

    // Large-range invalidation (>= capacity) uses full-table scan path.
    cache.invalidate_range(0, 64);
    assert!(cache.is_empty());

    // Remount clear drops everything cleanly.
    cache.insert(99, &pattern_block(99, 3), false);
    assert_eq!(cache.len(), 1);
    cache.clear();
    assert!(cache.is_empty());
    assert_eq!(cache.get(99, &mut out), None);
}

#[test]
fn read_cached_run_fetches_only_contiguous_missing_ranges() {
    let mut cache = PageCache::try_new(32).expect("32-block cache");
    let mut tracker = ReadAheadTracker::new();
    let mut staging = vec![0u8; 16 * BLOCK_SIZE];

    // Pre-cache blocks 0 and 4.
    cache.insert(0, &pattern_block(0, 5), false);
    cache.insert(4, &pattern_block(4, 5), false);

    let mut calls: Vec<(u64, usize)> = Vec::new();
    let mut out = vec![0u8; 5 * BLOCK_SIZE];
    let outcome = read_cached_run(
        &mut cache,
        &mut tracker,
        1000,
        0,
        &mut out,
        &mut staging,
        false,
        0,
        |first, count, buf| {
            calls.push((first, count));
            for i in 0..count {
                buf[i * BLOCK_SIZE..(i + 1) * BLOCK_SIZE]
                    .copy_from_slice(&pattern_block(first + i as u64, 5));
            }
            Ok(())
        },
    )
    .expect("read_cached_run");

    // Only one contiguous missing run [1..4) should be requested from the device.
    assert_eq!(calls, vec![(1, 3)]);
    assert_eq!(outcome.device_calls, 1);
    assert_eq!(outcome.demand_device_bytes, 3 * BLOCK_SIZE);
    assert_eq!(outcome.cache_hit_bytes, 2 * BLOCK_SIZE);
    for blk in 0..5u64 {
        assert_eq!(
            &out[blk as usize * BLOCK_SIZE..(blk as usize + 1) * BLOCK_SIZE],
            &pattern_block(blk, 5)[..]
        );
    }

    // Re-reading the same range hits 100% in cache with 0 device calls.
    calls.clear();
    let outcome2 = read_cached_run(
        &mut cache,
        &mut tracker,
        1000,
        0,
        &mut out,
        &mut staging,
        false,
        0,
        |first, count, _| {
            calls.push((first, count));
            Ok(())
        },
    )
    .expect("second read");
    assert!(calls.is_empty());
    assert_eq!(outcome2.device_calls, 0);
    assert_eq!(outcome2.cache_hit_bytes, 5 * BLOCK_SIZE);
}

#[test]
fn multistream_interleaved_sequential_readahead_survives_and_adapts() {
    let mut cache = PageCache::try_new(256).expect("256-block cache");
    let mut tracker = ReadAheadTracker::new();
    // 64-block staging buffer (256 KiB).
    let mut staging = vec![0u8; 64 * BLOCK_SIZE];

    let mut device_reads: Vec<(u64, usize)> = Vec::new();
    let mut out = vec![0u8; 2 * BLOCK_SIZE];

    let mut do_read = |cache: &mut PageCache,
                       tracker: &mut ReadAheadTracker,
                       first: u64,
                       out: &mut [u8]| {
        read_cached_run(
            cache,
            tracker,
            10_000,
            first,
            out,
            &mut staging,
            true,
            0,
            |blk, n, buf| {
                device_reads.push((blk, n));
                for k in 0..n {
                    buf[k * BLOCK_SIZE..(k + 1) * BLOCK_SIZE]
                        .copy_from_slice(&pattern_block(blk + k as u64, 3));
                }
                Ok(())
            },
        )
        .expect("read ok")
    };

    // Interleave Stream A (LBA 100..) and Stream B (LBA 500..), 2 blocks per step.
    let mut a_ra_hits = 0usize;
    let mut b_ra_hits = 0usize;
    for step in 0..32u64 {
        let a_blk = 100 + step * 2;
        let oa = do_read(&mut cache, &mut tracker, a_blk, &mut out);
        assert_eq!(&out[..BLOCK_SIZE], &pattern_block(a_blk, 3)[..]);
        assert_eq!(&out[BLOCK_SIZE..], &pattern_block(a_blk + 1, 3)[..]);
        a_ra_hits += oa.readahead_hit_bytes / BLOCK_SIZE;

        let b_blk = 500 + step * 2;
        let ob = do_read(&mut cache, &mut tracker, b_blk, &mut out);
        assert_eq!(&out[..BLOCK_SIZE], &pattern_block(b_blk, 3)[..]);
        assert_eq!(&out[BLOCK_SIZE..], &pattern_block(b_blk + 1, 3)[..]);
        b_ra_hits += ob.readahead_hit_bytes / BLOCK_SIZE;
    }

    // Both streams must achieve substantial read-ahead hits (proving Stream B
    // did NOT wipe Stream A's buffered read-ahead!).
    assert!(
        a_ra_hits >= 40,
        "Stream A should get strong read-ahead hits despite Stream B interleaving (got {})",
        a_ra_hits
    );
    assert!(
        b_ra_hits >= 40,
        "Stream B should get strong read-ahead hits despite Stream A interleaving (got {})",
        b_ra_hits
    );

    // 64 blocks per stream = 128 blocks total, served in far fewer than 64 device calls!
    assert!(
        device_reads.len() <= 16,
        "adaptive batched read-ahead should coalesce 64 requests into <=16 device calls (got {})",
        device_reads.len()
    );

    // Verify adaptive window growth reached MAX_AHEAD_WINDOW (32) for both streams.
    let max_fetch = device_reads.iter().map(|(_, n)| *n).max().unwrap_or(0);
    assert!(
        max_fetch >= 2 + MAX_AHEAD_WINDOW,
        "expected window to grow to {} (max fetch was {})",
        MAX_AHEAD_WINDOW,
        max_fetch
    );
}

#[test]
fn tracker_supports_eight_concurrent_streams_and_window_progression() {
    let mut tracker = ReadAheadTracker::new();

    // Establish 8 independent streams at disjoint LBAs.
    for s in 0..MAX_STREAMS as u64 {
        let base = 1000 * (s + 1);
        assert_eq!(tracker.observe_miss(base, 1, 0, 64), 0);
    }
    assert_eq!(tracker.active_streams(), MAX_STREAMS);

    // Verify 4 -> 8 -> 16 -> 32 progression for every one of the 8 streams concurrently.
    let expected_windows = [
        INITIAL_AHEAD_WINDOW,
        INITIAL_AHEAD_WINDOW * 2,
        INITIAL_AHEAD_WINDOW * 4,
        MAX_AHEAD_WINDOW,
        MAX_AHEAD_WINDOW,
    ];
    for (step, &want) in expected_windows.iter().enumerate() {
        for s in 0..MAX_STREAMS as u64 {
            let blk = 1000 * (s + 1) + 1 + step as u64;
            let got = tracker.observe_miss(blk, 1, 0, 64);
            assert_eq!(got, want, "stream {} step {} window mismatch", s, step);
        }
    }

    // 9th stream evicts the LRU slot without exceeding MAX_STREAMS.
    assert_eq!(tracker.observe_miss(99_000, 1, 0, 64), 0);
    assert_eq!(tracker.active_streams(), MAX_STREAMS);
}

#[test]
fn read_cached_run_clamps_readahead_at_disk_end() {
    let mut cache = PageCache::try_new(64).expect("cache");
    let mut tracker = ReadAheadTracker::new();
    let mut staging = vec![0u8; 32 * BLOCK_SIZE];
    let mut out = vec![0u8; BLOCK_SIZE];
    let disk_blocks = 20u64;

    let mut max_end = 0u64;
    for blk in 15..20u64 {
        read_cached_run(
            &mut cache,
            &mut tracker,
            disk_blocks,
            blk,
            &mut out,
            &mut staging,
            true,
            16,
            |first, n, buf| {
                let end = first + n as u64;
                assert!(end <= disk_blocks, "read past end of disk: {} > {}", end, disk_blocks);
                max_end = max_end.max(end);
                for k in 0..n {
                    buf[k * BLOCK_SIZE..(k + 1) * BLOCK_SIZE]
                        .copy_from_slice(&pattern_block(first + k as u64, 1));
                }
                Ok(())
            },
        )
        .expect("clamped read");
    }
    assert_eq!(max_end, disk_blocks);
}

#[test]
fn ipc_v2_endpoint_request_reply_dispatch() {
    use ipc_v2_wire::{
        decode_badge, dispatch_message, encode_badge, EndpointRequest, MessageInfo, Received,
        E_BADMSG,
    };

    // Badge round-trip.
    let badge = encode_badge(11, 42);
    assert_eq!(decode_badge(badge), (11, 42));

    // Valid request dispatch (e.g. fs::OP with 3 payload registers).
    let fs_op_label: u32 = 0x5E01;
    let info = MessageInfo::new(fs_op_label, 3, 0).expect("valid info").raw();
    let got = Received {
        badge,
        words: [info, 0x111, 0x222, 0x333],
    };

    let mut seen: Option<EndpointRequest> = None;
    let res = dispatch_message(&got, |req| {
        seen = Some(req);
        Some(req.arg0 ^ req.arg1 ^ req.arg2 ^ (req.caller_pd as u64))
    });

    let req = seen.expect("handler called");
    assert_eq!(req.caller_pd, 11);
    assert_eq!(req.caller_task, 42);
    assert_eq!(req.label, fs_op_label);
    assert_eq!((req.arg0, req.arg1, req.arg2), (0x111, 0x222, 0x333));
    assert!(res.should_reply);
    assert_eq!(res.label, fs_op_label);
    let expected_word = 0x111 ^ 0x222 ^ 0x333 ^ 11;
    assert_eq!(res.reply_word, expected_word);
    assert_eq!(res.reply_words[1], expected_word);
    let reply_info = MessageInfo::from_raw(res.reply_words[0]).expect("valid reply info");
    assert_eq!(reply_info.label(), fs_op_label);
    assert_eq!(reply_info.registers(), 1);

    // One-way send (handler returns None) -> should_reply = false.
    let oneway = dispatch_message(&got, |_| None);
    assert!(!oneway.should_reply);

    // Malformed MessageInfo (reserved bit set) -> E_BADMSG reply, handler not invoked.
    let bad = Received {
        badge,
        words: [1u64 << 20, 0, 0, 0],
    };
    let mut called = false;
    let bad_res = dispatch_message(&bad, |_| {
        called = true;
        Some(0)
    });
    assert!(!called);
    assert!(bad_res.should_reply);
    assert_eq!(bad_res.reply_word, E_BADMSG);
    assert_eq!(bad_res.reply_words[1], E_BADMSG);
}

#[test]
fn lazy_zero_byte_startup_and_static_footprint() {
    // Boot-time static slot `Option<PageCache>` must stay tiny (no large static arrays).
    assert!(
        core::mem::size_of::<Option<PageCache>>() <= 256,
        "Option<PageCache> static footprint too large: {} bytes",
        core::mem::size_of::<Option<PageCache>>()
    );
    assert!(
        core::mem::size_of::<ReadAheadTracker>() <= 512,
        "ReadAheadTracker static footprint too large: {} bytes",
        core::mem::size_of::<ReadAheadTracker>()
    );

    // Empty / uninitialized cache owns 0 heap bytes and acts as a clean pass-through.
    let mut cache = PageCache::empty();
    assert_eq!(cache.capacity(), 0);
    assert_eq!(cache.heap_bytes(), 0);

    let mut tracker = ReadAheadTracker::new();
    let mut staging = vec![0u8; 8 * BLOCK_SIZE];
    let mut out = vec![0u8; 2 * BLOCK_SIZE];
    let mut calls = Vec::new();
    let outcome = read_cached_run(
        &mut cache,
        &mut tracker,
        100,
        10,
        &mut out,
        &mut staging,
        true,
        4,
        |first, n, buf| {
            calls.push((first, n));
            for k in 0..n {
                buf[k * BLOCK_SIZE..(k + 1) * BLOCK_SIZE]
                    .copy_from_slice(&pattern_block(first + k as u64, 2));
            }
            Ok(())
        },
    )
    .expect("pass-through read ok");

    // Zero-capacity cache must not issue speculative read-ahead and must keep 0 heap bytes.
    assert_eq!(calls, vec![(10, 2)]);
    assert_eq!(outcome.device_calls, 1);
    assert_eq!(outcome.demand_device_bytes, 2 * BLOCK_SIZE);
    assert_eq!(cache.heap_bytes(), 0);
}

#[test]
fn tight_heap_and_grown_arena_budget_capping() {
    // 1. Any memory pressure -> 0 blocks, 0 heap bytes.
    let under_pressure = HeapBudget {
        heap_total_bytes: 32 * 1024 * 1024,
        heap_free_bytes: 16 * 1024 * 1024,
        arena_grown: true,
        pressure: 1,
    };
    assert_eq!(budget_for_heap(under_pressure), 0);
    let c_press = PageCache::new_for_budget(under_pressure);
    assert_eq!(c_press.capacity(), 0);
    assert_eq!(c_press.heap_bytes(), 0);

    // 2. Ungrown 128 KiB fixed heap with < 64 KiB free -> 0 blocks (never starve Files).
    let fixed_low_free = HeapBudget {
        heap_total_bytes: FIXED_HEAP_BYTES,
        heap_free_bytes: TIGHT_HEAP_MIN_FREE - 4096,
        arena_grown: false,
        pressure: 0,
    };
    assert_eq!(budget_for_heap(fixed_low_free), 0);
    let c_low = PageCache::new_for_budget(fixed_low_free);
    assert_eq!(c_low.capacity(), 0);
    assert_eq!(c_low.heap_bytes(), 0);

    // 3. Ungrown 128 KiB fixed heap with ample free space -> strictly capped at <= 16 KiB (4 blocks).
    let fixed_ok_free = HeapBudget {
        heap_total_bytes: FIXED_HEAP_BYTES,
        heap_free_bytes: 96 * 1024,
        arena_grown: false,
        pressure: 0,
    };
    assert_eq!(budget_for_heap(fixed_ok_free), TIGHT_HEAP_MAX_BLOCKS);
    let c_tight = PageCache::new_for_budget(fixed_ok_free);
    assert_eq!(c_tight.capacity(), TIGHT_HEAP_MAX_BLOCKS);
    assert!(
        c_tight.heap_bytes() <= 20 * 1024,
        "tight-heap cache must stay tiny (got {} bytes)",
        c_tight.heap_bytes()
    );

    // 4. Grown service arena (< 16 MiB, e.g. QEMU after place()) -> 64 blocks (256 KiB > 128 KiB,
    //    so data allocates from the grown arena rather than eating the 128 KiB fixed heap).
    let grown_small = HeapBudget {
        heap_total_bytes: 5 * 1024 * 1024,
        heap_free_bytes: 256 * 1024,
        arena_grown: true,
        pressure: 0,
    };
    assert_eq!(budget_for_heap(grown_small), GROWN_HEAP_BASE_CAPACITY);
    let c_base = PageCache::new_for_budget(grown_small);
    assert_eq!(c_base.capacity(), GROWN_HEAP_BASE_CAPACITY);
    assert!(c_base.capacity() * BLOCK_SIZE > FIXED_HEAP_BYTES);

    // 5. Larger grown arenas scale cleanly up to 256 and 512 blocks.
    let grown_mid = HeapBudget {
        heap_total_bytes: 16 * 1024 * 1024,
        heap_free_bytes: 2 * 1024 * 1024,
        arena_grown: true,
        pressure: 0,
    };
    assert_eq!(budget_for_heap(grown_mid), GROWN_HEAP_MID_CAPACITY);

    let grown_high = HeapBudget {
        heap_total_bytes: 64 * 1024 * 1024,
        heap_free_bytes: 8 * 1024 * 1024,
        arena_grown: true,
        pressure: 0,
    };
    assert_eq!(budget_for_heap(grown_high), GROWN_HEAP_HIGH_CAPACITY);
}

