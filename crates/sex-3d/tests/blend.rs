// SPDX-License-Identifier: GPL-3.0-only
//! `.blend` import tests against synthetic files from [`blendgen`].
//!
//! Real Blender-saved fixtures are still to be added; these files follow the
//! documented format closely (header, BHeads, SDNA with padding members) for
//! every mesh layout generation, both byte orders and both pointer sizes.
mod blendgen;

use blendgen::{cube_mat, cube_smooth, cube_uv, scene_file, Cfg, Layout, CUBE_FACES, CUBE_VERTS};
use sex_3d::io::blend::import_blend;
use sex_3d::scene::{LightKind, ObjectData, Scene};
use sex_3d::Vec3;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

const ALL: [Cfg; 12] = [
    Cfg { ptr: 4, big: true, layout: Layout::Legacy27 },
    Cfg { ptr: 8, big: false, layout: Layout::Legacy27 },
    Cfg { ptr: 8, big: false, layout: Layout::Legacy30 },
    Cfg { ptr: 4, big: false, layout: Layout::Legacy30 },
    Cfg { ptr: 8, big: true, layout: Layout::Legacy30 },
    Cfg { ptr: 8, big: false, layout: Layout::Attr36 },
    Cfg { ptr: 4, big: true, layout: Layout::Attr36 },
    Cfg { ptr: 8, big: false, layout: Layout::Attr42 },
    Cfg { ptr: 8, big: true, layout: Layout::Attr42 },
    Cfg { ptr: 4, big: false, layout: Layout::Attr42 },
    Cfg { ptr: 8, big: false, layout: Layout::Attr50 },
    Cfg { ptr: 8, big: true, layout: Layout::Attr50 },
];

/// Everything the synthetic scene should import as, whatever the layout.
fn check_scene(s: &Scene, what: &str) {
    let names = blendgen::names(s);
    assert_eq!(s.objects.len(), 8, "{what}");
    for n in ["Cube", "Child", "Instance", "Camera", "OrthoCam", "Spot", "Area", "Sun"] {
        assert!(names.contains_key(n), "{what}: missing {n}");
    }
    assert_eq!(s.name, "Main", "{what}");
    assert_eq!((s.frame_start, s.frame_end), (10, 120), "{what}");
    assert!(approx(s.fps, 30.0), "{what}");

    // Mesh, shared by Cube and Instance.
    assert_eq!(s.meshes.len(), 1, "{what}");
    assert_eq!(s.mesh_names[0], "Cube", "{what}");
    let cube = names["Cube"];
    let inst = names["Instance"];
    assert_eq!(s.objects[cube].data, ObjectData::Mesh(0), "{what}");
    assert_eq!(s.objects[inst].data, ObjectData::Mesh(0), "{what}");
    let m = &s.meshes[0];
    assert_eq!(m.vertex_count(), 9, "{what}");
    assert_eq!(m.face_count(), 6, "{what}");
    for (p, e) in m.positions.iter().zip(CUBE_VERTS) {
        assert_eq!(p.to_array(), e, "{what}");
    }
    for (fi, f) in m.faces.iter().enumerate() {
        assert_eq!(f.verts, CUBE_FACES[fi], "{what}: face {fi}");
        let uvs: Vec<[f32; 2]> = f.uvs.iter().map(|u| [u.x, u.y]).collect();
        assert_eq!(uvs, (0..4).map(|k| cube_uv(fi, k)).collect::<Vec<_>>(), "{what}: uv {fi}");
        assert_eq!(f.material as i64, cube_mat(fi), "{what}: material {fi}");
        assert_eq!(f.smooth, cube_smooth(fi), "{what}: smooth {fi}");
    }
    assert_eq!(m.loose_edges, vec![[8, 0]], "{what}");

    // Cube transform: T(1,2,3) R_z(90°) S(2).
    let t = s.objects[cube].transform;
    assert!(t.location.approx_eq(Vec3::new(1.0, 2.0, 3.0), 1e-5), "{what}: {t:?}");
    assert!(t.scale.approx_eq(Vec3::splat(2.0), 1e-5), "{what}: {t:?}");
    assert!(t.rotation.rotate(Vec3::X).approx_eq(Vec3::Y, 1e-5), "{what}: {t:?}");

    // Child keeps its world position through the parent inverse.
    let child = names["Child"];
    assert_eq!(s.objects[child].parent, Some(cube), "{what}");
    let wp = s.world_matrix(child).transform_point(Vec3::ZERO);
    assert!(wp.approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-4), "{what}: child at {wp:?}");
    assert_eq!(s.objects[inst].parent, None);

    // Materials and slots (Cube's slot 1 is linked to the object).
    let mat = |n: &str| s.materials.iter().position(|m| m.name == n).unwrap_or_else(|| panic!("{what}: no material {n}"));
    let (red, blue, green) = (mat("Red"), mat("Blue"), mat("Green"));
    let r = &s.materials[red];
    assert_eq!(r.base_color, [0.8, 0.1, 0.1, 1.0], "{what}");
    assert!(approx(r.metallic, 0.5) && approx(r.roughness, 0.3), "{what}");
    let b = &s.materials[blue];
    assert_eq!(b.base_color, [0.1, 0.2, 0.9, 1.0], "{what}");
    assert!(approx(b.metallic, 1.0) && approx(b.roughness, 0.2), "{what}");
    assert_eq!(s.materials[green].base_color, [0.1, 0.8, 0.1, 1.0], "{what}");
    assert_eq!(s.objects[cube].materials, vec![red, green], "{what}");
    assert_eq!(s.objects[inst].materials, vec![red, blue], "{what}");

    // Cameras.
    assert_eq!(s.active_camera, Some(names["Camera"]), "{what}");
    let ObjectData::Camera(c) = s.objects[names["Camera"]].data else { panic!("{what}: Camera is not a camera") };
    let want = 2.0 * (18.0f32 / 50.0 * 1080.0 / 1920.0).atan();
    assert!(approx(c.fov_y, want), "{what}: fov {} vs {want}", c.fov_y);
    assert_eq!(c.ortho_scale, None);
    assert!(approx(c.near, 0.1) && approx(c.far, 100.0), "{what}");
    assert!(s.objects[names["Camera"]].transform.location.approx_eq(Vec3::new(7.0, -7.0, 5.0), 1e-5));
    let ObjectData::Camera(o) = s.objects[names["OrthoCam"]].data else { panic!("{what}: OrthoCam") };
    assert!(approx(o.ortho_scale.unwrap(), 10.0 * 1080.0 / 1920.0), "{what}");
    assert!(approx(o.near, 0.5) && approx(o.far, 50.0), "{what}");

    // Lights.
    let light = |n: &str| match s.objects[names[n]].data {
        ObjectData::Light(l) => l,
        ref d => panic!("{what}: {n} is {d:?}"),
    };
    let spot = light("Spot");
    match spot.kind {
        LightKind::Spot { angle, blend } => assert!(approx(angle, 0.785398) && approx(blend, 0.2), "{what}"),
        k => panic!("{what}: spot is {k:?}"),
    }
    assert!(approx(spot.power, 1000.0) && approx(spot.radius, 0.25), "{what}");
    assert!(spot.color.approx_eq(Vec3::new(1.0, 0.5, 0.25), 1e-6), "{what}");
    let area = light("Area");
    assert_eq!(area.kind, LightKind::Area { size: 2.0 }, "{what}");
    assert!(approx(area.power, 500.0), "{what}");
    let sun = light("Sun");
    assert_eq!(sun.kind, LightKind::Sun, "{what}");
    assert!(approx(sun.power, 3.0) && approx(sun.radius, 0.01), "{what}");
}

#[test]
fn every_layout_endianness_and_pointer_size() {
    for c in ALL {
        let bytes = scene_file(c);
        let s = import_blend(&bytes).unwrap_or_else(|e| panic!("{c:?}: {e}"));
        check_scene(&s, &format!("{c:?}"));
    }
}

#[test]
fn headers_match_blender_conventions() {
    let classic = scene_file(Cfg { ptr: 4, big: true, layout: Layout::Legacy27 });
    assert_eq!(&classic[..12], b"BLENDER_V279");
    let classic = scene_file(Cfg { ptr: 8, big: false, layout: Layout::Attr42 });
    assert_eq!(&classic[..12], b"BLENDER-v402");
    let large = scene_file(Cfg { ptr: 8, big: false, layout: Layout::Attr50 });
    assert_eq!(&large[..17], b"BLENDER17-01v0500");
}

#[test]
fn gzip_compressed() {
    let c = Cfg { ptr: 8, big: false, layout: Layout::Legacy30 };
    let gz = blendgen::gzip(&scene_file(c));
    assert_eq!(&gz[..2], &[0x1f, 0x8b]);
    check_scene(&import_blend(&gz).unwrap(), "gzip");
}

#[test]
fn zstd_compressed() {
    let c = Cfg { ptr: 8, big: false, layout: Layout::Attr42 };
    let zs = blendgen::zstd(&scene_file(c));
    assert_eq!(&zs[..4], &[0x28, 0xb5, 0x2f, 0xfd]);
    check_scene(&import_blend(&zs).unwrap(), "zstd");
}

#[test]
fn zstd_raw_frames_with_seek_table() {
    let c = Cfg { ptr: 8, big: false, layout: Layout::Attr50 };
    let zs = blendgen::zstd_raw_frames(&scene_file(c));
    check_scene(&import_blend(&zs).unwrap(), "zstd raw frames");
}

