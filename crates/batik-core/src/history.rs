// SPDX-License-Identifier: GPL-3.0-only
//! Bounded undo and redo history journal with byte budget eviction.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use crate::adjust::AdjustmentKind;
use crate::blend::BlendMode;
use crate::document::Document;
use crate::filters::FilterNode;
use crate::geom::Rect;
use crate::layer::{Layer, LayerKind, RasterLayer};
use crate::text::TextLayer;

#[derive(Clone, Debug, PartialEq)]
pub enum HistoryAction {
    LayerPixelDelta {
        layer_id: u64,
        rect: Rect,
        old_pixels: Vec<u32>,
        new_pixels: Vec<u32>,
    },
    LayerAdded {
        layer: Layer,
    },
    LayerRemoved {
        layer_index: usize,
        layer: Layer,
    },
    LayerPropertyChange {
        layer_id: u64,
        old_opacity: u8,
        new_opacity: u8,
        old_blend: BlendMode,
        new_blend: BlendMode,
        old_visible: bool,
        new_visible: bool,
    },
    DocumentCrop {
        old_width: usize,
        old_height: usize,
        new_width: usize,
        new_height: usize,
        layer_shifts: Vec<(u64, i32, i32)>, // (layer_id, dx, dy)
    },
    LayerMaskDelta {
        layer_id: u64,
        old_mask: Option<Vec<u8>>,
        new_mask: Option<Vec<u8>>,
    },
    AdjustmentLayerChanged {
        layer_id: u64,
        old_kind: AdjustmentKind,
        new_kind: AdjustmentKind,
    },
    TextLayerChanged {
        layer_id: u64,
        old_text: TextLayer,
        new_text: TextLayer,
    },
    FilterStackChanged {
        layer_id: u64,
        old_stack: Vec<FilterNode>,
        new_stack: Vec<FilterNode>,
    },
}

impl HistoryAction {
    pub fn byte_size(&self) -> usize {
        let base = core::mem::size_of::<Self>();
        match self {
            HistoryAction::LayerPixelDelta { old_pixels, new_pixels, .. } => {
                base + (old_pixels.len() + new_pixels.len()) * core::mem::size_of::<u32>()
            }
            HistoryAction::LayerAdded { layer } | HistoryAction::LayerRemoved { layer, .. } => {
                let layer_bytes = match &layer.kind {
                    LayerKind::Raster(r) => {
                        r.pixels.len() * core::mem::size_of::<u32>()
                            + r.mask.as_ref().map_or(0, |m| m.len())
                    }
                    LayerKind::Text(t) => {
                        t.text.len() + t.mask.as_ref().map_or(0, |m| m.len())
                    }
                    LayerKind::Adjustment(a) => {
                        a.mask.as_ref().map_or(0, |m| m.len())
                    }
                    _ => 256,
                };
                base + layer.name.len() + layer_bytes + layer.filter_stack.len() * core::mem::size_of::<FilterNode>()
            }
            HistoryAction::LayerPropertyChange { .. } => base,
            HistoryAction::DocumentCrop { layer_shifts, .. } => {
                base + layer_shifts.len() * core::mem::size_of::<(u64, i32, i32)>()
            }
            HistoryAction::LayerMaskDelta { old_mask, new_mask, .. } => {
                base + old_mask.as_ref().map_or(0, |m| m.len()) + new_mask.as_ref().map_or(0, |m| m.len())
            }
            HistoryAction::AdjustmentLayerChanged { .. } => base + 128,
            HistoryAction::TextLayerChanged { old_text, new_text, .. } => {
                base + old_text.text.len() + new_text.text.len()
            }
            HistoryAction::FilterStackChanged { old_stack, new_stack, .. } => {
                base + (old_stack.len() + new_stack.len()) * core::mem::size_of::<FilterNode>()
            }
        }
    }
}

pub struct History {
    pub max_history_bytes: usize,
    undo_stack: VecDeque<HistoryAction>,
    redo_stack: VecDeque<HistoryAction>,
    current_bytes: usize,
}

