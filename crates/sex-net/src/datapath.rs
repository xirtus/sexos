// SPDX-License-Identifier: GPL-3.0-only
//! Zero-copy send/receive datapath primitives and per-core RSS lanes:
//! - Lock-free SPSC descriptor ring (`SpscRing<T, N>`)
//! - Zero-copy buffer descriptor handles (`BufferDesc`), slab pool (`ZeroCopyPool`),
//!   and scatter-gather slice views (`SgSlice`)
//! - 5-tuple Toeplitz & FNV-1a RSS hashing (`FlowTuple`, `toeplitz_hash`, `fnv1a_hash`)
//! - `PerCoreLanes` router distributing RX/TX descriptors across per-core rings
//!   without copying 1514-byte Ethernet frames.
//!
//! Strictly `#![no_std]` with `alloc`, integer-only (no FPU/SSE).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::MAX_FRAME;

/// Maximum scatter-gather segments per zero-copy packet view.
pub const MAX_SG_SEGMENTS: usize = 4;

/// Maximum supported CPU core lanes.
pub const MAX_CORES: usize = 16;

/// Default power-of-two capacity of each per-core SPSC descriptor ring (usable = 63).
pub const LANE_RING_CAPACITY: usize = 64;

/// Standard symmetric 40-byte Toeplitz RSS key (ensures both directions of a
/// TCP/UDP flow hash to the exact same core lane when symmetric hashing is used).
pub const SYMMETRIC_TOEPLITZ_KEY: [u8; 40] = [
    0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a,
    0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a,
    0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a,
    0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a, 0x6d, 0x5a,
];

/// Standard Microsoft RSS 40-byte Toeplitz key.
pub const STANDARD_TOEPLITZ_KEY: [u8; 40] = [
    0x6d, 0x5a, 0x56, 0xda, 0x25, 0x5b, 0x0e, 0xc2, 0x41, 0x67,
    0x25, 0x3d, 0x43, 0xa3, 0x8f, 0xb0, 0xd0, 0xca, 0x2b, 0xcb,
    0xae, 0x7b, 0x30, 0xb4, 0x77, 0xcb, 0x2d, 0xa3, 0x80, 0x30,
    0xf2, 0x0c, 0x6a, 0x42, 0xb7, 0x3b, 0xbe, 0xac, 0x01, 0xfa,
];

// ============================================================================
// Zero-Copy Buffer Descriptor & Scatter-Gather Views
// ============================================================================

/// Compact 16-byte handle referencing a frame slice inside a [`ZeroCopyPool`].
/// Passing descriptors across per-core rings avoids copying 1514-byte frames.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BufferDesc {
    /// Pool identifier (allows multiple NUMA / per-NIC pools).
    pub pool_id: u16,
    /// Slot index within the pool.
    pub buf_idx: u16,
    /// Byte offset to the start of valid packet data within the slot.
    pub offset: u16,
    /// Length of valid packet data in bytes.
    pub len: u16,
    /// Descriptor metadata flags (e.g., checksum offload, ECN, VLAN).
    pub flags: u16,
    /// Reserved alignment padding.
    pub reserved: u16,
    /// Computed RSS flow hash for core affinity.
    pub rss_hash: u32,
}

impl BufferDesc {
    pub const fn new(pool_id: u16, buf_idx: u16, offset: u16, len: u16) -> Self {
        Self {
            pool_id,
            buf_idx,
            offset,
            len,
            flags: 0,
            reserved: 0,
            rss_hash: 0,
        }
    }
}

/// Zero-copy scatter-gather slice view over up to [`MAX_SG_SEGMENTS`] disjoint byte slices
/// (e.g., Ethernet header + IP header + TCP header + payload).
#[derive(Clone, Copy, Debug)]
pub struct SgSlice<'a> {
    segments: [&'a [u8]; MAX_SG_SEGMENTS],
    count: usize,
    total_len: usize,
}

impl<'a> SgSlice<'a> {
    pub const fn new() -> Self {
        Self {
            segments: [&[]; MAX_SG_SEGMENTS],
            count: 0,
            total_len: 0,
        }
    }

    /// Append a zero-copy slice segment. Returns `false` if full.
    pub fn push(&mut self, seg: &'a [u8]) -> bool {
        if self.count >= MAX_SG_SEGMENTS {
            return false;
        }
        if seg.is_empty() {
            return true;
        }
        self.segments[self.count] = seg;
        self.count += 1;
        self.total_len = self.total_len.saturating_add(seg.len());
        true
    }

