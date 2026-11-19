// SPDX-License-Identifier: GPL-3.0-only
//! WebP still and animated decoding.

extern crate alloc;

use alloc::vec::Vec;
use image_webp::{Cursor, LoopCount, WebPDecoder};

use crate::animation::FrameTiming;
use crate::error::Error;
use crate::format::Probe;
use crate::image::{alloc_pixels, pack_argb, Image};
use crate::limits::{admit, Limits};

pub mod riff;

pub use riff::{demux_webp, AnmfChunk, Vp8xHeader, WebpFile};

/// Inspects WebP file metadata without full pixel decoding.
pub fn probe_webp(bytes: &[u8]) -> Result<Probe, Error> {
    if let Ok(demuxed) = demux_webp(bytes) {
        if let Some(vp8x) = demuxed.vp8x {
            let count = if vp8x.has_animation {
                Some(demuxed.frames.len().max(1))
            } else {
                Some(1)
            };
            return Ok(Probe {
                format: crate::Format::Webp,
                width: vp8x.canvas_width,
                height: vp8x.canvas_height,
                has_alpha: vp8x.has_alpha,
                is_animated: vp8x.has_animation,
                frame_count: count,
                bit_depth: 8,
            });
        }
    }

    // Fallback through image_webp header parser
    let decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|_| Error::Webp)?;
    let (w, h) = decoder.dimensions();
    let is_animated = decoder.is_animated();
    let frames = if is_animated {
        Some(decoder.num_frames() as usize)
    } else {
        Some(1)
    };

    Ok(Probe {
        format: crate::Format::Webp,
        width: w as usize,
        height: h as usize,
        has_alpha: decoder.has_alpha(),
        is_animated,
        frame_count: frames,
        bit_depth: 8,
    })
}

/// Decodes a still WebP or the first frame of an animated WebP into an `Image`.
pub fn decode_webp(bytes: &[u8], limits: &Limits) -> Result<Image, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|_| Error::Webp)?;
    let (w, h) = decoder.dimensions();
    let count = admit(limits, w as usize, h as usize)?;

    let mut pixels = alloc_pixels(count)?;
    let has_alpha = decoder.has_alpha();
    let buf_size = decoder.output_buffer_size().ok_or(Error::TooLarge)?;

    // Allocate temporary byte buffer for decoder output
    let mut raw_bytes = Vec::new();
    raw_bytes.try_reserve_exact(buf_size).map_err(|_| Error::OutOfMemory)?;
    raw_bytes.resize(buf_size, 0);

    if decoder.is_animated() {
        decoder.read_frame(&mut raw_bytes).map_err(|_| Error::Webp)?;
    } else {
        decoder.read_image(&mut raw_bytes).map_err(|_| Error::Webp)?;
    }

    // Convert RGBA / RGB bytes into 0xAARRGGBB
    if has_alpha {
        for (i, p) in raw_bytes.chunks_exact(4).take(count).enumerate() {
            pixels[i] = pack_argb(p[3], p[0], p[1], p[2]);
        }
    } else {
        for (i, p) in raw_bytes.chunks_exact(3).take(count).enumerate() {
            pixels[i] = pack_argb(0xFF, p[0], p[1], p[2]);
        }
    }

    Ok(Image {
        width: w as usize,
        height: h as usize,
        pixels,
    })
}

/// Decodes directly into an existing caller-supplied pixel slice.
pub fn decode_webp_into(bytes: &[u8], limits: &Limits, out: &mut [u32]) -> Result<(usize, usize), Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let img = decode_webp(bytes, limits)?;
    let count = img.width * img.height;
    if out.len() < count {
        return Err(Error::TooLarge);
    }
    out[..count].copy_from_slice(&img.pixels);
    Ok((img.width, img.height))
}

/// Streaming WebP animation decoder.
pub struct WebpAnimationDecoder<'a> {
    decoder: WebPDecoder<Cursor<&'a [u8]>>,
    width: usize,
    height: usize,
    has_alpha: bool,
    is_animated: bool,
    total_frames: usize,
    frames_left: usize,
    loop_count: Option<u32>,
    buf_size: usize,
    raw_bytes: Vec<u8>,
}

impl<'a> WebpAnimationDecoder<'a> {
    pub fn open(bytes: &'a [u8], limits: &Limits) -> Result<Self, Error> {
        if bytes.len() > limits.max_input_bytes {
            return Err(Error::InputTooLarge);
        }
        let decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|_| Error::Webp)?;
        let (w, h) = decoder.dimensions();
        admit(limits, w as usize, h as usize)?;

        let is_animated = decoder.is_animated();
        let total_frames = if is_animated {
            decoder.num_frames() as usize
        } else {
            1
        };
        let loop_count = match decoder.loop_count() {
            LoopCount::Forever => None,
            LoopCount::Times(n) => Some(u16::from(n) as u32),
        };
        let has_alpha = decoder.has_alpha();
        let buf_size = decoder.output_buffer_size().ok_or(Error::TooLarge)?;

        let mut raw_bytes = Vec::new();
        raw_bytes.try_reserve_exact(buf_size).map_err(|_| Error::OutOfMemory)?;
        raw_bytes.resize(buf_size, 0);

        Ok(Self {
            decoder,
            width: w as usize,
            height: h as usize,
            has_alpha,
            is_animated,
            total_frames,
            frames_left: total_frames,
            loop_count,
            buf_size,
            raw_bytes,
        })
    }

    #[inline]
    pub fn width(&self) -> usize { self.width }
    #[inline]
    pub fn height(&self) -> usize { self.height }
    #[inline]
    pub fn frame_count(&self) -> usize { self.total_frames }
    #[inline]
    pub fn loop_count(&self) -> Option<u32> { self.loop_count }

    pub fn reset(&mut self) -> Result<(), Error> {
        if self.is_animated {
            self.decoder.reset_animation();
        }
        self.frames_left = self.total_frames;
        Ok(())
    }

    pub fn next_frame(&mut self, canvas: &mut [u32]) -> Result<Option<FrameTiming>, Error> {
        let count = self.width * self.height;
        if canvas.len() < count {
            return Err(Error::TooLarge);
        }

        if self.frames_left == 0 {
            if self.is_animated {
                self.reset()?;
            } else {
                return Ok(None);
            }
        }

        let delay_ms = if self.is_animated {
            let delay = self.decoder.read_frame(&mut self.raw_bytes[..self.buf_size]).map_err(|_| Error::Webp)?;
            if delay <= 10 { 100 } else { delay }
        } else {
            self.decoder.read_image(&mut self.raw_bytes[..self.buf_size]).map_err(|_| Error::Webp)?;
            0
        };

        if self.has_alpha {
            for (i, p) in self.raw_bytes.chunks_exact(4).take(count).enumerate() {
                canvas[i] = pack_argb(p[3], p[0], p[1], p[2]);
            }
        } else {
            for (i, p) in self.raw_bytes.chunks_exact(3).take(count).enumerate() {
                canvas[i] = pack_argb(0xFF, p[0], p[1], p[2]);
            }
        }

        self.frames_left -= 1;
        Ok(Some(FrameTiming {
            duration_ms: delay_ms,
            is_last: self.frames_left == 0,
        }))
    }
}
