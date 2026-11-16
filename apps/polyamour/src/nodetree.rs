// SPDX-License-Identifier: GPL-3.0-only
//! The node trees a document holds (material and world shader trees, node
//! groups, the compositor) addressed by [`TreeRef`], the legacy material
//! fields kept in step with a material's Principled BSDF (the Solid and
//! Material viewports and the exporters read those), and the node operators
//! the Properties editor and the node editors share through [`Doc::run`].
use crate::doc::Doc;
use crate::ui::Op;
use sex_3d::nodes::{self, edit, NodeTree, Prop, Value};
use sex_3d::scene::{Material, Scene, World};
use sex_3d::Vec3;

/// Which tree an editor or operator works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeRef {
    Material(usize),
    World,
    /// `Scene::node_groups[i]`.
    Group(usize),
    Compositor,
}

impl TreeRef {
    pub fn get(self, s: &Scene) -> Option<&NodeTree> {
        match self {
            TreeRef::Material(i) => s.materials.get(i)?.node_tree.as_ref(),
            TreeRef::World => s.world.node_tree.as_ref(),
            TreeRef::Group(i) => s.node_groups.get(i),
            TreeRef::Compositor => s.compositor.as_ref(),
        }
    }
    pub fn get_mut(self, s: &mut Scene) -> Option<&mut NodeTree> {
        match self {
            TreeRef::Material(i) => s.materials.get_mut(i)?.node_tree.as_mut(),
            TreeRef::World => s.world.node_tree.as_mut(),
            TreeRef::Group(i) => s.node_groups.get_mut(i),
            TreeRef::Compositor => s.compositor.as_mut(),
        }
    }
    pub fn group(self) -> Option<usize> {
        match self {
            TreeRef::Group(i) => Some(i),
            _ => None,
        }
    }
    /// Text form for operator arguments: "material:2", "world", "group:0", "compositor".
    pub fn encode(self) -> String {
        match self {
            TreeRef::Material(i) => format!("material:{i}"),
            TreeRef::World => "world".into(),
            TreeRef::Group(i) => format!("group:{i}"),
            TreeRef::Compositor => "compositor".into(),
        }
    }
    pub fn decode(s: &str) -> Option<TreeRef> {
        match s.split_once(':') {
            Some(("material", i)) => i.parse().ok().map(TreeRef::Material),
            Some(("group", i)) => i.parse().ok().map(TreeRef::Group),
            _ => match s {
                "world" => Some(TreeRef::World),
                "compositor" => Some(TreeRef::Compositor),
                _ => None,
            },
        }
    }
}

/// A colour inside a node a colour picker edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeColor {
    Input(usize),
    Output(usize),
    /// A `Prop::Color` by property index.
    Prop(usize),
    /// Stop `1` of the colour ramp in property `0`.
    RampStop(usize, usize),
    /// The node's custom colour.
    Custom,
}

/// Read a node colour (linear RGB).
pub fn get_color(s: &Scene, r: TreeRef, n: usize, c: NodeColor) -> Option<[f32; 3]> {
    let node = r.get(s)?.nodes.get(n)?;
    let v = match c {
        NodeColor::Input(i) => node.inputs.get(i)?.value.as_color(),
        NodeColor::Output(i) => node.outputs.get(i)?.value.as_color(),
        NodeColor::Prop(p) => match &node.props.get(p)?.1 {
            Prop::Color(c) => *c,
            Prop::Vector(v) => [v[0], v[1], v[2], 1.0],
            _ => return None,
        },
        NodeColor::RampStop(p, e) => match &node.props.get(p)?.1 {
            Prop::Ramp(r) => r.elements.get(e)?.1,
            _ => return None,
        },
        NodeColor::Custom => {
            let c = node.color.unwrap_or([0.6, 0.6, 0.6]);
            [c[0], c[1], c[2], 1.0]
        }
    };
    Some([v[0], v[1], v[2]])
}

