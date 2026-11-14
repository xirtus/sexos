// SPDX-License-Identifier: GPL-3.0-only
//! File format tests: hand-written fixtures, error messages and round trips
//! (export -> import) for OBJ, STL, PLY, glTF and GLB.
use base64::Engine as _;
use sex_3d::io::{self, export, export_files, import, import_into, no_files, Format, IoError};
use sex_3d::math::{Quat, Vec2, Vec3};
use sex_3d::mesh::Mesh;
use sex_3d::primitives;
use sex_3d::scene::{Camera, Image, Light, LightKind, Material, Object, ObjectData, Scene};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Helpers.

fn resolver(files: &[(&str, Vec<u8>)]) -> impl Fn(&str) -> Option<Vec<u8>> {
    let map: HashMap<String, Vec<u8>> = files.iter().map(|(n, b)| (n.to_string(), b.clone())).collect();
    move |p: &str| map.get(p).cloned()
}

fn mesh_of(s: &Scene, obj: usize) -> &Mesh {
    match s.objects[obj].data {
        ObjectData::Mesh(m) => &s.meshes[m],
        ref d => panic!("object {} is {d:?}, not a mesh", s.objects[obj].name),
    }
}

/// Every position of `a` has one in `b` within `eps` and the counts match.
/// (Brute force; test meshes are small.)
fn same_point_set(a: &[Vec3], b: &[Vec3], eps: f32) {
    assert_eq!(a.len(), b.len(), "vertex counts differ");
    for p in a {
        assert!(b.iter().any(|q| p.approx_eq(*q, eps)), "{p:?} missing");
    }
}

/// (position, uv) pairs of every face corner, for comparing UV layouts across
/// re-triangulation and vertex reordering.
fn corner_uvs(m: &Mesh) -> Vec<(Vec3, Vec2)> {
    let mut v = Vec::new();
    for f in &m.faces {
        for (i, &c) in f.verts.iter().enumerate() {
            v.push((m.positions[c as usize], f.uvs[i]));
        }
    }
    v
}

fn assert_uvs_match(orig: &Mesh, got: &Mesh) {
    assert!(got.has_uvs(), "UVs lost");
    let want = corner_uvs(orig);
    for (p, uv) in corner_uvs(got) {
        assert!(
            want.iter().any(|(q, w)| p.approx_eq(*q, 1e-5) && (uv.x - w.x).abs() < 1e-5 && (uv.y - w.y).abs() < 1e-5),
            "corner at {p:?} has unexpected uv {uv:?}"
        );
    }
}

fn world_point(s: &Scene, obj: usize, p: Vec3) -> Vec3 {
    s.world_matrix(obj).transform_point(p)
}

fn quat_close(a: Quat, b: Quat) -> bool {
    (a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w).abs() > 1.0 - 1e-5
}

fn single_mesh_scene(name: &str, m: Mesh) -> Scene {
    let mut s = Scene::new();
    s.add_mesh_object(name, m);
    s
}

fn test_primitives() -> Vec<(&'static str, Mesh)> {
    vec![
        ("cube", primitives::cube(2.0)),
        ("grid", primitives::grid(3, 2, 2.0)),
        ("uv_sphere", primitives::uv_sphere(12, 6, 1.0)),
        ("ico_sphere", primitives::ico_sphere(1, 1.0)),
        ("cylinder", primitives::cylinder(8, 1.0, 2.0)),
        ("cone", primitives::cone(8, 1.0, 0.0, 2.0)),
        ("torus", primitives::torus(12, 6, 1.0, 0.25)),
    ]
}

// ---------------------------------------------------------------------------
// Format detection.

#[test]
fn format_from_path_and_sniff() {
    assert_eq!(Format::from_path("a/b/Box.GLB"), Some(Format::Glb));
    assert_eq!(Format::from_path("C:\\x\\scene.blend"), Some(Format::Blend));
    assert_eq!(Format::from_path("model.obj"), Some(Format::Obj));
    assert_eq!(Format::from_path("noext"), None);
    assert_eq!(Format::from_path("dir.v2/noext"), None);
    assert_eq!(Format::sniff(b"glTF\x02\0\0\0"), Some(Format::Glb));
    assert_eq!(Format::sniff(b"ply\nformat ascii 1.0\n"), Some(Format::Ply));
    assert_eq!(Format::sniff(b"BLENDER-v300"), Some(Format::Blend));
    assert_eq!(Format::sniff(b"  solid x\n"), Some(Format::Stl));
    assert_eq!(Format::sniff(b"{\"asset\":{\"version\":\"2.0\"}}"), Some(Format::Gltf));
    assert_eq!(Format::sniff(b"v 1 2 3\n"), None);
    for f in Format::ALL {
        assert_eq!(Format::from_extension(f.extension()), Some(f));
    }
}

// ---------------------------------------------------------------------------
// OBJ.

const OBJ: &str = "# test
mtllib test.mtl
o Pyramid
v -1 0 1
v 1 0 1
v 1 0 -1
v -1 0 -1
v 0 2 0
vt 0 0
vt 1 0
vt 1 1
vt 0 1
vt 0.5 1
vn 0 1 0
usemtl Red
s off
f 1/1/1 4/4/1 3/3/1 2/2/1
usemtl Glow
s 1
f 1/1 2/2 5/5
f 2/2 3/3 5/5
f 3/3 4/4 5/5
f 4/4 1/1 5/5
g Wire
v 5 0 0
v 6 0 0
v 6 0 1
l -3 -2 -1
";

const MTL: &str = "newmtl Red
Kd 0.8 0.1 0.1
Ns 250
d 0.5
Ni 1.45
Pm 0.25

newmtl Glow # comment
Kd 1 1 1
Ke 2 1 0
Pr 0.75
";

