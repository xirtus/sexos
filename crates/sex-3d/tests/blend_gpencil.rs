// SPDX-License-Identifier: GPL-3.0-only
//! `.blend` import of legacy Grease Pencil (`bGPdata`, 2.8 to 4.2) from
//! synthetic files, in both byte orders and pointer sizes.
mod blendgen;

use blendgen::{n, nv, p, s, Dna, Writer};
use sex_3d::grease_pencil::{CapType, FillStyle, KeyframeType, LayerBlend, LineMode};
use sex_3d::io::blend::import_blend;
use sex_3d::scene::{ObjectData, Scene};
use sex_3d::Vec3;

fn dna(ptr: usize) -> Dna {
    let mut d = Dna::new(ptr);
    d.def("ListBase", &[("void", "*first"), ("void", "*last")]);
    d.def("ID", &[("void", "*next"), ("void", "*prev"), ("char", "name[66]"), ("short", "flag"), ("int", "us")]);
    d.def(
        "MaterialGPencilStyle",
        &[
            ("Image", "*sima"),
            ("Image", "*ima"),
            ("float", "stroke_rgba[4]"),
            ("float", "fill_rgba[4]"),
            ("float", "mix_rgba[4]"),
            ("short", "flag"),
            ("short", "index"),
            ("short", "stroke_style"),
            ("short", "fill_style"),
            ("float", "mix_factor"),
            ("float", "gradient_angle"),
            ("float", "gradient_radius"),
            ("char", "_pad2[4]"),
            ("float", "gradient_scale[2]"),
            ("float", "gradient_shift[2]"),
            ("float", "texture_angle"),
            ("float", "texture_scale[2]"),
            ("float", "texture_offset[2]"),
            ("float", "texture_opacity"),
            ("float", "texture_pixsize"),
            ("int", "mode"),
            ("int", "gradient_type"),
            ("float", "mix_stroke_factor"),
            ("int", "alignment_mode"),
            ("float", "alignment_rotation"),
        ],
    );
    d.def("Material", &[("ID", "id"), ("float", "r"), ("float", "g"), ("float", "b"), ("float", "a"), ("MaterialGPencilStyle", "*gp_style")]);
    d.def("MDeformWeight", &[("int", "def_nr"), ("float", "weight")]);
    d.def("MDeformVert", &[("MDeformWeight", "*dw"), ("int", "totweight"), ("int", "flag")]);
    d.def("bDeformGroup", &[("bDeformGroup", "*next"), ("bDeformGroup", "*prev"), ("char", "name[64]"), ("char", "flag"), ("char", "_pad0[7]")]);
    d.def(
        "bGPDspoint",
        &[
            ("float", "x"),
            ("float", "y"),
            ("float", "z"),
            ("float", "pressure"),
            ("float", "strength"),
            ("float", "time"),
            ("int", "flag"),
            ("float", "uv_fac"),
            ("float", "uv_rot"),
            ("float", "uv_fill[2]"),
            ("float", "vert_color[4]"),
            ("char", "_pad2[4]"),
        ],
    );
    d.def(
        "bGPDstroke",
        &[
            ("bGPDstroke", "*next"),
            ("bGPDstroke", "*prev"),
            ("bGPDspoint", "*points"),
            ("void", "*triangles"),
            ("int", "totpoints"),
            ("int", "tot_triangles"),
            ("short", "thickness"),
            ("short", "flag"),
            ("short", "_flag"),
            ("char", "_pad[2]"),
            ("double", "inittime"),
            ("char", "colorname[128]"),
            ("int", "mat_nr"),
            ("short", "caps[2]"),
            ("float", "hardeness"),
            ("float", "aspect_ratio[2]"),
            ("float", "fill_opacity_fac"),
            ("float", "boundbox_min[3]"),
            ("float", "boundbox_max[3]"),
            ("float", "uv_rotation"),
            ("float", "uv_translation[2]"),
            ("float", "uv_scale"),
            ("int", "select_index"),
            ("char", "_pad4[4]"),
            ("MDeformVert", "*dvert"),
            ("void", "*_pad3"),
            ("float", "vert_color_fill[4]"),
        ],
    );
    d.def("bGPDframe", &[("bGPDframe", "*next"), ("bGPDframe", "*prev"), ("ListBase", "strokes"), ("int", "framenum"), ("short", "flag"), ("short", "key_type")]);
    d.def("bGPDlayer_Mask", &[("bGPDlayer_Mask", "*next"), ("bGPDlayer_Mask", "*prev"), ("char", "name[128]"), ("short", "sort_index"), ("short", "flag")]);
    d.def(
        "bGPDlayer",
        &[
            ("bGPDlayer", "*next"),
            ("bGPDlayer", "*prev"),
            ("ListBase", "frames"),
            ("void", "*actframe"),
            ("short", "flag"),
            ("short", "onion_flag"),
            ("float", "color[4]"),
            ("float", "fill[4]"),
            ("char", "info[128]"),
            ("short", "thickness"),
            ("short", "pass_index"),
            ("Object", "*parent"),
            ("float", "inverse[4][4]"),
            ("char", "parsubstr[64]"),
            ("short", "partype"),
            ("short", "line_change"),
            ("float", "tintcolor[4]"),
            ("float", "opacity"),
            ("char", "viewlayername[64]"),
            ("int", "blend_mode"),
            ("float", "vertex_paint_opacity"),
            ("short", "act_mask"),
            ("char", "_pad2[2]"),
            ("ListBase", "mask_layers"),
            ("float", "location[3]"),
            ("float", "rotation[3]"),
            ("float", "scale[3]"),
        ],
    );
    d.def(
        "bGPdata",
        &[
            ("ID", "id"),
            ("ListBase", "layers"),
            ("int", "flag"),
            ("short", "totlayer"),
            ("short", "totframe"),
            ("Material", "**mat"),
            ("short", "totcol"),
            ("short", "gstep"),
            ("short", "gstep_next"),
            ("char", "_pad1[2]"),
            ("float", "gcolor_prev[3]"),
            ("float", "gcolor_next[3]"),
            ("float", "onion_factor"),
            ("ListBase", "vertex_group_names"),
        ],
    );
    d.def(
        "Object",
        &[
            ("ID", "id"),
            ("short", "type"),
            ("short", "partype"),
            ("char", "_pad0[4]"),
            ("void", "*data"),
            ("Object", "*parent"),
            ("Material", "**mat"),
            ("char", "*matbits"),
            ("int", "totcol"),
            ("float", "loc[3]"),
            ("float", "scale[3]"),
            ("short", "rotmode"),
            ("char", "_pad1[2]"),
            ("ListBase", "defbase"),
        ],
    );
    d
}

