// SPDX-License-Identifier: GPL-3.0-only
//! Sculpt and paint data on a [`Mesh`]: color attributes (per vertex or per
//! face corner, Blender's `color_attributes`), the sculpt mask, face sets and
//! hidden faces.
//!
//! Every array here is optional: empty means "not there" (no mask, every face
//! in set 1, nothing hidden, no colors). Topology operations that go through
//! [`Mesh::remap_vertices`], [`Mesh::add_vertex_interp`] and [`Mesh::append`]
//! keep them in step; anything else can call [`Mesh::sanitize_attributes`]
//! to fit them to the current topology again.
use super::{Face, Mesh};

/// Where a color attribute stores its values (Blender's attribute domain).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AttrDomain {
    /// One value per vertex (Blender's "Vertex").
    Point,
    /// One value per face corner, faces in order (Blender's "Face Corner",
    /// the default for new color attributes).
    #[default]
    Corner,
}

impl AttrDomain {
    pub fn id(self) -> &'static str {
        match self {
            AttrDomain::Point => "POINT",
            AttrDomain::Corner => "CORNER",
        }
    }
    pub fn from_id(s: &str) -> Option<AttrDomain> {
        match s {
            "POINT" => Some(AttrDomain::Point),
            "CORNER" => Some(AttrDomain::Corner),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            AttrDomain::Point => "Vertex",
            AttrDomain::Corner => "Face Corner",
        }
    }
}

/// A named RGBA color layer. Values are linear (Blender's float colors; its
/// byte colors are sRGB on disk but linear when read).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorAttribute {
    pub name: String,
    pub domain: AttrDomain,
    /// One entry per vertex or per face corner (faces in order).
    pub data: Vec<[f32; 4]>,
    /// Stored as 8-bit (Blender's "Byte Color"); only a label here.
    pub byte: bool,
}

pub const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

impl Mesh {
    /// Total number of face corners.
    pub fn corner_count(&self) -> usize {
        self.faces.iter().map(|f| f.verts.len()).sum()
    }

    /// Start of each face's corners in a corner-domain array (and the total
    /// at the end).
    pub fn corner_offsets(&self) -> Vec<usize> {
        let mut o = Vec::with_capacity(self.faces.len() + 1);
        let mut n = 0;
        for f in &self.faces {
            o.push(n);
            n += f.verts.len();
        }
        o.push(n);
        o
    }

    /// Add a color attribute filled with `fill`; returns its index. It
    /// becomes active (and the render default when it is the first one).
    pub fn add_color_attribute(&mut self, name: &str, domain: AttrDomain, fill: [f32; 4]) -> usize {
        let name = crate::scene::unique_name(if name.is_empty() { "Color" } else { name }, |n| self.color_attributes.iter().any(|a| a.name == n));
        let len = match domain {
            AttrDomain::Point => self.positions.len(),
            AttrDomain::Corner => self.corner_count(),
        };
        self.color_attributes.push(ColorAttribute { name, domain, data: vec![fill; len], byte: domain == AttrDomain::Corner });
        let i = self.color_attributes.len() - 1;
        self.active_color = i;
        if i == 0 {
            self.render_color = 0;
        }
        i
    }

    pub fn remove_color_attribute(&mut self, i: usize) {
        if i >= self.color_attributes.len() {
            return;
        }
        self.color_attributes.remove(i);
        let fix = |x: &mut usize| {
            if *x > i || (*x == i && *x > 0) {
                *x -= 1;
            }
        };
        fix(&mut self.active_color);
        fix(&mut self.render_color);
    }

    /// The active color attribute, if there is one.
    pub fn active_color_attribute(&self) -> Option<&ColorAttribute> {
        self.color_attributes.get(self.active_color)
    }

    /// Color of attribute `a` at face `f`, corner `c` (white when missing).
    pub fn corner_color(&self, a: usize, offsets: &[usize], f: usize, c: usize) -> [f32; 4] {
        let Some(attr) = self.color_attributes.get(a) else { return WHITE };
        match attr.domain {
            AttrDomain::Point => self.faces.get(f).and_then(|face| face.verts.get(c)).and_then(|&v| attr.data.get(v as usize)).copied().unwrap_or(WHITE),
            AttrDomain::Corner => offsets.get(f).and_then(|&o| attr.data.get(o + c)).copied().unwrap_or(WHITE),
        }
    }

