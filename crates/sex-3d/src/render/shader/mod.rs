// SPDX-License-Identifier: GPL-3.0-only
//! Shader node evaluation for the path tracer, Polyamour's equivalent of
//! Cycles' SVM.
//!
//! **Compiling** ([`compile_material`], [`compile_world`]): when a scene is
//! prepared, each material/world [`NodeTree`](crate::nodes::NodeTree) becomes a
//! flat [`Program`]. Starting from the active output node the compiler walks
//! the links depth first, so every instruction comes after the ones it reads
//! (a topological order). Node groups are inlined (each group instance gets its
//! own frame), reroutes and muted nodes pass their input through the way
//! Blender's internal links do, links between different socket types get
//! conversion instructions, and nodes whose inputs are all constant are
//! evaluated once at compile time (constant folding). Unlinked "implicit"
//! inputs read the shading point: `Normal` the shading normal, texture
//! `Vector`s the generated coordinates (UVs for Image Texture), `Tangent` the
//! default radial tangent.
//!
//! Instructions read and write a register file of `[f32; 4]` values (float in
//! `[0]`, vectors and colours in `[0..3]`, alpha in `[3]`), whose constants are
//! preloaded from [`Program::init`]. Shader sockets do not become registers:
//! they form a separate tree of [`ShaderOp`]s (Mix, Add, closures) that is
//! walked after the data instructions with a running weight, like Cycles'
//! closure weight propagation, producing a [`Closures`] list.
//!
//! **Bump** is evaluated like Cycles: the `Height` input's sub-graph is compiled
//! three times, at the shading point and offset along two surface tangents
//! (instruction `variant` 0, 1, 2); position-reading inputs (texture
//! coordinates, geometry) see the offset point. The Material Output's
//! `Displacement` input is treated as "bump only" displacement the same way:
//! the displaced position is evaluated in three variants and the shading
//! normal is replaced by the normal of the displaced surface before the
//! surface shader runs.
//!
//! **Running** ([`Program::eval_surface`]): the VM executes the instructions
//! with a [`ShadeCtx`] describing the hit (position, normals, UV, incoming
//! direction, ray type for Light Path, object data, ...) and returns the
//! closures: BSDF lobes with weights (see `bsdf::closure`), emission,
//! transparency, holdout and volume coefficients.
pub(crate) mod color;
mod compile;
pub(crate) mod ops;
mod sky;
pub(crate) mod surface;
pub(crate) mod tex;
pub(crate) mod vm;

pub(crate) use compile::{compile_material, compile_world};
pub(crate) use vm::{Closures, Op};

use crate::math::Vec3;

pub(crate) type Reg = u32;
pub(crate) type V4 = [f32; 4];

/// One VM instruction: `op` reads registers `ins` (in the node's input order)
/// and writes its outputs to consecutive registers from `out`.
#[derive(Clone, Debug)]
pub(crate) struct Instr {
    pub op: Op,
    pub ins: Vec<Reg>,
    pub out: Reg,
    /// 0: the shading point; 1, 2: offset along dPdx, dPdy (bump).
    pub variant: u8,
}

/// The closure tree.
#[derive(Clone, Debug)]
pub(crate) enum ShaderOp {
    Mix { fac: Reg, a: Option<u32>, b: Option<u32> },
    Add { a: Option<u32>, b: Option<u32> },
    Closure { kind: ClosureKind, ins: Vec<Reg> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosureKind {
    /// `ms`: multiple-scattering GGX (energy compensation).
    Principled { ms: bool },
    Diffuse,
    Emission,
    Background,
    Glass { ms: bool },
    /// Glossy BSDF (anisotropic since 4.0; the legacy node maps here too).
    Glossy { ms: bool },
    Refraction { ms: bool },
    /// Sheen BSDF (the legacy Velvet maps here too).
    Sheen,
    Translucent,
    Transparent,
    Holdout,
    Toon { glossy: bool },
    Subsurface,
    Hair,
    HairPrincipled,
    VolumePrincipled,
    VolumeAbsorption,
    VolumeScatter,
}

/// What a compiled tree may produce (for light and shadow set-up).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProgramInfo {
    /// Some closure emits light.
    pub emission: bool,
    /// Some closure lets light through without refraction (Transparent,
    /// Principled alpha < 1, volume-only materials).
    pub transparent: bool,
    pub volume: bool,
    /// The program reads the UV tangent (Normal Map, Tangent node in UV mode).
    pub uv_tangent: bool,
}

/// A compiled shader node tree.
#[derive(Clone, Debug, Default)]
pub(crate) struct Program {
    pub init: Vec<V4>,
    /// Displacement instructions, run first with the original normal.
    pub disp: Vec<Instr>,
    /// Displacement vectors at the three bump variants.
    pub disp_out: Option<[Reg; 3]>,
    /// Surface and volume instructions.
    pub surf: Vec<Instr>,
    pub surface: Option<u32>,
    pub volume: Option<u32>,
    pub shaders: Vec<ShaderOp>,
    pub info: ProgramInfo,
}

/// Ray type flags for the Light Path node.
pub(crate) const RAY_CAMERA: u16 = 1;
pub(crate) const RAY_SHADOW: u16 = 2;
pub(crate) const RAY_DIFFUSE: u16 = 4;
pub(crate) const RAY_GLOSSY: u16 = 8;
pub(crate) const RAY_SINGULAR: u16 = 16;
pub(crate) const RAY_REFLECT: u16 = 32;
pub(crate) const RAY_TRANSMIT: u16 = 64;
pub(crate) const RAY_VOLUME_SCATTER: u16 = 128;

/// Where a path is: the type of the ray that reached the current vertex and
/// the bounce counts so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RayState {
    pub flags: u16,
    pub depth: u16,
    pub diffuse: u16,
    pub glossy: u16,
    pub transparent: u16,
    pub transmission: u16,
}

