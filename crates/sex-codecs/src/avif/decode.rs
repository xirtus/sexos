// SPDX-License-Identifier: GPL-3.0-only
//! AVIF still image and MIAF grid decoding engine backed by `rusty_av1d`.

extern crate alloc;

use alloc::vec::Vec;
use crate::Error;
use crate::format::Probe;
use crate::image::Image;
#[cfg(feature = "avif")]
use crate::image::{alloc_pixels, ycbcr_to_argb, MatrixCoefficients};
use crate::isobmff::MiafMeta;
#[cfg(feature = "avif")]
use crate::isobmff::{apply_clap, apply_imir, apply_irot};
use crate::limits::Limits;
#[cfg(feature = "avif")]
use crate::limits::admit;

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

/// Decodes an AVIF image into an `Image` with straight alpha ARGB pixels.
#[cfg(feature = "avif")]
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

    // Apply clap property (clean aperture crop)
    if let Some(ref clap) = props.clap {
        let (cw, ch, cpix) = apply_clap(&pixels, width, height, clap)?;
        width = cw;
        height = ch;
        pixels = cpix;
    }

    // Apply irot property (rotation)
    if let Some(angle) = props.irot_angle {
        let (nw, nh, npix) = apply_irot(&pixels, width, height, angle)?;
        width = nw;
        height = nh;
        pixels = npix;
    }

    // Apply imir property (mirroring)
    if let Some(axis) = props.imir_axis {
        apply_imir(&mut pixels, width, height, axis);
    }

    Ok(Image {
        width,
        height,
        pixels,
    })
}

/// Fallback when `avif` feature is disabled.
#[cfg(not(feature = "avif"))]
pub fn decode_avif(_bytes: &[u8], _limits: &Limits) -> Result<Image, Error> {
    Err(Error::Unsupported)
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

/// Decodes a single AV1 still image item bitstream using `rusty_av1d`.
#[cfg(feature = "avif")]
fn decode_av1_item(
    data: &[u8],
    props: &crate::isobmff::ItemProperties,
    limits: &Limits,
) -> Result<(usize, usize, Vec<u32>), Error> {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_av1_item_inner(data, props, limits)
    }));
    match res {
        Ok(inner_res) => inner_res,
        Err(_) => Err(Error::Avif),
    }
}

#[cfg(feature = "avif")]
fn decode_av1_item_inner(
    data: &[u8],
    props: &crate::isobmff::ItemProperties,
    limits: &Limits,
) -> Result<(usize, usize, Vec<u32>), Error> {
    if data.is_empty() || data.len() > (usize::MAX / 2) {
        return Err(Error::CorruptData);
    }

    // Reject bitstreams that lack a valid AV1 Sequence Header before invoking the decoder.
    let _ = find_sequence_header(data)?;

    use rusty_av1d::rust_api::{Decoder, Settings, PlanarImageComponent, PixelLayout};

    let mut settings = Settings::new();
    settings.set_n_threads(1);
    settings.set_max_frame_delay(1);

    let mut decoder = Decoder::with_settings(&settings).map_err(|_| Error::Avif)?;
    decoder
        .send_data(data.to_vec().into_boxed_slice(), None, None, None)
        .map_err(|_| Error::Avif)?;

    let mut retries = 0;
    let pic = loop {
        match decoder.get_picture() {
            Ok(p) => break p,
            Err(rusty_av1d::Rav1dError::TryAgain) => {
                retries += 1;
                if retries > 10 {
                    return Err(Error::Avif);
                }
                continue;
            }
            Err(_) => return Err(Error::Avif),
        }
    };

    let width = pic.width() as usize;
    let height = pic.height() as usize;
    let count = admit(limits, width, height)?;
    let mut pixels = alloc_pixels(count)?;

    let y_plane = pic.plane(PlanarImageComponent::Y);
    let u_plane = pic.plane(PlanarImageComponent::U);
    let v_plane = pic.plane(PlanarImageComponent::V);
    let y_stride = pic.stride(PlanarImageComponent::Y) as usize;
    let u_stride = pic.stride(PlanarImageComponent::U) as usize;
    let v_stride = pic.stride(PlanarImageComponent::V) as usize;

    let full_range = match pic.color_range() {
        rusty_av1d::pixel::YUVRange::Full => true,
        rusty_av1d::pixel::YUVRange::Limited => false,
    };
    let matrix = match pic.matrix_coefficients() {
        rusty_av1d::pixel::MatrixCoefficients::BT709 => MatrixCoefficients::Bt709,
        rusty_av1d::pixel::MatrixCoefficients::BT2020NonConstantLuminance
        | rusty_av1d::pixel::MatrixCoefficients::BT2020ConstantLuminance => MatrixCoefficients::Bt2020NonConstant,
        _ => props.matrix_coefficients.unwrap_or(MatrixCoefficients::Bt601),
    };

    let layout = pic.pixel_layout();
    for y in 0..height {
        let y_row = y * y_stride;
        let out_row = y * width;
        let uv_y = match layout {
            PixelLayout::I420 => y / 2,
            _ => y,
        };
        let u_row = uv_y * u_stride;
        let v_row = uv_y * v_stride;

        for x in 0..width {
            let uv_x = match layout {
                PixelLayout::I420 | PixelLayout::I422 => x / 2,
                _ => x,
            };
            let y_val = y_plane[y_row + x];
            let (u_val, v_val) = if matches!(layout, PixelLayout::I400) {
                (128, 128)
            } else {
                (u_plane[u_row + uv_x], v_plane[v_row + uv_x])
            };
            pixels[out_row + x] = ycbcr_to_argb(y_val, u_val, v_val, 0xFF, matrix, full_range);
        }
    }

    Ok((width, height, pixels))
}

/// Decodes an ISOBMFF / MIAF `grid` derived item.
#[cfg(feature = "avif")]
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
