//! All 19 OmaBroadcast video filters plus HDR tone mapping and SDR-on-HDR conversion.
//!
//! Up to [`MAX_FILTERS_PER_SOURCE`] (32) filters can be stacked per source via [`FilterChain`].

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::f32::consts::PI;

use crate::compositor::{scale_frame, FrameBuffer, LinearRgba, Scaler};
use crate::math::{
    absf, clamp01, cosf, expf, floorf, fract, lerp, powf, rec709_luma, rgb_to_ycbcr_709, sinf,
    smoothstep, sqrtf,
};

/// Maximum number of video filters allowed on a single source.
pub const MAX_FILTERS_PER_SOURCE: usize = 32;

/// Errors returned during filter chain management or evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterError {
    TooManyFilters,
    IndexOutOfBounds,
    InvalidLutSize(u8),
    LutEntryCountMismatch { expected: usize, actual: usize },
    InvalidCubeSyntax,
    InvalidDimensions,
}

/// Deinterlacing algorithm variants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DeinterlaceMode {
    Discard,
    Linear,
    Blend,
    #[default]
    MotionAdaptive,
}

/// Color wheel lift/gamma/gain/offset tuple (`r, g, b, master`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorWheel {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub master: f32,
}

impl ColorWheel {
    pub const ZERO: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        master: 0.0,
    };

    pub const ONE: Self = Self {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        master: 1.0,
    };
}

/// Blending mode for `ImageMaskBlend`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MaskBlendMode {
    #[default]
    AlphaMaskAlphaChannel,
    AlphaMaskColorChannel,
    ColorMultiply,
    ColorAdd,
    ColorSubtract,
}

/// Aspect handling for `ScaleAspect`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScaleAspectMode {
    Stretch,
    #[default]
    FitLetterbox,
    CoverCrop,
}

/// Blur algorithm variants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlurKind {
    Box,
    #[default]
    Gaussian,
    DualKawase,
}

/// AI background matte output mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AiMatteMode {
    #[default]
    Cutout,
    BackgroundBlur,
    DesaturateBackground,
}

/// Tone-mapping curve for `HdrToneMap`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToneMapMethod {
    Reinhard,
    HableFilmic,
    #[default]
    AcesFitted,
    Bt2390Eetf,
}

/// Parsed 3D Look-Up Table for `ApplyLut`.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut3D {
    pub size: u8,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    pub table: Vec<[f32; 3]>,
}

impl Lut3D {
    /// Builds an identity 3D LUT of `size^3` entries.
    pub fn identity(size: u8) -> Result<Self, FilterError> {
        if !(2..=64).contains(&size) {
            return Err(FilterError::InvalidLutSize(size));
        }
        let n = size as usize;
        let mut table = Vec::with_capacity(n * n * n);
        let denom = (n - 1) as f32;
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    table.push([r as f32 / denom, g as f32 / denom, b as f32 / denom]);
                }
            }
        }
        Ok(Self {
            size,
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            table,
        })
    }

    /// Parses an Adobe `.cube` 3D LUT text asset.
    pub fn from_cube_str(text: &str) -> Result<Self, FilterError> {
        let mut size: Option<u8> = None;
        let mut domain_min = [0.0_f32; 3];
        let mut domain_max = [1.0_f32; 3];
        let mut table = Vec::new();

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("LUT_3D_SIZE") {
                let s: u8 = rest
                    .trim()
                    .parse()
                    .map_err(|_| FilterError::InvalidCubeSyntax)?;
                if !(2..=64).contains(&s) {
                    return Err(FilterError::InvalidLutSize(s));
                }
                size = Some(s);
                continue;
            }
            if let Some(rest) = line.strip_prefix("DOMAIN_MIN") {
                domain_min = parse_vec3(rest)?;
                continue;
            }
            if let Some(rest) = line.strip_prefix("DOMAIN_MAX") {
                domain_max = parse_vec3(rest)?;
                continue;
            }
            if line.starts_with("TITLE") || line.starts_with("LUT_1D_SIZE") {
                continue;
            }
            table.push(parse_vec3(line)?);
        }

        let size = size.ok_or(FilterError::InvalidCubeSyntax)?;
        let n = size as usize;
        let expected = n * n * n;
        if table.len() != expected {
            return Err(FilterError::LutEntryCountMismatch {
                expected,
                actual: table.len(),
            });
        }
        Ok(Self {
            size,
            domain_min,
            domain_max,
            table,
        })
    }

    /// Evaluates the 3D LUT with trilinear interpolation.
    pub fn sample_trilinear(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size as usize;
        if n < 2 || self.table.len() != n * n * n {
            return rgb;
        }
        let max_idx = (n - 1) as f32;
        let mut coords = [0.0_f32; 3];
        for c in 0..3 {
            let span = (self.domain_max[c] - self.domain_min[c]).max(1e-6);
            let norm = clamp01((rgb[c] - self.domain_min[c]) / span);
            coords[c] = norm * max_idx;
        }

        let r0 = (floorf(coords[0]) as usize).min(n - 2);
        let g0 = (floorf(coords[1]) as usize).min(n - 2);
        let b0 = (floorf(coords[2]) as usize).min(n - 2);
        let fr = coords[0] - r0 as f32;
        let fg = coords[1] - g0 as f32;
        let fb = coords[2] - b0 as f32;

        let at = |r: usize, g: usize, b: usize| -> [f32; 3] {
            self.table[(b * n + g) * n + r]
        };

        let c000 = at(r0, g0, b0);
        let c100 = at(r0 + 1, g0, b0);
        let c010 = at(r0, g0 + 1, b0);
        let c110 = at(r0 + 1, g0 + 1, b0);
        let c001 = at(r0, g0, b0 + 1);
        let c101 = at(r0 + 1, g0, b0 + 1);
        let c011 = at(r0, g0 + 1, b0 + 1);
        let c111 = at(r0 + 1, g0 + 1, b0 + 1);

        let mut out = [0.0_f32; 3];
        for c in 0..3 {
            let c00 = lerp(c000[c], c100[c], fr);
            let c10 = lerp(c010[c], c110[c], fr);
            let c01 = lerp(c001[c], c101[c], fr);
            let c11 = lerp(c011[c], c111[c], fr);
            let c0 = lerp(c00, c10, fg);
            let c1 = lerp(c01, c11, fg);
            out[c] = lerp(c0, c1, fb);
        }
        out
    }
}

fn parse_vec3(s: &str) -> Result<[f32; 3], FilterError> {
    let mut parts = s.split_whitespace();
    let x: f32 = parts
        .next()
        .ok_or(FilterError::InvalidCubeSyntax)?
        .parse()
        .map_err(|_| FilterError::InvalidCubeSyntax)?;
    let y: f32 = parts
        .next()
        .ok_or(FilterError::InvalidCubeSyntax)?
        .parse()
        .map_err(|_| FilterError::InvalidCubeSyntax)?;
    let z: f32 = parts
        .next()
        .ok_or(FilterError::InvalidCubeSyntax)?
        .parse()
        .map_err(|_| FilterError::InvalidCubeSyntax)?;
    Ok([x, y, z])
}

