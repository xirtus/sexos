// SPDX-License-Identifier: GPL-3.0-only
//! Folder-native `.gaze.toml` non-destructive sidecars, culling metadata
//! (`stars`, `flag`, `color_label`, `keywords`), full `DevelopRecipe` persistence,
//! and `SmartAlbumRule` evaluator.
//!
//! Original image and camera RAW files stay 100% untouched on disk.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::develop::{
    film_stock_from_slug, film_stock_slug, BrushDab, BrushMask, ColorRangeMask, Curve1D,
    DevelopRecipe, FilmStock, HealPatch, InterpMode, LinearGradientMask, LocalMask, LocalMaskKind,
    LuminanceRangeMask, Lut3D, MaskAdjustments, RadialMask,
};
use crate::edits::{
    decode_filter_node, encode_filter_node, CropAspect, CropRect, EditRecipe, SharedFilterPreset,
};
use crate::library::PhotoRecord;
use crate::metadata::{PhotoDate, PhotoMeta};

const SIDECAR_VERSION: u32 = 1;
const MAX_KEYWORDS: usize = 128;
const MAX_MASKS: usize = 64;
const MAX_HEALS: usize = 128;
const MAX_DABS: usize = 2048;

/// Culling pick/reject triage flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CullFlag {
    #[default]
    Unflagged,
    Pick,
    Reject,
}

impl CullFlag {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Unflagged => "unflagged",
            Self::Pick => "pick",
            Self::Reject => "reject",
        }
    }

    pub fn from_slug(s: &str) -> Self {
        let t = s.trim();
        if t.eq_ignore_ascii_case("pick") || t.eq_ignore_ascii_case("picked") || t == "P" || t == "p" {
            Self::Pick
        } else if t.eq_ignore_ascii_case("reject")
            || t.eq_ignore_ascii_case("rejected")
            || t == "X"
            || t == "x"
        {
            Self::Reject
        } else {
            Self::Unflagged
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Unflagged => "Unflagged",
            Self::Pick => "Pick",
            Self::Reject => "Reject",
        }
    }
}

/// Standard photographic color triage label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorLabel {
    #[default]
    None,
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl ColorLabel {
    pub const ALL: [Self; 6] = [
        Self::None,
        Self::Red,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Purple => "purple",
        }
    }

    pub fn from_slug(s: &str) -> Self {
        let t = s.trim();
        if t.eq_ignore_ascii_case("red") {
            Self::Red
        } else if t.eq_ignore_ascii_case("yellow") {
            Self::Yellow
        } else if t.eq_ignore_ascii_case("green") {
            Self::Green
        } else if t.eq_ignore_ascii_case("blue") {
            Self::Blue
        } else if t.eq_ignore_ascii_case("purple") || t.eq_ignore_ascii_case("magenta") {
            Self::Purple
        } else {
            Self::None
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Red => "Red",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::None => Self::Red,
            Self::Red => Self::Yellow,
            Self::Yellow => Self::Green,
            Self::Green => Self::Blue,
            Self::Blue => Self::Purple,
            Self::Purple => Self::None,
        }
    }

    pub const fn argb_u32(self) -> u32 {
        match self {
            Self::None => 0xFF64_748B,
            Self::Red => 0xFFEF_4444,
            Self::Yellow => 0xFFF5_9E0B,
            Self::Green => 0xFF10_B981,
            Self::Blue => 0xFF3B_82F6,
            Self::Purple => 0xFFA8_55F7,
        }
    }
}

/// Errors returned by [`PhotoSidecar::from_toml`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidecarError {
    EmptyInput,
    ParseError,
}

/// Computes the deterministic folder-native sidecar path beside `photo_path`
/// (e.g. `"photos/IMG_001.DNG"` -> `"photos/IMG_001.DNG.gaze.toml"`).
pub fn sidecar_path_for(photo_path: &str) -> String {
    format!("{}.gaze.toml", photo_path)
}

/// Returns `true` if `name` is a `.gaze.toml` sidecar file.
pub fn is_sidecar_filename(name: &str) -> bool {
    name.ends_with(".gaze.toml")
}

/// Non-destructive folder-native `.gaze.toml` sidecar combining culling metadata,
/// geometry/filter [`EditRecipe`], and full OmaRAW [`DevelopRecipe`].
#[derive(Clone, Debug, PartialEq)]
pub struct PhotoSidecar {
    pub version: u32,
    pub source_name: String,
    /// Star rating (`0..=5`).
    pub stars: u8,
    pub flag: CullFlag,
    pub color_label: ColorLabel,
    pub keywords: Vec<String>,
    pub edit: EditRecipe,
    pub develop: DevelopRecipe,
}

impl Default for PhotoSidecar {
    fn default() -> Self {
        Self {
            version: SIDECAR_VERSION,
            source_name: String::new(),
            stars: 0,
            flag: CullFlag::Unflagged,
            color_label: ColorLabel::None,
            keywords: Vec::new(),
            edit: EditRecipe::default(),
            develop: DevelopRecipe::default(),
        }
    }
}

impl PhotoSidecar {
    pub fn new(source_name: &str) -> Self {
        Self {
            source_name: source_name.to_string(),
            ..Self::default()
        }
    }

    pub fn set_stars(&mut self, stars: u8) {
        self.stars = stars.min(5);
    }

    pub fn add_keyword(&mut self, raw: &str) {
        let kw = raw.trim();
        if kw.is_empty() || self.keywords.len() >= MAX_KEYWORDS {
            return;
        }
        if !self.keywords.iter().any(|k| k.eq_ignore_ascii_case(kw)) {
            self.keywords.push(kw.to_string());
        }
    }

    pub fn remove_keyword(&mut self, raw: &str) {
        let kw = raw.trim();
        self.keywords.retain(|k| !k.eq_ignore_ascii_case(kw));
    }

    pub fn is_keeper(&self) -> bool {
        self.flag != CullFlag::Reject && (self.flag == CullFlag::Pick || self.stars >= 1)
    }