/// Write a node colour (keeps alpha) and keep the legacy fields in step.
pub fn set_color(s: &mut Scene, r: TreeRef, n: usize, c: NodeColor, rgb: [f32; 3]) {
    let Some(node) = r.get_mut(s).and_then(|t| t.nodes.get_mut(n)) else { return };
    let put = |v: &mut Value| {
        let a = v.as_color()[3];
        *v = Value::Color([rgb[0], rgb[1], rgb[2], a]);
    };
    match c {
        NodeColor::Input(i) => {
            if let Some(so) = node.inputs.get_mut(i) {
                put(&mut so.value)
            }
        }
        NodeColor::Output(i) => {
            if let Some(so) = node.outputs.get_mut(i) {
                put(&mut so.value)
            }
        }
        NodeColor::Prop(p) => {
            if let Some((_, pr)) = node.props.get_mut(p) {
                match pr {
                    Prop::Color(c) => c[..3].copy_from_slice(&rgb),
                    Prop::Vector(v) => *v = rgb,
                    _ => {}
                }
            }
        }
        NodeColor::RampStop(p, e) => {
            if let Some((_, Prop::Ramp(ramp))) = node.props.get_mut(p) {
                if let Some(el) = ramp.elements.get_mut(e) {
                    el.1[..3].copy_from_slice(&rgb);
                }
            }
        }
        NodeColor::Custom => node.color = Some(rgb),
    }
    sync_legacy(s, r);
}

fn set(t: &mut NodeTree, n: usize, name: &str, v: Value) {
    t.nodes[n].set_input(name, v);
}

/// A material's node tree built from its legacy fields (Use Nodes on a
/// material that had none): Principled BSDF into Material Output, with an
/// Image Texture on Base Color when the material had a colour texture.
pub fn seed_material_tree(m: &Material) -> NodeTree {
    let mut t = NodeTree::default_material(&m.name);
    if let Some(p) = t.find_type("ShaderNodeBsdfPrincipled") {
        let c = m.base_color;
        set(&mut t, p, "Base Color", Value::Color([c[0], c[1], c[2], 1.0]));
        set(&mut t, p, "Metallic", Value::Float(m.metallic));
        set(&mut t, p, "Roughness", Value::Float(m.roughness));
        set(&mut t, p, "IOR", Value::Float(m.ior));
        set(&mut t, p, "Alpha", Value::Float(c[3]));
        set(&mut t, p, "Transmission Weight", Value::Float(m.transmission));
        let e = m.emission;
        set(&mut t, p, "Emission Color", Value::Color([e.x, e.y, e.z, 1.0]));
        set(&mut t, p, "Emission Strength", Value::Float(m.emission_strength));
        if let Some(img) = m.base_color_texture {
            let loc = t.nodes[p].location;
            if let Some(i) = t.add("ShaderNodeTexImage", [loc[0] - 300.0, loc[1]]) {
                t.nodes[i].set_prop("image", Prop::Id(Some(img)));
                let _ = t.link(i, "Color", p, "Base Color");
            }
        }
    }
    t
}

/// The world's tree from its colour, strength and HDRI.
pub fn seed_world_tree(w: &World) -> NodeTree {
    let mut t = NodeTree::default_world("World");
    if let Some(b) = t.find_type("ShaderNodeBackground") {
        set(&mut t, b, "Color", Value::Color([w.color.x, w.color.y, w.color.z, 1.0]));
        set(&mut t, b, "Strength", Value::Float(w.strength));
        if let Some(img) = w.hdri {
            let loc = t.nodes[b].location;
            if let Some(e) = t.add("ShaderNodeTexEnvironment", [loc[0] - 300.0, loc[1]]) {
                t.nodes[e].set_prop("image", Prop::Id(Some(img)));
                let _ = t.link(e, "Color", b, "Color");
            }
        }
    }
    t
}

/// The node linked into `out_idname`'s `input`, if any.
pub fn surface_node(t: &NodeTree, out_idname: &str, input: &str) -> Option<usize> {
    let o = t.output_node(out_idname)?;
    let s = t.nodes[o].input(input)?;
    t.input_link(o, s).map(|l| l.from_node)
}

/// The image an input is textured with (an Image/Environment Texture linked
/// straight into it).
fn linked_image(t: &NodeTree, n: usize, input: &str) -> Option<Option<usize>> {
    let s = t.nodes[n].input(input)?;
    let l = t.input_link(n, s)?;
    let src = &t.nodes[l.from_node];
    if !matches!(src.idname.as_str(), "ShaderNodeTexImage" | "ShaderNodeTexEnvironment") {
        return None;
    }
    match src.prop("image") {
        Some(Prop::Id(i)) => Some(*i),
        _ => Some(None),
    }
}

