//! Linear-light FP32 RGBA pixel buffers, BGRA8 sRGB conversion, and YUV (NV12 / 10-bit P010)
//! ingest/export with an automatic full-range clipping detector.

use alloc::vec;
use alloc::vec::Vec;
use libm::roundf;

use crate::space::{
    convert_primaries, hlg_to_linear_nits, linear_nits_to_hlg, linear_to_rec2020,
    linear_to_rec601, linear_to_rec709, linear_to_srgb, pq_eotf_nits, pq_oetf_nits,
    rec2020_to_linear, rec601_to_linear, rec709_to_linear, srgb_to_linear, tone_map,
    ColorPrimaries, ColorSpace, ToneMapConfig, HLG_NOMINAL_PEAK_NITS, REF_WHITE_NITS,
};

/// Maximum finite value produced by [`RgbaF32::sanitize`] when replacing `+inf`.
const SANITIZE_POS_INF_FALLBACK: f32 = 65_504.0;

#[inline]
fn sanitize_channel(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else if v.is_nan() || v.is_sign_negative() {
        0.0
    } else {
        SANITIZE_POS_INF_FALLBACK
    }
}

#[inline]
fn clamp_channel_nan_safe(v: f32, min: f32, max: f32) -> f32 {
    if v.is_nan() {
        min
    } else if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// 32-bit floating-point linear-light RGBA pixel `[R, G, B, A]`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct RgbaF32(pub [f32; 4]);

impl RgbaF32 {
    pub const TRANSPARENT: Self = Self([0.0, 0.0, 0.0, 0.0]);
    pub const BLACK: Self = Self([0.0, 0.0, 0.0, 1.0]);
    pub const WHITE: Self = Self([1.0, 1.0, 1.0, 1.0]);

    /// Constructs a pixel from `(r, g, b, a)`.
    #[inline]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self([r, g, b, a])
    }

    /// Constructs a pixel from an RGB triplet and alpha scalar.
    #[inline]
    pub const fn from_rgb(rgb: [f32; 3], a: f32) -> Self {
        Self([rgb[0], rgb[1], rgb[2], a])
    }

    /// Returns `[r, g, b]`.
    #[inline]
    pub const fn rgb(self) -> [f32; 3] {
        [self.0[0], self.0[1], self.0[2]]
    }

    #[inline]
    pub const fn r(self) -> f32 {
        self.0[0]
    }

    #[inline]
    pub const fn g(self) -> f32 {
        self.0[1]
    }

    #[inline]
    pub const fn b(self) -> f32 {
        self.0[2]
    }

    #[inline]
    pub const fn a(self) -> f32 {
        self.0[3]
    }

    /// Returns a `NaN`/`inf`-safe pixel where non-finite channels are replaced (`NaN`/`-inf` -> `0.0`,
    /// `+inf` -> `65504.0`) and alpha is clamped to `[0.0, 1.0]`. Finite RGB values (including
    /// negative out-of-gamut or HDR `> 1.0` values) are preserved.
    #[inline]
    pub fn sanitize(self) -> Self {
        Self([
            sanitize_channel(self.0[0]),
            sanitize_channel(self.0[1]),
            sanitize_channel(self.0[2]),
            clamp_channel_nan_safe(self.0[3], 0.0, 1.0),
        ])
    }

    /// Sanitizes non-finite values and clamps negative RGB channels to `0.0`.
    #[inline]
    pub fn sanitize_non_negative(self) -> Self {
        let s = self.sanitize();
        Self([s.0[0].max(0.0), s.0[1].max(0.0), s.0[2].max(0.0), s.0[3]])
    }

    /// Clamps all 4 channels to `[min, max]` safely (`NaN` -> `min`).
    #[inline]
    pub fn clamp(self, min: f32, max: f32) -> Self {
        let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
        Self([
            clamp_channel_nan_safe(self.0[0], lo, hi),
            clamp_channel_nan_safe(self.0[1], lo, hi),
            clamp_channel_nan_safe(self.0[2], lo, hi),
            clamp_channel_nan_safe(self.0[3], lo, hi),
        ])
    }

    /// Clamps all 4 channels to `[0.0, 1.0]` safely (`NaN` -> `0.0`).
    #[inline]
    pub fn clamp01(self) -> Self {
        self.clamp(0.0, 1.0)
    }

    /// Converts straight-alpha RGBA to premultiplied alpha (`[r*a, g*a, b*a, a]`).
    /// Guards against `0.0 * inf = NaN`.
    #[inline]
    pub fn premultiply(self) -> Self {
        let a = clamp_channel_nan_safe(self.0[3], 0.0, 1.0);
        if a <= 0.0 {
            return Self::TRANSPARENT;
        }
        let r = sanitize_channel(self.0[0]);
        let g = sanitize_channel(self.0[1]);
        let b = sanitize_channel(self.0[2]);
        Self([r * a, g * a, b * a, a])
    }

    /// Converts premultiplied RGBA back to straight alpha.
    /// Zero/near-zero alpha returns [`RgbaF32::TRANSPARENT`].
    #[inline]
    pub fn unpremultiply(self) -> Self {
        let a = clamp_channel_nan_safe(self.0[3], 0.0, 1.0);
        if a <= 1e-6 {
            return Self::TRANSPARENT;
        }
        let inv = 1.0 / a;
        Self([
            sanitize_channel(self.0[0]) * inv,
            sanitize_channel(self.0[1]) * inv,
            sanitize_channel(self.0[2]) * inv,
            a,
        ])
    }

    /// Porter-Duff source-over compositing (`self` over `dst`, assuming premultiplied inputs).
    #[inline]
    pub fn over_premultiplied(self, dst: Self) -> Self {
        let a = clamp_channel_nan_safe(self.0[3], 0.0, 1.0);
        let inv_a = 1.0 - a;
        Self([
            sanitize_channel(self.0[0]) + sanitize_channel(dst.0[0]) * inv_a,
            sanitize_channel(self.0[1]) + sanitize_channel(dst.0[1]) * inv_a,
            sanitize_channel(self.0[2]) + sanitize_channel(dst.0[2]) * inv_a,
            a + clamp_channel_nan_safe(dst.0[3], 0.0, 1.0) * inv_a,
        ])
    }

    /// Porter-Duff source-over compositing for straight-alpha inputs, returning straight alpha.
    #[inline]
    pub fn over_straight(self, dst: Self) -> Self {
        self.premultiply()
            .over_premultiplied(dst.premultiply())
            .unpremultiply()
    }

    /// ITU-R BT.709 / sRGB linear luma (`0.2126 R + 0.7152 G + 0.0722 B`).
    #[inline]
    pub fn rec709_luma(self) -> f32 {
        0.2126 * self.0[0] + 0.7152 * self.0[1] + 0.0722 * self.0[2]
    }

    /// ITU-R BT.2020 linear luma (`0.2627 R + 0.6780 G + 0.0593 B`).
    #[inline]
    pub fn rec2020_luma(self) -> f32 {
        0.2627 * self.0[0] + 0.6780 * self.0[1] + 0.0593 * self.0[2]
    }

    /// ITU-R BT.601 linear luma (`0.299 R + 0.587 G + 0.114 B`).
    #[inline]
    pub fn rec601_luma(self) -> f32 {
        0.2990 * self.0[0] + 0.5870 * self.0[1] + 0.1140 * self.0[2]
    }

    /// Linear luma computed with the weights of `primaries`.
    #[inline]
    pub fn luma_for(self, primaries: ColorPrimaries) -> f32 {
        let [kr, kg, kb] = primaries.luma_weights();
        kr * self.0[0] + kg * self.0[1] + kb * self.0[2]
    }
}

