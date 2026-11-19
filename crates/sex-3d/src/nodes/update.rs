// SPDX-License-Identifier: GPL-3.0-only
//! Blender's node "update" callbacks: which sockets a node's current mode
//! uses (Math's second input for Add but not Sine, Mix's float/vector/colour
//! sockets, Noise's W for 4D, ...) and which properties are shown. Editors
//! call [`update`] after changing an enum property; sockets a mode does not
//! use get `enabled = false` and are not drawn.
use super::Node;

fn enable(n: &mut Node, ident: &str, on: bool) {
    if let Some(s) = n.inputs.iter_mut().find(|s| s.identifier == ident) {
        s.enabled = on;
    }
}

fn enable_out(n: &mut Node, ident: &str, on: bool) {
    if let Some(s) = n.outputs.iter_mut().find(|s| s.identifier == ident) {
        s.enabled = on;
    }
}

fn rename(n: &mut Node, ident: &str, name: &str) {
    if let Some(s) = n.inputs.iter_mut().find(|s| s.identifier == ident) {
        s.name = name.into();
    }
}

/// Number of value inputs Blender's Math node uses for `op`.
pub fn math_arity(op: &str) -> usize {
    match op {
        "SQRT" | "INVERSE_SQRT" | "ABSOLUTE" | "EXPONENT" | "SIGN" | "ROUND" | "FLOOR" | "CEIL" | "TRUNC" | "FRACT" | "SINE" | "COSINE" | "TANGENT" | "ARCSINE" | "ARCCOSINE" | "ARCTANGENT"
        | "SINH" | "COSH" | "TANH" | "RADIANS" | "DEGREES" => 1,
        "MULTIPLY_ADD" | "COMPARE" | "SMOOTH_MIN" | "SMOOTH_MAX" | "WRAP" => 3,
        _ => 2,
    }
}

/// Bring `n`'s socket availability in line with its properties.
pub fn update(n: &mut Node) {
    super::geo_registry::geo_update(n);
    super::comp_registry::comp_update(n);
    match n.idname.as_str() {
        "ShaderNodeMath" | "CompositorNodeMath" | "FunctionNodeMath" => {
            let k = math_arity(n.enum_prop("operation"));
            enable(n, "Value", true);
            enable(n, "Value_001", k >= 2);
            enable(n, "Value_002", k >= 3);
        }
        "ShaderNodeVectorMath" => {
            let op = n.enum_prop("operation").to_string();
            let one = matches!(op.as_str(), "LENGTH" | "NORMALIZE" | "ABSOLUTE" | "SIGN" | "FLOOR" | "CEIL" | "FRACTION" | "SINE" | "COSINE" | "TANGENT" | "SCALE");
            let three = matches!(op.as_str(), "MULTIPLY_ADD" | "FACEFORWARD" | "WRAP");
            enable(n, "Vector_001", !one);
            enable(n, "Vector_002", three);
            enable(n, "Scale", matches!(op.as_str(), "SCALE" | "REFRACT"));
            rename(n, "Scale", if op == "REFRACT" { "IOR" } else { "Scale" });
            let value_out = matches!(op.as_str(), "DOT_PRODUCT" | "DISTANCE" | "LENGTH");
            enable_out(n, "Value", value_out);
            enable_out(n, "Vector", !value_out);
        }
        "ShaderNodeMix" => {
            let dt = n.enum_prop("data_type").to_string();
            let non_uniform = dt == "VECTOR" && n.enum_prop("factor_mode") == "NON_UNIFORM";
            enable(n, "Factor_Float", !non_uniform);
            enable(n, "Factor_Vector", non_uniform);
            for (suffix, on) in [("Float", dt == "FLOAT"), ("Vector", dt == "VECTOR"), ("Color", dt == "RGBA"), ("Rotation", dt == "ROTATION")] {
                enable(n, &format!("A_{suffix}"), on);
                enable(n, &format!("B_{suffix}"), on);
                enable_out(n, &format!("Result_{suffix}"), on);
            }
        }
        "ShaderNodeMapRange" => {
            let vector = n.enum_prop("data_type") == "FLOAT_VECTOR";
            let stepped = n.enum_prop("interpolation_type") == "STEPPED";
            for id in ["Value", "From Min", "From Max", "To Min", "To Max"] {
                enable(n, id, !vector);
            }
            for id in ["Vector", "From_Min_FLOAT3", "From_Max_FLOAT3", "To_Min_FLOAT3", "To_Max_FLOAT3"] {
                enable(n, id, vector);
            }
            enable(n, "Steps", !vector && stepped);
            enable(n, "Steps_FLOAT3", vector && stepped);
            enable_out(n, "Result", !vector);
            enable_out(n, "Vector", vector);
        }
        "ShaderNodeTexNoise" | "ShaderNodeTexVoronoi" | "ShaderNodeTexWhiteNoise" | "ShaderNodeTexMusgrave" => {
            let key = match n.idname.as_str() {
                "ShaderNodeTexVoronoi" => "voronoi_dimensions",
                "ShaderNodeTexMusgrave" => "musgrave_dimensions",
                _ => "noise_dimensions",
            };
            let dims = n.enum_prop(key).to_string();
            enable(n, "Vector", dims != "1D");
            enable(n, "W", dims == "1D" || dims == "4D");
            if n.idname == "ShaderNodeTexNoise" || n.idname == "ShaderNodeTexMusgrave" {
                let ty = n.enum_prop(if n.idname == "ShaderNodeTexNoise" { "noise_type" } else { "musgrave_type" }).to_string();
                if !ty.is_empty() {
                    enable(n, "Offset", matches!(ty.as_str(), "RIDGED_MULTIFRACTAL" | "HYBRID_MULTIFRACTAL" | "HETERO_TERRAIN"));
                    enable(n, "Gain", matches!(ty.as_str(), "RIDGED_MULTIFRACTAL" | "HYBRID_MULTIFRACTAL"));
                }
            }
            if n.idname == "ShaderNodeTexVoronoi" {
                let f = n.enum_prop("feature").to_string();
                let edge_or_radius = matches!(f.as_str(), "DISTANCE_TO_EDGE" | "N_SPHERE_RADIUS");
                enable(n, "Smoothness", f == "SMOOTH_F1");
                enable(n, "Exponent", n.enum_prop("distance") == "MINKOWSKI" && !edge_or_radius);
                enable_out(n, "Distance", f != "N_SPHERE_RADIUS");
                enable_out(n, "Color", !edge_or_radius);
                enable_out(n, "Position", !edge_or_radius && dims != "1D");
                enable_out(n, "W", !edge_or_radius && (dims == "1D" || dims == "4D"));
                enable_out(n, "Radius", f == "N_SPHERE_RADIUS");
            }
        }
        "ShaderNodeMapping" => {
            let t = n.enum_prop("vector_type").to_string();
            enable(n, "Location", !matches!(t.as_str(), "VECTOR" | "NORMAL"));
        }
        "ShaderNodeSeparateColor" | "ShaderNodeCombineColor" => {
            let names = match n.enum_prop("mode") {
                "HSV" => ["Hue", "Saturation", "Value"],
                "HSL" => ["Hue", "Saturation", "Lightness"],
                _ => ["Red", "Green", "Blue"],
            };
            let ids = ["Red", "Green", "Blue"];
            let sockets = if n.idname == "ShaderNodeSeparateColor" { &mut n.outputs } else { &mut n.inputs };
            for (id, name) in ids.iter().zip(names) {
                if let Some(s) = sockets.iter_mut().find(|s| s.identifier == *id) {
                    s.name = name.into();
                }
            }
        }
        _ => {}
    }
}

