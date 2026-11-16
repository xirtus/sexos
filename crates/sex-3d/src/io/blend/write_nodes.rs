// SPDX-License-Identifier: GPL-3.0-only
//! Node trees for the `.blend` writer: `bNodeTree`, `bNode`, `bNodeSocket`
//! with their default values, `bNodeLink`, group interfaces
//! (`bNodeTreeInterfacePanel`/`bNodeTreeInterfaceSocket`) and node storage.
//! Settings go where the reader (`nodes.rs`, `compnodes.rs`) finds them:
//! `custom1..4` or the storage struct Blender allocates for the node type.
use super::export::{Ids, LIB_EMBEDDED_DATA};
use super::write::{Rec, V, W};
use crate::nodes::{ColorRamp, CurveMapping, Node, NodeTree, Prop, Socket, SocketType, Subtype, TreeKind, Value};
use crate::render::shader_names::{blend_from_file, math_from_file, vector_math_from_file};

// bNode.flag
const NODE_OPTIONS: i64 = 1 << 1;
const NODE_HIDDEN: i64 = 1 << 3;
const NODE_DO_OUTPUT: i64 = 1 << 6;
const NODE_MUTED: i64 = 1 << 9;
const NODE_CUSTOM_COLOR: i64 = 1 << 15;
const NODE_INIT: i64 = 1 << 16;
// bNodeSocket.flag
const SOCK_HIDDEN: i64 = 1 << 1;
const SOCK_UNAVAIL: i64 = 1 << 3;

/// `eNodeSocketDatatype`.
pub(super) fn sock_type(t: SocketType) -> i64 {
    match t {
        SocketType::Float => 0,
        SocketType::Vector => 1,
        SocketType::Color => 2,
        SocketType::Shader => 3,
        SocketType::Bool => 4,
        SocketType::Int => 6,
        SocketType::String => 7,
        SocketType::Object => 8,
        SocketType::Image => 9,
        SocketType::Geometry => 10,
        SocketType::Collection => 11,
        SocketType::Texture => 12,
        SocketType::Material => 13,
        SocketType::Rotation => 14,
        SocketType::Menu => 15,
        SocketType::Matrix => 16,
        SocketType::Virtual => -1,
    }
}

/// The socket type's idname (`NodeSocketFloatFactor`, ...).
fn socket_idname(t: SocketType, sub: Subtype) -> String {
    let base = match t {
        SocketType::Float => "NodeSocketFloat",
        SocketType::Vector => "NodeSocketVector",
        SocketType::Color => "NodeSocketColor",
        SocketType::Shader => "NodeSocketShader",
        SocketType::Bool => "NodeSocketBool",
        SocketType::Int => "NodeSocketInt",
        SocketType::String => "NodeSocketString",
        SocketType::Object => "NodeSocketObject",
        SocketType::Image => "NodeSocketImage",
        SocketType::Geometry => "NodeSocketGeometry",
        SocketType::Collection => "NodeSocketCollection",
        SocketType::Texture => "NodeSocketTexture",
        SocketType::Material => "NodeSocketMaterial",
        SocketType::Rotation => "NodeSocketRotation",
        SocketType::Menu => "NodeSocketMenu",
        SocketType::Matrix => "NodeSocketMatrix",
        SocketType::Virtual => "NodeSocketVirtual",
    };
    let suffix = match (t, sub) {
        (SocketType::Float, Subtype::Factor) => "Factor",
        (SocketType::Float, Subtype::Percentage) => "Percentage",
        (SocketType::Float, Subtype::Angle) => "Angle",
        (SocketType::Float, Subtype::Distance) => "Distance",
        (SocketType::Float, Subtype::Wavelength) => "Wavelength",
        (SocketType::Float, Subtype::ColorTemperature) => "ColorTemperature",
        (SocketType::Int, Subtype::Factor) => "Factor",
        (SocketType::Int, Subtype::Percentage) => "Percentage",
        (SocketType::Vector, Subtype::Direction) => "Direction",
        (SocketType::Vector, Subtype::Euler) => "Euler",
        (SocketType::Vector, Subtype::Xyz) => "XYZ",
        (SocketType::Vector, Subtype::Translation) => "Translation",
        (SocketType::String, Subtype::FilePath) => "FilePath",
        _ => "",
    };
    format!("{base}{suffix}")
}

fn tree_idname(k: TreeKind) -> (&'static str, i64) {
    match k {
        TreeKind::Shader => ("ShaderNodeTree", 0),
        TreeKind::Compositor => ("CompositorNodeTree", 1),
        TreeKind::Geometry => ("GeometryNodeTree", 3),
        #[allow(unreachable_patterns)]
        _ => ("ShaderNodeTree", 0),
    }
}

/// Position of `v` in `names`, else 0.
fn ix(names: &[&str], v: &str) -> i64 {
    names.iter().position(|n| *n == v).unwrap_or(0) as i64
}

/// Inverse of a `*_from_file` name table.
fn file_number(f: fn(i64) -> &'static str, name: &str) -> i64 {
    (0..256).find(|&k| f(k) == name).unwrap_or(0)
}

fn dims(s: &str) -> i64 {
    match s {
        "1D" => 1,
        "2D" => 2,
        "4D" => 4,
        _ => 3,
    }
}

fn pf(n: &Node, k: &str, d: f32) -> f32 {
    match n.prop(k) {
        Some(Prop::Float(v)) => *v,
        Some(Prop::Int(v)) => *v as f32,
        _ => d,
    }
}
fn pi(n: &Node, k: &str, d: i64) -> i64 {
    match n.prop(k) {
        Some(Prop::Int(v)) => *v as i64,
        Some(Prop::Float(v)) => *v as i64,
        Some(Prop::Bool(b)) => *b as i64,
        _ => d,
    }
}
fn pb(n: &Node, k: &str) -> bool {
    matches!(n.prop(k), Some(Prop::Bool(true)))
}
fn ps(n: &Node, k: &str) -> String {
    match n.prop(k) {
        Some(Prop::String(s)) | Some(Prop::Enum(s)) => s.clone(),
        _ => String::new(),
    }
}
fn pv(n: &Node, k: &str) -> Option<[f32; 4]> {
    match n.prop(k) {
        Some(Prop::Color(c)) => Some(*c),
        Some(Prop::Vector(v)) => Some([v[0], v[1], v[2], 0.0]),
        _ => None,
    }
}

const NOISE_TYPES: [&str; 5] = ["MULTIFRACTAL", "FBM", "HYBRID_MULTIFRACTAL", "RIDGED_MULTIFRACTAL", "HETERO_TERRAIN"];
const DISTRIBUTIONS: [&str; 5] = ["SHARP", "BECKMANN", "GGX", "ASHIKHMIN_SHIRLEY", "MULTI_GGX"];
const INTERP: [&str; 4] = ["Linear", "Closest", "Cubic", "Smart"];
const BLUR_FILTERS: [&str; 8] = ["FLAT", "TENT", "QUAD", "CUBIC", "CATROM", "GAUSS", "MITCH", "FAST_GAUSS"];
const GLARE: [&str; 5] = ["SIMPLE_STAR", "FOG_GLOW", "STREAKS", "GHOSTS", "BLOOM"];
const FILTERS: [&str; 8] = ["SOFTEN", "SHARPEN", "LAPLACE", "SOBEL", "PREWITT", "KIRSCH", "SHADOW", "SHARPEN_DIAMOND"];
const SAMPLERS: [&str; 3] = ["NEAREST", "BILINEAR", "BICUBIC"];

/// `SHD_SUBSURFACE_*`: BURLEY 3, RANDOM_WALK 4, RANDOM_WALK_SKIN 5.
fn sss_method(s: &str) -> i64 {
    match s {
        "BURLEY" => 3,
        "RANDOM_WALK_SKIN" => 5,
        _ => 4,
    }
}

/// `SHD_PRINCIPLED_HAIR_*` by value: REFLECTANCE 0, PIGMENT_CONCENTRATION 1, DIRECT_ABSORPTION 2.
pub(super) const HAIR_PARAMETRIZATION: [&str; 3] = ["COLOR", "MELANIN", "ABSORPTION"];

