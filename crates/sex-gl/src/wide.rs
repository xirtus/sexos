// SPDX-License-Identifier: GPL-3.0-only
//! Runs `W` invocations of a stage at once. Every register is a row of `W` lanes
//! (`regs[slot][lane]`), so one prepared operation does the same work for all of
//! them and the arithmetic vectorizes. Each invocation keeps its own program counter;
//! the lowest counter runs next, with the invocations waiting there (the others are
//! masked off), so branches and loops that differ between pixels come back together
//! where they meet again. Expressions without a prepared operation run invocation by
//! invocation through the scalar `eval`, with their operands copied in and out.
use super::*;

/// Invocations per run.
pub const W: usize = 8;
const ALL: u32 = (1 << W) - 1;

type Row = [u32; W];

/// Per-run memory for `W` invocations of one stage: one row of `W` lanes per register.
pub struct WideMachine {
    regs: Vec<Row>,
    mem: Vec<Row>,
    loops: Vec<Row>,
    pcs: [u32; W],
    one: Machine,
    ret: u32,
    scratch: Vec<u32>,
    /// The invocations are 2x2 quads of pixels (lanes 4q.. are (x, y), (x+1, y),
    /// (x, y+1), (x+1, y+1)), so derivatives and mip levels come from neighbours.
    pub quads: bool,
}

impl WideMachine {
    /// Output lane `k` of invocation `lane` after a run.
    #[inline]
    pub fn out(&self, lane: usize, k: u32) -> u32 { self.regs[(self.ret + k) as usize][lane] }
    /// Register `slot` for every invocation (to set inputs before `run_rows`).
    #[inline]
    pub fn reg(&mut self, slot: u32) -> &mut [u32; W] { &mut self.regs[slot as usize] }
}

/// All-ones lanes for the invocations in `mask`.
#[inline(always)]
fn lane_mask(mask: u32) -> Row {
    let mut m = [0u32; W];
    for l in 0..W { m[l] = 0u32.wrapping_sub((mask >> l) & 1); }
    m
}

/// Writes `v` to register `d` for the masked invocations (`FULL`: all of them).
#[inline(always)]
fn wr<const FULL: bool>(r: &mut [Row], d: u32, v: Row, m: &Row) {
    let o = &mut r[d as usize];
    if FULL { *o = v; } else { for l in 0..W { o[l] = (v[l] & m[l]) | (o[l] & !m[l]); } }
}

#[inline(always)]
fn fv(r: &[Row], s: u32) -> [f32; W] { r[s as usize].map(f32::from_bits) }

#[inline(always)]
fn map1(a: [f32; W], f: impl Fn(f32) -> u32) -> Row { let mut o = [0u32; W]; for l in 0..W { o[l] = f(a[l]); } o }

#[inline(always)]
fn map2(a: [f32; W], b: [f32; W], f: impl Fn(f32, f32) -> u32) -> Row { let mut o = [0u32; W]; for l in 0..W { o[l] = f(a[l], b[l]); } o }

/// Float binary op over `n` lanes; `sa`/`sb` broadcast a scalar side.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn fbin<const FULL: bool>(r: &mut [Row], m: &Row, dst: u32, a: u32, b: u32, n: u32, sa: bool, sb: bool, f: impl Fn(f32, f32) -> u32) {
    for j in 0..n {
        let v = map2(fv(r, if sa { a } else { a + j }), fv(r, if sb { b } else { b + j }), &f);
        wr::<FULL>(r, dst + j, v, m);
    }
}

impl StageCode {
    pub fn wide_machine(&self) -> WideMachine {
        let n = self.regs_len as usize;
        WideMachine {
            regs: (0..n).map(|s| [self.consts[s]; W]).collect(),
            mem: vec![[0; W]; self.private_len as usize],
            loops: vec![[0; W]; self.loop_count as usize],
            pcs: [0; W],
            one: self.machine(),
            ret: self.entry.ret_off,
            scratch: Vec::new(),
            quads: false,
        }
    }

    /// Runs the entry point for invocations `0..count` (at most `W`). `input` fills each
    /// input binding's lanes for one invocation. Returns the invocations that were not
    /// discarded, as a bit mask.
    pub fn run_wide(&self, m: &mut WideMachine, count: usize, u: &[u32], tex: &dyn Textures, input: &mut dyn FnMut(usize, Io, &mut [u32])) -> u32 {
        for &(io, off, n) in &self.inputs {
            let n = n as usize;
            m.scratch.resize(n, 0);
            for l in 0..count.min(W) {
                for k in 0..n { m.scratch[k] = m.regs[off as usize + k][l]; }
                input(l, io, &mut m.scratch[..n]);
                for k in 0..n { m.regs[off as usize + k][l] = m.scratch[k]; }
            }
        }
        self.run_rows(m, count, u, tex)
    }

