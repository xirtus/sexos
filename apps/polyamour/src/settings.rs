// SPDX-License-Identifier: GPL-3.0-only
//! Scene settings Blender keeps that `sex_3d::Scene` has no field for: the
//! render engine and its sampling, output resolution and path, units, colour
//! management, and per-object / per-collection flags the Outliner and the
//! Properties editor show (disable in renders, exclude, rotation mode, ...).
//!
//! Per-object entries are keyed by object name (names are unique in a scene,
//! like Blender's); renaming through the Outliner or Properties carries them
//! over with [`Settings::rename_object`].
use sex_3d::modifiers::Modifier;
use sex_3d::render::{ColorManagement, EeveeSettings, LightPaths, Look, MistFalloff, MistSettings, MotionBlur, Pass, Passes, RenderBorder, RenderResult, RenderSettings, ShutterPosition, ViewTransform, Workbench, WorkbenchRender};
use sex_3d::scene::Scene;
use std::collections::{BTreeMap, BTreeSet};

/// Render engines (Blender's are EEVEE, Workbench and Cycles).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// The rasterized engine shading node trees per pixel (EEVEE).
    Eevee,
    /// The Solid view's engine (Workbench).
    Workbench,
    /// sex-3d's CPU path tracer (Cycles' place).
    PathTracer,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::Eevee => "EEVEE",
            Engine::Workbench => "Workbench",
            Engine::PathTracer => "Path Tracer",
        }
    }
    pub const ALL: [Engine; 3] = [Engine::Eevee, Engine::Workbench, Engine::PathTracer];
    pub fn from_label(v: &str) -> Option<Engine> {
        Engine::ALL.into_iter().find(|e| e.label() == v)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitSystem {
    None,
    Metric,
    Imperial,
}

impl UnitSystem {
    pub fn label(self) -> &'static str {
        match self {
            UnitSystem::None => "None",
            UnitSystem::Metric => "Metric",
            UnitSystem::Imperial => "Imperial",
        }
    }
    pub const ALL: [UnitSystem; 3] = [UnitSystem::None, UnitSystem::Metric, UnitSystem::Imperial];
}

/// Blender's object rotation modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationMode {
    Quaternion,
    EulerXyz,
    AxisAngle,
}

impl RotationMode {
    pub fn label(self) -> &'static str {
        match self {
            RotationMode::Quaternion => "Quaternion (WXYZ)",
            RotationMode::EulerXyz => "XYZ Euler",
            RotationMode::AxisAngle => "Axis Angle",
        }
    }
}

