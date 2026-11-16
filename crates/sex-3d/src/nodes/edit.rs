// SPDX-License-Identifier: GPL-3.0-only
//! Node-editor operations on a tree, as Blender's node operators do them:
//! delete with reconnect, duplicate, copy/paste, make links (F), frames,
//! select linked, and node groups (make, ungroup, interface changes).
//!
//! Groups follow Blender: a group is a [`NodeTree`] in `Scene::node_groups`
//! with interface sockets (`inputs`/`outputs`); a group node
//! (`ShaderNodeGroup`, `CompositorNodeGroup`, `GeometryNodeGroup`) refers to
//! it by its `node_tree` property (`Prop::Id(Some(index))`) and carries one
//! socket per interface socket, identified `Socket_<n>`. Inside the group a
//! Group Input node (`NodeGroupInput`) has the interface inputs as outputs and
//! a Group Output node (`NodeGroupOutput`) the interface outputs as inputs,
//! each followed by an empty "extend" socket.
use super::{new_node, InterfaceSocket, Link, Node, NodeTree, Prop, Socket, SocketType, TreeKind, Value};

pub const GROUP_INPUT: &str = "NodeGroupInput";
pub const GROUP_OUTPUT: &str = "NodeGroupOutput";
pub const FRAME: &str = "NodeFrame";
pub const REROUTE: &str = "NodeReroute";

/// The group node type of a tree kind.
pub fn group_idname(kind: TreeKind) -> &'static str {
    match kind {
        TreeKind::Shader => "ShaderNodeGroup",
        TreeKind::Compositor => "CompositorNodeGroup",
        TreeKind::Geometry => "GeometryNodeGroup",
    }
}

pub fn is_group_node(n: &Node) -> bool {
    matches!(n.idname.as_str(), "ShaderNodeGroup" | "CompositorNodeGroup" | "GeometryNodeGroup")
}

/// The node group a group node uses.
pub fn group_of(n: &Node) -> Option<usize> {
    if !is_group_node(n) {
        return None;
    }
    match n.prop("node_tree") {
        Some(Prop::Id(i)) => *i,
        _ => None,
    }
}

/// May an output of type `from` feed an input of type `to`? Data sockets
/// convert into each other; closures only feed closures; geometry and
/// data-block sockets only link to their own kind.
pub fn can_link(from: SocketType, to: SocketType) -> bool {
    use SocketType as T;
    let data = |t: T| matches!(t, T::Float | T::Int | T::Bool | T::Vector | T::Color | T::Rotation | T::Matrix);
    if from == T::Virtual || to == T::Virtual || from == to {
        return true;
    }
    match (from, to) {
        (T::Shader, _) => false,
        (f, T::Shader) => data(f),
        (f, t) => data(f) && data(t),
    }
}

/// A type's zero value for a fresh socket.
pub fn type_default(ty: SocketType) -> Value {
    match ty {
        SocketType::Float => Value::Float(0.0),
        SocketType::Int => Value::Int(0),
        SocketType::Bool => Value::Bool(false),
        SocketType::Vector | SocketType::Rotation => Value::Vector([0.0; 3]),
        SocketType::Color => Value::Color([0.8, 0.8, 0.8, 1.0]),
        SocketType::String => Value::String(String::new()),
        SocketType::Object | SocketType::Collection | SocketType::Image | SocketType::Material | SocketType::Texture => Value::Id(None),
        _ => Value::None,
    }
}

fn extend_socket() -> Socket {
    Socket::new("", SocketType::Virtual, Value::None).ident("__extend__")
}

fn iface_socket(i: usize, s: &InterfaceSocket) -> Socket {
    let mut so = Socket::new(&s.name, s.ty, if s.default == Value::None { type_default(s.ty) } else { s.default.clone() });
    so.identifier = format!("Socket_{i}");
    so.min = s.min;
    so.max = s.max;
    so
}

/// A group node for group `gi`.
pub fn group_node(kind: TreeKind, gi: usize, g: &NodeTree) -> Node {
    let mut n = Node::new(group_idname(kind), "Group");
    n.props = vec![("node_tree".into(), Prop::Id(Some(gi)))];
    n.inputs = g.inputs.iter().enumerate().map(|(i, s)| iface_socket(i, s)).collect();
    n.outputs = g.outputs.iter().enumerate().map(|(i, s)| iface_socket(i, s)).collect();
    n
}

