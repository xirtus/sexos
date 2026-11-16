// SPDX-License-Identifier: GPL-3.0-only
//! The Grease Pencil settings as [`Visit`] walks: layers, groups, materials,
//! onion skinning, modifiers and visual effects. The file format and the
//! Properties panels both go through these.
use super::effects::{ColorizeMode, EffectKind, GlowMode, GpEffect};
use super::lineart::LineArtSource;
use super::modifiers::{BuildMode, BuildTransition, ColorMode, DashSegment, EnvelopeMode, GpModKind, GpModifier, Influence, SimplifyMode, TimeMode};
use super::*;
use crate::props::{pick, NameKind, Range, Visit};

const BLENDS: [(LayerBlend, &str, &str); 6] = [
    (LayerBlend::Regular, "REGULAR", "Regular"),
    (LayerBlend::HardLight, "HARDLIGHT", "Hard Light"),
    (LayerBlend::Add, "ADD", "Add"),
    (LayerBlend::Subtract, "SUBTRACT", "Subtract"),
    (LayerBlend::Multiply, "MULTIPLY", "Multiply"),
    (LayerBlend::Divide, "DIVIDE", "Divide"),
];
const LINE_MODES: [(LineMode, &str, &str); 3] = [(LineMode::Line, "LINE", "Line"), (LineMode::Dots, "DOTS", "Dots"), (LineMode::Squares, "BOX", "Squares")];
const STROKE_STYLES: [(StrokeStyle, &str, &str); 2] = [(StrokeStyle::Solid, "SOLID", "Solid"), (StrokeStyle::Texture, "TEXTURE", "Texture")];
const FILL_STYLES: [(FillStyle, &str, &str); 3] = [(FillStyle::Solid, "SOLID", "Solid"), (FillStyle::Gradient, "GRADIENT", "Gradient"), (FillStyle::Texture, "TEXTURE", "Texture")];
const GRADIENTS: [(GradientType, &str, &str); 2] = [(GradientType::Linear, "LINEAR", "Linear"), (GradientType::Radial, "RADIAL", "Radial")];
const ONION_MODES: [(OnionMode, &str, &str); 3] = [(OnionMode::Keyframes, "KEYFRAMES", "Keyframes"), (OnionMode::Frames, "ABSOLUTE", "Frames"), (OnionMode::Selected, "SELECTED", "Selected")];
const DEPTH_ORDERS: [(DepthOrder, &str, &str); 2] = [(DepthOrder::Layers2D, "2D_LAYERS", "2D Layers"), (DepthOrder::Location3D, "3D_LOCATION", "3D Location")];
const BUILD_MODES: [(BuildMode, &str, &str); 3] = [(BuildMode::Sequential, "SEQUENTIAL", "Sequential"), (BuildMode::Concurrent, "CONCURRENT", "Concurrent"), (BuildMode::Additive, "ADDITIVE", "Additive")];
const BUILD_TRANSITIONS: [(BuildTransition, &str, &str); 3] = [(BuildTransition::Grow, "GROW", "Grow"), (BuildTransition::Shrink, "SHRINK", "Shrink"), (BuildTransition::Fade, "FADE", "Vanish")];
const ENVELOPE_MODES: [(EnvelopeMode, &str, &str); 3] = [(EnvelopeMode::Deform, "DEFORM", "Deform"), (EnvelopeMode::Segments, "SEGMENTS", "Segments"), (EnvelopeMode::Fills, "FILLS", "Fills")];
const COLOR_MODES: [(ColorMode, &str, &str); 3] = [(ColorMode::Stroke, "STROKE", "Stroke"), (ColorMode::Fill, "FILL", "Fill"), (ColorMode::Both, "BOTH", "Stroke & Fill")];
const TIME_MODES: [(TimeMode, &str, &str); 5] = [
    (TimeMode::Normal, "NORMAL", "Regular"),
    (TimeMode::Reverse, "REVERSE", "Reverse"),
    (TimeMode::Fixed, "FIX", "Fixed Frame"),
    (TimeMode::PingPong, "PINGPONG", "Ping Pong"),
    (TimeMode::Chain, "CHAIN", "Chain"),
];
const COLORIZE_MODES: [(ColorizeMode, &str, &str); 5] = [
    (ColorizeMode::GrayScale, "GRAYSCALE", "Gray Scale"),
    (ColorizeMode::Sepia, "SEPIA", "Sepia"),
    (ColorizeMode::Duotone, "DUOTONE", "Duotone"),
    (ColorizeMode::Transparent, "TRANSPARENT", "Transparent"),
    (ColorizeMode::Custom, "CUSTOM", "Custom"),
];
const GLOW_MODES: [(GlowMode, &str, &str); 2] = [(GlowMode::Luminance, "LUMINANCE", "Luminance"), (GlowMode::Color, "COLOR", "Color")];

