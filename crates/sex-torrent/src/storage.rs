// SPDX-License-Identifier: GPL-3.0-only
//! Where a torrent's bytes live: the file layout, the mapping from pieces and blocks to
//! file ranges, and the disk operations the embedding program runs for us.
//!
//! A torrent is one long byte string cut into pieces; the files are consecutive ranges
//! of it. [`TorrentLayout`] holds that mapping (for v1, hybrids with BEP 47 pad files,
//! and pure v2 where every file starts on a piece boundary and a file's last piece is
//! short). [`Storage`] turns "this block arrived" and "read this block" into [`DiskOp`]s:
//!
//! - Files with priority 0 (deselected) are never written. Bytes of a piece that fall
//!   into a deselected file go to a per-torrent "part file" region keyed by piece
//!   ([`DiskOp::WritePart`], like qBittorrent's `.parts`), so a piece that straddles a
//!   wanted and an unwanted file can still be completed and hashed. When a file is later
//!   selected, its bytes are copied out of the part region ([`DiskOp::PromotePart`]).
//! - Pad files are never written or read; they read as zeros.
//! - Blocks are collected in a bounded write cache until their piece is complete, so a
//!   piece is hashed from memory and written once, after it verified. When the cache is
//!   over budget the least recently touched piece is written out early (unverified); its
//!   blocks are read back when the piece completes.
//! - Reads of a range that spans files become several `Read`s with one id; the program
//!   answers each with [`Storage::on_read`] and gets the assembled buffer back.
//!
//! The program must run the ops of one torrent in order (a read after a write of the
//! same range must see the write).
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use crate::peer::{Bitfield, BLOCK_SIZE};

/// One file of the layout. `offset` is where it starts in the torrent's byte string.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LayoutFile { pub path: Vec<String>, pub length: u64, pub offset: u64, pub pad: bool }

/// How a torrent's pieces map to files.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TorrentLayout {
    pub piece_length: u32,
    pub total_length: u64,
    pub files: Vec<LayoutFile>,
    /// v2: each file starts on a piece boundary, a file's last piece is short.
    pub v2_aligned: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutError { BadPieceLength, NoFiles, Overlap, TooLarge, TooManyPieces }

/// Limits on what we accept.
pub const MAX_PIECE_LENGTH: u32 = 128 << 20;
pub const MAX_PIECES: u64 = 1 << 24;
pub const MAX_FILES: usize = 1 << 20;

/// The part of a byte range that lies in one file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slice {
    pub file: usize,
    pub file_offset: u64,
    pub len: u32,
    /// Where this slice starts within the range that was mapped.
    pub at: u32,
}

impl TorrentLayout {
    /// Builds a layout from `(path, length, pad)` entries, computing offsets (aligned to
    /// pieces when `v2_aligned`).
    pub fn new(piece_length: u32, files: Vec<(Vec<String>, u64, bool)>, v2_aligned: bool) -> Result<TorrentLayout, LayoutError> {
        if piece_length == 0 || piece_length > MAX_PIECE_LENGTH { return Err(LayoutError::BadPieceLength); }
        let mut off = 0u64;
        let mut out = Vec::with_capacity(files.len());
        for (path, length, pad) in files {
            if v2_aligned && length > 0 { off = off.div_ceil(piece_length as u64).checked_mul(piece_length as u64).ok_or(LayoutError::TooLarge)?; }
            out.push(LayoutFile { path, length, offset: off, pad });
            off = off.checked_add(length).ok_or(LayoutError::TooLarge)?;
        }
        let l = TorrentLayout { piece_length, total_length: off, files: out, v2_aligned };
        l.validate()?;
        Ok(l)
    }

    /// A single-file torrent.
    pub fn single(name: &str, length: u64, piece_length: u32) -> Result<TorrentLayout, LayoutError> {
        TorrentLayout::new(piece_length, alloc::vec![(alloc::vec![String::from(name)], length, false)], false)
    }

