// SPDX-License-Identifier: GPL-3.0-only
//! Video and sound for SexOS programs. FFmpeg (libavformat, libavcodec, libswscale,
//! libswresample) runs inside the calling program's own confinement, linked against
//! sex-libc: the C code reads the file only through a [`Source`], so it holds exactly
//! what the program holds. A hostile file can only crash that program.
//!
//! - [`Decoder`] opens a stream and gives its pictures (scaled to any size, as
//!   0x00RRGGBB) and its sound (48 kHz stereo i16) in order.
//! - [`Player`] plays one: sound goes to the program's sound ring, and the sound
//!   card's clock paces the pictures (ticks pace them when there is no sound). It never
//!   waits: call [`Player::step`] often, from a program's own loop or a browser's.
//!
//! Formats: MP4/MOV, Matroska/WebM, Ogg, MP3, AAC, FLAC and WAV, with H.264, HEVC,
//! VP8, VP9 and MPEG-4 video and AAC, MP3, Opus, Vorbis, FLAC and PCM sound. A build
//! without FFmpeg (see build.rs) refuses every file as unsupported.
#![no_std]

extern crate alloc;
extern crate sex_libc;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
#[allow(unused_imports)]
use core::ffi::{c_char, c_int, c_void};
use sex_app::sound::{Sound, RATE};

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

/// FFmpeg's error number, with its text through [`Error::text`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub i32);

impl Error {
    /// No FFmpeg in this build.
    pub const UNSUPPORTED: Error = Error(-0x5355_5050);
    pub const NO_MEMORY: Error = Error(-12);
    /// FFmpeg's words for it, such as "Invalid data found when processing input".
    pub fn text(self, buf: &mut [u8]) -> &str {
        if self == Error::UNSUPPORTED {
            let t = b"this build has no video or sound decoder";
            let n = t.len().min(buf.len());
            buf[..n].copy_from_slice(&t[..n]);
            return core::str::from_utf8(&buf[..n]).unwrap_or("");
        }
        if buf.is_empty() { return ""; }
        ffi::error(self.0, buf);
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        core::str::from_utf8(&buf[..n]).unwrap_or("?")
    }
}

/// What a stream holds.
#[derive(Clone, Debug, Default)]
pub struct Info {
    /// 0 when there are no pictures.
    pub width: u32,
    pub height: u32,
    /// Pictures per second, as a fraction; 0/0 when unknown.
    pub fps: (u32, u32),
    /// 0 when unknown.
    pub duration_ms: u64,
    pub video_codec: Name,
    /// 0 when there is no sound.
    pub sample_rate: u32,
    pub channels: u32,
    pub audio_codec: Name,
    pub format: Name,
}

/// A short name from FFmpeg, such as "h264" or "matroska,webm" (at most 15 bytes).
#[derive(Clone, Copy, Default)]
pub struct Name([u8; 16]);

impl Name {
    pub fn as_str(&self) -> &str {
        let n = self.0.iter().position(|&c| c == 0).unwrap_or(16);
        core::str::from_utf8(&self.0[..n]).unwrap_or("?")
    }
}

impl core::fmt::Debug for Name {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result { f.write_str(self.as_str()) }
}
impl core::fmt::Display for Name {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result { f.write_str(self.as_str()) }
}

/// What [`Decoder::next`] gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// A picture is in `out`; its time from the stream's start, or -1.
    Picture { at_ms: i64 },
    /// `frames` frames of sound are in `pcm`.
    Sound { frames: usize, at_ms: i64 },
    End,
}

#[cfg(ffmpeg)]
mod ffi {
    use core::ffi::{c_char, c_int, c_void};
    #[repr(C)]
    pub struct RawInfo {
        pub width: c_int,
        pub height: c_int,
        pub fps_num: c_int,
        pub fps_den: c_int,
        pub duration_ms: i64,
        pub codec: [c_char; 16],
        pub sample_rate: c_int,
        pub channels: c_int,
        pub audio_codec: [c_char; 16],
        pub format: [c_char; 16],
    }
    pub type ReadFn = unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int;
    pub type SeekFn = unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64;
    extern "C" {
        pub fn sx_open(opaque: *mut c_void, read: ReadFn, seek: SeekFn, info: *mut RawInfo, err: *mut c_int) -> *mut c_void;
        pub fn sx_next(s: *mut c_void, out: *mut u32, w: c_int, h: c_int, pcm: *mut i16, cap: c_int,
                       frames: *mut c_int, at_ms: *mut i64) -> c_int;
        pub fn sx_seek(s: *mut c_void, ms: i64) -> c_int;
        pub fn sx_close(s: *mut c_void);
        fn sx_error(err: c_int, buf: *mut c_char, len: c_int);
    }
    pub fn error(err: i32, buf: &mut [u8]) {
        unsafe { sx_error(err, buf.as_mut_ptr().cast(), buf.len() as c_int) };
    }
}

