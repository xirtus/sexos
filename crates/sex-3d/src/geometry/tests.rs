// SPDX-License-Identifier: GPL-3.0-only
use super::field::{self, Cache, Ctx};
use super::primitives::{self, Fill};
use super::*;
use crate::math::Vec3;
use crate::nodes::{edit, InterfaceSocket, NodeTree, Prop, SocketType, TreeKind, Value};

fn iface(name: &str, ty: SocketType) -> InterfaceSocket {
    InterfaceSocket { name: name.into(), ty, default: Value::None, min: f32::MIN, max: f32::MAX }
}

/// A tree with the given interface; Group Input is node 0, Group Output 1.
fn tree(inputs: &[(&str, SocketType)], outputs: &[(&str, SocketType)]) -> NodeTree {
    let mut t = NodeTree::new("T", TreeKind::Geometry);
    t.inputs = inputs.iter().map(|(n, ty)| iface(n, *ty)).collect();
    t.outputs = outputs.iter().map(|(n, ty)| iface(n, *ty)).collect();
    let gi = edit::group_input_node(&t);
    let go = edit::group_output_node(&t);
    t.nodes.push(gi);
    t.nodes.push(go);
    t
}

fn add(t: &mut NodeTree, idname: &str) -> usize {
    t.add(idname, [0.0, 0.0]).unwrap_or_else(|| panic!("no node type {idname}"))
}

fn link(t: &mut NodeTree, a: usize, from: &str, b: usize, to: &str) {
    t.link(a, from, b, to).unwrap_or_else(|e| panic!("link {from} -> {to}: {e:?}"));
}

fn run(t: &NodeTree) -> Vec<SocketValue> {
    evaluate_group(t, Vec::new(), &EvalOptions::default())
}

fn mesh_of(v: &SocketValue) -> crate::mesh::Mesh {
    eval::geometry_to_mesh(&v.geometry(), &|_| None)
}

fn counts(m: &crate::mesh::Mesh) -> (usize, usize, usize) {
    (m.positions.len(), m.edges().len(), m.faces.len())
}

/// One primitive node, evaluated with `inputs` set.
fn primitive(idname: &str, props: &[(&str, &str)], inputs: &[(&str, Value)]) -> crate::mesh::Mesh {
    let mut t = tree(&[], &[("Geometry", SocketType::Geometry)]);
    let p = add(&mut t, idname);
    for (k, v) in props {
        t.nodes[p].set_prop(k, Prop::Enum((*v).into()));
    }
    for (k, v) in inputs {
        assert!(t.nodes[p].set_input(k, v.clone()), "{idname} has no input {k}");
    }
    link(&mut t, p, "Mesh", 1, "Geometry");
    mesh_of(&run(&t)[0])
}

#[test]
fn uv_sphere_counts() {
    let m = primitive("GeometryNodeMeshUVSphere", &[], &[]);
    assert_eq!(m.positions.len(), 482);
    assert_eq!(m.faces.len(), 512);
    assert_eq!(m.edges().len(), 992);
    let m = primitives::uv_sphere(8, 4, 1.0);
    assert_eq!(counts(&m), (26, 56, 32));
    // Every vertex sits on the sphere.
    let m = primitive("GeometryNodeMeshUVSphere", &[], &[("Radius", Value::Float(2.0))]);
    assert!(m.positions.iter().all(|p| (p.length() - 2.0).abs() < 1e-4));
}

#[test]
fn ico_sphere_counts() {
    for (sub, v, e, f) in [(1, 12, 30, 20), (2, 42, 120, 80), (3, 162, 480, 320), (4, 642, 1920, 1280)] {
        let m = primitive("GeometryNodeMeshIcoSphere", &[], &[("Subdivisions", Value::Int(sub))]);
        assert_eq!(counts(&m), (v, e, f), "subdivisions {sub}");
        assert!(m.positions.iter().all(|p| (p.length() - 1.0).abs() < 1e-4));
        assert!(m.faces.iter().all(|f| f.verts.len() == 3));
    }
    // A vertex at each pole.
    let m = primitives::ico_sphere(1.0, 1);
    assert!(m.positions.iter().any(|p| (p.z - 1.0).abs() < 1e-5));
    assert!(m.positions.iter().any(|p| (p.z + 1.0).abs() < 1e-5));
}

