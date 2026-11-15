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

// ---------------------------------------------------------------------------
// More socket helpers

fn string(name: &str, v: &str) -> Socket {
    Socket::new(name, T::String, V::String(v.into()))
}
fn dist(name: &str, v: f32) -> Socket {
    f(name, v).subtype(S::Distance)
}
fn angle(name: &str, v: f32) -> Socket {
    f(name, v).subtype(S::Angle)
}
/// An unlinked "Vector" input that defaults to the texture coordinates.
fn vector_in() -> Socket {
    vec3("Vector", [0.0; 3]).hide_value()
}
fn tangent() -> Socket {
    vec3("Tangent", [0.0; 3]).hide_value()
}
fn gray(v: f32) -> [f32; 4] {
    [v, v, v, 1.0]
}
fn props(x: &mut Node, p: &[(&str, Prop)]) {
    x.props = p.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
}
fn b(v: bool) -> Prop {
    Prop::Bool(v)
}
fn pi(v: i32) -> Prop {
    Prop::Int(v)
}
fn pf(v: f32) -> Prop {
    Prop::Float(v)
}
fn ps(v: &str) -> Prop {
    Prop::String(v.into())
}

// ---------------------------------------------------------------------------
// Input

fn ambient_occlusion() -> Node {
    let mut x = n("ShaderNodeAmbientOcclusion", "Ambient Occlusion", 140.0);
    props(&mut x, &[("samples", pi(16)), ("inside", b(false)), ("only_local", b(false))]);
    x.inputs = vec![col("Color", [1.0; 4]), dist("Distance", 1.0).range(0.0, 1000.0), normal()];
    x.outputs = vec![out_c("Color"), out_f("AO")];
    x
}

fn attribute() -> Node {
    let mut x = n("ShaderNodeAttribute", "Attribute", 140.0);
    props(&mut x, &[("attribute_type", e("GEOMETRY")), ("attribute_name", ps(""))]);
    x.outputs = vec![out_c("Color"), out_v("Vector"), out_f("Fac"), out_f("Alpha")];
    x
}

fn bevel() -> Node {
    let mut x = n("ShaderNodeBevel", "Bevel", 140.0);
    props(&mut x, &[("samples", pi(4))]);
    x.inputs = vec![dist("Radius", 0.05).range(0.0, 1000.0), normal()];
    x.outputs = vec![out_v("Normal")];
    x
}

fn camera_data() -> Node {
    let mut x = n("ShaderNodeCameraData", "Camera Data", 140.0);
    x.outputs = vec![out_v("View Vector"), out_f("View Z Depth"), out_f("View Distance")];
    x
}

fn color_attribute() -> Node {
    let mut x = n("ShaderNodeVertexColor", "Color Attribute", 140.0);
    props(&mut x, &[("layer_name", ps(""))]);
    x.outputs = vec![out_c("Color"), out_f("Alpha")];
    x
}

fn curves_info() -> Node {
    let mut x = n("ShaderNodeHairInfo", "Curves Info", 140.0);
    x.outputs = vec![out_f("Is Strand"), out_f("Intercept"), out_f("Length"), out_f("Thickness"), out_v("Tangent Normal"), out_f("Random")];
    x
}

fn fresnel() -> Node {
    let mut x = n("ShaderNodeFresnel", "Fresnel", 140.0);
    x.inputs = vec![f("IOR", 1.5).range(0.0, 1000.0), normal()];
    x.outputs = vec![out_f("Fac")];
    x
}

fn geometry() -> Node {
    let mut x = n("ShaderNodeNewGeometry", "Geometry", 140.0);
    x.outputs = vec![
        out_v("Position"),
        out_v("Normal"),
        out_v("Tangent"),
        out_v("True Normal"),
        out_v("Incoming"),
        out_v("Parametric"),
        out_f("Backfacing"),
        out_f("Pointiness"),
        out_f("Random Per Island"),
    ];
    x
}

fn layer_weight() -> Node {
    let mut x = n("ShaderNodeLayerWeight", "Layer Weight", 140.0);
    x.inputs = vec![fac("Blend", 0.5), normal()];
    x.outputs = vec![out_f("Fresnel"), out_f("Facing")];
    x
}

fn light_path() -> Node {
    let mut x = n("ShaderNodeLightPath", "Light Path", 140.0);
    x.outputs = [
        "Is Camera Ray",
        "Is Shadow Ray",
        "Is Diffuse Ray",
        "Is Glossy Ray",
        "Is Singular Ray",
        "Is Reflection Ray",
        "Is Transmission Ray",
        "Is Volume Scatter Ray",
        "Ray Length",
        "Ray Depth",
        "Diffuse Depth",
        "Glossy Depth",
        "Transparent Depth",
        "Transmission Depth",
    ]
    .iter()
    .map(|s| out_f(s))
    .collect();
    x
}

fn object_info() -> Node {
    let mut x = n("ShaderNodeObjectInfo", "Object Info", 140.0);
    x.outputs = vec![out_v("Location"), out_c("Color"), out_f("Alpha"), out_f("Object Index"), out_f("Material Index"), out_f("Random")];
    x
}

fn particle_info() -> Node {
    let mut x = n("ShaderNodeParticleInfo", "Particle Info", 140.0);
    x.outputs = vec![out_f("Index"), out_f("Random"), out_f("Age"), out_f("Lifetime"), out_v("Location"), out_f("Size"), out_v("Velocity"), out_v("Angular Velocity")];
    x
}

fn point_info() -> Node {
    let mut x = n("ShaderNodePointInfo", "Point Info", 140.0);
    x.outputs = vec![out_v("Position"), out_f("Radius"), out_f("Random")];
    x
}

fn tangent_node() -> Node {
    let mut x = n("ShaderNodeTangent", "Tangent", 150.0);
    props(&mut x, &[("direction_type", e("RADIAL")), ("axis", e("Z")), ("uv_map", ps(""))]);
    x.outputs = vec![out_v("Tangent")];
    x
}

fn uv_map() -> Node {
    let mut x = n("ShaderNodeUVMap", "UV Map", 150.0);
    props(&mut x, &[("from_instancer", b(false)), ("uv_map", ps(""))]);
    x.outputs = vec![out_v("UV")];
    x
}

fn volume_info() -> Node {
    let mut x = n("ShaderNodeVolumeInfo", "Volume Info", 140.0);
    x.outputs = vec![out_c("Color"), out_f("Density"), out_f("Flame"), out_f("Temperature")];
    x
}

fn wireframe() -> Node {
    let mut x = n("ShaderNodeWireframe", "Wireframe", 140.0);
    props(&mut x, &[("use_pixel_size", b(false))]);
    x.inputs = vec![f("Size", 0.01).range(0.0, 100.0)];
    x.outputs = vec![out_f("Fac")];
    x
}

// ---------------------------------------------------------------------------
// Output

fn output_light() -> Node {
    let mut x = n("ShaderNodeOutputLight", "Light Output", 140.0);
    props(&mut x, &[("target", e("ALL")), ("is_active_output", b(true))]);
    x.inputs = vec![shader("Surface")];
    x
}