#[test]
fn garbage_is_rejected() {
    let cases: Vec<Vec<u8>> = vec![
        vec![],
        b"BLENDER".to_vec(),
        b"BLENDER-v402".to_vec(),
        b"BLENDER?v402xxxxxxxxxxxxxxxxxxxx".to_vec(),
        b"BLENDER17-09v0500".to_vec(),
        b"BLENDER99-01v0500".to_vec(),
        b"not a blend file at all".to_vec(),
        vec![0x1f, 0x8b, 0, 0, 0, 0],
        vec![0x28, 0xb5, 0x2f, 0xfd, 0xff, 0xff, 0xff],
        blendgen::gzip(b"hello, not blender"),
        blendgen::zstd_raw_frames(b"hello, not blender either"),
        (0..4096u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect(),
    ];
    for c in cases {
        assert!(import_blend(&c).is_err(), "{:?}", &c[..c.len().min(20)]);
    }
}

#[test]
fn every_truncation_is_an_error() {
    for c in [Cfg { ptr: 4, big: true, layout: Layout::Legacy30 }, Cfg { ptr: 8, big: false, layout: Layout::Attr50 }] {
        let full = scene_file(c);
        for len in 0..full.len() {
            assert!(import_blend(&full[..len]).is_err(), "{c:?} truncated to {len}");
        }
    }
    let full = scene_file(Cfg { ptr: 8, big: false, layout: Layout::Attr42 });
    // The raw-frame variant ends in a 16-byte skippable seek-table frame;
    // cutting exactly that off leaves a complete, valid stream.
    let raw = blendgen::zstd_raw_frames(&full);
    let seek_table = raw.len() - 16;
    for packed in [blendgen::gzip(&full), blendgen::zstd(&full), raw] {
        for len in (0..packed.len()).step_by(7).chain(packed.len().saturating_sub(20)..packed.len()) {
            let r = import_blend(&packed[..len]);
            if packed[0] == 0x28 && len == seek_table && packed.len() == seek_table + 16 {
                assert!(r.is_ok());
            } else {
                assert!(r.is_err(), "compressed truncated to {len} of {}", packed.len());
            }
        }
    }
}

/// Small deterministic PRNG (xorshift64*).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[test]
fn corrupted_files_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for c in ALL {
        let full = scene_file(c);
        for i in 0..400 {
            let mut b = full.clone();
            let flips = 1 + (rng.next() % 3) as usize;
            for _ in 0..flips {
                let at = (rng.next() % b.len() as u64) as usize;
                b[at] ^= 1 << (rng.next() % 8);
            }
            if i % 4 == 0 {
                // Also smash a whole word, the way a bad length or pointer looks.
                let at = (rng.next() % (b.len() as u64 - 8)) as usize;
                b[at..at + 8].copy_from_slice(&rng.next().to_le_bytes());
            }
            let _ = import_blend(&b);
        }
    }
    // Corrupt compressed payloads too.
    let full = scene_file(Cfg { ptr: 8, big: false, layout: Layout::Attr42 });
    for packed in [blendgen::gzip(&full), blendgen::zstd(&full)] {
        for _ in 0..200 {
            let mut b = packed.clone();
            let at = 4 + (rng.next() % (b.len() as u64 - 4)) as usize;
            b[at] ^= 1 << (rng.next() % 8);
            let _ = import_blend(&b);
        }
    }
}

#[test]
fn rotation_modes_agree() {
    // Every layout writes the cube's 90° Z rotation in a different rotation
    // mode (quaternion, axis-angle, XYZ and ZYX Euler).
    let rots: Vec<_> = ALL.iter().map(|&c| {
        let s = import_blend(&scene_file(c)).unwrap();
        s.objects[s.find_object("Cube").unwrap()].transform.matrix()
    }).collect();
    for r in &rots[1..] {
        for (a, b) in r.to_cols_array().iter().zip(rots[0].to_cols_array()) {
            assert!((a - b).abs() < 1e-5);
        }
    }
}

// ---------------------------------------------------------------------------
// Modifiers, collections, parenting chains, world and animation.

use sex_3d::io::blend::{import_blend_full, BlendKey, INTERP_LINEAR};
use sex_3d::modifiers::Modifier;

/// 2.8+ layouts: 2.8x names (`restrictflag`, no `affect_type`, node-less
/// world), 2.9-4.3 names, and 5.0's slotted actions.
const EXTRAS: [Cfg; 7] = [
    Cfg { ptr: 8, big: false, layout: Layout::Legacy30 },
    Cfg { ptr: 4, big: true, layout: Layout::Legacy30 },
    Cfg { ptr: 8, big: false, layout: Layout::Attr36 },
    Cfg { ptr: 8, big: false, layout: Layout::Attr42 },
    Cfg { ptr: 4, big: true, layout: Layout::Attr42 },
    Cfg { ptr: 8, big: false, layout: Layout::Attr50 },
    Cfg { ptr: 8, big: true, layout: Layout::Attr50 },
];

#[test]
fn modifier_stack() {
    let want = vec![
        Modifier::Mirror { x: true, y: false, z: true, merge: true, merge_distance: 0.01 },
        Modifier::Subdivision { levels: 2, simple: false },
        Modifier::Array { count: 3, relative: [1.5, 0.0, 0.0], constant: [0.0, 0.0, 2.0], merge: true },
        Modifier::Solidify { thickness: 0.2, offset: 0.5 },
        Modifier::Bevel { width: 0.05, segments: 3, angle: 0.6 },
        Modifier::Triangulate,
        Modifier::Weld { distance: 0.02 },
        Modifier::Displace { strength: 1.5, vertex_group: None, invert_vertex_group: false },
        Modifier::Smooth { factor: 0.7, iterations: 4, vertex_group: None, invert_vertex_group: false },
    ];
    for c in EXTRAS {
        let s = import_blend(&blendgen::extras_file(c)).unwrap_or_else(|e| panic!("{c:?}: {e}"));
        let o = &s.objects[s.find_object("Mod").unwrap()];
        assert_eq!(o.modifiers, want, "{c:?}");
        // The stack evaluates.
        let m = s.evaluated_mesh(s.find_object("Mod").unwrap()).unwrap();
        assert!(m.face_count() > 0, "{c:?}");
    }
    // The classic test scene has no modifiers.
    let s = import_blend(&scene_file(ALL[0])).unwrap();
    assert!(s.objects.iter().all(|o| o.modifiers.is_empty()));
}

#[test]
fn collections_and_visibility() {
    for c in EXTRAS {
        let s = import_blend(&blendgen::extras_file(c)).unwrap_or_else(|e| panic!("{c:?}: {e}"));
        let col = |n: &str| s.collections.iter().position(|x| x.name == n).unwrap_or_else(|| panic!("{c:?}: no collection {n}"));
        assert_eq!(s.collections.len(), 4, "{c:?}: {:?}", s.collections);
        assert_eq!(s.collections[0].name, "Scene Collection", "{c:?}");
        let (props, nested, hidden) = (col("Props"), col("Nested"), col("Hidden"));
        assert_eq!(s.collections[props].parent, Some(0), "{c:?}");
        assert_eq!(s.collections[nested].parent, Some(props), "{c:?}");
        // Linked under both the master and Nested; the first link wins.
        assert_eq!(s.collections[hidden].parent, Some(0), "{c:?}");
        assert!(s.collections[props].visible && s.collections[nested].visible && !s.collections[hidden].visible, "{c:?}");
        let ob = |n: &str| &s.objects[s.find_object(n).unwrap()];
        assert_eq!(ob("Root").collection, 0, "{c:?}");
        assert_eq!(ob("Mod").collection, props, "{c:?}");
        assert_eq!(ob("Mid").collection, props, "{c:?}");
        assert_eq!(ob("Anim").collection, nested, "{c:?}");
        assert_eq!(ob("Leaf").collection, hidden, "{c:?}");
        // Mid is disabled in viewports; Mod only in renders.
        assert!(!ob("Mid").visible && ob("Mod").visible && ob("Root").visible, "{c:?}");
        assert!(!s.is_visible(s.find_object("Leaf").unwrap()), "{c:?}");
        assert!(s.is_visible(s.find_object("Anim").unwrap()), "{c:?}");
    }
}

#[test]
fn parent_chain_world_transforms() {
    for c in EXTRAS {
        let s = import_blend(&blendgen::extras_file(c)).unwrap();
        let (root, mid, leaf) = (s.find_object("Root").unwrap(), s.find_object("Mid").unwrap(), s.find_object("Leaf").unwrap());
        assert_eq!(s.objects[mid].parent, Some(root), "{c:?}");
        assert_eq!(s.objects[leaf].parent, Some(mid), "{c:?}");
        // Mid's parent inverse cancels Root's transform; Leaf has none.
        let m = s.world_matrix(mid);
        assert!(m.transform_point(Vec3::ZERO).approx_eq(Vec3::new(0.0, 2.0, 0.0), 1e-5), "{c:?}");
        assert!(m.transform_vector(Vec3::X).approx_eq(Vec3::X, 1e-5), "{c:?}");
        let l = s.world_matrix(leaf);
        assert!(l.transform_point(Vec3::ZERO).approx_eq(Vec3::new(0.0, 2.0, 3.0), 1e-5), "{c:?}");
        assert!(l.transform_vector(Vec3::Y).approx_eq(Vec3::Y, 1e-5), "{c:?}");
        let r = s.world_matrix(root);
        assert!(r.transform_vector(Vec3::X).approx_eq(Vec3::Y, 1e-5), "{c:?}");
    }
}

#[test]
fn scene_settings_and_world() {
    for c in EXTRAS {
        let s = import_blend(&blendgen::extras_file(c)).unwrap();
        assert_eq!(s.name, "Extras");
        assert_eq!((s.frame_start, s.frame_end), (1, 48), "{c:?}");
        assert!(approx(s.fps, 25.0), "{c:?}");
        assert_eq!(s.active_camera, None, "{c:?}");
        let (color, strength) = blendgen::extras_world(c);
        assert!(s.world.color.approx_eq(Vec3::from_array(color), 1e-6), "{c:?}: {:?}", s.world);
        assert!(approx(s.world.strength, strength), "{c:?}: {:?}", s.world);
    }
    // The classic scene's camera is the active one and it has no world.
    let s = import_blend(&scene_file(ALL[3])).unwrap();
    assert_eq!(s.active_camera, s.find_object("Camera"));
    assert_eq!(s.world, sex_3d::scene::World::default());
}