/// All 19 OmaBroadcast video filters plus HDR tone mapping and SDR-on-HDR.
#[derive(Clone, Debug, PartialEq)]
pub enum VideoFilter {
    /// (1) Deinterlace scanline filter.
    Deinterlace {
        mode: DeinterlaceMode,
        top_field_first: bool,
    },
    /// (2) Temporal + edge-preserving guided chroma denoiser.
    Denoise {
        temporal_strength: f32,
        chroma_strength: f32,
        radius: u8,
    },
    /// (3) 4-way lift/gamma/gain/offset color wheels + saturation.
    ColorWheels4Way {
        lift: ColorWheel,
        gamma: ColorWheel,
        gain: ColorWheel,
        offset: ColorWheel,
        saturation: f32,
    },
    /// (4) Legacy OBS color correction filter.
    LegacyColor {
        brightness: f32,
        contrast: f32,
        gamma: f32,
        saturation: f32,
        hue_shift_deg: f32,
        opacity: f32,
    },
    /// (5) Unsharp-mask convolution sharpening.
    Sharpen {
        amount: f32,
    },
    /// (6) YCbCr chroma key with automatic backdrop color picker and spill suppression.
    ChromaKey {
        key_color_rgb: [f32; 3],
        auto_backdrop_pick: bool,
        similarity: f32,
        smoothness: f32,
        spill_suppression: f32,
    },
    /// (7) 3D RGB distance color key.
    ColorKey {
        key_color_rgb: [f32; 3],
        similarity: f32,
        smoothness: f32,
    },
    /// (8) Luminance key.
    LumaKey {
        luma_max: f32,
        luma_max_smooth: f32,
        luma_min: f32,
        luma_min_smooth: f32,
        invert: bool,
    },
    /// (9) 3D LUT color grading filter.
    ApplyLut {
        path: String,
        lut: Lut3D,
        intensity: f32,
    },
    /// (10) Image mask / blend compositing filter.
    ImageMaskBlend {
        mode: MaskBlendMode,
        mask: Option<FrameBuffer>,
        color: LinearRgba,
        stretch: bool,
    },
    /// (11) Pixel crop (>0) or pad (<0) filter.
    CropPad {
        top: i32,
        right: i32,
        bottom: i32,
        left: i32,
        pad_color: LinearRgba,
    },
    /// (12) Aspect-ratio scaling filter.
    ScaleAspect {
        target_width: u32,
        target_height: u32,
        scaler: Scaler,
        aspect_mode: ScaleAspectMode,
    },
    /// (13) Continuous UV scroll filter.
    Scroll {
        speed_x_px_s: f32,
        speed_y_px_s: f32,
        loop_wrap: bool,
        limit_width: Option<u32>,
        limit_height: Option<u32>,
    },
    /// (14) Separable Box / Gaussian / Dual-Kawase blur.
    Blur {
        kind: BlurKind,
        radius: f32,
        passes: u8,
    },
    /// (15) Alpha drop shadow filter.
    DropShadow {
        offset_x: f32,
        offset_y: f32,
        blur_radius: f32,
        color: LinearRgba,
        spread: f32,
    },
    /// (16) Bounded render-output delay ring.
    RenderDelay {
        delay_ms: u32,
    },
    /// (17) Bounded raw video frame ring for A/V sync.
    VideoDelay {
        delay_ms: u32,
        max_frames: usize,
    },
    /// (18) AI background segmentation with guided hair-edge refinement,
    /// temporal anti-shimmer, and fail-closed hide-on-error behavior.
    AiBackgroundMatte {
        mode: AiMatteMode,
        blur_radius: f32,
        feather: f32,
        temporal_smoothing: f32,
        edge_refine_radius: u8,
        model_id: String,
    },
    /// (19) Wraps live composited background luminance/color onto keyed foreground alpha edges.
    LightWrap {
        intensity: f32,
        radius: f32,
        luma_threshold: f32,
        soft_bleed: f32,
    },
    /// HDR scene-referred to SDR tone mapper.
    HdrToneMap {
        method: ToneMapMethod,
        source_peak_nits: f32,
        target_peak_nits: f32,
        desaturate_highlights: f32,
    },
    /// SDR content placement onto HDR canvas.
    SdrOnHdr {
        sdr_white_nits: f32,
        paper_white_scale: f32,
        expand_highlights: f32,
    },
}

/// Entry inside a source's [`FilterChain`].
#[derive(Clone, Debug, PartialEq)]
pub struct FilterEntry {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    pub filter: VideoFilter,
}

/// Ordered stack of up to 32 video filters attached to a source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterChain {
    pub entries: Vec<FilterEntry>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn add(&mut self, entry: FilterEntry) -> Result<(), FilterError> {
        if self.entries.len() >= MAX_FILTERS_PER_SOURCE {
            return Err(FilterError::TooManyFilters);
        }
        self.entries.push(entry);
        Ok(())
    }

    pub fn remove(&mut self, index: usize) -> Result<FilterEntry, FilterError> {
        if index >= self.entries.len() {
            return Err(FilterError::IndexOutOfBounds);
        }
        Ok(self.entries.remove(index))
    }

    pub fn move_filter(&mut self, from: usize, to: usize) -> Result<(), FilterError> {
        if from >= self.entries.len() || to >= self.entries.len() {
            return Err(FilterError::IndexOutOfBounds);
        }
        let item = self.entries.remove(from);
        self.entries.insert(to, item);
        Ok(())
    }

    pub fn set_enabled(&mut self, index: usize, enabled: bool) -> Result<(), FilterError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(FilterError::IndexOutOfBounds)?;
        entry.enabled = enabled;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Input segmentation prediction supplied to [`VideoFilter::AiBackgroundMatte`].
#[derive(Clone, Copy, Debug)]
pub enum AiMatteInput<'a> {
    /// Inference succeeded with per-pixel foreground alpha probability `[0.0..=1.0]`.
    Success(&'a [f32]),
    /// Inference failed or timed out — filter MUST fail closed (hide source completely).
    InferenceFailed,
    /// No model result is available — filter MUST fail closed (hide source completely).
    Unavailable,
}

/// Status emitted after evaluating `AiBackgroundMatte`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiMatteStatus {
    NotRun,
    Applied,
    FailedClosedHidden,
}

/// Per-frame runtime inputs & persistent temporal state for evaluating a `FilterChain`.
#[derive(Clone, Debug, Default)]
pub struct FilterContext {
    pub timestamp_us: i64,
    pub frame_duration_ms: f32,
    pub prev_frame: Option<FrameBuffer>,
    pub prev_matte_alpha: Option<Vec<f32>>,
    pub render_delay_ring: VecDeque<FrameBuffer>,
    pub video_delay_ring: VecDeque<FrameBuffer>,
    pub ai_matte_status: AiMatteStatus,
    pub last_picked_chroma: Option<[f32; 3]>,
}

impl Default for AiMatteStatus {
    fn default() -> Self {
        Self::NotRun
    }
}

impl FilterContext {
    pub fn new() -> Self {
        Self {
            timestamp_us: 0,
            frame_duration_ms: 16.666_666,
            prev_frame: None,
            prev_matte_alpha: None,
            render_delay_ring: VecDeque::new(),
            video_delay_ring: VecDeque::new(),
            ai_matte_status: AiMatteStatus::NotRun,
            last_picked_chroma: None,
        }
    }
}

/// Evaluates an entire [`FilterChain`] over `input` with stateful temporal buffers,
/// optional AI matte inference output, and optional live composited background for `LightWrap`.
pub fn process_filter_chain(
    chain: &FilterChain,
    input: &FrameBuffer,
    ctx: &mut FilterContext,
    ai_input: AiMatteInput<'_>,
    composited_bg: Option<&FrameBuffer>,
) -> Result<FrameBuffer, FilterError> {
    let mut current = input.clone();
    ctx.ai_matte_status = AiMatteStatus::NotRun;

    for entry in &chain.entries {
        if !entry.enabled {
            continue;
        }
        current = apply_video_filter(&entry.filter, &current, ctx, ai_input, composited_bg)?;
    }

    ctx.prev_frame = Some(input.clone());
    Ok(current)
}

