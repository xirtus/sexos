// SPDX-License-Identifier: GPL-3.0-only
//! WebGPU render pipelines on the same interpreter and rasterizer as WebGL: the vertex
//! and fragment entry points of WGSL modules (one each, possibly the same) become a
//! `Program`, and its resources are
//! listed by group and binding so the page can feed them. Uniform (and read-only storage)
//! buffers keep WGSL's byte layout in the page; each binding maps those bytes to the
//! interpreter's lanes.
use crate::exec::{Io, StageCode};
use crate::glsl::SamplerKind;
use crate::program::{crate_lanes, Program, Sampler};
use crate::program::types::{FLOAT, FLOAT_VEC2, FLOAT_VEC3, FLOAT_VEC4};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{AddressSpace, ArraySize, Expression as E, ImageClass, ImageDimension, Module, ShaderStage, TypeInner};

#[derive(Clone, Debug)]
pub enum BindKind {
    /// A buffer: (lane in the stage's uniform memory, byte in the buffer) for every lane,
    /// for the vertex and the fragment stage.
    Buffer([Vec<(u32, u32)>; 2]),
    /// A texture, sampled through `Program::samplers[_]`.
    Texture(usize),
    /// A sampler, used with these textures (`Program::samplers` indices).
    Sampler(Vec<usize>),
}

#[derive(Clone, Debug)]
pub struct GpuBind {
    pub group: u32,
    pub binding: u32,
    pub kind: BindKind,
}

/// Lane-by-lane byte offsets of a value of type `ty` at `byte` (WGSL host-shareable layout).
fn walk(m: &Module, ty: naga::Handle<naga::Type>, lane: &mut u32, byte: u32, out: &mut Vec<(u32, u32)>) {
    match &m.types[ty].inner {
        TypeInner::Scalar(_) | TypeInner::Atomic(_) => { out.push((*lane, byte)); *lane += 1; }
        TypeInner::Vector { size, .. } => for i in 0..*size as u32 { out.push((*lane, byte + 4 * i)); *lane += 1; },
        TypeInner::Matrix { columns, rows, .. } => {
            let stride = if *rows as u32 == 2 { 8 } else { 16 };
            for c in 0..*columns as u32 { for r in 0..*rows as u32 { out.push((*lane, byte + c * stride + 4 * r)); *lane += 1; } }
        }
        TypeInner::Array { base, size, stride } => {
            let n = match size { ArraySize::Constant(n) => n.get(), _ => crate::exec::RUNTIME_ARRAY_LEN };
            for i in 0..n { walk(m, *base, lane, byte + i * stride, out); }
        }
        TypeInner::Struct { members, .. } => for mem in members { walk(m, mem.ty, lane, byte + mem.offset, out); },
        _ => *lane += 1,
    }
}

fn stage_code(module: &Module, entry: &str, stage: ShaderStage) -> Result<StageCode, String> {
    let mut m = module.clone();
    m.entry_points.retain(|e| e.stage == stage && (entry.is_empty() || e.name == entry));
    m.entry_points.truncate(1);
    if m.entry_points.is_empty() { return Err(format!("no {stage:?} entry point '{entry}'")); }
    let info = Validator::new(ValidationFlags::all(), Capabilities::all()).validate(&m).map_err(|e| format!("{}", e.as_inner()))?;
    StageCode::new(m, info)
}

/// Texture and sampler globals sampled together anywhere in the module.
fn pairs(m: &Module) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut scan = |f: &naga::Function| {
        for (_, e) in f.expressions.iter() {
            if let E::ImageSample { image, sampler, .. } = e {
                if let (E::GlobalVariable(i), E::GlobalVariable(s)) = (&f.expressions[*image], &f.expressions[*sampler]) {
                    let p = (i.index() as u32, s.index() as u32);
                    if !out.contains(&p) { out.push(p); }
                }
            }
        }
    };
    for (_, f) in m.functions.iter() { scan(f); }
    for ep in &m.entry_points { scan(&ep.function); }
    out
}

