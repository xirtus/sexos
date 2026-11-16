// SPDX-License-Identifier: GPL-3.0-only
//! Sculpt Mode's core: brushes, strokes, masks, face sets, dynamic topology,
//! mesh filters and voxel remeshing, ported in spirit from Blender's
//! `editors/sculpt_paint` (GPL).
//!
//! A [`SculptSession`] holds what strokes need to be fast on big meshes: a
//! [`pbvh::Pbvh`] over the triangles, vertex neighbours, vertex faces and
//! vertex normals. A [`Stroke`] runs [`Dab`]s of one [`Brush`] (with
//! symmetry, locking, masking and auto-masking) and at the end turns what it
//! touched into [`crate::undo::Change`]s, so undo stores only that.
//!
//! Everything works in the mesh's object space; the caller (the 3D view)
//! converts the pointer and the brush radius into it.
pub mod brushes;
pub mod dyntopo;
pub mod ops;
pub mod pbvh;
#[cfg(test)]
mod tests;

use crate::math::Vec3;
use crate::mesh::Mesh;
use crate::undo::Change;
pub use pbvh::{Pbvh, Region};

/// Sculpt brushes (Blender's sculpt tools that paint strokes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BrushKind {
    Draw,
    DrawSharp,
    Clay,
    ClayStrips,
    ClayThumb,
    Layer,
    Inflate,
    Blob,
    Crease,
    Smooth,
    Flatten,
    Fill,
    Scrape,
    MultiplaneScrape,
    Pinch,
    Grab,
    ElasticDeform,
    SnakeHook,
    Thumb,
    Pose,
    Nudge,
    Rotate,
    SlideRelax,
    Simplify,
    Mask,
    DrawFaceSets,
}

impl BrushKind {
    pub const ALL: [BrushKind; 26] = [
        BrushKind::Draw,
        BrushKind::DrawSharp,
        BrushKind::Clay,
        BrushKind::ClayStrips,
        BrushKind::ClayThumb,
        BrushKind::Layer,
        BrushKind::Inflate,
        BrushKind::Blob,
        BrushKind::Crease,
        BrushKind::Smooth,
        BrushKind::Flatten,
        BrushKind::Fill,
        BrushKind::Scrape,
        BrushKind::MultiplaneScrape,
        BrushKind::Pinch,
        BrushKind::Grab,
        BrushKind::ElasticDeform,
        BrushKind::SnakeHook,
        BrushKind::Thumb,
        BrushKind::Pose,
        BrushKind::Nudge,
        BrushKind::Rotate,
        BrushKind::SlideRelax,
        BrushKind::Simplify,
        BrushKind::Mask,
        BrushKind::DrawFaceSets,
    ];
    pub fn label(self) -> &'static str {
        match self {
            BrushKind::Draw => "Draw",
            BrushKind::DrawSharp => "Draw Sharp",
            BrushKind::Clay => "Clay",
            BrushKind::ClayStrips => "Clay Strips",
            BrushKind::ClayThumb => "Clay Thumb",
            BrushKind::Layer => "Layer",
            BrushKind::Inflate => "Inflate",
            BrushKind::Blob => "Blob",
            BrushKind::Crease => "Crease",
            BrushKind::Smooth => "Smooth",
            BrushKind::Flatten => "Flatten",
            BrushKind::Fill => "Fill",
            BrushKind::Scrape => "Scrape",
            BrushKind::MultiplaneScrape => "Multi-plane Scrape",
            BrushKind::Pinch => "Pinch",
            BrushKind::Grab => "Grab",
            BrushKind::ElasticDeform => "Elastic Deform",
            BrushKind::SnakeHook => "Snake Hook",
            BrushKind::Thumb => "Thumb",
            BrushKind::Pose => "Pose",
            BrushKind::Nudge => "Nudge",
            BrushKind::Rotate => "Rotate",
            BrushKind::SlideRelax => "Slide Relax",
            BrushKind::Simplify => "Simplify",
            BrushKind::Mask => "Mask",
            BrushKind::DrawFaceSets => "Draw Face Sets",
        }
    }
    /// Blender's `sculpt_tool` identifier.
    pub fn id(self) -> &'static str {
        match self {
            BrushKind::Draw => "DRAW",
            BrushKind::DrawSharp => "DRAW_SHARP",
            BrushKind::Clay => "CLAY",
            BrushKind::ClayStrips => "CLAY_STRIPS",
            BrushKind::ClayThumb => "CLAY_THUMB",
            BrushKind::Layer => "LAYER",
            BrushKind::Inflate => "INFLATE",
            BrushKind::Blob => "BLOB",
            BrushKind::Crease => "CREASE",
            BrushKind::Smooth => "SMOOTH",
            BrushKind::Flatten => "FLATTEN",
            BrushKind::Fill => "FILL",
            BrushKind::Scrape => "SCRAPE",
            BrushKind::MultiplaneScrape => "MULTIPLANE_SCRAPE",
            BrushKind::Pinch => "PINCH",
            BrushKind::Grab => "GRAB",
            BrushKind::ElasticDeform => "ELASTIC_DEFORM",
            BrushKind::SnakeHook => "SNAKE_HOOK",
            BrushKind::Thumb => "THUMB",
            BrushKind::Pose => "POSE",
            BrushKind::Nudge => "NUDGE",
            BrushKind::Rotate => "ROTATE",
            BrushKind::SlideRelax => "SLIDE_RELAX",
            BrushKind::Simplify => "SIMPLIFY",
            BrushKind::Mask => "MASK",
            BrushKind::DrawFaceSets => "DRAW_FACE_SETS",
        }
    }
    pub fn from_id(s: &str) -> Option<BrushKind> {
        BrushKind::ALL.into_iter().find(|b| b.id() == s || b.label() == s)
    }
    /// Brushes that move vertices by a drag from where the stroke started
    /// (they sample the brush once and use the original coordinates).
    pub fn is_grab_like(self) -> bool {
        matches!(self, BrushKind::Grab | BrushKind::ElasticDeform | BrushKind::Thumb | BrushKind::Pose | BrushKind::Rotate)
    }
    /// Brushes that change positions (not the mask or face sets).
    pub fn deforms(self) -> bool {
        !matches!(self, BrushKind::Mask | BrushKind::DrawFaceSets)
    }
    /// Plane-based brushes (Clay, Flatten, Fill, Scrape...).
    pub fn uses_plane(self) -> bool {
        matches!(self, BrushKind::Clay | BrushKind::ClayStrips | BrushKind::ClayThumb | BrushKind::Flatten | BrushKind::Fill | BrushKind::Scrape | BrushKind::MultiplaneScrape)
    }
}

