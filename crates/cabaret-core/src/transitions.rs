//! Studio Mode, Multiview layouts, all 29 procedural wipe patterns, and track-matte stingers.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::f32::consts::PI;

use crate::compositor::{blend_pixel, BlendMode, FrameBuffer, LinearRgba, MixColorSpace};
use crate::math::{
    absf, atan2f, clamp01, cosf, fbm_2d, floorf, fract, hash2d, hash2d_vec2, lerp, powf,
    rec709_luma, sinf, smoothstep, sqrtf,
};
use crate::scene::{SceneCollection, Studio, StudioId};

/// Directional axis for Swipe, Slide, Cover, and Uncover transitions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WipeDirection {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

impl WipeDirection {
    /// Unit motion vector `(dx, dy)` for incoming transition frame movement.
    pub const fn vector(self) -> [f32; 2] {
        match self {
            Self::Left => [-1.0, 0.0],
            Self::Right => [1.0, 0.0],
            Self::Up => [0.0, -1.0],
            Self::Down => [0.0, 1.0],
        }
    }
}

/// All 29 procedural SDF / geometric wipe patterns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WipePattern {
    #[default]
    Linear,
    BarnH,
    BarnV,
    Box,
    IrisCircle,
    IrisDiamond,
    IrisHeart,
    IrisStar,
    Clock,
    Fan,
    DoubleFan,
    Pinwheel,
    BlindsH,
    BlindsV,
    Checkerboard,
    Spiral,
    Venetian,
    Diagonal,
    Cross,
    ZigZag,
    Sawtooth,
    RadialBurst,
    Ring,
    Split4,
    Curtain,
    Wave,
    CloudNoise,
    FractalCell,
    DissolveDither,
}

/// Canonical array containing all 29 [`WipePattern`] variants.
pub const ALL_WIPE_PATTERNS: [WipePattern; 29] = [
    WipePattern::Linear,
    WipePattern::BarnH,
    WipePattern::BarnV,
    WipePattern::Box,
    WipePattern::IrisCircle,
    WipePattern::IrisDiamond,
    WipePattern::IrisHeart,
    WipePattern::IrisStar,
    WipePattern::Clock,
    WipePattern::Fan,
    WipePattern::DoubleFan,
    WipePattern::Pinwheel,
    WipePattern::BlindsH,
    WipePattern::BlindsV,
    WipePattern::Checkerboard,
    WipePattern::Spiral,
    WipePattern::Venetian,
    WipePattern::Diagonal,
    WipePattern::Cross,
    WipePattern::ZigZag,
    WipePattern::Sawtooth,
    WipePattern::RadialBurst,
    WipePattern::Ring,
    WipePattern::Split4,
    WipePattern::Curtain,
    WipePattern::Wave,
    WipePattern::CloudNoise,
    WipePattern::FractalCell,
    WipePattern::DissolveDither,
];

/// Cut-point timing for a Stinger transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StingerCutPoint {
    Frame(u32),
    Milliseconds(u32),
    Normalized(f32),
}

impl Default for StingerCutPoint {
    fn default() -> Self {
        Self::Normalized(0.5)
    }
}

/// Track-matte mode for Stinger transitions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TrackMatte {
    /// Separate or embedded alpha matte channel.
    #[default]
    Alpha,
    /// Full-range luminance matte (white = incoming B, black = outgoing A).
    LumaFull,
    /// Left half of stinger frame is RGB overlay, right half is grayscale matte.
    SplitHorizontal,
    /// Top half of stinger frame is RGB overlay, bottom half is grayscale matte.
    SplitVertical,
}

/// Audio behavior during a Stinger transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StingerAudioMode {
    FadeOutFadeIn,
    Crossfade,
    DuckProgram { duck_db: f32 },
}

impl Default for StingerAudioMode {
    fn default() -> Self {
        Self::Crossfade
    }
}

/// Configuration for a Stinger video+audio transition.
#[derive(Clone, Debug, PartialEq)]
pub struct StingerConfig {
    pub asset_path: String,
    pub duration_ms: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub cut_point: StingerCutPoint,
    pub track_matte: Option<TrackMatte>,
    pub audio_mode: StingerAudioMode,
    pub monitoring_enabled: bool,
}

impl Default for StingerConfig {
    fn default() -> Self {
        Self {
            asset_path: String::new(),
            duration_ms: 1000,
            fps_num: 60,
            fps_den: 1,
            cut_point: StingerCutPoint::Normalized(0.5),
            track_matte: Some(TrackMatte::Alpha),
            audio_mode: StingerAudioMode::Crossfade,
            monitoring_enabled: false,
        }
    }
}

impl StingerConfig {
    /// Returns the normalized cut point `t in [0.0, 1.0]` where Program switches from A to B.
    pub fn cut_point_normalized(&self) -> f32 {
        let dur_ms = self.duration_ms.max(1) as f32;
        match self.cut_point {
            StingerCutPoint::Normalized(n) => clamp01(n),
            StingerCutPoint::Milliseconds(ms) => clamp01(ms as f32 / dur_ms),
            StingerCutPoint::Frame(frame) => {
                let fps = (self.fps_num.max(1) as f32) / (self.fps_den.max(1) as f32);
                let total_frames = (dur_ms / 1000.0) * fps;
                if total_frames <= 0.0 {
                    0.5
                } else {
                    clamp01(frame as f32 / total_frames)
                }
            }
        }
    }
}

