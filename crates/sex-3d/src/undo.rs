// SPDX-License-Identifier: GPL-3.0-only
//! Undo and redo by scene snapshots. Each step names what it undoes so the
//! Edit menu can say "Undo Extrude". Memory is bounded by `max_steps`.
//!
//! Sculpt and paint strokes would make whole-scene snapshots costly, so a
//! step can instead be a [`Change`] list: only the vertices, colors, image
//! regions or single mesh a stroke touched, with their old values. Applying
//! a change swaps its stored values with the scene's, so the same list
//! undoes and (applied again in the other order) redoes.
use crate::math::Vec3;
use crate::mesh::Mesh;
use crate::modifiers::Modifier;
use crate::scene::Scene;

/// One recorded difference; [`Change::swap`] exchanges it with the scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Positions of some vertices of a mesh.
    Positions { mesh: usize, verts: Vec<u32>, values: Vec<Vec3> },
    /// Mask values of some vertices.
    Mask { mesh: usize, verts: Vec<u32>, values: Vec<f32> },
    /// The whole mask array (for strokes that create or drop it).
    MaskAll { mesh: usize, values: Vec<f32> },
    /// Some elements (vertices or corners) of one color attribute.
    Colors { mesh: usize, attr: usize, elems: Vec<u32>, values: Vec<[f32; 4]> },
    /// The face set array.
    FaceSets { mesh: usize, values: Vec<i32> },
    /// The hidden-face array.
    Hidden { mesh: usize, values: Vec<bool> },
    /// A whole mesh (topology changes: dynamic topology, remesh).
    Mesh { mesh: usize, data: Box<Mesh> },
    /// A rectangle of an image's pixels (row-major).
    Image { image: usize, x: u32, y: u32, w: u32, h: u32, pixels: Vec<[f32; 4]> },
    /// One modifier of an object (Multires displacements).
    Modifier { object: usize, index: usize, data: Box<Modifier> },
    /// A whole Grease Pencil data-block (a stroke drawn, erased or sculpted).
    GreasePencil { gp: usize, data: Box<crate::grease_pencil::GreasePencil> },
}

fn swap_sparse<T>(dst: &mut [T], idx: &[u32], vals: &mut [T]) {
    for (&i, v) in idx.iter().zip(vals.iter_mut()) {
        if let Some(d) = dst.get_mut(i as usize) {
            std::mem::swap(d, v);
        }
    }
}

impl Change {
    /// Exchange the stored values with the scene's.
    pub fn swap(&mut self, scene: &mut Scene) {
        match self {
            Change::Positions { mesh, verts, values } => {
                if let Some(m) = scene.meshes.get_mut(*mesh) {
                    swap_sparse(&mut m.positions, verts, values);
                }
            }
            Change::Mask { mesh, verts, values } => {
                if let Some(m) = scene.meshes.get_mut(*mesh) {
                    swap_sparse(&mut m.mask, verts, values);
                }
            }
            Change::MaskAll { mesh, values } => {
                if let Some(m) = scene.meshes.get_mut(*mesh) {
                    std::mem::swap(&mut m.mask, values);
                }
            }
            Change::Colors { mesh, attr, elems, values } => {
                if let Some(a) = scene.meshes.get_mut(*mesh).and_then(|m| m.color_attributes.get_mut(*attr)) {
                    swap_sparse(&mut a.data, elems, values);
                }
            }
            Change::FaceSets { mesh, values } => {
                if let Some(m) = scene.meshes.get_mut(*mesh) {
                    std::mem::swap(&mut m.face_sets, values);
                }
            }
            Change::Hidden { mesh, values } => {
                if let Some(m) = scene.meshes.get_mut(*mesh) {
                    std::mem::swap(&mut m.hidden_faces, values);
                }
            }
            Change::Mesh { mesh, data } => {
                if let Some(m) = scene.meshes.get_mut(*mesh) {
                    std::mem::swap(m, data.as_mut());
                }
            }
            Change::Image { image, x, y, w, h, pixels } => {
                let Some(img) = scene.images.get_mut(*image) else { return };
                let (iw, ih) = (img.width, img.height);
                for row in 0..*h {
                    let yy = *y + row;
                    if yy >= ih {
                        break;
                    }
                    for col in 0..*w {
                        let xx = *x + col;
                        if xx >= iw {
                            break;
                        }
                        let (Some(d), Some(s)) = (img.pixels.get_mut((yy * iw + xx) as usize), pixels.get_mut((row * *w + col) as usize)) else { continue };
                        std::mem::swap(d, s);
                    }
                }
            }
            Change::Modifier { object, index, data } => {
                if let Some(md) = scene.objects.get_mut(*object).and_then(|o| o.modifiers.get_mut(*index)) {
                    std::mem::swap(md, data.as_mut());
                }
            }
            Change::GreasePencil { gp, data } => {
                if let Some(g) = scene.grease_pencils.get_mut(*gp) {
                    std::mem::swap(g, data.as_mut());
                }
            }
        }
    }

