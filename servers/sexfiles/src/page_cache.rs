// SPDX-License-Identifier: GPL-3.0-only
//! Fixed-budget, `no_std`+`alloc` file data page cache and multi-stream adaptive
//! read-ahead tracker for `sexfiles`.
//!
//! - O(1) lookup, insert, and eviction via open-addressing hash table with
//!   backward-shift deletion (no tombstones, no rehashing).
//! - CLOCK (second-chance) eviction; speculatively prefetched blocks enter with
//!   `referenced = false` until consumed by a demand read so unused read-ahead
//!   yields before hot demand-read pages.
//! - Multi-stream adaptive read-ahead tracks up to [`MAX_STREAMS`] concurrent
//!   sequential readers and grows each stream's window (`4 -> 8 -> 16 -> 32`
//!   blocks) independently, populating the page cache directly instead of
//!   relying on a single volatile span in the shared DMA buffer.

use alloc::vec::Vec;

/// Filesystem block size (4 KiB, matching `sexfs::BLOCK`).
pub const BLOCK_SIZE: usize = 4096;

/// Default page cache budget: 1024 blocks = 4 MiB.
#[allow(dead_code)]
pub const DEFAULT_CAPACITY: usize = 1024;

/// Graceful fallback capacities when `sexfiles`'s heap cannot satisfy 4 MiB.
#[allow(dead_code)]
pub const FALLBACK_CAPACITIES: [usize; 4] = [1024, 256, 64, 16];

/// Fixed ELF heap threshold (`sex-rt` `HEAP_BYTES = 128 KiB`).
pub const FIXED_HEAP_BYTES: usize = 128 * 1024;

/// Maximum cache blocks when `sexfiles` is on its fixed 128 KiB ELF heap (16 KiB max).
pub const TIGHT_HEAP_MAX_BLOCKS: usize = 4;

/// Minimum free heap bytes required before allocating even a tiny cache in a fixed 128 KiB heap.
pub const TIGHT_HEAP_MIN_FREE: usize = 64 * 1024;

/// Base cache capacity when `sexfiles`'s service arena grew (64 blocks = 256 KiB).
/// Exceeds `FIXED_HEAP_BYTES` so `data` allocates from the grown service arena rather than
/// eating the 128 KiB fixed ELF heap, while staying small enough for tight QEMU gates.
pub const GROWN_HEAP_BASE_CAPACITY: usize = 64;

/// Medium cache capacity when grown heap already exceeds 16 MiB (256 blocks = 1 MiB).
pub const GROWN_HEAP_MID_CAPACITY: usize = 256;

/// High cache capacity when grown heap already exceeds 64 MiB (512 blocks = 2 MiB).
pub const GROWN_HEAP_HIGH_CAPACITY: usize = 512;

/// Heap snapshot used to size the lazy page cache without starving `sexfiles`'s 128 KiB
/// fixed ELF heap or tight QEMU `program_frames`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapBudget {
    /// Total heap bytes currently owned by `sexfiles` (fixed + grown arena).
    pub heap_total_bytes: usize,
    /// Free heap bytes currently available without growing.
    pub heap_free_bytes: usize,
    /// Whether `sexfiles`'s service arena successfully grew during mount (`place()`).
    pub arena_grown: bool,
    /// Machine memory pressure (`0` = normal, `> 0` = low/critical).
    pub pressure: u64,
}

/// Computes a safe page-cache capacity (in 4 KiB blocks) for the given heap snapshot.
///
/// - Under any memory pressure (`pressure > 0`), returns `0` (pass-through, 0 heap bytes).
/// - On an ungrown 128 KiB fixed heap, returns `0` if free space is below `TIGHT_HEAP_MIN_FREE`,
///   otherwise caps at `min(free / 4 / BLOCK_SIZE, TIGHT_HEAP_MAX_BLOCKS)` (at most 16 KiB).
/// - When the service arena grew, scales `64 -> 256 -> 512` blocks (256 KiB -> 1 MiB -> 2 MiB)
///   so `data` always lives in the grown arena and never eats the 128 KiB fixed heap.
pub const fn budget_for_heap(budget: HeapBudget) -> usize {
    if budget.pressure > 0 {
        return 0;
    }
    if !budget.arena_grown || budget.heap_total_bytes <= FIXED_HEAP_BYTES {
        if budget.heap_free_bytes < TIGHT_HEAP_MIN_FREE {
            return 0;
        }
        let quarter_blocks = (budget.heap_free_bytes / 4) / BLOCK_SIZE;
        if quarter_blocks > TIGHT_HEAP_MAX_BLOCKS {
            TIGHT_HEAP_MAX_BLOCKS
        } else {
            quarter_blocks
        }
    } else if budget.heap_total_bytes >= 64 * 1024 * 1024 {
        GROWN_HEAP_HIGH_CAPACITY
    } else if budget.heap_total_bytes >= 16 * 1024 * 1024 {
        GROWN_HEAP_MID_CAPACITY
    } else {
        GROWN_HEAP_BASE_CAPACITY
    }
}