#[test]
fn obj_fixture_with_mtl() {
    let files = [("test.mtl", MTL.as_bytes().to_vec())];
    let s = import(OBJ.as_bytes(), Format::Obj, &resolver(&files)).unwrap();
    assert_eq!(s.objects.len(), 2);
    assert_eq!(s.objects[0].name, "Pyramid");
    assert_eq!(s.objects[1].name, "Wire");
    let m = mesh_of(&s, 0);
    assert_eq!((m.vertex_count(), m.face_count()), (5, 5));
    assert_eq!(m.faces[0].verts.len(), 4, "quad stays an n-gon");
    // Y-up apex (0, 2, 0) is scene Z-up (0, 0, 2); file -Z is scene +Y.
    assert!(m.positions[4].approx_eq(Vec3::new(0.0, 0.0, 2.0), 1e-6));
    assert!(m.positions[2].approx_eq(Vec3::new(1.0, 1.0, 0.0), 1e-6));
    assert!(m.has_uvs());
    assert_eq!(m.faces[1].uvs[2], Vec2::new(0.5, 1.0));
    assert!(!m.faces[0].smooth && m.faces[1].smooth);
    assert_eq!(m.faces[0].material, 0);
    assert_eq!(m.faces[1].material, 1);
    // A closed pyramid: the base quad faces down after the axis change.
    assert!(m.half_edges().is_closed_manifold());
    assert!(m.volume() > 0.0, "winding must survive the axis conversion");

    let slots = &s.objects[0].materials;
    let red = &s.materials[slots[0]];
    assert_eq!(red.name, "Red");
    assert_eq!(red.base_color, [0.8, 0.1, 0.1, 0.5]);
    assert!((red.roughness - 0.5).abs() < 1e-6, "Ns 250 -> roughness 0.5");
    assert_eq!((red.ior, red.metallic), (1.45, 0.25));
    let glow = &s.materials[slots[1]];
    assert_eq!(glow.emission, Vec3::new(2.0, 1.0, 0.0));
    assert_eq!(glow.emission_strength, 1.0);
    assert_eq!(glow.roughness, 0.75);

    let wire = mesh_of(&s, 1);
    assert_eq!(wire.vertex_count(), 3);
    assert_eq!(wire.loose_edges, vec![[0, 1], [1, 2]]);
    assert!(wire.faces.is_empty());
}

#[test]
fn obj_negative_indices_and_missing_mtl() {
    let src = "v 0 0 0\nv 1 0 0\nv 1 1 0\nf -3 -2 -1\nv 0 1 0\nf 1 3 -1\n";
    let s = import(src.as_bytes(), Format::Obj, &no_files).unwrap();
    let m = mesh_of(&s, 0);
    assert_eq!(m.faces[0].verts, vec![0, 1, 2]);
    assert_eq!(m.faces[1].verts, vec![0, 2, 3]);
    assert!(s.objects[0].materials.is_empty());
    // usemtl without a library makes a default material of that name.
    let s = import(b"mtllib gone.mtl\nusemtl Steel\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n", Format::Obj, &no_files).unwrap();
    assert_eq!(s.materials[s.objects[0].materials[0]].name, "Steel");
}

#[test]
fn obj_errors_have_line_numbers() {
    let e = import(b"v 0 0 0\nv 1 0 0\nv 0 x 0\n", Format::Obj, &no_files).unwrap_err();
    assert_eq!(e.line(), Some(3));
    assert_eq!(e.to_string(), "OBJ line 3: invalid number \"x\"");
    let e = import(b"v 0 0 0\nv 1 0 0\n\nf 1 2 9\n", Format::Obj, &no_files).unwrap_err();
    assert_eq!(e.to_string(), "OBJ line 4: vertex index 9 out of range (2 defined)");
    let e = import(b"v 0 0 0\nf 1 -2 1\n", Format::Obj, &no_files).unwrap_err();
    assert_eq!(e.line(), Some(2));
    let e = import(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1/4 2/1 3/1\n", Format::Obj, &no_files).unwrap_err();
    assert!(e.to_string().contains("texture coordinate index"), "{e}");
    let mtl = [("bad.mtl", b"newmtl A\nKd 1 1 1\nNs lots\n".to_vec())];
    let e = import(b"mtllib bad.mtl\n", Format::Obj, &resolver(&mtl)).unwrap_err();
    assert_eq!(e.to_string(), "OBJ line 3: in .mtl: invalid number \"lots\"");
}

#[test]
fn obj_round_trip_primitives() {
    for (name, m) in test_primitives() {
        let s = single_mesh_scene(name, m.clone());
        let back = import(&export(&s, Format::Obj).unwrap(), Format::Obj, &no_files).unwrap();
        let got = mesh_of(&back, 0);
        assert_eq!((got.vertex_count(), got.face_count()), (m.vertex_count(), m.face_count()), "{name}");
        for (a, b) in m.positions.iter().zip(&got.positions) {
            assert!(a.approx_eq(*b, 1e-5), "{name}: {a:?} vs {b:?}");
        }
        for (a, b) in m.faces.iter().zip(&got.faces) {
            assert_eq!(a.verts, b.verts, "{name}");
            assert_eq!(a.uvs, b.uvs, "{name}");
            assert_eq!(a.smooth, b.smooth, "{name}");
        }
    }
}

#[test]
fn obj_round_trip_startup_with_materials_and_texture() {
    let mut s = Scene::startup();
    let mut tex = Image::new("checker", 2, 2);
    tex.pixels = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [0.2, 0.2, 0.2, 0.5]];
    s.images.push(tex);
    let mat = s.add_material(Material {
        name: "Brick Red".into(),
        base_color: [0.6, 0.2, 0.1, 1.0],
        base_color_texture: Some(0),
        metallic: 0.5,
        roughness: 0.3,
        emission: Vec3::new(1.0, 0.5, 0.25),
        emission_strength: 2.0,
        ..Default::default()
    });
    let sphere = s.add_mesh_object("Sphere", primitives::uv_sphere(8, 4, 1.0));
    s.objects[sphere].materials.push(mat);
    s.objects[sphere].transform.location = Vec3::new(3.0, 0.0, 1.0);

    let files = export_files(&s, Format::Obj, "scene").unwrap();
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["scene.obj", "scene.mtl", "scene_checker.png"]);
    let main = files[0].1.clone();
    assert!(String::from_utf8_lossy(&main).starts_with("# Polyamour OBJ\nmtllib scene.mtl\n"));
    let lookup: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let back = import(&main, Format::Obj, &resolver(&lookup)).unwrap();
    assert_eq!(back.objects.len(), 2, "cube and sphere; light and camera are not OBJ data");
    // Transforms are applied: the sphere sits at x = 3 in world space.
    let sm = mesh_of(&back, 1);
    let c = sm.bounds().center();
    assert!(c.approx_eq(Vec3::new(3.0, 0.0, 1.0), 1e-5), "{c:?}");
    let m = &back.materials[back.objects[1].materials[0]];
    assert_eq!(m.name, "Brick_Red");
    assert!((m.roughness - 0.3).abs() < 1e-6 && (m.metallic - 0.5).abs() < 1e-6);
    assert!(m.emission.approx_eq(Vec3::new(2.0, 1.0, 0.5), 1e-6));
    let img = &back.images[m.base_color_texture.expect("texture loaded")];
    assert_eq!((img.width, img.height), (2, 2));
    for (a, b) in img.pixels.iter().zip(&s.images[0].pixels) {
        for k in 0..4 {
            assert!((a[k] - b[k]).abs() < 0.01, "{a:?} vs {b:?}");
        }
    }
    let cube = &back.materials[back.objects[0].materials[0]];
    assert_eq!(cube.name, "Material");
    assert_eq!(cube.base_color, [0.8, 0.8, 0.8, 1.0]);
}

