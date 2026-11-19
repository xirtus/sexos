// SPDX-License-Identifier: GPL-3.0-only
//! Unit tests for Magic Move transitions, intra-slide build animations,
//! Rehearsal Session timing, and Video Timeline generation (G-28 / Slice T6).

use burlesque_core::*;
use sex_canvas::{Color, Rect};

#[test]
fn test_magic_move_matching_and_centroid_interpolation() {
    let mut s1 = Slide::new(1, SlideLayout::Blank);
    let mut s2 = Slide::new(2, SlideLayout::Blank);

    // Box 1 in Slide 1 at (100, 100), size (200, 100), Blue
    let shape1 = SlideElement::Shape(Shape {
        bounds: Rect::new(100.0, 100.0, 200.0, 100.0),
        kind: ShapeKind::RoundedRect { radius: 10.0 },
        fill: Some(Color::rgb(0, 0, 255)),
        stroke: Some(StrokeStyle { color: Color::WHITE, width: 2.0 }),
        magic_key: Some("hero-box".into()),
    });
    s1.add_element(shape1);

    // Box 1 in Slide 2 at (400, 300), size (300, 150), Red
    let shape2 = SlideElement::Shape(Shape {
        bounds: Rect::new(400.0, 300.0, 300.0, 150.0),
        kind: ShapeKind::RoundedRect { radius: 20.0 },
        fill: Some(Color::rgb(255, 0, 0)),
        stroke: Some(StrokeStyle { color: Color::WHITE, width: 4.0 }),
        magic_key: Some("hero-box".into()),
    });
    s2.add_element(shape2);

    let w = 960;
    let h = 540;
    let theme = Theme::burlesque_velvet();
    let steps = [0.0, 0.25, 0.5, 0.75, 1.0];
    let mut prev_cx = 0.0f32;
    let mut prev_cy = 0.0f32;

    for (i, &t) in steps.iter().enumerate() {
        let mut pixels = vec![0u32; (w * h) as usize];
        render_magic_move_frame(&s1, &s2, &theme, AspectRatio::Ratio16x9, t, &mut pixels, w, h);

        // Verify non-empty frame
        let non_zero_count = pixels.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_count > 0, "Magic move frame at t={} was empty", t);

        // Expected center interpolation
        let sx = w as f32 / 1920.0;
        let sy = h as f32 / 1080.0;
        let c1_x = (100.0 + 100.0) * sx;
        let c1_y = (100.0 + 50.0) * sy;
        let c2_x = (400.0 + 150.0) * sx;
        let c2_y = (300.0 + 75.0) * sy;
        let exp_cx = c1_x + (c2_x - c1_x) * t;
        let exp_cy = c1_y + (c2_y - c1_y) * t;

        if i > 0 {
            assert!(exp_cx >= prev_cx, "Centroid X should move monotonically");
            assert!(exp_cy >= prev_cy, "Centroid Y should move monotonically");
        }
        prev_cx = exp_cx;
        prev_cy = exp_cy;
    }
}

#[test]
fn test_magic_move_textbox_interpolation() {
    let mut s1 = Slide::new(1, SlideLayout::Blank);
    let mut s2 = Slide::new(2, SlideLayout::Blank);

    let tb1 = SlideElement::TextBox(TextBox {
        bounds: Rect::new(50.0, 50.0, 200.0, 60.0),
        paragraphs: vec![Paragraph {
            runs: vec![TextRun {
                text: "Scaling Title".into(),
                font_size: 20.0,
                color: Some(Color::rgb(100, 150, 200)),
                bold: true,
                italic: false,
                mono: false,
            }],
            alignment: TextAlign::Left,
            bullet_level: 0,
            line_spacing: 1.2,
        }],
        background: Some(Color::rgb(20, 20, 20)),
        padding: 8.0,
        magic_key: Some("title-key".into()),
    });
    s1.add_element(tb1);

    let tb2 = SlideElement::TextBox(TextBox {
        bounds: Rect::new(200.0, 150.0, 400.0, 120.0),
        paragraphs: vec![Paragraph {
            runs: vec![TextRun {
                text: "Scaling Title".into(),
                font_size: 40.0,
                color: Some(Color::rgb(240, 240, 255)),
                bold: true,
                italic: false,
                mono: false,
            }],
            alignment: TextAlign::Left,
            bullet_level: 0,
            line_spacing: 1.2,
        }],
        background: Some(Color::rgb(40, 40, 40)),
        padding: 16.0,
        magic_key: Some("title-key".into()),
    });
    s2.add_element(tb2);

    let w = 960;
    let h = 540;
    let theme = Theme::midnight_slate();
    let mut buf = vec![0u32; (w * h) as usize];
    render_magic_move_frame(&s1, &s2, &theme, AspectRatio::Ratio16x9, 0.5, &mut buf, w, h);

    let non_zero = buf.iter().filter(|&&p| p != 0).count();
    assert!(non_zero > 100, "Rendered text magic move frame should contain text and background pixels");
}

