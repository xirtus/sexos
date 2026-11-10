// SPDX-License-Identifier: GPL-3.0-only
//! Media Source Extensions bookkeeping: which byte streams a SourceBuffer takes, and
//! what time each one covers so far (`buffered`), read from the segments as they are
//! appended. Players such as YouTube's, dash.js and hls.js decide what to fetch next
//! from `buffered`, so it must follow the real segment times. The bytes themselves go
//! to the decoder unchanged.
//!
//! Fragmented MP4 (ISO BMFF, the byte stream format YouTube, DASH and HLS fMP4 use):
//! an initialization segment (`ftyp` `moov`) then media segments (`styp`? `sidx`?
//! `moof` `mdat`). Times come from `moof/traf`: `tfdt` (the start) plus the sample
//! durations in `trun` (or the defaults in `tfhd`/`trex`), over `mdhd`'s timescale.
use alloc::vec::Vec;

/// Whether a SourceBuffer of this type can be made (`MediaSource.isTypeSupported`).
/// Fragmented MP4 with H.264, AV1, AAC or MP3 inside; WebM waits for its own parser.
pub fn supported(mime: &str) -> bool {
    let mut parts = mime.split(';');
    let base = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    if base != "video/mp4" && base != "audio/mp4" { return false; }
    for p in parts {
        let p = p.trim();
        let Some(v) = p.strip_prefix("codecs=").or_else(|| p.strip_prefix("codecs =")) else { continue };
        for c in v.trim().trim_matches('"').split(',') {
            let c = c.trim().to_ascii_lowercase();
            let family = c.split('.').next().unwrap_or("");
            if !matches!(family, "avc1" | "avc3" | "av01" | "mp4a" | "mp3") { return false; }
            // AAC-LC, HE-AAC and MP3 in mp4a.40.x / mp4a.6b.
            if family == "mp4a" && !(c == "mp4a" || c.starts_with("mp4a.40") || c == "mp4a.6b" || c == "mp4a.69") { return false; }
        }
    }
    true
}

#[derive(Clone, Copy, Default, Debug)]
struct Track { id: u32, timescale: u32, default_duration: u32, video: bool, audio: bool }

/// What one SourceBuffer has been given: parsed box by box as bytes arrive.
#[derive(Default)]
pub struct Stream {
    /// Bytes of a box not whole yet (box headers and `moov`/`moof` only: `mdat` is skipped).
    part: Vec<u8>,
    /// Bytes of the current box still to skip.
    skip: u64,
    tracks: Vec<Track>,
    /// Seconds covered, sorted and merged.
    ranges: Vec<(f64, f64)>,
    pub inited: bool,
    pub segments: u32,
    /// The largest box seen, in bytes (a fragment's samples, usually).
    pub largest: u64,
    pub broken: bool,
}

/// Two ranges closer than this are one (browsers allow a small gap between segments).
const JOIN: f64 = 0.1;
/// A `moov` or `moof` larger than this is refused rather than held.
const BOX_MAX: u64 = 8 << 20;

fn u32_at(b: &[u8], at: usize) -> Option<u32> { b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])) }
fn u64_at(b: &[u8], at: usize) -> Option<u64> { b.get(at..at + 8).map(|s| u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])) }

/// The child boxes of `body`: (type, contents).
fn boxes(body: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
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
        let inner = &body[at + head..at + size as usize];
        at += size as usize;
        Some((kind, inner))
    })
}
fn child<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> { boxes(body).find(|(k, _)| k == kind).map(|(_, b)| b) }

impl Stream {
    pub fn new() -> Stream { Stream::default() }

    /// Seconds buffered, as (start, end) pairs in order.
    pub fn buffered(&self) -> &[(f64, f64)] { &self.ranges }
    pub fn has_video(&self) -> bool { self.tracks.iter().any(|t| t.video) }
    pub fn has_audio(&self) -> bool { self.tracks.iter().any(|t| t.audio) }

