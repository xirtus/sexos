// SPDX-License-Identifier: GPL-3.0-only
//! Portable source buffering, shared by programs and standalone host tests.
use alloc::vec::Vec;

/// Where a stream's bytes come from. FFmpeg reads and seeks through it.
///
/// A source may still be arriving (a download, or Media Source segments): it says how
/// much can be read now with `available`, and [`Player`] decodes only while enough
/// lies ahead ([`AHEAD`]) or the stream is whole. `read_at` must never return 0
/// before the true end, since FFmpeg takes that as the end.
pub trait Source {
    /// The stream's length in bytes, if known.
    fn len(&self) -> Option<u64>;
    /// Read bytes at `at` into `out`; how many, 0 at the end, or `None` on an error.
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize>;
    /// Bytes readable from `at` without waiting; `u64::MAX` once every byte is here.
    fn available(&self, _at: u64) -> u64 { u64::MAX }
    /// How much must lie ahead before decoding goes on ([`AHEAD`] unless the source
    /// knows better, as a Media Source feed made of small segments does).
    fn ahead(&self) -> u64 { AHEAD }
    /// Whether bytes are still arriving at open: such a stream is read in order, as
    /// from a pipe, and not probed ahead (FFmpeg would run into bytes not here yet).
    fn growing(&self) -> bool { false }
    /// Seeking a stream read in order: the source starts over as a new stream that
    /// plays from `ms` (a Media Source feed hands out its init segment, then the
    /// segment holding `ms`), and the decoder opens again on it. False if the source
    /// can't (then FFmpeg seeks within the stream as usual).
    fn restart(&mut self, _ms: u64) -> bool { false }
    /// At the end of the stream (or where a growing one ran out): whether another
    /// follows on from here, as a Media Source feed's next segments do. `Some(true)`:
    /// it starts now, and the decoder opens again on it, keeping the player's pictures
    /// and clock (times run on); `Some(false)`: not yet, ask again once
    /// `available` says something is there; `None`: this is the end.
    fn next_stream(&mut self) -> Option<bool> { None }
}

/// Bytes a growing source must have ahead of the read position before decoding goes
/// on: FFmpeg reads 64 KiB at a time and may look further ahead to find a frame.
pub const AHEAD: u64 = 512 * 1024;

/// A stream already in memory, such as a fetched body.
impl Source for Vec<u8> {
    fn len(&self) -> Option<u64> { Some(self.as_slice().len() as u64) }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize> {
        let at = (at as usize).min(self.as_slice().len());
        let n = out.len().min(self.as_slice().len() - at);
        out[..n].copy_from_slice(&self[at..at + n]);
        Some(n)
    }
}

impl Source for alloc::rc::Rc<[u8]> {
    fn len(&self) -> Option<u64> { Some((**self).len() as u64) }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize> {
        let at = (at as usize).min((**self).len());
        let n = out.len().min((**self).len() - at);
        out[..n].copy_from_slice(&self[at..at + n]);
        Some(n)
    }
}

/// A download still arriving: the loader appends to `bytes` and sets `done` at the
/// end; the decoder reads what is there. Shared as [`Growing::shared`].
///
/// When the server takes Range requests (`ranges`), a read far from what is coming
/// (a seek ahead, or back into a gap) sets `want`: the loader then keeps the bytes it
/// has in `kept`, and starts a new piece there (`start`, `bytes` from empty).
#[derive(Default)]
pub struct Growing {
    /// The piece coming in now, which begins at `start` in the stream.
    pub bytes: Vec<u8>,
    pub start: u64,
    /// Pieces that came before, each with where it begins.
    pub kept: Vec<(u64, Vec<u8>)>,
    /// The length the server said, if it said one.
    pub total: Option<u64>,
    /// The piece coming in now has stopped (without `ranges`: the stream ends there).
    pub done: bool,
    /// The server can send a piece from any byte on.
    pub ranges: bool,
    /// Where the decoder wants to read and nothing is coming: the loader's next piece.
    pub want: Option<u64>,
}

/// How far past the coming piece's end a read just waits for it rather than asking
/// for a new piece (FFmpeg skips small gaps between sound and pictures).
// 1 MiB meant a seek could wait over ten seconds on a 100 KiB/s connection.
// Keep short demuxer skips local, but start a Range for larger missing gaps.
pub const NEAR: u64 = 128 * 1024;

impl Growing {
    pub fn shared() -> alloc::rc::Rc<core::cell::RefCell<Growing>> { Default::default() }

    /// The stream's end, if known now.
    fn end(&self) -> Option<u64> {
        if self.done && (!self.ranges || self.total.is_none()) { Some(self.start + self.bytes.len() as u64) } else { self.total }
    }