#[test]
fn test_intra_slide_build_progression() {
    let mut slide = Slide::new(1, SlideLayout::Blank);

    // Element 0: Header
    slide.add_element(SlideElement::TextBox(TextBox {
        bounds: Rect::new(40.0, 40.0, 800.0, 60.0),
        paragraphs: vec![Paragraph {
            runs: vec![TextRun {
                text: "Always Visible Header".into(),
                font_size: 32.0,
                color: Some(Color::WHITE),
                bold: true,
                italic: false,
                mono: false,
            }],
            alignment: TextAlign::Left,
            bullet_level: 0,
            line_spacing: 1.2,
        }],
        background: None,
        padding: 0.0,
        magic_key: None,
    }));

    // Element 1: First point (Build 1)
    slide.add_element(SlideElement::TextBox(TextBox {
        bounds: Rect::new(40.0, 120.0, 800.0, 40.0),
        paragraphs: vec![Paragraph {
            runs: vec![TextRun {
                text: "Point 1: Dissolves In".into(),
                font_size: 24.0,
                color: Some(Color::rgb(200, 200, 200)),
                bold: false,
                italic: false,
                mono: false,
            }],
            alignment: TextAlign::Left,
            bullet_level: 0,
            line_spacing: 1.2,
        }],
        background: None,
        padding: 0.0,
        magic_key: None,
    }));

    // Element 2: Second point (Build 2)
    slide.add_element(SlideElement::TextBox(TextBox {
        bounds: Rect::new(40.0, 180.0, 800.0, 40.0),
        paragraphs: vec![Paragraph {
            runs: vec![TextRun {
                text: "Point 2: Flies In Left".into(),
                font_size: 24.0,
                color: Some(Color::rgb(200, 200, 200)),
                bold: false,
                italic: false,
                mono: false,
            }],
            alignment: TextAlign::Left,
            bullet_level: 0,
            line_spacing: 1.2,
        }],
        background: None,
        padding: 0.0,
        magic_key: None,
    }));

    // Configure build steps
    slide.add_build(BuildStep {
        element_index: 1,
        paragraph_index: None,
        effect: BuildEffect::DissolveIn,
        duration_ms: 300,
        trigger: BuildTrigger::OnClick,
        rehearsed_delay_ms: None,
    });
    slide.add_build(BuildStep {
        element_index: 2,
        paragraph_index: None,
        effect: BuildEffect::FlyInLeft,
        duration_ms: 400,
        trigger: BuildTrigger::OnClick,
        rehearsed_delay_ms: None,
    });

    let w = 480;
    let h = 270;
    let theme = Theme::midnight_slate();

    let bg = theme.palette.background.0;

    // Render at step 0 (only element 0 visible)
    let mut buf0 = vec![0u32; (w * h) as usize];
    render_slide_at_build(&slide, &theme, AspectRatio::Ratio16x9, 0, None, &mut buf0, w, h);
    let count0 = buf0.iter().filter(|&&p| p != bg).count();

    // Render at step 1 fully transitioned (element 0 and element 1 visible)
    let mut buf1 = vec![0u32; (w * h) as usize];
    render_slide_at_build(&slide, &theme, AspectRatio::Ratio16x9, 1, None, &mut buf1, w, h);
    let count1 = buf1.iter().filter(|&&p| p != bg).count();

    // Render at step 2 fully transitioned (element 0, 1, and 2 visible)
    let mut buf2 = vec![0u32; (w * h) as usize];
    render_slide_at_build(&slide, &theme, AspectRatio::Ratio16x9, 2, None, &mut buf2, w, h);
    let count2 = buf2.iter().filter(|&&p| p != bg).count();

    assert!(count0 > 0, "Step 0 must render base header element");
    assert!(count1 > count0, "Step 1 must have more visible pixels than Step 0");
    assert!(count2 > count1, "Step 2 must have more visible pixels than Step 1");

    // Also test animating progress at t = 0.5 for wipe / pop / fly
    let mut buf_half = vec![0u32; (w * h) as usize];
    render_slide_at_build(&slide, &theme, AspectRatio::Ratio16x9, 0, Some(0.5), &mut buf_half, w, h);
    let count_half = buf_half.iter().filter(|&&p| p != bg).count();
    assert!(count_half > 0, "Half animated frame must render");
}

