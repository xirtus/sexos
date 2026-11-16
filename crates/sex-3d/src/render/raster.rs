// SPDX-License-Identifier: GPL-3.0-only
//! Tiled triangle rasterization shared by the Workbench (Solid) and EEVEE
//! style engines and their shadow maps.
//!
//! - **Layout**: frame buffers are stored tile-major ([`TILE`] x [`TILE`]
//!   pixels per tile, tiles row by row), so every tile is one contiguous
//!   slice that one worker owns while rasterizing (a per-tile z-buffer that
//!   stays in cache), and later passes read any pixel through
//!   [`Layout::index`].
//! - **Setup** ([`Piece::push`]): projected triangles get integer edge
//!   functions in 24.8 fixed point (exact, with the top-left fill rule, so
//!   shared edges are filled exactly once whatever the order), a guard band
//!   (triangles reaching far off screen are clipped to it in 2D), and affine
//!   screen-space planes for the depth key, `1/w` and the perspective-correct
//!   barycentrics of the *source* triangle (near-clipped pieces carry their
//!   corners' source barycentrics, so a pixel always maps back to the
//!   original triangle).
//! - **Binning**: each setup piece bins its triangles by the tiles their
//!   bounds touch. A tile visits the pieces in order and each piece's
//!   triangles in order, so the result never depends on how the work was
//!   split or on the thread count.
//! - **Raster** ([`raster_tile`]): per tile, edge functions stepped with
//!   integer adds (vectorizable, no float compares), whole-tile rejection
//!   against each edge, and a caller-supplied fragment closure (depth test
//!   and ID write for the visibility buffer, depth only for shadow maps, or
//!   blending for transparency and X-ray).
use super::parallel_for_each;
use super::viewport::ViewCamera;
use crate::math::{Mat4, Vec3};
use crate::scene::Camera;

/// Tile size in pixels (each side).
pub(crate) const TILE: usize = 32;
/// Pixels per tile.
pub(crate) const TILE_PX: usize = TILE * TILE;
const SUB_BITS: u32 = 8;
const SUB: f32 = (1 << SUB_BITS) as f32;
/// Screen positions are clipped to +-GUARD pixels before going fixed point.
const GUARD: f32 = (1 << 19) as f32;
/// Triangle ids: piece index in the top bits.
const LOCAL_BITS: u32 = 24;
pub(crate) const NO_ID: u32 = u32::MAX;

/// A projected vertex: pixel position (y down), depth key (larger is
/// closer, affine in screen space) and 1/w (1 for orthographic).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SVert {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub iw: f32,
}

/// `floor` without a libm call (baseline x86-64 has no SSE4.1 rounding,
/// so `f32::floor` is a function call in the hot loops). Saturates at
/// +-1e9; NaN gives 0.
#[inline(always)]
pub(crate) fn ffloor(x: f32) -> f32 {
    let c = x.clamp(-1e9, 1e9);
    let i = c as i64 as f32;
    if i > c { i - 1.0 } else { i }
}

#[inline(always)]
pub(crate) fn fceil(x: f32) -> f32 {
    -ffloor(-x)
}

/// Round half away from zero (like `f32::round`) to an integer.
#[inline(always)]
fn fround(x: f32) -> i64 {
    (x + if x >= 0.0 { 0.5 } else { -0.5 }) as i64
}

/// A pinhole (perspective or orthographic) camera mapping world space to
/// pixels of a `width` x `height` frame: the 3D view's orbit camera or a
/// scene camera, optionally with a sub-pixel jitter (anti-aliasing).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterCamera {
    pub eye: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// Unit vector from the scene towards the camera (it looks along -back).
    pub back: Vec3,
    pub ortho: bool,
    /// Pixels per unit of x/(-z) (perspective) or per world unit (ortho).
    pub scale: f32,
    /// Pixel position of the view axis.
    pub cx: f32,
    pub cy: f32,
    /// Near clip distance (perspective).
    pub near: f32,
    pub width: u32,
    pub height: u32,
}

