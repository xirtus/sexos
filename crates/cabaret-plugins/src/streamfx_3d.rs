//! Pure-Rust port of **StreamFX** 3D perspective, 4-point Corner-Pin homography,
//! multi-pass Dual-Kawase mip blur, and Euclidean-distance dynamic mask feathering.

use alloc::vec;
use alloc::vec::Vec;
use cabaret_core::compositor::{sample_linear, CompositorError, FrameBuffer, Scaler};

use crate::util::{absf, clamp01, cosf, deg_to_rad, lerp, powf, sinf, smoothstep, sqrtf, tanf};

/// Errors emitted by StreamFX 3D transforms, homography, and blur filters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamFxError {
    SingularHomography,
    BehindCamera,
    InvalidMaskLength { expected: usize, actual: usize },
    Compositor(CompositorError),
}

impl From<CompositorError> for StreamFxError {
    fn from(e: CompositorError) -> Self {
        Self::Compositor(e)
    }
}

/// 3x3 projective homography matrix stored in row-major `f64` precision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3(pub [[f64; 3]; 3]);

impl Mat3 {
    pub const IDENTITY: Self = Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    /// Multiplies `self` by homogeneous column vector `[x, y, z]`.
    #[inline]
    pub fn mul_vec3(self, v: [f64; 3]) -> [f64; 3] {
        [
            self.0[0][0] * v[0] + self.0[0][1] * v[1] + self.0[0][2] * v[2],
            self.0[1][0] * v[0] + self.0[1][1] * v[1] + self.0[1][2] * v[2],
            self.0[2][0] * v[0] + self.0[2][1] * v[1] + self.0[2][2] * v[2],
        ]
    }

    /// Projects a 2D point `(x, y)` through this homography, returning `None` if `w <= 1e-9`.
    #[inline]
    pub fn project_point(self, x: f64, y: f64) -> Option<[f64; 2]> {
        let [px, py, pw] = self.mul_vec3([x, y, 1.0]);
        if libm::fabs(pw) <= 1e-9 {
            None
        } else {
            Some([px / pw, py / pw])
        }
    }

