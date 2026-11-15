// SPDX-License-Identifier: GPL-3.0-only
//! Node trees in `.poly` files. Polyops has no words for node trees yet, so
//! each tree rides in `#@` lines after the scene:
//!
//! ```text
//! #@ntree OWNER\tKEY\tKIND\tNAME\tACTIVE      owner: group|material|world|compositor
//! #@niface in|out\tNAME\tTYPE\tDEFAULT\tMIN\tMAX
//! #@node NAME\tIDNAME\tLABEL\tX\tY\tWIDTH\tMUTE\tHIDE\tSELECT\tPARENT\tCOLOR
//! #@nsock in|out\tIDENT\tNAME\tTYPE\tVALUE\tHIDE\tENABLED\tHIDE_VALUE\tMIN\tMAX
//! #@nprop NAME\tVALUE
//! #@link FROM\tFROM_SOCKET\tTO\tTO_SOCKET\tMUTED
//! ```
//!
//! `nsock` and `nprop` lines belong to the `node` line before them; nodes and
//! sockets are referred to by their order. Materials are keyed by name. A
//! tree polyops itself already loaded wins over these lines.
use super::{esc, unesc};
use sex_3d::nodes::{edit, ColorRamp, CurveMapping, InterfaceSocket, Link, Node, NodeTree, Prop, Socket, SocketType, TreeKind, Value};
use sex_3d::scene::Scene;

const TYPES: [(SocketType, &str); 17] = [
    (SocketType::Float, "FLOAT"),
    (SocketType::Int, "INT"),
    (SocketType::Bool, "BOOL"),
    (SocketType::Vector, "VECTOR"),
    (SocketType::Color, "RGBA"),
    (SocketType::String, "STRING"),
    (SocketType::Shader, "SHADER"),
    (SocketType::Geometry, "GEOMETRY"),
    (SocketType::Object, "OBJECT"),
    (SocketType::Collection, "COLLECTION"),
    (SocketType::Image, "IMAGE"),
    (SocketType::Material, "MATERIAL"),
    (SocketType::Texture, "TEXTURE"),
    (SocketType::Rotation, "ROTATION"),
    (SocketType::Matrix, "MATRIX"),
    (SocketType::Menu, "MENU"),
    (SocketType::Virtual, "VIRTUAL"),
];

fn ty_name(t: SocketType) -> &'static str {
    TYPES.iter().find(|x| x.0 == t).map(|x| x.1).unwrap_or("FLOAT")
}

fn ty_parse(s: &str) -> SocketType {
    TYPES.iter().find(|x| x.1 == s).map(|x| x.0).unwrap_or(SocketType::Float)
}

fn kind_name(k: TreeKind) -> &'static str {
    match k {
        TreeKind::Shader => "SHADER",
        TreeKind::Compositor => "COMPOSITOR",
        TreeKind::Geometry => "GEOMETRY",
    }
}

fn kind_parse(s: &str) -> TreeKind {
    match s {
        "COMPOSITOR" => TreeKind::Compositor,
        "GEOMETRY" => TreeKind::Geometry,
        _ => TreeKind::Shader,
    }
}

