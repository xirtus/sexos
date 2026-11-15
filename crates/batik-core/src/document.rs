// SPDX-License-Identifier: GPL-3.0-only
//! Document model, background canvas, and straight-alpha layer compositor.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_image::{Image, Limits};

use crate::admit_limits;
use crate::blend::{blend_pixel, BlendMode};
use crate::error::BatikError;
use crate::geom::Rect;
use crate::layer::{Layer, LayerKind, RasterLayer};
use crate::selection::SelectionMask;

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
        Self::composite_layer_slice(&self.layers, clamped, out_buf, stride);
    }

    /// Renders all visible layers into a freshly allocated flat RGBA buffer.
    pub fn render_flattened(&self) -> Vec<u32> {
        let mut out = alloc::vec![0u32; self.width * self.height];
        self.render_region(Rect::new(0, 0, self.width, self.height), &mut out, self.width);
        out
    }

    fn composite_layer_slice(layers: &[Layer], roi: Rect, out_buf: &mut [u32], stride: usize) {
        for layer in layers {
            if !layer.visible || layer.opacity == 0 {
                continue;
            }

            match &layer.kind {
                LayerKind::Raster(raster) => {
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
                            let src_pixel = match raster.pixels.get(src_idx) {
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
                LayerKind::Adjustment(adj) => {
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
                                    if abs_x < roi.width && abs_y < roi.height {
                                        m.get(abs_y * roi.width + abs_x).copied().unwrap_or(255)
                                    } else {
                                        255
                                    }
                                }
                                None => 255,
                            };

                            let adjusted = adj.adjustment.apply_pixel(backdrop);
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
                        Self::composite_layer_slice(&group.layers, roi, out_buf, stride);
                    } else {
                        // Isolated group compositing
                        let group_len = roi.width * roi.height;
                        let mut group_buf = vec![0u32; group_len];
                        Self::composite_layer_slice(&group.layers, roi, &mut group_buf, roi.width);

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