#[test]
fn object_animation() {
    for c in EXTRAS {
        let bytes = blendgen::extras_file(c);
        let (s, actions) = import_blend_full(&bytes).unwrap_or_else(|e| panic!("{c:?}: {e}"));
        assert_eq!(s, import_blend(&bytes).unwrap());
        assert_eq!(actions.len(), 1, "{c:?}");
        let a = &actions[0];
        assert_eq!((a.object.as_str(), a.action.as_str()), ("Anim", "AnimAction"), "{c:?}");
        let key = |frame: f32, value: f32, interpolation: u8| BlendKey { frame, value, interpolation, handles: None };
        let want: Vec<(&str, usize, Vec<BlendKey>)> = vec![
            ("location", 0, vec![key(1., 0., 2), key(10., 5., 1)]),
            ("rotation_euler", 2, vec![key(1., 0., 0), key(20., 3., 2)]),
            ("scale", 1, vec![key(1., 1., INTERP_LINEAR), key(2., 2., INTERP_LINEAR)]),
        ];
        let got: Vec<(&str, usize, Vec<BlendKey>)> = a.curves.iter().map(|f| (f.path.as_str(), f.index, f.keys.iter().map(|k| BlendKey { handles: None, ..*k }).collect())).collect();
        assert_eq!(got, want, "{c:?}");
        // The fixture also wrote a non-transform curve and an empty one.
        assert_eq!(blendgen::extras_curves().len(), 5);
    }
    let (_, actions) = import_blend_full(&scene_file(ALL[5])).unwrap();
    assert!(actions.is_empty());
}

#[test]
fn corrupted_extras_never_panic() {
    let mut rng = Rng(0x0123_4567_89AB_CDEF);
    for c in EXTRAS {
        let full = blendgen::extras_file(c);
        for _ in 0..300 {
            let mut b = full.clone();
            for _ in 0..1 + rng.next() % 3 {
                let at = (rng.next() % b.len() as u64) as usize;
                b[at] ^= 1 << (rng.next() % 8);
            }
            let at = (rng.next() % (b.len() as u64 - 8)) as usize;
            b[at..at + 8].copy_from_slice(&rng.next().to_le_bytes());
            let _ = import_blend_full(&b);
        }
        for len in (0..full.len()).step_by(13) {
            assert!(import_blend_full(&full[..len]).is_err(), "{c:?} truncated to {len}");
        }
    }
}

#[test]
fn modifier_stacks_import() {
    use blendgen::{n, nv, p, s, Dna, Writer, V};
    use sex_3d::modifiers::{BooleanOp, DecimateMode, Modifier, SimpleDeformMode};
    for (ptr, big) in [(8, false), (4, true)] {
        let mut d = Dna::new(ptr);
        d.def("ListBase", &[("void", "*first"), ("void", "*last")]);
        d.def("ID", &[("void", "*next"), ("void", "*prev"), ("char", "name[66]"), ("short", "flag"), ("int", "us")]);
        d.def("MVert", &[("float", "co[3]")]);
        d.def("Mesh", &[("ID", "id"), ("MVert", "*mvert"), ("int", "totvert"), ("int", "totpoly"), ("int", "totloop"), ("char", "_pad[4]")]);
        d.def("Object", &[("ID", "id"), ("short", "type"), ("char", "_pad0[6]"), ("void", "*data"), ("float", "loc[3]"), ("float", "scale[3]"), ("short", "rotmode"), ("char", "_pad1[2]"), ("ListBase", "modifiers")]);
        d.def("ModifierData", &[("ModifierData", "*next"), ("ModifierData", "*prev"), ("int", "type"), ("int", "mode"), ("short", "flag"), ("char", "_pad[6]"), ("char", "name[64]")]);
        d.def("SubsurfModifierData", &[("ModifierData", "modifier"), ("short", "subdivType"), ("short", "levels"), ("short", "renderLevels"), ("short", "flags")]);
        d.def("DecimateModifierData", &[("ModifierData", "modifier"), ("float", "percent"), ("short", "iter"), ("char", "delimit"), ("char", "symmetry_axis"), ("float", "angle"), ("char", "defgrp_name[64]"), ("short", "flag"), ("short", "mode")]);
        d.def("BooleanModifierData", &[("ModifierData", "modifier"), ("Object", "*object"), ("Collection", "*collection"), ("float", "double_threshold"), ("char", "operation"), ("char", "solver"), ("char", "_pad[2]")]);
        d.def("SimpleDeformModifierData", &[("ModifierData", "modifier"), ("Object", "*origin"), ("char", "vgroup_name[64]"), ("float", "factor"), ("float", "limit[2]"), ("char", "mode"), ("char", "axis"), ("char", "deform_axis"), ("char", "flag")]);
        d.def("NoiseModifierData", &[("ModifierData", "modifier"), ("float", "x")]);
        let mut w = Writer::new(d, big, false, 402);
        let (me, ob, cutter) = (w.alloc(), w.alloc(), w.alloc());
        let mods: Vec<u64> = (0..6).map(|_| w.alloc()).collect();
        let vert = w.add("DATA", "MVert", &[("co", nv(&[0., 0., 1.]))]);
        let m = w.inst("Mesh", &[("id.name", s("MEMesh")), ("mvert", p(vert)), ("totvert", n(1.))]);
        w.put(me, "ME", "Mesh", 1, m);
        let o = w.inst("Object", &[("id.name", s("OBCube")), ("type", n(1.)), ("data", p(me)), ("scale", nv(&[1., 1., 1.])), ("rotmode", n(1.)), ("modifiers.first", p(mods[0])), ("modifiers.last", p(mods[5]))]);
        w.put(ob, "OB", "Object", 1, o);
        let o = w.inst("Object", &[("id.name", s("OBCutter")), ("type", n(1.)), ("data", p(me)), ("scale", nv(&[1., 1., 1.])), ("rotmode", n(1.))]);
        w.put(cutter, "OB", "Object", 1, o);
        // The ModifierData header: list links, enabled in viewport and render.
        let with = |i: usize, mode: f64, extra: Vec<(&str, V)>| -> Vec<(String, V)> {
            let mut f = vec![("modifier.next".to_string(), p(mods.get(i + 1).copied().unwrap_or(0))), ("modifier.mode".to_string(), n(mode))];
            if i > 0 {
                f.push(("modifier.prev".to_string(), p(mods[i - 1])));
            }
            f.extend(extra.into_iter().map(|(k, v)| (k.to_string(), v)));
            f
        };
        let b = w.inst("SubsurfModifierData", &with(0, 3., vec![("levels", n(2.)), ("subdivType", n(1.))]));
        w.put(mods[0], "DATA", "SubsurfModifierData", 1, b);
        let b = w.inst("DecimateModifierData", &with(1, 3., vec![("percent", n(0.25)), ("mode", n(0.)), ("flag", n(2.)), ("angle", n(0.1))]));
        w.put(mods[1], "DATA", "DecimateModifierData", 1, b);
        let b = w.inst("BooleanModifierData", &with(2, 3., vec![("object", p(cutter)), ("operation", n(1.))]));
        w.put(mods[2], "DATA", "BooleanModifierData", 1, b);
        // Switched off in the viewport: skipped.
        let b = w.inst("SubsurfModifierData", &with(3, 2., vec![("levels", n(1.))]));
        w.put(mods[3], "DATA", "SubsurfModifierData", 1, b);
        let b = w.inst("SimpleDeformModifierData", &with(4, 1., vec![("factor", n(0.5)), ("limit", nv(&[0.25, 0.75])), ("mode", n(2.)), ("deform_axis", n(0.)), ("axis", n(4.))]));
        w.put(mods[4], "DATA", "SimpleDeformModifierData", 1, b);
        // A type Polyamour does not have: skipped.
        let b = w.inst("NoiseModifierData", &with(5, 3., vec![]));
        w.put(mods[5], "DATA", "NoiseModifierData", 1, b);
        let scene = import_blend(&w.finish()).unwrap();
        let cube = &scene.objects[scene.find_object("Cube").unwrap()];
        assert_eq!(
            cube.modifiers,
            vec![
                Modifier::Subdivision { levels: 2, simple: true },
                Modifier::Decimate { mode: DecimateMode::Collapse, ratio: 0.25, iterations: 0, angle: 0.1, triangulate: true },
                Modifier::Boolean { object: "Cutter".into(), operation: BooleanOp::Union },
                Modifier::SimpleDeform { mode: SimpleDeformMode::Bend, axis: 0, factor: 0.5, limits: [0.25, 0.75], lock: [false, false, true], vertex_group: None, invert_vertex_group: false },
            ],
            "ptr {ptr} big {big}"
        );
    }
}

