//! Full camera RAW decoding through rawler (LGPL-2.1, from the dnglab
//! project; LGPL is acceptable for SexOS libraries we link, our own code
//! stays GPL-3.0). rawler needs std, so this is behind the `raw` feature.
//!
//! The embedded JPEG preview ([`crate::extract_preview`]) stays the fast
//! default for showing a RAW file; this develops the sensor data itself:
//! black/white levels, demosaic, white balance, the camera's colour matrix
//! to sRGB, the default crop and the sRGB gamma curve.

use alloc::vec::Vec;
use std::panic::{catch_unwind, AssertUnwindSafe};

use rawler::decoders::{Orientation, RawDecodeParams};
use rawler::imgop::chromatic_adaption::adapt_bradford;
use rawler::imgop::develop::{Intermediate, RawDevelop};
use rawler::imgop::matrix::{multiply, normalize, pseudo_inverse, transform_1d};
use rawler::imgop::sensor::bayer::superpixel::Superpixel3Channel;
use rawler::imgop::sensor::{Demosaic, SensorType};
use rawler::imgop::srgb::srgb_apply_gamma;
use rawler::imgop::xyz::{Illuminant, SRGB_TO_XYZ_D65};
use rawler::pixarray::{Color2D, PixF32};
use rawler::rawimage::RawPhotometricInterpretation;
use rawler::rawsource::RawSource;
use rawler::RawImage;

use crate::{admit, Error};
use sex_image::{Image, Limits};

/// How large a developed RAW comes out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RawSize {
    /// Each 2x2 Bayer cell becomes one pixel ("superpixel"): half the width
    /// and height, no interpolation, several times faster. Sensors that are
    /// not RGB Bayer (X-Trans, four-colour) are developed at full size and
    /// then halved.
    Half,
    /// Full sensor resolution, demosaiced with rawler's PPG interpolation.
    Full,
}

/// Develops a camera RAW file into sex-image's pixel layout, in sRGB.
/// EXIF orientation is not applied (see [`crate::decode_with`], which does).
/// `limits` bound the sensor size before any pixel memory is reserved.
///
/// rawler can panic on malformed files; on a host (unwinding) build that
/// panic is caught and returned as [`Error::Raw`].
pub fn decode_raw(bytes: &[u8], limits: &Limits, size: RawSize) -> Result<Image, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    catch_unwind(AssertUnwindSafe(|| develop(bytes, limits, size))).unwrap_or(Err(Error::Raw))
}

/// The orientation (1 to 8) rawler read from the RAW file, if any.
pub fn raw_orientation(bytes: &[u8]) -> Option<u8> {
    let raw = catch_unwind(AssertUnwindSafe(|| load(bytes).ok())).ok()??;
    Some(match raw.orientation {
        Orientation::Normal => 1,
        Orientation::HorizontalFlip => 2,
        Orientation::Rotate180 => 3,
        Orientation::VerticalFlip => 4,
        Orientation::Transpose => 5,
        Orientation::Rotate90 => 6,
        Orientation::Transverse => 7,
        Orientation::Rotate270 => 8,
        Orientation::Unknown => return None,
    })
}

fn load(bytes: &[u8]) -> Result<RawImage, Error> {
    let src = RawSource::new_from_slice(bytes);
    rawler::decode(&src, &RawDecodeParams::default()).map_err(|_| Error::Raw)
}

fn develop(bytes: &[u8], limits: &Limits, size: RawSize) -> Result<Image, Error> {
    let mut raw = load(bytes)?;
    // The sensor size (pixels per row include all samples) bounds memory.
    admit(limits, raw.width / raw.cpp.max(1), raw.height)?;
    let bayer = matches!(&raw.photometric,
        RawPhotometricInterpretation::Cfa(c) if raw.cpp == 1 && c.cfa.is_rgb() && c.sensor == SensorType::Bayer && c.colors.plane_count() == 3);
    match size {
        RawSize::Half if bayer => half_bayer(&mut raw),
        RawSize::Half => Ok(full(&raw)?.fit_within(raw.width / 2, raw.height / 2)?),
        RawSize::Full => full(&raw),
    }
}

