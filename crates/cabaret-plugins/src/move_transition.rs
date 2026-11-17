//! Pure-Rust port of **obs-move-transition** for Cabaret Studios.
//!
//! Matches items across outgoing Program and incoming Preview [`Studio`]s by [`SourceId`]
//! or item name and smoothly interpolates `Transform2D`, opacity, 3D corner-pin points,
//! and numeric filter parameters with selectable easing curves and enter/exit slide vectors.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use cabaret_core::compositor::{
    Anchor, Compositor, CompositorLayer, CropRect, FrameBuffer, Transform2D,
};
use cabaret_core::filters::{
    process_filter_chain, AiMatteInput, FilterChain, FilterContext, VideoFilter,
};
use cabaret_core::scene::{
    ItemId, SceneCollection, SceneError, SceneItem, SourceId, SourceKind, Studio, StudioId,
};

use crate::streamfx_3d::{warp_corner_pin_normalized, CornerPinQuad};
use crate::util::{absf, clamp01, cosf, expf, lerp, sinf, sqrtf};

/// Selectable easing curves for Move transitions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum EasingCurve {
    Linear,
    #[default]
    CubicInOut,
    BackOut,
    Spring {
        stiffness: f32,
        damping: f32,
    },
    Bounce,
}

impl EasingCurve {
    /// Evaluates the easing curve at normalized progress `t in [0.0, 1.0]`.
    /// Note: `BackOut` and underdamped `Spring` may intentionally overshoot `1.0` mid-transition.
    pub fn eval(self, t: f32) -> f32 {
        let t = clamp01(t);
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        match self {
            Self::Linear => t,
            Self::CubicInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let k = -2.0 * t + 2.0;
                    1.0 - (k * k * k) * 0.5
                }
            }
            Self::BackOut => {
                const C1: f32 = 1.701_58;
                const C3: f32 = C1 + 1.0;
                let u = t - 1.0;
                1.0 + C3 * u * u * u + C1 * u * u
            }
            Self::Spring { stiffness, damping } => {
                let k = stiffness.max(1.0);
                let c = damping.max(0.1);
                let wn = sqrtf(k);
                let zeta = c / (2.0 * wn);

                let eval_raw = |tau: f32| -> f32 {
                    if zeta < 0.999 {
                        let wd = wn * sqrtf((1.0 - zeta * zeta).max(1e-6));
                        let env = expf(-zeta * wn * tau);
                        let sin_coeff = zeta / sqrtf((1.0 - zeta * zeta).max(1e-6));
                        1.0 - env * (cosf(wd * tau) + sin_coeff * sinf(wd * tau))
                    } else if zeta <= 1.001 {
                        1.0 - expf(-wn * tau) * (1.0 + wn * tau)
                    } else {
                        let sq = sqrtf((zeta * zeta - 1.0).max(1e-6));
                        let r1 = -wn * (zeta - sq);
                        let r2 = -wn * (zeta + sq);
                        let denom = r2 - r1;
                        if absf(denom) < 1e-6 {
                            1.0 - expf(-wn * tau) * (1.0 + wn * tau)
                        } else {
                            1.0 - (r2 * expf(r1 * tau) - r1 * expf(r2 * tau)) / denom
                        }
                    }
                };

                let v = eval_raw(t);
                let end_err = 1.0 - eval_raw(1.0);
                v + end_err * (t * t * t)
            }
            Self::Bounce => {
                const N1: f32 = 7.5625;
                const D1: f32 = 2.75;
                if t < 1.0 / D1 {
                    N1 * t * t
                } else if t < 2.0 / D1 {
                    let u = t - 1.5 / D1;
                    N1 * u * u + 0.75
                } else if t < 2.5 / D1 {
                    let u = t - 2.25 / D1;
                    N1 * u * u + 0.9375
                } else {
                    let u = t - 2.625 / D1;
                    N1 * u * u + 0.984_375
                }
            }
        }
    }
}

/// Strategy for pairing scene items between Program and Preview studios.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MatchStrategy {
    #[default]
    SourceIdFirstThenName,
    SourceIdOnly,
    NameOnly,
}

/// Auxiliary plugin state attached to a `SceneItem` for Move interpolation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoveItemExtras {
    pub corner_pin: Option<CornerPinQuad>,
    pub filter_params: BTreeMap<String, f32>,
}

