use std::collections::BTreeMap;

use cabaret_core::audio::{AudioMixer, AudioStrip, ChannelLayout, StripId};
use cabaret_core::compositor::{BoundsMode, CropRect, FrameBuffer, Transform2D};
use cabaret_core::filters::{BlurKind, FilterChain, FilterEntry, VideoFilter};
use cabaret_core::outputs::{
    AudioEncodeProfile, AudioTrackMask, ContainerFormat, DestId, EncodedPacket, MediaKind,
    RecordingStep, ServicePlatform, ShibariSecretSlot, StreamDest, VideoEncodeProfile,
};
use cabaret_core::scene::{SceneCollection, SourceKind, StudioId};
use cabaret_core::transitions::{
    render_transition_frame, StudioMode, TallyState, TransitionKind, WipeDirection,
};

use cabaret_plugins::adv_switcher::{
    execute_macro_actions, Condition, MacroAction, MacroEngine, MacroOutputCommand, MacroRule,
    MacroWorldState, MediaPlaybackSnapshot, StreamActiveState,
};
use cabaret_plugins::downstream_keyer::{DownstreamKeyerBank, DskChannel, DskTransition};
use cabaret_plugins::local_captions::{
    decode_cea608_pairs, depacketize_cea708, encode_cea608_pairs, has_odd_parity, packetize_cea708,
    render_lower_third_captions, CaptionBuffer, CaptionBurnInStyle, Cea608Mode, TranscriptSegment,
};
use cabaret_plugins::move_transition::{
    apply_numeric_filter_params, extract_numeric_filter_params, EasingCurve, MatchStrategy,
    MoveItemExtras, MoveItemKind, MoveTransitionConfig, MoveTransitionPlan,
};
use cabaret_plugins::multi_canvas::{
    CanvasPreset, SecondaryCanvasItem, SecondaryCanvasManager,
};
use cabaret_plugins::overlays::{
    BellLastPlayer, InputOverlayHud, InputOverlayStyle, InputStateSnapshot, NowPlayingCard,
    NowPlayingStyle, NowPlayingTrack,
};
use cabaret_plugins::shader_filter::{
    apply_shader_preset, eval_shader_program, ShaderError, ShaderExpr, ShaderPreset, ShaderProgram,
    ShaderUniforms,
};
use cabaret_plugins::source_record::{
    decode_tapped_video_packet, FreezeFilter, SourceRecordTap,
};
use cabaret_plugins::spectralizer::{
    compute_stereo_correlation, render_bar_spectrum, render_circular_spectrum, render_goniometer,
    BarSpectrumConfig, CircularSpectrumConfig, GoniometerConfig, SpectrumAnalyzer, SpectrumConfig,
};
use cabaret_plugins::streamfx_3d::{
    apply_dynamic_mask_feather, apply_perspective_3d, dual_kawase_blur, solve_homography_4pt,
    warp_corner_pin_normalized, CornerPinQuad, DualKawaseBlur, DynamicMaskFeather, PitchYawRollFov,
    StreamFxError,
};
use cabaret_plugins::studio_link::{
    LinkAudioFrame, LinkError, LinkPacket, LinkPayload, LinkPixelFormat, LinkStreamDecoder,
    LinkTally, LinkVideoFrame, ServiceAnnouncement,
};
use cabaret_plugins::time_warp::{
    DynamicDelayBuffer, ScanDirection, TimeWarpConfig, TimeWarpScan,
};
use cabaret_plugins::wasm_fx::{
    framebuffer_to_linear_frame, linear_frame_to_framebuffer, ColorPrimaries, FxError, FxLimits,
    Rgba8Transfer, WasmFxBridge, WasmFxError, DEFAULT_FRAME_FUEL,
};

fn solid(w: u32, h: u32, rgba: [f32; 4]) -> FrameBuffer {
    FrameBuffer::filled(w, h, rgba).expect("valid framebuffer")
}

// =========================================================================
// 1. MOVE TRANSITION (obs-move-transition)
// =========================================================================

#[test]
fn test_move_transition_matches_by_source_id_and_name_with_slide_vectors() {
    let mut col = SceneCollection::new("Show", 1920, 1080);
    let cam = col
        .add_source("Cam", SourceKind::ColorSolid { rgba: [1.0, 0.0, 0.0, 1.0] })
        .unwrap();
    let logo_a = col
        .add_source("LogoA", SourceKind::ColorSolid { rgba: [0.0, 1.0, 0.0, 1.0] })
        .unwrap();
    let logo_b = col
        .add_source("LogoB", SourceKind::ColorSolid { rgba: [0.0, 1.0, 0.0, 1.0] })
        .unwrap();
    let chat = col
        .add_source("Chat", SourceKind::ColorSolid { rgba: [0.0, 0.0, 1.0, 1.0] })
        .unwrap();
    let alert = col
        .add_source("Alert", SourceKind::ColorSolid { rgba: [1.0, 1.0, 0.0, 1.0] })
        .unwrap();

    let s_from = col.add_studio("Program");
    let s_to = col.add_studio("Preview");

    let i_cam_from = col.add_item(s_from, cam, "Cam").unwrap();
    let i_logo_from = col.add_item(s_from, logo_a, "BrandBug").unwrap();
    let _i_chat_from = col.add_item(s_from, chat, "ChatBox").unwrap();

    let i_cam_to = col.add_item(s_to, cam, "CamWide").unwrap();
    let i_logo_to = col.add_item(s_to, logo_b, "BrandBug").unwrap(); // different SourceId, matched by name!
    let _i_alert_to = col.add_item(s_to, alert, "AlertBox").unwrap();

    // Configure transforms: shortest rotation path 350 -> 10 (+20 deg across 0).
    let st_from = col.studio_mut(s_from).unwrap();
    st_from.items[0].transform.pos = [100.0, 200.0];
    st_from.items[0].transform.scale = [1.0, 1.0];
    st_from.items[0].transform.rot_deg = 350.0;
    st_from.items[0].transform.crop = CropRect { top: 0.0, right: 10.0, bottom: 0.0, left: 0.0 };
    st_from.items[1].transform.pos = [50.0, 50.0];
    st_from.items[2].transform.pos = [400.0, 300.0];

    let st_to = col.studio_mut(s_to).unwrap();
    st_to.items[0].transform.pos = [300.0, 400.0];
    st_to.items[0].transform.scale = [2.0, 0.5];
    st_to.items[0].transform.rot_deg = 10.0;
    st_to.items[0].transform.crop = CropRect { top: 20.0, right: 0.0, bottom: 0.0, left: 0.0 };
    st_to.items[1].transform.pos = [150.0, 50.0];
    st_to.items[2].transform.pos = [800.0, 100.0];

    let plan = MoveTransitionPlan::build(
        col.studio(s_from).unwrap(),
        col.studio(s_to).unwrap(),
        MoveTransitionConfig {
            duration_ms: 500,
            easing: EasingCurve::Linear,
            match_strategy: MatchStrategy::SourceIdFirstThenName,
            enter_slide: [-200.0, 0.0],
            exit_slide: [0.0, 100.0],
            fade_unmatched: true,
            shortest_rotation_path: true,
        },
    );
    assert_eq!(plan.track_count(), 4);

    let mid = plan.evaluate(0.5);
    let cam_mid = mid
        .iter()
        .find(|m| m.from_item_id == Some(i_cam_from) && m.to_item_id == Some(i_cam_to))
        .expect("cam matched by SourceId");
    assert_eq!(cam_mid.kind, MoveItemKind::Matched);
    assert!((cam_mid.item.transform.pos[0] - 200.0).abs() < 1e-4);
    assert!((cam_mid.item.transform.pos[1] - 300.0).abs() < 1e-4);
    assert!((cam_mid.item.transform.scale[0] - 1.5).abs() < 1e-4);
    assert!((cam_mid.item.transform.rot_deg - 360.0).abs() < 1e-4);
    assert!((cam_mid.item.transform.crop.top - 10.0).abs() < 1e-4);
    assert!((cam_mid.item.transform.crop.right - 5.0).abs() < 1e-4);

    let logo_mid = mid
        .iter()
        .find(|m| m.from_item_id == Some(i_logo_from) && m.to_item_id == Some(i_logo_to))
        .expect("BrandBug matched by name");
    assert_eq!(logo_mid.kind, MoveItemKind::Matched);
    assert!((logo_mid.item.transform.pos[0] - 100.0).abs() < 1e-4);

    let exiting = mid.iter().find(|m| m.kind == MoveItemKind::Exiting).unwrap();
    assert!((exiting.item.transform.pos[1] - 350.0).abs() < 1e-4);
    assert!((exiting.item.opacity - 0.5).abs() < 1e-4);

    let entering = mid.iter().find(|m| m.kind == MoveItemKind::Entering).unwrap();
    assert!((entering.item.transform.pos[0] - 700.0).abs() < 1e-4);
    assert!((entering.item.opacity - 0.5).abs() < 1e-4);
}

