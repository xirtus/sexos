// SPDX-License-Identifier: GPL-3.0-only
//! Media Source Extensions bookkeeping: which byte streams a SourceBuffer takes, the
//! media segments it holds, and what time each one covers (`buffered`). Players such as
//! YouTube's, dash.js and hls.js decide what to fetch next from `buffered`, so it must
//! follow the real segment times.
//!
//! Appended bytes are cut into an initialization segment and media segments, each
//! kept with its times. The decoder is then handed init + segments in time order from
//! wherever playback is (see `apps/web/src/mse.rs`), so seeking, segments appended out
//! of order and a new init segment (a quality switch) all work on a stream FFmpeg reads
//! straight through.
//!
//! Fragmented MP4 (ISO BMFF, as YouTube, DASH and HLS fMP4 use): `ftyp` `moov`, then
//! `moof` `mdat` pairs; times from `moof/traf`: `tfdt` plus the sample durations in
//! `trun` (or the defaults in `tfhd`/`trex`), over `mdhd`'s timescale.
//!
//! WebM: the EBML header, a Segment (often of unknown size) with `Info` and `Tracks`,
//! then Clusters; times from each Cluster's `Timecode` plus its blocks' offsets, over
//! `TimecodeScale`.
//!
//! `timestampOffset` is applied by rewriting the times in the segment (`tfdt`, the
//! Cluster `Timecode`), so the decoder sees the times the page asked for.
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format { Mp4, WebM }

/// The byte stream format a SourceBuffer of this type takes, or None if it can't be
/// made (`MediaSource.isTypeSupported`).
pub fn format(mime: &str) -> Option<Format> {
    let mut parts = mime.split(';');
    let base = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let format = match base.as_str() {
        "video/mp4" | "audio/mp4" => Format::Mp4,
        "video/webm" | "audio/webm" => Format::WebM,
        _ => return None,
    };
    for p in parts {
        let p = p.trim();
        let Some(v) = p.strip_prefix("codecs=").or_else(|| p.strip_prefix("codecs =")) else { continue };
        for c in v.trim().trim_matches('"').split(',') {
            let c = c.trim().to_ascii_lowercase();
            let family = c.split('.').next().unwrap_or("");
            let ok = match format {
                // AAC-LC, HE-AAC and MP3 in mp4a.40.x / mp4a.6b / mp4a.69.
                Format::Mp4 => match family {
                    "avc1" | "avc3" | "av01" | "mp3" => true,
                    "mp4a" => c == "mp4a" || c.starts_with("mp4a.40") || c == "mp4a.6b" || c == "mp4a.69",
                    _ => false,
                },
                Format::WebM => matches!(family, "vp8" | "vp9" | "vp09" | "av01" | "opus" | "vorbis"),
            };
            if !ok { return None; }
        }
    }
    Some(format)
}

/// Whether a SourceBuffer of this type can be made.
pub fn supported(mime: &str) -> bool { format(mime).is_some() }

#[derive(Clone, Copy, Default, Debug)]
struct Track {
    id: u64,
    /// MP4: ticks per second. WebM: unused (the segment's TimecodeScale counts).
    timescale: u32,
    /// MP4: ticks (trex). WebM: nanoseconds (DefaultDuration).
    default_duration: u64,
    video: bool,
    audio: bool,
}

/// One media segment: a `moof` with its `mdat`, or a WebM Cluster.
pub struct Seg {
    /// Seconds, `timestampOffset` applied.
    pub start: f64,
    pub end: f64,
    /// Which init segment it follows (an index into [`Stream::inits`]).
    pub init: usize,
    pub bytes: Vec<u8>,
}

/// What one SourceBuffer has been given.
pub struct Stream {
    pub format: Format,
    /// Bytes of an element not whole yet (its header, or the whole of one held).
    part: Vec<u8>,
    /// Bytes of the current element still to skip.
    skip: u64,
    /// Bytes of the current `mdat` still to copy into `open`.
    copy: u64,
    /// A segment gathering its sample bytes.
    open: Option<Seg>,
    /// An init segment being put together (`ftyp`, or the EBML header and `Info`).
    building: Vec<u8>,
    /// WebM: inside an unknown-size Cluster, gathering its children.
    cluster: Option<Vec<u8>>,
    tracks: Vec<Track>,
    /// WebM: nanoseconds per Timecode tick.
    scale_ns: u64,
    /// Every init segment seen; segments name theirs.
    pub inits: Vec<Vec<u8>>,
    segs: Vec<Seg>,
    /// Seconds the page removed (`remove`) that segments still partly hold.
    cuts: Vec<(f64, f64)>,
    ranges: Vec<(f64, f64)>,
    held: usize,
    last_end: f64,
    /// `SourceBuffer.timestampOffset`, seconds.
    pub offset: f64,
    /// Where playback is, so a full buffer can let go of what has been played.
    pub playhead: f64,
    pub inited: bool,
    pub segments: u32,
    /// The largest segment, in bytes.
    pub largest: u64,
    pub broken: bool,
}

/// Two ranges closer than this are one (browsers allow a small gap between segments).
const JOIN: f64 = 0.1;
/// A held element (`moov`, `moof`, a Cluster) larger than this is refused.
const BOX_MAX: u64 = 16 << 20;
/// Bytes a SourceBuffer holds before it lets go of what has been played, and refuses
/// appends past that (QuotaExceededError in a browser).
pub const QUOTA: usize = 160 << 20;
const EPS: f64 = 1e-3;

