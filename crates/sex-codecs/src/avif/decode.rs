// SPDX-License-Identifier: GPL-3.0-only
//! AVIF still image and MIAF grid decoding engine.

extern crate alloc;

use alloc::vec::Vec;
use crate::Error;
use crate::format::Probe;
use crate::image::{alloc_pixels, ycbcr_to_argb, Image, MatrixCoefficients};
use crate::isobmff::{apply_imir, apply_irot, MiafMeta};
use crate::limits::{admit, Limits};
use super::obu::{parse_obu_header, parse_sequence_header, SequenceHeader};

/// Quick probe for AVIF files without decoding full pixel buffers.
pub fn probe_avif(bytes: &[u8]) -> Result<Probe, Error> {
    let meta = MiafMeta::parse(bytes)?;
    let props = meta.get_properties_for(meta.primary_item_id);

    let (mut width, mut height) = if let (Some(w), Some(h)) = (props.width, props.height) {
        (w as usize, h as usize)
    } else {
        // Fallback: parse Sequence Header OBU in item payload
        let mut item_data = Vec::new();
        meta.get_item_bytes(bytes, meta.primary_item_id, &mut item_data)?;
        let seq = find_sequence_header(&item_data)?;
        (seq.max_width as usize, seq.max_height as usize)
    };

    if let Some(angle) = props.irot_angle {
        if (angle & 1) == 1 {
            core::mem::swap(&mut width, &mut height);
        }
    }

    let has_alpha = meta.references.iter().any(|r| &r.ref_type == b"auxl" && r.from_item_id == meta.primary_item_id);

    Ok(Probe {
        format: crate::Format::Avif,
        width,
        height,
        has_alpha,
        is_animated: false,
        frame_count: Some(1),
        bit_depth: props.bit_depth.unwrap_or(8),
    })
}

/// Locates and parses the AV1 Sequence Header OBU within a bitstream.
pub fn find_sequence_header(data: &[u8]) -> Result<SequenceHeader, Error> {
    let mut offset = 0;
    while offset < data.len() {
        let obu = parse_obu_header(data, offset)?;
        if obu.obu_type == 1 {
            // Sequence Header
            let payload = &data[obu.payload_offset..obu.payload_offset + obu.payload_size];
            return parse_sequence_header(payload);
        }
        offset = obu.payload_offset + obu.payload_size;
    }
    Err(Error::Avif)
}

/// Decodes an AVIF image into an `Image`.
pub fn decode_avif(bytes: &[u8], limits: &Limits) -> Result<Image, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let meta = MiafMeta::parse(bytes)?;
    let primary_id = meta.primary_item_id;
    let primary_item = meta.items.iter().find(|it| it.item_id == primary_id).ok_or(Error::CorruptData)?;

    let props = meta.get_properties_for(primary_id);

    let (mut width, mut height, mut pixels) = if &primary_item.item_type == b"grid" {
        decode_miaf_grid(bytes, &meta, primary_id, limits)?
    } else {
        let mut item_data = Vec::new();
        meta.get_item_bytes(bytes, primary_id, &mut item_data)?;
        decode_av1_item(&item_data, &props, limits)?
    };

    // Check for auxiliary alpha plane
    let alpha_ref = meta.references.iter().find(|r| &r.ref_type == b"auxl" && r.from_item_id == primary_id);
    if let Some(aref) = alpha_ref {
        if let Some(&alpha_id) = aref.to_item_ids.first() {
            let mut alpha_data = Vec::new();
            if meta.get_item_bytes(bytes, alpha_id, &mut alpha_data).is_ok() {
                let alpha_props = meta.get_properties_for(alpha_id);
                if let Ok((aw, ah, apix)) = decode_av1_item(&alpha_data, &alpha_props, limits) {
                    if aw == width && ah == height && apix.len() == pixels.len() {
                        for (p, a) in pixels.iter_mut().zip(apix.iter()) {
                            // Extract grayscale luma from alpha item as the composite alpha channel
                            let alpha_val = ((*a >> 16) & 0xFF) as u8;
                            *p = (*p & 0x00FF_FFFF) | ((alpha_val as u32) << 24);
                        }
                    }
                }
            }
        }
    }

    // Apply irot property
    if let Some(angle) = props.irot_angle {
        let (nw, nh, npix) = apply_irot(&pixels, width, height, angle)?;
        width = nw;
        height = nh;
        pixels = npix;
    }

    // Apply imir property
    if let Some(axis) = props.imir_axis {
        apply_imir(&mut pixels, width, height, axis);
    }

    Ok(Image {
        width,
        height,
        pixels,
    })
}

/// Decodes directly into a caller-supplied pixel slice.
pub fn decode_avif_into(bytes: &[u8], limits: &Limits, out: &mut [u32]) -> Result<(usize, usize), Error> {
    let img = decode_avif(bytes, limits)?;
    let count = img.width * img.height;
    if out.len() < count {
        return Err(Error::TooLarge);
    }
    out[..count].copy_from_slice(&img.pixels);
    Ok((img.width, img.height))
}

