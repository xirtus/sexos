// SPDX-License-Identifier: GPL-3.0-only
//! Curves as Blender's `CurvesGeometry`: control points of every curve in
//! one array (`offsets` delimit the curves) with attributes on the point and
//! curve domains. Built-in attributes keep Blender's names: `curve_type`
//! (0 Catmull-Rom, 1 poly, 2 Bezier, 3 NURBS), `cyclic`, `resolution`,
//! `nurbs_order`, `knots_mode`, `normal_mode`, `radius`, `tilt`,
//! `handle_left`, `handle_right`, `handle_type_left`, `handle_type_right`
//! and `nurbs_weight`.
//!
//! Evaluation follows `BKE_curves`: poly curves are their points; Bezier
//! segments are sampled `resolution` times between a point, its right
//! handle, the next point's left handle and the next point; Catmull-Rom
//! uses the uniform Catmull-Rom basis (ends extrapolated); NURBS evaluate
//! the B-spline basis of `nurbs_order` over a uniform or endpoint knot vector.
use super::attributes::{interp, mix, AttrData, AttrType, Attributes, Domain, Val};
use crate::math::{Quat, Vec3};

pub const CATMULL_ROM: i32 = 0;
pub const POLY: i32 = 1;
pub const BEZIER: i32 = 2;
pub const NURBS: i32 = 3;

/// Bezier handle types (`BEZIER_HANDLE_*`).
pub const HANDLE_FREE: i32 = 0;
pub const HANDLE_AUTO: i32 = 1;
pub const HANDLE_VECTOR: i32 = 2;
pub const HANDLE_ALIGN: i32 = 3;

/// Built-in attributes whose default is not zero.
const BUILTIN_DEFAULTS: [(&str, Domain, Val); 5] =
    [("resolution", Domain::Curve, Val::Int(12)), ("radius", Domain::Point, Val::Float(1.0)), ("nurbs_order", Domain::Curve, Val::Int(4)), ("nurbs_weight", Domain::Point, Val::Float(1.0)), ("curve_type", Domain::Curve, Val::Int(CATMULL_ROM))];

#[derive(Clone, Debug, PartialEq)]
pub struct Curves {
    pub positions: Vec<Vec3>,
    /// `offsets[i]..offsets[i + 1]` are curve `i`'s points.
    pub offsets: Vec<u32>,
    pub attributes: Attributes,
}

impl Default for Curves {
    fn default() -> Self {
        Curves { positions: Vec::new(), offsets: vec![0], attributes: Attributes::default() }
    }
}

/// A curve after evaluation: dense positions and their frames, with where
/// each evaluated point came from (`src`: control points `a`, `b` and the
/// factor between them).
#[derive(Clone, Debug, Default)]
pub struct Evaluated {
    pub positions: Vec<Vec3>,
    pub tangents: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub src: Vec<(usize, usize, f32)>,
    pub cyclic: bool,
    /// Arc length at each evaluated point (and the closing length last when cyclic).
    pub lengths: Vec<f32>,
}

impl Evaluated {
    pub fn total_length(&self) -> f32 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    /// Segment count (one more than points-1 when cyclic).
    fn segments(&self) -> usize {
        let n = self.positions.len();
        if n < 2 {
            0
        } else if self.cyclic {
            n
        } else {
            n - 1
        }
    }

    /// (segment start index, factor) at arc length `l`.
    pub fn locate(&self, l: f32) -> (usize, f32) {
        let segs = self.segments();
        if segs == 0 {
            return (0, 0.0);
        }
        let l = l.clamp(0.0, self.total_length());
        // lengths[k] is the length up to point k; lengths[segs] the end.
        let mut k = self.lengths.partition_point(|&x| x <= l).saturating_sub(1);
        if k >= segs {
            k = segs - 1;
        }
        let (a, b) = (self.lengths[k], self.lengths[k + 1]);
        let f = if b > a { (l - a) / (b - a) } else { 0.0 };
        (k, f.clamp(0.0, 1.0))
    }

    fn next(&self, k: usize) -> usize {
        (k + 1) % self.positions.len().max(1)
    }

    /// Position, tangent and normal at arc length `l`.
    pub fn sample(&self, l: f32) -> (Vec3, Vec3, Vec3) {
        if self.positions.is_empty() {
            return (Vec3::ZERO, Vec3::X, Vec3::Z);
        }
        if self.positions.len() == 1 {
            return (self.positions[0], self.tangents[0], self.normals[0]);
        }
        let (k, f) = self.locate(l);
        let j = self.next(k);
        let p = self.positions[k].lerp(self.positions[j], f);
        let t = self.tangents[k].lerp(self.tangents[j], f).normalize();
        let n = self.normals[k].lerp(self.normals[j], f).normalize();
        (p, t, n)
    }

