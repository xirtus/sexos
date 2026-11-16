// SPDX-License-Identifier: GPL-3.0-only
//! The choices of node enum properties, as Blender's RNA lists them:
//! (identifier, UI label) for each (node `bl_idname`, property). Editors use
//! this for dropdowns; an unknown property shows its raw identifier.

type Items = &'static [(&'static str, &'static str)];

const MATH_OPS: Items = &[
    ("ADD", "Add"),
    ("SUBTRACT", "Subtract"),
    ("MULTIPLY", "Multiply"),
    ("DIVIDE", "Divide"),
    ("MULTIPLY_ADD", "Multiply Add"),
    ("POWER", "Power"),
    ("LOGARITHM", "Logarithm"),
    ("SQRT", "Square Root"),
    ("INVERSE_SQRT", "Inverse Square Root"),
    ("ABSOLUTE", "Absolute"),
    ("EXPONENT", "Exponent"),
    ("MINIMUM", "Minimum"),
    ("MAXIMUM", "Maximum"),
    ("LESS_THAN", "Less Than"),
    ("GREATER_THAN", "Greater Than"),
    ("SIGN", "Sign"),
    ("COMPARE", "Compare"),
    ("SMOOTH_MIN", "Smooth Minimum"),
    ("SMOOTH_MAX", "Smooth Maximum"),
    ("ROUND", "Round"),
    ("FLOOR", "Floor"),
    ("CEIL", "Ceil"),
    ("TRUNC", "Truncate"),
    ("FRACT", "Fraction"),
    ("MODULO", "Truncated Modulo"),
    ("FLOORED_MODULO", "Floored Modulo"),
    ("WRAP", "Wrap"),
    ("SNAP", "Snap"),
    ("PINGPONG", "Ping-Pong"),
    ("SINE", "Sine"),
    ("COSINE", "Cosine"),
    ("TANGENT", "Tangent"),
    ("ARCSINE", "Arcsine"),
    ("ARCCOSINE", "Arccosine"),
    ("ARCTANGENT", "Arctangent"),
    ("ARCTAN2", "Arctan2"),
    ("SINH", "Hyperbolic Sine"),
    ("COSH", "Hyperbolic Cosine"),
    ("TANH", "Hyperbolic Tangent"),
    ("RADIANS", "To Radians"),
    ("DEGREES", "To Degrees"),
];

const VECTOR_MATH_OPS: Items = &[
    ("ADD", "Add"),
    ("SUBTRACT", "Subtract"),
    ("MULTIPLY", "Multiply"),
    ("DIVIDE", "Divide"),
    ("MULTIPLY_ADD", "Multiply Add"),
    ("CROSS_PRODUCT", "Cross Product"),
    ("PROJECT", "Project"),
    ("REFLECT", "Reflect"),
    ("REFRACT", "Refract"),
    ("FACEFORWARD", "Faceforward"),
    ("DOT_PRODUCT", "Dot Product"),
    ("DISTANCE", "Distance"),
    ("LENGTH", "Length"),
    ("SCALE", "Scale"),
    ("NORMALIZE", "Normalize"),
    ("ABSOLUTE", "Absolute"),
    ("POWER", "Power"),
    ("SIGN", "Sign"),
    ("MINIMUM", "Minimum"),
    ("MAXIMUM", "Maximum"),
    ("FLOOR", "Floor"),
    ("CEIL", "Ceil"),
    ("FRACTION", "Fraction"),
    ("MODULO", "Truncated Modulo"),
    ("WRAP", "Wrap"),
    ("SNAP", "Snap"),
    ("SINE", "Sine"),
    ("COSINE", "Cosine"),
    ("TANGENT", "Tangent"),
];

const BLEND_TYPES: Items = &[
    ("MIX", "Mix"),
    ("DARKEN", "Darken"),
    ("MULTIPLY", "Multiply"),
    ("BURN", "Color Burn"),
    ("LIGHTEN", "Lighten"),
    ("SCREEN", "Screen"),
    ("DODGE", "Color Dodge"),
    ("ADD", "Add"),
    ("OVERLAY", "Overlay"),
    ("SOFT_LIGHT", "Soft Light"),
    ("LINEAR_LIGHT", "Linear Light"),
    ("DIFFERENCE", "Difference"),
    ("EXCLUSION", "Exclusion"),
    ("SUBTRACT", "Subtract"),
    ("DIVIDE", "Divide"),
    ("HUE", "Hue"),
    ("SATURATION", "Saturation"),
    ("COLOR", "Color"),
    ("VALUE", "Value"),
];