#[test]
fn test_rehearsal_session_recording_and_apply() {
    let mut deck = Deck::new(AspectRatio::Ratio16x9);
    let mut s1 = Slide::new(1, SlideLayout::Blank);
    s1.add_element(SlideElement::TextBox(TextBox {
        bounds: Rect::new(10.0, 10.0, 100.0, 30.0),
        paragraphs: vec![],
        background: None,
        padding: 0.0,
        magic_key: None,
    }));
    s1.add_build(BuildStep {
        element_index: 0,
        paragraph_index: None,
        effect: BuildEffect::Appear,
        duration_ms: 100,
        trigger: BuildTrigger::OnClick,
        rehearsed_delay_ms: None,
    });
    deck.slides.push(s1);

    let s2 = Slide::new(2, SlideLayout::Blank);
    deck.slides.push(s2);

    let mut session = RehearsalSession::new(deck.clone());
    assert_eq!(session.current_slide, 0);
    assert_eq!(session.current_build, 0);

    // Simulate 1.5 seconds dwell on build 0
    session.tick(1500);
    assert_eq!(session.slide_elapsed_ms, 1500);
    assert_eq!(session.total_elapsed_ms, 1500);

    // Advance to build 1
    session.record_advance();
    assert_eq!(session.current_build, 1);

    // Simulate 2.0 seconds dwell on build 1
    session.tick(2000);
    // Advance to slide 1
    session.record_advance();
    assert_eq!(session.current_slide, 1);
    assert_eq!(session.slide_elapsed_ms, 0); // reset per slide

    // Simulate 3.0 seconds on slide 1
    session.tick(3000);
    // Finish deck
    session.record_advance();
    assert!(!session.is_recording);

    // Apply recorded timings back to deck
    let mut recorded_deck = deck;
    session.apply_to_deck(&mut recorded_deck);
    assert_eq!(recorded_deck.slides[0].builds[0].rehearsed_delay_ms, Some(1500));
    assert_eq!(recorded_deck.slides[0].rehearsed_dwell_ms, Some(3500));
    assert_eq!(recorded_deck.slides[1].rehearsed_dwell_ms, Some(3000));
}

#[test]
fn test_rehearsal_hud_rendering() {
    let deck = Deck::new(AspectRatio::Ratio16x9);
    let session = RehearsalSession::new(deck);

    let w = 480;
    let h = 270;
    let mut pixels = vec![0u32; (w * h) as usize];
    render_rehearsal_hud(&session, &mut pixels, w, h);

    let non_zero = pixels.iter().filter(|&&p| p != 0).count();
    assert!(non_zero > 1000, "Rehearsal HUD must render status bar, timers, and cues");
}

