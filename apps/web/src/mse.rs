// SPDX-License-Identifier: GPL-3.0-only
//! Media Source Extensions for `<video>` and `<audio>`: the page appends byte stream
//! segments to SourceBuffers; sex-web's `mse::Stream` keeps them by time, and each
//! SourceBuffer's decoder reads its init segment then the segments in time order from
//! where playback is. Seeking starts that stream over at the segment holding the new
//! time (`Source::restart`); a new init segment (a quality switch) does the same where
//! its segments begin. Separate video and audio SourceBuffers (as YouTube, dash.js and
//! hls.js use them) play on one clock (`Player::pair`).
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use sex_media::{Decoder, Source};
use sex_web::mse::{Format, Stream};

/// Bytes kept behind the decoder's read position (FFmpeg may step back a little).
const KEEP_BEHIND: u64 = 2 << 20;
/// What a feed must hold before a decoder opens on it, unless it ended: FFmpeg reads
/// the init segment and looks into the first fragments.
const OPEN_AHEAD: u64 = 256 * 1024;
const OPEN_SECONDS: f64 = 1.0;

/// One SourceBuffer: its segments, and the stream its decoder reads.
struct Shared {
    stream: Stream,
    ended: bool,
    /// The decoder's stream: bytes from `base` on, made of an init segment and the
    /// segments handed out after it.
    out: Vec<u8>,
    base: u64,
    /// The last segment handed out (start, end), and the init segment in force.
    last: Option<(f64, f64)>,
    init: usize,
    /// The next segment follows another init segment: it starts here.
    switch: Option<f64>,
    /// The decoder ran out of segments and waits for the next one.
    parked: bool,
}

impl Shared {
    /// The segment that plays on after the last one handed out.
    fn next(&self) -> Option<usize> {
        let (a, z) = self.last?;
        self.stream.next_after(a, z)
    }

    /// Hands out the next segment, unless it needs another init segment.
    fn pull(&mut self) -> bool {
        let Some(i) = self.next() else { return false };
        let seg = &self.stream.segs()[i];
        if seg.init != self.init {
            self.switch = Some(seg.start);
            return false;
        }
        self.out.extend_from_slice(&seg.bytes);
        self.last = Some((seg.start, seg.end));
        true
    }

    /// A new stream that plays from `t`: the init segment, then the segment holding it.
    fn start_at(&mut self, t: f64) -> bool {
        let Some(i) = self.stream.seg_at(t) else { return false };
        self.start_with(i);
        true
    }

    /// A new stream: segment `i`'s init segment, then it.
    fn start_with(&mut self, i: usize) {
        let seg = &self.stream.segs()[i];
        let mut out = self.stream.inits[seg.init].clone();
        out.extend_from_slice(&seg.bytes);
        self.out = out;
        self.base = 0;
        self.init = seg.init;
        self.last = Some((seg.start, seg.end));
        self.switch = None;
        self.parked = false;
    }

    /// Whether nothing more will come: the page ended the stream and every segment
    /// that plays on has been handed out.
    fn done(&self) -> bool { self.ended && self.next().is_none() && self.switch.is_none() }

    /// Whether the decoder's stream ends here: nothing more will come, or what comes
    /// next needs another init segment.
    fn stops(&self) -> bool { self.done() || self.switch.is_some() }

    fn end(&self) -> u64 { self.base + self.out.len() as u64 }
}

/// The decoder's side of a SourceBuffer.
struct Feed(Rc<RefCell<Shared>>);