const MIX_DATA_TYPES: Items = &[("FLOAT", "Float"), ("VECTOR", "Vector"), ("RGBA", "Color"), ("ROTATION", "Rotation")];
const FACTOR_MODES: Items = &[("UNIFORM", "Uniform"), ("NON_UNIFORM", "Non-Uniform")];
const IMAGE_INTERP: Items = &[("Linear", "Linear"), ("Closest", "Closest"), ("Cubic", "Cubic"), ("Smart", "Smart")];
const IMAGE_PROJECTION: Items = &[("FLAT", "Flat"), ("BOX", "Box"), ("SPHERE", "Sphere"), ("TUBE", "Tube")];
const ENV_PROJECTION: Items = &[("EQUIRECTANGULAR", "Equirectangular"), ("MIRROR_BALL", "Mirror Ball")];
const IMAGE_EXTENSION: Items = &[("REPEAT", "Repeat"), ("EXTEND", "Extend"), ("CLIP", "Clip"), ("MIRROR", "Mirror")];
const DIMENSIONS: Items = &[("1D", "1D"), ("2D", "2D"), ("3D", "3D"), ("4D", "4D")];
const NOISE_TYPES: Items = &[
    ("MULTIFRACTAL", "Multifractal"),
    ("RIDGED_MULTIFRACTAL", "Ridged Multifractal"),
    ("HYBRID_MULTIFRACTAL", "Hybrid Multifractal"),
    ("FBM", "fBM"),
    ("HETERO_TERRAIN", "Hetero Terrain"),
];
const VORONOI_FEATURES: Items = &[("F1", "F1"), ("F2", "F2"), ("SMOOTH_F1", "Smooth F1"), ("DISTANCE_TO_EDGE", "Distance to Edge"), ("N_SPHERE_RADIUS", "N-Sphere Radius")];
const VORONOI_METRICS: Items = &[("EUCLIDEAN", "Euclidean"), ("MANHATTAN", "Manhattan"), ("CHEBYCHEV", "Chebychev"), ("MINKOWSKI", "Minkowski")];
const WAVE_TYPES: Items = &[("BANDS", "Bands"), ("RINGS", "Rings")];
const BANDS_DIRECTION: Items = &[("X", "X"), ("Y", "Y"), ("Z", "Z"), ("DIAGONAL", "Diagonal")];
const RINGS_DIRECTION: Items = &[("X", "X"), ("Y", "Y"), ("Z", "Z"), ("SPHERICAL", "Spherical")];
const WAVE_PROFILES: Items = &[("SIN", "Sine"), ("SAW", "Saw"), ("TRI", "Triangle")];
const GRADIENT_TYPES: Items = &[
    ("LINEAR", "Linear"),
    ("QUADRATIC", "Quadratic"),
    ("EASING", "Easing"),
    ("DIAGONAL", "Diagonal"),
    ("SPHERICAL", "Spherical"),
    ("QUADRATIC_SPHERE", "Quadratic Sphere"),
    ("RADIAL", "Radial"),
];
const SKY_TYPES: Items = &[("NISHITA", "Nishita"), ("HOSEK_WILKIE", "Hosek / Wilkie"), ("PREETHAM", "Preetham")];
const MAPPING_TYPES: Items = &[("POINT", "Point"), ("TEXTURE", "Texture"), ("VECTOR", "Vector"), ("NORMAL", "Normal")];
const PRINCIPLED_DIST: Items = &[("GGX", "GGX"), ("MULTI_GGX", "Multiscatter GGX")];
const GLOSSY_DIST: Items = &[("BECKMANN", "Beckmann"), ("GGX", "GGX"), ("ASHIKHMIN_SHIRLEY", "Ashikhmin-Shirley"), ("MULTI_GGX", "Multiscatter GGX")];
const GLASS_DIST: Items = &[("BECKMANN", "Beckmann"), ("GGX", "GGX"), ("MULTI_GGX", "Multiscatter GGX")];
const SSS_METHODS: Items = &[("BURLEY", "Christensen-Burley"), ("RANDOM_WALK", "Random Walk"), ("RANDOM_WALK_SKIN", "Random Walk (Skin)")];
const SHEEN_DIST: Items = &[("ASHIKHMIN", "Ashikhmin"), ("MICROFIBER", "Microfiber")];
const MAP_RANGE_TYPES: Items = &[("FLOAT", "Float"), ("FLOAT_VECTOR", "Vector")];
const MAP_RANGE_INTERP: Items = &[("LINEAR", "Linear"), ("STEPPED", "Stepped Linear"), ("SMOOTHSTEP", "Smooth Step"), ("SMOOTHERSTEP", "Smoother Step")];
const CLAMP_TYPES: Items = &[("MINMAX", "Min Max"), ("RANGE", "Range")];
const NORMAL_MAP_SPACES: Items = &[
    ("TANGENT", "Tangent Space"),
    ("OBJECT", "Object Space"),
    ("WORLD", "World Space"),
    ("BLENDER_OBJECT", "Blender Object Space"),
    ("BLENDER_WORLD", "Blender World Space"),
];
const DISPLACEMENT_SPACES: Items = &[("OBJECT", "Object Space"), ("WORLD", "World Space")];
const VECTOR_DISPLACEMENT_SPACES: Items = &[("TANGENT", "Tangent Space"), ("OBJECT", "Object Space"), ("WORLD", "World Space")];
const VECTOR_TYPES: Items = &[("POINT", "Point"), ("VECTOR", "Vector"), ("NORMAL", "Normal")];
const SPACES: Items = &[("WORLD", "World"), ("OBJECT", "Object"), ("CAMERA", "Camera")];
const ROTATION_TYPES: Items = &[("AXIS_ANGLE", "Axis Angle"), ("X_AXIS", "X Axis"), ("Y_AXIS", "Y Axis"), ("Z_AXIS", "Z Axis"), ("EULER_XYZ", "Euler")];
const OUTPUT_TARGETS: Items = &[("ALL", "All"), ("EEVEE", "EEVEE"), ("CYCLES", "Cycles")];
const RAMP_INTERP: Items = &[("EASE", "Ease"), ("CARDINAL", "Cardinal"), ("LINEAR", "Linear"), ("B_SPLINE", "B-Spline"), ("CONSTANT", "Constant")];
const COLOR_MODES: Items = &[("RGB", "RGB"), ("HSV", "HSV"), ("HSL", "HSL")];
const ATTRIBUTE_TYPES: Items = &[("GEOMETRY", "Geometry"), ("OBJECT", "Object"), ("INSTANCER", "Instancer"), ("VIEW_LAYER", "View Layer")];
const TANGENT_DIRECTIONS: Items = &[("RADIAL", "Radial"), ("UV_MAP", "UV Map")];
const AXES: Items = &[("X", "X"), ("Y", "Y"), ("Z", "Z")];
const TOON_COMPONENTS: Items = &[("DIFFUSE", "Diffuse"), ("GLOSSY", "Glossy")];
const METALLIC_FRESNEL: Items = &[("PHYSICAL_CONDUCTOR", "Physical Conductor"), ("F82", "F82 Tint")];
const HAIR_COMPONENTS: Items = &[("Reflection", "Reflection"), ("Transmission", "Transmission")];
const HAIR_PARAMETRIZATION: Items = &[("ABSORPTION", "Absorption Coefficient"), ("MELANIN", "Melanin Concentration"), ("COLOR", "Direct Coloring")];
const HAIR_MODELS: Items = &[("CHIANG", "Chiang"), ("HUANG", "Huang")];
const WHITE_NOISE_DIMS: Items = DIMENSIONS;
const GABOR_TYPES: Items = &[("2D", "2D"), ("3D", "3D")];

