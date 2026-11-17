//! Linear-light FP32 compositor, 2D transforms, snapping guides, 6 scalers, and 9 blend modes.

use alloc::vec;
use alloc::vec::Vec;
use core::f32::consts::PI;

use crate::math::{
    absf, bicubic_catmull_rom, clamp01, cosf, floorf, lanczos3_weight, lerp, linear_to_srgb,
    linear_to_srgb_u8, sinf, srgb_to_linear, srgb_u8_to_linear,
};

/// Linear-light RGBA pixel `[r, g, b, a]`.
///
/// RGB channels are straight (unpremultiplied) linear light (`0.0..=1.0` for SDR,
/// `> 1.0` allowed for HDR), and `a` is coverage in `0.0..=1.0`.
pub type LinearRgba = [f32; 4];

/// Maximum supported dimension for a canvas or group sub-composite (16k).
pub const MAX_COMPOSITOR_DIM: u32 = 16_384;

/// Errors returned by compositor frame allocation and resampling operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompositorError {
    ZeroDimensions,
    DimensionsExceedLimit { width: u32, height: u32 },
    PixelCountMismatch { expected: usize, actual: usize },
    ByteLengthMismatch { expected: usize, actual: usize },
}

/// Linear-light FP32 RGBA image surface.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameBuffer {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<LinearRgba>,
}

impl FrameBuffer {
    /// Allocates a transparent black frame (`[0.0, 0.0, 0.0, 0.0]`).
    pub fn new(width: u32, height: u32) -> Result<Self, CompositorError> {
        Self::filled(width, height, [0.0, 0.0, 0.0, 0.0])
    }

    /// Allocates a frame filled with a single linear RGBA value.
    pub fn filled(width: u32, height: u32, rgba: LinearRgba) -> Result<Self, CompositorError> {
        validate_dims(width, height)?;
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or(CompositorError::DimensionsExceedLimit { width, height })?;
        Ok(Self {
            width,
            height,
            pixels: vec![rgba; count],
        })
    }

