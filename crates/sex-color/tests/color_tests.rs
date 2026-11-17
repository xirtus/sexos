use libm::fabsf;
use sex_color::{
    cineon_log_to_linear, compute_histogram, convert_primaries, convert_rgb, detect_nv12_range,
    hlg_eotf, hlg_inverse_oetf, hlg_oetf, hlg_to_linear_nits, linear_nits_to_hlg,
    linear_rec2020_to_oklab, linear_rec2020_to_rec709, linear_rec2020_to_xyz,
    linear_rec709_to_oklab, linear_rec709_to_rec2020, linear_rec709_to_xyz, linear_to_cineon_log,
    linear_to_rec2020, linear_to_rec601, linear_to_rec709, linear_to_srgb, mul_mat3,
    oklab_to_linear_rec2020, oklab_to_linear_rec709, pq_eotf, pq_eotf_nits, pq_oetf, pq_oetf_nits,
    rec2020_to_linear, rec601_to_linear, rec709_to_linear, sdr_on_hdr_scale, srgb_to_linear,
    tone_map, xyz_to_linear_rec2020, xyz_to_linear_rec709, ColorGrade, ColorPrimaries, ColorSpace,
    CubeLut, Curve1D, FilmStock, FrameError, InterpMode, LinearFrame, Lut1D, Lut3D, LutError,
    RgbaF32, ScopeInput, ScopeStandard, ScopeSurface, ToneMapConfig, ToneMapMode, WaveformMode,
    YuvMatrix, YuvRange, REC2020_TO_REC709, REC709_TO_REC2020,
};
use sex_image::Image;

fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
    fabsf(a - b) <= eps
}

fn approx_rgb(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
    approx_eq(a[0], b[0], eps) && approx_eq(a[1], b[1], eps) && approx_eq(a[2], b[2], eps)
}

// ===========================================================================
// 1. space.rs tests
// ===========================================================================

#[test]
fn srgb_transfer_roundtrip_and_extended_continuity() {
    assert_eq!(srgb_to_linear(0.0), 0.0);
    assert!(approx_eq(srgb_to_linear(1.0), 1.0, 1e-6));
    assert!(approx_eq(linear_to_srgb(1.0), 1.0, 1e-6));

    // Check continuity around IEC 61966-2-1 cutoff 0.04045.
    let lo = srgb_to_linear(0.040449);
    let hi = srgb_to_linear(0.040451);
    assert!(approx_eq(lo, hi, 1e-5));

    // Roundtrip across [0.0, 1.0] plus negative & HDR extended values.
    for i in -20..=150 {
        let v = i as f32 / 100.0;
        let lin = srgb_to_linear(v);
        let back = linear_to_srgb(lin);
        assert!(approx_eq(v, back, 1e-5), "sRGB roundtrip failed at {v}: {back}");
    }
}

#[test]
fn rec709_rec601_rec2020_transfer_roundtrips() {
    for i in -10..=120 {
        let v = i as f32 / 100.0;
        assert!(approx_eq(v, linear_to_rec709(rec709_to_linear(v)), 1e-5));
        assert!(approx_eq(v, linear_to_rec601(rec601_to_linear(v)), 1e-5));
        assert!(approx_eq(v, linear_to_rec2020(rec2020_to_linear(v)), 1e-5));
    }
    // Check BT.2020 12-bit segment continuity at 4.5 * 0.01805397 = 0.08124286.
    let l = rec2020_to_linear(0.081_24);
    let r = rec2020_to_linear(0.081_25);
    assert!(approx_eq(l, r, 1e-5));
}

#[test]
fn pq_st2084_eotf_oetf_benchmarks() {
    assert_eq!(pq_eotf(0.0), 0.0);
    assert!(approx_eq(pq_eotf(1.0), 1.0, 1e-6));
    assert!(approx_eq(pq_eotf_nits(1.0), 10_000.0, 1e-2));
    // SMPTE ST.2084 benchmark values: ~0.5081 = 100 nits, ~0.5807 = 203 nits, ~0.7518 = 1000 nits.
    assert!(approx_eq(pq_eotf_nits(0.508_078_4), 100.0, 0.05));
    assert!(approx_eq(pq_oetf_nits(203.0), 0.580_689, 1e-4));
    assert!(approx_eq(pq_eotf_nits(0.751_827_1), 1_000.0, 0.2));

    for i in 0..=100 {
        let ep = i as f32 / 100.0;
        let round = pq_oetf(pq_eotf(ep));
        assert!(approx_eq(ep, round, 1e-5), "PQ roundtrip failed at {ep}: {round}");
    }
}

#[test]
fn hlg_arib_std_b67_and_ootf_roundtrip() {
    assert_eq!(hlg_oetf(0.0), 0.0);
    assert!(approx_eq(hlg_oetf(1.0 / 12.0), 0.5, 1e-6));
    assert!(approx_eq(hlg_oetf(1.0), 1.0, 1e-6));
    assert!(approx_eq(hlg_eotf(0.5), 1.0 / 12.0, 1e-6));

    for i in 0..=100 {
        let ep = i as f32 / 100.0;
        let back = hlg_oetf(hlg_inverse_oetf(ep));
        assert!(approx_eq(ep, back, 1e-5));
    }

    // BT.2100 1000-nit OOTF: 75% HLG ~ 203 nits, 100% HLG = 1000 nits.
    let nits_75 = hlg_to_linear_nits([0.75, 0.75, 0.75], 1_000.0);
    assert!(approx_eq(nits_75[0], 203.0, 1.0), "75% HLG nits: {}", nits_75[0]);
    let nits_100 = hlg_to_linear_nits([1.0, 1.0, 1.0], 1_000.0);
    assert!(approx_eq(nits_100[0], 1_000.0, 0.1));

    let sample = [0.62, 0.78, 0.41];
    let back = linear_nits_to_hlg(hlg_to_linear_nits(sample, 1_000.0), 1_000.0);
    assert!(approx_rgb(sample, back, 1e-4));
}

#[test]
fn gamut_matrices_preserve_d65_white_and_invert() {
    let white = [1.0, 1.0, 1.0];
    assert!(approx_rgb(mul_mat3(&REC709_TO_REC2020, white), white, 1e-5));
    assert!(approx_rgb(mul_mat3(&REC2020_TO_REC709, white), white, 1e-5));
    assert!(approx_rgb(
        convert_primaries(white, ColorPrimaries::Rec601, ColorPrimaries::Rec2020),
        white,
        1e-5
    ));

    let c = [0.35, 0.62, 0.19];
    assert!(approx_rgb(xyz_to_linear_rec709(linear_rec709_to_xyz(c)), c, 1e-5));
    assert!(approx_rgb(xyz_to_linear_rec2020(linear_rec2020_to_xyz(c)), c, 1e-5));
    assert!(approx_rgb(
        linear_rec2020_to_rec709(linear_rec709_to_rec2020(c)),
        c,
        1e-5
    ));
}