    /// The control-point source mix at arc length `l`.
    pub fn sample_src(&self, l: f32) -> Vec<(usize, f32)> {
        if self.src.is_empty() {
            return Vec::new();
        }
        if self.src.len() == 1 {
            return vec![(self.src[0].0, 1.0)];
        }
        let (k, f) = self.locate(l);
        let j = self.next(k);
        let (a, b) = (self.src[k], self.src[j]);
        vec![(a.0, (1.0 - a.2) * (1.0 - f)), (a.1, a.2 * (1.0 - f)), (b.0, (1.0 - b.2) * f), (b.1, b.2 * f)]
    }
}

fn bisect(a: Vec3, b: Vec3) -> Vec3 {
    let d = a.normalize() + b.normalize();
    if d.length_squared() < 1e-12 {
        a.normalize()
    } else {
        d.normalize()
    }
}

/// Tangents of a polyline (`poly::calculate_tangents`).
pub fn poly_tangents(p: &[Vec3], cyclic: bool) -> Vec<Vec3> {
    let n = p.len();
    if n < 2 {
        return vec![Vec3::Z; n];
    }
    (0..n)
        .map(|i| {
            if i == 0 && !cyclic {
                (p[1] - p[0]).normalize()
            } else if i == n - 1 && !cyclic {
                (p[n - 1] - p[n - 2]).normalize()
            } else {
                let prev = p[(i + n - 1) % n];
                let next = p[(i + 1) % n];
                bisect(p[i] - prev, next - p[i])
            }
        })
        .map(|t| if t.length_squared() < 0.5 { Vec3::Z } else { t })
        .collect()
}

/// Normals perpendicular to `t`: minimum twist (parallel transport) or Z up.
pub fn poly_normals(t: &[Vec3], cyclic: bool, z_up: bool) -> Vec<Vec3> {
    let n = t.len();
    if n == 0 {
        return Vec::new();
    }
    let z_normal = |t: Vec3| {
        let c = t.cross(Vec3::Z);
        if c.length_squared() < 1e-10 {
            Vec3::X
        } else {
            c.normalize()
        }
    };
    if z_up {
        return t.iter().map(|&x| z_normal(x)).collect();
    }
    let mut out = Vec::with_capacity(n);
    out.push(z_normal(t[0]));
    for i in 1..n {
        let prev = out[i - 1];
        let q = Quat::from_rotation_arc(t[i - 1], t[i]);
        let mut nn = q.rotate(prev);
        nn = (nn - t[i] * nn.dot(t[i])).normalize();
        if nn.length_squared() < 0.5 {
            nn = z_normal(t[i]);
        }
        out.push(nn);
    }
    if cyclic && n > 2 {
        // Spread the twist mismatch at the seam over the whole curve.
        let q = Quat::from_rotation_arc(t[n - 1], t[0]);
        let end = q.rotate(out[n - 1]);
        let start = out[0];
        let mut angle = end.cross(start).dot(t[0]).atan2(end.dot(start));
        if !angle.is_finite() {
            angle = 0.0;
        }
        for (i, nn) in out.iter_mut().enumerate() {
            let a = -angle * i as f32 / n as f32;
            *nn = Quat::from_axis_angle(t[i], -a).rotate(*nn).normalize();
        }
    }
    out
}

fn bezier(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let u = 1.0 - t;
    p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
}

fn catmull(p: [Vec3; 4], t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    let w = [-t + 2.0 * t2 - t3, 2.0 - 5.0 * t2 + 3.0 * t3, t + 4.0 * t2 - 3.0 * t3, -t2 + t3];
    (p[0] * w[0] + p[1] * w[1] + p[2] * w[2] + p[3] * w[3]) * 0.5
}