    /// Wraps an existing pixel vector after checking dimension invariants.
    pub fn from_pixels(
        width: u32,
        height: u32,
        pixels: Vec<LinearRgba>,
    ) -> Result<Self, CompositorError> {
        validate_dims(width, height)?;
        let expected = (width as usize)
            .checked_mul(height as usize)
            .ok_or(CompositorError::DimensionsExceedLimit { width, height })?;
        if pixels.len() != expected {
            return Err(CompositorError::PixelCountMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Decodes an 8-bit sRGB straight RGBA buffer (`[r, g, b, a, ...]`) into linear FP32.
    pub fn from_srgb_rgba8(
        width: u32,
        height: u32,
        bytes: &[u8],
    ) -> Result<Self, CompositorError> {
        validate_dims(width, height)?;
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or(CompositorError::DimensionsExceedLimit { width, height })?;
        let expected = count * 4;
        if bytes.len() != expected {
            return Err(CompositorError::ByteLengthMismatch {
                expected,
                actual: bytes.len(),
            });
        }
        let mut pixels = Vec::with_capacity(count);
        for chunk in bytes.chunks_exact(4) {
            pixels.push([
                srgb_u8_to_linear(chunk[0]),
                srgb_u8_to_linear(chunk[1]),
                srgb_u8_to_linear(chunk[2]),
                chunk[3] as f32 / 255.0,
            ]);
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Encodes the linear FP32 surface into 8-bit sRGB straight RGBA bytes.
    pub fn to_srgb_rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for &p in &self.pixels {
            out.push(linear_to_srgb_u8(p[0]));
            out.push(linear_to_srgb_u8(p[1]));
            out.push(linear_to_srgb_u8(p[2]));
            out.push((clamp01(p[3]) * 255.0 + 0.5).clamp(0.0, 255.0) as u8);
        }
        out
    }

    /// Reads the pixel at `(x, y)` or `None` if out of bounds.
    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<LinearRgba> {
        if x < self.width && y < self.height {
            self.pixels
                .get((y as usize) * (self.width as usize) + (x as usize))
                .copied()
        } else {
            None
        }
    }

    /// Writes the pixel at `(x, y)` if in bounds.
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: LinearRgba) {
        if x < self.width && y < self.height {
            let idx = (y as usize) * (self.width as usize) + (x as usize);
            if let Some(slot) = self.pixels.get_mut(idx) {
                *slot = rgba;
            }
        }
    }

    /// Reads the nearest edge-clamped pixel at signed integer coordinates.
    #[inline]
    pub fn sample_clamped(&self, x: i32, y: i32) -> LinearRgba {
        let cx = x.clamp(0, self.width as i32 - 1) as u32;
        let cy = y.clamp(0, self.height as i32 - 1) as u32;
        self.pixels[(cy as usize) * (self.width as usize) + (cx as usize)]
    }

    /// Fills all pixels with `rgba`.
    pub fn clear(&mut self, rgba: LinearRgba) {
        self.pixels.fill(rgba);
    }
}

#[inline]
fn validate_dims(width: u32, height: u32) -> Result<(), CompositorError> {
    if width == 0 || height == 0 {
        return Err(CompositorError::ZeroDimensions);
    }
    if width > MAX_COMPOSITOR_DIM || height > MAX_COMPOSITOR_DIM {
        return Err(CompositorError::DimensionsExceedLimit { width, height });
    }
    Ok(())
}

/// Normalized pivot/alignment anchor within a source or bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    Custom { x: f32, y: f32 },
}

impl Default for Anchor {
    fn default() -> Self {
        Self::TopLeft
    }
}

impl Anchor {
    /// Returns `[nx, ny]` in `0.0..=1.0`.
    pub const fn normalized(self) -> [f32; 2] {
        match self {
            Self::TopLeft => [0.0, 0.0],
            Self::TopCenter => [0.5, 0.0],
            Self::TopRight => [1.0, 0.0],
            Self::CenterLeft => [0.0, 0.5],
            Self::Center => [0.5, 0.5],
            Self::CenterRight => [1.0, 0.5],
            Self::BottomLeft => [0.0, 1.0],
            Self::BottomCenter => [0.5, 1.0],
            Self::BottomRight => [1.0, 1.0],
            Self::Custom { x, y } => [x, y],
        }
    }
}

/// Non-destructive source edge crop in source pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CropRect {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// All 8 OBS/Cabaret bounding-box scaling modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BoundsMode {
    /// Use raw `scale` directly without bounding box fitting.
    #[default]
    None,
    /// Stretch non-uniformly to match `bounds`.
    Stretch,
    /// Scale uniformly to fit entirely inside `bounds` (letterboxed).
    Fit,
    /// Scale uniformly to fill `bounds` (may extend beyond bounds unless clipped).
    Fill,
    /// Scale uniformly to cover `bounds` and clip any overflow cleanly to `bounds`.
    Cover,
    /// Scale uniformly so width equals `bounds[0]`.
    ScaleToWidth,
    /// Scale uniformly so height equals `bounds[1]`.
    ScaleToHeight,
    /// Scale down uniformly to fit inside `bounds`, never upscaling above `1.0`.
    MaxOnly,
}

/// Complete 2D transform specification for a scene item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform2D {
    pub pos: [f32; 2],
    pub scale: [f32; 2],
    pub rot_deg: f32,
    pub anchor: Anchor,
    pub crop: CropRect,
    pub flip_h: bool,
    pub flip_v: bool,
    pub bounds_mode: BoundsMode,
    pub bounds: [f32; 2],
    pub bounds_alignment: Anchor,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            pos: [0.0, 0.0],
            scale: [1.0, 1.0],
            rot_deg: 0.0,
            anchor: Anchor::TopLeft,
            crop: CropRect::default(),
            flip_h: false,
            flip_v: false,
            bounds_mode: BoundsMode::None,
            bounds: [0.0, 0.0],
            bounds_alignment: Anchor::Center,
        }
    }
}

/// Axis-aligned rectangle in canvas coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect2D {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect2D {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }
    pub fn center_x(self) -> f32 {
        self.x + self.w * 0.5
    }
    pub fn center_y(self) -> f32 {
        self.y + self.h * 0.5
    }
}