#[test]
fn test_move_transition_easing_curves_and_spring_overshoot() {
    let curves = [
        EasingCurve::Linear,
        EasingCurve::CubicInOut,
        EasingCurve::BackOut,
        EasingCurve::Spring {
            stiffness: 180.0,
            damping: 12.0,
        },
        EasingCurve::Bounce,
    ];
    for curve in curves {
        assert!((curve.eval(0.0) - 0.0).abs() < 1e-5, "{curve:?} at 0");
        assert!((curve.eval(1.0) - 1.0).abs() < 1e-5, "{curve:?} at 1");
    }
    assert!(EasingCurve::BackOut.eval(0.5) > 1.0);
    let spring = EasingCurve::Spring {
        stiffness: 180.0,
        damping: 10.0,
    };
    let max_spring = (1..20).map(|i| spring.eval(i as f32 / 20.0)).fold(0.0_f32, f32::max);
    assert!(max_spring > 1.05, "underdamped spring should overshoot: {max_spring}");
    assert!(EasingCurve::Bounce.eval(0.5) < 1.0);
}

#[test]
fn test_move_transition_corner_pin_and_filter_param_interpolation_and_render() {
    let mut col = SceneCollection::new("RenderShow", 64, 64);
    let src = col
        .add_source("Panel", SourceKind::ColorSolid { rgba: [0.8, 0.2, 0.4, 1.0] })
        .unwrap();
    let mut chain = FilterChain::new();
    chain
        .add(FilterEntry {
            id: 1,
            name: String::from("Blur"),
            enabled: true,
            filter: VideoFilter::Blur {
                kind: BlurKind::Gaussian,
                radius: 0.0,
                passes: 1,
            },
        })
        .unwrap();
    col.source_mut(src).unwrap().filters = chain.clone();

    let s_from = col.add_studio("A");
    let s_to = col.add_studio("B");
    let id_a = col.add_item(s_from, src, "Panel").unwrap();
    let id_b = col.add_item(s_to, src, "Panel").unwrap();

    let mut from_extras = BTreeMap::new();
    from_extras.insert(
        id_a,
        MoveItemExtras {
            corner_pin: Some(CornerPinQuad::unit()),
            filter_params: extract_numeric_filter_params(&chain),
        },
    );

    let mut chain_b = chain.clone();
    if let VideoFilter::Blur { ref mut radius, .. } = chain_b.entries[0].filter {
        *radius = 4.0;
    }
    let mut to_extras = BTreeMap::new();
    to_extras.insert(
        id_b,
        MoveItemExtras {
            corner_pin: Some(CornerPinQuad {
                top_left: [0.25, 0.25],
                top_right: [0.75, 0.25],
                bottom_right: [0.75, 0.75],
                bottom_left: [0.25, 0.75],
            }),
            filter_params: extract_numeric_filter_params(&chain_b),
        },
    );

    let plan = MoveTransitionPlan::build_with_extras(
        col.studio(s_from).unwrap(),
        col.studio(s_to).unwrap(),
        &from_extras,
        &to_extras,
        MoveTransitionConfig {
            easing: EasingCurve::Linear,
            ..Default::default()
        },
    );

    let mid = plan.evaluate(0.5);
    assert!((mid[0].filter_params["0.blur.radius"] - 2.0).abs() < 1e-4);
    let q = mid[0].corner_pin.unwrap();
    assert!((q.top_left[0] - 0.125).abs() < 1e-4);

    let mut patched = chain.clone();
    apply_numeric_filter_params(&mut patched, &mid[0].filter_params);
    assert!(matches!(patched.entries[0].filter, VideoFilter::Blur { radius, .. } if (radius - 2.0).abs() < 1e-4));

    let mut fctx = BTreeMap::new();
    let fb = plan
        .render(0.5, 32, 32, &col, &BTreeMap::new(), &mut fctx)
        .unwrap();
    // Center is inside warped quad, corner (0, 0) is outside.
    assert!(fb.get_pixel(16, 16).unwrap()[3] > 0.9);
    assert!(fb.get_pixel(0, 0).unwrap()[3] < 0.01);
}

// =========================================================================
// 2. SHADER FILTER (obs-shaderfilter)
// =========================================================================

#[test]
fn test_shader_ast_evaluator_uses_uv_time_pixel_and_audio_rms() {
    let input = solid(8, 8, [0.4, 0.2, 0.1, 1.0]);
    // Out = vec4(pixel.r + audio_rms * param("gain"), uv.x, sin(time), 1.0)
    let expr = ShaderExpr::Vec4(
        Box::new(ShaderExpr::Add(
            Box::new(ShaderExpr::Channel(Box::new(ShaderExpr::Pixel), 0)),
            Box::new(ShaderExpr::Mul(
                Box::new(ShaderExpr::AudioRms),
                Box::new(ShaderExpr::Param(String::from("gain"))),
            )),
        )),
        Box::new(ShaderExpr::Channel(Box::new(ShaderExpr::Uv), 0)),
        Box::new(ShaderExpr::Abs(Box::new(ShaderExpr::Sin(Box::new(
            ShaderExpr::Time,
        ))))),
        Box::new(ShaderExpr::Lit(1.0)),
    );
    let prog = ShaderProgram::new("reactive_tint", expr);
    let mut params = BTreeMap::new();
    params.insert(String::from("gain"), 0.5);
    let uniforms = ShaderUniforms {
        time_s: core::f32::consts::FRAC_PI_2,
        audio_rms: 0.6,
        params,
    };

    let out = eval_shader_program(&prog, &input, &uniforms).unwrap();
    assert!(out.fuel_consumed > 0);
    let px = out.frame.get_pixel(7, 0).unwrap();
    assert!((px[0] - 0.7).abs() < 1e-4);
    assert!((px[1] - (7.5 / 8.0)).abs() < 1e-4);
    assert!((px[2] - 1.0).abs() < 1e-4);
}

#[test]
fn test_shader_ast_fuel_exhaustion_and_unknown_param() {
    let input = solid(4, 4, [0.5, 0.5, 0.5, 1.0]);
    let loop_expr = ShaderExpr::Repeat {
        count: 500,
        init: Box::new(ShaderExpr::Lit(0.0)),
        body: Box::new(ShaderExpr::Add(
            Box::new(ShaderExpr::LoopAcc),
            Box::new(ShaderExpr::Lit(0.001)),
        )),
    };
    let mut prog = ShaderProgram::new("heavy", loop_expr);
    prog.max_fuel = 150;
    let res = eval_shader_program(&prog, &input, &ShaderUniforms::default());
    assert!(matches!(res, Err(ShaderError::FuelExhausted { .. })));

    let bad_param = ShaderProgram::new("missing", ShaderExpr::Param(String::from("nope")));
    assert_eq!(
        eval_shader_program(&bad_param, &input, &ShaderUniforms::default()),
        Err(ShaderError::UnknownParam(String::from("nope")))
    );
}