impl RasterCamera {
    /// The 3D view's orbit camera.
    pub fn from_view(v: &ViewCamera, w: u32, h: u32) -> RasterCamera {
        let t = (v.fov_y.clamp(1e-3, 3.0) * 0.5).tan();
        let half_h = h as f32 * 0.5;
        let scale = if v.ortho { half_h / (v.distance * t) } else { half_h / t };
        RasterCamera { eye: v.eye(), right: v.right(), up: v.up(), back: v.back(), ortho: v.ortho, scale, cx: w as f32 * 0.5, cy: half_h, near: (v.distance * 1e-3).max(1e-4), width: w, height: h }
    }

    /// A scene camera with world matrix `m` (it looks down its local -Z),
    /// with lens shift and orthographic scale; panoramic lenses fall back
    /// to perspective.
    pub fn from_camera(m: &Mat4, cam: &Camera, w: u32, h: u32) -> RasterCamera {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let half_h = h as f32 * 0.5;
        let aspect = w as f32 / h.max(1) as f32;
        let big = aspect.max(1.0);
        let (ortho, scale) = match cam.ortho_scale.filter(|s| *s > 0.0 && cam.panorama.is_none()) {
            Some(s) => (true, half_h / (s * 0.5)),
            None => (false, half_h / (fin(cam.fov_y, 0.8).clamp(1e-4, 3.1) * 0.5).tan()),
        };
        RasterCamera {
            eye: m.transform_point(Vec3::ZERO),
            right: m.transform_vector(Vec3::X).normalize(),
            up: m.transform_vector(Vec3::Y).normalize(),
            back: m.transform_vector(Vec3::Z).normalize(),
            ortho,
            scale,
            cx: w as f32 * 0.5 - fin(cam.shift_x, 0.0) * big * h as f32,
            cy: half_h + fin(cam.shift_y, 0.0) * big * h as f32,
            near: fin(cam.near, 0.1).max(1e-4),
            width: w,
            height: h,
        }
    }

    /// Shifted by (`jx`, `jy`) pixels.
    pub fn jittered(mut self, jx: f32, jy: f32) -> RasterCamera {
        self.cx += jx;
        self.cy += jy;
        self
    }