#[test]
fn cylinder_and_cone_fill_types() {
    for (fill, v, f) in [("NGON", 64, 34), ("TRIANGLE_FAN", 66, 96), ("NONE", 64, 32)] {
        let m = primitive("GeometryNodeMeshCylinder", &[("fill_type", fill)], &[]);
        assert_eq!((m.positions.len(), m.faces.len()), (v, f), "{fill}");
    }
    // Segments: 3 side rings and 2 fill rings.
    let (m, parts) = primitives::cone(8, 3, 2, 1.0, 1.0, 2.0, Fill::Ngon);
    assert_eq!(m.positions.len(), 8 * 4 + 2 * 8);
    assert_eq!(parts.side.iter().filter(|&&b| b).count(), 24);
    assert_eq!(parts.top.iter().filter(|&&b| b).count(), 9);
    // A cone's tip is one vertex.
    let m = primitive("GeometryNodeMeshCone", &[("fill_type", "NGON")], &[]);
    assert_eq!((m.positions.len(), m.faces.len()), (33, 33));
    let m = primitive("GeometryNodeMeshCone", &[("fill_type", "TRIANGLE_FAN")], &[]);
    assert_eq!((m.positions.len(), m.faces.len()), (34, 64));
}

#[test]
fn cube_grid_circle_line() {
    assert_eq!(counts(&primitive("GeometryNodeMeshCube", &[], &[])), (8, 12, 6));
    let m = primitive("GeometryNodeMeshCube", &[], &[("Vertices X", Value::Int(3)), ("Vertices Y", Value::Int(3)), ("Vertices Z", Value::Int(3))]);
    assert_eq!(counts(&m), (26, 48, 24));
    assert_eq!(counts(&primitive("GeometryNodeMeshGrid", &[], &[])), (9, 12, 4));
    let m = primitive("GeometryNodeMeshGrid", &[], &[("Vertices X", Value::Int(4)), ("Vertices Y", Value::Int(2))]);
    assert_eq!(counts(&m), (8, 10, 3));
    assert_eq!(counts(&primitive("GeometryNodeMeshCircle", &[], &[])), (32, 32, 0));
    assert_eq!(counts(&primitive("GeometryNodeMeshCircle", &[("fill_type", "NGON")], &[])), (32, 32, 1));
    assert_eq!(counts(&primitive("GeometryNodeMeshCircle", &[("fill_type", "TRIANGLE_FAN")], &[])), (33, 64, 32));
    assert_eq!(counts(&primitive("GeometryNodeMeshLine", &[], &[])), (10, 9, 0));
    // The cube faces point outward.
    let m = primitives::cube(Vec3::splat(2.0), [2, 2, 2]);
    for f in &m.faces {
        let c = f.verts.iter().fold(Vec3::ZERO, |a, &v| a + m.positions[v as usize]) / f.verts.len() as f32;
        let p = |k: usize| m.positions[f.verts[k] as usize];
        let n = (p(1) - p(0)).cross(p(2) - p(0));
        assert!(n.dot(c) > 0.0);
    }
}