/// A modifier switched off with its "Show in Viewport" toggle: it is taken out
/// of the object's stack (so viewport and render skip it) and kept here.
#[derive(Clone, Debug, PartialEq)]
pub struct ParkedModifier {
    pub object: String,
    /// Its position in the full (shown) stack.
    pub slot: usize,
    pub modifier: Modifier,
    /// The object's live stack when it was parked, to notice undo restoring it.
    pub stack: Vec<Modifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub engine: Engine,
    pub viewport_samples: u32,
    pub render_samples: u32,
    /// Adaptive sampling for F12 renders, its threshold and minimum samples
    /// (0 = automatic).
    pub noise_threshold: bool,
    pub adaptive_threshold: f32,
    pub adaptive_min_samples: u32,
    /// Adaptive sampling in the Rendered viewport.
    pub viewport_noise_threshold: bool,
    pub viewport_adaptive_threshold: f32,
    /// Seconds (0 = no limit).
    pub time_limit: f32,
    pub max_bounces: u32,
    pub diffuse_bounces: u32,
    pub glossy_bounces: u32,
    pub transmission_bounces: u32,
    pub volume_bounces: u32,
    pub transparent_bounces: u32,
    pub denoise: bool,
    pub viewport_denoise: bool,
    /// Clamp direct and indirect light (0 = off).
    pub clamp_direct: f32,
    pub clamp_indirect: f32,
    pub caustics_reflective: bool,
    pub caustics_refractive: bool,
    pub filter_glossy: f32,
    pub seed: u32,
    pub animated_seed: bool,
    pub film_transparent: bool,
    pub filter_width: f32,
    /// Worker threads (0 = auto-detect).
    pub threads: u32,
    pub motion_blur: bool,
    /// Shutter in frames.
    pub shutter: f32,
    pub shutter_position: ShutterPosition,
    /// Render region (Blender's border, 0..1 with y up) and cropping.
    pub use_border: bool,
    pub border: [f32; 4],
    pub crop_to_border: bool,
    pub resolution_x: u32,
    pub resolution_y: u32,
    pub resolution_pct: u32,
    pub frame_step: i32,
    pub output_path: String,
    pub file_format: String,
    pub color_mode: String,
    pub unit_system: UnitSystem,
    pub unit_scale: f32,
    /// Rotation fields in degrees (else radians).
    pub degrees: bool,
    pub gravity_on: bool,
    pub gravity: [f32; 3],
    pub view_transform: ViewTransform,
    pub look: String,
    pub exposure: f32,
    pub gamma: f32,
    /// View layer and passes.
    pub use_for_rendering: bool,
    pub passes: Passes,
    pub mist_start: f32,
    pub mist_depth: f32,
    pub mist_falloff: MistFalloff,
    pub ao_distance: f32,
    /// Objects whose camera toggle is off ("Disable in Renders").
    pub hide_render: BTreeSet<String>,
    /// Collections by index: excluded from the view layer (outliner checkbox).
    pub excluded: BTreeSet<usize>,
    /// Collections hidden with the eye (the scene's `visible` combines both).
    pub collection_hidden: BTreeSet<usize>,
    /// Collections disabled in renders.
    pub collection_hide_render: BTreeSet<usize>,
    pub rotation_mode: BTreeMap<String, RotationMode>,
    /// Camera sensor width (mm) by camera object name; 36 when missing.
    pub sensor: BTreeMap<String, f32>,
    pub parked: Vec<ParkedModifier>,
    /// Outliner's active collection (Blender adds new objects there).
    pub active_collection: usize,
    /// EEVEE: samples for the viewport and for F12.
    pub eevee_viewport_samples: u32,
    pub eevee_render_samples: u32,
    pub eevee_shadows: bool,
    /// Shadow map resolution (cube faces and sun cascades).
    pub eevee_shadow_resolution: u32,
    pub eevee_soft_shadows: bool,
    pub eevee_shadow_cascades: u32,
    pub eevee_ao: bool,
    pub eevee_ao_distance: f32,
    pub eevee_volumetrics: bool,
    pub eevee_volumetric_end: f32,
    /// Volumes: ray-march steps through object volumes, and volume shadows.
    pub eevee_volumetric_steps: u32,
    pub eevee_volumetric_shadows: bool,
    /// Raytracing (screen space): reflections and their limits.
    pub eevee_ssr: bool,
    pub eevee_ssr_max_roughness: f32,
    pub eevee_ssr_thickness: f32,
    pub eevee_ssr_fade: f32,
    /// Bloom.
    pub eevee_bloom: bool,
    pub eevee_bloom_threshold: f32,
    pub eevee_bloom_knee: f32,
    pub eevee_bloom_radius: f32,
    pub eevee_bloom_color: [f32; 3],
    pub eevee_bloom_intensity: f32,
    pub eevee_bloom_clamp: f32,
    /// Depth of Field: Max Size (px) and Jitter Camera.
    pub eevee_dof_max_size: f32,
    pub eevee_dof_jitter: bool,
    /// Subsurface Scattering: samples.
    pub eevee_sss_samples: u32,
    /// Material Preview: studio world (index into `STUDIO_WORLDS`), its
    /// rotation (radians), strength and opacity behind the scene.
    pub lookdev_world: usize,
    pub lookdev_rotation: f32,
    pub lookdev_strength: f32,
    pub lookdev_opacity: f32,
    /// Performance > Viewport: render at half resolution while navigating.
    pub viewport_half_res_navigation: bool,
    /// Workbench (F12): shading options and anti-aliasing samples.
    pub workbench: Workbench,
    pub workbench_samples: u32,
    /// Workbench anti-aliasing samples in the Rendered viewport.
    pub workbench_viewport_samples: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            engine: Engine::PathTracer,
            viewport_samples: 8,
            // Blender's Cycles defaults are 1024/4096, far beyond a CPU tracer
            // on SexOS; these keep F12 interactive.
            render_samples: 16,
            noise_threshold: true,
            adaptive_threshold: 0.01,
            adaptive_min_samples: 0,
            viewport_noise_threshold: true,
            viewport_adaptive_threshold: 0.1,
            time_limit: 0.0,
            max_bounces: 12,
            diffuse_bounces: 4,
            glossy_bounces: 4,
            transmission_bounces: 12,
            volume_bounces: 0,
            transparent_bounces: 8,
            denoise: true,
            viewport_denoise: false,
            clamp_direct: 0.0,
            clamp_indirect: 10.0,
            caustics_reflective: true,
            caustics_refractive: true,
            filter_glossy: 1.0,
            seed: 0,
            animated_seed: false,
            film_transparent: false,
            filter_width: 1.5,
            threads: 0,
            motion_blur: false,
            shutter: 0.5,
            shutter_position: ShutterPosition::Center,
            use_border: false,
            border: [0.0, 0.0, 1.0, 1.0],
            crop_to_border: false,
            resolution_x: 1920,
            resolution_y: 1080,
            resolution_pct: 50,
            frame_step: 1,
            output_path: "/tmp/".into(),
            file_format: "PNG".into(),
            color_mode: "RGBA".into(),
            unit_system: UnitSystem::Metric,
            unit_scale: 1.0,
            degrees: true,
            gravity_on: true,
            gravity: [0.0, 0.0, -9.81],
            view_transform: ViewTransform::AgX,
            look: "None".into(),
            exposure: 0.0,
            gamma: 1.0,
            use_for_rendering: true,
            passes: Passes::NONE.with(Pass::Depth),
            mist_start: 5.0,
            mist_depth: 25.0,
            mist_falloff: MistFalloff::Quadratic,
            ao_distance: 10.0,
            hide_render: BTreeSet::new(),
            excluded: BTreeSet::new(),
            collection_hidden: BTreeSet::new(),
            collection_hide_render: BTreeSet::new(),
            rotation_mode: BTreeMap::new(),
            sensor: BTreeMap::new(),
            parked: Vec::new(),
            active_collection: 0,
            eevee_viewport_samples: 16,
            eevee_render_samples: 64,
            eevee_shadows: true,
            eevee_shadow_resolution: 1024,
            eevee_soft_shadows: true,
            eevee_shadow_cascades: 3,
            eevee_ao: true,
            eevee_ao_distance: 0.2,
            eevee_volumetrics: true,
            eevee_volumetric_end: 100.0,
            eevee_volumetric_steps: 64,
            eevee_volumetric_shadows: false,
            eevee_ssr: false,
            eevee_ssr_max_roughness: 0.5,
            eevee_ssr_thickness: 0.2,
            eevee_ssr_fade: 0.075,
            eevee_bloom: false,
            eevee_bloom_threshold: 0.8,
            eevee_bloom_knee: 0.5,
            eevee_bloom_radius: 6.5,
            eevee_bloom_color: [1.0; 3],
            eevee_bloom_intensity: 0.05,
            eevee_bloom_clamp: 0.0,
            eevee_dof_max_size: 100.0,
            eevee_dof_jitter: false,
            eevee_sss_samples: 7,
            lookdev_world: 0,
            lookdev_rotation: 0.0,
            lookdev_strength: 1.0,
            lookdev_opacity: 1.0,
            viewport_half_res_navigation: false,
            workbench: Workbench::default(),
            workbench_samples: 8,
            workbench_viewport_samples: 8,
        }
    }
}

