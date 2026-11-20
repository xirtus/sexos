// SPDX-License-Identifier: GPL-3.0-only
//! Object Morph Transition Engine (Keynote Magic Move / OmaShow Morph).
//!
//! Matches slide elements between Slide A and Slide B by:
//! 1. Explicit `morph_key` (`SlideElement::magic_key` or `ElementMeta::morph_key`)
//! 2. Explicit `element_id` (`ElementMeta::element_id`)
//! 3. Automatic text/label affinity (exact text, slide title pairing, or token similarity)
//! 4. Automatic shape affinity (same shape kind or compatible rect/rounded-rect)
//! 5. Remaining elements of the same kind in spatial/ordinal order
//!
//! Interpolates position, size, rotation, fill/stroke colors, corner radius,
//! font size, and opacity (fade-in / fade-out for unmatched objects) at any
//! normalized progress `t` in `[0.0, 1.0]`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_canvas::tiny_skia::{PathBuilder, Transform};
use sex_canvas::{round_rect_path, Canvas, Color, Rect};

use crate::deck::{AspectRatio, Deck, Slide};
use crate::elements::{Background, ShapeKind, SlideElement, StrokeStyle};
use crate::render::{
    blend_buffer_over_alpha, lerp, lerp_color, lerp_color_opt, lerp_stroke_opt, render_master_chrome,
    render_single_element, render_text_box, scale_rect,
};
use crate::theme::Theme;

/// Optional per-element morph/transform metadata attached to a slide element.
#[derive(Clone, Debug, PartialEq)]
pub struct ElementMeta {
    pub element_id: Option<String>,
    pub morph_key: Option<String>,
    pub rotation_deg: f32,
    pub opacity: f32,
}

impl Default for ElementMeta {
    fn default() -> Self {
        Self {
            element_id: None,
            morph_key: None,
            rotation_deg: 0.0,
            opacity: 1.0,
        }
    }
}

impl ElementMeta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.element_id = Some(id.into());
        self
    }

    pub fn with_morph_key(mut self, key: impl Into<String>) -> Self {
        self.morph_key = Some(key.into());
        self
    }

    pub fn with_rotation(mut self, deg: f32) -> Self {
        self.rotation_deg = if deg.is_finite() { deg } else { 0.0 };
        self
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self
    }
}

/// Easing curve applied to normalized progress `t in [0.0, 1.0]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphEasing {
    Linear,
    EaseInOutCubic,
}

impl Default for MorphEasing {
    fn default() -> Self {
        Self::EaseInOutCubic
    }
}

impl MorphEasing {
    #[inline]
    pub fn apply(self, t: f32) -> f32 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        match self {
            Self::Linear => t,
            Self::EaseInOutCubic => 3.0 * t * t - 2.0 * t * t * t,
        }
    }
}

/// Reason two elements across Slide A and Slide B were paired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphMatchReason {
    MorphKey,
    ElementId,
    TextAffinity,
    ShapeAffinity,
    KindOrdinal,
}

/// A matched element pair between Slide A (`from_index`) and Slide B (`to_index`).
#[derive(Clone, Debug, PartialEq)]
pub struct MorphPair {
    pub from_index: usize,
    pub to_index: usize,
    pub reason: MorphMatchReason,
    pub affinity: f32,
}

/// Complete element correspondence plan between two slides.
#[derive(Clone, Debug, PartialEq)]
pub struct MorphPlan {
    pub pairs: Vec<MorphPair>,
    pub unmatched_from: Vec<usize>,
    pub unmatched_to: Vec<usize>,
}

/// Snapshot of interpolated visual properties for an element at progress `t`.
#[derive(Clone, Debug, PartialEq)]
pub struct InterpolatedElementState {
    pub bounds: Rect,
    pub rotation_deg: f32,
    pub opacity: f32,
    pub fill: Option<Color>,
    pub stroke: Option<StrokeStyle>,
    pub corner_radius: f32,
    pub font_size: f32,
    pub text_color: Option<Color>,
}

/// Single evaluated object participating in a morph frame.
#[derive(Clone, Debug, PartialEq)]
pub enum MorphFrameItem {
    Matched {
        from_index: usize,
        to_index: usize,
        reason: MorphMatchReason,
        state: InterpolatedElementState,
    },
    FadeOut {
        from_index: usize,
        state: InterpolatedElementState,
    },
    FadeIn {
        to_index: usize,
        state: InterpolatedElementState,
    },
}