    /// `run_wide` with the inputs already in their registers (`inputs` names them).
    pub fn run_rows(&self, m: &mut WideMachine, count: usize, u: &[u32], tex: &dyn Textures) -> u32 {
        let live = if count >= W { ALL } else { (1u32 << count) - 1 };
        for &(off, n, init) in &self.private_init {
            for k in 0..n {
                let v = match init { Some(h) => self.consts[(self.globals.slot[h.index()] + k) as usize], None => 0 };
                m.mem[(off + k) as usize] = [v; W];
            }
        }
        self.exec_wide(m, live, u, tex)
    }

    fn exec_wide(&self, m: &mut WideMachine, mut active: u32, u: &[u32], tex: &dyn Textures) -> u32 {
        let steps = &self.steps[..];
        let len = steps.len() as u32;
        m.pcs = [0; W];
        // Set while every active invocation is at the same step.
        let mut shared = Some(0u32);
        loop {
            let (pc, mask) = match shared {
                Some(pc) => (pc, active),
                None => {
                    let mut pc = u32::MAX;
                    for l in 0..W { if (active >> l) & 1 != 0 && m.pcs[l] < pc { pc = m.pcs[l]; } }
                    let mut mask = 0;
                    for l in 0..W { if (active >> l) & 1 != 0 && m.pcs[l] == pc { mask |= 1 << l; } }
                    if mask == active { shared = Some(pc); }
                    (pc, mask)
                }
            };
            if pc >= len || active == 0 { break; }
            let full = mask == active;
            let mut next = pc + 1;
            let r = &mut m.regs;
            match &steps[pc as usize] {
                Step::Op(op) => if full { self.op_w::<true>(r, &m.mem, op, u, tex, mask, m.quads) } else { self.op_w::<false>(r, &m.mem, op, u, tex, mask, m.quads) },
                &Step::Eval { code, h } => {
                    let (c, f) = self.code(code as usize);
                    let one = &mut m.one;
                    let uses = &c.uses[h.index()];
                    let (d, dn) = (c.slot[h.index()], c.ty[h.index()].lanes);
                    for l in 0..W {
                        if (mask >> l) & 1 == 0 { continue; }
                        for &(s, n) in uses.iter() { for k in 0..n { one.regs[(s + k) as usize] = r[(s + k) as usize][l]; } }
                        self.eval(one, c, &f.expressions, h, u, tex);
                        for k in 0..dn { r[(d + k) as usize][l] = one.regs[(d + k) as usize]; }
                    }
                }
                &Step::Store { ptr, val, n } => {
                    for l in 0..W {
                        if (mask >> l) & 1 == 0 { continue; }
                        let (space, off) = (r[ptr as usize][l], r[(ptr + 1) as usize][l]);
                        if space == SPACE_PRIVATE && off + n <= self.private_len {
                            for k in 0..n { m.mem[(off + k) as usize][l] = r[(val + k) as usize][l]; }
                        }
                    }
                }
                &Step::StoreAt { off, val, n } => if full && mask == ALL {
                    m.mem[off as usize..(off + n) as usize].copy_from_slice(&r[val as usize..(val + n) as usize]);
                } else {
                    for l in 0..W {
                        if (mask >> l) & 1 == 0 { continue; }
                        for k in 0..n { m.mem[(off + k) as usize][l] = r[(val + k) as usize][l]; }
                    }
                },
                &Step::ToMem { off, src, n } => for l in 0..W {
                    if (mask >> l) & 1 == 0 { continue; }
                    for k in 0..n { m.mem[(off + k) as usize][l] = r[(src + k) as usize][l]; }
                },
                &Step::Zero { off, n } => for l in 0..W {
                    if (mask >> l) & 1 == 0 { continue; }
                    for k in 0..n { m.mem[(off + k) as usize][l] = 0; }
                },
                &Step::Jump(to) => next = to,
                &Step::JumpIfZero { cond, to } => { self.branch(m, mask, &mut shared, |r, l| r[cond as usize][l] == 0, pc, to); continue; }
                &Step::JumpIfNonZero { cond, to } => { self.branch(m, mask, &mut shared, |r, l| r[cond as usize][l] != 0, pc, to); continue; }
                &Step::JumpIfEq { sel, val, to } => { self.branch(m, mask, &mut shared, |r, l| r[sel as usize][l] == val, pc, to); continue; }
                &Step::LoopEnter(ctr) => for l in 0..W { if (mask >> l) & 1 != 0 { m.loops[ctr as usize][l] = 0; } },
                &Step::LoopBack { ctr, to } => {
                    let loops = &mut m.loops;
                    self.branch_with(&mut m.pcs, mask, &mut shared, |l| {
                        let k = &mut loops[ctr as usize][l];
                        *k += 1;
                        *k < LOOP_LIMIT
                    }, pc, to);
                    continue;
                }
                Step::Kill => {
                    active &= !mask;
                    if active != 0 { shared = None; }
                    continue;
                }
                Step::Nop => {}
            }
            match &mut shared {
                Some(p) => *p = next,
                None => for l in 0..W { if (mask >> l) & 1 != 0 { m.pcs[l] = next; } },
            }
        }
        active
    }