impl RayState {
    pub fn camera() -> RayState {
        RayState { flags: RAY_CAMERA, ..Default::default() }
    }

    pub fn shadow() -> RayState {
        RayState { flags: RAY_SHADOW, ..Default::default() }
    }
}

/// Everything the VM may ask about the shading point.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShadeCtx<'a> {
    /// The scene, for nodes that trace rays (Ambient Occlusion).
    pub scene: Option<&'a crate::render::PreparedScene>,
    /// Images, objects and camera for texture and input nodes.
    pub data: Option<&'a surface::ShadeData>,
    /// World position (for the world: the ray direction).
    pub p: Vec3,
    /// Bump offsets of the position (world space).
    pub dpdx: Vec3,
    pub dpdy: Vec3,
    /// Shading normal, facing the incoming side (after displacement).
    pub n: Vec3,
    /// Interpolated normal before any perturbation, facing.
    pub n_smooth: Vec3,
    /// True (geometric) normal, facing.
    pub ng: Vec3,
    /// Towards where the ray came from (`-D`).
    pub i: Vec3,
    pub backfacing: bool,
    pub uv: Vec3,
    pub duvdx: Vec3,
    pub duvdy: Vec3,
    /// Barycentrics (u, v) of the hit.
    pub bary: (f32, f32),
    /// UV tangent (world, unit) and bitangent sign, when available.
    pub tangent: Option<(Vec3, f32)>,
    /// Scene object index.
    pub object: Option<u32>,
    pub material_index: f32,
    pub ray: RayState,
    pub ray_length: f32,
    /// The hit triangle's corners (world), for Wireframe.
    pub tri: [Vec3; 3],
    /// Evaluating the world shader.
    pub world: bool,
    /// Random bits for AO and Bevel sampling.
    pub seed: u32,
}

impl Default for ShadeCtx<'_> {
    fn default() -> Self {
        ShadeCtx {
            scene: None,
            data: None,
            p: Vec3::ZERO,
            dpdx: Vec3::ZERO,
            dpdy: Vec3::ZERO,
            n: Vec3::Z,
            n_smooth: Vec3::Z,
            ng: Vec3::Z,
            i: Vec3::Z,
            backfacing: false,
            uv: Vec3::ZERO,
            duvdx: Vec3::ZERO,
            duvdy: Vec3::ZERO,
            bary: (0.0, 0.0),
            tangent: None,
            object: None,
            material_index: 0.0,
            ray: RayState::camera(),
            ray_length: 0.0,
            tri: [Vec3::ZERO; 3],
            world: false,
            seed: 0,
        }
    }
}

impl ShadeCtx<'_> {
    /// The position seen by bump variant `v`.
    pub fn pos(&self, v: u8) -> Vec3 {
        match v {
            1 => self.p + self.dpdx,
            2 => self.p + self.dpdy,
            _ => self.p,
        }
    }

    pub fn uv_at(&self, v: u8) -> Vec3 {
        match v {
            1 => self.uv + self.duvdx,
            2 => self.uv + self.duvdy,
            _ => self.uv,
        }
    }
}

/// Per-thread scratch for running programs (registers), reused between hits.
#[derive(Clone, Debug, Default)]
pub(crate) struct Scratch {
    pub regs: Vec<V4>,
}

impl Program {
    /// Run the program at a surface point: displacement first (it may replace
    /// `ctx.n`), then the surface and volume closures.
    pub fn eval_surface(&self, ctx: &mut ShadeCtx, scratch: &mut Scratch) -> Closures {
        let regs = &mut scratch.regs;
        regs.clear();
        regs.extend_from_slice(&self.init);
        if let Some(d) = self.disp_out {
            vm::run(&self.disp, ctx, regs);
            vm::apply_displacement(ctx, regs, d);
        }
        vm::run(&self.surf, ctx, regs);
        let mut out = Closures::default();
        if let Some(s) = self.surface {
            vm::emit(self, s, Vec3::ONE, ctx, regs, &mut out, 0);
        }
        if let Some(v) = self.volume {
            vm::emit(self, v, Vec3::ONE, ctx, regs, &mut out, 0);
        }
        out
    }
}
