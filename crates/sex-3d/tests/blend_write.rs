// SPDX-License-Identifier: GPL-3.0-only
//! `.blend` writer tests: every data type written by `export_blend` reads
//! back through `import_blend_full`, and the container follows the format
//! (header, block order, `DNA1`, `ENDB`).
use sex_3d::armature::{Armature, Bone, PoseBone};
use sex_3d::io::blend::{export_blend, import_blend_full, BlendAction, BlendExport, BlendFCurve, BlendKey, INTERP_BEZIER, INTERP_LINEAR};
use sex_3d::math::EulerOrder;
use sex_3d::nodes::{InterfaceSocket, NodeTree, Prop, SocketType, TreeKind, Value};
use sex_3d::scene::{AreaShape, Camera, Image, Light, LightKind, Material, Object, ObjectData, ParentType, SensorFit};
use sex_3d::{Face, Mesh, Quat, Scene, Vec2, Vec3};

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4 * (1.0 + a.abs().max(b.abs()))
}

fn approx3(a: Vec3, b: Vec3) -> bool {
    approx(a.x, b.x) && approx(a.y, b.y) && approx(a.z, b.z)
}

fn obj<'s>(s: &'s Scene, name: &str) -> (usize, &'s Object) {
    let i = s.objects.iter().position(|o| o.name == name).unwrap_or_else(|| panic!("no object {name}"));
    (i, &s.objects[i])
}

/// A quad and a triangle with UVs, a sharp edge, flat/smooth faces, two
/// materials and two vertex groups.
fn test_mesh() -> Mesh {
    let positions = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(2.0, 0.5, 0.5)];
    let mut m = Mesh::new();
    m.positions = positions;
    let mut q = Face::new(vec![0, 1, 2, 3]);
    q.uvs = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)];
    q.smooth = true;
    let mut t = Face::new(vec![1, 4, 2]);
    t.uvs = vec![Vec2::new(0.25, 0.5), Vec2::new(0.75, 0.5), Vec2::new(0.5, 0.9)];
    t.material = 1;
    t.smooth = false;
    m.faces = vec![q, t];
    m.sharp_edges.insert((1, 2));
    m.vertex_groups = vec!["Bone".into(), "Tip".into()];
    m.weights = vec![vec![(0, 1.0)], vec![(0, 0.5), (1, 0.5)], vec![(1, 0.25)], vec![], vec![(1, 1.0)]];
    m
}

