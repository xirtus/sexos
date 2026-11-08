// SPDX-License-Identifier: GPL-3.0-only
//! Runs naga shader modules. Every value is a run of 32-bit lanes (floats, ints and bools
//! by bits; vectors, matrices by column, arrays and structs flattened in order). Shaders
//! can't recurse, so each function gets fixed regions: one slot per expression in `regs`,
//! and its locals in the invocation's private memory. A pointer is two lanes: its space
//! (private or uniform) and its lane offset.
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use naga::{
    ArraySize, BinaryOperator as B, Binding, BuiltIn, Expression as E, Function, Handle, Literal, MathFunction as M,
    Module, RelationalFunction, SampleLevel, ScalarKind, Statement as S, SwitchValue, Type, TypeInner, UnaryOperator,
};
use naga::valid::{FunctionInfo, ModuleInfo};

const SPACE_PRIVATE: u32 = 0;
const SPACE_UNIFORM: u32 = 1;
/// Loops stop after this many iterations (a runaway shader must not hang the page).
const LOOP_LIMIT: u32 = 1 << 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum K {
    F,
    I,
    U,
    B,
}

/// What an expression's value looks like: lane kind, shape and size.
#[derive(Clone, Copy, Debug)]
struct Ty {
    kind: K,
    /// Matrix columns, else 1.
    cols: u8,
    /// Vector size or matrix rows, 1 for scalars.
    rows: u8,
    lanes: u32,
    ptr: bool,
}

const fn kind(k: ScalarKind) -> K {
    match k {
        ScalarKind::Float | ScalarKind::AbstractFloat => K::F,
        ScalarKind::Sint | ScalarKind::AbstractInt => K::I,
        ScalarKind::Uint => K::U,
        ScalarKind::Bool => K::B,
    }
}

/// Lane counts and struct member offsets of the module's types.
pub struct Layout {
    lanes: Vec<u32>,
    members: BTreeMap<usize, Vec<u32>>,
}

impl Layout {
    fn new(module: &Module) -> Self {
        let mut lanes = Vec::with_capacity(module.types.len());
        let mut members = BTreeMap::new();
        for (h, t) in module.types.iter() {
            let n = match &t.inner {
                TypeInner::Struct { members: ms, .. } => {
                    let mut offs = Vec::with_capacity(ms.len());
                    let mut at = 0;
                    for m in ms {
                        offs.push(at);
                        at += lanes[m.ty.index()];
                    }
                    members.insert(h.index(), offs);
                    at
                }
                TypeInner::Array { base, size, .. } | TypeInner::BindingArray { base, size } => {
                    let n = match size { ArraySize::Constant(n) => n.get(), _ => 1 };
                    lanes[base.index()] * n
                }
                inner => inner_lanes(inner),
            };
            lanes.push(n);
        }
        Self { lanes, members }
    }
    pub fn of(&self, ty: Handle<Type>) -> u32 { self.lanes[ty.index()] }
    pub fn member(&self, ty: Handle<Type>, i: u32) -> u32 { self.members.get(&ty.index()).and_then(|m| m.get(i as usize)).copied().unwrap_or(0) }
}

fn inner_lanes(inner: &TypeInner) -> u32 {
    match inner {
        TypeInner::Scalar(_) | TypeInner::Atomic(_) => 1,
        TypeInner::Vector { size, .. } => *size as u32,
        TypeInner::Matrix { columns, rows, .. } => *columns as u32 * *rows as u32,
        TypeInner::Pointer { .. } | TypeInner::ValuePointer { .. } => 2,
        _ => 1,
    }
}

fn ty_of(module: &Module, layout: &Layout, inner: &TypeInner) -> Ty {
    match inner {
        TypeInner::Scalar(s) | TypeInner::Atomic(s) => Ty { kind: kind(s.kind), cols: 1, rows: 1, lanes: 1, ptr: false },
        TypeInner::Vector { size, scalar } => Ty { kind: kind(scalar.kind), cols: 1, rows: *size as u8, lanes: *size as u32, ptr: false },
        TypeInner::Matrix { columns, rows, scalar } => {
            Ty { kind: kind(scalar.kind), cols: *columns as u8, rows: *rows as u8, lanes: *columns as u32 * *rows as u32, ptr: false }
        }
        TypeInner::Pointer { .. } | TypeInner::ValuePointer { .. } => Ty { kind: K::U, cols: 1, rows: 1, lanes: 2, ptr: true },
        TypeInner::Array { base, size, .. } => {
            let n = match size { ArraySize::Constant(n) => n.get(), _ => 1 };
            let e = ty_of(module, layout, &module.types[*base].inner);
            Ty { kind: e.kind, cols: 1, rows: 1, lanes: layout.of(*base) * n, ptr: false }
        }
        TypeInner::Struct { members, .. } => {
            let lanes = members.iter().map(|m| layout.of(m.ty)).sum();
            Ty { kind: K::F, cols: 1, rows: 1, lanes, ptr: false }
        }
        _ => Ty { kind: K::U, cols: 1, rows: 1, lanes: 1, ptr: false },
    }
}

/// Element lanes and count when indexing a value or pointee of type `inner`.
fn index_shape(module: &Module, layout: &Layout, inner: &TypeInner) -> (u32, u32) {
    match inner {
        TypeInner::Vector { size, .. } => (1, *size as u32),
        TypeInner::Matrix { columns, rows, .. } => (*rows as u32, *columns as u32),
        TypeInner::Array { base, size, .. } | TypeInner::BindingArray { base, size } => {
            (layout.of(*base), match size { ArraySize::Constant(n) => n.get(), _ => u32::MAX })
        }
        TypeInner::Pointer { base, .. } => index_shape(module, layout, &module.types[*base].inner),
        TypeInner::ValuePointer { size: Some(size), .. } => (1, *size as u32),
        _ => (1, 1),
    }
}

/// One function's fixed regions.
#[derive(Clone, Copy)]
enum Src {
    Globals,
    Entry,
    Func(Handle<Function>),
}

struct Code {
    src: Src,
    slot: Vec<u32>,
    ty: Vec<Ty>,
    /// Expressions not covered by any `Emit`, evaluated on entry (arguments, pointers).
    on_entry: Vec<Handle<naga::Expression>>,
    /// Constant expressions evaluated once when the stage is built.
    once: Vec<Handle<naga::Expression>>,
    local_off: Vec<u32>,
    local_init: Vec<(u32, u32, Option<Handle<naga::Expression>>)>,
    arg_off: Vec<u32>,
    arg_lanes: Vec<u32>,
    ret_off: u32,
}