/// B-spline basis functions of `order` at `u` (Cox-de Boor).
fn nurbs_basis(knots: &[f32], order: usize, u: f32, n_ctrl: usize) -> Vec<f32> {
    let m = knots.len() - 1;
    let mut nb = vec![0.0f32; m];
    for (i, b) in nb.iter_mut().enumerate() {
        if knots[i] <= u && u < knots[i + 1] {
            *b = 1.0;
        }
    }
    // u at the very end belongs to the last non-empty span.
    if u >= knots[n_ctrl] {
        if let Some(i) = (0..m).rev().find(|&i| knots[i] < knots[i + 1] && i < n_ctrl) {
            nb.iter_mut().for_each(|b| *b = 0.0);
            nb[i] = 1.0;
        }
    }
    for k in 2..=order {
        for i in 0..m.saturating_sub(k - 1) {
            let mut v = 0.0;
            let d1 = knots[i + k - 1] - knots[i];
            if d1 > 0.0 {
                v += (u - knots[i]) / d1 * nb[i];
            }
            let d2 = knots[i + k] - knots[i + 1];
            if d2 > 0.0 {
                v += (knots[i + k] - u) / d2 * nb[i + 1];
            }
            nb[i] = v;
        }
    }
    nb.truncate(n_ctrl);
    nb
}

impl Curves {
    pub fn curves_num(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    pub fn points_num(&self) -> usize {
        self.positions.len()
    }

    pub fn points(&self, c: usize) -> std::ops::Range<usize> {
        self.offsets[c] as usize..self.offsets[c + 1] as usize
    }

    pub fn size(&self, d: Domain) -> usize {
        match d {
            Domain::Point => self.points_num(),
            Domain::Curve => self.curves_num(),
            _ => 0,
        }
    }

    /// The curve each point belongs to.
    pub fn point_curve(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.points_num());
        for c in 0..self.curves_num() {
            out.extend(std::iter::repeat_n(c, self.points(c).len()));
        }
        out
    }

    fn attr_or<T>(&self, name: &str, d: Domain, i: usize, def: T, f: impl Fn(Val) -> T) -> T {
        match self.attributes.get(name) {
            Some(a) if a.domain == d && i < a.data.len() => f(a.data.get(i)),
            _ => def,
        }
    }

    pub fn curve_type(&self, c: usize) -> i32 {
        self.attr_or("curve_type", Domain::Curve, c, CATMULL_ROM, |v| v.i())
    }

    pub fn cyclic(&self, c: usize) -> bool {
        self.attr_or("cyclic", Domain::Curve, c, false, |v| v.b())
    }

    pub fn resolution(&self, c: usize) -> usize {
        self.attr_or("resolution", Domain::Curve, c, 12, |v| v.i()).max(1) as usize
    }

    pub fn nurbs_order(&self, c: usize) -> usize {
        self.attr_or("nurbs_order", Domain::Curve, c, 4, |v| v.i()).max(1) as usize
    }

    pub fn radius(&self, p: usize) -> f32 {
        self.attr_or("radius", Domain::Point, p, 1.0, |v| v.f())
    }

    pub fn tilt(&self, p: usize) -> f32 {
        self.attr_or("tilt", Domain::Point, p, 0.0, |v| v.f())
    }

    /// Ensure a built-in attribute exists (filled with `def`).
    pub fn ensure(&mut self, name: &str, d: Domain, def: Val) -> &mut AttrData {
        let n = self.size(d);
        let ok = matches!(self.attributes.get(name), Some(a) if a.domain == d && a.data.len() == n);
        if !ok {
            let ty = AttrType::of_val(&def);
            self.attributes.set(name, d, AttrData::from_vals(ty, &vec![def; n]));
        }
        &mut self.attributes.get_mut(name).unwrap().data
    }

    pub fn set_curve_type(&mut self, c: usize, t: i32) {
        self.ensure("curve_type", Domain::Curve, Val::Int(CATMULL_ROM)).set(c, Val::Int(t));
    }

    pub fn set_cyclic(&mut self, c: usize, v: bool) {
        self.ensure("cyclic", Domain::Curve, Val::Bool(false)).set(c, Val::Bool(v));
    }

    pub fn set_resolution(&mut self, c: usize, r: i32) {
        self.ensure("resolution", Domain::Curve, Val::Int(12)).set(c, Val::Int(r));
    }

    /// Append a curve with `pts`; existing attributes are padded with
    /// defaults. Returns the new curve's index.
    pub fn add_curve(&mut self, pts: &[Vec3], ty: i32, cyclic: bool) -> usize {
        let (np, nc) = (self.points_num(), self.curves_num());
        self.positions.extend_from_slice(pts);
        self.offsets.push(self.positions.len() as u32);
        for a in &mut self.attributes.list {
            match a.domain {
                Domain::Point => a.data.resize(np + pts.len()),
                Domain::Curve => a.data.resize(nc + 1),
                _ => {}
            }
        }
        self.set_curve_type(nc, ty);
        self.set_cyclic(nc, cyclic);
        nc
    }

