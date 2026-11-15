// SPDX-License-Identifier: GPL-3.0-only
//! The scene-level tabs: Tool, Render, Output, View Layer, Scene, World,
//! Collection, and the placeholders Particles, Physics, Object Constraints and
//! Texture. Settings sex-3d cannot honour yet are shown greyed (inactive), as
//! Blender greys properties that do not apply.
use super::icons::Icon;
use super::layout::{L, ROW};
use super::{ColorTarget, Ctx};
use crate::doc::Doc;
use crate::draw::Rect;
use crate::settings::{Engine, UnitSystem};
use crate::ui::{MenuItem, NumOpts, Op};
use sex_3d::render::ViewTransform;
use sex_3d::scene::ObjectData;

fn inactive<R>(l: &mut L, f: impl FnOnce(&mut L) -> R) {
    let was = l.inactive;
    l.inactive = true;
    let _ = f(l);
    l.inactive = was;
}

/// A sub-panel whose header carries a checkbox before its title (Denoise,
/// Gravity, ...). Returns the new checkbox value when it was clicked.
fn check_header(l: &mut L, title: &str, value: bool, r: Rect) -> Option<bool> {
    let mut v = value;
    let tw = l.ui.p.text_width(title) + 24;
    let cr = Rect::new(r.x - 4, r.y, tw.min(r.w + 4), r.h);
    l.ui.checkbox(cr, title, &mut v).then_some(v)
}

pub fn tool(l: &mut L, d: &mut Doc) {
    l.panel(d, "tool.active", "Active Tool", true, &mut |l, _| {
        l.info("", "Select Box");
        inactive(l, |l| {
            l.expand("Mode", &["Set", "Extend", "Subtract", "Invert", "Intersect"], 0);
        });
    });
    l.panel(d, "tool.options", "Options", false, &mut |l, _| {
        inactive(l, |l| {
            let mut f = false;
            l.check("Transform Affect Only", "Origins", &mut f);
            l.check("", "Locations", &mut f);
            l.check("", "Parents", &mut f);
        });
    });
    l.panel(d, "tool.workspace", "Workspace", false, &mut |l, d| {
        l.info("Mode", d.mode.label());
    });
}

