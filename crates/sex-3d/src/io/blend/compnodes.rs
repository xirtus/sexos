// SPDX-License-Identifier: GPL-3.0-only
//! Compositor node settings from `.blend` files. Where each RNA property
//! lives follows Blender's `rna_nodetree.cc` (`def_cmp_*`) and the storage
//! structs of `DNA_node_types.h` (GPL-2+):
//!
//! - `custom1..4` on `bNode` (`custom1`/`custom2` shorts, `custom3`/`custom4`
//!   floats) hold the simple settings: Mix blend type and alpha/clamp bits,
//!   Math operation, Filter type, Flip/Split axis, Dilate/Erode mode and
//!   distance, Scale space and frame method, Despeckle thresholds, Bokeh
//!   Blur f-stop, Composite/Viewer `CMP_NODE_OUTPUT_IGNORE_ALPHA` and so on.
//! - Storage structs: `NodeBlurData`, `NodeDBlurData`,
//!   `NodeBilateralBlurData`, `NodeGlare`, `NodeTonemap`, `NodeColorBalance`,
//!   `NodeColorCorrection`, `NodeChroma` (all the keyers), `NodeColorspill`,
//!   `NodeKeyingData`, `NodeTwoXYs` (Crop), `NodeTwoFloats` (Alpha Over),
//!   `NodeBoxMask`/`NodeEllipseMask`, `NodeDefocus`, `NodeBokehImage`,
//!   `NodeLensDist`, `NodeTranslateData`, `NodeSunBeams`, `NodeDenoise`,
//!   `NodeKuwaharaData`, `NodeAntiAliasingData`, `NodeSetAlpha`,
//!   `NodeCMPCombSepColor`, `NodeConvertColorSpace`, `NodeDilateErode`,
//!   `TexMapping` (Map Value), `NodeImageMultiFile` with one
//!   `NodeImageMultiFileSocket {path}` per File Output input, `CurveMapping`
//!   and `ColorBand`.
use super::nodes::{color_ramp, curve_maps};
use super::{Importer, View};
use crate::nodes::{CurveMapping, Node, Prop};
use crate::render::shader_names::{blend_from_file, math_from_file};

fn e(s: &str) -> Prop {
    Prop::Enum(s.into())
}

fn pick(names: &[&str], i: f64) -> Prop {
    e(names.get(i.max(0.0) as usize).copied().unwrap_or(names[0]))
}

fn b(v: bool) -> Prop {
    Prop::Bool(v)
}

/// `R_FILTER_*` (box, tent, quad, cubic, catrom, gauss, mitch, fast gauss).
const BLUR_FILTERS: [&str; 8] = ["FLAT", "TENT", "QUAD", "CUBIC", "CATROM", "GAUSS", "MITCH", "FAST_GAUSS"];
/// `CMP_NODE_GLARE_*`.
const GLARE: [&str; 5] = ["SIMPLE_STAR", "FOG_GLOW", "STREAKS", "GHOSTS", "BLOOM"];
/// `ePropFalloff` as the curve-only falloff items.
fn falloff(i: f64) -> Prop {
    e(match i as i64 {
        1 => "SPHERE",
        2 => "ROOT",
        3 => "SHARP",
        4 => "LINEAR",
        7 => "INVERSE_SQUARE",
        _ => "SMOOTH",
    })
}
/// RGB channel stored 1-based (`prop_tri_channel_items`).
fn chan1(i: f64) -> Prop {
    pick(&["R", "R", "G", "B"], i)
}