/// Evaluates a single [`VideoFilter`] onto `input`.
pub fn apply_video_filter(
    filter: &VideoFilter,
    input: &FrameBuffer,
    ctx: &mut FilterContext,
    ai_input: AiMatteInput<'_>,
    composited_bg: Option<&FrameBuffer>,
) -> Result<FrameBuffer, FilterError> {
    match filter {
        VideoFilter::Deinterlace {
            mode,
            top_field_first,
        } => Ok(apply_deinterlace(
            input,
            ctx.prev_frame.as_ref(),
            *mode,
            *top_field_first,
        )),
        VideoFilter::Denoise {
            temporal_strength,
            chroma_strength,
            radius,
        } => Ok(apply_denoise(
            input,
            ctx.prev_frame.as_ref(),
            *temporal_strength,
            *chroma_strength,
            *radius,
        )),
        VideoFilter::ColorWheels4Way {
            lift,
            gamma,
            gain,
            offset,
            saturation,
        } => Ok(apply_color_wheels(
            input,
            *lift,
            *gamma,
            *gain,
            *offset,
            *saturation,
        )),
        VideoFilter::LegacyColor {
            brightness,
            contrast,
            gamma,
            saturation,
            hue_shift_deg,
            opacity,
        } => Ok(apply_legacy_color(
            input,
            *brightness,
            *contrast,
            *gamma,
            *saturation,
            *hue_shift_deg,
            *opacity,
        )),
        VideoFilter::Sharpen { amount } => Ok(apply_sharpen(input, *amount)),
        VideoFilter::ChromaKey {
            key_color_rgb,
            auto_backdrop_pick,
            similarity,
            smoothness,
            spill_suppression,
        } => {
            let key = if *auto_backdrop_pick {
                let picked = auto_pick_backdrop_color(input);
                ctx.last_picked_chroma = Some(picked);
                picked
            } else {
                *key_color_rgb
            };
            Ok(apply_chroma_key(
                input,
                key,
                *similarity,
                *smoothness,
                *spill_suppression,
            ))
        }
        VideoFilter::ColorKey {
            key_color_rgb,
            similarity,
            smoothness,
        } => Ok(apply_color_key(
            input,
            *key_color_rgb,
            *similarity,
            *smoothness,
        )),
        VideoFilter::LumaKey {
            luma_max,
            luma_max_smooth,
            luma_min,
            luma_min_smooth,
            invert,
        } => Ok(apply_luma_key(
            input,
            *luma_max,
            *luma_max_smooth,
            *luma_min,
            *luma_min_smooth,
            *invert,
        )),
        VideoFilter::ApplyLut { lut, intensity, .. } => Ok(apply_lut(input, lut, *intensity)),
        VideoFilter::ImageMaskBlend {
            mode,
            mask,
            color,
            stretch,
        } => Ok(apply_image_mask_blend(
            input,
            *mode,
            mask.as_ref(),
            *color,
            *stretch,
        )),
        VideoFilter::CropPad {
            top,
            right,
            bottom,
            left,
            pad_color,
        } => apply_crop_pad(input, *top, *right, *bottom, *left, *pad_color),
        VideoFilter::ScaleAspect {
            target_width,
            target_height,
            scaler,
            aspect_mode,
        } => apply_scale_aspect(
            input,
            *target_width,
            *target_height,
            *scaler,
            *aspect_mode,
        ),
        VideoFilter::Scroll {
            speed_x_px_s,
            speed_y_px_s,
            loop_wrap,
            limit_width,
            limit_height,
        } => apply_scroll(
            input,
            ctx.timestamp_us,
            *speed_x_px_s,
            *speed_y_px_s,
            *loop_wrap,
            *limit_width,
            *limit_height,
        ),
        VideoFilter::Blur {
            kind,
            radius,
            passes,
        } => Ok(apply_blur(input, *kind, *radius, *passes)),
        VideoFilter::DropShadow {
            offset_x,
            offset_y,
            blur_radius,
            color,
            spread,
        } => Ok(apply_drop_shadow(
            input,
            *offset_x,
            *offset_y,
            *blur_radius,
            *color,
            *spread,
        )),
        VideoFilter::RenderDelay { delay_ms } => {
            let frame_ms = ctx.frame_duration_ms.max(1.0);
            let target_len = ((*delay_ms as f32 / frame_ms) + 0.5) as usize;
            let cap = target_len.min(60);
            ctx.render_delay_ring.push_back(input.clone());
            if ctx.render_delay_ring.len() > cap {
                Ok(ctx.render_delay_ring.pop_front().unwrap_or_else(|| input.clone()))
            } else {
                Ok(FrameBuffer::new(input.width, input.height)
                    .map_err(|_| FilterError::InvalidDimensions)?)
            }
        }
        VideoFilter::VideoDelay {
            delay_ms,
            max_frames,
        } => {
            let frame_ms = ctx.frame_duration_ms.max(1.0);
            let target_len = (((*delay_ms as f32 / frame_ms) + 0.5) as usize).min(*max_frames);
            ctx.video_delay_ring.push_back(input.clone());
            while ctx.video_delay_ring.len() > (*max_frames).max(1) {
                ctx.video_delay_ring.pop_front();
            }
            if target_len == 0 {
                Ok(ctx.video_delay_ring.pop_back().unwrap_or_else(|| input.clone()))
            } else if ctx.video_delay_ring.len() > target_len {
                Ok(ctx.video_delay_ring.pop_front().unwrap_or_else(|| input.clone()))
            } else {
                Ok(FrameBuffer::new(input.width, input.height)
                    .map_err(|_| FilterError::InvalidDimensions)?)
            }
        }
        VideoFilter::AiBackgroundMatte {
            mode,
            blur_radius,
            feather,
            temporal_smoothing,
            edge_refine_radius,
            ..
        } => Ok(apply_ai_background_matte(
            input,
            ai_input,
            ctx,
            *mode,
            *blur_radius,
            *feather,
            *temporal_smoothing,
            *edge_refine_radius,
        )),
        VideoFilter::LightWrap {
            intensity,
            radius,
            luma_threshold,
            soft_bleed,
        } => Ok(apply_light_wrap(
            input,
            composited_bg,
            *intensity,
            *radius,
            *luma_threshold,
            *soft_bleed,
        )),
        VideoFilter::HdrToneMap {
            method,
            source_peak_nits,
            target_peak_nits,
            desaturate_highlights,
        } => Ok(apply_hdr_tonemap(
            input,
            *method,
            *source_peak_nits,
            *target_peak_nits,
            *desaturate_highlights,
        )),
        VideoFilter::SdrOnHdr {
            sdr_white_nits,
            paper_white_scale,
            expand_highlights,
        } => Ok(apply_sdr_on_hdr(
            input,
            *sdr_white_nits,
            *paper_white_scale,
            *expand_highlights,
        )),
    }
}

/// Automatically detects the dominant backdrop chroma key color from image borders.
pub fn auto_pick_backdrop_color(frame: &FrameBuffer) -> [f32; 3] {
    let w = frame.width as usize;
    let h = frame.height as usize;
    if w == 0 || h == 0 {
        return [0.0, 0.7, 0.0];
    }

    const BINS: usize = 16;
    let mut counts = [0u32; BINS * BINS];
    let mut sums = [[0.0_f32; 3]; BINS * BINS];

    let border_x = (w / 10).max(1);
    let border_y = (h / 10).max(1);

    for y in 0..h {
        for x in 0..w {
            let on_border = x < border_x || x >= w - border_x || y < border_y || y >= h - border_y;
            if !on_border {
                continue;
            }
            let p = frame.pixels[y * w + x];
            if p[3] < 0.2 {
                continue;
            }
            let ycc = rgb_to_ycbcr_709([p[0], p[1], p[2]]);
            let chroma_mag = sqrtf(ycc[1] * ycc[1] + ycc[2] * ycc[2]);
            if chroma_mag < 0.04 {
                continue;
            }
            let bx = (((ycc[1] + 0.5) * (BINS as f32 - 0.001)) as usize).min(BINS - 1);
            let by = (((ycc[2] + 0.5) * (BINS as f32 - 0.001)) as usize).min(BINS - 1);
            let bin = by * BINS + bx;
            counts[bin] += 1;
            sums[bin][0] += p[0];
            sums[bin][1] += p[1];
            sums[bin][2] += p[2];
        }
    }

    let mut best_bin = 0;
    let mut best_count = 0;
    for (idx, &cnt) in counts.iter().enumerate() {
        if cnt > best_count {
            best_count = cnt;
            best_bin = idx;
        }
    }

    if best_count == 0 {
        // Fallback: average top-left and top-right corners.
        let tl = frame.pixels[0];
        let tr = frame.pixels[w - 1];
        [
            0.5 * (tl[0] + tr[0]),
            0.5 * (tl[1] + tr[1]),
            0.5 * (tl[2] + tr[2]),
        ]
    } else {
        let inv = 1.0 / best_count as f32;
        [
            sums[best_bin][0] * inv,
            sums[best_bin][1] * inv,
            sums[best_bin][2] * inv,
        ]
    }
}

