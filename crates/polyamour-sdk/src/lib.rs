// SPDX-License-Identifier: GPL-3.0-only
//! `polyamour-sdk`: pure-Rust `#![no_std]` + `alloc` SDK for Polyamour `.wasm`
//! add-ons and high-throughput procedural generators.
//!
//! Speaks the exact same `polyops 1` protocol as `apps/polyamour/python` (`import bpy`),
//! adding zero-file bi-directional queries (`mesh_get`, `get`, `list`, `exists`) and
//! declarative operator UI panel descriptors (`ui_panel`, `ui_prop`).
#![no_std]
extern crate alloc;

pub mod math;
pub mod transport;
pub mod wire;

pub use math::{Color, Euler, Quat, Vec2, Vec3};
pub use transport::{BufferTransport, SdkError, Transport};
pub use wire::{MeshData, VERSION};

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt::Write as _;
use wire::{decode_mesh_options, encode_mesh_options, fmt_f32, fmt_f32_list, parse_floats, parse_tagged_reply, quote};

/// Reference to a scene object: either a bound `$N` handle (for created objects)
/// or an existing object's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef(String);

impl ObjectRef {
    pub fn named(name: &str) -> Self {
        Self(quote(name))
    }

    pub fn token(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshRef(String);

impl MeshRef {
    pub fn named(name: &str) -> Self {
        Self(quote(name))
    }

    pub fn token(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialRef(String);

impl MaterialRef {
    pub fn named(name: &str) -> Self {
        Self(quote(name))
    }

    pub fn token(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionRef(String);

impl CollectionRef {
    pub fn named(name: &str) -> Self {
        Self(quote(name))
    }

    pub fn token(&self) -> &str {
        &self.0
    }
}

/// Optional spatial placement passed to object-creating commands.
#[derive(Clone, Debug, Default)]
pub struct Placement {
    pub name: Option<String>,
    pub location: Option<Vec3>,
    pub rotation: Option<Euler>,
    pub quaternion: Option<Quat>,
    pub scale: Option<Vec3>,
    pub collection: Option<CollectionRef>,
    pub visible: Option<bool>,
}

impl Placement {
    pub fn at(location: Vec3) -> Self {
        Self { location: Some(location), ..Self::default() }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_rotation(mut self, rotation: Euler) -> Self {
        self.rotation = Some(rotation);
        self
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = Some(scale);
        self
    }

    pub fn in_collection(mut self, collection: &CollectionRef) -> Self {
        self.collection = Some(collection.clone());
        self
    }

    fn write_opts(&self, out: &mut String, include_name: bool) -> Result<(), SdkError> {
        if include_name {
            if let Some(n) = &self.name {
                let _ = write!(out, " name={}", quote(n));
            }
        }
        if let Some(loc) = self.location {
            let s = fmt_f32_list(&loc.to_array()).map_err(SdkError::Wire)?;
            let _ = write!(out, " location={s}");
        }
        if let Some(rot) = self.rotation {
            let s = fmt_f32_list(&rot.to_array()).map_err(SdkError::Wire)?;
            let _ = write!(out, " rotation={s}");
        }
        if let Some(q) = self.quaternion {
            let s = fmt_f32_list(&q.to_wxyz_array()).map_err(SdkError::Wire)?;
            let _ = write!(out, " quaternion={s}");
        }
        if let Some(sc) = self.scale {
            let s = fmt_f32_list(&sc.to_array()).map_err(SdkError::Wire)?;
            let _ = write!(out, " scale={s}");
        }
        if let Some(c) = &self.collection {
            let _ = write!(out, " collection={}", c.token());
        }
        if let Some(vis) = self.visible {
            let _ = write!(out, " visible={}", vis as u8);
        }
        Ok(())
    }
}

/// Built-in mesh primitives supported by `sex-3d`.
#[derive(Clone, Copy, Debug)]
pub enum Primitive {
    Cube { size: f32 },
    Plane { size: f32 },
    Grid { x_subdivisions: u32, y_subdivisions: u32, size: f32 },
    UvSphere { segments: u32, rings: u32, radius: f32 },
    IcoSphere { subdivisions: u32, radius: f32 },
    Cylinder { segments: u32, radius: f32, depth: f32 },
    Cone { segments: u32, radius1: f32, radius2: f32, depth: f32 },
    Torus { major_segments: u32, minor_segments: u32, major_radius: f32, minor_radius: f32 },
}

/// Principled PBR material attributes.
#[derive(Clone, Copy, Debug)]
pub struct MaterialParams {
    pub base_color: Color,
    pub metallic: f32,
    pub roughness: f32,
    pub emission: Vec3,
    pub emission_strength: f32,
    pub ior: f32,
    pub transmission: f32,
    pub double_sided: bool,
}

impl Default for MaterialParams {
    fn default() -> Self {
        Self {
            base_color: Color::default(),
            metallic: 0.0,
            roughness: 0.5,
            emission: Vec3::ZERO,
            emission_strength: 0.0,
            ior: 1.5,
            transmission: 0.0,
            double_sided: false,
        }
    }
}

/// Non-destructive modifiers matching `sex_3d::modifiers::Modifier`.
#[derive(Clone, Copy, Debug)]
pub enum ModifierSpec {
    Mirror { x: bool, y: bool, z: bool, merge: bool, merge_distance: f32 },
    Subdivision { levels: u32, simple: bool },
    Array { count: u32, relative: Vec3, constant: Vec3, merge: bool },
    Solidify { thickness: f32, offset: f32 },
    Bevel { width: f32, segments: u32, angle: f32 },
    Triangulate,
    Weld { distance: f32 },
    Displace { strength: f32 },
    Smooth { factor: f32, iterations: u32 },
}

impl ModifierSpec {
    fn write_cmd(&self, out: &mut String) -> Result<(), SdkError> {
        let f = |v: f32| fmt_f32(v).map_err(SdkError::Wire);
        let v3 = |v: Vec3| fmt_f32_list(&v.to_array()).map_err(SdkError::Wire);
        match *self {
            Self::Mirror { x, y, z, merge, merge_distance } => {
                let _ = write!(out, "Mirror x={} y={} z={} merge={} merge_distance={}", x as u8, y as u8, z as u8, merge as u8, f(merge_distance)?);
            }
            Self::Subdivision { levels, simple } => {
                let _ = write!(out, "Subdivision levels={levels} simple={}", simple as u8);
            }
            Self::Array { count, relative, constant, merge } => {
                let _ = write!(out, "Array count={count} relative={} constant={} merge={}", v3(relative)?, v3(constant)?, merge as u8);
            }
            Self::Solidify { thickness, offset } => {
                let _ = write!(out, "Solidify thickness={} offset={}", f(thickness)?, f(offset)?);
            }
            Self::Bevel { width, segments, angle } => {
                let _ = write!(out, "Bevel width={} segments={segments} angle={}", f(width)?, f(angle)?);
            }
            Self::Triangulate => out.push_str("Triangulate"),
            Self::Weld { distance } => {
                let _ = write!(out, "Weld distance={}", f(distance)?);
            }
            Self::Displace { strength } => {
                let _ = write!(out, "Displace strength={}", f(strength)?);
            }
            Self::Smooth { factor, iterations } => {
                let _ = write!(out, "Smooth factor={} iterations={iterations}", f(factor)?);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum LightSpec {
    Point { power: f32, color: Color, radius: f32 },
    Sun { power: f32, color: Color, radius: f32 },
    Spot { power: f32, color: Color, radius: f32, angle: f32, blend: f32 },
    Area { power: f32, color: Color, size: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiPropKind {
    Float,
    Int,
    Bool,
    Vec3,
    Color,
    String,
    Button,
}

impl UiPropKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Vec3 => "vec3",
            Self::Color => "color",
            Self::String => "string",
            Self::Button => "button",
        }
    }
}

#[derive(Clone, Debug)]
pub struct UiPropSpec {
    pub key: String,
    pub kind: UiPropKind,
    pub label: String,
    pub default: Option<String>,
    pub min: Option<f32>,
    pub max: Option<f32>,
}

/// Pure-Rust `polyops 1` client session over any [`Transport`].
pub struct Client<T: Transport> {
    transport: T,
    next_handle: u32,
    next_req: u32,
    closed: bool,
}

impl<T: Transport> Client<T> {
    /// Open a new `polyops 1` stream and emit the protocol header immediately.
    pub fn open(mut transport: T) -> Result<Self, SdkError> {
        transport.write_line(&format!("polyops {VERSION}"))?;
        Ok(Self {
            transport,
            next_handle: 0,
            next_req: 0,
            closed: false,
        })
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn log(&mut self, msg: &str) {
        self.transport.log(msg);
    }

    pub fn read_scene_snapshot(&mut self) -> Result<Option<String>, SdkError> {
        self.transport.read_scene_snapshot()
    }

    fn ensure_open(&self) -> Result<(), SdkError> {
        if self.closed {
            Err(SdkError::Closed)
        } else {
            Ok(())
        }
    }

    fn alloc_handle(&mut self) -> String {
        self.next_handle += 1;
        format!("${}", self.next_handle)
    }

    fn request(&mut self, cmd_body: &str) -> Result<String, SdkError> {
        self.ensure_open()?;
        self.next_req += 1;
        let id = format!("r{}", self.next_req);
        self.transport.write_line(&format!("@{id} {cmd_body}"))?;
        let raw_reply = self.transport.read_reply()?;
        let payload = parse_tagged_reply(&raw_reply, &id).map_err(SdkError::Protocol)?;
        Ok(String::from(payload))
    }

    // --- Scene & World -------------------------------------------------------

    pub fn set_scene(&mut self, name: Option<&str>, frame_start: Option<i32>, frame_end: Option<i32>, fps: Option<f32>) -> Result<(), SdkError> {
        self.ensure_open()?;
        let mut line = String::from("scene");
        if let Some(n) = name {
            let _ = write!(line, " name={}", quote(n));
        }
        if let Some(fs) = frame_start {
            let _ = write!(line, " frame_start={fs}");
        }
        if let Some(fe) = frame_end {
            let _ = write!(line, " frame_end={fe}");
        }
        if let Some(f) = fps {
            let s = fmt_f32(f).map_err(SdkError::Wire)?;
            let _ = write!(line, " fps={s}");
        }
        self.transport.write_line(&line)
    }

    pub fn set_world(&mut self, color: Color, strength: f32) -> Result<(), SdkError> {
        self.ensure_open()?;
        let c = fmt_f32_list(&color.to_rgb_array()).map_err(SdkError::Wire)?;
        let s = fmt_f32(strength).map_err(SdkError::Wire)?;
        self.transport.write_line(&format!("world color={c} strength={s}"))
    }

    pub fn set_render_option(&mut self, key: &str, value: &str) -> Result<(), SdkError> {
        self.ensure_open()?;
        self.transport.write_line(&format!("render {key}={}", quote(value)))
    }

    // --- Collections ---------------------------------------------------------

    pub fn collection_new(&mut self, name: &str, parent: Option<&CollectionRef>) -> Result<CollectionRef, SdkError> {
        self.ensure_open()?;
        let h = self.alloc_handle();
        let mut line = format!("collection_new {} as={h}", quote(name));
        if let Some(p) = parent {
            let _ = write!(line, " parent={}", p.token());
        }
        self.transport.write_line(&line)?;
        Ok(CollectionRef(h))
    }

    pub fn link_to_collection(&mut self, obj: &ObjectRef, coll: &CollectionRef) -> Result<(), SdkError> {
        self.ensure_open()?;
        self.transport.write_line(&format!("link {} {}", obj.token(), coll.token()))
    }

    // --- Primitives & Meshes -------------------------------------------------

    pub fn add_primitive(&mut self, prim: Primitive, placement: &Placement) -> Result<ObjectRef, SdkError> {
        self.ensure_open()?;
        let h = self.alloc_handle();
        let f = |v: f32| fmt_f32(v).map_err(SdkError::Wire);
        let mut line = String::from("add_primitive ");
        match prim {
            Primitive::Cube { size } => write!(line, "cube size={}", f(size)?).unwrap(),
            Primitive::Plane { size } => write!(line, "plane size={}", f(size)?).unwrap(),
            Primitive::Grid { x_subdivisions, y_subdivisions, size } => {
                write!(line, "grid x_subdivisions={x_subdivisions} y_subdivisions={y_subdivisions} size={}", f(size)?).unwrap();
            }
            Primitive::UvSphere { segments, rings, radius } => {
                write!(line, "uv_sphere segments={segments} rings={rings} radius={}", f(radius)?).unwrap();
            }
            Primitive::IcoSphere { subdivisions, radius } => {
                write!(line, "ico_sphere subdivisions={subdivisions} radius={}", f(radius)?).unwrap();
            }
            Primitive::Cylinder { segments, radius, depth } => {
                write!(line, "cylinder segments={segments} radius={} depth={}", f(radius)?, f(depth)?).unwrap();
            }
            Primitive::Cone { segments, radius1, radius2, depth } => {
                write!(line, "cone segments={segments} radius1={} radius2={} depth={}", f(radius1)?, f(radius2)?, f(depth)?).unwrap();
            }
            Primitive::Torus { major_segments, minor_segments, major_radius, minor_radius } => {
                write!(
                    line,
                    "torus major_segments={major_segments} minor_segments={minor_segments} major_radius={} minor_radius={}",
                    f(major_radius)?,
                    f(minor_radius)?
                )
                .unwrap();
            }
        }
        let _ = write!(line, " as={h}");
        placement.write_opts(&mut line, true)?;
        self.transport.write_line(&line)?;
        Ok(ObjectRef(h))
    }

    pub fn mesh_new(&mut self, name: &str, mesh: &MeshData) -> Result<MeshRef, SdkError> {
        self.ensure_open()?;
        let h = self.alloc_handle();
        let opts = encode_mesh_options(mesh).map_err(SdkError::Wire)?;
        self.transport.write_line(&format!("mesh_new {} as={h} {opts}", quote(name)))?;
        Ok(MeshRef(h))
    }

    pub fn add_mesh_object(&mut self, name: &str, mesh: &MeshData, placement: &Placement) -> Result<ObjectRef, SdkError> {
        self.ensure_open()?;
        let h = self.alloc_handle();
        let opts = encode_mesh_options(mesh).map_err(SdkError::Wire)?;
        let mut line = format!("add_mesh {} as={h} {opts}", quote(name));
        placement.write_opts(&mut line, false)?;
        self.transport.write_line(&line)?;
        Ok(ObjectRef(h))
    }

    pub fn add_object_with_mesh(&mut self, name: &str, mesh: &MeshRef, placement: &Placement) -> Result<ObjectRef, SdkError> {
        self.ensure_open()?;
        let h = self.alloc_handle();
        let mut line = format!("add_object {} data={} as={h}", quote(name), mesh.token());
        placement.write_opts(&mut line, false)?;
        self.transport.write_line(&line)?;
        Ok(ObjectRef(h))
    }

    pub fn add_empty(&mut self, name: &str, placement: &Placement) -> Result<ObjectRef, SdkError> {
        self.ensure_open()?;
        let h = self.alloc_handle();
        let mut line = format!("add_empty {} as={h}", quote(name));
        placement.write_opts(&mut line, false)?;
        self.transport.write_line(&line)?;
        Ok(ObjectRef(h))
    }

    pub fn mesh_set(&mut self, obj_or_mesh: &str, mesh: &MeshData) -> Result<(), SdkError> {
        self.ensure_open()?;
        let opts = encode_mesh_options(mesh).map_err(SdkError::Wire)?;
        self.transport.write_line(&format!("mesh_set {obj_or_mesh} {opts}"))
    }

    pub fn shade_smooth(&mut self, obj: &ObjectRef, smooth: bool) -> Result<(), SdkError> {
        self.ensure_open()?;
        let op = if smooth { "shade_smooth" } else { "shade_flat" };
        self.transport.write_line(&format!("mesh_edit {} {op}", obj.token()))
    }

    pub fn translate_verts(&mut self, obj: &ObjectRef, delta: Vec3, indices: Option<&[u32]>) -> Result<(), SdkError> {
        self.ensure_open()?;
        let d = fmt_f32_list(&delta.to_array()).map_err(SdkError::Wire)?;
        let mut line = format!("mesh_edit {} translate_verts {d}", obj.token());
        if let Some(idx) = indices {
            let list = idx.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            let _ = write!(line, " indices={list}");
        }
        self.transport.write_line(&line)
    }

    // --- Lights & Cameras ----------------------------------------------------

    pub fn add_light(&mut self, spec: LightSpec, placement: &Placement) -> Result<ObjectRef, SdkError> {
        self.ensure_open()?;
        let h = self.alloc_handle();
        let f = |v: f32| fmt_f32(v).map_err(SdkError::Wire);
        let rgb = |c: Color| fmt_f32_list(&c.to_rgb_array()).map_err(SdkError::Wire);
        let mut line = String::from("light_add ");
        match spec {
            LightSpec::Point { power, color, radius } => {
                write!(line, "point power={} color={} radius={}", f(power)?, rgb(color)?, f(radius)?).unwrap();
            }
            LightSpec::Sun { power, color, radius } => {
                write!(line, "sun power={} color={} radius={}", f(power)?, rgb(color)?, f(radius)?).unwrap();
            }
            LightSpec::Spot { power, color, radius, angle, blend } => {
                write!(line, "spot power={} color={} radius={} angle={} blend={}", f(power)?, rgb(color)?, f(radius)?, f(angle)?, f(blend)?).unwrap();
            }
            LightSpec::Area { power, color, size } => {
                write!(line, "area power={} color={} size={}", f(power)?, rgb(color)?, f(size)?).unwrap();
            }
        }
        let _ = write!(line, " as={h}");
        placement.write_opts(&mut line, true)?;
        self.transport.write_line(&line)?;
        Ok(ObjectRef(h))
    }

    pub fn add_camera(&mut self, fov_radians: Option<f32>, active: bool, placement: &Placement) -> Result<ObjectRef, SdkError> {
        self.ensure_open()?;
        let h = self.alloc_handle();
        let mut line = format!("camera_add active={} as={h}", active as u8);
        if let Some(fov) = fov_radians {
            let s = fmt_f32(fov).map_err(SdkError::Wire)?;
            let _ = write!(line, " fov={s}");
        }
        placement.write_opts(&mut line, true)?;
        self.transport.write_line(&line)?;
        Ok(ObjectRef(h))
    }

    // --- Transforms, Parenting & Materials -----------------------------------

    pub fn set_location(&mut self, obj: &ObjectRef, loc: Vec3) -> Result<(), SdkError> {
        self.ensure_open()?;
        let s = fmt_f32_list(&loc.to_array()).map_err(SdkError::Wire)?;
        self.transport.write_line(&format!("set {}.location {s}", obj.token()))
    }

    pub fn set_rotation_euler(&mut self, obj: &ObjectRef, rot: Euler) -> Result<(), SdkError> {
        self.ensure_open()?;
        let s = fmt_f32_list(&rot.to_array()).map_err(SdkError::Wire)?;
        self.transport.write_line(&format!("set {}.rotation_euler {s}", obj.token()))
    }

    pub fn set_scale(&mut self, obj: &ObjectRef, scale: Vec3) -> Result<(), SdkError> {
        self.ensure_open()?;
        let s = fmt_f32_list(&scale.to_array()).map_err(SdkError::Wire)?;
        self.transport.write_line(&format!("set {}.scale {s}", obj.token()))
    }

    pub fn set_parent(&mut self, child: &ObjectRef, parent: Option<&ObjectRef>, keep_transform: bool) -> Result<(), SdkError> {
        self.ensure_open()?;
        let p = parent.map(|o| o.token()).unwrap_or("none");
        self.transport.write_line(&format!("parent {} {p} keep_transform={}", child.token(), keep_transform as u8))
    }

    pub fn delete(&mut self, obj: &ObjectRef) -> Result<(), SdkError> {
        self.ensure_open()?;
        self.transport.write_line(&format!("delete {}", obj.token()))
    }

    pub fn material_new(&mut self, name: &str, params: &MaterialParams) -> Result<MaterialRef, SdkError> {
        self.ensure_open()?;
        let h = self.alloc_handle();
        let f = |v: f32| fmt_f32(v).map_err(SdkError::Wire);
        let bc = fmt_f32_list(&params.base_color.to_rgba_array()).map_err(SdkError::Wire)?;
        let em = fmt_f32_list(&params.emission.to_array()).map_err(SdkError::Wire)?;
        self.transport.write_line(&format!(
            "material_new {} as={h} base_color={bc} metallic={} roughness={} emission={em} emission_strength={} ior={} transmission={} double_sided={}",
            quote(name),
            f(params.metallic)?,
            f(params.roughness)?,
            f(params.emission_strength)?,
            f(params.ior)?,
            f(params.transmission)?,
            params.double_sided as u8,
        ))?;
        Ok(MaterialRef(h))
    }

    pub fn material_assign(&mut self, obj: &ObjectRef, mat: &MaterialRef, slot: Option<u32>) -> Result<(), SdkError> {
        self.ensure_open()?;
        let mut line = format!("material_assign {} {}", obj.token(), mat.token());
        if let Some(s) = slot {
            let _ = write!(line, " slot={s}");
        }
        self.transport.write_line(&line)
    }

    pub fn modifier_add(&mut self, obj: &ObjectRef, spec: ModifierSpec) -> Result<(), SdkError> {
        self.ensure_open()?;
        let mut line = format!("modifier_add {} ", obj.token());
        spec.write_cmd(&mut line)?;
        self.transport.write_line(&line)
    }

    // --- Declarative UI Panels -----------------------------------------------

    pub fn ui_panel(&mut self, id: &str, label: &str, category: &str) -> Result<(), SdkError> {
        self.ensure_open()?;
        self.transport.write_line(&format!(
            "ui_panel {} label={} category={}",
            quote(id),
            quote(label),
            quote(category)
        ))
    }

    pub fn ui_prop(&mut self, panel_id: &str, spec: &UiPropSpec) -> Result<(), SdkError> {
        self.ensure_open()?;
        let mut line = format!(
            "ui_prop {} {} kind={} label={}",
            quote(panel_id),
            quote(&spec.key),
            spec.kind.as_str(),
            quote(&spec.label)
        );
        if let Some(def) = &spec.default {
            let _ = write!(line, " default={}", quote(def));
        }
        if let Some(lo) = spec.min {
            let s = fmt_f32(lo).map_err(SdkError::Wire)?;
            let _ = write!(line, " min={s}");
        }
        if let Some(hi) = spec.max {
            let s = fmt_f32(hi).map_err(SdkError::Wire)?;
            let _ = write!(line, " max={s}");
        }
        self.transport.write_line(&line)
    }

    // --- Bi-Directional Queries ----------------------------------------------

    pub fn ping(&mut self) -> Result<String, SdkError> {
        self.request("ping")
    }

    pub fn exists(&mut self, obj: &ObjectRef) -> Result<bool, SdkError> {
        let reply = self.request(&format!("exists {}", obj.token()))?;
        Ok(reply == "1")
    }

    pub fn get_location(&mut self, obj: &ObjectRef) -> Result<Vec3, SdkError> {
        let reply = self.request(&format!("get {}.location", obj.token()))?;
        let nums: Vec<f32> = reply
            .split_whitespace()
            .map(parse_floats)
            .collect::<Result<Vec<_>, _>>()
            .map_err(SdkError::Protocol)?
            .concat();
        match nums.as_slice() {
            [x, y, z] => Ok(Vec3::new(*x, *y, *z)),
            _ => Err(SdkError::Protocol(format!("expected 3 floats for location, got '{reply}'"))),
        }
    }

    pub fn get_vertex_count(&mut self, obj: &ObjectRef) -> Result<usize, SdkError> {
        let reply = self.request(&format!("get {}.vertex_count", obj.token()))?;
        reply
            .parse::<usize>()
            .map_err(|_| SdkError::Protocol(format!("invalid vertex_count '{reply}'")))
    }

    pub fn get_face_count(&mut self, obj: &ObjectRef) -> Result<usize, SdkError> {
        let reply = self.request(&format!("get {}.face_count", obj.token()))?;
        reply
            .parse::<usize>()
            .map_err(|_| SdkError::Protocol(format!("invalid face_count '{reply}'")))
    }

    /// Fetch live base or modifier-evaluated mesh geometry from Polyamour!
    /// Solves the 0-vertex primitive limitation of snapshot-only scripts.
    pub fn mesh_get(&mut self, obj: &ObjectRef, evaluated: bool) -> Result<MeshData, SdkError> {
        let cmd = if evaluated {
            format!("mesh_get {} evaluated=1", obj.token())
        } else {
            format!("mesh_get {}", obj.token())
        };
        let payload = self.request(&cmd)?;
        decode_mesh_options(&payload).map_err(SdkError::Protocol)
    }

    // --- Transaction Completion ----------------------------------------------

    pub fn finish(mut self) -> Result<T, SdkError> {
        self.ensure_open()?;
        self.transport.write_line("end")?;
        self.closed = true;
        Ok(self.transport)
    }

    pub fn abort(mut self, reason: &str) -> Result<T, SdkError> {
        if !self.closed {
            let _ = self.transport.write_line(&format!("abort {}", quote(reason)));
            self.closed = true;
        }
        Ok(self.transport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn wire_mesh_roundtrip_text_and_base64() {
        // Small mesh (<= 64 verts): plain CSV/semicolon encoding
        let small = MeshData::from_pydata(
            &[Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, -1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)],
            &[[0, 1], [1, 2]], // already in face, so dropped from loose_edges
            &[vec![0, 1, 2]],
        )
        .unwrap();
        assert!(small.loose_edges.is_empty());
        let encoded_small = encode_mesh_options(&small).unwrap();
        assert!(!encoded_small.contains("b64:"));
        assert_eq!(decode_mesh_options(&encoded_small).unwrap(), small);

        // Large mesh (> 64 verts): switches automatically to binary base64
        let verts: Vec<Vec3> = (0..80).map(|i| Vec3::new(i as f32 * 0.25, 1.5, -0.5)).collect();
        let edges: Vec<[u32; 2]> = (0..79).map(|i| [i, i + 1]).collect();
        let large = MeshData::from_pydata(&verts, &edges, &[]).unwrap();
        let encoded_large = encode_mesh_options(&large).unwrap();
        assert!(encoded_large.contains("verts=b64:"));
        assert!(encoded_large.contains("edges=b64:"));
        assert_eq!(decode_mesh_options(&encoded_large).unwrap(), large);
    }

    #[test]
    fn quat_euler_roundtrip_matches_blender_xyz() {
        let euler = Euler::new(0.35, -0.42, 1.1);
        let q = Quat::from_euler_xyz(euler);
        let back = q.to_euler_xyz();
        assert!((euler.x - back.x).abs() < 1e-5);
        assert!((euler.y - back.y).abs() < 1e-5);
        assert!((euler.z - back.z).abs() < 1e-5);
    }
}
