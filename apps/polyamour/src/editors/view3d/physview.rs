// SPDX-License-Identifier: GPL-3.0-only
//! What the simulations add to the 3D view, drawn over the shaded image:
//! particles (Point, Circle, Cross and Axis display; As Rendered shows
//! points too, or the instances' origins), hair as polylines, and a smoke
//! domain's cached density as soft dots inside its box, a liquid domain's
//! FLIP particles (Show FLIP Particles) as dots; the liquid surface itself
//! is the domain's mesh (its Fluid modifier's output). Cloth and soft
//! bodies need nothing here: their modifiers deform the mesh the viewport
//! already draws, and rigid bodies move their objects.
use super::proj::Proj;
use crate::doc::Doc;
use crate::ui::Ui;
use sex_3d::math::Vec3;
use sex_3d::physics::cache::SimFrame;
use sex_3d::physics::{fluid, particles, DisplayMethod, ParticleType};

const PARTICLE: u32 = 0xe0e0e0;
const HAIR: u32 = 0xc89a5a;
const SMOKE: u32 = 0xdadada;
const FLAME: u32 = 0xff8a2a;
const DOMAIN: u32 = 0x5a7aa0;
const LIQUID: u32 = 0x4aa0ff;

/// Draw particles, hair and smoke.
pub fn draw(d: &Doc, ui: &mut Ui, p: &Proj) {
    let scene = &d.scene;
    for oi in 0..scene.objects.len() {
        if !scene.is_visible(oi) {
            continue;
        }
        for (slot, _, sys) in particles::systems(scene, oi) {
            let s = &sys.settings;
            if s.display_method == DisplayMethod::None {
                continue;
            }
            let v = particles::view(scene, oi, slot, sys, false);
            // Hair shows its display percentage of strands.
            let pct = s.display_percentage.clamp(0, 100) as usize;
            for (k, strand) in v.strands.iter().enumerate() {
                if pct < 100 && (k * 37 % 100) >= pct {
                    continue;
                }
                let pts: Vec<Option<(f32, f32)>> = strand.iter().map(|&q| p.to_screen(q)).collect();
                for w in pts.windows(2) {
                    if let (Some(a), Some(b)) = (w[0], w[1]) {
                        ui.p.line(a.0, a.1, b.0, b.1, HAIR);
                    }
                }
            }
            if s.kind == ParticleType::Hair {
                continue;
            }
            for pt in &v.points {
                let Some((x, y)) = p.to_screen(pt.pos) else { continue };
                let px = (s.display_size.max(pt.size * 0.5) / p.world_per_pixel(pt.pos).max(1e-6)).clamp(2.0, 40.0);
                match s.display_method {
                    DisplayMethod::Circle => ui.p.ring(x, y, px, PARTICLE),
                    DisplayMethod::Cross => {
                        ui.p.line(x - px, y - px, x + px, y + px, PARTICLE);
                        ui.p.line(x - px, y + px, x + px, y - px, PARTICLE);
                    }
                    DisplayMethod::Axis => {
                        let len = s.display_size.max(pt.size * 0.5).max(0.05);
                        for (axis, c) in [(Vec3::X, 0xff3352), (Vec3::Y, 0x8bdc00), (Vec3::Z, 0x2890ff)] {
                            if let Some((ex, ey)) = p.to_screen(pt.pos + pt.rot.rotate(axis) * len) {
                                ui.p.line(x, y, ex, ey, c);
                            }
                        }
                    }
                    _ => {
                        ui.p.line(x - 1.0, y, x + 1.0, y, PARTICLE);
                        ui.p.line(x, y - 1.0, x, y + 1.0, PARTICLE);
                    }
                }
            }
        }
    }
    smoke(d, ui, p);
}

fn smoke(d: &Doc, ui: &mut Ui, p: &Proj) {
    let scene = &d.scene;
    for (oi, slot, s) in fluid::domains(scene) {
        // The domain box (the object itself may be hidden in the viewport).
        let m = scene.world_matrix(oi);
        let corner = |i: usize| m.transform_point(Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 }));
        let bounds = scene.evaluated_mesh(oi).map(|mesh| mesh.bounds());
        let box_pt = |i: usize| match &bounds {
            Some(b) => m.transform_point(Vec3::new(if i & 1 == 0 { b.min.x } else { b.max.x }, if i & 2 == 0 { b.min.y } else { b.max.y }, if i & 4 == 0 { b.min.z } else { b.max.z })),
            None => corner(i),
        };
        for (a, b) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
            if let (Some(pa), Some(pb)) = (p.to_screen(box_pt(a)), p.to_screen(box_pt(b))) {
                ui.p.line_alpha(pa.0, pa.1, pb.0, pb.1, DOMAIN, 160);
            }
        }
        if scene.frame_current < s.cache_frame_start {
            continue;
        }
        let key = fluid::key(&scene.objects[oi].name, slot);
        let Some(f) = scene.physics.caches.frame_at(&key, scene.frame_current.min(s.cache_frame_end)) else { continue };
        if let SimFrame::Liquid(lf) = &*f {
            // Show FLIP Particles: the liquid's particles as dots.
            if let Some(pts) = &lf.particles {
                let stride = (pts.len() / 20000).max(1);
                for q in pts.iter().step_by(stride) {
                    if let Some((x, y)) = p.to_screen(*q) {
                        ui.p.fill_alpha(crate::draw::Rect::new(x as i32 - 1, y as i32 - 1, 2, 2), LIQUID, 200);
                    }
                }
            }
            continue;
        }
        let SimFrame::Fluid(ff) = &*f else { continue };
        let [nx, ny, nz] = ff.res;
        // At most ~24 dots per axis.
        let stride = (nx.max(ny).max(nz) / 24).max(1);
        let cell = ff.dx * stride as f32;
        for k in (0..nz).step_by(stride) {
            for j in (0..ny).step_by(stride) {
                for i in (0..nx).step_by(stride) {
                    let idx = i + nx * (j + ny * k);
                    let den = ff.density.get(idx).copied().unwrap_or(0.0);
                    let flame = ff.flame.as_ref().and_then(|fl| fl.get(idx).copied()).unwrap_or(0.0);
                    if den < 0.02 && flame < 0.02 {
                        continue;
                    }
                    let c = ff.min + Vec3::new(i as f32 + 0.5, j as f32 + 0.5, k as f32 + 0.5) * ff.dx;
                    let Some((x, y)) = p.to_screen(c) else { continue };
                    let r = (cell * 0.5 / p.world_per_pixel(c).max(1e-6)).clamp(1.0, 24.0) as i32;
                    let (color, a) = if flame > den { (FLAME, flame) } else { (SMOKE, den) };
                    let alpha = (a.min(1.0).sqrt() * 150.0) as u32 + 20;
                    ui.p.fill_alpha(crate::draw::Rect::new(x as i32 - r, y as i32 - r, 2 * r, 2 * r), color, alpha);
                }
            }
        }
    }
}
