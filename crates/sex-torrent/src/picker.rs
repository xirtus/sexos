// SPDX-License-Identifier: GPL-3.0-only
//! The piece picker: which blocks to ask which peer for next.
//!
//! Order of preference when a peer has room for requests:
//! 1. **Deadline pieces** (streaming), earliest deadline first. [`Picker::set_deadline`]
//!    takes single (piece, time) pairs (what `stream`'s planner produces), and
//!    [`Picker::set_playhead`] / [`Picker::seek`] manage a whole play window: the file's
//!    header and footer pieces are pinned first (players read the container index there),
//!    then a window ahead of the play position, with deadlines spaced by the playback
//!    rate; seeking moves the window. An overdue deadline block may be requested from a
//!    second peer at the same time, so one slow peer cannot stall playback.
//! 2. **Sequential** mode (if on): the lowest wanted piece.
//! 3. **Partial pieces** already started, highest priority and most complete first, so
//!    pieces finish and can be shared instead of many being half done.
//! 4. **Rarest first**: unstarted pieces ordered by (priority, availability, a random
//!    per-piece salt from the caller's randomness), kept in an ordered set so each pick
//!    and each availability change is O(log n). Peers that have everything ("seeds") are
//!    counted apart: they change no piece's rank, so a seed connecting costs O(1).
//! 5. **End game**: when every wanted block is requested, blocks are requested again
//!    from other peers (up to [`MAX_REQUESTERS`] per block); when one copy arrives the
//!    caller is told whom to cancel.
//!
//! Priorities are per piece, 0 (do not download) to 7; [`crate::storage`] maps file
//! priorities to piece priorities. A piece with a deadline is fetched even at priority 0
//! (streaming a file means wanting it). Blocks are 16 KiB; each block remembers which
//! peers it is requested from and since when ([`Picker::expire`] frees stale ones so they
//! are reissued to someone else).
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use crate::peer::{Bitfield, BlockRef, PeerKey, BLOCK_SIZE};
use crate::Millis;

/// At most this many peers are asked for the same block (end game, overdue deadlines).
pub const MAX_REQUESTERS: usize = 2;
/// Sequential mode looks at most this many pieces ahead for one the peer has.
const SEQ_SCAN: u32 = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Block { Free, Requested(Vec<(PeerKey, Millis)>), Received(PeerKey) }

#[derive(Clone, Debug)]
struct Partial { blocks: Vec<Block>, received: u32 }

impl Partial {
    fn free_blocks(&self) -> bool { self.blocks.iter().any(|b| *b == Block::Free) }
    fn idle(&self) -> bool { self.blocks.iter().all(|b| *b == Block::Free) }
}

/// A streaming play window over one file's pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Playhead {
    /// The file's first and last piece (inclusive).
    pub first: u32,
    pub last: u32,
    /// Pieces at the start and end of the file pinned as urgent (container headers).
    pub header: u32,
    pub footer: u32,
    /// The piece being played now.
    pub position: u32,
    /// Pieces ahead of the position to fetch with deadlines.
    pub window: u32,
    /// Milliseconds of playback per piece (spacing of the window's deadlines).
    pub step: Millis,
}

/// What a received block meant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockOutcome {
    /// Not a block of a piece we want (bad range, or the piece is done).
    NotWanted,
    /// We already had it.
    Duplicate,
    /// New data. `cancel`: other peers it was also requested from.
    Accepted { piece_complete: bool, cancel: Vec<PeerKey> },
}

type Key = (u8, u32, u32, u32); // (7 - priority, availability, salt, piece)

/// The picker for one torrent.
#[derive(Clone, Debug)]
pub struct Picker {
    num_pieces: u32,
    piece_length: u32,
    short: BTreeMap<u32, u32>,
    have: Bitfield,
    avail: Vec<u32>,
    seeds: u32,
    prio: Vec<u8>,
    salt: Vec<u32>,
    fresh: BTreeSet<Key>,
    partial: BTreeMap<u32, Partial>,
    deadlines: BTreeSet<(Millis, u32)>,
    deadline_of: BTreeMap<u32, Millis>,
    playhead: Option<Playhead>,
    playhead_pieces: BTreeSet<u32>,
    sequential: bool,
    seq_cursor: u32,
    wanted_left: u32,
}