#[test]
fn linear_rec2020_preserves_wide_gamut_without_bt709_clamping() {
    // Pure Rec.2020 green lies outside BT.709 gamut (its Rec.709 R component is negative).
    let rec2020_pure_green = [0.0, 1.0, 0.0];
    let in_709 = linear_rec2020_to_rec709(rec2020_pure_green);
    assert!(in_709[0] < -0.5, "Expected negative Rec.709 R for Rec.2020 green, got {:?}", in_709);

    // Converting Rec2020 -> LinearRec2020 -> Hdr10Pq -> LinearRec2020 -> Rec2020 must NOT clamp!
    let lin = convert_rgb(rec2020_pure_green, ColorSpace::Rec2020, ColorSpace::LinearRec2020);
    let pq = convert_rgb(lin, ColorSpace::LinearRec2020, ColorSpace::Hdr10Pq);
    let lin_back = convert_rgb(pq, ColorSpace::Hdr10Pq, ColorSpace::LinearRec2020);
    let rec2020_back = convert_rgb(lin_back, ColorSpace::LinearRec2020, ColorSpace::Rec2020);
    assert!(approx_rgb(rec2020_pure_green, rec2020_back, 1e-4));

    // Even converting to Oklab and back preserves out-of-BT.709 Rec.2020 primaries!
    let lab = convert_rgb(lin, ColorSpace::LinearRec2020, ColorSpace::Oklab);
    let back_from_oklab = convert_rgb(lab, ColorSpace::Oklab, ColorSpace::LinearRec2020);
    assert!(approx_rgb(lin, back_from_oklab, 1e-4));
}

#[test]
fn oklab_conversions_and_perceptual_ordering() {
    let black = linear_rec709_to_oklab([0.0, 0.0, 0.0]);
    let white = linear_rec709_to_oklab([1.0, 1.0, 1.0]);
    assert!(approx_rgb(black, [0.0, 0.0, 0.0], 1e-5));
    assert!(approx_rgb(white, [1.0, 0.0, 0.0], 1e-4));

    // Yellow has higher perceptual lightness L than blue.
    let blue_lab = linear_rec709_to_oklab([0.0, 0.0, 1.0]);
    let yellow_lab = linear_rec709_to_oklab([1.0, 1.0, 0.0]);
    assert!(yellow_lab[0] > blue_lab[0]);

    let sample = [0.75, 0.28, 0.12];
    assert!(approx_rgb(
        oklab_to_linear_rec709(linear_rec709_to_oklab(sample)),
        sample,
        1e-4
    ));
    assert!(approx_rgb(
        oklab_to_linear_rec2020(linear_rec2020_to_oklab(sample)),
        sample,
        1e-4
    ));
}

#[test]
fn hdr_tone_mapping_modes_monotonic_and_peak_bounded() {
    for mode in [
        ToneMapMode::Bt2390,
        ToneMapMode::Reinhard,
        ToneMapMode::HableFilmic,
        ToneMapMode::Clip,
    ] {
        let mut prev = -1.0f32;
        for step in 0..=20 {
            let v = step as f32 * 0.5; // 0 to 10.0 (0 to 1000 nits)
            let mapped = tone_map([v, 0.5 * v, 0.25 * v], mode, 1_000.0, 100.0);
            assert!(mapped[0] >= prev - 1e-6, "{mode:?} not monotonic at {v}");
            assert!(mapped[0] <= 1.0001, "{mode:?} exceeded 100-nit target at {v}");
            // Hue / channel ratio 1 : 0.5 : 0.25 preserved.
            if mapped[0] > 1e-4 {
                assert!(approx_eq(mapped[1] / mapped[0], 0.5, 1e-4));
                assert!(approx_eq(mapped[2] / mapped[0], 0.25, 1e-4));
            }
            prev = mapped[0];
        }
    }

    // BT.2390 leaves dark shadows below knee untouched when mapping 1000 -> 100 nits.
    let shadow = [0.1, 0.1, 0.1]; // 10 nits
    let bt_shadow = tone_map(shadow, ToneMapMode::Bt2390, 1_000.0, 100.0);
    assert!(approx_rgb(shadow, bt_shadow, 1e-4));
}

#[test]
fn sdr_on_hdr_scale_matches_bt2408() {
    assert!(approx_eq(sdr_on_hdr_scale(100.0), 1.0, 1e-6));
    assert!(approx_eq(sdr_on_hdr_scale(203.0), 2.03, 1e-6));
    assert_eq!(sdr_on_hdr_scale(-50.0), 0.0);
    assert_eq!(sdr_on_hdr_scale(f32::NAN), 0.0);

    // Compositing SDR white (1.0) at 203 nits onto LinearRec2020 -> Hdr10Pq yields ~0.5807 PQ.
    let sdr_white_scaled = [1.0 * sdr_on_hdr_scale(203.0); 3];
    let pq = convert_rgb(sdr_white_scaled, ColorSpace::LinearRec2020, ColorSpace::Hdr10Pq);
    assert!(approx_eq(pq[0], 0.580_689, 1e-4));
}

// ===========================================================================
// 2. frame.rs tests
// ===========================================================================

#[test]
fn rgba_f32_alpha_luma_and_nan_inf_safety() {
    // Premultiply / unpremultiply roundtrip and 0 * Inf NaN guard.
    let px = RgbaF32([0.8, 0.4, 0.2, 0.5]);
    let premul = px.premultiply();
    assert!(approx_rgb(premul.rgb(), [0.4, 0.2, 0.1], 1e-6));
    assert!(approx_rgb(premul.unpremultiply().rgb(), [0.8, 0.4, 0.2], 1e-6));

    let zero_alpha_inf = RgbaF32([f32::INFINITY, f32::NAN, -5.0, 0.0]).premultiply();
    assert_eq!(zero_alpha_inf, RgbaF32::TRANSPARENT);

    // Sanitize preserves finite negative wide-gamut & HDR >1 values, fixes NaN/Inf.
    let dirty = RgbaF32([-0.25, f32::NAN, f32::INFINITY, f32::NEG_INFINITY]);
    let clean = dirty.sanitize();
    assert_eq!(clean.r(), -0.25);
    assert_eq!(clean.g(), 0.0);
    assert!(clean.b().is_finite() && clean.b() > 1000.0);
    assert_eq!(clean.a(), 0.0);

    let clamped = dirty.clamp01();
    assert_eq!(clamped, RgbaF32([0.0, 0.0, 1.0, 0.0]));

    // Luma checks.
    assert!(approx_eq(RgbaF32::WHITE.rec709_luma(), 1.0, 1e-6));
    assert!(approx_eq(RgbaF32::WHITE.rec2020_luma(), 1.0, 1e-6));
    assert!(approx_eq(RgbaF32::WHITE.rec601_luma(), 1.0, 1e-6));
    assert!(approx_eq(RgbaF32::new(0.0, 1.0, 0.0, 1.0).rec709_luma(), 0.7152, 1e-6));
    assert!(approx_eq(RgbaF32::new(0.0, 1.0, 0.0, 1.0).rec2020_luma(), 0.6780, 1e-6));

    // Porter-Duff over.
    let fg = RgbaF32([1.0, 0.0, 0.0, 0.5]);
    let bg = RgbaF32([0.0, 0.0, 1.0, 1.0]);
    let comp = fg.over_straight(bg);
    assert!(approx_rgb(comp.rgb(), [0.5, 0.0, 0.5], 1e-5));
    assert!(approx_eq(comp.a(), 1.0, 1e-6));
}

