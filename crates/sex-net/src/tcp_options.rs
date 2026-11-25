// SPDX-License-Identifier: GPL-3.0-only
//! TCP transport option codecs and state machines:
//! - RFC 7323 Window Scaling (shift 0..=14, negotiation, scaled window tracking)
//! - RFC 2018 Selective Acknowledgment (SACK-Permitted, SACK blocks codec,
//!   bounded out-of-order receiver scoreboard with coalescing)
//! - RFC 6675 Sender-side SACK loss detection and fast recovery tracking
//!
//! Strictly `#![no_std]` and integer-only (no FPU/SSE).

/// TCP option kind constants (IANA / RFC 793 / RFC 2018 / RFC 7323).
pub const OPT_EOL: u8 = 0;
pub const OPT_NOP: u8 = 1;
pub const OPT_MSS: u8 = 2;
pub const OPT_WINDOW_SCALE: u8 = 3;
pub const OPT_SACK_PERMITTED: u8 = 4;
pub const OPT_SACK: u8 = 5;

/// RFC 7323 Section 2.3: maximum legal window scale shift count is 14.
pub const MAX_WINDOW_SCALE: u8 = 14;

/// RFC 2018 Section 3: a TCP header options area (max 40 bytes) holds at most
/// 4 SACK blocks (2 + 4*8 = 34 bytes).
pub const MAX_SACK_BLOCKS: usize = 4;

/// Bounded capacity of the receiver's out-of-order SACK scoreboard.
pub const SCOREBOARD_CAPACITY: usize = 16;

/// Bounded capacity of the sender's tracked in-flight segments for SACK recovery.
pub const SENDER_SCOREBOARD_CAPACITY: usize = 64;

/// RFC 6675 default duplicate ACK / SACK threshold for loss detection.
pub const DEFAULT_DUP_THRESH: u32 = 3;

/// Modular 32-bit TCP sequence comparison: `a < b` in half-space.
#[inline]
pub const fn seq_lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

/// Modular 32-bit TCP sequence comparison: `a <= b` in half-space.
#[inline]
pub const fn seq_le(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) <= 0
}

/// Modular 32-bit TCP sequence comparison: `a > b` in half-space.
#[inline]
pub const fn seq_gt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

/// Modular 32-bit TCP sequence comparison: `a >= b` in half-space.
#[inline]
pub const fn seq_ge(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) >= 0
}

/// Signed distance `a - b` in modular 32-bit TCP sequence space.
#[inline]
pub const fn seq_diff(a: u32, b: u32) -> i32 {
    a.wrapping_sub(b) as i32
}

// ============================================================================
// RFC 7323 TCP Window Scaling
// ============================================================================

/// Per-connection RFC 7323 Window Scale negotiation and scaled window tracker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowScaleState {
    /// True only when both endpoints included a Window Scale option in SYN/SYN-ACK.
    pub enabled: bool,
    /// Local receive shift (0..=14) advertised in our SYN/SYN-ACK.
    pub rcv_shift: u8,
    /// Peer's receive shift (0..=14) learned from the peer's SYN/SYN-ACK.
    pub snd_shift: u8,
    /// Most recent scaled remote receive window in bytes.
    pub snd_wnd: u32,
    /// Most recent local receive buffer space in bytes.
    pub rcv_wnd: u32,
}

impl WindowScaleState {
    /// Compute the smallest shift `0..=14` capable of representing `buffer_bytes`
    /// inside a 16-bit TCP window field (`65535 << shift >= buffer_bytes`).
    pub const fn compute_shift(buffer_bytes: usize) -> u8 {
        let mut shift: u8 = 0;
        while shift < MAX_WINDOW_SCALE {
            let max_wnd = (u16::MAX as usize) << (shift as usize);
            if max_wnd >= buffer_bytes {
                break;
            }
            shift += 1;
        }
        shift
    }

    /// Create window scaling state sized for a local receive buffer of `buffer_bytes`.
    pub const fn for_receive_buffer(buffer_bytes: usize) -> Self {
        let rcv_shift = Self::compute_shift(buffer_bytes);
        let rcv_wnd = if buffer_bytes > u32::MAX as usize {
            u32::MAX
        } else {
            buffer_bytes as u32
        };
        Self {
            enabled: false,
            rcv_shift,
            snd_shift: 0,
            snd_wnd: 0,
            rcv_wnd,
        }
    }

