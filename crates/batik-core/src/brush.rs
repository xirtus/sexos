// SPDX-License-Identifier: GPL-3.0-only
//! Pressure-sensitive brush engine, eraser, eyedropper, and stack-bounded flood fill.

use alloc::vec::Vec;
use crate::blend::{blend_pixel, pack_argb, split_argb, BlendMode};
use crate::document::Document;
use crate::error::BatikError;
use crate::geom::Rect;
use crate::layer::{LayerKind, RasterLayer};
use crate::selection::SelectionMask;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerSample {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
}

impl PointerSample {
    pub const fn new(x: f32, y: f32, pressure: f32) -> Self {
        Self { x, y, pressure }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushPreset {
    SoftAirbrush,
    HardRound,
    Calligraphy,
    Eraser,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Brush {
    pub size: f32,
    pub color: u32,
    pub opacity: f32,
    pub flow: f32,
    pub hardness: f32,
    pub spacing: f32,
    pub eraser: bool,
    pub size_dynamics: bool,
    pub opacity_dynamics: bool,
    pub chisel_angle: f32,
    pub chisel_ratio: f32,
}

impl Brush {
    pub fn preset(p: BrushPreset) -> Self {
        match p {
            BrushPreset::SoftAirbrush => Self {
                size: 24.0,
                color: 0xFF00_0000,
                opacity: 0.8,
                flow: 0.4,
                hardness: 0.05,
                spacing: 0.1,
                eraser: false,
                size_dynamics: true,
                opacity_dynamics: true,
                chisel_angle: 0.0,
                chisel_ratio: 1.0,
            },
            BrushPreset::HardRound => Self {
                size: 16.0,
                color: 0xFF00_0000,
                opacity: 1.0,
                flow: 1.0,
                hardness: 0.95,
                spacing: 0.15,
                eraser: false,
                size_dynamics: true,
                opacity_dynamics: false,
                chisel_angle: 0.0,
                chisel_ratio: 1.0,
            },
            BrushPreset::Calligraphy => Self {
                size: 20.0,
                color: 0xFF00_0000,
                opacity: 1.0,
                flow: 1.0,
                hardness: 0.9,
                spacing: 0.08,
                eraser: false,
                size_dynamics: true,
                opacity_dynamics: false,
                chisel_angle: 0.785398, // 45 degrees
                chisel_ratio: 0.25,
            },
            BrushPreset::Eraser => Self {
                size: 20.0,
                color: 0xFF00_0000,
                opacity: 1.0,
                flow: 1.0,
                hardness: 0.8,
                spacing: 0.15,
                eraser: true,
                size_dynamics: true,
                opacity_dynamics: false,
                chisel_angle: 0.0,
                chisel_ratio: 1.0,
            },
        }
    }

    pub fn from_preset(p: BrushPreset) -> Self {
        Self::preset(p)
    }

    pub fn effective_size(&self, pressure: f32) -> f32 {
        let base_size = if self.size.is_finite() && self.size > 0.0 {
            self.size
        } else {
            1.0
        };
        let p = if pressure.is_finite() {
            pressure.clamp(0.05, 1.0)
        } else {
            1.0
        };
        if self.size_dynamics {
            (base_size * p).max(1.0)
        } else {
            base_size.max(1.0)
        }
    }

    pub fn effective_opacity(&self, pressure: f32) -> f32 {
        let base_opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let p = if pressure.is_finite() {
            pressure.clamp(0.05, 1.0)
        } else {
            1.0
        };
        if self.opacity_dynamics {
            (base_opacity * p).clamp(0.0, 1.0)
        } else {
            base_opacity
        }
    }

    pub fn effective_flow(&self, pressure: f32) -> f32 {
        let base_flow = if self.flow.is_finite() {
            self.flow.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let p = if pressure.is_finite() {
            pressure.clamp(0.05, 1.0)
        } else {
            1.0
        };
        if self.opacity_dynamics {
            (base_flow * p).clamp(0.0, 1.0)
        } else {
            base_flow
        }
    }
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            size: 20.0,
            color: 0xFF00_0000, // opaque black
            opacity: 1.0,
            flow: 1.0,
            hardness: 0.8,
            spacing: 0.2,
            eraser: false,
            size_dynamics: true,
            opacity_dynamics: false,
            chisel_angle: 0.0,
            chisel_ratio: 1.0,
        }
    }
}

pub struct StrokeEngine {
    pub brush: Brush,
    last_sample: Option<PointerSample>,
}

impl StrokeEngine {
    pub fn new(brush: Brush) -> Self {
        Self {
            brush,
            last_sample: None,
        }
    }

    pub fn paint_to(
        &mut self,
        raster: &mut RasterLayer,
        selection: &SelectionMask,
        sample: PointerSample,
    ) -> Result<(), BatikError> {
        if raster.pixel_lock {
            return Err(BatikError::LayerLocked);
        }
        if !sample.x.is_finite() || !sample.y.is_finite() || !sample.pressure.is_finite() {
            return Ok(());
        }

        let prev = match self.last_sample {
            Some(p) => p,
            None => {
                self.render_dab(raster, selection, sample);
                self.last_sample = Some(sample);
                return Ok(());
            }
        };

        let dx = sample.x - prev.x;
        let dy = sample.y - prev.y;
        let dist = libm::sqrtf(dx * dx + dy * dy);
        if !dist.is_finite() || dist <= 0.0 {
            return Ok(());
        }

        let eff_size = self.effective_size(sample.pressure);
        let spacing = if self.brush.spacing.is_finite() && self.brush.spacing > 0.0 {
            self.brush.spacing.max(0.05)
        } else {
            0.1
        };
        let step = (eff_size * spacing).max(1.0);

        if dist < step {
            return Ok(());
        }

        let num_steps = (libm::floorf(dist / step) as usize).min(2048);
        for i in 1..=num_steps {
            let t = (i as f32 * step) / dist;
            let interp = PointerSample {
                x: prev.x + t * dx,
                y: prev.y + t * dy,
                pressure: prev.pressure + t * (sample.pressure - prev.pressure),
            };
            self.render_dab(raster, selection, interp);
        }

        self.last_sample = Some(sample);
        Ok(())
    }

    pub fn finish_stroke(&mut self) {
        self.last_sample = None;
    }

    /// Paints into the layer's 8-bit mask buffer instead of RGB pixels.
    pub fn paint_mask_to(
        &mut self,
        raster: &mut RasterLayer,
        selection: &SelectionMask,
        sample: PointerSample,
        mask_val: u8,
    ) -> Result<(), BatikError> {
        if raster.pixel_lock {
            return Err(BatikError::LayerLocked);
        }
        if raster.mask.is_none() {
            return Ok(());
        }
        if !sample.x.is_finite() || !sample.y.is_finite() || !sample.pressure.is_finite() {
            return Ok(());
        }

        let prev = match self.last_sample {
            Some(p) => p,
            None => {
                self.render_mask_dab(raster, selection, sample, mask_val);
                self.last_sample = Some(sample);
                return Ok(());
            }
        };

        let dx = sample.x - prev.x;
        let dy = sample.y - prev.y;
        let dist = libm::sqrtf(dx * dx + dy * dy);
        if !dist.is_finite() || dist <= 0.0 {
            return Ok(());
        }

        let eff_size = self.effective_size(sample.pressure);
        let spacing = if self.brush.spacing.is_finite() && self.brush.spacing > 0.0 {
            self.brush.spacing.max(0.05)
        } else {
            0.1
        };
        let step = (eff_size * spacing).max(1.0);

        if dist < step {
            return Ok(());
        }

        let num_steps = (libm::floorf(dist / step) as usize).min(2048);
        for i in 1..=num_steps {
            let t = (i as f32 * step) / dist;
            let interp = PointerSample {
                x: prev.x + t * dx,
                y: prev.y + t * dy,
                pressure: prev.pressure + t * (sample.pressure - prev.pressure),
            };
            self.render_mask_dab(raster, selection, interp, mask_val);
        }

        self.last_sample = Some(sample);
        Ok(())
    }

    pub fn effective_size(&self, pressure: f32) -> f32 {
        self.brush.effective_size(pressure)
    }

    pub fn effective_opacity(&self, pressure: f32) -> f32 {
        self.brush.effective_opacity(pressure)
    }

    pub fn effective_flow(&self, pressure: f32) -> f32 {
        self.brush.effective_flow(pressure)
    }

    pub fn render_dab(
        &self,
        raster: &mut RasterLayer,
        selection: &SelectionMask,
        sample: PointerSample,
    ) {
        if !sample.x.is_finite() || !sample.y.is_finite() || !sample.pressure.is_finite() {
            return;
        }
        if raster.width == 0 || raster.height == 0 {
            return;
        }
        let size = self.effective_size(sample.pressure);
        let radius = size / 2.0;
        let opacity = self.effective_opacity(sample.pressure);
        let flow = self.effective_flow(sample.pressure);

        let lx = sample.x - raster.offset_x as f32;
        let ly = sample.y - raster.offset_y as f32;

        let min_x = libm::floorf(lx - radius).max(0.0) as usize;
        let max_x = libm::ceilf(lx + radius).min((raster.width.saturating_sub(1)) as f32) as usize;
        let min_y = libm::floorf(ly - radius).max(0.0) as usize;
        let max_y = libm::ceilf(ly + radius).min((raster.height.saturating_sub(1)) as f32) as usize;

        if min_x >= raster.width || min_y >= raster.height || min_x > max_x || min_y > max_y {
            return;
        }

        let hardness = if self.brush.hardness.is_finite() {
            self.brush.hardness.clamp(0.0, 1.0)
        } else {
            0.8
        };
        let inner_radius = radius * hardness;

        let is_chisel = self.brush.chisel_ratio < 0.99 && self.brush.chisel_ratio > 0.01;
        let cos_a = if is_chisel { libm::cosf(self.brush.chisel_angle) } else { 1.0 };
        let sin_a = if is_chisel { libm::sinf(self.brush.chisel_angle) } else { 0.0 };

        for py in min_y..=max_y {
            let dy = py as f32 + 0.5 - ly;
            let dy2 = dy * dy;
            let row_offset = py * raster.width;

            for px in min_x..=max_x {
                let dx = px as f32 + 0.5 - lx;
                let d = if is_chisel {
                    let rx = dx * cos_a + dy * sin_a;
                    let ry = -dx * sin_a + dy * cos_a;
                    libm::sqrtf(rx * rx + (ry / self.brush.chisel_ratio) * (ry / self.brush.chisel_ratio))
                } else {
                    libm::sqrtf(dx * dx + dy2)
                };
                if d > radius {
                    continue;
                }

                let falloff = if hardness >= 1.0 || d <= inner_radius {
                    1.0
                } else {
                    let t = (d - inner_radius) / (radius - inner_radius);
                    0.5 * (1.0 + libm::cosf(3.14159265 * t))
                };

                let doc_x = (px as i32 + raster.offset_x).max(0) as usize;
                let doc_y = (py as i32 + raster.offset_y).max(0) as usize;
                let mask_factor = selection.sample(doc_x, doc_y) as f32 / 255.0;

                let dab_weight = falloff * opacity * flow * mask_factor;
                if dab_weight <= 0.0 {
                    continue;
                }

                let idx = row_offset + px;
                let dest = raster.pixels[idx];

                if self.brush.eraser {
                    if raster.alpha_lock {
                        continue;
                    }
                    let (da, dr, dg, db) = split_argb(dest);
                    let new_a = libm::roundf(da as f32 * (1.0 - dab_weight)).clamp(0.0, 255.0) as u8;
                    raster.pixels[idx] = pack_argb(new_a, dr, dg, db);
                } else {
                    let (_sa_raw, sr, sg, sb) = split_argb(self.brush.color);
                    let (da, dr, dg, db) = split_argb(dest);

                    if raster.alpha_lock {
                        let eff = dab_weight;
                        let nr = libm::roundf(dr as f32 + eff * (sr as f32 - dr as f32)).clamp(0.0, 255.0) as u8;
                        let ng = libm::roundf(dg as f32 + eff * (sg as f32 - dg as f32)).clamp(0.0, 255.0) as u8;
                        let nb = libm::roundf(db as f32 + eff * (sb as f32 - db as f32)).clamp(0.0, 255.0) as u8;
                        raster.pixels[idx] = pack_argb(da, nr, ng, nb);
                    } else {
                        let brush_opacity_u8 = libm::roundf(dab_weight * 255.0).clamp(0.0, 255.0) as u8;
                        raster.pixels[idx] = blend_pixel(
                            dest,
                            self.brush.color,
                            BlendMode::Normal,
                            brush_opacity_u8,
                            255,
                        );
                    }
                }
            }
        }
    }

    pub fn render_mask_dab(
        &self,
        raster: &mut RasterLayer,
        selection: &SelectionMask,
        sample: PointerSample,
        mask_val: u8,
    ) {
        if !sample.x.is_finite() || !sample.y.is_finite() || !sample.pressure.is_finite() {
            return;
        }
        if raster.width == 0 || raster.height == 0 || raster.mask.is_none() {
            return;
        }
        let size = self.effective_size(sample.pressure);
        let radius = size / 2.0;
        let opacity = self.effective_opacity(sample.pressure);
        let flow = self.effective_flow(sample.pressure);

        let lx = sample.x - raster.offset_x as f32;
        let ly = sample.y - raster.offset_y as f32;

        let min_x = libm::floorf(lx - radius).max(0.0) as usize;
        let max_x = libm::ceilf(lx + radius).min((raster.width.saturating_sub(1)) as f32) as usize;
        let min_y = libm::floorf(ly - radius).max(0.0) as usize;
        let max_y = libm::ceilf(ly + radius).min((raster.height.saturating_sub(1)) as f32) as usize;

        if min_x >= raster.width || min_y >= raster.height || min_x > max_x || min_y > max_y {
            return;
        }

        let hardness = if self.brush.hardness.is_finite() {
            self.brush.hardness.clamp(0.0, 1.0)
        } else {
            0.8
        };
        let inner_radius = radius * hardness;

        let is_chisel = self.brush.chisel_ratio < 0.99 && self.brush.chisel_ratio > 0.01;
        let cos_a = if is_chisel { libm::cosf(self.brush.chisel_angle) } else { 1.0 };
        let sin_a = if is_chisel { libm::sinf(self.brush.chisel_angle) } else { 0.0 };

        for py in min_y..=max_y {
            let dy = py as f32 + 0.5 - ly;
            let dy2 = dy * dy;
            let row_offset = py * raster.width;

            for px in min_x..=max_x {
                let dx = px as f32 + 0.5 - lx;
                let d = if is_chisel {
                    let rx = dx * cos_a + dy * sin_a;
                    let ry = -dx * sin_a + dy * cos_a;
                    libm::sqrtf(rx * rx + (ry / self.brush.chisel_ratio) * (ry / self.brush.chisel_ratio))
                } else {
                    libm::sqrtf(dx * dx + dy2)
                };
                if d > radius {
                    continue;
                }

                let falloff = if hardness >= 1.0 || d <= inner_radius {
                    1.0
                } else {
                    let t = (d - inner_radius) / (radius - inner_radius);
                    0.5 * (1.0 + libm::cosf(3.14159265 * t))
                };

                let doc_x = (px as i32 + raster.offset_x).max(0) as usize;
                let doc_y = (py as i32 + raster.offset_y).max(0) as usize;
                let mask_factor = selection.sample(doc_x, doc_y) as f32 / 255.0;

                let dab_weight = falloff * opacity * flow * mask_factor;
                if dab_weight <= 0.0 {
                    continue;
                }

                let idx = row_offset + px;
                let target = mask_val as f32;
                if let Some(ref mut mask) = raster.mask {
                    let cur = mask[idx] as f32;
                    let new_m = cur + dab_weight * (target - cur);
                    mask[idx] = libm::roundf(new_m).clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// Color picker (eyedropper) from document or specific layer.
pub fn eyedropper(
    doc: &Document,
    x: usize,
    y: usize,
    sample_merged: bool,
    layer_id: Option<u64>,
) -> Option<u32> {
    if x >= doc.width || y >= doc.height {
        return None;
    }

    if sample_merged {
        let mut buf = [0u32; 1];
        doc.render_region(Rect::new(x as i32, y as i32, 1, 1), &mut buf, 1);
        Some(buf[0])
    } else if let Some(id) = layer_id {
        let layer = doc.get_layer(id)?;
        if let LayerKind::Raster(ref raster) = layer.kind {
            raster.pixel_at(x as i32, y as i32)
        } else {
            None
        }
    } else {
        None
    }
}

/// Bounded scanline flood fill (zero recursion).
pub fn bucket_fill(
    raster: &mut RasterLayer,
    start_x: usize,
    start_y: usize,
    fill_color: u32,
    tolerance: u8,
    selection: &SelectionMask,
) -> Result<(), BatikError> {
    if raster.pixel_lock {
        return Err(BatikError::LayerLocked);
    }
    if start_x >= raster.width || start_y >= raster.height {
        return Ok(());
    }

    let target_color = raster.pixels[start_y * raster.width + start_x];
    if target_color == fill_color && tolerance == 0 {
        return Ok(());
    }

    let (ta, tr, tg, tb) = split_argb(target_color);
    let matches = |c: u32| -> bool {
        let (a, r, g, b) = split_argb(c);
        let da = (a as i16 - ta as i16).abs();
        let dr = (r as i16 - tr as i16).abs();
        let dg = (g as i16 - tg as i16).abs();
        let db = (b as i16 - tb as i16).abs();
        da.max(dr).max(dg).max(db) <= (tolerance as i16)
    };

    let w = raster.width;
    let h = raster.height;
    if w == 0 || h == 0 || raster.pixels.len() < w * h {
        return Ok(());
    }

    let total = w.checked_mul(h).ok_or(BatikError::TooLarge)?;
    let mut visited = Vec::new();
    visited.try_reserve_exact(total).map_err(|_| BatikError::OutOfMemory)?;
    visited.resize(total, false);

    // Stack of spans: (x_left, x_right, y, direction_y)
    let mut stack: Vec<(usize, usize, usize, i8)> = Vec::new();

    // Find initial span
    let mut x1 = start_x;
    while x1 > 0 && matches(raster.pixels[start_y * w + (x1 - 1)]) {
        x1 -= 1;
    }
    let mut x2 = start_x;
    while x2 + 1 < w && matches(raster.pixels[start_y * w + (x2 + 1)]) {
        x2 += 1;
    }

    stack.push((x1, x2, start_y, 1));
    stack.push((x1, x2, start_y, -1));

    while let Some((lx, rx, y, dy)) = stack.pop() {
        let row_start = y * w;
        for x in lx..=rx {
            let idx = row_start + x;
            if visited[idx] {
                continue;
            }
            visited[idx] = true;

            let doc_x = (x as i32 + raster.offset_x).max(0) as usize;
            let doc_y = (y as i32 + raster.offset_y).max(0) as usize;
            let m = selection.sample(doc_x, doc_y);
            if m == 0 {
                continue;
            }

            if raster.alpha_lock {
                let (da, _, _, _) = split_argb(raster.pixels[idx]);
                let (_, fr, fg, fb) = split_argb(fill_color);
                raster.pixels[idx] = pack_argb(da, fr, fg, fb);
            } else if m == 255 {
                raster.pixels[idx] = fill_color;
            } else {
                raster.pixels[idx] = blend_pixel(raster.pixels[idx], fill_color, BlendMode::Normal, 255, m);
            }
        }

        let next_y = y as isize + dy as isize;
        if next_y >= 0 && (next_y as usize) < h {
            let ny = next_y as usize;
            let mut x = lx;
            while x <= rx {
                if matches(raster.pixels[ny * w + x]) && !visited[ny * w + x] {
                    let mut nx1 = x;
                    while nx1 > 0 && matches(raster.pixels[ny * w + (nx1 - 1)]) && !visited[ny * w + (nx1 - 1)] {
                        nx1 -= 1;
                    }
                    let mut nx2 = x;
                    while nx2 + 1 < w && matches(raster.pixels[ny * w + (nx2 + 1)]) && !visited[ny * w + (nx2 + 1)] {
                        nx2 += 1;
                    }
                    stack.push((nx1, nx2, ny, dy));
                    if nx1 < lx || nx2 > rx {
                        stack.push((nx1, nx2, ny, -dy));
                    }
                    x = nx2 + 1;
                } else {
                    x += 1;
                }
            }
        }
    }

    Ok(())
}

/// Bounded scanline flood fill into layer mask buffer.
pub fn bucket_fill_mask(
    raster: &mut RasterLayer,
    start_x: usize,
    start_y: usize,
    fill_val: u8,
    tolerance: u8,
    selection: &SelectionMask,
) -> Result<(), BatikError> {
    if raster.pixel_lock {
        return Err(BatikError::LayerLocked);
    }
    let Some(ref mut mask) = raster.mask else {
        return Ok(());
    };
    if start_x >= raster.width || start_y >= raster.height {
        return Ok(());
    }
    let w = raster.width;
    let h = raster.height;
    if w == 0 || h == 0 || mask.len() < w * h {
        return Ok(());
    }
    let target_val = mask[start_y * w + start_x];
    if target_val == fill_val && tolerance == 0 {
        return Ok(());
    }
    let matches = |val: u8| -> bool {
        (val as i16 - target_val as i16).abs() <= (tolerance as i16)
    };
    let total = w.checked_mul(h).ok_or(BatikError::TooLarge)?;
    let mut visited = alloc::vec::Vec::new();
    visited.try_reserve_exact(total).map_err(|_| BatikError::OutOfMemory)?;
    visited.resize(total, false);
    let mut stack: alloc::vec::Vec<(usize, usize, usize, i8)> = alloc::vec::Vec::new();
    let mut x1 = start_x;
    while x1 > 0 && matches(mask[start_y * w + (x1 - 1)]) {
        x1 -= 1;
    }
    let mut x2 = start_x;
    while x2 + 1 < w && matches(mask[start_y * w + (x2 + 1)]) {
        x2 += 1;
    }
    stack.push((x1, x2, start_y, 1));
    stack.push((x1, x2, start_y, -1));
    while let Some((lx, rx, y, dy)) = stack.pop() {
        let row_start = y * w;
        for x in lx..=rx {
            let idx = row_start + x;
            if visited[idx] {
                continue;
            }
            visited[idx] = true;
            let doc_x = (x as i32 + raster.offset_x).max(0) as usize;
            let doc_y = (y as i32 + raster.offset_y).max(0) as usize;
            let m = selection.sample(doc_x, doc_y);
            if m == 0 {
                continue;
            }
            if m == 255 {
                mask[idx] = fill_val;
            } else {
                let cur = mask[idx] as f32;
                let w = m as f32 / 255.0;
                let res = cur + w * (fill_val as f32 - cur);
                mask[idx] = libm::roundf(res).clamp(0.0, 255.0) as u8;
            }
        }
        let next_y = y as isize + dy as isize;
        if next_y >= 0 && (next_y as usize) < h {
            let ny = next_y as usize;
            let mut x = lx;
            while x <= rx {
                if matches(mask[ny * w + x]) && !visited[ny * w + x] {
                    let mut nx1 = x;
                    while nx1 > 0 && matches(mask[ny * w + (nx1 - 1)]) && !visited[ny * w + (nx1 - 1)] {
                        nx1 -= 1;
                    }
                    let mut nx2 = x;
                    while nx2 + 1 < w && matches(mask[ny * w + (nx2 + 1)]) && !visited[ny * w + (nx2 + 1)] {
                        nx2 += 1;
                    }
                    stack.push((nx1, nx2, ny, dy));
                    if nx1 < lx || nx2 > rx {
                        stack.push((nx1, nx2, ny, -dy));
                    }
                    x = nx2 + 1;
                } else {
                    x += 1;
                }
            }
        }
    }
    Ok(())
}

