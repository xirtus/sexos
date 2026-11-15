// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;

const LIMITS: sex_image::Limits = sex_image::Limits {
    max_width: 512,
    max_height: 512,
    max_pixels: 262_144,
    max_input_bytes: 4 * 1024 * 1024,
};

fn exercise_psd(data: &[u8]) {
    if let Ok(doc) = batik_core::parse_psd(data, &LIMITS) {
        let _ = doc.flatten();
    }
}

fn build_structured_psd(
    width: u32,
    height: u32,
    color_mode: u16,
    channels: u16,
    layer_w: i32,
    layer_h: i32,
    packbits: bool,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();

    // 1. Header (26 bytes)
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1u16.to_be_bytes()); // version 1
    out.extend_from_slice(&[0u8; 6]); // reserved
    out.extend_from_slice(&channels.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&8u16.to_be_bytes()); // 8-bit depth
    out.extend_from_slice(&color_mode.to_be_bytes());

    // 2. Color mode data (0 bytes)
    out.extend_from_slice(&0u32.to_be_bytes());

    // 3. Image resources (0 bytes)
    out.extend_from_slice(&0u32.to_be_bytes());

    // 4. Layer & Mask section
    let mut layer_info = Vec::new();
    layer_info.extend_from_slice(&1i16.to_be_bytes()); // layer count = 1

    // Layer record
    let top = 0i32;
    let left = 0i32;
    let bottom = layer_h;
    let right = layer_w;
    layer_info.extend_from_slice(&top.to_be_bytes());
    layer_info.extend_from_slice(&left.to_be_bytes());
    layer_info.extend_from_slice(&bottom.to_be_bytes());
    layer_info.extend_from_slice(&right.to_be_bytes());

    let channel_ids: [i16; 5] = [0, 1, 2, -1, -2];
    let ch_count = channels.min(5) as usize;
    layer_info.extend_from_slice(&(ch_count as u16).to_be_bytes());

    let pixel_count = (layer_w.max(0) as usize).saturating_mul(layer_h.max(0) as usize);
    let ch_data_len = if packbits {
        // 2 bytes compression + 2*height line table + payload per line
        2 + (layer_h.max(0) as usize * 2) + payload.len()
    } else {
        2 + pixel_count.min(payload.len())
    };

    for &ch_id in &channel_ids[..ch_count] {
        layer_info.extend_from_slice(&ch_id.to_be_bytes());
        layer_info.extend_from_slice(&(ch_data_len as u32).to_be_bytes());
    }

    layer_info.extend_from_slice(b"8BIM");
    layer_info.extend_from_slice(b"norm"); // Normal blend mode
    layer_info.push(255); // opacity
    layer_info.push(0); // clipping
    layer_info.push(0); // flags (visible)
    layer_info.push(0); // filler

    // Extra data: mask_len(4) + blend_ranges_len(4) + name
    let name = b"Layer1";
    let name_len = name.len() as u8;
    let total_name_bytes = 1 + name.len();
    let pad = (4 - (total_name_bytes % 4)) % 4;

    let extra_len = 4 + 4 + total_name_bytes + pad;
    layer_info.extend_from_slice(&(extra_len as u32).to_be_bytes());
    layer_info.extend_from_slice(&0u32.to_be_bytes()); // mask_len = 0
    layer_info.extend_from_slice(&0u32.to_be_bytes()); // blend_ranges_len = 0
    layer_info.push(name_len);
    layer_info.extend_from_slice(name);
    layer_info.extend_from_slice(&vec![0u8; pad]);

    // Channel image data for the layer
    for _ in 0..ch_count {
        if packbits {
            layer_info.extend_from_slice(&1u16.to_be_bytes()); // PackBits RLE compression
            let h = layer_h.max(0) as usize;
            let per_line = if h > 0 { payload.len() / h } else { 0 };
            for _ in 0..h {
                layer_info.extend_from_slice(&(per_line as u16).to_be_bytes());
            }
            layer_info.extend_from_slice(payload);
        } else {
            layer_info.extend_from_slice(&0u16.to_be_bytes()); // Raw compression
            let to_copy = pixel_count.min(payload.len());
            layer_info.extend_from_slice(&payload[..to_copy]);
            if to_copy < pixel_count {
                layer_info.resize(layer_info.len() + (pixel_count - to_copy), 0);
            }
        }
    }

    // Wrap layer_info with its length
    let mut layer_section = Vec::new();
    layer_section.extend_from_slice(&(layer_info.len() as u32).to_be_bytes());
    layer_section.extend_from_slice(&layer_info);

    out.extend_from_slice(&(layer_section.len() as u32).to_be_bytes());
    out.extend_from_slice(&layer_section);

    // 5. Image data section (fallback merged composite)
    out.extend_from_slice(&0u16.to_be_bytes()); // Raw compression
    let total_doc_pixels = (width as usize).saturating_mul(height as usize);
    let comp_bytes = total_doc_pixels.saturating_mul(channels as usize).min(1024);
    out.extend_from_slice(&vec![0x7Fu8; comp_bytes]);

    out
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Raw byte input
    exercise_psd(data);

    // 2. Structured PSD generation
    let mode = data[0] % 6;
    let payload = &data[1..];

    let (width, height, color_mode, channels) = match mode {
        0 => (4, 4, 3, 3),   // 4x4 RGB 3 channels
        1 => (16, 16, 3, 4), // 16x16 RGBA 4 channels
        2 => (8, 8, 1, 1),   // 8x8 Grayscale 1 channel
        3 => (1, 1, 3, 3),   // 1x1 RGB
        4 => (64, 64, 3, 3), // 64x64 RGB
        _ => (
            (data[0] as u32 % 32).max(1),
            ((data[0] >> 4) as u32 % 32).max(1),
            if (data[0] & 0x02) != 0 { 3 } else { 1 },
            ((data[0] & 0x03) as u16 % 4) + 1,
        ),
    };

    let (layer_w, layer_h) = if (data[0] & 0x04) != 0 {
        (0i32, 0i32) // Empty layer
    } else {
        (width as i32, height as i32)
    };

    let packbits = (data[0] & 0x08) != 0;

    let psd_bytes = build_structured_psd(
        width, height, color_mode, channels, layer_w, layer_h, packbits, payload,
    );
    exercise_psd(&psd_bytes);
});