fn nums(v: &[f32]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

fn parse_nums<const N: usize>(s: &str) -> Option<[f32; N]> {
    let v: Vec<f32> = s.split(',').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    v.try_into().ok()
}

fn idx(i: Option<usize>) -> String {
    i.map(|i| i.to_string()).unwrap_or_else(|| "-".into())
}

fn enc_value(v: &Value) -> String {
    match v {
        Value::None => "n".into(),
        Value::Float(f) => format!("f:{f}"),
        Value::Int(i) => format!("i:{i}"),
        Value::Bool(b) => format!("b:{}", *b as u8),
        Value::Vector(x) => format!("v:{}", nums(x)),
        Value::Color(c) => format!("c:{}", nums(c)),
        Value::String(s) => format!("s:{}", esc(s)),
        Value::Id(i) => format!("id:{}", idx(*i)),
    }
}

fn dec_value(s: &str) -> Value {
    let (tag, rest) = s.split_once(':').unwrap_or((s, ""));
    match tag {
        "f" => rest.parse().map(Value::Float).unwrap_or_default(),
        "i" => rest.parse().map(Value::Int).unwrap_or_default(),
        "b" => Value::Bool(rest == "1"),
        "v" => parse_nums(rest).map(Value::Vector).unwrap_or_default(),
        "c" => parse_nums(rest).map(Value::Color).unwrap_or_default(),
        "s" => Value::String(unesc(rest)),
        "id" => Value::Id(rest.parse().ok()),
        _ => Value::None,
    }
}

fn enc_prop(p: &Prop) -> String {
    match p {
        Prop::Enum(e) => format!("e:{}", esc(e)),
        Prop::Bool(b) => format!("b:{}", *b as u8),
        Prop::Int(i) => format!("i:{i}"),
        Prop::Float(f) => format!("f:{f}"),
        Prop::Vector(v) => format!("v:{}", nums(v)),
        Prop::Color(c) => format!("c:{}", nums(c)),
        Prop::String(s) => format!("s:{}", esc(s)),
        Prop::Id(i) => format!("id:{}", idx(*i)),
        Prop::Ramp(r) => {
            let mut o = format!("ramp:{};{}", r.interpolation, r.color_mode);
            for (pos, c) in &r.elements {
                o.push_str(&format!(";{pos},{}", nums(c)));
            }
            o
        }
        Prop::Curve(c) => {
            let curves: Vec<String> = c.curves.iter().map(|pts| pts.iter().map(|p| nums(p)).collect::<Vec<_>>().join(" ")).collect();
            format!("curve:{}", curves.join(";"))
        }
    }
}

fn dec_prop(s: &str) -> Option<Prop> {
    let (tag, rest) = s.split_once(':')?;
    Some(match tag {
        "e" => Prop::Enum(unesc(rest)),
        "b" => Prop::Bool(rest == "1"),
        "i" => Prop::Int(rest.parse().ok()?),
        "f" => Prop::Float(rest.parse().ok()?),
        "v" => Prop::Vector(parse_nums(rest)?),
        "c" => Prop::Color(parse_nums(rest)?),
        "s" => Prop::String(unesc(rest)),
        "id" => Prop::Id(rest.parse().ok()),
        "ramp" => {
            let mut it = rest.split(';');
            let interpolation = it.next()?.to_string();
            let color_mode = it.next()?.to_string();
            let elements = it
                .filter_map(|e| {
                    let v: [f32; 5] = parse_nums(e)?;
                    Some((v[0], [v[1], v[2], v[3], v[4]]))
                })
                .collect();
            Prop::Ramp(ColorRamp { interpolation, color_mode, elements })
        }
        "curve" => {
            let curves = rest.split(';').map(|c| c.split(' ').filter_map(parse_nums::<2>).collect()).collect();
            Prop::Curve(CurveMapping { curves })
        }
        _ => return None,
    })
}

fn write_tree(out: &mut String, owner: &str, key: &str, t: &NodeTree) {
    let b = |v: bool| v as u8;
    out.push_str(&format!("#@ntree {owner}\t{}\t{}\t{}\t{}\n", esc(key), kind_name(t.kind), esc(&t.name), idx(t.active)));
    for (side, list) in [("in", &t.inputs), ("out", &t.outputs)] {
        for s in list {
            out.push_str(&format!("#@niface {side}\t{}\t{}\t{}\t{}\t{}\n", esc(&s.name), ty_name(s.ty), enc_value(&s.default), s.min, s.max));
        }
    }
    for n in &t.nodes {
        let color = n.color.map(|c| nums(&c)).unwrap_or_else(|| "-".into());
        out.push_str(&format!(
            "#@node {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{color}\n",
            esc(&n.name),
            esc(&n.idname),
            esc(&n.label),
            n.location[0],
            n.location[1],
            n.width,
            b(n.mute),
            b(n.hide),
            b(n.select),
            idx(n.parent),
        ));
        for (side, list) in [("in", &n.inputs), ("out", &n.outputs)] {
            for s in list {
                out.push_str(&format!(
                    "#@nsock {side}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    esc(&s.identifier),
                    esc(&s.name),
                    ty_name(s.ty),
                    enc_value(&s.value),
                    b(s.hide),
                    b(s.enabled),
                    b(s.hide_value),
                    s.min,
                    s.max
                ));
            }
        }
        for (name, p) in &n.props {
            out.push_str(&format!("#@nprop {}\t{}\n", esc(name), enc_prop(p)));
        }
    }
    for l in &t.links {
        out.push_str(&format!("#@link {}\t{}\t{}\t{}\t{}\n", l.from_node, l.from_socket, l.to_node, l.to_socket, b(l.muted)));
    }
}

/// Append every node tree of `scene` to `out`.
pub fn save(scene: &Scene, out: &mut String) {
    for (i, g) in scene.node_groups.iter().enumerate() {
        write_tree(out, "group", &i.to_string(), g);
    }
    for m in &scene.materials {
        if let Some(t) = &m.node_tree {
            write_tree(out, "material", &m.name, t);
        }
    }
    if let Some(t) = &scene.world.node_tree {
        write_tree(out, "world", "", t);
    }
    if let Some(t) = &scene.compositor {
        write_tree(out, "compositor", "", t);
    }
}

/// A tree being read: its owner, the tree, and the sockets of its last node
/// (taken from the node's type, then overridden by the saved ones).
struct Reading {
    owner: String,
    key: String,
    tree: NodeTree,
    template: Option<Node>,
    ins: Vec<Socket>,
    outs: Vec<Socket>,
}

impl Reading {
    fn finish_node(&mut self) {
        self.template = None;
        if let Some(n) = self.tree.nodes.last_mut() {
            n.inputs = core::mem::take(&mut self.ins);
            n.outputs = core::mem::take(&mut self.outs);
        }
    }
}

fn place(scene: &mut Scene, mut r: Reading, groups: &mut Vec<NodeTree>) {
    r.finish_node();
    let t = r.tree;
    match r.owner.as_str() {
        "group" => groups.push(t),
        "material" => {
            if let Some(m) = scene.materials.iter_mut().find(|m| m.name == r.key) {
                if m.node_tree.is_none() {
                    m.node_tree = Some(t);
                }
            }
        }
        "world" => {
            if scene.world.node_tree.is_none() {
                scene.world.node_tree = Some(t);
            }
        }
        "compositor" => {
            if scene.compositor.is_none() {
                scene.compositor = Some(t);
            }
        }
        _ => {}
    }
}

/// Read the trees saved by [`save`] into `scene` (after polyops ran).
pub fn load(scene: &mut Scene, text: &str) {
    let had_groups = !scene.node_groups.is_empty();
    let mut groups = Vec::new();
    let mut cur: Option<Reading> = None;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("#@") else { continue };
        let (tag, body) = rest.split_once(' ').unwrap_or((rest, ""));
        let f: Vec<&str> = body.split('\t').collect();
        let get = |i: usize| f.get(i).copied().unwrap_or("");
        match tag {
            "ntree" => {
                if let Some(r) = cur.take() {
                    place(scene, r, &mut groups);
                }
                let mut tree = NodeTree::new(&unesc(get(3)), kind_parse(get(2)));
                tree.active = get(4).parse().ok();
                cur = Some(Reading { owner: get(0).into(), key: unesc(get(1)), tree, template: None, ins: Vec::new(), outs: Vec::new() });
            }
            "niface" => {
                let Some(r) = cur.as_mut() else { continue };
                let s = InterfaceSocket { name: unesc(get(1)), ty: ty_parse(get(2)), default: dec_value(get(3)), min: get(4).parse().unwrap_or(f32::MIN), max: get(5).parse().unwrap_or(f32::MAX) };
                if get(0) == "in" { r.tree.inputs.push(s) } else { r.tree.outputs.push(s) }
            }
            "node" => {
                let Some(r) = cur.as_mut() else { continue };
                r.finish_node();
                let idname = unesc(get(1));
                let template = edit::make_node(&idname, r.tree.kind, &[], None);
                let mut n = template.clone().unwrap_or_else(|| Node::new(&idname, ""));
                n.name = unesc(get(0));
                n.label = unesc(get(2));
                n.location = [get(3).parse().unwrap_or(0.0), get(4).parse().unwrap_or(0.0)];
                n.width = get(5).parse().unwrap_or(n.width);
                n.mute = get(6) == "1";
                n.hide = get(7) == "1";
                n.select = get(8) == "1";
                n.parent = get(9).parse().ok();
                n.color = parse_nums(get(10));
                r.template = template;
                r.tree.nodes.push(n);
            }
            "nsock" => {
                let Some(r) = cur.as_mut() else { continue };
                if r.tree.nodes.is_empty() {
                    continue;
                }
                let input = get(0) == "in";
                let ident = unesc(get(1));
                let from_type = r.template.as_ref().and_then(|t| if input { &t.inputs } else { &t.outputs }.iter().find(|s| s.identifier == ident).cloned());
                let mut s = from_type.unwrap_or_else(|| {
                    let mut s = Socket::new(&unesc(get(2)), ty_parse(get(3)), Value::None);
                    s.identifier = ident.clone();
                    s.min = get(8).parse().unwrap_or(f32::MIN);
                    s.max = get(9).parse().unwrap_or(f32::MAX);
                    s
                });
                s.name = unesc(get(2));
                s.ty = ty_parse(get(3));
                s.value = dec_value(get(4));
                s.hide = get(5) == "1";
                s.enabled = get(6) != "0";
                s.hide_value = get(7) == "1";
                if input { r.ins.push(s) } else { r.outs.push(s) }
            }
            "nprop" => {
                let Some(r) = cur.as_mut() else { continue };
                let (Some(n), Some(p)) = (r.tree.nodes.last_mut(), dec_prop(get(1))) else { continue };
                n.set_prop(&unesc(get(0)), p);
            }
            "link" => {
                let Some(r) = cur.as_mut() else { continue };
                let v: Vec<usize> = f.iter().take(4).filter_map(|x| x.parse().ok()).collect();
                if v.len() == 4 {
                    r.tree.links.push(Link { from_node: v[0], from_socket: v[1], to_node: v[2], to_socket: v[3], muted: get(4) == "1" });
                }
            }
            _ => {}
        }
    }
    if let Some(r) = cur.take() {
        place(scene, r, &mut groups);
    }
    if !had_groups {
        scene.node_groups = groups;
    }
    sanitize(scene);
}

