// SPDX-License-Identifier: GPL-3.0-only
//! Comprehensive tests for OmaShow features and Cabaret integration:
//! - Object Morph transition engine (`morph.rs`)
//! - Apple Keynote `.key` importer (`keynote.rs`)
//! - Build timeline triggers & Light-Table multi-slide operations (`builds.rs`)
//! - Accessibility / layout lint & toast renderer (`review.rs`)
//! - Headless JSON inspection & atomic batch ops (`ops.rs`)
//! - Live presentation source feed for Cabaret (`live_source.rs`)

use burlesque_core::animation::{BuildEffect, BuildStep, BuildTrigger};
use burlesque_core::builds::{
    add_build_step, delete_slides, duplicate_slides, light_table_cards, move_build_step,
    remove_build_step, render_build_timeline_strip, render_light_table, reorder_slides,
    set_build_effect, set_build_timing, set_build_trigger, set_slides_skipped,
    toggle_slides_skipped, visible_slide_indices, BuildTimeline, Trigger,
};
use burlesque_core::deck::{AspectRatio, Deck, Slide};
use burlesque_core::elements::{
    ImageElement, Paragraph, Shape, ShapeKind, SlideElement, StrokeStyle, TextAlign, TextBox,
    TextRun, Transition,
};
use burlesque_core::keynote::{import_keynote, import_keynote_with_stats};
use burlesque_core::layout::SlideLayout;
use burlesque_core::live_source::{LivePresentationSource, PresentationOverlayMode};
use burlesque_core::morph::{
    compute_morph_plan, evaluate_morph, evaluate_morph_with_easing, render_deck_morph,
    render_morph_frame, ElementMeta, MorphEasing, MorphFrameItem, MorphMatchReason,
};
use burlesque_core::ops::{apply_ops_and_inspect_json, apply_ops_json, inspect_deck_json, JsonValue};
use burlesque_core::render::render_slide_at_build;
use burlesque_core::review::{
    render_review_toast, review_deck, wcag_contrast_ratio, IssueSeverity, ReviewIssueKind,
};
use burlesque_core::theme::Theme;
use sex_archive::{ArchiveBuilder, OutputFormat, VecReader};
use sex_canvas::{Color, Rect};

// ---------------------------------------------------------------------------
// Test fixture helpers (PNG, GZIP, Snappy, Protobuf IWA, ZIP)
// ---------------------------------------------------------------------------

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

fn push_png_chunk(out: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(payload);
    let mut crc_in = Vec::with_capacity(4 + payload.len());
    crc_in.extend_from_slice(tag);
    crc_in.extend_from_slice(payload);
    let crc = crc32fast::hash(&crc_in);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Build a tiny valid RGBA PNG using uncompressed DEFLATE stored blocks.
fn make_tiny_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut raw = Vec::new();
    for _ in 0..height {
        raw.push(0); // filter None
        for _ in 0..width {
            raw.extend_from_slice(&rgba);
        }
    }
    assert!(raw.len() <= u16::MAX as usize);
    let len = raw.len() as u16;
    let nlen = !len;

    let mut zlib = Vec::new();
    zlib.push(0x78); // CMF
    zlib.push(0x01); // FLG
    zlib.push(0x01); // BFINAL=1, BTYPE=00 (stored)
    zlib.extend_from_slice(&len.to_le_bytes());
    zlib.extend_from_slice(&nlen.to_le_bytes());
    zlib.extend_from_slice(&raw);
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    push_png_chunk(&mut png, b"IHDR", &ihdr);
    push_png_chunk(&mut png, b"IDAT", &zlib);
    push_png_chunk(&mut png, b"IEND", &[]);
    png
}

/// Build a valid gzip stream using stored DEFLATE block (`BTYPE=00`).
fn make_stored_gzip(data: &[u8]) -> Vec<u8> {
    assert!(data.len() <= u16::MAX as usize);
    let len = data.len() as u16;
    let nlen = !len;

    let mut gz = Vec::new();
    gz.extend_from_slice(&[0x1F, 0x8B, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xFF]);
    gz.push(0x01); // BFINAL=1, BTYPE=00
    gz.extend_from_slice(&len.to_le_bytes());
    gz.extend_from_slice(&nlen.to_le_bytes());
    gz.extend_from_slice(data);
    gz.extend_from_slice(&crc32fast::hash(data).to_le_bytes());
    gz.extend_from_slice(&(data.len() as u32).to_le_bytes());
    gz
}

fn push_varint(out: &mut Vec<u8>, mut val: u64) {
    loop {
        let mut byte = (val & 0x7F) as u8;
        val >>= 7;
        if val != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if val == 0 {
            break;
        }
    }
}