    /// Create window scaling state with an explicit local shift (clamped to 14 per RFC 7323 §2.3).
    pub const fn with_shift(rcv_shift: u8) -> Self {
        let clamped = if rcv_shift > MAX_WINDOW_SCALE {
            MAX_WINDOW_SCALE
        } else {
            rcv_shift
        };
        Self {
            enabled: false,
            rcv_shift: clamped,
            snd_shift: 0,
            snd_wnd: 0,
            rcv_wnd: (u16::MAX as u32) << (clamped as u32),
        }
    }

    /// Complete negotiation upon receiving the peer's SYN or SYN-ACK.
    /// Per RFC 7323 §2.2, window scaling is enabled only if both sides sent the option.
    pub fn negotiate_peer(&mut self, peer_shift: Option<u8>) {
        match peer_shift {
            Some(s) => {
                self.enabled = true;
                self.snd_shift = s.min(MAX_WINDOW_SCALE);
            }
            None => {
                self.enabled = false;
                self.rcv_shift = 0;
                self.snd_shift = 0;
            }
        }
    }

    /// Compute the 16-bit wire window field to advertise for `free_bytes`.
    /// Per RFC 7323 §2.2, the window field in SYN and SYN-ACK segments is NEVER scaled.
    pub fn advertise_window(&mut self, free_bytes: u32, is_syn: bool) -> u16 {
        self.rcv_wnd = free_bytes;
        if is_syn || !self.enabled {
            free_bytes.min(u16::MAX as u32) as u16
        } else {
            (free_bytes >> (self.rcv_shift as u32)).min(u16::MAX as u32) as u16
        }
    }

    /// Update and return the peer's scaled receive window (in bytes) from a segment's
    /// 16-bit wire window field. Per RFC 7323 §2.2, SYN/SYN-ACK windows are unscaled.
    pub fn update_peer_window(&mut self, raw_window: u16, is_syn: bool) -> u32 {
        let scaled = if is_syn || !self.enabled {
            raw_window as u32
        } else {
            (raw_window as u32) << (self.snd_shift as u32)
        };
        self.snd_wnd = scaled;
        scaled
    }

    /// Maximum representable local receive window in bytes given current state.
    pub const fn max_local_window(&self) -> u32 {
        let shift = if self.enabled { self.rcv_shift } else { 0 };
        (u16::MAX as u32) << (shift as u32)
    }

    /// Encode the 3-byte TCP Window Scale option `[3, 3, rcv_shift]` into `out`.
    pub fn encode_option(&self, out: &mut [u8]) -> Option<usize> {
        if out.len() < 3 {
            return None;
        }
        out[0] = OPT_WINDOW_SCALE;
        out[1] = 3;
        out[2] = self.rcv_shift.min(MAX_WINDOW_SCALE);
        Some(3)
    }

    /// Decode a 3-byte TCP Window Scale option. Values > 14 are clamped to 14 per RFC 7323 §2.3.
    pub fn decode_option(bytes: &[u8]) -> Option<u8> {
        if bytes.len() < 3 || bytes[0] != OPT_WINDOW_SCALE || bytes[1] != 3 {
            return None;
        }
        Some(bytes[2].min(MAX_WINDOW_SCALE))
    }
}

// ============================================================================
// RFC 2018 TCP SACK Codec & Options Parser
// ============================================================================

/// A half-open TCP sequence range `[left, right)` where `seq_lt(left, right)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SackBlock {
    pub left: u32,
    pub right: u32,
}

impl SackBlock {
    /// Construct a valid SACK block, returning `None` if `left >= right` in modular space.
    pub const fn new(left: u32, right: u32) -> Option<Self> {
        if seq_lt(left, right) {
            Some(Self { left, right })
        } else {
            None
        }
    }

    /// Length of the block in bytes.
    #[inline]
    pub const fn len(&self) -> u32 {
        self.right.wrapping_sub(self.left)
    }

    /// Whether the block contains sequence number `seq`.
    #[inline]
    pub const fn contains(&self, seq: u32) -> bool {
        seq_ge(seq, self.left) && seq_lt(seq, self.right)
    }

    /// Whether `self` and `other` overlap or are directly adjacent (can be coalesced).
    #[inline]
    pub const fn overlaps_or_adjacent(&self, other: &Self) -> bool {
        seq_le(self.left, other.right) && seq_le(other.left, self.right)
    }

    /// Merge two overlapping or adjacent blocks.
    #[inline]
    pub const fn merge(&self, other: &Self) -> Self {
        let left = if seq_lt(self.left, other.left) {
            self.left
        } else {
            other.left
        };
        let right = if seq_gt(self.right, other.right) {
            self.right
        } else {
            other.right
        };
        Self { left, right }
    }
}

