// SPDX-License-Identifier: GPL-3.0-only
//! Sculpt Mode, Vertex Paint and Texture Paint on the document: brush
//! settings, the per-mesh sculpt session (BVH, adjacency, normals), strokes
//! with per-stroke undo patches, Multires levels, color attributes, texture
//! paint slots and the operators (`sculpt.*`, `paint.*`, `object.multires_*`,
//! `object.voxel_remesh`, `geometry.color_attribute_*`, `image.*`).
//!
//! The 3D view drives strokes (pointer to [`Dab`]) and reads the live list
//! of changed vertices to patch its cached geometry between full rebuilds.
use crate::doc::{Doc, Mode};
use crate::ui::Op;
use sex_3d::mesh::AttrDomain;
use sex_3d::modifiers::{Modifier, MultiresData, MultiresSubdiv};
use sex_3d::nodes::Prop;
use sex_3d::paint::texture::{TexBrush, TexStroke, TpSettings};
use sex_3d::paint::vertex::{self, VpSettings, VpStroke};
use sex_3d::paint::BlendMode;
use sex_3d::scene::{Image, Material, ObjectData};
use sex_3d::sculpt::ops::{self as sops, FaceSetInit, FilterKind, MaskOp, Visibility};
use sex_3d::sculpt::{dyntopo, Brush, BrushKind, Dab, DyntopoSettings, Hit, SculptSession, Stroke, Symmetry};
use sex_3d::undo::Change;
use sex_3d::{Mat4, Mesh, Vec3};
use std::collections::BTreeSet;

/// What a stroke edits: the object's mesh, or a Multires level of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Mesh,
    Multires { md: usize, level: u32 },
}

/// Sculpt Mode tools that are not brushes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionTool {
    BoxMask,
    LassoMask,
    BoxHide,
    BoxFaceSet,
    LassoFaceSet,
    MeshFilter,
}

impl RegionTool {
    pub const ALL: [RegionTool; 6] = [RegionTool::BoxMask, RegionTool::LassoMask, RegionTool::BoxHide, RegionTool::BoxFaceSet, RegionTool::LassoFaceSet, RegionTool::MeshFilter];
    pub fn label(self) -> &'static str {
        match self {
            RegionTool::BoxMask => "Box Mask",
            RegionTool::LassoMask => "Lasso Mask",
            RegionTool::BoxHide => "Box Hide",
            RegionTool::BoxFaceSet => "Box Face Set",
            RegionTool::LassoFaceSet => "Lasso Face Set",
            RegionTool::MeshFilter => "Mesh Filter",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            RegionTool::BoxMask => "BOX_MASK",
            RegionTool::LassoMask => "LASSO_MASK",
            RegionTool::BoxHide => "BOX_HIDE",
            RegionTool::BoxFaceSet => "BOX_FACE_SET",
            RegionTool::LassoFaceSet => "LASSO_FACE_SET",
            RegionTool::MeshFilter => "MESH_FILTER",
        }
    }
    pub fn lasso(self) -> bool {
        matches!(self, RegionTool::LassoMask | RegionTool::LassoFaceSet)
    }
}

/// The live sculpt session of one mesh.
#[derive(Clone, Debug)]
struct Slot {
    oi: usize,
    mi: usize,
    target: Target,
    gen: u64,
    ss: SculptSession,
    /// The Multires level being sculpted.
    work: Option<Mesh>,
}

/// Sculpt and paint settings and runtime state (not part of undo).
#[derive(Clone, Debug)]
pub struct SculptState {
    pub kind: BrushKind,
    /// A region tool (box mask, mesh filter...) instead of the brush.
    pub region: Option<RegionTool>,
    /// Settings of every brush, in [`BrushKind::ALL`] order.
    pub brushes: Vec<Brush>,
    pub symmetry: Symmetry,
    pub dyntopo: DyntopoSettings,
    pub voxel_size: f32,
    pub voxel_preserve_volume: bool,
    pub filter: FilterKind,
    pub filter_strength: f32,
    pub filter_axes: [bool; 3],
    pub show_mask: bool,
    pub show_face_sets: bool,
    pub mask_opacity: f32,
    pub vp: VpSettings,
    pub tp: TpSettings,
    /// Texture Paint: index into [`texture_slots`] of the active object.
    pub tex_slot: usize,
    /// The Image Editor's paint image (when no object slot applies).
    pub paint_image: Option<usize>,
    /// The stencil the texture Mask brush paints, for `tex_mask_image`.
    pub tex_mask: Vec<f32>,
    pub tex_mask_image: usize,
    /// Images painted since they were last saved.
    pub dirty_images: BTreeSet<usize>,
    slot: Option<Slot>,
    /// Vertices of `live_object` moved since the viewport last patched.
    pub live: Vec<u32>,
    pub live_object: Option<usize>,
    /// The viewport must rebuild (colors, topology or visibility changed).
    pub live_full: bool,
}

impl Default for SculptState {
    fn default() -> Self {
        SculptState {
            kind: BrushKind::Draw,
            region: None,
            brushes: BrushKind::ALL.iter().map(|&k| Brush::new(k)).collect(),
            symmetry: Symmetry { mirror: [true, false, false], lock: [false; 3] },
            dyntopo: DyntopoSettings::default(),
            voxel_size: 0.05,
            voxel_preserve_volume: true,
            filter: FilterKind::Inflate,
            filter_strength: 0.25,
            filter_axes: [true; 3],
            show_mask: true,
            show_face_sets: true,
            mask_opacity: 0.75,
            vp: VpSettings::default(),
            tp: TpSettings::default(),
            tex_slot: 0,
            paint_image: None,
            tex_mask: Vec::new(),
            tex_mask_image: usize::MAX,
            dirty_images: BTreeSet::new(),
            slot: None,
            live: Vec::new(),
            live_object: None,
            live_full: false,
        }
    }
}

impl SculptState {
    pub fn brush(&self) -> &Brush {
        let i = BrushKind::ALL.iter().position(|&k| k == self.kind).unwrap_or(0);
        &self.brushes[i]
    }
    pub fn brush_mut(&mut self) -> &mut Brush {
        let i = BrushKind::ALL.iter().position(|&k| k == self.kind).unwrap_or(0);
        &mut self.brushes[i]
    }
    pub fn brush_of(&self, k: BrushKind) -> &Brush {
        let i = BrushKind::ALL.iter().position(|&b| b == k).unwrap_or(0);
        &self.brushes[i]
    }
    /// Radius (pixels) and strength of the brush of the current mode.
    pub fn radius_strength(&self, mode: Mode) -> (f32, f32) {
        match mode {
            Mode::VertexPaint => (self.vp.radius, self.vp.strength),
            Mode::TexturePaint => (self.tp.radius, self.tp.strength),
            _ => (self.brush().radius, self.brush().strength),
        }
    }
    pub fn set_radius(&mut self, mode: Mode, r: f32) {
        let r = r.clamp(1.0, 1000.0);
        match mode {
            Mode::VertexPaint => self.vp.radius = r,
            Mode::TexturePaint => self.tp.radius = r,
            _ => self.brush_mut().radius = r,
        }
    }
    pub fn set_strength(&mut self, mode: Mode, s: f32) {
        let s = s.clamp(0.0, if mode == Mode::Sculpt { 2.0 } else { 1.0 });
        match mode {
            Mode::VertexPaint => self.vp.strength = s,
            Mode::TexturePaint => self.tp.strength = s,
            _ => self.brush_mut().strength = s,
        }
    }
    /// Forget the session (another mesh, or the scene was replaced).
    pub fn reset_session(&mut self) {
        self.slot = None;
        self.live.clear();
        self.live_object = None;
    }
}