/// Precomputed affine geometry for rendering a `Transform2D` against a concrete source size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedTransform {
    /// Width and height of cropped source region in source pixels.
    pub cropped_size: [f32; 2],
    /// Effective scale factors `(sx, sy)` from cropped source pixels to canvas pixels.
    pub effective_scale: [f32; 2],
    /// Drawn box size before rotation.
    pub box_size: [f32; 2],
    /// Offset of drawn image inside `box_size` (used for bounded alignment).
    pub inner_offset: [f32; 2],
    /// Drawn image size before rotation (`cropped_size * effective_scale`).
    pub drawn_size: [f32; 2],
    /// Whether overflow outside `box_size` should be clipped (`BoundsMode::Cover`).
    pub clip_to_box: bool,
    /// Axis-aligned bounding box in canvas space after rotation.
    pub aabb: Rect2D,
}

impl Transform2D {
    /// Resolves bounds scaling, alignment, and rotated canvas AABB for a source of `(src_w, src_h)`.
    pub fn resolve(&self, src_w: f32, src_h: f32) -> ResolvedTransform {
        let crop_l = self.crop.left.max(0.0);
        let crop_t = self.crop.top.max(0.0);
        let cw = (src_w - crop_l - self.crop.right.max(0.0)).max(0.0);
        let ch = (src_h - crop_t - self.crop.bottom.max(0.0)).max(0.0);

        if cw <= 0.0 || ch <= 0.0 {
            return ResolvedTransform {
                cropped_size: [0.0, 0.0],
                effective_scale: [0.0, 0.0],
                box_size: [0.0, 0.0],
                inner_offset: [0.0, 0.0],
                drawn_size: [0.0, 0.0],
                clip_to_box: false,
                aabb: Rect2D::new(self.pos[0], self.pos[1], 0.0, 0.0),
            };
        }

        let bw = self.bounds[0].max(1.0);
        let bh = self.bounds[1].max(1.0);

        let (eff_sx, eff_sy, box_w, box_h, clip_to_box) = match self.bounds_mode {
            BoundsMode::None => (
                self.scale[0],
                self.scale[1],
                cw * absf(self.scale[0]),
                ch * absf(self.scale[1]),
                false,
            ),
            BoundsMode::Stretch => (bw / cw, bh / ch, bw, bh, false),
            BoundsMode::Fit => {
                let s = (bw / cw).min(bh / ch);
                (s, s, bw, bh, false)
            }
            BoundsMode::Fill => {
                let s = (bw / cw).max(bh / ch);
                (s, s, bw, bh, false)
            }
            BoundsMode::Cover => {
                let s = (bw / cw).max(bh / ch);
                (s, s, bw, bh, true)
            }
            BoundsMode::ScaleToWidth => {
                let s = bw / cw;
                (s, s, bw, ch * s, false)
            }
            BoundsMode::ScaleToHeight => {
                let s = bh / ch;
                (s, s, cw * s, bh, false)
            }
            BoundsMode::MaxOnly => {
                let s = 1.0_f32.min((bw / cw).min(bh / ch));
                (s, s, bw, bh, false)
            }
        };

        let drawn_w = cw * absf(eff_sx);
        let drawn_h = ch * absf(eff_sy);

        let align = self.bounds_alignment.normalized();
        let inner_x = if self.bounds_mode == BoundsMode::None {
            0.0
        } else {
            (box_w - drawn_w) * align[0]
        };
        let inner_y = if self.bounds_mode == BoundsMode::None {
            0.0
        } else {
            (box_h - drawn_h) * align[1]
        };

        // Compute rotated AABB around `self.pos`.
        let anc = self.anchor.normalized();
        let ax = anc[0] * box_w;
        let ay = anc[1] * box_h;

        let (x0, y0, x1, y1) = if clip_to_box {
            (0.0, 0.0, box_w, box_h)
        } else {
            (
                inner_x.min(0.0),
                inner_y.min(0.0),
                (inner_x + drawn_w).max(box_w),
                (inner_y + drawn_h).max(box_h),
            )
        };
        // If `BoundsMode::None` or `ScaleToWidth` or `ScaleToHeight`, tight image box:
        let (rx0, ry0, rx1, ry1) = if matches!(
            self.bounds_mode,
            BoundsMode::None | BoundsMode::ScaleToWidth | BoundsMode::ScaleToHeight
        ) {
            (0.0, 0.0, drawn_w, drawn_h)
        } else if clip_to_box {
            (0.0, 0.0, box_w, box_h)
        } else {
            (x0, y0, x1, y1)
        };

        let rad = self.rot_deg * (PI / 180.0);
        let c = cosf(rad);
        let s = sinf(rad);

        let corners = [(rx0, ry0), (rx1, ry0), (rx1, ry1), (rx0, ry1)];
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for (lx, ly) in corners {
            let dx = lx - ax;
            let dy = ly - ay;
            let wx = self.pos[0] + dx * c - dy * s;
            let wy = self.pos[1] + dx * s + dy * c;
            min_x = min_x.min(wx);
            min_y = min_y.min(wy);
            max_x = max_x.max(wx);
            max_y = max_y.max(wy);
        }

        ResolvedTransform {
            cropped_size: [cw, ch],
            effective_scale: [eff_sx, eff_sy],
            box_size: [box_w, box_h],
            inner_offset: [inner_x, inner_y],
            drawn_size: [drawn_w, drawn_h],
            clip_to_box,
            aabb: Rect2D::new(min_x, min_y, (max_x - min_x).max(0.0), (max_y - min_y).max(0.0)),
        }
    }

