// SPDX-License-Identifier: GPL-3.0-only
//! WebP RIFF container and chunk demuxer.

extern crate alloc;

use alloc::vec::Vec;
use crate::Error;

/// Extended WebP container features announced in `VP8X`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vp8xHeader {
    pub has_icc: bool,
    pub has_alpha: bool,
    pub has_exif: bool,
    pub has_xmp: bool,
    pub has_animation: bool,
    pub canvas_width: usize,
    pub canvas_height: usize,
}

/// An animation frame chunk (`ANMF`).
#[derive(Clone, Debug)]
pub struct AnmfChunk<'a> {
    pub frame_x: usize,
    pub frame_y: usize,
    pub frame_width: usize,
    pub frame_height: usize,
    pub duration_ms: u32,
    pub blend: bool,              // true = alpha blend, false = overwrite
    pub dispose_background: bool, // true = dispose to background, false = keep
    pub bitstream: &'a [u8],
}

/// Top-level WebP structure demuxed from the RIFF container.
#[derive(Clone, Debug)]
pub struct WebpFile<'a> {
    pub vp8x: Option<Vp8xHeader>,
    pub still_bitstream: Option<&'a [u8]>,
    pub background_color: u32,
    pub loop_count: Option<u32>, // None = infinite
    pub frames: Vec<AnmfChunk<'a>>,
}

/// Demuxes a WebP file into its container components.
pub fn demux_webp(bytes: &[u8]) -> Result<WebpFile<'_>, Error> {
    if bytes.len() < 12 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WEBP" {
        return Err(Error::UnknownFormat);
    }

    let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let end = (8 + riff_size).min(bytes.len());
    let mut offset = 12;

    let mut vp8x: Option<Vp8xHeader> = None;
    let mut still_bitstream = None;
    let mut background_color = 0u32;
    let mut loop_count = None;
    let mut frames = Vec::new();

    while offset + 8 <= end {
        let fourcc = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            bytes[offset + 4], bytes[offset + 5], bytes[offset + 6], bytes[offset + 7],
        ]) as usize;
        let data_start = offset + 8;
        let data_end = (data_start + chunk_size).min(end);
        let payload = &bytes[data_start..data_end];

        match fourcc {
            b"VP8X" => {
                if payload.len() >= 10 {
                    let flags = payload[0];
                    let has_icc = (flags & (1 << 5)) != 0;
                    let has_alpha = (flags & (1 << 4)) != 0;
                    let has_exif = (flags & (1 << 3)) != 0;
                    let has_xmp = (flags & (1 << 2)) != 0;
                    let has_animation = (flags & (1 << 1)) != 0;

                    let cw = u32::from_le_bytes([payload[4], payload[5], payload[6], 0]) as usize + 1;
                    let ch = u32::from_le_bytes([payload[7], payload[8], payload[9], 0]) as usize + 1;

                    vp8x = Some(Vp8xHeader {
                        has_icc,
                        has_alpha,
                        has_exif,
                        has_xmp,
                        has_animation,
                        canvas_width: cw,
                        canvas_height: ch,
                    });
                }
            }
            b"ANIM" => {
                if payload.len() >= 6 {
                    background_color = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    let lc = u16::from_le_bytes([payload[4], payload[5]]) as u32;
                    loop_count = if lc == 0 { None } else { Some(lc) };
                }
            }
            b"ANMF" => {
                if payload.len() >= 16 {
                    let fx = u32::from_le_bytes([payload[0], payload[1], payload[2], 0]) as usize * 2;
                    let fy = u32::from_le_bytes([payload[3], payload[4], payload[5], 0]) as usize * 2;
                    let fw = u32::from_le_bytes([payload[6], payload[7], payload[8], 0]) as usize + 1;
                    let fh = u32::from_le_bytes([payload[9], payload[10], payload[11], 0]) as usize + 1;
                    if let Some(ref v) = vp8x {
                        let end_x = fx.checked_add(fw).ok_or(Error::TooLarge)?;
                        let end_y = fy.checked_add(fh).ok_or(Error::TooLarge)?;
                        if end_x > v.canvas_width || end_y > v.canvas_height {
                            return Err(Error::TooLarge);
                        }
                    }
                    let dur = u32::from_le_bytes([payload[12], payload[13], payload[14], 0]);
                    let flags = payload[15];
                    let dispose_background = (flags & 1) != 0;
                    let blend = (flags & 2) == 0; // bit 1: 0 = blend, 1 = do not blend

                    let frame_bitstream = &payload[16..];
                    frames.push(AnmfChunk {
                        frame_x: fx,
                        frame_y: fy,
                        frame_width: fw,
                        frame_height: fh,
                        duration_ms: if dur <= 10 { 100 } else { dur },
                        blend,
                        dispose_background,
                        bitstream: frame_bitstream,
                    });
                }
            }
            b"VP8 " | b"VP8L" => {
                if still_bitstream.is_none() {
                    still_bitstream = Some(&bytes[offset..data_end]);
                }
            }
            _ => {}
        }

        // Chunks are padded to even sizes
        let padded = (chunk_size + 1) & !1;
        offset = offset.checked_add(8 + padded).unwrap_or(end);
    }

    Ok(WebpFile {
        vp8x,
        still_bitstream,
        background_color,
        loop_count,
        frames,
    })
}