pub fn render(l: &mut L, d: &mut Doc) {
    let engine = d.settings.engine;
    l.space(4);
    l.enumeration("Render Engine", "engine", engine.label(), &Engine::ALL.map(|e| e.label()));
    match engine {
        Engine::PathTracer => {
            l.enumeration("Device", "device", "CPU", &["CPU"]);
            l.space(4);
            l.panel(d, "render.sampling", "Sampling", true, &mut |l, d| {
                l.panel(d, "render.sampling.viewport", "Viewport", true, &mut |l, d| {
                    inactive(l, |l| {
                        let mut v = d.settings.noise_threshold;
                        l.check("Noise Threshold", "", &mut v);
                    });
                    let mut v = d.settings.viewport_samples;
                    if l.uint("Max Samples", "vp_samples", &mut v, 1, 65536) {
                        l.commit(d, "Viewport Samples");
                        d.settings.viewport_samples = v;
                    }
                    let mut v = d.settings.viewport_denoise;
                    if l.check("", "Denoise", &mut v) {
                        l.commit(d, "Viewport Denoise");
                        d.settings.viewport_denoise = v;
                    }
                });
                l.panel(d, "render.sampling.render", "Render", true, &mut |l, d| {
                    inactive(l, |l| {
                        let mut v = d.settings.noise_threshold;
                        l.check("Noise Threshold", "", &mut v);
                    });
                    let mut v = d.settings.render_samples;
                    if l.uint("Max Samples", "render_samples", &mut v, 1, 65536) {
                        l.commit(d, "Render Samples");
                        d.settings.render_samples = v;
                    }
                    inactive(l, |l| {
                        let mut t = 0.0;
                        l.number("Time Limit", "time_limit", &mut t, NumOpts { unit: "s", ..NumOpts::default() });
                    });
                    let on = d.settings.denoise;
                    let mut hit = None;
                    l.panel_with(d, "render.sampling.render.denoise", "", true, &mut |l, _, r| hit = check_header(l, "Denoise", on, r), &mut |l, d| {
                        let was = l.inactive;
                        l.inactive = !d.settings.denoise;
                        l.info("Denoiser", "Built-in");
                        l.info("Passes", "Albedo and Normal");
                        l.inactive = was;
                    });
                    if let Some(v) = hit {
                        l.commit(d, "Denoise");
                        d.settings.denoise = v;
                    }
                    let mut seed = d.settings.seed;
                    if l.uint("Seed", "seed", &mut seed, 0, u32::MAX / 2) {
                        l.commit(d, "Seed");
                        d.settings.seed = seed;
                    }
                });
            });
            l.panel(d, "render.light_paths", "Light Paths", false, &mut |l, d| {
                l.panel(d, "render.light_paths.bounces", "Max Bounces", true, &mut |l, d| {
                    let mut v = d.settings.max_bounces;
                    if l.uint("Total", "bounces", &mut v, 0, 1024) {
                        l.commit(d, "Max Bounces");
                        d.settings.max_bounces = v;
                    }
                    inactive(l, |l| {
                        for (n, k) in [("Diffuse", 4u32), ("Glossy", 4), ("Transmission", 12), ("Volume", 0), ("Transparent", 8)] {
                            let mut v = k;
                            l.uint(n, n, &mut v, 0, 1024);
                        }
                    });
                });
                l.panel(d, "render.light_paths.clamping", "Clamping", true, &mut |l, d| {
                    inactive(l, |l| {
                        let mut v = 0.0;
                        l.number("Direct Light", "clamp_direct", &mut v, NumOpts { min: 0.0, ..NumOpts::default() });
                    });
                    let mut v = d.settings.clamp_indirect;
                    if l.number("Indirect Light", "clamp_indirect", &mut v, NumOpts { step: 0.5, min: 0.0, ..NumOpts::default() }) {
                        l.commit(d, "Clamp Indirect");
                        d.settings.clamp_indirect = v;
                    }
                });
            });
            l.panel(d, "render.film", "Film", false, &mut |l, d| {
                let mut v = d.settings.exposure;
                if l.number("Exposure", "film_exposure", &mut v, NumOpts { step: 0.1, min: -10.0, max: 10.0, ..NumOpts::default() }) {
                    l.commit(d, "Exposure");
                    d.settings.exposure = v;
                }
                inactive(l, |l| {
                    l.info("Pixel Filter", "Blackman-Harris");
                    let mut w = 1.5;
                    l.number("Width", "filter_w", &mut w, NumOpts { unit: "px", ..NumOpts::default() });
                    let mut t = false;
                    l.check("", "Transparent", &mut t);
                });
            });
            l.panel(d, "render.performance", "Performance", false, &mut |l, _| {
                l.info("Threads", "Auto-Detect");
                l.info("Tiles", "Whole frame");
            });
        }
        Engine::Workbench => {
            l.space(4);
            l.panel(d, "render.wb.sampling", "Sampling", true, &mut |l, _| {
                l.info("Render", "1 Sample (direct light)");
                inactive(l, |l| l.enumeration("Viewport", "wb_vp", "8 Samples", &[]));
            });
            l.panel(d, "render.wb.lighting", "Lighting", true, &mut |l, _| {
                inactive(l, |l| {
                    l.expand("", &["Studio", "Flat", "MatCap"], 0);
                });
            });
            l.panel(d, "render.wb.color", "Color", true, &mut |l, _| {
                inactive(l, |l| {
                    l.expand("", &["Material", "Object", "Attribute", "Single", "Random", "Texture"], 0);
                });
            });
            l.panel(d, "render.wb.film", "Film", false, &mut |l, _| {
                inactive(l, |l| {
                    let mut t = false;
                    l.check("", "Transparent", &mut t);
                });
            });
        }
    }
    l.panel(d, "render.color_management", "Color Management", false, &mut |l, d| {
        l.info("Display Device", "sRGB");
        let vt = match d.settings.view_transform {
            ViewTransform::Standard => "Standard",
            ViewTransform::AgX => "AgX",
        };
        l.enumeration("View Transform", "view_transform", vt, &["Standard", "Khronos PBR Neutral", "AgX", "Filmic", "Filmic Log", "False Color", "Raw"]);
        let look = d.settings.look.clone();
        l.enumeration("Look", "look", &look, &["None", "Punchy", "Very High Contrast", "High Contrast", "Medium High Contrast", "Base Contrast", "Medium Low Contrast", "Low Contrast", "Very Low Contrast"]);
        let mut v = d.settings.exposure;
        if l.number("Exposure", "cm_exposure", &mut v, NumOpts { step: 0.1, min: -32.0, max: 32.0, ..NumOpts::default() }) {
            l.commit(d, "Exposure");
            d.settings.exposure = v;
        }
        let mut v = d.settings.gamma;
        if l.number("Gamma", "cm_gamma", &mut v, NumOpts { step: 0.05, min: 0.0, max: 5.0, ..NumOpts::default() }) {
            l.commit(d, "Gamma");
            d.settings.gamma = v;
        }
        l.info("Sequencer", "sRGB");
        l.panel(d, "render.color_management.curves", "Curves", false, &mut |l, _| l.note("Not available yet"));
    });
}