fn apply_deinterlace(
    frame: &FrameBuffer,
    prev: Option<&FrameBuffer>,
    mode: DeinterlaceMode,
    top_field_first: bool,
) -> FrameBuffer {
    let mut out = frame.clone();
    let w = frame.width;
    let h = frame.height;
    let keep_parity = if top_field_first { 0 } else { 1 };

    for y in 0..h {
        let is_kept = (y % 2) == keep_parity;
        for x in 0..w {
            let idx = (y as usize) * (w as usize) + (x as usize);
            match mode {
                DeinterlaceMode::Discard => {
                    if !is_kept {
                        let ref_y = if top_field_first {
                            y.saturating_sub(1)
                        } else {
                            (y + 1).min(h - 1)
                        };
                        out.pixels[idx] = frame.pixels[(ref_y as usize) * (w as usize) + (x as usize)];
                    }
                }
                DeinterlaceMode::Linear => {
                    if !is_kept {
                        let a = frame.sample_clamped(x as i32, y as i32 - 1);
                        let b = frame.sample_clamped(x as i32, y as i32 + 1);
                        out.pixels[idx] = [
                            0.5 * (a[0] + b[0]),
                            0.5 * (a[1] + b[1]),
                            0.5 * (a[2] + b[2]),
                            0.5 * (a[3] + b[3]),
                        ];
                    }
                }
                DeinterlaceMode::Blend => {
                    let prev_line = frame.sample_clamped(x as i32, y as i32 - 1);
                    let curr_line = frame.pixels[idx];
                    let next_line = frame.sample_clamped(x as i32, y as i32 + 1);
                    out.pixels[idx] = [
                        0.25 * prev_line[0] + 0.5 * curr_line[0] + 0.25 * next_line[0],
                        0.25 * prev_line[1] + 0.5 * curr_line[1] + 0.25 * next_line[1],
                        0.25 * prev_line[2] + 0.5 * curr_line[2] + 0.25 * next_line[2],
                        0.25 * prev_line[3] + 0.5 * curr_line[3] + 0.25 * next_line[3],
                    ];
                }
                DeinterlaceMode::MotionAdaptive => {
                    if !is_kept {
                        let curr = frame.pixels[idx];
                        let spatial = {
                            let a = frame.sample_clamped(x as i32, y as i32 - 1);
                            let b = frame.sample_clamped(x as i32, y as i32 + 1);
                            [
                                0.5 * (a[0] + b[0]),
                                0.5 * (a[1] + b[1]),
                                0.5 * (a[2] + b[2]),
                                0.5 * (a[3] + b[3]),
                            ]
                        };
                        if let Some(pf) = prev {
                            if pf.width == w && pf.height == h {
                                let p = pf.pixels[idx];
                                let motion = absf(curr[0] - p[0])
                                    .max(absf(curr[1] - p[1]))
                                    .max(absf(curr[2] - p[2]));
                                let m = smoothstep(0.02, 0.12, motion);
                                out.pixels[idx] = [
                                    lerp(curr[0], spatial[0], m),
                                    lerp(curr[1], spatial[1], m),
                                    lerp(curr[2], spatial[2], m),
                                    lerp(curr[3], spatial[3], m),
                                ];
                                continue;
                            }
                        }
                        out.pixels[idx] = spatial;
                    }
                }
            }
        }
    }
    out
}

fn apply_denoise(
    frame: &FrameBuffer,
    prev: Option<&FrameBuffer>,
    temporal_strength: f32,
    chroma_strength: f32,
    radius: u8,
) -> FrameBuffer {
    let mut out = frame.clone();
    let w = frame.width as i32;
    let h = frame.height as i32;
    let r = (radius as i32).clamp(1, 4);
    let temp_s = clamp01(temporal_strength);
    let chroma_s = clamp01(chroma_strength);

    // Spatial guided chroma smoothing while preserving luma detail.
    for y in 0..h {
        for x in 0..w {
            let center = frame.sample_clamped(x, y);
            let c_luma = rec709_luma([center[0], center[1], center[2]]);
            let mut sum_cb = 0.0_f32;
            let mut sum_cr = 0.0_f32;
            let mut wsum = 0.0_f32;

            for dy in -r..=r {
                for dx in -r..=r {
                    let s = frame.sample_clamped(x + dx, y + dy);
                    let ycc = rgb_to_ycbcr_709([s[0], s[1], s[2]]);
                    let dl = ycc[0] - c_luma;
                    let weight = expf(-(dl * dl) / 0.005);
                    sum_cb += ycc[1] * weight;
                    sum_cr += ycc[2] * weight;
                    wsum += weight;
                }
            }

            let center_ycc = rgb_to_ycbcr_709([center[0], center[1], center[2]]);
            let filt_cb = if wsum > 1e-6 { sum_cb / wsum } else { center_ycc[1] };
            let filt_cr = if wsum > 1e-6 { sum_cr / wsum } else { center_ycc[2] };

            let cb = lerp(center_ycc[1], filt_cb, chroma_s);
            let cr = lerp(center_ycc[2], filt_cr, chroma_s);
            let mut rgb = ycbcr_709_to_rgb([center_ycc[0], cb, cr]);

            // Motion-gated temporal filter.
            if temp_s > 0.0 {
                if let Some(pf) = prev {
                    if pf.width == frame.width && pf.height == frame.height {
                        let p = pf.pixels[(y as usize) * (w as usize) + (x as usize)];
                        let diff = absf(rgb[0] - p[0])
                            .max(absf(rgb[1] - p[1]))
                            .max(absf(rgb[2] - p[2]));
                        let motion_gate = 1.0 - smoothstep(0.03, 0.15, diff);
                        let blend = temp_s * motion_gate;
                        rgb = [
                            lerp(rgb[0], p[0], blend),
                            lerp(rgb[1], p[1], blend),
                            lerp(rgb[2], p[2], blend),
                        ];
                    }
                }
            }

            let idx = (y as usize) * (w as usize) + (x as usize);
            out.pixels[idx] = [rgb[0].max(0.0), rgb[1].max(0.0), rgb[2].max(0.0), center[3]];
        }
    }
    out
}

fn apply_color_wheels(
    frame: &FrameBuffer,
    lift: ColorWheel,
    gamma: ColorWheel,
    gain: ColorWheel,
    offset: ColorWheel,
    saturation: f32,
) -> FrameBuffer {
    let mut out = frame.clone();
    let lift_c = [
        lift.r + lift.master,
        lift.g + lift.master,
        lift.b + lift.master,
    ];
    let gamma_c = [
        (gamma.r + gamma.master).max(0.05),
        (gamma.g + gamma.master).max(0.05),
        (gamma.b + gamma.master).max(0.05),
    ];
    let gain_c = [
        (gain.r + gain.master).max(0.0),
        (gain.g + gain.master).max(0.0),
        (gain.b + gain.master).max(0.0),
    ];
    let offset_c = [
        offset.r + offset.master,
        offset.g + offset.master,
        offset.b + offset.master,
    ];

    for p in &mut out.pixels {
        let mut rgb = [p[0], p[1], p[2]];
        for c in 0..3 {
            // Gain * (x + lift*(1-x)) + offset, then gamma.
            let v = (rgb[c] + lift_c[c] * (1.0 - rgb[c])) * gain_c[c] + offset_c[c];
            let v = v.max(0.0);
            rgb[c] = powf(v, 1.0 / gamma_c[c]);
        }
        let luma = rec709_luma(rgb);
        for c in 0..3 {
            rgb[c] = (luma + (rgb[c] - luma) * saturation).max(0.0);
        }
        p[0] = rgb[0];
        p[1] = rgb[1];
        p[2] = rgb[2];
    }
    out
}

