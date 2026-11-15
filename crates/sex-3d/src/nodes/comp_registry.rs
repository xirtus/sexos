// SPDX-License-Identifier: GPL-3.0-only
//! Compositor node types (`CompositorNodeTree`) with Blender 4.2 idnames,
//! socket names/identifiers, defaults, RNA property names and Add-menu
//! categories (ported from `node_composite_*.cc` declarations and
//! `rna_nodetree.cc`). Evaluation lives in [`crate::compositor`].
//!
//! Properties use the RNA names Blender exposes to Python (`filter_type`,
//! `size_x`, `glare_type`, ...), so scripts and the `.blend` reader address
//! them the same way.
use super::registry::{Category, NodeType};
use super::{ColorRamp, CurveMapping, Node, Prop, Socket, SocketType as T, Subtype as S, TreeKind, Value as V};
use crate::render::Pass;

fn n(idname: &str, label: &str, width: f32) -> Node {
    let mut node = Node::new(idname, label);
    node.width = width;
    node
}
fn fl(name: &str, v: f32) -> Socket {
    Socket::new(name, T::Float, V::Float(v))
}
fn flr(name: &str, v: f32, min: f32, max: f32) -> Socket {
    fl(name, v).range(min, max)
}
fn fac(name: &str, v: f32) -> Socket {
    fl(name, v).subtype(S::Factor)
}
fn col(name: &str, c: [f32; 4]) -> Socket {
    Socket::new(name, T::Color, V::Color(c))
}
fn img(name: &str) -> Socket {
    col(name, [1.0; 4])
}
fn vec(name: &str, v: [f32; 3]) -> Socket {
    Socket::new(name, T::Vector, V::Vector(v))
}
fn o_c(name: &str) -> Socket {
    Socket::new(name, T::Color, V::Color([0.0, 0.0, 0.0, 1.0]))
}
fn o_f(name: &str) -> Socket {
    Socket::new(name, T::Float, V::Float(0.0))
}
fn o_v(name: &str) -> Socket {
    Socket::new(name, T::Vector, V::Vector([0.0; 3]))
}
fn e(s: &str) -> Prop {
    Prop::Enum(s.into())
}
fn b(v: bool) -> Prop {
    Prop::Bool(v)
}
fn i(v: i32) -> Prop {
    Prop::Int(v)
}
fn f(v: f32) -> Prop {
    Prop::Float(v)
}
fn node(idname: &str, label: &str, width: f32, ins: Vec<Socket>, outs: Vec<Socket>, props: &[(&str, Prop)]) -> Node {
    let mut x = n(idname, label, width);
    x.inputs = ins;
    x.outputs = outs;
    x.props = props.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    x
}
fn img_out(idname: &str, label: &str, ins: Vec<Socket>, props: &[(&str, Prop)]) -> Node {
    node(idname, label, 140.0, ins, vec![o_c("Image")], props)
}

// ---------------------------------------------------------------------------
// Input

/// Outputs of Render Layers: Image, Alpha, then one per renderer pass under
/// Blender's pass name. Pass outputs start disabled; [`set_render_passes`]
/// enables those of the view layer.
fn render_layers() -> Node {
    let mut outs = vec![o_c("Image"), o_f("Alpha")];
    for p in Pass::ALL.iter().skip(1) {
        let s = match p.channels() {
            1 => o_f(p.name()),
            _ if matches!(p, Pass::Normal | Pass::Position | Pass::Uv | Pass::DenoisingNormal) => o_v(p.name()),
            _ => o_c(p.name()),
        };
        let mut s = s;
        s.enabled = false;
        outs.push(s);
    }
    node("CompositorNodeRLayers", "Render Layers", 240.0, vec![], outs, &[("scene", Prop::Id(None)), ("layer", Prop::String("ViewLayer".into()))])
}

/// Enable the Render Layers outputs of the passes in `passes` (Image and
/// Alpha always), like Blender's `node_cmp_rlayers_outputs` availability.
pub fn set_render_passes(node: &mut Node, passes: crate::render::Passes) {
    for s in node.outputs.iter_mut().skip(2) {
        if let Some(p) = Pass::ALL.iter().find(|p| p.name() == s.identifier) {
            s.enabled = passes.contains(*p);
        }
    }
}

/// The pass behind a Render Layers output identifier.
pub fn pass_of_output(identifier: &str) -> Option<Pass> {
    Pass::ALL.iter().skip(1).copied().find(|p| p.name() == identifier)
}

fn image() -> Node {
    node(
        "CompositorNodeImage",
        "Image",
        240.0,
        vec![],
        vec![o_c("Image"), o_f("Alpha")],
        &[("image", Prop::Id(None)), ("use_straight_alpha_output", b(false)), ("frame_duration", i(1)), ("frame_start", i(1)), ("frame_offset", i(0))],
    )
}
fn mask() -> Node {
    node(
        "CompositorNodeMask",
        "Mask",
        140.0,
        vec![],
        vec![o_f("Mask")],
        &[("mask", Prop::Id(None)), ("use_feather", b(true)), ("size_source", e("SCENE")), ("size_x", i(256)), ("size_y", i(256)), ("use_motion_blur", b(false)), ("motion_blur_samples", i(16)), ("motion_blur_shutter", f(0.5))],
    )
}
fn movie_clip() -> Node {
    node(
        "CompositorNodeMovieClip",
        "Movie Clip",
        240.0,
        vec![],
        vec![o_c("Image"), o_f("Alpha"), o_f("Offset X"), o_f("Offset Y"), o_f("Scale"), o_f("Angle")],
        &[("clip", Prop::Id(None))],
    )
}
fn rgb() -> Node {
    let mut x = node("CompositorNodeRGB", "RGB", 140.0, vec![], vec![o_c("RGBA")], &[]);
    x.outputs[0].value = V::Color([0.5, 0.5, 0.5, 1.0]);
    x
}
fn value() -> Node {
    let mut x = node("CompositorNodeValue", "Value", 140.0, vec![], vec![o_f("Value")], &[]);
    x.outputs[0].value = V::Float(0.5);
    x
}
fn texture() -> Node {
    node(
        "CompositorNodeTexture",
        "Texture",
        140.0,
        vec![vec("Offset", [0.0; 3]).range(-2.0, 2.0).subtype(S::Translation), vec("Scale", [1.0; 3]).range(-10.0, 10.0).subtype(S::Xyz)],
        vec![o_f("Value"), o_c("Color")],
        &[("texture", Prop::Id(None)), ("node_output", i(0))],
    )
}
fn bokeh_image() -> Node {
    node(
        "CompositorNodeBokehImage",
        "Bokeh Image",
        140.0,
        vec![],
        vec![o_c("Image")],
        &[("angle", f(0.0)), ("flaps", i(5)), ("rounding", f(0.0)), ("catadioptric", f(0.0)), ("shift", f(0.0))],
    )
}
fn time_curve() -> Node {
    let curve = CurveMapping { curves: vec![vec![[0.0, 0.0], [1.0, 1.0]]] };
    node("CompositorNodeTime", "Time Curve", 200.0, vec![], vec![o_f("Fac")], &[("curve", Prop::Curve(curve)), ("frame_start", i(1)), ("frame_end", i(250))])
}
fn scene_time() -> Node {
    node("CompositorNodeSceneTime", "Scene Time", 140.0, vec![], vec![o_f("Seconds"), o_f("Frame")], &[])
}

