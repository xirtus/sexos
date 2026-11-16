// SPDX-License-Identifier: GPL-3.0-only
use super::*;
use crate::nodes::{new_node, Prop, TreeKind};

fn ctx(scene: &Scene) -> Context<'_> {
    let mut c = Context::new(scene, None, (64, 48));
    c.threads = 2;
    c
}

fn spot(w: usize, h: usize, x0: usize, y0: usize, s: usize, v: f32) -> Buf {
    let mut b = Buf::new(w, h, [0.0, 0.0, 0.0, 1.0]);
    for y in y0..y0 + s {
        for x in x0..x0 + s {
            b.px[y * w + x] = [v, v, v, 1.0];
        }
    }
    b
}

fn run(n: &Node, ins: &[Val]) -> Vec<Val> {
    let scene = Scene::default();
    ops::eval_node(&ctx(&scene), n, ins)
}

fn sum(b: &Buf) -> f64 {
    b.px.iter().map(|p| p[0] as f64).sum()
}

#[test]
fn blur_preserves_energy_for_every_filter() {
    let img = spot(64, 64, 28, 28, 6, 1.0);
    for kind in ["FLAT", "TENT", "QUAD", "CUBIC", "GAUSS", "FAST_GAUSS", "CATROM", "MITCH"] {
        let mut n = new_node("CompositorNodeBlur").unwrap();
        n.set_prop("filter_type", Prop::Enum(kind.into()));
        n.set_prop("size_x", Prop::Int(7));
        n.set_prop("size_y", Prop::Int(4));
        let out = run(&n, &[Val::img(img.clone()), Val::float(1.0)]);
        let b = out[0].image().unwrap();
        assert_eq!((b.w, b.h), (64, 64));
        assert!((sum(b) - sum(&img)).abs() < 1e-2, "{kind}: {} vs {}", sum(b), sum(&img));
        // It actually spread: the centre dropped, a neighbour gained.
        assert!(b.px[31 * 64 + 31][0] < 1.0 || matches!(kind, "CATROM" | "MITCH"));
        assert!(b.px[31 * 64 + 26][0] > 0.0, "{kind}");
    }
    // Relative size and extended bounds grow the canvas by the radius.
    let mut n = new_node("CompositorNodeBlur").unwrap();
    n.set_prop("use_relative", Prop::Bool(true));
    n.set_prop("factor_x", Prop::Float(10.0));
    n.set_prop("factor_y", Prop::Float(10.0));
    n.set_prop("use_extended_bounds", Prop::Bool(true));
    let out = run(&n, &[Val::img(img.clone()), Val::float(1.0)]);
    let b = out[0].image().unwrap();
    assert_eq!((b.w, b.h), (64 + 2 * 7, 64 + 2 * 7));
    assert!((sum(b) - sum(&img)).abs() < 1e-2);
}

#[test]
fn glare_bloom_spreads_only_above_threshold() {
    let bright_only = spot(64, 64, 40, 40, 4, 8.0);
    let mut img = bright_only.clone();
    for y in 10..14 {
        for x in 10..14 {
            img.px[y * 64 + x] = [0.8, 0.8, 0.8, 1.0];
        }
    }
    let mut n = new_node("CompositorNodeGlare").unwrap();
    n.set_prop("glare_type", Prop::Enum("BLOOM".into()));
    n.set_prop("quality", Prop::Enum("HIGH".into()));
    let with_dim = run(&n, &[Val::img(img.clone())]);
    let without = run(&n, &[Val::img(bright_only.clone())]);
    let (a, b) = (with_dim[0].image().unwrap(), without[0].image().unwrap());
    // The glare added is the same whether or not the below-threshold spot
    // is there: only highlights above the threshold spread.
    for i in 0..64 * 64 {
        let added_a = a.px[i][0] - img.px[i][0];
        let added_b = b.px[i][0] - bright_only.px[i][0];
        assert!((added_a - added_b).abs() < 1e-4, "{i}");
    }
    // Around the bright spot, light was added.
    assert!(b.px[42 * 64 + 48][0] > 1e-3, "{}", b.px[42 * 64 + 48][0]);
    assert!(b.px[42 * 64 + 42][0] > 8.0);
    // An image entirely below the threshold passes through unchanged.
    let dim = spot(32, 32, 8, 8, 8, 0.9);
    for kind in ["BLOOM", "FOG_GLOW", "STREAKS", "GHOSTS", "SIMPLE_STAR"] {
        let mut n = new_node("CompositorNodeGlare").unwrap();
        n.set_prop("glare_type", Prop::Enum(kind.into()));
        n.set_prop("size", Prop::Int(6));
        let out = run(&n, &[Val::img(dim.clone())]);
        let b = out[0].image().unwrap();
        for (p, q) in b.px.iter().zip(&dim.px) {
            assert!((p[0] - q[0]).abs() < 1e-4, "{kind}");
        }
    }
    // Every glare type spreads a bright spot.
    let bright = spot(64, 64, 30, 30, 4, 20.0);
    for kind in ["FOG_GLOW", "STREAKS", "GHOSTS", "SIMPLE_STAR"] {
        let mut n = new_node("CompositorNodeGlare").unwrap();
        n.set_prop("glare_type", Prop::Enum(kind.into()));
        n.set_prop("size", Prop::Int(6));
        n.set_prop("use_rotate_45", Prop::Bool(false));
        let out = run(&n, &[Val::img(bright.clone())]);
        let b = out[0].image().unwrap();
        assert!(sum(b) > sum(&bright) * 1.01, "{kind}: {} {}", sum(b), sum(&bright));
    }
}

