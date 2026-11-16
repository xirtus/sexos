// SPDX-License-Identifier: GPL-3.0-only
//! Grease Pencil, modelled on Blender 4.3's "GPv3" data: a [`GreasePencil`]
//! data-block holds layers (in layer groups), each layer holds keyframes
//! (frame number -> drawing) and a drawing is a set of curves (strokes) whose
//! points carry position, radius, opacity, vertex colour and rotation.
//! Materials carry the Grease Pencil stroke and fill settings.
//!
//! Layout:
//! - this file: the data model, frame lookup and sample objects.
//! - [`geom`]: stroke geometry (length, resampling, simplify, smooth,
//!   subdivide, ear-clipping triangulation).
//! - [`view`]: the projection strokes are drawn and edited through, and the
//!   drawing planes.
//! - [`tools`]: Draw Mode (draw, fill, erase, tint, primitives).
//! - [`edit`]: Edit Mode operators.
//! - [`sculpt`]: Sculpt, Vertex Paint and Weight Paint brushes.
//! - [`modifiers`]: the Grease Pencil modifiers and evaluation.
//! - [`effects`]: visual effects (2D post on the object's layer).
//! - [`raster`]: anti-aliased stroke and fill rasterization and compositing.
//! - [`interp`]: interpolation between keyframes.
//! - [`lineart`]: the Line Art modifier's edge extraction.
use crate::math::{Mat4, Vec3};
use crate::scene::Transform;

pub mod edit;
pub mod effects;
pub mod geom;
pub mod interp;
pub mod lineart;
pub mod modifiers;
pub mod raster;
pub mod sculpt;
pub mod tools;
pub mod view;
#[cfg(test)]
mod tests;

pub use effects::GpEffect;
pub use modifiers::GpModifier;

/// Blender's legacy stroke thickness to GPv3 radius factor (1 / 2000).
pub const LEGACY_RADIUS_FACTOR: f32 = 1.0 / 2000.0;

/// One control point of a stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct GpPoint {
    /// Layer space.
    pub position: Vec3,
    /// Half the stroke width, in layer units.
    pub radius: f32,
    /// 0..1 alpha of the point.
    pub opacity: f32,
    /// Linear RGBA; alpha is how much it replaces the material colour.
    pub vertex_color: [f32; 4],
    /// Texture rotation of dots and squares (radians).
    pub rotation: f32,
    pub select: bool,
    /// Vertex group weights: (group index, weight).
    pub weights: Vec<(u32, f32)>,
}

impl GpPoint {
    pub fn new(position: Vec3, radius: f32) -> GpPoint {
        GpPoint { position, radius, opacity: 1.0, vertex_color: [0.0; 4], rotation: 0.0, select: false, weights: Vec::new() }
    }
    pub fn weight(&self, group: u32) -> f32 {
        self.weights.iter().find(|w| w.0 == group).map_or(0.0, |w| w.1)
    }
    pub fn set_weight(&mut self, group: u32, w: f32) {
        match self.weights.iter_mut().find(|x| x.0 == group) {
            Some(x) => x.1 = w,
            None => self.weights.push((group, w)),
        }
    }
    /// Linear blend of two points (for resampling and interpolation).
    pub fn lerp(&self, o: &GpPoint, t: f32) -> GpPoint {
        let l = |a: f32, b: f32| a + (b - a) * t;
        let mut c = [0.0; 4];
        for i in 0..4 {
            c[i] = l(self.vertex_color[i], o.vertex_color[i]);
        }
        GpPoint {
            position: self.position.lerp(o.position, t),
            radius: l(self.radius, o.radius),
            opacity: l(self.opacity, o.opacity),
            vertex_color: c,
            rotation: l(self.rotation, o.rotation),
            select: self.select && o.select,
            weights: if t < 0.5 { self.weights.clone() } else { o.weights.clone() },
        }
    }
}

/// Stroke end shapes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CapType {
    #[default]
    Round,
    Flat,
}

