// SPDX-License-Identifier: GPL-3.0-only
//! Edit Mode operators on a drawing's selected points and strokes: select,
//! delete, dissolve, duplicate, extrude, join, separate, subdivide,
//! simplify, smooth, caps, cyclical, material assignment.
use super::geom;
use super::view::GpView;
use super::{CapType, Drawing, GpPoint, GpStroke};
use crate::math::{Mat4, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectAction {
    Select,
    Deselect,
    Toggle,
    Invert,
}

/// Point or stroke selection mode (1 / 2 / 3 keys select Point, Stroke, Segment).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectMode {
    #[default]
    Point,
    Stroke,
    Segment,
}

pub fn any_selected(d: &Drawing) -> bool {
    d.strokes.iter().any(|s| s.points.iter().any(|p| p.select))
}

/// Keep stroke flags in step with point flags.
pub fn sync_stroke_select(d: &mut Drawing) {
    for s in &mut d.strokes {
        s.select = s.points.iter().any(|p| p.select);
    }
}

pub fn select_all(d: &mut Drawing, action: SelectAction) {
    let action = if action == SelectAction::Toggle {
        if any_selected(d) { SelectAction::Deselect } else { SelectAction::Select }
    } else {
        action
    };
    for s in &mut d.strokes {
        for p in &mut s.points {
            p.select = match action {
                SelectAction::Select => true,
                SelectAction::Deselect => false,
                _ => !p.select,
            };
        }
    }
    sync_stroke_select(d);
}

/// Select every point of strokes with any selected point (Select Linked).
pub fn select_linked(d: &mut Drawing) {
    for s in &mut d.strokes {
        if s.points.iter().any(|p| p.select) {
            s.points.iter_mut().for_each(|p| p.select = true);
        }
    }
    sync_stroke_select(d);
}

/// Grow the selection by one point along strokes (Ctrl+Numpad+).
pub fn select_more(d: &mut Drawing) {
    for s in &mut d.strokes {
        let n = s.points.len();
        let old: Vec<bool> = s.points.iter().map(|p| p.select).collect();
        for i in 0..n {
            let prev = if i > 0 { old[i - 1] } else { s.cyclic && old[n - 1] };
            let next = if i + 1 < n { old[i + 1] } else { s.cyclic && old[0] };
            s.points[i].select |= prev || next;
        }
    }
    sync_stroke_select(d);
}

/// Select points inside a screen rectangle (or toggle/subtract per `action`).
pub fn select_box(d: &mut Drawing, view: &GpView, world: &Mat4, r: (f32, f32, f32, f32), action: SelectAction, mode: SelectMode) {
    let inside = |p: &GpPoint| view.project(world.transform_point(p.position)).is_some_and(|(x, y, _)| x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3);
    for s in &mut d.strokes {
        let hits: Vec<bool> = s.points.iter().map(inside).collect();
        let stroke_hit = hits.iter().any(|&h| h);
        for (p, &h) in s.points.iter_mut().zip(&hits) {
            let h = if mode == SelectMode::Stroke { stroke_hit } else { h };
            if h {
                p.select = action != SelectAction::Deselect;
            }
        }
    }
    sync_stroke_select(d);
}

/// Select (or with `extend`, toggle) the point nearest pixel `at`.
pub fn select_pick(d: &mut Drawing, view: &GpView, world: &Mat4, at: (f32, f32), radius: f32, extend: bool, mode: SelectMode) -> bool {
    let hit = super::tools::pick_point(d, view, world, at, radius);
    if !extend {
        select_all(d, SelectAction::Deselect);
    }
    let Some((si, pi, _)) = hit else { return false };
    let s = &mut d.strokes[si];
    let on = !(extend && s.points[pi].select);
    if mode == SelectMode::Stroke {
        s.points.iter_mut().for_each(|p| p.select = on);
    } else {
        s.points[pi].select = on;
    }
    sync_stroke_select(d);
    true
}

