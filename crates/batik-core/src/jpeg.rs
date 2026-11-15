// SPDX-License-Identifier: GPL-3.0-only
//! Pure-Rust baseline DCT JPEG encoder for confined SexOS programs.

use alloc::vec::Vec;
use crate::error::BatikError;

const STD_LUM_Q: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61,
    12, 12, 14, 19, 26, 58, 60, 55,
    14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62,
    18, 22, 37, 56, 68, 109, 103, 77,
    24, 35, 55, 64, 81, 104, 113, 92,
    49, 64, 78, 87, 103, 121, 120, 101,
    72, 92, 95, 98, 112, 100, 103, 99,
];

const STD_CHROM_Q: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99,
    18, 21, 26, 66, 99, 99, 99, 99,
    24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
];

const ZIGZAG_ORDER: [usize; 64] = [
     0,  1,  8, 16,  9,  2,  3, 10,
    17, 24, 32, 25, 18, 11,  4,  5,
    12, 19, 26, 33, 40, 48, 41, 34,
    27, 20, 13,  6,  7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36,
    29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46,
    53, 60, 61, 54, 47, 55, 62, 63,
];

const BITS_DC_LUM: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const VALS_DC_LUM: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

const BITS_AC_LUM: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7D];
const VALS_AC_LUM: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12,
    0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08,
    0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16,
    0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39,
    0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
    0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79,
    0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98,
    0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
    0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6,
    0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5,
    0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4,
    0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA,
    0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
];

const BITS_DC_CHROM: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const VALS_DC_CHROM: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

const BITS_AC_CHROM: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const VALS_AC_CHROM: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21,
    0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91,
    0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0,
    0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34,
    0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26,
    0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38,
    0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58,
    0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78,
    0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96,
    0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5,
    0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4,
    0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3,
    0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2,
    0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA,
    0x2B, 0x3B, 0x4B, 0x5B, 0x6B, 0x7B, 0x8B, 0x9B,
    0xAB, 0xBB, 0xCB, 0xDB, 0xEB, 0xFB, 0xE2, 0xE3,
    0xE4, 0xE5,
];

struct HuffmanTable {
    codes: [u16; 256],
    lengths: [u8; 256],
}

fn build_huffman_table(bits: &[u8; 16], vals: &[u8]) -> HuffmanTable {
    let mut codes = [0u16; 256];
    let mut lengths = [0u8; 256];
    let mut code: u16 = 0;
    let mut k = 0;
    for len in 1..=16 {
        let count = bits[len - 1] as usize;
        for _ in 0..count {
            if k < vals.len() {
                let val = vals[k] as usize;
                codes[val] = code;
                lengths[val] = len as u8;
                code += 1;
                k += 1;
            }
        }
        code <<= 1;
    }
    HuffmanTable { codes, lengths }
}

struct JpegBitWriter {
    bytes: Vec<u8>,
    buffer: u32,
    nbits: u8,
}

impl JpegBitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            buffer: 0,
            nbits: 0,
        }
    }

    fn write_bits(&mut self, val: u32, num_bits: u8) {
        if num_bits == 0 {
            return;
        }
        let mask = if num_bits >= 32 { !0 } else { (1u32 << num_bits) - 1 };
        self.buffer = (self.buffer << num_bits) | (val & mask);
        self.nbits += num_bits;
        while self.nbits >= 8 {
            self.nbits -= 8;
            let b = ((self.buffer >> self.nbits) & 0xFF) as u8;
            self.bytes.push(b);
            if b == 0xFF {
                self.bytes.push(0x00);
            }
        }
    }

    fn flush(&mut self) {
        if self.nbits > 0 {
            let pad = 8 - self.nbits;
            let b = ((self.buffer << pad) | ((1u32 << pad) - 1)) as u8;
            self.bytes.push(b);
            if b == 0xFF {
                self.bytes.push(0x00);
            }
            self.buffer = 0;
            self.nbits = 0;
        }
    }
}

fn scale_quant_table(std_table: &[u8; 64], quality: u8) -> [u8; 64] {
    let q = quality.clamp(1, 100) as u32;
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let mut out = [0u8; 64];
    for i in 0..64 {
        let val = ((std_table[i] as u32 * scale + 50) / 100).clamp(1, 255);
        out[i] = val as u8;
    }
    out
}

fn fdct_8x8(input: &[f32; 64], output: &mut [f32; 64]) {
    let mut temp = [0.0f32; 64];
    for y in 0..8 {
        let row_off = y * 8;
        for u in 0..8 {
            let cu = if u == 0 { 0.35355339 } else { 0.5 };
            let mut sum = 0.0f32;
            for x in 0..8 {
                let angle = (2.0 * x as f32 + 1.0) * (u as f32) * 3.141592653589793 / 16.0;
                sum += input[row_off + x] * libm::cosf(angle);
            }
            temp[row_off + u] = cu * sum;
        }
    }
    for x in 0..8 {
        for v in 0..8 {
            let cv = if v == 0 { 0.35355339 } else { 0.5 };
            let mut sum = 0.0f32;
            for y in 0..8 {
                let angle = (2.0 * y as f32 + 1.0) * (v as f32) * 3.141592653589793 / 16.0;
                sum += temp[y * 8 + x] * libm::cosf(angle);
            }
            output[v * 8 + x] = cv * sum;
        }
    }
}

