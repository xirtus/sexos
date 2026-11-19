// SPDX-License-Identifier: GPL-3.0-only
//! Document model, background canvas, and straight-alpha layer compositor.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_image::{Image, Limits};

use crate::admit_limits;
use crate::adjust::{AdjustmentKind, CurvesAdjustment, CurvesPreset, HueSatAdjustment};
use crate::blend::{blend_pixel, BlendMode};
use crate::error::BatikError;
use crate::filters::{apply_filter_stack, FilterNode, FilterSpec};
use crate::geom::Rect;
use crate::layer::{AdjustmentLayer, Layer, LayerKind, RasterLayer};
use crate::selection::SelectionMask;
use crate::text::TextLayer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundMode {
    Transparent,
    Solid(u32),
    Checkerboard,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub width: usize,
    pub height: usize,
    pub background: BackgroundMode,
    pub layers: Vec<Layer>,
    pub selection: SelectionMask,
    pub limits: Limits,
    next_layer_id: u64,
}

impl Document {
    pub fn new(
        width: usize,
        height: usize,
        background: BackgroundMode,
        limits: Limits,
    ) -> Result<Self, BatikError> {
        admit_limits(&limits, width, height)?;
        let selection = SelectionMask::new(width, height)?;

        Ok(Self {
            width,
            height,
            background,
            layers: Vec::new(),
            selection,
            limits,
            next_layer_id: 1,
        })
    }

    pub fn add_raster_layer(
        &mut self,
        name: &str,
        blend_mode: BlendMode,
        opacity: u8,
    ) -> Result<u64, BatikError> {
        let raster = RasterLayer::new(self.width, self.height, &self.limits)?;
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        self.layers.push(Layer::new_raster(
            id,
            String::from(name),
            raster,
            blend_mode,
            opacity,
        ));
        Ok(id)
    }

    pub fn add_curves_layer(
        &mut self,
        name: &str,
        curves: CurvesAdjustment,
        opacity: u8,
    ) -> u64 {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        let adj = AdjustmentLayer::new(AdjustmentKind::Curves(curves));
        self.layers.push(Layer::new_adjustment(
            id,
            String::from(name),
            adj,
            BlendMode::Normal,
            opacity,
        ));
        id
    }

    pub fn add_curves_preset_layer(
        &mut self,
        name: &str,
        preset: CurvesPreset,
        opacity: u8,
    ) -> u64 {
        self.add_curves_layer(name, preset.to_curves(), opacity)
    }

    pub fn add_hue_saturation_layer(
        &mut self,
        name: &str,
        hue_deg: i16,
        sat_pct: i16,
        light_pct: i16,
        opacity: u8,
    ) -> u64 {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        let hs = HueSatAdjustment::new(hue_deg, sat_pct, light_pct);
        let adj = AdjustmentLayer::new(AdjustmentKind::HueSaturation(hs));
        self.layers.push(Layer::new_adjustment(
            id,
            String::from(name),
            adj,
            BlendMode::Normal,
            opacity,
        ));
        id
    }

    pub fn update_adjustment_layer(
        &mut self,
        id: u64,
        new_kind: AdjustmentKind,
    ) -> Result<(), BatikError> {
        let layer = self
            .get_layer_mut(id)
            .ok_or(BatikError::LayerNotFound(id))?;
        if let LayerKind::Adjustment(ref mut adj) = layer.kind {
            adj.kind = new_kind;
            Ok(())
        } else {
            Err(BatikError::LayerNotFound(id))
        }
    }

    pub fn add_text_layer(
        &mut self,
        name: &str,
        text: &str,
        x: i32,
        y: i32,
        font_px: f32,
        color: u32,
        blend_mode: BlendMode,
        opacity: u8,
    ) -> Result<u64, BatikError> {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        let text_layer = TextLayer::new(text, x, y, font_px, color);
        self.layers.push(Layer::new_text(
            id,
            String::from(name),
            text_layer,
            blend_mode,
            opacity,
        ));
        Ok(id)
    }