// ---------------------------------------------------------------- targets

/// The active mesh object and its mesh index.
pub fn active_mesh(doc: &Doc) -> Option<(usize, usize)> {
    let a = doc.active()?;
    match doc.scene.objects[a].data {
        ObjectData::Mesh(mi) if mi < doc.scene.meshes.len() => Some((a, mi)),
        _ => None,
    }
}

/// The first Multires modifier of object `oi`.
pub fn multires_of(doc: &Doc, oi: usize) -> Option<usize> {
    doc.scene.objects.get(oi)?.modifiers.iter().position(|m| matches!(m, Modifier::Multires { .. }))
}

/// What Sculpt Mode edits on the active object.
pub fn target(doc: &Doc) -> Option<(usize, usize, Target)> {
    let (oi, mi) = active_mesh(doc)?;
    if doc.mode == Mode::Sculpt {
        if let Some(md) = multires_of(doc, oi) {
            if let Modifier::Multires { sculpt_levels, data, .. } = &doc.scene.objects[oi].modifiers[md] {
                let level = (*sculpt_levels).min(data.total_levels());
                if level > 0 {
                    return Some((oi, mi, Target::Multires { md, level }));
                }
            }
        }
    }
    Some((oi, mi, Target::Mesh))
}

fn multires_data(doc: &Doc, oi: usize, md: usize) -> Option<&MultiresData> {
    match doc.scene.objects.get(oi)?.modifiers.get(md)? {
        Modifier::Multires { data, .. } => Some(data),
        _ => None,
    }
}

/// Make the session match the active target; returns false without one.
fn ensure_slot(doc: &mut Doc) -> bool {
    let Some((oi, mi, target)) = target(doc) else {
        doc.sculpt.slot = None;
        return false;
    };
    let gen = doc.generation;
    if let Some(s) = &doc.sculpt.slot {
        if s.oi == oi && s.mi == mi && s.target == target && s.gen == gen {
            return true;
        }
    }
    let mut old = doc.sculpt.slot.take();
    let work = match target {
        Target::Mesh => None,
        Target::Multires { md, level } => {
            let base = &doc.scene.meshes[mi];
            let mut w = multires_data(doc, oi, md).map(|d| d.level_mesh(base, level)).unwrap_or_else(|| base.clone());
            // Keep the mask and face sets of a level being sculpted.
            if let Some(Some(prev)) = old.as_ref().filter(|o| o.oi == oi && o.target == target).map(|o| o.work.as_ref()) {
                if prev.positions.len() == w.positions.len() {
                    w.mask.clone_from(&prev.mask);
                }
                if prev.faces.len() == w.faces.len() {
                    w.face_sets.clone_from(&prev.face_sets);
                    w.hidden_faces.clone_from(&prev.hidden_faces);
                }
            }
            Some(w)
        }
    };
    let m = work.as_ref().unwrap_or(&doc.scene.meshes[mi]);
    let ss = match old.take() {
        Some(mut o) if o.oi == oi && o.mi == mi => {
            o.ss.rebuild(m);
            o.ss
        }
        _ => SculptSession::new(m),
    };
    doc.sculpt.slot = Some(Slot { oi, mi, target, gen, ss, work });
    true
}

/// Run `f` on the edited mesh and its session.
pub fn with_mesh<R>(doc: &mut Doc, f: impl FnOnce(&mut Mesh, &mut SculptSession) -> R) -> Option<R> {
    if !ensure_slot(doc) {
        return None;
    }
    let mut slot = doc.sculpt.slot.take()?;
    let r = match slot.work.as_mut() {
        Some(w) => f(w, &mut slot.ss),
        None => f(&mut doc.scene.meshes[slot.mi], &mut slot.ss),
    };
    doc.sculpt.slot = Some(slot);
    Some(r)
}

/// The edited mesh (read only), with its object index.
pub fn edited_mesh(doc: &Doc) -> Option<(usize, &Mesh, &SculptSession)> {
    let s = doc.sculpt.slot.as_ref()?;
    let m = s.work.as_ref().or_else(|| doc.scene.meshes.get(s.mi))?;
    Some((s.oi, m, &s.ss))
}

/// After the document changed under us (our own commit): keep the session.
/// The viewport must rebuild after our own change: keep the session.
pub fn touch_ours(doc: &mut Doc) {
    doc.touch();
    revalidate(doc);
}

pub fn revalidate(doc: &mut Doc) {
    let gen = doc.generation;
    if let Some(s) = &mut doc.sculpt.slot {
        s.gen = gen;
    }
}

pub fn note_live(doc: &mut Doc, oi: usize, verts: &[u32], full: bool) {
    let st = &mut doc.sculpt;
    if st.live_object != Some(oi) {
        st.live.clear();
        st.live_object = Some(oi);
    }
    st.live.extend_from_slice(verts);
    st.live_full |= full;
}

/// Hand the moved vertices to the viewport: `f(object, positions, normals,
/// verts)` returns false when it could not patch (then it rebuilds).
pub fn take_live(doc: &mut Doc, f: impl FnOnce(usize, &[Vec3], &[Vec3], &[u32]) -> bool) -> bool {
    if doc.sculpt.live.is_empty() && !doc.sculpt.live_full {
        return true;
    }
    let full = std::mem::take(&mut doc.sculpt.live_full);
    let verts = std::mem::take(&mut doc.sculpt.live);
    let Some(oi) = doc.sculpt.live_object else { return false };
    if full || !simple_stack(doc, oi) {
        return false;
    }
    match edited_mesh(doc) {
        Some((o, m, ss)) if o == oi => f(oi, &m.positions, &ss.normals, &verts),
        _ => false,
    }
}

/// True when the object's evaluated mesh is its sculpted mesh (no modifier
/// but Multires at the sculpt level, no shape keys).
fn simple_stack(doc: &Doc, oi: usize) -> bool {
    let o = &doc.scene.objects[oi];
    let keys = match o.data {
        ObjectData::Mesh(mi) => doc.scene.meshes.get(mi).is_some_and(|m| m.shape_keys.is_empty()),
        _ => false,
    };
    keys && o.modifiers.iter().all(|m| matches!(m, Modifier::Multires { .. }))
}