#[cfg(not(ffmpeg))]
mod ffi {
    pub fn error(_: i32, buf: &mut [u8]) { buf[0] = 0; }
}

/// The source with FFmpeg's read position, at a fixed address for the C callbacks.
#[cfg_attr(not(ffmpeg), allow(dead_code))]
struct Io {
    src: Box<dyn Source>,
    at: u64,
}

#[cfg(ffmpeg)]
const AVERROR_EOF: c_int = -0x2046_4f45; // FFERRTAG('E','O','F',' ')
#[cfg(ffmpeg)]
const AVERROR_EIO: c_int = -5;
#[cfg(ffmpeg)]
const AVSEEK_SIZE: c_int = 0x10000;

#[cfg(ffmpeg)]
unsafe extern "C" fn io_read(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    let io = unsafe { &mut *opaque.cast::<Io>() };
    if io.src.len().is_some_and(|len| io.at >= len) || size <= 0 {
        return AVERROR_EOF;
    }
    let out = unsafe { core::slice::from_raw_parts_mut(buf, size as usize) };
    match io.src.read_at(io.at, out) {
        Some(0) => AVERROR_EOF,
        Some(n) => { io.at += n as u64; n as c_int }
        None => AVERROR_EIO,
    }
}

#[cfg(ffmpeg)]
unsafe extern "C" fn io_seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let io = unsafe { &mut *opaque.cast::<Io>() };
    let len = io.src.len().map(|l| l as i64);
    if whence & AVSEEK_SIZE != 0 {
        return len.unwrap_or(-1);
    }
    let base = match (whence & 3, len) { (0, _) => 0, (1, _) => io.at as i64, (2, Some(l)) => l, _ => return -1 };
    let to = base + offset;
    if to < 0 || len.is_some_and(|l| to > l) {
        return -1;
    }
    io.at = to as u64;
    to
}

/// An open stream.
pub struct Decoder {
    #[allow(dead_code)]
    s: *mut c_void,
    io: Box<Io>,
    pub info: Info,
}

#[cfg_attr(not(ffmpeg), allow(dead_code))]
fn name(raw: &[c_char; 16]) -> Name {
    let mut n = [0u8; 16];
    for (d, s) in n.iter_mut().zip(raw.iter()).take(15) { *d = *s as u8; }
    Name(n)
}

impl Decoder {
    /// Open a stream: its container and codecs must be among those built in.
    /// A growing source should have [`AHEAD`] bytes (or all of them) first.
    pub fn open(src: Box<dyn Source>) -> Result<Decoder, Error> {
        Self::open_io(Box::new(Io { src, at: 0 }))
    }

    /// Whether decoding can go on without reading past what has arrived.
    pub fn ready(&self) -> bool {
        let at = self.io.at;
        let have = self.io.src.available(at);
        have >= AHEAD || self.io.src.len().is_some_and(|len| have >= len.saturating_sub(at))
    }

    #[cfg(not(ffmpeg))]
    fn open_io(_: Box<Io>) -> Result<Decoder, Error> { Err(Error::UNSUPPORTED) }

    #[cfg(ffmpeg)]
    fn open_io(mut io: Box<Io>) -> Result<Decoder, Error> {
        init_libc();
        let mut raw: ffi::RawInfo = unsafe { core::mem::zeroed() };
        let mut err = 0;
        let opaque: *mut Io = &mut *io;
        let s = unsafe { ffi::sx_open(opaque.cast(), io_read, io_seek, &mut raw, &mut err) };
        if s.is_null() {
            return Err(Error(err));
        }
        let pos = |v: c_int| v.max(0) as u32;
        let info = Info {
            width: pos(raw.width),
            height: pos(raw.height),
            fps: if raw.fps_num > 0 && raw.fps_den > 0 { (raw.fps_num as u32, raw.fps_den as u32) } else { (0, 0) },
            duration_ms: raw.duration_ms.max(0) as u64,
            video_codec: name(&raw.codec),
            sample_rate: pos(raw.sample_rate),
            channels: pos(raw.channels),
            audio_codec: name(&raw.audio_codec),
            format: name(&raw.format),
        };
        Ok(Decoder { s, io, info })
    }

