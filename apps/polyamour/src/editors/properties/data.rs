// SPDX-License-Identifier: GPL-3.0-only
//! Object Data properties per object type (Mesh, Light, Camera, Empty) and
//! the Material tab (slot list, data-block row, Principled BSDF inputs).
use super::icons::{self, Icon};
use super::layout::{L, GAP, ROW};
use super::{ColorTarget, Ctx};
use crate::doc::Doc;
use crate::draw::Rect;
use crate::theme as t;
use crate::ui::{MenuItem, NumOpts, Op};
use sex_3d::scene::{AreaShape, Camera, CameraType, DepthOfField, Light, LightKind, Material, ObjectData, PanoramaType, SensorFit};

fn inactive<R>(l: &mut L, f: impl FnOnce(&mut L) -> R) {
    let was = l.inactive;
    l.inactive = true;
    let _ = f(l);
    l.inactive = was;
}

const M: NumOpts<'static> = NumOpts { step: 0.1, unit: "m", min: 0.0, max: f32::INFINITY, slider: false, int: false };

pub fn data(l: &mut L, d: &mut Doc, c: &mut Ctx) {
    let Some(o) = c.obj else { return };
    match d.scene.objects[o].data.clone() {
        ObjectData::Mesh(mi) => mesh(l, d, mi),
        ObjectData::Light(li) => light(l, d, c, o, li),
        ObjectData::Camera(cam) => camera(l, d, c, o, cam),
        ObjectData::Empty | ObjectData::Armature(_) | ObjectData::Lattice(_) => {
            l.panel(d, "data.empty", "Empty", true, &mut |l, _| {
                l.info("Display As", "Plain Axes");
                inactive(l, |l| {
                    let mut v = 1.0;
                    l.number("Size", "empty_size", &mut v, M);
                });
            });
        }
    }
}


fn mesh(l: &mut L, d: &mut Doc, mi: usize) {
    let name = d.scene.mesh_names.get(mi).cloned().unwrap_or_default();
    if let Some(n) = l.id_block(Icon::Mesh, &name, Some("mesh_name")) {
        if !n.trim().is_empty() {
            d.push_undo("Rename Mesh");
            let n = sex_3d::scene::unique_name(n.trim(), |x| d.scene.mesh_names.iter().enumerate().any(|(i, m)| i != mi && m == x));
            d.scene.mesh_names[mi] = n;
        }
    }
    l.panel(d, "data.vertex_groups", "Vertex Groups", true, &mut |l, d| {
        let (b, _) = l.list_box(&[], 0, 3);
        if b != 0 {
            d.report = "Vertex groups are not available yet".into();
        }
    });
    l.panel(d, "data.shape_keys", "Shape Keys", true, &mut |l, d| {
        let (b, _) = l.list_box(&[], 0, 3);
        if b != 0 {
            d.report = "Shape keys are not available yet".into();
        }
    });
    l.panel(d, "data.uv_maps", "UV Maps", true, &mut |l, d| {
        let has_uv = d.scene.meshes[mi].faces.iter().any(|f| !f.uvs.is_empty());
        let rows: Vec<(Icon, String)> = if has_uv { vec![(Icon::Texture, "UVMap".into())] } else { Vec::new() };
        let (b, _) = l.list_box(&rows, 0, 3);
        if b != 0 {
            d.report = "Adding or removing UV maps is not available yet; use UV unwrapping in Edit Mode".into();
        }
    });
    l.panel(d, "data.color_attributes", "Color Attributes", false, &mut |l, d| {
        let (b, _) = l.list_box(&[], 0, 3);
        if b != 0 {
            d.report = "Color attributes are not available yet".into();
        }
    });
    l.panel(d, "data.attributes", "Attributes", false, &mut |l, d| {
        let m = &d.scene.meshes[mi];
        let mut rows = vec![(Icon::Mesh, format!("position    Vertex    Vector  ({})", m.vertex_count()))];
        if m.faces.iter().any(|f| f.smooth) {
            rows.push((Icon::Mesh, "sharp_face    Face    Boolean".into()));
        }
        if m.faces.iter().any(|f| f.material != 0) {
            rows.push((Icon::Mesh, "material_index    Face    Integer".into()));
        }
        let (b, _) = l.list_box(&rows, usize::MAX, 3);
        if b != 0 {
            d.report = "Custom attributes are not available yet".into();
        }
    });
    l.panel(d, "data.normals", "Normals", true, &mut |l, d| {
        let faces = &d.scene.meshes[mi].faces;
        let smooth = faces.iter().filter(|f| f.smooth).count() * 2 > faces.len();
        if let Some(i) = l.expand("Shading", &["Flat", "Smooth"], smooth as usize) {
            l.commit(d, if i == 1 { "Shade Smooth" } else { "Shade Flat" });
            for f in &mut d.scene.meshes[mi].faces {
                f.smooth = i == 1;
            }
        }
    });
    l.panel(d, "data.texture_space", "Texture Space", false, &mut |l, _| inactive(l, |l| {
        let mut t = true;
        l.check("", "Auto Texture Space", &mut t);
    }));
    l.panel(d, "data.remesh", "Remesh", false, &mut |l, _| inactive(l, |l| l.expand("", &["Voxel", "Quad"], 0)));
    l.panel(d, "data.geometry_data", "Geometry Data", false, &mut |l, d| {
        let m = &d.scene.meshes[mi];
        let (v, f) = (m.vertex_count(), m.face_count());
        l.info("Vertices", &v.to_string());
        l.info("Faces", &f.to_string());
    });
    l.panel(d, "data.custom_props", "Custom Properties", false, &mut |l, _| l.note("Not available yet"));
}