    /// Per-vertex colors of attribute `a` (corner colors averaged).
    pub fn point_colors(&self, a: usize) -> Vec<[f32; 4]> {
        let n = self.positions.len();
        let Some(attr) = self.color_attributes.get(a) else { return vec![WHITE; n] };
        match attr.domain {
            AttrDomain::Point => (0..n).map(|v| attr.data.get(v).copied().unwrap_or(WHITE)).collect(),
            AttrDomain::Corner => {
                let mut sum = vec![[0.0f32; 4]; n];
                let mut cnt = vec![0u32; n];
                let mut k = 0;
                for f in &self.faces {
                    for &v in &f.verts {
                        if let Some(c) = attr.data.get(k) {
                            for i in 0..4 {
                                sum[v as usize][i] += c[i];
                            }
                            cnt[v as usize] += 1;
                        }
                        k += 1;
                    }
                }
                sum.into_iter().zip(cnt).map(|(s, c)| if c == 0 { WHITE } else { s.map(|x| x / c as f32) }).collect()
            }
        }
    }

    /// Mask value of vertex `v` (0 when there is no mask).
    pub fn mask_at(&self, v: usize) -> f32 {
        self.mask.get(v).copied().unwrap_or(0.0)
    }

    /// Face set of face `f` (1 when there are none).
    pub fn face_set(&self, f: usize) -> i32 {
        self.face_sets.get(f).copied().unwrap_or(1)
    }

    pub fn is_face_hidden(&self, f: usize) -> bool {
        self.hidden_faces.get(f).copied().unwrap_or(false)
    }

    /// Make the mask, face set, hidden and color arrays fit the topology:
    /// short or long per-vertex and per-face arrays are padded or cut,
    /// corner arrays that no longer match are reset to white.
    pub fn sanitize_attributes(&mut self) {
        let (nv, nf) = (self.positions.len(), self.faces.len());
        let nc = self.corner_count();
        if !self.mask.is_empty() {
            self.mask.resize(nv, 0.0);
        }
        if !self.face_sets.is_empty() {
            self.face_sets.resize(nf, 1);
        }
        if !self.hidden_faces.is_empty() {
            self.hidden_faces.resize(nf, false);
        }
        for a in &mut self.color_attributes {
            match a.domain {
                AttrDomain::Point => a.data.resize(nv, WHITE),
                AttrDomain::Corner => {
                    if a.data.len() != nc {
                        a.data = vec![WHITE; nc];
                    }
                }
            }
        }
        if self.active_color >= self.color_attributes.len() {
            self.active_color = 0;
        }
        if self.render_color >= self.color_attributes.len() {
            self.render_color = 0;
        }
    }

    pub(super) fn has_point_attrs(&self) -> bool {
        !self.mask.is_empty() || self.color_attributes.iter().any(|a| a.domain == AttrDomain::Point)
    }

    pub(super) fn pad_point_attrs(&mut self) {
        let n = self.positions.len();
        if !self.mask.is_empty() {
            self.mask.resize(n, 0.0);
        }
        for a in &mut self.color_attributes {
            if a.domain == AttrDomain::Point {
                a.data.resize(n, WHITE);
            }
        }
    }

    /// Push mask and point colors for a new vertex blended from `src`.
    pub(super) fn push_point_attrs_interp(&mut self, src: &[(usize, f32)]) {
        if !self.mask.is_empty() {
            let m = src.iter().map(|&(v, f)| self.mask.get(v).copied().unwrap_or(0.0) * f).sum::<f32>();
            self.mask.push(m.clamp(0.0, 1.0));
        }
        for a in &mut self.color_attributes {
            if a.domain != AttrDomain::Point {
                continue;
            }
            let mut c = [0.0f32; 4];
            if src.is_empty() {
                c = WHITE;
            }
            for &(v, f) in src {
                let s = a.data.get(v).copied().unwrap_or(WHITE);
                for i in 0..4 {
                    c[i] += s[i] * f;
                }
            }
            a.data.push(c);
        }
    }

