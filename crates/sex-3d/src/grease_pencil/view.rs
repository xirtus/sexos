// SPDX-License-Identifier: GPL-3.0-only
//! The projection strokes are drawn, edited and rasterized through: the 3D
//! Viewport's orbit camera or a scene camera, with pixel coordinates from the
//! top-left and depth as distance along the view axis.
use crate::math::{Mat4, Vec3};
use crate::render::viewport::ViewCamera;
use crate::scene::Camera;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpView {
    pub eye: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// Unit vector pointing away from the view direction (towards the eye).
    pub back: Vec3,
    pub ortho: bool,
    /// Pixels per unit of x/z at distance 1 (perspective) or per world unit (ortho).
    pub scale: f32,
    pub cx: f32,
    pub cy: f32,
    pub width: u32,
    pub height: u32,
    pub near: f32,
}

impl GpView {
    /// The same projection as the Solid viewport's rasterizer.
    pub fn from_viewport(v: &ViewCamera, w: u32, h: u32) -> GpView {
        let t = (v.fov_y.clamp(1e-3, 3.0) * 0.5).tan();
        let half_h = h as f32 * 0.5;
        let scale = if v.ortho { half_h / (v.distance * t) } else { half_h / t };
        GpView { eye: v.eye(), right: v.right(), up: v.up(), back: v.back(), ortho: v.ortho, scale, cx: w as f32 * 0.5, cy: half_h, width: w, height: h, near: (v.distance * 1e-3).max(1e-4) }
    }
    /// A scene camera's view (perspective or orthographic; matches the path
    /// tracer's camera, shift included).
    pub fn from_camera(m: &Mat4, cam: &Camera, w: u32, h: u32) -> GpView {
        let right = m.transform_vector(Vec3::X).normalize();
        let up = m.transform_vector(Vec3::Y).normalize();
        let back = m.transform_vector(Vec3::Z).normalize();
        let half_h = h as f32 * 0.5;
        let aspect = w as f32 / h.max(1) as f32;
        let big = aspect.max(1.0);
        let (ortho, scale) = match cam.ortho_scale.filter(|s| *s > 0.0) {
            Some(s) => (true, half_h / (s * 0.5)),
            None => (false, half_h / (cam.fov_y.clamp(1e-4, 3.1) * 0.5).tan()),
        };
        // Shift is in units of the larger frame dimension.
        let cx = w as f32 * 0.5 - cam.shift_x * big * h as f32;
        let cy = half_h + cam.shift_y * big * h as f32;
        GpView { eye: m.transform_point(Vec3::ZERO), right, up, back, ortho, scale, cx, cy, width: w, height: h, near: cam.near.max(1e-4) }
    }
    /// View space (x right, y up, z towards the eye).
    pub fn to_view(&self, p: Vec3) -> Vec3 {
        let d = p - self.eye;
        Vec3::new(d.dot(self.right), d.dot(self.up), d.dot(self.back))
    }
    /// Pixel position and distance along the view axis; `None` behind the eye.
    pub fn project(&self, p: Vec3) -> Option<(f32, f32, f32)> {
        let v = self.to_view(p);
        let dist = -v.z;
        if self.ortho {
            return Some((self.cx + v.x * self.scale, self.cy - v.y * self.scale, dist));
        }
        if dist < self.near {
            return None;
        }
        Some((self.cx + v.x / dist * self.scale, self.cy - v.y / dist * self.scale, dist))
    }
    /// Pixels per world unit at view distance `dist`.
    pub fn pixels_per_unit(&self, dist: f32) -> f32 {
        if self.ortho {
            self.scale
        } else {
            self.scale / dist.max(self.near)
        }
    }
    /// World ray through pixel (`x`, `y`): origin and unit direction.
    pub fn ray(&self, x: f32, y: f32) -> (Vec3, Vec3) {
        let sx = (x - self.cx) / self.scale;
        let sy = (self.cy - y) / self.scale;
        if self.ortho {
            (self.eye + self.right * sx + self.up * sy, -self.back)
        } else {
            (self.eye, (-self.back + self.right * sx + self.up * sy).normalize())
        }
    }
    /// The point under pixel (`x`, `y`) at the depth of `at` (on the view plane through it).
    pub fn at_depth(&self, x: f32, y: f32, at: Vec3) -> Vec3 {
        self.on_plane(x, y, at, self.back).unwrap_or(at)
    }
    /// Intersection of the pixel's ray with the plane through `o` with normal `n`.
    pub fn on_plane(&self, x: f32, y: f32, o: Vec3, n: Vec3) -> Option<Vec3> {
        let (ro, rd) = self.ray(x, y);
        let den = rd.dot(n);
        if den.abs() < 1e-8 {
            return None;
        }
        let t = (o - ro).dot(n) / den;
        (t > 0.0 || self.ortho).then(|| ro + rd * t)
    }
    /// A world-space offset for a pixel offset at the depth of `at`.
    pub fn pixel_delta(&self, dx: f32, dy: f32, at: Vec3) -> Vec3 {
        let dist = -self.to_view(at).z;
        let ppu = self.pixels_per_unit(dist);
        self.right * (dx / ppu) - self.up * (dy / ppu)
    }
}

