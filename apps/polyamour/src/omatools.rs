// SPDX-License-Identifier: GPL-3.0-only
//! Omamesh-style pure-Rust 3D generation and 3D-printing preparation operators
//! for Polyamour.
use crate::doc::Doc;
use crate::ui::Op;
use sex_3d::io::decode_image;
use sex_3d::photo_to_mesh::{photo_to_mesh, PhotoMeshOptions, PhotoMeshQuality};
use sex_3d::print_prep::{
    decimate_qem, inspect_manifold, mesh_bounds_mm, orient_upright, place_on_bed, print_metrics,
    repair_for_printing, scale_to_height_mm, ManifoldReport, PrintMetrics,
};
use sex_3d::render::srgb_encode;
use sex_3d::scene::{Image, Material, ObjectData};
use sex_3d::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::path::Path;

/// Parsed arguments for `object.photo_to_mesh` / `omamesh.photo_to_mesh`.
#[derive(Clone, Debug, Default)]
pub struct PhotoOpArgs {
    pub path: Option<String>,
    pub image_name: Option<String>,
    pub name: Option<String>,
    pub options: PhotoMeshOptions,
}

fn parse_kv_pairs(arg: &str) -> (Vec<String>, HashMap<String, String>) {
    let mut bare = Vec::new();
    let mut kv = HashMap::new();
    for raw in arg.split(|c: char| c == ';' || c == ',' || c.is_whitespace()) {
        let tok = raw.trim();
        if tok.is_empty() {
            continue;
        }
        if let Some((k, v)) = tok.split_once('=') {
            kv.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        } else {
            bare.push(tok.to_string());
        }
    }
    (bare, kv)
}

fn parse_quality(s: &str) -> Option<PhotoMeshQuality> {
    match s.trim().to_ascii_lowercase().as_str() {
        "draft" | "low" => Some(PhotoMeshQuality::Draft),
        "standard" | "med" | "medium" => Some(PhotoMeshQuality::Standard),
        "high" | "ultra" => Some(PhotoMeshQuality::High),
        _ => None,
    }
}

/// Parse [`PhotoOpArgs`] from an operator `arg` string.
pub fn parse_photo_args(arg: &str) -> PhotoOpArgs {
    let mut out = PhotoOpArgs::default();
    let (bare, kv) = parse_kv_pairs(arg);
    for token in bare {
        if let Some(q) = parse_quality(&token) {
            out.options.quality = q;
        } else if out.path.is_none() && (token.contains('/') || token.contains('\\') || token.contains('.')) {
            out.path = Some(token);
        } else if out.image_name.is_none() {
            out.image_name = Some(token);
        }
    }
    if let Some(v) = kv.get("quality") {
        if let Some(q) = parse_quality(v) {
            out.options.quality = q;
        }
    }
    if let Some(v) = kv.get("seed").or_else(|| kv.get("take_seed")) {
        if let Ok(seed) = v.parse::<u32>() {
            out.options.take_seed = seed;
        }
    }
    if let Some(v) = kv.get("thickness").or_else(|| kv.get("thickness_ratio")) {
        if let Ok(t) = v.parse::<f32>() {
            out.options.thickness_ratio = t;
        }
    }
    if let Some(v) = kv.get("height").or_else(|| kv.get("height_mm")).or_else(|| kv.get("real_height_mm")) {
        if let Ok(h) = v.parse::<f32>() {
            if h > 0.0 && h.is_finite() {
                out.options.real_height_mm = Some(h);
            }
        }
    }
    if let Some(v) = kv.get("path").or_else(|| kv.get("file")) {
        out.path = Some(v.clone());
    }
    if let Some(v) = kv.get("image") {
        out.image_name = Some(v.clone());
    }
    if let Some(v) = kv.get("name") {
        out.name = Some(v.clone());
    }
    out
}