fn output_aov() -> Node {
    let mut x = n("ShaderNodeOutputAOV", "AOV Output", 140.0);
    props(&mut x, &[("aov_name", ps(""))]);
    x.inputs = vec![col("Color", [0.0, 0.0, 0.0, 1.0]), f("Value", 0.0)];
    x
}

// ---------------------------------------------------------------------------
// Shader

fn glass() -> Node {
    let mut x = n("ShaderNodeBsdfGlass", "Glass BSDF", 150.0);
    props(&mut x, &[("distribution", e("MULTI_GGX"))]);
    x.inputs = vec![col("Color", [1.0; 4]), fac("Roughness", 0.0), f("IOR", 1.5).range(1.0, 1000.0), normal()];
    x.outputs = vec![shader("BSDF")];
    x
}

fn refraction() -> Node {
    let mut x = n("ShaderNodeBsdfRefraction", "Refraction BSDF", 150.0);
    props(&mut x, &[("distribution", e("GGX"))]);
    x.inputs = vec![col("Color", [1.0; 4]), fac("Roughness", 0.0), f("IOR", 1.5).range(1.0, 1000.0), normal()];
    x.outputs = vec![shader("BSDF")];
    x
}

fn hair() -> Node {
    let mut x = n("ShaderNodeBsdfHair", "Hair BSDF", 150.0);
    props(&mut x, &[("component", e("Reflection"))]);
    x.inputs = vec![col("Color", gray(0.8)), angle("Offset", 0.0), fac("RoughnessU", 0.1), fac("RoughnessV", 1.0), tangent()];
    x.outputs = vec![shader("BSDF")];
    x
}

fn hair_principled() -> Node {
    let mut x = n("ShaderNodeBsdfHairPrincipled", "Principled Hair BSDF", 240.0);
    props(&mut x, &[("model", e("CHIANG")), ("parametrization", e("COLOR"))]);
    x.inputs = vec![
        col("Color", [0.017513, 0.005763, 0.002059, 1.0]),
        f("Melanin", 0.8).range(0.0, 1.0),
        f("Melanin Redness", 1.0).range(0.0, 1.0),
        col("Tint", [1.0; 4]),
        vec3("Absorption Coefficient", [0.245531, 0.52, 1.365]),
        fac("Aspect Ratio", 0.85),
        fac("Roughness", 0.3),
        fac("Radial Roughness", 0.3),
        fac("Coat", 0.0),
        f("IOR", 1.55).range(0.0, 1000.0),
        angle("Offset", 2f32.to_radians()),
        fac("Random Color", 0.0),
        fac("Random Roughness", 0.0),
        f("Random", 0.0).hide_value(),
    ];
    x.outputs = vec![shader("BSDF")];
    x
}

fn holdout() -> Node {
    let mut x = n("ShaderNodeHoldout", "Holdout", 140.0);
    x.outputs = vec![shader("Holdout")];
    x
}

fn principled_volume() -> Node {
    let mut x = n("ShaderNodeVolumePrincipled", "Principled Volume", 240.0);
    x.inputs = vec![
        col("Color", gray(0.5)),
        string("Color Attribute", ""),
        f("Density", 1.0).range(0.0, 1000.0),
        string("Density Attribute", "density"),
        f("Anisotropy", 0.0).range(-1.0, 1.0),
        col("Absorption Color", [0.0, 0.0, 0.0, 1.0]),
        f("Emission Strength", 0.0).range(0.0, 1000.0),
        col("Emission Color", [1.0; 4]),
        f("Blackbody Intensity", 0.0).range(0.0, 1.0).subtype(S::Factor),
        col("Blackbody Tint", [1.0; 4]),
        f("Temperature", 1000.0).range(0.0, 6500.0).subtype(S::ColorTemperature),
        string("Temperature Attribute", "temperature"),
    ];
    x.outputs = vec![shader("Volume")];
    x
}

fn sheen() -> Node {
    let mut x = n("ShaderNodeBsdfSheen", "Sheen BSDF", 150.0);
    props(&mut x, &[("distribution", e("MICROFIBER"))]);
    x.inputs = vec![col("Color", gray(0.8)), fac("Roughness", 0.5), normal()];
    x.outputs = vec![shader("BSDF")];
    x
}

fn subsurface() -> Node {
    let mut x = n("ShaderNodeSubsurfaceScattering", "Subsurface Scattering", 150.0);
    props(&mut x, &[("falloff", e("RANDOM_WALK"))]);
    x.inputs = vec![
        col("Color", gray(0.8)),
        dist("Scale", 0.05).range(0.0, 1000.0),
        vec3("Radius", [1.0, 0.2, 0.1]),
        f("IOR", 1.4).range(1.01, 3.8),
        fac("Roughness", 1.0),
        fac("Anisotropy", 0.0),
        normal(),
    ];
    x.outputs = vec![shader("BSSRDF")];
    x
}

fn toon() -> Node {
    let mut x = n("ShaderNodeBsdfToon", "Toon BSDF", 150.0);
    props(&mut x, &[("component", e("DIFFUSE"))]);
    x.inputs = vec![col("Color", gray(0.8)), fac("Size", 0.5), fac("Smooth", 0.0), normal()];
    x.outputs = vec![shader("BSDF")];
    x
}

fn translucent() -> Node {
    let mut x = n("ShaderNodeBsdfTranslucent", "Translucent BSDF", 150.0);
    x.inputs = vec![col("Color", gray(0.8)), normal()];
    x.outputs = vec![shader("BSDF")];
    x
}

fn transparent() -> Node {
    let mut x = n("ShaderNodeBsdfTransparent", "Transparent BSDF", 150.0);
    x.inputs = vec![col("Color", [1.0; 4])];
    x.outputs = vec![shader("BSDF")];
    x
}

fn volume_absorption() -> Node {
    let mut x = n("ShaderNodeVolumeAbsorption", "Volume Absorption", 150.0);
    x.inputs = vec![col("Color", gray(0.8)), f("Density", 1.0).range(0.0, 1000.0)];
    x.outputs = vec![shader("Volume")];
    x
}

fn volume_scatter() -> Node {
    let mut x = n("ShaderNodeVolumeScatter", "Volume Scatter", 150.0);
    props(&mut x, &[("phase", e("HENYEY_GREENSTEIN"))]);
    x.inputs = vec![col("Color", gray(0.8)), f("Density", 1.0).range(0.0, 1000.0), f("Anisotropy", 0.0).range(-1.0, 1.0)];
    x.outputs = vec![shader("Volume")];
    x
}

// ---------------------------------------------------------------------------
// Texture

fn tex_brick() -> Node {
    let mut x = n("ShaderNodeTexBrick", "Brick Texture", 150.0);
    props(&mut x, &[("offset", pf(0.5)), ("offset_frequency", pi(2)), ("squash", pf(1.0)), ("squash_frequency", pi(2))]);
    x.inputs = vec![
        vector_in(),
        col("Color1", gray(0.8)),
        col("Color2", gray(0.2)),
        col("Mortar", [0.0, 0.0, 0.0, 1.0]),
        f("Scale", 5.0).range(-1000.0, 1000.0),
        f("Mortar Size", 0.02).range(0.0, 0.125),
        fac("Mortar Smooth", 0.1),
        f("Bias", 0.0).range(-1.0, 1.0),
        f("Brick Width", 0.5).range(0.01, 100.0),
        f("Row Height", 0.25).range(0.01, 100.0),
    ];
    x.outputs = vec![out_c("Color"), out_f("Fac")];
    x
}

