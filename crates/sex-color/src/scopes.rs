//! Real-time broadcast video scopes rendered directly from [`LinearFrame`] or BGRA8 slices
//! into a `0xAARRGGBB` [`ScopeSurface`]:
//! - Waveform monitor (BT.709 / BT.2020 IRE 0..100 graticule, Luma or RGB overlay)
//! - RGB Parade (side-by-side R, G, B column traces with IRE markers)
//! - Vectorscope (Cb/Cr circle, 75% SMPTE R/Mg/B/Cy/G/Yl target boxes, skin-tone I-line)
//! - 256-bin R/G/B/Luma Histogram

use alloc::vec;
use alloc::vec::Vec;
use libm::{roundf, sqrtf};

use crate::frame::LinearFrame;
use crate::space::{
    convert_primaries, linear_to_rec2020, linear_to_rec709, ColorPrimaries,
};

const BG_COLOR: u32 = 0xFF0E_1014;
const GRATICULE_MAJOR: u32 = 0xFF48_5060;
const GRATICULE_MINOR: u32 = 0xFF28_2E38;
const SKIN_ILINE_COLOR: u32 = 0xFFD9_A048;
const TARGET_BOX_COLOR: u32 = 0xFF8C_96A8;

/// Color standard used for scope luma and Y'CbCr calculations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScopeStandard {
    /// Follow the source frame's primaries (`Rec2020` -> BT.2020, otherwise BT.709).
    Auto,
    /// ITU-R BT.709 (`0.2126 R + 0.7152 G + 0.0722 B`).
    Rec709,
    /// ITU-R BT.2020 (`0.2627 R + 0.6780 G + 0.0593 B`).
    Rec2020,
}

impl ScopeStandard {
    #[inline]
    fn resolve(self, frame_primaries: ColorPrimaries) -> ResolvedStandard {
        match self {
            Self::Rec709 => ResolvedStandard::Rec709,
            Self::Rec2020 => ResolvedStandard::Rec2020,
            Self::Auto => match frame_primaries {
                ColorPrimaries::Rec2020 => ResolvedStandard::Rec2020,
                ColorPrimaries::Rec709 | ColorPrimaries::Rec601 => ResolvedStandard::Rec709,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedStandard {
    Rec709,
    Rec2020,
}

impl ResolvedStandard {
    #[inline]
    const fn primaries(self) -> ColorPrimaries {
        match self {
            Self::Rec709 => ColorPrimaries::Rec709,
            Self::Rec2020 => ColorPrimaries::Rec2020,
        }
    }

    #[inline]
    const fn weights(self) -> [f32; 3] {
        self.primaries().luma_weights()
    }

    #[inline]
    fn oetf_rgb(self, lin: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Rec709 => [
                linear_to_rec709(lin[0]).clamp(0.0, 1.0),
                linear_to_rec709(lin[1]).clamp(0.0, 1.0),
                linear_to_rec709(lin[2]).clamp(0.0, 1.0),
            ],
            Self::Rec2020 => [
                linear_to_rec2020(lin[0]).clamp(0.0, 1.0),
                linear_to_rec2020(lin[1]).clamp(0.0, 1.0),
                linear_to_rec2020(lin[2]).clamp(0.0, 1.0),
            ],
        }
    }
}

/// Waveform display mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WaveformMode {
    /// Single green-cyan luma trace.
    Luma,
    /// Overlaid R, G, and B channel traces.
    RgbOverlay,
}

/// Borrowed source image input for scope rendering.
#[derive(Clone, Copy, Debug)]
pub enum ScopeInput<'a> {
    /// Linear-light floating-point frame.
    Linear(&'a LinearFrame),
    /// Packed BGRA8 (`[B, G, R, A]` bytes) sRGB/video buffer.
    Bgra8 {
        width: usize,
        height: usize,
        data: &'a [u8],
    },
    /// Packed `0xAARRGGBB` `u32` sRGB/video buffer.
    ArgbU32 {
        width: usize,
        height: usize,
        data: &'a [u32],
    },
}

impl<'a> From<&'a LinearFrame> for ScopeInput<'a> {
    #[inline]
    fn from(frame: &'a LinearFrame) -> Self {
        Self::Linear(frame)
    }
}

impl<'a> ScopeInput<'a> {
    /// Constructs a BGRA8 byte-slice input (`[B, G, R, A]`).
    #[inline]
    pub const fn bgra8(width: usize, height: usize, data: &'a [u8]) -> Self {
        Self::Bgra8 { width, height, data }
    }

