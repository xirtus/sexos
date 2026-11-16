// SPDX-License-Identifier: GPL-3.0-only
//! Grease Pencil from `.blend` files written before 4.3: the legacy
//! `bGPdata` (layers `bGPDlayer`, frames `bGPDframe`, strokes `bGPDstroke`
//! with `bGPDspoint` points) and the materials' `MaterialGPencilStyle`,
//! converted the way Blender's own GPv3 conversion does (stroke thickness
//! times point pressure over 2000 gives the radius).
use super::{BlendFile, View};
use crate::grease_pencil::{CapType, FillStyle, GpFrame, GpLayer, GpMaterial, GpPoint, GpStroke, GradientType, GreasePencil, KeyframeType, LayerBlend, LineMode, StrokeStyle, LEGACY_RADIUS_FACTOR};
use crate::math::{EulerOrder, Quat, Vec3};

// eGPDlayer_Flag
const LAYER_HIDE: i64 = 1 << 0;
const LAYER_LOCKED: i64 = 1 << 1;
const LAYER_USE_MASK: i64 = 1 << 13;
const LAYER_USE_LIGHTS: i64 = 1 << 15;
// eGPDstroke_Flag / eGPDspoint_Flag / eGPDframe_Flag
const STROKE_SELECT: i64 = 1 << 0;
const STROKE_CYCLIC: i64 = 1 << 7;
const POINT_SELECT: i64 = 1 << 0;
const FRAME_SELECT: i64 = 1 << 1;
// eMaterialGPencilStyle_Flag
const MAT_LOCKED: i64 = 1 << 0;
const MAT_HIDE: i64 = 1 << 1;
const MAT_STROKE_SHOW: i64 = 1 << 8;
const MAT_FILL_SHOW: i64 = 1 << 9;
const MAT_STROKE_HOLDOUT: i64 = 1 << 10;
const MAT_FILL_HOLDOUT: i64 = 1 << 11;

fn int(v: &View<'_, '_>, name: &str) -> i64 {
    v.num(name).unwrap_or(0.0) as i64
}

fn fl(v: &View<'_, '_>, name: &str, d: f32) -> f32 {
    v.f32(name).filter(|x| x.is_finite()).unwrap_or(d)
}

/// A material's Grease Pencil style (or a black line for a plain material).
pub(super) fn material(f: &BlendFile<'_>, ma: &View<'_, '_>) -> GpMaterial {
    let mut m = GpMaterial::new(&ma.id_name(), [0.0, 0.0, 0.0, 1.0]);
    let Some(st) = f.view_ptr(ma.ptr("gp_style"), &["MaterialGPencilStyle"]) else { return m };
    m.color = st.vec::<4>("stroke_rgba").unwrap_or(m.color);
    m.fill_color = st.vec::<4>("fill_rgba").unwrap_or(m.fill_color);
    m.mix_color = st.vec::<4>("mix_rgba").unwrap_or(m.mix_color);
    let flag = int(&st, "flag");
    m.show_stroke = flag & MAT_STROKE_SHOW != 0;
    m.show_fill = flag & MAT_FILL_SHOW != 0;
    m.use_stroke_holdout = flag & MAT_STROKE_HOLDOUT != 0;
    m.use_fill_holdout = flag & MAT_FILL_HOLDOUT != 0;
    m.lock = flag & MAT_LOCKED != 0;
    m.hide = flag & MAT_HIDE != 0;
    m.mode = match int(&st, "mode") {
        1 => LineMode::Dots,
        2 => LineMode::Squares,
        _ => LineMode::Line,
    };
    m.stroke_style = if int(&st, "stroke_style") == 1 { StrokeStyle::Texture } else { StrokeStyle::Solid };
    // GP_MATERIAL_FILL_STYLE_SOLID 0, GRADIENT 1, CHECKER 2 (gone), TEXTURE 3.
    m.fill_style = match int(&st, "fill_style") {
        1 => FillStyle::Gradient,
        3 => FillStyle::Texture,
        _ => FillStyle::Solid,
    };
    m.gradient_type = if int(&st, "gradient_type") == 1 { GradientType::Radial } else { GradientType::Linear };
    m.mix_factor = fl(&st, "mix_factor", m.mix_factor);
    m.texture_angle = fl(&st, "texture_angle", 0.0);
    m.texture_scale = st.vec::<2>("texture_scale").unwrap_or(m.texture_scale);
    m.texture_offset = st.vec::<2>("texture_offset").unwrap_or(m.texture_offset);
    m.pass_index = int(&st, "index").max(0) as u32;
    m
}

