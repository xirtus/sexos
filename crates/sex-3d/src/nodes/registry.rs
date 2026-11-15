// SPDX-License-Identifier: GPL-3.0-only
//! Node types with Blender's `bl_idname`, labels, Add-menu categories,
//! sockets and default values (Blender 4.x).
use super::{Node, Prop, Socket, SocketType as T, Subtype as S, TreeKind, Value as V};

/// An Add-menu (Shift+A) category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Input,
    Output,
    Shader,
    Displacement,
    Texture,
    Color,
    Vector,
    Converter,
    Script,
    Group,
    Layout,
    // Compositor
    Filter,
    Matte,
    Distort,
    Transform,
    // Geometry nodes
    Attribute,
    Geometry,
    Curve,
    Instances,
    Mesh,
    Point,
    Volume,
    SimulationZone,
    Material,
    Utilities,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Input => "Input",
            Category::Output => "Output",
            Category::Shader => "Shader",
            Category::Displacement => "Displacement",
            Category::Texture => "Texture",
            Category::Color => "Color",
            Category::Vector => "Vector",
            Category::Converter => "Converter",
            Category::Script => "Script",
            Category::Group => "Group",
            Category::Layout => "Layout",
            Category::Filter => "Filter",
            Category::Matte => "Matte",
            Category::Distort => "Distort",
            Category::Transform => "Transform",
            Category::Attribute => "Attribute",
            Category::Geometry => "Geometry",
            Category::Curve => "Curve",
            Category::Instances => "Instances",
            Category::Mesh => "Mesh",
            Category::Point => "Point",
            Category::Volume => "Volume",
            Category::SimulationZone => "Simulation",
            Category::Material => "Material",
            Category::Utilities => "Utilities",
        }
    }

    /// Header colour (sRGB) of nodes in this category, as Blender's theme.
    pub fn header_color(self) -> u32 {
        match self {
            Category::Input => 0x83314a,
            Category::Output => 0x6a1e29,
            Category::Shader => 0x2b652b,
            Category::Texture => 0x79461d,
            Category::Color => 0x6c696f,
            Category::Vector => 0x3c3c83,
            Category::Converter => 0x246283,
            Category::Group => 0x3b5959,
            Category::Layout => 0x6c696f,
            Category::Filter => 0x6c4a8b,
            Category::Matte => 0x973c3c,
            Category::Distort => 0x4c9797,
            Category::Geometry | Category::Mesh | Category::Curve | Category::Point | Category::Instances | Category::Volume => 0x00a379,
            Category::Attribute => 0x001566,
            _ => 0x3b3b3b,
        }
    }
}

#[derive(Clone, Copy)]
pub struct NodeType {
    pub idname: &'static str,
    pub label: &'static str,
    pub tree: TreeKind,
    pub category: Category,
    pub build: fn() -> Node,
}

fn n(idname: &str, label: &str, width: f32) -> Node {
    let mut node = Node::new(idname, label);
    node.width = width;
    node
}

fn f(name: &str, v: f32) -> Socket {
    Socket::new(name, T::Float, V::Float(v))
}
fn fac(name: &str, v: f32) -> Socket {
    f(name, v).subtype(S::Factor)
}
fn col(name: &str, c: [f32; 4]) -> Socket {
    Socket::new(name, T::Color, V::Color(c))
}
fn vec3(name: &str, v: [f32; 3]) -> Socket {
    Socket::new(name, T::Vector, V::Vector(v))
}
fn shader(name: &str) -> Socket {
    Socket::new(name, T::Shader, V::None)
}
fn normal() -> Socket {
    vec3("Normal", [0.0; 3]).hide_value()
}
fn out_f(name: &str) -> Socket {
    Socket::new(name, T::Float, V::Float(0.0))
}
fn out_c(name: &str) -> Socket {
    Socket::new(name, T::Color, V::Color([0.0, 0.0, 0.0, 1.0]))
}
fn out_v(name: &str) -> Socket {
    Socket::new(name, T::Vector, V::Vector([0.0; 3]))
}
fn e(s: &str) -> Prop {
    Prop::Enum(s.into())
}