    /// The next picture or piece of sound. A picture is scaled to `w` x `h` into `out`
    /// (at least `w * h` long); sound goes into `pcm`.
    #[cfg(ffmpeg)]
    pub fn next(&mut self, out: &mut [u32], w: usize, h: usize, pcm: &mut [[i16; 2]]) -> Result<Next, Error> {
        assert!(out.len() >= w * h);
        let (mut frames, mut at): (c_int, i64) = (0, -1);
        let r = unsafe {
            ffi::sx_next(self.s, out.as_mut_ptr(), w as c_int, h as c_int, pcm.as_mut_ptr().cast(),
                         pcm.len() as c_int, &mut frames, &mut at)
        };
        match r {
            0 => Ok(Next::End),
            1 => Ok(Next::Picture { at_ms: at }),
            2 => Ok(Next::Sound { frames: frames.max(0) as usize, at_ms: at }),
            e => Err(Error(e)),
        }
    }
    #[cfg(not(ffmpeg))]
    pub fn next(&mut self, _: &mut [u32], _: usize, _: usize, _: &mut [[i16; 2]]) -> Result<Next, Error> {
        Err(Error::UNSUPPORTED)
    }

    /// Go to `ms` from the start: decoding restarts at the key frame at or before it.
    #[cfg(ffmpeg)]
    pub fn seek(&mut self, ms: u64) -> Result<(), Error> {
        let r = unsafe { ffi::sx_seek(self.s, ms as i64) };
        if r < 0 { Err(Error(r)) } else { Ok(()) }
    }
    #[cfg(not(ffmpeg))]
    pub fn seek(&mut self, _: u64) -> Result<(), Error> { Err(Error::UNSUPPORTED) }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        #[cfg(ffmpeg)]
        unsafe { ffi::sx_close(self.s) };
    }
}

/// sex-libc's log, exit and clock, unless the program set its own.
#[cfg(ffmpeg)]
fn init_libc() {
    static mut DONE: bool = false;
    unsafe {
        if DONE { return; }
        DONE = true;
    }
    fn log(bytes: &[u8]) {
        sex_pdx::serial_record!("[media.ffmpeg] {}", core::str::from_utf8(bytes).unwrap_or("?").trim_end());
    }
    fn exit(code: c_int) -> ! { sex_pdx::task_finish(code as u64) }
    fn clock() -> (i64, u64) { (0, sex_pdx::get_ticks() * 1_000_000) }
    sex_libc::set_log_default(log);
    sex_libc::set_exit_default(exit);
    sex_libc::time::set_clock_default(clock);
}

/// The largest size with the stream's aspect ratio that fits in `max_w` x `max_h`.
pub fn fit(w: usize, h: usize, max_w: usize, max_h: usize) -> (usize, usize) {
    if w == 0 || h == 0 {
        return (0, 0);
    }
    if w <= max_w && h <= max_h {
        return (w, h);
    }
    if w * max_h >= h * max_w { (max_w, (h * max_w / w).max(1)) } else { ((w * max_h / h).max(1), max_h) }
}

/// What sets the pace: the sound card's clock while sound plays, ticks otherwise.
/// The card reports in whole periods (about 43 ms), so between reports the clock runs
/// on ticks from the last one, never more than a period ahead of it.
struct Clock {
    /// Ticks when PLAYED last moved, and what it read then.
    moved: u64,
    last_played: u64,
    /// The sound clock reads `base_ms` when the ring's PLAYED reaches `played_base`.
    started: bool,
    base_ms: i64,
    played_base: u64,
    /// The last time this clock gave, and the tick it gave it at.
    last_ms: i64,
    last_tick: u64,
    paused: bool,
}

/// Why sound stopped, when it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Silence {
    /// Sound was queued and nothing played for 750 ms: no card, or a stuck one.
    Stalled { played: u64, pending: u64 },
}

