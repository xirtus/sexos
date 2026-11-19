// SPDX-License-Identifier: GPL-3.0-only
//! Edit Mode's view of the edited mesh (its cage): every vertex projected to
//! the screen, and a depth and face-index buffer of its triangles, so overlays
//! hide what is behind the mesh (unless X-ray is on) and clicks find the face
//! under the pointer. Rebuilt only when the mesh or the view changes.
use super::proj::{seg_dist, Proj};
use crate::doc::{Doc, Elem, SelectMode};
use crate::draw::Rect;
use sex_3d::Vec3;

pub struct EditView {
    pub rect: Rect,
    /// Depth key per pixel (larger is closer); -inf where empty.
    pub depth: Vec<f32>,
    /// Face index per pixel; u32::MAX where empty.
    pub face: Vec<u32>,
    /// World positions of the cage vertices.
    pub world: Vec<Vec3>,
    /// Screen position and depth key of each vertex (None: behind the eye).
    pub screen: Vec<Option<(f32, f32, f32)>>,
    pub edges: Vec<[u32; 2]>,
    /// World face centres (face dots).
    pub centers: Vec<Vec3>,
    pub center_screen: Vec<Option<(f32, f32, f32)>>,
    pub hidden_face: Vec<bool>,
}

impl EditView {
    pub fn build(d: &Doc, proj: &Proj) -> Option<EditView> {
        let (oi, mi) = crate::edit::active_mesh(d)?;
        let m = &d.scene.meshes[mi];
        let w = d.scene.world_matrix(oi);
        let r = proj.rect;
        let (rw, rh) = (r.w.max(0) as usize, r.h.max(0) as usize);
        let world: Vec<Vec3> = m.positions.iter().map(|&p| w.transform_point(p)).collect();
        let screen: Vec<Option<(f32, f32, f32)>> = world.iter().map(|&p| proj.project(p)).collect();
        let centers: Vec<Vec3> = (0..m.faces.len()).map(|f| w.transform_point(m.face_center(f))).collect();
        let center_screen = centers.iter().map(|&p| proj.project(p)).collect();
        let hidden_face: Vec<bool> = (0..m.faces.len()).map(|f| crate::edit::face_hidden(m, f, &d.edit_hidden)).collect();
        let mut ev = EditView {
            rect: r,
            depth: vec![f32::NEG_INFINITY; rw * rh],
            face: vec![u32::MAX; rw * rh],
            world,
            screen,
            edges: m.edges(),
            centers,
            center_screen,
            hidden_face,
        };
        for t in m.triangulate() {
            if ev.hidden_face[t.face as usize] {
                continue;
            }
            let p = t.verts.map(|v| ev.screen[v as usize]);
            if let [Some(a), Some(b), Some(c)] = p {
                ev.tri(a, b, c, t.face);
            }
        }
        Some(ev)
    }

