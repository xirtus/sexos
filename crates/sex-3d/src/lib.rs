// SPDX-License-Identifier: GPL-3.0-only
//! sex-3d: the core of Polyamour, SexOS's native 3D suite.
//!
//! Plain Rust, no OS calls, so everything here is tested on the host and runs
//! unchanged inside SexOS std programs. Layout:
//! - [`math`]: vectors, quaternions, matrices, bounding boxes.
//! - [`mesh`]: polygon meshes (n-gons with face-corner UVs) and their half-edge view.
//! - [`primitives`]: cube, plane, grid, UV and ico spheres, cylinder, cone, torus.
//! - [`scene`]: objects, collections, materials, cameras, lights, world.
//! - [`undo`]: snapshot undo/redo of a scene.
//! - [`modifiers`]: the non-destructive modifier stack.
//! - [`ops`]: destructive modelling operations (extrude, inset, bevel, ...).
//! - [`ops2`]: more edit-mode operators (dissolve, subdivide, rip, fills, spin, hull, ...).
//! - [`render`]: the CPU path tracer.
//! - [`io`]: glTF 2.0, OBJ, STL, PLY and .blend files.
//! - [`script`]: the text command language the Python `polyamour` module speaks.
//! - [`uv`]: UV unwrapping (seams, ABF++/LSCM, projections, packing) and UV editing.
//! - [`armature`]: armatures, bones, poses and armature deformation (LBS/DQS, auto weights).
//! - [`constraints`]: object and bone constraints, IK and pose evaluation.
//! - [`lattice`]: lattices and lattice deformation.
//! - [`sculpt`]: Sculpt Mode: brushes, strokes, masks, face sets, dynamic topology, filters.
//! - [`paint`]: Vertex Paint and Texture Paint: color blending, brushes, image painting.
//! - [`physics`]: rigid bodies, cloth, soft bodies, particles, force fields, smoke and the point cache.
pub mod math;
pub mod mesh;
pub mod primitives;
pub mod scene;
pub mod undo;
pub mod modifiers;
pub mod nodes;
pub mod ops;
pub mod ops2;
pub mod render;
pub mod io;
pub mod script;
pub mod uv;
pub mod armature;
pub mod compositor;
pub mod constraints;
pub mod lattice;
pub mod geometry;
pub mod sculpt;
pub mod paint;
pub mod physics;

pub use math::{Aabb, Mat4, Quat, Vec2, Vec3};
pub use mesh::{Face, HalfEdgeMesh, Mesh};
pub use scene::{Collection, Light, LightKind, Material, Object, ObjectData, Scene, Transform};
