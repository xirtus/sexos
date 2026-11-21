// SPDX-License-Identifier: GPL-3.0-only
//! Autodesk FBX (7.x), binary and ASCII.
//!
//! Import reads the `Objects`/`Connections` graph:
//! - `Model` nodes (Mesh, Null, Camera, Light, LimbNode) with the full FBX
//!   transform (`Lcl` TRS, rotation order, pre/post rotation, pivots and
//!   offsets; geometric transforms are baked into the mesh) and their parent
//!   links;
//! - `Geometry` meshes: n-gons, `LayerElementUV`, `LayerElementMaterial`,
//!   `LayerElementSmoothing` (by polygon or by edge) and, without a smoothing
//!   layer, smooth/flat shading guessed from `LayerElementNormal`, plus
//!   `LayerElementColor` as a corner colour attribute;
//! - `Material` (Phong/Lambert properties mapped to Principled the way
//!   Blender does) and diffuse `Texture`s, embedded (`Video` `Content`) or
//!   fetched through the resolver;
//! - camera and light `NodeAttribute`s;
//! - `Skin` deformers: `LimbNode`s become one armature per skeleton, clusters
//!   become vertex groups and an Armature modifier.
//!
//! `GlobalSettings` axes and `UnitScaleFactor` are honoured: the file is
//! converted to the scene's Z-up metres by conjugating every local matrix
//! (so objects keep plain rotations and unit scale) and transforming the
//! geometry. FBX cameras look down their +X (up +Y) and lights shine down
//! their -Y; scene cameras and lights use -Z, so those nodes get a fixed turn.
//!
//! Export writes binary FBX 7.4, Y-up in metres (`UnitScaleFactor` 100) like
//! Blender's exporter with its default axes: meshes after their modifier
//! stack (except Armature), materials, embedded PNG textures, cameras,
//! lights, armatures as `LimbNode` skeletons and skinned meshes as `Skin`
//! clusters with a bind pose. Animation is not read or written yet.
pub mod tree;

use super::{decode_image, encode_png, Format, IoError, Resolver};
use crate::armature::{mat3_vec_to_roll, Armature, Bone, PoseBone};
use crate::math::{EulerOrder, Mat3, Mat4, Quat, Vec2, Vec3};
use crate::mesh::{AttrDomain, ColorAttribute};
use crate::mesh::{Face, Mesh};
use crate::modifiers::Modifier;
use crate::scene::{Camera, Light, LightKind, Material, Object, ObjectData, ParentType, Scene, Transform};
use std::collections::{HashMap, HashSet};
use tree::{object_name, Document, Node, Prop};

const F: Format = Format::Fbx;

fn perr(m: impl Into<String>) -> IoError {
    IoError::Parse { format: F, line: None, message: m.into() }
}

// ---------------------------------------------------------------- shared

/// The `Properties70` entries of a node, by name.
struct Props<'a>(HashMap<&'a str, &'a Node>);

impl<'a> Props<'a> {
    fn of(n: &'a Node) -> Props<'a> {
        let mut m = HashMap::new();
        if let Some(p) = n.child("Properties70") {
            for c in p.children_named("P") {
                if let Some(k) = c.str(0) {
                    m.insert(k, c);
                }
            }
        }
        Props(m)
    }
    /// Values start after name, type, label and flags.
    fn values(&self, k: &str) -> &'a [Prop] {
        self.0.get(k).map_or(&[], |n| n.props.get(4..).unwrap_or(&[]))
    }
    fn num(&self, k: &str, d: f64) -> f64 {
        self.values(k).first().and_then(Prop::as_f64).unwrap_or(d)
    }
    fn vec3(&self, k: &str, d: [f64; 3]) -> [f64; 3] {
        let v = self.values(k);
        if v.len() >= 3 {
            [0, 1, 2].map(|i| v[i].as_f64().unwrap_or(d[i]))
        } else {
            d
        }
    }
    fn has(&self, k: &str) -> bool {
        self.0.contains_key(k)
    }
}

fn v3(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32)
}

fn mat3_to_mat4(m: Mat3) -> Mat4 {
    m.to_mat4()
}

/// FBX `RotationOrder` (XYZ, XZY, YZX, YXZ, ZXY, ZYX, SphericXYZ).
fn euler_order(fbx: i64) -> EulerOrder {
    match fbx {
        1 => EulerOrder::XZY,
        2 => EulerOrder::YZX,
        3 => EulerOrder::YXZ,
        4 => EulerOrder::ZXY,
        5 => EulerOrder::ZYX,
        _ => EulerOrder::XYZ,
    }
}

fn euler_deg(d: [f64; 3], order: EulerOrder) -> Mat4 {
    mat3_to_mat4(Mat3::from_euler([d[0].to_radians() as f32, d[1].to_radians() as f32, d[2].to_radians() as f32], order))
}

/// A Model's local matrix, in file axes and units:
/// `T Roff Rp Rpre R Rpost⁻¹ Rp⁻¹ Soff Sp S Sp⁻¹`.
fn model_local(p: &Props) -> Mat4 {
    let order = euler_order(p.num("RotationOrder", 0.0) as i64);
    let t = Mat4::translation(v3(p.vec3("Lcl Translation", [0.0; 3])));
    let r = euler_deg(p.vec3("Lcl Rotation", [0.0; 3]), order);
    let s = Mat4::from_trs(Vec3::ZERO, Quat::IDENTITY, v3(p.vec3("Lcl Scaling", [1.0; 3])));
    let roff = Mat4::translation(v3(p.vec3("RotationOffset", [0.0; 3])));
    let rp = Mat4::translation(v3(p.vec3("RotationPivot", [0.0; 3])));
    let soff = Mat4::translation(v3(p.vec3("ScalingOffset", [0.0; 3])));
    let sp = Mat4::translation(v3(p.vec3("ScalingPivot", [0.0; 3])));
    let pre = euler_deg(p.vec3("PreRotation", [0.0; 3]), EulerOrder::XYZ);
    let post = euler_deg(p.vec3("PostRotation", [0.0; 3]), EulerOrder::XYZ);
    t * roff * rp * pre * r * post.inverse() * rp.inverse() * soff * sp * s * sp.inverse()
}

/// The geometric transform (object offset applied to the geometry only).
fn geometric(p: &Props) -> Mat4 {
    let t = v3(p.vec3("GeometricTranslation", [0.0; 3]));
    let r = euler_deg(p.vec3("GeometricRotation", [0.0; 3]), EulerOrder::XYZ);
    let s = v3(p.vec3("GeometricScaling", [1.0; 3]));
    Mat4::translation(t) * r * Mat4::from_trs(Vec3::ZERO, Quat::IDENTITY, s)
}

/// Rotation turning scene camera axes (look -Z, up +Y) into FBX camera axes
/// (look +X, up +Y): -90° about Y.
fn cam_fix() -> Mat4 {
    Mat4::from_trs(Vec3::ZERO, Quat::from_axis_angle(Vec3::Y, -core::f32::consts::FRAC_PI_2), Vec3::ONE)
}

/// Rotation turning scene light axes (-Z) into FBX light axes (-Y): -90° about X.
fn light_fix() -> Mat4 {
    Mat4::from_trs(Vec3::ZERO, Quat::from_axis_angle(Vec3::X, -core::f32::consts::FRAC_PI_2), Vec3::ONE)
}

/// Column-major 4×4 of doubles (FBX matrices) to `Mat4`.
fn mat_from(v: &[f64]) -> Option<Mat4> {
    (v.len() == 16).then(|| {
        let mut a = [0f32; 16];
        for (o, x) in a.iter_mut().zip(v) {
            *o = *x as f32;
        }
        Mat4::from_cols_array(a)
    })
}

fn mat_to(m: &Mat4) -> Vec<f64> {
    m.to_cols_array().iter().map(|&x| x as f64).collect()
}

// ---------------------------------------------------------------- import

/// The file-to-scene axis change as a signed permutation, from GlobalSettings.
fn axis_matrix(gs: &Props) -> Mat3 {
    let axis = |k: &str, d: f64| (gs.num(k, d) as i64).clamp(0, 2) as usize;
    let sign = |k: &str| if gs.num(k, 1.0) < 0.0 { -1.0f32 } else { 1.0 };
    let (up, front, coord) = (axis("UpAxis", 1.0), axis("FrontAxis", 2.0), axis("CoordAxis", 0.0));
    let (us, fs, cs) = (sign("UpAxisSign"), sign("FrontAxisSign"), sign("CoordAxisSign"));
    if up == front || up == coord || front == coord {
        // Inconsistent settings: assume the Y-up default.
        return axis_matrix(&Props(HashMap::new()));
    }
    // Scene X = coord, Z = up, -Y = front. Column j is where file axis j goes.
    let mut cols = [Vec3::ZERO; 3];
    cols[coord] = Vec3::X * cs;
    cols[up] = Vec3::Z * us;
    cols[front] = Vec3::Y * -fs;
    Mat3::from_cols(cols[0], cols[1], cols[2])
}

struct ModelInfo<'a> {
    id: i64,
    node: &'a Node,
    name: String,
    kind: String,
    props: Props<'a>,
}

struct Ctx<'a> {
    objects: HashMap<i64, &'a Node>,
    /// child -> [(parent, property)] in file order.
    parents: HashMap<i64, Vec<(i64, Option<String>)>>,
    /// parent -> [(child, property)] in file order.
    children: HashMap<i64, Vec<(i64, Option<String>)>>,
}

impl<'a> Ctx<'a> {
    fn class(&self, id: i64) -> Option<&'a str> {
        self.objects.get(&id).map(|n| n.name.as_str())
    }
    fn kids(&self, id: i64, class: &str) -> Vec<i64> {
        self.children.get(&id).map_or_else(Vec::new, |v| v.iter().filter(|(c, _)| self.class(*c) == Some(class)).map(|(c, _)| *c).collect())
    }
    fn kids_prop(&self, id: i64, class: &str) -> Vec<(i64, Option<String>)> {
        self.children.get(&id).map_or_else(Vec::new, |v| v.iter().filter(|(c, _)| self.class(*c) == Some(class)).cloned().collect())
    }
    fn parents_of(&self, id: i64, class: &str) -> Vec<i64> {
        self.parents.get(&id).map_or_else(Vec::new, |v| v.iter().filter(|(p, _)| self.class(*p) == Some(class)).map(|(p, _)| *p).collect())
    }
}