impl Settings {
    /// Output size after the percentage, at least 1x1.
    pub fn output_size(&self) -> (u32, u32) {
        let f = |v: u32| ((v as u64 * self.resolution_pct as u64 / 100) as u32).max(1);
        (f(self.resolution_x), f(self.resolution_y))
    }

    /// Light Paths as the renderer takes them.
    pub fn light_paths(&self) -> LightPaths {
        LightPaths {
            diffuse_bounces: self.diffuse_bounces,
            glossy_bounces: self.glossy_bounces,
            transmission_bounces: self.transmission_bounces,
            volume_bounces: self.volume_bounces,
            transparent_bounces: self.transparent_bounces,
            clamp_direct: self.clamp_direct.max(0.0),
            caustics_reflective: self.caustics_reflective,
            caustics_refractive: self.caustics_refractive,
            filter_glossy: self.filter_glossy.max(0.0),
        }
    }

    /// Settings shared by F12 and the Rendered viewport.
    fn path_tracer(&self, width: u32, height: u32) -> RenderSettings {
        RenderSettings {
            width,
            height,
            max_bounces: self.max_bounces,
            threads: self.threads as usize,
            clamp: self.clamp_indirect.max(0.0),
            seed: self.seed,
            light_paths: self.light_paths(),
            animated_seed: self.animated_seed,
            film_transparent: self.film_transparent,
            filter_width: self.filter_width,
            mist: MistSettings { start: self.mist_start, depth: self.mist_depth, falloff: self.mist_falloff },
            ao_distance: self.ao_distance,
            motion_blur: self.motion_blur.then_some(MotionBlur { shutter: self.shutter, position: self.shutter_position }),
            ..Default::default()
        }
    }