    /// Reads appended bytes. False once the stream can't be followed (a box too big or
    /// malformed): the page gets an error, as a browser's would.
    pub fn append(&mut self, mut bytes: &[u8]) -> bool {
        while !bytes.is_empty() && !self.broken {
            if self.skip > 0 {
                let n = (self.skip.min(bytes.len() as u64)) as usize;
                self.skip -= n as u64;
                bytes = &bytes[n..];
                continue;
            }
            // The header first (8 bytes, or 16 with a 64-bit size).
            let head = if self.part.len() >= 4 && u32_at(&self.part, 0) == Some(1) { 16 } else { 8 };
            if self.part.len() < head {
                let n = (head - self.part.len()).min(bytes.len());
                self.part.extend_from_slice(&bytes[..n]);
                bytes = &bytes[n..];
                continue;
            }
            let Some(len) = self.box_len() else { self.broken = true; break };
            if len < head as u64 { self.broken = true; break; }
            if len < u64::MAX >> 1 { self.largest = self.largest.max(len); }
            let kind: [u8; 4] = self.part[4..8].try_into().unwrap();
            if kind != *b"moov" && kind != *b"moof" {
                // Not needed for timing (mdat, sidx, styp...): skip the rest of it.
                self.skip = len - head as u64;
                self.part.clear();
                continue;
            }
            if len > BOX_MAX { self.broken = true; break; }
            if (self.part.len() as u64) < len {
                let n = ((len - self.part.len() as u64) as usize).min(bytes.len());
                self.part.extend_from_slice(&bytes[..n]);
                bytes = &bytes[n..];
                if (self.part.len() as u64) < len { continue; }
            }
            let whole = core::mem::take(&mut self.part);
            let head = if u32_at(&whole, 0) == Some(1) { 16 } else { 8 };
            let ok = if kind == *b"moov" { self.init(&whole[head..]) } else { self.fragment(&whole[head..]) };
            if !ok { self.broken = true; }
        }
        !self.broken
    }

    /// The length of the box in `part`, once its header is here.
    fn box_len(&self) -> Option<u64> {
        let size = u32_at(&self.part, 0)?;
        match size {
            1 => u64_at(&self.part, 8),
            0 => Some(u64::MAX >> 1),
            s => Some(s as u64),
        }
    }

