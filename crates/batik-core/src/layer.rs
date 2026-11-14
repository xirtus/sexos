// SPDX-License-Identifier: GPL-3.0-only
//! Layer representations: raster, adjustment, and group layers.

use alloc::string::String;
use alloc::vec::Vec;
use sex_image::Limits;

use crate::admit_limits;
use crate::adjust::Adjustment;
use crate::blend::BlendMode;
use crate::error::BatikError;
use crate::geom::Rect;

#[derive(Clone, Debug, PartialEq)]
pub struct RasterLayer {
    pub width: usize,
    pub height: usize,
    pub offset_x: i32,
    pub offset_y: i32,
    pub pixels: Vec<u32>,
    pub mask: Option<Vec<u8>>,
    pub alpha_lock: bool,
    pub pixel_lock: bool,
}

impl RasterLayer {
    pub fn new(width: usize, height: usize, limits: &Limits) -> Result<Self, BatikError> {
        let total_pixels = admit_limits(limits, width, height)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(total_pixels).map_err(|_| BatikError::OutOfMemory)?;
        pixels.resize(total_pixels, 0);

        Ok(Self {
            width,
            height,
            offset_x: 0,
            offset_y: 0,
            pixels,
            mask: None,
            alpha_lock: false,
            pixel_lock: false,
        })
    }

    pub fn with_pixels(
        width: usize,
        height: usize,
        pixels: Vec<u32>,
        limits: &Limits,
    ) -> Result<Self, BatikError> {
        let expected = admit_limits(limits, width, height)?;
        if pixels.len() != expected {
            return Err(BatikError::InvalidDimensions);
        }

        Ok(Self {
            width,
            height,
            offset_x: 0,
            offset_y: 0,
            pixels,
            mask: None,
            alpha_lock: false,
            pixel_lock: false,
        })
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(self.offset_x, self.offset_y, self.width, self.height)
    }

    pub fn pixel_at(&self, x: i32, y: i32) -> Option<u32> {
        let lx = x.checked_sub(self.offset_x)?;
        let ly = y.checked_sub(self.offset_y)?;
        if lx >= 0 && ly >= 0 && (lx as usize) < self.width && (ly as usize) < self.height {
            let idx = (ly as usize).checked_mul(self.width)?.checked_add(lx as usize)?;
            self.pixels.get(idx).copied()
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdjustmentLayer {
    pub adjustment: Adjustment,
    pub mask: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupLayer {
    pub layers: Vec<Layer>,
    pub pass_through: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayerKind {
    Raster(RasterLayer),
    Adjustment(AdjustmentLayer),
    Group(GroupLayer),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub opacity: u8,
    pub blend_mode: BlendMode,
    pub kind: LayerKind,
}

impl Layer {
    pub fn new_raster(
        id: u64,
        name: String,
        raster: RasterLayer,
        blend_mode: BlendMode,
        opacity: u8,
    ) -> Self {
        Self {
            id,
            name,
            visible: true,
            opacity,
            blend_mode,
            kind: LayerKind::Raster(raster),
        }
    }

    pub fn new_adjustment(
        id: u64,
        name: String,
        adjustment: AdjustmentLayer,
        blend_mode: BlendMode,
        opacity: u8,
    ) -> Self {
        Self {
            id,
            name,
            visible: true,
            opacity,
            blend_mode,
            kind: LayerKind::Adjustment(adjustment),
        }
    }

    pub fn new_group(
        id: u64,
        name: String,
        group: GroupLayer,
        blend_mode: BlendMode,
        opacity: u8,
    ) -> Self {
        Self {
            id,
            name,
            visible: true,
            opacity,
            blend_mode,
            kind: LayerKind::Group(group),
        }
    }
}