fn set_light(l: &mut L, d: &mut Doc, o: usize, nl: Light, label: &str) {
    l.commit(d, label);
    if let ObjectData::Light(li) = &mut d.scene.objects[o].data {
        *li = nl;
    }
}

fn light(l: &mut L, d: &mut Doc, c: &mut Ctx, o: usize, li: Light) {
    let name = d.scene.objects[o].name.clone();
    l.id_block(Icon::LightData, &name, None);
    l.panel(d, "data.light", "Light", true, &mut |l, d| {
        let cur = match li.kind {
            LightKind::Point => 0,
            LightKind::Sun => 1,
            LightKind::Spot { .. } => 2,
            LightKind::Area { .. } => 3,
        };
        if let Some(i) = l.expand("", &["Point", "Sun", "Spot", "Area"], cur) {
            let mut nl = li;
            nl.kind = match i {
                0 => LightKind::Point,
                1 => LightKind::Sun,
                2 => LightKind::Spot { angle: 45f32.to_radians(), blend: 0.15 },
                _ => LightKind::Area { size: 1.0 },
            };
            // Blender keeps the energy value; the defaults differ (1000 W vs 1).
            if i == 1 && li.power == 1000.0 {
                nl.power = 1.0;
            } else if cur == 1 && li.power == 1.0 {
                nl.power = 1000.0;
            }
            set_light(l, d, o, nl, "Light Type");
        }
        if let Some(r) = l.color("Color", [li.color.x, li.color.y, li.color.z]) {
            c.open_picker = Some((ColorTarget::Light(o), r));
        }
        let sun = matches!(li.kind, LightKind::Sun);
        let mut p = li.power;
        let (lab, unit) = if sun { ("Strength", "W/m²") } else { ("Power", "W") };
        if l.number(lab, "power", &mut p, NumOpts { step: if sun { 0.1 } else { 10.0 }, unit, min: 0.0, ..NumOpts::default() }) {
            set_light(l, d, o, Light { power: p, ..li }, "Light Power");
        }
        match li.kind {
            LightKind::Sun => {
                let mut a = li.radius.to_degrees();
                if l.number("Angle", "sun_angle", &mut a, NumOpts { step: 0.5, unit: "°", min: 0.0, max: 180.0, ..NumOpts::default() }) {
                    set_light(l, d, o, Light { radius: a.to_radians(), ..li }, "Sun Angle");
                }
            }
            LightKind::Area { size } => {
                let shape = match li.shape {
                    AreaShape::Square => "Square",
                    AreaShape::Rectangle => "Rectangle",
                    AreaShape::Disk => "Disk",
                    AreaShape::Ellipse => "Ellipse",
                };
                l.enumeration("Shape", "area_shape", shape, &["Square", "Rectangle", "Disk", "Ellipse"]);
                let two = matches!(li.shape, AreaShape::Rectangle | AreaShape::Ellipse);
                let mut s = size;
                if l.number(if two { "Size X" } else { "Size" }, "area_size", &mut s, M) {
                    set_light(l, d, o, Light { kind: LightKind::Area { size: s }, ..li }, "Area Size");
                }
                if two {
                    let mut s = li.size_y;
                    if l.number("Y", "area_size_y", &mut s, M) {
                        set_light(l, d, o, Light { size_y: s, ..li }, "Area Size Y");
                    }
                }
            }
            _ => {
                let mut r = li.radius;
                if l.number("Radius", "radius", &mut r, M) {
                    set_light(l, d, o, Light { radius: r, ..li }, "Light Radius");
                }
            }
        }
        let mut v = li.max_bounces;
        if l.uint("Max Bounces", "light_bounces", &mut v, 0, 1024) {
            set_light(l, d, o, Light { max_bounces: v, ..li }, "Light Max Bounces");
        }
        let mut t = li.cast_shadow;
        if l.check("", "Cast Shadow", &mut t) {
            set_light(l, d, o, Light { cast_shadow: t, ..li }, "Cast Shadow");
        }
        let mut t = li.use_mis;
        if l.check("", "Multiple Importance", &mut t) {
            set_light(l, d, o, Light { use_mis: t, ..li }, "Multiple Importance");
        }
    });
    if let LightKind::Spot { angle, blend } = li.kind {
        l.panel(d, "data.spot_shape", "Beam Shape", true, &mut |l, d| {
            let mut deg = angle.to_degrees();
            if l.number("Spot Size", "spot_size", &mut deg, NumOpts { step: 1.0, unit: "°", min: 1.0, max: 180.0, ..NumOpts::default() }) {
                set_light(l, d, o, Light { kind: LightKind::Spot { angle: deg.to_radians(), blend }, ..li }, "Spot Size");
            }
            let mut b = blend;
            if l.slider("Blend", "spot_blend", &mut b, 0.0, 1.0) {
                set_light(l, d, o, Light { kind: LightKind::Spot { angle, blend: b }, ..li }, "Spot Blend");
            }
            inactive(l, |l| {
                let mut f = false;
                l.check("", "Show Cone", &mut f);
            });
        });
    }
    if let LightKind::Area { .. } = li.kind {
        l.panel(d, "data.area_beam", "Beam Shape", true, &mut |l, d| {
            let mut deg = li.spread.to_degrees();
            if l.number("Spread", "area_spread", &mut deg, NumOpts { step: 1.0, unit: "°", min: 1.0, max: 180.0, ..NumOpts::default() }) {
                set_light(l, d, o, Light { spread: deg.to_radians(), ..li }, "Spread");
            }
        });
    }
    l.panel(d, "data.light.nodes", "Nodes", false, &mut |l, _| l.note("Light nodes are not available yet"));
    l.panel(d, "data.light.custom_props", "Custom Properties", false, &mut |l, _| l.note("Not available yet"));
}