pub const FPS: [&str; 10] = ["23.98 fps", "24 fps", "25 fps", "29.97 fps", "30 fps", "50 fps", "59.94 fps", "60 fps", "120 fps", "240 fps"];

pub fn output(l: &mut L, d: &mut Doc) {
    l.panel(d, "output.format", "Format", true, &mut |l, d| {
        let mut v = d.settings.resolution_x;
        if l.number_u32("Resolution X", "res_x", &mut v, "px") {
            l.commit(d, "Resolution X");
            d.settings.resolution_x = v;
        }
        let mut v = d.settings.resolution_y;
        if l.number_u32("Y", "res_y", &mut v, "px") {
            l.commit(d, "Resolution Y");
            d.settings.resolution_y = v;
        }
        let mut v = d.settings.resolution_pct as f32;
        if l.number("%", "res_pct", &mut v, NumOpts { step: 1.0, unit: "%", min: 1.0, max: 32767.0, int: true, ..NumOpts::default() }) {
            l.commit(d, "Resolution %");
            d.settings.resolution_pct = v as u32;
        }
        l.space(4);
        inactive(l, |l| {
            let mut a = 1.0;
            l.number("Aspect X", "aspect_x", &mut a, NumOpts::default());
            l.number("Y", "aspect_y", &mut a, NumOpts::default());
            let mut f = false;
            l.check("", "Render Region", &mut f);
            l.check("", "Crop to Render Region", &mut f);
        });
        l.space(4);
        let fps = d.scene.fps;
        let cur = FPS.iter().find(|f| f.split(' ').next().and_then(|n| n.parse::<f32>().ok()).is_some_and(|n| (n - fps).abs() < 0.01)).map(|s| s.to_string()).unwrap_or_else(|| format!("Custom ({fps} fps)"));
        l.enumeration("Frame Rate", "fps", &cur, &FPS);
    });
    l.panel(d, "output.frame_range", "Frame Range", true, &mut |l, d| {
        let mut v = d.scene.frame_start as i64;
        if l.int("Frame Start", "frame_start", &mut v, -1_048_574, 1_048_574) {
            l.commit(d, "Frame Start");
            d.scene.frame_start = v as i32;
            d.scene.frame_end = d.scene.frame_end.max(d.scene.frame_start);
        }
        let mut v = d.scene.frame_end as i64;
        if l.int("End", "frame_end", &mut v, -1_048_574, 1_048_574) {
            l.commit(d, "Frame End");
            d.scene.frame_end = v as i32;
            d.scene.frame_start = d.scene.frame_start.min(d.scene.frame_end);
        }
        let mut v = d.settings.frame_step as i64;
        if l.int("Step", "frame_step", &mut v, 1, 1_048_574) {
            l.commit(d, "Frame Step");
            d.settings.frame_step = v as i32;
        }
        l.panel(d, "output.frame_range.time_stretching", "Time Stretching", false, &mut |l, _| {
            inactive(l, |l| {
                let mut v = 100;
                l.int("Old", "ts_old", &mut v, 1, 900);
                l.int("New", "ts_new", &mut v, 1, 900);
            });
        });
    });
    l.panel(d, "output.stereoscopy", "Stereoscopy", false, &mut |l, _| {
        inactive(l, |l| l.expand("", &["Stereo 3D", "Multi-View"], 0));
    });
    l.panel(d, "output.output", "Output", true, &mut |l, d| {
        let mut p = d.settings.output_path.clone();
        if l.text("", "out_path", &mut p) {
            l.commit(d, "Output Path");
            d.settings.output_path = p;
        }
        l.space(4);
        let mut b = true;
        inactive(l, |l| {
            l.check("Saving", "File Extensions", &mut b);
            let mut c = false;
            l.check("", "Cache Result", &mut c);
        });
        let ff = d.settings.file_format.clone();
        l.enumeration("File Format", "file_format", &ff, &["PNG", "JPEG", "BMP", "OpenEXR", "TIFF", "WebP"]);
        let modes = ["BW", "RGB", "RGBA"];
        let cur = modes.iter().position(|m| *m == d.settings.color_mode).unwrap_or(2);
        if let Some(i) = l.expand("Color", &modes, cur) {
            l.commit(d, "Color Mode");
            d.settings.color_mode = modes[i].into();
        }
        inactive(l, |l| {
            l.expand("Color Depth", &["8", "16"], 0);
            let mut c = 15.0;
            l.number("Compression", "compression", &mut c, NumOpts { unit: "%", int: true, ..NumOpts::default() });
            let mut o = true;
            l.check("Image Sequence", "Overwrite", &mut o);
            let mut p = false;
            l.check("", "Placeholders", &mut p);
        });
    });
    l.panel(d, "output.metadata", "Metadata", false, &mut |l, _| {
        inactive(l, |l| {
            let mut t = true;
            for n in ["Date", "Time", "Render Time", "Frame", "Frame Range", "Memory", "Hostname", "Camera", "Lens", "Scene"] {
                l.check("", n, &mut t);
            }
        });
    });
    l.panel(d, "output.post_processing", "Post Processing", false, &mut |l, _| {
        inactive(l, |l| {
            let mut t = true;
            l.check("Pipeline", "Compositing", &mut t);
            l.check("", "Sequencer", &mut t);
            let mut v = 1.0;
            l.number("Dither", "dither", &mut v, NumOpts::default());
        });
    });
}