impl Picker {
    /// `short` lists pieces shorter than `piece_length` (see
    /// [`crate::storage::TorrentLayout::short_pieces`]); `random` gives the tie-break salts.
    pub fn new(num_pieces: u32, piece_length: u32, short: BTreeMap<u32, u32>, have: Bitfield, priorities: &[u8], random: &mut dyn FnMut() -> u32) -> Picker {
        let have = if have.len() == num_pieces { have } else { Bitfield::new(num_pieces) };
        let n = num_pieces as usize;
        let mut p = Picker {
            num_pieces, piece_length, short, have, avail: alloc::vec![0; n], seeds: 0,
            prio: (0..n).map(|i| priorities.get(i).copied().unwrap_or(1).min(7)).collect(),
            salt: (0..n).map(|_| random()).collect(), fresh: BTreeSet::new(), partial: BTreeMap::new(),
            deadlines: BTreeSet::new(), deadline_of: BTreeMap::new(), playhead: None, playhead_pieces: BTreeSet::new(),
            sequential: false, seq_cursor: 0, wanted_left: 0,
        };
        for i in 0..num_pieces {
            if p.prio[i as usize] > 0 && !p.have.get(i) { p.wanted_left += 1; }
            p.link(i);
        }
        p
    }

    pub fn num_pieces(&self) -> u32 { self.num_pieces }
    pub fn have(&self) -> &Bitfield { &self.have }
    pub fn has(&self, piece: u32) -> bool { self.have.get(piece) }
    pub fn piece_size(&self, piece: u32) -> u32 { if piece >= self.num_pieces { 0 } else { self.short.get(&piece).copied().unwrap_or(self.piece_length) } }
    pub fn blocks_in(&self, piece: u32) -> u32 { self.piece_size(piece).div_ceil(BLOCK_SIZE) }
    fn block(&self, piece: u32, i: u32) -> BlockRef {
        let begin = i * BLOCK_SIZE;
        BlockRef { piece, begin, len: (self.piece_size(piece) - begin).min(BLOCK_SIZE) }
    }
    pub fn priority(&self, piece: u32) -> u8 { self.prio.get(piece as usize).copied().unwrap_or(0) }
    /// Copies of the piece among connected peers (seeds included).
    pub fn availability(&self, piece: u32) -> u32 { self.avail.get(piece as usize).copied().unwrap_or(0) + self.seeds }
    /// Wanted pieces (priority > 0) we do not have yet.
    pub fn wanted_left(&self) -> u32 { self.wanted_left }
    /// All wanted pieces are done.
    pub fn is_finished(&self) -> bool { self.wanted_left == 0 }
    pub fn is_seed(&self) -> bool { self.have.all() }
    pub fn partial_pieces(&self) -> impl Iterator<Item = u32> + '_ { self.partial.keys().copied() }
    pub fn deadline(&self, piece: u32) -> Option<Millis> { self.deadline_of.get(&piece).copied() }
    /// Whether the piece is wanted: priority above 0 or a deadline, and not had.
    pub fn wanted(&self, piece: u32) -> bool { piece < self.num_pieces && !self.have.get(piece) && (self.prio[piece as usize] > 0 || self.deadline_of.contains_key(&piece)) }

    fn key(&self, p: u32) -> Key { (7 - self.prio[p as usize], self.avail[p as usize], self.salt[p as usize], p) }
    fn eligible(&self, p: u32) -> bool { !self.have.get(p) && self.prio[p as usize] > 0 && !self.partial.contains_key(&p) }
    fn unlink(&mut self, p: u32) { if self.eligible(p) { let k = self.key(p); self.fresh.remove(&k); } }
    fn link(&mut self, p: u32) { if self.eligible(p) { let k = self.key(p); self.fresh.insert(k); } }

    // ------------------------------------------------------------ availability

    pub fn inc(&mut self, piece: u32) {
        if piece >= self.num_pieces { return; }
        self.unlink(piece);
        self.avail[piece as usize] = self.avail[piece as usize].saturating_add(1);
        self.link(piece);
    }
    pub fn dec(&mut self, piece: u32) {
        if piece >= self.num_pieces { return; }
        self.unlink(piece);
        self.avail[piece as usize] = self.avail[piece as usize].saturating_sub(1);
        self.link(piece);
    }
    /// A peer's pieces arrived; a full bitfield counts as a seed (O(1)).
    pub fn add_peer_pieces(&mut self, have: &Bitfield) {
        if have.len() != self.num_pieces { return; }
        if have.all() { self.seeds += 1; } else { for p in have.ones() { self.inc(p); } }
    }
    /// The inverse of [`add_peer_pieces`](Self::add_peer_pieces) (pass the same kind:
    /// `as_seed` says whether the peer was counted as a seed).
    pub fn remove_peer_pieces(&mut self, have: &Bitfield, as_seed: bool) {
        if as_seed { self.seeds = self.seeds.saturating_sub(1); } else if have.len() == self.num_pieces { for p in have.ones() { self.dec(p); } }
    }
    /// A peer counted as a seed lost a piece (lt_donthave): it is now counted per piece.
    pub fn seed_lost_piece(&mut self, have_after: &Bitfield) {
        self.seeds = self.seeds.saturating_sub(1);
        for p in have_after.ones() { self.inc(p); }
    }

