// SPDX-License-Identifier: GPL-3.0-only
//! glTF 2.0: `.gltf` (JSON with data-URI or external buffers) and `.glb`.
//!
//! Import:
//! - Nodes become objects with parents; TRS or matrix transforms.
//! - Meshes: every primitive becomes a material slot on one mesh. Triangles,
//!   strips and fans become faces; lines, line loops and strips become loose
//!   edges. Accessors may use any component type (normalized integers
//!   included), `byteStride` and sparse storage. glTF splits vertices along
//!   UV and normal seams, so positions are welded afterwards and UVs kept per
//!   face corner; faces whose stored normals differ from their flat normal
//!   are marked smooth.
//! - Materials: metallic-roughness factors, base colour texture (PNG/JPEG,
//!   sRGB decoded to linear), emission with `KHR_materials_emissive_strength`,
//!   `KHR_materials_ior`, `KHR_materials_transmission`, `doubleSided`.
//! - Cameras and `KHR_lights_punctual` lights. Light units are converted like
//!   Blender's importer: point and spot `W = cd * 4π / 683`, sun `W/m² = lux`.
//!
//! Axes: glTF is Y-up. Like Blender's importer, positions, node transforms
//! and mesh data are rotated +90° about X so no visible rotation is left on
//! the objects. glTF cameras and lights look down their node's -Z with +Y up,
//! scene ones down -Z with +Y up in the *converted* frame, so their objects
//! get an extra +90° X turn (and their children the inverse).
//!
//! Export writes the inverse: one node per object, one mesh per (mesh, slot
//! materials) pair with one primitive per used slot (vertices split by
//! normal and UV seams), materials, PNG textures, cameras and lights. `.gltf`
//! embeds the binary buffer as a data URI.
use super::{decode_image, encode_png, weld_exact, y_up_to_z_up, z_up_to_y_up, Format, IoError, Resolver};
use crate::math::{Mat4, Quat, Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use crate::scene::{Camera, Light, LightKind, Material, Object, ObjectData, Scene, Transform};
use base64::Engine as _;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

/// Lumens per watt used by Blender's glTF light conversion.
const WATTS_TO_LUMENS: f32 = 683.0;
const GLB_MAGIC: &[u8; 4] = b"glTF";
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

/// Extensions this reader understands (or may safely ignore when required).
const SUPPORTED: [&str; 6] = [
    "KHR_lights_punctual",
    "KHR_materials_emissive_strength",
    "KHR_materials_ior",
    "KHR_materials_transmission",
    "KHR_texture_transform",
    "KHR_materials_unlit",
];

/// The +90° X rotation from Y-up to Z-up, as a quaternion.
fn q_c() -> Quat {
    Quat::from_axis_angle(Vec3::X, FRAC_PI_2)
}

/// `C q C⁻¹`: a rotation expressed in Y-up axes, re-expressed in Z-up axes.
fn conj_c(q: Quat) -> Quat {
    Quat { x: q.x, y: -q.z, z: q.y, w: q.w }
}

/// `C⁻¹ q C`, inverse of [`conj_c`].
fn conj_c_inv(q: Quat) -> Quat {
    Quat { x: q.x, y: q.z, z: -q.y, w: q.w }
}

/// A glTF node's local TRS to a scene object's transform. `parent_turned` and
/// `self_turned` say whether the parent's / this object's frame carries the
/// camera/light correction (see the module docs).
fn to_scene_transform(t: Vec3, q: Quat, s: Vec3, parent_turned: bool, self_turned: bool) -> Transform {
    let location = if parent_turned { t } else { y_up_to_z_up(t) };
    let mut rotation = if parent_turned { q * q_c().conjugate() } else { conj_c(q) };
    if self_turned {
        rotation = rotation * q_c();
    }
    let scale = if self_turned { s } else { Vec3::new(s.x, s.z, s.y) };
    Transform { location, rotation: rotation.normalize(), scale }
}

/// Inverse of [`to_scene_transform`].
fn to_gltf_transform(tr: &Transform, parent_turned: bool, self_turned: bool) -> (Vec3, Quat, Vec3) {
    let s = tr.scale;
    let t = if parent_turned { tr.location } else { z_up_to_y_up(tr.location) };
    let r = if self_turned { tr.rotation * q_c().conjugate() } else { tr.rotation };
    let q = if parent_turned { r * q_c() } else { conj_c_inv(r) };
    let scale = if self_turned { s } else { Vec3::new(s.x, s.z, s.y) };
    (t, q.normalize(), scale)
}

// ---------------------------------------------------------------------------
// JSON helpers.

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}
fn idx(v: &Value, key: &str) -> Option<usize> {
    v.get(key)?.as_u64().map(|x| x as usize)
}
fn num(v: &Value, key: &str) -> Option<f32> {
    v.get(key)?.as_f64().map(|x| x as f32)
}
fn floats<const N: usize>(v: Option<&Value>) -> Option<[f32; N]> {
    let a = v?.as_array()?;
    if a.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64()? as f32;
    }
    Some(out)
}
fn ext<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    v.get("extensions")?.get(name)
}

/// Decode `%XX` escapes of a relative URI.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Import.

struct View {
    buffer: usize,
    offset: usize,
    length: usize,
    stride: Option<usize>,
}

struct Reader<'a> {
    fmt: Format,
    doc: &'a Value,
    buffers: Vec<Vec<u8>>,
    views: Vec<View>,
}

/// Size in bytes of an accessor component type.
fn component_size(ct: u64) -> Option<usize> {
    match ct {
        5120 | 5121 => Some(1),
        5122 | 5123 => Some(2),
        5125 | 5126 => Some(4),
        _ => None,
    }
}

