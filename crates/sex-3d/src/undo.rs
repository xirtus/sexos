// SPDX-License-Identifier: GPL-3.0-only
//! Undo and redo by scene snapshots. Each step names what it undoes so the
//! Edit menu can say "Undo Extrude". Memory is bounded by `max_steps`.
use crate::scene::Scene;

pub struct UndoStack {
    undo: Vec<(String, Scene)>,
    redo: Vec<(String, Scene)>,
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
        self.undo.push((label.into(), scene.clone()));
        if self.undo.len() > self.max_steps {
            self.undo.remove(0);
        }
        self.redo.clear();
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
        self.redo.push((label.clone(), std::mem::replace(scene, prev)));
        Some(label)
    }
    pub fn redo(&mut self, scene: &mut Scene) -> Option<String> {
        let (label, next) = self.redo.pop()?;
        self.undo.push((label.clone(), std::mem::replace(scene, next)));
        Some(label)
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
}
