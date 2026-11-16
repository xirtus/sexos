// SPDX-License-Identifier: GPL-3.0-only
//! What each sculpt brush does to the vertices under it, per dab and per
//! symmetry pass. The formulas follow Blender's `sculpt_brushes.cc` closely
//! enough that strokes feel the same; the strength scales are our own (a
//! Draw dab at strength 1 lifts the centre by a tenth of the radius).
use super::{flip, BrushKind, Dab, GrabPass, Region, SculptPlane, SculptSession, Stroke};
use crate::math::{Mat3, Vec3};
use crate::mesh::Mesh;

/// Draw-like brushes move the centre this fraction of the radius per dab at
/// full strength.
pub const DRAW_SCALE: f32 = 0.1;

fn proj_plane(v: Vec3, n: Vec3) -> Vec3 {
    v - n * v.dot(n)
}

/// Run one dab over every symmetry pass; returns the changed vertices.
pub(crate) fn apply(st: &mut Stroke, m: &mut Mesh, ss: &mut SculptSession, d: &Dab) -> Vec<u32> {
    let passes = st.symmetry.passes();
    let mut all = Vec::new();
    for (pi, f) in passes.iter().enumerate() {
        let pd = Dab { center: flip(d.center, *f), view: flip(d.view, *f), grab: flip(d.grab, *f), step: flip(d.step, *f), angle: if f.iter().filter(|x| **x).count() % 2 == 1 { -d.angle } else { d.angle }, ..*d };
        if st.first {
            init_pass(st, m, ss, &pd);
        }
        let changed = pass(st, m, ss, &pd, pi);
        if changed.is_empty() {
            continue;
        }
        if st.brush.kind.deforms() {
            ss.pbvh.mark_verts(&changed);
            ss.pbvh.refit(&m.positions);
            ss.update_normals(m, &changed);
        }
        all.extend(changed);
    }
    all.sort_unstable();
    all.dedup();
    all
}

/// First dab of a pass: what auto-masking compares against.
fn init_pass(st: &mut Stroke, m: &Mesh, ss: &mut SculptSession, d: &Dab) {
    let near = ss.nearest_vert(m, d.center, d.radius.max(1e-6));
    let comp = near.map(|v| ss.component[v as usize]);
    let set = near.and_then(|v| ss.vert_faces(v).first().map(|&f| m.face_set(f as usize)));
    st.auto.push((comp, set));
}

/// Whether auto-masking lets vertex `v` move in pass `pi`.
fn auto_ok(st: &Stroke, m: &Mesh, ss: &SculptSession, v: u32, pi: usize) -> bool {
    let (comp, set) = st.auto.get(pi).copied().unwrap_or((None, None));
    if st.brush.automask_topology {
        if let Some(c) = comp {
            if ss.component[v as usize] != c {
                return false;
            }
        }
    }
    if st.brush.automask_face_sets {
        if let Some(s) = set {
            if !ss.vert_faces(v).iter().all(|&f| m.face_set(f as usize) == s) {
                return false;
            }
        }
    }
    true
}

/// Vertices under the brush with their fade (falloff, mask, auto-masking,
/// front faces), measured on positions `pos`.
fn weights(st: &Stroke, m: &Mesh, ss: &mut SculptSession, d: &Dab, pi: usize, radius: f32, use_mask: bool) -> Vec<(u32, f32)> {
    let region = match st.brush.falloff_shape {
        super::FalloffShape::Sphere => Region::Sphere { center: d.center, radius },
        super::FalloffShape::Projected => Region::Cylinder { center: d.center, axis: d.view, radius },
    };
    let hits = ss.verts_in(m, &region);
    let mut out = Vec::with_capacity(hits.len());
    for (v, t) in hits {
        if ss.is_hidden(v) {
            continue;
        }
        if st.brush.front_faces_only && ss.normals[v as usize].dot(d.view) < 0.0 {
            continue;
        }
        if !auto_ok(st, m, ss, v, pi) {
            continue;
        }
        let mut f = st.brush.falloff(t) * d.pressure;
        if use_mask {
            f *= 1.0 - m.mask_at(v as usize).clamp(0.0, 1.0);
        }
        if f > 0.0 {
            out.push((v, f));
        }
    }
    out
}