/// A sampled texture lookup, answered by the draw call.
pub trait Textures {
    /// Samples texture `unit_of_global` at `coord` (s, t, r/layer); `lod` is an explicit
    /// level or bias; `depth_ref` compares for shadow samplers.
    fn sample(&self, global: u32, coord: [f32; 4], lod: f32, depth_ref: Option<f32>) -> [f32; 4];
}

/// Stage inputs and outputs, by binding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Io {
    Location(u32),
    Position,
    PointSize,
    FragDepth,
    FrontFacing,
    PointCoord,
    VertexIndex,
    InstanceIndex,
    Other,
}

fn io(b: &Binding) -> Io {
    match b {
        Binding::Location { location, .. } => Io::Location(*location),
        Binding::BuiltIn(BuiltIn::Position { .. }) => Io::Position,
        Binding::BuiltIn(BuiltIn::PointSize) => Io::PointSize,
        Binding::BuiltIn(BuiltIn::FragDepth) => Io::FragDepth,
        Binding::BuiltIn(BuiltIn::FrontFacing) => Io::FrontFacing,
        Binding::BuiltIn(BuiltIn::PointCoord) => Io::PointCoord,
        Binding::BuiltIn(BuiltIn::VertexIndex) => Io::VertexIndex,
        Binding::BuiltIn(BuiltIn::InstanceIndex) => Io::InstanceIndex,
        _ => Io::Other,
    }
}

/// A uniform the program can set: its lane offset in the uniform memory, and its shape.
#[derive(Clone, Copy, Debug)]
pub struct UniformSlot {
    pub offset: u32,
    pub kind: K,
    pub cols: u8,
    pub rows: u8,
    /// Array length (1 for a single value).
    pub count: u32,
    /// Lanes per array element.
    pub stride: u32,
}

/// One compiled stage.
pub struct StageCode {
    module: Box<Module>,
    info: Box<ModuleInfo>,
    layout: Layout,
    funcs: Vec<Code>,
    entry: Code,
    globals: Code,
    regs_len: u32,
    private_len: u32,
    private_init: Vec<(u32, u32, Option<Handle<naga::Expression>>)>,
    global_off: Vec<(u32, u32)>,
    pub uniform_len: u32,
    /// Inputs: binding, lane offset in the entry's argument area, lanes.
    pub inputs: Vec<(Io, u32, u32)>,
    /// Outputs: binding, lane offset in the result, lanes.
    pub outputs: Vec<(Io, u32, u32)>,
    pub out_lanes: u32,
    /// Texture globals by name (`<sampler>_sxt`).
    pub textures: Vec<(String, u32)>,
    consts: Vec<u32>,
}

/// Per-invocation memory for one stage.
pub struct Machine {
    regs: Vec<u32>,
    mem: Vec<u32>,
    pub out: Vec<u32>,
    killed: bool,
}

enum Flow {
    Next,
    Break,
    Continue,
    Return,
    Kill,
}

impl StageCode {
    pub fn new(module: Module, info: ModuleInfo) -> Result<Self, String> {
        let layout = Layout::new(&module);
        let mut regs_len = 0u32;
        let mut private_len = 0u32;
        let mut uniform_len = 0u32;
        let mut global_off = Vec::new();
        let mut private_init = Vec::new();
        let mut textures = Vec::new();
        for (h, g) in module.global_variables.iter() {
            let n = layout.of(g.ty);
            match g.space {
                naga::AddressSpace::Uniform | naga::AddressSpace::Immediate => { global_off.push((SPACE_UNIFORM, uniform_len)); uniform_len += n; }
                naga::AddressSpace::Handle => {
                    global_off.push((u32::MAX, h.index() as u32));
                    if let Some(name) = &g.name { textures.push((name.clone(), h.index() as u32)); }
                }
                _ => { global_off.push((SPACE_PRIVATE, private_len)); private_init.push((private_len, n, g.init)); private_len += n; }
            }
        }
        // Global (constant) expressions.
        let ginfo: Vec<Ty> = module.global_expressions.iter().map(|(h, _)| ty_of(&module, &layout, info[h].inner_with(&module.types))).collect();
        let globals = Self::code_for(Src::Globals, &layout, &module.global_expressions, None, &ginfo, &mut regs_len, &mut private_len);
        let mut funcs = Vec::with_capacity(module.functions.len());
        for (h, f) in module.functions.iter() {
            let fi = &info[h];
            let tys = Self::expr_types(&module, &layout, f, fi);
            funcs.push(Self::code_for(Src::Func(h), &layout, &f.expressions, Some(f), &tys, &mut regs_len, &mut private_len));
        }
        let ep = module.entry_points.first().ok_or("no entry point")?;
        let fi = info.get_entry_point(0);
        let tys = Self::expr_types(&module, &layout, &ep.function, fi);
        let entry = Self::code_for(Src::Entry, &layout, &ep.function.expressions, Some(&ep.function), &tys, &mut regs_len, &mut private_len);
        let mut inputs = Vec::new();
        for (i, a) in ep.function.arguments.iter().enumerate() {
            match &a.binding {
                Some(b) => inputs.push((io(b), entry.arg_off[i], layout.of(a.ty))),
                None => {
                    if let TypeInner::Struct { members, .. } = &module.types[a.ty].inner {
                        for (j, m) in members.iter().enumerate() {
                            if let Some(b) = &m.binding { inputs.push((io(b), entry.arg_off[i] + layout.member(a.ty, j as u32), layout.of(m.ty))); }
                        }
                    }
                }
            }
        }
        let mut outputs = Vec::new();
        let mut out_lanes = 0;
        if let Some(r) = &ep.function.result {
            out_lanes = layout.of(r.ty);
            match &r.binding {
                Some(b) => outputs.push((io(b), 0, out_lanes)),
                None => {
                    if let TypeInner::Struct { members, .. } = &module.types[r.ty].inner {
                        for (j, m) in members.iter().enumerate() {
                            if let Some(b) = &m.binding { outputs.push((io(b), layout.member(r.ty, j as u32), layout.of(m.ty))); }
                        }
                    }
                }
            }
        }
        let mut st = Self {
            module: Box::new(module), info: Box::new(info), layout, funcs, entry, globals, regs_len, private_len, private_init,
            global_off, uniform_len, inputs, outputs, out_lanes, textures, consts: Vec::new(),
        };
        // Constant expressions, once.
        let mut m = Machine { regs: vec![0; st.regs_len as usize], mem: vec![0; st.private_len as usize], out: Vec::new(), killed: false };
        let empty: [u32; 0] = [];
        let no_tex = NoTextures;
        for h in st.globals.once.clone() { st.eval(&mut m, &st.globals, &st.module.global_expressions, h, &empty, &no_tex); }
        for i in 0..st.funcs.len() {
            let (fh, _) = st.module.functions.iter().nth(i).unwrap();
            let once = st.funcs[i].once.clone();
            for h in once { st.eval(&mut m, &st.funcs[i], &st.module.functions[fh].expressions, h, &empty, &no_tex); }
        }
        for h in st.entry.once.clone() { st.eval(&mut m, &st.entry, &st.module.entry_points[0].function.expressions, h, &empty, &no_tex); }
        st.consts = m.regs;
        Ok(st)
    }