/// Errors produced by [`LinearFrame`] constructors and conversions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Width or height is zero, overflows `width * height`, or is odd for 4:2:0 YUV.
    InvalidDimensions,
    /// Provided input slice is smaller than required for the declared dimensions.
    BufferTooShort,
    /// Transfer space is incompatible with pixel buffer operations (e.g. `Oklab`).
    UnsupportedColorSpace,
}

/// Y'CbCr matrix standard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum YuvMatrix {
    Bt601,
    Bt709,
    Bt2020,
}

impl YuvMatrix {
    /// Returns `[Kr, Kg, Kb]` coefficients.
    #[inline]
    pub const fn weights(self) -> [f32; 3] {
        match self {
            Self::Bt601 => [0.2990, 0.5870, 0.1140],
            Self::Bt709 => [0.2126, 0.7152, 0.0722],
            Self::Bt2020 => [0.2627, 0.6780, 0.0593],
        }
    }

    /// Returns the default [`ColorPrimaries`] associated with this YUV matrix.
    #[inline]
    pub const fn default_primaries(self) -> ColorPrimaries {
        match self {
            Self::Bt601 => ColorPrimaries::Rec601,
            Self::Bt709 => ColorPrimaries::Rec709,
            Self::Bt2020 => ColorPrimaries::Rec2020,
        }
    }
}

/// Y'CbCr sample quantization range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum YuvRange {
    /// Broadcast/TV/Studio limited range (8-bit: `Y in [16..235]`, `UV in [16..240]`;
    /// 10-bit: `Y in [64..940]`, `UV in [64..960]`).
    Limited,
    /// Full / PC / JPEG range (`[0..255]` or `[0..1023]`).
    Full,
    /// Automatically choose [`YuvRange::Full`] if out-of-limited-range clipping is detected.
    Auto,
}

/// Diagnostic report from the auto full-range clipping detector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeDetection {
    /// Resolved range (`Full` if clipping outside limited bounds was detected, else `Limited`).
    pub detected_range: YuvRange,
    /// Number of luma samples below limited black (`< 16` for 8-bit, `< 64` for 10-bit).
    pub y_below_min_count: usize,
    /// Number of luma samples above limited white (`> 235` for 8-bit, `> 940` for 10-bit).
    pub y_above_max_count: usize,
    /// Number of chroma samples outside limited bounds (`[16..240]` or `[64..960]`).
    pub uv_out_of_range_count: usize,
    /// Fraction of luma samples that would clip if decoded as limited range.
    pub would_clip_fraction: f32,
}