#[test]
fn bgra8_and_argb_u32_exact_256_roundtrip() {
    let mut bgra = Vec::with_capacity(256 * 4);
    for i in 0..=255u8 {
        bgra.push(i);
        bgra.push(255 - i);
        bgra.push((i / 2).saturating_add(40));
        bgra.push(i);
    }
    let frame = LinearFrame::from_bgra8_srgb(16, 16, &bgra).unwrap();
    assert_eq!(frame.primaries, ColorPrimaries::Rec709);
    let back = frame.to_bgra8_srgb();
    assert_eq!(bgra, back);

    let argb_in: Vec<u32> = (0..16u32)
        .map(|i| (0xFFu32 << 24) | ((i * 17) << 16) | ((255 - i * 17) << 8) | (i * 11))
        .collect();
    let f2 = LinearFrame::from_argb_u32_srgb(4, 4, &argb_in).unwrap();
    assert_eq!(f2.to_argb_u32_srgb(), argb_in);
}

#[test]
fn nv12_roundtrip_all_matrices_and_ranges() {
    // 4x4 frame with 2x2 distinct color blocks so 4:2:0 subsampling is exact.
    let mut pixels = vec![RgbaF32::BLACK; 16];
    let colors = [
        RgbaF32([0.8, 0.1, 0.1, 1.0]),
        RgbaF32([0.1, 0.7, 0.2, 1.0]),
        RgbaF32([0.1, 0.2, 0.9, 1.0]),
        RgbaF32([0.6, 0.5, 0.4, 1.0]),
    ];
    for y in 0..4 {
        for x in 0..4 {
            let block = (y / 2) * 2 + (x / 2);
            pixels[y * 4 + x] = colors[block];
        }
    }

    for (matrix, transfer) in [
        (YuvMatrix::Bt601, ColorSpace::Rec601),
        (YuvMatrix::Bt709, ColorSpace::Rec709),
        (YuvMatrix::Bt2020, ColorSpace::Rec2020),
    ] {
        for range in [YuvRange::Limited, YuvRange::Full] {
            let frame = LinearFrame::from_pixels(4, 4, matrix.default_primaries(), pixels.clone()).unwrap();
            let nv12 = frame.to_nv12(matrix, range, transfer).unwrap();
            let decoded = LinearFrame::from_nv12(4, 4, &nv12, matrix, range, transfer).unwrap();
            for (i, (&orig, &dec)) in frame.pixels.iter().zip(decoded.pixels.iter()).enumerate() {
                assert!(
                    approx_rgb(orig.rgb(), dec.rgb(), 0.025),
                    "NV12 {matrix:?} {range:?} mismatch at px {i}: orig={:?} dec={:?}",
                    orig.rgb(),
                    dec.rgb()
                );
            }
        }
    }
}

#[test]
fn nv12_auto_full_range_clipping_detector() {
    // Construct a 4x4 limited-range NV12 buffer (all Y in 16..=235, UV in 16..=240).
    let mut limited = vec![128u8; 4 * 4 + 8];
    for i in 0..16 {
        limited[i] = 16 + (i as u8) * 14; // 16..=226
    }
    let lim_report = detect_nv12_range(4, 4, &limited).unwrap();
    assert_eq!(lim_report.detected_range, YuvRange::Limited);
    assert_eq!(lim_report.y_below_min_count, 0);
    assert_eq!(lim_report.y_above_max_count, 0);
    assert_eq!(lim_report.would_clip_fraction, 0.0);

    // Construct a full-range NV12 buffer containing Y=0 (black) and Y=255 (white), plus Y=10/248.
    let mut full = vec![128u8; 4 * 4 + 8];
    full[0] = 0;
    full[1] = 10;
    full[2] = 248;
    full[3] = 255;
    for i in 4..16 {
        full[i] = 128;
    }
    let full_report = detect_nv12_range(4, 4, &full).unwrap();
    assert_eq!(full_report.detected_range, YuvRange::Full);
    assert_eq!(full_report.y_below_min_count, 2);
    assert_eq!(full_report.y_above_max_count, 2);
    assert!(full_report.would_clip_fraction > 0.2);

    // Auto decode distinguishes Y=0 vs Y=10 (which Limited would have clipped identically to 0.0!).
    let decoded_auto =
        LinearFrame::from_nv12(4, 4, &full, YuvMatrix::Bt709, YuvRange::Auto, ColorSpace::Rec709)
            .unwrap();
    assert_eq!(decoded_auto.pixels[0].r(), 0.0);
    assert!(decoded_auto.pixels[1].r() > 0.005);
    assert!(decoded_auto.pixels[2].r() < 0.99);
    assert!(approx_eq(decoded_auto.pixels[3].r(), 1.0, 1e-5));
}