fn falloff(s: &str) -> i64 {
    match s {
        "SPHERE" => 1,
        "ROOT" => 2,
        "SHARP" => 3,
        "LINEAR" => 4,
        "INVERSE_SQUARE" => 7,
        _ => 0,
    }
}
/// RGB channel stored 1-based.
fn chan1(s: &str) -> i64 {
    ix(&["R", "G", "B"], s) + 1
}

/// What a node stores outside its sockets.
struct Settings {
    c: [f64; 4],
    /// The storage struct, filled.
    storage: Option<Rec>,
    /// A ColorBand or CurveMapping storage written separately.
    ramp: Option<ColorRamp>,
    curve: Option<(CurveMapping, usize, [f32; 4])>,
    /// `bNode.id` (an image, object or node group address).
    id: u64,
}

impl W {
    /// Write a node tree at `addr` (`code` "NT" for a group ID, "DATA" for
    /// an embedded tree).
    pub(super) fn node_tree_at(&mut self, addr: u64, code: &[u8], t: &NodeTree, default_name: &str, embedded: bool, ids: &Ids) {
        let (idname, ty) = tree_idname(t.kind);
        let mut r = self.rec("bNodeTree");
        let name = if embedded || t.name.is_empty() { default_name } else { &t.name };
        self.s(&mut r, "id.name", &format!("NT{name}"));
        if embedded {
            self.i(&mut r, "id.flag", LIB_EMBEDDED_DATA);
        } else {
            self.i(&mut r, "id.us", 1);
        }
        self.s(&mut r, "idname", idname);
        self.i(&mut r, "type", ty);
        let node_addrs: Vec<u64> = t.nodes.iter().map(|_| self.addr()).collect();
        let link_addrs: Vec<u64> = t.links.iter().map(|_| self.addr()).collect();
        self.listbase(&mut r, "nodes", &node_addrs);
        self.listbase(&mut r, "links", &link_addrs);
        // Group interface: `Socket_<i>` identifiers, as group nodes use.
        let iface: Vec<(usize, bool)> = (0..t.inputs.len()).map(|i| (i, false)).chain((0..t.outputs.len()).map(|i| (i, true))).collect();
        let items: Vec<u64> = iface.iter().map(|_| self.addr()).collect();
        let arr = if items.is_empty() { 0 } else { self.addr() };
        self.p(&mut r, "tree_interface.root_panel.items_array", arr);
        self.i(&mut r, "tree_interface.root_panel.items_num", items.len() as i64);
        self.i(&mut r, "tree_interface.next_uid", items.len() as i64);
        self.put(code, addr, &[r]);

        if arr != 0 {
            self.put_raw_at(arr, items.iter().flat_map(|p| p.to_le_bytes()).collect());
            for (k, &(i, out)) in iface.iter().enumerate() {
                let s = if out { &t.outputs[i] } else { &t.inputs[i] };
                let mut it = self.rec("bNodeTreeInterfaceSocket");
                self.i(&mut it, "item.item_type", 1); // NODE_INTERFACE_SOCKET
                let nm = self.string(&s.name);
                self.p(&mut it, "name", nm);
                let st = self.string(&socket_idname(s.ty, Subtype::None));
                self.p(&mut it, "socket_type", st);
                self.i(&mut it, "flag", if out { 2 } else { 1 });
                let id = self.string(&format!("Socket_{k}"));
                self.p(&mut it, "identifier", id);
                let sock = Socket { min: s.min, max: s.max, ..Socket::new(&s.name, s.ty, s.default.clone()) };
                let dv = self.default_value(&sock);
                self.p(&mut it, "socket_data", dv);
                self.put(b"DATA", items[k], &[it]);
            }
        }

        // Socket addresses first, so links can point at them.
        let socks: Vec<(Vec<u64>, Vec<u64>)> = t.nodes.iter().map(|n| (n.inputs.iter().map(|_| self.addr()).collect(), n.outputs.iter().map(|_| self.addr()).collect())).collect();
        let mut first_output = std::collections::HashSet::new();
        for (ni, n) in t.nodes.iter().enumerate() {
            let set = self.settings(n, &t.nodes, ids);
            let mut b = self.rec("bNode");
            self.p(&mut b, "next", node_addrs.get(ni + 1).copied().unwrap_or(0));
            self.p(&mut b, "prev", if ni > 0 { node_addrs[ni - 1] } else { 0 });
            self.listbase(&mut b, "inputs", &socks[ni].0);
            self.listbase(&mut b, "outputs", &socks[ni].1);
            self.s(&mut b, "name", &n.name);
            self.i(&mut b, "identifier", ni as i64 + 1);
            let mut flag = NODE_OPTIONS | NODE_INIT;
            if n.hide {
                flag |= NODE_HIDDEN;
            }
            if n.mute {
                flag |= NODE_MUTED;
            }
            if n.color.is_some() {
                flag |= NODE_CUSTOM_COLOR;
            }
            let output = match n.idname.as_str() {
                "ShaderNodeOutputMaterial" | "ShaderNodeOutputWorld" | "ShaderNodeOutputLight" | "NodeGroupOutput" => !matches!(n.prop("is_active_output"), Some(Prop::Bool(false))),
                "CompositorNodeComposite" | "CompositorNodeViewer" => first_output.insert(n.idname.clone()),
                _ => false,
            };
            if output {
                flag |= NODE_DO_OUTPUT;
            }
            self.i(&mut b, "flag", flag);
            self.s(&mut b, "idname", &n.idname);
            self.s(&mut b, "label", &n.label);
            self.f(&mut b, "locx", &[n.location[0]]);
            self.f(&mut b, "locy", &[n.location[1]]);
            self.f(&mut b, "width", &[n.width]);
            self.f(&mut b, "height", &[100.0]);
            if let Some(c) = n.color {
                self.f(&mut b, "color", &c);
            }
            self.p(&mut b, "parent", n.parent.and_then(|p| node_addrs.get(p).copied()).unwrap_or(0));
            self.set(&mut b, "custom1", V::I(&[set.c[0] as i64]));
            self.set(&mut b, "custom2", V::I(&[set.c[1] as i64]));
            self.f(&mut b, "custom3", &[set.c[2] as f32]);
            self.f(&mut b, "custom4", &[set.c[3] as f32]);
            self.p(&mut b, "id", set.id);
            let st = if let Some(s) = set.storage {
                self.data(&[s])
            } else if let Some(r) = &set.ramp {
                self.color_band(r)
            } else if let Some((c, n, clip)) = &set.curve {
                self.curve_mapping(c, *n, *clip)
            } else {
                0
            };
            self.p(&mut b, "storage", st);
            self.put(b"DATA", node_addrs[ni], &[b]);
            // Sockets.
            for (out, list, addrs) in [(false, &n.inputs, &socks[ni].0), (true, &n.outputs, &socks[ni].1)] {
                let recs: Vec<Rec> = list
                    .iter()
                    .map(|s| {
                        let mut r = self.rec("bNodeSocket");
                        self.s(&mut r, "identifier", &s.identifier);
                        self.s(&mut r, "name", &s.name);
                        self.s(&mut r, "idname", &socket_idname(s.ty, s.subtype));
                        self.i(&mut r, "type", sock_type(s.ty));
                        let mut f = 0;
                        if s.hide {
                            f |= SOCK_HIDDEN;
                        }
                        if !s.enabled {
                            f |= SOCK_UNAVAIL;
                        }
                        self.i(&mut r, "flag", f);
                        self.i(&mut r, "in_out", if out { 2 } else { 1 });
                        self.i(&mut r, "limit", if out { 4095 } else { 1 });
                        let dv = if out && !matches!(s.ty, SocketType::Float | SocketType::Vector | SocketType::Color | SocketType::Int | SocketType::Bool) { 0 } else { self.default_value(s) };
                        self.p(&mut r, "default_value", dv);
                        // File Output slot paths live in socket storage.
                        if !out && n.idname == "CompositorNodeOutputFile" {
                            let mut m = self.rec("NodeImageMultiFileSocket");
                            self.s(&mut m, "path", &s.name);
                            self.i(&mut m, "use_node_format", 1);
                            let a = self.data(&[m]);
                            self.p(&mut r, "storage", a);
                        }
                        r
                    })
                    .collect();
                self.chain(recs, addrs);
            }
        }
        // Links.
        let recs: Vec<Rec> = t
            .links
            .iter()
            .map(|l| {
                let mut r = self.rec("bNodeLink");
                self.p(&mut r, "fromnode", node_addrs.get(l.from_node).copied().unwrap_or(0));
                self.p(&mut r, "tonode", node_addrs.get(l.to_node).copied().unwrap_or(0));
                self.p(&mut r, "fromsock", socks.get(l.from_node).and_then(|s| s.1.get(l.from_socket)).copied().unwrap_or(0));
                self.p(&mut r, "tosock", socks.get(l.to_node).and_then(|s| s.0.get(l.to_socket)).copied().unwrap_or(0));
                self.i(&mut r, "flag", 1 << 1 | if l.muted { 1 << 4 } else { 0 });
                r
            })
            .collect();
        self.chain(recs, &link_addrs);
    }