/// Copy a tree's Principled BSDF (or Diffuse/Emission) values that are not
/// driven by links into the legacy material fields; the world's Background
/// into the world colour and strength.
pub fn sync_legacy(scene: &mut Scene, r: TreeRef) {
    match r {
        TreeRef::Material(mi) => {
            let Some(m) = scene.materials.get_mut(mi) else { return };
            let Some(t) = m.node_tree.as_ref() else { return };
            let Some(n) = surface_node(t, "ShaderNodeOutputMaterial", "Surface") else { return };
            let node = &t.nodes[n];
            let val = |name: &str| -> Option<Value> {
                let i = node.input(name)?;
                t.input_link(n, i).is_none().then(|| node.inputs[i].value.clone())
            };
            match node.idname.as_str() {
                "ShaderNodeBsdfPrincipled" | "ShaderNodeBsdfDiffuse" => {
                    if let Some(v) = val(if node.idname == "ShaderNodeBsdfDiffuse" { "Color" } else { "Base Color" }) {
                        let c = v.as_color();
                        m.base_color[..3].copy_from_slice(&c[..3]);
                    }
                    let tex = linked_image(t, n, if node.idname == "ShaderNodeBsdfDiffuse" { "Color" } else { "Base Color" });
                    m.base_color_texture = tex.flatten();
                    if let Some(v) = val("Metallic") {
                        m.metallic = v.as_float().clamp(0.0, 1.0);
                    }
                    if let Some(v) = val("Roughness") {
                        m.roughness = v.as_float().clamp(0.0, 1.0);
                    }
                    if let Some(v) = val("IOR") {
                        m.ior = v.as_float().max(1.0);
                    }
                    if let Some(v) = val("Alpha") {
                        m.base_color[3] = v.as_float().clamp(0.0, 1.0);
                    }
                    if let Some(v) = val("Transmission Weight") {
                        m.transmission = v.as_float().clamp(0.0, 1.0);
                    }
                    if let Some(v) = val("Emission Color") {
                        m.emission = Vec3::from_array(v.as_vector());
                    }
                    if let Some(v) = val("Emission Strength") {
                        m.emission_strength = v.as_float().max(0.0);
                    }
                    if node.idname == "ShaderNodeBsdfDiffuse" {
                        m.metallic = 0.0;
                        m.emission_strength = 0.0;
                    }
                }
                "ShaderNodeEmission" => {
                    if let Some(v) = val("Color") {
                        m.emission = Vec3::from_array(v.as_vector());
                        let c = v.as_color();
                        m.base_color[..3].copy_from_slice(&c[..3]);
                    }
                    if let Some(v) = val("Strength") {
                        m.emission_strength = v.as_float().max(0.0);
                    }
                }
                _ => {}
            }
        }
        TreeRef::World => {
            let w = &mut scene.world;
            let Some(t) = w.node_tree.as_ref() else { return };
            let Some(n) = surface_node(t, "ShaderNodeOutputWorld", "Surface") else { return };
            if t.nodes[n].idname != "ShaderNodeBackground" {
                return;
            }
            let node = &t.nodes[n];
            if let Some(i) = node.input("Color") {
                match t.input_link(n, i) {
                    None => {
                        w.color = Vec3::from_array(node.inputs[i].value.as_vector());
                        w.hdri = None;
                    }
                    Some(_) => {
                        if let Some(img) = linked_image(t, n, "Color") {
                            w.hdri = img;
                        }
                    }
                }
            }
            if let Some(i) = node.input("Strength") {
                if t.input_link(n, i).is_none() {
                    w.strength = node.inputs[i].value.as_float().max(0.0);
                }
            }
        }
        _ => {}
    }
}

/// After a tree changed: keep the legacy fields in step and refresh views.
pub fn after_edit(d: &mut Doc, r: TreeRef) {
    sync_legacy(&mut d.scene, r);
    d.touch();
}