    /// Bytes of stored data, roughly.
    pub fn bytes(&self) -> usize {
        match self {
            Change::Positions { verts, .. } => verts.len() * 16,
            Change::Mask { verts, .. } => verts.len() * 8,
            Change::MaskAll { values, .. } => values.len() * 4,
            Change::Colors { elems, .. } => elems.len() * 20,
            Change::FaceSets { values, .. } => values.len() * 4,
            Change::Hidden { values, .. } => values.len(),
            Change::Mesh { data, .. } => data.positions.len() * 12 + data.corner_count() * 4 + data.faces.len() * 40,
            Change::Image { pixels, .. } => pixels.len() * 16,
            Change::Modifier { .. } => 64,
            Change::GreasePencil { data, .. } => data.point_count() * 48 + 256,
        }
    }
}

/// An undo step: a whole scene, or the changes of one stroke.
#[derive(Clone, Debug)]
enum Step {
    Scene(Scene),
    Patch(Vec<Change>),
}

pub struct UndoStack {
    undo: Vec<(String, Step)>,
    redo: Vec<(String, Step)>,
    pub max_steps: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        UndoStack { undo: Vec::new(), redo: Vec::new(), max_steps: 64 }
    }
}

impl UndoStack {
    pub fn new() -> UndoStack {
        UndoStack::default()
    }
    /// Record `scene` as it is *before* an operation named `label`.
    pub fn push(&mut self, label: &str, scene: &Scene) {
        self.push_step(label, Step::Scene(scene.clone()));
    }
    /// Record a step made of `changes`, already applied to the scene: each
    /// holds the values from *before* the operation.
    pub fn push_changes(&mut self, label: &str, changes: Vec<Change>) {
        self.push_step(label, Step::Patch(changes));
    }
    fn push_step(&mut self, label: &str, step: Step) {
        self.undo.push((label.into(), step));
        if self.undo.len() > self.max_steps {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    /// Bytes held by change-list steps (snapshots are not counted).
    pub fn patch_bytes(&self) -> usize {
        self.undo.iter().chain(&self.redo).map(|(_, s)| if let Step::Patch(c) = s { c.iter().map(Change::bytes).sum() } else { 0 }).sum()
    }
    /// Number of undo steps.
    pub fn len(&self) -> usize {
        self.undo.len()
    }
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|u| u.0.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|u| u.0.as_str())
    }
    /// Restore the last snapshot into `scene`. Returns the step's label.
    pub fn undo(&mut self, scene: &mut Scene) -> Option<String> {
        let (label, prev) = self.undo.pop()?;
        self.redo.push((label.clone(), apply(scene, prev)));
        Some(label)
    }
    pub fn redo(&mut self, scene: &mut Scene) -> Option<String> {
        let (label, next) = self.redo.pop()?;
        self.undo.push((label.clone(), apply(scene, next)));
        Some(label)
    }
}

/// Apply a step to `scene`; returns the step that reverses it.
fn apply(scene: &mut Scene, step: Step) -> Step {
    match step {
        Step::Scene(s) => Step::Scene(std::mem::replace(scene, s)),
        Step::Patch(mut changes) => {
            for c in changes.iter_mut().rev() {
                c.swap(scene);
            }
            changes.reverse();
            Step::Patch(changes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_round_trip() {
        let mut s = Scene::startup();
        let mut u = UndoStack::new();
        u.push("Add Torus", &s);
        s.add_mesh_object("Torus", crate::primitives::torus(8, 4, 1.0, 0.2));
        assert_eq!(s.objects.len(), 4);
        assert_eq!(u.undo(&mut s).as_deref(), Some("Add Torus"));
        assert_eq!(s.objects.len(), 3);
        assert_eq!(u.redo_label(), Some("Add Torus"));
        u.redo(&mut s);
        assert_eq!(s.objects.len(), 4);
    }

    #[test]
    fn change_steps_swap_back_and_forth() {
        let mut s = Scene::startup();
        let mut u = UndoStack::new();
        u.push("Add", &s);
        let mi = s.add_mesh("Grid", crate::primitives::grid(4, 4, 2.0));
        let before = s.meshes[mi].clone();
        // A "stroke" moves vertices 0 and 5 and makes a mask.
        let old = vec![s.meshes[mi].positions[0], s.meshes[mi].positions[5]];
        s.meshes[mi].positions[0].z += 1.0;
        s.meshes[mi].positions[5].z -= 1.0;
        let n = s.meshes[mi].positions.len();
        s.meshes[mi].mask = vec![0.5; n];
        u.push_changes("Draw", vec![Change::Positions { mesh: mi, verts: vec![0, 5], values: old }, Change::MaskAll { mesh: mi, values: Vec::new() }]);
        let after = s.meshes[mi].clone();
        assert!(u.patch_bytes() < 200);
        assert_eq!(u.undo(&mut s).as_deref(), Some("Draw"));
        assert_eq!(s.meshes[mi], before);
        assert_eq!(u.redo(&mut s).as_deref(), Some("Draw"));
        assert_eq!(s.meshes[mi], after);
        u.undo(&mut s);
        assert_eq!(s.meshes[mi], before);
        u.undo(&mut s);
        assert_eq!(s.meshes.len(), mi);
    }
}