#[test]
fn test_shader_all_12_presets_execute_cleanly() {
    let mut src = FrameBuffer::new(32, 32).unwrap();
    for y in 0..32 {
        for x in 0..32 {
            let v = if x < 16 { 0.1 } else { 0.9 };
            src.set_pixel(x, y, [v, (y as f32) / 32.0, 0.5, 1.0]);
        }
    }

    let presets = ShaderPreset::all_defaults();
    assert_eq!(presets.len(), 12);
    for preset in &presets {
        let out = apply_shader_preset(&src, preset, 1.25, 0.35).expect("preset succeeds");
        assert_eq!(out.width, 32);
        assert_eq!(out.height, 32);
        assert!(out.pixels.iter().all(|p| p.iter().all(|c| c.is_finite())));
    }

    // Vignette dims corners more than center.
    let vig = apply_shader_preset(
        &solid(32, 32, [1.0, 1.0, 1.0, 1.0]),
        &ShaderPreset::Vignette {
            radius: 0.5,
            softness: 0.3,
            strength: 0.8,
        },
        0.0,
        0.0,
    )
    .unwrap();
    assert!(vig.get_pixel(16, 16).unwrap()[0] > vig.get_pixel(0, 0).unwrap()[0] + 0.3);

    // ThermalVision produces distinct warm color on bright pixel vs cool color on dark pixel.
    let therm = apply_shader_preset(&src, &presets[10], 0.0, 0.0).unwrap();
    let cold = therm.get_pixel(2, 2).unwrap();
    let hot = therm.get_pixel(28, 28).unwrap();
    assert!(cold[2] > cold[0], "cold thermal pixel should be blue-shifted");
    assert!(hot[0] > hot[2], "hot thermal pixel should be warm red/yellow");
}

// =========================================================================
// 3. STREAMFX 3D (StreamFX)
// =========================================================================

#[test]
fn test_streamfx_exact_4pt_homography_and_corner_pin_warp() {
    let unit = CornerPinQuad::unit();
    let quad = CornerPinQuad {
        top_left: [0.2, 0.15],
        top_right: [0.85, 0.25],
        bottom_right: [0.95, 0.9],
        bottom_left: [0.1, 0.85],
    };
    let h = solve_homography_4pt(unit.corners(), quad.corners()).unwrap();
    for (s, d) in unit.corners().iter().zip(quad.corners().iter()) {
        let p = h.project_point(s[0] as f64, s[1] as f64).unwrap();
        assert!((p[0] as f32 - d[0]).abs() < 1e-5);
        assert!((p[1] as f32 - d[1]).abs() < 1e-5);
    }

    let src = solid(24, 24, [0.9, 0.4, 0.1, 1.0]);
    let warped = warp_corner_pin_normalized(&src, &quad, 40, 40).unwrap();
    assert!(warped.get_pixel(20, 20).unwrap()[3] > 0.95);
    assert!(warped.get_pixel(0, 0).unwrap()[3] < 0.01);
}

#[test]
fn test_streamfx_singular_homography_and_behind_camera_errors() {
    let unit = CornerPinQuad::unit();
    let degenerate = [[0.0, 0.0], [0.5, 0.5], [1.0, 1.0], [0.25, 0.25]];
    assert_eq!(
        solve_homography_4pt(unit.corners(), degenerate),
        Err(StreamFxError::SingularHomography)
    );

    let behind = PitchYawRollFov {
        z_distance: -20.0,
        ..Default::default()
    };
    assert_eq!(behind.project_quad(16.0 / 9.0), Err(StreamFxError::BehindCamera));
}

#[test]
fn test_streamfx_3d_perspective_pitch_yaw_roll_fov() {
    let id_cam = PitchYawRollFov::default();
    let q_id = id_cam.project_quad(1.0).unwrap();
    assert!((q_id.top_left[0] - 0.0).abs() < 1e-5);
    assert!((q_id.bottom_right[1] - 1.0).abs() < 1e-5);

    let turned = PitchYawRollFov {
        pitch_deg: 10.0,
        yaw_deg: 30.0,
        roll_deg: 5.0,
        fov_deg: 60.0,
        z_distance: 0.2,
        center: [0.5, 0.5],
    };
    let src = solid(32, 32, [0.2, 0.8, 0.9, 1.0]);
    let out = apply_perspective_3d(&src, &turned, 32, 32).unwrap();
    assert!(out.get_pixel(16, 16).unwrap()[3] > 0.9);
}

#[test]
fn test_streamfx_dual_kawase_blur_and_dynamic_mask_feather() {
    let mut impulse = solid(32, 32, [0.0, 0.0, 0.0, 1.0]);
    for y in 12..20 {
        for x in 12..20 {
            impulse.set_pixel(x, y, [1.0, 1.0, 1.0, 1.0]);
        }
    }
    let blurred = dual_kawase_blur(
        &impulse,
        DualKawaseBlur {
            passes: 3,
            offset: 1.5,
        },
    )
    .unwrap();
    // Energy spreads cleanly beyond the original 12..20 box.
    assert!(blurred.get_pixel(9, 16).unwrap()[0] > 0.02);
    assert!(blurred.get_pixel(16, 16).unwrap()[0] > blurred.get_pixel(9, 16).unwrap()[0]);

    let mut mask = vec![0.0_f32; 32 * 32];
    for y in 10..22 {
        for x in 10..22 {
            mask[y * 32 + x] = 1.0;
        }
    }
    let feathered = apply_dynamic_mask_feather(
        &solid(32, 32, [1.0, 0.5, 0.2, 1.0]),
        &mask,
        DynamicMaskFeather {
            feather_radius_px: 8.0,
            expand_px: 0.0,
            falloff_gamma: 1.0,
            invert: false,
        },
    )
    .unwrap();
    let a_center = feathered.get_pixel(16, 16).unwrap()[3];
    let a_edge = feathered.get_pixel(10, 16).unwrap()[3];
    let a_far = feathered.get_pixel(0, 0).unwrap()[3];
    assert!(a_center > 0.95);
    assert!(a_edge > 0.1 && a_edge < 0.9, "edge alpha should feather smoothly: {a_edge}");
    assert!(a_far < 0.05);
}

// =========================================================================
// 4. DOWNSTREAM KEYER (downstream-keyer)
// =========================================================================

#[test]
fn test_dsk_pins_above_main_transition_with_fade_and_slide() {
    let mut bank = DownstreamKeyerBank::new();
    let bug_id = 1;
    let ticker_id = 2;

    bank.add_channel(
        DskChannel::new(bug_id, "StationBug", StudioId(10))
            .with_transition(DskTransition::Fade { duration_ms: 200 })
            .with_z_order(1),
    );
    bank.add_channel(
        DskChannel::new(ticker_id, "NewsTicker", StudioId(11))
            .with_transition(DskTransition::Slide {
                direction: WipeDirection::Left,
                duration_ms: 400,
            })
            .with_z_order(2),
    );

    bank.set_active(bug_id, true);
    bank.set_active(ticker_id, true);
    bank.tick(200.0); // bug fully on (1.0), ticker halfway slid in (0.5).

    assert!((bank.channel(bug_id).unwrap().visibility - 1.0).abs() < 1e-5);
    assert!((bank.channel(ticker_id).unwrap().visibility - 0.5).abs() < 1e-5);

    // Main transition frame halfway between dark blue & dark green.
    let main_trans = render_transition_frame(
        &solid(32, 32, [0.0, 0.0, 0.5, 1.0]),
        &solid(32, 32, [0.0, 0.5, 0.0, 1.0]),
        &TransitionKind::Fade,
        0.5,
        None,
        None,
    );

    let mut bug_ov = solid(32, 32, [0.0, 0.0, 0.0, 0.0]);
    bug_ov.set_pixel(2, 2, [1.0, 1.0, 0.0, 1.0]);
    let mut ticker_ov = solid(32, 32, [0.0, 0.0, 0.0, 0.0]);
    for x in 0..32 {
        ticker_ov.set_pixel(x, 30, [1.0, 0.0, 0.0, 1.0]);
    }

    let mut frames = BTreeMap::new();
    frames.insert(bug_id, bug_ov);
    frames.insert(ticker_id, ticker_ov);

    let final_frame = bank.composite_overlays(&main_trans, &frames);
    // Bug pixel at (2, 2) sits cleanly on top of transition frame.
    let p_bug = final_frame.get_pixel(2, 2).unwrap();
    assert!((p_bug[0] - 1.0).abs() < 1e-4 && (p_bug[1] - 1.0).abs() < 1e-4);
    // Ticker at visibility 0.5 with Left slide is shifted right by +16px, so x=20 is red and x=4 is background.
    assert!(final_frame.get_pixel(20, 30).unwrap()[0] > 0.9);
    assert!(final_frame.get_pixel(4, 30).unwrap()[0] < 0.1);
}