/// Scans packed NV12 planes to detect whether full-range data would clip if decoded as limited-range.
pub fn detect_nv12_range(width: usize, height: usize, nv12: &[u8]) -> Result<RangeDetection, FrameError> {
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(FrameError::InvalidDimensions);
    }
    let y_len = width.checked_mul(height).ok_or(FrameError::InvalidDimensions)?;
    let uv_len = y_len / 2;
    let needed = y_len.checked_add(uv_len).ok_or(FrameError::InvalidDimensions)?;
    if nv12.len() < needed {
        return Err(FrameError::BufferTooShort);
    }
    Ok(detect_nv12_planes_range(
        width,
        height,
        &nv12[..y_len],
        width,
        &nv12[y_len..y_len + uv_len],
        width,
    ))
}

fn detect_nv12_planes_range(
    width: usize,
    height: usize,
    y_plane: &[u8],
    y_stride: usize,
    uv_plane: &[u8],
    uv_stride: usize,
) -> RangeDetection {
    let mut y_below = 0usize;
    let mut y_above = 0usize;
    let mut uv_out = 0usize;
    let mut extreme = 0usize;

    for row in 0..height {
        let line = &y_plane[row * y_stride..row * y_stride + width];
        for &y in line {
            if y < 16 {
                y_below += 1;
                if y <= 10 {
                    extreme += 1;
                }
            } else if y > 235 {
                y_above += 1;
                if y >= 245 {
                    extreme += 1;
                }
            }
        }
    }

    let chroma_h = height / 2;
    for row in 0..chroma_h {
        let line = &uv_plane[row * uv_stride..row * uv_stride + width];
        for &c in line {
            if !(16..=240).contains(&c) {
                uv_out += 1;
                if !(11..=244).contains(&c) {
                    extreme += 1;
                }
            }
        }
    }

    let total_y = (width * height).max(1);
    let total_uv = (width * chroma_h).max(1);
    let y_clip = y_below + y_above;
    let would_clip_fraction = y_clip as f32 / total_y as f32;
    let is_full = extreme > 0 || y_clip * 200 > total_y || uv_out * 200 > total_uv;

    RangeDetection {
        detected_range: if is_full { YuvRange::Full } else { YuvRange::Limited },
        y_below_min_count: y_below,
        y_above_max_count: y_above,
        uv_out_of_range_count: uv_out,
        would_clip_fraction,
    }
}

fn detect_p010_planes_range(
    width: usize,
    height: usize,
    y_plane: &[u16],
    y_stride: usize,
    uv_plane: &[u16],
    uv_stride: usize,
) -> RangeDetection {
    let mut y_below = 0usize;
    let mut y_above = 0usize;
    let mut uv_out = 0usize;
    let mut extreme = 0usize;

    for row in 0..height {
        let line = &y_plane[row * y_stride..row * y_stride + width];
        for &word in line {
            let y10 = word >> 6;
            if y10 < 64 {
                y_below += 1;
                if y10 <= 40 {
                    extreme += 1;
                }
            } else if y10 > 940 {
                y_above += 1;
                if y10 >= 980 {
                    extreme += 1;
                }
            }
        }
    }

    let chroma_h = height / 2;
    for row in 0..chroma_h {
        let line = &uv_plane[row * uv_stride..row * uv_stride + width];
        for &word in line {
            let c10 = word >> 6;
            if !(64..=960).contains(&c10) {
                uv_out += 1;
                if !(40..=980).contains(&c10) {
                    extreme += 1;
                }
            }
        }
    }

    let total_y = (width * height).max(1);
    let total_uv = (width * chroma_h).max(1);
    let y_clip = y_below + y_above;
    let would_clip_fraction = y_clip as f32 / total_y as f32;
    let is_full = extreme > 0 || y_clip * 200 > total_y || uv_out * 200 > total_uv;

    RangeDetection {
        detected_range: if is_full { YuvRange::Full } else { YuvRange::Limited },
        y_below_min_count: y_below,
        y_above_max_count: y_above,
        uv_out_of_range_count: uv_out,
        would_clip_fraction,
    }
}

/// Linear-light floating-point RGBA frame buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearFrame {
    pub width: usize,
    pub height: usize,
    pub primaries: ColorPrimaries,
    pub pixels: Vec<RgbaF32>,
}

impl LinearFrame {
    /// Allocates a new linear-light frame filled with [`RgbaF32::TRANSPARENT`].
    pub fn new(width: usize, height: usize, primaries: ColorPrimaries) -> Result<Self, FrameError> {
        Self::filled(width, height, primaries, RgbaF32::TRANSPARENT)
    }

