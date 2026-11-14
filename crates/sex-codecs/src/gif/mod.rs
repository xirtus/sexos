// SPDX-License-Identifier: GPL-3.0-only
//! Streaming GIF87a and GIF89a animation decoder.

extern crate alloc;

use alloc::vec::Vec;
use crate::animation::FrameTiming;
use crate::error::Error;
use crate::format::Probe;
use crate::image::{alloc_pixels, pack_argb, Image};
use crate::limits::{admit, Limits};

pub mod lzw;

/// One frame's descriptor and offset within the GIF byte stream.
#[derive(Clone, Copy, Debug)]
struct FrameInfo {
    left: usize,
    top: usize,
    width: usize,
    height: usize,
    interlaced: bool,
    palette_offset: usize,
    palette_len: usize,
    transparent_idx: Option<u8>,
    disposal: u8,
    delay_ms: u32,
    data_offset: usize,
    min_code_size: u8,
}

/// Metadata probe for GIF images.
pub fn probe_gif(bytes: &[u8]) -> Result<Probe, Error> {
    let (w, h) = gif_dimensions(bytes)?;
    let frames = count_frames(bytes);
    Ok(Probe {
        format: crate::Format::Gif,
        width: w,
        height: h,
        has_alpha: true,
        is_animated: frames > 1,
        frame_count: Some(frames),
        bit_depth: 8,
    })
}

fn u16_le(b: &[u8], at: usize) -> Result<usize, Error> {
    if at + 2 > b.len() {
        return Err(Error::Gif);
    }
    Ok(u16::from_le_bytes([b[at], b[at + 1]]) as usize)
}

fn skip_subblocks(b: &[u8], mut at: usize) -> Result<usize, Error> {
    loop {
        if at >= b.len() {
            return Err(Error::Gif);
        }
        let len = b[at] as usize;
        at += 1 + len;
        if len == 0 {
            return Ok(at);
        }
    }
}

pub fn gif_dimensions(b: &[u8]) -> Result<(usize, usize), Error> {
    if b.len() < 13 || !(b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
        return Err(Error::Gif);
    }
    Ok((u16_le(b, 6)?, u16_le(b, 8)?))
}

fn count_frames(b: &[u8]) -> usize {
    if b.len() < 13 { return 0; }
    let flags = b[10];
    let mut at = 13;
    if (flags & 0x80) != 0 {
        let n = 2usize << (flags & 7);
        at += 3 * n;
    }
    let mut frames = 0;
    while at < b.len() {
        match b[at] {
            0x3B => break,
            0x21 => {
                if at + 2 >= b.len() { break; }
                at = match skip_subblocks(b, at + 2) {
                    Ok(next) => next,
                    Err(_) => break,
                };
            }
            0x2C => {
                frames += 1;
                if at + 10 >= b.len() { break; }
                let lflags = b[at + 9];
                let mut data_pos = at + 10;
                if (lflags & 0x80) != 0 {
                    let n = 2usize << (lflags & 7);
                    data_pos += 3 * n;
                }
                if data_pos >= b.len() { break; }
                at = match skip_subblocks(b, data_pos + 1) {
                    Ok(next) => next,
                    Err(_) => break,
                };
            }
            _ => break,
        }
    }
    frames
}

/// Streaming GIF animation decoder.
pub struct GifAnimationDecoder<'a> {
    bytes: &'a [u8],
    width: usize,
    height: usize,
    frames: Vec<FrameInfo>,
    loop_count: Option<u32>,
    curr_frame: usize,
    saved_canvas: Vec<u32>,
    pending_disposal: Option<FrameInfo>,
    decomp_buf: Vec<u8>,
}

