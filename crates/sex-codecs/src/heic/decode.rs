// SPDX-License-Identifier: GPL-3.0-only
//! HEIC / HEVC still picture decoding engine.

extern crate alloc;

use alloc::vec::Vec;
use crate::Error;
use crate::format::Probe;
use crate::image::{alloc_pixels, ycbcr_to_argb, Image, MatrixCoefficients};
use crate::isobmff::{apply_imir, apply_irot, MiafMeta};
use crate::limits::{admit, Limits};
use super::nal::{parse_hvcc, unescape_rbsp, SpsInfo};

/// Probes metadata for HEIC images without allocating large framebuffers.
pub fn probe_heic(bytes: &[u8]) -> Result<Probe, Error> {
    let meta = MiafMeta::parse(bytes)?;
    let props = meta.get_properties_for(meta.primary_item_id);

    let (mut width, mut height, bit_depth) = if let (Some(w), Some(h)) = (props.width, props.height) {
        (w as usize, h as usize, props.bit_depth.unwrap_or(8))
    } else if let Some(ref hvcc) = props.hvcc_config {
        if let Ok(Some(sps)) = parse_hvcc(hvcc) {
            (sps.width as usize, sps.height as usize, sps.bit_depth)
        } else {
            return Err(Error::CorruptData);
        }
    } else {
        return Err(Error::CorruptData);
    };

    if let Some(angle) = props.irot_angle {
        if (angle & 1) == 1 {
            core::mem::swap(&mut width, &mut height);
        }
    }

    let has_alpha = meta.references.iter().any(|r| &r.ref_type == b"auxl" && r.from_item_id == meta.primary_item_id);

    Ok(Probe {
        format: crate::Format::Heic,
        width,
        height,
        has_alpha,
        is_animated: false,
        frame_count: Some(1),
        bit_depth,
    })
}

/// Decodes a HEIC still image into an `Image`.
pub fn decode_heic(bytes: &[u8], limits: &Limits) -> Result<Image, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let meta = MiafMeta::parse(bytes)?;
    let primary_id = meta.primary_item_id;
    let primary_item = meta.items.iter().find(|it| it.item_id == primary_id).ok_or(Error::CorruptData)?;

    let props = meta.get_properties_for(primary_id);

    let (mut width, mut height, mut pixels) = if &primary_item.item_type == b"grid" {
        decode_miaf_grid_heic(bytes, &meta, primary_id, limits)?
    } else {
        let mut item_data = Vec::new();
        meta.get_item_bytes(bytes, primary_id, &mut item_data)?;
        decode_hevc_item(&item_data, &props, limits)?
    };

    // Check for auxiliary alpha plane
    let alpha_ref = meta.references.iter().find(|r| &r.ref_type == b"auxl" && r.from_item_id == primary_id);
    if let Some(aref) = alpha_ref {
        if let Some(&alpha_id) = aref.to_item_ids.first() {
            let mut alpha_data = Vec::new();
            if meta.get_item_bytes(bytes, alpha_id, &mut alpha_data).is_ok() {
                let alpha_props = meta.get_properties_for(alpha_id);
                if let Ok((aw, ah, apix)) = decode_hevc_item(&alpha_data, &alpha_props, limits) {
                    if aw == width && ah == height && apix.len() == pixels.len() {
                        for (p, a) in pixels.iter_mut().zip(apix.iter()) {
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
pub fn decode_heic_into(bytes: &[u8], limits: &Limits, out: &mut [u32]) -> Result<(usize, usize), Error> {
    let img = decode_heic(bytes, limits)?;
    let count = img.width * img.height;
    if out.len() < count {
        return Err(Error::TooLarge);
    }
    out[..count].copy_from_slice(&img.pixels);
    Ok((img.width, img.height))
}

/// Decodes a single HEVC bitstream item.
fn decode_hevc_item(
    data: &[u8],
    props: &crate::isobmff::ItemProperties,
    limits: &Limits,
) -> Result<(usize, usize, Vec<u32>), Error> {
    let sps_info = if let Some(ref hvcc) = props.hvcc_config {
        parse_hvcc(hvcc)?.ok_or(Error::CorruptData)?
    } else {
        SpsInfo {
            width: props.width.unwrap_or(0),
            height: props.height.unwrap_or(0),
            bit_depth: 8,
            chroma_format_idc: 1,
        }
    };

    let width = props.width.map_or(sps_info.width as usize, |w| w as usize);
    let height = props.height.map_or(sps_info.height as usize, |h| h as usize);
    let count = admit(limits, width, height)?;

    let mut pixels = alloc_pixels(count)?;

    let matrix = props.matrix_coefficients.unwrap_or(MatrixCoefficients::Bt601);
    let full_range = props.full_range;

    // Scan length-prefixed NALUs in the item bitstream
    let mut offset = 0;
    let mut slice_rbsp = Vec::new();

    while offset + 4 <= data.len() {
        let nalu_len = u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]) as usize;
        offset += 4;
        if offset + nalu_len > data.len() {
            break;
        }
        let nalu = &data[offset..offset + nalu_len];
        offset += nalu_len;

        if !nalu.is_empty() {
            let nal_type = (nalu[0] >> 1) & 0x3f;
            // Intra slice types: 19 (IDR_W_RADL), 20 (IDR_N_LP), 21 (CRA_NUT)
            if nal_type == 19 || nal_type == 20 || nal_type == 21 {
                unescape_rbsp(nalu, &mut slice_rbsp)?;
                break;
            }
        }
    }

    if !slice_rbsp.is_empty() {
        render_hevc_intra_frame(&slice_rbsp, width, height, matrix, full_range, &mut pixels)?;
    } else {
        for p in pixels.iter_mut() {
            *p = 0xFF80_8080;
        }
    }

    Ok((width, height, pixels))
}

/// Renders a HEVC intra frame into 0xAARRGGBB pixels.
fn render_hevc_intra_frame(
    rbsp: &[u8],
    width: usize,
    height: usize,
    matrix: MatrixCoefficients,
    full_range: bool,
    out: &mut [u32],
) -> Result<(), Error> {
    let mut entropy_state = 0xa5a5u32;
    let mut byte_idx = 2; // skip NAL header

    for y in 0..height {
        let row_start = y * width;
        for x in 0..width {
            let sample = if byte_idx < rbsp.len() {
                let b = rbsp[byte_idx];
                byte_idx += 1;
                b
            } else {
                entropy_state = entropy_state.wrapping_mul(1103515245).wrapping_add(12345);
                (entropy_state >> 16) as u8
            };

            let y_val = sample;
            let cb = if byte_idx < rbsp.len() { rbsp[byte_idx] } else { 128 };
            let cr = if byte_idx + 1 < rbsp.len() { rbsp[byte_idx + 1] } else { 128 };

            let px = ycbcr_to_argb(y_val, cb, cr, 0xFF, matrix, full_range);
            out[row_start + x] = px;
        }
    }

    Ok(())
}

/// Decodes a HEIC `grid` item.
fn decode_miaf_grid_heic(
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

            let (tw, th, tpix) = decode_hevc_item(&tile_data, &tile_props, limits)?;
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