    /// Allocates a new linear-light frame filled with `fill`.
    pub fn filled(
        width: usize,
        height: usize,
        primaries: ColorPrimaries,
        fill: RgbaF32,
    ) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::InvalidDimensions);
        }
        let len = width.checked_mul(height).ok_or(FrameError::InvalidDimensions)?;
        Ok(Self {
            width,
            height,
            primaries,
            pixels: vec![fill; len],
        })
    }

    /// Wraps an existing pixel buffer after validating its length.
    pub fn from_pixels(
        width: usize,
        height: usize,
        primaries: ColorPrimaries,
        pixels: Vec<RgbaF32>,
    ) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::InvalidDimensions);
        }
        let len = width.checked_mul(height).ok_or(FrameError::InvalidDimensions)?;
        if pixels.len() != len {
            return Err(FrameError::BufferTooShort);
        }
        Ok(Self {
            width,
            height,
            primaries,
            pixels,
        })
    }

    /// Returns the pixel at `(x, y)`, or `None` if out of bounds.
    #[inline]
    pub fn pixel(&self, x: usize, y: usize) -> Option<RgbaF32> {
        if x < self.width && y < self.height {
            self.pixels.get(y * self.width + x).copied()
        } else {
            None
        }
    }

    /// Sets the pixel at `(x, y)` if in bounds.
    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, px: RgbaF32) {
        if x < self.width && y < self.height {
            self.pixels[y * self.width + x] = px;
        }
    }

    /// Converts all pixels in place to `target` primaries without gamut clipping.
    pub fn convert_primaries_in_place(&mut self, target: ColorPrimaries) {
        if self.primaries == target {
            return;
        }
        for px in &mut self.pixels {
            let rgb = convert_primaries(px.rgb(), self.primaries, target);
            *px = RgbaF32::from_rgb(rgb, px.0[3]);
        }
        self.primaries = target;
    }

    /// Returns a copy converted to `target` primaries without gamut clipping.
    pub fn convert_primaries(&self, target: ColorPrimaries) -> Self {
        let mut out = self.clone();
        out.convert_primaries_in_place(target);
        out
    }

    /// Applies HDR tone mapping in place across all pixels.
    pub fn tone_map_in_place(&mut self, cfg: ToneMapConfig) {
        for px in &mut self.pixels {
            let mapped = tone_map(px.rgb(), cfg.mode, cfg.source_peak_nits, cfg.target_peak_nits);
            *px = RgbaF32::from_rgb(mapped, px.0[3]);
        }
    }

    /// Decodes a packed BGRA8 (`[B, G, R, A]` bytes) sRGB buffer into a `LinearFrame` (`Rec709`).
    pub fn from_bgra8_srgb(width: usize, height: usize, bgra: &[u8]) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::InvalidDimensions);
        }
        let count = width.checked_mul(height).ok_or(FrameError::InvalidDimensions)?;
        let byte_len = count.checked_mul(4).ok_or(FrameError::InvalidDimensions)?;
        if bgra.len() < byte_len {
            return Err(FrameError::BufferTooShort);
        }
        let mut pixels = Vec::with_capacity(count);
        for chunk in bgra[..byte_len].chunks_exact(4) {
            let b = srgb_to_linear(chunk[0] as f32 / 255.0);
            let g = srgb_to_linear(chunk[1] as f32 / 255.0);
            let r = srgb_to_linear(chunk[2] as f32 / 255.0);
            let a = chunk[3] as f32 / 255.0;
            pixels.push(RgbaF32([r, g, b, a]));
        }
        Ok(Self {
            width,
            height,
            primaries: ColorPrimaries::Rec709,
            pixels,
        })
    }

    /// Encodes this frame to packed BGRA8 (`[B, G, R, A]` bytes) sRGB, converting primaries to
    /// `Rec709` if necessary and clamping `[0.0, 1.0]`.
    pub fn to_bgra8_srgb(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for &px in &self.pixels {
            let s = px.sanitize();
            let rgb = convert_primaries(s.rgb(), self.primaries, ColorPrimaries::Rec709);
            let r = roundf(linear_to_srgb(rgb[0].clamp(0.0, 1.0)) * 255.0) as u8;
            let g = roundf(linear_to_srgb(rgb[1].clamp(0.0, 1.0)) * 255.0) as u8;
            let b = roundf(linear_to_srgb(rgb[2].clamp(0.0, 1.0)) * 255.0) as u8;
            let a = roundf(s.0[3].clamp(0.0, 1.0) * 255.0) as u8;
            out.push(b);
            out.push(g);
            out.push(r);
            out.push(a);
        }
        out
    }

    /// Decodes a packed `0xAARRGGBB` `u32` sRGB slice (matching `sex-image`/`sex-graphics`) into a
    /// `LinearFrame` (`Rec709`).
    pub fn from_argb_u32_srgb(
        width: usize,
        height: usize,
        argb: &[u32],
    ) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::InvalidDimensions);
        }
        let count = width.checked_mul(height).ok_or(FrameError::InvalidDimensions)?;
        if argb.len() < count {
            return Err(FrameError::BufferTooShort);
        }
        let mut pixels = Vec::with_capacity(count);
        for &p in &argb[..count] {
            let a = ((p >> 24) & 0xFF) as f32 / 255.0;
            let r = srgb_to_linear(((p >> 16) & 0xFF) as f32 / 255.0);
            let g = srgb_to_linear(((p >> 8) & 0xFF) as f32 / 255.0);
            let b = srgb_to_linear((p & 0xFF) as f32 / 255.0);
            pixels.push(RgbaF32([r, g, b, a]));
        }
        Ok(Self {
            width,
            height,
            primaries: ColorPrimaries::Rec709,
            pixels,
        })
    }

    /// Encodes this frame to `0xAARRGGBB` `u32` sRGB pixels.
    pub fn to_argb_u32_srgb(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.pixels.len());
        for &px in &self.pixels {
            let s = px.sanitize();
            let rgb = convert_primaries(s.rgb(), self.primaries, ColorPrimaries::Rec709);
            let r = roundf(linear_to_srgb(rgb[0].clamp(0.0, 1.0)) * 255.0) as u32;
            let g = roundf(linear_to_srgb(rgb[1].clamp(0.0, 1.0)) * 255.0) as u32;
            let b = roundf(linear_to_srgb(rgb[2].clamp(0.0, 1.0)) * 255.0) as u32;
            let a = roundf(s.0[3].clamp(0.0, 1.0) * 255.0) as u32;
            out.push((a << 24) | (r << 16) | (g << 8) | b);
        }
        out
    }

    /// Decodes a contiguous 8-bit NV12 buffer (`Y` followed by interleaved `UV`) into a `LinearFrame`.
    ///
    /// Supports BT.601 / BT.709 / BT.2020 matrices, limited/full range, and [`YuvRange::Auto`]
    /// clipping detection.
    pub fn from_nv12(
        width: usize,
        height: usize,
        nv12: &[u8],
        matrix: YuvMatrix,
        range: YuvRange,
        transfer: ColorSpace,
    ) -> Result<Self, FrameError> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err(FrameError::InvalidDimensions);
        }
        let y_len = width.checked_mul(height).ok_or(FrameError::InvalidDimensions)?;
        let uv_len = y_len / 2;
        let total = y_len.checked_add(uv_len).ok_or(FrameError::InvalidDimensions)?;
        if nv12.len() < total {
            return Err(FrameError::BufferTooShort);
        }
        Self::from_nv12_planes(
            width,
            height,
            &nv12[..y_len],
            width,
            &nv12[y_len..total],
            width,
            matrix,
            range,
            transfer,
        )
    }

    /// Decodes separate strided 8-bit NV12 `Y` and `UV` planes into a `LinearFrame`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_nv12_planes(
        width: usize,
        height: usize,
        y_plane: &[u8],
        y_stride: usize,
        uv_plane: &[u8],
        uv_stride: usize,
        matrix: YuvMatrix,
        range: YuvRange,
        transfer: ColorSpace,
    ) -> Result<Self, FrameError> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err(FrameError::InvalidDimensions);
        }
        let primaries = transfer
            .primaries()
            .ok_or(FrameError::UnsupportedColorSpace)?;
        if y_stride < width || uv_stride < width {
            return Err(FrameError::BufferTooShort);
        }
        let needed_y = (height - 1)
            .checked_mul(y_stride)
            .and_then(|o| o.checked_add(width))
            .ok_or(FrameError::InvalidDimensions)?;
        let needed_uv = (height / 2 - 1)
            .checked_mul(uv_stride)
            .and_then(|o| o.checked_add(width))
            .ok_or(FrameError::InvalidDimensions)?;
        if y_plane.len() < needed_y || uv_plane.len() < needed_uv {
            return Err(FrameError::BufferTooShort);
        }

        let resolved_range = match range {
            YuvRange::Auto => {
                detect_nv12_planes_range(width, height, y_plane, y_stride, uv_plane, uv_stride)
                    .detected_range
            }
            other => other,
        };

        let [kr, kg, kb] = matrix.weights();
        let cr_to_r = 2.0 * (1.0 - kr);
        let cb_to_b = 2.0 * (1.0 - kb);
        let cr_to_g = -2.0 * kr * (1.0 - kr) / kg;
        let cb_to_g = -2.0 * kb * (1.0 - kb) / kg;

        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..height {
            let y_row = &y_plane[y * y_stride..y * y_stride + width];
            let uv_row = &uv_plane[(y / 2) * uv_stride..(y / 2) * uv_stride + width];
            for x in 0..width {
                let y_raw = y_row[x] as f32;
                let uv_idx = (x / 2) * 2;
                let u_raw = uv_row[uv_idx] as f32;
                let v_raw = uv_row[uv_idx + 1] as f32;

                let (y_norm, cb, cr) = match resolved_range {
                    YuvRange::Limited | YuvRange::Auto => (
                        ((y_raw - 16.0) / 219.0).clamp(0.0, 1.0),
                        (u_raw - 128.0) / 224.0,
                        (v_raw - 128.0) / 224.0,
                    ),
                    YuvRange::Full => (
                        (y_raw / 255.0).clamp(0.0, 1.0),
                        (u_raw - 128.0) / 255.0,
                        (v_raw - 128.0) / 255.0,
                    ),
                };

                let rp = (y_norm + cr_to_r * cr).clamp(0.0, 1.0);
                let gp = (y_norm + cr_to_g * cr + cb_to_g * cb).clamp(0.0, 1.0);
                let bp = (y_norm + cb_to_b * cb).clamp(0.0, 1.0);

                let lin = decode_transfer_rgb([rp, gp, bp], transfer)?;
                pixels.push(RgbaF32::from_rgb(lin, 1.0));
            }
        }

        Ok(Self {
            width,
            height,
            primaries,
            pixels,
        })
    }

    /// Encodes this linear frame to packed 8-bit NV12 bytes (`Y` plane followed by interleaved `UV`).
    ///
    /// Automatically converts pixel primaries to match `transfer`'s primaries before applying the
    /// OETF and Y'CbCr matrix. Note [`YuvRange::Auto`] resolves to [`YuvRange::Limited`] on encode.
    pub fn to_nv12(
        &self,
        matrix: YuvMatrix,
        range: YuvRange,
        transfer: ColorSpace,
    ) -> Result<Vec<u8>, FrameError> {
        if self.width == 0 || self.height == 0 || self.width % 2 != 0 || self.height % 2 != 0 {
            return Err(FrameError::InvalidDimensions);
        }
        let target_primaries = transfer
            .primaries()
            .ok_or(FrameError::UnsupportedColorSpace)?;
        let y_len = self.width * self.height;
        let uv_len = y_len / 2;
        let mut out = vec![0u8; y_len + uv_len];

        let [kr, kg, kb] = matrix.weights();
        let inv_cb = 1.0 / (2.0 * (1.0 - kb));
        let inv_cr = 1.0 / (2.0 * (1.0 - kr));
        let is_full = matches!(range, YuvRange::Full);

        // Pre-encode all pixels to R'G'B' once so chroma 2x2 subsampling avoids redundant EOTF work.
        let mut enc_rgb = Vec::with_capacity(self.pixels.len());
        for &px in &self.pixels {
            let s = px.sanitize();
            let rgb = convert_primaries(s.rgb(), self.primaries, target_primaries);
            let encoded = encode_transfer_rgb(rgb, transfer)?;
            enc_rgb.push([
                encoded[0].clamp(0.0, 1.0),
                encoded[1].clamp(0.0, 1.0),
                encoded[2].clamp(0.0, 1.0),
            ]);
        }

        for (i, rgb) in enc_rgb.iter().enumerate() {
            let y_norm = kr * rgb[0] + kg * rgb[1] + kb * rgb[2];
            out[i] = if is_full {
                roundf(y_norm * 255.0).clamp(0.0, 255.0) as u8
            } else {
                roundf(16.0 + 219.0 * y_norm).clamp(16.0, 235.0) as u8
            };
        }

        let half_w = self.width / 2;
        let half_h = self.height / 2;
        for cy in 0..half_h {
            let r0 = cy * 2 * self.width;
            let r1 = r0 + self.width;
            for cx in 0..half_w {
                let x0 = cx * 2;
                let p00 = enc_rgb[r0 + x0];
                let p01 = enc_rgb[r0 + x0 + 1];
                let p10 = enc_rgb[r1 + x0];
                let p11 = enc_rgb[r1 + x0 + 1];
                let avg = [
                    0.25 * (p00[0] + p01[0] + p10[0] + p11[0]),
                    0.25 * (p00[1] + p01[1] + p10[1] + p11[1]),
                    0.25 * (p00[2] + p01[2] + p10[2] + p11[2]),
                ];
                let y_norm = kr * avg[0] + kg * avg[1] + kb * avg[2];
                let cb = (avg[2] - y_norm) * inv_cb;
                let cr = (avg[0] - y_norm) * inv_cr;
                let uv_idx = y_len + (cy * half_w + cx) * 2;
                if is_full {
                    out[uv_idx] = roundf(128.0 + 255.0 * cb).clamp(0.0, 255.0) as u8;
                    out[uv_idx + 1] = roundf(128.0 + 255.0 * cr).clamp(0.0, 255.0) as u8;
                } else {
                    out[uv_idx] = roundf(128.0 + 224.0 * cb).clamp(16.0, 240.0) as u8;
                    out[uv_idx + 1] = roundf(128.0 + 224.0 * cr).clamp(16.0, 240.0) as u8;
                }
            }
        }

        Ok(out)
    }

    /// Decodes a little-endian 10-bit P010 byte buffer (`Y` plane followed by interleaved `UV`,
    /// 10-bit samples in the upper 10 bits of each 16-bit word) into a `LinearFrame`.
    ///
    /// Designed for 10-bit HDR10 PQ, HLG, and wide-gamut Rec.2020 ingest.
    pub fn from_p010(
        width: usize,
        height: usize,
        p010_le: &[u8],
        matrix: YuvMatrix,
        range: YuvRange,
        transfer: ColorSpace,
    ) -> Result<Self, FrameError> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err(FrameError::InvalidDimensions);
        }
        let y_words = width.checked_mul(height).ok_or(FrameError::InvalidDimensions)?;
        let uv_words = y_words / 2;
        let total_words = y_words.checked_add(uv_words).ok_or(FrameError::InvalidDimensions)?;
        let total_bytes = total_words.checked_mul(2).ok_or(FrameError::InvalidDimensions)?;
        if p010_le.len() < total_bytes {
            return Err(FrameError::BufferTooShort);
        }

        let mut y_plane = Vec::with_capacity(y_words);
        for chunk in p010_le[..y_words * 2].chunks_exact(2) {
            y_plane.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        let mut uv_plane = Vec::with_capacity(uv_words);
        for chunk in p010_le[y_words * 2..total_bytes].chunks_exact(2) {
            uv_plane.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        Self::from_p010_u16(
            width,
            height,
            &y_plane,
            width,
            &uv_plane,
            width,
            matrix,
            range,
            transfer,
        )
    }

    /// Decodes separate strided 10-bit P010 `u16` `Y` and `UV` planes (10 MSB sample format)
    /// into a `LinearFrame`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_p010_u16(
        width: usize,
        height: usize,
        y_plane: &[u16],
        y_stride: usize,
        uv_plane: &[u16],
        uv_stride: usize,
        matrix: YuvMatrix,
        range: YuvRange,
        transfer: ColorSpace,
    ) -> Result<Self, FrameError> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err(FrameError::InvalidDimensions);
        }
        let primaries = transfer
            .primaries()
            .ok_or(FrameError::UnsupportedColorSpace)?;
        if y_stride < width || uv_stride < width {
            return Err(FrameError::BufferTooShort);
        }
        let needed_y = (height - 1)
            .checked_mul(y_stride)
            .and_then(|o| o.checked_add(width))
            .ok_or(FrameError::InvalidDimensions)?;
        let needed_uv = (height / 2 - 1)
            .checked_mul(uv_stride)
            .and_then(|o| o.checked_add(width))
            .ok_or(FrameError::InvalidDimensions)?;
        if y_plane.len() < needed_y || uv_plane.len() < needed_uv {
            return Err(FrameError::BufferTooShort);
        }

        let resolved_range = match range {
            YuvRange::Auto => {
                detect_p010_planes_range(width, height, y_plane, y_stride, uv_plane, uv_stride)
                    .detected_range
            }
            other => other,
        };

        let [kr, kg, kb] = matrix.weights();
        let cr_to_r = 2.0 * (1.0 - kr);
        let cb_to_b = 2.0 * (1.0 - kb);
        let cr_to_g = -2.0 * kr * (1.0 - kr) / kg;
        let cb_to_g = -2.0 * kb * (1.0 - kb) / kg;

        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..height {
            let y_row = &y_plane[y * y_stride..y * y_stride + width];
            let uv_row = &uv_plane[(y / 2) * uv_stride..(y / 2) * uv_stride + width];
            for x in 0..width {
                let y10 = (y_row[x] >> 6) as f32;
                let uv_idx = (x / 2) * 2;
                let u10 = (uv_row[uv_idx] >> 6) as f32;
                let v10 = (uv_row[uv_idx + 1] >> 6) as f32;

                let (y_norm, cb, cr) = match resolved_range {
                    YuvRange::Limited | YuvRange::Auto => (
                        ((y10 - 64.0) / 876.0).clamp(0.0, 1.0),
                        (u10 - 512.0) / 896.0,
                        (v10 - 512.0) / 896.0,
                    ),
                    YuvRange::Full => (
                        (y10 / 1023.0).clamp(0.0, 1.0),
                        (u10 - 512.0) / 1023.0,
                        (v10 - 512.0) / 1023.0,
                    ),
                };

                let rp = (y_norm + cr_to_r * cr).clamp(0.0, 1.0);
                let gp = (y_norm + cr_to_g * cr + cb_to_g * cb).clamp(0.0, 1.0);
                let bp = (y_norm + cb_to_b * cb).clamp(0.0, 1.0);

                let lin = decode_transfer_rgb([rp, gp, bp], transfer)?;
                pixels.push(RgbaF32::from_rgb(lin, 1.0));
            }
        }

        Ok(Self {
            width,
            height,
            primaries,
            pixels,
        })
    }

    /// Encodes this frame to a little-endian 10-bit P010 byte buffer (`Y` followed by `UV`).
    pub fn to_p010(
        &self,
        matrix: YuvMatrix,
        range: YuvRange,
        transfer: ColorSpace,
    ) -> Result<Vec<u8>, FrameError> {
        if self.width == 0 || self.height == 0 || self.width % 2 != 0 || self.height % 2 != 0 {
            return Err(FrameError::InvalidDimensions);
        }
        let target_primaries = transfer
            .primaries()
            .ok_or(FrameError::UnsupportedColorSpace)?;
        let y_words = self.width * self.height;
        let uv_words = y_words / 2;
        let mut out = vec![0u8; (y_words + uv_words) * 2];

        let [kr, kg, kb] = matrix.weights();
        let inv_cb = 1.0 / (2.0 * (1.0 - kb));
        let inv_cr = 1.0 / (2.0 * (1.0 - kr));
        let is_full = matches!(range, YuvRange::Full);

        let mut enc_rgb = Vec::with_capacity(self.pixels.len());
        for &px in &self.pixels {
            let s = px.sanitize();
            let rgb = convert_primaries(s.rgb(), self.primaries, target_primaries);
            let encoded = encode_transfer_rgb(rgb, transfer)?;
            enc_rgb.push([
                encoded[0].clamp(0.0, 1.0),
                encoded[1].clamp(0.0, 1.0),
                encoded[2].clamp(0.0, 1.0),
            ]);
        }

        for (i, rgb) in enc_rgb.iter().enumerate() {
            let y_norm = kr * rgb[0] + kg * rgb[1] + kb * rgb[2];
            let y10 = if is_full {
                roundf(y_norm * 1023.0).clamp(0.0, 1023.0) as u16
            } else {
                roundf(64.0 + 876.0 * y_norm).clamp(64.0, 940.0) as u16
            };
            let bytes = (y10 << 6).to_le_bytes();
            out[i * 2] = bytes[0];
            out[i * 2 + 1] = bytes[1];
        }

        let half_w = self.width / 2;
        let half_h = self.height / 2;
        let uv_byte_base = y_words * 2;
        for cy in 0..half_h {
            let r0 = cy * 2 * self.width;
            let r1 = r0 + self.width;
            for cx in 0..half_w {
                let x0 = cx * 2;
                let p00 = enc_rgb[r0 + x0];
                let p01 = enc_rgb[r0 + x0 + 1];
                let p10 = enc_rgb[r1 + x0];
                let p11 = enc_rgb[r1 + x0 + 1];
                let avg = [
                    0.25 * (p00[0] + p01[0] + p10[0] + p11[0]),
                    0.25 * (p00[1] + p01[1] + p10[1] + p11[1]),
                    0.25 * (p00[2] + p01[2] + p10[2] + p11[2]),
                ];
                let y_norm = kr * avg[0] + kg * avg[1] + kb * avg[2];
                let cb = (avg[2] - y_norm) * inv_cb;
                let cr = (avg[0] - y_norm) * inv_cr;
                let (u10, v10) = if is_full {
                    (
                        roundf(512.0 + 1023.0 * cb).clamp(0.0, 1023.0) as u16,
                        roundf(512.0 + 1023.0 * cr).clamp(0.0, 1023.0) as u16,
                    )
                } else {
                    (
                        roundf(512.0 + 896.0 * cb).clamp(64.0, 960.0) as u16,
                        roundf(512.0 + 896.0 * cr).clamp(64.0, 960.0) as u16,
                    )
                };
                let off = uv_byte_base + (cy * half_w + cx) * 4;
                let ub = (u10 << 6).to_le_bytes();
                let vb = (v10 << 6).to_le_bytes();
                out[off] = ub[0];
                out[off + 1] = ub[1];
                out[off + 2] = vb[0];
                out[off + 3] = vb[1];
            }
        }

        Ok(out)
    }
}