// ---------------------------------------------------------------------------
// STL.

const ASCII_STL: &str = "solid tetra
  facet normal 0 0 -1
    outer loop
      vertex 0 0 0
      vertex 0 1 0
      vertex 1 0 0
    endloop
  endfacet
  facet normal 0 -1 0
    outer loop
      vertex 0 0 0
      vertex 1 0 0
      vertex 0 0 1
    endloop
  endfacet
  facet normal -1 0 0
    outer loop
      vertex 0 0 0
      vertex 0 0 1
      vertex 0 1 0
    endloop
  endfacet
  facet normal 1 1 1
    outer loop
      vertex 1 0 0
      vertex 0 1 0
      vertex 0 0 1
    endloop
  endfacet
endsolid tetra
";

fn binary_stl(header: &[u8], tris: &[[[f32; 3]; 3]]) -> Vec<u8> {
    let mut out = vec![b' '; 80];
    out[..header.len()].copy_from_slice(header);
    out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    for t in tris {
        out.extend_from_slice(&[0u8; 12]);
        for v in t {
            for c in v {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

#[test]
fn stl_ascii_fixture_is_welded() {
    let s = import(ASCII_STL.as_bytes(), Format::Stl, &no_files).unwrap();
    assert_eq!(s.objects[0].name, "tetra");
    let m = mesh_of(&s, 0);
    assert_eq!((m.vertex_count(), m.face_count()), (4, 4));
    assert!(m.half_edges().is_closed_manifold());
    assert!((m.volume() - 1.0 / 6.0).abs() < 1e-6);
}

#[test]
fn stl_binary_detected_by_size_even_with_solid_header() {
    let tris = [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]];
    let bytes = binary_stl(b"solid but actually binary", &tris);
    assert_eq!(Format::sniff(&bytes), Some(Format::Stl));
    let s = import(&bytes, Format::Stl, &no_files).unwrap();
    let m = mesh_of(&s, 0);
    assert_eq!((m.vertex_count(), m.face_count()), (4, 2));
    // Truncated binary files are reported, not misread as text.
    let e = import(&bytes[..bytes.len() - 10], Format::Stl, &no_files).unwrap_err();
    assert!(e.to_string().contains("declares 2 triangles"), "{e}");
}

#[test]
fn stl_ascii_errors_have_line_numbers() {
    let bad = ASCII_STL.replace("vertex 0 0 1\n    endloop\n  endfacet\n  facet normal -1", "vertex 0 0 one\n    endloop\n  endfacet\n  facet normal -1");
    let e = import(bad.as_bytes(), Format::Stl, &no_files).unwrap_err();
    assert_eq!(e.to_string(), "STL line 13: invalid number \"one\"");
    let e = import(b"solid x\n facet normal 0 0 1\n  outer loop\n   vertex 0 0\n", Format::Stl, &no_files).unwrap_err();
    assert_eq!(e.line(), Some(4));
}

#[test]
fn stl_round_trip_binary_and_ascii() {
    for (name, m) in test_primitives() {
        let s = single_mesh_scene(name, m.clone());
        let tris = m.triangulate().len();
        for bytes in [export(&s, Format::Stl).unwrap(), io::stl::export_ascii(&s)] {
            let back = import(&bytes, Format::Stl, &no_files).unwrap();
            let got = mesh_of(&back, 0);
            assert_eq!(got.face_count(), tris, "{name}");
            same_point_set(&m.positions, &got.positions, 1e-5);
            assert!((got.volume() - m.volume()).abs() < 1e-4, "{name}: orientation kept");
        }
    }
    let ascii = io::stl::export_ascii(&Scene::startup());
    assert!(String::from_utf8_lossy(&ascii).starts_with("solid Cube\n"));
    let bin = export(&Scene::startup(), Format::Stl).unwrap();
    assert_eq!(bin.len(), 84 + 50 * 12);
    assert!(!bin.starts_with(b"solid"));
}

// ---------------------------------------------------------------------------
// PLY.

const ASCII_PLY: &str = "ply
format ascii 1.0
comment a quad and a triangle with per-vertex UVs
element vertex 5
property float x
property float y
property float z
property float nx
property float ny
property float nz
property float s
property float t
property uchar red
property uchar green
property uchar blue
element face 2
property list uchar int vertex_indices
element edge 1
property int vertex1
property int vertex2
end_header
0 0 0 0 0 1 0 0 255 0 0
1 0 0 0 0 1 1 0 0 255 0
1 1 0 0 0 1 1 1 0 0 255
0 1 0 0 0 1 0 1 9 9 9
2 2 0 0 0 1 0.5 0.5 9 9 9
4 0 1 2 3
3 1 4 2
0 4
";

fn binary_ply(big_endian: bool) -> Vec<u8> {
    let mut out = format!(
        "ply\nformat binary_{}_endian 1.0\nelement vertex 4\nproperty double x\nproperty double y\nproperty double z\nelement face 1\nproperty list ushort uint vertex_index\nproperty uchar flags\nelement extra 1\nproperty list uchar float stuff\nend_header\n",
        if big_endian { "big" } else { "little" }
    )
    .into_bytes();
    let f64b = |v: f64| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
    for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.5]] {
        for c in p {
            out.extend_from_slice(&f64b(c));
        }
    }
    out.extend_from_slice(&if big_endian { 4u16.to_be_bytes() } else { 4u16.to_le_bytes() });
    for i in [0u32, 1, 2, 3] {
        out.extend_from_slice(&if big_endian { i.to_be_bytes() } else { i.to_le_bytes() });
    }
    out.push(7);
    out.push(2);
    out.extend_from_slice(&[0; 8]);
    out
}

#[test]
fn ply_ascii_fixture() {
    let s = import(ASCII_PLY.as_bytes(), Format::Ply, &no_files).unwrap();
    let m = mesh_of(&s, 0);
    assert_eq!((m.vertex_count(), m.face_count()), (5, 2));
    assert_eq!(m.faces[0].verts, vec![0, 1, 2, 3]);
    assert_eq!(m.faces[1].uvs, vec![Vec2::new(1.0, 0.0), Vec2::new(0.5, 0.5), Vec2::new(1.0, 1.0)]);
    assert_eq!(m.loose_edges, vec![[0, 4]]);
    assert_eq!(m.positions[4], Vec3::new(2.0, 2.0, 0.0));
}

#[test]
fn ply_binary_both_endians() {
    for big in [false, true] {
        let s = import(&binary_ply(big), Format::Ply, &no_files).unwrap();
        let m = mesh_of(&s, 0);
        assert_eq!((m.vertex_count(), m.face_count()), (4, 1));
        assert_eq!(m.positions[3], Vec3::new(0.0, 1.0, 0.5));
        assert_eq!(m.faces[0].verts, vec![0, 1, 2, 3]);
    }
}

#[test]
fn ply_errors() {
    let bad = ASCII_PLY.replace("1 1 0 0 0 1 1 1 0 0 255", "1 1 0 0 0 1 1 q 0 0 255");
    let e = import(bad.as_bytes(), Format::Ply, &no_files).unwrap_err();
    assert_eq!(e.to_string(), "PLY line 24: invalid number \"q\"");
    let bad = ASCII_PLY.replace("3 1 4 2", "3 1 7 2");
    let e = import(bad.as_bytes(), Format::Ply, &no_files).unwrap_err();
    assert_eq!(e.to_string(), "PLY line 28: vertex index 7 out of range (5 vertices)");
    let e = import(b"ply\nformat ascii 1.0\nelement vertex 1\nproperty fancy x\nend_header\n", Format::Ply, &no_files).unwrap_err();
    assert_eq!(e.to_string(), "PLY line 4: unknown property type \"fancy\"");
    let mut short = binary_ply(false);
    short.truncate(short.len() - 80);
    let e = import(&short, Format::Ply, &no_files).unwrap_err();
    assert!(e.to_string().contains("unexpected end of data"), "{e}");
}

#[test]
fn ply_round_trip_binary_and_ascii() {
    for (name, m) in test_primitives() {
        let mut m = m;
        m.loose_edges.push([0, 1]);
        let s = single_mesh_scene(name, m.clone());
        for bytes in [export(&s, Format::Ply).unwrap(), io::ply::export_ascii(&s)] {
            let back = import(&bytes, Format::Ply, &no_files).unwrap();
            let got = mesh_of(&back, 0);
            assert_eq!(got.positions, m.positions, "{name}: exact positions");
            assert_eq!(got.loose_edges, m.loose_edges, "{name}");
            for (a, b) in m.faces.iter().zip(&got.faces) {
                assert_eq!((&a.verts, &a.uvs), (&b.verts, &b.uvs), "{name}");
            }
            assert_eq!(got.face_count(), m.face_count());
        }
    }
    let head = export(&Scene::startup(), Format::Ply).unwrap();
    assert!(head.starts_with(b"ply\nformat binary_little_endian 1.0\n"));
}

// ---------------------------------------------------------------------------
// glTF.

/// A triangle with normals and UVs in one data-URI buffer.
fn triangle_gltf() -> String {
    let mut bin = Vec::new();
    for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    for v in [0.0f32, 1.0, 1.0, 1.0, 0.0, 0.0] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bin);
    format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"name": "Tri", "mesh": 0, "translation": [0, 1, 0]}}],
  "meshes": [{{"name": "TriMesh", "primitives": [{{"attributes": {{"POSITION": 0, "TEXCOORD_0": 1}}}}]}}],
  "buffers": [{{"byteLength": {}, "uri": "data:application/octet-stream;base64,{b64}"}}],
  "bufferViews": [{{"buffer": 0, "byteLength": 36}}, {{"buffer": 0, "byteOffset": 36, "byteLength": 24}}],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}},
    {{"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"}}
  ]
}}"#,
        bin.len()
    )
}

