// SPDX-License-Identifier: GPL-3.0-only
//! Geometry and function node settings for the `.blend` writer: the inverse
//! of `geonodes.rs` (storage structs and `custom1`/`custom2` from Blender's
//! `DNA_node_types.h`, GPL-2+), plus the zone and switch item arrays
//! Blender rebuilds the sockets from (`NodeRepeatItem`,
//! `NodeSimulationItem`, `IndexSwitchItem`, `NodeEnumItem`,
//! `NodeGeometryAttributeCaptureItem`). Item sockets are named
//! `Item_<identifier>`, as Blender names them.
use super::write::{Rec, W};
use super::write_nodes::sock_type;
use crate::nodes::{Node, Prop, Socket, SocketType};

const DOMAINS: [&str; 7] = ["POINT", "EDGE", "FACE", "CORNER", "CURVE", "INSTANCE", "LAYER"];
const FILL: [&str; 3] = ["NONE", "NGON", "TRIANGLE_FAN"];

fn ix(names: &[&str], v: &str) -> i64 {
    names.iter().position(|n| *n == v).unwrap_or(0) as i64
}

/// The RNA attribute type name as `eCustomDataType`.
fn data_type(s: &str) -> i64 {
    match s {
        "INT" => 11,
        "STRING" => 12,
        "BYTE_COLOR" => 17,
        "FLOAT4X4" => 20,
        "INT8" => 45,
        "FLOAT_COLOR" => 47,
        "FLOAT_VECTOR" => 48,
        "FLOAT2" => 49,
        "BOOLEAN" => 50,
        "QUATERNION" => 52,
        _ => 10,
    }
}

/// The RNA socket type name as `eNodeSocketDatatype`.
fn socket_type(s: &str) -> i64 {
    match s {
        "VECTOR" => 1,
        "RGBA" => 2,
        "SHADER" => 3,
        "BOOLEAN" => 4,
        "INT" => 6,
        "STRING" => 7,
        "OBJECT" => 8,
        "IMAGE" => 9,
        "GEOMETRY" => 10,
        "COLLECTION" => 11,
        "TEXTURE" => 12,
        "MATERIAL" => 13,
        "ROTATION" => 14,
        "MENU" => 15,
        "MATRIX" => 16,
        _ => 0,
    }
}

/// A socket's type as the `eCustomDataType` an attribute of it has.
fn socket_data_type(t: SocketType) -> i64 {
    match t {
        SocketType::Int => 11,
        SocketType::Bool => 50,
        SocketType::Vector => 48,
        SocketType::Color => 47,
        SocketType::Rotation => 52,
        SocketType::Matrix => 20,
        _ => 10,
    }
}

/// The number in an `Item_<k>`/`Value_<k>` socket identifier.
fn item_id(s: &Socket) -> Option<i64> {
    s.identifier.strip_prefix("Item_").or_else(|| s.identifier.strip_prefix("Value_")).and_then(|k| k.parse().ok())
}

