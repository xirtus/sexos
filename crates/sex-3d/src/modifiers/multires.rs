// SPDX-License-Identifier: GPL-3.0-only
//! Multiresolution: a hierarchy of subdivision levels over the base mesh,
//! each holding a displacement from the subdivided level below it.
//!
//! Level `L` is `subdivide(level L-1) + disp[L]`, with level 0 the base mesh.
//! Displacements live in object space (Blender keeps them in tangent space,
//! so editing the base mesh drags details along slightly differently here).
//! Subdivision keeps the vertices of the level below first and in order, so
//! base vertex `i` is vertex `i` of every level.
use super::{decimate, subdivide_catmull_clark, subdivide_simple};
use crate::math::Vec3;
use crate::mesh::Mesh;
use std::collections::HashMap;

/// How a level was subdivided (Blender's Subdivide, Simple and Linear).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MultiresSubdiv {
    #[default]
    CatmullClark,
    Simple,
    Linear,
}

impl MultiresSubdiv {
    pub fn id(self) -> &'static str {
        match self {
            MultiresSubdiv::CatmullClark => "CATMULL_CLARK",
            MultiresSubdiv::Simple => "SIMPLE",
            MultiresSubdiv::Linear => "LINEAR",
        }
    }
    pub fn from_id(s: &str) -> Option<MultiresSubdiv> {
        [MultiresSubdiv::CatmullClark, MultiresSubdiv::Simple, MultiresSubdiv::Linear].into_iter().find(|k| k.id().eq_ignore_ascii_case(s))
    }
}

/// One level above the base.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MultiresLevel {
    pub kind: MultiresSubdiv,
    /// One offset per vertex of this level (empty: none yet).
    pub disp: Vec<Vec3>,
}

/// The levels of a Multires modifier.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MultiresData {
    pub levels: Vec<MultiresLevel>,
}

fn subdivide(m: &Mesh, kind: MultiresSubdiv) -> Mesh {
    match kind {
        MultiresSubdiv::CatmullClark => subdivide_catmull_clark(m, 1),
        MultiresSubdiv::Simple | MultiresSubdiv::Linear => subdivide_simple(m, 1),
    }
}

impl MultiresData {
    pub fn total_levels(&self) -> u32 {
        self.levels.len() as u32
    }

    /// The mesh at `level` (clamped to the levels there are).
    pub fn level_mesh(&self, base: &Mesh, level: u32) -> Mesh {
        let mut m = base.clone();
        for l in self.levels.iter().take(level as usize) {
            m = subdivide(&m, l.kind);
            if l.disp.len() == m.positions.len() {
                for (p, d) in m.positions.iter_mut().zip(&l.disp) {
                    *p += *d;
                }
            }
        }
        m.sanitize_attributes();
        m
    }

    /// Every level's positions, level 0 (the base) first.
    fn all_positions(&self, base: &Mesh) -> Vec<Vec<Vec3>> {
        let mut out = vec![base.positions.clone()];
        let mut m = base.clone();
        for l in &self.levels {
            m = subdivide(&m, l.kind);
            if l.disp.len() == m.positions.len() {
                for (p, d) in m.positions.iter_mut().zip(&l.disp) {
                    *p += *d;
                }
            }
            out.push(m.positions.clone());
        }
        out
    }

    /// Recompute every displacement so that level `l` has `shapes[l]` over
    /// `base` (`shapes[0]` is ignored: the base is what it is).
    fn set_shapes(&mut self, base: &Mesh, shapes: &[Vec<Vec3>]) {
        let mut m = base.clone();
        for (i, l) in self.levels.iter_mut().enumerate() {
            m = subdivide(&m, l.kind);
            match shapes.get(i + 1) {
                Some(s) if s.len() == m.positions.len() => {
                    l.disp = s.iter().zip(&m.positions).map(|(a, b)| *a - *b).collect();
                    m.positions.clone_from(s);
                }
                _ => {
                    if l.disp.len() == m.positions.len() {
                        for (p, d) in m.positions.iter_mut().zip(&l.disp) {
                            *p += *d;
                        }
                    }
                }
            }
        }
    }

    /// Add a level on top (the new level starts as the plain subdivision of
    /// the current top level).
    pub fn subdivide(&mut self, base: &Mesh, kind: MultiresSubdiv) {
        if self.levels.len() >= super::MAX_SUBDIVISION_LEVELS as usize {
            return;
        }
        let top = self.level_mesh(base, self.total_levels());
        let n = subdivide(&top, kind).positions.len();
        self.levels.push(MultiresLevel { kind, disp: vec![Vec3::ZERO; n] });
    }

