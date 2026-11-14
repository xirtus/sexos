// SPDX-License-Identifier: GPL-3.0-only
//! polyops: every command, errors and quoting, the scene text round trip, and
//! the Python `polyamour`/`bpy` package end to end on the host's python3.
use sex_3d::math::{Quat, Vec3};
use sex_3d::modifiers::Modifier;
use sex_3d::scene::{LightKind, ObjectData, Scene};
use sex_3d::script::{self, quote, run, scene_to_polyops, RunReport, ScriptError, Session};
use std::path::{Path, PathBuf};
use std::process::Command;

fn ok(scene: &mut Scene, src: &str) -> RunReport {
    match run(scene, src) {
        Ok(r) => r,
        Err(e) => panic!("{e}\n--- program ---\n{src}"),
    }
}

fn err(src: &str) -> ScriptError {
    let mut s = Scene::startup();
    let before = s.clone();
    let e = run(&mut s, src).expect_err("program should fail");
    assert_eq!(s, before, "a failed run must leave the scene untouched");
    e
}

fn obj<'a>(s: &'a Scene, name: &str) -> &'a sex_3d::Object {
    &s.objects[s.find_object(name).unwrap_or_else(|| panic!("no object {name}"))]
}

fn close(a: Vec3, b: Vec3) -> bool {
    a.approx_eq(b, 1e-5)
}

#[test]
fn primitives_transforms_and_names() {
    let mut s = Scene::startup();
    let r = ok(
        &mut s,
        "polyops 1\n\
         # comment line\n\
         add_primitive cube name=Box size=1 location=1,2,3 as=$b   # trailing comment\n\
         add_primitive cube   # takes the next unique name\n\
         add_primitive uv_sphere segments=8 rings=4 radius=0.5\n\
         add_primitive ico_sphere subdivisions=1\n\
         add_primitive cylinder segments=6 depth=3\n\
         add_primitive cone segments=5 radius1=1 radius2=0.5\n\
         add_primitive torus major_segments=8 minor_segments=4\n\
         add_primitive plane size=4\n\
         add_primitive grid x_subdivisions=3 y_subdivisions=2\n\
         set $b.rotation_euler 0 0 1.5707964\n\
         set Box.scale 2\n\
         set \"Cube.001.location\" 0,0,5\n\
         end\n",
    );
    assert_eq!(r.version, 1);
    assert!(r.complete);
    assert_eq!(r.commands, 13);
    assert_eq!(r.created, ["Box", "Cube.001", "Sphere", "Icosphere", "Cylinder", "Cone", "Torus", "Plane", "Grid"]);
    let b = obj(&s, "Box");
    assert!(close(b.transform.location, Vec3::new(1.0, 2.0, 3.0)));
    assert_eq!(b.transform.scale, Vec3::splat(2.0));
    assert!(close(b.transform.rotation.rotate(Vec3::X), Vec3::Y));
    assert!(close(obj(&s, "Cube.001").transform.location, Vec3::new(0.0, 0.0, 5.0)));
    let mesh = |n: &str| match obj(&s, n).data {
        ObjectData::Mesh(m) => &s.meshes[m],
        _ => panic!("{n} has no mesh"),
    };
    assert_eq!(mesh("Box").bounds().size(), Vec3::ONE);
    assert_eq!(mesh("Cylinder").vertex_count(), 12);
    assert_eq!(mesh("Grid").face_count(), 6);
    assert_eq!(mesh("Torus").face_count(), 32);
    // Blender's add rule: the last added object is active and alone selected.
    let grid = s.find_object("Grid").unwrap();
    assert_eq!(s.active_object, Some(grid));
    assert_eq!(s.selected, vec![grid]);
}