fn principled() -> Node {
    let mut x = n("ShaderNodeBsdfPrincipled", "Principled BSDF", 240.0);
    x.props = vec![("distribution".into(), e("MULTI_GGX")), ("subsurface_method".into(), e("RANDOM_WALK"))];
    x.inputs = vec![
        col("Base Color", [0.8, 0.8, 0.8, 1.0]),
        fac("Metallic", 0.0),
        fac("Roughness", 0.5),
        f("IOR", 1.5).range(1.0, 1000.0),
        fac("Alpha", 1.0),
        normal(),
        fac("Weight", 0.0).ident("Subsurface Weight").panel("Subsurface"),
        vec3("Radius", [1.0, 0.2, 0.1]).ident("Subsurface Radius").panel("Subsurface"),
        f("Scale", 0.05).range(0.0, f32::MAX).subtype(S::Distance).ident("Subsurface Scale").panel("Subsurface"),
        f("IOR", 1.4).range(1.01, 3.8).ident("Subsurface IOR").panel("Subsurface"),
        fac("Anisotropy", 0.0).ident("Subsurface Anisotropy").panel("Subsurface"),
        f("IOR Level", 0.5).range(0.0, 1.0).subtype(S::Factor).ident("Specular IOR Level").panel("Specular"),
        col("Tint", [1.0; 4]).ident("Specular Tint").panel("Specular"),
        fac("Anisotropic", 0.0).panel("Specular"),
        fac("Anisotropic Rotation", 0.0).panel("Specular"),
        vec3("Tangent", [0.0; 3]).hide_value().panel("Specular"),
        fac("Weight", 0.0).ident("Transmission Weight").panel("Transmission"),
        fac("Weight", 0.0).ident("Coat Weight").panel("Coat"),
        fac("Roughness", 0.03).ident("Coat Roughness").panel("Coat"),
        f("IOR", 1.5).range(1.0, 4.0).ident("Coat IOR").panel("Coat"),
        col("Tint", [1.0; 4]).ident("Coat Tint").panel("Coat"),
        vec3("Normal", [0.0; 3]).hide_value().ident("Coat Normal").panel("Coat"),
        fac("Weight", 0.0).ident("Sheen Weight").panel("Sheen"),
        fac("Roughness", 0.5).ident("Sheen Roughness").panel("Sheen"),
        col("Tint", [1.0; 4]).ident("Sheen Tint").panel("Sheen"),
        col("Color", [1.0; 4]).ident("Emission Color").panel("Emission"),
        f("Strength", 0.0).range(0.0, f32::MAX).ident("Emission Strength").panel("Emission"),
        f("Thickness", 0.0).range(0.0, 100000.0).subtype(S::Wavelength).ident("Thin Film Thickness").panel("Thin Film"),
        f("IOR", 1.33).range(1.0, 1000.0).ident("Thin Film IOR").panel("Thin Film"),
    ];
    x.outputs = vec![shader("BSDF")];
    x
}

fn output_material() -> Node {
    let mut x = n("ShaderNodeOutputMaterial", "Material Output", 140.0);
    x.props = vec![("target".into(), e("ALL")), ("is_active_output".into(), Prop::Bool(true))];
    x.inputs = vec![shader("Surface"), shader("Volume"), vec3("Displacement", [0.0; 3]).hide_value(), f("Thickness", 0.0).hide_value()];
    x
}

fn output_world() -> Node {
    let mut x = n("ShaderNodeOutputWorld", "World Output", 140.0);
    x.props = vec![("target".into(), e("ALL")), ("is_active_output".into(), Prop::Bool(true))];
    x.inputs = vec![shader("Surface"), shader("Volume")];
    x
}

fn background() -> Node {
    let mut x = n("ShaderNodeBackground", "Background", 140.0);
    x.inputs = vec![col("Color", [0.050876, 0.050876, 0.050876, 1.0]), f("Strength", 1.0).range(0.0, 1_000_000.0)];
    x.outputs = vec![shader("Background")];
    x
}

fn emission() -> Node {
    let mut x = n("ShaderNodeEmission", "Emission", 140.0);
    x.inputs = vec![col("Color", [1.0; 4]), f("Strength", 1.0).range(0.0, 1_000_000.0)];
    x.outputs = vec![shader("Emission")];
    x
}