/// Read a layer element's per-corner values: `stride` numbers each, for
/// polygon corners `corner_vert` (control point of each corner) and polygon
/// of each corner `corner_poly`.
fn layer_values(le: &Node, data_name: &str, index_name: &str, stride: usize, corner_vert: &[u32], corner_poly: &[u32], nverts: usize) -> Option<Vec<Vec<f64>>> {
    let data = le.child_f64s(data_name)?;
    let mapping = le.child_str("MappingInformationType").unwrap_or("ByPolygonVertex");
    let reference = le.child_str("ReferenceInformationType").unwrap_or("Direct");
    let index = if reference == "IndexToDirect" || reference == "Index" { le.child_i64s(index_name) } else { None };
    let n = data.len() / stride;
    let get = |i: usize| -> Vec<f64> {
        let j = match &index {
            Some(ix) => ix.get(i).copied().unwrap_or(-1),
            None => i as i64,
        };
        if j >= 0 && (j as usize) < n {
            data[j as usize * stride..j as usize * stride + stride].to_vec()
        } else {
            vec![0.0; stride]
        }
    };
    Some(match mapping {
        "ByPolygonVertex" => (0..corner_vert.len()).map(get).collect(),
        "ByVertice" | "ByVertex" | "ByControlPoint" => corner_vert.iter().map(|&v| if (v as usize) < nverts { get(v as usize) } else { vec![0.0; stride] }).collect(),
        "ByPolygon" => corner_poly.iter().map(|&p| get(p as usize)).collect(),
        "AllSame" => vec![get(0); corner_vert.len()],
        _ => return None,
    })
}

/// A geometry converted to a mesh in file axes (the caller transforms it).
fn read_geometry(g: &Node) -> Result<Mesh, IoError> {
    let verts = g.child_f64s("Vertices").unwrap_or_default();
    let pvi = g.child_i64s("PolygonVertexIndex").unwrap_or_default();
    let nverts = verts.len() / 3;
    let mut mesh = Mesh::default();
    mesh.positions = verts.chunks_exact(3).map(|c| Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32)).collect();
    // Polygons and per-corner bookkeeping.
    let mut polys: Vec<std::ops::Range<usize>> = Vec::new();
    let mut corner_vert = Vec::with_capacity(pvi.len());
    let mut corner_poly = Vec::with_capacity(pvi.len());
    let mut start = 0;
    for (i, &raw) in pvi.iter().enumerate() {
        let (v, end) = if raw < 0 { (-raw - 1, true) } else { (raw, false) };
        if v as usize >= nverts {
            return Err(perr(format!("{}: polygon vertex {v} out of range", object_name(g.str(1).unwrap_or("Geometry")))));
        }
        corner_vert.push(v as u32);
        corner_poly.push(polys.len() as u32);
        if end {
            polys.push(start..i + 1);
            start = i + 1;
        }
    }
    if start < pvi.len() {
        polys.push(start..pvi.len());
        corner_poly.truncate(start);
        corner_vert.truncate(start);
        polys.pop();
    }
    let layer = |name: &'static str| g.children_named(name).next();
    let uvs = layer("LayerElementUV").and_then(|le| layer_values(le, "UV", "UVIndex", 2, &corner_vert, &corner_poly, nverts));
    let mats = layer("LayerElementMaterial").and_then(|le| {
        let m = le.child_i64s("Materials")?;
        let mapping = le.child_str("MappingInformationType").unwrap_or("AllSame");
        Some(if mapping == "AllSame" { vec![m.first().copied().unwrap_or(0); polys.len()] } else { m })
    });
    let smoothing = layer("LayerElementSmoothing").and_then(|le| Some((le.child_i64s("Smoothing")?, le.child_str("MappingInformationType").unwrap_or("ByPolygon").to_string())));
    let normals = layer("LayerElementNormal").and_then(|le| layer_values(le, "Normals", "NormalsIndex", 3, &corner_vert, &corner_poly, nverts));
    let colors = layer("LayerElementColor").and_then(|le| layer_values(le, "Colors", "ColorIndex", 4, &corner_vert, &corner_poly, nverts));

    for (pi, r) in polys.iter().enumerate() {
        if r.len() < 3 {
            for w in corner_vert[r.clone()].windows(2) {
                mesh.loose_edges.push([w[0], w[1]]);
            }
            continue;
        }
        let mut f = Face::new(corner_vert[r.clone()].to_vec());
        if let Some(uv) = &uvs {
            f.uvs = r.clone().map(|c| Vec2::new(uv[c][0] as f32, uv[c][1] as f32)).collect();
        }
        if let Some(m) = &mats {
            f.material = m.get(pi).copied().unwrap_or(0).clamp(0, u16::MAX as i64) as u16;
        }
        mesh.faces.push(f);
    }
    // Shading.
    let face_corners: Vec<std::ops::Range<usize>> = polys.iter().filter(|r| r.len() >= 3).cloned().collect();
    match smoothing {
        Some((s, mapping)) if mapping == "ByPolygon" => {
            let mut k = 0;
            for (pi, r) in polys.iter().enumerate() {
                if r.len() >= 3 {
                    mesh.faces[k].smooth = s.get(pi).copied().unwrap_or(0) != 0;
                    k += 1;
                }
            }
        }
        Some((s, mapping)) if mapping == "ByEdge" => {
            // Smooth faces, edges with 0 marked sharp. `Edges` lists the
            // polygon-vertex index each edge starts at.
            for f in &mut mesh.faces {
                f.smooth = true;
            }
            let edges = g.child_i64s("Edges").unwrap_or_default();
            for (e, &pv) in edges.iter().enumerate() {
                if s.get(e).copied().unwrap_or(1) != 0 || pv < 0 || pv as usize >= corner_vert.len() {
                    continue;
                }
                let pv = pv as usize;
                let r = &polys[corner_poly[pv] as usize];
                let next = if pv + 1 < r.end { pv + 1 } else { r.start };
                let (a, b) = (corner_vert[pv], corner_vert[next]);
                mesh.sharp_edges.insert((a.min(b), a.max(b)));
            }
        }
        _ => {
            if let Some(n) = &normals {
                // Smooth when a corner normal leaves the face normal.
                for (k, r) in face_corners.iter().enumerate() {
                    let fnorm = mesh.face_normal(k);
                    mesh.faces[k].smooth = r.clone().any(|c| {
                        let v = Vec3::new(n[c][0] as f32, n[c][1] as f32, n[c][2] as f32);
                        v.length() > 1e-6 && v.normalize().dot(fnorm) < 0.9999
                    });
                }
            }
        }
    }
    if let Some(c) = colors {
        let data: Vec<[f32; 4]> = face_corners.iter().flat_map(|r| r.clone()).map(|i| [c[i][0] as f32, c[i][1] as f32, c[i][2] as f32, c[i][3] as f32]).collect();
        mesh.color_attributes.push(ColorAttribute { name: "Col".into(), domain: AttrDomain::Corner, data, byte: true });
    }
    Ok(mesh)
}

fn read_material(n: &Node) -> Material {
    let p = Props::of(n);
    let mut m = Material { name: object_name(n.str(1).unwrap_or("Material")).to_string(), ..Default::default() };
    if m.name.is_empty() {
        m.name = "Material".into();
    }
    let dc = p.vec3("DiffuseColor", p.vec3("Diffuse", [0.8; 3]));
    let df = if p.has("DiffuseFactor") { p.num("DiffuseFactor", 1.0) } else { 1.0 };
    m.base_color = [(dc[0] * df) as f32, (dc[1] * df) as f32, (dc[2] * df) as f32, 1.0];
    let ec = p.vec3("EmissiveColor", p.vec3("Emissive", [0.0; 3]));
    let ef = p.num("EmissiveFactor", 1.0);
    m.emission = v3(ec);
    m.emission_strength = if m.emission == Vec3::ZERO { 0.0 } else { ef as f32 };
    // Blender: alpha from Opacity, else 1 - TransparencyFactor (only when
    // TransparentColor is not black).
    let alpha = if p.has("Opacity") {
        p.num("Opacity", 1.0)
    } else {
        let tc = p.vec3("TransparentColor", [1.0; 3]);
        let tf = p.num("TransparencyFactor", 0.0);
        if tc.iter().any(|&c| c > 0.0) {
            1.0 - tf * (tc[0] + tc[1] + tc[2]) / 3.0
        } else {
            1.0
        }
    };
    m.base_color[3] = alpha.clamp(0.0, 1.0) as f32;
    let shininess = p.num("ShininessExponent", p.num("Shininess", 20.0)).clamp(0.0, 100.0);
    m.roughness = (1.0 - shininess.sqrt() / 10.0).clamp(0.0, 1.0) as f32;
    m.metallic = p.num("ReflectionFactor", 0.0).clamp(0.0, 1.0) as f32;
    m
}

/// A camera attribute; `unit` converts file distances to metres.
fn read_camera(attr: Option<&Node>, unit: f32) -> Camera {
    let mut c = Camera::default();
    let Some(a) = attr else { return c };
    let p = Props::of(a);
    if p.has("FilmWidth") {
        c.sensor_width = (p.num("FilmWidth", 1.417) * 25.4) as f32;
        c.sensor_height = (p.num("FilmHeight", 0.945) * 25.4) as f32;
    }
    let aspect = {
        let (w, h) = (p.num("AspectWidth", 0.0), p.num("AspectHeight", 0.0));
        if w > 0.0 && h > 0.0 {
            (w / h) as f32
        } else {
            16.0 / 9.0
        }
    };
    if p.has("FocalLength") {
        c.set_focal_length(p.num("FocalLength", 50.0) as f32, aspect);
    } else if p.has("FieldOfView") {
        // Horizontal field of view in degrees.
        let fx = (p.num("FieldOfView", 40.0).to_radians() * 0.5).tan() as f32;
        c.fov_y = 2.0 * (fx / aspect).atan();
    }
    c.near = p.num("NearPlane", 10.0) as f32 * unit;
    c.far = p.num("FarPlane", 4000.0) as f32 * unit;
    if p.num("CameraProjectionType", 0.0) as i64 == 1 {
        c.ortho_scale = Some(p.num("OrthoZoom", 1.0).max(1e-3) as f32 * 2.0);
    }
    c
}