#[test]
fn meshes_text_and_base64() {
    let mut s = Scene::new();
    ok(
        &mut s,
        "polyops 1\n\
         add_mesh name=Quad verts=0,0,0,1,0,0,1,1,0,0,1,0 faces=0,1,2,3 uvs=0,0,1,0,1,1,0,1 smooth=1 mat=1\n\
         add_mesh Wire verts=0,0,0,1,0,0,2,0,0 edges=0,1;1,2\n\
         mesh_new Shared as=$m verts=0,0,0,1,0,0,0,1,0 faces=0,1,2\n\
         add_object A data=$m\n\
         add_object B data=Shared\n\
         add_object Nothing data=none\n\
         add_empty Null location=0,0,1\n",
    );
    let q = &s.meshes[0];
    assert_eq!(q.face_count(), 1);
    assert!(q.has_uvs());
    assert!(q.faces[0].smooth);
    assert_eq!(q.faces[0].material, 1);
    assert_eq!(s.meshes[1].loose_edges, vec![[0, 1], [1, 2]]);
    assert_eq!(obj(&s, "A").data, obj(&s, "B").data, "two objects share one mesh");
    assert_eq!(obj(&s, "Nothing").data, ObjectData::Empty);
    assert_eq!(obj(&s, "Null").data, ObjectData::Empty);

    // A big mesh written by scene_to_polyops uses b64: arrays and reads back exactly.
    let mut big = Scene::new();
    big.add_mesh_object("Ball", sex_3d::primitives::uv_sphere(16, 8, 1.3));
    let text = scene_to_polyops(&big);
    assert!(text.contains("verts=b64:"), "{text}");
    assert!(text.contains("faces=b64:"));
    let mut back = Scene::new();
    ok(&mut back, &text);
    assert_eq!(back.meshes, big.meshes);

    // mesh_set replaces, mesh_edit changes in place.
    ok(
        &mut s,
        "polyops 1\n\
         mesh_set Quad verts=0,0,0,2,0,0,0,2,0 faces=0,1,2\n\
         mesh_edit Quad translate_verts 1 0 0\n\
         mesh_edit Quad scale_verts 2 indices=0\n\
         mesh_edit Quad rotate_verts 0 0 3.14159265 center=1,0,0 indices=2\n\
         mesh_edit Quad set_verts indices=1 verts=9,9,9\n\
         mesh_edit Quad shade_flat\n\
         mesh_edit Quad flip_normals\n\
         mesh_edit Quad set_material 2 faces=0\n\
         mesh_edit Wire weld 5\n",
    );
    let q = &s.meshes[0];
    assert_eq!(q.vertex_count(), 3);
    assert!(close(q.positions[0], Vec3::new(2.0, 0.0, 0.0)));
    assert_eq!(q.positions[1], Vec3::new(9.0, 9.0, 9.0));
    assert!(close(q.positions[2], Vec3::new(1.0, -2.0, 0.0)));
    assert_eq!(q.faces[0].verts, vec![2, 1, 0]);
    assert_eq!(q.faces[0].material, 2);
    assert!(!q.faces[0].smooth);
    assert_eq!(s.meshes[1].vertex_count(), 1);
}