/// A stroke (one curve of a drawing).
#[derive(Clone, Debug, PartialEq)]
pub struct GpStroke {
    pub points: Vec<GpPoint>,
    pub cyclic: bool,
    /// Index into the data's material slots.
    pub material: usize,
    /// Linear RGBA fill vertex colour (alpha: how much it replaces the material's).
    pub fill_color: [f32; 4],
    pub fill_opacity: f32,
    /// 0 hard edge .. 1 fully soft (Blender's `softness`, from brush hardness).
    pub softness: f32,
    pub start_cap: CapType,
    pub end_cap: CapType,
    pub select: bool,
    /// Texture mapping along and across the stroke (Texture Mapping modifier).
    pub uv_offset: f32,
    pub uv_scale: f32,
    pub uv_rotation: f32,
    /// Seconds from the start of the drawing session when this stroke was drawn (Build: natural time).
    pub init_time: f32,
}

impl GpStroke {
    pub fn new(points: Vec<GpPoint>, material: usize) -> GpStroke {
        GpStroke {
            points,
            cyclic: false,
            material,
            fill_color: [0.0; 4],
            fill_opacity: 1.0,
            softness: 0.0,
            start_cap: CapType::Round,
            end_cap: CapType::Round,
            select: false,
            uv_offset: 0.0,
            uv_scale: 1.0,
            uv_rotation: 0.0,
            init_time: 0.0,
        }
    }
    /// A stroke through `pts` with one radius.
    pub fn from_positions(pts: &[Vec3], radius: f32, material: usize) -> GpStroke {
        GpStroke::new(pts.iter().map(|&p| GpPoint::new(p, radius)).collect(), material)
    }
    pub fn positions(&self) -> Vec<Vec3> {
        self.points.iter().map(|p| p.position).collect()
    }
    pub fn length(&self) -> f32 {
        geom::polyline_length(&self.positions(), self.cyclic)
    }
    /// A copy with only the points in `range`, other settings kept.
    pub fn sub(&self, range: std::ops::Range<usize>) -> GpStroke {
        GpStroke { points: self.points[range].to_vec(), cyclic: false, ..self.clone() }
    }
    pub fn any_selected(&self) -> bool {
        self.select || self.points.iter().any(|p| p.select)
    }
}

/// The strokes of one keyframe.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawing {
    pub strokes: Vec<GpStroke>,
}

impl Drawing {
    pub fn point_count(&self) -> usize {
        self.strokes.iter().map(|s| s.points.len()).sum()
    }
    pub fn transform(&mut self, m: &Mat4) {
        let s = scale_of(m);
        for st in &mut self.strokes {
            for p in &mut st.points {
                p.position = m.transform_point(p.position);
                p.radius *= s;
            }
        }
    }
}

/// Average scale of a matrix (for radii).
pub fn scale_of(m: &Mat4) -> f32 {
    let (_, _, s) = m.to_trs();
    ((s.x.abs() * s.y.abs() * s.z.abs()).cbrt()).max(1e-8)
}

/// Blender's keyframe types (the Dope Sheet's R menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyframeType {
    #[default]
    Keyframe,
    Breakdown,
    MovingHold,
    Extreme,
    Jitter,
    Generated,
}

impl KeyframeType {
    pub const ALL: [KeyframeType; 6] = [KeyframeType::Keyframe, KeyframeType::Breakdown, KeyframeType::MovingHold, KeyframeType::Extreme, KeyframeType::Jitter, KeyframeType::Generated];
    pub fn id(self) -> &'static str {
        match self {
            KeyframeType::Keyframe => "KEYFRAME",
            KeyframeType::Breakdown => "BREAKDOWN",
            KeyframeType::MovingHold => "MOVING_HOLD",
            KeyframeType::Extreme => "EXTREME",
            KeyframeType::Jitter => "JITTER",
            KeyframeType::Generated => "GENERATED",
        }
    }
    pub fn from_id(s: &str) -> Option<KeyframeType> {
        KeyframeType::ALL.into_iter().find(|k| k.id().eq_ignore_ascii_case(s))
    }
    /// Blender's `BEZT_KEYTYPE_*` numbers.
    pub fn from_blender(v: i64) -> KeyframeType {
        match v {
            1 => KeyframeType::Extreme,
            2 => KeyframeType::Breakdown,
            3 => KeyframeType::Jitter,
            4 => KeyframeType::MovingHold,
            5 => KeyframeType::Generated,
            _ => KeyframeType::Keyframe,
        }
    }
}