    // ------------------------------------------------------------ priorities and modes

    pub fn set_priority(&mut self, piece: u32, prio: u8) {
        if piece >= self.num_pieces { return; }
        let prio = prio.min(7);
        let old = self.prio[piece as usize];
        if old == prio { return; }
        self.unlink(piece);
        if !self.have.get(piece) {
            if old == 0 { self.wanted_left += 1; }
            if prio == 0 { self.wanted_left -= 1; }
        }
        self.prio[piece as usize] = prio;
        self.link(piece);
        if prio > 0 { self.seq_cursor = self.seq_cursor.min(piece); }
    }
    pub fn set_priorities(&mut self, prios: &[u8]) {
        for p in 0..self.num_pieces { self.set_priority(p, prios.get(p as usize).copied().unwrap_or(0)); }
    }
    pub fn set_sequential(&mut self, on: bool) { self.sequential = on; self.seq_cursor = 0; }
    pub fn sequential(&self) -> bool { self.sequential }

    /// Wants `piece` by `at` (earlier deadlines are fetched first). Ignored for pieces
    /// we have.
    pub fn set_deadline(&mut self, piece: u32, at: Millis) {
        if piece >= self.num_pieces || self.have.get(piece) { return; }
        self.clear_deadline(piece);
        self.deadlines.insert((at, piece));
        self.deadline_of.insert(piece, at);
    }
    pub fn clear_deadline(&mut self, piece: u32) {
        if let Some(t) = self.deadline_of.remove(&piece) { self.deadlines.remove(&(t, piece)); }
        self.playhead_pieces.remove(&piece);
    }
    pub fn clear_deadlines(&mut self) {
        self.deadlines.clear();
        self.deadline_of.clear();
        self.playhead_pieces.clear();
        self.playhead = None;
    }
    /// Pieces with a deadline, earliest first.
    pub fn deadlines(&self) -> impl Iterator<Item = (u32, Millis)> + '_ { self.deadlines.iter().map(|(t, p)| (*p, *t)) }

    /// Starts (or replaces) a play window and sets its deadlines.
    pub fn set_playhead(&mut self, ph: Playhead, now: Millis) {
        self.playhead = Some(ph);
        self.apply_playhead(now);
    }
    /// Moves the play position (a seek, or playback advancing).
    pub fn seek(&mut self, position: u32, now: Millis) {
        if let Some(ph) = self.playhead.as_mut() { ph.position = position; }
        self.apply_playhead(now);
    }
    pub fn clear_playhead(&mut self) {
        for p in core::mem::take(&mut self.playhead_pieces) { if let Some(t) = self.deadline_of.remove(&p) { self.deadlines.remove(&(t, p)); } }
        self.playhead = None;
    }
    pub fn playhead(&self) -> Option<Playhead> { self.playhead }

    fn apply_playhead(&mut self, now: Millis) {
        let Some(ph) = self.playhead else { return };
        for p in core::mem::take(&mut self.playhead_pieces) { if let Some(t) = self.deadline_of.remove(&p) { self.deadlines.remove(&(t, p)); } }
        let last = ph.last.min(self.num_pieces.saturating_sub(1));
        if ph.first > last { return; }
        let mut want: Vec<(u32, Millis)> = Vec::new();
        let header_end = ph.first.saturating_add(ph.header).min(last + 1);
        for p in ph.first..header_end { want.push((p, now)); }
        let footer_start = (last + 1).saturating_sub(ph.footer).max(ph.first);
        for p in footer_start..=last { if ph.footer > 0 { want.push((p, now)); } }
        let pos = ph.position.clamp(ph.first, last);
        let end = pos.saturating_add(ph.window).min(last + 1);
        for (i, p) in (pos..end).enumerate() { want.push((p, now.saturating_add(1).saturating_add((i as u64).saturating_mul(ph.step)))); }
        for (p, t) in want {
            if self.have.get(p) || self.deadline_of.get(&p).is_some_and(|old| *old <= t) { continue; }
            self.set_deadline(p, t);
            self.playhead_pieces.insert(p);
        }
    }

