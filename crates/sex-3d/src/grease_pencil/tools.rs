// SPDX-License-Identifier: GPL-3.0-only
//! Draw Mode: the Draw brush (stabilizer, simplify on the fly), the Fill
//! tool (view-space flood fill turned into a stroke), Erase (point, stroke,
//! dissolve, soft), Tint and the primitives (Line, Polyline, Arc, Curve,
//! Box, Circle).
use super::geom::{self, seg_dist2};
use super::view::{GpView, Projector};
use super::{CapType, Drawing, GpPoint, GpStroke};
use crate::math::{Mat4, Vec2, Vec3};

/// The Draw brush's settings (pressure-less: the mouse has none).
#[derive(Clone, Debug, PartialEq)]
pub struct DrawBrush {
    /// Radius in pixels.
    pub radius: f32,
    /// Opacity of the points.
    pub strength: f32,
    /// 1 hard .. 0 soft.
    pub hardness: f32,
    /// Lazy mouse: the stroke follows the pointer at `stabilizer_radius` px.
    pub use_stabilizer: bool,
    pub stabilizer_radius: f32,
    pub stabilizer_factor: f32,
    /// Simplify on the fly: drop points closer than this (px).
    pub spacing: f32,
    /// Post-processing: smoothing factor and iterations, simplify (px), subdivisions.
    pub smooth_factor: f32,
    pub smooth_steps: u32,
    pub simplify: f32,
    pub subdivide: u32,
    /// Vertex colour painted with the stroke (alpha 0: material colour only).
    pub vertex_color: [f32; 4],
    pub caps: CapType,
    /// Thinner ends (Blender's "Thickness" curve falloff at the tips), 0..1.
    pub taper: f32,
}

impl Default for DrawBrush {
    fn default() -> Self {
        DrawBrush {
            radius: 10.0,
            strength: 1.0,
            hardness: 1.0,
            use_stabilizer: false,
            stabilizer_radius: 35.0,
            stabilizer_factor: 0.75,
            spacing: 2.0,
            smooth_factor: 0.35,
            smooth_steps: 1,
            simplify: 0.0,
            subdivide: 0,
            vertex_color: [0.0; 4],
            caps: CapType::Round,
            taper: 0.0,
        }
    }
}

/// A stroke being drawn: screen samples collected as the pointer moves.
#[derive(Clone, Debug)]
pub struct StrokeBuilder {
    pub brush: DrawBrush,
    pub screen: Vec<(f32, f32)>,
    /// The stabilized pen position.
    pen: Option<(f32, f32)>,
}

impl StrokeBuilder {
    pub fn new(brush: DrawBrush, at: (f32, f32)) -> StrokeBuilder {
        StrokeBuilder { brush, screen: vec![at], pen: Some(at) }
    }
    /// Feed a pointer position; returns true when a point was added.
    pub fn add(&mut self, at: (f32, f32)) -> bool {
        let b = &self.brush;
        let mut p = at;
        if b.use_stabilizer {
            let pen = self.pen.unwrap_or(at);
            let (dx, dy) = (at.0 - pen.0, at.1 - pen.1);
            let d = (dx * dx + dy * dy).sqrt();
            if d <= b.stabilizer_radius {
                return false;
            }
            // The string pulls the pen by what exceeds its length, eased by the factor.
            let pull = (d - b.stabilizer_radius) / d * (1.0 - b.stabilizer_factor * 0.5);
            p = (pen.0 + dx * pull, pen.1 + dy * pull);
            self.pen = Some(p);
        }
        let last = *self.screen.last().unwrap();
        let (dx, dy) = (p.0 - last.0, p.1 - last.1);
        if (dx * dx + dy * dy).sqrt() < b.spacing.max(0.5) {
            return false;
        }
        self.screen.push(p);
        true
    }
    /// Turn the samples into a stroke in layer space.
    pub fn finish(&self, proj: &Projector<'_>, material: usize) -> Option<GpStroke> {
        screen_to_stroke(&self.screen, &self.brush, proj, material, false)
    }
}