    /// The path tracer's settings for an F12 render.
    pub fn render_settings(&self) -> RenderSettings {
        let (width, height) = self.output_size();
        let border = self.use_border.then_some(RenderBorder { min_x: self.border[0], min_y: self.border[1], max_x: self.border[2], max_y: self.border[3], crop: self.crop_to_border });
        match self.engine {
            Engine::PathTracer => RenderSettings {
                samples: self.render_samples.max(1),
                denoise: self.denoise,
                noise_threshold: if self.noise_threshold { self.adaptive_threshold.max(0.0) } else { 0.0 },
                min_samples: self.adaptive_min_samples,
                time_limit: self.time_limit.max(0.0),
                passes: if self.use_for_rendering { self.passes } else { Passes::NONE },
                border,
                ..self.path_tracer(width, height)
            },
            Engine::Workbench | Engine::Eevee => RenderSettings { width, height, samples: 1, max_bounces: 0, denoise: false, clamp: 0.0, seed: self.seed, film_transparent: self.film_transparent, border, light_paths: LightPaths::unlimited(), ..Default::default() },
        }
    }

    /// Settings for the Rendered viewport at `w` x `h` (samples are added
    /// progressively up to [`Settings::viewport_samples`]).
    pub fn viewport_render_settings(&self, w: u32, h: u32) -> RenderSettings {
        RenderSettings { samples: 1, denoise: false, ..self.path_tracer(w, h) }
    }

    /// EEVEE's settings at `width` x `height`, for the viewport or F12.
    pub fn eevee_settings(&self, width: u32, height: u32, viewport: bool) -> EeveeSettings {
        EeveeSettings {
            width,
            height,
            samples: if viewport { self.eevee_viewport_samples } else { self.eevee_render_samples }.max(1),
            shadows: self.eevee_shadows,
            shadow_resolution: self.eevee_shadow_resolution.clamp(64, 8192),
            soft_shadows: self.eevee_soft_shadows,
            shadow_cascades: self.eevee_shadow_cascades.clamp(1, 4),
            ao: self.eevee_ao,
            ao_distance: self.eevee_ao_distance.max(0.0),
            volumetrics: self.eevee_volumetrics,
            volumetric_end: self.eevee_volumetric_end.max(0.0),
            volumetric_steps: self.eevee_volumetric_steps.clamp(1, 256),
            volumetric_shadows: self.eevee_volumetric_shadows,
            ssr: self.eevee_ssr,
            ssr_max_roughness: self.eevee_ssr_max_roughness.clamp(0.0, 1.0),
            ssr_thickness: self.eevee_ssr_thickness.max(0.0),
            ssr_fade: self.eevee_ssr_fade.clamp(0.0, 0.5),
            bloom: self.eevee_bloom,
            bloom_threshold: self.eevee_bloom_threshold.max(0.0),
            bloom_knee: self.eevee_bloom_knee.clamp(0.0, 1.0),
            bloom_radius: self.eevee_bloom_radius.clamp(0.0, 10.0),
            bloom_color: self.eevee_bloom_color.map(|c| c.max(0.0)),
            bloom_intensity: self.eevee_bloom_intensity.max(0.0),
            bloom_clamp: self.eevee_bloom_clamp.max(0.0),
            dof_max_size: self.eevee_dof_max_size.max(0.0),
            dof_jitter: self.eevee_dof_jitter,
            sss_samples: self.eevee_sss_samples.clamp(1, 32),
            motion_blur: self.motion_blur.then(|| MotionBlur { shutter: self.shutter, position: self.shutter_position }.interval()),
            filter_size: self.filter_width.max(0.0),
            film_transparent: self.film_transparent,
            lookdev_world: self.lookdev_world,
            lookdev_rotation: self.lookdev_rotation,
            lookdev_strength: self.lookdev_strength.max(0.0),
            lookdev_opacity: self.lookdev_opacity.clamp(0.0, 1.0),
            threads: self.threads as usize,
            ..EeveeSettings::default()
        }
    }

    /// Workbench's settings for an F12 render.
    pub fn workbench_render(&self) -> WorkbenchRender {
        let (width, height) = self.output_size();
        WorkbenchRender { width, height, samples: self.workbench_samples.max(1), shading: self.workbench, film_transparent: self.film_transparent, backface_culling: false, threads: self.threads as usize }
    }

