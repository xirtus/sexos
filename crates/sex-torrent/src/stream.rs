// SPDX-License-Identifier: GPL-3.0-only
//! Play while downloading: the protocol between a player (Qupid) and Swingers, and the
//! planner that turns "the player is at byte X of this file" into piece deadlines.
//!
//! Swingers is the confined program that owns the torrent; Qupid never touches the
//! network or the swarm. Qupid opens a file of a torrent, reads byte ranges from it, and
//! tells Swingers where playback is. Swingers answers reads it can serve at once and
//! says "not yet" (with what is missing) for the rest, while the planner moves the piece
//! deadlines so the bytes Qupid needs next arrive first:
//!
//! - the header (first bytes: MP4 `ftyp`/`moov` when it is at the front, MKV EBML and
//!   `SeekHead`) and the footer (MP4 `moov` at the end, MKV `Cues`) are pinned first,
//!   because a demuxer reads both before it shows a frame;
//! - then a window ahead of the play position, sized from the bitrate, each piece due by
//!   the time playback will reach it;
//! - a seek moves the window; pieces behind it lose their deadlines and fall back to the
//!   normal (rarest first) order.
//!
//! The messages are fixed little-endian records, so either side can be no_std. They
//! travel however the two programs are connected (a PDX mailbox with a lent region for
//! the data); this module only defines the bytes.
use alloc::vec::Vec;

use crate::Millis;

/// Bytes of the file start pinned before anything else.
pub const HEADER_BYTES: u64 = 2 << 20;
/// Bytes of the file end pinned before anything else.
pub const FOOTER_BYTES: u64 = 1 << 20;
/// Seconds of playback kept ahead of the play position.
pub const WINDOW_SECONDS: u64 = 30;
/// Bitrate assumed until the player reports one: 8 Mbit/s (1 MB/s), a 1080p film.
pub const DEFAULT_BYTES_PER_SECOND: u64 = 1_000_000;
/// The window never shrinks below this many bytes, so a low bitrate guess still buffers.
pub const MIN_WINDOW_BYTES: u64 = 8 << 20;
/// Largest single read answered in one message.
pub const MAX_READ: u32 = 1 << 20;

/// Where a file sits in the torrent's piece space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSpan {
    /// Byte offset of the file's first byte in the torrent (v1 continuous layout; for v2
    /// the file starts on a piece boundary, which this handles the same way).
    pub offset: u64,
    pub length: u64,
    pub piece_length: u32,
}

impl FileSpan {
    /// The pieces holding bytes `[from, to)` of the file.
    pub fn pieces(&self, from: u64, to: u64) -> core::ops::Range<u32> {
        let to = to.min(self.length);
        if from >= to || self.piece_length == 0 { return 0..0; }
        let pl = self.piece_length as u64;
        let first = (self.offset + from) / pl;
        let last = (self.offset + to - 1) / pl;
        first as u32..(last + 1) as u32
    }
    /// The first byte of the file inside `piece`, as a file offset (0 when the piece
    /// starts before the file).
    pub fn file_offset_of(&self, piece: u32) -> u64 {
        (piece as u64 * self.piece_length as u64).saturating_sub(self.offset)
    }
}

/// A piece and the time by which playback needs it. Earlier deadlines are fetched first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Deadline { pub due: Millis, pub piece: u32 }

/// Turns play position and bitrate into deadlines for the piece picker.
#[derive(Clone, Debug)]
pub struct Planner {
    pub span: FileSpan,
    position: u64,
    bytes_per_second: u64,
    /// Time the position was reported.
    at: Millis,
}

impl Planner {
    pub fn new(span: FileSpan, now: Millis) -> Planner {
        Planner { span, position: 0, bytes_per_second: DEFAULT_BYTES_PER_SECOND, at: now }
    }

    /// The player is at `position` (a byte offset in the file). `bytes_per_second` is the
    /// stream's bitrate when the demuxer knows it (0 keeps the last guess).
    pub fn set_position(&mut self, position: u64, bytes_per_second: u64, now: Millis) {
        self.position = position.min(self.span.length);
        if bytes_per_second > 0 { self.bytes_per_second = bytes_per_second; }
        self.at = now;
    }

    pub fn position(&self) -> u64 { self.position }

