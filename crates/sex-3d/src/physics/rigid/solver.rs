// SPDX-License-Identifier: GPL-3.0-only
//! The rigid body solver: semi-implicit Euler with substeps, sort-and-sweep
//! broadphase, sequential impulses (Catto) with warm starting, a friction
//! cone, restitution, Baumgarte or split-impulse position correction,
//! joints as constraint rows (locks, limits, springs, motors) with a
//! breaking threshold, and sleeping.
use super::collide::{collide, Contact, Pose, WPoly};
use super::shape::{overlap, Shape};
use crate::math::{Aabb, Mat3, Quat, Vec3};
use crate::physics::settings::{ConstraintType, RigidBodyConstraint};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A body taking part in one step.
#[derive(Clone, Debug)]
pub struct Body {
    pub name: String,
    pub shape: Arc<Shape>,
    pub inv_mass: f32,
    pub inv_inertia: Vec3,
    pub pose: Pose,
    pub vel: Vec3,
    pub ang: Vec3,
    /// Moved by the solver.
    pub dynamic: bool,
    /// Kinematic target pose for the end of the frame (animated or passive bodies).
    pub target: Option<(Vec3, Quat)>,
    pub friction: f32,
    pub restitution: f32,
    pub lin_damp: f32,
    pub ang_damp: f32,
    pub margin: f32,
    pub deactivation: Option<(f32, f32)>,
    pub sleeping: bool,
    pub sleep_time: f32,
    /// External force (field effectors), per step.
    pub force: Vec3,
}

impl Body {
    fn inv_inertia_world(&self) -> Mat3 {
        let m = self.pose.m;
        m * Mat3::diagonal(self.inv_inertia) * m.transpose()
    }

    fn awake_dynamic(&self) -> bool {
        self.dynamic && !self.sleeping
    }
}

/// A joint between two bodies (or a body and the world).
#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    pub a: Option<usize>,
    pub b: Option<usize>,
    pub settings: RigidBodyConstraint,
    /// Anchor and frame in each body's space (world space for no body).
    pub anchor_a: Vec3,
    pub anchor_b: Vec3,
    pub frame_a: Quat,
    pub frame_b: Quat,
    pub broken: bool,
    pub impulses: Vec<f32>,
}

/// Warm-starting impulses carried between steps.
#[derive(Clone, Debug, Default)]
pub struct WarmCache {
    /// Per body pair: (local point on A, normal impulse, tangent impulses).
    pub contacts: HashMap<(usize, usize), Vec<(Vec3, f32, [f32; 2])>>,
}

#[derive(Clone, Copy)]
struct CRow {
    a: usize,
    b: usize,
    ra: Vec3,
    rb: Vec3,
    n: Vec3,
    t: [Vec3; 2],
    mn: f32,
    mt: [f32; 2],
    target: f32,
    push: f32,
    mu: f32,
    jn: f32,
    jt: [f32; 2],
    jp: f32,
    local_a: Vec3,
}

#[derive(Clone, Copy)]
struct JRow {
    joint: usize,
    slot: usize,
    a: Option<usize>,
    b: Option<usize>,
    la: Vec3,
    aa: Vec3,
    lb: Vec3,
    ab: Vec3,
    m: f32,
    bias: f32,
    gamma: f32,
    lo: f32,
    hi: f32,
}

pub struct World {
    pub bodies: Vec<Body>,
    pub joints: Vec<Joint>,
    pub gravity: Vec3,
    pub iterations: u32,
    pub split_impulse: bool,
    pub no_collide: HashSet<(usize, usize)>,
    pub warm: WarmCache,
}

const SLOP: f32 = 0.005;
const BAUMGARTE: f32 = 0.2;
const BOUNCE_THRESHOLD: f32 = 0.2;
const SLEEP_DELAY: f32 = 2.0;

fn tangents(n: Vec3) -> [Vec3; 2] {
    let t1 = n.any_orthonormal();
    [t1, n.cross(t1)]
}