/// Wrap `stream` in Snappy uncompressed literals and an IWA 4-byte chunk header `[0x00, len24_le]`.
fn encode_iwa_chunk(stream: &[u8]) -> Vec<u8> {
    let mut snappy = Vec::new();
    push_varint(&mut snappy, stream.len() as u64);

    let mut pos = 0usize;
    while pos < stream.len() {
        let chunk = (stream.len() - pos).min(60);
        let tag = ((chunk - 1) as u8) << 2;
        snappy.push(tag);
        snappy.extend_from_slice(&stream[pos..pos + chunk]);
        pos += chunk;
    }

    let len = snappy.len() as u32;
    let mut iwa = Vec::with_capacity(4 + snappy.len());
    iwa.push(0x00);
    iwa.push((len & 0xFF) as u8);
    iwa.push(((len >> 8) & 0xFF) as u8);
    iwa.push(((len >> 16) & 0xFF) as u8);
    iwa.extend_from_slice(&snappy);
    iwa
}

/// Build an IWA protobuf message record (`ArchiveInfo` framing + length-delimited string payloads).
fn make_iwa_text_stream(strings: &[&str]) -> Vec<u8> {
    // Message payload: consecutive field #1 length-delimited strings wrapped inside field #2 nested submessage
    let mut payload = Vec::new();
    for s in strings {
        let bytes = s.as_bytes();
        // field 1, wire type 2 => tag 0x0A
        payload.push(0x0A);
        push_varint(&mut payload, bytes.len() as u64);
        payload.extend_from_slice(bytes);
    }

    // MessageInfo: field 3 (length) = payload.len()
    let mut msg_info = Vec::new();
    push_varint(&mut msg_info, (3 << 3) | 0);
    push_varint(&mut msg_info, payload.len() as u64);

    // ArchiveInfo: field 1 = identifier (1), field 2 = msg_info
    let mut archive_info = Vec::new();
    push_varint(&mut archive_info, (1 << 3) | 0);
    push_varint(&mut archive_info, 1);
    push_varint(&mut archive_info, (2 << 3) | 2);
    push_varint(&mut archive_info, msg_info.len() as u64);
    archive_info.extend_from_slice(&msg_info);

    let mut stream = Vec::new();
    push_varint(&mut stream, archive_info.len() as u64);
    stream.extend_from_slice(&archive_info);
    stream.extend_from_slice(&payload);
    stream
}

fn build_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = ArchiveBuilder::new(OutputFormat::Zip);
    for (name, data) in files {
        let vec = data.to_vec();
        let len = vec.len() as u64;
        builder
            .add_file(name, len, 0, VecReader::new(vec))
            .expect("add_file to zip");
    }
    let mut out = Vec::new();
    builder
        .write(&mut out, &mut sex_archive::no_progress)
        .expect("write zip");
    out
}

fn make_textbox(bounds: Rect, text: &str, font_size: f32, color: Color) -> SlideElement {
    let mut p = Paragraph::new();
    p.alignment = TextAlign::Left;
    p.push_run(TextRun::plain(text, font_size).with_color(color));
    SlideElement::TextBox(TextBox::new(bounds).with_paragraph(p))
}

fn make_shape(
    bounds: Rect,
    kind: ShapeKind,
    fill: Color,
    stroke: Option<StrokeStyle>,
) -> SlideElement {
    SlideElement::Shape(Shape {
        bounds,
        kind,
        fill: Some(fill),
        stroke,
        magic_key: None,
    })
}

// ---------------------------------------------------------------------------
// 1. Object Morph Engine Tests
// ---------------------------------------------------------------------------

