// SPDX-License-Identifier: GPL-3.0-only
//! Geometry node settings and the Nodes modifier from `.blend` files (member
//! names from Blender's `DNA_node_types.h` and `DNA_modifier_types.h`,
//! GPL-2+):
//!
//! - Node storage structs: `NodeGeometryMeshCylinder/Cone/Circle
//!   {fill_type}` (0 none, 1 n-gon, 2 triangle fan), `NodeGeometryMeshLine
//!   {mode, count_mode}`, `NodeGeometryExtrudeMesh {mode}`,
//!   `NodeGeometryStoreNamedAttribute {data_type, domain}`,
//!   `NodeGeometryAttributeCapture {data_type, domain}`, `NodeAccumulateField
//!   {data_type, domain}`, `NodeGeometrySampleIndex {data_type, domain,
//!   clamp}`, `NodeGeometryDeleteGeometry {domain, mode}`, the curve
//!   primitive and curve operation `mode` structs, `NodeSwitch
//!   {input_type}`, `NodeRandomValue {data_type}`, `NodeFunctionCompare
//!   {operation, data_type, mode}`, the `NodeInput*` value structs and
//!   `NodeGeometryRepeatInput/SimulationInput {output_node_id}` (a
//!   `bNode.identifier`). Other nodes keep their settings in `custom1`/
//!   `custom2`.
//! - Domains are `AttrDomain` (point, edge, face, corner, curve, instance,
//!   layer); attribute types `eCustomDataType` (`CD_PROP_FLOAT` 10,
//!   `CD_PROP_INT32` 11, `CD_PROP_STRING` 12, `CD_PROP_BYTE_COLOR` 17,
//!   `CD_PROP_FLOAT4X4` 20, `CD_PROP_INT8` 45, `CD_PROP_COLOR` 47,
//!   `CD_PROP_FLOAT3` 48, `CD_PROP_FLOAT2` 49, `CD_PROP_BOOL` 50,
//!   `CD_PROP_QUATERNION` 52); socket types `eNodeSocketDatatype`.
//! - `NodesModifierData {modifier, node_group, settings
//!   NodesModifierSettings {properties}}`: `properties` is an `IDProperty`
//!   group (`type` 6, children in `data.group`) holding one property per
//!   group input, named by the input's identifier (`Socket_2`, before 4.0
//!   `Input_2`): `IDP_STRING` 0 (`data.pointer`), `IDP_INT` 1 and
//!   `IDP_BOOLEAN` 10 (`data.val`), `IDP_FLOAT` 2 (the bits of `data.val`),
//!   `IDP_DOUBLE` 8 (`data.val`/`data.val2`), `IDP_ARRAY` 5 (`len` values of
//!   `subtype` at `data.pointer`).
use super::{count, BlendFile, Importer, View};
use crate::nodes::{Node, Prop, Value};

fn e(s: &str) -> Prop {
    Prop::Enum(s.into())
}

fn pick(names: &[&str], i: f64) -> Prop {
    e(names.get(i.max(0.0) as usize).copied().unwrap_or(names[0]))
}

const DOMAINS: [&str; 7] = ["POINT", "EDGE", "FACE", "CORNER", "CURVE", "INSTANCE", "LAYER"];
const FILL: [&str; 3] = ["NONE", "NGON", "TRIANGLE_FAN"];

fn domain(i: f64) -> Prop {
    pick(&DOMAINS, i)
}

/// `eCustomDataType` as the RNA attribute type name.
fn data_type(i: f64) -> Prop {
    e(match i as i64 {
        11 => "INT",
        12 => "STRING",
        17 => "BYTE_COLOR",
        20 => "FLOAT4X4",
        45 => "INT8",
        47 => "FLOAT_COLOR",
        48 => "FLOAT_VECTOR",
        49 => "FLOAT2",
        50 => "BOOLEAN",
        52 => "QUATERNION",
        _ => "FLOAT",
    })
}

/// `eNodeSocketDatatype` as the RNA socket type name.
fn socket_type(i: f64) -> Prop {
    e(match i as i64 {
        1 => "VECTOR",
        2 => "RGBA",
        3 => "SHADER",
        4 => "BOOLEAN",
        6 => "INT",
        7 => "STRING",
        8 => "OBJECT",
        9 => "IMAGE",
        10 => "GEOMETRY",
        11 => "COLLECTION",
        12 => "TEXTURE",
        13 => "MATERIAL",
        14 => "ROTATION",
        15 => "MENU",
        16 => "MATRIX",
        _ => "FLOAT",
    })
}