    /// Rescales position and scale/bounds proportionally when canvas dimensions change.
    pub fn rescale(&mut self, sx: f32, sy: f32) {
        self.pos[0] *= sx;
        self.pos[1] *= sy;
        if self.bounds_mode == BoundsMode::None {
            self.scale[0] *= sx;
            self.scale[1] *= sy;
        } else {
            self.bounds[0] *= sx;
            self.bounds[1] *= sy;
        }
    }
}

/// Configuration for the canvas snapping guide solver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapConfig {
    pub snap_distance: f32,
    pub snap_to_canvas_edges: bool,
    pub snap_to_canvas_center: bool,
    pub snap_to_safe_areas: bool,
    pub snap_to_sources: bool,
}

impl Default for SnapConfig {
    fn default() -> Self {
        Self {
            snap_distance: 10.0,
            snap_to_canvas_edges: true,
            snap_to_canvas_center: true,
            snap_to_safe_areas: true,
            snap_to_sources: true,
        }
    }
}

/// Category of snapping target matched by the guide solver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapGuideKind {
    CanvasEdge,
    CanvasCenter,
    SafeArea90,
    SafeArea80,
    SourceEdge,
    SourceCenter,
}

/// Visual guide line produced by the snapping solver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapGuide {
    Vertical { x: f32, kind: SnapGuideKind },
    Horizontal { y: f32, kind: SnapGuideKind },
}

/// Result of snapping a candidate source rectangle on a canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapResult {
    pub dx: f32,
    pub dy: f32,
    pub snapped_rect: Rect2D,
    pub guides: Vec<SnapGuide>,
}