/// Falloff curve presets (Blender's `curve_preset`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Curve {
    #[default]
    Smooth,
    Smoother,
    Sphere,
    Root,
    Sharp,
    Linear,
    Sharper,
    InverseSquare,
    Constant,
}

impl Curve {
    pub const ALL: [Curve; 9] = [Curve::Smooth, Curve::Smoother, Curve::Sphere, Curve::Root, Curve::Sharp, Curve::Linear, Curve::Sharper, Curve::InverseSquare, Curve::Constant];
    pub fn label(self) -> &'static str {
        match self {
            Curve::Smooth => "Smooth",
            Curve::Smoother => "Smoother",
            Curve::Sphere => "Sphere",
            Curve::Root => "Root",
            Curve::Sharp => "Sharp",
            Curve::Linear => "Linear",
            Curve::Sharper => "Sharper",
            Curve::InverseSquare => "Inverse Square",
            Curve::Constant => "Constant",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Curve::Smooth => "SMOOTH",
            Curve::Smoother => "SMOOTHER",
            Curve::Sphere => "SPHERE",
            Curve::Root => "ROOT",
            Curve::Sharp => "SHARP",
            Curve::Linear => "LIN",
            Curve::Sharper => "POW4",
            Curve::InverseSquare => "INVSQUARE",
            Curve::Constant => "CONSTANT",
        }
    }
    pub fn from_id(s: &str) -> Option<Curve> {
        Curve::ALL.into_iter().find(|c| c.id() == s || c.label() == s)
    }
    /// Strength at `t` = distance / radius (Blender's `BKE_brush_curve_strength`).
    pub fn eval(self, t: f32) -> f32 {
        if !(0.0..1.0).contains(&t) {
            return if t < 0.0 { 1.0 } else { 0.0 };
        }
        let p = 1.0 - t;
        match self {
            Curve::Smooth => 3.0 * p * p - 2.0 * p * p * p,
            Curve::Smoother => p * p * p * (p * (p * 6.0 - 15.0) + 10.0),
            Curve::Sphere => (2.0 * p - p * p).max(0.0).sqrt(),
            Curve::Root => p.sqrt(),
            Curve::Sharp => p * p,
            Curve::Linear => p,
            Curve::Sharper => p * p * p * p,
            Curve::InverseSquare => p * (2.0 - p),
            Curve::Constant => 1.0,
        }
    }
}