/// Location, rotation (XYZ Euler, radians) and scale of a transform.
pub fn visit_transform(v: &mut dyn Visit, t: &mut crate::scene::Transform) {
    let mut loc = t.location.to_array();
    v.floats("location", "Location", &mut loc, Range::ANY, false);
    t.location = Vec3::from_array(loc);
    let e = t.rotation.to_euler(crate::math::EulerOrder::XYZ);
    let mut rot = e;
    v.floats("rotation", "Rotation", &mut rot, Range::ANGLE, false);
    if rot != e {
        t.rotation = crate::math::Quat::from_euler(rot, crate::math::EulerOrder::XYZ);
    }
    let mut sc = t.scale.to_array();
    v.floats("scale", "Scale", &mut sc, Range::ANY, false);
    t.scale = Vec3::from_array(sc);
}

fn opt_name(v: &mut dyn Visit, key: &str, label: &str, o: &mut Option<String>, kind: NameKind) {
    let mut s = o.clone().unwrap_or_default();
    v.name(key, label, &mut s, kind);
    *o = (!s.is_empty()).then_some(s);
}

fn name_list(v: &mut dyn Visit, key: &str, label: &str, list: &mut Vec<String>) {
    let mut s = list.join("\n");
    v.name(key, label, &mut s, NameKind::Text);
    *list = s.split('\n').filter(|x| !x.is_empty()).map(String::from).collect();
}

impl GpLayer {
    /// Settings (not the name, frames or group, which have their own words).
    pub fn visit(&mut self, v: &mut dyn Visit) {
        pick(v, "blend", "Blend", &BLENDS, &mut self.blend);
        v.float("opacity", "Opacity", &mut self.opacity, Range::FACTOR);
        v.boolean("use_lights", "Lights", &mut self.use_lights);
        v.boolean("hide", "Hide", &mut self.hide);
        v.boolean("lock", "Lock", &mut self.lock);
        v.boolean("use_onion_skinning", "Onion Skinning", &mut self.use_onion_skinning);
        v.heading("Adjustments");
        v.floats("tint_color", "Tint Color", &mut self.tint_color, Range::FACTOR, true);
        v.float("tint_factor", "Tint Factor", &mut self.tint_factor, Range::FACTOR);
        v.float("radius_offset", "Stroke Thickness", &mut self.radius_offset, Range::ANY);
        v.heading("Transform");
        visit_transform(v, &mut self.transform);
        v.heading("Relations");
        opt_name(v, "parent", "Parent", &mut self.parent, NameKind::Object);
        v.name("parent_bone", "Bone", &mut self.parent_bone, NameKind::Text);
        v.uint("pass_index", "Pass Index", &mut self.pass_index, 0, 255);
        v.heading("Masks");
        v.boolean("use_masks", "Use Masks", &mut self.use_masks);
        name_list(v, "masks", "Mask Layers", &mut self.masks);
    }
}

impl LayerGroup {
    pub fn visit(&mut self, v: &mut dyn Visit) {
        v.boolean("hide", "Hide", &mut self.hide);
        v.boolean("lock", "Lock", &mut self.lock);
        v.boolean("expanded", "Expanded", &mut self.expanded);
        v.byte("color_tag", "Color Tag", &mut self.color_tag, 8);
    }
}