// ---------------------------------------------------------------------------
// Output

fn composite() -> Node {
    node("CompositorNodeComposite", "Composite", 140.0, vec![col("Image", [0.0, 0.0, 0.0, 1.0]), flr("Alpha", 1.0, 0.0, 1.0)], vec![], &[("use_alpha", b(true))])
}
fn viewer() -> Node {
    node("CompositorNodeViewer", "Viewer", 140.0, vec![col("Image", [0.0, 0.0, 0.0, 1.0]), flr("Alpha", 1.0, 0.0, 1.0)], vec![], &[("use_alpha", b(true))])
}
/// File Output: one input per file slot; slot paths live in the
/// `file_slots` string prop, one `path` per line, in input order.
fn file_output() -> Node {
    node(
        "CompositorNodeOutputFile",
        "File Output",
        240.0,
        vec![col("Image", [0.0, 0.0, 0.0, 1.0])],
        vec![],
        &[("base_path", Prop::String("//".into())), ("file_slots", Prop::String("Image".into())), ("file_format", e("PNG")), ("color_mode", e("RGBA")), ("color_depth", e("8"))],
    )
}

// ---------------------------------------------------------------------------
// Color

fn alpha_over() -> Node {
    img_out("CompositorNodeAlphaOver", "Alpha Over", vec![fac("Fac", 1.0), img("Image"), img("Image").ident("Image_001")], &[("use_premultiply", b(false)), ("premul", f(0.0))])
}
fn bright_contrast() -> Node {
    img_out("CompositorNodeBrightContrast", "Brightness/Contrast", vec![img("Image"), flr("Bright", 0.0, -100.0, 100.0), flr("Contrast", 0.0, -100.0, 100.0)], &[("use_premultiply", b(true))])
}
fn color_balance() -> Node {
    img_out(
        "CompositorNodeColorBalance",
        "Color Balance",
        vec![fac("Fac", 1.0), img("Image")],
        &[
            ("correction_method", e("LIFT_GAMMA_GAIN")),
            ("lift", Prop::Color([1.0; 4])),
            ("gamma", Prop::Color([1.0; 4])),
            ("gain", Prop::Color([1.0; 4])),
            ("offset", Prop::Color([0.0, 0.0, 0.0, 1.0])),
            ("power", Prop::Color([1.0; 4])),
            ("slope", Prop::Color([1.0; 4])),
            ("offset_basis", f(0.0)),
        ],
    )
}
fn color_correction() -> Node {
    let mut p: Vec<(String, Prop)> = vec![
        ("red".into(), b(true)),
        ("green".into(), b(true)),
        ("blue".into(), b(true)),
        ("midtones_start".into(), f(0.2)),
        ("midtones_end".into(), f(0.7)),
    ];
    for r in ["master", "highlights", "midtones", "shadows"] {
        for (k, v) in [("saturation", 1.0), ("contrast", 1.0), ("gamma", 1.0), ("gain", 1.0), ("lift", 0.0)] {
            p.push((format!("{r}_{k}"), f(v)));
        }
    }
    let mut x = img_out("CompositorNodeColorCorrection", "Color Correction", vec![img("Image"), flr("Mask", 1.0, 0.0, 1.0)], &[]);
    x.width = 400.0;
    x.props = p;
    x
}
fn exposure() -> Node {
    img_out("CompositorNodeExposure", "Exposure", vec![img("Image"), flr("Exposure", 0.0, -10.0, 10.0)], &[])
}
fn gamma() -> Node {
    img_out("CompositorNodeGamma", "Gamma", vec![img("Image"), flr("Gamma", 1.0, 0.001, 10.0)], &[])
}
/// Hue Correct: three curves (hue, saturation, value) over hue, flat at 0.5.
fn hue_correct() -> Node {
    let flat: Vec<[f32; 2]> = (0..9).map(|k| [k as f32 / 8.0, 0.5]).collect();
    let m = CurveMapping { curves: vec![flat; 3] };
    let mut x = img_out("CompositorNodeHueCorrect", "Hue Correct", vec![fac("Fac", 1.0), img("Image")], &[("mapping", Prop::Curve(m))]);
    x.width = 320.0;
    x
}
fn hue_sat() -> Node {
    img_out(
        "CompositorNodeHueSat",
        "Hue/Saturation/Value",
        vec![img("Image"), fac("Hue", 0.5), fac("Saturation", 1.0).range(0.0, 2.0), fac("Value", 1.0).range(0.0, 2.0), fac("Fac", 1.0)],
        &[],
    )
}
fn invert() -> Node {
    node("CompositorNodeInvert", "Invert Color", 140.0, vec![fac("Fac", 1.0), img("Color")], vec![o_c("Color")], &[("invert_rgb", b(true)), ("invert_alpha", b(false))])
}
fn mix_rgb() -> Node {
    img_out("CompositorNodeMixRGB", "Mix", vec![fac("Fac", 1.0), img("Image"), img("Image").ident("Image_001")], &[("blend_type", e("MIX")), ("use_alpha", b(false)), ("use_clamp", b(false))])
}
fn rgb_curves() -> Node {
    let mut x = img_out(
        "CompositorNodeCurveRGB",
        "RGB Curves",
        vec![fac("Fac", 1.0).range(-1.0, 1.0), img("Image"), col("Black Level", [0.0, 0.0, 0.0, 1.0]), img("White Level")],
        &[("mapping", Prop::Curve(CurveMapping::default()))],
    );
    x.width = 200.0;
    x
}
fn tonemap() -> Node {
    let mut x = img_out(
        "CompositorNodeTonemap",
        "Tonemap",
        vec![img("Image")],
        &[
            ("tonemap_type", e("RD_PHOTORECEPTOR")),
            ("key", f(0.18)),
            ("offset", f(1.0)),
            ("gamma", f(1.0)),
            ("intensity", f(0.0)),
            ("contrast", f(0.0)),
            ("adaptation", f(1.0)),
            ("correction", f(0.0)),
        ],
    );
    x.width = 200.0;
    x
}
fn z_combine() -> Node {
    node(
        "CompositorNodeZcombine",
        "Z Combine",
        140.0,
        vec![img("Image"), flr("Z", 1.0, 0.0, 10000.0), img("Image").ident("Image_001"), flr("Z", 1.0, 0.0, 10000.0).ident("Z_001")],
        vec![o_c("Image"), o_f("Z")],
        &[("use_alpha", b(false)), ("use_antialias_z", b(true))],
    )
}
fn convert_colorspace() -> Node {
    img_out("CompositorNodeConvertColorSpace", "Convert Colorspace", vec![img("Image")], &[("from_color_space", e("Linear Rec.709")), ("to_color_space", e("sRGB"))])
}
fn posterize() -> Node {
    img_out("CompositorNodePosterize", "Posterize", vec![img("Image"), flr("Steps", 8.0, 2.0, 1024.0)], &[])
}
fn alpha_convert() -> Node {
    img_out("CompositorNodePremulKey", "Alpha Convert", vec![img("Image")], &[("mapping", e("STRAIGHT_TO_PREMUL"))])
}
fn set_alpha() -> Node {
    img_out("CompositorNodeSetAlpha", "Set Alpha", vec![img("Image"), flr("Alpha", 1.0, 0.0, 1.0)], &[("mode", e("APPLY"))])
}