#[test]
fn p010_10bit_hdr10_pq_and_hlg_roundtrip() {
    // Create 4x2 HDR LinearRec2020 frame with 0 nits, 100 nits (1.0), 203 nits (2.03), 1000 nits (10.0).
    let pixels = vec![
        RgbaF32([0.0, 0.0, 0.0, 1.0]),
        RgbaF32([0.0, 0.0, 0.0, 1.0]),
        RgbaF32([1.0, 1.0, 1.0, 1.0]),
        RgbaF32([1.0, 1.0, 1.0, 1.0]),
        RgbaF32([2.03, 2.03, 2.03, 1.0]),
        RgbaF32([2.03, 2.03, 2.03, 1.0]),
        RgbaF32([10.0, 10.0, 10.0, 1.0]),
        RgbaF32([10.0, 10.0, 10.0, 1.0]),
    ];
    let frame = LinearFrame::from_pixels(4, 2, ColorPrimaries::Rec2020, pixels).unwrap();

    for transfer in [ColorSpace::Hdr10Pq, ColorSpace::Hlg] {
        let p010 = frame.to_p010(YuvMatrix::Bt2020, YuvRange::Limited, transfer).unwrap();
        let dec =
            LinearFrame::from_p010(4, 2, &p010, YuvMatrix::Bt2020, YuvRange::Limited, transfer)
                .unwrap();
        assert_eq!(dec.primaries, ColorPrimaries::Rec2020);
        for (i, (&orig, &got)) in frame.pixels.iter().zip(dec.pixels.iter()).enumerate() {
            let tol = (orig.r() * 0.015).max(0.01);
            assert!(
                approx_rgb(orig.rgb(), got.rgb(), tol),
                "P010 {transfer:?} mismatch at {i}: orig={:?} got={:?}",
                orig.rgb(),
                got.rgb()
            );
        }
    }
}

#[test]
fn frame_error_handling() {
    assert_eq!(
        LinearFrame::from_bgra8_srgb(0, 4, &[]),
        Err(FrameError::InvalidDimensions)
    );
    assert_eq!(
        LinearFrame::from_bgra8_srgb(4, 4, &[0u8; 10]),
        Err(FrameError::BufferTooShort)
    );
    assert_eq!(
        LinearFrame::from_nv12(3, 4, &[0u8; 20], YuvMatrix::Bt709, YuvRange::Limited, ColorSpace::Srgb),
        Err(FrameError::InvalidDimensions)
    );
    assert_eq!(
        LinearFrame::from_p010(4, 4, &[0u8; 8], YuvMatrix::Bt2020, YuvRange::Limited, ColorSpace::Hdr10Pq),
        Err(FrameError::BufferTooShort)
    );
    assert_eq!(
        LinearFrame::from_nv12(2, 2, &[0u8; 6], YuvMatrix::Bt709, YuvRange::Limited, ColorSpace::Oklab),
        Err(FrameError::UnsupportedColorSpace)
    );
}

// ===========================================================================
// 3. lut.rs tests
// ===========================================================================

#[test]
fn cube_1d_parse_eval_and_roundtrip() {
    let cube = "\
# Comment header
TITLE \"Test 1D Gain\"
LUT_1D_SIZE 3
DOMAIN_MIN 0.0 0.0 0.0
DOMAIN_MAX 2.0 2.0 2.0

0.0 0.1 0.2 # inline comment
0.5 0.5 0.5
1.0 0.9 0.8
";
    let parsed = CubeLut::parse(cube).unwrap();
    let lut = match parsed {
        CubeLut::Lut1D(l) => l,
        _ => panic!("Expected 1D LUT"),
    };
    assert_eq!(lut.title.as_deref(), Some("Test 1D Gain"));
    assert_eq!(lut.size, 3);
    // Input 1.0 in [0.0, 2.0] maps to exact midpoint index 1 -> [0.5, 0.5, 0.5].
    assert!(approx_rgb(lut.eval([1.0, 1.0, 1.0]), [0.5, 0.5, 0.5], 1e-6));
    // Input 0.5 in [0.0, 2.0] maps to t=0.25 -> halfway between index 0 and 1.
    assert!(approx_rgb(lut.eval([0.5, 0.5, 0.5]), [0.25, 0.30, 0.35], 1e-6));

    let serialized = lut.to_cube();
    let reparsed = Lut1D::parse_cube(&serialized).unwrap();
    assert_eq!(reparsed.size, lut.size);
    assert!(approx_rgb(reparsed.eval([0.5, 0.5, 0.5]), [0.25, 0.30, 0.35], 1e-5));
}

#[test]
fn cube_3d_adobe_order_r_fastest_and_roundtrip() {
    // 2x2x2 LUT that swaps (R, G, B) -> (B, R, G) to verify R-fastest Adobe .cube order.
    let cube = "\
TITLE \"Swap BRG\"
LUT_3D_SIZE 2
DOMAIN_MIN 0.0 0.0 0.0
DOMAIN_MAX 1.0 1.0 1.0
0.0 0.0 0.0
0.0 1.0 0.0
0.0 0.0 1.0
0.0 1.0 1.0
1.0 0.0 0.0
1.0 1.0 0.0
1.0 0.0 1.0
1.0 1.0 1.0
";
    let lut = Lut3D::parse_cube(cube).unwrap();
    assert_eq!(lut.title.as_deref(), Some("Swap BRG"));
    assert!(approx_rgb(lut.eval([1.0, 0.0, 0.0], InterpMode::Tetrahedral), [0.0, 1.0, 0.0], 1e-6));
    assert!(approx_rgb(lut.eval([0.0, 1.0, 0.0], InterpMode::Tetrahedral), [0.0, 0.0, 1.0], 1e-6));
    assert!(approx_rgb(lut.eval([0.0, 0.0, 1.0], InterpMode::Tetrahedral), [1.0, 0.0, 0.0], 1e-6));

    let serialized = lut.to_cube();
    let reparsed = Lut3D::parse_cube(&serialized).unwrap();
    assert_eq!(reparsed.table, lut.table);
}

#[test]
fn cube_parser_rejects_invalid_inputs() {
    assert_eq!(CubeLut::parse("# empty"), Err(LutError::MissingSize));
    assert_eq!(
        CubeLut::parse("LUT_1D_SIZE 2\nLUT_3D_SIZE 2\n"),
        Err(LutError::ConflictingDimensions)
    );
    assert_eq!(CubeLut::parse("LUT_3D_SIZE 1\n0 0 0\n"), Err(LutError::InvalidSize));
    assert_eq!(
        CubeLut::parse("LUT_1D_SIZE 2\nDOMAIN_MIN 1 0 0\nDOMAIN_MAX 1 1 1\n0 0 0\n1 1 1\n"),
        Err(LutError::InvalidDomain)
    );
    assert_eq!(
        CubeLut::parse("LUT_3D_SIZE 2\n0 0 0\n1 1 1\n"),
        Err(LutError::EntryCountMismatch)
    );
    assert_eq!(
        CubeLut::parse("LUT_1D_SIZE 2\n0 0 0\nNaN 1 1\n"),
        Err(LutError::ParseError)
    );
}