#[test]
fn materials_modifiers_parents_collections() {
    let mut s = Scene::startup();
    let r = ok(
        &mut s,
        "polyops 1\n\
         material_new Red base_color=1,0,0 roughness=0.3 as=$red\n\
         material_new Red metallic=1\n\
         material_set Red.001 base_color=0,0,1,0.5 emission=1,1,0 emission_strength=2 ior=1.33 transmission=0.9 double_sided=yes name=Blue\n\
         material_assign Cube $red\n\
         material_assign Cube Blue slot=3\n\
         modifier_add Cube SUBSURF levels=2\n\
         modifier_add Cube Mirror y=1 merge=off\n\
         modifier_add Cube array count=3 relative=0,0,1.5\n\
         modifier_add Cube Triangulate\n\
         modifier_add Cube Bevel width=0.05 segments=3\n\
         modifier_add Cube Solidify thickness=0.2\n\
         modifier_add Cube Weld distance=0.01\n\
         modifier_add Cube Displace strength=0.3\n\
         modifier_add Cube Smooth factor=1 iterations=4\n\
         modifier_set Cube 0 levels=3 simple=1\n\
         modifier_remove Cube 3\n\
         add_empty Pivot as=$p\n\
         parent Cube $p\n\
         set $p.location 0 0 10\n\
         collection_new Props as=$c\n\
         collection_new Sub parent=$c visible=0\n\
         link Cube Sub\n\
         set Light.collection Props\n\
         collection_set Props name=Stuff\n",
    );
    assert_eq!(r.replies, Vec::<String>::new());
    let red = &s.materials[1];
    assert_eq!(red.name, "Red");
    assert_eq!(red.base_color, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(red.roughness, 0.3);
    let blue = &s.materials[2];
    assert_eq!(blue.name, "Blue");
    assert_eq!(blue.base_color, [0.0, 0.0, 1.0, 0.5]);
    assert_eq!(blue.metallic, 1.0);
    assert!(blue.double_sided);
    assert_eq!((blue.ior, blue.transmission, blue.emission_strength), (1.33, 0.9, 2.0));
    let cube = obj(&s, "Cube");
    assert_eq!(cube.materials, vec![0, 1, 2, 2], "slot 3 pads with the material");
    assert_eq!(
        cube.modifiers,
        vec![
            Modifier::Subdivision { levels: 3, simple: true },
            Modifier::Mirror { x: true, y: true, z: false, merge: false, merge_distance: 0.001 },
            Modifier::Array { count: 3, relative: [0.0, 0.0, 1.5], constant: [0.0; 3], merge: false },
            Modifier::Bevel { width: 0.05, segments: 3, angle: 30f32.to_radians() },
            Modifier::Solidify { thickness: 0.2, offset: -1.0 },
            Modifier::Weld { distance: 0.01 },
            Modifier::Displace { strength: 0.3 },
            Modifier::Smooth { factor: 1.0, iterations: 4 },
        ]
    );
    let ci = s.find_object("Cube").unwrap();
    assert!(close(s.world_matrix(ci).transform_point(Vec3::ZERO), Vec3::new(0.0, 0.0, 10.0)));
    assert_eq!(s.collections[cube.collection].name, "Sub");
    assert!(!s.is_visible(ci), "Sub is hidden");
    assert_eq!(s.collections[obj(&s, "Light").collection].name, "Stuff");
    assert_eq!(s.collections[2].parent, Some(1));

    // keep_transform keeps the world position; none clears the parent.
    ok(&mut s, "polyops 1\nset Cube.location 1 0 0\nparent Cube none keep_transform=1\n");
    assert!(close(obj(&s, "Cube").transform.location, Vec3::new(1.0, 0.0, 10.0)));
    assert_eq!(obj(&s, "Cube").parent, None);
    ok(&mut s, "polyops 1\nparent Cube Pivot keep_transform=1\nmaterial_clear Cube\nmodifier_clear Cube\n");
    assert!(close(obj(&s, "Cube").transform.location, Vec3::new(1.0, 0.0, 0.0)));
    assert!(obj(&s, "Cube").materials.is_empty() && obj(&s, "Cube").modifiers.is_empty());
}

#[test]
fn lights_cameras_selection_delete_world_render() {
    let mut s = Scene::startup();
    let r = ok(
        &mut s,
        "polyops 1\n\
         light_add sun power=3 color=1,0.9,0.8 rotation=0.5,0,0\n\
         light_add spot name=Key angle=0.6 blend=0.3 location=0,0,4\n\
         light_add area size=2 as=$a\n\
         set $a.power 250\n\
         set Key.color 0 1 0\n\
         camera_add name=Cam2 lens=35 active=1\n\
         set Cam2.far 50\n\
         set Camera.ortho_scale 7\n\
         select Cube Light\n\
         select_add Sun\n\
         deselect Light\n\
         active Sun\n\
         world color=0.1,0.2,0.3 strength=0.5\n\
         scene name=\"My Scene\" frame_start=10 frame_end=20 fps=30\n\
         render resolution_x=1280 resolution_y=720 engine=CYCLES filepath=\"/out/a b.png\"\n\
         delete Light Key\n",
    );
    assert!(!r.complete, "no end line");
    assert_eq!(r.render.get("resolution_x").map(String::as_str), Some("1280"));
    assert_eq!(r.render.get("filepath").map(String::as_str), Some("/out/a b.png"));
    match obj(&s, "Sun").data {
        ObjectData::Light(l) => {
            assert_eq!(l.kind, LightKind::Sun);
            assert_eq!(l.power, 3.0);
        }
        _ => panic!(),
    }
    match obj(&s, "Area").data {
        ObjectData::Light(l) => assert_eq!((l.kind, l.power), (LightKind::Area { size: 2.0 }, 250.0)),
        _ => panic!(),
    }
    match obj(&s, "Cam2").data {
        ObjectData::Camera(c) => {
            assert!((c.fov_y - 2.0 * (18.0f32 / 35.0 * 9.0 / 16.0).atan()).abs() < 1e-6);
            assert_eq!(c.far, 50.0);
        }
        _ => panic!(),
    }
    match obj(&s, "Camera").data {
        ObjectData::Camera(c) => assert_eq!(c.ortho_scale, Some(7.0)),
        _ => panic!(),
    }
    assert_eq!(s.active_camera, s.find_object("Cam2"));
    assert!(s.find_object("Light").is_none() && s.find_object("Key").is_none());
    let names: Vec<&str> = s.selected.iter().map(|&i| s.objects[i].name.as_str()).collect();
    assert_eq!(names, ["Cube", "Sun"]);
    assert_eq!(s.active_object, s.find_object("Sun"));
    assert_eq!(s.world.color, Vec3::new(0.1, 0.2, 0.3));
    assert_eq!((s.name.as_str(), s.frame_start, s.frame_end, s.fps), ("My Scene", 10, 20, 30.0));

    ok(&mut s, "polyops 1\nselect_all\nactive none\nactive_camera none\n");
    assert_eq!(s.selected.len(), s.objects.len());
    assert_eq!((s.active_object, s.active_camera), (None, None));
    ok(&mut s, "polyops 1\ndeselect\n");
    assert!(s.selected.is_empty());
}

#[test]
fn queries_and_request_ids() {
    let mut s = Scene::startup();
    let r = ok(
        &mut s,
        "polyops 1\n\
         ping\n\
         @a1 add_primitive cube as=$c location=1,2,3\n\
         @a2 get $c.location\n\
         get Cube.type\n\
         get Cube.vertex_count\n\
         get Camera.lens\n\
         get Light.power\n\
         @x material_new \"Shiny Red\"\n\
         material_assign $c \"Shiny Red\"\n\
         get Cube.001.materials\n\
         get Cube.materials\n\
         get Cube.parent\n\
         set $c.name \"New Name\"\n\
         get \"New Name.name\"\n\
         set $c.rotation_quaternion 0.70710677 0 0 0.70710677\n\
         get $c.rotation_euler\n\
         list objects\n\
         list collections\n\
         exists Cube\n\
         exists Nope\n\
         @done end\n",
    );
    // The Euler angle comes back through quaternion maths whose last bit
    // depends on the CPU's float contraction; compare it numerically.
    let mut replies = r.replies.clone();
    let euler: Vec<f32> = replies[12].trim_start_matches("ok ").split(' ').map(|v| v.parse().unwrap()).collect();
    assert!(euler[0].abs() < 1e-6 && euler[1].abs() < 1e-6 && (euler[2] - core::f32::consts::FRAC_PI_2).abs() < 1e-6, "{euler:?}");
    replies[12] = "ok 0 0 1.5707964".into();
    assert_eq!(
        replies,
        [
            "ok polyops 1",
            "@a1 ok Cube.001",
            "@a2 ok 1 2 3",
            "ok MESH",
            "ok 8",
            "ok 49.999996",
            "ok 1000",
            "@x ok \"Shiny Red\"",
            "ok \"Shiny Red\"",
            "ok Material",
            "ok none",
            "ok \"New Name\"",
            "ok 0 0 1.5707964",
            "ok Cube Light Camera \"New Name\"",
            "ok Collection",
            "ok 1",
            "ok 0",
            "@done ok",
        ]
    );
    // A live channel feeds lines one at a time and takes replies as they come.
    let mut sess = Session::new();
    let mut s2 = Scene::startup();
    sess.exec_line(&mut s2, "polyops 1").unwrap();
    sess.exec_line(&mut s2, "@1 exists Camera").unwrap();
    assert_eq!(sess.take_replies(), ["@1 ok 1"]);
    sess.exec_line(&mut s2, "@2 get Camera.near").unwrap();
    assert_eq!(sess.take_replies(), ["@2 ok 0.1"]);
    assert!(sess.finish().unwrap().replies.is_empty());
}

#[test]
fn quoting_round_trips() {
    for name in ["plain", "with space", "q\"uote", "back\\slash", "new\nline", "tab\there", "none", "$1", "#hash", "@at", "a=b", "", "émoji 🎲", "Cube.001"] {
        let mut s = Scene::new();
        let r = ok(&mut s, &format!("polyops 1\nadd_empty {}\nlist objects\n", quote(name)));
        let expected = if name.is_empty() { "Empty" } else { name };
        assert_eq!(s.objects[0].name, expected, "{name:?}");
        assert_eq!(r.replies, [format!("ok {}", quote(expected))]);
    }
    // Mixed bare and quoted pieces join into one token; `=` inside quotes is text.
    let mut s = Scene::new();
    ok(&mut s, "polyops 1\nadd_empty name=a\"b c\"d\nadd_empty \"x=y\"\n");
    assert_eq!(s.objects[0].name, "ab cd");
    assert_eq!(s.objects[1].name, "x=y");
}

#[test]
fn errors_name_their_line() {
    let cases: &[(&str, usize, &str)] = &[
        ("", 1, "missing 'polyops 1' header"),
        ("# only a comment\n", 1, "missing 'polyops 1' header"),
        ("add_primitive cube\n", 1, "the first command must be 'polyops 1'"),
        ("polyops 2\n", 1, "polyops version 2 is not supported"),
        ("polyops 1\npolyops 1\n", 2, "header appears twice"),
        ("polyops 1\n\nfrobnicate\n", 3, "unknown command 'frobnicate'"),
        ("polyops 1\nadd_primitive cube sise=2\n", 2, "unknown option 'sise' for add_primitive"),
        ("polyops 1\nadd_primitive cube size=2 size=3\n", 2, "option 'size' given twice"),
        ("polyops 1\nadd_primitive blob\n", 2, "unknown primitive 'blob'"),
        ("polyops 1\nadd_primitive cube size=big\n", 2, "size: 'big' is not a finite number"),
        ("polyops 1\nadd_primitive cube size=inf\n", 2, "not a finite number"),
        ("polyops 1\nset Cube.location 1 2\n", 2, "location needs 3 numbers, got 2"),
        ("polyops 1\nset Cube.wobble 1\n", 2, "unknown property 'wobble'"),
        ("polyops 1\nset Nope.location 1 2 3\n", 2, "no object named 'Nope'"),
        ("polyops 1\nset Cube 1 2 3\n", 2, "'Cube' is not OBJECT.property"),
        ("polyops 1\nset $9.location 1 2 3\n", 2, "handle $9 is not bound"),
        ("polyops 1\nadd_empty \"unterminated\n", 2, "unterminated quoted string"),
        ("polyops 1\nadd_empty \"bad\\q\"\n", 2, "unknown escape \\q"),
        ("polyops 1\nadd_mesh verts=0,0,0,1,0,0 faces=0,1,2\n", 2, "face 0 uses vertex 2 of 2"),
        ("polyops 1\nadd_mesh verts=0,0,0,1\n", 2, "not a multiple of 3"),
        ("polyops 1\nadd_mesh verts=0,0,0,1,0,0,0,1,0 faces=0,1,2 uvs=0,0\n", 2, "uvs= holds 2 numbers for 3 face corners"),
        ("polyops 1\nadd_mesh verts=b64:@@@@ \n", 2, "bad base64"),
        ("polyops 1\nmodifier_add Cube Explode\n", 2, "unknown modifier type 'Explode'"),
        ("polyops 1\nmodifier_set Cube 0 levels=2\n", 2, "modifier 0 does not exist"),
        ("polyops 1\nmodifier_add Cube Subdivision levels=99\n", 2, "6 or fewer"),
        ("polyops 1\nadd_empty A\nadd_empty B\nparent A B\nparent B A\n", 5, "cycle"),
        ("polyops 1\nmaterial_assign Cube Nope\n", 2, "no material named 'Nope'"),
        ("polyops 1\nactive_camera Cube\n", 2, "'Cube' is not a camera"),
        ("polyops 1\nlight_add laser\n", 2, "unknown light type 'laser'"),
        ("polyops 1\nadd_empty X\nset X.power 3\n", 3, "unknown property 'power' for 'X'"),
        ("polyops 1\nlight_add point size=2\n", 2, "size is only for area lights"),
        ("polyops 1\nend\nadd_empty X\n", 3, "text after 'end'"),
        ("polyops 1\nadd_empty X\nabort \"ZeroDivisionError: division by zero\"\n", 3, "script aborted: ZeroDivisionError: division by zero"),
        ("polyops 1\n@ ping\n", 2, "a request id is @"),
        ("polyops 1\nadd_empty X as=1\n", 2, "as= takes a handle"),
        ("polyops 1\nmesh_edit Cube twist\n", 2, "unknown mesh_edit operation 'twist'"),
        ("polyops 1\nmesh_edit Cube translate_verts 1 0 0 indices=8\n", 2, "vertex 8 does not exist"),
        ("polyops 1\nmesh_edit Light shade_smooth\n", 2, "object 'Light' has no mesh"),
        ("polyops 1\nlist cats\n", 2, "cannot list 'cats'"),
        ("polyops 1\ncollection_new A as=$a\ncollection_new B parent=$a as=$b\ncollection_set $a parent=$b\n", 4, "cycle"),
    ];
    for (src, line, msg) in cases {
        let e = err(src);
        assert_eq!(e.line, *line, "{src:?}: {e}");
        assert!(e.message.contains(msg), "{src:?}: got {e}, wanted {msg:?}");
        assert!(e.to_string().starts_with(&format!("line {line}: ")));
    }
}

#[test]
fn handles_follow_renames_and_deletes() {
    let mut s = Scene::new();
    let r = ok(
        &mut s,
        "polyops 1\n\
         add_empty Thing as=$1\n\
         add_empty Thing as=$2\n\
         set $2.name Other\n\
         set $1.location 1 1 1\n\
         set $2.location 2 2 2\n\
         get $2.name\n\
         delete $1\n\
         exists $2\n",
    );
    assert_eq!(r.created, ["Thing", "Thing.001"]);
    assert_eq!(r.replies, ["ok Other", "ok 1"]);
    assert_eq!(s.objects.len(), 1);
    assert_eq!(s.objects[0].transform.location, Vec3::splat(2.0));
    let e = run(&mut s, "polyops 1\nadd_empty T as=$1\ndelete $1\nset $1.location 0 0 0\n").unwrap_err();
    assert_eq!((e.line, e.message.as_str()), (4, "handle $1 is not bound"));
}

/// Startup plus one of everything: running the text on an empty scene gives
/// the same scene back, and writing that gives the same text.
#[test]
fn scene_round_trip() {
    let mut s = Scene::startup();
    ok(
        &mut s,
        "polyops 1\n\
         collection_new Props as=$c\n\
         collection_new \"Deep Props\" parent=$c visible=0\n\
         collection_set Collection name=Main\n\
         material_new Glass transmission=1 ior=1.45 roughness=0 double_sided=1\n\
         add_primitive torus name=Ring location=0,0,2 rotation=0.3,0.2,0.1 scale=1,2,3 collection=Props\n\
         add_primitive ico_sphere subdivisions=3\n\
         add_mesh name=\"Odd \\\"Mesh\\\"\" verts=0,0,0,1,0,0,1,1,0,0,1,0,5,5,5 faces=0,1,2;0,2,3 uvs=0,0,1,0,1,1,0,0,1,1,0,1 smooth=10 mat=0,1 edges=3,4\n\
         add_object Twin data=Ring collection=\"Deep Props\" visible=0\n\
         add_empty Root\n\
         light_add spot name=Spot angle=0.7 blend=0.2\n\
         light_add area name=Panel size=3 power=200\n\
         light_add sun name=Sun\n\
         camera_add name=Ortho ortho_scale=9 near=0.5 far=80\n\
         parent Ring Root\n\
         parent Twin Ring\n\
         material_assign Ring Glass\n\
         material_assign Ring Material slot=2\n\
         modifier_add Ring Subdivision levels=2\n\
         modifier_add Ring Array count=4 constant=0,0,1 merge=1\n\
         modifier_add Ring Mirror z=1\n\
         modifier_add Ring Triangulate\n\
         select Ring Twin Cube\n\
         active Twin\n\
         world color=0.2,0.3,0.4 strength=2\n\
         scene name=Shot frame_start=5 frame_end=99 fps=25\n",
    );
    let text = scene_to_polyops(&s);
    let mut back = Scene::new();
    let r = ok(&mut back, &text);
    assert!(r.complete);
    assert_eq!(back, s, "--- text ---\n{text}");
    assert_eq!(scene_to_polyops(&back), text);
    // Also exactly reproduces the startup scene by itself.
    let start = Scene::startup();
    let mut b2 = Scene::new();
    ok(&mut b2, &scene_to_polyops(&start));
    assert_eq!(b2, start);
}

#[test]
fn euler_and_quaternion_agree() {
    for (x, y, z) in [(0.3f32, -0.4f32, 1.2f32), (0.0, 1.5707964, 0.7), (2.0, -1.5707964, -0.2), (0.0, 0.0, 0.0)] {
        let q = Quat::from_euler_xyz(x, y, z);
        let [a, b, c] = script::quat_to_euler_xyz(q);
        let q2 = Quat::from_euler_xyz(a, b, c);
        for v in [Vec3::X, Vec3::Y, Vec3::Z] {
            assert!(q.rotate(v).approx_eq(q2.rotate(v), 1e-4), "{x} {y} {z}");
        }
    }
}

// --- Python, end to end ---------------------------------------------------------------

fn python() -> Option<&'static str> {
    let ok = Command::new("python3").arg("-c").arg("import sys; assert sys.version_info >= (3, 9)").output().map(|o| o.status.success()).unwrap_or(false);
    if !ok {
        eprintln!("skipping: python3 is not installed on this host");
    }
    ok.then_some("python3")
}