fn apply_legacy_color(
    frame: &FrameBuffer,
    brightness: f32,
    contrast: f32,
    gamma: f32,
    saturation: f32,
    hue_shift_deg: f32,
    opacity: f32,
) -> FrameBuffer {
    let mut out = frame.clone();
    let inv_gamma = 1.0 / gamma.max(0.05);
    let rad = hue_shift_deg * (PI / 180.0);
    let cos_h = cosf(rad);
    let sin_h = sinf(rad);
    let k = 1.0 / 3.0;
    let sqrt_k = sqrtf(k);

    for p in &mut out.pixels {
        let mut rgb = [
            powf(p[0].max(0.0), inv_gamma),
            powf(p[1].max(0.0), inv_gamma),
            powf(p[2].max(0.0), inv_gamma),
        ];
        for c in 0..3 {
            rgb[c] = ((rgb[c] - 0.5) * contrast + 0.5 + brightness).max(0.0);
        }
        // Saturation
        let luma = rec709_luma(rgb);
        for c in 0..3 {
            rgb[c] = luma + (rgb[c] - luma) * saturation;
        }
        // Hue rotation around (1,1,1)/sqrt(3).
        if absf(hue_shift_deg) > 1e-4 {
            let dot = (rgb[0] + rgb[1] + rgb[2]) * k;
            let cross = [
                (rgb[2] - rgb[1]) * sqrt_k,
                (rgb[0] - rgb[2]) * sqrt_k,
                (rgb[1] - rgb[0]) * sqrt_k,
            ];
            for c in 0..3 {
                rgb[c] = rgb[c] * cos_h + cross[c] * sin_h + dot * (1.0 - cos_h);
            }
        }
        p[0] = rgb[0].max(0.0);
        p[1] = rgb[1].max(0.0);
        p[2] = rgb[2].max(0.0);
        p[3] = clamp01(p[3] * opacity);
    }
    out
}

fn apply_sharpen(frame: &FrameBuffer, amount: f32) -> FrameBuffer {
    let mut out = frame.clone();
    let w = frame.width as i32;
    let h = frame.height as i32;
    let k = amount.clamp(0.0, 2.0);
    if k <= 1e-6 {
        return out;
    }

    for y in 0..h {
        for x in 0..w {
            let c = frame.sample_clamped(x, y);
            let n = frame.sample_clamped(x, y - 1);
            let s = frame.sample_clamped(x, y + 1);
            let w_px = frame.sample_clamped(x - 1, y);
            let e = frame.sample_clamped(x + 1, y);

            let idx = (y as usize) * (w as usize) + (x as usize);
            for ch in 0..3 {
                let sharp = c[ch] * (1.0 + 4.0 * k) - k * (n[ch] + s[ch] + w_px[ch] + e[ch]);
                out.pixels[idx][ch] = sharp.max(0.0);
            }
        }
    }
    out
}

fn apply_chroma_key(
    frame: &FrameBuffer,
    key_rgb: [f32; 3],
    similarity: f32,
    smoothness: f32,
    spill_suppression: f32,
) -> FrameBuffer {
    let mut out = frame.clone();
    let key_ycc = rgb_to_ycbcr_709(key_rgb);
    let key_cb = key_ycc[1];
    let key_cr = key_ycc[2];
    let key_mag = sqrtf(key_cb * key_cb + key_cr * key_cr);
    let key_dir = if key_mag > 1e-5 {
        [key_cb / key_mag, key_cr / key_mag]
    } else {
        [0.0, 0.0]
    };

    let sim = similarity.max(1e-4);
    let sm = smoothness.max(1e-4);
    let spill = clamp01(spill_suppression);

    let dom_idx = if key_rgb[1] > key_rgb[0] * 1.3 && key_rgb[1] > key_rgb[2] * 1.3 {
        Some(1usize)
    } else if key_rgb[2] > key_rgb[0] * 1.3 && key_rgb[2] > key_rgb[1] * 1.3 {
        Some(2usize)
    } else if key_rgb[0] > key_rgb[1] * 1.3 && key_rgb[0] > key_rgb[2] * 1.3 {
        Some(0usize)
    } else {
        None
    };

    for p in &mut out.pixels {
        let ycc = rgb_to_ycbcr_709([p[0], p[1], p[2]]);
        let dcb = ycc[1] - key_cb;
        let dcr = ycc[2] - key_cr;
        let dist = sqrtf(dcb * dcb + dcr * dcr);
        let key_alpha = smoothstep(sim, sim + sm, dist);
        p[3] = clamp01(p[3] * key_alpha);

        if spill > 0.0 && key_mag > 1e-5 {
            let proj = ycc[1] * key_dir[0] + ycc[2] * key_dir[1];
            if proj > 0.0 {
                if let Some(dom) = dom_idx {
                    let other_a = p[(dom + 1) % 3];
                    let other_b = p[(dom + 2) % 3];
                    let cap = other_a.max(other_b);
                    if p[dom] > cap {
                        p[dom] = lerp(p[dom], cap, spill);
                    }
                } else {
                    let new_cb = ycc[1] - spill * proj * key_dir[0];
                    let new_cr = ycc[2] - spill * proj * key_dir[1];
                    let new_y = (ycc[0] - spill * proj * 0.5).max(0.0);
                    let despill = ycbcr_709_to_rgb([new_y, new_cb, new_cr]);
                    p[0] = despill[0].max(0.0);
                    p[1] = despill[1].max(0.0);
                    p[2] = despill[2].max(0.0);
                }
            }
        }
    }
    out
}

fn apply_color_key(
    frame: &FrameBuffer,
    key_rgb: [f32; 3],
    similarity: f32,
    smoothness: f32,
) -> FrameBuffer {
    let mut out = frame.clone();
    let sim = similarity.max(1e-4);
    let sm = smoothness.max(1e-4);
    for p in &mut out.pixels {
        let dr = p[0] - key_rgb[0];
        let dg = p[1] - key_rgb[1];
        let db = p[2] - key_rgb[2];
        let dist = sqrtf(dr * dr + dg * dg + db * db);
        let mask = smoothstep(sim, sim + sm, dist);
        p[3] = clamp01(p[3] * mask);
    }
    out
}

fn apply_luma_key(
    frame: &FrameBuffer,
    luma_max: f32,
    luma_max_smooth: f32,
    luma_min: f32,
    luma_min_smooth: f32,
    invert: bool,
) -> FrameBuffer {
    let mut out = frame.clone();
    let min_sm = luma_min_smooth.max(1e-4);
    let max_sm = luma_max_smooth.max(1e-4);

    for p in &mut out.pixels {
        let luma = rec709_luma([p[0], p[1], p[2]]);
        let low = smoothstep(luma_min, luma_min + min_sm, luma);
        let high = 1.0 - smoothstep(luma_max - max_sm, luma_max, luma);
        let mut mask = clamp01(low * high);
        if invert {
            mask = 1.0 - mask;
        }
        p[3] = clamp01(p[3] * mask);
    }
    out
}

