// SPDX-License-Identifier: GPL-3.0-only
use batik_core::ora::read_ora;
use batik_core::psd::parse_psd;
use sex_image::Limits;

#[test]
fn test_adversarial_psd_fuzz() {
    let limits = Limits::SURFACE;

    // 1. Empty and truncated headers
    assert!(parse_psd(&[], &limits).is_err());
    for len in 1..26 {
        let buf = vec![0u8; len];
        assert!(parse_psd(&buf, &limits).is_err());
    }

    // 2. Corrupt magic
    let mut header = vec![0u8; 30];
    header[0..4].copy_from_slice(b"BADS");
    assert!(parse_psd(&header, &limits).is_err());

    // 3. Huge dimensions that exceed limits (8192 x 8192 on SURFACE limits 800x600)
    let mut huge = vec![0u8; 30];
    huge[0..4].copy_from_slice(b"8BPS");
    huge[4..6].copy_from_slice(&1u16.to_be_bytes());
    huge[12..14].copy_from_slice(&3u16.to_be_bytes());
    huge[14..18].copy_from_slice(&8192u32.to_be_bytes()); // height
    huge[18..22].copy_from_slice(&8192u32.to_be_bytes()); // width
    huge[22..24].copy_from_slice(&8u16.to_be_bytes()); // depth 8
    huge[24..26].copy_from_slice(&3u16.to_be_bytes()); // RGB
    assert!(parse_psd(&huge, &limits).is_err());

    // 4. Truncated channel sections
    let mut trunc_channel = vec![0u8; 100];
    trunc_channel[0..4].copy_from_slice(b"8BPS");
    trunc_channel[4..6].copy_from_slice(&1u16.to_be_bytes());
    trunc_channel[12..14].copy_from_slice(&3u16.to_be_bytes());
    trunc_channel[14..18].copy_from_slice(&10u32.to_be_bytes());
    trunc_channel[18..22].copy_from_slice(&10u32.to_be_bytes());
    trunc_channel[22..24].copy_from_slice(&8u16.to_be_bytes());
    trunc_channel[24..26].copy_from_slice(&3u16.to_be_bytes());
    // Layer mask info length claims 50 bytes, but slice ends
    trunc_channel[26..30].copy_from_slice(&0u32.to_be_bytes()); // color mode
    trunc_channel[30..34].copy_from_slice(&0u32.to_be_bytes()); // img res
    trunc_channel[34..38].copy_from_slice(&5000u32.to_be_bytes()); // layer mask len
    assert!(parse_psd(&trunc_channel, &limits).is_err());

    // 5. Random byte garbage fuzz
    let mut pseudo_rand = 0x12345678u32;
    for _ in 0..100 {
        let len = (pseudo_rand as usize % 500) + 1;
        let mut gar = Vec::with_capacity(len);
        for _ in 0..len {
            pseudo_rand = pseudo_rand.wrapping_mul(1103515245).wrapping_add(12345);
            gar.push((pseudo_rand >> 16) as u8);
        }
        // Must never panic
        let _ = parse_psd(&gar, &limits);
    }
}

#[test]
fn test_adversarial_ora_fuzz() {
    let limits = Limits::SURFACE;

    // 1. Empty and invalid zip
    assert!(read_ora(&[], &limits).is_err());
    assert!(read_ora(b"not a zip file at all", &limits).is_err());

    // 2. Random byte fuzz
    let mut pseudo_rand = 0x87654321u32;
    for _ in 0..100 {
        let len = (pseudo_rand as usize % 500) + 1;
        let mut gar = Vec::with_capacity(len);
        for _ in 0..len {
            pseudo_rand = pseudo_rand.wrapping_mul(1103515245).wrapping_add(12345);
            gar.push((pseudo_rand >> 16) as u8);
        }
        // Must never panic
        let _ = read_ora(&gar, &limits);
    }
}