#[test]
fn fft_round_trips() {
    assert!(glare::fft_roundtrip_error(256) < 1e-4);
}

#[test]
fn alpha_over_premultiplied_math() {
    // Premultiplied over: out = (1 - fac * a_over) * bg + fac * over.
    let bg = [0.2, 0.4, 0.6, 1.0];
    let over = [0.25, 0.0, 0.0, 0.5];
    let r = ops::alpha_over(1.0, bg, over, 0.0, false);
    let want = [0.5 * 0.2 + 0.25, 0.5 * 0.4, 0.5 * 0.6, 0.5 + 0.5];
    for k in 0..4 {
        assert!((r[k] - want[k]).abs() < 1e-6, "{r:?}");
    }
    // Factor scales the over colour.
    let r = ops::alpha_over(0.5, bg, over, 0.0, false);
    assert!((r[0] - (0.75 * 0.2 + 0.125)).abs() < 1e-6);
    // "Convert Premultiplied" treats the over colour as straight (key).
    let r = ops::alpha_over(1.0, bg, [1.0, 0.0, 0.0, 0.5], 0.0, true);
    assert!((r[0] - 0.6).abs() < 1e-6 && (r[1] - 0.2).abs() < 1e-6);
    // Opaque over replaces; transparent over keeps the background.
    assert_eq!(ops::alpha_over(1.0, bg, [1.0, 1.0, 1.0, 1.0], 0.0, false), [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(ops::alpha_over(1.0, bg, [0.0; 4], 0.0, true), bg);
    // Through the node with images.
    let n = new_node("CompositorNodeAlphaOver").unwrap();
    let out = run(&n, &[Val::float(1.0), Val::img(Buf::new(4, 4, bg)), Val::Single(over)]);
    let p = out[0].image().unwrap().px[5];
    assert!((p[0] - want[0]).abs() < 1e-6);
}

#[test]
fn keying_nodes_make_mattes() {
    let green = [0.1, 0.9, 0.1, 1.0];
    let red = [0.9, 0.1, 0.1, 1.0];
    let img = Val::img(Buf { w: 2, h: 1, px: vec![green, red] });
    // Chroma key on green: green keyed out, red kept.
    let n = new_node("CompositorNodeChromaMatte").unwrap();
    let out = run(&n, &[img.clone(), Val::Single(green)]);
    let m = out[1].image().unwrap();
    assert!(m.px[0][0] < 0.01 && m.px[1][0] > 0.99, "{:?}", m.px);
    assert!(out[0].image().unwrap().px[0][3] < 0.01);
    // Distance and difference keys.
    for id in ["CompositorNodeDistanceMatte", "CompositorNodeDiffMatte", "CompositorNodeColorMatte"] {
        let n = new_node(id).unwrap();
        let out = run(&n, &[img.clone(), Val::Single(green)]);
        let m = out[1].image().unwrap();
        assert!(m.px[0][0] < 0.01 && m.px[1][0] > 0.99, "{id} {:?}", m.px);
    }
    // Luminance key ramp between the limits.
    let mut n = new_node("CompositorNodeLumaMatte").unwrap();
    n.set_prop("limit_min", Prop::Float(0.2));
    n.set_prop("limit_max", Prop::Float(0.6));
    let out = run(&n, &[Val::Single([0.4, 0.4, 0.4, 1.0])]);
    assert!((out[1].single().unwrap()[0] - 0.5).abs() < 1e-4);
    // Channel key (green limited by max of red/blue).
    let n = new_node("CompositorNodeChannelMatte").unwrap();
    let out = run(&n, &[img.clone()]);
    let m = out[1].image().unwrap();
    assert!(m.px[0][0] < 0.25 && m.px[1][0] > 0.99, "{:?}", m.px);
    // Keying node: screen colour keyed out.
    let n = new_node("CompositorNodeKeying").unwrap();
    let out = run(&n, &[img, Val::Single(green), Val::float(0.0), Val::float(0.0)]);
    let m = out[1].image().unwrap();
    assert!(m.px[0][0] < 0.05 && m.px[1][0] > 0.95, "{:?}", m.px);
    // Box mask inside/outside.
    let n = new_node("CompositorNodeBoxMask").unwrap();
    let out = run(&n, &[Val::img(Buf::new(20, 20, [0.0, 0.0, 0.0, 1.0])), Val::float(1.0)]);
    let m = out[0].image().unwrap();
    assert_eq!(m.px[10 * 20 + 10][0], 1.0);
    assert_eq!(m.px[0][0], 0.0);
}

#[test]
fn color_balance_and_correction_identity() {
    let c = [0.3, 0.55, 0.8, 0.9];
    for method in ["LIFT_GAMMA_GAIN", "OFFSET_POWER_SLOPE"] {
        let mut n = new_node("CompositorNodeColorBalance").unwrap();
        n.set_prop("correction_method", Prop::Enum(method.into()));
        let out = run(&n, &[Val::float(1.0), Val::Single(c)]);
        let r = out[0].single().unwrap();
        for k in 0..4 {
            assert!((r[k] - c[k]).abs() < 1e-4, "{method} {r:?}");
        }
    }
    let n = new_node("CompositorNodeColorCorrection").unwrap();
    let r = run(&n, &[Val::Single(c), Val::float(1.0)])[0].single().unwrap();
    for k in 0..4 {
        assert!((r[k] - c[k]).abs() < 1e-4, "{r:?}");
    }
    for id in ["CompositorNodeHueSat", "CompositorNodeHueCorrect", "CompositorNodeCurveRGB", "CompositorNodeGamma", "CompositorNodeExposure", "CompositorNodeBrightContrast"] {
        let n = new_node(id).unwrap();
        let ins: Vec<Val> = n.inputs.iter().map(|s| Val::Single(socket_px(&s.value, s.ty))).collect();
        let k = n.inputs.iter().position(|s| s.name == "Image").unwrap();
        let mut ins = ins;
        ins[k] = Val::Single(c);
        let r = run(&n, &ins)[0].single().unwrap();
        for ch in 0..3 {
            assert!((r[ch] - c[ch]).abs() < 2e-3, "{id} {r:?}");
        }
    }
    // ASC CDL slope doubles.
    let mut n = new_node("CompositorNodeColorBalance").unwrap();
    n.set_prop("correction_method", Prop::Enum("OFFSET_POWER_SLOPE".into()));
    n.set_prop("slope", Prop::Color([2.0, 2.0, 2.0, 1.0]));
    let r = run(&n, &[Val::float(1.0), Val::Single(c)])[0].single().unwrap();
    assert!((r[0] - 0.6).abs() < 1e-5);
}

#[test]
fn mix_matches_shader_mix_for_every_blend_mode() {
    use crate::render::shader::color::{blend, Blend};
    let a = [0.2, 0.5, 0.9, 1.0];
    let b = [0.7, 0.3, 0.1, 0.5];
    for (id, _) in crate::nodes::enums::items("CompositorNodeMixRGB", "blend_type") {
        let mut n = new_node("CompositorNodeMixRGB").unwrap();
        n.set_prop("blend_type", Prop::Enum(id.to_string()));
        let r = run(&n, &[Val::float(0.6), Val::Single(a), Val::Single(b)])[0].single().unwrap();
        let want = blend(Blend::from_name(id), 0.6, [a[0], a[1], a[2]], [b[0], b[1], b[2]]);
        for k in 0..3 {
            assert!((r[k] - want[k]).abs() < 1e-6, "{id}");
        }
        assert_eq!(r[3], a[3]);
    }
    // Use Alpha scales the factor by the second image's alpha.
    let mut n = new_node("CompositorNodeMixRGB").unwrap();
    n.set_prop("use_alpha", Prop::Bool(true));
    let r = run(&n, &[Val::float(1.0), Val::Single(a), Val::Single(b)])[0].single().unwrap();
    assert!((r[0] - (0.2 + (0.7 - 0.2) * 0.5)).abs() < 1e-6);
}

#[test]
fn transform_nodes_sizes() {
    let img = Val::img(spot(40, 20, 5, 5, 4, 1.0));
    let mut n = new_node("CompositorNodeScale").unwrap();
    let out = run(&n, &[img.clone(), Val::float(0.5), Val::float(2.0)]);
    assert_eq!(out[0].size(), Some((20, 40)));
    n.set_prop("space", Prop::Enum("ABSOLUTE".into()));
    let out = run(&n, &[img.clone(), Val::float(17.0), Val::float(9.0)]);
    assert_eq!(out[0].size(), Some((17, 9)));
    n.set_prop("space", Prop::Enum("RENDER_SIZE".into()));
    n.set_prop("frame_method", Prop::Enum("FIT".into()));
    let out = run(&n, &[img.clone(), Val::float(1.0), Val::float(1.0)]);
    assert_eq!(out[0].size(), Some((64, 48)));
    // Transform/Rotate/Translate keep the canvas.
    for id in ["CompositorNodeTransform", "CompositorNodeRotate", "CompositorNodeTranslate", "CompositorNodeFlip", "CompositorNodeLensdist"] {
        let n = new_node(id).unwrap();
        let mut ins: Vec<Val> = n.inputs.iter().map(|s| Val::Single(socket_px(&s.value, s.ty))).collect();
        ins[0] = img.clone();
        if ins.len() > 1 {
            ins[1] = Val::float(0.3);
        }
        assert_eq!(run(&n, &ins)[0].size(), Some((40, 20)), "{id}");
    }
    // Crop with "Crop Image Size" makes the rectangle the canvas.
    let mut n = new_node("CompositorNodeCrop").unwrap();
    n.set_prop("use_crop_size", Prop::Bool(true));
    n.set_prop("min_x", Prop::Int(2));
    n.set_prop("max_x", Prop::Int(11));
    n.set_prop("min_y", Prop::Int(0));
    n.set_prop("max_y", Prop::Int(4));
    assert_eq!(run(&n, &[img.clone()])[0].size(), Some((10, 5)));
    n.set_prop("use_crop_size", Prop::Bool(false));
    let out = run(&n, &[img.clone()]);
    let b = out[0].image().unwrap();
    assert_eq!((b.w, b.h), (40, 20));
    assert_eq!(b.px[0][3], 0.0);
    // Translate by whole pixels moves the spot (Blender's y up).
    let n = new_node("CompositorNodeTranslate").unwrap();
    let out = run(&n, &[img.clone(), Val::float(3.0), Val::float(2.0)]);
    let b = out[0].image().unwrap();
    assert_eq!(b.px[(5 - 2) * 40 + 5 + 3][0], 1.0);
    // Flip X mirrors.
    let out = run(&new_node("CompositorNodeFlip").unwrap(), &[img.clone()]);
    assert_eq!(out[0].image().unwrap().px[5 * 40 + 34][0], 1.0);
}

#[test]
fn tree_evaluation_groups_mute_and_outputs() {
    let mut scene = Scene::default();
    // Group: Image * 2 (Mix Multiply with constant).
    let mut g = NodeTree::new("Double", TreeKind::Compositor);
    g.inputs.push(crate::nodes::InterfaceSocket { name: "Image".into(), ty: SocketType::Color, default: Value::Color([0.0; 4]), min: 0.0, max: 1.0 });
    g.outputs.push(crate::nodes::InterfaceSocket { name: "Image".into(), ty: SocketType::Color, default: Value::Color([0.0; 4]), min: 0.0, max: 1.0 });
    let gi = g.add("NodeGroupInput", [0.0, 0.0]).unwrap();
    let go = g.add("NodeGroupOutput", [400.0, 0.0]).unwrap();
    let gc = g.clone();
    crate::nodes::sync_group_sockets(&mut g.nodes[gi], &gc);
    crate::nodes::sync_group_sockets(&mut g.nodes[go], &gc);
    let ex = g.add("CompositorNodeExposure", [200.0, 0.0]).unwrap();
    g.nodes[ex].set_input("Exposure", Value::Float(1.0));
    g.link(gi, "Image", ex, "Image").unwrap();
    g.link(ex, "Image", go, "Image").unwrap();
    scene.node_groups.push(g);

    let mut t = NodeTree::new("Compositing", TreeKind::Compositor);
    let rgb = t.add("CompositorNodeRGB", [0.0, 0.0]).unwrap();
    t.nodes[rgb].outputs[0].value = Value::Color([0.25, 0.1, 0.05, 1.0]);
    let mut gn = new_node("CompositorNodeGroup").unwrap();
    gn.set_prop("node_tree", Prop::Id(Some(0)));
    crate::nodes::sync_group_sockets(&mut gn, &scene.node_groups[0]);
    t.nodes.push(gn);
    let grp = t.nodes.len() - 1;
    let inv = t.add("CompositorNodeInvert", [200.0, 0.0]).unwrap();
    let comp = t.add("CompositorNodeComposite", [400.0, 0.0]).unwrap();
    let view = t.add("CompositorNodeViewer", [400.0, -200.0]).unwrap();
    t.link_sockets(rgb, 0, grp, 0).unwrap();
    t.link(grp, "Image", inv, "Color").unwrap();
    t.link(inv, "Color", comp, "Image").unwrap();
    t.link(grp, "Image", view, "Image").unwrap();
    t.nodes[inv].mute = true;
    let out = composite(&t, &ctx(&scene));
    let c = out.composite.unwrap();
    assert_eq!((c.width, c.height), (64, 48));
    let p = c.pixels[100];
    assert!((p[0] - 0.5).abs() < 1e-6 && (p[1] - 0.2).abs() < 1e-6, "{p:?}");
    let v = out.viewer.unwrap();
    assert_eq!(v.name, "Viewer Node");
    // Unmuted invert.
    t.nodes[inv].mute = false;
    let c = composite(&t, &ctx(&scene)).composite.unwrap();
    assert!((c.pixels[0][0] - 0.5).abs() < 1e-6 && (c.pixels[0][1] - 0.8).abs() < 1e-6);
}

#[test]
fn utility_nodes() {
    let mut n = new_node("CompositorNodeMath").unwrap();
    n.set_prop("operation", Prop::Enum("MULTIPLY".into()));
    assert_eq!(run(&n, &[Val::float(3.0), Val::float(4.0), Val::float(0.0)])[0].single().unwrap()[0], 12.0);
    let n = new_node("CompositorNodeMapRange").unwrap();
    let r = run(&n, &[Val::float(0.25), Val::float(0.0), Val::float(0.5), Val::float(10.0), Val::float(20.0)]);
    assert!((r[0].single().unwrap()[0] - 15.0).abs() < 1e-5);
    // Separate/Combine round trip in every mode.
    for mode in ["RGB", "HSV", "HSL", "YCC", "YUV"] {
        let mut s = new_node("CompositorNodeSeparateColor").unwrap();
        s.set_prop("mode", Prop::Enum(mode.into()));
        let mut c = new_node("CompositorNodeCombineColor").unwrap();
        c.set_prop("mode", Prop::Enum(mode.into()));
        let col = [0.3, 0.6, 0.2, 0.7];
        let parts = run(&s, &[Val::Single(col)]);
        let back = run(&c, &parts)[0].single().unwrap();
        for k in 0..4 {
            assert!((back[k] - col[k]).abs() < 1e-3, "{mode} {back:?}");
        }
    }
    let img = Buf { w: 4, h: 1, px: vec![[0.0; 4], [2.0, 2.0, 2.0, 1.0], [4.0, 4.0, 4.0, 1.0], [6.0, 6.0, 6.0, 1.0]] };
    let r = run(&new_node("CompositorNodeNormalize").unwrap(), &[Val::img(img.clone())]);
    assert!((r[0].image().unwrap().px[2][0] - 2.0 / 3.0).abs() < 1e-6);
    let r = run(&new_node("CompositorNodeLevels").unwrap(), &[Val::img(img)]);
    assert!((r[0].single().unwrap()[0] - 3.0).abs() < 0.01);
    // Dilate grows a spot, erode removes it.
    let mut d = new_node("CompositorNodeDilateErode").unwrap();
    d.set_prop("distance", Prop::Int(2));
    let s = Val::img(spot(16, 16, 7, 7, 2, 1.0));
    let r = run(&d, &[s.clone()]);
    assert_eq!(r[0].image().unwrap().px[5 * 16 + 5][0], 1.0);
    d.set_prop("distance", Prop::Int(-1));
    let r = run(&d, &[s]);
    assert_eq!(r[0].image().unwrap().px[7 * 16 + 7][0], 0.0);
}