/// Screen points to a stroke: placement, radius from pixels, post-processing.
pub fn screen_to_stroke(screen: &[(f32, f32)], b: &DrawBrush, proj: &Projector<'_>, material: usize, cyclic: bool) -> Option<GpStroke> {
    if screen.is_empty() {
        return None;
    }
    let mut pts: Vec<GpPoint> = screen
        .iter()
        .map(|&(x, y)| {
            let w = proj.world(x, y);
            let local = proj.object.inverse().transform_point(w);
            let mut p = GpPoint::new(local, proj.radius(b.radius, w));
            p.opacity = b.strength;
            p.vertex_color = b.vertex_color;
            p
        })
        .collect();
    if pts.len() == 1 {
        // A dot: two coincident points draw a round cap.
        pts.push(pts[0].clone());
    }
    if b.smooth_factor > 0.0 {
        geom::smooth(&mut pts, cyclic, b.smooth_factor, b.smooth_steps, geom::SmoothWhat::POSITION, &|_| 1.0);
    }
    if b.simplify > 0.0 && pts.len() > 2 {
        let r = proj.radius(b.simplify, proj.object.transform_point(pts[0].position));
        pts = geom::simplify(&pts, r);
    }
    if b.subdivide > 0 {
        pts = geom::subdivide(&pts, cyclic, b.subdivide, &|_, _| true);
    }
    if b.taper > 0.0 && pts.len() > 2 {
        let n = pts.len() as f32 - 1.0;
        for (i, p) in pts.iter_mut().enumerate() {
            let t = i as f32 / n;
            let tip = (t.min(1.0 - t) * 2.0).min(1.0);
            p.radius *= 1.0 - b.taper * (1.0 - tip.sqrt());
        }
    }
    let mut s = GpStroke::new(pts, material);
    s.softness = 1.0 - b.hardness.clamp(0.0, 1.0);
    s.start_cap = b.caps;
    s.end_cap = b.caps;
    s.cyclic = cyclic;
    Some(s)
}

// ---------------------------------------------------------------- primitives

/// The primitive tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Line,
    Polyline,
    Arc,
    Curve,
    Box,
    Circle,
}

impl Primitive {
    pub const ALL: [Primitive; 6] = [Primitive::Line, Primitive::Polyline, Primitive::Arc, Primitive::Curve, Primitive::Box, Primitive::Circle];
    pub fn label(self) -> &'static str {
        match self {
            Primitive::Line => "Line",
            Primitive::Polyline => "Polyline",
            Primitive::Arc => "Arc",
            Primitive::Curve => "Curve",
            Primitive::Box => "Box",
            Primitive::Circle => "Circle",
        }
    }
    pub fn cyclic(self) -> bool {
        matches!(self, Primitive::Box | Primitive::Circle)
    }
}

/// Snap `b` relative to `a` to multiples of 45 degrees (Shift on Line/Polyline).
pub fn snap_angle(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    let ang = dy.atan2(dx);
    let step = std::f32::consts::FRAC_PI_4;
    let s = (ang / step).round() * step;
    (a.0 + s.cos() * len, a.1 + s.sin() * len)
}

/// Make `b` the corner of a square around `a` (Shift on Box/Circle).
pub fn snap_square(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let m = dx.abs().max(dy.abs());
    (a.0 + m * dx.signum(), a.1 + m * dy.signum())
}