impl Clock {
    fn new() -> Clock {
        let t = sex_pdx::get_ticks();
        Clock { moved: t, last_played: 0, started: false, base_ms: 0, played_base: 0, last_ms: 0, last_tick: t, paused: false }
    }

    /// Media time now, in ms. It never goes backwards, except by `set`.
    fn now(&mut self, sound: &mut Option<Sound>) -> (i64, Option<Silence>) {
        let ticks = sex_pdx::get_ticks();
        if self.paused {
            self.last_tick = ticks;
            self.moved = ticks;
            return (self.last_ms, None);
        }
        let mut at = self.last_ms + (ticks - self.last_tick) as i64;
        let mut silence = None;
        if let (Some(s), true) = (sound.as_ref(), self.started) {
            let played = s.played_total();
            if played != self.last_played {
                self.last_played = played;
                self.moved = ticks;
            }
            let pending = s.written_total().saturating_sub(played);
            if pending > 0 && ticks.saturating_sub(self.moved) > 750 {
                silence = Some(Silence::Stalled { played, pending });
            } else if pending > 0 {
                let heard = self.base_ms + (played.saturating_sub(self.played_base) * 1000 / RATE as u64) as i64;
                at = heard + ((ticks - self.moved) as i64).min(43);
            }
            // With nothing queued (before the first sound, or a gap), ticks carry on.
        }
        if silence.is_some() {
            *sound = None;
        }
        let at = at.max(self.last_ms);
        self.last_ms = at;
        self.last_tick = ticks;
        (at, silence)
    }

    /// Jump to `ms` (a seek); the sound clock starts again with the next sound.
    fn set(&mut self, ms: i64) {
        self.last_ms = ms;
        self.last_tick = sex_pdx::get_ticks();
        self.started = false;
    }
}

/// A decoded picture waiting for its time.
struct Pic {
    due: i64,
    px: Vec<u32>,
}

/// What [`Player::step`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// A new picture is due: [`Player::picture`] has it. `n` counts pictures shown.
    Picture { n: u32, at_ms: i64 },
    /// Nothing to show yet; there is no point in calling again for about `ms`.
    Wait { ms: u64 },
    /// Paused: nothing happens until [`Player::play`].
    Paused,
    /// The stream has played to its end.
    Ended,
}

const PCM_CAP: usize = 16384;
/// Pictures decoded ahead: up to about two seconds, within a memory budget, so sound
/// keeps flowing while the program is busy with other work.
const QUEUE_BYTES: usize = 32 * 1024 * 1024;
const QUEUE_MAX: usize = 48;
/// Silence that goes before sound starting later than the pictures; at most this long.
const PAD_MAX_MS: i64 = 30_000;
static SILENCE: [[i16; 2]; 1024] = [[0; 2]; 1024];
/// How long one step may decode before it gives the caller its loop back.
const STEP_BUDGET_MS: u64 = 12;
/// With less than this much sound queued (a quarter second), a step may decode for
/// up to LOW_BUDGET_MS.
const LOW_FRAMES: u64 = RATE as u64 / 4;
const LOW_BUDGET_MS: u64 = 40;

/// Plays one stream. Sound goes to this program's sound ring; pictures are decoded
/// ahead into a short queue and each becomes current at its time.
pub struct Player {
    dec: Decoder,
    w: usize,
    h: usize,
    sound: Option<Sound>,
    clock: Clock,
    queue: VecDeque<Pic>,
    free: Vec<Vec<u32>>,
    cap: usize,
    current: Vec<u32>,
    pcm: Vec<[i16; 2]>,
    /// Decoded sound not yet in the ring: pcm[left.0..left.1].
    left: (usize, usize),
    pad: u64,
    decoded: u64,
    eof: bool,
    ended: bool,
    /// After a seek: drop what comes before this.
    skip_until: i64,
    error: Option<Error>,
    silence: Option<Silence>,
    // What was shown and heard, for logs and tests.
    pub shown: u32,
    pub heard: u64,
    pub late_ms: i64,
    volume: u8,
    muted: bool,
    /// Sound is held back, and the clock with it, until this much is queued, so a
    /// slow start (FFmpeg warming up, a busy page) doesn't open with a gap.
    priming: bool,
    /// The ring's WRITTEN count when priming began: sound before it was skipped.
    prime_from: u64,
    /// Whether this player plays sound: it takes the ring when it first plays, and
    /// can give it up (`release_sound`) for another player in the same program.
    wants_sound: bool,
}