fn package_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/polyamour/python")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sex3d-script-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// Run a Python script against `scene` the way Polyamour does: the scene as
/// POLYAMOUR_SCENE, the commands back through POLYAMOUR_OPS (or stdout).
fn run_python(py: &str, script: &Path, scene: &mut Scene, via_stdout: bool) -> Result<RunReport, ScriptError> {
    let tag = script.file_stem().unwrap().to_string_lossy().to_string();
    let snap = tmp(&format!("{tag}.scene.polyops"));
    let ops = tmp(&format!("{tag}.ops.polyops"));
    std::fs::write(&snap, scene_to_polyops(scene)).unwrap();
    let mut cmd = Command::new(py);
    cmd.arg("-I").arg("-c").arg("import sys, runpy; sys.path[:0] = [sys.argv[1]]; sys.argv = sys.argv[2:]; runpy.run_path(sys.argv[0], run_name='__main__')");
    cmd.arg(package_dir()).arg(script).env("POLYAMOUR_SCENE", &snap).env_remove("POLYAMOUR_OPS");
    if !via_stdout {
        cmd.env("POLYAMOUR_OPS", &ops);
    }
    let out = cmd.output().unwrap();
    // The script's own output: stderr, plus stdout when commands went to the file.
    let mut stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let text = if via_stdout {
        String::from_utf8(out.stdout).unwrap()
    } else {
        stderr += &String::from_utf8_lossy(&out.stdout);
        std::fs::read_to_string(&ops).unwrap()
    };
    let r = run(scene, &text);
    if r.is_ok() {
        assert!(out.status.success(), "python failed:\n{stderr}");
    }
    r.map(|mut rep| {
        rep.replies.push(stderr);
        rep
    })
}