/// One component as f64, applying glTF's normalized-integer rules.
fn component(ct: u64, b: &[u8], normalized: bool) -> f64 {
    match ct {
        5120 => {
            let v = b[0] as i8 as f64;
            if normalized {
                (v / 127.0).max(-1.0)
            } else {
                v
            }
        }
        5121 => b[0] as f64 / if normalized { 255.0 } else { 1.0 },
        5122 => {
            let v = i16::from_le_bytes([b[0], b[1]]) as f64;
            if normalized {
                (v / 32767.0).max(-1.0)
            } else {
                v
            }
        }
        5123 => u16::from_le_bytes([b[0], b[1]]) as f64 / if normalized { 65535.0 } else { 1.0 },
        5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
        _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
    }
}

fn type_components(t: &str) -> Option<usize> {
    Some(match t {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" | "MAT2" => 4,
        "MAT3" => 9,
        "MAT4" => 16,
        _ => return None,
    })
}

impl<'a> Reader<'a> {
    fn err(&self, msg: impl Into<String>) -> IoError {
        IoError::parse(self.fmt, None, msg)
    }

    fn view(&self, i: usize) -> Result<(&[u8], Option<usize>), IoError> {
        let v = self.views.get(i).ok_or_else(|| self.err(format!("bufferView {i} does not exist")))?;
        Ok((&self.buffers[v.buffer][v.offset..v.offset + v.length], v.stride))
    }

    /// Read accessor `i` as `count * components` values converted by `conv`.
    /// Returns (components per element, values).
    fn read<T: Copy + Default>(&self, i: usize, conv: fn(f64) -> T) -> Result<(usize, Vec<T>), IoError> {
        let acc = self.doc.get("accessors").and_then(|a| a.get(i)).ok_or_else(|| self.err(format!("accessor {i} does not exist")))?;
        let ct = acc.get("componentType").and_then(Value::as_u64).unwrap_or(0);
        let csize = component_size(ct).ok_or_else(|| self.err(format!("accessor {i}: invalid componentType {ct}")))?;
        let ty = acc.get("type").and_then(Value::as_str).unwrap_or("");
        let n = type_components(ty).ok_or_else(|| self.err(format!("accessor {i}: invalid type \"{ty}\"")))?;
        let count = idx(acc, "count").ok_or_else(|| self.err(format!("accessor {i}: missing count")))?;
        let normalized = acc.get("normalized").and_then(Value::as_bool).unwrap_or(false);
        let elem = n * csize;
        let mut out = vec![T::default(); count.checked_mul(n).ok_or_else(|| self.err(format!("accessor {i}: count too large")))?];
        if let Some(vi) = idx(acc, "bufferView") {
            let (bytes, stride) = self.view(vi)?;
            let stride = stride.unwrap_or(elem);
            let off = idx(acc, "byteOffset").unwrap_or(0);
            let need = if count == 0 { 0 } else { off + stride * (count - 1) + elem };
            if stride < elem || need > bytes.len() {
                return Err(self.err(format!("accessor {i}: reads {need} bytes past the start of bufferView {vi}, which has {}", bytes.len())));
            }
            for (e, o) in out.chunks_exact_mut(n).enumerate() {
                let base = off + e * stride;
                for (c, slot) in o.iter_mut().enumerate() {
                    *slot = conv(component(ct, &bytes[base + c * csize..], normalized));
                }
            }
        }
        if let Some(sp) = acc.get("sparse") {
            let scount = idx(sp, "count").unwrap_or(0);
            let (ind, val) = (sp.get("indices").unwrap_or(&Value::Null), sp.get("values").unwrap_or(&Value::Null));
            let ict = ind.get("componentType").and_then(Value::as_u64).unwrap_or(0);
            let isize = component_size(ict).filter(|_| matches!(ict, 5121 | 5123 | 5125)).ok_or_else(|| self.err(format!("accessor {i}: invalid sparse index type {ict}")))?;
            let (ib, _) = self.view(idx(ind, "bufferView").unwrap_or(usize::MAX))?;
            let (vb, _) = self.view(idx(val, "bufferView").unwrap_or(usize::MAX))?;
            let (ioff, voff) = (idx(ind, "byteOffset").unwrap_or(0), idx(val, "byteOffset").unwrap_or(0));
            if ioff + scount * isize > ib.len() || voff + scount * elem > vb.len() {
                return Err(self.err(format!("accessor {i}: sparse data runs past its bufferView")));
            }
            for s in 0..scount {
                let target = component(ict, &ib[ioff + s * isize..], false) as usize;
                if target >= count {
                    return Err(self.err(format!("accessor {i}: sparse index {target} out of range ({count} elements)")));
                }
                for c in 0..n {
                    out[target * n + c] = conv(component(ct, &vb[voff + s * elem + c * csize..], normalized));
                }
            }
        }
        Ok((n, out))
    }

    fn read_f32(&self, i: usize, want: usize, what: &str) -> Result<Vec<f32>, IoError> {
        let (n, v) = self.read(i, |x| x as f32)?;
        if n != want {
            return Err(self.err(format!("{what} accessor {i} has {n} components, expected {want}")));
        }
        Ok(v)
    }

    /// Bytes of image `i` (data URI, external file or buffer view).
    fn image_bytes(&self, i: usize, resolver: Resolver) -> Option<Vec<u8>> {
        let img = self.doc.get("images")?.get(i)?;
        if let Some(uri) = img.get("uri").and_then(Value::as_str) {
            return load_uri(uri, resolver);
        }
        let (b, _) = self.view(idx(img, "bufferView")?).ok()?;
        Some(b.to_vec())
    }
}

/// Bytes behind a data URI or a relative path.
fn load_uri(uri: &str, resolver: Resolver) -> Option<Vec<u8>> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        if !meta.ends_with(";base64") {
            return None;
        }
        return base64::engine::general_purpose::STANDARD.decode(data.trim()).ok();
    }
    resolver(uri).or_else(|| resolver(&percent_decode(uri)))
}

