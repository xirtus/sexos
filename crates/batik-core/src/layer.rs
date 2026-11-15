// SPDX-License-Identifier: GPL-3.0-only
//! Layer representations: raster, adjustment, and group layers.

use alloc::string::String;
use alloc::vec::Vec;
use sex_image::Limits;

use crate::admit_limits;
use crate::adjust::AdjustmentKind;
use crate::blend::{pack_argb, split_argb, BlendMode};
use crate::error::BatikError;
use crate::filters::FilterNode;
use crate::geom::Rect;
use crate::selection::SelectionMask;
use crate::text::TextLayer;

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

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [u32] {
        &mut self.pixels
    }

    pub fn mask(&self) -> Option<&[u8]> {
        self.mask.as_deref()
    }

    pub fn mask_mut(&mut self) -> Option<&mut [u8]> {
        self.mask.as_deref_mut()
    }

    /// Adds an 8-bit layer mask buffer initialized to `default_val` (e.g. 255 for reveal-all, 0 for hide-all).
    pub fn add_mask(&mut self, default_val: u8) -> Result<(), BatikError> {
        let total = self.width.checked_mul(self.height).ok_or(BatikError::TooLarge)?;
        let mut mask = Vec::new();
        mask.try_reserve_exact(total).map_err(|_| BatikError::OutOfMemory)?;
        mask.resize(total, default_val);
        self.mask = Some(mask);
        Ok(())
    }

    /// Initializes or replaces the layer mask from an active selection mask.
    pub fn mask_from_selection(&mut self, selection: &SelectionMask) -> Result<(), BatikError> {
        let total = self.width.checked_mul(self.height).ok_or(BatikError::TooLarge)?;
        let mut mask = Vec::new();
        mask.try_reserve_exact(total).map_err(|_| BatikError::OutOfMemory)?;
        for y in 0..self.height {
            let doc_y = (y as i32 + self.offset_y).max(0) as usize;
            for x in 0..self.width {
                let doc_x = (x as i32 + self.offset_x).max(0) as usize;
                mask.push(selection.sample(doc_x, doc_y));
            }
        }
        self.mask = Some(mask);
        Ok(())
    }

    /// Inverts the current layer mask (0 becomes 255, 255 becomes 0).
    pub fn invert_mask(&mut self) {
        if let Some(ref mut mask) = self.mask {
            for m in mask.iter_mut() {
                *m = 255 - *m;
            }
        }
    }

    /// Bakes the layer mask into the layer's alpha channel and removes the mask.
    pub fn apply_mask(&mut self) {
        if let Some(mask) = self.mask.take() {
            for (p, &m) in self.pixels.iter_mut().zip(mask.iter()) {
                let (a, r, g, b) = split_argb(*p);
                let new_a = ((a as u16 * m as u16) / 255) as u8;
                *p = pack_argb(new_a, r, g, b);
            }
        }
    }

    /// Deletes the layer mask without modifying layer pixels.
    pub fn delete_mask(&mut self) {
        self.mask = None;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdjustmentLayer {
    pub kind: AdjustmentKind,
    pub mask: Option<Vec<u8>>,
}

impl AdjustmentLayer {
    pub fn new(kind: AdjustmentKind) -> Self {
        Self { kind, mask: None }
    }

    pub fn with_mask(kind: AdjustmentKind, mask: Vec<u8>) -> Self {
        Self { kind, mask: Some(mask) }
    }

    pub fn apply_pixel(&self, pixel: u32) -> u32 {
        self.kind.apply_pixel(pixel)
    }

    pub fn mask(&self) -> Option<&[u8]> {
        self.mask.as_deref()
    }

    pub fn mask_mut(&mut self) -> Option<&mut [u8]> {
        self.mask.as_deref_mut()
    }
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
    Text(TextLayer),
    Group(GroupLayer),
}

impl From<AdjustmentKind> for LayerKind {
    fn from(kind: AdjustmentKind) -> Self {
        LayerKind::Adjustment(AdjustmentLayer::new(kind))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub opacity: u8,
    pub blend_mode: BlendMode,
    pub kind: LayerKind,
    pub filter_stack: Vec<FilterNode>,
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
            filter_stack: Vec::new(),
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
            filter_stack: Vec::new(),
        }
    }

    pub fn new_text(
        id: u64,
        name: String,
        text: TextLayer,
        blend_mode: BlendMode,
        opacity: u8,
    ) -> Self {
        Self {
            id,
            name,
            visible: true,
            opacity,
            blend_mode,
            kind: LayerKind::Text(text),
            filter_stack: Vec::new(),
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
            filter_stack: Vec::new(),
        }
    }

    pub fn raster(&self) -> Option<&RasterLayer> {
        match &self.kind {
            LayerKind::Raster(ref r) => Some(r),
            _ => None,
        }
    }

    pub fn raster_mut(&mut self) -> Option<&mut RasterLayer> {
        match &mut self.kind {
            LayerKind::Raster(ref mut r) => Some(r),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&TextLayer> {
        match &self.kind {
            LayerKind::Text(ref t) => Some(t),
            _ => None,
        }
    }

    pub fn text_mut(&mut self) -> Option<&mut TextLayer> {
        match &mut self.kind {
            LayerKind::Text(ref mut t) => Some(t),
            _ => None,
        }
    }

    pub fn adjustment(&self) -> Option<&AdjustmentLayer> {
        match &self.kind {
            LayerKind::Adjustment(ref a) => Some(a),
            _ => None,
        }
    }

    pub fn adjustment_mut(&mut self) -> Option<&mut AdjustmentLayer> {
        match &mut self.kind {
            LayerKind::Adjustment(ref mut a) => Some(a),
            _ => None,
        }
    }

    pub fn group(&self) -> Option<&GroupLayer> {
        match &self.kind {
            LayerKind::Group(ref g) => Some(g),
            _ => None,
        }
    }

    pub fn group_mut(&mut self) -> Option<&mut GroupLayer> {
        match &mut self.kind {
            LayerKind::Group(ref mut g) => Some(g),
            _ => None,
        }
    }

    pub fn mask(&self) -> Option<&[u8]> {
        match &self.kind {
            LayerKind::Raster(ref r) => r.mask(),
            LayerKind::Adjustment(ref a) => a.mask(),
            LayerKind::Text(ref t) => t.mask.as_deref(),
            LayerKind::Group(_) => None,
        }
    }

    pub fn mask_mut(&mut self) -> Option<&mut [u8]> {
        match &mut self.kind {
            LayerKind::Raster(ref mut r) => r.mask_mut(),
            LayerKind::Adjustment(ref mut a) => a.mask_mut(),
            LayerKind::Text(ref mut t) => t.mask.as_deref_mut(),
            LayerKind::Group(_) => None,
        }
    }
}
