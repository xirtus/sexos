use std::collections::BTreeMap;

use cabaret_core::audio::{
    AudioError, AudioFilter, AudioFilterEntry, AudioMixer, AudioStrip, ChannelLayout,
    ExpanderPreset, MoanPlugFormat, MoanPlugSlot, MonitorMode, PttPtmState, R128Meter, StripId,
    MAX_AUDIO_FILTERS_PER_STRIP, MAX_SYNC_OFFSET_MS, MIN_SYNC_OFFSET_MS,
};
use cabaret_core::compositor::{
    blend_pixel, scale_frame, solve_snap, Anchor, BlendMode, BoundsMode, Compositor,
    CompositorLayer, CropRect, FrameBuffer, MixColorSpace, Rect2D, Scaler, SnapConfig,
    Transform2D,
};
use cabaret_core::filters::{
    apply_video_filter, auto_pick_backdrop_color, process_filter_chain, AiMatteInput,
    AiMatteMode, AiMatteStatus, BlurKind, ColorWheel, DeinterlaceMode, FilterChain, FilterContext,
    FilterEntry, FilterError, Lut3D, MaskBlendMode, ScaleAspectMode, ToneMapMethod, VideoFilter,
    MAX_FILTERS_PER_SOURCE,
};
use cabaret_core::outputs::{
    redact_log_line, validate_service_limits, AudioCodec, AudioEncodeProfile, AudioTrackMask,
    CongestionAction, CongestionController, ContainerFormat, DestId, EncodedPacket, MediaKind,
    MultiStreamRouter, OutputError, QcPlayerState, RateControl, RecordingSession, RecordingState,
    RecordingStep, ReplayBuffer, ServicePlatform, ShibariSecretSlot, SplitRule, StreamDelayQueue,
    StreamDest, StreamTarget, TeleprompterState, ValidationSeverity, VideoCodec,
    VideoEncodeProfile,
};
use cabaret_core::scene::{
    render_studio_frame, ColorRange, ColorSpaceProfile, CrashGuard, CrashRecoveryDecision, LoopMode,
    PhoneProtocol, Profile, RescalePolicy, SceneCollection, SceneError, SourceKind,
    StudioLayoutMode, TextOutline, TextShadow, UndoCommand, UndoStack, UvcPixelFormat,
    MAX_GROUP_DIM,
};
use cabaret_core::transitions::{
    evaluate_item_visibility_transition, evaluate_stinger_audio, render_transition_frame,
    wipe_alpha, MultiviewAction, MultiviewLayout, MultiviewPreset, StingerAudioMode,
    StingerConfig, StingerCutPoint, StudioMode, TallyState, TrackMatte, TransitionKind,
    TransitionSpec, TransitionTick, WipeDirection, ALL_WIPE_PATTERNS,
};

fn solid_frame(w: u32, h: u32, rgba: [f32; 4]) -> FrameBuffer {
    FrameBuffer::filled(w, h, rgba).expect("valid dimensions")
}

// =========================================================================
// 1. SCENE TESTS
// =========================================================================

#[test]
fn test_all_13_source_kinds_and_deterministic_roundtrip() {
    let mut col = SceneCollection::new("MainShow", 1920, 1080);

    let s_disp = col
        .add_source("Display1", SourceKind::Display { display_id: 0, show_cursor: true })
        .unwrap();
    let s_win = col
        .add_source(
            "SilkWin",
            SourceKind::SilkWindow {
                frame_id: 42,
                follow_focused: true,
                hide_self: true,
            },
        )
        .unwrap();
    let s_reg = col
        .add_source("Region", SourceKind::DisplayRegion { x: 100, y: 200, w: 640, h: 480 })
        .unwrap();
    let s_uvc = col
        .add_source(
            "Webcam",
            SourceKind::UvcCamera {
                device_id: String::from("uvc://0"),
                pixel_fmt: UvcPixelFormat::Nv12,
                range_auto: true,
            },
        )
        .unwrap();
    let s_phone = col
        .add_source(
            "Phone",
            SourceKind::PhoneStream {
                protocol: PhoneProtocol::HookupDirect,
                port: 9001,
                passphrase_or_key: String::from("slot:1"),
                hookup_device: String::from("Pixel-9"),
            },
        )
        .unwrap();
    let s_media = col
        .add_source(
            "IntroClip",
            SourceKind::MediaFile {
                path: String::from("/media/intro.mp4"),
                speed_025_to_4x: 8.0, // clamped to 4.0
                pitch_preserve: true,
                loop_mode: LoopMode::PingPong,
                alpha: true,
            },
        )
        .unwrap();
    assert!(matches!(
        col.source(s_media).unwrap().kind,
        SourceKind::MediaFile { speed_025_to_4x, .. } if (speed_025_to_4x - 4.0).abs() < 1e-6
    ));
    let s_browser = col
        .add_source(
            "Alerts",
            SourceKind::Browser {
                url: String::from("https://alerts.local/overlay"),
                width: 1280,
                height: 720,
                custom_css: String::from("body { background: transparent;\n }"),
                shutdown_when_hidden: true,
                private_audio: true,
            },
        )
        .unwrap();
    let s_deck = col
        .add_source(
            "Keynote",
            SourceKind::PresentationDeck {
                path: String::from("/decks/launch.burlesque"),
                slide_idx: 3,
                build_step: 2,
            },
        )
        .unwrap();
    let s_slide = col
        .add_source(
            "Slides",
            SourceKind::ImageSlideshow {
                paths: vec![String::from("a.png"), String::from("b.png")],
                interval_ms: 5000,
                shuffle: true,
            },
        )
        .unwrap();
    let s_text = col
        .add_source(
            "Ticker",
            SourceKind::RichText {
                content: String::from("Line 1\nLine \"2\""),
                font_px: 36.0,
                color: [1.0, 0.9, 0.8, 1.0],
                outline: Some(TextOutline {
                    width_px: 2.0,
                    color: [0.0, 0.0, 0.0, 1.0],
                }),
                shadow: Some(TextShadow {
                    offset_x: 3.0,
                    offset_y: 4.0,
                    blur_px: 6.0,
                    color: [0.0, 0.0, 0.0, 0.6],
                }),
                tail_lines: Some(5),
            },
        )
        .unwrap();
    let s_solid = col
        .add_source("Backdrop", SourceKind::ColorSolid { rgba: [0.1, 0.2, 0.3, 1.0] })
        .unwrap();

    let st_sub = col.add_studio("SubStudio");
    col.add_item(st_sub, s_solid, "SubBg").unwrap();

    let s_nested = col
        .add_source("Nested", SourceKind::NestedStudio { studio_id: st_sub })
        .unwrap();
    let s_group = col
        .add_source(
            "OverlayGroup",
            SourceKind::Group {
                width: 1920,
                height: 1080,
                collapsed: false,
                items: Vec::new(),
            },
        )
        .unwrap();
    col.add_item_to_group(s_group, s_text, "ChildTicker").unwrap();

    let st_main = col.add_studio("MainStudio");
    col.studio_mut(st_main).unwrap().layout = StudioLayoutMode::Grid { columns: 3 };
    col.studio_mut(st_main).unwrap().override_transition = Some(TransitionSpec::fade(450));

    for (i, sid) in [
        s_disp, s_win, s_reg, s_uvc, s_phone, s_media, s_browser, s_deck, s_slide, s_nested,
        s_group,
    ]
    .iter()
    .enumerate()
    {
        col.add_item(st_main, *sid, format!("Item{i}")).unwrap();
    }

    let text1 = col.to_text();
    let parsed = SceneCollection::from_text(&text1).expect("roundtrip parse");
    let text2 = parsed.to_text();
    assert_eq!(text1, text2, "serialization must be strictly deterministic");
    assert_eq!(parsed.sources.len(), col.sources.len());
    assert_eq!(parsed.studios.len(), col.studios.len());
}

#[test]
fn test_nested_studio_cycle_rejection() {
    let mut col = SceneCollection::new("CycleTest", 1920, 1080);
    let st_a = col.add_studio("A");
    let st_b = col.add_studio("B");
    let st_c = col.add_studio("C");

    let ref_a = col
        .add_source("RefA", SourceKind::NestedStudio { studio_id: st_a })
        .unwrap();
    let ref_b = col
        .add_source("RefB", SourceKind::NestedStudio { studio_id: st_b })
        .unwrap();
    let ref_c = col
        .add_source("RefC", SourceKind::NestedStudio { studio_id: st_c })
        .unwrap();

    // Direct self-cycle A -> A rejected.
    let err_self = col.add_item(st_a, ref_a, "SelfRef").unwrap_err();
    assert_eq!(err_self, SceneError::CycleDetected { from: st_a, to: st_a });

    // Chain A -> B -> C works.
    col.add_item(st_a, ref_b, "A->B").unwrap();
    col.add_item(st_b, ref_c, "B->C").unwrap();

    // Closing loop C -> A must fail and roll back cleanly.
    let err_loop = col.add_item(st_c, ref_a, "C->A").unwrap_err();
    assert!(matches!(err_loop, SceneError::CycleDetected { .. }));
    assert!(col.studio(st_c).unwrap().items.is_empty());

    // Even hiding C -> A inside a Group must be caught!
    let grp = col
        .add_source(
            "GroupTrap",
            SourceKind::Group {
                width: 640,
                height: 360,
                collapsed: false,
                items: Vec::new(),
            },
        )
        .unwrap();
    col.add_item_to_group(grp, ref_a, "HiddenRefA").unwrap();
    let err_grp = col.add_item(st_c, grp, "GroupWithCycle").unwrap_err();
    assert!(matches!(err_grp, SceneError::CycleDetected { .. }));
}

