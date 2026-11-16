// SPDX-License-Identifier: GPL-3.0-only
//! Geometry node types (`GeometryNodeTree`) with Blender 4.x idnames,
//! sockets, defaults and Add-menu categories. Shader math, colour and
//! texture nodes are shared with geometry trees (see `GEO_SHARED` in the
//! registry); everything specific to geometry nodes is here.
//!
//! Sockets whose type follows a `data_type` (or `input_type`) property,
//! like Store Named Attribute's Value, are retyped by [`geo_update`].
use super::registry::{Category, NodeType};
use super::{Node, Prop, Socket, SocketType as T, Subtype as S, TreeKind, Value as V};
use core::f32::consts::PI;

fn n(idname: &str, label: &str, width: f32) -> Node {
    let mut node = Node::new(idname, label);
    node.width = width;
    node
}
fn geo(name: &str) -> Socket {
    Socket::new(name, T::Geometry, V::None)
}
fn fl(name: &str, v: f32) -> Socket {
    Socket::new(name, T::Float, V::Float(v))
}
fn dist(name: &str, v: f32) -> Socket {
    fl(name, v).subtype(S::Distance)
}
fn int(name: &str, v: i32) -> Socket {
    Socket::new(name, T::Int, V::Int(v))
}
fn bo(name: &str, v: bool) -> Socket {
    Socket::new(name, T::Bool, V::Bool(v))
}
fn vec(name: &str, v: [f32; 3]) -> Socket {
    Socket::new(name, T::Vector, V::Vector(v))
}
fn xyz(name: &str, v: [f32; 3]) -> Socket {
    vec(name, v).subtype(S::Xyz)
}
fn col(name: &str, v: [f32; 4]) -> Socket {
    Socket::new(name, T::Color, V::Color(v))
}
fn rot(name: &str) -> Socket {
    Socket::new(name, T::Rotation, V::Vector([0.0; 3]))
}
fn matrix(name: &str) -> Socket {
    Socket::new(name, T::Matrix, V::None)
}
fn st(name: &str) -> Socket {
    Socket::new(name, T::String, V::String(String::new()))
}
fn obj(name: &str) -> Socket {
    Socket::new(name, T::Object, V::Id(None))
}
fn coll(name: &str) -> Socket {
    Socket::new(name, T::Collection, V::Id(None))
}
fn mat(name: &str) -> Socket {
    Socket::new(name, T::Material, V::Id(None))
}
fn menu(name: &str) -> Socket {
    Socket::new(name, T::Menu, V::String(String::new()))
}
fn sel() -> Socket {
    bo("Selection", true).hide_value()
}
fn e(s: &str) -> Prop {
    Prop::Enum(s.into())
}
fn props(x: &mut Node, p: &[(&str, Prop)]) {
    x.props = p.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
}
fn extend() -> Socket {
    Socket::new("", T::Virtual, V::None).ident("__extend__")
}

/// Socket type for a `data_type` enum value.
pub fn data_type_socket(dt: &str) -> T {
    match dt {
        "INT" | "INT8" => T::Int,
        "BOOLEAN" => T::Bool,
        "FLOAT_VECTOR" | "VECTOR" | "FLOAT2" => T::Vector,
        "FLOAT_COLOR" | "RGBA" | "BYTE_COLOR" => T::Color,
        "QUATERNION" | "ROTATION" => T::Rotation,
        "FLOAT4X4" | "MATRIX" => T::Matrix,
        "STRING" => T::String,
        "GEOMETRY" => T::Geometry,
        "OBJECT" => T::Object,
        "COLLECTION" => T::Collection,
        "MATERIAL" => T::Material,
        "IMAGE" => T::Image,
        "MENU" => T::Menu,
        _ => T::Float,
    }
}

/// A fresh socket value of type `ty`.
pub fn socket_default(ty: T) -> V {
    match ty {
        T::Float => V::Float(0.0),
        T::Int => V::Int(0),
        T::Bool => V::Bool(false),
        T::Vector | T::Rotation => V::Vector([0.0; 3]),
        T::Color => V::Color([0.8, 0.8, 0.8, 1.0]),
        T::String | T::Menu => V::String(String::new()),
        T::Object | T::Collection | T::Material | T::Image | T::Texture => V::Id(None),
        _ => V::None,
    }
}

fn retype(s: &mut Socket, ty: T) {
    if s.ty != ty {
        s.ty = ty;
        s.value = socket_default(ty);
        s.subtype = S::None;
        s.min = f32::MIN;
        s.max = f32::MAX;
    }
}

// ---------------------------------------------------------------------------
// Attribute

fn attribute_statistic() -> Node {
    let mut x = n("GeometryNodeAttributeStatistic", "Attribute Statistic", 140.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("domain", e("POINT"))]);
    x.inputs = vec![geo("Geometry"), sel(), fl("Attribute", 0.0).hide_value(), vec("Attribute", [0.0; 3]).hide_value().ident("Attribute_001")];
    x.inputs[3].enabled = false;
    let names = ["Mean", "Median", "Sum", "Min", "Max", "Range", "Standard Deviation", "Variance"];
    x.outputs = names.iter().map(|s| fl(s, 0.0)).collect();
    x.outputs.extend(names.iter().map(|s| {
        let mut k = vec(s, [0.0; 3]).ident(&format!("{s}_001"));
        k.enabled = false;
        k
    }));
    x
}

fn domain_size() -> Node {
    let mut x = n("GeometryNodeAttributeDomainSize", "Domain Size", 140.0);
    props(&mut x, &[("component", e("MESH"))]);
    x.inputs = vec![geo("Geometry")];
    x.outputs = ["Point Count", "Edge Count", "Face Count", "Face Corner Count", "Spline Count", "Instance Count", "Layer Count"].iter().map(|s| int(s, 0)).collect();
    x
}

fn blur_attribute() -> Node {
    let mut x = n("GeometryNodeBlurAttribute", "Blur Attribute", 140.0);
    props(&mut x, &[("data_type", e("FLOAT"))]);
    x.inputs = vec![fl("Value", 0.0).hide_value(), int("Iterations", 1).range(0.0, f32::MAX), fl("Weight", 1.0).subtype(S::Factor)];
    x.outputs = vec![fl("Value", 0.0)];
    x
}

fn capture_attribute() -> Node {
    let mut x = n("GeometryNodeCaptureAttribute", "Capture Attribute", 140.0);
    props(&mut x, &[("domain", e("POINT"))]);
    x.inputs = vec![geo("Geometry"), fl("Value", 0.0).hide_value(), extend()];
    x.outputs = vec![geo("Geometry"), fl("Value", 0.0), extend()];
    x
}

fn remove_attribute() -> Node {
    let mut x = n("GeometryNodeRemoveAttribute", "Remove Named Attribute", 170.0);
    props(&mut x, &[("pattern_mode", e("EXACT"))]);
    x.inputs = vec![geo("Geometry"), st("Name")];
    x.outputs = vec![geo("Geometry")];
    x
}

fn store_named_attribute() -> Node {
    let mut x = n("GeometryNodeStoreNamedAttribute", "Store Named Attribute", 140.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("domain", e("POINT"))]);
    x.inputs = vec![geo("Geometry"), sel(), st("Name"), fl("Value", 0.0)];
    x.outputs = vec![geo("Geometry")];
    x
}

fn named_attribute() -> Node {
    let mut x = n("GeometryNodeInputNamedAttribute", "Named Attribute", 140.0);
    props(&mut x, &[("data_type", e("FLOAT"))]);
    x.inputs = vec![st("Name")];
    x.outputs = vec![fl("Attribute", 0.0), bo("Exists", false)];
    x
}

// ---------------------------------------------------------------------------
// Input

fn input_bool() -> Node {
    let mut x = n("FunctionNodeInputBool", "Boolean", 140.0);
    props(&mut x, &[("boolean", Prop::Bool(false))]);
    x.outputs = vec![bo("Boolean", false)];
    x
}
fn input_color() -> Node {
    let mut x = n("FunctionNodeInputColor", "Color", 140.0);
    props(&mut x, &[("value", Prop::Color([0.5, 0.5, 0.5, 1.0]))]);
    x.outputs = vec![col("Color", [0.5, 0.5, 0.5, 1.0])];
    x
}
fn input_int() -> Node {
    let mut x = n("FunctionNodeInputInt", "Integer", 140.0);
    props(&mut x, &[("integer", Prop::Int(0))]);
    x.outputs = vec![int("Integer", 0)];
    x
}
fn input_rotation() -> Node {
    let mut x = n("FunctionNodeInputRotation", "Rotation", 140.0);
    props(&mut x, &[("rotation_euler", Prop::Vector([0.0; 3]))]);
    x.outputs = vec![rot("Rotation")];
    x
}
fn input_string() -> Node {
    let mut x = n("FunctionNodeInputString", "String", 140.0);
    props(&mut x, &[("string", Prop::String(String::new()))]);
    x.outputs = vec![st("String")];
    x
}
fn input_vector() -> Node {
    let mut x = n("FunctionNodeInputVector", "Vector", 140.0);
    props(&mut x, &[("vector", Prop::Vector([0.0; 3]))]);
    x.outputs = vec![vec("Vector", [0.0; 3])];
    x
}
fn input_material() -> Node {
    let mut x = n("GeometryNodeInputMaterial", "Material", 140.0);
    props(&mut x, &[("material", Prop::Id(None))]);
    x.outputs = vec![mat("Material")];
    x
}
fn collection_info() -> Node {
    let mut x = n("GeometryNodeCollectionInfo", "Collection Info", 140.0);
    props(&mut x, &[("transform_space", e("ORIGINAL"))]);
    x.inputs = vec![coll("Collection"), bo("Separate Children", false), bo("Reset Children", false)];
    x.outputs = vec![geo("Instances")];
    x
}
fn object_info() -> Node {
    let mut x = n("GeometryNodeObjectInfo", "Object Info", 140.0);
    props(&mut x, &[("transform_space", e("ORIGINAL"))]);
    x.inputs = vec![obj("Object"), bo("As Instance", false)];
    x.outputs = vec![matrix("Transform"), vec("Location", [0.0; 3]), rot("Rotation"), vec("Scale", [0.0; 3]), geo("Geometry")];
    x
}
fn simple_out(idname: &str, label: &str, outs: Vec<Socket>) -> Node {
    let mut x = n(idname, label, 140.0);
    x.outputs = outs;
    x
}
fn self_object() -> Node {
    simple_out("GeometryNodeSelfObject", "Self Object", vec![obj("Self Object")])
}
fn is_viewport() -> Node {
    simple_out("GeometryNodeIsViewport", "Is Viewport", vec![bo("Is Viewport", false)])
}
fn scene_time() -> Node {
    simple_out("GeometryNodeInputSceneTime", "Scene Time", vec![fl("Seconds", 0.0), fl("Frame", 0.0)])
}
fn input_id() -> Node {
    simple_out("GeometryNodeInputID", "ID", vec![int("ID", 0)])
}
fn input_index() -> Node {
    simple_out("GeometryNodeInputIndex", "Index", vec![int("Index", 0)])
}
fn input_normal() -> Node {
    simple_out("GeometryNodeInputNormal", "Normal", vec![vec("Normal", [0.0; 3])])
}
fn input_position() -> Node {
    simple_out("GeometryNodeInputPosition", "Position", vec![vec("Position", [0.0; 3])])
}
fn input_radius() -> Node {
    simple_out("GeometryNodeInputRadius", "Radius", vec![fl("Radius", 0.0)])
}
fn active_camera() -> Node {
    simple_out("GeometryNodeInputActiveCamera", "Active Camera", vec![obj("Active Camera")])
}
fn material_index() -> Node {
    simple_out("GeometryNodeInputMaterialIndex", "Material Index", vec![int("Material Index", 0)])
}
fn shade_smooth_in() -> Node {
    simple_out("GeometryNodeInputShadeSmooth", "Is Face Smooth", vec![bo("Smooth", false)])
}
fn edge_smooth_in() -> Node {
    simple_out("GeometryNodeInputEdgeSmooth", "Is Edge Smooth", vec![bo("Smooth", false)])
}