/// Vertex groups, shape keys, armatures, poses, constraints, bone parents,
/// lattices and the rigging modifiers, in both byte orders and pointer sizes.
#[test]
fn rigging_imports() {
    use blendgen::{n, nv, p, s, Dna, Writer, V};
    use sex_3d::armature::RotationMode;
    use sex_3d::constraints::{ConstraintKind, Space, TrackAxis};
    use sex_3d::mesh::KeyInterpolation;
    use sex_3d::modifiers::{Falloff, Modifier, WeightMixMode, WeightMixSet};
    use sex_3d::scene::ParentType;
    for (ptr, big, large) in [(8, false, false), (4, true, false), (8, false, true)] {
        let mut d = Dna::new(ptr);
        d.def("ListBase", &[("void", "*first"), ("void", "*last")]);
        d.def("ID", &[("void", "*next"), ("void", "*prev"), ("char", "name[66]"), ("short", "flag"), ("int", "us")]);
        d.def("MVert", &[("float", "co[3]")]);
        d.def("MDeformWeight", &[("int", "def_nr"), ("float", "weight")]);
        d.def("MDeformVert", &[("MDeformWeight", "*dw"), ("int", "totweight"), ("int", "flag")]);
        d.def("bDeformGroup", &[("bDeformGroup", "*next"), ("bDeformGroup", "*prev"), ("char", "name[64]"), ("char", "flag"), ("char", "_pad0[7]")]);
        d.def(
            "KeyBlock",
            &[
                ("KeyBlock", "*next"),
                ("KeyBlock", "*prev"),
                ("float", "pos"),
                ("float", "curval"),
                ("short", "type"),
                ("short", "_pad1"),
                ("short", "relative"),
                ("short", "flag"),
                ("int", "totelem"),
                ("int", "uid"),
                ("void", "*data"),
                ("char", "name[64]"),
                ("char", "vgroup[64]"),
                ("float", "slidermin"),
                ("float", "slidermax"),
            ],
        );
        d.def("Key", &[("ID", "id"), ("KeyBlock", "*refkey"), ("ListBase", "block"), ("char", "type"), ("char", "_pad[3]"), ("float", "ctime")]);
        d.def(
            "Mesh",
            &[("ID", "id"), ("MVert", "*mvert"), ("MDeformVert", "*dvert"), ("Key", "*key"), ("ListBase", "vertex_group_names"), ("int", "totvert"), ("int", "totpoly"), ("int", "totloop"), ("char", "_pad[4]")],
        );
        d.def(
            "Bone",
            &[
                ("Bone", "*next"),
                ("Bone", "*prev"),
                ("Bone", "*parent"),
                ("ListBase", "childbase"),
                ("char", "name[64]"),
                ("float", "roll"),
                ("int", "flag"),
                ("char", "inherit_scale_mode"),
                ("char", "_pad[3]"),
                ("float", "arm_head[3]"),
                ("float", "arm_tail[3]"),
                ("float", "arm_mat[4][4]"),
                ("float", "arm_roll"),
                ("float", "dist"),
                ("float", "weight"),
                ("float", "rad_head"),
                ("float", "rad_tail"),
                ("float", "length"),
                ("short", "segments"),
                ("char", "_pad2[2]"),
            ],
        );
        d.def("BoneCollectionMember", &[("BoneCollectionMember", "*next"), ("BoneCollectionMember", "*prev"), ("Bone", "*bone")]);
        d.def("BoneCollection", &[("BoneCollection", "*next"), ("BoneCollection", "*prev"), ("char", "name[64]"), ("ListBase", "bones")]);
        d.def("bArmature", &[("ID", "id"), ("ListBase", "bonebase"), ("BoneCollection", "**collection_array"), ("int", "collection_array_num"), ("char", "_pad[4]")]);
        d.def(
            "bConstraint",
            &[
                ("bConstraint", "*next"),
                ("bConstraint", "*prev"),
                ("void", "*data"),
                ("short", "type"),
                ("short", "flag"),
                ("char", "ownspace"),
                ("char", "tarspace"),
                ("char", "_pad[2]"),
                ("char", "name[64]"),
                ("float", "enforce"),
                ("float", "headtail"),
                ("Object", "*space_object"),
                ("char", "space_subtarget[64]"),
            ],
        );
        d.def(
            "bKinematicConstraint",
            &[
                ("Object", "*tar"),
                ("short", "iterations"),
                ("short", "flag"),
                ("short", "rootbone"),
                ("short", "max_rootbone"),
                ("char", "subtarget[64]"),
                ("Object", "*poletar"),
                ("char", "polesubtarget[64]"),
                ("float", "poleangle"),
                ("float", "weight"),
            ],
        );
        d.def("bTrackToConstraint", &[("Object", "*tar"), ("int", "reserved1"), ("int", "reserved2"), ("int", "flags"), ("char", "subtarget[64]"), ("char", "_pad[4]")]);
        d.def("bChildOfConstraint", &[("Object", "*tar"), ("int", "flag"), ("int", "_pad"), ("float", "invmat[4][4]"), ("char", "subtarget[64]")]);
        d.def(
            "bLocLimitConstraint",
            &[("float", "xmin"), ("float", "xmax"), ("float", "ymin"), ("float", "ymax"), ("float", "zmin"), ("float", "zmax"), ("short", "flag"), ("short", "flag2")],
        );
        d.def(
            "bPoseChannel",
            &[
                ("bPoseChannel", "*next"),
                ("bPoseChannel", "*prev"),
                ("char", "name[64]"),
                ("short", "ikflag"),
                ("short", "rotmode"),
                ("ListBase", "constraints"),
                ("float", "loc[3]"),
                ("float", "size[3]"),
                ("float", "eul[3]"),
                ("float", "quat[4]"),
                ("float", "rotAxis[3]"),
                ("float", "rotAngle"),
                ("float", "limitmin[3]"),
                ("float", "limitmax[3]"),
                ("float", "stiffness[3]"),
                ("float", "ikstretch"),
            ],
        );
        d.def("bPose", &[("ListBase", "chanbase")]);
        d.def("BPoint", &[("float", "vec[4]"), ("float", "weight"), ("short", "f1"), ("short", "hide"), ("float", "radius"), ("char", "_pad[4]")]);
        d.def(
            "Lattice",
            &[("ID", "id"), ("short", "pntsu"), ("short", "pntsv"), ("short", "pntsw"), ("short", "flag"), ("char", "typeu"), ("char", "typev"), ("char", "typew"), ("char", "_pad"), ("BPoint", "*def")],
        );
        d.def(
            "Object",
            &[
                ("ID", "id"),
                ("short", "type"),
                ("short", "partype"),
                ("char", "_pad0[4]"),
                ("void", "*data"),
                ("Object", "*parent"),
                ("char", "parsubstr[64]"),
                ("float", "loc[3]"),
                ("float", "scale[3]"),
                ("short", "rotmode"),
                ("char", "_pad1[2]"),
                ("ListBase", "modifiers"),
                ("ListBase", "constraints"),
                ("ListBase", "defbase"),
                ("bPose", "*pose"),
            ],
        );
        d.def("ModifierData", &[("ModifierData", "*next"), ("ModifierData", "*prev"), ("int", "type"), ("int", "mode"), ("short", "flag"), ("char", "_pad[6]"), ("char", "name[64]")]);
        d.def("ArmatureModifierData", &[("ModifierData", "modifier"), ("short", "deformflag"), ("short", "multi"), ("char", "_pad[4]"), ("Object", "*object"), ("char", "defgrp_name[64]")]);
        d.def(
            "HookModifierData",
            &[
                ("ModifierData", "modifier"),
                ("char", "subtarget[64]"),
                ("char", "flag"),
                ("char", "falloff_type"),
                ("char", "_pad[6]"),
                ("Object", "*object"),
                ("float", "parentinv[4][4]"),
                ("float", "cent[3]"),
                ("float", "falloff"),
                ("int", "*indexar"),
                ("int", "indexar_num"),
                ("float", "force"),
                ("char", "name[64]"),
            ],
        );
        d.def("SmoothModifierData", &[("ModifierData", "modifier"), ("float", "fac"), ("char", "defgrp_name[64]"), ("short", "flag"), ("short", "repeat")]);
        d.def(
            "WeightVGMixModifierData",
            &[
                ("ModifierData", "modifier"),
                ("char", "defgrp_name_a[64]"),
                ("char", "defgrp_name_b[64]"),
                ("float", "default_weight_a"),
                ("float", "default_weight_b"),
                ("char", "mix_mode"),
                ("char", "mix_set"),
                ("char", "_pad[6]"),
                ("float", "mask_constant"),
                ("int", "flag"),
            ],
        );
        d.def("LatticeModifierData", &[("ModifierData", "modifier"), ("Object", "*object"), ("char", "name[64]"), ("float", "strength"), ("short", "flag"), ("char", "_pad[2]")]);

        let mut w = Writer::new(d, big, large, if large { 500 } else { 402 });
        let ident: Vec<f64> = vec![1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];

        // --- Mesh: 3 vertices, groups "Lower"/"Upper", two shape keys.
        let me = w.alloc();
        let verts: Vec<u8> = [[0., 0., 0.], [0., 0., 1.], [0., 0., 2.]].iter().flat_map(|c| w.inst("MVert", &[("co", nv(c))])).collect();
        let mvert = w.alloc();
        w.put(mvert, "DATA", "MVert", 3, verts);
        let dw0 = w.alloc();
        let d0 = w.inst("MDeformWeight", &[("def_nr", n(0.)), ("weight", n(1.))]);
        w.put(dw0, "DATA", "MDeformWeight", 1, d0);
        let dw1 = w.alloc();
        let d1: Vec<u8> = [w.inst("MDeformWeight", &[("def_nr", n(0.)), ("weight", n(0.5))]), w.inst("MDeformWeight", &[("def_nr", n(1.)), ("weight", n(0.5))])].concat();
        w.put(dw1, "DATA", "MDeformWeight", 2, d1);
        let dv: Vec<u8> = [
            w.inst("MDeformVert", &[("dw", p(dw0)), ("totweight", n(1.))]),
            w.inst("MDeformVert", &[("dw", p(dw1)), ("totweight", n(2.))]),
            w.inst("MDeformVert", &[("totweight", n(0.))]),
        ]
        .concat();
        let dvert = w.alloc();
        w.put(dvert, "DATA", "MDeformVert", 3, dv);
        let (g0, g1) = (w.alloc(), w.alloc());
        let b = w.inst("bDeformGroup", &[("next", p(g1)), ("name", s("Lower"))]);
        w.put(g0, "DATA", "bDeformGroup", 1, b);
        let b = w.inst("bDeformGroup", &[("prev", p(g0)), ("name", s("Upper"))]);
        w.put(g1, "DATA", "bDeformGroup", 1, b);
        let (kb0, kb1, key) = (w.alloc(), w.alloc(), w.alloc());
        let basis = w.raw(w.f32s(&[0., 0., 0., 0., 0., 1., 0., 0., 2.]), 1);
        let smile = w.raw(w.f32s(&[1., 0., 0., 1., 0., 1., 1., 0., 2.]), 1);
        let b = w.inst("KeyBlock", &[("next", p(kb1)), ("name", s("Basis")), ("data", p(basis)), ("totelem", n(3.)), ("slidermax", n(1.))]);
        w.put(kb0, "DATA", "KeyBlock", 1, b);
        let b = w.inst(
            "KeyBlock",
            &[
                ("prev", p(kb0)),
                ("name", s("Smile")),
                ("data", p(smile)),
                ("totelem", n(3.)),
                ("curval", n(0.25)),
                ("slidermin", n(-1.)),
                ("slidermax", n(2.)),
                ("vgroup", s("Upper")),
                ("type", n(3.)),
                ("pos", n(0.1)),
                ("flag", n(1.)),
            ],
        );
        w.put(kb1, "DATA", "KeyBlock", 1, b);
        let k = w.inst("Key", &[("id.name", s("KEKey")), ("refkey", p(kb0)), ("block.first", p(kb0)), ("block.last", p(kb1)), ("type", n(1.)), ("ctime", n(5.))]);
        w.put(key, "KE", "Key", 1, k);
        let m = w.inst(
            "Mesh",
            &[
                ("id.name", s("MEBody")),
                ("mvert", p(mvert)),
                ("dvert", p(dvert)),
                ("key", p(key)),
                ("vertex_group_names.first", p(g0)),
                ("vertex_group_names.last", p(g1)),
                ("totvert", n(3.)),
            ],
        );
        w.put(me, "ME", "Mesh", 1, m);
        // A second mesh whose group names live on its object (pre-3.0).
        let me2 = w.alloc();
        let m = w.inst("Mesh", &[("id.name", s("MEOld")), ("mvert", p(mvert)), ("dvert", p(dvert)), ("totvert", n(3.))]);
        w.put(me2, "ME", "Mesh", 1, m);

        // --- Armature: Root -> Child (connected, rolled), collection "Deform".
        let (ar, root, child) = (w.alloc(), w.alloc(), w.alloc());
        let rest_z = [1., 0., 0., 0., 0., 0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1.];
        let b = w.inst(
            "Bone",
            &[
                ("name", s("Root")),
                ("childbase.first", p(child)),
                ("childbase.last", p(child)),
                ("arm_head", nv(&[0., 0., 0.])),
                ("arm_tail", nv(&[0., 0., 1.])),
                ("arm_mat", nv(&rest_z)),
                ("dist", n(0.5)),
                ("weight", n(1.)),
                ("rad_head", n(0.2)),
                ("rad_tail", n(0.1)),
                ("segments", n(3.)),
                ("flag", n((1 << 12) as f64)),
            ],
        );
        w.put(root, "DATA", "Bone", 1, b);
        let b = w.inst(
            "Bone",
            &[
                ("name", s("Child")),
                ("parent", p(root)),
                ("arm_head", nv(&[0., 0., 1.])),
                ("arm_tail", nv(&[0., 0., 2.])),
                ("arm_mat", nv(&rest_z)),
                ("arm_roll", n(0.5)),
                ("flag", n((1 << 4 | 1 << 9) as f64)),
                ("inherit_scale_mode", n(3.)),
                ("weight", n(1.)),
                ("segments", n(1.)),
            ],
        );
        w.put(child, "DATA", "Bone", 1, b);
        let (coll, member) = (w.alloc(), w.alloc());
        let b = w.inst("BoneCollectionMember", &[("bone", p(child))]);
        w.put(member, "DATA", "BoneCollectionMember", 1, b);
        let b = w.inst("BoneCollection", &[("name", s("Deform")), ("bones.first", p(member)), ("bones.last", p(member))]);
        w.put(coll, "DATA", "BoneCollection", 1, b);
        let carr = w.raw(w.ptrs(&[coll]), 1);
        let a = w.inst("bArmature", &[("id.name", s("ARSkel")), ("bonebase.first", p(root)), ("bonebase.last", p(root)), ("collection_array", p(carr)), ("collection_array_num", n(1.))]);
        w.put(ar, "AR", "bArmature", 1, a);

        // --- Lattice 2x2x2 with one point moved.
        let lt = w.alloc();
        let mut pts = Vec::new();
        for z in [-0.5, 0.5] {
            for y in [-0.5, 0.5] {
                for x in [-0.5, 0.5] {
                    let zz = if x > 0.0 && y > 0.0 && z > 0.0 { 0.9 } else { z };
                    pts.push(w.inst("BPoint", &[("vec", nv(&[x, y, zz, 1.]))]));
                }
            }
        }
        let def = w.alloc();
        w.put(def, "DATA", "BPoint", 8, pts.concat());
        let l = w.inst("Lattice", &[("id.name", s("LTCage")), ("pntsu", n(2.)), ("pntsv", n(2.)), ("pntsw", n(2.)), ("typeu", n(0.)), ("typev", n(1.)), ("typew", n(2.)), ("flag", n(2.)), ("def", p(def))]);
        w.put(lt, "LT", "Lattice", 1, l);

        // --- Objects.
        let (ob_rig, ob_body, ob_old, ob_goal, ob_prop, ob_lat) = (w.alloc(), w.alloc(), w.alloc(), w.alloc(), w.alloc(), w.alloc());
        // Pose: Child rotated (quaternion) with an IK constraint.
        let (pc_root, pc_child, ik_con, ik_data, pose) = (w.alloc(), w.alloc(), w.alloc(), w.alloc(), w.alloc());
        let b = w.inst("bKinematicConstraint", &[("tar", p(ob_goal)), ("iterations", n(200.)), ("flag", n(1.)), ("rootbone", n(2.)), ("poletar", p(ob_prop)), ("poleangle", n(0.5)), ("weight", n(1.))]);
        w.put(ik_data, "DATA", "bKinematicConstraint", 1, b);
        let b = w.inst("bConstraint", &[("data", p(ik_data)), ("name", s("IK")), ("enforce", n(0.75)), ("ownspace", n(2.)), ("tarspace", n(0.))]);
        w.put(ik_con, "DATA", "bConstraint", 1, b);
        let b = w.inst("bPoseChannel", &[("next", p(pc_child)), ("name", s("Root")), ("rotmode", n(0.)), ("quat", nv(&[1., 0., 0., 0.])), ("size", nv(&[1., 1., 1.]))]);
        w.put(pc_root, "DATA", "bPoseChannel", 1, b);
        let b = w.inst(
            "bPoseChannel",
            &[
                ("prev", p(pc_root)),
                ("name", s("Child")),
                ("rotmode", n(4.)),
                ("eul", nv(&[0.1, 0.2, 0.3])),
                ("quat", nv(&[1., 0., 0., 0.])),
                ("size", nv(&[1., 2., 1.])),
                ("loc", nv(&[0., 0.5, 0.])),
                ("ikflag", n((1 | 16) as f64)),
                ("limitmin", nv(&[0., -1., 0.])),
                ("limitmax", nv(&[0., 1., 0.])),
                ("stiffness", nv(&[0.25, 0., 0.])),
                ("ikstretch", n(0.1)),
                ("constraints.first", p(ik_con)),
                ("constraints.last", p(ik_con)),
            ],
        );
        w.put(pc_child, "DATA", "bPoseChannel", 1, b);
        let b = w.inst("bPose", &[("chanbase.first", p(pc_root)), ("chanbase.last", p(pc_child))]);
        w.put(pose, "DATA", "bPose", 1, b);
        let o = w.inst("Object", &[("id.name", s("OBRig")), ("type", n(25.)), ("data", p(ar)), ("scale", nv(&[1., 1., 1.])), ("rotmode", n(1.)), ("pose", p(pose))]);
        w.put(ob_rig, "OB", "Object", 1, o);

        // Body: armature parented, with Armature, Hook, Smooth, Mix and Lattice modifiers.
        let mods: Vec<u64> = (0..5).map(|_| w.alloc()).collect();
        let hdr = |i: usize| -> Vec<(String, V)> {
            let mut f = vec![("modifier.next".to_string(), p(mods.get(i + 1).copied().unwrap_or(0))), ("modifier.mode".to_string(), n(3.))];
            if i > 0 {
                f.push(("modifier.prev".to_string(), p(mods[i - 1])));
            }
            f
        };
        let with = |i: usize, extra: Vec<(&str, V)>| -> Vec<(String, V)> {
            let mut f = hdr(i);
            f.extend(extra.into_iter().map(|(k, v)| (k.to_string(), v)));
            f
        };
        let b = w.inst("ArmatureModifierData", &with(0, vec![("object", p(ob_rig)), ("deformflag", n((1 | 4 | 16) as f64)), ("multi", n(1.)), ("defgrp_name", s("Lower"))]));
        w.put(mods[0], "DATA", "ArmatureModifierData", 1, b);
        let idx = w.raw(w.i32s(&[0, 2]), 1);
        let mut inv = ident.clone();
        inv[14] = -1.0;
        let b = w.inst(
            "HookModifierData",
            &with(
                1,
                vec![
                    ("object", p(ob_rig)),
                    ("subtarget", s("Child")),
                    ("flag", n(1.)),
                    ("falloff_type", n(3.)),
                    ("falloff", n(2.)),
                    ("cent", nv(&[0., 0., 1.])),
                    ("parentinv", nv(&inv)),
                    ("indexar", p(idx)),
                    ("indexar_num", n(2.)),
                    ("force", n(0.5)),
                ],
            ),
        );
        w.put(mods[1], "DATA", "HookModifierData", 1, b);
        let b = w.inst("SmoothModifierData", &with(2, vec![("fac", n(0.5)), ("repeat", n(2.)), ("defgrp_name", s("Upper")), ("flag", n((1 << 4) as f64))]));
        w.put(mods[2], "DATA", "SmoothModifierData", 1, b);
        let b = w.inst(
            "WeightVGMixModifierData",
            &with(3, vec![("defgrp_name_a", s("Upper")), ("defgrp_name_b", s("Lower")), ("default_weight_b", n(0.25)), ("mix_mode", n(7.)), ("mix_set", n(4.)), ("mask_constant", n(1.)), ("flag", n(8.))]),
        );
        w.put(mods[3], "DATA", "WeightVGMixModifierData", 1, b);
        let b = w.inst("LatticeModifierData", &with(4, vec![("object", p(ob_lat)), ("strength", n(0.8)), ("name", s("Upper")), ("flag", n(1.))]));
        w.put(mods[4], "DATA", "LatticeModifierData", 1, b);
        let o = w.inst(
            "Object",
            &[
                ("id.name", s("OBBody")),
                ("type", n(1.)),
                ("data", p(me)),
                ("parent", p(ob_rig)),
                ("partype", n(4.)),
                ("scale", nv(&[1., 1., 1.])),
                ("rotmode", n(1.)),
                ("modifiers.first", p(mods[0])),
                ("modifiers.last", p(mods[4])),
            ],
        );
        w.put(ob_body, "OB", "Object", 1, o);

        // Old: groups from Object.defbase.
        let (og0, og1) = (w.alloc(), w.alloc());
        let b = w.inst("bDeformGroup", &[("next", p(og1)), ("name", s("A"))]);
        w.put(og0, "DATA", "bDeformGroup", 1, b);
        let b = w.inst("bDeformGroup", &[("prev", p(og0)), ("name", s("B"))]);
        w.put(og1, "DATA", "bDeformGroup", 1, b);
        let o = w.inst("Object", &[("id.name", s("OBOld")), ("type", n(1.)), ("data", p(me2)), ("scale", nv(&[1., 1., 1.])), ("rotmode", n(1.)), ("defbase.first", p(og0)), ("defbase.last", p(og1))]);
        w.put(ob_old, "OB", "Object", 1, o);

        // Goal: Track To, Child Of (inverse) and a muted Limit Location.
        let cons: Vec<u64> = (0..3).map(|_| w.alloc()).collect();
        let datas: Vec<u64> = (0..3).map(|_| w.alloc()).collect();
        let b = w.inst("bTrackToConstraint", &[("tar", p(ob_prop)), ("reserved1", n(5.)), ("reserved2", n(1.)), ("flags", n(1.))]);
        w.put(datas[0], "DATA", "bTrackToConstraint", 1, b);
        let mut invm = ident.clone();
        invm[12] = 2.0;
        let b = w.inst("bChildOfConstraint", &[("tar", p(ob_rig)), ("subtarget", s("Root")), ("flag", n((1 | 2 | 4 | 32 | 512) as f64)), ("invmat", nv(&invm))]);
        w.put(datas[1], "DATA", "bChildOfConstraint", 1, b);
        let b = w.inst("bLocLimitConstraint", &[("xmin", n(-1.)), ("zmax", n(3.)), ("flag", n((1 | 32) as f64))]);
        w.put(datas[2], "DATA", "bLocLimitConstraint", 1, b);
        let names = ["Track", "Child", "Limit"];
        for i in 0..3 {
            let mut f = vec![("data", p(datas[i])), ("name", s(names[i])), ("enforce", n(1.))];
            if i < 2 {
                f.push(("next", p(cons[i + 1])));
            }
            if i > 0 {
                f.push(("prev", p(cons[i - 1])));
            }
            if i == 1 {
                f.push(("headtail", n(0.5)));
                f.push(("tarspace", n(3.)));
            }
            if i == 2 {
                f.push(("flag", n((1 << 9) as f64)));
            }
            let b = w.inst("bConstraint", &f);
            w.put(cons[i], "DATA", "bConstraint", 1, b);
        }
        let o = w.inst(
            "Object",
            &[("id.name", s("OBGoal")), ("type", n(0.)), ("scale", nv(&[1., 1., 1.])), ("rotmode", n(1.)), ("loc", nv(&[0., 1., 2.])), ("constraints.first", p(cons[0])), ("constraints.last", p(cons[2]))],
        );
        w.put(ob_goal, "OB", "Object", 1, o);
        // Prop: parented to the Child bone.
        let o = w.inst("Object", &[("id.name", s("OBProp")), ("type", n(0.)), ("scale", nv(&[1., 1., 1.])), ("rotmode", n(1.)), ("parent", p(ob_rig)), ("partype", n(7.)), ("parsubstr", s("Child"))]);
        w.put(ob_prop, "OB", "Object", 1, o);
        let o = w.inst("Object", &[("id.name", s("OBCage")), ("type", n(22.)), ("data", p(lt)), ("scale", nv(&[2., 2., 2.])), ("rotmode", n(1.))]);
        w.put(ob_lat, "OB", "Object", 1, o);

        let scene = import_blend(&w.finish()).unwrap_or_else(|e| panic!("ptr {ptr} big {big}: {e}"));
        let ctx = format!("ptr {ptr} big {big} large {large}");
        let ob = |name: &str| &scene.objects[scene.find_object(name).unwrap_or_else(|| panic!("{ctx}: no {name}"))];

        // Mesh data.
        let body = ob("Body");
        let ObjectData::Mesh(mi) = body.data else { panic!("{ctx}") };
        let m = &scene.meshes[mi];
        assert_eq!(m.vertex_groups, vec!["Lower".to_string(), "Upper".to_string()], "{ctx}");
        assert_eq!(m.weight_opt(0, 0), Some(1.0));
        assert_eq!(m.weight_opt(1, 1), Some(0.5));
        assert_eq!(m.weight_opt(2, 0), None);
        assert_eq!(m.shape_keys.len(), 2);
        let sk = &m.shape_keys[1];
        assert_eq!((sk.name.as_str(), sk.value, sk.slider_min, sk.slider_max), ("Smile", 0.25, -1.0, 2.0));
        assert_eq!(sk.vertex_group.as_deref(), Some("Upper"));
        assert!(sk.mute);
        assert_eq!(sk.interpolation, KeyInterpolation::CatmullRom);
        assert!((sk.frame - 10.0).abs() < 1e-4);
        assert_eq!(sk.positions[1], Vec3::new(1.0, 0.0, 1.0));
        assert!(m.shape_key_settings.use_relative);
        assert_eq!(m.shape_key_settings.eval_time, 5.0);
        let ObjectData::Mesh(mi2) = ob("Old").data else { panic!("{ctx}") };
        assert_eq!(scene.meshes[mi2].vertex_groups, vec!["A".to_string(), "B".to_string()]);

        // Armature.
        let rig = ob("Rig");
        let ObjectData::Armature(ai) = rig.data else { panic!("{ctx}: rig is {:?}", rig.data) };
        let arm = &scene.armatures[ai];
        assert_eq!(scene.armature_names[ai], "Skel");
        assert_eq!(arm.bones.len(), 2);
        let (r, c) = (&arm.bones[0], &arm.bones[1]);
        assert_eq!((r.name.as_str(), c.name.as_str()), ("Root", "Child"));
        assert_eq!(c.parent, Some(0));
        assert!(c.use_connect && !c.use_inherit_rotation && c.use_deform);
        assert!(!r.use_deform);
        assert_eq!(c.inherit_scale, sex_3d::armature::InheritScale::None);
        assert_eq!(c.roll, 0.5);
        assert_eq!((r.envelope_distance, r.head_radius, r.bbone_segments), (0.5, 0.2, 3));
        assert_eq!(c.tail, Vec3::new(0.0, 0.0, 2.0));
        assert_eq!(arm.collections, vec!["Deform".to_string()]);
        assert_eq!(c.collections, vec!["Deform".to_string()]);
        // Pose.
        let pc = rig.pose.iter().find(|p| p.name == "Child").unwrap();
        assert_eq!(pc.rotation_mode, RotationMode::Euler(sex_3d::math::EulerOrder::YZX));
        assert_eq!(pc.rotation_euler, Vec3::new(0.1, 0.2, 0.3));
        assert_eq!(pc.scale, Vec3::new(1.0, 2.0, 1.0));
        assert_eq!(pc.ik.lock, [true, false, false]);
        assert_eq!(pc.ik.use_limit, [false, true, false]);
        assert_eq!(pc.ik.max, [0.0, 1.0, 0.0]);
        assert_eq!(pc.ik.stiffness[0], 0.25);
        assert_eq!(pc.constraints.len(), 1);
        let ik = &pc.constraints[0];
        assert_eq!((ik.name.as_str(), ik.influence, ik.owner_space), ("IK", 0.75, Space::Pose));
        match &ik.kind {
            ConstraintKind::Ik { target, pole_target, pole_angle, chain_count, iterations, use_tail, use_stretch, .. } => {
                assert_eq!(target.object, "Goal");
                assert_eq!(pole_target.as_ref().unwrap().object, "Prop");
                assert_eq!((*pole_angle, *chain_count, *iterations, *use_tail, *use_stretch), (0.5, 2, 200, true, false));
            }
            k => panic!("{ctx}: {k:?}"),
        }

        // Object constraints.
        let goal = ob("Goal");
        assert_eq!(goal.constraints.len(), 3);
        assert!(matches!(&goal.constraints[0].kind, ConstraintKind::TrackTo { target, track_axis: TrackAxis::NegZ, up_axis: 1, use_target_z: true } if target.object == "Prop"));
        match &goal.constraints[1].kind {
            ConstraintKind::ChildOf { target, use_location, use_rotation, use_scale, inverse } => {
                assert_eq!((target.object.as_str(), target.subtarget.as_str(), target.head_tail), ("Rig", "Root", 0.5));
                assert_eq!((*use_location, *use_rotation, *use_scale), ([true; 3], [false, false, true], [false; 3]));
                assert_eq!(inverse.translation_part(), Vec3::new(2.0, 0.0, 0.0));
            }
            k => panic!("{ctx}: {k:?}"),
        }
        assert_eq!(goal.constraints[1].target_space, Space::LocalWithParent);
        assert!(goal.constraints[2].mute);
        assert!(matches!(&goal.constraints[2].kind, ConstraintKind::LimitLocation { use_min: [true, false, false], use_max: [false, false, true], min, max } if min[0] == -1.0 && max[2] == 3.0));

        // Parenting.
        let prop = ob("Prop");
        assert_eq!((prop.parent_type, prop.parent_bone.as_str()), (ParentType::Bone, "Child"));
        assert_eq!(ob("Body").parent_type, ParentType::Armature);

        // Lattice.
        let cage = ob("Cage");
        let ObjectData::Lattice(li) = cage.data else { panic!("{ctx}") };
        let l = &scene.lattices[li];
        assert_eq!((l.points_u, l.points_v, l.points_w), (2, 2, 2));
        assert_eq!((l.interpolation_u, l.interpolation_v, l.interpolation_w), (KeyInterpolation::Linear, KeyInterpolation::Cardinal, KeyInterpolation::BSpline));
        assert!(l.use_outside);
        assert_eq!(l.points[7], Vec3::new(0.5, 0.5, 0.9));

        // Modifiers.
        let mods = &body.modifiers;
        assert_eq!(mods.len(), 5, "{ctx}: {mods:?}");
        assert_eq!(
            mods[0],
            Modifier::Armature {
                object: "Rig".into(),
                use_vertex_groups: true,
                use_bone_envelopes: false,
                use_deform_preserve_volume: true,
                use_multi_modifier: true,
                vertex_group: Some("Lower".into()),
                invert_vertex_group: true
            }
        );
        match &mods[1] {
            Modifier::Hook { object, subtarget, falloff_type, falloff_radius, center, matrix_inverse, strength, use_falloff_uniform, vertex_indices, vertex_group, .. } => {
                assert_eq!((object.as_str(), subtarget.as_str(), *falloff_type, *falloff_radius), ("Rig", "Child", Falloff::Smooth, 2.0));
                assert_eq!((*center, *strength, *use_falloff_uniform), ([0.0, 0.0, 1.0], 0.5, true));
                assert_eq!(matrix_inverse.translation_part(), Vec3::new(0.0, 0.0, -1.0));
                assert_eq!(vertex_indices, &vec![0, 2]);
                assert!(vertex_group.is_none());
            }
            m => panic!("{ctx}: {m:?}"),
        }
        assert_eq!(mods[2], Modifier::Smooth { factor: 0.5, iterations: 2, vertex_group: Some("Upper".into()), invert_vertex_group: true });
        match &mods[3] {
            Modifier::VertexWeightMix { vertex_group_a, vertex_group_b, default_weight_b, mix_mode, mix_set, invert_vertex_group_b, .. } => {
                assert_eq!((vertex_group_a.as_str(), vertex_group_b.as_str(), *default_weight_b), ("Upper", "Lower", 0.25));
                assert_eq!((*mix_mode, *mix_set, *invert_vertex_group_b), (WeightMixMode::Average, WeightMixSet::Or, true));
            }
            m => panic!("{ctx}: {m:?}"),
        }
        assert_eq!(mods[4], Modifier::Lattice { object: "Cage".into(), strength: 0.8, vertex_group: Some("Upper".into()), invert_vertex_group: true });

        // The imported scene evaluates and survives the polyops round trip.
        for i in 0..scene.objects.len() {
            let _ = scene.world_matrix(i);
            let _ = scene.evaluated_mesh(i);
        }
        let text = sex_3d::script::scene_to_polyops(&scene);
        let mut back = Scene::new();
        sex_3d::script::run(&mut back, &text).unwrap_or_else(|e| panic!("{ctx}: {e}\n{text}"));
        assert_eq!(back.objects, scene.objects, "{ctx}");
        assert_eq!(back.armatures, scene.armatures);
        assert_eq!(back.lattices, scene.lattices);
        assert_eq!(back.meshes, scene.meshes);
    }
}