/// Every tree in the scene, with the group index for node groups.
pub fn each_tree_mut(s: &mut Scene, mut f: impl FnMut(&mut NodeTree, Option<usize>)) {
    for m in &mut s.materials {
        if let Some(t) = &mut m.node_tree {
            f(t, None);
        }
    }
    if let Some(t) = &mut s.world.node_tree {
        f(t, None);
    }
    if let Some(t) = &mut s.compositor {
        f(t, None);
    }
    for (i, g) in s.node_groups.iter_mut().enumerate() {
        f(g, Some(i));
    }
}

/// After group `gi`'s interface changed: remap links with `map` (see
/// [`edit::remap_group_links`]) and rebuild every group node's sockets.
pub fn interface_changed(s: &mut Scene, gi: usize, input: bool, map: &dyn Fn(usize) -> Option<usize>) {
    each_tree_mut(s, |t, own| edit::remap_group_links(t, gi, input, own == Some(gi), map));
    let groups = s.node_groups.clone();
    each_tree_mut(s, |t, own| edit::sync_group_nodes(t, &groups, own));
}

/// Turn on Use Nodes for a material or the world (a tree seeded from the
/// legacy fields).
pub fn use_nodes(d: &mut Doc, r: TreeRef) -> bool {
    match r {
        TreeRef::Material(mi) => {
            let Some(m) = d.scene.materials.get(mi) else { return false };
            if m.node_tree.is_some() {
                return false;
            }
            let t = seed_material_tree(m);
            d.push_undo("Use Nodes");
            d.scene.materials[mi].node_tree = Some(t);
        }
        TreeRef::World => {
            if d.scene.world.node_tree.is_some() {
                return false;
            }
            let t = seed_world_tree(&d.scene.world);
            d.push_undo("Use Nodes");
            d.scene.world.node_tree = Some(t);
        }
        TreeRef::Compositor => {
            if d.scene.compositor.is_some() {
                return false;
            }
            d.push_undo("Use Nodes");
            let mut t = NodeTree::new("Compositing Nodetree", nodes::TreeKind::Compositor);
            let rl = t.add("CompositorNodeRLayers", [-300.0, 300.0]);
            let co = t.add("CompositorNodeComposite", [200.0, 300.0]);
            if let (Some(a), Some(b)) = (rl, co) {
                let _ = t.link_sockets(a, 0, b, 0);
            }
            d.scene.compositor = Some(t);
            let passes = d.settings.render_settings().passes;
            crate::compositing::sync_passes(&mut d.scene, passes);
        }
        TreeRef::Group(_) => return false,
    }
    after_edit(d, r);
    true
}

/// Set property `prop` of node `n` from its text form: enum identifiers as
/// they are, booleans "0"/"1", numbers, data-block indices ("" for none).
pub fn set_prop_text(t: &mut NodeTree, n: usize, prop: &str, value: &str) -> bool {
    let Some(node) = t.nodes.get_mut(n) else { return false };
    let v = match node.prop(prop) {
        Some(Prop::Bool(_)) => Prop::Bool(value == "1" || value == "true"),
        Some(Prop::Int(_)) => Prop::Int(value.parse().unwrap_or(0)),
        Some(Prop::Float(_)) => Prop::Float(value.parse().unwrap_or(0.0)),
        Some(Prop::Id(_)) => Prop::Id(value.parse().ok()),
        Some(Prop::String(_)) => Prop::String(value.into()),
        _ => Prop::Enum(value.into()),
    };
    node.set_prop(prop, v);
    nodes::update::update(node);
    true
}