    /// The piece holding `at`, if one does.
    fn piece(&self, at: u64) -> Option<(u64, &[u8])> {
        core::iter::once((self.start, self.bytes.as_slice())).chain(self.kept.iter().map(|(s, b)| (*s, b.as_slice())))
            .find(|(s, b)| at >= *s && at < s + b.len() as u64)
    }

    /// Where the bytes from `at` on stop being here (`at` when it is not).
    fn reach(&self, at: u64) -> u64 {
        let mut pos = at;
        for _ in 0..=self.kept.len() {
            match self.piece(pos) { Some((s, b)) => pos = s + b.len() as u64, None => break }
        }
        pos
    }

    /// Every byte is here: nothing more will be wanted.
    pub fn whole(&self) -> bool { self.end().is_some_and(|e| self.reach(0) >= e) }

    /// A read at `at` found nothing here: ask for a piece there unless the one coming
    /// will soon reach it.
    fn ask(&mut self, at: u64) {
        let edge = self.start + self.bytes.len() as u64;
        if self.ranges && (self.done || at < self.start || at > edge + NEAR) && self.end().is_none_or(|e| at < e) {
            self.want = Some(at);
        }
    }

    /// Starts the next piece at `at`, keeping what came (and letting go of the pieces
    /// furthest from `at` while more than `limit` bytes are kept).
    pub fn begin(&mut self, at: u64, limit: usize) {
        let old = core::mem::take(&mut self.bytes);
        if !old.is_empty() { self.kept.push((self.start, old)); }
        let mut held: usize = self.kept.iter().map(|(_, b)| b.len()).sum();
        while held > limit && !self.kept.is_empty() {
            let far = (0..self.kept.len()).max_by_key(|&i| self.kept[i].0.abs_diff(at)).unwrap();
            held -= self.kept.swap_remove(far).1.len();
        }
        (self.start, self.done, self.want) = (at, false, None);
    }
}

impl Source for alloc::rc::Rc<core::cell::RefCell<Growing>> {
    fn len(&self) -> Option<u64> { self.borrow().end() }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize> {
        let mut g = self.borrow_mut();
        if let Some((s, b)) = g.piece(at) {
            let b = &b[(at - s) as usize..];
            let n = out.len().min(b.len());
            out[..n].copy_from_slice(&b[..n]);
            return Some(n);
        }
        // Past what has come: the end once it is all here, else an error rather
        // than a 0 FFmpeg would take for the end.
        if g.end().is_some_and(|e| at >= e) { return Some(0); }
        g.ask(at);
        None
    }
    fn available(&self, at: u64) -> u64 {
        let mut g = self.borrow_mut();
        let reach = g.reach(at);
        if g.end().is_some_and(|e| reach >= e) { return u64::MAX; }
        g.ask(reach);
        reach - at
    }
}

#[cfg(test)]
mod growing_tests {
    use super::*;

    fn stream(ranges: bool) -> alloc::rc::Rc<core::cell::RefCell<Growing>> {
        let s = Growing::shared();
        { let mut g = s.borrow_mut();
          g.bytes.resize(600 * 1024, 7); g.total = Some(1500 * 1024); g.ranges = ranges; }
        s
    }

    #[test]
    fn forward_seek_asks_for_a_piece_before_whole_download() {
        let mut s = stream(true);
        let at = 1200 * 1024;
        assert_eq!(s.read_at(at, &mut [0; 16]), None);
        assert_eq!(s.borrow().want, Some(at));
        s.borrow_mut().begin(at, 2 * 1024 * 1024);
        s.borrow_mut().bytes.extend_from_slice(&[3; 16]);
        let mut out = [0; 16];
        assert_eq!(s.read_at(at, &mut out), Some(16));
        assert_eq!(out, [3; 16]);
        assert_eq!(s.read_at(0, &mut out), Some(16));
        assert_eq!(out, [7; 16]);
    }

    #[test]
    fn sequential_and_small_interleaved_gaps_keep_the_current_download() {
        let mut s = stream(true);
        for at in [600 * 1024, 616 * 1024, 728 * 1024] {
            assert_eq!(s.read_at(at, &mut [0; 16]), None);
            assert_eq!(s.available(at), 0);
            assert_eq!(s.borrow().want, None);
        }
    }

    #[test]
    fn no_range_server_and_end_of_stream_do_not_request_a_piece() {
        let mut s = stream(false);
        assert_eq!(s.read_at(1200 * 1024, &mut [0; 16]), None);
        assert_eq!(s.borrow().want, None);
        s.borrow_mut().ranges = true;
        assert_eq!(s.read_at(1500 * 1024, &mut [0; 16]), Some(0));
        assert_eq!(s.borrow().want, None);
    }
}