fn category(v: i16) -> (u8, u16) {
    if v == 0 {
        return (0, 0);
    }
    let abs_v = v.unsigned_abs();
    let cat = (16 - abs_v.leading_zeros()) as u8;
    let code_val = if v > 0 {
        v as u16
    } else {
        ((v as i32) + ((1 << cat) - 1)) as u16
    };
    (cat, code_val)
}

fn encode_block(
    writer: &mut JpegBitWriter,
    block: &[i16; 64],
    prev_dc: &mut i16,
    dc_table: &HuffmanTable,
    ac_table: &HuffmanTable,
) {
    let dc_diff = block[0] - *prev_dc;
    *prev_dc = block[0];
    let (cat, val) = category(dc_diff);
    let dc_len = dc_table.lengths[cat as usize];
    let dc_code = dc_table.codes[cat as usize];
    writer.write_bits(dc_code as u32, dc_len);
    if cat > 0 {
        writer.write_bits(val as u32, cat);
    }

    let mut last_non_zero = 0;
    for i in (1..64).rev() {
        if block[i] != 0 {
            last_non_zero = i;
            break;
        }
    }

    if last_non_zero == 0 {
        let eob_len = ac_table.lengths[0];
        let eob_code = ac_table.codes[0];
        writer.write_bits(eob_code as u32, eob_len);
        return;
    }

    let mut zero_run = 0u8;
    for i in 1..=last_non_zero {
        let coeff = block[i];
        if coeff == 0 {
            zero_run += 1;
            if zero_run == 16 {
                let zrl_len = ac_table.lengths[0xF0];
                let zrl_code = ac_table.codes[0xF0];
                writer.write_bits(zrl_code as u32, zrl_len);
                zero_run = 0;
            }
        } else {
            let (cat, val) = category(coeff);
            let symbol = (zero_run << 4) | cat;
            let ac_len = ac_table.lengths[symbol as usize];
            let ac_code = ac_table.codes[symbol as usize];
            writer.write_bits(ac_code as u32, ac_len);
            writer.write_bits(val as u32, cat);
            zero_run = 0;
        }
    }

    if last_non_zero < 63 {
        let eob_len = ac_table.lengths[0];
        let eob_code = ac_table.codes[0];
        writer.write_bits(eob_code as u32, eob_len);
    }
}