    fn expr_types(module: &Module, layout: &Layout, f: &Function, fi: &FunctionInfo) -> Vec<Ty> {
        f.expressions.iter().map(|(h, _)| ty_of(module, layout, fi[h].ty.inner_with(&module.types))).collect()
    }

    fn code_for(
        src: Src, layout: &Layout, exprs: &naga::Arena<naga::Expression>, f: Option<&Function>, tys: &[Ty],
        regs_len: &mut u32, private_len: &mut u32,
    ) -> Code {
        let mut slot = Vec::with_capacity(exprs.len());
        for t in tys { slot.push(*regs_len); *regs_len += t.lanes.max(1); }
        let mut emitted = vec![false; exprs.len()];
        let mut results = vec![false; exprs.len()];
        if let Some(f) = f { mark(&f.body, &mut emitted, &mut results); }
        let mut once = Vec::new();
        let mut on_entry = Vec::new();
        let mut constant = vec![false; exprs.len()];
        for (h, e) in exprs.iter() {
            let i = h.index();
            if f.is_some() && (emitted[i] || results[i]) { continue; }
            let c = match e {
                E::Literal(_) | E::Constant(_) | E::ZeroValue(_) | E::GlobalVariable(_) | E::LocalVariable(_) => true,
                _ => operands(e).is_some_and(|ops| ops.iter().all(|o| constant[o.index()])),
            };
            constant[i] = c;
            if c { once.push(h) } else { on_entry.push(h) }
        }
        let (mut local_off, mut local_init, mut arg_off, mut arg_lanes, mut ret_off) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), 0);
        if let Some(f) = f {
            for (_, l) in f.local_variables.iter() {
                let n = layout.of(l.ty);
                local_off.push(*private_len);
                local_init.push((*private_len, n, l.init));
                *private_len += n;
            }
            for a in &f.arguments {
                let n = layout.of(a.ty);
                arg_off.push(*regs_len);
                arg_lanes.push(n);
                *regs_len += n;
            }
            if let Some(r) = &f.result { ret_off = *regs_len; *regs_len += layout.of(r.ty); }
        }
        Code { src, slot, ty: tys.to_vec(), on_entry, once, local_off, local_init, arg_off, arg_lanes, ret_off }
    }

    pub fn machine(&self) -> Machine {
        Machine { regs: self.consts.clone(), mem: vec![0; self.private_len as usize], out: vec![0; self.out_lanes as usize], killed: false }
    }

    /// Runs the entry point. `input` fills each input binding's lanes; outputs land in `m.out`.
    /// Returns false if the invocation was discarded.
    pub fn run(&self, m: &mut Machine, uniforms: &[u32], tex: &dyn Textures, input: &mut dyn FnMut(Io, &mut [u32])) -> bool {
        m.killed = false;
        for &(off, n, init) in &self.private_init {
            match init {
                Some(h) => { let s = self.globals.slot[h.index()] as usize; let (r, mm) = (&m.regs, &mut m.mem); mm[off as usize..(off + n) as usize].copy_from_slice(&r[s..s + n as usize]); }
                None => m.mem[off as usize..(off + n) as usize].fill(0),
            }
        }
        for &(io, off, n) in &self.inputs { input(io, &mut m.regs[off as usize..(off + n) as usize]); }
        let ep = &self.module.entry_points[0].function;
        let flow = self.call(m, &self.entry, ep, uniforms, tex);
        if matches!(flow, Flow::Kill) || m.killed { return false; }
        let n = self.out_lanes as usize;
        if n > 0 {
            let s = self.entry.ret_off as usize;
            let (r, o) = (&m.regs, &mut m.out);
            o[..n].copy_from_slice(&r[s..s + n]);
        }
        true
    }

    fn call(&self, m: &mut Machine, c: &Code, f: &Function, u: &[u32], tex: &dyn Textures) -> Flow {
        for &(off, n, init) in &c.local_init {
            match init {
                Some(h) => {
                    // Locals are initialised from a constant or an expression evaluated on entry.
                    if !c.once.contains(&h) && !c.on_entry.contains(&h) { m.mem[off as usize..(off + n) as usize].fill(0); continue; }
                    if c.on_entry.contains(&h) { self.eval(m, c, &f.expressions, h, u, tex); }
                    let s = c.slot[h.index()] as usize;
                    let (r, mm) = (&m.regs, &mut m.mem);
                    mm[off as usize..(off + n) as usize].copy_from_slice(&r[s..s + n as usize]);
                }
                None => m.mem[off as usize..(off + n) as usize].fill(0),
            }
        }
        for &h in &c.on_entry { self.eval(m, c, &f.expressions, h, u, tex); }
        match self.block(m, c, f, &f.body, u, tex) {
            Flow::Kill => Flow::Kill,
            _ => Flow::Return,
        }
    }

    fn block(&self, m: &mut Machine, c: &Code, f: &Function, b: &naga::Block, u: &[u32], tex: &dyn Textures) -> Flow {
        for st in b.iter() {
            let flow = match st {
                S::Emit(range) => { for h in range.clone() { self.eval(m, c, &f.expressions, h, u, tex); } Flow::Next }
                S::Block(inner) => self.block(m, c, f, inner, u, tex),
                S::If { condition, accept, reject } => {
                    if m.regs[c.slot[condition.index()] as usize] != 0 { self.block(m, c, f, accept, u, tex) } else { self.block(m, c, f, reject, u, tex) }
                }
                S::Switch { selector, cases } => {
                    let v = m.regs[c.slot[selector.index()] as usize];
                    let mut matched = false;
                    let mut flow = Flow::Next;
                    for pass in 0..2 {
                        for case in cases {
                            let hit = match case.value {
                                SwitchValue::I32(x) => pass == 0 && x as u32 == v,
                                SwitchValue::U32(x) => pass == 0 && x == v,
                                SwitchValue::Default => pass == 1,
                            };
                            if !(matched || hit) { continue; }
                            matched = true;
                            flow = self.block(m, c, f, &case.body, u, tex);
                            if !case.fall_through || !matches!(flow, Flow::Next) { break; }
                        }
                        if matched { break; }
                    }
                    match flow { Flow::Break => Flow::Next, other => other }
                }
                S::Loop { body, continuing, break_if } => {
                    let mut n = 0;
                    loop {
                        n += 1;
                        if n > LOOP_LIMIT { break Flow::Next; }
                        match self.block(m, c, f, body, u, tex) {
                            Flow::Break => break Flow::Next,
                            Flow::Return => break Flow::Return,
                            Flow::Kill => break Flow::Kill,
                            Flow::Next | Flow::Continue => {}
                        }
                        match self.block(m, c, f, continuing, u, tex) {
                            Flow::Break => break Flow::Next,
                            Flow::Return => break Flow::Return,
                            Flow::Kill => break Flow::Kill,
                            _ => {}
                        }
                        if let Some(h) = break_if { if m.regs[c.slot[h.index()] as usize] != 0 { break Flow::Next; } }
                    }
                }
                S::Break => Flow::Break,
                S::Continue => Flow::Continue,
                S::Return { value } => {
                    if let Some(v) = value {
                        let n = c.ty[v.index()].lanes as usize;
                        let s = c.slot[v.index()] as usize;
                        m.regs.copy_within(s..s + n, c.ret_off as usize);
                    }
                    Flow::Return
                }
                S::Kill => { m.killed = true; Flow::Kill }
                S::Store { pointer, value } => {
                    let p = c.slot[pointer.index()] as usize;
                    let (space, off) = (m.regs[p], m.regs[p + 1] as usize);
                    let n = c.ty[value.index()].lanes as usize;
                    let s = c.slot[value.index()] as usize;
                    if space == SPACE_PRIVATE && off + n <= m.mem.len() {
                        let (r, mm) = (&m.regs, &mut m.mem);
                        mm[off..off + n].copy_from_slice(&r[s..s + n]);
                    }
                    Flow::Next
                }
                S::Call { function, arguments, result } => {
                    let callee = &self.funcs[function.index()];
                    for (i, a) in arguments.iter().enumerate() {
                        let n = callee.arg_lanes[i] as usize;
                        let s = c.slot[a.index()] as usize;
                        m.regs.copy_within(s..s + n, callee.arg_off[i] as usize);
                    }
                    let cf = &self.module.functions[*function];
                    if let Flow::Kill = self.call(m, callee, cf, u, tex) { Flow::Kill } else {
                        if let Some(r) = result {
                            let n = c.ty[r.index()].lanes as usize;
                            m.regs.copy_within(callee.ret_off as usize..callee.ret_off as usize + n, c.slot[r.index()] as usize);
                        }
                        Flow::Next
                    }
                }
                _ => Flow::Next,
            };
            match flow { Flow::Next => {} other => return other }
        }
        Flow::Next
    }

    #[allow(clippy::too_many_lines)]
    fn eval(&self, m: &mut Machine, c: &Code, exprs: &naga::Arena<naga::Expression>, h: Handle<naga::Expression>, u: &[u32], tex: &dyn Textures) {
        let i = h.index();
        let dst = c.slot[i] as usize;
        let t = c.ty[i];
        let n = t.lanes as usize;
        let module = &*self.module;
        match &exprs[h] {
            E::Literal(l) => m.regs[dst] = match *l {
                Literal::F32(v) => v.to_bits(),
                Literal::F64(v) | Literal::AbstractFloat(v) => (v as f32).to_bits(),
                Literal::I32(v) => v as u32,
                Literal::U32(v) => v,
                Literal::I64(v) | Literal::AbstractInt(v) => v as i32 as u32,
                Literal::U64(v) => v as u32,
                Literal::Bool(b) => b as u32,
                _ => 0,
            },
            E::Constant(k) => {
                let g = module.constants[*k].init;
                let s = self.globals.slot[g.index()] as usize;
                m.regs.copy_within(s..s + n, dst);
            }
            E::ZeroValue(_) => m.regs[dst..dst + n].fill(0),
            E::Compose { components, .. } => {
                let mut at = dst;
                for &comp in components {
                    let s = c.slot[comp.index()] as usize;
                    let k = c.ty[comp.index()].lanes as usize;
                    m.regs.copy_within(s..s + k, at);
                    at += k;
                }
            }
            E::Splat { value, .. } => {
                let v = m.regs[c.slot[value.index()] as usize];
                m.regs[dst..dst + n].fill(v);
            }
            E::Swizzle { vector, pattern, .. } => {
                let s = c.slot[vector.index()] as usize;
                let mut tmp = [0u32; 4];
                for (j, p) in pattern.iter().take(n).enumerate() { tmp[j] = m.regs[s + *p as usize]; }
                m.regs[dst..dst + n].copy_from_slice(&tmp[..n]);
            }
            E::FunctionArgument(a) => {
                let s = c.arg_off[*a as usize] as usize;
                m.regs.copy_within(s..s + n, dst);
            }
            E::GlobalVariable(g) => {
                let (space, off) = self.global_off[g.index()];
                if space == u32::MAX { m.regs[dst] = off; } else { m.regs[dst] = space; m.regs[dst + 1] = off; }
            }
            E::LocalVariable(l) => { m.regs[dst] = SPACE_PRIVATE; m.regs[dst + 1] = c.local_off[l.index()]; }
            E::Load { pointer } => {
                let p = c.slot[pointer.index()] as usize;
                let (space, off) = (m.regs[p], m.regs[p + 1] as usize);
                if space == SPACE_UNIFORM {
                    if off + n <= u.len() { m.regs[dst..dst + n].copy_from_slice(&u[off..off + n]); } else { m.regs[dst..dst + n].fill(0); }
                } else if off + n <= m.mem.len() {
                    let (mm, r) = (&m.mem, &mut m.regs);
                    r[dst..dst + n].copy_from_slice(&mm[off..off + n]);
                }
            }
            E::Access { base, index } => {
                let idx = m.regs[c.slot[index.index()] as usize] as i32;
                self.access(m, c, exprs, *base, idx.max(0) as u32, dst, n, i);
            }
            E::AccessIndex { base, index } => self.access(m, c, exprs, *base, *index, dst, n, i),
            E::Unary { op, expr } => {
                let s = c.slot[expr.index()] as usize;
                let k = c.ty[expr.index()].kind;
                for j in 0..n {
                    let v = m.regs[s + j];
                    m.regs[dst + j] = match (op, k) {
                        (UnaryOperator::Negate, K::F) => (-f32::from_bits(v)).to_bits(),
                        (UnaryOperator::Negate, _) => (v as i32).wrapping_neg() as u32,
                        (UnaryOperator::LogicalNot, _) => (v == 0) as u32,
                        (UnaryOperator::BitwiseNot, _) => !v,
                    };
                }
            }
            E::Binary { op, left, right } => self.binary(m, c, *op, *left, *right, dst, t),
            E::Select { condition, accept, reject } => {
                let cs = c.slot[condition.index()] as usize;
                let cn = c.ty[condition.index()].lanes as usize;
                let (a, r) = (c.slot[accept.index()] as usize, c.slot[reject.index()] as usize);
                for j in 0..n {
                    let pick = m.regs[cs + if cn == 1 { 0 } else { j }] != 0;
                    m.regs[dst + j] = if pick { m.regs[a + j] } else { m.regs[r + j] };
                }
            }
            E::Relational { fun, argument } => {
                let s = c.slot[argument.index()] as usize;
                let an = c.ty[argument.index()].lanes as usize;
                match fun {
                    RelationalFunction::All => m.regs[dst] = (0..an).all(|j| m.regs[s + j] != 0) as u32,
                    RelationalFunction::Any => m.regs[dst] = (0..an).any(|j| m.regs[s + j] != 0) as u32,
                    RelationalFunction::IsNan => for j in 0..n { m.regs[dst + j] = f32::from_bits(m.regs[s + j]).is_nan() as u32; },
                    RelationalFunction::IsInf => for j in 0..n { m.regs[dst + j] = f32::from_bits(m.regs[s + j]).is_infinite() as u32; },
                }
            }
            E::Math { fun, arg, arg1, arg2, .. } => self.math(m, c, *fun, *arg, *arg1, *arg2, dst, t),
            E::As { expr, kind: to, convert } => {
                let s = c.slot[expr.index()] as usize;
                let from = c.ty[expr.index()].kind;
                let to = kind(*to);
                for j in 0..n {
                    let v = m.regs[s + j];
                    m.regs[dst + j] = if convert.is_none() { v } else {
                        match (from, to) {
                            (K::F, K::I) => { let f = f32::from_bits(v); if f.is_nan() { 0 } else { f as i32 as u32 } }
                            (K::F, K::U) => { let f = f32::from_bits(v); if f.is_nan() { 0 } else { f as u32 } }
                            (K::F, K::B) => (f32::from_bits(v) != 0.0) as u32,
                            (K::I, K::F) => (v as i32 as f32).to_bits(),
                            (K::U, K::F) => (v as f32).to_bits(),
                            (K::B, K::F) => if v != 0 { 1.0f32.to_bits() } else { 0 },
                            (K::I | K::U, K::B) => (v != 0) as u32,
                            (K::B, _) => (v != 0) as u32,
                            _ => v,
                        }
                    };
                }
            }
            E::ImageSample { image, coordinate, array_index, level, depth_ref, .. } => {
                let g = m.regs[c.slot[image.index()] as usize];
                let cs = c.slot[coordinate.index()] as usize;
                let cn = c.ty[coordinate.index()].lanes as usize;
                let mut coord = [0f32; 4];
                for j in 0..cn.min(4) { coord[j] = f32::from_bits(m.regs[cs + j]); }
                if let Some(ai) = array_index {
                    let a = m.regs[c.slot[ai.index()] as usize];
                    coord[cn.min(3)] = if c.ty[ai.index()].kind == K::F { f32::from_bits(a) } else { a as i32 as f32 };
                }
                let lod = match level {
                    SampleLevel::Exact(e) | SampleLevel::Bias(e) => f32::from_bits(m.regs[c.slot[e.index()] as usize]),
                    _ => 0.0,
                };
                let dref = depth_ref.map(|d| f32::from_bits(m.regs[c.slot[d.index()] as usize]));
                let px = tex.sample(g, coord, lod, dref);
                if n == 1 { m.regs[dst] = px[0].to_bits(); } else { for j in 0..n.min(4) { m.regs[dst + j] = px[j].to_bits(); } }
            }
            E::Derivative { .. } => m.regs[dst..dst + n].fill(0),
            E::CallResult(_) | E::AtomicResult { .. } => {}
            E::ImageQuery { .. } | E::ImageLoad { .. } | E::ArrayLength(_) => m.regs[dst..dst + n].fill(0),
            _ => m.regs[dst..dst + n].fill(0),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn access(&self, m: &mut Machine, c: &Code, exprs: &naga::Arena<naga::Expression>, base: Handle<naga::Expression>, index: u32, dst: usize, n: usize, me: usize) {
        let module = &*self.module;
        let bt = c.ty[base.index()];
        let base_inner = self.inner_of(c, exprs, base);
        let (elem, count, member) = match base_inner {
            Some(TypeInner::Pointer { base: pt, .. }) => match &module.types[pt].inner {
                TypeInner::Struct { .. } => (0, u32::MAX, Some(self.layout.member(pt, index))),
                inner => { let (e, k) = index_shape(module, &self.layout, inner); (e, k, None) }
            },
            Some(TypeInner::ValuePointer { size, .. }) => (1, size.map_or(1, |s| s as u32), None),
            Some(inner @ TypeInner::Struct { .. }) => {
                let th = self.type_handle_of(c, exprs, base);
                let _ = inner;
                (0, u32::MAX, Some(th.map_or(0, |th| self.layout.member(th, index))))
            }
            Some(inner) => { let (e, k) = index_shape(module, &self.layout, &inner); (e, k, None) }
            None => (1, 1, None),
        };
        let off = match member { Some(o) => o, None => index.min(count.saturating_sub(1)) * elem };
        let bs = c.slot[base.index()] as usize;
        if bt.ptr {
            m.regs[dst] = m.regs[bs];
            m.regs[dst + 1] = m.regs[bs + 1] + off;
        } else {
            let o = off as usize;
            if o + n <= bt.lanes as usize { m.regs.copy_within(bs + o..bs + o + n, dst); } else { m.regs[dst..dst + n].fill(0); }
        }
        let _ = me;
    }

    fn inner_of(&self, c: &Code, exprs: &naga::Arena<naga::Expression>, h: Handle<naga::Expression>) -> Option<TypeInner> {
        let module = &*self.module;
        let res = self.resolution(c, exprs, h)?;
        Some(res.inner_with(&module.types).clone())
    }

    fn type_handle_of(&self, c: &Code, exprs: &naga::Arena<naga::Expression>, h: Handle<naga::Expression>) -> Option<Handle<Type>> {
        match self.resolution(c, exprs, h)? { naga::proc::TypeResolution::Handle(t) => Some(*t), _ => None }
    }

    fn resolution(&self, c: &Code, _: &naga::Arena<naga::Expression>, h: Handle<naga::Expression>) -> Option<&naga::proc::TypeResolution> {
        Some(match c.src {
            Src::Globals => &self.info[h],
            Src::Entry => &self.info.get_entry_point(0)[h].ty,
            Src::Func(f) => &self.info[f][h].ty,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn binary(&self, m: &mut Machine, c: &Code, op: B, l: Handle<naga::Expression>, r: Handle<naga::Expression>, dst: usize, t: Ty) {
        let (lt, rt) = (c.ty[l.index()], c.ty[r.index()]);
        let (ls, rs) = (c.slot[l.index()] as usize, c.slot[r.index()] as usize);
        // Matrix products.
        if op == B::Multiply && (lt.cols > 1 || rt.cols > 1) && lt.kind == K::F {
            let mut out = [0f32; 16];
            let f = |m: &Machine, s: usize, j: usize| f32::from_bits(m.regs[s + j]);
            if lt.cols > 1 && rt.cols > 1 {
                // mat(lc x lr) * mat(rc x rr), lc == rr: result rc columns of lr rows.
                let (lr, lc, rc) = (lt.rows as usize, lt.cols as usize, rt.cols as usize);
                for col in 0..rc { for row in 0..lr {
                    let mut s = 0.0;
                    for k in 0..lc { s += f(m, ls, k * lr + row) * f(m, rs, col * lc + k); }
                    out[col * lr + row] = s;
                } }
            } else if lt.cols > 1 && rt.lanes > 1 {
                // mat * vec
                let (lr, lc) = (lt.rows as usize, lt.cols as usize);
                for row in 0..lr { let mut s = 0.0; for k in 0..lc { s += f(m, ls, k * lr + row) * f(m, rs, k); } out[row] = s; }
            } else if rt.cols > 1 && lt.lanes > 1 {
                // vec * mat
                let (rr, rc) = (rt.rows as usize, rt.cols as usize);
                for col in 0..rc { let mut s = 0.0; for k in 0..rr { s += f(m, ls, k) * f(m, rs, col * rr + k); } out[col] = s; }
            } else {
                // matrix * scalar or scalar * matrix
                let (ms, ss, mn) = if lt.cols > 1 { (ls, rs, lt.lanes) } else { (rs, ls, rt.lanes) };
                let k = f(m, ss, 0);
                for j in 0..mn as usize { out[j] = f(m, ms, j) * k; }
            }
            for j in 0..t.lanes as usize { m.regs[dst + j] = out[j].to_bits(); }
            return;
        }
        let n = t.lanes as usize;
        let k = lt.kind;
        for j in 0..n {
            let a = m.regs[ls + if lt.lanes == 1 { 0 } else { j }];
            let b = m.regs[rs + if rt.lanes == 1 { 0 } else { j }];
            m.regs[dst + j] = match k {
                K::F => {
                    let (x, y) = (f32::from_bits(a), f32::from_bits(b));
                    match op {
                        B::Add => (x + y).to_bits(),
                        B::Subtract => (x - y).to_bits(),
                        B::Multiply => (x * y).to_bits(),
                        B::Divide => (x / y).to_bits(),
                        B::Modulo => libm::fmodf(x, y).to_bits(),
                        B::Equal => (x == y) as u32,
                        B::NotEqual => (x != y) as u32,
                        B::Less => (x < y) as u32,
                        B::LessEqual => (x <= y) as u32,
                        B::Greater => (x > y) as u32,
                        B::GreaterEqual => (x >= y) as u32,
                        _ => 0,
                    }
                }
                K::I => {
                    let (x, y) = (a as i32, b as i32);
                    match op {
                        B::Add => x.wrapping_add(y) as u32,
                        B::Subtract => x.wrapping_sub(y) as u32,
                        B::Multiply => x.wrapping_mul(y) as u32,
                        B::Divide => if y == 0 { 0 } else { x.wrapping_div(y) as u32 },
                        B::Modulo => if y == 0 { 0 } else { x.wrapping_rem(y) as u32 },
                        B::Equal => (x == y) as u32,
                        B::NotEqual => (x != y) as u32,
                        B::Less => (x < y) as u32,
                        B::LessEqual => (x <= y) as u32,
                        B::Greater => (x > y) as u32,
                        B::GreaterEqual => (x >= y) as u32,
                        B::And => (x & y) as u32,
                        B::InclusiveOr => (x | y) as u32,
                        B::ExclusiveOr => (x ^ y) as u32,
                        B::ShiftLeft => x.wrapping_shl(b) as u32,
                        B::ShiftRight => x.wrapping_shr(b) as u32,
                        B::LogicalAnd => (x != 0 && y != 0) as u32,
                        B::LogicalOr => (x != 0 || y != 0) as u32,
                    }
                }
                K::U | K::B => match op {
                    B::Add => a.wrapping_add(b),
                    B::Subtract => a.wrapping_sub(b),
                    B::Multiply => a.wrapping_mul(b),
                    B::Divide => if b == 0 { 0 } else { a / b },
                    B::Modulo => if b == 0 { 0 } else { a % b },
                    B::Equal => (a == b) as u32,
                    B::NotEqual => (a != b) as u32,
                    B::Less => (a < b) as u32,
                    B::LessEqual => (a <= b) as u32,
                    B::Greater => (a > b) as u32,
                    B::GreaterEqual => (a >= b) as u32,
                    B::And => a & b,
                    B::InclusiveOr => a | b,
                    B::ExclusiveOr => a ^ b,
                    B::ShiftLeft => a.wrapping_shl(b),
                    B::ShiftRight => a.wrapping_shr(b),
                    B::LogicalAnd => (a != 0 && b != 0) as u32,
                    B::LogicalOr => (a != 0 || b != 0) as u32,
                },
            };
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn math(&self, m: &mut Machine, c: &Code, fun: M, a: Handle<naga::Expression>, b: Option<Handle<naga::Expression>>, cc: Option<Handle<naga::Expression>>, dst: usize, t: Ty) {
        let at = c.ty[a.index()];
        let n = t.lanes as usize;
        let get = |m: &Machine, h: Option<Handle<naga::Expression>>, j: usize| -> f32 {
            match h {
                Some(h) => { let k = c.ty[h.index()].lanes as usize; f32::from_bits(m.regs[c.slot[h.index()] as usize + if k == 1 { 0 } else { j }]) }
                None => 0.0,
            }
        };
        let geta = |m: &Machine, j: usize| f32::from_bits(m.regs[c.slot[a.index()] as usize + if at.lanes == 1 { 0 } else { j }]);
        let an = at.lanes as usize;
        // Whole-vector functions first.
        match fun {
            M::Dot => {
                let mut s = 0.0;
                for j in 0..an { s += geta(m, j) * get(m, b, j); }
                m.regs[dst] = if at.kind == K::F { s.to_bits() } else {
                    let (sa, sb) = (c.slot[a.index()] as usize, c.slot[b.unwrap().index()] as usize);
                    (0..an).fold(0i32, |acc, j| acc.wrapping_add((m.regs[sa + j] as i32).wrapping_mul(m.regs[sb + j] as i32))) as u32
                };
                return;
            }
            M::Length => { let mut s = 0.0; for j in 0..an { let v = geta(m, j); s += v * v; } m.regs[dst] = libm::sqrtf(s).to_bits(); return; }
            M::Distance => { let mut s = 0.0; for j in 0..an { let v = geta(m, j) - get(m, b, j); s += v * v; } m.regs[dst] = libm::sqrtf(s).to_bits(); return; }
            M::Normalize => {
                let mut s = 0.0; for j in 0..an { let v = geta(m, j); s += v * v; }
                let k = 1.0 / libm::sqrtf(s);
                for j in 0..n { m.regs[dst + j] = (geta(m, j) * k).to_bits(); }
                return;
            }
            M::Cross => {
                let (x, y) = ([geta(m, 0), geta(m, 1), geta(m, 2)], [get(m, b, 0), get(m, b, 1), get(m, b, 2)]);
                let r = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
                for j in 0..3 { m.regs[dst + j] = r[j].to_bits(); }
                return;
            }
            M::Reflect => {
                let mut d = 0.0; for j in 0..an { d += get(m, b, j) * geta(m, j); }
                for j in 0..n { m.regs[dst + j] = (geta(m, j) - 2.0 * d * get(m, b, j)).to_bits(); }
                return;
            }
            M::Refract => {
                let eta = get(m, cc, 0);
                let mut d = 0.0; for j in 0..an { d += get(m, b, j) * geta(m, j); }
                let k = 1.0 - eta * eta * (1.0 - d * d);
                for j in 0..n {
                    m.regs[dst + j] = if k < 0.0 { 0 } else { (eta * geta(m, j) - (eta * d + libm::sqrtf(k)) * get(m, b, j)).to_bits() };
                }
                return;
            }
            M::FaceForward => {
                let mut d = 0.0; for j in 0..an { d += get(m, cc, j) * get(m, b, j); }
                for j in 0..n { let v = geta(m, j); m.regs[dst + j] = (if d < 0.0 { v } else { -v }).to_bits(); }
                return;
            }
            M::Outer => {
                let bn = c.ty[b.unwrap().index()].lanes as usize;
                for col in 0..bn { for row in 0..an { m.regs[dst + col * an + row] = (geta(m, row) * get(m, b, col)).to_bits(); } }
                return;
            }
            M::Transpose => {
                let (r, cols) = (at.rows as usize, at.cols as usize);
                let s = c.slot[a.index()] as usize;
                let mut tmp = [0u32; 16];
                for col in 0..cols { for row in 0..r { tmp[row * cols + col] = m.regs[s + col * r + row]; } }
                m.regs[dst..dst + n].copy_from_slice(&tmp[..n]);
                return;
            }
            M::Determinant => {
                let s = c.slot[a.index()] as usize;
                let x: Vec<f32> = (0..an).map(|j| f32::from_bits(m.regs[s + j])).collect();
                m.regs[dst] = det(&x, at.cols as usize).to_bits();
                return;
            }
            M::Inverse => {
                let s = c.slot[a.index()] as usize;
                let x: Vec<f32> = (0..an).map(|j| f32::from_bits(m.regs[s + j])).collect();
                let inv = inverse(&x, at.cols as usize);
                for j in 0..n { m.regs[dst + j] = inv[j].to_bits(); }
                return;
            }
            _ => {}
        }
        for j in 0..n {
            let x = geta(m, j);
            let r: f32 = match fun {
                M::Abs if at.kind == K::I => { m.regs[dst + j] = (m.regs[c.slot[a.index()] as usize + j] as i32).wrapping_abs() as u32; continue; }
                M::Min | M::Max | M::Clamp if at.kind != K::F => {
                    let ai = |h: Handle<naga::Expression>, j: usize| { let k = c.ty[h.index()].lanes as usize; m.regs[c.slot[h.index()] as usize + if k == 1 { 0 } else { j }] };
                    let (va, vb) = (ai(a, j), ai(b.unwrap(), j));
                    let v = if at.kind == K::I {
                        let (x, y) = (va as i32, vb as i32);
                        (match fun { M::Min => x.min(y), M::Max => x.max(y), _ => { let hi = ai(cc.unwrap(), j) as i32; x.max(y).min(hi) } }) as u32
                    } else {
                        match fun { M::Min => va.min(vb), M::Max => va.max(vb), _ => { let hi = ai(cc.unwrap(), j); va.max(vb).min(hi) } }
                    };
                    m.regs[dst + j] = v;
                    continue;
                }
                M::Abs => libm::fabsf(x),
                M::Min => x.min(get(m, b, j)),
                M::Max => x.max(get(m, b, j)),
                M::Clamp => x.max(get(m, b, j)).min(get(m, cc, j)),
                M::Saturate => x.clamp(0.0, 1.0),
                M::Cos => libm::cosf(x),
                M::Cosh => libm::coshf(x),
                M::Sin => libm::sinf(x),
                M::Sinh => libm::sinhf(x),
                M::Tan => libm::tanf(x),
                M::Tanh => libm::tanhf(x),
                M::Acos => libm::acosf(x),
                M::Asin => libm::asinf(x),
                M::Atan => libm::atanf(x),
                M::Atan2 => libm::atan2f(x, get(m, b, j)),
                M::Asinh => libm::asinhf(x),
                M::Acosh => libm::acoshf(x),
                M::Atanh => libm::atanhf(x),
                M::Radians => x * (core::f32::consts::PI / 180.0),
                M::Degrees => x * (180.0 / core::f32::consts::PI),
                M::Ceil => libm::ceilf(x),
                M::Floor => libm::floorf(x),
                M::Round => libm::rintf(x),
                M::Fract => x - libm::floorf(x),
                M::Trunc => libm::truncf(x),
                M::Ldexp => libm::ldexpf(x, m.regs[c.slot[b.unwrap().index()] as usize + j] as i32),
                M::Exp => libm::expf(x),
                M::Exp2 => libm::exp2f(x),
                M::Log => libm::logf(x),
                M::Log2 => libm::log2f(x),
                M::Pow => libm::powf(x, get(m, b, j)),
                M::Sign => if at.kind == K::I {
                    m.regs[dst + j] = (m.regs[c.slot[a.index()] as usize + j] as i32).signum() as u32; continue;
                } else if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 },
                M::Fma => x * get(m, b, j) + get(m, cc, j),
                M::Mix => { let k = get(m, cc, j); x * (1.0 - k) + get(m, b, j) * k }
                M::Step => if get(m, b, j) < x { 0.0 } else { 1.0 },
                M::SmoothStep => {
                    let (e0, e1, v) = (x, get(m, b, j), get(m, cc, j));
                    let k = ((v - e0) / (e1 - e0)).clamp(0.0, 1.0);
                    k * k * (3.0 - 2.0 * k)
                }
                M::Sqrt => libm::sqrtf(x),
                M::InverseSqrt => 1.0 / libm::sqrtf(x),
                M::QuantizeToF16 => x,
                _ => 0.0,
            };
            m.regs[dst + j] = r.to_bits();
        }
    }
}

struct NoTextures;
impl Textures for NoTextures {
    fn sample(&self, _: u32, _: [f32; 4], _: f32, _: Option<f32>) -> [f32; 4] { [0.0; 4] }
}

fn det(x: &[f32], n: usize) -> f32 {
    match n {
        2 => x[0] * x[3] - x[2] * x[1],
        3 => x[0] * (x[4] * x[8] - x[7] * x[5]) - x[3] * (x[1] * x[8] - x[7] * x[2]) + x[6] * (x[1] * x[5] - x[4] * x[2]),
        _ => {
            let mut d = 0.0;
            for c in 0..4 {
                let mut sub = [0f32; 9];
                let mut k = 0;
                for cc in 0..4 { if cc == c { continue; } for r in 1..4 { sub[k] = x[cc * 4 + r]; k += 1; } }
                // sub is column-major 3x3 of the remaining columns, rows 1..4
                let s = det(&sub, 3);
                d += if c % 2 == 0 { x[c * 4] * s } else { -x[c * 4] * s };
            }
            d
        }
    }
}

fn inverse(x: &[f32], n: usize) -> Vec<f32> {
    // Gauss-Jordan on a row-major copy.
    let mut a = vec![0f32; n * 2 * n];
    for r in 0..n { for c in 0..n { a[r * 2 * n + c] = x[c * n + r]; } a[r * 2 * n + n + r] = 1.0; }
    for col in 0..n {
        let mut piv = col;
        for r in col + 1..n { if libm::fabsf(a[r * 2 * n + col]) > libm::fabsf(a[piv * 2 * n + col]) { piv = r; } }
        if a[piv * 2 * n + col] == 0.0 { return vec![0.0; n * n]; }
        if piv != col { for k in 0..2 * n { a.swap(piv * 2 * n + k, col * 2 * n + k); } }
        let p = a[col * 2 * n + col];
        for k in 0..2 * n { a[col * 2 * n + k] /= p; }
        for r in 0..n {
            if r == col { continue; }
            let f = a[r * 2 * n + col];
            if f != 0.0 { for k in 0..2 * n { a[r * 2 * n + k] -= f * a[col * 2 * n + k]; } }
        }
    }
    let mut out = vec![0f32; n * n];
    for r in 0..n { for c in 0..n { out[c * n + r] = a[r * 2 * n + n + c]; } }
    out
}

fn mark(b: &naga::Block, emitted: &mut [bool], results: &mut [bool]) {
    for st in b.iter() {
        match st {
            S::Emit(r) => for h in r.clone() { emitted[h.index()] = true; },
            S::Block(x) => mark(x, emitted, results),
            S::If { accept, reject, .. } => { mark(accept, emitted, results); mark(reject, emitted, results); }
            S::Switch { cases, .. } => for c in cases { mark(&c.body, emitted, results); },
            S::Loop { body, continuing, .. } => { mark(body, emitted, results); mark(continuing, emitted, results); }
            S::Call { result: Some(r), .. } => results[r.index()] = true,
            S::Atomic { result: Some(r), .. } => results[r.index()] = true,
            _ => {}
        }
    }
}

fn operands(e: &naga::Expression) -> Option<Vec<Handle<naga::Expression>>> {
    Some(match e {
        E::Compose { components, .. } => components.clone(),
        E::Access { base, index } => vec![*base, *index],
        E::AccessIndex { base, .. } => vec![*base],
        E::Splat { value, .. } => vec![*value],
        E::Swizzle { vector, .. } => vec![*vector],
        E::Load { pointer } => vec![*pointer],
        E::Unary { expr, .. } | E::As { expr, .. } => vec![*expr],
        E::Binary { left, right, .. } => vec![*left, *right],
        E::Select { condition, accept, reject } => vec![*condition, *accept, *reject],
        E::Relational { argument, .. } => vec![*argument],
        E::Math { arg, arg1, arg2, arg3, .. } => {
            let mut v = vec![*arg];
            v.extend([*arg1, *arg2, *arg3].into_iter().flatten());
            v
        }
        // Anything else depends on the invocation.
        _ => return None,
    })
}
