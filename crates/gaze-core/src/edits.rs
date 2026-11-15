// SPDX-License-Identifier: GPL-3.0-only
//! Non-destructive photo edits (crop, geometry, linear-light EV exposure with filmic rolloff,
//! contrast, warmth, saturation, and shared `batik-core` filter presets & stacks).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub use batik_core::adjust::{CurvesAdjustment, CurvesPreset, HueSatAdjustment};
pub use batik_core::blend::{pack_argb, split_argb};
pub use batik_core::filters::{apply_filter_stack, FilterNode, FilterSpec, FilterStack};
use sex_image::Image;

/// Aspect ratio presets for non-destructive cropping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CropAspect {
    #[default]
    Free,
    Square1x1,
    FourThree,
    SixteenNine,
}

impl CropAspect {
    pub const ALL: [CropAspect; 4] = [
        CropAspect::Free,
        CropAspect::Square1x1,
        CropAspect::FourThree,
        CropAspect::SixteenNine,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CropAspect::Free => "Free",
            CropAspect::Square1x1 => "1:1",
            CropAspect::FourThree => "4:3",
            CropAspect::SixteenNine => "16:9",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            CropAspect::Free => "free",
            CropAspect::Square1x1 => "1x1",
            CropAspect::FourThree => "4x3",
            CropAspect::SixteenNine => "16x9",
        }
    }

    pub fn from_slug(s: &str) -> Self {
        match s {
            "1x1" => CropAspect::Square1x1,
            "4x3" => CropAspect::FourThree,
            "16x9" => CropAspect::SixteenNine,
            _ => CropAspect::Free,
        }
    }

    pub fn ratio(self) -> Option<(u32, u32)> {
        match self {
            CropAspect::Free => None,
            CropAspect::Square1x1 => Some((1, 1)),
            CropAspect::FourThree => Some((4, 3)),
            CropAspect::SixteenNine => Some((16, 9)),
        }
    }
}

/// Normalized sub-rectangle in permille (`0..=1000`) of the post-rotation image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropRect {
    pub x_permille: u16,
    pub y_permille: u16,
    pub w_permille: u16,
    pub h_permille: u16,
}

impl Default for CropRect {
    fn default() -> Self {
        Self::FULL
    }
}

impl CropRect {
    pub const FULL: Self = Self {
        x_permille: 0,
        y_permille: 0,
        w_permille: 1000,
        h_permille: 1000,
    };

    pub fn sanitized(self) -> Self {
        let w = self.w_permille.clamp(50, 1000);
        let h = self.h_permille.clamp(50, 1000);
        let x = self.x_permille.min(1000 - w);
        let y = self.y_permille.min(1000 - h);
        Self {
            x_permille: x,
            y_permille: y,
            w_permille: w,
            h_permille: h,
        }
    }

    pub fn is_full(self) -> bool {
        let s = self.sanitized();
        s.x_permille == 0 && s.y_permille == 0 && s.w_permille == 1000 && s.h_permille == 1000
    }

    /// Computes a centered permille crop matching `aspect` on an `img_w x img_h` frame,
    /// with an optional symmetric zoom inset (`0..=400` permille).
    pub fn from_aspect(aspect: CropAspect, img_w: usize, img_h: usize, inset_permille: u16) -> Self {
        let img_w = img_w.max(1) as u64;
        let img_h = img_h.max(1) as u64;
        let inset = inset_permille.min(400) as u64;
        let scale_num = 1000u64.saturating_sub(inset * 2).max(100);

        let (base_w, base_h) = match aspect.ratio() {
            None => (1000u64, 1000u64),
            Some((rw, rh)) => {
                let rw = rw as u64;
                let rh = rh as u64;
                // Target pixel aspect = rw / rh. Current = img_w / img_h.
                if img_w * rh >= img_h * rw {
                    // Image is wider than target aspect: full height, narrower width
                    let w_pm = (img_h * rw * 1000) / (img_w * rh);
                    (w_pm.clamp(50, 1000), 1000u64)
                } else {
                    // Image is taller than target aspect: full width, shorter height
                    let h_pm = (img_w * rh * 1000) / (img_h * rw);
                    (1000u64, h_pm.clamp(50, 1000))
                }
            }
        };

        let w = ((base_w * scale_num) / 1000).clamp(50, 1000) as u16;
        let h = ((base_h * scale_num) / 1000).clamp(50, 1000) as u16;
        let x = (1000 - w) / 2;
        let y = (1000 - h) / 2;
        Self {
            x_permille: x,
            y_permille: y,
            w_permille: w,
            h_permille: h,
        }
        .sanitized()
    }