#[test]
fn test_morph_matching_and_full_property_interpolation() {
    let mut s1 = Slide::new(1, SlideLayout::Blank);
    let mut s2 = Slide::new(2, SlideLayout::Blank);

    // 0: explicit morph_key
    s1.add_element(
        make_shape(
            Rect::new(100.0, 100.0, 200.0, 100.0),
            ShapeKind::RoundedRect { radius: 8.0 },
            Color::rgba(0, 0, 200, 255),
            Some(StrokeStyle {
                color: Color::rgb(10, 20, 30),
                width: 2.0,
            }),
        )
        .with_morph_key("card"),
    );
    s1.set_element_rotation(0, 0.0);
    s1.set_element_opacity(0, 1.0);

    s2.add_element(
        make_shape(
            Rect::new(500.0, 300.0, 400.0, 200.0),
            ShapeKind::RoundedRect { radius: 28.0 },
            Color::rgba(200, 0, 0, 255),
            Some(StrokeStyle {
                color: Color::rgb(210, 220, 230),
                width: 6.0,
            }),
        )
        .with_morph_key("card"),
    );
    s2.set_element_rotation(0, 90.0);
    s2.set_element_opacity(0, 0.5);

    // 1: element_id match
    s1.add_element(make_textbox(
        Rect::new(120.0, 40.0, 400.0, 60.0),
        "Title Alpha",
        24.0,
        Color::WHITE,
    ));
    s1.set_element_id(1, Some("hdr-id".into()));

    s2.add_element(make_textbox(
        Rect::new(320.0, 80.0, 600.0, 100.0),
        "Title Beta",
        48.0,
        Color::rgb(200, 220, 255),
    ));
    s2.set_element_id(1, Some("hdr-id".into()));

    // 2: Automatic text/label match (no key, same text)
    s1.add_element(make_textbox(
        Rect::new(100.0, 400.0, 300.0, 50.0),
        "Persistent Subtitle",
        20.0,
        Color::WHITE,
    ));
    // Unmatched shape on s1 (2.5 -> index 3) to test fade-out
    s1.add_element(make_shape(
        Rect::new(900.0, 100.0, 80.0, 80.0),
        ShapeKind::Ellipse,
        Color::rgb(0, 255, 0),
        None,
    ));

    // Matched label at idx 2 on s2, plus new unmatched shape at idx 3 on s2 (fade-in)
    s2.add_element(make_textbox(
        Rect::new(1000.0, 400.0, 300.0, 50.0),
        "Persistent Subtitle",
        28.0,
        Color::WHITE,
    ));
    s2.add_element(make_shape(
        Rect::new(1200.0, 600.0, 120.0, 120.0),
        ShapeKind::Rect,
        Color::rgb(255, 200, 0),
        None,
    ));

    let theme = Theme::midnight_slate();
    let plan = compute_morph_plan(&s1, &s2);
    assert!(plan.pairs.iter().any(|p| p.from_index == 0
        && p.to_index == 0
        && p.reason == MorphMatchReason::MorphKey));
    assert!(plan.pairs.iter().any(|p| p.from_index == 1
        && p.to_index == 1
        && p.reason == MorphMatchReason::ElementId));
    assert!(plan.pairs.iter().any(|p| p.from_index == 2
        && p.to_index == 2
        && p.reason == MorphMatchReason::TextAffinity));
    assert!(plan.unmatched_from.contains(&3));
    assert!(plan.unmatched_to.contains(&3));

    // Linear easing exact midpoint verification at t = 0.5
    let items = evaluate_morph_with_easing(&s1, &s2, &theme, 0.5, MorphEasing::Linear);
    let card_state = items
        .iter()
        .find_map(|it| match it {
            MorphFrameItem::Matched { from_index: 0, state, .. } => Some(state),
            _ => None,
        })
        .unwrap();
    assert!((card_state.bounds.x - 300.0).abs() < 0.01);
    assert!((card_state.bounds.y - 200.0).abs() < 0.01);
    assert!((card_state.bounds.w - 300.0).abs() < 0.01);
    assert!((card_state.bounds.h - 150.0).abs() < 0.01);
    assert!((card_state.rotation_deg - 45.0).abs() < 0.01);
    assert!((card_state.corner_radius - 18.0).abs() < 0.01);
    assert!((card_state.stroke.unwrap().width - 4.0).abs() < 0.01);
    assert!((card_state.opacity - 0.75).abs() < 0.01);
    let fill = card_state.fill.unwrap();
    assert!((fill.r() as i32 - 100).abs() <= 1);
    assert!((fill.b() as i32 - 100).abs() <= 1);

    let title_state = items
        .iter()
        .find_map(|it| match it {
            MorphFrameItem::Matched { from_index: 1, state, .. } => Some(state),
            _ => None,
        })
        .unwrap();
    assert!((title_state.font_size - 36.0).abs() < 0.01);

    // Unmatched elements fade out / fade in cleanly
    let faded_out = items
        .iter()
        .find_map(|it| match it {
            MorphFrameItem::FadeOut { from_index: 3, state } => Some(state),
            _ => None,
        })
        .unwrap();
    assert!((faded_out.opacity - 0.5).abs() < 0.01);

    let faded_in = items
        .iter()
        .find_map(|it| match it {
            MorphFrameItem::FadeIn { to_index: 3, state } => Some(state),
            _ => None,
        })
        .unwrap();
    assert!((faded_in.opacity - 0.5).abs() < 0.01);

    // Verify rendering actually draws rotated + morphed pixels at t=0.5 without panic
    let mut deck = Deck::new(AspectRatio::Ratio16x9);
    s1.transition = Transition::Morph;
    deck.slides.push(s1.clone());
    deck.slides.push(s2.clone());
    let mut buf_mid = vec![0u32; 480 * 270];
    let mut buf_start = vec![0u32; 480 * 270];
    render_morph_frame(&s1, &s2, &deck.theme, deck.aspect_ratio, 0.0, &mut buf_start, 480, 270);
    render_deck_morph(&deck, 0, 1, 0.5, &mut buf_mid, 480, 270);
    assert_ne!(buf_start, buf_mid);
    let _ = evaluate_morph(&s1, &s2, &theme, 0.25);
    let _ = ElementMeta::default();
}

