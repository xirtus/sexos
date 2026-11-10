// SPDX-License-Identifier: GPL-3.0-only
//! A stream read over HTTP in Range pieces, the way a browser streams video: the
//! bytes kept so far, which piece to ask for next, and how many bytes can be read
//! now. No I/O here; the caller runs the requests.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// `Content-Range: bytes FIRST-LAST/TOTAL` (TOTAL may be `*`).
pub fn content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes")?.trim_start();
    let (span, total) = rest.split_once('/')?;
    let (first, last) = span.trim().split_once('-')?;
    let (first, last): (u64, u64) = (first.trim().parse().ok()?, last.trim().parse().ok()?);
    let total = match total.trim() { "*" => None, t => Some(t.parse().ok()?) };
    if last < first || total.is_some_and(|t| last >= t) { return None; }
    Some((first, last, total))
}

/// The `Range` header value for piece `index`.
pub fn range_for(piece: u64, index: u64, len: Option<u64>) -> alloc::string::String {
    let first = index * piece;
    let mut last = first + piece - 1;
    if let Some(len) = len { last = last.min(len.saturating_sub(1)); }
    alloc::format!("bytes={}-{}", first, last)
}

/// Pieces of one stream, each `piece` bytes long (the last may be shorter), kept up
/// to about `limit` bytes; the ones furthest from where reading is go first.
pub struct Pieces {
    pub piece: u64,
    len: Option<u64>,
    have: BTreeMap<u64, Vec<u8>>,
    held: usize,
    limit: usize,
}

impl Pieces {
    pub fn new(piece: u64, limit: usize) -> Pieces { Pieces { piece: piece.max(1), len: None, have: BTreeMap::new(), held: 0, limit } }
    pub fn len(&self) -> Option<u64> { self.len }
    pub fn set_len(&mut self, len: u64) { self.len = Some(len); }
    pub fn count(&self) -> Option<u64> { self.len.map(|l| l.div_ceil(self.piece)) }
    pub fn held(&self) -> usize { self.held }
    pub fn has(&self, index: u64) -> bool { self.have.contains_key(&index) }
    pub fn index_of(&self, at: u64) -> u64 { at / self.piece }
    /// Whether `bytes` is the right size for piece `index`.
    pub fn fits(&self, index: u64, bytes: usize) -> bool {
        let want = match self.len { Some(len) => self.piece.min(len.saturating_sub(index * self.piece)), None => self.piece };
        bytes as u64 == want || (self.len.is_none() && (bytes as u64) < want && bytes > 0)
    }
    /// Keeps piece `index`, then lets go of far pieces while over the limit (never
    /// the one holding `reading`).
    pub fn insert(&mut self, index: u64, bytes: Vec<u8>, reading: u64) {
        self.held += bytes.len();
        if let Some(old) = self.have.insert(index, bytes) { self.held -= old.len(); }
        let here = self.index_of(reading);
        while self.held > self.limit && self.have.len() > 1 {
            // Furthest first; behind counts double, since playback seldom goes back.
            let far = self.have.keys().copied().filter(|k| *k != here)
                .max_by_key(|k| if *k < here { (here - k) * 2 } else { k - here });
            let Some(far) = far else { break };
            if let Some(gone) = self.have.remove(&far) { self.held -= gone.len(); }
        }
    }
    /// Copies bytes at `at` into `out`; 0 when that piece isn't here.
    pub fn read_at(&self, at: u64, out: &mut [u8]) -> usize {
        let index = self.index_of(at);
        let Some(bytes) = self.have.get(&index) else { return 0 };
        let off = (at - index * self.piece) as usize;
        if off >= bytes.len() { return 0; }
        let n = out.len().min(bytes.len() - off);
        out[..n].copy_from_slice(&bytes[off..off + n]);
        n
    }
    /// Bytes readable from `at` without a request; `u64::MAX` when they run to the end.
    pub fn available(&self, at: u64) -> u64 {
        if self.len.is_some_and(|l| at >= l) { return u64::MAX; }
        let mut index = self.index_of(at);
        let mut have = 0u64;
        let mut from = at - index * self.piece;
        while let Some(bytes) = self.have.get(&index) {
            have += (bytes.len() as u64).saturating_sub(from);
            from = 0;
            index += 1;
            if self.count().is_some_and(|c| index >= c) { return u64::MAX; }
            if (bytes.len() as u64) < self.piece { break; }
        }
        have
    }
    /// The next piece to ask for: the first one missing between `at` and `ahead`
    /// bytes after it, and not among `asked`.
    pub fn wanted(&self, at: u64, ahead: u64, asked: &[u64]) -> Option<u64> {
        let first = self.index_of(at);
        let mut last = self.index_of(at.saturating_add(ahead.max(1)) - 1);
        if let Some(count) = self.count() { if count == 0 { return None; } last = last.min(count - 1); }
        (first..=last).find(|k| !self.have.contains_key(k) && !asked.contains(k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn content_range_parses_and_refuses_nonsense() {
        assert_eq!(content_range("bytes 0-99/1000"), Some((0, 99, Some(1000))));
        assert_eq!(content_range("bytes 5-9/*"), Some((5, 9, None)));
        assert_eq!(content_range("bytes 9-5/100"), None);
        assert_eq!(content_range("bytes 0-100/100"), None);
        assert_eq!(content_range("items 0-1/2"), None);
        assert_eq!(range_for(100, 2, Some(250)), "bytes=200-249");
        assert_eq!(range_for(100, 0, None), "bytes=0-99");
    }

    #[test]
    fn reads_across_pieces_and_counts_what_is_here() {
        let mut p = Pieces::new(4, 1 << 20);
        p.set_len(10);
        assert_eq!(p.available(0), 0);
        p.insert(0, vec![0, 1, 2, 3], 0);
        p.insert(1, vec![4, 5, 6, 7], 0);
        assert_eq!(p.available(1), 7);
        let mut out = [0u8; 8];
        assert_eq!(p.read_at(2, &mut out), 2);
        assert_eq!(&out[..2], &[2, 3]);
        assert_eq!(p.read_at(9, &mut out), 0);
        assert!(!p.fits(2, 4) && p.fits(2, 2));
        p.insert(2, vec![8, 9], 0);
        assert_eq!(p.available(1), u64::MAX);
        assert_eq!(p.available(10), u64::MAX);
    }

    #[test]
    fn asks_for_missing_pieces_ahead_only() {
        let mut p = Pieces::new(10, 1 << 20);
        p.set_len(55);
        p.insert(1, vec![0; 10], 0);
        assert_eq!(p.wanted(0, 30, &[]), Some(0));
        assert_eq!(p.wanted(0, 30, &[0]), Some(2));
        assert_eq!(p.wanted(0, 30, &[0, 2]), None);
        assert_eq!(p.wanted(40, 1000, &[]), Some(4));
        assert_eq!(p.wanted(50, 1000, &[5]), None);
    }

    #[test]
    fn lets_go_of_far_pieces_first() {
        let mut p = Pieces::new(10, 30);
        for k in 0..4 { p.insert(k, vec![k as u8; 10], 20); }
        // Over by one piece: 0 is two behind (counts 4), 3 is one ahead.
        assert!(!p.has(0) && p.has(1) && p.has(2) && p.has(3));
        assert_eq!(p.held(), 30);
    }
}