fn tex_checker() -> Node {
    let mut x = n("ShaderNodeTexChecker", "Checker Texture", 140.0);
    x.inputs = vec![vector_in(), col("Color1", gray(0.8)), col("Color2", gray(0.2)), f("Scale", 5.0).range(-10000.0, 10000.0)];
    x.outputs = vec![out_c("Color"), out_f("Fac")];
    x
}

fn tex_environment() -> Node {
    let mut x = n("ShaderNodeTexEnvironment", "Environment Texture", 240.0);
    props(&mut x, &[("image", Prop::Id(None)), ("interpolation", e("Linear")), ("projection", e("EQUIRECTANGULAR"))]);
    x.inputs = vec![vector_in()];
    x.outputs = vec![out_c("Color")];
    x
}

fn tex_gabor() -> Node {
    let mut x = n("ShaderNodeTexGabor", "Gabor Texture", 150.0);
    props(&mut x, &[("gabor_type", e("2D"))]);
    x.inputs = vec![
        vector_in(),
        f("Scale", 5.0),
        f("Frequency", 2.0).range(0.0, f32::MAX),
        fac("Anisotropy", 1.0),
        angle("Orientation", core::f32::consts::FRAC_PI_4).ident("Orientation 2D"),
        vec3("Orientation", [std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2, 0.0]).subtype(S::Direction).ident("Orientation 3D"),
    ];
    x.inputs[5].enabled = false;
    x.outputs = vec![out_f("Value"), out_f("Phase"), out_f("Intensity")];
    x
}

fn tex_gradient() -> Node {
    let mut x = n("ShaderNodeTexGradient", "Gradient Texture", 140.0);
    props(&mut x, &[("gradient_type", e("LINEAR"))]);
    x.inputs = vec![vector_in()];
    x.outputs = vec![out_c("Color"), out_f("Fac")];
    x
}

fn tex_ies() -> Node {
    let mut x = n("ShaderNodeTexIES", "IES Texture", 140.0);
    props(&mut x, &[("mode", e("INTERNAL")), ("ies", Prop::Id(None)), ("filepath", ps(""))]);
    x.inputs = vec![vector_in(), f("Strength", 1.0).range(0.0, 1_000_000.0)];
    x.outputs = vec![out_f("Fac")];
    x
}

fn tex_magic() -> Node {
    let mut x = n("ShaderNodeTexMagic", "Magic Texture", 140.0);
    props(&mut x, &[("turbulence_depth", pi(2))]);
    x.inputs = vec![vector_in(), f("Scale", 5.0).range(-1000.0, 1000.0), f("Distortion", 1.0).range(-1000.0, 1000.0)];
    x.outputs = vec![out_c("Color"), out_f("Fac")];
    x
}

fn tex_noise() -> Node {
    let mut x = n("ShaderNodeTexNoise", "Noise Texture", 145.0);
    props(&mut x, &[("noise_dimensions", e("3D")), ("noise_type", e("FBM")), ("normalize", b(true))]);
    x.inputs = vec![
        vector_in(),
        f("W", 0.0).range(-1000.0, 1000.0),
        f("Scale", 5.0).range(-1000.0, 1000.0),
        f("Detail", 2.0).range(0.0, 15.0),
        fac("Roughness", 0.5),
        f("Lacunarity", 2.0).range(0.0, 1000.0),
        f("Offset", 0.0).range(-1000.0, 1000.0),
        f("Gain", 1.0).range(0.0, 1000.0),
        f("Distortion", 0.0).range(-1000.0, 1000.0),
    ];
    x.outputs = vec![out_f("Fac"), out_c("Color")];
    update_sockets(&mut x);
    x
}

fn tex_sky() -> Node {
    let mut x = n("ShaderNodeTexSky", "Sky Texture", 150.0);
    props(
        &mut x,
        &[
            ("sky_type", e("NISHITA")),
            ("sun_direction", Prop::Vector([0.0, 0.0, 1.0])),
            ("turbidity", pf(2.2)),
            ("ground_albedo", pf(0.3)),
            ("sun_disc", b(true)),
            ("sun_size", pf(0.545f32.to_radians())),
            ("sun_intensity", pf(1.0)),
            ("sun_elevation", pf(15f32.to_radians())),
            ("sun_rotation", pf(0.0)),
            ("altitude", pf(0.0)),
            ("air_density", pf(1.0)),
            ("dust_density", pf(1.0)),
            ("ozone_density", pf(1.0)),
        ],
    );
    x.inputs = vec![vector_in()];
    x.outputs = vec![out_c("Color")];
    x
}

fn tex_voronoi() -> Node {
    let mut x = n("ShaderNodeTexVoronoi", "Voronoi Texture", 145.0);
    props(&mut x, &[("voronoi_dimensions", e("3D")), ("feature", e("F1")), ("distance", e("EUCLIDEAN")), ("normalize", b(false))]);
    x.inputs = vec![
        vector_in(),
        f("W", 0.0).range(-1000.0, 1000.0),
        f("Scale", 5.0).range(-1000.0, 1000.0),
        f("Detail", 0.0).range(0.0, 15.0),
        fac("Roughness", 0.5),
        f("Lacunarity", 2.0).range(0.0, 1000.0),
        fac("Smoothness", 1.0),
        f("Exponent", 0.5).range(0.0, 32.0),
        fac("Randomness", 1.0),
    ];
    x.outputs = vec![out_f("Distance"), out_c("Color"), out_v("Position"), out_f("W"), out_f("Radius")];
    update_sockets(&mut x);
    x
}

fn tex_wave() -> Node {
    let mut x = n("ShaderNodeTexWave", "Wave Texture", 150.0);
    props(&mut x, &[("wave_type", e("BANDS")), ("bands_direction", e("X")), ("rings_direction", e("X")), ("wave_profile", e("SIN"))]);
    x.inputs = vec![
        vector_in(),
        f("Scale", 5.0).range(-1000.0, 1000.0),
        f("Distortion", 0.0).range(-1000.0, 1000.0),
        f("Detail", 2.0).range(0.0, 15.0),
        f("Detail Scale", 1.0).range(-1000.0, 1000.0),
        fac("Detail Roughness", 0.5),
        f("Phase Offset", 0.0).range(-1000.0, 1000.0),
    ];
    x.outputs = vec![out_c("Color"), out_f("Fac")];
    x
}

fn tex_white_noise() -> Node {
    let mut x = n("ShaderNodeTexWhiteNoise", "White Noise Texture", 140.0);
    props(&mut x, &[("noise_dimensions", e("3D"))]);
    x.inputs = vec![vector_in(), f("W", 0.0).range(-1000.0, 1000.0)];
    x.outputs = vec![out_f("Value"), out_c("Color")];
    update_sockets(&mut x);
    x
}

// ---------------------------------------------------------------------------
// Color