    /// Render `scene` with the chosen engine (F12); `rs` (from
    /// [`Settings::render_settings`]) gives the size and frame.
    pub fn render_with_engine(&self, scene: &Scene, rs: &RenderSettings) -> RenderResult {
        let raster = |color, samples| RenderResult { color, samples, complete: true, ..Default::default() };
        match self.engine {
            Engine::PathTracer => sex_3d::render::render(scene, rs),
            Engine::Eevee => {
                let st = self.eevee_settings(rs.width, rs.height, false);
                raster(sex_3d::render::render_eevee(scene, &st), st.samples)
            }
            Engine::Workbench => {
                let st = WorkbenchRender { width: rs.width, height: rs.height, ..self.workbench_render() };
                raster(sex_3d::render::render_workbench(scene, &st), st.samples)
            }
        }
    }

    /// [`sex_3d::render::render_frames`] with the chosen engine.
    pub fn render_frames(&self, frames: impl IntoIterator<Item = i32>, rs: &RenderSettings, mut scene_at: impl FnMut(i32) -> Scene, mut on_frame: impl FnMut(i32, RenderResult) -> bool) -> usize {
        if self.engine == Engine::PathTracer {
            return sex_3d::render::render_frames(frames, rs, scene_at, on_frame, None);
        }
        let mut n = 0;
        for f in frames {
            let r = self.render_with_engine(&scene_at(f), &RenderSettings { frame: f, ..*rs });
            n += 1;
            if !on_frame(f, r) {
                break;
            }
        }
        n
    }

    /// Colour management for display.
    pub fn color_management(&self) -> ColorManagement {
        ColorManagement { view: self.view_transform, look: Look::from_label(&self.look).unwrap_or_default(), exposure: self.exposure, gamma: self.gamma }
    }

    /// The scene as the renderer should see it: render visibility instead of
    /// viewport visibility (Blender's eye only hides in the viewport).
    pub fn render_scene(&self, scene: &Scene) -> Scene {
        let mut s = scene.clone();
        for o in &mut s.objects {
            o.visible = !self.hide_render.contains(&o.name);
        }
        for (i, c) in s.collections.iter_mut().enumerate() {
            c.visible = !self.excluded.contains(&i) && !self.collection_hide_render.contains(&i);
        }
        s
    }

    pub fn rotation_mode(&self, object: &str) -> RotationMode {
        self.rotation_mode.get(object).copied().unwrap_or(RotationMode::EulerXyz)
    }
    pub fn sensor_width(&self, camera: &str) -> f32 {
        self.sensor.get(camera).copied().unwrap_or(36.0)
    }

    /// Move sensor sizes kept here by older files onto the cameras.
    pub fn migrate_sensors(&mut self, scene: &mut Scene) {
        for (name, w) in std::mem::take(&mut self.sensor) {
            if let Some(o) = scene.objects.iter_mut().find(|o| o.name == name) {
                if let sex_3d::scene::ObjectData::Camera(c) = &mut o.data {
                    c.sensor_width = w;
                }
            }
        }
    }

    /// Carry per-object entries over to a new name.
    pub fn rename_object(&mut self, old: &str, new: &str) {
        if old == new {
            return;
        }
        if self.hide_render.remove(old) {
            self.hide_render.insert(new.to_string());
        }
        if let Some(m) = self.rotation_mode.remove(old) {
            self.rotation_mode.insert(new.to_string(), m);
        }
        if let Some(m) = self.sensor.remove(old) {
            self.sensor.insert(new.to_string(), m);
        }
        for p in &mut self.parked {
            if p.object == old {
                p.object = new.to_string();
            }
        }
    }

    /// Collection `idx` was removed: shift the per-collection indices.
    pub fn collection_removed(&mut self, idx: usize) {
        for set in [&mut self.excluded, &mut self.collection_hidden, &mut self.collection_hide_render] {
            *set = set.iter().filter(|&&i| i != idx).map(|&i| if i > idx { i - 1 } else { i }).collect();
        }
        if self.active_collection >= idx && self.active_collection > 0 {
            self.active_collection -= 1;
        }
    }

    /// Set the scene's collection visibility from exclude and eye flags.
    pub fn sync_collections(&self, scene: &mut Scene) {
        for (i, c) in scene.collections.iter_mut().enumerate() {
            c.visible = !self.excluded.contains(&i) && !self.collection_hidden.contains(&i);
        }
    }

