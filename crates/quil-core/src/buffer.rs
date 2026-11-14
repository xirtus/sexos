// SPDX-License-Identifier: GPL-3.0-only
//! The document: a piece table over a [`Source`] (the file as it was opened, read lazily
//! in blocks) and an append-only buffer of everything typed since. Nothing is copied when
//! a file opens, so a 1 GB file opens as fast as a 1 KB one; an edit costs a few pieces,
//! not a copy of the text. Line numbers come from a per-block newline index that fills in
//! as blocks are read, or step by step while the editor is idle.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ops::Range;

/// Where the original text lives: memory, or a file read on demand.
pub trait Source {
    fn len(&self) -> usize;
    /// Fills as much of `out` as there is from byte `at`; returns how much.
    fn read(&self, at: usize, out: &mut [u8]) -> usize;
}

/// Text already in memory.
pub struct MemSource(pub Vec<u8>);
impl Source for MemSource {
    fn len(&self) -> usize { self.0.len() }
    fn read(&self, at: usize, out: &mut [u8]) -> usize {
        let src = self.0.get(at..).unwrap_or(&[]);
        let n = src.len().min(out.len());
        out[..n].copy_from_slice(&src[..n]);
        n
    }
}

/// Original-file block size: the unit of reading, caching and line indexing.
pub const BLOCK: usize = 64 * 1024;
/// Blocks kept in memory (4 MiB).
const CACHED: usize = 64;
const UNKNOWN: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind { Orig, Add }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Piece { kind: Kind, start: usize, len: usize }

/// The piece list at one moment; restoring it restores the text exactly, because
/// neither the source nor the add buffer ever changes what is already in them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot { pieces: Vec<Piece>, len: usize }

struct Cache { blocks: Vec<(usize, u64, Vec<u8>)>, tick: u64 }

pub struct Buffer {
    source: Box<dyn Source>,
    add: Vec<u8>,
    pieces: Vec<Piece>,
    /// Start offset of each piece in the document.
    starts: Vec<usize>,
    len: usize,
    cache: RefCell<Cache>,
    /// Newlines in each block of the source, or UNKNOWN.
    index: RefCell<Vec<u32>>,
    /// The first block whose count might still be unknown.
    indexed_to: RefCell<usize>,
}

impl Buffer {
    pub fn new(source: Box<dyn Source>) -> Buffer {
        let n = source.len();
        let blocks = n.div_ceil(BLOCK);
        let pieces = if n == 0 { vec![] } else { vec![Piece { kind: Kind::Orig, start: 0, len: n }] };
        let mut b = Buffer {
            source, add: Vec::new(), pieces, starts: Vec::new(), len: n,
            cache: RefCell::new(Cache { blocks: Vec::new(), tick: 0 }),
            index: RefCell::new(vec![UNKNOWN; blocks]), indexed_to: RefCell::new(0),
        };
        b.restart();
        b
    }
    pub fn from_bytes(text: &[u8]) -> Buffer { Buffer::new(Box::new(MemSource(text.to_vec()))) }
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub fn pieces(&self) -> usize { self.pieces.len() }

    fn restart(&mut self) {
        self.starts.clear();
        let mut at = 0;
        for p in &self.pieces { self.starts.push(at); at += p.len; }
        self.len = at;
    }

    pub fn snapshot(&self) -> Snapshot { Snapshot { pieces: self.pieces.clone(), len: self.len } }
    pub fn restore(&mut self, s: &Snapshot) { self.pieces = s.pieces.clone(); self.restart(); }

    // ---- Reading ----

