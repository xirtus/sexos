// SPDX-License-Identifier: GPL-3.0-only
//! UV operators on the active mesh: marking seams and sharp edges
//! (`mesh.mark_seam`, `mesh.mark_sharp`) and the 3D Viewport's UV menu
//! (`uv.unwrap`, `uv.smart_project`, projections, packing, ...). The
//! geometry work is `sex_3d::uv`; every change is one undo step.
//!
//! Like Blender, the operators act on the faces selected in Edit Mode; in
//! other modes they act on the whole active mesh. Options ride in the op's
//! argument as words or `key=value` pairs (`uv.smart_project angle=66
//! margin=0.02`), with Blender's defaults when absent.
use crate::doc::{Doc, Elem, Mode};
use crate::edit::{active_mesh, face_hidden};
use crate::ui::Op;
use sex_3d::uv::{self, FollowMode, UnwrapMethod, UnwrapOptions};
use sex_3d::{Mesh, Vec2, Vec3};

/// The faces UV operators work on.
pub fn op_faces(d: &Doc) -> Vec<u32> {
    let Some((_, mi)) = active_mesh(d) else { return Vec::new() };
    let m = &d.scene.meshes[mi];
    if d.mode == Mode::Edit {
        d.edit.faces.iter().copied().filter(|&f| (f as usize) < m.faces.len() && !face_hidden(m, f as usize, &d.edit_hidden)).collect()
    } else {
        (0..m.faces.len() as u32).collect()
    }
}

/// Change the active mesh's UVs as one undo step. False if there is no mesh.
pub fn edit_uvs(d: &mut Doc, label: &str, f: impl FnOnce(&mut Mesh)) -> bool {
    let Some((_, mi)) = active_mesh(d) else { return false };
    d.push_undo(label);
    f(&mut d.scene.meshes[mi]);
    d.touch();
    true
}

/// `key=value` from an op argument, or None.
fn opt<'a>(arg: &'a str, key: &str) -> Option<&'a str> {
    arg.split([' ', ',']).find_map(|w| w.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v))
}
fn optf(arg: &str, key: &str, default: f32) -> f32 {
    opt(arg, key).and_then(|v| v.parse().ok()).filter(|v: &f32| v.is_finite()).unwrap_or(default)
}
fn has_word(arg: &str, w: &str) -> bool {
    arg.split([' ', ',']).any(|x| x.eq_ignore_ascii_case(w))
}

/// Run `mesh.mark_seam`/`mesh.mark_sharp` and the `uv.*` operators that
/// work on the mesh. False for ids this module does not handle.
pub fn run(d: &mut Doc, op: &Op) -> bool {
    let (id, arg) = (op.id.as_str(), op.arg.as_str());
    if arg.starts_with('@') {
        return false; // mail for an editor
    }
    match id {
        "mesh.mark_seam" | "mesh.mark_sharp" => {
            if d.mode != Mode::Edit {
                return false;
            }
            mark_edges(d, id == "mesh.mark_seam", has_word(arg, "CLEAR") || opt(arg, "clear").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")));
            true
        }
        _ if id.starts_with("uv.") => run_uv(d, id, arg),
        _ => false,
    }
}

/// Mark or clear the selected edges as seams or sharp.
fn mark_edges(d: &mut Doc, seam: bool, clear: bool) {
    let Some((_, mi)) = active_mesh(d) else {
        d.report = "No mesh in Edit Mode".into();
        return;
    };
    let edges: Vec<(u32, u32)> = d.edit.edges.iter().copied().filter(|e| !d.edit_hidden.contains(&e.0) && !d.edit_hidden.contains(&e.1)).collect();
    if edges.is_empty() {
        d.report = "No edges selected".into();
        return;
    }
    let label = match (seam, clear) {
        (true, false) => "Mark Seam",
        (true, true) => "Clear Seam",
        (false, false) => "Mark Sharp",
        (false, true) => "Clear Sharp",
    };
    d.push_undo(label);
    let m = &mut d.scene.meshes[mi];
    for (a, b) in edges {
        if seam {
            m.set_seam(a, b, !clear);
        } else {
            m.set_sharp(a, b, !clear);
        }
    }
    d.touch();
}

const KNOWN: [&str; 12] = [
    "uv.unwrap",
    "uv.smart_project",
    "uv.lightmap_pack",
    "uv.follow_active_quads",
    "uv.cube_project",
    "uv.cylinder_project",
    "uv.sphere_project",
    "uv.project_from_view",
    "uv.reset",
    "uv.pack_islands",
    "uv.average_islands_scale",
    "uv.seams_from_islands",
];

fn label(id: &str) -> &'static str {
    match id {
        "uv.unwrap" => "Unwrap",
        "uv.smart_project" => "Smart UV Project",
        "uv.lightmap_pack" => "Lightmap Pack",
        "uv.follow_active_quads" => "Follow Active Quads",
        "uv.cube_project" => "Cube Projection",
        "uv.cylinder_project" => "Cylinder Projection",
        "uv.sphere_project" => "Sphere Projection",
        "uv.project_from_view" => "Project from View",
        "uv.reset" => "Reset",
        "uv.pack_islands" => "Pack Islands",
        "uv.average_islands_scale" => "Average Islands Scale",
        _ => "Seams from Islands",
    }
}

