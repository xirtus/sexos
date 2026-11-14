// SPDX-License-Identifier: GPL-3.0-only
//! Bounded RGBA PNG encoder for Batik layer storage and flattened exports.

use alloc::vec::Vec;
use crate::error::BatikError;

const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

const fn make_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            if c & 1 != 0 {
                c = 0xEDB8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            j += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const CRC_TABLE: [u32; 256] = make_crc_table();

fn crc32(type_bytes: &[u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in type_bytes {
        crc = CRC_TABLE[((crc ^ (b as u32)) & 0xFF) as usize] ^ (crc >> 8);
    }
    for &b in data {
        crc = CRC_TABLE[((crc ^ (b as u32)) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

fn write_chunk(out: &mut Vec<u8>, type_name: &[u8; 4], data: &[u8]) {
    let len = data.len() as u32;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(type_name);
    out.extend_from_slice(data);
    let crc = crc32(type_name, data);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encodes a straight-alpha `0xAARRGGBB` pixel buffer into a standard 32-bit RGBA PNG.
pub fn encode_rgba_png(pixels: &[u32], width: usize, height: usize) -> Result<Vec<u8>, BatikError> {
    if width == 0 || height == 0 {
        return Err(BatikError::InvalidDimensions);
    }
    let expected_len = width.checked_mul(height).ok_or(BatikError::TooLarge)?;
    if pixels.len() < expected_len {
        return Err(BatikError::InvalidDimensions);
    }

    let mut out = Vec::new();
    out.extend_from_slice(&SIGNATURE);

    // IHDR chunk: 13 bytes
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&(width as u32).to_be_bytes());
    ihdr[4..8].copy_from_slice(&(height as u32).to_be_bytes());
    ihdr[8] = 8; // 8 bits per channel
    ihdr[9] = 6; // color type 6: RGBA
    ihdr[10] = 0; // compression 0
    ihdr[11] = 0; // filter 0
    ihdr[12] = 0; // interlace 0
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Prepare raw scanlines with filter byte 0 (None) for each row
    let row_bytes = width.checked_mul(4).and_then(|w| w.checked_add(1)).ok_or(BatikError::TooLarge)?;
    let total_raw = row_bytes.checked_mul(height).ok_or(BatikError::TooLarge)?;

    let mut raw = Vec::new();
    raw.try_reserve_exact(total_raw).map_err(|_| BatikError::OutOfMemory)?;

    for y in 0..height {
        raw.push(0); // filter byte: None
        let row_start = y * width;
        for x in 0..width {
            let p = pixels[row_start + x];
            let a = ((p >> 24) & 0xFF) as u8;
            let r = ((p >> 16) & 0xFF) as u8;
            let g = ((p >> 8) & 0xFF) as u8;
            let b = (p & 0xFF) as u8;
            raw.push(r);
            raw.push(g);
            raw.push(b);
            raw.push(a);
        }
    }

    // Compress raw IDAT with zlib deflate
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    write_chunk(&mut out, b"IDAT", &compressed);

    // IEND chunk
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}