fn build() -> (Scene, Vec<BlendAction>) {
    let mut s = Scene::new();
    s.name = "Shot".into();
    s.frame_start = 5;
    s.frame_end = 120;
    s.frame_current = 12;
    s.fps = 30.0;

    let props = s.add_collection("Props", None);
    let sub = s.add_collection("Sub", Some(props));
    let hidden = s.add_collection("Hidden", None);
    s.collections[hidden].visible = false;

    // Images: an albedo map and an environment map.
    let mut img = Image::new("Albedo", 4, 2);
    for (k, p) in img.pixels.iter_mut().enumerate() {
        *p = [k as f32 / 8.0, 0.5, 1.0 - k as f32 / 8.0, 1.0];
    }
    s.images.push(img);
    s.images.push(Image::new("Sky", 8, 4));

    // Materials: one from fields (with a texture), one from a tree.
    let red = s.add_material(Material { name: "Red".into(), base_color: [0.8, 0.1, 0.05, 1.0], metallic: 0.25, roughness: 0.3, ior: 1.45, emission: Vec3::new(1.0, 0.5, 0.0), emission_strength: 2.0, base_color_texture: Some(0), pass_index: 3, ..Material::default() });
    let mut tree = NodeTree::default_material("Shader Nodetree");
    let p = tree.nodes.iter().position(|n| n.idname == "ShaderNodeBsdfPrincipled").unwrap();
    tree.nodes[p].set_input("Roughness", Value::Float(0.9));
    let mix = tree.add("ShaderNodeMix", [-500.0, 0.0]).unwrap();
    tree.nodes[mix].set_prop("data_type", Prop::Enum("RGBA".into()));
    tree.nodes[mix].set_prop("blend_type", Prop::Enum("MULTIPLY".into()));
    let ramp = tree.add("ShaderNodeValToRGB", [-800.0, 0.0]).unwrap();
    tree.link(ramp, "Color", mix, "A").unwrap();
    tree.link(mix, "Result", p, "Base Color").unwrap();
    let mut green = Material { name: "Green".into(), node_tree: Some(tree), ..Material::default() };
    green.sync_from_tree();
    let green = s.add_material(green);

    // Meshes and objects.
    let mesh = s.add_mesh("Panel", test_mesh());
    let mut o = Object::new("Panel", ObjectData::Mesh(mesh));
    o.transform.location = Vec3::new(1.0, 2.0, 3.0);
    o.transform.rotation = Quat::from_euler([0.3, -0.2, 1.1], EulerOrder::XYZ);
    o.transform.scale = Vec3::new(1.0, 2.0, 0.5);
    o.materials = vec![red, green];
    o.collection = props;
    o.pass_index = 7;
    o.visibility.shadow = false;
    let panel = s.add_object(o);

    let cube = s.add_mesh("Cube", sex_3d::primitives::cube(2.0));
    let mut o = Object::new("Child", ObjectData::Mesh(cube));
    o.parent = Some(panel);
    o.collection = sub;
    o.visible = false;
    o.holdout = true;
    o.transform.location = Vec3::new(0.0, 0.0, 1.5);
    s.add_object(o);

    let mut e = Object::new("Empty", ObjectData::Empty);
    e.collection = hidden;
    s.add_object(e);

    let cam = Camera { fov_y: 0.6, near: 0.05, far: 250.0, shift_x: 0.1, sensor_fit: SensorFit::Auto, ..Camera::default() };
    let mut o = Object::new("Cam", ObjectData::Camera(cam));
    o.transform.location = Vec3::new(0.0, -8.0, 2.0);
    let c = s.add_object(o);
    s.active_camera = Some(c);
    let ortho = Camera { ortho_scale: Some(4.0), ..Camera::default() };
    s.add_object(Object::new("Ortho", ObjectData::Camera(ortho)));

    let lights = [
        ("Point", Light { power: 500.0, radius: 0.2, color: Vec3::new(1.0, 0.9, 0.8), ..Light::default() }),
        ("Sun", Light { kind: LightKind::Sun, power: 3.0, radius: 0.01, cast_shadow: false, ..Light::default() }),
        ("Spot", Light { kind: LightKind::Spot { angle: 0.8, blend: 0.3 }, power: 800.0, ..Light::default() }),
        ("Area", Light { kind: LightKind::Area { size: 2.0 }, shape: AreaShape::Rectangle, size_y: 0.5, power: 100.0, ..Light::default() }),
    ];
    for (n, l) in lights {
        s.add_object(Object::new(n, ObjectData::Light(l)));
    }

    // Armature with a pose and a bone-parented object.
    let mut arm = Armature::default();
    let mut root = Bone::new("Root", Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0));
    root.collections = vec!["Main".into()];
    let r = arm.add_bone(root);
    let mut tip = Bone::new("Tip", Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.5, 2.0));
    tip.parent = Some(r);
    tip.use_connect = true;
    tip.roll = 0.4;
    tip.collections = vec!["Main".into(), "Extra".into()];
    arm.add_bone(tip);
    arm.collections = vec!["Main".into(), "Extra".into()];
    let rig = s.add_armature_object("Rig", arm);
    let mut pb = PoseBone::new("Tip");
    pb.location = Vec3::new(0.0, 0.1, 0.0);
    pb.rotation_quaternion = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 0.5);
    s.objects[rig].pose = vec![pb];
    let mut o = Object::new("Hat", ObjectData::Empty);
    o.parent = Some(rig);
    o.parent_type = ParentType::Bone;
    o.parent_bone = "Tip".into();
    s.add_object(o);

    // World with an environment map.
    s.world.hdri = Some(1);
    s.world.strength = 1.5;

    // Compositor: Render Layers -> Glare -> Blur -> Composite.
    let mut c = NodeTree::new("Compositing Nodetree", TreeKind::Compositor);
    let rl = c.add("CompositorNodeRLayers", [0.0, 0.0]).unwrap();
    let glare = c.add("CompositorNodeGlare", [200.0, 0.0]).unwrap();
    c.nodes[glare].set_prop("glare_type", Prop::Enum("FOG_GLOW".into()));
    c.nodes[glare].set_prop("threshold", Prop::Float(0.7));
    let blur = c.add("CompositorNodeBlur", [400.0, 0.0]).unwrap();
    c.nodes[blur].set_prop("size_x", Prop::Int(5));
    c.nodes[blur].set_prop("filter_type", Prop::Enum("TENT".into()));
    let out = c.add("CompositorNodeComposite", [600.0, 0.0]).unwrap();
    c.link(rl, "Image", glare, "Image").unwrap();
    c.link(glare, "Image", blur, "Image").unwrap();
    c.link(blur, "Image", out, "Image").unwrap();
    s.compositor = Some(c);

    let key = |frame, value, interpolation| BlendKey { frame, value, interpolation, handles: None };
    let actions = vec![BlendAction {
        object: "Panel".into(),
        action: "PanelAction".into(),
        curves: vec![
            BlendFCurve { path: "location".into(), index: 0, keys: vec![key(1.0, 0.0, INTERP_BEZIER), key(10.0, 2.0, INTERP_LINEAR), key(20.0, 1.0, INTERP_BEZIER)] },
            BlendFCurve { path: "rotation_euler".into(), index: 2, keys: vec![key(1.0, 0.0, INTERP_LINEAR), key(30.0, 3.0, INTERP_LINEAR)] },
            BlendFCurve {
                path: "scale".into(),
                index: 1,
                keys: vec![
                    BlendKey { handles: Some([[-2.0, 1.0], [4.0, 1.5]]), ..key(1.0, 1.0, INTERP_BEZIER) },
                    BlendKey { handles: Some([[8.0, 2.5], [12.0, 1.5]]), ..key(10.0, 2.0, INTERP_BEZIER) },
                ],
            },
        ],
        parent_inverse: None,
    }];
    (s, actions)
}

