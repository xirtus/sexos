// SPDX-License-Identifier: GPL-3.0-only
//! World <-> screen for the 3D view, matching sex-3d's viewport rasterizer
//! exactly, so overlays (edit-mode vertices, gizmos, the 3D cursor) sit on the
//! shaded image.
use crate::draw::Rect;
use sex_3d::render::viewport::ViewCamera;
use sex_3d::Vec3;

#[derive(Clone, Copy, Debug)]
pub struct Proj {
    pub eye: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub back: Vec3,
    pub target: Vec3,
    pub ortho: bool,
    pub distance: f32,
    /// Pixels per unit of x/(-z) (perspective) or per world unit (ortho).
    pub scale: f32,
    /// Screen position of the view centre (absolute window pixels).
    pub cx: f32,
    pub cy: f32,
    pub near: f32,
    pub rect: Rect,
    pub tan_half: f32,
}

impl Proj {
    pub fn new(cam: &ViewCamera, r: Rect) -> Proj {
        let tan_half = (cam.fov_y.clamp(1e-3, 3.0) * 0.5).tan();
        let half_h = r.h.max(1) as f32 * 0.5;
        let scale = if cam.ortho { half_h / (cam.distance * tan_half) } else { half_h / tan_half };
        Proj {
            eye: cam.eye(),
            right: cam.right(),
            up: cam.up(),
            back: cam.back(),
            target: cam.target,
            ortho: cam.ortho,
            distance: cam.distance,
            scale,
            cx: r.x as f32 + r.w as f32 * 0.5,
            cy: r.y as f32 + half_h,
            near: (cam.distance * 1e-3).max(1e-4),
            rect: r,
            tan_half,
        }
    }
    pub fn to_view(&self, p: Vec3) -> Vec3 {
        let d = p - self.eye;
        Vec3::new(d.dot(self.right), d.dot(self.up), d.dot(self.back))
    }
    /// Window pixel position and a depth key (larger is closer, affine in
    /// screen space) of a world point, or None behind the near plane.
    pub fn project(&self, p: Vec3) -> Option<(f32, f32, f32)> {
        let v = self.to_view(p);
        if self.ortho {
            return Some((self.cx + v.x * self.scale, self.cy - v.y * self.scale, v.z));
        }
        if v.z > -self.near {
            return None;
        }
        let iw = -1.0 / v.z;
        Some((self.cx + v.x * iw * self.scale, self.cy - v.y * iw * self.scale, iw))
    }
    pub fn to_screen(&self, p: Vec3) -> Option<(f32, f32)> {
        self.project(p).map(|(x, y, _)| (x, y))
    }
    /// World units per pixel at the depth of `p` (for screen-sized gizmos and
    /// mouse-to-world motion).
    pub fn world_per_pixel(&self, p: Vec3) -> f32 {
        if self.ortho {
            1.0 / self.scale
        } else {
            let d = (-self.to_view(p).z).max(self.near);
            d / self.scale
        }
    }
    /// World ray through window pixel (x, y): origin and unit direction.
    pub fn ray(&self, x: f32, y: f32) -> (Vec3, Vec3) {
        let sx = (x - self.cx) / self.scale;
        let sy = (self.cy - y) / self.scale;
        if self.ortho {
            let o = self.eye + self.right * sx + self.up * sy + self.back * (self.distance * 1000.0);
            (o, -self.back)
        } else {
            (self.eye, (-self.back + self.right * sx + self.up * sy).normalize())
        }
    }
    /// Where the ray through (x, y) meets the plane through `p` with normal `n`.
    pub fn on_plane(&self, x: f32, y: f32, p: Vec3, n: Vec3) -> Option<Vec3> {
        let (o, d) = self.ray(x, y);
        let den = d.dot(n);
        if den.abs() < 1e-6 {
            return None;
        }
        let t = (p - o).dot(n) / den;
        (t > 0.0 || self.ortho).then(|| o + d * t)
    }
    /// The point under (x, y) at the depth of `p` (on the view plane through it).
    pub fn at_depth(&self, x: f32, y: f32, p: Vec3) -> Vec3 {
        self.on_plane(x, y, p, self.back).unwrap_or(p)
    }
}

/// Distance from point (px, py) to segment a-b in screen space.
pub fn seg_dist(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-9 { 0.0 } else { (((px - a.0) * dx + (py - a.1) * dy) / l2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + dx * t, a.1 + dy * t);
    ((px - qx).powi(2) + (py - qy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_target_to_centre_and_rays_back() {
        let cam = ViewCamera::default();
        let p = Proj::new(&cam, Rect::new(10, 20, 400, 300));
        let (x, y) = p.to_screen(Vec3::ZERO).unwrap();
        assert!((x - 210.0).abs() < 1e-3 && (y - 170.0).abs() < 1e-3);
        let q = Vec3::new(1.0, 0.5, 0.25);
        let (sx, sy) = p.to_screen(q).unwrap();
        let back = p.at_depth(sx, sy, q);
        assert!(back.approx_eq(q, 1e-3), "{back:?}");
    }
}