    pub fn set_text_layer_content(&mut self, id: u64, text: &str) -> Result<(), BatikError> {
        let layer = self
            .get_layer_mut(id)
            .ok_or(BatikError::LayerNotFound(id))?;
        if let LayerKind::Text(ref mut t) = layer.kind {
            t.text = String::from(text);
            Ok(())
        } else {
            Err(BatikError::LayerNotFound(id))
        }
    }

    pub fn set_text_layer_style(
        &mut self,
        id: u64,
        font_px: f32,
        color: u32,
        bold: bool,
    ) -> Result<(), BatikError> {
        let layer = self
            .get_layer_mut(id)
            .ok_or(BatikError::LayerNotFound(id))?;
        if let LayerKind::Text(ref mut t) = layer.kind {
            if font_px.is_finite() && font_px > 0.0 {
                t.font_px = font_px;
            }
            t.color = color;
            t.bold = bold;
            Ok(())
        } else {
            Err(BatikError::LayerNotFound(id))
        }
    }

    pub fn move_text_layer(&mut self, id: u64, x: i32, y: i32) -> Result<(), BatikError> {
        let layer = self
            .get_layer_mut(id)
            .ok_or(BatikError::LayerNotFound(id))?;
        if let LayerKind::Text(ref mut t) = layer.kind {
            t.x = x;
            t.y = y;
            Ok(())
        } else {
            Err(BatikError::LayerNotFound(id))
        }
    }

    /// Explicitly converts a TextLayer to a RasterLayer baked with current dimensions.
    pub fn rasterize_layer(&mut self, id: u64) -> Result<(), BatikError> {
        let doc_w = self.width;
        let doc_h = self.height;
        let layer = self
            .get_layer_mut(id)
            .ok_or(BatikError::LayerNotFound(id))?;
        if let LayerKind::Text(ref t) = layer.kind {
            let raster = t.to_raster_layer(doc_w, doc_h)?;
            layer.kind = LayerKind::Raster(raster);
            Ok(())
        } else {
            Ok(())
        }
    }

    pub fn push_filter(&mut self, layer_id: u64, spec: FilterSpec) -> Result<usize, BatikError> {
        let layer = self
            .get_layer_mut(layer_id)
            .ok_or(BatikError::LayerNotFound(layer_id))?;
        layer.filter_stack.push(FilterNode::new(spec));
        Ok(layer.filter_stack.len() - 1)
    }

    pub fn set_filter_enabled(
        &mut self,
        layer_id: u64,
        index: usize,
        enabled: bool,
    ) -> Result<(), BatikError> {
        let layer = self
            .get_layer_mut(layer_id)
            .ok_or(BatikError::LayerNotFound(layer_id))?;
        if let Some(node) = layer.filter_stack.get_mut(index) {
            node.enabled = enabled;
            Ok(())
        } else {
            Err(BatikError::LayerNotFound(layer_id))
        }
    }

    pub fn remove_filter(
        &mut self,
        layer_id: u64,
        index: usize,
    ) -> Result<FilterNode, BatikError> {
        let layer = self
            .get_layer_mut(layer_id)
            .ok_or(BatikError::LayerNotFound(layer_id))?;
        if index < layer.filter_stack.len() {
            Ok(layer.filter_stack.remove(index))
        } else {
            Err(BatikError::LayerNotFound(layer_id))
        }
    }

    pub fn clear_filter_stack(&mut self, layer_id: u64) -> Result<(), BatikError> {
        let layer = self
            .get_layer_mut(layer_id)
            .ok_or(BatikError::LayerNotFound(layer_id))?;
        layer.filter_stack.clear();
        Ok(())
    }