// ---------------------------------------------------------------------------
// Filter

fn antialiasing() -> Node {
    img_out("CompositorNodeAntiAliasing", "Anti-Aliasing", vec![img("Image")], &[("threshold", f(1.0)), ("contrast_limit", f(0.2)), ("corner_rounding", f(0.25))])
}
fn bilateral_blur() -> Node {
    img_out(
        "CompositorNodeBilateralblur",
        "Bilateral Blur",
        vec![img("Image"), img("Determinator")],
        &[("iterations", i(1)), ("sigma_color", f(0.3)), ("sigma_space", f(5.0))],
    )
}
fn blur() -> Node {
    let mut x = img_out(
        "CompositorNodeBlur",
        "Blur",
        vec![img("Image"), flr("Size", 1.0, 0.0, 1.0)],
        &[
            ("filter_type", e("GAUSS")),
            ("size_x", i(0)),
            ("size_y", i(0)),
            ("use_relative", b(false)),
            ("aspect_correction", e("NONE")),
            ("factor", f(0.0)),
            ("factor_x", f(0.0)),
            ("factor_y", f(0.0)),
            ("use_variable_size", b(false)),
            ("use_extended_bounds", b(false)),
            ("use_bokeh", b(false)),
            ("use_gamma_correction", b(false)),
        ],
    );
    x.width = 160.0;
    x
}
fn bokeh_blur() -> Node {
    img_out(
        "CompositorNodeBokehBlur",
        "Bokeh Blur",
        vec![col("Image", [0.8, 0.8, 0.8, 1.0]), img("Bokeh"), flr("Size", 1.0, 0.0, 10.0), flr("Bounding box", 1.0, 0.0, 1.0)],
        &[("use_variable_size", b(false)), ("use_extended_bounds", b(false)), ("f_stop", f(4.0)), ("blur_max", f(16.0))],
    )
}
fn defocus() -> Node {
    img_out(
        "CompositorNodeDefocus",
        "Defocus",
        vec![img("Image"), flr("Z", 1.0, 0.0, 1.0)],
        &[
            ("scene", Prop::Id(None)),
            ("bokeh", e("CIRCLE")),
            ("angle", f(0.0)),
            ("use_gamma_correction", b(false)),
            ("f_stop", f(128.0)),
            ("blur_max", f(16.0)),
            ("threshold", f(1.0)),
            ("use_preview", b(true)),
            ("use_zbuffer", b(true)),
            ("z_scale", f(1.0)),
        ],
    )
}
fn denoise() -> Node {
    img_out(
        "CompositorNodeDenoise",
        "Denoise",
        vec![img("Image"), vec("Normal", [0.0; 3]).range(-1.0, 1.0).hide_value(), img("Albedo").hide_value()],
        &[("use_hdr", b(true)), ("prefilter", e("ACCURATE"))],
    )
}
fn despeckle() -> Node {
    img_out("CompositorNodeDespeckle", "Despeckle", vec![fac("Fac", 1.0), img("Image")], &[("threshold", f(0.5)), ("threshold_neighbor", f(0.5))])
}
fn dilate_erode() -> Node {
    node(
        "CompositorNodeDilateErode",
        "Dilate/Erode",
        140.0,
        vec![flr("Mask", 0.0, 0.0, 1.0)],
        vec![o_f("Mask")],
        &[("mode", e("STEP")), ("distance", i(0)), ("edge", f(0.0)), ("falloff", e("SMOOTH"))],
    )
}
fn directional_blur() -> Node {
    img_out(
        "CompositorNodeDBlur",
        "Directional Blur",
        vec![img("Image")],
        &[("iterations", i(1)), ("center_x", f(0.5)), ("center_y", f(0.5)), ("distance", f(0.0)), ("angle", f(0.0)), ("spin", f(0.0)), ("zoom", f(0.0))],
    )
}
fn filter() -> Node {
    img_out("CompositorNodeFilter", "Filter", vec![fac("Fac", 1.0), img("Image")], &[("filter_type", e("SOFTEN"))])
}
fn glare() -> Node {
    let mut x = img_out(
        "CompositorNodeGlare",
        "Glare",
        vec![img("Image")],
        &[
            ("glare_type", e("STREAKS")),
            ("quality", e("MEDIUM")),
            ("iterations", i(3)),
            ("color_modulation", f(0.25)),
            ("mix", f(0.0)),
            ("threshold", f(1.0)),
            ("streaks", i(4)),
            ("angle_offset", f(0.0)),
            ("fade", f(0.9)),
            ("use_rotate_45", b(true)),
            ("size", i(8)),
        ],
    );
    x.width = 160.0;
    x
}
fn inpaint() -> Node {
    img_out("CompositorNodeInpaint", "Inpaint", vec![img("Image")], &[("distance", i(0))])
}
fn pixelate() -> Node {
    node("CompositorNodePixelate", "Pixelate", 140.0, vec![col("Color", [0.0, 0.0, 0.0, 1.0])], vec![o_c("Color")], &[("pixel_size", i(1))])
}
fn sun_beams() -> Node {
    img_out("CompositorNodeSunBeams", "Sun Beams", vec![img("Image")], &[("source", Prop::Vector([0.5, 0.5, 0.0])), ("ray_length", f(0.0))])
}
fn vector_blur() -> Node {
    img_out(
        "CompositorNodeVecBlur",
        "Vector Blur",
        vec![img("Image"), flr("Z", 0.0, 0.0, 1.0), vec("Speed", [0.0; 3]).range(0.0, 1.0)],
        &[("samples", i(32)), ("speed_min", i(0)), ("speed_max", i(0)), ("factor", f(0.25)), ("use_curved", b(false))],
    )
}
fn kuwahara() -> Node {
    img_out(
        "CompositorNodeKuwahara",
        "Kuwahara",
        vec![img("Image"), fl("Size", 6.0)],
        &[("variation", e("CLASSIC")), ("use_high_precision", b(false)), ("uniformity", i(4)), ("sharpness", f(0.5)), ("eccentricity", f(1.0))],
    )
}