// ---------------------------------------------------------------------------
// Output

fn viewer() -> Node {
    let mut x = n("GeometryNodeViewer", "Viewer", 140.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("domain", e("AUTO"))]);
    x.inputs = vec![geo("Geometry"), fl("Value", 0.0).hide_value()];
    x
}

// ---------------------------------------------------------------------------
// Geometry

fn bound_box() -> Node {
    let mut x = n("GeometryNodeBoundBox", "Bounding Box", 140.0);
    x.inputs = vec![geo("Geometry")];
    x.outputs = vec![geo("Bounding Box"), vec("Min", [0.0; 3]), vec("Max", [0.0; 3])];
    x
}
fn convex_hull() -> Node {
    let mut x = n("GeometryNodeConvexHull", "Convex Hull", 140.0);
    x.inputs = vec![geo("Geometry")];
    x.outputs = vec![geo("Convex Hull")];
    x
}
fn delete_geometry() -> Node {
    let mut x = n("GeometryNodeDeleteGeometry", "Delete Geometry", 140.0);
    props(&mut x, &[("domain", e("POINT")), ("mode", e("ALL"))]);
    x.inputs = vec![geo("Geometry"), sel()];
    x.inputs[1].value = V::Bool(true);
    x.outputs = vec![geo("Geometry")];
    x
}
fn duplicate_elements() -> Node {
    let mut x = n("GeometryNodeDuplicateElements", "Duplicate Elements", 140.0);
    props(&mut x, &[("domain", e("POINT"))]);
    x.inputs = vec![geo("Geometry"), sel(), int("Amount", 1).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Geometry"), int("Duplicate Index", 0)];
    x
}
fn merge_by_distance() -> Node {
    let mut x = n("GeometryNodeMergeByDistance", "Merge by Distance", 140.0);
    props(&mut x, &[("mode", e("ALL"))]);
    x.inputs = vec![geo("Geometry"), sel(), dist("Distance", 0.001).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Geometry")];
    x
}
fn transform_geometry() -> Node {
    let mut x = n("GeometryNodeTransform", "Transform Geometry", 140.0);
    props(&mut x, &[("mode", e("COMPONENTS"))]);
    x.inputs = vec![geo("Geometry"), vec("Translation", [0.0; 3]).subtype(S::Translation), rot("Rotation"), xyz("Scale", [1.0; 3]), matrix("Transform")];
    x.inputs[4].enabled = false;
    x.outputs = vec![geo("Geometry")];
    x
}
fn join_geometry() -> Node {
    let mut x = n("GeometryNodeJoinGeometry", "Join Geometry", 140.0);
    x.inputs = vec![geo("Geometry")];
    x.outputs = vec![geo("Geometry")];
    x
}
fn separate_components() -> Node {
    let mut x = n("GeometryNodeSeparateComponents", "Separate Components", 140.0);
    x.inputs = vec![geo("Geometry")];
    x.outputs = vec![geo("Mesh"), geo("Curve"), geo("Grease Pencil"), geo("Point Cloud"), geo("Volume"), geo("Instances")];
    x
}
fn separate_geometry() -> Node {
    let mut x = n("GeometryNodeSeparateGeometry", "Separate Geometry", 140.0);
    props(&mut x, &[("domain", e("POINT"))]);
    x.inputs = vec![geo("Geometry"), sel()];
    x.outputs = vec![geo("Selection"), geo("Inverted")];
    x
}
fn set_position() -> Node {
    let mut x = n("GeometryNodeSetPosition", "Set Position", 140.0);
    x.inputs = vec![geo("Geometry"), sel(), vec("Position", [0.0; 3]).hide_value(), vec("Offset", [0.0; 3]).subtype(S::Translation)];
    x.outputs = vec![geo("Geometry")];
    x
}
fn set_id() -> Node {
    let mut x = n("GeometryNodeSetID", "Set ID", 140.0);
    x.inputs = vec![geo("Geometry"), sel(), int("ID", 0).hide_value()];
    x.outputs = vec![geo("Geometry")];
    x
}
fn geometry_to_instance() -> Node {
    let mut x = n("GeometryNodeGeometryToInstance", "Geometry to Instance", 160.0);
    x.inputs = vec![geo("Geometry")];
    x.outputs = vec![geo("Instances")];
    x
}
fn sort_elements() -> Node {
    let mut x = n("GeometryNodeSortElements", "Sort Elements", 140.0);
    props(&mut x, &[("domain", e("POINT"))]);
    x.inputs = vec![geo("Geometry"), sel(), int("Group ID", 0).hide_value(), fl("Sort Key", 0.0).hide_value()];
    x.outputs = vec![geo("Geometry")];
    x
}
fn raycast() -> Node {
    let mut x = n("GeometryNodeRaycast", "Raycast", 150.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("mapping", e("INTERPOLATED"))]);
    x.inputs = vec![geo("Target Geometry"), fl("Attribute", 0.0).hide_value(), vec("Source Position", [0.0; 3]).hide_value(), vec("Ray Direction", [0.0, 0.0, -1.0]), dist("Ray Length", 100.0).range(0.0, f32::MAX)];
    x.outputs = vec![bo("Is Hit", false), vec("Hit Position", [0.0; 3]), vec("Hit Normal", [0.0; 3]), fl("Hit Distance", 0.0), fl("Attribute", 0.0)];
    x
}
fn sample_index() -> Node {
    let mut x = n("GeometryNodeSampleIndex", "Sample Index", 140.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("domain", e("POINT")), ("clamp", Prop::Bool(false))]);
    x.inputs = vec![geo("Geometry"), fl("Value", 0.0).hide_value(), int("Index", 0)];
    x.outputs = vec![fl("Value", 0.0)];
    x
}
fn sample_nearest() -> Node {
    let mut x = n("GeometryNodeSampleNearest", "Sample Nearest", 140.0);
    props(&mut x, &[("domain", e("POINT"))]);
    x.inputs = vec![geo("Geometry"), vec("Sample Position", [0.0; 3]).hide_value()];
    x.outputs = vec![int("Index", 0)];
    x
}
fn sample_nearest_surface() -> Node {
    let mut x = n("GeometryNodeSampleNearestSurface", "Sample Nearest Surface", 160.0);
    props(&mut x, &[("data_type", e("FLOAT"))]);
    x.inputs = vec![geo("Mesh"), fl("Value", 0.0).hide_value(), int("Group ID", 0).hide_value(), vec("Sample Position", [0.0; 3]).hide_value(), int("Sample Group ID", 0).hide_value()];
    x.outputs = vec![fl("Value", 0.0), bo("Is Valid", false)];
    x
}
fn proximity() -> Node {
    let mut x = n("GeometryNodeProximity", "Geometry Proximity", 140.0);
    props(&mut x, &[("target_element", e("FACES"))]);
    x.inputs = vec![geo("Target"), int("Group ID", 0).hide_value(), vec("Source Position", [0.0; 3]).hide_value(), int("Sample Group ID", 0).hide_value()];
    x.outputs = vec![vec("Position", [0.0; 3]), fl("Distance", 0.0), bo("Is Valid", false)];
    x
}

// ---------------------------------------------------------------------------
// Curve