fn u32_at(b: &[u8], at: usize) -> Option<u32> { b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])) }
fn u64_at(b: &[u8], at: usize) -> Option<u64> { b.get(at..at + 8).map(|s| u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])) }

/// The child boxes of `body`: (type, where its contents are).
fn spans(body: &[u8]) -> impl Iterator<Item = ([u8; 4], core::ops::Range<usize>)> + '_ {
    let mut at = 0usize;
    core::iter::from_fn(move || {
        let size = u32_at(body, at)? as u64;
        let kind: [u8; 4] = body.get(at + 4..at + 8)?.try_into().ok()?;
        let (head, size) = match size {
            1 => (16usize, u64_at(body, at + 8)?),
            0 => (8, (body.len() - at) as u64),
            s => (8, s),
        };
        if size < head as u64 || at as u64 + size > body.len() as u64 { return None; }
        let r = at + head..at + size as usize;
        at += size as usize;
        Some((kind, r))
    })
}
fn boxes(body: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> { spans(body).map(move |(k, r)| (k, &body[r])) }
fn child<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> { boxes(body).find(|(k, _)| k == kind).map(|(_, b)| b) }

// EBML (WebM) element ids.
const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const INFO: u32 = 0x1549_A966;
const TIMECODE_SCALE: u32 = 0x2A_D7B1;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const DEFAULT_DURATION: u32 = 0x23_E383;
const CLUSTER: u32 = 0x1F43_B675;
const TIMECODE: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const BLOCK_DURATION: u32 = 0x9B;
/// Elements that sit directly in the Segment: one ends an unknown-size Cluster.
const TOP: [u32; 9] = [EBML, SEGMENT, INFO, TRACKS, CLUSTER, 0x114D_9B74, 0x1C53_BB6B, 0x1254_C367, 0x1043_A770];

/// An EBML id (marker kept) and its length, from its first bytes.
fn ebml_id(b: &[u8]) -> Option<(u32, usize)> {
    let first = *b.first()?;
    let n = first.leading_zeros() as usize + 1;
    if n > 4 { return None; }
    let bytes = b.get(..n)?;
    Some((bytes.iter().fold(0u32, |v, &x| v << 8 | x as u32), n))
}
/// An EBML size: (value, length, unknown).
fn ebml_size(b: &[u8]) -> Option<(u64, usize, bool)> {
    let first = *b.first()?;
    let n = first.leading_zeros() as usize + 1;
    if n > 8 { return None; }
    let bytes = b.get(..n)?;
    let mut v = (first as u64) & (0xFF >> n);
    for &x in &bytes[1..] { v = v << 8 | x as u64; }
    let unknown = v == (1u64 << (7 * n)) - 1;
    Some((v, n, unknown))
}
/// How many header bytes an element needs before its id and size can be read.
fn ebml_head_len(b: &[u8]) -> Option<usize> {
    let (_, n) = ebml_id(b).or_else(|| (!b.is_empty() && b[0].leading_zeros() < 4).then_some((0, b[0].leading_zeros() as usize + 1)))?;
    let s = *b.get(n)?;
    let m = s.leading_zeros() as usize + 1;
    (m <= 8).then_some(n + m)
}
/// The children of an EBML element's body: (id, contents). Stops at an unknown size.
fn elements(body: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    let mut at = 0usize;
    core::iter::from_fn(move || {
        let (id, n) = ebml_id(&body[at..])?;
        let (size, m, unknown) = ebml_size(&body[at + n..])?;
        if unknown { return None; }
        let start = at + n + m;
        let end = start.checked_add(size as usize)?;
        if end > body.len() { return None; }
        at = end;
        Some((id, &body[start..end]))
    })
}
/// Nearest whole number (no_std has no `f64::round`).
fn rnd(x: f64) -> f64 { if x >= 0.0 { (x + 0.5) as i64 as f64 } else { -((-x + 0.5) as i64 as f64) } }
fn ebml_uint(b: &[u8]) -> u64 { b.iter().take(8).fold(0u64, |v, &x| v << 8 | x as u64) }
/// An EBML element with an 8-byte size.
fn ebml_put(out: &mut Vec<u8>, id: u32, body: &[u8]) {
    let idb = id.to_be_bytes();
    let skip = idb.iter().take_while(|&&b| b == 0).count();
    out.extend_from_slice(&idb[skip..]);
    out.push(0x01);
    out.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
    out.extend_from_slice(body);
}

impl Stream {
    pub fn new(format: Format) -> Stream {
        Stream {
            format, part: Vec::new(), skip: 0, copy: 0, open: None, building: Vec::new(), cluster: None,
            tracks: Vec::new(), scale_ns: 1_000_000, inits: Vec::new(), segs: Vec::new(), cuts: Vec::new(),
            ranges: Vec::new(), held: 0, last_end: 0.0, offset: 0.0, playhead: 0.0,
            inited: false, segments: 0, largest: 0, broken: false,
        }
    }

    /// Seconds buffered, as (start, end) pairs in order.
    pub fn buffered(&self) -> &[(f64, f64)] { &self.ranges }
    pub fn has_video(&self) -> bool { self.tracks.iter().any(|t| t.video) }
    pub fn has_audio(&self) -> bool { self.tracks.iter().any(|t| t.audio) }
    /// Segment bytes held.
    pub fn held(&self) -> usize { self.held }

    pub fn segs(&self) -> &[Seg] { &self.segs }
    /// The segment to play `t` from: the latest one starting at or before it that
    /// reaches past it, or one starting just after.
    pub fn seg_at(&self, t: f64) -> Option<usize> {
        let mut best = None;
        for (i, s) in self.segs.iter().enumerate() {
            if s.start <= t + EPS && t < s.end - EPS { best = Some(i); }
        }
        best.or_else(|| self.segs.iter().position(|s| s.start > t && s.start <= t + JOIN))
    }
    /// The segment that plays on after one from `start` to `end`: the next start after
    /// it, if no further than a small gap past its end.
    pub fn next_after(&self, start: f64, end: f64) -> Option<usize> {
        self.segs.iter().position(|s| s.start > start + EPS && s.start <= end + JOIN)
    }
    /// What plays on from segment `i` (itself included) without a gap: its bytes (up
    /// to `limit`) and where it ends, in seconds.
    pub fn ahead_from(&self, mut i: usize, limit: u64) -> (u64, f64) {
        let mut n = 0u64;
        loop {
            let s = &self.segs[i];
            n += s.bytes.len() as u64;
            if n >= limit { return (n, s.end); }
            match self.next_after(s.start, s.end) { Some(j) => i = j, None => return (n, s.end) }
        }
    }

    /// Reads appended bytes. False once the stream can't be followed (an element too
    /// big or malformed), or when the buffer is full of what is still to play.
    pub fn append(&mut self, bytes: &[u8]) -> bool {
        if self.broken { return false; }
        if self.held + bytes.len() > QUOTA {
            let keep_from = self.playhead - 1.0;
            let before = self.segs.len();
            self.segs.retain(|s| s.end >= keep_from);
            if self.segs.len() != before {
                self.held = self.segs.iter().map(|s| s.bytes.len()).sum();
                self.ranges_again();
            }
            if self.held + bytes.len() > QUOTA { return false; }
        }
        match self.format {
            Format::Mp4 => self.append_mp4(bytes),
            Format::WebM => self.append_webm(bytes),
        }
        !self.broken
    }

    /// The stream ended (`endOfStream`): a WebM Cluster of unknown size ends here.
    pub fn finish(&mut self) {
        if let Some(body) = self.cluster.take() { self.cluster(&body); }
    }

    /// `abort()`: drop a segment half read.
    pub fn reset_parser(&mut self) {
        self.part.clear();
        self.skip = 0;
        self.copy = 0;
        self.open = None;
        self.cluster = None;
    }

    /// Takes bytes into `part` until it holds `want`; false if more must come.
    fn fill(&mut self, bytes: &mut &[u8], want: usize) -> bool {
        if self.part.len() < want {
            let n = (want - self.part.len()).min(bytes.len());
            self.part.extend_from_slice(&bytes[..n]);
            *bytes = &bytes[n..];
        }
        self.part.len() >= want
    }

    fn append_mp4(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() && !self.broken {
            if self.skip > 0 {
                let n = self.skip.min(bytes.len() as u64) as usize;
                self.skip -= n as u64;
                bytes = &bytes[n..];
                continue;
            }
            if self.copy > 0 {
                let n = self.copy.min(bytes.len() as u64) as usize;
                if let Some(o) = self.open.as_mut() { o.bytes.extend_from_slice(&bytes[..n]); }
                self.copy -= n as u64;
                bytes = &bytes[n..];
                if self.copy == 0 { self.close(); }
                continue;
            }
            // The header first (8 bytes, or 16 with a 64-bit size).
            if !self.fill(&mut bytes, 8) { continue; }
            let head = if u32_at(&self.part, 0) == Some(1) { 16 } else { 8 };
            if !self.fill(&mut bytes, head) { continue; }
            let len = match u32_at(&self.part, 0) {
                Some(1) => u64_at(&self.part, 8).unwrap_or(0),
                Some(0) => u64::MAX >> 1,
                Some(s) => s as u64,
                None => 0,
            };
            if len < head as u64 { self.broken = true; break; }
            let kind: [u8; 4] = self.part[4..8].try_into().unwrap();
            match &kind {
                b"mdat" => {
                    if len == u64::MAX >> 1 { self.broken = true; break; }
                    let header = core::mem::take(&mut self.part);
                    match self.open.as_mut() {
                        Some(o) => { o.bytes.extend_from_slice(&header); self.copy = len - head as u64; if self.copy == 0 { self.close(); } }
                        None => self.skip = len - head as u64,
                    }
                }
                b"ftyp" | b"moov" | b"moof" => {
                    if len > BOX_MAX { self.broken = true; break; }
                    if !self.fill(&mut bytes, len as usize) { continue; }
                    let mut whole = core::mem::take(&mut self.part);
                    match &kind {
                        b"ftyp" => self.building = whole,
                        b"moov" => {
                            self.open = None;
                            if !self.init_mp4(&whole[head..]) { self.broken = true; break; }
                            let mut init = core::mem::take(&mut self.building);
                            init.extend_from_slice(&whole);
                            self.new_init(init);
                        }
                        _ => {
                            self.open = None;
                            match self.fragment(&mut whole[head..]) {
                                Some((start, end)) => {
                                    let init = self.inits.len() - 1;
                                    self.open = Some(Seg { start, end, init, bytes: whole });
                                }
                                None => { self.broken = true; break; }
                            }
                        }
                    }
                }
                _ => {
                    // Not needed (styp, sidx, emsg, free...): skip it.
                    if len == u64::MAX >> 1 { self.broken = true; break; }
                    self.skip = len - head as u64;
                    self.part.clear();
                }
            }
        }
    }

    /// An init segment, unless it is the one already in force (players append the same
    /// one again freely; a new one is a quality or codec switch).
    fn new_init(&mut self, init: Vec<u8>) {
        if self.inits.last() != Some(&init) { self.inits.push(init); }
        self.inited = true;
    }

    fn init_mp4(&mut self, moov: &[u8]) -> bool {
        let mut tracks = Vec::new();
        for (k, trak) in boxes(moov) {
            if &k != b"trak" { continue; }
            let Some(tkhd) = child(trak, b"tkhd") else { return false };
            let id = if tkhd.first() == Some(&1) { u32_at(tkhd, 20) } else { u32_at(tkhd, 12) };
            let Some(mdia) = child(trak, b"mdia") else { return false };
            let Some(mdhd) = child(mdia, b"mdhd") else { return false };
            let timescale = if mdhd.first() == Some(&1) { u32_at(mdhd, 20) } else { u32_at(mdhd, 12) };
            let handler = child(mdia, b"hdlr").and_then(|h| h.get(8..12));
            match (id, timescale) {
                (Some(id), Some(ts)) if ts > 0 => tracks.push(Track { id: id as u64, timescale: ts, default_duration: 0, video: handler == Some(b"vide"), audio: handler == Some(b"soun") }),
                _ => return false,
            }
        }
        if let Some(mvex) = child(moov, b"mvex") {
            for (k, trex) in boxes(mvex) {
                if &k != b"trex" { continue; }
                if let (Some(id), Some(d)) = (u32_at(trex, 4), u32_at(trex, 12)) {
                    if let Some(t) = tracks.iter_mut().find(|t| t.id == id as u64) { t.default_duration = d as u64; }
                }
            }
        }
        if tracks.is_empty() { return false; }
        self.tracks = tracks;
        true
    }

    /// A `moof`'s times, `timestampOffset` written into its `tfdt`s.
    fn fragment(&mut self, moof: &mut [u8]) -> Option<(f64, f64)> {
        if !self.inited { return None; }
        let (mut first, mut last) = (f64::MAX, f64::MIN);
        let trafs: Vec<_> = spans(moof).filter(|(k, _)| k == b"traf").map(|(_, r)| r).collect();
        for r in trafs {
            let base = r.start;
            let traf = &moof[r.clone()];
            let tfhd = child(traf, b"tfhd")?;
            let flags = u32_at(tfhd, 0).unwrap_or(0) & 0xFF_FFFF;
            let id = u32_at(tfhd, 4)? as u64;
            let track = self.tracks.iter().find(|t| t.id == id).copied()?;
            let mut at = 8;
            if flags & 0x1 != 0 { at += 8; }
            if flags & 0x2 != 0 { at += 4; }
            let mut default = track.default_duration;
            if flags & 0x8 != 0 { default = u32_at(tfhd, at).map_or(default, u64::from); }
            let tfdt = spans(traf).find(|(k, _)| k == b"tfdt").map(|(_, r)| base + r.start);
            let ts = track.timescale as f64;
            let mut start = match tfdt {
                Some(p) if moof[p] == 1 => u64_at(moof, p + 4)?,
                Some(p) => u32_at(moof, p + 4)? as u64,
                // Without tfdt the fragment follows the last one.
                None => ((self.last_end - self.offset).max(0.0) * ts) as u64,
            };
            if let (Some(p), true) = (tfdt, self.offset != 0.0) {
                let moved = rnd(start as f64 + self.offset * ts).max(0.0);
                if moof[p] == 1 {
                    start = moved as u64;
                    moof[p + 4..p + 12].copy_from_slice(&start.to_be_bytes());
                } else {
                    start = (moved as u64).min(u32::MAX as u64);
                    moof[p + 4..p + 8].copy_from_slice(&(start as u32).to_be_bytes());
                }
            } else if tfdt.is_none() {
                start = (start as f64 + self.offset.max(0.0) * ts) as u64;
            }
            let traf = &moof[r];
            let mut total = 0u64;
            for (k, trun) in boxes(traf) {
                if &k != b"trun" { continue; }
                let flags = u32_at(trun, 0).unwrap_or(0) & 0xFF_FFFF;
                let count = u32_at(trun, 4)?;
                let mut at = 8;
                if flags & 0x1 != 0 { at += 4; }
                if flags & 0x4 != 0 { at += 4; }
                let per = [0x100, 0x200, 0x400, 0x800].iter().filter(|f| flags & **f != 0).count() * 4;
                if per > 0 && trun.len() < at + per * count as usize { return None; }
                for i in 0..count as usize {
                    total += if flags & 0x100 != 0 { u32_at(trun, at + i * per).unwrap_or(0) as u64 } else { default };
                }
            }
            first = first.min(start as f64 / ts);
            last = last.max((start + total) as f64 / ts);
        }
        (first <= last).then_some((first, last))
    }

    /// The segment gathered its samples: keep it.
    fn close(&mut self) {
        if let Some(seg) = self.open.take() { self.insert(seg); }
    }

    fn insert(&mut self, seg: Seg) {
        // It replaces what it overlaps, as a browser's coded frame removal does.
        let (a, z) = (seg.start, seg.end);
        self.segs.retain(|s| !(s.start >= a - EPS && s.start < z - EPS));
        let at = self.segs.partition_point(|s| s.start < a);
        self.last_end = z;
        self.largest = self.largest.max(seg.bytes.len() as u64);
        self.segs.insert(at, seg);
        self.held = self.segs.iter().map(|s| s.bytes.len()).sum();
        self.segments += 1;
        // What it brings back is no longer cut.
        let mut cuts = Vec::new();
        for &(x, y) in &self.cuts {
            if y <= a || x >= z { cuts.push((x, y)); continue; }
            if x < a { cuts.push((x, a)); }
            if y > z { cuts.push((z, y)); }
        }
        self.cuts = cuts;
        self.ranges_again();
    }

    fn ranges_again(&mut self) {
        let mut merged: Vec<(f64, f64)> = Vec::with_capacity(self.segs.len());
        for s in &self.segs {
            if s.end <= s.start { continue; }
            match merged.last_mut() {
                Some(last) if s.start <= last.1 + JOIN => last.1 = last.1.max(s.end),
                _ => merged.push((s.start, s.end)),
            }
        }
        for &(x, y) in &self.cuts {
            let mut out = Vec::new();
            for &(a, b) in &merged {
                if b <= x || a >= y { out.push((a, b)); continue; }
                if a < x { out.push((a, x)); }
                if b > y { out.push((y, b)); }
            }
            merged = out;
        }
        self.ranges = merged;
    }

    /// `SourceBuffer.remove(start, end)`: segments wholly inside go; the seconds of
    /// any partly inside are no longer reported as buffered.
    pub fn remove(&mut self, start: f64, end: f64) {
        if end <= start { return; }
        self.segs.retain(|s| !(s.start >= start - EPS && s.end <= end + EPS));
        self.held = self.segs.iter().map(|s| s.bytes.len()).sum();
        self.cuts.push((start, end));
        self.ranges_again();
    }

    fn append_webm(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() && !self.broken {
            if self.skip > 0 {
                let n = self.skip.min(bytes.len() as u64) as usize;
                self.skip -= n as u64;
                bytes = &bytes[n..];
                continue;
            }
            if !self.fill(&mut bytes, 1) { continue; }
            if self.part[0].leading_zeros() >= 4 { self.broken = true; break; }
            let id_len = self.part[0].leading_zeros() as usize + 1;
            if !self.fill(&mut bytes, id_len + 1) { continue; }
            let Some(head) = ebml_head_len(&self.part) else { self.broken = true; break };
            if !self.fill(&mut bytes, head) { continue; }
            let (id, n) = ebml_id(&self.part).unwrap();
            let (size, _, unknown) = ebml_size(&self.part[n..]).unwrap();
            // An element of the Segment ends a Cluster of unknown size.
            if self.cluster.is_some() && TOP.contains(&id) {
                let body = self.cluster.take().unwrap();
                self.cluster(&body);
            }
            match id {
                SEGMENT => {
                    // Go inside it. The decoder's copy says "unknown size": segments follow.
                    self.part.clear();
                    let mut head = Vec::new();
                    ebml_put(&mut head, SEGMENT, &[]);
                    let l = head.len();
                    head[l - 8..].copy_from_slice(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
                    self.building.extend_from_slice(&head);
                }
                CLUSTER if unknown => { self.part.clear(); self.cluster = Some(Vec::new()); }
                EBML | INFO | TRACKS | CLUSTER => {
                    if unknown || size > BOX_MAX { self.broken = true; break; }
                    if !self.fill(&mut bytes, head + size as usize) { continue; }
                    let whole = core::mem::take(&mut self.part);
                    let body = &whole[head..];
                    match id {
                        EBML => self.building = whole,
                        INFO => {
                            if let Some((_, s)) = elements(body).find(|(i, _)| *i == TIMECODE_SCALE) { self.scale_ns = ebml_uint(s).max(1); }
                            self.building.extend_from_slice(&whole);
                        }
                        TRACKS => {
                            if !self.init_webm(body) { self.broken = true; break; }
                            let mut init = core::mem::take(&mut self.building);
                            init.extend_from_slice(&whole);
                            self.new_init(init);
                        }
                        _ => { let body = body.to_vec(); self.cluster(&body); }
                    }
                }
                _ if self.cluster.is_some() => {
                    // A child of an unknown-size Cluster: keep it whole.
                    if unknown || size > BOX_MAX { self.broken = true; break; }
                    if !self.fill(&mut bytes, head + size as usize) { continue; }
                    let whole = core::mem::take(&mut self.part);
                    let c = self.cluster.as_mut().unwrap();
                    c.extend_from_slice(&whole);
                    if c.len() as u64 > BOX_MAX { self.broken = true; break; }
                }
                _ => {
                    // SeekHead, Cues, Tags, Void...: not needed.
                    if unknown { self.broken = true; break; }
                    self.part.clear();
                    self.skip = size;
                }
            }
        }
    }

    fn init_webm(&mut self, tracks: &[u8]) -> bool {
        let mut out = Vec::new();
        for (id, entry) in elements(tracks) {
            if id != TRACK_ENTRY { continue; }
            let mut t = Track::default();
            for (id, v) in elements(entry) {
                match id {
                    TRACK_NUMBER => t.id = ebml_uint(v),
                    TRACK_TYPE => { let k = ebml_uint(v); t.video = k == 1; t.audio = k == 2; }
                    DEFAULT_DURATION => t.default_duration = ebml_uint(v),
                    _ => {}
                }
            }
            if t.id != 0 { out.push(t); }
        }
        if out.is_empty() { return false; }
        self.tracks = out;
        true
    }

    /// A whole Cluster (its body): times from its Timecode and blocks; written again
    /// with a known size and `timestampOffset` applied.
    fn cluster(&mut self, body: &[u8]) {
        if !self.inited { self.broken = true; return; }
        let scale = self.scale_ns as f64 / 1e9;
        let mut tc = 0i64;
        // (track, ticks from the Cluster's Timecode, duration in ticks if known)
        let mut blocks: Vec<(u64, i64, Option<i64>)> = Vec::new();
        let block = |b: &[u8], dur: Option<i64>, out: &mut Vec<(u64, i64, Option<i64>)>| {
            let Some((track, n, _)) = ebml_size(b) else { return };
            let Some(rel) = b.get(n..n + 2) else { return };
            out.push((track, i16::from_be_bytes([rel[0], rel[1]]) as i64, dur));
        };
        for (id, v) in elements(body) {
            match id {
                TIMECODE => tc = ebml_uint(v) as i64,
                SIMPLE_BLOCK => block(v, None, &mut blocks),
                BLOCK_GROUP => {
                    let dur = elements(v).find(|(i, _)| *i == BLOCK_DURATION).map(|(_, d)| ebml_uint(d) as i64);
                    if let Some((_, b)) = elements(v).find(|(i, _)| *i == BLOCK) { block(b, dur, &mut blocks); }
                }
                _ => {}
            }
        }
        let shift = rnd(self.offset / scale) as i64;
        let tc = (tc + shift).max(0);
        // The length of a block: its own, its track's default, or the step to the
        // track's next block.
        let mut end = i64::MIN;
        let mut start = i64::MAX;
        for (i, &(track, rel, dur)) in blocks.iter().enumerate() {
            let step = blocks[i + 1..].iter().filter(|b| b.0 == track && b.1 > rel).map(|b| b.1 - rel).min();
            let default = self.tracks.iter().find(|t| t.id == track).map(|t| t.default_duration).filter(|d| *d > 0)
                .map(|d| rnd(d as f64 / self.scale_ns as f64) as i64);
            let last_step = blocks[..i].iter().rev().find(|b| b.0 == track && b.1 < rel).map(|b| rel - b.1);
            let len = dur.or(default).or(step).or(last_step).unwrap_or(0);
            start = start.min(rel);
            end = end.max(rel + len);
        }
        if blocks.is_empty() { return; }
        let mut cluster = Vec::with_capacity(body.len() + 24);
        let mut inner = Vec::with_capacity(body.len() + 12);
        ebml_put(&mut inner, TIMECODE, &(tc as u64).to_be_bytes());
        // Copy the rest as it was.
        let mut at = 0usize;
        while at < body.len() {
            let Some((id, n)) = ebml_id(&body[at..]) else { break };
            let Some((size, m, _)) = ebml_size(&body[at + n..]) else { break };
            let next = (at + n + m + size as usize).min(body.len());
            if id != TIMECODE { inner.extend_from_slice(&body[at..next]); }
            at = next;
        }
        ebml_put(&mut cluster, CLUSTER, &inner);
        let seg = Seg { start: (tc + start) as f64 * scale, end: (tc + end) as f64 * scale, init: self.inits.len() - 1, bytes: cluster };
        self.insert(seg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }
    fn full(version: u8, flags: u32, rest: &[u8]) -> Vec<u8> {
        let mut v = vec![version];
        v.extend_from_slice(&flags.to_be_bytes()[1..]);
        v.extend_from_slice(rest);
        v
    }
    fn be(n: u32) -> [u8; 4] { n.to_be_bytes() }

    /// An init segment: one track `id` at `timescale`, handler `hdlr`, trex default.
    fn init(id: u32, timescale: u32, hdlr: &[u8; 4], default: u32) -> Vec<u8> {
        let tkhd = bx(b"tkhd", &full(0, 3, &[&be(0)[..], &be(0), &be(id), &[0; 68]].concat()));
        let mdhd = bx(b"mdhd", &full(0, 0, &[&be(0)[..], &be(0), &be(timescale), &be(0), &[0; 4]].concat()));
        let hdlr = bx(b"hdlr", &full(0, 0, &[&be(0)[..], hdlr, &[0; 13]].concat()));
        let trak = bx(b"trak", &[tkhd, bx(b"mdia", &[mdhd, hdlr].concat())].concat());
        let trex = bx(b"trex", &full(0, 0, &[&be(id)[..], &be(1), &be(default), &be(0), &be(0)].concat()));
        [bx(b"ftyp", b"iso6\0\0\0\0"), bx(b"moov", &[trak, bx(b"mvex", &trex)].concat())].concat()
    }
    /// A media segment: `count` samples from `start`, durations listed or default.
    fn segment(id: u32, start: u64, count: u32, each: Option<u32>) -> Vec<u8> {
        let tfhd = bx(b"tfhd", &full(0, 0x20000, &be(id)));
        let tfdt = bx(b"tfdt", &full(1, 0, &start.to_be_bytes()));
        let mut run = vec![];
        run.extend_from_slice(&be(count));
        run.extend_from_slice(&be(0)); // data offset
        let flags = match each { Some(d) => { for _ in 0..count { run.extend_from_slice(&be(d)); run.extend_from_slice(&be(100)); } 0x301 } None => 0x1 };
        let trun = bx(b"trun", &full(0, flags, &run));
        let moof = bx(b"moof", &[bx(b"mfhd", &full(0, 0, &be(1))), bx(b"traf", &[tfhd, tfdt, trun].concat())].concat());
        [bx(b"styp", b"msdh\0\0\0\0"), moof, bx(b"mdat", &vec![0xAB; 5000])].concat()
    }

    #[test]
    fn types_youtube_asks_about() {
        assert!(supported("video/mp4; codecs=\"avc1.4d401f\""));
        assert!(supported("audio/mp4; codecs=\"mp4a.40.2\""));
        assert!(supported("video/mp4; codecs=\"av01.0.05M.08\""));
        assert!(supported("video/mp4"));
        assert_eq!(format("video/webm; codecs=\"vp9\""), Some(Format::WebM));
        assert_eq!(format("video/webm; codecs=\"vp09.00.51.08.01.01.01.01.00\""), Some(Format::WebM));
        assert_eq!(format("audio/webm; codecs=\"opus\""), Some(Format::WebM));
        assert!(!supported("video/webm; codecs=\"avc1.4d401f\""));
        assert!(!supported("video/mp4; codecs=\"hev1.1.6.L93.B0\""));
        assert!(!supported("audio/mp4; codecs=\"ec-3\""));
        assert!(!supported("video/x-matroska"));
    }

    #[test]
    fn follows_segment_times_fed_in_odd_pieces() {
        let mut s = Stream::new(Format::Mp4);
        let mut all = init(1, 90000, b"vide", 3000);
        all.extend(segment(1, 0, 60, None));          // 0..2 s with trex's default
        all.extend(segment(1, 180_000, 30, Some(6000))); // 2..4 s with listed durations
        for piece in all.chunks(77) { assert!(s.append(piece)); }
        assert!(s.inited && s.has_video() && !s.has_audio());
        assert_eq!(s.segments, 2);
        assert_eq!(s.buffered(), &[(0.0, 4.0)]);
        // Each segment keeps its moof and mdat, styp left out.
        assert_eq!(&s.segs()[0].bytes[4..8], b"moof");
        assert_eq!(s.segs()[0].bytes.len(), segment(1, 0, 60, None).len() - 16);
        // A gap: 10..12 s stays a range of its own.
        assert!(s.append(&segment(1, 900_000, 60, None)));
        assert_eq!(s.buffered(), &[(0.0, 4.0), (10.0, 12.0)]);
        assert_eq!(s.seg_at(3.0), Some(1));
        assert_eq!(s.next_after(0.0, 2.0), Some(1));
        assert_eq!(s.next_after(2.0, 4.0), None);
        s.remove(1.0, 11.0);
        assert_eq!(s.buffered(), &[(0.0, 1.0), (11.0, 12.0)]);
        // The segment wholly inside went; those partly inside stay for the decoder.
        assert_eq!(s.segs().len(), 2);
        // Appending 2..4 again brings it back.
        assert!(s.append(&segment(1, 180_000, 30, Some(6000))));
        assert_eq!(s.buffered(), &[(0.0, 1.0), (2.0, 4.0), (11.0, 12.0)]);
    }

    #[test]
    fn out_of_order_and_replaced_segments() {
        let mut s = Stream::new(Format::Mp4);
        assert!(s.append(&init(1, 1000, b"vide", 0)));
        assert!(s.append(&segment(1, 4000, 2, Some(1000))));
        assert!(s.append(&segment(1, 0, 2, Some(1000))));
        assert!(s.append(&segment(1, 2000, 2, Some(1000))));
        let starts: Vec<f64> = s.segs().iter().map(|g| g.start).collect();
        assert_eq!(starts, vec![0.0, 2.0, 4.0]);
        assert_eq!(s.buffered(), &[(0.0, 6.0)]);
        // The same times again replace the old segment.
        assert!(s.append(&segment(1, 2000, 2, Some(1000))));
        assert_eq!(s.segs().len(), 3);
        // A new init segment (a quality switch) is the next one; segments name it.
        assert!(s.append(&init(1, 1000, b"vide", 7)));
        assert!(s.append(&segment(1, 6000, 2, Some(1000))));
        assert_eq!(s.inits.len(), 2);
        assert_eq!(s.segs()[3].init, 1);
        // The same init again is not a switch.
        assert!(s.append(&init(1, 1000, b"vide", 7)));
        assert_eq!(s.inits.len(), 2);
    }

    #[test]
    fn timestamp_offset_moves_segments() {
        let mut s = Stream::new(Format::Mp4);
        assert!(s.append(&init(1, 1000, b"vide", 0)));
        s.offset = 10.0;
        assert!(s.append(&segment(1, 0, 2, Some(1000))));
        assert_eq!(s.buffered(), &[(10.0, 12.0)]);
        // Written into the tfdt the decoder reads.
        let seg = &s.segs()[0].bytes;
        let traf = boxes(&seg[8..]).find(|(k, _)| k == b"traf").unwrap().1;
        let tfdt = child(traf, b"tfdt").unwrap();
        assert_eq!(u64_at(tfdt, 4), Some(10_000));
    }

    #[test]
    fn sound_track_and_refusals() {
        let mut s = Stream::new(Format::Mp4);
        assert!(s.append(&init(2, 48000, b"soun", 1024)));
        assert!(s.has_audio());
        assert!(s.append(&segment(2, 48000, 47, None)));
        let r = s.buffered()[0];
        assert!((r.0 - 1.0).abs() < 1e-9 && (r.1 - (1.0 + 47.0 * 1024.0 / 48000.0)).abs() < 1e-9);
        // A fragment for a track the init segment never named.
        assert!(!s.append(&segment(9, 0, 1, None)));
        // A segment before any init segment.
        let mut t = Stream::new(Format::Mp4);
        assert!(!t.append(&segment(1, 0, 1, None)));
    }

    // WebM, written by hand: EBML header, unknown-size Segment, Info, Tracks, Clusters.
    fn el(id: u32, body: &[u8]) -> Vec<u8> { let mut v = Vec::new(); ebml_put(&mut v, id, body); v }
    fn webm_init() -> Vec<u8> {
        let mut v = el(EBML, &el(0x4282, b"webm"));
        v.extend_from_slice(&[0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        v.extend(el(INFO, &el(TIMECODE_SCALE, &1_000_000u32.to_be_bytes())));
        let entry = [el(TRACK_NUMBER, &[1]), el(TRACK_TYPE, &[1]), el(DEFAULT_DURATION, &40_000_000u32.to_be_bytes())].concat();
        v.extend(el(0x114D_9B74, &[0; 10])); // a SeekHead, skipped
        v.extend(el(TRACKS, &el(TRACK_ENTRY, &entry)));
        v
    }
    fn webm_cluster(tc: u64, frames: i16) -> Vec<u8> {
        let mut body = el(TIMECODE, &tc.to_be_bytes());
        for i in 0..frames {
            let mut b = vec![0x81];
            b.extend_from_slice(&(i * 40).to_be_bytes());
            b.push(0x80);
            b.extend_from_slice(&[0x55; 300]);
            body.extend(el(SIMPLE_BLOCK, &b));
        }
        el(CLUSTER, &body)
    }

    #[test]
    fn webm_clusters() {
        let mut s = Stream::new(Format::WebM);
        let mut all = webm_init();
        all.extend(webm_cluster(0, 25));    // 0..1 s
        all.extend(webm_cluster(1000, 25)); // 1..2 s
        for p in all.chunks(33) { assert!(s.append(p)); }
        assert!(s.inited && s.has_video());
        assert_eq!(s.buffered(), &[(0.0, 2.0)]);
        assert_eq!(s.inits[0][..4], [0x1A, 0x45, 0xDF, 0xA3]);
        // The init the decoder gets has no SeekHead.
        assert!(!s.inits[0].windows(4).any(|w| w == [0x11, 0x4D, 0x9B, 0x74]));
        // An offset moves the Cluster's Timecode.
        s.offset = 5.0;
        assert!(s.append(&webm_cluster(0, 25)));
        assert_eq!(s.buffered(), &[(0.0, 2.0), (5.0, 6.0)]);
        let body = elements(&s.segs()[2].bytes).next().unwrap().1;
        let tc = elements(body).find(|(i, _)| *i == TIMECODE).unwrap().1;
        assert_eq!(ebml_uint(tc), 5000);
    }

    #[test]
    fn webm_unknown_size_clusters() {
        let mut s = Stream::new(Format::WebM);
        let mut all = webm_init();
        for tc in [0u64, 1000] {
            let c = webm_cluster(tc, 25);
            // Same Cluster with its size marked unknown.
            all.extend_from_slice(&[0x1F, 0x43, 0xB6, 0x75, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
            all.extend_from_slice(&c[12..]);
        }
        for p in all.chunks(100) { assert!(s.append(p)); }
        // The second one ends only at the end of the stream.
        assert_eq!(s.buffered(), &[(0.0, 1.0)]);
        s.finish();
        assert_eq!(s.buffered(), &[(0.0, 2.0)]);
    }
}

#[cfg(test)]
mod made_by_ffmpeg {
    use super::*;
    // The MSE gate's fixtures, 4 s each: fragmented MP4 as FFmpeg writes it for DASH
    // (frag_keyframe+empty_moov+default_base_moof), and WebM (VP9, Opus).
    static VIDEO: &[u8] = include_bytes!("../../../scripts/web_site/tube/mse-video.mp4");
    static AUDIO: &[u8] = include_bytes!("../../../scripts/web_site/tube/mse-audio.mp4");
    static WEBM_VIDEO: &[u8] = include_bytes!("../../../scripts/web_site/tube/mse-video.webm");
    static WEBM_AUDIO: &[u8] = include_bytes!("../../../scripts/web_site/tube/mse-audio.webm");

    #[test]
    fn real_segments_cover_four_seconds() {
        for (format, bytes, video, piece) in [(Format::Mp4, VIDEO, true, 32 * 1024), (Format::Mp4, AUDIO, false, 1000),
                                              (Format::WebM, WEBM_VIDEO, true, 7000), (Format::WebM, WEBM_AUDIO, false, 999)] {
            let mut s = Stream::new(format);
            for p in bytes.chunks(piece) { assert!(s.append(p)); }
            s.finish();
            assert_eq!(s.has_video(), video);
            assert_eq!(s.has_audio(), !video);
            assert!(s.segments >= 2, "{}", s.segments);
            let r = s.buffered();
            assert_eq!(r.len(), 1, "{:?} {:?}", format, r);
            assert!(r[0].0 < 0.05 && (r[0].1 - 4.0).abs() < 0.05, "{:?} {:?}", format, r);
        }
    }
}