/// Width-over-height of the render.
fn aspect(d: &Doc) -> f32 {
    d.settings.resolution_x.max(1) as f32 / d.settings.resolution_y.max(1) as f32
}

/// Blender's focal length for a vertical FOV with "Auto" sensor fit.
#[cfg(test)]
pub fn focal_from_fov_y(fov_y: f32, sensor: f32, aspect: f32) -> f32 {
    let t = (fov_y * 0.5).tan().max(1e-6);
    // Auto fit: the sensor spans the larger dimension.
    let tan_major = if aspect >= 1.0 { t * aspect } else { t };
    sensor * 0.5 / tan_major
}

fn set_camera(l: &mut L, d: &mut Doc, o: usize, nc: Camera, label: &str) {
    l.commit(d, label);
    if let ObjectData::Camera(c) = &mut d.scene.objects[o].data {
        *c = nc;
    }
}

const DEG: NumOpts<'static> = NumOpts { step: 1.0, unit: "°", min: -360.0, max: 360.0, slider: false, int: false };

fn camera(l: &mut L, d: &mut Doc, c: &mut Ctx, o: usize, cam: Camera) {
    let name = d.scene.objects[o].name.clone();
    let asp = aspect(d);
    l.id_block(Icon::CameraData, &name, None);
    l.panel(d, "data.lens", "Lens", true, &mut |l, d| {
        let ty = match cam.camera_type() {
            CameraType::Perspective => "Perspective",
            CameraType::Orthographic => "Orthographic",
            CameraType::Panoramic => "Panoramic",
        };
        l.enumeration("Type", "lens_type", ty, &["Perspective", "Orthographic", "Panoramic"]);
        match cam.camera_type() {
            CameraType::Perspective => {
                let f = cam.focal_length(asp);
                if c.fov_unit {
                    // The angle across the dimension the sensor fit picks.
                    let sensor = match cam.sensor_fit {
                        SensorFit::Vertical => cam.sensor_height,
                        _ => cam.sensor_width,
                    };
                    let mut deg = (2.0 * (sensor * 0.5 / f).atan()).to_degrees();
                    if l.number("Field of View", "fov", &mut deg, NumOpts { step: 1.0, unit: "°", min: 0.367, max: 172.847, ..NumOpts::default() }) {
                        let mut nc = cam;
                        nc.set_focal_length(sensor * 0.5 / (deg.to_radians() * 0.5).tan(), asp);
                        set_camera(l, d, o, nc, "Field of View");
                    }
                } else {
                    let mut f = f;
                    if l.number("Focal Length", "focal", &mut f, NumOpts { step: 1.0, unit: "mm", min: 1.0, max: 5000.0, ..NumOpts::default() }) {
                        let mut nc = cam;
                        nc.set_focal_length(f, asp);
                        set_camera(l, d, o, nc, "Focal Length");
                    }
                }
                l.enumeration("Lens Unit", "lens_unit", if c.fov_unit { "Field of View" } else { "Millimeters" }, &["Millimeters", "Field of View"]);
            }
            CameraType::Orthographic => {
                // Blender's scale spans the larger dimension; ours is the height.
                let h = cam.ortho_scale.unwrap_or(1.0);
                let k = asp.max(1.0);
                let mut s = h * k;
                if l.number("Orthographic Scale", "ortho_scale", &mut s, NumOpts { step: 0.1, min: 0.001, ..NumOpts::default() }) {
                    set_camera(l, d, o, Camera { ortho_scale: Some(s / k), ..cam }, "Orthographic Scale");
                }
            }
            CameraType::Panoramic => {
                let pano = cam.panorama.unwrap_or_default();
                let label = match pano {
                    PanoramaType::Equirectangular => "Equirectangular",
                    PanoramaType::FisheyeEquidistant => "Fisheye Equidistant",
                    PanoramaType::FisheyeEquisolid => "Fisheye Equisolid",
                };
                l.enumeration("Panorama Type", "panorama_type", label, &["Equirectangular", "Fisheye Equidistant", "Fisheye Equisolid"]);
                match pano {
                    PanoramaType::Equirectangular => {
                        let rows: [(&str, &str, fn(&Camera) -> f32, fn(&mut Camera, f32)); 4] = [
                            ("Latitude Min", "lat_min", |c| c.latitude_min, |c, v| c.latitude_min = v),
                            ("Max", "lat_max", |c| c.latitude_max, |c, v| c.latitude_max = v),
                            ("Longitude Min", "long_min", |c| c.longitude_min, |c, v| c.longitude_min = v),
                            ("Max", "long_max", |c| c.longitude_max, |c, v| c.longitude_max = v),
                        ];
                        for (n, k, get, set) in rows {
                            let mut v = get(&cam).to_degrees();
                            if l.number(n, k, &mut v, DEG) {
                                let mut nc = cam;
                                set(&mut nc, v.to_radians());
                                set_camera(l, d, o, nc, "Panorama Range");
                            }
                        }
                    }
                    PanoramaType::FisheyeEquidistant => {
                        let mut v = cam.fisheye_fov.to_degrees();
                        if l.number("Field of View", "fisheye_fov", &mut v, NumOpts { step: 1.0, unit: "°", min: 0.1, max: 360.0, ..NumOpts::default() }) {
                            set_camera(l, d, o, Camera { fisheye_fov: v.to_radians(), ..cam }, "Fisheye Field of View");
                        }
                    }
                    PanoramaType::FisheyeEquisolid => {
                        let mut v = cam.fisheye_lens;
                        if l.number("Lens", "fisheye_lens", &mut v, NumOpts { step: 0.5, unit: "mm", min: 0.01, max: 100.0, ..NumOpts::default() }) {
                            set_camera(l, d, o, Camera { fisheye_lens: v, ..cam }, "Fisheye Lens");
                        }
                        let mut v = cam.fisheye_fov.to_degrees();
                        if l.number("Field of View", "fisheye_fov", &mut v, NumOpts { step: 1.0, unit: "°", min: 0.1, max: 360.0, ..NumOpts::default() }) {
                            set_camera(l, d, o, Camera { fisheye_fov: v.to_radians(), ..cam }, "Fisheye Field of View");
                        }
                    }
                }
            }
        }
        l.space(4);
        let mut v = cam.shift_x;
        if l.number("Shift X", "shift_x", &mut v, NumOpts { step: 0.01, ..NumOpts::default() }) {
            set_camera(l, d, o, Camera { shift_x: v, ..cam }, "Shift X");
        }
        let mut v = cam.shift_y;
        if l.number("Y", "shift_y", &mut v, NumOpts { step: 0.01, ..NumOpts::default() }) {
            set_camera(l, d, o, Camera { shift_y: v, ..cam }, "Shift Y");
        }
        l.space(4);
        let mut n = cam.near;
        if l.number("Clip Start", "clip_start", &mut n, NumOpts { step: 0.01, unit: "m", min: 1e-6, ..NumOpts::default() }) {
            set_camera(l, d, o, Camera { near: n.min(cam.far), ..cam }, "Clip Start");
        }
        let mut f = cam.far;
        if l.number("End", "clip_end", &mut f, NumOpts { step: 1.0, unit: "m", min: 1e-6, ..NumOpts::default() }) {
            set_camera(l, d, o, Camera { far: f.max(cam.near), ..cam }, "Clip End");
        }
    });
    let dof = cam.dof;
    let mut hit = None;
    l.panel_with(d, "data.dof", "", false, &mut |l, _, r| {
        let mut v = dof.enabled;
        let tw = l.ui.p.text_width("Depth of Field") + 24;
        let cr = Rect::new(r.x - 4, r.y, tw.min(r.w + 4), r.h);
        if l.ui.checkbox(cr, "Depth of Field", &mut v) {
            hit = Some(v);
        }
    }, &mut |l, d| {
        let was = l.inactive;
        l.inactive = was || !dof.enabled;
        let objs: Vec<String> = d.scene.objects.iter().enumerate().filter(|(i, _)| *i != o).map(|(_, ob)| ob.name.clone()).collect();
        let mut items: Vec<&str> = vec!["None"];
        items.extend(objs.iter().map(|s| s.as_str()));
        let cur = dof.focus_object.and_then(|f| d.scene.objects.get(f)).map(|ob| ob.name.clone()).unwrap_or_default();
        l.enumeration("Focus on Object", "dof_focus_object", &cur, &items);
        let was2 = l.inactive;
        l.inactive = l.inactive || dof.focus_object.is_some();
        let mut v = dof.focus_distance;
        if l.number("Focus Distance", "dof_dist", &mut v, M) {
            set_camera(l, d, o, Camera { dof: DepthOfField { focus_distance: v, ..dof }, ..cam }, "Focus Distance");
        }
        l.inactive = was2;
        l.panel(d, "data.dof.aperture", "Aperture", true, &mut |l, d| {
            let mut v = dof.fstop;
            if l.number("F-Stop", "fstop", &mut v, NumOpts { step: 0.1, min: 0.1, max: 128.0, ..NumOpts::default() }) {
                set_camera(l, d, o, Camera { dof: DepthOfField { fstop: v, ..dof }, ..cam }, "F-Stop");
            }
            let mut v = dof.blades;
            if l.uint("Blades", "dof_blades", &mut v, 0, 16) {
                set_camera(l, d, o, Camera { dof: DepthOfField { blades: v, ..dof }, ..cam }, "Blades");
            }
            let mut v = dof.rotation.to_degrees();
            if l.number("Rotation", "dof_rotation", &mut v, NumOpts { step: 1.0, unit: "°", min: -180.0, max: 180.0, ..NumOpts::default() }) {
                set_camera(l, d, o, Camera { dof: DepthOfField { rotation: v.to_radians(), ..dof }, ..cam }, "Aperture Rotation");
            }
            let mut v = dof.ratio;
            if l.number("Ratio", "dof_ratio", &mut v, NumOpts { step: 0.05, min: 0.01, max: 2.0, ..NumOpts::default() }) {
                set_camera(l, d, o, Camera { dof: DepthOfField { ratio: v, ..dof }, ..cam }, "Aperture Ratio");
            }
        });
        l.inactive = was;
    });
    if let Some(v) = hit {
        set_camera(l, d, o, Camera { dof: DepthOfField { enabled: v, ..dof }, ..cam }, "Depth of Field");
    }
    l.panel(d, "data.camera", "Camera", false, &mut |l, d| {
        let fit = match cam.sensor_fit {
            SensorFit::Auto => "Auto",
            SensorFit::Horizontal => "Horizontal",
            SensorFit::Vertical => "Vertical",
        };
        l.enumeration("Sensor Fit", "sensor_fit", fit, &["Auto", "Horizontal", "Vertical"]);
        // Keep the focal length: the field of view follows the sensor.
        let f = cam.focal_length(asp);
        let vertical = cam.sensor_fit == SensorFit::Vertical;
        let mut s = if vertical { cam.sensor_height } else { cam.sensor_width };
        let label = match cam.sensor_fit {
            SensorFit::Auto => "Size",
            SensorFit::Horizontal => "Width",
            SensorFit::Vertical => "Height",
        };
        if l.number(label, "sensor", &mut s, NumOpts { step: 1.0, unit: "mm", min: 1.0, max: 100.0, ..NumOpts::default() }) {
            let mut nc = cam;
            if vertical {
                nc.sensor_height = s;
            } else {
                nc.sensor_width = s;
            }
            nc.set_focal_length(f, asp);
            set_camera(l, d, o, nc, "Sensor Size");
        }
    });
    for (k, tt) in [("data.safe_areas", "Safe Areas"), ("data.bg_images", "Background Images"), ("data.cam_display", "Viewport Display")] {
        l.panel(d, k, tt, false, &mut |l, _| l.note("Not available yet"));
    }
}