/// All supported transition kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum TransitionKind {
    Cut,
    Fade,
    FadeThroughColor {
        color: LinearRgba,
        peak_point: f32,
    },
    Swipe {
        direction: WipeDirection,
        slide_out: bool,
    },
    Slide {
        direction: WipeDirection,
    },
    Cover {
        direction: WipeDirection,
    },
    Uncover {
        direction: WipeDirection,
    },
    Wipe29 {
        pattern: WipePattern,
        softness: f32,
        invert: bool,
        angle_deg: f32,
    },
    Stinger {
        config: StingerConfig,
    },
}

impl Default for TransitionKind {
    fn default() -> Self {
        Self::Fade
    }
}

/// Transition specification pairing a [`TransitionKind`] with its duration in milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionSpec {
    pub name: String,
    pub kind: TransitionKind,
    pub duration_ms: u32,
}

impl Default for TransitionSpec {
    fn default() -> Self {
        Self {
            name: String::from("Fade"),
            kind: TransitionKind::Fade,
            duration_ms: 300,
        }
    }
}

impl TransitionSpec {
    pub fn cut() -> Self {
        Self {
            name: String::from("Cut"),
            kind: TransitionKind::Cut,
            duration_ms: 0,
        }
    }

    pub fn fade(duration_ms: u32) -> Self {
        Self {
            name: String::from("Fade"),
            kind: TransitionKind::Fade,
            duration_ms,
        }
    }

    pub fn wipe(pattern: WipePattern, duration_ms: u32, softness: f32) -> Self {
        Self {
            name: format!("Wipe::{pattern:?}"),
            kind: TransitionKind::Wipe29 {
                pattern,
                softness,
                invert: false,
                angle_deg: 0.0,
            },
            duration_ms,
        }
    }
}

/// 8x8 Bayer ordered-dither matrix normalized to `[0..64)`.
const BAYER_8X8: [u8; 64] = [
    0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38,
    60, 28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25,
    15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
];

/// Computes the normalized procedural wipe threshold in `[0.0, 1.0]` at UV coordinates `(u, v) in [0, 1]^2`.
pub fn wipe_threshold(pattern: WipePattern, u: f32, v: f32, angle_deg: f32) -> f32 {
    let u = clamp01(u);
    let v = clamp01(v);
    let dx = u - 0.5;
    let dy = v - 0.5;

    let rad = angle_deg * (PI / 180.0);
    let c = cosf(rad);
    let s = sinf(rad);
    let rx = dx * c - dy * s;
    let ry = dx * s + dy * c;
    let cx = clamp01(rx + 0.5);
    let cy = clamp01(ry + 0.5);

    let r = sqrtf(dx * dx + dy * dy);
    let r_norm = clamp01(r / 0.707_106_77);
    let phi_cw = {
        let a = atan2f(rx, -ry);
        if a < 0.0 {
            (a + 2.0 * PI) / (2.0 * PI)
        } else {
            a / (2.0 * PI)
        }
    };

    match pattern {
        WipePattern::Linear => cx,
        WipePattern::BarnH => clamp01(absf(rx) * 2.0),
        WipePattern::BarnV => clamp01(absf(ry) * 2.0),
        WipePattern::Box => clamp01(absf(rx).max(absf(ry)) * 2.0),
        WipePattern::IrisCircle => r_norm,
        WipePattern::IrisDiamond => clamp01(absf(rx) + absf(ry)),
        WipePattern::IrisHeart => {
            // Symmetric algebraic heart distance field normalized to [0..1].
            let x = absf(rx) * 2.3;
            let y = (-ry + 0.14) * 2.3;
            let k = sqrtf(x.max(0.0));
            let d = sqrtf(x * x + (y - 0.5 * k) * (y - 0.5 * k)) / 1.45;
            clamp01(d)
        }
        WipePattern::IrisStar => {
            let sector = absf(fract(phi_cw * 5.0 + 0.5) - 0.5) * 2.0;
            let star_envelope = lerp(1.0, 0.42, sector);
            clamp01((r / star_envelope) / 0.95)
        }
        WipePattern::Clock => clamp01(phi_cw),
        WipePattern::Fan => clamp01(absf(atan2f(rx, -ry)) / PI),
        WipePattern::DoubleFan => clamp01(absf(atan2f(ry, absf(rx))) / (PI * 0.5)),
        WipePattern::Pinwheel => clamp01(fract(phi_cw * 4.0)),
        WipePattern::BlindsH => clamp01(fract(cy * 8.0)),
        WipePattern::BlindsV => clamp01(fract(cx * 8.0)),
        WipePattern::Checkerboard => {
            let ix = (floorf(u * 8.0) as i32).clamp(0, 7);
            let iy = (floorf(v * 8.0) as i32).clamp(0, 7);
            let parity = ((ix + iy) & 1) as f32;
            let local = fract(u * 8.0);
            clamp01(0.5 * parity + 0.5 * local)
        }
        WipePattern::Spiral => clamp01(0.55 * r_norm + 0.45 * fract(phi_cw + r * 2.5)),
        WipePattern::Venetian => clamp01(absf(fract(cy * 12.0) - 0.5) * 2.0),
        WipePattern::Diagonal => clamp01((u + v) * 0.5),
        WipePattern::Cross => clamp01(absf(rx).min(absf(ry)) * 2.0),
        WipePattern::ZigZag => {
            let teeth = absf(fract(cy * 6.0) - 0.5) * 2.0 - 0.5;
            clamp01(cx + 0.14 * teeth)
        }
        WipePattern::Sawtooth => {
            let teeth = fract(cy * 8.0) - 0.5;
            clamp01(cx + 0.16 * teeth)
        }
        WipePattern::RadialBurst => {
            let rays = absf(sinf(atan2f(ry, rx) * 6.0));
            clamp01(0.5 * r_norm + 0.5 * rays)
        }
        WipePattern::Ring => clamp01(0.4 * r_norm + 0.6 * absf(sinf(r * 18.0))),
        WipePattern::Split4 => {
            let qx = absf(fract(u * 2.0) - 0.5);
            let qy = absf(fract(v * 2.0) - 0.5);
            clamp01(qx + qy)
        }
        WipePattern::Curtain => clamp01(absf(rx) * 2.0 - 0.10 * (1.0 - v)),
        WipePattern::Wave => clamp01(cx + 0.08 * sinf(cy * 6.0 * PI)),
        WipePattern::CloudNoise => clamp01(fbm_2d(u * 6.0, v * 6.0, 4)),
        WipePattern::FractalCell => {
            // Cellular Voronoi F1 distance across a 6x6 jittered lattice.
            let gx = u * 6.0;
            let gy = v * 6.0;
            let ix = floorf(gx) as i32;
            let iy = floorf(gy) as i32;
            let mut min_d2 = 10.0_f32;
            for oy in -1..=1 {
                for ox in -1..=1 {
                    let cx_i = ix + ox;
                    let cy_i = iy + oy;
                    let j = hash2d_vec2(cx_i, cy_i);
                    let px = cx_i as f32 + j[0];
                    let py = cy_i as f32 + j[1];
                    let ddx = px - gx;
                    let ddy = py - gy;
                    min_d2 = min_d2.min(ddx * ddx + ddy * ddy);
                }
            }
            clamp01(sqrtf(min_d2) / 1.1)
        }
        WipePattern::DissolveDither => {
            hash2d(floorf(u * 1999.0) as i32, floorf(v * 1997.0) as i32)
        }
    }
}