impl<'a> GifAnimationDecoder<'a> {
    pub fn open(bytes: &'a [u8], limits: &Limits) -> Result<Self, Error> {
        if bytes.len() > limits.max_input_bytes {
            return Err(Error::InputTooLarge);
        }
        let (width, height) = gif_dimensions(bytes)?;
        admit(limits, width, height)?;

        let flags = bytes[10];
        let mut at = 13;
        let global_palette = if (flags & 0x80) != 0 {
            let n = 2usize << (flags & 7);
            let p = (at, n);
            at += 3 * n;
            Some(p)
        } else {
            None
        };

        let mut frames = Vec::new();
        let mut loop_count = None;
        let mut transparent_idx = None;
        let mut disposal = 0u8;
        let mut delay_ms = 100u32;

        while at < bytes.len() {
            match bytes[at] {
                0x3B => break, // Trailer
                0x21 => {
                    // Extension
                    if at + 2 > bytes.len() { return Err(Error::Gif); }
                    let label = bytes[at + 1];
                    let block = at + 2;
                    if label == 0xF9 && bytes.get(block) == Some(&4) {
                        // Graphic Control Extension
                        let f = *bytes.get(block + 1).ok_or(Error::Gif)?;
                        disposal = (f >> 2) & 7;
                        let cs = u16_le(bytes, block + 2)? as u32;
                        delay_ms = if cs <= 1 { 100 } else { cs * 10 };
                        transparent_idx = if (f & 1) != 0 {
                            Some(*bytes.get(block + 4).ok_or(Error::Gif)?)
                        } else {
                            None
                        };
                    } else if label == 0xFF && bytes.get(block) == Some(&11)
                        && bytes.get(block + 1..block + 12) == Some(b"NETSCAPE2.0")
                    {
                        let sub = block + 12;
                        if bytes.get(sub) == Some(&3) && bytes.get(sub + 1) == Some(&1) {
                            let count = u16_le(bytes, sub + 2)? as u32;
                            loop_count = if count == 0 { None } else { Some(count) };
                        }
                    }
                    at = skip_subblocks(bytes, block)?;
                }
                0x2C => {
                    // Image Descriptor
                    if at + 10 > bytes.len() { return Err(Error::Gif); }
                    let left = u16_le(bytes, at + 1)?;
                    let top = u16_le(bytes, at + 3)?;
                    let fw = u16_le(bytes, at + 5)?;
                    let fh = u16_le(bytes, at + 7)?;
                    let lflags = bytes[at + 9];
                    let interlaced = (lflags & 0x40) != 0;

                    let mut data_pos = at + 10;
                    let (palette_offset, palette_len) = if (lflags & 0x80) != 0 {
                        let n = 2usize << (lflags & 7);
                        let p = (data_pos, n);
                        data_pos += 3 * n;
                        p
                    } else {
                        global_palette.ok_or(Error::Gif)?
                    };

                    if data_pos >= bytes.len() { return Err(Error::Gif); }
                    let min_code_size = bytes[data_pos];
                    let data_offset = data_pos + 1;

                    frames.push(FrameInfo {
                        left,
                        top,
                        width: fw,
                        height: fh,
                        interlaced,
                        palette_offset,
                        palette_len,
                        transparent_idx,
                        disposal,
                        delay_ms,
                        data_offset,
                        min_code_size,
                    });

                    at = skip_subblocks(bytes, data_offset)?;
                    // Reset transient per-frame state
                    transparent_idx = None;
                    disposal = 0;
                    delay_ms = 100;
                }
                _ => return Err(Error::Gif),
            }
        }

        if frames.is_empty() {
            return Err(Error::Gif);
        }

        let max_frame_pixels = frames.iter().map(|f| f.width * f.height).max().unwrap_or(0);
        let mut decomp_buf = Vec::new();
        decomp_buf.try_reserve_exact(max_frame_pixels).map_err(|_| Error::OutOfMemory)?;
        decomp_buf.resize(max_frame_pixels, 0);

        Ok(Self {
            bytes,
            width,
            height,
            frames,
            loop_count,
            curr_frame: 0,
            saved_canvas: Vec::new(),
            pending_disposal: None,
            decomp_buf,
        })
    }

    #[inline]
    pub fn width(&self) -> usize { self.width }
    #[inline]
    pub fn height(&self) -> usize { self.height }
    #[inline]
    pub fn frame_count(&self) -> usize { self.frames.len() }
    #[inline]
    pub fn loop_count(&self) -> Option<u32> { self.loop_count }

    pub fn reset(&mut self) -> Result<(), Error> {
        self.curr_frame = 0;
        self.pending_disposal = None;
        self.saved_canvas.clear();
        Ok(())
    }