/// One keyframe of a layer.
#[derive(Clone, Debug, PartialEq)]
pub struct GpFrame {
    pub frame: i32,
    pub drawing: Drawing,
    pub key_type: KeyframeType,
    pub select: bool,
}

/// Layer blend modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerBlend {
    #[default]
    Regular,
    HardLight,
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl LayerBlend {
    pub const ALL: [LayerBlend; 6] = [LayerBlend::Regular, LayerBlend::HardLight, LayerBlend::Add, LayerBlend::Subtract, LayerBlend::Multiply, LayerBlend::Divide];
    pub fn id(self) -> &'static str {
        match self {
            LayerBlend::Regular => "REGULAR",
            LayerBlend::HardLight => "HARDLIGHT",
            LayerBlend::Add => "ADD",
            LayerBlend::Subtract => "SUBTRACT",
            LayerBlend::Multiply => "MULTIPLY",
            LayerBlend::Divide => "DIVIDE",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            LayerBlend::Regular => "Regular",
            LayerBlend::HardLight => "Hard Light",
            LayerBlend::Add => "Add",
            LayerBlend::Subtract => "Subtract",
            LayerBlend::Multiply => "Multiply",
            LayerBlend::Divide => "Divide",
        }
    }
    pub fn from_id(s: &str) -> Option<LayerBlend> {
        LayerBlend::ALL.into_iter().find(|k| k.id().eq_ignore_ascii_case(s) || k.label() == s)
    }
}

/// A Grease Pencil layer.
#[derive(Clone, Debug, PartialEq)]
pub struct GpLayer {
    pub name: String,
    pub opacity: f32,
    pub blend: LayerBlend,
    pub hide: bool,
    pub lock: bool,
    /// Lights affect the layer's strokes (Use Lights).
    pub use_lights: bool,
    pub tint_color: [f32; 3],
    pub tint_factor: f32,
    /// Added to every point's radius (Blender's "Stroke Thickness" adjustment).
    pub radius_offset: f32,
    /// Layer transform (relative to the object).
    pub transform: Transform,
    /// Parent object (by name) and bone.
    pub parent: Option<String>,
    pub parent_bone: String,
    /// Index into [`GreasePencil::groups`].
    pub group: Option<usize>,
    /// Keyframes, sorted by frame number.
    pub frames: Vec<GpFrame>,
    pub use_onion_skinning: bool,
    pub pass_index: u32,
    /// Layers this layer is masked by (by name); `use_masks` switches it.
    pub masks: Vec<String>,
    pub use_masks: bool,
}

impl GpLayer {
    pub fn new(name: &str) -> GpLayer {
        GpLayer {
            name: name.into(),
            opacity: 1.0,
            blend: LayerBlend::Regular,
            hide: false,
            lock: false,
            use_lights: false,
            tint_color: [1.0; 3],
            tint_factor: 0.0,
            radius_offset: 0.0,
            transform: Transform::default(),
            parent: None,
            parent_bone: String::new(),
            group: None,
            frames: Vec::new(),
            use_onion_skinning: true,
            pass_index: 0,
            masks: Vec::new(),
            use_masks: false,
        }
    }
    /// Index of the keyframe shown at `frame` (the last key at or before it).
    pub fn frame_index_at(&self, frame: i32) -> Option<usize> {
        let i = self.frames.partition_point(|f| f.frame <= frame);
        i.checked_sub(1)
    }
    pub fn drawing_at(&self, frame: i32) -> Option<&Drawing> {
        self.frame_index_at(frame).map(|i| &self.frames[i].drawing)
    }
    /// The key exactly on `frame`.
    pub fn key_at(&self, frame: i32) -> Option<usize> {
        self.frames.iter().position(|f| f.frame == frame)
    }
    /// The drawing to draw into at `frame`: the key on that frame, created
    /// when missing (a copy of the held drawing with `additive`, else blank).
    pub fn ensure_key(&mut self, frame: i32, additive: bool) -> usize {
        if let Some(i) = self.key_at(frame) {
            return i;
        }
        let drawing = if additive { self.drawing_at(frame).cloned().unwrap_or_default() } else { Drawing::default() };
        let at = self.frames.partition_point(|f| f.frame < frame);
        self.frames.insert(at, GpFrame { frame, drawing, key_type: KeyframeType::Keyframe, select: false });
        at
    }
    pub fn sort_frames(&mut self) {
        self.frames.sort_by_key(|f| f.frame);
        self.frames.dedup_by_key(|f| f.frame);
    }
    pub fn local_matrix(&self) -> Mat4 {
        self.transform.matrix()
    }
}