fn apply_lut(frame: &FrameBuffer, lut: &Lut3D, intensity: f32) -> FrameBuffer {
    let mut out = frame.clone();
    let k = clamp01(intensity);
    for p in &mut out.pixels {
        let mapped = lut.sample_trilinear([p[0], p[1], p[2]]);
        p[0] = lerp(p[0], mapped[0], k);
        p[1] = lerp(p[1], mapped[1], k);
        p[2] = lerp(p[2], mapped[2], k);
    }
    out
}

fn apply_image_mask_blend(
    frame: &FrameBuffer,
    mode: MaskBlendMode,
    mask: Option<&FrameBuffer>,
    color: LinearRgba,
    stretch: bool,
) -> FrameBuffer {
    let mut out = frame.clone();
    let w = frame.width;
    let h = frame.height;
    for y in 0..h {
        for x in 0..w {
            let m = if let Some(mf) = mask {
                let (mx, my) = if stretch {
                    (
                        ((x as u64 * mf.width as u64) / w as u64) as u32,
                        ((y as u64 * mf.height as u64) / h as u64) as u32,
                    )
                } else {
                    (x % mf.width, y % mf.height)
                };
                mf.get_pixel(mx, my).unwrap_or([1.0, 1.0, 1.0, 1.0])
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            let idx = (y as usize) * (w as usize) + (x as usize);
            let p = &mut out.pixels[idx];
            let tinted = [
                m[0] * color[0],
                m[1] * color[1],
                m[2] * color[2],
                m[3] * color[3],
            ];
            match mode {
                MaskBlendMode::AlphaMaskAlphaChannel => {
                    p[3] = clamp01(p[3] * tinted[3]);
                }
                MaskBlendMode::AlphaMaskColorChannel => {
                    let luma = rec709_luma([tinted[0], tinted[1], tinted[2]]);
                    p[3] = clamp01(p[3] * luma);
                }
                MaskBlendMode::ColorMultiply => {
                    p[0] *= tinted[0];
                    p[1] *= tinted[1];
                    p[2] *= tinted[2];
                }
                MaskBlendMode::ColorAdd => {
                    p[0] += tinted[0];
                    p[1] += tinted[1];
                    p[2] += tinted[2];
                }
                MaskBlendMode::ColorSubtract => {
                    p[0] = (p[0] - tinted[0]).max(0.0);
                    p[1] = (p[1] - tinted[1]).max(0.0);
                    p[2] = (p[2] - tinted[2]).max(0.0);
                }
            }
        }
    }
    out
}

fn apply_crop_pad(
    frame: &FrameBuffer,
    top: i32,
    right: i32,
    bottom: i32,
    left: i32,
    pad_color: LinearRgba,
) -> Result<FrameBuffer, FilterError> {
    let new_w = frame.width as i32 - left - right;
    let new_h = frame.height as i32 - top - bottom;
    if new_w <= 0 || new_h <= 0 {
        return Err(FilterError::InvalidDimensions);
    }
    let mut out = FrameBuffer::filled(new_w as u32, new_h as u32, pad_color)
        .map_err(|_| FilterError::InvalidDimensions)?;

    for dy in 0..new_h {
        let sy = dy + top;
        if sy < 0 || sy >= frame.height as i32 {
            continue;
        }
        for dx in 0..new_w {
            let sx = dx + left;
            if sx < 0 || sx >= frame.width as i32 {
                continue;
            }
            if let Some(p) = frame.get_pixel(sx as u32, sy as u32) {
                out.set_pixel(dx as u32, dy as u32, p);
            }
        }
    }
    Ok(out)
}

fn apply_scale_aspect(
    frame: &FrameBuffer,
    target_w: u32,
    target_h: u32,
    scaler: Scaler,
    mode: ScaleAspectMode,
) -> Result<FrameBuffer, FilterError> {
    if target_w == 0 || target_h == 0 {
        return Err(FilterError::InvalidDimensions);
    }
    match mode {
        ScaleAspectMode::Stretch => {
            scale_frame(frame, target_w, target_h, scaler).map_err(|_| FilterError::InvalidDimensions)
        }
        ScaleAspectMode::FitLetterbox => {
            let s = (target_w as f32 / frame.width as f32)
                .min(target_h as f32 / frame.height as f32);
            let sw = ((frame.width as f32 * s + 0.5) as u32).clamp(1, target_w);
            let sh = ((frame.height as f32 * s + 0.5) as u32).clamp(1, target_h);
            let scaled = scale_frame(frame, sw, sh, scaler)
                .map_err(|_| FilterError::InvalidDimensions)?;
            let mut out = FrameBuffer::new(target_w, target_h)
                .map_err(|_| FilterError::InvalidDimensions)?;
            let ox = (target_w - sw) / 2;
            let oy = (target_h - sh) / 2;
            for y in 0..sh {
                for x in 0..sw {
                    if let Some(p) = scaled.get_pixel(x, y) {
                        out.set_pixel(ox + x, oy + y, p);
                    }
                }
            }
            Ok(out)
        }
        ScaleAspectMode::CoverCrop => {
            let s = (target_w as f32 / frame.width as f32)
                .max(target_h as f32 / frame.height as f32);
            let sw = ((frame.width as f32 * s + 0.5) as u32).max(target_w);
            let sh = ((frame.height as f32 * s + 0.5) as u32).max(target_h);
            let scaled = scale_frame(frame, sw, sh, scaler)
                .map_err(|_| FilterError::InvalidDimensions)?;
            let mut out = FrameBuffer::new(target_w, target_h)
                .map_err(|_| FilterError::InvalidDimensions)?;
            let ox = (sw - target_w) / 2;
            let oy = (sh - target_h) / 2;
            for y in 0..target_h {
                for x in 0..target_w {
                    if let Some(p) = scaled.get_pixel(ox + x, oy + y) {
                        out.set_pixel(x, y, p);
                    }
                }
            }
            Ok(out)
        }
    }
}

fn apply_scroll(
    frame: &FrameBuffer,
    timestamp_us: i64,
    speed_x_px_s: f32,
    speed_y_px_s: f32,
    loop_wrap: bool,
    limit_width: Option<u32>,
    limit_height: Option<u32>,
) -> Result<FrameBuffer, FilterError> {
    let out_w = limit_width.unwrap_or(frame.width).max(1);
    let out_h = limit_height.unwrap_or(frame.height).max(1);
    let mut out = FrameBuffer::new(out_w, out_h).map_err(|_| FilterError::InvalidDimensions)?;
    let t_s = timestamp_us as f32 / 1_000_000.0;
    let off_x = speed_x_px_s * t_s;
    let off_y = speed_y_px_s * t_s;
    let fw = frame.width as f32;
    let fh = frame.height as f32;

    for y in 0..out_h {
        for x in 0..out_w {
            let sx = x as f32 + off_x;
            let sy = y as f32 + off_y;
            let (ux, uy) = if loop_wrap {
                (fract(sx / fw) * fw, fract(sy / fh) * fh)
            } else {
                (sx, sy)
            };
            if ux >= 0.0 && ux < fw && uy >= 0.0 && uy < fh {
                let ix = floorf(ux) as u32;
                let iy = floorf(uy) as u32;
                if let Some(p) = frame.get_pixel(ix, iy) {
                    out.set_pixel(x, y, p);
                }
            }
        }
    }
    Ok(out)
}

fn apply_blur(frame: &FrameBuffer, kind: BlurKind, radius: f32, passes: u8) -> FrameBuffer {
    let r = (floorf(radius) as i32).clamp(0, 32);
    if r == 0 {
        return frame.clone();
    }
    let num_passes = passes.clamp(1, 6);
    let mut cur = frame.clone();

    for _ in 0..num_passes {
        cur = match kind {
            BlurKind::Box => separable_blur(&cur, r, false),
            BlurKind::Gaussian => separable_blur(&cur, r, true),
            BlurKind::DualKawase => {
                let d = separable_blur(&cur, (r / 2).max(1), true);
                separable_blur(&d, r, true)
            }
        };
    }
    cur
}

fn separable_blur(frame: &FrameBuffer, radius: i32, gaussian: bool) -> FrameBuffer {
    let w = frame.width as i32;
    let h = frame.height as i32;
    let sigma = (radius as f32 * 0.5).max(0.5);
    let two_sig2 = 2.0 * sigma * sigma;

    let mut weights = Vec::with_capacity((radius * 2 + 1) as usize);
    let mut wsum = 0.0_f32;
    for k in -radius..=radius {
        let wt = if gaussian {
            expf(-(k as f32 * k as f32) / two_sig2)
        } else {
            1.0
        };
        weights.push(wt);
        wsum += wt;
    }
    for wt in &mut weights {
        *wt /= wsum;
    }

    // Horizontal pass (premultiplied alpha).
    let mut horiz = vec![[0.0_f32; 4]; frame.pixels.len()];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0_f32; 4];
            for (i, k) in (-radius..=radius).enumerate() {
                let s = frame.sample_clamped(x + k, y);
                let wt = weights[i];
                let a = clamp01(s[3]);
                acc[0] += s[0] * a * wt;
                acc[1] += s[1] * a * wt;
                acc[2] += s[2] * a * wt;
                acc[3] += a * wt;
            }
            horiz[(y as usize) * (w as usize) + (x as usize)] = acc;
        }
    }

    // Vertical pass.
    let mut out = frame.clone();
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0_f32; 4];
            for (i, k) in (-radius..=radius).enumerate() {
                let cy = (y + k).clamp(0, h - 1) as usize;
                let s = horiz[cy * (w as usize) + (x as usize)];
                let wt = weights[i];
                for c in 0..4 {
                    acc[c] += s[c] * wt;
                }
            }
            let a = clamp01(acc[3]);
            let idx = (y as usize) * (w as usize) + (x as usize);
            if a <= 1e-6 {
                out.pixels[idx] = [0.0, 0.0, 0.0, 0.0];
            } else {
                out.pixels[idx] = [acc[0] / a, acc[1] / a, acc[2] / a, a];
            }
        }
    }
    out
}

