// SPDX-License-Identifier: GPL-3.0-only
//! Render -> composite end to end: Render Layers with View Layer passes
//! through compositor nodes into the Composite and Viewer outputs.
use sex_3d::compositor::{composite, Context};
use sex_3d::nodes::comp_registry::set_render_passes;
use sex_3d::nodes::{NodeTree, Prop, TreeKind, Value};
use sex_3d::render::{render, Pass, Passes, RenderSettings};
use sex_3d::scene::Scene;

#[test]
fn render_then_composite_with_passes() {
    let scene = Scene::startup();
    let settings = RenderSettings { width: 48, height: 32, samples: 4, denoise: false, threads: 2, passes: Passes::NONE.with(Pass::Depth).with(Pass::Normal), ..Default::default() };
    let r = render(&scene, &settings);
    assert!(r.passes.contains_key(&Pass::Depth));

    let mut t = NodeTree::new("Compositing Nodetree", TreeKind::Compositor);
    let rl = t.add("CompositorNodeRLayers", [0.0, 0.0]).unwrap();
    set_render_passes(&mut t.nodes[rl], settings.passes);
    assert!(t.nodes[rl].outputs.iter().find(|s| s.name == "Depth").unwrap().enabled);
    assert!(!t.nodes[rl].outputs.iter().find(|s| s.name == "Mist").unwrap().enabled);
    let bc = t.add("CompositorNodeBrightContrast", [200.0, 0.0]).unwrap();
    t.nodes[bc].set_input("Bright", Value::Float(10.0));
    let co = t.add("CompositorNodeComposite", [400.0, 0.0]).unwrap();
    let norm = t.add("CompositorNodeNormalize", [200.0, -200.0]).unwrap();
    let vw = t.add("CompositorNodeViewer", [400.0, -200.0]).unwrap();
    t.link(rl, "Image", bc, "Image").unwrap();
    t.link(bc, "Image", co, "Image").unwrap();
    t.link(rl, "Depth", norm, "Value").unwrap();
    t.link(norm, "Value", vw, "Image").unwrap();

    let mut scene2 = scene.clone();
    scene2.compositor = Some(t.clone());
    let ctx = Context::new(&scene2, Some(&r), (0, 0));
    let out = composite(&t, &ctx);
    let c = out.composite.expect("composite output");
    assert_eq!((c.width, c.height), (48, 32));
    // Brightness +10 adds 0.1 to every channel (no contrast change).
    for (a, b) in c.pixels.iter().zip(&r.color.pixels) {
        let want = if b[3] > 0.0 { (b[0] / b[3] + 0.1) * b[3] } else { b[0] + 0.1 };
        assert!((a[0] - want).abs() < 1e-4, "{a:?} {b:?}");
    }
    // The normalized depth spans 0..1 (background depth is ignored).
    let v = out.viewer.expect("viewer");
    let (lo, hi) = v.pixels.iter().fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(p[0]), h.max(p[0])));
    assert!(lo >= 0.0 && lo < 0.05 && hi > 0.95, "{lo} {hi}");

    // A Blur on the render keeps its size; a Scale changes it.
    let mut t2 = NodeTree::new("C", TreeKind::Compositor);
    let rl = t2.add("CompositorNodeRLayers", [0.0, 0.0]).unwrap();
    let sc = t2.add("CompositorNodeScale", [0.0, 0.0]).unwrap();
    let vw = t2.add("CompositorNodeViewer", [0.0, 0.0]).unwrap();
    let co = t2.add("CompositorNodeComposite", [0.0, 0.0]).unwrap();
    t2.nodes[sc].set_input("X", Value::Float(0.5));
    t2.nodes[sc].set_input("Y", Value::Float(0.5));
    t2.link(rl, "Image", sc, "Image").unwrap();
    t2.link(sc, "Image", vw, "Image").unwrap();
    t2.link(sc, "Image", co, "Image").unwrap();
    let out = composite(&t2, &ctx);
    let v = out.viewer.unwrap();
    assert_eq!((v.width, v.height), (24, 16));
    // The Composite output is always the render size (the smaller image centred).
    let c = out.composite.unwrap();
    assert_eq!((c.width, c.height), (48, 32));
    assert_eq!(c.pixels[0], [0.0; 4]);
    let _ = Prop::Bool(true);
}