    /// Deadlines to hand the picker, earliest first. Header and footer pieces are due
    /// at once; window pieces are due when playback reaches them (with a margin of a
    /// quarter of the time, so a slow peer has time to be replaced).
    pub fn deadlines(&self, have: &dyn Fn(u32) -> bool) -> Vec<Deadline> {
        let mut out: Vec<Deadline> = Vec::new();
        let push = |piece: u32, due: Millis, out: &mut Vec<Deadline>| {
            if have(piece) { return; }
            match out.iter_mut().find(|d| d.piece == piece) {
                Some(d) => d.due = d.due.min(due),
                None => out.push(Deadline { due, piece }),
            }
        };
        let len = self.span.length;
        for p in self.span.pieces(0, HEADER_BYTES.min(len)) { push(p, self.at, &mut out); }
        for p in self.span.pieces(len.saturating_sub(FOOTER_BYTES), len) { push(p, self.at, &mut out); }
        let window = (self.bytes_per_second.saturating_mul(WINDOW_SECONDS)).max(MIN_WINDOW_BYTES);
        let rate = self.bytes_per_second.max(1);
        for p in self.span.pieces(self.position, self.position.saturating_add(window)) {
            let ahead = self.span.file_offset_of(p).saturating_sub(self.position);
            let due = self.at + ahead.saturating_mul(1000) / rate * 3 / 4;
            push(p, due, &mut out);
        }
        out.sort();
        out
    }

    /// Bytes from `offset` that are already on disk, given which pieces we have: how far
    /// a read starting there can be served now.
    pub fn readable(&self, offset: u64, have: &dyn Fn(u32) -> bool) -> u64 {
        if offset >= self.span.length { return 0; }
        let mut end = offset;
        for p in self.span.pieces(offset, self.span.length) {
            if !have(p) { break; }
            let piece_end = (p as u64 + 1) * self.span.piece_length as u64;
            end = piece_end.saturating_sub(self.span.offset).min(self.span.length);
        }
        end - offset
    }
}

// ---- the messages ---------------------------------------------------------------------

/// Player to Swingers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Open file `file` of the torrent named by its 20-byte wire hash (Swingers adds it
    /// first when Qupid hands over a magnet with [`Request::AddMagnet`]).
    Open { info_hash: [u8; 20], file: u32 },
    /// Add a torrent from a magnet link (from Qupid's indexer search) and start it in
    /// streaming mode. Answered with [`Reply::Added`].
    AddMagnet { uri: Vec<u8> },
    /// Read up to `length` bytes at `offset` of an open stream.
    Read { stream: u32, offset: u64, length: u32 },
    /// Where playback is, and the bitrate if known (0 = unknown).
    Position { stream: u32, offset: u64, bytes_per_second: u64 },
    Close { stream: u32 },
}

/// Swingers to player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Added { info_hash: [u8; 20] },
    /// `length` of the file, and its name's last component (for the title and for
    /// picking a demuxer by extension).
    Opened { stream: u32, length: u64, name: Vec<u8> },
    /// Bytes at `offset`. In a lent region the data travels outside the message; here
    /// it is inline for transports without one.
    Data { stream: u32, offset: u64, bytes: Vec<u8> },
    /// Nothing at `offset` yet. `eta` is the expected wait in milliseconds (0 unknown),
    /// so the player can show "buffering 40%" instead of a frozen frame.
    NotYet { stream: u32, offset: u64, eta: u32, buffered_permille: u16 },
    /// The torrent or file is gone, or the request was malformed.
    Failed { stream: u32, code: Failure },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Failure { NoTorrent = 1, NoFile = 2, NoStream = 3, BadMagnet = 4, Malformed = 5, Removed = 6 }

impl Failure {
    fn from_u8(b: u8) -> Option<Failure> {
        Some(match b { 1 => Failure::NoTorrent, 2 => Failure::NoFile, 3 => Failure::NoStream, 4 => Failure::BadMagnet, 5 => Failure::Malformed, 6 => Failure::Removed, _ => return None })
    }
}

/// Largest message accepted (a Data reply of [`MAX_READ`] plus its header).
pub const MAX_MESSAGE: usize = MAX_READ as usize + 64;
/// Longest magnet link or file name accepted.
pub const MAX_TEXT: usize = 8192;

struct Reader<'a> { b: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> { let s = self.b.get(self.at..self.at.checked_add(n)?)?; self.at += n; Some(s) }
    fn u8(&mut self) -> Option<u8> { Some(self.take(1)?[0]) }
    fn u16(&mut self) -> Option<u16> { Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?)) }
    fn u32(&mut self) -> Option<u32> { Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?)) }
    fn u64(&mut self) -> Option<u64> { Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?)) }
    fn hash(&mut self) -> Option<[u8; 20]> { self.take(20)?.try_into().ok() }
    fn text(&mut self, max: usize) -> Option<Vec<u8>> { let n = self.u32()? as usize; if n > max { return None; } Some(self.take(n)?.to_vec()) }
    fn done(&self) -> bool { self.at == self.b.len() }
}