// ---------------------------------------------------------------------------
// 2. Apple Keynote `.key` Importer Tests
// ---------------------------------------------------------------------------

#[test]
fn test_keynote_import_modern_iwa_with_snappy_and_images() {
    let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>title</key><string>Keynote Q4 Strategy</string>
  <key>author</key><string>Carbon Studio</string>
  <key>comment</key><string>Executive roadmap deck</string>
</dict>
</plist>"#;

    let s1_stream = make_iwa_text_stream(&["Keynote Q4 Strategy", "Growth Targets"]);
    let s1_iwa = encode_iwa_chunk(&s1_stream);

    let s2_stream = make_iwa_text_stream(&["Architecture", "Snappy IWA Decoded"]);
    let s2_iwa = encode_iwa_chunk(&s2_stream);

    let png = make_tiny_png(4, 4, [0x22, 0x88, 0xFF, 0xFF]);

    let key_bytes = build_zip(&[
        ("Metadata/Properties.plist", plist.as_bytes()),
        ("Index/Slide-1.iwa", &s1_iwa),
        ("Index/Slide-2.iwa", &s2_iwa),
        ("Data/diagram.png", &png),
    ]);

    let (deck, stats) = import_keynote_with_stats(&key_bytes).expect("import modern .key");
    assert_eq!(deck.metadata.title, "Keynote Q4 Strategy");
    assert_eq!(deck.metadata.author, "Carbon Studio");
    assert_eq!(deck.metadata.description, "Executive roadmap deck");
    assert_eq!(stats.slide_count, 2);
    assert_eq!(stats.text_slides, 2);
    assert_eq!(stats.extracted_images, 1);
    assert!(!stats.used_preview_fallback);
    assert_eq!(deck.slides[0].title, "Keynote Q4 Strategy");
    assert_eq!(deck.slides[1].title, "Architecture");
    assert!(deck.slides[0]
        .elements
        .iter()
        .any(|e| matches!(e, SlideElement::Image(_))));
}

#[test]
fn test_keynote_import_legacy_apxl_gz_and_preview_fallback_and_reject_corrupt() {
    let apxl = r#"<?xml version="1.0" encoding="UTF-8"?>
<key:presentation xmlns:key="http://developer.apple.com/namespaces/keynote2">
  <key:slide title="Legacy Opening">
    <key:p>Legacy iWork Slide</key:p>
    <key:p>Imported seamlessly</key:p>
    <key:notes><key:p>Speak clearly here</key:p></key:notes>
    <key:image href="Data/photo.png"/>
  </key:slide>
</key:presentation>"#;

    let gz_apxl = make_stored_gzip(apxl.as_bytes());
    let png = make_tiny_png(4, 4, [0xFF, 0x44, 0x11, 0xFF]);

    let legacy_zip = build_zip(&[
        ("index.apxl.gz", &gz_apxl),
        ("Data/photo.png", &png),
    ]);

    let deck = import_keynote(&legacy_zip).expect("import legacy .key with apxl.gz");
    assert_eq!(deck.slide_count(), 1);
    assert_eq!(deck.slides[0].title, "Legacy iWork Slide");
    assert!(deck.slides[0].notes().contains("Speak clearly here"));
    assert!(deck.slides[0]
        .elements
        .iter()
        .any(|e| matches!(e, SlideElement::Image(_))));

    // Preview fallback when no text is present in IWA
    let plist_only = r#"<plist><dict><key>title</key><string>Visual Deck</string></dict></plist>"#;
    let empty_iwa = encode_iwa_chunk(&[0x08, 0x01]);
    let preview_zip = build_zip(&[
        ("Metadata/Properties.plist", plist_only.as_bytes()),
        ("Index/Document.iwa", &empty_iwa),
        ("preview.png", &png),
    ]);
    let (prev_deck, prev_stats) =
        import_keynote_with_stats(&preview_zip).expect("import preview fallback .key");
    assert!(prev_stats.used_preview_fallback);
    assert_eq!(prev_deck.slide_count(), 1);
    assert_eq!(prev_deck.slides[0].title, "Visual Deck");
    assert!(prev_deck.slides[0]
        .elements
        .iter()
        .any(|e| matches!(e, SlideElement::Image(_))));

    // Reject non-ZIP / non-Keynote garbage without panicking
    assert!(import_keynote(b"not a zip at all").is_err());
    let non_keynote_zip = build_zip(&[("random.txt", b"hello")]);
    assert!(import_keynote(&non_keynote_zip).is_err());
}