impl Source for Feed {
    fn len(&self) -> Option<u64> {
        let b = self.0.borrow();
        b.stops().then(|| b.end())
    }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize> {
        let mut b = self.0.borrow_mut();
        if at < b.base { return None; }
        while b.end() < at + out.len() as u64 && b.pull() {}
        let i = (at - b.base) as usize;
        let n = out.len().min(b.out.len().saturating_sub(i));
        out[..n].copy_from_slice(&b.out[i..i + n]);
        // Past what has arrived (always at a segment's end): not yet. FFmpeg answers
        // "later" and the decoder waits for more.
        if n == 0 && !b.stops() { return None; }
        // Let go of what is long behind.
        if at > b.base + KEEP_BEHIND * 2 {
            let drop = (at - KEEP_BEHIND - b.base) as usize;
            b.out.drain(..drop);
            b.base += drop as u64;
        }
        Some(n)
    }
    fn available(&self, at: u64) -> u64 {
        let mut b = self.0.borrow_mut();
        if b.parked { return if b.next().is_some() || b.ended { u64::MAX } else { 0 }; }
        // Whole segments only: running out is then always at a segment's end, where
        // FFmpeg can wait and go on.
        while b.end() < at + 256 * 1024 && b.pull() {}
        if b.stops() { return u64::MAX; }
        b.end().saturating_sub(at)
    }
    fn growing(&self) -> bool { true }
    // Decoding goes on whenever there is anything: FFmpeg says when it ran out.
    fn ahead(&self) -> u64 { 0 }
    fn restart(&mut self, ms: u64) -> bool {
        self.0.borrow_mut().start_at(ms as f64 / 1000.0)
    }
    fn next_stream(&mut self) -> Option<bool> {
        let mut b = self.0.borrow_mut();
        if let Some(at) = b.switch {
            // A new init segment (a quality switch): a new stream from there.
            sex_pdx::serial_record!("[web.mse.switch] at={}", at);
            return Some(b.start_at(at));
        }
        if b.done() { return None; }
        // Ran out: the decoder carries on with the next segment once it comes.
        match b.next() {
            Some(i) => { b.start_with(i); Some(true) }
            None => { b.parked = true; Some(false) }
        }
    }
}

pub struct Buffer {
    pub kind: String,
    shared: Rc<RefCell<Shared>>,
    pub dropped: bool,
}

impl Buffer {
    /// The `mse` dispatch for this buffer: `ok`, then its ranges as A-B,C-D.
    fn reply(&self, node: usize, index: usize, ok: bool) -> String {
        let b = self.shared.borrow();
        let ranges: Vec<String> = b.stream.buffered().iter().map(|(a, z)| alloc::format!("{}-{}", a, z)).collect();
        alloc::format!("mse\t{}\t{}\t{}\t{}", node, index, ok as u8, ranges.join(","))
    }

    /// Whether the stream can play from `t` now: its segment is here with enough after
    /// it, or the page ended the stream.
    fn ready_at(&self, t: f64) -> bool {
        let b = self.shared.borrow();
        let s = &b.stream;
        let Some(i) = s.seg_at(t) else { return false };
        let need = OPEN_AHEAD.min(s.largest * 2 + 64 * 1024).max(64 * 1024);
        let (bytes, until) = s.ahead_from(i, need);
        // Enough bytes for FFmpeg to open on, or a second of playing.
        b.ended || bytes >= need || until - t >= OPEN_SECONDS
    }
}

/// An element's MediaSource.
#[derive(Default)]
pub struct Mse {
    pub buffers: Vec<Buffer>,
    /// Seconds, as the page set it (NaN until then).
    pub duration: f64,
    pub ended: bool,
    /// Dispatches for the page, in order.
    pub replies: Vec<String>,
    pub appended: u64,
}

fn arg2(arg: &str) -> Option<(usize, &str)> {
    let (i, rest) = arg.split_once(' ').unwrap_or((arg, ""));
    Some((i.parse().ok()?, rest))
}

impl Mse {
    pub fn new() -> Mse { Mse { duration: f64::NAN, ..Default::default() } }

    pub fn add(&mut self, kind: &str) {
        let format = sex_web::mse::format(kind).unwrap_or(Format::Mp4);
        let shared = Shared { stream: Stream::new(format), ended: false, out: Vec::new(), base: 0, last: None, init: 0, switch: None, parked: false };
        self.buffers.push(Buffer { kind: String::from(kind), shared: Rc::new(RefCell::new(shared)), dropped: false });
    }