    /// Checks invariants (also for layouts built by hand): ordered, non-overlapping files
    /// inside the total, sane counts.
    pub fn validate(&self) -> Result<(), LayoutError> {
        if self.piece_length == 0 || self.piece_length > MAX_PIECE_LENGTH { return Err(LayoutError::BadPieceLength); }
        if self.files.is_empty() || self.files.len() > MAX_FILES || self.total_length == 0 { return Err(LayoutError::NoFiles); }
        if self.total_length.div_ceil(self.piece_length as u64) > MAX_PIECES { return Err(LayoutError::TooManyPieces); }
        let mut end = 0u64;
        for f in &self.files {
            let e = f.offset.checked_add(f.length).ok_or(LayoutError::TooLarge)?;
            if f.offset < end || e > self.total_length { return Err(LayoutError::Overlap); }
            if self.v2_aligned && f.length > 0 && f.offset % self.piece_length as u64 != 0 { return Err(LayoutError::Overlap); }
            end = e;
        }
        Ok(())
    }

    pub fn num_pieces(&self) -> u32 { self.total_length.div_ceil(self.piece_length as u64) as u32 }
    pub fn piece_offset(&self, piece: u32) -> u64 { piece as u64 * self.piece_length as u64 }

    /// The file holding byte `offset` (zero-length files skipped).
    pub fn file_at(&self, offset: u64) -> Option<usize> {
        let i = self.files.partition_point(|f| f.offset + f.length <= offset);
        let f = self.files.get(i)?;
        (f.offset <= offset && offset < f.offset + f.length).then_some(i)
    }

    /// Bytes in a piece (0 out of range).
    pub fn piece_size(&self, piece: u32) -> u32 {
        let start = self.piece_offset(piece);
        if start >= self.total_length { return 0; }
        let end = if self.v2_aligned { self.file_at(start).map(|i| self.files[i].offset + self.files[i].length).unwrap_or(start) } else { self.total_length };
        (end - start).min(self.piece_length as u64) as u32
    }

    /// Pieces shorter than `piece_length` (the last one; in v2 each file's last one).
    pub fn short_pieces(&self) -> BTreeMap<u32, u32> {
        let mut m = BTreeMap::new();
        let mut check = |p: u32| { let s = self.piece_size(p); if s != self.piece_length { m.insert(p, s); } };
        if self.v2_aligned {
            for f in self.files.iter().filter(|f| f.length > 0) { check(((f.offset + f.length - 1) / self.piece_length as u64) as u32); }
        } else if self.num_pieces() > 0 {
            check(self.num_pieces() - 1);
        }
        m
    }

    /// The file ranges covering `len` bytes at `begin` within `piece`.
    pub fn map(&self, piece: u32, begin: u32, len: u32) -> Vec<Slice> {
        let mut out = Vec::new();
        let mut pos = self.piece_offset(piece) + begin as u64;
        let mut left = len as u64;
        let Some(mut i) = self.file_at(pos) else { return out };
        while left > 0 {
            let Some(f) = self.files.get(i) else { break };
            if f.length == 0 { i += 1; continue; }
            if pos < f.offset { break; } // a v2 gap: not part of any piece's data
            let n = (f.offset + f.length - pos).min(left);
            out.push(Slice { file: i, file_offset: pos - f.offset, len: n as u32, at: len - left as u32 });
            pos += n;
            left -= n;
            i += 1;
        }
        out
    }

    /// The pieces holding a file's bytes, inclusive, or `None` for an empty file.
    pub fn file_pieces(&self, file: usize) -> Option<(u32, u32)> {
        let f = self.files.get(file).filter(|f| f.length > 0)?;
        let pl = self.piece_length as u64;
        Some(((f.offset / pl) as u32, ((f.offset + f.length - 1) / pl) as u32))
    }