/// Drop references that no longer point anywhere (links to missing sockets,
/// images that were not saved) and refresh group nodes from their groups.
fn sanitize(scene: &mut Scene) {
    let (n_images, n_objects, n_groups) = (scene.images.len(), scene.objects.len(), scene.node_groups.len());
    let fix = |t: &mut NodeTree| {
        let len = t.nodes.len();
        for n in &mut t.nodes {
            n.parent = n.parent.filter(|&p| p < len);
            for (name, p) in &mut n.props {
                if let Prop::Id(Some(i)) = p {
                    let limit = match name.as_str() {
                        "image" => n_images,
                        "object" => n_objects,
                        "node_tree" => n_groups,
                        _ => usize::MAX,
                    };
                    if *i >= limit {
                        *p = Prop::Id(None);
                    }
                }
            }
        }
        t.active = t.active.filter(|&a| a < len);
        let nodes = &t.nodes;
        t.links.retain(|l| {
            nodes.get(l.from_node).is_some_and(|n| l.from_socket < n.outputs.len()) && nodes.get(l.to_node).is_some_and(|n| l.to_socket < n.inputs.len())
        });
    };
    for g in &mut scene.node_groups {
        fix(g);
    }
    for m in &mut scene.materials {
        if let Some(t) = &mut m.node_tree {
            fix(t);
        }
    }
    if let Some(t) = &mut scene.world.node_tree {
        fix(t);
    }
    if let Some(t) = &mut scene.compositor {
        fix(t);
    }
    let groups = scene.node_groups.clone();
    for (i, g) in scene.node_groups.iter_mut().enumerate() {
        edit::sync_group_nodes(g, &groups, Some(i));
    }
    for m in &mut scene.materials {
        if let Some(t) = &mut m.node_tree {
            edit::sync_group_nodes(t, &groups, None);
        }
    }
    if let Some(t) = &mut scene.world.node_tree {
        edit::sync_group_nodes(t, &groups, None);
    }
    if let Some(t) = &mut scene.compositor {
        edit::sync_group_nodes(t, &groups, None);
    }
}