/// Split a `.glb` into its JSON and optional BIN chunk.
fn split_glb(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>), IoError> {
    let err = |m: &str| IoError::parse(Format::Glb, None, m);
    let u = |o: usize| bytes.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if u(4) != Some(2) {
        return Err(err("only GLB version 2 is supported"));
    }
    let total = (u(8).unwrap_or(0) as usize).min(bytes.len());
    let mut pos = 12;
    let (mut json, mut bin) = (None, None);
    while pos + 8 <= total {
        let (len, kind) = (u(pos).unwrap_or(0) as usize, u(pos + 4).unwrap_or(0));
        let data = bytes.get(pos + 8..pos + 8 + len).ok_or_else(|| err("chunk runs past the end of the file"))?;
        match kind {
            CHUNK_JSON if json.is_none() => json = Some(data),
            CHUNK_BIN if bin.is_none() => bin = Some(data),
            _ => {}
        }
        pos += 8 + ((len + 3) & !3);
    }
    Ok((json.ok_or_else(|| err("no JSON chunk"))?, bin))
}

/// Import a `.gltf` or `.glb` (detected by the `glTF` magic).
pub fn import_into(scene: &mut Scene, bytes: &[u8], resolver: Resolver) -> Result<Vec<usize>, IoError> {
    let (fmt, json_bytes, bin) = if bytes.starts_with(GLB_MAGIC) {
        let (j, b) = split_glb(bytes)?;
        (Format::Glb, j, b)
    } else {
        (Format::Gltf, bytes.strip_prefix(b"\xEF\xBB\xBF".as_slice()).unwrap_or(bytes), None)
    };
    let doc: Value = serde_json::from_slice(json_bytes).map_err(|e| IoError::Parse { format: fmt, line: Some(e.line()), message: format!("invalid JSON: {e}") })?;
    let version = doc.get("asset").and_then(|a| a.get("version")).and_then(Value::as_str).unwrap_or("");
    if !version.starts_with('2') {
        return Err(IoError::Unsupported { format: fmt, message: format!("glTF version \"{version}\" (only 2.x is read)") });
    }
    for r in arr(&doc, "extensionsRequired") {
        let name = r.as_str().unwrap_or("");
        if !SUPPORTED.contains(&name) {
            return Err(IoError::Unsupported { format: fmt, message: format!("required extension {name}") });
        }
    }

    // Buffers and views.
    let mut buffers = Vec::new();
    for (i, b) in arr(&doc, "buffers").iter().enumerate() {
        let data = match b.get("uri").and_then(Value::as_str) {
            Some(uri) => load_uri(uri, resolver).ok_or_else(|| {
                if uri.starts_with("data:") {
                    IoError::parse(fmt, None, format!("buffer {i}: invalid data URI"))
                } else {
                    IoError::MissingFile { format: fmt, path: uri.to_string() }
                }
            })?,
            None if i == 0 => bin.ok_or_else(|| IoError::parse(fmt, None, "buffer 0 has no uri and there is no GLB BIN chunk"))?.to_vec(),
            None => return Err(IoError::parse(fmt, None, format!("buffer {i} has no uri"))),
        };
        let len = idx(b, "byteLength").unwrap_or(data.len());
        if len > data.len() {
            return Err(IoError::parse(fmt, None, format!("buffer {i}: byteLength {len} but only {} bytes of data", data.len())));
        }
        buffers.push(data);
    }
    let mut views = Vec::new();
    for (i, v) in arr(&doc, "bufferViews").iter().enumerate() {
        let buffer = idx(v, "buffer").filter(|&b| b < buffers.len()).ok_or_else(|| IoError::parse(fmt, None, format!("bufferView {i}: invalid buffer")))?;
        let (offset, length) = (idx(v, "byteOffset").unwrap_or(0), idx(v, "byteLength").unwrap_or(0));
        if offset.checked_add(length).is_none_or(|end| end > buffers[buffer].len()) {
            return Err(IoError::parse(fmt, None, format!("bufferView {i}: bytes {offset}..{} past the end of buffer {buffer} ({} bytes)", offset + length, buffers[buffer].len())));
        }
        views.push(View { buffer, offset, length, stride: idx(v, "byteStride").filter(|&s| s > 0) });
    }
    let r = Reader { fmt, doc: &doc, buffers, views };

    // Images (decoded on first use) and materials.
    let mut images: HashMap<usize, Option<usize>> = HashMap::new();
    let mut image_for_texture = |tex: usize, scene: &mut Scene| -> Option<usize> {
        let t = doc.get("textures")?.get(tex)?;
        let src = idx(t, "source")?;
        *images.entry(src).or_insert_with(|| {
            let bytes = r.image_bytes(src, resolver)?;
            let name = doc["images"][src].get("name").and_then(Value::as_str).map_or_else(|| format!("Image{src}"), str::to_string);
            let img = decode_image(&name, &bytes).ok()?;
            scene.images.push(img);
            Some(scene.images.len() - 1)
        })
    };
    let mut materials = Vec::new();
    for (i, m) in arr(&doc, "materials").iter().enumerate() {
        let pbr = m.get("pbrMetallicRoughness").unwrap_or(&Value::Null);
        let mut mat = Material {
            name: m.get("name").and_then(Value::as_str).map_or_else(|| format!("Material{i}"), str::to_string),
            base_color: floats::<4>(pbr.get("baseColorFactor")).unwrap_or([1.0; 4]),
            metallic: num(pbr, "metallicFactor").unwrap_or(1.0),
            roughness: num(pbr, "roughnessFactor").unwrap_or(1.0),
            double_sided: m.get("doubleSided").and_then(Value::as_bool).unwrap_or(false),
            ..Default::default()
        };
        if let Some(tex) = pbr.get("baseColorTexture").and_then(|t| idx(t, "index")) {
            mat.base_color_texture = image_for_texture(tex, scene);
        }
        let e = floats::<3>(m.get("emissiveFactor")).unwrap_or([0.0; 3]);
        mat.emission = Vec3::new(e[0], e[1], e[2]);
        mat.emission_strength = if mat.emission == Vec3::ZERO {
            0.0
        } else {
            ext(m, "KHR_materials_emissive_strength").and_then(|x| num(x, "emissiveStrength")).unwrap_or(1.0)
        };
        if let Some(x) = ext(m, "KHR_materials_ior") {
            mat.ior = num(x, "ior").unwrap_or(1.5);
        }
        if let Some(x) = ext(m, "KHR_materials_transmission") {
            mat.transmission = num(x, "transmissionFactor").unwrap_or(0.0);
        }
        materials.push(scene.add_material(mat));
    }

    // Meshes are built when a node first uses them: (scene mesh, slots).
    let mut meshes: HashMap<usize, (usize, Vec<usize>)> = HashMap::new();
    let mut default_material = None;

    // Which nodes to instantiate.
    let nodes = arr(&doc, "nodes");
    let roots: Vec<usize> = match idx(&doc, "scene").or_else(|| (!arr(&doc, "scenes").is_empty()).then_some(0)) {
        Some(s) => {
            let sc = doc.get("scenes").and_then(|a| a.get(s)).ok_or_else(|| r.err(format!("scene {s} does not exist")))?;
            arr(sc, "nodes").iter().filter_map(Value::as_u64).map(|n| n as usize).collect()
        }
        None => {
            let mut is_child = vec![false; nodes.len()];
            for n in nodes {
                for c in arr(n, "children").iter().filter_map(Value::as_u64) {
                    if let Some(f) = is_child.get_mut(c as usize) {
                        *f = true;
                    }
                }
            }
            (0..nodes.len()).filter(|&i| !is_child[i]).collect()
        }
    };

    let lights = ext(&doc, "KHR_lights_punctual").map_or(&[][..], |l| arr(l, "lights"));
    // Which nodes the scene reaches, and each one's parent node. A node may
    // only appear once; repeats and cycles are ignored.
    let mut included = vec![false; nodes.len()];
    let mut parent_node: Vec<Option<usize>> = vec![None; nodes.len()];
    let mut stack: Vec<(usize, Option<usize>)> = roots.iter().rev().map(|&n| (n, None)).collect();
    while let Some((ni, parent)) = stack.pop() {
        let node = nodes.get(ni).ok_or_else(|| r.err(format!("node {ni} does not exist")))?;
        if std::mem::replace(&mut included[ni], true) {
            continue;
        }
        parent_node[ni] = parent;
        for c in arr(node, "children").iter().rev().filter_map(Value::as_u64) {
            stack.push((c as usize, Some(ni)));
        }
    }
    // A node's object carries the camera/light turn when it is a camera or
    // light without a mesh.
    let turned_node = |n: &Value| idx(n, "mesh").is_none() && (idx(n, "camera").is_some() || ext(n, "KHR_lights_punctual").is_some_and(|l| idx(l, "light").is_some()));

    // Objects are created in node order so exports round-trip their order.
    let mut node_object: Vec<Option<usize>> = vec![None; nodes.len()];
    let mut added = Vec::new();
    for (ni, node) in nodes.iter().enumerate() {
        if !included[ni] {
            continue;
        }
        let parent_turned = parent_node[ni].is_some_and(|p| turned_node(&nodes[p]));
        let (t, q, s) = match floats::<16>(node.get("matrix")) {
            Some(m) => Mat4::from_cols_array(m).to_trs(),
            None => {
                let t = floats::<3>(node.get("translation")).unwrap_or([0.0; 3]);
                let q = floats::<4>(node.get("rotation")).unwrap_or([0.0, 0.0, 0.0, 1.0]);
                let s = floats::<3>(node.get("scale")).unwrap_or([1.0; 3]);
                (Vec3::from_array(t), Quat { x: q[0], y: q[1], z: q[2], w: q[3] }.normalize(), Vec3::from_array(s))
            }
        };
        let camera = match idx(node, "camera") {
            Some(c) => Some(read_camera(doc.get("cameras").and_then(|a| a.get(c)).ok_or_else(|| r.err(format!("node {ni}: camera {c} does not exist")))?)),
            None => None,
        };
        let light = match ext(node, "KHR_lights_punctual").and_then(|l| idx(l, "light")) {
            Some(l) => Some(read_light(lights.get(l).ok_or_else(|| r.err(format!("node {ni}: light {l} does not exist")))?)),
            None => None,
        };
        let mesh = match idx(node, "mesh") {
            Some(mi) => Some(match meshes.get(&mi) {
                Some(m) => m.clone(),
                None => {
                    let gm = doc.get("meshes").and_then(|a| a.get(mi)).ok_or_else(|| r.err(format!("node {ni}: mesh {mi} does not exist")))?;
                    let (mesh, slots) = read_mesh(&r, mi, gm)?;
                    let slots: Vec<usize> = if slots.iter().all(Option::is_none) {
                        Vec::new()
                    } else {
                        slots
                            .iter()
                            .map(|s| match s.and_then(|m| materials.get(m).copied()) {
                                Some(m) => m,
                                None => *default_material.get_or_insert_with(|| scene.add_material(Material::default())),
                            })
                            .collect()
                    };
                    let name = gm.get("name").and_then(Value::as_str).unwrap_or("Mesh");
                    let entry = (scene.add_mesh(name, mesh), slots);
                    meshes.insert(mi, entry.clone());
                    entry
                }
            }),
            None => None,
        };
        let mut extra: Vec<ObjectData> = Vec::new();
        let data = match (mesh.as_ref(), camera, light) {
            (Some(&(m, _)), c, l) => {
                extra.extend(c.map(ObjectData::Camera));
                extra.extend(l.map(ObjectData::Light));
                ObjectData::Mesh(m)
            }
            (None, Some(c), l) => {
                extra.extend(l.map(ObjectData::Light));
                ObjectData::Camera(c)
            }
            (None, None, Some(l)) => ObjectData::Light(l),
            (None, None, None) => ObjectData::Empty,
        };
        let turned = matches!(data, ObjectData::Camera(_) | ObjectData::Light(_));
        let default_name = match data {
            ObjectData::Mesh(m) => scene.mesh_names[m].clone(),
            ObjectData::Camera(_) => "Camera".into(),
            ObjectData::Light(_) => "Light".into(),
            ObjectData::Empty => "Empty".into(),
        };
        let name = node.get("name").and_then(Value::as_str).map_or(default_name, str::to_string);
        let mut o = Object::new(&name, data);
        o.transform = to_scene_transform(t, q, s, parent_turned, turned);
        o.materials = mesh.map(|(_, s)| s).unwrap_or_default();
        let oi = scene.add_object(o);
        added.push(oi);
        // A camera or light sharing a node with a mesh hangs under it.
        for d in extra {
            let mut c = Object::new(if matches!(d, ObjectData::Camera(_)) { "Camera" } else { "Light" }, d);
            c.transform.rotation = q_c();
            c.parent = Some(oi);
            added.push(scene.add_object(c));
        }
        node_object[ni] = Some(oi);
    }
    for (ni, p) in parent_node.iter().enumerate() {
        if let (Some(o), Some(p)) = (node_object[ni], p.and_then(|p| node_object[p])) {
            scene.objects[o].parent = Some(p);
        }
    }
    Ok(added)
}