/// Concurrent sequential streams tracked for adaptive read-ahead.
pub const MAX_STREAMS: usize = 8;

/// Initial adaptive read-ahead window (blocks) once a sequential stream is detected.
pub const INITIAL_AHEAD_WINDOW: usize = 4;

/// Maximum adaptive read-ahead window (blocks = 128 KiB).
pub const MAX_AHEAD_WINDOW: usize = 32;

/// Small forward gap (in blocks) still recognized as part of the same sequential stream.
const STREAM_GAP_TOLERANCE: u64 = 2;

const EMPTY_SLOT: u32 = u32::MAX;

/// Diagnostic counters for the page cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub readahead_hits: u64,
    pub evictions: u64,
    pub writes_updated: u64,
    pub invalidations: u64,
    pub capacity: usize,
    pub used: usize,
}

#[derive(Clone, Copy)]
struct CacheEntry {
    block: u64,
    valid: bool,
    referenced: bool,
    prefetched: bool,
}

impl CacheEntry {
    const EMPTY: Self = Self {
        block: 0,
        valid: false,
        referenced: false,
        prefetched: false,
    };
}

/// Fixed-budget file data page cache indexed by block LBA.
pub struct PageCache {
    data: Vec<[u8; BLOCK_SIZE]>,
    entries: Vec<CacheEntry>,
    table: Vec<u32>,
    table_mask: usize,
    cap: usize,
    used: usize,
    clock_hand: usize,
    stats: CacheStats,
}

impl PageCache {
    /// Creates an empty pass-through cache (capacity 0, no heap allocation).
    pub const fn empty() -> Self {
        Self {
            data: Vec::new(),
            entries: Vec::new(),
            table: Vec::new(),
            table_mask: 0,
            cap: 0,
            used: 0,
            clock_hand: 0,
            stats: CacheStats {
                hits: 0,
                misses: 0,
                readahead_hits: 0,
                evictions: 0,
                writes_updated: 0,
                invalidations: 0,
                capacity: 0,
                used: 0,
            },
        }
    }

    /// Fallibly allocates a page cache holding `capacity` blocks.
    /// Returns `None` without panicking if memory is insufficient.
    pub fn try_new(capacity: usize) -> Option<Self> {
        if capacity == 0 {
            return Some(Self::empty());
        }
        // Keep hash load factor <= 0.5 for O(1) probing.
        let table_len = capacity.checked_mul(2)?.checked_next_power_of_two()?;

        let mut data: Vec<[u8; BLOCK_SIZE]> = Vec::new();
        data.try_reserve_exact(capacity).ok()?;
        data.resize(capacity, [0u8; BLOCK_SIZE]);

        let mut entries: Vec<CacheEntry> = Vec::new();
        entries.try_reserve_exact(capacity).ok()?;
        entries.resize(capacity, CacheEntry::EMPTY);

        let mut table: Vec<u32> = Vec::new();
        table.try_reserve_exact(table_len).ok()?;
        table.resize(table_len, EMPTY_SLOT);

        Some(Self {
            data,
            entries,
            table,
            table_mask: table_len - 1,
            cap: capacity,
            used: 0,
            clock_hand: 0,
            stats: CacheStats {
                capacity,
                ..CacheStats::default()
            },
        })
    }

    /// Allocates `preferred` blocks, falling back to smaller budgets on tight memory.
    #[allow(dead_code)]
    pub fn new_with_fallback(preferred: usize) -> Self {
        if let Some(cache) = Self::try_new(preferred) {
            return cache;
        }
        for &tier in &FALLBACK_CAPACITIES {
            if tier < preferred {
                if let Some(cache) = Self::try_new(tier) {
                    return cache;
                }
            }
        }
        Self::empty()
    }