#[cfg(test)]
mod tests {
    use crate::doc::Doc;
    use crate::persist::{load, save};
    use sex_3d::nodes::{edit, ColorRamp, Prop, Value};

    #[test]
    fn node_trees_round_trip_through_poly() {
        let mut doc = Doc::default();
        let groups = doc.scene.node_groups.clone();
        let t = doc.scene.materials[0].node_tree.as_mut().expect("default material has a tree");
        let math = edit::add_node(t, "ShaderNodeMath", [-400.0, 100.0], &groups, None).unwrap();
        t.nodes[math].set_prop("operation", Prop::Enum("MULTIPLY".into()));
        t.nodes[math].set_input("Value", Value::Float(0.25));
        t.nodes[math].label = "Half\tlabel".into();
        t.nodes[math].color = Some([0.1, 0.2, 0.3]);
        // Any node may carry a ramp or curve; the registry's own ramp nodes
        // are not guaranteed to exist yet.
        let ramp = edit::add_node(t, "ShaderNodeRGB", [-200.0, 100.0], &groups, None).unwrap();
        let r = ColorRamp { interpolation: "EASE".into(), color_mode: "RGB".into(), elements: vec![(0.0, [1.0, 0.0, 0.0, 1.0]), (0.4, [0.0, 1.0, 0.5, 1.0]), (1.0, [0.0, 0.0, 1.0, 1.0])] };
        t.nodes[ramp].set_prop("color_ramp", Prop::Ramp(r));
        t.nodes[ramp].set_prop("mapping", Prop::Curve(sex_3d::nodes::CurveMapping { curves: vec![vec![[0.0, 0.1], [0.5, 0.7], [1.0, 1.0]]] }));
        let bsdf = t.find_type("ShaderNodeBsdfPrincipled").unwrap();
        t.link(math, "Value", bsdf, "Roughness").unwrap();
        let frame = edit::join_frame(t, &[math, ramp]);
        assert!(t.nodes[math].parent == Some(frame));
        // Group the math node.
        let mut groups = doc.scene.node_groups.clone();
        let tree = doc.scene.materials[0].node_tree.as_mut().unwrap();
        let gn = edit::make_group(tree, &[math], &mut groups).expect("group made");
        assert_eq!(edit::group_of(&tree.nodes[gn]), Some(groups.len() - 1));
        doc.scene.node_groups = groups;
        doc.scene.compositor = Some(sex_3d::nodes::NodeTree::new("Compositing", sex_3d::nodes::TreeKind::Compositor));

        let text = save(&doc);
        let back = load(&text).unwrap();
        assert_eq!(back.scene.node_groups, doc.scene.node_groups, "groups survive");
        assert_eq!(back.scene.materials[0].node_tree, doc.scene.materials[0].node_tree, "material tree survives");
        assert_eq!(back.scene.world.node_tree, doc.scene.world.node_tree, "world tree survives");
        assert_eq!(back.scene.compositor, doc.scene.compositor);
        let t = back.scene.materials[0].node_tree.as_ref().unwrap();
        assert!(t.nodes.iter().any(|n| edit::group_of(n) == Some(0)));
        assert!(back.scene.node_groups[0].nodes.iter().any(|n| n.enum_prop("operation") == "MULTIPLY"));
        // Saving again gives the same text.
        assert_eq!(save(&back), text);
    }
}