    #[inline(always)]
    pub fn to_view(&self, p: Vec3) -> Vec3 {
        let d = p - self.eye;
        Vec3::new(d.dot(self.right), d.dot(self.up), d.dot(self.back))
    }
    #[inline(always)]
    pub fn rotate(&self, n: Vec3) -> Vec3 {
        Vec3::new(n.dot(self.right), n.dot(self.up), n.dot(self.back))
    }
    /// View space to world space.
    #[inline(always)]
    pub fn to_world(&self, v: Vec3) -> Vec3 {
        self.eye + self.right * v.x + self.up * v.y + self.back * v.z
    }
    /// View-space z of the near plane (negative); everything is in front in ortho.
    pub fn near_z(&self) -> f32 {
        if self.ortho { f32::INFINITY } else { -self.near }
    }
    /// Pixel position, depth key and 1/w of a view-space point in front.
    #[inline(always)]
    pub fn project(&self, v: Vec3) -> Option<(f32, f32, f32)> {
        if self.ortho {
            return Some((self.cx + v.x * self.scale, self.cy - v.y * self.scale, v.z));
        }
        if v.z > -self.near {
            return None;
        }
        let iw = -1.0 / v.z;
        Some((self.cx + v.x * iw * self.scale, self.cy - v.y * iw * self.scale, iw))
    }
    #[inline(always)]
    pub(crate) fn svert(&self, v: Vec3) -> SVert {
        let (x, y, z) = self.project(v).unwrap_or((0.0, 0.0, 0.0));
        SVert { x, y, z, iw: self.inv_w(v) }
    }
    /// 1/w for perspective-correct interpolation (1 in ortho).
    #[inline(always)]
    pub fn inv_w(&self, v: Vec3) -> f32 {
        if self.ortho { 1.0 } else { -1.0 / v.z }
    }
    /// The view-space point at the centre of pixel (`x`, `y`) with depth key `key`.
    #[inline(always)]
    pub fn unproject(&self, x: usize, y: usize, key: f32) -> Vec3 {
        let (px, py) = (x as f32 + 0.5 - self.cx, self.cy - (y as f32 + 0.5));
        if self.ortho {
            Vec3::new(px / self.scale, py / self.scale, key)
        } else {
            let k = 1.0 / (self.scale * key);
            Vec3::new(px * k, py * k, -1.0 / key)
        }
    }
    /// Unit view-space direction from the eye through pixel (`x`, `y`)
    /// (the view axis in ortho).
    #[inline(always)]
    pub fn view_dir(&self, x: usize, y: usize) -> Vec3 {
        if self.ortho {
            return -Vec3::Z;
        }
        let (px, py) = (x as f32 + 0.5 - self.cx, self.cy - (y as f32 + 0.5));
        Vec3::new(px / self.scale, py / self.scale, -1.0).normalize()
    }
    /// Distance along the view axis for a depth key.
    #[inline(always)]
    pub fn depth_of(&self, key: f32) -> f32 {
        if self.ortho { -key } else { 1.0 / key }
    }
    /// Clip a view-space triangle against the near plane and hand the
    /// projected pieces to `emit` with their corners' source barycentrics.
    #[inline]
    pub(crate) fn clip_emit(&self, p: [Vec3; 3], mut emit: impl FnMut([SVert; 3], [[f32; 2]; 3])) {
        const B: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let near = self.near_z();
        let inside = p.map(|q| q.z <= near);
        if inside[0] && inside[1] && inside[2] {
            emit(p.map(|q| self.svert(q)), B);
            return;
        }
        if !(inside[0] || inside[1] || inside[2]) {
            return;
        }
        let mut poly = [(Vec3::ZERO, [0.0f32; 2]); 4];
        let mut n = 0;
        for k in 0..3 {
            let (a, b) = ((p[k], B[k]), (p[(k + 1) % 3], B[(k + 1) % 3]));
            let (ia, ib) = (inside[k], inside[(k + 1) % 3]);
            if ia {
                poly[n] = a;
                n += 1;
            }
            if ia != ib {
                let s = (near - a.0.z) / (b.0.z - a.0.z);
                poly[n] = (a.0.lerp(b.0, s), [a.1[0] + (b.1[0] - a.1[0]) * s, a.1[1] + (b.1[1] - a.1[1]) * s]);
                n += 1;
            }
        }
        for k in 1..n.saturating_sub(1) {
            let (a, b, c) = (poly[0], poly[k], poly[k + 1]);
            emit([self.svert(a.0), self.svert(b.0), self.svert(c.0)], [a.1, b.1, c.1]);
        }
    }
    /// Whether a world-space box can be seen (conservative frustum test).
    pub fn sees_box(&self, min: Vec3, max: Vec3) -> bool {
        let corners: [Vec3; 8] = core::array::from_fn(|c| self.to_view(Vec3::new(if c & 1 == 0 { min.x } else { max.x }, if c & 2 == 0 { min.y } else { max.y }, if c & 4 == 0 { min.z } else { max.z })));
        let (w, h) = (self.width as f32, self.height as f32);
        // Planes as "outside" tests on view-space points.
        let out = |f: &dyn Fn(Vec3) -> bool| corners.iter().all(|&v| f(v));
        if self.ortho {
            let sx = |v: Vec3| self.cx + v.x * self.scale;
            let sy = |v: Vec3| self.cy - v.y * self.scale;
            return !(out(&|v| sx(v) < 0.0) || out(&|v| sx(v) > w) || out(&|v| sy(v) < 0.0) || out(&|v| sy(v) > h));
        }
        // x_pix = cx + x * scale / (-z): left of 0 when x * scale < -cx * (-z).
        let near = -self.near;
        !(out(&|v| v.z > near)
            || out(&|v| v.x * self.scale < -self.cx * -v.z)
            || out(&|v| v.x * self.scale > (w - self.cx) * -v.z)
            || out(&|v| -v.y * self.scale < -self.cy * -v.z)
            || out(&|v| -v.y * self.scale > (h - self.cy) * -v.z))
    }
}

/// Tile-major frame layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Layout {
    pub w: usize,
    pub h: usize,
    pub tx: usize,
    pub ty: usize,
}

