// SPDX-License-Identifier: GPL-3.0-only
//! Media Source Extensions for `<video>` and `<audio>`: the page appends byte stream
//! segments to SourceBuffers, and each SourceBuffer becomes a growing stream that
//! sex-media decodes as it arrives. Separate video and audio SourceBuffers (as
//! YouTube, dash.js and hls.js use them) play on one clock (`Player::pair`).
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use sex_media::{Decoder, Source};
use sex_web::mse::Stream;

/// Bytes kept behind the decoder's read position (FFmpeg may step back a little).
const KEEP_BEHIND: u64 = 2 << 20;
/// What a feed must hold before a decoder opens on it, unless it ended: FFmpeg reads
/// the init segment and looks into the first fragments.
const OPEN_AHEAD: u64 = 256 * 1024;

/// One SourceBuffer's bytes, as appended. Bytes long read are let go.
#[derive(Default)]
struct Bytes {
    base: u64,
    data: Vec<u8>,
    ended: bool,
    /// The largest box seen (a whole fragment's samples): decoding goes on only while
    /// this much (plus FFmpeg's read size) lies ahead.
    largest: u64,
}

/// The decoder's side of a feed.
struct Feed(Rc<RefCell<Bytes>>);

impl Source for Feed {
    fn len(&self) -> Option<u64> {
        let b = self.0.borrow();
        b.ended.then(|| b.base + b.data.len() as u64)
    }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize> {
        let mut b = self.0.borrow_mut();
        if at < b.base { return None; }
        let i = (at - b.base) as usize;
        let n = out.len().min(b.data.len().saturating_sub(i));
        out[..n].copy_from_slice(&b.data[i..i + n]);
        if n == 0 && !b.ended {
            // Read past what has arrived: the player keeps decoding to `ahead`, so this
            // means a stream FFmpeg can't follow without more than the page gave.
            sex_pdx::serial_record!("[web.mse.starved] at={} have={}", at, b.base + b.data.len() as u64);
            return None;
        }
        // Let go of what is long behind.
        if at > b.base + KEEP_BEHIND * 2 {
            let drop = (at - KEEP_BEHIND - b.base) as usize;
            b.data.drain(..drop);
            b.base += drop as u64;
        }
        Some(n)
    }
    fn available(&self, at: u64) -> u64 {
        let b = self.0.borrow();
        if b.ended { return u64::MAX; }
        (b.base + b.data.len() as u64).saturating_sub(at)
    }
    fn growing(&self) -> bool { !self.0.borrow().ended }
    fn ahead(&self) -> u64 {
        let b = self.0.borrow();
        (b.largest + 64 * 1024).min(sex_media::AHEAD)
    }
}

pub struct Buffer {
    pub kind: String,
    bytes: Rc<RefCell<Bytes>>,
    stream: Stream,
    pub dropped: bool,
}

impl Buffer {
    /// The `mse` dispatch for this buffer: `ok`, then its ranges as A-B,C-D.
    fn reply(&self, node: usize, index: usize, ok: bool) -> String {
        let ranges: Vec<String> = self.stream.buffered().iter().map(|(a, b)| alloc::format!("{}-{}", a, b)).collect();
        alloc::format!("mse\t{}\t{}\t{}\t{}", node, index, ok as u8, ranges.join(","))
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

impl Mse {
    pub fn new() -> Mse { Mse { duration: f64::NAN, ..Default::default() } }

    pub fn add(&mut self, kind: &str) {
        self.buffers.push(Buffer { kind: String::from(kind), bytes: Rc::new(RefCell::new(Bytes::default())), stream: Stream::new(), dropped: false });
    }

    pub fn append(&mut self, node: usize, index: usize, data: &[u8]) {
        let Some(b) = self.buffers.get_mut(index) else { return };
        let ok = !b.dropped && b.stream.append(data);
        if ok {
            let mut bytes = b.bytes.borrow_mut();
            if bytes.ended { bytes.ended = false; }
            bytes.data.extend_from_slice(data);
            bytes.largest = bytes.largest.max(b.stream.largest);
            self.appended += data.len() as u64;
        } else {
            sex_pdx::serial_record!("[web.mse.refused] node={} buffer={} bytes={}", node, index, data.len());
        }
        let r = b.reply(node, index, ok);
        self.replies.push(r);
    }

    pub fn remove(&mut self, node: usize, arg: &str) {
        let mut f = arg.split(' ');
        let (Some(i), Some(a), Some(z)) = (f.next().and_then(|v| v.parse::<usize>().ok()), f.next().and_then(|v| v.parse::<f64>().ok()), f.next().and_then(|v| v.parse::<f64>().ok())) else { return };
        let Some(b) = self.buffers.get_mut(i) else { return };
        // The bytes stay in the stream (the decoder reads them in order); only the
        // times the page sees change, as it asked.
        b.stream.remove(a, z);
        let r = b.reply(node, i, true);
        self.replies.push(r);
    }

    pub fn end(&mut self) {
        self.ended = true;
        for b in &self.buffers { b.bytes.borrow_mut().ended = true; }
    }
    pub fn reopen(&mut self) {
        self.ended = false;
        for b in &self.buffers { b.bytes.borrow_mut().ended = false; }
    }

    /// Whether every buffer has its init segment and enough bytes to open on.
    pub fn openable(&self) -> bool {
        let live: Vec<&Buffer> = self.buffers.iter().filter(|b| !b.dropped).collect();
        !live.is_empty() && live.iter().all(|b| {
            let bytes = b.bytes.borrow();
            b.stream.inited && (bytes.ended || bytes.data.len() as u64 >= OPEN_AHEAD.min(bytes.largest * 2 + 64 * 1024).max(64 * 1024))
        })
    }

    /// Decoders for the video and the sound buffers (one of them may be missing; a
    /// single buffer may carry both).
    pub fn decoders(&self) -> Result<(Option<Decoder>, Option<Decoder>), sex_media::Error> {
        let mut video = None;
        let mut audio = None;
        for b in self.buffers.iter().filter(|b| !b.dropped) {
            let dec = Decoder::open(Box::new(Feed(b.bytes.clone())))?;
            sex_pdx::serial_record!("[web.mse.decoder] kind={} format={} video={} audio={} w={} h={}", b.kind, dec.info.format, dec.info.video_codec, dec.info.audio_codec, dec.info.width, dec.info.height);
            if b.stream.has_video() && video.is_none() { video = Some(dec); } else if audio.is_none() { audio = Some(dec); }
        }
        Ok((video, audio))
    }
}