    /// Steps the animation by decoding the next frame into `canvas`.
    pub fn next_frame(&mut self, canvas: &mut [u32]) -> Result<Option<FrameTiming>, Error> {
        let count = self.width * self.height;
        if canvas.len() < count {
            return Err(Error::TooLarge);
        }

        if self.curr_frame >= self.frames.len() {
            self.reset()?;
        }

        // Apply disposal requested by previous frame
        if let Some(prev) = self.pending_disposal.take() {
            match prev.disposal {
                2 => {
                    // Restore to background (transparent 0)
                    for y in prev.top..(prev.top + prev.height).min(self.height) {
                        for x in prev.left..(prev.left + prev.width).min(self.width) {
                            canvas[y * self.width + x] = 0;
                        }
                    }
                }
                3 => {
                    // Restore to previous saved canvas
                    if self.saved_canvas.len() == count {
                        canvas[..count].copy_from_slice(&self.saved_canvas);
                    }
                }
                _ => {} // 0, 1: keep
            }
        }

        let frame = self.frames[self.curr_frame];

        // Save canvas if current frame asks for disposal 3
        if frame.disposal == 3 {
            if self.saved_canvas.len() != count {
                self.saved_canvas.try_reserve_exact(count).map_err(|_| Error::OutOfMemory)?;
                self.saved_canvas.resize(count, 0);
            }
            self.saved_canvas.copy_from_slice(&canvas[..count]);
        }

        // Decompress frame indices
        let pixels_in_frame = frame.width * frame.height;
        if self.decomp_buf.len() < pixels_in_frame {
            self.decomp_buf.try_reserve_exact(pixels_in_frame).map_err(|_| Error::OutOfMemory)?;
            self.decomp_buf.resize(pixels_in_frame, 0);
        }

        let decoded_len = lzw::decode_lzw(
            self.bytes,
            frame.data_offset,
            frame.min_code_size,
            &mut self.decomp_buf[..pixels_in_frame],
        )?;

        // Map indices to pixels and write to canvas
        let pal_end = frame.palette_offset + frame.palette_len * 3;
        if pal_end > self.bytes.len() {
            return Err(Error::Gif);
        }
        let palette = &self.bytes[frame.palette_offset..pal_end];

        let mut src_idx = 0;
        let passes: &[(usize, usize)] = if frame.interlaced {
            &[(0, 8), (4, 8), (2, 4), (1, 2)]
        } else {
            &[(0, 1)]
        };

        for &(start_y, step_y) in passes {
            let mut y = start_y;
            while y < frame.height {
                let cy = frame.top + y;
                if cy < self.height {
                    for x in 0..frame.width {
                        if src_idx >= decoded_len { break; }
                        let idx = self.decomp_buf[src_idx];
                        src_idx += 1;

                        let cx = frame.left + x;
                        if cx < self.width {
                            if Some(idx) != frame.transparent_idx {
                                let c_idx = idx as usize;
                                if c_idx < frame.palette_len {
                                    let r = palette[c_idx * 3];
                                    let g = palette[c_idx * 3 + 1];
                                    let b = palette[c_idx * 3 + 2];
                                    canvas[cy * self.width + cx] = pack_argb(0xFF, r, g, b);
                                }
                            }
                        }
                    }
                } else {
                    src_idx += frame.width;
                }
                y += step_y;
            }
        }

        self.pending_disposal = Some(frame);
        self.curr_frame += 1;

        Ok(Some(FrameTiming {
            duration_ms: frame.delay_ms,
            is_last: self.curr_frame == self.frames.len(),
        }))
    }
}

/// Decodes the first frame of a GIF file into an `Image`.
pub fn decode_gif(bytes: &[u8], limits: &Limits) -> Result<Image, Error> {
    let mut anim = GifAnimationDecoder::open(bytes, limits)?;
    let count = anim.width * anim.height;
    let mut pixels = alloc_pixels(count)?;
    anim.next_frame(&mut pixels)?;
    Ok(Image {
        width: anim.width,
        height: anim.height,
        pixels,
    })
}

/// Decodes the first frame of a GIF directly into a caller slice.
pub fn decode_gif_into(bytes: &[u8], limits: &Limits, out: &mut [u32]) -> Result<(usize, usize), Error> {
    let mut anim = GifAnimationDecoder::open(bytes, limits)?;
    let count = anim.width * anim.height;
    if out.len() < count {
        return Err(Error::TooLarge);
    }
    anim.next_frame(out)?;
    Ok((anim.width, anim.height))
}