impl GpMaterial {
    pub fn visit(&mut self, v: &mut dyn Visit) {
        v.heading("Surface");
        v.boolean("show_stroke", "Stroke", &mut self.show_stroke);
        pick(v, "mode", "Line Type", &LINE_MODES, &mut self.mode);
        pick(v, "stroke_style", "Style", &STROKE_STYLES, &mut self.stroke_style);
        v.floats("color", "Base Color", &mut self.color, Range::FACTOR, true);
        v.index("stroke_image", "Stroke Image", &mut self.stroke_image);
        v.boolean("use_stroke_holdout", "Holdout", &mut self.use_stroke_holdout);
        v.boolean("show_fill", "Fill", &mut self.show_fill);
        pick(v, "fill_style", "Fill Style", &FILL_STYLES, &mut self.fill_style);
        v.floats("fill_color", "Fill Color", &mut self.fill_color, Range::FACTOR, true);
        pick(v, "gradient_type", "Gradient Type", &GRADIENTS, &mut self.gradient_type);
        v.floats("mix_color", "Secondary Color", &mut self.mix_color, Range::FACTOR, true);
        v.float("mix_factor", "Blend", &mut self.mix_factor, Range::FACTOR);
        v.index("fill_image", "Fill Image", &mut self.fill_image);
        v.floats("texture_scale", "Texture Scale", &mut self.texture_scale, Range::ANY, false);
        v.floats("texture_offset", "Texture Offset", &mut self.texture_offset, Range::ANY, false);
        v.float("texture_angle", "Texture Rotation", &mut self.texture_angle, Range::ANGLE);
        v.boolean("use_fill_holdout", "Fill Holdout", &mut self.use_fill_holdout);
        v.heading("Settings");
        v.boolean("hide", "Hide", &mut self.hide);
        v.boolean("lock", "Lock", &mut self.lock);
        v.uint("pass_index", "Pass Index", &mut self.pass_index, 0, 255);
    }
}

impl OnionSkin {
    pub fn visit(&mut self, v: &mut dyn Visit) {
        v.boolean("onion", "Onion Skin Overlay", &mut self.enabled);
        pick(v, "onion_mode", "Mode", &ONION_MODES, &mut self.mode);
        v.float("onion_opacity", "Opacity", &mut self.opacity, Range::FACTOR);
        v.uint("ghost_before", "Keyframes Before", &mut self.before, 0, 120);
        v.uint("ghost_after", "Keyframes After", &mut self.after, 0, 120);
        v.boolean("use_onion_colors", "Custom Colors", &mut self.use_colors);
        v.floats("before_color", "Before", &mut self.color_before, Range::FACTOR, true);
        v.floats("after_color", "After", &mut self.color_after, Range::FACTOR, true);
        v.boolean("use_onion_fade", "Fade", &mut self.use_fade);
    }
}

impl GreasePencil {
    /// Data-block settings (onion skinning, stroke depth order, actives).
    pub fn visit(&mut self, v: &mut dyn Visit) {
        pick(v, "stroke_depth_order", "Stroke Depth Order", &DEPTH_ORDERS, &mut self.depth_order);
        self.onion.visit(v);
    }
}

impl Influence {
    pub fn visit(&mut self, v: &mut dyn Visit) {
        v.heading("Influence");
        v.name("layer_filter", "Layer", &mut self.layer, NameKind::Layer);
        v.boolean("invert_layer_filter", "Invert Layer", &mut self.invert_layer);
        v.uint("layer_pass_filter", "Layer Pass", &mut self.layer_pass, 0, 255);
        v.name("material_filter", "Material", &mut self.material, NameKind::Material);
        v.boolean("invert_material_filter", "Invert Material", &mut self.invert_material);
        v.name("vertex_group_name", "Vertex Group", &mut self.vertex_group, NameKind::VertexGroup);
        v.boolean("invert_vertex_group", "Invert Vertex Group", &mut self.invert_vertex_group);
    }
}

fn three(v: &mut dyn Visit, key: &str, label: &str, a: &mut [f32; 3], r: Range) {
    v.floats(key, label, a, r, false);
}