fn bright_contrast() -> Node {
    let mut x = n("ShaderNodeBrightContrast", "Brightness/Contrast", 140.0);
    x.inputs = vec![col("Color", [1.0; 4]), f("Bright", 0.0).range(-100.0, 100.0), f("Contrast", 0.0).range(-100.0, 100.0)];
    x.outputs = vec![out_c("Color")];
    x
}

fn gamma() -> Node {
    let mut x = n("ShaderNodeGamma", "Gamma", 140.0);
    x.inputs = vec![col("Color", [1.0; 4]), f("Gamma", 1.0).range(0.001, 10.0)];
    x.outputs = vec![out_c("Color")];
    x
}

fn hue_saturation() -> Node {
    let mut x = n("ShaderNodeHueSaturation", "Hue/Saturation/Value", 150.0);
    x.inputs = vec![fac("Hue", 0.5), f("Saturation", 1.0).range(0.0, 2.0), f("Value", 1.0).range(0.0, 2.0), fac("Fac", 1.0), col("Color", gray(0.8))];
    x.outputs = vec![out_c("Color")];
    x
}

fn invert() -> Node {
    let mut x = n("ShaderNodeInvert", "Invert Color", 140.0);
    x.inputs = vec![fac("Fac", 1.0), col("Color", [0.0, 0.0, 0.0, 1.0])];
    x.outputs = vec![out_c("Color")];
    x
}

fn light_falloff() -> Node {
    let mut x = n("ShaderNodeLightFalloff", "Light Falloff", 150.0);
    x.inputs = vec![f("Strength", 100.0).range(0.0, 1_000_000.0), f("Smooth", 0.0).range(0.0, 1000.0)];
    x.outputs = vec![out_f("Quadratic"), out_f("Linear"), out_f("Constant")];
    x
}

fn mix_rgb() -> Node {
    let mut x = n("ShaderNodeMixRGB", "Mix", 140.0);
    props(&mut x, &[("blend_type", e("MIX")), ("use_alpha", b(false)), ("use_clamp", b(false))]);
    x.inputs = vec![fac("Fac", 0.5), col("Color1", gray(0.5)), col("Color2", gray(0.5))];
    x.outputs = vec![out_c("Color")];
    x
}

fn rgb_curve() -> Node {
    let mut x = n("ShaderNodeRGBCurve", "RGB Curves", 240.0);
    props(&mut x, &[("mapping", Prop::Curve(Default::default()))]);
    x.inputs = vec![fac("Fac", 1.0), col("Color", [1.0; 4])];
    x.outputs = vec![out_c("Color")];
    x
}

// ---------------------------------------------------------------------------
// Vector

fn bump() -> Node {
    let mut x = n("ShaderNodeBump", "Bump", 140.0);
    props(&mut x, &[("invert", b(false))]);
    x.inputs = vec![fac("Strength", 1.0), dist("Distance", 1.0).range(0.0, 1000.0), f("Height", 1.0).hide_value(), normal()];
    x.outputs = vec![out_v("Normal")];
    x
}

fn displacement() -> Node {
    let mut x = n("ShaderNodeDisplacement", "Displacement", 140.0);
    props(&mut x, &[("space", e("OBJECT"))]);
    x.inputs = vec![f("Height", 0.0).hide_value(), f("Midlevel", 0.5).range(0.0, 1000.0), f("Scale", 1.0).range(0.0, 1000.0), normal()];
    x.outputs = vec![out_v("Displacement")];
    x
}

fn mapping() -> Node {
    let mut x = n("ShaderNodeMapping", "Mapping", 140.0);
    props(&mut x, &[("vector_type", e("POINT"))]);
    x.inputs = vec![
        vec3("Vector", [0.0; 3]),
        vec3("Location", [0.0; 3]).subtype(S::Translation),
        vec3("Rotation", [0.0; 3]).subtype(S::Euler),
        vec3("Scale", [1.0; 3]).subtype(S::Xyz),
    ];
    x.outputs = vec![out_v("Vector")];
    update_sockets(&mut x);
    x
}

fn normal_node() -> Node {
    let mut x = n("ShaderNodeNormal", "Normal", 140.0);
    x.inputs = vec![vec3("Normal", [0.0, 0.0, 1.0]).subtype(S::Direction)];
    x.outputs = vec![vec3("Normal", [0.0, 0.0, 1.0]).subtype(S::Direction), out_f("Dot")];
    x
}

fn normal_map() -> Node {
    let mut x = n("ShaderNodeNormalMap", "Normal Map", 150.0);
    props(&mut x, &[("space", e("TANGENT")), ("uv_map", ps(""))]);
    x.inputs = vec![f("Strength", 1.0).range(0.0, 10.0), col("Color", [0.5, 0.5, 1.0, 1.0])];
    x.outputs = vec![out_v("Normal")];
    x
}

fn vector_curve() -> Node {
    let mut x = n("ShaderNodeVectorCurve", "Vector Curves", 240.0);
    let mut m = super::CurveMapping::default();
    m.curves = vec![vec![[-1.0, -1.0], [1.0, 1.0]]; 3];
    props(&mut x, &[("mapping", Prop::Curve(m))]);
    x.inputs = vec![fac("Fac", 1.0), vec3("Vector", [0.0; 3])];
    x.outputs = vec![out_v("Vector")];
    x
}

fn vector_displacement() -> Node {
    let mut x = n("ShaderNodeVectorDisplacement", "Vector Displacement", 150.0);
    props(&mut x, &[("space", e("TANGENT"))]);
    x.inputs = vec![col("Vector", [0.0, 0.0, 0.0, 1.0]).hide_value(), f("Midlevel", 0.0).range(0.0, 1000.0), f("Scale", 1.0).range(0.0, 1000.0)];
    x.outputs = vec![out_v("Displacement")];
    x
}

fn vector_rotate() -> Node {
    let mut x = n("ShaderNodeVectorRotate", "Vector Rotate", 140.0);
    props(&mut x, &[("rotation_type", e("AXIS_ANGLE")), ("invert", b(false))]);
    x.inputs = vec![vec3("Vector", [0.0; 3]).hide_value(), vec3("Center", [0.0; 3]), vec3("Axis", [0.0, 0.0, 1.0]), angle("Angle", 0.0), vec3("Rotation", [0.0; 3]).subtype(S::Euler)];
    x.outputs = vec![out_v("Vector")];
    update_sockets(&mut x);
    x
}

fn vector_transform() -> Node {
    let mut x = n("ShaderNodeVectorTransform", "Vector Transform", 150.0);
    props(&mut x, &[("vector_type", e("VECTOR")), ("convert_from", e("WORLD")), ("convert_to", e("OBJECT"))]);
    x.inputs = vec![vec3("Vector", [0.5; 3])];
    x.outputs = vec![out_v("Vector")];
    x
}

// ---------------------------------------------------------------------------
// Converter

fn blackbody() -> Node {
    let mut x = n("ShaderNodeBlackbody", "Blackbody", 140.0);
    x.inputs = vec![f("Temperature", 1500.0).range(800.0, 12000.0).subtype(S::ColorTemperature)];
    x.outputs = vec![out_c("Color")];
    x
}