/// Screen points of a primitive from its control points. `ctrl`: Line [a, b];
/// Polyline: every click; Arc [a, b] (bulging to the left of a->b, or [a, b, c]
/// through c); Curve [a, b, h1, h2] (cubic Bezier; handles default to the
/// thirds); Box and Circle [corner, corner]. `subdiv`: points per edge.
pub fn primitive_points(kind: Primitive, ctrl: &[(f32, f32)], subdiv: u32) -> Vec<(f32, f32)> {
    let seg = subdiv.max(1) as usize + 1;
    let lerp = |a: (f32, f32), b: (f32, f32), t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let line = |a, b| (0..=seg).map(|i| lerp(a, b, i as f32 / seg as f32)).collect::<Vec<_>>();
    match (kind, ctrl) {
        (_, []) => Vec::new(),
        (Primitive::Line, [a, b, ..]) => line(*a, *b),
        (Primitive::Polyline, pts) => {
            let mut out = vec![pts[0]];
            for w in pts.windows(2) {
                out.extend(line(w[0], w[1]).into_iter().skip(1));
            }
            out
        }
        (Primitive::Arc, [a, b, rest @ ..]) => {
            // Through the control point c, or a half circle-ish bulge.
            let c = rest.first().copied().unwrap_or_else(|| {
                let m = lerp(*a, *b, 0.5);
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                (m.0 + dy * 0.5, m.1 - dx * 0.5)
            });
            let n = seg * 4;
            // Quadratic Bezier whose midpoint passes through c.
            let h = (2.0 * c.0 - 0.5 * (a.0 + b.0), 2.0 * c.1 - 0.5 * (a.1 + b.1));
            (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    (u * u * a.0 + 2.0 * u * t * h.0 + t * t * b.0, u * u * a.1 + 2.0 * u * t * h.1 + t * t * b.1)
                })
                .collect()
        }
        (Primitive::Curve, [a, b, rest @ ..]) => {
            let h1 = rest.first().copied().unwrap_or_else(|| lerp(*a, *b, 1.0 / 3.0));
            let h2 = rest.get(1).copied().unwrap_or_else(|| lerp(*a, *b, 2.0 / 3.0));
            let n = seg * 4;
            (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    (w0 * a.0 + w1 * h1.0 + w2 * h2.0 + w3 * b.0, w0 * a.1 + w1 * h1.1 + w2 * h2.1 + w3 * b.1)
                })
                .collect()
        }
        (Primitive::Box, [a, b, ..]) => {
            let corners = [*a, (b.0, a.1), *b, (a.0, b.1)];
            let mut out = Vec::new();
            for i in 0..4 {
                out.extend(line(corners[i], corners[(i + 1) % 4]).into_iter().take(seg));
            }
            out
        }
        (Primitive::Circle, [a, b, ..]) => {
            let c = lerp(*a, *b, 0.5);
            let (rx, ry) = ((b.0 - a.0).abs() * 0.5, (b.1 - a.1).abs() * 0.5);
            let n = (seg * 8).max(16);
            (0..n)
                .map(|i| {
                    let t = std::f32::consts::TAU * i as f32 / n as f32;
                    (c.0 + t.cos() * rx, c.1 + t.sin() * ry)
                })
                .collect()
        }
        (_, [a]) => vec![*a],
    }
}

/// A primitive as a stroke.
pub fn primitive_stroke(kind: Primitive, ctrl: &[(f32, f32)], subdiv: u32, b: &DrawBrush, proj: &Projector<'_>, material: usize) -> Option<GpStroke> {
    let pts = primitive_points(kind, ctrl, subdiv);
    let b = DrawBrush { smooth_factor: 0.0, simplify: 0.0, subdivide: 0, ..b.clone() };
    screen_to_stroke(&pts, &b, proj, material, kind.cyclic())
}

// ---------------------------------------------------------------- fill

/// Why a fill failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FillError {
    /// The click was on a boundary.
    OnBoundary,
    /// The region is not closed (the flood reached the view's border).
    Leak,
}

/// The Fill tool's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct FillSettings {
    /// Boundary lines are drawn at least this thick (px) to close small gaps.
    pub leak_size: f32,
    /// Grow the filled region by this many pixels (under the boundary).
    pub dilate: i32,
    /// Simplify the outline (px).
    pub simplify: f32,
    /// Put the fill below the other strokes.
    pub on_back: bool,
}

impl Default for FillSettings {
    fn default() -> Self {
        FillSettings { leak_size: 3.0, dilate: 1, simplify: 0.5, on_back: true }
    }
}

/// Boundary strokes of the drawing(s) in screen space: (points, radius px).
pub fn screen_strokes(view: &GpView, world: &Mat4, d: &Drawing) -> Vec<(Vec<Vec2>, f32, bool)> {
    let s = super::scale_of(world);
    d.strokes
        .iter()
        .map(|st| {
            let mut pts = Vec::new();
            let mut rad = 0.0f32;
            for p in &st.points {
                let w = world.transform_point(p.position);
                if let Some((x, y, dist)) = view.project(w) {
                    pts.push(Vec2::new(x, y));
                    rad = rad.max(p.radius * s * view.pixels_per_unit(dist));
                }
            }
            (pts, rad, st.cyclic)
        })
        .collect()
}

