// SPDX-License-Identifier: GPL-3.0-only
//! Dimension and memory bounds checking.

pub use sex_image::Limits;

use crate::Error;

/// Checks whether an announced dimension fits within the provided limits.
/// Returns the total pixel count if valid.
#[inline]
pub fn admit(limits: &Limits, width: usize, height: usize) -> Result<usize, Error> {
    if width == 0 || height == 0 {
        return Err(Error::TooLarge);
    }
    if width > limits.max_width || height > limits.max_height {
        return Err(Error::TooLarge);
    }
    let pixels = width.checked_mul(height).ok_or(Error::TooLarge)?;
    if pixels > limits.max_pixels {
        return Err(Error::TooLarge);
    }
    Ok(pixels)
}