    #[inline(always)]
    fn branch(&self, m: &mut WideMachine, mask: u32, shared: &mut Option<u32>, take: impl Fn(&[Row], usize) -> bool, pc: u32, to: u32) {
        let r = &m.regs;
        self.branch_with(&mut m.pcs, mask, shared, |l| take(r, l), pc, to);
    }

    /// Sends each masked invocation to `to` if `take` says so, else to the next step.
    #[inline(always)]
    fn branch_with(&self, pcs: &mut [u32; W], mask: u32, shared: &mut Option<u32>, mut take: impl FnMut(usize) -> bool, pc: u32, to: u32) {
        let mut taken = 0u32;
        for l in 0..W { if (mask >> l) & 1 != 0 && take(l) { taken |= 1 << l; } }
        if shared.is_some() && (taken == 0 || taken == mask) {
            *shared = Some(if taken == 0 { pc + 1 } else { to });
            return;
        }
        if let Some(p) = *shared {
            // Diverging: the inactive invocations all wait at `p`.
            for l in 0..W { pcs[l] = p; }
            *shared = None;
        }
        for l in 0..W {
            if (mask >> l) & 1 == 0 { continue; }
            pcs[l] = if (taken >> l) & 1 != 0 { to } else { pc + 1 };
        }
    }