/// Evaluates wipe blend factor `alpha in [0.0, 1.0]` (0 = outgoing A, 1 = incoming B)
/// ensuring zero edge leakage at `t == 0.0` and `t == 1.0` even when `softness > 0.0`.
pub fn wipe_alpha(
    pattern: WipePattern,
    u: f32,
    v: f32,
    t: f32,
    softness: f32,
    invert: bool,
    angle_deg: f32,
) -> f32 {
    let t = clamp01(t);
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let s = softness.clamp(0.001, 0.5);
    let mut thresh = wipe_threshold(pattern, u, v, angle_deg);
    if invert {
        thresh = 1.0 - thresh;
    }
    let t_mapped = -s + t * (1.0 + 2.0 * s);
    smoothstep(thresh - s, thresh + s, t_mapped)
}

/// Audio gains emitted during a Stinger transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StingerAudioGains {
    pub outgoing_a_gain: f32,
    pub incoming_b_gain: f32,
    pub stinger_gain: f32,
}

/// Evaluates audio gain envelopes for a Stinger transition at progress `t in [0.0, 1.0]`.
pub fn evaluate_stinger_audio(t: f32, config: &StingerConfig) -> StingerAudioGains {
    let t = clamp01(t);
    let cut = config.cut_point_normalized().clamp(0.05, 0.95);
    let stinger_gain = 1.0;

    match config.audio_mode {
        StingerAudioMode::FadeOutFadeIn => {
            let a = if t < cut { 1.0 - t / cut } else { 0.0 };
            let b = if t >= cut { (t - cut) / (1.0 - cut) } else { 0.0 };
            StingerAudioGains {
                outgoing_a_gain: a,
                incoming_b_gain: b,
                stinger_gain,
            }
        }
        StingerAudioMode::Crossfade => {
            // Equal-power crossfade centered around cut_point.
            let x = clamp01((t - (cut - 0.25)) / 0.5);
            StingerAudioGains {
                outgoing_a_gain: cosf(x * PI * 0.5),
                incoming_b_gain: sinf(x * PI * 0.5),
                stinger_gain,
            }
        }
        StingerAudioMode::DuckProgram { duck_db } => {
            let duck_lin = powf(10.0, duck_db.min(0.0) * 0.05);
            let env = 1.0 - sinf(t * PI) * (1.0 - duck_lin);
            let a = if t < cut { env } else { 0.0 };
            let b = if t >= cut { env } else { 0.0 };
            StingerAudioGains {
                outgoing_a_gain: a,
                incoming_b_gain: b,
                stinger_gain,
            }
        }
    }
}