/// Encode the 2-byte SACK-Permitted option `[4, 2]`.
pub fn encode_sack_permitted(out: &mut [u8]) -> Option<usize> {
    if out.len() < 2 {
        return None;
    }
    out[0] = OPT_SACK_PERMITTED;
    out[1] = 2;
    Some(2)
}

/// Encode up to 4 SACK blocks as a TCP SACK option `[5, 2 + 8*n, ...]`.
/// Returns `Some(0)` if `blocks` is empty, `None` if `out` is too small.
pub fn encode_sack_blocks(blocks: &[SackBlock], out: &mut [u8]) -> Option<usize> {
    let count = blocks.len().min(MAX_SACK_BLOCKS);
    if count == 0 {
        return Some(0);
    }
    let total_len = 2 + count * 8;
    if out.len() < total_len {
        return None;
    }
    out[0] = OPT_SACK;
    out[1] = total_len as u8;
    let mut off = 2;
    for b in &blocks[..count] {
        out[off..off + 4].copy_from_slice(&b.left.to_be_bytes());
        out[off + 4..off + 8].copy_from_slice(&b.right.to_be_bytes());
        off += 8;
    }
    Some(total_len)
}

/// Decoded TCP header options relevant to modern transport negotiation and SACK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedTcpOptions {
    pub mss: Option<u16>,
    pub window_scale: Option<u8>,
    pub sack_permitted: bool,
    pub sack_blocks: [Option<SackBlock>; MAX_SACK_BLOCKS],
    pub sack_count: usize,
}

impl ParsedTcpOptions {
    pub const EMPTY: Self = Self {
        mss: None,
        window_scale: None,
        sack_permitted: false,
        sack_blocks: [None; MAX_SACK_BLOCKS],
        sack_count: 0,
    };

    /// Return a slice view of the parsed SACK blocks into `buf`.
    pub fn copy_sack_blocks(&self, buf: &mut [SackBlock; MAX_SACK_BLOCKS]) -> usize {
        let mut n = 0;
        for b in self.sack_blocks.iter().flatten() {
            if n < MAX_SACK_BLOCKS {
                buf[n] = *b;
                n += 1;
            }
        }
        n
    }
}

/// Parse a TCP options byte slice safely without panicking on malformed lengths.
pub fn parse_tcp_options(opts: &[u8]) -> ParsedTcpOptions {
    let mut parsed = ParsedTcpOptions::EMPTY;
    let mut i = 0;
    while i < opts.len() {
        let kind = opts[i];
        match kind {
            OPT_EOL => break,
            OPT_NOP => {
                i += 1;
            }
            _ => {
                if i + 1 >= opts.len() {
                    break;
                }
                let len = opts[i + 1] as usize;
                if len < 2 || i + len > opts.len() {
                    break;
                }
                match kind {
                    OPT_MSS if len == 4 => {
                        parsed.mss = Some(u16::from_be_bytes([opts[i + 2], opts[i + 3]]));
                    }
                    OPT_WINDOW_SCALE if len == 3 => {
                        parsed.window_scale = Some(opts[i + 2].min(MAX_WINDOW_SCALE));
                    }
                    OPT_SACK_PERMITTED if len == 2 => {
                        parsed.sack_permitted = true;
                    }
                    OPT_SACK if len >= 10 && (len - 2) % 8 == 0 => {
                        let num = ((len - 2) / 8).min(MAX_SACK_BLOCKS);
                        let mut off = i + 2;
                        parsed.sack_count = 0;
                        parsed.sack_blocks = [None; MAX_SACK_BLOCKS];
                        for _ in 0..num {
                            let left = u32::from_be_bytes([
                                opts[off],
                                opts[off + 1],
                                opts[off + 2],
                                opts[off + 3],
                            ]);
                            let right = u32::from_be_bytes([
                                opts[off + 4],
                                opts[off + 5],
                                opts[off + 6],
                                opts[off + 7],
                            ]);
                            if let Some(block) = SackBlock::new(left, right) {
                                parsed.sack_blocks[parsed.sack_count] = Some(block);
                                parsed.sack_count += 1;
                            }
                            off += 8;
                        }
                    }
                    _ => {}
                }
                i += len;
            }
        }
    }
    parsed
}

// ============================================================================
// RFC 2018 Receiver-Side Bounded Out-of-Order SACK Scoreboard
// ============================================================================