    /// A prepared operation for the masked invocations (`FULL`: all of them).
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn op_w<const FULL: bool>(&self, r: &mut [Row], mem: &[Row], op: &Op, u: &[u32], tex: &dyn Textures, mask: u32, quads: bool) {
        let lm = if FULL { [0; W] } else { lane_mask(mask) };
        let m = &lm;
        match op {
            &Op::Sample { dst, n, coord, cn, .. } => {
                let mut out = [[0u32; W]; 4];
                for l in 0..W {
                    if (mask >> l) & 1 == 0 { continue; }
                    // Coarse derivatives: across the top row and down the left column.
                    let grad = (quads && cn >= 2).then(|| {
                        let q = l & !3;
                        let c = |lane: usize, j: u32| f32::from_bits(r[(coord + j) as usize][lane]);
                        [c(q + 1, 0) - c(q, 0), c(q + 1, 1) - c(q, 1), c(q + 2, 0) - c(q, 0), c(q + 2, 1) - c(q, 1)]
                    });
                    let px = sample_op(op, |x| r[x as usize][l], tex, grad);
                    for j in 0..4 { out[j][l] = px[j].to_bits(); }
                }
                for j in 0..n.min(4) { wr::<FULL>(r, dst + j, out[j as usize], m); }
            }
            &Op::Deriv { dst, src, n, axis } => for j in 0..n {
                let v = fv(r, src + j);
                let mut o = [0u32; W];
                if quads {
                    for l in 0..W {
                        let q = l & !3;
                        let (dx, dy) = (v[q + 1] - v[q], v[q + 2] - v[q]);
                        o[l] = match axis { 0 => dx, 1 => dy, _ => libm::fabsf(dx) + libm::fabsf(dy) }.to_bits();
                    }
                }
                wr::<FULL>(r, dst + j, o, m);
            },
            Op::Generic => {}
            &Op::Copy { dst, src, n } => {
                if FULL {
                    r.copy_within(src as usize..(src + n) as usize, dst as usize);
                } else {
                    for j in 0..n { let v = r[(src + j) as usize]; wr::<FULL>(r, dst + j, v, m); }
                }
            }
            &Op::Splat { dst, src, n } => { let v = r[src as usize]; for j in 0..n { wr::<FULL>(r, dst + j, v, m); } }
            &Op::Swizzle { dst, src, pat, n } => {
                let mut tmp = [[0u32; W]; 4];
                for j in 0..n.min(4) as usize { tmp[j] = r[(src + pat[j] as u32) as usize]; }
                for j in 0..n.min(4) { wr::<FULL>(r, dst + j, tmp[j as usize], m); }
            }
            Op::Compose { dst, parts } => {
                let mut at = *dst;
                for &(s, k) in parts.iter() {
                    for j in 0..k { let v = r[(s + j) as usize]; wr::<FULL>(r, at + j, v, m); }
                    at += k;
                }
            }
            &Op::Ptr { dst, space, off } => { wr::<FULL>(r, dst, [space; W], m); wr::<FULL>(r, dst + 1, [off; W], m); }
            &Op::Load { dst, ptr, n } => {
                let (sp, of) = (r[ptr as usize], r[ptr as usize + 1]);
                let same = sp.iter().all(|&x| x == sp[0]) && of.iter().all(|&x| x == of[0]);
                if same {
                    // Every invocation reads the same place (a uniform, a local).
                    let (space, off) = (sp[0], of[0]);
                    if space == SPACE_UNIFORM {
                        let ok = (off + n) as usize <= u.len();
                        for k in 0..n { wr::<FULL>(r, dst + k, [if ok { u[(off + k) as usize] } else { 0 }; W], m); }
                    } else if off + n <= self.private_len {
                        for k in 0..n { let v = mem[(off + k) as usize]; wr::<FULL>(r, dst + k, v, m); }
                    }
                } else {
                    for l in 0..W {
                        if (mask >> l) & 1 == 0 { continue; }
                        let (space, off) = (sp[l], of[l]);
                        if space == SPACE_UNIFORM {
                            let ok = (off + n) as usize <= u.len();
                            for k in 0..n { r[(dst + k) as usize][l] = if ok { u[(off + k) as usize] } else { 0 }; }
                        } else if off + n <= self.private_len {
                            for k in 0..n { r[(dst + k) as usize][l] = mem[(off + k) as usize][l]; }
                        }
                    }
                }
            }
            &Op::Field { dst, base, off, n, ptr, base_lanes } => {
                if ptr {
                    let (s, mut o) = (r[base as usize], r[base as usize + 1]);
                    for x in o.iter_mut() { *x = x.wrapping_add(off); }
                    wr::<FULL>(r, dst, s, m);
                    wr::<FULL>(r, dst + 1, o, m);
                } else if off + n <= base_lanes {
                    for j in 0..n { let v = r[(base + off + j) as usize]; wr::<FULL>(r, dst + j, v, m); }
                } else {
                    for j in 0..n { wr::<FULL>(r, dst + j, [0; W], m); }
                }
            }
            &Op::FBin { op, dst, a, b, n, sa, sb } => {
                let d = dst;
                match op {
                    B::Add => fbin::<FULL>(r, m, d, a, b, n, sa, sb, |x, y| (x + y).to_bits()),
                    B::Subtract => fbin::<FULL>(r, m, d, a, b, n, sa, sb, |x, y| (x - y).to_bits()),
                    B::Multiply => fbin::<FULL>(r, m, d, a, b, n, sa, sb, |x, y| (x * y).to_bits()),
                    B::Divide => fbin::<FULL>(r, m, d, a, b, n, sa, sb, |x, y| (x / y).to_bits()),
                    B::Less => fbin::<FULL>(r, m, d, a, b, n, sa, sb, |x, y| (x < y) as u32),
                    B::LessEqual => fbin::<FULL>(r, m, d, a, b, n, sa, sb, |x, y| (x <= y) as u32),
                    B::Greater => fbin::<FULL>(r, m, d, a, b, n, sa, sb, |x, y| (x > y) as u32),
                    B::GreaterEqual => fbin::<FULL>(r, m, d, a, b, n, sa, sb, |x, y| (x >= y) as u32),
                    B::Equal => fbin::<FULL>(r, m, d, a, b, n, sa, sb, |x, y| (x == y) as u32),
                    _ => fbin::<FULL>(r, m, d, a, b, n, sa, sb, |x, y| (x != y) as u32),
                }
            }
            &Op::FMatVec { dst, m: mt, v, rows, cols } => {
                let mut out = [[0f32; W]; 4];
                for row in 0..rows.min(4) {
                    let mut s = [0f32; W];
                    for k in 0..cols {
                        let (a, b) = (fv(r, mt + k * rows + row), fv(r, v + k));
                        for l in 0..W { s[l] += a[l] * b[l]; }
                    }
                    out[row as usize] = s;
                }
                for row in 0..rows.min(4) { wr::<FULL>(r, dst + row, map1(out[row as usize], f32::to_bits), m); }
            }
            &Op::Dot { dst, a, b, n } => {
                let mut s = [0f32; W];
                for j in 0..n { let (x, y) = (fv(r, a + j), fv(r, b + j)); for l in 0..W { s[l] += x[l] * y[l]; } }
                wr::<FULL>(r, dst, map1(s, f32::to_bits), m);
            }
            &Op::Normalize { dst, a, n } => {
                let mut s = [0f32; W];
                for j in 0..n { let x = fv(r, a + j); for l in 0..W { s[l] += x[l] * x[l]; } }
                let mut k = [0f32; W];
                for l in 0..W { k[l] = 1.0 / libm::sqrtf(s[l]); }
                for j in 0..n { let v = map2(fv(r, a + j), k, |x, k| (x * k).to_bits()); wr::<FULL>(r, dst + j, v, m); }
            }
            &Op::FMax { dst, a, b, n, sb } => fbin::<FULL>(r, m, dst, a, b, n, false, sb, |x, y| x.max(y).to_bits()),
            &Op::FMin { dst, a, b, n, sb } => fbin::<FULL>(r, m, dst, a, b, n, false, sb, |x, y| x.min(y).to_bits()),
            &Op::Clamp { dst, a, lo, hi, n, s } => for j in 0..n {
                let k = if s { 0 } else { j };
                let (x, lo, hi) = (fv(r, a + j), fv(r, lo + k), fv(r, hi + k));
                let mut o = [0u32; W];
                for l in 0..W { o[l] = x[l].max(lo[l]).min(hi[l]).to_bits(); }
                wr::<FULL>(r, dst + j, o, m);
            },
            &Op::Mix { dst, a, b, t, n, st } => for j in 0..n {
                let k = if st { 0 } else { j };
                let (x, y, w) = (fv(r, a + j), fv(r, b + j), fv(r, t + k));
                let mut o = [0u32; W];
                for l in 0..W { o[l] = (x[l] * (1.0 - w[l]) + y[l] * w[l]).to_bits(); }
                wr::<FULL>(r, dst + j, o, m);
            },
            &Op::LoadAt { dst, space, off, n } => {
                if space == SPACE_UNIFORM {
                    let ok = (off + n) as usize <= u.len();
                    for k in 0..n { wr::<FULL>(r, dst + k, [if ok { u[(off + k) as usize] } else { 0 }; W], m); }
                } else {
                    for k in 0..n { let v = mem[(off + k) as usize]; wr::<FULL>(r, dst + k, v, m); }
                }
            }
            &Op::Bin { op, k, dst, a, b, n, sa, sb } => for j in 0..n {
                let (x, y) = (r[(if sa { a } else { a + j }) as usize], r[(if sb { b } else { b + j }) as usize]);
                let mut o = [0u32; W];
                for l in 0..W { o[l] = bin_lane(k, op, x[l], y[l]); }
                wr::<FULL>(r, dst + j, o, m);
            },
            &Op::Un { op, k, dst, a, n } => for j in 0..n {
                let x = r[(a + j) as usize];
                let mut o = [0u32; W];
                for l in 0..W { o[l] = un_lane(op, k, x[l]); }
                wr::<FULL>(r, dst + j, o, m);
            },
            &Op::Conv { from, to, convert, dst, a, n } => for j in 0..n {
                let x = r[(a + j) as usize];
                let mut o = [0u32; W];
                for l in 0..W { o[l] = as_lane(from, to, convert, x[l]); }
                wr::<FULL>(r, dst + j, o, m);
            },
            &Op::FMath { fun, dst, a, b, c, n, sa, sb, sc } => for j in 0..n {
                let (x, y, z) = (fv(r, if sa { a } else { a + j }), fv(r, if sb { b } else { b + j }), fv(r, if sc { c } else { c + j }));
                let mut o = [0u32; W];
                // Only the masked lanes: a dead lane's value may be anything.
                for l in 0..W { if (mask >> l) & 1 != 0 { o[l] = fmath_lane(fun, x[l], y[l], z[l]).to_bits(); } }
                wr::<FULL>(r, dst + j, o, m);
            },
            &Op::Select { dst, cond, a, r: rj, n, sc } => {
                let mut tmp = [[0u32; W]; 16];
                for j in 0..n.min(16) {
                    let (cv, av, rv) = (r[(if sc { cond } else { cond + j }) as usize], r[(a + j) as usize], r[(rj + j) as usize]);
                    for l in 0..W { tmp[j as usize][l] = if cv[l] != 0 { av[l] } else { rv[l] }; }
                }
                for j in 0..n.min(16) { wr::<FULL>(r, dst + j, tmp[j as usize], m); }
            }
        }
    }
}
