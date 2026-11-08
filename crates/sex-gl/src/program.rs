// SPDX-License-Identifier: GPL-3.0-only
//! Shader compile and program link: GLSL ES -> GLSL 450 (glsl.rs) -> naga module, validated,
//! -> interpreter code (exec.rs), plus the uniform and attribute tables WebGL asks for.
use crate::exec::{Io, StageCode, UniformSlot, K};
use crate::glsl::{self, SamplerKind, Stage};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use naga::front::glsl::{Frontend, Options};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{ArraySize, Module, ShaderStage, TypeInner};

/// GL type enums for active uniforms and attributes.
pub mod types {
    pub const FLOAT: u32 = 0x1406;
    pub const FLOAT_VEC2: u32 = 0x8B50;
    pub const FLOAT_VEC3: u32 = 0x8B51;
    pub const FLOAT_VEC4: u32 = 0x8B52;
    pub const INT: u32 = 0x1404;
    pub const INT_VEC2: u32 = 0x8B53;
    pub const INT_VEC3: u32 = 0x8B54;
    pub const INT_VEC4: u32 = 0x8B55;
    pub const BOOL: u32 = 0x8B56;
    pub const BOOL_VEC2: u32 = 0x8B57;
    pub const BOOL_VEC3: u32 = 0x8B58;
    pub const BOOL_VEC4: u32 = 0x8B59;
    pub const FLOAT_MAT2: u32 = 0x8B5A;
    pub const FLOAT_MAT3: u32 = 0x8B5B;
    pub const FLOAT_MAT4: u32 = 0x8B5C;
    pub const SAMPLER_2D: u32 = 0x8B5E;
    pub const SAMPLER_3D: u32 = 0x8B5F;
    pub const SAMPLER_CUBE: u32 = 0x8B60;
    pub const SAMPLER_2D_SHADOW: u32 = 0x8B62;
    pub const SAMPLER_2D_ARRAY: u32 = 0x8DC1;
    pub const UNSIGNED_INT: u32 = 0x1405;
    pub const UNSIGNED_INT_VEC2: u32 = 0x8DC6;
    pub const UNSIGNED_INT_VEC3: u32 = 0x8DC7;
    pub const UNSIGNED_INT_VEC4: u32 = 0x8DC8;
}

fn gl_type(kind: K, cols: u8, rows: u8) -> u32 {
    use types::*;
    if cols > 1 { return match cols { 2 => FLOAT_MAT2, 3 => FLOAT_MAT3, _ => FLOAT_MAT4 }; }
    let base = match kind { K::F => FLOAT_VEC2 - 1, K::I => INT_VEC2 - 1, K::U => UNSIGNED_INT_VEC2 - 1, K::B => BOOL_VEC2 - 1 };
    match (kind, rows) {
        (K::F, 1) => FLOAT,
        (K::I, 1) => INT,
        (K::U, 1) => UNSIGNED_INT,
        (K::B, 1) => BOOL,
        (_, r) => base + r as u32 - 1,
    }
}

fn sampler_type(kind: SamplerKind) -> u32 {
    match kind {
        SamplerKind::D2 => types::SAMPLER_2D,
        SamplerKind::Cube => types::SAMPLER_CUBE,
        SamplerKind::D3 => types::SAMPLER_3D,
        SamplerKind::D2Array => types::SAMPLER_2D_ARRAY,
        SamplerKind::D2Shadow => types::SAMPLER_2D_SHADOW,
    }
}

/// A checked shader stage, kept as its source until link (varyings are matched then).
pub struct Shader {
    pub stage: Stage,
    pub source: String,
    pub ok: bool,
    pub log: String,
}

impl Shader {
    /// "Compiles": translates and parses on its own to report errors now.
    pub fn compile(stage: Stage, source: &str) -> Shader {
        let mut varyings = Vec::new();
        let res = glsl::translate(source, stage, &mut varyings).and_then(|t| parse(stage, &t.placed()).map(|_| ()));
        match res {
            Ok(()) => Shader { stage, source: source.to_string(), ok: true, log: String::new() },
            Err(e) => Shader { stage, source: source.to_string(), ok: false, log: e },
        }
    }
}