// ---------------------------------------------------------------------------
// 3. Build Timeline & Light-Table Operations Tests
// ---------------------------------------------------------------------------

#[test]
fn test_builds_triggers_disappear_and_timeline_mutations() {
    let mut slide = Slide::new(1, SlideLayout::Blank);
    slide.add_element(make_shape(
        Rect::new(40.0, 40.0, 120.0, 80.0),
        ShapeKind::Rect,
        Color::rgb(255, 0, 0),
        None,
    ));
    slide.add_element(make_shape(
        Rect::new(200.0, 40.0, 120.0, 80.0),
        ShapeKind::Rect,
        Color::rgb(0, 255, 0),
        None,
    ));
    slide.add_element(make_shape(
        Rect::new(360.0, 40.0, 120.0, 80.0),
        ShapeKind::Rect,
        Color::rgb(0, 0, 255),
        None,
    ));

    // Step 0: OnClick element 0 Fade 300ms (with 50ms delay)
    let b0 = add_build_step(&mut slide, 0, BuildEffect::Fade, Trigger::OnClick, 50, 300);
    // Step 1: WithPrevious element 1 Scale 500ms delay 100ms
    let b1 = add_build_step(&mut slide, 1, BuildEffect::Scale, Trigger::WithPrevious, 100, 500);
    // Step 2: AfterPrevious element 2 Wipe 200ms delay 50ms
    let b2 = add_build_step(&mut slide, 2, BuildEffect::Wipe, Trigger::AfterPrevious, 50, 200);
    assert_eq!((b0, b1, b2), (0, 1, 2));

    // Mutate step 2 to Disappear and verify move/timing/trigger helpers
    assert!(set_build_effect(&mut slide, 2, BuildEffect::Disappear));
    assert!(set_build_timing(&mut slide, 2, 75, 250));
    assert!(set_build_trigger(&mut slide, 2, Trigger::AfterPrevious, 75));

    let tl = BuildTimeline::from_slide(&slide);
    assert_eq!(tl.click_group_count, 1);
    assert_eq!(tl.items[0].click_group, 0);
    assert_eq!(tl.items[0].start_ms, 50);
    assert_eq!(tl.items[0].end_ms, 350);
    assert_eq!(tl.items[1].click_group, 0); // WithPrevious shares click_group 0!
    assert_eq!(tl.items[1].start_ms, 50 + 100);
    assert_eq!(tl.items[1].end_ms, 150 + 500);
    // AfterPrevious starts after step 1 end_ms (650) + delay (75) = 725
    assert_eq!(tl.items[2].start_ms, 725);
    assert_eq!(tl.items[2].end_ms, 975);
    assert_eq!(tl.total_duration_ms, 975);

    // Verify rendering WithPrevious group & Disappear effect:
    // Swap target of Disappear step to element 0 for disappear visibility test
    slide.builds[2].element_index = 0;
    let theme = Theme::midnight_slate();
    let mut b_before = vec![0u32; 320 * 180];
    render_slide_at_build(&slide, &theme, AspectRatio::Ratio16x9, 0, None, &mut b_before, 320, 180);
    // Element 0 has an entrance AND an exit build -> hidden before step 0 completes, visible at step 2, hidden again after step 3!
    let mut b_after_enter = vec![0u32; 320 * 180];
    render_slide_at_build(
        &slide,
        &theme,
        AspectRatio::Ratio16x9,
        2,
        None,
        &mut b_after_enter,
        320,
        180,
    );
    let mut b_after_disappear = vec![0u32; 320 * 180];
    render_slide_at_build(
        &slide,
        &theme,
        AspectRatio::Ratio16x9,
        3,
        None,
        &mut b_after_disappear,
        320,
        180,
    );
    assert!(b_before != b_after_enter);
    assert!(b_after_disappear != b_after_enter);
    // Element 0 center (100, 80) in 1920x1080 -> pixel (16, 13) in 320x180
    let e0_px = 13 * 320 + 16;
    assert_eq!(b_after_enter[e0_px], Color::rgb(255, 0, 0).0);
    assert_eq!(
        b_after_disappear[e0_px], b_before[e0_px],
        "Disappear should hide element 0 at step 3"
    );
    // Element 1 center (260, 80) in 1920x1080 -> pixel (43, 13) stays visible
    let e1_px = 13 * 320 + 43;
    assert_eq!(b_after_enter[e1_px], Color::rgb(0, 255, 0).0);
    assert_eq!(b_after_disappear[e1_px], Color::rgb(0, 255, 0).0);

    // Reorder and remove build step
    assert!(move_build_step(&mut slide, 2, 1));
    assert_eq!(slide.builds[1].effect, BuildEffect::Disappear);
    assert!(remove_build_step(&mut slide, 1).is_some());
    assert_eq!(slide.builds.len(), 2);

    // Strip renderer produces pixels
    let mut strip = vec![0u32; 480 * 72];
    render_build_timeline_strip(&slide, 0, &mut strip, 480, 72, 72);
    assert!(strip.iter().any(|&p| p != strip[0]));
    let _ = BuildStep::new(0, BuildEffect::FlyIn);
    let _ = BuildTrigger::OnClick;
}