#[test]
fn field_domains() {
    let m = primitives::grid(2.0, 2.0, 3, 3);
    let t = MeshTopo::new(&m);
    let ctx = Ctx::Mesh(&m, &t);
    let mut cache = Cache::default();
    assert_eq!(ctx.size(Domain::Point), 9);
    assert_eq!(ctx.size(Domain::Edge), 12);
    assert_eq!(ctx.size(Domain::Face), 4);
    assert_eq!(ctx.size(Domain::Corner), 16);
    // Position on faces is the face centre (point values averaged).
    let centres = field::eval(&field::position(), &ctx, Domain::Face, &mut cache);
    for (f, c) in m.faces.iter().zip(centres.iter()) {
        let want = f.verts.iter().fold(Vec3::ZERO, |a, &v| a + m.positions[v as usize]) / 4.0;
        assert!((c.v() - want).length() < 1e-5);
    }
    // Index is per domain.
    let idx = field::eval(&field::index(), &ctx, Domain::Edge, &mut cache);
    assert_eq!(idx.iter().map(|v| v.i()).collect::<Vec<_>>(), (0..12).collect::<Vec<_>>());
    // Face -> point averages the faces around each point: the centre vertex
    // touches all four faces.
    let face_index = field::eval(&field::index(), &ctx, Domain::Face, &mut cache);
    let fvals: Vec<Val> = face_index.iter().map(|v| Val::Float(v.i() as f32)).collect();
    let on_points = ctx.adapt(&fvals, Domain::Face, Domain::Point);
    let centre = m.positions.iter().position(|p| p.length() < 1e-6).unwrap();
    assert!((on_points[centre].f() - 1.5).abs() < 1e-5, "{:?}", on_points);
    // Bool face -> point is "any".
    let sel: Vec<Val> = (0..4).map(|i| Val::Bool(i == 0)).collect();
    let pts = ctx.adapt(&sel, Domain::Face, Domain::Point);
    assert_eq!(pts.iter().filter(|v| v.b()).count(), 4);
    // Point -> edge is the mean of the two ends.
    let px: Vec<Val> = m.positions.iter().map(|p| Val::Float(p.x)).collect();
    let ex = ctx.adapt(&px, Domain::Point, Domain::Edge);
    for (e, v) in t.edges.iter().zip(ex) {
        let want = (m.positions[e[0] as usize].x + m.positions[e[1] as usize].x) / 2.0;
        assert!((v.f() - want).abs() < 1e-5);
    }
}

#[test]
fn store_and_capture_attributes() {
    // Store the face index on faces, read it back on points in a Set
    // Position offset: z = mean of the adjacent face indices.
    let mut t = tree(&[], &[("Geometry", SocketType::Geometry)]);
    let grid = add(&mut t, "GeometryNodeMeshGrid");
    let store = add(&mut t, "GeometryNodeStoreNamedAttribute");
    t.nodes[store].set_prop("domain", Prop::Enum("FACE".into()));
    t.nodes[store].set_input("Name", Value::String("f".into()));
    let index = add(&mut t, "GeometryNodeInputIndex");
    link(&mut t, grid, "Mesh", store, "Geometry");
    link(&mut t, index, "Index", store, "Value");
    link(&mut t, store, "Geometry", 1, "Geometry");
    let m = mesh_of(&run(&t)[0]);
    let a = m.attributes.get("f").expect("stored attribute");
    assert_eq!(a.domain, Domain::Face);
    assert_eq!(a.data.len(), 4);
    assert_eq!(a.data.get(3).f(), 3.0);
}

#[test]
fn extrude_topology() {
    let cube = primitives::cube(Vec3::splat(2.0), [2, 2, 2]);
    let mut sel = vec![false; 6];
    sel[0] = true;
    let offs = vec![Vec3::Z; 6];
    let r = meshops::extrude_faces(&cube, &sel, &offs, false);
    assert_eq!(counts(&r.mesh), (12, 20, 10));
    let r = meshops::extrude_faces(&cube, &[true; 6], &offs, true);
    assert_eq!((r.mesh.positions.len(), r.mesh.faces.len()), (32, 30));
    // Region extrusion of the whole cube keeps it closed: every vertex is
    // duplicated, the originals lose their faces.
    let r = meshops::extrude_faces(&cube, &[true; 6], &offs, false);
    assert_eq!(r.mesh.faces.len(), 6);

    // Through the node: extrude the top face of a grid by 1 along the
    // normal; Top marks the moved face and Side the 4 new quads.
    let mut t = tree(&[], &[("Geometry", SocketType::Geometry)]);
    let grid = add(&mut t, "GeometryNodeMeshGrid");
    t.nodes[grid].set_input("Vertices X", Value::Int(2));
    t.nodes[grid].set_input("Vertices Y", Value::Int(2));
    let ex = add(&mut t, "GeometryNodeExtrudeMesh");
    link(&mut t, grid, "Mesh", ex, "Mesh");
    link(&mut t, ex, "Mesh", 1, "Geometry");
    let m = mesh_of(&run(&t)[0]);
    assert_eq!(counts(&m), (8, 12, 5));
    assert_eq!(m.positions.iter().filter(|p| (p.z - 1.0).abs() < 1e-5).count(), 4);

    // Edges mode on a grid: one quad per edge.
    t.nodes[ex].set_prop("mode", Prop::Enum("EDGES".into()));
    let m = mesh_of(&run(&t)[0]);
    assert_eq!((m.positions.len(), m.faces.len()), (8, 5));
    t.nodes[ex].set_prop("mode", Prop::Enum("VERTICES".into()));
    let m = mesh_of(&run(&t)[0]);
    assert_eq!((m.positions.len(), m.edges().len(), m.faces.len()), (8, 8, 1));
}