fn stroke(f: &BlendFile<'_>, s: &View<'_, '_>) -> GpStroke {
    let n = int(s, "totpoints").clamp(0, 10_000_000) as usize;
    let thickness = fl(s, "thickness", 3.0);
    let pts = f.view_array(s.ptr("points"), &["bGPDspoint"], n).unwrap_or_default();
    let dverts = f.view_array(s.ptr("dvert"), &["MDeformVert"], n);
    let mut out = GpStroke::new(Vec::new(), int(s, "mat_nr").max(0) as usize);
    out.points = pts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let pos = Vec3::new(fl(p, "x", 0.0), fl(p, "y", 0.0), fl(p, "z", 0.0));
            let mut q = GpPoint::new(pos, thickness * fl(p, "pressure", 1.0) * LEGACY_RADIUS_FACTOR);
            q.opacity = fl(p, "strength", 1.0);
            q.vertex_color = p.vec::<4>("vert_color").unwrap_or([0.0; 4]);
            q.rotation = fl(p, "uv_rot", 0.0);
            q.select = int(p, "flag") & POINT_SELECT != 0;
            if let Some(dv) = dverts.as_ref().and_then(|d| d.get(i)) {
                let tw = int(dv, "totweight").clamp(0, 1024) as usize;
                for w in f.view_array(dv.ptr("dw"), &["MDeformWeight"], tw).unwrap_or_default() {
                    q.set_weight(int(&w, "def_nr").max(0) as u32, fl(&w, "weight", 0.0));
                }
            }
            q
        })
        .collect();
    let flag = int(s, "flag");
    out.cyclic = flag & STROKE_CYCLIC != 0;
    out.select = flag & STROKE_SELECT != 0;
    let cap = |i: usize| if s.num_at("caps", i).unwrap_or(0.0) as i64 == 1 { CapType::Flat } else { CapType::Round };
    out.start_cap = cap(0);
    out.end_cap = cap(1);
    out.softness = 1.0 - fl(s, "hardeness", 1.0).clamp(0.0, 1.0);
    out.fill_color = s.vec::<4>("vert_color_fill").unwrap_or(out.fill_color);
    out.fill_opacity = fl(s, "fill_opacity_fac", 1.0);
    out.uv_rotation = fl(s, "uv_rotation", 0.0);
    out.uv_offset = s.vec::<2>("uv_translation").map_or(0.0, |t| t[0]);
    out.uv_scale = fl(s, "uv_scale", 1.0);
    out.init_time = s.num("inittime").unwrap_or(0.0) as f32;
    out
}