#[test]
fn test_light_table_reorder_duplicate_skip_delete_and_render() {
    let mut deck = Deck::new(AspectRatio::Ratio16x9);
    for (i, title) in ["Intro", "Agenda", "Demo", "Wrap"].iter().enumerate() {
        let mut s = Slide::new((i + 1) as u64, SlideLayout::Blank);
        s.title = (*title).into();
        s.add_element(make_textbox(
            Rect::new(100.0, 100.0, 600.0, 80.0),
            title,
            32.0,
            Color::WHITE,
        ));
        deck.slides.push(s);
    }

    // Duplicate slides 1 and 3 -> inserted sequentially with updated titles/ids
    let dups = duplicate_slides(&mut deck, &[1, 3]);
    assert_eq!(dups.len(), 2);
    assert_eq!(deck.slide_count(), 6);

    // Skip slides 2 and 4
    assert_eq!(set_slides_skipped(&mut deck, &[2, 4], true), 2);
    let vis = visible_slide_indices(&deck);
    assert_eq!(vis, vec![0, 1, 3, 5]);
    assert_eq!(toggle_slides_skipped(&mut deck, &[2]), 1);
    assert!(!deck.slides[2].hidden);

    // Multi-slide reorder [0, 2] to target index 4
    let moved = reorder_slides(&mut deck, &[0, 2], 4);
    assert_eq!(moved.len(), 2);
    assert_eq!(deck.slides[moved[0]].title, "Intro");

    let cards = light_table_cards(&deck);
    assert_eq!(cards.len(), 6);
    assert_eq!(cards[moved[0]].title, "Intro");

    // Render light-table grid
    let mut lt_buf = vec![0u32; 640 * 360];
    render_light_table(&deck, &moved, 0, &mut lt_buf, 640, 360);
    let unique: std::collections::HashSet<u32> = lt_buf.iter().copied().collect();
    assert!(unique.len() > 6);

    // Delete slides leaves at least 1 slide when keep_one = true
    let removed = delete_slides(&mut deck, &[0, 1, 2, 3, 4, 5], true);
    assert_eq!(removed, 5);
    assert_eq!(deck.slide_count(), 1);
}

// ---------------------------------------------------------------------------
// 4. Deck Review & Headless JSON Engine Tests
// ---------------------------------------------------------------------------

#[test]
fn test_review_deck_detects_all_lint_categories_and_renders_toast() {
    let mut deck = Deck::new(AspectRatio::Ratio16x9);
    deck.set_theme(Theme::editorial_light()); // light background ~ #FAF8F5

    let mut bad_slide = Slide::new(1, SlideLayout::Blank);
    bad_slide.title = "Lint Slide".into();
    // 1. Bounds overflow (x = -40)
    bad_slide.add_element(make_shape(
        Rect::new(-40.0, 20.0, 200.0, 100.0),
        ShapeKind::Rect,
        Color::rgb(20, 40, 180),
        None,
    ));
    // 2. Low contrast text (near-white #E6E6E6 on editorial_light)
    bad_slide.add_element(make_textbox(
        Rect::new(120.0, 160.0, 500.0, 80.0),
        "Hard to read light gray text",
        20.0,
        Color::rgb(230, 230, 230),
    ));
    // 3. Empty placeholder
    bad_slide.add_element(make_textbox(
        Rect::new(120.0, 300.0, 500.0, 80.0),
        "Click to add subtitle",
        20.0,
        Color::rgb(15, 23, 42),
    ));
    // 4. Missing/corrupt image
    bad_slide.add_element(SlideElement::Image(ImageElement::new(
        Rect::new(800.0, 200.0, 200.0, 200.0),
        b"corrupt-img-bytes".to_vec(),
        burlesque_core::elements::FitMode::Contain,
    )));
    deck.slides.push(bad_slide);

    let report = review_deck(&deck);
    assert!(!report.is_clean());
    assert!(report.has_errors());
    assert!(report
        .issues
        .iter()
        .any(|i| i.kind == ReviewIssueKind::BoundsOverflow));
    assert!(report
        .issues
        .iter()
        .any(|i| i.kind == ReviewIssueKind::LowContrast));
    assert!(report
        .issues
        .iter()
        .any(|i| i.kind == ReviewIssueKind::EmptyPlaceholder));
    assert!(report
        .issues
        .iter()
        .any(|i| i.kind == ReviewIssueKind::MissingImage && i.severity == IssueSeverity::Error));

    assert!(wcag_contrast_ratio(Color::BLACK, Color::WHITE) > 20.0);

    // Verify toast overlays onto canvas when issues present
    let canvas_before = vec![0xFF101820u32; 640 * 360];
    let mut canvas_after = canvas_before.clone();
    render_review_toast(&report, &mut canvas_after, 640, 360);
    assert_ne!(canvas_before, canvas_after);

    // Clean slide should report clean
    let mut clean_deck = Deck::new(AspectRatio::Ratio16x9);
    clean_deck.set_theme(Theme::editorial_light());
    let mut clean_s = Slide::new(1, SlideLayout::Blank);
    clean_s.title = "Readable".into();
    clean_s.add_element(make_textbox(
        Rect::new(120.0, 120.0, 600.0, 80.0),
        "Sharp High Contrast Headline",
        36.0,
        Color::rgb(15, 23, 42),
    ));
    clean_deck.slides.push(clean_s);
    let clean_report = review_deck(&clean_deck);
    assert!(
        clean_report.is_clean(),
        "expected clean deck, got {:?}",
        clean_report.issues
    );

    // Clean toast renders a distinct green badge compared to the red error toast
    let mut clean_canvas = canvas_before.clone();
    render_review_toast(&clean_report, &mut clean_canvas, 640, 360);
    assert_ne!(clean_canvas, canvas_before);
    assert_ne!(clean_canvas, canvas_after);
}