// ---------------------------------------------------------------------------
// Keying

fn keyed(idname: &str, label: &str, ins: Vec<Socket>, props: &[(&str, Prop)]) -> Node {
    node(idname, label, 200.0, ins, vec![o_c("Image"), o_f("Matte")], props)
}
fn channel_key() -> Node {
    keyed(
        "CompositorNodeChannelMatte",
        "Channel Key",
        vec![img("Image")],
        &[("color_space", e("RGB")), ("matte_channel", e("G")), ("limit_method", e("MAX")), ("limit_channel", e("R")), ("limit_max", f(1.0)), ("limit_min", f(0.0))],
    )
}
fn chroma_key() -> Node {
    keyed(
        "CompositorNodeChromaMatte",
        "Chroma Key",
        vec![img("Image"), img("Key Color")],
        &[("tolerance", f(30f32.to_radians())), ("threshold", f(10f32.to_radians())), ("gain", f(1.0)), ("lift", f(0.0)), ("shadow_adjust", f(0.0))],
    )
}
fn color_key() -> Node {
    keyed("CompositorNodeColorMatte", "Color Key", vec![img("Image"), img("Key Color")], &[("color_hue", f(0.01)), ("color_saturation", f(0.1)), ("color_value", f(0.1))])
}
fn color_spill() -> Node {
    img_out(
        "CompositorNodeColorSpill",
        "Color Spill",
        vec![img("Image"), fac("Fac", 1.0)],
        &[
            ("channel", e("G")),
            ("limit_method", e("SIMPLE")),
            ("limit_channel", e("R")),
            ("ratio", f(1.0)),
            ("use_unspill", b(false)),
            ("unspill_red", f(0.0)),
            ("unspill_green", f(0.0)),
            ("unspill_blue", f(0.0)),
        ],
    )
}
fn difference_key() -> Node {
    keyed("CompositorNodeDiffMatte", "Difference Key", vec![img("Image 1"), img("Image 2")], &[("tolerance", f(0.1)), ("falloff", f(0.1))])
}
fn distance_key() -> Node {
    keyed("CompositorNodeDistanceMatte", "Distance Key", vec![img("Image"), img("Key Color")], &[("channel", e("RGB")), ("tolerance", f(0.1)), ("falloff", f(0.1))])
}
fn keying() -> Node {
    node(
        "CompositorNodeKeying",
        "Keying",
        200.0,
        vec![col("Image", [0.8, 0.8, 0.8, 1.0]), img("Key Color"), fl("Garbage Matte", 0.0).hide_value(), fl("Core Matte", 0.0).hide_value()],
        vec![o_c("Image"), o_f("Matte"), o_f("Edges")],
        &[
            ("blur_pre", i(0)),
            ("screen_balance", f(0.5)),
            ("despill_factor", f(1.0)),
            ("despill_balance", f(0.5)),
            ("edge_kernel_radius", i(3)),
            ("edge_kernel_tolerance", f(0.1)),
            ("clip_black", f(0.0)),
            ("clip_white", f(1.0)),
            ("dilate_distance", i(0)),
            ("feather_falloff", e("SMOOTH")),
            ("feather_distance", i(0)),
            ("blur_post", i(0)),
        ],
    )
}
fn keying_screen() -> Node {
    node("CompositorNodeKeyingScreen", "Keying Screen", 140.0, vec![], vec![o_c("Screen")], &[("clip", Prop::Id(None)), ("smoothness", f(0.0))])
}
fn luminance_key() -> Node {
    keyed("CompositorNodeLumaMatte", "Luminance Key", vec![img("Image")], &[("limit_max", f(1.0)), ("limit_min", f(0.0))])
}
fn cryptomatte() -> Node {
    node(
        "CompositorNodeCryptomatteV2",
        "Cryptomatte",
        240.0,
        vec![col("Image", [0.0, 0.0, 0.0, 1.0])],
        vec![o_c("Image"), o_f("Matte"), o_c("Pick")],
        &[("source", e("RENDER")), ("layer_name", Prop::String(String::new())), ("matte_id", Prop::String(String::new()))],
    )
}

// ---------------------------------------------------------------------------
// Mask

fn shape_mask(idname: &str, label: &str) -> Node {
    node(
        idname,
        label,
        140.0,
        vec![flr("Mask", 0.0, 0.0, 1.0), flr("Value", 1.0, 0.0, 1.0)],
        vec![o_f("Mask")],
        &[("mask_type", e("ADD")), ("x", f(0.5)), ("y", f(0.5)), ("mask_width", f(0.2)), ("mask_height", f(0.1)), ("rotation", f(0.0))],
    )
}
fn box_mask() -> Node {
    shape_mask("CompositorNodeBoxMask", "Box Mask")
}
fn ellipse_mask() -> Node {
    shape_mask("CompositorNodeEllipseMask", "Ellipse Mask")
}
fn double_edge_mask() -> Node {
    node(
        "CompositorNodeDoubleEdgeMask",
        "Double Edge Mask",
        200.0,
        vec![flr("Inner Mask", 0.8, 0.0, 1.0), flr("Outer Mask", 0.8, 0.0, 1.0)],
        vec![o_f("Mask")],
        &[("inner_mode", e("ALL")), ("edge_mode", e("BLEED_OUT"))],
    )
}
fn id_mask() -> Node {
    node("CompositorNodeIDMask", "ID Mask", 140.0, vec![flr("ID value", 1.0, 0.0, 1.0)], vec![o_f("Alpha")], &[("index", i(0)), ("use_antialiasing", b(false))])
}

// ---------------------------------------------------------------------------
// Transform

