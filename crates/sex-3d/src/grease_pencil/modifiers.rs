// SPDX-License-Identifier: GPL-3.0-only
//! Grease Pencil modifiers (Blender 4.3's GPv3 set) and evaluation: which
//! drawing each layer shows at a frame (Time Offset), then each modifier in
//! stack order on the strokes it influences (layer, material and vertex
//! group filters).
use super::geom::{self, SmoothWhat};
use super::lineart::{self, LineArtSettings};
use super::{GpPoint, GpStroke, GreasePencil};
use crate::math::{Mat4, Quat, Vec3};
use crate::scene::{ObjectData, Scene};

/// Which strokes a modifier touches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Influence {
    /// Layer name; empty: every layer.
    pub layer: String,
    pub invert_layer: bool,
    /// Material name; empty: every material.
    pub material: String,
    pub invert_material: bool,
    /// Vertex group name scaling the effect per point; empty: full effect.
    pub vertex_group: String,
    pub invert_vertex_group: bool,
    /// Layer pass index filter (0: off).
    pub layer_pass: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuildMode {
    #[default]
    Sequential,
    Concurrent,
    Additive,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuildTransition {
    #[default]
    Grow,
    Shrink,
    Fade,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EnvelopeMode {
    #[default]
    Deform,
    Segments,
    Fills,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SimplifyMode {
    /// Remove every other point `step` times.
    Fixed { step: u32 },
    /// Ramer-Douglas-Peucker within `factor`.
    Adaptive { factor: f32 },
    /// Resample every `length`.
    Sample { length: f32 },
    /// Merge points closer than `distance`.
    Merge { distance: f32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorMode {
    Stroke,
    Fill,
    #[default]
    Both,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeMode {
    #[default]
    Normal,
    Reverse,
    Fixed,
    PingPong,
    Chain,
}

/// One Dash modifier segment (lengths in points).
#[derive(Clone, Debug, PartialEq)]
pub struct DashSegment {
    pub name: String,
    pub dash: u32,
    pub gap: u32,
    pub radius: f32,
    pub opacity: f32,
    pub material: Option<usize>,
    pub use_cyclic: bool,
}

impl Default for DashSegment {
    fn default() -> Self {
        DashSegment { name: "Segment".into(), dash: 2, gap: 1, radius: 1.0, opacity: 1.0, material: None, use_cyclic: false }
    }
}

/// The modifier types and their settings.
#[derive(Clone, Debug, PartialEq)]
pub enum GpModKind {
    Array { count: u32, constant: [f32; 3], relative: [f32; 3], use_constant: bool, use_relative: bool, random_offset: [f32; 3], random_rotation: [f32; 3], random_scale: [f32; 3], seed: u32, material: Option<usize> },
    Build { mode: BuildMode, transition: BuildTransition, start_delay: f32, length: f32, use_restrict: bool, frame_start: f32, frame_end: f32 },
    Envelope { mode: EnvelopeMode, spread: u32, thickness: f32, strength: f32, skip: u32 },
    Length { start: f32, end: f32, relative: bool },
    LineArt(LineArtSettings),
    Mirror { axes: [bool; 3], object: String },
    Multiply { duplicates: u32, distance: f32, offset: f32, use_fade: bool, fade_center: f32, thickness_fade: f32, opacity_fade: f32 },
    Outline { thickness: f32, keep_shape: bool, subdivision: u32, sample_length: f32, material: Option<usize> },
    Simplify(SimplifyMode),
    Smooth { factor: f32, steps: u32, position: bool, thickness: bool, strength: bool },
    Subdivide { level: u32, simple: bool },
    Thickness { factor: f32, thickness: f32, normalize: bool },
    Noise { factor: f32, factor_thickness: f32, factor_strength: f32, noise_scale: f32, noise_offset: f32, seed: u32, step: u32, use_random: bool },
    Offset { location: [f32; 3], rotation: [f32; 3], scale: [f32; 3], random_offset: [f32; 3], random_rotation: [f32; 3], random_scale: [f32; 3], seed: u32 },
    Opacity { factor: f32, mode: ColorMode, normalize: bool },
    Tint { color: [f32; 3], factor: f32, gradient: bool, object: String, radius: f32, mode: ColorMode },
    Hook { object: String, strength: f32, falloff_radius: f32 },
    Lattice { object: String, strength: f32 },
    Armature { object: String, use_vertex_groups: bool, use_bone_envelopes: bool },
    TimeOffset { mode: TimeMode, offset: i32, scale: f32, use_custom_range: bool, frame_start: i32, frame_end: i32 },
    Dash { segments: Vec<DashSegment>, offset: i32 },
    TextureMapping { uv_offset: f32, uv_scale: f32, rotation: f32 },
}

/// A modifier in an object's Grease Pencil stack.
#[derive(Clone, Debug, PartialEq)]
pub struct GpModifier {
    pub name: String,
    pub show_viewport: bool,
    pub show_render: bool,
    pub influence: Influence,
    pub kind: GpModKind,
}

/// Every type's Python identifier (the `GREASE_PENCIL_*` names lowercased
/// without the prefix) and UI label.
pub const TYPES: [(&str, &str); 22] = [
    ("ARRAY", "Array"),
    ("BUILD", "Build"),
    ("ENVELOPE", "Envelope"),
    ("LENGTH", "Length"),
    ("LINEART", "Line Art"),
    ("MIRROR", "Mirror"),
    ("MULTIPLY", "Multiple Strokes"),
    ("OUTLINE", "Outline"),
    ("SIMPLIFY", "Simplify"),
    ("SMOOTH", "Smooth"),
    ("SUBDIV", "Subdivide"),
    ("THICKNESS", "Thickness"),
    ("NOISE", "Noise"),
    ("OFFSET", "Offset"),
    ("OPACITY", "Opacity"),
    ("TINT", "Tint"),
    ("HOOK", "Hook"),
    ("LATTICE", "Lattice"),
    ("ARMATURE", "Armature"),
    ("TIME", "Time Offset"),
    ("DASH", "Dot Dash"),
    ("TEXTURE", "Texture Mapping"),
];

impl GpModifier {
    /// A new modifier of type `id` (from [`TYPES`]) with Blender's defaults.
    pub fn new(id: &str) -> Option<GpModifier> {
        let kind = match id.to_ascii_uppercase().trim_start_matches("GREASE_PENCIL_") {
            "ARRAY" => GpModKind::Array { count: 2, constant: [0.0; 3], relative: [1.0, 0.0, 0.0], use_constant: false, use_relative: true, random_offset: [0.0; 3], random_rotation: [0.0; 3], random_scale: [0.0; 3], seed: 1, material: None },
            "BUILD" => GpModKind::Build { mode: BuildMode::Sequential, transition: BuildTransition::Grow, start_delay: 0.0, length: 100.0, use_restrict: false, frame_start: 1.0, frame_end: 125.0 },
            "ENVELOPE" => GpModKind::Envelope { mode: EnvelopeMode::Deform, spread: 10, thickness: 1.0, strength: 1.0, skip: 0 },
            "LENGTH" => GpModKind::Length { start: 0.1, end: 0.1, relative: true },
            "LINEART" => GpModKind::LineArt(LineArtSettings::default()),
            "MIRROR" => GpModKind::Mirror { axes: [true, false, false], object: String::new() },
            "MULTIPLY" => GpModKind::Multiply { duplicates: 2, distance: 0.1, offset: 0.0, use_fade: false, fade_center: 0.5, thickness_fade: 0.5, opacity_fade: 0.5 },
            "OUTLINE" => GpModKind::Outline { thickness: 0.02, keep_shape: true, subdivision: 3, sample_length: 0.0, material: None },
            "SIMPLIFY" => GpModKind::Simplify(SimplifyMode::Fixed { step: 1 }),
            "SMOOTH" => GpModKind::Smooth { factor: 1.0, steps: 1, position: true, thickness: false, strength: false },
            "SUBDIV" | "SUBDIVIDE" => GpModKind::Subdivide { level: 1, simple: false },
            "THICKNESS" => GpModKind::Thickness { factor: 1.0, thickness: 0.02, normalize: false },
            "NOISE" => GpModKind::Noise { factor: 0.5, factor_thickness: 0.0, factor_strength: 0.0, noise_scale: 0.0, noise_offset: 0.0, seed: 1, step: 4, use_random: true },
            "OFFSET" => GpModKind::Offset { location: [0.0; 3], rotation: [0.0; 3], scale: [0.0; 3], random_offset: [0.0; 3], random_rotation: [0.0; 3], random_scale: [0.0; 3], seed: 1 },
            "OPACITY" => GpModKind::Opacity { factor: 1.0, mode: ColorMode::Stroke, normalize: false },
            "TINT" => GpModKind::Tint { color: [1.0; 3], factor: 0.5, gradient: false, object: String::new(), radius: 1.0, mode: ColorMode::Both },
            "HOOK" => GpModKind::Hook { object: String::new(), strength: 1.0, falloff_radius: 0.0 },
            "LATTICE" => GpModKind::Lattice { object: String::new(), strength: 1.0 },
            "ARMATURE" => GpModKind::Armature { object: String::new(), use_vertex_groups: true, use_bone_envelopes: false },
            "TIME" | "TIME_OFFSET" => GpModKind::TimeOffset { mode: TimeMode::Normal, offset: 1, scale: 1.0, use_custom_range: false, frame_start: 1, frame_end: 250 },
            "DASH" => GpModKind::Dash { segments: vec![DashSegment::default()], offset: 0 },
            "TEXTURE" => GpModKind::TextureMapping { uv_offset: 0.0, uv_scale: 1.0, rotation: 0.0 },
            _ => return None,
        };
        let label = TYPES.iter().find(|t| id.to_ascii_uppercase().ends_with(t.0)).map_or("Modifier", |t| t.1);
        Some(GpModifier { name: label.into(), show_viewport: true, show_render: true, influence: Influence::default(), kind })
    }
    pub fn type_id(&self) -> &'static str {
        match &self.kind {
            GpModKind::Array { .. } => "ARRAY",
            GpModKind::Build { .. } => "BUILD",
            GpModKind::Envelope { .. } => "ENVELOPE",
            GpModKind::Length { .. } => "LENGTH",
            GpModKind::LineArt(_) => "LINEART",
            GpModKind::Mirror { .. } => "MIRROR",
            GpModKind::Multiply { .. } => "MULTIPLY",
            GpModKind::Outline { .. } => "OUTLINE",
            GpModKind::Simplify(_) => "SIMPLIFY",
            GpModKind::Smooth { .. } => "SMOOTH",
            GpModKind::Subdivide { .. } => "SUBDIV",
            GpModKind::Thickness { .. } => "THICKNESS",
            GpModKind::Noise { .. } => "NOISE",
            GpModKind::Offset { .. } => "OFFSET",
            GpModKind::Opacity { .. } => "OPACITY",
            GpModKind::Tint { .. } => "TINT",
            GpModKind::Hook { .. } => "HOOK",
            GpModKind::Lattice { .. } => "LATTICE",
            GpModKind::Armature { .. } => "ARMATURE",
            GpModKind::TimeOffset { .. } => "TIME",
            GpModKind::Dash { .. } => "DASH",
            GpModKind::TextureMapping { .. } => "TEXTURE",
        }
    }
    pub fn label(&self) -> &'static str {
        let id = self.type_id();
        TYPES.iter().find(|t| t.0 == id).map_or("Modifier", |t| t.1)
    }
}

/// What evaluation needs from outside the data-block.
pub struct EvalContext<'a> {
    pub scene: Option<&'a Scene>,
    /// The Grease Pencil object being evaluated.
    pub owner: usize,
    pub frame: i32,
    /// Render (true) or viewport (false) visibility of modifiers.
    pub render: bool,
}

/// A layer's evaluated strokes (layer space) at a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalLayer {
    pub layer: usize,
    /// The keyframe shown (for Build), or `None` when generated only.
    pub key_frame: Option<i32>,
    pub strokes: Vec<GpStroke>,
}

/// Evaluate `gp` at `cx.frame` through `mods`.
pub fn evaluate(gp: &GreasePencil, mods: &[GpModifier], cx: &EvalContext<'_>) -> Vec<EvalLayer> {
    let active: Vec<&GpModifier> = mods.iter().filter(|m| if cx.render { m.show_render } else { m.show_viewport }).collect();
    let mut layers: Vec<EvalLayer> = Vec::with_capacity(gp.layers.len());
    for (li, l) in gp.layers.iter().enumerate() {
        // Time Offset picks another frame for the layers it influences.
        let mut f = cx.frame;
        for m in &active {
            if let GpModKind::TimeOffset { .. } = m.kind {
                if layer_matches(&m.influence, gp, li) {
                    f = time_offset(&m.kind, f, l);
                }
            }
        }
        let idx = l.frame_index_at(f);
        layers.push(EvalLayer { layer: li, key_frame: idx.map(|i| l.frames[i].frame), strokes: idx.map(|i| l.frames[i].drawing.strokes.clone()).unwrap_or_default() });
    }
    for m in active {
        if matches!(m.kind, GpModKind::TimeOffset { .. }) {
            continue;
        }
        if let GpModKind::LineArt(s) = &m.kind {
            line_art(gp, s, cx, &mut layers);
            continue;
        }
        for el in &mut layers {
            if !layer_matches(&m.influence, gp, el.layer) {
                continue;
            }
            let prev_count = prev_key_strokes(gp, el);
            let strokes = std::mem::take(&mut el.strokes);
            let (hit, keep): (Vec<(usize, GpStroke)>, Vec<(usize, GpStroke)>) = strokes.into_iter().enumerate().partition(|(_, s)| material_matches(&m.influence, gp, s.material));
            let hit_strokes: Vec<GpStroke> = hit.iter().map(|(_, s)| s.clone()).collect();
            let weights = group_weights(&m.influence, gp);
            let out = apply(m, gp, hit_strokes, el, prev_count, weights.as_ref(), cx);
            // Untouched strokes keep their place below the modified ones.
            el.strokes = keep.into_iter().map(|(_, s)| s).chain(out).collect();
        }
    }
    layers
}

fn layer_matches(inf: &Influence, gp: &GreasePencil, li: usize) -> bool {
    let l = &gp.layers[li];
    let mut ok = inf.layer.is_empty() || (l.name == inf.layer) != inf.invert_layer;
    if inf.layer_pass > 0 {
        ok &= l.pass_index == inf.layer_pass;
    }
    ok
}

fn material_matches(inf: &Influence, gp: &GreasePencil, m: usize) -> bool {
    inf.material.is_empty() || (gp.materials.get(m).is_some_and(|x| x.name == inf.material) != inf.invert_material)
}

/// Per-point factor from the vertex group filter: (group index, invert).
fn group_weights(inf: &Influence, gp: &GreasePencil) -> Option<(u32, bool)> {
    if inf.vertex_group.is_empty() {
        return None;
    }
    Some((gp.vertex_group_index(&inf.vertex_group).unwrap_or(u32::MAX), inf.invert_vertex_group))
}

fn point_factor(p: &GpPoint, w: Option<&(u32, bool)>) -> f32 {
    match w {
        None => 1.0,
        Some(&(g, inv)) => {
            let v = p.weight(g);
            if inv { 1.0 - v } else { v }
        }
    }
}

/// How many strokes the previous keyframe of the layer had (Build Additive).
fn prev_key_strokes(gp: &GreasePencil, el: &EvalLayer) -> usize {
    let Some(k) = el.key_frame else { return 0 };
    let l = &gp.layers[el.layer];
    match l.frames.iter().position(|f| f.frame == k) {
        Some(i) if i > 0 => l.frames[i - 1].drawing.strokes.len(),
        _ => 0,
    }
}

fn time_offset(kind: &GpModKind, f: i32, l: &super::GpLayer) -> i32 {
    let GpModKind::TimeOffset { mode, offset, scale, use_custom_range, frame_start, frame_end } = kind else { return f };
    let (first, last) = match (l.frames.first(), l.frames.last()) {
        (Some(a), Some(b)) => (a.frame, b.frame),
        _ => return f,
    };
    let (s, e) = if *use_custom_range { (*frame_start, (*frame_end).max(*frame_start)) } else { (first, last) };
    let len = (e - s + 1).max(1);
    let scaled = |x: i32| ((x as f32) * scale).floor() as i32;
    match mode {
        TimeMode::Normal | TimeMode::Chain => {
            let x = scaled(f - s) + offset;
            if *use_custom_range { s + x.rem_euclid(len) } else { s + x }
        }
        TimeMode::Reverse => {
            let x = scaled(f - s) + offset;
            e - x.rem_euclid(len)
        }
        TimeMode::Fixed => *offset,
        TimeMode::PingPong => {
            let x = (scaled(f - s) + offset).rem_euclid((2 * len - 2).max(1));
            if x < len { s + x } else { s + (2 * len - 2 - x) }
        }
    }
}

/// A deterministic random number in -1..1.
pub fn hash01(mut x: u32) -> f32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// Smooth 1D value noise in -1..1.
fn noise1(t: f32, seed: u32) -> f32 {
    let i = t.floor();
    let f = t - i;
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash01((i as i32 as u32).wrapping_mul(2_654_435_761).wrapping_add(seed));
    let b = hash01(((i as i32 + 1) as u32).wrapping_mul(2_654_435_761).wrapping_add(seed));
    a + (b - a) * u
}

/// The plane a stroke lies in (its normal), for offsets "beside" it.
fn stroke_normal(s: &GpStroke) -> Vec3 {
    let pts = s.positions();
    let n = geom::polygon_normal(&pts);
    if n.length() > 1e-6 * (1.0 + geom::polyline_length(&pts, false).powi(2)) {
        n.normalize()
    } else {
        // Straight strokes: assume the front (X-Z) drawing plane, unless the stroke runs along Y.
        let t = pts.last().map_or(Vec3::X, |&l| l - pts[0]);
        if t.cross(Vec3::Y).length() > 1e-6 { Vec3::Y } else { Vec3::X }
    }
}

/// Unit "side" vector at each point (perpendicular to the tangent in the stroke's plane).
fn sides(s: &GpStroke) -> Vec<Vec3> {
    let n = s.points.len();
    let nrm = stroke_normal(s);
    (0..n)
        .map(|i| {
            let a = s.points[i.saturating_sub(1)].position;
            let b = s.points[(i + 1).min(n - 1)].position;
            let t = b - a;
            let side = t.cross(nrm);
            if side.length() > 1e-9 { side.normalize() } else { nrm.any_orthonormal() }
        })
        .collect()
}

fn euler_quat(r: [f32; 3]) -> Quat {
    Quat::from_euler_xyz(r[0], r[1], r[2])
}

#[allow(clippy::too_many_arguments)]
fn apply(m: &GpModifier, gp: &GreasePencil, mut strokes: Vec<GpStroke>, el: &EvalLayer, prev_count: usize, w: Option<&(u32, bool)>, cx: &EvalContext<'_>) -> Vec<GpStroke> {
    match &m.kind {
        GpModKind::Array { count, constant, relative, use_constant, use_relative, random_offset, random_rotation, random_scale, seed, material } => {
            let mut b = crate::math::Aabb::EMPTY;
            for s in &strokes {
                for p in &s.points {
                    b.grow(p.position);
                }
            }
            let size = if b.is_empty() { Vec3::ZERO } else { b.size() };
            let mut out = Vec::new();
            for k in 0..(*count).max(1) {
                let kf = k as f32;
                let mut off = Vec3::ZERO;
                if *use_constant {
                    off += Vec3::from_array(*constant) * kf;
                }
                if *use_relative {
                    off += Vec3::from_array(*relative).mul_elem(size) * kf;
                }
                let r = |c: u32| hash01(seed.wrapping_mul(7919).wrapping_add(k * 31 + c));
                off += Vec3::new(random_offset[0] * r(1), random_offset[1] * r(2), random_offset[2] * r(3));
                let rot = euler_quat([random_rotation[0] * r(4), random_rotation[1] * r(5), random_rotation[2] * r(6)]);
                let sc = Vec3::ONE + Vec3::new(random_scale[0] * r(7), random_scale[1] * r(8), random_scale[2] * r(9));
                for s in &strokes {
                    let mut c = s.clone();
                    for p in &mut c.points {
                        let f = point_factor(p, w);
                        let moved = rot.rotate(p.position.mul_elem(sc)) + off;
                        p.position = p.position.lerp(moved, f);
                    }
                    if k > 0 {
                        if let Some(mi) = material {
                            c.material = *mi;
                        }
                    }
                    out.push(c);
                }
            }
            out
        }
        GpModKind::Build { mode, transition, start_delay, length, use_restrict, frame_start, frame_end } => {
            let f = cx.frame as f32;
            if *use_restrict && (f < *frame_start || f > *frame_end) {
                return strokes;
            }
            let start = el.key_frame.unwrap_or(cx.frame) as f32 + start_delay;
            let t = if *length > 0.0 { ((f - start) / length).clamp(0.0, 1.0) } else { 1.0 };
            let t = if *transition == BuildTransition::Shrink { 1.0 - t } else { t };
            let first = if *mode == BuildMode::Additive { prev_count.min(strokes.len()) } else { 0 };
            let total: usize = strokes[first..].iter().map(|s| s.points.len()).sum();
            let mut budget = (total as f32 * t).round() as usize;
            let mut out = Vec::new();
            for (i, mut s) in strokes.into_iter().enumerate() {
                if i < first {
                    out.push(s);
                    continue;
                }
                match (mode, transition) {
                    (_, BuildTransition::Fade) => {
                        let k = if *mode == BuildMode::Concurrent { t } else { (budget as f32 / s.points.len().max(1) as f32).clamp(0.0, 1.0) };
                        if *mode != BuildMode::Concurrent {
                            budget = budget.saturating_sub(s.points.len());
                        }
                        s.points.iter_mut().for_each(|p| p.opacity *= k);
                        s.fill_opacity *= k;
                        out.push(s);
                    }
                    (BuildMode::Concurrent, _) => {
                        let n = ((s.points.len() as f32) * t).round() as usize;
                        if n > 0 {
                            s.points.truncate(n);
                            s.cyclic &= t >= 1.0;
                            out.push(s);
                        }
                    }
                    _ => {
                        let n = budget.min(s.points.len());
                        budget -= n;
                        if n > 0 {
                            s.cyclic &= n == s.points.len();
                            s.points.truncate(n);
                            out.push(s);
                        }
                    }
                }
            }
            out
        }
        GpModKind::Envelope { mode, spread, thickness, strength, skip } => {
            let sp = (*spread).max(1) as usize;
            let mut out = Vec::new();
            for s in strokes {
                let n = s.points.len();
                if n < 3 {
                    out.push(s);
                    continue;
                }
                match mode {
                    EnvelopeMode::Deform => {
                        let mut c = s.clone();
                        for i in 0..n {
                            let (a, b) = (i.saturating_sub(sp), (i + sp).min(n - 1));
                            let mut avg = Vec3::ZERO;
                            for p in &s.points[a..=b] {
                                avg += p.position;
                            }
                            avg = avg / (b - a + 1) as f32;
                            let half = s.points[a].position.distance(s.points[b].position) * 0.5;
                            let f = point_factor(&s.points[i], w);
                            c.points[i].position = s.points[i].position.lerp(avg, f);
                            c.points[i].radius = s.points[i].radius + (half * thickness * 0.5 - s.points[i].radius).max(0.0) * f;
                            c.points[i].opacity = s.points[i].opacity * (1.0 - f + f * strength);
                        }
                        out.push(c);
                    }
                    EnvelopeMode::Segments | EnvelopeMode::Fills => {
                        let step = *skip as usize + 1;
                        let mut i = 0;
                        while i + sp < n {
                            let (a, b) = (&s.points[i], &s.points[i + sp]);
                            let mut seg = GpStroke { points: vec![a.clone(), b.clone()], cyclic: false, ..s.clone() };
                            seg.points.iter_mut().for_each(|p| {
                                p.radius *= thickness;
                                p.opacity *= strength;
                            });
                            if *mode == EnvelopeMode::Fills && i + sp + 1 < n {
                                seg.points.push(s.points[i + sp + 1].clone());
                                seg.points.push(s.points[i + 1].clone());
                                seg.cyclic = true;
                            }
                            out.push(seg);
                            i += step;
                        }
                        out.push(s);
                    }
                }
            }
            out
        }
        GpModKind::Length { start, end, relative } => {
            for s in &mut strokes {
                if s.points.len() < 2 || s.cyclic {
                    continue;
                }
                let len = s.length();
                let (ds, de) = if *relative { (start * len, end * len) } else { (*start, *end) };
                change_length(s, ds, de);
            }
            strokes
        }
        GpModKind::Mirror { axes, object } => {
            let (to_mirror, from_mirror) = match (cx.scene, object.is_empty()) {
                (Some(sc), false) => match sc.find_object(object) {
                    Some(o) => {
                        let ob = sc.world_matrix(cx.owner);
                        let mo = sc.world_matrix(o);
                        (mo.inverse() * ob, ob.inverse() * mo)
                    }
                    None => (Mat4::IDENTITY, Mat4::IDENTITY),
                },
                _ => (Mat4::IDENTITY, Mat4::IDENTITY),
            };
            let mut out = strokes.clone();
            for (ax, &on) in axes.iter().enumerate() {
                if !on {
                    continue;
                }
                let copies: Vec<GpStroke> = out
                    .iter()
                    .map(|s| {
                        let mut c = s.clone();
                        for p in &mut c.points {
                            let mut q = to_mirror.transform_point(p.position);
                            match ax {
                                0 => q.x = -q.x,
                                1 => q.y = -q.y,
                                _ => q.z = -q.z,
                            }
                            p.position = from_mirror.transform_point(q);
                        }
                        c
                    })
                    .collect();
                out.extend(copies);
            }
            out
        }
        GpModKind::Multiply { duplicates, distance, offset, use_fade, fade_center, thickness_fade, opacity_fade } => {
            let mut out = Vec::new();
            for s in strokes {
                let side = sides(&s);
                let n = *duplicates as i32;
                for k in -n..=n {
                    if k == 0 {
                        continue;
                    }
                    let kf = k as f32 / n.max(1) as f32;
                    let d = distance * (kf + offset);
                    let mut c = s.clone();
                    for (p, sd) in c.points.iter_mut().zip(&side) {
                        p.position += *sd * d * point_factor(p, w);
                    }
                    if *use_fade {
                        let fade = 1.0 - ((kf.abs() - fade_center).abs()).min(1.0);
                        c.points.iter_mut().for_each(|p| {
                            p.radius *= 1.0 - thickness_fade * (1.0 - fade);
                            p.opacity *= 1.0 - opacity_fade * (1.0 - fade);
                        });
                    }
                    out.push(c);
                }
                out.push(s);
            }
            out
        }
        GpModKind::Outline { thickness, keep_shape, subdivision, sample_length, material } => strokes
            .into_iter()
            .map(|s| {
                let mut o = outline(&s, if *keep_shape { 1.0 } else { 0.0 }, *subdivision);
                if *sample_length > 0.0 {
                    o.points = geom::resample_length(&o.points, *sample_length);
                }
                o.points.iter_mut().for_each(|p| p.radius = *thickness);
                if let Some(mi) = material {
                    o.material = *mi;
                }
                o
            })
            .collect(),
        GpModKind::Simplify(mode) => {
            for s in &mut strokes {
                s.points = match *mode {
                    SimplifyMode::Fixed { step } => {
                        let mut p = s.points.clone();
                        for _ in 0..step {
                            if p.len() <= 2 {
                                break;
                            }
                            let last = p.len() - 1;
                            p = p.into_iter().enumerate().filter(|(i, _)| i % 2 == 0 || *i == last).map(|(_, q)| q).collect();
                        }
                        p
                    }
                    SimplifyMode::Adaptive { factor } => geom::simplify(&s.points, factor),
                    SimplifyMode::Sample { length } => geom::resample_length(&s.points, length),
                    SimplifyMode::Merge { distance } => geom::merge_by_distance(&s.points, distance),
                };
            }
            strokes
        }
        GpModKind::Smooth { factor, steps, position, thickness, strength } => {
            let what = SmoothWhat { position: *position, radius: *thickness, opacity: *strength };
            for s in &mut strokes {
                let f: Vec<f32> = s.points.iter().map(|p| point_factor(p, w)).collect();
                let cyc = s.cyclic;
                geom::smooth(&mut s.points, cyc, *factor, *steps, what, &|i| f[i]);
            }
            strokes
        }
        GpModKind::Subdivide { level, simple } => {
            for s in &mut strokes {
                s.points = if *simple { geom::subdivide(&s.points, s.cyclic, (1 << (*level).min(8)) - 1, &|_, _| true) } else { geom::subdivide_smooth(&s.points, s.cyclic, (*level).min(8)) };
            }
            strokes
        }
        GpModKind::Thickness { factor, thickness, normalize } => {
            for s in &mut strokes {
                for p in &mut s.points {
                    let f = point_factor(p, w);
                    let to = if *normalize { *thickness } else { p.radius * factor };
                    p.radius += (to - p.radius) * f;
                }
            }
            strokes
        }
        GpModKind::Noise { factor, factor_thickness, factor_strength, noise_scale, noise_offset, seed, step, use_random } => {
            let time_seed = if *use_random { (cx.frame.max(0) as u32) / (*step).max(1) } else { 0 };
            for (si, s) in strokes.iter_mut().enumerate() {
                let side = sides(s);
                let sd = seed.wrapping_add(si as u32 * 101).wrapping_add(time_seed * 7919);
                let lens = geom::arc_lengths(&s.points);
                let scale = if *noise_scale > 0.0 { 1.0 / noise_scale } else { 4.0 };
                for (i, p) in s.points.iter_mut().enumerate() {
                    let t = lens[i] * scale + noise_offset;
                    let f = point_factor(p, w);
                    p.position += side[i] * (noise1(t, sd) * factor * 0.1 * f);
                    p.radius = (p.radius * (1.0 + noise1(t, sd ^ 0x55) * factor_thickness * f)).max(0.0);
                    p.opacity = (p.opacity + noise1(t, sd ^ 0xaa) * factor_strength * f).clamp(0.0, 1.0);
                }
            }
            strokes
        }
        GpModKind::Offset { location, rotation, scale, random_offset, random_rotation, random_scale, seed } => {
            for (si, s) in strokes.iter_mut().enumerate() {
                let r = |c: u32| hash01(seed.wrapping_mul(7919).wrapping_add(si as u32 * 31 + c));
                let loc = Vec3::from_array(*location) + Vec3::new(random_offset[0] * r(1), random_offset[1] * r(2), random_offset[2] * r(3));
                let rot = euler_quat([rotation[0] + random_rotation[0] * r(4), rotation[1] + random_rotation[1] * r(5), rotation[2] + random_rotation[2] * r(6)]);
                let sc = Vec3::ONE + Vec3::from_array(*scale) + Vec3::new(random_scale[0] * r(7), random_scale[1] * r(8), random_scale[2] * r(9));
                let m = Mat4::from_trs(loc, rot, sc);
                let rs = super::scale_of(&m);
                for p in &mut s.points {
                    let f = point_factor(p, w);
                    p.position = p.position.lerp(m.transform_point(p.position), f);
                    p.radius *= 1.0 + (rs - 1.0) * f;
                }
            }
            strokes
        }
        GpModKind::Opacity { factor, mode, normalize } => {
            for s in &mut strokes {
                if *mode != ColorMode::Fill {
                    for p in &mut s.points {
                        let f = point_factor(p, w);
                        let to = if *normalize { *factor } else { p.opacity * factor };
                        p.opacity = (p.opacity + (to - p.opacity) * f).clamp(0.0, 1.0);
                    }
                }
                if *mode != ColorMode::Stroke {
                    s.fill_opacity = if *normalize { *factor } else { s.fill_opacity * factor }.clamp(0.0, 1.0);
                }
            }
            strokes
        }
        GpModKind::Tint { color, factor, gradient, object, radius, mode } => {
            let centre = match (gradient, cx.scene) {
                (true, Some(sc)) => sc.find_object(object).map(|o| sc.world_matrix(cx.owner).inverse().transform_point(sc.world_matrix(o).transform_point(Vec3::ZERO))),
                _ => None,
            };
            let mix = |c: &mut [f32; 4], f: f32| {
                for i in 0..3 {
                    c[i] += (color[i] - c[i]) * f;
                }
                c[3] += (1.0 - c[3]) * f;
            };
            for s in &mut strokes {
                let mut fill_f = 0.0f32;
                for p in &mut s.points {
                    let mut f = factor * point_factor(p, w);
                    if *gradient {
                        let d = centre.map_or(0.0, |c| p.position.distance(c));
                        f *= (1.0 - d / radius.max(1e-6)).clamp(0.0, 1.0);
                    }
                    fill_f = fill_f.max(f);
                    if *mode != ColorMode::Fill {
                        mix(&mut p.vertex_color, f.clamp(0.0, 1.0));
                    }
                }
                if *mode != ColorMode::Stroke {
                    mix(&mut s.fill_color, if *gradient { fill_f } else { *factor }.clamp(0.0, 1.0));
                }
            }
            strokes
        }
        GpModKind::Hook { object, strength, falloff_radius } => {
            use crate::modifiers::{Falloff, Modifier};
            let Some(sc) = cx.scene else { return strokes };
            let Some(oi) = sc.find_object(object) else { return strokes };
            let inv = sc.world_matrix(oi).inverse() * sc.world_matrix(cx.owner);
            let md = Modifier::Hook {
                object: object.clone(),
                subtarget: String::new(),
                falloff_type: if *falloff_radius > 0.0 { Falloff::Smooth } else { Falloff::None },
                falloff_radius: *falloff_radius,
                center: [0.0; 3],
                matrix_inverse: inv,
                strength: *strength,
                use_falloff_uniform: false,
                vertex_indices: Vec::new(),
                vertex_group: None,
                invert_vertex_group: false,
            };
            mesh_deform(gp, el, strokes, &md, w, cx)
        }
        GpModKind::Lattice { object, strength } => {
            let md = crate::modifiers::Modifier::Lattice { object: object.clone(), strength: *strength, vertex_group: None, invert_vertex_group: false };
            mesh_deform(gp, el, strokes, &md, w, cx)
        }
        GpModKind::Armature { object, use_vertex_groups, use_bone_envelopes } => {
            let md = crate::modifiers::Modifier::Armature {
                object: object.clone(),
                use_vertex_groups: *use_vertex_groups,
                use_bone_envelopes: *use_bone_envelopes,
                use_deform_preserve_volume: false,
                use_multi_modifier: false,
                vertex_group: None,
                invert_vertex_group: false,
            };
            mesh_deform(gp, el, strokes, &md, None, cx)
        }
        GpModKind::Dash { segments, offset } => {
            let segs: Vec<&DashSegment> = segments.iter().filter(|s| s.dash > 0).collect();
            if segs.is_empty() {
                return strokes;
            }
            let period: i32 = segs.iter().map(|s| (s.dash + s.gap) as i32).sum();
            let mut out = Vec::new();
            for s in strokes {
                let n = s.points.len();
                let mut i = 0usize;
                // The offset shifts the pattern along the stroke.
                let mut phase = offset.rem_euclid(period.max(1)) as usize;
                let mut k = 0usize;
                while phase > 0 {
                    let seg = segs[k % segs.len()];
                    let l = (seg.dash + seg.gap) as usize;
                    if phase >= l {
                        phase -= l;
                        k += 1;
                    } else {
                        break;
                    }
                }
                let mut skip = phase;
                while i < n {
                    let seg = segs[k % segs.len()];
                    let dash = (seg.dash as usize).saturating_sub(skip.min(seg.dash as usize));
                    let gap_skip = skip.saturating_sub(seg.dash as usize);
                    skip = 0;
                    let end = (i + dash + 1).min(n);
                    if dash > 0 && end > i + 1 {
                        let mut piece = s.sub(i..end);
                        piece.cyclic = seg.use_cyclic;
                        piece.points.iter_mut().for_each(|p| {
                            p.radius *= seg.radius;
                            p.opacity *= seg.opacity;
                        });
                        if let Some(m) = seg.material {
                            piece.material = m;
                        }
                        out.push(piece);
                    }
                    i = end.max(i + 1) + (seg.gap as usize).saturating_sub(gap_skip);
                    k += 1;
                }
            }
            out
        }
        GpModKind::TextureMapping { uv_offset, uv_scale, rotation } => {
            for s in &mut strokes {
                s.uv_offset += uv_offset;
                s.uv_scale *= uv_scale;
                s.uv_rotation += rotation;
            }
            strokes
        }
        GpModKind::TimeOffset { .. } | GpModKind::LineArt(_) => strokes,
    }
}

/// Extend (positive) or trim (negative) the ends along their tangents.
fn change_length(s: &mut GpStroke, start: f32, end: f32) {
    let trim = |pts: &mut Vec<GpPoint>, amount: f32| {
        // Trim from the front.
        let lens = geom::arc_lengths(pts);
        let total = *lens.last().unwrap_or(&0.0);
        if amount >= total {
            pts.truncate(1);
            return;
        }
        let p = geom::sample_at(pts, &lens, amount);
        let i = lens.partition_point(|&l| l <= amount);
        pts.drain(..i);
        pts.insert(0, p);
    };
    if start < 0.0 {
        trim(&mut s.points, -start);
    } else if start > 0.0 {
        let (a, b) = (s.points[0].clone(), &s.points[1]);
        let t = (a.position - b.position).normalize();
        if t.length() > 0.0 {
            let mut p = a.clone();
            p.position = a.position + t * start;
            s.points.insert(0, p);
        }
    }
    if end != 0.0 && s.points.len() >= 2 {
        s.points.reverse();
        if end < 0.0 {
            trim(&mut s.points, -end);
        } else {
            let (a, b) = (s.points[0].clone(), &s.points[1]);
            let t = (a.position - b.position).normalize();
            if t.length() > 0.0 {
                let mut p = a.clone();
                p.position = a.position + t * end;
                s.points.insert(0, p);
            }
        }
        s.points.reverse();
    }
}

/// The outline of a stroke's ribbon as a cyclic stroke (in the stroke's plane):
/// one side, a round cap, the other side back, the other cap.
pub fn outline(s: &GpStroke, keep: f32, cap_segments: u32) -> GpStroke {
    let n = s.points.len();
    let side = sides(s);
    let nrm = stroke_normal(s);
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for i in 0..n {
        let p = &s.points[i];
        let r = p.radius * (0.5 + 0.5 * keep.clamp(0.0, 1.0)).max(0.5);
        let mut a = p.clone();
        a.position = p.position + side[i] * r;
        let mut b = p.clone();
        b.position = p.position - side[i] * r;
        left.push(a);
        right.push(b);
    }
    let cap = |c: &GpPoint, from: Vec3, out: &mut Vec<GpPoint>| {
        // Half circle from `from` around c (rotating about the stroke normal).
        let segs = cap_segments.max(1) * 2;
        for k in 1..segs {
            let ang = std::f32::consts::PI * k as f32 / segs as f32;
            let q = Quat::from_axis_angle(nrm, -ang);
            let mut p = c.clone();
            p.position = c.position + q.rotate(from - c.position);
            out.push(p);
        }
    };
    let mut pts = left.clone();
    if !s.cyclic && n > 0 {
        cap(&s.points[n - 1], left[n - 1].position, &mut pts);
    }
    pts.extend(right.iter().rev().cloned());
    if !s.cyclic && n > 0 {
        cap(&s.points[0], right[0].position, &mut pts);
    }
    let mut o = GpStroke { points: pts, ..s.clone() };
    o.cyclic = true;
    o
}

/// Deform points with a mesh deform modifier (Hook, Lattice, Armature) by
/// treating them as a point cloud mesh in object space.
fn mesh_deform(gp: &GreasePencil, el: &EvalLayer, mut strokes: Vec<GpStroke>, md: &crate::modifiers::Modifier, w: Option<&(u32, bool)>, cx: &EvalContext<'_>) -> Vec<GpStroke> {
    let Some(sc) = cx.scene else { return strokes };
    let lm = gp.layers[el.layer].local_matrix();
    let lmi = lm.inverse();
    let mut mesh = crate::mesh::Mesh::new();
    mesh.vertex_groups = gp.vertex_groups.clone();
    for s in &strokes {
        for p in &s.points {
            mesh.positions.push(lm.transform_point(p.position));
            mesh.weights.push(p.weights.clone());
        }
    }
    if mesh.positions.is_empty() {
        return strokes;
    }
    let out = sc.apply_modifiers(cx.owner, &mesh, std::slice::from_ref(md));
    if out.positions.len() != mesh.positions.len() {
        return strokes;
    }
    let mut k = 0;
    for s in &mut strokes {
        for p in &mut s.points {
            let f = point_factor(p, w);
            p.position = p.position.lerp(lmi.transform_point(out.positions[k]), f);
            k += 1;
        }
    }
    strokes
}

/// Line Art: edges of the scene's meshes seen from the active camera, into
/// the target layer.
fn line_art(gp: &GreasePencil, s: &LineArtSettings, cx: &EvalContext<'_>, layers: &mut [EvalLayer]) {
    let Some(sc) = cx.scene else { return };
    let li = gp.find_layer(&s.target_layer).unwrap_or(0);
    let Some(el) = layers.iter_mut().find(|e| e.layer == li) else { return };
    let to_layer = super::layer_world_matrix(sc, cx.owner, gp, li).inverse();
    let lines = lineart::compute(sc, cx.owner, s);
    let mat = s.material.min(gp.materials.len().saturating_sub(1));
    for line in lines {
        let pts: Vec<Vec3> = line.iter().map(|&p| to_layer.transform_point(p)).collect();
        let mut st = GpStroke::from_positions(&pts, s.radius, mat);
        st.points.iter_mut().for_each(|p| p.opacity = s.opacity);
        el.strokes.push(st);
    }
    let _ = ObjectData::Empty;
}