fn parse(stage: Stage, src: &str) -> Result<(Module, naga::valid::ModuleInfo), String> {
    let st = match stage { Stage::Vertex => ShaderStage::Vertex, Stage::Fragment => ShaderStage::Fragment };
    let mut fe = Frontend::default();
    let module = fe.parse(&Options::from(st), src).map_err(|e| {
        let mut s = String::new();
        for err in e.errors.iter() { s.push_str(&format!("ERROR: {}\n", err.kind)); }
        s
    })?;
    let info = Validator::new(ValidationFlags::all() ^ ValidationFlags::BINDINGS, Capabilities::all())
        .validate(&module)
        .map_err(|e| format!("ERROR: {}\n", e.as_inner()))?;
    Ok((module, info))
}

/// An active uniform: GL type and array size, and where it lives in each stage.
#[derive(Clone, Debug)]
pub struct Uniform {
    pub name: String,
    pub gl_type: u32,
    pub size: u32,
    pub vs: Option<UniformSlot>,
    pub fs: Option<UniformSlot>,
    /// For samplers: index into `Program::samplers`.
    pub sampler: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Sampler {
    pub name: String,
    pub kind: SamplerKind,
    pub units: Vec<i32>,
    /// Texture global index per stage, per array element.
    pub vs_globals: Vec<u32>,
    pub fs_globals: Vec<u32>,
}

/// A linked program.
pub struct Program {
    pub vs: StageCode,
    pub fs: StageCode,
    pub vs_uniforms: Vec<u32>,
    pub fs_uniforms: Vec<u32>,
    pub uniforms: Vec<Uniform>,
    pub samplers: Vec<Sampler>,
    /// Active attributes: name, location, GL type, size.
    pub attributes: Vec<(String, u32, u32, u32)>,
    /// Varyings: vertex output offset, fragment input offset (in the fragment's input
    /// area order), lanes.
    pub links: Vec<(u32, u32, u32)>,
    /// Lane offset of gl_Position (and gl_PointSize) in the vertex outputs.
    pub position: u32,
    pub point_size: Option<u32>,
    /// Fragment color output offset (location 0) and lanes.
    pub color: Option<(u32, u32)>,
    pub frag_depth: Option<u32>,
    /// Lanes of interpolated data per vertex (the varyings, packed).
    pub varying_lanes: u32,
}

impl Program {
    pub fn link(vs: &Shader, fs: &Shader, bindings: &BTreeMap<String, u32>) -> Result<Program, String> {
        if !vs.ok || !fs.ok { return Err("ERROR: a shader did not compile".into()); }
        let mut varyings = Vec::new();
        let tv = glsl::translate(&vs.source, Stage::Vertex, &mut varyings)?;
        let tf = glsl::translate(&fs.source, Stage::Fragment, &mut varyings)?;
        // Locations bound by the program, and those the shader declares itself
        // (`layout(location=N)` in ES 3.00), are kept; the rest take free ones.
        let fixed_loc = |name: &String, loc: u32| bindings.get(name).copied().or(if tv.explicit.contains(name) { Some(loc) } else { None });
        let used: Vec<u32> = tv.attributes.iter().filter_map(|(n, l)| fixed_loc(n, *l)).collect();
        let mut attrs: Vec<(String, u32)> = Vec::new();
        let mut vsrc = tv.source.clone();
        let mut next = 0;
        // Highest index first, so `sxa1` doesn't match inside `sxa10`.
        let mut placed: Vec<(usize, u32)> = Vec::new();
        for (i, (name, loc)) in tv.attributes.iter().enumerate() {
            let l = match fixed_loc(name, *loc) {
                Some(l) => l,
                None => {
                    while used.contains(&next) || attrs.iter().any(|(_, l)| *l == next) { next += 1; }
                    next += 1;
                    next - 1
                }
            };
            placed.push((i, l));
            attrs.push((name.clone(), l));
        }
        for (i, l) in placed.into_iter().rev() {
            vsrc = vsrc.replace(&format!("location=sxa{i})"), &format!("location={l})"));
        }

        let (vm, vi) = parse(Stage::Vertex, &vsrc)?;
        let (fm, fi) = parse(Stage::Fragment, &tf.source)?;
        let vu = reflect(&vm);
        let fu = reflect(&fm);
        let vs_code = StageCode::new(vm, vi)?;
        let fs_code = StageCode::new(fm, fi)?;

        let mut uniforms: Vec<Uniform> = Vec::new();
        for (name, slot) in vu {
            uniforms.push(Uniform { gl_type: gl_type(slot.kind, slot.cols, slot.rows), size: slot.count, name, vs: Some(slot), fs: None, sampler: None });
        }
        for (name, slot) in fu {
            match uniforms.iter_mut().find(|u| u.name == name) {
                Some(u) => u.fs = Some(slot),
                None => uniforms.push(Uniform { gl_type: gl_type(slot.kind, slot.cols, slot.rows), size: slot.count, name, vs: None, fs: Some(slot), sampler: None }),
            }
        }
        let mut samplers: Vec<Sampler> = Vec::new();
        for (decls, code, is_vs) in [(&tv.samplers, &vs_code, true), (&tf.samplers, &fs_code, false)] {
            for d in decls {
                let globals: Vec<u32> = code.textures.iter().filter(|(n, _)| *n == format!("{}_sxt", d.name)).map(|(_, g)| *g).collect();
                let idx = match samplers.iter().position(|s| s.name == d.name) {
                    Some(i) => i,
                    None => {
                        samplers.push(Sampler { name: d.name.clone(), kind: d.kind, units: alloc::vec![0; d.count as usize], vs_globals: Vec::new(), fs_globals: Vec::new() });
                        uniforms.push(Uniform { name: d.name.clone(), gl_type: sampler_type(d.kind), size: d.count, vs: None, fs: None, sampler: Some(samplers.len() - 1) });
                        samplers.len() - 1
                    }
                };
                if is_vs { samplers[idx].vs_globals = globals; } else { samplers[idx].fs_globals = globals; }
            }
        }

        let mut attributes = Vec::new();
        for (io, off, lanes) in &vs_code.inputs {
            let _ = off;
            if let Io::Location(l) = io {
                if let Some((name, _)) = attrs.iter().find(|(_, al)| al == l) {
                    let t = match lanes { 1 => types::FLOAT, 2 => types::FLOAT_VEC2, 3 => types::FLOAT_VEC3, 4 => types::FLOAT_VEC4, 9 => types::FLOAT_MAT3, 16 => types::FLOAT_MAT4, _ => types::FLOAT_VEC4 };
                    attributes.push((name.clone(), *l, t, 1));
                }
            }
        }

        // Varyings: pack the vertex outputs that the fragment stage reads.
        let mut links = Vec::new();
        let mut varying_lanes = 0;
        for (io, off, lanes) in &fs_code.inputs {
            if let Io::Location(l) = io {
                if let Some((_, vo, vl)) = vs_code.outputs.iter().find(|(o, _, _)| *o == Io::Location(*l)) {
                    let n = (*lanes).min(*vl);
                    links.push((*vo, *off, n));
                    varying_lanes += n;
                }
            }
        }
        let position = vs_code.outputs.iter().find(|(o, _, _)| *o == Io::Position).map(|(_, off, _)| *off).ok_or("ERROR: vertex shader writes no gl_Position")?;
        let point_size = vs_code.outputs.iter().find(|(o, _, _)| *o == Io::PointSize).map(|(_, off, _)| *off);
        let color = fs_code.outputs.iter().find(|(o, _, _)| *o == Io::Location(0)).map(|(_, off, l)| (*off, *l));
        let frag_depth = fs_code.outputs.iter().find(|(o, _, _)| *o == Io::FragDepth).map(|(_, off, _)| *off);
        Ok(Program {
            vs_uniforms: alloc::vec![0; vs_code.uniform_len as usize],
            fs_uniforms: alloc::vec![0; fs_code.uniform_len as usize],
            vs: vs_code, fs: fs_code, uniforms, samplers, attributes, links, position, point_size, color, frag_depth, varying_lanes,
        })
    }