/// Configuration for [`MoveTransitionPlan`].
#[derive(Clone, Debug, PartialEq)]
pub struct MoveTransitionConfig {
    pub duration_ms: u32,
    pub easing: EasingCurve,
    pub match_strategy: MatchStrategy,
    /// Offset applied at `t = 0` to entering unmatched items (slides to `[0, 0]` at `t = 1`).
    pub enter_slide: [f32; 2],
    /// Offset applied at `t = 1` to exiting unmatched items (slides from `[0, 0]` at `t = 0`).
    pub exit_slide: [f32; 2],
    /// When `true`, entering items fade in `0 -> target.opacity` and exiting items fade `from.opacity -> 0`.
    pub fade_unmatched: bool,
    /// Interpolates rotation using shortest angular path (`[-180..180]`).
    pub shortest_rotation_path: bool,
}

impl Default for MoveTransitionConfig {
    fn default() -> Self {
        Self {
            duration_ms: 450,
            easing: EasingCurve::CubicInOut,
            match_strategy: MatchStrategy::SourceIdFirstThenName,
            enter_slide: [0.0, 0.0],
            exit_slide: [0.0, 0.0],
            fade_unmatched: true,
            shortest_rotation_path: true,
        }
    }
}

/// Role of an item inside a Move transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MoveItemKind {
    Matched,
    Entering,
    Exiting,
}

#[derive(Clone, Debug)]
struct MoveTrack {
    kind: MoveItemKind,
    from_item: Option<SceneItem>,
    to_item: Option<SceneItem>,
    from_extras: MoveItemExtras,
    to_extras: MoveItemExtras,
}

/// Evaluated interpolated state of one scene item at progress `t`.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveItemState {
    pub source_id: SourceId,
    pub name: String,
    pub from_item_id: Option<ItemId>,
    pub to_item_id: Option<ItemId>,
    pub kind: MoveItemKind,
    pub item: SceneItem,
    pub corner_pin: Option<CornerPinQuad>,
    pub filter_params: BTreeMap<String, f32>,
}

/// Precomputed Move transition matching and interpolation plan between two [`Studio`]s.
#[derive(Clone, Debug)]
pub struct MoveTransitionPlan {
    pub from_studio: StudioId,
    pub to_studio: StudioId,
    pub config: MoveTransitionConfig,
    tracks: Vec<MoveTrack>,
}

impl MoveTransitionPlan {
    /// Builds a Move transition plan matching items between `from` and `to`.
    pub fn build(from: &Studio, to: &Studio, config: MoveTransitionConfig) -> Self {
        let empty = BTreeMap::new();
        Self::build_with_extras(from, to, &empty, &empty, config)
    }

