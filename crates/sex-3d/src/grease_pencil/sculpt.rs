// SPDX-License-Identifier: GPL-3.0-only
//! Grease Pencil Sculpt Mode, Vertex Paint and Weight Paint brushes. Each
//! dab works on the points whose projection falls under the brush circle,
//! weighted by a smooth falloff; movements are in the view plane.
use super::tools::falloff;
use super::view::GpView;
use super::{Drawing, GpPoint, GpStroke};
use crate::math::{Mat4, Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SculptTool {
    Smooth,
    Thickness,
    Strength,
    Randomize,
    Grab,
    Push,
    Twist,
    Pinch,
    Clone,
}

impl SculptTool {
    pub const ALL: [SculptTool; 9] = [SculptTool::Smooth, SculptTool::Thickness, SculptTool::Strength, SculptTool::Randomize, SculptTool::Grab, SculptTool::Push, SculptTool::Twist, SculptTool::Pinch, SculptTool::Clone];
    pub fn label(self) -> &'static str {
        match self {
            SculptTool::Smooth => "Smooth",
            SculptTool::Thickness => "Thickness",
            SculptTool::Strength => "Strength",
            SculptTool::Randomize => "Randomize",
            SculptTool::Grab => "Grab",
            SculptTool::Push => "Push",
            SculptTool::Twist => "Twist",
            SculptTool::Pinch => "Pinch",
            SculptTool::Clone => "Clone",
        }
    }
    pub fn from_label(s: &str) -> Option<SculptTool> {
        SculptTool::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(s))
    }
}

/// One brush dab: where the brush is now, where it was, its size and
/// strength, and whether Ctrl inverts it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    pub at: (f32, f32),
    pub prev: (f32, f32),
    pub radius: f32,
    pub strength: f32,
    pub invert: bool,
}

/// Points under the brush with their falloff weights: (stroke, point, weight).
fn under(d: &Drawing, view: &GpView, world: &Mat4, at: (f32, f32), radius: f32) -> Vec<(usize, usize, f32)> {
    let mut out = Vec::new();
    for (si, s) in d.strokes.iter().enumerate() {
        for (pi, p) in s.points.iter().enumerate() {
            let Some((x, y, _)) = view.project(world.transform_point(p.position)) else { continue };
            let w = falloff(((x - at.0).powi(2) + (y - at.1).powi(2)).sqrt(), radius);
            if w > 0.0 {
                out.push((si, pi, w));
            }
        }
    }
    out
}

/// Grab keeps the points it picked up at the start of the stroke.
#[derive(Clone, Debug, Default)]
pub struct SculptStroke {
    pub grabbed: Vec<(usize, usize, f32)>,
    pub cloned: bool,
    rng: u32,
}