fn curve_circle() -> Node {
    let mut x = n("GeometryNodeCurvePrimitiveCircle", "Curve Circle", 140.0);
    props(&mut x, &[("mode", e("RADIUS"))]);
    x.inputs = vec![int("Resolution", 32).range(3.0, 512.0), vec("Point 1", [-1.0, 0.0, 0.0]), vec("Point 2", [0.0, 1.0, 0.0]), vec("Point 3", [1.0, 0.0, 0.0]), dist("Radius", 1.0).range(0.0, f32::MAX)];
    for k in 1..4 {
        x.inputs[k].enabled = false;
    }
    x.outputs = vec![geo("Curve"), vec("Center", [0.0; 3])];
    x.outputs[1].enabled = false;
    x
}
fn curve_line() -> Node {
    let mut x = n("GeometryNodeCurvePrimitiveLine", "Curve Line", 140.0);
    props(&mut x, &[("mode", e("POINTS"))]);
    x.inputs = vec![vec("Start", [0.0; 3]).subtype(S::Translation), vec("End", [0.0, 0.0, 1.0]).subtype(S::Translation), vec("Direction", [0.0, 0.0, 1.0]), dist("Length", 1.0)];
    x.inputs[2].enabled = false;
    x.inputs[3].enabled = false;
    x.outputs = vec![geo("Curve")];
    x
}
fn curve_arc() -> Node {
    let mut x = n("GeometryNodeCurveArc", "Arc", 140.0);
    props(&mut x, &[("mode", e("RADIUS"))]);
    x.inputs = vec![
        int("Resolution", 16).range(2.0, 256.0),
        vec("Start", [-1.0, 0.0, 0.0]),
        vec("Middle", [0.0, 2.0, 0.0]),
        vec("End", [1.0, 0.0, 0.0]),
        dist("Radius", 1.0).range(0.0, f32::MAX),
        fl("Start Angle", 0.0).subtype(S::Angle),
        fl("Sweep Angle", 7.0 * PI / 4.0).subtype(S::Angle),
        fl("Offset Angle", 0.0).subtype(S::Angle),
        bo("Connect Center", false),
        bo("Invert Arc", false),
    ];
    for k in [1, 2, 3, 7] {
        x.inputs[k].enabled = false;
    }
    x.outputs = vec![geo("Curve"), vec("Center", [0.0; 3]), vec("Normal", [0.0; 3]), fl("Radius", 0.0)];
    for k in 1..4 {
        x.outputs[k].enabled = false;
    }
    x
}
fn curve_spiral() -> Node {
    let mut x = n("GeometryNodeCurveSpiral", "Spiral", 140.0);
    x.inputs = vec![int("Resolution", 32).range(1.0, 1024.0), fl("Rotations", 2.0), dist("Start Radius", 1.0), dist("End Radius", 2.0), dist("Height", 2.0), bo("Reverse", false)];
    x.outputs = vec![geo("Curve")];
    x
}
fn curve_star() -> Node {
    let mut x = n("GeometryNodeCurveStar", "Star", 140.0);
    x.inputs = vec![int("Points", 8).range(3.0, 256.0), dist("Inner Radius", 1.0).range(0.0, f32::MAX), dist("Outer Radius", 2.0).range(0.0, f32::MAX), fl("Twist", 0.0).subtype(S::Angle)];
    x.outputs = vec![geo("Curve"), bo("Outer Points", false)];
    x
}
fn curve_quadrilateral() -> Node {
    let mut x = n("GeometryNodeCurvePrimitiveQuadrilateral", "Quadrilateral", 140.0);
    props(&mut x, &[("mode", e("RECTANGLE"))]);
    x.inputs = vec![
        dist("Width", 2.0).range(0.0, f32::MAX),
        dist("Height", 2.0).range(0.0, f32::MAX),
        dist("Bottom Width", 4.0).range(0.0, f32::MAX),
        dist("Top Width", 2.0).range(0.0, f32::MAX),
        dist("Offset", 1.0),
        dist("Bottom Height", 3.0).range(0.0, f32::MAX),
        dist("Top Height", 1.0),
        vec("Point 1", [-1.0, -1.0, 0.0]),
        vec("Point 2", [1.0, -1.0, 0.0]),
        vec("Point 3", [1.0, 1.0, 0.0]),
        vec("Point 4", [-1.0, 1.0, 0.0]),
    ];
    for k in 2..11 {
        x.inputs[k].enabled = false;
    }
    x.outputs = vec![geo("Curve")];
    x
}
fn bezier_segment() -> Node {
    let mut x = n("GeometryNodeCurvePrimitiveBezierSegment", "Bezier Segment", 140.0);
    props(&mut x, &[("mode", e("POSITION"))]);
    x.inputs = vec![int("Resolution", 16).range(1.0, 256.0), vec("Start", [-1.0, 0.0, 0.0]), vec("Start Handle", [-0.5, 0.5, 0.0]), vec("End Handle", [0.0; 3]), vec("End", [1.0, 0.0, 0.0])];
    x.outputs = vec![geo("Curve")];
    x
}
fn quadratic_bezier() -> Node {
    let mut x = n("GeometryNodeCurveQuadraticBezier", "Quadratic Bezier", 140.0);
    x.inputs = vec![int("Resolution", 16).range(1.0, 256.0), vec("Start", [-1.0, 0.0, 0.0]), vec("Middle", [0.0, 2.0, 0.0]), vec("End", [1.0, 0.0, 0.0])];
    x.outputs = vec![geo("Curve")];
    x
}
fn curve_length() -> Node {
    let mut x = n("GeometryNodeCurveLength", "Curve Length", 140.0);
    x.inputs = vec![geo("Curve")];
    x.outputs = vec![fl("Length", 0.0)];
    x
}
fn curve_to_mesh() -> Node {
    let mut x = n("GeometryNodeCurveToMesh", "Curve to Mesh", 140.0);
    x.inputs = vec![geo("Curve"), geo("Profile Curve"), fl("Scale", 1.0).hide_value(), bo("Fill Caps", false)];
    x.outputs = vec![geo("Mesh")];
    x
}
fn curve_to_points() -> Node {
    let mut x = n("GeometryNodeCurveToPoints", "Curve to Points", 140.0);
    props(&mut x, &[("mode", e("COUNT"))]);
    x.inputs = vec![geo("Curve"), int("Count", 10).range(2.0, 100_000.0), dist("Length", 0.1).range(0.001, f32::MAX)];
    x.inputs[2].enabled = false;
    x.outputs = vec![geo("Points"), vec("Tangent", [0.0; 3]), vec("Normal", [0.0; 3]), rot("Rotation")];
    x
}
fn fill_curve() -> Node {
    let mut x = n("GeometryNodeFillCurve", "Fill Curve", 140.0);
    props(&mut x, &[("mode", e("TRIANGLES"))]);
    x.inputs = vec![geo("Curve"), int("Group ID", 0).hide_value()];
    x.outputs = vec![geo("Mesh")];
    x
}
fn fillet_curve() -> Node {
    let mut x = n("GeometryNodeFilletCurve", "Fillet Curve", 140.0);
    props(&mut x, &[("mode", e("BEZIER"))]);
    x.inputs = vec![geo("Curve"), int("Count", 1).range(1.0, 1000.0), dist("Radius", 0.25).range(0.0, f32::MAX), bo("Limit Radius", false)];
    x.inputs[1].enabled = false;
    x.outputs = vec![geo("Curve")];
    x
}
fn resample_curve() -> Node {
    let mut x = n("GeometryNodeResampleCurve", "Resample Curve", 140.0);
    props(&mut x, &[("mode", e("COUNT"))]);
    x.inputs = vec![geo("Curve"), sel(), int("Count", 10).range(1.0, 100_000.0), dist("Length", 0.1).range(0.01, f32::MAX)];
    x.inputs[3].enabled = false;
    x.outputs = vec![geo("Curve")];
    x
}
fn reverse_curve() -> Node {
    let mut x = n("GeometryNodeReverseCurve", "Reverse Curve", 140.0);
    x.inputs = vec![geo("Curve"), sel()];
    x.outputs = vec![geo("Curve")];
    x
}
fn subdivide_curve() -> Node {
    let mut x = n("GeometryNodeSubdivideCurve", "Subdivide Curve", 140.0);
    x.inputs = vec![geo("Curve"), int("Cuts", 1).range(0.0, 1000.0)];
    x.outputs = vec![geo("Curve")];
    x
}
fn trim_curve() -> Node {
    let mut x = n("GeometryNodeTrimCurve", "Trim Curve", 140.0);
    props(&mut x, &[("mode", e("FACTOR"))]);
    x.inputs = vec![geo("Curve"), sel(), fl("Start", 0.0).subtype(S::Factor), fl("End", 1.0).subtype(S::Factor), dist("Start", 0.0).ident("Start_001").range(0.0, f32::MAX), dist("End", 1.0).ident("End_001").range(0.0, f32::MAX)];
    x.inputs[4].enabled = false;
    x.inputs[5].enabled = false;
    x.outputs = vec![geo("Curve")];
    x
}
fn set_curve_radius() -> Node {
    let mut x = n("GeometryNodeSetCurveRadius", "Set Curve Radius", 140.0);
    x.inputs = vec![geo("Curve"), sel(), dist("Radius", 0.005).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Curve")];
    x
}
fn set_curve_tilt() -> Node {
    let mut x = n("GeometryNodeSetCurveTilt", "Set Curve Tilt", 140.0);
    x.inputs = vec![geo("Curve"), sel(), fl("Tilt", 0.0).subtype(S::Angle)];
    x.outputs = vec![geo("Curve")];
    x
}
fn set_handle_positions() -> Node {
    let mut x = n("GeometryNodeSetCurveHandlePositions", "Set Handle Positions", 160.0);
    props(&mut x, &[("mode", e("LEFT"))]);
    x.inputs = vec![geo("Curve"), sel(), vec("Position", [0.0; 3]).hide_value(), vec("Offset", [0.0; 3]).subtype(S::Translation)];
    x.outputs = vec![geo("Curve")];
    x
}
fn set_handle_type() -> Node {
    let mut x = n("GeometryNodeCurveSetHandles", "Set Handle Type", 140.0);
    props(&mut x, &[("handle_type", e("AUTO")), ("mode", e("LEFT_RIGHT"))]);
    x.inputs = vec![geo("Curve"), sel()];
    x.outputs = vec![geo("Curve")];
    x
}
fn set_spline_type() -> Node {
    let mut x = n("GeometryNodeCurveSplineType", "Set Spline Type", 140.0);
    props(&mut x, &[("spline_type", e("POLY"))]);
    x.inputs = vec![geo("Curve"), sel()];
    x.outputs = vec![geo("Curve")];
    x
}
fn set_spline_cyclic() -> Node {
    let mut x = n("GeometryNodeSetSplineCyclic", "Set Spline Cyclic", 140.0);
    x.inputs = vec![geo("Geometry"), sel(), bo("Cyclic", false)];
    x.outputs = vec![geo("Geometry")];
    x
}
fn set_spline_resolution() -> Node {
    let mut x = n("GeometryNodeSetSplineResolution", "Set Spline Resolution", 160.0);
    x.inputs = vec![geo("Geometry"), sel(), int("Resolution", 12)];
    x.outputs = vec![geo("Geometry")];
    x
}
fn spline_parameter() -> Node {
    simple_out("GeometryNodeSplineParameter", "Spline Parameter", vec![fl("Factor", 0.0), fl("Length", 0.0), int("Index", 0)])
}
fn spline_length() -> Node {
    simple_out("GeometryNodeSplineLength", "Spline Length", vec![fl("Length", 0.0), int("Point Count", 0)])
}
fn curve_tangent() -> Node {
    simple_out("GeometryNodeInputTangent", "Curve Tangent", vec![vec("Tangent", [0.0; 3])])
}
fn curve_tilt() -> Node {
    simple_out("GeometryNodeInputCurveTilt", "Curve Tilt", vec![fl("Tilt", 0.0)])
}
fn handle_positions() -> Node {
    let mut x = n("GeometryNodeInputCurveHandlePositions", "Curve Handle Positions", 160.0);
    x.inputs = vec![bo("Relative", false)];
    x.outputs = vec![vec("Left", [0.0; 3]), vec("Right", [0.0; 3])];
    x
}
fn spline_cyclic() -> Node {
    simple_out("GeometryNodeInputSplineCyclic", "Is Spline Cyclic", vec![bo("Cyclic", false)])
}
fn spline_resolution() -> Node {
    simple_out("GeometryNodeInputSplineResolution", "Spline Resolution", vec![int("Resolution", 0)])
}
fn endpoint_selection() -> Node {
    let mut x = n("GeometryNodeCurveEndpointSelection", "Endpoint Selection", 140.0);
    x.inputs = vec![int("Start Size", 1).range(0.0, f32::MAX), int("End Size", 1).range(0.0, f32::MAX)];
    x.outputs = vec![bo("Selection", false)];
    x
}
fn sample_curve() -> Node {
    let mut x = n("GeometryNodeSampleCurve", "Sample Curve", 140.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("mode", e("FACTOR")), ("use_all_curves", Prop::Bool(false))]);
    x.inputs = vec![geo("Curves"), fl("Value", 0.0).hide_value(), fl("Factor", 0.0).subtype(S::Factor), dist("Length", 0.0), int("Curve Index", 0)];
    x.inputs[3].enabled = false;
    x.outputs = vec![fl("Value", 0.0), vec("Position", [0.0; 3]), vec("Tangent", [0.0; 3]), vec("Normal", [0.0; 3])];
    x
}
fn curve_of_point() -> Node {
    let mut x = n("GeometryNodeCurveOfPoint", "Curve of Point", 140.0);
    x.inputs = vec![int("Point Index", 0).hide_value()];
    x.outputs = vec![int("Curve Index", 0), int("Index in Curve", 0)];
    x
}
fn points_of_curve() -> Node {
    let mut x = n("GeometryNodePointsOfCurve", "Points of Curve", 140.0);
    x.inputs = vec![int("Curve Index", 0).hide_value(), fl("Weights", 0.0).hide_value(), int("Sort Index", 0)];
    x.outputs = vec![int("Point Index", 0), int("Total", 0)];
    x
}
fn offset_point_in_curve() -> Node {
    let mut x = n("GeometryNodeOffsetPointInCurve", "Offset Point in Curve", 160.0);
    x.inputs = vec![int("Point Index", 0).hide_value(), int("Offset", 0)];
    x.outputs = vec![bo("Is Valid Offset", false), int("Point Index", 0)];
    x
}

// ---------------------------------------------------------------------------
// Instances

