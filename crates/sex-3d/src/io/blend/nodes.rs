// SPDX-License-Identifier: GPL-3.0-only
//! Shader node trees from `.blend` files (field names from Blender's
//! `DNA_node_types.h`, GPL-2+):
//!
//! - `bNodeTree`: `nodes` and `links` ListBases; group interfaces from 4.0
//!   in `tree_interface.root_panel` (`bNodeTreeInterfacePanel {items_array,
//!   items_num}` of `bNodeTreeInterfaceSocket {name, socket_type, flag (bit 0
//!   input, bit 1 output), identifier, socket_data}` and nested panels),
//!   before 4.0 in the `inputs`/`outputs` ListBases of `bNodeSocket`.
//! - `bNode`: `name`, `idname`, `label`, `locx`/`locy` (4.4+: `location[2]`),
//!   `width`, `flag` (`NODE_HIDDEN` 1<<3, `NODE_DO_OUTPUT` 1<<6, `NODE_MUTED`
//!   1<<9), `parent`, `color[3]`, `custom1..4`, `storage`, `id`, and the
//!   `inputs`/`outputs` socket lists.
//! - `bNodeSocket`: `identifier`, `name`, `type`, `flag` (`SOCK_HIDDEN` 1<<1),
//!   `default_value` -> `bNodeSocketValueFloat/Int/Boolean {value}`,
//!   `bNodeSocketValueVector {value[3]}`, `bNodeSocketValueRGBA {value[4]}`.
//! - `bNodeLink`: `fromnode`, `tonode`, `fromsock`, `tosock`, `flag`
//!   (`NODE_LINK_MUTED` 1<<4).
//! - Node settings: `custom1` (Math/Vector Math operation, Mapping type,
//!   Clamp type, Vector Rotate type, distribution, Toon component, AO
//!   samples, ...), `custom2` (Math clamp, legacy Mix alpha/clamp bits,
//!   Vector Rotate invert, AO inside), and storage structs `NodeShaderMix
//!   {data_type, factor_mode, clamp_factor, clamp_result, blend_type}`,
//!   `NodeTexImage {projection, projection_blend, interpolation,
//!   extension}`, `NodeTexEnvironment {projection, interpolation}`,
//!   `NodeTexNoise {dimensions, type, normalize}`, `NodeTexVoronoi
//!   {dimensions, feature, distance, normalize}`, `NodeTexWave`,
//!   `NodeTexMagic {depth}`, `NodeTexBrick`, `NodeTexGradient`,
//!   `NodeTexMusgrave`, `NodeTexSky`, `NodeTexGabor`, `NodeMapRange`,
//!   `NodeCombSepColor {mode}`, `NodeShaderNormalMap {space}`,
//!   `NodeShaderTangent`, `NodeShaderAttribute {name}`,
//!   `NodeShaderVectTransform`, `ColorBand {tot, ipotype, color_mode,
//!   data[32] CBData {r, g, b, a, pos}}` and `CurveMapping {cm[4] CurveMap
//!   {totpoint, curve -> CurveMapPoint {x, y}}}` (RGB curves keep the combined
//!   curve in `cm[3]`).
//! - `Image` (`IM`): the first packed file (`packedfiles` ListBase of
//!   `ImagePackedFile {packedfile}`, or the older `packedfile`) ->
//!   `PackedFile {size, data}`, decoded as PNG or JPEG;
//!   `colorspace_settings.name` "Non-Color"/"Raw"/"Linear*" keeps the values
//!   unconverted. Images without packed data import empty (they render pink,
//!   as missing images do in Blender).
//!
//! Pre-4.0 Principled sockets are renamed as Blender's versioning does
//! (Subsurface -> Subsurface Weight, Specular -> Specular IOR Level,
//! Clearcoat -> Coat Weight, Transmission -> Transmission Weight, Sheen ->
//! Sheen Weight, Emission -> Emission Color).
use super::{count, BlendFile, Importer, View};
use crate::nodes::{new_node, sync_group_sockets, update_sockets, ColorRamp, CurveMapping, InterfaceSocket, Link, Node, NodeTree, Prop, Socket, SocketType, TreeKind, Value};
use crate::render::shader_names::{blend_from_file, math_from_file, vector_math_from_file};
use crate::scene::Image;
use std::collections::HashMap;

const NODE_HIDDEN: i64 = 1 << 3;
const NODE_DO_OUTPUT: i64 = 1 << 6;
const NODE_MUTED: i64 = 1 << 9;
const NODE_CUSTOM_COLOR: i64 = 1 << 17;
const SOCK_HIDDEN: i64 = 1 << 1;
const NODE_LINK_MUTED: i64 = 1 << 4;