fn read_camera(c: &Value) -> Camera {
    let mut cam = Camera::default();
    if let Some(p) = c.get("perspective") {
        cam.fov_y = num(p, "yfov").unwrap_or(cam.fov_y);
        cam.near = num(p, "znear").unwrap_or(cam.near);
        cam.far = num(p, "zfar").unwrap_or(cam.far);
    } else if let Some(o) = c.get("orthographic") {
        cam.ortho_scale = Some(2.0 * num(o, "ymag").unwrap_or(1.0).abs());
        cam.near = num(o, "znear").unwrap_or(cam.near);
        cam.far = num(o, "zfar").unwrap_or(cam.far);
    }
    cam
}

fn read_light(l: &Value) -> Light {
    let color = floats::<3>(l.get("color")).map_or(Vec3::ONE, Vec3::from_array);
    let intensity = num(l, "intensity").unwrap_or(1.0);
    let point_watts = intensity * 4.0 * PI / WATTS_TO_LUMENS;
    let (kind, power) = match l.get("type").and_then(Value::as_str) {
        Some("directional") => (LightKind::Sun, intensity),
        Some("spot") => {
            let sp = l.get("spot").unwrap_or(&Value::Null);
            let outer = num(sp, "outerConeAngle").unwrap_or(FRAC_PI_4).max(1e-4);
            let inner = num(sp, "innerConeAngle").unwrap_or(0.0);
            (LightKind::Spot { angle: 2.0 * outer, blend: (1.0 - inner / outer).clamp(0.0, 1.0) }, point_watts)
        }
        _ => (LightKind::Point, point_watts),
    };
    // Punctual lights have no size.
    Light { kind, color, power, radius: 0.0 }
}