    /// Symmetrically zooms the crop inward by `step_permille` (e.g. `80` = 8% tighter),
    /// preserving the current center and relative proportions.
    pub fn zoom_in(self, step_permille: u16) -> Self {
        let s = self.sanitized();
        let factor = 1000u32.saturating_sub(step_permille.min(300) as u32);
        let nw = ((s.w_permille as u32 * factor) / 1000).clamp(100, 1000) as u16;
        let nh = ((s.h_permille as u32 * factor) / 1000).clamp(100, 1000) as u16;
        let cx = s.x_permille as u32 + s.w_permille as u32 / 2;
        let cy = s.y_permille as u32 + s.h_permille as u32 / 2;
        let nx = cx.saturating_sub(nw as u32 / 2).min(1000 - nw as u32) as u16;
        let ny = cy.saturating_sub(nh as u32 / 2).min(1000 - nh as u32) as u16;
        Self {
            x_permille: nx,
            y_permille: ny,
            w_permille: nw,
            h_permille: nh,
        }
        .sanitized()
    }

    /// Converts permille coordinates into concrete pixel bounds `(x, y, w, h)` for `width x height`.
    pub fn pixel_bounds(self, width: usize, height: usize) -> (usize, usize, usize, usize) {
        if width == 0 || height == 0 {
            return (0, 0, 0, 0);
        }
        let s = self.sanitized();
        if s.is_full() {
            return (0, 0, width, height);
        }
        let x = ((width as u64 * s.x_permille as u64) / 1000) as usize;
        let y = ((height as u64 * s.y_permille as u64) / 1000) as usize;
        let x = x.min(width - 1);
        let y = y.min(height - 1);
        let w = (((width as u64 * s.w_permille as u64 + 500) / 1000) as usize)
            .max(1)
            .min(width - x);
        let h = (((height as u64 * s.h_permille as u64 + 500) / 1000) as usize)
            .max(1)
            .min(height - y);
        (x, y, w, h)
    }
}

/// Curated photographic filter presets directly backed by `batik_core` filters and tone curves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SharedFilterPreset {
    #[default]
    None,
    WarmVintage,
    CoolCinema,
    SCurveContrast,
    LiftShadows,
    Sepia,
    Sharpen,
    SoftGlowBlur,
    NoirMono,
    VividPop,
    Posterize,
    Emboss,
}