// ---------------------------------------------------------------- strokes

/// Object-space ray hit on the edited mesh, from a world-space ray.
pub fn raycast(doc: &mut Doc, o: Vec3, d: Vec3) -> Option<(Hit, Vec3)> {
    ensure_slot(doc);
    let (oi, m, ss) = edited_mesh(doc)?;
    let w = doc.scene.world_matrix(oi);
    let inv = w.inverse();
    let lo = inv.transform_point(o);
    let ld = inv.transform_vector(d).normalize();
    let h = ss.raycast(m, lo, ld)?;
    Some((h, w.transform_point(h.pos)))
}

/// The world matrix of the edited object.
pub fn world(doc: &Doc) -> Option<Mat4> {
    let s = doc.sculpt.slot.as_ref()?;
    Some(doc.scene.world_matrix(s.oi))
}

pub enum Kind {
    Sculpt(Box<Stroke>),
    Vertex(Box<VpStroke>),
    Texture { stroke: Box<TexStroke>, image: usize, mesh: Box<Mesh>, oi: usize },
}

/// A stroke in progress.
pub struct PaintStroke {
    pub kind: Kind,
    oi: usize,
    mi: usize,
    target: Target,
    label: String,
    /// Multires: the modifier before the stroke (undo).
    before: Option<Box<Modifier>>,
}

/// Start a sculpt stroke with `brush`.
pub fn begin_sculpt(doc: &mut Doc, brush: &Brush) -> Option<PaintStroke> {
    let dyn_on = doc.sculpt.dyntopo.enabled;
    let sym = doc.sculpt.symmetry;
    let (oi, mi, target) = target(doc)?;
    let multires = matches!(target, Target::Multires { .. });
    let stroke = with_mesh(doc, |m, ss| Stroke::new(m, ss, brush, sym, dyn_on && !multires))?;
    let before = match target {
        Target::Multires { md, .. } => Some(Box::new(doc.scene.objects[oi].modifiers[md].clone())),
        Target::Mesh => None,
    };
    Some(PaintStroke { kind: Kind::Sculpt(Box::new(stroke)), oi, mi, target, label: brush.kind.label().to_string(), before })
}

/// One sculpt dab.
pub fn dab(doc: &mut Doc, st: &mut PaintStroke, d: &Dab) {
    let oi = st.oi;
    match &mut st.kind {
        Kind::Sculpt(s) => {
            let mut d = *d;
            if matches!(st.target, Target::Multires { .. }) {
                d.detail = 0.0;
            }
            let Some(changed) = with_mesh(doc, |m, ss| s.dab(m, ss, &d)) else { return };
            let colors = matches!(s.brush.kind, BrushKind::Mask | BrushKind::DrawFaceSets);
            note_live(doc, oi, &changed, s.rebuilt || colors);
        }
        Kind::Vertex(s) => {
            let vp = doc.sculpt.vp.clone();
            let Some(changed) = with_mesh(doc, |m, ss| s.dab(m, ss, &vp, d)) else { return };
            note_live(doc, oi, &changed, !changed.is_empty());
        }
        Kind::Texture { .. } => {}
    }
}

/// Start a Vertex Paint stroke on the active color attribute (one is
/// added when there is none).
pub fn begin_vertex(doc: &mut Doc) -> Option<PaintStroke> {
    let (oi, mi) = active_mesh(doc)?;
    if doc.scene.meshes[mi].color_attributes.is_empty() {
        doc.push_undo("Add Color Attribute");
        let m = &mut doc.scene.meshes[mi];
        m.add_color_attribute("Color", AttrDomain::Corner, [1.0; 4]);
        doc.touch();
    }
    let attr = doc.scene.meshes[mi].active_color.min(doc.scene.meshes[mi].color_attributes.len() - 1);
    let stroke = VpStroke::new(&doc.scene.meshes[mi], attr)?;
    ensure_slot(doc);
    Some(PaintStroke { kind: Kind::Vertex(Box::new(stroke)), oi, mi, target: Target::Mesh, label: "Vertex Paint".into(), before: None })
}

/// Start a Texture Paint stroke from the 3D view on the active paint slot.
pub fn begin_texture(doc: &mut Doc) -> Option<PaintStroke> {
    let (oi, mi) = active_mesh(doc)?;
    let Some(image) = paint_image(doc) else {
        doc.report = "Missing texture: add a texture paint slot (Properties, Texture Slots)".into();
        return None;
    };
    let mesh = doc.scene.evaluated_mesh(oi).unwrap_or_else(|| doc.scene.meshes[mi].clone());
    if mesh.faces.iter().all(|f| f.uvs.len() != f.verts.len()) {
        doc.report = "Missing UVs: unwrap the mesh first".into();
        return None;
    }
    let stroke = TexStroke::new(image, &doc.scene.images[image]);
    Some(PaintStroke { kind: Kind::Texture { stroke: Box::new(stroke), image, mesh: Box::new(mesh), oi }, oi, mi, target: Target::Mesh, label: "Texture Paint".into(), before: None })
}

/// Start a 2D paint stroke on image `image` (Image Editor).
pub fn begin_image(doc: &mut Doc, image: usize) -> Option<PaintStroke> {
    let img = doc.scene.images.get(image)?;
    let stroke = TexStroke::new(image, img);
    Some(PaintStroke { kind: Kind::Texture { stroke: Box::new(stroke), image, mesh: Box::new(Mesh::new()), oi: usize::MAX }, oi: usize::MAX, mi: usize::MAX, target: Target::Mesh, label: "Image Paint".into(), before: None })
}

fn tex_mask_for(st: &mut SculptState, image: usize, len: usize) {
    if st.tex_mask_image != image || st.tex_mask.len() != len {
        st.tex_mask = vec![0.0; len];
        st.tex_mask_image = image;
    }
}

/// One texture dab from the 3D view: `project` maps object space to
/// (screen x, y, depth with smaller nearer). Returns texels written.
#[allow(clippy::too_many_arguments)]
pub fn texture_dab(doc: &mut Doc, st: &mut PaintStroke, project: &dyn Fn(Vec3) -> Option<(f32, f32, f32)>, at: (f32, f32), pressure: f32, step: (f32, f32), invert: bool) -> usize {
    let Kind::Texture { stroke, image, mesh, .. } = &mut st.kind else { return 0 };
    let image = *image;
    let tp = doc.sculpt.tp.clone();
    let Some(img) = doc.scene.images.get_mut(image) else { return 0 };
    let len = img.pixels.len();
    tex_mask_for(&mut doc.sculpt, image, len);
    let Some(img) = doc.scene.images.get_mut(image) else { return 0 };
    let n = stroke.dab_3d(img, mesh, project, &tp, at, tp.radius, pressure, step, invert, Some(&mut doc.sculpt.tex_mask));
    if n > 0 {
        doc.sculpt.dirty_images.insert(image);
    }
    n
}