/// Bounded out-of-order receive scoreboard.
///
/// Maintains disjoint coalesced [`SackBlock`] ranges ordered by recency
/// (index 0 is the block containing the most recently received out-of-order
/// segment, per RFC 2018 Section 4).
#[derive(Clone, Debug)]
pub struct SackScoreboard {
    blocks: [Option<SackBlock>; SCOREBOARD_CAPACITY],
    len: usize,
}

impl SackScoreboard {
    pub const fn new() -> Self {
        Self {
            blocks: [None; SCOREBOARD_CAPACITY],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.blocks = [None; SCOREBOARD_CAPACITY];
        self.len = 0;
    }

    /// Record an out-of-order segment `[seq, seq + seg_len)` when the receiver expects `rcv_nxt`.
    ///
    /// - Segments entirely below `rcv_nxt` or with `seg_len == 0` are ignored.
    /// - Overlapping or adjacent blocks are coalesced into a single range.
    /// - The resulting block is placed at index 0 (most recently received, RFC 2018 §4).
    /// - Returns `true` if the scoreboard was updated.
    pub fn insert(&mut self, rcv_nxt: u32, seq: u32, seg_len: u32) -> bool {
        if seg_len == 0 {
            return false;
        }
        let end = seq.wrapping_add(seg_len);
        if seq_le(end, rcv_nxt) {
            return false;
        }
        let clipped_left = if seq_lt(seq, rcv_nxt) { rcv_nxt } else { seq };
        let Some(mut merged) = SackBlock::new(clipped_left, end) else {
            return false;
        };

        // Coalesce with all overlapping/adjacent existing blocks and compact the remainder.
        let mut remaining = [None; SCOREBOARD_CAPACITY];
        let mut rem_len = 0usize;
        for i in 0..self.len {
            if let Some(existing) = self.blocks[i] {
                if merged.overlaps_or_adjacent(&existing) {
                    merged = merged.merge(&existing);
                } else {
                    remaining[rem_len] = Some(existing);
                    rem_len += 1;
                }
            }
        }

        // Place the newly merged block at index 0 (most recent), evicting the oldest if full.
        let keep = rem_len.min(SCOREBOARD_CAPACITY - 1);
        self.blocks = [None; SCOREBOARD_CAPACITY];
        self.blocks[0] = Some(merged);
        for i in 0..keep {
            self.blocks[i + 1] = remaining[i];
        }
        self.len = keep + 1;
        true
    }

    /// Advance the contiguous receive edge `rcv_nxt`.
    ///
    /// Any scoreboard blocks that now overlap or adjoin `rcv_nxt` are consumed and
    /// extend `rcv_nxt` forward. Returns the new contiguous `rcv_nxt`.
    pub fn advance_rcv_nxt(&mut self, mut rcv_nxt: u32) -> u32 {
        loop {
            let mut extended = false;
            let mut next_blocks = [None; SCOREBOARD_CAPACITY];
            let mut next_len = 0usize;
            for i in 0..self.len {
                if let Some(b) = self.blocks[i] {
                    if seq_le(b.right, rcv_nxt) {
                        // Completely covered by rcv_nxt; drop.
                        extended = true;
                    } else if seq_le(b.left, rcv_nxt) {
                        // Contiguous with rcv_nxt; consume and extend.
                        rcv_nxt = b.right;
                        extended = true;
                    } else {
                        next_blocks[next_len] = Some(b);
                        next_len += 1;
                    }
                }
            }
            self.blocks = next_blocks;
            self.len = next_len;
            if !extended {
                break;
            }
        }
        rcv_nxt
    }

    /// Fill `out` with up to 4 SACK blocks ordered by recency (RFC 2018 §4).
    pub fn blocks_for_ack(&self, out: &mut [SackBlock; MAX_SACK_BLOCKS]) -> usize {
        let mut n = 0;
        for i in 0..self.len.min(MAX_SACK_BLOCKS) {
            if let Some(b) = self.blocks[i] {
                out[n] = b;
                n += 1;
            }
        }
        n
    }

    /// Total out-of-order bytes currently tracked in the scoreboard.
    pub fn total_bytes(&self) -> u32 {
        let mut total = 0u32;
        for i in 0..self.len {
            if let Some(b) = self.blocks[i] {
                total = total.saturating_add(b.len());
            }
        }
        total
    }
}

impl Default for SackScoreboard {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// RFC 6675 Sender-Side SACK Loss Detection & Fast Recovery
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SentSegment {
    seq: u32,
    len: u32,
    sacked: bool,
    retransmitted: bool,
    lost: bool,
}

impl SentSegment {
    #[inline]
    const fn end(&self) -> u32 {
        self.seq.wrapping_add(self.len)
    }
}

/// Outcome of processing an incoming ACK + SACK blocks at the sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SackAckOutcome {
    /// Bytes newly cumulatively acknowledged (advancing `snd_una`).
    pub newly_acked_bytes: u32,
    /// Bytes newly marked as SACKed in this ACK.
    pub newly_sacked_bytes: u32,
    /// True if this ACK triggered entry into SACK Fast Recovery.
    pub enter_recovery: bool,
    /// True if this ACK exited SACK Fast Recovery (`ack >= recovery_point`).
    pub exit_recovery: bool,
    /// Number of segments currently marked lost and awaiting retransmission.
    pub lost_segments: usize,
    /// RFC 6675 `pipe`: estimated bytes currently in flight in the network.
    pub pipe: u32,
}

/// Sender-side SACK scoreboard implementing RFC 6675 loss detection and fast recovery.
#[derive(Clone, Debug)]
pub struct SackSenderTracker {
    pub snd_una: u32,
    pub snd_nxt: u32,
    pub recovery_point: u32,
    pub in_fast_recovery: bool,
    pub dup_acks: u32,
    pub dup_thresh: u32,
    pub mss: u32,
    segments: [Option<SentSegment>; SENDER_SCOREBOARD_CAPACITY],
    len: usize,
}

impl SackSenderTracker {
    pub const fn new(initial_seq: u32, mss: u32) -> Self {
        Self {
            snd_una: initial_seq,
            snd_nxt: initial_seq,
            recovery_point: initial_seq,
            in_fast_recovery: false,
            dup_acks: 0,
            dup_thresh: DEFAULT_DUP_THRESH,
            mss: if mss == 0 { 1460 } else { mss },
            segments: [None; SENDER_SCOREBOARD_CAPACITY],
            len: 0,
        }
    }