impl Layout {
    pub fn new(w: usize, h: usize) -> Layout {
        Layout { w, h, tx: w.div_ceil(TILE), ty: h.div_ceil(TILE) }
    }
    pub fn tiles(&self) -> usize {
        self.tx * self.ty
    }
    /// Length of a buffer in this layout (whole tiles).
    pub fn len(&self) -> usize {
        self.tiles() * TILE_PX
    }
    #[inline(always)]
    pub fn index(&self, x: usize, y: usize) -> usize {
        ((y / TILE) * self.tx + x / TILE) * TILE_PX + (y % TILE) * TILE + x % TILE
    }
    /// Pixel rectangle of tile `t`: x0, y0, x1, y1 (exclusive), inside the frame.
    pub fn tile_rect(&self, t: usize) -> (usize, usize, usize, usize) {
        let (x0, y0) = ((t % self.tx) * TILE, (t / self.tx) * TILE);
        (x0, y0, (x0 + TILE).min(self.w), (y0 + TILE).min(self.h))
    }
}

/// An affine function of the pixel position, evaluated at pixel centres:
/// `a * x + b * y + c` for integer pixel coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Plane {
    pub a: f32,
    pub b: f32,
    pub c: f32,
}

impl Plane {
    #[inline(always)]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.a * x as f32 + self.b * y as f32 + self.c
    }
    /// Through values `f` at the corners `v` (pixel space), for pixel centres.
    fn new(v: &[SVert; 3], f: [f32; 3], inv_det: f32) -> Plane {
        let (dx1, dy1, dx2, dy2) = (v[1].x - v[0].x, v[1].y - v[0].y, v[2].x - v[0].x, v[2].y - v[0].y);
        let (df1, df2) = (f[1] - f[0], f[2] - f[0]);
        let a = (df1 * dy2 - df2 * dy1) * inv_det;
        let b = (df2 * dx1 - df1 * dx2) * inv_det;
        // Shift by half a pixel so integer coordinates mean pixel centres.
        let c = f[0] - a * (v[0].x - 0.5) - b * (v[0].y - 0.5);
        Plane { a, b, c }
    }
}

/// A triangle ready to rasterize.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SetupTri {
    /// Edge functions at the centre of pixel (bx0, by0), with the fill-rule bias.
    e0: [i64; 3],
    /// Steps per pixel in x and y.
    dx: [i64; 3],
    dy: [i64; 3],
    /// Inclusive pixel bounds.
    bx0: i32,
    by0: i32,
    bx1: i32,
    by1: i32,
    /// Depth key.
    pub z: Plane,
    /// Sum of `l_k / w_k` (perspective-correct interpolation denominator).
    pub w: Plane,
    /// Numerators of the source triangle's barycentrics 1 and 2.
    pub u: Plane,
    pub v: Plane,
}

impl SetupTri {
    /// Perspective-correct source barycentrics (b0, b1, b2) at a pixel.
    #[inline(always)]
    pub fn bary(&self, x: usize, y: usize) -> [f32; 3] {
        let w = self.w.at(x, y);
        let inv = if w != 0.0 { 1.0 / w } else { 0.0 };
        let (b1, b2) = (self.u.at(x, y) * inv, self.v.at(x, y) * inv);
        [1.0 - b1 - b2, b1, b2]
    }
}

/// Which screen winding to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum Facing {
    Both,
    /// Positive signed area in pixel space (y down), i.e. clockwise on screen.
    Positive,
    Negative,
}

/// One setup worker's output: triangles, their payloads and per-tile bins.
#[derive(Debug)]
pub(crate) struct Piece<P> {
    pub tris: Vec<SetupTri>,
    pub data: Vec<P>,
    bins: Vec<Vec<u32>>,
    layout: Layout,
}

impl<P> Default for Piece<P> {
    fn default() -> Self {
        Piece { tris: Vec::new(), data: Vec::new(), bins: Vec::new(), layout: Layout::default() }
    }
}

impl<P: Copy> Piece<P> {
    fn reset(&mut self, layout: Layout) {
        self.tris.clear();
        self.data.clear();
        self.layout = layout;
        self.bins.resize_with(layout.tiles(), Vec::new);
        for b in &mut self.bins {
            b.clear();
        }
    }

