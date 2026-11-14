//! Splicy's export encoder: finished pictures and stereo sound in, a WebM or MP4 file
//! out as ordered writes the caller makes through its own file grant (the same bounded
//! output queue as the recorder). Built when FFmpeg has the export profile
//! (`FFMPEG_RECORDING=2 scripts/build_ffmpeg.sh`); otherwise [`Exporter::new`] answers
//! [`Error::Unavailable`]. sex-splice's `export` module decides which frame shows what.
use crate::recording::{output, Drain, Error, Writer, MAX_BUFFERED_OUTPUT};
use alloc::boxed::Box;
use core::ffi::{c_int, c_void};

/// The file formats Splicy writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// WebM with VP9 picture and Opus sound. Royalty-free.
    WebmVp9Opus = 0,
    /// MP4 with H.264 picture and AAC sound. Plays on everything.
    Mp4H264Aac = 1,
    /// MP4 with AV1 picture and Opus sound. Royalty-free and smallest.
    Mp4Av1Opus = 2,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::WebmVp9Opus => "webm",
            _ => "mp4",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub format: Format,
    /// Even, at least 16.
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    /// Export sound at 48 kHz stereo.
    pub sound: bool,
    /// Constant quality (CRF); 0 picks the format's default.
    pub quality: u8,
}

/// Sound is always exported at this rate.
pub const RATE: u32 = 48_000;

pub struct Exporter {
    #[cfg_attr(not(ffmpeg_export), allow(dead_code))]
    raw: *mut c_void,
    writer: Box<Writer>,
    pixels: usize,
    #[cfg_attr(not(ffmpeg_export), allow(dead_code))]
    ended: bool,
    failed: Option<Error>,
}

impl Exporter {
    pub fn new(config: Config) -> Result<Self, Error> {
        if config.width < 16 || config.height < 16 || (config.width | config.height) & 1 != 0 || config.fps_den == 0 {
            return Err(Error::Invalid);
        }
        let mut writer = Box::new(Writer { output: output::Output::new(MAX_BUFFERED_OUTPUT), failure: None });
        let pixels = config.width as usize * config.height as usize;
        #[cfg(not(ffmpeg_export))]
        {
            let _ = (&mut writer, pixels);
            Err(Error::Unavailable)
        }
        #[cfg(ffmpeg_export)]
        {
            crate::init_libc();
            let mut error = 0;
            let raw = unsafe {
                sx_export_open(
                    config.format as c_int,
                    config.width as c_int,
                    config.height as c_int,
                    config.fps_num as c_int,
                    config.fps_den as c_int,
                    if config.sound { RATE as c_int } else { 0 },
                    config.quality as c_int,
                    (&mut *writer as *mut Writer).cast(),
                    crate::recording::write_bytes,
                    crate::recording::seek_bytes,
                    &mut error,
                )
            };
            if raw.is_null() {
                return Err(writer.failure.unwrap_or(if error == -38 { Error::Unavailable } else { Error::Codec(error) }));
            }
            Ok(Exporter { raw, writer, pixels, ended: false, failed: None })
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

    #[allow(dead_code)]
    fn result(&mut self, code: c_int) -> Result<(), Error> {
        if code >= 0 && self.writer.failure.is_none() {
            return Ok(());
        }
        let e = self.writer.failure.unwrap_or(Error::Codec(code));
        self.failed = Some(e);
        Err(e)
    }

    /// Picture number `frame` (increasing; a gap holds the previous picture). Drain the
    /// output before the next call.
    pub fn picture(&mut self, pixels: &[u32], frame: u64) -> Result<(), Error> {
        self.ready()?;
        if pixels.len() != self.pixels {
            return Err(Error::Invalid);
        }
        #[cfg(ffmpeg_export)]
        {
            let code = unsafe { sx_export_picture(self.raw, pixels.as_ptr(), frame as i64) };
            self.result(code)
        }
        #[cfg(not(ffmpeg_export))]
        {
            let _ = frame;
            Err(Error::Unavailable)
        }
    }

    /// The next stereo samples at [`RATE`]. Drain the output before the next call.
    pub fn sound(&mut self, pcm: &[[i16; 2]]) -> Result<(), Error> {
        self.ready()?;
        #[cfg(ffmpeg_export)]
        {
            let code = unsafe { sx_export_sound(self.raw, pcm.as_ptr().cast(), pcm.len() as c_int) };
            self.result(code)
        }
        #[cfg(not(ffmpeg_export))]
        {
            let _ = pcm;
            Err(Error::Unavailable)
        }
    }

    /// Write the file's index and close it; drain every write after.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.ready()?;
        #[cfg(ffmpeg_export)]
        {
            let code = unsafe { sx_export_finish(self.raw) };
            self.result(code)?;
            unsafe { sx_export_free(self.raw) };
            self.raw = core::ptr::null_mut();
            self.ended = true;
            Ok(())
        }
        #[cfg(not(ffmpeg_export))]
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
        Ok(if self.ended { Drain::End } else { Drain::NeedFrame })
    }

    pub fn acknowledge(&mut self, token: u64, bytes: usize) -> Result<(), Error> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        self.writer.output.consume(token, bytes).map_err(Into::into)
    }

    pub fn length(&self) -> u64 {
        self.writer.output.len()
    }
}

impl Drop for Exporter {
    fn drop(&mut self) {
        #[cfg(ffmpeg_export)]
        unsafe {
            sx_export_free(self.raw);
        }
    }
}

#[cfg(ffmpeg_export)]
unsafe extern "C" {
    fn sx_export_open(
        format: c_int,
        width: c_int,
        height: c_int,
        fps_num: c_int,
        fps_den: c_int,
        rate: c_int,
        quality: c_int,
        opaque: *mut c_void,
        write: unsafe extern "C" fn(*mut c_void, *const u8, c_int) -> c_int,
        seek: unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64,
        error: *mut c_int,
    ) -> *mut c_void;
    fn sx_export_picture(s: *mut c_void, pixels: *const u32, frame: i64) -> c_int;
    fn sx_export_sound(s: *mut c_void, pcm: *const i16, frames: c_int) -> c_int;
    fn sx_export_finish(s: *mut c_void) -> c_int;
    fn sx_export_free(s: *mut c_void);
}
