// SPDX-License-Identifier: GPL-3.0-only
use shuttle::{
    is_emoji_char, layout_paragraph, rasterize_emoji, BaseDirection, LayoutOptions, Shaper,
};

#[test]
fn test_mixed_english_arabic_and_numbers() {
    let shaper = Shaper::default();
    let text = "Order #42: السلام عليكم 2026";
    let opts = LayoutOptions {
        max_width: Some(600.0),
        font_size: 16.0,
        base_direction: BaseDirection::Auto,
        ..Default::default()
    };
    let layout = layout_paragraph(&shaper, text, &opts);
    assert!(!layout.lines.is_empty(), "Must produce visual lines");
    let line = &layout.lines[0];
    assert!(line.glyphs.len() >= 10, "Must shape all tokens");

    // Must contain both LTR glyphs (English/numbers) and RTL glyphs (Arabic)
    let has_ltr = line.glyphs.iter().any(|g| !g.is_rtl);
    let has_rtl = line.glyphs.iter().any(|g| g.is_rtl);
    assert!(has_ltr, "Must contain LTR glyphs");
    assert!(has_rtl, "Must contain Arabic RTL glyphs");

    // Hit-testing across the line must succeed with valid offsets
    for x in [0.0, 50.0, 150.0, 250.0, 350.0] {
        let hit = layout.hit_test(x, 8.0);
        assert!(hit.byte_offset <= text.len());
    }

    // Cursor position for various byte offsets
    let c0 = layout.byte_to_cursor(0);
    assert_eq!(c0.x, 0.0);
    assert_eq!(c0.line_index, 0);

    let c_mid = layout.byte_to_cursor(11);
    assert!(c_mid.x >= 0.0);
}

#[test]
fn test_mixed_hebrew_rtl() {
    let shaper = Shaper::default();
    let text = "שלום world";
    let opts = LayoutOptions {
        max_width: Some(300.0),
        font_size: 16.0,
        base_direction: BaseDirection::Auto,
        ..Default::default()
    };
    let layout = layout_paragraph(&shaper, text, &opts);
    assert_eq!(layout.lines.len(), 1);
    let line = &layout.lines[0];

    // Paragraph starts with Hebrew strong character, so paragraph is RTL
    assert!(layout.is_paragraph_rtl, "Paragraph starting with Hebrew should be RTL");

    // Verify Hebrew glyphs have RTL flag set
    let hebrew_glyphs: Vec<_> = line.glyphs.iter().filter(|g| g.is_rtl).collect();
    assert!(!hebrew_glyphs.is_empty(), "Hebrew glyphs must be RTL");
}

#[test]
fn test_arabic_lam_alef_ligature_and_joining() {
    let shaper = Shaper::default();

    // Plain Lam and Alef separate vs combined Lam-Alef
    let text_ligature = "لا"; // U+0644 (Lam) + U+0627 (Alef)
    let opts = LayoutOptions {
        font_size: 18.0,
        ..Default::default()
    };
    let layout = layout_paragraph(&shaper, text_ligature, &opts);
    assert_eq!(layout.lines.len(), 1);
    let glyphs = &layout.lines[0].glyphs;

    // HarfBuzz/rustybuzz with Noto Sans Arabic forms a single ligature glyph for Lam-Alef
    assert_eq!(glyphs.len(), 1, "Lam-Alef must produce a single ligature glyph");
    assert!(glyphs[0].is_rtl);
    assert!(glyphs[0].advance > 0.0);

    // Three-letter Arabic word with medial joining: "كتب" (Kaf, Ta, Ba)
    let text_word = "كتب";
    let layout_word = layout_paragraph(&shaper, text_word, &opts);
    assert_eq!(layout_word.lines.len(), 1);
    let word_glyphs = &layout_word.lines[0].glyphs;
    assert_eq!(word_glyphs.len(), 3, "Kaf, Ta, Ba should produce 3 shaped glyphs");
    assert!(word_glyphs.iter().all(|g| g.is_rtl));
}

#[test]
fn test_cjk_line_breaking() {
    let shaper = Shaper::default();
    let text = "天地玄黄宇宙洪荒";
    // 8 CJK ideographs at 16px. Each em square is ~16px.
    // Setting max_width = 50px forces breaks every 3 characters.
    let opts = LayoutOptions {
        max_width: Some(50.0),
        font_size: 16.0,
        ..Default::default()
    };
    let layout = layout_paragraph(&shaper, text, &opts);
    assert!(
        layout.lines.len() >= 3,
        "CJK text must wrap at character boundaries, got {} lines",
        layout.lines.len()
    );

    // Each line must not exceed max_width
    for (i, line) in layout.lines.iter().enumerate() {
        assert!(
            line.width <= 55.0,
            "Line {} width {} exceeded limit",
            i,
            line.width
        );
    }
}