/// Node operators that name their tree in the argument; the node editors and
/// the Properties editor queue these from menus.
///
/// * `node.set_prop` "TREE\tNODE\tPROP\tVALUE"
/// * `node.use_nodes` "TREE"
/// * `node.replace_surface` "TREE\tNODE\tIDNAME": swap the shader node that
///   feeds an output for a new one of another type (Properties' Surface menu).
/// A small textured material for screenshots and demos: a generated checker
/// Image Texture on the active object's first material, mixed with a colour
/// into the Principled BSDF's Base Color, and a Math node on Roughness.
pub fn demo_textures(d: &mut Doc) -> Option<usize> {
    let s = &mut d.scene;
    let obj = s.active_object.or(s.selected.first().copied())?;
    let mi = *s.objects.get(obj)?.materials.first()?;
    let (w, h) = (64u32, 64u32);
    let pixels = (0..w * h)
        .map(|i| if ((i % w) / 8 + (i / w) / 8) % 2 == 0 { [0.8, 0.8, 0.8, 1.0] } else { [0.05, 0.05, 0.05, 1.0] })
        .collect();
    s.images.push(sex_3d::scene::Image { name: "Checker".into(), width: w, height: h, pixels });
    let img = s.images.len() - 1;
    let m = s.materials.get_mut(mi)?;
    if m.node_tree.is_none() {
        m.node_tree = Some(seed_material_tree(m));
    }
    let t = m.node_tree.as_mut()?;
    let p = t.find_type("ShaderNodeBsdfPrincipled")?;
    let [x, y] = t.nodes[p].location;
    let coord = t.add("ShaderNodeTexCoord", [x - 760.0, y - 20.0])?;
    let tex = t.add("ShaderNodeTexImage", [x - 540.0, y])?;
    t.nodes[tex].set_prop("image", Prop::Id(Some(img)));
    let rgb = t.add("ShaderNodeRGB", [x - 540.0, y - 300.0])?;
    t.nodes[rgb].outputs[0].value = Value::Color([0.8, 0.25, 0.05, 1.0]);
    let mix = t.add("ShaderNodeMix", [x - 260.0, y])?;
    t.nodes[mix].set_prop("blend_type", Prop::Enum("MULTIPLY".into()));
    nodes::update::update(&mut t.nodes[mix]);
    let math = t.add("ShaderNodeMath", [x - 260.0, y - 260.0])?;
    t.nodes[math].set_prop("operation", Prop::Enum("MULTIPLY".into()));
    nodes::update::update(&mut t.nodes[math]);
    set(t, math, "Value_001", Value::Float(0.6));
    t.link(coord, "UV", tex, "Vector").ok()?;
    t.link(tex, "Color", mix, "A").ok()?;
    t.link(rgb, "Color", mix, "B").ok()?;
    t.link(mix, "Result", p, "Base Color").ok()?;
    t.link(tex, "Alpha", math, "Value").ok()?;
    t.link(math, "Value", p, "Roughness").ok()?;
    t.active = Some(mix);
    sync_legacy(s, TreeRef::Material(mi));
    d.touch();
    Some(mi)
}

pub fn run(d: &mut Doc, op: &Op) -> bool {
    match op.id.as_str() {
        "node.set_prop" => {
            let f: Vec<&str> = op.arg.splitn(4, '\t').collect();
            if f.len() < 4 {
                return true;
            }
            let (Some(r), Ok(n)) = (TreeRef::decode(f[0]), f[1].parse::<usize>()) else { return true };
            if r.get(&d.scene).and_then(|t| t.nodes.get(n)).is_none() {
                return true;
            }
            d.push_undo("Change Node Property");
            let groups = d.scene.node_groups.clone();
            if let Some(t) = r.get_mut(&mut d.scene) {
                set_prop_text(t, n, f[2], f[3]);
                if f[2] == "node_tree" {
                    edit::sync_group_nodes(t, &groups, r.group());
                }
            }
            after_edit(d, r);
            true
        }
        "node.interface_type" => {
            let f: Vec<&str> = op.arg.split('\t').collect();
            if f.len() < 4 {
                return true;
            }
            let (Ok(g), Ok(i)) = (f[0].parse::<usize>(), f[2].parse::<usize>()) else { return true };
            let ty = match f[3] {
                "Float" => nodes::SocketType::Float,
                "Integer" => nodes::SocketType::Int,
                "Boolean" => nodes::SocketType::Bool,
                "Vector" => nodes::SocketType::Vector,
                "Color" => nodes::SocketType::Color,
                "Shader" => nodes::SocketType::Shader,
                "Geometry" => nodes::SocketType::Geometry,
                "String" => nodes::SocketType::String,
                _ => return true,
            };
            let input = f[1] == "in";
            let Some(gr) = d.scene.node_groups.get(g) else { return true };
            if (if input { gr.inputs.len() } else { gr.outputs.len() }) <= i {
                return true;
            }
            d.push_undo("Socket Type");
            let gr = &mut d.scene.node_groups[g];
            let so = if input { &mut gr.inputs[i] } else { &mut gr.outputs[i] };
            so.ty = ty;
            so.default = edit::type_default(ty);
            let groups = d.scene.node_groups.clone();
            each_tree_mut(&mut d.scene, |t, own| edit::sync_group_nodes(t, &groups, own));
            d.touch();
            true
        }
        "node.use_nodes" => {
            if let Some(r) = TreeRef::decode(&op.arg) {
                use_nodes(d, r);
            }
            true
        }
        "node.replace_surface" => {
            let f: Vec<&str> = op.arg.splitn(3, '\t').collect();
            if f.len() < 3 {
                return true;
            }
            let (Some(r), Ok(n)) = (TreeRef::decode(f[0]), f[1].parse::<usize>()) else { return true };
            if r.get(&d.scene).and_then(|t| t.nodes.get(n)).is_none() || nodes::new_node(f[2]).is_none() {
                return true;
            }
            d.push_undo("Replace Shader");
            if let Some(t) = r.get_mut(&mut d.scene) {
                replace_node(t, n, f[2]);
            }
            after_edit(d, r);
            true
        }
        _ => false,
    }
}