fn rotate() -> Node {
    img_out("CompositorNodeRotate", "Rotate", vec![img("Image"), fl("Degr", 0.0).subtype(S::Angle)], &[("filter_type", e("BILINEAR"))])
}
fn scale() -> Node {
    img_out(
        "CompositorNodeScale",
        "Scale",
        vec![img("Image"), flr("X", 1.0, 0.0001, 100000.0), flr("Y", 1.0, 0.0001, 100000.0)],
        &[("space", e("RELATIVE")), ("frame_method", e("STRETCH")), ("offset_x", f(0.0)), ("offset_y", f(0.0))],
    )
}
fn transform() -> Node {
    img_out(
        "CompositorNodeTransform",
        "Transform",
        vec![col("Image", [0.8, 0.8, 0.8, 1.0]), fl("X", 0.0), fl("Y", 0.0), fl("Angle", 0.0).subtype(S::Angle), flr("Scale", 1.0, 0.0001, 100000.0)],
        &[("filter_type", e("NEAREST"))],
    )
}
fn translate() -> Node {
    img_out("CompositorNodeTranslate", "Translate", vec![img("Image"), fl("X", 0.0), fl("Y", 0.0)], &[("interpolation", e("Nearest")), ("use_relative", b(false)), ("wrap_axis", e("NONE"))])
}
fn flip() -> Node {
    img_out("CompositorNodeFlip", "Flip", vec![img("Image")], &[("axis", e("X"))])
}
fn crop() -> Node {
    img_out(
        "CompositorNodeCrop",
        "Crop",
        vec![img("Image")],
        &[
            ("use_crop_size", b(false)),
            ("relative", b(false)),
            ("min_x", i(0)),
            ("max_x", i(0)),
            ("min_y", i(0)),
            ("max_y", i(0)),
            ("rel_min_x", f(0.0)),
            ("rel_max_x", f(0.0)),
            ("rel_min_y", f(0.0)),
            ("rel_max_y", f(0.0)),
        ],
    )
}
fn displace() -> Node {
    img_out(
        "CompositorNodeDisplace",
        "Displace",
        vec![img("Image"), vec("Vector", [1.0; 3]).subtype(S::Translation), flr("X Scale", 0.0, -1000.0, 1000.0), flr("Y Scale", 0.0, -1000.0, 1000.0)],
        &[],
    )
}
fn map_uv() -> Node {
    img_out("CompositorNodeMapUV", "Map UV", vec![img("Image"), vec("UV", [1.0, 0.0, 0.0])], &[("alpha", i(0)), ("filter_type", e("ANISOTROPIC"))])
}
fn lens_distortion() -> Node {
    img_out(
        "CompositorNodeLensdist",
        "Lens Distortion",
        vec![img("Image"), flr("Distortion", 0.0, -0.999, 1.0), flr("Dispersion", 0.0, 0.0, 1.0)],
        &[("use_projector", b(false)), ("use_jitter", b(false)), ("use_fit", b(false))],
    )
}
fn corner_pin() -> Node {
    node(
        "CompositorNodeCornerPin",
        "Corner Pin",
        140.0,
        vec![img("Image"), vec("Upper Left", [0.0, 1.0, 0.0]), vec("Upper Right", [1.0, 1.0, 0.0]), vec("Lower Left", [0.0; 3]), vec("Lower Right", [1.0, 0.0, 0.0])],
        vec![o_c("Image"), o_f("Plane")],
        &[],
    )
}
fn stabilize() -> Node {
    img_out("CompositorNodeStabilize", "Stabilize 2D", vec![col("Image", [0.8, 0.8, 0.8, 1.0])], &[("clip", Prop::Id(None)), ("filter_type", e("BILINEAR")), ("invert", b(false))])
}
fn plane_track_deform() -> Node {
    node("CompositorNodePlaneTrackDeform", "Plane Track Deform", 140.0, vec![img("Image")], vec![o_c("Image"), o_f("Plane")], &[("clip", Prop::Id(None))])
}
fn movie_distortion() -> Node {
    img_out("CompositorNodeMovieDistortion", "Movie Distortion", vec![col("Image", [0.8, 0.8, 0.8, 1.0])], &[("clip", Prop::Id(None)), ("distortion_type", e("UNDISTORT"))])
}

// ---------------------------------------------------------------------------
// Utilities

fn map_range() -> Node {
    node(
        "CompositorNodeMapRange",
        "Map Range",
        140.0,
        vec![flr("Value", 1.0, 0.0, 1.0), fl("From Min", 0.0), fl("From Max", 1.0), fl("To Min", 0.0), fl("To Max", 1.0)],
        vec![o_f("Value")],
        &[("use_clamp", b(false))],
    )
}
fn map_value() -> Node {
    node(
        "CompositorNodeMapValue",
        "Map Value",
        140.0,
        vec![flr("Value", 1.0, 0.0, 1.0)],
        vec![o_f("Value")],
        &[("offset", f(0.0)), ("size", f(1.0)), ("use_min", b(false)), ("use_max", b(false)), ("min", f(0.0)), ("max", f(1.0))],
    )
}
fn math() -> Node {
    let v = |id: &str| fl("Value", 0.5).range(-10000.0, 10000.0).ident(id);
    node("CompositorNodeMath", "Math", 140.0, vec![v("Value"), v("Value_001"), v("Value_002")], vec![o_f("Value")], &[("operation", e("ADD")), ("use_clamp", b(false))])
}
fn normalize() -> Node {
    node("CompositorNodeNormalize", "Normalize", 140.0, vec![flr("Value", 1.0, 0.0, 1.0)], vec![o_f("Value")], &[])
}
fn switch() -> Node {
    img_out("CompositorNodeSwitch", "Switch", vec![col("Off", [0.8, 0.8, 0.8, 1.0]), col("On", [0.8, 0.8, 0.8, 1.0])], &[("check", b(false))])
}
fn levels() -> Node {
    node("CompositorNodeLevels", "Levels", 140.0, vec![col("Image", [0.0, 0.0, 0.0, 1.0])], vec![o_f("Mean"), o_f("Std Dev")], &[("channel", e("COMBINED_RGB"))])
}
fn split() -> Node {
    img_out("CompositorNodeSplit", "Split", vec![col("Image", [0.0; 4]), col("Image", [0.0; 4]).ident("Image_001")], &[("axis", e("X")), ("factor", i(50))])
}
fn separate_color() -> Node {
    node(
        "CompositorNodeSeparateColor",
        "Separate Color",
        140.0,
        vec![img("Image")],
        vec![o_f("Red"), o_f("Green"), o_f("Blue"), o_f("Alpha")],
        &[("mode", e("RGB")), ("ycc_mode", e("ITUBT709"))],
    )
}
fn combine_color() -> Node {
    node(
        "CompositorNodeCombineColor",
        "Combine Color",
        140.0,
        vec![fac("Red", 0.0), fac("Green", 0.0), fac("Blue", 0.0), fac("Alpha", 1.0)],
        vec![o_c("Image")],
        &[("mode", e("RGB")), ("ycc_mode", e("ITUBT709"))],
    )
}
fn rgb_to_bw() -> Node {
    node("CompositorNodeRGBToBW", "RGB to BW", 140.0, vec![col("Image", [0.8, 0.8, 0.8, 1.0])], vec![o_f("Val")], &[])
}
fn color_ramp() -> Node {
    node("CompositorNodeValToRGB", "Color Ramp", 240.0, vec![fac("Fac", 0.5)], vec![o_c("Image"), o_f("Alpha")], &[("color_ramp", Prop::Ramp(ColorRamp::default()))])
}

// ---------------------------------------------------------------------------
// Vector