impl SharedFilterPreset {
    pub const ALL: [SharedFilterPreset; 12] = [
        SharedFilterPreset::None,
        SharedFilterPreset::WarmVintage,
        SharedFilterPreset::CoolCinema,
        SharedFilterPreset::SCurveContrast,
        SharedFilterPreset::LiftShadows,
        SharedFilterPreset::Sepia,
        SharedFilterPreset::Sharpen,
        SharedFilterPreset::SoftGlowBlur,
        SharedFilterPreset::NoirMono,
        SharedFilterPreset::VividPop,
        SharedFilterPreset::Posterize,
        SharedFilterPreset::Emboss,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SharedFilterPreset::None => "Original",
            SharedFilterPreset::WarmVintage => "Warm Vintage",
            SharedFilterPreset::CoolCinema => "Cool Cinema",
            SharedFilterPreset::SCurveContrast => "S-Curve",
            SharedFilterPreset::LiftShadows => "Lift Shadows",
            SharedFilterPreset::Sepia => "Sepia",
            SharedFilterPreset::Sharpen => "Sharpen",
            SharedFilterPreset::SoftGlowBlur => "Soft Blur",
            SharedFilterPreset::NoirMono => "Noir Mono",
            SharedFilterPreset::VividPop => "Vivid Pop",
            SharedFilterPreset::Posterize => "Posterize",
            SharedFilterPreset::Emboss => "Emboss",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            SharedFilterPreset::None => "none",
            SharedFilterPreset::WarmVintage => "warm_vintage",
            SharedFilterPreset::CoolCinema => "cool_cinema",
            SharedFilterPreset::SCurveContrast => "s_curve",
            SharedFilterPreset::LiftShadows => "lift_shadows",
            SharedFilterPreset::Sepia => "sepia",
            SharedFilterPreset::Sharpen => "sharpen",
            SharedFilterPreset::SoftGlowBlur => "soft_blur",
            SharedFilterPreset::NoirMono => "noir_mono",
            SharedFilterPreset::VividPop => "vivid_pop",
            SharedFilterPreset::Posterize => "posterize",
            SharedFilterPreset::Emboss => "emboss",
        }
    }

    pub fn from_slug(s: &str) -> Self {
        match s {
            "warm_vintage" => SharedFilterPreset::WarmVintage,
            "cool_cinema" => SharedFilterPreset::CoolCinema,
            "s_curve" => SharedFilterPreset::SCurveContrast,
            "lift_shadows" => SharedFilterPreset::LiftShadows,
            "sepia" => SharedFilterPreset::Sepia,
            "sharpen" => SharedFilterPreset::Sharpen,
            "soft_blur" => SharedFilterPreset::SoftGlowBlur,
            "noir_mono" => SharedFilterPreset::NoirMono,
            "vivid_pop" => SharedFilterPreset::VividPop,
            "posterize" => SharedFilterPreset::Posterize,
            "emboss" => SharedFilterPreset::Emboss,
            _ => SharedFilterPreset::None,
        }
    }

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// Converts this preset into the shared `batik_core::filters::FilterSpec`.
    pub fn to_filter_spec(self) -> Option<FilterSpec> {
        match self {
            SharedFilterPreset::None => None,
            SharedFilterPreset::WarmVintage => {
                Some(FilterSpec::Curves(CurvesPreset::WarmVintage.to_curves()))
            }
            SharedFilterPreset::CoolCinema => {
                Some(FilterSpec::Curves(CurvesPreset::CoolCinema.to_curves()))
            }
            SharedFilterPreset::SCurveContrast => {
                Some(FilterSpec::Curves(CurvesPreset::SCurveContrast.to_curves()))
            }
            SharedFilterPreset::LiftShadows => {
                Some(FilterSpec::Curves(CurvesPreset::LiftShadows.to_curves()))
            }
            SharedFilterPreset::Sepia => Some(FilterSpec::Sepia { intensity_pct: 85 }),
            SharedFilterPreset::Sharpen => Some(FilterSpec::Sharpen { amount_pct: 75 }),
            SharedFilterPreset::SoftGlowBlur => Some(FilterSpec::GaussianBlur { radius: 2 }),
            SharedFilterPreset::NoirMono => Some(FilterSpec::HueSaturation {
                hue_deg: 0,
                saturation_pct: -100,
                lightness_pct: 4,
            }),
            SharedFilterPreset::VividPop => Some(FilterSpec::HueSaturation {
                hue_deg: 0,
                saturation_pct: 45,
                lightness_pct: 2,
            }),
            SharedFilterPreset::Posterize => Some(FilterSpec::Posterize { levels: 5 }),
            SharedFilterPreset::Emboss => Some(FilterSpec::Emboss),
        }
    }

    pub fn to_filter_node(self) -> Option<FilterNode> {
        self.to_filter_spec().map(FilterNode::new)
    }
}