    /// Add a triangle of the source triangle (corners with their source
    /// barycentrics). Returns false when it covers no pixel centre.
    pub fn push(&mut self, v: [SVert; 3], bary: [[f32; 2]; 3], facing: Facing, data: P) -> bool {
        let area = (v[1].x - v[0].x) * (v[2].y - v[0].y) - (v[2].x - v[0].x) * (v[1].y - v[0].y);
        if !(area.abs() > 1e-12) {
            return false;
        }
        match facing {
            Facing::Positive if area < 0.0 => return false,
            Facing::Negative if area > 0.0 => return false,
            _ => {}
        }
        let (xmin, xmax) = (v[0].x.min(v[1].x).min(v[2].x), v[0].x.max(v[1].x).max(v[2].x));
        let (ymin, ymax) = (v[0].y.min(v[1].y).min(v[2].y), v[0].y.max(v[1].y).max(v[2].y));
        let (w, h) = (self.layout.w as f32, self.layout.h as f32);
        if !(xmax >= 0.0 && ymax >= 0.0 && xmin <= w && ymin <= h) {
            return false;
        }
        // Pixels whose centres the bounds can contain.
        let x0 = fceil(xmin - 0.5).max(0.0);
        let x1 = ffloor(xmax - 0.5).min(w - 1.0);
        let y0 = fceil(ymin - 0.5).max(0.0);
        let y1 = ffloor(ymax - 0.5).min(h - 1.0);
        if x1 < x0 || y1 < y0 {
            return false;
        }
        let bounds = [x0 as i32, y0 as i32, x1 as i32, y1 as i32];
        // Interpolation planes from the whole triangle.
        let inv_det = 1.0 / area;
        let z = Plane::new(&v, [v[0].z, v[1].z, v[2].z], inv_det);
        let wp = Plane::new(&v, [v[0].iw, v[1].iw, v[2].iw], inv_det);
        let u = Plane::new(&v, [v[0].iw * bary[0][0], v[1].iw * bary[1][0], v[2].iw * bary[2][0]], inv_det);
        let vp = Plane::new(&v, [v[0].iw * bary[0][1], v[1].iw * bary[1][1], v[2].iw * bary[2][1]], inv_det);
        if xmin >= -GUARD && ymin >= -GUARD && xmax <= GUARD && ymax <= GUARD {
            return self.add(v.map(|s| (s.x, s.y)), bounds, z, wp, u, vp, data);
        }
        // Clip to the guard band in 2D (the planes stay the whole triangle's).
        let mut poly: Vec<(f32, f32)> = v.iter().map(|s| (s.x, s.y)).collect();
        for (axis, lim, keep_below) in [(0, GUARD, true), (0, -GUARD, false), (1, GUARD, true), (1, -GUARD, false)] {
            let get = |p: (f32, f32)| if axis == 0 { p.0 } else { p.1 };
            let inside = |p: (f32, f32)| if keep_below { get(p) <= lim } else { get(p) >= lim };
            let mut out = Vec::with_capacity(poly.len() + 2);
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                if inside(a) {
                    out.push(a);
                }
                if inside(a) != inside(b) {
                    let t = (lim - get(a)) / (get(b) - get(a));
                    out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
                }
            }
            poly = out;
            if poly.len() < 3 {
                return false;
            }
        }
        let mut any = false;
        for k in 1..poly.len() - 1 {
            any |= self.add([poly[0], poly[k], poly[k + 1]], bounds, z, wp, u, vp, data);
        }
        any
    }

    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, p: [(f32, f32); 3], bounds: [i32; 4], z: Plane, w: Plane, u: Plane, v: Plane, data: P) -> bool {
        let fx = p.map(|q| fround(q.0 * SUB));
        let fy = p.map(|q| fround(q.1 * SUB));
        let area = (fx[1] - fx[0]) * (fy[2] - fy[0]) - (fx[2] - fx[0]) * (fy[1] - fy[0]);
        if area == 0 {
            return false;
        }
        // Positive orientation; edge k runs from corner k+1 to k+2.
        let (ix, iy) = if area > 0 { ([0, 1, 2], [0, 1, 2]) } else { ([0, 2, 1], [0, 2, 1]) };
        let (fx, fy) = (ix.map(|i| fx[i]), iy.map(|i| fy[i]));
        // Bounds of this (possibly guard-clipped) part.
        let half = (SUB as i64) / 2;
        let px = |f: i64| f >> SUB_BITS;
        let xmin = fx.iter().copied().min().unwrap_or(0);
        let xmax = fx.iter().copied().max().unwrap_or(0);
        let ymin = fy.iter().copied().min().unwrap_or(0);
        let ymax = fy.iter().copied().max().unwrap_or(0);
        let bx0 = (px(xmin - half + (1 << SUB_BITS) - 1) as i32).max(bounds[0]);
        let bx1 = (px(xmax - half) as i32).min(bounds[2]);
        let by0 = (px(ymin - half + (1 << SUB_BITS) - 1) as i32).max(bounds[1]);
        let by1 = (px(ymax - half) as i32).min(bounds[3]);
        if bx1 < bx0 || by1 < by0 {
            return false;
        }
        let (cx, cy) = (bx0 as i64 * (1 << SUB_BITS) + half, by0 as i64 * (1 << SUB_BITS) + half);
        let mut e0 = [0i64; 3];
        let mut dx = [0i64; 3];
        let mut dy = [0i64; 3];
        for k in 0..3 {
            let (i, j) = ((k + 1) % 3, (k + 2) % 3);
            let (ex, ey) = (fx[j] - fx[i], fy[j] - fy[i]);
            // E(P) = ex * (Py - Yi) - ey * (Px - Xi): >= 0 inside.
            let top_left = (ey == 0 && ex > 0) || ey < 0;
            let bias = if top_left { 0 } else { -1 };
            e0[k] = ex * (cy - fy[i]) - ey * (cx - fx[i]) + bias;
            dx[k] = -ey << SUB_BITS;
            dy[k] = ex << SUB_BITS;
        }
        let local = self.tris.len() as u32;
        if local >= 1 << LOCAL_BITS {
            return false;
        }
        self.tris.push(SetupTri { e0, dx, dy, bx0, by0, bx1, by1, z, w, u, v });
        self.data.push(data);
        let l = self.layout;
        for ty in by0 as usize / TILE..=by1 as usize / TILE {
            for tx in bx0 as usize / TILE..=bx1 as usize / TILE {
                self.bins[ty * l.tx + tx].push(local);
            }
        }
        true
    }
}

