// SPDX-License-Identifier: GPL-3.0-only
//! The non-destructive modifier stack. (Stub: filled in by the modifiers work.)
use crate::mesh::Mesh;

#[derive(Clone, Debug, PartialEq)]
pub enum Modifier {
    /// Mirror across the object's local axes; `merge` welds seam vertices
    /// within `merge_distance`.
    Mirror { x: bool, y: bool, z: bool, merge: bool, merge_distance: f32 },
    /// Catmull-Clark subdivision `levels` times (`simple` keeps the shape).
    Subdivision { levels: u32, simple: bool },
    /// `count` copies offset by `relative` (times bounds size) plus `constant`.
    Array { count: u32, relative: [f32; 3], constant: [f32; 3], merge: bool },
    /// Give surfaces `thickness` (offset along normals, -1..1 picks the side).
    Solidify { thickness: f32, offset: f32 },
    /// Bevel every edge sharper than `angle` radians by `width`, `segments` cuts.
    Bevel { width: f32, segments: u32, angle: f32 },
    Triangulate,
    /// Merge by distance.
    Weld { distance: f32 },
    /// Move vertices along normals by `strength` (uniform).
    Displace { strength: f32 },
    /// Laplacian smoothing.
    Smooth { factor: f32, iterations: u32 },
}

impl Modifier {
    pub fn name(&self) -> &'static str {
        match self {
            Modifier::Mirror { .. } => "Mirror",
            Modifier::Subdivision { .. } => "Subdivision",
            Modifier::Array { .. } => "Array",
            Modifier::Solidify { .. } => "Solidify",
            Modifier::Bevel { .. } => "Bevel",
            Modifier::Triangulate => "Triangulate",
            Modifier::Weld { .. } => "Weld",
            Modifier::Displace { .. } => "Displace",
            Modifier::Smooth { .. } => "Smooth",
        }
    }
    /// Apply this one modifier to `m`.
    pub fn apply(&self, m: &Mesh) -> Mesh {
        m.clone()
    }
}

/// Run a stack top to bottom.
pub fn apply_stack(m: &Mesh, stack: &[Modifier]) -> Mesh {
    let mut out = m.clone();
    for md in stack {
        out = md.apply(&out);
    }
    out
}