#[test]
fn instance_on_points_and_realize() {
    let mut t = tree(&[], &[("Geometry", SocketType::Geometry)]);
    let grid = add(&mut t, "GeometryNodeMeshGrid");
    let cube = add(&mut t, "GeometryNodeMeshCube");
    let iop = add(&mut t, "GeometryNodeInstanceOnPoints");
    link(&mut t, grid, "Mesh", iop, "Points");
    link(&mut t, cube, "Mesh", iop, "Instance");
    link(&mut t, iop, "Instances", 1, "Geometry");
    let out = run(&t);
    let g = out[0].geometry();
    assert_eq!(g.instances.as_ref().map_or(0, |i| i.len()), 9);
    assert!(g.mesh.is_none());
    assert_eq!(g.domain_size(Component::Instances, Domain::Instance), 9);
    // Realize Instances turns them into one mesh.
    let r = add(&mut t, "GeometryNodeRealizeInstances");
    link(&mut t, iop, "Instances", r, "Geometry");
    link(&mut t, r, "Geometry", 1, "Geometry");
    let g = run(&t)[0].geometry();
    assert!(g.instances.as_ref().is_none_or(|i| i.is_empty()));
    let m = g.mesh.unwrap();
    assert_eq!((m.positions.len(), m.faces.len()), (72, 54));
    // A cube sits at each grid point.
    let mut xs: Vec<i32> = m.positions.iter().map(|p| (p.x * 2.0).round() as i32).collect();
    xs.sort();
    xs.dedup();
    assert_eq!(xs, vec![-2, -1, 0, 1, 2]);
    // Join Geometry with two inputs on its multi-input socket.
    let j = add(&mut t, "GeometryNodeJoinGeometry");
    link(&mut t, grid, "Mesh", j, "Geometry");
    link(&mut t, r, "Geometry", j, "Geometry");
    link(&mut t, j, "Geometry", 1, "Geometry");
    let m = mesh_of(&run(&t)[0]);
    assert_eq!(m.positions.len(), 72 + 9);
}

#[test]
fn distribute_is_deterministic_per_seed() {
    let build = |seed: i32| {
        let mut t = tree(&[], &[("Geometry", SocketType::Geometry)]);
        let grid = add(&mut t, "GeometryNodeMeshGrid");
        t.nodes[grid].set_input("Size X", Value::Float(4.0));
        t.nodes[grid].set_input("Size Y", Value::Float(4.0));
        let d = add(&mut t, "GeometryNodeDistributePointsOnFaces");
        t.nodes[d].set_input("Seed", Value::Int(seed));
        link(&mut t, grid, "Mesh", d, "Mesh");
        link(&mut t, d, "Points", 1, "Geometry");
        run(&t)[0].geometry().points.unwrap()
    };
    let a = build(0);
    let b = build(0);
    let c = build(7);
    assert_eq!(a.positions, b.positions);
    assert_ne!(a.positions, c.positions);
    // Density 10 on 16 square units: about 160 points, all on the grid.
    assert!((100..230).contains(&a.positions.len()), "{}", a.positions.len());
    assert!(a.positions.iter().all(|p| p.x.abs() <= 2.0 + 1e-5 && p.y.abs() <= 2.0 + 1e-5 && p.z.abs() < 1e-5));
    assert!(a.attributes.get("id").is_some());
}