impl W {
    /// Settings of a geometry or function node: its storage struct and
    /// `custom1`/`custom2`. `nodes` is the node's tree, for zone pairing.
    pub(super) fn geo_settings(&mut self, n: &Node, nodes: &[Node]) -> (Option<Rec>, [i64; 2]) {
        let e = |k: &str| n.enum_prop(k).to_string();
        let b = |k: &str| matches!(n.prop(k), Some(Prop::Bool(true))) as i64;
        let dom = |k: &str| ix(&DOMAINS, &e(k));
        let mut c = [0i64; 2];
        // A storage struct with `(member, value)` integer fields.
        let st = |w: &mut W, name: &str, fields: &[(&str, i64)]| {
            let mut r = w.rec(name);
            for (m, v) in fields {
                w.i(&mut r, m, *v);
            }
            Some(r)
        };
        let id = n.idname.as_str();
        let storage = match id {
            "GeometryNodeMeshCylinder" => st(self, "NodeGeometryMeshCylinder", &[("fill_type", ix(&FILL, &e("fill_type")))]),
            "GeometryNodeMeshCone" => st(self, "NodeGeometryMeshCone", &[("fill_type", ix(&FILL, &e("fill_type")))]),
            "GeometryNodeMeshCircle" => st(self, "NodeGeometryMeshCircle", &[("fill_type", ix(&FILL, &e("fill_type")))]),
            "GeometryNodeMeshLine" => st(self, "NodeGeometryMeshLine", &[("mode", ix(&["END_POINTS", "OFFSET"], &e("mode"))), ("count_mode", ix(&["TOTAL", "RESOLUTION"], &e("count_mode")))]),
            "GeometryNodeExtrudeMesh" => st(self, "NodeGeometryExtrudeMesh", &[("mode", ix(&["VERTICES", "EDGES", "FACES"], &e("mode")))]),
            "GeometryNodeStoreNamedAttribute" => st(self, "NodeGeometryStoreNamedAttribute", &[("data_type", data_type(&e("data_type"))), ("domain", dom("domain"))]),
            "GeometryNodeInputNamedAttribute" => st(self, "NodeGeometryInputNamedAttribute", &[("data_type", data_type(&e("data_type")))]),
            "GeometryNodeViewer" => {
                let d = if e("domain") == "AUTO" { -1 } else { dom("domain") };
                st(self, "NodeGeometryViewer", &[("data_type", data_type(&e("data_type"))), ("domain", d)])
            }
            "GeometryNodeAccumulateField" => st(self, "NodeAccumulateField", &[("data_type", data_type(&e("data_type"))), ("domain", dom("domain"))]),
            "GeometryNodeSampleIndex" => st(self, "NodeGeometrySampleIndex", &[("data_type", data_type(&e("data_type"))), ("domain", dom("domain")), ("clamp", b("clamp"))]),
            "GeometryNodeRaycast" => st(self, "NodeGeometryRaycast", &[("mapping", ix(&["INTERPOLATED", "NEAREST"], &e("mapping"))), ("data_type", data_type(&e("data_type")))]),
            "GeometryNodeProximity" => st(self, "NodeGeometryProximity", &[("target_element", ix(&["POINTS", "EDGES", "FACES"], &e("target_element")))]),
            "GeometryNodeDeleteGeometry" => st(self, "NodeGeometryDeleteGeometry", &[("domain", dom("domain")), ("mode", ix(&["ALL", "EDGE_FACE", "ONLY_FACE"], &e("mode")))]),
            "GeometryNodeSeparateGeometry" => st(self, "NodeGeometrySeparateGeometry", &[("domain", dom("domain"))]),
            "GeometryNodeDuplicateElements" => st(self, "NodeGeometryDuplicateElements", &[("domain", dom("domain"))]),
            "GeometryNodeMergeByDistance" => st(self, "NodeGeometryMergeByDistance", &[("mode", ix(&["ALL", "CONNECTED"], &e("mode")))]),
            "GeometryNodeObjectInfo" => st(self, "NodeGeometryObjectInfo", &[("transform_space", ix(&["ORIGINAL", "RELATIVE"], &e("transform_space")))]),
            "GeometryNodeCollectionInfo" => st(self, "NodeGeometryCollectionInfo", &[("transform_space", ix(&["ORIGINAL", "RELATIVE"], &e("transform_space")))]),
            "GeometryNodeMeshToPoints" => st(self, "NodeGeometryMeshToPoints", &[("mode", ix(&["VERTICES", "EDGES", "FACES", "CORNERS"], &e("mode")))]),
            "GeometryNodeCurvePrimitiveCircle" => st(self, "NodeGeometryCurvePrimitiveCircle", &[("mode", ix(&["POINTS", "RADIUS"], &e("mode")))]),
            "GeometryNodeCurvePrimitiveLine" => st(self, "NodeGeometryCurvePrimitiveLine", &[("mode", ix(&["POINTS", "DIRECTION"], &e("mode")))]),
            "GeometryNodeCurveArc" => st(self, "NodeGeometryCurvePrimitiveArc", &[("mode", ix(&["POINTS", "RADIUS"], &e("mode")))]),
            "GeometryNodeCurvePrimitiveQuadrilateral" => st(self, "NodeGeometryCurvePrimitiveQuad", &[("mode", ix(&["RECTANGLE", "PARALLELOGRAM", "TRAPEZOID", "KITE", "POINTS"], &e("mode")))]),
            "GeometryNodeCurvePrimitiveBezierSegment" => st(self, "NodeGeometryCurvePrimitiveBezierSegment", &[("mode", ix(&["POSITION", "OFFSET"], &e("mode")))]),
            "GeometryNodeCurveToPoints" => st(self, "NodeGeometryCurveToPoints", &[("mode", ix(&["EVALUATED", "COUNT", "LENGTH"], &e("mode")))]),
            "GeometryNodeFillCurve" => st(self, "NodeGeometryCurveFill", &[("mode", ix(&["TRIANGLES", "NGONS"], &e("mode")))]),
            "GeometryNodeFilletCurve" => st(self, "NodeGeometryCurveFillet", &[("mode", ix(&["BEZIER", "POLY"], &e("mode")))]),
            "GeometryNodeResampleCurve" => st(self, "NodeGeometryCurveResample", &[("mode", ix(&["EVALUATED", "COUNT", "LENGTH"], &e("mode")))]),
            "GeometryNodeTrimCurve" => st(self, "NodeGeometryCurveTrim", &[("mode", ix(&["FACTOR", "LENGTH"], &e("mode")))]),
            "GeometryNodeSetCurveHandlePositions" => st(self, "NodeGeometrySetCurveHandlePositions", &[("mode", ix(&["LEFT", "RIGHT"], &e("mode")))]),
            "GeometryNodeCurveSetHandles" => {
                // GEO_NODE_CURVE_HANDLE_LEFT 1, RIGHT 2 (flags; both 3).
                let mode = match e("mode").as_str() {
                    "LEFT" => 1,
                    "RIGHT" => 2,
                    _ => 3,
                };
                st(self, "NodeGeometryCurveSetHandles", &[("handle_type", ix(&["FREE", "AUTO", "VECTOR", "ALIGN"], &e("handle_type"))), ("mode", mode)])
            }
            "GeometryNodeCurveSplineType" => st(self, "NodeGeometryCurveSplineType", &[("spline_type", ix(&["CATMULL_ROM", "POLY", "BEZIER", "NURBS"], &e("spline_type")))]),
            "GeometryNodeSampleCurve" => st(self, "NodeGeometryCurveSample", &[("mode", ix(&["FACTOR", "LENGTH"], &e("mode"))), ("data_type", data_type(&e("data_type"))), ("use_all_curves", b("use_all_curves"))]),
            "GeometryNodeSubdivisionSurface" => {
                // SUBSURF_UV_SMOOTH_*; SUBSURF_BOUNDARY_SMOOTH_ALL 0, PRESERVE_CORNERS 1.
                let uv = ix(&["NONE", "PRESERVE_CORNERS", "PRESERVE_CORNERS_AND_JUNCTIONS", "PRESERVE_CORNERS_JUNCTIONS_AND_CONCAVE", "PRESERVE_BOUNDARIES", "SMOOTH_ALL"], &e("uv_smooth"));
                st(self, "NodeGeometrySubdivisionSurface", &[("uv_smooth", uv), ("boundary_smooth", ix(&["ALL", "PRESERVE_CORNERS"], &e("boundary_smooth")))])
            }
            "GeometryNodeSwitch" => st(self, "NodeSwitch", &[("input_type", socket_type(&e("input_type")))]),
            "FunctionNodeRandomValue" => st(self, "NodeRandomValue", &[("data_type", data_type(&e("data_type")))]),
            "FunctionNodeCompare" => st(
                self,
                "NodeFunctionCompare",
                &[
                    ("operation", ix(&["LESS_THAN", "LESS_EQUAL", "GREATER_THAN", "GREATER_EQUAL", "EQUAL", "NOT_EQUAL", "BRIGHTER", "DARKER"], &e("operation"))),
                    ("data_type", socket_type(&e("data_type"))),
                    ("mode", ix(&["ELEMENT", "LENGTH", "AVERAGE", "DOT_PRODUCT", "DIRECTION"], &e("mode"))),
                ],
            ),
            "FunctionNodeCombineColor" | "FunctionNodeSeparateColor" => st(self, "NodeCombSepColor", &[("mode", ix(&["RGB", "HSV", "HSL"], &e("mode")))]),
            "FunctionNodeInputBool" => st(self, "NodeInputBool", &[("boolean", b("boolean"))]),
            "FunctionNodeInputInt" => {
                let v = match n.prop("integer") {
                    Some(Prop::Int(i)) => *i as i64,
                    _ => 0,
                };
                st(self, "NodeInputInt", &[("integer", v)])
            }
            "FunctionNodeInputVector" => {
                let mut r = self.rec("NodeInputVector");
                if let Some(Prop::Vector(v)) = n.prop("vector") {
                    self.f(&mut r, "vector", v);
                }
                Some(r)
            }
            "FunctionNodeInputColor" => {
                let mut r = self.rec("NodeInputColor");
                if let Some(Prop::Color(v)) = n.prop("value") {
                    self.f(&mut r, "color", v);
                }
                Some(r)
            }
            "FunctionNodeInputString" => {
                let mut r = self.rec("NodeInputString");
                let s = match n.prop("string") {
                    Some(Prop::String(s)) => s.clone(),
                    _ => String::new(),
                };
                let a = self.string(&s);
                self.p(&mut r, "string", a);
                Some(r)
            }
            "GeometryNodeRepeatInput" | "GeometryNodeSimulationInput" => {
                let paired = match n.prop("paired_output") {
                    Some(Prop::String(s)) => nodes.iter().position(|o| &o.name == s).map_or(0, |i| i as i64 + 1),
                    _ => 0,
                };
                let sname = if id == "GeometryNodeRepeatInput" { "NodeGeometryRepeatInput" } else { "NodeGeometrySimulationInput" };
                st(self, sname, &[("output_node_id", paired)])
            }
            "GeometryNodeRepeatOutput" | "GeometryNodeSimulationOutput" => {
                let repeat = id == "GeometryNodeRepeatOutput";
                let items: Vec<&Socket> = n.outputs.iter().filter(|s| item_id(s).is_some()).collect();
                let recs: Vec<Rec> = items
                    .iter()
                    .map(|s| {
                        let mut r = self.rec(if repeat { "NodeRepeatItem" } else { "NodeSimulationItem" });
                        let nm = self.string(&s.name);
                        self.p(&mut r, "name", nm);
                        self.i(&mut r, "socket_type", sock_type(s.ty));
                        self.i(&mut r, "identifier", item_id(s).unwrap_or(0));
                        r
                    })
                    .collect();
                let arr = if recs.is_empty() { 0 } else { self.data(&recs) };
                let next = items.iter().filter_map(|s| item_id(s)).max().map_or(0, |k| k + 1);
                let mut r = self.rec(if repeat { "NodeGeometryRepeatOutput" } else { "NodeGeometrySimulationOutput" });
                self.p(&mut r, "items", arr);
                self.i(&mut r, "items_num", recs.len() as i64);
                self.i(&mut r, "next_identifier", next);
                if repeat {
                    if let Some(Prop::Int(i)) = n.prop("inspection_index") {
                        self.i(&mut r, "inspection_index", *i as i64);
                    }
                }
                Some(r)
            }
            "GeometryNodeIndexSwitch" => {
                let ks: Vec<i64> = n.inputs.iter().filter_map(item_id).collect();
                let recs: Vec<Rec> = ks
                    .iter()
                    .map(|k| {
                        let mut r = self.rec("IndexSwitchItem");
                        self.i(&mut r, "identifier", *k);
                        r
                    })
                    .collect();
                let arr = if recs.is_empty() { 0 } else { self.data(&recs) };
                let mut r = self.rec("NodeIndexSwitch");
                self.p(&mut r, "items", arr);
                self.i(&mut r, "items_num", recs.len() as i64);
                self.i(&mut r, "data_type", socket_type(&e("data_type")));
                self.i(&mut r, "next_identifier", ks.iter().max().map_or(0, |k| k + 1));
                Some(r)
            }
            "GeometryNodeMenuSwitch" => {
                let items: Vec<&Socket> = n.inputs.iter().filter(|s| item_id(s).is_some()).collect();
                let recs: Vec<Rec> = items
                    .iter()
                    .map(|s| {
                        let mut r = self.rec("NodeEnumItem");
                        let nm = self.string(&s.name);
                        self.p(&mut r, "name", nm);
                        let d = self.string("");
                        self.p(&mut r, "description", d);
                        self.i(&mut r, "identifier", item_id(s).unwrap_or(0));
                        r
                    })
                    .collect();
                let arr = if recs.is_empty() { 0 } else { self.data(&recs) };
                let mut r = self.rec("NodeMenuSwitch");
                self.p(&mut r, "enum_definition.items_array", arr);
                self.i(&mut r, "enum_definition.items_num", recs.len() as i64);
                self.i(&mut r, "enum_definition.next_identifier", items.iter().filter_map(|s| item_id(s)).max().map_or(0, |k| k + 1));
                self.i(&mut r, "data_type", socket_type(&e("data_type")));
                Some(r)
            }
            "GeometryNodeCaptureAttribute" => {
                // 4.2: one item per captured value (`Value_<identifier>`
                // sockets; Polyamour's first value is `Value`, identifier 0).
                let items: Vec<(i64, &Socket)> = n.inputs.iter().filter(|s| s.identifier == "Value" || item_id(s).is_some()).map(|s| (item_id(s).unwrap_or(0), s)).collect();
                let recs: Vec<Rec> = items
                    .iter()
                    .map(|(k, s)| {
                        let mut r = self.rec("NodeGeometryAttributeCaptureItem");
                        self.i(&mut r, "data_type", socket_data_type(s.ty));
                        self.i(&mut r, "identifier", *k);
                        let nm = self.string(&s.name);
                        self.p(&mut r, "name", nm);
                        r
                    })
                    .collect();
                let arr = if recs.is_empty() { 0 } else { self.data(&recs) };
                let mut r = self.rec("NodeGeometryAttributeCapture");
                self.i(&mut r, "domain", dom("domain"));
                self.p(&mut r, "capture_items", arr);
                self.i(&mut r, "capture_items_num", recs.len() as i64);
                self.i(&mut r, "next_identifier", items.iter().map(|(k, _)| *k).max().map_or(0, |k| k + 1));
                Some(r)
            }
            "GeometryNodeAttributeStatistic" => {
                c = [data_type(&e("data_type")), dom("domain")];
                None
            }
            "GeometryNodeBlurAttribute" | "GeometryNodeSampleNearestSurface" => {
                c[0] = data_type(&e("data_type"));
                None
            }
            "GeometryNodeFieldAtIndex" | "GeometryNodeFieldOnDomain" => {
                c = [dom("domain"), data_type(&e("data_type"))];
                None
            }
            "GeometryNodeSampleNearest" => {
                c[1] = dom("domain");
                None
            }
            "GeometryNodeSortElements" | "GeometryNodeSetShadeSmooth" => {
                c[0] = dom("domain");
                None
            }
            "GeometryNodeScaleElements" => {
                c = [dom("domain"), ix(&["UNIFORM", "SINGLE_AXIS"], &e("scale_mode"))];
                None
            }
            "GeometryNodeAttributeDomainSize" => {
                c[0] = match e("component").as_str() {
                    "POINTCLOUD" => 1,
                    "INSTANCES" => 2,
                    "CURVE" => 4,
                    _ => 0,
                };
                None
            }
            "GeometryNodeDistributePointsOnFaces" => {
                c[0] = ix(&["RANDOM", "POISSON"], &e("distribute_method"));
                None
            }
            "GeometryNodeTriangulate" => {
                c = [ix(&["BEAUTY", "FIXED", "FIXED_ALTERNATE", "SHORTEST_DIAGONAL", "LONGEST_DIAGONAL"], &e("quad_method")), ix(&["BEAUTY", "CLIP"], &e("ngon_method"))];
                None
            }
            "GeometryNodeMeshBoolean" => {
                c = [ix(&["INTERSECT", "UNION", "DIFFERENCE"], &e("operation")), ix(&["EXACT", "FLOAT"], &e("solver"))];
                None
            }
            "FunctionNodeBooleanMath" => {
                c[0] = ix(&["AND", "OR", "NOT", "NAND", "NOR", "XNOR", "XOR", "IMPLY", "NIMPLY"], &e("operation"));
                None
            }
            "FunctionNodeFloatToInt" => {
                c[0] = ix(&["ROUND", "FLOOR", "CEILING", "TRUNCATE"], &e("rounding_mode"));
                None
            }
            _ => None,
        };
        (storage, c)
    }
}
