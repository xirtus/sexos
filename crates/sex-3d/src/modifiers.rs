// SPDX-License-Identifier: GPL-3.0-only
//! The non-destructive modifier stack.
//!
//! A [`Modifier`] is a pure function from mesh to mesh; an object's stack is
//! evaluated top to bottom by [`apply_stack`] whenever the viewport or the
//! renderer needs the evaluated mesh. Modifiers that share an algorithm with
//! an edit-mode operation (bevel, doubled-face cleanup) call into [`crate::ops`].
use crate::math::{Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use crate::ops;
use std::collections::HashMap;

pub(crate) mod bvh;
mod decimate;
mod deform;
mod generate;
pub mod multires;
mod remesh;
mod rig;
#[cfg(test)]
mod tests;

pub use crate::ops::BooleanOp;
pub use multires::{MultiresData, MultiresLevel, MultiresSubdiv};
pub use rig::{Falloff, ProximityMode, WeightMixMode, WeightMixSet};

/// Subdivision levels above this are clamped: each level quadruples the face
/// count, so level 6 already turns a cube into 24k quads.
pub const MAX_SUBDIVISION_LEVELS: u32 = 6;

/// Decimate's three algorithms (Blender's Collapse / Un-Subdivide / Planar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimateMode {
    /// Quadric edge collapse down to `ratio` of the triangles.
    Collapse,
    /// Undo `iterations` levels of grid subdivision on quad meshes.
    Unsubdivide,
    /// Dissolve faces that meet at less than `angle` (limited dissolve).
    Planar,
}