#[test]
fn test_emoji_fallback_scaling_and_coverage() {
    let emojis = ['😀', '🚀', '✨', '👍', '❤'];
    for &e in &emojis {
        assert!(is_emoji_char(e), "Character {} must be recognized as emoji", e);
        for size in [16, 24, 32, 48] {
            let raster = rasterize_emoji(e as u32, size)
                .unwrap_or_else(|| panic!("Emoji {} must rasterize at {}px", e, size));
            assert_eq!(raster.size, size);
            assert_eq!(raster.coverage.len(), size * size);
            let non_zero = raster.coverage.iter().filter(|&&c| c > 0).count();
            assert!(
                non_zero > 5,
                "Emoji {} at {}px must have non-empty ink coverage (got {})",
                e,
                size,
                non_zero
            );
        }
    }
}

#[test]
fn test_adversarial_inputs_zero_panics() {
    let shaper = Shaper::default();

    // 1. Empty string
    let layout_empty = layout_paragraph(&shaper, "", &LayoutOptions::default());
    assert!(layout_empty.lines.is_empty() || layout_empty.lines[0].glyphs.is_empty());
    let _ = layout_empty.byte_to_cursor(0);
    let _ = layout_empty.hit_test(0.0, 0.0);

    // 2. Huge string
    let huge = "A".repeat(10000);
    let layout_huge = layout_paragraph(&shaper, &huge, &LayoutOptions { max_width: Some(100.0), ..Default::default() });
    assert!(!layout_huge.lines.is_empty());

    // 3. Unpaired surrogate ranges / unassigned code points
    let weird = "\u{0000}\u{0001}\u{001F}\u{007F}\u{0080}\u{00A0}\u{E000}\u{F8FF}\u{10FFFF}";
    let layout_weird = layout_paragraph(&shaper, weird, &LayoutOptions::default());
    assert!(!layout_weird.lines.is_empty());

    // 4. Zero and negative dimensions
    let mut target = vec![0u32; 100];
    layout_weird.draw(&shaper, &mut target, 10, 10, 10, -500, -500, 0xFFFFFFFF);
    layout_weird.draw(&shaper, &mut target, 10, 10, 10, 500, 500, 0xFFFFFFFF);

    // 5. Malformed font bytes to Shaper
    let corrupt_shaper = Shaper::new(b"Not a real TrueType font header at all");
    let layout_corrupt = layout_paragraph(&corrupt_shaper, "Fallback works!", &LayoutOptions::default());
    assert!(!layout_corrupt.lines.is_empty());
}

#[test]
fn test_uax9_l2_visual_run_positions() {
    let shaper = Shaper::default();
    // In RTL paragraph: "שלום world עולם"
    // Run 0: Hebrew "שלום" (level 1)
    // Run 1: Latin " world " (level 2)
    // Run 2: Hebrew "עולם" (level 1)
    // L2 reorders the runs on the line: [Run 2, Run 1, Run 0]
    // Visual X order: Run 2 ("עולם") < Run 1 (" world ") < Run 0 ("שלום")
    let text = "שלום world עולם";
    let opts = LayoutOptions {
        max_width: Some(500.0),
        font_size: 16.0,
        base_direction: BaseDirection::Rtl,
        ..Default::default()
    };
    let layout = layout_paragraph(&shaper, text, &opts);
    assert_eq!(layout.lines.len(), 1, "Must be single line");
    let line = &layout.lines[0];

    let shalom_glyphs: Vec<_> = line.glyphs.iter().filter(|g| g.cluster <= 8).collect();
    let world_glyphs: Vec<_> = line.glyphs.iter().filter(|g| g.cluster >= 9 && g.cluster <= 14).collect();
    let olam_glyphs: Vec<_> = line.glyphs.iter().filter(|g| g.cluster >= 15).collect();

    assert!(!shalom_glyphs.is_empty(), "Shalom glyphs must exist");
    assert!(!world_glyphs.is_empty(), "World glyphs must exist");
    assert!(!olam_glyphs.is_empty(), "Olam glyphs must exist");

    let shalom_min_x = shalom_glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
    let world_min_x = world_glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
    let world_max_x = world_glyphs.iter().map(|g| g.x + g.advance).fold(0.0f32, f32::max);
    let olam_max_x = olam_glyphs.iter().map(|g| g.x + g.advance).fold(0.0f32, f32::max);

    // Assert: visual X of "שלום" > visual X of "world" > visual X of "עולם"
    assert!(
        shalom_min_x >= world_max_x,
        "Shalom (start of RTL sentence) must be to the right of world: shalom_min_x={} vs world_max_x={}",
        shalom_min_x,
        world_max_x
    );
    assert!(
        world_min_x >= olam_max_x,
        "World must be to the right of olam (end of sentence): world_min_x={} vs olam_max_x={}",
        world_min_x,
        olam_max_x
    );
}