/// Distance measure of the brush (Blender's `falloff_shape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FalloffShape {
    #[default]
    Sphere,
    /// A cylinder along the view: everything under the circle on screen.
    Projected,
}

/// Where plane brushes take their plane from (Blender's `sculpt_plane`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SculptPlane {
    #[default]
    Area,
    View,
    X,
    Y,
    Z,
}

impl SculptPlane {
    pub const ALL: [SculptPlane; 5] = [SculptPlane::Area, SculptPlane::View, SculptPlane::X, SculptPlane::Y, SculptPlane::Z];
    pub fn label(self) -> &'static str {
        match self {
            SculptPlane::Area => "Area Plane",
            SculptPlane::View => "View Plane",
            SculptPlane::X => "X Plane",
            SculptPlane::Y => "Y Plane",
            SculptPlane::Z => "Z Plane",
        }
    }
}

/// How a stroke places dabs (Blender's `stroke_method`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StrokeMethod {
    /// Dabs every `spacing` percent of the radius along the pointer path.
    #[default]
    Space,
    /// A dab on every pointer event.
    Dots,
    /// One dab that follows the pointer until release.
    DragDot,
    /// One dab at the press point, sized by the drag.
    Anchored,
    /// Dabs along a straight line from press to release.
    Line,
    /// Dabs repeatedly while the button is held, even without moving.
    Airbrush,
}

impl StrokeMethod {
    pub const ALL: [StrokeMethod; 6] = [StrokeMethod::Space, StrokeMethod::Dots, StrokeMethod::DragDot, StrokeMethod::Anchored, StrokeMethod::Line, StrokeMethod::Airbrush];
    pub fn label(self) -> &'static str {
        match self {
            StrokeMethod::Space => "Space",
            StrokeMethod::Dots => "Dots",
            StrokeMethod::DragDot => "Drag Dot",
            StrokeMethod::Anchored => "Anchored",
            StrokeMethod::Line => "Line",
            StrokeMethod::Airbrush => "Airbrush",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            StrokeMethod::Space => "SPACE",
            StrokeMethod::Dots => "DOTS",
            StrokeMethod::DragDot => "DRAG_DOT",
            StrokeMethod::Anchored => "ANCHORED",
            StrokeMethod::Line => "LINE",
            StrokeMethod::Airbrush => "AIRBRUSH",
        }
    }
}

/// One brush's settings (Blender's `Brush` datablock, the sculpt part).
#[derive(Clone, Debug, PartialEq)]
pub struct Brush {
    pub kind: BrushKind,
    /// Radius in pixels (view radius unit) or object units (scene unit);
    /// the view converts it into the [`Dab`]'s radius.
    pub radius: f32,
    pub scene_radius: f32,
    /// Radius in scene units instead of pixels.
    pub radius_in_scene: bool,
    pub strength: f32,
    /// Direction: subtract instead of add (Ctrl flips it during a stroke).
    pub invert: bool,
    pub curve: Curve,
    pub falloff_shape: FalloffShape,
    /// Dab spacing in percent of the radius.
    pub spacing: f32,
    pub stroke: StrokeMethod,
    pub front_faces_only: bool,
    pub automask_topology: bool,
    pub automask_face_sets: bool,
    /// Normal sampling radius, fraction of the radius.
    pub normal_radius: f32,
    /// Plane sampling radius (plane brushes), fraction of the radius.
    pub area_radius: f32,
    pub plane: SculptPlane,
    pub plane_offset: f32,
    /// Plane trim: vertices farther than this fraction of the radius from
    /// the plane are left alone.
    pub plane_trim: Option<f32>,
    /// Layer height in object units.
    pub height: f32,
    /// Pinch factor of Crease, Blob and Snake Hook.
    pub pinch: f32,
    /// Angle of Multi-plane Scrape's planes, radians.
    pub plane_angle: f32,
    /// Lazy mouse (stabilize stroke).
    pub lazy: bool,
    /// Lazy mouse distance in pixels.
    pub lazy_radius: f32,
    pub lazy_factor: f32,
    /// Hardness: the part of the radius at full strength.
    pub hardness: f32,
}

