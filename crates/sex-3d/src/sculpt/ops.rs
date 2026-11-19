// SPDX-License-Identifier: GPL-3.0-only
//! Sculpt Mode operators that are not strokes: mask clear/invert/fill, face
//! set creation and visibility, box and lasso masking/hiding/face sets, the
//! Mesh Filter and voxel remeshing.
use super::SculptSession;
use crate::math::Vec3;
use crate::mesh::Mesh;
use crate::modifiers::{Modifier, RemeshMode};
use std::collections::HashMap;

/// Alt+M (value 0), Ctrl+I invert, or fill with a value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MaskOp {
    Fill(f32),
    Invert,
}

pub fn mask_flood(m: &mut Mesh, op: MaskOp) {
    let n = m.positions.len();
    match op {
        MaskOp::Fill(v) if v == 0.0 => m.mask.clear(),
        MaskOp::Fill(v) => m.mask = vec![v.clamp(0.0, 1.0); n],
        MaskOp::Invert => {
            m.mask.resize(n, 0.0);
            for x in &mut m.mask {
                *x = 1.0 - *x;
            }
        }
    }
}

/// Grow (or shrink, with `grow` false) the mask by one ring of vertices.
pub fn mask_grow(m: &mut Mesh, ss: &SculptSession, grow: bool) {
    if m.mask.is_empty() {
        return;
    }
    let old = m.mask.clone();
    for v in 0..m.positions.len() {
        let nb = ss.neighbors(v as u32).iter().map(|&u| old[u as usize]);
        m.mask[v] = if grow { nb.fold(old[v], f32::max) } else { nb.fold(old[v], f32::min) };
    }
}

/// Smooth the mask (Blender's Mask Filter > Smooth).
pub fn mask_smooth(m: &mut Mesh, ss: &SculptSession) {
    if m.mask.is_empty() {
        return;
    }
    let old = m.mask.clone();
    for v in 0..m.positions.len() {
        let nb = ss.neighbors(v as u32);
        if !nb.is_empty() {
            m.mask[v] = (old[v] + nb.iter().map(|&u| old[u as usize]).sum::<f32>()) / (nb.len() + 1) as f32;
        }
    }
}

/// How Ctrl+W makes face sets (Blender's `sculpt.face_sets_init` modes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FaceSetInit {
    LooseParts,
    Materials,
    /// Faces whose normals differ by less than the angle (radians) join.
    Normals(f32),
    UvSeams,
    SharpEdges,
}

/// Replace the face sets.
pub fn face_sets_init(m: &mut Mesh, mode: FaceSetInit) {
    let nf = m.faces.len();
    if let FaceSetInit::Materials = mode {
        m.face_sets = m.faces.iter().map(|f| f.material as i32 + 1).collect();
        return;
    }
    // Flood fill across shared edges that the mode lets through.
    let mut edge_faces: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (fi, f) in m.faces.iter().enumerate() {
        for (a, b) in f.edges() {
            edge_faces.entry((a.min(b), a.max(b))).or_default().push(fi as u32);
        }
    }
    let normals: Vec<Vec3> = (0..nf).map(|f| m.face_normal(f)).collect();
    let pass = |e: (u32, u32), f: usize, g: usize| -> bool {
        match mode {
            FaceSetInit::LooseParts => true,
            FaceSetInit::Normals(a) => normals[f].dot(normals[g]) >= a.cos(),
            FaceSetInit::UvSeams => !m.is_seam(e.0, e.1),
            FaceSetInit::SharpEdges => !m.is_sharp(e.0, e.1),
            FaceSetInit::Materials => true,
        }
    };
    let mut set = vec![0i32; nf];
    let mut next = 1;
    for start in 0..nf {
        if set[start] != 0 {
            continue;
        }
        set[start] = next;
        let mut stack = vec![start];
        while let Some(f) = stack.pop() {
            for (a, b) in m.faces[f].edges() {
                let k = (a.min(b), a.max(b));
                for &g in edge_faces.get(&k).map_or(&[][..], |v| v.as_slice()) {
                    let g = g as usize;
                    if set[g] == 0 && pass(k, f, g) {
                        set[g] = next;
                        stack.push(g);
                    }
                }
            }
        }
        next += 1;
    }
    m.face_sets = set;
}

/// Visibility by face set (H, Shift+H, Alt+H).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    /// Hide the faces of this set.
    HideSet(i32),
    /// Show only this set (Blender's "Show Active Face Set").
    Isolate(i32),
    ShowAll,
    /// Swap hidden and shown.
    Invert,
}

pub fn face_set_visibility(m: &mut Mesh, op: Visibility) {
    let nf = m.faces.len();
    let hidden: Vec<bool> = match op {
        Visibility::ShowAll => Vec::new(),
        Visibility::HideSet(s) => (0..nf).map(|f| m.is_face_hidden(f) || m.face_set(f) == s).collect(),
        Visibility::Isolate(s) => (0..nf).map(|f| m.face_set(f) != s).collect(),
        Visibility::Invert => (0..nf).map(|f| !m.is_face_hidden(f)).collect(),
    };
    m.hidden_faces = if hidden.iter().any(|&h| h) { hidden } else { Vec::new() };
}