#[test]
fn test_group_16k_limits_and_subcomposite_render() {
    let mut col = SceneCollection::new("Group16k", 64, 64);
    assert_eq!(MAX_GROUP_DIM, 16_384);

    assert!(matches!(
        col.add_source(
            "Zero",
            SourceKind::Group {
                width: 0,
                height: 100,
                collapsed: false,
                items: Vec::new(),
            },
        ),
        Err(SceneError::GroupSizeOutOfRange { .. })
    ));
    assert!(matches!(
        col.add_source(
            "TooBig",
            SourceKind::Group {
                width: 16_385,
                height: 100,
                collapsed: false,
                items: Vec::new(),
            },
        ),
        Err(SceneError::GroupSizeOutOfRange { .. })
    ));

    // Exact 16384x16384 metadata is admitted.
    let g_max = col
        .add_source(
            "Max16k",
            SourceKind::Group {
                width: 16_384,
                height: 16_384,
                collapsed: true,
                items: Vec::new(),
            },
        )
        .unwrap();
    assert!(col.source(g_max).is_some());

    // Functional Group sub-composite rendering:
    let red = col
        .add_source("Red", SourceKind::ColorSolid { rgba: [1.0, 0.0, 0.0, 1.0] })
        .unwrap();
    let grp = col
        .add_source(
            "Sub",
            SourceKind::Group {
                width: 16,
                height: 16,
                collapsed: false,
                items: Vec::new(),
            },
        )
        .unwrap();
    col.add_item_to_group(grp, red, "RedInGroup").unwrap();

    let st = col.add_studio("Main");
    let item_id = col.add_item(st, grp, "SubItem").unwrap();
    col.studio_mut(st).unwrap().items[0].transform.pos = [8.0, 8.0];
    let _ = item_id;

    let raw = BTreeMap::new();
    let mut ctxs = BTreeMap::new();
    let ai = BTreeMap::new();
    let fb = render_studio_frame(&col, st, 32, 32, &raw, &mut ctxs, &ai).unwrap();
    assert_eq!(fb.get_pixel(0, 0).unwrap(), [0.0, 0.0, 0.0, 0.0]);
    assert!((fb.get_pixel(12, 12).unwrap()[0] - 1.0).abs() < 1e-4);
}

#[test]
fn test_copy_reference_vs_paste_duplicate() {
    let mut col = SceneCollection::new("CopyTest", 1920, 1080);
    let st1 = col.add_studio("S1");
    let st2 = col.add_studio("S2");
    let src = col
        .add_source("Cam", SourceKind::ColorSolid { rgba: [0.2, 0.4, 0.6, 1.0] })
        .unwrap();
    let item1 = col.add_item(st1, src, "CamItem").unwrap();

    let clip = col.copy_reference(st1, item1).unwrap();
    let ref_item = col.paste_reference(st2, &clip).unwrap();
    let (dup_item, dup_src) = col.paste_duplicate(st2, &clip).unwrap();

    let s2 = col.studio(st2).unwrap();
    assert_eq!(s2.items[0].id, ref_item);
    assert_eq!(s2.items[0].source_id, src); // shared SourceId!
    assert_eq!(s2.items[1].id, dup_item);
    assert_eq!(s2.items[1].source_id, dup_src);
    assert_ne!(dup_src, src); // brand-new SourceId!
}

#[test]
fn test_undo_redo_reversible_history() {
    let mut col = SceneCollection::new("UndoTest", 1920, 1080);
    let st = col.add_studio("Main");
    let src = col
        .add_source("Box", SourceKind::ColorSolid { rgba: [1.0, 1.0, 1.0, 1.0] })
        .unwrap();
    let item_id = col.add_item(st, src, "BoxItem").unwrap();
    let orig_tr = col.studio(st).unwrap().items[0].transform;

    let mut new_tr = orig_tr;
    new_tr.pos = [400.0, 300.0];

    let mut stack = UndoStack::new(32);
    stack
        .execute(
            &mut col,
            UndoCommand::SetItemTransform {
                studio_id: st,
                item_id,
                old_transform: orig_tr,
                new_transform: new_tr,
            },
        )
        .unwrap();
    stack
        .execute(
            &mut col,
            UndoCommand::RescaleCanvas {
                old_w: 1920,
                old_h: 1080,
                new_w: 3840,
                new_h: 2160,
                policy: RescalePolicy::Stretch,
                old_transforms: vec![(st, item_id, new_tr)],
            },
        )
        .unwrap();

    assert_eq!(col.base_width, 3840);
    assert!((col.studio(st).unwrap().items[0].transform.pos[0] - 800.0).abs() < 1e-4);

    assert!(stack.undo(&mut col).unwrap().is_some());
    assert_eq!(col.base_width, 1920);
    assert!((col.studio(st).unwrap().items[0].transform.pos[0] - 400.0).abs() < 1e-4);

    assert!(stack.undo(&mut col).unwrap().is_some());
    assert_eq!(col.studio(st).unwrap().items[0].transform.pos, [0.0, 0.0]);
    assert!(!stack.can_undo());

    assert!(stack.redo(&mut col).unwrap().is_some());
    assert!(stack.redo(&mut col).unwrap().is_some());
    assert_eq!(col.base_width, 3840);
    assert!((col.studio(st).unwrap().items[0].transform.pos[0] - 800.0).abs() < 1e-4);
}

#[test]
fn test_profile_serialization_and_canvas_rescaling() {
    let mut prof = Profile::default();
    prof.name = String::from("4K HDR Studio");
    prof.output_width = 3840;
    prof.output_height = 2160;
    prof.color_space = ColorSpaceProfile::Rec2020Pq;
    prof.color_range = ColorRange::Full;
    prof.channel_layout = ChannelLayout::Surround5_1;
    prof.video_bitrate_kbps = 25_000;

    let txt = prof.to_text();
    let parsed = Profile::from_text(&txt).unwrap();
    assert_eq!(parsed, prof);

    // KeepAspectCenter rescaling from 1920x1080 -> 1080x1920 (vertical canvas).
    let mut col = SceneCollection::new("Vertical", 1920, 1080);
    let st = col.add_studio("Main");
    let src = col
        .add_source("Bg", SourceKind::ColorSolid { rgba: [1.0; 4] })
        .unwrap();
    col.add_item(st, src, "Bg").unwrap();
    col.rescale_canvas(1080, 1920, RescalePolicy::KeepAspectCenter)
        .unwrap();
    let tr = col.studio(st).unwrap().items[0].transform;
    assert!((tr.scale[0] - (1080.0 / 1920.0)).abs() < 1e-5);
    assert!(tr.pos[1] > 0.0); // vertically centered letterbox offset
}

#[test]
fn test_crash_guard_and_safe_mode_recovery() {
    assert!(matches!(
        CrashGuard::evaluate_marker(None),
        CrashRecoveryDecision::CleanStart(flags) if !flags.safe_mode
    ));

    let running_marker = CrashGuard::begin_session("sess-99", 1000, "Main", "Default");
    match CrashGuard::evaluate_marker(Some(&running_marker)) {
        CrashRecoveryDecision::UncleanShutdownDetected {
            previous_session_id,
            recommended,
        } => {
            assert_eq!(previous_session_id, "sess-99");
            assert!(recommended.safe_mode);
            assert!(recommended.disable_third_party_plugins);
            assert!(recommended.disable_browser_sources);
            assert!(recommended.disable_virtual_cam);
        }
        other => panic!("expected unclean shutdown, got {other:?}"),
    }

    let clean_marker = CrashGuard::mark_clean_exit("sess-99", 5000);
    assert!(matches!(
        CrashGuard::evaluate_marker(Some(&clean_marker)),
        CrashRecoveryDecision::CleanStart(flags) if !flags.safe_mode
    ));
}

// =========================================================================
// 2. COMPOSITOR TESTS
// =========================================================================

#[test]
fn test_all_8_bounds_modes() {
    let base = Transform2D {
        bounds: [200.0, 100.0],
        ..Transform2D::default()
    };

    let modes = [
        (BoundsMode::None, [1.0, 1.0], false),
        (BoundsMode::Stretch, [2.0, 0.5], false),
        (BoundsMode::Fit, [0.5, 0.5], false),
        (BoundsMode::Fill, [2.0, 2.0], false),
        (BoundsMode::Cover, [2.0, 2.0], true),
        (BoundsMode::ScaleToWidth, [2.0, 2.0], false),
        (BoundsMode::ScaleToHeight, [0.5, 0.5], false),
        (BoundsMode::MaxOnly, [0.5, 0.5], false),
    ];

    for (mode, expected_scale, expected_clip) in modes {
        let mut t = base;
        t.bounds_mode = mode;
        let res = t.resolve(100.0, 200.0);
        assert!(
            (res.effective_scale[0] - expected_scale[0]).abs() < 1e-4
                && (res.effective_scale[1] - expected_scale[1]).abs() < 1e-4,
            "mode {mode:?} got {:?}",
            res.effective_scale
        );
        assert_eq!(res.clip_to_box, expected_clip, "mode {mode:?}");
    }
}