impl Brush {
    /// A brush of `kind` with Blender's defaults.
    pub fn new(kind: BrushKind) -> Brush {
        use BrushKind as K;
        let strength = match kind {
            K::Grab | K::ElasticDeform | K::SnakeHook | K::Thumb | K::Pose | K::Nudge | K::Rotate | K::Mask | K::DrawFaceSets | K::SlideRelax => 1.0,
            K::Clay | K::ClayStrips | K::ClayThumb => 0.6,
            K::Simplify => 0.5,
            K::Layer => 0.5,
            _ => 0.5,
        };
        let curve = match kind {
            K::DrawSharp => Curve::Sharper,
            K::ClayStrips => Curve::Sphere,
            K::Mask | K::DrawFaceSets => Curve::Smooth,
            _ => Curve::Smooth,
        };
        Brush {
            kind,
            radius: 50.0,
            scene_radius: 0.25,
            radius_in_scene: false,
            strength,
            invert: false,
            curve,
            falloff_shape: if kind == K::DrawFaceSets { FalloffShape::Projected } else { FalloffShape::Sphere },
            spacing: if kind == K::ClayStrips { 7.0 } else { 10.0 },
            stroke: match kind {
                K::Grab | K::ElasticDeform | K::Thumb | K::Pose | K::Rotate => StrokeMethod::DragDot,
                _ => StrokeMethod::Space,
            },
            front_faces_only: false,
            automask_topology: false,
            automask_face_sets: false,
            normal_radius: 0.5,
            area_radius: 0.5,
            plane: SculptPlane::Area,
            plane_offset: 0.0,
            plane_trim: None,
            height: 0.05,
            pinch: 0.5,
            plane_angle: 60f32.to_radians(),
            lazy: false,
            lazy_radius: 20.0,
            lazy_factor: 0.9,
            hardness: 0.0,
        }
    }

    /// Falloff at distance fraction `t` (hardness keeps the inner part at 1).
    pub fn falloff(&self, t: f32) -> f32 {
        let t = if self.hardness > 0.0 { ((t - self.hardness) / (1.0 - self.hardness).max(1e-4)).max(0.0) } else { t };
        self.curve.eval(t)
    }
}

/// Symmetry and locking (Blender's sculpt Symmetry panel).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Symmetry {
    pub mirror: [bool; 3],
    /// Vertices may not move along these axes.
    pub lock: [bool; 3],
}

impl Symmetry {
    /// The mirror passes: flip flags per axis, the identity first.
    pub fn passes(&self) -> Vec<[bool; 3]> {
        let mut v = Vec::new();
        for m in 0..8u8 {
            let f = [m & 1 != 0, m & 2 != 0, m & 4 != 0];
            if (0..3).all(|k| !f[k] || self.mirror[k]) {
                v.push(f);
            }
        }
        v
    }
}

pub fn flip(v: Vec3, f: [bool; 3]) -> Vec3 {
    Vec3::new(if f[0] { -v.x } else { v.x }, if f[1] { -v.y } else { v.y }, if f[2] { -v.z } else { v.z })
}

/// How dynamic topology picks its edge length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DetailMode {
    /// `detail_size` pixels on screen.
    #[default]
    Relative,
    /// `constant_detail` edges per object unit.
    Constant,
    /// `detail_percent` of the brush radius.
    Brush,
}

/// Dynamic topology settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DyntopoSettings {
    pub enabled: bool,
    pub mode: DetailMode,
    pub detail_size: f32,
    pub constant_detail: f32,
    pub detail_percent: f32,
    /// Subdivide long edges and/or collapse short ones.
    pub subdivide: bool,
    pub collapse: bool,
}

impl Default for DyntopoSettings {
    fn default() -> Self {
        DyntopoSettings { enabled: false, mode: DetailMode::Relative, detail_size: 12.0, constant_detail: 3.0, detail_percent: 25.0, subdivide: true, collapse: true }
    }
}

impl DyntopoSettings {
    /// Target edge length in object units for a dab of object-space radius
    /// `radius` at `units_per_pixel` object units per screen pixel.
    pub fn edge_length(&self, radius: f32, units_per_pixel: f32) -> f32 {
        match self.mode {
            DetailMode::Relative => self.detail_size.max(0.5) * units_per_pixel,
            DetailMode::Constant => 1.0 / self.constant_detail.max(1e-3),
            DetailMode::Brush => radius * self.detail_percent.max(1.0) / 100.0,
        }
    }
}