    pub fn point_vals(&self, name: &str, def: Val) -> Vec<Val> {
        match self.attributes.get(name) {
            Some(a) if a.domain == Domain::Point && a.data.len() == self.points_num() => a.data.to_vals(),
            _ => vec![def; self.points_num()],
        }
    }

    /// Bezier handles (left, right) of curve `c`, computing automatic and
    /// vector handles from the neighbours.
    pub fn handles(&self, c: usize) -> (Vec<Vec3>, Vec<Vec3>) {
        let r = self.points(c);
        let p = &self.positions[r.clone()];
        let n = p.len();
        let cyclic = self.cyclic(c);
        let has = |name: &str| matches!(self.attributes.get(name), Some(a) if a.data.len() == self.points_num());
        let types = |name: &str| -> Vec<i32> {
            if has(name) {
                r.clone().map(|i| self.attributes.get(name).unwrap().data.get(i).i()).collect()
            } else if has("handle_left") || has("handle_right") {
                vec![HANDLE_FREE; n]
            } else {
                vec![HANDLE_AUTO; n]
            }
        };
        let (tl, tr) = (types("handle_type_left"), types("handle_type_right"));
        let read = |name: &str| -> Vec<Vec3> { if has(name) { r.clone().map(|i| self.attributes.get(name).unwrap().data.get(i).v()).collect() } else { p.to_vec() } };
        let (mut left, mut right) = (read("handle_left"), read("handle_right"));
        for i in 0..n {
            let prev = if i > 0 {
                p[i - 1]
            } else if cyclic {
                p[n - 1]
            } else if n > 1 {
                p[0] * 2.0 - p[1]
            } else {
                p[0]
            };
            let next = if i + 1 < n {
                p[i + 1]
            } else if cyclic {
                p[0]
            } else if n > 1 {
                p[i] * 2.0 - p[i - 1]
            } else {
                p[i]
            };
            if tl[i] == HANDLE_AUTO || tr[i] == HANDLE_AUTO {
                let (dp, dn) = (p[i] - prev, next - p[i]);
                let lp = if dp.length() > 0.0 { dp.length() } else { 1.0 };
                let ln = if dn.length() > 0.0 { dn.length() } else { 1.0 };
                let dir = dn / ln + dp / lp;
                let len = dir.length() * 2.5614;
                let (l, rr) = if len != 0.0 { (p[i] - dir * (lp / len), p[i] + dir * (ln / len)) } else { (p[i] - dp / 3.0, p[i] + dn / 3.0) };
                if tl[i] == HANDLE_AUTO {
                    left[i] = l;
                }
                if tr[i] == HANDLE_AUTO {
                    right[i] = rr;
                }
            }
            if tl[i] == HANDLE_VECTOR {
                left[i] = p[i] + (prev - p[i]) / 3.0;
            }
            if tr[i] == HANDLE_VECTOR {
                right[i] = p[i] + (next - p[i]) / 3.0;
            }
        }
        (left, right)
    }

    /// Number of evaluated points of curve `c`.
    pub fn evaluated_len(&self, c: usize) -> usize {
        self.evaluate_positions(c).0.len()
    }