#[test]
fn test_transform_crop_flip_rotation_and_cover_clip() {
    let mut src = FrameBuffer::new(4, 4).unwrap();
    src.set_pixel(0, 0, [1.0, 0.0, 0.0, 1.0]); // red top-left
    src.set_pixel(3, 0, [0.0, 1.0, 0.0, 1.0]); // green top-right
    src.set_pixel(0, 3, [0.0, 0.0, 1.0, 1.0]); // blue bottom-left

    let mut tr = Transform2D::default();
    tr.pos = [8.0, 8.0];
    tr.anchor = Anchor::Center;
    tr.rot_deg = 90.0;
    tr.flip_h = true;
    tr.crop = CropRect {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    let comp = Compositor::new([0.0, 0.0, 0.0, 1.0]);
    let out = comp
        .composite(
            16,
            16,
            &[CompositorLayer {
                frame: &src,
                transform: tr,
                blend_mode: BlendMode::Normal,
                opacity: 1.0,
                mix_space: MixColorSpace::Linear,
                scaler: Scaler::Point,
                visible: true,
            }],
        )
        .unwrap();

    // Check rotated & flipped pixel landed inside [6..10, 6..10] while [0,0] stays background black.
    assert_eq!(out.get_pixel(0, 0).unwrap(), [0.0, 0.0, 0.0, 1.0]);
    let p = out.get_pixel(9, 6).unwrap();
    assert!(p[0] > 0.5 || p[1] > 0.5 || p[2] > 0.5);
}

#[test]
fn test_snapping_guide_solver() {
    let cfg = SnapConfig::default();
    let other = Rect2D::new(800.0, 100.0, 200.0, 100.0);

    // Near horizontal center (canvas_w/2 = 960) and sibling top y=100 (closer than 80% safe area at y=108).
    let cand = Rect2D::new(855.0, 102.0, 200.0, 100.0);
    let res = solve_snap(cand, 1920.0, 1080.0, &[other], cfg);
    assert!((res.snapped_rect.center_x() - 960.0).abs() < 1e-3);
    assert!((res.snapped_rect.y - 100.0).abs() < 1e-3);
    assert!(!res.guides.is_empty());
}

#[test]
fn test_all_6_scalers_and_linear_light_filtering() {
    // Alternating white (1.0) and black (0.0) stripes -> Area downsample 2x must average exactly 0.5 linear!
    let mut stripes = FrameBuffer::new(8, 8).unwrap();
    for y in 0..8 {
        for x in 0..8 {
            let v = if x % 2 == 0 { 1.0 } else { 0.0 };
            stripes.set_pixel(x, y, [v, v, v, 1.0]);
        }
    }

    let area = scale_frame(&stripes, 2, 2, Scaler::Area).unwrap();
    assert!((area.get_pixel(0, 0).unwrap()[0] - 0.5).abs() < 0.05);

    for scaler in [
        Scaler::Auto,
        Scaler::Point,
        Scaler::Bilinear,
        Scaler::Bicubic,
        Scaler::Lanczos3,
        Scaler::Area,
    ] {
        let up = scale_frame(&stripes, 16, 16, scaler).unwrap();
        assert_eq!(up.width, 16);
        assert_eq!(up.height, 16);
    }
}

#[test]
fn test_all_9_blend_modes_and_linear_vs_srgb_opacity() {
    let dst = [0.2, 0.4, 0.8, 1.0];
    let src = [0.5, 0.5, 0.25, 1.0];

    let add = blend_pixel(dst, src, BlendMode::Additive, 1.0, MixColorSpace::Linear);
    assert!((add[0] - 0.7).abs() < 1e-5);

    let sub = blend_pixel(dst, src, BlendMode::Subtract, 1.0, MixColorSpace::Linear);
    assert!((sub[2] - 0.55).abs() < 1e-5);

    let mul = blend_pixel(dst, src, BlendMode::Multiply, 1.0, MixColorSpace::Linear);
    assert!((mul[0] - 0.10).abs() < 1e-5);

    let scr = blend_pixel(dst, src, BlendMode::Screen, 1.0, MixColorSpace::Linear);
    assert!((scr[0] - (0.2 + 0.5 - 0.1)).abs() < 1e-5);

    let lgt = blend_pixel(dst, src, BlendMode::Lighten, 1.0, MixColorSpace::Linear);
    assert!((lgt[0] - 0.5).abs() < 1e-5 && (lgt[2] - 0.8).abs() < 1e-5);

    let drk = blend_pixel(dst, src, BlendMode::Darken, 1.0, MixColorSpace::Linear);
    assert!((drk[0] - 0.2).abs() < 1e-5 && (drk[2] - 0.25).abs() < 1e-5);

    let lb = blend_pixel([0.8, 0.8, 0.8, 1.0], [0.5, 0.5, 0.5, 1.0], BlendMode::LinearBurn, 1.0, MixColorSpace::Linear);
    assert!((lb[0] - 0.3).abs() < 1e-5);

    let ov = blend_pixel(dst, src, BlendMode::Overlay, 1.0, MixColorSpace::Linear);
    assert!((ov[0] - (2.0 * 0.2 * 0.5)).abs() < 1e-5);

    // Linear vs sRGB opacity mixing differs significantly for black-over-white 50% opacity.
    let lin_50 = blend_pixel(
        [1.0, 1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0, 1.0],
        BlendMode::Normal,
        0.5,
        MixColorSpace::Linear,
    );
    let srgb_50 = blend_pixel(
        [1.0, 1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0, 1.0],
        BlendMode::Normal,
        0.5,
        MixColorSpace::Srgb,
    );
    assert!((lin_50[0] - 0.5).abs() < 1e-4);
    assert!(srgb_50[0] < 0.25); // sRGB 0.5 converts to ~0.214 linear!
}

// =========================================================================
// 3. FILTERS TESTS (ALL 19 + HDR)
// =========================================================================

#[test]
fn test_filter_chain_max_32_enforced() {
    let mut chain = FilterChain::new();
    for i in 0..MAX_FILTERS_PER_SOURCE {
        chain
            .add(FilterEntry {
                id: i as u64,
                name: format!("Sharpen{i}"),
                enabled: true,
                filter: VideoFilter::Sharpen { amount: 0.1 },
            })
            .unwrap();
    }
    let err = chain
        .add(FilterEntry {
            id: 999,
            name: String::from("Overflow"),
            enabled: true,
            filter: VideoFilter::Sharpen { amount: 0.1 },
        })
        .unwrap_err();
    assert_eq!(err, FilterError::TooManyFilters);
}

#[test]
fn test_deinterlace_all_4_modes() {
    let mut combed = FrameBuffer::new(4, 4).unwrap();
    for y in 0..4 {
        let v = if y % 2 == 0 { 1.0 } else { 0.0 };
        for x in 0..4 {
            combed.set_pixel(x, y, [v, v, v, 1.0]);
        }
    }
    let mut ctx = FilterContext::new();
    for mode in [
        DeinterlaceMode::Discard,
        DeinterlaceMode::Linear,
        DeinterlaceMode::Blend,
        DeinterlaceMode::MotionAdaptive,
    ] {
        let res = apply_video_filter(
            &VideoFilter::Deinterlace {
                mode,
                top_field_first: true,
            },
            &combed,
            &mut ctx,
            AiMatteInput::Unavailable,
            None,
        )
        .unwrap();
        // Odd line (y=1) should now be interpolated toward 1.0 rather than 0.0.
        assert!(res.get_pixel(0, 1).unwrap()[0] > 0.2, "mode {mode:?}");
    }
}

#[test]
fn test_chroma_key_auto_backdrop_and_spill_suppression() {
    // 16x16 frame: green screen border + clean warm skin center + green-spilled edge at (5, 5).
    let mut frame = solid_frame(16, 16, [0.05, 0.85, 0.05, 1.0]);
    for y in 4..12 {
        for x in 4..12 {
            frame.set_pixel(x, y, [0.6, 0.45, 0.3, 1.0]);
        }
    }
    // Hair/shoulder fringe contaminated with green spill (green excess > red/blue):
    frame.set_pixel(5, 5, [0.30, 0.58, 0.28, 1.0]);

    let picked = auto_pick_backdrop_color(&frame);
    assert!(picked[1] > picked[0] && picked[1] > picked[2]);

    let mut ctx = FilterContext::new();
    let keyed = apply_video_filter(
        &VideoFilter::ChromaKey {
            key_color_rgb: [0.0, 0.0, 0.0],
            auto_backdrop_pick: true,
            similarity: 0.15,
            smoothness: 0.05,
            spill_suppression: 0.9,
        },
        &frame,
        &mut ctx,
        AiMatteInput::Unavailable,
        None,
    )
    .unwrap();

    // Backdrop corner must be keyed transparent, clean skin stays opaque, and spilled fringe green is suppressed!
    assert!(keyed.get_pixel(0, 0).unwrap()[3] < 0.05);
    let subj = keyed.get_pixel(8, 8).unwrap();
    assert!(subj[3] > 0.95);
    let fringe = keyed.get_pixel(5, 5).unwrap();
    assert!(fringe[1] < 0.50, "green spill must be suppressed below 0.50, got {}", fringe[1]);
}

#[test]
fn test_ai_background_matte_fail_closed_and_guided_refinement() {
    let mut frame = solid_frame(12, 12, [0.1, 0.2, 0.8, 1.0]);
    for y in 3..9 {
        for x in 3..9 {
            frame.set_pixel(x, y, [0.9, 0.7, 0.6, 1.0]);
        }
    }

    let mut chain = FilterChain::new();
    chain
        .add(FilterEntry {
            id: 1,
            name: String::from("Matte"),
            enabled: true,
            filter: VideoFilter::AiBackgroundMatte {
                mode: AiMatteMode::Cutout,
                blur_radius: 4.0,
                feather: 0.1,
                temporal_smoothing: 0.5,
                edge_refine_radius: 2,
                model_id: String::from("cabaret-seg-v1"),
            },
        })
        .unwrap();

    // 1. FAIL-CLOSED: InferenceFailed must hide entire source (every pixel alpha == 0).
    let mut ctx = FilterContext::new();
    let failed_out =
        process_filter_chain(&chain, &frame, &mut ctx, AiMatteInput::InferenceFailed, None)
            .unwrap();
    assert_eq!(ctx.ai_matte_status, AiMatteStatus::FailedClosedHidden);
    assert!(failed_out.pixels.iter().all(|&p| p == [0.0, 0.0, 0.0, 0.0]));

    // 2. FAIL-CLOSED: Unavailable must also hide entire source.
    let unavail_out =
        process_filter_chain(&chain, &frame, &mut ctx, AiMatteInput::Unavailable, None).unwrap();
    assert_eq!(ctx.ai_matte_status, AiMatteStatus::FailedClosedHidden);
    assert!(unavail_out.pixels.iter().all(|&p| p == [0.0, 0.0, 0.0, 0.0]));

    // 3. SUCCESS: Guided filter preserves subject while cutting out background + temporal anti-shimmer.
    let mut mask = vec![0.0_f32; 144];
    for y in 3..9 {
        for x in 3..9 {
            mask[y * 12 + x] = 1.0;
        }
    }
    let ok_out = process_filter_chain(&chain, &frame, &mut ctx, AiMatteInput::Success(&mask), None)
        .unwrap();
    assert_eq!(ctx.ai_matte_status, AiMatteStatus::Applied);
    assert!(ok_out.get_pixel(0, 0).unwrap()[3] < 0.2);
    assert!(ok_out.get_pixel(6, 6).unwrap()[3] > 0.8);
}

#[test]
fn test_light_wrap_wraps_live_background_onto_foreground_edges() {
    // Keyed foreground: opaque white circle/box in center [4..12, 4..12], transparent outside.
    let mut fg = FrameBuffer::new(16, 16).unwrap();
    for y in 4..12 {
        for x in 4..12 {
            fg.set_pixel(x, y, [0.2, 0.2, 0.2, 1.0]);
        }
    }

    // Live composited bright orange/red stage background behind foreground.
    let bg = solid_frame(16, 16, [1.0, 0.4, 0.0, 1.0]);

    let mut ctx = FilterContext::new();
    let wrapped = apply_video_filter(
        &VideoFilter::LightWrap {
            intensity: 0.8,
            radius: 2.0,
            luma_threshold: 0.1,
            soft_bleed: 0.2,
        },
        &fg,
        &mut ctx,
        AiMatteInput::Unavailable,
        Some(&bg),
    )
    .unwrap();

    // Transparent outer pixel remains zeroed.
    assert_eq!(wrapped.get_pixel(0, 0).unwrap(), [0.0, 0.0, 0.0, 0.0]);
    // Deep opaque interior pixel (8, 8) stays untouched gray.
    assert!((wrapped.get_pixel(8, 8).unwrap()[0] - 0.2).abs() < 1e-4);
    // Foreground perimeter edge pixel (4, 4) receives warm red/orange light wrap from live bg!
    let edge = wrapped.get_pixel(4, 4).unwrap();
    assert!(edge[0] > 0.4 && edge[0] > edge[2]);
}

#[test]
fn test_cube_lut_and_remaining_video_filters() {
    let cube = "\
TITLE \"Test Warm\"
LUT_3D_SIZE 2
0.0 0.0 0.0
1.0 0.0 0.0
0.0 1.0 0.0
1.0 1.0 0.0
0.0 0.0 1.0
1.0 0.0 1.0
0.0 1.0 1.0
1.0 1.0 1.0
";
    let lut = Lut3D::from_cube_str(cube).unwrap();
    let mid = lut.sample_trilinear([0.5, 0.25, 0.75]);
    assert!((mid[0] - 0.5).abs() < 1e-5 && (mid[1] - 0.25).abs() < 1e-5 && (mid[2] - 0.75).abs() < 1e-5);

    let img = solid_frame(8, 8, [2.5, 1.8, 0.5, 1.0]);
    let mut ctx = FilterContext::new();
    let mask = solid_frame(8, 8, [0.5, 0.5, 0.5, 0.5]);

    // Verify Denoise, ColorWheels4Way, LegacyColor, ColorKey, LumaKey, ApplyLut,
    // ImageMaskBlend, CropPad, ScaleAspect, Scroll, Blur, DropShadow, RenderDelay,
    // VideoDelay, HdrToneMap, SdrOnHdr all execute cleanly.
    let filters = vec![
        VideoFilter::Denoise { temporal_strength: 0.4, chroma_strength: 0.6, radius: 1 },
        VideoFilter::ColorWheels4Way {
            lift: ColorWheel::ZERO,
            gamma: ColorWheel::ONE,
            gain: ColorWheel::ONE,
            offset: ColorWheel::ZERO,
            saturation: 1.0,
        },
        VideoFilter::LegacyColor {
            brightness: 0.0,
            contrast: 1.0,
            gamma: 1.0,
            saturation: 1.0,
            hue_shift_deg: 15.0,
            opacity: 1.0,
        },
        VideoFilter::ColorKey {
            key_color_rgb: [0.0, 1.0, 0.0],
            similarity: 0.1,
            smoothness: 0.05,
        },
        VideoFilter::LumaKey {
            luma_max: 1.0,
            luma_max_smooth: 0.05,
            luma_min: 0.0,
            luma_min_smooth: 0.05,
            invert: false,
        },
        VideoFilter::ApplyLut { path: String::from("test.cube"), lut, intensity: 1.0 },
        VideoFilter::ImageMaskBlend {
            mode: MaskBlendMode::AlphaMaskAlphaChannel,
            mask: Some(mask),
            color: [1.0, 1.0, 1.0, 1.0],
            stretch: true,
        },
        VideoFilter::Blur { kind: BlurKind::DualKawase, radius: 2.0, passes: 1 },
        VideoFilter::DropShadow {
            offset_x: 1.0,
            offset_y: 1.0,
            blur_radius: 1.0,
            color: [0.0, 0.0, 0.0, 0.8],
            spread: 0.2,
        },
        VideoFilter::HdrToneMap {
            method: ToneMapMethod::AcesFitted,
            source_peak_nits: 1000.0,
            target_peak_nits: 100.0,
            desaturate_highlights: 0.3,
        },
        VideoFilter::SdrOnHdr {
            sdr_white_nits: 203.0,
            paper_white_scale: 1.0,
            expand_highlights: 0.25,
        },
    ];

    for f in &filters {
        let out = apply_video_filter(f, &img, &mut ctx, AiMatteInput::Unavailable, None).unwrap();
        assert_eq!(out.width, 8);
    }

    let cropped = apply_video_filter(
        &VideoFilter::CropPad {
            top: 2,
            right: 2,
            bottom: -2,
            left: -2,
            pad_color: [0.0; 4],
        },
        &img,
        &mut ctx,
        AiMatteInput::Unavailable,
        None,
    )
    .unwrap();
    assert_eq!((cropped.width, cropped.height), (8, 8));

    let scaled = apply_video_filter(
        &VideoFilter::ScaleAspect {
            target_width: 16,
            target_height: 8,
            scaler: Scaler::Bilinear,
            aspect_mode: ScaleAspectMode::FitLetterbox,
        },
        &img,
        &mut ctx,
        AiMatteInput::Unavailable,
        None,
    )
    .unwrap();
    assert_eq!((scaled.width, scaled.height), (16, 8));

    ctx.timestamp_us = 500_000;
    let scrolled = apply_video_filter(
        &VideoFilter::Scroll {
            speed_x_px_s: 4.0,
            speed_y_px_s: 0.0,
            loop_wrap: true,
            limit_width: Some(6),
            limit_height: Some(6),
        },
        &img,
        &mut ctx,
        AiMatteInput::Unavailable,
        None,
    )
    .unwrap();
    assert_eq!((scrolled.width, scrolled.height), (6, 6));

    // RenderDelay + VideoDelay rings:
    ctx.frame_duration_ms = 20.0;
    let _ = apply_video_filter(
        &VideoFilter::RenderDelay { delay_ms: 20 },
        &img,
        &mut ctx,
        AiMatteInput::Unavailable,
        None,
    )
    .unwrap();
    let delayed = apply_video_filter(
        &VideoFilter::RenderDelay { delay_ms: 20 },
        &solid_frame(8, 8, [0.1; 4]),
        &mut ctx,
        AiMatteInput::Unavailable,
        None,
    )
    .unwrap();
    assert!((delayed.get_pixel(0, 0).unwrap()[0] - 2.5).abs() < 1e-4);

    let _ = apply_video_filter(
        &VideoFilter::VideoDelay { delay_ms: 20, max_frames: 5 },
        &img,
        &mut ctx,
        AiMatteInput::Unavailable,
        None,
    )
    .unwrap();
    let v_delayed = apply_video_filter(
        &VideoFilter::VideoDelay { delay_ms: 20, max_frames: 5 },
        &solid_frame(8, 8, [0.2; 4]),
        &mut ctx,
        AiMatteInput::Unavailable,
        None,
    )
    .unwrap();
    assert!((v_delayed.get_pixel(0, 0).unwrap()[0] - 2.5).abs() < 1e-4);
}

// =========================================================================
// 4. TRANSITIONS & MULTIVIEW TESTS
// =========================================================================

#[test]
fn test_all_29_wipe_patterns_coverage_and_boundaries() {
    assert_eq!(ALL_WIPE_PATTERNS.len(), 29);

    let a = solid_frame(16, 16, [1.0, 0.0, 0.0, 1.0]);
    let b = solid_frame(16, 16, [0.0, 0.0, 1.0, 1.0]);

    for &pattern in &ALL_WIPE_PATTERNS {
        // Exact boundary checks at t=0 and t=1 even with high softness.
        for v in [0.1, 0.5, 0.9] {
            for u in [0.1, 0.5, 0.9] {
                assert_eq!(wipe_alpha(pattern, u, v, 0.0, 0.2, false, 0.0), 0.0);
                assert_eq!(wipe_alpha(pattern, u, v, 1.0, 0.2, false, 0.0), 1.0);
            }
        }

        // Mid-transition render should mix A and B across the 16x16 frame.
        let mid = render_transition_frame(
            &a,
            &b,
            &TransitionKind::Wipe29 {
                pattern,
                softness: 0.05,
                invert: false,
                angle_deg: 0.0,
            },
            0.5,
            None,
            None,
        );
        let has_red = mid.pixels.iter().any(|p| p[0] > 0.35);
        let has_blue = mid.pixels.iter().any(|p| p[2] > 0.35);
        assert!(has_red && has_blue, "pattern {pattern:?} did not split frame at t=0.5");
    }
}

#[test]
fn test_transition_kinds_and_stinger_track_mattes() {
    let a = solid_frame(8, 8, [1.0, 0.0, 0.0, 1.0]);
    let b = solid_frame(8, 8, [0.0, 1.0, 0.0, 1.0]);

    for kind in [
        TransitionKind::Cut,
        TransitionKind::Fade,
        TransitionKind::FadeThroughColor {
            color: [0.0, 0.0, 0.0, 1.0],
            peak_point: 0.5,
        },
        TransitionKind::Swipe {
            direction: WipeDirection::Right,
            slide_out: true,
        },
        TransitionKind::Slide {
            direction: WipeDirection::Left,
        },
        TransitionKind::Cover {
            direction: WipeDirection::Up,
        },
        TransitionKind::Uncover {
            direction: WipeDirection::Down,
        },
    ] {
        let mid = render_transition_frame(&a, &b, &kind, 0.5, None, None);
        assert_eq!((mid.width, mid.height), (8, 8));
    }

    // SplitHorizontal stinger: left half white semi-transparent graphic, right half half-white matte (0.5).
    let mut split_stinger = FrameBuffer::new(16, 8).unwrap();
    for y in 0..8 {
        for x in 0..8 {
            split_stinger.set_pixel(x, y, [1.0, 1.0, 1.0, 0.25]);
        }
        for x in 8..16 {
            split_stinger.set_pixel(x, y, [1.0, 1.0, 1.0, 1.0]); // matte luminance = 1.0 -> B!
        }
    }
    let st_cfg = StingerConfig {
        asset_path: String::from("stinger.mov"),
        duration_ms: 1000,
        fps_num: 60,
        fps_den: 1,
        cut_point: StingerCutPoint::Frame(30),
        track_matte: Some(TrackMatte::SplitHorizontal),
        audio_mode: StingerAudioMode::DuckProgram { duck_db: -12.0 },
        monitoring_enabled: true,
    };
    assert!((st_cfg.cut_point_normalized() - 0.5).abs() < 1e-4);

    let out = render_transition_frame(
        &a,
        &b,
        &TransitionKind::Stinger {
            config: st_cfg.clone(),
        },
        0.5,
        Some(&split_stinger),
        None,
    );
    // Matte is 1.0 -> base is green B, overlaid with 0.25 white stinger -> strong green component.
    assert!(out.get_pixel(2, 2).unwrap()[1] > 0.7);

    let gains = evaluate_stinger_audio(0.5, &st_cfg);
    assert!(gains.stinger_gain > 0.9 && gains.incoming_b_gain < 0.35);
}

#[test]
fn test_studio_mode_tbar_isolated_staging_and_overrides() {
    let mut col = SceneCollection::new("StudioMode", 1920, 1080);
    let st_pv = col.add_studio("Preview");
    let st_pg = col.add_studio("Program");
    col.studio_mut(st_pv).unwrap().override_transition = Some(TransitionSpec::cut());

    let mut mode = StudioMode::new(st_pv, st_pg);
    assert!(mode.copy_sources_isolated);

    // Edit staged copy of Preview without touching SceneCollection until take().
    {
        let staged = mode.begin_isolated_preview_edit(&col).unwrap();
        staged.name = String::from("Preview (Edited)");
    }
    assert_eq!(col.studio(st_pv).unwrap().name, "Preview");

    mode.set_t_bar(0.6);
    assert!((mode.t_bar - 0.6).abs() < 1e-6);

    // Take honors per-studio Cut override and commits isolated staged preview!
    let tick = mode.take(Some(&mut col));
    assert_eq!(tick, TransitionTick::Completed { new_program: st_pv });
    assert_eq!(mode.program_studio, st_pv);
    assert_eq!(mode.preview_studio, st_pg);
    assert_eq!(col.studio(st_pv).unwrap().name, "Preview (Edited)");

    // Non-cut timed transition advances via tick().
    mode.active_transition = TransitionSpec::fade(200);
    col.studio_mut(mode.preview_studio).unwrap().override_transition = None;
    let t0 = mode.take(None);
    assert!(matches!(t0, TransitionTick::InProgress { .. }));
    assert!(matches!(mode.tick(100.0), TransitionTick::InProgress { t_bar } if (t_bar - 0.5).abs() < 1e-4));
    assert!(matches!(mode.tick(100.0), TransitionTick::Completed { .. }));

    // Per-item Show/Hide transition helper.
    assert!((evaluate_item_visibility_transition(true, 0.25, None) - 0.25).abs() < 1e-5);
    assert!((evaluate_item_visibility_transition(false, 0.25, None) - 0.75).abs() < 1e-5);
}

#[test]
fn test_multiview_5_presets_tally_safe_areas_and_clicks() {
    let mut col = SceneCollection::new("MV", 1920, 1080);
    let s1 = col.add_studio("S1");
    let s2 = col.add_studio("S2");
    let s3 = col.add_studio("S3");
    let studios = vec![s1, s2, s3];
    let mut mode = StudioMode::new(s1, s2);
    mode.active_transition = TransitionSpec::cut();

    for (preset, expected_slots) in [
        (MultiviewPreset::Standard8, 10),
        (MultiviewPreset::Grid4, 4),
        (MultiviewPreset::Grid9, 9),
        (MultiviewPreset::Grid16, 16),
        (MultiviewPreset::Grid25, 25),
    ] {
        let mv = MultiviewLayout::compute(preset, 1920.0, 1080.0, 10.0, &studios, &mode);
        assert_eq!(mv.slots.len(), expected_slots);
    }

    let mv = MultiviewLayout::compute(MultiviewPreset::Standard8, 1920.0, 1080.0, 10.0, &studios, &mode);
    assert_eq!(mv.slots[0].tally, TallyState::PreviewGreen);
    assert_eq!(mv.slots[1].tally, TallyState::ProgramRed);
    // 90% and 80% safe areas are nested properly inside viewport.
    let sa = mv.slots[0].safe_areas;
    assert!(sa.title_safe_80.x > sa.action_safe_90.x);
    assert!(sa.title_safe_80.w < sa.action_safe_90.w);

    // Click slot 4 (which maps to studios[2] = s3) once -> Preview.
    let s3_slot = &mv.slots[4];
    let cx = s3_slot.rect.x + 10.0;
    let cy = s3_slot.rect.y + 10.0;
    assert_eq!(
        mv.handle_click(cx, cy, 1, &mut mode, Some(&mut col)),
        Some(MultiviewAction::ClickToPreview(s3))
    );
    assert_eq!(mode.preview_studio, s3);

    // Double-click slot 4 -> Take to Program!
    assert_eq!(
        mv.handle_click(cx, cy, 2, &mut mode, Some(&mut col)),
        Some(MultiviewAction::DoubleClickToTake(s3))
    );
    assert_eq!(mode.program_studio, s3);
}

// =========================================================================
// 5. AUDIO TESTS
// =========================================================================

#[test]
fn test_surround_itu_downmix_and_sync_offset_clamping() {
    assert_eq!(ChannelLayout::Mono.channels(), 1);
    assert_eq!(ChannelLayout::Stereo.channels(), 2);
    assert_eq!(ChannelLayout::Surround5_1.channels(), 6);
    assert_eq!(ChannelLayout::Surround7_1.channels(), 8);

    // Center channel only at 1.0 -> ITU -3dB (0.7071) on both L & R.
    let c_only = [0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    let st = ChannelLayout::Surround5_1.downmix_to_stereo(&c_only, 0.0);
    assert!((st[0] - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    assert!((st[1] - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);

    let mut strip = AudioStrip::new(StripId(1), "Mic", ChannelLayout::Mono, 48_000);
    strip.set_sync_offset_ms(-5000.0);
    assert_eq!(strip.sync_offset_ms, MIN_SYNC_OFFSET_MS);
    strip.set_sync_offset_ms(5000.0);
    assert_eq!(strip.sync_offset_ms, MAX_SYNC_OFFSET_MS);
}

#[test]
fn test_feedback_safe_monitor_routing_and_ptt_ptm() {
    let mut mixer = AudioMixer::new(48_000, "Speakers-Main");

    let mut desktop = AudioStrip::new(StripId(1), "Desktop", ChannelLayout::Stereo, 48_000);
    desktop.captures_monitor_device_id = Some(String::from("Speakers-Main"));
    desktop.monitor_mode = MonitorMode::MonitorAndOutput;

    let mut mic = AudioStrip::new(StripId(2), "Mic", ChannelLayout::Mono, 48_000);
    mic.monitor_mode = MonitorMode::MonitorOnly;

    mixer.add_strip(desktop);
    mixer.add_strip(mic);

    let desktop_pcm = vec![0.5_f32; 256 * 2];
    let mic_pcm = vec![0.25_f32; 256];
    let mut inputs = BTreeMap::new();
    inputs.insert(StripId(1), desktop_pcm.as_slice());
    inputs.insert(StripId(2), mic_pcm.as_slice());

    let out = mixer.mix_block(&inputs, 256).unwrap();
    // Desktop capture targeting the same monitor device MUST be feedback-suppressed on monitor bus!
    assert_eq!(out.feedback_suppressed_strips, vec![StripId(1)]);
    // Desktop still reaches Track 1, whereas Mic (MonitorOnly) does not reach Track 1.
    assert!((out.tracks[0][10][0] - 0.5).abs() < 1e-4);
    // Monitor bus contains ONLY Mic's equal-power stereo feed, not Desktop loopback!
    let expected_mic = 0.25 * core::f32::consts::FRAC_1_SQRT_2;
    assert!((out.monitor_bus[10][0] - expected_mic).abs() < 1e-4);

    // Push-to-Talk hold timer check:
    let mut ptt = PttPtmState {
        ptt_enabled: true,
        ptt_delay_ms: 100,
        ptt_pressed: true,
        ..PttPtmState::default()
    };
    assert!(ptt.advance(10.0));
    ptt.ptt_pressed = false;
    assert!(ptt.advance(50.0)); // still open inside 100ms release hold
    assert!(!ptt.advance(60.0)); // closed once hold expires
}

#[test]
fn test_4x_true_peak_intersample_and_ebu_r128_lufs() {
    let sr = 48_000_u32;
    let mut meter = R128Meter::new(sr);

    // 12 kHz (fs/4) sine sampled at +45°, +135°, +225°, +315° with amplitude 1.0:
    // Every sample magnitude is cos(45°) = 0.7071 (-3.01 dBFS), while continuous wave peaks at 1.0 (0.0 dBTP)!
    let mut intersample_wave = Vec::with_capacity(4800);
    for n in 0..4800 {
        let phase = core::f32::consts::FRAC_PI_4 + (n as f32) * core::f32::consts::FRAC_PI_2;
        let v = libm::sinf(phase);
        intersample_wave.push([v, v]);
    }
    let m1 = meter.process_stereo_block(&intersample_wave);
    assert!(m1.sample_peak_db < -2.9, "sample peak should be ~-3.01 dBFS, got {}", m1.sample_peak_db);
    assert!(m1.true_peak_dbtp > -0.6, "4x True-Peak must detect > -0.6 dBTP intersample peak, got {}", m1.true_peak_dbtp);

    // 1 kHz stereo sine at 0.1 amplitude (-20 dBFS peak) for 0.6s -> LUFS should sit around -18..-22 LUFS.
    let mut meter2 = R128Meter::new(sr);
    let mut sine_1k = Vec::with_capacity(28_800);
    for n in 0..28_800 {
        let v = 0.1 * libm::sinf(2.0 * core::f32::consts::PI * 1000.0 * (n as f32) / sr as f32);
        sine_1k.push([v, v]);
    }
    let m2 = meter2.process_stereo_block(&sine_1k);
    assert!(
        m2.momentary_lufs > -24.0 && m2.momentary_lufs < -14.0,
        "got momentary {}",
        m2.momentary_lufs
    );
    assert!(
        m2.integrated_lufs > -24.0 && m2.integrated_lufs < -14.0,
        "got integrated {}",
        m2.integrated_lufs
    );
}

#[test]
fn test_all_12_audio_filters_cap_16_moan_plug_and_sidechain_ducking() {
    let mut strip = AudioStrip::new(StripId(1), "Voice", ChannelLayout::Stereo, 48_000);
    for i in 0..MAX_AUDIO_FILTERS_PER_STRIP {
        strip
            .add_filter(AudioFilterEntry {
                id: i as u64,
                name: format!("G{i}"),
                enabled: false,
                filter: AudioFilter::Gain { gain_db: 0.0 },
            })
            .unwrap();
    }
    assert_eq!(
        strip
            .add_filter(AudioFilterEntry {
                id: 99,
                name: String::from("Overflow"),
                enabled: true,
                filter: AudioFilter::Gain { gain_db: 0.0 },
            })
            .unwrap_err(),
        AudioError::TooManyFilters
    );

    // Build real 12-filter test + MoanPlugSlot + Sidechain ducking test:
    let mut mixer = AudioMixer::new(48_000, "Out");
    let mut voice = AudioStrip::new(StripId(1), "Voice", ChannelLayout::Stereo, 48_000);
    voice.add_moan_plugin(MoanPlugSlot {
        slot_id: 1,
        plugin_id: String::from("org.moan.deverb"),
        format: MoanPlugFormat::Clap,
        bypass: false,
        params: vec![(String::from("dry_wet"), 0.7)],
    });
    for (idx, f) in [
        AudioFilter::Gain { gain_db: 0.0 },
        AudioFilter::NoiseGate {
            open_threshold_db: -45.0,
            close_threshold_db: -50.0,
            attack_ms: 1.0,
            hold_ms: 20.0,
            release_ms: 50.0,
        },
        AudioFilter::VoiceDenoise {
            strength: 0.3,
            floor_db: -55.0,
        },
        AudioFilter::SpectralSuppress {
            threshold_db: -50.0,
            reduction_db: 12.0,
        },
        AudioFilter::Expander {
            ratio: 1.5,
            threshold_db: -50.0,
            attack_ms: 2.0,
            release_ms: 40.0,
            output_gain_db: 0.0,
            preset: ExpanderPreset::Expander,
        },
        AudioFilter::UpwardCompressor {
            ratio: 2.0,
            threshold_db: -35.0,
            max_boost_db: 6.0,
            attack_ms: 5.0,
            release_ms: 80.0,
        },
        AudioFilter::PolarityInvert { invert: false },
        AudioFilter::Eq3Band {
            low_db: -2.0,
            low_hz: 100.0,
            mid_db: 2.0,
            mid_hz: 1500.0,
            mid_q: 1.0,
            high_db: 1.5,
            high_hz: 8000.0,
        },
        AudioFilter::DeEsser {
            freq_hz: 6500.0,
            bandwidth_q: 2.0,
            threshold_db: -18.0,
            ratio: 3.0,
        },
        AudioFilter::Limiter {
            threshold_db: -1.0,
            release_ms: 50.0,
        },
        AudioFilter::AudioDelay {
            delay_ms: 5.0,
            feedback: 0.2,
            wet_mix: 0.0,
        },
    ]
    .into_iter()
    .enumerate()
    {
        voice
            .add_filter(AudioFilterEntry {
                id: idx as u64,
                name: format!("F{idx}"),
                enabled: true,
                filter: f,
            })
            .unwrap();
    }

    let mut music = AudioStrip::new(StripId(2), "Music", ChannelLayout::Stereo, 48_000);
    music.track_mask = 0b00_0010; // Route music to Track 2 so Track 1 stays voice-only!
    music
        .add_filter(AudioFilterEntry {
            id: 100,
            name: String::from("SidechainDuck"),
            enabled: true,
            filter: AudioFilter::Compressor {
                ratio: 8.0,
                threshold_db: -24.0,
                attack_ms: 1.0,
                release_ms: 50.0,
                output_gain_db: 0.0,
                sidechain_source: Some(StripId(1)),
            },
        })
        .unwrap();

    mixer.add_strip(voice);
    mixer.add_strip(music);

    let loud_voice = vec![0.8_f32; 1024 * 2];
    let steady_music = vec![0.5_f32; 1024 * 2];
    let mut inputs = BTreeMap::new();
    inputs.insert(StripId(1), loud_voice.as_slice());
    inputs.insert(StripId(2), steady_music.as_slice());

    let out_ducked = mixer.mix_block(&inputs, 1024).unwrap();
    let ducked_level = out_ducked.tracks[1][1000][0].abs();
    assert!(
        ducked_level < 0.25,
        "music must duck below 0.25 when voice speaks, got {ducked_level}"
    );

    // Silence voice -> music recovers back toward 0.5!
    let silent_voice = vec![0.0_f32; 1024 * 2];
    let mut inputs2 = BTreeMap::new();
    inputs2.insert(StripId(1), silent_voice.as_slice());
    inputs2.insert(StripId(2), steady_music.as_slice());
    let mut recovered_level = 0.0;
    for _ in 0..10 {
        let out_rec = mixer.mix_block(&inputs2, 1024).unwrap();
        recovered_level = out_rec.tracks[1][1000][0].abs();
    }
    assert!(
        recovered_level > 0.42,
        "music must release back above 0.42 when voice stops, got {recovered_level}"
    );
}

// =========================================================================
// 6. OUTPUTS, MULTI-STREAM, REPLAY, QC PLAYER & TELEPROMPTER TESTS
// =========================================================================

#[test]
fn test_multistream_destinations_encoder_groups_and_service_validator() {
    let mut router = MultiStreamRouter::new();
    let shared_v = VideoEncodeProfile::default(); // 1080p60 H264 6000kbps 2s
    let shared_a = AudioEncodeProfile::default();

    // Target 1 & 2 share identical Video+Audio profiles -> single shared EncoderGroup!
    router.add_target(StreamTarget {
        id: DestId(1),
        name: String::from("Twitch"),
        platform: ServicePlatform::Twitch,
        dest: StreamDest::Rtmp {
            url: String::from("rtmp://live.twitch.tv/app"),
            secret_slot: ShibariSecretSlot::new(10, "twitch-key"),
        },
        video: shared_v,
        audio: shared_a,
        enabled: true,
        delay_sec: 0,
    });
    router.add_target(StreamTarget {
        id: DestId(2),
        name: String::from("Kick"),
        platform: ServicePlatform::Kick,
        dest: StreamDest::EnhancedRtmp {
            url: String::from("rtmp://ingest.kick.com/app"),
            secret_slot: ShibariSecretSlot::new(11, "kick-key"),
            codec: VideoCodec::H264,
        },
        video: shared_v,
        audio: shared_a,
        enabled: true,
        delay_sec: 0,
    });
    // Target 3 uses 4K AV1 18Mbps -> gets a split EncoderGroup!
    let mut yt_v = shared_v;
    yt_v.width = 3840;
    yt_v.height = 2160;
    yt_v.codec = VideoCodec::Av1;
    yt_v.bitrate_kbps = 18_000;
    yt_v.rate_control = RateControl::Vbr;
    router.add_target(StreamTarget {
        id: DestId(3),
        name: String::from("YouTube"),
        platform: ServicePlatform::YouTube,
        dest: StreamDest::Whip {
            endpoint_url: String::from("https://yt.example/whip"),
            bearer_slot: Some(ShibariSecretSlot::new(12, "yt-bearer")),
        },
        video: yt_v,
        audio: AudioEncodeProfile {
            codec: AudioCodec::Opus,
            bitrate_kbps: 256,
            sample_rate_hz: 48_000,
            track_index: 1,
        },
        enabled: true,
        delay_sec: 15,
    });

    let groups = router.plan_encoder_groups();
    assert_eq!(groups.len(), 2);
    assert!(groups.iter().any(|g| g.shared && g.dest_ids.len() == 2));
    assert!(groups.iter().any(|g| !g.shared && g.dest_ids == vec![DestId(3)]));
    assert!(router.validate_all().is_empty());

    // Invalid Twitch target exceeding 8500 kbps & keyint=4 triggers Errors!
    let bad_twitch = StreamTarget {
        id: DestId(99),
        name: String::from("BadTwitch"),
        platform: ServicePlatform::Twitch,
        dest: StreamDest::Srt {
            host: String::from("ingest.example"),
            port: 9000,
            passphrase_slot: None,
            latency_ms: 200,
            stream_id: String::from("s1"),
        },
        video: VideoEncodeProfile {
            bitrate_kbps: 12_000,
            keyint_sec: 4,
            ..shared_v
        },
        audio: shared_a,
        enabled: true,
        delay_sec: 0,
    };
    let issues = validate_service_limits(&bad_twitch);
    assert!(issues.iter().any(|i| i.severity == ValidationSeverity::Error));

    // Also exercise remaining StreamDest variants cleanly:
    let _other_dests = [
        StreamDest::Whep { endpoint_url: String::from("https://w/whep"), bearer_slot: None },
        StreamDest::Rist { url: String::from("rist://127.0.0.1:5000"), buffer_ms: 150, secret_slot: None },
        StreamDest::Hls { manifest_path: String::from("/var/hls/index.m3u8"), segment_duration_ms: 2000 },
        StreamDest::NdiLink { stream_name: String::from("CabaretMain"), groups: String::from("Public") },
        StreamDest::CustomMuxer {
            format_name: String::from("matroska"),
            target: String::from("udp://127.0.0.1:4000"),
            options: vec![(String::from("flush_packets"), String::from("1"))],
        },
    ];
}

#[test]
fn test_congestion_step_down_delay_queue_and_shibari_redaction() {
    let mut cc = CongestionController::new(6000, 1500);
    let act1 = cc.observe(10, 100, 120, 0.85);
    assert!(matches!(act1, CongestionAction::StepDown { new_bitrate_kbps } if new_bitrate_kbps == 4500));

    // Three consecutive healthy ticks step bitrate back up.
    assert!(matches!(cc.observe(0, 100, 40, 0.1), CongestionAction::Hold { .. }));
    assert!(matches!(cc.observe(0, 100, 40, 0.1), CongestionAction::Hold { .. }));
    assert!(matches!(cc.observe(0, 100, 40, 0.1), CongestionAction::RecoverUp { .. }));

    // Reconnect-preserving StreamDelayQueue keeps delayed packets across disconnect/reconnect!
    let mut dq = StreamDelayQueue::new(10, true);
    dq.push(EncodedPacket {
        pts_us: 1_000_000,
        dts_us: 1_000_000,
        is_keyframe: true,
        kind: MediaKind::Video,
        track_index: 0,
        payload: vec![1, 2, 3],
    });
    dq.on_disconnect(2_000_000);
    assert_eq!(dq.queued_len(), 1);
    assert!(dq.pop_ready(12_000_000).is_empty());
    dq.on_reconnect();
    let released = dq.pop_ready(12_000_000);
    assert_eq!(released.len(), 1);

    // Shibari secret redaction:
    let slot = ShibariSecretSlot::new(7, "twitch-live");
    assert_eq!(format!("{slot}"), "[SHIBARI_SLOT#7:twitch-live]");
    let redacted = redact_log_line(
        "Connecting with key live_secret_abc123 to rtmp://twitch",
        &[(slot, "live_secret_abc123")],
    );
    assert!(!redacted.contains("live_secret_abc123"));
    assert!(redacted.contains("[SHIBARI:REDACTED:slot#7]"));
}

#[test]
fn test_recording_hybrid_mp4_6_tracks_pause_split_and_low_disk() {
    let mask = AudioTrackMask::from_tracks(&[0, 2, 5]);
    assert_eq!(mask.active_tracks(), vec![0, 2, 5]);

    let mut rec = RecordingSession::new(ContainerFormat::HybridMp4, mask);
    rec.split_rule = SplitRule::TimeSeconds(2);
    rec.min_free_disk_bytes = 100_000;
    rec.start();

    let pkt = |pts: i64, kf: bool, kind: MediaKind, track: u8| EncodedPacket {
        pts_us: pts,
        dts_us: pts,
        is_keyframe: kf,
        kind,
        track_index: track,
        payload: vec![0u8; 512],
    };

    assert!(matches!(
        rec.push_packet(&pkt(0, true, MediaKind::Video, 0), 500_000),
        RecordingStep::Written { segment_index: 1, adjusted_pts_us: 0 }
    ));
    // Track 1 is NOT in mask -> ignored; Track 2 IS in mask -> written.
    assert_eq!(
        rec.push_packet(&pkt(500_000, false, MediaKind::Audio, 1), 500_000),
        RecordingStep::IgnoredPausedOrTrack
    );
    assert!(matches!(
        rec.push_packet(&pkt(500_000, false, MediaKind::Audio, 2), 500_000),
        RecordingStep::Written { .. }
    ));

    // Pause for 1 second -> PTS advances monotonically without gap.
    rec.pause(1_000_000);
    assert_eq!(
        rec.push_packet(&pkt(1_500_000, false, MediaKind::Video, 0), 500_000),
        RecordingStep::IgnoredPausedOrTrack
    );
    rec.resume(2_000_000);
    rec.add_chapter("Boss Fight");
    assert_eq!(rec.chapters[0].timestamp_us, 500_000);

    // Non-keyframe past 2s does not split; incoming keyframe triggers clean segment split!
    assert!(matches!(
        rec.push_packet(&pkt(3_200_000, false, MediaKind::Video, 0), 500_000),
        RecordingStep::Written { segment_index: 1, .. }
    ));
    assert_eq!(
        rec.push_packet(&pkt(3_500_000, true, MediaKind::Video, 0), 500_000),
        RecordingStep::SplitToSegment { new_segment_index: 2 }
    );

    // Low disk triggers clean stop + Hybrid MP4 moov finalization!
    assert_eq!(
        rec.push_packet(&pkt(4_000_000, false, MediaKind::Video, 0), 50_000),
        RecordingStep::StoppedLowDiskSpace
    );
    assert_eq!(rec.state, RecordingState::StoppedLowDisk);
    assert!(rec.hybrid_mp4_moov_finalized);
}

#[test]
fn test_circular_replay_buffer_keyframe_alignment() {
    let mut rb = ReplayBuffer::new(10, 1024 * 1024);
    assert_eq!(rb.save_last_seconds(5.0).unwrap_err(), OutputError::EmptyReplayBuffer);

    for i in 0..10 {
        let pts = i * 1_000_000;
        let is_kf = i % 3 == 0; // keyframes at 0s, 3s, 6s, 9s
        rb.push_packet(EncodedPacket {
            pts_us: pts,
            dts_us: pts,
            is_keyframe: is_kf,
            kind: MediaKind::Video,
            track_index: 0,
            payload: vec![i as u8; 100],
        });
    }

    // Asking for last 2 seconds from 9s -> target 7s -> snaps back to keyframe at 6s!
    let saved = rb.save_last_seconds(2.0).unwrap();
    assert!(saved[0].is_keyframe);
    assert_eq!(saved[0].pts_us, 6_000_000);
    assert_eq!(saved.last().unwrap().pts_us, 9_000_000);
}

#[test]
fn test_qc_player_jkl_shuttle_and_teleprompter_voice_tracking() {
    let mut qc = QcPlayerState::new("/rec/take1.mp4", 10_000_000, 60, 1);
    qc.shuttle_l();
    assert_eq!(qc.shuttle_speed, 1.0);
    qc.shuttle_l();
    assert_eq!(qc.shuttle_speed, 2.0);
    qc.shuttle_l();
    assert_eq!(qc.shuttle_speed, 4.0);
    qc.advance(1_000_000); // +4s at 4x!
    assert_eq!(qc.position_us, 4_000_000);

    qc.mark_in();
    qc.shuttle_j();
    assert_eq!(qc.shuttle_speed, -1.0);
    qc.shuttle_j();
    assert_eq!(qc.shuttle_speed, -2.0);
    qc.shuttle_k();
    assert_eq!(qc.shuttle_speed, 0.0);

    qc.frame_step(3);
    assert!(qc.position_us > 4_000_000);
    qc.mark_out();
    let splicy = qc.export_splicy_clip().unwrap();
    assert!(splicy.mark_out_us > splicy.mark_in_us);

    let mut tp = TeleprompterState::new("Welcome to Cabaret live on SexOS today!");
    assert!(tp.hide_from_capture);
    tp.mirror_h = true;
    tp.advance_time(1.0);
    assert!(tp.scroll_offset_px > 0.0);

    let idx = tp.update_voice_match(&["Welcome", "to", "Cabaret"]);
    assert_eq!(idx, 3);
    let idx2 = tp.update_voice_match(&["SexOS", "today"]);
    assert_eq!(idx2, 7);
}

#[test]
fn test_srgb_rgba8_roundtrip_and_linear_midpoint() {
    let bytes = [188_u8, 128, 0, 255, 255, 255, 255, 128];
    let fb = FrameBuffer::from_srgb_rgba8(2, 1, &bytes).unwrap();
    // sRGB 188 maps near 0.5 linear light; sRGB 128 maps near 0.215 linear light!
    let p0 = fb.get_pixel(0, 0).unwrap();
    assert!((p0[0] - 0.5).abs() < 0.02);
    assert!((p0[1] - 0.215).abs() < 0.02);
    let out = fb.to_srgb_rgba8();
    assert_eq!(out, bytes);
}

#[test]
fn test_studio_nested_render_with_chroma_key_and_light_wrap() {
    let mut col = SceneCollection::new("LiveComposite", 16, 16);
    let bg_src = col
        .add_source("StageBg", SourceKind::ColorSolid { rgba: [1.0, 0.3, 0.0, 1.0] })
        .unwrap();
    let cam_src = col
        .add_source(
            "TalentCam",
            SourceKind::UvcCamera {
                device_id: String::from("cam0"),
                pixel_fmt: UvcPixelFormat::Rgba8,
                range_auto: true,
            },
        )
        .unwrap();

    // Attach ChromaKey + LightWrap onto camera source.
    let cam_def = col.source_mut(cam_src).unwrap();
    cam_def
        .filters
        .add(FilterEntry {
            id: 1,
            name: String::from("Key"),
            enabled: true,
            filter: VideoFilter::ChromaKey {
                key_color_rgb: [0.0, 0.9, 0.0],
                auto_backdrop_pick: true,
                similarity: 0.15,
                smoothness: 0.05,
                spill_suppression: 0.8,
            },
        })
        .unwrap();
    cam_def
        .filters
        .add(FilterEntry {
            id: 2,
            name: String::from("Wrap"),
            enabled: true,
            filter: VideoFilter::LightWrap {
                intensity: 0.9,
                radius: 2.0,
                luma_threshold: 0.1,
                soft_bleed: 0.3,
            },
        })
        .unwrap();

    let st_bg = col.add_studio("BackgroundStudio");
    col.add_item(st_bg, bg_src, "OrangeStage").unwrap();
    let nested_bg = col
        .add_source("NestedBg", SourceKind::NestedStudio { studio_id: st_bg })
        .unwrap();

    let st_main = col.add_studio("MainStudio");
    col.add_item(st_main, nested_bg, "BgLayer").unwrap();
    col.add_item(st_main, cam_src, "TalentLayer").unwrap();

    let mut cam_raw = solid_frame(16, 16, [0.02, 0.9, 0.02, 1.0]);
    for y in 4..12 {
        for x in 4..12 {
            cam_raw.set_pixel(x, y, [0.25, 0.25, 0.25, 1.0]);
        }
    }
    let mut raw_sources = BTreeMap::new();
    raw_sources.insert(cam_src, cam_raw);

    let mut ctxs = BTreeMap::new();
    let ai = BTreeMap::new();
    let out = render_studio_frame(&col, st_main, 16, 16, &raw_sources, &mut ctxs, &ai).unwrap();

    // Outside talent shows nested orange background; perimeter of talent receives warm wrap!
    assert!((out.get_pixel(0, 0).unwrap()[0] - 1.0).abs() < 1e-4);
    let rim = out.get_pixel(4, 4).unwrap();
    assert!(rim[0] > 0.35 && rim[0] > rim[2]);
}

#[test]
fn test_audio_negative_sync_offset_causal_alignment() {
    let mut mixer = AudioMixer::new(48_000, "Out");
    let mut early = AudioStrip::new(StripId(1), "EarlyMic", ChannelLayout::Stereo, 48_000);
    early.set_sync_offset_ms(-10.0); // 10ms ahead -> other strips get delayed by +10ms (480 samples)!
    let normal = AudioStrip::new(StripId(2), "CamAudio", ChannelLayout::Stereo, 48_000);
    normal.track_mask; // track 1

    let mut early_s = early;
    early_s.track_mask = 0b00_0001;
    let mut normal_s = normal;
    normal_s.track_mask = 0b00_0010;

    mixer.add_strip(early_s);
    mixer.add_strip(normal_s);

    // Feed impulse at sample 0 to both strips across a 960-sample block.
    let mut pcm1 = vec![0.0_f32; 960 * 2];
    pcm1[0] = 1.0;
    pcm1[1] = 1.0;
    let mut pcm2 = vec![0.0_f32; 960 * 2];
    pcm2[0] = 1.0;
    pcm2[1] = 1.0;

    let mut inputs = BTreeMap::new();
    inputs.insert(StripId(1), pcm1.as_slice());
    inputs.insert(StripId(2), pcm2.as_slice());

    let out = mixer.mix_block(&inputs, 960).unwrap();
    // Strip 1 (-10ms) arrives immediately at sample 0 on Track 1; Strip 2 (0ms) is delayed +480 samples on Track 2!
    assert!((out.tracks[0][0][0] - 1.0).abs() < 1e-4);
    assert!((out.tracks[1][0][0]).abs() < 1e-4);
    assert!((out.tracks[1][480][0] - 1.0).abs() < 1e-4);
}

#[test]
fn test_audio_delay_feedback_echo_and_polarity_invert() {
    let mut mixer = AudioMixer::new(10_000, "Out");
    let mut strip = AudioStrip::new(StripId(1), "FX", ChannelLayout::Stereo, 10_000);
    strip
        .add_filter(AudioFilterEntry {
            id: 1,
            name: String::from("Invert"),
            enabled: true,
            filter: AudioFilter::PolarityInvert { invert: true },
        })
        .unwrap();
    strip
        .add_filter(AudioFilterEntry {
            id: 2,
            name: String::from("Echo"),
            enabled: true,
            filter: AudioFilter::AudioDelay {
                delay_ms: 10.0, // 100 samples at 10kHz
                feedback: 0.5,
                wet_mix: 1.0,
            },
        })
        .unwrap();
    mixer.add_strip(strip);

    let mut pcm = vec![0.0_f32; 350 * 2];
    pcm[0] = 1.0;
    pcm[1] = 1.0;
    let mut inputs = BTreeMap::new();
    inputs.insert(StripId(1), pcm.as_slice());

    let out = mixer.mix_block(&inputs, 350).unwrap();
    // Polarity inverted + 100-sample delay with 0.5 recursive feedback:
    assert!((out.tracks[0][100][0] + 1.0).abs() < 1e-4);
    assert!((out.tracks[0][200][0] + 0.5).abs() < 1e-4);
    assert!((out.tracks[0][300][0] + 0.25).abs() < 1e-4);
}

#[test]
fn test_recording_size_split_and_manual_split() {
    let mut rec = RecordingSession::new(ContainerFormat::Mkv, AudioTrackMask::ALL_6);
    rec.split_rule = SplitRule::SizeBytes(1000);
    rec.start();

    let pkt = |pts: i64, kf: bool, len: usize| EncodedPacket {
        pts_us: pts,
        dts_us: pts,
        is_keyframe: kf,
        kind: MediaKind::Video,
        track_index: 0,
        payload: vec![0u8; len],
    };

    assert!(matches!(
        rec.push_packet(&pkt(0, true, 600), 10_000_000_000),
        RecordingStep::Written { segment_index: 1, .. }
    ));
    assert!(matches!(
        rec.push_packet(&pkt(33_333, false, 600), 10_000_000_000),
        RecordingStep::Written { segment_index: 1, .. }
    ));
    // Next keyframe triggers automatic size split to segment 2!
    assert_eq!(
        rec.push_packet(&pkt(66_666, true, 600), 10_000_000_000),
        RecordingStep::SplitToSegment { new_segment_index: 2 }
    );

    // Manual split arms until next keyframe.
    rec.request_manual_split();
    assert!(matches!(
        rec.push_packet(&pkt(100_000, false, 200), 10_000_000_000),
        RecordingStep::Written { segment_index: 2, .. }
    ));
    assert_eq!(
        rec.push_packet(&pkt(133_333, true, 200), 10_000_000_000),
        RecordingStep::SplitToSegment { new_segment_index: 3 }
    );
}

#[test]
fn test_stinger_alpha_and_luma_full_track_mattes() {
    let a = solid_frame(4, 4, [1.0, 0.0, 0.0, 1.0]);
    let b = solid_frame(4, 4, [0.0, 0.0, 1.0, 1.0]);

    // LumaFull matte: top half white (1.0 -> incoming B), bottom half black (0.0 -> outgoing A).
    let mut matte = FrameBuffer::new(4, 4).unwrap();
    for y in 0..2 {
        for x in 0..4 {
            matte.set_pixel(x, y, [1.0, 1.0, 1.0, 1.0]);
        }
    }
    let stinger_overlay = solid_frame(4, 4, [0.0, 0.0, 0.0, 0.0]);
    let cfg = StingerConfig {
        track_matte: Some(TrackMatte::LumaFull),
        ..StingerConfig::default()
    };

    let out = render_transition_frame(
        &a,
        &b,
        &TransitionKind::Stinger { config: cfg },
        0.5,
        Some(&stinger_overlay),
        Some(&matte),
    );
    assert!((out.get_pixel(1, 0).unwrap()[2] - 1.0).abs() < 1e-4);
    assert!((out.get_pixel(1, 3).unwrap()[0] - 1.0).abs() < 1e-4);

    // Audio crossfade & fade-out-fade-in modes:
    let cfg_fo = StingerConfig {
        audio_mode: StingerAudioMode::FadeOutFadeIn,
        ..StingerConfig::default()
    };
    let g_early = evaluate_stinger_audio(0.25, &cfg_fo);
    assert!(g_early.outgoing_a_gain > 0.0 && g_early.incoming_b_gain == 0.0);
    let g_late = evaluate_stinger_audio(0.75, &cfg_fo);
    assert!(g_late.outgoing_a_gain == 0.0 && g_late.incoming_b_gain > 0.0);
}