#[test]
fn tetrahedral_preserves_neutral_axis_unlike_trilinear() {
    // 2x2x2 LUT with exact neutral diagonal [0,0,0]->[0,0,0], [1,1,1]->[1,1,1],
    // but skewed off-diagonal chromatic corners.
    let mut lut = Lut3D::identity(2).unwrap();
    // Corrupt c100 (pure red corner) to bright green [0, 1, 0].
    lut.table[1] = [0.0, 1.0, 0.0];

    let gray = [0.5, 0.5, 0.5];
    let tri = lut.eval(gray, InterpMode::Trilinear);
    let tet = lut.eval(gray, InterpMode::Tetrahedral);

    // Tetrahedral touches ONLY c000 and c111 on the neutral diagonal -> exact [0.5, 0.5, 0.5]!
    assert!(approx_rgb(tet, gray, 1e-6), "Tetrahedral contaminated gray: {:?}", tet);
    // Trilinear mixes 1/8 of c100 into gray -> non-neutral!
    assert!((tri[0] - tri[1]).abs() > 0.05);
}

#[test]
fn tetrahedral_affine_exact_and_continuous_across_all_6_sectors() {
    // Arbitrary affine color matrix + offset.
    let affine = |rgb: [f32; 3]| -> [f32; 3] {
        [
            0.1 + 0.7 * rgb[0] + 0.1 * rgb[1] + 0.05 * rgb[2],
            0.05 + 0.15 * rgb[0] + 0.65 * rgb[1] + 0.1 * rgb[2],
            0.02 + 0.05 * rgb[0] + 0.2 * rgb[1] + 0.7 * rgb[2],
        ]
    };
    let size = 5;
    let mut table = Vec::with_capacity(size * size * size);
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                table.push(affine([r as f32 / 4.0, g as f32 / 4.0, b as f32 / 4.0]));
            }
        }
    }
    let lut = Lut3D::from_table(size, [0.0; 3], [1.0; 3], table, None).unwrap();

    // Test points strictly inside each of the 6 tetrahedra orderings of (r, g, b).
    let pts = [
        [0.83, 0.51, 0.17], // r > g > b
        [0.83, 0.17, 0.51], // r > b > g
        [0.51, 0.17, 0.83], // b > r > g
        [0.51, 0.83, 0.17], // g > r > b
        [0.17, 0.83, 0.51], // g > b > r
        [0.17, 0.51, 0.83], // b > g > r
    ];
    for pt in pts {
        let got_tet = lut.eval(pt, InterpMode::Tetrahedral);
        let got_tri = lut.eval(pt, InterpMode::Trilinear);
        let expected = affine(pt);
        assert!(approx_rgb(got_tet, expected, 1e-5), "tet failed at {:?}", pt);
        assert!(approx_rgb(got_tri, expected, 1e-5), "tri failed at {:?}", pt);
    }

    // Also test continuity across a tetrahedron boundary (fr == fg > fb) on a non-linear LUT.
    let film_lut = FilmStock::Kodak2383Print.bake_lut3d(17);
    let eps = 1e-4;
    let a = film_lut.eval([0.42 + eps, 0.42, 0.20], InterpMode::Tetrahedral);
    let b = film_lut.eval([0.42, 0.42 + eps, 0.20], InterpMode::Tetrahedral);
    assert!(approx_rgb(a, b, 5e-4), "Discontinuity across tetrahedron face: {:?} vs {:?}", a, b);
}

#[test]
fn hald_clut_sex_image_roundtrip() {
    // Level 2 Hald CLUT => 8x8 image => 4x4x4 3D LUT.
    let size = 4;
    let mut table = Vec::with_capacity(64);
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                // Simple invertible grade: boost red slightly, keep in [0, 1].
                table.push([
                    (r as f32 / 3.0 * 0.8 + 0.1).clamp(0.0, 1.0),
                    g as f32 / 3.0,
                    b as f32 / 3.0,
                ]);
            }
        }
    }
    let lut = Lut3D::from_table(size, [0.0; 3], [1.0; 3], table, None).unwrap();
    let img: Image = lut.to_hald_image().unwrap();
    assert_eq!(img.width, 8);
    assert_eq!(img.height, 8);

    let parsed = Lut3D::from_hald_image(&img).unwrap();
    assert_eq!(parsed.size, 4);
    let out = parsed.eval([1.0, 0.5, 0.0], InterpMode::Tetrahedral);
    assert!(approx_rgb(out, [0.9, 0.5, 0.0], 0.01));

    // Reject non-cubic image dimensions.
    let bad = Image {
        width: 10,
        height: 10,
        pixels: vec![0; 100],
    };
    assert_eq!(Lut3D::from_hald_image(&bad), Err(LutError::InvalidHaldImage));
}

// ===========================================================================
// 4. film.rs tests
// ===========================================================================

#[test]
fn all_12_film_stocks_are_monotonic_bounded_and_distinct() {
    assert_eq!(FilmStock::ALL.len(), 12);
    for stock in FilmStock::ALL {
        assert!(!stock.name().is_empty());
        assert!((0.0..=1.0).contains(&stock.halation_strength()));
        assert!((0.0..=1.0).contains(&stock.grain_strength()));

        let mut prev = [-1.0f32; 3];
        for step in 0..=20 {
            let v = step as f32 / 20.0;
            let out = stock.apply_linear([v, v, v]);
            for c in 0..3 {
                assert!(out[c].is_finite() && out[c] >= 0.0 && out[c] <= 1.0);
                assert!(out[c] >= prev[c] - 1e-6, "{stock:?} non-monotonic on ch {c} at {v}");
            }
            prev = out;
        }
    }

    // Kodak 2383 vs Fuji 3510 vs Ektar 100 produce distinct chromatic signatures on skin/warm midtone.
    let sample = [0.45, 0.25, 0.14];
    let k2383 = FilmStock::Kodak2383Print.apply_linear(sample);
    let f3510 = FilmStock::Fuji3510Print.apply_linear(sample);
    let ektar = FilmStock::Ektar100.apply_linear(sample);
    assert!(!approx_rgb(k2383, f3510, 0.01));
    assert!(!approx_rgb(k2383, ektar, 0.01));
}

#[test]
fn cineon_log_exact_black_white_and_roundtrip() {
    assert!(approx_eq(linear_to_cineon_log(0.0), 95.0 / 1023.0, 1e-6));
    assert!(approx_eq(linear_to_cineon_log(1.0), 685.0 / 1023.0, 1e-6));
    // Cineon log leaves highlight headroom above 1.0 up to ~13.5x linear!
    assert!(linear_to_cineon_log(10.0) < 0.99);
    assert_eq!(linear_to_cineon_log(14.0), 1.0);

    for step in 0..=100 {
        let lin = (step as f32 / 30.0).powi(2); // 0.0 ..= 11.11x (within 1023 code ceiling)
        let log_cv = linear_to_cineon_log(lin);
        let back = cineon_log_to_linear(log_cv);
        assert!(approx_eq(lin, back, 1e-4), "Cineon roundtrip failed at {lin}: {back}");
    }
}