fn quat_integrate(q: Quat, w: Vec3, dt: f32) -> Quat {
    let wq = Quat { x: w.x, y: w.y, z: w.z, w: 0.0 };
    let dq = wq * q;
    Quat { x: q.x + 0.5 * dt * dq.x, y: q.y + 0.5 * dt * dq.y, z: q.z + 0.5 * dt * dq.z, w: q.w + 0.5 * dt * dq.w }.normalize()
}

/// Small rotation vector taking `a` to `b` (both unit quaternions).
fn rot_error(a: Quat, b: Quat) -> Vec3 {
    let mut q = b * a.conjugate();
    if q.w < 0.0 {
        q = Quat { x: -q.x, y: -q.y, z: -q.z, w: -q.w };
    }
    let v = Vec3::new(q.x, q.y, q.z);
    let s = v.length();
    if s < 1e-9 {
        return v * 2.0;
    }
    v * (2.0 * s.atan2(q.w) / s)
}

impl World {
    fn aabb(&self, i: usize, dt: f32) -> Aabb {
        let b = &self.bodies[i];
        let lb = b.shape.local_bounds();
        let mut out = Aabb::EMPTY;
        for k in 0..8 {
            let p = Vec3::new(if k & 1 == 0 { lb.min.x } else { lb.max.x }, if k & 2 == 0 { lb.min.y } else { lb.max.y }, if k & 4 == 0 { lb.min.z } else { lb.max.z });
            out.grow(b.pose.point(p));
        }
        let grow = Vec3::splat(b.margin + 0.02) + b.vel.abs() * dt;
        Aabb { min: out.min - grow, max: out.max + grow }
    }

    /// Sort and sweep along X.
    fn pairs(&self, dt: f32) -> Vec<(usize, usize)> {
        let boxes: Vec<Aabb> = (0..self.bodies.len()).map(|i| self.aabb(i, dt)).collect();
        let mut order: Vec<usize> = (0..self.bodies.len()).collect();
        order.sort_by(|&a, &b| boxes[a].min.x.partial_cmp(&boxes[b].min.x).unwrap_or(std::cmp::Ordering::Equal));
        let mut out = Vec::new();
        for (k, &i) in order.iter().enumerate() {
            for &j in &order[k + 1..] {
                if boxes[j].min.x > boxes[i].max.x {
                    break;
                }
                let (a, b) = (i.min(j), i.max(j));
                let (ba, bb) = (&self.bodies[a], &self.bodies[b]);
                if !ba.dynamic && !bb.dynamic {
                    continue;
                }
                if !ba.awake_dynamic() && !bb.awake_dynamic() {
                    continue;
                }
                if self.no_collide.contains(&(a, b)) || !overlap(&boxes[a], &boxes[b]) {
                    continue;
                }
                out.push((a, b));
            }
        }
        out.sort_unstable();
        out
    }