/// Socket type from Blender's `eNodeSocketDatatype`.
fn sock_type(t: i64) -> SocketType {
    match t {
        0 => SocketType::Float,
        1 => SocketType::Vector,
        2 => SocketType::Color,
        3 => SocketType::Shader,
        4 => SocketType::Bool,
        6 => SocketType::Int,
        7 => SocketType::String,
        8 => SocketType::Object,
        9 => SocketType::Image,
        _ => SocketType::Float,
    }
}

/// Socket type from an interface `socket_type` idname ("NodeSocketFloatFactor").
fn sock_type_name(s: &str) -> SocketType {
    let s = s.trim_start_matches("NodeSocket");
    if s.starts_with("Float") {
        SocketType::Float
    } else if s.starts_with("Int") {
        SocketType::Int
    } else if s.starts_with("Bool") {
        SocketType::Bool
    } else if s.starts_with("Vector") {
        SocketType::Vector
    } else if s.starts_with("Color") {
        SocketType::Color
    } else if s.starts_with("Shader") {
        SocketType::Shader
    } else if s.starts_with("String") {
        SocketType::String
    } else {
        SocketType::Float
    }
}

/// A socket's default value from the block it points at.
fn socket_value(f: &BlendFile<'_>, ptr: u64) -> Option<Value> {
    let v = f.view_own(ptr)?;
    Some(match v.struct_name() {
        "bNodeSocketValueFloat" => Value::Float(v.f32("value")?),
        "bNodeSocketValueInt" => Value::Int(v.num("value")? as i32),
        "bNodeSocketValueBoolean" => Value::Bool(v.num("value")? != 0.0),
        "bNodeSocketValueVector" | "bNodeSocketValueRotation" => Value::Vector(v.vec::<3>("value")?),
        "bNodeSocketValueRGBA" => Value::Color(v.vec::<4>("value")?),
        "bNodeSocketValueString" => Value::String(v.string("value")?),
        _ => return None,
    })
}

/// The value as the type of the socket it goes into.
fn convert(v: Value, ty: SocketType) -> Value {
    match (ty, &v) {
        (SocketType::Float, Value::Float(_)) | (SocketType::Color, Value::Color(_)) | (SocketType::Vector, Value::Vector(_)) | (SocketType::Int, Value::Int(_)) | (SocketType::Bool, Value::Bool(_)) => v,
        (SocketType::Float, _) => Value::Float(v.as_float()),
        (SocketType::Int, _) => Value::Int(v.as_float() as i32),
        (SocketType::Bool, _) => Value::Bool(v.as_float() != 0.0),
        (SocketType::Vector, _) => Value::Vector(v.as_vector()),
        (SocketType::Color, _) => Value::Color(v.as_color()),
        _ => v,
    }
}

/// Pre-4.0 Principled socket names.
fn principled_alias(id: &str) -> Option<&'static str> {
    Some(match id {
        "Subsurface" => "Subsurface Weight",
        "Specular" => "Specular IOR Level",
        "Clearcoat" => "Coat Weight",
        "Clearcoat Roughness" => "Coat Roughness",
        "Clearcoat Normal" => "Coat Normal",
        "Transmission" => "Transmission Weight",
        "Sheen" => "Sheen Weight",
        "Emission" => "Emission Color",
        "Sheen Tint" => "Sheen Tint",
        _ => return None,
    })
}

/// The sockets of a 4.x interface panel and its sub-panels, in order.
fn interface_panel(f: &BlendFile<'_>, panel: &View<'_, '_>, ins: &mut Vec<InterfaceSocket>, outs: &mut Vec<InterfaceSocket>, depth: usize) {
    let n = count(panel.num("items_num").unwrap_or(0.0));
    for p in f.ptr_array(panel.ptr("items_array"), n) {
        let Some(it) = f.view_own(p) else { continue };
        match it.struct_name() {
            "bNodeTreeInterfacePanel" if depth < 16 => interface_panel(f, &it, ins, outs, depth + 1),
            "bNodeTreeInterfaceSocket" => {
                let ty = sock_type_name(&f.cstr_at(it.ptr("socket_type")).unwrap_or_default());
                let default = socket_value(f, it.ptr("socket_data")).map(|v| convert(v, ty)).unwrap_or(Value::None);
                let s = InterfaceSocket { name: f.cstr_at(it.ptr("name")).unwrap_or_default(), ty, default, min: f32::MIN, max: f32::MAX };
                let flag = it.num("flag").unwrap_or(0.0) as i64;
                if flag & 2 != 0 {
                    outs.push(s.clone());
                }
                if flag & 1 != 0 {
                    ins.push(s);
                }
            }
            _ => {}
        }
    }
}