/// Solves axis-aligned magnetic snapping against canvas edges, centers, safe areas, and sibling sources.
pub fn solve_snap(
    candidate: Rect2D,
    canvas_w: f32,
    canvas_h: f32,
    other_sources: &[Rect2D],
    config: SnapConfig,
) -> SnapResult {
    let mut x_targets: Vec<(f32, SnapGuideKind)> = Vec::new();
    let mut y_targets: Vec<(f32, SnapGuideKind)> = Vec::new();

    if config.snap_to_canvas_edges {
        x_targets.push((0.0, SnapGuideKind::CanvasEdge));
        x_targets.push((canvas_w, SnapGuideKind::CanvasEdge));
        y_targets.push((0.0, SnapGuideKind::CanvasEdge));
        y_targets.push((canvas_h, SnapGuideKind::CanvasEdge));
    }
    if config.snap_to_canvas_center {
        x_targets.push((canvas_w * 0.5, SnapGuideKind::CanvasCenter));
        y_targets.push((canvas_h * 0.5, SnapGuideKind::CanvasCenter));
    }
    if config.snap_to_safe_areas {
        for (margin, kind) in [
            (0.05, SnapGuideKind::SafeArea90),
            (0.10, SnapGuideKind::SafeArea80),
        ] {
            x_targets.push((canvas_w * margin, kind));
            x_targets.push((canvas_w * (1.0 - margin), kind));
            y_targets.push((canvas_h * margin, kind));
            y_targets.push((canvas_h * (1.0 - margin), kind));
        }
    }
    if config.snap_to_sources {
        for r in other_sources {
            x_targets.push((r.x, SnapGuideKind::SourceEdge));
            x_targets.push((r.right(), SnapGuideKind::SourceEdge));
            x_targets.push((r.center_x(), SnapGuideKind::SourceCenter));
            y_targets.push((r.y, SnapGuideKind::SourceEdge));
            y_targets.push((r.bottom(), SnapGuideKind::SourceEdge));
            y_targets.push((r.center_y(), SnapGuideKind::SourceCenter));
        }
    }

    let cand_xs = [candidate.x, candidate.center_x(), candidate.right()];
    let cand_ys = [candidate.y, candidate.center_y(), candidate.bottom()];

    let mut best_dx = 0.0_f32;
    let mut best_x_dist = config.snap_distance + 1e-4;
    let mut x_matched = false;

    for &cx in &cand_xs {
        for &(tx, _) in &x_targets {
            let dist = absf(tx - cx);
            if dist <= config.snap_distance && dist < best_x_dist {
                best_x_dist = dist;
                best_dx = tx - cx;
                x_matched = true;
            }
        }
    }

    let mut best_dy = 0.0_f32;
    let mut best_y_dist = config.snap_distance + 1e-4;
    let mut y_matched = false;

    for &cy in &cand_ys {
        for &(ty, _) in &y_targets {
            let dist = absf(ty - cy);
            if dist <= config.snap_distance && dist < best_y_dist {
                best_y_dist = dist;
                best_dy = ty - cy;
                y_matched = true;
            }
        }
    }

    let snapped_rect = Rect2D::new(
        candidate.x + best_dx,
        candidate.y + best_dy,
        candidate.w,
        candidate.h,
    );

    let mut guides = Vec::new();
    if x_matched {
        let sx_pts = [
            snapped_rect.x,
            snapped_rect.center_x(),
            snapped_rect.right(),
        ];
        for &(tx, kind) in &x_targets {
            if sx_pts.iter().any(|&p| absf(p - tx) < 1e-3) {
                let g = SnapGuide::Vertical { x: tx, kind };
                if !guides.contains(&g) {
                    guides.push(g);
                }
            }
        }
    }
    if y_matched {
        let sy_pts = [
            snapped_rect.y,
            snapped_rect.center_y(),
            snapped_rect.bottom(),
        ];
        for &(ty, kind) in &y_targets {
            if sy_pts.iter().any(|&p| absf(p - ty) < 1e-3) {
                let g = SnapGuide::Horizontal { y: ty, kind };
                if !guides.contains(&g) {
                    guides.push(g);
                }
            }
        }
    }

    SnapResult {
        dx: best_dx,
        dy: best_dy,
        snapped_rect,
        guides,
    }
}

/// All 6 linear-light resampling scalers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scaler {
    #[default]
    Auto,
    Point,
    Bilinear,
    Bicubic,
    Lanczos3,
    Area,
}

/// Resolves `Scaler::Auto` to a concrete algorithm based on effective scale factors.
#[inline]
pub fn resolve_auto_scaler(scale_x: f32, scale_y: f32) -> Scaler {
    let min_s = absf(scale_x).min(absf(scale_y));
    let max_s = absf(scale_x).max(absf(scale_y));
    if absf(scale_x - 1.0) < 1e-3 && absf(scale_y - 1.0) < 1e-3 {
        Scaler::Point
    } else if min_s < 0.5 {
        Scaler::Area
    } else if max_s > 1.0 {
        Scaler::Lanczos3
    } else {
        Scaler::Bicubic
    }
}

#[inline]
fn to_premul(p: LinearRgba) -> [f32; 4] {
    let a = clamp01(p[3]);
    [p[0] * a, p[1] * a, p[2] * a, a]
}

#[inline]
fn from_premul(acc: [f32; 4], wsum: f32) -> LinearRgba {
    if absf(wsum) < 1e-7 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let a = clamp01(acc[3] / wsum);
    if a <= 1e-6 {
        [0.0, 0.0, 0.0, 0.0]
    } else {
        let inv = 1.0 / (a * wsum);
        [
            (acc[0] * inv).max(0.0),
            (acc[1] * inv).max(0.0),
            (acc[2] * inv).max(0.0),
            a,
        ]
    }
}