/// Setup output of all pieces for one frame (keeps capacity between frames).
#[derive(Debug)]
pub(crate) struct Bins<P> {
    pub pieces: Vec<Piece<P>>,
    pub layout: Layout,
}

impl<P> Default for Bins<P> {
    fn default() -> Self {
        Bins { pieces: Vec::new(), layout: Layout::default() }
    }
}

impl<P: Copy + Send + Sync> Bins<P> {
    /// Empty for a frame of `layout` with `pieces` setup pieces.
    pub fn begin(&mut self, layout: Layout, pieces: usize) {
        self.layout = layout;
        self.pieces.resize_with(pieces.clamp(1, 255), Piece::default);
        for p in &mut self.pieces {
            p.reset(layout);
        }
    }

    /// Fill piece `k` with `f(k, piece)` for every piece, in parallel.
    pub fn setup(&mut self, threads: usize, f: impl Fn(usize, &mut Piece<P>) + Sync) {
        parallel_for_each(self.pieces.iter_mut().enumerate(), threads, |(k, p)| f(k, p));
    }

    #[inline(always)]
    pub fn get(&self, id: u32) -> (&SetupTri, &P) {
        let p = &self.pieces[(id >> LOCAL_BITS) as usize];
        let l = (id & ((1 << LOCAL_BITS) - 1)) as usize;
        (&p.tris[l], &p.data[l])
    }