pub fn view_layer(l: &mut L, d: &mut Doc) {
    l.panel(d, "vl.view_layer", "View Layer", true, &mut |l, d| {
        let mut v = d.settings.use_for_rendering;
        if l.check("", "Use for Rendering", &mut v) {
            l.commit(d, "Use for Rendering");
            d.settings.use_for_rendering = v;
        }
        inactive(l, |l| {
            let mut f = false;
            l.check("", "Render Single Layer", &mut f);
        });
    });
    l.panel(d, "vl.passes", "Passes", true, &mut |l, d| {
        l.panel(d, "vl.passes.data", "Data", true, &mut |l, d| {
            inactive(l, |l| {
                let mut t = true;
                l.check("Include", "Combined", &mut t);
            });
            let st = &d.settings;
            let (mut z, mut n, mut dn) = (st.pass_z, st.pass_normal, st.pass_denoising);
            if l.check("", "Z", &mut z) {
                l.commit(d, "Z Pass");
                d.settings.pass_z = z;
            }
            inactive(l, |l| {
                let mut f = false;
                l.check("", "Mist", &mut f);
                l.check("", "Position", &mut f);
            });
            if l.check("", "Normal", &mut n) {
                l.commit(d, "Normal Pass");
                d.settings.pass_normal = n;
            }
            if l.check("", "Denoising Data", &mut dn) {
                l.commit(d, "Denoising Data");
                d.settings.pass_denoising = dn;
            }
        });
        l.panel(d, "vl.passes.light", "Light", false, &mut |l, _| {
            inactive(l, |l| {
                let mut f = false;
                for n in ["Diffuse", "Glossy", "Transmission", "Emission", "Environment", "Ambient Occlusion", "Shadow Catcher"] {
                    l.check("", n, &mut f);
                }
            });
        });
        l.panel(d, "vl.passes.cryptomatte", "Cryptomatte", false, &mut |l, _| {
            inactive(l, |l| {
                let mut f = false;
                l.check("Include", "Object", &mut f);
                l.check("", "Material", &mut f);
                l.check("", "Asset", &mut f);
            });
        });
    });
    l.panel(d, "vl.filter", "Filter", false, &mut |l, _| {
        inactive(l, |l| {
            let mut t = true;
            for n in ["Environment", "Surfaces", "Curves", "Volumes", "Motion Blur"] {
                l.check("", n, &mut t);
            }
        });
    });
}

