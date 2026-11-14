// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_font::{draw_cell, raster, Font};

fuzz_target!(|data: &[u8]| {
    if let Some(font) = Font::new(data) {
        let _ = font.ascender(16 * 64);
        let _ = font.descender(16 * 64);
        let _ = font.ink_extent();

        // Sample common characters and glyph advance/rasterization
        for scalar in [b'A' as u32, b'g' as u32, b'0' as u32, 0x20, 0x00] {
            if let Some(glyph) = font.glyph(scalar) {
                let _ = font.advance(glyph, 16 * 64);
                let (w, h) = (32, 32);
                let mut acc = vec![0i32; raster::scratch_len(w, h)];
                let mut out = vec![0u8; w * h];
                let _ = draw_cell(&font, scalar, w, h, &mut acc, &mut out);
            }
        }
    }
});