    /// Allocates a page cache sized safely to `budget`.
    ///
    /// Never exceeds `budget_for_heap(budget)` and never falls back to a mid-sized tier
    /// (such as 16 blocks = 64 KiB) that would consume half of `sexfiles`'s 128 KiB fixed heap.
    pub fn new_for_budget(budget: HeapBudget) -> Self {
        let target = budget_for_heap(budget);
        if target == 0 {
            return Self::empty();
        }
        if let Some(cache) = Self::try_new(target) {
            return cache;
        }
        if budget.arena_grown {
            for &tier in &[
                GROWN_HEAP_MID_CAPACITY,
                GROWN_HEAP_BASE_CAPACITY,
            ] {
                if tier < target {
                    if let Some(cache) = Self::try_new(tier) {
                        return cache;
                    }
                }
            }
        }
        Self::empty()
    }

    /// Approximate heap bytes currently owned by this cache (`0` for `PageCache::empty()`).
    #[inline]
    #[allow(dead_code)]
    pub fn heap_bytes(&self) -> usize {
        self.data.capacity() * BLOCK_SIZE
            + self.entries.capacity() * core::mem::size_of::<CacheEntry>()
            + self.table.capacity() * core::mem::size_of::<u32>()
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.cap
    }

    #[inline]
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.used
    }

    #[inline]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    #[inline]
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            capacity: self.cap,
            used: self.used,
            ..self.stats
        }
    }

    #[inline]
    fn hash_index(&self, block: u64) -> usize {
        // Fibonacci / splitmix64 finalizer step for uniform distribution over sequential LBAs.
        let mut h = block.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 30;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 27;
        (h as usize) & self.table_mask
    }

    fn find_slot(&self, block: u64) -> Option<usize> {
        if self.cap == 0 {
            return None;
        }
        let mut idx = self.hash_index(block);
        loop {
            let slot = self.table[idx];
            if slot == EMPTY_SLOT {
                return None;
            }
            let s = slot as usize;
            if self.entries[s].valid && self.entries[s].block == block {
                return Some(s);
            }
            idx = (idx + 1) & self.table_mask;
        }
    }

    fn table_insert(&mut self, block: u64, slot: u32) {
        let mut idx = self.hash_index(block);
        while self.table[idx] != EMPTY_SLOT {
            idx = (idx + 1) & self.table_mask;
        }
        self.table[idx] = slot;
    }

    /// Removes `block` from the open-addressing hash table using backward-shift deletion.
    fn table_remove(&mut self, block: u64) {
        let mut i = self.hash_index(block);
        loop {
            let slot = self.table[i];
            if slot == EMPTY_SLOT {
                return;
            }
            if self.entries[slot as usize].block == block {
                break;
            }
            i = (i + 1) & self.table_mask;
        }
        self.table[i] = EMPTY_SLOT;
        let mut j = (i + 1) & self.table_mask;
        while self.table[j] != EMPTY_SLOT {
            let moved_slot = self.table[j];
            let natural = self.hash_index(self.entries[moved_slot as usize].block);
            let dist_from_natural = j.wrapping_sub(natural) & self.table_mask;
            let dist_from_hole = j.wrapping_sub(i) & self.table_mask;
            if dist_from_natural >= dist_from_hole {
                self.table[i] = moved_slot;
                self.table[j] = EMPTY_SLOT;
                i = j;
            }
            j = (j + 1) & self.table_mask;
        }
    }

    /// Checks whether `block` is resident without updating stats or CLOCK bits.
    #[inline]
    pub fn contains(&self, block: u64) -> bool {
        self.find_slot(block).is_some()
    }

    #[inline]
    pub fn record_miss(&mut self) {
        self.stats.misses = self.stats.misses.saturating_add(1);
    }

    /// Looks up `block`. On hit, copies into `out`, marks the page referenced,
    /// and returns `Some(was_prefetched)`. On miss, increments `misses` and returns `None`.
    pub fn get(&mut self, block: u64, out: &mut [u8; BLOCK_SIZE]) -> Option<bool> {
        let Some(slot) = self.find_slot(block) else {
            self.stats.misses = self.stats.misses.saturating_add(1);
            return None;
        };
        let entry = &mut self.entries[slot];
        let was_prefetched = entry.prefetched;
        entry.referenced = true;
        entry.prefetched = false;
        self.stats.hits = self.stats.hits.saturating_add(1);
        if was_prefetched {
            self.stats.readahead_hits = self.stats.readahead_hits.saturating_add(1);
        }
        out.copy_from_slice(&self.data[slot]);
        Some(was_prefetched)
    }

    fn alloc_slot(&mut self) -> usize {
        loop {
            let slot = self.clock_hand;
            self.clock_hand = (slot + 1) % self.cap;
            if !self.entries[slot].valid {
                return slot;
            }
            if self.entries[slot].referenced {
                self.entries[slot].referenced = false;
            } else {
                let old_block = self.entries[slot].block;
                self.table_remove(old_block);
                self.entries[slot] = CacheEntry::EMPTY;
                self.used -= 1;
                self.stats.evictions = self.stats.evictions.saturating_add(1);
                return slot;
            }
        }
    }

    /// Inserts `block` into the cache.
    ///
    /// When `prefetched == true`, an already-cached block is left untouched so
    /// speculative read-ahead never demotes hot pages or resets their state.
    pub fn insert(&mut self, block: u64, src: &[u8; BLOCK_SIZE], prefetched: bool) {
        if self.cap == 0 {
            return;
        }
        if let Some(slot) = self.find_slot(block) {
            if !prefetched {
                self.data[slot].copy_from_slice(src);
                self.entries[slot].referenced = true;
                self.entries[slot].prefetched = false;
            }
            return;
        }
        let slot = self.alloc_slot();
        self.data[slot].copy_from_slice(src);
        self.entries[slot] = CacheEntry {
            block,
            valid: true,
            referenced: !prefetched,
            prefetched,
        };
        self.table_insert(block, slot as u32);
        self.used += 1;
    }

    /// Write-through update: updates cached blocks (and populates the cache)
    /// for a contiguous write starting at `first`.
    #[allow(dead_code)]
    pub fn write_through(&mut self, first: u64, data: &[u8]) {
        if self.cap == 0 || data.len() % BLOCK_SIZE != 0 {
            return;
        }
        let blocks = data.len() / BLOCK_SIZE;
        for i in 0..blocks {
            let chunk: &[u8; BLOCK_SIZE] = data[i * BLOCK_SIZE..(i + 1) * BLOCK_SIZE]
                .try_into()
                .expect("chunk is BLOCK_SIZE");
            self.insert(first + i as u64, chunk, false);
            self.stats.writes_updated = self.stats.writes_updated.saturating_add(1);
        }
    }

    /// Invalidates a single block if present. Returns `true` if it was evicted.
    pub fn invalidate_block(&mut self, block: u64) -> bool {
        let Some(slot) = self.find_slot(block) else {
            return false;
        };
        self.table_remove(block);
        self.entries[slot] = CacheEntry::EMPTY;
        self.used -= 1;
        self.stats.invalidations = self.stats.invalidations.saturating_add(1);
        true
    }

    /// Invalidates `[first, first + count)` from the cache.
    pub fn invalidate_range(&mut self, first: u64, count: u64) {
        if self.cap == 0 || self.used == 0 || count == 0 {
            return;
        }
        // If invalidating more blocks than currently resident, scanning entries is faster.
        if count >= self.cap as u64 {
            let end = first.saturating_add(count);
            for slot in 0..self.cap {
                if self.entries[slot].valid {
                    let b = self.entries[slot].block;
                    if b >= first && b < end {
                        self.table_remove(b);
                        self.entries[slot] = CacheEntry::EMPTY;
                        self.used -= 1;
                        self.stats.invalidations = self.stats.invalidations.saturating_add(1);
                    }
                }
            }
            return;
        }
        for i in 0..count {
            let _ = self.invalidate_block(first.saturating_add(i));
        }
    }

    /// Drops all cached blocks cleanly (e.g. on remount or device reset).
    pub fn clear(&mut self) {
        if self.cap == 0 {
            return;
        }
        for e in self.entries.iter_mut() {
            *e = CacheEntry::EMPTY;
        }
        for t in self.table.iter_mut() {
            *t = EMPTY_SLOT;
        }
        self.used = 0;
        self.clock_hand = 0;
    }
}