    pub const fn segment_count(&self) -> usize {
        self.count
    }

    pub const fn total_len(&self) -> usize {
        self.total_len
    }

    pub fn segments(&self) -> &[&'a [u8]] {
        &self.segments[..self.count]
    }

    /// Read a single byte at logical offset `index` across scatter-gather segments.
    pub fn read_byte(&self, mut index: usize) -> Option<u8> {
        for seg in self.segments() {
            if index < seg.len() {
                return Some(seg[index]);
            }
            index -= seg.len();
        }
        None
    }

    /// Linearize into a destination buffer (e.g., for legacy single-buffer NIC TX).
    /// Returns total bytes written, or `None` if `dst` is too small.
    pub fn copy_into(&self, dst: &mut [u8]) -> Option<usize> {
        if dst.len() < self.total_len {
            return None;
        }
        let mut off = 0;
        for seg in self.segments() {
            dst[off..off + seg.len()].copy_from_slice(seg);
            off += seg.len();
        }
        Some(off)
    }
}

impl Default for SgSlice<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Pre-allocated fixed-slab zero-copy frame buffer pool.
pub struct ZeroCopyPool {
    pool_id: u16,
    slabs: Box<[[u8; MAX_FRAME]]>,
    free_stack: Vec<u16>,
    allocated: Vec<bool>,
}

impl ZeroCopyPool {
    /// Allocate a bounded pool of `capacity` frame slabs (each [`MAX_FRAME`] bytes).
    pub fn new(pool_id: u16, capacity: usize) -> Self {
        let cap = capacity.clamp(1, u16::MAX as usize);
        let mut slabs = Vec::with_capacity(cap);
        slabs.resize(cap, [0u8; MAX_FRAME]);
        let mut free_stack = Vec::with_capacity(cap);
        for i in (0..cap).rev() {
            free_stack.push(i as u16);
        }
        let mut allocated = Vec::with_capacity(cap);
        allocated.resize(cap, false);
        Self {
            pool_id,
            slabs: slabs.into_boxed_slice(),
            free_stack,
            allocated,
        }
    }

    pub fn pool_id(&self) -> u16 {
        self.pool_id
    }

    pub fn capacity(&self) -> usize {
        self.slabs.len()
    }

    pub fn available(&self) -> usize {
        self.free_stack.len()
    }

    pub fn in_use(&self) -> usize {
        self.slabs.len() - self.free_stack.len()
    }

    /// Allocate a slab index from the pool without heap allocation.
    pub fn alloc(&mut self) -> Option<u16> {
        let idx = self.free_stack.pop()?;
        self.allocated[idx as usize] = true;
        Some(idx)
    }

    /// Return a slab index to the pool. Ignores out-of-range or double-free indices safely.
    pub fn free(&mut self, idx: u16) -> bool {
        let i = idx as usize;
        if i >= self.allocated.len() || !self.allocated[i] {
            return false;
        }
        self.allocated[i] = false;
        self.free_stack.push(idx);
        true
    }

    /// Mutable access to an allocated slab for zero-copy NIC DMA / header assembly.
    pub fn slab_mut(&mut self, idx: u16) -> Option<&mut [u8; MAX_FRAME]> {
        let i = idx as usize;
        if i < self.allocated.len() && self.allocated[i] {
            Some(&mut self.slabs[i])
        } else {
            None
        }
    }

    /// Immutable zero-copy slice view described by `desc`.
    pub fn slice(&self, desc: BufferDesc) -> Option<&[u8]> {
        if desc.pool_id != self.pool_id {
            return None;
        }
        let i = desc.buf_idx as usize;
        if i >= self.allocated.len() || !self.allocated[i] {
            return None;
        }
        let start = desc.offset as usize;
        let end = start.checked_add(desc.len as usize)?;
        self.slabs[i].get(start..end)
    }

    /// Populate a fresh slab from a scatter-gather view and return its [`BufferDesc`].
    pub fn alloc_from_sg(&mut self, sg: &SgSlice<'_>) -> Option<BufferDesc> {
        if sg.total_len() > MAX_FRAME {
            return None;
        }
        let idx = self.alloc()?;
        let slab = self.slab_mut(idx)?;
        let len = sg.copy_into(slab)?;
        Some(BufferDesc::new(self.pool_id, idx, 0, len as u16))
    }
}