fn read_light(attr: Option<&Node>) -> Light {
    let mut l = Light::default();
    let Some(a) = attr else { return l };
    let p = Props::of(a);
    l.color = v3(p.vec3("Color", [1.0; 3]));
    let intensity = p.num("Intensity", 100.0) as f32;
    l.kind = match p.num("LightType", 0.0) as i64 {
        1 => LightKind::Sun,
        2 => {
            let outer = p.num("OuterAngle", p.num("ConeAngle", 45.0)) as f32;
            let inner = p.num("InnerAngle", outer as f64) as f32;
            LightKind::Spot { angle: outer.to_radians().clamp(1e-3, core::f32::consts::PI), blend: if outer > 0.0 { (1.0 - inner / outer).clamp(0.0, 1.0) } else { 0.0 } }
        }
        3 => LightKind::Area { size: 1.0 },
        _ => LightKind::Point,
    };
    // Intensity 100 is "full": a 1.0 strength sun, a 1000 W lamp.
    l.power = match l.kind {
        LightKind::Sun => intensity / 100.0,
        _ => intensity * 10.0,
    };
    l.cast_shadow = p.num("CastShadows", 1.0) != 0.0;
    l
}

fn resolve_texture(ctx: &Ctx, tex: i64, resolver: Resolver) -> Option<(String, Vec<u8>)> {
    let tnode = ctx.objects.get(&tex)?;
    let videos = ctx.kids(tex, "Video");
    let mut names: Vec<String> = Vec::new();
    for v in &videos {
        let vn = ctx.objects[v];
        if let Some(Prop::Raw(bytes)) = vn.child("Content").and_then(|c| c.prop(0)) {
            if !bytes.is_empty() {
                return Some((vn.child_str("RelativeFilename").or(vn.child_str("Filename")).unwrap_or("texture").to_string(), bytes.clone()));
            }
        }
        names.extend(vn.child_str("RelativeFilename").map(String::from));
        names.extend(vn.child_str("Filename").map(String::from));
    }
    names.extend(tnode.child_str("RelativeFilename").map(String::from));
    names.extend(tnode.child_str("FileName").map(String::from));
    for n in &names {
        if n.is_empty() {
            continue;
        }
        let n = n.replace('\\', "/");
        if let Some(b) = resolver(&n) {
            return Some((n, b));
        }
        let base = n.rsplit('/').next().unwrap_or(&n).to_string();
        if let Some(b) = resolver(&base) {
            return Some((base, b));
        }
    }
    None
}

pub fn import_into(scene: &mut Scene, bytes: &[u8], resolver: Resolver) -> Result<Vec<usize>, IoError> {
    let doc = tree::parse(bytes).map_err(perr)?;
    import_doc(scene, &doc, resolver)
}