    /// Constructs a `0xAARRGGBB` `u32` slice input.
    #[inline]
    pub const fn argb_u32(width: usize, height: usize, data: &'a [u32]) -> Self {
        Self::ArgbU32 { width, height, data }
    }

    fn dimensions_and_primaries(&self) -> (usize, usize, ColorPrimaries) {
        match *self {
            Self::Linear(f) => (f.width, f.height, f.primaries),
            Self::Bgra8 { width, height, data } => {
                let avail = data.len() / 4;
                if width == 0 || avail < width.saturating_mul(height) {
                    (0, 0, ColorPrimaries::Rec709)
                } else {
                    (width, height, ColorPrimaries::Rec709)
                }
            }
            Self::ArgbU32 { width, height, data } => {
                if width == 0 || data.len() < width.saturating_mul(height) {
                    (0, 0, ColorPrimaries::Rec709)
                } else {
                    (width, height, ColorPrimaries::Rec709)
                }
            }
        }
    }

    #[inline]
    fn sample_encoded_rgb(&self, idx: usize, target: ResolvedStandard) -> [f32; 3] {
        match *self {
            Self::Linear(frame) => {
                let px = frame.pixels[idx].sanitize();
                let rgb = convert_primaries(px.rgb(), frame.primaries, target.primaries());
                target.oetf_rgb(rgb)
            }
            Self::Bgra8 { data, .. } => {
                let off = idx * 4;
                let b = data[off] as f32 / 255.0;
                let g = data[off + 1] as f32 / 255.0;
                let r = data[off + 2] as f32 / 255.0;
                [r, g, b]
            }
            Self::ArgbU32 { data, .. } => {
                let p = data[idx];
                let r = ((p >> 16) & 0xFF) as f32 / 255.0;
                let g = ((p >> 8) & 0xFF) as f32 / 255.0;
                let b = (p & 0xFF) as f32 / 255.0;
                [r, g, b]
            }
        }
    }
}

/// 256-bin R, G, B, and Luma histogram data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistogramData {
    pub r: [u32; 256],
    pub g: [u32; 256],
    pub b: [u32; 256],
    pub luma: [u32; 256],
    pub total_pixels: usize,
}

impl Default for HistogramData {
    fn default() -> Self {
        Self {
            r: [0; 256],
            g: [0; 256],
            b: [0; 256],
            luma: [0; 256],
            total_pixels: 0,
        }
    }
}

/// Computes a 256-bin R/G/B/Luma histogram directly from a [`LinearFrame`] or BGRA8/ARGB32 slice.
pub fn compute_histogram<'a>(
    input: impl Into<ScopeInput<'a>>,
    standard: ScopeStandard,
) -> HistogramData {
    let input = input.into();
    let (w, h, src_prim) = input.dimensions_and_primaries();
    let mut data = HistogramData::default();
    if w == 0 || h == 0 {
        return data;
    }
    let std_resolved = standard.resolve(src_prim);
    let [kr, kg, kb] = std_resolved.weights();
    let count = w * h;
    data.total_pixels = count;
    for i in 0..count {
        let [r, g, b] = input.sample_encoded_rgb(i, std_resolved);
        let y = (kr * r + kg * g + kb * b).clamp(0.0, 1.0);
        let ri = (roundf(r * 255.0) as usize).min(255);
        let gi = (roundf(g * 255.0) as usize).min(255);
        let bi = (roundf(b * 255.0) as usize).min(255);
        let yi = (roundf(y * 255.0) as usize).min(255);
        data.r[ri] += 1;
        data.g[gi] += 1;
        data.b[bi] += 1;
        data.luma[yi] += 1;
    }
    data
}