/// Read the settings of a geometry or function node into its properties.
pub(super) fn geo_props(f: &BlendFile<'_>, bn: &View<'_, '_>, n: &mut Node) {
    let idname = n.idname.clone();
    let c1 = bn.num("custom1").unwrap_or(0.0);
    let c2 = bn.num("custom2").unwrap_or(0.0);
    let st = bn.ptr("storage");
    let storage = |name: &str| f.view_ptr(st, &[name]);
    // `field` of storage struct `sname` mapped through `conv` into `prop`.
    let mut from = |sname: &str, fields: &[(&str, &str, &dyn Fn(f64) -> Prop)]| {
        if let Some(s) = storage(sname) {
            for (field, prop, conv) in fields {
                if let Some(x) = s.num(field) {
                    n.set_prop(prop, conv(x));
                }
            }
        }
    };
    let fill: &dyn Fn(f64) -> Prop = &|x| pick(&FILL, x);
    let dom: &dyn Fn(f64) -> Prop = &domain;
    let dt: &dyn Fn(f64) -> Prop = &data_type;
    let stype: &dyn Fn(f64) -> Prop = &socket_type;
    let modes = |names: &'static [&'static str]| move |x: f64| pick(names, x);
    match idname.as_str() {
        "GeometryNodeMeshCylinder" => from("NodeGeometryMeshCylinder", &[("fill_type", "fill_type", fill)]),
        "GeometryNodeMeshCone" => from("NodeGeometryMeshCone", &[("fill_type", "fill_type", fill)]),
        "GeometryNodeMeshCircle" => from("NodeGeometryMeshCircle", &[("fill_type", "fill_type", fill)]),
        "GeometryNodeMeshLine" => from("NodeGeometryMeshLine", &[("mode", "mode", &modes(&["END_POINTS", "OFFSET"])), ("count_mode", "count_mode", &modes(&["TOTAL", "RESOLUTION"]))]),
        "GeometryNodeExtrudeMesh" => from("NodeGeometryExtrudeMesh", &[("mode", "mode", &modes(&["VERTICES", "EDGES", "FACES"]))]),
        "GeometryNodeStoreNamedAttribute" => from("NodeGeometryStoreNamedAttribute", &[("data_type", "data_type", dt), ("domain", "domain", dom)]),
        "GeometryNodeInputNamedAttribute" => from("NodeGeometryInputNamedAttribute", &[("data_type", "data_type", dt)]),
        "GeometryNodeCaptureAttribute" => from("NodeGeometryAttributeCapture", &[("domain", "domain", dom)]),
        "GeometryNodeViewer" => from("NodeGeometryViewer", &[("data_type", "data_type", dt)]),
        "GeometryNodeAccumulateField" => from("NodeAccumulateField", &[("data_type", "data_type", dt), ("domain", "domain", dom)]),
        "GeometryNodeSampleIndex" => {
            from("NodeGeometrySampleIndex", &[("data_type", "data_type", dt), ("domain", "domain", dom)]);
            if let Some(c) = storage("NodeGeometrySampleIndex").and_then(|s| s.num("clamp")) {
                n.set_prop("clamp", Prop::Bool(c != 0.0));
            }
        }
        "GeometryNodeRaycast" => from("NodeGeometryRaycast", &[("mapping", "mapping", &modes(&["INTERPOLATED", "NEAREST"])), ("data_type", "data_type", dt)]),
        "GeometryNodeProximity" => from("NodeGeometryProximity", &[("target_element", "target_element", &modes(&["POINTS", "EDGES", "FACES"]))]),
        "GeometryNodeDeleteGeometry" => from("NodeGeometryDeleteGeometry", &[("domain", "domain", dom), ("mode", "mode", &modes(&["ALL", "EDGE_FACE", "ONLY_FACE"]))]),
        "GeometryNodeSeparateGeometry" => from("NodeGeometrySeparateGeometry", &[("domain", "domain", dom)]),
        "GeometryNodeDuplicateElements" => from("NodeGeometryDuplicateElements", &[("domain", "domain", dom)]),
        "GeometryNodeMergeByDistance" => from("NodeGeometryMergeByDistance", &[("mode", "mode", &modes(&["ALL", "CONNECTED"]))]),
        "GeometryNodeTransform" => from("NodeGeometryTransform", &[("mode", "mode", &modes(&["COMPONENTS", "MATRIX"]))]),
        "GeometryNodeObjectInfo" => from("NodeGeometryObjectInfo", &[("transform_space", "transform_space", &modes(&["ORIGINAL", "RELATIVE"]))]),
        "GeometryNodeCollectionInfo" => from("NodeGeometryCollectionInfo", &[("transform_space", "transform_space", &modes(&["ORIGINAL", "RELATIVE"]))]),
        "GeometryNodeMeshToPoints" => from("NodeGeometryMeshToPoints", &[("mode", "mode", &modes(&["VERTICES", "EDGES", "FACES", "CORNERS"]))]),
        "GeometryNodeCurvePrimitiveCircle" => from("NodeGeometryCurvePrimitiveCircle", &[("mode", "mode", &modes(&["POINTS", "RADIUS"]))]),
        "GeometryNodeCurvePrimitiveLine" => from("NodeGeometryCurvePrimitiveLine", &[("mode", "mode", &modes(&["POINTS", "DIRECTION"]))]),
        "GeometryNodeCurveArc" => from("NodeGeometryCurveArc", &[("mode", "mode", &modes(&["POINTS", "RADIUS"]))]),
        "GeometryNodeCurvePrimitiveQuadrilateral" => from("NodeGeometryCurvePrimitiveQuad", &[("mode", "mode", &modes(&["RECTANGLE", "PARALLELOGRAM", "TRAPEZOID", "KITE", "POINTS"]))]),
        "GeometryNodeCurvePrimitiveBezierSegment" => from("NodeGeometryCurvePrimitiveBezierSegment", &[("mode", "mode", &modes(&["POSITION", "OFFSET"]))]),
        "GeometryNodeCurveToPoints" => from("NodeGeometryCurveToPoints", &[("mode", "mode", &modes(&["EVALUATED", "COUNT", "LENGTH"]))]),
        "GeometryNodeFillCurve" => from("NodeGeometryCurveFill", &[("mode", "mode", &modes(&["TRIANGLES", "NGONS"]))]),
        "GeometryNodeFilletCurve" => from("NodeGeometryCurveFillet", &[("mode", "mode", &modes(&["BEZIER", "POLY"]))]),
        "GeometryNodeResampleCurve" => from("NodeGeometryCurveResample", &[("mode", "mode", &modes(&["EVALUATED", "COUNT", "LENGTH"]))]),
        "GeometryNodeTrimCurve" => from("NodeGeometryCurveTrim", &[("mode", "mode", &modes(&["FACTOR", "LENGTH"]))]),
        "GeometryNodeSetCurveHandlePositions" => from("NodeGeometrySetCurveHandlePositions", &[("mode", "mode", &modes(&["LEFT", "RIGHT"]))]),
        "GeometryNodeCurveSetHandles" => from("NodeGeometryCurveSetHandles", &[("handle_type", "handle_type", &modes(&["FREE", "AUTO", "VECTOR", "ALIGN"])), ("mode", "mode", &modes(&["LEFT_RIGHT", "LEFT", "RIGHT", "LEFT_RIGHT"]))]),
        "GeometryNodeCurveSplineType" => from("NodeGeometryCurveSplineType", &[("spline_type", "spline_type", &modes(&["CATMULL_ROM", "POLY", "BEZIER", "NURBS"]))]),
        "GeometryNodeSampleCurve" => {
            from("NodeGeometryCurveSample", &[("mode", "mode", &modes(&["FACTOR", "LENGTH"])), ("data_type", "data_type", dt)]);
            if let Some(a) = storage("NodeGeometryCurveSample").and_then(|s| s.num("use_all_curves")) {
                n.set_prop("use_all_curves", Prop::Bool(a != 0.0));
            }
        }
        "GeometryNodeSwitch" => from("NodeSwitch", &[("input_type", "input_type", stype)]),
        "GeometryNodeIndexSwitch" => from("NodeIndexSwitch", &[("data_type", "data_type", stype)]),
        "GeometryNodeMenuSwitch" => from("NodeMenuSwitch", &[("data_type", "data_type", stype)]),
        "FunctionNodeRandomValue" => from("NodeRandomValue", &[("data_type", "data_type", dt)]),
        "FunctionNodeCompare" => from(
            "NodeFunctionCompare",
            &[
                ("operation", "operation", &modes(&["LESS_THAN", "LESS_EQUAL", "GREATER_THAN", "GREATER_EQUAL", "EQUAL", "NOT_EQUAL", "BRIGHTER", "DARKER"])),
                ("data_type", "data_type", stype),
                ("mode", "mode", &modes(&["ELEMENT", "LENGTH", "AVERAGE", "DOT_PRODUCT", "DIRECTION"])),
            ],
        ),
        "FunctionNodeInputBool" => {
            if let Some(b) = storage("NodeInputBool").and_then(|s| s.num("boolean")) {
                n.set_prop("boolean", Prop::Bool(b != 0.0));
            }
        }
        "FunctionNodeInputInt" => {
            if let Some(i) = storage("NodeInputInt").and_then(|s| s.num("integer")) {
                n.set_prop("integer", Prop::Int(i as i32));
            }
        }
        "FunctionNodeInputVector" => {
            if let Some(v) = storage("NodeInputVector").and_then(|s| s.vec::<3>("vector")) {
                n.set_prop("vector", Prop::Vector(v));
            }
        }
        "FunctionNodeInputColor" => {
            if let Some(v) = storage("NodeInputColor").and_then(|s| s.vec::<4>("color")) {
                n.set_prop("value", Prop::Color(v));
            }
        }
        "FunctionNodeInputString" => {
            if let Some(s) = storage("NodeInputString") {
                n.set_prop("string", Prop::String(f.cstr_at(s.ptr("string")).unwrap_or_default()));
            }
        }
        "GeometryNodeRepeatInput" | "GeometryNodeSimulationInput" => {
            let sname = if idname == "GeometryNodeRepeatInput" { "NodeGeometryRepeatInput" } else { "NodeGeometrySimulationInput" };
            if let Some(id) = storage(sname).and_then(|s| s.num("output_node_id")) {
                // Resolved to the output node's name once the tree is read.
                n.set_prop("__output_node_id", Prop::Int(id as i32));
            }
        }
        "GeometryNodeAttributeStatistic" => {
            n.set_prop("data_type", data_type(c1));
            n.set_prop("domain", domain(c2));
        }
        "GeometryNodeBlurAttribute" | "GeometryNodeSampleNearestSurface" => n.set_prop("data_type", data_type(c1)),
        "GeometryNodeFieldAtIndex" | "GeometryNodeFieldOnDomain" => {
            n.set_prop("domain", domain(c1));
            n.set_prop("data_type", data_type(c2));
        }
        "GeometryNodeSampleNearest" => n.set_prop("domain", domain(c2)),
        "GeometryNodeSortElements" | "GeometryNodeSetShadeSmooth" => n.set_prop("domain", domain(c1)),
        "GeometryNodeScaleElements" => {
            n.set_prop("domain", domain(c1));
            n.set_prop("scale_mode", pick(&["UNIFORM", "SINGLE_AXIS"], c2));
        }
        "GeometryNodeAttributeDomainSize" => n.set_prop("component", pick(&["MESH", "POINTCLOUD", "INSTANCES", "MESH", "CURVE"], c1)),
        "GeometryNodeDistributePointsOnFaces" => n.set_prop("distribute_method", pick(&["RANDOM", "POISSON"], c1)),
        "GeometryNodeTriangulate" => {
            n.set_prop("quad_method", pick(&["BEAUTY", "FIXED", "FIXED_ALTERNATE", "SHORTEST_DIAGONAL", "LONGEST_DIAGONAL"], c1));
            n.set_prop("ngon_method", pick(&["BEAUTY", "CLIP"], c2));
        }
        "GeometryNodeMeshBoolean" => {
            n.set_prop("operation", pick(&["INTERSECT", "UNION", "DIFFERENCE"], c1));
            n.set_prop("solver", pick(&["EXACT", "FLOAT"], c2));
        }
        "FunctionNodeBooleanMath" => n.set_prop("operation", pick(&["AND", "OR", "NOT", "NAND", "NOR", "XNOR", "XOR", "IMPLY", "NIMPLY"], c1)),
        "FunctionNodeFloatToInt" => n.set_prop("rounding_mode", pick(&["ROUND", "FLOOR", "CEILING", "TRUNCATE"], c1)),
        _ => {}
    }
}

