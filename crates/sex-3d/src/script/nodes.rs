// SPDX-License-Identifier: GPL-3.0-only
//! polyops commands for shader node trees, and their writer. The syntax is in
//! the [`super`] module docs.
use super::{boolean, floats, fmt_f, fmt_list, num, quote, Cmd, Res, Session, Token};
use crate::nodes::{new_node, sync_group_sockets, update_sockets, ColorRamp, CurveMapping, InterfaceSocket, Node, NodeTree, Prop, Socket, SocketType, TreeKind, Value};
use crate::scene::Scene;
use std::fmt::Write as _;

/// The tree a node command works on.
#[derive(Clone, Copy)]
enum Owner {
    Material(usize),
    World,
    Group(usize),
}

/// What an ID-valued property or socket points at.
#[derive(Clone, Copy, PartialEq)]
enum IdKind {
    Image,
    Object,
    Group,
    Collection,
    Material,
}

const SOCKET_TYPES: [(&str, SocketType); 17] = [
    ("float", SocketType::Float),
    ("int", SocketType::Int),
    ("bool", SocketType::Bool),
    ("vector", SocketType::Vector),
    ("color", SocketType::Color),
    ("string", SocketType::String),
    ("shader", SocketType::Shader),
    ("geometry", SocketType::Geometry),
    ("object", SocketType::Object),
    ("collection", SocketType::Collection),
    ("image", SocketType::Image),
    ("material", SocketType::Material),
    ("texture", SocketType::Texture),
    ("rotation", SocketType::Rotation),
    ("matrix", SocketType::Matrix),
    ("menu", SocketType::Menu),
    ("virtual", SocketType::Virtual),
];

fn socket_type(t: &str) -> Res<SocketType> {
    SOCKET_TYPES.iter().find(|x| x.0 == t).map(|x| x.1).ok_or_else(|| format!("unknown socket type '{t}' ({})", SOCKET_TYPES.map(|x| x.0).join(", ")))
}

pub(super) fn socket_type_name(t: SocketType) -> &'static str {
    SOCKET_TYPES.iter().find(|x| x.1 == t).map_or("float", |x| x.0)
}

fn id_kind_of_socket(t: SocketType) -> Option<IdKind> {
    match t {
        SocketType::Object => Some(IdKind::Object),
        SocketType::Image => Some(IdKind::Image),
        SocketType::Collection => Some(IdKind::Collection),
        SocketType::Material => Some(IdKind::Material),
        _ => None,
    }
}

fn id_kind_of_prop(name: &str) -> Option<IdKind> {
    match name {
        "image" | "ies" => Some(IdKind::Image),
        "object" => Some(IdKind::Object),
        "node_tree" => Some(IdKind::Group),
        "collection" => Some(IdKind::Collection),
        "material" => Some(IdKind::Material),
        _ => None,
    }
}

fn id_kind_name(k: IdKind) -> &'static str {
    match k {
        IdKind::Image => "image",
        IdKind::Object => "object",
        IdKind::Group => "group",
        IdKind::Collection => "collection",
        IdKind::Material => "material",
    }
}

fn id_name(scene: &Scene, k: IdKind, i: usize) -> Option<&str> {
    match k {
        IdKind::Image => scene.images.get(i).map(|x| x.name.as_str()),
        IdKind::Object => scene.objects.get(i).map(|x| x.name.as_str()),
        IdKind::Group => scene.node_groups.get(i).map(|x| x.name.as_str()),
        IdKind::Collection => scene.collections.get(i).map(|x| x.name.as_str()),
        IdKind::Material => scene.materials.get(i).map(|x| x.name.as_str()),
    }
}