/// Remesh modes. Blender's "Sharp" mode is not implemented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemeshMode {
    /// Axis-aligned cubes on a `2^octree_depth` grid.
    Blocks,
    /// A smooth surface on a `2^octree_depth` grid.
    Smooth,
    /// A smooth surface on a grid of `voxel_size` cubes (the default).
    Voxel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimpleDeformMode {
    Twist,
    Bend,
    Taper,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastType {
    Sphere,
    Cylinder,
    Cuboid,
}

/// How Shrinkwrap finds a vertex's place on the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShrinkwrapMethod {
    NearestSurfacePoint,
    /// Cast a ray along the vertex normal (or the chosen axes).
    Project,
    NearestVertex,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Modifier {
    /// Mirror across the object's local axes; `merge` welds seam vertices
    /// within `merge_distance`.
    Mirror { x: bool, y: bool, z: bool, merge: bool, merge_distance: f32 },
    /// Catmull-Clark subdivision `levels` times (`simple` keeps the shape).
    Subdivision { levels: u32, simple: bool },
    /// `count` copies offset by `relative` (times bounds size) plus `constant`.
    Array { count: u32, relative: [f32; 3], constant: [f32; 3], merge: bool },
    /// Give surfaces `thickness` (offset along normals, -1..1 picks the side).
    Solidify { thickness: f32, offset: f32 },
    /// Bevel every edge sharper than `angle` radians by `width`, `segments` cuts.
    Bevel { width: f32, segments: u32, angle: f32 },
    Triangulate,
    /// Merge by distance.
    Weld { distance: f32 },
    /// Move vertices along normals by `strength` (uniform).
    Displace { strength: f32, vertex_group: Option<String>, invert_vertex_group: bool },
    /// Laplacian smoothing.
    Smooth { factor: f32, iterations: u32, vertex_group: Option<String>, invert_vertex_group: bool },
    /// Reduce the face count. `ratio` drives Collapse, `iterations`
    /// Un-Subdivide and `angle` (radians) Planar; `triangulate` keeps
    /// Collapse's output all triangles instead of restoring untouched faces.
    Decimate { mode: DecimateMode, ratio: f32, iterations: u32, angle: f32, triangulate: bool },
    /// Rebuild the surface on a regular grid: Voxel mode samples a signed
    /// distance field every `voxel_size`; Blocks and Smooth use a
    /// `2^octree_depth` grid over the bounds divided by `scale`.
    Remesh { mode: RemeshMode, voxel_size: f32, octree_depth: u32, scale: f32, smooth_shade: bool },
    /// Sweep every edge around a local `axis` (0 X, 1 Y, 2 Z) by `angle`
    /// radians in `steps`, rising `screw_offset` per turn, `iterations` turns.
    Screw { axis: u8, angle: f32, steps: u32, screw_offset: f32, iterations: u32, merge: bool, merge_distance: f32, flip: bool, smooth_shade: bool },
    /// Turn every edge into a square tube `thickness` wide; `offset` (-1..1)
    /// picks the side like Solidify.
    Wireframe { thickness: f32, offset: f32, even: bool, relative: bool, boundary: bool, replace: bool },
    /// Split edges whose faces meet at more than `angle` radians. (Mesh has
    /// no sharp-edge flags yet, so Blender's "Sharp Edges" source is absent.)
    EdgeSplit { angle: f32, use_angle: bool },
    /// Reveal faces one by one from frame `start` over `length` frames.
    Build { start: f32, length: f32, reverse: bool, random: bool, seed: u32 },
    /// Boolean with another object's evaluated mesh, found by name.
    Boolean { object: String, operation: BooleanOp },
    /// Twist/Bend (`factor` is an angle in radians) or Taper/Stretch along
    /// local `axis`, between `limits` (fractions of the bounds along it).
    SimpleDeform { mode: SimpleDeformMode, axis: u8, factor: f32, limits: [f32; 2], lock: [bool; 3], vertex_group: Option<String>, invert_vertex_group: bool },
    /// Pull vertices towards a sphere, cylinder or box around the origin.
    /// `radius` > 0 limits the effect; `size` 0 means the average distance.
    Cast { kind: CastType, factor: f32, radius: f32, size: f32, axes: [bool; 3], size_from_radius: bool, vertex_group: Option<String>, invert_vertex_group: bool },
    /// A travelling Gaussian ripple; time is the scene frame.
    Wave {
        x: bool,
        y: bool,
        cyclic: bool,
        normals: bool,
        falloff: f32,
        height: f32,
        width: f32,
        narrowness: f32,
        speed: f32,
        time_offset: f32,
        lifetime: f32,
        damping: f32,
        start: [f32; 2],
        vertex_group: Option<String>,
        invert_vertex_group: bool,
    },
    /// Move vertices onto another object's surface (found by name).
    Shrinkwrap { target: String, method: ShrinkwrapMethod, offset: f32, axes: [bool; 3], positive: bool, negative: bool, vertex_group: Option<String>, invert_vertex_group: bool },
    /// Implicit Laplacian smoothing with cotangent weights.
    LaplacianSmooth { iterations: u32, factor: f32, border: f32, axes: [bool; 3], preserve_volume: bool, normalized: bool, vertex_group: Option<String>, invert_vertex_group: bool },
    /// Smooth away deformation artefacts while keeping the detail of the
    /// rest shape (the stack's input).
    CorrectiveSmooth { factor: f32, iterations: u32, scale: f32, length_weighted: bool, only_smooth: bool, vertex_group: Option<String>, invert_vertex_group: bool },
    /// Smooth shading with face-area-weighted vertex normals. Meshes carry no
    /// custom split normals, so this marks every face smooth (the renderers
    /// already weight vertex normals by face area); `keep_sharp` splits edges
    /// sharper than 30 degrees so they stay crisp.
    WeightedNormal { keep_sharp: bool },
    /// Deform by an armature object's pose (linear blend skinning, or dual
    /// quaternions with `use_deform_preserve_volume`). Weights come from
    /// vertex groups named after bones and/or bone envelopes;
    /// `vertex_group` scales the whole effect. With `use_multi_modifier`
    /// the armature deforms the previous modifier's input and the result is
    /// mixed with that modifier's output by `vertex_group`.
    Armature {
        object: String,
        use_vertex_groups: bool,
        use_bone_envelopes: bool,
        use_deform_preserve_volume: bool,
        use_multi_modifier: bool,
        vertex_group: Option<String>,
        invert_vertex_group: bool,
    },
    /// Move vertices with an object (or one of its bones): `matrix_inverse`
    /// is the hook's parent inverse ("Reset"/"Recenter"), `center` the
    /// falloff centre. Vertices come from `vertex_indices` or `vertex_group`.
    Hook {
        object: String,
        subtarget: String,
        falloff_type: Falloff,
        falloff_radius: f32,
        center: [f32; 3],
        matrix_inverse: crate::math::Mat4,
        strength: f32,
        use_falloff_uniform: bool,
        vertex_indices: Vec<u32>,
        vertex_group: Option<String>,
        invert_vertex_group: bool,
    },
    /// Deform by a lattice object.
    Lattice { object: String, strength: f32, vertex_group: Option<String>, invert_vertex_group: bool },
    /// Warp space from `object_from` to `object_to` within `falloff_radius`.
    Warp { object_from: String, object_to: String, strength: f32, falloff_type: Falloff, falloff_radius: f32, use_volume_preserve: bool, vertex_group: Option<String>, invert_vertex_group: bool },
    /// Edit the weights of `vertex_group` (map, add, remove).
    VertexWeightEdit {
        vertex_group: String,
        default_weight: f32,
        use_add: bool,
        add_threshold: f32,
        use_remove: bool,
        remove_threshold: f32,
        falloff_type: Falloff,
        invert_falloff: bool,
        normalize: bool,
        mask_constant: f32,
    },
    /// Mix group B into group A.
    VertexWeightMix {
        vertex_group_a: String,
        vertex_group_b: String,
        default_weight_a: f32,
        default_weight_b: f32,
        mix_mode: WeightMixMode,
        mix_set: WeightMixSet,
        invert_vertex_group_a: bool,
        invert_vertex_group_b: bool,
        normalize: bool,
        mask_constant: f32,
    },
    /// Set `vertex_group` weights from the distance to `target` (its origin,
    /// or its nearest vertex/edge/face).
    VertexWeightProximity {
        vertex_group: String,
        target: String,
        proximity_mode: ProximityMode,
        use_proximity_geometry: [bool; 3],
        min_dist: f32,
        max_dist: f32,
        falloff_type: Falloff,
        invert_falloff: bool,
        normalize: bool,
        mask_constant: f32,
    },
    /// Geometry Nodes: evaluate node group `group` (by name, in
    /// `Scene::node_groups`) with the mesh as its first geometry input and
    /// `inputs` (interface socket name, value) for the rest; the result is
    /// realized into a mesh. Without a scene the mesh passes through.
    Nodes { group: String, inputs: Vec<(String, crate::nodes::Value)> },
    /// Multiresolution: sculptable subdivision levels with per-level
    /// displacement (see [`multires`]). `levels` is shown in the viewport,
    /// `sculpt_levels` is sculpted, `render_levels` is rendered.
    Multires { levels: u32, sculpt_levels: u32, render_levels: u32, optimal_display: bool, data: MultiresData },
    /// Cloth, Soft Body, Collision, Particle System and Fluid: the
    /// simulation's place in the stack (see [`crate::physics`]).
    Physics(crate::physics::PhysicsModifier),
}

/// What a modifier may need from outside its own mesh.
#[derive(Clone, Copy)]
pub struct EvalContext<'a> {
    /// The scene's current frame (Wave and Build animate with it).
    pub frame: f32,
    /// Another object's evaluated mesh by name, in the space of the object
    /// being modified (Boolean, Shrinkwrap).
    pub object: Option<&'a dyn Fn(&str) -> Option<Mesh>>,
    /// Rest positions: the stack's input (Corrective Smooth).
    pub rest: Option<&'a [Vec3]>,
    /// The scene and the modified object (Armature, Hook, Lattice, Warp and
    /// Vertex Weight Proximity read other objects' transforms and data).
    pub scene: Option<&'a crate::scene::Scene>,
    pub owner: usize,
    /// Modifier recursion depth (objects referring to each other).
    pub depth: u32,
}

impl Default for EvalContext<'_> {
    fn default() -> Self {
        EvalContext { frame: 1.0, object: None, rest: None, scene: None, owner: 0, depth: 0 }
    }
}

impl EvalContext<'_> {
    fn lookup(&self, name: &str) -> Option<Mesh> {
        if name.is_empty() {
            return None;
        }
        self.object.and_then(|f| f(name))
    }
}