#[test]
fn film_stock_bake_lut3d_matches_direct_evaluation() {
    for stock in [FilmStock::Kodak2383Print, FilmStock::Portra400, FilmStock::CineonLog] {
        let lut = stock.bake_lut3d(33);
        assert_eq!(lut.size, 33);
        assert_eq!(lut.title.as_deref(), Some(stock.name()));

        let probe = [0.32, 0.58, 0.19];
        let direct = stock.apply_linear(probe);
        let from_lut = lut.eval(probe, InterpMode::Tetrahedral);
        assert!(approx_rgb(direct, from_lut, 0.01), "{stock:?}: {direct:?} vs {from_lut:?}");

        // Roundtrip through .cube text format.
        let parsed = Lut3D::parse_cube(&lut.to_cube()).unwrap();
        assert!(approx_rgb(parsed.eval(probe, InterpMode::Tetrahedral), from_lut, 1e-5));
    }
}

// ===========================================================================
// 5. wheels.rs tests
// ===========================================================================

#[test]
fn color_grade_default_is_exact_identity_sdr_and_hdr() {
    let grade = ColorGrade::default();
    for sample in [
        [0.0, 0.0, 0.0],
        [0.18, 0.18, 0.18],
        [0.8, 0.3, 0.1],
        [1.0, 1.0, 1.0],
        [4.5, 2.2, 0.9],  // HDR highlights
        [-0.1, 0.5, 0.2], // wide-gamut out-of-709 values
    ] {
        assert!(approx_rgb(grade.eval(sample), sample, 1e-6));
    }
}