/// Encodes unpremultiplied `0xAARRGGBB` pixels into a baseline DCT JPEG file (4:4:4).
pub fn encode_jpeg(
    pixels: &[u32],
    width: usize,
    height: usize,
    quality: u8,
) -> Result<Vec<u8>, BatikError> {
    if width == 0 || height == 0 || width > 65535 || height > 65535 {
        return Err(BatikError::InvalidDimensions);
    }
    let total = width.checked_mul(height).ok_or(BatikError::TooLarge)?;
    if pixels.len() != total {
        return Err(BatikError::InvalidDimensions);
    }

    let q_lum = scale_quant_table(&STD_LUM_Q, quality);
    let q_chrom = scale_quant_table(&STD_CHROM_Q, quality);

    let dc_lum_tbl = build_huffman_table(&BITS_DC_LUM, &VALS_DC_LUM);
    let ac_lum_tbl = build_huffman_table(&BITS_AC_LUM, &VALS_AC_LUM);
    let dc_chrom_tbl = build_huffman_table(&BITS_DC_CHROM, &VALS_DC_CHROM);
    let ac_chrom_tbl = build_huffman_table(&BITS_AC_CHROM, &VALS_AC_CHROM);

    let mut out = Vec::new();
    out.try_reserve(1024 + total).map_err(|_| BatikError::OutOfMemory)?;

    // 1. SOI
    out.extend_from_slice(&[0xFF, 0xD8]);

    // 2. APP0 (JFIF 1.1)
    out.extend_from_slice(&[
        0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
    ]);

    // 3. DQT (Quantization Tables: 0 for Lum, 1 for Chrom, in zig-zag order)
    out.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x84]);
    out.push(0x00); // Table 0
    for &idx in &ZIGZAG_ORDER {
        out.push(q_lum[idx]);
    }
    out.push(0x01); // Table 1
    for &idx in &ZIGZAG_ORDER {
        out.push(q_chrom[idx]);
    }

    // 4. SOF0 (Baseline DCT, 8-bit, 3 components, 1x1 4:4:4)
    out.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
    out.extend_from_slice(&(height as u16).to_be_bytes());
    out.extend_from_slice(&(width as u16).to_be_bytes());
    out.push(0x03); // 3 components
    out.extend_from_slice(&[0x01, 0x11, 0x00]); // Y: id=1, sample=1x1, q=0
    out.extend_from_slice(&[0x02, 0x11, 0x01]); // Cb: id=2, sample=1x1, q=1
    out.extend_from_slice(&[0x03, 0x11, 0x01]); // Cr: id=3, sample=1x1, q=1

    // 5. DHT (Huffman Tables)
    // Table 0x00: DC Lum
    out.extend_from_slice(&[0xFF, 0xC4, 0x00, (3 + 16 + VALS_DC_LUM.len()) as u8, 0x00]);
    out.extend_from_slice(&BITS_DC_LUM);
    out.extend_from_slice(&VALS_DC_LUM);

    // Table 0x10: AC Lum
    out.extend_from_slice(&[0xFF, 0xC4, 0x00, (3 + 16 + VALS_AC_LUM.len()) as u8, 0x10]);
    out.extend_from_slice(&BITS_AC_LUM);
    out.extend_from_slice(&VALS_AC_LUM);

    // Table 0x01: DC Chrom
    out.extend_from_slice(&[0xFF, 0xC4, 0x00, (3 + 16 + VALS_DC_CHROM.len()) as u8, 0x01]);
    out.extend_from_slice(&BITS_DC_CHROM);
    out.extend_from_slice(&VALS_DC_CHROM);

    // Table 0x11: AC Chrom
    out.extend_from_slice(&[0xFF, 0xC4, 0x00, (3 + 16 + VALS_AC_CHROM.len()) as u8, 0x11]);
    out.extend_from_slice(&BITS_AC_CHROM);
    out.extend_from_slice(&VALS_AC_CHROM);

    // 6. SOS (Start of Scan)
    out.extend_from_slice(&[
        0xFF, 0xDA, 0x00, 0x0C, 0x03,
        0x01, 0x00, // Y: dc=0, ac=0
        0x02, 0x11, // Cb: dc=1, ac=1
        0x03, 0x11, // Cr: dc=1, ac=1
        0x00, 0x3F, 0x00, // spectral selection 0..63, approx=0
    ]);

    // 7. Entropy Coded Data
    let mut writer = JpegBitWriter::new();
    let mcus_x = (width + 7) / 8;
    let mcus_y = (height + 7) / 8;

    let mut prev_dc_y = 0i16;
    let mut prev_dc_cb = 0i16;
    let mut prev_dc_cr = 0i16;

    let mut block_y = [0.0f32; 64];
    let mut block_cb = [0.0f32; 64];
    let mut block_cr = [0.0f32; 64];

    let mut dct_out = [0.0f32; 64];
    let mut zz_block = [0i16; 64];

    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            // Load 8x8 pixels into Y, Cb, Cr blocks
            for by in 0..8 {
                let py = (my * 8 + by).min(height - 1);
                let row_off = py * width;
                for bx in 0..8 {
                    let px = (mx * 8 + bx).min(width - 1);
                    let pixel = pixels[row_off + px];
                    let r = ((pixel >> 16) & 0xFF) as f32;
                    let g = ((pixel >> 8) & 0xFF) as f32;
                    let b = (pixel & 0xFF) as f32;

                    let y = 0.299 * r + 0.587 * g + 0.114 * b - 128.0;
                    let cb = -0.168736 * r - 0.331264 * g + 0.5 * b;
                    let cr = 0.5 * r - 0.418688 * g - 0.081312 * b;

                    let idx = by * 8 + bx;
                    block_y[idx] = y;
                    block_cb[idx] = cb;
                    block_cr[idx] = cr;
                }
            }

            // Encode Y block
            fdct_8x8(&block_y, &mut dct_out);
            for (k, &nat) in ZIGZAG_ORDER.iter().enumerate() {
                let q = q_lum[nat] as f32;
                let c = libm::roundf(dct_out[nat] / q) as i16;
                zz_block[k] = c;
            }
            encode_block(&mut writer, &zz_block, &mut prev_dc_y, &dc_lum_tbl, &ac_lum_tbl);

            // Encode Cb block
            fdct_8x8(&block_cb, &mut dct_out);
            for (k, &nat) in ZIGZAG_ORDER.iter().enumerate() {
                let q = q_chrom[nat] as f32;
                let c = libm::roundf(dct_out[nat] / q) as i16;
                zz_block[k] = c;
            }
            encode_block(&mut writer, &zz_block, &mut prev_dc_cb, &dc_chrom_tbl, &ac_chrom_tbl);

            // Encode Cr block
            fdct_8x8(&block_cr, &mut dct_out);
            for (k, &nat) in ZIGZAG_ORDER.iter().enumerate() {
                let q = q_chrom[nat] as f32;
                let c = libm::roundf(dct_out[nat] / q) as i16;
                zz_block[k] = c;
            }
            encode_block(&mut writer, &zz_block, &mut prev_dc_cr, &dc_chrom_tbl, &ac_chrom_tbl);
        }
    }

    writer.flush();
    out.extend_from_slice(&writer.bytes);

    // 8. EOI
    out.extend_from_slice(&[0xFF, 0xD9]);

    Ok(out)
}