/// Drawing planes (Blender's "Drawing Plane" in Draw Mode).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawPlane {
    /// X-Z plane (looking along +Y).
    #[default]
    Front,
    /// Y-Z plane.
    Side,
    /// X-Y plane.
    Top,
    /// Facing the view.
    View,
    /// The 3D cursor's rotated XY plane.
    Cursor,
}

impl DrawPlane {
    pub const ALL: [DrawPlane; 5] = [DrawPlane::Front, DrawPlane::Side, DrawPlane::Top, DrawPlane::View, DrawPlane::Cursor];
    pub fn label(self) -> &'static str {
        match self {
            DrawPlane::Front => "Front (X-Z)",
            DrawPlane::Side => "Side (Y-Z)",
            DrawPlane::Top => "Top (X-Y)",
            DrawPlane::View => "View",
            DrawPlane::Cursor => "Cursor",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            DrawPlane::Front => "AXIS_Y",
            DrawPlane::Side => "AXIS_X",
            DrawPlane::Top => "AXIS_Z",
            DrawPlane::View => "VIEW",
            DrawPlane::Cursor => "CURSOR",
        }
    }
    pub fn from_id(s: &str) -> Option<DrawPlane> {
        DrawPlane::ALL.into_iter().find(|p| p.id().eq_ignore_ascii_case(s) || p.label() == s)
    }
    /// The plane normal in world space (`cursor_rot`: the 3D cursor's rotation matrix).
    pub fn normal(self, view: &GpView, object: &Mat4, cursor_rot: &Mat4) -> Vec3 {
        match self {
            DrawPlane::Front => object.transform_vector(Vec3::Y).normalize(),
            DrawPlane::Side => object.transform_vector(Vec3::X).normalize(),
            DrawPlane::Top => object.transform_vector(Vec3::Z).normalize(),
            DrawPlane::View => view.back,
            DrawPlane::Cursor => cursor_rot.transform_vector(Vec3::Z).normalize(),
        }
    }
}

/// Where new strokes are placed (Blender's "Stroke Placement").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// On the drawing plane through the object origin.
    #[default]
    Origin,
    /// On the drawing plane through the 3D cursor.
    Cursor,
    /// Projected onto visible surfaces (falls back to Origin off them).
    Surface,
    /// Onto existing strokes (their depth; falls back to Origin).
    Stroke,
}

impl Placement {
    pub const ALL: [Placement; 4] = [Placement::Origin, Placement::Cursor, Placement::Surface, Placement::Stroke];
    pub fn label(self) -> &'static str {
        match self {
            Placement::Origin => "Origin",
            Placement::Cursor => "3D Cursor",
            Placement::Surface => "Surface",
            Placement::Stroke => "Stroke",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Placement::Origin => "ORIGIN",
            Placement::Cursor => "CURSOR",
            Placement::Surface => "SURFACE",
            Placement::Stroke => "STROKE",
        }
    }
    pub fn from_id(s: &str) -> Option<Placement> {
        Placement::ALL.into_iter().find(|p| p.id().eq_ignore_ascii_case(s) || p.label() == s)
    }
}

/// Everything needed to turn pixels into stroke points.
pub struct Projector<'a> {
    pub view: &'a GpView,
    pub plane: DrawPlane,
    pub placement: Placement,
    /// Object (layer) to world.
    pub object: Mat4,
    pub cursor: Vec3,
    pub cursor_rot: Mat4,
    /// Distance (along the view axis) of the nearest surface or stroke under
    /// a pixel, for Surface and Stroke placement.
    pub depth_at: Option<&'a dyn Fn(f32, f32) -> Option<f32>>,
    /// Offset from the surface along the view (Blender's Surface offset).
    pub surface_offset: f32,
}

impl Projector<'_> {
    fn origin(&self) -> Vec3 {
        match self.placement {
            Placement::Cursor => self.cursor,
            _ => self.object.transform_point(Vec3::ZERO),
        }
    }
    /// The world-space point for pixel (`x`, `y`).
    pub fn world(&self, x: f32, y: f32) -> Vec3 {
        if matches!(self.placement, Placement::Surface | Placement::Stroke) {
            if let Some(d) = self.depth_at.and_then(|f| f(x, y)).filter(|d| d.is_finite()) {
                let (o, dir) = self.view.ray(x, y);
                let d = (d - self.surface_offset).max(self.view.near);
                // `d` is along the view axis; convert to the ray parameter.
                let cos = dir.dot(-self.view.back).max(1e-6);
                return if self.view.ortho { o + dir * (d + self.view.to_view(o).z) } else { o + dir * (d / cos) };
            }
        }
        let n = self.plane.normal(self.view, &self.object, &self.cursor_rot);
        self.view.on_plane(x, y, self.origin(), n).unwrap_or_else(|| self.view.at_depth(x, y, self.origin()))
    }
    /// Layer-space point.
    pub fn local(&self, x: f32, y: f32) -> Vec3 {
        self.object.inverse().transform_point(self.world(x, y))
    }
    /// A pixel radius as a layer-space radius at world point `w`.
    pub fn radius(&self, px: f32, w: Vec3) -> f32 {
        let dist = -self.view.to_view(w).z;
        px / self.view.pixels_per_unit(dist) / super::scale_of(&self.object)
    }
}