fn run_uv(d: &mut Doc, id: &str, arg: &str) -> bool {
    if !KNOWN.contains(&id) {
        return false;
    }
    if active_mesh(d).is_none() {
        d.report = "No active mesh object".into();
        return true;
    }
    let faces = op_faces(d);
    if faces.is_empty() {
        d.report = "No faces selected".into();
        return true;
    }
    if id == "uv.project_from_view" {
        // Reached only without a 3D Viewport (the viewport handles it with
        // its own view): use the front view.
        project_from_view(d, None, has_word(arg, "BOUNDS"));
        return true;
    }
    if id == "uv.follow_active_quads" {
        let active = match d.edit.active {
            Some(Elem::Face(f)) if faces.contains(&f) => f,
            _ => {
                d.report = "Active face must exist and be a quad".into();
                return true;
            }
        };
        let mode = match opt(arg, "mode").unwrap_or(arg) {
            "EVEN" => FollowMode::Even,
            "LENGTH" => FollowMode::Length,
            _ => FollowMode::LengthAverage,
        };
        let mi = active_mesh(d).map(|a| a.1).expect("mesh");
        let mut m = d.scene.meshes[mi].clone();
        if !uv::follow_active_quads(&mut m, &faces, active, mode) {
            d.report = "Active face must exist and be a quad".into();
            return true;
        }
        edit_uvs(d, label(id), |mesh| *mesh = m);
        return true;
    }
    let mut report = String::new();
    edit_uvs(d, label(id), |m| match id {
        "uv.unwrap" => {
            let method = if has_word(arg, "CONFORMAL") || opt(arg, "method") == Some("CONFORMAL") { UnwrapMethod::Conformal } else { UnwrapMethod::AngleBased };
            let opts = UnwrapOptions { method, fill_holes: opt(arg, "fill_holes") != Some("0"), margin: optf(arg, "margin", 0.001) };
            let r = uv::unwrap(m, &faces, &opts);
            if r.failed > 0 {
                report = if r.islands == 0 {
                    "Unwrap could not solve any island(s), edge seams may need to be added".into()
                } else {
                    format!("Unwrap failed to solve {} of {} island(s), edge seams may need to be added", r.failed, r.islands + r.failed)
                };
            }
        }
        "uv.smart_project" => {
            let angle = optf(arg, "angle", 66.0).clamp(0.0, 89.0).to_radians();
            uv::smart_project(m, &faces, angle, optf(arg, "margin", 0.0), optf(arg, "area_weight", 0.0));
        }
        "uv.lightmap_pack" => uv::lightmap_pack(m, &faces, optf(arg, "margin", 0.1)),
        "uv.cube_project" => {
            uv::cube_project(m, &faces, opt(arg, "size").and_then(|v| v.parse().ok()));
            uv::clip_correct(m, &faces, has_word(arg, "BOUNDS"), has_word(arg, "CLIP"));
        }
        "uv.cylinder_project" => {
            uv::cylinder_project(m, &faces, optf(arg, "radius", 1.0));
            uv::clip_correct(m, &faces, has_word(arg, "BOUNDS"), has_word(arg, "CLIP"));
        }
        "uv.sphere_project" => {
            uv::sphere_project(m, &faces);
            uv::clip_correct(m, &faces, has_word(arg, "BOUNDS"), has_word(arg, "CLIP"));
        }
        "uv.reset" => uv::reset(m, &faces),
        "uv.pack_islands" => uv::pack_islands(m, &faces, optf(arg, "margin", 0.001), opt(arg, "rotate") != Some("0")),
        "uv.average_islands_scale" => uv::average_islands_scale(m, &faces),
        _ => seams_from_islands(m, &faces),
    });
    d.report = report;
    true
}

/// UV > Seams from Islands: mark seams on edges whose two faces have
/// different UVs (or that bound the UV islands).
fn seams_from_islands(m: &mut Mesh, faces: &[u32]) {
    let isl = uv::uv_islands(m, faces);
    let mut of = std::collections::HashMap::new();
    for (i, f) in isl.iter().enumerate() {
        for &x in f {
            of.insert(x, i);
        }
    }
    let mut by_edge: std::collections::HashMap<(u32, u32), Vec<u32>> = std::collections::HashMap::new();
    for &f in faces {
        for (a, b) in m.faces[f as usize].edges() {
            by_edge.entry(sex_3d::mesh::edge_key(a, b)).or_default().push(f);
        }
    }
    for (e, fs) in by_edge {
        if fs.len() == 2 && of.get(&fs[0]) != of.get(&fs[1]) {
            m.seams.insert(e);
        }
    }
}