/// Complete non-destructive edit recipe persisted per photo in `library.gaze`.
/// Source image files on disk are never overwritten.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditRecipe {
    pub crop: CropRect,
    pub aspect: CropAspect,
    pub quarter_turns: u8,
    pub flip_h: bool,
    /// Exposure offset in hundredths of an EV stop (`-200..=+200` => `-2.00 EV .. +2.00 EV`).
    pub exposure_ev_x100: i16,
    /// Contrast percentage (`-100..=+100`).
    pub contrast_pct: i16,
    /// White-balance warmth shift (`-100..=+100`, blue <-> amber).
    pub warmth_pct: i16,
    /// Saturation shift (`-100..=+100`), evaluated via `batik_core::adjust::HueSatAdjustment`.
    pub saturation_pct: i16,
    /// Primary shared Batik filter preset.
    pub preset: SharedFilterPreset,
    /// Additional stacked `batik_core::filters::FilterNode` operations.
    pub filter_stack: FilterStack,
}

impl EditRecipe {
    pub fn is_identity(&self) -> bool {
        self.crop.is_full()
            && self.quarter_turns % 4 == 0
            && !self.flip_h
            && self.exposure_ev_x100 == 0
            && self.contrast_pct == 0
            && self.warmth_pct == 0
            && self.saturation_pct == 0
            && self.preset == SharedFilterPreset::None
            && self.filter_stack.iter().all(|n| !n.enabled)
    }

    /// Assembles the full shared `batik_core::filters::FilterStack` (`preset` + `filter_stack`).
    pub fn effective_batik_stack(&self) -> FilterStack {
        let mut stack = Vec::with_capacity(1 + self.filter_stack.len());
        if let Some(node) = self.preset.to_filter_node() {
            stack.push(node);
        }
        for node in &self.filter_stack {
            if node.enabled {
                stack.push(node.clone());
            }
        }
        stack
    }

    pub fn summary(&self) -> String {
        if self.is_identity() {
            return "Original (Unedited)".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        if !self.crop.is_full() {
            if self.aspect != CropAspect::Free {
                parts.push(format!("Crop {}", self.aspect.label()));
            } else {
                parts.push(format!(
                    "Crop {}%x{}%",
                    self.crop.w_permille / 10,
                    self.crop.h_permille / 10
                ));
            }
        }
        if self.quarter_turns % 4 != 0 {
            parts.push(format!("{}°", (self.quarter_turns % 4) as u16 * 90));
        }
        if self.flip_h {
            parts.push("FlipH".to_string());
        }
        if self.exposure_ev_x100 != 0 {
            let sign = if self.exposure_ev_x100 > 0 { "+" } else { "-" };
            let abs = self.exposure_ev_x100.unsigned_abs();
            parts.push(format!("{}{}.{:02} EV", sign, abs / 100, abs % 100));
        }
        if self.contrast_pct != 0 {
            parts.push(format!("Con {:+}%", self.contrast_pct));
        }
        if self.warmth_pct != 0 {
            parts.push(format!("Warm {:+}%", self.warmth_pct));
        }
        if self.saturation_pct != 0 {
            parts.push(format!("Sat {:+}%", self.saturation_pct));
        }
        if self.preset != SharedFilterPreset::None {
            parts.push(self.preset.label().to_string());
        }
        let extra_enabled = self.filter_stack.iter().filter(|n| n.enabled).count();
        if extra_enabled > 0 {
            parts.push(format!("+{} Batik FX", extra_enabled));
        }
        parts.join(" | ")
    }

    pub fn encode_compact(&self) -> String {
        let s = self.crop.sanitized();
        let mut fx_parts = Vec::new();
        for node in &self.filter_stack {
            if let Some(tok) = encode_filter_node(node) {
                fx_parts.push(tok);
            }
        }
        let fx_joined = if fx_parts.is_empty() {
            "-".to_string()
        } else {
            fx_parts.join(",")
        };
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            s.x_permille,
            s.y_permille,
            s.w_permille,
            s.h_permille,
            self.aspect.slug(),
            self.quarter_turns % 4,
            self.flip_h as u8,
            self.exposure_ev_x100.clamp(-200, 200),
            self.contrast_pct.clamp(-100, 100),
            self.warmth_pct.clamp(-100, 100),
            self.saturation_pct.clamp(-100, 100),
            format!("{}~{}", self.preset.slug(), fx_joined)
        )
    }