/// One application of the brush.
#[derive(Clone, Copy, Debug)]
pub struct Dab {
    /// Brush centre on the surface (object space).
    pub center: Vec3,
    /// Brush radius (object space).
    pub radius: f32,
    /// Unit vector from the surface towards the viewer (object space).
    pub view: Vec3,
    pub pressure: f32,
    /// Ctrl: the opposite direction.
    pub invert: bool,
    /// Pointer movement since the stroke began, on the stroke's plane
    /// (Grab, Thumb, Elastic Deform, Pose).
    pub grab: Vec3,
    /// Pointer movement since the last dab (Snake Hook, Nudge, Slide, the
    /// direction of Clay Strips and Multi-plane Scrape).
    pub step: Vec3,
    /// Rotate brush: angle since the stroke began (radians).
    pub angle: f32,
    /// Slide Relax with Shift: relax instead of slide.
    pub relax: bool,
    /// Dynamic topology edge length (0: off).
    pub detail: f32,
    pub dyntopo_subdivide: bool,
    pub dyntopo_collapse: bool,
}

impl Dab {
    pub fn at(center: Vec3, radius: f32, view: Vec3) -> Dab {
        Dab { center, radius, view, pressure: 1.0, invert: false, grab: Vec3::ZERO, step: Vec3::ZERO, angle: 0.0, relax: false, detail: 0.0, dyntopo_subdivide: true, dyntopo_collapse: true }
    }
}

/// What a stroke or brush hit under the pointer.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub pos: Vec3,
    pub normal: Vec3,
    pub face: u32,
    pub dist: f32,
}

/// Per-mesh acceleration data for sculpting.
#[derive(Clone, Debug, Default)]
pub struct SculptSession {
    pub pbvh: Pbvh,
    adj_start: Vec<u32>,
    adj: Vec<u32>,
    vf_start: Vec<u32>,
    vf: Vec<u32>,
    pub normals: Vec<Vec3>,
    pub boundary: Vec<bool>,
    /// Connected part of every vertex (topology auto-masking).
    pub component: Vec<u32>,
    /// Vertices whose faces are all hidden.
    pub vert_hidden: Vec<bool>,
    /// (vertices, faces) the session was built for.
    pub topology: (usize, usize),
    stamp: Vec<u32>,
    stamp_gen: u32,
}

impl SculptSession {
    pub fn new(m: &Mesh) -> SculptSession {
        let mut s = SculptSession::default();
        s.rebuild(m);
        s
    }

    /// Rebuild everything (after topology changes).
    pub fn rebuild(&mut self, m: &Mesh) {
        let nv = m.positions.len();
        self.pbvh = Pbvh::build(m);
        // Neighbours (unique) and boundary flags from the edges.
        let mut edges: Vec<(u32, u32, u32)> = Vec::with_capacity(m.faces.len() * 4);
        for f in &m.faces {
            for (a, b) in f.edges() {
                if a != b {
                    edges.push((a.min(b), a.max(b), 1));
                }
            }
        }
        edges.sort_unstable();
        let mut uniq: Vec<(u32, u32, u32)> = Vec::with_capacity(edges.len());
        for e in edges {
            match uniq.last_mut() {
                Some(l) if l.0 == e.0 && l.1 == e.1 => l.2 += 1,
                _ => uniq.push(e),
            }
        }
        let mut cnt = vec![0u32; nv + 1];
        for &(a, b, _) in &uniq {
            cnt[a as usize + 1] += 1;
            cnt[b as usize + 1] += 1;
        }
        for i in 0..nv {
            cnt[i + 1] += cnt[i];
        }
        let mut fill = cnt.clone();
        self.adj = vec![0; cnt[nv] as usize];
        self.boundary = vec![false; nv];
        for &(a, b, n) in &uniq {
            self.adj[fill[a as usize] as usize] = b;
            fill[a as usize] += 1;
            self.adj[fill[b as usize] as usize] = a;
            fill[b as usize] += 1;
            if n == 1 {
                self.boundary[a as usize] = true;
                self.boundary[b as usize] = true;
            }
        }
        self.adj_start = cnt;
        // Vertex faces.
        let mut cnt = vec![0u32; nv + 1];
        for f in &m.faces {
            for &v in &f.verts {
                cnt[v as usize + 1] += 1;
            }
        }
        for i in 0..nv {
            cnt[i + 1] += cnt[i];
        }
        let mut fill = cnt.clone();
        self.vf = vec![0; cnt[nv] as usize];
        for (fi, f) in m.faces.iter().enumerate() {
            for &v in &f.verts {
                self.vf[fill[v as usize] as usize] = fi as u32;
                fill[v as usize] += 1;
            }
        }
        self.vf_start = cnt;
        // Components by union-find.
        let mut parent: Vec<u32> = (0..nv as u32).collect();
        fn find(p: &mut [u32], mut x: u32) -> u32 {
            while p[x as usize] != x {
                p[x as usize] = p[p[x as usize] as usize];
                x = p[x as usize];
            }
            x
        }
        for &(a, b, _) in &uniq {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb {
                parent[ra.max(rb) as usize] = ra.min(rb);
            }
        }
        self.component = (0..nv as u32).map(|v| find(&mut parent, v)).collect();
        self.normals = vec![Vec3::ZERO; nv];
        let all: Vec<u32> = (0..nv as u32).collect();
        self.update_normals(m, &all);
        self.update_hidden(m);
        self.stamp = vec![0; nv];
        self.stamp_gen = 0;
        self.topology = (nv, m.faces.len());
    }