    fn block(&self, n: usize, f: &mut dyn FnMut(&[u8])) {
        let mut c = self.cache.borrow_mut();
        c.tick += 1;
        let tick = c.tick;
        if let Some(e) = c.blocks.iter_mut().find(|e| e.0 == n) { e.1 = tick; f(&e.2); return; }
        let mut data = vec![0u8; BLOCK.min(self.source.len() - n * BLOCK)];
        let got = self.source.read(n * BLOCK, &mut data);
        data.truncate(got);
        // Reading a block is the moment to count its lines.
        let mut idx = self.index.borrow_mut();
        if idx[n] == UNKNOWN { idx[n] = memchr::memchr_iter(b'\n', &data).count() as u32; }
        f(&data);
        if c.blocks.len() < CACHED { c.blocks.push((n, tick, data)); } else {
            let old = c.blocks.iter_mut().min_by_key(|e| e.1).unwrap();
            *old = (n, tick, data);
        }
    }
    fn read_orig(&self, at: usize, out: &mut [u8]) {
        let mut done = 0;
        while done < out.len() {
            let pos = at + done;
            let (n, off) = (pos / BLOCK, pos % BLOCK);
            let mut took = 0;
            self.block(n, &mut |d| { let k = (d.len() - off.min(d.len())).min(out.len() - done); out[done..done + k].copy_from_slice(&d[off..off + k]); took = k; });
            if took == 0 { out[done..].fill(0); return; }
            done += took;
        }
    }
    fn piece_at(&self, at: usize) -> usize { self.starts.partition_point(|s| *s <= at).saturating_sub(1) }

    /// Copies bytes from `at` into `out`; returns how many (fewer at the end).
    pub fn read(&self, at: usize, out: &mut [u8]) -> usize {
        if at >= self.len { return 0; }
        let want = out.len().min(self.len - at);
        let mut done = 0;
        let mut i = self.piece_at(at);
        while done < want {
            let p = self.pieces[i];
            let off = at + done - self.starts[i];
            let k = (p.len - off).min(want - done);
            match p.kind {
                Kind::Add => out[done..done + k].copy_from_slice(&self.add[p.start + off..p.start + off + k]),
                Kind::Orig => self.read_orig(p.start + off, &mut out[done..done + k]),
            }
            done += k;
            i += 1;
        }
        done
    }
    pub fn slice(&self, r: Range<usize>) -> Vec<u8> {
        let (a, b) = (r.start.min(self.len), r.end.min(self.len).max(r.start.min(self.len)));
        let mut v = vec![0u8; b - a];
        self.read(a, &mut v);
        v
    }
    /// The whole text. For tests and small files; large files stream with [`Buffer::chunks`].
    pub fn to_vec(&self) -> Vec<u8> { self.slice(0..self.len) }
    pub fn byte(&self, at: usize) -> Option<u8> { let mut b = [0u8]; (self.read(at, &mut b) == 1).then_some(b[0]) }
    /// Calls `f` with the text in order, a chunk at a time (for saving and hashing).
    pub fn chunks<E>(&self, mut f: impl FnMut(&[u8]) -> Result<(), E>) -> Result<(), E> {
        let mut buf = vec![0u8; BLOCK];
        let mut at = 0;
        while at < self.len { let n = self.read(at, &mut buf); f(&buf[..n])?; at += n; }
        Ok(())
    }

    // ---- Editing ----

    /// Cuts the piece list at `at` so a piece starts there; returns that piece's index.
    fn split(&mut self, at: usize) -> usize {
        if at >= self.len { return self.pieces.len(); }
        let i = self.piece_at(at);
        let off = at - self.starts[i];
        if off == 0 { return i; }
        let p = self.pieces[i];
        self.pieces[i].len = off;
        self.pieces.insert(i + 1, Piece { kind: p.kind, start: p.start + off, len: p.len - off });
        self.starts.insert(i + 1, at);
        i + 1
    }
    /// Replaces `range` with `text`. The one primitive every edit goes through.
    pub fn replace(&mut self, range: Range<usize>, text: &[u8]) {
        let (a, b) = (range.start.min(self.len), range.end.min(self.len).max(range.start.min(self.len)));
        let first = self.split(a);
        let last = self.split(b);
        self.pieces.drain(first..last);
        if !text.is_empty() {
            // Typing at the end of the last typed piece grows it instead of adding one.
            let extend = first > 0 && { let p = self.pieces[first - 1]; p.kind == Kind::Add && p.start + p.len == self.add.len() };
            if extend { self.pieces[first - 1].len += text.len(); } else {
                self.pieces.insert(first, Piece { kind: Kind::Add, start: self.add.len(), len: text.len() });
            }
            self.add.extend_from_slice(text);
        }
        self.restart();
    }
    pub fn insert(&mut self, at: usize, text: &[u8]) { self.replace(at..at, text); }
    pub fn delete(&mut self, r: Range<usize>) { self.replace(r, b""); }