    /// Record a newly transmitted segment `[seq, seq + seg_len)`.
    /// Returns `false` if the bounded scoreboard is full or `seg_len == 0`.
    pub fn on_send(&mut self, seq: u32, seg_len: u32) -> bool {
        if seg_len == 0 || self.len >= SENDER_SCOREBOARD_CAPACITY {
            return false;
        }
        let end = seq.wrapping_add(seg_len);
        if seq_gt(end, self.snd_nxt) {
            self.snd_nxt = end;
        }
        let entry = SentSegment {
            seq,
            len: seg_len,
            sacked: false,
            retransmitted: false,
            lost: false,
        };
        // Keep segments ordered by sequence number for fast O(N) scanning.
        let mut pos = self.len;
        while pos > 0 {
            if let Some(prev) = self.segments[pos - 1] {
                if seq_lt(entry.seq, prev.seq) {
                    self.segments[pos] = self.segments[pos - 1];
                    pos -= 1;
                    continue;
                }
            }
            break;
        }
        self.segments[pos] = Some(entry);
        self.len += 1;
        true
    }

    /// Process an incoming cumulative `ack` and optional `sack_blocks` (RFC 6675).
    pub fn on_ack(&mut self, ack: u32, sack_blocks: &[SackBlock]) -> SackAckOutcome {
        let mut newly_acked_bytes = 0u32;
        let mut newly_sacked_bytes = 0u32;
        let mut enter_recovery = false;
        let mut exit_recovery = false;

        // 1. Advance cumulative ACK (`snd_una`) and prune fully ACKed segments.
        if seq_gt(ack, self.snd_una) && seq_le(ack, self.snd_nxt) {
            newly_acked_bytes = ack.wrapping_sub(self.snd_una);
            self.snd_una = ack;
            self.dup_acks = 0;

            let mut write = 0usize;
            for i in 0..self.len {
                if let Some(mut seg) = self.segments[i] {
                    if seq_le(seg.end(), ack) {
                        continue;
                    }
                    if seq_lt(seg.seq, ack) {
                        let trim = ack.wrapping_sub(seg.seq);
                        seg.seq = ack;
                        seg.len = seg.len.saturating_sub(trim);
                    }
                    self.segments[write] = Some(seg);
                    write += 1;
                }
            }
            for i in write..self.len {
                self.segments[i] = None;
            }
            self.len = write;
        } else if ack == self.snd_una && self.len > 0 {
            self.dup_acks = self.dup_acks.saturating_add(1);
        }

        // 2. Mark segments covered by SACK blocks.
        for block in sack_blocks {
            if !seq_lt(block.left, block.right) {
                continue;
            }
            for i in 0..self.len {
                if let Some(seg) = self.segments[i].as_mut() {
                    if !seg.sacked && seq_le(block.left, seg.seq) && seq_ge(block.right, seg.end())
                    {
                        seg.sacked = true;
                        seg.lost = false;
                        newly_sacked_bytes = newly_sacked_bytes.saturating_add(seg.len);
                    }
                }
            }
        }

        // 3. RFC 6675 IsLost(seq): an unsacked segment is lost if at least `dup_thresh`
        //    higher-sequence segments are SACKed, or at least `dup_thresh * mss` bytes
        //    with higher sequence numbers are SACKed.
        let mut lost_segments = 0usize;
        for i in 0..self.len {
            let Some(seg) = self.segments[i] else {
                continue;
            };
            if seg.sacked {
                continue;
            }
            let mut sacked_segs_above = 0u32;
            let mut sacked_bytes_above = 0u32;
            for j in (i + 1)..self.len {
                if let Some(higher) = self.segments[j] {
                    if higher.sacked {
                        sacked_segs_above += 1;
                        sacked_bytes_above = sacked_bytes_above.saturating_add(higher.len);
                    }
                }
            }
            let is_lost = sacked_segs_above >= self.dup_thresh
                || sacked_bytes_above >= self.dup_thresh.saturating_mul(self.mss);
            if is_lost {
                if let Some(s) = self.segments[i].as_mut() {
                    s.lost = true;
                }
            }
            if self.segments[i].is_some_and(|s| s.lost && !s.retransmitted) {
                lost_segments += 1;
            }
        }

        // 4. Enter or exit Fast Recovery.
        if self.in_fast_recovery {
            if seq_ge(self.snd_una, self.recovery_point) {
                self.in_fast_recovery = false;
                self.dup_acks = 0;
                exit_recovery = true;
            }
        } else if lost_segments > 0 || self.dup_acks >= self.dup_thresh {
            self.in_fast_recovery = true;
            self.recovery_point = self.snd_nxt;
            enter_recovery = true;
            // If triggered by dup_acks without SACK loss yet, mark the oldest unsacked segment lost.
            if lost_segments == 0 {
                for i in 0..self.len {
                    if let Some(s) = self.segments[i].as_mut() {
                        if !s.sacked {
                            s.lost = true;
                            lost_segments = 1;
                            break;
                        }
                    }
                }
            }
        }

        // 5. Compute RFC 6675 `pipe`: bytes still estimated in flight.
        let pipe = self.pipe();

        SackAckOutcome {
            newly_acked_bytes,
            newly_sacked_bytes,
            enter_recovery,
            exit_recovery,
            lost_segments,
            pipe,
        }
    }

