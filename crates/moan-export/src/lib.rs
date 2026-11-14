// SPDX-License-Identifier: GPL-3.0-only
//! Audio export in the caller's organ: 48 kHz stereo PCM -> real FFmpeg WAV,
//! FLAC, or Shine MP3. C callbacks enqueue ordered writes only; the app drains
//! them through its held Files route and must sync before reporting success.
#![no_std]
extern crate alloc;
use alloc::boxed::Box;
use core::ffi::{c_int, c_void};
mod output;
pub use output::Write;
pub const MAX_FRAMES: u64 = 3662 * 48000;
pub const MAX_BUFFERED_OUTPUT: usize = 1 << 20;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Format {
    Wav = 0,
    Flac = 1,
    Mp3 = 2,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Mp3 => "mp3",
        }
    }
}
impl TryFrom<u8> for Format {
    type Error = Error;
    fn try_from(value: u8) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Wav),
            1 => Ok(Self::Flac),
            2 => Ok(Self::Mp3),
            _ => Err(Error::Invalid),
        }
    }
}
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
    fn from(value: output::Failure) -> Self {
        match value {
            output::Failure::Limit => Self::OutputLimit,
            output::Failure::Memory => Self::Memory,
            _ => Self::Invalid,
        }
    }
}
pub enum Drain<'a> {
    Write(Write<'a>),
    NeedFrames,
    End,
}
struct Writer {
    output: output::Output,
    failed: Option<Error>,
}
pub struct Encoder {
    raw: *mut c_void,
    writer: Box<Writer>,
    capacity: usize,
    expected: u64,
    accepted: u64,
    ended: bool,
    failed: Option<Error>,
}
impl Encoder {
    pub fn new(format: Format, frames: u64) -> Result<Self, Error> {
        if frames == 0 || frames > MAX_FRAMES {
            return Err(Error::Invalid);
        }
        #[cfg(not(moan_codecs))]
        {
            let _ = format;
            return Err(Error::Unavailable);
        }
        #[cfg(moan_codecs)]
        {
            init();
            let mut writer = Box::new(Writer {
                output: output::Output::new(MAX_BUFFERED_OUTPUT),
                failed: None,
            });
            let mut error = 0;
            let mut capacity = 0;
            let raw = unsafe {
                moan_encoder_open(
                    format as c_int,
                    frames as i64,
                    (&mut *writer as *mut Writer).cast(),
                    write_bytes,
                    seek_bytes,
                    &mut capacity,
                    &mut error,
                )
            };
            if raw.is_null() {
                return Err(writer.failed.unwrap_or(Error::Codec(error)));
            }
            if !(1..=8192).contains(&capacity) {
                unsafe { moan_encoder_free(raw) };
                return Err(Error::Invalid);
            }
            Ok(Self {
                raw,
                writer,
                capacity: capacity as usize,
                expected: frames,
                accepted: 0,
                ended: false,
                failed: None,
            })
        }
    }
    pub fn frame_capacity(&self) -> usize {
        self.capacity
    }
    pub fn frames(&self) -> u64 {
        self.accepted
    }
    pub fn length(&self) -> u64 {
        self.writer.output.len()
    }
    pub fn buffered_bytes(&self) -> usize {
        self.writer.output.pending_bytes()
    }
    fn ready(&self) -> Result<(), Error> {
        if let Some(error) = self.failed {
            return Err(error);
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
        if code >= 0 && self.writer.failed.is_none() {
            return Ok(());
        }
        let error = self.writer.failed.unwrap_or(Error::Codec(code));
        self.failed = Some(error);
        Err(error)
    }
    /// Submit one codec-sized block, except the final block may be short. Input
    /// is copied into codec-owned memory before returning. Drain output first.
    pub fn push(&mut self, frames: &[[i16; 2]]) -> Result<(), Error> {
        self.ready()?;
        let end = self
            .accepted
            .checked_add(frames.len() as u64)
            .ok_or(Error::Invalid)?;
        if frames.is_empty()
            || frames.len() > self.capacity
            || end > self.expected
            || (frames.len() < self.capacity && end != self.expected)
        {
            return Err(Error::Invalid);
        }
        #[cfg(moan_codecs)]
        {
            let code = unsafe {
                moan_encoder_frame(self.raw, frames.as_ptr().cast(), frames.len() as c_int)
            };
            self.result(code)?;
            self.accepted = end;
            Ok(())
        }
        #[cfg(not(moan_codecs))]
        {
            Err(Error::Unavailable)
        }
    }
    pub fn finish(&mut self) -> Result<(), Error> {
        self.ready()?;
        if self.accepted != self.expected {
            return Err(Error::Invalid);
        }
        #[cfg(moan_codecs)]
        {
            let code = unsafe { moan_encoder_finish(self.raw) };
            self.result(code)?;
            unsafe { moan_encoder_free(self.raw) };
            self.raw = core::ptr::null_mut();
            self.ended = true;
            Ok(())
        }
        #[cfg(not(moan_codecs))]
        {
            Err(Error::Unavailable)
        }
    }
    pub fn output(&self) -> Result<Drain<'_>, Error> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        if let Some(write) = self.writer.output.pending() {
            return Ok(Drain::Write(write));
        }
        Ok(if self.ended {
            Drain::End
        } else {
            Drain::NeedFrames
        })
    }
    /// Exact turn acknowledgements support short writes without silently losing
    /// queued bytes. A zero write is refused; the file owner decides to abort.
    pub fn acknowledge(&mut self, token: u64, bytes: usize) -> Result<(), Error> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        self.writer.output.consume(token, bytes).map_err(Into::into)
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        #[cfg(moan_codecs)]
        unsafe {
            moan_encoder_free(self.raw)
        };
    }
}
#[cfg(all(moan_codecs, target_os = "none"))]
fn init() {
    fn log(bytes: &[u8]) {
        sex_pdx::serial_record!(
            "[moan.codec] {}",
            core::str::from_utf8(bytes).unwrap_or("?").trim_end()
        );
    }
    fn exit(code: c_int) -> ! {
        sex_pdx::task_finish(code as u64)
    }
    fn clock() -> (i64, u64) {
        (0, sex_pdx::get_ticks() * 1_000_000)
    }
    sex_libc::set_log_default(log);
    sex_libc::set_exit_default(exit);
    sex_libc::time::set_clock_default(clock);
}
#[cfg(all(moan_codecs, not(target_os = "none")))]
fn init() {}
#[cfg(moan_codecs)]
unsafe extern "C" {
    fn moan_encoder_open(
        format: c_int,
        expected: i64,
        opaque: *mut c_void,
        write: unsafe extern "C" fn(*mut c_void, *const u8, c_int) -> c_int,
        seek: unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64,
        capacity: *mut c_int,
        error: *mut c_int,
    ) -> *mut c_void;
    fn moan_encoder_frame(raw: *mut c_void, frames: *const i16, samples: c_int) -> c_int;
    fn moan_encoder_finish(raw: *mut c_void) -> c_int;
    fn moan_encoder_free(raw: *mut c_void);
}
#[cfg(moan_codecs)]
unsafe extern "C" fn write_bytes(opaque: *mut c_void, bytes: *const u8, size: c_int) -> c_int {
    if opaque.is_null() || size < 0 || (size != 0 && bytes.is_null()) {
        return -22;
    }
    let writer = unsafe { &mut *opaque.cast::<Writer>() };
    if writer.failed.is_some() {
        return -5;
    }
    let data = if size == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(bytes, size as usize) }
    };
    match writer.output.write(data) {
        Ok(()) => size,
        Err(error) => {
            writer.failed = Some(error.into());
            if error == output::Failure::Memory {
                -12
            } else {
                -28
            }
        }
    }
}
#[cfg(moan_codecs)]
unsafe extern "C" fn seek_bytes(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    if opaque.is_null() {
        return -22;
    }
    let writer = unsafe { &mut *opaque.cast::<Writer>() };
    if writer.failed.is_some() {
        return -5;
    }
    match writer.output.seek(offset, whence) {
        Ok(at) => at as i64,
        Err(error) => {
            writer.failed = Some(error.into());
            -22
        }
    }
}