    /// New vertex `v` takes old vertex `src[v]`'s mask and point colors.
    pub(super) fn remap_point_attrs(&mut self, src: &[Option<usize>]) {
        if !self.mask.is_empty() {
            let old = std::mem::take(&mut self.mask);
            self.mask = src.iter().map(|s| s.and_then(|i| old.get(i).copied()).unwrap_or(0.0)).collect();
        }
        for a in &mut self.color_attributes {
            if a.domain == AttrDomain::Point {
                let old = std::mem::take(&mut a.data);
                a.data = src.iter().map(|s| s.and_then(|i| old.get(i).copied()).unwrap_or(WHITE)).collect();
            }
        }
    }

    /// After [`Mesh::append`]: the other mesh's mask, face sets, hidden faces
    /// and colors for the vertices and faces just appended. Attributes match
    /// by name and domain; missing ones fill with defaults.
    pub(super) fn append_sculpt_data(&mut self, o: &Mesh, v_off: usize, f_off: usize) {
        let (nv, nf) = (self.positions.len(), self.faces.len());
        if !self.mask.is_empty() || !o.mask.is_empty() {
            self.mask.resize(v_off, 0.0);
            self.mask.extend((0..o.positions.len()).map(|v| o.mask_at(v)));
            self.mask.resize(nv, 0.0);
        }
        if !self.face_sets.is_empty() || !o.face_sets.is_empty() {
            self.face_sets.resize(f_off, 1);
            self.face_sets.extend((0..o.faces.len()).map(|f| o.face_set(f)));
            self.face_sets.resize(nf, 1);
        }
        if !self.hidden_faces.is_empty() || !o.hidden_faces.is_empty() {
            self.hidden_faces.resize(f_off, false);
            self.hidden_faces.extend((0..o.faces.len()).map(|f| o.is_face_hidden(f)));
            self.hidden_faces.resize(nf, false);
        }
        if self.color_attributes.is_empty() && o.color_attributes.is_empty() {
            return;
        }
        // Corners before the appended faces.
        let c_off: usize = self.faces[..f_off].iter().map(|f| f.verts.len()).sum();
        for a in &o.color_attributes {
            if !self.color_attributes.iter().any(|x| x.name == a.name && x.domain == a.domain) {
                let len = match a.domain {
                    AttrDomain::Point => v_off,
                    AttrDomain::Corner => c_off,
                };
                self.color_attributes.push(ColorAttribute { name: a.name.clone(), domain: a.domain, data: vec![WHITE; len], byte: a.byte });
            }
        }
        let o_offsets = o.corner_offsets();
        for a in &mut self.color_attributes {
            let other = o.color_attributes.iter().find(|x| x.name == a.name && x.domain == a.domain);
            match a.domain {
                AttrDomain::Point => {
                    a.data.resize(v_off, WHITE);
                    a.data.extend((0..o.positions.len()).map(|v| other.and_then(|x| x.data.get(v).copied()).unwrap_or(WHITE)));
                }
                AttrDomain::Corner => {
                    a.data.resize(c_off, WHITE);
                    let n = *o_offsets.last().unwrap_or(&0);
                    a.data.extend((0..n).map(|k| other.and_then(|x| x.data.get(k).copied()).unwrap_or(WHITE)));
                }
            }
        }
    }
}

/// Carries per-face and per-corner data through [`Mesh::remap_vertices`],
/// which drops faces and corners.
pub(super) struct FaceDataRemap {
    sets: Option<Vec<i32>>,
    hidden: Option<Vec<bool>>,
    /// (attribute index, old data) of corner-domain attributes.
    corners: Vec<(usize, Vec<[f32; 4]>)>,
    offsets: Vec<usize>,
    new_sets: Vec<i32>,
    new_hidden: Vec<bool>,
    new_corners: Vec<Vec<[f32; 4]>>,
}

impl FaceDataRemap {
    pub(super) fn take(m: &mut Mesh, faces: &[Face]) -> FaceDataRemap {
        let sets = (!m.face_sets.is_empty()).then(|| std::mem::take(&mut m.face_sets));
        let hidden = (!m.hidden_faces.is_empty()).then(|| std::mem::take(&mut m.hidden_faces));
        let mut offsets = Vec::new();
        let mut corners = Vec::new();
        if m.color_attributes.iter().any(|a| a.domain == AttrDomain::Corner) {
            let mut n = 0;
            for f in faces {
                offsets.push(n);
                n += f.verts.len();
            }
            offsets.push(n);
            for (i, a) in m.color_attributes.iter_mut().enumerate() {
                if a.domain == AttrDomain::Corner {
                    corners.push((i, std::mem::take(&mut a.data)));
                }
            }
        }
        let k = corners.len();
        FaceDataRemap { sets, hidden, corners, offsets, new_sets: Vec::new(), new_hidden: Vec::new(), new_corners: vec![Vec::new(); k] }
    }