#[test]
fn test_dsk_tie_to_transition_and_exclusive_group_radio() {
    let mut bank = DownstreamKeyerBank::new();
    bank.add_channel(
        DskChannel::new(1, "LowerThirdA", StudioId(1))
            .with_exclusive_group("lower_thirds")
            .with_tie_to_transition(true),
    );
    bank.add_channel(
        DskChannel::new(2, "LowerThirdB", StudioId(2)).with_exclusive_group("lower_thirds"),
    );

    // Arm tied channel 1; it must stay off until main transition triggers.
    assert!(bank.arm_tied(1, true));
    assert!(!bank.channel(1).unwrap().active);

    bank.set_active(2, true);
    assert!(bank.channel(2).unwrap().active);

    // Main TAKE triggers tied channel 1, which also radio-deactivates channel 2 in same exclusive_group!
    let fired = bank.on_main_transition_triggered();
    assert_eq!(fired, vec![(1, true)]);
    assert!(bank.channel(1).unwrap().active);
    assert!(!bank.channel(2).unwrap().active);
}

#[test]
fn test_dsk_auto_hide_timer() {
    let mut bank = DownstreamKeyerBank::new();
    bank.add_channel(
        DskChannel::new(9, "FollowAlert", StudioId(99)).with_auto_hide_ms(500),
    );
    bank.set_active(9, true);
    assert!(bank.channel(9).unwrap().active);

    bank.tick(300.0);
    assert!(bank.channel(9).unwrap().active);
    bank.tick(250.0);
    assert!(!bank.channel(9).unwrap().active);
    assert!(!bank.channel(9).unwrap().is_visible_on_air());
}

// =========================================================================
// 5. ADVANCED SCENE SWITCHER (adv_switcher)
// =========================================================================

#[test]
fn test_adv_switcher_priority_composite_conditions_and_core_execution() {
    let mut col = SceneCollection::new("Main", 1920, 1080);
    let cam = col
        .add_source("Cam", SourceKind::ColorSolid { rgba: [1.0, 1.0, 1.0, 1.0] })
        .unwrap();
    let s_idle = col.add_studio("BRB");
    let s_game = col.add_studio("Game");
    let item_id = col.add_item(s_game, cam, "Cam").unwrap();

    let mut studio_mode = StudioMode::new(s_game, s_idle);
    let mut mixer = AudioMixer::new(48_000, "default");
    mixer.add_strip(AudioStrip::new(StripId(1), "Mic", ChannelLayout::Stereo, 48_000));
    let mut dsk = DownstreamKeyerBank::new();
    dsk.add_channel(DskChannel::new(5, "Alert", s_game));

    let mut engine = MacroEngine::new();
    engine.add_rule(MacroRule::new(
        10,
        "SwitchToGameWhenFocusedAndLive",
        100,
        Condition::CompositeAnd(vec![
            Condition::FocusedSilkWindow {
                frame_id: Some(77),
                title_contains: Some(String::from("Elden")),
            },
            Condition::StreamState {
                expected: StreamActiveState::Live,
            },
        ]),
        vec![
            MacroAction::SwitchStudio {
                studio_id: s_game,
                preview_only: false,
            },
            MacroAction::SetSourceVisible {
                studio_id: s_game,
                item_id,
                visible: false,
            },
            MacroAction::SetStripMute {
                strip_id: StripId(1),
                muted: true,
            },
            MacroAction::SetDskActive {
                channel_id: 5,
                active: true,
            },
            MacroAction::SaveReplay { seconds: 30.0 },
        ],
    ));
    engine.add_rule(MacroRule::new(
        20,
        "MediaEndOrIdle",
        50,
        Condition::CompositeOr(vec![
            Condition::MediaRemainingMs {
                source_id: cam,
                threshold_ms: 1500,
            },
            Condition::UserIdleMs { min_idle_ms: 60_000 },
        ]),
        vec![MacroAction::StartStopRecord { start: false }],
    ));

    let mut media = BTreeMap::new();
    media.insert(
        cam,
        MediaPlaybackSnapshot {
            source_id: cam,
            remaining_ms: 1200,
            playing: true,
        },
    );

    let world = MacroWorldState {
        now_ms: 1000,
        focused_window_frame_id: Some(77),
        focused_window_title: String::from("Elden Ring"),
        strip_levels_db: BTreeMap::new(),
        media,
        user_idle_ms: 0,
        stream_state: StreamActiveState::Live,
        ..Default::default()
    };

    let events = engine.evaluate(&world);
    assert_eq!(events.len(), 2);
    // Priority 100 evaluates before priority 50.
    assert_eq!(events[0].rule_id, 10);
    assert_eq!(events[1].rule_id, 20);

    let cmds = execute_macro_actions(&events, &mut studio_mode, &mut col, &mut mixer, &mut dsk);
    studio_mode.tick(300.0);
    assert_eq!(studio_mode.program_studio, s_game);
    assert!(!col.studio(s_game).unwrap().items[0].visible);
    assert!(mixer.strip(StripId(1)).unwrap().muted);
    assert!(dsk.channel(5).unwrap().active);
    assert_eq!(
        cmds,
        vec![
            MacroOutputCommand::SaveReplay { seconds: 30.0 },
            MacroOutputCommand::StopRecord,
        ]
    );
}

#[test]
fn test_adv_switcher_audio_hysteresis_hold_and_cooldown_anti_flap() {
    let mut engine = MacroEngine::new();
    engine.add_rule(
        MacroRule::new(
            1,
            "MicDuck",
            10,
            Condition::AudioThresholdDb {
                strip_id: StripId(1),
                threshold_db: -20.0,
                above: true,
                hysteresis_db: 4.0, // stays active until < -24 dB
            },
            vec![MacroAction::TriggerTake],
        )
        .with_hold_ms(100)
        .with_cooldown_ms(500)
        .with_edge_triggered(true),
    );

    let mut world = MacroWorldState::default();
    world.strip_levels_db.insert(StripId(1), -18.0);
    world.now_ms = 0;
    assert!(engine.evaluate(&world).is_empty(), "must wait for hold_ms");

    // Transient drop at 50ms resets hold timer.
    world.now_ms = 50;
    world.strip_levels_db.insert(StripId(1), -30.0);
    assert!(engine.evaluate(&world).is_empty());

    // Sustained signal starting at 100ms fires at 200ms.
    world.now_ms = 100;
    world.strip_levels_db.insert(StripId(1), -18.0);
    assert!(engine.evaluate(&world).is_empty());
    world.now_ms = 200;
    assert_eq!(engine.evaluate(&world).len(), 1);

    // Dipping to -22 dB (inside hysteresis band -24..-20) does NOT release latch,
    // and bouncing back to -18 dB does NOT re-trigger!
    world.now_ms = 300;
    world.strip_levels_db.insert(StripId(1), -22.0);
    assert!(engine.evaluate(&world).is_empty());
    world.now_ms = 350;
    world.strip_levels_db.insert(StripId(1), -18.0);
    assert!(engine.evaluate(&world).is_empty());

    // Drop below -24 dB releases latch, then re-cross inside 500ms cooldown is blocked.
    world.now_ms = 400;
    world.strip_levels_db.insert(StripId(1), -26.0);
    assert!(engine.evaluate(&world).is_empty());
    world.now_ms = 450;
    world.strip_levels_db.insert(StripId(1), -15.0);
    assert!(engine.evaluate(&world).is_empty());
    world.now_ms = 560; // held >= 100ms, still within 500ms of t=200 (elapsed 360ms)
    assert!(engine.evaluate(&world).is_empty());
    world.now_ms = 710; // >= 500ms cooldown cleared
    assert_eq!(engine.evaluate(&world).len(), 1);
}