impl GpModifier {
    /// The type's settings, then the influence filters.
    pub fn visit(&mut self, v: &mut dyn Visit) {
        v.boolean("show_viewport", "Viewport", &mut self.show_viewport);
        v.boolean("show_render", "Render", &mut self.show_render);
        match &mut self.kind {
            GpModKind::Array { count, constant, relative, use_constant, use_relative, random_offset, random_rotation, random_scale, seed, material } => {
                v.uint("count", "Count", count, 1, 10_000);
                v.boolean("use_relative_offset", "Relative Offset", use_relative);
                three(v, "relative_offset", "Factor", relative, Range::ANY);
                v.boolean("use_constant_offset", "Constant Offset", use_constant);
                three(v, "constant_offset", "Distance", constant, Range::ANY);
                three(v, "offset", "Random Offset", random_offset, Range::ANY);
                three(v, "rotation", "Random Rotation", random_rotation, Range::ANGLE);
                three(v, "scale", "Random Scale", random_scale, Range::ANY);
                v.uint("seed", "Seed", seed, 0, u32::MAX);
                v.index("replace_material", "Material Override", material);
            }
            GpModKind::Build { mode, transition, start_delay, length, use_restrict, frame_start, frame_end } => {
                pick(v, "mode", "Mode", &BUILD_MODES, mode);
                pick(v, "transition", "Transition", &BUILD_TRANSITIONS, transition);
                v.float("start_delay", "Delay", start_delay, Range::POSITIVE);
                v.float("length", "Frames", length, Range::new(1.0, f32::INFINITY));
                v.boolean("use_restrict_frame_range", "Restrict Frame Range", use_restrict);
                v.float("frame_start", "Start", frame_start, Range::ANY);
                v.float("frame_end", "End", frame_end, Range::ANY);
            }
            GpModKind::Envelope { mode, spread, thickness, strength, skip } => {
                pick(v, "mode", "Mode", &ENVELOPE_MODES, mode);
                v.uint("spread", "Spread Length", spread, 1, 1000);
                v.float("thickness", "Thickness", thickness, Range::POSITIVE);
                v.float("strength", "Strength", strength, Range::FACTOR);
                v.uint("skip", "Skip Segments", skip, 0, 1000);
            }
            GpModKind::Length { start, end, relative } => {
                v.boolean("use_relative", "Relative", relative);
                v.float("start_factor", "Start", start, Range::ANY);
                v.float("end_factor", "End", end, Range::ANY);
            }
            GpModKind::LineArt(s) => {
                let mut src = match &s.source {
                    LineArtSource::Scene => 0,
                    LineArtSource::Collection(_) => 1,
                    LineArtSource::Object(_) => 2,
                };
                let mut name = match &s.source {
                    LineArtSource::Scene => String::new(),
                    LineArtSource::Collection(n) | LineArtSource::Object(n) => n.clone(),
                };
                v.choice("source_type", "Source Type", &[("SCENE", "Scene"), ("COLLECTION", "Collection"), ("OBJECT", "Object")], &mut src);
                v.name("source", "Source", &mut name, if src == 1 { NameKind::Collection } else { NameKind::Object });
                s.source = match src {
                    1 => LineArtSource::Collection(name),
                    2 => LineArtSource::Object(name),
                    _ => LineArtSource::Scene,
                };
                v.name("source_camera", "Camera", &mut s.camera, NameKind::Object);
                v.name("target_layer", "Layer", &mut s.target_layer, NameKind::Layer);
                let mut m = Some(s.material);
                v.index("target_material", "Material", &mut m);
                s.material = m.unwrap_or(0);
                v.float("radius", "Line Radius", &mut s.radius, Range::POSITIVE);
                v.float("opacity", "Opacity", &mut s.opacity, Range::FACTOR);
                v.boolean("use_contour", "Contour", &mut s.use_contour);
                v.boolean("use_crease", "Crease", &mut s.use_crease);
                v.float("crease_threshold", "Crease Threshold", &mut s.crease_threshold, Range::new(0.0, 180.0));
                v.boolean("use_edge_mark", "Border", &mut s.use_border);
                v.boolean("use_loose", "Loose", &mut s.use_loose);
                v.uint("resolution", "Resolution", &mut s.resolution, 16, 8192);
                v.float("aspect", "Aspect", &mut s.aspect, Range::new(0.1, 10.0));
            }
            GpModKind::Mirror { axes, object } => {
                v.bools("use_axis", "Axis", axes);
                v.name("object", "Object", object, NameKind::Object);
            }
            GpModKind::Multiply { duplicates, distance, offset, use_fade, fade_center, thickness_fade, opacity_fade } => {
                v.uint("duplicates", "Duplicates", duplicates, 0, 100);
                v.float("distance", "Distance", distance, Range::ANY);
                v.float("offset", "Offset", offset, Range::new(-1.0, 1.0));
                v.boolean("use_fade", "Fade", use_fade);
                v.float("fading_center", "Center", fade_center, Range::FACTOR);
                v.float("fading_thickness", "Thickness", thickness_fade, Range::FACTOR);
                v.float("fading_opacity", "Opacity", opacity_fade, Range::FACTOR);
            }
            GpModKind::Outline { thickness, keep_shape, subdivision, sample_length, material } => {
                v.float("thickness", "Thickness", thickness, Range::POSITIVE);
                v.boolean("use_keep_shape", "Keep Shape", keep_shape);
                v.uint("subdivision", "Subdivisions", subdivision, 0, 10);
                v.float("sample_length", "Sample Length", sample_length, Range::POSITIVE);
                v.index("outline_material", "Outline Material", material);
            }
            GpModKind::Simplify(m) => {
                let mut i = match m {
                    SimplifyMode::Fixed { .. } => 0,
                    SimplifyMode::Adaptive { .. } => 1,
                    SimplifyMode::Sample { .. } => 2,
                    SimplifyMode::Merge { .. } => 3,
                };
                let was = i;
                v.choice("mode", "Mode", &[("FIXED", "Fixed"), ("ADAPTIVE", "Adaptive"), ("SAMPLE", "Sample"), ("MERGE", "Merge")], &mut i);
                if i != was {
                    *m = match i {
                        1 => SimplifyMode::Adaptive { factor: 0.0 },
                        2 => SimplifyMode::Sample { length: 0.1 },
                        3 => SimplifyMode::Merge { distance: 0.001 },
                        _ => SimplifyMode::Fixed { step: 1 },
                    };
                }
                match m {
                    SimplifyMode::Fixed { step } => v.uint("step", "Iterations", step, 1, 50),
                    SimplifyMode::Adaptive { factor } => v.float("factor", "Factor", factor, Range::POSITIVE),
                    SimplifyMode::Sample { length } => v.float("length", "Length", length, Range::new(0.001, f32::INFINITY)),
                    SimplifyMode::Merge { distance } => v.float("distance", "Distance", distance, Range::POSITIVE),
                }
            }
            GpModKind::Smooth { factor, steps, position, thickness, strength } => {
                v.boolean("use_edit_position", "Position", position);
                v.boolean("use_edit_thickness", "Thickness", thickness);
                v.boolean("use_edit_strength", "Strength", strength);
                v.float("factor", "Factor", factor, Range::new(0.0, 2.0));
                v.uint("step", "Repeat", steps, 1, 100);
            }
            GpModKind::Subdivide { level, simple } => {
                v.boolean("simple", "Simple", simple);
                v.uint("level", "Subdivisions", level, 0, 16);
            }
            GpModKind::Thickness { factor, thickness, normalize } => {
                v.boolean("use_uniform_thickness", "Uniform Thickness", normalize);
                v.float("thickness", "Thickness", thickness, Range::POSITIVE);
                v.float("thickness_factor", "Thickness Factor", factor, Range::POSITIVE);
            }
            GpModKind::Noise { factor, factor_thickness, factor_strength, noise_scale, noise_offset, seed, step, use_random } => {
                v.float("factor", "Position", factor, Range::POSITIVE);
                v.float("factor_thickness", "Thickness", factor_thickness, Range::POSITIVE);
                v.float("factor_strength", "Strength", factor_strength, Range::POSITIVE);
                v.float("noise_scale", "Noise Scale", noise_scale, Range::POSITIVE);
                v.float("noise_offset", "Noise Offset", noise_offset, Range::ANY);
                v.uint("seed", "Seed", seed, 0, u32::MAX);
                v.boolean("use_random", "Random", use_random);
                v.uint("step", "Step", step, 1, 100);
            }
            GpModKind::Offset { location, rotation, scale, random_offset, random_rotation, random_scale, seed } => {
                three(v, "location", "Location", location, Range::ANY);
                three(v, "rotation", "Rotation", rotation, Range::ANGLE);
                three(v, "scale", "Scale", scale, Range::ANY);
                three(v, "stroke_location", "Random Offset", random_offset, Range::ANY);
                three(v, "stroke_rotation", "Random Rotation", random_rotation, Range::ANGLE);
                three(v, "stroke_scale", "Random Scale", random_scale, Range::ANY);
                v.uint("seed", "Seed", seed, 0, u32::MAX);
            }
            GpModKind::Opacity { factor, mode, normalize } => {
                pick(v, "color_mode", "Mode", &COLOR_MODES, mode);
                v.float("color_factor", "Opacity Factor", factor, Range::new(0.0, 2.0));
                v.boolean("use_uniform_opacity", "Uniform Opacity", normalize);
            }
            GpModKind::Tint { color, factor, gradient, object, radius, mode } => {
                pick(v, "color_mode", "Mode", &COLOR_MODES, mode);
                v.float("factor", "Factor", factor, Range::new(0.0, 2.0));
                v.boolean("use_gradient", "Gradient", gradient);
                v.floats("color", "Color", color, Range::FACTOR, true);
                v.name("object", "Object", object, NameKind::Object);
                v.float("radius", "Radius", radius, Range::POSITIVE);
            }
            GpModKind::Hook { object, strength, falloff_radius } => {
                v.name("object", "Object", object, NameKind::Object);
                v.float("strength", "Strength", strength, Range::FACTOR);
                v.float("falloff_radius", "Radius", falloff_radius, Range::POSITIVE);
            }
            GpModKind::Lattice { object, strength } => {
                v.name("object", "Object", object, NameKind::Object);
                v.float("strength", "Strength", strength, Range::FACTOR);
            }
            GpModKind::Armature { object, use_vertex_groups, use_bone_envelopes } => {
                v.name("object", "Object", object, NameKind::Object);
                v.boolean("use_vertex_groups", "Vertex Groups", use_vertex_groups);
                v.boolean("use_bone_envelopes", "Bone Envelopes", use_bone_envelopes);
            }
            GpModKind::TimeOffset { mode, offset, scale, use_custom_range, frame_start, frame_end } => {
                pick(v, "mode", "Mode", &TIME_MODES, mode);
                v.int("offset", "Frame Offset", offset, -100_000, 100_000);
                v.float("frame_scale", "Frame Scale", scale, Range::new(0.001, 100.0));
                v.boolean("use_custom_frame_range", "Custom Range", use_custom_range);
                v.int("frame_start", "Start", frame_start, -100_000, 100_000);
                v.int("frame_end", "End", frame_end, -100_000, 100_000);
            }
            GpModKind::Dash { segments, offset } => {
                v.int("dash_offset", "Offset", offset, -10_000, 10_000);
                let mut n = segments.len() as u32;
                v.uint("segments", "Segments", &mut n, 1, 64);
                segments.resize(n.max(1) as usize, DashSegment::default());
                for (i, s) in segments.iter_mut().enumerate() {
                    v.heading(&format!("Segment {}", i + 1));
                    v.name(&format!("seg{i}_name"), "Name", &mut s.name, NameKind::Text);
                    v.uint(&format!("seg{i}_dash"), "Dash", &mut s.dash, 1, 1000);
                    v.uint(&format!("seg{i}_gap"), "Gap", &mut s.gap, 0, 1000);
                    v.float(&format!("seg{i}_radius"), "Radius", &mut s.radius, Range::POSITIVE);
                    v.float(&format!("seg{i}_opacity"), "Opacity", &mut s.opacity, Range::FACTOR);
                    v.index(&format!("seg{i}_material"), "Material", &mut s.material);
                    v.boolean(&format!("seg{i}_cyclic"), "Cyclic", &mut s.use_cyclic);
                }
            }
            GpModKind::TextureMapping { uv_offset, uv_scale, rotation } => {
                v.float("uv_offset", "UV Offset", uv_offset, Range::ANY);
                v.float("uv_scale", "UV Scale", uv_scale, Range::new(0.01, 100.0));
                v.float("rotation", "Rotation", rotation, Range::ANGLE);
            }
        }
        self.influence.visit(v);
    }
}