// ============================================================================
// Lock-Free SPSC Descriptor Ring
// ============================================================================

/// Lock-free single-producer single-consumer (SPSC) ring buffer.
///
/// `N` MUST be a power of two >= 2. Usable capacity is `N - 1`.
/// Safe for concurrent access by one producer thread/core and one consumer thread/core
/// using `Acquire`/`Release` atomic ordering.
pub struct SpscRing<T: Copy + Default, const N: usize> {
    head: AtomicUsize,
    tail: AtomicUsize,
    buf: UnsafeCell<[T; N]>,
}

// SAFETY: SPSC protocol guarantees only the producer writes slots at `tail` and updates `tail`
// with Release, while only the consumer reads slots at `head` and updates `head` with Release.
unsafe impl<T: Send + Copy + Default, const N: usize> Sync for SpscRing<T, N> {}
unsafe impl<T: Send + Copy + Default, const N: usize> Send for SpscRing<T, N> {}

impl<T: Copy + Default, const N: usize> SpscRing<T, N> {
    const _VALID_N: () = assert!(N >= 2 && N.is_power_of_two(), "SpscRing N must be power of 2 >= 2");

    pub fn new() -> Self {
        let _ = Self::_VALID_N;
        Self {
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            buf: UnsafeCell::new([T::default(); N]),
        }
    }

    #[inline]
    pub const fn capacity(&self) -> usize {
        N - 1
    }

    #[inline]
    pub fn len(&self) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Acquire);
        tail.wrapping_sub(head) & (N - 1)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire) == self.tail.load(Ordering::Acquire)
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let next_tail = (tail + 1) & (N - 1);
        next_tail == self.head.load(Ordering::Acquire)
    }

    /// Producer: push a single item without locking. Returns `Err(item)` if full.
    #[inline]
    pub fn try_push(&self, item: T) -> Result<(), T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let next_tail = (tail + 1) & (N - 1);
        if next_tail == self.head.load(Ordering::Acquire) {
            return Err(item);
        }
        // SAFETY: Producer exclusively owns slot `tail` until `tail` is published via Release.
        unsafe {
            (*self.buf.get())[tail] = item;
        }
        self.tail.store(next_tail, Ordering::Release);
        Ok(())
    }

    /// Consumer: pop a single item without locking. Returns `None` if empty.
    #[inline]
    pub fn try_pop(&self) -> Option<T> {
        let head = self.head.load(Ordering::Relaxed);
        if head == self.tail.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: Consumer exclusively reads slot `head` after observing `tail` via Acquire.
        let item = unsafe { (*self.buf.get())[head] };
        let next_head = (head + 1) & (N - 1);
        self.head.store(next_head, Ordering::Release);
        Some(item)
    }

    /// Producer: push up to `items.len()` items in a batch. Returns count pushed.
    pub fn push_batch(&self, items: &[T]) -> usize {
        let mut pushed = 0;
        for &item in items {
            if self.try_push(item).is_err() {
                break;
            }
            pushed += 1;
        }
        pushed
    }

    /// Consumer: pop up to `out.len()` items into `out`. Returns count popped.
    pub fn pop_batch(&self, out: &mut [T]) -> usize {
        let mut popped = 0;
        for slot in out.iter_mut() {
            match self.try_pop() {
                Some(item) => {
                    *slot = item;
                    popped += 1;
                }
                None => break,
            }
        }
        popped
    }
}

impl<T: Copy + Default, const N: usize> Default for SpscRing<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// 5-Tuple RSS Hashing (Toeplitz & FNV-1a)
// ============================================================================

/// RSS hash algorithm selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RssHashAlgo {
    /// Standard Toeplitz hash (Microsoft RSS / NIC hardware compatible).
    Toeplitz,
    /// Fast FNV-1a hash over the 5-tuple.
    Fnv1a,
}

/// Extracted L3/L4 5-tuple for RSS core routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlowTuple {
    pub src_ip: [u8; 16],
    pub dst_ip: [u8; 16],
    pub src_port: u16,
    pub dst_port: u16,
    pub proto: u8,
    pub is_v6: bool,
}