/// Sound queued before it starts: half a second.
const PRIME_FRAMES: u64 = RATE as u64 / 2;

impl Player {
    /// Play `dec` with pictures scaled to `w` x `h` (see [`fit`]). With `sound` false
    /// the stream plays silent, paced by ticks. It starts paused if `paused`.
    pub fn new(dec: Decoder, w: usize, h: usize, sound: bool, paused: bool) -> Player {
        let pictures = dec.info.width > 0 && dec.info.height > 0 && w > 0 && h > 0;
        let (w, h) = if pictures { (w, h) } else { (0, 0) };
        let cap = (QUEUE_BYTES / (w * h * 4).max(1)).clamp(2, QUEUE_MAX);
        let wants_sound = sound && dec.info.sample_rate > 0;
        let mut p = Player {
            dec, w, h, sound: None, clock: Clock::new(), queue: VecDeque::new(), free: Vec::new(), cap,
            current: Vec::new(), pcm: Vec::new(), left: (0, 0), pad: 0, decoded: 0, eof: false, ended: false,
            skip_until: i64::MIN, error: None, silence: None, shown: 0, heard: 0, late_ms: 0, volume: 100, muted: false, priming: false, prime_from: 0, wants_sound,
        };
        if paused {
            p.clock.paused = true;
        } else {
            p.claim_sound();
        }
        p
    }

    pub fn info(&self) -> &Info { &self.dec.info }
    /// The picture size, (0, 0) for sound only.
    pub fn size(&self) -> (usize, usize) { (self.w, self.h) }
    /// The picture shown now, `w * h` pixels, empty before the first.
    pub fn picture(&self) -> &[u32] { &self.current }
    /// Whether this player holds the program's sound ring: false before it first
    /// plays, after `release_sound`, with no ring, or after the sound stalled.
    pub fn has_sound(&self) -> bool { self.sound.is_some() }
    pub fn silence(&self) -> Option<Silence> { self.silence }
    /// FFmpeg's error, if decoding stopped on one.
    pub fn error(&self) -> Option<Error> { self.error }
    pub fn paused(&self) -> bool { self.clock.paused }
    pub fn ended(&self) -> bool { self.ended }
    /// Frames of sound the card has played since this player opened its ring.
    pub fn played(&self) -> u64 { self.sound.as_ref().map_or(0, |s| s.played()) }
    pub fn duration_ms(&self) -> u64 { self.dec.info.duration_ms }

    /// Media time now, in ms.
    pub fn time_ms(&mut self) -> i64 {
        let (at, silence) = self.clock.now(&mut self.sound);
        if silence.is_some() { self.silence = silence; }
        at
    }

    pub fn pause(&mut self) {
        if self.clock.paused { return; }
        let _ = self.time_ms();
        self.clock.paused = true;
        if let Some(s) = self.sound.as_mut() { s.pause(true); }
    }

    pub fn play(&mut self) {
        let claimed = self.claim_sound();
        if self.ended {
            // Playing an ended stream starts it again, as a browser does.
            self.seek(0);
        } else if claimed && self.decoded > 0 {
            // Sound decoded while the ring was elsewhere was dropped: decode again
            // from here so sound and pictures line up.
            let at = self.clock.last_ms.max(0) as u64;
            self.seek(at);
        }
        if !self.clock.paused { return; }
        self.clock.paused = false;
        let t = sex_pdx::get_ticks();
        self.clock.last_tick = t;
        self.clock.moved = t;
        if self.priming { return; }
        if let Some(s) = self.sound.as_mut() { s.pause(false); }
    }

    /// Takes this program's sound ring if this player wants sound and has none.
    /// True when it did. A program has one ring: before this, every other player
    /// in the program must have called `release_sound`.
    fn claim_sound(&mut self) -> bool {
        if !self.wants_sound || self.sound.is_some() { return false; }
        self.sound = Sound::open();
        if self.sound.is_none() { self.wants_sound = false; return false; }
        self.apply_volume();
        self.clock.started = false;
        self.prime();
        true
    }

