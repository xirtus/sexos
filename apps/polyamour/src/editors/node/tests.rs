// SPDX-License-Identifier: GPL-3.0-only
use super::*;
use crate::editors::Editor;
use crate::App;

fn shading() -> App {
    let mut app = App::new(1600, 900);
    app.doc.want_workspace = Some("Shading".into());
    app.frame(Input::Tick);
    app.frame(Input::Tick);
    app
}

fn ed(app: &App) -> &State {
    for a in app.workspaces[app.active_ws].areas() {
        if let Editor::Node(s) = &a.editor {
            return s;
        }
    }
    panic!("no node editor")
}

fn tree(app: &App) -> &NodeTree {
    let s = ed(app);
    tree_ref(s, &app.doc).and_then(|r| r.get(&app.doc.scene)).expect("tree")
}

fn node(app: &App, name: &str) -> usize {
    tree(app).find(name).unwrap_or_else(|| panic!("node {name} in {:?}", tree(app).nodes.iter().map(|n| &n.name).collect::<Vec<_>>()))
}

fn sock(app: &App, name: &str, input: bool, ident: &str) -> (i32, i32) {
    let n = node(app, name);
    let t = tree(app);
    let i = if input { t.nodes[n].input(ident) } else { t.nodes[n].output(ident) }.expect("socket");
    let l = &ed(app).layout[n];
    let p = if input { l.inputs[i] } else { l.outputs[i] }.expect("socket drawn");
    (p.0.round() as i32, p.1.round() as i32)
}

fn key(app: &mut App, k: Key, m: Mods) {
    app.frame(Input::key_mods(k, m));
}

fn click(app: &mut App, x: i32, y: i32, mods: Mods) {
    app.frame(Input::Move { x, y });
    app.frame(Input::Down { x, y, button: Button::Left, mods });
    app.frame(Input::Up { x, y, button: Button::Left });
}

fn drag(app: &mut App, a: (i32, i32), b: (i32, i32), button: Button, mods: Mods) {
    app.frame(Input::Move { x: a.0, y: a.1 });
    app.frame(Input::Down { x: a.0, y: a.1, button, mods });
    for k in 1..=4 {
        let (x, y) = (a.0 + (b.0 - a.0) * k / 4, a.1 + (b.1 - a.1) * k / 4);
        app.frame(Input::Move { x, y });
    }
    app.frame(Input::Up { x: b.0, y: b.1, button });
}

/// Add a node with the editor's operator at (x, y) and click to place it.
fn add(app: &mut App, idname: &str, x: i32, y: i32) -> usize {
    app.frame(Input::Move { x, y });
    let id = ed(app).id;
    app.doc.queue.push(anim::mail("node.add_node", id, idname));
    app.frame(Input::Tick);
    click(app, x, y, Mods::NONE);
    tree(app).nodes.len() - 1
}

fn center(app: &App, n: usize) -> (i32, i32) {
    let r = ed(app).layout[n].rect;
    (r.x + r.w / 2, r.y + 8)
}

fn canvas(app: &App) -> Rect {
    ed(app).view.region
}

#[test]
fn roughness_field_edits_the_material_and_undoes() {
    let mut app = shading();
    let r = ed(&app).fields.iter().find(|f| f.0 == "Principled BSDF/Roughness").expect("roughness field").1;
    let (x, y) = r.center();
    click(&mut app, x, y, Mods::NONE);
    for c in "0.2".chars() {
        app.frame(Input::Key { key: Key::Char(c), mods: Mods::NONE, text: Some(c) });
    }
    key(&mut app, Key::Enter, Mods::NONE);
    let mi = app.doc.scene.objects[app.doc.active().unwrap()].materials[0];
    // The tree and the legacy field both change.
    let t = app.doc.scene.materials[mi].node_tree.as_ref().unwrap();
    let p = t.find_type("ShaderNodeBsdfPrincipled").unwrap();
    assert_eq!(t.nodes[p].inputs[t.nodes[p].input("Roughness").unwrap()].value, Value::Float(0.2));
    assert!((app.doc.scene.materials[mi].roughness - 0.2).abs() < 1e-6);
    // Dragging changes it too, as one undo step.
    app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Move { x: x + 30, y });
    app.frame(Input::Move { x: x + 40, y });
    app.frame(Input::Up { x: x + 40, y, button: Button::Left });
    assert!(app.doc.scene.materials[mi].roughness > 0.25);
    app.frame(Input::Move { x: 300, y: 300 });
    key(&mut app, Key::Char('z'), Mods::CTRL);
    assert!((app.doc.scene.materials[mi].roughness - 0.2).abs() < 1e-6);
    key(&mut app, Key::Char('z'), Mods::CTRL);
    assert!((app.doc.scene.materials[mi].roughness - 0.5).abs() < 1e-6);
    key(&mut app, Key::Char('z'), Mods::CTRL_SHIFT);
    assert!((app.doc.scene.materials[mi].roughness - 0.2).abs() < 1e-6);
}