pub fn group_input_node(g: &NodeTree) -> Node {
    let mut n = Node::new(GROUP_INPUT, "Group Input");
    n.outputs = g.inputs.iter().enumerate().map(|(i, s)| iface_socket(i, s)).collect();
    n.outputs.push(extend_socket());
    n
}

pub fn group_output_node(g: &NodeTree) -> Node {
    let mut n = Node::new(GROUP_OUTPUT, "Group Output");
    n.props = vec![("is_active_output".into(), Prop::Bool(true))];
    n.inputs = g.outputs.iter().enumerate().map(|(i, s)| iface_socket(i, s)).collect();
    n.inputs.push(extend_socket());
    n
}

/// A node of any type: registered types from the registry, plus the
/// group-related and layout nodes every editor has.
pub fn make_node(idname: &str, kind: TreeKind, groups: &[NodeTree], own: Option<&NodeTree>) -> Option<Node> {
    if let Some(n) = new_node(idname) {
        return Some(n);
    }
    match idname {
        GROUP_INPUT => own.map(group_input_node).or_else(|| Some(group_input_node(&NodeTree::default()))),
        GROUP_OUTPUT => own.map(group_output_node).or_else(|| Some(group_output_node(&NodeTree::default()))),
        FRAME => {
            let mut n = Node::new(FRAME, "Frame");
            n.width = 150.0;
            n.props = vec![("shrink".into(), Prop::Bool(true)), ("label_size".into(), Prop::Int(20))];
            Some(n)
        }
        REROUTE => {
            let mut n = Node::new(REROUTE, "Reroute");
            n.width = 16.0;
            n.inputs = vec![Socket::new("Input", SocketType::Color, Value::Color([0.0, 0.0, 0.0, 1.0]))];
            n.outputs = vec![Socket::new("Output", SocketType::Color, Value::Color([0.0, 0.0, 0.0, 1.0]))];
            Some(n)
        }
        _ => {
            let gi = idname.strip_prefix("group:").and_then(|s| s.parse::<usize>().ok())?;
            let g = groups.get(gi)?;
            let mut n = group_node(kind, gi, g);
            n.name = g.name.clone();
            Some(n)
        }
    }
}

/// Add a node (see [`make_node`]) at `location`, uniquely named.
pub fn add_node(t: &mut NodeTree, idname: &str, location: [f32; 2], groups: &[NodeTree], own: Option<&NodeTree>) -> Option<usize> {
    let mut n = make_node(idname, t.kind, groups, own)?;
    n.name = t.unique_name(&n.name);
    n.location = location;
    t.nodes.push(n);
    Some(t.nodes.len() - 1)
}

pub fn selected(t: &NodeTree) -> Vec<usize> {
    (0..t.nodes.len()).filter(|&i| t.nodes[i].select).collect()
}

pub fn select_only(t: &mut NodeTree, sel: &[usize]) {
    for (i, n) in t.nodes.iter_mut().enumerate() {
        n.select = sel.contains(&i);
    }
}

/// Remove several nodes; returns the old-to-new index map (None = removed).
pub fn remove_nodes(t: &mut NodeTree, nodes: &[usize]) -> Vec<Option<usize>> {
    let mut v: Vec<usize> = nodes.iter().copied().filter(|&i| i < t.nodes.len()).collect();
    v.sort_unstable();
    v.dedup();
    let mut map = Vec::with_capacity(t.nodes.len());
    let mut removed = 0;
    for i in 0..t.nodes.len() {
        if v.binary_search(&i).is_ok() {
            removed += 1;
            map.push(None);
        } else {
            map.push(Some(i - removed));
        }
    }
    for &i in v.iter().rev() {
        t.remove(i);
    }
    map
}

/// The inputs a node passes through when muted or dissolved: for each
/// output, the input whose value flows on (same type first, linked first),
/// as Blender's internal links. (input, output) pairs.
pub fn internal_links(t: &NodeTree, n: usize) -> Vec<(usize, usize)> {
    let node = &t.nodes[n];
    let linked = |i: usize| t.links.iter().any(|l| l.to_node == n && l.to_socket == i);
    let mut out = Vec::new();
    for (o, os) in node.outputs.iter().enumerate() {
        if !os.enabled || os.ty == SocketType::Virtual {
            continue;
        }
        let cands: Vec<usize> = (0..node.inputs.len()).filter(|&i| node.inputs[i].enabled && node.inputs[i].ty != SocketType::Virtual).collect();
        let pick = cands
            .iter()
            .find(|&&i| linked(i) && node.inputs[i].ty == os.ty)
            .or_else(|| cands.iter().find(|&&i| node.inputs[i].ty == os.ty))
            .or_else(|| cands.iter().find(|&&i| linked(i) && can_link(node.inputs[i].ty, os.ty)));
        if let Some(&i) = pick {
            out.push((i, o));
        }
    }
    out
}