    /// Whether the session still fits `m`'s topology.
    pub fn fits(&self, m: &Mesh) -> bool {
        self.topology == (m.positions.len(), m.faces.len())
    }

    pub fn update_hidden(&mut self, m: &Mesh) {
        let nv = m.positions.len();
        if m.hidden_faces.iter().all(|h| !h) {
            self.vert_hidden = Vec::new();
            return;
        }
        self.vert_hidden = (0..nv).map(|v| {
            let fs = self.vert_faces(v as u32);
            !fs.is_empty() && fs.iter().all(|&f| m.is_face_hidden(f as usize))
        }).collect();
    }

    pub fn neighbors(&self, v: u32) -> &[u32] {
        match (self.adj_start.get(v as usize), self.adj_start.get(v as usize + 1)) {
            (Some(&a), Some(&b)) => &self.adj[a as usize..b as usize],
            _ => &[],
        }
    }

    pub fn vert_faces(&self, v: u32) -> &[u32] {
        match (self.vf_start.get(v as usize), self.vf_start.get(v as usize + 1)) {
            (Some(&a), Some(&b)) => &self.vf[a as usize..b as usize],
            _ => &[],
        }
    }

    pub fn is_hidden(&self, v: u32) -> bool {
        self.vert_hidden.get(v as usize).copied().unwrap_or(false)
    }

    /// Recompute the normals of `verts` and of every vertex sharing a face
    /// with them.
    pub fn update_normals(&mut self, m: &Mesh, verts: &[u32]) {
        let gen = self.next_stamp();
        let mut todo: Vec<u32> = Vec::with_capacity(verts.len() * 2);
        for &v in verts {
            let (a, b) = match (self.vf_start.get(v as usize), self.vf_start.get(v as usize + 1)) {
                (Some(&a), Some(&b)) => (a as usize, b as usize),
                _ => continue,
            };
            for i in a..b {
                let f = self.vf[i] as usize;
                for &w in &m.faces[f].verts {
                    if self.stamp[w as usize] != gen {
                        self.stamp[w as usize] = gen;
                        todo.push(w);
                    }
                }
            }
        }
        for v in todo {
            let mut n = Vec3::ZERO;
            for &f in self.vert_faces(v) {
                n += newell(m, f as usize);
            }
            self.normals[v as usize] = n.normalize();
        }
    }

    fn next_stamp(&mut self) -> u32 {
        if self.stamp.len() < self.normals.len() {
            self.stamp.resize(self.normals.len(), 0);
        }
        self.stamp_gen = self.stamp_gen.wrapping_add(1);
        if self.stamp_gen == 0 {
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.stamp_gen = 1;
        }
        self.stamp_gen
    }

    /// Vertices in `r` with their distance fraction (0 centre .. 1 edge).
    pub fn verts_in(&mut self, m: &Mesh, r: &Region) -> Vec<(u32, f32)> {
        let leaves = self.pbvh.leaves_in(r);
        let gen = self.next_stamp();
        let rad = r.radius().max(1e-12);
        let mut out = Vec::new();
        for l in leaves {
            for &v in self.pbvh.leaf_verts(l) {
                let s = &mut self.stamp[v as usize];
                if *s == gen {
                    continue;
                }
                *s = gen;
                let d = r.dist(m.positions[v as usize]);
                if d < rad {
                    out.push((v, d / rad));
                }
            }
        }
        out
    }

