//! Lossless video encoding in the caller's own confinement. This module has no
//! capture or file authority. Feed owned XRGB pixels, then drain ordered writes
//! through the caller's normal main loop before submitting another frame.
use alloc::boxed::Box;
use core::ffi::{c_int, c_void};
mod output;
pub use output::{Write, MAX_FILE_BYTES, TURN_BYTES};

pub const MAX_PIXELS: usize = 4 * 1024 * 1024;
pub const MAX_BUFFERED_OUTPUT: usize = 32 * 1024 * 1024;
pub const MAX_DURATION_MS: u64 = 600_000;
pub const MAX_FRAMES: u64 = 36_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    Invalid,
    PendingOutput,
    Closed,
    OutputLimit,
    Memory,
    Codec(i32),
}
impl From<output::Failure> for Error {
    fn from(e: output::Failure) -> Self {
        match e {
            output::Failure::Memory => Self::Memory,
            output::Failure::Limit => Self::OutputLimit,
            output::Failure::Invalid | output::Failure::Sequence => Self::Invalid,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}
impl Config {
    pub fn pixels(self) -> Option<usize> {
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 4320
            || !(1..=60).contains(&self.fps)
        {
            return None;
        }
        (self.width as usize)
            .checked_mul(self.height as usize)
            .filter(|&n| n <= MAX_PIXELS)
    }
}

pub enum Drain<'a> {
    Write(Write<'a>),
    NeedFrame,
    End,
}
struct Writer {
    output: output::Output,
    failure: Option<Error>,
}

/// A single-thread-owned encoder. Its C allocator must be provisioned by a
/// bounded program/recorder heap; do not link this into an allocator-free app.
/// The queue bound excludes FFmpeg's bounded-by-dimensions frames/codec state.
pub struct Encoder {
    raw: *mut c_void,
    writer: Box<Writer>,
    config: Config,
    last: Option<u64>,
    frames: u64,
    ended: bool,
    failed: Option<Error>,
}
impl Encoder {
    pub fn new(config: Config) -> Result<Self, Error> {
        config.pixels().ok_or(Error::Invalid)?;
        #[cfg(not(ffmpeg_recording))]
        {
            let _ = config;
            Err(Error::Unavailable)
        }
        #[cfg(ffmpeg_recording)]
        {
            crate::init_libc();
            let mut writer = Box::new(Writer {
                output: output::Output::new(MAX_BUFFERED_OUTPUT),
                failure: None,
            });
            let mut error = 0;
            let raw = unsafe {
                sx_record_open(
                    config.width as c_int,
                    config.height as c_int,
                    config.fps as c_int,
                    (&mut *writer as *mut Writer).cast(),
                    write_bytes,
                    seek_bytes,
                    &mut error,
                )
            };
            if raw.is_null() {
                return Err(writer.failure.unwrap_or(Error::Codec(error)));
            }
            Ok(Self {
                raw,
                writer,
                config,
                last: None,
                frames: 0,
                ended: false,
                failed: None,
            })
        }
    }
    fn ready(&self) -> Result<(), Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        if self.ended {
            return Err(Error::Closed);
        }
        if !self.writer.output.empty() {
            return Err(Error::PendingOutput);
        }
        Ok(())
    }
    fn result(&mut self, code: c_int) -> Result<(), Error> {
        if code >= 0 && self.writer.failure.is_none() {
            return Ok(());
        }
        let e = self.writer.failure.unwrap_or(Error::Codec(code));
        self.failed = Some(e);
        Err(e)
    }
    /// Pixels are copied into an owned C frame during this call. Never pass a
    /// borrowed shared capture bank slice: resolve its lease and copy atomically
    /// into private storage first. Timestamp gaps preserve dropped-frame time.
    pub fn frame(&mut self, pixels: &[u32], timestamp_ms: u64) -> Result<(), Error> {
        self.ready()?;
        if self.config.pixels() != Some(pixels.len())
            || timestamp_ms > MAX_DURATION_MS
            || self.last.is_some_and(|p| timestamp_ms <= p)
            || self.frames >= MAX_FRAMES
        {
            return Err(Error::Invalid);
        }
        #[cfg(ffmpeg_recording)]
        {
            let code = unsafe { sx_record_frame(self.raw, pixels.as_ptr(), timestamp_ms as i64) };
            self.result(code)?;
        }
        #[cfg(not(ffmpeg_recording))]
        {
            return Err(Error::Unavailable);
        }
        #[cfg(ffmpeg_recording)]
        {
            self.last = Some(timestamp_ms);
            self.frames += 1;
            Ok(())
        }
    }
    /// Finalize codec/container metadata. Drain every resulting ordered write,
    /// then durably sync in the file owner before reporting a saved recording.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.ready()?;
        if self.frames == 0 {
            return Err(Error::Invalid);
        }
        #[cfg(ffmpeg_recording)]
        {
            let code = unsafe { sx_record_finish(self.raw) };
            self.result(code)?;
            unsafe {
                sx_record_free(self.raw);
            }
            self.raw = core::ptr::null_mut();
            self.ended = true;
            Ok(())
        }
        #[cfg(not(ffmpeg_recording))]
        {
            Err(Error::Unavailable)
        }
    }
    pub fn output(&self) -> Result<Drain<'_>, Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        if let Some(write) = self.writer.output.pending() {
            return Ok(Drain::Write(write));
        }
        Ok(if self.ended {
            Drain::End
        } else {
            Drain::NeedFrame
        })
    }
    /// The caller authenticates its actual file reply and retains its file
    /// buffer until then. This local token only fences queue advancement.
    pub fn acknowledge(&mut self, token: u64, bytes: usize) -> Result<(), Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        self.writer.output.consume(token, bytes).map_err(Into::into)
    }
    pub fn frames(&self) -> u64 {
        self.frames
    }
    pub fn length(&self) -> u64 {
        self.writer.output.len()
    }
    pub fn buffered_bytes(&self) -> usize {
        self.writer.output.pending_bytes()
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        #[cfg(ffmpeg_recording)]
        unsafe {
            sx_record_free(self.raw);
        }
        // Aborting never writes a trailer or performs IPC from Drop.
    }
}