    /// One substep of `dt` seconds; `alpha` is how far through the frame the
    /// substep ends (for kinematic targets).
    pub fn step(&mut self, dt: f32, alpha: f32, start: &[(Vec3, Quat)]) {
        let n = self.bodies.len();
        // Kinematic bodies move towards their targets.
        for (i, b) in self.bodies.iter_mut().enumerate() {
            if b.dynamic {
                continue;
            }
            if let Some((tp, tq)) = b.target {
                let (sp, sq) = start[i];
                let p = sp.lerp(tp, alpha);
                let q = sq.slerp(tq, alpha).normalize();
                b.vel = (p - b.pose.pos) / dt;
                b.ang = rot_error(b.pose.rot, q) / dt;
                b.pose = Pose::new(p, q);
            } else {
                b.vel = Vec3::ZERO;
                b.ang = Vec3::ZERO;
            }
        }
        // Forces.
        for b in self.bodies.iter_mut() {
            if !b.awake_dynamic() {
                continue;
            }
            b.vel += (self.gravity + b.force * b.inv_mass) * dt;
            b.vel *= (1.0 - b.lin_damp).clamp(0.0, 1.0).powf(dt);
            b.ang *= (1.0 - b.ang_damp).clamp(0.0, 1.0).powf(dt);
        }
        // Contacts.
        let pairs = self.pairs(dt);
        let placed: Vec<(Arc<Shape>, Pose)> = self.bodies.iter().map(|b| (b.shape.clone(), b.pose)).collect();
        let polys: Vec<Option<WPoly>> = placed
            .iter()
            .map(|(sh, pose)| match &**sh {
                Shape::Poly(p) => Some(WPoly::new(p, pose)),
                _ => None,
            })
            .collect();
        let mut rows: Vec<CRow> = Vec::new();
        let mut cs: Vec<Contact> = Vec::new();
        let mut new_warm = WarmCache::default();
        for &(a, b) in &pairs {
            cs.clear();
            let (ba, bb) = (&self.bodies[a], &self.bodies[b]);
            let margin = ba.margin + bb.margin + 0.01;
            collide(&ba.shape, &ba.pose, polys[a].as_ref(), &bb.shape, &bb.pose, polys[b].as_ref(), margin, &mut cs);
            if cs.is_empty() {
                continue;
            }
            // Wake sleepers touched by moving bodies.
            let speed = |x: &Body| x.vel.length() + x.ang.length();
            if self.bodies[a].sleeping && !self.bodies[b].sleeping && speed(&self.bodies[b]) > 0.05 {
                self.bodies[a].sleeping = false;
                self.bodies[a].sleep_time = 0.0;
            }
            if self.bodies[b].sleeping && !self.bodies[a].sleeping && speed(&self.bodies[a]) > 0.05 {
                self.bodies[b].sleeping = false;
                self.bodies[b].sleep_time = 0.0;
            }
            let (ba, bb) = (&self.bodies[a], &self.bodies[b]);
            let old = self.warm.contacts.get(&(a, b));
            let mu = (ba.friction * bb.friction).max(0.0);
            let e = (ba.restitution * bb.restitution).clamp(0.0, 1.0);
            let (ima, imb) = (if ba.awake_dynamic() { ba.inv_mass } else { 0.0 }, if bb.awake_dynamic() { bb.inv_mass } else { 0.0 });
            let (ia, ib) = (if ba.awake_dynamic() { ba.inv_inertia_world() } else { Mat3::ZERO }, if bb.awake_dynamic() { bb.inv_inertia_world() } else { Mat3::ZERO });
            for c in &cs {
                let ra = c.p - ba.pose.pos;
                let rb = c.p - bb.pose.pos;
                let k = |d: Vec3| {
                    let x = ra.cross(d);
                    let y = rb.cross(d);
                    ima + imb + x.dot(ia.mul_vec(x)) + y.dot(ib.mul_vec(y))
                };
                let t = tangents(c.n);
                let rel = bb.vel + bb.ang.cross(rb) - ba.vel - ba.ang.cross(ra);
                let vn = rel.dot(c.n);
                let push = BAUMGARTE / dt * (c.depth - SLOP).max(0.0);
                let bounce = if vn < -BOUNCE_THRESHOLD { -e * vn } else { 0.0 };
                let target = if self.split_impulse { bounce } else { bounce.max(push) };
                let local_a = ba.pose.inv_point(c.p);
                let mut jn = 0.0;
                let mut jt = [0.0; 2];
                if let Some(old) = old {
                    if let Some(o) = old.iter().min_by(|x, y| x.0.distance(local_a).partial_cmp(&y.0.distance(local_a)).unwrap()) {
                        if o.0.distance(local_a) < 0.05 {
                            jn = o.1;
                            jt = o.2;
                        }
                    }
                }
                let kn = k(c.n);
                rows.push(CRow {
                    a,
                    b,
                    ra,
                    rb,
                    n: c.n,
                    t,
                    mn: if kn > 0.0 { 1.0 / kn } else { 0.0 },
                    mt: [k(t[0]), k(t[1])].map(|x| if x > 0.0 { 1.0 / x } else { 0.0 }),
                    target,
                    push,
                    mu,
                    jn: jn * 0.9,
                    jt: jt.map(|x| x * 0.9),
                    jp: 0.0,
                    local_a,
                });
            }
        }
        // Joint rows.
        let mut jrows = self.joint_rows(dt);
        for j in self.joints.iter_mut() {
            if j.impulses.len() < 14 {
                j.impulses.resize(14, 0.0);
            }
        }
        // Warm start.
        for r in &rows {
            let p = r.n * r.jn + r.t[0] * r.jt[0] + r.t[1] * r.jt[1];
            self.apply(r.a, r.b, r.ra, r.rb, p);
        }
        for r in &jrows {
            let j = self.joints[r.joint].impulses.get(r.slot).copied().unwrap_or(0.0);
            self.apply_row(r, j);
        }
        // Iterate.
        let mut broke = vec![0.0f32; self.joints.len()];
        for _ in 0..self.iterations.max(1) {
            for r in jrows.iter_mut() {
                let jv = self.row_velocity(r);
                let acc = self.joints[r.joint].impulses[r.slot];
                let mut l = -r.m * (jv + r.bias + r.gamma * acc);
                let new = (acc + l).clamp(r.lo, r.hi);
                l = new - acc;
                self.joints[r.joint].impulses[r.slot] = new;
                self.apply_row(r, l);
            }
            for r in rows.iter_mut() {
                let (ba, bb) = (&self.bodies[r.a], &self.bodies[r.b]);
                let rel = bb.vel + bb.ang.cross(r.rb) - ba.vel - ba.ang.cross(r.ra);
                // Friction (cone).
                let lim = r.mu * r.jn;
                let vt = [rel.dot(r.t[0]), rel.dot(r.t[1])];
                let mut nt = [r.jt[0] - r.mt[0] * vt[0], r.jt[1] - r.mt[1] * vt[1]];
                let len = (nt[0] * nt[0] + nt[1] * nt[1]).sqrt();
                if len > lim && len > 0.0 {
                    nt = [nt[0] * lim / len, nt[1] * lim / len];
                }
                let d = [nt[0] - r.jt[0], nt[1] - r.jt[1]];
                r.jt = nt;
                let p = r.t[0] * d[0] + r.t[1] * d[1];
                let (a, b, ra, rb) = (r.a, r.b, r.ra, r.rb);
                self.apply(a, b, ra, rb, p);
                // Normal.
                let (ba, bb) = (&self.bodies[r.a], &self.bodies[r.b]);
                let rel = bb.vel + bb.ang.cross(r.rb) - ba.vel - ba.ang.cross(r.ra);
                let vn = rel.dot(r.n);
                let new = (r.jn + r.mn * (r.target - vn)).max(0.0);
                let dl = new - r.jn;
                r.jn = new;
                self.apply(a, b, ra, rb, r.n * dl);
            }
        }
        for r in &jrows {
            broke[r.joint] += self.joints[r.joint].impulses[r.slot].abs();
        }
        for (j, &imp) in self.joints.iter_mut().zip(&broke) {
            if j.settings.use_breaking && imp > j.settings.breaking_threshold {
                j.broken = true;
            }
        }
        // Split impulse: position correction with pseudo velocities.
        let mut pseudo: Vec<(Vec3, Vec3)> = vec![(Vec3::ZERO, Vec3::ZERO); n];
        if self.split_impulse {
            for _ in 0..self.iterations.max(1) {
                for r in rows.iter_mut() {
                    let (pa, pb) = (pseudo[r.a], pseudo[r.b]);
                    let vn = (pb.0 + pb.1.cross(r.rb) - pa.0 - pa.1.cross(r.ra)).dot(r.n);
                    let new = (r.jp + r.mn * (r.push - vn)).max(0.0);
                    let dl = new - r.jp;
                    r.jp = new;
                    let p = r.n * dl;
                    for (i, rr, s) in [(r.a, r.ra, -1.0f32), (r.b, r.rb, 1.0)] {
                        let b = &self.bodies[i];
                        if b.awake_dynamic() {
                            pseudo[i].0 += p * (s * b.inv_mass);
                            pseudo[i].1 += b.inv_inertia_world().mul_vec(rr.cross(p * s));
                        }
                    }
                }
            }
        }
        // Integrate.
        for (i, b) in self.bodies.iter_mut().enumerate() {
            if !b.awake_dynamic() {
                continue;
            }
            let v = b.vel + pseudo[i].0;
            let w = b.ang + pseudo[i].1;
            b.pose = Pose::new(b.pose.pos + v * dt, quat_integrate(b.pose.rot, w, dt));
            // Sleeping.
            if let Some((lin, ang)) = b.deactivation {
                if b.vel.length() < lin && b.ang.length() < ang {
                    b.sleep_time += dt;
                    if b.sleep_time > SLEEP_DELAY {
                        b.sleeping = true;
                        b.vel = Vec3::ZERO;
                        b.ang = Vec3::ZERO;
                    }
                } else {
                    b.sleep_time = 0.0;
                }
            }
        }
        for r in &rows {
            new_warm.contacts.entry((r.a, r.b)).or_default().push((r.local_a, r.jn, r.jt));
        }
        self.warm = new_warm;
    }