/// `none`, or a name of the kind. Images are not written by
/// [`super::scene_to_polyops`], so an image name that does not resolve leaves
/// the slot empty instead of failing the script.
fn parse_id(scene: &Scene, k: IdKind, t: &Token) -> Res<Option<usize>> {
    if t.is_word("none") {
        return Ok(None);
    }
    let n = &t.text;
    let found = match k {
        IdKind::Image => return Ok(scene.images.iter().position(|x| x.name == *n)),
        IdKind::Object => scene.find_object(n),
        IdKind::Group => scene.node_groups.iter().position(|x| x.name == *n),
        IdKind::Collection => scene.collections.iter().position(|x| x.name == *n),
        IdKind::Material => scene.materials.iter().position(|x| x.name == *n),
    };
    found.map(Some).ok_or_else(|| format!("no {} named '{n}'", id_kind_name(k)))
}

fn write_id(scene: &Scene, k: IdKind, i: Option<usize>) -> String {
    i.and_then(|i| id_name(scene, k, i)).map_or_else(|| "none".into(), quote)
}

/// A socket value, parsed as `like` (or as the socket type when `like` is empty).
fn parse_value(scene: &Scene, like: &Value, ty: SocketType, t: &Token) -> Res<Value> {
    let like = match like {
        Value::None => match ty {
            SocketType::Float => Value::Float(0.0),
            SocketType::Int => Value::Int(0),
            SocketType::Bool => Value::Bool(false),
            SocketType::Vector | SocketType::Rotation => Value::Vector([0.0; 3]),
            SocketType::Color => Value::Color([0.0; 4]),
            SocketType::String | SocketType::Menu => Value::String(String::new()),
            SocketType::Object | SocketType::Collection | SocketType::Image | SocketType::Material => Value::Id(None),
            _ if t.is_word("none") => return Ok(Value::None),
            _ => return Err(format!("a {} socket holds no value", socket_type_name(ty))),
        },
        v => v.clone(),
    };
    Ok(match like {
        Value::None => Value::None,
        Value::Float(_) => Value::Float(num(&t.text)?),
        Value::Int(_) => Value::Int(t.text.trim().parse().map_err(|_| format!("'{}' is not an integer", t.text))?),
        Value::Bool(_) => Value::Bool(boolean(&t.text)?),
        Value::Vector(_) => match floats(&t.text)?.as_slice() {
            &[x, y, z] => Value::Vector([x, y, z]),
            v => return Err(format!("a vector needs 3 numbers, got {}", v.len())),
        },
        Value::Color(_) => match floats(&t.text)?.as_slice() {
            &[r, g, b] => Value::Color([r, g, b, 1.0]),
            &[r, g, b, a] => Value::Color([r, g, b, a]),
            v => return Err(format!("a colour needs 3 or 4 numbers, got {}", v.len())),
        },
        Value::String(_) => Value::String(t.text.clone()),
        Value::Id(_) => Value::Id(parse_id(scene, id_kind_of_socket(ty).ok_or("this socket holds no ID")?, t)?),
    })
}

pub(super) fn value_text(scene: &Scene, v: &Value, ty: SocketType) -> Option<String> {
    Some(match v {
        Value::None => return None,
        Value::Float(x) => fmt_f(*x),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => (*b as u8).to_string(),
        Value::Vector(x) => fmt_list(x),
        Value::Color(x) => fmt_list(x),
        Value::String(s) => quote(s),
        Value::Id(i) => write_id(scene, id_kind_of_socket(ty).unwrap_or(IdKind::Object), *i),
    })
}

/// Colour ramp: `INTERPOLATION;COLOR_MODE;pos,r,g,b,a;...`.
fn parse_ramp(t: &str) -> Res<ColorRamp> {
    let mut parts = t.split(';');
    let interpolation = parts.next().unwrap_or("").to_string();
    let color_mode = parts.next().ok_or("a colour ramp is INTERPOLATION;COLOR_MODE;pos,r,g,b,a;...")?.to_string();
    let mut elements = Vec::new();
    for e in parts {
        match floats(e)?.as_slice() {
            &[p, r, g, b, a] => elements.push((p, [r, g, b, a])),
            _ => return Err(format!("a colour ramp element is pos,r,g,b,a, got '{e}'")),
        }
    }
    Ok(ColorRamp { interpolation, color_mode, elements })
}