fn normal() -> Node {
    let mut x = node(
        "CompositorNodeNormal",
        "Normal",
        140.0,
        vec![vec("Normal", [0.0, 0.0, 1.0]).range(-1.0, 1.0).subtype(S::Direction)],
        vec![o_v("Normal"), o_f("Dot")],
        &[],
    );
    x.outputs[0].value = V::Vector([0.0, 0.0, 1.0]);
    x
}
fn vector_curves() -> Node {
    let m = CurveMapping { curves: vec![vec![[-1.0, -1.0], [1.0, 1.0]]; 3] };
    let mut x = node("CompositorNodeCurveVec", "Vector Curves", 200.0, vec![vec("Vector", [0.0; 3]).range(-1.0, 1.0)], vec![o_v("Vector")], &[("mapping", Prop::Curve(m))]);
    x.width = 200.0;
    x
}
fn separate_xyz() -> Node {
    node("CompositorNodeSeparateXYZ", "Separate XYZ", 140.0, vec![vec("Vector", [0.0; 3])], vec![o_f("X"), o_f("Y"), o_f("Z")], &[])
}
fn combine_xyz() -> Node {
    node("CompositorNodeCombineXYZ", "Combine XYZ", 140.0, vec![fl("X", 0.0), fl("Y", 0.0), fl("Z", 0.0)], vec![o_v("Vector")], &[])
}

fn group() -> Node {
    node("CompositorNodeGroup", "Group", 140.0, vec![], vec![], &[("node_tree", Prop::Id(None))])
}

macro_rules! ty {
    ($id:literal, $label:literal, $cat:ident, $f:expr) => {
        NodeType { idname: $id, label: $label, tree: TreeKind::Compositor, category: Category::$cat, build: $f }
    };
}

/// Compositor node types in Add-menu order (Blender 4.2's
/// `node_add_menu_compositor.py`).
pub static COMP_TYPES: &[NodeType] = &[
    ty!("CompositorNodeBokehImage", "Bokeh Image", Input, bokeh_image),
    ty!("CompositorNodeImage", "Image", Input, image),
    ty!("CompositorNodeMask", "Mask", Input, mask),
    ty!("CompositorNodeMovieClip", "Movie Clip", Input, movie_clip),
    ty!("CompositorNodeTexture", "Texture", Input, texture),
    ty!("CompositorNodeRGB", "RGB", Input, rgb),
    ty!("CompositorNodeValue", "Value", Input, value),
    ty!("CompositorNodeRLayers", "Render Layers", Input, render_layers),
    ty!("CompositorNodeSceneTime", "Scene Time", Input, scene_time),
    ty!("CompositorNodeTime", "Time Curve", Input, time_curve),
    ty!("CompositorNodeComposite", "Composite", Output, composite),
    ty!("CompositorNodeViewer", "Viewer", Output, viewer),
    ty!("CompositorNodeOutputFile", "File Output", Output, file_output),
    ty!("CompositorNodePremulKey", "Alpha Convert", Color, alpha_convert),
    ty!("CompositorNodeValToRGB", "Color Ramp", Color, color_ramp),
    ty!("CompositorNodeConvertColorSpace", "Convert Colorspace", Color, convert_colorspace),
    ty!("CompositorNodeSetAlpha", "Set Alpha", Color, set_alpha),
    ty!("CompositorNodeInvert", "Invert Color", Color, invert),
    ty!("CompositorNodeRGBToBW", "RGB to BW", Color, rgb_to_bw),
    ty!("CompositorNodeBrightContrast", "Brightness/Contrast", Color, bright_contrast),
    ty!("CompositorNodeColorBalance", "Color Balance", Color, color_balance),
    ty!("CompositorNodeColorCorrection", "Color Correction", Color, color_correction),
    ty!("CompositorNodeExposure", "Exposure", Color, exposure),
    ty!("CompositorNodeGamma", "Gamma", Color, gamma),
    ty!("CompositorNodeHueCorrect", "Hue Correct", Color, hue_correct),
    ty!("CompositorNodeHueSat", "Hue/Saturation/Value", Color, hue_sat),
    ty!("CompositorNodeCurveRGB", "RGB Curves", Color, rgb_curves),
    ty!("CompositorNodeTonemap", "Tonemap", Color, tonemap),
    ty!("CompositorNodeAlphaOver", "Alpha Over", Color, alpha_over),
    ty!("CompositorNodeMixRGB", "Mix", Color, mix_rgb),
    ty!("CompositorNodeZcombine", "Z Combine", Color, z_combine),
    ty!("CompositorNodePosterize", "Posterize", Color, posterize),
    ty!("CompositorNodeAntiAliasing", "Anti-Aliasing", Filter, antialiasing),
    ty!("CompositorNodeDenoise", "Denoise", Filter, denoise),
    ty!("CompositorNodeDespeckle", "Despeckle", Filter, despeckle),
    ty!("CompositorNodeDilateErode", "Dilate/Erode", Filter, dilate_erode),
    ty!("CompositorNodeInpaint", "Inpaint", Filter, inpaint),
    ty!("CompositorNodeFilter", "Filter", Filter, filter),
    ty!("CompositorNodeGlare", "Glare", Filter, glare),
    ty!("CompositorNodeKuwahara", "Kuwahara", Filter, kuwahara),
    ty!("CompositorNodePixelate", "Pixelate", Filter, pixelate),
    ty!("CompositorNodeSunBeams", "Sun Beams", Filter, sun_beams),
    ty!("CompositorNodeBilateralblur", "Bilateral Blur", Filter, bilateral_blur),
    ty!("CompositorNodeBlur", "Blur", Filter, blur),
    ty!("CompositorNodeBokehBlur", "Bokeh Blur", Filter, bokeh_blur),
    ty!("CompositorNodeDefocus", "Defocus", Filter, defocus),
    ty!("CompositorNodeDBlur", "Directional Blur", Filter, directional_blur),
    ty!("CompositorNodeVecBlur", "Vector Blur", Filter, vector_blur),
    ty!("CompositorNodeChannelMatte", "Channel Key", Keying, channel_key),
    ty!("CompositorNodeChromaMatte", "Chroma Key", Keying, chroma_key),
    ty!("CompositorNodeColorMatte", "Color Key", Keying, color_key),
    ty!("CompositorNodeColorSpill", "Color Spill", Keying, color_spill),
    ty!("CompositorNodeDiffMatte", "Difference Key", Keying, difference_key),
    ty!("CompositorNodeDistanceMatte", "Distance Key", Keying, distance_key),
    ty!("CompositorNodeKeying", "Keying", Keying, keying),
    ty!("CompositorNodeKeyingScreen", "Keying Screen", Keying, keying_screen),
    ty!("CompositorNodeLumaMatte", "Luminance Key", Keying, luminance_key),
    ty!("CompositorNodeCryptomatteV2", "Cryptomatte", Mask, cryptomatte),
    ty!("CompositorNodeBoxMask", "Box Mask", Mask, box_mask),
    ty!("CompositorNodeEllipseMask", "Ellipse Mask", Mask, ellipse_mask),
    ty!("CompositorNodeDoubleEdgeMask", "Double Edge Mask", Mask, double_edge_mask),
    ty!("CompositorNodeIDMask", "ID Mask", Mask, id_mask),
    ty!("CompositorNodePlaneTrackDeform", "Plane Track Deform", Transform, plane_track_deform),
    ty!("CompositorNodeStabilize", "Stabilize 2D", Transform, stabilize),
    ty!("CompositorNodeRotate", "Rotate", Transform, rotate),
    ty!("CompositorNodeScale", "Scale", Transform, scale),
    ty!("CompositorNodeTransform", "Transform", Transform, transform),
    ty!("CompositorNodeTranslate", "Translate", Transform, translate),
    ty!("CompositorNodeCornerPin", "Corner Pin", Transform, corner_pin),
    ty!("CompositorNodeCrop", "Crop", Transform, crop),
    ty!("CompositorNodeDisplace", "Displace", Transform, displace),
    ty!("CompositorNodeFlip", "Flip", Transform, flip),
    ty!("CompositorNodeMapUV", "Map UV", Transform, map_uv),
    ty!("CompositorNodeLensdist", "Lens Distortion", Transform, lens_distortion),
    ty!("CompositorNodeMovieDistortion", "Movie Distortion", Transform, movie_distortion),
    ty!("CompositorNodeMapRange", "Map Range", Utilities, map_range),
    ty!("CompositorNodeMapValue", "Map Value", Utilities, map_value),
    ty!("CompositorNodeMath", "Math", Utilities, math),
    ty!("CompositorNodeLevels", "Levels", Utilities, levels),
    ty!("CompositorNodeNormalize", "Normalize", Utilities, normalize),
    ty!("CompositorNodeSplit", "Split", Utilities, split),
    ty!("CompositorNodeSwitch", "Switch", Utilities, switch),
    ty!("CompositorNodeSeparateColor", "Separate Color", Utilities, separate_color),
    ty!("CompositorNodeCombineColor", "Combine Color", Utilities, combine_color),
    ty!("CompositorNodeCombineXYZ", "Combine XYZ", Vector, combine_xyz),
    ty!("CompositorNodeSeparateXYZ", "Separate XYZ", Vector, separate_xyz),
    ty!("CompositorNodeNormal", "Normal", Vector, normal),
    ty!("CompositorNodeCurveVec", "Vector Curves", Vector, vector_curves),
    ty!("CompositorNodeGroup", "Group", Group, group),
];