    /// Piece priorities from file priorities: a piece gets the highest priority of the
    /// (non-pad) files it touches.
    pub fn piece_priorities(&self, file_prio: &[u8]) -> Vec<u8> {
        let mut p = alloc::vec![0u8; self.num_pieces() as usize];
        for (i, f) in self.files.iter().enumerate() {
            if f.pad { continue; }
            let prio = file_prio.get(i).copied().unwrap_or(1).min(7);
            if let Some((a, b)) = self.file_pieces(i) { for x in &mut p[a as usize..=b as usize] { *x = (*x).max(prio); } }
        }
        p
    }
}

/// What the program does for us on disk.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DiskOp {
    Write { file: usize, offset: u64, data: Vec<u8> },
    /// Bytes of `piece` that belong to a deselected file, at `offset` within the piece.
    WritePart { piece: u32, offset: u32, data: Vec<u8> },
    /// Read `len` bytes; answer with [`Storage::on_read`]`(id, at, bytes)`.
    Read { id: u64, file: usize, offset: u64, len: u32, at: u32 },
    ReadPart { id: u64, piece: u32, offset: u32, len: u32, at: u32 },
    /// Create the file at its full size (or create it empty when `len` is 0).
    Preallocate { file: usize, len: u64 },
    /// Copy bytes of a piece from the part region into a file that was just selected.
    PromotePart { piece: u32, part_offset: u32, file: usize, file_offset: u64, len: u32 },
    /// The part region holds nothing more for this piece.
    DiscardPart { piece: u32 },
}

/// Files allocated up front, or grown as written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Allocation { Sparse, Full }

/// How a read started.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReadStart {
    /// No disk needed (all pad bytes).
    Ready(Vec<u8>),
    Pending { id: u64, ops: Vec<DiskOp> },
}

/// A block that arrived, as far as the cache is concerned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BlockDone {
    /// More blocks needed.
    Pending,
    /// The whole piece, ready to hash.
    Complete(Vec<u8>),
    /// Complete, but some blocks were written out early: they are being read back (the
    /// ops are in the returned list); the read with this id completes the piece.
    Reading(u64),
}

#[derive(Clone, Debug)]
struct Cached { blocks: Vec<Option<Vec<u8>>>, on_disk: Vec<bool>, touched: u64 }

#[derive(Clone, Debug)]
struct PendingRead { buf: Vec<u8>, left: u64 }

/// A torrent's storage state.
#[derive(Clone, Debug)]
pub struct Storage {
    pub layout: TorrentLayout,
    file_prio: Vec<u8>,
    /// (piece, file) pairs whose bytes are in the part region.
    part: BTreeSet<(u32, u32)>,
    cache: BTreeMap<u32, Cached>,
    cache_bytes: usize,
    max_cache: usize,
    clock: u64,
    next_id: u64,
    reads: BTreeMap<u64, PendingRead>,
}

impl Storage {
    /// `file_prio` has one entry per file (missing entries mean 1; values above 7 are 7).
    pub fn new(layout: TorrentLayout, file_prio: &[u8], max_cache: usize) -> Storage {
        let n = layout.files.len();
        let file_prio = (0..n).map(|i| file_prio.get(i).copied().unwrap_or(1).min(7)).collect();
        Storage { layout, file_prio, part: BTreeSet::new(), cache: BTreeMap::new(), cache_bytes: 0, max_cache, clock: 0, next_id: 1, reads: BTreeMap::new() }
    }

    pub fn file_priority(&self, file: usize) -> u8 { self.file_prio.get(file).copied().unwrap_or(0) }
    pub fn file_priorities(&self) -> &[u8] { &self.file_prio }
    pub fn piece_priorities(&self) -> Vec<u8> { self.layout.piece_priorities(&self.file_prio) }
    pub fn cache_bytes(&self) -> usize { self.cache_bytes }
    /// The (piece, file) pairs held in the part region (for resume data and tests).
    pub fn part_slices(&self) -> impl Iterator<Item = (u32, u32)> + '_ { self.part.iter().copied() }
    /// Restores part-region bookkeeping from resume data.
    pub fn set_part_slices(&mut self, it: impl IntoIterator<Item = (u32, u32)>) {
        for (p, f) in it { if p < self.layout.num_pieces() && (f as usize) < self.layout.files.len() { self.part.insert((p, f)); } }
    }