#[test]
fn test_ops_json_inspect_apply_and_atomic_rollback() {
    let mut deck = Deck::new(AspectRatio::Ratio16x9);

    let ops = r##"[
      { "op": "set_metadata", "title": "OmaShow JSON Deck", "author": "Agent" },
      { "op": "set_theme", "name": "midnight_slate" },
      { "op": "add_slide", "title": "Opening", "body": "First line", "transition": "morph", "notes": "Say hi" },
      { "op": "add_shape", "slide_index": 0, "shape": "rounded_rect", "corner_radius": 24, "x": 100, "y": 500, "w": 220, "h": 140, "fill": "#22C55E", "morph_key": "badge", "rotation_deg": 15, "opacity": 0.9 },
      { "op": "add_build", "slide_index": 0, "element_index": 2, "effect": "fade", "trigger": "on_click", "delay_ms": 50, "duration_ms": 350 },
      { "op": "duplicate_slides", "slide_index": 0 },
      { "op": "update_element", "slide_index": 1, "element_index": 2, "x": 900, "rotation_deg": 45, "corner_radius": 60 },
      { "op": "set_slide_skipped", "indices": [1], "skipped": false }
    ]"##;

    let res = apply_ops_json(&mut deck, ops).expect("apply ops batch");
    assert_eq!(res.applied_ops, 8);
    assert_eq!(res.slide_count, 2);
    assert_eq!(deck.metadata.title, "OmaShow JSON Deck");
    assert_eq!(deck.theme.name, "Midnight Slate");
    assert_eq!(deck.slides[0].transition, Transition::Morph);
    assert_eq!(deck.slides[1].element_meta(2).rotation_deg, 45.0);

    // inspect_deck_json determinism and valid JSON parsing
    let snap1 = inspect_deck_json(&deck);
    let snap2 = apply_ops_and_inspect_json(&mut deck, "[]").expect("noop inspect");
    assert_eq!(snap1, snap2);
    let parsed = JsonValue::parse(&snap1).expect("valid inspect JSON");
    assert_eq!(parsed.get("slide_count").unwrap().as_usize(), Some(2));

    // Atomic rollback if operation #2 in batch fails
    let bad_batch = r#"[
      { "op": "set_metadata", "title": "SHOULD ROLLBACK" },
      { "op": "remove_element", "slide_index": 99, "element_index": 0 }
    ]"#;
    let err = apply_ops_json(&mut deck, bad_batch).unwrap_err();
    assert_eq!(err.op_index, Some(1));
    assert_eq!(inspect_deck_json(&deck), snap1);
}

// ---------------------------------------------------------------------------
// 5. LivePresentationSource Tests (Cabaret Wave 3 Bridge)
// ---------------------------------------------------------------------------