    /// The socket's `default_value` block (0 for types without one).
    fn default_value(&mut self, s: &Socket) -> u64 {
        let lim = |x: f32, d: f32| if x.is_finite() && x.abs() < 3.0e38 { x } else { d };
        let r = match s.ty {
            SocketType::Float => {
                let mut r = self.rec("bNodeSocketValueFloat");
                self.f(&mut r, "value", &[s.value.as_float()]);
                self.f(&mut r, "min", &[lim(s.min, -f32::MAX)]);
                self.f(&mut r, "max", &[lim(s.max, f32::MAX)]);
                r
            }
            SocketType::Int => {
                let mut r = self.rec("bNodeSocketValueInt");
                self.i(&mut r, "value", s.value.as_float() as i64);
                self.i(&mut r, "min", lim(s.min, i32::MIN as f32).max(i32::MIN as f32) as i64);
                self.i(&mut r, "max", lim(s.max, i32::MAX as f32).min(i32::MAX as f32) as i64);
                r
            }
            SocketType::Bool => {
                let mut r = self.rec("bNodeSocketValueBoolean");
                self.i(&mut r, "value", (s.value.as_float() != 0.0) as i64);
                r
            }
            SocketType::Vector => {
                let mut r = self.rec("bNodeSocketValueVector");
                self.f(&mut r, "value", &s.value.as_vector());
                self.f(&mut r, "min", &[lim(s.min, -f32::MAX)]);
                self.f(&mut r, "max", &[lim(s.max, f32::MAX)]);
                r
            }
            SocketType::Color => {
                let mut r = self.rec("bNodeSocketValueRGBA");
                self.f(&mut r, "value", &s.value.as_color());
                r
            }
            SocketType::Rotation => {
                let mut r = self.rec("bNodeSocketValueRotation");
                self.f(&mut r, "value_euler", &s.value.as_vector());
                r
            }
            SocketType::String => {
                let mut r = self.rec("bNodeSocketValueString");
                if let Value::String(v) = &s.value {
                    self.s(&mut r, "value", v);
                }
                r
            }
            _ => return 0,
        };
        self.data(&[r])
    }

    fn color_band(&mut self, r: &ColorRamp) -> u64 {
        let mut b = self.rec("ColorBand");
        let n = r.elements.len().min(32);
        self.i(&mut b, "tot", n as i64);
        self.i(&mut b, "ipotype", ix(&["LINEAR", "EASE", "B_SPLINE", "CARDINAL", "CONSTANT"], &r.interpolation));
        self.i(&mut b, "color_mode", ix(&["RGB", "HSV", "HSL"], &r.color_mode));
        for (k, (pos, c)) in r.elements.iter().take(32).enumerate() {
            let mut d = self.rec("CBData");
            self.f(&mut d, "r", &[c[0]]);
            self.f(&mut d, "g", &[c[1]]);
            self.f(&mut d, "b", &[c[2]]);
            self.f(&mut d, "a", &[c[3]]);
            self.f(&mut d, "pos", &[*pos]);
            self.i(&mut d, "cur", k as i64);
            self.put_sub(&mut b, "data", k, &d);
        }
        self.data(&[b])
    }

    /// `CurveMapping` with `n` curves in file order and clip rect `clip`
    /// (xmin, ymin, xmax, ymax).
    pub(super) fn curve_mapping(&mut self, c: &CurveMapping, n: usize, clip: [f32; 4]) -> u64 {
        let mut m = self.rec("CurveMapping");
        self.i(&mut m, "flag", 1); // CUMA_DO_CLIP
        for (k, v) in ["xmin", "ymin", "xmax", "ymax"].iter().zip(clip) {
            self.f(&mut m, &format!("curr.{k}"), &[v]);
            self.f(&mut m, &format!("clipr.{k}"), &[v]);
        }
        self.f(&mut m, "white", &[1.0, 1.0, 1.0]);
        self.f(&mut m, "bwmul", &[1.0, 1.0, 1.0]);
        let mut point_blocks = Vec::new();
        for k in 0..n.min(4) {
            let pts = c.curves.get(k).cloned().unwrap_or_else(|| vec![[clip[0], clip[1]], [clip[2], clip[3]]]);
            let a = self.addr();
            let mut cm = self.rec("CurveMap");
            self.i(&mut cm, "totpoint", pts.len() as i64);
            self.p(&mut cm, "curve", a);
            self.f(&mut cm, "mintable", &[clip[0]]);
            self.f(&mut cm, "maxtable", &[clip[2]]);
            self.put_sub(&mut m, "cm", k, &cm);
            point_blocks.push((a, pts));
        }
        let addr = self.data(&[m]);
        for (a, pts) in point_blocks {
            let recs: Vec<Rec> = pts
                .iter()
                .map(|p| {
                    let mut r = self.rec("CurveMapPoint");
                    self.f(&mut r, "x", &[p[0]]);
                    self.f(&mut r, "y", &[p[1]]);
                    r
                })
                .collect();
            self.put(b"DATA", a, &recs);
        }
        addr
    }

    /// A storage struct with `TexMapping`/`ColorMapping` defaults where it
    /// embeds `NodeTexBase`.
    fn storage_rec(&mut self, name: &str) -> Rec {
        let mut r = self.rec(name);
        if self.dna.sdna.structs[r.s].members.contains_key("base") {
            let id = [1.0, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];
            self.f(&mut r, "base.tex_mapping.size", &[1.0, 1.0, 1.0]);
            self.f(&mut r, "base.tex_mapping.mat", &id);
            self.f(&mut r, "base.tex_mapping.max", &[1.0, 1.0, 1.0]);
            self.i(&mut r, "base.tex_mapping.projx", 1);
            self.i(&mut r, "base.tex_mapping.projy", 2);
            self.i(&mut r, "base.tex_mapping.projz", 3);
            self.f(&mut r, "base.color_mapping.bright", &[1.0]);
            self.f(&mut r, "base.color_mapping.contrast", &[1.0]);
            self.f(&mut r, "base.color_mapping.saturation", &[1.0]);
            self.f(&mut r, "base.color_mapping.blend_color", &[0.8, 0.8, 0.8]);
        }
        if self.dna.sdna.structs[r.s].members.contains_key("iuser") {
            self.i(&mut r, "iuser.frames", 100);
            self.i(&mut r, "iuser.sfra", 1);
        }
        r
    }