    fn apply(&mut self, a: usize, b: usize, ra: Vec3, rb: Vec3, p: Vec3) {
        let ba = &self.bodies[a];
        if ba.awake_dynamic() {
            let ii = ba.inv_inertia_world();
            let im = ba.inv_mass;
            let ba = &mut self.bodies[a];
            ba.vel -= p * im;
            ba.ang -= ii.mul_vec(ra.cross(p));
        }
        let bb = &self.bodies[b];
        if bb.awake_dynamic() {
            let ii = bb.inv_inertia_world();
            let im = bb.inv_mass;
            let bb = &mut self.bodies[b];
            bb.vel += p * im;
            bb.ang += ii.mul_vec(rb.cross(p));
        }
    }

    fn row_velocity(&self, r: &JRow) -> f32 {
        let mut v = 0.0;
        if let Some(a) = r.a {
            let b = &self.bodies[a];
            v += r.la.dot(b.vel) + r.aa.dot(b.ang);
        }
        if let Some(bi) = r.b {
            let b = &self.bodies[bi];
            v += r.lb.dot(b.vel) + r.ab.dot(b.ang);
        }
        v
    }

    fn apply_row(&mut self, r: &JRow, l: f32) {
        for (i, lin, ang) in [(r.a, r.la, r.aa), (r.b, r.lb, r.ab)] {
            let Some(i) = i else { continue };
            let b = &self.bodies[i];
            if !b.awake_dynamic() {
                continue;
            }
            let ii = b.inv_inertia_world();
            let im = b.inv_mass;
            let b = &mut self.bodies[i];
            b.vel += lin * (im * l);
            b.ang += ii.mul_vec(ang * l);
        }
    }

