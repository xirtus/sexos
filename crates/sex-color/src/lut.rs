//! 1D and 3D `.cube` LUT parsing, Hald CLUT decoding (`sex_image`),
//! and trilinear / exact 6-tetrahedra tetrahedral interpolation.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use libm::{floorf, roundf};
use sex_image::Image;

use crate::frame::{LinearFrame, RgbaF32};

const MAX_LUT_1D_SIZE: usize = 65_536;
const MAX_LUT_3D_SIZE: usize = 256;

/// 3D LUT interpolation algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InterpMode {
    /// Standard 8-corner trilinear interpolation.
    Trilinear,
    /// Exact 6-tetrahedra simplex interpolation (neutral-axis exact, ideal for skin tones & HDR).
    Tetrahedral,
}

/// Errors produced when parsing, validating, or converting LUTs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutError {
    /// Neither `LUT_1D_SIZE` nor `LUT_3D_SIZE` was found.
    MissingSize,
    /// Both `LUT_1D_SIZE` and `LUT_3D_SIZE` were specified in one file.
    ConflictingDimensions,
    /// LUT size is below 2 or exceeds supported bounds.
    InvalidSize,
    /// `DOMAIN_MIN` or `DOMAIN_MAX` is malformed or `min >= max`.
    InvalidDomain,
    /// Expected a 1D LUT but found 3D (or vice versa).
    WrongDimension,
    /// Data row count does not match `size` (1D) or `size^3` (3D).
    EntryCountMismatch,
    /// Failed to parse a header directive or float value, or encountered non-finite data.
    ParseError,
    /// Image dimensions are not a valid cubic Hald CLUT (`width == height == L^3`, `L >= 2`).
    InvalidHaldImage,
}

/// Parsed Adobe `.cube` LUT (either 1D or 3D).
#[derive(Clone, Debug, PartialEq)]
pub enum CubeLut {
    Lut1D(Lut1D),
    Lut3D(Lut3D),
}

impl CubeLut {
    /// Parses a standard Adobe `.cube` LUT (1D or 3D).
    pub fn parse(src: &str) -> Result<Self, LutError> {
        parse_cube_internal(src)
    }
}

/// 1D per-channel lookup table (`size` RGB triplets across `[domain_min, domain_max]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Lut1D {
    pub title: Option<String>,
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    pub table: Vec<[f32; 3]>,
}

impl Lut1D {
    /// Creates an identity 1D LUT of `size` entries over `[0.0, 1.0]`.
    pub fn identity(size: usize) -> Result<Self, LutError> {
        if !(2..=MAX_LUT_1D_SIZE).contains(&size) {
            return Err(LutError::InvalidSize);
        }
        let denom = (size - 1) as f32;
        let mut table = Vec::with_capacity(size);
        for i in 0..size {
            let v = i as f32 / denom;
            table.push([v, v, v]);
        }
        Ok(Self {
            title: None,
            size,
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            table,
        })
    }

    /// Parses a 1D `.cube` LUT string.
    pub fn parse_cube(src: &str) -> Result<Self, LutError> {
        match parse_cube_internal(src)? {
            CubeLut::Lut1D(lut) => Ok(lut),
            CubeLut::Lut3D(_) => Err(LutError::WrongDimension),
        }
    }

