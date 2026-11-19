// SPDX-License-Identifier: GPL-3.0-only
//! Grease Pencil for the `.blend` writer: the legacy `bGPdata` that Blender
//! 4.2 saves (layers `bGPDlayer`, frames `bGPDframe`, strokes `bGPDstroke`
//! with `bGPDspoint` points and `MDeformVert` weights, layer masks) and the
//! Grease Pencil materials (`Material.gp_style`). The inverse of
//! `blend/gpencil.rs`: a point's radius is the stroke `thickness` times its
//! `pressure` over 2000, so each stroke gets the thickness of its widest
//! point and the pressures carry the rest. Flags and enums follow Blender
//! 4.2's `DNA_gpencil_legacy_types.h` and `DNA_material_types.h` (GPL-2+).
use super::export::Ids;
use super::write::{Rec, W};
use crate::grease_pencil::{CapType, DepthOrder, FillStyle, GpLayer, GpMaterial, GpStroke, GradientType, GreasePencil, KeyframeType, LayerBlend, LineMode, OnionMode, StrokeStyle, LEGACY_RADIUS_FACTOR};
use crate::math::EulerOrder;

fn bits(on: &[(bool, i64)]) -> i64 {
    on.iter().filter(|(b, _)| *b).fold(0, |a, (_, m)| a | m)
}

fn key_type(k: KeyframeType) -> i64 {
    (0..6).find(|&v| KeyframeType::from_blender(v) == k).unwrap_or(0)
}

impl W {
    /// A Grease Pencil material (`MA` ID with `gp_style`).
    pub(super) fn gp_material(&mut self, addr: u64, name: &str, m: &GpMaterial, ids: &Ids) {
        self.start_id();
        let mut r = self.id("Material", "MA", name);
        self.f(&mut r, "r", &[m.color[0]]);
        self.f(&mut r, "g", &[m.color[1]]);
        self.f(&mut r, "b", &[m.color[2]]);
        self.i(&mut r, "use_nodes", 0);
        let st = self.addr();
        self.p(&mut r, "gp_style", st);
        self.put(b"MA", addr, &[r]);
        let mut s = self.rec("MaterialGPencilStyle");
        let img = |i: Option<usize>| i.and_then(|i| ids.images.get(i).copied()).unwrap_or(0);
        self.p(&mut s, "sima", img(m.stroke_image));
        self.p(&mut s, "ima", img(m.fill_image));
        self.f(&mut s, "stroke_rgba", &m.color);
        self.f(&mut s, "fill_rgba", &m.fill_color);
        self.f(&mut s, "mix_rgba", &m.mix_color);
        self.i(&mut s, "flag", bits(&[(m.hide, 1 << 1), (m.lock, 1 << 2), (m.show_stroke, 1 << 8), (m.show_fill, 1 << 9), (m.use_stroke_holdout, 1 << 13), (m.use_fill_holdout, 1 << 14)]));
        self.i(&mut s, "index", m.pass_index.min(32767) as i64);
        self.i(&mut s, "stroke_style", (m.stroke_style == StrokeStyle::Texture) as i64);
        self.i(&mut s, "fill_style", match m.fill_style {
            FillStyle::Solid => 0,
            FillStyle::Gradient => 1,
            FillStyle::Texture => 3,
        });
        self.f(&mut s, "mix_factor", &[m.mix_factor]);
        self.f(&mut s, "texture_angle", &[m.texture_angle]);
        self.f(&mut s, "texture_scale", &m.texture_scale);
        self.f(&mut s, "texture_offset", &m.texture_offset);
        self.i(&mut s, "mode", match m.mode {
            LineMode::Line => 0,
            LineMode::Dots => 1,
            LineMode::Squares => 2,
        });
        self.i(&mut s, "gradient_type", (m.gradient_type == GradientType::Radial) as i64);
        // BKE_gpencil_material_attr_init's values.
        self.f(&mut s, "gradient_radius", &[0.5]);
        self.f(&mut s, "gradient_scale", &[1.0, 1.0]);
        self.f(&mut s, "texture_opacity", &[1.0]);
        self.f(&mut s, "texture_pixsize", &[100.0]);
        self.put(b"DATA", st, &[s]);
    }