    /// RFC 6675 `SetPipe`: estimated bytes currently in flight.
    pub fn pipe(&self) -> u32 {
        let mut pipe = 0u32;
        for i in 0..self.len {
            if let Some(seg) = self.segments[i] {
                if !seg.sacked && !seg.lost {
                    pipe = pipe.saturating_add(seg.len);
                }
                if seg.retransmitted && !seg.sacked {
                    pipe = pipe.saturating_add(seg.len);
                }
            }
        }
        pipe
    }

    /// RFC 6675 `NextSeg`: return the lowest-sequence lost segment that has not yet
    /// been retransmitted, and mark it retransmitted.
    pub fn next_retransmit(&mut self) -> Option<SackBlock> {
        for i in 0..self.len {
            if let Some(seg) = self.segments[i].as_mut() {
                if seg.lost && !seg.sacked && !seg.retransmitted {
                    seg.retransmitted = true;
                    return SackBlock::new(seg.seq, seg.end());
                }
            }
        }
        None
    }

    /// On RTO timeout, clear SACK/retransmit/lost bits per RFC 6675 §5.1 and exit fast recovery.
    pub fn on_timeout(&mut self) {
        self.in_fast_recovery = false;
        self.dup_acks = 0;
        for i in 0..self.len {
            if let Some(seg) = self.segments[i].as_mut() {
                seg.sacked = false;
                seg.retransmitted = false;
                seg.lost = false;
            }
        }
    }