    // ------------------------------------------------------------ picking

    /// Takes free blocks of `p` for `peer` (starting the piece if needed); with `dup`,
    /// also blocks already requested from others (end game, overdue deadline).
    fn take(&mut self, p: u32, peer: PeerKey, now: Millis, dup: bool, max: usize, out: &mut Vec<BlockRef>) {
        if out.len() >= max || !self.wanted(p) { return; }
        if !self.partial.contains_key(&p) {
            self.unlink(p);
            let n = self.blocks_in(p) as usize;
            self.partial.insert(p, Partial { blocks: alloc::vec![Block::Free; n], received: 0 });
        }
        let mut taken = Vec::new();
        if let Some(part) = self.partial.get_mut(&p) {
            for (i, b) in part.blocks.iter_mut().enumerate() {
                if out.len() + taken.len() >= max { break; }
                match b {
                    Block::Free => { *b = Block::Requested(alloc::vec![(peer, now)]); taken.push(i as u32); }
                    Block::Requested(who) if dup && who.len() < MAX_REQUESTERS && !who.iter().any(|(k, _)| *k == peer) => { who.push((peer, now)); taken.push(i as u32); }
                    _ => {}
                }
            }
        }
        for i in taken { out.push(self.block(p, i)); }
    }

    /// Up to `max` blocks to request from `peer`, which has the pieces in `has`. `only`
    /// restricts to a set of pieces (the allowed-fast set while the peer chokes us).
    pub fn pick(&mut self, peer: PeerKey, has: &Bitfield, max: usize, only: Option<&[u32]>, now: Millis) -> Vec<BlockRef> {
        let mut out = Vec::new();
        if max == 0 || has.len() != self.num_pieces { return out; }
        let ok = |p: u32| has.get(p) && only.is_none_or(|o| o.contains(&p));
        // 1. Deadlines.
        let dl: Vec<(Millis, u32)> = self.deadlines.iter().copied().collect();
        for (t, p) in &dl {
            if out.len() >= max { return out; }
            if ok(*p) { self.take(*p, peer, now, *t <= now, max, &mut out); }
        }
        // 2. Sequential.
        if self.sequential {
            // Skip pieces that are done or deselected (a priority change resets the cursor).
            while self.seq_cursor < self.num_pieces && (self.have.get(self.seq_cursor) || self.prio[self.seq_cursor as usize] == 0) {
                self.seq_cursor += 1;
            }
            let end = self.seq_cursor.saturating_add(SEQ_SCAN).min(self.num_pieces);
            for p in self.seq_cursor..end {
                if out.len() >= max { return out; }
                if self.prio[p as usize] > 0 && ok(p) { self.take(p, peer, now, false, max, &mut out); }
            }
        }
        // 3. Partial pieces: highest priority, then most received.
        let mut parts: Vec<(u8, u32, u32)> = self.partial.iter().filter(|(p, part)| ok(**p) && part.free_blocks()).map(|(p, part)| (7 - self.prio[*p as usize], u32::MAX - part.received, *p)).collect();
        parts.sort_unstable();
        for (_, _, p) in parts {
            if out.len() >= max { return out; }
            self.take(p, peer, now, false, max, &mut out);
        }
        // 4. Rarest first.
        if out.len() < max {
            let mut chosen = Vec::new();
            let mut blocks = out.len();
            for k in self.fresh.iter() {
                if blocks >= max { break; }
                if ok(k.3) { chosen.push(k.3); blocks += self.blocks_in(k.3) as usize; }
            }
            for p in chosen { self.take(p, peer, now, false, max, &mut out); }
        }
        // 5. End game: nothing unrequested is left anywhere.
        if out.is_empty() && self.in_end_game() {
            let ps: Vec<u32> = self.partial.keys().copied().filter(|p| ok(*p)).collect();
            for p in ps { if out.len() >= max { break; } self.take(p, peer, now, true, max, &mut out); }
        }
        out
    }

    /// Every wanted block is requested or received.
    pub fn in_end_game(&self) -> bool {
        self.wanted_left > 0 && self.fresh.is_empty() && self.partial.values().all(|p| !p.free_blocks())
    }

    // ------------------------------------------------------------ results

