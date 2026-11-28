// SPDX-License-Identifier: GPL-3.0-only
//! Live recording, for the browser's `MediaRecorder` (and any program that records
//! while it runs): pictures at the caller's own times and 48 kHz stereo sound in, a
//! streaming WebM (VP8 or VP9 with Opus) out in pieces, each one playable after the
//! ones before it. No index and no duration, as other browsers record. Built when
//! FFmpeg has the export profile (`FFMPEG_RECORDING=2 scripts/build_ffmpeg.sh`);
//! otherwise [`supported`] answers false and [`Live::new`] [`Error::Unavailable`].
//!
//! Keeping a page responsive is the caller's: encode at most one picture per pass of
//! its loop, and let a [`Pacer`] say when the next picture is due. The pacer spaces
//! pictures out when encoding falls behind, so encoding never takes more than about
//! half of the time.
pub use crate::recording::Error;
use alloc::boxed::Box;
use alloc::vec::Vec;
#[allow(unused_imports)]
use core::ffi::{c_int, c_void};

/// Sound is always recorded at this rate.
pub const RATE: u32 = 48_000;
/// Most bytes waiting for [`Live::take`]; more fails the recording.
pub const MAX_PENDING: usize = 32 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Video {
    Vp8 = 1,
    Vp9 = 2,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// None: sound only (`audio/webm`).
    pub video: Option<Video>,
    /// The recording's picture size (even, at least 16); pictures of other sizes are
    /// scaled to it. See [`size_for`].
    pub width: u32,
    pub height: u32,
    /// About how many pictures a second come (the encoder's rate control).
    pub fps: u32,
    pub video_bps: u32,
    pub audio: bool,
    pub audio_bps: u32,
}

impl Config {
    fn valid(&self) -> bool {
        if self.video.is_none() && !self.audio { return false; }
        if self.video.is_some() && (self.width < 16 || self.height < 16 || (self.width | self.height) & 1 != 0
            || self.width > 3840 || self.height > 2160 || !(1..=60).contains(&self.fps)
            || !(100_000..=20_000_000).contains(&self.video_bps)) { return false; }
        !self.audio || (6_000..=510_000).contains(&self.audio_bps)
    }
}

/// The recording size for pictures of `w` x `h`: no bigger than `max_w` x `max_h`,
/// same shape, even.
pub fn size_for(w: usize, h: usize, max_w: usize, max_h: usize) -> (u32, u32) {
    let (w, h) = if w == 0 || h == 0 { (640, 480) } else { crate::fit(w, h, max_w, max_h) };
    (((w as u32) & !1).max(16), ((h as u32) & !1).max(16))
}

/// Whether this build can record `video` (None: sound only) with or without sound.
pub fn supported(video: Option<Video>, audio: bool) -> bool {
    #[cfg(ffmpeg_export)]
    {
        crate::init_libc();
        unsafe { sx_live_supported(video.map_or(0, |v| v as c_int), audio as c_int) == 1 }
    }
    #[cfg(not(ffmpeg_export))]
    {
        let _ = (video, audio);
        false
    }
}

struct Sink {
    bytes: Vec<u8>,
    failed: Option<Error>,
}

#[cfg(ffmpeg_export)]
unsafe extern "C" fn write_bytes(opaque: *mut c_void, data: *const u8, len: c_int) -> c_int {
    let sink = unsafe { &mut *opaque.cast::<Sink>() };
    if sink.failed.is_some() || len < 0 { return -5; }
    let data = unsafe { core::slice::from_raw_parts(data, len as usize) };
    if sink.bytes.len() + data.len() > MAX_PENDING { sink.failed = Some(Error::OutputLimit); return -5; }
    if sink.bytes.try_reserve(data.len()).is_err() { sink.failed = Some(Error::Memory); return -12; }
    sink.bytes.extend_from_slice(data);
    len
}

/// A recording in progress. Single-thread owned.
pub struct Live {
    #[cfg_attr(not(ffmpeg_export), allow(dead_code))]
    raw: *mut c_void,
    sink: Box<Sink>,
    config: Config,
    last_ms: Option<u64>,
    /// Pictures and sound frames given so far.
    pub pictures: u64,
    pub samples: u64,
    /// Bytes handed out by [`Live::take`] so far.
    pub taken: u64,
    ended: bool,
    failed: Option<Error>,
}