impl Modifier {
    /// Every polyops/Blender type [`Modifier::with_defaults`] knows.
    pub const TYPE_NAMES: &'static str = "Mirror, Subdivision, Array, Solidify, Bevel, Triangulate, Weld, Displace, Smooth, Decimate, Remesh, Screw, \
Wireframe, EdgeSplit, Build, Boolean, SimpleDeform, Cast, Wave, Shrinkwrap, LaplacianSmooth, CorrectiveSmooth, WeightedNormal, \
Armature, Hook, Lattice, Warp, VertexWeightEdit, VertexWeightMix, VertexWeightProximity, Nodes, Multires, \
Cloth, Softbody, Collision, ParticleSystem, Fluid";

    /// The name Blender shows on the modifier's panel.
    pub fn name(&self) -> &'static str {
        match self {
            Modifier::Mirror { .. } => "Mirror",
            Modifier::Subdivision { .. } => "Subdivision",
            Modifier::Array { .. } => "Array",
            Modifier::Solidify { .. } => "Solidify",
            Modifier::Bevel { .. } => "Bevel",
            Modifier::Triangulate => "Triangulate",
            Modifier::Weld { .. } => "Weld",
            Modifier::Displace { .. } => "Displace",
            Modifier::Smooth { .. } => "Smooth",
            Modifier::Decimate { .. } => "Decimate",
            Modifier::Remesh { .. } => "Remesh",
            Modifier::Screw { .. } => "Screw",
            Modifier::Wireframe { .. } => "Wireframe",
            Modifier::EdgeSplit { .. } => "Edge Split",
            Modifier::Build { .. } => "Build",
            Modifier::Boolean { .. } => "Boolean",
            Modifier::SimpleDeform { .. } => "Simple Deform",
            Modifier::Cast { .. } => "Cast",
            Modifier::Wave { .. } => "Wave",
            Modifier::Shrinkwrap { .. } => "Shrinkwrap",
            Modifier::LaplacianSmooth { .. } => "Laplacian Smooth",
            Modifier::CorrectiveSmooth { .. } => "Corrective Smooth",
            Modifier::WeightedNormal { .. } => "Weighted Normal",
            Modifier::Armature { .. } => "Armature",
            Modifier::Hook { .. } => "Hook",
            Modifier::Lattice { .. } => "Lattice",
            Modifier::Warp { .. } => "Warp",
            Modifier::VertexWeightEdit { .. } => "Vertex Weight Edit",
            Modifier::VertexWeightMix { .. } => "Vertex Weight Mix",
            Modifier::VertexWeightProximity { .. } => "Vertex Weight Proximity",
            Modifier::Nodes { .. } => "Geometry Nodes",
            Modifier::Multires { .. } => "Multires",
            Modifier::Physics(p) => p.name(),
        }
    }

    /// The polyops type word: [`Modifier::name`] without spaces.
    pub fn type_name(&self) -> String {
        if let Modifier::Nodes { .. } = self {
            return "Nodes".into();
        }
        self.name().replace(' ', "")
    }

    /// A modifier with Blender's defaults, by Blender type identifier
    /// (`SUBSURF`, `SIMPLE_DEFORM`, ...) or name (`Subdivision`, `Simple
    /// Deform`, `SimpleDeform`), in any case.
    pub fn with_defaults(ty: &str) -> Option<Modifier> {
        let key = ty.chars().filter(|c| !matches!(c, ' ' | '_' | '-')).collect::<String>().to_ascii_lowercase();
        Some(match key.as_str() {
            "mirror" => Modifier::Mirror { x: true, y: false, z: false, merge: true, merge_distance: 0.001 },
            "subdivision" | "subsurf" | "subdivisionsurface" => Modifier::Subdivision { levels: 1, simple: false },
            "array" => Modifier::Array { count: 2, relative: [1.0, 0.0, 0.0], constant: [0.0; 3], merge: false },
            "solidify" => Modifier::Solidify { thickness: 0.01, offset: -1.0 },
            "bevel" => Modifier::Bevel { width: 0.1, segments: 1, angle: 30f32.to_radians() },
            "triangulate" => Modifier::Triangulate,
            "weld" => Modifier::Weld { distance: 0.001 },
            "displace" => Modifier::Displace { strength: 1.0, vertex_group: None, invert_vertex_group: false },
            "smooth" => Modifier::Smooth { factor: 0.5, iterations: 1, vertex_group: None, invert_vertex_group: false },
            "decimate" => Modifier::Decimate { mode: DecimateMode::Collapse, ratio: 1.0, iterations: 0, angle: 5f32.to_radians(), triangulate: false },
            "remesh" => Modifier::Remesh { mode: RemeshMode::Voxel, voxel_size: 0.1, octree_depth: 4, scale: 0.9, smooth_shade: false },
            "screw" => Modifier::Screw { axis: 2, angle: std::f32::consts::TAU, steps: 16, screw_offset: 0.0, iterations: 1, merge: false, merge_distance: 0.01, flip: false, smooth_shade: true },
            "wireframe" => Modifier::Wireframe { thickness: 0.02, offset: -1.0, even: true, relative: false, boundary: false, replace: true },
            "edgesplit" => Modifier::EdgeSplit { angle: 30f32.to_radians(), use_angle: true },
            "build" => Modifier::Build { start: 1.0, length: 100.0, reverse: false, random: false, seed: 0 },
            "boolean" => Modifier::Boolean { object: String::new(), operation: BooleanOp::Difference },
            "simpledeform" => Modifier::SimpleDeform { mode: SimpleDeformMode::Twist, axis: 2, factor: 45f32.to_radians(), limits: [0.0, 1.0], lock: [false; 3], vertex_group: None, invert_vertex_group: false },
            "cast" => Modifier::Cast { kind: CastType::Sphere, factor: 0.5, radius: 0.0, size: 0.0, axes: [true; 3], size_from_radius: false, vertex_group: None, invert_vertex_group: false },
            "wave" => Modifier::Wave {
                x: true,
                y: true,
                cyclic: true,
                normals: false,
                falloff: 0.0,
                height: 0.5,
                width: 1.5,
                narrowness: 1.5,
                speed: 0.25,
                time_offset: 0.0,
                lifetime: 0.0,
                damping: 10.0,
                start: [0.0; 2],
                vertex_group: None,
                invert_vertex_group: false,
            },
            "shrinkwrap" => Modifier::Shrinkwrap { target: String::new(), method: ShrinkwrapMethod::NearestSurfacePoint, offset: 0.0, axes: [false; 3], positive: true, negative: false, vertex_group: None, invert_vertex_group: false },
            "laplaciansmooth" | "smoothlaplacian" => Modifier::LaplacianSmooth { iterations: 1, factor: 0.01, border: 0.01, axes: [true; 3], preserve_volume: true, normalized: true, vertex_group: None, invert_vertex_group: false },
            "correctivesmooth" | "smoothcorrective" => Modifier::CorrectiveSmooth { factor: 0.5, iterations: 5, scale: 1.0, length_weighted: false, only_smooth: false, vertex_group: None, invert_vertex_group: false },
            "weightednormal" => Modifier::WeightedNormal { keep_sharp: false },
            "nodes" | "geometrynodes" => Modifier::Nodes { group: String::new(), inputs: Vec::new() },
            "armature" => Modifier::Armature {
                object: String::new(),
                use_vertex_groups: true,
                use_bone_envelopes: false,
                use_deform_preserve_volume: false,
                use_multi_modifier: false,
                vertex_group: None,
                invert_vertex_group: false,
            },
            "hook" => Modifier::Hook {
                object: String::new(),
                subtarget: String::new(),
                falloff_type: Falloff::None,
                falloff_radius: 0.0,
                center: [0.0; 3],
                matrix_inverse: crate::math::Mat4::IDENTITY,
                strength: 1.0,
                use_falloff_uniform: false,
                vertex_indices: Vec::new(),
                vertex_group: None,
                invert_vertex_group: false,
            },
            "lattice" => Modifier::Lattice { object: String::new(), strength: 1.0, vertex_group: None, invert_vertex_group: false },
            "warp" => Modifier::Warp {
                object_from: String::new(),
                object_to: String::new(),
                strength: 1.0,
                falloff_type: Falloff::Smooth,
                falloff_radius: 1.0,
                use_volume_preserve: false,
                vertex_group: None,
                invert_vertex_group: false,
            },
            "vertexweightedit" | "weightvgedit" => Modifier::VertexWeightEdit {
                vertex_group: String::new(),
                default_weight: 0.0,
                use_add: false,
                add_threshold: 0.01,
                use_remove: false,
                remove_threshold: 0.01,
                falloff_type: Falloff::None,
                invert_falloff: false,
                normalize: false,
                mask_constant: 1.0,
            },
            "vertexweightmix" | "weightvgmix" => Modifier::VertexWeightMix {
                vertex_group_a: String::new(),
                vertex_group_b: String::new(),
                default_weight_a: 0.0,
                default_weight_b: 0.0,
                mix_mode: WeightMixMode::Set,
                mix_set: WeightMixSet::And,
                invert_vertex_group_a: false,
                invert_vertex_group_b: false,
                normalize: false,
                mask_constant: 1.0,
            },
            "vertexweightproximity" | "weightvgproximity" => Modifier::VertexWeightProximity {
                vertex_group: String::new(),
                target: String::new(),
                proximity_mode: ProximityMode::Geometry,
                use_proximity_geometry: [false, false, true],
                min_dist: 0.0,
                max_dist: 1.0,
                falloff_type: Falloff::None,
                invert_falloff: false,
                normalize: false,
                mask_constant: 1.0,
            },
            "multires" | "multiresolution" => Modifier::Multires { levels: 0, sculpt_levels: 0, render_levels: 0, optimal_display: true, data: MultiresData::default() },
            _ => return crate::physics::PhysicsModifier::with_defaults(&key).map(Modifier::Physics),
        })
    }

    /// True when the result changes with the frame.
    pub fn is_animated(&self) -> bool {
        matches!(self, Modifier::Wave { .. } | Modifier::Build { .. } | Modifier::Nodes { .. } | Modifier::Physics(_))
    }

    /// The other object this modifier reads, if any (Warp: the "from" object).
    pub fn object_ref(&self) -> Option<&str> {
        match self {
            Modifier::Boolean { object, .. } | Modifier::Armature { object, .. } | Modifier::Hook { object, .. } | Modifier::Lattice { object, .. } => Some(object),
            Modifier::Shrinkwrap { target, .. } | Modifier::VertexWeightProximity { target, .. } => Some(target),
            Modifier::Warp { object_from, .. } => Some(object_from),
            _ => None,
        }
    }

    /// Point a reference to object `old` at `new` (after a rename).
    pub fn rename_object_ref(&mut self, old: &str, new: &str) {
        match self {
            Modifier::Boolean { object: s, .. }
            | Modifier::Shrinkwrap { target: s, .. }
            | Modifier::Armature { object: s, .. }
            | Modifier::Hook { object: s, .. }
            | Modifier::Lattice { object: s, .. }
            | Modifier::VertexWeightProximity { target: s, .. }
                if s == old =>
            {
                *s = new.to_string()
            }
            Modifier::Warp { object_from, object_to, .. } => {
                for s in [object_from, object_to] {
                    if s == old {
                        *s = new.to_string();
                    }
                }
            }
            _ => {}
        }
    }

    /// The common `vertex_group` / `invert_vertex_group` pair, for the
    /// modifiers that have one (every deform modifier, Armature, Hook,
    /// Lattice and Warp).
    pub fn vertex_group_mut(&mut self) -> Option<(&mut Option<String>, &mut bool)> {
        match self {
            Modifier::Displace { vertex_group, invert_vertex_group, .. }
            | Modifier::Smooth { vertex_group, invert_vertex_group, .. }
            | Modifier::SimpleDeform { vertex_group, invert_vertex_group, .. }
            | Modifier::Cast { vertex_group, invert_vertex_group, .. }
            | Modifier::Wave { vertex_group, invert_vertex_group, .. }
            | Modifier::Shrinkwrap { vertex_group, invert_vertex_group, .. }
            | Modifier::LaplacianSmooth { vertex_group, invert_vertex_group, .. }
            | Modifier::CorrectiveSmooth { vertex_group, invert_vertex_group, .. }
            | Modifier::Armature { vertex_group, invert_vertex_group, .. }
            | Modifier::Hook { vertex_group, invert_vertex_group, .. }
            | Modifier::Lattice { vertex_group, invert_vertex_group, .. }
            | Modifier::Warp { vertex_group, invert_vertex_group, .. } => Some((vertex_group, invert_vertex_group)),
            _ => None,
        }
    }

    /// The vertex group masking a deform modifier that has no group logic of
    /// its own: the result is blended with the input by the group weight.
    fn mask_group(&self) -> Option<(&str, bool)> {
        match self {
            Modifier::Displace { vertex_group: Some(g), invert_vertex_group: i, .. }
            | Modifier::Smooth { vertex_group: Some(g), invert_vertex_group: i, .. }
            | Modifier::SimpleDeform { vertex_group: Some(g), invert_vertex_group: i, .. }
            | Modifier::Cast { vertex_group: Some(g), invert_vertex_group: i, .. }
            | Modifier::Wave { vertex_group: Some(g), invert_vertex_group: i, .. }
            | Modifier::Shrinkwrap { vertex_group: Some(g), invert_vertex_group: i, .. }
            | Modifier::LaplacianSmooth { vertex_group: Some(g), invert_vertex_group: i, .. }
            | Modifier::CorrectiveSmooth { vertex_group: Some(g), invert_vertex_group: i, .. }
                if !g.is_empty() =>
            {
                Some((g.as_str(), *i))
            }
            _ => None,
        }
    }

    /// Apply this one modifier to `m` with no scene around it: frame 1, no
    /// other objects (Boolean and Shrinkwrap pass the mesh through).
    pub fn apply(&self, m: &Mesh) -> Mesh {
        self.apply_with(m, &EvalContext::default())
    }

    /// Apply this one modifier to `m`.
    pub fn apply_with(&self, m: &Mesh, cx: &EvalContext<'_>) -> Mesh {
        self.apply_prev(m, cx, None)
    }

    /// Apply with the previous modifier's input at hand (Armature's Multi
    /// Modifier).
    fn apply_prev(&self, m: &Mesh, cx: &EvalContext<'_>, prev: Option<&[Vec3]>) -> Mesh {
        let mut out = self.apply_inner(m, cx, prev);
        if let Some((g, invert)) = self.mask_group() {
            rig::mask_by_group(m, &mut out, g, invert);
        }
        out
    }

    fn apply_inner(&self, m: &Mesh, cx: &EvalContext<'_>, prev: Option<&[Vec3]>) -> Mesh {
        match self {
            &Modifier::Mirror { x, y, z, merge, merge_distance } => mirror(m, [x, y, z], merge.then_some(merge_distance.max(0.0))),
            &Modifier::Subdivision { levels, simple } => {
                if simple {
                    subdivide_simple(m, levels)
                } else {
                    subdivide_catmull_clark(m, levels)
                }
            }
            &Modifier::Array { count, relative, constant, merge } => array(m, count, Vec3::from_array(relative), Vec3::from_array(constant), merge),
            &Modifier::Solidify { thickness, offset } => solidify(m, thickness, offset),
            &Modifier::Bevel { width, segments, angle } => bevel_by_angle(m, width, segments, angle),
            Modifier::Triangulate => triangulate(m),
            &Modifier::Weld { distance } => {
                let mut out = m.clone();
                out.weld(distance);
                out
            }
            &Modifier::Displace { strength, .. } => displace(m, strength),
            &Modifier::Smooth { factor, iterations, .. } => smooth(m, factor, iterations),
            &Modifier::Decimate { mode, ratio, iterations, angle, triangulate } => match mode {
                DecimateMode::Collapse => decimate::collapse(m, ratio, triangulate),
                DecimateMode::Unsubdivide => decimate::unsubdivide(m, iterations),
                DecimateMode::Planar => {
                    let mut out = m.clone();
                    ops::dissolve_coplanar(&mut out, angle.max(0.0));
                    out
                }
            },
            &Modifier::Remesh { mode, voxel_size, octree_depth, scale, smooth_shade } => remesh::remesh(m, mode, voxel_size, octree_depth, scale, smooth_shade),
            &Modifier::Screw { axis, angle, steps, screw_offset, iterations, merge, merge_distance, flip, smooth_shade } => {
                generate::screw(m, &generate::ScrewParams { axis, angle, steps, screw_offset, iterations, merge: merge.then_some(merge_distance), flip, smooth_shade })
            }
            &Modifier::Wireframe { thickness, offset, even, relative, boundary, replace } => generate::wireframe(m, &generate::WireParams { thickness, offset, even, relative, boundary, replace }),
            &Modifier::EdgeSplit { angle, use_angle } => generate::edge_split(m, use_angle.then_some(angle)),
            &Modifier::Build { start, length, reverse, random, seed } => generate::build(m, cx.frame, start, length, reverse, random.then_some(seed)),
            Modifier::Boolean { object, operation } => match cx.lookup(object) {
                Some(other) if !other.faces.is_empty() => ops::boolean(m, &other, *operation),
                _ => m.clone(),
            },
            &Modifier::SimpleDeform { mode, axis, factor, limits, lock, .. } => deform::simple_deform(m, mode, axis, factor, limits, lock),
            &Modifier::Cast { kind, factor, radius, size, axes, size_from_radius, .. } => deform::cast(m, kind, factor, radius, size, axes, size_from_radius),
            &Modifier::Wave { x, y, cyclic, normals, falloff, height, width, narrowness, speed, time_offset, lifetime, damping, start, .. } => {
                deform::wave(m, cx.frame, &deform::WaveParams { x, y, cyclic, normals, falloff, height, width, narrowness, speed, time_offset, lifetime, damping, start })
            }
            Modifier::Shrinkwrap { target, method, offset, axes, positive, negative, .. } => match cx.lookup(target) {
                Some(t) if !t.positions.is_empty() => deform::shrinkwrap(m, &t, *method, *offset, *axes, *positive, *negative),
                _ => m.clone(),
            },
            &Modifier::LaplacianSmooth { iterations, factor, border, axes, preserve_volume, normalized, .. } => deform::laplacian_smooth(m, iterations, factor, border, axes, preserve_volume, normalized),
            &Modifier::CorrectiveSmooth { factor, iterations, scale, length_weighted, only_smooth, .. } => {
                let rest = cx.rest.filter(|r| r.len() == m.positions.len()).unwrap_or(&m.positions);
                deform::corrective_smooth(m, rest, factor, iterations, scale, length_weighted, only_smooth)
            }
            &Modifier::WeightedNormal { keep_sharp } => {
                let mut out = if keep_sharp { generate::edge_split(m, Some(30f32.to_radians())) } else { m.clone() };
                out.faces.iter_mut().for_each(|f| f.smooth = true);
                out
            }
            Modifier::Armature { .. } => rig::armature(self, m, cx, prev),
            Modifier::Hook { .. } => rig::hook(self, m, cx),
            Modifier::Lattice { .. } => rig::lattice(self, m, cx),
            Modifier::Physics(p) => p.apply(self, m, cx),
            Modifier::Warp { .. } => rig::warp(self, m, cx),
            Modifier::VertexWeightEdit { .. } | Modifier::VertexWeightMix { .. } | Modifier::VertexWeightProximity { .. } => rig::weights(self, m, cx),
            Modifier::Nodes { group, inputs } => match cx.scene {
                Some(s) if !group.is_empty() => crate::geometry::run_modifier(m, s, group, inputs, cx.owner, cx.frame, cx.depth),
                _ => m.clone(),
            },
            Modifier::Multires { levels, data, .. } => data.level_mesh(m, (*levels).min(data.total_levels())),
        }
    }
}

