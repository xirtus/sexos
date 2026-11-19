// SPDX-License-Identifier: GPL-3.0-only
//! Camera rays as Cycles makes them: perspective (with sensor fit and lens
//! shift), orthographic, and panoramic (equirectangular, fisheye equidistant
//! and equisolid) lenses; thin-lens depth of field with polygonal bokeh and
//! anamorphic ratio; clip start/end; and a camera moving during the shutter.
use super::sampling::{concentric_disk, onb};
use crate::math::{Mat4, Vec3};
use crate::scene::{Camera, PanoramaType, Scene, Transform};
use core::f32::consts::{PI, TAU};

/// An object's local transforms up its parent chain, each moving linearly
/// towards its motion transform: the world matrix at any shutter time.
#[derive(Clone, Debug, Default)]
pub(crate) struct MotionChain {
    /// Object first, then its parents.
    links: Vec<(Transform, Option<Transform>)>,
}

impl MotionChain {
    pub fn from_scene(scene: &Scene, idx: usize) -> MotionChain {
        let mut links = Vec::new();
        let mut i = Some(idx);
        while let Some(c) = i {
            if links.len() > scene.objects.len() {
                break; // parent cycle
            }
            let o = &scene.objects[c];
            links.push((o.transform, o.motion));
            i = o.parent;
        }
        MotionChain { links }
    }