    fn settings(&mut self, n: &Node, nodes: &[Node], ids: &Ids) -> Settings {
        let mut s = Settings { c: [0.0; 4], storage: None, ramp: None, curve: None, id: 0 };
        let e = |k: &str| n.enum_prop(k).to_string();
        let image = |k: &str| match n.prop(k) {
            Some(Prop::Id(Some(i))) => ids.images.get(*i).copied().unwrap_or(0),
            _ => 0,
        };
        let id = n.idname.as_str();
        match id {
            "ShaderNodeOutputMaterial" | "ShaderNodeOutputWorld" | "ShaderNodeOutputLight" => s.c[0] = ix(&["ALL", "EEVEE", "CYCLES"], &e("target")) as f64,
            "ShaderNodeMath" | "CompositorNodeMath" => {
                s.c[0] = file_number(math_from_file, &e("operation")) as f64;
                s.c[1] = pb(n, "use_clamp") as i64 as f64;
            }
            "ShaderNodeVectorMath" => s.c[0] = file_number(vector_math_from_file, &e("operation")) as f64,
            "ShaderNodeMix" => {
                let mut r = self.storage_rec("NodeShaderMix");
                // eNodeSocketDatatype: FLOAT 0, VECTOR 1, RGBA 2, ROTATION 14.
                self.i(&mut r, "data_type", if e("data_type") == "ROTATION" { 14 } else { ix(&["FLOAT", "VECTOR", "RGBA"], &e("data_type")) });
                self.i(&mut r, "factor_mode", ix(&["UNIFORM", "NON_UNIFORM"], &e("factor_mode")));
                self.i(&mut r, "clamp_factor", pb(n, "clamp_factor") as i64);
                self.i(&mut r, "clamp_result", pb(n, "clamp_result") as i64);
                self.i(&mut r, "blend_type", file_number(blend_from_file, &e("blend_type")));
                s.storage = Some(r);
            }
            "ShaderNodeMixRGB" | "CompositorNodeMixRGB" => {
                s.c[0] = file_number(blend_from_file, &e("blend_type")) as f64;
                s.c[1] = (pb(n, "use_alpha") as i64 | (pb(n, "use_clamp") as i64) << 1) as f64;
            }
            "ShaderNodeMapping" => s.c[0] = ix(&["POINT", "TEXTURE", "VECTOR", "NORMAL"], &e("vector_type")) as f64,
            "ShaderNodeClamp" => s.c[0] = ix(&["MINMAX", "RANGE"], &e("clamp_type")) as f64,
            "ShaderNodeMapRange" => {
                let mut r = self.storage_rec("NodeMapRange");
                self.i(&mut r, "data_type", (e("data_type") == "FLOAT_VECTOR") as i64);
                self.i(&mut r, "interpolation_type", ix(&["LINEAR", "STEPPED", "SMOOTHSTEP", "SMOOTHERSTEP"], &e("interpolation_type")));
                self.i(&mut r, "clamp", pb(n, "clamp") as i64);
                s.storage = Some(r);
                s.c[0] = pb(n, "clamp") as i64 as f64;
            }
            "ShaderNodeVectorRotate" => {
                s.c[0] = ix(&["AXIS_ANGLE", "X_AXIS", "Y_AXIS", "Z_AXIS", "EULER_XYZ"], &e("rotation_type")) as f64;
                s.c[1] = pb(n, "invert") as i64 as f64;
            }
            "ShaderNodeVectorTransform" => {
                let mut r = self.storage_rec("NodeShaderVectTransform");
                self.i(&mut r, "type", ix(&["VECTOR", "POINT", "NORMAL"], &e("vector_type")));
                self.i(&mut r, "convert_from", ix(&["WORLD", "OBJECT", "CAMERA"], &e("convert_from")));
                self.i(&mut r, "convert_to", ix(&["WORLD", "OBJECT", "CAMERA"], &e("convert_to")));
                s.storage = Some(r);
            }
            "ShaderNodeCombineColor" | "ShaderNodeSeparateColor" => {
                let mut r = self.storage_rec("NodeCombSepColor");
                self.i(&mut r, "mode", ix(&["RGB", "HSV", "HSL"], &e("mode")));
                s.storage = Some(r);
            }
            "ShaderNodeBump" => s.c[0] = pb(n, "invert") as i64 as f64,
            "ShaderNodeDisplacement" => s.c[0] = ix(&["OBJECT", "WORLD"], &e("space")) as f64,
            "ShaderNodeVectorDisplacement" => s.c[0] = ix(&["TANGENT", "OBJECT", "WORLD"], &e("space")) as f64,
            "ShaderNodeNormalMap" => {
                let mut r = self.storage_rec("NodeShaderNormalMap");
                self.i(&mut r, "space", ix(&["TANGENT", "OBJECT", "WORLD", "BLENDER_OBJECT", "BLENDER_WORLD"], &e("space")));
                self.s(&mut r, "uv_map", &ps(n, "uv_map"));
                s.storage = Some(r);
            }
            "ShaderNodeTangent" => {
                let mut r = self.storage_rec("NodeShaderTangent");
                self.i(&mut r, "direction_type", ix(&["RADIAL", "UV_MAP"], &e("direction_type")));
                self.i(&mut r, "axis", if n.prop("axis").is_some() { ix(&["X", "Y", "Z"], &e("axis")) } else { 2 });
                self.s(&mut r, "uv_map", &ps(n, "uv_map"));
                s.storage = Some(r);
            }
            "ShaderNodeAttribute" => {
                let mut r = self.storage_rec("NodeShaderAttribute");
                self.s(&mut r, "name", &ps(n, "attribute_name"));
                self.i(&mut r, "type", ix(&["GEOMETRY", "OBJECT", "INSTANCER", "VIEW_LAYER"], &e("attribute_type")));
                s.storage = Some(r);
            }
            "ShaderNodeUVMap" => {
                let mut r = self.storage_rec("NodeShaderUVMap");
                self.s(&mut r, "uv_map", &ps(n, "uv_map"));
                s.storage = Some(r);
            }
            "ShaderNodeVertexColor" => {
                let mut r = self.storage_rec("NodeShaderVertexColor");
                self.s(&mut r, "layer_name", &ps(n, "layer_name"));
                s.storage = Some(r);
            }
            "ShaderNodeOutputAOV" => {
                let mut r = self.storage_rec("NodeShaderOutputAOV");
                self.s(&mut r, "name", &ps(n, "aov_name"));
                s.storage = Some(r);
            }
            "ShaderNodeTexIES" => s.storage = Some(self.storage_rec("NodeShaderTexIES")),
            "ShaderNodeScript" => s.storage = Some(self.storage_rec("NodeShaderScript")),
            "ShaderNodeTexPointDensity" => s.storage = Some(self.storage_rec("NodeShaderTexPointDensity")),
            "ShaderNodeBsdfPrincipled" => {
                s.c[0] = if e("distribution") == "GGX" { 2.0 } else { 5.0 };
                s.c[1] = sss_method(&e("subsurface_method")) as f64;
            }
            "ShaderNodeBsdfGlossy" | "ShaderNodeBsdfAnisotropic" | "ShaderNodeBsdfGlass" | "ShaderNodeBsdfRefraction" => s.c[0] = ix(&DISTRIBUTIONS, &e("distribution")) as f64,
            "ShaderNodeBsdfToon" => s.c[0] = ix(&["DIFFUSE", "GLOSSY"], &e("component")) as f64,
            "ShaderNodeBsdfHair" => s.c[0] = ix(&["Reflection", "Transmission"], &e("component")) as f64,
            "ShaderNodeSubsurfaceScattering" => s.c[0] = sss_method(&e("falloff")) as f64,
            "ShaderNodeBsdfHairPrincipled" => {
                let mut r = self.storage_rec("NodeShaderHairPrincipled");
                self.i(&mut r, "model", ix(&["CHIANG", "HUANG"], &e("model")));
                self.i(&mut r, "parametrization", ix(&HAIR_PARAMETRIZATION, &e("parametrization")));
                s.storage = Some(r);
            }
            "ShaderNodeBsdfSheen" => s.c[0] = ix(&["ASHIKHMIN", "MICROFIBER"], &e("distribution")) as f64,
            "ShaderNodeAmbientOcclusion" => {
                s.c[0] = pi(n, "samples", 16).max(1) as f64;
                s.c[1] = (pb(n, "inside") as i64 | (pb(n, "only_local") as i64) << 1) as f64;
            }
            "ShaderNodeWireframe" => s.c[0] = pb(n, "use_pixel_size") as i64 as f64,
            "ShaderNodeTexWhiteNoise" => s.c[0] = dims(&e("noise_dimensions")) as f64,
            "ShaderNodeTexImage" => {
                let mut r = self.storage_rec("NodeTexImage");
                self.i(&mut r, "projection", ix(&["FLAT", "BOX", "SPHERE", "TUBE"], &e("projection")));
                self.f(&mut r, "projection_blend", &[pf(n, "projection_blend", 0.0)]);
                self.i(&mut r, "interpolation", ix(&INTERP, &e("interpolation")));
                self.i(&mut r, "extension", ix(&["REPEAT", "EXTEND", "CLIP", "MIRROR"], &e("extension")));
                s.storage = Some(r);
                s.id = image("image");
            }
            "ShaderNodeTexEnvironment" => {
                let mut r = self.storage_rec("NodeTexEnvironment");
                self.i(&mut r, "projection", ix(&["EQUIRECTANGULAR", "MIRROR_BALL"], &e("projection")));
                self.i(&mut r, "interpolation", ix(&INTERP, &e("interpolation")));
                s.storage = Some(r);
                s.id = image("image");
            }
            "ShaderNodeTexNoise" => {
                let mut r = self.storage_rec("NodeTexNoise");
                self.i(&mut r, "dimensions", dims(&e("noise_dimensions")));
                self.i(&mut r, "type", if n.prop("noise_type").is_some() { ix(&NOISE_TYPES, &e("noise_type")) } else { 1 });
                self.i(&mut r, "normalize", if n.prop("normalize").is_some() { pb(n, "normalize") as i64 } else { 1 });
                s.storage = Some(r);
            }
            "ShaderNodeTexVoronoi" => {
                let mut r = self.storage_rec("NodeTexVoronoi");
                self.i(&mut r, "dimensions", dims(&e("voronoi_dimensions")));
                self.i(&mut r, "feature", ix(&["F1", "F2", "SMOOTH_F1", "DISTANCE_TO_EDGE", "N_SPHERE_RADIUS"], &e("feature")));
                self.i(&mut r, "distance", ix(&["EUCLIDEAN", "MANHATTAN", "CHEBYCHEV", "MINKOWSKI"], &e("distance")));
                self.i(&mut r, "normalize", pb(n, "normalize") as i64);
                s.storage = Some(r);
            }
            "ShaderNodeTexWave" => {
                let mut r = self.storage_rec("NodeTexWave");
                self.i(&mut r, "wave_type", ix(&["BANDS", "RINGS"], &e("wave_type")));
                self.i(&mut r, "bands_direction", ix(&["X", "Y", "Z", "DIAGONAL"], &e("bands_direction")));
                self.i(&mut r, "rings_direction", ix(&["X", "Y", "Z", "SPHERICAL"], &e("rings_direction")));
                self.i(&mut r, "wave_profile", ix(&["SIN", "SAW", "TRI"], &e("wave_profile")));
                s.storage = Some(r);
            }
            "ShaderNodeTexMagic" => {
                let mut r = self.storage_rec("NodeTexMagic");
                self.i(&mut r, "depth", pi(n, "turbulence_depth", 2));
                s.storage = Some(r);
            }
            "ShaderNodeTexBrick" => {
                let mut r = self.storage_rec("NodeTexBrick");
                self.f(&mut r, "offset", &[pf(n, "offset", 0.5)]);
                self.i(&mut r, "offset_freq", pi(n, "offset_frequency", 2));
                self.f(&mut r, "squash", &[pf(n, "squash", 1.0)]);
                self.i(&mut r, "squash_freq", pi(n, "squash_frequency", 2));
                s.storage = Some(r);
            }
            "ShaderNodeTexGradient" => {
                let mut r = self.storage_rec("NodeTexGradient");
                self.i(&mut r, "gradient_type", ix(&["LINEAR", "QUADRATIC", "EASING", "DIAGONAL", "SPHERICAL", "QUADRATIC_SPHERE", "RADIAL"], &e("gradient_type")));
                s.storage = Some(r);
            }
            "ShaderNodeTexChecker" => s.storage = Some(self.storage_rec("NodeTexChecker")),
            "ShaderNodeTexSky" => {
                let mut r = self.storage_rec("NodeTexSky");
                self.i(&mut r, "sky_model", if n.prop("sky_type").is_some() { ix(&["PREETHAM", "HOSEK_WILKIE", "NISHITA"], &e("sky_type")) } else { 2 });
                if let Some(v) = pv(n, "sun_direction") {
                    self.f(&mut r, "sun_direction", &v[..3]);
                }
                for (k, d) in [("turbidity", 2.2), ("ground_albedo", 0.3), ("sun_size", 0.009512), ("sun_intensity", 1.0), ("sun_elevation", 0.2618), ("sun_rotation", 0.0), ("altitude", 0.0), ("air_density", 1.0), ("dust_density", 1.0), ("ozone_density", 1.0)] {
                    self.f(&mut r, k, &[pf(n, k, d)]);
                }
                self.i(&mut r, "sun_disc", if n.prop("sun_disc").is_some() { pb(n, "sun_disc") as i64 } else { 1 });
                s.storage = Some(r);
            }
            "ShaderNodeValToRGB" | "CompositorNodeValToRGB" => {
                s.ramp = Some(match n.prop("color_ramp") {
                    Some(Prop::Ramp(r)) => r.clone(),
                    _ => ColorRamp::default(),
                });
            }
            "ShaderNodeRGBCurve" | "CompositorNodeCurveRGB" => {
                if let Some(Prop::Curve(c)) = n.prop("mapping") {
                    let mut c = c.clone();
                    if c.curves.len() == 4 {
                        // Ours C, R, G, B; the file R, G, B, C.
                        c.curves.rotate_left(1);
                    }
                    s.curve = Some((c, 4, [0.0, 0.0, 1.0, 1.0]));
                }
            }
            "ShaderNodeVectorCurve" | "CompositorNodeCurveVec" => {
                if let Some(Prop::Curve(c)) = n.prop("mapping") {
                    s.curve = Some((c.clone(), 3, [-1.0, -1.0, 1.0, 1.0]));
                }
            }
            "ShaderNodeFloatCurve" => {
                if let Some(Prop::Curve(c)) = n.prop("mapping") {
                    s.curve = Some((c.clone(), 1, [0.0, 0.0, 1.0, 1.0]));
                }
            }
            "ShaderNodeGroup" | "CompositorNodeGroup" | "GeometryNodeGroup" => {
                if let Some(Prop::Id(Some(g))) = n.prop("node_tree") {
                    s.id = ids.groups.get(*g).copied().unwrap_or(0);
                }
            }
            "ShaderNodeTexCoord" => {
                if let Some(Prop::Id(Some(o))) = n.prop("object") {
                    s.id = ids.objects.get(*o).copied().unwrap_or(0);
                }
            }
            "NodeFrame" => {
                let mut r = self.storage_rec("NodeFrame");
                self.i(&mut r, "flag", 2); // NODE_FRAME_RESIZEABLE
                self.i(&mut r, "label_size", 20);
                s.storage = Some(r);
            }
            _ if id.starts_with("CompositorNode") => self.comp_settings(n, ids, &mut s),
            _ if id.starts_with("GeometryNode") || id.starts_with("FunctionNode") => {
                let (st, c) = self.geo_settings(n, nodes);
                s.storage = st;
                s.c[0] = c[0] as f64;
                s.c[1] = c[1] as f64;
            }
            _ => {}
        }
        s
    }