#[test]
fn legacy_grease_pencil_imports() {
    for (ptr, big, large) in [(8, false, false), (4, true, false), (8, false, true)] {
        let ctx = format!("ptr {ptr} big {big} large {large}");
        let mut w = Writer::new(dna(ptr), big, large, if large { 500 } else { 402 });

        // Materials: a dotted ink line and a gradient fill.
        let (ma_ink, st_ink, ma_fill, st_fill) = (w.alloc(), w.alloc(), w.alloc(), w.alloc());
        let st = w.inst("MaterialGPencilStyle", &[("stroke_rgba", nv(&[0.1, 0.2, 0.3, 1.0])), ("flag", n((1 << 8) as f64)), ("mode", n(1.)), ("index", n(2.))]);
        w.put(st_ink, "DATA", "MaterialGPencilStyle", 1, st);
        let m = w.inst("Material", &[("id.name", s("MAInk")), ("gp_style", p(st_ink))]);
        w.put(ma_ink, "MA", "Material", 1, m);
        let st = w.inst(
            "MaterialGPencilStyle",
            &[("fill_rgba", nv(&[1.0, 0.5, 0.0, 1.0])), ("mix_rgba", nv(&[0.0, 0.0, 1.0, 1.0])), ("flag", n((1 << 9 | 1 << 11) as f64)), ("fill_style", n(1.)), ("gradient_type", n(1.)), ("mix_factor", n(0.25))],
        );
        w.put(st_fill, "DATA", "MaterialGPencilStyle", 1, st);
        let m = w.inst("Material", &[("id.name", s("MAFill")), ("gp_style", p(st_fill))]);
        w.put(ma_fill, "MA", "Material", 1, m);

        // One stroke of three points with weights, cyclic, flat end cap.
        let pts: Vec<u8> = [[0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 0.5], [1.0, 0.0, 1.0, 1.0]]
            .iter()
            .enumerate()
            .flat_map(|(i, q)| w.inst("bGPDspoint", &[("x", n(q[0])), ("y", n(q[1])), ("z", n(q[2])), ("pressure", n(q[3])), ("strength", n(0.75)), ("vert_color", nv(&[1., 0., 0., if i == 1 { 0.5 } else { 0.0 }])), ("flag", n(if i == 2 { 1. } else { 0. }))]))
            .collect();
        let points = w.alloc();
        w.put(points, "DATA", "bGPDspoint", 3, pts);
        let dw = w.alloc();
        let b = w.inst("MDeformWeight", &[("def_nr", n(0.)), ("weight", n(0.5))]);
        w.put(dw, "DATA", "MDeformWeight", 1, b);
        let dv: Vec<u8> = [w.inst("MDeformVert", &[("dw", p(dw)), ("totweight", n(1.))]), w.inst::<&str>("MDeformVert", &[]), w.inst::<&str>("MDeformVert", &[])].concat();
        let dvert = w.alloc();
        w.put(dvert, "DATA", "MDeformVert", 3, dv);
        let (st0, st1) = (w.alloc(), w.alloc());
        let b = w.inst(
            "bGPDstroke",
            &[
                ("next", p(st1)),
                ("points", p(points)),
                ("totpoints", n(3.)),
                ("thickness", n(40.)),
                ("flag", n((1 << 7) as f64)),
                ("caps", nv(&[0., 1.])),
                ("hardeness", n(0.75)),
                ("fill_opacity_fac", n(1.)),
                ("uv_scale", n(1.)),
                ("dvert", p(dvert)),
            ],
        );
        w.put(st0, "DATA", "bGPDstroke", 1, b);
        // A fill stroke using the second material.
        let fill_pts: Vec<u8> = [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]].iter().flat_map(|q| w.inst("bGPDspoint", &[("x", n(q[0])), ("z", n(q[1])), ("pressure", n(1.)), ("strength", n(1.))])).collect();
        let fp = w.alloc();
        w.put(fp, "DATA", "bGPDspoint", 3, fill_pts);
        let b = w.inst("bGPDstroke", &[("prev", p(st0)), ("points", p(fp)), ("totpoints", n(3.)), ("thickness", n(10.)), ("mat_nr", n(1.)), ("hardeness", n(1.)), ("vert_color_fill", nv(&[0., 1., 0., 0.5])), ("fill_opacity_fac", n(1.)), ("uv_scale", n(1.))]);
        w.put(st1, "DATA", "bGPDstroke", 1, b);

        // Frames 1 (keyframe) and 5 (breakdown, empty) on layer "Ink";
        // layer "Paper" hidden, multiplied, masked by "Ink".
        let (f1, f5) = (w.alloc(), w.alloc());
        let b = w.inst("bGPDframe", &[("next", p(f5)), ("strokes.first", p(st0)), ("strokes.last", p(st1)), ("framenum", n(1.))]);
        w.put(f1, "DATA", "bGPDframe", 1, b);
        let b = w.inst("bGPDframe", &[("prev", p(f1)), ("framenum", n(5.)), ("key_type", n(2.)), ("flag", n(2.))]);
        w.put(f5, "DATA", "bGPDframe", 1, b);
        let (l_ink, l_paper, mask) = (w.alloc(), w.alloc(), w.alloc());
        let b = w.inst("bGPDlayer_Mask", &[("name", s("Ink"))]);
        w.put(mask, "DATA", "bGPDlayer_Mask", 1, b);
        let b = w.inst(
            "bGPDlayer",
            &[
                ("next", p(l_paper)),
                ("frames.first", p(f1)),
                ("frames.last", p(f5)),
                ("info", s("Ink")),
                ("opacity", n(0.8)),
                ("flag", n((1 << 2) as f64)),
                ("onion_flag", n(1.)),
                ("line_change", n(20.)),
                ("tintcolor", nv(&[1., 0., 0., 0.25])),
                ("location", nv(&[0., 0., 1.])),
                ("scale", nv(&[1., 1., 1.])),
            ],
        );
        w.put(l_ink, "DATA", "bGPDlayer", 1, b);
        let b = w.inst(
            "bGPDlayer",
            &[
                ("prev", p(l_ink)),
                ("info", s("Paper")),
                ("opacity", n(1.)),
                ("flag", n((1 | 2 | 1 << 13) as f64)),
                ("blend_mode", n(4.)),
                ("mask_layers.first", p(mask)),
                ("mask_layers.last", p(mask)),
                ("scale", nv(&[1., 1., 1.])),
            ],
        );
        w.put(l_paper, "DATA", "bGPDlayer", 1, b);
        let vg = w.alloc();
        let b = w.inst("bDeformGroup", &[("name", s("Hand"))]);
        w.put(vg, "DATA", "bDeformGroup", 1, b);
        let mats = w.raw(w.ptrs(&[ma_ink, ma_fill]), 1);
        let gd = w.alloc();
        let b = w.inst(
            "bGPdata",
            &[
                ("id.name", s("GDDrawing")),
                ("layers.first", p(l_ink)),
                ("layers.last", p(l_paper)),
                ("mat", p(mats)),
                ("totcol", n(2.)),
                ("gstep", n(2.)),
                ("gstep_next", n(3.)),
                ("onion_factor", n(0.4)),
                ("vertex_group_names.first", p(vg)),
                ("vertex_group_names.last", p(vg)),
            ],
        );
        w.put(gd, "GD", "bGPdata", 1, b);
        let o = w.inst("Object", &[("id.name", s("OBSketch")), ("type", n(26.)), ("data", p(gd)), ("loc", nv(&[0., 2., 0.])), ("scale", nv(&[1., 1., 1.])), ("rotmode", n(1.))]);
        let ob = w.alloc();
        w.put(ob, "OB", "Object", 1, o);

        let scene = import_blend(&w.finish()).unwrap_or_else(|e| panic!("{ctx}: {e}"));
        let oi = scene.find_object("Sketch").unwrap_or_else(|| panic!("{ctx}: no Sketch"));
        let ObjectData::GreasePencil(gi) = scene.objects[oi].data else { panic!("{ctx}: {:?}", scene.objects[oi].data) };
        assert_eq!(scene.grease_pencil_names[gi], "Drawing");
        assert_eq!(scene.objects[oi].transform.location, Vec3::new(0.0, 2.0, 0.0));
        let g = &scene.grease_pencils[gi];
        assert_eq!(g.layers.len(), 2, "{ctx}");
        assert_eq!(g.active_layer, 0);
        assert_eq!(g.vertex_groups, vec!["Hand".to_string()]);
        assert_eq!((g.onion.before, g.onion.after), (2, 3));
        assert!((g.onion.opacity - 0.4).abs() < 1e-6);

        // Materials.
        assert_eq!(g.materials.len(), 2);
        let (ink, fill) = (&g.materials[0], &g.materials[1]);
        assert_eq!(ink.name, "Ink");
        assert!(ink.show_stroke && !ink.show_fill);
        assert_eq!((ink.mode, ink.color, ink.pass_index), (LineMode::Dots, [0.1, 0.2, 0.3, 1.0], 2));
        assert!(fill.show_fill && !fill.show_stroke && fill.use_fill_holdout);
        assert_eq!((fill.fill_style, fill.mix_factor), (FillStyle::Gradient, 0.25));
        assert_eq!(fill.gradient_type, sex_3d::grease_pencil::GradientType::Radial);

        // Layers.
        let ink_l = &g.layers[0];
        assert_eq!(ink_l.name, "Ink");
        assert!((ink_l.opacity - 0.8).abs() < 1e-6 && ink_l.use_onion_skinning && !ink_l.hide);
        assert!((ink_l.radius_offset - 0.01).abs() < 1e-6);
        assert_eq!((ink_l.tint_color, ink_l.tint_factor), ([1.0, 0.0, 0.0], 0.25));
        assert_eq!(ink_l.transform.location, Vec3::new(0.0, 0.0, 1.0));
        let paper = &g.layers[1];
        assert!(paper.hide && paper.lock && paper.use_masks);
        assert_eq!(paper.blend, LayerBlend::Multiply);
        assert_eq!(paper.masks, vec!["Ink".to_string()]);

        // Frames and strokes.
        assert_eq!(ink_l.frames.iter().map(|f| (f.frame, f.key_type, f.select)).collect::<Vec<_>>(), vec![(1, KeyframeType::Keyframe, false), (5, KeyframeType::Breakdown, true)]);
        let strokes = &ink_l.frames[0].drawing.strokes;
        assert_eq!(strokes.len(), 2);
        let s0 = &strokes[0];
        assert!(s0.cyclic);
        assert_eq!((s0.start_cap, s0.end_cap), (CapType::Round, CapType::Flat));
        assert!((s0.softness - 0.25).abs() < 1e-6);
        assert_eq!(s0.points.len(), 3);
        assert!((s0.points[0].radius - 0.02).abs() < 1e-6, "40 px * 1.0 / 2000");
        assert!((s0.points[1].radius - 0.01).abs() < 1e-6);
        assert_eq!(s0.points[2].position, Vec3::new(1.0, 0.0, 1.0));
        assert_eq!(s0.points[1].opacity, 0.75);
        assert_eq!(s0.points[1].vertex_color, [1.0, 0.0, 0.0, 0.5]);
        assert!(s0.points[2].select && !s0.points[0].select);
        assert_eq!(s0.points[0].weight(0), 0.5);
        assert_eq!(s0.points[1].weight(0), 0.0);
        let s1 = &strokes[1];
        assert_eq!((s1.material, s1.fill_color), (1, [0.0, 1.0, 0.0, 0.5]));
        assert!(ink_l.frames[1].drawing.strokes.is_empty());

        // It draws, and survives the polyops round trip.
        let text = sex_3d::script::scene_to_polyops(&scene);
        let mut back = Scene::new();
        sex_3d::script::run(&mut back, &text).unwrap_or_else(|e| panic!("{ctx}: {e}\n{text}"));
        assert_eq!(back.grease_pencils, scene.grease_pencils, "{ctx}");
    }
}