#[test]
fn shift_a_menu_and_search_add_nodes() {
    let mut app = shading();
    let c = canvas(&app);
    let (x, y) = (c.x + 120, c.y + 120);
    app.frame(Input::Move { x, y });
    key(&mut app, Key::Char('a'), Mods::SHIFT);
    let m = ed(&app).add_menu.clone().expect("add menu open");
    let cats = addmenu::categories(TreeKind::Shader, &app.doc.scene.node_groups, None);
    let ci = cats.iter().position(|c| c.0 == "Converter").unwrap();
    let row = m.row_rect(&cats, c, ci + 1).unwrap();
    app.frame(Input::Move { x: row.x + 20, y: row.y + 10 });
    let m = ed(&app).add_menu.clone().unwrap();
    assert_eq!(m.sub, Some(ci));
    let k = cats[ci].1.iter().position(|e| e.idname == "ShaderNodeMath").unwrap();
    let er = m.entry_rect(&cats, c, k).unwrap();
    let n0 = tree(&app).nodes.len();
    click(&mut app, er.x + 20, er.y + 10, Mods::NONE);
    assert!(ed(&app).add_menu.is_none());
    assert_eq!(tree(&app).nodes.len(), n0 + 1);
    // The new node follows the pointer until a click.
    let before = tree(&app).nodes[n0].location;
    app.frame(Input::Move { x: x + 50, y: y + 20 });
    let after = tree(&app).nodes[n0].location;
    assert!((after[0] - before[0]).abs() > 1.0, "{before:?} {after:?}");
    click(&mut app, x + 50, y + 20, Mods::NONE);
    assert!(!ed(&app).is_modal());
    // Search: type to filter, Enter adds the first match.
    key(&mut app, Key::Char('a'), Mods::SHIFT);
    for ch in "emiss".chars() {
        app.frame(Input::Key { key: Key::Char(ch), mods: Mods::NONE, text: Some(ch) });
    }
    key(&mut app, Key::Enter, Mods::NONE);
    click(&mut app, x, y + 60, Mods::NONE);
    assert_eq!(tree(&app).nodes.last().unwrap().idname, "ShaderNodeEmission");
    // Search finds Math operations too.
    key(&mut app, Key::Char('a'), Mods::SHIFT);
    for ch in "math mult".chars() {
        app.frame(Input::Key { key: Key::Char(ch), mods: Mods::NONE, text: Some(ch) });
    }
    key(&mut app, Key::Enter, Mods::NONE);
    click(&mut app, x, y + 100, Mods::NONE);
    let last = tree(&app).nodes.last().unwrap();
    assert_eq!((last.idname.as_str(), last.enum_prop("operation")), ("ShaderNodeMath", "MULTIPLY"));
    // Each add is one undo step.
    let n = tree(&app).nodes.len();
    key(&mut app, Key::Char('z'), Mods::CTRL);
    assert_eq!(tree(&app).nodes.len(), n - 1);
}