fn diffuse() -> Node {
    let mut x = n("ShaderNodeBsdfDiffuse", "Diffuse BSDF", 150.0);
    x.inputs = vec![col("Color", [0.8, 0.8, 0.8, 1.0]), fac("Roughness", 0.0), normal()];
    x.outputs = vec![shader("BSDF")];
    x
}

fn glossy() -> Node {
    let mut x = n("ShaderNodeBsdfAnisotropic", "Glossy BSDF", 150.0);
    x.props = vec![("distribution".into(), e("MULTI_GGX"))];
    x.inputs = vec![col("Color", [0.8, 0.8, 0.8, 1.0]), fac("Roughness", 0.5), fac("Anisotropy", 0.0), fac("Rotation", 0.0), normal(), vec3("Tangent", [0.0; 3]).hide_value()];
    x.outputs = vec![shader("BSDF")];
    x
}

fn mix_shader() -> Node {
    let mut x = n("ShaderNodeMixShader", "Mix Shader", 140.0);
    x.inputs = vec![fac("Fac", 0.5), shader("Shader").ident("Shader"), shader("Shader").ident("Shader_001")];
    x.outputs = vec![shader("Shader")];
    x
}

fn add_shader() -> Node {
    let mut x = n("ShaderNodeAddShader", "Add Shader", 140.0);
    x.inputs = vec![shader("Shader").ident("Shader"), shader("Shader").ident("Shader_001")];
    x.outputs = vec![shader("Shader")];
    x
}

fn tex_image() -> Node {
    let mut x = n("ShaderNodeTexImage", "Image Texture", 240.0);
    x.props = vec![
        ("image".into(), Prop::Id(None)),
        ("interpolation".into(), e("Linear")),
        ("projection".into(), e("FLAT")),
        ("extension".into(), e("REPEAT")),
    ];
    x.inputs = vec![vec3("Vector", [0.0; 3]).hide_value()];
    x.outputs = vec![out_c("Color"), out_f("Alpha")];
    x
}

fn tex_coord() -> Node {
    let mut x = n("ShaderNodeTexCoord", "Texture Coordinate", 140.0);
    x.props = vec![("object".into(), Prop::Id(None)), ("from_instancer".into(), Prop::Bool(false))];
    x.outputs = vec![out_v("Generated"), out_v("Normal"), out_v("UV"), out_v("Object"), out_v("Camera"), out_v("Window"), out_v("Reflection")];
    x
}

fn value() -> Node {
    let mut x = n("ShaderNodeValue", "Value", 140.0);
    x.outputs = vec![f("Value", 0.5)];
    x
}

fn rgb() -> Node {
    let mut x = n("ShaderNodeRGB", "RGB", 140.0);
    x.outputs = vec![col("Color", [0.5, 0.5, 0.5, 1.0])];
    x
}

fn math() -> Node {
    let mut x = n("ShaderNodeMath", "Math", 140.0);
    x.props = vec![("operation".into(), e("ADD")), ("use_clamp".into(), Prop::Bool(false))];
    x.inputs = vec![f("Value", 0.5).ident("Value"), f("Value", 0.5).ident("Value_001"), f("Value", 0.5).ident("Value_002")];
    x.inputs[2].enabled = false;
    x.outputs = vec![out_f("Value")];
    x
}

fn mix() -> Node {
    let mut x = n("ShaderNodeMix", "Mix", 140.0);
    x.props = vec![
        ("data_type".into(), e("RGBA")),
        ("blend_type".into(), e("MIX")),
        ("clamp_factor".into(), Prop::Bool(true)),
        ("clamp_result".into(), Prop::Bool(false)),
        ("factor_mode".into(), e("UNIFORM")),
    ];
    x.inputs = vec![
        fac("Factor", 0.5).ident("Factor_Float"),
        vec3("Factor", [0.5; 3]).ident("Factor_Vector"),
        f("A", 0.0).ident("A_Float"),
        f("B", 0.0).ident("B_Float"),
        vec3("A", [0.0; 3]).ident("A_Vector"),
        vec3("B", [0.0; 3]).ident("B_Vector"),
        col("A", [0.5, 0.5, 0.5, 1.0]).ident("A_Color"),
        col("B", [0.5, 0.5, 0.5, 1.0]).ident("B_Color"),
    ];
    x.outputs = vec![out_f("Result").ident("Result_Float"), out_v("Result").ident("Result_Vector"), out_c("Result").ident("Result_Color")];
    for (i, s) in x.inputs.iter_mut().enumerate() {
        s.enabled = matches!(i, 0 | 6 | 7);
    }
    for (i, s) in x.outputs.iter_mut().enumerate() {
        s.enabled = i == 2;
    }
    x
}