/// A layer group (folder of layers).
#[derive(Clone, Debug, PartialEq)]
pub struct LayerGroup {
    pub name: String,
    pub parent: Option<usize>,
    pub hide: bool,
    pub lock: bool,
    pub expanded: bool,
    /// Blender's color tag (0 none, 1..8).
    pub color_tag: u8,
}

/// Stroke line types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineMode {
    #[default]
    Line,
    Dots,
    Squares,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StrokeStyle {
    #[default]
    Solid,
    Texture,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillStyle {
    #[default]
    Solid,
    Gradient,
    Texture,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GradientType {
    #[default]
    Linear,
    Radial,
}

/// A material's Grease Pencil settings (Blender's `MaterialGPencilStyle`).
#[derive(Clone, Debug, PartialEq)]
pub struct GpMaterial {
    pub name: String,
    pub show_stroke: bool,
    pub mode: LineMode,
    pub stroke_style: StrokeStyle,
    /// Linear RGBA.
    pub color: [f32; 4],
    pub stroke_image: Option<usize>,
    pub show_fill: bool,
    pub fill_style: FillStyle,
    pub fill_color: [f32; 4],
    pub gradient_type: GradientType,
    /// The gradient's second colour.
    pub mix_color: [f32; 4],
    /// Gradient position / texture mix factor.
    pub mix_factor: f32,
    pub fill_image: Option<usize>,
    pub texture_scale: [f32; 2],
    pub texture_offset: [f32; 2],
    pub texture_angle: f32,
    /// Cut holes in strokes below (stroke and fill separately).
    pub use_stroke_holdout: bool,
    pub use_fill_holdout: bool,
    pub hide: bool,
    pub lock: bool,
    pub pass_index: u32,
}

impl GpMaterial {
    pub fn new(name: &str, color: [f32; 4]) -> GpMaterial {
        GpMaterial {
            name: name.into(),
            show_stroke: true,
            mode: LineMode::Line,
            stroke_style: StrokeStyle::Solid,
            color,
            stroke_image: None,
            show_fill: false,
            fill_style: FillStyle::Solid,
            fill_color: [0.5, 0.5, 0.5, 1.0],
            gradient_type: GradientType::Linear,
            mix_color: [1.0; 4],
            mix_factor: 0.5,
            fill_image: None,
            texture_scale: [1.0, 1.0],
            texture_offset: [0.0, 0.0],
            texture_angle: 0.0,
            use_stroke_holdout: false,
            use_fill_holdout: false,
            hide: false,
            lock: false,
            pass_index: 0,
        }
    }
    /// A fill-only material.
    pub fn fill(name: &str, color: [f32; 4]) -> GpMaterial {
        GpMaterial { show_stroke: false, show_fill: true, fill_color: color, ..GpMaterial::new(name, [0.0, 0.0, 0.0, 1.0]) }
    }
}

/// Onion skinning modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnionMode {
    /// Keyframes before and after.
    #[default]
    Keyframes,
    /// Frames (in scene frames) before and after.
    Frames,
    /// Selected keyframes only.
    Selected,
}

/// Onion skinning settings (on the data-block, like Blender's).
#[derive(Clone, Debug, PartialEq)]
pub struct OnionSkin {
    pub mode: OnionMode,
    pub opacity: f32,
    pub before: u32,
    pub after: u32,
    pub color_before: [f32; 3],
    pub color_after: [f32; 3],
    pub use_colors: bool,
    pub use_fade: bool,
    /// The viewport overlay toggle.
    pub enabled: bool,
}

impl Default for OnionSkin {
    fn default() -> Self {
        OnionSkin { mode: OnionMode::Keyframes, opacity: 0.5, before: 1, after: 1, color_before: [0.145, 0.419, 0.137], color_after: [0.125, 0.082, 0.529], use_colors: true, use_fade: true, enabled: false }
    }
}

/// How strokes of an object sort against each other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DepthOrder {
    /// Painter's order: layers then strokes, as drawn.
    #[default]
    Layers2D,
    /// Depth tested by 3D location.
    Location3D,
}