pub fn material(l: &mut L, d: &mut Doc, c: &mut Ctx) {
    let Some(o) = c.obj else { return };
    let slots = d.scene.objects[o].materials.clone();
    if !slots.is_empty() {
        c.mat_slot = c.mat_slot.min(slots.len() - 1);
    } else {
        c.mat_slot = 0;
    }
    // Slot list.
    l.space(4);
    let rows: Vec<(Icon, String)> = slots.iter().map(|&m| (Icon::Material, d.scene.materials.get(m).map(|m| m.name.clone()).unwrap_or_default())).collect();
    let (b, picked) = l.list_box(&rows, c.mat_slot, 3);
    if let Some(p) = picked {
        c.mat_slot = p;
    }
    match b {
        1 => {
            // Blender adds an empty slot; slots here always hold a material,
            // so + adds a new one.
            l.commit(d, "Add Material Slot");
            let name = sex_3d::scene::unique_name("Material", |n| d.scene.materials.iter().any(|m| m.name == n));
            let mi = d.scene.add_material(Material { name, ..Material::default() });
            d.scene.objects[o].materials.push(mi);
            c.mat_slot = d.scene.objects[o].materials.len() - 1;
        }
        2 if !slots.is_empty() => {
            l.commit(d, "Remove Material Slot");
            d.scene.objects[o].materials.remove(c.mat_slot);
            c.mat_slot = c.mat_slot.saturating_sub(1);
        }
        _ => {}
    }
    let slots = d.scene.objects[o].materials.clone();
    // Data-block row: browse, name, unlink; or "New".
    if l.band(ROW) {
        let (x, w) = l.inner();
        match slots.get(c.mat_slot).copied() {
            None => {
                if l.ui.button(Rect::new(x, l.y, w, ROW), "New") {
                    l.commit(d, "New Material");
                    let name = sex_3d::scene::unique_name("Material", |n| d.scene.materials.iter().any(|m| m.name == n));
                    let mi = d.scene.add_material(Material { name, ..Material::default() });
                    d.scene.objects[o].materials.push(mi);
                    c.mat_slot = 0;
                }
            }
            Some(mi) => {
                let br = Rect::new(x, l.y, 40, ROW);
                if l.ui.dropdown(br, "") {
                    let menu = d.scene.materials.iter().map(|m| MenuItem::op(&m.name, "", Op::with("properties.set", &format!("material={}", m.name)))).collect();
                    l.ui.open_menu(br, "Browse Material", menu);
                }
                icons::draw(l.ui.p, Icon::Material, br.x + 13, br.y + ROW / 2);
                let xr = Rect::new(x + w - 22, l.y, 22, ROW);
                let nr = Rect::new(br.right() + 2, l.y, xr.x - br.right() - 4, ROW);
                let mut name = d.scene.materials[mi].name.clone();
                let wid = l.wid("mat_name");
                if l.ui.text_field(wid, nr, &mut name) && !name.trim().is_empty() {
                    l.commit(d, "Rename Material");
                    let n = sex_3d::scene::unique_name(name.trim(), |n| d.scene.materials.iter().enumerate().any(|(i, m)| i != mi && m.name == n));
                    d.scene.materials[mi].name = n;
                }
                let hov = l.ui.hovered(xr);
                l.ui.p.round_rect(xr, 4, if hov { t::BUTTON_HOVER } else { t::BUTTON });
                icons::draw(l.ui.p, Icon::Cross, xr.x + 11, xr.y + ROW / 2);
                if l.ui.clicked(xr) {
                    l.commit(d, "Unlink Material");
                    d.scene.objects[o].materials.remove(c.mat_slot);
                    c.mat_slot = c.mat_slot.saturating_sub(1);
                }
            }
        }
        l.y += ROW + 6 + GAP;
    }
    let Some(mi) = d.scene.objects[o].materials.get(c.mat_slot).copied() else { return };
    if mi >= d.scene.materials.len() {
        return;
    }
    l.panel(d, "material.preview", "Preview", false, &mut |l, d| {
        if l.band(120) {
            let (x, w) = l.inner();
            preview_sphere(l, &d.scene.materials[mi], Rect::new(x + w / 2 - 55, l.y + 5, 110, 110));
            l.y += 120 + GAP;
        }
    });
    let has_tree = d.scene.materials[mi].node_tree.is_some();
    if has_tree {
        super::nodepanel::material(l, d, &mut c.open_picker, mi);
    }
    if !has_tree {
    l.panel(d, "material.surface", "Surface", true, &mut |l, d| {
        let m = d.scene.materials[mi].clone();
        super::nodepanel::use_nodes_button(l, d, crate::nodetree::TreeRef::Material(mi));
        l.enumeration("Surface", "surface", "Principled BSDF", &["Principled BSDF"]);
        if let Some(ti) = m.base_color_texture {
            let n = d.scene.images.get(ti).map(|i| i.name.clone()).unwrap_or_default();
            l.info("Base Color", &format!("Image Texture: {n}"));
        }
        if let Some(r) = l.color(if m.base_color_texture.is_some() { "Tint" } else { "Base Color" }, [m.base_color[0], m.base_color[1], m.base_color[2]]) {
            c.open_picker = Some((ColorTarget::BaseColor(mi), r));
        }
        let edit = |l: &mut L, d: &mut Doc, label: &str, f: &dyn Fn(&mut Material)| {
            l.commit(d, label);
            f(&mut d.scene.materials[mi]);
        };
        let mut v = m.metallic;
        if l.slider("Metallic", "metallic", &mut v, 0.0, 1.0) {
            edit(l, d, "Metallic", &|mm| mm.metallic = v);
        }
        let mut v = m.roughness;
        if l.slider("Roughness", "roughness", &mut v, 0.0, 1.0) {
            edit(l, d, "Roughness", &|mm| mm.roughness = v);
        }
        let mut v = m.ior;
        if l.number("IOR", "ior", &mut v, NumOpts { step: 0.01, min: 1.0, max: 1000.0, ..NumOpts::default() }) {
            edit(l, d, "IOR", &|mm| mm.ior = v);
        }
        let mut v = m.base_color[3];
        if l.slider("Alpha", "alpha", &mut v, 0.0, 1.0) {
            edit(l, d, "Alpha", &|mm| mm.base_color[3] = v);
        }
        inactive(l, |l| l.info("Normal", "Default"));
        l.panel(d, "material.surface.subsurface", "Subsurface", false, &mut |l, _| {
            inactive(l, |l| {
                let mut z = 0.0;
                l.slider("Weight", "ss_w", &mut z, 0.0, 1.0);
            });
        });
        l.panel(d, "material.surface.specular", "Specular", false, &mut |l, _| {
            inactive(l, |l| {
                let mut h = 0.5;
                l.slider("IOR Level", "spec_lvl", &mut h, 0.0, 1.0);
                l.color("Tint", [1.0, 1.0, 1.0]);
                let mut z = 0.0;
                l.slider("Anisotropic", "aniso", &mut z, 0.0, 1.0);
            });
        });
        l.panel(d, "material.surface.transmission", "Transmission", false, &mut |l, d| {
            let mut v = d.scene.materials[mi].transmission;
            if l.slider("Weight", "transmission", &mut v, 0.0, 1.0) {
                l.commit(d, "Transmission");
                d.scene.materials[mi].transmission = v;
            }
        });
        l.panel(d, "material.surface.coat", "Coat", false, &mut |l, _| {
            inactive(l, |l| {
                let mut z = 0.0;
                l.slider("Weight", "coat_w", &mut z, 0.0, 1.0);
            });
        });
        l.panel(d, "material.surface.sheen", "Sheen", false, &mut |l, _| {
            inactive(l, |l| {
                let mut z = 0.0;
                l.slider("Weight", "sheen_w", &mut z, 0.0, 1.0);
            });
        });
        l.panel(d, "material.surface.emission", "Emission", false, &mut |l, d| {
            let e = d.scene.materials[mi].emission;
            if let Some(r) = l.color("Color", [e.x, e.y, e.z]) {
                c.open_picker = Some((ColorTarget::Emission(mi), r));
            }
            let mut v = d.scene.materials[mi].emission_strength;
            if l.number("Strength", "emission_strength", &mut v, NumOpts { step: 0.1, min: 0.0, max: 1e6, ..NumOpts::default() }) {
                l.commit(d, "Emission Strength");
                d.scene.materials[mi].emission_strength = v;
            }
        });
        l.panel(d, "material.surface.thin_film", "Thin Film", false, &mut |l, _| {
            inactive(l, |l| {
                let mut z = 0.0;
                l.number("Thickness", "tf", &mut z, NumOpts::default());
            });
        });
    });
    }
    if !has_tree {
        l.panel(d, "material.volume", "Volume", false, &mut |l, _| l.info("Volume", "None"));
        l.panel(d, "material.displacement", "Displacement", false, &mut |l, _| l.info("Displacement", "Default"));
    }
    l.panel(d, "material.settings", "Settings", false, &mut |l, d| {
        let mut v = !d.scene.materials[mi].double_sided;
        if l.check("", "Backface Culling", &mut v) {
            l.commit(d, "Backface Culling");
            d.scene.materials[mi].double_sided = !v;
        }
        inactive(l, |l| {
            l.enumeration("Blend Mode", "blend", "Dithered", &[]);
            l.enumeration("Shadow Mode", "shadow", "Opaque", &[]);
        });
    });
    l.panel(d, "material.line_art", "Line Art", false, &mut |l, _| l.note("Not available yet"));
    l.panel(d, "material.viewport_display", "Viewport Display", false, &mut |l, d| {
        let m = &d.scene.materials[mi];
        let (bc, mut me, mut ro) = ([m.base_color[0], m.base_color[1], m.base_color[2]], m.metallic, m.roughness);
        inactive(l, |l| {
            l.color("Color", bc);
            l.slider("Metallic", "vd_metal", &mut me, 0.0, 1.0);
            l.slider("Roughness", "vd_rough", &mut ro, 0.0, 1.0);
        });
    });
}