    /// Exact 3x3 determinant.
    pub fn determinant(self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Exact 3x3 matrix inverse via adjugate / determinant.
    pub fn invert(self) -> Result<Self, StreamFxError> {
        let det = self.determinant();
        if libm::fabs(det) <= 1e-12 {
            return Err(StreamFxError::SingularHomography);
        }
        let m = &self.0;
        let inv_det = 1.0 / det;
        Ok(Self([
            [
                (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv_det,
                (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det,
                (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det,
            ],
            [
                (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv_det,
                (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det,
                (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det,
            ],
            [
                (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv_det,
                (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det,
                (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det,
            ],
        ]))
    }
}

/// 4-point Corner-Pin quad in clockwise order starting from top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornerPinQuad {
    pub top_left: [f32; 2],
    pub top_right: [f32; 2],
    pub bottom_right: [f32; 2],
    pub bottom_left: [f32; 2],
}

impl Default for CornerPinQuad {
    fn default() -> Self {
        Self::unit()
    }
}

impl CornerPinQuad {
    /// Unit normalized quad `[0..1] x [0..1]`.
    pub const fn unit() -> Self {
        Self {
            top_left: [0.0, 0.0],
            top_right: [1.0, 0.0],
            bottom_right: [1.0, 1.0],
            bottom_left: [0.0, 1.0],
        }
    }

    /// Pixel-space rectangle quad `[0..w] x [0..h]`.
    pub const fn from_rect(w: f32, h: f32) -> Self {
        Self {
            top_left: [0.0, 0.0],
            top_right: [w, 0.0],
            bottom_right: [w, h],
            bottom_left: [0.0, h],
        }
    }

    pub const fn corners(self) -> [[f32; 2]; 4] {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }

    /// Linearly interpolates each of the 4 corners at progress `t`.
    pub fn lerp(self, target: Self, t: f32) -> Self {
        Self {
            top_left: [
                lerp(self.top_left[0], target.top_left[0], t),
                lerp(self.top_left[1], target.top_left[1], t),
            ],
            top_right: [
                lerp(self.top_right[0], target.top_right[0], t),
                lerp(self.top_right[1], target.top_right[1], t),
            ],
            bottom_right: [
                lerp(self.bottom_right[0], target.bottom_right[0], t),
                lerp(self.bottom_right[1], target.bottom_right[1], t),
            ],
            bottom_left: [
                lerp(self.bottom_left[0], target.bottom_left[0], t),
                lerp(self.bottom_left[1], target.bottom_left[1], t),
            ],
        }
    }

    /// Normalizes pixel-space corners by `(1 / width, 1 / height)`.
    pub fn normalized(self, width: u32, height: u32) -> Self {
        let iw = 1.0 / (width.max(1) as f32);
        let ih = 1.0 / (height.max(1) as f32);
        Self {
            top_left: [self.top_left[0] * iw, self.top_left[1] * ih],
            top_right: [self.top_right[0] * iw, self.top_right[1] * ih],
            bottom_right: [self.bottom_right[0] * iw, self.bottom_right[1] * ih],
            bottom_left: [self.bottom_left[0] * iw, self.bottom_left[1] * ih],
        }
    }
}

/// Solves the exact 3x3 projective homography `H` mapping `src[0..4]` -> `dst[0..4]`
/// using 8x8 Gaussian elimination with partial pivoting in `f64`.
pub fn solve_homography_4pt(
    src: [[f32; 2]; 4],
    dst: [[f32; 2]; 4],
) -> Result<Mat3, StreamFxError> {
    let mut a = [[0.0_f64; 9]; 8];
    for i in 0..4 {
        let sx = src[i][0] as f64;
        let sy = src[i][1] as f64;
        let dx = dst[i][0] as f64;
        let dy = dst[i][1] as f64;

        let r0 = i * 2;
        a[r0][0] = sx;
        a[r0][1] = sy;
        a[r0][2] = 1.0;
        a[r0][6] = -sx * dx;
        a[r0][7] = -sy * dx;
        a[r0][8] = dx;

        let r1 = r0 + 1;
        a[r1][3] = sx;
        a[r1][4] = sy;
        a[r1][5] = 1.0;
        a[r1][6] = -sx * dy;
        a[r1][7] = -sy * dy;
        a[r1][8] = dy;
    }

    // Gaussian elimination with partial pivoting.
    for col in 0..8 {
        let mut pivot_row = col;
        let mut max_val = libm::fabs(a[col][col]);
        for row in (col + 1)..8 {
            let v = libm::fabs(a[row][col]);
            if v > max_val {
                max_val = v;
                pivot_row = row;
            }
        }
        if max_val <= 1e-12 {
            return Err(StreamFxError::SingularHomography);
        }
        if pivot_row != col {
            a.swap(col, pivot_row);
        }
        let piv = a[col][col];
        for c in col..9 {
            a[col][c] /= piv;
        }
        for row in 0..8 {
            if row == col {
                continue;
            }
            let factor = a[row][col];
            if factor != 0.0 {
                for c in col..9 {
                    a[row][c] -= factor * a[col][c];
                }
            }
        }
    }

    let h = Mat3([
        [a[0][8], a[1][8], a[2][8]],
        [a[3][8], a[4][8], a[5][8]],
        [a[6][8], a[7][8], 1.0],
    ]);
    // Verify invertibility as well.
    let _ = h.invert()?;
    Ok(h)
}

/// Warps `src` into an `(out_w, out_h)` linear-light `FrameBuffer` mapped by a normalized
/// [`CornerPinQuad`] (`[0..1]^2`) using exact inverse homography + bilinear linear-light sampling.
pub fn warp_corner_pin_normalized(
    src: &FrameBuffer,
    quad: &CornerPinQuad,
    out_w: u32,
    out_h: u32,
) -> Result<FrameBuffer, StreamFxError> {
    let mut out = FrameBuffer::new(out_w, out_h)?;
    let unit = CornerPinQuad::unit();
    let h = solve_homography_4pt(unit.corners(), quad.corners())?;
    let h_inv = h.invert()?;

    let ow = out_w as f64;
    let oh = out_h as f64;
    let sw = src.width as f32;
    let sh = src.height as f32;

    for py in 0..out_h {
        let dy = (py as f64 + 0.5) / oh;
        for px in 0..out_w {
            let dx = (px as f64 + 0.5) / ow;
            let [hx, hy, hw] = h_inv.mul_vec3([dx, dy, 1.0]);
            if hw <= 1e-9 {
                continue;
            }
            let u = hx / hw;
            let v = hy / hw;
            if u >= 0.0 && u <= 1.0 && v >= 0.0 && v <= 1.0 {
                let sample = sample_linear(
                    src,
                    (u as f32) * sw,
                    (v as f32) * sh,
                    1.0,
                    1.0,
                    Scaler::Bilinear,
                );
                out.set_pixel(px, py, sample);
            }
        }
    }
    Ok(out)
}

/// Convenience wrapper accepting pixel-space corners on `(out_w, out_h)`.
pub fn warp_corner_pin_pixels(
    src: &FrameBuffer,
    quad_px: &CornerPinQuad,
    out_w: u32,
    out_h: u32,
) -> Result<FrameBuffer, StreamFxError> {
    if out_w == 0 || out_h == 0 {
        return Err(StreamFxError::Compositor(CompositorError::ZeroDimensions));
    }
    let norm = quad_px.normalized(out_w, out_h);
    warp_corner_pin_normalized(src, &norm, out_w, out_h)
}

/// 3D perspective camera transform parameters (`PitchYawRollFov`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PitchYawRollFov {
    pub pitch_deg: f32,
    pub yaw_deg: f32,
    pub roll_deg: f32,
    pub fov_deg: f32,
    pub z_distance: f32,
    pub center: [f32; 2],
}

impl Default for PitchYawRollFov {
    fn default() -> Self {
        Self {
            pitch_deg: 0.0,
            yaw_deg: 0.0,
            roll_deg: 0.0,
            fov_deg: 60.0,
            z_distance: 0.0,
            center: [0.5, 0.5],
        }
    }
}

impl PitchYawRollFov {
    /// Projects the source plane's 4 corners through a 3D Euler rotation + pinhole camera
    /// into normalized `[0..1]^2` [`CornerPinQuad`] coordinates.
    pub fn project_quad(&self, aspect: f32) -> Result<CornerPinQuad, StreamFxError> {
        let asp = aspect.max(1e-4) as f64;
        let fov_rad = deg_to_rad(self.fov_deg.clamp(1.0, 170.0)) as f64;
        let focal = 0.5 / libm::tan(fov_rad * 0.5);

        let rx = deg_to_rad(self.pitch_deg) as f64;
        let ry = deg_to_rad(self.yaw_deg) as f64;
        let rz = deg_to_rad(self.roll_deg) as f64;

        let (sx, cx) = (libm::sin(rx), libm::cos(rx));
        let (sy, cy) = (libm::sin(ry), libm::cos(ry));
        let (sz, cz) = (libm::sin(rz), libm::cos(rz));

        let local_corners = [
            [-0.5 * asp, -0.5, 0.0],
            [0.5 * asp, -0.5, 0.0],
            [0.5 * asp, 0.5, 0.0],
            [-0.5 * asp, 0.5, 0.0],
        ];

        let mut projected = [[0.0_f32; 2]; 4];
        for (i, [x0, y0, z0]) in local_corners.into_iter().enumerate() {
            // Rotate X (pitch)
            let y1 = y0 * cx - z0 * sx;
            let z1 = y0 * sx + z0 * cx;
            let x1 = x0;
            // Rotate Y (yaw)
            let x2 = x1 * cy + z1 * sy;
            let z2 = -x1 * sy + z1 * cy;
            let y2 = y1;
            // Rotate Z (roll)
            let x3 = x2 * cz - y2 * sz;
            let y3 = x2 * sz + y2 * cz;
            let z3 = z2;

            let depth = focal + (self.z_distance as f64) + z3;
            if depth <= 1e-4 {
                return Err(StreamFxError::BehindCamera);
            }
            let nx = self.center[0] as f64 + (x3 * focal) / (asp * depth);
            let ny = self.center[1] as f64 + (y3 * focal) / depth;
            projected[i] = [nx as f32, ny as f32];
        }

        Ok(CornerPinQuad {
            top_left: projected[0],
            top_right: projected[1],
            bottom_right: projected[2],
            bottom_left: projected[3],
        })
    }
}

/// Applies 3D perspective (`PitchYawRollFov`) to `src` into an `(out_w, out_h)` `FrameBuffer`.
pub fn apply_perspective_3d(
    src: &FrameBuffer,
    params: &PitchYawRollFov,
    out_w: u32,
    out_h: u32,
) -> Result<FrameBuffer, StreamFxError> {
    if out_w == 0 || out_h == 0 {
        return Err(StreamFxError::Compositor(CompositorError::ZeroDimensions));
    }
    let aspect = out_w as f32 / out_h as f32;
    let quad = params.project_quad(aspect)?;
    warp_corner_pin_normalized(src, &quad, out_w, out_h)
}

/// Configuration for multi-pass Dual-Kawase pyramid blur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DualKawaseBlur {
    pub passes: u8,
    pub offset: f32,
}

impl Default for DualKawaseBlur {
    fn default() -> Self {
        Self {
            passes: 3,
            offset: 1.0,
        }
    }
}

#[inline]
fn sample_bilinear_premul(fb: &FrameBuffer, u: f32, v: f32) -> [f32; 4] {
    let fx = u * fb.width as f32 - 0.5;
    let fy = v * fb.height as f32 - 0.5;
    let x0 = libm::floorf(fx) as i32;
    let y0 = libm::floorf(fy) as i32;
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;

    let p00 = fb.sample_clamped(x0, y0);
    let p10 = fb.sample_clamped(x0 + 1, y0);
    let p01 = fb.sample_clamped(x0, y0 + 1);
    let p11 = fb.sample_clamped(x0 + 1, y0 + 1);

    let mut out = [0.0_f32; 4];
    for c in 0..4 {
        let top = lerp(p00[c], p10[c], tx);
        let bot = lerp(p01[c], p11[c], tx);
        out[c] = lerp(top, bot, ty);
    }
    out
}

/// Multi-pass Dual-Kawase mip-chain blur operating in premultiplied linear light.
pub fn dual_kawase_blur(
    src: &FrameBuffer,
    config: DualKawaseBlur,
) -> Result<FrameBuffer, StreamFxError> {
    let passes = config.passes.clamp(0, 8);
    if passes == 0 || config.offset <= 0.0 {
        return Ok(src.clone());
    }

    // Premultiply once at mip 0.
    let mut mip0 = src.clone();
    for p in &mut mip0.pixels {
        let a = clamp01(p[3]);
        *p = [p[0] * a, p[1] * a, p[2] * a, a];
    }

    let mut mips = Vec::with_capacity(passes as usize + 1);
    mips.push(mip0);

    let off = config.offset.max(0.25);

    // Downsample pyramid.
    for _ in 0..passes {
        let prev = mips.last().expect("mip0 present");
        let nw = (prev.width / 2).max(1);
        let nh = (prev.height / 2).max(1);
        let dx = off / prev.width as f32;
        let dy = off / prev.height as f32;

        let mut next = FrameBuffer::new(nw, nh)?;
        for y in 0..nh {
            let v = (y as f32 + 0.5) / nh as f32;
            for x in 0..nw {
                let u = (x as f32 + 0.5) / nw as f32;
                let c = sample_bilinear_premul(prev, u, v);
                let tl = sample_bilinear_premul(prev, u - dx, v - dy);
                let tr = sample_bilinear_premul(prev, u + dx, v - dy);
                let bl = sample_bilinear_premul(prev, u - dx, v + dy);
                let br = sample_bilinear_premul(prev, u + dx, v + dy);
                let mut acc = [0.0_f32; 4];
                for ch in 0..4 {
                    acc[ch] = (4.0 * c[ch] + tl[ch] + tr[ch] + bl[ch] + br[ch]) * (1.0 / 8.0);
                }
                next.set_pixel(x, y, acc);
            }
        }
        mips.push(next);
    }

    // Upsample pyramid.
    let mut cur = mips.pop().expect("non-empty mips");
    while let Some(target_level) = mips.pop() {
        let tw = target_level.width;
        let th = target_level.height;
        let dx = 0.5 * off / tw as f32;
        let dy = 0.5 * off / th as f32;
        let mut up = FrameBuffer::new(tw, th)?;

        for y in 0..th {
            let v = (y as f32 + 0.5) / th as f32;
            for x in 0..tw {
                let u = (x as f32 + 0.5) / tw as f32;
                let l = sample_bilinear_premul(&cur, u - 2.0 * dx, v);
                let r = sample_bilinear_premul(&cur, u + 2.0 * dx, v);
                let t = sample_bilinear_premul(&cur, u, v - 2.0 * dy);
                let b = sample_bilinear_premul(&cur, u, v + 2.0 * dy);
                let tl = sample_bilinear_premul(&cur, u - dx, v - dy);
                let tr = sample_bilinear_premul(&cur, u + dx, v - dy);
                let bl = sample_bilinear_premul(&cur, u - dx, v + dy);
                let br = sample_bilinear_premul(&cur, u + dx, v + dy);

                let mut acc = [0.0_f32; 4];
                for ch in 0..4 {
                    acc[ch] = (l[ch] + r[ch] + t[ch] + b[ch]
                        + 2.0 * (tl[ch] + tr[ch] + bl[ch] + br[ch]))
                        * (1.0 / 12.0);
                }
                up.set_pixel(x, y, acc);
            }
        }
        cur = up;
    }

    // Unpremultiply back to straight linear RGBA.
    for p in &mut cur.pixels {
        let a = clamp01(p[3]);
        if a <= 1e-6 {
            *p = [0.0, 0.0, 0.0, 0.0];
        } else {
            let inv = 1.0 / a;
            *p = [(p[0] * inv).max(0.0), (p[1] * inv).max(0.0), (p[2] * inv).max(0.0), a];
        }
    }
    Ok(cur)
}

/// Configuration for dynamic Euclidean-distance mask feathering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DynamicMaskFeather {
    pub feather_radius_px: f32,
    pub expand_px: f32,
    pub falloff_gamma: f32,
    pub invert: bool,
}

impl Default for DynamicMaskFeather {
    fn default() -> Self {
        Self {
            feather_radius_px: 8.0,
            expand_px: 0.0,
            falloff_gamma: 1.0,
            invert: false,
        }
    }
}

/// 1D squared Euclidean Distance Transform (Felzenszwalb & Huttenlocher lower envelope).
fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k: usize = 0;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;

    for q in 1..n {
        let fq = f[q];
        let qf = q as f32;
        let mut s = loop {
            let vk = v[k];
            let vkf = vk as f32;
            let inter = ((fq + qf * qf) - (f[vk] + vkf * vkf)) / (2.0 * (qf - vkf));
            if inter > z[k] || k == 0 {
                break inter;
            }
            k -= 1;
        };
        if s <= z[0] {
            s = z[0];
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::INFINITY;
    }

    let mut k_scan = 0usize;
    for q in 0..n {
        let qf = q as f32;
        while z[k_scan + 1] < qf {
            k_scan += 1;
        }
        let dx = qf - v[k_scan] as f32;
        d[q] = dx * dx + f[v[k_scan]];
    }
}

/// Computes exact 2D Euclidean distance to `target_val` pixels in `grid`.
fn euclidean_distance_2d(binary_fg: &[bool], w: usize, h: usize, measure_to_fg: bool) -> Vec<f32> {
    const INF: f32 = 1e12;
    let n = w * h;
    let mut grid = vec![INF; n];
    for (i, &is_fg) in binary_fg.iter().enumerate() {
        if is_fg == measure_to_fg {
            grid[i] = 0.0;
        }
    }

    let max_dim = w.max(h);
    let mut f_col = vec![0.0_f32; max_dim];
    let mut d_col = vec![0.0_f32; max_dim];
    let mut v = vec![0usize; max_dim];
    let mut z = vec![0.0_f32; max_dim + 1];

    // Transform columns.
    for x in 0..w {
        for y in 0..h {
            f_col[y] = grid[y * w + x];
        }
        edt_1d(&f_col[..h], &mut d_col[..h], &mut v, &mut z);
        for y in 0..h {
            grid[y * w + x] = d_col[y];
        }
    }

    // Transform rows.
    for y in 0..h {
        let row = y * w;
        f_col[..w].copy_from_slice(&grid[row..row + w]);
        edt_1d(&f_col[..w], &mut d_col[..w], &mut v, &mut z);
        for x in 0..w {
            grid[row + x] = sqrtf(d_col[x]);
        }
    }

    grid
}

/// Applies dynamic Euclidean-distance mask feathering + expansion/contraction to `src`
/// using per-pixel `mask` values in `0.0..=1.0`.
pub fn apply_dynamic_mask_feather(
    src: &FrameBuffer,
    mask: &[f32],
    config: DynamicMaskFeather,
) -> Result<FrameBuffer, StreamFxError> {
    let w = src.width as usize;
    let h = src.height as usize;
    let expected = w * h;
    if mask.len() != expected {
        return Err(StreamFxError::InvalidMaskLength {
            expected,
            actual: mask.len(),
        });
    }

    let binary_fg: Vec<bool> = mask.iter().map(|&m| m >= 0.5).collect();
    let dist_to_fg = euclidean_distance_2d(&binary_fg, w, h, true);
    let dist_to_bg = euclidean_distance_2d(&binary_fg, w, h, false);

    let radius = config.feather_radius_px.max(0.0);
    let half_r = radius * 0.5;
    let gamma = config.falloff_gamma.max(0.05);

    let mut out = src.clone();
    for i in 0..expected {
        // Signed distance: negative inside foreground, positive outside.
        let sdf = if binary_fg[i] {
            -dist_to_bg[i]
        } else {
            dist_to_fg[i]
        };
        let shifted = sdf - config.expand_px;
        let mut alpha = if half_r <= 1e-4 {
            if shifted <= 0.0 {
                1.0
            } else {
                0.0
            }
        } else {
            // 1.0 deep inside, 0.0 outside.
            1.0 - smoothstep(-half_r, half_r, shifted)
        };
        if absf(gamma - 1.0) > 1e-4 {
            alpha = powf(clamp01(alpha), gamma);
        }
        if config.invert {
            alpha = 1.0 - alpha;
        }
        let orig_a = clamp01(out.pixels[i][3]);
        out.pixels[i][3] = clamp01(orig_a * alpha);
    }

    // Also include helper so sinf/cosf stay cleanly utilized even if called selectively.
    let _ = (sinf(0.0), cosf(0.0), tanf(0.0));
    Ok(out)
}

/// Applies dynamic mask feathering using another `FrameBuffer`'s alpha channel as the mask.
pub fn apply_dynamic_mask_feather_frame(
    src: &FrameBuffer,
    mask_frame: &FrameBuffer,
    config: DynamicMaskFeather,
) -> Result<FrameBuffer, StreamFxError> {
    let w = src.width;
    let h = src.height;
    let mut mask = Vec::with_capacity((w as usize) * (h as usize));
    for y in 0..h {
        let my = ((y as u64 * mask_frame.height as u64) / h as u64) as u32;
        for x in 0..w {
            let mx = ((x as u64 * mask_frame.width as u64) / w as u64) as u32;
            let a = mask_frame
                .get_pixel(mx, my)
                .map(|p| clamp01(p[3]))
                .unwrap_or(0.0);
            mask.push(a);
        }
    }
    apply_dynamic_mask_feather(src, &mask, config)
}