#[test]
fn repeat_zone_iterations() {
    // Translate a point by +1 z per iteration.
    let mut t = tree(&[], &[("Geometry", SocketType::Geometry)]);
    let pts = add(&mut t, "GeometryNodePoints");
    let rin = add(&mut t, "GeometryNodeRepeatInput");
    let rout = add(&mut t, "GeometryNodeRepeatOutput");
    let name = t.nodes[rout].name.clone();
    t.nodes[rin].set_prop("paired_output", Prop::String(name));
    let tr = add(&mut t, "GeometryNodeTransform");
    t.nodes[tr].set_input("Translation", Value::Vector([0.0, 0.0, 1.0]));
    t.nodes[rin].set_input("Iterations", Value::Int(5));
    link(&mut t, pts, "Geometry", rin, "Item_0");
    link(&mut t, rin, "Item_0", tr, "Geometry");
    link(&mut t, tr, "Geometry", rout, "Item_0");
    link(&mut t, rout, "Item_0", 1, "Geometry");
    let z = |t: &NodeTree| run(t)[0].geometry().points.unwrap().positions[0].z;
    assert_eq!(z(&t), 5.0);
    t.nodes[rin].set_input("Iterations", Value::Int(0));
    assert_eq!(z(&t), 0.0);
    // A float item counts the iterations: the sum 0+1+..+4 via Iteration.
    let mut t = tree(&[], &[("Value", SocketType::Float)]);
    let rin = add(&mut t, "GeometryNodeRepeatInput");
    let rout = add(&mut t, "GeometryNodeRepeatOutput");
    let id = crate::nodes::geo_registry::add_zone_item(&mut t, rin, rout, "Sum", SocketType::Float);
    t.nodes[rin].set_input("Iterations", Value::Int(5));
    let math = add(&mut t, "ShaderNodeMath");
    link(&mut t, rin, &id, math, "Value");
    link(&mut t, rin, "Iteration", math, "Value_001");
    link(&mut t, math, "Value", rout, &id);
    link(&mut t, rout, &id, 1, "Value");
    assert_eq!(run(&t)[0].val().f(), 10.0);
}

#[test]
fn simulation_zone_across_frames() {
    clear_simulation_cache();
    // A float state that adds Delta Time * 24 every frame, and a point that
    // moves up 1 per step.
    let mut t = tree(&[], &[("Value", SocketType::Float), ("Geometry", SocketType::Geometry)]);
    t.name = "SimTest".into();
    let sin = add(&mut t, "GeometryNodeSimulationInput");
    let sout = add(&mut t, "GeometryNodeSimulationOutput");
    let id = crate::nodes::geo_registry::add_zone_item(&mut t, sin, sout, "Time", SocketType::Float);
    let scale = add(&mut t, "ShaderNodeMath");
    t.nodes[scale].set_prop("operation", Prop::Enum("MULTIPLY".into()));
    t.nodes[scale].set_input("Value_001", Value::Float(24.0));
    let sum = add(&mut t, "ShaderNodeMath");
    link(&mut t, sin, "Delta Time", scale, "Value");
    link(&mut t, sin, &id, sum, "Value");
    link(&mut t, scale, "Value", sum, "Value_001");
    link(&mut t, sum, "Value", sout, &id);
    link(&mut t, sout, &id, 1, "Value");
    let pts = add(&mut t, "GeometryNodePoints");
    let tr = add(&mut t, "GeometryNodeTransform");
    t.nodes[tr].set_input("Translation", Value::Vector([0.0, 0.0, 1.0]));
    link(&mut t, pts, "Geometry", sin, "Item_0");
    link(&mut t, sin, "Item_0", tr, "Geometry");
    link(&mut t, tr, "Geometry", sout, "Item_0");
    link(&mut t, sout, "Item_0", 1, "Geometry");
    let at = |frame: f32| {
        let o = evaluate_group(&t, Vec::new(), &EvalOptions { frame, ..Default::default() });
        (o[0].val().f(), o[1].geometry().points.unwrap().positions[0].z)
    };
    // Frame 1 (the start) runs one step with a zero delta time.
    assert_eq!(at(1.0), (0.0, 1.0));
    let (time, z) = at(5.0);
    assert!((time - 4.0).abs() < 1e-4, "{time}");
    assert_eq!(z, 5.0);
    // Going back reads the cache; going forward continues from it.
    let (time, _) = at(3.0);
    assert!((time - 2.0).abs() < 1e-4);
    assert!((at(10.0).0 - 9.0).abs() < 1e-4);
    // Before the start the zone passes its input through.
    assert_eq!(at(0.0).1, 0.0);
    // The start frame resets.
    assert_eq!(at(1.0), (0.0, 1.0));
    assert!((at(2.0).0 - 1.0).abs() < 1e-4);
    clear_simulation_cache();
}