/// One 2D dab at texel `at` with radius `radius` texels.
pub fn image_dab(doc: &mut Doc, st: &mut PaintStroke, at: (f32, f32), radius: f32, pressure: f32, step: (f32, f32), invert: bool) -> usize {
    let Kind::Texture { stroke, image, .. } = &mut st.kind else { return 0 };
    let image = *image;
    let tp = doc.sculpt.tp.clone();
    let Some(img) = doc.scene.images.get(image) else { return 0 };
    let len = img.pixels.len();
    tex_mask_for(&mut doc.sculpt, image, len);
    let Some(img) = doc.scene.images.get_mut(image) else { return 0 };
    let n = stroke.dab_2d(img, &tp, at, radius, pressure, step, invert, Some(&mut doc.sculpt.tex_mask));
    if n > 0 {
        doc.sculpt.dirty_images.insert(image);
    }
    n
}

/// The Clone brush's source offset (texels), set by Ctrl+click.
pub fn set_clone_offset(st: &mut PaintStroke, off: (f32, f32)) {
    if let Kind::Texture { stroke, .. } = &mut st.kind {
        stroke.clone_offset = off;
    }
}

/// Undo the stroke so far without ending it (anchored and line strokes
/// re-apply from the start on every move).
pub fn restore(doc: &mut Doc, st: &mut PaintStroke) {
    if let Kind::Sculpt(s) = &mut st.kind {
        let oi = st.oi;
        let moved = with_mesh(doc, |m, ss| s.restore(m, ss)).unwrap_or_default();
        note_live(doc, oi, &moved, false);
    }
}

/// End a stroke: one undo step with what it changed.
pub fn end(doc: &mut Doc, st: PaintStroke) {
    let PaintStroke { kind, mi, target, label, before, .. } = st;
    let changes: Vec<Change> = match kind {
        Kind::Sculpt(s) => {
            let ch = with_mesh(doc, |m, _| s.finish(m, mi)).unwrap_or_default();
            if let Target::Multires { md, level } = target {
                let oi = doc.sculpt.slot.as_ref().map(|s| s.oi).unwrap_or(usize::MAX);
                let pos = doc.sculpt.slot.as_ref().and_then(|s| s.work.as_ref()).map(|w| w.positions.clone());
                if let (Some(pos), Some(o)) = (pos, doc.scene.objects.get(oi)) {
                    let base = doc.scene.meshes[mi].clone();
                    let mut md_now = o.modifiers[md].clone();
                    if let Modifier::Multires { data, .. } = &mut md_now {
                        data.store(&base, level, &pos);
                    }
                    let changed = Some(&md_now) != before.as_deref();
                    doc.scene.objects[oi].modifiers[md] = md_now;
                    if changed {
                        if let Some(b) = before {
                            doc.push_undo_patch(&label, vec![Change::Modifier { object: oi, index: md, data: b }]);
                        }
                    }
                }
                doc.touch();
                revalidate(doc);
                doc.sculpt.live_full = false;
                doc.sculpt.live.clear();
                return;
            }
            ch
        }
        Kind::Vertex(s) => s.finish(&doc.scene.meshes[mi], mi),
        Kind::Texture { stroke, image, .. } => match doc.scene.images.get(image) {
            Some(img) => stroke.undo.finish(img),
            None => Vec::new(),
        },
    };
    if !changes.is_empty() {
        doc.push_undo_patch(&label, changes);
    }
    doc.touch();
    revalidate(doc);
    doc.sculpt.live_full = false;
    doc.sculpt.live.clear();
}

// ---------------------------------------------------------------- texture slots

/// An Image Texture node of one of the object's materials.
#[derive(Clone, Debug, PartialEq)]
pub struct TexSlot {
    pub material: usize,
    pub node: usize,
    pub image: usize,
    pub name: String,
}

pub fn texture_slots(doc: &Doc, oi: usize) -> Vec<TexSlot> {
    let mut out = Vec::new();
    let Some(o) = doc.scene.objects.get(oi) else { return out };
    for &mi in &o.materials {
        let Some(t) = doc.scene.materials.get(mi).and_then(|m| m.node_tree.as_ref()) else { continue };
        for (ni, n) in t.nodes.iter().enumerate() {
            if n.idname != "ShaderNodeTexImage" {
                continue;
            }
            if let Some(Prop::Id(Some(img))) = n.prop("image") {
                if let Some(im) = doc.scene.images.get(*img) {
                    out.push(TexSlot { material: mi, node: ni, image: *img, name: im.name.clone() });
                }
            }
        }
    }
    out
}

/// The image Texture Paint paints: the active slot, else the Image
/// Editor's image.
pub fn paint_image(doc: &Doc) -> Option<usize> {
    if let Some((oi, _)) = active_mesh(doc) {
        let slots = texture_slots(doc, oi);
        if !slots.is_empty() {
            return Some(slots[doc.sculpt.tex_slot.min(slots.len() - 1)].image);
        }
    }
    doc.sculpt.paint_image.filter(|&i| i < doc.scene.images.len())
}

/// Add an image and an Image Texture node feeding `input` of the active
/// material's shader (Blender's Add Texture Paint Slot).
fn add_texture_slot(doc: &mut Doc, input: &str, size: u32, color: [f32; 4]) {
    let Some((oi, _)) = active_mesh(doc) else { return };
    doc.push_undo("Add Paint Slot");
    if doc.scene.objects[oi].materials.is_empty() {
        let name = format!("{} Material", doc.scene.objects[oi].name);
        let mut m = Material { name, ..Default::default() };
        m.node_tree = Some(crate::nodetree::seed_material_tree(&m));
        let idx = doc.scene.add_material(m);
        doc.scene.objects[oi].materials.push(idx);
    }
    let mat = doc.scene.objects[oi].materials[0];
    let oname = doc.scene.objects[oi].name.clone();
    let mut img = Image::new(&format!("{oname} {input}"), size, size);
    img.pixels.iter_mut().for_each(|p| *p = color);
    doc.scene.images.push(img);
    let image = doc.scene.images.len() - 1;
    doc.sculpt.dirty_images.insert(image);
    let m = &mut doc.scene.materials[mat];
    if m.node_tree.is_none() {
        m.node_tree = Some(crate::nodetree::seed_material_tree(m));
    }
    let Some(t) = m.node_tree.as_mut() else { return };
    let shader = crate::nodetree::surface_node(t, "ShaderNodeOutputMaterial", "Surface").or_else(|| t.find_type("ShaderNodeBsdfPrincipled"));
    let loc = shader.map(|s| t.nodes[s].location).unwrap_or([0.0, 0.0]);
    if let Some(n) = t.add("ShaderNodeTexImage", [loc[0] - 300.0, loc[1] - 40.0 * t.nodes.len() as f32 * 0.0]) {
        t.nodes[n].set_prop("image", Prop::Id(Some(image)));
        if let Some(s) = shader {
            let _ = t.link(n, "Color", s, input);
        }
        t.active = Some(n);
    }
    crate::nodetree::sync_legacy(&mut doc.scene, crate::nodetree::TreeRef::Material(mat));
    doc.sculpt.tex_slot = texture_slots(doc, oi).iter().position(|s| s.image == image).unwrap_or(0);
    doc.report = format!("Added paint slot \"{}\"", doc.scene.images[image].name);
    doc.touch();
}