fn ramp_text(r: &ColorRamp) -> String {
    let mut s = format!("{};{}", r.interpolation, r.color_mode);
    for (p, c) in &r.elements {
        let _ = write!(s, ";{},{}", fmt_f(*p), fmt_list(c));
    }
    s
}

/// Curves: `x,y,x,y,...;x,y,...` one curve per `;` part.
fn parse_curves(t: &str) -> Res<CurveMapping> {
    let mut curves = Vec::new();
    for c in t.split(';') {
        let v = floats(c)?;
        if v.len() % 2 != 0 {
            return Err(format!("a curve is x,y pairs, got '{c}'"));
        }
        curves.push(v.chunks_exact(2).map(|p| [p[0], p[1]]).collect());
    }
    Ok(CurveMapping { curves })
}

fn curves_text(c: &CurveMapping) -> String {
    c.curves.iter().map(|c| fmt_list(&c.iter().flatten().copied().collect::<Vec<_>>())).collect::<Vec<_>>().join(";")
}

const PROP_TYPES: [&str; 14] = ["enum", "bool", "int", "float", "vector", "color", "string", "ramp", "curve", "image", "object", "group", "collection", "material"];

fn prop_type(p: &Prop, name: &str) -> &'static str {
    match p {
        Prop::Enum(_) => "enum",
        Prop::Bool(_) => "bool",
        Prop::Int(_) => "int",
        Prop::Float(_) => "float",
        Prop::Vector(_) => "vector",
        Prop::Color(_) => "color",
        Prop::String(_) => "string",
        Prop::Ramp(_) => "ramp",
        Prop::Curve(_) => "curve",
        Prop::Id(_) => id_kind_name(id_kind_of_prop(name).unwrap_or(IdKind::Image)),
    }
}

fn parse_prop(scene: &Scene, ty: &str, t: &Token) -> Res<Prop> {
    let id = |k| parse_id(scene, k, t).map(Prop::Id);
    Ok(match ty {
        "enum" => Prop::Enum(t.text.clone()),
        "bool" => Prop::Bool(boolean(&t.text)?),
        "int" => Prop::Int(t.text.trim().parse().map_err(|_| format!("'{}' is not an integer", t.text))?),
        "float" => Prop::Float(num(&t.text)?),
        "vector" => match floats(&t.text)?.as_slice() {
            &[x, y, z] => Prop::Vector([x, y, z]),
            v => return Err(format!("a vector needs 3 numbers, got {}", v.len())),
        },
        "color" => match floats(&t.text)?.as_slice() {
            &[r, g, b, a] => Prop::Color([r, g, b, a]),
            &[r, g, b] => Prop::Color([r, g, b, 1.0]),
            v => return Err(format!("a colour needs 3 or 4 numbers, got {}", v.len())),
        },
        "string" => Prop::String(t.text.clone()),
        "ramp" => Prop::Ramp(parse_ramp(&t.text)?),
        "curve" => Prop::Curve(parse_curves(&t.text)?),
        "image" => id(IdKind::Image)?,
        "object" => id(IdKind::Object)?,
        "group" => id(IdKind::Group)?,
        "collection" => id(IdKind::Collection)?,
        "material" => id(IdKind::Material)?,
        other => return Err(format!("unknown property type '{other}' ({})", PROP_TYPES.join(", "))),
    })
}

pub(super) fn prop_text(scene: &Scene, p: &Prop, name: &str) -> String {
    match p {
        Prop::Enum(s) | Prop::String(s) => quote(s),
        Prop::Bool(b) => (*b as u8).to_string(),
        Prop::Int(i) => i.to_string(),
        Prop::Float(x) => fmt_f(*x),
        Prop::Vector(v) => fmt_list(v),
        Prop::Color(v) => fmt_list(v),
        Prop::Ramp(r) => ramp_text(r),
        Prop::Curve(c) => curves_text(c),
        Prop::Id(i) => write_id(scene, id_kind_of_prop(name).unwrap_or(IdKind::Image), *i),
    }
}