    pub fn move_filter(
        &mut self,
        layer_id: u64,
        from: usize,
        to: usize,
    ) -> Result<(), BatikError> {
        let layer = self
            .get_layer_mut(layer_id)
            .ok_or(BatikError::LayerNotFound(layer_id))?;
        if from < layer.filter_stack.len() && to < layer.filter_stack.len() {
            let item = layer.filter_stack.remove(from);
            layer.filter_stack.insert(to, item);
            Ok(())
        } else {
            Err(BatikError::LayerNotFound(layer_id))
        }
    }

    pub fn add_layer_mask(&mut self, layer_id: u64, default_val: u8) -> Result<(), BatikError> {
        let doc_w = self.width;
        let doc_h = self.height;
        let layer = self
            .get_layer_mut(layer_id)
            .ok_or(BatikError::LayerNotFound(layer_id))?;
        match &mut layer.kind {
            LayerKind::Raster(ref mut r) => r.add_mask(default_val),
            LayerKind::Adjustment(ref mut a) => {
                let total = doc_w.checked_mul(doc_h).ok_or(BatikError::TooLarge)?;
                let mut mask = Vec::new();
                mask.try_reserve_exact(total).map_err(|_| BatikError::OutOfMemory)?;
                mask.resize(total, default_val);
                a.mask = Some(mask);
                Ok(())
            }
            LayerKind::Text(ref mut t) => {
                let total = doc_w.checked_mul(doc_h).ok_or(BatikError::TooLarge)?;
                let mut mask = Vec::new();
                mask.try_reserve_exact(total).map_err(|_| BatikError::OutOfMemory)?;
                mask.resize(total, default_val);
                t.mask = Some(mask);
                Ok(())
            }
            LayerKind::Group(_) => Ok(()),
        }
    }

    pub fn apply_selection_to_mask(&mut self, layer_id: u64) -> Result<(), BatikError> {
        let doc_w = self.width;
        let doc_h = self.height;
        let total = doc_w.checked_mul(doc_h).ok_or(BatikError::TooLarge)?;
        let mut mask_data = Vec::new();
        mask_data.try_reserve_exact(total).map_err(|_| BatikError::OutOfMemory)?;
        for y in 0..doc_h {
            for x in 0..doc_w {
                mask_data.push(self.selection.sample(x, y));
            }
        }

        let layer = self
            .get_layer_mut(layer_id)
            .ok_or(BatikError::LayerNotFound(layer_id))?;
        match &mut layer.kind {
            LayerKind::Raster(ref mut r) => {
                r.mask = Some(mask_data);
                Ok(())
            }
            LayerKind::Adjustment(ref mut a) => {
                a.mask = Some(mask_data);
                Ok(())
            }
            LayerKind::Text(ref mut t) => {
                t.mask = Some(mask_data);
                Ok(())
            }
            LayerKind::Group(_) => Ok(()),
        }
    }

    pub fn invert_layer_mask(&mut self, layer_id: u64) -> Result<(), BatikError> {
        let layer = self
            .get_layer_mut(layer_id)
            .ok_or(BatikError::LayerNotFound(layer_id))?;
        if let Some(mask) = layer.mask_mut() {
            for m in mask.iter_mut() {
                *m = 255 - *m;
            }
        }
        Ok(())
    }

    pub fn delete_layer_mask(&mut self, layer_id: u64) -> Result<(), BatikError> {
        let layer = self
            .get_layer_mut(layer_id)
            .ok_or(BatikError::LayerNotFound(layer_id))?;
        match &mut layer.kind {
            LayerKind::Raster(ref mut r) => r.delete_mask(),
            LayerKind::Adjustment(ref mut a) => a.mask = None,
            LayerKind::Text(ref mut t) => t.mask = None,
            LayerKind::Group(_) => {}
        }
        Ok(())
    }

    pub fn add_layer(&mut self, mut layer: Layer) -> u64 {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        layer.id = id;
        self.layers.push(layer);
        id
    }

    pub fn get_layer(&self, id: u64) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn get_layer_mut(&mut self, id: u64) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn remove_layer(&mut self, id: u64) -> Option<Layer> {
        if let Some(pos) = self.layers.iter().position(|l| l.id == id) {
            Some(self.layers.remove(pos))
        } else {
            None
        }
    }