    /// Compositor node settings (the inverse of `compnodes.rs`).
    fn comp_settings(&mut self, n: &Node, ids: &Ids, s: &mut Settings) {
        let e = |k: &str| n.enum_prop(k).to_string();
        let b = |k: &str| pb(n, k) as i64;
        let id = n.idname.clone();
        let st: Option<Rec> = match id.as_str() {
            "CompositorNodeImage" => {
                if let Some(Prop::Id(Some(i))) = n.prop("image") {
                    s.id = ids.images.get(*i).copied().unwrap_or(0);
                }
                s.c[0] = b("use_straight_alpha_output") as f64;
                let mut r = self.storage_rec("ImageUser");
                self.i(&mut r, "frames", pi(n, "frame_duration", 1));
                self.i(&mut r, "sfra", pi(n, "frame_start", 1));
                self.i(&mut r, "offset", pi(n, "frame_offset", 0));
                Some(r)
            }
            "CompositorNodeViewer" => {
                s.c[1] = (!pb(n, "use_alpha")) as i64 as f64;
                let mut r = self.storage_rec("ImageUser");
                self.i(&mut r, "frames", 100);
                self.i(&mut r, "sfra", 1);
                Some(r)
            }
            "CompositorNodeComposite" => {
                s.c[1] = (!pb(n, "use_alpha")) as i64 as f64;
                None
            }
            "CompositorNodeMask" => {
                let fixed = match e("size_source").as_str() {
                    "FIXED" => 2,
                    "FIXED_SCENE" => 4,
                    _ => 0,
                };
                s.c[0] = ((!pb(n, "use_feather")) as i64 | fixed | b("use_motion_blur") << 3) as f64;
                s.c[1] = pi(n, "motion_blur_samples", 16) as f64;
                s.c[2] = pf(n, "motion_blur_shutter", 0.5) as f64;
                let mut r = self.storage_rec("NodeMask");
                self.i(&mut r, "size_x", pi(n, "size_x", 256));
                self.i(&mut r, "size_y", pi(n, "size_y", 256));
                Some(r)
            }
            "CompositorNodeBokehImage" => {
                let mut r = self.storage_rec("NodeBokehImage");
                self.f(&mut r, "angle", &[pf(n, "angle", 0.0)]);
                self.i(&mut r, "flaps", pi(n, "flaps", 5));
                self.f(&mut r, "rounding", &[pf(n, "rounding", 0.0)]);
                self.f(&mut r, "catadioptric", &[pf(n, "catadioptric", 0.0)]);
                self.f(&mut r, "lensshift", &[pf(n, "shift", 0.0)]);
                Some(r)
            }
            "CompositorNodeTime" => {
                s.c[0] = pi(n, "frame_start", 1) as f64;
                s.c[1] = pi(n, "frame_end", 250) as f64;
                if let Some(Prop::Curve(c)) = n.prop("curve") {
                    s.curve = Some((c.clone(), 1, [0.0, 0.0, 1.0, 1.0]));
                }
                None
            }
            "CompositorNodeOutputFile" => {
                let mut r = self.storage_rec("NodeImageMultiFile");
                self.s(&mut r, "base_path", &ps(n, "base_path"));
                self.i(&mut r, "format.imtype", 17); // R_IMF_IMTYPE_PNG
                // R_IMF_PLANES_*: BW 8, RGB 24, RGBA 32; R_IMF_CHAN_DEPTH_8 1 << 1, _16 1 << 4.
                self.i(&mut r, "format.planes", match e("color_mode").as_str() {
                    "BW" => 8,
                    "RGB" => 24,
                    _ => 32,
                });
                self.i(&mut r, "format.depth", if e("color_depth") == "16" { 1 << 4 } else { 1 << 1 });
                self.i(&mut r, "format.compress", 15);
                self.i(&mut r, "format.quality", 90);
                Some(r)
            }
            "CompositorNodeAlphaOver" => {
                s.c[0] = b("use_premultiply") as f64;
                let mut r = self.storage_rec("NodeTwoFloats");
                self.f(&mut r, "x", &[pf(n, "premul", 0.0)]);
                Some(r)
            }
            "CompositorNodeBrightContrast" => {
                s.c[0] = b("use_premultiply") as f64;
                None
            }
            "CompositorNodeColorBalance" => {
                s.c[0] = ix(&["LIFT_GAMMA_GAIN", "OFFSET_POWER_SLOPE"], &e("correction_method")) as f64;
                let mut r = self.storage_rec("NodeColorBalance");
                for (k, d) in [("lift", 1.0), ("gamma", 1.0), ("gain", 1.0), ("offset", 0.0), ("power", 1.0), ("slope", 1.0)] {
                    let v = pv(n, k).unwrap_or([d; 4]);
                    self.f(&mut r, k, &v[..3]);
                }
                self.f(&mut r, "offset_basis", &[pf(n, "offset_basis", 0.0)]);
                Some(r)
            }
            "CompositorNodeColorCorrection" => {
                s.c[0] = (b("red") | b("green") << 1 | b("blue") << 2) as f64;
                let mut r = self.storage_rec("NodeColorCorrection");
                self.f(&mut r, "startmidtones", &[pf(n, "midtones_start", 0.2)]);
                self.f(&mut r, "endmidtones", &[pf(n, "midtones_end", 0.7)]);
                for g in ["master", "highlights", "midtones", "shadows"] {
                    for (k, d) in [("saturation", 1.0), ("contrast", 1.0), ("gamma", 1.0), ("gain", 1.0), ("lift", 0.0)] {
                        self.f(&mut r, &format!("{g}.{k}"), &[pf(n, &format!("{g}_{k}"), d)]);
                    }
                }
                Some(r)
            }
            "CompositorNodeHueCorrect" => {
                if let Some(Prop::Curve(c)) = n.prop("mapping") {
                    s.curve = Some((c.clone(), 3, [0.0, 0.0, 1.0, 1.0]));
                }
                None
            }
            "CompositorNodeInvert" => {
                s.c[0] = (b("invert_rgb") | b("invert_alpha") << 1) as f64;
                None
            }
            "CompositorNodeTonemap" => {
                let mut r = self.storage_rec("NodeTonemap");
                self.i(&mut r, "type", (e("tonemap_type") != "RH_SIMPLE") as i64);
                for (p, k, d) in [("key", "key", 0.18), ("offset", "offset", 1.0), ("gamma", "gamma", 1.0), ("intensity", "f", 0.0), ("contrast", "m", 0.0), ("adaptation", "a", 1.0), ("correction", "c", 0.0)] {
                    self.f(&mut r, k, &[pf(n, p, d)]);
                }
                Some(r)
            }
            "CompositorNodeZcombine" => {
                s.c[0] = b("use_alpha") as f64;
                s.c[1] = (!pb(n, "use_antialias_z")) as i64 as f64;
                None
            }
            "CompositorNodeConvertColorSpace" => {
                let mut r = self.storage_rec("NodeConvertColorSpace");
                self.s(&mut r, "from_color_space", &e("from_color_space"));
                self.s(&mut r, "to_color_space", &e("to_color_space"));
                Some(r)
            }
            "CompositorNodePremulKey" => {
                s.c[0] = ix(&["STRAIGHT_TO_PREMUL", "PREMUL_TO_STRAIGHT"], &e("mapping")) as f64;
                None
            }
            "CompositorNodeSetAlpha" => {
                let mut r = self.storage_rec("NodeSetAlpha");
                self.i(&mut r, "mode", ix(&["APPLY", "REPLACE_ALPHA"], &e("mode")));
                Some(r)
            }
            "CompositorNodeAntiAliasing" => {
                let mut r = self.storage_rec("NodeAntiAliasingData");
                for (k, d) in [("threshold", 1.0), ("contrast_limit", 0.2), ("corner_rounding", 0.25)] {
                    self.f(&mut r, k, &[pf(n, k, d)]);
                }
                Some(r)
            }
            "CompositorNodeBilateralblur" => {
                let mut r = self.storage_rec("NodeBilateralBlurData");
                self.i(&mut r, "iter", pi(n, "iterations", 1));
                self.f(&mut r, "sigma_color", &[pf(n, "sigma_color", 0.3)]);
                self.f(&mut r, "sigma_space", &[pf(n, "sigma_space", 5.0)]);
                Some(r)
            }
            "CompositorNodeBlur" => {
                s.c[0] = (b("use_variable_size") | b("use_extended_bounds") << 1) as f64;
                let mut r = self.storage_rec("NodeBlurData");
                self.i(&mut r, "filtertype", ix(&BLUR_FILTERS, &e("filter_type")));
                self.i(&mut r, "sizex", pi(n, "size_x", 0));
                self.i(&mut r, "sizey", pi(n, "size_y", 0));
                self.i(&mut r, "relative", b("use_relative"));
                self.i(&mut r, "aspect", ix(&["NONE", "Y", "X"], &e("aspect_correction")));
                self.f(&mut r, "fac", &[pf(n, "factor", 0.0)]);
                self.f(&mut r, "percentx", &[pf(n, "factor_x", 0.0)]);
                self.f(&mut r, "percenty", &[pf(n, "factor_y", 0.0)]);
                self.i(&mut r, "bokeh", b("use_bokeh"));
                self.i(&mut r, "gamma", b("use_gamma_correction"));
                Some(r)
            }
            "CompositorNodeBokehBlur" => {
                s.c[0] = (b("use_variable_size") | b("use_extended_bounds") << 1) as f64;
                s.c[2] = pf(n, "f_stop", 4.0) as f64;
                s.c[3] = pf(n, "blur_max", 16.0) as f64;
                None
            }
            "CompositorNodeDefocus" => {
                let mut r = self.storage_rec("NodeDefocus");
                let bk = match e("bokeh").as_str() {
                    "TRIANGLE" => 3,
                    "SQUARE" => 4,
                    "PENTAGON" => 5,
                    "HEXAGON" => 6,
                    "HEPTAGON" => 7,
                    "OCTAGON" => 8,
                    _ => 0,
                };
                self.i(&mut r, "bktype", bk);
                self.f(&mut r, "rotation", &[pf(n, "angle", 0.0)]);
                self.i(&mut r, "gamco", b("use_gamma_correction"));
                self.f(&mut r, "fstop", &[pf(n, "f_stop", 128.0)]);
                self.f(&mut r, "maxblur", &[pf(n, "blur_max", 16.0)]);
                self.f(&mut r, "bthresh", &[pf(n, "threshold", 1.0)]);
                self.i(&mut r, "preview", b("use_preview"));
                self.i(&mut r, "no_zbuf", (!pb(n, "use_zbuffer")) as i64);
                self.f(&mut r, "scale", &[pf(n, "z_scale", 1.0)]);
                Some(r)
            }
            "CompositorNodeDenoise" => {
                let mut r = self.storage_rec("NodeDenoise");
                self.i(&mut r, "hdr", b("use_hdr"));
                self.i(&mut r, "prefilter", ix(&["FAST", "NONE", "ACCURATE"], &e("prefilter")));
                Some(r)
            }
            "CompositorNodeDespeckle" => {
                s.c[2] = pf(n, "threshold", 0.5) as f64;
                s.c[3] = pf(n, "threshold_neighbor", 0.5) as f64;
                None
            }
            "CompositorNodeDilateErode" => {
                s.c[0] = ix(&["STEP", "THRESHOLD", "DISTANCE", "FEATHER"], &e("mode")) as f64;
                s.c[1] = pi(n, "distance", 0) as f64;
                s.c[2] = pf(n, "edge", 0.0) as f64;
                let mut r = self.storage_rec("NodeDilateErode");
                self.i(&mut r, "falloff", falloff(&e("falloff")));
                Some(r)
            }
            "CompositorNodeDBlur" => {
                let mut r = self.storage_rec("NodeDBlurData");
                self.i(&mut r, "iter", pi(n, "iterations", 1));
                for (k, d) in [("center_x", 0.5), ("center_y", 0.5), ("distance", 0.0), ("angle", 0.0), ("spin", 0.0), ("zoom", 0.0)] {
                    self.f(&mut r, k, &[pf(n, k, d)]);
                }
                Some(r)
            }
            "CompositorNodeFilter" => {
                s.c[0] = ix(&FILTERS, &e("filter_type")) as f64;
                None
            }
            "CompositorNodeGlare" => {
                let mut r = self.storage_rec("NodeGlare");
                self.i(&mut r, "type", ix(&GLARE, &e("glare_type")));
                self.i(&mut r, "quality", ix(&["HIGH", "MEDIUM", "LOW"], &e("quality")));
                self.i(&mut r, "iter", pi(n, "iterations", 3));
                self.f(&mut r, "colmod", &[pf(n, "color_modulation", 0.25)]);
                self.f(&mut r, "mix", &[pf(n, "mix", 0.0)]);
                self.f(&mut r, "threshold", &[pf(n, "threshold", 1.0)]);
                self.i(&mut r, "streaks", pi(n, "streaks", 4));
                self.f(&mut r, "angle_ofs", &[pf(n, "angle_offset", 0.0)]);
                self.f(&mut r, "fade", &[pf(n, "fade", 0.9)]);
                self.i(&mut r, "star_45", b("use_rotate_45"));
                self.i(&mut r, "size", pi(n, "size", 8));
                Some(r)
            }
            "CompositorNodeInpaint" => {
                s.c[1] = pi(n, "distance", 0) as f64;
                None
            }
            "CompositorNodePixelate" => {
                s.c[0] = pi(n, "pixel_size", 1) as f64;
                None
            }
            "CompositorNodeSunBeams" => {
                let mut r = self.storage_rec("NodeSunBeams");
                let src = pv(n, "source").unwrap_or([0.5, 0.5, 0.0, 0.0]);
                self.f(&mut r, "source", &src[..2]);
                self.f(&mut r, "ray_length", &[pf(n, "ray_length", 0.0)]);
                Some(r)
            }
            "CompositorNodeVecBlur" => {
                let mut r = self.storage_rec("NodeBlurData");
                self.i(&mut r, "samples", pi(n, "samples", 32));
                self.i(&mut r, "minspeed", pi(n, "speed_min", 0));
                self.i(&mut r, "maxspeed", pi(n, "speed_max", 0));
                self.f(&mut r, "fac", &[pf(n, "factor", 0.25)]);
                self.i(&mut r, "curved", b("use_curved"));
                Some(r)
            }
            "CompositorNodeKuwahara" => {
                let mut r = self.storage_rec("NodeKuwaharaData");
                self.i(&mut r, "variation", ix(&["CLASSIC", "ANISOTROPIC"], &e("variation")));
                self.i(&mut r, "high_precision", b("use_high_precision"));
                self.i(&mut r, "uniformity", pi(n, "uniformity", 4));
                self.f(&mut r, "sharpness", &[pf(n, "sharpness", 0.5)]);
                self.f(&mut r, "eccentricity", &[pf(n, "eccentricity", 1.0)]);
                Some(r)
            }
            "CompositorNodeChannelMatte" => {
                s.c[0] = (ix(&["RGB", "HSV", "YUV", "YCC"], &e("color_space")) + 1) as f64;
                s.c[1] = chan1(&e("matte_channel")) as f64;
                let mut r = self.storage_rec("NodeChroma");
                self.i(&mut r, "algorithm", ix(&["SINGLE", "MAX"], &e("limit_method")));
                self.i(&mut r, "channel", chan1(&e("limit_channel")));
                self.f(&mut r, "t1", &[pf(n, "limit_max", 1.0)]);
                self.f(&mut r, "t2", &[pf(n, "limit_min", 0.0)]);
                Some(r)
            }
            "CompositorNodeChromaMatte" => {
                let mut r = self.storage_rec("NodeChroma");
                self.f(&mut r, "t1", &[pf(n, "tolerance", 0.5236)]);
                self.f(&mut r, "t2", &[pf(n, "threshold", 0.1745)]);
                self.f(&mut r, "t3", &[pf(n, "shadow_adjust", 0.0)]);
                self.f(&mut r, "fsize", &[pf(n, "lift", 0.0)]);
                self.f(&mut r, "fstrength", &[pf(n, "gain", 1.0)]);
                Some(r)
            }
            "CompositorNodeColorMatte" => {
                let mut r = self.storage_rec("NodeChroma");
                self.f(&mut r, "t1", &[pf(n, "color_hue", 0.01)]);
                self.f(&mut r, "t2", &[pf(n, "color_saturation", 0.1)]);
                self.f(&mut r, "t3", &[pf(n, "color_value", 0.1)]);
                Some(r)
            }
            "CompositorNodeColorSpill" => {
                s.c[0] = chan1(&e("channel")) as f64;
                s.c[1] = ix(&["SIMPLE", "AVERAGE"], &e("limit_method")) as f64;
                let mut r = self.storage_rec("NodeColorspill");
                self.i(&mut r, "limchan", ix(&["R", "G", "B"], &e("limit_channel")));
                self.f(&mut r, "limscale", &[pf(n, "ratio", 1.0)]);
                self.i(&mut r, "unspill", b("use_unspill"));
                self.f(&mut r, "uspillr", &[pf(n, "unspill_red", 0.0)]);
                self.f(&mut r, "uspillg", &[pf(n, "unspill_green", 0.0)]);
                self.f(&mut r, "uspillb", &[pf(n, "unspill_blue", 0.0)]);
                Some(r)
            }
            "CompositorNodeDiffMatte" | "CompositorNodeDistanceMatte" => {
                let mut r = self.storage_rec("NodeChroma");
                self.f(&mut r, "t1", &[pf(n, "tolerance", 0.1)]);
                self.f(&mut r, "t2", &[pf(n, "falloff", 0.1)]);
                if id == "CompositorNodeDistanceMatte" {
                    self.i(&mut r, "channel", if e("channel") == "YCC" { 2 } else { 1 });
                }
                Some(r)
            }
            "CompositorNodeLumaMatte" => {
                let mut r = self.storage_rec("NodeChroma");
                self.f(&mut r, "t1", &[pf(n, "limit_max", 1.0)]);
                self.f(&mut r, "t2", &[pf(n, "limit_min", 0.0)]);
                Some(r)
            }
            "CompositorNodeKeying" => {
                let mut r = self.storage_rec("NodeKeyingData");
                for (k, d) in [("screen_balance", 0.5), ("despill_factor", 1.0), ("despill_balance", 0.5), ("edge_kernel_tolerance", 0.1), ("clip_black", 0.0), ("clip_white", 1.0)] {
                    self.f(&mut r, k, &[pf(n, k, d)]);
                }
                for (k, d) in [("edge_kernel_radius", 3), ("dilate_distance", 0), ("feather_distance", 0), ("blur_pre", 0), ("blur_post", 0)] {
                    self.i(&mut r, k, pi(n, k, d));
                }
                self.i(&mut r, "feather_falloff", falloff(&e("feather_falloff")));
                Some(r)
            }
            "CompositorNodeKeyingScreen" => {
                let mut r = self.storage_rec("NodeKeyingScreenData");
                self.f(&mut r, "smoothness", &[pf(n, "smoothness", 0.0)]);
                Some(r)
            }
            "CompositorNodeCryptomatteV2" => Some(self.storage_rec("NodeCryptomatte")),
            "CompositorNodeBoxMask" | "CompositorNodeEllipseMask" => {
                s.c[0] = ix(&["ADD", "SUBTRACT", "MULTIPLY", "NOT"], &e("mask_type")) as f64;
                let mut r = self.storage_rec(if id == "CompositorNodeBoxMask" { "NodeBoxMask" } else { "NodeEllipseMask" });
                self.f(&mut r, "x", &[pf(n, "x", 0.5)]);
                self.f(&mut r, "y", &[pf(n, "y", 0.5)]);
                self.f(&mut r, "width", &[pf(n, "mask_width", 0.2)]);
                self.f(&mut r, "height", &[pf(n, "mask_height", 0.1)]);
                self.f(&mut r, "rotation", &[pf(n, "rotation", 0.0)]);
                Some(r)
            }
            "CompositorNodeDoubleEdgeMask" => {
                s.c[0] = ix(&["ALL", "ADJACENT_ONLY"], &e("inner_mode")) as f64;
                s.c[1] = ix(&["BLEED_OUT", "KEEP_IN"], &e("edge_mode")) as f64;
                None
            }
            "CompositorNodeIDMask" => {
                s.c[0] = pi(n, "index", 0) as f64;
                s.c[1] = b("use_antialiasing") as f64;
                None
            }
            "CompositorNodeRotate" | "CompositorNodeTransform" => {
                s.c[0] = ix(&SAMPLERS, &e("filter_type")) as f64;
                None
            }
            "CompositorNodeStabilize" => {
                s.c[0] = ix(&SAMPLERS, &e("filter_type")) as f64;
                s.c[1] = b("invert") as f64;
                None
            }
            "CompositorNodeMovieDistortion" => {
                s.c[0] = ix(&["UNDISTORT", "DISTORT"], &e("distortion_type")) as f64;
                None
            }
            "CompositorNodeScale" => {
                s.c[0] = ix(&["RELATIVE", "ABSOLUTE", "SCENE_SIZE", "RENDER_SIZE"], &e("space")) as f64;
                s.c[1] = match e("frame_method").as_str() {
                    "FIT" => 1.0,
                    "CROP" => 2.0,
                    _ => 0.0,
                };
                s.c[2] = pf(n, "offset_x", 0.0) as f64;
                s.c[3] = pf(n, "offset_y", 0.0) as f64;
                None
            }
            "CompositorNodeTranslate" => {
                let mut r = self.storage_rec("NodeTranslateData");
                self.i(&mut r, "interpolation", ix(&["Nearest", "Bilinear", "Bicubic"], &e("interpolation")));
                self.i(&mut r, "relative", b("use_relative"));
                self.i(&mut r, "wrap_axis", ix(&["NONE", "XAXIS", "YAXIS", "BOTH"], &e("wrap_axis")));
                Some(r)
            }
            "CompositorNodeFlip" => {
                s.c[0] = ix(&["X", "Y", "XY"], &e("axis")) as f64;
                None
            }
            "CompositorNodeCrop" => {
                s.c[0] = b("use_crop_size") as f64;
                s.c[1] = b("relative") as f64;
                let mut r = self.storage_rec("NodeTwoXYs");
                for (p, k) in [("min_x", "x1"), ("max_x", "x2"), ("min_y", "y1"), ("max_y", "y2")] {
                    self.i(&mut r, k, pi(n, p, 0));
                }
                for (p, k) in [("rel_min_x", "fac_x1"), ("rel_max_x", "fac_x2"), ("rel_min_y", "fac_y1"), ("rel_max_y", "fac_y2")] {
                    self.f(&mut r, k, &[pf(n, p, 0.0)]);
                }
                Some(r)
            }
            "CompositorNodeMapUV" => {
                s.c[0] = pi(n, "alpha", 0) as f64;
                s.c[1] = if e("filter_type") == "NEAREST" { 0.0 } else { 3.0 };
                None
            }
            "CompositorNodeLensdist" => {
                let mut r = self.storage_rec("NodeLensDist");
                self.i(&mut r, "proj", b("use_projector"));
                self.i(&mut r, "jit", b("use_jitter"));
                self.i(&mut r, "fit", b("use_fit"));
                Some(r)
            }
            "CompositorNodePlaneTrackDeform" => Some(self.storage_rec("NodePlaneTrackDeformData")),
            "CompositorNodeMapRange" => {
                s.c[0] = b("use_clamp") as f64;
                None
            }
            "CompositorNodeMapValue" => {
                let mut r = self.storage_rec("TexMapping");
                let flag = b("use_min") | b("use_max") << 1;
                self.f(&mut r, "loc", &[pf(n, "offset", 0.0)]);
                self.f(&mut r, "size", &[pf(n, "size", 1.0)]);
                self.i(&mut r, "flag", flag);
                self.f(&mut r, "min", &[pf(n, "min", 0.0)]);
                self.f(&mut r, "max", &[pf(n, "max", 1.0)]);
                let id = [1.0, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];
                self.f(&mut r, "mat", &id);
                Some(r)
            }
            "CompositorNodeSwitch" => {
                s.c[0] = b("check") as f64;
                None
            }
            "CompositorNodeLevels" => {
                s.c[0] = match e("channel").as_str() {
                    "RED" => 2.0,
                    "GREEN" => 3.0,
                    "BLUE" => 4.0,
                    "LUMINANCE" => 5.0,
                    _ => 1.0,
                };
                None
            }
            "CompositorNodeSplit" => {
                s.c[0] = pi(n, "factor", 50) as f64;
                s.c[1] = ix(&["X", "Y"], &e("axis")) as f64;
                None
            }
            "CompositorNodeSeparateColor" | "CompositorNodeCombineColor" => {
                let mut r = self.storage_rec("NodeCMPCombSepColor");
                self.i(&mut r, "mode", ix(&["RGB", "HSV", "HSL", "YCC", "YUV"], &e("mode")));
                self.i(&mut r, "ycc_mode", if n.prop("ycc_mode").is_some() { ix(&["ITUBT601", "ITUBT709", "JFIF"], &e("ycc_mode")) } else { 1 });
                Some(r)
            }
            _ => None,
        };
        if st.is_some() {
            s.storage = st;
        }
    }
}