/// Ctrl+X: delete nodes and connect what fed them to what they fed.
pub fn delete_reconnect(t: &mut NodeTree, nodes: &[usize]) {
    let gone = |i: usize| nodes.contains(&i);
    // Where does output `o` of removed node `n` really come from?
    let source = |t: &NodeTree, mut n: usize, mut o: usize| -> Option<(usize, usize)> {
        for _ in 0..t.nodes.len() + 1 {
            let (i, _) = internal_links(t, n).into_iter().find(|p| p.1 == o)?;
            let l = t.links.iter().find(|l| l.to_node == n && l.to_socket == i && !l.muted)?;
            if !gone(l.from_node) {
                return Some((l.from_node, l.from_socket));
            }
            n = l.from_node;
            o = l.from_socket;
        }
        None
    };
    let mut relink = Vec::new();
    for l in &t.links {
        if gone(l.from_node) && !gone(l.to_node) {
            if let Some(src) = source(t, l.from_node, l.from_socket) {
                relink.push((src, l.to_node, l.to_socket));
            }
        }
    }
    let map = remove_nodes(t, nodes);
    for ((fnode, fs), tn, ts) in relink {
        if let (Some(a), Some(b)) = (map[fnode], map[tn]) {
            let ok = can_link(t.nodes[a].outputs[fs].ty, t.nodes[b].inputs[ts].ty);
            if ok {
                let _ = t.link_sockets(a, fs, b, ts);
            }
        }
    }
}

/// Shift+D: copies of `nodes` (selected, the originals deselected) with the
/// links among them, and with `keep_inputs` the links coming in from other
/// nodes too. Returns the new indices.
pub fn duplicate(t: &mut NodeTree, nodes: &[usize], keep_inputs: bool) -> Vec<usize> {
    let base = t.nodes.len();
    let mut map = vec![None; base];
    for (k, &i) in nodes.iter().enumerate() {
        map[i] = Some(base + k);
    }
    for n in &mut t.nodes {
        n.select = false;
    }
    for &i in nodes {
        let mut n = t.nodes[i].clone();
        n.name = t.unique_name(&t.nodes[i].name);
        n.select = true;
        n.parent = n.parent.map(|p| map[p].unwrap_or(p));
        t.nodes.push(n);
    }
    let links: Vec<Link> = t.links.clone();
    for l in links {
        match (map[l.from_node], map[l.to_node]) {
            (Some(a), Some(b)) => t.links.push(Link { from_node: a, to_node: b, ..l }),
            (None, Some(b)) if keep_inputs => t.links.push(Link { to_node: b, ..l }),
            _ => {}
        }
    }
    if let Some(a) = t.active {
        if let Some(Some(na)) = map.get(a) {
            t.active = Some(*na);
        }
    }
    (base..t.nodes.len()).collect()
}

/// Nodes on the clipboard (Ctrl+C / Ctrl+V).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clipboard {
    pub kind: TreeKind,
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
}

pub fn copy(t: &NodeTree, nodes: &[usize]) -> Clipboard {
    let pos = |i: usize| nodes.iter().position(|&n| n == i);
    let mut c = Clipboard { kind: t.kind, ..Default::default() };
    for &i in nodes {
        let mut n = t.nodes[i].clone();
        n.parent = n.parent.and_then(pos);
        c.nodes.push(n);
    }
    for l in &t.links {
        if let (Some(a), Some(b)) = (pos(l.from_node), pos(l.to_node)) {
            c.links.push(Link { from_node: a, to_node: b, ..*l });
        }
    }
    c
}