type Items = &'static [(&'static str, &'static str)];

const BLUR_FILTERS: Items = &[
    ("FLAT", "Flat"),
    ("TENT", "Tent"),
    ("QUAD", "Quadratic"),
    ("CUBIC", "Cubic"),
    ("GAUSS", "Gaussian"),
    ("FAST_GAUSS", "Fast Gaussian"),
    ("CATROM", "Catrom"),
    ("MITCH", "Mitch"),
];
const FILTERS: Items = &[
    ("SOFTEN", "Soften"),
    ("SHARPEN", "Box Sharpen"),
    ("SHARPEN_DIAMOND", "Diamond Sharpen"),
    ("LAPLACE", "Laplace"),
    ("SOBEL", "Sobel"),
    ("PREWITT", "Prewitt"),
    ("KIRSCH", "Kirsch"),
    ("SHADOW", "Shadow"),
];
const GLARE_TYPES: Items = &[("BLOOM", "Bloom"), ("GHOSTS", "Ghosts"), ("STREAKS", "Streaks"), ("FOG_GLOW", "Fog Glow"), ("SIMPLE_STAR", "Simple Star")];
const QUALITY: Items = &[("HIGH", "High"), ("MEDIUM", "Medium"), ("LOW", "Low")];
const TONEMAP: Items = &[("RD_PHOTORECEPTOR", "R/D Photoreceptor"), ("RH_SIMPLE", "Rh Simple")];
const BALANCE: Items = &[("LIFT_GAMMA_GAIN", "Lift/Gamma/Gain"), ("OFFSET_POWER_SLOPE", "Offset/Power/Slope (ASC-CDL)")];
const PREMUL: Items = &[("STRAIGHT_TO_PREMUL", "To Premultiplied"), ("PREMUL_TO_STRAIGHT", "To Straight")];
const SET_ALPHA: Items = &[("APPLY", "Apply Mask"), ("REPLACE_ALPHA", "Replace Alpha")];
const COLORSPACES: Items = &[("Linear Rec.709", "Linear Rec.709"), ("sRGB", "sRGB"), ("Non-Color", "Non-Color")];
const COMB_MODES: Items = &[("RGB", "RGB"), ("HSV", "HSV"), ("HSL", "HSL"), ("YCC", "YCbCr"), ("YUV", "YUV")];
const YCC_MODES: Items = &[("ITUBT601", "ITU 601"), ("ITUBT709", "ITU 709"), ("JFIF", "JPEG")];
const ASPECT: Items = &[("NONE", "None"), ("Y", "Y"), ("X", "X")];
const DILATE: Items = &[("STEP", "Step"), ("THRESHOLD", "Threshold"), ("DISTANCE", "Distance"), ("FEATHER", "Feather")];
const FALLOFF: Items = &[("SMOOTH", "Smooth"), ("SPHERE", "Sphere"), ("ROOT", "Root"), ("INVERSE_SQUARE", "Inverse Square"), ("SHARP", "Sharp"), ("LINEAR", "Linear")];
const SCALE_SPACE: Items = &[("RELATIVE", "Relative"), ("ABSOLUTE", "Absolute"), ("SCENE_SIZE", "Scene Size"), ("RENDER_SIZE", "Render Size")];
const FRAME_METHOD: Items = &[("STRETCH", "Stretch"), ("FIT", "Fit"), ("CROP", "Crop")];
const SAMPLERS: Items = &[("NEAREST", "Nearest"), ("BILINEAR", "Bilinear"), ("BICUBIC", "Bicubic")];
const TRANSLATE_INTERP: Items = &[("Nearest", "Nearest"), ("Bilinear", "Bilinear"), ("Bicubic", "Bicubic")];
const WRAP: Items = &[("NONE", "None"), ("XAXIS", "X Axis"), ("YAXIS", "Y Axis"), ("BOTH", "Both Axes")];
const FLIP: Items = &[("X", "Flip X"), ("Y", "Flip Y"), ("XY", "Flip X & Y")];
const AXIS_XY: Items = &[("X", "X"), ("Y", "Y")];
const RGB_CHANNELS: Items = &[("R", "R"), ("G", "G"), ("B", "B")];
const MATTE_SPACES: Items = &[("RGB", "RGB"), ("HSV", "HSV"), ("YUV", "YUV"), ("YCC", "YCbCr")];
const LIMIT: Items = &[("SINGLE", "Single"), ("MAX", "Max")];
const SPILL_LIMIT: Items = &[("SIMPLE", "Simple"), ("AVERAGE", "Average")];
const DIST_SPACES: Items = &[("RGB", "RGB"), ("YCC", "YCC")];
const MASK_TYPES: Items = &[("ADD", "Add"), ("SUBTRACT", "Subtract"), ("MULTIPLY", "Multiply"), ("NOT", "Not")];
const INNER: Items = &[("ALL", "All"), ("ADJACENT_ONLY", "Adjacent Only")];
const EDGE: Items = &[("BLEED_OUT", "Bleed Out"), ("KEEP_IN", "Keep In")];
const BOKEH: Items = &[
    ("OCTAGON", "Octagonal"),
    ("HEPTAGON", "Heptagonal"),
    ("HEXAGON", "Hexagonal"),
    ("PENTAGON", "Pentagonal"),
    ("SQUARE", "Square"),
    ("TRIANGLE", "Triangle"),
    ("CIRCLE", "Circular"),
];
const LEVELS: Items = &[("COMBINED_RGB", "Combined"), ("RED", "Red"), ("GREEN", "Green"), ("BLUE", "Blue"), ("LUMINANCE", "Luminance")];
const KUWAHARA: Items = &[("CLASSIC", "Classic"), ("ANISOTROPIC", "Anisotropic")];
const PREFILTER: Items = &[("NONE", "None"), ("FAST", "Fast"), ("ACCURATE", "Accurate")];
const UV_FILTER: Items = &[("NEAREST", "Nearest"), ("ANISOTROPIC", "Anisotropic")];
const FILE_FORMATS: Items = &[("PNG", "PNG")];