fn node<'t>(t: &'t sex_3d::nodes::NodeTree, name: &str) -> &'t sex_3d::nodes::Node {
    t.nodes.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("no node {name}"))
}

fn input_float(n: &sex_3d::nodes::Node, id: &str) -> f32 {
    n.inputs[n.input(id).unwrap_or_else(|| panic!("{}: no input {id}", n.name))].value.as_float()
}

fn linked(t: &sex_3d::nodes::NodeTree, from: &str, to: &str, to_socket: &str) -> Option<bool> {
    let (f, k) = (t.find(from)?, t.find(to)?);
    let ts = t.nodes[k].input(to_socket)?;
    t.links.iter().find(|l| l.from_node == f && l.to_node == k && l.to_socket == ts).map(|l| l.muted)
}

#[test]
fn shader_node_trees_import() {
    use sex_3d::nodes::{Prop, Value};
    for c in ALL {
        let what = format!("{c:?}");
        let s = import_blend(&blendgen::nodes_file(c)).unwrap_or_else(|e| panic!("{what}: {e}"));
        let m = s.materials.iter().find(|m| m.name == "Nodes").unwrap_or_else(|| panic!("{what}"));
        let t = m.node_tree.as_ref().unwrap_or_else(|| panic!("{what}: no tree"));
        assert_eq!(t.nodes.len(), 13, "{what}");

        let out = node(t, "Material Output");
        assert_eq!(out.prop("is_active_output"), Some(&Prop::Bool(true)), "{what}");
        assert_eq!(linked(t, "Principled BSDF", "Material Output", "Surface"), Some(false), "{what}");

        let p = node(t, "Principled BSDF");
        assert_eq!(p.label, "Shiny", "{what}");
        assert_eq!(p.location, [-300.0, 120.0], "{what}");
        assert_eq!(p.width, 260.0, "{what}");
        assert!(approx(input_float(p, "Metallic"), 0.6), "{what}");
        // Pre-4.0 "Specular" lands on "Specular IOR Level".
        assert!(approx(input_float(p, "Specular IOR Level"), 0.25), "{what}");
        assert_eq!(linked(t, "Math", "Principled BSDF", "Roughness"), Some(false), "{what}");
        assert_eq!(linked(t, "Mix", "Principled BSDF", "Base Color"), Some(false), "{what}");
        assert_eq!(linked(t, "Group", "Principled BSDF", "Coat Weight"), Some(true), "{what}: muted link");

        let im = node(t, "Image Texture");
        assert_eq!(im.enum_prop("projection"), "BOX", "{what}");
        assert_eq!(im.enum_prop("interpolation"), "Closest", "{what}");
        assert_eq!(im.enum_prop("extension"), "MIRROR", "{what}");
        assert_eq!(im.prop("projection_blend"), Some(&Prop::Float(0.25)), "{what}");
        let Some(Prop::Id(Some(ii))) = im.prop("image") else { panic!("{what}: no image") };
        let img = &s.images[*ii];
        assert_eq!((img.name.as_str(), img.width, img.height), ("tex.png", 2, 2), "{what}");
        for (px, e) in img.pixels.iter().zip(blendgen::NODE_IMAGE) {
            for k in 0..4 {
                assert!(approx(px[k], e[k] as f32 / 255.0), "{what}: {:?}", img.pixels);
            }
        }
        assert_eq!(linked(t, "Image Texture", "Mix", "A_Color"), Some(false), "{what}");
        assert_eq!(linked(t, "Texture Coordinate", "Image Texture", "Vector"), Some(false), "{what}");

        let mix = node(t, "Mix");
        assert_eq!((mix.enum_prop("data_type"), mix.enum_prop("blend_type")), ("RGBA", "MULTIPLY"), "{what}");
        assert!(approx(input_float(mix, "Factor_Float"), 0.75), "{what}");
        assert_eq!(mix.inputs[mix.input("B_Color").unwrap()].value, Value::Color([1.0, 0.5, 0.25, 1.0]), "{what}");

        let math = node(t, "Math");
        assert_eq!(math.enum_prop("operation"), "MULTIPLY", "{what}");
        assert!(math.bool_prop("use_clamp"), "{what}");
        assert!(approx(input_float(math, "Value_001"), 3.0), "{what}");

        let ramp = node(t, "Color Ramp");
        let Some(Prop::Ramp(r)) = ramp.prop("color_ramp") else { panic!("{what}: no ramp") };
        assert_eq!(r.interpolation, "CONSTANT", "{what}");
        assert_eq!(r.elements, vec![(0.2, [0.0, 0.0, 1.0, 1.0]), (0.8, [1.0, 0.0, 0.0, 1.0])], "{what}");
        // A node type we do not know keeps its sockets and links.
        let fut = node(t, "Future");
        assert_eq!((fut.inputs.len(), fut.outputs.len()), (1, 1), "{what}");
        assert!(approx(input_float(fut, "Thing"), 2.0), "{what}");
        assert_eq!(linked(t, "Future", "Color Ramp", "Fac"), Some(false), "{what}");

        let noise = node(t, "Noise Texture");
        assert_eq!((noise.enum_prop("noise_dimensions"), noise.enum_prop("noise_type")), ("2D", "FBM"), "{what}");
        assert!(noise.bool_prop("normalize"), "{what}");
        assert!(approx(input_float(noise, "Scale"), 7.0), "{what}");
        assert_eq!(noise.parent, t.find("Frame"), "{what}");
        assert_eq!(node(t, "Frame").label, "Textures", "{what}");

        let inv = node(t, "Invert");
        assert!(inv.mute && inv.hide, "{what}");
        assert_eq!(inv.color, Some([0.1, 0.2, 0.3]), "{what}");
        assert!(inv.inputs[inv.input("Fac").unwrap()].hide, "{what}");

        let Some(Prop::Curve(cm)) = node(t, "RGB Curves").prop("mapping") else { panic!("{what}: no curves") };
        assert_eq!(cm.curves.len(), 4, "{what}");
        // The combined curve is file cm[3], our curves[0].
        assert_eq!(cm.curves[0], vec![[0.0, 0.0], [1.0, 0.5]], "{what}");
        assert_eq!(cm.curves[1], vec![[0.0, 0.0], [1.0, 1.0]], "{what}");

        let gn = node(t, "Group");
        let Some(Prop::Id(Some(gi))) = gn.prop("node_tree") else { panic!("{what}: no group") };
        let g = &s.node_groups[*gi];
        assert_eq!(g.name, "Half", "{what}");
        assert_eq!((g.inputs.len(), g.outputs.len()), (1, 1), "{what}");
        assert_eq!((g.inputs[0].name.as_str(), g.outputs[0].name.as_str()), ("In", "Out"), "{what}");
        assert_eq!(g.inputs[0].default, Value::Float(1.0), "{what}");
        assert_eq!((gn.inputs[0].name.as_str(), gn.outputs[0].name.as_str()), ("In", "Out"), "{what}");
        assert!(approx(gn.inputs[0].value.as_float(), 0.25), "{what}");
        assert_eq!(g.nodes.len(), 3, "{what}");
        assert_eq!(g.links.len(), 2, "{what}: {:?}", g.links);
        assert_eq!(linked(g, "Math", "Group Output", "Out"), Some(false), "{what}");

        let names = blendgen::names(&s);
        assert_eq!(node(t, "Texture Coordinate").prop("object"), Some(&Prop::Id(names.get("Empty").copied())), "{what}");
        assert!(names.contains_key("Empty"), "{what}");
        assert!(node(t, "Texture Coordinate").prop("__object_ptr").is_none(), "{what}");

        let wt = s.world.node_tree.as_ref().unwrap_or_else(|| panic!("{what}: no world tree"));
        let bg = node(wt, "Background");
        assert_eq!(bg.inputs[bg.input("Color").unwrap()].value, Value::Color([0.2, 0.3, 0.4, 1.0]), "{what}");
        assert!(approx(input_float(bg, "Strength"), 2.0), "{what}");
        assert_eq!(linked(wt, "Background", "World Output", "Surface"), Some(false), "{what}");
    }
}