    pub fn is_identity(&self) -> bool {
        self.stars == 0
            && self.flag == CullFlag::Unflagged
            && self.color_label == ColorLabel::None
            && self.keywords.is_empty()
            && self.edit.is_identity()
            && self.develop.is_identity()
    }

    /// Serializes this sidecar into a deterministic, human-readable `.gaze.toml` document.
    pub fn to_toml(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("# Gaze Non-Destructive Sidecar (.gaze.toml)\n");
        out.push_str(&format!("version = {}\n", self.version.max(1)));
        out.push_str(&format!("source = {}\n", toml_quote(&self.source_name)));

        out.push_str("\n[cull]\n");
        out.push_str(&format!("stars = {}\n", self.stars.min(5)));
        out.push_str(&format!("flag = {}\n", toml_quote(self.flag.slug())));
        out.push_str(&format!("color_label = {}\n", toml_quote(self.color_label.slug())));
        out.push_str("keywords = [");
        for (i, kw) in self.keywords.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&toml_quote(kw));
        }
        out.push_str("]\n");

        out.push_str("\n[edit]\n");
        let crop = self.edit.crop.sanitized();
        out.push_str(&format!(
            "crop = [{}, {}, {}, {}]\n",
            crop.x_permille, crop.y_permille, crop.w_permille, crop.h_permille
        ));
        out.push_str(&format!("aspect = {}\n", toml_quote(self.edit.aspect.slug())));
        out.push_str(&format!("quarter_turns = {}\n", self.edit.quarter_turns % 4));
        out.push_str(&format!("flip_h = {}\n", self.edit.flip_h));
        out.push_str(&format!("exposure_ev_x100 = {}\n", self.edit.exposure_ev_x100));
        out.push_str(&format!("contrast_pct = {}\n", self.edit.contrast_pct));
        out.push_str(&format!("warmth_pct = {}\n", self.edit.warmth_pct));
        out.push_str(&format!("saturation_pct = {}\n", self.edit.saturation_pct));
        out.push_str(&format!("preset = {}\n", toml_quote(self.edit.preset.slug())));
        out.push_str("filter_stack = [");
        let mut first_fx = true;
        for node in &self.edit.filter_stack {
            if let Some(tok) = encode_filter_node(node) {
                if !first_fx {
                    out.push_str(", ");
                }
                first_fx = false;
                out.push_str(&toml_quote(&tok));
            }
        }
        out.push_str("]\n");

        let d = &self.develop;
        let g = &d.grade;
        out.push_str("\n[develop]\n");
        out.push_str(&format!("exposure_ev = {:.6}\n", g.exposure_ev));
        out.push_str(&format!("temperature = {:.6}\n", g.temperature));
        out.push_str(&format!("tint = {:.6}\n", g.tint));
        out.push_str(&format!("contrast = {:.6}\n", g.contrast));
        out.push_str(&format!("pivot = {:.6}\n", g.pivot));
        out.push_str(&format!("saturation = {:.6}\n", g.saturation));
        out.push_str(&format!("vibrance = {:.6}\n", g.vibrance));
        out.push_str(&format!("highlights = {:.6}\n", d.highlights));
        out.push_str(&format!("shadows = {:.6}\n", d.shadows));
        out.push_str(&format!("whites = {:.6}\n", d.whites));
        out.push_str(&format!("blacks = {:.6}\n", d.blacks));
        out.push_str(&format!("sharpness = {:.6}\n", d.sharpness));
        out.push_str(&format!("vignette = {:.6}\n", d.vignette));
        out.push_str(&format!(
            "lift = [{:.6}, {:.6}, {:.6}]\n",
            g.lift[0], g.lift[1], g.lift[2]
        ));
        out.push_str(&format!(
            "gamma = [{:.6}, {:.6}, {:.6}]\n",
            g.gamma[0], g.gamma[1], g.gamma[2]
        ));
        out.push_str(&format!(
            "gain = [{:.6}, {:.6}, {:.6}]\n",
            g.gain[0], g.gain[1], g.gain[2]
        ));
        out.push_str(&format!(
            "offset = [{:.6}, {:.6}, {:.6}]\n",
            g.offset[0], g.offset[1], g.offset[2]
        ));
        let stock_slug = d.film_stock.map(film_stock_slug).unwrap_or("none");
        out.push_str(&format!("film_stock = {}\n", toml_quote(stock_slug)));
        out.push_str(&format!("film_strength = {:.6}\n", d.film_strength));
        if let Some(ref l_name) = d.lut_name {
            out.push_str(&format!("lut_name = {}\n", toml_quote(l_name)));
        }
        out.push_str(&format!("lut_strength = {:.6}\n", d.lut_strength));
        let interp_s = match d.lut_interp {
            InterpMode::Trilinear => "trilinear",
            InterpMode::Tetrahedral => "tetrahedral",
        };
        out.push_str(&format!("lut_interp = {}\n", toml_quote(interp_s)));
        out.push_str(&format!("lut_in_srgb = {}\n", d.lut_in_srgb));
        if let Some(ref lut) = d.lut {
            out.push_str("lut_cube = \"\"\"\n");
            out.push_str(&lut.to_cube());
            if !lut.to_cube().ends_with('\n') {
                out.push('\n');
            }
            out.push_str("\"\"\"\n");
        }

        out.push_str("\n[develop.curves]\n");
        out.push_str(&format!("master = {}\n", format_curve(&g.master)));
        out.push_str(&format!("red = {}\n", format_curve(&g.r)));
        out.push_str(&format!("green = {}\n", format_curve(&g.g)));
        out.push_str(&format!("blue = {}\n", format_curve(&g.b)));
        out.push_str(&format!("hue_vs_sat = {}\n", format_curve(&g.hue_vs_sat)));

        out.push_str("\n[develop.denoise]\n");
        out.push_str(&format!("luma_strength = {:.6}\n", d.denoise.luma_strength));
        out.push_str(&format!("chroma_strength = {:.6}\n", d.denoise.chroma_strength));
        out.push_str(&format!("radius = {}\n", d.denoise.radius.clamp(1, 5)));
        out.push_str(&format!("edge_preserve = {:.6}\n", d.denoise.edge_preserve));

        for heal in &d.heal_spots {
            out.push_str("\n[[develop.heal]]\n");
            out.push_str(&format!("target_x = {:.6}\n", heal.target_x));
            out.push_str(&format!("target_y = {:.6}\n", heal.target_y));
            out.push_str(&format!("radius = {:.6}\n", heal.radius));
            if let (Some(sx), Some(sy)) = (heal.source_x, heal.source_y) {
                out.push_str(&format!("source_x = {:.6}\n", sx));
                out.push_str(&format!("source_y = {:.6}\n", sy));
            }
            out.push_str(&format!("feather = {:.6}\n", heal.feather));
            out.push_str(&format!("opacity = {:.6}\n", heal.opacity));
            out.push_str(&format!("enabled = {}\n", heal.enabled));
        }

        for mask in &d.masks {
            out.push_str("\n[[develop.mask]]\n");
            out.push_str(&format!("kind = {}\n", toml_quote(mask.kind.slug())));
            out.push_str(&format!("name = {}\n", toml_quote(&mask.name)));
            out.push_str(&format!("enabled = {}\n", mask.enabled));
            out.push_str(&format!("inverted = {}\n", mask.inverted));
            out.push_str(&format!("opacity = {:.6}\n", mask.opacity));

            match &mask.kind {
                LocalMaskKind::Brush(b) => {
                    out.push_str("dabs = [");
                    for (i, dab) in b.dabs.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(&format!(
                            "[{:.6}, {:.6}, {:.6}, {:.6}, {:.6}]",
                            dab.x, dab.y, dab.radius, dab.feather, dab.flow
                        ));
                    }
                    out.push_str("]\n");
                }
                LocalMaskKind::LinearGradient(lg) => {
                    out.push_str(&format!("x0 = {:.6}\n", lg.x0));
                    out.push_str(&format!("y0 = {:.6}\n", lg.y0));
                    out.push_str(&format!("x1 = {:.6}\n", lg.x1));
                    out.push_str(&format!("y1 = {:.6}\n", lg.y1));
                }
                LocalMaskKind::Radial(rm) => {
                    out.push_str(&format!("cx = {:.6}\n", rm.cx));
                    out.push_str(&format!("cy = {:.6}\n", rm.cy));
                    out.push_str(&format!("rx = {:.6}\n", rm.rx));
                    out.push_str(&format!("ry = {:.6}\n", rm.ry));
                    out.push_str(&format!("feather = {:.6}\n", rm.feather));
                    out.push_str(&format!("rotation_deg = {:.6}\n", rm.rotation_deg));
                }
                LocalMaskKind::LuminanceRange(lm) => {
                    out.push_str(&format!("min_luma = {:.6}\n", lm.min_luma));
                    out.push_str(&format!("max_luma = {:.6}\n", lm.max_luma));
                    out.push_str(&format!("feather = {:.6}\n", lm.feather));
                }
                LocalMaskKind::ColorRange(cm) => {
                    out.push_str(&format!(
                        "target_oklab = [{:.6}, {:.6}, {:.6}]\n",
                        cm.target_oklab[0], cm.target_oklab[1], cm.target_oklab[2]
                    ));
                    out.push_str(&format!("hue_tolerance = {:.6}\n", cm.hue_tolerance));
                    out.push_str(&format!("chroma_tolerance = {:.6}\n", cm.chroma_tolerance));
                    out.push_str(&format!("luma_tolerance = {:.6}\n", cm.luma_tolerance));
                    out.push_str(&format!("feather = {:.6}\n", cm.feather));
                }
            }

            let a = &mask.adjustments;
            out.push_str(&format!("exposure_ev = {:.6}\n", a.exposure_ev));
            out.push_str(&format!("contrast = {:.6}\n", a.contrast));
            out.push_str(&format!("highlights = {:.6}\n", a.highlights));
            out.push_str(&format!("shadows = {:.6}\n", a.shadows));
            out.push_str(&format!("temperature = {:.6}\n", a.temperature));
            out.push_str(&format!("tint = {:.6}\n", a.tint));
            out.push_str(&format!("saturation = {:.6}\n", a.saturation));
            out.push_str(&format!("sharpness = {:.6}\n", a.sharpness));
        }

        out
    }

    /// Parses a `.gaze.toml` sidecar document safely without panics.
    pub fn from_toml(src: &str) -> Result<Self, SidecarError> {
        if src.trim().is_empty() {
            return Err(SidecarError::EmptyInput);
        }
        let entries = parse_toml_entries(src)?;
        if entries.is_empty() {
            return Err(SidecarError::EmptyInput);
        }
        let mut sidecar = Self::default();
        let mut seen_known = false;

        let mut cur_heal: Option<HealPatch> = None;
        let mut cur_heal_idx: Option<usize> = None;
        let mut cur_mask: Option<MaskBuilder> = None;
        let mut cur_mask_idx: Option<usize> = None;

        for entry in entries {
            match entry.section.as_str() {
                "" => match entry.key.as_str() {
                    "version" => {
                        if let Some(v) = parse_u32(&entry.val) {
                            sidecar.version = v.max(1);
                            seen_known = true;
                        }
                    }
                    "source" => {
                        sidecar.source_name = unquote_toml(&entry.val)?;
                        seen_known = true;
                    }
                    _ => {}
                },
                "cull" => {
                    seen_known = true;
                    match entry.key.as_str() {
                        "stars" => {
                            if let Some(s) = parse_u32(&entry.val) {
                                sidecar.stars = (s as u8).min(5);
                            }
                        }
                        "flag" => {
                            let s = unquote_toml(&entry.val)?;
                            sidecar.flag = CullFlag::from_slug(&s);
                        }
                        "color_label" => {
                            let s = unquote_toml(&entry.val)?;
                            sidecar.color_label = ColorLabel::from_slug(&s);
                        }
                        "keywords" => {
                            let items = parse_string_array(&entry.val)?;
                            sidecar.keywords.clear();
                            for kw in items {
                                sidecar.add_keyword(&kw);
                            }
                        }
                        _ => {}
                    }
                }
                "edit" => {
                    seen_known = true;
                    match entry.key.as_str() {
                        "crop" => {
                            let nums = parse_f32_array(&entry.val)?;
                            if nums.len() == 4 {
                                sidecar.edit.crop = CropRect {
                                    x_permille: nums[0].clamp(0.0, 1000.0) as u16,
                                    y_permille: nums[1].clamp(0.0, 1000.0) as u16,
                                    w_permille: nums[2].clamp(0.0, 1000.0) as u16,
                                    h_permille: nums[3].clamp(0.0, 1000.0) as u16,
                                }
                                .sanitized();
                            }
                        }
                        "aspect" => {
                            let s = unquote_toml(&entry.val)?;
                            sidecar.edit.aspect = CropAspect::from_slug(&s);
                        }
                        "quarter_turns" => {
                            if let Some(q) = parse_u32(&entry.val) {
                                sidecar.edit.quarter_turns = (q % 4) as u8;
                            }
                        }
                        "flip_h" => {
                            if let Some(b) = parse_bool(&entry.val) {
                                sidecar.edit.flip_h = b;
                            }
                        }
                        "exposure_ev_x100" => {
                            if let Some(v) = parse_i32(&entry.val) {
                                sidecar.edit.exposure_ev_x100 = v.clamp(-200, 200) as i16;
                            }
                        }
                        "contrast_pct" => {
                            if let Some(v) = parse_i32(&entry.val) {
                                sidecar.edit.contrast_pct = v.clamp(-100, 100) as i16;
                            }
                        }
                        "warmth_pct" => {
                            if let Some(v) = parse_i32(&entry.val) {
                                sidecar.edit.warmth_pct = v.clamp(-100, 100) as i16;
                            }
                        }
                        "saturation_pct" => {
                            if let Some(v) = parse_i32(&entry.val) {
                                sidecar.edit.saturation_pct = v.clamp(-100, 100) as i16;
                            }
                        }
                        "preset" => {
                            let s = unquote_toml(&entry.val)?;
                            sidecar.edit.preset = SharedFilterPreset::from_slug(&s);
                        }
                        "filter_stack" => {
                            let items = parse_string_array(&entry.val)?;
                            sidecar.edit.filter_stack.clear();
                            for item in items.iter().take(16) {
                                if let Some(node) = decode_filter_node(item) {
                                    sidecar.edit.filter_stack.push(node);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                "develop" => {
                    seen_known = true;
                    let d = &mut sidecar.develop;
                    let g = &mut d.grade;
                    match entry.key.as_str() {
                        "exposure_ev" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                g.exposure_ev = v.clamp(-8.0, 8.0);
                            }
                        }
                        "temperature" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                g.temperature = v.clamp(-3.0, 3.0);
                            }
                        }
                        "tint" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                g.tint = v.clamp(-3.0, 3.0);
                            }
                        }
                        "contrast" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                g.contrast = v.clamp(0.05, 5.0);
                            }
                        }
                        "pivot" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                g.pivot = v.clamp(0.01, 0.99);
                            }
                        }
                        "saturation" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                g.saturation = v.clamp(0.0, 4.0);
                            }
                        }
                        "vibrance" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                g.vibrance = v.clamp(-1.0, 2.0);
                            }
                        }
                        "highlights" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                d.highlights = v.clamp(-1.0, 1.0);
                            }
                        }
                        "shadows" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                d.shadows = v.clamp(-1.0, 1.0);
                            }
                        }
                        "whites" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                d.whites = v.clamp(-1.0, 1.0);
                            }
                        }
                        "blacks" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                d.blacks = v.clamp(-1.0, 1.0);
                            }
                        }
                        "sharpness" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                d.sharpness = v.clamp(0.0, 1.0);
                            }
                        }
                        "vignette" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                d.vignette = v.clamp(-1.0, 1.0);
                            }
                        }
                        "lift" => {
                            let v = parse_f32_array(&entry.val)?;
                            if v.len() == 3 {
                                g.lift = [v[0], v[1], v[2]];
                            }
                        }
                        "gamma" => {
                            let v = parse_f32_array(&entry.val)?;
                            if v.len() == 3 {
                                g.gamma = [v[0].max(0.05), v[1].max(0.05), v[2].max(0.05)];
                            }
                        }
                        "gain" => {
                            let v = parse_f32_array(&entry.val)?;
                            if v.len() == 3 {
                                g.gain = [v[0].max(0.0), v[1].max(0.0), v[2].max(0.0)];
                            }
                        }
                        "offset" => {
                            let v = parse_f32_array(&entry.val)?;
                            if v.len() == 3 {
                                g.offset = [v[0], v[1], v[2]];
                            }
                        }
                        "film_stock" => {
                            let s = unquote_toml(&entry.val)?;
                            d.film_stock = film_stock_from_slug(&s);
                        }
                        "film_strength" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                d.film_strength = v.clamp(0.0, 1.0);
                            }
                        }
                        "lut_name" => {
                            let s = unquote_toml(&entry.val)?;
                            d.lut_name = (!s.is_empty()).then_some(s);
                        }
                        "lut_strength" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                d.lut_strength = v.clamp(0.0, 1.0);
                            }
                        }
                        "lut_interp" => {
                            let s = unquote_toml(&entry.val)?;
                            d.lut_interp = if s.eq_ignore_ascii_case("trilinear") {
                                InterpMode::Trilinear
                            } else {
                                InterpMode::Tetrahedral
                            };
                        }
                        "lut_in_srgb" => {
                            if let Some(b) = parse_bool(&entry.val) {
                                d.lut_in_srgb = b;
                            }
                        }
                        "lut_cube" => {
                            let cube_str = unquote_toml(&entry.val)?;
                            if let Ok(lut) = Lut3D::parse_cube(&cube_str) {
                                if d.lut_name.is_none() {
                                    d.lut_name = lut.title.clone();
                                }
                                d.lut = Some(lut);
                            }
                        }
                        _ => {}
                    }
                }
                "develop.curves" => {
                    seen_known = true;
                    let g = &mut sidecar.develop.grade;
                    match entry.key.as_str() {
                        "master" => {
                            let pts = parse_points_array(&entry.val)?;
                            if !pts.is_empty() {
                                g.master = Curve1D::from_points(&pts);
                            }
                        }
                        "red" => {
                            let pts = parse_points_array(&entry.val)?;
                            if !pts.is_empty() {
                                g.r = Curve1D::from_points(&pts);
                            }
                        }
                        "green" => {
                            let pts = parse_points_array(&entry.val)?;
                            if !pts.is_empty() {
                                g.g = Curve1D::from_points(&pts);
                            }
                        }
                        "blue" => {
                            let pts = parse_points_array(&entry.val)?;
                            if !pts.is_empty() {
                                g.b = Curve1D::from_points(&pts);
                            }
                        }
                        "hue_vs_sat" => {
                            let pts = parse_points_array(&entry.val)?;
                            if !pts.is_empty() {
                                g.hue_vs_sat = Curve1D::from_points(&pts);
                            }
                        }
                        _ => {}
                    }
                }
                "develop.denoise" => {
                    seen_known = true;
                    let dn = &mut sidecar.develop.denoise;
                    match entry.key.as_str() {
                        "luma_strength" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                dn.luma_strength = v.clamp(0.0, 1.0);
                            }
                        }
                        "chroma_strength" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                dn.chroma_strength = v.clamp(0.0, 1.0);
                            }
                        }
                        "radius" => {
                            if let Some(v) = parse_u32(&entry.val) {
                                dn.radius = (v as u8).clamp(1, 5);
                            }
                        }
                        "edge_preserve" => {
                            if let Some(v) = parse_f32(&entry.val) {
                                dn.edge_preserve = v.clamp(0.01, 1.0);
                            }
                        }
                        _ => {}
                    }
                }
                "develop.heal" => {
                    seen_known = true;
                    if cur_heal_idx != Some(entry.array_idx) {
                        if let Some(h) = cur_heal.take() {
                            if sidecar.develop.heal_spots.len() < MAX_HEALS {
                                sidecar.develop.heal_spots.push(h);
                            }
                        }
                        cur_heal = Some(HealPatch::auto(0.5, 0.5, 0.02));
                        cur_heal_idx = Some(entry.array_idx);
                    }
                    if let Some(ref mut h) = cur_heal {
                        match entry.key.as_str() {
                            "target_x" => {
                                if let Some(v) = parse_f32(&entry.val) {
                                    h.target_x = v.clamp(0.0, 1.0);
                                }
                            }
                            "target_y" => {
                                if let Some(v) = parse_f32(&entry.val) {
                                    h.target_y = v.clamp(0.0, 1.0);
                                }
                            }
                            "radius" => {
                                if let Some(v) = parse_f32(&entry.val) {
                                    h.radius = v.clamp(0.002, 0.45);
                                }
                            }
                            "source_x" => {
                                if let Some(v) = parse_f32(&entry.val) {
                                    h.source_x = Some(v.clamp(0.0, 1.0));
                                }
                            }
                            "source_y" => {
                                if let Some(v) = parse_f32(&entry.val) {
                                    h.source_y = Some(v.clamp(0.0, 1.0));
                                }
                            }
                            "feather" => {
                                if let Some(v) = parse_f32(&entry.val) {
                                    h.feather = v.clamp(0.0, 1.0);
                                }
                            }
                            "opacity" => {
                                if let Some(v) = parse_f32(&entry.val) {
                                    h.opacity = v.clamp(0.0, 1.0);
                                }
                            }
                            "enabled" => {
                                if let Some(b) = parse_bool(&entry.val) {
                                    h.enabled = b;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "develop.mask" => {
                    seen_known = true;
                    if cur_mask_idx != Some(entry.array_idx) {
                        if let Some(mb) = cur_mask.take() {
                            if sidecar.develop.masks.len() < MAX_MASKS {
                                sidecar.develop.masks.push(mb.build());
                            }
                        }
                        cur_mask = Some(MaskBuilder::new());
                        cur_mask_idx = Some(entry.array_idx);
                    }
                    if let Some(ref mut mb) = cur_mask {
                        mb.apply_kv(&entry.key, &entry.val)?;
                    }
                }
                _ => {}
            }
        }

        if let Some(h) = cur_heal.take() {
            if sidecar.develop.heal_spots.len() < MAX_HEALS {
                sidecar.develop.heal_spots.push(h);
            }
        }
        if let Some(mb) = cur_mask.take() {
            if sidecar.develop.masks.len() < MAX_MASKS {
                sidecar.develop.masks.push(mb.build());
            }
        }

        if !seen_known {
            return Err(SidecarError::ParseError);
        }
        Ok(sidecar)
    }
}

/// Evaluates declarative catalog smart albums across culling state, keywords,
/// camera/lens metadata, capture date/range, and `FilmStock`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SmartAlbumRule {
    pub name: String,
    pub min_stars: Option<u8>,
    pub max_stars: Option<u8>,
    pub required_flag: Option<CullFlag>,
    pub exclude_rejected: bool,
    pub required_label: Option<ColorLabel>,
    /// All listed keywords must be present (case-insensitive).
    pub required_keywords: Vec<String>,
    /// Case-insensitive substring matched against camera or lens metadata.
    pub camera_contains: Option<String>,
    /// Case-insensitive substring matched against resolved place / region.
    pub place_contains: Option<String>,
    pub year: Option<u16>,
    pub year_range: Option<(u16, u16)>,
    pub month: Option<u8>,
    pub date_from: Option<PhotoDate>,
    pub date_to: Option<PhotoDate>,
    pub film_stock: Option<FilmStock>,
    pub only_edited: bool,
    pub only_favorites: bool,
}

