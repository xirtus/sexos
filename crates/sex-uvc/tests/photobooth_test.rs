// SPDX-License-Identifier: GPL-3.0-only
//! Unit tests for Photo Booth live filters, 3x3 effects grid, 2x2 contact sheet,
//! and sex-image decode compatibility.

use sex_uvc::photobooth::{
    apply_filter, compose_burst_contact_sheet, render_effects_grid, Filter,
};

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let mut s1 = 1u32;
    let mut s2 = 0u32;
    for &b in data {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

fn encode_png(pixels: &[u32], width: usize, height: usize) -> Vec<u8> {
    let mut raw = Vec::with_capacity(height * (1 + width * 4));
    for y in 0..height {
        raw.push(0); // Filter byte 0 (None)
        let row = y * width;
        for x in 0..width {
            let p = pixels[row + x];
            let r = ((p >> 16) & 0xFF) as u8;
            let g = ((p >> 8) & 0xFF) as u8;
            let b = (p & 0xFF) as u8;
            let a = ((p >> 24) & 0xFF) as u8;
            raw.push(r);
            raw.push(g);
            raw.push(b);
            raw.push(if a == 0 { 255 } else { a });
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    // IHDR
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(b"IHDR");
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.push(8); // Bit depth
    ihdr.push(6); // Color type: RGBA
    ihdr.push(0); // Compression method
    ihdr.push(0); // Filter method
    ihdr.push(0); // Interlace method
    let ihdr_crc = crc32(&ihdr);
    out.extend_from_slice(&(13u32).to_be_bytes());
    out.extend_from_slice(&ihdr);
    out.extend_from_slice(&ihdr_crc.to_be_bytes());

    // IDAT with zlib deflate stored blocks
    let mut zlib = Vec::new();
    zlib.push(0x78);
    zlib.push(0x01); // No compression
    let mut offset = 0;
    while offset < raw.len() {
        let chunk_len = (raw.len() - offset).min(65535);
        let is_last = (offset + chunk_len) == raw.len();
        zlib.push(if is_last { 0x01 } else { 0x00 });
        zlib.extend_from_slice(&(chunk_len as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(chunk_len as u16)).to_le_bytes());
        zlib.extend_from_slice(&raw[offset..offset + chunk_len]);
        offset += chunk_len;
    }
    let adler = adler32(&raw);
    zlib.extend_from_slice(&adler.to_be_bytes());

    let mut idat = Vec::new();
    idat.extend_from_slice(b"IDAT");
    idat.extend_from_slice(&zlib);
    let idat_crc = crc32(&idat);
    out.extend_from_slice(&(zlib.len() as u32).to_be_bytes());
    out.extend_from_slice(&idat);
    out.extend_from_slice(&idat_crc.to_be_bytes());

    // IEND
    let mut iend = Vec::new();
    iend.extend_from_slice(b"IEND");
    let iend_crc = crc32(&iend);
    out.extend_from_slice(&(0u32).to_be_bytes());
    out.extend_from_slice(&iend);
    out.extend_from_slice(&iend_crc.to_be_bytes());

    out
}

fn make_test_frame(w: usize, h: usize) -> Vec<u32> {
    let mut frame = vec![0u32; w * h];
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let r = ((x * 255) / w.max(1)) as u32;
            let g = ((y * 255) / h.max(1)) as u32;
            let b = (((x + y) * 128) / (w + h).max(1)) as u32;
            frame[row + x] = (r << 16) | (g << 8) | b;
        }
    }
    frame
}

#[test]
fn test_all_filters_produce_valid_deterministic_pixels() {
    let w = 160;
    let h = 120;
    let src = make_test_frame(w, h);
    let mut dst = vec![0u32; w * h];

    for filter in Filter::ALL {
        dst.fill(0);
        apply_filter(filter, &src, &mut dst, w, h);

        assert_eq!(dst.len(), w * h);
        let non_zero = dst.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > (w * h) / 2, "Filter {:?} produced too many zeroes: {}", filter, non_zero);

        // Determinism test: running again produces identical output
        let mut dst2 = vec![0u32; w * h];
        apply_filter(filter, &src, &mut dst2, w, h);
        assert_eq!(dst, dst2, "Filter {:?} not deterministic", filter);
    }
}