    /// Visit the triangles binned to tile `t`, in order: (id, triangle).
    #[inline]
    pub fn for_tile(&self, t: usize, mut f: impl FnMut(u32, &SetupTri, &P)) {
        for (k, p) in self.pieces.iter().enumerate() {
            for &l in &p.bins[t] {
                f(((k as u32) << LOCAL_BITS) | l, &p.tris[l as usize], &p.data[l as usize]);
            }
        }
    }

    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.pieces.iter().map(|p| p.tris.len()).sum()
    }

    /// Depth test and write the nearest triangle id per pixel (larger depth
    /// keys win; ties keep the first). Buffers are resized and cleared.
    pub fn visibility(&self, depth: &mut Vec<f32>, ids: &mut Vec<u32>, threads: usize) {
        let l = self.layout;
        depth.resize(l.len(), 0.0);
        ids.resize(l.len(), 0);
        parallel_for_each(depth.chunks_mut(TILE_PX).zip(ids.chunks_mut(TILE_PX)).enumerate(), threads, |(t, (d, id))| {
            d.fill(f32::NEG_INFINITY);
            id.fill(NO_ID);
            let rect = l.tile_rect(t);
            self.for_tile(t, |tid, tri, _| {
                raster_tile(tri, rect, |k, x, y| {
                    let z = tri.z.at(x, y);
                    if z > d[k] {
                        d[k] = z;
                        id[k] = tid;
                    }
                });
            });
        });
    }

    /// Depth only (shadow maps): nearest depth key per pixel.
    pub fn depth_only(&self, depth: &mut Vec<f32>, threads: usize) {
        let l = self.layout;
        depth.resize(l.len(), 0.0);
        parallel_for_each(depth.chunks_mut(TILE_PX).enumerate(), threads, |(t, d)| {
            d.fill(f32::NEG_INFINITY);
            let rect = l.tile_rect(t);
            self.for_tile(t, |_, tri, _| {
                raster_tile(tri, rect, |k, x, y| {
                    let z = tri.z.at(x, y);
                    if z > d[k] {
                        d[k] = z;
                    }
                });
            });
        });
    }
}

/// Call `f(local index in the tile, x, y)` for every pixel centre of tile
/// `rect` (x0, y0, x1, y1 exclusive) covered by `t`.
#[inline(always)]
pub(crate) fn raster_tile(t: &SetupTri, rect: (usize, usize, usize, usize), mut f: impl FnMut(usize, usize, usize)) {
    let x0 = (t.bx0.max(rect.0 as i32)) as usize;
    let x1 = (t.bx1.min(rect.2 as i32 - 1)) as i64;
    let y0 = (t.by0.max(rect.1 as i32)) as usize;
    let y1 = (t.by1.min(rect.3 as i32 - 1)) as i64;
    if (x1 as i32) < x0 as i32 || (y1 as i32) < y0 as i32 {
        return;
    }
    let (ox, oy) = ((x0 as i64 - t.bx0 as i64), (y0 as i64 - t.by0 as i64));
    let mut row = [0i64; 3];
    for k in 0..3 {
        row[k] = t.e0[k] + t.dx[k] * ox + t.dy[k] * oy;
    }
    // Reject the block when one edge is negative at all four corners.
    let (nx, ny) = (x1 - x0 as i64, y1 - y0 as i64);
    for k in 0..3 {
        let c = [row[k], row[k] + t.dx[k] * nx, row[k] + t.dy[k] * ny, row[k] + t.dx[k] * nx + t.dy[k] * ny];
        if c.iter().all(|&e| e < 0) {
            return;
        }
    }
    for y in y0..=y1 as usize {
        let base = (y % TILE) * TILE;
        let (mut a, mut b, mut c) = (row[0], row[1], row[2]);
        for x in x0..=x1 as usize {
            if (a | b | c) >= 0 {
                f(base + x % TILE, x, y);
            }
            a += t.dx[0];
            b += t.dx[1];
            c += t.dx[2];
        }
        row[0] += t.dy[0];
        row[1] += t.dy[1];
        row[2] += t.dy[2];
    }
}