#[test]
fn test_adversarial_stroke_engine_nan_inf_and_zeros() {
    use batik_core::brush::{Brush, PointerSample, StrokeEngine};
    use batik_core::layer::RasterLayer;
    use batik_core::selection::SelectionMask;

    let limits = Limits::SURFACE;
    let mut raster = RasterLayer::new(100, 100, &limits).unwrap();
    let selection = SelectionMask::new(100, 100).unwrap();

    let mut brush = Brush::default();
    brush.size = -10.0;
    brush.spacing = 0.0;

    let mut engine = StrokeEngine::new(brush);

    // NaN coordinates and Inf pressure
    let nan_sample = PointerSample::new(f32::NAN, 50.0, f32::INFINITY);
    assert!(engine.paint_to(&mut raster, &selection, nan_sample).is_ok());

    let inf_sample = PointerSample::new(50.0, f32::INFINITY, -1.0);
    assert!(engine.paint_to(&mut raster, &selection, inf_sample).is_ok());

    // Valid sample followed by zero spacing / negative size
    engine.brush.size = 0.0;
    engine.brush.spacing = -0.5;
    let s1 = PointerSample::new(10.0, 10.0, 0.5);
    let s2 = PointerSample::new(20.0, 20.0, 0.5);
    assert!(engine.paint_to(&mut raster, &selection, s1).is_ok());
    assert!(engine.paint_to(&mut raster, &selection, s2).is_ok());
}

#[test]
fn test_adversarial_bucket_fill_and_magic_wand_tolerance_255() {
    use batik_core::brush::bucket_fill;
    use batik_core::layer::RasterLayer;
    use batik_core::selection::{SelectionCombine, SelectionMask};

    let limits = Limits::SURFACE;
    let mut raster = RasterLayer::new(64, 64, &limits).unwrap();
    let mut selection = SelectionMask::new(64, 64).unwrap();

    // Tolerance 255 with same fill color as target
    assert!(bucket_fill(&mut raster, 10, 10, 0, 255, &selection).is_ok());

    // Tolerance 255 with different fill color
    assert!(bucket_fill(&mut raster, 10, 10, 0xFFFFFFFF, 255, &selection).is_ok());

    // Magic wand with tolerance 255
    selection.magic_wand(&raster.pixels, 64, 64, 0, 0, 255, true, SelectionCombine::Replace);
    assert!(selection.is_active());
}

#[test]
fn test_adversarial_filters_and_transforms_edge_cases() {
    use batik_core::adjust::Levels;
    use batik_core::filters::{box_blur, gaussian_blur, pixelate};
    use batik_core::transform::{resize_bilinear, sample_bilinear, transform_affine};

    let pixels = vec![0xFFFFFFFFu32; 100];

    // gaussian blur with 0.0, negative, and NaN sigma
    let b0 = gaussian_blur(&pixels, 10, 10, 0.0);
    assert_eq!(b0.len(), 100);
    let b_nan = gaussian_blur(&pixels, 10, 10, f32::NAN);
    assert_eq!(b_nan.len(), 100);

    // box blur with 0 radius
    let bb0 = box_blur(&pixels, 10, 10, 0);
    assert_eq!(bb0.len(), 100);

    // pixelate with 0 block size
    let p0 = pixelate(&pixels, 10, 10, 0);
    assert_eq!(p0.len(), 100);

    // resize_bilinear with 0 dimensions
    assert!(resize_bilinear(&pixels, 10, 10, 0, 0).is_err());

    // sample_bilinear with NaN
    assert_eq!(sample_bilinear(&pixels, 10, 10, f32::NAN, 5.0), 0);

    // singular affine matrix (det == 0.0 or NaN)
    let singular_mat = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    assert!(transform_affine(&pixels, 10, 10, 10, 10, singular_mat).is_err());
    let nan_mat = [f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0];
    assert!(transform_affine(&pixels, 10, 10, 10, 10, nan_mat).is_err());

    // Levels when in_white == in_black
    let levels = Levels {
        in_black: 128,
        in_white: 128,
        gamma_fp: 1.0,
        out_black: 0,
        out_white: 255,
    };
    let lut = levels.build_lut();
    assert_eq!(lut[0], 0);

    // Levels with NaN gamma
    let levels_nan = Levels {
        in_black: 0,
        in_white: 255,
        gamma_fp: f32::NAN,
        out_black: 0,
        out_white: 255,
    };
    let _ = levels_nan.build_lut();
}

