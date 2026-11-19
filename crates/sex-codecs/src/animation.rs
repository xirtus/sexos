// SPDX-License-Identifier: GPL-3.0-only
//! Unified streaming animation decoder interface.

use crate::error::Error;
use crate::format::{sniff, Format};
use crate::gif::GifAnimationDecoder;
use crate::image::Image;
use crate::limits::Limits;
use crate::webp::WebpAnimationDecoder;

/// Timing and sequencing information for an animation frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameTiming {
    /// How long this frame should be displayed, in milliseconds (minimum 20ms).
    pub duration_ms: u32,
    /// Whether this is the final frame in the animation cycle.
    pub is_last: bool,
}

enum DecoderKind<'a> {
    Webp(WebpAnimationDecoder<'a>),
    Gif(GifAnimationDecoder<'a>),
    Still {
        image: Image,
        shown: bool,
    },
}

/// Unified streaming animation decoder covering animated WebP, animated GIF,
/// and still images (PNG, JPEG, AVIF, HEIC) treated as single-frame animations.
pub struct AnimationDecoder<'a> {
    kind: DecoderKind<'a>,
    width: usize,
    height: usize,
    frame_count: usize,
    loop_count: Option<u32>,
}

impl<'a> AnimationDecoder<'a> {
    /// Opens an image or animation file and prepares the streaming decoder.
    pub fn open(bytes: &'a [u8], limits: &Limits) -> Result<Self, Error> {
        let fmt = sniff(bytes).ok_or(Error::UnknownFormat)?;
        match fmt {
            Format::Webp => {
                let dec = WebpAnimationDecoder::open(bytes, limits)?;
                let w = dec.width();
                let h = dec.height();
                let fc = dec.frame_count();
                let lc = dec.loop_count();
                Ok(Self {
                    kind: DecoderKind::Webp(dec),
                    width: w,
                    height: h,
                    frame_count: fc,
                    loop_count: lc,
                })
            }
            Format::Gif => {
                let dec = GifAnimationDecoder::open(bytes, limits)?;
                let w = dec.width();
                let h = dec.height();
                let fc = dec.frame_count();
                let lc = dec.loop_count();
                Ok(Self {
                    kind: DecoderKind::Gif(dec),
                    width: w,
                    height: h,
                    frame_count: fc,
                    loop_count: lc,
                })
            }
            Format::Avif => {
                let img = crate::avif::decode_avif(bytes, limits)?;
                let (w, h) = (img.width, img.height);
                Ok(Self {
                    kind: DecoderKind::Still { image: img, shown: false },
                    width: w,
                    height: h,
                    frame_count: 1,
                    loop_count: Some(0),
                })
            }
            Format::Heic => {
                let img = crate::heic::decode_heic(bytes, limits)?;
                let (w, h) = (img.width, img.height);
                Ok(Self {
                    kind: DecoderKind::Still { image: img, shown: false },
                    width: w,
                    height: h,
                    frame_count: 1,
                    loop_count: Some(0),
                })
            }
            Format::Png => {
                let img = sex_image::decode(bytes, limits)?;
                let (w, h) = (img.width, img.height);
                Ok(Self {
                    kind: DecoderKind::Still { image: img, shown: false },
                    width: w,
                    height: h,
                    frame_count: 1,
                    loop_count: Some(0),
                })
            }
            Format::Jpeg => {
                let img = sex_image::decode(bytes, limits)?;
                let (w, h) = (img.width, img.height);
                Ok(Self {
                    kind: DecoderKind::Still { image: img, shown: false },
                    width: w,
                    height: h,
                    frame_count: 1,
                    loop_count: Some(0),
                })
            }
        }
    }

    #[inline]
    pub fn width(&self) -> usize { self.width }
    #[inline]
    pub fn height(&self) -> usize { self.height }
    #[inline]
    pub fn frame_count(&self) -> usize { self.frame_count }
    #[inline]
    pub fn loop_count(&self) -> Option<u32> { self.loop_count }

    /// Resets animation playback to the first frame.
    pub fn reset(&mut self) -> Result<(), Error> {
        match &mut self.kind {
            DecoderKind::Webp(w) => w.reset(),
            DecoderKind::Gif(g) => g.reset(),
            DecoderKind::Still { shown, .. } => {
                *shown = false;
                Ok(())
            }
        }
    }

    /// Draws the next frame directly onto `canvas` without reallocation.
    /// Returns frame delay timing, or `None` when completed (for non-looping stills).
    pub fn next_frame(&mut self, canvas: &mut [u32]) -> Result<Option<FrameTiming>, Error> {
        let count = self.width * self.height;
        if canvas.len() < count {
            return Err(Error::TooLarge);
        }

        match &mut self.kind {
            DecoderKind::Webp(w) => w.next_frame(canvas),
            DecoderKind::Gif(g) => g.next_frame(canvas),
            DecoderKind::Still { image, shown } => {
                if *shown {
                    Ok(None)
                } else {
                    canvas[..count].copy_from_slice(&image.pixels);
                    *shown = true;
                    Ok(Some(FrameTiming {
                        duration_ms: 0,
                        is_last: true,
                    }))
                }
            }
        }
    }
}