fn round_trip() -> (Scene, Scene, Vec<BlendAction>, Vec<BlendAction>) {
    let (s, a) = build();
    let bytes = export_blend(&s, &a, BlendExport { width: 1280, height: 720 });
    let (r, ra) = import_blend_full(&bytes).expect("written file imports");
    (s, r, a, ra)
}

#[test]
fn scene_settings_round_trip() {
    let (s, r, _, _) = round_trip();
    assert_eq!(r.name, "Shot");
    assert_eq!((r.frame_start, r.frame_end, r.frame_current), (5, 120, 12));
    assert!(approx(r.fps, 30.0));
    assert_eq!(r.objects.len(), s.objects.len());
    let (c, _) = obj(&r, "Cam");
    assert_eq!(r.active_camera, Some(c));
}

#[test]
fn collections_round_trip() {
    let (_, r, _, _) = round_trip();
    let col = |n: &str| r.collections.iter().position(|c| c.name == n).unwrap_or_else(|| panic!("no collection {n}"));
    assert_eq!(r.collections.len(), 4);
    assert_eq!(r.collections[0].name, "Scene Collection");
    let (props, sub, hidden) = (col("Props"), col("Sub"), col("Hidden"));
    assert_eq!(r.collections[sub].parent, Some(props));
    assert_eq!(r.collections[hidden].parent, Some(0));
    assert_eq!(r.collections[props].parent, Some(0));
    assert!(!r.collections[hidden].visible && r.collections[props].visible);
    assert_eq!(obj(&r, "Panel").1.collection, props);
    assert_eq!(obj(&r, "Child").1.collection, sub);
    assert_eq!(obj(&r, "Empty").1.collection, hidden);
    assert_eq!(obj(&r, "Cam").1.collection, 0);
}

#[test]
fn objects_round_trip() {
    let (s, r, _, _) = round_trip();
    let (_, a) = obj(&s, "Panel");
    let (pi, b) = obj(&r, "Panel");
    assert!(approx3(a.transform.location, b.transform.location));
    assert!(approx3(a.transform.scale, b.transform.scale));
    assert!(a.transform.rotation.dot(b.transform.rotation).abs() > 0.9999, "{:?} vs {:?}", a.transform.rotation, b.transform.rotation);
    assert_eq!(b.pass_index, 7);
    assert!(!b.visibility.shadow && b.visibility.camera && b.visibility.diffuse);
    let (_, child) = obj(&r, "Child");
    assert_eq!(child.parent, Some(pi));
    assert!(!child.visible && child.holdout);
    assert!(approx3(child.transform.location, Vec3::new(0.0, 0.0, 1.5)));
    let (ri, _) = obj(&r, "Rig");
    let (_, hat) = obj(&r, "Hat");
    assert_eq!((hat.parent, hat.parent_type, hat.parent_bone.as_str()), (Some(ri), ParentType::Bone, "Tip"));
    assert!(matches!(obj(&r, "Empty").1.data, ObjectData::Empty));
}