#[test]
fn test_adv_switcher_timer_interval_deterministic() {
    let mut engine = MacroEngine::new();
    engine.add_rule(
        MacroRule::new(
            7,
            "Heartbeat",
            1,
            Condition::TimerIntervalMs { interval_ms: 1000 },
            vec![MacroAction::TriggerTake],
        )
        .with_edge_triggered(false),
    );
    let mut fired_at = Vec::new();
    for t in [0_u64, 500, 1000, 1500, 2000, 3000] {
        let evs = engine.evaluate(&MacroWorldState {
            now_ms: t,
            ..Default::default()
        });
        if !evs.is_empty() {
            fired_at.push(t);
        }
    }
    assert_eq!(fired_at, vec![1000, 2000, 3000]);
}

// =========================================================================
// 6. SOURCE RECORD + FREEZE FILTER
// =========================================================================

#[test]
fn test_source_record_tap_isolated_pre_composite_and_replay_save() {
    let mut tap = SourceRecordTap::new(
        cabaret_core::scene::SourceId(42),
        ContainerFormat::HybridMp4,
        AudioTrackMask::ALL_6,
        10,
        1024 * 1024,
    );
    tap.start();

    let clean_a = solid(8, 8, [0.9, 0.1, 0.2, 1.0]);
    let clean_b = solid(8, 8, [0.1, 0.9, 0.2, 1.0]);
    let step1 = tap.tap_video_frame(0, &clean_a, true, 10_000_000_000);
    assert!(matches!(step1, RecordingStep::Written { .. }));
    let _ = tap.tap_audio_block(500_000, 0, &[[0.25, -0.25]; 128], 10_000_000_000);
    let _ = tap.tap_video_frame(1_000_000, &clean_b, true, 10_000_000_000);

    let replay = tap.save_replay(0.5).unwrap();
    assert!(!replay.is_empty());
    assert_eq!(replay[0].kind, MediaKind::Video);
    assert!(replay[0].is_keyframe);
    let recovered = decode_tapped_video_packet(&replay[0]).unwrap();
    assert!((recovered.get_pixel(0, 0).unwrap()[0] - 0.9).abs() < 0.02);

    tap.stop();
    assert!(tap.recording.hybrid_mp4_moov_finalized);
}

#[test]
fn test_freeze_filter_instant_timed_and_strobe() {
    let red = solid(4, 4, [1.0, 0.0, 0.0, 1.0]);
    let green = solid(4, 4, [0.0, 1.0, 0.0, 1.0]);
    let blue = solid(4, 4, [0.0, 0.0, 1.0, 1.0]);

    let mut ff = FreezeFilter::new();
    ff.trigger_instant();
    let f0 = ff.process(&red, 16.0);
    let f1 = ff.process(&green, 16.0);
    assert!((f0.get_pixel(0, 0).unwrap()[0] - 1.0).abs() < 1e-5);
    assert!((f1.get_pixel(0, 0).unwrap()[0] - 1.0).abs() < 1e-5);
    ff.unfreeze();
    assert!((ff.process(&blue, 16.0).get_pixel(0, 0).unwrap()[2] - 1.0).abs() < 1e-5);

    ff.trigger_timed(100.0);
    assert!((ff.process(&red, 0.0).get_pixel(0, 0).unwrap()[0] - 1.0).abs() < 1e-5);
    assert!((ff.process(&green, 50.0).get_pixel(0, 0).unwrap()[0] - 1.0).abs() < 1e-5);
    assert!((ff.process(&green, 60.0).get_pixel(0, 0).unwrap()[1] - 1.0).abs() < 1e-5);

    ff.set_strobe(100.0, 100.0);
    assert!((ff.process(&red, 50.0).get_pixel(0, 0).unwrap()[0] - 1.0).abs() < 1e-5);
    assert!((ff.process(&green, 60.0).get_pixel(0, 0).unwrap()[0] - 1.0).abs() < 1e-5); // still < 100ms when sampled
    assert!((ff.process(&blue, 50.0).get_pixel(0, 0).unwrap()[2] - 1.0).abs() < 1e-5); // phase >= 100 -> live
}

// =========================================================================
// 7. TIME WARP SCAN + DYNAMIC DELAY
// =========================================================================

#[test]
fn test_time_warp_scan_slit_freeze_and_scan_bar() {
    let mut scan = TimeWarpScan::new(TimeWarpConfig {
        direction: ScanDirection::LeftToRight,
        duration_ms: 1000.0,
        show_scan_bar: true,
        scan_bar_thickness_px: 2.0,
        scan_bar_color: [1.0, 1.0, 1.0, 1.0],
        loop_scan: false,
    });

    let red = solid(20, 4, [1.0, 0.0, 0.0, 1.0]);
    let blue = solid(20, 4, [0.0, 0.0, 1.0, 1.0]);

    // First half (progress -> 0.5) freezes left 10 columns as red.
    let _ = scan.process(&red, 500.0);
    // Second step (progress -> 0.75) with blue input: columns 0..10 stay red, 10..15 turn blue,
    // column 15 shows white scan bar, 16..20 stay live blue.
    let out = scan.process(&blue, 250.0);
    assert!(out.get_pixel(2, 2).unwrap()[0] > 0.9);
    assert!(out.get_pixel(12, 2).unwrap()[2] > 0.9);
    let bar_px = out.get_pixel(15, 2).unwrap();
    assert!(bar_px[0] > 0.4 && bar_px[1] > 0.4 && bar_px[2] > 0.4);

    // Radial outward completes cleanly at duration_ms.
    let mut rad = TimeWarpScan::new(TimeWarpConfig {
        direction: ScanDirection::RadialOutward,
        duration_ms: 100.0,
        show_scan_bar: false,
        ..Default::default()
    });
    let r_out = rad.process(&red, 100.0);
    assert!(rad.finished);
    assert!((r_out.get_pixel(0, 0).unwrap()[0] - 1.0).abs() < 1e-5);
}

#[test]
fn test_dynamic_delay_buffer_slow_mo_reverse_instant_replay_and_scrub() {
    let mut delay = DynamicDelayBuffer::new(16);
    for i in 0..10 {
        delay.push_frame(i * 16_667, solid(2, 2, [i as f32 / 10.0, 0.0, 0.0, 1.0]));
    }
    // Live mode yields frame 9.
    assert!((delay.step_and_sample(1.0).unwrap().get_pixel(0, 0).unwrap()[0] - 0.9).abs() < 1e-4);

    // Instant replay rewind 6 frames (seq 3) at 0.5x slow-motion.
    delay.trigger_instant_replay(6, 0.5);
    let s1 = delay.step_and_sample(2.0).unwrap().get_pixel(0, 0).unwrap()[0]; // seq 3 + 1.0 = 4
    assert!((s1 - 0.4).abs() < 1e-4);

    // Reverse at -1.0x
    delay.set_speed(-1.0);
    let s2 = delay.step_and_sample(2.0).unwrap().get_pixel(0, 0).unwrap()[0]; // seq 4 - 2.0 = 2
    assert!((s2 - 0.2).abs() < 1e-4);

    // Scrub to oldest (0.0) and snap back live.
    delay.scrub_to_normalized(0.0);
    assert!((delay.step_and_sample(0.0).unwrap().get_pixel(0, 0).unwrap()[0] - 0.0).abs() < 1e-4);
    delay.go_live();
    assert!((delay.step_and_sample(1.0).unwrap().get_pixel(0, 0).unwrap()[0] - 0.9).abs() < 1e-4);
}

// =========================================================================
// 8. SPECTRALIZER + GONIOMETER
// =========================================================================