impl FlowTuple {
    pub const fn v4(src: [u8; 4], dst: [u8; 4], src_port: u16, dst_port: u16, proto: u8) -> Self {
        Self {
            src_ip: [src[0], src[1], src[2], src[3], 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            dst_ip: [dst[0], dst[1], dst[2], dst[3], 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            src_port,
            dst_port,
            proto,
            is_v6: false,
        }
    }

    pub const fn v6(src: [u8; 16], dst: [u8; 16], src_port: u16, dst_port: u16, proto: u8) -> Self {
        Self {
            src_ip: src,
            dst_ip: dst,
            src_port,
            dst_port,
            proto,
            is_v6: true,
        }
    }

    /// Return the reverse direction of this 5-tuple (swap src/dst IP and port).
    pub const fn reversed(&self) -> Self {
        Self {
            src_ip: self.dst_ip,
            dst_ip: self.src_ip,
            src_port: self.dst_port,
            dst_port: self.src_port,
            proto: self.proto,
            is_v6: self.is_v6,
        }
    }

    /// Zero-copy extraction of an IPv4/IPv6 TCP/UDP 5-tuple from a raw Ethernet frame.
    pub fn from_ethernet_frame(frame: &[u8]) -> Option<Self> {
        if frame.len() < 14 {
            return None;
        }
        let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
        match ethertype {
            0x0800 => {
                // IPv4
                if frame.len() < 14 + 20 || (frame[14] >> 4) != 4 {
                    return None;
                }
                let ihl = ((frame[14] & 0x0f) as usize) * 4;
                if ihl < 20 || frame.len() < 14 + ihl {
                    return None;
                }
                let proto = frame[23];
                let src = [frame[26], frame[27], frame[28], frame[29]];
                let dst = [frame[30], frame[31], frame[32], frame[33]];
                let (src_port, dst_port) = if (proto == 6 || proto == 17) && frame.len() >= 14 + ihl + 4 {
                    let l4 = 14 + ihl;
                    (
                        u16::from_be_bytes([frame[l4], frame[l4 + 1]]),
                        u16::from_be_bytes([frame[l4 + 2], frame[l4 + 3]]),
                    )
                } else {
                    (0, 0)
                };
                Some(Self::v4(src, dst, src_port, dst_port, proto))
            }
            0x86dd => {
                // IPv6
                if frame.len() < 14 + 40 || (frame[14] >> 4) != 6 {
                    return None;
                }
                let proto = frame[20];
                let mut src = [0u8; 16];
                let mut dst = [0u8; 16];
                src.copy_from_slice(&frame[22..38]);
                dst.copy_from_slice(&frame[38..54]);
                let (src_port, dst_port) = if (proto == 6 || proto == 17) && frame.len() >= 14 + 40 + 4 {
                    let l4 = 14 + 40;
                    (
                        u16::from_be_bytes([frame[l4], frame[l4 + 1]]),
                        u16::from_be_bytes([frame[l4 + 2], frame[l4 + 3]]),
                    )
                } else {
                    (0, 0)
                };
                Some(Self::v6(src, dst, src_port, dst_port, proto))
            }
            _ => None,
        }
    }
}

/// Compute standard 32-bit Toeplitz hash over `input` (up to 36 bytes) using a 40-byte key.
pub fn toeplitz_hash(key: &[u8; 40], input: &[u8]) -> u32 {
    let mut result: u32 = 0;
    let mut k = u32::from_be_bytes([key[0], key[1], key[2], key[3]]);
    let mut next_byte_idx = 4usize;

    for &byte in input {
        for bit in (0..8).rev() {
            if ((byte >> bit) & 1) != 0 {
                result ^= k;
            }
            let next_bit = if next_byte_idx < key.len() {
                ((key[next_byte_idx] >> bit) & 1) as u32
            } else {
                0
            };
            k = (k << 1) | next_bit;
        }
        next_byte_idx += 1;
    }
    result
}

/// Compute Toeplitz RSS hash for a [`FlowTuple`].
pub fn rss_toeplitz(key: &[u8; 40], tuple: &FlowTuple) -> u32 {
    if tuple.is_v6 {
        let mut input = [0u8; 36];
        input[0..16].copy_from_slice(&tuple.src_ip);
        input[16..32].copy_from_slice(&tuple.dst_ip);
        input[32..34].copy_from_slice(&tuple.src_port.to_be_bytes());
        input[34..36].copy_from_slice(&tuple.dst_port.to_be_bytes());
        toeplitz_hash(key, &input)
    } else {
        let mut input = [0u8; 12];
        input[0..4].copy_from_slice(&tuple.src_ip[0..4]);
        input[4..8].copy_from_slice(&tuple.dst_ip[0..4]);
        input[8..10].copy_from_slice(&tuple.src_port.to_be_bytes());
        input[10..12].copy_from_slice(&tuple.dst_port.to_be_bytes());
        toeplitz_hash(key, &input)
    }
}

/// Compute fast 32-bit FNV-1a RSS hash for a [`FlowTuple`].
pub fn rss_fnv1a(tuple: &FlowTuple) -> u32 {
    const FNV_BASIS: u32 = 0x811c_9dc5;
    const FNV_PRIME: u32 = 0x0100_0193;
    let mut h = FNV_BASIS;
    let ip_len = if tuple.is_v6 { 16 } else { 4 };
    for &b in &tuple.src_ip[..ip_len] {
        h ^= b as u32;
        h = h.wrapping_mul(FNV_PRIME);
    }
    for &b in &tuple.dst_ip[..ip_len] {
        h ^= b as u32;
        h = h.wrapping_mul(FNV_PRIME);
    }
    for b in tuple.src_port.to_be_bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(FNV_PRIME);
    }
    for b in tuple.dst_port.to_be_bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h ^= tuple.proto as u32;
    h = h.wrapping_mul(FNV_PRIME);
    h
}

// ============================================================================
// Per-Core Zero-Copy Lanes Router
// ============================================================================

/// Snapshot of per-core lane telemetry counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaneStats {
    pub rx_enqueued: u64,
    pub rx_dequeued: u64,
    pub rx_dropped: u64,
    pub tx_enqueued: u64,
    pub tx_dequeued: u64,
    pub tx_dropped: u64,
}