    /// A legacy `bGPdata` ID (`GD`) at `addr`, its materials at `mats`.
    pub(super) fn gpencil(&mut self, addr: u64, name: &str, g: &GreasePencil, mats: &[u64], ids: &Ids) {
        self.start_id();
        let mut r = self.id("bGPdata", "GD", name);
        let layers: Vec<u64> = g.layers.iter().map(|_| self.addr()).collect();
        self.listbase(&mut r, "layers", &layers);
        let o = &g.onion;
        self.i(&mut r, "flag", bits(&[(o.enabled, 1 << 9)]) | 1 << 1); // GP_DATA_DISPINFO
        self.i(&mut r, "curve_edit_resolution", 32);
        self.f(&mut r, "curve_edit_threshold", &[0.1]);
        self.f(&mut r, "curve_edit_corner_angle", &[std::f32::consts::FRAC_PI_2]);
        self.f(&mut r, "pixfactor", &[1.0]);
        self.f(&mut r, "line_color", &[0.6, 0.6, 0.6, 0.5]);
        self.f(&mut r, "onion_factor", &[o.opacity]);
        self.i(&mut r, "onion_mode", match o.mode {
            OnionMode::Frames => 0,
            OnionMode::Keyframes => 1,
            OnionMode::Selected => 2,
        });
        self.i(&mut r, "onion_flag", bits(&[(o.use_colors, 1 | 2), (o.use_fade, 1 << 3)]));
        self.i(&mut r, "gstep", o.before.min(32767) as i64);
        self.i(&mut r, "gstep_next", o.after.min(32767) as i64);
        self.f(&mut r, "gcolor_prev", &o.color_before);
        self.f(&mut r, "gcolor_next", &o.color_after);
        self.f(&mut r, "zdepth_offset", &[0.15]);
        self.i(&mut r, "draw_mode", (g.depth_order == DepthOrder::Location3D) as i64);
        self.i(&mut r, "totlayer", g.layers.len() as i64);
        self.f(&mut r, "grid.color", &[0.5, 0.5, 0.5]);
        self.f(&mut r, "grid.scale", &[1.0, 1.0]);
        self.i(&mut r, "grid.lines", 4);
        if !mats.is_empty() {
            let a = self.ptrs(mats);
            self.p(&mut r, "mat", a);
            self.i(&mut r, "totcol", mats.len() as i64);
        }
        let vgs: Vec<u64> = g.vertex_groups.iter().map(|_| self.addr()).collect();
        self.listbase(&mut r, "vertex_group_names", &vgs);
        if !vgs.is_empty() {
            self.i(&mut r, "vertex_group_active_index", g.active_vertex_group.min(vgs.len() - 1) as i64 + 1);
        }
        self.put(b"GD", addr, &[r]);
        let recs: Vec<Rec> = g
            .vertex_groups
            .iter()
            .map(|n| {
                let mut d = self.rec("bDeformGroup");
                self.s(&mut d, "name", n);
                d
            })
            .collect();
        self.chain(recs, &vgs);
        let active = g.active_layer.min(g.layers.len().saturating_sub(1));
        let recs: Vec<Rec> = g.layers.iter().enumerate().map(|(k, l)| self.gp_layer(l, k == active, ids)).collect();
        self.chain(recs, &layers);
    }

    fn gp_layer(&mut self, l: &GpLayer, active: bool, ids: &Ids) -> Rec {
        let mut r = self.rec("bGPDlayer");
        self.s(&mut r, "info", &l.name);
        self.i(&mut r, "flag", bits(&[(l.hide, 1), (l.lock, 2), (active, 4), (l.use_lights, 1 << 11), (l.use_masks, 1 << 13)]));
        self.i(&mut r, "onion_flag", l.use_onion_skinning as i64);
        self.f(&mut r, "opacity", &[l.opacity]);
        self.f(&mut r, "vertex_paint_opacity", &[1.0]);
        self.i(&mut r, "blend_mode", match l.blend {
            LayerBlend::Regular => 0,
            LayerBlend::HardLight => 1,
            LayerBlend::Add => 2,
            LayerBlend::Subtract => 3,
            LayerBlend::Multiply => 4,
            LayerBlend::Divide => 5,
        });
        let t = l.tint_color;
        self.f(&mut r, "tintcolor", &[t[0], t[1], t[2], l.tint_factor]);
        self.i(&mut r, "line_change", (l.radius_offset / LEGACY_RADIUS_FACTOR).round().clamp(-32768.0, 32767.0) as i64);
        self.i(&mut r, "thickness", 3);
        self.i(&mut r, "pass_index", l.pass_index.min(32767) as i64);
        self.f(&mut r, "location", &l.transform.location.to_array());
        self.f(&mut r, "rotation", &l.transform.rotation.normalize().to_euler(EulerOrder::XYZ));
        self.f(&mut r, "scale", &l.transform.scale.to_array());
        let m = l.transform.matrix();
        self.f(&mut r, "layer_mat", &m.to_cols_array());
        self.f(&mut r, "layer_invmat", &m.inverse().to_cols_array());
        self.f(&mut r, "inverse", &crate::math::Mat4::IDENTITY.to_cols_array());
        if let Some(p) = &l.parent {
            self.p(&mut r, "parent", ids.object(p));
            self.s(&mut r, "parsubstr", &l.parent_bone);
            self.i(&mut r, "partype", if l.parent_bone.is_empty() { 0 } else { 7 }); // PAROBJECT / PARBONE
        }
        let masks: Vec<u64> = l.masks.iter().map(|_| self.addr()).collect();
        self.listbase(&mut r, "mask_layers", &masks);
        let recs: Vec<Rec> = l
            .masks
            .iter()
            .enumerate()
            .map(|(k, n)| {
                let mut m = self.rec("bGPDlayer_Mask");
                self.s(&mut m, "name", n);
                self.i(&mut m, "sort_index", k as i64);
                m
            })
            .collect();
        self.chain(recs, &masks);
        let frames: Vec<u64> = l.frames.iter().map(|_| self.addr()).collect();
        self.listbase(&mut r, "frames", &frames);
        let recs: Vec<Rec> = l
            .frames
            .iter()
            .map(|fr| {
                let mut f = self.rec("bGPDframe");
                self.i(&mut f, "framenum", fr.frame as i64);
                self.i(&mut f, "key_type", key_type(fr.key_type));
                self.i(&mut f, "flag", bits(&[(fr.select, 1 << 1)]));
                let strokes: Vec<u64> = fr.drawing.strokes.iter().map(|_| self.addr()).collect();
                self.listbase(&mut f, "strokes", &strokes);
                let recs: Vec<Rec> = fr.drawing.strokes.iter().map(|s| self.gp_stroke(s)).collect();
                self.chain(recs, &strokes);
                f
            })
            .collect();
        self.chain(recs, &frames);
        r
    }