/// Build one mesh from a glTF mesh's primitives. Returns the mesh and each
/// slot's glTF material (one slot per distinct primitive material).
fn read_mesh(r: &Reader, mi: usize, gm: &Value) -> Result<(Mesh, Vec<Option<usize>>), IoError> {
    let mut mesh = Mesh::new();
    let mut slots: Vec<Option<usize>> = Vec::new();
    for (pi, prim) in arr(gm, "primitives").iter().enumerate() {
        let attrs = prim.get("attributes").unwrap_or(&Value::Null);
        let Some(pos_acc) = idx(attrs, "POSITION") else { continue };
        let pos = r.read_f32(pos_acc, 3, "POSITION")?;
        let count = pos.len() / 3;
        let normals = idx(attrs, "NORMAL").map(|a| r.read_f32(a, 3, "NORMAL")).transpose()?;
        let uvs = idx(attrs, "TEXCOORD_0").map(|a| r.read_f32(a, 2, "TEXCOORD_0")).transpose()?;
        if normals.as_ref().is_some_and(|n| n.len() != pos.len()) || uvs.as_ref().is_some_and(|u| u.len() / 2 != count) {
            return Err(r.err(format!("mesh {mi} primitive {pi}: attribute counts differ from POSITION")));
        }
        let indices: Vec<u32> = match idx(prim, "indices") {
            Some(a) => r.read(a, |x| x as u32)?.1,
            None => (0..count as u32).collect(),
        };
        if let Some(&bad) = indices.iter().find(|&&i| i as usize >= count) {
            return Err(r.err(format!("mesh {mi} primitive {pi}: index {bad} out of range ({count} vertices)")));
        }
        let material = idx(prim, "material");
        let slot = match slots.iter().position(|&s| s == material) {
            Some(s) => s,
            None => {
                slots.push(material);
                slots.len() - 1
            }
        } as u16;
        let base = mesh.positions.len() as u32;
        let p = |i: u32| Vec3::new(pos[i as usize * 3], pos[i as usize * 3 + 1], pos[i as usize * 3 + 2]);
        mesh.positions.extend((0..count as u32).map(|i| y_up_to_z_up(p(i))));

        let n = indices.len();
        let mut tris: Vec<[u32; 3]> = Vec::new();
        let mut lines: Vec<[u32; 2]> = Vec::new();
        match idx(prim, "mode").unwrap_or(4) {
            4 => tris.extend(indices.chunks_exact(3).map(|c| [c[0], c[1], c[2]])),
            5 => tris.extend((0..n.saturating_sub(2)).map(|i| if i % 2 == 0 { [indices[i], indices[i + 1], indices[i + 2]] } else { [indices[i], indices[i + 2], indices[i + 1]] })),
            6 => tris.extend((1..n.saturating_sub(1)).map(|i| [indices[0], indices[i], indices[i + 1]])),
            1 => lines.extend(indices.chunks_exact(2).map(|c| [c[0], c[1]])),
            3 => lines.extend(indices.windows(2).map(|c| [c[0], c[1]])),
            2 => {
                lines.extend(indices.windows(2).map(|c| [c[0], c[1]]));
                if n > 2 {
                    lines.push([indices[n - 1], indices[0]]);
                }
            }
            _ => {} // points carry no faces or edges
        }
        mesh.faces.reserve(tris.len());
        for t in tris {
            if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
                continue; // degenerate (strip restarts)
            }
            let mut face = Face { verts: t.map(|i| base + i).to_vec(), material: slot, ..Default::default() };
            if let Some(uv) = &uvs {
                face.uvs = t.iter().map(|&i| Vec2::new(uv[i as usize * 2], 1.0 - uv[i as usize * 2 + 1])).collect();
            }
            if let Some(nr) = &normals {
                let [a, b, c] = t.map(p);
                let flat = (b - a).cross(c - a).normalize();
                let corner = |i: u32| Vec3::new(nr[i as usize * 3], nr[i as usize * 3 + 1], nr[i as usize * 3 + 2]).normalize();
                face.smooth = flat != Vec3::ZERO && t.iter().any(|&i| corner(i).dot(flat) < 0.9999);
            }
            mesh.faces.push(face);
        }
        mesh.loose_edges.extend(lines.into_iter().filter(|l| l[0] != l[1]).map(|l| [base + l[0], base + l[1]]));
    }
    weld_exact(&mut mesh);
    // Edges that a face now covers are not loose.
    if !mesh.loose_edges.is_empty() {
        let face_edges: std::collections::HashSet<(u32, u32)> = mesh.faces.iter().flat_map(|f| f.edges().map(|(a, b)| (a.min(b), a.max(b)))).collect();
        let mut seen = std::collections::HashSet::new();
        mesh.loose_edges.retain(|e| {
            let k = (e[0].min(e[1]), e[0].max(e[1]));
            !face_edges.contains(&k) && seen.insert(k)
        });
    }
    mesh.remove_unused_vertices();
    Ok((mesh, slots))
}