fn decode_transfer_rgb(rgb: [f32; 3], transfer: ColorSpace) -> Result<[f32; 3], FrameError> {
    match transfer {
        ColorSpace::Srgb => Ok([
            srgb_to_linear(rgb[0]),
            srgb_to_linear(rgb[1]),
            srgb_to_linear(rgb[2]),
        ]),
        ColorSpace::Rec709 => Ok([
            rec709_to_linear(rgb[0]),
            rec709_to_linear(rgb[1]),
            rec709_to_linear(rgb[2]),
        ]),
        ColorSpace::Rec601 => Ok([
            rec601_to_linear(rgb[0]),
            rec601_to_linear(rgb[1]),
            rec601_to_linear(rgb[2]),
        ]),
        ColorSpace::Rec2020 => Ok([
            rec2020_to_linear(rgb[0]),
            rec2020_to_linear(rgb[1]),
            rec2020_to_linear(rgb[2]),
        ]),
        ColorSpace::LinearRec709 | ColorSpace::LinearRec2020 => Ok(rgb),
        ColorSpace::Hdr10Pq => Ok([
            pq_eotf_nits(rgb[0]) / REF_WHITE_NITS,
            pq_eotf_nits(rgb[1]) / REF_WHITE_NITS,
            pq_eotf_nits(rgb[2]) / REF_WHITE_NITS,
        ]),
        ColorSpace::Hlg => {
            let nits = hlg_to_linear_nits(rgb, HLG_NOMINAL_PEAK_NITS);
            Ok([
                nits[0] / REF_WHITE_NITS,
                nits[1] / REF_WHITE_NITS,
                nits[2] / REF_WHITE_NITS,
            ])
        }
        ColorSpace::Oklab => Err(FrameError::UnsupportedColorSpace),
    }
}