impl SmartAlbumRule {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }

    pub fn keepers(name: &str) -> Self {
        Self {
            name: name.to_string(),
            min_stars: Some(1),
            exclude_rejected: true,
            ..Self::default()
        }
    }

    pub fn picks(name: &str) -> Self {
        Self {
            name: name.to_string(),
            required_flag: Some(CullFlag::Pick),
            exclude_rejected: true,
            ..Self::default()
        }
    }

    /// Evaluates this rule directly against a catalog [`PhotoRecord`].
    pub fn matches(&self, rec: &PhotoRecord) -> bool {
        self.matches_parts(
            &rec.meta,
            rec.favorite,
            rec.stars,
            rec.flag,
            rec.color_label,
            &rec.keywords,
            rec.is_edited(),
            rec.develop.film_stock,
        )
    }

    /// Evaluates this rule against explicit metadata + a loaded [`PhotoSidecar`].
    pub fn matches_sidecar(&self, meta: &PhotoMeta, favorite: bool, sidecar: &PhotoSidecar) -> bool {
        let is_edited = !sidecar.edit.is_identity() || !sidecar.develop.is_identity();
        self.matches_parts(
            meta,
            favorite,
            sidecar.stars,
            sidecar.flag,
            sidecar.color_label,
            &sidecar.keywords,
            is_edited,
            sidecar.develop.film_stock,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn matches_parts(
        &self,
        meta: &PhotoMeta,
        favorite: bool,
        stars: u8,
        flag: CullFlag,
        color_label: ColorLabel,
        keywords: &[String],
        is_edited: bool,
        film_stock: Option<FilmStock>,
    ) -> bool {
        if self.exclude_rejected && flag == CullFlag::Reject {
            return false;
        }
        if let Some(req_flag) = self.required_flag {
            if flag != req_flag {
                return false;
            }
        }
        if let Some(min_s) = self.min_stars {
            if stars < min_s {
                return false;
            }
        }
        if let Some(max_s) = self.max_stars {
            if stars > max_s {
                return false;
            }
        }
        if let Some(req_lbl) = self.required_label {
            if color_label != req_lbl {
                return false;
            }
        }
        if self.only_favorites && !favorite {
            return false;
        }
        if self.only_edited && !is_edited {
            return false;
        }
        if let Some(req_film) = self.film_stock {
            if film_stock != Some(req_film) {
                return false;
            }
        }
        for req_kw in &self.required_keywords {
            let needle = req_kw.trim();
            if needle.is_empty() {
                continue;
            }
            if !keywords.iter().any(|k| contains_ci(k, needle)) {
                return false;
            }
        }
        if let Some(ref cam_needle) = self.camera_contains {
            if !cam_needle.is_empty() {
                let cam_ok = meta
                    .camera
                    .as_deref()
                    .map_or(false, |c| contains_ci(c, cam_needle));
                let lens_ok = meta
                    .lens
                    .as_deref()
                    .map_or(false, |l| contains_ci(l, cam_needle));
                if !cam_ok && !lens_ok {
                    return false;
                }
            }
        }
        if let Some(ref place_needle) = self.place_contains {
            if !place_needle.is_empty() {
                let p_ok = meta
                    .place
                    .as_deref()
                    .map_or(false, |p| contains_ci(p, place_needle));
                let r_ok = meta
                    .place_region
                    .as_deref()
                    .map_or(false, |r| contains_ci(r, place_needle));
                if !p_ok && !r_ok {
                    return false;
                }
            }
        }
        if let Some(yr) = self.year {
            if meta.date.map_or(true, |d| d.year != yr) {
                return false;
            }
        }
        if let Some((y0, y1)) = self.year_range {
            let (lo, hi) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
            if meta.date.map_or(true, |d| d.year < lo || d.year > hi) {
                return false;
            }
        }
        if let Some(mo) = self.month {
            if meta.date.map_or(true, |d| d.month != mo) {
                return false;
            }
        }
        if let Some(df) = self.date_from {
            if meta.date.map_or(true, |d| d < df) {
                return false;
            }
        }
        if let Some(dt) = self.date_to {
            if meta.date.map_or(true, |d| d > dt) {
                return false;
            }
        }
        true
    }
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    let h_lower = haystack
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .collect::<String>();
    let n_lower = needle
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .collect::<String>();
    h_lower.contains(&n_lower)
}

fn format_curve(c: &Curve1D) -> String {
    let mut s = String::from("[");
    for (i, pt) in c.points().iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&format!("[{:.6}, {:.6}]", pt[0], pt[1]));
    }
    s.push(']');
    s
}