    pub fn decode_compact(text: &str) -> Self {
        let cols: Vec<&str> = text.split(':').collect();
        if cols.len() < 12 {
            return Self::default();
        }
        let x = cols[0].parse::<u16>().unwrap_or(0);
        let y = cols[1].parse::<u16>().unwrap_or(0);
        let w = cols[2].parse::<u16>().unwrap_or(1000);
        let h = cols[3].parse::<u16>().unwrap_or(1000);
        let aspect = CropAspect::from_slug(cols[4]);
        let quarter_turns = cols[5].parse::<u8>().unwrap_or(0) % 4;
        let flip_h = cols[6] == "1";
        let exposure_ev_x100 = cols[7].parse::<i16>().unwrap_or(0).clamp(-200, 200);
        let contrast_pct = cols[8].parse::<i16>().unwrap_or(0).clamp(-100, 100);
        let warmth_pct = cols[9].parse::<i16>().unwrap_or(0).clamp(-100, 100);
        let saturation_pct = cols[10].parse::<i16>().unwrap_or(0).clamp(-100, 100);
        let tail = cols[11..].join(":");
        let (preset_slug, fx_str) = tail.split_once('~').unwrap_or((&tail, "-"));
        let preset = SharedFilterPreset::from_slug(preset_slug);
        let mut filter_stack = Vec::new();
        if fx_str != "-" && !fx_str.is_empty() {
            for item in fx_str.split(',').take(16) {
                if let Some(node) = decode_filter_node(item) {
                    filter_stack.push(node);
                }
            }
        }
        Self {
            crop: CropRect {
                x_permille: x,
                y_permille: y,
                w_permille: w,
                h_permille: h,
            }
            .sanitized(),
            aspect,
            quarter_turns,
            flip_h,
            exposure_ev_x100,
            contrast_pct,
            warmth_pct,
            saturation_pct,
            preset,
            filter_stack,
        }
    }
}

fn encode_filter_node(node: &FilterNode) -> Option<String> {
    let en = if node.enabled { '1' } else { '0' };
    let body = match &node.spec {
        FilterSpec::BoxBlur { radius } => format!("boxblur.{}", radius),
        FilterSpec::GaussianBlur { radius } => format!("gauss.{}", radius),
        FilterSpec::Sharpen { amount_pct } => format!("sharpen.{}", amount_pct),
        FilterSpec::Emboss => "emboss".to_string(),
        FilterSpec::EdgeDetect => "edge".to_string(),
        FilterSpec::Posterize { levels } => format!("poster.{}", levels),
        FilterSpec::Sepia { intensity_pct } => format!("sepia.{}", intensity_pct),
        FilterSpec::Invert => "invert".to_string(),
        FilterSpec::HueSaturation {
            hue_deg,
            saturation_pct,
            lightness_pct,
        } => format!("huesat.{}.{}.{}", hue_deg, saturation_pct, lightness_pct),
        FilterSpec::Curves(c) => {
            if *c == CurvesPreset::WarmVintage.to_curves() {
                "curves.warm".to_string()
            } else if *c == CurvesPreset::CoolCinema.to_curves() {
                "curves.cool".to_string()
            } else if *c == CurvesPreset::SCurveContrast.to_curves() {
                "curves.scurve".to_string()
            } else if *c == CurvesPreset::LiftShadows.to_curves() {
                "curves.lift".to_string()
            } else {
                return None;
            }
        }
    };
    Some(format!("{}.{}", en, body))
}