/// Flood fill from pixel `at` inside the boundaries (screen-space strokes),
/// returning the outline of the region in screen space.
pub fn fill_region(width: u32, height: u32, boundaries: &[(Vec<Vec2>, f32, bool)], at: (f32, f32), s: &FillSettings) -> Result<Vec<(f32, f32)>, FillError> {
    let (w, h) = (width as i32, height as i32);
    let mut wall = vec![false; (w * h) as usize];
    let mut mark = |x: i32, y: i32| {
        if x >= 0 && y >= 0 && x < w && y < h {
            wall[(y * w + x) as usize] = true;
        }
    };
    for (pts, rad, cyclic) in boundaries {
        let r = rad.max(s.leak_size * 0.5);
        let mut segs: Vec<(Vec2, Vec2)> = pts.windows(2).map(|p| (p[0], p[1])).collect();
        if *cyclic && pts.len() > 2 {
            segs.push((pts[pts.len() - 1], pts[0]));
        }
        if pts.len() == 1 {
            segs.push((pts[0], pts[0]));
        }
        for (a, b) in segs {
            let (x0, x1) = ((a.x.min(b.x) - r).floor() as i32, (a.x.max(b.x) + r).ceil() as i32);
            let (y0, y1) = ((a.y.min(b.y) - r).floor() as i32, (a.y.max(b.y) + r).ceil() as i32);
            for y in y0.max(0)..=y1.min(h - 1) {
                for x in x0.max(0)..=x1.min(w - 1) {
                    if seg_dist2(Vec2::new(x as f32 + 0.5, y as f32 + 0.5), a, b).0 <= r {
                        mark(x, y);
                    }
                }
            }
        }
    }
    let (sx, sy) = (at.0 as i32, at.1 as i32);
    if sx < 0 || sy < 0 || sx >= w || sy >= h {
        return Err(FillError::Leak);
    }
    if wall[(sy * w + sx) as usize] {
        return Err(FillError::OnBoundary);
    }
    let mut inside = vec![false; (w * h) as usize];
    let mut stack = vec![(sx, sy)];
    inside[(sy * w + sx) as usize] = true;
    while let Some((x, y)) = stack.pop() {
        if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
            return Err(FillError::Leak);
        }
        for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            let i = (ny * w + nx) as usize;
            if !inside[i] && !wall[i] {
                inside[i] = true;
                stack.push((nx, ny));
            }
        }
    }
    // Grow under the boundary so the fill meets the line's middle.
    for _ in 0..s.dilate.max(0) {
        let prev = inside.clone();
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = (y * w + x) as usize;
                if !prev[i] && (prev[i - 1] || prev[i + 1] || prev[i - w as usize] || prev[i + w as usize]) {
                    inside[i] = true;
                }
            }
        }
    }
    let outline = trace_outline(&inside, w, h, (sx, sy));
    let pts: Vec<Vec3> = outline.iter().map(|&(x, y)| Vec3::new(x, y, 0.0)).collect();
    let keep = geom::rdp_keep(&pts, s.simplify.max(0.0));
    Ok(outline.into_iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| p).collect())
}