#[test]
fn meshes_round_trip() {
    let (_, r, _, _) = round_trip();
    let (_, o) = obj(&r, "Panel");
    let ObjectData::Mesh(mi) = o.data else { panic!("Panel is not a mesh") };
    let m = &r.meshes[mi];
    let want = test_mesh();
    assert_eq!(m.positions.len(), 5);
    for (a, b) in m.positions.iter().zip(&want.positions) {
        assert!(approx3(*a, *b));
    }
    assert_eq!(m.faces.len(), 2);
    for (a, b) in m.faces.iter().zip(&want.faces) {
        assert_eq!(a.verts, b.verts);
        assert_eq!(a.material, b.material);
        assert_eq!(a.smooth, b.smooth);
        assert_eq!(a.uvs.len(), b.uvs.len());
        for (u, v) in a.uvs.iter().zip(&b.uvs) {
            assert!(approx(u.x, v.x) && approx(u.y, v.y));
        }
    }
    assert!(m.sharp_edges.contains(&(1, 2)));
    assert_eq!(m.sharp_edges.len(), 1);
    assert_eq!(m.vertex_groups, want.vertex_groups);
    for (v, ws) in want.weights.iter().enumerate() {
        let got = m.weights.get(v).cloned().unwrap_or_default();
        assert_eq!(got.len(), ws.len(), "vertex {v}");
        for ((g, w), (h, x)) in got.iter().zip(ws) {
            assert!(g == h && approx(*w, *x));
        }
    }
    // Material slots by name.
    let names: Vec<&str> = o.materials.iter().map(|&k| r.materials[k].name.as_str()).collect();
    assert_eq!(names, ["Red", "Green"]);
    // The cube, unchanged in size.
    let (_, c) = obj(&r, "Child");
    let ObjectData::Mesh(ci) = c.data else { panic!() };
    assert_eq!(r.meshes[ci].positions.len(), 8);
    assert_eq!(r.meshes[ci].faces.len(), 6);
}

#[test]
fn materials_round_trip() {
    let (_, r, _, _) = round_trip();
    let red = r.materials.iter().find(|m| m.name == "Red").unwrap();
    for (a, b) in red.base_color.iter().zip([1.0, 1.0, 1.0, 1.0]) {
        // An image on Base Color shows as a white base under the texture.
        assert!(approx(*a, b), "{:?}", red.base_color);
    }
    assert_eq!(red.base_color_texture.map(|i| r.images[i].name.as_str()), Some("Albedo"));
    assert!(approx(red.metallic, 0.25) && approx(red.roughness, 0.3) && approx(red.ior, 1.45), "{red:?}");
    assert!(approx3(red.emission, Vec3::new(1.0, 0.5, 0.0)) && approx(red.emission_strength, 2.0));
    assert_eq!(red.pass_index, 3);
    let green = r.materials.iter().find(|m| m.name == "Green").unwrap();
    let t = green.node_tree.as_ref().expect("tree");
    assert!(approx(green.roughness, 0.9));
    let mix = t.nodes.iter().find(|n| n.idname == "ShaderNodeMix").expect("mix node");
    assert_eq!(mix.enum_prop("blend_type"), "MULTIPLY");
    assert_eq!(mix.enum_prop("data_type"), "RGBA");
    assert!(t.nodes.iter().any(|n| n.idname == "ShaderNodeValToRGB"));
    assert_eq!(t.links.len(), 3);
}

#[test]
fn world_round_trip() {
    let (_, r, _, _) = round_trip();
    let t = r.world.node_tree.as_ref().expect("world tree");
    let env = t.nodes.iter().find(|n| n.idname == "ShaderNodeTexEnvironment").expect("environment node");
    let Some(Prop::Id(Some(i))) = env.prop("image") else { panic!("no image") };
    assert_eq!(r.images[*i].name, "Sky");
    assert!(approx(r.world.strength, 1.5));
}

#[test]
fn plain_world_round_trip() {
    let mut s = Scene::new();
    s.world.color = Vec3::new(0.2, 0.3, 0.4);
    let bytes = export_blend(&s, &[], BlendExport::default());
    let (r, _) = import_blend_full(&bytes).unwrap();
    assert!(r.world.node_tree.is_none());
    assert!(approx3(r.world.color * r.world.strength, Vec3::new(0.2, 0.3, 0.4)));
}

