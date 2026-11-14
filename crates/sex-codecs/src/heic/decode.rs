// SPDX-License-Identifier: GPL-3.0-only
//! HEIC / HEVC still picture decoding engine backed by pure-Rust `heic-rs`.

extern crate alloc;

use crate::Error;
use crate::format::Probe;
use crate::image::{alloc_pixels, Image};
#[cfg(not(feature = "heic"))]
use crate::isobmff::MiafMeta;
use crate::limits::{admit, Limits};


/// Probes metadata for HEIC images without allocating large framebuffers.
#[cfg(feature = "heic")]
pub fn probe_heic(bytes: &[u8]) -> Result<Probe, Error> {
    let info = heic_rs::probe(bytes).map_err(|_| Error::CorruptData)?;
    Ok(Probe {
        format: crate::Format::Heic,
        width: info.width as usize,
        height: info.height as usize,
        has_alpha: info.has_alpha,
        is_animated: false,
        frame_count: Some(1),
        bit_depth: info.bit_depth,
    })
}

/// Fallback probe when `heic` feature is not active.
#[cfg(not(feature = "heic"))]
pub fn probe_heic(bytes: &[u8]) -> Result<Probe, Error> {
    let meta = MiafMeta::parse(bytes)?;
    let props = meta.get_properties_for(meta.primary_item_id);

    let (mut width, mut height, bit_depth) = if let (Some(w), Some(h)) = (props.width, props.height) {
        (w as usize, h as usize, props.bit_depth.unwrap_or(8))
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

/// Decodes a HEIC still image into an `Image` with straight alpha ARGB pixels.
#[cfg(feature = "heic")]
pub fn decode_heic(bytes: &[u8], limits: &Limits) -> Result<Image, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let opts = heic_rs::DecodeOptions::default()
        .with_layout(heic_rs::PixelLayout::Rgba8)
        .with_max_pixels(Some(limits.max_pixels as u64))
        .with_transforms(true)
        .with_alpha(true);

    let img = heic_rs::decode(bytes, &opts).map_err(|e| match e {
        heic_rs::Error::PixelLimit { .. } => Error::TooLarge,
        _ => Error::CorruptData,
    })?;

    let width = img.width as usize;
    let height = img.height as usize;
    let count = admit(limits, width, height)?;
    let mut pixels = alloc_pixels(count)?;

    for (out, chunk) in pixels.iter_mut().zip(img.data.chunks_exact(4)) {
        *out = crate::image::pack_argb(chunk[3], chunk[0], chunk[1], chunk[2]);
    }

    Ok(Image {
        width,
        height,
        pixels,
    })
}

/// Fallback when `heic` feature is disabled.
#[cfg(not(feature = "heic"))]
pub fn decode_heic(_bytes: &[u8], _limits: &Limits) -> Result<Image, Error> {
    Err(Error::Unsupported)
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