fn toml_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct MaskBuilder {
    kind: String,
    name: String,
    enabled: bool,
    inverted: bool,
    opacity: f32,
    dabs: Vec<BrushDab>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    feather: f32,
    rotation_deg: f32,
    min_luma: f32,
    max_luma: f32,
    target_oklab: [f32; 3],
    hue_tolerance: f32,
    chroma_tolerance: f32,
    luma_tolerance: f32,
    adj: MaskAdjustments,
}

impl MaskBuilder {
    fn new() -> Self {
        Self {
            kind: String::new(),
            name: String::new(),
            enabled: true,
            inverted: false,
            opacity: 1.0,
            dabs: Vec::new(),
            x0: 0.5,
            y0: 0.0,
            x1: 0.5,
            y1: 0.55,
            cx: 0.5,
            cy: 0.5,
            rx: 0.35,
            ry: 0.35,
            feather: 0.5,
            rotation_deg: 0.0,
            min_luma: 0.0,
            max_luma: 1.0,
            target_oklab: [0.5, 0.0, 0.0],
            hue_tolerance: 0.2,
            chroma_tolerance: 0.2,
            luma_tolerance: 0.45,
            adj: MaskAdjustments::default(),
        }
    }

    fn apply_kv(&mut self, key: &str, val: &str) -> Result<(), SidecarError> {
        match key {
            "kind" => self.kind = unquote_toml(val)?,
            "name" => self.name = unquote_toml(val)?,
            "enabled" => {
                if let Some(b) = parse_bool(val) {
                    self.enabled = b;
                }
            }
            "inverted" => {
                if let Some(b) = parse_bool(val) {
                    self.inverted = b;
                }
            }
            "opacity" => {
                if let Some(v) = parse_f32(val) {
                    self.opacity = v.clamp(0.0, 1.0);
                }
            }
            "dabs" => {
                let rows = parse_nested_f32_arrays(val)?;
                self.dabs.clear();
                for r in rows.into_iter().take(MAX_DABS) {
                    if r.len() >= 5 {
                        self.dabs.push(BrushDab::new(r[0], r[1], r[2], r[3], r[4]));
                    }
                }
            }
            "x0" => {
                if let Some(v) = parse_f32(val) {
                    self.x0 = v;
                }
            }
            "y0" => {
                if let Some(v) = parse_f32(val) {
                    self.y0 = v;
                }
            }
            "x1" => {
                if let Some(v) = parse_f32(val) {
                    self.x1 = v;
                }
            }
            "y1" => {
                if let Some(v) = parse_f32(val) {
                    self.y1 = v;
                }
            }
            "cx" => {
                if let Some(v) = parse_f32(val) {
                    self.cx = v;
                }
            }
            "cy" => {
                if let Some(v) = parse_f32(val) {
                    self.cy = v;
                }
            }
            "rx" => {
                if let Some(v) = parse_f32(val) {
                    self.rx = v;
                }
            }
            "ry" => {
                if let Some(v) = parse_f32(val) {
                    self.ry = v;
                }
            }
            "feather" => {
                if let Some(v) = parse_f32(val) {
                    self.feather = v;
                }
            }
            "rotation_deg" => {
                if let Some(v) = parse_f32(val) {
                    self.rotation_deg = v;
                }
            }
            "min_luma" => {
                if let Some(v) = parse_f32(val) {
                    self.min_luma = v;
                }
            }
            "max_luma" => {
                if let Some(v) = parse_f32(val) {
                    self.max_luma = v;
                }
            }
            "target_oklab" => {
                let v = parse_f32_array(val)?;
                if v.len() == 3 {
                    self.target_oklab = [v[0], v[1], v[2]];
                }
            }
            "hue_tolerance" => {
                if let Some(v) = parse_f32(val) {
                    self.hue_tolerance = v;
                }
            }
            "chroma_tolerance" => {
                if let Some(v) = parse_f32(val) {
                    self.chroma_tolerance = v;
                }
            }
            "luma_tolerance" => {
                if let Some(v) = parse_f32(val) {
                    self.luma_tolerance = v;
                }
            }
            "exposure_ev" => {
                if let Some(v) = parse_f32(val) {
                    self.adj.exposure_ev = v.clamp(-6.0, 6.0);
                }
            }
            "contrast" => {
                if let Some(v) = parse_f32(val) {
                    self.adj.contrast = v.clamp(-1.0, 2.0);
                }
            }
            "highlights" => {
                if let Some(v) = parse_f32(val) {
                    self.adj.highlights = v.clamp(-1.0, 1.0);
                }
            }
            "shadows" => {
                if let Some(v) = parse_f32(val) {
                    self.adj.shadows = v.clamp(-1.0, 1.0);
                }
            }
            "temperature" => {
                if let Some(v) = parse_f32(val) {
                    self.adj.temperature = v.clamp(-2.0, 2.0);
                }
            }
            "tint" => {
                if let Some(v) = parse_f32(val) {
                    self.adj.tint = v.clamp(-2.0, 2.0);
                }
            }
            "saturation" => {
                if let Some(v) = parse_f32(val) {
                    self.adj.saturation = v.clamp(-1.0, 2.0);
                }
            }
            "sharpness" => {
                if let Some(v) = parse_f32(val) {
                    self.adj.sharpness = v.clamp(-1.0, 1.0);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn build(self) -> LocalMask {
        let kind = match self.kind.as_str() {
            "brush" => LocalMaskKind::Brush(BrushMask::new(self.dabs)),
            "linear_gradient" => LocalMaskKind::LinearGradient(LinearGradientMask {
                x0: self.x0,
                y0: self.y0,
                x1: self.x1,
                y1: self.y1,
            }),
            "radial" => LocalMaskKind::Radial(RadialMask {
                cx: self.cx,
                cy: self.cy,
                rx: if self.rx > 1e-4 { self.rx } else { 0.35 },
                ry: if self.ry > 1e-4 { self.ry } else { 0.35 },
                feather: self.feather.clamp(0.0, 1.0),
                rotation_deg: self.rotation_deg,
            }),
            "luminance_range" => LocalMaskKind::LuminanceRange(LuminanceRangeMask {
                min_luma: self.min_luma.clamp(0.0, 1.0),
                max_luma: self.max_luma.clamp(0.0, 1.0),
                feather: self.feather.clamp(0.0, 0.5),
            }),
            "color_range" => LocalMaskKind::ColorRange(ColorRangeMask {
                target_oklab: self.target_oklab,
                hue_tolerance: self.hue_tolerance.clamp(0.02, 1.0),
                chroma_tolerance: self.chroma_tolerance.clamp(0.02, 1.0),
                luma_tolerance: self.luma_tolerance.clamp(0.05, 2.0),
                feather: self.feather.clamp(0.0, 1.0),
            }),
            _ => LocalMaskKind::Radial(RadialMask::default()),
        };
        let name = if self.name.is_empty() {
            kind.label().to_string()
        } else {
            self.name
        };
        LocalMask {
            name,
            enabled: self.enabled,
            inverted: self.inverted,
            opacity: self.opacity.clamp(0.0, 1.0),
            kind,
            adjustments: self.adj,
        }
    }
}

struct TomlEntry {
    section: String,
    array_idx: usize,
    key: String,
    val: String,
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_str = false;
    let mut esc = false;
    for (i, ch) in line.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        if in_str {
            if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
        } else if ch == '"' {
            in_str = true;
        } else if ch == '#' {
            return &line[..i];
        }
    }
    line
}

fn parse_toml_entries(src: &str) -> Result<Vec<TomlEntry>, SidecarError> {
    let mut out = Vec::new();
    let mut section = String::new();
    let mut array_idx = 0usize;

    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let trimmed = strip_toml_comment(lines[i]).trim();
        i += 1;
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("[[") && trimmed.ends_with("]]") {
            let inner = trimmed[2..trimmed.len() - 2].trim();
            if inner.is_empty() {
                return Err(SidecarError::ParseError);
            }
            section = inner.to_string();
            array_idx += 1;
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let inner = trimmed[1..trimmed.len() - 1].trim();
            if inner.is_empty() {
                return Err(SidecarError::ParseError);
            }
            section = inner.to_string();
            continue;
        }
        let Some((k, v)) = trimmed.split_once('=') else {
            return Err(SidecarError::ParseError);
        };
        let key = k.trim();
        if key.is_empty() {
            return Err(SidecarError::ParseError);
        }
        let mut val = v.trim().to_string();

        // Support multiline basic string `"""..."""`
        if val.starts_with("\"\"\"") {
            let after_open = &val[3..];
            if !after_open.ends_with("\"\"\"") || after_open.len() < 3 {
                let mut buf = String::from("\"\"\"");
                buf.push_str(after_open);
                buf.push('\n');
                let mut closed = false;
                while i < lines.len() {
                    let ln = lines[i];
                    i += 1;
                    if ln.trim_end() == "\"\"\"" {
                        buf.push_str("\"\"\"");
                        closed = true;
                        break;
                    }
                    buf.push_str(ln);
                    buf.push('\n');
                }
                if !closed {
                    return Err(SidecarError::ParseError);
                }
                val = buf;
            }
        } else if val.starts_with('[') && bracket_balance(&val) > 0 {
            while i < lines.len() && bracket_balance(&val) > 0 {
                let ln = strip_toml_comment(lines[i]).trim();
                i += 1;
                val.push(' ');
                val.push_str(ln);
            }
            if bracket_balance(&val) != 0 {
                return Err(SidecarError::ParseError);
            }
        }

        out.push(TomlEntry {
            section: section.clone(),
            array_idx,
            key: key.to_string(),
            val,
        });
    }
    Ok(out)
}

fn bracket_balance(s: &str) -> i32 {
    let mut bal = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for ch in s.chars() {
        if esc {
            esc = false;
            continue;
        }
        if in_str {
            if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
        } else {
            match ch {
                '"' => in_str = true,
                '[' => bal += 1,
                ']' => bal -= 1,
                _ => {}
            }
        }
    }
    bal
}

fn unquote_toml(raw: &str) -> Result<String, SidecarError> {
    let t = raw.trim();
    if t.starts_with("\"\"\"") && t.ends_with("\"\"\"") && t.len() >= 6 {
        let inner = &t[3..t.len() - 3];
        let trimmed = inner.strip_prefix('\n').unwrap_or(inner);
        return Ok(trimmed.to_string());
    }
    if !(t.starts_with('"') && t.ends_with('"') && t.len() >= 2) {
        return Err(SidecarError::ParseError);
    }
    let inner = &t[1..t.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => return Err(SidecarError::ParseError),
            }
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

fn parse_u32(raw: &str) -> Option<u32> {
    raw.trim().parse::<u32>().ok()
}

fn parse_i32(raw: &str) -> Option<i32> {
    raw.trim().parse::<i32>().ok()
}

fn parse_f32(raw: &str) -> Option<f32> {
    raw.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn split_top_level_items(inner: &str) -> Result<Vec<&str>, SidecarError> {
    let mut items = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;

    for (idx, ch) in inner.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        if in_str {
            if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth < 0 {
                    return Err(SidecarError::ParseError);
                }
            }
            ',' if depth == 0 => {
                let piece = inner[start..idx].trim();
                if !piece.is_empty() {
                    items.push(piece);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }
    if in_str || depth != 0 {
        return Err(SidecarError::ParseError);
    }
    let tail = inner[start..].trim();
    if !tail.is_empty() {
        items.push(tail);
    }
    Ok(items)
}

fn strip_brackets(raw: &str) -> Result<&str, SidecarError> {
    let t = raw.trim();
    if !(t.starts_with('[') && t.ends_with(']')) {
        return Err(SidecarError::ParseError);
    }
    Ok(t[1..t.len() - 1].trim())
}

fn parse_string_array(raw: &str) -> Result<Vec<String>, SidecarError> {
    let inner = strip_brackets(raw)?;
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let parts = split_top_level_items(inner)?;
    let mut out = Vec::with_capacity(parts.len());
    for p in parts {
        out.push(unquote_toml(p)?);
    }
    Ok(out)
}

fn parse_f32_array(raw: &str) -> Result<Vec<f32>, SidecarError> {
    let inner = strip_brackets(raw)?;
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let parts = split_top_level_items(inner)?;
    let mut out = Vec::with_capacity(parts.len());
    for p in parts {
        out.push(parse_f32(p).ok_or(SidecarError::ParseError)?);
    }
    Ok(out)
}

fn parse_nested_f32_arrays(raw: &str) -> Result<Vec<Vec<f32>>, SidecarError> {
    let inner = strip_brackets(raw)?;
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let parts = split_top_level_items(inner)?;
    let mut out = Vec::with_capacity(parts.len());
    for p in parts {
        out.push(parse_f32_array(p)?);
    }
    Ok(out)
}

fn parse_points_array(raw: &str) -> Result<Vec<[f32; 2]>, SidecarError> {
    let rows = parse_nested_f32_arrays(raw)?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        if r.len() != 2 {
            return Err(SidecarError::ParseError);
        }
        out.push([r[0], r[1]]);
    }
    Ok(out)
}