/// Paste the clipboard moved by `offset`; the pasted nodes are selected.
pub fn paste(t: &mut NodeTree, c: &Clipboard, offset: [f32; 2]) -> Vec<usize> {
    let base = t.nodes.len();
    for n in &mut t.nodes {
        n.select = false;
    }
    for n in &c.nodes {
        let mut m = n.clone();
        m.name = t.unique_name(&n.name);
        m.location = [n.location[0] + offset[0], n.location[1] + offset[1]];
        m.parent = n.parent.map(|p| p + base);
        m.select = true;
        t.nodes.push(m);
    }
    for l in &c.links {
        t.links.push(Link { from_node: l.from_node + base, to_node: l.to_node + base, ..*l });
    }
    t.active = (base < t.nodes.len()).then_some(base);
    (base..t.nodes.len()).collect()
}

fn visible(s: &Socket) -> bool {
    s.enabled && !s.hide && s.ty != SocketType::Virtual
}

/// F (Make Links): link each selected node to the next one to its right,
/// from the left node's first output to the right node's first free input
/// of the same type (any free input when no type matches). With `replace`,
/// linked inputs may be taken over. Returns the number of links made.
pub fn make_links(t: &mut NodeTree, nodes: &[usize], replace: bool) -> usize {
    let mut order: Vec<usize> = nodes.iter().copied().filter(|&i| !matches!(t.nodes[i].idname.as_str(), FRAME)).collect();
    order.sort_by(|a, b| t.nodes[*a].location[0].total_cmp(&t.nodes[*b].location[0]));
    let mut made = 0;
    for w in order.windows(2) {
        let (a, b) = (w[0], w[1]);
        let free = |t: &NodeTree, i: usize| replace || !t.links.iter().any(|l| l.to_node == b && l.to_socket == i);
        let outs: Vec<usize> = (0..t.nodes[a].outputs.len()).filter(|&o| visible(&t.nodes[a].outputs[o])).collect();
        let ins: Vec<usize> = (0..t.nodes[b].inputs.len()).filter(|&i| visible(&t.nodes[b].inputs[i]) && free(t, i)).collect();
        let mut pick = None;
        'find: for &o in &outs {
            for &i in &ins {
                if t.nodes[a].outputs[o].ty == t.nodes[b].inputs[i].ty {
                    pick = Some((o, i));
                    break 'find;
                }
            }
        }
        if pick.is_none() {
            pick = outs.iter().flat_map(|&o| ins.iter().map(move |&i| (o, i))).find(|&(o, i)| can_link(t.nodes[a].outputs[o].ty, t.nodes[b].inputs[i].ty));
        }
        if let Some((o, i)) = pick {
            if t.link_sockets(a, o, b, i).is_ok() {
                made += 1;
            }
        }
    }
    made
}

/// Ctrl+J: put the nodes in a new frame; returns its index.
pub fn join_frame(t: &mut NodeTree, nodes: &[usize]) -> usize {
    let parents: Vec<Option<usize>> = nodes.iter().map(|&i| t.nodes[i].parent).collect();
    let common = parents.first().copied().flatten().filter(|p| parents.iter().all(|q| *q == Some(*p)) && !nodes.contains(p));
    let f = add_node(t, FRAME, [0.0, 0.0], &[], None).expect("frame");
    let (mut x0, mut y1) = (f32::INFINITY, f32::NEG_INFINITY);
    for &i in nodes {
        x0 = x0.min(t.nodes[i].location[0]);
        y1 = y1.max(t.nodes[i].location[1]);
        // Nested frames: only the outermost selected nodes move in.
        if t.nodes[i].parent.map_or(true, |p| !nodes.contains(&p)) {
            t.nodes[i].parent = Some(f);
        }
    }
    if x0.is_finite() {
        t.nodes[f].location = [x0 - 20.0, y1 + 40.0];
    }
    t.nodes[f].parent = common;
    f
}

/// Alt+P: take nodes out of their frames.
pub fn detach(t: &mut NodeTree, nodes: &[usize]) {
    for &i in nodes {
        t.nodes[i].parent = None;
    }
}

/// Nodes feeding `nodes` (Shift+L ... L in Blender's keymap: L selects
/// linked to, Shift+L linked from).
pub fn linked_from(t: &NodeTree, nodes: &[usize]) -> Vec<usize> {
    let mut v: Vec<usize> = t.links.iter().filter(|l| nodes.contains(&l.to_node)).map(|l| l.from_node).collect();
    v.sort_unstable();
    v.dedup();
    v
}

