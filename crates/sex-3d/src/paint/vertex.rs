// SPDX-License-Identifier: GPL-3.0-only
//! Vertex Paint: brushes that paint the active color attribute (per vertex
//! or per face corner) with Blender's Draw, Blur, Average and Smear.
use super::{blend, mix4, BlendMode};
use crate::math::Vec3;
use crate::mesh::{AttrDomain, Mesh};
use crate::sculpt::{Curve, Dab, Region, SculptSession};
use crate::undo::Change;

/// Vertex Paint brushes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VpBrush {
    #[default]
    Draw,
    Blur,
    Average,
    Smear,
}

impl VpBrush {
    pub const ALL: [VpBrush; 4] = [VpBrush::Draw, VpBrush::Blur, VpBrush::Average, VpBrush::Smear];
    pub fn label(self) -> &'static str {
        match self {
            VpBrush::Draw => "Draw",
            VpBrush::Blur => "Blur",
            VpBrush::Average => "Average",
            VpBrush::Smear => "Smear",
        }
    }
}

/// Vertex Paint brush settings.
#[derive(Clone, Debug, PartialEq)]
pub struct VpSettings {
    pub brush: VpBrush,
    /// Linear RGB.
    pub color: [f32; 3],
    pub secondary: [f32; 3],
    pub strength: f32,
    /// Radius in pixels.
    pub radius: f32,
    pub blend: BlendMode,
    pub curve: Curve,
    pub front_faces_only: bool,
    /// Palette colors (linear).
    pub palette: Vec<[f32; 3]>,
}

impl Default for VpSettings {
    fn default() -> Self {
        VpSettings {
            brush: VpBrush::Draw,
            color: [1.0, 1.0, 1.0],
            secondary: [0.0, 0.0, 0.0],
            strength: 1.0,
            radius: 50.0,
            blend: BlendMode::Mix,
            curve: Curve::Smooth,
            front_faces_only: true,
            palette: vec![[1.0, 1.0, 1.0], [0.0, 0.0, 0.0], [0.8, 0.05, 0.05], [0.05, 0.6, 0.05], [0.05, 0.1, 0.8], [0.9, 0.7, 0.05]],
        }
    }
}

/// A running Vertex Paint stroke on one color attribute.
pub struct VpStroke {
    pub attr: usize,
    domain: AttrDomain,
    orig: Vec<[f32; 4]>,
    touched: Vec<bool>,
    /// Vertex to its elements (corners for corner attributes).
    ve_start: Vec<u32>,
    ve: Vec<u32>,
}

impl VpStroke {
    /// Start painting attribute `attr` of `m` (it must exist).
    pub fn new(m: &Mesh, attr: usize) -> Option<VpStroke> {
        let a = m.color_attributes.get(attr)?;
        let nv = m.positions.len();
        let (ve_start, ve) = match a.domain {
            AttrDomain::Point => ((0..=nv as u32).collect(), (0..nv as u32).collect()),
            AttrDomain::Corner => {
                let mut cnt = vec![0u32; nv + 1];
                for f in &m.faces {
                    for &v in &f.verts {
                        cnt[v as usize + 1] += 1;
                    }
                }
                for i in 0..nv {
                    cnt[i + 1] += cnt[i];
                }
                let mut fill = cnt.clone();
                let mut ve = vec![0u32; cnt[nv] as usize];
                let mut k = 0u32;
                for f in &m.faces {
                    for &v in &f.verts {
                        ve[fill[v as usize] as usize] = k;
                        fill[v as usize] += 1;
                        k += 1;
                    }
                }
                (cnt, ve)
            }
        };
        Some(VpStroke { attr, domain: a.domain, orig: a.data.clone(), touched: vec![false; a.data.len()], ve_start, ve })
    }

    fn elems(&self, v: u32) -> &[u32] {
        match (self.ve_start.get(v as usize), self.ve_start.get(v as usize + 1)) {
            (Some(&a), Some(&b)) => &self.ve[a as usize..b as usize],
            _ => &[],
        }
    }

    fn vert_color(&self, data: &[[f32; 4]], v: u32) -> [f32; 4] {
        let e = self.elems(v);
        if e.is_empty() {
            return [1.0; 4];
        }
        let mut c = [0.0; 4];
        for &i in e {
            let x = data.get(i as usize).copied().unwrap_or([1.0; 4]);
            for k in 0..4 {
                c[k] += x[k];
            }
        }
        c.map(|x| x / e.len() as f32)
    }