#[test]
fn gltf_minimal_data_uri() {
    let s = import(triangle_gltf().as_bytes(), Format::Gltf, &no_files).unwrap();
    assert_eq!(s.objects.len(), 1);
    assert_eq!(s.objects[0].name, "Tri");
    let m = mesh_of(&s, 0);
    assert_eq!((m.vertex_count(), m.face_count()), (3, 1));
    // glTF +Y is scene +Z, for vertices and for node translations.
    assert!(m.positions[2].approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-6));
    assert!(s.objects[0].transform.location.approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-6));
    assert_eq!(s.objects[0].transform.rotation, Quat::IDENTITY);
    // glTF v runs down from the top; scene v runs up.
    assert_eq!(m.faces[0].uvs, vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)]);
    assert!(s.objects[0].materials.is_empty());
    assert!(s.materials.is_empty());
}

/// The official "Box" sample's structure: a Y-up-to-Z-up matrix root node,
/// a child with the mesh, interleaved NORMAL/POSITION in one strided view,
/// u16 indices and one material. 24 vertices (split per face) weld to 8.
fn box_gltf(external_buffer: bool) -> (String, Vec<u8>) {
    let mut normals = Vec::new();
    let mut positions = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (-Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (-Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (-Vec3::Z, Vec3::Y, Vec3::X),
    ];
    for (n, u, v) in faces {
        let base = positions.len() as u16;
        for (a, b) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
            positions.push(n * 0.5 + u * a + v * b);
            normals.push(n);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut bin = Vec::new();
    for i in &indices {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let interleaved_start = bin.len();
    // NORMAL block then POSITION block, both read through a 12-byte stride.
    for v in normals.iter().chain(&positions) {
        for c in v.to_array() {
            bin.extend_from_slice(&c.to_le_bytes());
        }
    }
    let uri = if external_buffer { "Box0.bin".to_string() } else { format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bin)) };
    let json = format!(
        r#"{{
  "asset": {{"generator": "COLLADA2GLTF", "version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [
    {{"children": [1], "matrix": [1,0,0,0, 0,0,-1,0, 0,1,0,0, 0,0,0,1]}},
    {{"mesh": 0}}
  ],
  "meshes": [{{"name": "Mesh", "primitives": [{{"attributes": {{"NORMAL": 1, "POSITION": 2}}, "indices": 0, "mode": 4, "material": 0}}]}}],
  "accessors": [
    {{"bufferView": 0, "byteOffset": 0, "componentType": 5123, "count": 36, "max": [23], "min": [0], "type": "SCALAR"}},
    {{"bufferView": 1, "byteOffset": 0, "componentType": 5126, "count": 24, "max": [1,1,1], "min": [-1,-1,-1], "type": "VEC3"}},
    {{"bufferView": 1, "byteOffset": 288, "componentType": 5126, "count": 24, "max": [0.5,0.5,0.5], "min": [-0.5,-0.5,-0.5], "type": "VEC3"}}
  ],
  "materials": [{{"pbrMetallicRoughness": {{"baseColorFactor": [0.8, 0.0, 0.0, 1.0], "metallicFactor": 0.0}}, "name": "Red"}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 72, "target": 34963}},
    {{"buffer": 0, "byteOffset": {interleaved_start}, "byteLength": 576, "byteStride": 12, "target": 34962}}
  ],
  "buffers": [{{"byteLength": {}, "uri": "{uri}"}}]
}}"#,
        bin.len()
    );
    (json, bin)
}

