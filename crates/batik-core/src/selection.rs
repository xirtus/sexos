// SPDX-License-Identifier: GPL-3.0-only
//! 8-bit feathered selection masks and selection geometry.

use alloc::vec;
use alloc::vec::Vec;
use crate::blend::split_argb;
use crate::error::BatikError;
use crate::geom::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionCombine {
    Replace,
    Add,
    Subtract,
    Intersect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionMask {
    pub width: usize,
    pub height: usize,
    pub mask: Vec<u8>,
    pub active: bool,
}

impl SelectionMask {
    pub fn new(width: usize, height: usize) -> Result<Self, BatikError> {
        let len = width.checked_mul(height).ok_or(BatikError::TooLarge)?;
        let mut mask = Vec::new();
        mask.try_reserve_exact(len).map_err(|_| BatikError::OutOfMemory)?;
        mask.resize(len, 0);
        Ok(Self {
            width,
            height,
            mask,
            active: false,
        })
    }

    #[inline(always)]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[inline(always)]
    pub fn sample(&self, x: usize, y: usize) -> u8 {
        if !self.active {
            255
        } else if x < self.width && y < self.height {
            self.mask[y * self.width + x]
        } else {
            0
        }
    }

    pub fn select_all(&mut self) {
        self.mask.fill(255);
        self.active = true;
    }

    pub fn select_none(&mut self) {
        self.mask.fill(0);
        self.active = false;
    }

    pub fn invert(&mut self) {
        if !self.active {
            self.select_all();
            return;
        }
        for byte in self.mask.iter_mut() {
            *byte = 255 - *byte;
        }
    }

    fn combine_candidate(&mut self, candidate: &[u8], combine: SelectionCombine) {
        if !self.active && combine != SelectionCombine::Replace {
            // When no selection is active, treat backdrop as fully selected
            self.mask.fill(255);
        }
        match combine {
            SelectionCombine::Replace => {
                self.mask.copy_from_slice(candidate);
            }
            SelectionCombine::Add => {
                for (dst, &src) in self.mask.iter_mut().zip(candidate.iter()) {
                    *dst = dst.saturating_add(src);
                }
            }
            SelectionCombine::Subtract => {
                for (dst, &src) in self.mask.iter_mut().zip(candidate.iter()) {
                    *dst = dst.saturating_sub(src);
                }
            }
            SelectionCombine::Intersect => {
                for (dst, &src) in self.mask.iter_mut().zip(candidate.iter()) {
                    *dst = ((*dst as u16 * src as u16) / 255) as u8;
                }
            }
        }
        self.active = true;
    }

    pub fn select_rect(&mut self, rect: Rect, combine: SelectionCombine) {
        let mut candidate = vec![0u8; self.mask.len()];
        let clamped = match rect.clamp_to_bounds(self.width, self.height) {
            Some(r) => r,
            None => {
                self.combine_candidate(&candidate, combine);
                return;
            }
        };

        for y in (clamped.y as usize)..((clamped.y + clamped.height as i32) as usize) {
            let row_start = y * self.width;
            for x in (clamped.x as usize)..((clamped.x + clamped.width as i32) as usize) {
                candidate[row_start + x] = 255;
            }
        }
        self.combine_candidate(&candidate, combine);
    }

    pub fn select_ellipse(&mut self, rect: Rect, combine: SelectionCombine) {
        let mut candidate = vec![0u8; self.mask.len()];
        if rect.is_empty() {
            self.combine_candidate(&candidate, combine);
            return;
        }

        let cx = rect.x as f32 + (rect.width as f32) / 2.0;
        let cy = rect.y as f32 + (rect.height as f32) / 2.0;
        let rx = (rect.width as f32) / 2.0;
        let ry = (rect.height as f32) / 2.0;

        if rx <= 0.0 || ry <= 0.0 {
            self.combine_candidate(&candidate, combine);
            return;
        }

        let clamped = match rect.clamp_to_bounds(self.width, self.height) {
            Some(r) => r,
            None => {
                self.combine_candidate(&candidate, combine);
                return;
            }
        };

        for y in (clamped.y as usize)..((clamped.y + clamped.height as i32) as usize) {
            let dy = (y as f32 + 0.5 - cy) / ry;
            let dy2 = dy * dy;
            if dy2 > 1.0 {
                continue;
            }
            let row_start = y * self.width;
            for x in (clamped.x as usize)..((clamped.x + clamped.width as i32) as usize) {
                let dx = (x as f32 + 0.5 - cx) / rx;
                if dx * dx + dy2 <= 1.0 {
                    candidate[row_start + x] = 255;
                }
            }
        }
        self.combine_candidate(&candidate, combine);
    }

    pub fn select_polygon(&mut self, points: &[(i32, i32)], combine: SelectionCombine) {
        let mut candidate = vec![0u8; self.mask.len()];
        if points.len() < 3 {
            self.combine_candidate(&candidate, combine);
            return;
        }

        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for &(px, py) in points {
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        }

        let bound = Rect::new(
            min_x,
            min_y,
            (max_x - min_x + 1).max(0) as usize,
            (max_y - min_y + 1).max(0) as usize,
        );
        let clamped = match bound.clamp_to_bounds(self.width, self.height) {
            Some(r) => r,
            None => {
                self.combine_candidate(&candidate, combine);
                return;
            }
        };

        for y in (clamped.y as usize)..((clamped.y + clamped.height as i32) as usize) {
            let py = y as f32 + 0.5;
            let row_start = y * self.width;
            for x in (clamped.x as usize)..((clamped.x + clamped.width as i32) as usize) {
                let px = x as f32 + 0.5;
                let mut inside = false;
                let mut j = points.len() - 1;
                for i in 0..points.len() {
                    let (xi, yi) = (points[i].0 as f32, points[i].1 as f32);
                    let (xj, yj) = (points[j].0 as f32, points[j].1 as f32);
                    let intersect = ((yi > py) != (yj > py))
                        && (px < (xj - xi) * (py - yi) / (yj - yi) + xi);
                    if intersect {
                        inside = !inside;
                    }
                    j = i;
                }
                if inside {
                    candidate[row_start + x] = 255;
                }
            }
        }
        self.combine_candidate(&candidate, combine);
    }

    pub fn magic_wand(
        &mut self,
        pixels: &[u32],
        img_w: usize,
        img_h: usize,
        seed_x: usize,
        seed_y: usize,
        tolerance: u8,
        contiguous: bool,
        combine: SelectionCombine,
    ) {
        if img_w == 0 || img_h == 0 || seed_x >= img_w || seed_y >= img_h || img_w != self.width || img_h != self.height || pixels.len() < img_w * img_h {
            return;
        }
        let target_color = pixels[seed_y * img_w + seed_x];
        let (ta, tr, tg, tb) = split_argb(target_color);

        let matches = |c: u32| -> bool {
            let (a, r, g, b) = split_argb(c);
            let da = (a as i16 - ta as i16).abs();
            let dr = (r as i16 - tr as i16).abs();
            let dg = (g as i16 - tg as i16).abs();
            let db = (b as i16 - tb as i16).abs();
            let max_diff = da.max(dr).max(dg).max(db);
            max_diff <= (tolerance as i16)
        };

        let mut candidate = Vec::new();
        if candidate.try_reserve_exact(self.mask.len()).is_err() {
            return;
        }
        candidate.resize(self.mask.len(), 0);

        if !contiguous {
            for (idx, &p) in pixels[..self.mask.len()].iter().enumerate() {
                if matches(p) {
                    candidate[idx] = 255;
                }
            }
        } else {
            // Stack-bounded flood fill without recursion
            let mut visited = Vec::new();
            if visited.try_reserve_exact(self.mask.len()).is_err() {
                return;
            }
            visited.resize(self.mask.len(), false);

            let mut stack = Vec::new();
            stack.push((seed_x, seed_y));
            visited[seed_y * self.width + seed_x] = true;

            while let Some((cx, cy)) = stack.pop() {
                let idx = cy * self.width + cx;
                candidate[idx] = 255;

                // 4-connected neighbors
                let neighbors = [
                    (cx.wrapping_sub(1), cy, cx > 0),
                    (cx + 1, cy, cx + 1 < self.width),
                    (cx, cy.wrapping_sub(1), cy > 0),
                    (cx, cy + 1, cy + 1 < self.height),
                ];

                for (nx, ny, valid) in neighbors {
                    if valid {
                        let nidx = ny * self.width + nx;
                        if !visited[nidx] {
                            visited[nidx] = true;
                            if matches(pixels[nidx]) {
                                stack.push((nx, ny));
                            }
                        }
                    }
                }
            }
        }

        self.combine_candidate(&candidate, combine);
    }

    /// Feathers the selection mask using a separable box blur approximation of Gaussian blur.
    pub fn feather(&mut self, radius: usize) {
        if radius == 0 || !self.active || self.width == 0 || self.height == 0 || self.mask.len() < self.width * self.height {
            return;
        }

        let w = self.width;
        let h = self.height;
        let mut temp = Vec::new();
        if temp.try_reserve_exact(self.mask.len()).is_err() {
            return;
        }
        temp.resize(self.mask.len(), 0);

        // Perform 2 passes of box blur for a smooth curve
        for _ in 0..2 {
            // Horizontal blur
            for y in 0..h {
                let row_offset = y * w;
                let mut sum: u32 = 0;
                let window_size = 2 * radius + 1;

                // Initialize window
                for x in 0..=radius {
                    sum += self.mask[row_offset + x.min(w - 1)] as u32;
                }
                for _ in 0..radius {
                    sum += self.mask[row_offset] as u32;
                }

                for x in 0..w {
                    temp[row_offset + x] = (sum / window_size as u32) as u8;
                    let left_x = (x as isize - radius as isize).max(0) as usize;
                    let right_x = (x + radius + 1).min(w - 1);
                    sum = sum.saturating_sub(self.mask[row_offset + left_x] as u32)
                        .saturating_add(self.mask[row_offset + right_x] as u32);
                }
            }

            // Vertical blur
            for x in 0..w {
                let mut sum: u32 = 0;
                let window_size = 2 * radius + 1;

                for y in 0..=radius {
                    sum += temp[y.min(h - 1) * w + x] as u32;
                }
                for _ in 0..radius {
                    sum += temp[x] as u32;
                }

                for y in 0..h {
                    self.mask[y * w + x] = (sum / window_size as u32) as u8;
                    let top_y = (y as isize - radius as isize).max(0) as usize;
                    let bot_y = (y + radius + 1).min(h - 1);
                    sum = sum.saturating_sub(temp[top_y * w + x] as u32)
                        .saturating_add(temp[bot_y * w + x] as u32);
                }
            }
        }
    }
}