/// Where `image.save` writes an image: next to the document.
pub fn image_path(doc: &Doc, image: usize) -> String {
    let name = doc.scene.images.get(image).map(|i| i.name.clone()).unwrap_or_else(|| "image".into());
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    let dir = doc.path.as_deref().and_then(|p| p.rsplit_once('/').map(|x| x.0.to_string()));
    match dir {
        Some(d) => format!("{d}/{safe}.png"),
        None => format!("{safe}.png"),
    }
}

fn save_image(doc: &mut Doc, image: usize, path: &str) {
    let Some(img) = doc.scene.images.get(image) else {
        doc.report = "No image to save".into();
        return;
    };
    let png = sex_3d::io::encode_png(img);
    match std::fs::write(path, png) {
        Ok(()) => {
            doc.sculpt.dirty_images.remove(&image);
            doc.report = format!("Saved image \"{path}\"");
        }
        Err(e) => doc.report = format!("Cannot write {path}: {e}"),
    }
}

// ---------------------------------------------------------------- operators

/// Pixels per object unit are not known here: Relative detail uses this
/// world size per pixel when no view gave one.
pub const DEFAULT_UNITS_PER_PIXEL: f32 = 0.004;

fn parse_kind(s: &str) -> Option<BrushKind> {
    BrushKind::from_id(s).or_else(|| BrushKind::ALL.into_iter().find(|k| k.label().eq_ignore_ascii_case(s)))
}