fn clamp() -> Node {
    let mut x = n("ShaderNodeClamp", "Clamp", 140.0);
    props(&mut x, &[("clamp_type", e("MINMAX"))]);
    x.inputs = vec![f("Value", 1.0), f("Min", 0.0), f("Max", 1.0)];
    x.outputs = vec![out_f("Result")];
    x
}

fn color_ramp() -> Node {
    let mut x = n("ShaderNodeValToRGB", "Color Ramp", 240.0);
    props(&mut x, &[("color_ramp", Prop::Ramp(Default::default()))]);
    x.inputs = vec![fac("Fac", 0.5)];
    x.outputs = vec![out_c("Color"), out_f("Alpha")];
    x
}

fn combine_color() -> Node {
    let mut x = n("ShaderNodeCombineColor", "Combine Color", 140.0);
    props(&mut x, &[("mode", e("RGB"))]);
    x.inputs = vec![fac("Red", 0.0), fac("Green", 0.0), fac("Blue", 0.0)];
    x.outputs = vec![out_c("Color")];
    x
}

fn combine_xyz() -> Node {
    let mut x = n("ShaderNodeCombineXYZ", "Combine XYZ", 140.0);
    x.inputs = vec![f("X", 0.0), f("Y", 0.0), f("Z", 0.0)];
    x.outputs = vec![out_v("Vector")];
    x
}

fn float_curve() -> Node {
    let mut x = n("ShaderNodeFloatCurve", "Float Curve", 240.0);
    let mut m = super::CurveMapping::default();
    m.curves.truncate(1);
    props(&mut x, &[("mapping", Prop::Curve(m))]);
    x.inputs = vec![fac("Factor", 1.0), f("Value", 1.0)];
    x.outputs = vec![out_f("Value")];
    x
}

fn map_range() -> Node {
    let mut x = n("ShaderNodeMapRange", "Map Range", 140.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("interpolation_type", e("LINEAR")), ("clamp", b(true))]);
    x.inputs = vec![
        f("Value", 1.0),
        f("From Min", 0.0),
        f("From Max", 1.0),
        f("To Min", 0.0),
        f("To Max", 1.0),
        f("Steps", 4.0),
        vec3("Vector", [0.0; 3]),
        vec3("From Min", [0.0; 3]).ident("From_Min_FLOAT3"),
        vec3("From Max", [1.0; 3]).ident("From_Max_FLOAT3"),
        vec3("To Min", [0.0; 3]).ident("To_Min_FLOAT3"),
        vec3("To Max", [1.0; 3]).ident("To_Max_FLOAT3"),
        vec3("Steps", [4.0; 3]).ident("Steps_FLOAT3"),
    ];
    x.outputs = vec![out_f("Result"), out_v("Vector")];
    update_sockets(&mut x);
    x
}

fn rgb_to_bw() -> Node {
    let mut x = n("ShaderNodeRGBToBW", "RGB to BW", 140.0);
    x.inputs = vec![col("Color", gray(0.5))];
    x.outputs = vec![out_f("Val")];
    x
}

fn separate_color() -> Node {
    let mut x = n("ShaderNodeSeparateColor", "Separate Color", 140.0);
    props(&mut x, &[("mode", e("RGB"))]);
    x.inputs = vec![col("Color", gray(0.8))];
    x.outputs = vec![out_f("Red"), out_f("Green"), out_f("Blue")];
    x
}

fn separate_xyz() -> Node {
    let mut x = n("ShaderNodeSeparateXYZ", "Separate XYZ", 140.0);
    x.inputs = vec![vec3("Vector", [0.0; 3])];
    x.outputs = vec![out_f("X"), out_f("Y"), out_f("Z")];
    x
}

fn shader_to_rgb() -> Node {
    let mut x = n("ShaderNodeShaderToRGB", "Shader to RGB", 140.0);
    x.inputs = vec![shader("Shader")];
    x.outputs = vec![out_c("Color"), out_f("Alpha")];
    x
}

fn vector_math() -> Node {
    let mut x = n("ShaderNodeVectorMath", "Vector Math", 140.0);
    props(&mut x, &[("operation", e("ADD"))]);
    x.inputs = vec![vec3("Vector", [0.0; 3]), vec3("Vector", [0.0; 3]).ident("Vector_001"), vec3("Vector", [0.0; 3]).ident("Vector_002"), f("Scale", 1.0)];
    x.outputs = vec![out_v("Vector"), out_f("Value")];
    update_sockets(&mut x);
    x
}

fn wavelength() -> Node {
    let mut x = n("ShaderNodeWavelength", "Wavelength", 140.0);
    x.inputs = vec![f("Wavelength", 500.0).range(380.0, 780.0).subtype(S::Wavelength)];
    x.outputs = vec![out_c("Color")];
    x
}

// ---------------------------------------------------------------------------
// Group and layout

fn group() -> Node {
    let mut x = n("ShaderNodeGroup", "Group", 140.0);
    props(&mut x, &[("node_tree", Prop::Id(None))]);
    x
}

fn group_input() -> Node {
    let mut x = n("NodeGroupInput", "Group Input", 140.0);
    x.outputs = vec![Socket::new("", T::Virtual, V::None).ident("__extend__")];
    x
}

fn group_output() -> Node {
    let mut x = n("NodeGroupOutput", "Group Output", 140.0);
    props(&mut x, &[("is_active_output", b(true))]);
    x.inputs = vec![Socket::new("", T::Virtual, V::None).ident("__extend__")];
    x
}

// ---------------------------------------------------------------------------
// Deprecated types, built only to read older files

fn legacy_glossy() -> Node {
    let mut x = n("ShaderNodeBsdfGlossy", "Glossy BSDF", 150.0);
    props(&mut x, &[("distribution", e("GGX"))]);
    x.inputs = vec![col("Color", gray(0.8)), fac("Roughness", 0.5), normal()];
    x.outputs = vec![shader("BSDF")];
    x
}

fn legacy_velvet() -> Node {
    let mut x = n("ShaderNodeBsdfVelvet", "Velvet BSDF", 150.0);
    x.inputs = vec![col("Color", gray(0.8)), fac("Sigma", 1.0), normal()];
    x.outputs = vec![shader("BSDF")];
    x
}

fn legacy_musgrave() -> Node {
    let mut x = n("ShaderNodeTexMusgrave", "Musgrave Texture", 150.0);
    props(&mut x, &[("musgrave_dimensions", e("3D")), ("musgrave_type", e("FBM"))]);
    x.inputs = vec![
        vector_in(),
        f("W", 0.0),
        f("Scale", 5.0),
        f("Detail", 2.0).range(0.0, 15.0),
        f("Dimension", 2.0).range(0.0, 1000.0),
        f("Lacunarity", 2.0).range(0.0, 1000.0),
        f("Offset", 0.0),
        f("Gain", 1.0),
    ];
    x.outputs = vec![out_f("Fac")];
    x
}

fn legacy_combine_rgb() -> Node {
    let mut x = n("ShaderNodeCombineRGB", "Combine RGB", 140.0);
    x.inputs = vec![fac("R", 0.0), fac("G", 0.0), fac("B", 0.0)];
    x.outputs = vec![out_c("Image")];
    x
}