fn decode_filter_node(s: &str) -> Option<FilterNode> {
    let (en_s, rest) = s.split_once('.')?;
    let enabled = en_s == "1";
    let parts: Vec<&str> = rest.split('.').collect();
    let spec = match parts.first().copied()? {
        "boxblur" => FilterSpec::BoxBlur {
            radius: parts.get(1)?.parse().ok()?,
        },
        "gauss" => FilterSpec::GaussianBlur {
            radius: parts.get(1)?.parse().ok()?,
        },
        "sharpen" => FilterSpec::Sharpen {
            amount_pct: parts.get(1)?.parse().ok()?,
        },
        "emboss" => FilterSpec::Emboss,
        "edge" => FilterSpec::EdgeDetect,
        "poster" => FilterSpec::Posterize {
            levels: parts.get(1)?.parse().ok()?,
        },
        "sepia" => FilterSpec::Sepia {
            intensity_pct: parts.get(1)?.parse().ok()?,
        },
        "invert" => FilterSpec::Invert,
        "huesat" => FilterSpec::HueSaturation {
            hue_deg: parts.get(1)?.parse().ok()?,
            saturation_pct: parts.get(2)?.parse().ok()?,
            lightness_pct: parts.get(3)?.parse().ok()?,
        },
        "curves" => match *parts.get(1)? {
            "warm" => FilterSpec::Curves(CurvesPreset::WarmVintage.to_curves()),
            "cool" => FilterSpec::Curves(CurvesPreset::CoolCinema.to_curves()),
            "scurve" => FilterSpec::Curves(CurvesPreset::SCurveContrast.to_curves()),
            "lift" => FilterSpec::Curves(CurvesPreset::LiftShadows.to_curves()),
            _ => return None,
        },
        _ => return None,
    };
    Some(FilterNode { enabled, spec })
}

fn srgb_to_linear(u: u8) -> f32 {
    let c = u as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        libm::powf((c + 0.055) / 1.055, 2.4)
    }
}

fn linear_to_srgb(l: f32) -> f32 {
    let c = l.max(0.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * libm::powf(c, 1.0 / 2.4) - 0.055
    }
}

/// Precomputes 256-entry `(R, G, B)` tone LUTs combining linear-light EV exposure
/// (`2^ev` with smooth highlight shoulder rolloff), white-balance warmth, and sRGB contrast.
pub fn build_tone_luts(
    exposure_ev_x100: i16,
    contrast_pct: i16,
    warmth_pct: i16,
) -> ([u8; 256], [u8; 256], [u8; 256]) {
    let mut lut_r = [0u8; 256];
    let mut lut_g = [0u8; 256];
    let mut lut_b = [0u8; 256];

    if exposure_ev_x100 == 0 && contrast_pct == 0 && warmth_pct == 0 {
        for i in 0..=255 {
            lut_r[i] = i as u8;
            lut_g[i] = i as u8;
            lut_b[i] = i as u8;
        }
        return (lut_r, lut_g, lut_b);
    }

    let ev = (exposure_ev_x100.clamp(-200, 200) as f32) / 100.0;
    let base_gain = libm::powf(2.0, ev);
    let warm = (warmth_pct.clamp(-100, 100) as f32) / 100.0;
    let gain_r = base_gain * (1.0 + warm * 0.16);
    let gain_g = base_gain * (1.0 + warm * 0.02);
    let gain_b = base_gain * (1.0 - warm * 0.16);

    let contrast = (contrast_pct.clamp(-100, 100) as f32) / 100.0;
    let contrast_factor = if contrast >= 0.0 {
        1.0 + contrast * 1.15
    } else {
        (1.0 + contrast * 0.75).max(0.15)
    };

    let map_channel = |u: u8, gain: f32| -> u8 {
        let lin = srgb_to_linear(u) * gain;
        // Smooth filmic highlight shoulder above 0.80 linear so +EV keeps highlight detail
        let rolled = if lin > 0.80 {
            let excess = lin - 0.80;
            0.80 + 0.20 * (excess / (excess + 0.20))
        } else {
            lin
        };
        let srgb = linear_to_srgb(rolled.clamp(0.0, 1.0));
        let contrasted = (srgb - 0.5) * contrast_factor + 0.5;
        libm::roundf(contrasted.clamp(0.0, 1.0) * 255.0) as u8
    };

    for i in 0..=255 {
        let u = i as u8;
        lut_r[i] = map_channel(u, gain_r);
        lut_g[i] = map_channel(u, gain_g);
        lut_b[i] = map_channel(u, gain_b);
    }

    (lut_r, lut_g, lut_b)
}