/// Sculpt and paint operators. Returns false for names it does not know.
pub fn run(doc: &mut Doc, op: &Op) -> bool {
    let id = op.id.as_str();
    let arg = op.arg.trim();
    let ours = id.starts_with("sculpt.")
        || id.starts_with("object.multires_")
        || id == "object.voxel_remesh"
        || id.starts_with("geometry.color_attribute")
        || matches!(id, "paint.mask_flood_fill" | "paint.mask_filter" | "paint.hide_show" | "paint.vertex_color_set" | "paint.vertex_brush" | "paint.texture_brush" | "paint.add_texture_paint_slot" | "paint.texture_slot_set" | "paint.blend" | "paint.color_set")
        || matches!(id, "image.save" | "image.save_as" | "image.new" | "image.paint_select");
    if !ours {
        return false;
    }
    match id {
        "sculpt.brush_select" => match parse_kind(arg) {
            Some(k) => {
                doc.sculpt.kind = k;
                doc.sculpt.region = None;
                if doc.mode != Mode::Sculpt {
                    crate::rig::mode_set(doc, "SCULPT");
                }
            }
            None => doc.report = format!("Unknown sculpt brush \"{arg}\""),
        },
        "sculpt.tool_select" => match RegionTool::ALL.into_iter().find(|t| t.id() == arg || t.label() == arg) {
            Some(t) => doc.sculpt.region = Some(t),
            None => doc.report = format!("Unknown sculpt tool \"{arg}\""),
        },
        "sculpt.curve" => {
            if let Some(c) = sex_3d::sculpt::Curve::from_id(arg) {
                doc.sculpt.brush_mut().curve = c;
                doc.sculpt.vp.curve = c;
                doc.sculpt.tp.curve = c;
            }
        }
        "sculpt.stroke_method" => {
            use sex_3d::sculpt::StrokeMethod as M;
            if let Some(m) = [M::Space, M::Dots, M::DragDot, M::Anchored, M::Line, M::Airbrush].into_iter().find(|m| m.id() == arg) {
                doc.sculpt.brush_mut().stroke = m;
            }
        }
        "sculpt.filter_set" => {
            if let Some(k) = FilterKind::from_id(arg) {
                doc.sculpt.filter = k;
                doc.sculpt.region = Some(RegionTool::MeshFilter);
            }
        }
        "sculpt.radius" => {
            if let Ok(v) = arg.parse::<f32>() {
                doc.sculpt.set_radius(doc.mode, v);
            }
        }
        "sculpt.strength" => {
            if let Ok(v) = arg.parse::<f32>() {
                doc.sculpt.set_strength(doc.mode, v);
            }
        }
        "sculpt.symmetry" => {
            // "X", "Y", "Z" toggle mirroring; "LOCK_X"... toggle locks.
            let (lock, ax) = match arg.strip_prefix("LOCK_") {
                Some(a) => (true, a),
                None => (false, arg),
            };
            if let Some(k) = ["X", "Y", "Z"].iter().position(|a| a.eq_ignore_ascii_case(ax)) {
                let v = if lock { &mut doc.sculpt.symmetry.lock[k] } else { &mut doc.sculpt.symmetry.mirror[k] };
                *v = !*v;
            }
        }
        "sculpt.dynamic_topology_toggle" => dyntopo_toggle(doc),
        "sculpt.detail_flood_fill" => detail_flood(doc),
        "object.voxel_remesh" => voxel_remesh(doc),
        "paint.mask_flood_fill" => {
            let opm = match arg {
                "INVERT" => MaskOp::Invert,
                a => MaskOp::Fill(a.strip_prefix("VALUE").map(str::trim).and_then(|v| v.parse().ok()).unwrap_or(0.0)),
            };
            mesh_op(doc, if opm == MaskOp::Invert { "Invert Mask" } else { "Mask Flood Fill" }, |m, _| {
                sops::mask_flood(m, opm);
                true
            });
        }
        "paint.mask_filter" => {
            mesh_op(doc, "Mask Filter", |m, ss| {
                match arg {
                    "SHRINK" => sops::mask_grow(m, ss, false),
                    "SMOOTH" => sops::mask_smooth(m, ss),
                    _ => sops::mask_grow(m, ss, true),
                }
                true
            });
        }
        "sculpt.face_sets_init" => {
            let mode = match arg {
                "MATERIALS" => FaceSetInit::Materials,
                "NORMALS" => FaceSetInit::Normals(30f32.to_radians()),
                "UV_SEAMS" => FaceSetInit::UvSeams,
                "SHARP_EDGES" => FaceSetInit::SharpEdges,
                _ => FaceSetInit::LooseParts,
            };
            mesh_op(doc, "Init Face Sets", |m, _| {
                sops::face_sets_init(m, mode);
                true
            });
        }
        "sculpt.face_set_change_visibility" => {
            // "HIDE n", "ISOLATE n" (show only n), "SHOW", "INVERT"
            let mut it = arg.split_whitespace();
            let what = it.next().unwrap_or("SHOW");
            let n: i32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(1);
            let v = match what {
                "HIDE" => Visibility::HideSet(n),
                "ISOLATE" | "TOGGLE" => Visibility::Isolate(n),
                "INVERT" => Visibility::Invert,
                _ => Visibility::ShowAll,
            };
            mesh_op(doc, "Face Set Visibility", |m, ss| {
                sops::face_set_visibility(m, v);
                ss.update_hidden(m);
                true
            });
        }
        "paint.hide_show" if arg == "SHOW" => {
            mesh_op(doc, "Show All", |m, ss| {
                m.hidden_faces.clear();
                ss.update_hidden(m);
                true
            });
        }
        "sculpt.mesh_filter" => {
            let kind = FilterKind::from_id(arg).unwrap_or(doc.sculpt.filter);
            let (amount, axes) = (doc.sculpt.filter_strength, doc.sculpt.filter_axes);
            let seed = doc.generation as u32;
            mesh_op(doc, "Mesh Filter", |m, ss| !sops::mesh_filter(m, ss, kind, amount, axes, seed).is_empty());
        }
        "object.multires_subdivide" => {
            let mode = MultiresSubdiv::from_id(arg).unwrap_or_default();
            multires_op(doc, "Multires Subdivide", |data, base| {
                data.subdivide(base, mode);
                None
            });
        }
        "object.multires_unsubdivide" => {
            let mut fail = false;
            multires_op(doc, "Multires Unsubdivide", |data, base| match data.unsubdivide(base) {
                Some(nb) => Some(nb),
                None => {
                    fail = true;
                    None
                }
            });
            if fail {
                doc.report = "Unsubdivide needs a grid-like base mesh (all quads from subdivision)".into();
            }
        }
        "object.multires_base_apply" => {
            multires_op(doc, "Apply Base", |data, base| {
                let mut nb = base.clone();
                nb.positions = data.apply_base(base);
                Some(nb)
            });
        }
        "object.multires_higher_levels_delete" => {
            let Some((oi, _)) = active_mesh(doc) else { return true };
            let Some(md) = multires_of(doc, oi) else { return true };
            let lvl = match &doc.scene.objects[oi].modifiers[md] {
                Modifier::Multires { sculpt_levels, .. } => *sculpt_levels,
                _ => 0,
            };
            multires_op(doc, "Delete Higher", |data, _| {
                data.delete_higher(lvl);
                None
            });
        }
        "geometry.color_attribute_add" => {
            let Some((_, mi)) = active_mesh(doc) else { return true };
            let mut it = arg.split('\t');
            let name = it.next().filter(|s| !s.is_empty()).unwrap_or("Color").to_string();
            let domain = it.next().and_then(AttrDomain::from_id).unwrap_or(AttrDomain::Corner);
            doc.push_undo("Add Color Attribute");
            let m = &mut doc.scene.meshes[mi];
            let mut nm = name.clone();
            let mut k = 1;
            while m.color_attributes.iter().any(|a| a.name == nm) {
                nm = format!("{name}.{k:03}");
                k += 1;
            }
            let i = m.add_color_attribute(&nm, domain, [1.0; 4]);
            m.active_color = i;
            doc.touch();
        }
        "geometry.color_attribute_remove" => {
            let Some((_, mi)) = active_mesh(doc) else { return true };
            let m = &doc.scene.meshes[mi];
            if m.color_attributes.is_empty() {
                return true;
            }
            let i = arg.parse::<usize>().unwrap_or(m.active_color);
            doc.push_undo("Remove Color Attribute");
            doc.scene.meshes[mi].remove_color_attribute(i);
            doc.touch();
        }
        "geometry.color_attribute_set_active" | "geometry.color_attribute_render_set" => {
            let Some((_, mi)) = active_mesh(doc) else { return true };
            let m = &mut doc.scene.meshes[mi];
            let i = arg.parse::<usize>().ok().or_else(|| m.color_attributes.iter().position(|a| a.name == arg));
            if let Some(i) = i.filter(|&i| i < m.color_attributes.len()) {
                if id.ends_with("render_set") {
                    m.render_color = i;
                } else {
                    m.active_color = i;
                }
            }
            doc.touch();
        }
        "paint.vertex_color_set" => {
            let Some((_, mi)) = active_mesh(doc) else { return true };
            if doc.scene.meshes[mi].color_attributes.is_empty() {
                doc.scene.meshes[mi].add_color_attribute("Color", AttrDomain::Corner, [1.0; 4]);
            }
            let (color, blend, strength) = (doc.sculpt.vp.color, doc.sculpt.vp.blend, doc.sculpt.vp.strength);
            doc.push_undo("Set Vertex Colors");
            let m = &mut doc.scene.meshes[mi];
            let a = m.active_color.min(m.color_attributes.len() - 1);
            vertex::fill(m, a, color, blend, strength);
            doc.touch();
        }
        "paint.vertex_brush" => {
            if let Some(b) = sex_3d::paint::vertex::VpBrush::ALL.into_iter().find(|b| b.label().eq_ignore_ascii_case(arg)) {
                doc.sculpt.vp.brush = b;
            }
        }
        "paint.texture_brush" => {
            if let Some(b) = TexBrush::ALL.into_iter().find(|b| b.label().eq_ignore_ascii_case(arg)) {
                doc.sculpt.tp.brush = b;
            }
        }
        "paint.blend" => {
            if let Some(b) = BlendMode::from_id(arg) {
                if doc.mode == Mode::TexturePaint {
                    doc.sculpt.tp.blend = b;
                } else {
                    doc.sculpt.vp.blend = b;
                }
            }
        }
        "paint.color_set" => {
            // "r g b" (linear)
            let v: Vec<f32> = arg.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() == 3 {
                let c = [v[0], v[1], v[2]];
                doc.sculpt.vp.color = c;
                doc.sculpt.tp.color = c;
            }
        }
        "paint.add_texture_paint_slot" => {
            let input = if arg.is_empty() { "Base Color" } else { arg };
            let color = if input == "Base Color" { [0.8, 0.8, 0.8, 1.0] } else { [0.0, 0.0, 0.0, 1.0] };
            add_texture_slot(doc, input, 1024, color);
        }
        "paint.texture_slot_set" => {
            if let Ok(i) = arg.parse() {
                doc.sculpt.tex_slot = i;
            }
        }
        "image.paint_select" => {
            doc.sculpt.paint_image = arg.parse().ok().filter(|&i| i < doc.scene.images.len());
        }
        "image.new" => {
            let mut it = arg.split('\t');
            let name = it.next().filter(|s| !s.is_empty()).unwrap_or("Untitled").to_string();
            let size: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(1024).clamp(1, 8192);
            doc.push_undo("New Image");
            let mut img = Image::new(&name, size, size);
            img.pixels.iter_mut().for_each(|p| *p = [0.0, 0.0, 0.0, 1.0]);
            doc.scene.images.push(img);
            let i = doc.scene.images.len() - 1;
            doc.sculpt.paint_image = Some(i);
            doc.sculpt.dirty_images.insert(i);
            doc.touch();
        }
        "image.save" | "image.save_as" => {
            let (img, path) = match arg.split_once('\t') {
                Some((i, p)) => (i.parse().ok(), Some(p.to_string())),
                None if arg.ends_with(".png") => (paint_image(doc), Some(arg.to_string())),
                None => (arg.parse().ok().or_else(|| paint_image(doc)), None),
            };
            let Some(img) = img.filter(|&i| i < doc.scene.images.len()) else {
                doc.report = "No image to save".into();
                return true;
            };
            let path = path.unwrap_or_else(|| image_path(doc, img));
            save_image(doc, img, &path);
        }
        _ => doc.report = format!("Unknown operator {id}"),
    }
    true
}