fn put_text(out: &mut Vec<u8>, t: &[u8]) { out.extend_from_slice(&(t.len() as u32).to_le_bytes()); out.extend_from_slice(t); }

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        match self {
            Request::Open { info_hash, file } => { o.push(1); o.extend_from_slice(info_hash); o.extend_from_slice(&file.to_le_bytes()); }
            Request::AddMagnet { uri } => { o.push(2); put_text(&mut o, uri); }
            Request::Read { stream, offset, length } => { o.push(3); o.extend_from_slice(&stream.to_le_bytes()); o.extend_from_slice(&offset.to_le_bytes()); o.extend_from_slice(&length.to_le_bytes()); }
            Request::Position { stream, offset, bytes_per_second } => { o.push(4); o.extend_from_slice(&stream.to_le_bytes()); o.extend_from_slice(&offset.to_le_bytes()); o.extend_from_slice(&bytes_per_second.to_le_bytes()); }
            Request::Close { stream } => { o.push(5); o.extend_from_slice(&stream.to_le_bytes()); }
        }
        o
    }
    pub fn decode(b: &[u8]) -> Option<Request> {
        if b.len() > MAX_MESSAGE { return None; }
        let mut r = Reader { b, at: 0 };
        let m = match r.u8()? {
            1 => Request::Open { info_hash: r.hash()?, file: r.u32()? },
            2 => Request::AddMagnet { uri: r.text(MAX_TEXT)? },
            3 => { let stream = r.u32()?; let offset = r.u64()?; let length = r.u32()?; if length > MAX_READ { return None; } Request::Read { stream, offset, length } }
            4 => Request::Position { stream: r.u32()?, offset: r.u64()?, bytes_per_second: r.u64()? },
            5 => Request::Close { stream: r.u32()? },
            _ => return None,
        };
        r.done().then_some(m)
    }
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        match self {
            Reply::Added { info_hash } => { o.push(1); o.extend_from_slice(info_hash); }
            Reply::Opened { stream, length, name } => { o.push(2); o.extend_from_slice(&stream.to_le_bytes()); o.extend_from_slice(&length.to_le_bytes()); put_text(&mut o, name); }
            Reply::Data { stream, offset, bytes } => { o.push(3); o.extend_from_slice(&stream.to_le_bytes()); o.extend_from_slice(&offset.to_le_bytes()); put_text(&mut o, bytes); }
            Reply::NotYet { stream, offset, eta, buffered_permille } => { o.push(4); o.extend_from_slice(&stream.to_le_bytes()); o.extend_from_slice(&offset.to_le_bytes()); o.extend_from_slice(&eta.to_le_bytes()); o.extend_from_slice(&buffered_permille.to_le_bytes()); }
            Reply::Failed { stream, code } => { o.push(5); o.extend_from_slice(&stream.to_le_bytes()); o.push(*code as u8); }
        }
        o
    }
    pub fn decode(b: &[u8]) -> Option<Reply> {
        if b.len() > MAX_MESSAGE { return None; }
        let mut r = Reader { b, at: 0 };
        let m = match r.u8()? {
            1 => Reply::Added { info_hash: r.hash()? },
            2 => Reply::Opened { stream: r.u32()?, length: r.u64()?, name: r.text(MAX_TEXT)? },
            3 => Reply::Data { stream: r.u32()?, offset: r.u64()?, bytes: r.text(MAX_READ as usize)? },
            4 => Reply::NotYet { stream: r.u32()?, offset: r.u64()?, eta: r.u32()?, buffered_permille: r.u16()? },
            5 => Reply::Failed { stream: r.u32()?, code: Failure::from_u8(r.u8()?)? },
            _ => return None,
        };
        r.done().then_some(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const MIB: u64 = 1 << 20;

    fn film() -> FileSpan { FileSpan { offset: 3 * MIB / 2, length: 700 * MIB, piece_length: (MIB / 2) as u32 } }

    #[test]
    fn pieces_cover_a_range() {
        let s = film();
        assert_eq!(s.pieces(0, 1), 3..4);
        assert_eq!(s.pieces(0, MIB / 2 + 1), 3..5);
        assert_eq!(s.pieces(5, 5), 0..0);
        assert_eq!(s.pieces(699 * MIB, 800 * MIB), 1401..1403);
        assert_eq!(s.file_offset_of(3), 0);
        assert_eq!(s.file_offset_of(5), MIB);
    }

    #[test]
    fn header_and_footer_first_then_window_in_order() {
        let p = Planner::new(film(), 1000);
        let d = p.deadlines(&|_| false);
        let header: Vec<u32> = film().pieces(0, HEADER_BYTES).collect();
        let footer: Vec<u32> = film().pieces(700 * MIB - FOOTER_BYTES, 700 * MIB).collect();
        for x in header.iter().chain(footer.iter()) { assert!(d.iter().any(|e| e.piece == *x && e.due == 1000)); }
        // Window pieces get later deadlines the further they are from the play position.
        let later: Vec<&Deadline> = d.iter().filter(|e| e.due > 1000).collect();
        assert!(later.windows(2).all(|w| w[0].piece < w[1].piece));
        // 30 s at 1 MB/s is under the 8 MiB minimum window... so the window is 30 MB.
        let last = later.last().unwrap().piece;
        assert_eq!(last, film().pieces(0, 30_000_000).end - 1);
    }

    #[test]
    fn seek_moves_the_window_and_drops_pieces_behind() {
        let mut p = Planner::new(film(), 0);
        p.set_position(400 * MIB, 2_000_000, 5000);
        let d = p.deadlines(&|_| false);
        let near: Vec<u32> = film().pieces(400 * MIB, 400 * MIB + 1).collect();
        assert!(d.iter().any(|e| e.piece == near[0] && e.due == 5000));
        // Nothing between the header and the new position is asked for.
        let gap = film().pieces(HEADER_BYTES + MIB, 399 * MIB);
        assert!(!d.iter().any(|e| gap.contains(&e.piece)));
        // A higher bitrate makes a longer window (60 MB instead of 30).
        let far = film().pieces(400 * MIB, 400 * MIB + 59_000_000).end - 1;
        assert!(d.iter().any(|e| e.piece == far));
    }

    #[test]
    fn pieces_we_have_are_left_out_and_readable_counts_them() {
        let p = Planner::new(film(), 0);
        let have = |x: u32| x < 10;
        assert!(!p.deadlines(&have).iter().any(|e| e.piece < 10));
        // Pieces 3..10 hold file bytes 0..3.5 MiB.
        assert_eq!(p.readable(0, &have), 7 * MIB / 2);
        assert_eq!(p.readable(MIB, &have), 5 * MIB / 2);
        assert_eq!(p.readable(4 * MIB, &have), 0);
        assert_eq!(p.readable(800 * MIB, &have), 0);
    }

    #[test]
    fn messages_round_trip() {
        let reqs = vec![
            Request::Open { info_hash: [7; 20], file: 2 },
            Request::AddMagnet { uri: b"magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".to_vec() },
            Request::Read { stream: 1, offset: 1 << 40, length: 65536 },
            Request::Position { stream: 1, offset: 99, bytes_per_second: 0 },
            Request::Close { stream: 9 },
        ];
        for r in reqs { assert_eq!(Request::decode(&r.encode()), Some(r)); }
        let reps = vec![
            Reply::Added { info_hash: [1; 20] },
            Reply::Opened { stream: 3, length: 700 * MIB, name: b"film.mkv".to_vec() },
            Reply::Data { stream: 3, offset: 0, bytes: vec![0x1a, 0x45, 0xdf, 0xa3] },
            Reply::NotYet { stream: 3, offset: 5, eta: 1200, buffered_permille: 400 },
            Reply::Failed { stream: 3, code: Failure::Removed },
        ];
        for r in reps { assert_eq!(Reply::decode(&r.encode()), Some(r)); }
    }

    #[test]
    fn hostile_messages_are_refused() {
        assert_eq!(Request::decode(&[]), None);
        assert_eq!(Request::decode(&[9]), None);
        let mut big = Request::Read { stream: 1, offset: 0, length: 1 }.encode();
        big[13..17].copy_from_slice(&(MAX_READ + 1).to_le_bytes());
        assert_eq!(Request::decode(&big), None);
        let mut long = vec![2u8];
        long.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Request::decode(&long), None);
        let mut trailing = Request::Close { stream: 1 }.encode();
        trailing.push(0);
        assert_eq!(Request::decode(&trailing), None);
        assert_eq!(Reply::decode(&[5, 0, 0, 0, 0, 99]), None);
        for n in 0..40 { let _ = Reply::decode(&vec![3u8; n]); let _ = Request::decode(&vec![4u8; n]); }
    }
}