fn import_doc(scene: &mut Scene, doc: &Document, resolver: Resolver) -> Result<Vec<usize>, IoError> {
    if doc.version < 7000 {
        return Err(IoError::Unsupported { format: F, message: format!("FBX {}.{} (only 7.x files are read; re-save as FBX 2011 or newer)", doc.version / 1000, doc.version % 1000 / 100) });
    }
    let objects_node = doc.get("Objects").ok_or_else(|| perr("no Objects section"))?;
    let mut ctx = Ctx { objects: HashMap::new(), parents: HashMap::new(), children: HashMap::new() };
    for o in &objects_node.children {
        if let Some(id) = o.i64(0) {
            ctx.objects.insert(id, o);
        }
    }
    if let Some(conns) = doc.get("Connections") {
        for c in conns.children_named("C") {
            let (Some(child), Some(parent)) = (c.i64(1), c.i64(2)) else { continue };
            let prop = if c.str(0) == Some("OP") { c.str(3).map(String::from) } else { None };
            ctx.parents.entry(child).or_default().push((parent, prop.clone()));
            ctx.children.entry(parent).or_default().push((child, prop));
        }
    }

    // Axes and units.
    let gs_node = doc.get("GlobalSettings");
    let empty = Node::default();
    let gs = Props::of(gs_node.unwrap_or(&empty));
    let a3 = axis_matrix(&gs);
    let a = a3.to_mat4();
    let unit = (gs.num("UnitScaleFactor", 1.0) / 100.0) as f32;
    let unit = if unit.is_finite() && unit > 0.0 { unit } else { 0.01 };
    let m = a * Mat4::from_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::splat(unit));
    let m_inv = m.inverse();
    let mirrored = a3.determinant() < 0.0;
    let conj = |l: &Mat4| m * *l * m_inv;

    // Models, in file order.
    let models: Vec<ModelInfo> = objects_node
        .children_named("Model")
        .filter_map(|n| {
            Some(ModelInfo { id: n.i64(0)?, node: n, name: object_name(n.str(1).unwrap_or("Object")).to_string(), kind: n.str(2).unwrap_or("Null").to_string(), props: Props::of(n) })
        })
        .collect();
    let model_index: HashMap<i64, usize> = models.iter().enumerate().map(|(i, mi)| (mi.id, i)).collect();
    let model_parent: Vec<Option<usize>> = models.iter().map(|mi| ctx.parents_of(mi.id, "Model").first().and_then(|p| model_index.get(p).copied())).collect();
    // File-axis world matrices of every model (current pose).
    let locals: Vec<Mat4> = models.iter().map(|mi| model_local(&mi.props)).collect();
    let mut world_f: Vec<Option<Mat4>> = vec![None; models.len()];
    fn world_of(i: usize, locals: &[Mat4], parent: &[Option<usize>], memo: &mut Vec<Option<Mat4>>, depth: usize) -> Mat4 {
        if let Some(w) = memo[i] {
            return w;
        }
        let w = match parent[i] {
            Some(p) if depth < locals.len() => world_of(p, locals, parent, memo, depth + 1) * locals[i],
            _ => locals[i],
        };
        memo[i] = Some(w);
        w
    }
    for i in 0..models.len() {
        world_of(i, &locals, &model_parent, &mut world_f, 0);
    }
    let world_f: Vec<Mat4> = world_f.into_iter().map(|w| w.unwrap_or_default()).collect();

    // Materials and textures.
    let mut material_ids: HashMap<i64, usize> = HashMap::new();
    let mut image_cache: HashMap<String, usize> = HashMap::new();
    for n in objects_node.children_named("Material") {
        let Some(id) = n.i64(0) else { continue };
        let mut mat = read_material(n);
        for (tex, prop) in ctx.kids_prop(id, "Texture") {
            let p = prop.as_deref().unwrap_or("");
            if !(p == "DiffuseColor" || p == "Diffuse" || p.eq_ignore_ascii_case("Maya|baseColor") || p == "3dsMax|Parameters|base_color_map" || p.is_empty()) {
                continue;
            }
            if let Some((name, data)) = resolve_texture(&ctx, tex, resolver) {
                let idx = match image_cache.get(&name) {
                    Some(&i) => Some(i),
                    None => decode_image(&name, &data).ok().map(|img| {
                        scene.images.push(img);
                        image_cache.insert(name.clone(), scene.images.len() - 1);
                        scene.images.len() - 1
                    }),
                };
                if idx.is_some() {
                    mat.base_color_texture = idx;
                    // Blender multiplies nothing: the texture replaces the colour.
                    mat.base_color = [1.0, 1.0, 1.0, mat.base_color[3]];
                    break;
                }
            }
        }
        material_ids.insert(id, scene.add_material(mat));
    }

    // Skeletons: LimbNodes and every model a skin cluster links to.
    let mut is_bone = vec![false; models.len()];
    // geometry id -> [(bone model, indexes, weights, Transform, TransformLink)]
    let mut clusters: HashMap<i64, Vec<(usize, Vec<i64>, Vec<f64>, Option<Mat4>, Option<Mat4>)>> = HashMap::new();
    for (i, mi) in models.iter().enumerate() {
        if mi.kind == "LimbNode" || mi.kind == "Root" {
            is_bone[i] = true;
        }
    }
    for n in objects_node.children_named("Deformer") {
        if n.str(2) != Some("Skin") {
            continue;
        }
        let Some(skin) = n.i64(0) else { continue };
        let geoms = ctx.parents_of(skin, "Geometry");
        for cl in ctx.kids(skin, "Deformer") {
            let cn = ctx.objects[&cl];
            let Some(&bone) = ctx.kids(cl, "Model").first().and_then(|b| model_index.get(b)) else { continue };
            is_bone[bone] = true;
            let idx = cn.child_i64s("Indexes").unwrap_or_default();
            let w = cn.child_f64s("Weights").unwrap_or_default();
            let t = cn.child_f64s("Transform").and_then(|v| mat_from(&v));
            let tl = cn.child_f64s("TransformLink").and_then(|v| mat_from(&v));
            for g in &geoms {
                clusters.entry(*g).or_default().push((bone, idx.clone(), w.clone(), t, tl));
            }
        }
    }
    // Bind poses from the Pose node, for bones without a cluster.
    let mut bind_pose: HashMap<usize, Mat4> = HashMap::new();
    for pose in objects_node.children_named("Pose") {
        for pn in pose.children_named("PoseNode") {
            let (Some(node), Some(mat)) = (pn.child_i64s("Node").and_then(|v| v.first().copied()), pn.child_f64s("Matrix").and_then(|v| mat_from(&v))) else { continue };
            if let Some(&i) = model_index.get(&node) {
                bind_pose.entry(i).or_insert(mat);
            }
        }
    }
    for cl in clusters.values() {
        for (b, _, _, _, tl) in cl {
            if let Some(tl) = tl {
                bind_pose.insert(*b, *tl);
            }
        }
    }

    // One armature per group of skeleton roots sharing a parent.
    let bone_parent = |i: usize| model_parent[i].filter(|&p| is_bone[p]);
    let mut arm_of_root: HashMap<Option<usize>, usize> = HashMap::new(); // host model -> group
    struct Group {
        host: Option<usize>,
        members: Vec<usize>,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut bone_group: Vec<Option<usize>> = vec![None; models.len()];
    for i in 0..models.len() {
        if !is_bone[i] {
            continue;
        }
        let mut root = i;
        let mut guard = 0;
        while let Some(p) = bone_parent(root) {
            root = p;
            guard += 1;
            if guard > models.len() {
                break;
            }
        }
        let host = model_parent[root];
        let g = *arm_of_root.entry(host).or_insert_with(|| {
            groups.push(Group { host, members: Vec::new() });
            groups.len() - 1
        });
        groups[g].members.push(i);
        bone_group[i] = Some(g);
    }

    // Scene objects for non-bone models.
    let mut added = Vec::new();
    let mut model_object: Vec<Option<usize>> = vec![None; models.len()];
    // Camera and light turns, all known before any child needs its parent's.
    let model_attr = |mi: &ModelInfo| ctx.kids(mi.id, "NodeAttribute").first().map(|a| ctx.objects[a]);
    let model_geom = |mi: &ModelInfo| ctx.kids(mi.id, "Geometry").into_iter().find(|g| ctx.objects[g].str(2).is_none_or(|t| t == "Mesh"));
    let fix_of: Vec<Option<Mat4>> = models
        .iter()
        .enumerate()
        .map(|(i, mi)| {
            if is_bone[i] || model_geom(mi).is_some() {
                return None;
            }
            match (mi.kind.as_str(), model_attr(mi).and_then(|a| a.str(2))) {
                ("Camera", _) | (_, Some("Camera")) => Some(a * cam_fix()),
                ("Light", _) | (_, Some("Light")) => Some(a * light_fix()),
                _ => None,
            }
        })
        .collect();
    let mut geom_mesh: HashMap<i64, usize> = HashMap::new();
    let mut mesh_object_geom: Vec<(usize, i64, usize)> = Vec::new(); // (object, geometry, model)
    for (i, mi) in models.iter().enumerate() {
        if is_bone[i] {
            continue;
        }
        let attr = model_attr(mi);
        let geom = model_geom(mi);
        let fix = fix_of[i];
        let data = if let Some(gid) = geom {
            let gm = geometric(&mi.props);
            let plain = gm.approx_eq(&Mat4::IDENTITY, 1e-6);
            let mesh_idx = match geom_mesh.get(&gid) {
                Some(&mi_) if plain => mi_,
                _ => {
                    let gnode = ctx.objects[&gid];
                    let mut mesh = read_geometry(gnode)?;
                    mesh.transform(&(m * gm));
                    if mirrored {
                        mesh.flip_normals();
                    }
                    let name = object_name(gnode.str(1).unwrap_or("Mesh"));
                    let idx = scene.add_mesh(if name.is_empty() { &mi.name } else { name }, mesh);
                    if plain {
                        geom_mesh.insert(gid, idx);
                    }
                    idx
                }
            };
            ObjectData::Mesh(mesh_idx)
        } else {
            match (mi.kind.as_str(), attr.and_then(|a| a.str(2))) {
                ("Camera", _) | (_, Some("Camera")) => ObjectData::Camera(read_camera(attr, unit)),
                ("Light", _) | (_, Some("Light")) => ObjectData::Light(read_light(attr)),
                _ => ObjectData::Empty,
            }
        };
        let mut o = Object::new(&mi.name, data);
        // Children of a turned node undo its turn.
        let parent_fix = model_parent[i].and_then(|p| fix_of[p]).map(|f| f.inverse()).unwrap_or(Mat4::IDENTITY);
        let mut local = parent_fix * conj(&locals[i]);
        if let Some(f) = &fix {
            local = local * *f;
        }
        o.transform = Transform::from_matrix(&local);
        o.visible = mi.props.num("Visibility", 1.0) != 0.0 && mi.node.child("Visibility").and_then(|v| v.prop(0)).and_then(Prop::as_f64).unwrap_or(1.0) != 0.0;
        if let ObjectData::Mesh(_) = o.data {
            o.materials = ctx.kids(mi.id, "Material").iter().filter_map(|mid| material_ids.get(mid).copied()).collect();
        }
        let oi = scene.add_object(o);
        added.push(oi);
        model_object[i] = Some(oi);
        if let Some(g) = geom {
            mesh_object_geom.push((oi, g, i));
        }
    }

    // Armatures.
    let mut group_object: Vec<Option<usize>> = vec![None; groups.len()];
    let mut model_bone: HashMap<usize, (usize, String, f32)> = HashMap::new(); // model -> (group, bone name, length)
    for (g, grp) in groups.iter().enumerate() {
        // Armature space: the host's world (scene axes).
        let host_world = grp.host.map_or(Mat4::IDENTITY, |h| conj(&world_f[h]));
        let host_inv = host_world.inverse();
        let rest_of = |j: usize| host_inv * conj(bind_pose.get(&j).unwrap_or(&world_f[j]));
        let pose_of = |j: usize| host_inv * conj(&world_f[j]);
        // Parents first.
        let mut order = Vec::new();
        let mut done = HashSet::new();
        while order.len() < grp.members.len() {
            let before = order.len();
            for &j in &grp.members {
                if !done.contains(&j) && bone_parent(j).is_none_or(|p| done.contains(&p)) {
                    done.insert(j);
                    order.push(j);
                }
            }
            if order.len() == before {
                break;
            }
        }
        let heads: HashMap<usize, Vec3> = order.iter().map(|&j| (j, rest_of(j).translation_part())).collect();
        let mut arm = Armature::default();
        let mut bone_idx: HashMap<usize, usize> = HashMap::new();
        let mut lengths: HashMap<usize, f32> = HashMap::new();
        for &j in &order {
            let (head, r, _) = rest_of(j).to_loc_mat3_scale();
            let child = order.iter().filter(|&&c| bone_parent(c) == Some(j)).map(|c| heads[c].distance(head)).find(|&d| d > 1e-5);
            let len = child.or_else(|| bone_parent(j).and_then(|p| lengths.get(&p).copied())).unwrap_or(0.1 * unit / 0.01).max(1e-4);
            lengths.insert(j, len);
            let v = r.mul_vec(Vec3::Y) * len;
            let mut b = Bone::new(&models[j].name, head, head + v);
            b.roll = mat3_vec_to_roll(&r, v);
            b.parent = bone_parent(j).and_then(|p| bone_idx.get(&p).copied());
            let bi = arm.add_bone(b);
            bone_idx.insert(j, bi);
            model_bone.insert(j, (g, arm.bones[bi].name.clone(), len));
        }
        // Pose: each bone at its node's current transform.
        let mut pose_mats = vec![Mat4::IDENTITY; arm.bones.len()];
        let mut pose = Vec::new();
        for &j in &order {
            let bi = bone_idx[&j];
            let target = pose_of(j);
            let pp = arm.parent(bi).map(|p| pose_mats[p]);
            let chan = arm.parent_transform(bi, pp.as_ref()).inverse().apply(&target);
            pose_mats[bi] = target;
            if !chan.approx_eq(&Mat4::IDENTITY, 1e-5) {
                let mut pb = PoseBone::new(&arm.bones[bi].name);
                pb.set_from_matrix(&chan);
                pose.push(pb);
            }
        }
        // The host Null becomes the armature object when it carries nothing else.
        let host_obj = grp.host.and_then(|h| model_object[h]).filter(|&o| matches!(scene.objects[o].data, ObjectData::Empty) && !groups.iter().enumerate().any(|(g2, x)| g2 < g && x.host == grp.host));
        let name = grp.host.map_or_else(|| "Armature".to_string(), |h| models[h].name.clone());
        let ai = scene.add_armature(&name, arm);
        let oi = match host_obj {
            Some(o) => {
                scene.objects[o].data = ObjectData::Armature(ai);
                scene.objects[o].pose = pose;
                o
            }
            None => {
                let mut o = Object::new(&name, ObjectData::Armature(ai));
                o.pose = pose;
                o.parent = grp.host.and_then(|h| model_object[h]);
                let oi = scene.add_object(o);
                added.push(oi);
                oi
            }
        };
        group_object[g] = Some(oi);
    }

    // Parents: plain models, or bones (object hangs at the bone's tail).
    for (i, oi) in model_object.iter().enumerate() {
        let Some(oi) = *oi else { continue };
        let Some(p) = model_parent[i] else { continue };
        if let Some(po) = model_object[p] {
            if po != oi {
                scene.objects[oi].parent = Some(po);
            }
        } else if let Some((g, name, len)) = model_bone.get(&p).cloned() {
            let ob = &mut scene.objects[oi];
            ob.parent = group_object[g];
            ob.parent_type = ParentType::Bone;
            ob.parent_bone = name;
            // The bone frame is the node frame moved to the tail.
            ob.transform.location -= Vec3::Y * len;
        }
    }

    // Skinned meshes: weights, Armature modifier, parented to the armature.
    let mut weighted: HashSet<usize> = HashSet::new();
    for (oi, gid, mi) in mesh_object_geom {
        let Some(cl) = clusters.get(&gid) else { continue };
        let Some(&(g, _, _)) = cl.first().and_then(|c| model_bone.get(&c.0)) else { continue };
        let Some(ao) = group_object[g] else { continue };
        let ObjectData::Mesh(mesh_i) = scene.objects[oi].data else { continue };
        if weighted.insert(mesh_i) {
            let mesh = &mut scene.meshes[mesh_i];
            let nv = mesh.positions.len();
            for (bone, idx, w, _, _) in cl {
                let Some((_, name, _)) = model_bone.get(bone) else { continue };
                let vg = mesh.ensure_vertex_group(name);
                for (k, &v) in idx.iter().enumerate() {
                    if v >= 0 && (v as usize) < nv {
                        mesh.set_weight(v as usize, vg, w.get(k).copied().unwrap_or(0.0) as f32);
                    }
                }
            }
        }
        // Where the mesh sat at bind time. The FBX SDK reads `Transform` as
        // the mesh's world matrix; Blender (and files it wrote) stores it
        // relative to the bone (`TransformLink⁻¹ · mesh`). Take whichever
        // reading lands nearer the mesh's current placement.
        let near = |m: &Mat4| m.to_cols_array().iter().zip(world_f[mi].to_cols_array()).map(|(a, b)| (a - b).abs()).sum::<f32>();
        let bind = cl
            .iter()
            .find_map(|c| {
                let t = c.3?;
                Some(match c.4 {
                    Some(tl) if near(&(tl * t)) < near(&t) => tl * t,
                    _ => t,
                })
            })
            .unwrap_or(world_f[mi]);
        let arm_world = scene.world_matrix(ao);
        let arm_name = scene.objects[ao].name.clone();
        let o = &mut scene.objects[oi];
        o.parent = Some(ao);
        o.parent_type = ParentType::Armature;
        o.transform = Transform::from_matrix(&(arm_world.inverse() * conj(&bind)));
        let mut md = Modifier::with_defaults("Armature").expect("armature modifier");
        if let Modifier::Armature { object, .. } = &mut md {
            *object = arm_name;
        }
        o.modifiers.insert(0, md);
    }
    Ok(added)
}

// ---------------------------------------------------------------- export

struct Ids(i64);
impl Ids {
    fn next(&mut self) -> i64 {
        self.0 += 1;
        self.0
    }
}

fn s(v: &str) -> Prop {
    Prop::Str(v.into())
}

fn named(name: &str, class: &str) -> Prop {
    Prop::Str(format!("{name}\0\u{1}{class}"))
}