/// An undoable whole-mesh operation on the edited mesh (mask, face sets,
/// filters): records only what changed.
pub fn mesh_op(doc: &mut Doc, label: &str, f: impl FnOnce(&mut Mesh, &mut SculptSession) -> bool) {
    let Some((oi, mi, target)) = target(doc) else { return };
    let before = with_mesh(doc, |m, _| (m.positions.clone(), m.mask.clone(), m.face_sets.clone(), m.hidden_faces.clone()));
    let Some((pos, mask, sets, hidden)) = before else { return };
    let changed = with_mesh(doc, |m, ss| {
        let r = f(m, ss);
        if r {
            let moved: Vec<u32> = (0..m.positions.len() as u32).filter(|&v| m.positions[v as usize] != pos[v as usize]).collect();
            ss.update_normals(m, &moved);
            ss.pbvh.mark_verts(&moved);
            ss.pbvh.refit(&m.positions);
        }
        r
    });
    if changed != Some(true) {
        return;
    }
    let Some((_, m, _)) = edited_mesh(doc) else { return };
    let mut ch = Vec::new();
    let mut verts = Vec::new();
    let mut values = Vec::new();
    for (v, p) in pos.iter().enumerate() {
        if m.positions.get(v) != Some(p) {
            verts.push(v as u32);
            values.push(*p);
        }
    }
    if !verts.is_empty() {
        ch.push(Change::Positions { mesh: mi, verts, values });
    }
    if mask != m.mask {
        ch.push(Change::MaskAll { mesh: mi, values: mask });
    }
    if sets != m.face_sets {
        ch.push(Change::FaceSets { mesh: mi, values: sets });
    }
    if hidden != m.hidden_faces {
        ch.push(Change::Hidden { mesh: mi, values: hidden });
    }
    if matches!(target, Target::Mesh) {
        doc.push_undo_patch(label, ch);
    } else if let Target::Multires { md, level } = target {
        // A Multires level: positions go into the modifier.
        let pos = edited_mesh(doc).map(|(_, m, _)| m.positions.clone()).unwrap_or_default();
        let before = doc.scene.objects[oi].modifiers[md].clone();
        let base = doc.scene.meshes[mi].clone();
        if let Modifier::Multires { data, .. } = &mut doc.scene.objects[oi].modifiers[md] {
            data.store(&base, level, &pos);
        }
        if doc.scene.objects[oi].modifiers[md] != before {
            doc.push_undo_patch(label, vec![Change::Modifier { object: oi, index: md, data: Box::new(before) }]);
        }
    }
    doc.touch();
    revalidate(doc);
}

/// Change the active object's first Multires modifier; `f` may return a
/// new base mesh.
fn multires_op(doc: &mut Doc, label: &str, f: impl FnOnce(&mut MultiresData, &Mesh) -> Option<Mesh>) {
    let Some((oi, mi)) = active_mesh(doc) else { return };
    let Some(md) = multires_of(doc, oi) else {
        doc.report = "No Multires modifier".into();
        return;
    };
    let base = doc.scene.meshes[mi].clone();
    let mut m = doc.scene.objects[oi].modifiers[md].clone();
    let Modifier::Multires { levels, sculpt_levels, render_levels, data, .. } = &mut m else { return };
    let total_before = data.total_levels();
    let nb = f(data, &base);
    let total = data.total_levels();
    let below = label == "Multires Unsubdivide" && nb.is_some();
    for l in [&mut *levels, &mut *sculpt_levels, &mut *render_levels] {
        *l = if below {
            // A level was added under the others: they keep their shapes.
            (*l + 1).min(total)
        } else if total > total_before {
            // Like Blender: subdividing shows and sculpts the new top level.
            total
        } else {
            (*l).min(total)
        };
    }
    doc.push_undo(label);
    doc.scene.objects[oi].modifiers[md] = m;
    if let Some(nb) = nb {
        doc.scene.meshes[mi] = nb;
    }
    doc.sculpt.reset_session();
    doc.touch();
}

fn dyntopo_toggle(doc: &mut Doc) {
    if doc.sculpt.dyntopo.enabled {
        doc.sculpt.dyntopo.enabled = false;
        doc.report = "Dynamic Topology disabled".into();
        return;
    }
    let Some((oi, mi)) = active_mesh(doc) else { return };
    if multires_of(doc, oi).is_some() {
        doc.report = "Dynamic Topology cannot be used with Multires".into();
        return;
    }
    if doc.scene.meshes[mi].faces.iter().any(|f| f.verts.len() != 3) {
        doc.push_undo("Dynamic Topology Toggle");
        dyntopo::triangulate(&mut doc.scene.meshes[mi]);
        doc.touch();
    }
    doc.sculpt.dyntopo.enabled = true;
    doc.report = "Dynamic Topology enabled".into();
}

/// Detail Flood Fill: bring the whole mesh to the constant detail size.
fn detail_flood(doc: &mut Doc) {
    let Some((_, mi)) = active_mesh(doc) else { return };
    let detail = 1.0 / doc.sculpt.dyntopo.constant_detail.max(1e-3);
    doc.push_undo("Detail Flood Fill");
    let m = &mut doc.scene.meshes[mi];
    let b = m.bounds();
    let c = (b.min + b.max) * 0.5;
    let r = (b.max - b.min).length() * 0.5 + detail;
    for _ in 0..6 {
        if !dyntopo::update(m, c, r, detail, true, true, &Symmetry::default()).changed() {
            break;
        }
    }
    doc.sculpt.reset_session();
    doc.touch();
}