    // ---- Characters and lines ----

    /// The start of the character after the one at `at`.
    pub fn next_char(&self, at: usize) -> usize {
        if at >= self.len { return self.len; }
        let mut b = [0u8; 4];
        let n = self.read(at, &mut b);
        let w = match b[0] { 0..=0x7F => 1, 0xC0..=0xDF => 2, 0xE0..=0xEF => 3, 0xF0..=0xF7 => 4, _ => 1 };
        let mut k = 1;
        while k < w && k < n && b[k] & 0xC0 == 0x80 { k += 1; }
        at + k
    }
    /// The start of the character before `at`.
    pub fn prev_char(&self, at: usize) -> usize {
        let at = at.min(self.len);
        if at == 0 { return 0; }
        let from = at.saturating_sub(4);
        let mut b = [0u8; 4];
        let n = self.read(from, &mut b[..at - from]);
        let mut i = n;
        while i > 1 { i -= 1; if b[i] & 0xC0 != 0x80 { return from + i; } }
        // All continuation bytes back to `from`; only valid if from+0 starts it.
        if at - from == 4 && b[0] & 0xC0 == 0x80 { at - 1 } else { from }
    }
    pub fn char_at(&self, at: usize) -> Option<char> {
        let end = self.next_char(at);
        if end == at { return None; }
        let b = self.slice(at..end);
        core::str::from_utf8(&b).ok().and_then(|s| s.chars().next()).or(Some(char::REPLACEMENT_CHARACTER))
    }
    pub fn line_start(&self, at: usize) -> usize {
        let mut end = at.min(self.len);
        let mut buf = [0u8; 4096];
        while end > 0 {
            let from = end.saturating_sub(buf.len());
            let n = self.read(from, &mut buf[..end - from]);
            if let Some(i) = memchr::memrchr(b'\n', &buf[..n]) { return from + i + 1; }
            end = from;
        }
        0
    }
    pub fn line_end(&self, at: usize) -> usize {
        let mut from = at.min(self.len);
        let mut buf = [0u8; 4096];
        while from < self.len {
            let n = self.read(from, &mut buf);
            if let Some(i) = memchr::memchr(b'\n', &buf[..n]) { return from + i; }
            from += n;
        }
        self.len
    }
    /// The start of the next line, or the end of the text on the last line.
    pub fn next_line(&self, at: usize) -> usize { let e = self.line_end(at); if e < self.len { e + 1 } else { e } }
    pub fn prev_line(&self, at: usize) -> Option<usize> { let s = self.line_start(at); (s > 0).then(|| self.line_start(s - 1)) }
    /// Characters from the line start to `at`.
    pub fn column(&self, at: usize) -> usize {
        let s = self.line_start(at);
        let b = self.slice(s..at);
        b.iter().filter(|c| **c & 0xC0 != 0x80).count()
    }
    /// The offset `col` characters into the line starting at `start` (or its end).
    pub fn at_column(&self, start: usize, col: usize) -> usize {
        let end = self.line_end(start);
        let mut at = start;
        for _ in 0..col { if at >= end { break; } at = self.next_char(at); }
        at
    }

    // ---- Line index ----