#[test]
fn imported_node_trees_render() {
    use sex_3d::render::{self, RenderSettings};
    let mut s = import_blend(&blendgen::nodes_file(ALL[7])).unwrap();
    let mi = s.materials.iter().position(|m| m.name == "Nodes").unwrap();
    let o = s.add_mesh_object("Plane", sex_3d::primitives::plane(4.0));
    s.objects[o].materials.push(mi);
    let mut cam = sex_3d::scene::Object::new("Cam", sex_3d::scene::ObjectData::Camera(sex_3d::scene::Camera::default()));
    cam.transform.location = Vec3::new(0.0, 0.0, 3.0);
    let c = s.add_object(cam);
    s.active_camera = Some(c);
    let st = RenderSettings { width: 12, height: 12, samples: 4, threads: 1, denoise: false, ..RenderSettings::default() };
    let img = render::render(&s, &st).color;
    assert!(img.pixels.iter().all(|p| p.iter().all(|v| v.is_finite())));
    // The world tree lights the plane: its pixels are not black.
    let mean: f32 = img.pixels.iter().map(|p| p[0] + p[1] + p[2]).sum::<f32>() / img.pixels.len() as f32;
    assert!(mean > 0.01, "{mean}");
}

#[test]
fn corrupted_node_files_never_panic() {
    let mut rng = Rng(0x5151_A3C1_0F0F_7777);
    for c in [ALL[1], ALL[4], ALL[8], ALL[11]] {
        let full = blendgen::nodes_file(c);
        for _ in 0..250 {
            let mut b = full.clone();
            for _ in 0..1 + rng.next() % 3 {
                let at = (rng.next() % b.len() as u64) as usize;
                b[at] ^= 1 << (rng.next() % 8);
            }
            let at = (rng.next() % (b.len() as u64 - 8)) as usize;
            b[at..at + 8].copy_from_slice(&rng.next().to_le_bytes());
            let _ = import_blend(&b);
        }
    }
}