/// A group tree's inputs as (identifier, name), in interface order: 4.0+
/// `tree_interface` items (sub-panels included), else the `inputs` list.
pub(super) fn input_identifiers(f: &BlendFile<'_>, tree: &View<'_, '_>) -> Vec<(String, String)> {
    fn panel(f: &BlendFile<'_>, p: &View<'_, '_>, out: &mut Vec<(String, String)>, depth: usize) {
        let n = count(p.num("items_num").unwrap_or(0.0));
        for ptr in f.ptr_array(p.ptr("items_array"), n) {
            let Some(it) = f.view_own(ptr) else { continue };
            match it.struct_name() {
                "bNodeTreeInterfacePanel" if depth < 16 => panel(f, &it, out, depth + 1),
                "bNodeTreeInterfaceSocket" if it.num("flag").unwrap_or(0.0) as i64 & 1 != 0 => {
                    out.push((f.cstr_at(it.ptr("identifier")).unwrap_or_default(), f.cstr_at(it.ptr("name")).unwrap_or_default()));
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    if let Some(root) = tree.sub("tree_interface").and_then(|ti| ti.sub("root_panel")) {
        panel(f, &root, &mut out, 0);
    } else {
        for s in f.list(tree.sub("inputs"), &["bNodeSocket"]) {
            out.push((s.string("identifier").unwrap_or_default(), s.string("name").unwrap_or_default()));
        }
    }
    out
}

/// One `IDProperty` as a socket value.
fn id_property(f: &BlendFile<'_>, p: &View<'_, '_>) -> Option<Value> {
    let data = p.sub("data")?;
    let val = data.num("val").unwrap_or(0.0) as i64;
    let val2 = data.num("val2").unwrap_or(0.0) as i64;
    let double = |lo: i64, hi: i64| {
        // `data.val` and `data.val2` hold the double's bytes in file order.
        let (first, second) = (lo as u32 as u64, hi as u32 as u64);
        let bits = if f.e.big { (first << 32) | second } else { (second << 32) | first };
        f64::from_bits(bits) as f32
    };
    Some(match p.num("type")? as i64 {
        0 => Value::String(f.cstr_at(data.ptr("pointer")).unwrap_or_default()),
        1 => Value::Int(val as i32),
        2 => Value::Float(f32::from_bits(val as u32)),
        8 => Value::Float(double(val, val2)),
        10 => Value::Bool(val != 0),
        5 => {
            let n = count(p.num("len").unwrap_or(0.0)).min(4);
            let sub = p.num("subtype").unwrap_or(2.0) as i64;
            let elem = if sub == 8 { 8 } else { 4 };
            let raw = f.raw_array(data.ptr("pointer"), elem, n)?;
            let xs: Vec<f32> = (0..n)
                .map(|i| match sub {
                    8 => f64::from_bits(f.e.u64(raw, i * 8).unwrap_or(0)) as f32,
                    1 => f.e.i32(raw, i * 4).unwrap_or(0) as f32,
                    _ => f32::from_bits(f.e.u32(raw, i * 4).unwrap_or(0)),
                })
                .collect();
            match xs.len() {
                3 => Value::Vector([xs[0], xs[1], xs[2]]),
                4 => Value::Color([xs[0], xs[1], xs[2], xs[3]]),
                1 => Value::Float(xs[0]),
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// A `NodesModifierData` block: the group's address and the modifier's
/// input values by interface name.
pub(super) fn nodes_modifier(f: &BlendFile<'_>, v: &View<'_, '_>) -> (u64, Vec<(String, Value)>) {
    let group = v.ptr("node_group");
    let mut inputs = Vec::new();
    let Some(tree) = f.view_ptr(group, &["bNodeTree"]) else { return (group, inputs) };
    let idents = input_identifiers(f, &tree);
    let props = v.sub("settings").map_or(0, |s| s.ptr("properties"));
    if let Some(root) = f.view_ptr(props, &["IDProperty"]) {
        for p in f.list(root.sub("data").and_then(|d| d.sub("group")), &["IDProperty"]) {
            let key = p.string("name").unwrap_or_default();
            let Some((_, name)) = idents.iter().find(|(id, _)| *id == key) else { continue };
            if inputs.iter().any(|(n, _)| n == name) {
                continue;
            }
            if let Some(val) = id_property(f, &p) {
                inputs.push((name.clone(), val));
            }
        }
    }
    (group, inputs)
}

impl<'f, 'a> Importer<'f, 'a> {
    /// Import the node groups an object's Nodes modifiers use.
    pub(super) fn modifier_groups(&mut self, ob: &View<'f, 'a>) {
        let f = self.f;
        let mut seen = std::collections::HashSet::new();
        let mut p = ob.sub("modifiers").map_or(0, |l| l.ptr("first"));
        while p != 0 && seen.insert(p) && seen.len() < 1000 {
            let Some(v) = f.block(p).and_then(|b| f.block_view(b)) else { break };
            p = v.sub("modifier").map_or(0, |m| m.ptr("next"));
            if v.struct_name() == "NodesModifierData" {
                let _ = self.group(v.ptr("node_group"));
            }
        }
    }
}