/// Run a stack top to bottom with no scene around it (see [`Modifier::apply`]).
pub fn apply_stack(m: &Mesh, stack: &[Modifier]) -> Mesh {
    apply_stack_with(m, stack, &EvalContext::default())
}

/// Run a stack top to bottom. The rest shape for Corrective Smooth is `m`
/// unless `cx` names one.
pub fn apply_stack_with(m: &Mesh, stack: &[Modifier], cx: &EvalContext<'_>) -> Mesh {
    let cx = EvalContext { rest: Some(cx.rest.unwrap_or(&m.positions)), ..*cx };
    let mut out = m.clone();
    let mut prev_input: Option<Vec<Vec3>> = None;
    for md in stack {
        let multi = matches!(md, Modifier::Armature { use_multi_modifier: true, .. });
        let input = out.positions.clone();
        out = md.apply_prev(&out, &cx, if multi { prev_input.as_deref() } else { None });
        // A Multi Modifier armature shares its input with the next one too.
        if !multi {
            prev_input = Some(input);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Subdivision

/// Midpoint ("simple") subdivision: every n-gon becomes n quads through its
/// edge midpoints and centroid; the shape does not change.
pub fn subdivide_simple(m: &Mesh, levels: u32) -> Mesh {
    let mut out = m.clone();
    for _ in 0..levels.min(MAX_SUBDIVISION_LEVELS) {
        out = subdivide_once(&out, true);
    }
    out
}

/// Catmull-Clark subdivision on n-gons. Boundary edges (and non-manifold or
/// loose edges) are treated as creases: boundary vertices follow the cubic
/// B-spline rule along the boundary, and boundary corners (one face) stay put,
/// like Blender's "keep corners" default. UVs are interpolated linearly.
pub fn subdivide_catmull_clark(m: &Mesh, levels: u32) -> Mesh {
    let mut out = m.clone();
    for _ in 0..levels.min(MAX_SUBDIVISION_LEVELS) {
        out = subdivide_once(&out, false);
    }
    out
}

/// One undirected edge of the input with up to two adjacent faces.
struct SubEdge {
    v: [u32; 2],
    faces: [u32; 2],
    count: u32,
}

/// One level of subdivision. New vertex layout: the original vertices (moved),
/// then one point per edge, then one per face.
fn subdivide_once(m: &Mesh, simple: bool) -> Mesh {
    let nv = m.positions.len();
    let pos = &m.positions;

    // Unique edges, and for every face corner the edge leaving it.
    let mut edge_index: HashMap<(u32, u32), u32> = HashMap::with_capacity(m.faces.len() * 2);
    let mut edges: Vec<SubEdge> = Vec::with_capacity(m.faces.len() * 2);
    let mut corner_edge: Vec<u32> = Vec::new();
    let mut face_offset: Vec<usize> = Vec::with_capacity(m.faces.len());
    let mut edge_of = |a: u32, b: u32, edges: &mut Vec<SubEdge>| -> u32 {
        *edge_index.entry(ekey(a, b)).or_insert_with(|| {
            edges.push(SubEdge { v: [a, b], faces: [u32::MAX; 2], count: 0 });
            (edges.len() - 1) as u32
        })
    };
    for (fi, f) in m.faces.iter().enumerate() {
        face_offset.push(corner_edge.len());
        for (a, b) in f.edges() {
            let e = edge_of(a, b, &mut edges);
            let se = &mut edges[e as usize];
            if se.count < 2 {
                se.faces[se.count as usize] = fi as u32;
            }
            se.count += 1;
            corner_edge.push(e);
        }
    }
    let loose: Vec<u32> = m.loose_edges.iter().map(|e| edge_of(e[0], e[1], &mut edges)).collect();

    // Face points: centroids.
    let face_pts: Vec<Vec3> = (0..m.faces.len()).map(|f| m.face_center(f)).collect();

    // Edge points: average of the ends and the two face points, or the midpoint.
    let edge_pts: Vec<Vec3> = edges
        .iter()
        .map(|e| {
            let mid = (pos[e.v[0] as usize] + pos[e.v[1] as usize]) * 0.5;
            if !simple && e.count == 2 {
                (mid * 2.0 + face_pts[e.faces[0] as usize] + face_pts[e.faces[1] as usize]) * 0.25
            } else {
                mid
            }
        })
        .collect();

    // Vertex points.
    let vert_pts: Vec<Vec3> = if simple {
        pos.clone()
    } else {
        let mut face_sum = vec![Vec3::ZERO; nv];
        let mut face_n = vec![0u32; nv];
        let mut mid_sum = vec![Vec3::ZERO; nv];
        let mut edge_n = vec![0u32; nv];
        let mut sharp_sum = vec![Vec3::ZERO; nv];
        let mut sharp_n = vec![0u32; nv];
        for (fi, f) in m.faces.iter().enumerate() {
            for &v in &f.verts {
                face_sum[v as usize] += face_pts[fi];
                face_n[v as usize] += 1;
            }
        }
        for e in &edges {
            let [a, b] = [e.v[0] as usize, e.v[1] as usize];
            let mid = (pos[a] + pos[b]) * 0.5;
            mid_sum[a] += mid;
            mid_sum[b] += mid;
            edge_n[a] += 1;
            edge_n[b] += 1;
            if e.count != 2 {
                sharp_sum[a] += pos[b];
                sharp_sum[b] += pos[a];
                sharp_n[a] += 1;
                sharp_n[b] += 1;
            }
        }
        (0..nv)
            .map(|v| {
                let p = pos[v];
                if face_n[v] == 0 {
                    p
                } else if sharp_n[v] == 0 {
                    let n = face_n[v] as f32;
                    let f = face_sum[v] / n;
                    let r = mid_sum[v] / edge_n[v] as f32;
                    (f + r * 2.0 + p * (n - 3.0)) / n
                } else if sharp_n[v] == 2 && face_n[v] >= 2 {
                    (p * 6.0 + sharp_sum[v]) / 8.0
                } else {
                    p
                }
            })
            .collect()
    };

    let ne = edges.len() as u32;
    let mut out = Mesh::new();
    out.positions = vert_pts;
    out.positions.extend_from_slice(&edge_pts);
    out.positions.extend_from_slice(&face_pts);
    out.faces.reserve(corner_edge.len());
    for (fi, f) in m.faces.iter().enumerate() {
        let n = f.verts.len();
        if n < 3 {
            continue;
        }
        let fp = nv as u32 + ne + fi as u32;
        let ce = &corner_edge[face_offset[fi]..face_offset[fi] + n];
        let has_uv = f.uvs.len() == n;
        let face_uv = if has_uv { f.uvs.iter().fold(Vec2::ZERO, |a, &b| a + b) * (1.0 / n as f32) } else { Vec2::ZERO };
        for i in 0..n {
            let ip = (i + n - 1) % n;
            let verts = vec![f.verts[i], nv as u32 + ce[i], fp, nv as u32 + ce[ip]];
            let uvs = if has_uv {
                let (u, un, up) = (f.uvs[i], f.uvs[(i + 1) % n], f.uvs[ip]);
                vec![u, u.lerp(un, 0.5), face_uv, up.lerp(u, 0.5)]
            } else {
                Vec::new()
            };
            out.faces.push(Face { verts, uvs, material: f.material, smooth: f.smooth });
        }
    }
    for e in loose {
        let [a, b] = edges[e as usize].v;
        let mid = nv as u32 + e;
        out.loose_edges.push([a, mid]);
        out.loose_edges.push([mid, b]);
    }
    out
}

fn ekey(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

// ---------------------------------------------------------------------------
// Mirror, array, solidify

fn axis_value(p: Vec3, axis: usize) -> f32 {
    p[axis]
}

fn set_axis(p: &mut Vec3, axis: usize, v: f32) {
    match axis {
        0 => p.x = v,
        1 => p.y = v,
        _ => p.z = v,
    }
}

/// Mirror across each enabled local axis in turn (X, then Y, then Z).
fn mirror(m: &Mesh, axes: [bool; 3], merge: Option<f32>) -> Mesh {
    let mut out = m.clone();
    for (axis, &on) in axes.iter().enumerate() {
        if on {
            out = mirror_axis(&out, axis, merge);
        }
    }
    out
}

/// Append a copy of `m` reflected through the plane `axis = 0`. The copy's
/// winding is reversed so its normals still face outwards. With `merge`, seam
/// vertices within the distance of the plane are snapped onto it and shared by
/// both halves, and copied faces lying entirely in the plane are dropped.
fn mirror_axis(m: &Mesh, axis: usize, merge: Option<f32>) -> Mesh {
    let n = m.positions.len();
    let mut out = m.clone();
    let mut map = vec![0u32; n];
    let mut on_plane = vec![false; n];
    let flip: Vec<u32> = m
        .vertex_groups
        .iter()
        .enumerate()
        .map(|(g, name)| m.vertex_group_index(&crate::mesh::flip_side_name(name)).unwrap_or(g) as u32)
        .collect();
    for i in 0..n {
        let c = axis_value(m.positions[i], axis);
        if let Some(d) = merge {
            if c.abs() <= d {
                set_axis(&mut out.positions[i], axis, 0.0);
                map[i] = i as u32;
                on_plane[i] = true;
                continue;
            }
        }
        let mut p = m.positions[i];
        set_axis(&mut p, axis, -c);
        map[i] = out.copy_vertex(i as u32);
        out.positions[map[i] as usize] = p;
        // Shape keys are mirrored too; weights move to the flipped group
        // ("Arm.L" -> "Arm.R") when the mesh has it (Mirror Vertex Groups).
        for k in &mut out.shape_keys {
            if let Some(kp) = k.positions.get_mut(map[i] as usize) {
                let c = axis_value(*kp, axis);
                set_axis(kp, axis, -c);
            }
        }
        if let Some(w) = out.weights.get_mut(map[i] as usize) {
            for e in w.iter_mut() {
                e.0 = flip.get(e.0 as usize).copied().unwrap_or(e.0);
            }
        }
    }
    for f in &m.faces {
        if merge.is_some() && f.verts.iter().all(|&v| on_plane[v as usize]) {
            continue;
        }
        let mut nf = f.clone();
        nf.verts = f.verts.iter().rev().map(|&v| map[v as usize]).collect();
        nf.uvs.reverse();
        out.faces.push(nf);
    }
    for e in &m.loose_edges {
        if !(on_plane[e[0] as usize] && on_plane[e[1] as usize]) {
            out.loose_edges.push([map[e[0] as usize], map[e[1] as usize]]);
        }
    }
    out
}

/// `count` copies, copy `i` offset by `i * (relative * bounds size + constant)`.
/// With `merge`, coincident vertices of neighbouring copies are welded and the
/// back-to-back faces between them removed, so an array of cubes is one box.
fn array(m: &Mesh, count: u32, relative: Vec3, constant: Vec3, merge: bool) -> Mesh {
    let count = count.max(1);
    let b = m.bounds();
    let size = if b.is_empty() { Vec3::ZERO } else { b.size() };
    let step = relative.mul_elem(size) + constant;
    let mut out = Mesh::new();
    for i in 0..count {
        let mut c = m.clone();
        c.translate(step * i as f32);
        out.append(&c);
    }
    if merge && count > 1 {
        out.weld(1e-4 * size.max_elem().max(1.0));
        ops::remove_doubled_faces(&mut out);
        out.remove_unused_vertices();
    }
    out
}

/// Thicken a surface into a shell. The original surface moves to
/// `thickness * (offset + 1) / 2` along the vertex normals and a reversed copy
/// to `thickness * (offset - 1) / 2`, so offset -1 (Blender's default) grows
/// the shell behind the surface. Boundary edges get rim quads, so an open
/// surface becomes a closed solid.
pub fn solidify(m: &Mesh, thickness: f32, offset: f32) -> Mesh {
    let o = offset.clamp(-1.0, 1.0);
    let outer = thickness * (o + 1.0) * 0.5;
    let inner = thickness * (o - 1.0) * 0.5;
    let normals = m.vertex_normals();
    let n = m.positions.len() as u32;
    let mut out = Mesh::new();
    out.positions.reserve(2 * n as usize);
    out.positions.extend(m.positions.iter().zip(&normals).map(|(&p, &nr)| p + nr * outer));
    out.positions.extend(m.positions.iter().zip(&normals).map(|(&p, &nr)| p + nr * inner));
    let directed: std::collections::HashSet<(u32, u32)> = m.faces.iter().flat_map(|f| f.edges()).collect();
    out.faces.extend(m.faces.iter().cloned());
    for f in &m.faces {
        let mut nf = f.clone();
        nf.verts = f.verts.iter().rev().map(|&v| v + n).collect();
        nf.uvs.reverse();
        out.faces.push(nf);
    }
    for f in &m.faces {
        let k = f.verts.len();
        for i in 0..k {
            let (a, b) = (f.verts[i], f.verts[(i + 1) % k]);
            if directed.contains(&(b, a)) {
                continue;
            }
            let uvs = if f.uvs.len() == k {
                let (ua, ub) = (f.uvs[i], f.uvs[(i + 1) % k]);
                vec![ub, ua, ua, ub]
            } else {
                Vec::new()
            };
            out.faces.push(Face { verts: vec![b, a, a + n, b + n], uvs, material: f.material, smooth: f.smooth });
        }
    }
    out.loose_edges = m.loose_edges.clone();
    if thickness < 0.0 {
        out.flip_normals();
    }
    out
}

// ---------------------------------------------------------------------------
// Bevel, triangulate, displace, smooth

/// Bevel every manifold edge whose dihedral angle (between face normals)
/// exceeds `angle` radians.
fn bevel_by_angle(m: &Mesh, width: f32, segments: u32, angle: f32) -> Mesh {
    let mut out = m.clone();
    if width <= 0.0 {
        return out;
    }
    let normals: Vec<Vec3> = (0..m.faces.len()).map(|f| m.face_normal(f)).collect();
    let mut by_edge: HashMap<(u32, u32), (u32, u32, u32)> = HashMap::new();
    for (fi, f) in m.faces.iter().enumerate() {
        for (a, b) in f.edges() {
            let e = by_edge.entry(ekey(a, b)).or_insert((u32::MAX, u32::MAX, 0));
            if e.2 == 0 {
                e.0 = fi as u32;
            } else {
                e.1 = fi as u32;
            }
            e.2 += 1;
        }
    }
    let cos_limit = angle.cos();
    let mut edges: Vec<[u32; 2]> = by_edge
        .iter()
        .filter(|(_, &(f0, f1, c))| c == 2 && normals[f0 as usize].dot(normals[f1 as usize]) < cos_limit)
        .map(|(&(a, b), _)| [a, b])
        .collect();
    edges.sort_unstable();
    ops::bevel_edges(&mut out, &edges, width, segments.max(1));
    out
}

/// Split every face into triangles (ear clipping for n-gons), keeping UVs,
/// material and shading.
fn triangulate(m: &Mesh) -> Mesh {
    let mut out = m.clone();
    out.faces = Vec::new();
    for t in m.triangulate() {
        let f = &m.faces[t.face as usize];
        let uvs = if f.uvs.len() == f.verts.len() { t.corners.iter().map(|&c| f.uvs[c as usize]).collect() } else { Vec::new() };
        out.faces.push(Face { verts: t.verts.to_vec(), uvs, material: f.material, smooth: f.smooth });
    }
    out
}

/// Push every vertex along its normal by `strength`.
fn displace(m: &Mesh, strength: f32) -> Mesh {
    let mut out = m.clone();
    for (p, n) in out.positions.iter_mut().zip(m.vertex_normals()) {
        *p += n * strength;
    }
    out
}

/// Laplacian smoothing: each iteration moves every vertex `factor` of the way
/// towards the average of its edge neighbours.
fn smooth(m: &Mesh, factor: f32, iterations: u32) -> Mesh {
    let mut out = m.clone();
    let nv = m.positions.len();
    let (start, adj) = adjacency(m);
    for _ in 0..iterations {
        let prev = out.positions.clone();
        for v in 0..nv {
            let nb = &adj[start[v]..start[v + 1]];
            if nb.is_empty() {
                continue;
            }
            let avg = nb.iter().fold(Vec3::ZERO, |a, &u| a + prev[u as usize]) / nb.len() as f32;
            out.positions[v] = prev[v] + (avg - prev[v]) * factor;
        }
    }
    out
}

/// Compressed vertex adjacency: the edge neighbours of `v` are
/// `adj[start[v]..start[v + 1]]`.
fn adjacency(m: &Mesh) -> (Vec<usize>, Vec<u32>) {
    let nv = m.positions.len();
    let edges = m.edges();
    let mut start = vec![0usize; nv + 1];
    for e in &edges {
        start[e[0] as usize + 1] += 1;
        start[e[1] as usize + 1] += 1;
    }
    for i in 0..nv {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    let mut adj = vec![0u32; start[nv]];
    for e in &edges {
        adj[fill[e[0] as usize]] = e[1];
        fill[e[0] as usize] += 1;
        adj[fill[e[1] as usize]] = e[0];
        fill[e[1] as usize] += 1;
    }
    (start, adj)
}