impl GpEffect {
    pub fn visit(&mut self, v: &mut dyn Visit) {
        v.boolean("show_viewport", "Viewport", &mut self.show_viewport);
        v.boolean("show_render", "Render", &mut self.show_render);
        let px = Range { step: 1.0, ..Range::POSITIVE };
        match &mut self.kind {
            EffectKind::Blur { size, samples, rotation } => {
                v.floats("size", "Size", size, px, false);
                v.uint("samples", "Samples", samples, 0, 32);
                v.float("rotation", "Rotation", rotation, Range::ANGLE);
            }
            EffectKind::Colorize { mode, low_color, high_color, factor } => {
                pick(v, "mode", "Mode", &COLORIZE_MODES, mode);
                v.floats("low_color", "Low Color", low_color, Range::FACTOR, true);
                v.floats("high_color", "High Color", high_color, Range::FACTOR, true);
                v.float("factor", "Factor", factor, Range::FACTOR);
            }
            EffectKind::Flip { horizontal, vertical } => {
                v.boolean("use_flip_x", "Horizontal", horizontal);
                v.boolean("use_flip_y", "Vertical", vertical);
            }
            EffectKind::Glow { mode, threshold, select_color, glow_color, size, opacity, use_glow_under } => {
                pick(v, "mode", "Mode", &GLOW_MODES, mode);
                v.float("threshold", "Threshold", threshold, Range::FACTOR);
                v.floats("select_color", "Select Color", select_color, Range::FACTOR, true);
                v.floats("glow_color", "Glow Color", glow_color, Range::FACTOR, true);
                v.floats("size", "Size", size, px, false);
                v.float("opacity", "Opacity", opacity, Range::FACTOR);
                v.boolean("use_glow_under", "Glow Under", use_glow_under);
            }
            EffectKind::Pixelate { size } => v.uints("size", "Size", size),
            EffectKind::Rim { offset, rim_color, mask_color, blur } => {
                v.ints("offset", "Offset", offset);
                v.floats("rim_color", "Rim Color", rim_color, Range::FACTOR, true);
                v.floats("mask_color", "Mask Color", mask_color, Range::FACTOR, true);
                v.floats("blur", "Blur", blur, px, false);
            }
            EffectKind::Shadow { offset, scale, rotation, color, blur } => {
                v.floats("shadow_color", "Shadow Color", color, Range::FACTOR, true);
                v.ints("offset", "Offset", offset);
                v.floats("scale", "Scale", scale, Range::ANY, false);
                v.float("rotation", "Rotation", rotation, Range::ANGLE);
                v.floats("blur", "Blur", blur, px, false);
            }
            EffectKind::Swirl { radius, angle } => {
                v.float("radius", "Radius", radius, px);
                v.float("angle", "Angle", angle, Range::ANGLE);
            }
            EffectKind::Wave { orientation_vertical, amplitude, period, phase } => {
                let mut o = *orientation_vertical as usize;
                v.choice("orientation", "Orientation", &[("HORIZONTAL", "Horizontal"), ("VERTICAL", "Vertical")], &mut o);
                *orientation_vertical = o == 1;
                v.float("amplitude", "Amplitude", amplitude, Range::ANY);
                v.float("period", "Period", period, Range::new(0.01, f32::INFINITY));
                v.float("phase", "Phase", phase, Range::ANY);
            }
        }
    }
}