fn voxel_remesh(doc: &mut Doc) {
    let Some((_, mi)) = active_mesh(doc) else { return };
    let size = doc.sculpt.voxel_size.max(1e-4);
    let m = &doc.scene.meshes[mi];
    let b = m.bounds();
    let cells = ((b.max - b.min) / size).to_array().iter().fold(1.0f64, |a, &x| a * (x.max(1.0) as f64 + 2.0));
    if cells > 64e6 {
        doc.report = format!("Voxel size {size} is too small for this mesh");
        return;
    }
    let out = sops::voxel_remesh(m, size, doc.sculpt.voxel_preserve_volume);
    if out.faces.is_empty() {
        doc.report = "Voxel remesh produced no faces".into();
        return;
    }
    doc.push_undo("Voxel Remesh");
    doc.scene.meshes[mi] = out;
    doc.sculpt.reset_session();
    doc.touch();
}

/// Mask, face set and color tints of a mesh for the viewport (linear RGB
/// per triangle corner), in the order of `tris`.
pub fn overlay_colors(doc: &Doc, oi: usize, m: &Mesh, tris: &[sex_3d::mesh::Tri]) -> Option<Vec<[[f32; 3]; 3]>> {
    match doc.mode {
        Mode::Sculpt => {
            let st = &doc.sculpt;
            // The edited mesh carries the mask (a Multires level's lives there).
            let src = match edited_mesh(doc) {
                Some((o, e, _)) if o == oi && e.positions.len() == m.positions.len() && e.faces.len() == m.faces.len() => e,
                _ => m,
            };
            let mask_on = st.show_mask && !src.mask.is_empty();
            let sets_on = st.show_face_sets && src.face_sets.iter().any(|&s| s != 1);
            if !mask_on && !sets_on {
                return None;
            }
            let base = 0.8f32;
            Some(
                tris.iter()
                    .map(|t| {
                        let fc = if sets_on { face_set_color(src.face_set(t.face as usize)) } else { [1.0; 3] };
                        t.verts.map(|v| {
                            let k = if mask_on { 1.0 - st.mask_opacity * src.mask_at(v as usize) } else { 1.0 };
                            [base * fc[0] * k, base * fc[1] * k, base * fc[2] * k]
                        })
                    })
                    .collect(),
            )
        }
        Mode::VertexPaint => {
            if m.color_attributes.is_empty() {
                return None;
            }
            let a = m.active_color.min(m.color_attributes.len() - 1);
            let off = m.corner_offsets();
            Some(tris.iter().map(|t| [0, 1, 2].map(|k| { let c = m.corner_color(a, &off, t.face as usize, t.corners[k] as usize); [c[0], c[1], c[2]] })).collect())
        }
        _ => None,
    }
}

/// Blender-like pastel colours for face sets (set 1 stays white).
pub fn face_set_color(s: i32) -> [f32; 3] {
    if s == 1 {
        return [1.0; 3];
    }
    let h = (s as u32).wrapping_mul(2_654_435_761) % 360;
    let (r, g, b) = hsv(h as f32 / 360.0, 0.45, 1.0);
    [r, g, b]
}

fn hsv(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

// ---------------------------------------------------------------- demo

/// A sculpted sphere with a mask and face sets in the Sculpting workspace
/// (`--sculpt-demo` screenshots).
pub fn demo(doc: &mut Doc) {
    if let Some(cube) = doc.scene.objects.iter().position(|o| o.name == "Cube") {
        doc.scene.remove_object(cube);
    }
    let mut m = sex_3d::primitives::ico_sphere(5, 1.0);
    for f in &mut m.faces {
        f.smooth = true;
    }
    let i = doc.scene.add_mesh_object("Sphere", m);
    doc.select_only(Some(i));
    let sym = Symmetry { mirror: [true, false, false], lock: [false; 3] };
    let on_sphere = |a: f32, z: f32| {
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * a.cos(), r * a.sin(), z)
    };
    // A brow ridge, two eye sockets, a nose and a crease, mirrored in X.
    let strokes: Vec<(BrushKind, f32, bool, Vec<Vec3>)> = vec![
        (BrushKind::ClayStrips, 0.22, false, (0..14).map(|k| on_sphere(-1.2 + k as f32 * 0.06, 0.35)).collect()),
        (BrushKind::Draw, 0.2, true, vec![on_sphere(-1.25, 0.12), on_sphere(-1.9, 0.12)]),
        (BrushKind::Inflate, 0.18, false, (0..6).map(|k| on_sphere(-std::f32::consts::FRAC_PI_2, 0.05 - k as f32 * 0.05)).collect()),
        (BrushKind::Crease, 0.12, false, (0..12).map(|k| on_sphere(-1.15 - k as f32 * 0.07, -0.4 - (k as f32 - 6.0).abs() * -0.01)).collect()),
        (BrushKind::Blob, 0.2, false, vec![on_sphere(-0.6, -0.05), on_sphere(-0.55, -0.1)]),
    ];
    for (kind, radius, invert, pts) in strokes {
        let mut b = Brush::new(kind);
        b.strength = 0.9;
        let _ = with_mesh(doc, |m, ss| {
            let mut st = Stroke::new(m, ss, &b, sym, false);
            let mut last = pts[0];
            for &p in &pts {
                for _ in 0..3 {
                    let mut d = Dab::at(p, radius, p.normalize());
                    d.invert = invert;
                    d.step = p - last;
                    st.dab(m, ss, &d);
                }
                last = p;
            }
        });
    }
    let _ = with_mesh(doc, |m, _| {
        // Mask the lower part and paint two face sets.
        m.mask = m.positions.iter().map(|p| ((-p.z - 0.15) * 3.0).clamp(0.0, 1.0)).collect();
        let centers: Vec<Vec3> = (0..m.faces.len()).map(|f| m.face_center(f)).collect();
        m.face_sets = centers.iter().map(|c| if c.z > 0.62 { 2 } else if c.x.abs() > 0.82 { 3 } else { 1 }).collect();
    });
    doc.sculpt.kind = BrushKind::ClayStrips;
    doc.sculpt.reset_session();
    doc.touch();
    doc.want_workspace = Some("Sculpting".into());
    // Face the startup view.
    doc.scene.objects[i].transform.rotation = sex_3d::Quat::from_axis_angle(Vec3::Z, std::f32::consts::FRAC_PI_2 - 0.6);
}

/// Keep sculpt state consistent after undo, redo or a file load.
pub fn sanitize(doc: &mut Doc) {
    if doc.mode != Mode::Sculpt && doc.mode != Mode::VertexPaint && doc.mode != Mode::TexturePaint {
        doc.sculpt.live.clear();
        doc.sculpt.live_full = false;
    }
    doc.sculpt.dirty_images.retain(|&i| i < doc.scene.images.len());
}