/// Split `ranges` (half-open index ranges, in order) into `pieces` runs of
/// about equal total length. Returns, per piece, its sub-ranges.
pub(crate) fn split_ranges(ranges: &[(usize, usize)], pieces: usize) -> Vec<Vec<(usize, usize)>> {
    let total: usize = ranges.iter().map(|r| r.1 - r.0).sum();
    let pieces = pieces.max(1);
    let per = total.div_ceil(pieces).max(1);
    let mut out = vec![Vec::new(); pieces];
    let (mut k, mut filled) = (0, 0);
    for &(mut a, b) in ranges {
        while a < b {
            let take = (per - filled).min(b - a);
            out[k].push((a, a + take));
            a += take;
            filled += take;
            if filled == per && k + 1 < pieces {
                k += 1;
                filled = 0;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(x: f32, y: f32) -> SVert {
        SVert { x, y, z: 1.0, iw: 1.0 }
    }

    fn coverage(bins: &Bins<u32>, w: usize, h: usize) -> Vec<u32> {
        let (mut d, mut ids) = (Vec::new(), Vec::new());
        bins.visibility(&mut d, &mut ids, 3);
        let l = bins.layout;
        (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| ids[l.index(x, y)]).collect()
    }

    #[test]
    fn shared_edges_fill_each_pixel_once() {
        // A fan of triangles around a centre: every covered pixel belongs to
        // exactly one triangle (top-left rule), whatever the winding.
        let (w, h) = (70, 50);
        let l = Layout::new(w, h);
        let c = sv(33.3, 24.7);
        let n = 13;
        let mut counts = vec![0u32; w * h];
        for k in 0..n {
            let a0 = k as f32 / n as f32 * core::f32::consts::TAU;
            let a1 = (k + 1) as f32 / n as f32 * core::f32::consts::TAU;
            let p0 = sv(33.3 + 20.0 * a0.cos(), 24.7 + 20.0 * a0.sin());
            let p1 = sv(33.3 + 20.0 * a1.cos(), 24.7 + 20.0 * a1.sin());
            let mut bins = Bins::<u32>::default();
            bins.begin(l, 1);
            let tri = if k % 2 == 0 { [c, p0, p1] } else { [c, p1, p0] };
            bins.pieces[0].push(tri, [[0.0; 2]; 3], Facing::Both, 0);
            for (i, id) in coverage(&bins, w, h).into_iter().enumerate() {
                counts[i] += (id != NO_ID) as u32;
            }
        }
        assert!(counts.iter().all(|&c| c <= 1));
        let filled = counts.iter().filter(|&&c| c == 1).count();
        // Area of the 13-gon of radius 20.
        let area = 0.5 * n as f32 * 400.0 * (core::f32::consts::TAU / n as f32).sin();
        assert!((filled as f32 - area).abs() < area * 0.03, "{filled} vs {area}");
    }

    #[test]
    fn guard_band_and_planes() {
        let (w, h) = (40, 30);
        let l = Layout::new(w, h);
        let mut bins = Bins::<u32>::default();
        bins.begin(l, 1);
        // Huge triangle covering the whole frame.
        let v = [SVert { x: -1e7, y: -1e7, z: 0.5, iw: 1.0 }, SVert { x: 1e7, y: -1e7, z: 0.5, iw: 1.0 }, SVert { x: 0.0, y: 1e7, z: 0.5, iw: 1.0 }];
        assert!(bins.pieces[0].push(v, [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], Facing::Both, 7));
        let ids = coverage(&bins, w, h);
        assert!(ids.iter().all(|&i| i != NO_ID));
        let (t, d) = bins.get(ids[0]);
        assert_eq!(*d, 7);
        let b = t.bary(20, 15);
        assert!((b[0] + b[1] + b[2] - 1.0).abs() < 1e-4);
        // Off-screen and back-facing triangles are dropped.
        assert!(!bins.pieces[0].push([sv(-10.0, -10.0), sv(-5.0, -10.0), sv(-5.0, -5.0)], [[0.0; 2]; 3], Facing::Both, 0));
        let cw = [sv(1.0, 1.0), sv(10.0, 1.0), sv(1.0, 10.0)];
        assert!(bins.pieces[0].push(cw, [[0.0; 2]; 3], Facing::Positive, 0));
        assert!(!bins.pieces[0].push(cw, [[0.0; 2]; 3], Facing::Negative, 0));
    }

    #[test]
    fn split_ranges_covers_everything_in_order() {
        let r = [(0, 10), (20, 23), (30, 31)];
        let s = split_ranges(&r, 4);
        let flat: Vec<usize> = s.iter().flatten().flat_map(|&(a, b)| a..b).collect();
        let want: Vec<usize> = r.iter().flat_map(|&(a, b)| a..b).collect();
        assert_eq!(flat, want);
        assert!(s.iter().all(|p| p.iter().map(|r| r.1 - r.0).sum::<usize>() <= 4));
    }
}