    fn locate(&self, b: &BlockRef) -> Option<usize> {
        if b.piece >= self.num_pieces || b.begin % BLOCK_SIZE != 0 { return None; }
        let i = b.begin / BLOCK_SIZE;
        (i < self.blocks_in(b.piece) && self.block(b.piece, i) == *b).then_some(i as usize)
    }

    /// A block arrived from `peer`.
    pub fn on_block(&mut self, peer: PeerKey, b: &BlockRef) -> BlockOutcome {
        let Some(i) = self.locate(b) else { return BlockOutcome::NotWanted };
        if self.have.get(b.piece) { return BlockOutcome::NotWanted; }
        if !self.partial.contains_key(&b.piece) {
            if !self.wanted(b.piece) { return BlockOutcome::NotWanted; }
            // Unrequested but useful (for example after its request timed out).
            self.unlink(b.piece);
            let n = self.blocks_in(b.piece) as usize;
            self.partial.entry(b.piece).or_insert(Partial { blocks: alloc::vec![Block::Free; n], received: 0 });
        }
        let Some(part) = self.partial.get_mut(&b.piece) else { return BlockOutcome::NotWanted };
        let cancel = match &part.blocks[i] {
            Block::Received(_) => return BlockOutcome::Duplicate,
            Block::Requested(who) => who.iter().map(|(k, _)| *k).filter(|k| *k != peer).collect(),
            Block::Free => Vec::new(),
        };
        part.blocks[i] = Block::Received(peer);
        part.received += 1;
        BlockOutcome::Accepted { piece_complete: part.received as usize == part.blocks.len(), cancel }
    }

    /// A request to `peer` ended without data (reject, choke, timeout, cancel): the block
    /// is free for others again.
    pub fn on_request_lost(&mut self, peer: PeerKey, b: &BlockRef) {
        let Some(i) = self.locate(b) else { return };
        let Some(part) = self.partial.get_mut(&b.piece) else { return };
        if let Block::Requested(who) = &mut part.blocks[i] {
            who.retain(|(k, _)| *k != peer);
            if who.is_empty() { part.blocks[i] = Block::Free; }
        }
        self.drop_if_idle(b.piece);
    }

    fn drop_if_idle(&mut self, p: u32) {
        if self.partial.get(&p).is_some_and(|x| x.idle()) { self.partial.remove(&p); self.link(p); }
    }

    /// A peer disconnected: all its requests are freed.
    pub fn peer_gone(&mut self, peer: PeerKey) {
        let ps: Vec<u32> = self.partial.keys().copied().collect();
        for p in ps {
            if let Some(part) = self.partial.get_mut(&p) {
                for b in part.blocks.iter_mut() {
                    if let Block::Requested(who) = b { who.retain(|(k, _)| *k != peer); if who.is_empty() { *b = Block::Free; } }
                }
            }
            self.drop_if_idle(p);
        }
    }

    /// Frees requests older than `age`; returns them so the caller can cancel them with
    /// the peers. They are reissued by later picks.
    pub fn expire(&mut self, now: Millis, age: Millis) -> Vec<(PeerKey, BlockRef)> {
        let mut out = Vec::new();
        let ps: Vec<u32> = self.partial.keys().copied().collect();
        for p in ps {
            let mut freed = Vec::new();
            if let Some(part) = self.partial.get_mut(&p) {
                for (i, b) in part.blocks.iter_mut().enumerate() {
                    if let Block::Requested(who) = b {
                        who.retain(|(k, t)| { let old = now.saturating_sub(*t) > age; if old { freed.push((*k, i as u32)); } !old });
                        if who.is_empty() { *b = Block::Free; }
                    }
                }
            }
            for (k, i) in freed { out.push((k, self.block(p, i))); }
            self.drop_if_idle(p);
        }
        out
    }

    /// The piece verified: we have it.
    pub fn piece_passed(&mut self, piece: u32) {
        if piece >= self.num_pieces || self.have.get(piece) { return; }
        self.unlink(piece);
        self.partial.remove(&piece);
        self.have.set(piece);
        if self.prio[piece as usize] > 0 { self.wanted_left -= 1; }
        self.clear_deadline(piece);
    }

    /// The piece failed its hash: it is downloaded again. Returns the peers that sent its
    /// blocks.
    pub fn piece_failed(&mut self, piece: u32) -> Vec<PeerKey> {
        let mut who = Vec::new();
        if let Some(part) = self.partial.remove(&piece) {
            for b in part.blocks { if let Block::Received(k) = b { if !who.contains(&k) { who.push(k); } } }
        }
        self.link(piece);
        who
    }

