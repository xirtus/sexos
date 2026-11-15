// SPDX-License-Identifier: GPL-3.0-only
//! Adobe Photoshop (.psd) parser and multi-layer importer.

use alloc::string::String;
use alloc::vec::Vec;
use sex_image::Limits;

use crate::admit_limits;
use crate::blend::{pack_argb, BlendMode};
use crate::document::{BackgroundMode, Document};
use crate::error::BatikError;
use crate::layer::{Layer, LayerKind, RasterLayer};

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], BatikError> {
        let end = self.pos.checked_add(len).ok_or(BatikError::TooLarge)?;
        if end > self.data.len() {
            return Err(BatikError::CorruptPsd("unexpected end of file"));
        }
        let start = self.pos;
        self.pos = end;
        Ok(&self.data[start..end])
    }

    fn skip(&mut self, len: usize) -> Result<(), BatikError> {
        let end = self.pos.checked_add(len).ok_or(BatikError::TooLarge)?;
        if end > self.data.len() {
            return Err(BatikError::CorruptPsd("unexpected end of file"));
        }
        self.pos = end;
        Ok(())
    }

    fn read_u8(&mut self) -> Result<u8, BatikError> {
        let b = self.read_bytes(1)?;
        Ok(b[0])
    }

    fn read_u16(&mut self) -> Result<u16, BatikError> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn read_i16(&mut self) -> Result<i16, BatikError> {
        let b = self.read_bytes(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, BatikError> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_i32(&mut self) -> Result<i32, BatikError> {
        let b = self.read_bytes(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

struct ChannelInfo {
    id: i16,
    _length: u32,
}

struct LayerRecord {
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
    channels: Vec<ChannelInfo>,
    blend_mode: BlendMode,
    opacity: u8,
    visible: bool,
    name: String,
}

/// Parses an Adobe Photoshop (.psd) file and constructs a multi-layer Batik `Document`.
pub fn parse_psd(data: &[u8], limits: &Limits) -> Result<Document, BatikError> {
    if data.len() < 26 {
        return Err(BatikError::CorruptPsd("header too short"));
    }

    let mut cursor = Cursor::new(data);

    // 1. Header (26 bytes)
    let sig = cursor.read_bytes(4)?;
    if sig != b"8BPS" {
        return Err(BatikError::CorruptPsd("invalid magic signature"));
    }

    let version = cursor.read_u16()?;
    if version != 1 {
        return Err(BatikError::Unsupported("only PSD version 1 is supported"));
    }

    cursor.skip(6)?; // reserved

    let channels_count = cursor.read_u16()? as usize;
    if channels_count < 1 || channels_count > 4 {
        return Err(BatikError::Unsupported("only 1 to 4 channels supported"));
    }
    let height = cursor.read_u32()? as usize;
    let width = cursor.read_u32()? as usize;
    let depth = cursor.read_u16()?;
    let color_mode = cursor.read_u16()?;

    if depth != 8 {
        return Err(BatikError::Unsupported("only 8-bit depth is supported"));
    }
    if color_mode != 1 && color_mode != 3 {
        return Err(BatikError::Unsupported("only RGB and Grayscale color modes supported"));
    }

    admit_limits(limits, width, height)?;

    // 2. Color Mode Data
    let color_mode_len = cursor.read_u32()? as usize;
    cursor.skip(color_mode_len)?;

    // 3. Image Resources
    let img_res_len = cursor.read_u32()? as usize;
    cursor.skip(img_res_len)?;

    // 4. Layer and Mask Information
    let layer_mask_len = cursor.read_u32()? as usize;
    let layer_section_start = cursor.pos;

    let mut layers = Vec::new();

    if layer_mask_len > 0 && cursor.remaining() >= 4 {
        let layer_info_len = cursor.read_u32()? as usize;
        if layer_info_len > 0 && cursor.remaining() >= 2 {
            let raw_count = cursor.read_i16()?;
            let layer_count = (raw_count as i32).abs() as usize;
            if layer_count > 512 {
                return Err(BatikError::TooLarge);
            }

            let mut records = Vec::new();
            records.try_reserve_exact(layer_count).map_err(|_| BatikError::OutOfMemory)?;

            for _ in 0..layer_count {
                let top = cursor.read_i32()?;
                let left = cursor.read_i32()?;
                let bottom = cursor.read_i32()?;
                let right = cursor.read_i32()?;
                let num_channels = cursor.read_u16()? as usize;
                if num_channels > 16 {
                    return Err(BatikError::Unsupported("too many layer channels"));
                }

                let mut channel_infos = Vec::new();
                for _ in 0..num_channels {
                    let id = cursor.read_i16()?;
                    let length = cursor.read_u32()?;
                    channel_infos.push(ChannelInfo { id, _length: length });
                }

                let blend_sig = cursor.read_bytes(4)?;
                if blend_sig != b"8BIM" {
                    return Err(BatikError::CorruptPsd("invalid blend mode signature"));
                }
                let blend_key = cursor.read_bytes(4)?;
                let blend_key_arr = [blend_key[0], blend_key[1], blend_key[2], blend_key[3]];
                let blend_mode = BlendMode::from_psd_key(&blend_key_arr).unwrap_or(BlendMode::Normal);

                let opacity = cursor.read_u8()?;
                let _clipping = cursor.read_u8()?;
                let flags = cursor.read_u8()?;
                let visible = (flags & 0x02) == 0;
                cursor.skip(1)?; // filler

                let extra_len = cursor.read_u32()? as usize;
                let extra_end = cursor.pos.checked_add(extra_len).ok_or(BatikError::TooLarge)?;

                // Mask data
                let mask_len = cursor.read_u32()? as usize;
                cursor.skip(mask_len)?;

                // Blending ranges
                let blend_ranges_len = cursor.read_u32()? as usize;
                cursor.skip(blend_ranges_len)?;

                // Layer name: Pascal string padded to multiple of 4 bytes
                let name_len = cursor.read_u8()? as usize;
                let name_bytes = cursor.read_bytes(name_len)?;
                let name = String::from_utf8_lossy(name_bytes).into_owned();

                let total_name_bytes = 1 + name_len;
                let pad = (4 - (total_name_bytes % 4)) % 4;
                cursor.skip(pad)?;

                // Skip any remainder in extra data
                if cursor.pos > extra_end {
                    return Err(BatikError::CorruptPsd("layer extra data overrun"));
                }
                let rem = extra_end - cursor.pos;
                cursor.skip(rem)?;

                records.push(LayerRecord {
                    top,
                    left,
                    bottom,
                    right,
                    channels: channel_infos,
                    blend_mode,
                    opacity,
                    visible,
                    name,
                });
            }

            let mut accumulated_pixels: usize = 0;

            // Read channel image data for each layer record
            for (idx, rec) in records.iter().enumerate() {
                let layer_w = match rec.right.checked_sub(rec.left) {
                    Some(w) if w > 0 => w as usize,
                    _ => 0,
                };
                let layer_h = match rec.bottom.checked_sub(rec.top) {
                    Some(h) if h > 0 => h as usize,
                    _ => 0,
                };

                let mut red_channel = Vec::new();
                let mut green_channel = Vec::new();
                let mut blue_channel = Vec::new();
                let mut alpha_channel = Vec::new();
                let mut mask_channel = Vec::new();

                for ch in &rec.channels {
                    let ch_data = decompress_channel(&mut cursor, layer_w, layer_h)?;
                    match ch.id {
                        0 => red_channel = ch_data,
                        1 => green_channel = ch_data,
                        2 => blue_channel = ch_data,
                        -1 => alpha_channel = ch_data,
                        -2 => mask_channel = ch_data,
                        _ => {} // Ignore unknown extra channels
                    }
                }

                if layer_w > 0 && layer_h > 0 {
                    admit_limits(limits, layer_w, layer_h)?;
                    let total = layer_w.checked_mul(layer_h).ok_or(BatikError::TooLarge)?;
                    accumulated_pixels = accumulated_pixels.checked_add(total).ok_or(BatikError::TooLarge)?;
                    if accumulated_pixels > limits.max_pixels {
                        return Err(BatikError::TooLarge);
                    }

                    let mut pixels = Vec::new();
                    pixels.try_reserve_exact(total).map_err(|_| BatikError::OutOfMemory)?;

                    for i in 0..total {
                        let a = if !alpha_channel.is_empty() { alpha_channel[i] } else { 255 };
                        let (r, g, b) = if color_mode == 1 {
                            // Grayscale
                            let gray = if !red_channel.is_empty() { red_channel[i] } else { 0 };
                            (gray, gray, gray)
                        } else {
                            // RGB
                            let r = if !red_channel.is_empty() { red_channel[i] } else { 0 };
                            let g = if !green_channel.is_empty() { green_channel[i] } else { 0 };
                            let b = if !blue_channel.is_empty() { blue_channel[i] } else { 0 };
                            (r, g, b)
                        };
                        pixels.push(pack_argb(a, r, g, b));
                    }

                    let mask = if !mask_channel.is_empty() && mask_channel.len() == total {
                        Some(mask_channel)
                    } else {
                        None
                    };

                    let raster = RasterLayer {
                        width: layer_w,
                        height: layer_h,
                        offset_x: rec.left,
                        offset_y: rec.top,
                        pixels,
                        mask,
                        alpha_lock: false,
                        pixel_lock: false,
                    };

                    let layer = Layer {
                        id: (idx + 1) as u64,
                        name: rec.name.clone(),
                        visible: rec.visible,
                        opacity: rec.opacity,
                        blend_mode: rec.blend_mode,
                        kind: crate::layer::LayerKind::Raster(raster),
                    };
                    layers.push(layer);
                }
            }
        }
    }

    // Skip to end of layer & mask section
    let layer_section_end = layer_section_start.checked_add(layer_mask_len).ok_or(BatikError::TooLarge)?;
    if cursor.pos > layer_section_end {
        return Err(BatikError::CorruptPsd("layer section overrun"));
    }
    let skip_len = layer_section_end - cursor.pos;
    cursor.skip(skip_len)?;

    // 5. If no layers were decoded, fall back to Image Data Section (merged composite)
    if layers.is_empty() && cursor.remaining() >= 2 {
        let compression = cursor.read_u16()?;
        let total_pixels = admit_limits(limits, width, height)?;

        let mut planar_channels = Vec::new();
        for _ in 0..channels_count {
            let mut ch = Vec::new();
            ch.try_reserve_exact(total_pixels).map_err(|_| BatikError::OutOfMemory)?;
            planar_channels.push(ch);
        }

        if compression == 0 {
            // Raw
            for ch in &mut planar_channels {
                let bytes = cursor.read_bytes(total_pixels)?;
                ch.extend_from_slice(bytes);
            }
        } else if compression == 1 {
            // RLE (PackBits)
            // Table of scanline byte counts: channels * height * 2 bytes
            let num_lines = channels_count.checked_mul(height).ok_or(BatikError::TooLarge)?;
            let mut line_lengths = Vec::new();
            line_lengths.try_reserve_exact(num_lines).map_err(|_| BatikError::OutOfMemory)?;
            let mut table_sum: usize = 0;
            for _ in 0..num_lines {
                let l = cursor.read_u16()? as usize;
                table_sum = table_sum.checked_add(l).ok_or(BatikError::TooLarge)?;
                line_lengths.push(l);
            }
            if table_sum > cursor.remaining() {
                return Err(BatikError::CorruptPsd("PackBits table exceeds remaining input"));
            }

            let mut line_idx = 0;
            for ch in &mut planar_channels {
                for _ in 0..height {
                    let line_len = line_lengths[line_idx];
                    line_idx += 1;
                    let line_bytes = cursor.read_bytes(line_len)?;
                    decompress_packbits_line(line_bytes, width, ch)?;
                }
            }
        }

        let mut pixels = Vec::new();
        pixels.try_reserve_exact(total_pixels).map_err(|_| BatikError::OutOfMemory)?;

        for i in 0..total_pixels {
            if color_mode == 1 {
                let g = planar_channels[0][i];
                let a = if channels_count > 1 { planar_channels[1][i] } else { 255 };
                pixels.push(pack_argb(a, g, g, g));
            } else {
                let r = planar_channels[0][i];
                let g = planar_channels[1][i];
                let b = planar_channels[2][i];
                let a = if channels_count > 3 { planar_channels[3][i] } else { 255 };
                pixels.push(pack_argb(a, r, g, b));
            }
        }

        let raster = RasterLayer {
            width,
            height,
            offset_x: 0,
            offset_y: 0,
            pixels,
            mask: None,
            alpha_lock: false,
            pixel_lock: false,
        };

        layers.push(Layer {
            id: 1,
            name: String::from("Background"),
            visible: true,
            opacity: 255,
            blend_mode: BlendMode::Normal,
            kind: crate::layer::LayerKind::Raster(raster),
        });
    }

    let mut doc = Document::new(width, height, BackgroundMode::Transparent, *limits)?;
    for layer in layers {
        doc.add_layer(layer);
    }

    Ok(doc)
}

fn decompress_channel(
    cursor: &mut Cursor<'_>,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, BatikError> {
    if width == 0 || height == 0 {
        return Ok(Vec::new());
    }

    let compression = cursor.read_u16()?;
    let total = width.checked_mul(height).ok_or(BatikError::TooLarge)?;
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| BatikError::OutOfMemory)?;

    if compression == 0 {
        // Raw
        let bytes = cursor.read_bytes(total)?;
        out.extend_from_slice(bytes);
    } else if compression == 1 {
        // RLE (PackBits)
        let mut line_lengths = Vec::new();
        line_lengths.try_reserve_exact(height).map_err(|_| BatikError::OutOfMemory)?;
        let mut table_sum: usize = 0;
        for _ in 0..height {
            let l = cursor.read_u16()? as usize;
            table_sum = table_sum.checked_add(l).ok_or(BatikError::TooLarge)?;
            line_lengths.push(l);
        }
        if table_sum > cursor.remaining() {
            return Err(BatikError::CorruptPsd("PackBits channel table exceeds remaining input"));
        }

        for &len in &line_lengths {
            let line_bytes = cursor.read_bytes(len)?;
            decompress_packbits_line(line_bytes, width, &mut out)?;
        }
    } else {
        return Err(BatikError::Unsupported("unknown channel compression"));
    }

    Ok(out)
}

fn decompress_packbits_line(
    mut src: &[u8],
    expected_len: usize,
    dst: &mut Vec<u8>,
) -> Result<(), BatikError> {
    let start_len = dst.len();
    let target_len = start_len.checked_add(expected_len).ok_or(BatikError::TooLarge)?;

    while dst.len() < target_len && !src.is_empty() {
        let n = src[0] as i8;
        src = &src[1..];

        if n >= 0 {
            // Literal run: 1 + n bytes (0..=127)
            let count = (n as usize) + 1;
            let new_len = dst.len().checked_add(count).ok_or(BatikError::TooLarge)?;
            if src.len() < count || new_len > target_len {
                return Err(BatikError::CorruptPsd("PackBits literal overrun"));
            }
            dst.extend_from_slice(&src[..count]);
            src = &src[count..];
        } else if n > -128 {
            // Repeat run: 1 - n bytes (-127..=-1)
            let count = (1 - (n as i16)) as usize;
            let new_len = dst.len().checked_add(count).ok_or(BatikError::TooLarge)?;
            if src.is_empty() || new_len > target_len {
                return Err(BatikError::CorruptPsd("PackBits repeat overrun"));
            }
            let val = src[0];
            src = &src[1..];
            dst.resize(new_len, val);
        }
        // n == -128 (0x80) is treated as a no-op per PackBits spec
    }

    if dst.len() != target_len {
        return Err(BatikError::CorruptPsd("incomplete PackBits scanline"));
    }

    Ok(())
}

/// Encodes a Batik `Document` into a valid multi-layer Adobe Photoshop (.psd) file.
pub fn encode_psd(doc: &Document) -> Result<Vec<u8>, BatikError> {
    let width = doc.width;
    let height = doc.height;
    if width == 0 || height == 0 {
        return Err(BatikError::InvalidDimensions);
    }
    admit_limits(&doc.limits, width, height)?;

    let mut out = Vec::new();
    let est_size = 26 + 8 + 8 + (width * height * 4);
    out.try_reserve(est_size).map_err(|_| BatikError::OutOfMemory)?;

    // 1. Header (26 bytes)
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1u16.to_be_bytes()); // version 1
    out.extend_from_slice(&[0u8; 6]); // reserved
    out.extend_from_slice(&4u16.to_be_bytes()); // 4 channels (RGBA)
    out.extend_from_slice(&(height as u32).to_be_bytes());
    out.extend_from_slice(&(width as u32).to_be_bytes());
    out.extend_from_slice(&8u16.to_be_bytes()); // depth 8
    out.extend_from_slice(&3u16.to_be_bytes()); // color mode 3 (RGB)

    // 2. Color Mode Data (length 0)
    out.extend_from_slice(&0u32.to_be_bytes());

    // 3. Image Resources (length 0)
    out.extend_from_slice(&0u32.to_be_bytes());

    // 4. Layer and Mask Information section
    let raster_layers: Vec<(&Layer, &RasterLayer)> = doc
        .layers
        .iter()
        .filter_map(|l| match &l.kind {
            LayerKind::Raster(ref r) => Some((l, r)),
            _ => None,
        })
        .collect();

    let layer_count = raster_layers.len();
    let mut layer_info = Vec::new();
    layer_info.extend_from_slice(&(layer_count as i16).to_be_bytes());

    let mut all_channel_data = Vec::new();

    for (l, r) in &raster_layers {
        let top = r.offset_y;
        let left = r.offset_x;
        let bottom = r.offset_y + r.height as i32;
        let right = r.offset_x + r.width as i32;
        layer_info.extend_from_slice(&top.to_be_bytes());
        layer_info.extend_from_slice(&left.to_be_bytes());
        layer_info.extend_from_slice(&bottom.to_be_bytes());
        layer_info.extend_from_slice(&right.to_be_bytes());

        let has_mask = r.mask.as_ref().map_or(false, |m| m.len() == r.width * r.height);
        let num_channels: u16 = if has_mask { 5 } else { 4 };
        layer_info.extend_from_slice(&num_channels.to_be_bytes());

        let total_pixels = r.width * r.height;
        let ch_len = 2 + total_pixels as u32;

        // Channel headers: 0=R, 1=G, 2=B, -1=A, -2=Mask
        layer_info.extend_from_slice(&0i16.to_be_bytes());
        layer_info.extend_from_slice(&ch_len.to_be_bytes());
        layer_info.extend_from_slice(&1i16.to_be_bytes());
        layer_info.extend_from_slice(&ch_len.to_be_bytes());
        layer_info.extend_from_slice(&2i16.to_be_bytes());
        layer_info.extend_from_slice(&ch_len.to_be_bytes());
        layer_info.extend_from_slice(&(-1i16).to_be_bytes());
        layer_info.extend_from_slice(&ch_len.to_be_bytes());
        if has_mask {
            layer_info.extend_from_slice(&(-2i16).to_be_bytes());
            layer_info.extend_from_slice(&ch_len.to_be_bytes());
        }

        // Blend signature and key
        layer_info.extend_from_slice(b"8BIM");
        layer_info.extend_from_slice(&l.blend_mode.to_psd_key());
        layer_info.push(l.opacity);
        layer_info.push(0); // clipping
        layer_info.push(if l.visible { 0 } else { 2 }); // flags
        layer_info.push(0); // filler

        // Extra data: mask_len + blend_ranges_len + Pascal string name
        let name_bytes = l.name.as_bytes();
        let name_len = name_bytes.len().min(255);
        let total_name_bytes = 1 + name_len;
        let pad = (4 - (total_name_bytes % 4)) % 4;
        let extra_len = 4 + 4 + total_name_bytes + pad;

        layer_info.extend_from_slice(&(extra_len as u32).to_be_bytes());
        layer_info.extend_from_slice(&0u32.to_be_bytes()); // mask_len
        layer_info.extend_from_slice(&0u32.to_be_bytes()); // blend_ranges_len
        layer_info.push(name_len as u8);
        layer_info.extend_from_slice(&name_bytes[..name_len]);
        for _ in 0..pad {
            layer_info.push(0);
        }

        // Channel image data (comp 0: Raw)
        // Red
        all_channel_data.extend_from_slice(&0u16.to_be_bytes());
        for p in &r.pixels {
            all_channel_data.push(((p >> 16) & 0xFF) as u8);
        }
        // Green
        all_channel_data.extend_from_slice(&0u16.to_be_bytes());
        for p in &r.pixels {
            all_channel_data.push(((p >> 8) & 0xFF) as u8);
        }
        // Blue
        all_channel_data.extend_from_slice(&0u16.to_be_bytes());
        for p in &r.pixels {
            all_channel_data.push((p & 0xFF) as u8);
        }
        // Alpha
        all_channel_data.extend_from_slice(&0u16.to_be_bytes());
        for p in &r.pixels {
            all_channel_data.push(((p >> 24) & 0xFF) as u8);
        }
        // Mask
        if has_mask {
            all_channel_data.extend_from_slice(&0u16.to_be_bytes());
            all_channel_data.extend_from_slice(r.mask.as_ref().unwrap());
        }
    }

    layer_info.extend_from_slice(&all_channel_data);

    let mut layer_section = Vec::new();
    layer_section.extend_from_slice(&(layer_info.len() as u32).to_be_bytes());
    layer_section.extend_from_slice(&layer_info);

    out.extend_from_slice(&(layer_section.len() as u32).to_be_bytes());
    out.extend_from_slice(&layer_section);

    // 5. Composite Image Data Section (flattened preview)
    let flat = doc.flatten()?;
    out.extend_from_slice(&0u16.to_be_bytes()); // comp 0 (Raw)
    for p in &flat.pixels {
        out.push(((p >> 16) & 0xFF) as u8);
    }
    for p in &flat.pixels {
        out.push(((p >> 8) & 0xFF) as u8);
    }
    for p in &flat.pixels {
        out.push((p & 0xFF) as u8);
    }
    for p in &flat.pixels {
        out.push(((p >> 24) & 0xFF) as u8);
    }

    Ok(out)
}