// ---------------------------------------------------------------------------
// Export.

/// Binary buffer, views and accessors under construction.
#[derive(Default)]
struct Bin {
    data: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Bin {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.data.len() % 4 != 0 {
            self.data.push(0);
        }
        let mut v = json!({ "buffer": 0, "byteOffset": self.data.len(), "byteLength": bytes.len() });
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.data.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }

    /// Float accessor of `n`-component elements; positions also get min/max.
    fn floats(&mut self, values: &[f32], n: usize, with_bounds: bool) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34962));
        let ty = ["SCALAR", "VEC2", "VEC3", "VEC4"][n - 1];
        let mut a = json!({ "bufferView": view, "componentType": 5126, "count": values.len() / n, "type": ty });
        if with_bounds {
            let (mut lo, mut hi) = (vec![f32::INFINITY; n], vec![f32::NEG_INFINITY; n]);
            for e in values.chunks_exact(n) {
                for c in 0..n {
                    lo[c] = lo[c].min(e[c]);
                    hi[c] = hi[c].max(e[c]);
                }
            }
            a["min"] = json!(lo);
            a["max"] = json!(hi);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn indices(&mut self, idx: &[u32]) -> usize {
        let bytes: Vec<u8> = idx.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34963));
        self.accessors.push(json!({ "bufferView": view, "componentType": 5125, "count": idx.len(), "type": "SCALAR" }));
        self.accessors.len() - 1
    }
}

/// Vertex attributes of one primitive, deduplicated per (vertex, normal, UV).
#[derive(Default)]
struct PrimBuild {
    map: HashMap<(u32, [u32; 3], [u32; 2]), u32>,
    positions: Vec<f32>,
    normals: Vec<f32>,
    uvs: Vec<f32>,
    indices: Vec<u32>,
}

/// glTF primitives for `m` with one primitive per used slot (`slots` maps
/// slot to glTF material index), plus a LINES primitive for loose edges.
fn mesh_primitives(bin: &mut Bin, m: &Mesh, slots: &[usize]) -> Vec<Value> {
    let with_uv = m.faces.iter().any(|f| !f.uvs.is_empty());
    let vnormals = m.vertex_normals();
    let fnormals: Vec<Vec3> = (0..m.faces.len()).map(|f| m.face_normal(f)).collect();
    let nslots = slots.len().max(1);
    let mut prims: Vec<PrimBuild> = (0..nslots).map(|_| PrimBuild::default()).collect();
    for t in m.triangulate() {
        let f = &m.faces[t.face as usize];
        let slot = (f.material as usize).min(nslots - 1);
        let pb = &mut prims[slot];
        for (k, &v) in t.verts.iter().enumerate() {
            let n = z_up_to_y_up(if f.smooth { vnormals[v as usize] } else { fnormals[t.face as usize] });
            let uv = if f.uvs.len() == f.verts.len() { f.uvs[t.corners[k] as usize] } else { Vec2::ZERO };
            let uv = Vec2::new(uv.x, 1.0 - uv.y);
            let key = (v, [n.x, n.y, n.z].map(f32::to_bits), [uv.x, uv.y].map(f32::to_bits));
            let next = pb.map.len() as u32;
            let i = *pb.map.entry(key).or_insert(next);
            if i == next {
                let p = z_up_to_y_up(m.positions[v as usize]);
                pb.positions.extend_from_slice(&[p.x, p.y, p.z]);
                pb.normals.extend_from_slice(&[n.x, n.y, n.z]);
                pb.uvs.extend_from_slice(&[uv.x, uv.y]);
            }
            pb.indices.push(i);
        }
    }
    let mut out = Vec::new();
    for (slot, pb) in prims.iter().enumerate() {
        if pb.indices.is_empty() {
            continue;
        }
        let mut attrs = Map::new();
        attrs.insert("POSITION".into(), json!(bin.floats(&pb.positions, 3, true)));
        attrs.insert("NORMAL".into(), json!(bin.floats(&pb.normals, 3, false)));
        if with_uv {
            attrs.insert("TEXCOORD_0".into(), json!(bin.floats(&pb.uvs, 2, false)));
        }
        let mut p = json!({ "attributes": attrs, "indices": bin.indices(&pb.indices), "mode": 4 });
        if let Some(&mat) = slots.get(slot) {
            p["material"] = json!(mat);
        }
        out.push(p);
    }
    if !m.loose_edges.is_empty() {
        let pos: Vec<f32> = m.positions.iter().flat_map(|&p| z_up_to_y_up(p).to_array()).collect();
        let idx: Vec<u32> = m.loose_edges.iter().flatten().copied().collect();
        let pa = bin.floats(&pos, 3, true);
        out.push(json!({ "attributes": { "POSITION": pa }, "indices": bin.indices(&idx), "mode": 1 }));
    }
    out
}