/// Samples a `FrameBuffer` in linear light at source coordinates `(sx, sy)` where
/// pixel `(0, 0)` spans `[0..1, 0..1]` with center `(0.5, 0.5)`.
pub fn sample_linear(
    frame: &FrameBuffer,
    sx: f32,
    sy: f32,
    scale_x: f32,
    scale_y: f32,
    scaler: Scaler,
) -> LinearRgba {
    let mode = match scaler {
        Scaler::Auto => resolve_auto_scaler(scale_x, scale_y),
        other => other,
    };

    match mode {
        Scaler::Point => {
            let ix = floorf(sx) as i32;
            let iy = floorf(sy) as i32;
            frame.sample_clamped(ix, iy)
        }
        Scaler::Bilinear => {
            let fx = sx - 0.5;
            let fy = sy - 0.5;
            let x0 = floorf(fx) as i32;
            let y0 = floorf(fy) as i32;
            let tx = fx - x0 as f32;
            let ty = fy - y0 as f32;

            let p00 = to_premul(frame.sample_clamped(x0, y0));
            let p10 = to_premul(frame.sample_clamped(x0 + 1, y0));
            let p01 = to_premul(frame.sample_clamped(x0, y0 + 1));
            let p11 = to_premul(frame.sample_clamped(x0 + 1, y0 + 1));

            let mut acc = [0.0_f32; 4];
            for c in 0..4 {
                let top = lerp(p00[c], p10[c], tx);
                let bot = lerp(p01[c], p11[c], tx);
                acc[c] = lerp(top, bot, ty);
            }
            from_premul(acc, 1.0)
        }
        Scaler::Bicubic => sample_separable_kernel(frame, sx, sy, 2, bicubic_catmull_rom),
        Scaler::Lanczos3 => sample_separable_kernel(frame, sx, sy, 3, lanczos3_weight),
        Scaler::Area => {
            let inv_x = if absf(scale_x) > 1e-5 {
                (1.0 / absf(scale_x)).max(1.0)
            } else {
                1.0
            };
            let inv_y = if absf(scale_y) > 1e-5 {
                (1.0 / absf(scale_y)).max(1.0)
            } else {
                1.0
            };
            let half_w = 0.5 * inv_x;
            let half_h = 0.5 * inv_y;
            let x0 = floorf(sx - half_w) as i32;
            let x1 = floorf(sx + half_w - 1e-5) as i32;
            let y0 = floorf(sy - half_h) as i32;
            let y1 = floorf(sy + half_h - 1e-5) as i32;

            let mut acc = [0.0_f32; 4];
            let mut wsum = 0.0_f32;
            let rx0 = sx - half_w;
            let rx1 = sx + half_w;
            let ry0 = sy - half_h;
            let ry1 = sy + half_h;

            for iy in y0..=y1 {
                let oy = (ry1.min(iy as f32 + 1.0) - ry0.max(iy as f32)).max(0.0);
                if oy <= 0.0 {
                    continue;
                }
                for ix in x0..=x1 {
                    let ox = (rx1.min(ix as f32 + 1.0) - rx0.max(ix as f32)).max(0.0);
                    if ox <= 0.0 {
                        continue;
                    }
                    let w = ox * oy;
                    let p = to_premul(frame.sample_clamped(ix, iy));
                    for c in 0..4 {
                        acc[c] += p[c] * w;
                    }
                    wsum += w;
                }
            }
            if wsum <= 1e-7 {
                sample_linear(frame, sx, sy, scale_x, scale_y, Scaler::Bilinear)
            } else {
                from_premul(acc, wsum)
            }
        }
        Scaler::Auto => unreachable!(),
    }
}

fn sample_separable_kernel(
    frame: &FrameBuffer,
    sx: f32,
    sy: f32,
    radius: i32,
    kernel: fn(f32) -> f32,
) -> LinearRgba {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let cx = floorf(fx) as i32;
    let cy = floorf(fy) as i32;
    let tx = fx - cx as f32;
    let ty = fy - cy as f32;

    let mut acc = [0.0_f32; 4];
    let mut wsum = 0.0_f32;

    for dy in (1 - radius)..=radius {
        let wy = kernel(dy as f32 - ty);
        if absf(wy) < 1e-7 {
            continue;
        }
        for dx in (1 - radius)..=radius {
            let wx = kernel(dx as f32 - tx);
            let w = wx * wy;
            if absf(w) < 1e-7 {
                continue;
            }
            let p = to_premul(frame.sample_clamped(cx + dx, cy + dy));
            for c in 0..4 {
                acc[c] += p[c] * w;
            }
            wsum += w;
        }
    }
    from_premul(acc, wsum)
}

