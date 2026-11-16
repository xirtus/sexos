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

// ---------------------------------------------------------------------------
// Modifiers, constraints, shape keys, lattices, Grease Pencil and physics.

use sex_3d::constraints::{Constraint, ConstraintKind, LimitDistanceMode, RotationMix, Space, StretchPlane, StretchVolume, Target, TrackAxis, TransformChannel, TransformMix, TransformationMix, VolumeMode};
use sex_3d::grease_pencil::{CapType, FillStyle, GpFrame, GpLayer, GpMaterial, GpPoint, GpStroke, GreasePencil, KeyframeType, LayerBlend, LineMode};
use sex_3d::lattice::Lattice;
use sex_3d::math::Mat4;
use sex_3d::mesh::{KeyInterpolation, ShapeKey};
use sex_3d::modifiers::{BooleanOp, CastType, DecimateMode, Falloff, Modifier, ProximityMode, RemeshMode, ShrinkwrapMethod, SimpleDeformMode, WeightMixMode, WeightMixSet};
use sex_3d::physics::props::{entries, set_value, Props, RefKind, Value as PValue};
use sex_3d::physics::{PhysicsModifier, RigidBody, RigidBodyConstraint, RigidBodyWorld};

fn export_import(s: &Scene) -> Scene {
    let bytes = export_blend(s, &[], BlendExport::default());
    import_blend_full(&bytes).expect("written file imports").0
}

/// A scene with the objects modifiers and constraints refer to: "Rig" (an
/// armature with bones "Root" and "Tip"), "Goal", "Pole", "Cage" (a
/// lattice) and "Body" (a mesh with vertex groups "Upper" and "Lower").
fn rig_scene() -> Scene {
    let mut s = Scene::new();
    let mut arm = Armature::default();
    let r = arm.add_bone(Bone::new("Root", Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0)));
    let mut tip = Bone::new("Tip", Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 2.0));
    tip.parent = Some(r);
    arm.add_bone(tip);
    s.add_armature_object("Rig", arm);
    s.add_object(Object::new("Goal", ObjectData::Empty));
    s.add_object(Object::new("Pole", ObjectData::Empty));
    s.add_lattice_object("Cage", Lattice::new(2, 2, 2));
    let mut m = test_mesh();
    m.vertex_groups = vec!["Upper".into(), "Lower".into()];
    s.add_mesh_object("Body", m);
    s
}