/// Moore-neighbour tracing of the outer boundary of the marked region.
/// Points are pixel centres.
fn trace_outline(inside: &[bool], w: i32, h: i32, seed: (i32, i32)) -> Vec<(f32, f32)> {
    let get = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && inside[(y * w + x) as usize];
    // The first region pixel in raster order: west and north of it are outside,
    // so the trace follows the outer boundary (never a hole's).
    let first = (0..w * h).find(|&i| inside[i as usize]).map(|i| (i % w, i / w));
    let (sx, sy) = first.unwrap_or(seed);
    // Clockwise on screen (y down), starting west.
    const DIRS: [(i32, i32); 8] = [(-1, 0), (-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1)];
    let start = (sx, sy);
    let mut cur = start;
    // The cell we came from: west of the start is outside.
    let mut from = (sx - 1, sy);
    let mut out = vec![(cur.0 as f32 + 0.5, cur.1 as f32 + 0.5)];
    let limit = (w as usize * h as usize) * 4;
    for _ in 0..limit {
        let k0 = DIRS.iter().position(|d| (cur.0 + d.0, cur.1 + d.1) == from).unwrap_or(0);
        let mut next = None;
        let mut prev = from;
        for k in 1..=8 {
            let d = DIRS[(k0 + k) % 8];
            let c = (cur.0 + d.0, cur.1 + d.1);
            if get(c.0, c.1) {
                next = Some(c);
                break;
            }
            prev = c;
        }
        let Some(n) = next else { break };
        from = prev;
        cur = n;
        if cur == start {
            break;
        }
        out.push((cur.0 as f32 + 0.5, cur.1 as f32 + 0.5));
    }
    out
}

/// Fill at pixel `at`: a new cyclic stroke on the plane, or why not.
pub fn fill(view: &GpView, boundary: &[(Vec<Vec2>, f32, bool)], at: (f32, f32), s: &FillSettings, b: &DrawBrush, proj: &Projector<'_>, material: usize) -> Result<GpStroke, FillError> {
    let outline = fill_region(view.width, view.height, boundary, at, s)?;
    let brush = DrawBrush { smooth_factor: 0.0, simplify: 0.0, subdivide: 0, radius: b.radius.min(2.0), ..b.clone() };
    let mut st = screen_to_stroke(&outline, &brush, proj, material, true).ok_or(FillError::Leak)?;
    st.fill_color = b.vertex_color;
    Ok(st)
}

// ---------------------------------------------------------------- erase

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EraseMode {
    /// Remove points, splitting strokes.
    Point,
    /// Remove whole strokes touched.
    Stroke,
    /// Remove points but keep strokes connected.
    Dissolve,
    /// Fade opacity; points that fade out are removed.
    Soft,
}

impl EraseMode {
    pub const ALL: [EraseMode; 4] = [EraseMode::Point, EraseMode::Stroke, EraseMode::Dissolve, EraseMode::Soft];
    pub fn label(self) -> &'static str {
        match self {
            EraseMode::Point => "Point",
            EraseMode::Stroke => "Stroke",
            EraseMode::Dissolve => "Dissolve",
            EraseMode::Soft => "Soft",
        }
    }
}

/// Brush falloff (smooth): 1 at the centre, 0 at the radius.
pub fn falloff(d: f32, r: f32) -> f32 {
    if d >= r || r <= 0.0 {
        return 0.0;
    }
    let t = 1.0 - d / r;
    t * t * (3.0 - 2.0 * t)
}