    /// Number of tracked unacknowledged segments.
    pub const fn tracked_segments(&self) -> usize {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_scale_negotiation_and_syn_unscaled_rules() {
        // 1 MiB receive buffer requires shift = 5 (65535 << 4 = 1048560 < 1048576, << 5 = 2097120).
        let mut ws = WindowScaleState::for_receive_buffer(1024 * 1024);
        assert_eq!(ws.rcv_shift, 5);
        assert!(!ws.enabled);

        // Encode/decode option roundtrip.
        let mut opt = [0u8; 4];
        assert_eq!(ws.encode_option(&mut opt), Some(3));
        assert_eq!(&opt[..3], &[OPT_WINDOW_SCALE, 3, 5]);
        assert_eq!(WindowScaleState::decode_option(&opt[..3]), Some(5));

        // RFC 7323 §2.3: shift > 14 must be clamped to 14.
        assert_eq!(WindowScaleState::decode_option(&[OPT_WINDOW_SCALE, 3, 25]), Some(14));
        let ws_max = WindowScaleState::with_shift(20);
        assert_eq!(ws_max.rcv_shift, 14);

        // RFC 7323 §2.2: SYN window is never scaled.
        assert_eq!(ws.advertise_window(1024 * 1024, true), u16::MAX);

        // Peer responds with shift 7.
        ws.negotiate_peer(Some(7));
        assert!(ws.enabled);
        assert_eq!(ws.snd_shift, 7);

        // Non-SYN local window is shifted right by rcv_shift (5).
        assert_eq!(ws.advertise_window(1024 * 1024, false), 32768);
        // Peer's SYN-ACK window is unscaled; subsequent ACK windows are shifted left by 7.
        assert_eq!(ws.update_peer_window(65535, true), 65535);
        assert_eq!(ws.update_peer_window(4096, false), 4096 << 7);

        // If peer did NOT send Window Scale, both shifts drop to 0 and scaling disables.
        let mut no_ws = WindowScaleState::with_shift(6);
        no_ws.negotiate_peer(None);
        assert!(!no_ws.enabled);
        assert_eq!(no_ws.rcv_shift, 0);
        assert_eq!(no_ws.snd_shift, 0);
        assert_eq!(no_ws.update_peer_window(4096, false), 4096);
    }

    #[test]
    fn sack_option_codec_and_parser_handle_valid_and_malformed_inputs() {
        let mut buf = [0u8; 40];
        let mut off = 0;
        // MSS = 1460
        buf[off..off + 4].copy_from_slice(&[OPT_MSS, 4, 0x05, 0xb4]);
        off += 4;
        buf[off] = OPT_NOP;
        off += 1;
        // Window Scale = 8
        let ws = WindowScaleState::with_shift(8);
        off += ws.encode_option(&mut buf[off..]).unwrap();
        // SACK-Permitted
        off += encode_sack_permitted(&mut buf[off..]).unwrap();
        // 3 SACK blocks
        let blocks = [
            SackBlock::new(2000, 3000).unwrap(),
            SackBlock::new(5000, 6000).unwrap(),
            SackBlock::new(0xffff_ff00, 0x0000_0100).unwrap(), // wraps across 2^32!
        ];
        off += encode_sack_blocks(&blocks, &mut buf[off..]).unwrap();

        let parsed = parse_tcp_options(&buf[..off]);
        assert_eq!(parsed.mss, Some(1460));
        assert_eq!(parsed.window_scale, Some(8));
        assert!(parsed.sack_permitted);
        assert_eq!(parsed.sack_count, 3);
        let mut out = [SackBlock { left: 0, right: 0 }; MAX_SACK_BLOCKS];
        assert_eq!(parsed.copy_sack_blocks(&mut out), 3);
        assert_eq!(&out[..3], &blocks);

        // Malformed option lengths must fail closed without panicking.
        let bad = [OPT_SACK, 0, 1, 2, 3];
        let p_bad = parse_tcp_options(&bad);
        assert_eq!(p_bad.sack_count, 0);
        let truncated = [OPT_WINDOW_SCALE, 3, 7, OPT_SACK, 18, 0, 0];
        let p_trunc = parse_tcp_options(&truncated);
        assert_eq!(p_trunc.window_scale, Some(7));
        assert_eq!(p_trunc.sack_count, 0);
    }

    #[test]
    fn sack_scoreboard_coalesces_orders_by_recency_and_advances_rcv_nxt() {
        let mut sb = SackScoreboard::new();
        let rcv_nxt = 1000;

        // Out-of-order arrivals with gaps.
        assert!(sb.insert(rcv_nxt, 2000, 500)); // [2000, 2500)
        assert!(sb.insert(rcv_nxt, 3000, 500)); // [3000, 3500) -> most recent!
        assert_eq!(sb.len(), 2);

        let mut ack_blocks = [SackBlock { left: 0, right: 0 }; MAX_SACK_BLOCKS];
        assert_eq!(sb.blocks_for_ack(&mut ack_blocks), 2);
        // RFC 2018 §4: most recently updated block is first.
        assert_eq!(ack_blocks[0], SackBlock { left: 3000, right: 3500 });
        assert_eq!(ack_blocks[1], SackBlock { left: 2000, right: 2500 });

        // Bridge the gap [2500, 3000) -> coalesces into [2000, 3500).
        assert!(sb.insert(rcv_nxt, 2500, 500));
        assert_eq!(sb.len(), 1);
        assert_eq!(sb.blocks_for_ack(&mut ack_blocks), 1);
        assert_eq!(ack_blocks[0], SackBlock { left: 2000, right: 3500 });
        assert_eq!(sb.total_bytes(), 1500);

        // Duplicate below rcv_nxt is ignored.
        assert!(!sb.insert(rcv_nxt, 500, 500));

        // Fill initial hole [1000, 2000) -> advance_rcv_nxt jumps all the way to 3500!
        assert_eq!(sb.advance_rcv_nxt(2000), 3500);
        assert!(sb.is_empty());
    }

    #[test]
    fn sack_scoreboard_is_strictly_bounded_under_fragmentation() {
        let mut sb = SackScoreboard::new();
        for i in 0..(SCOREBOARD_CAPACITY as u32 + 10) {
            let seq = 1000 + i * 200; // non-adjacent 100-byte blocks
            assert!(sb.insert(500, seq, 100));
            assert!(sb.len() <= SCOREBOARD_CAPACITY);
        }
        assert_eq!(sb.len(), SCOREBOARD_CAPACITY);
    }

    #[test]
    fn sack_sender_loss_detection_fast_recovery_and_retransmit() {
        let mss = 1000u32;
        let mut tx = SackSenderTracker::new(1000, mss);
        // Send 5 segments: [1000..2000), [2000..3000), [3000..4000), [4000..5000), [5000..6000)
        for i in 0..5u32 {
            assert!(tx.on_send(1000 + i * mss, mss));
        }
        assert_eq!(tx.pipe(), 5000);

        // Segment 0 ([1000..2000)) is lost; peer SACKs segments 1, 2, 3 one by one.
        let r1 = tx.on_ack(1000, &[SackBlock::new(2000, 3000).unwrap()]);
        assert!(!r1.enter_recovery);
        assert_eq!(r1.newly_sacked_bytes, 1000);

        let r2 = tx.on_ack(1000, &[SackBlock::new(2000, 4000).unwrap()]);
        assert!(!r2.enter_recovery);
        assert_eq!(r2.newly_sacked_bytes, 1000);

        // 3rd SACK block above [1000..2000) -> triggers RFC 6675 loss & fast recovery!
        let r3 = tx.on_ack(1000, &[SackBlock::new(2000, 5000).unwrap()]);
        assert!(r3.enter_recovery);
        assert!(tx.in_fast_recovery);
        assert_eq!(r3.lost_segments, 1);
        assert_eq!(tx.recovery_point, 6000);
        // Pipe: only [5000..6000) is unsacked & unlost = 1000 bytes.
        assert_eq!(r3.pipe, 1000);

        // NextSeg returns [1000..2000) for fast retransmit.
        assert_eq!(tx.next_retransmit(), Some(SackBlock { left: 1000, right: 2000 }));
        assert_eq!(tx.next_retransmit(), None);
        // Pipe now includes the retransmitted 1000-byte segment -> 2000 bytes.
        assert_eq!(tx.pipe(), 2000);

        // Partial ACK (e.g., to 5000) keeps us in fast recovery until recovery_point (6000).
        let r4 = tx.on_ack(5000, &[]);
        assert!(!r4.exit_recovery);
        assert!(tx.in_fast_recovery);
        assert_eq!(r4.newly_acked_bytes, 4000);

        // Full ACK to 6000 exits fast recovery.
        let r5 = tx.on_ack(6000, &[]);
        assert!(r5.exit_recovery);
        assert!(!tx.in_fast_recovery);
        assert_eq!(tx.tracked_segments(), 0);
        assert_eq!(tx.pipe(), 0);
    }
}