    fn tri(&mut self, a: (f32, f32, f32), b: (f32, f32, f32), c: (f32, f32, f32), face: u32) {
        let r = self.rect;
        let (w, h) = (r.w, r.h);
        let (ax, ay) = (a.0 - r.x as f32, a.1 - r.y as f32);
        let (bx, by) = (b.0 - r.x as f32, b.1 - r.y as f32);
        let (cx, cy) = (c.0 - r.x as f32, c.1 - r.y as f32);
        let area = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
        if area.abs() < 1e-9 {
            return;
        }
        let x0 = ax.min(bx).min(cx).floor().max(0.0) as i32;
        let x1 = ax.max(bx).max(cx).ceil().min(w as f32 - 1.0) as i32;
        let y0 = ay.min(by).min(cy).floor().max(0.0) as i32;
        let y1 = ay.max(by).max(cy).ceil().min(h as f32 - 1.0) as i32;
        if x1 < x0 || y1 < y0 {
            return;
        }
        let inv = 1.0 / area;
        for y in y0..=y1 {
            let py = y as f32 + 0.5;
            for x in x0..=x1 {
                let px = x as f32 + 0.5;
                let w0 = ((bx - px) * (cy - py) - (cx - px) * (by - py)) * inv;
                let w1 = ((cx - px) * (ay - py) - (ax - px) * (cy - py)) * inv;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                    continue;
                }
                let z = w0 * a.2 + w1 * b.2 + w2 * c.2;
                let i = y as usize * w as usize + x as usize;
                if z > self.depth[i] {
                    self.depth[i] = z;
                    self.face[i] = face;
                }
            }
        }
    }

    /// Is a point at window (x, y) with depth key `z` in front of the mesh?
    pub fn visible(&self, x: f32, y: f32, z: f32) -> bool {
        let r = self.rect;
        let (ix, iy) = ((x - r.x as f32).floor() as i32, (y - r.y as f32).floor() as i32);
        let mut far = f32::INFINITY;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (xx, yy) = (ix + dx, iy + dy);
                if xx < 0 || yy < 0 || xx >= r.w || yy >= r.h {
                    continue;
                }
                far = far.min(self.depth[yy as usize * r.w as usize + xx as usize]);
            }
        }
        if !far.is_finite() {
            return true;
        }
        z >= far - (far.abs() * 4e-3 + 1e-6)
    }

    pub fn vert_visible(&self, v: u32, xray: bool) -> bool {
        match self.screen.get(v as usize).copied().flatten() {
            Some((x, y, z)) => xray || self.visible(x, y, z),
            None => false,
        }
    }

    /// The face under window pixel (x, y).
    pub fn face_at(&self, x: i32, y: i32) -> Option<u32> {
        let r = self.rect;
        if !r.contains(x, y) {
            return None;
        }
        let f = self.face[(y - r.y) as usize * r.w as usize + (x - r.x) as usize];
        (f != u32::MAX).then_some(f)
    }

    /// The element of the select mode nearest to the pointer.
    pub fn pick(&self, d: &Doc, x: i32, y: i32, xray: bool) -> Option<Elem> {
        let (fx, fy) = (x as f32, y as f32);
        let hidden = &d.edit_hidden;
        match d.select_mode {
            SelectMode::Vertex => {
                let mut best: Option<(f32, u32)> = None;
                for (i, s) in self.screen.iter().enumerate() {
                    let Some((sx, sy, _)) = *s else { continue };
                    if hidden.contains(&(i as u32)) || !self.vert_visible(i as u32, xray) {
                        continue;
                    }
                    let dist = (sx - fx).hypot(sy - fy);
                    if dist < 20.0 && best.is_none_or(|b| dist < b.0) {
                        best = Some((dist, i as u32));
                    }
                }
                best.map(|b| Elem::Vert(b.1))
            }
            SelectMode::Edge => {
                let mut best: Option<(f32, [u32; 2])> = None;
                for e in &self.edges {
                    if hidden.contains(&e[0]) || hidden.contains(&e[1]) {
                        continue;
                    }
                    let (Some(a), Some(b)) = (self.screen[e[0] as usize], self.screen[e[1] as usize]) else { continue };
                    let dist = seg_dist(fx, fy, (a.0, a.1), (b.0, b.1));
                    if dist > 15.0 || best.is_some_and(|b| dist >= b.0) {
                        continue;
                    }
                    // The nearest point of the edge must be visible.
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    let l2 = (dx * dx + dy * dy).max(1e-6);
                    let t = (((fx - a.0) * dx + (fy - a.1) * dy) / l2).clamp(0.0, 1.0);
                    let (px, py, pz) = (a.0 + dx * t, a.1 + dy * t, a.2 + (b.2 - a.2) * t);
                    if xray || self.visible(px, py, pz) {
                        best = Some((dist, *e));
                    }
                }
                best.map(|b| Elem::Edge(b.1[0], b.1[1]))
            }
            SelectMode::Face => {
                if !xray {
                    if let Some(f) = self.face_at(x, y) {
                        return Some(Elem::Face(f));
                    }
                }
                let mut best: Option<(f32, u32)> = None;
                for (i, c) in self.center_screen.iter().enumerate() {
                    let Some((sx, sy, sz)) = *c else { continue };
                    if self.hidden_face[i] || !(xray || self.visible(sx, sy, sz)) {
                        continue;
                    }
                    let dist = (sx - fx).hypot(sy - fy);
                    if dist < 15.0 && best.is_none_or(|b| dist < b.0) {
                        best = Some((dist, i as u32));
                    }
                }
                best.map(|b| Elem::Face(b.1))
            }
        }
    }

    /// Elements of the select mode inside the window rectangle `b`.
    pub fn in_box(&self, d: &Doc, b: Rect, xray: bool) -> Vec<Elem> {
        let inside = |p: Option<(f32, f32, f32)>| p.is_some_and(|(x, y, _)| b.contains(x as i32, y as i32));
        let hidden = &d.edit_hidden;
        match d.select_mode {
            SelectMode::Vertex => (0..self.screen.len() as u32).filter(|&v| !hidden.contains(&v) && inside(self.screen[v as usize]) && self.vert_visible(v, xray)).map(Elem::Vert).collect(),
            SelectMode::Edge => self
                .edges
                .iter()
                .filter(|e| !hidden.contains(&e[0]) && !hidden.contains(&e[1]))
                .filter(|e| inside(self.screen[e[0] as usize]) && inside(self.screen[e[1] as usize]))
                .filter(|e| xray || self.vert_visible(e[0], false) || self.vert_visible(e[1], false))
                .map(|e| Elem::Edge(e[0], e[1]))
                .collect(),
            SelectMode::Face => (0..self.centers.len())
                .filter(|&f| !self.hidden_face[f] && inside(self.center_screen[f]))
                .filter(|&f| xray || self.center_screen[f].is_some_and(|(x, y, z)| self.visible(x, y, z)))
                .map(|f| Elem::Face(f as u32))
                .collect(),
        }
    }

    /// The edge nearest the pointer (any select mode), for loop cut and knife.
    pub fn nearest_edge(&self, d: &Doc, x: f32, y: f32, xray: bool, max: f32) -> Option<[u32; 2]> {
        let mut best: Option<(f32, [u32; 2])> = None;
        for e in &self.edges {
            if d.edit_hidden.contains(&e[0]) || d.edit_hidden.contains(&e[1]) {
                continue;
            }
            let (Some(a), Some(b)) = (self.screen[e[0] as usize], self.screen[e[1] as usize]) else { continue };
            let dist = seg_dist(x, y, (a.0, a.1), (b.0, b.1));
            if dist < max && best.is_none_or(|bb| dist < bb.0) && (xray || self.visible((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5, (a.2 + b.2) * 0.5)) {
                best = Some((dist, *e));
            }
        }
        best.map(|b| b.1)
    }
}