/// Resizes `frame` to `dst_w` x `dst_h` in linear light using the specified `Scaler`.
pub fn scale_frame(
    frame: &FrameBuffer,
    dst_w: u32,
    dst_h: u32,
    scaler: Scaler,
) -> Result<FrameBuffer, CompositorError> {
    let mut out = FrameBuffer::new(dst_w, dst_h)?;
    let sx_ratio = dst_w as f32 / frame.width as f32;
    let sy_ratio = dst_h as f32 / frame.height as f32;
    let effective = match scaler {
        Scaler::Auto => resolve_auto_scaler(sx_ratio, sy_ratio),
        other => other,
    };

    for y in 0..dst_h {
        let sy = (y as f32 + 0.5) / sy_ratio;
        for x in 0..dst_w {
            let sx = (x as f32 + 0.5) / sx_ratio;
            let p = sample_linear(frame, sx, sy, sx_ratio, sy_ratio, effective);
            out.set_pixel(x, y, p);
        }
    }
    Ok(out)
}

/// All 9 compositor blend modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlendMode {
    #[default]
    Normal,
    Additive,
    Subtract,
    Screen,
    Multiply,
    Lighten,
    Darken,
    LinearBurn,
    Overlay,
}

/// Color space used when mixing source opacity against the destination canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MixColorSpace {
    #[default]
    Linear,
    Srgb,
}

#[inline]
fn blend_channel(s: f32, d: f32, mode: BlendMode) -> f32 {
    match mode {
        BlendMode::Normal => s,
        BlendMode::Additive => s + d,
        BlendMode::Subtract => (d - s).max(0.0),
        BlendMode::Screen => s + d - s * d,
        BlendMode::Multiply => s * d,
        BlendMode::Lighten => s.max(d),
        BlendMode::Darken => s.min(d),
        BlendMode::LinearBurn => (s + d - 1.0).max(0.0),
        BlendMode::Overlay => {
            if d < 0.5 {
                2.0 * s * d
            } else {
                1.0 - 2.0 * (1.0 - s) * (1.0 - d)
            }
        }
    }
}

/// Blends straight RGBA `src` onto straight RGBA `dst` using Porter-Duff alpha
/// and the chosen `BlendMode`, `opacity`, and `MixColorSpace`.
pub fn blend_pixel(
    dst: LinearRgba,
    src: LinearRgba,
    mode: BlendMode,
    opacity: f32,
    mix_space: MixColorSpace,
) -> LinearRgba {
    let sa = clamp01(src[3] * clamp01(opacity));
    if sa <= 1e-6 {
        return dst;
    }
    let da = clamp01(dst[3]);

    let (sr, dr) = match mix_space {
        MixColorSpace::Linear => ([src[0], src[1], src[2]], [dst[0], dst[1], dst[2]]),
        MixColorSpace::Srgb => (
            [
                linear_to_srgb(src[0]),
                linear_to_srgb(src[1]),
                linear_to_srgb(src[2]),
            ],
            [
                linear_to_srgb(dst[0]),
                linear_to_srgb(dst[1]),
                linear_to_srgb(dst[2]),
            ],
        ),
    };

    let out_a = sa + da * (1.0 - sa);
    if out_a <= 1e-6 {
        return [0.0, 0.0, 0.0, 0.0];
    }

    let mut out_rgb = [0.0_f32; 3];
    for c in 0..3 {
        let b = blend_channel(sr[c], dr[c], mode);
        let premul = sa * (1.0 - da) * sr[c] + da * (1.0 - sa) * dr[c] + sa * da * b;
        let straight = premul / out_a;
        out_rgb[c] = match mix_space {
            MixColorSpace::Linear => straight.max(0.0),
            MixColorSpace::Srgb => srgb_to_linear(straight),
        };
    }

    [out_rgb[0], out_rgb[1], out_rgb[2], out_a]
}