    /// We lost a piece (failed recheck, or the file was deleted): fetch it again.
    pub fn piece_lost(&mut self, piece: u32) {
        if piece >= self.num_pieces || !self.have.get(piece) { return; }
        self.have.clear(piece);
        if self.prio[piece as usize] > 0 { self.wanted_left += 1; }
        self.link(piece);
    }

    /// Marks a block as received (resume data); its piece becomes a started piece.
    pub fn restore_block(&mut self, piece: u32, block: u32) {
        if !self.wanted(piece) || block >= self.blocks_in(piece) { return; }
        if !self.partial.contains_key(&piece) {
            self.unlink(piece);
            let n = self.blocks_in(piece) as usize;
            self.partial.insert(piece, Partial { blocks: alloc::vec![Block::Free; n], received: 0 });
        }
        if let Some(part) = self.partial.get_mut(&piece) {
            if !matches!(part.blocks[block as usize], Block::Received(_)) { part.blocks[block as usize] = Block::Received(PeerKey::MAX); part.received += 1; }
        }
    }

    /// Blocks received so far of unfinished pieces (for resume data).
    pub fn unfinished(&self) -> Vec<(u32, Vec<bool>)> {
        self.partial.iter().map(|(p, part)| (*p, part.blocks.iter().map(|b| matches!(b, Block::Received(_))).collect())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn rng() -> impl FnMut() -> u32 { let mut s = 0x9e37_79b9u32; move || { s ^= s << 13; s ^= s >> 17; s ^= s << 5; s } }
    const B: u32 = BLOCK_SIZE;

    fn picker(n: u32, plen: u32) -> Picker { Picker::new(n, plen, BTreeMap::new(), Bitfield::new(n), &[], &mut rng()) }

    fn bf(n: u32, ones: &[u32]) -> Bitfield { let mut b = Bitfield::new(n); for &i in ones { b.set(i); } b }

    #[test]
    fn rarest_first_with_seeds_apart() {
        let mut p = picker(4, B);
        p.add_peer_pieces(&bf(4, &[0, 1, 2]));
        p.add_peer_pieces(&bf(4, &[0, 1]));
        p.add_peer_pieces(&bf(4, &[0]));
        p.add_peer_pieces(&Bitfield::full(4));
        assert_eq!((p.availability(0), p.availability(3)), (4, 1));
        // Piece 3 (only the seed) is rarest, then 2, 1, 0.
        let got: Vec<u32> = p.pick(9, &Bitfield::full(4), 4, None, 0).iter().map(|b| b.piece).collect();
        assert_eq!(got, vec![3, 2, 1, 0]);
        p.remove_peer_pieces(&Bitfield::full(4), true);
        assert_eq!(p.availability(3), 0);
    }

    #[test]
    fn whole_pieces_partials_first_and_blocks() {
        let mut p = Picker::new(3, 3 * B, BTreeMap::from([(2, B + 5)]), Bitfield::new(3), &[7, 1, 1], &mut rng());
        assert_eq!(p.blocks_in(2), 2);
        let a = p.pick(1, &Bitfield::full(3), 2, None, 0);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].piece, a[1].piece, "blocks of one piece together");
        // Another peer continues the started piece first.
        let b = p.pick(2, &Bitfield::full(3), 1, None, 0);
        assert_eq!(b[0].piece, a[0].piece);
        assert_eq!(b[0].begin, 2 * B);
        // The short last piece has a short last block.
        let all = p.pick(3, &Bitfield::full(3), 100, None, 0);
        assert!(all.contains(&BlockRef { piece: 2, begin: B, len: 5 }));
        assert!(p.in_end_game());
    }

    #[test]
    fn block_lifecycle_and_hash_failure() {
        let mut p = picker(2, 2 * B);
        let blocks = p.pick(1, &Bitfield::full(2), 2, None, 0);
        let piece = blocks[0].piece;
        assert_eq!(p.on_block(1, &blocks[0]), BlockOutcome::Accepted { piece_complete: false, cancel: vec![] });
        assert_eq!(p.on_block(1, &blocks[0]), BlockOutcome::Duplicate);
        assert_eq!(p.on_block(1, &BlockRef { piece, begin: 1, len: B }), BlockOutcome::NotWanted);
        assert_eq!(p.on_block(1, &BlockRef { piece: 9, begin: 0, len: B }), BlockOutcome::NotWanted);
        assert_eq!(p.on_block(2, &blocks[1]), BlockOutcome::Accepted { piece_complete: true, cancel: vec![1] });
        assert_eq!(p.piece_failed(piece), vec![1, 2]);
        assert_eq!(p.wanted_left(), 2);
        // Refetched, passes.
        let again = p.pick(3, &bf(2, &[piece]), 5, None, 0);
        assert_eq!(again.len(), 2);
        for b in &again { p.on_block(3, b); }
        p.piece_passed(piece);
        assert!(p.has(piece) && p.wanted_left() == 1);
        assert_eq!(p.on_block(3, &again[0]), BlockOutcome::NotWanted);
        p.piece_lost(piece);
        assert_eq!(p.wanted_left(), 2);
    }