#[cfg(ffmpeg_recording)]
unsafe extern "C" {
    fn sx_record_open(
        width: c_int,
        height: c_int,
        fps: c_int,
        opaque: *mut c_void,
        write: unsafe extern "C" fn(*mut c_void, *const u8, c_int) -> c_int,
        seek: unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64,
        error: *mut c_int,
    ) -> *mut c_void;
    fn sx_record_frame(s: *mut c_void, pixels: *const u32, pts: i64) -> c_int;
    fn sx_record_finish(s: *mut c_void) -> c_int;
    fn sx_record_free(s: *mut c_void);
}
#[cfg(ffmpeg_recording)]
unsafe extern "C" fn write_bytes(opaque: *mut c_void, bytes: *const u8, size: c_int) -> c_int {
    if opaque.is_null() || size < 0 || (size != 0 && bytes.is_null()) {
        return -22;
    }
    let writer = unsafe { &mut *opaque.cast::<Writer>() };
    if writer.failure.is_some() {
        return -5;
    }
    let source = if size == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(bytes, size as usize) }
    };
    match writer.output.write(source) {
        Ok(()) => size,
        Err(e) => {
            writer.failure = Some(e.into());
            if e == output::Failure::Memory {
                -12
            } else {
                -28
            }
        }
    }
}
#[cfg(ffmpeg_recording)]
unsafe extern "C" fn seek_bytes(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    if opaque.is_null() {
        return -22;
    }
    let writer = unsafe { &mut *opaque.cast::<Writer>() };
    if writer.failure.is_some() {
        return -5;
    }
    match writer.output.seek(offset, whence) {
        Ok(at) => at as i64,
        Err(_) => -22,
    }
}