#[test]
fn gltf_box_sample_structure() {
    for external in [false, true] {
        let (json, bin) = box_gltf(external);
        let files = [("Box0.bin", bin)];
        let s = import(json.as_bytes(), Format::Gltf, &resolver(&files)).unwrap();
        assert_eq!(s.objects.len(), 2, "root empty + mesh child");
        assert!(matches!(s.objects[0].data, ObjectData::Empty));
        assert_eq!(s.objects[1].parent, Some(0));
        let m = mesh_of(&s, 1);
        assert_eq!((m.vertex_count(), m.face_count()), (8, 12), "welded across normal seams");
        assert!(m.half_edges().is_closed_manifold());
        assert!(m.faces.iter().all(|f| !f.smooth), "flat normals stay flat");
        assert!(m.volume() > 0.99 && m.volume() < 1.01);
        let mat = &s.materials[s.objects[1].materials[0]];
        assert_eq!(mat.name, "Red");
        assert_eq!(mat.base_color, [0.8, 0.0, 0.0, 1.0]);
        assert_eq!((mat.metallic, mat.roughness), (0.0, 1.0));
        // The unit box stays a unit box in world space.
        let mut b = sex_3d::math::Aabb::EMPTY;
        for &p in &m.positions {
            b.grow(world_point(&s, 1, p));
        }
        assert!(b.min.approx_eq(Vec3::splat(-0.5), 1e-5) && b.max.approx_eq(Vec3::splat(0.5), 1e-5), "{b:?}");
    }
    // Without the resolver the external buffer is a missing file.
    let (json, _) = box_gltf(true);
    let e = import(json.as_bytes(), Format::Gltf, &no_files).unwrap_err();
    assert_eq!(e, IoError::MissingFile { format: Format::Gltf, path: "Box0.bin".into() });
}

/// Strips, fans, absent indices, normalized u8 texcoords, sparse positions,
/// lines and a node with both a mesh and a camera.
#[test]
fn gltf_modes_component_types_and_sparse() {
    let mut bin = Vec::new();
    // 0: 4 positions (a unit quad in glTF XY), 48 bytes.
    for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    // 48: normalized u8 UVs, 4 x 2 bytes.
    bin.extend_from_slice(&[0, 255, 255, 255, 0, 0, 255, 0]);
    // 56: sparse indices (u8, padded) then values: move vertex 3 up to z = 1.
    bin.extend_from_slice(&[3, 0, 0, 0]);
    for v in [1.0f32, 1.0, 1.0] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    // 72: fan indices u8: 0 1 3 2 (two triangles), padded.
    bin.extend_from_slice(&[0, 1, 3, 2]);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bin);
    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "nodes": [{{"name": "Shapes", "mesh": 0, "camera": 0}}],
  "cameras": [{{"type": "perspective", "perspective": {{"yfov": 0.8, "znear": 0.05}}}}],
  "meshes": [{{"primitives": [
    {{"attributes": {{"POSITION": 0, "TEXCOORD_0": 1}}, "mode": 5}},
    {{"attributes": {{"POSITION": 2}}, "indices": 3, "mode": 6}},
    {{"attributes": {{"POSITION": 0}}, "mode": 3}}
  ]}}],
  "buffers": [{{"byteLength": {}, "uri": "data:application/gltf-buffer;base64,{b64}"}}],
  "bufferViews": [
    {{"buffer": 0, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": 48, "byteLength": 8}},
    {{"buffer": 0, "byteOffset": 56, "byteLength": 16}},
    {{"buffer": 0, "byteOffset": 72, "byteLength": 4}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3"}},
    {{"bufferView": 1, "componentType": 5121, "normalized": true, "count": 4, "type": "VEC2"}},
    {{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3",
      "sparse": {{"count": 1, "indices": {{"bufferView": 2, "componentType": 5121}}, "values": {{"bufferView": 2, "byteOffset": 4}}}}}},
    {{"bufferView": 3, "componentType": 5121, "count": 4, "type": "SCALAR"}}
  ]
}}"#,
        bin.len()
    );
    let s = import(json.as_bytes(), Format::Gltf, &no_files).unwrap();
    // No "scenes": every parentless node is a root.
    assert_eq!(s.objects.len(), 2);
    assert_eq!(s.objects[1].parent, Some(0));
    match s.objects[1].data {
        ObjectData::Camera(c) => assert_eq!((c.fov_y, c.near), (0.8, 0.05)),
        ref d => panic!("{d:?}"),
    }
    let m = mesh_of(&s, 0);
    // Strip: 2 triangles; fan over the sparse copy: 2 triangles, of which the
    // vertex moved by the sparse accessor is new: 4 + 1 positions.
    assert_eq!(m.face_count(), 4);
    assert_eq!(m.vertex_count(), 5);
    assert!(m.positions.iter().any(|p| p.approx_eq(Vec3::new(1.0, -1.0, 1.0), 1e-6)), "sparse value (1,1,1) in Y-up");
    // Strip triangles keep a consistent winding (both face glTF +Z = scene -Y).
    for f in 0..2 {
        assert!(m.face_normal(f).approx_eq(-Vec3::Y, 1e-5), "{:?}", m.face_normal(f));
    }
    // u8 normalized UVs: 255 -> 1.0, then flipped.
    assert_eq!(m.faces[0].uvs[0], Vec2::new(0.0, 0.0));
    assert_eq!(m.faces[0].uvs[1], Vec2::new(1.0, 0.0));
    // The line strip 0-1-2-3 runs along edges the strip triangles already
    // have, so after welding none of its segments is loose.
    assert!(m.loose_edges.is_empty(), "{:?}", m.loose_edges);
}