/// Vertices whose projection falls inside a screen region (box or lasso).
/// `project` maps object space to the screen; with `front` only vertices
/// whose normal faces `view(p)` (towards the viewer) count.
pub fn verts_in_screen(m: &Mesh, ss: &SculptSession, project: &dyn Fn(Vec3) -> Option<(f32, f32)>, inside: &dyn Fn(f32, f32) -> bool, front: Option<&dyn Fn(Vec3) -> Vec3>) -> Vec<bool> {
    (0..m.positions.len())
        .map(|v| {
            let p = m.positions[v];
            if ss.is_hidden(v as u32) {
                return false;
            }
            if let Some(view) = front {
                if ss.normals.get(v).is_some_and(|n| n.dot(view(p)) < 0.0) {
                    return false;
                }
            }
            project(p).is_some_and(|(x, y)| inside(x, y))
        })
        .collect()
}

/// Point-in-polygon (even-odd) for lasso tools.
pub fn in_lasso(pts: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut inside = false;
    let n = pts.len();
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + n - 1) % n]);
        if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
    }
    inside
}

/// Box/Lasso Mask: set the mask of the selected vertices to `value`.
pub fn mask_select(m: &mut Mesh, sel: &[bool], value: f32) {
    m.mask.resize(m.positions.len(), 0.0);
    for (v, &s) in sel.iter().enumerate() {
        if s {
            m.mask[v] = value;
        }
    }
    if m.mask.iter().all(|&x| x == 0.0) {
        m.mask.clear();
    }
}

/// Box/Lasso Hide: hide (or with `show` reveal) faces whose vertices are
/// all selected.
pub fn hide_select(m: &mut Mesh, sel: &[bool], show: bool) {
    let nf = m.faces.len();
    m.hidden_faces.resize(nf, false);
    for f in 0..nf {
        let all = m.faces[f].verts.iter().all(|&v| sel.get(v as usize).copied().unwrap_or(false));
        if all {
            m.hidden_faces[f] = !show;
        }
    }
    if m.hidden_faces.iter().all(|h| !h) {
        m.hidden_faces.clear();
    }
}

/// Box/Lasso Face Set: faces whose vertices are all selected get a new set.
/// Returns the new set's id.
pub fn face_set_select(m: &mut Mesh, sel: &[bool]) -> i32 {
    let nf = m.faces.len();
    let id = m.face_sets.iter().copied().max().unwrap_or(1) + 1;
    m.face_sets.resize(nf, 1);
    for f in 0..nf {
        if m.faces[f].verts.iter().all(|&v| sel.get(v as usize).copied().unwrap_or(false)) && !m.is_face_hidden(f) {
            m.face_sets[f] = id;
        }
    }
    id
}

/// Mesh Filter types (Blender's `sculpt.mesh_filter` `type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Smooth,
    Scale,
    Inflate,
    Sphere,
    Random,
    Sharpen,
    EnhanceDetails,
}

impl FilterKind {
    pub const ALL: [FilterKind; 7] = [FilterKind::Smooth, FilterKind::Scale, FilterKind::Inflate, FilterKind::Sphere, FilterKind::Random, FilterKind::Sharpen, FilterKind::EnhanceDetails];
    pub fn label(self) -> &'static str {
        match self {
            FilterKind::Smooth => "Smooth",
            FilterKind::Scale => "Scale",
            FilterKind::Inflate => "Inflate",
            FilterKind::Sphere => "Sphere",
            FilterKind::Random => "Random",
            FilterKind::Sharpen => "Sharpen",
            FilterKind::EnhanceDetails => "Enhance Details",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            FilterKind::Smooth => "SMOOTH",
            FilterKind::Scale => "SCALE",
            FilterKind::Inflate => "INFLATE",
            FilterKind::Sphere => "SPHERE",
            FilterKind::Random => "RANDOM",
            FilterKind::Sharpen => "SHARPEN",
            FilterKind::EnhanceDetails => "ENHANCE_DETAILS",
        }
    }
    pub fn from_id(s: &str) -> Option<FilterKind> {
        FilterKind::ALL.into_iter().find(|k| k.id() == s || k.label() == s)
    }
}