    /// Ops to create the files: with [`Allocation::Full`] every selected file at full
    /// size; in both modes empty selected files are created.
    pub fn allocate(&self, mode: Allocation) -> Vec<DiskOp> {
        self.layout.files.iter().enumerate()
            .filter(|(i, f)| !f.pad && self.file_prio[*i] > 0 && (mode == Allocation::Full || f.length == 0))
            .map(|(i, f)| DiskOp::Preallocate { file: i, len: f.length }).collect()
    }

    /// Changes a file's priority. Selecting a file that was deselected copies its bytes
    /// out of the part region.
    pub fn set_file_priority(&mut self, file: usize, prio: u8) -> Vec<DiskOp> {
        let mut ops = Vec::new();
        let Some(old) = self.file_prio.get(file).copied() else { return ops };
        let prio = prio.min(7);
        self.file_prio[file] = prio;
        if old == 0 && prio > 0 {
            if let Some(f) = self.layout.files.get(file) { if f.length == 0 { ops.push(DiskOp::Preallocate { file, len: 0 }); } }
            let pieces: Vec<u32> = self.part.iter().filter(|(_, f)| *f as usize == file).map(|(p, _)| *p).collect();
            for p in pieces {
                self.part.remove(&(p, file as u32));
                for s in self.layout.map(p, 0, self.layout.piece_size(p)).into_iter().filter(|s| s.file == file) {
                    ops.push(DiskOp::PromotePart { piece: p, part_offset: s.at, file, file_offset: s.file_offset, len: s.len });
                }
                if !self.part.iter().any(|(q, _)| *q == p) { ops.push(DiskOp::DiscardPart { piece: p }); }
            }
        }
        ops
    }

    /// Ops to write `data` at `begin` within `piece`, routed around deselected and pad files.
    pub fn write(&mut self, piece: u32, begin: u32, data: &[u8]) -> Vec<DiskOp> {
        let mut ops = Vec::new();
        for s in self.layout.map(piece, begin, data.len() as u32) {
            let f = &self.layout.files[s.file];
            if f.pad { continue; }
            let bytes = data[s.at as usize..(s.at + s.len) as usize].to_vec();
            if self.file_prio[s.file] == 0 {
                self.part.insert((piece, s.file as u32));
                ops.push(DiskOp::WritePart { piece, offset: begin + s.at, data: bytes });
            } else {
                ops.push(DiskOp::Write { file: s.file, offset: s.file_offset, data: bytes });
            }
        }
        ops
    }

    fn read_ops(&mut self, piece: u32, begin: u32, len: u32, id: u64, at0: u32, buf: &mut [u8]) -> (Vec<DiskOp>, u64) {
        let mut ops = Vec::new();
        let mut pending = 0u64;
        for s in self.layout.map(piece, begin, len) {
            let at = at0 + s.at;
            if self.layout.files[s.file].pad {
                if let Some(b) = buf.get_mut(at as usize..(at + s.len) as usize) { b.fill(0); }
                continue;
            }
            pending += s.len as u64;
            if self.part.contains(&(piece, s.file as u32)) {
                ops.push(DiskOp::ReadPart { id, piece, offset: begin + s.at, len: s.len, at });
            } else {
                ops.push(DiskOp::Read { id, file: s.file, offset: s.file_offset, len: s.len, at });
            }
        }
        (ops, pending)
    }

    /// Starts reading `len` bytes at `begin` of `piece`.
    pub fn read(&mut self, piece: u32, begin: u32, len: u32) -> ReadStart {
        let id = self.next_id;
        self.next_id += 1;
        let mut buf = alloc::vec![0u8; len as usize];
        let (ops, left) = self.read_ops(piece, begin, len, id, 0, &mut buf);
        if left == 0 { return ReadStart::Ready(buf); }
        self.reads.insert(id, PendingRead { buf, left });
        ReadStart::Pending { id, ops }
    }