#[test]
fn group_nodes_and_fields_cross_groups() {
    // An inner group that offsets a geometry by a vector input.
    let mut inner = tree(&[("Geometry", SocketType::Geometry), ("Offset", SocketType::Vector)], &[("Geometry", SocketType::Geometry)]);
    inner.name = "Inner".into();
    let sp = add(&mut inner, "GeometryNodeSetPosition");
    link(&mut inner, 0, "Geometry", sp, "Geometry");
    link(&mut inner, 0, "Offset", sp, "Offset");
    link(&mut inner, sp, "Geometry", 1, "Geometry");
    let mut scene = crate::scene::Scene::new();
    scene.node_groups.push(inner);
    let mut t = tree(&[], &[("Geometry", SocketType::Geometry)]);
    let grid = add(&mut t, "GeometryNodeMeshGrid");
    let g = edit::add_node(&mut t, "group:0", [0.0, 0.0], &scene.node_groups, None).expect("group node");
    // The offset is a field: the position itself, doubling the grid.
    let pos = add(&mut t, "GeometryNodeInputPosition");
    link(&mut t, grid, "Mesh", g, "Geometry");
    link(&mut t, pos, "Position", g, "Offset");
    link(&mut t, g, "Geometry", 1, "Geometry");
    let out = evaluate_group(&t, Vec::new(), &EvalOptions { scene: Some(&scene), ..Default::default() });
    let m = mesh_of(&out[0]);
    let max = m.positions.iter().fold(0.0f32, |a, p| a.max(p.x));
    assert!((max - 1.0).abs() < 1e-5, "{max}");
}

fn nodes_scene() -> (crate::scene::Scene, usize) {
    let mut s = crate::scene::Scene::new();
    let o = s.add_mesh_object("Plane", primitives::grid(2.0, 2.0, 2, 2));
    let mut g = new_group("Geometry Nodes");
    g.inputs.push(InterfaceSocket { name: "Height".into(), ty: SocketType::Float, default: Value::Float(1.0), min: 0.0, max: 10.0 });
    let gi = edit::group_input_node(&g);
    g.nodes[0] = gi;
    let ex = add(&mut g, "GeometryNodeExtrudeMesh");
    g.links.clear();
    link(&mut g, 0, "Geometry", ex, "Mesh");
    link(&mut g, 0, "Height", ex, "Offset Scale");
    link(&mut g, ex, "Mesh", 1, "Geometry");
    s.node_groups.push(g);
    s.objects[o].modifiers.push(crate::modifiers::Modifier::Nodes { group: "Geometry Nodes".into(), inputs: vec![("Height".into(), Value::Float(2.5))] });
    (s, o)
}

#[test]
fn nodes_modifier_in_stack() {
    let (mut s, o) = nodes_scene();
    let m = s.evaluated_mesh(o).unwrap();
    assert_eq!(counts(&m), (8, 12, 5));
    let top = m.positions.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    assert!((top - 2.5).abs() < 1e-5, "{top}");
    // A later modifier sees the node output.
    s.objects[o].modifiers.push(crate::modifiers::Modifier::with_defaults("triangulate").unwrap());
    let m = s.evaluated_mesh(o).unwrap();
    assert_eq!(m.faces.len(), 10);
    // A missing group passes the mesh through; an empty group too.
    s.objects[o].modifiers.truncate(0);
    s.objects[o].modifiers.push(crate::modifiers::Modifier::Nodes { group: "Nope".into(), inputs: Vec::new() });
    assert_eq!(s.evaluated_mesh(o).unwrap().faces.len(), 1);
    let fresh = new_group("Plain");
    s.node_groups.push(fresh);
    s.objects[o].modifiers[0] = crate::modifiers::Modifier::Nodes { group: "Plain".into(), inputs: Vec::new() };
    assert_eq!(counts(&s.evaluated_mesh(o).unwrap()), (4, 4, 1));
    // Interface inputs the modifier panel shows.
    let ui = eval::modifier_interface(&s.node_groups[0]);
    assert_eq!(ui.len(), 1);
    assert_eq!(ui[0].0, "Height");
}