#[test]
fn test_filter_names_and_parsing() {
    for filter in Filter::ALL {
        let name = filter.name();
        let parsed = Filter::from_name(name);
        assert_eq!(parsed, Some(filter), "Failed to parse back filter {}", name);

        // Case insensitivity
        let upper = name.to_uppercase();
        assert_eq!(Filter::from_name(&upper), Some(filter));
        let lower = name.to_lowercase();
        assert_eq!(Filter::from_name(&lower), Some(filter));
    }

    // Aliases
    assert_eq!(Filter::from_name("x-ray"), Some(Filter::XRay));
    assert_eq!(Filter::from_name("x_ray"), Some(Filter::XRay));
    assert_eq!(Filter::from_name("pop_art"), Some(Filter::PopArt));
    assert_eq!(Filter::from_name("fisheye"), Some(Filter::Bulge));
    assert_eq!(Filter::from_name("unknown"), None);
}

#[test]
fn test_filter_cycle_next() {
    let mut cur = Filter::Normal;
    for _ in 0..Filter::ALL.len() {
        cur = cur.next();
    }
    assert_eq!(cur, Filter::Normal);
}

#[test]
fn test_effects_grid_rendering() {
    let src_w = 320;
    let src_h = 240;
    let src = make_test_frame(src_w, src_h);

    let dst_w = 640;
    let dst_h = 480;
    let mut dst = vec![0u32; dst_w * dst_h];

    render_effects_grid(&src, src_w, src_h, &mut dst, dst_w, dst_h);

    // Verify non-empty
    let non_zero = dst.iter().filter(|&&p| p != 0).count();
    assert!(non_zero > (dst_w * dst_h * 8) / 10);

    // Verify grid borders are present (dark pixels around cell_w)
    let border_px = dst[dst_h / 2 * dst_w + (dst_w / 3)];
    assert_eq!(border_px, 0x0010141C);
}

#[test]
fn test_burst_contact_sheet_composition() {
    let w = 160;
    let h = 120;
    let f0 = make_test_frame(w, h);
    let mut f1 = vec![0u32; w * h];
    let mut f2 = vec![0u32; w * h];
    let mut f3 = vec![0u32; w * h];
    apply_filter(Filter::Sepia, &f0, &mut f1, w, h);
    apply_filter(Filter::Thermal, &f0, &mut f2, w, h);
    apply_filter(Filter::PopArt, &f0, &mut f3, w, h);

    let (cw, ch, sheet) = compose_burst_contact_sheet([&f0, &f1, &f2, &f3], w, h);
    assert_eq!(sheet.len(), cw * ch);
    assert!(cw > w);
    assert!(ch > h);

    // Check top-left corner is white photo-strip paper margin
    assert_eq!(sheet[0], 0x00FBFBFC);
}

#[test]
fn test_png_decode_compatibility_of_all_modes() {
    let w = 160;
    let h = 120;
    let frame = make_test_frame(w, h);
    let limits = sex_image::Limits {
        max_width: 2048,
        max_height: 2048,
        max_pixels: 2048 * 2048,
        max_input_bytes: 8 << 20,
    };

    // 1. Single photo (Normal)
    let png_single = encode_png(&frame, w, h);
    let decoded_single = sex_image::decode(&png_single, &limits).expect("decode single");
    assert_eq!(decoded_single.width, w);
    assert_eq!(decoded_single.height, h);
    assert_eq!(decoded_single.pixels.len(), w * h);

    // 2. Timer photo (Filtered e.g. Sepia)
    let mut timer_frame = vec![0u32; w * h];
    apply_filter(Filter::Sepia, &frame, &mut timer_frame, w, h);
    let png_timer = encode_png(&timer_frame, w, h);
    let decoded_timer = sex_image::decode(&png_timer, &limits).expect("decode timer");
    assert_eq!(decoded_timer.width, w);
    assert_eq!(decoded_timer.height, h);

    // 3. 4-Shot Burst individual frames
    for i in 0..4 {
        let png_burst = encode_png(&frame, w, h);
        let decoded = sex_image::decode(&png_burst, &limits).expect("decode burst frame");
        assert_eq!(decoded.width, w);
        assert_eq!(decoded.height, h);
        assert_eq!(i, i);
    }

    // 4. 2x2 Contact Sheet
    let (cw, ch, sheet) = compose_burst_contact_sheet([&frame, &timer_frame, &frame, &timer_frame], w, h);
    let png_contact = encode_png(&sheet, cw, ch);
    let decoded_contact = sex_image::decode(&png_contact, &limits).expect("decode contact sheet");
    assert_eq!(decoded_contact.width, cw);
    assert_eq!(decoded_contact.height, ch);
    assert_eq!(decoded_contact.pixels.len(), cw * ch);
}
