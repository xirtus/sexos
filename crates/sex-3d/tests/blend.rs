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