    #[test]
    fn lost_requests_and_peer_gone_reissue() {
        let mut p = picker(1, 2 * B);
        let a = p.pick(1, &Bitfield::full(1), 2, None, 0);
        assert!(p.pick(2, &Bitfield::full(1), 2, None, 0).iter().all(|b| a.contains(b)), "end game duplicates only");
        p.peer_gone(1);
        p.peer_gone(2);
        assert_eq!(p.pick(3, &Bitfield::full(1), 2, None, 0), a, "freed blocks go to the next peer");
        p.on_request_lost(3, &a[0]);
        assert_eq!(p.pick(4, &Bitfield::full(1), 1, None, 0), vec![a[0]]);
        // Expire frees stale requests.
        let gone = p.expire(100_000, 30_000);
        assert_eq!(gone.len(), 2);
        assert_eq!(p.pick(5, &Bitfield::full(1), 2, None, 100_000).len(), 2);
    }

    #[test]
    fn end_game_duplicates_and_cancels() {
        let mut p = picker(1, B);
        let a = p.pick(1, &Bitfield::full(1), 5, None, 0);
        assert_eq!(a.len(), 1);
        assert!(p.in_end_game());
        let b = p.pick(2, &Bitfield::full(1), 5, None, 0);
        assert_eq!(b, a);
        assert!(p.pick(3, &Bitfield::full(1), 5, None, 0).is_empty(), "at most two requesters");
        assert_eq!(p.on_block(2, &a[0]), BlockOutcome::Accepted { piece_complete: true, cancel: vec![1] });
    }

    #[test]
    fn priorities_and_dont_download() {
        let mut p = Picker::new(4, B, BTreeMap::new(), Bitfield::new(4), &[1, 0, 7, 1], &mut rng());
        assert_eq!(p.wanted_left(), 3);
        let got: Vec<u32> = p.pick(1, &Bitfield::full(4), 10, None, 0).iter().map(|b| b.piece).collect();
        assert_eq!(got[0], 2, "priority 7 first");
        assert!(!got.contains(&1), "priority 0 never");
        p.set_priority(1, 3);
        assert_eq!(p.wanted_left(), 4);
        assert_eq!(p.pick(1, &Bitfield::full(4), 10, None, 0).iter().map(|b| b.piece).collect::<Vec<_>>(), vec![1]);
        p.set_priorities(&[0, 0, 0, 0]);
        assert_eq!(p.wanted_left(), 0);
        assert!(p.is_finished());
    }

    #[test]
    fn sequential_mode_and_only_filter() {
        let mut p = picker(20, B);
        p.set_sequential(true);
        let got: Vec<u32> = p.pick(1, &Bitfield::full(20), 5, None, 0).iter().map(|b| b.piece).collect();
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
        for i in 0..5 { p.on_block(1, &BlockRef { piece: i, begin: 0, len: B }); p.piece_passed(i); }
        let got: Vec<u32> = p.pick(2, &bf(20, &[7, 6, 19]), 2, None, 0).iter().map(|b| b.piece).collect();
        assert_eq!(got, vec![6, 7]);
        let only: Vec<u32> = p.pick(3, &Bitfield::full(20), 5, Some(&[12, 15]), 0).iter().map(|b| b.piece).collect();
        assert_eq!(only, vec![12, 15]);
        assert!(p.pick(3, &Bitfield::new(19), 5, None, 0).is_empty(), "wrong-size bitfield");
    }