/// One tracked sequential read stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamState {
    pub next_block: u64,
    pub window: usize,
    pub streak: u32,
    pub last_used: u64,
    pub active: bool,
}

impl StreamState {
    const EMPTY: Self = Self {
        next_block: 0,
        window: 0,
        streak: 0,
        last_used: 0,
        active: false,
    };
}

/// Multi-stream adaptive read-ahead tracker (up to [`MAX_STREAMS`] concurrent streams).
pub struct ReadAheadTracker {
    streams: [StreamState; MAX_STREAMS],
    tick: u64,
}

impl ReadAheadTracker {
    pub const fn new() -> Self {
        Self {
            streams: [StreamState::EMPTY; MAX_STREAMS],
            tick: 0,
        }
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.streams = [StreamState::EMPTY; MAX_STREAMS];
        self.tick = 0;
    }

    #[allow(dead_code)]
    pub fn active_streams(&self) -> usize {
        self.streams.iter().filter(|s| s.active).count()
    }

    #[allow(dead_code)]
    pub fn streams(&self) -> &[StreamState; MAX_STREAMS] {
        &self.streams
    }

    fn find_matching(&self, first: u64) -> Option<usize> {
        // Prefer exact sequential continuation over small forward gap.
        let mut gap_match = None;
        for (i, s) in self.streams.iter().enumerate() {
            if !s.active {
                continue;
            }
            if s.next_block == first {
                return Some(i);
            }
            if first > s.next_block && first <= s.next_block.saturating_add(STREAM_GAP_TOLERANCE) {
                gap_match = Some(i);
            }
        }
        gap_match
    }

