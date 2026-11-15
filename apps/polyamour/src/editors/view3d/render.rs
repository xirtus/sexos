// SPDX-License-Identifier: GPL-3.0-only
//! Material Preview and Rendered shading: sex-3d's path tracer refined one
//! sample per frame (the window keeps sending ticks while it refines) at a
//! quarter of the viewport's resolution, from the viewport's own camera.
use sex_3d::render::viewport::ViewCamera;
use crate::settings::Settings;
use sex_3d::render::{to_display8, ColorManagement, LightPaths, Progressive, RenderSettings};
use sex_3d::scene::{Camera, Object, ObjectData, Scene};
use sex_3d::{Mat4, Vec3};

/// Pixels of the viewport per rendered pixel along each axis.
pub const DOWNSCALE: u32 = 4;

pub struct RenderView {
    key: (u64, [u32; 8], u32, u32, bool, bool),
    prog: Progressive,
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
    pub target: u32,
    cm: ColorManagement,
}

fn cam_key(c: &ViewCamera) -> [u32; 8] {
    [c.target.x, c.target.y, c.target.z, c.distance, c.yaw, c.pitch, c.fov_y, if c.ortho { 1.0 } else { 0.0 }].map(f32::to_bits)
}

/// A copy of `scene` seen through `view` (an added camera made active).
/// Material Preview lights it with a neutral studio world.
pub fn view_scene(scene: &Scene, view: &ViewCamera, preview: bool, camera_view: bool) -> Scene {
    let mut s = scene.clone();
    let (r, u, b) = (view.right(), view.up(), view.back());
    let tan_half = (view.fov_y.clamp(1e-3, 3.0) * 0.5).tan();
    let (eye, ortho) = if view.ortho { (view.target + b * (view.distance * 10.0 + 100.0), Some(2.0 * view.distance * tan_half)) } else { (view.eye(), None) };
    let m = Mat4::from_cols_array([r.x, r.y, r.z, 0.0, u.x, u.y, u.z, 0.0, b.x, b.y, b.z, 0.0, eye.x, eye.y, eye.z, 1.0]);
    // The view keeps the scene camera's depth of field when looking through it.
    let dof = scene.active_camera.and_then(|c| scene.objects.get(c)).and_then(|o| match &o.data {
        ObjectData::Camera(c) if camera_view => Some(c.dof),
        _ => None,
    });
    let cam = Camera { fov_y: view.fov_y, ortho_scale: ortho, near: 0.01, far: 1e5, dof: dof.unwrap_or_default(), ..Camera::default() };
    let mut o = Object::new("__view__", ObjectData::Camera(cam));
    o.transform = sex_3d::Transform::from_matrix(&m);
    let ci = s.add_object(o);
    s.active_camera = Some(ci);
    if preview {
        s.world.color = Vec3::splat(0.45);
        s.world.strength = 1.0;
        s.world.hdri = None;
    }
    s
}

impl RenderView {
    /// Keep `slot` current for this frame and add one sample. Returns true
    /// while more samples are wanted (the window should keep ticking).
    #[allow(clippy::too_many_arguments)]
    pub fn refresh(slot: &mut Option<RenderView>, scene: &Scene, settings: &Settings, generation: u64, view: &ViewCamera, camera_view: bool, vw: i32, vh: i32, preview: bool) -> bool {
        let (w, h) = (((vw.max(1) as u32) / DOWNSCALE).max(1), ((vh.max(1) as u32) / DOWNSCALE).max(1));
        let key = (generation, cam_key(view), w, h, preview, camera_view);
        if slot.as_ref().is_none_or(|r| r.key != key) {
            let s = view_scene(scene, view, preview, camera_view);
            let (rs, target) = if preview {
                (RenderSettings { width: w, height: h, samples: 1, max_bounces: 2, denoise: false, light_paths: LightPaths::unlimited(), ..Default::default() }, 16)
            } else {
                let mut rs = settings.viewport_render_settings(w, h);
                rs.noise_threshold = if settings.viewport_noise_threshold { settings.viewport_adaptive_threshold.max(0.0) } else { 0.0 };
                (rs, settings.viewport_samples.max(1))
            };
            let cm = if preview { Default::default() } else { settings.color_management() };
            *slot = Some(RenderView { key, prog: Progressive::new(&s, &rs), rgba: vec![0; (w * h * 4) as usize], w, h, target, cm });
        }
        let Some(rv) = slot.as_mut() else { return false };
        if rv.wants_samples() {
            rv.prog.add_sample(None);
            rv.rgba = to_display8(&rv.prog.image(), &rv.cm);
        }
        rv.wants_samples()
    }
    pub fn samples(&self) -> u32 {
        self.prog.samples()
    }
    fn wants_samples(&self) -> bool {
        self.prog.samples() < self.target && !self.prog.converged()
    }
}