fn hash01(v: u32, seed: u32) -> f32 {
    let mut x = v.wrapping_mul(0x9E37_79B9) ^ seed.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// One step of the Mesh Filter with `amount` (the drag since the last
/// step; negative reverses). Masked and hidden vertices stay; `axes` limits
/// the movement (Blender's deform axes). Returns the moved vertices.
pub fn mesh_filter(m: &mut Mesh, ss: &mut SculptSession, kind: FilterKind, amount: f32, axes: [bool; 3], seed: u32) -> Vec<u32> {
    let n = m.positions.len();
    if n == 0 || amount == 0.0 {
        return Vec::new();
    }
    let b = m.bounds();
    let size = b.size().length().max(1e-6);
    let center = b.center();
    let avg_r = m.positions.iter().map(|p| p.distance(center)).sum::<f32>() / n as f32;
    let old = m.positions.clone();
    let lap = |v: usize| -> Vec3 {
        let nb = ss.neighbors(v as u32);
        if nb.is_empty() {
            return Vec3::ZERO;
        }
        nb.iter().fold(Vec3::ZERO, |a, &u| a + old[u as usize]) / nb.len() as f32 - old[v]
    };
    let mut moved = Vec::new();
    for v in 0..n {
        if ss.is_hidden(v as u32) {
            continue;
        }
        let k = 1.0 - m.mask_at(v).clamp(0.0, 1.0);
        if k <= 0.0 {
            continue;
        }
        let p = old[v];
        let nrm = ss.normals.get(v).copied().unwrap_or(Vec3::ZERO);
        let d = match kind {
            FilterKind::Smooth => lap(v) * amount.clamp(-1.0, 1.0),
            FilterKind::EnhanceDetails => -lap(v) * amount.clamp(-1.0, 1.0),
            FilterKind::Sharpen => {
                // Push along the normal by how far the vertex sits from its
                // neighbours' average: ridges sharpen, valleys deepen.
                let l = lap(v);
                nrm * (-l.dot(nrm) * amount.clamp(-1.0, 1.0)) + l * (amount.clamp(-1.0, 1.0) * 0.25)
            }
            FilterKind::Scale => (p - center) * amount,
            FilterKind::Inflate => nrm * (amount * size * 0.1),
            FilterKind::Sphere => {
                let dir = (p - center).normalize();
                (center + dir * avg_r - p) * amount.clamp(-1.0, 1.0)
            }
            FilterKind::Random => nrm * (hash01(v as u32, seed) * amount * size * 0.05),
        } * k;
        let d = Vec3::new(if axes[0] { d.x } else { 0.0 }, if axes[1] { d.y } else { 0.0 }, if axes[2] { d.z } else { 0.0 });
        if d != Vec3::ZERO {
            m.positions[v] = p + d;
            moved.push(v as u32);
        }
    }
    ss.pbvh.mark_verts(&moved);
    ss.pbvh.refit(&m.positions);
    ss.update_normals(m, &moved);
    moved
}

/// Voxel remesh (Ctrl+R): a new watertight surface on a `voxel_size` grid.
/// Masks, face sets and point colors move to the new vertices from the
/// nearest old ones; `preserve_volume` rescales to the old volume.
pub fn voxel_remesh(m: &Mesh, voxel_size: f32, preserve_volume: bool) -> Mesh {
    let md = Modifier::Remesh { mode: RemeshMode::Voxel, voxel_size: voxel_size.max(1e-4), octree_depth: 4, scale: 0.9, smooth_shade: m.faces.iter().any(|f| f.smooth) };
    let mut out = md.apply(m);
    if out.positions.is_empty() {
        return out;
    }
    if preserve_volume {
        let (v0, v1) = (m.volume().abs(), out.volume().abs());
        if v0 > 1e-12 && v1 > 1e-12 {
            let c = out.bounds().center();
            let k = (v0 / v1).cbrt();
            for p in &mut out.positions {
                *p = c + (*p - c) * k;
            }
        }
    }
    // Nearest old vertex for every new one (uniform grid).
    let cell = voxel_size.max(1e-4) * 2.0;
    let key = |p: Vec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    for (i, &p) in m.positions.iter().enumerate() {
        grid.entry(key(p)).or_default().push(i as u32);
    }
    let nearest = |p: Vec3| -> Option<u32> {
        let k = key(p);
        let mut best: Option<(f32, u32)> = None;
        for r in 1..4i64 {
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        for &i in grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)).map_or(&[][..], |v| v.as_slice()) {
                            let d = (m.positions[i as usize] - p).length_squared();
                            if best.is_none_or(|b| d < b.0) {
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
        best.map(|b| b.1)
    };
    let src: Vec<Option<u32>> = out.positions.iter().map(|&p| nearest(p)).collect();
    if !m.mask.is_empty() {
        out.mask = src.iter().map(|s| s.map_or(0.0, |i| m.mask_at(i as usize))).collect();
    }
    for a in &m.color_attributes {
        let cols = m.point_colors(m.color_attributes.iter().position(|x| x.name == a.name).unwrap_or(0));
        let data = src.iter().map(|s| s.map_or([1.0; 4], |i| cols[i as usize])).collect();
        out.color_attributes.push(crate::mesh::ColorAttribute { name: a.name.clone(), domain: crate::mesh::AttrDomain::Point, data, byte: a.byte });
    }
    out.active_color = m.active_color;
    out.render_color = m.render_color;
    if !m.face_sets.is_empty() {
        // A face takes the set of the old vertex nearest its first corner.
        let mut vset = vec![1i32; m.positions.len()];
        for (f, face) in m.faces.iter().enumerate() {
            for &v in &face.verts {
                vset[v as usize] = m.face_set(f);
            }
        }
        out.face_sets = out.faces.iter().map(|f| src[f.verts[0] as usize].map_or(1, |i| vset[i as usize])).collect();
    }
    out
}