    /// The surface under a ray (object space), skipping hidden faces.
    pub fn raycast(&self, m: &Mesh, o: Vec3, d: Vec3) -> Option<Hit> {
        let skip = |f: u32| m.is_face_hidden(f as usize);
        let (t, ti, u, v) = self.pbvh.raycast(&m.positions, o, d, &skip)?;
        let [a, b, c] = self.pbvh.tris[ti as usize];
        let pos = o + d * t;
        let n = (self.normals[a as usize] * (1.0 - u - v) + self.normals[b as usize] * u + self.normals[c as usize] * v).normalize();
        let n = if n == Vec3::ZERO { (m.positions[b as usize] - m.positions[a as usize]).cross(m.positions[c as usize] - m.positions[a as usize]).normalize() } else { n };
        Some(Hit { pos, normal: n, face: self.pbvh.tri_face[ti as usize], dist: t })
    }

    /// The vertex nearest to `p` among those within `radius`.
    pub fn nearest_vert(&mut self, m: &Mesh, p: Vec3, radius: f32) -> Option<u32> {
        let hits = self.verts_in(m, &Region::Sphere { center: p, radius: radius.max(1e-6) });
        hits.into_iter().min_by(|a, b| a.1.total_cmp(&b.1)).map(|h| h.0)
    }
}

/// Unnormalised Newell normal (twice the area for planar faces).
pub(crate) fn newell(m: &Mesh, f: usize) -> Vec3 {
    let face = &m.faces[f];
    let mut n = Vec3::ZERO;
    for (a, b) in face.edges() {
        let (p, q) = (m.positions[a as usize], m.positions[b as usize]);
        n.x += (p.y - q.y) * (p.z + q.z);
        n.y += (p.z - q.z) * (p.x + q.x);
        n.z += (p.x - q.x) * (p.y + q.y);
    }
    n
}

/// Stroke state per symmetry pass of a grab-like brush.
#[derive(Clone, Debug)]
pub(crate) struct GrabPass {
    pub center: Vec3,
    pub normal: Vec3,
    pub verts: Vec<(u32, f32)>,
}

/// A stroke in progress.
pub struct Stroke {
    pub brush: Brush,
    pub symmetry: Symmetry,
    /// Positions, normals and mask when the stroke began.
    pub(crate) orig: Vec<Vec3>,
    pub(crate) orig_normals: Vec<Vec3>,
    orig_mask: Vec<f32>,
    orig_sets: Vec<i32>,
    /// The whole mesh, kept when dynamic topology may change it.
    orig_mesh: Option<Mesh>,
    pub(crate) topology_changed: bool,
    pub(crate) touched: Vec<bool>,
    pub(crate) layer: Vec<f32>,
    pub(crate) grab: Vec<GrabPass>,
    /// Auto-masking: (component, face set) under the stroke start, per pass.
    pub(crate) auto: Vec<(Option<u32>, Option<i32>)>,
    /// Face set Draw Face Sets paints.
    pub face_set: i32,
    pub(crate) first: bool,
    /// Last dab's stroke direction (Clay Strips).
    pub(crate) last_dir: Vec3,
    pub dabs: usize,
    /// The last dab changed the topology (the session was rebuilt; redraw
    /// the whole mesh).
    pub rebuilt: bool,
}

impl Stroke {
    /// Begin a stroke on `m`. With `dyntopo` the whole mesh is kept for undo.
    pub fn new(m: &Mesh, ss: &SculptSession, brush: &Brush, symmetry: Symmetry, dyntopo: bool) -> Stroke {
        let deforms = brush.kind.deforms();
        let face_set = m.face_sets.iter().copied().max().unwrap_or(1) + 1;
        Stroke {
            brush: brush.clone(),
            symmetry,
            orig: if deforms { m.positions.clone() } else { Vec::new() },
            orig_normals: if deforms { ss.normals.clone() } else { Vec::new() },
            orig_mask: if brush.kind == BrushKind::Mask { m.mask.clone() } else { Vec::new() },
            orig_sets: if brush.kind == BrushKind::DrawFaceSets { m.face_sets.clone() } else { Vec::new() },
            orig_mesh: dyntopo.then(|| m.clone()),
            topology_changed: false,
            touched: vec![false; m.positions.len()],
            layer: Vec::new(),
            grab: Vec::new(),
            auto: Vec::new(),
            face_set,
            first: true,
            last_dir: Vec3::ZERO,
            dabs: 0,
            rebuilt: false,
        }
    }