pub fn scene(l: &mut L, d: &mut Doc) {
    l.panel(d, "scene.scene", "Scene", true, &mut |l, d| {
        let cams: Vec<String> = d.scene.objects.iter().filter(|o| matches!(o.data, ObjectData::Camera(_))).map(|o| o.name.clone()).collect();
        let cur = d.scene.active_camera.and_then(|c| d.scene.objects.get(c)).map(|o| o.name.clone()).unwrap_or_default();
        let items: Vec<&str> = cams.iter().map(|s| s.as_str()).collect();
        l.enumeration("Camera", "camera", &cur, &items);
        inactive(l, |l| {
            l.enumeration("Background Scene", "bg_scene", "", &[]);
            l.enumeration("Active Clip", "clip", "", &[]);
        });
    });
    l.panel(d, "scene.units", "Units", true, &mut |l, d| {
        let us = d.settings.unit_system;
        l.enumeration("Unit System", "units", us.label(), &UnitSystem::ALL.map(|u| u.label()));
        let mut v = d.settings.unit_scale;
        if l.number("Unit Scale", "unit_scale", &mut v, NumOpts { step: 0.01, min: 0.00001, max: 100000.0, ..NumOpts::default() }) {
            l.commit(d, "Unit Scale");
            d.settings.unit_scale = v;
        }
        inactive(l, |l| {
            let mut f = false;
            l.check("", "Separate Units", &mut f);
        });
        let rot = if d.settings.degrees { "Degrees" } else { "Radians" };
        l.enumeration("Rotation", "rotation_units", rot, &["Degrees", "Radians"]);
        inactive(l, |l| {
            l.enumeration("Length", "length_unit", "Meters", &[]);
            l.enumeration("Mass", "mass_unit", "Kilograms", &[]);
            l.enumeration("Time", "time_unit", "Seconds", &[]);
            l.enumeration("Temperature", "temp_unit", "Kelvin", &[]);
        });
    });
    let on = d.settings.gravity_on;
    let mut hit = None;
    l.panel_with(d, "scene.gravity", "", true, &mut |l, _, r| hit = check_header(l, "Gravity", on, r), &mut |l, d| {
        let was = l.inactive;
        l.inactive = !d.settings.gravity_on;
        for (i, n) in ["Gravity X", "Y", "Z"].iter().enumerate() {
            let mut v = d.settings.gravity[i];
            if l.number(n, &format!("gravity{i}"), &mut v, NumOpts { step: 0.1, unit: "m/s²", ..NumOpts::default() }) {
                l.commit(d, "Gravity");
                d.settings.gravity[i] = v;
            }
        }
        l.inactive = was;
    });
    if let Some(v) = hit {
        l.commit(d, "Gravity");
        d.settings.gravity_on = v;
    }
    for (k, t) in [("scene.simulation", "Simulation"), ("scene.audio", "Audio"), ("scene.keying_sets", "Keying Sets"), ("scene.rigid_body", "Rigid Body World")] {
        l.panel(d, k, t, false, &mut |l, _| l.note("Not available yet"));
    }
}

pub fn world(l: &mut L, d: &mut Doc, c: &mut Ctx) {
    l.id_block(Icon::World, "World", None);
    if d.scene.world.node_tree.is_some() {
        super::nodepanel::world(l, d, &mut c.open_picker);
    } else {
        world_legacy(l, d, c);
    }
    l.panel(d, "world.viewport_display", "Viewport Display", false, &mut |l, d| {
        let w = d.scene.world.color;
        if let Some(r) = l.color("Color", [w.x, w.y, w.z]) {
            c.open_picker = Some((ColorTarget::World, r));
        }
    });
}

/// The World tab without a node tree: colour, strength and Use Nodes.
fn world_legacy(l: &mut L, d: &mut Doc, c: &mut Ctx) {
    l.panel(d, "world.surface", "Surface", true, &mut |l, d| {
        super::nodepanel::use_nodes_button(l, d, crate::nodetree::TreeRef::World);
        l.info("Surface", "Background");
        match d.scene.world.hdri {
            Some(i) => {
                let name = d.scene.images.get(i).map(|im| im.name.clone()).unwrap_or_default();
                l.info("Color", &format!("Environment Texture: {name}"));
            }
            None => {
                let w = d.scene.world.color;
                if let Some(r) = l.color("Color", [w.x, w.y, w.z]) {
                    c.open_picker = Some((ColorTarget::World, r));
                }
            }
        }
        let mut v = d.scene.world.strength;
        if l.number("Strength", "world_strength", &mut v, NumOpts { step: 0.1, min: 0.0, max: 1e6, ..NumOpts::default() }) {
            l.commit(d, "World Strength");
            d.scene.world.strength = v;
        }
    });
    l.panel(d, "world.volume", "Volume", false, &mut |l, _| l.info("Volume", "None"));
}