#[test]
fn test_spectralizer_fft_bins_gravity_peak_caps_and_stereo_mirror() {
    let mut analyzer = SpectrumAnalyzer::new(SpectrumConfig {
        sample_rate_hz: 48_000,
        fft_size: 1024,
        num_bins: 24,
        smoothing: 0.0,
        gravity: 2.0,
        peak_hold_ms: 100.0,
        ..Default::default()
    });

    // Left = 1000 Hz sine, Right = 4000 Hz sine.
    let samples: Vec<[f32; 2]> = (0..1024)
        .map(|i| {
            let t = i as f32 / 48_000.0;
            let l = 0.8 * (2.0 * core::f32::consts::PI * 1000.0 * t).sin();
            let r = 0.7 * (2.0 * core::f32::consts::PI * 4000.0 * t).sin();
            [l, r]
        })
        .collect();

    analyzer.analyze(&samples, 16.0);
    let peak_bin_l = analyzer
        .levels_left
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap()
        .0;
    let peak_bin_r = analyzer
        .levels_right
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap()
        .0;
    let freq_l = analyzer.bin_center_freq_hz(peak_bin_l);
    let freq_r = analyzer.bin_center_freq_hz(peak_bin_r);
    assert!((freq_l - 1000.0).abs() < 250.0, "left peak freq was {freq_l}");
    assert!((freq_r - 4000.0).abs() < 800.0, "right peak freq was {freq_r}");

    let peak_before = analyzer.peak_caps[peak_bin_l];
    // Feed silence past hold window -> gravity pulls peak cap down.
    let silence = vec![[0.0, 0.0]; 1024];
    analyzer.analyze(&silence, 150.0);
    analyzer.analyze(&silence, 200.0);
    assert!(analyzer.peak_caps[peak_bin_l] < peak_before);

    let mirror_fb = render_bar_spectrum(
        &analyzer,
        &BarSpectrumConfig {
            width: 240,
            height: 120,
            stereo_mirror: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(mirror_fb.width, 240);
}

#[test]
fn test_spectralizer_circular_spectrum_and_goniometer_mono_vs_antiphase() {
    let mut analyzer = SpectrumAnalyzer::new(SpectrumConfig {
        fft_size: 256,
        num_bins: 16,
        smoothing: 0.0,
        ..Default::default()
    });
    let tone: Vec<[f32; 2]> = (0..256)
        .map(|i| {
            let v = (2.0 * core::f32::consts::PI * 2000.0 * (i as f32 / 48_000.0)).sin();
            [v, v]
        })
        .collect();
    analyzer.analyze(&tone, 16.0);

    let circ = render_circular_spectrum(&analyzer, &CircularSpectrumConfig::default()).unwrap();
    // Center pixel inside inner_radius should remain background.
    assert!(circ.get_pixel(160, 160).unwrap()[0] < 0.05);

    // Mono signal: correlation +1, goniometer trace strictly vertical along x = 32.
    let mono: Vec<[f32; 2]> = (0..64)
        .map(|i| {
            let s = ((i as f32) * 0.2).sin() * 0.8;
            [s, s]
        })
        .collect();
    assert!((compute_stereo_correlation(&mono) - 1.0).abs() < 1e-5);
    let g_mono = render_goniometer(
        &mono,
        &GoniometerConfig {
            width: 64,
            height: 64,
            show_reticle: false,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert!(g_mono.get_pixel(32, 20).unwrap()[1] > 0.5);
    assert!(g_mono.get_pixel(10, 32).unwrap()[1] < 0.1);

    // Anti-phase signal: correlation -1, goniometer trace strictly horizontal along y = 32.
    let anti: Vec<[f32; 2]> = mono.iter().map(|&[l, _]| [l, -l]).collect();
    assert!((compute_stereo_correlation(&anti) + 1.0).abs() < 1e-5);
    let g_anti = render_goniometer(
        &anti,
        &GoniometerConfig {
            width: 64,
            height: 64,
            show_reticle: false,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert!(g_anti.get_pixel(20, 32).unwrap()[1] > 0.5);
    assert!(g_anti.get_pixel(32, 10).unwrap()[1] < 0.1);
}

// =========================================================================
// 9. LOCAL CAPTIONS (obs-localvocal / CEA-608 / CEA-708 / SRT / VTT)
// =========================================================================

#[test]
fn test_local_captions_rolling_buffer_srt_and_vtt_export() {
    let mut buf = CaptionBuffer::new(2, 24, 100);
    buf.push_transcript_segment(
        TranscriptSegment::new(1, 1250, 3500, "interim text").with_final(false),
    );
    assert_eq!(buf.current_lines(), &["interim text"]);

    buf.push_transcript_segment(
        TranscriptSegment::new(1, 1250, 3500, "Welcome to Cabaret live!")
            .with_speaker("Host")
            .with_final(true),
    );
    assert_eq!(buf.segments().len(), 1);
    assert_eq!(buf.active_segments_at(2000).len(), 1);

    let srt = buf.export_srt();
    assert!(srt.contains("1\n00:00:01,250 --> 00:00:03,500\nHost: Welcome to Cabaret\nlive!\n"));

    let vtt = buf.export_vtt();
    assert!(vtt.starts_with("WEBVTT\n\n00:00:01.250 --> 00:00:03.500\n"));
}

#[test]
fn test_local_captions_cea608_odd_parity_and_cea708_dtvcc() {
    let pairs = encode_cea608_pairs("Hello Cabaret!", Cea608Mode::PopOn);
    assert!(pairs.iter().all(|p| has_odd_parity(p.0[0]) && has_odd_parity(p.0[1])));
    assert_eq!(decode_cea608_pairs(&pairs).as_deref(), Some("Hello Cabaret!"));

    let roll = encode_cea608_pairs("Live", Cea608Mode::RollUp2);
    assert!(roll.iter().all(|p| p.all_odd_parity()));
    assert_eq!(decode_cea608_pairs(&roll).as_deref(), Some("Live"));

    let pkt708 = packetize_cea708("Hello Cabaret!", 1, 2);
    assert_eq!(pkt708.cc_triplets[0][0], 0xFF); // DTVCC Packet Start + cc_valid
    assert!(pkt708.cc_triplets.len() >= 2);
    assert_eq!(pkt708.cc_triplets[1][0], 0xFE); // DTVCC Packet Data + cc_valid
    assert_eq!(
        depacketize_cea708(&pkt708.cc_triplets).as_deref(),
        Some("Hello Cabaret!")
    );
}

#[test]
fn test_local_captions_lower_third_burn_in_rasterizer() {
    let mut fb = solid(320, 180, [0.05, 0.05, 0.05, 1.0]);
    render_lower_third_captions(
        &mut fb,
        &["CLOSED CAPTION", "TEST LINE"],
        &CaptionBurnInStyle::default(),
    );
    // Lower region should contain bright rendered glyphs & accent bar.
    let mut bright_pixels = 0usize;
    for y in 100..170 {
        for x in 20..300 {
            let p = fb.get_pixel(x, y).unwrap();
            if p[0] > 0.7 || p[2] > 0.7 {
                bright_pixels += 1;
            }
        }
    }
    assert!(bright_pixels > 50, "expected burned-in caption glyphs");
}

// =========================================================================
// 10. STUDIO LINK (CabaretLink / DistroAV + Teleport)
// =========================================================================

#[test]
fn test_studio_link_mdns_service_announcement_roundtrip() {
    let ann = ServiceAnnouncement {
        instance_name: String::from("Stage-Main-Cam"),
        host_name: String::from("studio-box.local"),
        ipv4: [192, 168, 1, 44],
        port: 5962,
        groups: vec![String::from("Public"), String::from("Cameras")],
        width: 1920,
        height: 1080,
        fps_num: 60000,
        fps_den: 1001,
        sample_rate_hz: 48_000,
        channels: 2,
        ttl_sec: 90,
    };
    let raw = ann.encode_mdns_payload();
    let decoded = ServiceAnnouncement::decode_mdns_payload(&raw).unwrap();
    assert_eq!(decoded, ann);
}

#[test]
fn test_studio_link_rle_rgba8_and_yuv422_roundtrip() {
    let mut fb = FrameBuffer::new(16, 8).unwrap();
    for y in 0..8 {
        for x in 0..16 {
            let col = if x < 8 {
                [0.9, 0.1, 0.1, 1.0]
            } else {
                [0.1, 0.8, 0.2, 1.0]
            };
            fb.set_pixel(x, y, col);
        }
    }

    let enc_rgba = LinkVideoFrame::encode(&fb, LinkPixelFormat::RleRgba8).unwrap();
    assert!(enc_rgba.rle_data.len() < 128, "flat runs should compress tightly");
    let dec_rgba = enc_rgba.decode().unwrap();
    for (a, b) in fb.to_srgb_rgba8().iter().zip(dec_rgba.to_srgb_rgba8().iter()) {
        assert_eq!(a, b);
    }

    let enc_yuv = LinkVideoFrame::encode(&fb, LinkPixelFormat::RleYuv422).unwrap();
    let dec_yuv = enc_yuv.decode().unwrap();
    assert!((dec_yuv.get_pixel(2, 2).unwrap()[0] - 0.9).abs() < 0.05);
    assert!((dec_yuv.get_pixel(12, 2).unwrap()[1] - 0.8).abs() < 0.05);
}

#[test]
fn test_studio_link_packet_framer_crc_tally_audio_and_streaming_deframer() {
    let audio_pkt = LinkPacket {
        seq: 1,
        timestamp_us: 100_000,
        payload: LinkPayload::Audio(LinkAudioFrame::from_stereo_f32(
            48_000,
            &[[0.5, -0.5], [0.25, -0.25]],
        )),
    };
    let tally_pkt = LinkPacket {
        seq: 2,
        timestamp_us: 116_667,
        payload: LinkPayload::Tally(LinkTally::from_tally_state(
            TallyState::ProgramRed,
            "Cam1",
        )),
    };

    let mut wire = Vec::new();
    wire.extend_from_slice(b"NOISE"); // test magic resync
    let b1 = audio_pkt.encode();
    let b2 = tally_pkt.encode();
    wire.extend_from_slice(&b1);
    wire.extend_from_slice(&b2);

    // Feed arbitrary chunks into LinkStreamDecoder.
    let mut dec = LinkStreamDecoder::new();
    let mut out_pkts = Vec::new();
    for chunk in wire.chunks(7) {
        dec.push_bytes(chunk);
        while let Some(pkt) = dec.next_packet().unwrap() {
            out_pkts.push(pkt);
        }
    }
    assert_eq!(out_pkts.len(), 2);
    assert_eq!(out_pkts[0], audio_pkt);
    assert_eq!(out_pkts[1], tally_pkt);
    if let LinkPayload::Tally(ref t) = out_pkts[1].payload {
        assert_eq!(t.to_tally_state(), TallyState::ProgramRed);
    } else {
        panic!("expected tally packet");
    }

    // Flip 1 bit -> CRC mismatch error.
    let mut corrupted = b2.clone();
    corrupted[10] ^= 0x01;
    assert!(matches!(
        LinkPacket::decode(&corrupted),
        Err(LinkError::ChecksumMismatch { .. })
    ));
}

// =========================================================================
// 11. OVERLAYS (Tuna NowPlayingCard + InputOverlayHud)
// =========================================================================

#[test]
fn test_overlays_now_playing_card_bell_last_player_and_progress() {
    // Construct genuine Bell OP_MIXER Stream word: tag=0x4D, pd=0x42, playing=true, muted=false, vol=75.
    let bell_word: u64 = (0x4D_u64 << 56) | (0x42_u64 << 32) | (1 << 9) | 75;
    let parsed = BellLastPlayer::from_bell_word(bell_word).unwrap();
    assert_eq!(parsed.pd, 0x42);
    assert!(parsed.playing);
    assert_eq!(parsed.volume_pct, 75);

    let mut card = NowPlayingCard::new(
        NowPlayingTrack {
            title: String::from("Midnight Synth"),
            artist: String::from("Cabaret Band"),
            duration_ms: 200_000,
            position_ms: 50_000,
            ..Default::default()
        },
        NowPlayingStyle::default(),
    );
    assert!(card.apply_bell_last_player(bell_word, Some("Qupid")));
    card.advance_time(50_000);
    assert!((card.progress_ratio() - 0.5).abs() < 1e-5);

    let fb = card.render().unwrap();
    assert_eq!(fb.width, 440);
    assert_eq!(fb.height, 112);
    // Progress bar mid-section has accent color vs background at far right end.
    assert!(fb.get_pixel(150, 92).unwrap()[0] > 0.5);
    assert!(fb.get_pixel(410, 92).unwrap()[0] < 0.3);
}

#[test]
fn test_overlays_input_overlay_hud_wasd_and_mouse_vector() {
    let mut hud = InputOverlayHud::new(InputOverlayStyle::default());
    hud.update(
        InputStateSnapshot {
            w: true,
            shift: true,
            mouse_left: true,
            mouse_dx: 36.0,
            mouse_dy: -12.0,
            scroll_delta: 1.0,
            ..Default::default()
        },
        40.0,
    );
    let fb = hud.render().unwrap();
    assert_eq!(fb.width, 360);
    assert_eq!(fb.height, 150);
    // 'W' keycap at (16 + 34 + 10, 14 + 10) is lit active blue/cyan vs 'A' keycap idle dark.
    assert!(fb.get_pixel(55, 22).unwrap()[2] > 0.8);
    assert!(fb.get_pixel(22, 55).unwrap()[2] < 0.3);
}

// =========================================================================
// 12. MULTI-CANVAS (aitum-vertical + aitum-multistream)
// =========================================================================

#[test]
fn test_multi_canvas_linked_vertical_9x16_and_square_1x1_shared_sources() {
    let mut col = SceneCollection::new("Show", 1920, 1080);
    let cam = col
        .add_source("Facecam", SourceKind::ColorSolid { rgba: [0.9, 0.3, 0.1, 1.0] })
        .unwrap();
    let game = col
        .add_source("Gameplay", SourceKind::ColorSolid { rgba: [0.1, 0.2, 0.8, 1.0] })
        .unwrap();
    let studio_live = col.add_studio("Live");

    let mut mgr = SecondaryCanvasManager::new();
    let vert_id = mgr.create_canvas("TikTokVertical", CanvasPreset::Vertical9x16, 30, 10 * 1024 * 1024);
    let sq_id = mgr.create_canvas("InstaSquare", CanvasPreset::Square1x1, 15, 5 * 1024 * 1024);

    // Independent per-canvas Transform2D layout sharing live SourceIds.
    let mut cam_vert = Transform2D::default();
    cam_vert.bounds_mode = BoundsMode::Stretch;
    cam_vert.bounds = [1080.0, 960.0];
    let mut game_vert = Transform2D::default();
    game_vert.pos = [0.0, 960.0];
    game_vert.bounds_mode = BoundsMode::Stretch;
    game_vert.bounds = [1080.0, 960.0];
    mgr.canvas_mut(vert_id).unwrap().set_studio_layout(
        studio_live,
        vec![
            SecondaryCanvasItem::new(cam, "TopCam", cam_vert),
            SecondaryCanvasItem::new(game, "BotGame", game_vert),
        ],
    );

    let mut sq_t = Transform2D::default();
    sq_t.bounds_mode = BoundsMode::Stretch;
    sq_t.bounds = [1080.0, 1080.0];
    mgr.canvas_mut(sq_id)
        .unwrap()
        .add_default_item(SecondaryCanvasItem::new(game, "FullGame", sq_t));

    // Add independent StreamDest targets & test backtrack replay buffer.
    mgr.canvas_mut(vert_id).unwrap().add_stream_dest(
        DestId(1),
        "TikTok",
        ServicePlatform::Custom,
        StreamDest::Rtmp {
            url: String::from("rtmp://tiktok.invalid/live"),
            secret_slot: ShibariSecretSlot::new(1, "tiktok"),
        },
        VideoEncodeProfile {
            width: 1080,
            height: 1920,
            ..Default::default()
        },
        AudioEncodeProfile::default(),
    );
    mgr.canvas_mut(sq_id).unwrap().add_stream_dest(
        DestId(2),
        "Twitch1x1",
        ServicePlatform::Twitch,
        StreamDest::NdiLink {
            stream_name: String::from("SquareFeed"),
            groups: String::from("Public"),
        },
        VideoEncodeProfile::default(),
        AudioEncodeProfile::default(),
    );

    mgr.on_main_studio_switched(studio_live);
    let mut fctx = BTreeMap::new();
    let rendered = mgr
        .render_all(&col, &BTreeMap::new(), &mut fctx, &BTreeMap::new())
        .unwrap();

    let v_fb = &rendered[&vert_id];
    assert_eq!((v_fb.width, v_fb.height), (1080, 1920));
    // Top half is Facecam (red), bottom half is Gameplay (blue).
    assert!(v_fb.get_pixel(540, 200).unwrap()[0] > 0.8);
    assert!(v_fb.get_pixel(540, 1500).unwrap()[2] > 0.7);

    let s_fb = &rendered[&sq_id];
    assert_eq!((s_fb.width, s_fb.height), (1080, 1080));
    assert!(s_fb.get_pixel(540, 540).unwrap()[2] > 0.7);

    // Push packets to vertical backtrack replay buffer.
    mgr.canvas_mut(vert_id).unwrap().push_encoded_packet(EncodedPacket {
        pts_us: 0,
        dts_us: 0,
        is_keyframe: true,
        kind: MediaKind::Video,
        track_index: 0,
        payload: vec![1, 2, 3, 4],
    });
    assert_eq!(mgr.canvas(vert_id).unwrap().save_backtrack(5.0).unwrap().len(), 1);
    assert!(mgr.validate_all_streams()[&sq_id].is_empty());
}

// =========================================================================
// 13. WASM FX BRIDGE (sex-wasm FrameFxHost)
// =========================================================================

fn push_uleb128(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
}

fn push_section(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.push(id);
    push_uleb128(out, payload.len() as u32);
    out.extend_from_slice(payload);
}

/// Builds a minimal valid WASM binary exporting `"memory"` and `"process_frame"`:
/// - `process_frame(in_ptr, out_ptr, w, h, params_ptr, params_len)` loops `i = 0 .. w*h*4`
///   and writes `out[i] = 255 - in[i]`, returning 0.
fn build_wasm_invert_module() -> Vec<u8> {
    let mut wasm = Vec::new();
    wasm.extend_from_slice(b"\0asm\x01\0\0\0");

    // Type section: 1 func type: (6x i32) -> i32
    push_section(&mut wasm, 1, &[1, 0x60, 6, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 1, 0x7F]);
    // Function section: 1 func of type 0
    push_section(&mut wasm, 3, &[1, 0]);
    // Memory section: 1 mem, min 1 page
    push_section(&mut wasm, 5, &[1, 0, 1]);
    // Export section: "memory" (mem 0), "process_frame" (func 0)
    let mut exp = Vec::new();
    exp.push(2);
    exp.push(6);
    exp.extend_from_slice(b"memory");
    exp.push(0x02);
    exp.push(0);
    exp.push(13);
    exp.extend_from_slice(b"process_frame");
    exp.push(0x00);
    exp.push(0);
    push_section(&mut wasm, 7, &exp);

    // Code section: 1 body with 2 locals (local 6: $i, local 7: $total)
    let mut body = Vec::new();
    body.push(1); // 1 local decl run
    body.push(2); // count = 2 i32 locals
    body.push(0x7F);
    // total = (w * h) * 4
    body.extend_from_slice(&[
        0x20, 2, // local.get $w
        0x20, 3, // local.get $h
        0x6C,    // i32.mul
        0x41, 4, // i32.const 4
        0x6C,    // i32.mul
        0x21, 7, // local.set $total
        0x41, 0, // i32.const 0
        0x21, 6, // local.set $i
        // loop $L
        0x03, 0x40,
        // if i < total
        0x20, 6, 0x20, 7, 0x48, // i32.lt_s
        0x04, 0x40,
        // out[i] = 255 - in[i]
        0x20, 1, 0x20, 6, 0x6A, // out_ptr + i
        0x41, 0xFF, 0x01,       // i32.const 255 (signed LEB128!)
        0x20, 0, 0x20, 6, 0x6A, // in_ptr + i
        0x2D, 0x00, 0x00,       // i32.load8_u align=0 offset=0
        0x6B,                   // i32.sub
        0x3A, 0x00, 0x00,       // i32.store8 align=0 offset=0
        // i += 1
        0x20, 6, 0x41, 1, 0x6A, 0x21, 6,
        0x0C, 1, // br 1 (loop)
        0x0B,    // end if
        0x0B,    // end loop
        0x41, 0, // return 0
        0x0B,    // end func
    ]);

    let mut code = Vec::new();
    code.push(1); // 1 body
    push_uleb128(&mut code, body.len() as u32);
    code.extend_from_slice(&body);
    push_section(&mut wasm, 10, &code);
    wasm
}

/// Builds a minimal WASM binary with an infinite loop inside `process_frame`.
fn build_wasm_infinite_loop_module() -> Vec<u8> {
    let mut wasm = Vec::new();
    wasm.extend_from_slice(b"\0asm\x01\0\0\0");
    push_section(&mut wasm, 1, &[1, 0x60, 6, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 1, 0x7F]);
    push_section(&mut wasm, 3, &[1, 0]);
    push_section(&mut wasm, 5, &[1, 0, 1]);
    let mut exp = Vec::new();
    exp.push(2);
    exp.push(6);
    exp.extend_from_slice(b"memory");
    exp.push(0x02);
    exp.push(0);
    exp.push(13);
    exp.extend_from_slice(b"process_frame");
    exp.push(0x00);
    exp.push(0);
    push_section(&mut wasm, 7, &exp);
    let body = [0u8, 0x03, 0x40, 0x0C, 0x00, 0x0B, 0x41, 0x00, 0x0B];
    let mut code = Vec::new();
    code.push(1);
    push_uleb128(&mut code, body.len() as u32);
    code.extend_from_slice(&body);
    push_section(&mut wasm, 10, &code);
    wasm
}

#[test]
fn test_wasm_fx_bridge_invert_and_sex_color_roundtrip() {
    let wasm = build_wasm_invert_module();
    let mut bridge = WasmFxBridge::new().with_transfer(Rgba8Transfer::Linear);
    assert_eq!(bridge.limits().fuel_per_frame, DEFAULT_FRAME_FUEL);

    bridge.register_wasm("invert_fx", 1, &wasm).unwrap();
    assert!(bridge.is_cached("invert_fx", 1));

    let input = solid(4, 4, [0.2, 0.4, 0.8, 1.0]);
    let out = bridge.process_frame("invert_fx", &input, &[1.0], 5000).unwrap();
    assert!(out.report.fuel_consumed > 0);
    let px = out.frame.get_pixel(0, 0).unwrap();
    assert!((px[0] - 0.8).abs() < 0.02);
    assert!((px[1] - 0.6).abs() < 0.02);
    assert!((px[2] - 0.2).abs() < 0.02);

    // Verify sex_color::LinearFrame bridge roundtrip.
    let lf = framebuffer_to_linear_frame(&out.frame, ColorPrimaries::Rec709);
    let roundtrip = linear_frame_to_framebuffer(&lf).unwrap();
    assert_eq!(roundtrip, out.frame);
}

#[test]
fn test_wasm_fx_bridge_fuel_exhaustion() {
    let wasm = build_wasm_infinite_loop_module();
    let mut bridge = WasmFxBridge::with_limits(FxLimits {
        fuel_per_frame: 10_000,
        ..Default::default()
    });
    bridge.register_wasm("hang_fx", 1, &wasm).unwrap();

    let input = solid(2, 2, [0.5, 0.5, 0.5, 1.0]);
    let res = bridge.process_frame("hang_fx", &input, &[], 0);
    assert_eq!(res, Err(WasmFxError::Wasm(FxError::FuelExhausted)));
}