    /// Duplicates an existing layer, inserting the duplicate directly above it.
    pub fn duplicate_layer(&mut self, id: u64) -> Result<u64, BatikError> {
        let pos = self
            .layers
            .iter()
            .position(|l| l.id == id)
            .ok_or(BatikError::LayerNotFound(id))?;
        let orig = &self.layers[pos];
        let new_id = self.next_layer_id;
        self.next_layer_id += 1;
        let mut cloned = orig.clone();
        cloned.id = new_id;
        cloned.name = format!("{} Copy", orig.name);
        self.layers.insert(pos + 1, cloned);
        Ok(new_id)
    }

    /// Merges the layer `id` down into the layer beneath it.
    pub fn merge_down(&mut self, id: u64) -> Result<Option<u64>, BatikError> {
        let pos = self
            .layers
            .iter()
            .position(|l| l.id == id)
            .ok_or(BatikError::LayerNotFound(id))?;
        if pos == 0 {
            return Ok(None);
        }
        let top = self.layers.remove(pos);
        let bottom = &mut self.layers[pos - 1];

        if let (LayerKind::Raster(ref top_r), LayerKind::Raster(ref mut bot_r)) =
            (&top.kind, &mut bottom.kind)
        {
            let top_bounds = top_r.bounds();
            let bot_bounds = bot_r.bounds();
            if let Some(inter) = top_bounds.intersect(&bot_bounds) {
                for y in inter.y..inter.bottom() {
                    let bot_row = ((y - bot_r.offset_y) as usize) * bot_r.width;
                    let top_row = ((y - top_r.offset_y) as usize) * top_r.width;
                    for x in inter.x..inter.right() {
                        let bot_col = (x - bot_r.offset_x) as usize;
                        let top_col = (x - top_r.offset_x) as usize;
                        let bot_idx = bot_row + bot_col;
                        let top_idx = top_row + top_col;
                        if bot_idx < bot_r.pixels.len() && top_idx < top_r.pixels.len() {
                            let top_p = top_r.pixels[top_idx];
                            let top_m = match &top_r.mask {
                                Some(m) => m.get(top_idx).copied().unwrap_or(255),
                                None => 255,
                            };
                            bot_r.pixels[bot_idx] = blend_pixel(
                                bot_r.pixels[bot_idx],
                                top_p,
                                top.blend_mode,
                                top.opacity,
                                top_m,
                            );
                        }
                    }
                }
            }
        }
        Ok(Some(bottom.id))
    }

    /// Moves a layer up in stack order (drawn on top of the next layer).
    pub fn move_layer_up(&mut self, id: u64) -> bool {
        if let Some(pos) = self.layers.iter().position(|l| l.id == id) {
            if pos + 1 < self.layers.len() {
                self.layers.swap(pos, pos + 1);
                return true;
            }
        }
        false
    }

    /// Moves a layer down in stack order.
    pub fn move_layer_down(&mut self, id: u64) -> bool {
        if let Some(pos) = self.layers.iter().position(|l| l.id == id) {
            if pos > 0 {
                self.layers.swap(pos, pos - 1);
                return true;
            }
        }
        false
    }