fn effective_morph_key<'a>(elem: &'a SlideElement, meta: &'a ElementMeta) -> Option<&'a str> {
    meta.morph_key
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| elem.magic_key().filter(|s| !s.trim().is_empty()))
}

fn effective_element_id<'a>(meta: &'a ElementMeta) -> Option<&'a str> {
    meta.element_id.as_deref().filter(|s| !s.trim().is_empty())
}

fn shape_corner_radius(kind: ShapeKind) -> f32 {
    match kind {
        ShapeKind::RoundedRect { radius } => {
            if radius.is_finite() {
                radius.max(0.0)
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

fn shapes_compatible(a: ShapeKind, b: ShapeKind) -> Option<f32> {
    match (a, b) {
        (ShapeKind::Rect, ShapeKind::Rect) => Some(1.0),
        (ShapeKind::RoundedRect { .. }, ShapeKind::RoundedRect { .. }) => Some(1.0),
        (ShapeKind::Rect, ShapeKind::RoundedRect { .. })
        | (ShapeKind::RoundedRect { .. }, ShapeKind::Rect) => Some(0.85),
        (ShapeKind::Ellipse, ShapeKind::Ellipse) => Some(1.0),
        (ShapeKind::Line, ShapeKind::Line) => Some(1.0),
        (ShapeKind::Arrow, ShapeKind::Arrow) => Some(1.0),
        _ => None,
    }
}

fn normalize_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                cur.push(lc);
            }
        } else if !cur.is_empty() {
            out.push(core::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn text_affinity(a: &str, b: &str, title_a: &str, title_b: &str) -> f32 {
    let ta = a.trim();
    let tb = b.trim();
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    if ta == tb {
        return 1.0;
    }
    if !title_a.trim().is_empty() && ta == title_a.trim() && tb == title_b.trim() {
        return 0.92;
    }
    if ta.eq_ignore_ascii_case(tb) {
        return 0.95;
    }
    let toks_a = normalize_tokens(ta);
    let toks_b = normalize_tokens(tb);
    if toks_a.is_empty() || toks_b.is_empty() {
        return 0.0;
    }
    let mut overlap = 0usize;
    for tok in &toks_a {
        if toks_b.iter().any(|other| other == tok) {
            overlap += 1;
        }
    }
    if overlap == 0 {
        return 0.0;
    }
    let denom = toks_a.len().max(toks_b.len()) as f32;
    (overlap as f32 / denom) * 0.85
}

fn spatial_affinity(a: Rect, b: Rect) -> f32 {
    let cax = a.x + a.w * 0.5;
    let cay = a.y + a.h * 0.5;
    let cbx = b.x + b.w * 0.5;
    let cby = b.y + b.h * 0.5;
    let dx = cax - cbx;
    let dy = cay - cby;
    let dist = libm::sqrtf(dx * dx + dy * dy);
    1.0 / (1.0 + dist / 400.0)
}

/// Computes deterministic object matching between `from_slide` and `to_slide`.
pub fn compute_morph_plan(from_slide: &Slide, to_slide: &Slide) -> MorphPlan {
    let from_elems = &from_slide.elements;
    let to_elems = &to_slide.elements;
    let n_from = from_elems.len();
    let n_to = to_elems.len();

    let mut matched_from = vec![false; n_from];
    let mut matched_to = vec![false; n_to];
    let mut pairs = Vec::new();

    // Pass 1: Explicit morph_key / magic_key
    for (i, ef) in from_elems.iter().enumerate() {
        let mf = from_slide.element_meta(i);
        let Some(kf) = effective_morph_key(ef, &mf) else {
            continue;
        };
        for (j, et) in to_elems.iter().enumerate() {
            if matched_to[j] {
                continue;
            }
            let mt = to_slide.element_meta(j);
            if let Some(kt) = effective_morph_key(et, &mt) {
                if kf == kt {
                    matched_from[i] = true;
                    matched_to[j] = true;
                    pairs.push(MorphPair {
                        from_index: i,
                        to_index: j,
                        reason: MorphMatchReason::MorphKey,
                        affinity: 1.0,
                    });
                    break;
                }
            }
        }
    }

    // Pass 2: Explicit element_id
    for i in 0..n_from {
        if matched_from[i] {
            continue;
        }
        let mf = from_slide.element_meta(i);
        let Some(id_f) = effective_element_id(&mf) else {
            continue;
        };
        for j in 0..n_to {
            if matched_to[j] {
                continue;
            }
            let mt = to_slide.element_meta(j);
            if let Some(id_t) = effective_element_id(&mt) {
                if id_f == id_t {
                    matched_from[i] = true;
                    matched_to[j] = true;
                    pairs.push(MorphPair {
                        from_index: i,
                        to_index: j,
                        reason: MorphMatchReason::ElementId,
                        affinity: 1.0,
                    });
                    break;
                }
            }
        }
    }

    // Pass 3: Automatic TextBox / label affinity (best-scoring candidate >= 0.40)
    for (i, ef) in from_elems.iter().enumerate() {
        if matched_from[i] {
            continue;
        }
        let SlideElement::TextBox(tbf) = ef else {
            continue;
        };
        let tf = tbf.plain_text();
        let mut best: Option<(usize, f32)> = None;
        for (j, et) in to_elems.iter().enumerate() {
            if matched_to[j] {
                continue;
            }
            let SlideElement::TextBox(tbt) = et else {
                continue;
            };
            let score = text_affinity(&tf, &tbt.plain_text(), &from_slide.title, &to_slide.title);
            if score >= 0.40 && best.map_or(true, |(_, bs)| score > bs) {
                best = Some((j, score));
            }
        }
        if let Some((j, score)) = best {
            matched_from[i] = true;
            matched_to[j] = true;
            pairs.push(MorphPair {
                from_index: i,
                to_index: j,
                reason: MorphMatchReason::TextAffinity,
                affinity: score,
            });
        }
    }

    // Pass 4: Automatic Shape affinity (kind compatibility + spatial/color proximity)
    for (i, ef) in from_elems.iter().enumerate() {
        if matched_from[i] {
            continue;
        }
        let SlideElement::Shape(sf) = ef else {
            continue;
        };
        let mut best: Option<(usize, f32)> = None;
        for (j, et) in to_elems.iter().enumerate() {
            if matched_to[j] {
                continue;
            }
            let SlideElement::Shape(st) = et else {
                continue;
            };
            let Some(kind_score) = shapes_compatible(sf.kind, st.kind) else {
                continue;
            };
            let dist_score = spatial_affinity(sf.bounds, st.bounds);
            let color_bonus = if sf.fill == st.fill && sf.fill.is_some() {
                0.15
            } else {
                0.0
            };
            let score = kind_score * 0.7 + dist_score * 0.25 + color_bonus;
            if best.map_or(true, |(_, bs)| score > bs) {
                best = Some((j, score));
            }
        }
        if let Some((j, score)) = best {
            matched_from[i] = true;
            matched_to[j] = true;
            pairs.push(MorphPair {
                from_index: i,
                to_index: j,
                reason: MorphMatchReason::ShapeAffinity,
                affinity: score,
            });
        }
    }

    // Pass 5: Remaining non-text/non-shape elements (Image, Code, Table) of same variant in ordinal order.
    // TextBoxes and Shapes that failed affinity matching intentionally remain unmatched so they fade out/in.
    for (i, ef) in from_elems.iter().enumerate() {
        if matched_from[i] || matches!(ef, SlideElement::TextBox(_) | SlideElement::Shape(_)) {
            continue;
        }
        for (j, et) in to_elems.iter().enumerate() {
            if matched_to[j] {
                continue;
            }
            if core::mem::discriminant(ef) == core::mem::discriminant(et) {
                matched_from[i] = true;
                matched_to[j] = true;
                pairs.push(MorphPair {
                    from_index: i,
                    to_index: j,
                    reason: MorphMatchReason::KindOrdinal,
                    affinity: spatial_affinity(ef.bounds(), et.bounds()),
                });
                break;
            }
        }
    }

    let unmatched_from = (0..n_from).filter(|i| !matched_from[*i]).collect();
    let unmatched_to = (0..n_to).filter(|j| !matched_to[*j]).collect();

    MorphPlan {
        pairs,
        unmatched_from,
        unmatched_to,
    }
}

fn element_font_and_color(elem: &SlideElement, theme: &Theme) -> (f32, Option<Color>) {
    match elem {
        SlideElement::TextBox(tb) => {
            let run = tb.paragraphs.first().and_then(|p| p.runs.first());
            let fs = run.map(|r| r.font_size).unwrap_or(theme.typography.body_size);
            let col = run.and_then(|r| r.color).unwrap_or(theme.palette.foreground);
            (fs, Some(col))
        }
        SlideElement::Code(cb) => (cb.font_size, Some(cb.text_color)),
        _ => (0.0, None),
    }
}

fn element_fill_and_stroke(elem: &SlideElement) -> (Option<Color>, Option<StrokeStyle>, f32) {
    match elem {
        SlideElement::Shape(s) => (s.fill, s.stroke, shape_corner_radius(s.kind)),
        SlideElement::TextBox(tb) => (tb.background, None, 8.0),
        SlideElement::Code(cb) => (Some(cb.background), None, 0.0),
        SlideElement::Table(t) => (
            t.header_background,
            Some(StrokeStyle {
                color: t.border_color,
                width: t.border_width,
            }),
            0.0,
        ),
        SlideElement::Image(_) => (None, None, 0.0),
    }
}

/// Interpolate properties of a matched pair at raw factor `ease` in `[0.0, 1.0]`.
pub fn interpolate_pair(
    from_elem: &SlideElement,
    from_meta: &ElementMeta,
    to_elem: &SlideElement,
    to_meta: &ElementMeta,
    theme: &Theme,
    ease: f32,
) -> InterpolatedElementState {
    let ease = if ease.is_finite() { ease.clamp(0.0, 1.0) } else { 0.0 };
    let rf = from_elem.bounds();
    let rt = to_elem.bounds();
    let bounds = Rect::new(
        lerp(rf.x, rt.x, ease),
        lerp(rf.y, rt.y, ease),
        lerp(rf.w, rt.w, ease).max(0.0),
        lerp(rf.h, rt.h, ease).max(0.0),
    );
    let rotation_deg = lerp(from_meta.rotation_deg, to_meta.rotation_deg, ease);
    let opacity = lerp(from_meta.opacity, to_meta.opacity, ease).clamp(0.0, 1.0);

    let (fill_f, stroke_f, rad_f) = element_fill_and_stroke(from_elem);
    let (fill_t, stroke_t, rad_t) = element_fill_and_stroke(to_elem);
    let fill = lerp_color_opt(fill_f, fill_t, ease);
    let stroke = lerp_stroke_opt(stroke_f, stroke_t, ease);
    let corner_radius = lerp(rad_f, rad_t, ease).max(0.0);

    let (fs_f, col_f) = element_font_and_color(from_elem, theme);
    let (fs_t, col_t) = element_font_and_color(to_elem, theme);
    let font_size = lerp(fs_f, fs_t, ease);
    let text_color = lerp_color_opt(col_f, col_t, ease);

    InterpolatedElementState {
        bounds,
        rotation_deg,
        opacity,
        fill,
        stroke,
        corner_radius,
        font_size,
        text_color,
    }
}

fn single_state(elem: &SlideElement, meta: &ElementMeta, theme: &Theme, opacity: f32) -> InterpolatedElementState {
    let (fill, stroke, corner_radius) = element_fill_and_stroke(elem);
    let (font_size, text_color) = element_font_and_color(elem, theme);
    InterpolatedElementState {
        bounds: elem.bounds(),
        rotation_deg: meta.rotation_deg,
        opacity: opacity.clamp(0.0, 1.0),
        fill,
        stroke,
        corner_radius,
        font_size,
        text_color,
    }
}

/// Evaluate all matched and unmatched morph states at `t in [0.0, 1.0]` using `EaseInOutCubic`.
pub fn evaluate_morph(
    from_slide: &Slide,
    to_slide: &Slide,
    theme: &Theme,
    t: f32,
) -> Vec<MorphFrameItem> {
    evaluate_morph_with_easing(from_slide, to_slide, theme, t, MorphEasing::EaseInOutCubic)
}

/// Evaluate all matched and unmatched morph states at `t in [0.0, 1.0]` with an explicit easing curve.
pub fn evaluate_morph_with_easing(
    from_slide: &Slide,
    to_slide: &Slide,
    theme: &Theme,
    t: f32,
    easing: MorphEasing,
) -> Vec<MorphFrameItem> {
    let ease = easing.apply(t);
    let plan = compute_morph_plan(from_slide, to_slide);
    let mut items = Vec::with_capacity(plan.pairs.len() + plan.unmatched_from.len() + plan.unmatched_to.len());

    for &i in &plan.unmatched_from {
        let ef = &from_slide.elements[i];
        let mf = from_slide.element_meta(i);
        let op = mf.opacity * (1.0 - ease);
        items.push(MorphFrameItem::FadeOut {
            from_index: i,
            state: single_state(ef, &mf, theme, op),
        });
    }

    for pair in &plan.pairs {
        let ef = &from_slide.elements[pair.from_index];
        let et = &to_slide.elements[pair.to_index];
        let mf = from_slide.element_meta(pair.from_index);
        let mt = to_slide.element_meta(pair.to_index);
        items.push(MorphFrameItem::Matched {
            from_index: pair.from_index,
            to_index: pair.to_index,
            reason: pair.reason,
            state: interpolate_pair(ef, &mf, et, &mt, theme, ease),
        });
    }

    for &j in &plan.unmatched_to {
        let et = &to_slide.elements[j];
        let mt = to_slide.element_meta(j);
        let op = mt.opacity * ease;
        items.push(MorphFrameItem::FadeIn {
            to_index: j,
            state: single_state(et, &mt, theme, op),
        });
    }

    items
}

/// Render a full Object Morph frame between `from_slide` and `to_slide` at `progress in [0.0, 1.0]`.
pub fn render_morph_frame(
    from_slide: &Slide,
    to_slide: &Slide,
    theme: &Theme,
    aspect_ratio: AspectRatio,
    progress: f32,
    buffer: &mut [u32],
    target_w: u32,
    target_h: u32,
) {
    render_morph_frame_with_options(
        from_slide,
        to_slide,
        theme,
        aspect_ratio,
        progress,
        MorphEasing::EaseInOutCubic,
        false,
        buffer,
        target_w,
        target_h,
    );
}

/// Render an Object Morph frame with transparent background support and configurable easing.
#[allow(clippy::too_many_arguments)]
pub fn render_morph_frame_with_options(
    from_slide: &Slide,
    to_slide: &Slide,
    theme: &Theme,
    aspect_ratio: AspectRatio,
    progress: f32,
    easing: MorphEasing,
    transparent_background: bool,
    buffer: &mut [u32],
    target_w: u32,
    target_h: u32,
) {
    let needed = match (target_w as usize).checked_mul(target_h as usize) {
        Some(n) => n,
        None => return,
    };
    if target_w == 0 || target_h == 0 || buffer.len() < needed {
        return;
    }

    let mut canvas = match Canvas::new(buffer, target_w, target_h) {
        Some(c) => c,
        None => return,
    };

    let deck_w = aspect_ratio.width();
    let deck_h = aspect_ratio.height();
    let sx = target_w as f32 / deck_w;
    let sy = target_h as f32 / deck_h;
    let ease = easing.apply(progress);

    // 1. Background
    if transparent_background {
        canvas.clear(Color::TRANSPARENT);
    } else {
        let bg_from = match from_slide.background_override {
            Some(Background::Solid(c)) => c,
            Some(Background::Gradient(c1, _)) => c1,
            None => theme.palette.background,
        };
        let bg_to = match to_slide.background_override {
            Some(Background::Solid(c)) => c,
            Some(Background::Gradient(c1, _)) => c1,
            None => theme.palette.background,
        };
        canvas.clear(lerp_color(bg_from, bg_to, ease));

        if theme.show_master_chrome {
            let active_id = if ease < 0.5 { from_slide.id } else { to_slide.id };
            render_master_chrome(&mut canvas, theme, active_id, target_w, target_h, sx, sy);
        }
    }

    // 2. Plan & evaluate
    let plan = compute_morph_plan(from_slide, to_slide);
    let len = needed;

    // 3. Unmatched from_slide elements (fade out)
    if ease < 1.0 {
        for &i in &plan.unmatched_from {
            let ef = &from_slide.elements[i];
            let mf = from_slide.element_meta(i);
            let op = (mf.opacity * (1.0 - ease)).clamp(0.0, 1.0);
            if op <= 0.001 {
                continue;
            }
            render_element_transformed(
                ef,
                None,
                mf.rotation_deg,
                op,
                theme,
                sx,
                sy,
                canvas.pixels_mut(),
                target_w,
                target_h,
                len,
            );
        }
    }

    // 4. Matched pairs (position, size, rotation, fill/stroke, corner radius, font size, opacity)
    for pair in &plan.pairs {
        let ef = &from_slide.elements[pair.from_index];
        let et = &to_slide.elements[pair.to_index];
        let mf = from_slide.element_meta(pair.from_index);
        let mt = to_slide.element_meta(pair.to_index);

        let st_eval = interpolate_pair(ef, &mf, et, &mt, theme, ease);
        if st_eval.opacity <= 0.001 {
            continue;
        }
        let r_interp = scale_rect(st_eval.bounds, sx, sy);

        match (ef, et) {
            (SlideElement::Shape(sf), SlideElement::Shape(st_shape)) => {
                render_interpolated_shape(
                    sf.kind,
                    st_shape.kind,
                    r_interp,
                    st_eval.corner_radius * sx,
                    st_eval.fill,
                    st_eval.stroke,
                    st_eval.rotation_deg,
                    st_eval.opacity,
                    sx,
                    &mut canvas,
                    target_w,
                    target_h,
                );
            }
            (SlideElement::TextBox(tbf), SlideElement::TextBox(tbt)) => {
                let active_tb = if ease < 0.5 { tbf } else { tbt };
                let mut interp_tb = active_tb.clone();
                interp_tb.padding = lerp(tbf.padding, tbt.padding, ease);
                interp_tb.background = st_eval.fill;
                for p in &mut interp_tb.paragraphs {
                    for r in &mut p.runs {
                        r.font_size = st_eval.font_size.max(4.0);
                        r.color = st_eval.text_color;
                    }
                }
                let synth = SlideElement::TextBox(interp_tb);
                render_element_transformed(
                    &synth,
                    Some(r_interp),
                    st_eval.rotation_deg,
                    st_eval.opacity,
                    theme,
                    sx,
                    sy,
                    canvas.pixels_mut(),
                    target_w,
                    target_h,
                    len,
                );
            }
            (SlideElement::Image(img_f), SlideElement::Image(img_t)) => {
                let active_img = if ease < 0.5 { img_f } else { img_t };
                let synth = SlideElement::Image(active_img.clone());
                render_element_transformed(
                    &synth,
                    Some(r_interp),
                    st_eval.rotation_deg,
                    st_eval.opacity,
                    theme,
                    sx,
                    sy,
                    canvas.pixels_mut(),
                    target_w,
                    target_h,
                    len,
                );
            }
            (SlideElement::Code(cf), SlideElement::Code(ct)) => {
                let active_c = if ease < 0.5 { cf } else { ct };
                let mut c_clone = active_c.clone();
                c_clone.font_size = st_eval.font_size.max(4.0);
                if let Some(bg) = st_eval.fill {
                    c_clone.background = bg;
                }
                if let Some(tc) = st_eval.text_color {
                    c_clone.text_color = tc;
                }
                let synth = SlideElement::Code(c_clone);
                render_element_transformed(
                    &synth,
                    Some(r_interp),
                    st_eval.rotation_deg,
                    st_eval.opacity,
                    theme,
                    sx,
                    sy,
                    canvas.pixels_mut(),
                    target_w,
                    target_h,
                    len,
                );
            }
            _ => {
                render_element_transformed(
                    ef,
                    Some(r_interp),
                    st_eval.rotation_deg,
                    st_eval.opacity * (1.0 - ease),
                    theme,
                    sx,
                    sy,
                    canvas.pixels_mut(),
                    target_w,
                    target_h,
                    len,
                );
                render_element_transformed(
                    et,
                    Some(r_interp),
                    st_eval.rotation_deg,
                    st_eval.opacity * ease,
                    theme,
                    sx,
                    sy,
                    canvas.pixels_mut(),
                    target_w,
                    target_h,
                    len,
                );
            }
        }
    }

    // 5. Unmatched to_slide elements (fade in)
    if ease > 0.0 {
        for &j in &plan.unmatched_to {
            let et = &to_slide.elements[j];
            let mt = to_slide.element_meta(j);
            let op = (mt.opacity * ease).clamp(0.0, 1.0);
            if op <= 0.001 {
                continue;
            }
            render_element_transformed(
                et,
                None,
                mt.rotation_deg,
                op,
                theme,
                sx,
                sy,
                canvas.pixels_mut(),
                target_w,
                target_h,
                len,
            );
        }
    }
}

fn scale_color_alpha(c: Color, opacity: f32) -> Color {
    let op = opacity.clamp(0.0, 1.0);
    let new_a = ((c.a() as f32) * op + 0.5).clamp(0.0, 255.0) as u8;
    c.with_alpha(new_a)
}

fn rotate_path(path: sex_canvas::Path, needs_rotate: bool, xform: Transform) -> sex_canvas::Path {
    if needs_rotate {
        path.clone().transform(xform).unwrap_or(path)
    } else {
        path
    }
}

#[allow(clippy::too_many_arguments)]
fn render_interpolated_shape(
    kind_from: ShapeKind,
    kind_to: ShapeKind,
    rect: Rect,
    corner_radius_px: f32,
    fill: Option<Color>,
    stroke: Option<StrokeStyle>,
    rotation_deg: f32,
    opacity: f32,
    sx: f32,
    canvas: &mut Canvas<'_>,
    _w: u32,
    _h: u32,
) {
    let fill = fill.map(|c| scale_color_alpha(c, opacity));
    let stroke = stroke.map(|s| StrokeStyle {
        color: scale_color_alpha(s.color, opacity),
        width: s.width,
    });

    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    let needs_rotate = libm::fabsf(rotation_deg) > 0.01;
    let xform = if needs_rotate {
        Transform::from_rotate_at(rotation_deg, cx, cy)
    } else {
        Transform::identity()
    };

    match (kind_from, kind_to) {
        (ShapeKind::Ellipse, ShapeKind::Ellipse) => {
            if !needs_rotate {
                let r = (rect.w.min(rect.h) / 2.0).max(1.0);
                if let Some(c) = fill {
                    canvas.fill_circle(cx, cy, r, c);
                }
                if let Some(st) = stroke {
                    canvas.stroke_circle(cx, cy, r, st.width * sx, st.color);
                }
            } else {
                let rx = (rect.w / 2.0).max(1.0);
                let ry = (rect.h / 2.0).max(1.0);
                if let Some(oval) = tiny_skia_ellipse_path(cx, cy, rx, ry) {
                    let path = rotate_path(oval, true, xform);
                    if let Some(c) = fill {
                        canvas.fill_path(&path, c);
                    }
                    if let Some(st) = stroke {
                        canvas.stroke_path(&path, st.width * sx, st.color);
                    }
                }
            }
        }
        (ShapeKind::Line, ShapeKind::Line) => {
            if let Some(st) = stroke {
                let mut pb = PathBuilder::new();
                pb.move_to(rect.x, rect.y);
                pb.line_to(rect.right(), rect.bottom());
                if let Some(p) = pb.finish() {
                    let path = rotate_path(p, needs_rotate, xform);
                    canvas.stroke_path(&path, st.width * sx, st.color);
                }
            }
        }
        (ShapeKind::Arrow, ShapeKind::Arrow) => {
            if let Some(st) = stroke {
                let mut pb = PathBuilder::new();
                let mid_y = rect.y + rect.h * 0.5;
                pb.move_to(rect.x, mid_y);
                pb.line_to(rect.right(), mid_y);
                let head_len = (rect.h * 0.4).max(6.0);
                pb.move_to(rect.right(), mid_y);
                pb.line_to(rect.right() - head_len, mid_y - head_len);
                pb.move_to(rect.right(), mid_y);
                pb.line_to(rect.right() - head_len, mid_y + head_len);
                if let Some(p) = pb.finish() {
                    let path = rotate_path(p, needs_rotate, xform);
                    canvas.stroke_path(&path, st.width * sx, st.color);
                }
            }
        }
        _ => {
            if let Some(base_path) = round_rect_path(rect, corner_radius_px) {
                let path = rotate_path(base_path, needs_rotate, xform);
                if let Some(c) = fill {
                    canvas.fill_path(&path, c);
                }
                if let Some(st) = stroke {
                    canvas.stroke_path(&path, st.width * sx, st.color);
                }
            }
        }
    }
}

fn tiny_skia_ellipse_path(cx: f32, cy: f32, rx: f32, ry: f32) -> Option<sex_canvas::Path> {
    const K: f32 = 0.552_284_8;
    let kx = rx * K;
    let ky = ry * K;
    let mut pb = PathBuilder::new();
    pb.move_to(cx - rx, cy);
    pb.cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry);
    pb.cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy);
    pb.cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry);
    pb.cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy);
    pb.close();
    pb.finish()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_element_transformed(
    elem: &SlideElement,
    override_bounds: Option<Rect>,
    rotation_deg: f32,
    opacity: f32,
    theme: &Theme,
    sx: f32,
    sy: f32,
    dst_buf: &mut [u32],
    target_w: u32,
    target_h: u32,
    len: usize,
) {
    let opacity = opacity.clamp(0.0, 1.0);
    if opacity <= 0.001 {
        return;
    }

    let needs_rotate = libm::fabsf(rotation_deg) > 0.01;
    if !needs_rotate && opacity >= 0.999 {
        if let Some(mut canvas) = Canvas::new(dst_buf, target_w, target_h) {
            render_single_element(elem, override_bounds, None, theme, sx, sy, &mut canvas, target_w, target_h);
        }
        return;
    }

    // Check fast vector rotation path for Shapes
    if let SlideElement::Shape(s) = elem {
        let rect = override_bounds.unwrap_or_else(|| scale_rect(s.bounds, sx, sy));
        if let Some(mut canvas) = Canvas::new(dst_buf, target_w, target_h) {
            render_interpolated_shape(
                s.kind,
                s.kind,
                rect,
                shape_corner_radius(s.kind) * sx,
                s.fill,
                s.stroke,
                rotation_deg,
                opacity,
                sx,
                &mut canvas,
                target_w,
                target_h,
            );
        }
        return;
    }

    let mut scratch = vec![0u32; len];
    if let Some(mut sc) = Canvas::new(&mut scratch, target_w, target_h) {
        if let SlideElement::TextBox(tb) = elem {
            let rect = override_bounds.unwrap_or_else(|| scale_rect(tb.bounds, sx, sy));
            if let Some(bg) = tb.background {
                sc.fill_round_rect(rect, 8.0 * sx, bg);
            }
            render_text_box(tb, rect, theme, sx, sy, sc.pixels_mut(), target_w, target_h, None);
        } else {
            render_single_element(elem, override_bounds, None, theme, sx, sy, &mut sc, target_w, target_h);
        }
    }

    if !needs_rotate {
        blend_buffer_over_alpha(&scratch, dst_buf, opacity, None, target_w, target_h);
        return;
    }

    let rect = override_bounds.unwrap_or_else(|| scale_rect(elem.bounds(), sx, sy));
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    let rad = rotation_deg * (core::f32::consts::PI / 180.0);
    let cos_t = libm::cosf(rad);
    let sin_t = libm::sinf(rad);

    // Rotated AABB extent
    let hw = rect.w * 0.5;
    let hh = rect.h * 0.5;
    let r_hw = (libm::fabsf(hw * cos_t) + libm::fabsf(hh * sin_t)) + 4.0;
    let r_hh = (libm::fabsf(hw * sin_t) + libm::fabsf(hh * cos_t)) + 4.0;
    let x0 = libm::floorf(cx - r_hw).max(0.0) as u32;
    let y0 = libm::floorf(cy - r_hh).max(0.0) as u32;
    let x1 = (libm::ceilf(cx + r_hw).max(0.0) as u32).min(target_w);
    let y1 = (libm::ceilf(cy + r_hh).max(0.0) as u32).min(target_h);

    let alpha_mul = (opacity * 255.0 + 0.5) as u32;
    let w_usize = target_w as usize;
    let h_usize = target_h as usize;

    for dy in y0..y1 {
        let vy = dy as f32 + 0.5 - cy;
        let dst_row = dy as usize * w_usize;
        for dx in x0..x1 {
            let vx = dx as f32 + 0.5 - cx;
            // Inverse rotate around center
            let src_xf = cx + vx * cos_t + vy * sin_t;
            let src_yf = cy - vx * sin_t + vy * cos_t;
            let sx_i = libm::floorf(src_xf) as i32;
            let sy_i = libm::floorf(src_yf) as i32;
            if sx_i < 0 || sy_i < 0 || sx_i >= target_w as i32 || sy_i >= target_h as i32 {
                continue;
            }
            let src = scratch[sy_i as usize * w_usize + sx_i as usize];
            let sa = (src >> 24) & 0xFF;
            if sa == 0 {
                continue;
            }
            let eff_a = ((sa * alpha_mul + 127) / 255).min(255);
            if eff_a == 0 {
                continue;
            }
            let sr = (((src >> 16) & 0xFF) * alpha_mul + 127) / 255;
            let sg = (((src >> 8) & 0xFF) * alpha_mul + 127) / 255;
            let sb = ((src & 0xFF) * alpha_mul + 127) / 255;

            let idx = dst_row + dx as usize;
            if idx >= w_usize * h_usize {
                continue;
            }
            let dst = dst_buf[idx];
            let da = (dst >> 24) & 0xFF;
            let dr = (dst >> 16) & 0xFF;
            let dg = (dst >> 8) & 0xFF;
            let db = dst & 0xFF;

            let inv_a = 255 - eff_a;
            let out_a = (eff_a + (da * inv_a + 127) / 255).min(255);
            let out_r = (sr + (dr * inv_a + 127) / 255).min(255);
            let out_g = (sg + (dg * inv_a + 127) / 255).min(255);
            let out_b = (sb + (db * inv_a + 127) / 255).min(255);
            dst_buf[idx] = (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b;
        }
    }
}

/// Apply Morph transition to two slides in a deck by indices.
pub fn render_deck_morph(
    deck: &Deck,
    from_idx: usize,
    to_idx: usize,
    progress: f32,
    buffer: &mut [u32],
    target_w: u32,
    target_h: u32,
) -> bool {
    let (Some(from_s), Some(to_s)) = (deck.get_slide(from_idx), deck.get_slide(to_idx)) else {
        return false;
    };
    render_morph_frame(
        from_s,
        to_s,
        &deck.theme,
        deck.aspect_ratio,
        progress,
        buffer,
        target_w,
        target_h,
    );
    true
}