    /// Gives up the sound ring without touching it, so another player in this
    /// program can take it. Sound decoded meanwhile is dropped; `play` takes the
    /// ring back. Pause the player too, or it plays on silent, paced by ticks.
    pub fn release_sound(&mut self) {
        if let Some(s) = self.sound.take() {
            // The ring is the next owner's now: closing or pausing it would be theirs.
            core::mem::forget(s);
        }
        self.priming = false;
        self.left = (0, 0);
        self.pad = 0;
    }

    /// Hold the sound ring until PRIME_FRAMES are queued (see `step`).
    fn prime(&mut self) {
        if let Some(s) = self.sound.as_mut() {
            s.pause(true);
            self.prime_from = s.written_total();
            self.priming = true;
        }
    }

    /// Whether priming is still under way; ends it once enough sound is queued.
    fn priming(&mut self) -> bool {
        if !self.priming { return false; }
        let lead = self.sound.as_ref().map_or(u64::MAX, |s| s.written_total().saturating_sub(s.played_total().max(self.prime_from)));
        // A ring with no room ends it too: Bell applies a seek's skip only while
        // the ring plays, so a held ring can stay full of the skipped sound.
        let blocked = self.left.0 < self.left.1 || self.pad > 0;
        let full = self.queue.len() >= self.cap;
        if lead < PRIME_FRAMES && !self.eof && !full && !blocked && self.error.is_none() {
            // Time stands still meanwhile, as when paused.
            let t = sex_pdx::get_ticks();
            self.clock.last_tick = t;
            self.clock.moved = t;
            return true;
        }
        self.priming = false;
        let t = sex_pdx::get_ticks();
        self.clock.last_tick = t;
        self.clock.moved = t;
        if !self.clock.paused {
            if let Some(s) = self.sound.as_mut() { s.pause(false); }
        }
        false
    }