    /// Produces baked raster layer representations of all layers for multi-layer export (e.g. PSD, ORA).
    /// TextLayers and layers with FilterStacks are rasterized with their non-destructive effects baked.
    pub fn exportable_raster_layers(&self) -> Vec<(Layer, RasterLayer)> {
        let mut list = Vec::new();
        for l in &self.layers {
            match &l.kind {
                LayerKind::Raster(r) => {
                    let mut pixels = r.pixels.clone();
                    if l.filter_stack.iter().any(|f| f.enabled) {
                        apply_filter_stack(&l.filter_stack, &mut pixels, r.width, r.height);
                    }
                    let baked_raster = RasterLayer {
                        width: r.width,
                        height: r.height,
                        offset_x: r.offset_x,
                        offset_y: r.offset_y,
                        pixels,
                        mask: r.mask.clone(),
                        alpha_lock: r.alpha_lock,
                        pixel_lock: r.pixel_lock,
                    };
                    list.push((l.clone(), baked_raster));
                }
                LayerKind::Text(t) => {
                    let mut pixels = t.render_to_buffer(self.width, self.height);
                    if l.filter_stack.iter().any(|f| f.enabled) {
                        apply_filter_stack(&l.filter_stack, &mut pixels, self.width, self.height);
                    }
                    let baked_raster = RasterLayer {
                        width: self.width,
                        height: self.height,
                        offset_x: 0,
                        offset_y: 0,
                        pixels,
                        mask: t.mask.clone(),
                        alpha_lock: false,
                        pixel_lock: false,
                    };
                    list.push((l.clone(), baked_raster));
                }
                _ => {}
            }
        }
        list
    }

    /// Creates a single-layer Document from an already-decoded `sex_image::Image`.
    pub fn from_image(image: Image, limits: Limits) -> Result<Self, BatikError> {
        let (w, h) = (image.width, image.height);
        let mut doc = Self::new(w, h, BackgroundMode::Transparent, limits)?;
        let raster = RasterLayer::with_pixels(w, h, image.pixels, &limits)?;
        let layer = Layer::new_raster(1, String::from("Background"), raster, BlendMode::Normal, 255);
        doc.add_layer(layer);
        Ok(doc)
    }

    /// Decodes a PNG image and constructs a Document.
    pub fn from_png(png_bytes: &[u8], limits: Limits) -> Result<Self, BatikError> {
        let img = sex_image::decode(png_bytes, &limits)?;
        Self::from_image(img, limits)
    }

    /// Automatically sniffs format (PSD, ORA, PNG, JPEG, WebP, GIF) and imports as a Document.
    pub fn import_bytes(bytes: &[u8], limits: Limits) -> Result<Self, BatikError> {
        if bytes.starts_with(b"8BPS") {
            crate::psd::parse_psd(bytes, &limits)
        } else if bytes.starts_with(b"PK\x03\x04") {
            crate::ora::read_ora(bytes, &limits)
        } else {
            let img = sex_image::decode(bytes, &limits)?;
            Self::from_image(img, limits)
        }
    }

    /// Renders a region of interest (ROI) into a target pixel buffer.
    pub fn render_region(&self, roi: Rect, out_buf: &mut [u32], stride: usize) {
        let clamped = match roi.clamp_to_bounds(self.width, self.height) {
            Some(r) => r,
            None => return,
        };

        // Initialize background
        for y in 0..clamped.height {
            let row_offset = y * stride;
            let abs_y = clamped.y + y as i32;

            for x in 0..clamped.width {
                let abs_x = clamped.x + x as i32;
                let bg_color = match self.background {
                    BackgroundMode::Transparent => 0,
                    BackgroundMode::Solid(c) => c,
                    BackgroundMode::Checkerboard => {
                        let sq = ((abs_x / 16) + (abs_y / 16)) % 2;
                        if sq == 0 {
                            0xFFFFFFFF
                        } else {
                            0xFFCCCCCC
                        }
                    }
                };
                out_buf[row_offset + x] = bg_color;
            }
        }

        // Composite layers
        Self::composite_layer_slice(&self.layers, clamped, out_buf, stride, self.width, self.height);
    }

    /// Renders all visible layers into a freshly allocated flat RGBA buffer.
    pub fn render_flattened(&self) -> Vec<u32> {
        let mut out = alloc::vec![0u32; self.width * self.height];
        self.render_region(Rect::new(0, 0, self.width, self.height), &mut out, self.width);
        out
    }