/// Single CPU core's lock-free RX and TX zero-copy descriptor lanes.
pub struct CoreLane {
    rx: SpscRing<BufferDesc, LANE_RING_CAPACITY>,
    tx: SpscRing<BufferDesc, LANE_RING_CAPACITY>,
    rx_enqueued: AtomicU64,
    rx_dequeued: AtomicU64,
    rx_dropped: AtomicU64,
    tx_enqueued: AtomicU64,
    tx_dequeued: AtomicU64,
    tx_dropped: AtomicU64,
}

impl CoreLane {
    pub fn new() -> Self {
        Self {
            rx: SpscRing::new(),
            tx: SpscRing::new(),
            rx_enqueued: AtomicU64::new(0),
            rx_dequeued: AtomicU64::new(0),
            rx_dropped: AtomicU64::new(0),
            tx_enqueued: AtomicU64::new(0),
            tx_dequeued: AtomicU64::new(0),
            tx_dropped: AtomicU64::new(0),
        }
    }

    pub fn stats(&self) -> LaneStats {
        LaneStats {
            rx_enqueued: self.rx_enqueued.load(Ordering::Relaxed),
            rx_dequeued: self.rx_dequeued.load(Ordering::Relaxed),
            rx_dropped: self.rx_dropped.load(Ordering::Relaxed),
            tx_enqueued: self.tx_enqueued.load(Ordering::Relaxed),
            tx_dequeued: self.tx_dequeued.load(Ordering::Relaxed),
            tx_dropped: self.tx_dropped.load(Ordering::Relaxed),
        }
    }
}

impl Default for CoreLane {
    fn default() -> Self {
        Self::new()
    }
}

/// Multi-core zero-copy RX/TX lane router using 5-tuple RSS hashing.
pub struct PerCoreLanes {
    lanes: Box<[CoreLane]>,
    algo: RssHashAlgo,
    key: [u8; 40],
}

impl PerCoreLanes {
    /// Create a router with `num_cores` lanes (clamped to `1..=MAX_CORES`).
    pub fn new(num_cores: usize, algo: RssHashAlgo) -> Self {
        let count = num_cores.clamp(1, MAX_CORES);
        let mut lanes = Vec::with_capacity(count);
        for _ in 0..count {
            lanes.push(CoreLane::new());
        }
        Self {
            lanes: lanes.into_boxed_slice(),
            algo,
            key: SYMMETRIC_TOEPLITZ_KEY,
        }
    }

    /// Override the 40-byte Toeplitz RSS key.
    pub fn set_toeplitz_key(&mut self, key: [u8; 40]) {
        self.key = key;
    }

    pub fn num_cores(&self) -> usize {
        self.lanes.len()
    }

