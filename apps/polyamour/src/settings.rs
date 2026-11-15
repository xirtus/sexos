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
use sex_3d::render::ViewTransform;
use sex_3d::render::RenderSettings;
use sex_3d::scene::Scene;
use std::collections::{BTreeMap, BTreeSet};

/// Render engines, named plainly (Blender's are EEVEE, Workbench and Cycles).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// sex-3d's CPU path tracer (Cycles' place).
    PathTracer,
    /// Fast direct-light preview (Workbench's place).
    Workbench,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::PathTracer => "Path Tracer",
            Engine::Workbench => "Workbench",
        }
    }
    pub const ALL: [Engine; 2] = [Engine::PathTracer, Engine::Workbench];
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
    pub noise_threshold: bool,
    pub max_bounces: u32,
    pub denoise: bool,
    pub viewport_denoise: bool,
    /// Clamp indirect light (0 = off).
    pub clamp_indirect: f32,
    pub seed: u32,
    pub film_transparent: bool,
    pub filter_width: f32,
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
    pub pass_z: bool,
    pub pass_normal: bool,
    pub pass_denoising: bool,
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            engine: Engine::PathTracer,
            viewport_samples: 8,
            // Blender's Cycles defaults are 1024/4096, far beyond a CPU tracer
            // on SexOS; these keep F12 interactive.
            render_samples: 16,
            noise_threshold: false,
            max_bounces: 12,
            denoise: true,
            viewport_denoise: false,
            clamp_indirect: 10.0,
            seed: 0,
            film_transparent: false,
            filter_width: 1.5,
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
            pass_z: true,
            pass_normal: false,
            pass_denoising: false,
            hide_render: BTreeSet::new(),
            excluded: BTreeSet::new(),
            collection_hidden: BTreeSet::new(),
            collection_hide_render: BTreeSet::new(),
            rotation_mode: BTreeMap::new(),
            sensor: BTreeMap::new(),
            parked: Vec::new(),
            active_collection: 0,
        }
    }
}

impl Settings {
    /// Output size after the percentage, at least 1x1.
    pub fn output_size(&self) -> (u32, u32) {
        let f = |v: u32| ((v as u64 * self.resolution_pct as u64 / 100) as u32).max(1);
        (f(self.resolution_x), f(self.resolution_y))
    }

    /// The path tracer's settings for an F12 render.
    pub fn render_settings(&self) -> RenderSettings {
        let (width, height) = self.output_size();
        match self.engine {
            Engine::PathTracer => RenderSettings {
                width,
                height,
                samples: self.render_samples.max(1),
                max_bounces: self.max_bounces,
                denoise: self.denoise,
                clamp: self.clamp_indirect.max(0.0),
                seed: self.seed,
                ..Default::default()
            },
            Engine::Workbench => RenderSettings { width, height, samples: 1, max_bounces: 0, denoise: false, clamp: 0.0, seed: self.seed, ..Default::default() },
        }
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