/// True for objects whose glTF node carries the camera/light axis turn.
fn is_turned(o: &Object) -> bool {
    match &o.data {
        ObjectData::Camera(_) => true,
        ObjectData::Light(l) => !matches!(l.kind, LightKind::Area { .. }),
        _ => false,
    }
}

fn material_json(m: &Material, texture: Option<usize>, used: &mut Vec<&'static str>) -> Value {
    let mut pbr = json!({ "baseColorFactor": m.base_color, "metallicFactor": m.metallic, "roughnessFactor": m.roughness });
    if let Some(t) = texture {
        pbr["baseColorTexture"] = json!({ "index": t });
    }
    let mut v = json!({ "name": m.name, "pbrMetallicRoughness": pbr });
    if m.double_sided {
        v["doubleSided"] = json!(true);
    }
    if m.base_color[3] < 1.0 {
        v["alphaMode"] = json!("BLEND");
    }
    let mut exts = Map::new();
    let mut use_ext = |name: &'static str, val: Value, used: &mut Vec<&'static str>| {
        exts.insert(name.into(), val);
        if !used.contains(&name) {
            used.push(name);
        }
    };
    let e = m.emission * m.emission_strength;
    let peak = e.max_elem();
    if peak > 0.0 {
        if peak > 1.0 {
            v["emissiveFactor"] = json!((e / peak).to_array());
            use_ext("KHR_materials_emissive_strength", json!({ "emissiveStrength": peak }), used);
        } else {
            v["emissiveFactor"] = json!(e.to_array());
        }
    }
    if (m.ior - 1.5).abs() > 1e-6 {
        use_ext("KHR_materials_ior", json!({ "ior": m.ior }), used);
    }
    if m.transmission > 0.0 {
        use_ext("KHR_materials_transmission", json!({ "transmissionFactor": m.transmission }), used);
    }
    if !exts.is_empty() {
        v["extensions"] = Value::Object(exts);
    }
    v
}

/// Effective parent of each object: `None` for roots, invalid indices and
/// objects caught in a parent cycle.
fn effective_parents(scene: &Scene) -> Vec<Option<usize>> {
    let n = scene.objects.len();
    (0..n)
        .map(|i| {
            let p = scene.objects[i].parent.filter(|&p| p < n && p != i)?;
            // Walk up; a chain longer than n objects is a cycle.
            let mut cur = Some(p);
            for _ in 0..n {
                match cur {
                    None => return Some(p),
                    Some(c) if c == i => return None,
                    Some(c) => cur = scene.objects[c].parent.filter(|&q| q < n),
                }
            }
            None
        })
        .collect()
}