fn image_to_rgba8(img: &Image) -> Result<Vec<u8>, String> {
    let expected = (img.width as usize).saturating_mul(img.height as usize);
    if expected == 0 || img.pixels.len() != expected {
        return Err(format!("image '{}' has invalid dimensions {}x{}", img.name, img.width, img.height));
    }
    let mut rgba = Vec::with_capacity(expected * 4);
    for p in &img.pixels {
        let r = (srgb_encode(p[0].clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
        let g = (srgb_encode(p[1].clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
        let b = (srgb_encode(p[2].clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
        let a = (p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        rgba.extend_from_slice(&[r, g, b, a]);
    }
    Ok(rgba)
}

/// Generate a watertight textured relief mesh from an RGBA8 pixel buffer and
/// add it as an active selected object to `d`.
pub fn generate_photo_mesh_rgba(
    d: &mut Doc,
    name: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
    options: PhotoMeshOptions,
) -> Result<usize, String> {
    let res = photo_to_mesh(width, height, rgba, options)?;
    let obj_name = if name.trim().is_empty() { "PhotoMesh" } else { name.trim() };

    d.push_undo("Photo to Mesh");
    d.scene.images.push(res.texture);
    let img_idx = d.scene.images.len() - 1;
    let mat = Material {
        name: format!("{obj_name}_Mat"),
        base_color: [1.0, 1.0, 1.0, 1.0],
        base_color_texture: Some(img_idx),
        metallic: 0.0,
        roughness: 0.6,
        ..Default::default()
    };
    let mat_idx = d.scene.add_material(mat);
    let mesh = res.mesh;
    let rep = inspect_manifold(&mesh);
    let metrics = print_metrics(&mesh);
    let obj_idx = d.scene.add_mesh_object(obj_name, mesh);
    d.scene.objects[obj_idx].materials.push(mat_idx);
    // Keep min.z at 0.0 so the generated object lands upright on the print bed.
    d.scene.objects[obj_idx].transform.location = Vec3::new(d.cursor.x, d.cursor.y, 0.0);
    d.select_only(Some(obj_idx));
    d.last_manifold = Some(rep.clone());
    d.last_print_metrics = Some(metrics);
    d.report = format!(
        "Generated '{obj_name}' ({} tris, {:.1}x{:.1}x{:.1} mm, {:.2} cm^3{})",
        rep.triangle_count,
        metrics.bounds_mm.width,
        metrics.bounds_mm.depth,
        metrics.bounds_mm.height,
        metrics.volume_cm3,
        if rep.is_watertight { ", watertight" } else { "" }
    );
    d.touch();
    Ok(obj_idx)
}

/// Generate a watertight textured relief mesh from an existing linear-light [`Image`].
pub fn generate_photo_mesh_from_image(
    d: &mut Doc,
    name: &str,
    img: &Image,
    options: PhotoMeshOptions,
) -> Result<usize, String> {
    let rgba = image_to_rgba8(img)?;
    generate_photo_mesh_rgba(d, name, img.width, img.height, &rgba, options)
}

fn selected_mesh_targets(d: &Doc) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for &oi in &d.scene.selected {
        if let Some(obj) = d.scene.objects.get(oi) {
            if let ObjectData::Mesh(mi) = obj.data {
                out.push((oi, mi));
            }
        }
    }
    if out.is_empty() {
        if let Some(oi) = d.active() {
            if let Some(obj) = d.scene.objects.get(oi) {
                if let ObjectData::Mesh(mi) = obj.data {
                    out.push((oi, mi));
                }
            }
        }
    }
    out
}

/// Bake non-identity rotation and scale from the object's transform into its
/// mesh so print-prep geometry operations align both raw mesh coordinates and
/// viewport appearance.
fn bake_object_rot_scale(d: &mut Doc, oi: usize, mi: usize) {
    let t = d.scene.objects[oi].transform;
    if t.rotation != Quat::IDENTITY || t.scale != Vec3::ONE {
        let m = Mat4::from_trs(Vec3::ZERO, t.rotation, t.scale);
        d.scene.meshes[mi].transform(&m);
        d.scene.objects[oi].transform.rotation = Quat::IDENTITY;
        d.scene.objects[oi].transform.scale = Vec3::ONE;
    }
}

fn find_source_image(d: &Doc, args: &PhotoOpArgs) -> Result<(String, Image), String> {
    if let Some(path) = &args.path {
        let bytes = std::fs::read(path).map_err(|e| format!("failed to read '{path}': {e}"))?;
        let stem = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("PhotoMesh")
            .to_string();
        let img = decode_image(&stem, &bytes)?;
        return Ok((args.name.clone().unwrap_or(stem), img));
    }
    if let Some(img_ref) = &args.image_name {
        if let Ok(idx) = img_ref.parse::<usize>() {
            if let Some(img) = d.scene.images.get(idx) {
                let name = args.name.clone().unwrap_or_else(|| img.name.clone());
                return Ok((name, img.clone()));
            }
        }
        if let Some(img) = d.scene.images.iter().find(|i| i.name == *img_ref) {
            let name = args.name.clone().unwrap_or_else(|| img.name.clone());
            return Ok((name, img.clone()));
        }
        return Err(format!("image '{img_ref}' not found in scene"));
    }
    if let Some(oi) = d.active() {
        if let Some(mat_idx) = d.scene.objects[oi].materials.first().copied() {
            if let Some(img_idx) = d.scene.materials.get(mat_idx).and_then(|m| m.base_color_texture) {
                if let Some(img) = d.scene.images.get(img_idx) {
                    let name = args.name.clone().unwrap_or_else(|| format!("{}_Relief", img.name));
                    return Ok((name, img.clone()));
                }
            }
        }
    }
    if let Some(img) = d.scene.images.last() {
        let name = args.name.clone().unwrap_or_else(|| format!("{}_Relief", img.name));
        return Ok((name, img.clone()));
    }
    Err("no image available (load a scene image or pass path=...)".into())
}

fn parse_scalar_arg(arg: &str, keys: &[&str]) -> Option<f32> {
    let (bare, kv) = parse_kv_pairs(arg);
    for k in keys {
        if let Some(v) = kv.get(*k) {
            if let Ok(val) = v.parse::<f32>() {
                return Some(val);
            }
        }
    }
    bare.first().and_then(|s| s.parse::<f32>().ok())
}

/// Scale selected mesh object(s) to `target_height_mm` millimetres in Z.
pub fn run_scale_to_height_mm(d: &mut Doc, target_height_mm: f32) -> Result<PrintMetrics, String> {
    if !target_height_mm.is_finite() || target_height_mm <= 0.0 {
        return Err("target height in mm must be positive".into());
    }
    let targets = selected_mesh_targets(d);
    if targets.is_empty() {
        return Err("Select a mesh object first".into());
    }
    d.push_undo("Scale to Height (mm)");
    let mut last_metrics = None;
    for (oi, mi) in targets {
        bake_object_rot_scale(d, oi, mi);
        let mesh = &mut d.scene.meshes[mi];
        let before = mesh.bounds();
        scale_to_height_mm(mesh, target_height_mm);
        let after = mesh.bounds();
        if !before.is_empty() && !after.is_empty() && before.size().z > 1e-8 {
            let factor = after.size().z / before.size().z;
            d.scene.objects[oi].transform.location.z *= factor;
        }
        last_metrics = Some(print_metrics(mesh));
    }
    d.touch();
    let metrics = last_metrics.unwrap_or_default();
    d.last_print_metrics = Some(metrics);
    d.report = format!(
        "Scaled to {:.2} x {:.2} x {:.2} mm ({:.2} cm^3)",
        metrics.bounds_mm.width,
        metrics.bounds_mm.depth,
        metrics.bounds_mm.height,
        metrics.volume_cm3
    );
    Ok(metrics)
}

/// Orient selected mesh object(s) upright using PCA/support-hull alignment.
pub fn run_orient_upright(d: &mut Doc) -> Result<(), String> {
    let targets = selected_mesh_targets(d);
    if targets.is_empty() {
        return Err("Select a mesh object first".into());
    }
    d.push_undo("Orient Upright");
    for (oi, mi) in &targets {
        bake_object_rot_scale(d, *oi, *mi);
        orient_upright(&mut d.scene.meshes[*mi]);
    }
    let (_, mi) = targets[0];
    let metrics = print_metrics(&d.scene.meshes[mi]);
    d.last_print_metrics = Some(metrics);
    d.touch();
    d.report = format!(
        "Oriented upright ({:.2} x {:.2} x {:.2} mm)",
        metrics.bounds_mm.width, metrics.bounds_mm.depth, metrics.bounds_mm.height
    );
    Ok(())
}

/// Place selected mesh object(s) flat on the `Z = 0` print bed.
pub fn run_place_on_bed(d: &mut Doc) -> Result<PrintMetrics, String> {
    let targets = selected_mesh_targets(d);
    if targets.is_empty() {
        return Err("Select a mesh object first".into());
    }
    d.push_undo("Place on Bed");
    let mut last_metrics = None;
    for (oi, mi) in targets {
        bake_object_rot_scale(d, oi, mi);
        place_on_bed(&mut d.scene.meshes[mi]);
        d.scene.objects[oi].transform.location.z = 0.0;
        last_metrics = Some(print_metrics(&d.scene.meshes[mi]));
    }
    d.touch();
    let metrics = last_metrics.unwrap_or_default();
    d.last_print_metrics = Some(metrics);
    d.report = format!(
        "Placed on bed at Z=0 ({:.2} x {:.2} x {:.2} mm)",
        metrics.bounds_mm.width, metrics.bounds_mm.depth, metrics.bounds_mm.height
    );
    Ok(metrics)
}

/// Decimate selected mesh object(s) with boundary & UV-seam-preserving QEM.
pub fn run_decimate_qem(d: &mut Doc, target_ratio: f32) -> Result<ManifoldReport, String> {
    if !target_ratio.is_finite() || target_ratio <= 0.0 {
        return Err("decimation ratio must be in (0, 1]".into());
    }
    let targets = selected_mesh_targets(d);
    if targets.is_empty() {
        return Err("Select a mesh object first".into());
    }
    d.push_undo("QEM Decimate");
    let mut before_tris = 0usize;
    let mut after_rep = ManifoldReport::default();
    let mut after_metrics = PrintMetrics::default();
    for (_oi, mi) in targets {
        before_tris = d.scene.meshes[mi].triangulate().len();
        let dec = decimate_qem(&d.scene.meshes[mi], target_ratio);
        after_rep = inspect_manifold(&dec);
        after_metrics = print_metrics(&dec);
        d.scene.meshes[mi] = dec;
    }
    d.last_manifold = Some(after_rep.clone());
    d.last_print_metrics = Some(after_metrics);
    d.touch();
    d.report = format!(
        "QEM decimated: {} -> {} tris (watertight: {})",
        before_tris, after_rep.triangle_count, after_rep.is_watertight
    );
    Ok(after_rep)
}

/// Inspect active/selected mesh object(s) for manifold & watertight readiness.
pub fn run_inspect_manifold(d: &mut Doc) -> Result<(ManifoldReport, PrintMetrics), String> {
    let targets = selected_mesh_targets(d);
    if targets.is_empty() {
        return Err("Select a mesh object first".into());
    }
    let (oi, mi) = targets[0];
    let mesh = d.scene.evaluated_mesh(oi).unwrap_or_else(|| d.scene.meshes[mi].clone());
    let rep = inspect_manifold(&mesh);
    let metrics = print_metrics(&mesh);
    let b = mesh_bounds_mm(&mesh);
    d.last_manifold = Some(rep.clone());
    d.last_print_metrics = Some(metrics);
    d.report = format!(
        "Manifold: {}, Watertight: {} | open={}, non_manif={}, degen={} | {:.1}x{:.1}x{:.1}mm, {:.2}cm^3, {:.1}cm^2",
        rep.is_manifold,
        rep.is_watertight,
        rep.boundary_edges,
        rep.non_manifold_edges + rep.non_manifold_vertices,
        rep.degenerate_faces,
        b.width,
        b.depth,
        b.height,
        metrics.volume_cm3,
        metrics.surface_area_cm2,
    );
    Ok((rep, metrics))
}

/// Repair selected mesh object(s) for 3D printing (weld, degenerate cleanup,
/// loop closing, outward normals).
pub fn run_repair_for_printing(d: &mut Doc) -> Result<ManifoldReport, String> {
    let targets = selected_mesh_targets(d);
    if targets.is_empty() {
        return Err("Select a mesh object first".into());
    }
    d.push_undo("Repair for Printing");
    let mut last_rep = ManifoldReport::default();
    let mut last_metrics = PrintMetrics::default();
    for (oi, mi) in targets {
        bake_object_rot_scale(d, oi, mi);
        repair_for_printing(&mut d.scene.meshes[mi]);
        last_rep = inspect_manifold(&d.scene.meshes[mi]);
        last_metrics = print_metrics(&d.scene.meshes[mi]);
    }
    d.last_manifold = Some(last_rep.clone());
    d.last_print_metrics = Some(last_metrics);
    d.touch();
    d.report = format!(
        "Repaired: {} tris, open={}, non_manif={}, watertight={}",
        last_rep.triangle_count,
        last_rep.boundary_edges,
        last_rep.non_manifold_edges + last_rep.non_manifold_vertices,
        last_rep.is_watertight
    );
    Ok(last_rep)
}

/// Dispatch Omamesh-style generation & print-prep operators from `Doc::run`.
pub fn run(d: &mut Doc, op: &Op) -> bool {
    let arg = op.arg.as_str();
    match op.id.as_str() {
        "object.photo_to_mesh" | "mesh.photo_to_mesh" => {
            let args = parse_photo_args(arg);
            let res = find_source_image(d, &args)
                .and_then(|(name, img)| generate_photo_mesh_from_image(d, &name, &img, args.options));
            if let Err(e) = res {
                d.report = format!("Photo to Mesh: {e}");
            }
            true
        }
        "object.scale_to_height_mm" | "mesh.scale_to_height_mm" => {
            let h = parse_scalar_arg(arg, &["height_mm", "height", "mm"]).unwrap_or(50.0);
            if let Err(e) = run_scale_to_height_mm(d, h) {
                d.report = format!("Scale to Height (mm): {e}");
            }
            true
        }
        "object.place_on_bed" | "mesh.place_on_bed" => {
            if let Err(e) = run_place_on_bed(d) {
                d.report = format!("Place on Bed: {e}");
            }
            true
        }
        "object.orient_upright" | "mesh.orient_upright" => {
            if let Err(e) = run_orient_upright(d) {
                d.report = format!("Orient Upright: {e}");
            }
            true
        }
        "object.decimate_qem" | "mesh.decimate_qem" => {
            let ratio = parse_scalar_arg(arg, &["ratio", "target_ratio"]).unwrap_or(0.5);
            if let Err(e) = run_decimate_qem(d, ratio) {
                d.report = format!("QEM Decimate: {e}");
            }
            true
        }
        "object.inspect_manifold" | "mesh.inspect_manifold" => {
            if let Err(e) = run_inspect_manifold(d) {
                d.report = format!("Inspect Manifold: {e}");
            }
            true
        }
        "object.repair_for_printing" | "mesh.repair_for_printing" => {
            if let Err(e) = run_repair_for_printing(d) {
                d.report = format!("Repair for Printing: {e}");
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::editors::{view3d, Editor};
    use crate::input::Input;
    use sex_3d::io::encode_png;
    use sex_3d::primitives;
    use sex_3d::render::workbench::{Lighting, ShadingPreset};

    fn test_badge_image() -> Image {
        let w = 64u32;
        let h = 64u32;
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            let ny = y as f32 / h as f32 * 2.0 - 1.0;
            for x in 0..w {
                let nx = x as f32 / w as f32 * 2.0 - 1.0;
                if nx * nx + ny * ny <= 0.36 {
                    pixels.push([0.8, 0.2, 0.15, 1.0]);
                } else {
                    pixels.push([0.92, 0.92, 0.94, 1.0]);
                }
            }
        }
        Image { name: "badge".into(), width: w, height: h, pixels }
    }

    fn view_state(app: &mut App) -> &mut view3d::State {
        for a in app.workspaces[app.active_ws].areas_mut() {
            if let Editor::View3D(s) = &mut a.editor {
                return s;
            }
        }
        panic!("missing 3D viewport");
    }

    #[test]
    fn photo_to_mesh_operator_generates_watertight_mesh_from_scene_image_and_undoes() {
        let mut d = Doc::default();
        let orig_objs = d.scene.objects.len();
        d.scene.images.push(test_badge_image());

        d.run(&Op::with("object.photo_to_mesh", "quality=draft seed=7 height_mm=48"));
        assert_eq!(d.scene.objects.len(), orig_objs + 1, "report: {}", d.report);
        let obj = d.active().expect("generated object active");
        let mi = match d.scene.objects[obj].data {
            ObjectData::Mesh(mi) => mi,
            _ => panic!("expected mesh object"),
        };
        let rep = inspect_manifold(&d.scene.meshes[mi]);
        assert!(rep.is_watertight, "generated mesh must be watertight: {rep:?}");
        let b = mesh_bounds_mm(&d.scene.meshes[mi]);
        assert!((b.height - 48.0).abs() < 1e-2, "height_mm mismatch: {b:?}");
        assert!(d.scene.meshes[mi].bounds().min.z.abs() < 1e-4);

        d.run(&Op::new("ed.undo"));
        assert_eq!(d.scene.objects.len(), orig_objs);
    }

    #[test]
    fn photo_to_mesh_operator_reads_png_path_and_exports_glb() {
        let img = test_badge_image();
        let png_bytes = encode_png(&img);
        let tmp = std::env::temp_dir().join(format!("polyamour-omatools-test-{}.png", std::process::id()));
        std::fs::write(&tmp, png_bytes).expect("write temp png");

        let mut d = Doc::default();
        let arg = format!("path={};quality=draft;height_mm=60", tmp.to_string_lossy());
        d.run(&Op::with("object.photo_to_mesh", &arg));
        let _ = std::fs::remove_file(&tmp);

        let obj = d.active().expect("generated object");
        let rep = d.last_manifold.clone().expect("last_manifold stored");
        assert!(rep.is_watertight, "report: {rep:?}");
        assert!(!d.scene.objects[obj].materials.is_empty());

        let glb = sex_3d::io::gltf::export(&d.scene, true).expect("glb export");
        assert!(glb.starts_with(b"glTF"));
    }

    #[test]
    fn print_prep_operators_scale_bed_orient_decimate_and_repair() {
        let mut d = Doc::default();
        // Replace Cube with a cube rotated in object space and broken slightly.
        let mut broken = primitives::cube(10.0);
        broken.faces.pop();
        broken.faces[0].verts.reverse();
        let broken_mi = d.scene.add_mesh("TestPart", broken);
        let oi = d.add_object("TestPart", ObjectData::Mesh(broken_mi), "Add TestPart");
        d.scene.objects[oi].transform.location = Vec3::new(1.0, 2.0, 7.5);
        d.scene.objects[oi].transform.rotation = Quat::from_axis_angle(Vec3::new(0.3, 0.7, 0.1).normalize(), 0.9);
        d.select_only(Some(oi));

        // 1. Inspect detects open boundary.
        d.run(&Op::new("object.inspect_manifold"));
        let rep0 = d.last_manifold.clone().unwrap();
        assert!(!rep0.is_watertight);
        assert!(rep0.boundary_edges > 0);

        // 2. Repair closes boundary and restores outward normals.
        d.run(&Op::new("object.repair_for_printing"));
        let rep1 = d.last_manifold.clone().unwrap();
        assert!(rep1.is_watertight, "after repair: {rep1:?}");

        // 3. Orient upright + Place on bed + Scale to 80 mm height.
        d.run(&Op::new("object.orient_upright"));
        d.run(&Op::new("object.place_on_bed"));
        let mi = match d.scene.objects[oi].data {
            ObjectData::Mesh(mi) => mi,
            _ => unreachable!(),
        };
        assert_eq!(d.scene.objects[oi].transform.location.z, 0.0);
        assert!(d.scene.meshes[mi].bounds().min.z.abs() < 1e-4);

        d.run(&Op::with("object.scale_to_height_mm", "80"));
        let b = mesh_bounds_mm(&d.scene.meshes[mi]);
        assert!((b.height - 80.0).abs() < 1e-2, "bounds: {b:?}");
        let metrics = d.last_print_metrics.unwrap();
        assert!(metrics.volume_cm3 > 0.0);

        // 4. QEM decimate preserves watertightness on a UV sphere.
        let sphere_mi = d.scene.add_mesh("DecimateSphere", primitives::uv_sphere(20, 10, 15.0));
        let sp = d.add_object("DecimateSphere", ObjectData::Mesh(sphere_mi), "Add Sphere");
        d.select_only(Some(sp));
        let before = d.scene.evaluated_mesh(sp).unwrap().triangulate().len();
        d.run(&Op::with("object.decimate_qem", "0.5"));
        let dec_rep = d.last_manifold.clone().unwrap();
        assert!(dec_rep.triangle_count < before);
        assert!(dec_rep.is_watertight, "decimated sphere must stay watertight: {dec_rep:?}");
    }

    #[test]
    fn view3d_shading_presets_shadow_catcher_and_turntable_drive_app_cleanly() {
        let mut app = App::new(640, 360);
        app.frame(Input::Tick);

        app.doc.queue.push(Op::with("view3d.shading_preset", "clay_matcap"));
        app.frame(Input::Tick);
        {
            let st = view_state(&mut app);
            assert_eq!(st.shading_preset, Some(ShadingPreset::ClayMatcap));
            assert_eq!(st.solid.lighting, Lighting::MatCap);
            assert!(!st.show_wire);
        }

        app.doc.queue.push(Op::with("view3d.shading_preset", "world_normals"));
        app.frame(Input::Tick);
        assert!(view_state(&mut app).solid.world_normals);

        app.doc.queue.push(Op::with("view3d.shading_preset", "wireframe_overlay"));
        app.frame(Input::Tick);
        {
            let st = view_state(&mut app);
            assert!(st.show_wire);
            assert!(!st.solid.world_normals);
        }

        app.doc.queue.push(Op::with("view3d.shadow_catcher", "on"));
        app.frame(Input::Tick);
        assert!(view_state(&mut app).shadow_catcher);

        let yaw0 = view_state(&mut app).cam.yaw;
        app.doc.queue.push(Op::with("view3d.turntable", "on"));
        app.frame(Input::Tick);
        assert!(app.wants_tick());
        app.frame(Input::Tick);
        app.frame(Input::Tick);
        let yaw1 = view_state(&mut app).cam.yaw;
        assert!((yaw1 - yaw0).abs() > 1e-3, "expected turntable to advance yaw: {yaw0} -> {yaw1}");

        app.doc.queue.push(Op::with("view3d.turntable", "off"));
        app.frame(Input::Tick);
        assert!(!view_state(&mut app).turntable.active);
        assert!(!app.wants_tick());
    }
}