fn tree<'s>(scene: &'s mut Scene, o: Owner) -> Res<&'s mut NodeTree> {
    match o {
        Owner::Material(i) => {
            let m = &mut scene.materials[i];
            m.node_tree.as_mut().ok_or_else(|| format!("material '{}' has no node tree (use node_tree first)", m.name))
        }
        Owner::World => scene.world.node_tree.as_mut().ok_or_else(|| "the world has no node tree (use node_tree first)".to_string()),
        Owner::Group(i) => Ok(&mut scene.node_groups[i]),
    }
}

fn node_index(t: &NodeTree, tok: &Token) -> Res<usize> {
    t.find(&tok.text).ok_or_else(|| format!("no node named '{}' in tree '{}'", tok.text, t.name))
}

/// Socket index by identifier, then by (enabled) name.
fn socket_index(n: &Node, out: bool, id: &str) -> Res<usize> {
    let list = if out { &n.outputs } else { &n.inputs };
    list.iter().position(|s| s.identifier == id).or_else(|| if out { n.output(id) } else { n.input(id) }).ok_or_else(|| format!("node '{}' has no {} socket '{id}'", n.name, if out { "output" } else { "input" }))
}

/// The options shared by `node_add` and `node_set`: label, location, ...
fn node_field(n: &mut Node, key: &str, t: &Token) -> Res<bool> {
    match key {
        "label" => n.label = t.text.clone(),
        "location" => match floats(&t.text)?.as_slice() {
            &[x, y] => n.location = [x, y],
            v => return Err(format!("location needs 2 numbers, got {}", v.len())),
        },
        "width" => n.width = num(&t.text)?,
        "mute" => n.mute = boolean(&t.text)?,
        "hide" => n.hide = boolean(&t.text)?,
        "select" => n.select = boolean(&t.text)?,
        "color" if t.is_word("none") => n.color = None,
        "color" => match floats(&t.text)?.as_slice() {
            &[r, g, b] => n.color = Some([r, g, b]),
            v => return Err(format!("color needs 3 numbers, got {}", v.len())),
        },
        _ => return Ok(false),
    }
    Ok(true)
}

const NODE_FIELDS: [&str; 7] = ["label", "location", "width", "mute", "hide", "select", "color"];

/// Re-read a group's interface into its Group Input/Output nodes and every
/// Group node that uses it.
fn resync_group(scene: &mut Scene, g: usize) {
    let group = scene.node_groups[g].clone();
    let fix = |t: &mut NodeTree, own: bool| {
        for n in &mut t.nodes {
            let uses = n.idname == "ShaderNodeGroup" && n.prop("node_tree") == Some(&Prop::Id(Some(g)));
            if uses || (own && (n.idname == "NodeGroupInput" || n.idname == "NodeGroupOutput")) {
                sync_group_sockets(n, &group);
            }
        }
        // Links to sockets that are gone.
        let nodes = &t.nodes;
        t.links.retain(|l| l.from_socket < nodes[l.from_node].outputs.len() && l.to_socket < nodes[l.to_node].inputs.len());
    };
    for m in &mut scene.materials {
        if let Some(t) = &mut m.node_tree {
            fix(t, false);
        }
    }
    if let Some(t) = &mut scene.world.node_tree {
        fix(t, false);
    }
    for (i, t) in scene.node_groups.iter_mut().enumerate() {
        fix(t, i == g);
    }
}

impl Session {
    /// `material NAME`, `world` or `group NAME` at the start of the arguments;
    /// returns the owner and how many arguments it took.
    fn owner(&self, scene: &mut Scene, cmd: &Cmd, create_group: bool) -> Res<(Owner, usize)> {
        let kind = cmd.arg(0, "material NAME, world or group NAME")?;
        match kind.text.as_str() {
            "world" if !kind.quoted => Ok((Owner::World, 1)),
            "material" if !kind.quoted => Ok((Owner::Material(self.material(scene, cmd.arg(1, "a material")?)?), 2)),
            "group" if !kind.quoted => {
                let name = cmd.arg(1, "a node group name")?.text.clone();
                if let Some(i) = scene.node_groups.iter().position(|g| g.name == name) {
                    return Ok((Owner::Group(i), 2));
                }
                if !create_group {
                    return Err(format!("no node group named '{name}'"));
                }
                scene.node_groups.push(NodeTree::new(&name, TreeKind::Shader));
                Ok((Owner::Group(scene.node_groups.len() - 1), 2))
            }
            other => Err(format!("a node tree belongs to 'material NAME', 'world' or 'group NAME', not '{other}'")),
        }
    }