    fn joint_pose(&self, i: Option<usize>, anchor: Vec3, frame: Quat) -> (Vec3, Quat, Vec3) {
        match i {
            Some(i) => {
                let b = &self.bodies[i];
                let p = b.pose.point(anchor);
                (p, (b.pose.rot * frame).normalize(), p - b.pose.pos)
            }
            None => (anchor, frame, Vec3::ZERO),
        }
    }

    fn joint_rows(&mut self, dt: f32) -> Vec<JRow> {
        let mut out = Vec::new();
        for ji in 0..self.joints.len() {
            let j = &self.joints[ji];
            if j.broken || !j.settings.enabled {
                continue;
            }
            let woke = |w: &World, i: Option<usize>| i.is_some_and(|i| w.bodies[i].awake_dynamic());
            if !woke(self, j.a) && !woke(self, j.b) {
                continue;
            }
            let (pa, qa, ra) = self.joint_pose(j.a, j.anchor_a, j.frame_a);
            let (pb, qb, rb) = self.joint_pose(j.b, j.anchor_b, j.frame_b);
            let axes = [qa.rotate(Vec3::X), qa.rotate(Vec3::Y), qa.rotate(Vec3::Z)];
            let s = &j.settings;
            let ang_err = rot_error(qa, qb);
            let qrel = (qa.conjugate() * qb).normalize();
            let twist = |k: usize| {
                let c = [qrel.x, qrel.y, qrel.z][k];
                let mut a = 2.0 * c.atan2(qrel.w);
                if a > std::f32::consts::PI {
                    a -= std::f32::consts::TAU;
                } else if a < -std::f32::consts::PI {
                    a += std::f32::consts::TAU;
                }
                a
            };
            // (axis index 0..6, mode) where mode: 0 lock, 1 limit, 2 free.
            let kinds: [u8; 6] = match s.kind {
                ConstraintType::Fixed => [0; 6],
                ConstraintType::Point => [0, 0, 0, 2, 2, 2],
                ConstraintType::Hinge => [0, 0, 0, 0, 0, if s.axes[5].use_limit { 1 } else { 2 }],
                ConstraintType::Slider => [if s.axes[0].use_limit { 1 } else { 2 }, 0, 0, 0, 0, 0],
                ConstraintType::Piston => [if s.axes[0].use_limit { 1 } else { 2 }, 0, 0, if s.axes[3].use_limit { 1 } else { 2 }, 0, 0],
                ConstraintType::Generic | ConstraintType::GenericSpring => {
                    let mut k = [2u8; 6];
                    for (i, a) in s.axes.iter().enumerate() {
                        if a.use_limit {
                            k[i] = if (a.upper - a.lower).abs() < 1e-6 { 0 } else { 1 };
                        }
                    }
                    k
                }
                ConstraintType::Motor => [2; 6],
            };
            let ima = j.a.map_or(0.0, |i| if self.bodies[i].awake_dynamic() { self.bodies[i].inv_mass } else { 0.0 });
            let imb = j.b.map_or(0.0, |i| if self.bodies[i].awake_dynamic() { self.bodies[i].inv_mass } else { 0.0 });
            let iia = j.a.map_or(Mat3::ZERO, |i| if self.bodies[i].awake_dynamic() { self.bodies[i].inv_inertia_world() } else { Mat3::ZERO });
            let iib = j.b.map_or(Mat3::ZERO, |i| if self.bodies[i].awake_dynamic() { self.bodies[i].inv_inertia_world() } else { Mat3::ZERO });
            let push = |out: &mut Vec<JRow>, slot: usize, la: Vec3, aa: Vec3, lb: Vec3, ab: Vec3, c: f32, lo: f32, hi: f32, soft: Option<(f32, f32)>, vel_bias: Option<f32>| {
                let k = la.dot(la) * ima + aa.dot(iia.mul_vec(aa)) + lb.dot(lb) * imb + ab.dot(iib.mul_vec(ab));
                if k <= 0.0 {
                    return;
                }
                let (bias, gamma) = match (soft, vel_bias) {
                    (_, Some(v)) => (v, 0.0),
                    (Some((ks, cs)), None) => {
                        let den = cs + dt * ks;
                        if den <= 0.0 {
                            (0.0, 0.0)
                        } else {
                            let g = 1.0 / (dt * den);
                            (dt * ks * g * c, g)
                        }
                    }
                    (None, None) => (BAUMGARTE / dt * c, 0.0),
                };
                out.push(JRow { joint: ji, slot, a: j.a, b: j.b, la, aa, lb, ab, m: 1.0 / (k + gamma), bias, gamma, lo, hi });
            };
            let d = pb - pa;
            for k in 0..3 {
                let ax = axes[k];
                let c = ax.dot(d);
                let (la, aa, lb, ab) = (-ax, -ra.cross(ax), ax, rb.cross(ax));
                let sl = 2 * k;
                match kinds[k] {
                    0 => push(&mut out, sl, la, aa, lb, ab, c, f32::NEG_INFINITY, f32::INFINITY, None, None),
                    1 => {
                        let lim = s.axes[k];
                        if c < lim.lower {
                            push(&mut out, sl, la, aa, lb, ab, c - lim.lower, 0.0, f32::INFINITY, None, None);
                        } else if c > lim.upper {
                            push(&mut out, sl, la, aa, lb, ab, c - lim.upper, f32::NEG_INFINITY, 0.0, None, None);
                        }
                    }
                    _ => {}
                }
                let sp = s.axes[k];
                if s.kind == ConstraintType::GenericSpring && sp.use_spring {
                    push(&mut out, sl + 1, la, aa, lb, ab, c, f32::NEG_INFINITY, f32::INFINITY, Some((sp.stiffness, sp.damping)), None);
                }
            }
            for k in 0..3 {
                let ax = axes[k];
                let i = 3 + k;
                let c = match kinds[i] {
                    1 => twist(k),
                    _ => ax.dot(ang_err),
                };
                let sl = 2 * i;
                match kinds[i] {
                    0 => push(&mut out, sl, Vec3::ZERO, -ax, Vec3::ZERO, ax, c, f32::NEG_INFINITY, f32::INFINITY, None, None),
                    1 => {
                        let lim = s.axes[i];
                        if c < lim.lower {
                            push(&mut out, sl, Vec3::ZERO, -ax, Vec3::ZERO, ax, c - lim.lower, 0.0, f32::INFINITY, None, None);
                        } else if c > lim.upper {
                            push(&mut out, sl, Vec3::ZERO, -ax, Vec3::ZERO, ax, c - lim.upper, f32::NEG_INFINITY, 0.0, None, None);
                        }
                    }
                    _ => {}
                }
                let sp = s.axes[i];
                if s.kind == ConstraintType::GenericSpring && sp.use_spring {
                    push(&mut out, sl + 1, Vec3::ZERO, -ax, Vec3::ZERO, ax, twist(k), f32::NEG_INFINITY, f32::INFINITY, Some((sp.stiffness, sp.damping)), None);
                }
            }
            if s.kind == ConstraintType::Motor {
                let ax = axes[0];
                if s.use_motor_lin {
                    let m = s.motor_lin_max_impulse;
                    push(&mut out, 12, -ax, -ra.cross(ax), ax, rb.cross(ax), 0.0, -m, m, None, Some(-s.motor_lin_target_velocity));
                }
                if s.use_motor_ang {
                    let m = s.motor_ang_max_impulse;
                    push(&mut out, 13, Vec3::ZERO, -ax, Vec3::ZERO, ax, 0.0, -m, m, None, Some(-s.motor_ang_target_velocity));
                }
            }
        }
        for r in &out {
            let j = &mut self.joints[r.joint];
            if j.impulses.len() <= r.slot {
                j.impulses.resize(r.slot + 1, 0.0);
            }
        }
        // Rows that are not active this step forget their impulse.
        let mut active: HashSet<(usize, usize)> = HashSet::new();
        for r in &out {
            active.insert((r.joint, r.slot));
        }
        for (ji, j) in self.joints.iter_mut().enumerate() {
            for (s, imp) in j.impulses.iter_mut().enumerate() {
                if !active.contains(&(ji, s)) {
                    *imp = 0.0;
                }
            }
        }
        out
    }
}