/// The Grease Pencil data-block.
#[derive(Clone, Debug, PartialEq)]
pub struct GreasePencil {
    /// Bottom to top.
    pub layers: Vec<GpLayer>,
    pub groups: Vec<LayerGroup>,
    pub active_layer: usize,
    pub materials: Vec<GpMaterial>,
    pub active_material: usize,
    pub onion: OnionSkin,
    pub vertex_groups: Vec<String>,
    pub active_vertex_group: usize,
    pub depth_order: DepthOrder,
}

impl Default for GreasePencil {
    fn default() -> Self {
        GreasePencil {
            layers: Vec::new(),
            groups: Vec::new(),
            active_layer: 0,
            materials: Vec::new(),
            active_material: 0,
            onion: OnionSkin::default(),
            vertex_groups: Vec::new(),
            active_vertex_group: 0,
            depth_order: DepthOrder::Layers2D,
        }
    }
}

impl GreasePencil {
    /// Blender's Add > Grease Pencil > Blank: one layer and the black and
    /// fill materials.
    pub fn blank() -> GreasePencil {
        let mut g = GreasePencil::default();
        g.layers.push(GpLayer::new("Layer"));
        g.materials.push(GpMaterial::new("Black", [0.0, 0.0, 0.0, 1.0]));
        g
    }
    pub fn add_layer(&mut self, name: &str) -> usize {
        let n = crate::scene::unique_name(name, |n| self.layers.iter().any(|l| l.name == n));
        self.layers.push(GpLayer::new(&n));
        self.active_layer = self.layers.len() - 1;
        self.active_layer
    }
    pub fn find_layer(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name == name)
    }
    pub fn layer_mut(&mut self) -> Option<&mut GpLayer> {
        let i = self.active_layer;
        self.layers.get_mut(i)
    }
    pub fn add_material(&mut self, m: GpMaterial) -> usize {
        if let Some(i) = self.materials.iter().position(|x| x.name == m.name) {
            return i;
        }
        self.materials.push(m);
        self.materials.len() - 1
    }
    /// Remove a layer; groups and the active index are kept consistent.
    pub fn remove_layer(&mut self, i: usize) -> Option<GpLayer> {
        if i >= self.layers.len() {
            return None;
        }
        let l = self.layers.remove(i);
        if self.active_layer >= self.layers.len() {
            self.active_layer = self.layers.len().saturating_sub(1);
        }
        Some(l)
    }
    /// Whether a layer and every group above it are visible.
    pub fn layer_visible(&self, i: usize) -> bool {
        let Some(l) = self.layers.get(i) else { return false };
        let mut g = l.group;
        let mut guard = 0;
        while let Some(gi) = g {
            let Some(grp) = self.groups.get(gi) else { break };
            if grp.hide {
                return false;
            }
            g = grp.parent;
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        !l.hide
    }
    pub fn layer_locked(&self, i: usize) -> bool {
        let Some(l) = self.layers.get(i) else { return true };
        l.lock || l.group.and_then(|g| self.groups.get(g)).is_some_and(|g| g.lock)
    }
    /// Every keyframe number of every layer, sorted and unique.
    pub fn key_frames(&self) -> Vec<i32> {
        let mut v: Vec<i32> = self.layers.iter().flat_map(|l| l.frames.iter().map(|f| f.frame)).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
    pub fn point_count(&self) -> usize {
        self.layers.iter().flat_map(|l| &l.frames).map(|f| f.drawing.point_count()).sum()
    }
    /// The drawing each layer shows at `frame`.
    pub fn drawings_at(&self, frame: i32) -> Vec<(usize, &Drawing)> {
        self.layers.iter().enumerate().filter_map(|(i, l)| l.drawing_at(frame).map(|d| (i, d))).collect()
    }
    pub fn vertex_group_index(&self, name: &str) -> Option<u32> {
        self.vertex_groups.iter().position(|g| g == name).map(|i| i as u32)
    }
    /// Bounds of what is shown at `frame` (layer space; layer transforms applied).
    pub fn bounds_at(&self, frame: i32) -> crate::math::Aabb {
        let mut b = crate::math::Aabb::EMPTY;
        for (li, d) in self.drawings_at(frame) {
            let m = self.layers[li].local_matrix();
            for s in &d.strokes {
                for p in &s.points {
                    b.grow(m.transform_point(p.position));
                }
            }
        }
        b
    }
}

/// The world matrix of layer `li` of a Grease Pencil object `oi` (object
/// matrix, the layer's parent if any, then the layer transform).
pub fn layer_world_matrix(scene: &crate::scene::Scene, oi: usize, gp: &GreasePencil, li: usize) -> Mat4 {
    let ob = scene.world_matrix(oi);
    let Some(l) = gp.layers.get(li) else { return ob };
    let base = match l.parent.as_deref().and_then(|p| scene.find_object(p)) {
        Some(pi) => {
            let pm = scene.world_matrix(pi);
            let bone = (!l.parent_bone.is_empty()).then(|| bone_matrix(scene, pi, &l.parent_bone)).flatten();
            match bone {
                Some(b) => pm * b,
                None => pm,
            }
        }
        None => ob,
    };
    base * l.local_matrix()
}

fn bone_matrix(scene: &crate::scene::Scene, oi: usize, bone: &str) -> Option<Mat4> {
    let arm = scene.armature_of(oi)?;
    let bi = arm.find(bone)?;
    let mats = crate::armature::simple_pose(arm, &scene.objects[oi].pose);
    mats.get(bi).copied()
}

// ---------------------------------------------------------------- samples

/// Linear RGBA from an sRGB hex colour.
pub fn hex(c: u32) -> [f32; 4] {
    let ch = |s: u32| crate::render::srgb_decode(((c >> s) & 0xff) as f32 / 255.0);
    [ch(16), ch(8), ch(0), 1.0]
}

/// Add > Grease Pencil > Stroke: the "Lines" layer with one curvy stroke and
/// the default materials, drawn on the front (XZ) plane.
pub fn sample_stroke() -> GreasePencil {
    let mut g = GreasePencil::default();
    g.layers.push(GpLayer::new("Lines"));
    g.materials.push(GpMaterial::new("Black", [0.0, 0.0, 0.0, 1.0]));
    g.materials.push(GpMaterial::new("Red", hex(0xd02020)));
    g.materials.push(GpMaterial::fill("Grey", hex(0x8a8a8a)));
    let n = 48;
    let pts: Vec<GpPoint> = (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1) as f32;
            let x = -1.2 + 2.4 * t;
            let z = (t * std::f32::consts::TAU * 1.2).sin() * 0.35 + 0.2 * t;
            let mut p = GpPoint::new(Vec3::new(x, 0.0, z), 0.02 * (0.35 + (t * std::f32::consts::PI).sin()));
            p.opacity = 1.0;
            p
        })
        .collect();
    g.layers[0].frames.push(GpFrame { frame: 1, drawing: Drawing { strokes: vec![GpStroke::new(pts, 0)] }, key_type: KeyframeType::Keyframe, select: false });
    g
}