impl Program {
    /// A render pipeline's program from WGSL: the vertex entry point of `vs_code` and the
    /// fragment entry point of `fs_code` (empty names: the first of that stage). The two
    /// modules are parsed separately, so they may reuse names.
    pub fn from_wgsl(vs_code: &str, vs_entry: &str, fs_code: &str, fs_entry: &str) -> Result<Program, String> {
        let parse = |code: &str| naga::front::wgsl::parse_str(code).map_err(|e| format!("{}", e.message()));
        let vm = parse(vs_code)?;
        let fm = if fs_code == vs_code { vm.clone() } else { parse(fs_code)? };
        let vs = stage_code(&vm, vs_entry, ShaderStage::Vertex)?;
        let fs = stage_code(&fm, fs_entry, ShaderStage::Fragment)?;
        let mut gpu: Vec<GpuBind> = Vec::new();
        let mut samplers: Vec<Sampler> = Vec::new();
        for (st, module) in [(0usize, &vm), (1, &fm)] {
            let slot = |gpu: &mut Vec<GpuBind>, group: u32, binding: u32, kind: BindKind| -> usize {
                match gpu.iter().position(|b| b.group == group && b.binding == binding) {
                    Some(i) => i,
                    None => { gpu.push(GpuBind { group, binding, kind }); gpu.len() - 1 }
                }
            };
            // (sampler binding, this stage's sampler global)
            let mut stage_samplers = Vec::new();
            let mut base = 0u32;
            for (h, g) in module.global_variables.iter() {
                let buffer = matches!(g.space, AddressSpace::Uniform | AddressSpace::Immediate | AddressSpace::Storage { .. });
                if buffer {
                    if let Some(b) = &g.binding {
                        let mut lane = base;
                        let mut map = Vec::new();
                        walk(module, g.ty, &mut lane, 0, &mut map);
                        let i = slot(&mut gpu, b.group, b.binding, BindKind::Buffer([Vec::new(), Vec::new()]));
                        if let BindKind::Buffer(maps) = &mut gpu[i].kind { maps[st] = map; }
                    }
                    base += crate_lanes(module, g.ty);
                    continue;
                }
                let Some(b) = &g.binding else { continue };
                let gi = h.index() as u32;
                match &module.types[g.ty].inner {
                    TypeInner::Image { dim, arrayed, class } => {
                        let kind = match (dim, arrayed, class) {
                            (ImageDimension::Cube, _, _) => SamplerKind::Cube,
                            (ImageDimension::D3, _, _) => SamplerKind::D3,
                            (_, _, ImageClass::Depth { .. }) => SamplerKind::D2Shadow,
                            (_, true, _) => SamplerKind::D2Array,
                            _ => SamplerKind::D2,
                        };
                        let i = slot(&mut gpu, b.group, b.binding, BindKind::Texture(samplers.len()));
                        let BindKind::Texture(si) = gpu[i].kind else { continue };
                        if si == samplers.len() {
                            samplers.push(Sampler { name: g.name.clone().unwrap_or_default(), kind, units: vec![0], vs_globals: Vec::new(), fs_globals: Vec::new() });
                        }
                        if st == 0 { samplers[si].vs_globals.push(gi) } else { samplers[si].fs_globals.push(gi) }
                    }
                    TypeInner::Sampler { .. } => {
                        let i = slot(&mut gpu, b.group, b.binding, BindKind::Sampler(Vec::new()));
                        stage_samplers.push((i, gi));
                    }
                    _ => {}
                }
            }
            // Which textures each sampler samples in this stage.
            let used = pairs(module);
            for (at, sg) in stage_samplers {
                for (img, _) in used.iter().filter(|(_, s)| *s == sg) {
                    let found = samplers.iter().position(|x| if st == 0 { x.vs_globals.contains(img) } else { x.fs_globals.contains(img) });
                    if let (Some(t), BindKind::Sampler(list)) = (found, &mut gpu[at].kind) {
                        if !list.contains(&t) { list.push(t); }
                    }
                }
            }
        }
        let mut attributes = Vec::new();
        for (io, _, lanes) in &vs.inputs {
            if let Io::Location(l) = io {
                let t = match lanes { 1 => FLOAT, 2 => FLOAT_VEC2, 3 => FLOAT_VEC3, _ => FLOAT_VEC4 };
                attributes.push((format!("loc{l}"), *l, t, 1));
            }
        }
        let mut p = Program::assemble(vs, fs, Vec::new(), samplers, attributes)?;
        p.zero_one = true;
        p.gpu = gpu;
        Ok(p)
    }

    /// Copies a bound buffer's bytes into the uniform memory of binding `group`/`binding`.
    pub fn bind_buffer(&mut self, group: u32, binding: u32, bytes: &[u8]) {
        let Some(b) = self.gpu.iter().find(|b| b.group == group && b.binding == binding) else { return };
        let BindKind::Buffer(maps) = &b.kind else { return };
        for (map, mem) in maps.iter().zip([&mut self.vs_uniforms, &mut self.fs_uniforms]) {
            for &(lane, byte) in map {
                let at = byte as usize;
                let v = bytes.get(at..at + 4).map_or(0, |x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]));
                if let Some(slot) = mem.get_mut(lane as usize) { *slot = v; }
            }
        }
    }
}