    fn gp_stroke(&mut self, s: &GpStroke) -> Rec {
        let mut r = self.rec("bGPDstroke");
        let widest = s.points.iter().map(|p| p.radius).fold(0.0f32, f32::max);
        let thickness = (widest / LEGACY_RADIUS_FACTOR).ceil().clamp(1.0, 32767.0);
        self.i(&mut r, "thickness", thickness as i64);
        self.i(&mut r, "totpoints", s.points.len() as i64);
        self.i(&mut r, "mat_nr", s.material as i64);
        // GP_STROKE_3DSPACE: object strokes live in 3D.
        self.i(&mut r, "flag", 1 | bits(&[(s.select, 1 << 3), (s.cyclic, 1 << 7)]));
        let cap = |c: CapType| (c == CapType::Flat) as i64;
        self.set(&mut r, "caps", super::write::V::I(&[cap(s.start_cap), cap(s.end_cap)]));
        self.f(&mut r, "gradient_f", &[1.0 - s.softness]);
        self.f(&mut r, "gradient_s", &[1.0, 1.0]);
        self.f(&mut r, "vert_color_fill", &s.fill_color);
        self.f(&mut r, "fill_opacity_fac", &[s.fill_opacity]);
        self.f(&mut r, "uv_rotation", &[s.uv_rotation]);
        self.f(&mut r, "uv_translation", &[s.uv_offset, 0.0]);
        self.f(&mut r, "uv_scale", &[s.uv_scale]);
        self.f(&mut r, "inittime", &[s.init_time]);
        let pts: Vec<Rec> = s
            .points
            .iter()
            .map(|p| {
                let mut q = self.rec("bGPDspoint");
                self.f(&mut q, "x", &[p.position.x]);
                self.f(&mut q, "y", &[p.position.y]);
                self.f(&mut q, "z", &[p.position.z]);
                self.f(&mut q, "pressure", &[p.radius / (thickness * LEGACY_RADIUS_FACTOR)]);
                self.f(&mut q, "strength", &[p.opacity]);
                self.f(&mut q, "vert_color", &p.vertex_color);
                self.f(&mut q, "uv_rot", &[p.rotation]);
                self.i(&mut q, "flag", p.select as i64);
                q
            })
            .collect();
        let pa = self.data(&pts);
        self.p(&mut r, "points", pa);
        if s.points.iter().any(|p| !p.weights.is_empty()) {
            let mut dvs = Vec::new();
            let mut dws = Vec::new();
            for p in &s.points {
                let mut d = self.rec("MDeformVert");
                if !p.weights.is_empty() {
                    let a = self.addr();
                    self.p(&mut d, "dw", a);
                    self.i(&mut d, "totweight", p.weights.len() as i64);
                    dws.push((a, p.weights.clone()));
                }
                dvs.push(d);
            }
            let da = self.data(&dvs);
            self.p(&mut r, "dvert", da);
            for (a, ws) in dws {
                let recs: Vec<Rec> = ws
                    .iter()
                    .map(|(g, w)| {
                        let mut x = self.rec("MDeformWeight");
                        self.i(&mut x, "def_nr", *g as i64);
                        self.f(&mut x, "weight", &[*w]);
                        x
                    })
                    .collect();
                self.put(b"DATA", a, &recs);
            }
        }
        r
    }
}