#[test]
fn gltf_lights_cameras_and_units() {
    let json = r#"{
  "asset": {"version": "2.0"},
  "extensionsUsed": ["KHR_lights_punctual"],
  "extensions": {"KHR_lights_punctual": {"lights": [
    {"type": "point", "intensity": 683, "color": [1, 0.5, 0.25]},
    {"type": "spot", "intensity": 10, "spot": {"innerConeAngle": 0.2, "outerConeAngle": 0.4}},
    {"type": "directional", "intensity": 3}
  ]}},
  "scenes": [{"nodes": [0, 1, 2, 3, 4]}],
  "nodes": [
    {"name": "P", "extensions": {"KHR_lights_punctual": {"light": 0}}},
    {"name": "S", "extensions": {"KHR_lights_punctual": {"light": 1}}},
    {"name": "D", "extensions": {"KHR_lights_punctual": {"light": 2}}, "children": [4]},
    {"name": "Cam", "camera": 0, "translation": [0, 0, 5]},
    {"name": "UnderSun", "translation": [0, 0, -2]}
  ],
  "cameras": [{"type": "orthographic", "orthographic": {"xmag": 2, "ymag": 1.5, "znear": 0.1, "zfar": 50}}]
}"#;
    let s = import(json.as_bytes(), Format::Gltf, &no_files).unwrap();
    // Node 4 is listed as a root but is also a child of node 2: it appears once.
    assert_eq!(s.objects.len(), 5);
    let light = |i: usize| match s.objects[i].data {
        ObjectData::Light(l) => l,
        ref d => panic!("{d:?}"),
    };
    let p = light(0);
    assert_eq!(p.kind, LightKind::Point);
    assert!((p.power - 4.0 * std::f32::consts::PI).abs() < 1e-4, "683 cd = 4π W");
    assert_eq!(p.color, Vec3::new(1.0, 0.5, 0.25));
    match light(1).kind {
        LightKind::Spot { angle, blend } => assert!((angle - 0.8).abs() < 1e-6 && (blend - 0.5).abs() < 1e-6),
        k => panic!("{k:?}"),
    }
    assert_eq!((light(2).kind, light(2).power), (LightKind::Sun, 3.0));
    // A glTF light with no rotation shines down glTF -Z, which is scene +Y.
    let dir = s.world_matrix(2).transform_vector(-Vec3::Z).normalize();
    assert!(dir.approx_eq(Vec3::Y, 1e-5), "{dir:?}");
    // The sun's child compensates for the sun's axis turn: glTF (0,0,-2) is scene (0,2,0).
    // Objects follow node order: P, S, D, Cam, UnderSun.
    assert_eq!(s.objects[4].parent, Some(2));
    let child = world_point(&s, 4, Vec3::ZERO);
    assert!(child.approx_eq(Vec3::new(0.0, 2.0, 0.0), 1e-5), "{child:?}");
    // Camera at glTF (0,0,5) = scene (0,-5,0), looking at the origin along +Y, Z up.
    let cam = s.active_camera.expect("active camera");
    assert_eq!(s.objects[cam].name, "Cam");
    let m = s.world_matrix(cam);
    assert!(m.transform_point(Vec3::ZERO).approx_eq(Vec3::new(0.0, -5.0, 0.0), 1e-5));
    assert!(m.transform_vector(-Vec3::Z).approx_eq(Vec3::Y, 1e-5));
    assert!(m.transform_vector(Vec3::Y).approx_eq(Vec3::Z, 1e-5));
    match s.objects[cam].data {
        ObjectData::Camera(c) => assert_eq!((c.ortho_scale, c.near, c.far), (Some(3.0), 0.1, 50.0)),
        ref d => panic!("{d:?}"),
    }
}