fn frame() -> Node {
    let mut x = n("NodeFrame", "Frame", 150.0);
    x.props = vec![("shrink".into(), Prop::Bool(true)), ("label_size".into(), Prop::Int(20))];
    x
}

fn reroute() -> Node {
    let mut x = n("NodeReroute", "Reroute", 16.0);
    x.inputs = vec![col("Input", [0.0, 0.0, 0.0, 1.0])];
    x.outputs = vec![out_c("Output")];
    x
}

macro_rules! ty {
    ($id:literal, $label:literal, $tree:ident, $cat:ident, $f:expr) => {
        NodeType { idname: $id, label: $label, tree: TreeKind::$tree, category: Category::$cat, build: $f }
    };
}

static TYPES: &[NodeType] = &[
    ty!("ShaderNodeTexCoord", "Texture Coordinate", Shader, Input, tex_coord),
    ty!("ShaderNodeRGB", "RGB", Shader, Input, rgb),
    ty!("ShaderNodeValue", "Value", Shader, Input, value),
    ty!("ShaderNodeOutputMaterial", "Material Output", Shader, Output, output_material),
    ty!("ShaderNodeOutputWorld", "World Output", Shader, Output, output_world),
    ty!("ShaderNodeAddShader", "Add Shader", Shader, Shader, add_shader),
    ty!("ShaderNodeBackground", "Background", Shader, Shader, background),
    ty!("ShaderNodeBsdfDiffuse", "Diffuse BSDF", Shader, Shader, diffuse),
    ty!("ShaderNodeEmission", "Emission", Shader, Shader, emission),
    ty!("ShaderNodeBsdfAnisotropic", "Glossy BSDF", Shader, Shader, glossy),
    ty!("ShaderNodeMixShader", "Mix Shader", Shader, Shader, mix_shader),
    ty!("ShaderNodeBsdfPrincipled", "Principled BSDF", Shader, Shader, principled),
    ty!("ShaderNodeTexImage", "Image Texture", Shader, Texture, tex_image),
    ty!("ShaderNodeMix", "Mix", Shader, Color, mix),
    ty!("ShaderNodeMath", "Math", Shader, Converter, math),
    ty!("NodeFrame", "Frame", Shader, Layout, frame),
    ty!("NodeReroute", "Reroute", Shader, Layout, reroute),
];

/// Every registered node type.
pub fn node_types() -> &'static [NodeType] {
    TYPES
}

pub fn node_type(idname: &str) -> Option<&'static NodeType> {
    TYPES.iter().find(|t| t.idname == idname)
}

/// A new node of type `idname` with its default name, sockets and props.
pub fn new_node(idname: &str) -> Option<Node> {
    node_type(idname).map(|t| (t.build)())
}

/// The Add-menu categories for a tree kind, in Blender's order, each with
/// its node types. Layout nodes (Frame, Reroute) appear in every tree.
pub fn categories(kind: TreeKind) -> Vec<(Category, Vec<&'static NodeType>)> {
    let mut out: Vec<(Category, Vec<&'static NodeType>)> = Vec::new();
    for t in TYPES {
        if t.tree != kind && t.category != Category::Layout {
            continue;
        }
        match out.iter_mut().find(|c| c.0 == t.category) {
            Some(c) => c.1.push(t),
            None => out.push((t.category, vec![t])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_builds_with_its_idname_and_unique_identifiers() {
        for t in node_types() {
            let n = (t.build)();
            assert_eq!(n.idname, t.idname);
            for (i, a) in n.inputs.iter().enumerate() {
                for b in &n.inputs[i + 1..] {
                    assert_ne!(a.identifier, b.identifier, "{} input {}", t.idname, a.identifier);
                }
            }
            for (i, a) in n.outputs.iter().enumerate() {
                for b in &n.outputs[i + 1..] {
                    assert_ne!(a.identifier, b.identifier, "{} output {}", t.idname, a.identifier);
                }
            }
        }
    }
}