#[test]
fn link_replace_pick_up_and_cut() {
    let mut app = shading();
    let c = canvas(&app);
    let v = add(&mut app, "ShaderNodeValue", c.x + 90, c.y + 60);
    let rgb = add(&mut app, "ShaderNodeRGB", c.x + 90, c.y + 200);
    let vname = tree(&app).nodes[v].name.clone();
    let rname = tree(&app).nodes[rgb].name.clone();
    let p = node(&app, "Principled BSDF");
    let rough = tree(&app).nodes[p].input("Roughness").unwrap();
    // Drag Value's output onto Roughness.
    {
        let (a, b) = (sock(&app, &vname, false, "Value"), sock(&app, "Principled BSDF", true, "Roughness"));
        drag(&mut app, a, b, Button::Left, Mods::NONE);
    }
    assert_eq!(tree(&app).input_link(p, rough).map(|l| l.from_node), Some(node(&app, &vname)));
    // Linked inputs hide their value field.
    assert!(!ed(&app).fields.iter().any(|f| f.0 == "Principled BSDF/Roughness"));
    // Dropping RGB on the same input replaces the link.
    {
        let (a, b) = (sock(&app, &rname, false, "Color"), sock(&app, "Principled BSDF", true, "Roughness"));
        drag(&mut app, a, b, Button::Left, Mods::NONE);
    }
    let t = tree(&app);
    assert_eq!(t.links.iter().filter(|l| l.to_node == p && l.to_socket == rough).count(), 1);
    assert_eq!(t.input_link(p, rough).map(|l| l.from_node), Some(node(&app, &rname)));
    // Dragging from the linked input picks the link up; dropped on Metallic it moves there.
    {
        let (a, b) = (sock(&app, "Principled BSDF", true, "Roughness"), sock(&app, "Principled BSDF", true, "Metallic"));
        drag(&mut app, a, b, Button::Left, Mods::NONE);
    }
    let met = tree(&app).nodes[p].input("Metallic").unwrap();
    assert!(tree(&app).input_link(p, rough).is_none());
    assert_eq!(tree(&app).input_link(p, met).map(|l| l.from_node), Some(node(&app, &rname)));
    // Ctrl+right-drag across the link cuts it.
    let a = sock(&app, &rname, false, "Color");
    let b = sock(&app, "Principled BSDF", true, "Metallic");
    let mid = ((a.0 + b.0) / 2, (a.1 + b.1) / 2);
    let links = tree(&app).links.len();
    drag(&mut app, (mid.0, mid.1 - 60), (mid.0, mid.1 + 60), Button::Right, Mods::CTRL);
    assert_eq!(tree(&app).links.len(), links - 1, "{:?}", tree(&app).links);
    // Undo brings it back.
    key(&mut app, Key::Char('z'), Mods::CTRL);
    assert_eq!(tree(&app).links.len(), links);
    // Shift+right-drag across it adds a reroute in the link.
    drag(&mut app, (mid.0, mid.1 - 60), (mid.0, mid.1 + 60), Button::Right, Mods::SHIFT);
    let t = tree(&app);
    let r = t.find_type("NodeReroute").expect("reroute");
    assert!(t.links.iter().any(|l| l.to_node == r) && t.links.iter().any(|l| l.from_node == r && l.to_node == p));
}

#[test]
fn delete_dissolve_duplicate_mute_and_hide() {
    let mut app = shading();
    let c = canvas(&app);
    let v = add(&mut app, "ShaderNodeValue", c.x + 60, c.y + 60);
    let m = add(&mut app, "ShaderNodeMath", c.x + 60, c.y + 200);
    let (vn, mn) = (tree(&app).nodes[v].name.clone(), tree(&app).nodes[m].name.clone());
    {
        let (a, b) = (sock(&app, &vn, false, "Value"), sock(&app, &mn, true, "Value"));
        drag(&mut app, a, b, Button::Left, Mods::NONE);
    }
    {
        let (a, b) = (sock(&app, &mn, false, "Value"), sock(&app, "Principled BSDF", true, "Roughness"));
        drag(&mut app, a, b, Button::Left, Mods::NONE);
    }
    // Ctrl+X on Math links Value straight to Roughness.
    let mc = center(&app, node(&app, &mn));
    click(&mut app, mc.0, mc.1, Mods::NONE);
    key(&mut app, Key::Char('x'), Mods::CTRL);
    assert!(tree(&app).find(&mn).is_none());
    let p = node(&app, "Principled BSDF");
    let rough = tree(&app).nodes[p].input("Roughness").unwrap();
    assert_eq!(tree(&app).input_link(p, rough).map(|l| l.from_node), Some(node(&app, &vn)));
    // Shift+D duplicates and grabs; a click places the copy.
    let vc = center(&app, node(&app, &vn));
    click(&mut app, vc.0, vc.1, Mods::NONE);
    let n = tree(&app).nodes.len();
    key(&mut app, Key::Char('d'), Mods::SHIFT);
    app.frame(Input::Move { x: vc.0 + 40, y: vc.1 + 80 });
    click(&mut app, vc.0 + 40, vc.1 + 80, Mods::NONE);
    assert_eq!(tree(&app).nodes.len(), n + 1);
    let copy = tree(&app).nodes.last().unwrap();
    assert!(copy.select && copy.name.starts_with("Value"));
    // M mutes, H collapses, X deletes.
    key(&mut app, Key::Char('m'), Mods::NONE);
    assert!(tree(&app).nodes.last().unwrap().mute);
    key(&mut app, Key::Char('h'), Mods::NONE);
    assert!(tree(&app).nodes.last().unwrap().hide);
    key(&mut app, Key::Char('x'), Mods::NONE);
    assert_eq!(tree(&app).nodes.len(), n);
    // Ctrl+C / Ctrl+V.
    let vc = center(&app, node(&app, &vn));
    click(&mut app, vc.0, vc.1, Mods::NONE);
    key(&mut app, Key::Char('c'), Mods::CTRL);
    app.frame(Input::Move { x: vc.0 + 100, y: vc.1 });
    key(&mut app, Key::Char('v'), Mods::CTRL);
    assert_eq!(tree(&app).nodes.len(), n + 1);
    // Box select from empty space, then A / Alt+A.
    key(&mut app, Key::Char('a'), Mods::ALT);
    assert!(tree(&app).nodes.iter().all(|n| !n.select));
    key(&mut app, Key::Char('a'), Mods::NONE);
    assert!(tree(&app).nodes.iter().all(|n| n.select));
}