/// Area normal and centre from the vertices within `frac` of the radius.
fn area(m: &Mesh, ss: &SculptSession, w: &[(u32, f32)], d: &Dab, frac: f32) -> (Vec3, Vec3) {
    let r2 = (d.radius * frac.clamp(0.05, 2.0)).powi(2);
    let (mut n, mut c, mut k) = (Vec3::ZERO, Vec3::ZERO, 0.0f32);
    for &(v, _) in w {
        let p = m.positions[v as usize];
        if (p - d.center).length_squared() <= r2 {
            n += ss.normals[v as usize];
            c += p;
            k += 1.0;
        }
    }
    if k == 0.0 {
        for &(v, _) in w {
            n += ss.normals[v as usize];
            c += m.positions[v as usize];
            k += 1.0;
        }
    }
    let n = if n.length_squared() > 1e-20 { n.normalize() } else { d.view };
    let c = if k > 0.0 { c / k } else { d.center };
    (n, c)
}

/// The brush plane (point, normal) for plane brushes.
fn plane(st: &Stroke, area_n: Vec3, area_c: Vec3, d: &Dab) -> (Vec3, Vec3) {
    let axis = |a: Vec3| if a.dot(area_n) < 0.0 { -a } else { a };
    let n = match st.brush.plane {
        SculptPlane::Area => area_n,
        SculptPlane::View => d.view,
        SculptPlane::X => axis(Vec3::X),
        SculptPlane::Y => axis(Vec3::Y),
        SculptPlane::Z => axis(Vec3::Z),
    };
    (area_c, n)
}

fn sign(st: &Stroke, d: &Dab) -> f32 {
    if st.brush.invert != d.invert {
        -1.0
    } else {
        1.0
    }
}

/// Write new positions (respecting axis locks); returns the changed ones.
fn write(st: &mut Stroke, m: &mut Mesh, new: Vec<(u32, Vec3)>) -> Vec<u32> {
    let lock = st.symmetry.lock;
    let mut out = Vec::with_capacity(new.len());
    for (v, mut p) in new {
        let old = m.positions[v as usize];
        if lock[0] {
            p.x = old.x;
        }
        if lock[1] {
            p.y = old.y;
        }
        if lock[2] {
            p.z = old.z;
        }
        if p != old && p.x.is_finite() && p.y.is_finite() && p.z.is_finite() {
            m.positions[v as usize] = p;
            if let Some(t) = st.touched.get_mut(v as usize) {
                *t = true;
            }
            out.push(v);
        }
    }
    out
}

fn laplacian(m: &Mesh, ss: &SculptSession, v: u32) -> Vec3 {
    let p = m.positions[v as usize];
    let nb = ss.neighbors(v);
    let on_boundary = ss.boundary.get(v as usize).copied().unwrap_or(false);
    let (mut s, mut k) = (Vec3::ZERO, 0.0f32);
    for &u in nb {
        if on_boundary && !ss.boundary[u as usize] {
            continue;
        }
        s += m.positions[u as usize];
        k += 1.0;
    }
    if k == 0.0 {
        return Vec3::ZERO;
    }
    s / k - p
}

/// Regularized Kelvinlet displacement for a force `f` at offset `r`
/// (de Goes and James 2017), scaled so the centre moves by exactly `f`.
pub fn kelvinlet(r: Vec3, f: Vec3, eps: f32) -> Vec3 {
    let nu = 0.4f32; // Poisson ratio (Blender's default volume preservation)
    let a = 1.0;
    let b = a / (4.0 * (1.0 - nu));
    let re = (r.length_squared() + eps * eps).sqrt();
    let re3 = re * re * re;
    let u = f * ((a - b) / re + a * eps * eps / (2.0 * re3)) + r * (b / re3 * r.dot(f));
    let u0 = (a - b) / eps + a / (2.0 * eps);
    u / u0
}

