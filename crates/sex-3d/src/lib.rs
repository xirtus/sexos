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
//! - [`render`]: the CPU path tracer.
//! - [`io`]: glTF 2.0, OBJ, STL, PLY and .blend files.
//! - [`script`]: the text command language the Python `polyamour` module speaks.
pub mod math;
pub mod mesh;
pub mod primitives;
pub mod scene;
pub mod undo;
pub mod modifiers;
pub mod ops;
pub mod render;
pub mod io;
pub mod script;

pub use math::{Aabb, Mat4, Quat, Vec2, Vec3};
pub use mesh::{Face, HalfEdgeMesh, Mesh};
pub use scene::{Collection, Light, LightKind, Material, Object, ObjectData, Scene, Transform};
