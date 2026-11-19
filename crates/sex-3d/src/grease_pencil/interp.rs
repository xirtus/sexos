// SPDX-License-Identifier: GPL-3.0-only
//! Interpolation between keyframes (Ctrl+E and Interpolate Sequence):
//! strokes are paired by index, resampled to a common point count and
//! blended with an easing curve; the results are Breakdown keyframes.
use super::geom;
use super::{Drawing, GpFrame, GpLayer, GpStroke, KeyframeType};

/// Interpolation easing (Blender's Interpolate Sequence "Type").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Easing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => t * t * (3.0 - 2.0 * t),
        }
    }
    pub fn from_id(s: &str) -> Easing {
        match s.to_ascii_uppercase().as_str() {
            "EASE_IN" | "QUAD_IN" => Easing::EaseIn,
            "EASE_OUT" | "QUAD_OUT" => Easing::EaseOut,
            "EASE_IN_OUT" | "QUAD" => Easing::EaseInOut,
            _ => Easing::Linear,
        }
    }
}

/// Blend two strokes at `t` (0 = `a`).
pub fn interpolate_stroke(a: &GpStroke, b: &GpStroke, t: f32) -> GpStroke {
    let n = a.points.len().max(b.points.len()).max(2);
    let pa = geom::resample_count(&a.points, n);
    let pb = geom::resample_count(&b.points, n);
    let points = pa.iter().zip(&pb).map(|(p, q)| p.lerp(q, t)).collect();
    let l4 = |x: [f32; 4], y: [f32; 4]| [0, 1, 2, 3].map(|i| x[i] + (y[i] - x[i]) * t);
    GpStroke {
        points,
        cyclic: if t < 0.5 { a.cyclic } else { b.cyclic },
        material: if t < 0.5 { a.material } else { b.material },
        fill_color: l4(a.fill_color, b.fill_color),
        fill_opacity: a.fill_opacity + (b.fill_opacity - a.fill_opacity) * t,
        softness: a.softness + (b.softness - a.softness) * t,
        ..a.clone()
    }
}

/// The in-between drawing of two drawings at `t`. Unpaired strokes are
/// kept from the nearer side (Blender only interpolates pairs).
pub fn interpolate_drawing(a: &Drawing, b: &Drawing, t: f32) -> Drawing {
    let n = a.strokes.len().min(b.strokes.len());
    let mut strokes: Vec<GpStroke> = (0..n).map(|i| interpolate_stroke(&a.strokes[i], &b.strokes[i], t)).collect();
    let extra = if t < 0.5 { &a.strokes[n..] } else { &b.strokes[n..] };
    strokes.extend(extra.iter().cloned());
    Drawing { strokes }
}

/// The keyframes around `frame` (strictly before and after).
pub fn neighbours(l: &GpLayer, frame: i32) -> Option<(usize, usize)> {
    let prev = l.frames.iter().rposition(|f| f.frame < frame)?;
    let next = l.frames.iter().position(|f| f.frame > frame)?;
    Some((prev, next))
}

/// Ctrl+E: a breakdown at `frame` between the surrounding keys. Returns false
/// when there is no key on both sides or `frame` already has a key.
pub fn interpolate(l: &mut GpLayer, frame: i32, easing: Easing) -> bool {
    if l.key_at(frame).is_some() {
        return false;
    }
    let Some((a, b)) = neighbours(l, frame) else { return false };
    let (fa, fb) = (l.frames[a].frame, l.frames[b].frame);
    let t = easing.apply((frame - fa) as f32 / (fb - fa) as f32);
    let d = interpolate_drawing(&l.frames[a].drawing, &l.frames[b].drawing, t);
    let at = l.frames.partition_point(|f| f.frame < frame);
    l.frames.insert(at, GpFrame { frame, drawing: d, key_type: KeyframeType::Breakdown, select: false });
    true
}

/// Interpolate Sequence: breakdowns on every `step`-th frame between the
/// keys around `frame`. Returns how many were made.
pub fn interpolate_sequence(l: &mut GpLayer, frame: i32, step: i32, easing: Easing) -> usize {
    let Some((a, b)) = neighbours(l, frame).or_else(|| {
        // On a key: interpolate towards the next one.
        let i = l.key_at(frame)?;
        (i + 1 < l.frames.len()).then_some((i, i + 1))
    }) else {
        return 0;
    };
    let (fa, fb) = (l.frames[a].frame, l.frames[b].frame);
    let (da, db) = (l.frames[a].drawing.clone(), l.frames[b].drawing.clone());
    let mut made = 0;
    let mut f = fa + step.max(1);
    while f < fb {
        if l.key_at(f).is_none() {
            let t = easing.apply((f - fa) as f32 / (fb - fa) as f32);
            let at = l.frames.partition_point(|x| x.frame < f);
            l.frames.insert(at, GpFrame { frame: f, drawing: interpolate_drawing(&da, &db, t), key_type: KeyframeType::Breakdown, select: false });
            made += 1;
        }
        f += step.max(1);
    }
    made
}