#[test]
fn python_spiral_staircase() {
    let Some(py) = python() else { return };
    let mut s = Scene::startup();
    let r = run_python(py, &package_dir().join("examples/spiral_staircase.py"), &mut s, false).unwrap();
    assert!(r.complete);
    assert!(r.replies.last().unwrap().contains("staircase: 32 objects"), "{:?}", r.replies);
    assert_eq!(r.render.get("resolution_x").map(String::as_str), Some("1280"));
    assert_eq!(s.objects.len(), 3 + 24 + 5);
    let pole = s.find_object("Pole").unwrap();
    let stairs = s.collections.iter().position(|c| c.name == "Staircase").unwrap();
    let brass = s.materials.iter().position(|m| m.name == "Brass").unwrap();
    assert_eq!(s.materials[brass].metallic, 1.0);
    for i in 0..24 {
        let si = s.find_object(&format!("Step{i:02}")).unwrap();
        let st = &s.objects[si];
        assert_eq!(st.parent, Some(pole));
        assert_eq!(st.collection, stairs);
        assert_eq!(st.modifiers, vec![Modifier::Subdivision { levels: 2, simple: false }]);
        assert_eq!(s.materials[st.materials[0]].name, ["Terracotta", "Sage", "Slate"][i % 3]);
        // World position: on the circle of radius 1.5, rising 0.2 per step.
        let a = i as f32 * std::f32::consts::TAU / 12.0;
        let p = s.world_matrix(si).transform_point(Vec3::ZERO);
        assert!(p.approx_eq(Vec3::new(1.5 * a.cos(), 1.5 * a.sin(), i as f32 * 0.2), 1e-4), "step {i} at {p:?}");
        assert!(st.transform.rotation.rotate(Vec3::X).approx_eq(Vec3::new(a.cos(), a.sin(), 0.0), 1e-4));
        assert!(st.transform.scale.approx_eq(Vec3::new(1.2, 0.4, 0.08), 1e-6));
    }
    let rail = obj(&s, "Handrail");
    match rail.data {
        ObjectData::Mesh(m) => {
            assert_eq!(s.meshes[m].vertex_count(), 97);
            assert_eq!(s.meshes[m].loose_edges.len(), 96);
            assert!(s.meshes[m].faces.is_empty());
        }
        _ => panic!("handrail has no mesh"),
    }
    assert_eq!(rail.materials, vec![brass]);
    let landing = obj(&s, "Top Landing \"A\"");
    assert_eq!(landing.materials, vec![brass]);
    match obj(&s, "Area").data {
        ObjectData::Light(l) => assert_eq!((l.kind, l.power), (LightKind::Area { size: 3.0 }, 500.0)),
        _ => panic!(),
    }
    assert_eq!(s.active_camera, s.find_object("Camera.001"));
    assert_eq!(s.world.color, Vec3::new(0.02, 0.02, 0.05));
    assert_eq!(s.frame_end, 48);
}