/// A small lit sphere approximating the material (Blender's Preview panel).
fn preview_sphere(l: &mut L, m: &Material, r: Rect) {
    l.ui.p.round_rect(r, 6, 0x2a2a2a);
    let (cx, cy, rad) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0, r.w as f32 * 0.42);
    let light = sex_3d::Vec3::new(-0.5, -0.6, 0.65).normalize();
    let shininess = 2.0 + (1.0 - m.roughness).powi(2) * 200.0;
    for py in (cy - rad) as i32..=(cy + rad) as i32 {
        for px in (cx - rad) as i32..=(cx + rad) as i32 {
            let (dx, dy) = ((px as f32 + 0.5 - cx) / rad, (py as f32 + 0.5 - cy) / rad);
            let rr = dx * dx + dy * dy;
            if rr > 1.0 {
                continue;
            }
            let n = sex_3d::Vec3::new(dx, dy, (1.0 - rr).sqrt());
            let diff = n.dot(light).max(0.0) * 0.85 + 0.15;
            let h = (light + sex_3d::Vec3::Z).normalize();
            let spec = n.dot(h).max(0.0).powf(shininess) * (0.04 + 0.96 * m.metallic) * (1.0 - m.roughness * 0.7);
            let mut col = [0u32; 3];
            for i in 0..3 {
                let base = m.base_color[i] * (1.0 - m.metallic * 0.5);
                let tint = if m.metallic > 0.5 { m.base_color[i] } else { 1.0 };
                let v = base * diff + spec * tint + m.emission.to_array()[i] * m.emission_strength;
                col[i] = (sex_3d::render::srgb_encode(v.clamp(0.0, 1.0)) * 255.0) as u32;
            }
            l.ui.p.fill(Rect::new(px, py, 1, 1), col[0] << 16 | col[1] << 8 | col[2]);
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_camera_is_50mm() {
        let c = Camera::default();
        let f = focal_from_fov_y(c.fov_y, 36.0, 16.0 / 9.0);
        assert!((f - 50.0).abs() < 0.01, "{f}");
        assert!((c.focal_length(16.0 / 9.0) - f).abs() < 0.01);
        let mut back = c;
        back.set_focal_length(f, 16.0 / 9.0);
        assert!((back.fov_y - c.fov_y).abs() < 1e-5);
    }
}