#[test]
fn test_live_presentation_source_stepping_tick_notes_and_overlay_modes() {
    let mut deck = Deck::new(AspectRatio::Ratio16x9);
    deck.set_theme(Theme::midnight_slate());

    // Slide 0: Morph transition, 2 visible click groups (step 0 + step 1 WithPrevious, then step 2)
    let mut s0 = Slide::new(1, SlideLayout::Blank);
    s0.title = "Cabaret Keynote Slide".into();
    s0.set_speaker_notes("Greet audience and introduce Carbon stream.");
    s0.transition = Transition::Morph;
    s0.transition_duration_ms = 400;
    s0.add_element(
        make_shape(
            Rect::new(80.0, 120.0, 260.0, 140.0),
            ShapeKind::RoundedRect { radius: 16.0 },
            Color::rgb(59, 130, 246),
            None,
        )
        .with_morph_key("hero"),
    );
    s0.add_element(make_textbox(
        Rect::new(400.0, 140.0, 700.0, 80.0),
        "Live Bullet Point 1",
        32.0,
        Color::WHITE,
    ));
    s0.add_element(make_textbox(
        Rect::new(400.0, 260.0, 700.0, 80.0),
        "Live Bullet Point 2",
        32.0,
        Color::WHITE,
    ));
    add_build_step(&mut s0, 1, BuildEffect::Fade, Trigger::OnClick, 0, 200);
    add_build_step(&mut s0, 2, BuildEffect::Scale, Trigger::WithPrevious, 0, 250);
    deck.slides.push(s0);

    // Slide 1: Skipped!
    let mut s1 = Slide::new(2, SlideLayout::Blank);
    s1.title = "Hidden Draft".into();
    s1.hidden = true;
    deck.slides.push(s1);

    // Slide 2: Second visible slide
    let mut s2 = Slide::new(3, SlideLayout::Blank);
    s2.title = "Finale".into();
    s2.set_speaker_notes("Wrap up live demo.");
    s2.add_element(
        make_shape(
            Rect::new(500.0, 320.0, 420.0, 220.0),
            ShapeKind::RoundedRect { radius: 24.0 },
            Color::rgb(16, 185, 129),
            None,
        )
        .with_morph_key("hero"),
    );
    deck.slides.push(s2);

    let mut live = LivePresentationSource::new(deck);
    assert_eq!(live.state(), (0, 3, 0, 2));
    assert_eq!(live.presenter_notes(), "Greet audience and introduce Carbon stream.");

    // Advance click step -> executes builds 0 & 1 together because build 1 is WithPrevious
    assert!(live.next_step());
    assert_eq!(live.state(), (0, 3, 2, 2));
    assert!(live.is_animating());
    assert!(live.tick(120));
    assert!(live.tick(200));
    assert!(!live.is_animating());

    // Advance past end of slide 0 -> skips slide 1 -> starts Morph transition to slide 2
    assert!(live.next_step());
    assert_eq!(live.state(), (2, 3, 0, 0));
    assert_eq!(live.presenter_notes(), "Wrap up live demo.");
    assert!(live.is_animating());
    assert!(live.tick(200)); // mid-morph

    // FullSlide: all pixels have alpha == 255
    let full_rgba = live.render_rgba(320, 180);
    assert_eq!(full_rgba.len(), 320 * 180 * 4);
    assert!(full_rgba.chunks_exact(4).all(|px| px[3] == 255));

    // TransparentSlide: corners outside elements have alpha == 0, morphed shape pixels have alpha > 0
    live.set_overlay_mode(PresentationOverlayMode::TransparentSlide);
    let trans_rgba = live.render_rgba(320, 180);
    assert_eq!(trans_rgba[3], 0);
    assert!(trans_rgba.chunks_exact(4).any(|px| px[3] > 0));

    // Complete animation, switch to LowerThird mode
    live.flush_animations();
    live.set_overlay_mode(PresentationOverlayMode::LowerThird);
    let mut lt_rgba = vec![0u8; 640 * 360 * 4];
    assert!(live.render_rgba_into(&mut lt_rgba, 640, 360));
    // Upper 70% must be transparent [0, 0, 0, 0]
    let upper_rows = (360usize * 70) / 100;
    assert!(
        lt_rgba[..upper_rows * 640 * 4]
            .chunks_exact(4)
            .all(|px| px == [0, 0, 0, 0]),
        "LowerThird upper 70% must be transparent"
    );
    // Lower band must have non-zero drawn pixels
    assert!(lt_rgba[upper_rows * 640 * 4..]
        .chunks_exact(4)
        .any(|px| px[3] > 0));

    // Navigate backward -> skips hidden slide 1 back to slide 0
    assert!(live.prev_step());
    assert_eq!(live.slide_index(), 0);
    assert_eq!(live.step_index(), 2);
    assert!(live.prev_step());
    assert_eq!(live.step_index(), 0);
    assert!(live.goto_slide(2));
    assert_eq!(live.slide_index(), 2);
}