    /// Builds a Move transition plan with per-item corner-pin and numeric filter parameters.
    pub fn build_with_extras(
        from: &Studio,
        to: &Studio,
        from_extras_map: &BTreeMap<ItemId, MoveItemExtras>,
        to_extras_map: &BTreeMap<ItemId, MoveItemExtras>,
        config: MoveTransitionConfig,
    ) -> Self {
        let mut to_used = alloc::vec![false; to.items.len()];
        let mut from_used = alloc::vec![false; from.items.len()];
        let mut tracks = Vec::new();

        // Pass 1: match by SourceId (when enabled).
        if matches!(
            config.match_strategy,
            MatchStrategy::SourceIdFirstThenName | MatchStrategy::SourceIdOnly
        ) {
            for (fi, f_item) in from.items.iter().enumerate() {
                if let Some(ti) = to
                    .items
                    .iter()
                    .enumerate()
                    .position(|(idx, t_item)| !to_used[idx] && t_item.source_id == f_item.source_id)
                {
                    from_used[fi] = true;
                    to_used[ti] = true;
                    let t_item = &to.items[ti];
                    tracks.push(MoveTrack {
                        kind: MoveItemKind::Matched,
                        from_item: Some(f_item.clone()),
                        to_item: Some(t_item.clone()),
                        from_extras: from_extras_map.get(&f_item.id).cloned().unwrap_or_default(),
                        to_extras: to_extras_map.get(&t_item.id).cloned().unwrap_or_default(),
                    });
                }
            }
        }

        // Pass 2: match remaining items by name (when enabled).
        if matches!(
            config.match_strategy,
            MatchStrategy::SourceIdFirstThenName | MatchStrategy::NameOnly
        ) {
            for (fi, f_item) in from.items.iter().enumerate() {
                if from_used[fi] {
                    continue;
                }
                if let Some(ti) = to
                    .items
                    .iter()
                    .enumerate()
                    .position(|(idx, t_item)| !to_used[idx] && t_item.name == f_item.name)
                {
                    from_used[fi] = true;
                    to_used[ti] = true;
                    let t_item = &to.items[ti];
                    tracks.push(MoveTrack {
                        kind: MoveItemKind::Matched,
                        from_item: Some(f_item.clone()),
                        to_item: Some(t_item.clone()),
                        from_extras: from_extras_map.get(&f_item.id).cloned().unwrap_or_default(),
                        to_extras: to_extras_map.get(&t_item.id).cloned().unwrap_or_default(),
                    });
                }
            }
        }

        // Unmatched outgoing items.
        for (fi, f_item) in from.items.iter().enumerate() {
            if !from_used[fi] {
                tracks.push(MoveTrack {
                    kind: MoveItemKind::Exiting,
                    from_item: Some(f_item.clone()),
                    to_item: None,
                    from_extras: from_extras_map.get(&f_item.id).cloned().unwrap_or_default(),
                    to_extras: MoveItemExtras::default(),
                });
            }
        }

        // Unmatched incoming items.
        for (ti, t_item) in to.items.iter().enumerate() {
            if !to_used[ti] {
                tracks.push(MoveTrack {
                    kind: MoveItemKind::Entering,
                    from_item: None,
                    to_item: Some(t_item.clone()),
                    from_extras: MoveItemExtras::default(),
                    to_extras: to_extras_map.get(&t_item.id).cloned().unwrap_or_default(),
                });
            }
        }

        Self {
            from_studio: from.id,
            to_studio: to.id,
            config,
            tracks,
        }
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// Evaluates all tracked items at normalized linear progress `t in [0.0, 1.0]`.
    pub fn evaluate(&self, t: f32) -> Vec<MoveItemState> {
        let t_clamped = clamp01(t);
        let e = self.config.easing.eval(t_clamped);
        let e_alpha = t_clamped; // Keep opacity clamped monotonically in [0, 1] even under spring/back overshoot.

        self.tracks
            .iter()
            .filter_map(|track| match track.kind {
                MoveItemKind::Matched => {
                    let f = track.from_item.as_ref()?;
                    let to = track.to_item.as_ref()?;
                    let mut out = if t_clamped < 0.5 { f.clone() } else { to.clone() };
                    out.transform = interpolate_transform(
                        &f.transform,
                        &to.transform,
                        e,
                        t_clamped,
                        self.config.shortest_rotation_path,
                    );
                    let f_op = if f.visible { f.opacity } else { 0.0 };
                    let t_op = if to.visible { to.opacity } else { 0.0 };
                    out.opacity = clamp01(lerp(f_op, t_op, e_alpha));
                    out.visible = out.opacity > 1e-4;

                    let corner_pin = interpolate_corner_pin(
                        track.from_extras.corner_pin,
                        track.to_extras.corner_pin,
                        e,
                    );
                    let filter_params = interpolate_filter_params(
                        &track.from_extras.filter_params,
                        &track.to_extras.filter_params,
                        e,
                    );

                    Some(MoveItemState {
                        source_id: out.source_id,
                        name: out.name.clone(),
                        from_item_id: Some(f.id),
                        to_item_id: Some(to.id),
                        kind: MoveItemKind::Matched,
                        item: out,
                        corner_pin,
                        filter_params,
                    })
                }
                MoveItemKind::Exiting => {
                    let f = track.from_item.as_ref()?;
                    let mut out = f.clone();
                    out.transform.pos = [
                        f.transform.pos[0] + self.config.exit_slide[0] * e,
                        f.transform.pos[1] + self.config.exit_slide[1] * e,
                    ];
                    if self.config.fade_unmatched {
                        out.opacity = clamp01(f.opacity * (1.0 - e_alpha));
                    }
                    if t_clamped >= 1.0 || !f.visible {
                        out.visible = false;
                        out.opacity = 0.0;
                    }
                    Some(MoveItemState {
                        source_id: out.source_id,
                        name: out.name.clone(),
                        from_item_id: Some(f.id),
                        to_item_id: None,
                        kind: MoveItemKind::Exiting,
                        item: out,
                        corner_pin: track.from_extras.corner_pin,
                        filter_params: track.from_extras.filter_params.clone(),
                    })
                }
                MoveItemKind::Entering => {
                    let to = track.to_item.as_ref()?;
                    let mut out = to.clone();
                    let inv_e = 1.0 - e;
                    out.transform.pos = [
                        to.transform.pos[0] + self.config.enter_slide[0] * inv_e,
                        to.transform.pos[1] + self.config.enter_slide[1] * inv_e,
                    ];
                    if self.config.fade_unmatched {
                        out.opacity = clamp01(to.opacity * e_alpha);
                    }
                    if t_clamped <= 0.0 || !to.visible {
                        out.visible = false;
                        out.opacity = 0.0;
                    }
                    Some(MoveItemState {
                        source_id: out.source_id,
                        name: out.name.clone(),
                        from_item_id: None,
                        to_item_id: Some(to.id),
                        kind: MoveItemKind::Entering,
                        item: out,
                        corner_pin: track.to_extras.corner_pin,
                        filter_params: track.to_extras.filter_params.clone(),
                    })
                }
            })
            .collect()
    }

    /// Materializes an interpolated [`Studio`] snapshot at progress `t`.
    pub fn evaluate_into_studio(
        &self,
        t: f32,
        studio_id: StudioId,
        name: impl Into<String>,
    ) -> Studio {
        let mut st = Studio::new(studio_id, name);
        st.items = self.evaluate(t).into_iter().map(|s| s.item).collect();
        st
    }

    /// Renders an interpolated transition frame at progress `t`, including Corner-Pin warps
    /// and interpolated numeric filter parameters when present.
    pub fn render(
        &self,
        t: f32,
        width: u32,
        height: u32,
        collection: &SceneCollection,
        raw_sources: &BTreeMap<SourceId, FrameBuffer>,
        filter_contexts: &mut BTreeMap<SourceId, FilterContext>,
    ) -> Result<FrameBuffer, SceneError> {
        let mut canvas = FrameBuffer::new(width, height).map_err(|_| SceneError::InvalidDimensions {
            width,
            height,
        })?;
        let compositor = Compositor::new([0.0, 0.0, 0.0, 0.0]);
        let states = self.evaluate(t);

        for st in states {
            if !st.item.visible || st.item.opacity <= 0.0 {
                continue;
            }
            let Some(src_def) = collection.sources.get(&st.source_id) else {
                continue;
            };
            let base = match &src_def.kind {
                SourceKind::ColorSolid { rgba } => FrameBuffer::filled(width, height, *rgba)
                    .map_err(|_| SceneError::InvalidDimensions { width, height })?,
                _ => match raw_sources.get(&src_def.id) {
                    Some(fb) => fb.clone(),
                    None => continue,
                },
            };

            let mut chain = src_def.filters.clone();
            if !st.filter_params.is_empty() {
                apply_numeric_filter_params(&mut chain, &st.filter_params);
            }
            let filtered = if chain.is_empty() {
                base
            } else {
                let ctx = filter_contexts
                    .entry(src_def.id)
                    .or_insert_with(FilterContext::new);
                process_filter_chain(&chain, &base, ctx, AiMatteInput::Unavailable, Some(&canvas))
                    .unwrap_or(base)
            };

            if let Some(quad) = st.corner_pin {
                if let Ok(warped) = warp_corner_pin_normalized(&filtered, &quad, width, height) {
                    compositor.draw_layer(
                        &mut canvas,
                        &CompositorLayer {
                            frame: &warped,
                            transform: Transform2D::default(),
                            blend_mode: st.item.blend_mode,
                            opacity: st.item.opacity,
                            mix_space: st.item.opacity_space,
                            scaler: st.item.scaler,
                            visible: true,
                        },
                    );
                    continue;
                }
            }

            compositor.draw_layer(
                &mut canvas,
                &CompositorLayer {
                    frame: &filtered,
                    transform: st.item.transform,
                    blend_mode: st.item.blend_mode,
                    opacity: st.item.opacity,
                    mix_space: st.item.opacity_space,
                    scaler: st.item.scaler,
                    visible: true,
                },
            );
        }

        Ok(canvas)
    }
}

/// Smoothly interpolates two [`Transform2D`] structs at eased progress `e` (and raw `t_clamped`).
pub fn interpolate_transform(
    from: &Transform2D,
    to: &Transform2D,
    e: f32,
    t_clamped: f32,
    shortest_rot: bool,
) -> Transform2D {
    let rot_deg = if shortest_rot {
        let mut delta = (to.rot_deg - from.rot_deg) % 360.0;
        if delta > 180.0 {
            delta -= 360.0;
        } else if delta < -180.0 {
            delta += 360.0;
        }
        from.rot_deg + delta * e
    } else {
        lerp(from.rot_deg, to.rot_deg, e)
    };

    let anchor = match (from.anchor, to.anchor) {
        (Anchor::Custom { x: x0, y: y0 }, Anchor::Custom { x: x1, y: y1 }) => Anchor::Custom {
            x: lerp(x0, x1, e),
            y: lerp(y0, y1, e),
        },
        _ => {
            if t_clamped < 0.5 {
                from.anchor
            } else {
                to.anchor
            }
        }
    };

    Transform2D {
        pos: [
            lerp(from.pos[0], to.pos[0], e),
            lerp(from.pos[1], to.pos[1], e),
        ],
        scale: [
            lerp(from.scale[0], to.scale[0], e),
            lerp(from.scale[1], to.scale[1], e),
        ],
        rot_deg,
        anchor,
        crop: CropRect {
            top: lerp(from.crop.top, to.crop.top, e).max(0.0),
            right: lerp(from.crop.right, to.crop.right, e).max(0.0),
            bottom: lerp(from.crop.bottom, to.crop.bottom, e).max(0.0),
            left: lerp(from.crop.left, to.crop.left, e).max(0.0),
        },
        flip_h: if t_clamped < 0.5 {
            from.flip_h
        } else {
            to.flip_h
        },
        flip_v: if t_clamped < 0.5 {
            from.flip_v
        } else {
            to.flip_v
        },
        bounds_mode: if t_clamped < 0.5 {
            from.bounds_mode
        } else {
            to.bounds_mode
        },
        bounds: [
            lerp(from.bounds[0], to.bounds[0], e).max(0.0),
            lerp(from.bounds[1], to.bounds[1], e).max(0.0),
        ],
        bounds_alignment: if t_clamped < 0.5 {
            from.bounds_alignment
        } else {
            to.bounds_alignment
        },
    }
}

fn interpolate_corner_pin(
    from: Option<CornerPinQuad>,
    to: Option<CornerPinQuad>,
    e: f32,
) -> Option<CornerPinQuad> {
    match (from, to) {
        (Some(a), Some(b)) => Some(a.lerp(b, e)),
        (Some(a), None) => Some(a.lerp(CornerPinQuad::unit(), e)),
        (None, Some(b)) => Some(CornerPinQuad::unit().lerp(b, e)),
        (None, None) => None,
    }
}

fn interpolate_filter_params(
    from: &BTreeMap<String, f32>,
    to: &BTreeMap<String, f32>,
    e: f32,
) -> BTreeMap<String, f32> {
    let mut out = BTreeMap::new();
    for (k, &v0) in from {
        let v1 = to.get(k).copied().unwrap_or(v0);
        out.insert(k.clone(), lerp(v0, v1, e));
    }
    for (k, &v1) in to {
        if !out.contains_key(k) {
            out.insert(k.clone(), v1);
        }
    }
    out
}

/// Extracts numeric filter parameters from a `cabaret_core::filters::FilterChain`
/// keyed deterministically by `"<idx>.<param>"` for smooth Move transition interpolation.
pub fn extract_numeric_filter_params(chain: &FilterChain) -> BTreeMap<String, f32> {
    let mut map = BTreeMap::new();
    for (idx, entry) in chain.entries.iter().enumerate() {
        match &entry.filter {
            VideoFilter::Blur { radius, .. } => {
                map.insert(format!("{idx}.blur.radius"), *radius);
            }
            VideoFilter::Sharpen { amount } => {
                map.insert(format!("{idx}.sharpen.amount"), *amount);
            }
            VideoFilter::LegacyColor {
                brightness,
                contrast,
                gamma,
                saturation,
                hue_shift_deg,
                opacity,
            } => {
                map.insert(format!("{idx}.color.brightness"), *brightness);
                map.insert(format!("{idx}.color.contrast"), *contrast);
                map.insert(format!("{idx}.color.gamma"), *gamma);
                map.insert(format!("{idx}.color.saturation"), *saturation);
                map.insert(format!("{idx}.color.hue_shift_deg"), *hue_shift_deg);
                map.insert(format!("{idx}.color.opacity"), *opacity);
            }
            VideoFilter::ColorWheels4Way { saturation, .. } => {
                map.insert(format!("{idx}.wheels.saturation"), *saturation);
            }
            VideoFilter::ChromaKey {
                similarity,
                smoothness,
                spill_suppression,
                ..
            } => {
                map.insert(format!("{idx}.chroma.similarity"), *similarity);
                map.insert(format!("{idx}.chroma.smoothness"), *smoothness);
                map.insert(format!("{idx}.chroma.spill"), *spill_suppression);
            }
            VideoFilter::DropShadow {
                offset_x,
                offset_y,
                blur_radius,
                spread,
                ..
            } => {
                map.insert(format!("{idx}.shadow.offset_x"), *offset_x);
                map.insert(format!("{idx}.shadow.offset_y"), *offset_y);
                map.insert(format!("{idx}.shadow.blur_radius"), *blur_radius);
                map.insert(format!("{idx}.shadow.spread"), *spread);
            }
            VideoFilter::ApplyLut { intensity, .. } => {
                map.insert(format!("{idx}.lut.intensity"), *intensity);
            }
            _ => {}
        }
    }
    map
}

/// Applies interpolated numeric filter parameters back onto a [`FilterChain`].
pub fn apply_numeric_filter_params(chain: &mut FilterChain, params: &BTreeMap<String, f32>) {
    for (idx, entry) in chain.entries.iter_mut().enumerate() {
        match &mut entry.filter {
            VideoFilter::Blur { radius, .. } => {
                if let Some(&v) = params.get(&format!("{idx}.blur.radius")) {
                    *radius = v.max(0.0);
                }
            }
            VideoFilter::Sharpen { amount } => {
                if let Some(&v) = params.get(&format!("{idx}.sharpen.amount")) {
                    *amount = v.max(0.0);
                }
            }
            VideoFilter::LegacyColor {
                brightness,
                contrast,
                gamma,
                saturation,
                hue_shift_deg,
                opacity,
            } => {
                if let Some(&v) = params.get(&format!("{idx}.color.brightness")) {
                    *brightness = v;
                }
                if let Some(&v) = params.get(&format!("{idx}.color.contrast")) {
                    *contrast = v;
                }
                if let Some(&v) = params.get(&format!("{idx}.color.gamma")) {
                    *gamma = v.max(0.05);
                }
                if let Some(&v) = params.get(&format!("{idx}.color.saturation")) {
                    *saturation = v.max(0.0);
                }
                if let Some(&v) = params.get(&format!("{idx}.color.hue_shift_deg")) {
                    *hue_shift_deg = v;
                }
                if let Some(&v) = params.get(&format!("{idx}.color.opacity")) {
                    *opacity = clamp01(v);
                }
            }
            VideoFilter::ColorWheels4Way { saturation, .. } => {
                if let Some(&v) = params.get(&format!("{idx}.wheels.saturation")) {
                    *saturation = v.max(0.0);
                }
            }
            VideoFilter::ChromaKey {
                similarity,
                smoothness,
                spill_suppression,
                ..
            } => {
                if let Some(&v) = params.get(&format!("{idx}.chroma.similarity")) {
                    *similarity = v.max(1e-4);
                }
                if let Some(&v) = params.get(&format!("{idx}.chroma.smoothness")) {
                    *smoothness = v.max(1e-4);
                }
                if let Some(&v) = params.get(&format!("{idx}.chroma.spill")) {
                    *spill_suppression = clamp01(v);
                }
            }
            VideoFilter::DropShadow {
                offset_x,
                offset_y,
                blur_radius,
                spread,
                ..
            } => {
                if let Some(&v) = params.get(&format!("{idx}.shadow.offset_x")) {
                    *offset_x = v;
                }
                if let Some(&v) = params.get(&format!("{idx}.shadow.offset_y")) {
                    *offset_y = v;
                }
                if let Some(&v) = params.get(&format!("{idx}.shadow.blur_radius")) {
                    *blur_radius = v.max(0.0);
                }
                if let Some(&v) = params.get(&format!("{idx}.shadow.spread")) {
                    *spread = v.max(0.0);
                }
            }
            VideoFilter::ApplyLut { intensity, .. } => {
                if let Some(&v) = params.get(&format!("{idx}.lut.intensity")) {
                    *intensity = clamp01(v);
                }
            }
            _ => {}
        }
    }
}