/// Composites a transition frame between `a` (outgoing Program) and `b` (incoming Preview) at `t in [0, 1]`.
pub fn render_transition_frame(
    a: &FrameBuffer,
    b: &FrameBuffer,
    kind: &TransitionKind,
    t: f32,
    stinger_frame: Option<&FrameBuffer>,
    matte_frame: Option<&FrameBuffer>,
) -> FrameBuffer {
    let t = clamp01(t);
    let w = a.width.max(b.width);
    let h = a.height.max(b.height);
    let mut out = a.clone();

    match kind {
        TransitionKind::Cut => {
            return if t >= 0.5 { b.clone() } else { a.clone() };
        }
        TransitionKind::Fade => {
            for (dst, (&pa, &pb)) in out.pixels.iter_mut().zip(a.pixels.iter().zip(b.pixels.iter())) {
                *dst = [
                    lerp(pa[0], pb[0], t),
                    lerp(pa[1], pb[1], t),
                    lerp(pa[2], pb[2], t),
                    lerp(pa[3], pb[3], t),
                ];
            }
        }
        TransitionKind::FadeThroughColor { color, peak_point } => {
            let peak = peak_point.clamp(0.1, 0.9);
            if t < peak {
                let k = clamp01(t / peak);
                for (dst, &pa) in out.pixels.iter_mut().zip(a.pixels.iter()) {
                    *dst = [
                        lerp(pa[0], color[0], k),
                        lerp(pa[1], color[1], k),
                        lerp(pa[2], color[2], k),
                        lerp(pa[3], color[3], k),
                    ];
                }
            } else {
                let k = clamp01((t - peak) / (1.0 - peak));
                for (dst, &pb) in out.pixels.iter_mut().zip(b.pixels.iter()) {
                    *dst = [
                        lerp(color[0], pb[0], k),
                        lerp(color[1], pb[1], k),
                        lerp(color[2], pb[2], k),
                        lerp(color[3], pb[3], k),
                    ];
                }
            }
        }
        TransitionKind::Swipe {
            direction,
            slide_out,
        } => {
            let dir = direction.vector();
            for y in 0..h {
                for x in 0..w {
                    let u = (x as f32 + 0.5) / w as f32;
                    let v = (y as f32 + 0.5) / h as f32;
                    let prog = match direction {
                        WipeDirection::Left => 1.0 - u,
                        WipeDirection::Right => u,
                        WipeDirection::Up => 1.0 - v,
                        WipeDirection::Down => v,
                    };
                    if prog <= t {
                        let sx = (x as i32 - (dir[0] * (1.0 - t) * w as f32) as i32)
                            .clamp(0, w as i32 - 1) as u32;
                        let sy = (y as i32 - (dir[1] * (1.0 - t) * h as f32) as i32)
                            .clamp(0, h as i32 - 1) as u32;
                        out.set_pixel(x, y, b.get_pixel(sx, sy).unwrap_or([0.0; 4]));
                    } else if *slide_out {
                        let sx = (x as i32 - (dir[0] * t * w as f32) as i32)
                            .clamp(0, w as i32 - 1) as u32;
                        let sy = (y as i32 - (dir[1] * t * h as f32) as i32)
                            .clamp(0, h as i32 - 1) as u32;
                        out.set_pixel(x, y, a.get_pixel(sx, sy).unwrap_or([0.0; 4]));
                    }
                }
            }
        }
        TransitionKind::Slide { direction } | TransitionKind::Cover { direction } => {
            let dir = direction.vector();
            let shift_x = (dir[0] * (1.0 - t) * w as f32) as i32;
            let shift_y = (dir[1] * (1.0 - t) * h as f32) as i32;
            for y in 0..h {
                for x in 0..w {
                    let bx = x as i32 + shift_x;
                    let by = y as i32 + shift_y;
                    if bx >= 0 && bx < w as i32 && by >= 0 && by < h as i32 {
                        out.set_pixel(x, y, b.get_pixel(bx as u32, by as u32).unwrap_or([0.0; 4]));
                    } else if matches!(kind, TransitionKind::Slide { .. }) {
                        let ax = (x as i32 - (dir[0] * t * w as f32) as i32)
                            .clamp(0, w as i32 - 1) as u32;
                        let ay = (y as i32 - (dir[1] * t * h as f32) as i32)
                            .clamp(0, h as i32 - 1) as u32;
                        out.set_pixel(x, y, a.get_pixel(ax, ay).unwrap_or([0.0; 4]));
                    }
                }
            }
        }
        TransitionKind::Uncover { direction } => {
            let dir = direction.vector();
            let shift_x = (dir[0] * t * w as f32) as i32;
            let shift_y = (dir[1] * t * h as f32) as i32;
            for y in 0..h {
                for x in 0..w {
                    let ax = x as i32 - shift_x;
                    let ay = y as i32 - shift_y;
                    if ax >= 0 && ax < w as i32 && ay >= 0 && ay < h as i32 {
                        out.set_pixel(x, y, a.get_pixel(ax as u32, ay as u32).unwrap_or([0.0; 4]));
                    } else {
                        out.set_pixel(x, y, b.get_pixel(x, y).unwrap_or([0.0; 4]));
                    }
                }
            }
        }
        TransitionKind::Wipe29 {
            pattern,
            softness,
            invert,
            angle_deg,
        } => {
            for y in 0..h {
                let v = (y as f32 + 0.5) / h as f32;
                for x in 0..w {
                    let u = (x as f32 + 0.5) / w as f32;
                    let k = if *pattern == WipePattern::DissolveDither {
                        if t <= 0.0 {
                            0.0
                        } else if t >= 1.0 {
                            1.0
                        } else {
                            let s = softness.clamp(0.001, 0.5);
                            let mut thresh =
                                (BAYER_8X8[(y as usize & 7) * 8 + (x as usize & 7)] as f32 + 0.5)
                                    / 64.0;
                            if *invert {
                                thresh = 1.0 - thresh;
                            }
                            let t_mapped = -s + t * (1.0 + 2.0 * s);
                            smoothstep(thresh - s, thresh + s, t_mapped)
                        }
                    } else {
                        wipe_alpha(*pattern, u, v, t, *softness, *invert, *angle_deg)
                    };
                    let pa = a.get_pixel(x, y).unwrap_or([0.0; 4]);
                    let pb = b.get_pixel(x, y).unwrap_or([0.0; 4]);
                    out.set_pixel(
                        x,
                        y,
                        [
                            lerp(pa[0], pb[0], k),
                            lerp(pa[1], pb[1], k),
                            lerp(pa[2], pb[2], k),
                            lerp(pa[3], pb[3], k),
                        ],
                    );
                }
            }
        }
        TransitionKind::Stinger { config } => {
            let cut_t = config.cut_point_normalized();
            for y in 0..h {
                for x in 0..w {
                    let pa = a.get_pixel(x, y).unwrap_or([0.0; 4]);
                    let pb = b.get_pixel(x, y).unwrap_or([0.0; 4]);

                    // Determine base mix from track matte or cut point.
                    let matte_mix = match config.track_matte {
                        None => {
                            if t >= cut_t {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        Some(TrackMatte::Alpha) => {
                            if let Some(mf) = matte_frame {
                                mf.get_pixel(x.min(mf.width - 1), y.min(mf.height - 1))
                                    .map(|p| clamp01(p[3]))
                                    .unwrap_or(if t >= cut_t { 1.0 } else { 0.0 })
                            } else if let Some(sf) = stinger_frame {
                                sf.get_pixel(x.min(sf.width - 1), y.min(sf.height - 1))
                                    .map(|p| clamp01(p[3]))
                                    .unwrap_or(if t >= cut_t { 1.0 } else { 0.0 })
                            } else if t >= cut_t {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        Some(TrackMatte::LumaFull) => {
                            let src = matte_frame.or(stinger_frame);
                            if let Some(mf) = src {
                                mf.get_pixel(x.min(mf.width - 1), y.min(mf.height - 1))
                                    .map(|p| clamp01(rec709_luma([p[0], p[1], p[2]])))
                                    .unwrap_or(if t >= cut_t { 1.0 } else { 0.0 })
                            } else if t >= cut_t {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        Some(TrackMatte::SplitHorizontal) => {
                            if let Some(sf) = stinger_frame {
                                let half_w = (sf.width / 2).max(1);
                                let mx = (half_w + (x * half_w / w)).min(sf.width - 1);
                                let my = (y * sf.height / h).min(sf.height - 1);
                                sf.get_pixel(mx, my)
                                    .map(|p| clamp01(rec709_luma([p[0], p[1], p[2]])))
                                    .unwrap_or(if t >= cut_t { 1.0 } else { 0.0 })
                            } else if t >= cut_t {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        Some(TrackMatte::SplitVertical) => {
                            if let Some(sf) = stinger_frame {
                                let half_h = (sf.height / 2).max(1);
                                let mx = (x * sf.width / w).min(sf.width - 1);
                                let my = (half_h + (y * half_h / h)).min(sf.height - 1);
                                sf.get_pixel(mx, my)
                                    .map(|p| clamp01(rec709_luma([p[0], p[1], p[2]])))
                                    .unwrap_or(if t >= cut_t { 1.0 } else { 0.0 })
                            } else if t >= cut_t {
                                1.0
                            } else {
                                0.0
                            }
                        }
                    };

                    let base = [
                        lerp(pa[0], pb[0], matte_mix),
                        lerp(pa[1], pb[1], matte_mix),
                        lerp(pa[2], pb[2], matte_mix),
                        lerp(pa[3], pb[3], matte_mix),
                    ];

                    // Composite stinger RGB overlay on top of the transition base.
                    let final_px = if let Some(sf) = stinger_frame {
                        let (sx, sy) = match config.track_matte {
                            Some(TrackMatte::SplitHorizontal) => {
                                let half_w = (sf.width / 2).max(1);
                                ((x * half_w / w).min(half_w - 1), (y * sf.height / h).min(sf.height - 1))
                            }
                            Some(TrackMatte::SplitVertical) => {
                                let half_h = (sf.height / 2).max(1);
                                ((x * sf.width / w).min(sf.width - 1), (y * half_h / h).min(half_h - 1))
                            }
                            _ => (
                                (x * sf.width / w).min(sf.width - 1),
                                (y * sf.height / h).min(sf.height - 1),
                            ),
                        };
                        let st = sf.get_pixel(sx, sy).unwrap_or([0.0; 4]);
                        blend_pixel(base, st, BlendMode::Normal, 1.0, MixColorSpace::Linear)
                    } else {
                        base
                    };
                    out.set_pixel(x, y, final_px);
                }
            }
        }
    }
    out
}

/// Active running automated transition inside [`StudioMode`].
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveTransition {
    pub from_studio: StudioId,
    pub to_studio: StudioId,
    pub spec: TransitionSpec,
    pub elapsed_ms: f32,
}

/// Outcome of advancing `StudioMode` by `dt_ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransitionTick {
    Idle,
    InProgress { t_bar: f32 },
    Completed { new_program: StudioId },
}

/// Broadcast Studio Mode (Preview / Program split, interactive T-Bar, and isolated staging).
#[derive(Clone, Debug, PartialEq)]
pub struct StudioMode {
    pub enabled: bool,
    pub preview_studio: StudioId,
    pub program_studio: StudioId,
    pub t_bar: f32,
    pub active_transition: TransitionSpec,
    pub running: Option<ActiveTransition>,
    /// When `true`, live edits to Preview are staged in `staged_preview` without altering Program until `take()`.
    pub copy_sources_isolated: bool,
    pub staged_preview: Option<Studio>,
}

impl StudioMode {
    pub fn new(preview_studio: StudioId, program_studio: StudioId) -> Self {
        Self {
            enabled: true,
            preview_studio,
            program_studio,
            t_bar: 0.0,
            active_transition: TransitionSpec::default(),
            running: None,
            copy_sources_isolated: true,
            staged_preview: None,
        }
    }

    /// Stages an isolated working copy of the preview studio if `copy_sources_isolated` is enabled.
    pub fn begin_isolated_preview_edit<'a>(
        &'a mut self,
        collection: &SceneCollection,
    ) -> Option<&'a mut Studio> {
        if !self.copy_sources_isolated {
            return None;
        }
        if self.staged_preview.is_none() {
            if let Some(studio) = collection.studio(self.preview_studio) {
                self.staged_preview = Some(studio.clone());
            }
        }
        self.staged_preview.as_mut()
    }

    /// Swaps Preview and Program studio IDs immediately.
    pub fn swap(&mut self) {
        core::mem::swap(&mut self.preview_studio, &mut self.program_studio);
        self.t_bar = 0.0;
        self.running = None;
    }

    /// Resolves the effective transition for switching to `to_studio`, honoring per-studio overrides.
    pub fn resolve_transition(
        &self,
        collection: Option<&SceneCollection>,
        to_studio: StudioId,
    ) -> TransitionSpec {
        if let Some(col) = collection {
            if let Some(studio) = col.studio(to_studio) {
                if let Some(override_tr) = &studio.override_transition {
                    return override_tr.clone();
                }
            }
        }
        self.active_transition.clone()
    }

    /// Initiates a transition from Program to Preview (`TAKE`).
    ///
    /// Commits any isolated staged preview edits into `collection` before switching.
    pub fn take(&mut self, mut collection: Option<&mut SceneCollection>) -> TransitionTick {
        // Commit isolated staged preview if present.
        if let Some(staged) = self.staged_preview.take() {
            if let Some(col) = collection.as_deref_mut() {
                col.replace_studio_in_place(staged);
            }
        }

        let spec = self.resolve_transition(collection.as_deref(), self.preview_studio);
        if matches!(spec.kind, TransitionKind::Cut) || spec.duration_ms == 0 {
            let new_program = self.preview_studio;
            self.swap();
            return TransitionTick::Completed { new_program };
        }

        self.t_bar = 0.0;
        self.running = Some(ActiveTransition {
            from_studio: self.program_studio,
            to_studio: self.preview_studio,
            spec,
            elapsed_ms: 0.0,
        });
        TransitionTick::InProgress { t_bar: 0.0 }
    }

    /// Initiates `TAKE` using any per-studio transition override from `collection`.
    pub fn take_with_collection(&mut self, collection: &mut SceneCollection) -> TransitionTick {
        if let Some(staged) = self.staged_preview.take() {
            collection.replace_studio_in_place(staged);
        }
        let spec = self.resolve_transition(Some(collection), self.preview_studio);
        if matches!(spec.kind, TransitionKind::Cut) || spec.duration_ms == 0 {
            let new_program = self.preview_studio;
            self.swap();
            return TransitionTick::Completed { new_program };
        }
        self.t_bar = 0.0;
        self.running = Some(ActiveTransition {
            from_studio: self.program_studio,
            to_studio: self.preview_studio,
            spec,
            elapsed_ms: 0.0,
        });
        TransitionTick::InProgress { t_bar: 0.0 }
    }

    /// Moves the interactive hardware/UI T-Bar (`0.0..=1.0`).
    pub fn set_t_bar(&mut self, value: f32) -> f32 {
        self.t_bar = clamp01(value);
        self.t_bar
    }

    /// Completes an interactive T-Bar drag when it reaches `>= 0.999` (or commits immediately).
    pub fn complete_t_bar(&mut self, collection: Option<&mut SceneCollection>) -> Option<StudioId> {
        if self.t_bar >= 0.999 {
            if let Some(staged) = self.staged_preview.take() {
                if let Some(col) = collection {
                    col.replace_studio_in_place(staged);
                }
            }
            self.swap();
            Some(self.program_studio)
        } else {
            None
        }
    }

    /// Advances any active automated transition by `dt_ms`.
    pub fn tick(&mut self, dt_ms: f32) -> TransitionTick {
        let Some(ref mut active) = self.running else {
            return TransitionTick::Idle;
        };
        active.elapsed_ms += dt_ms.max(0.0);
        let dur = active.spec.duration_ms.max(1) as f32;
        let t = clamp01(active.elapsed_ms / dur);
        self.t_bar = t;
        if t >= 1.0 {
            let new_program = active.to_studio;
            self.running = None;
            self.swap();
            TransitionTick::Completed { new_program }
        } else {
            TransitionTick::InProgress { t_bar: t }
        }
    }
}

/// Evaluates a per-item Show/Hide visibility transition opacity `[0.0..=1.0]`.
pub fn evaluate_item_visibility_transition(
    becoming_visible: bool,
    t: f32,
    spec: Option<&TransitionSpec>,
) -> f32 {
    let t = clamp01(t);
    let eff_t = match spec {
        Some(s) if matches!(s.kind, TransitionKind::Cut) || s.duration_ms == 0 => {
            if t >= 0.5 {
                1.0
            } else {
                0.0
            }
        }
        _ => t,
    };
    if becoming_visible {
        eff_t
    } else {
        1.0 - eff_t
    }
}

/// Multiview preset configurations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MultiviewPreset {
    #[default]
    Standard8,
    Grid4,
    Grid9,
    Grid16,
    Grid25,
}

/// Tally light state for a Multiview viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TallyState {
    #[default]
    None,
    PreviewGreen,
    ProgramRed,
}

impl TallyState {
    pub const fn border_rgba(self) -> LinearRgba {
        match self {
            Self::ProgramRed => [1.0, 0.08, 0.08, 1.0],
            Self::PreviewGreen => [0.05, 0.88, 0.25, 1.0],
            Self::None => [0.22, 0.22, 0.25, 1.0],
        }
    }
}

/// Role assigned to a Multiview cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiviewRole {
    Preview,
    Program,
    StudioSlot(usize),
}

/// Axis-aligned pixel rectangle for Multiview viewports and safe areas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl ViewportRect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// 90% action-safe and 80% title-safe overlay rectangles for a viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SafeAreaGuides {
    pub action_safe_90: ViewportRect,
    pub title_safe_80: ViewportRect,
    pub center: [f32; 2],
}

impl SafeAreaGuides {
    pub fn from_viewport(vp: ViewportRect) -> Self {
        let inset = |frac: f32| -> ViewportRect {
            let m = (1.0 - frac) * 0.5;
            ViewportRect {
                x: vp.x + vp.w * m,
                y: vp.y + vp.h * m,
                w: vp.w * frac,
                h: vp.h * frac,
            }
        };
        Self {
            action_safe_90: inset(0.90),
            title_safe_80: inset(0.80),
            center: [vp.x + vp.w * 0.5, vp.y + vp.h * 0.5],
        }
    }
}

/// Individual computed viewport inside a [`MultiviewLayout`].
#[derive(Clone, Debug, PartialEq)]
pub struct MultiviewSlot {
    pub index: usize,
    pub role: MultiviewRole,
    pub studio_id: Option<StudioId>,
    pub rect: ViewportRect,
    pub tally: TallyState,
    pub border_color: LinearRgba,
    pub safe_areas: SafeAreaGuides,
    pub label: String,
}

/// Action triggered by clicking inside a Multiview viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiviewAction {
    ClickToPreview(StudioId),
    DoubleClickToTake(StudioId),
}