    /// Store sculpted positions of `level` (after a stroke). Levels above
    /// keep their displacements, so their details ride on the change.
    pub fn store(&mut self, base: &Mesh, level: u32, positions: &[Vec3]) -> bool {
        if level == 0 || level > self.total_levels() {
            return false;
        }
        let below = self.level_mesh(base, level - 1);
        let l = &mut self.levels[level as usize - 1];
        let rest = subdivide(&below, l.kind);
        if rest.positions.len() != positions.len() {
            return false;
        }
        l.disp = positions.iter().zip(&rest.positions).map(|(a, b)| *a - *b).collect();
        true
    }

    /// Remove every level above `level`.
    pub fn delete_higher(&mut self, level: u32) {
        self.levels.truncate(level as usize);
    }

    /// Apply Base: move the base vertices to where the top level puts them,
    /// keeping every level's shape. Returns the new base positions.
    pub fn apply_base(&mut self, base: &Mesh) -> Vec<Vec3> {
        if self.levels.is_empty() {
            return base.positions.clone();
        }
        let shapes = self.all_positions(base);
        let top = shapes.last().cloned().unwrap_or_default();
        let mut nb = base.clone();
        for (i, p) in nb.positions.iter_mut().enumerate() {
            if let Some(t) = top.get(i) {
                *p = *t;
            }
        }
        // Catmull-Clark pulls corners inwards: push the base out by the
        // difference its own first subdivision makes, a few times.
        if self.levels[0].kind == MultiresSubdiv::CatmullClark {
            for _ in 0..4 {
                let s = subdivide(&nb, MultiresSubdiv::CatmullClark);
                for (i, p) in nb.positions.iter_mut().enumerate() {
                    if let (Some(t), Some(q)) = (top.get(i), s.positions.get(i)) {
                        *p += (*t - *q) * 0.5;
                    }
                }
            }
        }
        self.set_shapes(&nb, &shapes);
        nb.positions
    }

    /// Unsubdivide: rebuild a level below the base. Returns the new base
    /// mesh, or None when the base is not a subdivided grid.
    pub fn unsubdivide(&mut self, base: &Mesh) -> Option<Mesh> {
        let coarse = decimate::unsubdivide(base, 1);
        if coarse.positions.len() >= base.positions.len() || coarse.faces.is_empty() {
            return None;
        }
        let shapes = self.all_positions(base);
        let kind = MultiresSubdiv::Simple;
        let rest = subdivide(&coarse, kind);
        if rest.positions.len() != base.positions.len() {
            return None;
        }
        // Level 1 vertex -> base vertex, by position (coarse vertices are base
        // vertices, edge and face points sit on the old edge and face points
        // of a grid that came from simple subdivision; otherwise nearest).
        let map = match_points(&rest.positions, &base.positions)?;
        let mut new_shapes: Vec<Vec<Vec3>> = vec![coarse.positions.clone(), map.iter().map(|&i| base.positions[i]).collect()];
        // Higher levels: subdivide the reordered rest shapes on both sides and
        // match them up the same way.
        let mut old_rest = base.clone();
        let mut new_rest = coarse.clone();
        new_rest = subdivide(&new_rest, kind);
        new_rest.positions.clone_from(&new_shapes[1]);
        for (li, l) in self.levels.iter().enumerate() {
            old_rest = subdivide(&old_rest, l.kind);
            new_rest = subdivide(&new_rest, l.kind);
            let map = match_points(&new_rest.positions, &old_rest.positions)?;
            let shape = &shapes[li + 1];
            new_shapes.push(map.iter().map(|&i| shape.get(i).copied().unwrap_or(Vec3::ZERO)).collect());
        }
        let mut levels = vec![MultiresLevel { kind, disp: Vec::new() }];
        levels.extend(self.levels.iter().cloned());
        self.levels = levels;
        let mut nb = coarse;
        nb.sanitize_attributes();
        self.set_shapes(&nb, &new_shapes);
        Some(nb)
    }
}

