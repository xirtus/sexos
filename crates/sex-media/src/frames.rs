// SPDX-License-Identifier: GPL-3.0-only
//! Decoding frames that come without a container, as WebRTC delivers them: one VP8,
//! VP9 or H.264 picture, or one Opus packet, at a time. Built with FFmpeg (`cfg(ffmpeg)`);
//! without it [`FrameDecoder::new`] is refused.
use crate::Error;
use alloc::vec::Vec;
#[allow(unused_imports)]
use core::ffi::{c_int, c_void};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec { Vp8 = 1, Vp9 = 2, H264 = 3, Opus = 4 }

#[cfg(ffmpeg)]
extern "C" {
    fn sx_frames_open(which: c_int, err: *mut c_int) -> *mut c_void;
    fn sx_frames_close(f: *mut c_void);
    fn sx_frames_video(f: *mut c_void, data: *const u8, len: c_int, out: *mut u32, max_w: c_int, max_h: c_int, w: *mut c_int, h: *mut c_int, ow: *mut c_int, oh: *mut c_int) -> c_int;
    fn sx_frames_audio(f: *mut c_void, data: *const u8, len: c_int, pcm: *mut i16, cap: c_int) -> c_int;
}

/// A picture: its size as sent, and the pixels fitted into the caller's bounds.
pub struct Picture { pub width: usize, pub height: usize, pub shown_w: usize, pub shown_h: usize }

pub struct FrameDecoder {
    #[cfg_attr(not(ffmpeg), allow(dead_code))]
    raw: *mut c_void,
    pub codec: Codec,
}

impl FrameDecoder {
    pub fn new(codec: Codec) -> Result<FrameDecoder, Error> {
        #[cfg(ffmpeg)]
        {
            crate::init_libc();
            let mut err = 0;
            let raw = unsafe { sx_frames_open(codec as c_int, &mut err) };
            if raw.is_null() { return Err(Error(err)); }
            Ok(FrameDecoder { raw, codec })
        }
        #[cfg(not(ffmpeg))]
        {
            let _ = codec;
            Err(Error::UNSUPPORTED)
        }
    }

    /// One encoded picture into `out` (0x00RRGGBB), fitted to `max_w` x `max_h`.
    /// None while the decoder has no picture to show yet (it waits for a key frame).
    pub fn picture(&mut self, data: &[u8], out: &mut Vec<u32>, max_w: usize, max_h: usize) -> Result<Option<Picture>, Error> {
        #[cfg(ffmpeg)]
        {
            out.resize(max_w * max_h, 0);
            let (mut w, mut h, mut ow, mut oh) = (0, 0, 0, 0);
            let r = unsafe { sx_frames_video(self.raw, data.as_ptr(), data.len() as c_int, out.as_mut_ptr(), max_w as c_int, max_h as c_int, &mut w, &mut h, &mut ow, &mut oh) };
            if r < 0 { return Err(Error(r)); }
            if r == 0 { return Ok(None); }
            out.truncate((ow * oh) as usize);
            Ok(Some(Picture { width: w as usize, height: h as usize, shown_w: ow as usize, shown_h: oh as usize }))
        }
        #[cfg(not(ffmpeg))]
        {
            let _ = (data, out, max_w, max_h);
            Err(Error::UNSUPPORTED)
        }
    }

    /// One sound packet: its 48 kHz stereo frames appended to `out`.
    pub fn sound(&mut self, data: &[u8], out: &mut Vec<[i16; 2]>) -> Result<usize, Error> {
        #[cfg(ffmpeg)]
        {
            let at = out.len();
            out.resize(at + 5760, [0; 2]);
            let n = unsafe { sx_frames_audio(self.raw, data.as_ptr(), data.len() as c_int, out[at..].as_mut_ptr().cast(), 5760) };
            if n < 0 { out.truncate(at); return Err(Error(n)); }
            out.truncate(at + n as usize);
            Ok(n as usize)
        }
        #[cfg(not(ffmpeg))]
        {
            let _ = (data, out);
            Err(Error::UNSUPPORTED)
        }
    }
}

impl Drop for FrameDecoder {
    fn drop(&mut self) {
        #[cfg(ffmpeg)]
        unsafe { sx_frames_close(self.raw) }
    }
}