#[test]
fn python_plain_bpy_script() {
    let Some(py) = python() else { return };
    let mut s = Scene::startup();
    let r = run_python(py, &package_dir().join("examples/blender_pyramid.py"), &mut s, true).unwrap();
    assert!(r.complete);
    assert!(s.find_object("Cube").is_none(), "the default cube is deleted");
    let names: Vec<&str> = s.objects.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["Light", "Camera", "Pyramid", "Ball", "Sun"]);
    let pyr = obj(&s, "Pyramid");
    let ObjectData::Mesh(pm) = pyr.data else { panic!() };
    assert_eq!((s.meshes[pm].vertex_count(), s.meshes[pm].face_count()), (5, 5));
    assert!(s.meshes[pm].volume() > 0.0, "faces point outwards");
    let gold = &s.materials[pyr.materials[0]];
    assert_eq!((gold.name.as_str(), gold.metallic, gold.roughness), ("Gold", 1.0, 0.25));
    assert_eq!(gold.base_color, [1.0, 0.77, 0.34, 1.0]);
    let ball = obj(&s, "Ball");
    assert_eq!(
        ball.modifiers,
        vec![
            Modifier::Subdivision { levels: 2, simple: false },
            Modifier::Mirror { x: false, y: true, z: false, merge: true, merge_distance: 0.001 }
        ]
    );
    assert!(close(ball.transform.location, Vec3::new(3.0, 1.0, 0.5)));
    let ObjectData::Mesh(bm) = ball.data else { panic!() };
    assert!(s.meshes[bm].faces.iter().all(|f| f.smooth));
    match obj(&s, "Sun").data {
        ObjectData::Light(l) => assert_eq!((l.kind, l.power), (LightKind::Sun, 3.0)),
        _ => panic!(),
    }
    let sun_x = obj(&s, "Sun").transform.rotation.rotate(Vec3::Z);
    let want = Quat::from_euler_xyz(45f32.to_radians(), 0.0, 30f32.to_radians()).rotate(Vec3::Z);
    assert!(close(sun_x, want));
    assert_eq!(s.active_object, s.find_object("Pyramid"));
    assert_eq!(r.render.get("engine").map(String::as_str), Some("CYCLES"));
}