/// Write the scene as glTF 2.0: `.glb` when `binary`, else `.gltf` JSON with
/// the buffer embedded as a data URI.
pub fn export(scene: &Scene, binary: bool) -> Result<Vec<u8>, IoError> {
    let mut bin = Bin::default();
    let mut used_ext: Vec<&'static str> = Vec::new();

    // Textures: only images that materials use, encoded once each.
    let mut image_json = Vec::new();
    let mut texture_of_image: HashMap<usize, usize> = HashMap::new();
    let mut materials = Vec::with_capacity(scene.materials.len());
    for m in &scene.materials {
        let tex = m.base_color_texture.filter(|&i| i < scene.images.len()).map(|ii| {
            *texture_of_image.entry(ii).or_insert_with(|| {
                let view = bin.view(&encode_png(&scene.images[ii]), None);
                image_json.push(json!({ "name": scene.images[ii].name, "mimeType": "image/png", "bufferView": view }));
                image_json.len() - 1
            })
        });
        materials.push(material_json(m, tex, &mut used_ext));
    }

    let parents = effective_parents(scene);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); scene.objects.len()];
    for (i, p) in parents.iter().enumerate() {
        if let Some(p) = p {
            children[*p].push(i);
        }
    }
    let mut meshes: Vec<Value> = Vec::new();
    let mut mesh_cache: HashMap<(usize, Vec<usize>), Option<usize>> = HashMap::new();
    let mut cameras = Vec::new();
    let mut lights = Vec::new();
    let mut nodes = Vec::with_capacity(scene.objects.len());
    for (i, o) in scene.objects.iter().enumerate() {
        let turned = is_turned(o);
        let parent_turned = parents[i].is_some_and(|p| is_turned(&scene.objects[p]));
        let (t, q, s) = to_gltf_transform(&o.transform, parent_turned, turned);
        let mut node = json!({ "name": o.name });
        if t != Vec3::ZERO {
            node["translation"] = json!(t.to_array());
        }
        if q != Quat::IDENTITY {
            node["rotation"] = json!(q.to_array());
        }
        if s != Vec3::ONE {
            node["scale"] = json!(s.to_array());
        }
        if !children[i].is_empty() {
            node["children"] = json!(children[i]);
        }
        match &o.data {
            ObjectData::Mesh(mi) if *mi < scene.meshes.len() => {
                let slots: Vec<usize> = o.materials.iter().copied().filter(|&m| m < scene.materials.len()).collect();
                let build = |bin: &mut Bin, meshes: &mut Vec<Value>, mesh: &Mesh| {
                    let prims = mesh_primitives(bin, mesh, &slots);
                    (!prims.is_empty()).then(|| {
                        meshes.push(json!({ "name": scene.mesh_names.get(*mi).cloned().unwrap_or_default(), "primitives": prims }));
                        meshes.len() - 1
                    })
                };
                let gm = if o.modifiers.is_empty() {
                    let key = (*mi, slots.clone());
                    match mesh_cache.get(&key) {
                        Some(&g) => g,
                        None => {
                            let g = build(&mut bin, &mut meshes, &scene.meshes[*mi]);
                            mesh_cache.insert(key, g);
                            g
                        }
                    }
                } else {
                    let evaluated = scene.evaluated_mesh(i).unwrap_or_default();
                    build(&mut bin, &mut meshes, &evaluated)
                };
                if let Some(g) = gm {
                    node["mesh"] = json!(g);
                }
            }
            ObjectData::Camera(c) => {
                let mut cam = match c.ortho_scale {
                    Some(h) => json!({ "type": "orthographic", "orthographic": { "xmag": h * 0.5 * 16.0 / 9.0, "ymag": h * 0.5, "znear": c.near, "zfar": c.far } }),
                    None => json!({ "type": "perspective", "perspective": { "yfov": c.fov_y, "znear": c.near, "zfar": c.far } }),
                };
                cam["name"] = json!(o.name);
                cameras.push(cam);
                node["camera"] = json!(cameras.len() - 1);
            }
            ObjectData::Light(l) if turned => {
                let to_cd = WATTS_TO_LUMENS / (4.0 * PI);
                let mut lj = match l.kind {
                    LightKind::Sun => json!({ "type": "directional", "intensity": l.power }),
                    LightKind::Spot { angle, blend } => {
                        let outer = (angle * 0.5).clamp(1e-4, FRAC_PI_2);
                        json!({ "type": "spot", "intensity": l.power * to_cd, "spot": { "innerConeAngle": outer * (1.0 - blend.clamp(0.0, 1.0)), "outerConeAngle": outer } })
                    }
                    _ => json!({ "type": "point", "intensity": l.power * to_cd }),
                };
                lj["color"] = json!(l.color.to_array());
                lj["name"] = json!(o.name);
                lights.push(lj);
                node["extensions"] = json!({ "KHR_lights_punctual": { "light": lights.len() - 1 } });
            }
            _ => {}
        }
        nodes.push(node);
    }
    let roots: Vec<usize> = (0..scene.objects.len()).filter(|&i| parents[i].is_none()).collect();

    let mut doc = json!({
        "asset": { "version": "2.0", "generator": "Polyamour (SexOS)" },
        "scene": 0,
        "scenes": [{ "name": scene.name, "nodes": roots }],
        "nodes": nodes,
    });
    if !lights.is_empty() {
        used_ext.push("KHR_lights_punctual");
        doc["extensions"] = json!({ "KHR_lights_punctual": { "lights": lights } });
    }
    let sets: [(&str, Vec<Value>); 4] = [("meshes", meshes), ("materials", materials), ("cameras", cameras), ("images", image_json.clone())];
    for (key, v) in sets {
        if !v.is_empty() {
            doc[key] = Value::Array(v);
        }
    }
    if !image_json.is_empty() {
        doc["samplers"] = json!([{}]);
        doc["textures"] = Value::Array((0..image_json.len()).map(|i| json!({ "source": i, "sampler": 0 })).collect());
    }
    if !used_ext.is_empty() {
        doc["extensionsUsed"] = json!(used_ext);
    }
    let Bin { data, views, accessors } = bin;
    if !views.is_empty() {
        doc["bufferViews"] = Value::Array(views);
    }
    if !accessors.is_empty() {
        doc["accessors"] = Value::Array(accessors);
    }
    if !data.is_empty() {
        let mut buf = json!({ "byteLength": data.len() });
        if !binary {
            buf["uri"] = json!(format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(&data)));
        }
        doc["buffers"] = json!([buf]);
    }

    let fmt = if binary { Format::Glb } else { Format::Gltf };
    let to_err = |e: serde_json::Error| IoError::parse(fmt, None, format!("JSON encoding failed: {e}"));
    if !binary {
        return serde_json::to_vec_pretty(&doc).map_err(to_err);
    }
    let mut json_chunk = serde_json::to_vec(&doc).map_err(to_err)?;
    while json_chunk.len() % 4 != 0 {
        json_chunk.push(b' ');
    }
    let mut bin_chunk = data;
    while bin_chunk.len() % 4 != 0 {
        bin_chunk.push(0);
    }
    let total = 12 + 8 + json_chunk.len() + if bin_chunk.is_empty() { 0 } else { 8 + bin_chunk.len() };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(GLB_MAGIC);
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json_chunk);
    if !bin_chunk.is_empty() {
        out.extend_from_slice(&(bin_chunk.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(&bin_chunk);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_conversion_round_trips() {
        let q = Quat::from_euler_xyz(0.3, -0.7, 1.1);
        for (pt, st) in [(false, false), (true, false), (false, true), (true, true)] {
            let tr = to_scene_transform(Vec3::new(1.0, 2.0, 3.0), q, Vec3::new(1.0, 2.0, 3.0), pt, st);
            let (t, q2, s) = to_gltf_transform(&tr, pt, st);
            assert!(t.approx_eq(Vec3::new(1.0, 2.0, 3.0), 1e-5));
            assert!(s.approx_eq(Vec3::new(1.0, 2.0, 3.0), 1e-5));
            let d = (q.x * q2.x + q.y * q2.y + q.z * q2.z + q.w * q2.w).abs();
            assert!((d - 1.0).abs() < 1e-5);
        }
        // A Y-up translation along +Y lands on scene +Z.
        assert!(to_scene_transform(Vec3::Y, Quat::IDENTITY, Vec3::ONE, false, false).location.approx_eq(Vec3::Z, 1e-6));
    }

    #[test]
    fn conj_c_matches_quaternion_product() {
        let q = Quat::from_euler_xyz(0.2, 0.5, -0.4);
        let a = conj_c(q);
        let b = q_c() * q * q_c().conjugate();
        assert!((a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.z - b.z).abs() < 1e-6 && (a.w - b.w).abs() < 1e-6);
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("my%20file.bin"), "my file.bin");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