#[test]
fn group_make_edit_and_ungroup() {
    let mut app = shading();
    let c = canvas(&app);
    let v = add(&mut app, "ShaderNodeValue", c.x + 60, c.y + 60);
    let m = add(&mut app, "ShaderNodeMath", c.x + 60, c.y + 200);
    let (vn, mn) = (tree(&app).nodes[v].name.clone(), tree(&app).nodes[m].name.clone());
    {
        let (a, b) = (sock(&app, &vn, false, "Value"), sock(&app, &mn, true, "Value"));
        drag(&mut app, a, b, Button::Left, Mods::NONE);
    }
    {
        let (a, b) = (sock(&app, &mn, false, "Value"), sock(&app, "Principled BSDF", true, "Roughness"));
        drag(&mut app, a, b, Button::Left, Mods::NONE);
    }
    let before = (tree(&app).nodes.len(), tree(&app).links.len());
    let mc = center(&app, node(&app, &mn));
    click(&mut app, mc.0, mc.1, Mods::NONE);
    key(&mut app, Key::Char('g'), Mods::CTRL);
    assert_eq!(app.doc.scene.node_groups.len(), 1);
    let g = &app.doc.scene.node_groups[0];
    assert_eq!((g.inputs.len(), g.outputs.len()), (1, 1));
    let gn = tree(&app).find_type("ShaderNodeGroup").expect("group node");
    assert!(tree(&app).links.iter().any(|l| l.to_node == gn) && tree(&app).links.iter().any(|l| l.from_node == gn));
    // Tab enters the group, Tab again leaves it.
    key(&mut app, Key::Tab, Mods::NONE);
    assert_eq!(ed(&app).path, vec![0]);
    assert!(tree(&app).find_type("NodeGroupInput").is_some() && tree(&app).find_type("ShaderNodeMath").is_some());
    key(&mut app, Key::Tab, Mods::NONE);
    assert!(ed(&app).path.is_empty());
    // Ctrl+Alt+G ungroups back to the same nodes and links.
    let gn = tree(&app).find_type("ShaderNodeGroup").unwrap();
    let gc = center(&app, gn);
    click(&mut app, gc.0, gc.1, Mods::NONE);
    key(&mut app, Key::Char('g'), Mods { shift: false, ctrl: true, alt: true });
    assert!(tree(&app).find_type("ShaderNodeGroup").is_none());
    assert_eq!((tree(&app).nodes.len(), tree(&app).links.len()), before);
    // Undo returns to the grouped tree, redo ungroups again.
    key(&mut app, Key::Char('z'), Mods::CTRL);
    assert!(tree(&app).find_type("ShaderNodeGroup").is_some());
    key(&mut app, Key::Char('z'), Mods::CTRL_SHIFT);
    assert!(tree(&app).find_type("ShaderNodeGroup").is_none());
}

#[test]
fn frames_carry_their_nodes() {
    let mut app = shading();
    let c = canvas(&app);
    let v = add(&mut app, "ShaderNodeValue", c.x + 200, c.y + 100);
    let vn = tree(&app).nodes[v].name.clone();
    let vc = center(&app, node(&app, &vn));
    click(&mut app, vc.0, vc.1, Mods::NONE);
    key(&mut app, Key::Char('j'), Mods::CTRL);
    let f = tree(&app).find_type("NodeFrame").expect("frame");
    assert_eq!(tree(&app).nodes[node(&app, &vn)].parent, Some(f));
    // Moving the frame moves the node inside.
    let before = tree(&app).nodes[node(&app, &vn)].location;
    let fr = ed(&app).layout[f].rect;
    drag(&mut app, (fr.x + 5, fr.y + 5), (fr.x + 65, fr.y + 5), Button::Left, Mods::NONE);
    let after = tree(&app).nodes[node(&app, &vn)].location;
    assert!(after[0] - before[0] > 30.0, "{before:?} {after:?}");
    // Alt+P takes it out.
    let vc = center(&app, node(&app, &vn));
    click(&mut app, vc.0, vc.1, Mods::NONE);
    key(&mut app, Key::Char('p'), Mods::ALT);
    assert_eq!(tree(&app).nodes[node(&app, &vn)].parent, None);
}

