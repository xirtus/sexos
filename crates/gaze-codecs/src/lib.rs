//! Picture formats and photo metadata for Gaze, beyond what sex-image decodes.
//!
//! * [`sniff`]: recognises every picture format Gaze knows (decodable or not:
//!   AVIF and HEIC are only detected; their decoders come through FFmpeg).
//! * [`Exif`]: date taken, camera, lens, exposure, GPS and orientation from
//!   JPEG, PNG, WebP, JPEG XL, TIFF and TIFF-based RAW files.
//! * [`apply_orientation`]: turns pixels upright for all eight EXIF orientations.
//! * [`extract_preview`]: the embedded JPEG preview of camera RAW files.
//! * [`decode`]: one entry point that returns upright pixels in sex-image's
//!   [`Image`] layout (`u32` 0xAARRGGBB, straight alpha, rows top to bottom).
//! * JPEG XL (feature `jxl`), SVG (feature `svg`) and full camera RAW
//!   development through rawler (feature `raw`, LGPL-2.1). All three need
//!   std, so each turns on the `std` feature; without them the crate is
//!   `no_std` + alloc.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod exif;
mod format;
#[cfg(feature = "jxl")]
mod jxl;
mod orient;
mod raw;
#[cfg(feature = "raw")]
mod raw_decode;
#[cfg(feature = "svg")]
mod svg;
mod tiff;

pub use exif::{dms_to_degrees, find_exif, DateTime, Exif, Gps, Rational};
pub use format::{sniff, Format, Support};
#[cfg(feature = "jxl")]
pub use jxl::{decode_jxl, jxl_dimensions};
pub use orient::{apply_orientation, orient_image, swaps_axes};
pub use raw::{extract_preview, Preview};
#[cfg(feature = "raw")]
pub use raw_decode::{decode_raw, raw_orientation, RawSize};
pub use sex_image::{Image, Limits};
#[cfg(feature = "svg")]
pub use svg::{render_svg, svg_fit, svg_size, SVG_MAX_SIDE};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Not a picture format this crate recognises.
    UnknownFormat,
    /// A recognised format with no decoder in this build (AVIF, HEIC, TIFF,
    /// and JPEG XL or SVG without their features).
    NoDecoder(Format),
    /// A RAW file without a usable embedded JPEG preview.
    NoPreview,
    /// sex-image refused the file (or the RAW preview).
    Image(sex_image::Error),
    /// rawler could not develop the RAW file (feature `raw`).
    Raw,
    /// The JPEG XL decoder refused the file.
    Jxl,
    /// The SVG could not be parsed.
    Svg,
    /// The file is larger than `Limits::max_input_bytes`.
    InputTooLarge,
    /// The picture is larger than the limits allow, or empty.
    TooLarge,
    OutOfMemory,
    /// A colour layout this crate does not convert (CMYK JPEG XL).
    Unsupported,
}

impl From<sex_image::Error> for Error {
    fn from(e: sex_image::Error) -> Error {
        Error::Image(e)
    }
}

/// Checks a size against `limits`, returning the pixel count.
#[allow(dead_code)]
pub(crate) fn admit(limits: &Limits, w: usize, h: usize) -> Result<usize, Error> {
    if w == 0 || h == 0 || w > limits.max_width || h > limits.max_height {
        return Err(Error::TooLarge);
    }
    let n = w.checked_mul(h).ok_or(Error::TooLarge)?;
    if n > limits.max_pixels {
        return Err(Error::TooLarge);
    }
    Ok(n)
}

/// How [`decode_with`] works.
#[derive(Clone, Copy, Debug)]
pub struct Options<'a> {
    pub limits: Limits,
    /// Apply the EXIF orientation so the picture shows upright (default true).
    pub auto_orient: bool,
    /// Font files for SVG text (feature `svg`); empty leaves text out.
    pub fonts: &'a [&'a [u8]],
    /// Camera RAW (feature `raw`): `None` (default) shows the embedded JPEG
    /// preview, the fast path; `Some(size)` develops the sensor data. A RAW
    /// file without a usable preview is developed at half size either way.
    #[cfg(feature = "raw")]
    pub raw_develop: Option<RawSize>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options {
            limits: Limits::PHOTO,
            auto_orient: true,
            fonts: &[],
            #[cfg(feature = "raw")]
            raw_develop: None,
        }
    }
}

/// [`decode_with`] with default options for `limits`: upright, no fonts.
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, Error> {
    decode_with(bytes, &Options { limits: *limits, ..Options::default() })
}

/// Decodes any format Gaze can show into sex-image's layout.
///
/// * PNG, JPEG, WebP, GIF: sex-image; EXIF orientation applied when asked.
/// * Camera RAW: the largest embedded JPEG preview, oriented by the RAW's
///   own EXIF (falling back to the preview's).
/// * JPEG XL (`jxl`): the decoder applies the codestream orientation itself.
/// * SVG (`svg`): drawn at its intrinsic size, scaled down to fit the limits.
///
/// Limits apply to the stored size, before orientation turns it.
pub fn decode_with(bytes: &[u8], opt: &Options<'_>) -> Result<Image, Error> {
    let limits = &opt.limits;
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let format = sniff(bytes).ok_or(Error::UnknownFormat)?;
    let orient = |image: Image, orientation: Option<u8>| match (opt.auto_orient, orientation) {
        (true, Some(o)) => orient_image(image, o),
        _ => Ok(image),
    };
    match format {
        Format::Png | Format::Jpeg | Format::Webp | Format::Gif => {
            let image = sex_image::decode(bytes, limits)?;
            orient(image, Exif::read(bytes).and_then(|e| e.orientation))
        }
        #[cfg(not(feature = "raw"))]
        Format::Cr3 => Err(Error::NoDecoder(format)),
        #[cfg(feature = "raw")]
        f if f.is_raw() && (opt.raw_develop.is_some() || f == Format::Cr3 || extract_preview(bytes).is_none()) => {
            let image = decode_raw(bytes, limits, opt.raw_develop.unwrap_or(RawSize::Half))?;
            let orientation = Exif::read(bytes).and_then(|e| e.orientation).or_else(|| raw_orientation(bytes));
            orient(image, orientation)
        }
        f if f.is_raw() => {
            let preview = extract_preview(bytes).ok_or(Error::NoPreview)?;
            let image = sex_image::decode(preview.jpeg, limits)?;
            let orientation = Exif::read(bytes).and_then(|e| e.orientation).or_else(|| Exif::read(preview.jpeg).and_then(|e| e.orientation));
            orient(image, orientation)
        }
        #[cfg(feature = "jxl")]
        Format::Jxl => decode_jxl(bytes, limits),
        #[cfg(feature = "svg")]
        Format::Svg => {
            let (w, h) = svg_size(bytes)?;
            let scale = (limits.max_width as f32 / w).min(limits.max_height as f32 / h).min(1.0);
            let scale = scale.min((limits.max_pixels as f32 / (w * h)).sqrt());
            let side = |v: f32| ((v * scale + 0.5) as u32).clamp(1, SVG_MAX_SIDE);
            Ok(render_svg(bytes, side(w), side(h), opt.fonts)?)
        }
        f => Err(Error::NoDecoder(f)),
    }
}