#[test]
fn cameras_and_lights_round_trip() {
    let (s, r, _, _) = round_trip();
    let ObjectData::Camera(a) = &obj(&s, "Cam").1.data else { panic!() };
    let ObjectData::Camera(b) = &obj(&r, "Cam").1.data else { panic!() };
    assert!(approx(a.fov_y, b.fov_y), "{} vs {}", a.fov_y, b.fov_y);
    assert!(approx(b.near, 0.05) && approx(b.far, 250.0) && approx(b.shift_x, 0.1));
    let ObjectData::Camera(o) = &obj(&r, "Ortho").1.data else { panic!() };
    assert!(o.ortho_scale.is_some_and(|v| approx(v, 4.0)), "{:?}", o.ortho_scale);
    for (n, _) in [("Point", 0), ("Sun", 1), ("Spot", 2), ("Area", 3)] {
        let ObjectData::Light(a) = &obj(&s, n).1.data else { panic!() };
        let ObjectData::Light(b) = &obj(&r, n).1.data else { panic!("{n}") };
        assert_eq!(a.kind, b.kind, "{n}");
        assert!(approx(a.power, b.power) && approx3(a.color, b.color), "{n}");
        assert_eq!(a.cast_shadow, b.cast_shadow, "{n}");
        if let LightKind::Area { .. } = a.kind {
            assert_eq!(b.shape, AreaShape::Rectangle);
            assert!(approx(b.size_y, 0.5));
        } else {
            assert!(approx(a.radius, b.radius), "{n}");
        }
    }
}

#[test]
fn armature_and_pose_round_trip() {
    let (s, r, _, _) = round_trip();
    let ObjectData::Armature(ai) = obj(&s, "Rig").1.data else { panic!() };
    let ObjectData::Armature(bi) = obj(&r, "Rig").1.data else { panic!() };
    let (a, b) = (&s.armatures[ai], &r.armatures[bi]);
    assert_eq!(b.bones.len(), 2);
    for (x, y) in a.bones.iter().zip(&b.bones) {
        assert_eq!(x.name, y.name);
        assert_eq!(x.parent, y.parent);
        assert!(approx3(x.head, y.head) && approx3(x.tail, y.tail), "{}", x.name);
        assert!(approx(x.roll, y.roll), "{} roll {} vs {}", x.name, x.roll, y.roll);
        assert_eq!(x.use_connect, y.use_connect);
        assert_eq!(x.collections, y.collections);
    }
    assert_eq!(b.collections, a.collections);
    let pose = &obj(&r, "Rig").1.pose;
    let tip = pose.iter().find(|p| p.name == "Tip").expect("Tip channel");
    assert!(approx3(tip.location, Vec3::new(0.0, 0.1, 0.0)));
    let q = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 0.5);
    assert!(tip.rotation_quaternion.dot(q).abs() > 0.9999);
}

#[test]
fn actions_round_trip() {
    let (_, _, a, ra) = round_trip();
    assert_eq!(ra.len(), 1);
    let (x, y) = (&a[0], &ra[0]);
    assert_eq!(y.object, "Panel");
    assert_eq!(y.action, "PanelAction");
    assert_eq!(y.curves.len(), 3);
    for (c, d) in x.curves.iter().zip(&y.curves) {
        assert_eq!((c.path.as_str(), c.index), (d.path.as_str(), d.index));
        assert_eq!(c.keys.len(), d.keys.len());
        for (k, l) in c.keys.iter().zip(&d.keys) {
            assert!(approx(k.frame, l.frame) && approx(k.value, l.value));
            assert_eq!(k.interpolation, l.interpolation);
            if let Some(h) = k.handles {
                assert_eq!(Some(h), l.handles);
            }
        }
    }
}

/// Keys without handles get auto-clamped ones: flat at the ends and at
/// extremes, along the neighbours' slope in between.
#[test]
fn auto_handles_are_clamped() {
    let (_, _, _, ra) = round_trip();
    let c = &ra[0].curves[0];
    let h = |k: usize| c.keys[k].handles.unwrap();
    assert_eq!(h(0)[0][1], 0.0);
    assert_eq!(h(0)[1][1], 0.0);
    // Key 1 (frame 10, value 2) is a maximum between 0 and 1: flat.
    assert_eq!(h(1)[0][1], 2.0);
    assert_eq!(h(1)[1][1], 2.0);
    assert!((h(1)[0][0] - 7.0).abs() < 1e-5 && (h(1)[1][0] - 40.0 / 3.0).abs() < 1e-5);
}