fn p_int(name: &str, v: i32) -> Node {
    Node::new("P", vec![s(name), s("int"), s("Integer"), s(""), Prop::I32(v)])
}
fn p_enum(name: &str, v: i32) -> Node {
    Node::new("P", vec![s(name), s("enum"), s(""), s(""), Prop::I32(v)])
}
fn p_num(name: &str, v: f64) -> Node {
    Node::new("P", vec![s(name), s("double"), s("Number"), s(""), Prop::F64(v)])
}
fn p_anim_num(name: &str, v: f64) -> Node {
    Node::new("P", vec![s(name), s("Number"), s(""), s("A"), Prop::F64(v)])
}
fn p_vec(name: &str, ty: &str, label: &str, flags: &str, v: [f64; 3]) -> Node {
    Node::new("P", vec![s(name), s(ty), s(label), s(flags), Prop::F64(v[0]), Prop::F64(v[1]), Prop::F64(v[2])])
}
fn p_color(name: &str, v: [f64; 3]) -> Node {
    p_vec(name, "Color", "", "A", v)
}
fn p_str(name: &str, v: &str) -> Node {
    Node::new("P", vec![s(name), s("KString"), s(""), s(""), s(v)])
}
fn props70(ps: Vec<Node>) -> Node {
    Node { name: "Properties70".into(), props: vec![], children: ps }
}
fn leaf(name: &str, p: Prop) -> Node {
    Node::new(name, vec![p])
}
fn conn(child: i64, parent: i64) -> Node {
    Node::new("C", vec![s("OO"), Prop::I64(child), Prop::I64(parent)])
}
fn conn_prop(child: i64, parent: i64, prop: &str) -> Node {
    Node::new("C", vec![s("OP"), Prop::I64(child), Prop::I64(parent), s(prop)])
}

/// Scene (Z-up metres) to file (Y-up) axes: (x, y, z) -> (x, z, -y).
fn export_axes() -> Mat4 {
    Mat3::from_cols(Vec3::X, Vec3::new(0.0, 0.0, -1.0), Vec3::Y).to_mat4()
}

fn model_props(local_f: &Mat4) -> Node {
    let (t, r, sc) = local_f.to_loc_mat3_scale();
    let e = r.to_euler(EulerOrder::XYZ);
    props70(vec![
        p_enum("RotationOrder", 0),
        p_int("DefaultAttributeIndex", 0),
        p_enum("InheritType", 1),
        p_vec("Lcl Translation", "Lcl Translation", "", "A", [t.x as f64, t.y as f64, t.z as f64]),
        p_vec("Lcl Rotation", "Lcl Rotation", "", "A", e.map(|x| (x as f64).to_degrees())),
        p_vec("Lcl Scaling", "Lcl Scaling", "", "A", [sc.x as f64, sc.y as f64, sc.z as f64]),
    ])
}

fn model_node(id: i64, name: &str, kind: &str, local_f: &Mat4) -> Node {
    Node { name: "Model".into(), props: vec![Prop::I64(id), named(name, "Model"), s(kind)], children: vec![leaf("Version", Prop::I32(232)), model_props(local_f), leaf("Shading", Prop::Bool(true)), leaf("Culling", s("CullingOff"))] }
}

fn geometry_node(id: i64, name: &str, mesh: &Mesh, axes: &Mat4) -> Node {
    let mut verts = Vec::with_capacity(mesh.positions.len() * 3);
    for p in &mesh.positions {
        let q = axes.transform_point(*p);
        verts.extend([q.x as f64, q.y as f64, q.z as f64]);
    }
    let vnorm = mesh.vertex_normals();
    let mut pvi = Vec::new();
    let mut normals = Vec::new();
    let mut uv_data: Vec<f64> = Vec::new();
    let mut uv_index: Vec<i32> = Vec::new();
    let mut uv_map: HashMap<[u32; 2], i32> = HashMap::new();
    let has_uv = mesh.faces.iter().any(|f| !f.uvs.is_empty());
    let mut smoothing = Vec::with_capacity(mesh.faces.len());
    let mut mats = Vec::with_capacity(mesh.faces.len());
    for (fi, f) in mesh.faces.iter().enumerate() {
        let fnorm = mesh.face_normal(fi);
        for (k, &v) in f.verts.iter().enumerate() {
            pvi.push(if k + 1 == f.verts.len() { -(v as i32) - 1 } else { v as i32 });
            let n = if f.smooth { vnorm.get(v as usize).copied().unwrap_or(fnorm) } else { fnorm };
            let n = axes.transform_vector(n);
            normals.extend([n.x as f64, n.y as f64, n.z as f64]);
            if has_uv {
                let uv = f.uvs.get(k).copied().unwrap_or(Vec2::new(0.0, 0.0));
                let key = [uv.x.to_bits(), uv.y.to_bits()];
                let ix = *uv_map.entry(key).or_insert_with(|| {
                    uv_data.extend([uv.x as f64, uv.y as f64]);
                    (uv_data.len() / 2 - 1) as i32
                });
                uv_index.push(ix);
            }
        }
        smoothing.push(f.smooth as i32);
        mats.push(f.material as i32);
    }
    let mut lone = Vec::new();
    for e in &mesh.loose_edges {
        lone.push(e[0] as i32);
        lone.push(-(e[1] as i32) - 1);
    }
    pvi.extend(lone);
    let layer_el = |name: &str, version: i32, mapping: &str, reference: &str| Node::new(name, vec![Prop::I32(0)]).with(leaf("Version", Prop::I32(version))).with(leaf("Name", s(""))).with(leaf("MappingInformationType", s(mapping))).with(leaf("ReferenceInformationType", s(reference)));
    let mut g = Node::new("Geometry", vec![Prop::I64(id), named(name, "Geometry"), s("Mesh")]).with(props70(vec![])).with(leaf("GeometryVersion", Prop::I32(124))).with(leaf("Vertices", Prop::ArrF64(verts))).with(leaf("PolygonVertexIndex", Prop::ArrI32(pvi)));
    g = g.with(layer_el("LayerElementNormal", 101, "ByPolygonVertex", "Direct").with(leaf("Normals", Prop::ArrF64(normals))));
    let mut layer = Node::new("Layer", vec![Prop::I32(0)]).with(leaf("Version", Prop::I32(100)));
    let le_ref = |ty: &str| Node::new("LayerElement", vec![]).with(leaf("Type", s(ty))).with(leaf("TypedIndex", Prop::I32(0)));
    layer = layer.with(le_ref("LayerElementNormal"));
    g = g.with(layer_el("LayerElementSmoothing", 102, "ByPolygon", "Direct").with(leaf("Smoothing", Prop::ArrI32(smoothing))));
    layer = layer.with(le_ref("LayerElementSmoothing"));
    if has_uv {
        let mut uvl = layer_el("LayerElementUV", 101, "ByPolygonVertex", "IndexToDirect").with(leaf("UV", Prop::ArrF64(uv_data))).with(leaf("UVIndex", Prop::ArrI32(uv_index)));
        uvl.children[1] = leaf("Name", s("UVMap"));
        g = g.with(uvl);
        layer = layer.with(le_ref("LayerElementUV"));
    }
    g = g.with(layer_el("LayerElementMaterial", 101, "ByPolygon", "IndexToDirect").with(leaf("Materials", Prop::ArrI32(mats))));
    layer = layer.with(le_ref("LayerElementMaterial"));
    g.with(layer)
}

fn material_node(id: i64, m: &Material) -> Node {
    let c = [m.base_color[0] as f64, m.base_color[1] as f64, m.base_color[2] as f64];
    let e = m.emission * m.emission_strength;
    let shin = ((1.0 - m.roughness as f64).clamp(0.0, 1.0) * 10.0).powi(2);
    Node::new("Material", vec![Prop::I64(id), named(&m.name, "Material"), s("")]).with(leaf("Version", Prop::I32(102))).with(leaf("ShadingModel", s("Phong"))).with(leaf("MultiLayer", Prop::I32(0))).with(props70(vec![
        p_str("ShadingModel", "Phong"),
        p_color("DiffuseColor", c),
        p_anim_num("DiffuseFactor", 1.0),
        p_color("EmissiveColor", [e.x as f64, e.y as f64, e.z as f64]),
        p_anim_num("EmissiveFactor", 1.0),
        p_color("SpecularColor", [1.0; 3]),
        p_anim_num("SpecularFactor", 0.5),
        p_anim_num("ShininessExponent", shin),
        p_anim_num("ReflectionFactor", m.metallic as f64),
        p_color("TransparentColor", [1.0; 3]),
        p_anim_num("TransparencyFactor", 1.0 - m.base_color[3] as f64),
        p_anim_num("Opacity", m.base_color[3] as f64),
    ]))
}

fn camera_attr(id: i64, name: &str, c: &Camera, aspect: f32) -> Node {
    let mut ps = vec![
        p_vec("Position", "Vector", "", "A", [0.0; 3]),
        p_vec("UpVector", "Vector", "", "A", [0.0, 1.0, 0.0]),
        p_vec("InterestPosition", "Vector", "", "A", [0.0; 3]),
        p_num("FilmWidth", c.sensor_width as f64 / 25.4),
        p_num("FilmHeight", c.sensor_height as f64 / 25.4),
        p_enum("ApertureMode", 3),
        p_num("FocalLength", c.focal_length(aspect) as f64),
        p_num("NearPlane", c.near as f64),
        p_num("FarPlane", c.far as f64),
        p_num("AspectWidth", aspect as f64),
        p_num("AspectHeight", 1.0),
    ];
    if let Some(o) = c.ortho_scale {
        ps.push(p_enum("CameraProjectionType", 1));
        ps.push(p_num("OrthoZoom", o as f64 / 2.0));
    }
    Node::new("NodeAttribute", vec![Prop::I64(id), named(name, "NodeAttribute"), s("Camera")]).with(props70(ps)).with(leaf("TypeFlags", s("Camera"))).with(leaf("GeometryVersion", Prop::I32(124)))
}

fn light_attr(id: i64, name: &str, l: &Light) -> Node {
    let (ty, intensity) = match l.kind {
        LightKind::Point => (0, l.power / 10.0),
        LightKind::Sun => (1, l.power * 100.0),
        LightKind::Spot { .. } => (2, l.power / 10.0),
        LightKind::Area { .. } => (3, l.power / 10.0),
    };
    let mut ps = vec![p_enum("LightType", ty), p_color("Color", [l.color.x as f64, l.color.y as f64, l.color.z as f64]), p_anim_num("Intensity", intensity as f64), p_int("CastShadows", l.cast_shadow as i32)];
    if let LightKind::Spot { angle, blend } = l.kind {
        let outer = (angle as f64).to_degrees();
        ps.push(p_anim_num("OuterAngle", outer));
        ps.push(p_anim_num("InnerAngle", outer * (1.0 - blend as f64)));
    }
    Node::new("NodeAttribute", vec![Prop::I64(id), named(name, "NodeAttribute"), s("Light")]).with(props70(ps)).with(leaf("TypeFlags", s("Light"))).with(leaf("GeometryVersion", Prop::I32(124)))
}