/// Applies `recipe` non-destructively to `src`, returning a newly allocated `Image` while
/// leaving `src` untouched.
pub fn apply_edits(src: &Image, recipe: &EditRecipe) -> Result<Image, gaze_codecs::Error> {
    if src.width == 0 || src.height == 0 || src.pixels.len() < src.width * src.height {
        return Err(gaze_codecs::Error::TooLarge);
    }

    if recipe.is_identity() {
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(src.pixels.len())
            .map_err(|_| gaze_codecs::Error::OutOfMemory)?;
        pixels.extend_from_slice(&src.pixels);
        return Ok(Image {
            width: src.width,
            height: src.height,
            pixels,
        });
    }

    // 1. Quarter-turn clockwise rotation + optional horizontal mirror
    let turns = recipe.quarter_turns % 4;
    let (rot_w, rot_h, mut work) = if turns != 0 {
        let mut dst = Vec::new();
        dst.try_reserve_exact(src.width * src.height)
            .map_err(|_| gaze_codecs::Error::OutOfMemory)?;
        dst.resize(src.width * src.height, 0);
        let (nw, nh) = sex_image::view::rotate_into(&src.pixels, (src.width, src.height), turns, &mut dst);
        (nw, nh, dst)
    } else {
        let mut dst = Vec::new();
        dst.try_reserve_exact(src.width * src.height)
            .map_err(|_| gaze_codecs::Error::OutOfMemory)?;
        dst.extend_from_slice(&src.pixels[..src.width * src.height]);
        (src.width, src.height, dst)
    };

    if recipe.flip_h {
        for y in 0..rot_h {
            let row = &mut work[y * rot_w..(y + 1) * rot_w];
            row.reverse();
        }
    }

    // 2. Non-destructive crop
    let (cx, cy, cw, ch) = recipe.crop.pixel_bounds(rot_w, rot_h);
    let mut pixels = if cx == 0 && cy == 0 && cw == rot_w && ch == rot_h {
        work
    } else {
        let mut sub = Vec::new();
        sub.try_reserve_exact(cw * ch)
            .map_err(|_| gaze_codecs::Error::OutOfMemory)?;
        for y in 0..ch {
            let src_row = &work[(cy + y) * rot_w + cx..(cy + y) * rot_w + cx + cw];
            sub.extend_from_slice(src_row);
        }
        sub
    };

    // 3. Exposure + Contrast + Warmth LUT pass
    if recipe.exposure_ev_x100 != 0 || recipe.contrast_pct != 0 || recipe.warmth_pct != 0 {
        let (lr, lg, lb) = build_tone_luts(
            recipe.exposure_ev_x100,
            recipe.contrast_pct,
            recipe.warmth_pct,
        );
        for p in pixels.iter_mut() {
            let (a, r, g, b) = split_argb(*p);
            if a != 0 {
                *p = pack_argb(a, lr[r as usize], lg[g as usize], lb[b as usize]);
            }
        }
    }

    // 4. Saturation adjustment via shared batik_core::adjust::HueSatAdjustment
    if recipe.saturation_pct != 0 {
        let hs = HueSatAdjustment::new(0, recipe.saturation_pct, 0);
        for p in pixels.iter_mut() {
            *p = hs.apply_pixel(*p);
        }
    }

    // 5. Shared batik_core::filters::apply_filter_stack (preset + extra stacked nodes)
    let stack = recipe.effective_batik_stack();
    if !stack.is_empty() {
        apply_filter_stack(&stack, &mut pixels, cw, ch);
    }

    Ok(Image {
        width: cw,
        height: ch,
        pixels,
    })
}