    fn init(&mut self, moov: &[u8]) -> bool {
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
                (Some(id), Some(ts)) if ts > 0 => tracks.push(Track { id, timescale: ts, default_duration: 0, video: handler == Some(b"vide"), audio: handler == Some(b"soun") }),
                _ => return false,
            }
        }
        if let Some(mvex) = child(moov, b"mvex") {
            for (k, trex) in boxes(mvex) {
                if &k != b"trex" { continue; }
                if let (Some(id), Some(d)) = (u32_at(trex, 4), u32_at(trex, 12)) {
                    if let Some(t) = tracks.iter_mut().find(|t| t.id == id) { t.default_duration = d; }
                }
            }
        }
        if tracks.is_empty() { return false; }
        self.tracks = tracks;
        self.inited = true;
        true
    }

    fn fragment(&mut self, moof: &[u8]) -> bool {
        if !self.inited { return false; }
        for (k, traf) in boxes(moof) {
            if &k != b"traf" { continue; }
            let Some(tfhd) = child(traf, b"tfhd") else { return false };
            let flags = u32_at(tfhd, 0).unwrap_or(0) & 0xFF_FFFF;
            let Some(id) = u32_at(tfhd, 4) else { return false };
            let Some(track) = self.tracks.iter().find(|t| t.id == id).copied() else { return false };
            let mut at = 8;
            if flags & 0x1 != 0 { at += 8; }
            if flags & 0x2 != 0 { at += 4; }
            let mut default = track.default_duration;
            if flags & 0x8 != 0 { default = u32_at(tfhd, at).unwrap_or(default); }
            let start = match child(traf, b"tfdt") {
                Some(t) if t.first() == Some(&1) => u64_at(t, 4),
                Some(t) => u32_at(t, 4).map(u64::from),
                None => None,
            };
            // Without tfdt the fragment follows the last one.
            let start = match start { Some(s) => s, None => self.ranges.last().map_or(0, |r| (r.1 * track.timescale as f64) as u64) };
            let mut total = 0u64;
            for (k, trun) in boxes(traf) {
                if &k != b"trun" { continue; }
                let flags = u32_at(trun, 0).unwrap_or(0) & 0xFF_FFFF;
                let Some(count) = u32_at(trun, 4) else { return false };
                let mut at = 8;
                if flags & 0x1 != 0 { at += 4; }
                if flags & 0x4 != 0 { at += 4; }
                let per = [0x100, 0x200, 0x400, 0x800].iter().filter(|f| flags & **f != 0).count() * 4;
                if per > 0 && trun.len() < at + per * count as usize { return false; }
                for i in 0..count as usize {
                    let d = if flags & 0x100 != 0 { u32_at(trun, at + i * per).unwrap_or(0) } else { default };
                    total += d as u64;
                }
            }
            let ts = track.timescale as f64;
            self.add(start as f64 / ts, (start + total) as f64 / ts);
        }
        self.segments += 1;
        true
    }

    fn add(&mut self, a: f64, b: f64) {
        if b <= a { return; }
        self.ranges.push((a, b));
        self.ranges.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(core::cmp::Ordering::Equal));
        let mut merged: Vec<(f64, f64)> = Vec::with_capacity(self.ranges.len());
        for r in self.ranges.drain(..) {
            match merged.last_mut() {
                Some(last) if r.0 <= last.1 + JOIN => last.1 = last.1.max(r.1),
                _ => merged.push(r),
            }
        }
        self.ranges = merged;
    }

    /// `SourceBuffer.remove(start, end)`: those seconds are no longer buffered.
    pub fn remove(&mut self, start: f64, end: f64) {
        let mut out = Vec::new();
        for &(a, b) in &self.ranges {
            if b <= start || a >= end { out.push((a, b)); continue; }
            if a < start { out.push((a, start)); }
            if b > end { out.push((end, b)); }
        }
        self.ranges = out;
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
        assert!(!supported("video/webm; codecs=\"vp9\""));
        assert!(!supported("video/mp4; codecs=\"hev1.1.6.L93.B0\""));
        assert!(!supported("audio/mp4; codecs=\"ec-3\""));
    }

    #[test]
    fn follows_segment_times_fed_in_odd_pieces() {
        let mut s = Stream::new();
        let mut all = init(1, 90000, b"vide", 3000);
        all.extend(segment(1, 0, 60, None));          // 0..2 s with trex's default
        all.extend(segment(1, 180_000, 30, Some(6000))); // 2..4 s with listed durations
        for piece in all.chunks(77) { assert!(s.append(piece)); }
        assert!(s.inited && s.has_video() && !s.has_audio());
        assert_eq!(s.segments, 2);
        assert_eq!(s.buffered(), &[(0.0, 4.0)]);
        // A gap: 10..12 s stays a range of its own.
        assert!(s.append(&segment(1, 900_000, 60, None)));
        assert_eq!(s.buffered(), &[(0.0, 4.0), (10.0, 12.0)]);
        s.remove(1.0, 11.0);
        assert_eq!(s.buffered(), &[(0.0, 1.0), (11.0, 12.0)]);
    }

    #[test]
    fn sound_track_and_refusals() {
        let mut s = Stream::new();
        assert!(s.append(&init(2, 48000, b"soun", 1024)));
        assert!(s.has_audio());
        assert!(s.append(&segment(2, 48000, 47, None)));
        let r = s.buffered()[0];
        assert!((r.0 - 1.0).abs() < 1e-9 && (r.1 - (1.0 + 47.0 * 1024.0 / 48000.0)).abs() < 1e-9);
        // A fragment for a track the init segment never named.
        assert!(!s.append(&segment(9, 0, 1, None)));
        // A segment before any init segment.
        let mut t = Stream::new();
        assert!(!t.append(&segment(1, 0, 1, None)));
    }
}

#[cfg(test)]
mod made_by_ffmpeg {
    use super::*;
    // Fragmented MP4 as FFmpeg writes it for DASH (frag_keyframe+empty_moov+
    // default_base_moof): the MSE gate's fixtures. 4 s each.
    static VIDEO: &[u8] = include_bytes!("../../../scripts/web_site/tube/mse-video.mp4");
    static AUDIO: &[u8] = include_bytes!("../../../scripts/web_site/tube/mse-audio.mp4");

    #[test]
    fn real_segments_cover_four_seconds() {
        for (bytes, video, piece) in [(VIDEO, true, 32 * 1024), (AUDIO, false, 1000)] {
            let mut s = Stream::new();
            for p in bytes.chunks(piece) { assert!(s.append(p)); }
            assert_eq!(s.has_video(), video);
            assert_eq!(s.has_audio(), !video);
            assert!(s.segments >= 4, "{}", s.segments);
            let r = s.buffered();
            assert_eq!(r.len(), 1, "{:?}", r);
            assert!(r[0].0 < 0.05 && (r[0].1 - 4.0).abs() < 0.05, "{:?}", r);
        }
    }
}