pub fn export(scene: &Scene) -> Vec<u8> {
    let axes = export_axes();
    let axes_inv = axes.inverse();
    let conj = |l: &Mat4| axes * *l * axes_inv;
    let mut ids = Ids(1_000_000);
    let mut objects: Vec<Node> = Vec::new();
    let mut conns: Vec<Node> = Vec::new();
    let mut counts: Vec<(&str, usize)> = Vec::new();
    let count = |c: &mut Vec<(&'static str, usize)>, k: &'static str| match c.iter_mut().find(|x| x.0 == k) {
        Some(x) => x.1 += 1,
        None => c.push((k, 1)),
    };
    let aspect = 16.0 / 9.0;

    // Materials (by scene index) and their textures.
    let mut mat_id: HashMap<usize, i64> = HashMap::new();
    let used: Vec<usize> = {
        let mut u: Vec<usize> = scene.objects.iter().filter(|o| matches!(o.data, ObjectData::Mesh(_))).flat_map(|o| o.materials.iter().copied()).filter(|&m| m < scene.materials.len()).collect();
        u.sort_unstable();
        u.dedup();
        u
    };
    let mut tex_of_image: HashMap<usize, i64> = HashMap::new();
    for &mi in &used {
        let m = &scene.materials[mi];
        let id = ids.next();
        objects.push(material_node(id, m));
        count(&mut counts, "Material");
        mat_id.insert(mi, id);
        if let Some(img) = m.base_color_texture.filter(|&i| i < scene.images.len()) {
            let tid = match tex_of_image.get(&img) {
                Some(&t) => t,
                None => {
                    let image = &scene.images[img];
                    let file = if image.name.to_ascii_lowercase().ends_with(".png") { image.name.clone() } else { format!("{}.png", image.name) };
                    let (vid, tid) = (ids.next(), ids.next());
                    objects.push(Node::new("Video", vec![Prop::I64(vid), named(&image.name, "Video"), s("Clip")]).with(leaf("Type", s("Clip"))).with(props70(vec![Node::new("P", vec![s("Path"), s("KString"), s("XRefUrl"), s(""), s(&file)])])).with(leaf("UseMipMap", Prop::I32(0))).with(leaf("Filename", s(&file))).with(leaf("RelativeFilename", s(&file))).with(leaf("Content", Prop::Raw(encode_png(image)))));
                    objects.push(Node::new("Texture", vec![Prop::I64(tid), named(&image.name, "Texture"), s("")]).with(leaf("Type", s("TextureVideoClip"))).with(leaf("Version", Prop::I32(202))).with(leaf("TextureName", named(&image.name, "Texture"))).with(leaf("Media", named(&image.name, "Video"))).with(leaf("FileName", s(&file))).with(leaf("RelativeFilename", s(&file))).with(Node::new("ModelUVTranslation", vec![Prop::F64(0.0), Prop::F64(0.0)])).with(Node::new("ModelUVScaling", vec![Prop::F64(1.0), Prop::F64(1.0)])).with(leaf("Texture_Alpha_Source", s("None"))).with(Node::new("Cropping", vec![Prop::I32(0), Prop::I32(0), Prop::I32(0), Prop::I32(0)])));
                    count(&mut counts, "Video");
                    count(&mut counts, "Texture");
                    conns.push(conn(vid, tid));
                    tex_of_image.insert(img, tid);
                    tid
                }
            };
            conns.push(conn_prop(tid, id, "DiffuseColor"));
        }
    }

    // Models.
    let mut obj_id: HashMap<usize, i64> = HashMap::new();
    let mut fix_of: HashMap<usize, Mat4> = HashMap::new();
    // armature object -> (bone model ids, bone file-world rest matrices)
    let mut bones_of: HashMap<usize, Vec<(i64, Mat4)>> = HashMap::new();
    let mut pose_nodes: Vec<Node> = Vec::new();
    for (oi, o) in scene.objects.iter().enumerate() {
        obj_id.insert(oi, ids.next());
        // The same turns import applies, with file-to-scene axes `axes⁻¹`.
        match o.data {
            ObjectData::Camera(_) => {
                fix_of.insert(oi, axes_inv * cam_fix());
            }
            ObjectData::Light(_) => {
                fix_of.insert(oi, axes_inv * light_fix());
            }
            ObjectData::Armature(ai) => {
                let w = scene.world_matrix(oi);
                ensure_bones(&mut bones_of, oi, &scene.armatures[ai], &w, &mut ids, &conj);
            }
            _ => {}
        }
    }
    for (oi, o) in scene.objects.iter().enumerate() {
        let id = obj_id[&oi];
        // Scene local -> file local: undo the parent's turn, add the
        // bone's head-to-tail offset, remove this node's own turn.
        let mut local = o.transform.matrix();
        if o.parent_type != ParentType::Bone {
            if let Some(p) = o.parent.and_then(|p| fix_of.get(&p)) {
                local = *p * local;
            }
        }
        if o.parent_type == ParentType::Bone && !o.parent_bone.is_empty() {
            if let Some(ObjectData::Armature(ai)) = o.parent.map(|p| &scene.objects[p].data) {
                if let Some(b) = scene.armatures.get(*ai).and_then(|a| a.find(&o.parent_bone).map(|bi| &a.bones[bi])) {
                    local = Mat4::translation(Vec3::Y * b.length()) * local;
                }
            }
        }
        if let Some(f) = fix_of.get(&oi) {
            local = local * f.inverse();
        }
        let local_f = conj(&local);
        let kind = match o.data {
            ObjectData::Mesh(_) => "Mesh",
            ObjectData::Camera(_) => "Camera",
            ObjectData::Light(_) => "Light",
            _ => "Null",
        };
        objects.push(model_node(id, &o.name, kind, &local_f));
        count(&mut counts, "Model");
        let parent_id = match (o.parent_type, o.parent) {
            (ParentType::Bone, Some(p)) => match (&scene.objects[p].data, bones_of.get(&p)) {
                (ObjectData::Armature(ai), Some(b)) => scene.armatures[*ai].find(&o.parent_bone).map_or(obj_id[&p], |bi| b[bi].0),
                _ => obj_id.get(&p).copied().unwrap_or(0),
            },
            (_, Some(p)) => obj_id.get(&p).copied().unwrap_or(0),
            _ => 0,
        };
        conns.push(conn(id, parent_id));
        match &o.data {
            ObjectData::Mesh(_) => {
                let stack: Vec<Modifier> = o.modifiers.iter().filter(|m| !matches!(m, Modifier::Armature { .. })).cloned().collect();
                let ObjectData::Mesh(mi) = o.data else { unreachable!() };
                let mesh = scene.apply_modifiers(oi, &scene.meshes[mi], &stack);
                let gid = ids.next();
                let gname = match o.data {
                    ObjectData::Mesh(mi) => scene.mesh_names.get(mi).cloned().unwrap_or_else(|| o.name.clone()),
                    _ => o.name.clone(),
                };
                objects.push(geometry_node(gid, &gname, &mesh, &axes));
                count(&mut counts, "Geometry");
                conns.push(conn(gid, id));
                for m in &o.materials {
                    if let Some(&mid) = mat_id.get(m) {
                        conns.push(conn(mid, id));
                    }
                }
                // Skin.
                if let Some(ao) = o.modifiers.iter().find_map(|m| match m {
                    Modifier::Armature { object, .. } => scene.find_object(object),
                    _ => None,
                }) {
                    if let ObjectData::Armature(ai) = scene.objects[ao].data {
                        let arm = &scene.armatures[ai];
                        let arm_world = scene.world_matrix(ao);
                        let mesh_world = scene.world_matrix(oi);
                        let bone_ids = ensure_bones(&mut bones_of, ao, arm, &arm_world, &mut ids, &conj).clone();
                        let skin = ids.next();
                        objects.push(Node::new("Deformer", vec![Prop::I64(skin), named(&o.name, "Deformer"), s("Skin")]).with(leaf("Version", Prop::I32(101))).with(leaf("Link_DeformAcuracy", Prop::F64(50.0))));
                        count(&mut counts, "Deformer");
                        conns.push(conn(skin, gid));
                        pose_nodes.push(Node::new("PoseNode", vec![]).with(leaf("Node", Prop::I64(id))).with(leaf("Matrix", Prop::ArrF64(mat_to(&conj(&mesh_world))))));
                        for (bi, b) in arm.bones.iter().enumerate() {
                            let Some(vg) = mesh.vertex_group_index(&b.name) else { continue };
                            let (mut ix, mut ws) = (Vec::new(), Vec::new());
                            for v in 0..mesh.positions.len() {
                                if let Some(w) = mesh.weight_opt(v, vg).filter(|&w| w > 0.0) {
                                    ix.push(v as i32);
                                    ws.push(w as f64);
                                }
                            }
                            let cl = ids.next();
                            let (bid, bworld) = bone_ids[bi];
                            objects.push(
                                Node::new("Deformer", vec![Prop::I64(cl), named(&b.name, "SubDeformer"), s("Cluster")])
                                    .with(leaf("Version", Prop::I32(100)))
                                    .with(leaf("UserData", s("")))
                                    .with(leaf("Indexes", Prop::ArrI32(ix)))
                                    .with(leaf("Weights", Prop::ArrF64(ws)))
                                    .with(leaf("Transform", Prop::ArrF64(mat_to(&(bworld.inverse() * conj(&mesh_world))))))
                                    .with(leaf("TransformLink", Prop::ArrF64(mat_to(&bworld)))),
                            );
                            count(&mut counts, "Deformer");
                            conns.push(conn(cl, skin));
                            conns.push(conn(bid, cl));
                        }
                    }
                }
            }
            ObjectData::Camera(c) => {
                let aid = ids.next();
                objects.push(camera_attr(aid, &o.name, c, aspect));
                count(&mut counts, "NodeAttribute");
                conns.push(conn(aid, id));
            }
            ObjectData::Light(l) => {
                let aid = ids.next();
                objects.push(light_attr(aid, &o.name, l));
                count(&mut counts, "NodeAttribute");
                conns.push(conn(aid, id));
            }
            _ => {}
        }
    }
    // Bone models (rest pose), parented bone to bone, roots to the armature.
    let mut arm_list: Vec<_> = bones_of.iter().collect();
    arm_list.sort_by_key(|(k, _)| **k);
    for (&ao, bones) in arm_list {
        let ObjectData::Armature(ai) = scene.objects[ao].data else { continue };
        let arm = &scene.armatures[ai];
        let arm_world_f = conj(&scene.world_matrix(ao));
        for (bi, b) in arm.bones.iter().enumerate() {
            let (bid, world) = bones[bi];
            let parent_world = b.parent.map_or(arm_world_f, |p| bones[p].1);
            let local = parent_world.inverse() * world;
            objects.push(model_node(bid, &b.name, "LimbNode", &local));
            count(&mut counts, "Model");
            let aid = ids.next();
            objects.push(Node::new("NodeAttribute", vec![Prop::I64(aid), named(&b.name, "NodeAttribute"), s("LimbNode")]).with(props70(vec![p_num("Size", (b.length() * 100.0) as f64)])).with(leaf("TypeFlags", s("Skeleton"))));
            count(&mut counts, "NodeAttribute");
            conns.push(conn(aid, bid));
            conns.push(conn(bid, b.parent.map_or(obj_id[&ao], |p| bones[p].0)));
            pose_nodes.push(Node::new("PoseNode", vec![]).with(leaf("Node", Prop::I64(bid))).with(leaf("Matrix", Prop::ArrF64(mat_to(&world)))));
        }
    }
    if !pose_nodes.is_empty() {
        let pid = ids.next();
        let n = pose_nodes.len();
        let mut pose = Node::new("Pose", vec![Prop::I64(pid), named("BindPose", "Pose"), s("BindPose")]).with(leaf("Type", s("BindPose"))).with(leaf("Version", Prop::I32(100))).with(leaf("NbPoseNodes", Prop::I32(n as i32)));
        pose.children.extend(pose_nodes);
        objects.push(pose);
        count(&mut counts, "Pose");
    }

    // Header, settings, definitions.
    let header = Node::new("FBXHeaderExtension", vec![])
        .with(leaf("FBXHeaderVersion", Prop::I32(1003)))
        .with(leaf("FBXVersion", Prop::I32(7400)))
        .with(leaf("EncryptionType", Prop::I32(0)))
        .with(Node::new("CreationTimeStamp", vec![]).with(leaf("Version", Prop::I32(1000))).with(leaf("Year", Prop::I32(1970))).with(leaf("Month", Prop::I32(1))).with(leaf("Day", Prop::I32(1))).with(leaf("Hour", Prop::I32(10))).with(leaf("Minute", Prop::I32(0))).with(leaf("Second", Prop::I32(0))).with(leaf("Millisecond", Prop::I32(0))))
        .with(leaf("Creator", s("Polyamour (SexOS) FBX 7.4 writer")));
    let fps = if scene.fps > 0.0 { scene.fps as f64 } else { 24.0 };
    let ticks = |f: i32| Prop::I64((f as f64 / fps * 46_186_158_000.0) as i64);
    let gs = Node::new("GlobalSettings", vec![]).with(leaf("Version", Prop::I32(1000))).with(props70(vec![
        p_int("UpAxis", 1),
        p_int("UpAxisSign", 1),
        p_int("FrontAxis", 2),
        p_int("FrontAxisSign", 1),
        p_int("CoordAxis", 0),
        p_int("CoordAxisSign", 1),
        p_int("OriginalUpAxis", 2),
        p_int("OriginalUpAxisSign", 1),
        p_num("UnitScaleFactor", 100.0),
        p_num("OriginalUnitScaleFactor", 100.0),
        p_enum("TimeMode", 14),
        Node::new("P", vec![s("TimeSpanStart"), s("KTime"), s("Time"), s(""), ticks(scene.frame_start)]),
        Node::new("P", vec![s("TimeSpanStop"), s("KTime"), s("Time"), s(""), ticks(scene.frame_end)]),
        Node::new("P", vec![s("CustomFrameRate"), s("double"), s("Number"), s(""), Prop::F64(fps)]),
    ]));
    let doc_id = ids.next();
    let documents = Node::new("Documents", vec![]).with(leaf("Count", Prop::I32(1))).with(Node::new("Document", vec![Prop::I64(doc_id), s("Scene"), s("Scene")]).with(props70(vec![Node::new("P", vec![s("SourceObject"), s("object"), s(""), s("")]), p_str("ActiveAnimStackName", "")])).with(leaf("RootNode", Prop::I64(0))));
    let total: usize = counts.iter().map(|c| c.1).sum::<usize>() + 1;
    let mut defs = Node::new("Definitions", vec![]).with(leaf("Version", Prop::I32(100))).with(leaf("Count", Prop::I32(total as i32))).with(Node::new("ObjectType", vec![s("GlobalSettings")]).with(leaf("Count", Prop::I32(1))));
    for (k, n) in &counts {
        defs = defs.with(Node::new("ObjectType", vec![s(k)]).with(leaf("Count", Prop::I32(*n as i32))));
    }
    let objects_node = Node { name: "Objects".into(), props: vec![], children: objects };
    let conns_node = Node { name: "Connections".into(), props: vec![], children: conns };
    let nodes = vec![
        header,
        leaf("FileId", tree::file_id()),
        leaf("CreationTime", s("1970-01-01 10:00:00:000")),
        leaf("Creator", s("Polyamour (SexOS) FBX 7.4 writer")),
        gs,
        documents,
        Node::new("References", vec![]),
        defs,
        objects_node,
        conns_node,
        Node::new("Takes", vec![]).with(leaf("Current", s(""))),
    ];
    tree::write_binary(&nodes)
}

fn ensure_bones<'a>(bones_of: &'a mut HashMap<usize, Vec<(i64, Mat4)>>, ao: usize, arm: &Armature, arm_world: &Mat4, ids: &mut Ids, conj: &dyn Fn(&Mat4) -> Mat4) -> &'a Vec<(i64, Mat4)> {
    bones_of.entry(ao).or_insert_with(|| arm.bones.iter().map(|b| (ids.next(), conj(&(*arm_world * b.rest_matrix())))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{self, no_files};

    fn approx_m(a: &Mat4, b: &Mat4, eps: f32) -> bool {
        a.to_cols_array().iter().zip(b.to_cols_array()).all(|(x, y)| (x - y).abs() < eps)
    }

    fn sample_scene() -> Scene {
        let mut sc = Scene::default();
        sc.objects.clear();
        let red = sc.add_material(Material { name: "Red".into(), base_color: [0.8, 0.1, 0.1, 1.0], metallic: 0.25, roughness: 0.4, ..Default::default() });
        let mut img = crate::scene::Image::new("checker", 2, 2);
        img.pixels = vec![[1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
        sc.images.push(img);
        let tex = sc.add_material(Material { name: "Checker".into(), base_color_texture: Some(sc.images.len() - 1), ..Default::default() });
        let mut cube = crate::primitives::cube(2.0);
        for (i, f) in cube.faces.iter_mut().enumerate() {
            f.material = (i % 2) as u16;
            f.smooth = i < 3;
        }
        let c = sc.add_mesh_object("Cube", cube);
        sc.objects[c].materials = vec![red, tex];
        sc.objects[c].transform = Transform { location: Vec3::new(1.0, 2.0, 3.0), rotation: Quat::from_euler_xyz(0.3, -0.2, 0.9), scale: Vec3::new(1.0, 2.0, 0.5) };
        let mut e = Object::new("Empty", ObjectData::Empty);
        e.parent = Some(c);
        e.transform.location = Vec3::new(0.0, 0.0, 1.5);
        e.transform.rotation = Quat::from_euler_xyz(0.0, 0.0, 0.7);
        sc.add_object(e);
        let mut cam = Object::new("Camera", ObjectData::Camera(Camera::default()));
        cam.transform.location = Vec3::new(7.0, -6.0, 5.0);
        cam.transform.rotation = Quat::from_euler_xyz(1.1, 0.0, 0.8);
        let cam_i = sc.add_object(cam);
        let mut tag = Object::new("CamTag", ObjectData::Empty);
        tag.parent = Some(cam_i);
        tag.transform.location = Vec3::new(0.0, 0.5, -1.0);
        sc.add_object(tag);
        let mut l = Object::new("Spot", ObjectData::Light(Light { kind: LightKind::Spot { angle: 0.8, blend: 0.25 }, power: 500.0, color: Vec3::new(1.0, 0.9, 0.7), ..Default::default() }));
        l.transform.location = Vec3::new(-3.0, 1.0, 4.0);
        l.transform.rotation = Quat::from_euler_xyz(0.4, 0.2, -0.3);
        sc.add_object(l);
        sc
    }

    #[test]
    fn round_trip_scene() {
        let sc = sample_scene();
        let bytes = export(&sc);
        assert_eq!(Format::sniff(&bytes), Some(Format::Fbx));
        let back = io::import(&bytes, Format::Fbx, &no_files).unwrap();
        assert_eq!(back.objects.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), ["Cube", "Empty", "Camera", "CamTag", "Spot"]);
        for (i, o) in sc.objects.iter().enumerate() {
            let j = back.find_object(&o.name).unwrap();
            assert!(approx_m(&sc.world_matrix(i), &back.world_matrix(j), 1e-4), "{} moved:\n{:?}\n{:?}", o.name, sc.world_matrix(i), back.world_matrix(j));
        }
        let cube = back.find_object("Cube").unwrap();
        let ObjectData::Mesh(m) = back.objects[cube].data else { panic!() };
        let (a, b) = (&sc.meshes[0], &back.meshes[m]);
        assert_eq!((a.positions.len(), a.faces.len()), (b.positions.len(), b.faces.len()));
        for (fa, fb) in a.faces.iter().zip(&b.faces) {
            assert_eq!(fa.verts, fb.verts);
            assert_eq!(fa.material, fb.material);
            assert_eq!(fa.smooth, fb.smooth);
            for (ua, ub) in fa.uvs.iter().zip(&fb.uvs) {
                assert!((ua.x - ub.x).abs() < 1e-6 && (ua.y - ub.y).abs() < 1e-6);
            }
        }
        for (pa, pb) in a.positions.iter().zip(&b.positions) {
            assert!(pa.approx_eq(*pb, 1e-5));
        }
        let mats: Vec<&Material> = back.objects[cube].materials.iter().map(|&i| &back.materials[i]).collect();
        assert_eq!(mats[0].name, "Red");
        assert!((mats[0].base_color[0] - 0.8).abs() < 1e-5 && (mats[0].metallic - 0.25).abs() < 1e-5 && (mats[0].roughness - 0.4).abs() < 1e-3);
        let img = mats[1].base_color_texture.expect("embedded texture");
        assert_eq!((back.images[img].width, back.images[img].height), (2, 2));
        let ObjectData::Camera(c) = &back.objects[back.find_object("Camera").unwrap()].data else { panic!() };
        assert!((c.fov_y - Camera::default().fov_y).abs() < 1e-4);
        let ObjectData::Light(l) = &back.objects[back.find_object("Spot").unwrap()].data else { panic!() };
        assert!(matches!(l.kind, LightKind::Spot { angle, blend } if (angle - 0.8).abs() < 1e-4 && (blend - 0.25).abs() < 1e-4));
        assert!((l.power - 500.0).abs() < 1e-2);
    }

    fn skin_scene() -> (Scene, usize) {
        let mut sc = Scene::default();
        sc.objects.clear();
        let mut arm = Armature::default();
        let root = arm.add_bone(Bone::new("Root", Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0)));
        let mut tip = Bone::new("Tip", Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 2.0));
        tip.parent = Some(root);
        arm.add_bone(tip);
        let ai = sc.add_armature("Rig", arm);
        let mut ao = Object::new("Rig", ObjectData::Armature(ai));
        ao.transform.location = Vec3::new(2.0, 0.0, 0.0);
        let ao = sc.add_object(ao);
        let mut mesh = crate::primitives::cylinder(8, 0.3, 2.0);
        mesh.transform(&Mat4::translation(Vec3::new(0.0, 0.0, 1.0)));
        let (g0, g1) = (mesh.add_vertex_group("Root"), mesh.add_vertex_group("Tip"));
        for v in 0..mesh.positions.len() {
            let w = (mesh.positions[v].z / 2.0).clamp(0.0, 1.0);
            mesh.set_weight(v, g0, 1.0 - w);
            mesh.set_weight(v, g1, w);
        }
        let mo = sc.add_mesh_object("Body", mesh);
        let mut md = Modifier::with_defaults("Armature").unwrap();
        if let Modifier::Armature { object, .. } = &mut md {
            *object = "Rig".into();
        }
        sc.objects[mo].modifiers.push(md);
        sc.objects[mo].parent = Some(ao);
        sc.objects[mo].parent_type = ParentType::Armature;
        sc.objects[mo].transform.location = Vec3::new(-2.0, 0.0, 0.0);

        (sc, mo)
    }

    #[test]
    fn round_trip_skin() {
        let (sc, mo) = skin_scene();
        let back = io::import(&export(&sc), Format::Fbx, &no_files).unwrap();
        let rig = back.find_object("Rig").unwrap();
        let ObjectData::Armature(bai) = back.objects[rig].data else { panic!("Rig is not an armature") };
        let barm = &back.armatures[bai];
        assert_eq!(barm.bones.len(), 2);
        let tipb = &barm.bones[barm.find("Tip").unwrap()];
        assert_eq!(tipb.parent, barm.find("Root"));
        let world = back.world_matrix(rig);
        assert!(world.transform_point(tipb.head).approx_eq(Vec3::new(2.0, 0.0, 1.0), 1e-4), "{:?}", world.transform_point(tipb.head));
        let body = back.find_object("Body").unwrap();
        assert_eq!(back.objects[body].parent, Some(rig));
        assert!(back.objects[body].modifiers.iter().any(|m| matches!(m, Modifier::Armature { object, .. } if object == "Rig")));
        assert!(approx_m(&sc.world_matrix(mo), &back.world_matrix(body), 1e-4));
        let ObjectData::Mesh(bm) = back.objects[body].data else { panic!() };
        let (a, b) = (&sc.meshes[0], &back.meshes[bm]);
        for v in 0..a.positions.len() {
            for name in ["Root", "Tip"] {
                let wa = a.weight(v, a.vertex_group_index(name).unwrap());
                let wb = b.vertex_group_index(name).map_or(0.0, |g| b.weight(v, g));
                assert!((wa - wb).abs() < 1e-5, "vertex {v} {name}: {wa} vs {wb}");
            }
        }
    }

    #[test]
    fn ascii_z_up_centimetres_with_pivots() {
        // Z-up, centimetres, a PreRotation and a geometric offset.
        let text = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXVersion: 7400
}
GlobalSettings:  {
	Properties70:  {
		P: "UpAxis", "int", "Integer", "",2
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",1
		P: "FrontAxisSign", "int", "Integer", "",-1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",1
	}
}
Objects:  {
	Geometry: 10, "Geometry::Tri", "Mesh" {
		Vertices: *9 {
			a: 0,0,0,100,0,0,0,100,0
		}
		PolygonVertexIndex: *3 {
			a: 0,1,-3
		}
		LayerElementUV: 0 {
			MappingInformationType: "ByPolygonVertex"
			ReferenceInformationType: "IndexToDirect"
			UV: *4 {
				a: 0,0,1,1
			}
			UVIndex: *3 {
				a: 0,1,0
			}
		}
	}
	Model: 20, "Model::Tri", "Mesh" {
		Properties70:  {
			P: "PreRotation", "Vector3D", "Vector", "",0,0,90
			P: "Lcl Translation", "Lcl Translation", "", "A",100,0,0
			P: "GeometricTranslation", "Vector3D", "Vector", "",0,0,50
		}
	}
}
Connections:  {
	C: "OO",20,0
	C: "OO",10,20
}
"#;
        let sc = io::import(text.as_bytes(), Format::Fbx, &no_files).unwrap();
        assert_eq!(sc.objects.len(), 1);
        let (m, _) = sc.world_mesh(0).unwrap();
        // Metres; the +90° Z pre-rotation turns file +X into scene +Y.
        assert!(m.positions[0].approx_eq(Vec3::new(1.0, 0.0, 0.5), 1e-5), "{:?}", m.positions[0]);
        assert!(m.positions[1].approx_eq(Vec3::new(1.0, 1.0, 0.5), 1e-5), "{:?}", m.positions[1]);
        assert!(m.positions[2].approx_eq(Vec3::new(0.0, 0.0, 0.5), 1e-5), "{:?}", m.positions[2]);
        assert_eq!(m.faces[0].uvs[1], Vec2::new(1.0, 1.0));
    }

    #[test]
    fn child_listed_before_its_camera_and_cm_clip_planes() {
        // The child Model comes before its camera parent in Objects.
        let text = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXVersion: 7400
}
Objects:  {
	Model: 2, "Model::Tag", "Null" {
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",100,0,0
		}
	}
	NodeAttribute: 3, "NodeAttribute::Cam", "Camera" {
		Properties70:  {
			P: "NearPlane", "double", "Number", "",10
			P: "FarPlane", "double", "Number", "",5000
		}
	}
	Model: 1, "Model::Cam", "Camera" {
	}
}
Connections:  {
	C: "OO",1,0
	C: "OO",3,1
	C: "OO",2,1
}
"#;
        let sc = io::import(text.as_bytes(), Format::Fbx, &no_files).unwrap();
        let tag = sc.find_object("Tag").unwrap();
        let cam = sc.find_object("Cam").unwrap();
        // File +X (camera look axis, Y-up file) at 1 m is scene +X at 1 m.
        assert!(sc.world_matrix(tag).translation_part().approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-5), "{:?}", sc.world_matrix(tag).translation_part());
        let ObjectData::Camera(c) = &sc.objects[cam].data else { panic!() };
        assert!((c.near - 0.1).abs() < 1e-6 && (c.far - 50.0).abs() < 1e-4);
    }

    /// Set FBX_OUT to a folder to write sample exports there (for checking with other tools).
    #[test]
    #[ignore]
    fn write_samples() {
        let Ok(dir) = std::env::var("FBX_OUT") else { return };
        std::fs::write(format!("{dir}/scene.fbx"), export(&sample_scene())).unwrap();
        let mut sc = sample_scene();
        sc.objects.truncate(1);
        sc.add_mesh_object("Monkey", crate::primitives::monkey());
        std::fs::write(format!("{dir}/mesh.fbx"), export(&sc)).unwrap();
        std::fs::write(format!("{dir}/skin.fbx"), export(&skin_scene().0)).unwrap();
    }

    #[test]
    fn rejects_fbx6() {
        let text = "; FBX 6.1.0 project file\nFBXHeaderExtension:  {\n\tFBXVersion: 6100\n}\nObjects:  {\n}\n";
        assert!(matches!(io::import(text.as_bytes(), Format::Fbx, &no_files), Err(IoError::Unsupported { .. })));
    }

    /// Set FBX_SAMPLES to a folder of .fbx files to print what import makes of them.
    #[test]
    #[ignore]
    fn probe_samples() {
        let Ok(dir) = std::env::var("FBX_SAMPLES") else { return };
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let bytes = std::fs::read(&p).unwrap();
            let doc = tree::parse(&bytes);
            match io::import(&bytes, Format::Fbx, &no_files) {
                Ok(sc) => {
                    eprintln!("{}: v{} {} objects, {} meshes, {} materials, {} images, {} armatures", p.display(), doc.as_ref().map_or(0, |d| d.version), sc.objects.len(), sc.meshes.len(), sc.materials.len(), sc.images.len(), sc.armatures.len());
                    for (i, o) in sc.objects.iter().enumerate().take(12) {
                        let w = sc.world_matrix(i).translation_part();
                        let extra = match o.data {
                            ObjectData::Mesh(m) => format!("mesh v={} f={} groups={}", sc.meshes[m].positions.len(), sc.meshes[m].faces.len(), sc.meshes[m].vertex_groups.len()),
                            ObjectData::Armature(a) => format!("armature bones={}", sc.armatures[a].bones.len()),
                            ObjectData::Camera(_) | ObjectData::Light(_) => {
                                let d = sc.world_matrix(i).transform_vector(Vec3::new(0.0, 0.0, -1.0)).normalize();
                                let to0 = (Vec3::ZERO - w).normalize();
                                format!("look·to_origin={:.3}", d.dot(to0))
                            }
                            _ => "empty".into(),
                        };
                        eprintln!("   {} parent={:?} world=({:.3},{:.3},{:.3}) scale={:?} {}", o.name, o.parent, w.x, w.y, w.z, o.transform.scale, extra);
                    }
                    if let Ok(d) = &doc {
                        // Cluster Transform convention check.
                        let objs = d.get("Objects").unwrap();
                        for c in objs.children_named("Deformer").filter(|n| n.str(2) == Some("Cluster")).take(2) {
                            let t = c.child_f64s("Transform").and_then(|v| mat_from(&v));
                            let tl = c.child_f64s("TransformLink").and_then(|v| mat_from(&v));
                            eprintln!("   cluster T={:?}\n           TL={:?}", t.map(|m| m.translation_part()), tl.map(|m| m.translation_part()));
                        }
                    }
                }
                Err(e) => eprintln!("{}: ERROR {e}", p.display()),
            }
        }
    }
}