fn legacy_separate_rgb() -> Node {
    let mut x = n("ShaderNodeSeparateRGB", "Separate RGB", 140.0);
    x.inputs = vec![col("Image", gray(0.8))];
    x.outputs = vec![out_f("R"), out_f("G"), out_f("B")];
    x
}

fn legacy_combine_hsv() -> Node {
    let mut x = n("ShaderNodeCombineHSV", "Combine HSV", 140.0);
    x.inputs = vec![fac("H", 0.0), fac("S", 0.0), fac("V", 0.0)];
    x.outputs = vec![out_c("Color")];
    x
}

fn legacy_separate_hsv() -> Node {
    let mut x = n("ShaderNodeSeparateHSV", "Separate HSV", 140.0);
    x.inputs = vec![col("Color", gray(0.8))];
    x.outputs = vec![out_f("H"), out_f("S"), out_f("V")];
    x
}

macro_rules! ty {
    ($id:literal, $label:literal, $tree:ident, $cat:ident, $f:expr) => {
        NodeType { idname: $id, label: $label, tree: TreeKind::$tree, category: Category::$cat, build: $f }
    };
}

/// In Add-menu order: categories as Blender lists them, alphabetical within.
static TYPES: &[NodeType] = &[
    ty!("ShaderNodeAmbientOcclusion", "Ambient Occlusion", Shader, Input, ambient_occlusion),
    ty!("ShaderNodeAttribute", "Attribute", Shader, Input, attribute),
    ty!("ShaderNodeBevel", "Bevel", Shader, Input, bevel),
    ty!("ShaderNodeCameraData", "Camera Data", Shader, Input, camera_data),
    ty!("ShaderNodeVertexColor", "Color Attribute", Shader, Input, color_attribute),
    ty!("ShaderNodeHairInfo", "Curves Info", Shader, Input, curves_info),
    ty!("ShaderNodeFresnel", "Fresnel", Shader, Input, fresnel),
    ty!("ShaderNodeNewGeometry", "Geometry", Shader, Input, geometry),
    ty!("ShaderNodeLayerWeight", "Layer Weight", Shader, Input, layer_weight),
    ty!("ShaderNodeLightPath", "Light Path", Shader, Input, light_path),
    ty!("ShaderNodeObjectInfo", "Object Info", Shader, Input, object_info),
    ty!("ShaderNodeParticleInfo", "Particle Info", Shader, Input, particle_info),
    ty!("ShaderNodePointInfo", "Point Info", Shader, Input, point_info),
    ty!("ShaderNodeRGB", "RGB", Shader, Input, rgb),
    ty!("ShaderNodeTangent", "Tangent", Shader, Input, tangent_node),
    ty!("ShaderNodeTexCoord", "Texture Coordinate", Shader, Input, tex_coord),
    ty!("ShaderNodeUVMap", "UV Map", Shader, Input, uv_map),
    ty!("ShaderNodeValue", "Value", Shader, Input, value),
    ty!("ShaderNodeVolumeInfo", "Volume Info", Shader, Input, volume_info),
    ty!("ShaderNodeWireframe", "Wireframe", Shader, Input, wireframe),
    ty!("ShaderNodeOutputAOV", "AOV Output", Shader, Output, output_aov),
    ty!("ShaderNodeOutputLight", "Light Output", Shader, Output, output_light),
    ty!("ShaderNodeOutputMaterial", "Material Output", Shader, Output, output_material),
    ty!("ShaderNodeOutputWorld", "World Output", Shader, Output, output_world),
    ty!("ShaderNodeAddShader", "Add Shader", Shader, Shader, add_shader),
    ty!("ShaderNodeBackground", "Background", Shader, Shader, background),
    ty!("ShaderNodeBsdfDiffuse", "Diffuse BSDF", Shader, Shader, diffuse),
    ty!("ShaderNodeEmission", "Emission", Shader, Shader, emission),
    ty!("ShaderNodeBsdfGlass", "Glass BSDF", Shader, Shader, glass),
    ty!("ShaderNodeBsdfAnisotropic", "Glossy BSDF", Shader, Shader, glossy),
    ty!("ShaderNodeBsdfHair", "Hair BSDF", Shader, Shader, hair),
    ty!("ShaderNodeHoldout", "Holdout", Shader, Shader, holdout),
    ty!("ShaderNodeMixShader", "Mix Shader", Shader, Shader, mix_shader),
    ty!("ShaderNodeBsdfPrincipled", "Principled BSDF", Shader, Shader, principled),
    ty!("ShaderNodeBsdfHairPrincipled", "Principled Hair BSDF", Shader, Shader, hair_principled),
    ty!("ShaderNodeVolumePrincipled", "Principled Volume", Shader, Shader, principled_volume),
    ty!("ShaderNodeBsdfRefraction", "Refraction BSDF", Shader, Shader, refraction),
    ty!("ShaderNodeBsdfSheen", "Sheen BSDF", Shader, Shader, sheen),
    ty!("ShaderNodeSubsurfaceScattering", "Subsurface Scattering", Shader, Shader, subsurface),
    ty!("ShaderNodeBsdfToon", "Toon BSDF", Shader, Shader, toon),
    ty!("ShaderNodeBsdfTranslucent", "Translucent BSDF", Shader, Shader, translucent),
    ty!("ShaderNodeBsdfTransparent", "Transparent BSDF", Shader, Shader, transparent),
    ty!("ShaderNodeVolumeAbsorption", "Volume Absorption", Shader, Shader, volume_absorption),
    ty!("ShaderNodeVolumeScatter", "Volume Scatter", Shader, Shader, volume_scatter),
    ty!("ShaderNodeTexBrick", "Brick Texture", Shader, Texture, tex_brick),
    ty!("ShaderNodeTexChecker", "Checker Texture", Shader, Texture, tex_checker),
    ty!("ShaderNodeTexEnvironment", "Environment Texture", Shader, Texture, tex_environment),
    ty!("ShaderNodeTexGabor", "Gabor Texture", Shader, Texture, tex_gabor),
    ty!("ShaderNodeTexGradient", "Gradient Texture", Shader, Texture, tex_gradient),
    ty!("ShaderNodeTexIES", "IES Texture", Shader, Texture, tex_ies),
    ty!("ShaderNodeTexImage", "Image Texture", Shader, Texture, tex_image),
    ty!("ShaderNodeTexMagic", "Magic Texture", Shader, Texture, tex_magic),
    ty!("ShaderNodeTexNoise", "Noise Texture", Shader, Texture, tex_noise),
    ty!("ShaderNodeTexSky", "Sky Texture", Shader, Texture, tex_sky),
    ty!("ShaderNodeTexVoronoi", "Voronoi Texture", Shader, Texture, tex_voronoi),
    ty!("ShaderNodeTexWave", "Wave Texture", Shader, Texture, tex_wave),
    ty!("ShaderNodeTexWhiteNoise", "White Noise Texture", Shader, Texture, tex_white_noise),
    ty!("ShaderNodeBrightContrast", "Brightness/Contrast", Shader, Color, bright_contrast),
    ty!("ShaderNodeGamma", "Gamma", Shader, Color, gamma),
    ty!("ShaderNodeHueSaturation", "Hue/Saturation/Value", Shader, Color, hue_saturation),
    ty!("ShaderNodeInvert", "Invert Color", Shader, Color, invert),
    ty!("ShaderNodeLightFalloff", "Light Falloff", Shader, Color, light_falloff),
    ty!("ShaderNodeMix", "Mix", Shader, Color, mix),
    ty!("ShaderNodeRGBCurve", "RGB Curves", Shader, Color, rgb_curve),
    ty!("ShaderNodeBump", "Bump", Shader, Vector, bump),
    ty!("ShaderNodeDisplacement", "Displacement", Shader, Vector, displacement),
    ty!("ShaderNodeMapping", "Mapping", Shader, Vector, mapping),
    ty!("ShaderNodeNormal", "Normal", Shader, Vector, normal_node),
    ty!("ShaderNodeNormalMap", "Normal Map", Shader, Vector, normal_map),
    ty!("ShaderNodeVectorCurve", "Vector Curves", Shader, Vector, vector_curve),
    ty!("ShaderNodeVectorDisplacement", "Vector Displacement", Shader, Vector, vector_displacement),
    ty!("ShaderNodeVectorRotate", "Vector Rotate", Shader, Vector, vector_rotate),
    ty!("ShaderNodeVectorTransform", "Vector Transform", Shader, Vector, vector_transform),
    ty!("ShaderNodeBlackbody", "Blackbody", Shader, Converter, blackbody),
    ty!("ShaderNodeClamp", "Clamp", Shader, Converter, clamp),
    ty!("ShaderNodeValToRGB", "Color Ramp", Shader, Converter, color_ramp),
    ty!("ShaderNodeCombineColor", "Combine Color", Shader, Converter, combine_color),
    ty!("ShaderNodeCombineXYZ", "Combine XYZ", Shader, Converter, combine_xyz),
    ty!("ShaderNodeFloatCurve", "Float Curve", Shader, Converter, float_curve),
    ty!("ShaderNodeMapRange", "Map Range", Shader, Converter, map_range),
    ty!("ShaderNodeMath", "Math", Shader, Converter, math),
    ty!("ShaderNodeRGBToBW", "RGB to BW", Shader, Converter, rgb_to_bw),
    ty!("ShaderNodeSeparateColor", "Separate Color", Shader, Converter, separate_color),
    ty!("ShaderNodeSeparateXYZ", "Separate XYZ", Shader, Converter, separate_xyz),
    ty!("ShaderNodeShaderToRGB", "Shader to RGB", Shader, Converter, shader_to_rgb),
    ty!("ShaderNodeVectorMath", "Vector Math", Shader, Converter, vector_math),
    ty!("ShaderNodeWavelength", "Wavelength", Shader, Converter, wavelength),
    ty!("ShaderNodeGroup", "Group", Shader, Group, group),
    ty!("NodeGroupInput", "Group Input", Shader, Group, group_input),
    ty!("NodeGroupOutput", "Group Output", Shader, Group, group_output),
    ty!("NodeFrame", "Frame", Shader, Layout, frame),
    ty!("NodeReroute", "Reroute", Shader, Layout, reroute),
];