    /// Drop parked modifiers that no longer fit the scene (the object went
    /// away, or undo put the modifier back into the live stack).
    pub fn validate_parked(&mut self, scene: &Scene) {
        self.parked.retain_mut(|p| {
            let Some(o) = scene.objects.iter().find(|o| o.name == p.object) else { return false };
            if o.modifiers == p.stack {
                return true;
            }
            // Undo restored the stack from before parking: the modifier is live again.
            if o.modifiers.len() == p.stack.len() + 1 && p.slot < o.modifiers.len() && o.modifiers[p.slot] == p.modifier {
                let mut rest = o.modifiers.clone();
                rest.remove(p.slot);
                if rest == p.stack {
                    return false;
                }
            }
            // The stack was edited since: keep it parked against the new stack.
            p.stack = o.modifiers.clone();
            p.slot = p.slot.min(o.modifiers.len() + 1);
            true
        });
    }

    /// The object's full modifier list as shown: (modifier, shown in viewport,
    /// live index or parked index).
    pub fn shown_stack(&self, scene: &Scene, obj: usize) -> Vec<(Modifier, bool, usize)> {
        let o = &scene.objects[obj];
        let mut out: Vec<(Modifier, bool, usize)> = o.modifiers.iter().cloned().enumerate().map(|(i, m)| (m, true, i)).collect();
        let mut parked: Vec<(usize, &ParkedModifier)> = self.parked.iter().enumerate().filter(|(_, p)| p.object == o.name).collect();
        parked.sort_by_key(|(_, p)| p.slot);
        for (pi, p) in parked {
            let at = p.slot.min(out.len());
            out.insert(at, (p.modifier.clone(), false, pi));
        }
        out
    }

    /// Toggle "Show in Viewport" of entry `shown` in [`Settings::shown_stack`].
    pub fn toggle_modifier(&mut self, scene: &mut Scene, obj: usize, shown: usize) {
        let stack = self.shown_stack(scene, obj);
        let Some((m, live, idx)) = stack.get(shown).cloned() else { return };
        if live {
            scene.objects[obj].modifiers.remove(idx);
            self.parked.push(ParkedModifier { object: scene.objects[obj].name.clone(), slot: shown, modifier: m, stack: scene.objects[obj].modifiers.clone() });
        } else {
            let p = self.parked.remove(idx);
            // Live modifiers before this entry in the shown order.
            let before = stack[..shown].iter().filter(|e| e.1).count();
            scene.objects[obj].modifiers.insert(before, p.modifier);
            // Re-anchor the other parked entries of this object.
            let name = scene.objects[obj].name.clone();
            let live = scene.objects[obj].modifiers.clone();
            for q in self.parked.iter_mut().filter(|q| q.object == name) {
                q.stack = live.clone();
            }
        }
        for q in self.parked.iter_mut().filter(|q| q.object == scene.objects[obj].name) {
            q.stack = scene.objects[obj].modifiers.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_size_and_render_scene() {
        let mut s = Settings::default();
        assert_eq!(s.output_size(), (960, 540));
        s.resolution_pct = 100;
        assert_eq!(s.render_settings().width, 1920);
        let mut scene = Scene::startup();
        scene.objects[0].visible = false;
        s.hide_render.insert("Light".into());
        let r = s.render_scene(&scene);
        assert!(r.objects[0].visible, "eye only hides in the viewport");
        assert!(!r.objects[r.find_object("Light").unwrap()].visible);
    }

    #[test]
    fn parking_a_modifier_and_undo() {
        let mut scene = Scene::startup();
        let mut s = Settings::default();
        scene.objects[0].modifiers = vec![Modifier::Triangulate, Modifier::Weld { distance: 0.1 }];
        let before = scene.clone();
        s.toggle_modifier(&mut scene, 0, 0);
        assert_eq!(scene.objects[0].modifiers, vec![Modifier::Weld { distance: 0.1 }]);
        let shown = s.shown_stack(&scene, 0);
        assert_eq!(shown.len(), 2);
        assert!(!shown[0].1 && shown[1].1);
        s.toggle_modifier(&mut scene, 0, 0);
        assert_eq!(scene.objects[0].modifiers, before.objects[0].modifiers);
        assert!(s.parked.is_empty());
        // Undo restoring the old stack un-parks.
        s.toggle_modifier(&mut scene, 0, 1);
        s.validate_parked(&before);
        assert!(s.parked.is_empty());
    }
}