    pub(super) fn node_command(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<Option<String>> {
        let name = cmd.name.clone();
        let (owner, k) = self.owner(scene, cmd, name == "node_tree")?;
        match name.as_str() {
            "node_tree" => {
                cmd.max_args(k)?;
                if cmd.b("remove")?.unwrap_or(false) {
                    match owner {
                        Owner::Material(i) => scene.materials[i].node_tree = None,
                        Owner::World => scene.world.node_tree = None,
                        Owner::Group(_) => return Err("node groups cannot be removed here".into()),
                    }
                    return Ok(None);
                }
                let slot = match owner {
                    Owner::Material(i) => Some(&mut scene.materials[i].node_tree),
                    Owner::World => Some(&mut scene.world.node_tree),
                    Owner::Group(_) => None,
                };
                if let Some(slot) = slot {
                    slot.get_or_insert_with(|| NodeTree::new("Shader Nodetree", TreeKind::Shader));
                }
                if let (Some(t), Owner::Group(g)) = (cmd.opt("name"), owner) {
                    if scene.node_groups.iter().enumerate().any(|(j, x)| j != g && x.name == t.text) {
                        return Err(format!("a node group named '{}' exists", t.text));
                    }
                    scene.node_groups[g].name = t.text;
                } else if let Some(t) = cmd.opt("name") {
                    tree(scene, owner)?.name = t.text;
                }
                if cmd.b("clear")?.unwrap_or(false) {
                    let t = tree(scene, owner)?;
                    t.nodes.clear();
                    t.links.clear();
                    t.active = None;
                    if let Owner::Group(g) = owner {
                        t.inputs.clear();
                        t.outputs.clear();
                        resync_group(scene, g);
                    }
                }
                if let Some(a) = cmd.opt("active") {
                    let t = tree(scene, owner)?;
                    t.active = if a.is_word("none") { None } else { Some(node_index(t, &a)?) };
                }
                Ok(Some(quote(&tree(scene, owner)?.name)))
            }
            "node_add" => {
                cmd.max_args(k + 1)?;
                let idname = cmd.arg(k, "a node type")?.text.clone();
                let unknown = cmd.b("unknown")?.unwrap_or(false);
                let mut n = match new_node(&idname) {
                    Some(n) => n,
                    None if unknown => Node::new(&idname, &idname),
                    None => return Err(format!("unknown node type '{idname}' (unknown=1 adds it without sockets)")),
                };
                for key in NODE_FIELDS {
                    if let Some(t) = cmd.opt(key) {
                        node_field(&mut n, key, &t)?;
                    }
                }
                let given = cmd.opt("name").map(|t| t.text);
                let t = tree(scene, owner)?;
                n.name = t.unique_name(given.as_deref().unwrap_or(&n.name.clone()));
                if n.idname == "NodeGroupInput" || n.idname == "NodeGroupOutput" {
                    sync_group_sockets(&mut n, t);
                }
                update_sockets(&mut n);
                let name = n.name.clone();
                t.nodes.push(n);
                Ok(Some(quote(&name)))
            }
            "node_remove" => {
                cmd.max_args(k + 1)?;
                let t = tree(scene, owner)?;
                let i = node_index(t, cmd.arg(k, "a node")?)?;
                t.remove(i);
                Ok(None)
            }
            "node_socket" => {
                cmd.max_args(k + 4)?;
                let (node, side, id, ty) = (cmd.arg(k, "a node")?.clone(), cmd.arg(k + 1, "in or out")?.text.clone(), cmd.arg(k + 2, "an identifier")?.text.clone(), socket_type(&cmd.arg(k + 3, "a socket type")?.text)?);
                let label = cmd.opt("name").map_or_else(|| id.clone(), |t| t.text);
                let t = tree(scene, owner)?;
                let i = node_index(t, &node)?;
                let s = Socket::new(&label, ty, Value::None).ident(&id);
                match side.as_str() {
                    "in" => t.nodes[i].inputs.push(s),
                    "out" => t.nodes[i].outputs.push(s),
                    o => return Err(format!("node_socket takes in or out, not '{o}'")),
                }
                Ok(None)
            }
            "node_set" => {
                cmd.max_args(k + 3)?;
                let node = cmd.arg(k, "a node")?.clone();
                let target = cmd.arg(k + 1, "what to set")?.text.clone();
                let val = cmd.arg(k + 2, "a value")?.clone();
                let ty = cmd.opt("type").map(|t| t.text);
                let i = node_index(tree(scene, owner)?, &node)?;
                if let Some(p) = target.strip_prefix("prop:") {
                    let current = tree(scene, owner)?.nodes[i].prop(p).cloned();
                    let ty = match (&ty, &current) {
                        (Some(t), _) => t.as_str(),
                        (None, Some(c)) => prop_type(c, p),
                        (None, None) => match id_kind_of_prop(p) {
                            Some(k) => id_kind_name(k),
                            None => return Err(format!("node '{}' has no property '{p}' (type= adds one)", node.text)),
                        },
                    };
                    let v = parse_prop(scene, ty, &val)?;
                    let group = match (&v, p) {
                        (Prop::Id(Some(g)), "node_tree") => scene.node_groups.get(*g).cloned(),
                        _ => None,
                    };
                    let n = &mut tree(scene, owner)?.nodes[i];
                    n.set_prop(p, v);
                    if let (Some(g), "ShaderNodeGroup") = (&group, n.idname.as_str()) {
                        sync_group_sockets(n, g);
                    }
                    update_sockets(n);
                    let t = tree(scene, owner)?;
                    let nodes = &t.nodes;
                    t.links.retain(|l| l.from_socket < nodes[l.from_node].outputs.len() && l.to_socket < nodes[l.to_node].inputs.len());
                    return Ok(None);
                }
                if ty.is_some() {
                    return Err("type= is only for prop: targets".into());
                }
                let (hide, rest) = match target.strip_prefix("hide:") {
                    Some(r) => (true, r),
                    None => (false, target.as_str()),
                };
                let side = rest.strip_prefix("in:").map(|s| (false, s)).or_else(|| rest.strip_prefix("out:").map(|s| (true, s)));
                if let Some((out, id)) = side {
                    let n = &tree(scene, owner)?.nodes[i];
                    let si = socket_index(n, out, id)?;
                    let s = if out { &n.outputs[si] } else { &n.inputs[si] };
                    let (like, sty) = (s.value.clone(), s.ty);
                    let v = if hide { None } else { Some(parse_value(scene, &like, sty, &val)?) };
                    let h = if hide { Some(boolean(&val.text)?) } else { None };
                    let n = &mut tree(scene, owner)?.nodes[i];
                    let s = if out { &mut n.outputs[si] } else { &mut n.inputs[si] };
                    if let Some(v) = v {
                        s.value = v;
                    }
                    if let Some(h) = h {
                        s.hide = h;
                    }
                    return Ok(None);
                }
                if hide {
                    return Err(format!("'{target}': hide: is followed by in:SOCKET or out:SOCKET"));
                }
                let t = tree(scene, owner)?;
                match target.as_str() {
                    "parent" => {
                        let p = if val.is_word("none") { None } else { Some(node_index(t, &val)?) };
                        let mut c = p;
                        while let Some(x) = c {
                            if x == i {
                                return Err("node parenting would make a cycle".into());
                            }
                            c = t.nodes[x].parent;
                        }
                        t.nodes[i].parent = p;
                    }
                    "name" => {
                        if t.nodes.iter().enumerate().any(|(j, n)| j != i && n.name == val.text) {
                            return Err(format!("a node named '{}' exists", val.text));
                        }
                        t.nodes[i].name = val.text.clone();
                    }
                    key => {
                        if !node_field(&mut t.nodes[i], key, &val)? {
                            return Err(format!("cannot set '{key}' on a node (prop:NAME, in:SOCKET, out:SOCKET, hide:in:SOCKET, parent, name, {})", NODE_FIELDS.join(", ")));
                        }
                    }
                }
                Ok(None)
            }
            "node_link" | "node_unlink" => {
                let link = name == "node_link";
                cmd.max_args(k + if link { 4 } else { 2 })?;
                let muted = if link { cmd.b("muted")?.unwrap_or(false) } else { false };
                let args: Vec<Token> = cmd.pos[k..].to_vec();
                let t = tree(scene, owner)?;
                if !link {
                    let (to, ts) = (node_index(t, cmd.arg(k, "a node")?)?, cmd.arg(k + 1, "an input")?.text.clone());
                    let ts = socket_index(&t.nodes[to], false, &ts)?;
                    t.unlink_input(to, ts);
                    return Ok(None);
                }
                if args.len() < 4 {
                    return Err("node_link needs FROM_NODE FROM_SOCKET TO_NODE TO_SOCKET".into());
                }
                let (from, to) = (node_index(t, &args[0])?, node_index(t, &args[2])?);
                let fs = socket_index(&t.nodes[from], true, &args[1].text)?;
                let ts = socket_index(&t.nodes[to], false, &args[3].text)?;
                let li = t.link_sockets(from, fs, to, ts).map_err(|e| e.to_string())?;
                t.links[li].muted = muted;
                Ok(None)
            }
            "node_interface" => {
                let Owner::Group(g) = owner else { return Err("node_interface takes group NAME".into()) };
                cmd.max_args(k + 3)?;
                let side = cmd.arg(k, "input or output")?.text.clone();
                let sname = cmd.arg(k + 1, "a socket name")?.text.clone();
                let ty = socket_type(&cmd.arg(k + 2, "a socket type")?.text)?;
                let default = match cmd.opt("default") {
                    Some(t) => parse_value(scene, &Value::None, ty, &t)?,
                    None => Value::None,
                };
                let min = cmd.f("min")?.unwrap_or(f32::MIN);
                let max = cmd.f("max")?.unwrap_or(f32::MAX);
                let s = InterfaceSocket { name: sname, ty, default, min, max };
                let t = &mut scene.node_groups[g];
                match side.as_str() {
                    "input" => t.inputs.push(s),
                    "output" => t.outputs.push(s),
                    o => return Err(format!("node_interface takes input or output, not '{o}'")),
                }
                resync_group(scene, g);
                Ok(None)
            }
            _ => unreachable!("node command {name}"),
        }
    }
}

/// Write every node group, material tree and the world tree. Run after the
/// objects and materials exist (Texture Coordinate nodes name objects).
pub(super) fn write_node_trees(o: &mut String, scene: &Scene) {
    let groups: Vec<String> = scene.node_groups.iter().map(|g| format!("group {}", quote(&g.name))).collect();
    // Every group and its interface first: trees may use any group.
    for (g, owner) in scene.node_groups.iter().zip(&groups) {
        let _ = writeln!(o, "node_tree {owner}");
        for (side, list) in [("input", &g.inputs), ("output", &g.outputs)] {
            for s in list.iter() {
                let mut extra = String::new();
                if let Some(v) = value_text(scene, &s.default, s.ty) {
                    let _ = write!(extra, " default={v}");
                }
                if s.min != f32::MIN {
                    let _ = write!(extra, " min={}", fmt_f(s.min));
                }
                if s.max != f32::MAX {
                    let _ = write!(extra, " max={}", fmt_f(s.max));
                }
                let _ = writeln!(o, "node_interface {owner} {side} {} {}{extra}", quote(&s.name), socket_type_name(s.ty));
            }
        }
    }
    for (g, owner) in scene.node_groups.iter().zip(&groups) {
        write_tree(o, scene, owner, g);
    }
    for m in &scene.materials {
        if let Some(t) = &m.node_tree {
            let owner = format!("material {}", quote(&m.name));
            let _ = writeln!(o, "node_tree {owner} name={}", quote(&t.name));
            write_tree(o, scene, &owner, t);
        }
    }
    if let Some(t) = &scene.world.node_tree {
        let _ = writeln!(o, "node_tree world name={}", quote(&t.name));
        write_tree(o, scene, "world", t);
    }
}

fn write_tree(o: &mut String, scene: &Scene, owner: &str, t: &NodeTree) {
    for n in &t.nodes {
        let fresh = new_node(&n.idname);
        let name = quote(&n.name);
        let mut line = format!("node_add {owner} {} name={name} location={} width={}", quote(&n.idname), fmt_list(&n.location), fmt_f(n.width));
        if !n.label.is_empty() {
            let _ = write!(line, " label={}", quote(&n.label));
        }
        for (key, on) in [("mute", n.mute), ("hide", n.hide), ("select", n.select)] {
            if on {
                let _ = write!(line, " {key}=1");
            }
        }
        if let Some(c) = n.color {
            let _ = write!(line, " color={}", fmt_list(&c));
        }
        if fresh.is_none() {
            line += " unknown=1";
        }
        let _ = writeln!(o, "{line}");
        if fresh.is_none() {
            for (side, list) in [("in", &n.inputs), ("out", &n.outputs)] {
                for s in list.iter() {
                    let label = if s.name != s.identifier { format!(" name={}", quote(&s.name)) } else { String::new() };
                    let _ = writeln!(o, "node_socket {owner} {name} {side} {} {}{label}", quote(&s.identifier), socket_type_name(s.ty));
                }
            }
        }
        for (key, p) in &n.props {
            let old = fresh.as_ref().and_then(|f| f.prop(key));
            if old == Some(p) {
                continue;
            }
            let ty = if old.is_none() && id_kind_of_prop(key).is_none() { format!(" type={}", prop_type(p, key)) } else { String::new() };
            let _ = writeln!(o, "node_set {owner} {name} {} {}{ty}", quote(&format!("prop:{key}")), prop_text(scene, p, key));
        }
        for (side, list, flist) in [("in", &n.inputs, fresh.as_ref().map(|f| &f.inputs)), ("out", &n.outputs, fresh.as_ref().map(|f| &f.outputs))] {
            for s in list.iter() {
                let old = flist.and_then(|l| l.iter().find(|x| x.identifier == s.identifier)).map(|x| &x.value);
                if old != Some(&s.value) {
                    if let Some(v) = value_text(scene, &s.value, s.ty) {
                        let _ = writeln!(o, "node_set {owner} {name} {} {v}", quote(&format!("{side}:{}", s.identifier)));
                    }
                }
                if s.hide {
                    let _ = writeln!(o, "node_set {owner} {name} {} 1", quote(&format!("hide:{side}:{}", s.identifier)));
                }
            }
        }
    }
    for n in &t.nodes {
        if let Some(p) = n.parent.and_then(|p| t.nodes.get(p)) {
            let _ = writeln!(o, "node_set {owner} {} parent {}", quote(&n.name), quote(&p.name));
        }
    }
    for l in &t.links {
        let (Some(f), Some(to)) = (t.nodes.get(l.from_node), t.nodes.get(l.to_node)) else { continue };
        let (Some(fs), Some(ts)) = (f.outputs.get(l.from_socket), to.inputs.get(l.to_socket)) else { continue };
        let muted = if l.muted { " muted=1" } else { "" };
        let _ = writeln!(o, "node_link {owner} {} {} {} {}{muted}", quote(&f.name), quote(&fs.identifier), quote(&to.name), quote(&ts.identifier));
    }
    if let Some(a) = t.active.and_then(|a| t.nodes.get(a)) {
        let _ = writeln!(o, "node_tree {owner} active={}", quote(&a.name));
    }
}