/// Erase with a circle at `at` of `radius` px; `strength` for Soft. Points
/// are projected with `world`. Returns whether anything changed.
pub fn erase(d: &mut Drawing, view: &GpView, world: &Mat4, at: (f32, f32), radius: f32, mode: EraseMode, strength: f32) -> bool {
    let c = Vec2::new(at.0, at.1);
    let mut changed = false;
    let mut out = Vec::with_capacity(d.strokes.len());
    for s in d.strokes.drain(..) {
        let scr: Vec<Option<Vec2>> = s.points.iter().map(|p| view.project(world.transform_point(p.position)).map(|(x, y, _)| Vec2::new(x, y))).collect();
        let dist = |i: usize| scr[i].map_or(f32::INFINITY, |q| ((q.x - c.x).powi(2) + (q.y - c.y).powi(2)).sqrt());
        // Strokes are also hit along their segments (a fast drag over a long segment).
        let seg_hit = || {
            (0..s.points.len().saturating_sub(1)).any(|i| match (scr[i], scr[i + 1]) {
                (Some(a), Some(b)) => seg_dist2(c, a, b).0 <= radius,
                _ => false,
            })
        };
        match mode {
            EraseMode::Stroke => {
                if (0..s.points.len()).any(|i| dist(i) <= radius) || seg_hit() {
                    changed = true;
                    continue;
                }
                out.push(s);
            }
            EraseMode::Point | EraseMode::Dissolve => {
                let mut cut: Vec<bool> = (0..s.points.len()).map(|i| dist(i) <= radius).collect();
                // A segment crossing the brush without points inside: cut it in the middle.
                if mode == EraseMode::Point && !cut.iter().any(|&c| c) && seg_hit() {
                    let mut st = s.clone();
                    let i = (0..s.points.len() - 1).find(|&i| matches!((scr[i], scr[i + 1]), (Some(a), Some(b)) if seg_dist2(c, a, b).0 <= radius)).unwrap();
                    let mid = st.points[i].lerp(&st.points[i + 1], 0.5);
                    st.points.insert(i + 1, mid);
                    cut = vec![false; st.points.len()];
                    cut[i + 1] = true;
                    changed = true;
                    out.extend(geom::split_where(&st, &cut).into_iter().filter(|x| !x.points.is_empty()));
                    continue;
                }
                if !cut.iter().any(|&c| c) {
                    out.push(s);
                    continue;
                }
                changed = true;
                if mode == EraseMode::Dissolve {
                    let pts: Vec<GpPoint> = s.points.iter().zip(&cut).filter(|(_, c)| !**c).map(|(p, _)| p.clone()).collect();
                    if pts.len() >= 1 {
                        out.push(GpStroke { points: pts, ..s });
                    }
                } else {
                    out.extend(geom::split_where(&s, &cut));
                }
            }
            EraseMode::Soft => {
                let mut st = s;
                let mut cut = vec![false; st.points.len()];
                let mut any = false;
                for (i, p) in st.points.iter_mut().enumerate() {
                    let f = falloff(dist(i), radius) * strength;
                    if f > 0.0 {
                        any = true;
                        p.opacity = (p.opacity - f).max(0.0);
                        cut[i] = p.opacity < 0.05;
                    }
                }
                changed |= any;
                if cut.iter().any(|&c| c) {
                    out.extend(geom::split_where(&st, &cut));
                } else {
                    out.push(st);
                }
            }
        }
    }
    d.strokes = out;
    changed
}

/// Tint: blend vertex colours towards `color` under the brush.
pub fn tint(d: &mut Drawing, view: &GpView, world: &Mat4, at: (f32, f32), radius: f32, strength: f32, color: [f32; 3], stroke: bool, fill: bool) -> bool {
    let mut changed = false;
    for s in &mut d.strokes {
        let mut hit = 0.0f32;
        for p in &mut s.points {
            let Some((x, y, _)) = view.project(world.transform_point(p.position)) else { continue };
            let f = falloff(((x - at.0).powi(2) + (y - at.1).powi(2)).sqrt(), radius) * strength;
            if f <= 0.0 {
                continue;
            }
            hit = hit.max(f);
            if stroke {
                for i in 0..3 {
                    p.vertex_color[i] += (color[i] - p.vertex_color[i]) * f;
                }
                p.vertex_color[3] += (1.0 - p.vertex_color[3]) * f;
                changed = true;
            }
        }
        if fill && hit > 0.0 && s.cyclic {
            for i in 0..3 {
                s.fill_color[i] += (color[i] - s.fill_color[i]) * hit;
            }
            s.fill_color[3] += (1.0 - s.fill_color[3]) * hit;
            changed = true;
        }
    }
    changed
}

/// Nearest stroke point under pixel `at` within `radius` px: (stroke, point, distance).
pub fn pick_point(d: &Drawing, view: &GpView, world: &Mat4, at: (f32, f32), radius: f32) -> Option<(usize, usize, f32)> {
    let mut best: Option<(usize, usize, f32)> = None;
    for (si, s) in d.strokes.iter().enumerate() {
        for (pi, p) in s.points.iter().enumerate() {
            let Some((x, y, _)) = view.project(world.transform_point(p.position)) else { continue };
            let dd = ((x - at.0).powi(2) + (y - at.1).powi(2)).sqrt();
            if dd <= radius && best.is_none_or(|b| dd < b.2) {
                best = Some((si, pi, dd));
            }
        }
    }
    best
}