fn ellipse(c: Vec3, rx: f32, rz: f32, n: usize, a0: f32, a1: f32) -> Vec<Vec3> {
    (0..n)
        .map(|i| {
            let t = a0 + (a1 - a0) * i as f32 / (n.max(2) - 1) as f32;
            c + Vec3::new(t.cos() * rx, 0.0, t.sin() * rz)
        })
        .collect()
}

/// Add > Grease Pencil > Cat Head: Polyamour's own sample character, a
/// friendly cat face with ears, eyes, nose, whiskers and a fill, on the
/// "Lines" and "Fills" layers.
pub fn sample_cat() -> GreasePencil {
    use std::f32::consts::{PI, TAU};
    let mut g = GreasePencil::default();
    g.layers.push(GpLayer::new("Fills"));
    g.layers.push(GpLayer::new("Lines"));
    g.active_layer = 1;
    let ink = g.add_material(GpMaterial::new("Ink", hex(0x1b1b1f)));
    let fur = g.add_material(GpMaterial::fill("Fur", hex(0xf0a050)));
    let pink = g.add_material(GpMaterial::fill("Pink", hex(0xf2a0b0)));
    let eye = g.add_material(GpMaterial::fill("Eyes", hex(0x40a060)));
    let r = 0.012;
    let mut fills = Vec::new();
    let mut lines = Vec::new();
    // Head: a wide rounded jaw, the right ear, the brow, the left ear.
    let ear = |x: f32| [Vec3::new(x * 0.88, 0.0, 0.38), Vec3::new(x * 0.78, 0.0, 1.05), Vec3::new(x * 0.35, 0.0, 0.72)];
    let mut head = ellipse(Vec3::ZERO, 1.0, 0.8, 22, PI + 0.45, TAU - 0.45);
    head.extend(ear(1.0));
    head.extend(ellipse(Vec3::new(0.0, 0.0, -0.06), 0.62, 0.8, 8, 1.1, PI - 1.1));
    head.extend(ear(-1.0).iter().rev());
    let mut fill = GpStroke::from_positions(&head, r, fur);
    fill.cyclic = true;
    fills.push(fill);
    let mut ol = GpStroke::from_positions(&head, r * 1.6, ink);
    ol.cyclic = true;
    lines.push(ol);
    // Inner ears.
    for x in [-1.0f32, 1.0] {
        let tri = [Vec3::new(x * 0.8, 0.0, 0.5), Vec3::new(x * 0.76, 0.0, 0.9), Vec3::new(x * 0.48, 0.0, 0.68)];
        let mut s = GpStroke::from_positions(&tri, r, pink);
        s.cyclic = true;
        fills.push(s);
    }
    // Eyes: green almonds with ink pupils.
    for x in [-0.38f32, 0.38] {
        let c = Vec3::new(x, 0.0, 0.12);
        let mut e = GpStroke::from_positions(&ellipse(c, 0.17, 0.12, 20, 0.0, TAU * 0.95), r, eye);
        e.cyclic = true;
        fills.push(e);
        let mut o = GpStroke::from_positions(&ellipse(c, 0.17, 0.12, 20, 0.0, TAU * 0.95), r, ink);
        o.cyclic = true;
        lines.push(o);
        lines.push(GpStroke::from_positions(&[c + Vec3::new(0.0, 0.0, 0.08), c - Vec3::new(0.0, 0.0, 0.08)], r * 2.2, ink));
    }
    // Nose and mouth.
    let mut nose = GpStroke::from_positions(&[Vec3::new(-0.09, 0.0, -0.12), Vec3::new(0.09, 0.0, -0.12), Vec3::new(0.0, 0.0, -0.22)], r, pink);
    nose.cyclic = true;
    fills.push(nose);
    let mouth: Vec<Vec3> = ellipse(Vec3::new(-0.1, 0.0, -0.28), 0.1, 0.08, 8, PI, TAU).into_iter().chain(ellipse(Vec3::new(0.1, 0.0, -0.28), 0.1, 0.08, 8, PI, TAU)).collect();
    lines.push(GpStroke::from_positions(&mouth, r, ink));
    // Whiskers.
    for side in [-1.0f32, 1.0] {
        for k in 0..3 {
            let y = -0.16 - k as f32 * 0.07;
            let a = Vec3::new(side * 0.3, 0.0, y);
            let b = Vec3::new(side * 1.15, 0.0, y + (k as f32 - 1.0) * 0.12);
            let mut s = GpStroke::from_positions(&[a, a.lerp(b, 0.5) + Vec3::new(0.0, 0.0, 0.03), b], r * 0.8, ink);
            s.points[2].radius *= 0.3;
            lines.push(s);
        }
    }
    g.layers[0].frames.push(GpFrame { frame: 1, drawing: Drawing { strokes: fills }, key_type: KeyframeType::Keyframe, select: false });
    g.layers[1].frames.push(GpFrame { frame: 1, drawing: Drawing { strokes: lines }, key_type: KeyframeType::Keyframe, select: false });
    g
}