impl<'f, 'a> Importer<'f, 'a> {
    /// Settings of a `CompositorNode*`; false when the node has none here.
    pub(super) fn comp_props(&mut self, bn: &View<'f, 'a>, n: &mut Node) -> bool {
        let f = self.f;
        let c1 = bn.num("custom1").unwrap_or(0.0);
        let c2 = bn.num("custom2").unwrap_or(0.0);
        let c3 = bn.f32("custom3").unwrap_or(0.0);
        let c4 = bn.f32("custom4").unwrap_or(0.0);
        let (i1, i2) = (c1 as i64, c2 as i64);
        let st = bn.ptr("storage");
        let storage = |name: &str| f.view_ptr(st, &[name]);
        let fl = |v: &View<'f, 'a>, k: &str| Prop::Float(v.f32(k).unwrap_or(0.0));
        let int = |v: &View<'f, 'a>, k: &str| Prop::Int(v.num(k).unwrap_or(0.0) as i32);
        let idname = n.idname.clone();
        match idname.as_str() {
            "CompositorNodeRLayers" => {}
            "CompositorNodeImage" => {
                let img = self.image(bn.ptr("id"));
                n.set_prop("image", Prop::Id(img));
                n.set_prop("use_straight_alpha_output", b(i1 & 1 != 0));
                if let Some(iu) = storage("ImageUser") {
                    n.set_prop("frame_duration", int(&iu, "frames"));
                    n.set_prop("frame_start", int(&iu, "sfra"));
                    n.set_prop("frame_offset", int(&iu, "offset"));
                }
            }
            "CompositorNodeMask" => {
                n.set_prop("use_feather", b(i1 & 1 == 0));
                n.set_prop("use_motion_blur", b(i1 & 8 != 0));
                n.set_prop("size_source", e(if i1 & 2 != 0 { "FIXED" } else if i1 & 4 != 0 { "FIXED_SCENE" } else { "SCENE" }));
                n.set_prop("motion_blur_samples", Prop::Int(i2 as i32));
                n.set_prop("motion_blur_shutter", Prop::Float(c3));
                if let Some(s) = storage("NodeMask") {
                    n.set_prop("size_x", int(&s, "size_x"));
                    n.set_prop("size_y", int(&s, "size_y"));
                }
            }
            "CompositorNodeBokehImage" => {
                if let Some(s) = storage("NodeBokehImage") {
                    n.set_prop("angle", fl(&s, "angle"));
                    n.set_prop("flaps", int(&s, "flaps"));
                    n.set_prop("rounding", fl(&s, "rounding"));
                    n.set_prop("catadioptric", fl(&s, "catadioptric"));
                    n.set_prop("shift", fl(&s, "lensshift"));
                }
            }
            "CompositorNodeTime" => {
                n.set_prop("frame_start", Prop::Int(i1 as i32));
                n.set_prop("frame_end", Prop::Int(i2 as i32));
                if let Some(mut c) = curve_maps(f, st) {
                    c.truncate(1);
                    n.set_prop("curve", Prop::Curve(CurveMapping { curves: c }));
                }
            }
            "CompositorNodeComposite" | "CompositorNodeViewer" => n.set_prop("use_alpha", b(i2 & 1 == 0)),
            "CompositorNodeOutputFile" => {
                if let Some(s) = storage("NodeImageMultiFile") {
                    n.set_prop("base_path", Prop::String(s.string("base_path").unwrap_or_default()));
                    if let Some(fm) = s.sub("format") {
                        // R_IMF_PLANES_*: BW 8, RGB 24, RGBA 32; R_IMF_CHAN_DEPTH_16 is 1 << 4.
                        let planes = match fm.num("planes").unwrap_or(32.0) as i64 {
                            8 => "BW",
                            24 => "RGB",
                            _ => "RGBA",
                        };
                        n.set_prop("color_mode", Prop::Enum(planes.into()));
                        let depth = if fm.num("depth").unwrap_or(0.0) as i64 == 1 << 4 { "16" } else { "8" };
                        n.set_prop("color_depth", Prop::Enum(depth.into()));
                    }
                }
                // One slot per input, from each socket's storage.
                let slots: Vec<String> = f
                    .list(bn.sub("inputs"), &["bNodeSocket"])
                    .iter()
                    .map(|s| f.view_ptr(s.ptr("storage"), &["NodeImageMultiFileSocket"]).and_then(|m| m.string("path")).or_else(|| s.string("name")).unwrap_or_default())
                    .collect();
                if !slots.is_empty() {
                    n.set_prop("file_slots", Prop::String(slots.join("\n")));
                }
            }
            // ------------------------------------------------------ Color
            "CompositorNodeAlphaOver" => {
                n.set_prop("use_premultiply", b(i1 & 1 != 0));
                if let Some(s) = storage("NodeTwoFloats") {
                    n.set_prop("premul", fl(&s, "x"));
                }
            }
            "CompositorNodeBrightContrast" => n.set_prop("use_premultiply", b(i1 & 1 != 0)),
            "CompositorNodeColorBalance" => {
                n.set_prop("correction_method", pick(&["LIFT_GAMMA_GAIN", "OFFSET_POWER_SLOPE"], c1));
                if let Some(s) = storage("NodeColorBalance") {
                    for k in ["lift", "gamma", "gain", "offset", "power", "slope"] {
                        if let Some(v) = s.vec::<3>(k) {
                            n.set_prop(k, Prop::Color([v[0], v[1], v[2], 1.0]));
                        }
                    }
                    n.set_prop("offset_basis", fl(&s, "offset_basis"));
                }
            }
            "CompositorNodeColorCorrection" => {
                n.set_prop("red", b(i1 & 1 != 0));
                n.set_prop("green", b(i1 & 2 != 0));
                n.set_prop("blue", b(i1 & 4 != 0));
                if let Some(s) = storage("NodeColorCorrection") {
                    n.set_prop("midtones_start", fl(&s, "startmidtones"));
                    n.set_prop("midtones_end", fl(&s, "endmidtones"));
                    for r in ["master", "highlights", "midtones", "shadows"] {
                        let Some(d) = s.sub(r) else { continue };
                        for k in ["saturation", "contrast", "gamma", "gain", "lift"] {
                            n.set_prop(&format!("{r}_{k}"), fl(&d, k));
                        }
                    }
                }
            }
            "CompositorNodeHueCorrect" => {
                if let Some(mut c) = curve_maps(f, st) {
                    c.truncate(3);
                    n.set_prop("mapping", Prop::Curve(CurveMapping { curves: c }));
                }
            }
            "CompositorNodeInvert" => {
                n.set_prop("invert_rgb", b(i1 & 1 != 0));
                n.set_prop("invert_alpha", b(i1 & 2 != 0));
            }
            "CompositorNodeMixRGB" => {
                n.set_prop("blend_type", e(blend_from_file(i1)));
                n.set_prop("use_alpha", b(i2 & 1 != 0));
                n.set_prop("use_clamp", b(i2 & 2 != 0));
            }
            "CompositorNodeCurveRGB" => {
                if let Some(mut c) = curve_maps(f, st) {
                    if c.len() == 4 {
                        // File order R, G, B, C; ours C, R, G, B.
                        c.rotate_right(1);
                    }
                    n.set_prop("mapping", Prop::Curve(CurveMapping { curves: c }));
                }
            }
            "CompositorNodeTonemap" => {
                if let Some(s) = storage("NodeTonemap") {
                    n.set_prop("tonemap_type", e(if s.num("type").unwrap_or(1.0) as i64 == 0 { "RH_SIMPLE" } else { "RD_PHOTORECEPTOR" }));
                    for (p, k) in [("key", "key"), ("offset", "offset"), ("gamma", "gamma"), ("intensity", "f"), ("contrast", "m"), ("adaptation", "a"), ("correction", "c")] {
                        n.set_prop(p, fl(&s, k));
                    }
                }
            }
            "CompositorNodeZcombine" => {
                n.set_prop("use_alpha", b(i1 != 0));
                n.set_prop("use_antialias_z", b(i2 == 0));
            }
            "CompositorNodeConvertColorSpace" => {
                if let Some(s) = storage("NodeConvertColorSpace") {
                    n.set_prop("from_color_space", e(&s.string("from_color_space").unwrap_or_default()));
                    n.set_prop("to_color_space", e(&s.string("to_color_space").unwrap_or_default()));
                }
            }
            "CompositorNodePremulKey" => n.set_prop("mapping", pick(&["STRAIGHT_TO_PREMUL", "PREMUL_TO_STRAIGHT"], c1)),
            "CompositorNodeSetAlpha" => {
                if let Some(s) = storage("NodeSetAlpha") {
                    n.set_prop("mode", pick(&["APPLY", "REPLACE_ALPHA"], s.num("mode").unwrap_or(0.0)));
                }
            }
            // ----------------------------------------------------- Filter
            "CompositorNodeAntiAliasing" => {
                if let Some(s) = storage("NodeAntiAliasingData") {
                    for k in ["threshold", "contrast_limit", "corner_rounding"] {
                        n.set_prop(k, fl(&s, k));
                    }
                }
            }
            "CompositorNodeBilateralblur" => {
                if let Some(s) = storage("NodeBilateralBlurData") {
                    n.set_prop("iterations", int(&s, "iter"));
                    n.set_prop("sigma_color", fl(&s, "sigma_color"));
                    n.set_prop("sigma_space", fl(&s, "sigma_space"));
                }
            }
            "CompositorNodeBlur" => {
                n.set_prop("use_variable_size", b(i1 & 1 != 0));
                n.set_prop("use_extended_bounds", b(i1 & 2 != 0));
                if let Some(s) = storage("NodeBlurData") {
                    n.set_prop("filter_type", pick(&BLUR_FILTERS, s.num("filtertype").unwrap_or(0.0)));
                    n.set_prop("size_x", int(&s, "sizex"));
                    n.set_prop("size_y", int(&s, "sizey"));
                    n.set_prop("use_relative", b(s.num("relative").unwrap_or(0.0) != 0.0));
                    n.set_prop("aspect_correction", pick(&["NONE", "Y", "X"], s.num("aspect").unwrap_or(0.0)));
                    n.set_prop("factor", fl(&s, "fac"));
                    n.set_prop("factor_x", fl(&s, "percentx"));
                    n.set_prop("factor_y", fl(&s, "percenty"));
                    n.set_prop("use_bokeh", b(s.num("bokeh").unwrap_or(0.0) != 0.0));
                    n.set_prop("use_gamma_correction", b(s.num("gamma").unwrap_or(0.0) != 0.0));
                }
            }
            "CompositorNodeBokehBlur" => {
                n.set_prop("use_variable_size", b(i1 & 1 != 0));
                n.set_prop("use_extended_bounds", b(i1 & 2 != 0));
                n.set_prop("f_stop", Prop::Float(c3));
                n.set_prop("blur_max", Prop::Float(c4));
            }
            "CompositorNodeDefocus" => {
                if let Some(s) = storage("NodeDefocus") {
                    n.set_prop(
                        "bokeh",
                        e(match s.num("bktype").unwrap_or(0.0) as i64 {
                            3 => "TRIANGLE",
                            4 => "SQUARE",
                            5 => "PENTAGON",
                            6 => "HEXAGON",
                            7 => "HEPTAGON",
                            8 => "OCTAGON",
                            _ => "CIRCLE",
                        }),
                    );
                    n.set_prop("angle", fl(&s, "rotation"));
                    n.set_prop("use_gamma_correction", b(s.num("gamco").unwrap_or(0.0) != 0.0));
                    n.set_prop("f_stop", fl(&s, "fstop"));
                    n.set_prop("blur_max", fl(&s, "maxblur"));
                    n.set_prop("threshold", fl(&s, "bthresh"));
                    n.set_prop("use_preview", b(s.num("preview").unwrap_or(0.0) != 0.0));
                    n.set_prop("use_zbuffer", b(s.num("no_zbuf").unwrap_or(0.0) == 0.0));
                    n.set_prop("z_scale", fl(&s, "scale"));
                }
            }
            "CompositorNodeDenoise" => {
                if let Some(s) = storage("NodeDenoise") {
                    n.set_prop("use_hdr", b(s.num("hdr").unwrap_or(1.0) != 0.0));
                    n.set_prop("prefilter", pick(&["FAST", "NONE", "ACCURATE"], s.num("prefilter").unwrap_or(2.0)));
                }
            }
            "CompositorNodeDespeckle" => {
                n.set_prop("threshold", Prop::Float(c3));
                n.set_prop("threshold_neighbor", Prop::Float(c4));
            }
            "CompositorNodeDilateErode" => {
                n.set_prop("mode", pick(&["STEP", "THRESHOLD", "DISTANCE", "FEATHER"], c1));
                n.set_prop("distance", Prop::Int(i2 as i32));
                n.set_prop("edge", Prop::Float(c3));
                if let Some(s) = storage("NodeDilateErode") {
                    n.set_prop("falloff", falloff(s.num("falloff").unwrap_or(0.0)));
                }
            }
            "CompositorNodeDBlur" => {
                if let Some(s) = storage("NodeDBlurData") {
                    n.set_prop("iterations", int(&s, "iter"));
                    for k in ["center_x", "center_y", "distance", "angle", "spin", "zoom"] {
                        n.set_prop(k, fl(&s, k));
                    }
                }
            }
            "CompositorNodeFilter" => n.set_prop("filter_type", pick(&["SOFTEN", "SHARPEN", "LAPLACE", "SOBEL", "PREWITT", "KIRSCH", "SHADOW", "SHARPEN_DIAMOND"], c1)),
            "CompositorNodeGlare" => {
                if let Some(s) = storage("NodeGlare") {
                    n.set_prop("glare_type", pick(&GLARE, s.num("type").unwrap_or(2.0)));
                    n.set_prop("quality", pick(&["HIGH", "MEDIUM", "LOW"], s.num("quality").unwrap_or(1.0)));
                    n.set_prop("iterations", int(&s, "iter"));
                    n.set_prop("color_modulation", fl(&s, "colmod"));
                    n.set_prop("mix", fl(&s, "mix"));
                    n.set_prop("threshold", fl(&s, "threshold"));
                    n.set_prop("streaks", int(&s, "streaks"));
                    n.set_prop("angle_offset", fl(&s, "angle_ofs"));
                    n.set_prop("fade", fl(&s, "fade"));
                    n.set_prop("use_rotate_45", b(s.num("star_45").unwrap_or(0.0) != 0.0));
                    n.set_prop("size", int(&s, "size"));
                }
            }
            "CompositorNodeInpaint" => n.set_prop("distance", Prop::Int(i2 as i32)),
            "CompositorNodePixelate" => n.set_prop("pixel_size", Prop::Int((i1 as i32).max(1))),
            "CompositorNodeSunBeams" => {
                if let Some(s) = storage("NodeSunBeams") {
                    let src = s.vec::<2>("source").unwrap_or([0.5, 0.5]);
                    n.set_prop("source", Prop::Vector([src[0], src[1], 0.0]));
                    n.set_prop("ray_length", fl(&s, "ray_length"));
                }
            }
            "CompositorNodeVecBlur" => {
                if let Some(s) = storage("NodeBlurData") {
                    n.set_prop("samples", int(&s, "samples"));
                    n.set_prop("speed_min", int(&s, "minspeed"));
                    n.set_prop("speed_max", int(&s, "maxspeed"));
                    n.set_prop("factor", fl(&s, "fac"));
                    n.set_prop("use_curved", b(s.num("curved").unwrap_or(0.0) != 0.0));
                }
            }
            "CompositorNodeKuwahara" => {
                if let Some(s) = storage("NodeKuwaharaData") {
                    n.set_prop("variation", pick(&["CLASSIC", "ANISOTROPIC"], s.num("variation").unwrap_or(0.0)));
                    n.set_prop("use_high_precision", b(s.num("high_precision").unwrap_or(0.0) != 0.0));
                    n.set_prop("uniformity", int(&s, "uniformity"));
                    n.set_prop("sharpness", fl(&s, "sharpness"));
                    n.set_prop("eccentricity", fl(&s, "eccentricity"));
                }
            }
            // ----------------------------------------------------- Keying
            "CompositorNodeChannelMatte" => {
                n.set_prop("color_space", pick(&["RGB", "RGB", "HSV", "YUV", "YCC"], c1));
                n.set_prop("matte_channel", chan1(c2));
                if let Some(s) = storage("NodeChroma") {
                    n.set_prop("limit_method", pick(&["SINGLE", "MAX"], s.num("algorithm").unwrap_or(1.0)));
                    n.set_prop("limit_channel", chan1(s.num("channel").unwrap_or(1.0)));
                    n.set_prop("limit_max", fl(&s, "t1"));
                    n.set_prop("limit_min", fl(&s, "t2"));
                }
            }
            "CompositorNodeChromaMatte" => {
                if let Some(s) = storage("NodeChroma") {
                    n.set_prop("tolerance", fl(&s, "t1"));
                    n.set_prop("threshold", fl(&s, "t2"));
                    n.set_prop("shadow_adjust", fl(&s, "t3"));
                    n.set_prop("lift", fl(&s, "fsize"));
                    n.set_prop("gain", fl(&s, "fstrength"));
                }
            }
            "CompositorNodeColorMatte" => {
                if let Some(s) = storage("NodeChroma") {
                    n.set_prop("color_hue", fl(&s, "t1"));
                    n.set_prop("color_saturation", fl(&s, "t2"));
                    n.set_prop("color_value", fl(&s, "t3"));
                }
            }
            "CompositorNodeColorSpill" => {
                n.set_prop("channel", chan1(c1));
                n.set_prop("limit_method", pick(&["SIMPLE", "AVERAGE"], c2));
                if let Some(s) = storage("NodeColorspill") {
                    n.set_prop("limit_channel", pick(&["R", "G", "B"], s.num("limchan").unwrap_or(0.0)));
                    n.set_prop("ratio", fl(&s, "limscale"));
                    n.set_prop("use_unspill", b(s.num("unspill").unwrap_or(0.0) != 0.0));
                    n.set_prop("unspill_red", fl(&s, "uspillr"));
                    n.set_prop("unspill_green", fl(&s, "uspillg"));
                    n.set_prop("unspill_blue", fl(&s, "uspillb"));
                }
            }
            "CompositorNodeDiffMatte" => {
                if let Some(s) = storage("NodeChroma") {
                    n.set_prop("tolerance", fl(&s, "t1"));
                    n.set_prop("falloff", fl(&s, "t2"));
                }
            }
            "CompositorNodeDistanceMatte" => {
                if let Some(s) = storage("NodeChroma") {
                    n.set_prop("channel", e(if s.num("channel").unwrap_or(1.0) as i64 == 2 { "YCC" } else { "RGB" }));
                    n.set_prop("tolerance", fl(&s, "t1"));
                    n.set_prop("falloff", fl(&s, "t2"));
                }
            }
            "CompositorNodeLumaMatte" => {
                if let Some(s) = storage("NodeChroma") {
                    n.set_prop("limit_max", fl(&s, "t1"));
                    n.set_prop("limit_min", fl(&s, "t2"));
                }
            }
            "CompositorNodeKeying" => {
                if let Some(s) = storage("NodeKeyingData") {
                    for k in ["screen_balance", "despill_factor", "despill_balance", "edge_kernel_tolerance", "clip_black", "clip_white"] {
                        n.set_prop(k, fl(&s, k));
                    }
                    for k in ["edge_kernel_radius", "dilate_distance", "feather_distance", "blur_pre", "blur_post"] {
                        n.set_prop(k, int(&s, k));
                    }
                    n.set_prop("feather_falloff", falloff(s.num("feather_falloff").unwrap_or(0.0)));
                }
            }
            // ------------------------------------------------------- Mask
            "CompositorNodeBoxMask" | "CompositorNodeEllipseMask" => {
                n.set_prop("mask_type", pick(&["ADD", "SUBTRACT", "MULTIPLY", "NOT"], c1));
                let sn = if idname == "CompositorNodeBoxMask" { "NodeBoxMask" } else { "NodeEllipseMask" };
                if let Some(s) = storage(sn) {
                    n.set_prop("x", fl(&s, "x"));
                    n.set_prop("y", fl(&s, "y"));
                    n.set_prop("mask_width", fl(&s, "width"));
                    n.set_prop("mask_height", fl(&s, "height"));
                    n.set_prop("rotation", fl(&s, "rotation"));
                }
            }
            "CompositorNodeDoubleEdgeMask" => {
                n.set_prop("inner_mode", pick(&["ALL", "ADJACENT_ONLY"], c1));
                n.set_prop("edge_mode", pick(&["BLEED_OUT", "KEEP_IN"], c2));
            }
            "CompositorNodeIDMask" => {
                n.set_prop("index", Prop::Int(i1 as i32));
                n.set_prop("use_antialiasing", b(i2 != 0));
            }
            // -------------------------------------------------- Transform
            "CompositorNodeRotate" | "CompositorNodeTransform" => n.set_prop("filter_type", pick(&["NEAREST", "BILINEAR", "BICUBIC"], c1)),
            "CompositorNodeStabilize" => {
                n.set_prop("filter_type", pick(&["NEAREST", "BILINEAR", "BICUBIC"], c1));
                n.set_prop("invert", b(i2 & 1 != 0));
            }
            "CompositorNodeMovieDistortion" => n.set_prop("distortion_type", pick(&["UNDISTORT", "DISTORT"], c1)),
            "CompositorNodeScale" => {
                n.set_prop("space", pick(&["RELATIVE", "ABSOLUTE", "SCENE_SIZE", "RENDER_SIZE"], c1));
                n.set_prop("frame_method", e(if i2 & 2 != 0 { "CROP" } else if i2 & 1 != 0 { "FIT" } else { "STRETCH" }));
                n.set_prop("offset_x", Prop::Float(c3));
                n.set_prop("offset_y", Prop::Float(c4));
            }
            "CompositorNodeTranslate" => {
                if let Some(s) = storage("NodeTranslateData") {
                    n.set_prop("interpolation", pick(&["Nearest", "Bilinear", "Bicubic"], s.num("interpolation").unwrap_or(0.0)));
                    n.set_prop("use_relative", b(s.num("relative").unwrap_or(0.0) != 0.0));
                    n.set_prop("wrap_axis", pick(&["NONE", "XAXIS", "YAXIS", "BOTH"], s.num("wrap_axis").unwrap_or(0.0)));
                }
            }
            "CompositorNodeFlip" => n.set_prop("axis", pick(&["X", "Y", "XY"], c1)),
            "CompositorNodeCrop" => {
                n.set_prop("use_crop_size", b(i1 != 0));
                n.set_prop("relative", b(i2 != 0));
                if let Some(s) = storage("NodeTwoXYs") {
                    for (p, k) in [("min_x", "x1"), ("max_x", "x2"), ("min_y", "y1"), ("max_y", "y2")] {
                        n.set_prop(p, int(&s, k));
                    }
                    for (p, k) in [("rel_min_x", "fac_x1"), ("rel_max_x", "fac_x2"), ("rel_min_y", "fac_y1"), ("rel_max_y", "fac_y2")] {
                        n.set_prop(p, fl(&s, k));
                    }
                }
            }
            "CompositorNodeMapUV" => {
                n.set_prop("alpha", Prop::Int(i1 as i32));
                n.set_prop("filter_type", e(if i2 == 0 { "NEAREST" } else { "ANISOTROPIC" }));
            }
            "CompositorNodeLensdist" => {
                if let Some(s) = storage("NodeLensDist") {
                    n.set_prop("use_projector", b(s.num("proj").unwrap_or(0.0) != 0.0));
                    n.set_prop("use_jitter", b(s.num("jit").unwrap_or(0.0) != 0.0));
                    n.set_prop("use_fit", b(s.num("fit").unwrap_or(0.0) != 0.0));
                }
            }
            // -------------------------------------------------- Utilities
            "CompositorNodeMapRange" => n.set_prop("use_clamp", b(i1 != 0)),
            "CompositorNodeMapValue" => {
                if let Some(s) = storage("TexMapping") {
                    let flag = s.num("flag").unwrap_or(0.0) as i64;
                    n.set_prop("offset", Prop::Float(s.num_at("loc", 0).unwrap_or(0.0) as f32));
                    n.set_prop("size", Prop::Float(s.num_at("size", 0).unwrap_or(1.0) as f32));
                    n.set_prop("use_min", b(flag & 1 != 0));
                    n.set_prop("use_max", b(flag & 2 != 0));
                    n.set_prop("min", Prop::Float(s.num_at("min", 0).unwrap_or(0.0) as f32));
                    n.set_prop("max", Prop::Float(s.num_at("max", 0).unwrap_or(1.0) as f32));
                }
            }
            "CompositorNodeMath" => {
                n.set_prop("operation", e(math_from_file(i1)));
                n.set_prop("use_clamp", b(i2 & 1 != 0));
            }
            "CompositorNodeSwitch" => n.set_prop("check", b(i1 != 0)),
            "CompositorNodeLevels" => n.set_prop("channel", pick(&["COMBINED_RGB", "COMBINED_RGB", "RED", "GREEN", "BLUE", "LUMINANCE"], c1)),
            "CompositorNodeSplit" => {
                n.set_prop("factor", Prop::Int(i1 as i32));
                n.set_prop("axis", pick(&["X", "Y"], c2));
            }
            "CompositorNodeSeparateColor" | "CompositorNodeCombineColor" => {
                if let Some(s) = storage("NodeCMPCombSepColor") {
                    n.set_prop("mode", pick(&["RGB", "HSV", "HSL", "YCC", "YUV"], s.num("mode").unwrap_or(0.0)));
                    n.set_prop("ycc_mode", pick(&["ITUBT601", "ITUBT709", "JFIF"], s.num("ycc_mode").unwrap_or(1.0)));
                }
            }
            "CompositorNodeValToRGB" => {
                if let Some(r) = color_ramp(f, st) {
                    n.set_prop("color_ramp", Prop::Ramp(r));
                }
            }
            "CompositorNodeCurveVec" => {
                if let Some(mut c) = curve_maps(f, st) {
                    c.truncate(3);
                    n.set_prop("mapping", Prop::Curve(CurveMapping { curves: c }));
                }
            }
            _ => return false,
        }
        true
    }
}