/// Replace node `n` by a new node of type `idname` at its place, moving its
/// output links over (Blender's Properties shader menu).
pub fn replace_node(t: &mut NodeTree, n: usize, idname: &str) -> Option<usize> {
    let loc = t.nodes[n].location;
    let k = t.add(idname, loc)?;
    let outs: Vec<(usize, usize)> = t.links.iter().filter(|l| l.from_node == n).map(|l| (l.to_node, l.to_socket)).collect();
    // Carry over inputs by name where the types agree.
    let ins: Vec<(String, Value, Option<(usize, usize)>)> =
        t.nodes[n].inputs.iter().enumerate().map(|(i, s)| (s.name.clone(), s.value.clone(), t.input_link(n, i).map(|l| (l.from_node, l.from_socket)))).collect();
    for (name, v, link) in ins {
        let Some(j) = t.nodes[k].inputs.iter().position(|s| s.name == name && s.enabled) else { continue };
        if std::mem::discriminant(&t.nodes[k].inputs[j].value) == std::mem::discriminant(&v) {
            t.nodes[k].inputs[j].value = v;
        }
        if let Some((a, s)) = link {
            let _ = t.link_sockets(a, s, k, j);
        }
    }
    let first_out = t.nodes[k].outputs.iter().position(|s| s.enabled)?;
    for (b, s) in outs {
        let _ = t.link_sockets(k, first_out, b, s);
    }
    t.remove(n);
    Some(if k > n { k - 1 } else { k })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_textures_builds_a_linked_tree() {
        let mut d = Doc::default();
        let mi = demo_textures(&mut d).expect("startup scene has a material");
        let t = d.scene.materials[mi].node_tree.as_ref().unwrap();
        assert!(t.find_type("ShaderNodeTexImage").is_some() && t.find_type("ShaderNodeMix").is_some());
        assert!(t.links.len() >= 7, "{:?}", t.links);
    }

    #[test]
    fn seeded_tree_syncs_back_to_the_material() {
        let mut s = Scene::startup();
        let m = Material { roughness: 0.25, metallic: 1.0, ..Material::default() };
        let mi = s.add_material(m);
        s.materials[mi].node_tree = Some(seed_material_tree(&s.materials[mi]));
        let t = s.materials[mi].node_tree.as_mut().unwrap();
        let p = t.find_type("ShaderNodeBsdfPrincipled").unwrap();
        assert_eq!(t.nodes[p].inputs[t.nodes[p].input("Roughness").unwrap()].value, Value::Float(0.25));
        t.nodes[p].set_input("Roughness", Value::Float(0.75));
        t.nodes[p].set_input("Base Color", Value::Color([1.0, 0.0, 0.0, 1.0]));
        sync_legacy(&mut s, TreeRef::Material(mi));
        assert_eq!(s.materials[mi].roughness, 0.75);
        assert_eq!(s.materials[mi].metallic, 1.0);
        assert_eq!(&s.materials[mi].base_color[..3], &[1.0, 0.0, 0.0]);
        for r in [TreeRef::Material(3), TreeRef::World, TreeRef::Group(1), TreeRef::Compositor] {
            assert_eq!(TreeRef::decode(&r.encode()), Some(r));
        }
    }
}