    /// 0..=100.
    pub fn set_volume(&mut self, percent: u8) {
        self.volume = percent.min(100);
        self.apply_volume();
    }
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.apply_volume();
    }
    fn apply_volume(&mut self) {
        let v = if self.muted { 0 } else { self.volume };
        if let Some(s) = self.sound.as_mut() { s.set_volume(v); }
    }

    /// Go to `ms` from the start. The picture at that time shows once it is decoded.
    pub fn seek(&mut self, ms: u64) {
        let ms = if self.dec.info.duration_ms > 0 { ms.min(self.dec.info.duration_ms) } else { ms };
        if let Err(e) = self.dec.seek(ms) {
            sex_pdx::serial_record!("[media.seek.failed] ms={} err={}", ms, e.0);
            return;
        }
        while let Some(p) = self.queue.pop_front() { self.free.push(p.px); }
        self.left = (0, 0);
        self.pad = 0;
        self.eof = false;
        self.ended = false;
        self.skip_until = ms as i64;
        // A stream that reached its end closed its ring: open it again first, or Bell
        // drops the stream the moment the skip empties it.
        if let Some(s) = self.sound.as_mut() { s.reopen(); s.skip(); }
        self.clock.set(ms as i64);
        self.prime();
    }

    /// Do the work there is now without waiting: decode ahead, keep the sound ring
    /// fed, and make the next picture current when its time has come.
    pub fn step(&mut self) -> Step {
        if self.ended {
            return Step::Ended;
        }
        if self.clock.paused {
            return Step::Paused;
        }
        let begun = sex_pdx::get_ticks();
        loop {
            self.feed();
            self.priming();
            let now = self.time_ms();
            // The oldest picture whose time has come becomes current. A caller that
            // only draws now and then calls again until it gets no picture, then
            // draws the last.
            if self.queue.front().is_some_and(|p| p.due <= now) {
                let p = self.queue.pop_front().unwrap();
                self.late_ms = self.late_ms.max(now - p.due);
                let old = core::mem::replace(&mut self.current, p.px);
                if !old.is_empty() { self.free.push(old); }
                self.shown += 1;
                return Step::Picture { n: self.shown - 1, at_ms: p.due };
            }
            let sound_waits = self.left.0 < self.left.1 || self.pad > 0;
            let full = self.queue.len() >= self.cap;
            if self.eof && self.queue.is_empty() && !sound_waits {
                // Let the last sound play out before calling it ended.
                let pending = self.sound.as_ref().map_or(0, |s| s.written_total().saturating_sub(s.played_total()));
                if pending == 0 {
                    self.ended = true;
                    return Step::Ended;
                }
                return Step::Wait { ms: 10 };
            }
            let starved = !self.dec.ready();
            // Sound running low gets a longer turn: a gap is worse than a late picture.
            let low = self.sound.as_ref().is_some_and(|s| s.written_total().saturating_sub(s.played_total()) < LOW_FRAMES);
            let budget = if low && !self.priming { LOW_BUDGET_MS } else { STEP_BUDGET_MS };
            if self.eof || full || sound_waits || starved || sex_pdx::get_ticks() - begun >= budget {
                let until = self.queue.front().map_or(10, |p| (p.due - now).clamp(1, 16) as u64);
                return Step::Wait { ms: if sound_waits || starved { until.min(5) } else { until } };
            }
            self.decode();
        }
    }

    /// Put waiting sound into the ring; false if some still waits for room.
    fn feed(&mut self) -> bool {
        while self.pad > 0 {
            let Some(s) = self.sound.as_mut() else { self.pad = 0; break };
            let n = s.write(&SILENCE[..self.pad.min(SILENCE.len() as u64) as usize]);
            self.pad -= n as u64;
            if n == 0 { return false; }
        }
        if self.left.0 < self.left.1 {
            let Some(s) = self.sound.as_mut() else { self.left = (0, 0); return true };
            let n = s.write(&self.pcm[self.left.0..self.left.1]);
            self.left.0 += n;
            self.heard += n as u64;
            if self.left.0 < self.left.1 { return false; }
        }
        true
    }

    /// Decode one thing. False when it gave nothing to queue (sound, or a dropped
    /// picture), so the caller may go on.
    fn decode(&mut self) -> bool {
        let (w, h) = (self.w, self.h);
        let mut px = match self.free.pop() {
            Some(px) => px,
            None => {
                let mut px = Vec::new();
                if px.try_reserve_exact(w * h).is_err() {
                    self.fail(Error::NO_MEMORY);
                    return true;
                }
                px.resize(w * h, 0u32);
                px
            }
        };
        if self.pcm.is_empty() {
            self.pcm = alloc::vec![[0; 2]; PCM_CAP];
        }
        match self.dec.next(&mut px, w, h, &mut self.pcm) {
            Err(e) => {
                self.free.push(px);
                self.fail(e);
                true
            }
            Ok(Next::End) => {
                self.free.push(px);
                self.eof = true;
                if let Some(s) = self.sound.as_mut() { s.close(); }
                true
            }
            Ok(Next::Picture { at_ms }) => {
                let (num, den) = match self.dec.info.fps { (0, _) | (_, 0) => (25, 1), f => f };
                let due = if at_ms >= 0 { at_ms } else { (self.decoded * 1000 * den as u64 / num as u64) as i64 };
                self.decoded += 1;
                if due < self.skip_until {
                    self.free.push(px);
                    return false;
                }
                self.queue.push_back(Pic { due, px });
                true
            }
            Ok(Next::Sound { frames, at_ms }) => {
                self.free.push(px);
                if self.sound.is_none() || (at_ms >= 0 && at_ms < self.skip_until) {
                    return false;
                }
                if !self.clock.started {
                    // The sound clock reads this stream time once its first frame plays.
                    // Sound that starts later than now waits behind silence, keeping its
                    // place.
                    let (now, _) = self.clock.now(&mut self.sound);
                    let at = at_ms.max(0);
                    let s = self.sound.as_ref().unwrap();
                    self.clock.started = true;
                    self.clock.base_ms = at.min(now);
                    self.clock.played_base = s.written_total();
                    self.clock.moved = sex_pdx::get_ticks();
                    self.pad = ((at - at.min(now)).min(PAD_MAX_MS) as u64) * RATE as u64 / 1000;
                }
                self.left = (0, frames.min(self.pcm.len()));
                false
            }
        }
    }

    fn fail(&mut self, e: Error) {
        sex_pdx::serial_record!("[media.decode.failed] err={}", e.0);
        self.error = Some(e);
        self.eof = true;
        if let Some(s) = self.sound.as_mut() { s.close(); }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(s) = self.sound.as_mut() {
            s.skip();
            s.close();
        }
    }
}