    fn evaluate_positions(&self, c: usize) -> (Vec<Vec3>, Vec<(usize, usize, f32)>) {
        let r = self.points(c);
        let base = r.start;
        let p = &self.positions[r.clone()];
        let n = p.len();
        let cyclic = self.cyclic(c);
        if n == 0 {
            return (Vec::new(), Vec::new());
        }
        let ty = self.curve_type(c);
        if n == 1 || ty == POLY {
            return (p.to_vec(), (0..n).map(|i| (base + i, base + i, 0.0)).collect());
        }
        let res = self.resolution(c);
        let segs = if cyclic { n } else { n - 1 };
        let mut pos = Vec::new();
        let mut src = Vec::new();
        match ty {
            BEZIER => {
                let (left, right) = self.handles(c);
                for s in 0..segs {
                    let j = (s + 1) % n;
                    for k in 0..res {
                        let t = k as f32 / res as f32;
                        pos.push(bezier(p[s], right[s], left[j], p[j], t));
                        src.push((base + s, base + j, t));
                    }
                }
            }
            NURBS => {
                let order = self.nurbs_order(c).min(n).max(2);
                let weights: Vec<f32> = r.clone().map(|i| self.attr_or("nurbs_weight", Domain::Point, i, 1.0, |v| v.f())).collect();
                let endpoint = self.attr_or("knots_mode", Domain::Curve, c, 0, |v| v.i()) & 1 != 0;
                // Cyclic: wrap order-1 points.
                let ctrl: Vec<usize> = if cyclic { (0..n + order - 1).map(|i| i % n).collect() } else { (0..n).collect() };
                let m = ctrl.len();
                let knots: Vec<f32> = (0..m + order)
                    .map(|i| {
                        if endpoint && !cyclic {
                            (i as i64 - (order as i64 - 1)).clamp(0, (m - order + 1) as i64) as f32
                        } else {
                            i as f32
                        }
                    })
                    .collect();
                let (u0, u1) = (knots[order - 1], knots[m]);
                let count = segs * res + usize::from(!cyclic);
                for k in 0..count {
                    let t = k as f32 / (segs * res) as f32;
                    let u = u0 + (u1 - u0) * t;
                    let b = nurbs_basis(&knots, order, u, m);
                    let mut acc = Vec3::ZERO;
                    let mut wsum = 0.0;
                    let mut best = (0usize, 0.0f32);
                    for (i, &bi) in b.iter().enumerate() {
                        let w = bi * weights[ctrl[i]];
                        acc += p[ctrl[i]] * w;
                        wsum += w;
                        if w > best.1 {
                            best = (ctrl[i], w);
                        }
                    }
                    pos.push(if wsum > 0.0 { acc / wsum } else { p[0] });
                    let f = t * segs as f32;
                    let a = (f.floor() as usize).min(segs - 1);
                    src.push((base + a % n, base + (a + 1) % n, f - a as f32));
                }
                return (pos, src);
            }
            _ => {
                // Catmull-Rom.
                let get = |i: isize| -> Vec3 {
                    if cyclic {
                        p[i.rem_euclid(n as isize) as usize]
                    } else if i < 0 {
                        p[0] * 2.0 - p[1]
                    } else if i as usize >= n {
                        p[n - 1] * 2.0 - p[n - 2]
                    } else {
                        p[i as usize]
                    }
                };
                for s in 0..segs {
                    let si = s as isize;
                    let q = [get(si - 1), get(si), get(si + 1), get(si + 2)];
                    for k in 0..res {
                        let t = k as f32 / res as f32;
                        pos.push(catmull(q, t));
                        src.push((base + s, base + (s + 1) % n, t));
                    }
                }
            }
        }
        if !cyclic {
            pos.push(p[n - 1]);
            src.push((base + n - 1, base + n - 1, 0.0));
        }
        (pos, src)
    }

    /// Evaluate curve `c` with tangents, normals (tilt applied) and lengths.
    pub fn evaluate(&self, c: usize) -> Evaluated {
        let (positions, src) = self.evaluate_positions(c);
        let cyclic = self.cyclic(c) && positions.len() > 1;
        let tangents = poly_tangents(&positions, cyclic);
        let z_up = self.attr_or("normal_mode", Domain::Curve, c, 0, |v| v.i()) == 1;
        let mut normals = poly_normals(&tangents, cyclic, z_up);
        let tilt = self.point_vals("tilt", Val::Float(0.0));
        if self.attributes.contains("tilt") {
            for (i, nn) in normals.iter_mut().enumerate() {
                let (a, b, f) = src[i];
                let t = tilt[a].f() * (1.0 - f) + tilt[b].f() * f;
                if t != 0.0 {
                    *nn = Quat::from_axis_angle(tangents[i], t).rotate(*nn);
                }
            }
        }
        let mut lengths = Vec::with_capacity(positions.len() + 1);
        let mut acc = 0.0;
        lengths.push(0.0);
        for i in 1..positions.len() {
            acc += positions[i].distance(positions[i - 1]);
            lengths.push(acc);
        }
        if cyclic {
            acc += positions[positions.len() - 1].distance(positions[0]);
            lengths.push(acc);
        }
        Evaluated { positions, tangents, normals, src, cyclic, lengths }
    }

    /// Interpolate point values onto the evaluated points of curve `c`.
    pub fn interp_evaluated(&self, ev: &Evaluated, vals: &[Val]) -> Vec<Val> {
        ev.src.iter().map(|&(a, b, f)| if f == 0.0 { vals[a] } else { AttrType::of_val(&vals[a]).convert(interp(&[(vals[a], 1.0 - f), (vals[b], f)])) }).collect()
    }

    pub fn length(&self, c: usize) -> f32 {
        self.evaluate(c).total_length()
    }