/// Every modifier type (Multires aside) with non-default settings.
fn every_modifier() -> Vec<Modifier> {
    let vg = || Some("Upper".to_string());
    let inv = Mat4::translation(Vec3::new(0.0, 0.0, -1.0));
    vec![
        Modifier::Mirror { x: false, y: true, z: true, merge: false, merge_distance: 0.01 },
        Modifier::Subdivision { levels: 2, simple: true },
        Modifier::Array { count: 3, relative: [0.0, 1.5, 0.0], constant: [0.25, 0.0, 0.5], merge: true },
        Modifier::Solidify { thickness: 0.2, offset: 0.5 },
        Modifier::Bevel { width: 0.05, segments: 3, angle: 0.5 },
        Modifier::Triangulate,
        Modifier::Weld { distance: 0.01 },
        Modifier::Displace { strength: 0.25, vertex_group: vg(), invert_vertex_group: true },
        Modifier::Smooth { factor: 0.75, iterations: 3, vertex_group: vg(), invert_vertex_group: true },
        Modifier::Decimate { mode: DecimateMode::Planar, ratio: 0.5, iterations: 2, angle: 0.2, triangulate: true },
        Modifier::Remesh { mode: RemeshMode::Smooth, voxel_size: 0.25, octree_depth: 5, scale: 0.75, smooth_shade: true },
        Modifier::Screw { axis: 1, angle: 3.0, steps: 8, screw_offset: 0.5, iterations: 2, merge: true, merge_distance: 0.05, flip: true, smooth_shade: false },
        Modifier::Wireframe { thickness: 0.05, offset: 0.5, even: false, relative: true, boundary: true, replace: false },
        Modifier::EdgeSplit { angle: 0.75, use_angle: false },
        Modifier::Build { start: 10.0, length: 50.0, reverse: true, random: true, seed: 7 },
        Modifier::Boolean { object: "Goal".into(), operation: BooleanOp::Union },
        Modifier::SimpleDeform { mode: SimpleDeformMode::Bend, axis: 0, factor: 0.5, limits: [0.25, 0.75], lock: [true, false, true], vertex_group: vg(), invert_vertex_group: true },
        Modifier::Cast { kind: CastType::Cuboid, factor: 0.25, radius: 2.0, size: 1.5, axes: [true, false, true], size_from_radius: true, vertex_group: vg(), invert_vertex_group: true },
        Modifier::Wave {
            x: false,
            y: true,
            cyclic: false,
            normals: true,
            falloff: 1.0,
            height: 0.25,
            width: 2.0,
            narrowness: 1.0,
            speed: 0.5,
            time_offset: 2.0,
            lifetime: 10.0,
            damping: 5.0,
            start: [0.5, -0.5],
            vertex_group: vg(),
            invert_vertex_group: true,
        },
        Modifier::Shrinkwrap { target: "Goal".into(), method: ShrinkwrapMethod::Project, offset: 0.1, axes: [false, true, true], positive: true, negative: true, vertex_group: vg(), invert_vertex_group: true },
        Modifier::LaplacianSmooth { iterations: 3, factor: 0.5, border: 0.25, axes: [true, false, true], preserve_volume: false, normalized: false, vertex_group: vg(), invert_vertex_group: true },
        Modifier::CorrectiveSmooth { factor: 0.25, iterations: 7, scale: 2.0, length_weighted: true, only_smooth: true, vertex_group: vg(), invert_vertex_group: true },
        Modifier::WeightedNormal { keep_sharp: true },
        Modifier::Armature {
            object: "Rig".into(),
            use_vertex_groups: false,
            use_bone_envelopes: true,
            use_deform_preserve_volume: true,
            use_multi_modifier: true,
            vertex_group: vg(),
            invert_vertex_group: true,
        },
        Modifier::Hook {
            object: "Rig".into(),
            subtarget: "Tip".into(),
            falloff_type: Falloff::Smooth,
            falloff_radius: 2.0,
            center: [0.0, 0.0, 1.0],
            matrix_inverse: inv,
            strength: 0.5,
            use_falloff_uniform: true,
            vertex_indices: vec![0, 2],
            vertex_group: None,
            invert_vertex_group: false,
        },
        Modifier::Lattice { object: "Cage".into(), strength: 0.8, vertex_group: vg(), invert_vertex_group: true },
        Modifier::Warp { object_from: "Goal".into(), object_to: "Pole".into(), strength: 0.5, falloff_type: Falloff::Sphere, falloff_radius: 3.0, use_volume_preserve: true, vertex_group: vg(), invert_vertex_group: true },
        Modifier::VertexWeightEdit {
            vertex_group: "Upper".into(),
            default_weight: 0.25,
            use_add: true,
            add_threshold: 0.1,
            use_remove: true,
            remove_threshold: 0.2,
            falloff_type: Falloff::Linear,
            invert_falloff: true,
            normalize: true,
            mask_constant: 0.5,
        },
        Modifier::VertexWeightMix {
            vertex_group_a: "Upper".into(),
            vertex_group_b: "Lower".into(),
            default_weight_a: 0.25,
            default_weight_b: 0.5,
            mix_mode: WeightMixMode::Average,
            mix_set: WeightMixSet::Or,
            invert_vertex_group_a: true,
            invert_vertex_group_b: true,
            normalize: true,
            mask_constant: 0.75,
        },
        Modifier::VertexWeightProximity {
            vertex_group: "Lower".into(),
            target: "Goal".into(),
            proximity_mode: ProximityMode::Object,
            use_proximity_geometry: [true, true, false],
            min_dist: 0.5,
            max_dist: 2.0,
            falloff_type: Falloff::Step,
            invert_falloff: true,
            normalize: true,
            mask_constant: 0.25,
        },
    ]
}

#[test]
fn modifiers_round_trip() {
    let mut s = rig_scene();
    let (bi, _) = obj(&s, "Body");
    s.objects[bi].modifiers = every_modifier();
    let r = export_import(&s);
    let (_, o) = obj(&r, "Body");
    assert_eq!(o.modifiers.len(), s.objects[bi].modifiers.len());
    for (a, b) in s.objects[bi].modifiers.iter().zip(&o.modifiers) {
        assert_eq!(a, b);
    }
}