    fn live(&self) -> impl Iterator<Item = &Buffer> { self.buffers.iter().filter(|b| !b.dropped) }

    pub fn append(&mut self, node: usize, index: usize, data: &[u8]) {
        let Some(b) = self.buffers.get(index) else { return };
        let ok = {
            let mut s = b.shared.borrow_mut();
            let ok = !b.dropped && s.stream.append(data);
            if ok { s.ended = false; }
            ok
        };
        if ok {
            self.appended += data.len() as u64;
        } else {
            let s = b.shared.borrow();
            sex_pdx::serial_record!("[web.mse.refused] node={} buffer={} bytes={} broken={} held={}", node, index, data.len(), s.stream.broken as u8, s.stream.held());
        }
        let r = b.reply(node, index, ok);
        self.replies.push(r);
    }

    /// `remove`, `offset`, `abort` and `type` for buffer N ("N rest").
    pub fn buffer_request(&mut self, node: usize, verb: &str, arg: &str) {
        let Some((i, rest)) = arg2(arg) else { return };
        let Some(b) = self.buffers.get(i) else { return };
        let mut s = b.shared.borrow_mut();
        match verb {
            "remove" => {
                let mut f = rest.split(' ').map(|v| v.parse::<f64>().ok());
                let (Some(Some(a)), Some(Some(z))) = (f.next(), f.next()) else { return };
                s.stream.remove(a, z);
            }
            "offset" => s.stream.offset = rest.parse().unwrap_or(0.0),
            "abort" => s.stream.reset_parser(),
            "type" => if let Some(f) = sex_web::mse::format(rest) {
                if f != s.stream.format { s.stream.format = f; s.stream.reset_parser(); }
            },
            _ => {}
        }
        drop(s);
        if verb == "remove" {
            let r = b.reply(node, i, true);
            self.replies.push(r);
        }
    }

    pub fn end(&mut self) {
        self.ended = true;
        for b in &self.buffers {
            let mut s = b.shared.borrow_mut();
            s.stream.finish();
            s.ended = true;
        }
    }
    pub fn reopen(&mut self) {
        self.ended = false;
        for b in &self.buffers { b.shared.borrow_mut().ended = false; }
    }

    /// Whether every buffer can play from `t` now.
    pub fn ready_at(&self, t: f64) -> bool {
        let mut live = self.live().peekable();
        live.peek().is_some() && live.all(|b| b.shared.borrow().stream.inited && b.ready_at(t))
    }

    /// Where the page's segments begin, for a stream that does not start at 0.
    pub fn first(&self) -> Option<f64> {
        self.live().filter_map(|b| b.shared.borrow().stream.segs().first().map(|s| s.start)).reduce(f64::max)
    }

    /// Tells the buffers where playback is, so full ones can let go of what was played.
    pub fn playhead(&self, t: f64) {
        for b in &self.buffers { b.shared.borrow_mut().stream.playhead = t; }
    }

    /// Decoders for the video and the sound buffers, reading from `t` (one may be
    /// missing; a single buffer may carry both).
    pub fn decoders(&self, t: f64) -> Result<(Option<Decoder>, Option<Decoder>), sex_media::Error> {
        let mut video = None;
        let mut audio = None;
        for b in self.live() {
            if !b.shared.borrow_mut().start_at(t) { return Err(sex_media::Error::UNSUPPORTED); }
            let dec = Decoder::open(Box::new(Feed(b.shared.clone())))?;
            sex_pdx::serial_record!("[web.mse.decoder] kind={} format={} video={} audio={} w={} h={} at={}", b.kind, dec.info.format, dec.info.video_codec, dec.info.audio_codec, dec.info.width, dec.info.height, t);
            let has_video = b.shared.borrow().stream.has_video();
            if has_video && video.is_none() { video = Some(dec); } else if audio.is_none() { audio = Some(dec); }
        }
        Ok((video, audio))
    }
}
