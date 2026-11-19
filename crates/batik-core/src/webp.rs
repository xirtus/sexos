// SPDX-License-Identifier: GPL-3.0-only
//! Pure-Rust lossless WebP (VP8L) encoder for confined SexOS programs.

use alloc::vec::Vec;
use crate::error::BatikError;

struct BitWriter {
    bytes: Vec<u8>,
    buffer: u64,
    nbits: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            buffer: 0,
            nbits: 0,
        }
    }

    fn write_bits(&mut self, val: u64, num_bits: u8) {
        if num_bits == 0 {
            return;
        }
        let mask = if num_bits >= 64 { !0 } else { (1u64 << num_bits) - 1 };
        self.buffer |= (val & mask) << self.nbits;
        self.nbits += num_bits;
        while self.nbits >= 8 {
            self.bytes.push(self.buffer as u8);
            self.buffer >>= 8;
            self.nbits -= 8;
        }
    }

    fn flush(&mut self) {
        if self.nbits > 0 {
            self.bytes.push(self.buffer as u8);
            self.buffer = 0;
            self.nbits = 0;
        }
    }
}

/// Encodes unpremultiplied `0xAARRGGBB` pixels into a lossless WebP (VP8L) container.
pub fn encode_webp(pixels: &[u32], width: usize, height: usize) -> Result<Vec<u8>, BatikError> {
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(BatikError::InvalidDimensions);
    }
    let total = width.checked_mul(height).ok_or(BatikError::TooLarge)?;
    if pixels.len() != total {
        return Err(BatikError::InvalidDimensions);
    }

    let mut has_alpha = false;
    for &p in pixels {
        if (p >> 24) != 0xFF {
            has_alpha = true;
            break;
        }
    }

    let mut writer = BitWriter::new();
    let est_bytes = 32 + total * 4;
    writer.bytes.try_reserve(est_bytes).map_err(|_| BatikError::OutOfMemory)?;

    // 1. VP8L Header:
    // 8 bits signature (0x2F)
    writer.write_bits(0x2F, 8);
    // 14 bits (width - 1)
    writer.write_bits((width - 1) as u64, 14);
    // 14 bits (height - 1)
    writer.write_bits((height - 1) as u64, 14);
    // 1 bit alpha_is_used
    writer.write_bits(if has_alpha { 1 } else { 0 }, 1);
    // 3 bits version (0)
    writer.write_bits(0, 3);

    // 2. Transforms: 1 bit = 0 (no transforms)
    writer.write_bits(0, 1);

    // 3. Color Cache: 1 bit = 0 (no color cache)
    writer.write_bits(0, 1);

    // 4. Meta Huffman: 1 bit = 0 (single Huffman code group)
    writer.write_bits(0, 1);

    // 5. Huffman Code Group (5 trees: Green/Literal, Red, Blue, Alpha, Distance)

    // Tree 0: Green (Alphabet size = 256 + 24 = 280)
    // We define a canonical tree where symbols 0..=255 have code length 8, and symbols 256..=279 have length 0.
    // simple = 0 (1 bit)
    writer.write_bits(0, 1);
    // num_code_lengths: 4 + 8 = 12 (4 bits: 8)
    writer.write_bits(8, 4);
    // CODE_LENGTH_CODE_ORDER: [17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, ...]
    // Index 11 corresponds to symbol 8. Indices 0..=10 are length 0, index 11 is length 1.
    for _ in 0..11 {
        writer.write_bits(0, 3);
    }
    writer.write_bits(1, 3); // symbol 8 has code length 1

    // max_symbol flag = 1 (1 bit)
    writer.write_bits(1, 1);
    // length_nbits: 2 + 2 * 3 = 8 bits (3 bits: 3)
    writer.write_bits(3, 3);
    // max_minus_two = 254 (8 bits: 254 -> max_symbol = 256)
    writer.write_bits(254, 8);

    // Trees 1, 2, 3: Red, Blue, Alpha (Alphabet size = 256 each)
    // All 256 symbols have code length 8.
    for _ in 0..3 {
        writer.write_bits(0, 1); // simple = 0
        writer.write_bits(8, 4); // 12 code lengths
        for _ in 0..11 {
            writer.write_bits(0, 3);
        }
        writer.write_bits(1, 3); // symbol 8 has length 1
        writer.write_bits(0, 1); // max_symbol = num_symbols (256)
    }

    // Tree 4: Distance (Alphabet size = 40)
    // We emit a simple 1-symbol tree for symbol 0 (unused since we emit only literals).
    writer.write_bits(1, 1); // simple = 1
    writer.write_bits(0, 1); // num_symbols - 1 = 0 (1 symbol)
    writer.write_bits(1, 1); // is_first_8bits = 1
    writer.write_bits(0, 8); // symbol = 0

    // 6. Image Data: emit each pixel as Green, Red, Blue, Alpha
    for &p in pixels {
        let a = ((p >> 24) & 0xFF) as u8;
        let r = ((p >> 16) & 0xFF) as u8;
        let g = ((p >> 8) & 0xFF) as u8;
        let b = (p & 0xFF) as u8;

        // In our canonical Huffman tree of length 8, symbol s gets code s.
        // Because the bit reader reads LSB-first, the bits in the stream must be s.reverse_bits().
        writer.write_bits(g.reverse_bits() as u64, 8);
        writer.write_bits(r.reverse_bits() as u64, 8);
        writer.write_bits(b.reverse_bits() as u64, 8);
        writer.write_bits(a.reverse_bits() as u64, 8);
    }

    writer.flush();

    let vp8l_data_len = writer.bytes.len();
    let pad = (vp8l_data_len % 2 != 0) as usize;

    // 7. RIFF Container:
    // "RIFF" [file_len - 8] "WEBP" "VP8L" [vp8l_len] [vp8l_data] [pad]
    let riff_payload = 4 + 8 + vp8l_data_len + pad;
    let mut out = Vec::new();
    out.try_reserve(8 + riff_payload).map_err(|_| BatikError::OutOfMemory)?;

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(riff_payload as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(b"VP8L");
    out.extend_from_slice(&(vp8l_data_len as u32).to_le_bytes());
    out.extend_from_slice(&writer.bytes);
    if pad != 0 {
        out.push(0);
    }

    Ok(out)
}