fn encode_transfer_rgb(rgb: [f32; 3], transfer: ColorSpace) -> Result<[f32; 3], FrameError> {
    match transfer {
        ColorSpace::Srgb => Ok([
            linear_to_srgb(rgb[0].clamp(0.0, 1.0)),
            linear_to_srgb(rgb[1].clamp(0.0, 1.0)),
            linear_to_srgb(rgb[2].clamp(0.0, 1.0)),
        ]),
        ColorSpace::Rec709 => Ok([
            linear_to_rec709(rgb[0].clamp(0.0, 1.0)),
            linear_to_rec709(rgb[1].clamp(0.0, 1.0)),
            linear_to_rec709(rgb[2].clamp(0.0, 1.0)),
        ]),
        ColorSpace::Rec601 => Ok([
            linear_to_rec601(rgb[0].clamp(0.0, 1.0)),
            linear_to_rec601(rgb[1].clamp(0.0, 1.0)),
            linear_to_rec601(rgb[2].clamp(0.0, 1.0)),
        ]),
        ColorSpace::Rec2020 => Ok([
            linear_to_rec2020(rgb[0].clamp(0.0, 1.0)),
            linear_to_rec2020(rgb[1].clamp(0.0, 1.0)),
            linear_to_rec2020(rgb[2].clamp(0.0, 1.0)),
        ]),
        ColorSpace::LinearRec709 | ColorSpace::LinearRec2020 => Ok(rgb),
        ColorSpace::Hdr10Pq => Ok([
            pq_oetf_nits(rgb[0].max(0.0) * REF_WHITE_NITS),
            pq_oetf_nits(rgb[1].max(0.0) * REF_WHITE_NITS),
            pq_oetf_nits(rgb[2].max(0.0) * REF_WHITE_NITS),
        ]),
        ColorSpace::Hlg => Ok(linear_nits_to_hlg(
            [
                rgb[0].max(0.0) * REF_WHITE_NITS,
                rgb[1].max(0.0) * REF_WHITE_NITS,
                rgb[2].max(0.0) * REF_WHITE_NITS,
            ],
            HLG_NOMINAL_PEAK_NITS,
        )),
        ColorSpace::Oklab => Err(FrameError::UnsupportedColorSpace),
    }
}