pub fn collection(l: &mut L, d: &mut Doc) {
    let ci = d.settings.active_collection.min(d.scene.collections.len().saturating_sub(1));
    if d.scene.collections.is_empty() {
        l.note("No collection");
        return;
    }
    let name = d.scene.collections[ci].name.clone();
    if let Some(name) = l.id_block(Icon::Collection, &name, Some("col_name")) {
        l.commit(d, "Rename Collection");
        let n = sex_3d::scene::unique_name(&name, |n| d.scene.collections.iter().enumerate().any(|(i, c)| i != ci && c.name == n));
        d.scene.collections[ci].name = n;
    }
    l.panel(d, "collection.restrictions", "Restrictions", true, &mut |l, d| {
        inactive(l, |l| {
            let mut t = true;
            l.check("", "Selectable", &mut t);
        });
        let mut v = d.settings.collection_hide_render.contains(&ci);
        if l.check("", "Disable in Renders", &mut v) {
            l.commit(d, "Disable in Renders");
            if v {
                d.settings.collection_hide_render.insert(ci);
            } else {
                d.settings.collection_hide_render.remove(&ci);
            }
        }
        inactive(l, |l| {
            let mut f = false;
            l.check("", "Holdout", &mut f);
            l.check("", "Indirect Only", &mut f);
        });
    });
    l.panel(d, "collection.instancing", "Instancing", false, &mut |l, _| {
        inactive(l, |l| {
            let mut v = 0.0;
            l.number("Offset X", "inst_x", &mut v, NumOpts { unit: "m", ..NumOpts::default() });
            l.number("Y", "inst_y", &mut v, NumOpts { unit: "m", ..NumOpts::default() });
            l.number("Z", "inst_z", &mut v, NumOpts { unit: "m", ..NumOpts::default() });
        });
    });
    l.panel(d, "collection.lineart", "Line Art", false, &mut |l, _| inactive(l, |l| l.enumeration("Usage", "la_usage", "Include", &[])));
    l.panel(d, "collection.exporters", "Exporters", false, &mut |l, _| l.note("Use File > Export"));
}

pub fn particles(l: &mut L, d: &mut Doc) {
    l.space(4);
    let (b, _) = l.list_box(&[], 0, 3);
    if b != 0 {
        d.report = "Particle systems are not available yet".into();
    }
}

pub fn physics(l: &mut L, d: &mut Doc) {
    l.space(4);
    l.panel(d, "physics.enable", "Enable Physics For:", true, &mut |l, d| {
        let names = ["Force Field", "Collision", "Cloth", "Dynamic Paint", "Soft Body", "Fluid", "Rigid Body", "Rigid Body Constraint"];
        for pair in names.chunks(2) {
            if !l.band(ROW) {
                continue;
            }
            let (x, w) = l.inner();
            let half = (w - 4) / 2;
            for (i, n) in pair.iter().enumerate() {
                let r = Rect::new(x + i as i32 * (half + 4), l.y, half, ROW);
                if l.ui.button(r, n) {
                    d.report = format!("{n} physics is not available yet");
                }
            }
            l.y += ROW + super::layout::GAP;
        }
    });
}

pub fn constraints(l: &mut L, _d: &mut Doc) {
    l.space(4);
    if !l.band(ROW) {
        return;
    }
    let (x, w) = l.inner();
    let r = Rect::new(x, l.y, w, ROW);
    if l.ui.dropdown(r, "Add Object Constraint") {
        let it = |n: &str| MenuItem::op(n, "", Op::with("properties.unavailable", &format!("{n} constraint")));
        let col = |names: &[&str]| names.iter().map(|n| it(n)).collect::<Vec<_>>();
        let menu = vec![
            MenuItem::Sub("Motion Tracking".into(), col(&["Camera Solver", "Follow Track", "Object Solver"])),
            MenuItem::Sub("Transform".into(), col(&["Copy Location", "Copy Rotation", "Copy Scale", "Copy Transforms", "Limit Distance", "Limit Location", "Limit Rotation", "Limit Scale", "Maintain Volume", "Transformation", "Transform Cache"])),
            MenuItem::Sub("Tracking".into(), col(&["Clamp To", "Damped Track", "Inverse Kinematics", "Locked Track", "Spline IK", "Stretch To", "Track To"])),
            MenuItem::Sub("Relationship".into(), col(&["Action", "Armature", "Child Of", "Floor", "Follow Path", "Pivot", "Shrinkwrap"])),
        ];
        l.ui.open_menu(r, "", menu);
    }
    l.y += ROW + 4;
}

pub fn texture(l: &mut L, d: &mut Doc) {
    l.space(4);
    if l.button("New") {
        d.report = "Textures are not available yet; image textures come from imported materials".into();
    }
    l.note("Brush and modifier textures are not available yet.");
}
