//! EXIF orientation: turning stored pixels upright for display.

use alloc::vec::Vec;

use crate::Error;

/// Whether `orientation` swaps width and height (5 to 8 turn by 90 degrees).
pub fn swaps_axes(orientation: u8) -> bool {
    (5..=8).contains(&orientation)
}

/// Rearranges `pixels` (`w * h`, rows top to bottom; any pixel type, such as
/// sex-image's `u32` or `[u8; 4]` RGBA) as EXIF `orientation` says, so the
/// result shows upright. Returns the pixels with their new width and height.
///
/// | value | stored picture is shown after |
/// | --- | --- |
/// | 1 | nothing |
/// | 2 | mirroring left to right |
/// | 3 | rotating 180 degrees |
/// | 4 | mirroring top to bottom |
/// | 5 | transposing (mirror along the top-left to bottom-right diagonal) |
/// | 6 | rotating 90 degrees clockwise |
/// | 7 | transversing (mirror along the top-right to bottom-left diagonal) |
/// | 8 | rotating 90 degrees counter-clockwise |
///
/// Any other value is treated as 1.
pub fn apply_orientation<T: Copy>(pixels: &[T], w: usize, h: usize, orientation: u8) -> Result<(Vec<T>, usize, usize), Error> {
    let n = w.checked_mul(h).ok_or(Error::TooLarge)?;
    if pixels.len() < n {
        return Err(Error::TooLarge);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| Error::OutOfMemory)?;
    let (ow, oh) = if swaps_axes(orientation) { (h, w) } else { (w, h) };
    for y in 0..oh {
        for x in 0..ow {
            let (sx, sy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                8 => (w - 1 - y, x),
                _ => (x, y),
            };
            out.push(pixels[sy * w + sx]);
        }
    }
    Ok((out, ow, oh))
}

/// `apply_orientation` on a decoded sex-image picture.
pub fn orient_image(image: sex_image::Image, orientation: u8) -> Result<sex_image::Image, Error> {
    if !(2..=8).contains(&orientation) {
        return Ok(image);
    }
    let (pixels, width, height) = apply_orientation(&image.pixels, image.width, image.height, orientation)?;
    Ok(sex_image::Image { width, height, pixels })
}