#[test]
fn color_grade_lift_gamma_gain_offset_and_exposure() {
    // Lift raises black while anchoring white at 1.0.
    let mut g = ColorGrade::default();
    g.lift = [0.1, 0.0, -0.05];
    assert!(approx_rgb(g.eval([0.0, 0.0, 0.0]), [0.1, 0.0, -0.05], 1e-6));
    assert!(approx_rgb(g.eval([1.0, 1.0, 1.0]), [1.0, 1.0, 1.0], 1e-6));

    // Gain scales white while anchoring black at 0.0.
    let mut g = ColorGrade::default();
    g.gain = [1.5, 1.0, 0.5];
    assert!(approx_rgb(g.eval([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0], 1e-6));
    assert!(approx_rgb(g.eval([1.0, 1.0, 1.0]), [1.5, 1.0, 0.5], 1e-6));

    // Gamma > 1 brightens midtones while anchoring 0.0 and 1.0.
    let mut g = ColorGrade::default();
    g.gamma = [2.0, 2.0, 2.0];
    assert!(approx_rgb(g.eval([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0], 1e-6));
    assert!(approx_rgb(g.eval([1.0, 1.0, 1.0]), [1.0, 1.0, 1.0], 1e-6));
    assert!(approx_rgb(g.eval([0.25, 0.25, 0.25]), [0.5, 0.5, 0.5], 1e-5));

    // Offset shifts uniformly, Exposure EV scales by 2^EV.
    let mut g = ColorGrade::default();
    g.offset = [0.05, -0.02, 0.01];
    g.exposure_ev = 1.0; // * 2.0
    assert!(approx_rgb(g.eval([0.2, 0.2, 0.2]), [0.45, 0.38, 0.41], 1e-6));
}

#[test]
fn color_grade_contrast_pivot_and_luma_preserving_sat_vibrance() {
    let mut g = ColorGrade::default();
    g.contrast = 1.6;
    g.pivot = 0.18;
    // Exact anchor at pivot 0.18.
    assert!(approx_rgb(g.eval([0.18, 0.18, 0.18]), [0.18, 0.18, 0.18], 1e-6));
    // Below pivot becomes darker, above pivot becomes brighter.
    assert!(g.eval([0.09, 0.09, 0.09])[0] < 0.09);
    assert!(g.eval([0.50, 0.50, 0.50])[0] > 0.50);

    // Saturation = 0 gives neutral luma, Saturation = 1.8 preserves exact BT.709 & BT.2020 luma!
    let sample = [0.7, 0.35, 0.15];
    let orig_luma709 = RgbaF32::from_rgb(sample, 1.0).rec709_luma();
    let orig_luma2020 = RgbaF32::from_rgb(sample, 1.0).rec2020_luma();

    let mut gs0 = ColorGrade::default();
    gs0.saturation = 0.0;
    assert!(approx_rgb(gs0.eval(sample), [orig_luma709; 3], 1e-6));

    let mut gs2 = ColorGrade::default();
    gs2.saturation = 1.8;
    let out709 = gs2.eval_rgb(sample, ColorPrimaries::Rec709);
    let out2020 = gs2.eval_rgb(sample, ColorPrimaries::Rec2020);
    assert!(approx_eq(RgbaF32::from_rgb(out709, 1.0).rec709_luma(), orig_luma709, 1e-5));
    assert!(approx_eq(RgbaF32::from_rgb(out2020, 1.0).rec2020_luma(), orig_luma2020, 1e-5));

    // Vibrance boosts muted colors more than already-saturated colors while preserving luma.
    let mut gv = ColorGrade::default();
    gv.vibrance = 0.8;
    let muted = [0.45, 0.40, 0.38];
    let vivid = [0.90, 0.10, 0.10];
    let muted_out = gv.eval(muted);
    let vivid_out = gv.eval(vivid);
    let muted_gain = (muted_out[0] - muted_out[2]) / (muted[0] - muted[2]);
    let vivid_gain = (vivid_out[0] - vivid_out[2]) / (vivid[0] - vivid[2]);
    assert!(muted_gain > vivid_gain);
    assert!(approx_eq(
        RgbaF32::from_rgb(muted_out, 1.0).rec709_luma(),
        RgbaF32::from_rgb(muted, 1.0).rec709_luma(),
        1e-5
    ));
}

#[test]
fn color_grade_temperature_tint_and_hue_vs_sat() {
    let mut gw = ColorGrade::default();
    gw.temperature = 0.5;
    let warm = gw.eval([0.5, 0.5, 0.5]);
    assert!(warm[0] > warm[2]);

    let mut gt = ColorGrade::default();
    gt.tint = 0.5; // magenta -> lower green
    let mag = gt.eval([0.5, 0.5, 0.5]);
    assert!(mag[1] < mag[0] && mag[1] < mag[2]);

    // Desaturate red (hue 0.0 / 1.0) while keeping green (hue 0.333) untouched.
    let mut gh = ColorGrade::default();
    gh.hue_vs_sat = Curve1D::from_points(&[
        [0.0, 0.0],
        [0.15, 1.0],
        [0.85, 1.0],
        [1.0, 0.0],
    ]);
    let red_out = gh.eval([0.8, 0.1, 0.1]);
    let green_out = gh.eval([0.1, 0.8, 0.1]);
    assert!(approx_eq(red_out[0], red_out[1], 1e-4) && approx_eq(red_out[1], red_out[2], 1e-4));
    assert!(approx_rgb(green_out, [0.1, 0.8, 0.1], 1e-4));
}

#[test]
fn curve1d_pchip_monotonicity_without_overshoot() {
    let knots = [[0.0, 0.0], [0.25, 0.04], [0.35, 0.65], [0.75, 0.96], [1.0, 1.0]];
    let curve = Curve1D::from_points(&knots);

    // Passes exactly through every control point.
    for [x, y] in knots {
        assert!(approx_eq(curve.eval(x), y, 1e-6));
    }

    // Strictly monotonic everywhere with zero overshoot across 500 samples.
    let mut prev = -1.0f32;
    for i in 0..=500 {
        let x = i as f32 / 500.0;
        let y = curve.eval(x);
        assert!(y >= prev - 1e-6, "PCHIP overshoot/non-monotonic at {x}: {y} < {prev}");
        assert!((0.0..=1.0).contains(&y));
        prev = y;
    }
}

// ===========================================================================
// 6. scopes.rs tests
// ===========================================================================

#[test]
fn waveform_ire_graticule_and_bt709_vs_bt2020_luma() {
    let mut surf709 = ScopeSurface::new(64, 64);
    let mut surf2020 = ScopeSurface::new(64, 64);

    // Solid pure green linear frame.
    let green = LinearFrame::filled(
        8,
        8,
        ColorPrimaries::Rec709,
        RgbaF32([0.0, 1.0, 0.0, 1.0]),
    )
    .unwrap();
    surf709.render_waveform(&green, WaveformMode::Luma, ScopeStandard::Rec709);

    let green2020 = LinearFrame::filled(
        8,
        8,
        ColorPrimaries::Rec2020,
        RgbaF32([0.0, 1.0, 0.0, 1.0]),
    )
    .unwrap();
    surf2020.render_waveform(&green2020, WaveformMode::Luma, ScopeStandard::Rec2020);

    // BT.709 green luma (71.52 IRE) sits strictly higher (smaller y) than BT.2020 green luma (67.80 IRE)!
    let y709 = surf709.ire_to_y(71.52);
    let y2020 = surf2020.ire_to_y(67.80);
    assert!(y709 < y2020);
    // Trace pixels at respective IRE rows should be brighter green than background.
    let px709 = surf709.pixel(32, y709).unwrap();
    let px2020 = surf2020.pixel(32, y2020).unwrap();
    assert!(((px709 >> 8) & 0xFF) > 150);
    assert!(((px2020 >> 8) & 0xFF) > 150);

    // Black frame hits 0 IRE row, White frame hits 100 IRE row.
    let black = LinearFrame::filled(4, 4, ColorPrimaries::Rec709, RgbaF32::BLACK).unwrap();
    surf709.render_waveform(&black, WaveformMode::Luma, ScopeStandard::Rec709);
    let y0 = surf709.ire_to_y(0.0);
    assert!(((surf709.pixel(16, y0).unwrap() >> 8) & 0xFF) > 150);
}

#[test]
fn waveform_rgb_overlay_from_bgra8_slice() {
    // 4x2 BGRA8 solid pixel: B=0, G=128, R=255, A=255.
    let bgra = [0u8, 128, 255, 255].repeat(8);
    let mut surf = ScopeSurface::new(64, 64);
    surf.render_waveform(ScopeInput::bgra8(4, 2, &bgra), WaveformMode::RgbOverlay, ScopeStandard::Rec709);

    let y_r = surf.ire_to_y(100.0);
    let y_g = surf.ire_to_y(128.0 / 255.0 * 100.0);
    let y_b = surf.ire_to_y(0.0);

    assert!(((surf.pixel(32, y_r).unwrap() >> 16) & 0xFF) > 200);
    assert!(((surf.pixel(32, y_g).unwrap() >> 8) & 0xFF) > 200);
    assert!((surf.pixel(32, y_b).unwrap() & 0xFF) > 200);
}

#[test]
fn rgb_parade_separates_three_panels() {
    // Pure blue linear frame: R=0 IRE, G=0 IRE, B=100 IRE.
    let blue = LinearFrame::filled(
        6,
        4,
        ColorPrimaries::Rec709,
        RgbaF32([0.0, 0.0, 1.0, 1.0]),
    )
    .unwrap();
    let mut surf = ScopeSurface::new(90, 60);
    surf.render_rgb_parade(&blue, ScopeStandard::Rec709);

    let y_0 = surf.ire_to_y(0.0);
    let y_100 = surf.ire_to_y(100.0);

    // Left panel (x=15): red peak at 0 IRE.
    assert!(((surf.pixel(15, y_0).unwrap() >> 16) & 0xFF) > 200);
    // Middle panel (x=45): green peak at 0 IRE.
    assert!(((surf.pixel(45, y_0).unwrap() >> 8) & 0xFF) > 200);
    // Right panel (x=75): blue peak at 100 IRE.
    assert!((surf.pixel(75, y_100).unwrap() & 0xFF) > 200);
}

#[test]
fn vectorscope_smpte_targets_and_skin_tone_iline() {
    let mut surf = ScopeSurface::new(128, 128);
    // Neutral gray lands at exact center (cx, cy).
    let gray = LinearFrame::filled(4, 4, ColorPrimaries::Rec709, RgbaF32([0.2, 0.2, 0.2, 1.0])).unwrap();
    surf.render_vectorscope(&gray, ScopeStandard::Rec709);
    let (cx, cy, _) = surf.vectorscope_geometry();
    let center_px = surf.pixel(cx as usize, cy as usize).unwrap();
    assert!((center_px & 0xFF) > 100);

    // Realistic skin tone BGRA8 [B=117, G=145, R=194, A=255] plots in upper-left quadrant near I-line!
    let skin_bgra = [117u8, 145, 194, 255].repeat(16);
    surf.render_vectorscope(ScopeInput::bgra8(4, 4, &skin_bgra), ScopeStandard::Rec709);
    let (sx, sy) = surf.vectorscope_coords([194.0 / 255.0, 145.0 / 255.0, 117.0 / 255.0], ScopeStandard::Rec709);
    assert!(sx < cx && sy < cy, "Skin tone must land in upper-left Cb<0 Cr>0 quadrant");
    let skin_px = surf.pixel(sx as usize, sy as usize).unwrap();
    assert!(((skin_px >> 16) & 0xFF) > 150);

    // Skin-tone I-line endpoint is in upper-left quadrant (~123 deg).
    let (ix, iy) = surf.skin_tone_iline_endpoint();
    assert!(ix < cx && iy < cy);
}

#[test]
fn histogram_256_bins_counts_and_rendering() {
    // 4 pixels in BGRA8: two identical mid-tones and two distinct primaries.
    let bgra = [
        0u8, 128, 255, 255, // R=255, G=128, B=0
        0u8, 128, 255, 255, // R=255, G=128, B=0
        255u8, 0, 0, 255,   // R=0,   G=0,   B=255
        0u8, 255, 0, 255,   // R=0,   G=255, B=0
    ];
    let hist = compute_histogram(ScopeInput::bgra8(2, 2, &bgra), ScopeStandard::Rec709);
    assert_eq!(hist.total_pixels, 4);
    assert_eq!(hist.r[255], 2);
    assert_eq!(hist.r[0], 2);
    assert_eq!(hist.g[128], 2);
    assert_eq!(hist.g[255], 1);
    assert_eq!(hist.g[0], 1);
    assert_eq!(hist.b[0], 3);
    assert_eq!(hist.b[255], 1);

    let mut surf = ScopeSurface::new(256, 64);
    let rendered = surf.render_histogram(ScopeInput::bgra8(2, 2, &bgra), ScopeStandard::Rec709);
    assert_eq!(rendered, hist);
    // Peak bin is b[0] == 3, so at x=0, y=0 the blue curve reaches the very top of the histogram!
    assert_eq!(surf.pixel(0, 0).unwrap(), 0xFF40_90FF);
}

#[test]
fn linear_frame_tone_map_and_wide_gamut_compositing() {
    // Composite an SDR sRGB graphic scaled to 203 nits onto an HDR LinearRec2020 canvas,
    // then tone map down to 100-nit SDR and verify bounded output.
    let mut hdr = LinearFrame::filled(
        2,
        2,
        ColorPrimaries::Rec2020,
        RgbaF32([0.0, 8.0, 0.0, 1.0]), // 800-nit wide-gamut Rec.2020 green
    )
    .unwrap();
    let sdr_white_in_2020 = convert_primaries([1.0, 1.0, 1.0], ColorPrimaries::Rec709, ColorPrimaries::Rec2020);
    let scale = sdr_on_hdr_scale(203.0);
    let overlay = RgbaF32::from_rgb(
        [
            sdr_white_in_2020[0] * scale,
            sdr_white_in_2020[1] * scale,
            sdr_white_in_2020[2] * scale,
        ],
        0.5,
    );
    let composited = overlay.over_straight(hdr.pixels[0]);
    hdr.set_pixel(0, 0, composited);

    hdr.tone_map_in_place(ToneMapConfig {
        mode: ToneMapMode::Bt2390,
        source_peak_nits: 1_000.0,
        target_peak_nits: 100.0,
    });
    for px in &hdr.pixels {
        assert!(px.r() <= 1.0001 && px.g() <= 1.0001 && px.b() <= 1.0001);
        assert!(px.r() >= 0.0 && px.g() >= 0.0 && px.b() >= 0.0);
    }
}

#[test]
fn color_grade_apply_frame_and_bake_lut3d() {
    let mut grade = ColorGrade::default();
    grade.exposure_ev = 0.5;
    grade.contrast = 1.25;
    grade.saturation = 1.2;
    grade.master = Curve1D::from_points(&[[0.0, 0.0], [0.5, 0.55], [1.0, 1.0]]);
    grade.r = Curve1D::from_points(&[[0.0, 0.02], [1.0, 0.98]]);

    let mut frame = LinearFrame::filled(2, 2, ColorPrimaries::Rec709, RgbaF32([0.2, 0.3, 0.4, 0.75])).unwrap();
    let expected = grade.eval_rgb([0.2, 0.3, 0.4], ColorPrimaries::Rec709);
    grade.apply_frame(&mut frame);
    assert!(approx_rgb(frame.pixels[0].rgb(), expected, 1e-6));
    assert_eq!(frame.pixels[0].a(), 0.75);

    let lut = grade.bake_lut3d(17, ColorPrimaries::Rec709);
    let probe = [0.25, 0.35, 0.45];
    assert!(approx_rgb(
        lut.eval(probe, InterpMode::Tetrahedral),
        grade.eval_rgb(probe, ColorPrimaries::Rec709),
        0.015
    ));
}

#[test]
fn lut1d_and_lut3d_apply_frame_preserve_alpha() {
    let lut1d = Lut1D::identity(8).unwrap();
    let lut3d = Lut3D::identity(8).unwrap();
    let mut frame = LinearFrame::filled(3, 3, ColorPrimaries::Rec2020, RgbaF32([0.3, 0.6, 0.9, 0.4])).unwrap();
    let orig = frame.clone();
    lut1d.apply_frame(&mut frame);
    lut3d.apply_frame(&mut frame, InterpMode::Tetrahedral);
    for (a, b) in orig.pixels.iter().zip(frame.pixels.iter()) {
        assert!(approx_rgb(a.rgb(), b.rgb(), 1e-5));
        assert_eq!(a.a(), b.a());
    }
}

#[test]
fn p010_auto_full_range_and_strided_planes() {
    // 2x2 full-range 10-bit P010 with Y=0 and Y=1023 shifted into upper 10 bits (<< 6).
    let y_plane = [0u16 << 6, 512u16 << 6, 999u16, 768u16 << 6, 1023u16 << 6, 999u16];
    let uv_plane = [512u16 << 6, 512u16 << 6, 999u16];
    let frame = LinearFrame::from_p010_u16(
        2,
        2,
        &y_plane,
        3, // stride = 3 > width = 2
        &uv_plane,
        3,
        YuvMatrix::Bt2020,
        YuvRange::Auto,
        ColorSpace::Rec2020,
    )
    .unwrap();
    assert_eq!(frame.pixels[0].r(), 0.0);
    assert!(approx_eq(frame.pixels[3].r(), 1.0, 1e-4));
}

#[test]
fn linear_frame_nv12_strided_and_convert_primaries() {
    let mut f = LinearFrame::filled(2, 2, ColorPrimaries::Rec709, RgbaF32([0.5, 0.25, 0.1, 0.9])).unwrap();
    let f2020 = f.convert_primaries(ColorPrimaries::Rec2020);
    assert_eq!(f2020.primaries, ColorPrimaries::Rec2020);
    f.convert_primaries_in_place(ColorPrimaries::Rec2020);
    assert_eq!(f, f2020);
    let back = f.convert_primaries(ColorPrimaries::Rec709);
    assert!(approx_rgb(back.pixels[0].rgb(), [0.5, 0.25, 0.1], 1e-5));
    assert_eq!(back.pixels[0].a(), 0.9);
}