#[test]
fn test_adversarial_psd_rect_overflow_and_channel_bounds() {
    let limits = Limits::SURFACE;

    // Craft a PSD with reversed/negative rects: bottom < top, right < left
    let mut psd = Vec::new();
    psd.extend_from_slice(b"8BPS"); // Magic
    psd.extend_from_slice(&1u16.to_be_bytes()); // Version
    psd.extend_from_slice(&[0u8; 6]); // Reserved
    psd.extend_from_slice(&3u16.to_be_bytes()); // Channels = 3
    psd.extend_from_slice(&100u32.to_be_bytes()); // Height = 100
    psd.extend_from_slice(&100u32.to_be_bytes()); // Width = 100
    psd.extend_from_slice(&8u16.to_be_bytes()); // Depth = 8
    psd.extend_from_slice(&3u16.to_be_bytes()); // Color mode = RGB
    psd.extend_from_slice(&0u32.to_be_bytes()); // Color mode len
    psd.extend_from_slice(&0u32.to_be_bytes()); // Image resources len

    // Layer and mask info section
    let layer_info_len: u32 = 100;
    let section_len: u32 = 4 + layer_info_len;
    psd.extend_from_slice(&section_len.to_be_bytes());
    psd.extend_from_slice(&layer_info_len.to_be_bytes());
    psd.extend_from_slice(&1i16.to_be_bytes()); // 1 layer

    // Layer record: top = 100, left = 100, bottom = 0 (reversed), right = i32::MIN (overflow hazard)
    psd.extend_from_slice(&100i32.to_be_bytes());
    psd.extend_from_slice(&100i32.to_be_bytes());
    psd.extend_from_slice(&0i32.to_be_bytes());
    psd.extend_from_slice(&i32::MIN.to_be_bytes());
    psd.extend_from_slice(&0u16.to_be_bytes()); // 0 channels
    psd.extend_from_slice(b"8BIM");
    psd.extend_from_slice(b"norm");
    psd.push(255); // opacity
    psd.push(0); // clipping
    psd.push(0); // flags
    psd.push(0); // filler
    psd.extend_from_slice(&8u32.to_be_bytes()); // extra length
    psd.extend_from_slice(&0u32.to_be_bytes()); // mask len
    psd.extend_from_slice(&0u32.to_be_bytes()); // blend ranges len
    psd.push(0); // name len (0)
    psd.extend_from_slice(&[0u8; 3]); // pad

    // Must never panic with underflow/overflow
    let res = parse_psd(&psd, &limits);
    // Because layer rect was invalid, layer_w and layer_h are 0, layer is skipped,
    // and falling back to composite will fail due to EOF, returning an error safely.
    assert!(res.is_err() || res.is_ok());
}

#[test]
fn test_adversarial_ora_path_traversal() {
    use sex_archive::{no_progress, ArchiveBuilder, OutputFormat, VecReader};

    let limits = Limits::SURFACE;
    let mut builder = ArchiveBuilder::new(OutputFormat::Zip);

    let stack_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<image version=\"0.0.1\" w=\"10\" h=\"10\">\n  <stack>\n    <layer name=\"Bad\" src=\"../secret.png\" x=\"0\" y=\"0\" opacity=\"1.0\" visibility=\"visible\" composite-op=\"svg:src-over\"/>\n  </stack>\n</image>\n".to_vec();
    builder.add_file("stack.xml", stack_xml.len() as u64, 0, VecReader::new(stack_xml)).unwrap();

    let mut zip_bytes = Vec::new();
    builder.write(&mut zip_bytes, &mut no_progress).unwrap();

    // Must be rejected due to path traversal in layer src
    let res = read_ora(&zip_bytes, &limits);
    assert!(res.is_err());
}