fn layer(f: &BlendFile<'_>, l: &View<'_, '_>) -> GpLayer {
    let mut out = GpLayer::new(&l.string("info").unwrap_or_else(|| "Layer".into()));
    let flag = int(l, "flag");
    out.hide = flag & LAYER_HIDE != 0;
    out.lock = flag & LAYER_LOCKED != 0;
    out.use_masks = flag & LAYER_USE_MASK != 0;
    out.use_lights = flag & LAYER_USE_LIGHTS != 0;
    out.opacity = fl(l, "opacity", 1.0);
    out.blend = match int(l, "blend_mode") {
        1 => LayerBlend::HardLight,
        2 => LayerBlend::Add,
        3 => LayerBlend::Subtract,
        4 => LayerBlend::Multiply,
        5 => LayerBlend::Divide,
        _ => LayerBlend::Regular,
    };
    if let Some(t) = l.vec::<4>("tintcolor") {
        out.tint_color = [t[0], t[1], t[2]];
        out.tint_factor = t[3];
    }
    out.radius_offset = fl(l, "line_change", 0.0) * LEGACY_RADIUS_FACTOR;
    out.transform.location = l.vec3("location").unwrap_or(Vec3::ZERO);
    if let Some(r) = l.vec::<3>("rotation") {
        out.transform.rotation = Quat::from_euler(r, EulerOrder::XYZ);
    }
    out.transform.scale = l.vec3("scale").filter(|s| *s != Vec3::ZERO).unwrap_or(Vec3::ONE);
    out.parent = f.view_ptr(l.ptr("parent"), &["Object"]).map(|o| o.id_name());
    out.parent_bone = l.string("parsubstr").unwrap_or_default();
    out.pass_index = int(l, "pass_index").max(0) as u32;
    out.use_onion_skinning = int(l, "onion_flag") & 1 != 0;
    out.masks = f.list(l.sub("mask_layers"), &["bGPDlayer_Mask"]).iter().filter_map(|m| m.string("name")).collect();
    out.frames = f
        .list(l.sub("frames"), &["bGPDframe"])
        .iter()
        .map(|fr| GpFrame {
            frame: int(fr, "framenum") as i32,
            drawing: crate::grease_pencil::Drawing { strokes: f.list(fr.sub("strokes"), &["bGPDstroke"]).iter().map(|s| stroke(f, s)).collect() },
            key_type: KeyframeType::from_blender(int(fr, "key_type")),
            select: int(fr, "flag") & FRAME_SELECT != 0,
        })
        .collect();
    out.sort_frames();
    out
}

/// A legacy `bGPdata` block. `object_materials` are the owner object's
/// slots (used when the data has none of its own).
pub(super) fn legacy(f: &BlendFile<'_>, gd: &View<'_, '_>, object_materials: &[u64], object_groups: Vec<String>) -> GreasePencil {
    let mut g = GreasePencil::default();
    let layers = f.list(gd.sub("layers"), &["bGPDlayer"]);
    g.layers = layers.iter().map(|l| layer(f, l)).collect();
    g.active_layer = layers.iter().position(|l| int(l, "flag") & (1 << 2) != 0).unwrap_or(g.layers.len().saturating_sub(1));
    let n = int(gd, "totcol").clamp(0, 32767) as usize;
    let mut mats = f.ptr_array(gd.ptr("mat"), n);
    if mats.iter().all(|&p| p == 0) {
        mats = object_materials.to_vec();
    }
    g.materials = mats.iter().map(|&p| f.view_ptr(p, &["Material"]).map_or_else(|| GpMaterial::new("Material", [0.0, 0.0, 0.0, 1.0]), |ma| material(f, &ma))).collect();
    if g.materials.is_empty() {
        g.materials.push(GpMaterial::new("Black", [0.0, 0.0, 0.0, 1.0]));
    }
    g.vertex_groups = super::rig::group_names(f, gd.sub("vertex_group_names"));
    if g.vertex_groups.is_empty() {
        g.vertex_groups = object_groups;
    }
    // Onion skinning: steps before/after, opacity and custom colours.
    if let Some(v) = gd.num("gstep") {
        g.onion.before = v.max(0.0) as u32;
    }
    if let Some(v) = gd.num("gstep_next") {
        g.onion.after = v.max(0.0) as u32;
    }
    g.onion.opacity = fl(gd, "onion_factor", g.onion.opacity);
    if let Some(c) = gd.vec::<3>("gcolor_prev") {
        g.onion.color_before = c;
    }
    if let Some(c) = gd.vec::<3>("gcolor_next") {
        g.onion.color_after = c;
    }
    g
}