    /// Old face `fi` survives keeping its corners `kept` (in order).
    pub(super) fn keep(&mut self, fi: usize, kept: &[usize]) {
        if let Some(s) = &self.sets {
            self.new_sets.push(s.get(fi).copied().unwrap_or(1));
        }
        if let Some(h) = &self.hidden {
            self.new_hidden.push(h.get(fi).copied().unwrap_or(false));
        }
        for (k, (_, data)) in self.corners.iter().enumerate() {
            let base = self.offsets.get(fi).copied().unwrap_or(0);
            for &c in kept {
                self.new_corners[k].push(data.get(base + c).copied().unwrap_or(WHITE));
            }
        }
    }

    pub(super) fn finish(self, m: &mut Mesh) {
        if self.sets.is_some() {
            m.face_sets = self.new_sets;
        }
        if self.hidden.is_some() {
            m.hidden_faces = self.new_hidden;
        }
        for ((i, _), data) in self.corners.into_iter().zip(self.new_corners) {
            if let Some(a) = m.color_attributes.get_mut(i) {
                a.data = data;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn attributes_follow_topology_changes() {
        let mut m = primitives::grid(3, 3, 2.0);
        let nv = m.positions.len();
        m.mask = (0..nv).map(|v| v as f32 / nv as f32).collect();
        m.face_sets = (0..m.faces.len() as i32).collect();
        let p = m.add_color_attribute("Col", AttrDomain::Point, WHITE);
        let c = m.add_color_attribute("Corner", AttrDomain::Corner, [0.5, 0.0, 0.0, 1.0]);
        m.color_attributes[p].data[3] = [0.0, 1.0, 0.0, 1.0];
        // Remove face 0's vertex 0 (the corner): face 0 goes, the rest stay.
        let mut remap: Vec<u32> = (0..nv as u32).collect();
        remap[0] = crate::mesh::NONE;
        for r in remap.iter_mut().skip(1) {
            *r -= 1;
        }
        let mask3 = m.mask[3];
        let set_last = *m.face_sets.last().unwrap();
        m.positions.remove(0);
        m.remap_vertices(&remap);
        assert_eq!(m.mask.len(), nv - 1);
        assert_eq!(m.mask[2], mask3);
        assert_eq!(m.face_sets.len(), m.faces.len());
        assert_eq!(*m.face_sets.last().unwrap(), set_last);
        assert_eq!(m.color_attributes[p].data[2], [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(m.color_attributes[c].data.len(), m.corner_count());
        // Interpolated vertices blend mask and colors.
        let v = m.add_vertex_interp(sex_mid(&m), &[(1, 0.5), (2, 0.5)]);
        assert_eq!(m.mask.len(), m.positions.len());
        assert!((m.mask[v as usize] - (m.mask[1] + m.mask[2]) * 0.5).abs() < 1e-6);
        assert_eq!(m.color_attributes[p].data.len(), m.positions.len());
    }

    fn sex_mid(m: &Mesh) -> crate::Vec3 {
        (m.positions[1] + m.positions[2]) * 0.5
    }

    #[test]
    fn append_and_sanitize() {
        let mut a = primitives::cube(1.0);
        let mut b = primitives::cube(1.0);
        b.add_color_attribute("Color", AttrDomain::Corner, [1.0, 0.0, 0.0, 1.0]);
        b.face_sets = vec![7; 6];
        a.append(&b);
        assert_eq!(a.face_sets.len(), 12);
        assert_eq!(a.face_sets[0], 1);
        assert_eq!(a.face_sets[11], 7);
        let col = &a.color_attributes[0];
        assert_eq!(col.data.len(), a.corner_count());
        assert_eq!(col.data[0], WHITE);
        assert_eq!(col.data[24], [1.0, 0.0, 0.0, 1.0]);
        let off = a.corner_offsets();
        assert_eq!(a.corner_color(0, &off, 6, 0), [1.0, 0.0, 0.0, 1.0]);
        a.faces.pop();
        a.sanitize_attributes();
        assert_eq!(a.face_sets.len(), 11);
        assert_eq!(a.color_attributes[0].data.len(), a.corner_count());
    }
}