    fn alloc_stream(&self) -> usize {
        if let Some(i) = self.streams.iter().position(|s| !s.active) {
            return i;
        }
        // Evict least-recently-used stream.
        let mut victim = 0;
        let mut oldest = self.streams[0].last_used;
        for i in 1..MAX_STREAMS {
            if self.streams[i].last_used < oldest {
                oldest = self.streams[i].last_used;
                victim = i;
            }
        }
        victim
    }

    /// Observes a read that required device I/O and returns the adaptive number of
    /// additional read-ahead blocks to fetch in the same batched device command.
    pub fn observe_miss(
        &mut self,
        first: u64,
        count: u64,
        hint_ahead: usize,
        max_ahead: usize,
    ) -> usize {
        self.tick = self.tick.saturating_add(1);
        let next = first.saturating_add(count);

        if let Some(idx) = self.find_matching(first) {
            let s = &mut self.streams[idx];
            s.streak = s.streak.saturating_add(1);
            s.window = if s.window == 0 {
                INITIAL_AHEAD_WINDOW
            } else {
                (s.window.saturating_mul(2)).min(MAX_AHEAD_WINDOW)
            };
            s.next_block = next;
            s.last_used = self.tick;
            return s.window.max(hint_ahead).min(max_ahead);
        }

        let idx = self.alloc_stream();
        let initial_window = if hint_ahead > 0 { INITIAL_AHEAD_WINDOW } else { 0 };
        self.streams[idx] = StreamState {
            next_block: next,
            window: initial_window,
            streak: 0,
            last_used: self.tick,
            active: true,
        };
        if hint_ahead > 0 {
            initial_window.max(hint_ahead).min(max_ahead)
        } else {
            0
        }
    }

    /// Observes a read that was fully satisfied from the page cache, advancing the
    /// matching stream's expected next LBA without prematurely doubling its window.
    pub fn observe_hit(&mut self, first: u64, count: u64) {
        self.tick = self.tick.saturating_add(1);
        let next = first.saturating_add(count);
        if let Some(idx) = self.find_matching(first) {
            let s = &mut self.streams[idx];
            s.next_block = next;
            s.last_used = self.tick;
        }
    }
}

/// Accounting summary returned by [`read_cached_run`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadOutcome {
    /// Number of device read commands issued.
    pub device_calls: u64,
    /// Demand payload bytes fetched from the device (excludes speculative ahead bytes).
    pub demand_device_bytes: usize,
    /// Bytes satisfied from previously read-ahead (prefetched) cache pages.
    pub readahead_hit_bytes: usize,
    /// Bytes satisfied from already-warm demand-cached pages.
    pub cache_hit_bytes: usize,
}