#[test]
fn gltf_errors() {
    let e = import(b"{\n  \"asset\": {\"version\": \"2.0\"},\n  \"nodes\": [,]\n}", Format::Gltf, &no_files).unwrap_err();
    assert_eq!(e.line(), Some(3), "{e}");
    let e = import(br#"{"asset": {"version": "1.0"}}"#, Format::Gltf, &no_files).unwrap_err();
    assert!(matches!(e, IoError::Unsupported { .. }), "{e}");
    let e = import(br#"{"asset": {"version": "2.0"}, "extensionsRequired": ["KHR_draco_mesh_compression"]}"#, Format::Gltf, &no_files).unwrap_err();
    assert_eq!(e.to_string(), "glTF: unsupported: required extension KHR_draco_mesh_compression");
    let bad = triangle_gltf().replace("\"count\": 3, \"type\": \"VEC2\"", "\"count\": 30, \"type\": \"VEC2\"");
    let e = import(bad.as_bytes(), Format::Gltf, &no_files).unwrap_err();
    assert!(e.to_string().contains("accessor 1: reads"), "{e}");
    let bad = triangle_gltf().replace("\"mesh\": 0", "\"mesh\": 4");
    let e = import(bad.as_bytes(), Format::Gltf, &no_files).unwrap_err();
    assert_eq!(e.to_string(), "glTF: node 0: mesh 4 does not exist");
    let e = import(b"glTF\x01\0\0\0\x0c\0\0\0", Format::Glb, &no_files).unwrap_err();
    assert_eq!(e.to_string(), "GLB: only GLB version 2 is supported");
}

/// A scene exercising everything glTF export writes.
fn rich_scene() -> Scene {
    let mut s = Scene::startup();
    let mut tex = Image::new("tiles", 4, 2);
    for (i, p) in tex.pixels.iter_mut().enumerate() {
        *p = [i as f32 / 8.0, 1.0 - i as f32 / 8.0, 0.5, 1.0];
    }
    s.images.push(tex);
    let glass = s.add_material(Material { name: "Glass".into(), transmission: 1.0, ior: 1.33, roughness: 0.05, double_sided: true, base_color: [0.9, 0.95, 1.0, 0.4], ..Default::default() });
    let lamp = s.add_material(Material { name: "Lamp".into(), emission: Vec3::new(1.0, 0.8, 0.6), emission_strength: 5.0, base_color_texture: Some(0), metallic: 0.7, ..Default::default() });
    // A sphere with two slots, parented under the cube, rotated and scaled.
    let mut sphere = primitives::uv_sphere(12, 6, 0.5);
    for (i, f) in sphere.faces.iter_mut().enumerate() {
        f.material = (i % 2) as u16;
    }
    let sp = s.add_mesh_object("Sphere", sphere);
    s.objects[sp].materials = vec![glass, lamp];
    s.objects[sp].parent = Some(0);
    s.objects[sp].transform.location = Vec3::new(0.0, 3.0, 1.0);
    s.objects[sp].transform.rotation = Quat::from_euler_xyz(0.4, 0.0, 1.2);
    s.objects[sp].transform.scale = Vec3::new(1.0, 2.0, 0.5);
    // A second user of the cube mesh, and an empty with a camera child.
    let cube_mesh = match s.objects[0].data {
        ObjectData::Mesh(m) => m,
        _ => unreachable!(),
    };
    let mut twin = Object::new("Twin", ObjectData::Mesh(cube_mesh));
    twin.materials = s.objects[0].materials.clone();
    twin.transform.location = Vec3::new(-4.0, 0.0, 0.0);
    s.add_object(twin);
    let rig = s.add_object(Object::new("Rig", ObjectData::Empty));
    s.objects[rig].transform.rotation = Quat::from_axis_angle(Vec3::Z, 0.5);
    let mut cam = Object::new("Ortho", ObjectData::Camera(Camera { ortho_scale: Some(6.0), ..Default::default() }));
    cam.parent = Some(rig);
    cam.transform.location = Vec3::new(0.0, -8.0, 2.0);
    cam.transform.rotation = Quat::from_axis_angle(Vec3::X, 1.3);
    let cam = s.add_object(cam);
    // A light under the camera (child of a turned frame).
    let mut spot = Object::new("Spot", ObjectData::Light(Light { kind: LightKind::Spot { angle: 1.0, blend: 0.25 }, power: 250.0, color: Vec3::new(1.0, 0.9, 0.8), radius: 0.0 }));
    spot.parent = Some(cam);
    spot.transform.location = Vec3::new(0.5, 0.0, 0.0);
    spot.transform.rotation = Quat::from_axis_angle(Vec3::Y, 0.3);
    s.add_object(spot);
    let mut sun = Object::new("Sun", ObjectData::Light(Light { kind: LightKind::Sun, power: 4.0, ..Default::default() }));
    sun.transform.rotation = Quat::from_euler_xyz(0.3, 0.2, 0.1);
    s.add_object(sun);
    // Wire-only geometry.
    let mut wire = Mesh::new();
    for p in [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0)] {
        wire.add_vertex(p);
    }
    wire.loose_edges = vec![[0, 1], [1, 2]];
    s.add_mesh_object("Wire", wire);
    s
}

fn check_gltf_round_trip(orig: &Scene, back: &Scene) {
    assert_eq!(back.objects.len(), orig.objects.len());
    for (i, (a, b)) in orig.objects.iter().zip(&back.objects).enumerate() {
        assert_eq!(a.name, b.name);
        assert_eq!(a.parent, b.parent, "{}", a.name);
        assert!(a.transform.location.approx_eq(b.transform.location, 1e-5), "{}: {:?} vs {:?}", a.name, a.transform.location, b.transform.location);
        assert!(quat_close(a.transform.rotation, b.transform.rotation), "{}: {:?} vs {:?}", a.name, a.transform.rotation, b.transform.rotation);
        assert!(a.transform.scale.approx_eq(b.transform.scale, 1e-5), "{}", a.name);
        // World placement and orientation agree.
        let (wa, wb) = (orig.world_matrix(i), back.world_matrix(i));
        for v in [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z] {
            assert!(wa.transform_point(v).approx_eq(wb.transform_point(v), 1e-4), "{}", a.name);
        }
        match (&a.data, &b.data) {
            (ObjectData::Mesh(ma), ObjectData::Mesh(_)) => {
                let (ma, mb) = (&orig.meshes[*ma], mesh_of(back, i));
                same_point_set(&ma.positions, &mb.positions, 1e-5);
                assert_eq!(mb.face_count(), ma.triangulate().len(), "{}", a.name);
                assert_eq!(mb.loose_edges.len(), ma.loose_edges.len(), "{}", a.name);
                if ma.has_uvs() {
                    assert_uvs_match(ma, mb);
                }
                let slot_names = |s: &Scene, o: &Object| o.materials.iter().map(|&m| s.materials[m].name.clone()).collect::<Vec<_>>();
                assert_eq!(slot_names(orig, a), slot_names(back, b), "{}", a.name);
                let smooth = |m: &Mesh| m.faces.iter().filter(|f| f.smooth).count() > 0;
                assert_eq!(smooth(ma), smooth(mb), "{}", a.name);
            }
            (ObjectData::Camera(ca), ObjectData::Camera(cb)) => {
                assert!((ca.fov_y - cb.fov_y).abs() < 1e-6 && ca.ortho_scale == cb.ortho_scale && ca.near == cb.near && ca.far == cb.far, "{ca:?} {cb:?}");
            }
            (ObjectData::Light(la), ObjectData::Light(lb)) => {
                assert_eq!(std::mem::discriminant(&la.kind), std::mem::discriminant(&lb.kind));
                assert!((la.power - lb.power).abs() < la.power * 1e-5, "{la:?} {lb:?}");
                assert_eq!(la.color, lb.color);
                if let (LightKind::Spot { angle: a1, blend: b1 }, LightKind::Spot { angle: a2, blend: b2 }) = (la.kind, lb.kind) {
                    assert!((a1 - a2).abs() < 1e-6 && (b1 - b2).abs() < 1e-5);
                }
            }
            (ObjectData::Empty, ObjectData::Empty) => {}
            (x, y) => panic!("{}: {x:?} became {y:?}", a.name),
        }
    }
    assert_eq!(back.active_camera.map(|c| &back.objects[c].name), orig.objects.iter().position(|o| matches!(o.data, ObjectData::Camera(_))).map(|c| &orig.objects[c].name));
}

#[test]
fn gltf_and_glb_round_trip_rich_scene() {
    let s = rich_scene();
    for format in [Format::Gltf, Format::Glb] {
        let bytes = export(&s, format).unwrap();
        assert_eq!(Format::sniff(&bytes), Some(format));
        let back = import(&bytes, format, &no_files).unwrap();
        check_gltf_round_trip(&s, &back);
        // Shared mesh stays shared.
        let (a, b) = (back.find_object("Cube").unwrap(), back.find_object("Twin").unwrap());
        assert_eq!(back.objects[a].data, back.objects[b].data);
        // Materials.
        let get = |n: &str| back.materials.iter().find(|m| m.name == n).unwrap_or_else(|| panic!("{n}"));
        let glass = get("Glass");
        assert_eq!((glass.transmission, glass.ior, glass.double_sided), (1.0, 1.33, true));
        assert_eq!(glass.base_color, [0.9, 0.95, 1.0, 0.4]);
        assert!((glass.roughness - 0.05).abs() < 1e-7 && glass.metallic == 0.0);
        let lamp = get("Lamp");
        assert!((lamp.emission * lamp.emission_strength).approx_eq(Vec3::new(5.0, 4.0, 3.0), 1e-5));
        assert_eq!(lamp.metallic, 0.7);
        let img = &back.images[lamp.base_color_texture.expect("texture")];
        assert_eq!((img.width, img.height), (4, 2));
        for (a, b) in img.pixels.iter().zip(&s.images[0].pixels) {
            for k in 0..4 {
                assert!((a[k] - b[k]).abs() < 0.01, "{a:?} vs {b:?}");
            }
        }
        // Per-face slots survive: half the sphere's triangles are glass.
        let sphere = mesh_of(&back, back.find_object("Sphere").unwrap());
        let glass_faces = sphere.faces.iter().filter(|f| f.material == 0).count();
        assert!(glass_faces > 0 && glass_faces < sphere.face_count());
    }
}

#[test]
fn gltf_round_trip_primitives_and_startup() {
    for (name, m) in test_primitives() {
        let s = single_mesh_scene(name, m.clone());
        for format in [Format::Gltf, Format::Glb] {
            let back = import(&export(&s, format).unwrap(), format, &no_files).unwrap();
            check_gltf_round_trip(&s, &back);
            let got = mesh_of(&back, 0);
            if name != "grid" {
                assert!(got.half_edges().is_closed_manifold(), "{name}: welded closed");
            }
            assert!((got.volume() - m.volume()).abs() < 1e-4, "{name}");
        }
    }
    let s = Scene::startup();
    let back = import(&export(&s, Format::Glb).unwrap(), Format::Glb, &no_files).unwrap();
    check_gltf_round_trip(&s, &back);
    // The startup camera still looks at the cube.
    let cam = back.active_camera.unwrap();
    let m = back.world_matrix(cam);
    let fwd = m.transform_vector(-Vec3::Z).normalize();
    assert!(fwd.approx_eq((Vec3::ZERO - m.transform_point(Vec3::ZERO)).normalize(), 1e-4));
    // Default point light: 1000 W.
    let light = back.find_object("Light").unwrap();
    match back.objects[light].data {
        ObjectData::Light(l) => assert!((l.power - 1000.0).abs() < 0.01),
        ref d => panic!("{d:?}"),
    }
}

#[test]
fn gltf_glb_built_by_hand() {
    // Turn the data-URI triangle into a GLB with a BIN chunk.
    let json = triangle_gltf();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let uri = v["buffers"][0]["uri"].as_str().unwrap();
    let bin = base64::engine::general_purpose::STANDARD.decode(uri.split_once(',').unwrap().1).unwrap();
    let mut v = v.clone();
    v["buffers"][0].as_object_mut().unwrap().remove("uri");
    let mut j = serde_json::to_vec(&v).unwrap();
    while j.len() % 4 != 0 {
        j.push(b' ');
    }
    let mut b = bin.clone();
    while b.len() % 4 != 0 {
        b.push(0);
    }
    let total = 12 + 8 + j.len() + 8 + b.len();
    let mut glb = b"glTF".to_vec();
    for x in [2u32, total as u32, j.len() as u32, 0x4E4F534A] {
        glb.extend_from_slice(&x.to_le_bytes());
    }
    glb.extend_from_slice(&j);
    for x in [b.len() as u32, 0x004E4942] {
        glb.extend_from_slice(&x.to_le_bytes());
    }
    glb.extend_from_slice(&b);
    let s = import(&glb, Format::Glb, &no_files).unwrap();
    assert_eq!(mesh_of(&s, 0).face_count(), 1);
    // .gltf and .glb are read by the same entry point, whichever is named.
    let s2 = import(&glb, Format::Gltf, &no_files).unwrap();
    assert_eq!(s2.objects.len(), 1);
}

// ---------------------------------------------------------------------------
// API behaviour shared by all formats.

#[test]
fn import_into_appends_to_existing_scene() {
    let mut s = Scene::startup();
    let n = s.objects.len();
    let added = import_into(&mut s, OBJ.as_bytes(), Format::Obj, &no_files).unwrap();
    assert_eq!(added, vec![n, n + 1]);
    let added = import_into(&mut s, ASCII_STL.as_bytes(), Format::Stl, &no_files).unwrap();
    assert_eq!(added, vec![n + 2]);
    // glTF parents index into the scene, not the file.
    let (json, _) = box_gltf(false);
    let added = import_into(&mut s, json.as_bytes(), Format::Gltf, &no_files).unwrap();
    assert_eq!(s.objects[added[1]].parent, Some(added[0]));
    // Names stay unique.
    let gltf = import_into(&mut s, triangle_gltf().as_bytes(), Format::Gltf, &no_files).unwrap();
    let again = import_into(&mut s, triangle_gltf().as_bytes(), Format::Gltf, &no_files).unwrap();
    assert_eq!(s.objects[gltf[0]].name, "Tri");
    assert_eq!(s.objects[again[0]].name, "Tri.001");
}

#[test]
fn export_files_names_main_file_first() {
    let s = Scene::startup();
    for f in [Format::Stl, Format::Ply, Format::Gltf, Format::Glb] {
        let files = export_files(&s, f, "out").unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, format!("out.{}", f.extension()));
    }
}