/// UV > Project from View. `project` maps a world-space point to window
/// pixels with the view rectangle (x, y, w, h); None uses the front view.
pub fn project_from_view(d: &mut Doc, view: Option<(&dyn Fn(Vec3) -> Option<(f32, f32)>, (f32, f32, f32, f32))>, bounds: bool) {
    let Some((oi, mi)) = active_mesh(d) else {
        d.report = "No active mesh object".into();
        return;
    };
    let faces = op_faces(d);
    if faces.is_empty() {
        d.report = "No faces selected".into();
        return;
    }
    let world = d.scene.world_matrix(oi);
    d.push_undo("Project from View");
    let m = &mut d.scene.meshes[mi];
    match view {
        Some((to_screen, (x, y, w, h))) => {
            // Blender's BLI_uvproject_from_view: the view's larger side spans 0..1.
            let s = w.max(h).max(1.0);
            let (cx, cy) = (x + w * 0.5, y + h * 0.5);
            uv::project_from_view(m, &faces, |p| to_screen(world.transform_point(p)).map(|(sx, sy)| Vec2::new(0.5 + (sx - cx) / s, 0.5 + (cy - sy) / s)), bounds);
        }
        None => uv::project_front(m, &faces, bounds),
    }
    d.touch();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit_doc() -> Doc {
        let mut d = Doc::default();
        d.run(&Op::with("object.mode_set", "EDIT"));
        d
    }

    fn mesh(d: &Doc) -> &Mesh {
        &d.scene.meshes[active_mesh(d).unwrap().1]
    }

    fn finite_uvs(m: &Mesh) {
        assert!(m.has_uvs());
        assert!(m.faces.iter().all(|f| f.uvs.iter().all(|u| u.x.is_finite() && u.y.is_finite())));
    }

    #[test]
    fn mark_and_clear_seams_and_sharp_with_undo() {
        let mut d = edit_doc();
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        crate::edit::select_elem(&mut d, Some(Elem::Vert(0)), false);
        crate::edit::select_elem(&mut d, Some(Elem::Vert(1)), true);
        let sel = d.edit.edges.clone();
        assert_eq!(sel.len(), 1);
        d.run(&Op::new("mesh.mark_seam"));
        d.run(&Op::new("mesh.mark_sharp"));
        let e = *sel.iter().next().unwrap();
        assert!(mesh(&d).is_seam(e.0, e.1) && mesh(&d).is_sharp(e.0, e.1));
        d.run(&Op::with("mesh.mark_seam", "CLEAR"));
        assert!(!mesh(&d).is_seam(e.0, e.1));
        d.run(&Op::new("ed.undo"));
        assert!(mesh(&d).is_seam(e.0, e.1));
        // The .poly file keeps them.
        let text = crate::persist::save(&d);
        assert!(text.contains(" seams=") && text.contains(" sharp="), "{text}");
    }

    #[test]
    fn uv_menu_ops_all_run() {
        for id in KNOWN {
            let mut d = edit_doc();
            // Seam the cube open (all edges: six islands) so Unwrap can solve.
            d.run(&Op::with("mesh.select_all", "SELECT"));
            d.run(&Op::new("mesh.mark_seam"));
            if id == "uv.follow_active_quads" {
                d.edit.active = Some(Elem::Face(0));
            }
            d.report.clear();
            d.run(&Op::new(id));
            if id == "uv.project_from_view" {
                // Queued for the 3D Viewport, which projects with its own view.
                assert!(d.view_ops.iter().any(|o| o.id == id));
                continue;
            }
            assert!(d.report.is_empty() || !d.report.starts_with("Unknown"), "{id}: {}", d.report);
            finite_uvs(mesh(&d));
            assert_eq!(d.undo.undo_label(), Some(label(id)), "{id}");
        }
    }

    #[test]
    fn unwrap_without_seams_reports_like_blender() {
        let mut d = edit_doc();
        d.run(&Op::new("uv.unwrap"));
        assert!(d.report.contains("edge seams may need to be added"), "{}", d.report);
    }

    #[test]
    fn unwrap_selected_faces_only() {
        let mut d = edit_doc();
        d.run(&Op::with("mesh.select_all", "SELECT"));
        d.run(&Op::new("uv.reset"));
        let before = mesh(&d).faces[1].uvs.clone();
        d.run(&Op::with("mesh.select_mode", "FACE"));
        crate::edit::select_faces(&mut d, &[0]);
        d.run(&Op::new("uv.sphere_project"));
        assert_eq!(mesh(&d).faces[1].uvs, before);
        assert_ne!(mesh(&d).faces[0].uvs, before);
        // Project from View without a viewport uses the front view.
        project_from_view(&mut d, None, true);
        finite_uvs(mesh(&d));
    }
}