#[test]
fn python_failure_applies_nothing() {
    let Some(py) = python() else { return };
    let script = tmp("broken.py");
    std::fs::write(&script, "import bpy\nbpy.ops.mesh.primitive_cube_add()\nbpy.data.objects['Missing']\n").unwrap();
    let mut s = Scene::startup();
    let before = s.clone();
    let e = run_python(py, &script, &mut s, false).unwrap_err();
    assert!(e.message.contains("script aborted: KeyError"), "{e}");
    assert_eq!(e.line, 3);
    assert_eq!(s, before);
}

#[test]
fn python_reads_the_scene_snapshot() {
    let Some(py) = python() else { return };
    let script = tmp("reader.py");
    std::fs::write(
        &script,
        "import bpy, math\n\
         cam = bpy.context.scene.camera\n\
         cube = bpy.data.objects['Cube']\n\
         assert cube.type == 'MESH' and cube.active_material.name == 'Material', cube.active_material\n\
         assert len(cube.data.vertices) == 8 and len(cube.data.polygons) == 6\n\
         assert bpy.context.active_object is cube\n\
         assert cam.name == 'Camera' and abs(cam.data.lens - 50) < 1e-3, cam.data.lens\n\
         assert abs(bpy.data.objects['Light'].location.z - 5.9) < 1e-5\n\
         cube.location.z += 1\n\
         cube.data.vertices[0].co = (-2, -2, -2)\n\
         bpy.data.objects.new('Copy', cube.data)\n\
         bpy.data.materials['Material'].roughness = 0.9\n\
         print([o.name for o in bpy.data.objects])\n",
    )
    .unwrap();
    let mut s = Scene::startup();
    let r = run_python(py, &script, &mut s, false).unwrap();
    assert!(r.replies.last().unwrap().contains("['Cube', 'Light', 'Camera', 'Copy']"), "{:?}", r.replies);
    assert!(close(obj(&s, "Cube").transform.location, Vec3::new(0.0, 0.0, 1.0)));
    assert_eq!(obj(&s, "Copy").data, obj(&s, "Cube").data);
    assert_eq!(obj(&s, "Copy").materials, vec![0], "a new user of the mesh gets its materials");
    let ObjectData::Mesh(m) = obj(&s, "Cube").data else { panic!() };
    assert_eq!(s.meshes[m].positions[0], Vec3::splat(-2.0));
    assert_eq!(s.materials[0].roughness, 0.9);
}