    /// One dab; returns the vertices whose colors changed.
    pub fn dab(&mut self, m: &mut Mesh, ss: &mut SculptSession, s: &VpSettings, d: &Dab) -> Vec<u32> {
        if self.attr >= m.color_attributes.len() {
            return Vec::new();
        }
        let hits = ss.verts_in(m, &Region::Cylinder { center: d.center, axis: d.view, radius: d.radius.max(1e-6) });
        let mut w: Vec<(u32, f32)> = Vec::with_capacity(hits.len());
        for (v, t) in hits {
            if ss.is_hidden(v) {
                continue;
            }
            if s.front_faces_only && ss.normals[v as usize].dot(d.view) < 0.0 {
                continue;
            }
            // Behind the surface under the pointer (another layer of the mesh).
            if (m.positions[v as usize] - d.center).dot(d.view) < -d.radius {
                continue;
            }
            let f = s.curve.eval(t) * d.pressure * s.strength;
            if f > 0.0 {
                w.push((v, f));
            }
        }
        if w.is_empty() {
            return Vec::new();
        }
        let data = m.color_attributes[self.attr].data.clone();
        let color = if d.invert { s.secondary } else { s.color };
        let avg = {
            let mut c = [0.0; 4];
            for &(v, _) in &w {
                let x = self.vert_color(&data, v);
                for k in 0..4 {
                    c[k] += x[k];
                }
            }
            c.map(|x| x / w.len() as f32)
        };
        let step = d.step;
        let mut writes: Vec<(u32, [f32; 4])> = Vec::new();
        for &(v, f) in &w {
            let target = match s.brush {
                VpBrush::Draw => None,
                VpBrush::Blur => {
                    let nb = ss.neighbors(v);
                    if nb.is_empty() {
                        continue;
                    }
                    let mut c = [0.0; 4];
                    for &u in nb {
                        let x = self.vert_color(&data, u);
                        for k in 0..4 {
                            c[k] += x[k];
                        }
                    }
                    Some(c.map(|x| x / nb.len() as f32))
                }
                VpBrush::Average => Some(avg),
                VpBrush::Smear => {
                    if step.length_squared() < 1e-20 {
                        continue;
                    }
                    let p = m.positions[v as usize];
                    let dir = step.normalize();
                    let behind = ss.neighbors(v).iter().copied().min_by(|&a, &b| (m.positions[a as usize] - p).dot(dir).total_cmp(&(m.positions[b as usize] - p).dot(dir)));
                    match behind {
                        Some(u) if (m.positions[u as usize] - p).dot(dir) < 0.0 => Some(self.vert_color(&data, u)),
                        _ => continue,
                    }
                }
            };
            for &e in self.elems(v) {
                let c = data.get(e as usize).copied().unwrap_or([1.0; 4]);
                let n = match target {
                    None => blend(s.blend, c, color, f),
                    Some(t) => mix4(c, t, f.min(1.0)),
                };
                writes.push((e, n));
            }
        }
        let attr = &mut m.color_attributes[self.attr];
        let mut changed = Vec::new();
        for (e, c) in writes {
            if let Some(x) = attr.data.get_mut(e as usize) {
                if *x != c {
                    *x = c;
                    if let Some(t) = self.touched.get_mut(e as usize) {
                        *t = true;
                    }
                }
            }
        }
        for &(v, _) in &w {
            changed.push(v);
        }
        changed
    }

    /// The undo record for scene mesh `mesh`.
    pub fn finish(self, m: &Mesh, mesh: usize) -> Vec<Change> {
        let Some(a) = m.color_attributes.get(self.attr) else { return Vec::new() };
        let mut elems = Vec::new();
        let mut values = Vec::new();
        for (i, t) in self.touched.iter().enumerate() {
            if *t && self.orig.get(i) != a.data.get(i) {
                elems.push(i as u32);
                values.push(self.orig[i]);
            }
        }
        let _ = self.domain;
        if elems.is_empty() {
            Vec::new()
        } else {
            vec![Change::Colors { mesh, attr: self.attr, elems, values }]
        }
    }
}

/// Shift+K: set every element of attribute `attr` (on visible faces) to
/// `color` blended with `mode` at `strength`.
pub fn fill(m: &mut Mesh, attr: usize, color: [f32; 3], mode: BlendMode, strength: f32) {
    let offsets = m.corner_offsets();
    let hidden: Vec<bool> = (0..m.faces.len()).map(|f| m.is_face_hidden(f)).collect();
    let mut vert_visible = vec![hidden.iter().all(|h| !h); m.positions.len()];
    for (f, face) in m.faces.iter().enumerate() {
        if !hidden[f] {
            for &v in &face.verts {
                vert_visible[v as usize] = true;
            }
        }
    }
    let Some(a) = m.color_attributes.get_mut(attr) else { return };
    match a.domain {
        AttrDomain::Point => {
            for (v, c) in a.data.iter_mut().enumerate() {
                if vert_visible.get(v).copied().unwrap_or(true) {
                    *c = blend(mode, *c, color, strength);
                }
            }
        }
        AttrDomain::Corner => {
            for f in 0..offsets.len().saturating_sub(1) {
                if hidden[f] {
                    continue;
                }
                for k in offsets[f]..offsets[f + 1] {
                    if let Some(c) = a.data.get_mut(k) {
                        *c = blend(mode, *c, color, strength);
                    }
                }
            }
        }
    }
}

/// The color under a surface point: the active attribute at the nearest
/// vertex of face `face` (the sample tool / color picker).
pub fn sample(m: &Mesh, attr: usize, face: usize, p: Vec3) -> Option<[f32; 4]> {
    let f = m.faces.get(face)?;
    let (c, _) = f.verts.iter().enumerate().map(|(i, &v)| (i, m.positions[v as usize].distance(p))).min_by(|a, b| a.1.total_cmp(&b.1))?;
    let offsets = m.corner_offsets();
    m.color_attributes.get(attr)?;
    Some(m.corner_color(attr, &offsets, face, c))
}