/// Pixel surface (`0xAARRGGBB`, matching `sex-image` / `sex-graphics`) for rendering video scopes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeSurface {
    pub width: usize,
    pub height: usize,
    pub bgra: Vec<u32>,
}

impl ScopeSurface {
    /// Creates a new scope surface initialized to the scope background color.
    pub fn new(width: usize, height: usize) -> Self {
        let len = width.saturating_mul(height);
        Self {
            width,
            height,
            bgra: vec![BG_COLOR; len],
        }
    }

    /// Clears the surface to `color` (`0xAARRGGBB`).
    pub fn clear(&mut self, color: u32) {
        self.bgra.fill(color);
    }

    /// Reads the pixel at `(x, y)` if in bounds.
    #[inline]
    pub fn pixel(&self, x: usize, y: usize) -> Option<u32> {
        if x < self.width && y < self.height {
            self.bgra.get(y * self.width + x).copied()
        } else {
            None
        }
    }

    /// Sets the pixel at `(x, y)` if in bounds.
    #[inline]
    pub fn put_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            self.bgra[y * self.width + x] = color;
        }
    }

    #[inline]
    fn put_pixel_signed(&mut self, x: isize, y: isize, color: u32) {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
            self.bgra[y as usize * self.width + x as usize] = color;
        }
    }

    #[inline]
    fn add_color_pixel(&mut self, x: usize, y: usize, add_r: u8, add_g: u8, add_b: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = y * self.width + x;
        let dst = self.bgra[idx];
        let r = (((dst >> 16) & 0xFF) as u16 + add_r as u16).min(255) as u32;
        let g = (((dst >> 8) & 0xFF) as u16 + add_g as u16).min(255) as u32;
        let b = ((dst & 0xFF) as u16 + add_b as u16).min(255) as u32;
        self.bgra[idx] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
    }

    /// Draws an integer Bresenham line clipped to the surface bounds.
    pub fn draw_line(&mut self, mut x0: isize, mut y0: isize, x1: isize, y1: isize, color: u32) {
        let dx = (x1 - x0).abs();
        let sx: isize = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy: isize = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.put_pixel_signed(x0, y0, color);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn draw_hline(&mut self, x0: usize, x1: usize, y: usize, color: u32) {
        if y >= self.height || x0 >= self.width {
            return;
        }
        let end = x1.min(self.width - 1);
        let row = y * self.width;
        for x in x0..=end {
            self.bgra[row + x] = color;
        }
    }

    fn draw_vline(&mut self, x: usize, y0: usize, y1: usize, color: u32) {
        if x >= self.width || y0 >= self.height {
            return;
        }
        let end = y1.min(self.height - 1);
        for y in y0..=end {
            self.bgra[y * self.width + x] = color;
        }
    }

    fn draw_rect_outline(&mut self, cx: isize, cy: isize, half: isize, color: u32) {
        let x0 = cx - half;
        let y0 = cy - half;
        let x1 = cx + half;
        let y1 = cy + half;
        self.draw_line(x0, y0, x1, y0, color);
        self.draw_line(x1, y0, x1, y1, color);
        self.draw_line(x1, y1, x0, y1, color);
        self.draw_line(x0, y1, x0, y0, color);
    }

    fn draw_circle(&mut self, cx: isize, cy: isize, r: isize, color: u32) {
        if r <= 0 {
            return;
        }
        let mut x = r;
        let mut y = 0isize;
        let mut err = 1 - r;
        while x >= y {
            self.put_pixel_signed(cx + x, cy + y, color);
            self.put_pixel_signed(cx + y, cy + x, color);
            self.put_pixel_signed(cx - y, cy + x, color);
            self.put_pixel_signed(cx - x, cy + y, color);
            self.put_pixel_signed(cx - x, cy - y, color);
            self.put_pixel_signed(cx - y, cy - x, color);
            self.put_pixel_signed(cx + y, cy - x, color);
            self.put_pixel_signed(cx + x, cy - y, color);
            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
    }

    /// Maps an IRE value in `[0.0, 100.0]` to its vertical pixel row on this scope surface.
    #[inline]
    pub fn ire_to_y(&self, ire: f32) -> usize {
        if self.height <= 1 {
            return 0;
        }
        let pad_y = if self.height >= 32 {
            (self.height / 16).clamp(2, 12)
        } else {
            0
        };
        let y_top = pad_y;
        let y_bot = self.height - 1 - pad_y;
        let t = (ire / 100.0).clamp(0.0, 1.0);
        (roundf(y_bot as f32 - t * (y_bot - y_top) as f32) as usize).min(self.height - 1)
    }

    fn draw_ire_graticule(&mut self) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        for &(ire, major) in &[
            (0.0, true),
            (25.0, false),
            (50.0, true),
            (75.0, false),
            (100.0, true),
        ] {
            let y = self.ire_to_y(ire);
            let color = if major { GRATICULE_MAJOR } else { GRATICULE_MINOR };
            self.draw_hline(0, self.width - 1, y, color);
        }
    }

    /// Renders a waveform monitor (IRE 0..100 graticule, Luma or RGB overlay).
    pub fn render_waveform<'a>(
        &mut self,
        input: impl Into<ScopeInput<'a>>,
        mode: WaveformMode,
        standard: ScopeStandard,
    ) {
        self.clear(BG_COLOR);
        if self.width == 0 || self.height == 0 {
            return;
        }
        self.draw_ire_graticule();

        let input = input.into();
        let (src_w, src_h, src_prim) = input.dimensions_and_primaries();
        if src_w == 0 || src_h == 0 {
            return;
        }
        let std_resolved = standard.resolve(src_prim);
        let [kr, kg, kb] = std_resolved.weights();
        let sw = self.width;
        let sh = self.height;

        match mode {
            WaveformMode::Luma => {
                let mut counts = vec![0u32; sw * sh];
                let mut max_hit = 1u32;
                for y in 0..src_h {
                    for x in 0..src_w {
                        let sx = (x * sw) / src_w;
                        let [r, g, b] = input.sample_encoded_rgb(y * src_w + x, std_resolved);
                        let luma = (kr * r + kg * g + kb * b).clamp(0.0, 1.0);
                        let sy = self.ire_to_y(luma * 100.0);
                        let cell = &mut counts[sy * sw + sx];
                        *cell += 1;
                        if *cell > max_hit {
                            max_hit = *cell;
                        }
                    }
                }
                let inv_max = 1.0 / (max_hit as f32);
                for (idx, &hit) in counts.iter().enumerate() {
                    if hit > 0 {
                        let intensity = (0.35 + 0.65 * sqrtf(hit as f32 * inv_max)).clamp(0.0, 1.0);
                        let g = roundf(intensity * 235.0) as u8;
                        let b = roundf(intensity * 165.0) as u8;
                        let r = roundf(intensity * 55.0) as u8;
                        self.add_color_pixel(idx % sw, idx / sw, r, g, b);
                    }
                }
            }
            WaveformMode::RgbOverlay => {
                let mut counts = vec![[0u32; 3]; sw * sh];
                let mut max_hit = 1u32;
                for y in 0..src_h {
                    for x in 0..src_w {
                        let sx = (x * sw) / src_w;
                        let rgb = input.sample_encoded_rgb(y * src_w + x, std_resolved);
                        for ch in 0..3 {
                            let sy = self.ire_to_y(rgb[ch].clamp(0.0, 1.0) * 100.0);
                            let cell = &mut counts[sy * sw + sx][ch];
                            *cell += 1;
                            if *cell > max_hit {
                                max_hit = *cell;
                            }
                        }
                    }
                }
                let inv_max = 1.0 / (max_hit as f32);
                for (idx, &hits) in counts.iter().enumerate() {
                    if hits[0] > 0 || hits[1] > 0 || hits[2] > 0 {
                        let r = if hits[0] > 0 {
                            roundf((0.35 + 0.65 * sqrtf(hits[0] as f32 * inv_max)) * 255.0) as u8
                        } else {
                            0
                        };
                        let g = if hits[1] > 0 {
                            roundf((0.35 + 0.65 * sqrtf(hits[1] as f32 * inv_max)) * 255.0) as u8
                        } else {
                            0
                        };
                        let b = if hits[2] > 0 {
                            roundf((0.35 + 0.65 * sqrtf(hits[2] as f32 * inv_max)) * 255.0) as u8
                        } else {
                            0
                        };
                        self.add_color_pixel(idx % sw, idx / sw, r, g, b);
                    }
                }
            }
        }
    }

    /// Renders a side-by-side R / G / B parade with IRE 0..100 graticule lines.
    pub fn render_rgb_parade<'a>(
        &mut self,
        input: impl Into<ScopeInput<'a>>,
        standard: ScopeStandard,
    ) {
        self.clear(BG_COLOR);
        if self.width < 3 || self.height == 0 {
            return;
        }
        self.draw_ire_graticule();

        let p1 = self.width / 3;
        let p2 = (self.width * 2) / 3;
        self.draw_vline(p1, 0, self.height - 1, GRATICULE_MAJOR);
        self.draw_vline(p2, 0, self.height - 1, GRATICULE_MAJOR);

        let input = input.into();
        let (src_w, src_h, src_prim) = input.dimensions_and_primaries();
        if src_w == 0 || src_h == 0 {
            return;
        }
        let std_resolved = standard.resolve(src_prim);

        let panel_bounds = [(0usize, p1.max(1)), (p1, p2.max(p1 + 1)), (p2, self.width)];
        let sw = self.width;
        let sh = self.height;
        let mut counts = vec![0u32; sw * sh];
        let mut max_hit = 1u32;

        for y in 0..src_h {
            for x in 0..src_w {
                let rgb = input.sample_encoded_rgb(y * src_w + x, std_resolved);
                for ch in 0..3 {
                    let (px0, px1) = panel_bounds[ch];
                    let pw = (px1 - px0).max(1);
                    let sx = px0 + (x * pw) / src_w;
                    let sy = self.ire_to_y(rgb[ch].clamp(0.0, 1.0) * 100.0);
                    let cell = &mut counts[sy * sw + sx];
                    *cell += 1;
                    if *cell > max_hit {
                        max_hit = *cell;
                    }
                }
            }
        }

        let inv_max = 1.0 / (max_hit as f32);
        for sy in 0..sh {
            for sx in 0..sw {
                let hit = counts[sy * sw + sx];
                if hit == 0 {
                    continue;
                }
                let intensity = (0.35 + 0.65 * sqrtf(hit as f32 * inv_max)).clamp(0.0, 1.0);
                let val = roundf(intensity * 255.0) as u8;
                let dim = roundf(intensity * 45.0) as u8;
                if sx < p1 {
                    self.add_color_pixel(sx, sy, val, dim, dim);
                } else if sx < p2 {
                    self.add_color_pixel(sx, sy, dim, val, dim);
                } else {
                    self.add_color_pixel(sx, sy, dim, dim, val);
                }
            }
        }
    }

    /// Returns `(cx, cy, radius)` for the vectorscope circular graticule.
    #[inline]
    pub fn vectorscope_geometry(&self) -> (isize, isize, f32) {
        let cx = (self.width / 2) as isize;
        let cy = (self.height / 2) as isize;
        let r = ((self.width.min(self.height) as f32) * 0.42).max(4.0);
        (cx, cy, r)
    }

    /// Maps an OETF-encoded `[r, g, b]` color to `(x, y)` coordinates on the vectorscope.
    #[inline]
    pub fn vectorscope_coords(&self, rgb: [f32; 3], standard: ScopeStandard) -> (isize, isize) {
        let [kr, kg, kb] = match standard {
            ScopeStandard::Rec2020 => ColorPrimaries::Rec2020.luma_weights(),
            ScopeStandard::Auto | ScopeStandard::Rec709 => ColorPrimaries::Rec709.luma_weights(),
        };
        let y = kr * rgb[0] + kg * rgb[1] + kb * rgb[2];
        let cb = (rgb[2] - y) / (2.0 * (1.0 - kb));
        let cr = (rgb[0] - y) / (2.0 * (1.0 - kr));
        let (cx, cy, r) = self.vectorscope_geometry();
        let scale = r / 0.55;
        (
            cx + roundf(cb * scale) as isize,
            cy - roundf(cr * scale) as isize,
        )
    }

    /// Returns the outer endpoint `(x, y)` of the 123-degree skin-tone I-line graticule.
    #[inline]
    pub fn skin_tone_iline_endpoint(&self) -> (isize, isize) {
        // 123 deg from +Cb axis: cos(123 deg) = -0.544639, sin(123 deg) = +0.838671
        let (cx, cy, r) = self.vectorscope_geometry();
        (
            cx + roundf(r * -0.544_639) as isize,
            cy - roundf(r * 0.838_671) as isize,
        )
    }

    /// Renders a Cb/Cr vectorscope with outer & 75% circles, 75% SMPTE R/Mg/B/Cy/G/Yl target boxes,
    /// and the broadcast skin-tone I-line graticule.
    pub fn render_vectorscope<'a>(
        &mut self,
        input: impl Into<ScopeInput<'a>>,
        standard: ScopeStandard,
    ) {
        self.clear(BG_COLOR);
        if self.width < 8 || self.height < 8 {
            return;
        }
        let input = input.into();
        let (src_w, src_h, src_prim) = input.dimensions_and_primaries();
        let std_resolved = standard.resolve(src_prim);
        let eff_std = match std_resolved {
            ResolvedStandard::Rec709 => ScopeStandard::Rec709,
            ResolvedStandard::Rec2020 => ScopeStandard::Rec2020,
        };

        let (cx, cy, r) = self.vectorscope_geometry();
        let ri = roundf(r) as isize;
        let r75 = roundf(r * 0.75) as isize;

        // Crosshairs and concentric circles.
        self.draw_line(cx - ri, cy, cx + ri, cy, GRATICULE_MINOR);
        self.draw_line(cx, cy - ri, cx, cy + ri, GRATICULE_MINOR);
        self.draw_circle(cx, cy, r75, GRATICULE_MINOR);
        self.draw_circle(cx, cy, ri, GRATICULE_MAJOR);

        // Skin-tone I-line (123 degrees).
        let (ix, iy) = self.skin_tone_iline_endpoint();
        self.draw_line(cx, cy, ix, iy, SKIN_ILINE_COLOR);

        // 75% SMPTE color bar target boxes: Red, Magenta, Blue, Cyan, Green, Yellow.
        const SMPTE_75: [[f32; 3]; 6] = [
            [0.75, 0.0, 0.0],
            [0.75, 0.0, 0.75],
            [0.0, 0.0, 0.75],
            [0.0, 0.75, 0.75],
            [0.0, 0.75, 0.0],
            [0.75, 0.75, 0.0],
        ];
        let box_half = (ri / 18).clamp(2, 5);
        for target in SMPTE_75 {
            let (tx, ty) = self.vectorscope_coords(target, eff_std);
            self.draw_rect_outline(tx, ty, box_half, TARGET_BOX_COLOR);
        }

        if src_w == 0 || src_h == 0 {
            return;
        }

        let sw = self.width;
        let sh = self.height;
        let mut counts = vec![0u32; sw * sh];
        let mut sum_rgb = vec![[0f32; 3]; sw * sh];
        let mut max_hit = 1u32;

        let count = src_w * src_h;
        for i in 0..count {
            let rgb = input.sample_encoded_rgb(i, std_resolved);
            let (px, py) = self.vectorscope_coords(rgb, eff_std);
            if px >= 0 && py >= 0 && (px as usize) < sw && (py as usize) < sh {
                let idx = py as usize * sw + px as usize;
                counts[idx] += 1;
                sum_rgb[idx][0] += rgb[0];
                sum_rgb[idx][1] += rgb[1];
                sum_rgb[idx][2] += rgb[2];
                if counts[idx] > max_hit {
                    max_hit = counts[idx];
                }
            }
        }

        let inv_max = 1.0 / (max_hit as f32);
        for (idx, &hit) in counts.iter().enumerate() {
            if hit > 0 {
                let intensity = (0.45 + 0.55 * sqrtf(hit as f32 * inv_max)).clamp(0.0, 1.0);
                let inv_h = 1.0 / hit as f32;
                let r_avg = (sum_rgb[idx][0] * inv_h).max(0.25);
                let g_avg = (sum_rgb[idx][1] * inv_h).max(0.25);
                let b_avg = (sum_rgb[idx][2] * inv_h).max(0.25);
                let r_u8 = roundf((r_avg * intensity).clamp(0.0, 1.0) * 255.0) as u8;
                let g_u8 = roundf((g_avg * intensity).clamp(0.0, 1.0) * 255.0) as u8;
                let b_u8 = roundf((b_avg * intensity).clamp(0.0, 1.0) * 255.0) as u8;
                self.add_color_pixel(idx % sw, idx / sw, r_u8, g_u8, b_u8);
            }
        }
    }

    /// Renders 256-bin R, G, B, and Luma histogram curves and returns the computed [`HistogramData`].
    pub fn render_histogram<'a>(
        &mut self,
        input: impl Into<ScopeInput<'a>>,
        standard: ScopeStandard,
    ) -> HistogramData {
        self.clear(BG_COLOR);
        let hist = compute_histogram(input, standard);
        if self.width == 0 || self.height == 0 || hist.total_pixels == 0 {
            return hist;
        }

        // Quarter-level vertical graticule lines.
        for &bin in &[64usize, 128, 192] {
            let x = (bin * (self.width - 1)) / 255;
            self.draw_vline(x, 0, self.height - 1, GRATICULE_MINOR);
        }
        self.draw_hline(0, self.width - 1, self.height - 1, GRATICULE_MAJOR);

        let mut peak = 1u32;
        for i in 0..256 {
            peak = peak
                .max(hist.r[i])
                .max(hist.g[i])
                .max(hist.b[i])
                .max(hist.luma[i]);
        }
        let sw = self.width;
        let sh = self.height;
        let inv_peak = 1.0 / peak as f32;
        let max_y = (sh - 1) as f32;

        let bin_y = |count: u32| -> usize {
            let h_norm = (count as f32 * inv_peak).clamp(0.0, 1.0);
            (roundf(max_y - h_norm * max_y) as usize).min(sh - 1)
        };

        // Soft additive fill under curves + crisp curve outline.
        let mut prev_r = bin_y(hist.r[0]);
        let mut prev_g = bin_y(hist.g[0]);
        let mut prev_b = bin_y(hist.b[0]);
        let mut prev_y = bin_y(hist.luma[0]);

        for sx in 0..sw {
            let bin = if sw > 1 { (sx * 255) / (sw - 1) } else { 0 };
            let yr = bin_y(hist.r[bin]);
            let yg = bin_y(hist.g[bin]);
            let yb = bin_y(hist.b[bin]);
            let yy = bin_y(hist.luma[bin]);

            for y in yr..sh {
                self.add_color_pixel(sx, y, 55, 0, 0);
            }
            for y in yg..sh {
                self.add_color_pixel(sx, y, 0, 55, 0);
            }
            for y in yb..sh {
                self.add_color_pixel(sx, y, 0, 0, 55);
            }

            if sx > 0 {
                self.draw_line((sx - 1) as isize, prev_r as isize, sx as isize, yr as isize, 0xFFFF_4040);
                self.draw_line((sx - 1) as isize, prev_g as isize, sx as isize, yg as isize, 0xFF40_FF55);
                self.draw_line((sx - 1) as isize, prev_b as isize, sx as isize, yb as isize, 0xFF40_90FF);
                self.draw_line((sx - 1) as isize, prev_y as isize, sx as isize, yy as isize, 0xFFE8_ECF2);
            } else {
                self.put_pixel(sx, yr, 0xFFFF_4040);
                self.put_pixel(sx, yg, 0xFF40_FF55);
                self.put_pixel(sx, yb, 0xFF40_90FF);
                self.put_pixel(sx, yy, 0xFFE8_ECF2);
            }
            prev_r = yr;
            prev_g = yg;
            prev_b = yb;
            prev_y = yy;
        }

        hist
    }
}