/// Whether the node editor draws property `prop` for the node's current mode.
pub fn prop_visible(n: &Node, prop: &str) -> bool {
    match (n.idname.as_str(), prop) {
        // Shown by the editor in its own way (data-block row) or internal.
        (_, "is_active_output" | "label_size" | "shrink" | "height") => false,
        ("ShaderNodeMix", "blend_type" | "clamp_result") => n.enum_prop("data_type") == "RGBA",
        ("ShaderNodeMix", "factor_mode") => n.enum_prop("data_type") == "VECTOR",
        ("ShaderNodeTexWave", "bands_direction") => n.enum_prop("wave_type") != "RINGS",
        ("ShaderNodeTexWave", "rings_direction") => n.enum_prop("wave_type") == "RINGS",
        ("ShaderNodeTexVoronoi", "distance") => !matches!(n.enum_prop("feature"), "N_SPHERE_RADIUS"),
        ("ShaderNodeTexImage", "projection_blend") => n.enum_prop("projection") == "BOX",
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::{new_node, Prop};

    #[test]
    fn math_sockets_follow_operation() {
        let mut m = new_node("ShaderNodeMath").unwrap();
        m.set_prop("operation", Prop::Enum("SINE".into()));
        update(&mut m);
        assert!(m.inputs[0].enabled && !m.inputs[1].enabled && !m.inputs[2].enabled);
        m.set_prop("operation", Prop::Enum("MULTIPLY_ADD".into()));
        update(&mut m);
        assert!(m.inputs.iter().all(|s| s.enabled));
    }

    #[test]
    fn mix_sockets_follow_data_type() {
        let mut m = new_node("ShaderNodeMix").unwrap();
        m.set_prop("data_type", Prop::Enum("FLOAT".into()));
        update(&mut m);
        let on: Vec<&str> = m.inputs.iter().filter(|s| s.enabled).map(|s| s.identifier.as_str()).collect();
        assert_eq!(on, ["Factor_Float", "A_Float", "B_Float"]);
        assert!(!prop_visible(&m, "blend_type"));
    }
}