    fn composite_layer_slice(
        layers: &[Layer],
        roi: Rect,
        out_buf: &mut [u32],
        stride: usize,
        doc_width: usize,
        doc_height: usize,
    ) {
        for layer in layers {
            if !layer.visible || layer.opacity == 0 {
                continue;
            }

            match &layer.kind {
                LayerKind::Raster(raster) => {
                    let has_fx = layer.filter_stack.iter().any(|f| f.enabled);
                    let filtered_storage;
                    let pixels_ref: &[u32] = if has_fx {
                        filtered_storage = {
                            let mut p = raster.pixels.clone();
                            apply_filter_stack(&layer.filter_stack, &mut p, raster.width, raster.height);
                            p
                        };
                        &filtered_storage
                    } else {
                        &raster.pixels
                    };

                    let layer_bounds = raster.bounds();
                    let inter = match layer_bounds.intersect(&roi) {
                        Some(i) => i,
                        None => continue,
                    };

                    for y in inter.y..inter.bottom() {
                        let out_row = ((y - roi.y) as usize) * stride;
                        let src_ly = (y - raster.offset_y) as usize;
                        let src_row = src_ly * raster.width;

                        for x in inter.x..inter.right() {
                            let out_col = (x - roi.x) as usize;
                            let src_lx = (x - raster.offset_x) as usize;
                            let src_idx = src_row + src_lx;
                            let src_pixel = match pixels_ref.get(src_idx) {
                                Some(&p) => p,
                                None => continue,
                            };

                            let mask_val = match &raster.mask {
                                Some(m) => m.get(src_idx).copied().unwrap_or(255),
                                None => 255,
                            };

                            let out_idx = out_row + out_col;
                            if out_idx < out_buf.len() {
                                let backdrop = out_buf[out_idx];
                                out_buf[out_idx] = blend_pixel(
                                    backdrop,
                                    src_pixel,
                                    layer.blend_mode,
                                    layer.opacity,
                                    mask_val,
                                );
                            }
                        }
                    }
                }
                LayerKind::Text(text) => {
                    let mut text_buf = text.render_to_buffer(doc_width, doc_height);
                    if layer.filter_stack.iter().any(|f| f.enabled) {
                        apply_filter_stack(&layer.filter_stack, &mut text_buf, doc_width, doc_height);
                    }

                    let text_bounds = Rect::new(0, 0, doc_width, doc_height);
                    let inter = match text_bounds.intersect(&roi) {
                        Some(i) => i,
                        None => continue,
                    };

                    for y in inter.y..inter.bottom() {
                        let out_row = ((y - roi.y) as usize) * stride;
                        let src_row = (y as usize) * doc_width;

                        for x in inter.x..inter.right() {
                            let out_col = (x - roi.x) as usize;
                            let src_idx = src_row + (x as usize);
                            let src_pixel = match text_buf.get(src_idx) {
                                Some(&p) => p,
                                None => continue,
                            };

                            let mask_val = match &text.mask {
                                Some(m) => m.get(src_idx).copied().unwrap_or(255),
                                None => 255,
                            };

                            let out_idx = out_row + out_col;
                            if out_idx < out_buf.len() {
                                let backdrop = out_buf[out_idx];
                                out_buf[out_idx] = blend_pixel(
                                    backdrop,
                                    src_pixel,
                                    layer.blend_mode,
                                    layer.opacity,
                                    mask_val,
                                );
                            }
                        }
                    }
                }
                LayerKind::Adjustment(adj) => {
                    let curves_luts = if let AdjustmentKind::Curves(ref c) = adj.kind {
                        Some(c.build_all_luts())
                    } else {
                        None
                    };

                    for y in 0..roi.height {
                        let row_offset = y * stride;
                        let abs_y = (roi.y + y as i32) as usize;

                        for x in 0..roi.width {
                            let abs_x = (roi.x + x as i32) as usize;
                            let idx = row_offset + x;
                            if idx >= out_buf.len() {
                                continue;
                            }
                            let backdrop = out_buf[idx];

                            let mask_val = match &adj.mask {
                                Some(m) => {
                                    if abs_x < doc_width && abs_y < doc_height {
                                        m.get(abs_y * doc_width + abs_x).copied().unwrap_or(255)
                                    } else {
                                        255
                                    }
                                }
                                None => 255,
                            };

                            let adjusted = if let Some((ref lm, ref lr, ref lg, ref lb)) = curves_luts {
                                crate::adjust::CurvesAdjustment::apply_with_luts(backdrop, lm, lr, lg, lb)
                            } else {
                                adj.apply_pixel(backdrop)
                            };

                            out_buf[idx] = blend_pixel(
                                backdrop,
                                adjusted,
                                layer.blend_mode,
                                layer.opacity,
                                mask_val,
                            );
                        }
                    }
                }
                LayerKind::Group(group) => {
                    if group.pass_through {
                        Self::composite_layer_slice(&group.layers, roi, out_buf, stride, doc_width, doc_height);
                    } else {
                        // Isolated group compositing
                        let group_len = roi.width * roi.height;
                        let mut group_buf = vec![0u32; group_len];
                        Self::composite_layer_slice(&group.layers, roi, &mut group_buf, roi.width, doc_width, doc_height);

                        for y in 0..roi.height {
                            let out_row = y * stride;
                            let grp_row = y * roi.width;
                            for x in 0..roi.width {
                                let backdrop = out_buf[out_row + x];
                                let src_p = group_buf[grp_row + x];
                                out_buf[out_row + x] = blend_pixel(
                                    backdrop,
                                    src_p,
                                    layer.blend_mode,
                                    layer.opacity,
                                    255,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Flattens the entire document into an unpremultiplied `0xAARRGGBB` `sex_image::Image`.
    pub fn flatten(&self) -> Result<Image, BatikError> {
        let roi = Rect::new(0, 0, self.width, self.height);
        self.flatten_rect(roi)
    }

    /// Flattens a sub-rectangle into a `sex_image::Image`.
    pub fn flatten_rect(&self, roi: Rect) -> Result<Image, BatikError> {
        let clamped = match roi.clamp_to_bounds(self.width, self.height) {
            Some(r) => r,
            None => return Err(BatikError::InvalidDimensions),
        };

        let total_pixels = admit_limits(&self.limits, clamped.width, clamped.height)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(total_pixels).map_err(|_| BatikError::OutOfMemory)?;
        pixels.resize(total_pixels, 0);

        self.render_region(clamped, &mut pixels, clamped.width);

        Ok(Image {
            width: clamped.width,
            height: clamped.height,
            pixels,
        })
    }

    /// Crops the document to the specified rectangle.
    pub fn crop(&mut self, rect: Rect) -> Result<(), BatikError> {
        let clamped = match rect.clamp_to_bounds(self.width, self.height) {
            Some(r) => r,
            None => return Err(BatikError::InvalidDimensions),
        };

        admit_limits(&self.limits, clamped.width, clamped.height)?;

        // Adjust offsets of raster layers
        for layer in &mut self.layers {
            if let LayerKind::Raster(ref mut raster) = layer.kind {
                raster.offset_x -= clamped.x;
                raster.offset_y -= clamped.y;
            } else if let LayerKind::Text(ref mut text) = layer.kind {
                text.x -= clamped.x;
                text.y -= clamped.y;
            }
        }

        // Adjust selection mask
        let mut new_sel = SelectionMask::new(clamped.width, clamped.height)?;
        if self.selection.is_active() {
            new_sel.active = true;
            for y in 0..clamped.height {
                let old_y = (clamped.y + y as i32) as usize;
                for x in 0..clamped.width {
                    let old_x = (clamped.x + x as i32) as usize;
                    new_sel.mask[y * clamped.width + x] = self.selection.sample(old_x, old_y);
                }
            }
        }

        self.width = clamped.width;
        self.height = clamped.height;
        self.selection = new_sel;

        Ok(())
    }
}