#[test]
fn nodes_modifier_round_trip() {
    let mut s = rig_scene();
    let mut g = NodeTree::new("Displace", TreeKind::Geometry);
    let sock = |name: &str, ty: SocketType, default: Value, max: f32| InterfaceSocket { name: name.into(), ty, default, min: 0.0, max };
    g.inputs.push(sock("Geometry", SocketType::Geometry, Value::None, 0.0));
    g.inputs.push(sock("Amount", SocketType::Float, Value::Float(1.0), 10.0));
    g.inputs.push(sock("Count", SocketType::Int, Value::Int(2), 100.0));
    g.inputs.push(sock("Flip", SocketType::Bool, Value::Bool(false), 1.0));
    g.inputs.push(sock("Offset", SocketType::Vector, Value::Vector([0.0; 3]), 1.0));
    g.inputs.push(sock("Tint", SocketType::Color, Value::Color([1.0; 4]), 1.0));
    g.outputs.push(sock("Geometry", SocketType::Geometry, Value::None, 0.0));
    let gi = g.nodes.len();
    let go = gi + 1;
    let (n_in, n_out) = (sex_3d::nodes::edit::group_input_node(&g), sex_3d::nodes::edit::group_output_node(&g));
    g.nodes.push(n_in);
    g.nodes.push(n_out);
    let sub = g.add("GeometryNodeSubdivisionSurface", [0.0, 0.0]).unwrap();
    g.nodes[sub].set_prop("uv_smooth", Prop::Enum("SMOOTH_ALL".into()));
    g.link(gi, "Geometry", sub, "Mesh").unwrap();
    g.link(sub, "Mesh", go, "Geometry").unwrap();
    s.node_groups.push(g);
    let (bi, _) = obj(&s, "Body");
    let inputs = vec![
        ("Amount".to_string(), Value::Float(2.5)),
        ("Count".to_string(), Value::Int(4)),
        ("Flip".to_string(), Value::Bool(true)),
        ("Offset".to_string(), Value::Vector([0.5, 0.0, -0.5])),
        ("Tint".to_string(), Value::Color([0.25, 0.5, 0.75, 1.0])),
    ];
    s.objects[bi].modifiers = vec![Modifier::Nodes { group: "Displace".into(), inputs: inputs.clone() }, Modifier::Subdivision { levels: 1, simple: false }];
    let r = export_import(&s);
    let (_, o) = obj(&r, "Body");
    let Modifier::Nodes { group, inputs: got } = &o.modifiers[0] else { panic!("{:?}", o.modifiers) };
    assert_eq!(group, "Displace");
    assert_eq!(got, &inputs);
    let g = r.node_groups.iter().find(|g| g.name == "Displace").expect("group read back");
    assert_eq!(g.kind, TreeKind::Geometry);
    assert_eq!(g.inputs.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Geometry", "Amount", "Count", "Flip", "Offset", "Tint"]);
    let n = g.nodes.iter().find(|n| n.idname == "GeometryNodeSubdivisionSurface").unwrap();
    assert_eq!(n.enum_prop("uv_smooth"), "SMOOTH_ALL");
    assert_eq!(g.links.len(), 2);
    assert_eq!(o.modifiers[1], Modifier::Subdivision { levels: 1, simple: false });
}

/// Every constraint type with non-default settings.
fn every_constraint(bone: bool) -> Vec<Constraint> {
    let t = || if bone { Target { object: "Rig".into(), subtarget: "Root".into(), head_tail: 0.5 } } else { Target::object("Goal") };
    let kinds = vec![
        ConstraintKind::CopyLocation { target: t(), use_xyz: [true, false, true], invert: [false, true, false], use_offset: true },
        ConstraintKind::CopyRotation { target: t(), use_xyz: [false, true, true], invert: [true, false, false], mix: RotationMix::Before, euler_order: Some(EulerOrder::ZXY) },
        ConstraintKind::CopyScale { target: t(), use_xyz: [true, true, false], power: 0.5, use_make_uniform: true, use_offset: true, use_add: true },
        ConstraintKind::CopyTransforms { target: t(), mix: TransformMix::AfterSplit },
        ConstraintKind::LimitDistance { target: t(), distance: 2.5, mode: LimitDistanceMode::OnSurface },
        ConstraintKind::LimitLocation { use_min: [true, false, true], min: [-1.0, 0.0, -2.0], use_max: [false, true, false], max: [0.0, 3.0, 0.0] },
        ConstraintKind::LimitRotation { use_limit: [true, false, true], min: [-0.5, 0.0, -1.0], max: [0.5, 0.0, 1.0], euler_order: Some(EulerOrder::YZX) },
        ConstraintKind::LimitScale { use_min: [true, true, false], min: [0.5, 0.25, 0.0], use_max: [false, false, true], max: [0.0, 0.0, 2.0] },
        ConstraintKind::MaintainVolume { mode: VolumeMode::SingleAxis, free_axis: 2, volume: 1.5 },
        ConstraintKind::Transformation {
            target: t(),
            map_from: TransformChannel::Rotation,
            map_to: TransformChannel::Scale,
            from_min: [-1.0, -1.0, -1.0],
            from_max: [1.0, 1.0, 1.0],
            to_min: [0.5, 0.5, 0.5],
            to_max: [2.0, 2.0, 2.0],
            map: [2, 1, 0],
            use_motion_extrapolate: true,
            mix: TransformationMix::Multiply,
            to_euler_order: Some(EulerOrder::XZY),
        },
        ConstraintKind::ChildOf { target: t(), use_location: [true, false, true], use_rotation: [false, true, true], use_scale: [true, true, false], inverse: Mat4::translation(Vec3::new(0.0, 0.0, -1.0)) },
        ConstraintKind::TrackTo { target: t(), track_axis: TrackAxis::NegY, up_axis: 0, use_target_z: true },
        ConstraintKind::DampedTrack { target: t(), track_axis: TrackAxis::NegX },
        ConstraintKind::LockedTrack { target: t(), track_axis: TrackAxis::Z, lock_axis: 0 },
        ConstraintKind::StretchTo { target: t(), rest_length: 1.5, bulge: 2.0, use_bulge_min: true, bulge_min: 0.5, use_bulge_max: true, bulge_max: 1.5, bulge_smooth: 0.25, volume: StretchVolume::Z, keep_axis: StretchPlane::Z },
        ConstraintKind::Floor { target: t(), floor_location: TrackAxis::NegZ, offset: 0.25, use_rotation: true },
        ConstraintKind::Armature { targets: vec![(Target::bone("Rig", "Root"), 0.25), (Target::bone("Rig", "Tip"), 0.75)], use_deform_preserve_volume: true, use_bone_envelopes: true },
        ConstraintKind::Ik { target: t(), pole_target: Some(Target::object("Pole")), pole_angle: 0.5, chain_count: 2, iterations: 250, use_tail: false, use_stretch: false, weight: 0.5 },
    ];
    kinds
        .into_iter()
        .enumerate()
        .map(|(k, kind)| {
            let mut c = Constraint::new(kind);
            c.influence = 0.25 + k as f32 / 40.0;
            c.mute = k % 3 == 0;
            c.owner_space = if bone { Space::Pose } else { Space::Local };
            c.target_space = if k % 2 == 0 { Space::LocalWithParent } else { Space::World };
            c
        })
        .collect()
}

#[test]
fn constraints_round_trip() {
    let mut s = rig_scene();
    let (gi, _) = obj(&s, "Pole");
    // Armature and IK constraints belong to bones.
    let mut cons: Vec<Constraint> = every_constraint(false).into_iter().filter(|c| !matches!(c.kind, ConstraintKind::Armature { .. } | ConstraintKind::Ik { .. })).collect();
    cons[0].target_space = Space::Custom;
    cons[0].space_object = "Rig".into();
    cons[0].space_subtarget = "Root".into();
    s.objects[gi].constraints = cons;
    let (ri, _) = obj(&s, "Rig");
    let mut pb = PoseBone::new("Tip");
    pb.constraints = every_constraint(true);
    s.objects[ri].pose = vec![pb];
    let r = export_import(&s);
    let (_, o) = obj(&r, "Pole");
    assert_eq!(o.constraints.len(), s.objects[gi].constraints.len());
    for (a, b) in s.objects[gi].constraints.iter().zip(&o.constraints) {
        assert_eq!(a, b);
    }
    let (_, rig) = obj(&r, "Rig");
    let tip = rig.pose.iter().find(|p| p.name == "Tip").expect("pose bone Tip");
    let want = &s.objects[ri].pose[0].constraints;
    assert_eq!(tip.constraints.len(), want.len());
    for (a, b) in want.iter().zip(&tip.constraints) {
        assert_eq!(a, b);
    }
}

#[test]
fn shape_keys_round_trip() {
    let mut s = Scene::new();
    let mut m = test_mesh();
    let basis = m.positions.clone();
    let up: Vec<Vec3> = basis.iter().map(|p| *p + Vec3::new(0.0, 0.0, 1.0)).collect();
    let side: Vec<Vec3> = basis.iter().map(|p| *p + Vec3::new(0.5, 0.0, 0.0)).collect();
    let mut k1 = ShapeKey::new("Up", up);
    k1.value = 0.5;
    k1.slider_min = -1.0;
    k1.slider_max = 2.0;
    k1.vertex_group = Some("Tip".into());
    k1.frame = 10.0;
    let mut k2 = ShapeKey::new("Side", side);
    k2.relative_key = 1;
    k2.mute = true;
    k2.interpolation = KeyInterpolation::BSpline;
    k2.frame = 20.0;
    m.shape_keys = vec![ShapeKey::new("Basis", basis), k1, k2];
    s.add_mesh_object("Keyed", m);
    let r = export_import(&s);
    let (_, o) = obj(&r, "Keyed");
    let ObjectData::Mesh(mi) = o.data else { panic!() };
    let got = &r.meshes[mi];
    let want = &s.meshes[0];
    assert_eq!(got.vertex_groups, want.vertex_groups);
    assert_eq!(got.weights, want.weights);
    assert_eq!(got.shape_keys.len(), 3);
    for (a, b) in want.shape_keys.iter().zip(&got.shape_keys) {
        assert_eq!((&a.name, a.relative_key, &a.vertex_group, a.mute, a.interpolation), (&b.name, b.relative_key, &b.vertex_group, b.mute, b.interpolation));
        assert!(approx(a.value, b.value) && approx(a.slider_min, b.slider_min) && approx(a.slider_max, b.slider_max), "{}", a.name);
        assert!(approx(a.frame, b.frame), "{}: {} vs {}", a.name, a.frame, b.frame);
        assert_eq!(a.positions.len(), b.positions.len());
        assert!(a.positions.iter().zip(&b.positions).all(|(p, q)| approx3(*p, *q)), "{}", a.name);
    }
}

#[test]
fn lattice_round_trip() {
    let mut s = Scene::new();
    let mut l = Lattice::new(3, 2, 4);
    l.interpolation_u = KeyInterpolation::Linear;
    l.interpolation_v = KeyInterpolation::Cardinal;
    l.interpolation_w = KeyInterpolation::CatmullRom;
    l.use_outside = true;
    l.points[5] += Vec3::new(0.1, 0.2, 0.3);
    let i = s.add_lattice_object("Cage", l.clone());
    s.objects[i].transform.location = Vec3::new(1.0, 2.0, 3.0);
    let r = export_import(&s);
    let (_, o) = obj(&r, "Cage");
    let ObjectData::Lattice(li) = o.data else { panic!("{:?}", o.data) };
    assert!(approx3(o.transform.location, Vec3::new(1.0, 2.0, 3.0)));
    let g = &r.lattices[li];
    assert_eq!((g.points_u, g.points_v, g.points_w, g.use_outside), (3, 2, 4, true));
    assert_eq!((g.interpolation_u, g.interpolation_v, g.interpolation_w), (l.interpolation_u, l.interpolation_v, l.interpolation_w));
    assert_eq!(g.points.len(), 24);
    assert!(g.points.iter().zip(&l.points).all(|(p, q)| approx3(*p, *q)));
    assert_eq!(r.lattice_names[li], s.lattice_names[0]);
}

#[test]
fn grease_pencil_round_trip() {
    let mut s = Scene::new();
    let mut g = GreasePencil::default();
    let mut ink = GpMaterial::new("Ink", [0.1, 0.2, 0.3, 1.0]);
    ink.mode = LineMode::Dots;
    ink.pass_index = 2;
    ink.use_stroke_holdout = true;
    ink.lock = true;
    let mut fill = GpMaterial::fill("Paint", [1.0, 0.5, 0.0, 1.0]);
    fill.fill_style = FillStyle::Gradient;
    fill.mix_factor = 0.25;
    fill.use_fill_holdout = true;
    fill.hide = true;
    g.materials = vec![ink, fill];
    g.vertex_groups = vec!["Hand".into()];
    g.onion.before = 2;
    g.onion.after = 3;
    g.onion.opacity = 0.4;
    let mut p0 = GpPoint::new(Vec3::new(0.0, 0.0, 0.0), 0.02);
    p0.opacity = 0.75;
    p0.vertex_color = [1.0, 0.0, 0.0, 0.5];
    p0.weights = vec![(0, 0.5)];
    let mut p1 = GpPoint::new(Vec3::new(1.0, 0.0, 0.5), 0.01);
    p1.select = true;
    let p2 = GpPoint::new(Vec3::new(1.0, 0.0, 1.0), 0.02);
    let mut st = GpStroke::new(vec![p0, p1, p2], 0);
    st.cyclic = true;
    st.end_cap = CapType::Flat;
    st.softness = 0.25;
    st.select = true;
    let mut fs = GpStroke::new(vec![GpPoint::new(Vec3::ZERO, 0.005), GpPoint::new(Vec3::new(1.0, 0.0, 0.0), 0.005), GpPoint::new(Vec3::new(0.5, 0.0, 1.0), 0.005)], 1);
    fs.fill_color = [0.0, 1.0, 0.0, 0.5];
    let mut l = GpLayer::new("Ink");
    l.opacity = 0.8;
    l.tint_color = [1.0, 0.0, 0.0];
    l.tint_factor = 0.25;
    l.use_lights = true;
    l.transform.location = Vec3::new(0.0, 0.0, 1.0);
    l.frames = vec![
        GpFrame { frame: 1, drawing: sex_3d::grease_pencil::Drawing { strokes: vec![st, fs] }, key_type: KeyframeType::Keyframe, select: false },
        GpFrame { frame: 5, drawing: Default::default(), key_type: KeyframeType::Breakdown, select: true },
    ];
    let mut paper = GpLayer::new("Paper");
    paper.hide = true;
    paper.lock = true;
    paper.blend = LayerBlend::Multiply;
    paper.masks = vec!["Ink".into()];
    paper.use_masks = true;
    g.layers = vec![l, paper];
    g.active_layer = 1;
    let gi = s.add_grease_pencil_object("Sketch", g);
    s.objects[gi].transform.location = Vec3::new(0.0, 2.0, 0.0);
    let r = export_import(&s);
    let (_, o) = obj(&r, "Sketch");
    let ObjectData::GreasePencil(ri) = o.data else { panic!("{:?}", o.data) };
    let (a, b) = (&s.grease_pencils[0], &r.grease_pencils[ri]);
    assert_eq!(r.grease_pencil_names[ri], s.grease_pencil_names[0]);
    assert!(approx3(o.transform.location, Vec3::new(0.0, 2.0, 0.0)));
    // GP materials stay with the drawing, not the scene's materials.
    assert!(r.materials.iter().all(|m| m.name != "Ink" && m.name != "Paint"));
    assert_eq!(a.materials, b.materials);
    assert_eq!((&a.vertex_groups, a.active_layer), (&b.vertex_groups, b.active_layer));
    assert_eq!((a.onion.before, a.onion.after), (b.onion.before, b.onion.after));
    assert!(approx(a.onion.opacity, b.onion.opacity));
    assert_eq!(a.layers.len(), b.layers.len());
    for (x, y) in a.layers.iter().zip(&b.layers) {
        assert_eq!((&x.name, x.hide, x.lock, x.use_lights, x.blend, &x.masks, x.use_masks), (&y.name, y.hide, y.lock, y.use_lights, y.blend, &y.masks, y.use_masks));
        assert!(approx(x.opacity, y.opacity) && approx(x.tint_factor, y.tint_factor) && x.tint_color == y.tint_color, "{}", x.name);
        assert!(approx3(x.transform.location, y.transform.location));
        assert_eq!(x.frames.len(), y.frames.len());
        for (f, h) in x.frames.iter().zip(&y.frames) {
            assert_eq!((f.frame, f.key_type, f.select), (h.frame, h.key_type, h.select));
            assert_eq!(f.drawing.strokes.len(), h.drawing.strokes.len());
            for (s0, s1) in f.drawing.strokes.iter().zip(&h.drawing.strokes) {
                assert_eq!((s0.cyclic, s0.material, s0.start_cap, s0.end_cap, s0.select), (s1.cyclic, s1.material, s1.start_cap, s1.end_cap, s1.select));
                assert!(approx(s0.softness, s1.softness) && s0.fill_color == s1.fill_color);
                assert_eq!(s0.points.len(), s1.points.len());
                for (p, q) in s0.points.iter().zip(&s1.points) {
                    assert!(approx3(p.position, q.position));
                    assert!(approx(p.radius, q.radius), "radius {} vs {}", p.radius, q.radius);
                    assert!(approx(p.opacity, q.opacity) && p.vertex_color == q.vertex_color && p.select == q.select);
                    assert_eq!(p.weights, q.weights);
                }
            }
        }
    }
}

/// Change every property of `p` away from its value (numbers within their
/// range, the next enum item, references to `refs`).
fn perturb(p: &mut dyn Props, refs: &dyn Fn(RefKind) -> String) {
    for e in entries(p) {
        let v = match &e.value {
            PValue::F(x) => {
                let (lo, hi) = e.range.map_or((f32::MIN, f32::MAX), |r| if r.min < r.max { (r.min, r.max) } else { (f32::MIN, f32::MAX) });
                let mut y = (x * 0.5 + 0.375).clamp(lo, hi);
                if (y - x).abs() < 1e-6 {
                    y = if hi < 1e30 { (lo + hi) * 0.5 } else { x + 1.0 };
                }
                PValue::F(y)
            }
            PValue::I(i) => PValue::I(i + 1),
            PValue::B(b) => PValue::B(!b),
            PValue::E(i, _) => PValue::E((i + 1) % e.items.len().max(1), ""),
            PValue::V3(v) => PValue::V3([v[0] + 0.25, v[1] - 0.25, v[2] + 0.5]),
            PValue::S(_) => match e.reference {
                Some(k) => PValue::S(refs(k)),
                None => continue,
            },
        };
        set_value(p, e.id, &v);
    }
}

/// Properties of `a` and `b` that differ, as "id: a vs b".
fn props_diffs(a: &mut dyn Props, b: &mut dyn Props) -> Vec<String> {
    let eb = entries(b);
    let mut out = Vec::new();
    for x in entries(a) {
        let Some(y) = eb.iter().find(|y| y.id == x.id) else {
            out.push(format!("{}: missing", x.id));
            continue;
        };
        let same = match (&x.value, &y.value) {
            (PValue::F(p), PValue::F(q)) => approx(*p, *q),
            (PValue::V3(p), PValue::V3(q)) => (0..3).all(|i| approx(p[i], q[i])),
            (PValue::E(i, _), PValue::E(j, _)) => i == j,
            (p, q) => p == q,
        };
        if !same {
            out.push(format!("{}: {:?} vs {:?}", x.id, x.value, y.value));
        }
    }
    out
}

fn physics_refs(k: RefKind) -> String {
    match k {
        RefKind::Object => "Goal".into(),
        RefKind::Collection => "Props".into(),
        RefKind::VertexGroup => "Upper".into(),
        _ => String::new(),
    }
}

#[test]
fn physics_round_trip() {
    let mut s = rig_scene();
    s.add_collection("Props", None);
    let mut w = RigidBodyWorld::default();
    perturb(&mut w, &physics_refs);
    s.physics.rigidbody_world = Some(w);
    perturb(&mut s.physics.gravity, &physics_refs);
    let (bi, _) = obj(&s, "Body");
    let mut rb = RigidBody::default();
    perturb(&mut rb, &physics_refs);
    s.objects[bi].physics.rigid_body = Some(rb);
    let mut field = sex_3d::physics::FieldSettings::default();
    perturb(&mut field, &physics_refs);
    s.objects[bi].physics.field = Some(field);
    let mut mods = Vec::new();
    for key in ["collision", "cloth", "softbody", "particlesystem"] {
        let mut m = PhysicsModifier::with_defaults(key).unwrap();
        perturb(m.props(), &physics_refs);
        // Stored as whole numbers (`short`).
        if let PhysicsModifier::SoftBody(sb) = &mut m {
            sb.plastic = 5.0;
            sb.spring_length = 3.0;
        }
        mods.push(Modifier::Physics(m));
    }
    s.objects[bi].modifiers = mods;
    let (gi, _) = obj(&s, "Goal");
    let mut c = RigidBodyConstraint::default();
    perturb(&mut c, &physics_refs);
    c.object1 = "Body".into();
    c.object2 = "Pole".into();
    s.objects[gi].physics.rigid_body_constraint = Some(c);
    // Every fluid type on its own mesh, every field type on its own empty.
    let fluid_types = entries(PhysicsModifier::with_defaults("fluid").unwrap().props()).into_iter().find(|e| e.id == "fluid_type").unwrap().items.len();
    for k in 0..fluid_types {
        let pi = s.add_mesh_object(&format!("Tank{k}"), test_mesh());
        let mut fluid = PhysicsModifier::with_defaults("fluid").unwrap();
        set_value(fluid.props(), "fluid_type", &PValue::E(k, ""));
        perturb(fluid.props(), &physics_refs);
        set_value(fluid.props(), "fluid_type", &PValue::E(k, ""));
        s.objects[pi].modifiers = vec![Modifier::Physics(fluid)];
    }
    let joint_types = entries(&mut RigidBodyConstraint::default()).into_iter().find(|e| e.id == "type").unwrap().items.len();
    for k in 0..joint_types {
        let ji = s.add_object(Object::new(&format!("Joint{k}"), ObjectData::Empty));
        let mut c = RigidBodyConstraint::default();
        set_value(&mut c, "type", &PValue::E(k, ""));
        perturb(&mut c, &physics_refs);
        set_value(&mut c, "type", &PValue::E(k, ""));
        c.object1 = "Body".into();
        c.object2 = "Pole".into();
        s.objects[ji].physics.rigid_body_constraint = Some(c);
    }
    // A hair system next to the emitter.
    let hi = s.add_mesh_object("Hairy", test_mesh());
    let mut hair = PhysicsModifier::with_defaults("particlesystem").unwrap();
    set_value(hair.props(), "type", &PValue::E(1, ""));
    perturb(hair.props(), &physics_refs);
    set_value(hair.props(), "type", &PValue::E(1, ""));
    s.objects[hi].modifiers = vec![Modifier::Physics(hair)];
    let field_types = entries(&mut sex_3d::physics::FieldSettings::default()).into_iter().find(|e| e.id == "type").map_or(0, |e| e.items.len());
    assert!(field_types > 1);
    for k in 0..field_types {
        let fi = s.add_object(Object::new(&format!("Field{k}"), ObjectData::Empty));
        let mut f = sex_3d::physics::FieldSettings::default();
        set_value(&mut f, "type", &PValue::E(k, ""));
        perturb(&mut f, &physics_refs);
        set_value(&mut f, "type", &PValue::E(k, ""));
        s.objects[fi].physics.field = Some(f);
    }

    let r = export_import(&s);
    let mut diffs = Vec::new();
    let mut cmp = |what: &str, a: &mut dyn Props, b: &mut dyn Props| diffs.extend(props_diffs(a, b).into_iter().map(|d| format!("{what}.{d}")));
    let (mut sw, mut rw) = (s.physics.rigidbody_world.clone().unwrap(), r.physics.rigidbody_world.clone().expect("rigid body world"));
    cmp("world", &mut sw, &mut rw);
    let (mut sg, mut rg) = (s.physics.gravity.clone(), r.physics.gravity.clone());
    cmp("gravity", &mut sg, &mut rg);
    let (_, ob) = obj(&r, "Body");
    let mut a = s.objects[bi].physics.rigid_body.clone().unwrap();
    let mut b = ob.physics.rigid_body.clone().expect("rigid body");
    cmp("rigid_body", &mut a, &mut b);
    let mut a = s.objects[bi].physics.field.clone().unwrap();
    let mut b = ob.physics.field.clone().expect("field");
    cmp("field", &mut a, &mut b);
    assert_eq!(ob.modifiers.len(), 4, "{:?}", ob.modifiers.iter().map(|m| m.name()).collect::<Vec<_>>());
    for (x, y) in s.objects[bi].modifiers.iter().zip(&ob.modifiers) {
        let (Modifier::Physics(x), Modifier::Physics(y)) = (x, y) else { panic!("{y:?}") };
        let (mut x, mut y) = (x.clone(), y.clone());
        let what = x.name();
        cmp(what, x.props(), y.props());
    }
    let (_, og) = obj(&r, "Goal");
    let mut a = s.objects[gi].physics.rigid_body_constraint.clone().unwrap();
    let mut b = og.physics.rigid_body_constraint.clone().expect("rigid body constraint");
    cmp("rbc", &mut a, &mut b);
    for k in 0..fluid_types {
        let name = format!("Tank{k}");
        let (si, _) = obj(&s, &name);
        let (_, op) = obj(&r, &name);
        let (Modifier::Physics(x), Some(Modifier::Physics(y))) = (&s.objects[si].modifiers[0], op.modifiers.first()) else { panic!("{name}: {:?}", op.modifiers) };
        let (mut x, mut y) = (x.clone(), y.clone());
        cmp(&name, x.props(), y.props());
    }
    for k in 0..joint_types {
        let name = format!("Joint{k}");
        let (si, _) = obj(&s, &name);
        let (_, oj) = obj(&r, &name);
        let mut a = s.objects[si].physics.rigid_body_constraint.clone().unwrap();
        let mut b = oj.physics.rigid_body_constraint.clone().unwrap_or_else(|| panic!("{name}: no constraint"));
        cmp(&name, &mut a, &mut b);
        assert_eq!((b.object1.as_str(), b.object2.as_str()), ("Body", "Pole"));
    }
    {
        let (si, _) = obj(&s, "Hairy");
        let (_, oh) = obj(&r, "Hairy");
        let (Modifier::Physics(x), Some(Modifier::Physics(y))) = (&s.objects[si].modifiers[0], oh.modifiers.first()) else { panic!("{:?}", oh.modifiers) };
        let (mut x, mut y) = (x.clone(), y.clone());
        cmp("Hair", x.props(), y.props());
    }
    for k in 0..field_types {
        let name = format!("Field{k}");
        let (si, _) = obj(&s, &name);
        let (_, of) = obj(&r, &name);
        let mut a = s.objects[si].physics.field.clone().unwrap();
        let mut b = of.physics.field.clone().unwrap_or_else(|| panic!("{name}: no field"));
        // Vortex inflow and drag's quadratic term are Blender's `f_flow`,
        // the same member as Flow.
        if entries(&mut a).iter().any(|e| e.id == "inflow" || e.id == "quadratic_drag") {
            b.flow = a.flow;
        }
        cmp(&name, &mut a, &mut b);
    }
    assert!(diffs.is_empty(), "{} physics settings changed:\n{}", diffs.len(), diffs.join("\n"));
}