fn apply_drop_shadow(
    frame: &FrameBuffer,
    offset_x: f32,
    offset_y: f32,
    blur_radius: f32,
    color: LinearRgba,
    spread: f32,
) -> FrameBuffer {
    let w = frame.width;
    let h = frame.height;
    let mut alpha_buf = FrameBuffer::filled(w, h, [0.0, 0.0, 0.0, 0.0]).unwrap_or_else(|_| frame.clone());
    let spread_gain = 1.0 + spread.clamp(0.0, 4.0);

    for y in 0..h {
        for x in 0..w {
            let sx = floorf(x as f32 - offset_x) as i32;
            let sy = floorf(y as f32 - offset_y) as i32;
            let a = if sx >= 0 && sx < w as i32 && sy >= 0 && sy < h as i32 {
                clamp01(frame.pixels[(sy as usize) * (w as usize) + (sx as usize)][3] * spread_gain)
            } else {
                0.0
            };
            alpha_buf.set_pixel(x, y, [color[0], color[1], color[2], a * color[3]]);
        }
    }

    let blurred_shadow = apply_blur(&alpha_buf, BlurKind::Gaussian, blur_radius, 1);
    let mut out = blurred_shadow;

    // Composite foreground source OVER shadow.
    for (dst, &src) in out.pixels.iter_mut().zip(frame.pixels.iter()) {
        let sa = clamp01(src[3]);
        let da = clamp01(dst[3]);
        let out_a = sa + da * (1.0 - sa);
        if out_a <= 1e-6 {
            *dst = [0.0, 0.0, 0.0, 0.0];
        } else {
            for c in 0..3 {
                dst[c] = (src[c] * sa + dst[c] * da * (1.0 - sa)) / out_a;
            }
            dst[3] = out_a;
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn apply_ai_background_matte(
    frame: &FrameBuffer,
    ai_input: AiMatteInput<'_>,
    ctx: &mut FilterContext,
    mode: AiMatteMode,
    blur_radius: f32,
    feather: f32,
    temporal_smoothing: f32,
    edge_refine_radius: u8,
) -> FrameBuffer {
    let count = (frame.width as usize) * (frame.height as usize);
    let raw_mask = match ai_input {
        AiMatteInput::Success(m) if m.len() == count => m,
        // FAIL-CLOSED: Inference failure or missing prediction MUST completely hide the source
        // so private backgrounds are never leaked on air.
        AiMatteInput::Success(_) | AiMatteInput::InferenceFailed | AiMatteInput::Unavailable => {
            ctx.ai_matte_status = AiMatteStatus::FailedClosedHidden;
            ctx.prev_matte_alpha = None;
            return FrameBuffer {
                width: frame.width,
                height: frame.height,
                pixels: vec![[0.0, 0.0, 0.0, 0.0]; count],
            };
        }
    };

    ctx.ai_matte_status = AiMatteStatus::Applied;

    // 1. Guided-filter edge refinement using image luminance as guide `I`.
    let rad = (edge_refine_radius as i32).clamp(1, 8);
    let mut refined = guided_filter_alpha(frame, raw_mask, rad, 1e-3);

    // 2. Feather ramp.
    let f = clamp01(feather) * 0.45;
    if f > 1e-4 {
        for a in &mut refined {
            *a = smoothstep(0.5 - f, 0.5 + f, *a);
        }
    }

    // 3. Temporal anti-shimmer.
    let ts = clamp01(temporal_smoothing);
    if ts > 0.0 {
        if let Some(prev_a) = &ctx.prev_matte_alpha {
            if prev_a.len() == count {
                for i in 0..count {
                    let delta = absf(refined[i] - prev_a[i]);
                    // Suppress subtle boundary shimmer (<0.20 delta), allow real motion through.
                    let shimmer_suppress = ts * (1.0 - smoothstep(0.05, 0.25, delta));
                    refined[i] = lerp(refined[i], prev_a[i], shimmer_suppress);
                }
            }
        }
    }
    ctx.prev_matte_alpha = Some(refined.clone());

    // 4. Apply matte mode.
    match mode {
        AiMatteMode::Cutout => {
            let mut out = frame.clone();
            for (p, &a) in out.pixels.iter_mut().zip(refined.iter()) {
                p[3] = clamp01(p[3] * a);
            }
            out
        }
        AiMatteMode::BackgroundBlur => {
            let bg = apply_blur(frame, BlurKind::Gaussian, blur_radius.max(2.0), 1);
            let mut out = frame.clone();
            for ((dst, &fg), (&b, &a)) in out
                .pixels
                .iter_mut()
                .zip(frame.pixels.iter())
                .zip(bg.pixels.iter().zip(refined.iter()))
            {
                *dst = [
                    lerp(b[0], fg[0], a),
                    lerp(b[1], fg[1], a),
                    lerp(b[2], fg[2], a),
                    fg[3],
                ];
            }
            out
        }
        AiMatteMode::DesaturateBackground => {
            let mut out = frame.clone();
            for (p, &a) in out.pixels.iter_mut().zip(refined.iter()) {
                let y = rec709_luma([p[0], p[1], p[2]]);
                p[0] = lerp(y, p[0], a);
                p[1] = lerp(y, p[1], a);
                p[2] = lerp(y, p[2], a);
            }
            out
        }
    }
}

/// Fast separable guided filter refining coarse AI matte `p` against image luminance guide `I`.
fn guided_filter_alpha(
    guide: &FrameBuffer,
    p_mask: &[f32],
    radius: i32,
    eps: f32,
) -> Vec<f32> {
    let w = guide.width as usize;
    let h = guide.height as usize;
    let n = w * h;

    let mut i_vals = Vec::with_capacity(n);
    let mut ii_vals = Vec::with_capacity(n);
    let mut ip_vals = Vec::with_capacity(n);
    for (px, &p) in guide.pixels.iter().zip(p_mask.iter()) {
        let g = rec709_luma([px[0], px[1], px[2]]);
        let pv = clamp01(p);
        i_vals.push(g);
        ii_vals.push(g * g);
        ip_vals.push(g * pv);
    }

    let mean_i = box_filter_scalar(&i_vals, w, h, radius);
    let mean_p = box_filter_scalar(p_mask, w, h, radius);
    let corr_i = box_filter_scalar(&ii_vals, w, h, radius);
    let corr_ip = box_filter_scalar(&ip_vals, w, h, radius);

    let mut a_buf = vec![0.0_f32; n];
    let mut b_buf = vec![0.0_f32; n];
    for idx in 0..n {
        let var_i = (corr_i[idx] - mean_i[idx] * mean_i[idx]).max(0.0);
        let cov_ip = corr_ip[idx] - mean_i[idx] * mean_p[idx];
        let a = cov_ip / (var_i + eps);
        let b = mean_p[idx] - a * mean_i[idx];
        a_buf[idx] = a;
        b_buf[idx] = b;
    }

    let mean_a = box_filter_scalar(&a_buf, w, h, radius);
    let mean_b = box_filter_scalar(&b_buf, w, h, radius);

    let mut q = Vec::with_capacity(n);
    for idx in 0..n {
        q.push(clamp01(mean_a[idx] * i_vals[idx] + mean_b[idx]));
    }
    q
}

fn box_filter_scalar(data: &[f32], w: usize, h: usize, radius: i32) -> Vec<f32> {
    let mut horiz = vec![0.0_f32; w * h];
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let mut sum = 0.0_f32;
            let mut cnt = 0.0_f32;
            let x0 = (x as i32 - radius).max(0) as usize;
            let x1 = (x as i32 + radius).min(w as i32 - 1) as usize;
            for sx in x0..=x1 {
                sum += data[row + sx];
                cnt += 1.0;
            }
            horiz[row + x] = sum / cnt;
        }
    }

    let mut out = vec![0.0_f32; w * h];
    for y in 0..h {
        let y0 = (y as i32 - radius).max(0) as usize;
        let y1 = (y as i32 + radius).min(h as i32 - 1) as usize;
        let cnt = (y1 - y0 + 1) as f32;
        for x in 0..w {
            let mut sum = 0.0_f32;
            for sy in y0..=y1 {
                sum += horiz[sy * w + x];
            }
            out[y * w + x] = sum / cnt;
        }
    }
    out
}

fn apply_light_wrap(
    fg: &FrameBuffer,
    composited_bg: Option<&FrameBuffer>,
    intensity: f32,
    radius: f32,
    luma_threshold: f32,
    soft_bleed: f32,
) -> FrameBuffer {
    let Some(bg_raw) = composited_bg else {
        return fg.clone();
    };
    let intens = intensity.max(0.0);
    if intens <= 1e-6 {
        return fg.clone();
    }

    let bg_matched = if bg_raw.width == fg.width && bg_raw.height == fg.height {
        bg_raw.clone()
    } else {
        match scale_frame(bg_raw, fg.width, fg.height, Scaler::Bilinear) {
            Ok(scaled) => scaled,
            Err(_) => return fg.clone(),
        }
    };

    // Gate background highlights by luma_threshold.
    let mut bg_gated = bg_matched;
    for p in &mut bg_gated.pixels {
        let luma = rec709_luma([p[0], p[1], p[2]]);
        let gate = smoothstep(luma_threshold, (luma_threshold + 0.25).max(luma_threshold + 1e-3), luma);
        p[0] *= gate;
        p[1] *= gate;
        p[2] *= gate;
    }

    let rad = (floorf(radius) as i32).clamp(1, 32);
    let bg_blur = separable_blur(&bg_gated, rad, true);

    // Compute blurred inverted foreground alpha to locate interior perimeter rim.
    let w = fg.width as usize;
    let h = fg.height as usize;
    let inv_alpha: Vec<f32> = fg.pixels.iter().map(|p| 1.0 - clamp01(p[3])).collect();
    let blurred_inv_alpha = box_filter_scalar(&inv_alpha, w, h, rad);

    let bleed = 1.0 + soft_bleed.max(0.0);
    let mut out = fg.clone();
    for i in 0..fg.pixels.len() {
        let fg_a = clamp01(fg.pixels[i][3]);
        let wrap_amt = clamp01(fg_a * blurred_inv_alpha[i] * bleed) * intens;
        if wrap_amt > 1e-6 {
            let bg_p = bg_blur.pixels[i];
            for c in 0..3 {
                out.pixels[i][c] = fg.pixels[i][c] + bg_p[c] * wrap_amt;
            }
        }
    }
    out
}

fn apply_hdr_tonemap(
    frame: &FrameBuffer,
    method: ToneMapMethod,
    source_peak_nits: f32,
    target_peak_nits: f32,
    desaturate_highlights: f32,
) -> FrameBuffer {
    let mut out = frame.clone();
    let ratio = (source_peak_nits / target_peak_nits.max(1.0)).max(1.0);
    let desat = clamp01(desaturate_highlights);

    for p in &mut out.pixels {
        let mut mapped = [0.0_f32; 3];
        for c in 0..3 {
            let x = p[c].max(0.0);
            mapped[c] = match method {
                ToneMapMethod::Reinhard => x / (1.0 + x),
                ToneMapMethod::HableFilmic => {
                    let h = |v: f32| -> f32 {
                        let a = 0.15;
                        let b = 0.50;
                        let c = 0.10;
                        let d = 0.20;
                        let e = 0.02;
                        let f = 0.30;
                        ((v * (a * v + c * b) + d * e) / (v * (a * v + b) + d * f)) - e / f
                    };
                    h(x * 2.0) / h(11.2).max(1e-4)
                }
                ToneMapMethod::AcesFitted => {
                    let a = 2.51;
                    let b = 0.03;
                    let c_k = 2.43;
                    let d = 0.59;
                    let e = 0.14;
                    clamp01((x * (a * x + b)) / (x * (c_k * x + d) + e))
                }
                ToneMapMethod::Bt2390Eetf => {
                    let scaled = x / ratio;
                    scaled / (1.0 + scaled) * (1.0 + 1.0 / ratio)
                }
            };
        }
        if desat > 0.0 {
            let luma = rec709_luma(mapped);
            let hi = smoothstep(0.6, 1.0, luma) * desat;
            for c in 0..3 {
                mapped[c] = lerp(mapped[c], luma, hi);
            }
        }
        p[0] = clamp01(mapped[0]);
        p[1] = clamp01(mapped[1]);
        p[2] = clamp01(mapped[2]);
    }
    out
}

fn apply_sdr_on_hdr(
    frame: &FrameBuffer,
    sdr_white_nits: f32,
    paper_white_scale: f32,
    expand_highlights: f32,
) -> FrameBuffer {
    let mut out = frame.clone();
    // Normalized relative to 100-nit or 203-nit ITU-R BT.2408 diffuse white.
    let base_scale = (sdr_white_nits.max(80.0) / 100.0) * paper_white_scale.max(0.1);
    let boost = expand_highlights.max(0.0);

    for p in &mut out.pixels {
        for c in 0..3 {
            let v = p[c].max(0.0);
            let hi = if v > 0.85 && boost > 0.0 {
                1.0 + boost * ((v - 0.85) / 0.15)
            } else {
                1.0
            };
            p[c] = v * base_scale * hi;
        }
    }
    out
}

#[inline]
fn ycbcr_709_to_rgb(ycc: [f32; 3]) -> [f32; 3] {
    let y = ycc[0];
    let cb = ycc[1];
    let cr = ycc[2];
    let r = y + 1.5748 * cr;
    let b = y + 1.8556 * cb;
    let g = (y - 0.2126 * r - 0.0722 * b) / 0.7152;
    [r, g, b]
}