#[test]
fn nodes_modifier_polyops_round_trip() {
    let (s, o) = nodes_scene();
    let text = crate::script::scene_to_polyops(&s);
    assert!(text.contains("kind=geometry"), "{text}");
    assert!(text.contains("group="), "{text}");
    let mut back = crate::scene::Scene::new();
    crate::script::run(&mut back, &text).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
    let bo = back.objects.iter().position(|x| x.name == "Plane").unwrap();
    assert_eq!(back.objects[bo].modifiers, s.objects[o].modifiers);
    let g = back.node_groups.iter().find(|g| g.name == "Geometry Nodes").unwrap();
    assert_eq!(g.kind, TreeKind::Geometry);
    assert_eq!(g.nodes.len(), 3);
    assert_eq!(g.links.len(), 3);
    assert_eq!(counts(&back.evaluated_mesh(bo).unwrap()), (8, 12, 5));
    // The command form.
    let mut s2 = crate::scene::Scene::new();
    crate::script::run(&mut s2, &text).unwrap();
    let o2 = s2.objects.iter().position(|x| x.name == "Plane").unwrap();
    s2.objects[o2].modifiers.clear();
    crate::script::run(&mut s2, "polyops 1\nmodifier_add Plane Nodes group=\"Geometry Nodes\" input:Height=0.5").unwrap();
    assert_eq!(s2.objects[o2].modifiers, vec![crate::modifiers::Modifier::Nodes { group: "Geometry Nodes".into(), inputs: vec![("Height".into(), Value::Float(0.5))] }]);
    let top = s2.evaluated_mesh(o2).unwrap().positions.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    assert!((top - 0.5).abs() < 1e-5);
}

#[test]
fn curves_to_mesh_and_resample() {
    let mut t = tree(&[], &[("Geometry", SocketType::Geometry)]);
    let circle = add(&mut t, "GeometryNodeCurvePrimitiveCircle");
    let profile = add(&mut t, "GeometryNodeCurvePrimitiveCircle");
    t.nodes[circle].set_input("Resolution", Value::Int(16));
    t.nodes[profile].set_input("Resolution", Value::Int(8));
    t.nodes[profile].set_input("Radius", Value::Float(0.1));
    let c2m = add(&mut t, "GeometryNodeCurveToMesh");
    link(&mut t, circle, "Curve", c2m, "Curve");
    link(&mut t, profile, "Curve", c2m, "Profile Curve");
    link(&mut t, c2m, "Mesh", 1, "Geometry");
    let m = mesh_of(&run(&t)[0]);
    // A torus: 16 x 8 quads, closed.
    assert_eq!(counts(&m), (128, 256, 128));
    // Without a profile, a cyclic curve becomes a closed edge loop.
    t.links.retain(|l| !(l.to_node == c2m && l.to_socket == 1));
    let m = mesh_of(&run(&t)[0]);
    assert_eq!(counts(&m), (16, 16, 0));
    // Resample to a count.
    let rs = add(&mut t, "GeometryNodeResampleCurve");
    t.nodes[rs].set_input("Count", Value::Int(5));
    link(&mut t, circle, "Curve", rs, "Curve");
    link(&mut t, rs, "Curve", c2m, "Curve");
    let m = mesh_of(&run(&t)[0]);
    assert_eq!(m.positions.len(), 5);
}

#[test]
fn accumulate_and_sample_index() {
    // Accumulate the value 1 over a line's points: positions z = 1..=10.
    let mut t = tree(&[], &[("Geometry", SocketType::Geometry)]);
    let line = add(&mut t, "GeometryNodeMeshLine");
    t.nodes[line].set_input("Offset", Value::Vector([0.0, 0.0, 0.0]));
    let acc = add(&mut t, "GeometryNodeAccumulateField");
    t.nodes[acc].set_input("Value", Value::Float(1.0));
    let comb = add(&mut t, "ShaderNodeCombineXYZ");
    let sp = add(&mut t, "GeometryNodeSetPosition");
    link(&mut t, acc, "Leading", comb, "Z");
    link(&mut t, line, "Mesh", sp, "Geometry");
    link(&mut t, comb, "Vector", sp, "Position");
    link(&mut t, sp, "Geometry", 1, "Geometry");
    let m = mesh_of(&run(&t)[0]);
    assert_eq!(m.positions.iter().map(|p| p.z).collect::<Vec<_>>(), (1..=10).map(|i| i as f32).collect::<Vec<_>>());
}
