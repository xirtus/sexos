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
    /// For `Access`/`AccessIndex`, by expression: element lanes and count, or for a
    /// struct member (or constant index) the lane offset itself as `(offset, u32::MAX)`
    /// with `fixed` set. Worked out once, so a run never looks at types.
    acc: Vec<Acc>,
    /// By expression: in `once`, in `on_entry`.
    is_once: Vec<bool>,
    is_on_entry: Vec<bool>,
    /// One prepared operation per expression, run in place of `eval` for the common
    /// cases (slots, sizes and shapes resolved once).
    ops: Vec<Op>,
}

/// Copies `n` lanes within `r`; most values are 1 to 4 lanes, too few for `memmove`.
#[inline(always)]
fn lanes(r: &mut [u32], src: usize, dst: usize, n: usize) {
    if n == 1 { r[dst] = r[src]; } else { r.copy_within(src..src + n, dst); }
}

/// One step of a stage's flat program: the entry point with every call inlined (shaders
/// can't recurse) and control flow turned into jumps.
#[derive(Clone, Debug)]
enum Step {
    /// A prepared expression.
    Op(Op),
    /// An expression `eval` runs: code (0.. functions, then the entry) and expression.
    Eval { code: u32, h: Handle<naga::Expression> },
    Store { ptr: u32, val: u32, n: u32 },
    /// Copies registers into private memory (a local's initial value).
    ToMem { off: u32, src: u32, n: u32 },
    Zero { off: u32, n: u32 },
    Jump(u32),
    JumpIfZero { cond: u32, to: u32 },
    JumpIfNonZero { cond: u32, to: u32 },
    JumpIfEq { sel: u32, val: u32, to: u32 },
    /// Starts loop counter `ctr`; `LoopBack` counts an iteration and jumps back unless
    /// the loop ran `LOOP_LIMIT` times.
    LoopEnter(u32),
    LoopBack { ctr: u32, to: u32 },
    Kill,
    Nop,
}

/// Jumps waiting for a label while a body is lowered.
#[derive(Default)]
struct Lowering {
    steps: Vec<Step>,
    loops: u32,
    /// Innermost last: jumps to patch at the end of the enclosing loop or switch.
    breaks: Vec<Vec<usize>>,
    /// Innermost loop last: jumps to patch at its continuing block.
    continues: Vec<Vec<usize>>,
    /// Jumps to patch at the end of the function being inlined.
    returns: Vec<Vec<usize>>,
}

impl Lowering {
    fn here(&self) -> u32 { self.steps.len() as u32 }
    fn jump(&mut self, s: Step) -> usize { self.steps.push(s); self.steps.len() - 1 }
    fn patch(&mut self, at: &[usize], to: u32) {
        for &i in at {
            match &mut self.steps[i] {
                Step::Jump(t) | Step::JumpIfZero { to: t, .. } | Step::JumpIfNonZero { to: t, .. } | Step::JumpIfEq { to: t, .. } => *t = to,
                _ => {}
            }
        }
    }
}

/// A prepared expression. `Generic` falls back to `eval`.
#[derive(Clone, Debug)]
enum Op {
    Generic,
    Copy { dst: u32, src: u32, n: u32 },
    Splat { dst: u32, src: u32, n: u32 },
    Swizzle { dst: u32, src: u32, pat: [u8; 4], n: u32 },
    Compose { dst: u32, parts: Box<[(u32, u32)]> },
    Ptr { dst: u32, space: u32, off: u32 },
    Load { dst: u32, ptr: u32, n: u32 },
    /// A fixed offset into a pointer (`ptr`) or a value of `base_lanes` lanes.
    Field { dst: u32, base: u32, off: u32, n: u32, ptr: bool, base_lanes: u32 },
    /// Float arithmetic or comparison, lane by lane; `sa`/`sb` broadcast a scalar side.
    FBin { op: B, dst: u32, a: u32, b: u32, n: u32, sa: bool, sb: bool },
    FMatVec { dst: u32, m: u32, v: u32, rows: u32, cols: u32 },
    Dot { dst: u32, a: u32, b: u32, n: u32 },
    Normalize { dst: u32, a: u32, n: u32 },
    FMax { dst: u32, a: u32, b: u32, n: u32, sb: bool },
    FMin { dst: u32, a: u32, b: u32, n: u32, sb: bool },
    Clamp { dst: u32, a: u32, lo: u32, hi: u32, n: u32, s: bool },
    Mix { dst: u32, a: u32, b: u32, t: u32, n: u32, st: bool },
}
#[derive(Clone, Copy, Default)]
struct Acc {
    elem: u32,
    count: u32,
    fixed: Option<u32>,
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
    /// The entry point as one flat program.
    steps: Vec<Step>,
    loop_count: u32,
    fhandles: Vec<Handle<Function>>,
}