/// For every point of `a`, the index of the nearest point of `b` (a
/// bijection is required: None if two points pick the same one).
fn match_points(a: &[Vec3], b: &[Vec3]) -> Option<Vec<usize>> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let (mut lo, mut hi) = (b[0], b[0]);
    for p in b {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let ext = hi - lo;
    let cell = (ext.x.max(ext.y).max(ext.z) / (b.len() as f32).cbrt().max(1.0)).max(1e-6);
    let key = |p: Vec3| (((p.x - lo.x) / cell).floor() as i64, ((p.y - lo.y) / cell).floor() as i64, ((p.z - lo.z) / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in b.iter().enumerate() {
        grid.entry(key(*p)).or_default().push(i);
    }
    let mut used = vec![false; b.len()];
    let mut out = Vec::with_capacity(a.len());
    for p in a {
        let k = key(*p);
        let mut best: Option<(f32, usize)> = None;
        for r in 0..4i64 {
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != r {
                            continue;
                        }
                        for &i in grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)).map_or(&[][..], |v| v.as_slice()) {
                            let d = (b[i] - *p).length_squared();
                            if !used[i] && best.is_none_or(|bb| d < bb.0) {
                                best = Some((d, i));
                            }
                        }
                    }
                }
            }
            if best.is_some() {
                break;
            }
        }
        let (_, i) = best?;
        used[i] = true;
        out.push(i);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn close(a: &[Vec3], b: &[Vec3], eps: f32) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.distance(*y) < eps)
    }

    #[test]
    fn levels_round_trip() {
        let base = primitives::cube(2.0);
        let mut md = MultiresData::default();
        md.subdivide(&base, MultiresSubdiv::CatmullClark);
        md.subdivide(&base, MultiresSubdiv::CatmullClark);
        assert_eq!(md.total_levels(), 2);
        let plain = subdivide_catmull_clark(&base, 2);
        let l2 = md.level_mesh(&base, 2);
        assert!(close(&l2.positions, &plain.positions, 1e-6));
        // Sculpt level 2: the stored shape comes back exactly.
        let mut sculpted = l2.positions.clone();
        sculpted[30] += Vec3::new(0.0, 0.0, 0.3);
        sculpted[40] -= Vec3::new(0.1, 0.0, 0.0);
        assert!(md.store(&base, 2, &sculpted));
        assert!(close(&md.level_mesh(&base, 2).positions, &sculpted, 1e-6));
        // Lower levels are untouched.
        assert!(close(&md.level_mesh(&base, 1).positions, &subdivide_catmull_clark(&base, 1).positions, 1e-6));
        // Sculpt level 1: level 2 keeps its detail on top.
        let mut l1 = md.level_mesh(&base, 1).positions;
        l1[0] += Vec3::new(0.0, 0.0, 0.5);
        assert!(md.store(&base, 1, &l1));
        let l2b = md.level_mesh(&base, 2).positions;
        let d_before = sculpted[30] - plain.positions[30];
        let rest2 = subdivide_catmull_clark(&md.level_mesh(&base, 1), 1).positions;
        assert!((l2b[30] - rest2[30]).distance(d_before) < 1e-5);
        // Apply Base keeps the top shape.
        let top = md.level_mesh(&base, 2).positions;
        let nb_pos = md.apply_base(&base);
        let mut nb = base.clone();
        nb.positions = nb_pos;
        assert!(close(&md.level_mesh(&nb, 2).positions, &top, 1e-4));
        assert!(nb.positions[0].distance(base.positions[0]) > 1e-3);
        // Delete higher.
        md.delete_higher(1);
        assert_eq!(md.total_levels(), 1);
    }

    #[test]
    fn simple_and_linear_keep_the_shape() {
        let base = primitives::cube(2.0);
        let mut md = MultiresData::default();
        md.subdivide(&base, MultiresSubdiv::Simple);
        md.subdivide(&base, MultiresSubdiv::Linear);
        let m = md.level_mesh(&base, 2);
        assert_eq!(m.faces.len(), 6 * 16);
        assert!(m.positions.iter().all(|p| p.x.abs().max(p.y.abs()).max(p.z.abs()) > 0.999));
    }

    #[test]
    fn unsubdivide_adds_a_level_below() {
        let base = subdivide_simple(&primitives::grid(2, 2, 2.0), 1);
        let mut base = base;
        base.positions[12].z = 0.2;
        let mut md = MultiresData::default();
        md.subdivide(&base, MultiresSubdiv::CatmullClark);
        let mut top = md.level_mesh(&base, 1).positions;
        top[20].z += 0.1;
        md.store(&base, 1, &top);
        let shape0 = base.positions.clone();
        let shape1 = md.level_mesh(&base, 1);
        let nb = md.unsubdivide(&base).expect("grid unsubdivides");
        assert_eq!(nb.faces.len(), 4);
        assert_eq!(md.total_levels(), 2);
        // Level 1 is the old base, level 2 the old level 1, as point sets.
        let l1 = md.level_mesh(&nb, 1).positions;
        let mut a: Vec<[i64; 3]> = l1.iter().map(|p| [(p.x * 1e4) as i64, (p.y * 1e4) as i64, (p.z * 1e4) as i64]).collect();
        let mut b: Vec<[i64; 3]> = shape0.iter().map(|p| [(p.x * 1e4) as i64, (p.y * 1e4) as i64, (p.z * 1e4) as i64]).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        let l2 = md.level_mesh(&nb, 2);
        assert_eq!(l2.positions.len(), shape1.positions.len());
        let mut s2: Vec<f32> = l2.positions.iter().map(|p| p.z).collect();
        let mut s1: Vec<f32> = shape1.positions.iter().map(|p| p.z).collect();
        s2.sort_by(f32::total_cmp);
        s1.sort_by(f32::total_cmp);
        assert!(s1.iter().zip(&s2).all(|(x, y)| (x - y).abs() < 1e-4));
    }
}