/// Complete Multiview layout engine with tally borders, safe areas, and click hit-testing.
#[derive(Clone, Debug, PartialEq)]
pub struct MultiviewLayout {
    pub preset: MultiviewPreset,
    pub width: f32,
    pub height: f32,
    pub padding: f32,
    pub show_safe_areas: bool,
    pub slots: Vec<MultiviewSlot>,
}

impl MultiviewLayout {
    /// Builds all viewports for `preset` within `(width, height)` and assigns live tally states.
    pub fn compute(
        preset: MultiviewPreset,
        width: f32,
        height: f32,
        padding: f32,
        studios: &[StudioId],
        mode: &StudioMode,
    ) -> Self {
        let mut slots = Vec::new();
        let pad = padding.max(0.0);

        let resolve_tally = |sid: Option<StudioId>| -> TallyState {
            match sid {
                Some(id) if id == mode.program_studio => TallyState::ProgramRed,
                Some(id) if id == mode.preview_studio => TallyState::PreviewGreen,
                _ => TallyState::None,
            }
        };

        match preset {
            MultiviewPreset::Standard8 => {
                // Top half: Preview (left) + Program (right).
                // Bottom half: 2 rows x 4 columns = 8 studio slots.
                let top_h = (height - pad * 3.0) * 0.5;
                let top_w = (width - pad * 3.0) * 0.5;

                let pv_rect = ViewportRect {
                    x: pad,
                    y: pad,
                    w: top_w,
                    h: top_h,
                };
                let pg_rect = ViewportRect {
                    x: pad * 2.0 + top_w,
                    y: pad,
                    w: top_w,
                    h: top_h,
                };

                slots.push(MultiviewSlot {
                    index: 0,
                    role: MultiviewRole::Preview,
                    studio_id: Some(mode.preview_studio),
                    rect: pv_rect,
                    tally: TallyState::PreviewGreen,
                    border_color: TallyState::PreviewGreen.border_rgba(),
                    safe_areas: SafeAreaGuides::from_viewport(pv_rect),
                    label: String::from("Preview"),
                });
                slots.push(MultiviewSlot {
                    index: 1,
                    role: MultiviewRole::Program,
                    studio_id: Some(mode.program_studio),
                    rect: pg_rect,
                    tally: TallyState::ProgramRed,
                    border_color: TallyState::ProgramRed.border_rgba(),
                    safe_areas: SafeAreaGuides::from_viewport(pg_rect),
                    label: String::from("Program"),
                });

                let bot_y0 = pad * 2.0 + top_h;
                let cell_w = (width - pad * 5.0) / 4.0;
                let cell_h = (top_h - pad) / 2.0;

                for i in 0..8 {
                    let row = i / 4;
                    let col = i % 4;
                    let rect = ViewportRect {
                        x: pad + col as f32 * (cell_w + pad),
                        y: bot_y0 + row as f32 * (cell_h + pad),
                        w: cell_w,
                        h: cell_h,
                    };
                    let sid = studios.get(i).copied();
                    let tally = resolve_tally(sid);
                    slots.push(MultiviewSlot {
                        index: i + 2,
                        role: MultiviewRole::StudioSlot(i),
                        studio_id: sid,
                        rect,
                        tally,
                        border_color: tally.border_rgba(),
                        safe_areas: SafeAreaGuides::from_viewport(rect),
                        label: format!("Studio {}", i + 1),
                    });
                }
            }
            MultiviewPreset::Grid4
            | MultiviewPreset::Grid9
            | MultiviewPreset::Grid16
            | MultiviewPreset::Grid25 => {
                let n = match preset {
                    MultiviewPreset::Grid4 => 2,
                    MultiviewPreset::Grid9 => 3,
                    MultiviewPreset::Grid16 => 4,
                    MultiviewPreset::Grid25 => 5,
                    MultiviewPreset::Standard8 => unreachable!(),
                };
                let cell_w = (width - pad * (n as f32 + 1.0)) / n as f32;
                let cell_h = (height - pad * (n as f32 + 1.0)) / n as f32;
                for i in 0..(n * n) {
                    let row = i / n;
                    let col = i % n;
                    let rect = ViewportRect {
                        x: pad + col as f32 * (cell_w + pad),
                        y: pad + row as f32 * (cell_h + pad),
                        w: cell_w,
                        h: cell_h,
                    };
                    let sid = studios.get(i).copied();
                    let tally = resolve_tally(sid);
                    slots.push(MultiviewSlot {
                        index: i,
                        role: MultiviewRole::StudioSlot(i),
                        studio_id: sid,
                        rect,
                        tally,
                        border_color: tally.border_rgba(),
                        safe_areas: SafeAreaGuides::from_viewport(rect),
                        label: format!("Studio {}", i + 1),
                    });
                }
            }
        }

        Self {
            preset,
            width,
            height,
            padding: pad,
            show_safe_areas: true,
            slots,
        }
    }

    /// Returns the slot containing `(px, py)` if any.
    pub fn hit_test(&self, px: f32, py: f32) -> Option<&MultiviewSlot> {
        self.slots.iter().find(|slot| slot.rect.contains(px, py))
    }

    /// Dispatches a click (`click_count == 1` -> Preview, `>= 2` -> Take) against `StudioMode`.
    pub fn handle_click(
        &self,
        px: f32,
        py: f32,
        click_count: u8,
        mode: &mut StudioMode,
        collection: Option<&mut SceneCollection>,
    ) -> Option<MultiviewAction> {
        let slot = self.hit_test(px, py)?;
        let sid = slot.studio_id?;
        if click_count >= 2 {
            mode.preview_studio = sid;
            mode.take(collection);
            Some(MultiviewAction::DoubleClickToTake(sid))
        } else {
            mode.preview_studio = sid;
            Some(MultiviewAction::ClickToPreview(sid))
        }
    }
}