/// Types Blender replaced but still reads from old files; not in the Add menu.
static LEGACY: &[NodeType] = &[
    ty!("ShaderNodeMixRGB", "Mix (Legacy)", Shader, Color, mix_rgb),
    ty!("ShaderNodeBsdfGlossy", "Glossy BSDF (Legacy)", Shader, Shader, legacy_glossy),
    ty!("ShaderNodeBsdfVelvet", "Velvet BSDF", Shader, Shader, legacy_velvet),
    ty!("ShaderNodeTexMusgrave", "Musgrave Texture", Shader, Texture, legacy_musgrave),
    ty!("ShaderNodeCombineRGB", "Combine RGB", Shader, Converter, legacy_combine_rgb),
    ty!("ShaderNodeSeparateRGB", "Separate RGB", Shader, Converter, legacy_separate_rgb),
    ty!("ShaderNodeCombineHSV", "Combine HSV", Shader, Converter, legacy_combine_hsv),
    ty!("ShaderNodeSeparateHSV", "Separate HSV", Shader, Converter, legacy_separate_hsv),
];

/// Every registered node type (the Add menu's).
pub fn node_types() -> &'static [NodeType] {
    TYPES
}

/// Deprecated types kept for reading older files.
pub fn legacy_node_types() -> &'static [NodeType] {
    LEGACY
}

pub fn node_type(idname: &str) -> Option<&'static NodeType> {
    TYPES.iter().chain(LEGACY).find(|t| t.idname == idname)
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

/// Math operations with one input (the rest use two, see [`math_inputs`]).
const MATH_UNARY: &[&str] = &[
    "SQRT", "INVERSE_SQRT", "ABSOLUTE", "EXPONENT", "SIGN", "ROUND", "FLOOR", "CEIL", "TRUNC", "FRACT", "SINE", "COSINE", "TANGENT", "SINH", "COSH", "TANH", "ARCSINE",
    "ARCCOSINE", "ARCTANGENT", "RADIANS", "DEGREES",
];

/// How many inputs a Math `operation` reads.
pub fn math_inputs(op: &str) -> usize {
    if MATH_UNARY.contains(&op) {
        1
    } else if matches!(op, "MULTIPLY_ADD" | "COMPARE" | "SMOOTH_MIN" | "SMOOTH_MAX" | "WRAP") {
        3
    } else {
        2
    }
}