/// Median of the selected points (layer space).
pub fn selection_center(d: &Drawing) -> Option<Vec3> {
    let mut sum = Vec3::ZERO;
    let mut n = 0;
    for s in &d.strokes {
        for p in s.points.iter().filter(|p| p.select) {
            sum += p.position;
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f32)
}

/// Apply `f` to every selected point.
pub fn transform_selected(d: &mut Drawing, f: &dyn Fn(Vec3) -> Vec3) {
    for s in &mut d.strokes {
        for p in s.points.iter_mut().filter(|p| p.select) {
            p.position = f(p.position);
        }
    }
}

/// Delete selected points, splitting strokes.
pub fn delete_points(d: &mut Drawing) -> bool {
    let mut changed = false;
    let mut out = Vec::new();
    for s in d.strokes.drain(..) {
        let cut: Vec<bool> = s.points.iter().map(|p| p.select).collect();
        if cut.iter().any(|&c| c) {
            changed = true;
            out.extend(geom::split_where(&s, &cut));
        } else {
            out.push(s);
        }
    }
    d.strokes = out;
    changed
}

/// Delete strokes with any selected point.
pub fn delete_strokes(d: &mut Drawing) -> bool {
    let n = d.strokes.len();
    d.strokes.retain(|s| !s.points.iter().any(|p| p.select));
    n != d.strokes.len()
}

/// Dissolve selected points (strokes stay connected; strokes left with
/// fewer than two points go away).
pub fn dissolve_points(d: &mut Drawing) -> bool {
    let mut changed = false;
    for s in &mut d.strokes {
        let n = s.points.len();
        s.points.retain(|p| !p.select);
        changed |= n != s.points.len();
    }
    d.strokes.retain(|s| s.points.len() >= 2);
    changed
}

/// Split partially selected strokes into their selected runs; returns the
/// selected parts removed from `d` (used by Separate and Move to Layer).
pub fn extract_selected(d: &mut Drawing) -> Vec<GpStroke> {
    let mut taken = Vec::new();
    let mut keep = Vec::new();
    for s in d.strokes.drain(..) {
        let sel: Vec<bool> = s.points.iter().map(|p| p.select).collect();
        if sel.iter().all(|&x| x) {
            taken.push(s);
        } else if sel.iter().any(|&x| x) {
            let not: Vec<bool> = sel.iter().map(|x| !x).collect();
            taken.extend(geom::split_where(&s, &not));
            keep.extend(geom::split_where(&s, &sel));
        } else {
            keep.push(s);
        }
    }
    d.strokes = keep;
    taken
}

/// Duplicate the selected parts; the copies become the selection.
pub fn duplicate(d: &mut Drawing) -> usize {
    let mut copies = Vec::new();
    for s in &mut d.strokes {
        let sel: Vec<bool> = s.points.iter().map(|p| p.select).collect();
        if !sel.iter().any(|&x| x) {
            continue;
        }
        let not: Vec<bool> = sel.iter().map(|x| !x).collect();
        copies.extend(geom::split_where(s, &not));
        s.points.iter_mut().for_each(|p| p.select = false);
        s.select = false;
    }
    let n = copies.len();
    d.strokes.extend(copies);
    n
}

/// Extrude: selected end points grow a new point (selected, ready to move);
/// selected inner points start a new two-point stroke.
pub fn extrude(d: &mut Drawing) -> usize {
    let mut made = 0;
    let mut new_strokes = Vec::new();
    for s in &mut d.strokes {
        let n = s.points.len();
        if n == 0 || s.cyclic {
            continue;
        }
        let first = s.points[0].select;
        let last = n > 1 && s.points[n - 1].select;
        let inner: Vec<usize> = (1..n.saturating_sub(1)).filter(|&i| s.points[i].select).collect();
        s.points.iter_mut().for_each(|p| p.select = false);
        for i in inner {
            let mut a = s.points[i].clone();
            a.select = false;
            let mut b = a.clone();
            b.select = true;
            new_strokes.push(GpStroke { points: vec![a, b], cyclic: false, ..s.clone() });
            made += 1;
        }
        if last {
            let mut p = s.points[n - 1].clone();
            p.select = true;
            s.points.push(p);
            made += 1;
        }
        if first {
            let mut p = s.points[0].clone();
            p.select = true;
            s.points.insert(0, p);
            made += 1;
        }
    }
    d.strokes.extend(new_strokes);
    sync_stroke_select(d);
    made
}

/// Join selected strokes into one, chaining each to the nearest remaining
/// end. With `copy`, the originals stay.
pub fn join(d: &mut Drawing, copy: bool) -> bool {
    let idx: Vec<usize> = (0..d.strokes.len()).filter(|&i| d.strokes[i].points.iter().any(|p| p.select)).collect();
    if idx.len() < 2 {
        return false;
    }
    let mut parts: Vec<GpStroke> = idx.iter().map(|&i| d.strokes[i].clone()).collect();
    let mut joined = parts.remove(0);
    joined.cyclic = false;
    while !parts.is_empty() {
        let end = joined.points.last().unwrap().position;
        let (mut best, mut bi, mut rev) = (f32::INFINITY, 0, false);
        for (k, p) in parts.iter().enumerate() {
            let (a, b) = (p.points[0].position.distance(end), p.points.last().unwrap().position.distance(end));
            if a < best {
                (best, bi, rev) = (a, k, false);
            }
            if b < best {
                (best, bi, rev) = (b, k, true);
            }
        }
        let mut next = parts.remove(bi);
        if rev {
            next.points.reverse();
        }
        joined.points.extend(next.points);
    }
    if !copy {
        for &i in idx.iter().rev() {
            d.strokes.remove(i);
        }
    }
    d.strokes.push(joined);
    true
}

/// Subdivide segments between selected points.
pub fn subdivide(d: &mut Drawing, cuts: u32) -> bool {
    let mut changed = false;
    for s in &mut d.strokes {
        if !s.points.iter().any(|p| p.select) {
            continue;
        }
        let pts = s.points.clone();
        let sel = |i: usize, j: usize| pts[i].select && pts[j].select;
        let out = geom::subdivide(&pts, s.cyclic, cuts, &sel);
        changed |= out.len() != pts.len();
        s.points = out;
    }
    changed
}

/// Simplify selected strokes (Adaptive, `factor` in layer units).
pub fn simplify(d: &mut Drawing, factor: f32) -> bool {
    let mut changed = false;
    for s in d.strokes.iter_mut().filter(|s| s.points.iter().any(|p| p.select)) {
        let out = geom::simplify(&s.points, factor);
        changed |= out.len() != s.points.len();
        s.points = out;
    }
    changed
}

/// Smooth selected points.
pub fn smooth(d: &mut Drawing, factor: f32, iterations: u32, what: geom::SmoothWhat) {
    for s in &mut d.strokes {
        let sel: Vec<f32> = s.points.iter().map(|p| if p.select { 1.0 } else { 0.0 }).collect();
        if sel.iter().all(|&x| x == 0.0) {
            continue;
        }
        let cyc = s.cyclic;
        geom::smooth(&mut s.points, cyc, factor, iterations, what, &|i| sel[i]);
    }
}

/// Set the caps of selected strokes.
pub fn set_caps(d: &mut Drawing, start: Option<CapType>, end: Option<CapType>) {
    for s in d.strokes.iter_mut().filter(|s| s.points.iter().any(|p| p.select)) {
        if let Some(c) = start {
            s.start_cap = c;
        }
        if let Some(c) = end {
            s.end_cap = c;
        }
    }
}

/// Toggle Cyclic (F).
pub fn toggle_cyclic(d: &mut Drawing) -> bool {
    let mut changed = false;
    for s in d.strokes.iter_mut().filter(|s| s.points.iter().any(|p| p.select)) {
        s.cyclic = !s.cyclic;
        changed = true;
    }
    changed
}

pub fn set_material(d: &mut Drawing, m: usize) -> bool {
    let mut changed = false;
    for s in d.strokes.iter_mut().filter(|s| s.points.iter().any(|p| p.select)) {
        s.material = m;
        changed = true;
    }
    changed
}

/// Reverse the direction of selected strokes.
pub fn reverse(d: &mut Drawing) {
    for s in d.strokes.iter_mut().filter(|s| s.points.iter().any(|p| p.select)) {
        s.points.reverse();
    }
}

/// Scale the radius of selected points.
pub fn scale_radius(d: &mut Drawing, f: f32) {
    for s in &mut d.strokes {
        for p in s.points.iter_mut().filter(|p| p.select) {
            p.radius = (p.radius * f).max(0.0);
        }
    }
}