#[test]
fn images_round_trip_packed() {
    let (s, r, _, _) = round_trip();
    let a = &s.images[0];
    let b = r.images.iter().find(|i| i.name == "Albedo").expect("image");
    assert_eq!((b.width, b.height), (a.width, a.height));
    for (p, q) in a.pixels.iter().zip(&b.pixels) {
        for c in 0..4 {
            assert!((p[c] - q[c]).abs() < 0.02, "{p:?} vs {q:?}");
        }
    }
}

#[test]
fn compositor_round_trip() {
    let (_, r, _, _) = round_trip();
    let c = r.compositor.as_ref().expect("compositor tree");
    assert_eq!(c.kind, TreeKind::Compositor);
    assert_eq!(c.nodes.len(), 4);
    assert_eq!(c.links.len(), 3);
    let glare = c.nodes.iter().find(|n| n.idname == "CompositorNodeGlare").unwrap();
    assert_eq!(glare.enum_prop("glare_type"), "FOG_GLOW");
    assert!(matches!(glare.prop("threshold"), Some(Prop::Float(v)) if approx(*v, 0.7)));
    let blur = c.nodes.iter().find(|n| n.idname == "CompositorNodeBlur").unwrap();
    assert_eq!(blur.enum_prop("filter_type"), "TENT");
    assert!(matches!(blur.prop("size_x"), Some(Prop::Int(5))));
}

#[test]
fn node_groups_round_trip() {
    let mut s = Scene::new();
    let mut g = NodeTree::new("Tint", TreeKind::Shader);
    let gi = g.add("NodeGroupInput", [0.0, 0.0]).unwrap();
    let go = g.add("NodeGroupOutput", [300.0, 0.0]).unwrap();
    let sock = |name: &str| InterfaceSocket { name: name.into(), ty: SocketType::Color, default: Value::Color([1.0, 0.5, 0.25, 1.0]), min: 0.0, max: 1.0 };
    g.inputs.push(sock("Color"));
    g.outputs.push(sock("Result"));
    let _ = (gi, go);
    s.node_groups.push(g);
    let mut t = NodeTree::default_material("Shader Nodetree");
    let n = t.add("ShaderNodeGroup", [-400.0, 0.0]).unwrap();
    t.nodes[n].set_prop("node_tree", Prop::Id(Some(0)));
    s.add_material(Material { name: "Grouped".into(), node_tree: Some(t), ..Material::default() });
    let bytes = export_blend(&s, &[], BlendExport::default());
    let (r, _) = import_blend_full(&bytes).unwrap();
    assert_eq!(r.node_groups.len(), 1);
    assert_eq!(r.node_groups[0].name, "Tint");
    assert_eq!(r.node_groups[0].inputs.len(), 1);
    assert_eq!(r.node_groups[0].outputs[0].name, "Result");
    let t = r.materials[0].node_tree.as_ref().unwrap();
    let n = t.nodes.iter().find(|n| n.idname == "ShaderNodeGroup").unwrap();
    assert!(matches!(n.prop("node_tree"), Some(Prop::Id(Some(0)))));
}

/// The container: magic, block headers, an ID's data after it, `DNA1` and
/// `ENDB` last.
#[test]
fn container_layout() {
    let (s, a) = build();
    let b = export_blend(&s, &a, BlendExport::default());
    assert_eq!(&b[..12], b"BLENDER-v402");
    let mut at = 12;
    let mut codes = Vec::new();
    let mut seen_id = false;
    while at + 24 <= b.len() {
        let code = [b[at], b[at + 1], b[at + 2], b[at + 3]];
        let len = i32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        let old = u64::from_le_bytes(b[at + 8..at + 16].try_into().unwrap());
        codes.push(code);
        if code[2] == 0 && code[3] == 0 {
            seen_id = true;
        }
        if &code == b"DATA" {
            assert!(seen_id, "a DATA block before any ID");
            assert_ne!(old, 0);
        }
        at += 24 + len;
        if &code == b"ENDB" {
            break;
        }
    }
    assert_eq!(at, b.len(), "ENDB is the last block");
    assert_eq!(&codes[0], b"REND");
    assert_eq!(&codes[1], b"GLOB");
    assert_eq!(&codes[codes.len() - 2], b"DNA1");
    assert_eq!(&codes[2], b"SC\0\0", "the scene comes first");
    for id in [b"OB\0\0", b"ME\0\0", b"MA\0\0", b"CA\0\0", b"LA\0\0", b"AR\0\0", b"WO\0\0", b"IM\0\0", b"AC\0\0", b"GR\0\0"] {
        assert!(codes.contains(id), "no {} block", String::from_utf8_lossy(id));
    }
}