    /// One read finished. Returns the whole buffer when all reads of `id` are in.
    /// Mismatched lengths or unknown ids are ignored.
    pub fn on_read(&mut self, id: u64, at: u32, data: &[u8]) -> Option<Vec<u8>> {
        let r = self.reads.get_mut(&id)?;
        let dst = r.buf.get_mut(at as usize..at as usize + data.len())?;
        dst.copy_from_slice(data);
        r.left = r.left.saturating_sub(data.len() as u64);
        if r.left > 0 { return None; }
        self.reads.remove(&id).map(|r| r.buf)
    }

    /// A read failed; forgets it. Returns whether it was pending.
    pub fn read_failed(&mut self, id: u64) -> bool { self.reads.remove(&id).is_some() }

    fn blocks_in(&self, piece: u32) -> usize { self.layout.piece_size(piece).div_ceil(BLOCK_SIZE) as usize }

    /// Puts a verified-to-be-requested block in the write cache. The returned ops (cache
    /// evictions, read-backs) must be run.
    pub fn add_block(&mut self, piece: u32, begin: u32, data: &[u8]) -> (Vec<DiskOp>, BlockDone) {
        let mut ops = Vec::new();
        let size = self.layout.piece_size(piece);
        let n = self.blocks_in(piece);
        let b = (begin / BLOCK_SIZE) as usize;
        if begin % BLOCK_SIZE != 0 || b >= n || data.len() as u32 != (size - begin).min(BLOCK_SIZE) { return (ops, BlockDone::Pending); }
        self.clock += 1;
        let clock = self.clock;
        let c = self.cache.entry(piece).or_insert_with(|| Cached { blocks: alloc::vec![None; n], on_disk: alloc::vec![false; n], touched: 0 });
        c.touched = clock;
        if c.blocks[b].is_none() && !c.on_disk[b] { c.blocks[b] = Some(data.to_vec()); self.cache_bytes += data.len(); }
        let complete = c.blocks.iter().zip(&c.on_disk).all(|(m, d)| m.is_some() || *d);
        if complete {
            let Some(c) = self.cache.remove(&piece) else { return (ops, BlockDone::Pending) };
            let mut buf = alloc::vec![0u8; size as usize];
            let mut need = Vec::new();
            for (i, blk) in c.blocks.into_iter().enumerate() {
                let at = i * BLOCK_SIZE as usize;
                match blk {
                    Some(d) => { self.cache_bytes -= d.len(); buf[at..at + d.len()].copy_from_slice(&d); }
                    None => need.push(i as u32),
                }
            }
            if need.is_empty() { return (ops, BlockDone::Complete(buf)); }
            let id = self.next_id;
            self.next_id += 1;
            let mut left = 0;
            for i in need {
                let begin = i * BLOCK_SIZE;
                let len = (size - begin).min(BLOCK_SIZE);
                let (o, l) = self.read_ops(piece, begin, len, id, begin, &mut buf);
                ops.extend(o);
                left += l;
            }
            if left == 0 { return (ops, BlockDone::Complete(buf)); }
            self.reads.insert(id, PendingRead { buf, left });
            return (ops, BlockDone::Reading(id));
        }
        self.evict(self.max_cache, Some(piece), &mut ops);
        (ops, BlockDone::Pending)
    }