/// The choices for `prop` on a node of type `idname`; empty when unknown.
pub fn items(idname: &str, prop: &str) -> Items {
    if idname.starts_with("CompositorNode") {
        let c = super::comp_registry::comp_items(idname, prop);
        if !c.is_empty() {
            return c;
        }
    }
    match (idname, prop) {
        ("ShaderNodeMath" | "CompositorNodeMath" | "FunctionNodeMath" | "GeometryNodeMath", "operation") => MATH_OPS,
        ("ShaderNodeVectorMath" | "GeometryNodeVectorMath", "operation") => VECTOR_MATH_OPS,
        ("ShaderNodeMix" | "GeometryNodeMix", "data_type") => MIX_DATA_TYPES,
        ("ShaderNodeMix", "factor_mode") => FACTOR_MODES,
        (_, "blend_type") => BLEND_TYPES,
        ("ShaderNodeTexImage" | "ShaderNodeTexEnvironment", "interpolation") => IMAGE_INTERP,
        ("ShaderNodeTexImage", "projection") => IMAGE_PROJECTION,
        ("ShaderNodeTexEnvironment", "projection") => ENV_PROJECTION,
        ("ShaderNodeTexImage", "extension") => IMAGE_EXTENSION,
        ("ShaderNodeTexWhiteNoise", "noise_dimensions") => WHITE_NOISE_DIMS,
        ("ShaderNodeTexGabor", "gabor_type") => GABOR_TYPES,
        (_, "noise_dimensions" | "voronoi_dimensions" | "musgrave_dimensions") => DIMENSIONS,
        (_, "noise_type" | "musgrave_type") => NOISE_TYPES,
        (_, "feature") => VORONOI_FEATURES,
        ("ShaderNodeTexVoronoi", "distance") => VORONOI_METRICS,
        ("ShaderNodeTexWave", "wave_type") => WAVE_TYPES,
        ("ShaderNodeTexWave", "bands_direction") => BANDS_DIRECTION,
        ("ShaderNodeTexWave", "rings_direction") => RINGS_DIRECTION,
        ("ShaderNodeTexWave", "wave_profile") => WAVE_PROFILES,
        ("ShaderNodeTexGradient", "gradient_type") => GRADIENT_TYPES,
        ("ShaderNodeTexSky", "sky_type") => SKY_TYPES,
        ("ShaderNodeMapping", "vector_type") => MAPPING_TYPES,
        ("ShaderNodeBsdfPrincipled", "distribution") => PRINCIPLED_DIST,
        ("ShaderNodeBsdfPrincipled", "subsurface_method") => SSS_METHODS,
        ("ShaderNodeSubsurfaceScattering", "falloff") => SSS_METHODS,
        ("ShaderNodeBsdfGlass" | "ShaderNodeBsdfRefraction", "distribution") => GLASS_DIST,
        ("ShaderNodeBsdfSheen", "distribution") => SHEEN_DIST,
        ("ShaderNodeBsdfMetallic", "fresnel_type") => METALLIC_FRESNEL,
        (_, "distribution") => GLOSSY_DIST,
        ("ShaderNodeMapRange" | "GeometryNodeMapRange", "data_type") => MAP_RANGE_TYPES,
        ("ShaderNodeMapRange" | "GeometryNodeMapRange", "interpolation_type") => MAP_RANGE_INTERP,
        (_, "clamp_type") => CLAMP_TYPES,
        ("ShaderNodeNormalMap", "space") => NORMAL_MAP_SPACES,
        ("ShaderNodeDisplacement", "space") => DISPLACEMENT_SPACES,
        ("ShaderNodeVectorDisplacement", "space") => VECTOR_DISPLACEMENT_SPACES,
        ("ShaderNodeVectorTransform", "vector_type") => VECTOR_TYPES,
        ("ShaderNodeVectorTransform", "convert_from" | "convert_to") => SPACES,
        (_, "rotation_type") => ROTATION_TYPES,
        (_, "target") => OUTPUT_TARGETS,
        (_, "interpolation") => RAMP_INTERP,
        (_, "color_mode") => COLOR_MODES,
        ("ShaderNodeSeparateColor" | "ShaderNodeCombineColor" | "CompositorNodeSeparateColor" | "CompositorNodeCombineColor", "mode") => COLOR_MODES,
        ("ShaderNodeAttribute", "attribute_type") => ATTRIBUTE_TYPES,
        ("ShaderNodeTangent", "direction_type") => TANGENT_DIRECTIONS,
        ("ShaderNodeTangent", "axis") => AXES,
        ("ShaderNodeBsdfToon", "component") => TOON_COMPONENTS,
        ("ShaderNodeBsdfHair", "component") => HAIR_COMPONENTS,
        ("ShaderNodeBsdfHairPrincipled", "parametrization") => HAIR_PARAMETRIZATION,
        ("ShaderNodeBsdfHairPrincipled", "model") => HAIR_MODELS,
        _ => &[],
    }
}

/// The UI label of `value` (falls back to the raw identifier).
pub fn label<'a>(idname: &str, prop: &str, value: &'a str) -> &'a str {
    items(idname, prop).iter().find(|i| i.0 == value).map(|i| i.1).unwrap_or(value)
}

/// A property's UI name from its RNA name: "use_clamp" -> "Clamp",
/// "noise_dimensions" -> "Noise Dimensions".
pub fn prop_label(name: &str) -> String {
    let n = name.strip_prefix("use_").unwrap_or(name);
    n.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_fallback_labels() {
        assert_eq!(label("ShaderNodeMath", "operation", "MULTIPLY_ADD"), "Multiply Add");
        assert_eq!(label("ShaderNodeBsdfPrincipled", "distribution", "MULTI_GGX"), "Multiscatter GGX");
        assert_eq!(label("ShaderNodeSomethingNew", "mode_x", "ODD"), "ODD");
        assert_eq!(prop_label("use_clamp"), "Clamp");
        assert_eq!(prop_label("noise_dimensions"), "Noise Dimensions");
        assert!(items("ShaderNodeMath", "operation").len() > 30);
    }
}