/// Imported trees survive the polyops text round trip (images are not
/// written, so the Image Texture's image comes back empty).
#[test]
fn imported_node_trees_round_trip_through_polyops() {
    use sex_3d::nodes::Prop;
    for c in [ALL[0], ALL[5], ALL[10]] {
        let mut s = import_blend(&blendgen::nodes_file(c)).unwrap();
        let text = sex_3d::script::scene_to_polyops(&s);
        let mut back = Scene::new();
        sex_3d::script::run(&mut back, &text).unwrap_or_else(|e| panic!("{c:?}: {e}\n{text}"));
        for m in &mut s.materials {
            for n in m.node_tree.iter_mut().flat_map(|t| t.nodes.iter_mut()) {
                if n.idname == "ShaderNodeTexImage" {
                    n.set_prop("image", Prop::Id(None));
                }
            }
        }
        let nodes = |s: &Scene| s.materials.iter().find(|m| m.name == "Nodes").and_then(|m| m.node_tree.clone());
        assert_eq!(nodes(&back), nodes(&s), "{c:?}");
        assert_eq!(back.node_groups, s.node_groups, "{c:?}");
        assert_eq!(back.world.node_tree, s.world.node_tree, "{c:?}");
    }
}

/// A Nodes modifier and its geometry node group: the tree kind, node
/// settings from storage structs, the multi-input Join, interface inputs
/// set by ID property, and the evaluated result.
#[test]
fn geometry_nodes_modifier() {
    use sex_3d::nodes::{Prop, TreeKind, Value};
    for c in ALL.into_iter().filter(|c| matches!(c.layout, Layout::Attr36 | Layout::Attr42 | Layout::Attr50)) {
        let what = format!("{c:?}");
        let s = import_blend(&blendgen::geonodes_file(c)).unwrap_or_else(|e| panic!("{what}: {e}"));
        let o = blendgen::names(&s)["GN"];
        assert_eq!(s.objects[o].modifiers, vec![Modifier::Nodes { group: "Geometry Nodes".into(), inputs: vec![("Height".into(), Value::Float(blendgen::GN_HEIGHT))] }], "{what}");
        let g = s.node_groups.iter().find(|g| g.name == "Geometry Nodes").unwrap_or_else(|| panic!("{what}: no group"));
        assert_eq!(g.kind, TreeKind::Geometry, "{what}");
        assert_eq!(g.inputs.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Geometry", "Height"], "{what}");
        let node = |name: &str| &g.nodes[g.find(name).unwrap_or_else(|| panic!("{what}: no {name}"))];
        assert_eq!(node("Extrude Mesh").prop("mode"), Some(&Prop::Enum("FACES".into())), "{what}");
        assert_eq!(node("Cylinder").prop("fill_type"), Some(&Prop::Enum("TRIANGLE_FAN".into())), "{what}");
        assert_eq!(g.links.len(), 5, "{what}: {:?}", g.links);
        // The cube's 6 faces extruded one by one (9 + 24 vertices, 6 + 24
        // faces) and a 32-vertex fan-capped cylinder (66 vertices, 96 faces).
        let m = s.evaluated_mesh(o).unwrap();
        assert_eq!((m.positions.len(), m.faces.len()), (33 + 66, 30 + 96), "{what}");
        let top = m.positions.iter().filter(|p| approx(p.z, 1.0 + blendgen::GN_HEIGHT)).count();
        assert_eq!(top, 4, "{what}");
        // And it round-trips through polyops.
        let text = sex_3d::script::scene_to_polyops(&s);
        let mut back = Scene::new();
        sex_3d::script::run(&mut back, &text).unwrap_or_else(|e| panic!("{what}: {e}\n{text}"));
        assert_eq!(back.node_groups, s.node_groups, "{what}");
        assert_eq!(back.objects[blendgen::names(&back)["GN"]].modifiers, s.objects[o].modifiers, "{what}");
    }
}

#[test]
fn compositor_tree_imports() {
    use sex_3d::nodes::{Prop, TreeKind, Value};
    for c in ALL {
        let what = format!("{c:?}");
        let s = import_blend(&blendgen::compositor_file(c)).unwrap_or_else(|e| panic!("{what}: {e}"));
        let t = s.compositor.as_ref().unwrap_or_else(|| panic!("{what}: no compositor"));
        assert_eq!(t.kind, TreeKind::Compositor, "{what}");
        assert_eq!(t.nodes.len(), 9, "{what}");
        let e = |n: &sex_3d::nodes::Node, k: &str| n.enum_prop(k).to_string();
        let f = |n: &sex_3d::nodes::Node, k: &str| match n.prop(k) {
            Some(Prop::Float(v)) => *v,
            other => panic!("{what}: {k} {other:?}"),
        };
        let i = |n: &sex_3d::nodes::Node, k: &str| match n.prop(k) {
            Some(Prop::Int(v)) => *v,
            other => panic!("{what}: {k} {other:?}"),
        };
        let bo = |n: &sex_3d::nodes::Node, k: &str| matches!(n.prop(k), Some(Prop::Bool(true)));
        let gl = node(t, "Glare");
        assert_eq!((e(gl, "glare_type"), e(gl, "quality")), ("BLOOM".into(), "LOW".into()), "{what}");
        assert_eq!((i(gl, "iterations"), i(gl, "size"), i(gl, "streaks")), (5, 7, 6), "{what}");
        assert!(approx(f(gl, "threshold"), 0.75) && approx(f(gl, "mix"), -0.25) && approx(f(gl, "fade"), 0.8) && !bo(gl, "use_rotate_45"), "{what}");
        let bl = node(t, "Blur");
        assert_eq!((e(bl, "filter_type"), e(bl, "aspect_correction")), ("FAST_GAUSS".into(), "X".into()), "{what}");
        assert_eq!((i(bl, "size_x"), i(bl, "size_y")), (3, 5), "{what}");
        assert!(bo(bl, "use_relative") && bo(bl, "use_extended_bounds") && !bo(bl, "use_variable_size") && bo(bl, "use_gamma_correction"), "{what}");
        assert!(approx(f(bl, "factor_x"), 10.0), "{what}");
        let mix = node(t, "Mix");
        assert_eq!(e(mix, "blend_type"), "ADD", "{what}");
        assert!(bo(mix, "use_clamp") && !bo(mix, "use_alpha"), "{what}");
        assert!(approx(input_float(mix, "Fac"), 0.6), "{what}");
        assert!(!bo(node(t, "Composite"), "use_alpha"), "{what}");
        let key = node(t, "Channel Key");
        assert_eq!((e(key, "color_space"), e(key, "matte_channel"), e(key, "limit_method"), e(key, "limit_channel")), ("HSV".into(), "R".into(), "SINGLE".into(), "B".into()), "{what}");
        assert!(approx(f(key, "limit_max"), 0.9) && approx(f(key, "limit_min"), 0.1), "{what}");
        let cb = node(t, "Color Balance");
        assert_eq!(e(cb, "correction_method"), "OFFSET_POWER_SLOPE", "{what}");
        assert_eq!(cb.prop("slope"), Some(&Prop::Color([1.5, 1.0, 1.0, 1.0])), "{what}");
        assert_eq!(cb.prop("power"), Some(&Prop::Color([1.0, 1.0, 2.0, 1.0])), "{what}");
        let cr = node(t, "Crop");
        assert!(cr.mute && bo(cr, "use_crop_size"), "{what}");
        assert_eq!((i(cr, "min_x"), i(cr, "max_x"), i(cr, "min_y"), i(cr, "max_y")), (2, 30, 4, 40), "{what}");
        let ma = node(t, "Math");
        assert_eq!(e(ma, "operation"), "POWER", "{what}");
        assert!(bo(ma, "use_clamp"), "{what}");
        assert_eq!(ma.inputs[1].value, Value::Float(3.0), "{what}");
        for (from, to, sock) in [("Render Layers", "Blur", "Image"), ("Blur", "Glare", "Image"), ("Glare", "Mix", "Image"), ("Mix", "Composite", "Image")] {
            assert_eq!(linked(t, from, to, sock), Some(false), "{what}: {from} -> {to}");
        }
        assert!(t.links.iter().any(|l| t.nodes[l.to_node].name == "Mix" && t.nodes[l.to_node].inputs[l.to_socket].identifier == "Image_001"), "{what}");
    }
}

#[test]
fn imported_compositor_runs() {
    use sex_3d::render::{render, RenderSettings};
    let mut s = import_blend(&blendgen::compositor_file(ALL[7])).unwrap();
    s.compositor.as_mut().unwrap().nodes.iter_mut().for_each(|n| {
        if n.idname == "CompositorNodeGlare" {
            n.set_prop("quality", sex_3d::nodes::Prop::Enum("HIGH".into()));
        }
    });
    let settings = RenderSettings { width: 16, height: 12, samples: 1, denoise: false, threads: 1, ..Default::default() };
    let r = render(&s, &settings);
    let ctx = sex_3d::compositor::Context::new(&s, Some(&r), (16, 12));
    let out = sex_3d::compositor::composite(s.compositor.as_ref().unwrap(), &ctx);
    let c = out.composite.expect("composite");
    assert_eq!((c.width, c.height), (16, 12));
    assert!(c.pixels.iter().all(|p| p.iter().all(|v| v.is_finite())));
}