#[test]
fn test_visual_line_byte_range_and_cursor() {
    let shaper = Shaper::default();
    // 1-char line
    let layout_single = layout_paragraph(&shaper, "A", &LayoutOptions::default());
    assert_eq!(layout_single.lines.len(), 1);
    assert_eq!(layout_single.lines[0].byte_range, 0..1);

    // Multi-byte CJK
    let layout_cjk = layout_paragraph(&shaper, "世界", &LayoutOptions::default());
    assert_eq!(layout_cjk.lines.len(), 1);
    assert_eq!(layout_cjk.lines[0].byte_range, 0..6);

    // Cursor at start, middle, and end
    let c0 = layout_cjk.byte_to_cursor(0);
    let c3 = layout_cjk.byte_to_cursor(3);
    let c6 = layout_cjk.byte_to_cursor(6);
    assert!(c0.x < c3.x);
    assert!(c3.x < c6.x);
}

#[test]
fn test_mark_positioning_preserves_offset() {
    let shaper = Shaper::default();
    // Arabic text with fathah mark: "بَ" (Ba + Fathah U+064E)
    let text = "بَ";
    let layout = layout_paragraph(&shaper, text, &LayoutOptions::default());
    assert_eq!(layout.lines.len(), 1);
    let glyphs = &layout.lines[0].glyphs;
    assert!(glyphs.len() >= 1);
    // VisualGlyph has x_offset and y
    for g in glyphs {
        assert!(g.x_offset.is_finite());
        assert!(g.y.is_finite());
    }
}

#[test]
fn test_adversarial_picker_and_emoji_buffers() {
    use shuttle::emoji::{blit_color_argb, blit_coverage};
    use shuttle::FontPicker;

    // Zero-length or truncated target buffer for picker
    let picker = FontPicker::new();
    let mut tiny_buf = [0u32; 10];
    picker.render(&mut tiny_buf, 480, 360); // Must return safely without panic

    // Zero-length or truncated buffer for blit_coverage
    let cov = [255u8; 10];
    blit_coverage(&mut tiny_buf, 10, 10, 10, 0, 0, &cov, 20, 20, 0xFFFFFFFF);
    blit_coverage(&mut [], 10, 10, 10, 0, 0, &cov, 2, 2, 0xFFFFFFFF);

    // Zero-length or truncated buffer for blit_color_argb
    let colors = [0xFFFFFFFFu32; 10];
    blit_color_argb(&mut tiny_buf, 10, 10, 10, 0, 0, &colors, 20, 20);
    blit_color_argb(&mut [], 10, 10, 10, 0, 0, &colors, 2, 2);
}

#[test]
fn test_multiline_rtl_wrapping_top_to_bottom() {
    let shaper = Shaper::default();
    // RTL Arabic paragraph with 3 clauses:
    // "كلمة اولى كلمة ثانية كلمة ثالثة"
    let text = "كلمة اولى كلمة ثانية كلمة ثالثة";
    let opts = LayoutOptions {
        max_width: Some(75.0), // Force multi-line wrapping across words
        font_size: 16.0,
        base_direction: BaseDirection::Rtl,
        ..Default::default()
    };
    let layout = layout_paragraph(&shaper, text, &opts);
    assert!(
        layout.lines.len() >= 3,
        "Text must wrap into at least 3 lines, got {}",
        layout.lines.len()
    );

    // Line 0 (top line, y = 0) must contain the first phrase "كلمة اولى" (low cluster IDs)
    let line0 = &layout.lines[0];
    assert!(
        line0.byte_range.start <= 5,
        "Line 0 byte_range must start near 0: {:?}",
        line0.byte_range
    );
    assert!(
        line0.glyphs.iter().any(|g| g.cluster < 19),
        "Line 0 must contain glyphs from the first phrase 'كلمة اولى'"
    );

    // Last line (bottom line) must contain the last phrase "كلمة ثالثة" (high cluster IDs)
    let line_last = layout.lines.last().unwrap();
    assert!(
        line_last.glyphs.iter().any(|g| g.cluster >= 30),
        "Last line must contain glyphs from the final phrase 'كلمة ثالثة'"
    );
}