    #[test]
    fn deadlines_come_first_and_overdue_ones_duplicate() {
        let mut p = picker(100, B);
        p.set_deadline(50, 2_000);
        p.set_deadline(70, 1_000);
        p.set_deadline(20, 3_000);
        let got: Vec<u32> = p.pick(1, &Bitfield::full(100), 3, None, 0).iter().map(|b| b.piece).collect();
        assert_eq!(got, vec![70, 50, 20]);
        // Not yet overdue: no duplicates; overdue: a second peer gets them too.
        let two: Vec<u32> = p.pick(2, &Bitfield::full(100), 3, None, 500).iter().map(|b| b.piece).collect();
        assert!(!two.contains(&70));
        let late: Vec<u32> = p.pick(3, &Bitfield::full(100), 2, None, 1_500).iter().map(|b| b.piece).collect();
        assert_eq!(late.len(), 2);
        assert_eq!(late[0], 70);
        // A deadline makes even a priority-0 piece wanted; arrival clears it.
        p.set_priority(90, 0);
        p.set_deadline(90, 0);
        assert_eq!(p.pick(4, &bf(100, &[90]), 1, None, 0)[0].piece, 90);
        p.on_block(4, &BlockRef { piece: 90, begin: 0, len: B });
        p.piece_passed(90);
        assert_eq!(p.deadline(90), None);
        p.set_deadline(90, 5);
        assert_eq!(p.deadline(90), None, "no deadline on pieces we have");
    }

    #[test]
    fn playhead_window_header_footer_and_seek() {
        // A file spanning pieces 10..=109.
        let mut p = picker(200, B);
        let ph = Playhead { first: 10, last: 109, header: 2, footer: 1, position: 10, window: 5, step: 1000 };
        p.set_playhead(ph, 0);
        let order: Vec<u32> = p.deadlines().map(|(p, _)| p).collect();
        // Header (10, 11) and footer (109) pinned at "now", then the window 10..15 (10, 11 already pinned).
        assert_eq!(order, vec![10, 11, 109, 12, 13, 14]);
        let got: Vec<u32> = p.pick(1, &Bitfield::full(200), 6, None, 0).iter().map(|b| b.piece).collect();
        assert_eq!(got, order);
        // Seek to the middle: the old window goes, header/footer stay, the new window comes.
        p.seek(60, 10_000);
        let order: Vec<u32> = p.deadlines().map(|(p, _)| p).collect();
        assert_eq!(order, vec![10, 11, 109, 60, 61, 62, 63, 64]);
        // Pieces we have get no deadline when the window passes them.
        for i in [61u32, 62] { p.on_block(1, &BlockRef { piece: i, begin: 0, len: B }); p.piece_passed(i); }
        p.seek(60, 20_000);
        assert!(!p.deadlines().any(|(x, _)| x == 61 || x == 62));
        // User deadlines survive clear_playhead.
        p.set_deadline(150, 1);
        p.clear_playhead();
        assert_eq!(p.deadlines().map(|(x, _)| x).collect::<Vec<_>>(), vec![150]);
        // Degenerate windows do not panic.
        p.set_playhead(Playhead { first: 500, last: 900, header: 9, footer: 9, position: 0, window: 9, step: 0 }, 0);
        p.set_playhead(Playhead { first: 190, last: u32::MAX, header: u32::MAX, footer: u32::MAX, position: u32::MAX, window: u32::MAX, step: u64::MAX / 4 }, 0);
        assert!(p.deadlines().count() > 0);
    }

    #[test]
    fn unrequested_useful_blocks_are_accepted() {
        let mut p = picker(2, 2 * B);
        assert!(matches!(p.on_block(1, &BlockRef { piece: 1, begin: B, len: B }), BlockOutcome::Accepted { piece_complete: false, .. }));
        assert_eq!(p.unfinished(), vec![(1, vec![false, true])]);
        let next = p.pick(1, &Bitfield::full(2), 1, None, 0);
        assert_eq!(next, vec![BlockRef { piece: 1, begin: 0, len: B }], "the started piece first");
        p.restore_block(0, 1);
        p.restore_block(0, 1);
        p.restore_block(0, 7);
        assert_eq!(p.unfinished(), vec![(0, vec![false, true]), (1, vec![false, true])]);
    }

    #[test]
    fn big_torrent_picks_fast() {
        let n = 200_000;
        let mut p = picker(n, 4 * B);
        let mut r = rng();
        for _ in 0..50 { let mut b = Bitfield::new(n); for _ in 0..1000 { b.set(r() % n); } p.add_peer_pieces(&b); }
        for _ in 0..1000 { p.add_peer_pieces(&Bitfield::full(n)); }
        for k in 0..500 { assert_eq!(p.pick(k, &Bitfield::full(n), 8, None, 0).len(), 8); }
    }
}