/// A `ListBase` walk that also returns each element's old address.
fn list_with_ptrs<'f, 'a>(f: &'f BlendFile<'a>, lb: Option<View<'f, 'a>>, names: &[&str]) -> Vec<(u64, View<'f, 'a>)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut p = lb.map_or(0, |l| l.ptr("first"));
    while p != 0 && seen.insert(p) && out.len() < 100_000 {
        match f.view_ptr(p, names) {
            Some(v) => {
                out.push((p, v));
                p = v.ptr("next");
            }
            None => break,
        }
    }
    out
}

fn e(s: &str) -> Prop {
    Prop::Enum(s.to_string())
}

fn pick(names: &[&'static str], i: f64) -> Prop {
    e(names.get(i.max(0.0) as usize).copied().unwrap_or(names[0]))
}

fn dims_name(d: f64) -> Prop {
    e(match d as i64 {
        1 => "1D",
        2 => "2D",
        4 => "4D",
        _ => "3D",
    })
}

const NOISE_TYPES: [&str; 5] = ["MULTIFRACTAL", "FBM", "HYBRID_MULTIFRACTAL", "RIDGED_MULTIFRACTAL", "HETERO_TERRAIN"];
const DISTRIBUTIONS: [&str; 5] = ["SHARP", "BECKMANN", "GGX", "ASHIKHMIN_SHIRLEY", "MULTI_GGX"];

fn color_ramp(f: &BlendFile<'_>, ptr: u64) -> Option<ColorRamp> {
    let cb = f.view_ptr(ptr, &["ColorBand"])?;
    let tot = count(cb.num("tot")?).min(32);
    let m = cb.member("data")?;
    let s = cb.sub("data")?.s;
    let size = f.sdna.structs[s].size;
    let mut elements = Vec::new();
    for i in 0..tot {
        let d = f.view(s, cb.data.get(m.offset + i * size..)?)?;
        elements.push((d.f32("pos")?, [d.f32("r")?, d.f32("g")?, d.f32("b")?, d.f32("a")?]));
    }
    elements.sort_by(|a, b| a.0.total_cmp(&b.0));
    let interpolation = ["LINEAR", "EASE", "B_SPLINE", "CARDINAL", "CONSTANT"].get(cb.num("ipotype").unwrap_or(0.0) as usize).copied().unwrap_or("LINEAR");
    let color_mode = ["RGB", "HSV", "HSL"].get(cb.num("color_mode").unwrap_or(0.0) as usize).copied().unwrap_or("RGB");
    Some(ColorRamp { interpolation: interpolation.into(), color_mode: color_mode.into(), elements })
}

/// `n` curves of a `CurveMapping` in file order.
fn curve_maps(f: &BlendFile<'_>, ptr: u64) -> Option<Vec<Vec<[f32; 2]>>> {
    let cm = f.view_ptr(ptr, &["CurveMapping"])?;
    let m = cm.member("cm")?;
    let s = cm.sub("cm")?.s;
    let size = f.sdna.structs[s].size;
    let mut out = Vec::new();
    for i in 0..m.len.min(4) {
        let c = f.view(s, cm.data.get(m.offset + i * size..)?)?;
        let n = count(c.num("totpoint").unwrap_or(0.0));
        let pts = f.view_array(c.ptr("curve"), &["CurveMapPoint"], n).unwrap_or_default();
        let mut v: Vec<[f32; 2]> = pts.iter().filter_map(|p| Some([p.f32("x")?, p.f32("y")?])).collect();
        if v.is_empty() {
            v = vec![[0.0, 0.0], [1.0, 1.0]];
        }
        out.push(v);
    }
    Some(out)
}

impl<'f, 'a> Importer<'f, 'a> {
    /// The scene image for an `IM` block, imported once.
    fn image(&mut self, ptr: u64) -> Option<usize> {
        if let Some(&i) = self.images.get(&ptr) {
            return Some(i);
        }
        let f = self.f;
        let im = f.view_ptr(ptr, &["Image"])?;
        let name = im.id_name();
        let packed = f
            .list(im.sub("packedfiles"), &["ImagePackedFile"])
            .first()
            .map(|p| p.ptr("packedfile"))
            .filter(|&p| p != 0)
            .unwrap_or_else(|| im.ptr("packedfile"));
        let mut img = Image { name: name.clone(), ..Image::default() };
        if let Some(pf) = f.view_ptr(packed, &["PackedFile"]) {
            let size = count(pf.num("size").unwrap_or(0.0));
            if let Some(bytes) = f.raw_array(pf.ptr("data"), 1, size) {
                if let Ok(mut d) = crate::io::decode_image(&name, bytes) {
                    let cs = im.sub("colorspace_settings").and_then(|c| c.string("name")).unwrap_or_default();
                    if cs == "Non-Color" || cs == "Raw" || cs.starts_with("Linear") {
                        // Data images: undo the sRGB decode.
                        for p in &mut d.pixels {
                            for c in p.iter_mut().take(3) {
                                *c = crate::render::srgb_encode(*c);
                            }
                        }
                    }
                    img = d;
                }
            }
        }
        let idx = self.scene.images.len();
        self.scene.images.push(img);
        self.images.insert(ptr, idx);
        Some(idx)
    }

    /// A node group (`NT` block), imported once into `Scene::node_groups`.
    fn group(&mut self, ptr: u64) -> Option<usize> {
        if let Some(&i) = self.groups.get(&ptr) {
            return Some(i);
        }
        let tree = self.f.view_ptr(ptr, &["bNodeTree"])?;
        let idx = self.scene.node_groups.len();
        // A placeholder first, so groups that contain themselves terminate.
        self.scene.node_groups.push(NodeTree::new(&tree.id_name(), TreeKind::Shader));
        self.groups.insert(ptr, idx);
        let mut t = self.node_tree(&tree)?;
        t.name = tree.id_name();
        self.scene.node_groups[idx] = t;
        Some(idx)
    }

    /// The interface of a group tree.
    fn interface(&self, tree: &View<'f, 'a>) -> (Vec<InterfaceSocket>, Vec<InterfaceSocket>) {
        let f = self.f;
        if let Some(root) = tree.sub("tree_interface").and_then(|ti| ti.sub("root_panel")) {
            let (mut ins, mut outs) = (Vec::new(), Vec::new());
            interface_panel(f, &root, &mut ins, &mut outs, 0);
            return (ins, outs);
        }
        let read = |lb: &str| -> Vec<InterfaceSocket> {
            f.list(tree.sub(lb), &["bNodeSocket"])
                .iter()
                .map(|s| {
                    let ty = sock_type(s.num("type").unwrap_or(0.0) as i64);
                    InterfaceSocket { name: s.string("name").unwrap_or_default(), ty, default: socket_value(f, s.ptr("default_value")).map(|v| convert(v, ty)).unwrap_or(Value::None), min: f32::MIN, max: f32::MAX }
                })
                .collect()
        };
        (read("inputs"), read("outputs"))
    }

    /// Read a whole node tree.
    pub(super) fn node_tree(&mut self, tree: &View<'f, 'a>) -> Option<NodeTree> {
        let f = self.f;
        let mut t = NodeTree::new(&tree.id_name(), TreeKind::Shader);
        if t.name.is_empty() {
            t.name = "Shader Nodetree".into();
        }
        let (ins, outs) = self.interface(tree);
        t.inputs = ins;
        t.outputs = outs;
        let file_nodes = list_with_ptrs(f, tree.sub("nodes"), &["bNode"]);
        let mut node_idx: HashMap<u64, usize> = HashMap::new();
        // Socket address -> (node, is_output, index).
        let mut socks: HashMap<u64, (usize, bool, usize)> = HashMap::new();
        let mut parents = Vec::new();
        for (ptr, bn) in &file_nodes {
            let i = t.nodes.len();
            node_idx.insert(*ptr, i);
            let node = self.node(bn, &t, &mut socks, i);
            parents.push(bn.ptr("parent"));
            t.nodes.push(node);
        }
        for (i, p) in parents.into_iter().enumerate() {
            t.nodes[i].parent = node_idx.get(&p).copied().filter(|&p| p != i);
        }
        for l in f.list(tree.sub("links"), &["bNodeLink"]) {
            let (Some(&(fnode, true, fs)), Some(&(tnode, false, ts))) = (socks.get(&l.ptr("fromsock")), socks.get(&l.ptr("tosock"))) else { continue };
            if fnode == tnode {
                continue;
            }
            let muted = l.num("flag").unwrap_or(0.0) as i64 & NODE_LINK_MUTED != 0;
            t.links.retain(|x| !(x.to_node == tnode && x.to_socket == ts && t.nodes[tnode].inputs[ts].ty != SocketType::Geometry));
            t.links.push(Link { from_node: fnode, from_socket: fs, to_node: tnode, to_socket: ts, muted });
        }
        t.active = None;
        Some(t)
    }

    fn node(&mut self, bn: &View<'f, 'a>, tree: &NodeTree, socks: &mut HashMap<u64, (usize, bool, usize)>, ni: usize) -> Node {
        let f = self.f;
        let idname = bn.string("idname").unwrap_or_default();
        let name = bn.string("name").unwrap_or_default();
        let known = new_node(&idname);
        let mut n = known.clone().unwrap_or_else(|| Node::new(&idname, &name));
        n.name = name;
        n.label = bn.string("label").unwrap_or_default();
        n.location = match (bn.f32("locx"), bn.f32("locy")) {
            (Some(x), Some(y)) => [x, y],
            _ => bn.vec::<2>("location").unwrap_or([0.0, 0.0]),
        };
        if let Some(w) = bn.f32("width").filter(|w| *w > 0.0) {
            n.width = w;
        }
        let flag = bn.num("flag").unwrap_or(0.0) as i64;
        n.mute = flag & NODE_MUTED != 0;
        n.hide = flag & NODE_HIDDEN != 0;
        if flag & NODE_CUSTOM_COLOR != 0 {
            n.color = bn.vec::<3>("color");
        }
        self.node_props(bn, &mut n, flag);
        // Group nodes take their sockets from the group's interface.
        if idname == "ShaderNodeGroup" {
            if let Some(Prop::Id(Some(g))) = n.prop("node_tree").cloned() {
                if let Some(gt) = self.scene.node_groups.get(g).cloned() {
                    sync_group_sockets(&mut n, &gt);
                }
            }
        } else if idname == "NodeGroupInput" || idname == "NodeGroupOutput" {
            sync_group_sockets(&mut n, tree);
        }
        update_sockets(&mut n);
        let alias = idname == "ShaderNodeBsdfPrincipled";
        for (is_out, lb) in [(false, "inputs"), (true, "outputs")] {
            let file = list_with_ptrs(f, bn.sub(lb), &["bNodeSocket"]);
            for (k, (sp, s)) in file.iter().enumerate() {
                let ident = s.string("identifier").unwrap_or_default();
                let sname = s.string("name").unwrap_or_default();
                let ty = sock_type(s.num("type").unwrap_or(0.0) as i64);
                let list = if is_out { &mut n.outputs } else { &mut n.inputs };
                let find = |id: &str| list.iter().position(|x| x.identifier == id);
                let mut idx = find(&ident).or_else(|| if alias { principled_alias(&ident).and_then(find) } else { None }).or_else(|| list.iter().position(|x| x.name == sname && x.ty == ty)).or_else(|| list.iter().position(|x| x.name == sname));
                if idx.is_none() && known.is_none() {
                    // Unknown node type: keep the file's sockets so links survive.
                    list.push(Socket::new(&sname, ty, Value::None).ident(&ident));
                    idx = Some(list.len() - 1);
                }
                let Some(idx) = idx.or_else(|| (known.is_none() && k < list.len()).then_some(k)) else { continue };
                let sock = &mut list[idx];
                if let Some(v) = socket_value(f, s.ptr("default_value")) {
                    sock.value = convert(v, sock.ty);
                }
                sock.hide = s.num("flag").unwrap_or(0.0) as i64 & SOCK_HIDDEN != 0;
                socks.insert(*sp, (ni, is_out, idx));
            }
        }
        n
    }

    fn node_props(&mut self, bn: &View<'f, 'a>, n: &mut Node, flag: i64) {
        let f = self.f;
        let c1 = bn.num("custom1").unwrap_or(0.0);
        let c2 = bn.num("custom2").unwrap_or(0.0) as i64;
        let st = bn.ptr("storage");
        let id = bn.ptr("id");
        let storage = |names: &[&str]| f.view_ptr(st, names);
        let idname = n.idname.clone();
        match idname.as_str() {
            "ShaderNodeOutputMaterial" | "ShaderNodeOutputWorld" | "ShaderNodeOutputLight" | "NodeGroupOutput" => {
                n.set_prop("is_active_output", Prop::Bool(flag & NODE_DO_OUTPUT != 0));
                if idname != "NodeGroupOutput" {
                    n.set_prop("target", pick(&["ALL", "EEVEE", "CYCLES"], c1));
                }
            }
            "ShaderNodeMath" => {
                n.set_prop("operation", e(math_from_file(c1 as i64)));
                n.set_prop("use_clamp", Prop::Bool(c2 & 1 != 0));
            }
            "ShaderNodeVectorMath" => n.set_prop("operation", e(vector_math_from_file(c1 as i64))),
            "ShaderNodeMix" => {
                if let Some(s) = storage(&["NodeShaderMix"]) {
                    n.set_prop("data_type", pick(&["FLOAT", "VECTOR", "RGBA"], s.num("data_type").unwrap_or(2.0)));
                    n.set_prop("factor_mode", pick(&["UNIFORM", "NON_UNIFORM"], s.num("factor_mode").unwrap_or(0.0)));
                    n.set_prop("clamp_factor", Prop::Bool(s.num("clamp_factor").unwrap_or(1.0) != 0.0));
                    n.set_prop("clamp_result", Prop::Bool(s.num("clamp_result").unwrap_or(0.0) != 0.0));
                    n.set_prop("blend_type", e(blend_from_file(s.num("blend_type").unwrap_or(0.0) as i64)));
                }
            }
            "ShaderNodeMixRGB" => {
                n.set_prop("blend_type", e(blend_from_file(c1 as i64)));
                n.set_prop("use_alpha", Prop::Bool(c2 & 1 != 0));
                n.set_prop("use_clamp", Prop::Bool(c2 & 2 != 0));
            }
            "ShaderNodeMapping" => n.set_prop("vector_type", pick(&["POINT", "TEXTURE", "VECTOR", "NORMAL"], c1)),
            "ShaderNodeClamp" => n.set_prop("clamp_type", pick(&["MINMAX", "RANGE"], c1)),
            "ShaderNodeMapRange" => {
                let s = storage(&["NodeMapRange"]);
                let dt = s.and_then(|s| s.num("data_type")).unwrap_or(0.0);
                n.set_prop("data_type", e(if dt == 1.0 { "FLOAT_VECTOR" } else { "FLOAT" }));
                n.set_prop("interpolation_type", pick(&["LINEAR", "STEPPED", "SMOOTHSTEP", "SMOOTHERSTEP"], s.and_then(|s| s.num("interpolation_type")).unwrap_or(0.0)));
                n.set_prop("clamp", Prop::Bool(c1 != 0.0));
            }
            "ShaderNodeVectorRotate" => {
                n.set_prop("rotation_type", pick(&["AXIS_ANGLE", "X_AXIS", "Y_AXIS", "Z_AXIS", "EULER_XYZ"], c1));
                n.set_prop("invert", Prop::Bool(c2 != 0));
            }
            "ShaderNodeVectorTransform" => {
                if let Some(s) = storage(&["NodeShaderVectTransform"]) {
                    n.set_prop("vector_type", pick(&["VECTOR", "POINT", "NORMAL"], s.num("type").unwrap_or(0.0)));
                    n.set_prop("convert_from", pick(&["WORLD", "OBJECT", "CAMERA"], s.num("convert_from").unwrap_or(0.0)));
                    n.set_prop("convert_to", pick(&["WORLD", "OBJECT", "CAMERA"], s.num("convert_to").unwrap_or(1.0)));
                }
            }
            "ShaderNodeCombineColor" | "ShaderNodeSeparateColor" => {
                if let Some(s) = storage(&["NodeCombSepColor"]) {
                    n.set_prop("mode", pick(&["RGB", "HSV", "HSL"], s.num("mode").unwrap_or(0.0)));
                }
            }
            "ShaderNodeBump" => n.set_prop("invert", Prop::Bool(c1 != 0.0)),
            "ShaderNodeDisplacement" => n.set_prop("space", pick(&["OBJECT", "WORLD"], c1)),
            "ShaderNodeVectorDisplacement" => n.set_prop("space", pick(&["TANGENT", "OBJECT", "WORLD"], c1)),
            "ShaderNodeNormalMap" => {
                if let Some(s) = storage(&["NodeShaderNormalMap"]) {
                    n.set_prop("space", pick(&["TANGENT", "OBJECT", "WORLD", "BLENDER_OBJECT", "BLENDER_WORLD"], s.num("space").unwrap_or(0.0)));
                    n.set_prop("uv_map", Prop::String(s.string("uv_map").unwrap_or_default()));
                }
            }
            "ShaderNodeTangent" => {
                if let Some(s) = storage(&["NodeShaderTangent"]) {
                    n.set_prop("direction_type", pick(&["RADIAL", "UV_MAP"], s.num("direction_type").unwrap_or(0.0)));
                    n.set_prop("axis", pick(&["X", "Y", "Z"], s.num("axis").unwrap_or(2.0)));
                    n.set_prop("uv_map", Prop::String(s.string("uv_map").unwrap_or_default()));
                }
            }
            "ShaderNodeAttribute" => {
                if let Some(s) = storage(&["NodeShaderAttribute"]) {
                    n.set_prop("attribute_name", Prop::String(s.string("name").unwrap_or_default()));
                    n.set_prop("attribute_type", pick(&["GEOMETRY", "OBJECT", "INSTANCER", "VIEW_LAYER"], s.num("type").unwrap_or(0.0)));
                }
            }
            "ShaderNodeUVMap" => {
                if let Some(s) = storage(&["NodeShaderUVMap"]) {
                    n.set_prop("uv_map", Prop::String(s.string("uv_map").unwrap_or_default()));
                }
            }
            "ShaderNodeBsdfPrincipled" => {
                n.set_prop("distribution", e(if c1 == 2.0 { "GGX" } else { "MULTI_GGX" }));
                n.set_prop("subsurface_method", pick(&["CUBIC", "GAUSSIAN", "BURLEY", "RANDOM_WALK_SKIN", "RANDOM_WALK", "RANDOM_WALK"], c2 as f64));
            }
            "ShaderNodeBsdfGlossy" | "ShaderNodeBsdfAnisotropic" | "ShaderNodeBsdfGlass" | "ShaderNodeBsdfRefraction" => n.set_prop("distribution", pick(&DISTRIBUTIONS, c1)),
            "ShaderNodeBsdfToon" => n.set_prop("component", pick(&["DIFFUSE", "GLOSSY"], c1)),
            "ShaderNodeBsdfSheen" => n.set_prop("distribution", pick(&["ASHIKHMIN", "MICROFIBER"], c1)),
            "ShaderNodeAmbientOcclusion" => {
                n.set_prop("samples", Prop::Int(c1.max(1.0) as i32));
                n.set_prop("inside", Prop::Bool(c2 & 1 != 0));
                n.set_prop("only_local", Prop::Bool(c2 & 2 != 0));
            }
            "ShaderNodeWireframe" => n.set_prop("use_pixel_size", Prop::Bool(c1 != 0.0)),
            "ShaderNodeTexWhiteNoise" => n.set_prop("noise_dimensions", dims_name(c1)),
            "ShaderNodeTexImage" => {
                if let Some(s) = storage(&["NodeTexImage"]) {
                    n.set_prop("projection", pick(&["FLAT", "BOX", "SPHERE", "TUBE"], s.num("projection").unwrap_or(0.0)));
                    n.set_prop("projection_blend", Prop::Float(s.f32("projection_blend").unwrap_or(0.0)));
                    n.set_prop("interpolation", pick(&["Linear", "Closest", "Cubic", "Smart"], s.num("interpolation").unwrap_or(0.0)));
                    n.set_prop("extension", pick(&["REPEAT", "EXTEND", "CLIP", "MIRROR"], s.num("extension").unwrap_or(0.0)));
                }
                n.set_prop("image", Prop::Id(self.image(id)));
            }
            "ShaderNodeTexEnvironment" => {
                if let Some(s) = storage(&["NodeTexEnvironment"]) {
                    n.set_prop("projection", pick(&["EQUIRECTANGULAR", "MIRROR_BALL"], s.num("projection").unwrap_or(0.0)));
                    n.set_prop("interpolation", pick(&["Linear", "Closest", "Cubic", "Smart"], s.num("interpolation").unwrap_or(0.0)));
                }
                n.set_prop("image", Prop::Id(self.image(id)));
            }
            "ShaderNodeTexNoise" => {
                if let Some(s) = storage(&["NodeTexNoise"]) {
                    n.set_prop("noise_dimensions", dims_name(s.num("dimensions").unwrap_or(3.0)));
                    n.set_prop("noise_type", pick(&NOISE_TYPES, s.num("type").unwrap_or(1.0)));
                    n.set_prop("normalize", Prop::Bool(s.num("normalize").unwrap_or(1.0) != 0.0));
                }
            }
            "ShaderNodeTexMusgrave" => {
                if let Some(s) = storage(&["NodeTexMusgrave"]) {
                    n.set_prop("musgrave_dimensions", dims_name(s.num("dimensions").unwrap_or(3.0)));
                    n.set_prop("musgrave_type", pick(&NOISE_TYPES, s.num("musgrave_type").unwrap_or(1.0)));
                }
            }
            "ShaderNodeTexVoronoi" => {
                if let Some(s) = storage(&["NodeTexVoronoi"]) {
                    n.set_prop("voronoi_dimensions", dims_name(s.num("dimensions").unwrap_or(3.0)));
                    n.set_prop("feature", pick(&["F1", "F2", "SMOOTH_F1", "DISTANCE_TO_EDGE", "N_SPHERE_RADIUS"], s.num("feature").unwrap_or(0.0)));
                    n.set_prop("distance", pick(&["EUCLIDEAN", "MANHATTAN", "CHEBYCHEV", "MINKOWSKI"], s.num("distance").unwrap_or(0.0)));
                    n.set_prop("normalize", Prop::Bool(s.num("normalize").unwrap_or(0.0) != 0.0));
                }
            }
            "ShaderNodeTexWave" => {
                if let Some(s) = storage(&["NodeTexWave"]) {
                    n.set_prop("wave_type", pick(&["BANDS", "RINGS"], s.num("wave_type").unwrap_or(0.0)));
                    n.set_prop("bands_direction", pick(&["X", "Y", "Z", "DIAGONAL"], s.num("bands_direction").unwrap_or(0.0)));
                    n.set_prop("rings_direction", pick(&["X", "Y", "Z", "SPHERICAL"], s.num("rings_direction").unwrap_or(0.0)));
                    n.set_prop("wave_profile", pick(&["SIN", "SAW", "TRI"], s.num("wave_profile").unwrap_or(0.0)));
                }
            }
            "ShaderNodeTexMagic" => {
                if let Some(s) = storage(&["NodeTexMagic"]) {
                    n.set_prop("turbulence_depth", Prop::Int(s.num("depth").unwrap_or(2.0) as i32));
                }
            }
            "ShaderNodeTexBrick" => {
                if let Some(s) = storage(&["NodeTexBrick"]) {
                    n.set_prop("offset", Prop::Float(s.f32("offset").unwrap_or(0.5)));
                    n.set_prop("offset_frequency", Prop::Int(s.num("offset_freq").unwrap_or(2.0) as i32));
                    n.set_prop("squash", Prop::Float(s.f32("squash").unwrap_or(1.0)));
                    n.set_prop("squash_frequency", Prop::Int(s.num("squash_freq").unwrap_or(2.0) as i32));
                }
            }
            "ShaderNodeTexGradient" => {
                if let Some(s) = storage(&["NodeTexGradient"]) {
                    n.set_prop("gradient_type", pick(&["LINEAR", "QUADRATIC", "EASING", "DIAGONAL", "SPHERICAL", "QUADRATIC_SPHERE", "RADIAL"], s.num("gradient_type").unwrap_or(0.0)));
                }
            }
            "ShaderNodeTexGabor" => {
                if let Some(s) = storage(&["NodeTexGabor"]) {
                    n.set_prop("gabor_type", pick(&["2D", "3D"], s.num("type").unwrap_or(0.0)));
                }
            }
            "ShaderNodeTexSky" => {
                if let Some(s) = storage(&["NodeTexSky"]) {
                    n.set_prop("sky_type", pick(&["PREETHAM", "HOSEK_WILKIE", "NISHITA"], s.num("sky_model").unwrap_or(2.0)));
                    if let Some(v) = s.vec::<3>("sun_direction") {
                        n.set_prop("sun_direction", Prop::Vector(v));
                    }
                    for (k, p) in [("turbidity", "turbidity"), ("ground_albedo", "ground_albedo"), ("sun_size", "sun_size"), ("sun_intensity", "sun_intensity"), ("sun_elevation", "sun_elevation"), ("sun_rotation", "sun_rotation"), ("altitude", "altitude"), ("air_density", "air_density"), ("dust_density", "dust_density"), ("ozone_density", "ozone_density")] {
                        if let Some(v) = s.f32(k) {
                            n.set_prop(p, Prop::Float(v));
                        }
                    }
                    n.set_prop("sun_disc", Prop::Bool(s.num("sun_disc").unwrap_or(1.0) != 0.0));
                }
            }
            "ShaderNodeValToRGB" => {
                if let Some(r) = color_ramp(f, st) {
                    n.set_prop("color_ramp", Prop::Ramp(r));
                }
            }
            "ShaderNodeRGBCurve" | "ShaderNodeVectorCurve" | "ShaderNodeFloatCurve" => {
                if let Some(mut c) = curve_maps(f, st) {
                    if idname == "ShaderNodeRGBCurve" && c.len() == 4 {
                        // File order R, G, B, C; ours C, R, G, B.
                        c.rotate_right(1);
                    } else if idname == "ShaderNodeVectorCurve" {
                        c.truncate(3);
                    } else if idname == "ShaderNodeFloatCurve" {
                        c.truncate(1);
                    }
                    n.set_prop("mapping", Prop::Curve(CurveMapping { curves: c }));
                }
            }
            "ShaderNodeGroup" => {
                let g = self.group(id);
                n.set_prop("node_tree", Prop::Id(g));
            }
            "ShaderNodeTexCoord" => {
                // Resolved once objects exist (see `resolve_object_refs`).
                n.set_prop("object", Prop::Id(None));
                n.set_prop("__object_ptr", Prop::String(format!("{id}")));
            }
            _ => {}
        }
    }

    /// Point Texture Coordinate nodes at their objects once those exist.
    pub(super) fn resolve_object_refs(&mut self, objects: &HashMap<u64, usize>) {
        let fix = |t: &mut NodeTree| {
            for n in &mut t.nodes {
                if let Some(Prop::String(p)) = n.prop("__object_ptr").cloned() {
                    let ptr: u64 = p.parse().unwrap_or(0);
                    n.props.retain(|(k, _)| k != "__object_ptr");
                    n.set_prop("object", Prop::Id(objects.get(&ptr).copied()));
                }
            }
        };
        for m in &mut self.scene.materials {
            if let Some(t) = &mut m.node_tree {
                fix(t);
            }
        }
        for g in &mut self.scene.node_groups {
            fix(g);
        }
        if let Some(t) = &mut self.scene.world.node_tree {
            fix(t);
        }
    }
}
