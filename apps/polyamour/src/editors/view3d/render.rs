// SPDX-License-Identifier: GPL-3.0-only
//! Material Preview and Rendered shading, refined one sample per frame (the
//! window keeps sending ticks while it refines) from the viewport's own
//! camera.
//!
//! Material Preview is the rasterized engine (EEVEE) in look-dev mode: the
//! studio environment instead of the scene's world and lights. Rendered
//! uses the scene's engine: EEVEE and Workbench at the viewport's
//! resolution, the path tracer at a quarter of it. EEVEE keeps its prepared
//! scene while only the view moves.
use crate::settings::{Engine, Settings};
use sex_3d::render::viewport::{ViewCamera, ViewportScene, ViewportSettings};
use sex_3d::render::{srgb_encode, to_display8, ColorManagement, Eevee, EeveeScene, Progressive, RasterCamera};
use sex_3d::scene::{Camera, Object, ObjectData, Scene};
use sex_3d::{Mat4, Vec3};

/// Pixels of the viewport per path-traced pixel along each axis.
pub const DOWNSCALE: u32 = 4;

enum Backend {
    Path(Box<Progressive>),
    Eevee(Box<Eevee>),
    Workbench { scene: Box<ViewportScene>, done: bool },
}

pub struct RenderView {
    /// What forces a rebuild: scene generation, engine and mode.
    scene_key: (u64, Engine, bool, bool),
    /// Everything else (view, size, settings).
    view_key: ([u32; 8], u32, u32, u64),
    backend: Backend,
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
    pub target: u32,
    cm: ColorManagement,
}

fn cam_key(c: &ViewCamera) -> [u32; 8] {
    [c.target.x, c.target.y, c.target.z, c.distance, c.yaw, c.pitch, c.fov_y, if c.ortho { 1.0 } else { 0.0 }].map(f32::to_bits)
}

/// A hash of the settings that change the image.
fn settings_key(settings: &Settings) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{settings:?}").hash(&mut h);
    h.finish()
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
        let engine = if preview { Engine::Eevee } else { settings.engine };
        let (vw, vh) = (vw.max(1) as u32, vh.max(1) as u32);
        let (w, h) = if engine == Engine::PathTracer { ((vw / DOWNSCALE).max(1), (vh / DOWNSCALE).max(1)) } else { (vw, vh) };
        let scene_key = (generation, engine, preview, camera_view);
        let view_key = (cam_key(view), w, h, if preview { 0 } else { settings_key(settings) });
        let cm = if preview { Default::default() } else { settings.color_management() };
        let rebuild = slot.as_ref().is_none_or(|r| r.scene_key != scene_key || (engine != Engine::Eevee && r.view_key != view_key));
        if rebuild {
            let backend = match engine {
                Engine::PathTracer => {
                    let s = view_scene(scene, view, false, camera_view);
                    let mut rs = settings.viewport_render_settings(w, h);
                    rs.noise_threshold = if settings.viewport_noise_threshold { settings.viewport_adaptive_threshold.max(0.0) } else { 0.0 };
                    Backend::Path(Box::new(Progressive::new(&s, &rs)))
                }
                Engine::Eevee => Backend::Eevee(Box::new(Eevee::new(EeveeScene::new(scene, preview), &eevee_settings(settings, w, h, preview)))),
                Engine::Workbench => {
                    let mut s = scene.clone();
                    s.selected.clear();
                    s.active_object = None;
                    Backend::Workbench { scene: Box::new(ViewportScene::new(&s)), done: false }
                }
            };
            *slot = Some(RenderView { scene_key, view_key, backend, rgba: vec![0; (w * h * 4) as usize], w, h, target: 1, cm });
        }
        let Some(rv) = slot.as_mut() else { return false };
        rv.cm = cm;
        rv.target = match engine {
            Engine::PathTracer => settings.viewport_samples.max(1),
            Engine::Eevee => eevee_settings(settings, w, h, preview).samples,
            Engine::Workbench => 1,
        };
        if let Backend::Eevee(e) = &mut rv.backend {
            // The prepared scene stays; view, size and settings restart sampling.
            e.set_settings(&eevee_settings(settings, w, h, preview));
            e.set_view(RasterCamera::from_view(view, w, h));
            rv.view_key = view_key;
            rv.w = w;
            rv.h = h;
        }
        if rv.wants_samples() {
            match &mut rv.backend {
                Backend::Path(p) => {
                    p.add_sample(None);
                    rv.rgba = to_display8(&p.image(), &rv.cm);
                }
                Backend::Eevee(e) => {
                    e.add_sample();
                    rv.rgba = to_display8(&e.image(), &rv.cm);
                }
                Backend::Workbench { scene: vs, done } => {
                    let st = ViewportSettings { width: w, height: h, threads: settings.threads as usize, workbench: settings.workbench, overlays: false, background: Some(world_bg(scene)), ..ViewportSettings::default() };
                    vs.draw_into(view, &st, &mut rv.rgba);
                    *done = true;
                }
            }
        }
        rv.wants_samples()
    }

    pub fn samples(&self) -> u32 {
        match &self.backend {
            Backend::Path(p) => p.samples(),
            Backend::Eevee(e) => e.samples(),
            Backend::Workbench { done, .. } => *done as u32,
        }
    }

    /// What the view is doing while it refines.
    pub fn activity(&self) -> &'static str {
        match &self.backend {
            Backend::Path(_) => "Path Tracing",
            _ => "Rendering",
        }
    }

    fn wants_samples(&self) -> bool {
        match &self.backend {
            Backend::Path(p) => p.samples() < self.target && !p.converged(),
            Backend::Eevee(e) => e.wants_samples(),
            Backend::Workbench { done, .. } => !done,
        }
    }
}

/// The world colour as a display-space background (Workbench).
fn world_bg(scene: &Scene) -> [f32; 3] {
    let w = scene.world.color * if scene.world.strength.is_finite() { scene.world.strength.max(0.0) } else { 0.0 };
    [w.x, w.y, w.z].map(|c| srgb_encode(c.clamp(0.0, 1.0)))
}

/// EEVEE's viewport settings; Material Preview uses the studio environment.
fn eevee_settings(settings: &Settings, w: u32, h: u32, preview: bool) -> sex_3d::render::EeveeSettings {
    let mut st = settings.eevee_settings(w, h, true);
    if preview {
        st.lookdev = true;
        st.film_transparent = false;
    }
    st
}