fn pass(st: &mut Stroke, m: &mut Mesh, ss: &mut SculptSession, d: &Dab, pi: usize) -> Vec<u32> {
    use BrushKind as K;
    let kind = st.brush.kind;
    let r = d.radius.max(1e-6);
    let s = st.brush.strength;
    let dir = sign(st, d);
    if kind.is_grab_like() {
        return grab_pass(st, m, ss, d, pi);
    }
    match kind {
        K::Mask => {
            let w = weights(st, m, ss, d, pi, r, false);
            if m.mask.len() != m.positions.len() {
                m.mask.resize(m.positions.len(), 0.0);
            }
            let mut changed = Vec::new();
            if d.relax {
                // Shift: smooth the mask.
                let new: Vec<(u32, f32)> = w
                    .iter()
                    .map(|&(v, f)| {
                        let nb = ss.neighbors(v);
                        let avg = if nb.is_empty() { m.mask[v as usize] } else { nb.iter().map(|&u| m.mask[u as usize]).sum::<f32>() / nb.len() as f32 };
                        (v, m.mask[v as usize] + (avg - m.mask[v as usize]) * (f * s).min(1.0))
                    })
                    .collect();
                for (v, x) in new {
                    if m.mask[v as usize] != x {
                        m.mask[v as usize] = x;
                        changed.push(v);
                    }
                }
            } else {
                for (v, f) in w {
                    let old = m.mask[v as usize];
                    let x = (old + f * s * dir).clamp(0.0, 1.0);
                    if x != old {
                        m.mask[v as usize] = x;
                        changed.push(v);
                    }
                }
            }
            changed
        }
        K::DrawFaceSets => {
            let w = weights(st, m, ss, d, pi, r, false);
            if m.face_sets.len() != m.faces.len() {
                m.face_sets.resize(m.faces.len(), 1);
            }
            let set = if dir < 0.0 { st.auto.get(pi).and_then(|a| a.1).unwrap_or(st.face_set) } else { st.face_set };
            let mut changed = Vec::new();
            for (v, f) in w {
                if f < 0.25 {
                    continue;
                }
                for &fi in ss.vert_faces(v) {
                    if m.face_sets[fi as usize] != set && !m.is_face_hidden(fi as usize) {
                        m.face_sets[fi as usize] = set;
                        changed.push(v);
                    }
                }
            }
            changed
        }
        K::Simplify => Vec::new(),
        _ => {
            let w = weights(st, m, ss, d, pi, r, true);
            if w.is_empty() {
                return Vec::new();
            }
            let frac = if kind.uses_plane() { st.brush.area_radius } else { st.brush.normal_radius };
            let (an, ac) = area(m, ss, &w, d, frac);
            let new = deform(st, m, ss, d, &w, an, ac, s, dir);
            write(st, m, new)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn deform(st: &mut Stroke, m: &Mesh, ss: &SculptSession, d: &Dab, w: &[(u32, f32)], an: Vec3, ac: Vec3, s: f32, dir: f32) -> Vec<(u32, Vec3)> {
    use BrushKind as K;
    let r = d.radius;
    let kind = st.brush.kind;
    let pos = |v: u32| m.positions[v as usize];
    let mut out = Vec::with_capacity(w.len());
    match kind {
        K::Draw => {
            let off = an * (r * DRAW_SCALE * s * dir);
            out.extend(w.iter().map(|&(v, f)| (v, pos(v) + off * f)));
        }
        K::DrawSharp => {
            // Original coordinates and normal: the stroke does not drift.
            let off = an * (r * DRAW_SCALE * s * dir);
            for &(v, _) in w {
                let o = st.orig.get(v as usize).copied().unwrap_or(pos(v));
                let t = (o - d.center).length() / r;
                let f = st.brush.falloff(t) * d.pressure * (1.0 - m.mask_at(v as usize));
                out.push((v, pos(v) + off * f));
            }
        }
        K::Inflate => {
            for &(v, f) in w {
                out.push((v, pos(v) + ss.normals[v as usize] * (r * DRAW_SCALE * s * dir * f)));
            }
        }
        K::Blob | K::Crease => {
            let off = an * (r * DRAW_SCALE * s * dir);
            let pinch = st.brush.pinch.clamp(0.0, 1.0) * if kind == K::Blob { 0.5 } else { 1.0 };
            for &(v, f) in w {
                let p = pos(v);
                let to_c = proj_plane(d.center - p, an);
                let pinch_v = if kind == K::Blob { to_c * (pinch * s * f * 0.5) } else { to_c * (pinch * s * f * 0.5) };
                out.push((v, p + off * f + pinch_v));
            }
        }
        K::Smooth => {
            for &(v, f) in w {
                let a = (f * s).min(1.0);
                let l = laplacian(m, ss, v);
                // Ctrl: Enhance Details pushes away from the average.
                out.push((v, pos(v) + l * (a * dir)));
            }
        }
        K::SlideRelax => {
            if d.relax {
                for &(v, f) in w {
                    let n = ss.normals[v as usize];
                    out.push((v, pos(v) + proj_plane(laplacian(m, ss, v), n) * (f * s).min(1.0)));
                }
            } else {
                for &(v, f) in w {
                    let n = ss.normals[v as usize];
                    out.push((v, pos(v) + proj_plane(d.step, n) * (f * s * dir)));
                }
            }
        }
        K::Pinch => {
            for &(v, f) in w {
                let p = pos(v);
                out.push((v, p + proj_plane(d.center - p, an) * (0.5 * f * s * dir)));
            }
        }
        K::SnakeHook => {
            for &(v, f) in w {
                let p = pos(v);
                let pinch = proj_plane(d.center - p, an) * (st.brush.pinch * f * s * 0.1);
                out.push((v, p + d.step * (f * s) + pinch));
            }
        }
        K::Nudge => {
            let t = proj_plane(d.step, an);
            out.extend(w.iter().map(|&(v, f)| (v, pos(v) + t * (f * s))));
        }
        K::Layer => {
            if st.layer.len() < m.positions.len() {
                st.layer.resize(m.positions.len(), 0.0);
            }
            let h = st.brush.height;
            for &(v, f) in w {
                let l = &mut st.layer[v as usize];
                *l = (*l + f * s * dir * 0.25).clamp(-1.0, 1.0);
                let o = st.orig.get(v as usize).copied().unwrap_or(pos(v));
                let n = st.orig_normals.get(v as usize).copied().unwrap_or(ss.normals[v as usize]);
                let target = o + n * (*l * h);
                // Never pull a vertex back past where the stroke already put it.
                out.push((v, target));
            }
        }
        K::Clay | K::ClayThumb | K::Flatten | K::Fill | K::Scrape | K::ClayStrips | K::MultiplaneScrape => {
            let (pc, pn) = plane(st, an, ac, d);
            let offset = match kind {
                K::Clay => r * (0.25 + st.brush.plane_offset),
                K::ClayStrips => r * (0.15 + st.brush.plane_offset),
                K::ClayThumb => r * (0.1 + st.brush.plane_offset),
                _ => r * st.brush.plane_offset,
            } * dir;
            let p0 = pc + pn * offset;
            let trim = st.brush.plane_trim.map(|t| t * r);
            // Stroke direction in the plane (Clay Strips, Multi-plane Scrape).
            let mut tdir = proj_plane(d.step, pn);
            if tdir.length_squared() < 1e-20 {
                tdir = proj_plane(st.last_dir, pn);
            }
            if tdir.length_squared() < 1e-20 {
                tdir = pn.any_orthonormal();
            }
            let tdir = tdir.normalize();
            st.last_dir = tdir;
            let bdir = pn.cross(tdir);
            let half = st.brush.plane_angle * 0.5;
            let (n_left, n_right) = (Mat3::from_axis_angle(tdir, half).mul_vec(pn), Mat3::from_axis_angle(tdir, -half).mul_vec(pn));
            for &(v, f) in w {
                let p = pos(v);
                let (q0, qn) = if kind == K::MultiplaneScrape {
                    // The side of the stroke line picks the plane; the ridge
                    // runs along the stroke.
                    if (p - pc).dot(bdir) >= 0.0 {
                        (pc, n_right)
                    } else {
                        (pc, n_left)
                    }
                } else {
                    (p0, pn)
                };
                let side = (p - q0).dot(qn);
                let affect = match kind {
                    K::Clay | K::ClayStrips | K::ClayThumb => side * dir < 0.0,
                    K::Flatten => true,
                    K::Fill => (side < 0.0) == (dir > 0.0),
                    K::Scrape | K::MultiplaneScrape => (side > 0.0) == (dir > 0.0),
                    _ => true,
                };
                if !affect {
                    continue;
                }
                if trim.is_some_and(|t| side.abs() > t) {
                    continue;
                }
                let mut fade = f;
                if kind == K::ClayStrips {
                    // Square brush tip in the stroke frame.
                    let lp = p - d.center;
                    let (x, y) = (lp.dot(tdir).abs(), lp.dot(bdir).abs());
                    let t = x.max(y) / r;
                    if t >= 1.0 {
                        continue;
                    }
                    fade = st.brush.falloff(t) * d.pressure * (1.0 - m.mask_at(v as usize));
                }
                let target = p - qn * side;
                let k = if kind == K::Flatten && dir < 0.0 { -0.5 } else { 0.5 };
                out.push((v, p + (target - p) * (fade * s * k).clamp(-1.0, 1.0)));
            }
        }
        _ => {}
    }
    out
}

/// Grab, Thumb, Elastic Deform, Pose and Rotate: move from the original
/// positions by the drag since the stroke began.
fn grab_pass(st: &mut Stroke, m: &mut Mesh, ss: &mut SculptSession, d: &Dab, pi: usize) -> Vec<u32> {
    use BrushKind as K;
    let kind = st.brush.kind;
    let r = d.radius.max(1e-6);
    if st.grab.len() <= pi {
        let reach = if kind == K::ElasticDeform { r * 3.0 } else { r };
        let w = weights(st, m, ss, d, pi, reach, true);
        let (an, _) = area(m, ss, &w, d, st.brush.normal_radius);
        st.grab.push(GrabPass { center: d.center, normal: an, verts: w });
    }
    let g = st.grab[pi].clone();
    let s = st.brush.strength;
    let orig = |v: u32| st.orig.get(v as usize).copied().unwrap_or(m.positions[v as usize]);
    let mut new = Vec::with_capacity(g.verts.len());
    match kind {
        K::Grab => {
            for &(v, f) in &g.verts {
                new.push((v, orig(v) + d.grab * (f * s)));
            }
        }
        K::Thumb => {
            let t = proj_plane(d.grab, g.normal);
            for &(v, f) in &g.verts {
                new.push((v, orig(v) + t * (f * s)));
            }
        }
        K::ElasticDeform => {
            for &(v, _) in &g.verts {
                let o = orig(v);
                let mask = 1.0 - m.mask_at(v as usize);
                new.push((v, o + kelvinlet(o - g.center, d.grab * s, r) * (mask * d.pressure)));
            }
        }
        K::Pose => {
            let pivot = g.center - g.normal * r;
            let axis = g.normal.cross(d.grab);
            if axis.length_squared() > 1e-20 {
                let axis = axis.normalize();
                let angle = d.grab.length() / r;
                for &(v, f) in &g.verts {
                    let rot = Mat3::from_axis_angle(axis, angle * f * s);
                    new.push((v, pivot + rot.mul_vec(orig(v) - pivot)));
                }
            } else {
                new.extend(g.verts.iter().map(|&(v, _)| (v, orig(v))));
            }
        }
        K::Rotate => {
            for &(v, f) in &g.verts {
                let rot = Mat3::from_axis_angle(g.normal, d.angle * f * s);
                new.push((v, g.center + rot.mul_vec(orig(v) - g.center)));
            }
        }
        _ => {}
    }
    write(st, m, new)
}