/// Dropdown choices of compositor node properties (empty when unknown).
pub fn comp_items(idname: &str, prop: &str) -> Items {
    match (idname, prop) {
        ("CompositorNodeBlur", "filter_type") => BLUR_FILTERS,
        ("CompositorNodeBlur", "aspect_correction") => ASPECT,
        ("CompositorNodeFilter", "filter_type") => FILTERS,
        ("CompositorNodeGlare", "glare_type") => GLARE_TYPES,
        ("CompositorNodeGlare", "quality") => QUALITY,
        ("CompositorNodeTonemap", "tonemap_type") => TONEMAP,
        ("CompositorNodeColorBalance", "correction_method") => BALANCE,
        ("CompositorNodePremulKey", "mapping") => PREMUL,
        ("CompositorNodeSetAlpha", "mode") => SET_ALPHA,
        ("CompositorNodeConvertColorSpace", _) => COLORSPACES,
        ("CompositorNodeSeparateColor" | "CompositorNodeCombineColor", "mode") => COMB_MODES,
        ("CompositorNodeSeparateColor" | "CompositorNodeCombineColor", "ycc_mode") => YCC_MODES,
        ("CompositorNodeDilateErode", "mode") => DILATE,
        (_, "falloff" | "feather_falloff") => FALLOFF,
        ("CompositorNodeScale", "space") => SCALE_SPACE,
        ("CompositorNodeScale", "frame_method") => FRAME_METHOD,
        ("CompositorNodeRotate" | "CompositorNodeTransform" | "CompositorNodeStabilize", "filter_type") => SAMPLERS,
        ("CompositorNodeTranslate", "interpolation") => TRANSLATE_INTERP,
        ("CompositorNodeTranslate", "wrap_axis") => WRAP,
        ("CompositorNodeFlip", "axis") => FLIP,
        ("CompositorNodeSplit", "axis") => AXIS_XY,
        ("CompositorNodeChannelMatte", "color_space") => MATTE_SPACES,
        ("CompositorNodeChannelMatte" | "CompositorNodeColorSpill", "matte_channel" | "limit_channel" | "channel") => RGB_CHANNELS,
        ("CompositorNodeChannelMatte", "limit_method") => LIMIT,
        ("CompositorNodeColorSpill", "limit_method") => SPILL_LIMIT,
        ("CompositorNodeDistanceMatte", "channel") => DIST_SPACES,
        ("CompositorNodeBoxMask" | "CompositorNodeEllipseMask", "mask_type") => MASK_TYPES,
        ("CompositorNodeDoubleEdgeMask", "inner_mode") => INNER,
        ("CompositorNodeDoubleEdgeMask", "edge_mode") => EDGE,
        ("CompositorNodeDefocus", "bokeh") => BOKEH,
        ("CompositorNodeLevels", "channel") => LEVELS,
        ("CompositorNodeKuwahara", "variation") => KUWAHARA,
        ("CompositorNodeDenoise", "prefilter") => PREFILTER,
        ("CompositorNodeMapUV", "filter_type") => UV_FILTER,
        ("CompositorNodeOutputFile", "file_format") => FILE_FORMATS,
        _ => &[],
    }
}

/// Socket availability and names after a property change, like the
/// compositor nodes' `update` callbacks.
pub fn comp_update(node: &mut Node) {
    let mode = node.enum_prop("mode").to_string();
    match node.idname.as_str() {
        "CompositorNodeMath" => {
            let k = super::registry::math_inputs(node.enum_prop("operation"));
            node.inputs[1].enabled = k >= 2;
            node.inputs[2].enabled = k >= 3;
        }
        "CompositorNodeSeparateColor" | "CompositorNodeCombineColor" => {
            let names: [&str; 3] = match mode.as_str() {
                "HSV" => ["Hue", "Saturation", "Value"],
                "HSL" => ["Hue", "Saturation", "Lightness"],
                "YCC" => ["Y", "Cb", "Cr"],
                "YUV" => ["Y", "U", "V"],
                _ => ["Red", "Green", "Blue"],
            };
            let socks = if node.idname == "CompositorNodeSeparateColor" { &mut node.outputs } else { &mut node.inputs };
            for (s, nm) in socks.iter_mut().zip(names) {
                s.name = nm.into();
            }
        }
        "CompositorNodeScale" => {
            let rel = matches!(node.enum_prop("space"), "RELATIVE" | "ABSOLUTE");
            node.inputs[1].enabled = rel;
            node.inputs[2].enabled = rel;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compositor_menu_matches_blender_categories() {
        let menu = super::super::categories(TreeKind::Compositor);
        let names: Vec<&str> = menu.iter().map(|c| c.0.label()).collect();
        for want in ["Input", "Output", "Color", "Filter", "Keying", "Mask", "Transform", "Utilities", "Vector", "Group", "Layout"] {
            assert!(names.contains(&want), "missing {want} in {names:?}");
        }
        let n = super::super::new_node("CompositorNodeAlphaOver").unwrap();
        assert_eq!(n.inputs.iter().map(|s| s.identifier.as_str()).collect::<Vec<_>>(), ["Fac", "Image", "Image_001"]);
        let mut sep = super::super::new_node("CompositorNodeSeparateColor").unwrap();
        sep.set_prop("mode", e("HSV"));
        super::super::update_sockets(&mut sep);
        assert_eq!(sep.outputs[0].name, "Hue");
        assert_eq!(sep.outputs[0].identifier, "Red");
    }
}