fn instance_on_points() -> Node {
    let mut x = n("GeometryNodeInstanceOnPoints", "Instance on Points", 140.0);
    x.inputs = vec![geo("Points"), sel(), geo("Instance"), bo("Pick Instance", false), int("Instance Index", 0).hide_value(), rot("Rotation"), xyz("Scale", [1.0; 3])];
    x.outputs = vec![geo("Instances")];
    x
}
fn realize_instances() -> Node {
    let mut x = n("GeometryNodeRealizeInstances", "Realize Instances", 140.0);
    x.inputs = vec![geo("Geometry"), sel(), bo("Realize All", true), int("Depth", 0)];
    x.outputs = vec![geo("Geometry")];
    x
}
fn rotate_instances() -> Node {
    let mut x = n("GeometryNodeRotateInstances", "Rotate Instances", 140.0);
    x.inputs = vec![geo("Instances"), sel(), rot("Rotation"), vec("Pivot Point", [0.0; 3]).subtype(S::Translation), bo("Local Space", true)];
    x.outputs = vec![geo("Instances")];
    x
}
fn scale_instances() -> Node {
    let mut x = n("GeometryNodeScaleInstances", "Scale Instances", 140.0);
    x.inputs = vec![geo("Instances"), sel(), xyz("Scale", [1.0; 3]), vec("Center", [0.0; 3]).subtype(S::Translation), bo("Local Space", true)];
    x.outputs = vec![geo("Instances")];
    x
}
fn translate_instances() -> Node {
    let mut x = n("GeometryNodeTranslateInstances", "Translate Instances", 140.0);
    x.inputs = vec![geo("Instances"), sel(), vec("Translation", [0.0; 3]).subtype(S::Translation), bo("Local Space", true)];
    x.outputs = vec![geo("Instances")];
    x
}
fn instances_to_points() -> Node {
    let mut x = n("GeometryNodeInstancesToPoints", "Instances to Points", 140.0);
    x.inputs = vec![geo("Instances"), sel(), vec("Position", [0.0; 3]).hide_value(), dist("Radius", 0.05).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Points")];
    x
}
fn instance_rotation() -> Node {
    simple_out("GeometryNodeInputInstanceRotation", "Instance Rotation", vec![rot("Rotation")])
}
fn instance_scale() -> Node {
    simple_out("GeometryNodeInputInstanceScale", "Instance Scale", vec![vec("Scale", [0.0; 3])])
}
fn instance_transform() -> Node {
    simple_out("GeometryNodeInstanceTransform", "Instance Transform", vec![matrix("Transform")])
}

// ---------------------------------------------------------------------------
// Mesh

fn dual_mesh() -> Node {
    let mut x = n("GeometryNodeDualMesh", "Dual Mesh", 140.0);
    x.inputs = vec![geo("Mesh"), bo("Keep Boundaries", false)];
    x.outputs = vec![geo("Dual Mesh")];
    x
}
fn extrude_mesh() -> Node {
    let mut x = n("GeometryNodeExtrudeMesh", "Extrude Mesh", 140.0);
    props(&mut x, &[("mode", e("FACES"))]);
    x.inputs = vec![geo("Mesh"), sel(), vec("Offset", [0.0; 3]).subtype(S::Translation).hide_value(), fl("Offset Scale", 1.0), bo("Individual", true)];
    x.outputs = vec![geo("Mesh"), bo("Top", false), bo("Side", false)];
    x
}
fn flip_faces() -> Node {
    let mut x = n("GeometryNodeFlipFaces", "Flip Faces", 140.0);
    x.inputs = vec![geo("Mesh"), sel()];
    x.outputs = vec![geo("Mesh")];
    x
}
fn mesh_boolean() -> Node {
    let mut x = n("GeometryNodeMeshBoolean", "Mesh Boolean", 140.0);
    props(&mut x, &[("operation", e("DIFFERENCE")), ("solver", e("EXACT"))]);
    x.inputs = vec![geo("Mesh 1"), geo("Mesh 2"), bo("Self Intersection", false), bo("Hole Tolerant", false)];
    x.outputs = vec![geo("Mesh"), bo("Intersecting Edges", false)];
    x
}
fn mesh_to_curve() -> Node {
    let mut x = n("GeometryNodeMeshToCurve", "Mesh to Curve", 140.0);
    x.inputs = vec![geo("Mesh"), sel()];
    x.outputs = vec![geo("Curve")];
    x
}
fn mesh_to_points() -> Node {
    let mut x = n("GeometryNodeMeshToPoints", "Mesh to Points", 140.0);
    props(&mut x, &[("mode", e("VERTICES"))]);
    x.inputs = vec![geo("Mesh"), sel(), vec("Position", [0.0; 3]).hide_value(), dist("Radius", 0.05).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Points")];
    x
}
fn scale_elements() -> Node {
    let mut x = n("GeometryNodeScaleElements", "Scale Elements", 140.0);
    props(&mut x, &[("domain", e("FACE")), ("scale_mode", e("UNIFORM"))]);
    x.inputs = vec![geo("Geometry"), sel(), fl("Scale", 1.0), vec("Center", [0.0; 3]).subtype(S::Translation).hide_value(), vec("Axis", [1.0, 0.0, 0.0])];
    x.inputs[4].enabled = false;
    x.outputs = vec![geo("Geometry")];
    x
}
fn split_edges() -> Node {
    let mut x = n("GeometryNodeSplitEdges", "Split Edges", 140.0);
    x.inputs = vec![geo("Mesh"), sel()];
    x.outputs = vec![geo("Mesh")];
    x
}
fn subdivide_mesh() -> Node {
    let mut x = n("GeometryNodeSubdivideMesh", "Subdivide Mesh", 140.0);
    x.inputs = vec![geo("Mesh"), int("Level", 1).range(0.0, 6.0)];
    x.outputs = vec![geo("Mesh")];
    x
}
fn subdivision_surface() -> Node {
    let mut x = n("GeometryNodeSubdivisionSurface", "Subdivision Surface", 150.0);
    props(&mut x, &[("uv_smooth", e("PRESERVE_BOUNDARIES")), ("boundary_smooth", e("ALL"))]);
    x.inputs = vec![geo("Mesh"), int("Level", 1).range(0.0, 6.0), fl("Edge Crease", 0.0).subtype(S::Factor).hide_value(), fl("Vertex Crease", 0.0).subtype(S::Factor).hide_value()];
    x.outputs = vec![geo("Mesh")];
    x
}
fn triangulate() -> Node {
    let mut x = n("GeometryNodeTriangulate", "Triangulate", 140.0);
    props(&mut x, &[("quad_method", e("SHORTEST_DIAGONAL")), ("ngon_method", e("BEAUTY"))]);
    x.inputs = vec![geo("Mesh"), sel(), int("Minimum Vertices", 4).range(4.0, 10000.0)];
    x.outputs = vec![geo("Mesh")];
    x
}
fn set_shade_smooth() -> Node {
    let mut x = n("GeometryNodeSetShadeSmooth", "Set Shade Smooth", 140.0);
    props(&mut x, &[("domain", e("FACE"))]);
    x.inputs = vec![geo("Geometry"), sel(), bo("Shade Smooth", true)];
    x.outputs = vec![geo("Geometry")];
    x
}
fn mesh_cone() -> Node {
    let mut x = n("GeometryNodeMeshCone", "Cone", 140.0);
    props(&mut x, &[("fill_type", e("NGON"))]);
    x.inputs = vec![int("Vertices", 32).range(3.0, 512.0), int("Side Segments", 1).range(1.0, 512.0), int("Fill Segments", 1).range(1.0, 512.0), dist("Radius Top", 0.0).range(0.0, f32::MAX), dist("Radius Bottom", 1.0).range(0.0, f32::MAX), dist("Depth", 2.0).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Mesh"), bo("Top", false), bo("Bottom", false), bo("Side", false), vec("UV Map", [0.0; 3])];
    x
}
fn mesh_cube() -> Node {
    let mut x = n("GeometryNodeMeshCube", "Cube", 140.0);
    x.inputs = vec![xyz("Size", [1.0; 3]).subtype(S::Translation), int("Vertices X", 2).range(2.0, 1000.0), int("Vertices Y", 2).range(2.0, 1000.0), int("Vertices Z", 2).range(2.0, 1000.0)];
    x.outputs = vec![geo("Mesh"), vec("UV Map", [0.0; 3])];
    x
}
fn mesh_cylinder() -> Node {
    let mut x = n("GeometryNodeMeshCylinder", "Cylinder", 140.0);
    props(&mut x, &[("fill_type", e("NGON"))]);
    x.inputs = vec![int("Vertices", 32).range(3.0, 512.0), int("Side Segments", 1).range(1.0, 512.0), int("Fill Segments", 1).range(1.0, 512.0), dist("Radius", 1.0).range(0.0, f32::MAX), dist("Depth", 2.0).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Mesh"), bo("Top", false), bo("Side", false), bo("Bottom", false), vec("UV Map", [0.0; 3])];
    x
}
fn mesh_grid() -> Node {
    let mut x = n("GeometryNodeMeshGrid", "Grid", 140.0);
    x.inputs = vec![dist("Size X", 1.0).range(0.0, f32::MAX), dist("Size Y", 1.0).range(0.0, f32::MAX), int("Vertices X", 3).range(2.0, 1000.0), int("Vertices Y", 3).range(2.0, 1000.0)];
    x.outputs = vec![geo("Mesh"), vec("UV Map", [0.0; 3])];
    x
}
fn mesh_ico_sphere() -> Node {
    let mut x = n("GeometryNodeMeshIcoSphere", "Ico Sphere", 140.0);
    x.inputs = vec![dist("Radius", 1.0).range(0.0, f32::MAX), int("Subdivisions", 1).range(1.0, 10.0)];
    x.outputs = vec![geo("Mesh"), vec("UV Map", [0.0; 3])];
    x
}
fn mesh_circle() -> Node {
    let mut x = n("GeometryNodeMeshCircle", "Mesh Circle", 140.0);
    props(&mut x, &[("fill_type", e("NONE"))]);
    x.inputs = vec![int("Vertices", 32).range(3.0, f32::MAX), dist("Radius", 1.0).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Mesh")];
    x
}
fn mesh_line() -> Node {
    let mut x = n("GeometryNodeMeshLine", "Mesh Line", 140.0);
    props(&mut x, &[("mode", e("OFFSET")), ("count_mode", e("TOTAL"))]);
    x.inputs = vec![int("Count", 10).range(1.0, 10000.0), dist("Resolution", 1.0).range(0.001, f32::MAX), vec("Start Location", [0.0; 3]).subtype(S::Translation), vec("Offset", [0.0, 0.0, 1.0]).subtype(S::Translation)];
    x.inputs[1].enabled = false;
    x.outputs = vec![geo("Mesh")];
    x
}
fn mesh_uv_sphere() -> Node {
    let mut x = n("GeometryNodeMeshUVSphere", "UV Sphere", 140.0);
    x.inputs = vec![int("Segments", 32).range(3.0, 1024.0), int("Rings", 16).range(2.0, 1024.0), dist("Radius", 1.0).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Mesh"), vec("UV Map", [0.0; 3])];
    x
}
fn edge_angle() -> Node {
    simple_out("GeometryNodeInputMeshEdgeAngle", "Edge Angle", vec![fl("Unsigned Angle", 0.0), fl("Signed Angle", 0.0)])
}
fn edge_neighbors() -> Node {
    simple_out("GeometryNodeInputMeshEdgeNeighbors", "Edge Neighbors", vec![int("Face Count", 0)])
}
fn edge_vertices() -> Node {
    simple_out("GeometryNodeInputMeshEdgeVertices", "Edge Vertices", vec![int("Vertex Index 1", 0), int("Vertex Index 2", 0), vec("Position 1", [0.0; 3]), vec("Position 2", [0.0; 3])])
}
fn face_area() -> Node {
    simple_out("GeometryNodeInputMeshFaceArea", "Face Area", vec![fl("Area", 0.0)])
}
fn face_neighbors() -> Node {
    simple_out("GeometryNodeInputMeshFaceNeighbors", "Face Neighbors", vec![int("Vertex Count", 0), int("Face Count", 0)])
}
fn face_is_planar() -> Node {
    let mut x = n("GeometryNodeInputMeshFaceIsPlanar", "Is Face Planar", 140.0);
    x.inputs = vec![dist("Threshold", 0.01).range(0.0, f32::MAX)];
    x.outputs = vec![bo("Planar", false)];
    x
}
fn mesh_island() -> Node {
    simple_out("GeometryNodeInputMeshIsland", "Mesh Island", vec![int("Island Index", 0), int("Island Count", 0)])
}
fn vertex_neighbors() -> Node {
    simple_out("GeometryNodeInputMeshVertexNeighbors", "Vertex Neighbors", vec![int("Vertex Count", 0), int("Face Count", 0)])
}
fn shortest_edge_paths() -> Node {
    let mut x = n("GeometryNodeInputShortestEdgePaths", "Shortest Edge Paths", 140.0);
    x.inputs = vec![bo("End Vertex", false).hide_value(), fl("Edge Cost", 1.0).hide_value()];
    x.outputs = vec![int("Next Vertex Index", 0), fl("Total Cost", 0.0)];
    x
}
fn index_query(idname: &str, label: &str, input: &str, weights: bool, outs: Vec<Socket>) -> Node {
    let mut x = n(idname, label, 140.0);
    x.inputs = vec![int(input, 0).hide_value()];
    if weights {
        x.inputs.push(fl("Weights", 0.0).hide_value());
        x.inputs.push(int("Sort Index", 0));
    }
    x.outputs = outs;
    x
}
fn corners_of_face() -> Node {
    index_query("GeometryNodeCornersOfFace", "Corners of Face", "Face Index", true, vec![int("Corner Index", 0), int("Total", 0)])
}
fn corners_of_vertex() -> Node {
    index_query("GeometryNodeCornersOfVertex", "Corners of Vertex", "Vertex Index", true, vec![int("Corner Index", 0), int("Total", 0)])
}
fn edges_of_corner() -> Node {
    index_query("GeometryNodeEdgesOfCorner", "Edges of Corner", "Corner Index", false, vec![int("Next Edge Index", 0), int("Previous Edge Index", 0)])
}
fn edges_of_vertex() -> Node {
    index_query("GeometryNodeEdgesOfVertex", "Edges of Vertex", "Vertex Index", true, vec![int("Edge Index", 0), int("Total", 0)])
}
fn face_of_corner() -> Node {
    index_query("GeometryNodeFaceOfCorner", "Face of Corner", "Corner Index", false, vec![int("Face Index", 0), int("Index in Face", 0)])
}
fn offset_corner_in_face() -> Node {
    let mut x = index_query("GeometryNodeOffsetCornerInFace", "Offset Corner in Face", "Corner Index", false, vec![int("Corner Index", 0)]);
    x.inputs.push(int("Offset", 0));
    x
}
fn vertex_of_corner() -> Node {
    index_query("GeometryNodeVertexOfCorner", "Vertex of Corner", "Corner Index", false, vec![int("Vertex Index", 0)])
}

// ---------------------------------------------------------------------------
// Point

fn distribute_points() -> Node {
    let mut x = n("GeometryNodeDistributePointsOnFaces", "Distribute Points on Faces", 170.0);
    props(&mut x, &[("distribute_method", e("RANDOM")), ("use_legacy_normal", Prop::Bool(false))]);
    x.inputs = vec![
        geo("Mesh"),
        sel(),
        dist("Distance Min", 0.0).range(0.0, f32::MAX),
        fl("Density Max", 10.0).range(0.0, f32::MAX),
        fl("Density", 10.0).range(0.0, f32::MAX),
        fl("Density Factor", 1.0).subtype(S::Factor),
        int("Seed", 0),
    ];
    x.inputs[2].enabled = false;
    x.inputs[3].enabled = false;
    x.inputs[5].enabled = false;
    x.outputs = vec![geo("Points"), vec("Normal", [0.0; 3]), rot("Rotation")];
    x
}
fn points() -> Node {
    let mut x = n("GeometryNodePoints", "Points", 140.0);
    x.inputs = vec![int("Count", 1).range(0.0, f32::MAX), vec("Position", [0.0; 3]).subtype(S::Translation), dist("Radius", 0.1).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Geometry")];
    x
}
fn points_to_vertices() -> Node {
    let mut x = n("GeometryNodePointsToVertices", "Points to Vertices", 140.0);
    x.inputs = vec![geo("Points"), sel()];
    x.outputs = vec![geo("Mesh")];
    x
}
fn set_point_radius() -> Node {
    let mut x = n("GeometryNodeSetPointRadius", "Set Point Radius", 140.0);
    x.inputs = vec![geo("Points"), sel(), dist("Radius", 0.05).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Points")];
    x
}

// ---------------------------------------------------------------------------
// Material

fn replace_material() -> Node {
    let mut x = n("GeometryNodeReplaceMaterial", "Replace Material", 140.0);
    x.inputs = vec![geo("Geometry"), mat("Old"), mat("New")];
    x.outputs = vec![geo("Geometry")];
    x
}
fn material_selection() -> Node {
    let mut x = n("GeometryNodeMaterialSelection", "Material Selection", 140.0);
    x.inputs = vec![mat("Material")];
    x.outputs = vec![bo("Selection", false)];
    x
}
fn set_material() -> Node {
    let mut x = n("GeometryNodeSetMaterial", "Set Material", 140.0);
    x.inputs = vec![geo("Geometry"), sel(), mat("Material")];
    x.outputs = vec![geo("Geometry")];
    x
}
fn set_material_index() -> Node {
    let mut x = n("GeometryNodeSetMaterialIndex", "Set Material Index", 140.0);
    x.inputs = vec![geo("Geometry"), sel(), int("Material Index", 0).range(0.0, f32::MAX)];
    x.outputs = vec![geo("Geometry")];
    x
}

// ---------------------------------------------------------------------------
// Utilities

fn boolean_math() -> Node {
    let mut x = n("FunctionNodeBooleanMath", "Boolean Math", 140.0);
    props(&mut x, &[("operation", e("AND"))]);
    x.inputs = vec![bo("Boolean", false), bo("Boolean", false).ident("Boolean_001")];
    x.outputs = vec![bo("Boolean", false)];
    x
}
fn integer_math() -> Node {
    let mut x = n("FunctionNodeIntegerMath", "Integer Math", 140.0);
    props(&mut x, &[("operation", e("ADD"))]);
    x.inputs = vec![int("Value", 0), int("Value", 0).ident("Value_001"), int("Value", 0).ident("Value_002")];
    x.inputs[2].enabled = false;
    x.outputs = vec![int("Value", 0)];
    x
}
fn compare() -> Node {
    let mut x = n("FunctionNodeCompare", "Compare", 140.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("operation", e("GREATER_THAN")), ("mode", e("ELEMENT"))]);
    x.inputs = vec![
        fl("A", 0.0),
        fl("B", 0.0),
        int("A", 0).ident("A_INT"),
        int("B", 0).ident("B_INT"),
        vec("A", [0.0; 3]).ident("A_VEC3"),
        vec("B", [0.0; 3]).ident("B_VEC3"),
        col("A", [0.8, 0.8, 0.8, 1.0]).ident("A_COL"),
        col("B", [0.8, 0.8, 0.8, 1.0]).ident("B_COL"),
        st("A").ident("A_STR"),
        st("B").ident("B_STR"),
        fl("C", 0.9),
        fl("Angle", 0.0872665).subtype(S::Angle),
        fl("Epsilon", 0.001).range(-10000.0, 10000.0),
    ];
    for k in 2..13 {
        x.inputs[k].enabled = false;
    }
    x.outputs = vec![bo("Result", false)];
    x
}
fn switch() -> Node {
    let mut x = n("GeometryNodeSwitch", "Switch", 140.0);
    props(&mut x, &[("input_type", e("GEOMETRY"))]);
    x.inputs = vec![bo("Switch", false), geo("False"), geo("True")];
    x.outputs = vec![geo("Output")];
    x
}
fn index_switch() -> Node {
    let mut x = n("GeometryNodeIndexSwitch", "Index Switch", 140.0);
    props(&mut x, &[("data_type", e("GEOMETRY"))]);
    x.inputs = vec![int("Index", 0), geo("0").ident("Item_0"), geo("1").ident("Item_1")];
    x.outputs = vec![geo("Output")];
    x
}
fn menu_switch() -> Node {
    let mut x = n("GeometryNodeMenuSwitch", "Menu Switch", 140.0);
    props(&mut x, &[("data_type", e("GEOMETRY"))]);
    let mut m = menu("Menu");
    m.value = V::String("A".into());
    x.inputs = vec![m, geo("A").ident("Item_0"), geo("B").ident("Item_1")];
    x.outputs = vec![geo("Output")];
    x
}
fn random_value() -> Node {
    let mut x = n("FunctionNodeRandomValue", "Random Value", 140.0);
    props(&mut x, &[("data_type", e("FLOAT"))]);
    x.inputs = vec![
        vec("Min", [0.0; 3]),
        vec("Max", [1.0; 3]),
        fl("Min", 0.0).ident("Min_001"),
        fl("Max", 1.0).ident("Max_001"),
        int("Min", 0).ident("Min_002"),
        int("Max", 100).ident("Max_002"),
        fl("Probability", 0.5).subtype(S::Factor),
        int("ID", 0).hide_value(),
        int("Seed", 0),
    ];
    for k in [0, 1, 4, 5, 6] {
        x.inputs[k].enabled = false;
    }
    x.outputs = vec![vec("Value", [0.0; 3]), fl("Value", 0.0).ident("Value_001"), int("Value", 0).ident("Value_002"), bo("Value", false).ident("Value_003")];
    for k in [0, 2, 3] {
        x.outputs[k].enabled = false;
    }
    x
}
fn combine_color() -> Node {
    let mut x = n("FunctionNodeCombineColor", "Combine Color", 140.0);
    props(&mut x, &[("mode", e("RGB"))]);
    x.inputs = vec![fl("Red", 0.0).subtype(S::Factor), fl("Green", 0.0).subtype(S::Factor), fl("Blue", 0.0).subtype(S::Factor), fl("Alpha", 1.0).subtype(S::Factor)];
    x.outputs = vec![col("Color", [0.0, 0.0, 0.0, 1.0])];
    x
}
fn separate_color() -> Node {
    let mut x = n("FunctionNodeSeparateColor", "Separate Color", 140.0);
    props(&mut x, &[("mode", e("RGB"))]);
    x.inputs = vec![col("Color", [1.0; 4])];
    x.outputs = vec![fl("Red", 0.0), fl("Green", 0.0), fl("Blue", 0.0), fl("Alpha", 0.0)];
    x
}
fn float_to_int() -> Node {
    let mut x = n("FunctionNodeFloatToInt", "Float to Integer", 140.0);
    props(&mut x, &[("rounding_mode", e("ROUND"))]);
    x.inputs = vec![fl("Float", 0.0)];
    x.outputs = vec![int("Integer", 0)];
    x
}
fn rot_node(idname: &str, label: &str, ins: Vec<Socket>, outs: Vec<Socket>, p: &[(&str, Prop)]) -> Node {
    let mut x = n(idname, label, 140.0);
    props(&mut x, p);
    x.inputs = ins;
    x.outputs = outs;
    x
}
fn euler_to_rotation() -> Node {
    rot_node("FunctionNodeEulerToRotation", "Euler to Rotation", vec![vec("Euler", [0.0; 3]).subtype(S::Euler)], vec![rot("Rotation")], &[])
}
fn rotation_to_euler() -> Node {
    rot_node("FunctionNodeRotationToEuler", "Rotation to Euler", vec![rot("Rotation")], vec![vec("Euler", [0.0; 3]).subtype(S::Euler)], &[])
}
fn axis_angle_to_rotation() -> Node {
    rot_node("FunctionNodeAxisAngleToRotation", "Axis Angle to Rotation", vec![vec("Axis", [0.0, 0.0, 1.0]), fl("Angle", 0.0).subtype(S::Angle)], vec![rot("Rotation")], &[])
}
fn rotation_to_axis_angle() -> Node {
    rot_node("FunctionNodeRotationToAxisAngle", "Rotation to Axis Angle", vec![rot("Rotation")], vec![vec("Axis", [0.0; 3]), fl("Angle", 0.0).subtype(S::Angle)], &[])
}
fn quaternion_to_rotation() -> Node {
    rot_node("FunctionNodeQuaternionToRotation", "Quaternion to Rotation", vec![fl("W", 1.0), fl("X", 0.0), fl("Y", 0.0), fl("Z", 0.0)], vec![rot("Rotation")], &[])
}
fn rotation_to_quaternion() -> Node {
    rot_node("FunctionNodeRotationToQuaternion", "Rotation to Quaternion", vec![rot("Rotation")], vec![fl("W", 0.0), fl("X", 0.0), fl("Y", 0.0), fl("Z", 0.0)], &[])
}
fn rotate_vector() -> Node {
    rot_node("FunctionNodeRotateVector", "Rotate Vector", vec![vec("Vector", [0.0; 3]), rot("Rotation")], vec![vec("Vector", [0.0; 3])], &[])
}
fn rotate_rotation() -> Node {
    rot_node("FunctionNodeRotateRotation", "Rotate Rotation", vec![rot("Rotation"), rot("Rotate By")], vec![rot("Rotation")], &[("rotation_space", e("GLOBAL"))])
}
fn invert_rotation() -> Node {
    rot_node("FunctionNodeInvertRotation", "Invert Rotation", vec![rot("Rotation")], vec![rot("Rotation")], &[])
}
fn align_rotation_to_vector() -> Node {
    rot_node(
        "FunctionNodeAlignRotationToVector",
        "Align Rotation to Vector",
        vec![rot("Rotation"), fl("Factor", 1.0).subtype(S::Factor), vec("Vector", [0.0, 0.0, 1.0])],
        vec![rot("Rotation")],
        &[("axis", e("Z")), ("pivot_axis", e("AUTO"))],
    )
}
fn align_euler_to_vector() -> Node {
    rot_node(
        "FunctionNodeAlignEulerToVector",
        "Align Euler to Vector",
        vec![vec("Rotation", [0.0; 3]).subtype(S::Euler), fl("Factor", 1.0).subtype(S::Factor), vec("Vector", [0.0, 0.0, 1.0])],
        vec![vec("Rotation", [0.0; 3]).subtype(S::Euler)],
        &[("axis", e("X")), ("pivot_axis", e("AUTO"))],
    )
}
fn combine_transform() -> Node {
    rot_node("FunctionNodeCombineTransform", "Combine Transform", vec![vec("Translation", [0.0; 3]), rot("Rotation"), xyz("Scale", [1.0; 3])], vec![matrix("Transform")], &[])
}
fn separate_transform() -> Node {
    rot_node("FunctionNodeSeparateTransform", "Separate Transform", vec![matrix("Transform")], vec![vec("Translation", [0.0; 3]), rot("Rotation"), vec("Scale", [0.0; 3])], &[])
}
fn transform_point() -> Node {
    rot_node("FunctionNodeTransformPoint", "Transform Point", vec![vec("Vector", [0.0; 3]), matrix("Transform")], vec![vec("Vector", [0.0; 3])], &[])
}
fn transform_direction() -> Node {
    rot_node("FunctionNodeTransformDirection", "Transform Direction", vec![vec("Direction", [0.0; 3]), matrix("Transform")], vec![vec("Direction", [0.0; 3])], &[])
}
fn invert_matrix() -> Node {
    rot_node("FunctionNodeInvertMatrix", "Invert Matrix", vec![matrix("Matrix")], vec![matrix("Matrix"), bo("Invertible", false)], &[])
}
fn multiply_matrices() -> Node {
    rot_node("FunctionNodeMatrixMultiply", "Multiply Matrices", vec![matrix("Matrix"), matrix("Matrix").ident("Matrix_001")], vec![matrix("Matrix")], &[])
}
fn transpose_matrix() -> Node {
    rot_node("FunctionNodeTransposeMatrix", "Transpose Matrix", vec![matrix("Matrix")], vec![matrix("Matrix")], &[])
}
fn matrix_determinant() -> Node {
    rot_node("FunctionNodeMatrixDeterminant", "Matrix Determinant", vec![matrix("Matrix")], vec![fl("Determinant", 0.0)], &[])
}
fn accumulate_field() -> Node {
    let mut x = n("GeometryNodeAccumulateField", "Accumulate Field", 140.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("domain", e("POINT"))]);
    x.inputs = vec![fl("Value", 1.0), int("Group ID", 0).hide_value()];
    x.outputs = vec![fl("Leading", 0.0), fl("Trailing", 0.0), fl("Total", 0.0)];
    x
}
fn field_at_index() -> Node {
    let mut x = n("GeometryNodeFieldAtIndex", "Evaluate at Index", 140.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("domain", e("POINT"))]);
    x.inputs = vec![int("Index", 0), fl("Value", 0.0).hide_value()];
    x.outputs = vec![fl("Value", 0.0)];
    x
}
fn field_on_domain() -> Node {
    let mut x = n("GeometryNodeFieldOnDomain", "Evaluate on Domain", 140.0);
    props(&mut x, &[("data_type", e("FLOAT")), ("domain", e("POINT"))]);
    x.inputs = vec![fl("Value", 0.0).hide_value()];
    x.outputs = vec![fl("Value", 0.0)];
    x
}
fn repeat_input() -> Node {
    let mut x = n("GeometryNodeRepeatInput", "Repeat Input", 140.0);
    props(&mut x, &[("paired_output", Prop::String(String::new()))]);
    x.inputs = vec![int("Iterations", 1).range(0.0, f32::MAX), geo("Geometry").ident("Item_0"), extend()];
    x.outputs = vec![int("Iteration", 0), geo("Geometry").ident("Item_0"), extend()];
    x
}
fn repeat_output() -> Node {
    let mut x = n("GeometryNodeRepeatOutput", "Repeat Output", 140.0);
    props(&mut x, &[("inspection_index", Prop::Int(0))]);
    x.inputs = vec![geo("Geometry").ident("Item_0"), extend()];
    x.outputs = vec![geo("Geometry").ident("Item_0"), extend()];
    x
}
fn simulation_input() -> Node {
    let mut x = n("GeometryNodeSimulationInput", "Simulation Input", 140.0);
    props(&mut x, &[("paired_output", Prop::String(String::new()))]);
    x.inputs = vec![geo("Geometry").ident("Item_0"), extend()];
    x.outputs = vec![fl("Delta Time", 0.0), geo("Geometry").ident("Item_0"), extend()];
    x
}
fn simulation_output() -> Node {
    let mut x = n("GeometryNodeSimulationOutput", "Simulation Output", 140.0);
    x.inputs = vec![bo("Skip", false), geo("Geometry").ident("Item_0"), extend()];
    x.outputs = vec![geo("Geometry").ident("Item_0"), extend()];
    x
}

macro_rules! ty {
    ($id:literal, $label:literal, $cat:ident, $f:expr) => {
        NodeType { idname: $id, label: $label, tree: TreeKind::Geometry, category: Category::$cat, build: $f }
    };
}

/// Geometry node types in Add-menu order.
pub static GEO_TYPES: &[NodeType] = &[
    ty!("GeometryNodeAttributeStatistic", "Attribute Statistic", Attribute, attribute_statistic),
    ty!("GeometryNodeAttributeDomainSize", "Domain Size", Attribute, domain_size),
    ty!("GeometryNodeBlurAttribute", "Blur Attribute", Attribute, blur_attribute),
    ty!("GeometryNodeCaptureAttribute", "Capture Attribute", Attribute, capture_attribute),
    ty!("GeometryNodeRemoveAttribute", "Remove Named Attribute", Attribute, remove_attribute),
    ty!("GeometryNodeStoreNamedAttribute", "Store Named Attribute", Attribute, store_named_attribute),
    ty!("FunctionNodeInputBool", "Boolean", Input, input_bool),
    ty!("FunctionNodeInputColor", "Color", Input, input_color),
    ty!("FunctionNodeInputInt", "Integer", Input, input_int),
    ty!("GeometryNodeInputMaterial", "Material", Input, input_material),
    ty!("FunctionNodeInputRotation", "Rotation", Input, input_rotation),
    ty!("FunctionNodeInputString", "String", Input, input_string),
    ty!("FunctionNodeInputVector", "Vector", Input, input_vector),
    ty!("GeometryNodeCollectionInfo", "Collection Info", Input, collection_info),
    ty!("GeometryNodeObjectInfo", "Object Info", Input, object_info),
    ty!("GeometryNodeSelfObject", "Self Object", Input, self_object),
    ty!("GeometryNodeIsViewport", "Is Viewport", Input, is_viewport),
    ty!("GeometryNodeInputActiveCamera", "Active Camera", Input, active_camera),
    ty!("GeometryNodeInputSceneTime", "Scene Time", Input, scene_time),
    ty!("GeometryNodeInputNamedAttribute", "Named Attribute", Input, named_attribute),
    ty!("GeometryNodeInputID", "ID", Input, input_id),
    ty!("GeometryNodeInputIndex", "Index", Input, input_index),
    ty!("GeometryNodeInputNormal", "Normal", Input, input_normal),
    ty!("GeometryNodeInputPosition", "Position", Input, input_position),
    ty!("GeometryNodeInputRadius", "Radius", Input, input_radius),
    ty!("GeometryNodeViewer", "Viewer", Output, viewer),
    ty!("GeometryNodeBoundBox", "Bounding Box", Geometry, bound_box),
    ty!("GeometryNodeConvexHull", "Convex Hull", Geometry, convex_hull),
    ty!("GeometryNodeDeleteGeometry", "Delete Geometry", Geometry, delete_geometry),
    ty!("GeometryNodeDuplicateElements", "Duplicate Elements", Geometry, duplicate_elements),
    ty!("GeometryNodeMergeByDistance", "Merge by Distance", Geometry, merge_by_distance),
    ty!("GeometryNodeSortElements", "Sort Elements", Geometry, sort_elements),
    ty!("GeometryNodeTransform", "Transform Geometry", Geometry, transform_geometry),
    ty!("GeometryNodeSeparateComponents", "Separate Components", Geometry, separate_components),
    ty!("GeometryNodeSeparateGeometry", "Separate Geometry", Geometry, separate_geometry),
    ty!("GeometryNodeGeometryToInstance", "Geometry to Instance", Geometry, geometry_to_instance),
    ty!("GeometryNodeJoinGeometry", "Join Geometry", Geometry, join_geometry),
    ty!("GeometryNodeProximity", "Geometry Proximity", Geometry, proximity),
    ty!("GeometryNodeRaycast", "Raycast", Geometry, raycast),
    ty!("GeometryNodeSampleIndex", "Sample Index", Geometry, sample_index),
    ty!("GeometryNodeSampleNearest", "Sample Nearest", Geometry, sample_nearest),
    ty!("GeometryNodeSampleNearestSurface", "Sample Nearest Surface", Geometry, sample_nearest_surface),
    ty!("GeometryNodeSetID", "Set ID", Geometry, set_id),
    ty!("GeometryNodeSetPosition", "Set Position", Geometry, set_position),
    ty!("GeometryNodeInputCurveHandlePositions", "Curve Handle Positions", Curve, handle_positions),
    ty!("GeometryNodeCurveLength", "Curve Length", Curve, curve_length),
    ty!("GeometryNodeInputTangent", "Curve Tangent", Curve, curve_tangent),
    ty!("GeometryNodeInputCurveTilt", "Curve Tilt", Curve, curve_tilt),
    ty!("GeometryNodeCurveEndpointSelection", "Endpoint Selection", Curve, endpoint_selection),
    ty!("GeometryNodeInputSplineCyclic", "Is Spline Cyclic", Curve, spline_cyclic),
    ty!("GeometryNodeSplineLength", "Spline Length", Curve, spline_length),
    ty!("GeometryNodeSplineParameter", "Spline Parameter", Curve, spline_parameter),
    ty!("GeometryNodeInputSplineResolution", "Spline Resolution", Curve, spline_resolution),
    ty!("GeometryNodeSampleCurve", "Sample Curve", Curve, sample_curve),
    ty!("GeometryNodeSetCurveRadius", "Set Curve Radius", Curve, set_curve_radius),
    ty!("GeometryNodeSetCurveTilt", "Set Curve Tilt", Curve, set_curve_tilt),
    ty!("GeometryNodeSetCurveHandlePositions", "Set Handle Positions", Curve, set_handle_positions),
    ty!("GeometryNodeCurveSetHandles", "Set Handle Type", Curve, set_handle_type),
    ty!("GeometryNodeSetSplineCyclic", "Set Spline Cyclic", Curve, set_spline_cyclic),
    ty!("GeometryNodeSetSplineResolution", "Set Spline Resolution", Curve, set_spline_resolution),
    ty!("GeometryNodeCurveSplineType", "Set Spline Type", Curve, set_spline_type),
    ty!("GeometryNodeCurveToMesh", "Curve to Mesh", Curve, curve_to_mesh),
    ty!("GeometryNodeCurveToPoints", "Curve to Points", Curve, curve_to_points),
    ty!("GeometryNodeFillCurve", "Fill Curve", Curve, fill_curve),
    ty!("GeometryNodeFilletCurve", "Fillet Curve", Curve, fillet_curve),
    ty!("GeometryNodeResampleCurve", "Resample Curve", Curve, resample_curve),
    ty!("GeometryNodeReverseCurve", "Reverse Curve", Curve, reverse_curve),
    ty!("GeometryNodeSubdivideCurve", "Subdivide Curve", Curve, subdivide_curve),
    ty!("GeometryNodeTrimCurve", "Trim Curve", Curve, trim_curve),
    ty!("GeometryNodeCurveArc", "Arc", Curve, curve_arc),
    ty!("GeometryNodeCurvePrimitiveBezierSegment", "Bezier Segment", Curve, bezier_segment),
    ty!("GeometryNodeCurvePrimitiveCircle", "Curve Circle", Curve, curve_circle),
    ty!("GeometryNodeCurvePrimitiveLine", "Curve Line", Curve, curve_line),
    ty!("GeometryNodeCurveSpiral", "Spiral", Curve, curve_spiral),
    ty!("GeometryNodeCurveQuadraticBezier", "Quadratic Bezier", Curve, quadratic_bezier),
    ty!("GeometryNodeCurvePrimitiveQuadrilateral", "Quadrilateral", Curve, curve_quadrilateral),
    ty!("GeometryNodeCurveStar", "Star", Curve, curve_star),
    ty!("GeometryNodeCurveOfPoint", "Curve of Point", Curve, curve_of_point),
    ty!("GeometryNodeOffsetPointInCurve", "Offset Point in Curve", Curve, offset_point_in_curve),
    ty!("GeometryNodePointsOfCurve", "Points of Curve", Curve, points_of_curve),
    ty!("GeometryNodeInstanceOnPoints", "Instance on Points", Instances, instance_on_points),
    ty!("GeometryNodeInstancesToPoints", "Instances to Points", Instances, instances_to_points),
    ty!("GeometryNodeRealizeInstances", "Realize Instances", Instances, realize_instances),
    ty!("GeometryNodeRotateInstances", "Rotate Instances", Instances, rotate_instances),
    ty!("GeometryNodeScaleInstances", "Scale Instances", Instances, scale_instances),
    ty!("GeometryNodeTranslateInstances", "Translate Instances", Instances, translate_instances),
    ty!("GeometryNodeInputInstanceRotation", "Instance Rotation", Instances, instance_rotation),
    ty!("GeometryNodeInputInstanceScale", "Instance Scale", Instances, instance_scale),
    ty!("GeometryNodeInstanceTransform", "Instance Transform", Instances, instance_transform),
    ty!("GeometryNodeDualMesh", "Dual Mesh", Mesh, dual_mesh),
    ty!("GeometryNodeExtrudeMesh", "Extrude Mesh", Mesh, extrude_mesh),
    ty!("GeometryNodeFlipFaces", "Flip Faces", Mesh, flip_faces),
    ty!("GeometryNodeMeshBoolean", "Mesh Boolean", Mesh, mesh_boolean),
    ty!("GeometryNodeMeshToCurve", "Mesh to Curve", Mesh, mesh_to_curve),
    ty!("GeometryNodeMeshToPoints", "Mesh to Points", Mesh, mesh_to_points),
    ty!("GeometryNodeScaleElements", "Scale Elements", Mesh, scale_elements),
    ty!("GeometryNodeSetShadeSmooth", "Set Shade Smooth", Mesh, set_shade_smooth),
    ty!("GeometryNodeSplitEdges", "Split Edges", Mesh, split_edges),
    ty!("GeometryNodeSubdivideMesh", "Subdivide Mesh", Mesh, subdivide_mesh),
    ty!("GeometryNodeSubdivisionSurface", "Subdivision Surface", Mesh, subdivision_surface),
    ty!("GeometryNodeTriangulate", "Triangulate", Mesh, triangulate),
    ty!("GeometryNodeMeshCone", "Cone", Mesh, mesh_cone),
    ty!("GeometryNodeMeshCube", "Cube", Mesh, mesh_cube),
    ty!("GeometryNodeMeshCylinder", "Cylinder", Mesh, mesh_cylinder),
    ty!("GeometryNodeMeshGrid", "Grid", Mesh, mesh_grid),
    ty!("GeometryNodeMeshIcoSphere", "Ico Sphere", Mesh, mesh_ico_sphere),
    ty!("GeometryNodeMeshCircle", "Mesh Circle", Mesh, mesh_circle),
    ty!("GeometryNodeMeshLine", "Mesh Line", Mesh, mesh_line),
    ty!("GeometryNodeMeshUVSphere", "UV Sphere", Mesh, mesh_uv_sphere),
    ty!("GeometryNodeInputMeshEdgeAngle", "Edge Angle", Mesh, edge_angle),
    ty!("GeometryNodeInputMeshEdgeNeighbors", "Edge Neighbors", Mesh, edge_neighbors),
    ty!("GeometryNodeInputMeshEdgeVertices", "Edge Vertices", Mesh, edge_vertices),
    ty!("GeometryNodeInputEdgeSmooth", "Is Edge Smooth", Mesh, edge_smooth_in),
    ty!("GeometryNodeInputMeshFaceArea", "Face Area", Mesh, face_area),
    ty!("GeometryNodeInputMeshFaceNeighbors", "Face Neighbors", Mesh, face_neighbors),
    ty!("GeometryNodeInputMeshFaceIsPlanar", "Is Face Planar", Mesh, face_is_planar),
    ty!("GeometryNodeInputShadeSmooth", "Is Face Smooth", Mesh, shade_smooth_in),
    ty!("GeometryNodeInputMeshIsland", "Mesh Island", Mesh, mesh_island),
    ty!("GeometryNodeInputShortestEdgePaths", "Shortest Edge Paths", Mesh, shortest_edge_paths),
    ty!("GeometryNodeInputMeshVertexNeighbors", "Vertex Neighbors", Mesh, vertex_neighbors),
    ty!("GeometryNodeCornersOfFace", "Corners of Face", Mesh, corners_of_face),
    ty!("GeometryNodeCornersOfVertex", "Corners of Vertex", Mesh, corners_of_vertex),
    ty!("GeometryNodeEdgesOfCorner", "Edges of Corner", Mesh, edges_of_corner),
    ty!("GeometryNodeEdgesOfVertex", "Edges of Vertex", Mesh, edges_of_vertex),
    ty!("GeometryNodeFaceOfCorner", "Face of Corner", Mesh, face_of_corner),
    ty!("GeometryNodeOffsetCornerInFace", "Offset Corner in Face", Mesh, offset_corner_in_face),
    ty!("GeometryNodeVertexOfCorner", "Vertex of Corner", Mesh, vertex_of_corner),
    ty!("GeometryNodeDistributePointsOnFaces", "Distribute Points on Faces", Point, distribute_points),
    ty!("GeometryNodePoints", "Points", Point, points),
    ty!("GeometryNodePointsToVertices", "Points to Vertices", Point, points_to_vertices),
    ty!("GeometryNodeSetPointRadius", "Set Point Radius", Point, set_point_radius),
    ty!("GeometryNodeSimulationInput", "Simulation Input", SimulationZone, simulation_input),
    ty!("GeometryNodeSimulationOutput", "Simulation Output", SimulationZone, simulation_output),
    ty!("GeometryNodeReplaceMaterial", "Replace Material", Material, replace_material),
    ty!("GeometryNodeInputMaterialIndex", "Material Index", Material, material_index),
    ty!("GeometryNodeMaterialSelection", "Material Selection", Material, material_selection),
    ty!("GeometryNodeSetMaterial", "Set Material", Material, set_material),
    ty!("GeometryNodeSetMaterialIndex", "Set Material Index", Material, set_material_index),
    ty!("FunctionNodeCombineColor", "Combine Color", Utilities, combine_color),
    ty!("FunctionNodeSeparateColor", "Separate Color", Utilities, separate_color),
    ty!("FunctionNodeBooleanMath", "Boolean Math", Utilities, boolean_math),
    ty!("FunctionNodeIntegerMath", "Integer Math", Utilities, integer_math),
    ty!("FunctionNodeCompare", "Compare", Utilities, compare),
    ty!("FunctionNodeFloatToInt", "Float to Integer", Utilities, float_to_int),
    ty!("FunctionNodeRandomValue", "Random Value", Utilities, random_value),
    ty!("GeometryNodeSwitch", "Switch", Utilities, switch),
    ty!("GeometryNodeIndexSwitch", "Index Switch", Utilities, index_switch),
    ty!("GeometryNodeMenuSwitch", "Menu Switch", Utilities, menu_switch),
    ty!("GeometryNodeAccumulateField", "Accumulate Field", Utilities, accumulate_field),
    ty!("GeometryNodeFieldAtIndex", "Evaluate at Index", Utilities, field_at_index),
    ty!("GeometryNodeFieldOnDomain", "Evaluate on Domain", Utilities, field_on_domain),
    ty!("FunctionNodeAlignEulerToVector", "Align Euler to Vector", Utilities, align_euler_to_vector),
    ty!("FunctionNodeAlignRotationToVector", "Align Rotation to Vector", Utilities, align_rotation_to_vector),
    ty!("FunctionNodeAxisAngleToRotation", "Axis Angle to Rotation", Utilities, axis_angle_to_rotation),
    ty!("FunctionNodeEulerToRotation", "Euler to Rotation", Utilities, euler_to_rotation),
    ty!("FunctionNodeInvertRotation", "Invert Rotation", Utilities, invert_rotation),
    ty!("FunctionNodeQuaternionToRotation", "Quaternion to Rotation", Utilities, quaternion_to_rotation),
    ty!("FunctionNodeRotateRotation", "Rotate Rotation", Utilities, rotate_rotation),
    ty!("FunctionNodeRotateVector", "Rotate Vector", Utilities, rotate_vector),
    ty!("FunctionNodeRotationToAxisAngle", "Rotation to Axis Angle", Utilities, rotation_to_axis_angle),
    ty!("FunctionNodeRotationToEuler", "Rotation to Euler", Utilities, rotation_to_euler),
    ty!("FunctionNodeRotationToQuaternion", "Rotation to Quaternion", Utilities, rotation_to_quaternion),
    ty!("FunctionNodeCombineTransform", "Combine Transform", Utilities, combine_transform),
    ty!("FunctionNodeInvertMatrix", "Invert Matrix", Utilities, invert_matrix),
    ty!("FunctionNodeMatrixDeterminant", "Matrix Determinant", Utilities, matrix_determinant),
    ty!("FunctionNodeMatrixMultiply", "Multiply Matrices", Utilities, multiply_matrices),
    ty!("FunctionNodeSeparateTransform", "Separate Transform", Utilities, separate_transform),
    ty!("FunctionNodeTransformDirection", "Transform Direction", Utilities, transform_direction),
    ty!("FunctionNodeTransformPoint", "Transform Point", Utilities, transform_point),
    ty!("FunctionNodeTransposeMatrix", "Transpose Matrix", Utilities, transpose_matrix),
    ty!("GeometryNodeRepeatInput", "Repeat Input", Utilities, repeat_input),
    ty!("GeometryNodeRepeatOutput", "Repeat Output", Utilities, repeat_output),
];

/// Multi-input sockets (several links into one socket).
pub fn is_multi_input(idname: &str, id: &str) -> bool {
    matches!((idname, id), ("GeometryNodeJoinGeometry", "Geometry") | ("GeometryNodeMeshBoolean", "Mesh 2") | ("GeometryNodeGeometryToInstance", "Geometry") | ("GeometryNodeStringJoin", "Strings"))
}

/// The sockets of `node` that follow its data type.
fn typed_sockets(idname: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match idname {
        "GeometryNodeStoreNamedAttribute" => (&["Value"], &[]),
        "GeometryNodeInputNamedAttribute" => (&[], &["Attribute"]),
        "GeometryNodeSampleIndex" | "GeometryNodeSampleNearestSurface" | "GeometryNodeFieldAtIndex" | "GeometryNodeFieldOnDomain" | "GeometryNodeBlurAttribute" => (&["Value"], &["Value"]),
        "GeometryNodeRaycast" => (&["Attribute"], &["Attribute"]),
        "GeometryNodeSampleCurve" => (&["Value"], &["Value"]),
        "GeometryNodeViewer" => (&["Value"], &[]),
        "GeometryNodeAccumulateField" => (&["Value"], &["Leading", "Trailing", "Total"]),
        "GeometryNodeSwitch" => (&["False", "True"], &["Output"]),
        _ => (&[], &[]),
    }
}

/// Retype data-type-following sockets and enable mode-dependent ones, as
/// Blender's geometry node `update` callbacks do.
pub fn geo_update(node: &mut Node) {
    let dt = match node.idname.as_str() {
        "GeometryNodeSwitch" => node.enum_prop("input_type").to_string(),
        _ => node.enum_prop("data_type").to_string(),
    };
    let (ins, outs) = typed_sockets(&node.idname);
    if !dt.is_empty() {
        let ty = data_type_socket(&dt);
        for s in node.inputs.iter_mut().filter(|s| ins.contains(&s.identifier.as_str())) {
            retype(s, ty);
        }
        for s in node.outputs.iter_mut().filter(|s| outs.contains(&s.identifier.as_str())) {
            retype(s, ty);
        }
        if matches!(node.idname.as_str(), "GeometryNodeIndexSwitch" | "GeometryNodeMenuSwitch") {
            for s in node.inputs.iter_mut().skip(1) {
                retype(s, ty);
            }
            for s in node.outputs.iter_mut() {
                retype(s, ty);
            }
        }
    }
    let en = |node: &mut Node, id: &str, on: bool| {
        for s in node.inputs.iter_mut().filter(|s| s.identifier == id) {
            s.enabled = on;
        }
    };
    let en_out = |node: &mut Node, id: &str, on: bool| {
        for s in node.outputs.iter_mut().filter(|s| s.identifier == id) {
            s.enabled = on;
        }
    };
    let p = |node: &Node, k: &str| node.enum_prop(k).to_string();
    match node.idname.as_str() {
        "GeometryNodeAttributeStatistic" => {
            let v = dt == "FLOAT_VECTOR";
            en(node, "Attribute", !v);
            en(node, "Attribute_001", v);
            for s in node.outputs.iter_mut() {
                s.enabled = s.identifier.ends_with("_001") == v;
            }
        }
        "FunctionNodeCompare" => {
            let mode = p(node, "mode");
            let op = p(node, "operation");
            for (id, t) in [("A", "FLOAT"), ("B", "FLOAT"), ("A_INT", "INT"), ("B_INT", "INT"), ("A_VEC3", "VECTOR"), ("B_VEC3", "VECTOR"), ("A_COL", "RGBA"), ("B_COL", "RGBA"), ("A_STR", "STRING"), ("B_STR", "STRING")] {
                en(node, id, dt == t);
            }
            en(node, "C", dt == "VECTOR" && mode == "DOT_PRODUCT");
            en(node, "Angle", dt == "VECTOR" && mode == "DIRECTION");
            en(node, "Epsilon", matches!(op.as_str(), "EQUAL" | "NOT_EQUAL") && dt != "INT" && dt != "STRING");
        }
        "FunctionNodeRandomValue" => {
            for (id, t) in [("Min", "FLOAT_VECTOR"), ("Max", "FLOAT_VECTOR"), ("Min_001", "FLOAT"), ("Max_001", "FLOAT"), ("Min_002", "INT"), ("Max_002", "INT"), ("Probability", "BOOLEAN")] {
                en(node, id, dt == t);
            }
            for (id, t) in [("Value", "FLOAT_VECTOR"), ("Value_001", "FLOAT"), ("Value_002", "INT"), ("Value_003", "BOOLEAN")] {
                en_out(node, id, dt == t);
            }
        }
        "FunctionNodeIntegerMath" => {
            let op = p(node, "operation");
            en(node, "Value_001", !matches!(op.as_str(), "ABSOLUTE" | "NEGATE" | "SIGN"));
            en(node, "Value_002", op == "MULTIPLY_ADD");
        }
        "FunctionNodeBooleanMath" => {
            let op = p(node, "operation");
            en(node, "Boolean_001", op != "NOT");
        }
        "GeometryNodeCurvePrimitiveCircle" => {
            let pts = p(node, "mode") == "POINTS";
            for id in ["Point 1", "Point 2", "Point 3"] {
                en(node, id, pts);
            }
            en(node, "Radius", !pts);
            en_out(node, "Center", pts);
        }
        "GeometryNodeCurvePrimitiveLine" => {
            let dir = p(node, "mode") == "DIRECTION";
            en(node, "End", !dir);
            en(node, "Direction", dir);
            en(node, "Length", dir);
        }
        "GeometryNodeCurveArc" => {
            let pts = p(node, "mode") == "POINTS";
            for id in ["Start", "Middle", "End", "Offset Angle"] {
                en(node, id, pts);
            }
            for id in ["Radius", "Start Angle", "Sweep Angle", "Invert Arc"] {
                en(node, id, !pts);
            }
            en(node, "Invert Arc", true);
            for id in ["Center", "Normal", "Radius"] {
                en_out(node, id, pts);
            }
        }
        "GeometryNodeCurvePrimitiveQuadrilateral" => {
            let m = p(node, "mode");
            let on: &[&str] = match m.as_str() {
                "PARALLELOGRAM" => &["Width", "Height", "Offset"],
                "TRAPEZOID" => &["Bottom Width", "Top Width", "Height", "Offset"],
                "KITE" => &["Width", "Bottom Height", "Top Height"],
                "POINTS" => &["Point 1", "Point 2", "Point 3", "Point 4"],
                _ => &["Width", "Height"],
            };
            let ids: Vec<String> = node.inputs.iter().map(|s| s.identifier.clone()).collect();
            for id in ids {
                en(node, &id, on.contains(&id.as_str()));
            }
        }
        "GeometryNodeCurveToPoints" | "GeometryNodeResampleCurve" => {
            let m = p(node, "mode");
            en(node, "Count", m == "COUNT");
            en(node, "Length", m == "LENGTH");
        }
        "GeometryNodeTrimCurve" => {
            let len = p(node, "mode") == "LENGTH";
            en(node, "Start", !len);
            en(node, "End", !len);
            en(node, "Start_001", len);
            en(node, "End_001", len);
        }
        "GeometryNodeFilletCurve" => {
            let poly = p(node, "mode") == "POLY";
            en(node, "Count", poly);
        }
        "GeometryNodeSampleCurve" => {
            let len = p(node, "mode") == "LENGTH";
            en(node, "Factor", !len);
            en(node, "Length", len);
            en(node, "Curve Index", !node.bool_prop("use_all_curves"));
        }
        "GeometryNodeMeshLine" => {
            let end = p(node, "mode") == "END_POINTS";
            let res = end && p(node, "count_mode") == "RESOLUTION";
            en(node, "Count", !res);
            en(node, "Resolution", res);
            if let Some(s) = node.inputs.iter_mut().find(|s| s.identifier == "Offset") {
                s.name = if end { "End Location".into() } else { "Offset".into() };
            }
        }
        "GeometryNodeDistributePointsOnFaces" => {
            let poisson = p(node, "distribute_method") == "POISSON";
            en(node, "Distance Min", poisson);
            en(node, "Density Max", poisson);
            en(node, "Density", !poisson);
            en(node, "Density Factor", poisson);
        }
        "GeometryNodeScaleElements" => {
            let axis = p(node, "scale_mode") == "SINGLE_AXIS";
            en(node, "Axis", axis);
        }
        "GeometryNodeTransform" => {
            let m = p(node, "mode") == "MATRIX";
            for id in ["Translation", "Rotation", "Scale"] {
                en(node, id, !m);
            }
            en(node, "Transform", m);
        }
        _ => {}
    }
}

/// Blender's geometry-tree Add-menu items for shared shader nodes.
pub static GEO_SHARED: &[(&str, Category)] = &[
    ("ShaderNodeValue", Category::Input),
    ("ShaderNodeTexBrick", Category::Texture),
    ("ShaderNodeTexChecker", Category::Texture),
    ("ShaderNodeTexGabor", Category::Texture),
    ("ShaderNodeTexGradient", Category::Texture),
    ("ShaderNodeTexMagic", Category::Texture),
    ("ShaderNodeTexNoise", Category::Texture),
    ("ShaderNodeTexVoronoi", Category::Texture),
    ("ShaderNodeTexWave", Category::Texture),
    ("ShaderNodeTexWhiteNoise", Category::Texture),
    ("ShaderNodeValToRGB", Category::Utilities),
    ("ShaderNodeRGBCurve", Category::Utilities),
    ("ShaderNodeMix", Category::Utilities),
    ("ShaderNodeClamp", Category::Utilities),
    ("ShaderNodeFloatCurve", Category::Utilities),
    ("ShaderNodeMapRange", Category::Utilities),
    ("ShaderNodeMath", Category::Utilities),
    ("ShaderNodeCombineXYZ", Category::Utilities),
    ("ShaderNodeSeparateXYZ", Category::Utilities),
    ("ShaderNodeVectorCurve", Category::Utilities),
    ("ShaderNodeVectorMath", Category::Utilities),
    ("ShaderNodeVectorRotate", Category::Utilities),
];

/// Add a zone item (a state socket) to a zone's input and output nodes:
/// Repeat, Simulation or Capture Attribute (`zone_in` may be the same node
/// for Capture Attribute). Returns the new identifier.
pub fn add_zone_item(t: &mut super::NodeTree, zone_in: usize, zone_out: usize, name: &str, ty: T) -> String {
    let used: Vec<String> = t.nodes[zone_out].inputs.iter().chain(t.nodes[zone_in].inputs.iter()).map(|s| s.identifier.clone()).collect();
    let k = (0..).find(|k| !used.contains(&format!("Item_{k}"))).unwrap();
    let id = if t.nodes[zone_in].idname == "GeometryNodeCaptureAttribute" { format!("Value_{k}") } else { format!("Item_{k}") };
    let mk = || Socket::new(name, ty, socket_default(ty)).ident(&id);
    let insert = |v: &mut Vec<Socket>| {
        let at = v.iter().position(|s| s.ty == T::Virtual).unwrap_or(v.len());
        v.insert(at, mk());
        at
    };
    let a = insert(&mut t.nodes[zone_in].inputs);
    let b = insert(&mut t.nodes[zone_in].outputs);
    // Shift links into/out of sockets after the insertion points.
    for l in &mut t.links {
        if l.to_node == zone_in && l.to_socket >= a {
            l.to_socket += 1;
        }
        if l.from_node == zone_in && l.from_socket >= b {
            l.from_socket += 1;
        }
    }
    if zone_out != zone_in {
        let c = insert(&mut t.nodes[zone_out].inputs);
        let d = insert(&mut t.nodes[zone_out].outputs);
        for l in &mut t.links {
            if l.to_node == zone_out && l.to_socket >= c {
                l.to_socket += 1;
            }
            if l.from_node == zone_out && l.from_socket >= d {
                l.from_socket += 1;
            }
        }
    }
    id
}