pub fn linked_to(t: &NodeTree, nodes: &[usize]) -> Vec<usize> {
    let mut v: Vec<usize> = t.links.iter().filter(|l| nodes.contains(&l.from_node)).map(|l| l.to_node).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Ctrl+G: move `nodes` into a new node group (pushed onto `groups`) and put
/// a group node in their place, with interface sockets for the links that
/// crossed the selection. Returns the group node's index.
pub fn make_group(t: &mut NodeTree, nodes: &[usize], groups: &mut Vec<NodeTree>) -> Option<usize> {
    let sel: Vec<usize> = nodes.iter().copied().filter(|&i| i < t.nodes.len() && !matches!(t.nodes[i].idname.as_str(), GROUP_INPUT | GROUP_OUTPUT)).collect();
    if sel.is_empty() {
        return None;
    }
    let inside = |i: usize| sel.iter().position(|&s| s == i);
    let name = {
        let mut n = String::from("NodeGroup");
        let mut k = 1;
        while groups.iter().any(|g| g.name == n) {
            n = format!("NodeGroup.{k:03}");
            k += 1;
        }
        n
    };
    let mut g = NodeTree::new(&name, t.kind);
    let (mut x0, mut x1, mut ysum) = (f32::INFINITY, f32::NEG_INFINITY, 0.0);
    for &i in &sel {
        let mut n = t.nodes[i].clone();
        n.parent = n.parent.and_then(inside);
        n.select = false;
        x0 = x0.min(n.location[0]);
        x1 = x1.max(n.location[0] + n.width);
        ysum += n.location[1];
        g.nodes.push(n);
    }
    let ymid = ysum / sel.len() as f32;
    for l in &t.links {
        if let (Some(a), Some(b)) = (inside(l.from_node), inside(l.to_node)) {
            g.links.push(Link { from_node: a, to_node: b, ..*l });
        }
    }
    // Interface sockets for the crossing links.
    let mut in_keys: Vec<(usize, usize)> = Vec::new(); // external (node, output)
    let mut in_targets: Vec<(usize, usize, usize)> = Vec::new(); // (iface, inner node, input)
    let mut out_keys: Vec<(usize, usize)> = Vec::new(); // inner (node, output)
    let mut out_targets: Vec<(usize, usize, usize)> = Vec::new(); // (iface, external node, input)
    for l in &t.links {
        match (inside(l.from_node), inside(l.to_node)) {
            (None, Some(b)) => {
                let key = (l.from_node, l.from_socket);
                let k = match in_keys.iter().position(|x| *x == key) {
                    Some(k) => k,
                    None => {
                        let s = &t.nodes[l.to_node].inputs[l.to_socket];
                        g.inputs.push(InterfaceSocket { name: s.name.clone(), ty: s.ty, default: s.value.clone(), min: s.min, max: s.max });
                        in_keys.push(key);
                        in_keys.len() - 1
                    }
                };
                in_targets.push((k, b, l.to_socket));
            }
            (Some(a), None) => {
                let key = (a, l.from_socket);
                let k = match out_keys.iter().position(|x| *x == key) {
                    Some(k) => k,
                    None => {
                        let s = &t.nodes[l.from_node].outputs[l.from_socket];
                        g.outputs.push(InterfaceSocket { name: s.name.clone(), ty: s.ty, default: type_default(s.ty), min: s.min, max: s.max });
                        out_keys.push(key);
                        out_keys.len() - 1
                    }
                };
                out_targets.push((k, l.to_node, l.to_socket));
            }
            _ => {}
        }
    }
    let gin = g.nodes.len();
    let mut n = group_input_node(&g);
    n.location = [x0 - 200.0, ymid];
    g.nodes.push(n);
    let gout = g.nodes.len();
    let mut n = group_output_node(&g);
    n.location = [x1 + 60.0, ymid];
    g.nodes.push(n);
    for &(k, b, s) in &in_targets {
        g.links.push(Link { from_node: gin, from_socket: k, to_node: b, to_socket: s, muted: false });
    }
    for (k, &(a, s)) in out_keys.iter().enumerate() {
        g.links.push(Link { from_node: a, from_socket: s, to_node: gout, to_socket: k, muted: false });
    }
    let gi = groups.len();
    // The group node replaces the selection in the parent tree.
    let mut gn = group_node(t.kind, gi, &g);
    gn.name = t.unique_name("Group");
    gn.location = [(x0 + x1) / 2.0 - 70.0, ymid];
    gn.select = true;
    let parent = t.nodes[sel[0]].parent.filter(|p| !sel.contains(p));
    gn.parent = parent;
    groups.push(g);
    t.nodes.push(gn);
    let map = remove_nodes(t, &sel);
    let gnode = t.nodes.len() - 1;
    for (k, &(fnode, fs)) in in_keys.iter().enumerate() {
        if let Some(a) = map[fnode] {
            let _ = t.link_sockets(a, fs, gnode, k);
        }
    }
    for &(k, b, s) in &out_targets {
        if let Some(b) = map[b] {
            let _ = t.link_sockets(gnode, k, b, s);
        }
    }
    for n in &mut t.nodes {
        n.select = false;
    }
    t.nodes[gnode].select = true;
    t.active = Some(gnode);
    Some(gnode)
}

/// Ctrl+Alt+G: replace a group node by the group's nodes. Returns the
/// indices of the nodes brought in.
pub fn ungroup(t: &mut NodeTree, node: usize, groups: &[NodeTree]) -> Option<Vec<usize>> {
    let gi = group_of(t.nodes.get(node)?)?;
    let g = groups.get(gi)?.clone();
    let io = |i: usize| matches!(g.nodes[i].idname.as_str(), GROUP_INPUT | GROUP_OUTPUT);
    // Centre the group's content on the group node.
    let (mut cx, mut cy, mut k) = (0.0, 0.0, 0.0);
    for (i, n) in g.nodes.iter().enumerate() {
        if !io(i) {
            cx += n.location[0];
            cy += n.location[1];
            k += 1.0;
        }
    }
    let off = if k > 0.0 { [t.nodes[node].location[0] - cx / k, t.nodes[node].location[1] - cy / k] } else { [0.0, 0.0] };
    let base = t.nodes.len();
    let mut map = vec![None; g.nodes.len()];
    for n in &mut t.nodes {
        n.select = false;
    }
    let parent = t.nodes[node].parent;
    for (i, n) in g.nodes.iter().enumerate() {
        if io(i) {
            continue;
        }
        map[i] = Some(t.nodes.len());
        let mut m = n.clone();
        m.name = t.unique_name(&n.name);
        m.location = [n.location[0] + off[0], n.location[1] + off[1]];
        m.select = true;
        t.nodes.push(m);
    }
    for i in 0..g.nodes.len() {
        if let Some(ni) = map[i] {
            t.nodes[ni].parent = match g.nodes[i].parent {
                Some(p) => map[p].or(parent),
                None => parent,
            };
        }
    }
    let feeds: Vec<Link> = t.links.iter().filter(|l| l.to_node == node).copied().collect();
    let takes: Vec<Link> = t.links.iter().filter(|l| l.from_node == node).copied().collect();
    let gnode_inputs = t.nodes[node].inputs.clone();
    let mut new_links: Vec<(usize, usize, usize, usize)> = Vec::new();
    // What feeds interface input k from outside: a link or the node's value.
    let outer_in = |k: usize| feeds.iter().find(|l| l.to_socket == k).map(|l| (l.from_node, l.from_socket));
    // What reaches Group Output input k from inside, in parent-tree terms.
    let inner_out = |k: usize| -> Option<Result<(usize, usize), usize>> {
        let l = g.links.iter().find(|l| g.nodes[l.to_node].idname == GROUP_OUTPUT && l.to_socket == k)?;
        if g.nodes[l.from_node].idname == GROUP_INPUT {
            Some(Err(l.from_socket))
        } else {
            map[l.from_node].map(|n| Ok((n, l.from_socket)))
        }
    };
    let mut set_values: Vec<(usize, usize, Value)> = Vec::new();
    for l in &g.links {
        let fi = g.nodes[l.from_node].idname.as_str();
        match (map[l.from_node], map[l.to_node]) {
            (Some(a), Some(b)) => new_links.push((a, l.from_socket, b, l.to_socket)),
            (None, Some(b)) if fi == GROUP_INPUT => match outer_in(l.from_socket) {
                Some((a, s)) => new_links.push((a, s, b, l.to_socket)),
                None => {
                    if let Some(sock) = gnode_inputs.get(l.from_socket) {
                        set_values.push((b, l.to_socket, sock.value.clone()));
                    }
                }
            },
            _ => {}
        }
    }
    for l in &takes {
        match inner_out(l.from_socket) {
            Some(Ok((a, s))) => new_links.push((a, s, l.to_node, l.to_socket)),
            Some(Err(k)) => {
                if let Some((a, s)) = outer_in(k) {
                    new_links.push((a, s, l.to_node, l.to_socket));
                }
            }
            None => {}
        }
    }
    for (b, s, v) in set_values {
        if let Some(sock) = t.nodes[b].inputs.get_mut(s) {
            if std::mem::discriminant(&sock.value) == std::mem::discriminant(&v) {
                sock.value = v;
            }
        }
    }
    for (a, s, b, ts) in new_links {
        t.links.push(Link { from_node: a, from_socket: s, to_node: b, to_socket: ts, muted: false });
    }
    let brought: Vec<usize> = (base..t.nodes.len()).collect();
    let rm = remove_nodes(t, &[node]);
    let brought: Vec<usize> = brought.into_iter().filter_map(|i| rm[i]).collect();
    // Drop any duplicate links into one input (keep the last).
    let mut seen: Vec<(usize, usize)> = Vec::new();
    let mut links = std::mem::take(&mut t.links);
    links.reverse();
    links.retain(|l| {
        let k = (l.to_node, l.to_socket);
        if seen.contains(&k) {
            false
        } else {
            seen.push(k);
            true
        }
    });
    links.reverse();
    t.links = links;
    t.active = brought.first().copied();
    Some(brought)
}

/// Rebuild a node's sockets, keeping values (by identifier) and the links
/// to sockets that still exist (by position).
fn rebuild(t: &mut NodeTree, n: usize, inputs: Option<Vec<Socket>>, outputs: Option<Vec<Socket>>) {
    if let Some(mut ins) = inputs {
        for s in &mut ins {
            if let Some(old) = t.nodes[n].inputs.iter().find(|o| o.identifier == s.identifier && o.ty == s.ty) {
                s.value = old.value.clone();
                s.hide = old.hide;
            }
        }
        let len = ins.len();
        t.links.retain(|l| l.to_node != n || l.to_socket < len);
        t.nodes[n].inputs = ins;
    }
    if let Some(mut outs) = outputs {
        for s in &mut outs {
            if let Some(old) = t.nodes[n].outputs.iter().find(|o| o.identifier == s.identifier && o.ty == s.ty) {
                s.hide = old.hide;
            }
        }
        let len = outs.len();
        t.links.retain(|l| l.from_node != n || l.from_socket < len);
        t.nodes[n].outputs = outs;
    }
}

/// Bring group nodes in `t` (and, when `t` is itself group `own`, its Group
/// Input/Output nodes) in line with the groups' interfaces.
pub fn sync_group_nodes(t: &mut NodeTree, groups: &[NodeTree], own: Option<usize>) {
    for n in 0..t.nodes.len() {
        if let Some(gi) = group_of(&t.nodes[n]) {
            if let Some(g) = groups.get(gi) {
                let gn = group_node(t.kind, gi, g);
                rebuild(t, n, Some(gn.inputs), Some(gn.outputs));
            }
        } else if let Some(g) = own.and_then(|o| groups.get(o)) {
            match t.nodes[n].idname.as_str() {
                GROUP_INPUT => {
                    let s = group_input_node(g).outputs;
                    rebuild(t, n, None, Some(s));
                }
                GROUP_OUTPUT => {
                    let s = group_output_node(g).inputs;
                    rebuild(t, n, Some(s), None);
                }
                _ => {}
            }
        }
    }
}

/// After group `gi`'s inputs (`input`) or outputs changed order, move the
/// links on its sockets: `map(old index)` gives the new index or None for a
/// removed socket. `own` says `t` is group `gi` itself.
pub fn remap_group_links(t: &mut NodeTree, gi: usize, input: bool, own: bool, map: &dyn Fn(usize) -> Option<usize>) {
    let is_gnode = |n: &Node| group_of(n) == Some(gi);
    let mut keep = Vec::with_capacity(t.links.len());
    for l in t.links.clone() {
        let mut l = l;
        let mut ok = true;
        if input && is_gnode(&t.nodes[l.to_node]) {
            match map(l.to_socket) {
                Some(s) => l.to_socket = s,
                None => ok = false,
            }
        }
        if !input && is_gnode(&t.nodes[l.from_node]) {
            match map(l.from_socket) {
                Some(s) => l.from_socket = s,
                None => ok = false,
            }
        }
        if own && input && t.nodes[l.from_node].idname == GROUP_INPUT {
            match map(l.from_socket) {
                Some(s) => l.from_socket = s,
                None => ok = false,
            }
        }
        if own && !input && t.nodes[l.to_node].idname == GROUP_OUTPUT {
            match map(l.to_socket) {
                Some(s) => l.to_socket = s,
                None => ok = false,
            }
        }
        if ok {
            keep.push(l);
        }
    }
    t.links = keep;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> NodeTree {
        // Value -> Math -> Math.001 -> Output (Principled feeds output too).
        let mut t = NodeTree::default_material("M");
        let v = t.add("ShaderNodeValue", [-600.0, 0.0]).unwrap();
        let a = t.add("ShaderNodeMath", [-400.0, 0.0]).unwrap();
        let b = t.add("ShaderNodeMath", [-300.0, 0.0]).unwrap();
        let p = t.find_type("ShaderNodeBsdfPrincipled").unwrap();
        t.link(v, "Value", a, "Value").unwrap();
        t.link(a, "Value", b, "Value").unwrap();
        t.link(b, "Value", p, "Roughness").unwrap();
        t
    }

    #[test]
    fn dissolve_reconnects() {
        let mut t = chain();
        let a = t.find("Math").unwrap();
        delete_reconnect(&mut t, &[a]);
        let v = t.find("Value").unwrap();
        let b = t.find("Math.001").unwrap();
        assert!(t.links.iter().any(|l| l.from_node == v && l.to_node == b));
        // Dissolving both maths links Value straight to Roughness.
        let mut t = chain();
        let (a, b) = (t.find("Math").unwrap(), t.find("Math.001").unwrap());
        delete_reconnect(&mut t, &[a, b]);
        let v = t.find("Value").unwrap();
        let p = t.find_type("ShaderNodeBsdfPrincipled").unwrap();
        let r = t.nodes[p].input("Roughness").unwrap();
        assert_eq!(t.input_link(p, r).map(|l| l.from_node), Some(v));
    }

    #[test]
    fn duplicate_copies_internal_links() {
        let mut t = chain();
        let (a, b) = (t.find("Math").unwrap(), t.find("Math.001").unwrap());
        let n = t.links.len();
        let new = duplicate(&mut t, &[a, b], false);
        assert_eq!(new.len(), 2);
        assert_eq!(t.links.len(), n + 1);
        assert!(t.nodes[new[0]].select && !t.nodes[a].select);
        let new2 = duplicate(&mut t, &[b], true);
        assert!(t.links.iter().any(|l| l.to_node == new2[0]));
    }

    #[test]
    fn group_round_trip() {
        let mut t = chain();
        let before = t.clone();
        let mut groups = Vec::new();
        let (a, b) = (t.find("Math").unwrap(), t.find("Math.001").unwrap());
        let g = make_group(&mut t, &[a, b], &mut groups).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].inputs.len(), 1);
        assert_eq!(groups[0].outputs.len(), 1);
        assert_eq!(t.nodes[g].inputs.len(), 1);
        assert_eq!(t.nodes.len(), before.nodes.len() - 1);
        let p = t.find_type("ShaderNodeBsdfPrincipled").unwrap();
        let r = t.nodes[p].input("Roughness").unwrap();
        assert_eq!(t.input_link(p, r).map(|l| l.from_node), Some(g));
        // Ungroup brings the same structure back.
        let brought = ungroup(&mut t, g, &groups).unwrap();
        assert_eq!(brought.len(), 2);
        assert_eq!(t.nodes.len(), before.nodes.len());
        assert_eq!(t.links.len(), before.links.len());
        let v = t.find("Value").unwrap();
        let m = t.links.iter().find(|l| l.from_node == v).unwrap().to_node;
        assert_eq!(t.nodes[m].idname, "ShaderNodeMath");
        let p = t.find_type("ShaderNodeBsdfPrincipled").unwrap();
        assert!(t.input_link(p, r).is_some());
    }

    #[test]
    fn make_links_pairs_left_to_right() {
        let mut t = NodeTree::new("T", TreeKind::Shader);
        let a = t.add("ShaderNodeValue", [0.0, 0.0]).unwrap();
        let b = t.add("ShaderNodeMath", [200.0, 0.0]).unwrap();
        assert_eq!(make_links(&mut t, &[b, a], false), 1);
        assert_eq!(t.links[0].from_node, a);
        assert!(!can_link(SocketType::Shader, SocketType::Float));
        assert!(can_link(SocketType::Color, SocketType::Shader));
    }
}