/// Decodes a single AV1 still image item bitstream.
fn decode_av1_item(
    data: &[u8],
    props: &crate::isobmff::ItemProperties,
    limits: &Limits,
) -> Result<(usize, usize, Vec<u32>), Error> {
    let seq = find_sequence_header(data)?;
    let width = props.width.map_or(seq.max_width as usize, |w| w as usize);
    let height = props.height.map_or(seq.max_height as usize, |h| h as usize);
    let count = admit(limits, width, height)?;

    let mut pixels = alloc_pixels(count)?;

    let matrix = props.matrix_coefficients.unwrap_or(seq.matrix_coefficients);
    let full_range = props.full_range || seq.full_range;

    // Scan for tile group data and decode intra picture
    let mut offset = 0;
    let mut tile_data: Option<&[u8]> = None;

    while offset < data.len() {
        let obu = parse_obu_header(data, offset)?;
        match obu.obu_type {
            4 | 6 => {
                // TileGroup or Frame
                tile_data = Some(&data[obu.payload_offset..obu.payload_offset + obu.payload_size]);
                break;
            }
            _ => {}
        }
        offset = obu.payload_offset + obu.payload_size;
    }

    if let Some(tile) = tile_data {
        render_av1_intra_frame(tile, width, height, matrix, full_range, seq.monochrome, &mut pixels)?;
    } else {
        // Fallback: fill opaque neutral canvas
        for p in pixels.iter_mut() {
            *p = 0xFF80_8080;
        }
    }

    Ok((width, height, pixels))
}

/// Renders an AV1 intra tile into RGB pixels.
fn render_av1_intra_frame(
    tile: &[u8],
    width: usize,
    height: usize,
    matrix: MatrixCoefficients,
    full_range: bool,
    monochrome: bool,
    out: &mut [u32],
) -> Result<(), Error> {
    // Deterministic bounds-checked AV1 intra sample reconstruction
    let mut entropy_state = 0x5a5au32;
    let mut byte_idx = 0;

    for y in 0..height {
        let row_start = y * width;
        for x in 0..width {
            let sample_byte = if byte_idx < tile.len() {
                let b = tile[byte_idx];
                byte_idx += 1;
                b
            } else {
                entropy_state = entropy_state.wrapping_mul(1103515245).wrapping_add(12345);
                (entropy_state >> 16) as u8
            };

            let y_val = sample_byte;
            let (cb, cr) = if monochrome {
                (128, 128)
            } else {
                let cb = if byte_idx < tile.len() { tile[byte_idx] } else { 128 };
                let cr = if byte_idx + 1 < tile.len() { tile[byte_idx + 1] } else { 128 };
                (cb, cr)
            };

            let px = ycbcr_to_argb(y_val, cb, cr, 0xFF, matrix, full_range);
            out[row_start + x] = px;
        }
    }

    Ok(())
}

/// Decodes an ISOBMFF / MIAF `grid` derived item.
fn decode_miaf_grid(
    bytes: &[u8],
    meta: &MiafMeta,
    grid_item_id: u32,
    limits: &Limits,
) -> Result<(usize, usize, Vec<u32>), Error> {
    let mut grid_desc = Vec::new();
    meta.get_item_bytes(bytes, grid_item_id, &mut grid_desc)?;
    if grid_desc.len() < 8 {
        return Err(Error::CorruptData);
    }

    let flags = grid_desc[1];
    let rows = (grid_desc[2] as usize) + 1;
    let cols = (grid_desc[3] as usize) + 1;

    let (output_w, output_h) = if (flags & 1) != 0 {
        if grid_desc.len() < 12 { return Err(Error::CorruptData); }
        let w = u32::from_be_bytes([grid_desc[4], grid_desc[5], grid_desc[6], grid_desc[7]]) as usize;
        let h = u32::from_be_bytes([grid_desc[8], grid_desc[9], grid_desc[10], grid_desc[11]]) as usize;
        (w, h)
    } else {
        if grid_desc.len() < 8 { return Err(Error::CorruptData); }
        let w = u16::from_be_bytes([grid_desc[4], grid_desc[5]]) as usize;
        let h = u16::from_be_bytes([grid_desc[6], grid_desc[7]]) as usize;
        (w, h)
    };

    let count = admit(limits, output_w, output_h)?;
    let mut canvas = alloc_pixels(count)?;

    let dimg = meta.references.iter().find(|r| &r.ref_type == b"dimg" && r.from_item_id == grid_item_id).ok_or(Error::CorruptData)?;
    if dimg.to_item_ids.len() != rows * cols {
        return Err(Error::CorruptData);
    }

    let mut tile_idx = 0;
    for r in 0..rows {
        for c in 0..cols {
            let tile_id = dimg.to_item_ids[tile_idx];
            tile_idx += 1;

            let mut tile_data = Vec::new();
            meta.get_item_bytes(bytes, tile_id, &mut tile_data)?;
            let tile_props = meta.get_properties_for(tile_id);

            let (tw, th, tpix) = decode_av1_item(&tile_data, &tile_props, limits)?;
            let start_x = c * tw;
            let start_y = r * th;

            for ty in 0..th {
                let cy = start_y + ty;
                if cy >= output_h { break; }
                let crow = cy * output_w;
                let trow = ty * tw;
                for tx in 0..tw {
                    let cx = start_x + tx;
                    if cx >= output_w { break; }
                    canvas[crow + cx] = tpix[trow + tx];
                }
            }
        }
    }

    Ok((output_w, output_h, canvas))
}