impl Live {
    pub fn new(config: Config) -> Result<Live, Error> {
        if !config.valid() { return Err(Error::Invalid); }
        #[allow(unused_mut)]
        let mut sink = Box::new(Sink { bytes: Vec::new(), failed: None });
        #[cfg(not(ffmpeg_export))]
        {
            let _ = &mut sink;
            Err(Error::Unavailable)
        }
        #[cfg(ffmpeg_export)]
        {
            crate::init_libc();
            let mut error = 0;
            let raw = unsafe {
                sx_live_open(
                    config.video.map_or(0, |v| v as c_int),
                    config.width as c_int,
                    config.height as c_int,
                    config.fps as c_int,
                    config.video_bps as c_int,
                    if config.audio { RATE as c_int } else { 0 },
                    config.audio_bps as c_int,
                    (&mut *sink as *mut Sink).cast(),
                    write_bytes,
                    &mut error,
                )
            };
            if raw.is_null() {
                return Err(sink.failed.unwrap_or(if error == -38 { Error::Unavailable } else { Error::Codec(error) }));
            }
            Ok(Live { raw, sink, config, last_ms: None, pictures: 0, samples: 0, taken: 0, ended: false, failed: None })
        }
    }

    pub fn config(&self) -> Config { self.config }

    fn ready(&self) -> Result<(), Error> {
        if let Some(e) = self.failed { return Err(e); }
        if self.ended { return Err(Error::Closed); }
        Ok(())
    }

    #[allow(dead_code)]
    fn result(&mut self, code: c_int) -> Result<(), Error> {
        if code >= 0 && self.sink.failed.is_none() { return Ok(()); }
        let e = self.sink.failed.unwrap_or(Error::Codec(code));
        self.failed = Some(e);
        Err(e)
    }

    /// A picture of `w` x `h` (0x00RRGGBB; the top byte is ignored) shown `at_ms`
    /// after the recording began. Times must increase.
    pub fn picture(&mut self, pixels: &[u32], w: usize, h: usize, at_ms: u64) -> Result<(), Error> {
        self.ready()?;
        if self.config.video.is_none() || w == 0 || h == 0 || w > 8192 || h > 8192 || pixels.len() < w * h
            || self.last_ms.is_some_and(|l| at_ms <= l) || at_ms > i64::MAX as u64 {
            return Err(Error::Invalid);
        }
        #[cfg(ffmpeg_export)]
        {
            let code = unsafe { sx_live_picture(self.raw, pixels.as_ptr(), w as c_int, h as c_int, at_ms as i64) };
            self.result(code)?;
        }
        self.last_ms = Some(at_ms);
        self.pictures += 1;
        #[cfg(not(ffmpeg_export))]
        return Err(Error::Unavailable);
        #[cfg(ffmpeg_export)]
        Ok(())
    }

    /// The time of the last picture given.
    pub fn last_picture_ms(&self) -> Option<u64> { self.last_ms }

    /// The next stereo samples at [`RATE`].
    pub fn sound(&mut self, pcm: &[[i16; 2]]) -> Result<(), Error> {
        self.ready()?;
        if !self.config.audio { return Err(Error::Invalid); }
        if pcm.is_empty() { return Ok(()); }
        #[cfg(ffmpeg_export)]
        {
            for part in pcm.chunks(1 << 16) {
                let code = unsafe { sx_live_sound(self.raw, part.as_ptr().cast(), part.len() as c_int) };
                self.result(code)?;
            }
            self.samples += pcm.len() as u64;
            Ok(())
        }
        #[cfg(not(ffmpeg_export))]
        Err(Error::Unavailable)
    }

    /// Silence until the sound reaches `at_ms` from the start (a muted or quiet
    /// microphone still makes a track that lasts as long as the pictures).
    pub fn sound_until(&mut self, at_ms: u64) -> Result<(), Error> {
        let due = at_ms.saturating_mul(RATE as u64) / 1000;
        let zero = [[0i16; 2]; 960];
        while self.samples < due {
            let n = ((due - self.samples) as usize).min(zero.len());
            self.sound(&zero[..n])?;
        }
        Ok(())
    }