    /// Writes out least recently touched pieces (`keep` last) until under `budget`.
    fn evict(&mut self, budget: usize, keep: Option<u32>, ops: &mut Vec<DiskOp>) {
        while self.cache_bytes > budget {
            let victim = self.cache.iter().filter(|(p, c)| Some(**p) != keep && c.blocks.iter().any(|b| b.is_some())).min_by_key(|(_, c)| c.touched).map(|(p, _)| *p)
                .or_else(|| keep.filter(|k| self.cache.get(k).is_some_and(|c| c.blocks.iter().any(|b| b.is_some()))));
            let Some(p) = victim else { break };
            let blocks: Vec<(usize, Vec<u8>)> = match self.cache.get_mut(&p) {
                Some(c) => c.blocks.iter_mut().enumerate().filter_map(|(i, b)| b.take().map(|d| { c.on_disk[i] = true; (i, d) })).collect(),
                None => break,
            };
            for (i, d) in blocks {
                self.cache_bytes -= d.len();
                ops.extend(self.write(p, i as u32 * BLOCK_SIZE, &d));
            }
        }
    }

    /// Writes every cached block out (before saving resume data or shutting down); the
    /// blocks stay known as written, so their pieces still complete.
    pub fn flush(&mut self) -> Vec<DiskOp> { let mut ops = Vec::new(); self.evict(0, None, &mut ops); ops }

    /// Marks a block as already on disk (from resume data after a [`flush`](Self::flush)).
    pub fn restore_block(&mut self, piece: u32, block: u32) {
        let n = self.blocks_in(piece);
        if block as usize >= n { return; }
        let c = self.cache.entry(piece).or_insert_with(|| Cached { blocks: alloc::vec![None; n], on_disk: alloc::vec![false; n], touched: 0 });
        c.on_disk[block as usize] = true;
    }

    /// Drops a piece from the cache (it failed its hash, or is no longer wanted).
    pub fn discard(&mut self, piece: u32) {
        if let Some(c) = self.cache.remove(&piece) { self.cache_bytes -= c.blocks.iter().flatten().map(|b| b.len()).sum::<usize>(); }
    }

    /// Whether the piece is known to have bytes in the part region.
    pub fn piece_in_part(&self, piece: u32) -> bool { self.part.range((piece, 0)..=(piece, u32::MAX)).next().is_some() }

    /// Number of files.
    pub fn num_files(&self) -> usize { self.layout.files.len() }