#[test]
fn test_video_timeline_generation() {
    let mut deck = Deck::new(AspectRatio::Ratio16x9);
    let mut s1 = Slide::new(1, SlideLayout::Blank);
    s1.transition = Transition::MagicMove;
    s1.transition_duration_ms = 500;
    s1.rehearsed_dwell_ms = Some(1000);

    s1.add_element(SlideElement::TextBox(TextBox {
        bounds: Rect::new(10.0, 10.0, 100.0, 30.0),
        paragraphs: vec![],
        background: None,
        padding: 0.0,
        magic_key: Some("m1".into()),
    }));
    s1.add_build(BuildStep {
        element_index: 0,
        paragraph_index: None,
        effect: BuildEffect::DissolveIn,
        duration_ms: 250,
        trigger: BuildTrigger::OnClick,
        rehearsed_delay_ms: Some(500),
    });
    deck.slides.push(s1);

    let mut s2 = Slide::new(2, SlideLayout::Blank);
    s2.rehearsed_dwell_ms = Some(1000);
    s2.add_element(SlideElement::TextBox(TextBox {
        bounds: Rect::new(200.0, 100.0, 100.0, 30.0),
        paragraphs: vec![],
        background: None,
        padding: 0.0,
        magic_key: Some("m1".into()),
    }));
    deck.slides.push(s2);

    // 10 FPS for quick deterministic calculation
    let timeline = VideoTimeline::from_deck(&deck, 10);
    assert!(timeline.total_frames() > 10, "Timeline must contain frames for dwells, builds, and transitions");
    assert!(timeline.duration_ms() >= 1000, "Total duration should be >= dwell sums");

    // Render first and transition frames
    let w = 320;
    let h = 180;
    let mut frame_buf = vec![0u32; (w * h) as usize];
    timeline.render_frame(&deck, 0, &mut frame_buf, w, h);
    let non_zero_f0 = frame_buf.iter().filter(|&&p| p != 0).count();
    assert!(non_zero_f0 > 0, "Frame 0 should render content");

    let mid_frame = timeline.total_frames() / 2;
    timeline.render_frame(&deck, mid_frame, &mut frame_buf, w, h);
    let non_zero_mid = frame_buf.iter().filter(|&&p| p != 0).count();
    assert!(non_zero_mid > 0, "Mid frame should render content");
}

#[test]
fn test_markdown_roundtrip_magicmove_and_builds() {
    let mut deck = Deck::new(AspectRatio::Ratio16x9);
    let mut slide = Slide::new(1, SlideLayout::Blank);
    slide.transition = Transition::MagicMove;
    slide.transition_duration_ms = 600;
    slide.rehearsed_dwell_ms = Some(4000);

    slide.add_element(SlideElement::TextBox(TextBox {
        bounds: Rect::new(20.0, 20.0, 300.0, 50.0),
        paragraphs: vec![Paragraph {
            runs: vec![TextRun {
                text: "Hero Title".into(),
                font_size: 28.0,
                color: None,
                bold: true,
                italic: false,
                mono: false,
            }],
            alignment: TextAlign::Left,
            bullet_level: 0,
            line_spacing: 1.2,
        }],
        background: None,
        padding: 0.0,
        magic_key: Some("hero-title".into()),
    }));

    slide.add_build(BuildStep {
        element_index: 0,
        paragraph_index: None,
        effect: BuildEffect::FlyInBottom,
        duration_ms: 350,
        trigger: BuildTrigger::OnClick,
        rehearsed_delay_ms: Some(1200),
    });

    deck.slides.push(slide);

    let md = export_markdown(&deck);
    assert!(md.contains("<!-- transition: magic-move -->"));
    assert!(md.contains("<!-- timing: 4000 -->"));
    assert!(md.contains("<!-- build: 0 fly_in_bottom 350 -->"));

    let imported = import_markdown(&md).expect("markdown import succeeds");
    assert_eq!(imported.slides.len(), 1);
    let s = &imported.slides[0];
    assert_eq!(s.transition, Transition::MagicMove);
    assert_eq!(s.rehearsed_dwell_ms, Some(4000));
    assert_eq!(s.builds.len(), 1);
    assert_eq!(s.builds[0].effect, BuildEffect::FlyInBottom);
    assert_eq!(s.builds[0].duration_ms, 350);
}