/// Reads `[first, first + out.len() / BLOCK_SIZE)` using `cache` and `tracker`,
/// issuing `read_device` only for contiguous missing ranges (and batching adaptive
/// read-ahead onto the tail miss when `enable_readahead` is true).
pub fn read_cached_run<F>(
    cache: &mut PageCache,
    tracker: &mut ReadAheadTracker,
    disk_blocks: u64,
    first: u64,
    out: &mut [u8],
    staging: &mut [u8],
    enable_readahead: bool,
    hint_ahead: usize,
    mut read_device: F,
) -> Result<ReadOutcome, ()>
where
    F: FnMut(u64, usize, &mut [u8]) -> Result<(), ()>,
{
    if out.is_empty() || out.len() % BLOCK_SIZE != 0 || staging.len() < BLOCK_SIZE {
        return Err(());
    }
    let count = out.len() / BLOCK_SIZE;
    let max_run_blocks = staging.len() / BLOCK_SIZE;
    let mut outcome = ReadOutcome::default();

    let mut i = 0usize;
    while i < count {
        let block = first.saturating_add(i as u64);
        let dst: &mut [u8; BLOCK_SIZE] = (&mut out[i * BLOCK_SIZE..(i + 1) * BLOCK_SIZE])
            .try_into()
            .map_err(|_| ())?;
        if let Some(was_prefetched) = cache.get(block, dst) {
            if was_prefetched {
                outcome.readahead_hit_bytes += BLOCK_SIZE;
            } else {
                outcome.cache_hit_bytes += BLOCK_SIZE;
            }
            i += 1;
            continue;
        }

        // Contiguous missing range starting at `i`, bounded by `max_run_blocks`.
        let start = i;
        let mut end = i + 1;
        while end < count && (end - start) < max_run_blocks {
            if cache.contains(first.saturating_add(end as u64)) {
                break;
            }
            cache.record_miss();
            end += 1;
        }

        let run_blocks = end - start;
        let run_first = first.saturating_add(start as u64);

        // Attach adaptive read-ahead to the trailing miss when enabled and cache is active.
        let extra_ahead = if enable_readahead && end == count && cache.capacity() > 0 {
            let room_in_buf = max_run_blocks.saturating_sub(run_blocks);
            let room_on_disk = disk_blocks
                .saturating_sub(first.saturating_add(count as u64))
                .min(usize::MAX as u64) as usize;
            let room_in_cache = (cache.capacity() / 2).max(1);
            let max_ahead = room_in_buf.min(room_on_disk).min(room_in_cache);
            tracker.observe_miss(first, count as u64, hint_ahead, max_ahead)
        } else {
            0
        };

        let fetch_blocks = run_blocks + extra_ahead;
        let fetch_bytes = fetch_blocks * BLOCK_SIZE;
        read_device(run_first, fetch_blocks, &mut staging[..fetch_bytes])?;

        outcome.device_calls += 1;
        outcome.demand_device_bytes += run_blocks * BLOCK_SIZE;

        // Copy demand blocks to caller and populate cache.
        let demand_bytes = run_blocks * BLOCK_SIZE;
        out[start * BLOCK_SIZE..end * BLOCK_SIZE].copy_from_slice(&staging[..demand_bytes]);
        for k in 0..run_blocks {
            let chunk: &[u8; BLOCK_SIZE] = staging[k * BLOCK_SIZE..(k + 1) * BLOCK_SIZE]
                .try_into()
                .map_err(|_| ())?;
            cache.insert(run_first.saturating_add(k as u64), chunk, false);
        }

        // Populate read-ahead blocks into the page cache directly.
        for k in 0..extra_ahead {
            let off = (run_blocks + k) * BLOCK_SIZE;
            let chunk: &[u8; BLOCK_SIZE] = staging[off..off + BLOCK_SIZE]
                .try_into()
                .map_err(|_| ())?;
            cache.insert(
                run_first.saturating_add((run_blocks + k) as u64),
                chunk,
                true,
            );
        }

        i = end;
    }

    if outcome.device_calls == 0 && enable_readahead {
        tracker.observe_hit(first, count as u64);
    }

    Ok(outcome)
}