    /// A helper for the session: pieces of `have` touching file `file`.
    pub fn file_have(&self, file: usize, have: &Bitfield) -> u64 {
        let Some((a, b)) = self.layout.file_pieces(file) else { return 0 };
        (a..=b).filter(|p| have.get(*p)).count() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn p(s: &str) -> Vec<String> { vec![String::from(s)] }

    fn three() -> TorrentLayout {
        // 10 + 30 + 25 bytes, pieces of 16: files straddle pieces.
        TorrentLayout::new(16, vec![(p("a"), 10, false), (p("b"), 30, false), (p("c"), 25, false)], false).unwrap()
    }

    #[test]
    fn layout_math() {
        let l = three();
        assert_eq!(l.total_length, 65);
        assert_eq!(l.num_pieces(), 5);
        assert_eq!(l.piece_size(4), 1);
        assert_eq!(l.piece_size(5), 0);
        assert_eq!(l.short_pieces(), BTreeMap::from([(4, 1)]));
        assert_eq!(l.map(0, 0, 16), vec![Slice { file: 0, file_offset: 0, len: 10, at: 0 }, Slice { file: 1, file_offset: 0, len: 6, at: 10 }]);
        assert_eq!(l.map(2, 4, 12), vec![Slice { file: 1, file_offset: 26, len: 4, at: 0 }, Slice { file: 2, file_offset: 0, len: 8, at: 4 }]);
        assert_eq!(l.file_pieces(1), Some((0, 2)));
        assert_eq!(l.piece_priorities(&[0, 0, 3]), vec![0, 0, 3, 3, 3]);
        assert_eq!(l.piece_priorities(&[5, 0, 0]), vec![5, 0, 0, 0, 0]);
        assert!(l.map(9, 0, 16).is_empty());
    }

    #[test]
    fn empty_and_pad_files() {
        let l = TorrentLayout::new(16, vec![(p("e"), 0, false), (p("a"), 10, false), (p(".pad"), 6, true), (p("b"), 16, false), (p("z"), 0, false)], false).unwrap();
        assert_eq!(l.num_pieces(), 2);
        assert_eq!(l.file_at(0), Some(1));
        assert_eq!(l.file_pieces(0), None);
        let mut s = Storage::new(l, &[], 1 << 20);
        let ops = s.write(0, 0, &[1; 16]);
        assert_eq!(ops, vec![DiskOp::Write { file: 1, offset: 0, data: vec![1; 10] }], "pad bytes never written");
        assert_eq!(s.allocate(Allocation::Sparse), vec![DiskOp::Preallocate { file: 0, len: 0 }, DiskOp::Preallocate { file: 4, len: 0 }]);
        assert_eq!(s.allocate(Allocation::Full).len(), 4);
        match s.read(0, 0, 16) {
            ReadStart::Pending { id, ops } => {
                assert_eq!(ops, vec![DiskOp::Read { id, file: 1, offset: 0, len: 10, at: 0 }]);
                assert_eq!(s.on_read(id, 0, &[7; 10]), Some([vec![7; 10], vec![0; 6]].concat()));
            }
            r => panic!("{r:?}"),
        }
        assert_eq!(s.read(0, 10, 6), ReadStart::Ready(vec![0; 6]));
    }

    #[test]
    fn v2_alignment() {
        let l = TorrentLayout::new(16, vec![(p("a"), 20, false), (p("b"), 5, false), (p("c"), 16, false)], true).unwrap();
        assert_eq!(l.files[1].offset, 32);
        assert_eq!(l.files[2].offset, 48);
        assert_eq!(l.num_pieces(), 4);
        assert_eq!((l.piece_size(0), l.piece_size(1), l.piece_size(2), l.piece_size(3)), (16, 4, 5, 16));
        assert_eq!(l.short_pieces(), BTreeMap::from([(1, 4), (2, 5)]));
        assert_eq!(l.map(1, 0, 4), vec![Slice { file: 0, file_offset: 16, len: 4, at: 0 }]);
        let mut bad = l.clone();
        bad.files[1].offset = 33;
        assert_eq!(bad.validate(), Err(LayoutError::Overlap));
    }

    #[test]
    fn hostile_layouts() {
        assert_eq!(TorrentLayout::new(0, vec![(p("a"), 1, false)], false), Err(LayoutError::BadPieceLength));
        assert_eq!(TorrentLayout::new(16, vec![], false), Err(LayoutError::NoFiles));
        assert_eq!(TorrentLayout::new(16, vec![(p("a"), u64::MAX, false), (p("b"), 2, false)], false), Err(LayoutError::TooLarge));
        assert_eq!(TorrentLayout::new(1, vec![(p("a"), 1 << 40, false)], false), Err(LayoutError::TooManyPieces));
        let mut l = three();
        l.files[1].offset = 5;
        assert_eq!(l.validate(), Err(LayoutError::Overlap));
    }

    #[test]
    fn deselected_files_go_to_part_region() {
        let mut s = Storage::new(three(), &[1, 0, 1], 1 << 20);
        let ops = s.write(0, 0, &[9; 16]);
        assert_eq!(ops, vec![DiskOp::Write { file: 0, offset: 0, data: vec![9; 10] }, DiskOp::WritePart { piece: 0, offset: 10, data: vec![9; 6] }]);
        assert!(s.piece_in_part(0));
        // Reading back takes the part bytes from the part region.
        let ReadStart::Pending { id, ops } = s.read(0, 8, 4) else { panic!() };
        assert_eq!(ops, vec![DiskOp::Read { id, file: 0, offset: 8, len: 2, at: 0 }, DiskOp::ReadPart { id, piece: 0, offset: 10, len: 2, at: 2 }]);
        assert_eq!(s.on_read(id, 2, &[2, 2]), None);
        assert_eq!(s.on_read(id, 0, &[1, 1]), Some(vec![1, 1, 2, 2]));
        // Selecting the file promotes the bytes.
        let ops = s.set_file_priority(1, 4);
        assert_eq!(ops, vec![DiskOp::PromotePart { piece: 0, part_offset: 10, file: 1, file_offset: 0, len: 6 }, DiskOp::DiscardPart { piece: 0 }]);
        assert!(!s.piece_in_part(0));
        let ReadStart::Pending { ops, .. } = s.read(0, 10, 6) else { panic!() };
        assert!(matches!(ops[0], DiskOp::Read { file: 1, .. }));
    }

    #[test]
    fn cache_coalesces_and_evicts() {
        let l = TorrentLayout::single("x", 3 * 16384 + 100, 32768).unwrap();
        let mut s = Storage::new(l, &[1], 20_000);
        let (ops, d) = s.add_block(0, 0, &[1; 16384]);
        assert!(ops.is_empty() && d == BlockDone::Pending);
        assert_eq!(s.cache_bytes(), 16384);
        // A second piece's block pushes the cache over budget: piece 0 is written early.
        let (ops, d) = s.add_block(1, 0, &[2; 16384]);
        assert_eq!(d, BlockDone::Pending);
        assert_eq!(ops, vec![DiskOp::Write { file: 0, offset: 0, data: vec![1; 16384] }]);
        // Piece 1 completes from memory.
        let (ops, d) = s.add_block(1, 16384, &[3; 100]);
        assert!(ops.is_empty());
        assert_eq!(d, BlockDone::Complete([vec![2; 16384], vec![3; 100]].concat()));
        // Piece 0: completing reads the written block back.
        let mut s = Storage::new(TorrentLayout::single("x", 3 * 16384 + 100, 32768).unwrap(), &[1], 20_000);
        s.add_block(0, 0, &[1; 16384]);
        s.add_block(1, 0, &[2; 16384]);
        let (ops, d) = s.add_block(0, 16384, &[4; 16384]);
        let BlockDone::Reading(id) = d else { panic!("{d:?}") };
        assert_eq!(ops, vec![DiskOp::Read { id, file: 0, offset: 0, len: 16384, at: 0 }]);
        let full = s.on_read(id, 0, &[1; 16384]).unwrap();
        assert_eq!(full, [vec![1; 16384], vec![4; 16384]].concat());
        // Bad blocks are ignored; a whole piece in memory completes at once.
        let mut s = Storage::new(TorrentLayout::single("x", 100, 32768).unwrap(), &[1], 1 << 20);
        assert_eq!(s.add_block(0, 0, &[1; 99]).1, BlockDone::Pending);
        assert_eq!(s.add_block(0, 5, &[1; 100]).1, BlockDone::Pending);
        assert_eq!(s.add_block(7, 0, &[1; 100]).1, BlockDone::Pending);
        assert_eq!(s.add_block(0, 0, &[1; 100]).1, BlockDone::Complete(vec![1; 100]));
        assert_eq!(s.cache_bytes(), 0);
        s.add_block(0, 0, &[1; 50]);
        s.discard(0);
        assert_eq!(s.cache_bytes(), 0);
        assert_eq!(s.on_read(999, 0, &[1]), None);
        // Flush writes everything; restored blocks are read back on completion.
        let mut s = Storage::new(TorrentLayout::single("x", 2 * 16384, 32768).unwrap(), &[1], 1 << 20);
        s.add_block(0, 0, &[5; 16384]);
        assert_eq!(s.flush(), vec![DiskOp::Write { file: 0, offset: 0, data: vec![5; 16384] }]);
        let mut t = Storage::new(TorrentLayout::single("x", 2 * 16384, 32768).unwrap(), &[1], 1 << 20);
        t.restore_block(0, 0);
        t.restore_block(0, 9);
        assert!(matches!(t.add_block(0, 16384, &[6; 16384]).1, BlockDone::Reading(_)));
        assert!(!s.read_failed(999));
    }
}