/// Per-invocation memory for one stage.
pub struct Machine {
    regs: Vec<u32>,
    mem: Vec<u32>,
    pub out: Vec<u32>,
    killed: bool,
    loops: Vec<u32>,
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
            global_off, uniform_len, inputs, outputs, out_lanes, textures, consts: Vec::new(), steps: Vec::new(), loop_count: 0, fhandles: Vec::new(),
        };
        st.globals.acc = st.access_shapes(&st.globals, &st.module.global_expressions);
        for i in 0..st.funcs.len() {
            let (fh, _) = st.module.functions.iter().nth(i).unwrap();
            st.funcs[i].acc = st.access_shapes(&st.funcs[i], &st.module.functions[fh].expressions);
            st.funcs[i].ops = st.prepare(&st.funcs[i], &st.module.functions[fh].expressions);
        }
        st.entry.acc = st.access_shapes(&st.entry, &st.module.entry_points[0].function.expressions);
        st.entry.ops = st.prepare(&st.entry, &st.module.entry_points[0].function.expressions);
        st.fhandles = st.module.functions.iter().map(|(h, _)| h).collect();
        let mut lw = Lowering::default();
        st.lower_function(&mut lw, st.funcs.len());
        st.steps = lw.steps;
        // A jump to the very next step (a return at the end of a function) does nothing.
        for i in 0..st.steps.len() {
            if matches!(st.steps[i], Step::Jump(t) if t as usize == i + 1) { st.steps[i] = Step::Nop; }
        }
        st.loop_count = lw.loops;
        // Constant expressions, once.
        let mut m = Machine { regs: vec![0; st.regs_len as usize], mem: vec![0; st.private_len as usize], out: Vec::new(), killed: false, loops: Vec::new() };
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
        let mut is_once = vec![false; exprs.len()];
        for h in &once { is_once[h.index()] = true; }
        let mut is_on_entry = vec![false; exprs.len()];
        for h in &on_entry { is_on_entry[h.index()] = true; }
        Code { src, slot, ty: tys.to_vec(), on_entry, once, local_off, local_init, arg_off, arg_lanes, ret_off, acc: Vec::new(), ops: Vec::new(), is_once, is_on_entry }
    }

    /// The flat program's steps, for looking at.
    pub fn describe(&self) -> alloc::vec::Vec<alloc::string::String> { self.steps.iter().map(|s| alloc::format!("{s:?}")).collect() }

    pub fn machine(&self) -> Machine {
        Machine { regs: self.consts.clone(), mem: vec![0; self.private_len as usize], out: vec![0; self.out_lanes as usize], killed: false, loops: vec![0; self.loop_count as usize] }
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
        if !self.exec(m, uniforms, tex) { return false; }
        let n = self.out_lanes as usize;
        if n > 0 {
            let s = self.entry.ret_off as usize;
            let (r, o) = (&m.regs, &mut m.out);
            o[..n].copy_from_slice(&r[s..s + n]);
        }
        true
    }

    /// Code `i` (a function, or the entry after the functions) and its function.
    fn code(&self, i: usize) -> (&Code, &Function) {
        if i < self.funcs.len() {
            (&self.funcs[i], &self.module.functions[self.fhandles[i]])
        } else {
            (&self.entry, &self.module.entry_points[0].function)
        }
    }

    fn step_for(c: &Code, ci: usize, h: Handle<naga::Expression>) -> Step {
        match &c.ops[h.index()] {
            Op::Generic => Step::Eval { code: ci as u32, h },
            op => Step::Op(op.clone()),
        }
    }

    /// Inlines function `ci` (with its locals' initial values) into `lw`.
    fn lower_function(&self, lw: &mut Lowering, ci: usize) {
        let (c, f) = self.code(ci);
        for &(off, n, init) in &c.local_init {
            match init {
                Some(h) if c.is_once[h.index()] || c.is_on_entry[h.index()] => {
                    if c.is_on_entry[h.index()] { lw.steps.push(Self::step_for(c, ci, h)); }
                    lw.steps.push(Step::ToMem { off, src: c.slot[h.index()], n });
                }
                _ => lw.steps.push(Step::Zero { off, n }),
            }
        }
        for &h in &c.on_entry { lw.steps.push(Self::step_for(c, ci, h)); }
        // Loops and switches around a call don't reach into the callee.
        let (brk, cnt) = (core::mem::take(&mut lw.breaks), core::mem::take(&mut lw.continues));
        lw.returns.push(Vec::new());
        self.lower_block(lw, ci, &f.body);
        let rets = lw.returns.pop().unwrap();
        let end = lw.here();
        lw.patch(&rets, end);
        lw.breaks = brk;
        lw.continues = cnt;
    }

    fn lower_block(&self, lw: &mut Lowering, ci: usize, b: &naga::Block) {
        let (c, _) = self.code(ci);
        for st in b.iter() {
            match st {
                S::Emit(range) => for h in range.clone() { lw.steps.push(Self::step_for(c, ci, h)); },
                S::Block(inner) => self.lower_block(lw, ci, inner),
                S::If { condition, accept, reject } => {
                    let skip = lw.jump(Step::JumpIfZero { cond: c.slot[condition.index()], to: 0 });
                    self.lower_block(lw, ci, accept);
                    if reject.is_empty() {
                        let end = lw.here();
                        lw.patch(&[skip], end);
                    } else {
                        let over = lw.jump(Step::Jump(0));
                        let at = lw.here();
                        lw.patch(&[skip], at);
                        self.lower_block(lw, ci, reject);
                        let end = lw.here();
                        lw.patch(&[over], end);
                    }
                }
                S::Switch { selector, cases } => {
                    let sel = c.slot[selector.index()];
                    let mut tests = Vec::new();
                    for case in cases.iter() {
                        let val = match case.value { SwitchValue::I32(x) => x as u32, SwitchValue::U32(x) => x, SwitchValue::Default => continue };
                        tests.push(lw.jump(Step::JumpIfEq { sel, val, to: 0 }));
                    }
                    let to_default = lw.jump(Step::Jump(0));
                    lw.breaks.push(Vec::new());
                    let (mut t, mut default_at) = (0, None);
                    for case in cases.iter() {
                        let at = lw.here();
                        match case.value {
                            SwitchValue::Default => default_at = Some(at),
                            _ => { let i = tests[t]; lw.patch(&[i], at); t += 1; }
                        }
                        self.lower_block(lw, ci, &case.body);
                        if !case.fall_through {
                            let i = lw.jump(Step::Jump(0));
                            lw.breaks.last_mut().unwrap().push(i);
                        }
                    }
                    let end = lw.here();
                    lw.patch(&[to_default], default_at.unwrap_or(end));
                    let brk = lw.breaks.pop().unwrap();
                    lw.patch(&brk, end);
                }
                S::Loop { body, continuing, break_if } => {
                    let ctr = lw.loops;
                    lw.loops += 1;
                    lw.steps.push(Step::LoopEnter(ctr));
                    let start = lw.here();
                    lw.breaks.push(Vec::new());
                    lw.continues.push(Vec::new());
                    self.lower_block(lw, ci, body);
                    let cont = lw.here();
                    let conts = lw.continues.pop().unwrap();
                    lw.patch(&conts, cont);
                    self.lower_block(lw, ci, continuing);
                    if let Some(h) = break_if {
                        let i = lw.jump(Step::JumpIfNonZero { cond: c.slot[h.index()], to: 0 });
                        lw.breaks.last_mut().unwrap().push(i);
                    }
                    lw.steps.push(Step::LoopBack { ctr, to: start });
                    let end = lw.here();
                    let brk = lw.breaks.pop().unwrap();
                    lw.patch(&brk, end);
                }
                S::Break => {
                    let i = lw.jump(Step::Jump(0));
                    if let Some(b) = lw.breaks.last_mut() { b.push(i); }
                }
                S::Continue => {
                    let i = lw.jump(Step::Jump(0));
                    if let Some(b) = lw.continues.last_mut() { b.push(i); }
                }
                S::Return { value } => {
                    if let Some(v) = value {
                        lw.steps.push(Step::Op(Op::Copy { dst: c.ret_off, src: c.slot[v.index()], n: c.ty[v.index()].lanes }));
                    }
                    let i = lw.jump(Step::Jump(0));
                    lw.returns.last_mut().unwrap().push(i);
                }
                S::Kill => lw.steps.push(Step::Kill),
                S::Store { pointer, value } => {
                    lw.steps.push(Step::Store { ptr: c.slot[pointer.index()], val: c.slot[value.index()], n: c.ty[value.index()].lanes });
                }
                S::Call { function, arguments, result } => {
                    let callee = &self.funcs[function.index()];
                    for (i, a) in arguments.iter().enumerate() {
                        lw.steps.push(Step::Op(Op::Copy { dst: callee.arg_off[i], src: c.slot[a.index()], n: callee.arg_lanes[i] }));
                    }
                    self.lower_function(lw, function.index());
                    if let Some(r) = result {
                        lw.steps.push(Step::Op(Op::Copy { dst: c.slot[r.index()], src: callee.ret_off, n: c.ty[r.index()].lanes }));
                    }
                }
                _ => {}
            }
        }
    }

    /// Runs the flat program. False if the invocation was discarded.
    fn exec(&self, m: &mut Machine, u: &[u32], tex: &dyn Textures) -> bool {
        let steps = &self.steps[..];
        let mut pc = 0usize;
        while pc < steps.len() {
            match &steps[pc] {
                Step::Op(op) => self.run_op(m, op, u),
                &Step::Eval { code, h } => {
                    let (c, f) = self.code(code as usize);
                    self.eval(m, c, &f.expressions, h, u, tex);
                }
                &Step::Store { ptr, val, n } => {
                    let (space, off) = (m.regs[ptr as usize], m.regs[ptr as usize + 1] as usize);
                    let (s, n) = (val as usize, n as usize);
                    if space == SPACE_PRIVATE && off + n <= m.mem.len() {
                        if n == 1 { m.mem[off] = m.regs[s]; } else { m.mem[off..off + n].copy_from_slice(&m.regs[s..s + n]); }
                    }
                }
                &Step::ToMem { off, src, n } => {
                    let (o, s, n) = (off as usize, src as usize, n as usize);
                    m.mem[o..o + n].copy_from_slice(&m.regs[s..s + n]);
                }
                &Step::Zero { off, n } => m.mem[off as usize..(off + n) as usize].fill(0),
                &Step::Jump(to) => { pc = to as usize; continue; }
                &Step::JumpIfZero { cond, to } => if m.regs[cond as usize] == 0 { pc = to as usize; continue; },
                &Step::JumpIfNonZero { cond, to } => if m.regs[cond as usize] != 0 { pc = to as usize; continue; },
                &Step::JumpIfEq { sel, val, to } => if m.regs[sel as usize] == val { pc = to as usize; continue; },
                &Step::LoopEnter(ctr) => m.loops[ctr as usize] = 0,
                &Step::LoopBack { ctr, to } => {
                    let k = &mut m.loops[ctr as usize];
                    *k += 1;
                    if *k < LOOP_LIMIT { pc = to as usize; continue; }
                }
                Step::Kill => { m.killed = true; return false; }
                Step::Nop => {}
            }
            pc += 1;
        }
        true
    }

    /// Prepares one operation per expression of `c`.
    fn prepare(&self, c: &Code, exprs: &naga::Arena<naga::Expression>) -> Vec<Op> {
        let module = &*self.module;
        exprs.iter().map(|(h, e)| {
            let i = h.index();
            let dst = c.slot[i];
            let t = c.ty[i];
            let n = t.lanes;
            let sl = |x: Handle<naga::Expression>| c.slot[x.index()];
            let ty = |x: Handle<naga::Expression>| c.ty[x.index()];
            match e {
                E::Splat { value, .. } => Op::Splat { dst, src: sl(*value), n },
                E::Swizzle { vector, pattern, .. } if n <= 4 => {
                    let mut pat = [0u8; 4];
                    for (j, p) in pattern.iter().take(n as usize).enumerate() { pat[j] = *p as u8; }
                    Op::Swizzle { dst, src: sl(*vector), pat, n }
                }
                E::Compose { components, .. } => Op::Compose { dst, parts: components.iter().map(|x| (sl(*x), ty(*x).lanes)).collect() },
                E::FunctionArgument(a) => Op::Copy { dst, src: c.arg_off[*a as usize], n },
                E::Constant(k) => Op::Copy { dst, src: self.globals.slot[module.constants[*k].init.index()], n },
                E::GlobalVariable(g) => {
                    let (space, off) = self.global_off[g.index()];
                    if space == u32::MAX { Op::Generic } else { Op::Ptr { dst, space, off } }
                }
                E::LocalVariable(l) => Op::Ptr { dst, space: SPACE_PRIVATE, off: c.local_off[l.index()] },
                E::Load { pointer } => Op::Load { dst, ptr: sl(*pointer), n },
                E::AccessIndex { base, .. } => match c.acc[i].fixed {
                    Some(off) => Op::Field { dst, base: sl(*base), off, n, ptr: ty(*base).ptr, base_lanes: ty(*base).lanes },
                    None => Op::Generic,
                },
                E::Binary { op, left, right } => {
                    let (lt, rt) = (ty(*left), ty(*right));
                    let arith = matches!(op, B::Add | B::Subtract | B::Multiply | B::Divide | B::Less | B::LessEqual | B::Greater | B::GreaterEqual | B::Equal | B::NotEqual);
                    if lt.kind == K::F && rt.kind == K::F && arith && lt.cols == 1 && rt.cols == 1 {
                        Op::FBin { op: *op, dst, a: sl(*left), b: sl(*right), n, sa: lt.lanes == 1, sb: rt.lanes == 1 }
                    } else if *op == B::Multiply && lt.kind == K::F && lt.cols > 1 && rt.cols == 1 && rt.lanes > 1 {
                        Op::FMatVec { dst, m: sl(*left), v: sl(*right), rows: lt.rows as u32, cols: lt.cols as u32 }
                    } else { Op::Generic }
                }
                E::Math { fun, arg, arg1, arg2, .. } if ty(*arg).kind == K::F && ty(*arg).cols == 1 => match (fun, arg1, arg2) {
                    (M::Dot, Some(b), _) => Op::Dot { dst, a: sl(*arg), b: sl(*b), n: ty(*arg).lanes },
                    (M::Normalize, _, _) => Op::Normalize { dst, a: sl(*arg), n },
                    (M::Max, Some(b), _) if ty(*arg).lanes == n => Op::FMax { dst, a: sl(*arg), b: sl(*b), n, sb: ty(*b).lanes == 1 },
                    (M::Min, Some(b), _) if ty(*arg).lanes == n => Op::FMin { dst, a: sl(*arg), b: sl(*b), n, sb: ty(*b).lanes == 1 },
                    (M::Clamp, Some(lo), Some(hi)) if ty(*arg).lanes == n && ty(*lo).lanes == ty(*hi).lanes => {
                        Op::Clamp { dst, a: sl(*arg), lo: sl(*lo), hi: sl(*hi), n, s: ty(*lo).lanes == 1 && n > 1 }
                    }
                    (M::Mix, Some(b), Some(t)) if ty(*arg).lanes == n && ty(*b).lanes == n => {
                        Op::Mix { dst, a: sl(*arg), b: sl(*b), t: sl(*t), n, st: ty(*t).lanes == 1 && n > 1 }
                    }
                    _ => Op::Generic,
                },
                _ => Op::Generic,
            }
        }).collect()
    }

    /// Runs a prepared operation.
    #[inline(always)]
    fn run_op(&self, m: &mut Machine, op: &Op, u: &[u32]) {
        let r = &mut m.regs;
        let f = |r: &[u32], i: u32| f32::from_bits(r[i as usize]);
        match op {
            Op::Generic => {}
            &Op::Copy { dst, src, n } => lanes(r, src as usize, dst as usize, n as usize),
            &Op::Splat { dst, src, n } => { let v = r[src as usize]; r[dst as usize..(dst + n) as usize].fill(v); }
            &Op::Swizzle { dst, src, pat, n } => {
                let tmp = [r[src as usize + pat[0] as usize], r[src as usize + pat[1] as usize], r[src as usize + pat[2] as usize], r[src as usize + pat[3] as usize]];
                r[dst as usize..(dst + n) as usize].copy_from_slice(&tmp[..n as usize]);
            }
            Op::Compose { dst, parts } => {
                let mut at = *dst as usize;
                for &(s, k) in parts.iter() {
                    lanes(r, s as usize, at, k as usize);
                    at += k as usize;
                }
            }
            &Op::Ptr { dst, space, off } => { r[dst as usize] = space; r[dst as usize + 1] = off; }
            &Op::Load { dst, ptr, n } => {
                let (space, off) = (r[ptr as usize], r[ptr as usize + 1] as usize);
                let (d, n) = (dst as usize, n as usize);
                let from: &[u32] = if space == SPACE_UNIFORM { u } else { &m.mem };
                if off + n <= from.len() {
                    if n == 1 { r[d] = from[off]; } else { r[d..d + n].copy_from_slice(&from[off..off + n]); }
                } else if space == SPACE_UNIFORM {
                    r[d..d + n].fill(0);
                }
            }
            &Op::Field { dst, base, off, n, ptr, base_lanes } => {
                if ptr {
                    r[dst as usize] = r[base as usize];
                    r[dst as usize + 1] = r[base as usize + 1] + off;
                } else if off + n <= base_lanes {
                    let s = (base + off) as usize;
                    lanes(r, s, dst as usize, n as usize);
                } else {
                    r[dst as usize..(dst + n) as usize].fill(0);
                }
            }
            &Op::FBin { op, dst, a, b, n, sa, sb } => {
                for j in 0..n {
                    let x = f(r, if sa { a } else { a + j });
                    let y = f(r, if sb { b } else { b + j });
                    r[(dst + j) as usize] = match op {
                        B::Add => (x + y).to_bits(),
                        B::Subtract => (x - y).to_bits(),
                        B::Multiply => (x * y).to_bits(),
                        B::Divide => (x / y).to_bits(),
                        B::Less => (x < y) as u32,
                        B::LessEqual => (x <= y) as u32,
                        B::Greater => (x > y) as u32,
                        B::GreaterEqual => (x >= y) as u32,
                        B::Equal => (x == y) as u32,
                        _ => (x != y) as u32,
                    };
                }
            }
            &Op::FMatVec { dst, m: mt, v, rows, cols } => {
                let mut out = [0f32; 4];
                for row in 0..rows.min(4) {
                    let mut s = 0.0;
                    for k in 0..cols { s += f(r, mt + k * rows + row) * f(r, v + k); }
                    out[row as usize] = s;
                }
                for row in 0..rows.min(4) { r[(dst + row) as usize] = out[row as usize].to_bits(); }
            }
            &Op::Dot { dst, a, b, n } => {
                let mut s = 0.0;
                for j in 0..n { s += f(r, a + j) * f(r, b + j); }
                r[dst as usize] = s.to_bits();
            }
            &Op::Normalize { dst, a, n } => {
                let mut s = 0.0;
                for j in 0..n { let v = f(r, a + j); s += v * v; }
                let k = 1.0 / libm::sqrtf(s);
                for j in 0..n { r[(dst + j) as usize] = (f(r, a + j) * k).to_bits(); }
            }
            &Op::FMax { dst, a, b, n, sb } => for j in 0..n {
                let (x, y) = (f(r, a + j), f(r, if sb { b } else { b + j }));
                r[(dst + j) as usize] = x.max(y).to_bits();
            },
            &Op::FMin { dst, a, b, n, sb } => for j in 0..n {
                let (x, y) = (f(r, a + j), f(r, if sb { b } else { b + j }));
                r[(dst + j) as usize] = x.min(y).to_bits();
            },
            &Op::Clamp { dst, a, lo, hi, n, s } => for j in 0..n {
                let k = if s { 0 } else { j };
                let (x, l, h) = (f(r, a + j), f(r, lo + k), f(r, hi + k));
                r[(dst + j) as usize] = x.max(l).min(h).to_bits();
            },
            &Op::Mix { dst, a, b, t, n, st } => for j in 0..n {
                let (x, y, w) = (f(r, a + j), f(r, b + j), f(r, if st { t } else { t + j }));
                r[(dst + j) as usize] = (x * (1.0 - w) + y * w).to_bits();
            },
        }
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
                self.access(m, c, *base, idx.max(0) as u32, dst, n, i);
            }
            E::AccessIndex { base, index } => self.access(m, c, *base, *index, dst, n, i),
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

    /// The shape of each `Access`/`AccessIndex` in `c`.
    fn access_shapes(&self, c: &Code, exprs: &naga::Arena<naga::Expression>) -> Vec<Acc> {
        let mut out = vec![Acc::default(); exprs.len()];
        for (h, e) in exprs.iter() {
            let (base, index) = match e {
                E::Access { base, .. } => (*base, None),
                E::AccessIndex { base, index } => (*base, Some(*index)),
                _ => continue,
            };
            out[h.index()] = self.shape(c, exprs, base, index);
        }
        out
    }

    fn shape(&self, c: &Code, exprs: &naga::Arena<naga::Expression>, base: Handle<naga::Expression>, index: Option<u32>) -> Acc {
        let module = &*self.module;
        let member = |ty: Handle<Type>| Acc { elem: 0, count: u32::MAX, fixed: Some(self.layout.member(ty, index.unwrap_or(0))) };
        let (elem, count) = match self.inner_of(c, exprs, base) {
            Some(TypeInner::Pointer { base: pt, .. }) => match &module.types[pt].inner {
                TypeInner::Struct { .. } => return member(pt),
                inner => index_shape(module, &self.layout, inner),
            },
            Some(TypeInner::ValuePointer { size, .. }) => (1, size.map_or(1, |s| s as u32)),
            Some(TypeInner::Struct { .. }) => return match self.type_handle_of(c, exprs, base) {
                Some(th) => member(th),
                None => Acc { elem: 0, count: u32::MAX, fixed: Some(0) },
            },
            Some(inner) => index_shape(module, &self.layout, &inner),
            None => (1, 1),
        };
        let fixed = index.map(|i| i.min(count.saturating_sub(1)) * elem);
        Acc { elem, count, fixed }
    }

    #[allow(clippy::too_many_arguments)]
    fn access(&self, m: &mut Machine, c: &Code, base: Handle<naga::Expression>, index: u32, dst: usize, n: usize, me: usize) {
        let a = c.acc[me];
        let off = match a.fixed { Some(o) => o, None => index.min(a.count.saturating_sub(1)) * a.elem };
        let bt = c.ty[base.index()];
        let bs = c.slot[base.index()] as usize;
        if bt.ptr {
            m.regs[dst] = m.regs[bs];
            m.regs[dst + 1] = m.regs[bs + 1] + off;
        } else {
            let o = off as usize;
            if o + n <= bt.lanes as usize { m.regs.copy_within(bs + o..bs + o + n, dst); } else { m.regs[dst..dst + n].fill(0); }
        }
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