#[test]
fn enum_dropdown_updates_sockets() {
    let mut app = shading();
    let c = canvas(&app);
    let m = add(&mut app, "ShaderNodeMath", c.x + 60, c.y + 60);
    let r = TreeRef::Material(0).encode();
    app.doc.queue.push(Op::with("node.set_prop", &format!("{r}\t{m}\toperation\tSINE")));
    app.frame(Input::Tick);
    let n = &tree(&app).nodes[m];
    assert_eq!(n.enum_prop("operation"), "SINE");
    assert!(!n.inputs[1].enabled);
    app.frame(Input::Tick);
    assert!(ed(&app).layout[m].inputs[1].is_none(), "disabled sockets are not drawn");
}

#[test]
fn compositor_and_geometry_editors_use_the_same_canvas() {
    let mut app = App::new(1600, 900);
    app.doc.want_workspace = Some("Geometry Nodes".into());
    app.frame(Input::Tick);
    app.frame(Input::Tick);
    let id = ed(&app).id;
    app.doc.queue.push(anim::mail("node.geo_new", id, ""));
    app.frame(Input::Tick);
    app.frame(Input::Tick);
    let t = tree(&app);
    assert_eq!(t.kind, TreeKind::Geometry);
    assert_eq!(t.links.len(), 1);
    // "New" puts the group on the active mesh object as a Nodes modifier,
    // which passes the mesh through unchanged.
    let a = app.doc.active().expect("active object");
    let group = t.name.clone();
    assert_eq!(app.doc.scene.objects[a].modifiers.last(), Some(&sex_3d::modifiers::Modifier::Nodes { group, inputs: Vec::new() }));
    let before = app.doc.scene.meshes[match app.doc.scene.objects[a].data {
        sex_3d::scene::ObjectData::Mesh(m) => m,
        _ => panic!("active object is not a mesh"),
    }]
    .positions
    .len();
    assert_eq!(app.doc.scene.evaluated_mesh(a).unwrap().positions.len(), before);
    // Frames and reroutes work in any tree.
    let c = canvas(&app);
    add(&mut app, "NodeReroute", c.x + 200, c.y + 200);
    assert_eq!(tree(&app).nodes.last().unwrap().idname, "NodeReroute");
}

#[test]
fn material_properties_lists_the_surface_inputs() {
    let mut app = shading();
    // Link a Value into Roughness: the Surface panel shows the link.
    let c = canvas(&app);
    let v = add(&mut app, "ShaderNodeValue", c.x + 60, c.y + 60);
    let vn = tree(&app).nodes[v].name.clone();
    {
        let (a, b) = (sock(&app, &vn, false, "Value"), sock(&app, "Principled BSDF", true, "Metallic"));
        drag(&mut app, a, b, Button::Left, Mods::NONE);
    }
    let mut l_app = App::new(1600, 900);
    std::mem::swap(&mut l_app.doc, &mut app.doc);
    let mut app = l_app;
    app.frame(Input::Tick);
    let props = app.workspaces[app.active_ws].areas().into_iter().find_map(|a| match &a.editor {
        Editor::Properties(_) => Some(a.id),
        _ => None,
    });
    let pid = props.expect("properties");
    if let Some(Editor::Properties(p)) = app.workspaces[app.active_ws].area_mut(pid).map(|a| &mut a.editor) {
        p.tab = crate::editors::properties::Tab::Material;
    }
    app.frame(Input::Tick);
    app.frame(Input::Tick);
    let fields = match &app.workspaces[app.active_ws].area_mut(pid).unwrap().editor {
        Editor::Properties(p) => p.fields.clone(),
        _ => unreachable!(),
    };
    let names: Vec<&str> = fields.iter().map(|f| f.0.as_str()).collect();
    assert!(names.contains(&"Roughness") && names.contains(&"Base Color"), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("Metallic: Value")), "{names:?}");
}