    pub fn matrix(&self, time: f32) -> Mat4 {
        let mut m: Option<Mat4> = None;
        for (t, motion) in &self.links {
            let local = match motion {
                Some(e) if time != 0.0 => t.lerp(e, time),
                _ => *t,
            };
            let lm = local.matrix();
            m = Some(match m {
                None => lm,
                Some(child) => lm * child,
            });
        }
        m.unwrap_or_else(|| Transform::default().matrix())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Pose {
    pub origin: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
}

impl Pose {
    fn from(m: &Mat4) -> Pose {
        Pose {
            origin: m.transform_point(Vec3::ZERO),
            right: m.transform_vector(Vec3::X).normalize(),
            up: m.transform_vector(Vec3::Y).normalize(),
            forward: m.transform_vector(-Vec3::Z).normalize(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Lens {
    Perspective { tan_half: f32 },
    /// Half the frame height in metres.
    Orthographic { half: f32 },
    Equirectangular { lat: (f32, f32), lon: (f32, f32) },
    FisheyeEquidistant { fov: f32 },
    /// Focal length and sensor width in mm.
    FisheyeEquisolid { lens: f32, fov: f32, sensor: f32 },
}

/// Camera rig for generating primary rays.
#[derive(Clone, Debug)]
pub(crate) struct CameraRig {
    pub pose: Pose,
    /// The camera's transform chain when it moves during the shutter.
    motion: Option<MotionChain>,
    pub lens: Lens,
    cam: Camera,
    shift: (f32, f32),
    near: f32,
    far: f32,
    /// Depth of field: f-stop (0 = pinhole) and focus distance.
    fstop: f32,
    focus: f32,
    inv_ratio: f32,
}

/// A primary ray: origin, unit direction, the usable distance, and the view
/// axis (for Z depth).
pub(crate) struct CameraRay {
    /// The camera (lens) position the ray leaves from, before clipping.
    pub eye: Vec3,
    pub o: Vec3,
    pub d: Vec3,
    pub tmax: f32,
    pub forward: Vec3,
    pub panoramic: bool,
    pub orthographic: bool,
}

impl CameraRig {
    /// `focus_point`: world position of the focus object, if any.
    pub fn new(m: &Mat4, cam: &Camera, focus_point: Option<Vec3>, motion: Option<MotionChain>) -> CameraRig {
        let pose = Pose::from(m);
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let lens = match cam.panorama {
            Some(PanoramaType::Equirectangular) => Lens::Equirectangular { lat: (fin(cam.latitude_min, -PI / 2.0), fin(cam.latitude_max, PI / 2.0)), lon: (fin(cam.longitude_min, -PI), fin(cam.longitude_max, PI)) },
            Some(PanoramaType::FisheyeEquidistant) => Lens::FisheyeEquidistant { fov: fin(cam.fisheye_fov, PI).clamp(1e-3, TAU) },
            Some(PanoramaType::FisheyeEquisolid) => Lens::FisheyeEquisolid { lens: fin(cam.fisheye_lens, 10.5).max(0.01), fov: fin(cam.fisheye_fov, PI).clamp(1e-3, TAU), sensor: fin(cam.sensor_width, 36.0).max(0.01) },
            None => match cam.ortho_scale.filter(|s| *s > 0.0) {
                Some(s) => Lens::Orthographic { half: s * 0.5 },
                None => Lens::Perspective { tan_half: (fin(cam.fov_y, 0.8).clamp(1e-4, 3.1) * 0.5).tan() },
            },
        };
        let dof = &cam.dof;
        let fstop = if dof.enabled && dof.fstop.is_finite() && dof.fstop > 0.0 { dof.fstop } else { 0.0 };
        let focus = match focus_point {
            Some(p) => (p - pose.origin).dot(pose.forward).max(1e-4),
            None => fin(dof.focus_distance, 10.0).max(1e-4),
        };
        CameraRig {
            pose,
            motion,
            lens,
            cam: *cam,
            shift: (fin(cam.shift_x, 0.0), fin(cam.shift_y, 0.0)),
            near: fin(cam.near, 0.0).max(0.0),
            far: if cam.far > cam.near { cam.far } else { f32::INFINITY },
            fstop,
            focus,
            inv_ratio: if dof.ratio.is_finite() && dof.ratio > 0.0 { 1.0 / dof.ratio } else { 1.0 },
        }
    }

    /// The camera pose at shutter time `time` (frames from the current one).
    fn pose_at(&self, time: f32) -> Pose {
        match &self.motion {
            Some(c) if time != 0.0 => Pose::from(&c.matrix(time)),
            _ => self.pose,
        }
    }

    /// Point on the unit aperture from two uniforms.
    fn aperture_sample(&self, u1: f32, u2: f32) -> (f32, f32) {
        let blades = self.cam.dof.blades;
        let (x, y) = if blades >= 3 {
            // Uniform point in a regular polygon: pick a triangle, then a point in it.
            let n = blades as f32;
            let k = (u1 * n).floor().min(n - 1.0);
            let u1 = u1 * n - k;
            let a0 = self.cam.dof.rotation + TAU * k / n;
            let a1 = a0 + TAU / n;
            let su = u1.sqrt();
            let (b0, b1) = (su * (1.0 - u2), su * u2);
            (b0 * a0.cos() + b1 * a1.cos(), b0 * a0.sin() + b1 * a1.sin())
        } else {
            concentric_disk(u1, u2)
        };
        (x * self.inv_ratio, y)
    }

    /// Ray through film position (`fx`, `fy`) in pixels of a `w` x `h`
    /// frame at shutter `time`, with lens sample (`lu`, `lv`). `None` outside
    /// a fisheye's circle.
    #[allow(clippy::too_many_arguments)]
    pub fn ray(&self, fx: f32, fy: f32, w: u32, h: u32, time: f32, lu: f32, lv: f32) -> Option<CameraRay> {
        let p = self.pose_at(time);
        let aspect = w as f32 / h.max(1) as f32;
        let sx = 2.0 * fx / w.max(1) as f32 - 1.0;
        let sy = 1.0 - 2.0 * fy / h.max(1) as f32;
        // Normalised so the larger dimension spans [-1, 1].
        let big = aspect.max(1.0);
        let (nx, ny) = (sx * aspect / big, sy / big);
        let (o, d, cos) = match self.lens {
            Lens::Perspective { tan_half } => {
                // Shift is in units of the larger frame dimension (its full extent = 2).
                let x = (sx * aspect + 2.0 * self.shift.0 * big) * tan_half;
                let y = (sy + 2.0 * self.shift.1 * big) * tan_half;
                let d = (p.forward + p.right * x + p.up * y).normalize();
                (p.origin, d, 1.0 / (1.0 + x * x + y * y).sqrt())
            }
            Lens::Orthographic { half } => {
                let x = (sx * aspect + 2.0 * self.shift.0 * big) * half;
                let y = (sy + 2.0 * self.shift.1 * big) * half;
                (p.origin + p.right * x + p.up * y, p.forward, 1.0)
            }
            Lens::Equirectangular { lat, lon } => {
                let u = fx / w.max(1) as f32 + self.shift.0;
                let v = fy / h.max(1) as f32 - self.shift.1;
                let phi = lon.0 + (lon.1 - lon.0) * u;
                let theta = lat.1 - (lat.1 - lat.0) * v;
                let d = p.forward * (theta.cos() * phi.cos()) + p.right * (theta.cos() * phi.sin()) + p.up * theta.sin();
                (p.origin, d.normalize(), 1.0)
            }
            Lens::FisheyeEquidistant { fov } => {
                let r = (nx * nx + ny * ny).sqrt();
                if r > 1.0 {
                    return None;
                }
                (p.origin, fisheye_dir(&p, nx, ny, r, r * fov * 0.5), 1.0)
            }
            Lens::FisheyeEquisolid { lens, fov, sensor } => {
                // Position on the sensor in mm; the sensor width spans the larger side.
                let (ux, uy) = (nx * sensor * 0.5, ny * sensor * 0.5);
                let r = (ux * ux + uy * uy).sqrt();
                if r > 2.0 * lens * (fov * 0.25).sin() {
                    return None;
                }
                let theta = 2.0 * (r / (2.0 * lens)).min(1.0).asin();
                (p.origin, fisheye_dir(&p, ux, uy, r, theta), 1.0)
            }
        };
        let panoramic = !matches!(self.lens, Lens::Perspective { .. } | Lens::Orthographic { .. });
        // Clip planes are planes for perspective/orthographic, spheres for panoramas.
        let (t0, t1) = (self.near / cos, self.far / cos);
        let (mut o, mut d) = (o, d);
        if self.fstop > 0.0 {
            // Cycles: aperture radius = focal length / (2 f-stop).
            let radius = self.cam.focal_length(aspect) * 1e-3 / (2.0 * self.fstop);
            let (ax, ay) = self.aperture_sample(lu, lv);
            let (ax, ay) = (ax * radius, ay * radius);
            if panoramic {
                // Focus on a sphere around the camera; the lens faces along the ray.
                let focus = o + d * self.focus;
                let (t, b) = onb(d);
                o += t * ax + b * ay;
                d = (focus - o).normalize();
            } else {
                // Focus plane at `focus` along the view axis.
                let focus = o + d * (self.focus / d.dot(p.forward).max(1e-6));
                o += p.right * ax + p.up * ay;
                d = (focus - o).normalize();
            }
        }
        let orthographic = matches!(self.lens, Lens::Orthographic { .. });
        Some(CameraRay { eye: o, o: o + d * t0, d, tmax: (t1 - t0).max(0.0), forward: p.forward, panoramic, orthographic })
    }
}

fn fisheye_dir(p: &Pose, x: f32, y: f32, r: f32, theta: f32) -> Vec3 {
    let (cx, cy) = if r > 0.0 { (x / r, y / r) } else { (0.0, 0.0) };
    (p.forward * theta.cos() + (p.right * cx + p.up * cy) * theta.sin()).normalize()
}