    /// Apply one dab. Returns the vertices whose positions (or mask) changed;
    /// after a topology change the session is rebuilt and the list is empty
    /// (redraw everything).
    pub fn dab(&mut self, m: &mut Mesh, ss: &mut SculptSession, d: &Dab) -> Vec<u32> {
        if !ss.fits(m) {
            ss.rebuild(m);
        }
        if self.touched.len() < m.positions.len() {
            self.touched.resize(m.positions.len(), false);
        }
        self.rebuilt = false;
        let changed = brushes::apply(self, m, ss, d);
        self.first = false;
        self.dabs += 1;
        if d.detail > 0.0 && self.brush.kind.deforms() && !self.brush.kind.is_grab_like() {
            let collapse = d.dyntopo_collapse || self.brush.kind == BrushKind::Simplify;
            let r = dyntopo::update(m, d.center, d.radius, d.detail, d.dyntopo_subdivide, collapse, &self.symmetry);
            if r.changed() {
                self.topology_changed = true;
                ss.rebuild(m);
                self.touched = vec![true; m.positions.len()];
                self.rebuilt = true;
                return Vec::new();
            }
        }
        changed
    }

    /// Put back everything the stroke changed so far, keeping the stroke
    /// open (anchored and line strokes redo their dabs on every move).
    /// Returns the vertices that moved back.
    pub fn restore(&mut self, m: &mut Mesh, ss: &mut SculptSession) -> Vec<u32> {
        if self.topology_changed {
            if let Some(o) = &self.orig_mesh {
                *m = o.clone();
                ss.rebuild(m);
                self.topology_changed = false;
                self.touched = vec![false; m.positions.len()];
                self.layer.clear();
                self.rebuilt = true;
                return (0..m.positions.len() as u32).collect();
            }
        }
        let mut moved = Vec::new();
        for (v, t) in self.touched.iter_mut().enumerate() {
            if !*t {
                continue;
            }
            *t = false;
            if let Some(&o) = self.orig.get(v) {
                if m.positions[v] != o {
                    m.positions[v] = o;
                    moved.push(v as u32);
                }
            }
        }
        if self.orig_mask.len() == m.mask.len() && !self.orig_mask.is_empty() {
            m.mask.clone_from(&self.orig_mask);
        }
        if self.orig_sets.len() == m.face_sets.len() && !self.orig_sets.is_empty() {
            m.face_sets.clone_from(&self.orig_sets);
        }
        self.layer.clear();
        if !moved.is_empty() {
            ss.pbvh.mark_verts(&moved);
            ss.pbvh.refit(&m.positions);
            ss.update_normals(m, &moved);
        }
        moved
    }

    /// End the stroke: the undo record for scene mesh `mesh`.
    pub fn finish(self, m: &Mesh, mesh: usize) -> Vec<Change> {
        let mut out = Vec::new();
        if self.topology_changed {
            if let Some(o) = self.orig_mesh {
                out.push(Change::Mesh { mesh, data: Box::new(o) });
            }
            return out;
        }
        if !self.orig.is_empty() {
            let mut verts = Vec::new();
            let mut values = Vec::new();
            for (v, t) in self.touched.iter().enumerate() {
                if *t && self.orig.get(v).is_some_and(|o| *o != m.positions[v]) {
                    verts.push(v as u32);
                    values.push(self.orig[v]);
                }
            }
            if !verts.is_empty() {
                out.push(Change::Positions { mesh, verts, values });
            }
        }
        if self.brush.kind == BrushKind::Mask && self.orig_mask != m.mask {
            if self.orig_mask.len() == m.mask.len() {
                let mut verts = Vec::new();
                let mut values = Vec::new();
                for (v, (&a, &b)) in self.orig_mask.iter().zip(&m.mask).enumerate() {
                    if a != b {
                        verts.push(v as u32);
                        values.push(a);
                    }
                }
                out.push(Change::Mask { mesh, verts, values });
            } else {
                out.push(Change::MaskAll { mesh, values: self.orig_mask });
            }
        }
        if self.brush.kind == BrushKind::DrawFaceSets && self.orig_sets != m.face_sets {
            out.push(Change::FaceSets { mesh, values: self.orig_sets });
        }
        out
    }
}