impl History {
    pub fn new(max_history_bytes: usize) -> Self {
        Self {
            max_history_bytes,
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            current_bytes: 0,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.current_bytes = 0;
    }

    pub fn push_action(&mut self, action: HistoryAction) {
        // Redo stack is discarded on new action
        while let Some(r) = self.redo_stack.pop_back() {
            self.current_bytes = self.current_bytes.saturating_sub(r.byte_size());
        }

        let action_bytes = action.byte_size();
        self.current_bytes += action_bytes;
        self.undo_stack.push_back(action);

        // Evict oldest undo steps while over byte budget
        while self.current_bytes > self.max_history_bytes && self.undo_stack.len() > 1 {
            if let Some(oldest) = self.undo_stack.pop_front() {
                self.current_bytes = self.current_bytes.saturating_sub(oldest.byte_size());
            }
        }
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        let action = match self.undo_stack.pop_back() {
            Some(a) => a,
            None => return false,
        };

        match &action {
            HistoryAction::LayerPixelDelta { layer_id, rect, old_pixels, .. } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    if let LayerKind::Raster(ref mut raster) = layer.kind {
                        Self::apply_tile_pixels(raster, *rect, old_pixels);
                    }
                }
            }
            HistoryAction::LayerAdded { layer } => {
                doc.remove_layer(layer.id);
            }
            HistoryAction::LayerRemoved { layer_index, layer } => {
                let idx = (*layer_index).min(doc.layers.len());
                doc.layers.insert(idx, layer.clone());
            }
            HistoryAction::LayerPropertyChange {
                layer_id,
                old_opacity,
                old_blend,
                old_visible,
                ..
            } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    layer.opacity = *old_opacity;
                    layer.blend_mode = *old_blend;
                    layer.visible = *old_visible;
                }
            }
            HistoryAction::DocumentCrop {
                old_width,
                old_height,
                layer_shifts,
                ..
            } => {
                doc.width = *old_width;
                doc.height = *old_height;
                for &(id, dx, dy) in layer_shifts {
                    if let Some(layer) = doc.get_layer_mut(id) {
                        if let LayerKind::Raster(ref mut raster) = layer.kind {
                            raster.offset_x += dx;
                            raster.offset_y += dy;
                        }
                    }
                }
            }
            HistoryAction::LayerMaskDelta { layer_id, old_mask, .. } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    match &mut layer.kind {
                        LayerKind::Raster(ref mut raster) => raster.mask = old_mask.clone(),
                        LayerKind::Adjustment(ref mut adj) => adj.mask = old_mask.clone(),
                        LayerKind::Text(ref mut text) => text.mask = old_mask.clone(),
                        LayerKind::Group(_) => {}
                    }
                }
            }
            HistoryAction::AdjustmentLayerChanged { layer_id, old_kind, .. } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    if let LayerKind::Adjustment(ref mut adj) = layer.kind {
                        adj.kind = old_kind.clone();
                    }
                }
            }
            HistoryAction::TextLayerChanged { layer_id, old_text, .. } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    if let LayerKind::Text(ref mut text) = layer.kind {
                        *text = old_text.clone();
                    }
                }
            }
            HistoryAction::FilterStackChanged { layer_id, old_stack, .. } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    layer.filter_stack = old_stack.clone();
                }
            }
        }

        self.redo_stack.push_back(action);
        true
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        let action = match self.redo_stack.pop_back() {
            Some(a) => a,
            None => return false,
        };

        match &action {
            HistoryAction::LayerPixelDelta { layer_id, rect, new_pixels, .. } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    if let LayerKind::Raster(ref mut raster) = layer.kind {
                        Self::apply_tile_pixels(raster, *rect, new_pixels);
                    }
                }
            }
            HistoryAction::LayerAdded { layer } => {
                doc.layers.push(layer.clone());
            }
            HistoryAction::LayerRemoved { layer, .. } => {
                doc.remove_layer(layer.id);
            }
            HistoryAction::LayerPropertyChange {
                layer_id,
                new_opacity,
                new_blend,
                new_visible,
                ..
            } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    layer.opacity = *new_opacity;
                    layer.blend_mode = *new_blend;
                    layer.visible = *new_visible;
                }
            }
            HistoryAction::DocumentCrop {
                new_width,
                new_height,
                layer_shifts,
                ..
            } => {
                doc.width = *new_width;
                doc.height = *new_height;
                for &(id, dx, dy) in layer_shifts {
                    if let Some(layer) = doc.get_layer_mut(id) {
                        if let LayerKind::Raster(ref mut raster) = layer.kind {
                            raster.offset_x -= dx;
                            raster.offset_y -= dy;
                        }
                    }
                }
            }
            HistoryAction::LayerMaskDelta { layer_id, new_mask, .. } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    match &mut layer.kind {
                        LayerKind::Raster(ref mut raster) => raster.mask = new_mask.clone(),
                        LayerKind::Adjustment(ref mut adj) => adj.mask = new_mask.clone(),
                        LayerKind::Text(ref mut text) => text.mask = new_mask.clone(),
                        LayerKind::Group(_) => {}
                    }
                }
            }
            HistoryAction::AdjustmentLayerChanged { layer_id, new_kind, .. } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    if let LayerKind::Adjustment(ref mut adj) = layer.kind {
                        adj.kind = new_kind.clone();
                    }
                }
            }
            HistoryAction::TextLayerChanged { layer_id, new_text, .. } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    if let LayerKind::Text(ref mut text) = layer.kind {
                        *text = new_text.clone();
                    }
                }
            }
            HistoryAction::FilterStackChanged { layer_id, new_stack, .. } => {
                if let Some(layer) = doc.get_layer_mut(*layer_id) {
                    layer.filter_stack = new_stack.clone();
                }
            }
        }

        self.undo_stack.push_back(action);
        true
    }

    fn apply_tile_pixels(raster: &mut RasterLayer, rect: Rect, pixels: &[u32]) {
        if rect.width == 0 || rect.height == 0 || raster.width == 0 || raster.height == 0 || pixels.len() < rect.width * rect.height {
            return;
        }

        for y in 0..rect.height {
            let signed_y = rect.y.saturating_add(y as i32);
            if signed_y < 0 || (signed_y as usize) >= raster.height {
                continue;
            }
            let ry = signed_y as usize;
            let src_row = y * rect.width;
            let dst_row = ry * raster.width;

            for x in 0..rect.width {
                let signed_x = rect.x.saturating_add(x as i32);
                if signed_x < 0 || (signed_x as usize) >= raster.width {
                    continue;
                }
                let rx = signed_x as usize;
                let dst_idx = dst_row + rx;
                if dst_idx < raster.pixels.len() {
                    raster.pixels[dst_idx] = pixels[src_row + x];
                }
            }
        }
    }
}