/// Input descriptor for rendering a single layer into a canvas.
#[derive(Clone, Copy, Debug)]
pub struct CompositorLayer<'a> {
    pub frame: &'a FrameBuffer,
    pub transform: Transform2D,
    pub blend_mode: BlendMode,
    pub opacity: f32,
    pub mix_space: MixColorSpace,
    pub scaler: Scaler,
    pub visible: bool,
}

/// Linear-light FP32 compositor.
#[derive(Clone, Debug, Default)]
pub struct Compositor {
    pub background: LinearRgba,
}

impl Compositor {
    pub const fn new(background: LinearRgba) -> Self {
        Self { background }
    }

    /// Renders `layers` in bottom-to-top painter's order onto a fresh `(width, height)` canvas.
    pub fn composite(
        &self,
        width: u32,
        height: u32,
        layers: &[CompositorLayer<'_>],
    ) -> Result<FrameBuffer, CompositorError> {
        let mut canvas = FrameBuffer::filled(width, height, self.background)?;
        for layer in layers {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            self.draw_layer(&mut canvas, layer);
        }
        Ok(canvas)
    }

    /// Composites a single transformed layer directly onto `canvas`.
    pub fn draw_layer(&self, canvas: &mut FrameBuffer, layer: &CompositorLayer<'_>) {
        let res = layer
            .transform
            .resolve(layer.frame.width as f32, layer.frame.height as f32);
        if res.cropped_size[0] <= 0.0 || res.cropped_size[1] <= 0.0 {
            return;
        }
        if absf(res.effective_scale[0]) < 1e-6 || absf(res.effective_scale[1]) < 1e-6 {
            return;
        }

        let min_x = floorf(res.aabb.x).max(0.0) as u32;
        let min_y = floorf(res.aabb.y).max(0.0) as u32;
        let max_x = floorf(res.aabb.right() + 1.0).min(canvas.width as f32) as u32;
        let max_y = floorf(res.aabb.bottom() + 1.0).min(canvas.height as f32) as u32;
        if min_x >= max_x || min_y >= max_y {
            return;
        }

        let anc = layer.transform.anchor.normalized();
        let ax = anc[0] * res.box_size[0];
        let ay = anc[1] * res.box_size[1];
        let rad = layer.transform.rot_deg * (PI / 180.0);
        let c = cosf(rad);
        let s = sinf(rad);

        let crop_l = layer.transform.crop.left.max(0.0);
        let crop_t = layer.transform.crop.top.max(0.0);
        let cw = res.cropped_size[0];
        let ch = res.cropped_size[1];
        let inv_sx = 1.0 / res.effective_scale[0];
        let inv_sy = 1.0 / res.effective_scale[1];

        for y in min_y..max_y {
            let py = y as f32 + 0.5 - layer.transform.pos[1];
            for x in min_x..max_x {
                let px = x as f32 + 0.5 - layer.transform.pos[0];

                // Inverse rotation: rotate (px, py) by -rad, then add anchor (ax, ay).
                let bx = px * c + py * s + ax;
                let by = -px * s + py * c + ay;

                if res.clip_to_box
                    && (bx < 0.0 || bx >= res.box_size[0] || by < 0.0 || by >= res.box_size[1])
                {
                    continue;
                }

                let lx = (bx - res.inner_offset[0]) * absf(inv_sx);
                let ly = (by - res.inner_offset[1]) * absf(inv_sy);

                if lx < 0.0 || lx >= cw || ly < 0.0 || ly >= ch {
                    continue;
                }

                let u = if layer.transform.flip_h { cw - lx } else { lx };
                let v = if layer.transform.flip_v { ch - ly } else { ly };
                let src_x = crop_l + u;
                let src_y = crop_t + v;

                let sample = sample_linear(
                    layer.frame,
                    src_x,
                    src_y,
                    res.effective_scale[0],
                    res.effective_scale[1],
                    layer.scaler,
                );

                let dst = canvas.get_pixel(x, y).unwrap_or([0.0, 0.0, 0.0, 0.0]);
                let out = blend_pixel(dst, sample, layer.blend_mode, layer.opacity, layer.mix_space);
                canvas.set_pixel(x, y, out);
            }
        }
    }
}