    /// Compute the RSS hash for `tuple`.
    pub fn hash_tuple(&self, tuple: &FlowTuple) -> u32 {
        match self.algo {
            RssHashAlgo::Toeplitz => rss_toeplitz(&self.key, tuple),
            RssHashAlgo::Fnv1a => rss_fnv1a(tuple),
        }
    }

    /// Map a 5-tuple to its target core lane index `0..num_cores`.
    pub fn route_core(&self, tuple: &FlowTuple) -> (usize, u32) {
        let h = self.hash_tuple(tuple);
        ((h as usize) % self.lanes.len(), h)
    }

    /// Enqueue a zero-copy RX buffer descriptor to the RSS-selected core lane.
    /// Non-IP / non-tuple frames (e.g., ARP) default to core 0.
    /// Returns `Ok(core_id)` on success, or `Err(desc)` if that core's RX ring is full.
    pub fn enqueue_rx(
        &self,
        mut desc: BufferDesc,
        tuple: Option<&FlowTuple>,
    ) -> Result<usize, BufferDesc> {
        let (core, hash) = match tuple {
            Some(t) => self.route_core(t),
            None => (0, 0),
        };
        desc.rss_hash = hash;
        let lane = &self.lanes[core];
        match lane.rx.try_push(desc) {
            Ok(()) => {
                lane.rx_enqueued.fetch_add(1, Ordering::Relaxed);
                Ok(core)
            }
            Err(d) => {
                lane.rx_dropped.fetch_add(1, Ordering::Relaxed);
                Err(d)
            }
        }
    }

    /// Dequeue up to `out.len()` zero-copy RX descriptors for `core`.
    pub fn dequeue_rx_batch(&self, core: usize, out: &mut [BufferDesc]) -> usize {
        let Some(lane) = self.lanes.get(core) else {
            return 0;
        };
        let n = lane.rx.pop_batch(out);
        if n > 0 {
            lane.rx_dequeued.fetch_add(n as u64, Ordering::Relaxed);
        }
        n
    }