    /// Resolves `name`, `name[i]` (arrays of values or of samplers) to a uniform and
    /// element.
    pub fn locate(&self, name: &str) -> Option<(usize, u32)> {
        if let Some(i) = self.uniforms.iter().position(|u| u.name == name) { return Some((i, 0)); }
        let (base, idx) = name.strip_suffix(']').and_then(|s| s.rsplit_once('['))?;
        let idx: u32 = idx.parse().ok()?;
        let i = self.uniforms.iter().position(|u| u.name == base)?;
        if idx >= self.uniforms[i].size { return None; }
        Some((i, idx))
    }

    /// Sets uniform `u`, starting at element `elem`, from floats or ints (`bits` holds the
    /// raw lanes, already in the uniform's kind; matrices column by column).
    pub fn set(&mut self, u: usize, elem: u32, bits: &[u32]) {
        let uni = &self.uniforms[u];
        if let Some(si) = uni.sampler {
            let s = &mut self.samplers[si];
            for (j, b) in bits.iter().enumerate() {
                if let Some(slot) = s.units.get_mut(elem as usize + j) { *slot = *b as i32; }
            }
            return;
        }
        for (slot, mem) in [(uni.vs, &mut self.vs_uniforms), (uni.fs, &mut self.fs_uniforms)] {
            let Some(slot) = slot else { continue };
            let per = (slot.cols as u32 * slot.rows as u32) as usize;
            let mut at = 0;
            let mut e = elem;
            while at < bits.len() && e < slot.count {
                let off = (slot.offset + e * slot.stride) as usize;
                let n = per.min(bits.len() - at);
                if off + n <= mem.len() { mem[off..off + n].copy_from_slice(&bits[at..at + n]); }
                at += per;
                e += 1;
            }
        }
    }
}

/// Lists the leaf uniforms in a module's uniform blocks.
fn reflect(m: &Module) -> Vec<(String, UniformSlot)> {
    let layout_lanes = |ty| crate_lanes(m, ty);
    let mut out = Vec::new();
    let mut base = 0u32;
    for (_, g) in m.global_variables.iter() {
        if g.space != naga::AddressSpace::Uniform { continue; }
        if let TypeInner::Struct { members, .. } = &m.types[g.ty].inner {
            let mut off = base;
            for mem in members {
                if let Some(name) = &mem.name { leaf(m, name.clone(), mem.ty, off, &mut out); }
                off += layout_lanes(mem.ty);
            }
        }
        base += layout_lanes(g.ty);
    }
    // Booleans the translation stored as uint.
    for (name, slot) in out.iter_mut() {
        if let Some(n) = name.strip_suffix("_sxb") { *name = n.into(); slot.kind = K::B; }
    }
    out
}

fn crate_lanes(m: &Module, ty: naga::Handle<naga::Type>) -> u32 {
    match &m.types[ty].inner {
        TypeInner::Scalar(_) => 1,
        TypeInner::Vector { size, .. } => *size as u32,
        TypeInner::Matrix { columns, rows, .. } => *columns as u32 * *rows as u32,
        TypeInner::Array { base, size, .. } => crate_lanes(m, *base) * match size { ArraySize::Constant(n) => n.get(), _ => 1 },
        TypeInner::Struct { members, .. } => members.iter().map(|x| crate_lanes(m, x.ty)).sum(),
        _ => 1,
    }
}

fn scalar_k(s: naga::Scalar) -> K {
    match s.kind { naga::ScalarKind::Float | naga::ScalarKind::AbstractFloat => K::F, naga::ScalarKind::Uint => K::U, naga::ScalarKind::Bool => K::B, _ => K::I }
}

fn leaf(m: &Module, name: String, ty: naga::Handle<naga::Type>, off: u32, out: &mut Vec<(String, UniformSlot)>) {
    match &m.types[ty].inner {
        TypeInner::Struct { members, .. } => {
            let mut o = off;
            for mem in members {
                if let Some(n) = &mem.name { leaf(m, format!("{name}.{n}"), mem.ty, o, out); }
                o += crate_lanes(m, mem.ty);
            }
        }
        TypeInner::Array { base, size, .. } => {
            let n = match size { ArraySize::Constant(n) => n.get(), _ => 1 };
            let stride = crate_lanes(m, *base);
            match &m.types[*base].inner {
                TypeInner::Struct { .. } | TypeInner::Array { .. } => {
                    for i in 0..n { leaf(m, format!("{name}[{i}]"), *base, off + i * stride, out); }
                }
                inner => {
                    let (kind, cols, rows) = shape(inner);
                    out.push((name, UniformSlot { offset: off, kind, cols, rows, count: n, stride }));
                }
            }
        }
        inner => {
            let (kind, cols, rows) = shape(inner);
            out.push((name, UniformSlot { offset: off, kind, cols, rows, count: 1, stride: crate_lanes(m, ty) }));
        }
    }
}

fn shape(inner: &TypeInner) -> (K, u8, u8) {
    match inner {
        TypeInner::Scalar(s) => (scalar_k(*s), 1, 1),
        TypeInner::Vector { size, scalar } => (scalar_k(*scalar), 1, *size as u8),
        TypeInner::Matrix { columns, rows, scalar } => (scalar_k(*scalar), *columns as u8, *rows as u8),
        _ => (K::F, 1, 1),
    }
}