impl SculptStroke {
    fn rand(&mut self) -> f32 {
        // xorshift32
        let mut x = if self.rng == 0 { 0x9e37_79b9 } else { self.rng };
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Apply one sculpt dab; `clipboard` feeds the Clone brush. Returns whether
/// anything changed.
pub fn sculpt_dab(d: &mut Drawing, view: &GpView, world: &Mat4, tool: SculptTool, dab: &Dab, st: &mut SculptStroke, clipboard: &[GpStroke]) -> bool {
    let inv = world.inverse();
    let sign = if dab.invert { -1.0 } else { 1.0 };
    let local_delta = |p: Vec3, dx: f32, dy: f32| -> Vec3 {
        let w = world.transform_point(p);
        inv.transform_vector(view.pixel_delta(dx, dy, w))
    };
    match tool {
        SculptTool::Grab => {
            if st.grabbed.is_empty() {
                st.grabbed = under(d, view, world, dab.prev, dab.radius);
            }
            let (dx, dy) = (dab.at.0 - dab.prev.0, dab.at.1 - dab.prev.1);
            for &(si, pi, w) in &st.grabbed {
                let Some(p) = d.strokes.get_mut(si).and_then(|s| s.points.get_mut(pi)) else { continue };
                p.position += local_delta(p.position, dx * w, dy * w);
            }
            !st.grabbed.is_empty()
        }
        SculptTool::Clone => {
            if st.cloned || clipboard.is_empty() {
                return false;
            }
            // Paste the clipboard centred under the brush (in the view plane).
            let mut c = Vec3::ZERO;
            let mut n = 0;
            for s in clipboard {
                for p in &s.points {
                    c += p.position;
                    n += 1;
                }
            }
            let c = c / n.max(1) as f32;
            let cw = world.transform_point(c);
            let target = view.at_depth(dab.at.0, dab.at.1, cw);
            let off = inv.transform_vector(target - cw);
            for s in clipboard {
                let mut s = s.clone();
                s.points.iter_mut().for_each(|p| p.position += off);
                d.strokes.push(s);
            }
            st.cloned = true;
            true
        }
        _ => {
            let hits = under(d, view, world, dab.at, dab.radius);
            if hits.is_empty() {
                return false;
            }
            let (dx, dy) = (dab.at.0 - dab.prev.0, dab.at.1 - dab.prev.1);
            let centre_world = {
                let mut c = Vec3::ZERO;
                for &(si, pi, _) in &hits {
                    c += world.transform_point(d.strokes[si].points[pi].position);
                }
                c / hits.len() as f32
            };
            let centre = view.at_depth(dab.at.0, dab.at.1, centre_world);
            for &(si, pi, w) in &hits {
                let f = w * dab.strength;
                let s = &mut d.strokes[si];
                let n = s.points.len();
                match tool {
                    SculptTool::Smooth => {
                        if n < 3 || (!s.cyclic && (pi == 0 || pi == n - 1)) {
                            continue;
                        }
                        let (a, b) = (s.points[(pi + n - 1) % n].position, s.points[(pi + 1) % n].position);
                        let p = &mut s.points[pi];
                        p.position = p.position.lerp((a + b) * 0.5, f.min(1.0));
                    }
                    SculptTool::Thickness => {
                        let p = &mut s.points[pi];
                        p.radius = (p.radius * (1.0 + sign * f * 0.1)).max(0.0);
                    }
                    SculptTool::Strength => {
                        let p = &mut s.points[pi];
                        p.opacity = (p.opacity + sign * f * 0.1).clamp(0.0, 1.0);
                    }
                    SculptTool::Randomize => {
                        let (rx, ry) = (st.rand(), st.rand());
                        let p = &mut s.points[pi].position;
                        *p += local_delta(*p, rx * f * dab.radius * 0.1, ry * f * dab.radius * 0.1);
                    }
                    SculptTool::Push => {
                        let p = &mut s.points[pi].position;
                        *p += local_delta(*p, dx * f, dy * f);
                    }
                    SculptTool::Twist => {
                        let pw = world.transform_point(s.points[pi].position);
                        let q = Quat::from_axis_angle(view.back, sign * f * 0.1);
                        let nw = centre + q.rotate(pw - centre);
                        s.points[pi].position = inv.transform_point(nw);
                    }
                    SculptTool::Pinch => {
                        let pw = world.transform_point(s.points[pi].position);
                        let nw = pw.lerp(centre, (sign * f * 0.1).clamp(-1.0, 1.0));
                        s.points[pi].position = inv.transform_point(nw);
                    }
                    SculptTool::Grab | SculptTool::Clone => {}
                }
            }
            true
        }
    }
}

// ---------------------------------------------------------------- vertex paint

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VertexTool {
    Draw,
    Blur,
    Average,
    Smear,
    Replace,
}

impl VertexTool {
    pub const ALL: [VertexTool; 5] = [VertexTool::Draw, VertexTool::Blur, VertexTool::Average, VertexTool::Smear, VertexTool::Replace];
    pub fn label(self) -> &'static str {
        match self {
            VertexTool::Draw => "Draw",
            VertexTool::Blur => "Blur",
            VertexTool::Average => "Average",
            VertexTool::Smear => "Smear",
            VertexTool::Replace => "Replace",
        }
    }
    pub fn from_label(s: &str) -> Option<VertexTool> {
        VertexTool::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(s))
    }
}

fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// One Vertex Paint dab on point colours (and fills with `fill`).
pub fn vertex_dab(d: &mut Drawing, view: &GpView, world: &Mat4, tool: VertexTool, dab: &Dab, color: [f32; 3], fill: bool) -> bool {
    let hits = under(d, view, world, dab.at, dab.radius);
    if hits.is_empty() {
        return false;
    }
    let target = [color[0], color[1], color[2], 1.0];
    let avg = {
        let mut a = [0.0; 4];
        for &(si, pi, _) in &hits {
            let c = d.strokes[si].points[pi].vertex_color;
            for k in 0..4 {
                a[k] += c[k];
            }
        }
        a.map(|v| v / hits.len() as f32)
    };
    // Smear samples the colour behind the brush.
    let behind = (dab.at.0 - (dab.at.0 - dab.prev.0) * 2.0, dab.at.1 - (dab.at.1 - dab.prev.1) * 2.0);
    let smear_src = super::tools::pick_point(d, view, world, behind, dab.radius).map(|(s, p, _)| d.strokes[s].points[p].vertex_color);
    for &(si, pi, w) in &hits {
        let f = w * dab.strength;
        let s = &mut d.strokes[si];
        let n = s.points.len();
        let neighbours = {
            let a = s.points[if pi > 0 { pi - 1 } else { pi }].vertex_color;
            let b = s.points[if pi + 1 < n { pi + 1 } else { pi }].vertex_color;
            mix4(a, b, 0.5)
        };
        let p: &mut GpPoint = &mut s.points[pi];
        p.vertex_color = match tool {
            VertexTool::Draw => mix4(p.vertex_color, target, f),
            VertexTool::Blur => mix4(p.vertex_color, neighbours, f),
            VertexTool::Average => mix4(p.vertex_color, avg, f),
            VertexTool::Smear => mix4(p.vertex_color, smear_src.unwrap_or(p.vertex_color), f),
            VertexTool::Replace => {
                if p.vertex_color[3] > 0.0 {
                    [color[0], color[1], color[2], p.vertex_color[3]]
                } else {
                    p.vertex_color
                }
            }
        };
        if fill && s.cyclic && tool == VertexTool::Draw {
            s.fill_color = mix4(s.fill_color, target, f);
        }
    }
    true
}

// ---------------------------------------------------------------- weight paint

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightTool {
    Draw,
    Blur,
    Average,
    Smear,
}

impl WeightTool {
    pub const ALL: [WeightTool; 4] = [WeightTool::Draw, WeightTool::Blur, WeightTool::Average, WeightTool::Smear];
    pub fn label(self) -> &'static str {
        match self {
            WeightTool::Draw => "Draw",
            WeightTool::Blur => "Blur",
            WeightTool::Average => "Average",
            WeightTool::Smear => "Smear",
        }
    }
}

/// One Weight Paint dab on vertex group `group` towards `weight` (Draw;
/// Ctrl inverts to subtract).
pub fn weight_dab(d: &mut Drawing, view: &GpView, world: &Mat4, tool: WeightTool, dab: &Dab, group: u32, weight: f32) -> bool {
    let hits = under(d, view, world, dab.at, dab.radius);
    if hits.is_empty() {
        return false;
    }
    let avg = hits.iter().map(|&(s, p, _)| d.strokes[s].points[p].weight(group)).sum::<f32>() / hits.len() as f32;
    let behind = (dab.at.0 - (dab.at.0 - dab.prev.0) * 2.0, dab.at.1 - (dab.at.1 - dab.prev.1) * 2.0);
    let smear = super::tools::pick_point(d, view, world, behind, dab.radius).map(|(s, p, _)| d.strokes[s].points[p].weight(group));
    let target = if dab.invert { 0.0 } else { weight };
    for &(si, pi, w) in &hits {
        let f = (w * dab.strength).clamp(0.0, 1.0);
        let s = &mut d.strokes[si];
        let n = s.points.len();
        let nb = (s.points[pi.saturating_sub(1)].weight(group) + s.points[(pi + 1).min(n - 1)].weight(group)) * 0.5;
        let cur = s.points[pi].weight(group);
        let to = match tool {
            WeightTool::Draw => target,
            WeightTool::Blur => nb,
            WeightTool::Average => avg,
            WeightTool::Smear => smear.unwrap_or(cur),
        };
        s.points[pi].set_weight(group, cur + (to - cur) * f);
    }
    true
}