    /// Enqueue a zero-copy TX buffer descriptor onto `core`'s TX ring.
    pub fn enqueue_tx(&self, core: usize, desc: BufferDesc) -> Result<(), BufferDesc> {
        let Some(lane) = self.lanes.get(core) else {
            return Err(desc);
        };
        match lane.tx.try_push(desc) {
            Ok(()) => {
                lane.tx_enqueued.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(d) => {
                lane.tx_dropped.fetch_add(1, Ordering::Relaxed);
                Err(d)
            }
        }
    }

    /// Dequeue up to `out.len()` zero-copy TX descriptors from `core`'s TX ring.
    pub fn dequeue_tx_batch(&self, core: usize, out: &mut [BufferDesc]) -> usize {
        let Some(lane) = self.lanes.get(core) else {
            return 0;
        };
        let n = lane.tx.pop_batch(out);
        if n > 0 {
            lane.tx_dequeued.fetch_add(n as u64, Ordering::Relaxed);
        }
        n
    }

    /// Query telemetry counters for `core`.
    pub fn stats(&self, core: usize) -> Option<LaneStats> {
        self.lanes.get(core).map(CoreLane::stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spsc_ring_fifo_wraparound_and_backpressure() {
        let ring: SpscRing<u32, 8> = SpscRing::new();
        assert_eq!(ring.capacity(), 7);
        assert!(ring.is_empty());

        for i in 0..7u32 {
            assert_eq!(ring.try_push(i + 10), Ok(()));
        }
        assert!(ring.is_full());
        assert_eq!(ring.try_push(99), Err(99));

        // Pop 4, push 4 -> exercises ring wraparound.
        let mut out = [0u32; 4];
        assert_eq!(ring.pop_batch(&mut out), 4);
        assert_eq!(out, [10, 11, 12, 13]);

        assert_eq!(ring.push_batch(&[20, 21, 22, 23, 24]), 4);
        assert!(ring.is_full());

        let mut all = [0u32; 8];
        assert_eq!(ring.pop_batch(&mut all), 7);
        assert_eq!(&all[..7], &[14, 15, 16, 20, 21, 22, 23]);
        assert!(ring.is_empty());
    }

    #[test]
    fn zero_copy_pool_and_scatter_gather_roundtrip() {
        let mut pool = ZeroCopyPool::new(1, 4);
        assert_eq!(pool.capacity(), 4);
        assert_eq!(pool.available(), 4);

        let hdr = [0xaa; 14];
        let ip = [0x45; 20];
        let payload = b"zero-copy-payload";
        let mut sg = SgSlice::new();
        assert!(sg.push(&hdr));
        assert!(sg.push(&ip));
        assert!(sg.push(payload));
        assert_eq!(sg.total_len(), 14 + 20 + payload.len());
        assert_eq!(sg.read_byte(0), Some(0xaa));
        assert_eq!(sg.read_byte(14), Some(0x45));
        assert_eq!(sg.read_byte(34), Some(b'z'));

        let desc = pool.alloc_from_sg(&sg).unwrap();
        assert_eq!(pool.in_use(), 1);
        let slice = pool.slice(desc).unwrap();
        assert_eq!(&slice[34..], payload);

        // Wrong pool_id fails closed.
        let mut bad_desc = desc;
        bad_desc.pool_id = 99;
        assert!(pool.slice(bad_desc).is_none());

        assert!(pool.free(desc.buf_idx));
        assert!(!pool.free(desc.buf_idx)); // double-free rejected safely
        assert_eq!(pool.available(), 4);
    }

    #[test]
    fn symmetric_toeplitz_hashes_bidirectional_flows_to_same_core_and_distributes() {
        let lanes = PerCoreLanes::new(4, RssHashAlgo::Toeplitz);
        let fwd = FlowTuple::v4([10, 0, 2, 15], [93, 184, 216, 34], 49152, 443, 6);
        let rev = fwd.reversed();
        assert_eq!(lanes.route_core(&fwd), lanes.route_core(&rev));

        // Distribute 64 distinct flows across 4 cores -> every core should receive traffic.
        let mut counts = [0usize; 4];
        for port in 1000..1064u16 {
            let t = FlowTuple::v4([10, 0, 2, 15], [1, 1, 1, 1], port, 443, 6);
            let (core, _) = lanes.route_core(&t);
            counts[core] += 1;
        }
        assert!(counts.iter().all(|&c| c > 0), "RSS should spread flows across all cores: {counts:?}");
    }

    #[test]
    fn per_core_lanes_zero_copy_rx_tx_routing_and_frame_extraction() {
        let mut pool = ZeroCopyPool::new(0, 8);
        let lanes = PerCoreLanes::new(4, RssHashAlgo::Fnv1a);

        // Build a minimal Ethernet + IPv4 + TCP frame header in a zero-copy slab.
        let idx = pool.alloc().unwrap();
        let slab = pool.slab_mut(idx).unwrap();
        slab[12..14].copy_from_slice(&[0x08, 0x00]); // IPv4
        slab[14] = 0x45; // v4, IHL=5
        slab[23] = 6; // TCP
        slab[26..30].copy_from_slice(&[10, 0, 2, 15]);
        slab[30..34].copy_from_slice(&[10, 0, 2, 2]);
        slab[34..36].copy_from_slice(&40001u16.to_be_bytes());
        slab[36..38].copy_from_slice(&80u16.to_be_bytes());
        let desc = BufferDesc::new(0, idx, 0, 54);

        let tuple = FlowTuple::from_ethernet_frame(pool.slice(desc).unwrap()).unwrap();
        assert_eq!(tuple.src_port, 40001);
        assert_eq!(tuple.dst_port, 80);
        assert_eq!(tuple.proto, 6);

        let core = lanes.enqueue_rx(desc, Some(&tuple)).unwrap();
        let mut rx_batch = [BufferDesc::default(); 4];
        assert_eq!(lanes.dequeue_rx_batch(core, &mut rx_batch), 1);
        assert_eq!(rx_batch[0].buf_idx, idx);
        assert_ne!(rx_batch[0].rss_hash, 0);

        // Roundtrip descriptor through core's TX ring.
        lanes.enqueue_tx(core, rx_batch[0]).unwrap();
        let mut tx_batch = [BufferDesc::default(); 4];
        assert_eq!(lanes.dequeue_tx_batch(core, &mut tx_batch), 1);
        assert_eq!(tx_batch[0].buf_idx, idx);

        let st = lanes.stats(core).unwrap();
        assert_eq!(st.rx_enqueued, 1);
        assert_eq!(st.rx_dequeued, 1);
        assert_eq!(st.tx_enqueued, 1);
        assert_eq!(st.tx_dequeued, 1);
        assert!(pool.free(idx));
    }
}