    /// Evaluates the 1D LUT using piecewise linear interpolation per channel.
    #[inline]
    pub fn eval(&self, rgb: [f32; 3]) -> [f32; 3] {
        let max_idx = (self.size - 1) as f32;
        let mut out = [0.0f32; 3];
        for c in 0..3 {
            let v = if rgb[c].is_finite() { rgb[c] } else { 0.0 };
            let span = self.domain_max[c] - self.domain_min[c];
            let norm = if span > 1e-12 {
                ((v - self.domain_min[c]) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let pos = norm * max_idx;
            let i0 = (floorf(pos) as usize).min(self.size - 1);
            let i1 = (i0 + 1).min(self.size - 1);
            let f = pos - i0 as f32;
            let a = self.table[i0][c];
            let b = self.table[i1][c];
            out[c] = a + (b - a) * f;
        }
        out
    }

    /// Applies this 1D LUT in place to every pixel in `frame` (preserving alpha).
    pub fn apply_frame(&self, frame: &mut LinearFrame) {
        for px in &mut frame.pixels {
            let rgb = self.eval(px.rgb());
            *px = RgbaF32::from_rgb(rgb, px.0[3]);
        }
    }

    /// Serializes this 1D LUT to Adobe `.cube` format.
    pub fn to_cube(&self) -> String {
        let mut out = String::new();
        if let Some(title) = &self.title {
            out.push_str(&format!("TITLE \"{title}\"\n"));
        }
        out.push_str(&format!("LUT_1D_SIZE {}\n", self.size));
        out.push_str(&format!(
            "DOMAIN_MIN {:.6} {:.6} {:.6}\n",
            self.domain_min[0], self.domain_min[1], self.domain_min[2]
        ));
        out.push_str(&format!(
            "DOMAIN_MAX {:.6} {:.6} {:.6}\n",
            self.domain_max[0], self.domain_max[1], self.domain_max[2]
        ));
        for &[r, g, b] in &self.table {
            out.push_str(&format!("{r:.6} {g:.6} {b:.6}\n"));
        }
        out
    }
}

/// 3D RGB lookup table (`size^3` entries, R fastest, then G, then B slowest).
#[derive(Clone, Debug, PartialEq)]
pub struct Lut3D {
    pub title: Option<String>,
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    pub table: Vec<[f32; 3]>,
}

impl Lut3D {
    /// Creates an identity 3D LUT of `size x size x size` over `[0.0, 1.0]^3`.
    pub fn identity(size: usize) -> Result<Self, LutError> {
        if !(2..=MAX_LUT_3D_SIZE).contains(&size) {
            return Err(LutError::InvalidSize);
        }
        let denom = (size - 1) as f32;
        let mut table = Vec::with_capacity(size * size * size);
        for b in 0..size {
            let bf = b as f32 / denom;
            for g in 0..size {
                let gf = g as f32 / denom;
                for r in 0..size {
                    let rf = r as f32 / denom;
                    table.push([rf, gf, bf]);
                }
            }
        }
        Ok(Self {
            title: None,
            size,
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            table,
        })
    }

    /// Constructs a 3D LUT from an explicit `R`-fastest table of length `size^3`.
    pub fn from_table(
        size: usize,
        domain_min: [f32; 3],
        domain_max: [f32; 3],
        table: Vec<[f32; 3]>,
        title: Option<String>,
    ) -> Result<Self, LutError> {
        if !(2..=MAX_LUT_3D_SIZE).contains(&size) {
            return Err(LutError::InvalidSize);
        }
        if !domain_valid(domain_min, domain_max) {
            return Err(LutError::InvalidDomain);
        }
        let expected = size
            .checked_mul(size)
            .and_then(|s| s.checked_mul(size))
            .ok_or(LutError::InvalidSize)?;
        if table.len() != expected {
            return Err(LutError::EntryCountMismatch);
        }
        for &[r, g, b] in &table {
            if !r.is_finite() || !g.is_finite() || !b.is_finite() {
                return Err(LutError::ParseError);
            }
        }
        Ok(Self {
            title,
            size,
            domain_min,
            domain_max,
            table,
        })
    }

    /// Parses a 3D `.cube` LUT string.
    pub fn parse_cube(src: &str) -> Result<Self, LutError> {
        match parse_cube_internal(src)? {
            CubeLut::Lut3D(lut) => Ok(lut),
            CubeLut::Lut1D(_) => Err(LutError::WrongDimension),
        }
    }

    /// Parses a Hald CLUT image (`width == height == L^3`, `L >= 2`, cube size `N = L^2`)
    /// from [`sex_image::Image`].
    pub fn from_hald_image(image: &Image) -> Result<Self, LutError> {
        if image.width == 0 || image.width != image.height {
            return Err(LutError::InvalidHaldImage);
        }
        if image.pixels.len() != image.width.saturating_mul(image.height) {
            return Err(LutError::InvalidHaldImage);
        }
        let level = find_hald_level(image.width).ok_or(LutError::InvalidHaldImage)?;
        let size = level * level;
        if !(2..=MAX_LUT_3D_SIZE).contains(&size) {
            return Err(LutError::InvalidSize);
        }
        let mut table = Vec::with_capacity(image.pixels.len());
        for &px in &image.pixels {
            let r = ((px >> 16) & 0xFF) as f32 / 255.0;
            let g = ((px >> 8) & 0xFF) as f32 / 255.0;
            let b = (px & 0xFF) as f32 / 255.0;
            table.push([r, g, b]);
        }
        Ok(Self {
            title: None,
            size,
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            table,
        })
    }

    /// Exports this 3D LUT to a Hald CLUT [`sex_image::Image`] if `size` is a perfect square (`L^2`).
    pub fn to_hald_image(&self) -> Result<Image, LutError> {
        let level = find_int_sqrt(self.size).ok_or(LutError::InvalidHaldImage)?;
        if level < 2 {
            return Err(LutError::InvalidHaldImage);
        }
        let side = level * level * level;
        let mut pixels = Vec::with_capacity(self.table.len());
        for &[r, g, b] in &self.table {
            let ri = roundf(r.clamp(0.0, 1.0) * 255.0) as u32;
            let gi = roundf(g.clamp(0.0, 1.0) * 255.0) as u32;
            let bi = roundf(b.clamp(0.0, 1.0) * 255.0) as u32;
            pixels.push(0xFF00_0000 | (ri << 16) | (gi << 8) | bi);
        }
        Ok(Image {
            width: side,
            height: side,
            pixels,
        })
    }

    #[inline]
    fn sample(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        self.table[(b * self.size + g) * self.size + r]
    }

    #[inline]
    fn cell_coords(&self, rgb: [f32; 3]) -> ([usize; 3], [usize; 3], [f32; 3]) {
        let max_idx = (self.size - 1) as f32;
        let mut i0 = [0usize; 3];
        let mut i1 = [0usize; 3];
        let mut f = [0.0f32; 3];
        for c in 0..3 {
            let v = if rgb[c].is_finite() { rgb[c] } else { 0.0 };
            let span = self.domain_max[c] - self.domain_min[c];
            let norm = if span > 1e-12 {
                ((v - self.domain_min[c]) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let pos = norm * max_idx;
            let lo = (floorf(pos) as usize).min(self.size - 2);
            let hi = lo + 1;
            i0[c] = lo;
            i1[c] = hi;
            f[c] = (pos - lo as f32).clamp(0.0, 1.0);
        }
        (i0, i1, f)
    }

    /// Evaluates the 3D LUT using [`InterpMode::Trilinear`] or [`InterpMode::Tetrahedral`].
    #[inline]
    pub fn eval(&self, rgb: [f32; 3], mode: InterpMode) -> [f32; 3] {
        match mode {
            InterpMode::Trilinear => self.eval_trilinear(rgb),
            InterpMode::Tetrahedral => self.eval_tetrahedral(rgb),
        }
    }

    /// Evaluates the 3D LUT with 8-corner trilinear interpolation.
    pub fn eval_trilinear(&self, rgb: [f32; 3]) -> [f32; 3] {
        let ([r0, g0, b0], [r1, g1, b1], [fr, fg, fb]) = self.cell_coords(rgb);
        let c000 = self.sample(r0, g0, b0);
        let c100 = self.sample(r1, g0, b0);
        let c010 = self.sample(r0, g1, b0);
        let c110 = self.sample(r1, g1, b0);
        let c001 = self.sample(r0, g0, b1);
        let c101 = self.sample(r1, g0, b1);
        let c011 = self.sample(r0, g1, b1);
        let c111 = self.sample(r1, g1, b1);

        let mut out = [0.0f32; 3];
        for c in 0..3 {
            let c00 = c000[c] + (c100[c] - c000[c]) * fr;
            let c10 = c010[c] + (c110[c] - c010[c]) * fr;
            let c01 = c001[c] + (c101[c] - c001[c]) * fr;
            let c11 = c011[c] + (c111[c] - c011[c]) * fr;
            let c0 = c00 + (c10 - c00) * fg;
            let c1 = c01 + (c11 - c01) * fg;
            out[c] = c0 + (c1 - c0) * fb;
        }
        out
    }

    /// Evaluates the 3D LUT with exact 6-tetrahedra simplex interpolation.
    ///
    /// Partitions each unit cube along the main `(0,0,0) -> (1,1,1)` neutral axis into
    /// 6 non-overlapping tetrahedra, preventing neutral-gray contamination from chromatic corners.
    pub fn eval_tetrahedral(&self, rgb: [f32; 3]) -> [f32; 3] {
        let ([r0, g0, b0], [r1, g1, b1], [fr, fg, fb]) = self.cell_coords(rgb);
        let c000 = self.sample(r0, g0, b0);
        let c111 = self.sample(r1, g1, b1);

        let (w0, w1, v1, w2, v2, w3) = if fr >= fg {
            if fg >= fb {
                // fr >= fg >= fb
                (
                    1.0 - fr,
                    fr - fg,
                    self.sample(r1, g0, b0),
                    fg - fb,
                    self.sample(r1, g1, b0),
                    fb,
                )
            } else if fr >= fb {
                // fr >= fb > fg
                (
                    1.0 - fr,
                    fr - fb,
                    self.sample(r1, g0, b0),
                    fb - fg,
                    self.sample(r1, g0, b1),
                    fg,
                )
            } else {
                // fb > fr >= fg
                (
                    1.0 - fb,
                    fb - fr,
                    self.sample(r0, g0, b1),
                    fr - fg,
                    self.sample(r1, g0, b1),
                    fg,
                )
            }
        } else if fb > fg {
            // fb > fg > fr
            (
                1.0 - fb,
                fb - fg,
                self.sample(r0, g0, b1),
                fg - fr,
                self.sample(r0, g1, b1),
                fr,
            )
        } else if fb > fr {
            // fg >= fb > fr
            (
                1.0 - fg,
                fg - fb,
                self.sample(r0, g1, b0),
                fb - fr,
                self.sample(r0, g1, b1),
                fr,
            )
        } else {
            // fg > fr >= fb
            (
                1.0 - fg,
                fg - fr,
                self.sample(r0, g1, b0),
                fr - fb,
                self.sample(r1, g1, b0),
                fb,
            )
        };

        [
            w0 * c000[0] + w1 * v1[0] + w2 * v2[0] + w3 * c111[0],
            w0 * c000[1] + w1 * v1[1] + w2 * v2[1] + w3 * c111[1],
            w0 * c000[2] + w1 * v1[2] + w2 * v2[2] + w3 * c111[2],
        ]
    }

    /// Applies this 3D LUT in place across every pixel in `frame` (preserving alpha).
    pub fn apply_frame(&self, frame: &mut LinearFrame, mode: InterpMode) {
        for px in &mut frame.pixels {
            let rgb = self.eval(px.rgb(), mode);
            *px = RgbaF32::from_rgb(rgb, px.0[3]);
        }
    }

    /// Serializes this 3D LUT to Adobe `.cube` format (`R` fastest, `B` slowest).
    pub fn to_cube(&self) -> String {
        let mut out = String::new();
        if let Some(title) = &self.title {
            out.push_str(&format!("TITLE \"{title}\"\n"));
        }
        out.push_str(&format!("LUT_3D_SIZE {}\n", self.size));
        out.push_str(&format!(
            "DOMAIN_MIN {:.6} {:.6} {:.6}\n",
            self.domain_min[0], self.domain_min[1], self.domain_min[2]
        ));
        out.push_str(&format!(
            "DOMAIN_MAX {:.6} {:.6} {:.6}\n",
            self.domain_max[0], self.domain_max[1], self.domain_max[2]
        ));
        for &[r, g, b] in &self.table {
            out.push_str(&format!("{r:.6} {g:.6} {b:.6}\n"));
        }
        out
    }
}

fn find_hald_level(side: usize) -> Option<usize> {
    let mut l = 2usize;
    while let Some(cube) = l.checked_mul(l).and_then(|sq| sq.checked_mul(l)) {
        if cube == side {
            return Some(l);
        }
        if cube > side {
            break;
        }
        l += 1;
    }
    None
}

fn find_int_sqrt(n: usize) -> Option<usize> {
    let mut k = 1usize;
    while let Some(sq) = k.checked_mul(k) {
        if sq == n {
            return Some(k);
        }
        if sq > n {
            break;
        }
        k += 1;
    }
    None
}

fn domain_valid(min: [f32; 3], max: [f32; 3]) -> bool {
    for i in 0..3 {
        if !min[i].is_finite() || !max[i].is_finite() || min[i] >= max[i] {
            return false;
        }
    }
    true
}

fn parse_three_floats(rest: &str) -> Result<[f32; 3], LutError> {
    let mut iter = rest.split_whitespace();
    let a = iter.next().ok_or(LutError::ParseError)?.parse::<f32>().map_err(|_| LutError::ParseError)?;
    let b = iter.next().ok_or(LutError::ParseError)?.parse::<f32>().map_err(|_| LutError::ParseError)?;
    let c = iter.next().ok_or(LutError::ParseError)?.parse::<f32>().map_err(|_| LutError::ParseError)?;
    if iter.next().is_some() || !a.is_finite() || !b.is_finite() || !c.is_finite() {
        return Err(LutError::ParseError);
    }
    Ok([a, b, c])
}

fn parse_cube_internal(src: &str) -> Result<CubeLut, LutError> {
    let mut title: Option<String> = None;
    let mut size_1d: Option<usize> = None;
    let mut size_3d: Option<usize> = None;
    let mut domain_min = [0.0f32, 0.0, 0.0];
    let mut domain_max = [1.0f32, 1.0, 1.0];
    let mut table: Vec<[f32; 3]> = Vec::new();

    for raw_line in src.lines() {
        let no_comment = match raw_line.find('#') {
            Some(idx) => &raw_line[..idx],
            None => raw_line,
        };
        let line = no_comment.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = strip_keyword(line, "TITLE") {
            let trimmed = rest.trim();
            let unquoted = if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
                &trimmed[1..trimmed.len() - 1]
            } else {
                trimmed
            };
            title = Some(unquoted.to_string());
        } else if let Some(rest) = strip_keyword(line, "LUT_1D_SIZE") {
            if size_3d.is_some() {
                return Err(LutError::ConflictingDimensions);
            }
            let n = rest.trim().parse::<usize>().map_err(|_| LutError::InvalidSize)?;
            if !(2..=MAX_LUT_1D_SIZE).contains(&n) {
                return Err(LutError::InvalidSize);
            }
            size_1d = Some(n);
            table.reserve(n);
        } else if let Some(rest) = strip_keyword(line, "LUT_3D_SIZE") {
            if size_1d.is_some() {
                return Err(LutError::ConflictingDimensions);
            }
            let n = rest.trim().parse::<usize>().map_err(|_| LutError::InvalidSize)?;
            if !(2..=MAX_LUT_3D_SIZE).contains(&n) {
                return Err(LutError::InvalidSize);
            }
            size_3d = Some(n);
            table.reserve(n * n * n);
        } else if let Some(rest) = strip_keyword(line, "DOMAIN_MIN") {
            domain_min = parse_three_floats(rest).map_err(|_| LutError::InvalidDomain)?;
        } else if let Some(rest) = strip_keyword(line, "DOMAIN_MAX") {
            domain_max = parse_three_floats(rest).map_err(|_| LutError::InvalidDomain)?;
        } else if let Some(rest) = strip_keyword(line, "LUT_1D_INPUT_RANGE")
            .or_else(|| strip_keyword(line, "LUT_3D_INPUT_RANGE"))
        {
            let mut it = rest.split_whitespace();
            let lo = it.next().ok_or(LutError::InvalidDomain)?.parse::<f32>().map_err(|_| LutError::InvalidDomain)?;
            let hi = it.next().ok_or(LutError::InvalidDomain)?.parse::<f32>().map_err(|_| LutError::InvalidDomain)?;
            if it.next().is_some() || !lo.is_finite() || !hi.is_finite() || lo >= hi {
                return Err(LutError::InvalidDomain);
            }
            domain_min = [lo, lo, lo];
            domain_max = [hi, hi, hi];
        } else {
            let triplet = parse_three_floats(line)?;
            table.push(triplet);
        }
    }

    if !domain_valid(domain_min, domain_max) {
        return Err(LutError::InvalidDomain);
    }

    match (size_1d, size_3d) {
        (Some(n), None) => {
            if table.len() != n {
                return Err(LutError::EntryCountMismatch);
            }
            Ok(CubeLut::Lut1D(Lut1D {
                title,
                size: n,
                domain_min,
                domain_max,
                table,
            }))
        }
        (None, Some(n)) => {
            let expected = n.checked_mul(n).and_then(|s| s.checked_mul(n)).ok_or(LutError::InvalidSize)?;
            if table.len() != expected {
                return Err(LutError::EntryCountMismatch);
            }
            Ok(CubeLut::Lut3D(Lut3D {
                title,
                size: n,
                domain_min,
                domain_max,
                table,
            }))
        }
        (Some(_), Some(_)) => Err(LutError::ConflictingDimensions),
        (None, None) => Err(LutError::MissingSize),
    }
}

fn strip_keyword<'a>(line: &'a str, kw: &str) -> Option<&'a str> {
    if !line.starts_with(kw) {
        return None;
    }
    let rest = &line[kw.len()..];
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest)
    } else {
        None
    }
}