    /// Ends the piece being written: what has been encoded so far becomes takeable.
    pub fn flush(&mut self) -> Result<(), Error> {
        self.ready()?;
        #[cfg(ffmpeg_export)]
        {
            let code = unsafe { sx_live_flush(self.raw) };
            self.result(code)
        }
        #[cfg(not(ffmpeg_export))]
        Err(Error::Unavailable)
    }

    /// Ends the recording; [`Live::take`] then has the rest.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.ready()?;
        #[cfg(ffmpeg_export)]
        {
            let code = unsafe { sx_live_finish(self.raw) };
            self.result(code)?;
            unsafe { sx_live_free(self.raw) };
            self.raw = core::ptr::null_mut();
            self.ended = true;
            Ok(())
        }
        #[cfg(not(ffmpeg_export))]
        Err(Error::Unavailable)
    }

    /// The bytes written since the last take (the next piece of the file).
    pub fn take(&mut self) -> Vec<u8> {
        let out = core::mem::take(&mut self.sink.bytes);
        self.taken += out.len() as u64;
        out
    }

    pub fn pending(&self) -> usize { self.sink.bytes.len() }
    pub fn ended(&self) -> bool { self.ended }
    pub fn error(&self) -> Option<Error> { self.failed }
}

impl Drop for Live {
    fn drop(&mut self) {
        #[cfg(ffmpeg_export)]
        unsafe { sx_live_free(self.raw) };
    }
}

/// When the next picture is due, so encoding keeps up in real time. Pictures come
/// at most `fps` a second; when encoding one takes longer than half the time between
/// pictures, pictures are spaced out (down to [`Pacer::SLOWEST_MS`]), so the program
/// that records always has time for its own work.
#[derive(Clone, Copy, Debug)]
pub struct Pacer {
    base_ms: u32,
    /// Encoding time per picture, averaged (in sixteenths of a millisecond).
    cost16: u32,
    next_ms: u64,
    /// Pictures left out to keep up.
    pub skipped: u64,
}

impl Pacer {
    pub const SLOWEST_MS: u32 = 500;
    pub fn new(fps: u32) -> Pacer { Pacer { base_ms: 1000 / fps.clamp(1, 60), cost16: 0, next_ms: 0, skipped: 0 } }

    /// The time between pictures now.
    pub fn interval_ms(&self) -> u32 { self.base_ms.max(self.cost16 * 2 / 16).min(Self::SLOWEST_MS) }

    /// Whether a picture at `at_ms` should be encoded.
    pub fn due(&mut self, at_ms: u64) -> bool {
        if at_ms + 2 >= self.next_ms { return true; }
        self.skipped += 1;
        false
    }

    /// A picture at `at_ms` took `ms` to encode.
    pub fn spent(&mut self, at_ms: u64, ms: u64) {
        let c = (ms.min(10_000) as u32) * 16;
        self.cost16 = if self.cost16 == 0 { c } else { (self.cost16 * 3 + c) / 4 };
        self.next_ms = at_ms + self.interval_ms() as u64;
    }
}

#[cfg(ffmpeg_export)]
unsafe extern "C" {
    fn sx_live_supported(video: c_int, audio: c_int) -> c_int;
    fn sx_live_open(
        video: c_int,
        width: c_int,
        height: c_int,
        fps: c_int,
        vbps: c_int,
        rate: c_int,
        abps: c_int,
        opaque: *mut c_void,
        write: unsafe extern "C" fn(*mut c_void, *const u8, c_int) -> c_int,
        error: *mut c_int,
    ) -> *mut c_void;
    fn sx_live_picture(s: *mut c_void, pixels: *const u32, w: c_int, h: c_int, at_ms: i64) -> c_int;
    fn sx_live_sound(s: *mut c_void, pcm: *const i16, frames: c_int) -> c_int;
    fn sx_live_flush(s: *mut c_void) -> c_int;
    fn sx_live_finish(s: *mut c_void) -> c_int;
    fn sx_live_free(s: *mut c_void);
}