/// Enable/disable sockets the way Blender's `update` callbacks do after a
/// property change (Math's operation, Noise's dimensions, Mix's data type, ...).
/// Hidden-by-user flags are left alone.
pub fn update_sockets(node: &mut Node) {
    let en = |node: &mut Node, id: &str, on: bool| {
        if let Some(s) = node.inputs.iter_mut().find(|s| s.identifier == id) {
            s.enabled = on;
        }
    };
    let en_out = |node: &mut Node, id: &str, on: bool| {
        if let Some(s) = node.outputs.iter_mut().find(|s| s.identifier == id) {
            s.enabled = on;
        }
    };
    let p = |node: &Node, k: &str| node.enum_prop(k).to_string();
    match node.idname.as_str() {
        "ShaderNodeMath" => {
            let k = math_inputs(&p(node, "operation"));
            en(node, "Value_001", k >= 2);
            en(node, "Value_002", k >= 3);
        }
        "ShaderNodeVectorMath" => {
            let op = p(node, "operation");
            let one = matches!(op.as_str(), "NORMALIZE" | "FLOOR" | "CEIL" | "FRACTION" | "ABSOLUTE" | "SINE" | "COSINE" | "TANGENT" | "LENGTH" | "SIGN");
            let three = matches!(op.as_str(), "MULTIPLY_ADD" | "FACEFORWARD" | "WRAP");
            en(node, "Vector_001", !one && op != "SCALE");
            en(node, "Vector_002", three);
            en(node, "Scale", matches!(op.as_str(), "SCALE" | "REFRACT"));
            let value = matches!(op.as_str(), "DOT_PRODUCT" | "DISTANCE" | "LENGTH");
            en_out(node, "Vector", !value);
            en_out(node, "Value", value);
        }
        "ShaderNodeMix" => {
            let dt = p(node, "data_type");
            let uniform = p(node, "factor_mode") != "NON_UNIFORM";
            en(node, "Factor_Float", dt != "VECTOR" || uniform);
            en(node, "Factor_Vector", dt == "VECTOR" && !uniform);
            for (id, t) in [("A_Float", "FLOAT"), ("B_Float", "FLOAT"), ("A_Vector", "VECTOR"), ("B_Vector", "VECTOR"), ("A_Color", "RGBA"), ("B_Color", "RGBA")] {
                en(node, id, dt == t);
            }
            for (id, t) in [("Result_Float", "FLOAT"), ("Result_Vector", "VECTOR"), ("Result_Color", "RGBA")] {
                en_out(node, id, dt == t);
            }
        }
        "ShaderNodeTexNoise" | "ShaderNodeTexWhiteNoise" | "ShaderNodeTexVoronoi" | "ShaderNodeTexMusgrave" => {
            let key = match node.idname.as_str() {
                "ShaderNodeTexVoronoi" => "voronoi_dimensions",
                "ShaderNodeTexMusgrave" => "musgrave_dimensions",
                _ => "noise_dimensions",
            };
            let d = p(node, key);
            en(node, "Vector", d != "1D");
            en(node, "W", d == "1D" || d == "4D");
            if node.idname == "ShaderNodeTexNoise" {
                let t = p(node, "noise_type");
                en(node, "Offset", matches!(t.as_str(), "RIDGED_MULTIFRACTAL" | "HYBRID_MULTIFRACTAL" | "HETERO_TERRAIN"));
                en(node, "Gain", matches!(t.as_str(), "RIDGED_MULTIFRACTAL" | "HYBRID_MULTIFRACTAL"));
            }
            if node.idname == "ShaderNodeTexVoronoi" {
                let feat = p(node, "feature");
                let metric = p(node, "distance");
                let fractal = !matches!(feat.as_str(), "N_SPHERE_RADIUS");
                en(node, "Detail", fractal);
                en(node, "Roughness", fractal);
                en(node, "Lacunarity", fractal);
                en(node, "Smoothness", feat == "SMOOTH_F1");
                en(node, "Exponent", metric == "MINKOWSKI" && d != "1D" && !matches!(feat.as_str(), "DISTANCE_TO_EDGE" | "N_SPHERE_RADIUS"));
                let cell = matches!(feat.as_str(), "F1" | "F2" | "SMOOTH_F1");
                en_out(node, "Distance", feat != "N_SPHERE_RADIUS");
                en_out(node, "Color", cell);
                en_out(node, "Position", cell && d != "1D");
                en_out(node, "W", cell && (d == "1D" || d == "4D"));
                en_out(node, "Radius", feat == "N_SPHERE_RADIUS");
            }
        }
        "ShaderNodeTexGabor" => {
            let d = p(node, "gabor_type");
            en(node, "Orientation 2D", d != "3D");
            en(node, "Orientation 3D", d == "3D");
        }
        "ShaderNodeMapRange" => {
            let v = p(node, "data_type") == "FLOAT_VECTOR";
            let stepped = p(node, "interpolation_type") == "STEPPED";
            for id in ["Value", "From Min", "From Max", "To Min", "To Max"] {
                en(node, id, !v);
            }
            en(node, "Steps", !v && stepped);
            for id in ["Vector", "From_Min_FLOAT3", "From_Max_FLOAT3", "To_Min_FLOAT3", "To_Max_FLOAT3"] {
                en(node, id, v);
            }
            en(node, "Steps_FLOAT3", v && stepped);
            en_out(node, "Result", !v);
            en_out(node, "Vector", v);
        }
        "ShaderNodeMapping" => {
            let t = p(node, "vector_type");
            en(node, "Location", matches!(t.as_str(), "POINT" | "TEXTURE"));
        }
        "ShaderNodeVectorRotate" => {
            let t = p(node, "rotation_type");
            en(node, "Axis", t == "AXIS_ANGLE");
            en(node, "Angle", t != "EULER_XYZ");
            en(node, "Rotation", t == "EULER_XYZ");
        }
        _ => {}
    }
}

/// Give a group node (or a group's Group Input/Output node) the sockets of
/// `group`'s interface, keeping the values of sockets whose names still exist.
pub fn sync_group_sockets(node: &mut Node, group: &super::NodeTree) {
    let make = |s: &super::InterfaceSocket, i: usize| {
        let mut k = Socket::new(&s.name, s.ty, s.default.clone()).ident(&format!("Socket_{i}"));
        k.min = s.min;
        k.max = s.max;
        k
    };
    let keep = |old: &[Socket], mut new: Socket| {
        if let Some(o) = old.iter().find(|o| o.name == new.name && o.ty == new.ty) {
            new.value = o.value.clone();
            new.hide = o.hide;
        }
        new
    };
    let ins: Vec<Socket> = group.inputs.iter().enumerate().map(|(i, s)| make(s, i)).collect();
    let outs: Vec<Socket> = group.outputs.iter().enumerate().map(|(i, s)| make(s, group.inputs.len() + i)).collect();
    let extend = || Socket::new("", T::Virtual, V::None).ident("__extend__");
    match node.idname.as_str() {
        "NodeGroupInput" => {
            let mut v: Vec<Socket> = ins.into_iter().map(|s| keep(&node.outputs, s)).collect();
            v.push(extend());
            node.outputs = v;
        }
        "NodeGroupOutput" => {
            let mut v: Vec<Socket> = outs.into_iter().map(|s| keep(&node.inputs, s)).collect();
            v.push(extend());
            node.inputs = v;
        }
        _ => {
            node.inputs = ins.into_iter().map(|s| keep(&node.inputs, s)).collect();
            node.outputs = outs.into_iter().map(|s| keep(&node.outputs, s)).collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_builds_with_its_idname_and_unique_identifiers() {
        for t in node_types().iter().chain(legacy_node_types()) {
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
        // idnames are unique across both lists.
        let all: Vec<&str> = node_types().iter().chain(legacy_node_types()).map(|t| t.idname).collect();
        for (i, a) in all.iter().enumerate() {
            assert!(!all[i + 1..].contains(a), "{a} registered twice");
        }
    }

    #[test]
    fn socket_availability_follows_props() {
        let mut m = new_node("ShaderNodeMath").unwrap();
        m.set_prop("operation", e("SINE"));
        update_sockets(&mut m);
        assert!(!m.inputs[1].enabled && !m.inputs[2].enabled);
        m.set_prop("operation", e("MULTIPLY_ADD"));
        update_sockets(&mut m);
        assert!(m.inputs[1].enabled && m.inputs[2].enabled);
        let n = new_node("ShaderNodeTexNoise").unwrap();
        assert!(!n.inputs[n.input("W").unwrap()].enabled);
        let menu = categories(TreeKind::Shader);
        let names: Vec<&str> = menu.iter().map(|c| c.0.label()).collect();
        assert_eq!(names, ["Input", "Output", "Shader", "Texture", "Color", "Vector", "Converter", "Group", "Layout"]);
    }
}