fn full(raw: &RawImage) -> Result<Image, Error> {
    match RawDevelop::default().develop_intermediate(raw).map_err(|_| Error::Raw)? {
        Intermediate::ThreeColor(c) => Ok(to_image(&c.data, c.width, c.height, |p| p)),
        Intermediate::Monochrome(m) => Ok(to_image(&m.data, m.width, m.height, |v| [v, v, v])),
        Intermediate::FourColor(_) => Err(Error::Unsupported),
    }
}

/// rawler's develop pipeline, with the superpixel debayer in place of PPG.
fn half_bayer(raw: &mut RawImage) -> Result<Image, Error> {
    let RawPhotometricInterpretation::Cfa(config) = raw.photometric.clone() else {
        return Err(Error::Raw);
    };
    raw.apply_scaling().map_err(|_| Error::Raw)?;
    let pixels = PixF32::new_with(raw.data.as_f32().into_owned(), raw.width, raw.height);
    let roi = raw.active_area.unwrap_or(pixels.rect());
    let mut rgb: Color2D<f32, 3> = Superpixel3Channel::new().demosaic(&pixels, &config.cfa, &config.colors, roi);

    // White balance and camera colour to linear sRGB.
    let cam2rgb = cam_to_srgb(raw);
    let wb = if raw.wb_coeffs[0].is_nan() { [1.0; 4] } else { raw.wb_coeffs };
    for p in rgb.data.iter_mut() {
        let (r, g, b) = (p[0] * wb[0], p[1] * wb[1], p[2] * wb[2]);
        *p = [0, 1, 2].map(|i| cam2rgb[i][0] * r + cam2rgb[i][1] * g + cam2rgb[i][2] * b);
    }

    // Default crop, relative to the active area and halved.
    if let Some(mut crop) = raw.crop_area.or(raw.active_area) {
        if let Some(active) = &raw.active_area {
            if crop.p.x >= active.p.x && crop.p.y >= active.p.y && crop.d.w <= active.d.w && crop.d.h <= active.d.h {
                crop = crop.adapt(active);
            }
        }
        crop.scale(0.5);
        let fits = crop.p.x + crop.d.w <= rgb.width && crop.p.y + crop.d.h <= rgb.height && crop.d.w > 0 && crop.d.h > 0;
        if fits && crop.d != rgb.dim() {
            rgb = rgb.crop(crop);
        }
    }
    Ok(to_image(&rgb.data, rgb.width, rgb.height, |[r, g, b]| [r, g, b].map(srgb_apply_gamma)))
}

/// The camera-RGB to linear-sRGB matrix from the RAW's colour matrix
/// (XYZ to camera), preferring its D65 one; identity when it has none.
fn cam_to_srgb(raw: &RawImage) -> [[f32; 3]; 3] {
    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let order = [
        Illuminant::D65,
        Illuminant::A,
        Illuminant::B,
        Illuminant::C,
        Illuminant::D50,
        Illuminant::D55,
        Illuminant::D75,
        Illuminant::Daylight,
        Illuminant::Flash,
    ];
    let Some((illu, flat)) = raw.color_matrix_find_first(order) else { return IDENTITY };
    let Some(m) = flat.get(..9).and_then(transform_1d::<3, 3>) else { return IDENTITY };
    let xyz2cam = if illu == Illuminant::D65 { m } else { adapt_bradford(&illu, &Illuminant::D65, &m) };
    let rgb2cam = normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65));
    pseudo_inverse(rgb2cam)
}

fn to_image<T: Copy>(data: &[T], width: usize, height: usize, f: impl Fn(T) -> [f32; 3]) -> Image {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let pixels: Vec<u32> = data
        .iter()
        .map(|&p| {
            let [r, g, b] = f(p);
            0xff00_0000 | q(r) << 16 | q(g) << 8 | q(b)
        })
        .collect();
    Image { width, height, pixels }
}
