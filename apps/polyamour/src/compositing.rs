// SPDX-License-Identifier: GPL-3.0-only
//! Glue between F12 / Ctrl F12 and the compositor: when the scene has a
//! compositing tree (Compositor > Use Nodes) the render result goes through
//! it; the Composite node becomes the Render Result, the Viewer node the
//! "Viewer Node" image, and File Output nodes write PNGs.
use crate::doc::Doc;
use sex_3d::compositor::{self, CompositeOutput, Context};
use sex_3d::nodes::comp_registry::set_render_passes;
use sex_3d::render::{ColorManagement, Passes, RenderResult};
use sex_3d::scene::Scene;

/// Show the View Layer's passes on every Render Layers node.
pub fn sync_passes(scene: &mut Scene, passes: Passes) {
    let mut trees: Vec<&mut sex_3d::nodes::NodeTree> = scene.compositor.iter_mut().collect();
    trees.extend(scene.node_groups.iter_mut().filter(|g| g.kind == sex_3d::nodes::TreeKind::Compositor));
    for t in trees {
        for n in t.nodes.iter_mut().filter(|n| n.idname == "CompositorNodeRLayers") {
            set_render_passes(n, passes);
        }
    }
}

/// Run the scene's compositor on `r` at `frame`; None without Use Nodes.
pub fn run(scene: &Scene, r: &RenderResult, frame: i32, scale: f32) -> Option<CompositeOutput> {
    let tree = scene.compositor.as_ref()?;
    let mut ctx = Context::new(scene, Some(r), (r.color.width as usize, r.color.height as usize));
    ctx.frame = frame;
    ctx.render_scale = scale;
    Some(compositor::composite(tree, &ctx))
}

/// Write File Output images as colour-managed PNGs; returns the paths.
pub fn write_files(out: &CompositeOutput, cm: &ColorManagement) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for (path, img) in &out.files {
        let name = format!("{path}.png");
        let png = crate::editors::image::encode_png(img.width, img.height, &sex_3d::render::to_display8(img, cm));
        std::fs::write(&name, png).map_err(|e| format!("Cannot write {name}: {e}"))?;
        written.push(name);
    }
    Ok(written)
}

/// F12's compositing step: replaces the render shown with the Composite
/// output and stores the Viewer image. Returns a note for the report.
pub fn apply(doc: &mut Doc, r: &RenderResult) -> String {
    let passes = doc.settings.render_settings().passes;
    sync_passes(&mut doc.scene, passes);
    let scale = doc.settings.resolution_pct as f32 / 100.0;
    let Some(out) = run(&doc.scene, r, doc.frame, scale) else { return String::new() };
    if let Some(v) = out.viewer.clone() {
        doc.viewer = Some(v);
        doc.viewer_gen += 1;
    }
    let mut note = String::from(", composited");
    if let Err(e) = write_files(&out, &doc.settings.color_management()) {
        note = format!(", {e}");
    }
    if let Some(c) = out.composite {
        doc.render = Some(c);
    }
    note
}