    /// Move values between the point and curve domains.
    pub fn adapt(&self, vals: &[Val], from: Domain, to: Domain) -> Vec<Val> {
        if from == to {
            return vals.to_vec();
        }
        let def = vals.first().map(|v| AttrType::of_val(v).default_val()).unwrap_or(Val::Float(0.0));
        match (from, to) {
            (Domain::Curve, Domain::Point) => self.point_curve().into_iter().map(|c| vals.get(c).copied().unwrap_or(def)).collect(),
            (Domain::Point, Domain::Curve) => (0..self.curves_num()).map(|c| mix(self.points(c).map(|i| vals[i]), false).unwrap_or(def)).collect(),
            _ => vec![def; self.size(to)],
        }
    }

    /// Keep the curves `keep` (in that order, duplicates allowed).
    pub fn gather_curves(&self, keep: &[usize]) -> Curves {
        let mut out = Curves { attributes: self.attributes.clone(), ..Default::default() };
        let mut pmap = Vec::new();
        for &c in keep {
            for i in self.points(c) {
                out.positions.push(self.positions[i]);
                pmap.push(i);
            }
            out.offsets.push(out.positions.len() as u32);
        }
        out.attributes.gather_domain(Domain::Point, &pmap);
        out.attributes.gather_domain(Domain::Curve, keep);
        out
    }

    /// Keep the points `keep` of each curve (`keep[c]` lists curve c's
    /// point indices); curves left empty are removed.
    pub fn gather_points(&self, keep: &[Vec<usize>]) -> Curves {
        let mut out = Curves { attributes: self.attributes.clone(), ..Default::default() };
        let mut pmap = Vec::new();
        let mut cmap = Vec::new();
        for (c, pts) in keep.iter().enumerate() {
            if pts.is_empty() {
                continue;
            }
            for &i in pts {
                out.positions.push(self.positions[i]);
                pmap.push(i);
            }
            out.offsets.push(out.positions.len() as u32);
            cmap.push(c);
        }
        out.attributes.gather_domain(Domain::Point, &pmap);
        out.attributes.gather_domain(Domain::Curve, &cmap);
        out
    }

    /// Append another curves geometry.
    pub fn join(&mut self, o: &Curves) {
        let (sp, sc) = (self.points_num(), self.curves_num());
        let (op, oc) = (o.points_num(), o.curves_num());
        let had = self.attributes.names();
        self.attributes.join(&o.attributes, |d| match d {
            Domain::Point => (sp, op),
            Domain::Curve => (sc, oc),
            _ => (0, 0),
        });
        for (name, d, def) in BUILTIN_DEFAULTS {
            let (sh, oh) = (had.iter().any(|h| h == name), o.attributes.contains(name));
            if let Some(a) = self.attributes.get_mut(name) {
                let (s, on) = if d == Domain::Point { (sp, op) } else { (sc, oc) };
                if !sh {
                    (0..s).for_each(|i| a.data.set(i, def));
                }
                if !oh {
                    (s..s + on).for_each(|i| a.data.set(i, def));
                }
            }
        }
        self.positions.extend_from_slice(&o.positions);
        for &off in &o.offsets[1..] {
            self.offsets.push(off + sp as u32);
        }
    }

    /// Reverse the point order of the curves in `sel`.
    pub fn reverse(&mut self, sel: &[bool]) {
        let mut map: Vec<usize> = (0..self.points_num()).collect();
        for c in 0..self.curves_num() {
            if sel.get(c).copied().unwrap_or(false) {
                let r = self.points(c);
                map[r.clone()].reverse();
            }
        }
        self.positions = map.iter().map(|&i| self.positions[i]).collect();
        let swap = self.attributes.get("handle_left").cloned();
        self.attributes.gather_domain(Domain::Point, &map);
        // Reversed curves swap their left and right handles.
        if let (Some(_), Some(_)) = (swap, self.attributes.get("handle_right").cloned()) {
            let l = self.attributes.get("handle_left").unwrap().data.clone();
            let r = self.attributes.get("handle_right").unwrap().data.clone();
            let mut nl = l.clone();
            let mut nr = r.clone();
            for c in 0..self.curves_num() {
                if sel.get(c).copied().unwrap_or(false) {
                    for i in self.points(c) {
                        nl.set(i, r.get(i));
                        nr.set(i, l.get(i));
                    }
                }
            }
            self.attributes.get_mut("handle_left").unwrap().data = nl;
            self.attributes.get_mut("handle_right").unwrap().data = nr;
        }
    }
}