    fn count(&self, r: Range<usize>) -> Option<usize> {
        // Newlines in a range of the source: whole blocks from the index, ends by reading.
        let mut total = 0;
        let mut at = r.start;
        while at < r.end {
            let n = at / BLOCK;
            let block_end = ((n + 1) * BLOCK).min(r.end);
            if at % BLOCK == 0 && block_end == (n + 1) * BLOCK {
                let c = self.index.borrow()[n];
                if c == UNKNOWN { return None; }
                total += c as usize;
            } else {
                let mut buf = vec![0u8; block_end - at];
                self.read_orig(at, &mut buf);
                total += memchr::memchr_iter(b'\n', &buf).count();
            }
            at = block_end;
        }
        Some(total)
    }
    fn piece_lines(&self, p: &Piece, upto: usize) -> Option<usize> {
        match p.kind {
            Kind::Add => Some(memchr::memchr_iter(b'\n', &self.add[p.start..p.start + upto]).count()),
            Kind::Orig => self.count(p.start..p.start + upto),
        }
    }
    /// The zero-based line of `at`, if the index covers it yet.
    pub fn line_of(&self, at: usize) -> Option<usize> {
        let at = at.min(self.len);
        let mut total = 0;
        for (i, p) in self.pieces.iter().enumerate() {
            let s = self.starts[i];
            if s >= at { break; }
            total += self.piece_lines(p, (at - s).min(p.len))?;
        }
        Some(total)
    }
    pub fn line_count(&self) -> Option<usize> { self.line_of(self.len).map(|n| n + 1) }
    /// Where line `n` (zero-based) starts, if the index covers it; the end of the text past
    /// the last line.
    pub fn offset_of_line(&self, n: usize) -> Option<usize> {
        if n == 0 { return Some(0); }
        let mut seen = 0;
        for (i, p) in self.pieces.iter().enumerate() {
            let here = self.piece_lines(p, p.len)?;
            if seen + here >= n {
                // The line starts inside this piece: walk it.
                let mut at = self.starts[i];
                let end = at + p.len;
                while seen < n && at < end {
                    let e = self.line_end(at);
                    if e >= end { break; }
                    seen += 1;
                    at = e + 1;
                }
                if seen == n { return Some(at); }
            }
            seen += here;
        }
        Some(self.len)
    }
    /// Counts up to `blocks` more blocks of the source; true once every block is counted.
    pub fn index_step(&self, blocks: usize) -> bool {
        let total = self.index.borrow().len();
        let mut done = 0;
        let mut at = *self.indexed_to.borrow();
        while at < total && done < blocks {
            if self.index.borrow()[at] == UNKNOWN {
                // Count without caching: the idle pass must not evict what's on screen.
                let mut data = vec![0u8; BLOCK.min(self.source.len() - at * BLOCK)];
                let got = self.source.read(at * BLOCK, &mut data);
                self.index.borrow_mut()[at] = memchr::memchr_iter(b'\n', &data[..got]).count() as u32;
                done += 1;
            }
            at += 1;
        }
        *self.indexed_to.borrow_mut() = at;
        at >= total
    }
    pub fn indexed(&self) -> bool { self.index.borrow().iter().all(|c| *c != UNKNOWN) }

    // ---- Search ----

    /// The first match of `needle` at or after `from`, case-insensitively for ASCII when
    /// asked. Reads in overlapping chunks, so it works on files of any size.
    pub fn find(&self, needle: &[u8], from: usize, fold: bool) -> Option<usize> {
        if needle.is_empty() || needle.len() > self.len { return None; }
        let lower: Vec<u8> = needle.iter().map(|b| if fold { b.to_ascii_lowercase() } else { *b }).collect();
        let finder = memchr::memmem::Finder::new(&lower);
        let chunk = BLOCK.max(needle.len() * 2);
        let mut buf = vec![0u8; chunk];
        let mut at = from;
        while at < self.len {
            let n = self.read(at, &mut buf);
            if fold { buf[..n].make_ascii_lowercase(); }
            if let Some(i) = finder.find(&buf[..n]) { return Some(at + i); }
            if at + n >= self.len { break; }
            at += n + 1 - needle.len();
        }
        None
    }
    /// The last match of `needle` that starts before `before`.
    pub fn rfind(&self, needle: &[u8], before: usize, fold: bool) -> Option<usize> {
        if needle.is_empty() || needle.len() > self.len { return None; }
        let lower: Vec<u8> = needle.iter().map(|b| if fold { b.to_ascii_lowercase() } else { *b }).collect();
        let finder = memchr::memmem::FinderRev::new(&lower);
        let chunk = BLOCK.max(needle.len() * 2);
        let mut buf = vec![0u8; chunk];
        // Matches must start before `before`, so they end before before+len-1.
        let mut end = (before + needle.len() - 1).min(self.len);
        while end >= needle.len() {
            let from = end.saturating_sub(chunk);
            let n = self.read(from, &mut buf[..end - from]);
            if fold { buf[..n].make_ascii_lowercase(); }
            if let Some(i) = finder.rfind(&buf[..n]) { return Some(from + i); }
            if from == 0 { break; }
            end = from + needle.len() - 1;
        }
        None
    }
}
