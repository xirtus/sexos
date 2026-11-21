// SPDX-License-Identifier: GPL-3.0-only
//! Burlesque Weft window host tests: slides, layout, editing, themes, transitions,
//! speaker notes, presenter view, PPTX/ODP/PDF import/export, and software rendering.

use burlesque::weft;
use burlesque::weft::input::{Button, Event, Pointer, PointerKind};
use burlesque::weft::{Response, Strings, Theme, Ui};
use burlesque::Burlesque;
use burlesque_core::elements::Transition;
use burlesque_core::layout::SlideLayout;
use burlesque_core::Theme as BurlesqueTheme;

fn click_widget(ui: &mut Ui, b: &mut Burlesque, id: weft::widget::Id) {
    ui.layout();
    let c = ui.rect(id).center();
    let _ = b.step(
        ui,
        Event::Pointer(Pointer {
            x: c.x,
            y: c.y,
            kind: PointerKind::Move,
        }),
    );
    let _ = b.step(
        ui,
        Event::Pointer(Pointer {
            x: c.x,
            y: c.y,
            kind: PointerKind::Down(Button::Left),
        }),
    );
    let _ = b.step(
        ui,
        Event::Pointer(Pointer {
            x: c.x,
            y: c.y,
            kind: PointerKind::Up(Button::Left),
        }),
    );
}

#[test]
fn test_burlesque_window_interactions() {
    let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
    let mut b = Burlesque::build(&mut ui);
    ui.layout();

    // 1. Initial State
    assert_eq!(b.deck.slide_count(), 1);
    assert_eq!(b.current_slide, 0);
    assert_eq!(b.presenter_mode, false);
    assert_eq!(b.theme_slug(), "burlesque_velvet");
    assert_eq!(b.current_transition(), Transition::None);
    assert_eq!(
        b.deck.get_slide(0).unwrap().notes(),
        "Welcome to Burlesque presentations on SexOS: themes, transitions, presenter view, and full PPTX/ODP/PDF workflows."
    );

    // 2. Add Slide
    let btn_add = b.btn_add_slide;
    click_widget(&mut ui, &mut b, btn_add);
    assert_eq!(b.deck.slide_count(), 2);
    assert_eq!(b.current_slide, 1);

    // 3. Navigation: Prev and Next
    let btn_prev = b.btn_prev;
    click_widget(&mut ui, &mut b, btn_prev);
    assert_eq!(b.current_slide, 0);

    let btn_next = b.btn_next;
    click_widget(&mut ui, &mut b, btn_next);
    assert_eq!(b.current_slide, 1);

    // 4. Switch Layout to Two Column
    let btn_twocol = b.btn_layout_twocol;
    click_widget(&mut ui, &mut b, btn_twocol);
    assert_eq!(
        b.deck.get_slide(b.current_slide).unwrap().layout,
        SlideLayout::TwoColumn
    );

    // 5. Edit Slide Title, Body, and Speaker Notes
    let field_title = b.field_title;
    ui.set_text(field_title, "SexOS Architecture");
    ui.set_text(b.field_body, "- Microkernel\n- PKU Domains\n---\n- Weft Toolkit\n- Zero-copy PDX");
    ui.set_text(b.field_notes, "Explain domain protection keys and zero-copy IPC.");
    let _ = b.on_response(&mut ui, &Response::Changed(field_title));

    let current = b.deck.get_slide(b.current_slide).unwrap();
    assert_eq!(current.title, "SexOS Architecture");
    assert_eq!(current.elements.len(), 3); // Title, Col1, Col2
    assert_eq!(current.notes(), "Explain domain protection keys and zero-copy IPC.");

    // 6. Theme Switching and Cycling
    let btn_theme = b.btn_theme;
    click_widget(&mut ui, &mut b, btn_theme);
    assert_eq!(b.theme_slug(), "editorial_light");

    b.set_theme(BurlesqueTheme::midnight_slate(), &mut ui);
    assert_eq!(b.theme_slug(), "midnight_slate");

    b.set_theme(BurlesqueTheme::cyber_prism(), &mut ui);
    assert_eq!(b.theme_slug(), "cyber_prism");

    // 7. Transition Selection and Animated Playback
    let btn_transition = b.btn_transition;
    click_widget(&mut ui, &mut b, btn_transition);
    assert_eq!(b.current_transition(), Transition::Dissolve);

    let btn_play = b.btn_play_transition;
    click_widget(&mut ui, &mut b, btn_play);
    let (trans, frames) = b.play_transition(&mut ui);
    assert_eq!(trans, Transition::Dissolve);
    assert_eq!(frames, 8);

    b.set_transition(Transition::PushLeft);
    assert_eq!(b.current_transition(), Transition::PushLeft);
    let (trans, frames) = b.play_transition(&mut ui);
    assert_eq!(trans, Transition::PushLeft);
    assert_eq!(frames, 8);

    // 8. Presenter View Toggle & Speaker Notes Rendering
    let btn_pres = b.btn_presenter;
    click_widget(&mut ui, &mut b, btn_pres);
    assert_eq!(b.presenter_mode, true);
    assert!(b.presenter_session.is_some());
    assert_eq!(
        b.presenter_session.as_ref().unwrap().current_notes(),
        "Explain domain protection keys and zero-copy IPC."
    );

    // Toggle back
    click_widget(&mut ui, &mut b, btn_pres);
    assert_eq!(b.presenter_mode, false);
    assert!(b.presenter_session.is_none());

    // 9. PPTX Export & Import Roundtrip
    let pptx_bytes = b.export_pptx().expect("pptx export");
    assert!(pptx_bytes.starts_with(b"PK\x03\x04"));
    let (imp_slides, imp_notes) = b.import_pptx(&pptx_bytes, &mut ui).expect("pptx import");
    assert_eq!(imp_slides, 2);
    assert!(imp_notes >= 1);

    // 10. ODP Export & Import Roundtrip
    let odp_bytes = b.export_odp().expect("odp export");
    assert!(odp_bytes.starts_with(b"PK\x03\x04"));
    let (imp_slides_odp, imp_notes_odp) = b.import_odp(&odp_bytes, &mut ui).expect("odp import");
    assert_eq!(imp_slides_odp, 2);
    assert!(imp_notes_odp >= 1);

    // 11. PDF Export
    let pdf_bytes = b.export_pdf().expect("pdf export");
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF\n") || pdf_bytes.ends_with(b"%%EOF"));

    // 12. Delete Slide
    let btn_del = b.btn_delete_slide;
    click_widget(&mut ui, &mut b, btn_del);
    assert_eq!(b.deck.slide_count(), 1);
    assert_eq!(b.current_slide, 0);

    // 13. Paint interface into pixel buffer
    let mut pixels = vec![0u32; 800 * 600];
    assert!(ui.paint(&mut pixels).is_some());

    let mut unique_colors = std::collections::HashSet::new();
    for p in &pixels {
        unique_colors.insert(*p);
    }
    assert!(
        unique_colors.len() > 8,
        "expected rich UI rendering, got {} distinct colors",
        unique_colors.len()
    );
}

#[test]
fn test_burlesque_magic_move_builds_rehearse_and_splicy_timeline() {
    let mut ui = Ui::new(1280, 800, Theme::light(), Strings::new("en"));
    let mut b = Burlesque::build(&mut ui);
    ui.layout();

    // Add second slide so Magic Move morphs hero-stat + hero-title across layouts
    let btn_add = b.btn_add_slide;
    click_widget(&mut ui, &mut b, btn_add);
    assert_eq!(b.deck.slide_count(), 2);

    let btn_prev = b.btn_prev;
    click_widget(&mut ui, &mut b, btn_prev);
    assert_eq!(b.current_slide, 0);

    // Add intra-slide build step and step through it
    let btn_add_build = b.btn_add_build;
    click_widget(&mut ui, &mut b, btn_add_build);
    assert_eq!(b.deck.get_slide(0).unwrap().builds.len(), 1);
    assert_eq!(b.active_build_index, 1);

    let btn_step_build = b.btn_step_build;
    click_widget(&mut ui, &mut b, btn_step_build);
    assert_eq!(b.active_build_index, 0);

    // Activate Magic Move and play transition -> preserves 50% mid-morph preview
    let btn_magic = b.btn_magic_move;
    click_widget(&mut ui, &mut b, btn_magic);
    assert_eq!(b.current_transition(), Transition::MagicMove);
    assert_eq!(b.magic_move_preview, Some(0.5));

    let (trans, frames) = b.play_transition(&mut ui);
    assert_eq!(trans, Transition::MagicMove);
    assert_eq!(frames, 8);
    assert_eq!(b.magic_move_preview, Some(0.5));

    // Rehearse timings across build + slide advance and commit to deck
    let btn_rehearse = b.btn_rehearse;
    click_widget(&mut ui, &mut b, btn_rehearse);
    assert!(b.rehearsal_session.is_some());
    b.rehearsal_session.as_mut().unwrap().tick(400);

    let btn_next = b.btn_next;
    click_widget(&mut ui, &mut b, btn_next); // advances build 0 -> 1
    b.rehearsal_session.as_mut().unwrap().tick(850);
    click_widget(&mut ui, &mut b, btn_next); // advances slide 0 -> 1
    b.rehearsal_session.as_mut().unwrap().tick(900);

    // Commit rehearsal timings
    click_widget(&mut ui, &mut b, btn_rehearse);
    assert!(b.rehearsal_session.is_none());
    assert!(b.deck.get_slide(0).unwrap().rehearsed_dwell_ms.unwrap_or(0) >= 850);
    assert!(
        b.deck.get_slide(0).unwrap().builds[0]
            .rehearsed_delay_ms
            .unwrap_or(0)
            >= 400
    );

    // Verify Splicy timeline export driver renders all frames deterministically
    let mut rendered_frames = 0usize;
    let (splice_tl, written, hash) =
        burlesque::render_splicy_frames(&b.deck, 320, 180, 15, |_idx, px| {
            assert_eq!(px.len(), 320 * 180);
            rendered_frames += 1;
            Ok(())
        })
        .expect("splicy timeline render");
    assert_eq!(written, rendered_frames);
    assert!(written > 20, "expected >20 rehearsed frames, got {}", written);
    assert_eq!(splice_tl.markers.len(), 2);
    assert_ne!(hash, 0);
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320u32 & mask);
        }
    }
    !crc
}

fn build_store_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();

    for &(name, data) in files {
        let name_b = name.as_bytes();
        let crc = crc32_ieee(data);
        let sz = data.len() as u32;
        let offset = out.len() as u32;

        // Local file header (30 bytes)
        out.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // store
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&sz.to_le_bytes());
        out.extend_from_slice(&sz.to_le_bytes());
        out.extend_from_slice(&(name_b.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name_b);
        out.extend_from_slice(data);

        // Central directory header (46 bytes)
        cd.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&crc.to_le_bytes());
        cd.extend_from_slice(&sz.to_le_bytes());
        cd.extend_from_slice(&sz.to_le_bytes());
        cd.extend_from_slice(&(name_b.len() as u16).to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u32.to_le_bytes());
        cd.extend_from_slice(&offset.to_le_bytes());
        cd.extend_from_slice(name_b);
    }

    let cd_offset = out.len() as u32;
    let cd_size = cd.len() as u32;
    let count = files.len() as u16;
    out.extend_from_slice(&cd);

    // End of central directory (22 bytes)
    out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[test]
fn test_burlesque_omashow_ui_features() {
    let mut ui = Ui::new(1024, 768, Theme::light(), Strings::new("en"));
    let mut b = Burlesque::build(&mut ui);
    ui.layout();

    // 1. Duplicate slide, skip/unskip, move left/right
    let btn_dup = b.btn_dup_slide;
    click_widget(&mut ui, &mut b, btn_dup);
    assert_eq!(b.deck.slide_count(), 2);
    assert_eq!(b.current_slide, 1);

    // Edit slide 1 title so we can track reorder cleanly
    ui.set_text(b.field_title, "Slide Two Cloned");
    let _ = b.on_response(&mut ui, &Response::Changed(b.field_title));

    let btn_left = b.btn_move_slide_left;
    click_widget(&mut ui, &mut b, btn_left);
    assert_eq!(b.current_slide, 0);
    assert_eq!(b.deck.slides[0].title, "Slide Two Cloned");

    let btn_right = b.btn_move_slide_right;
    click_widget(&mut ui, &mut b, btn_right);
    assert_eq!(b.current_slide, 1);
    assert_eq!(b.deck.slides[1].title, "Slide Two Cloned");

    let btn_skip = b.btn_skip_slide;
    click_widget(&mut ui, &mut b, btn_skip);
    assert!(b.deck.slides[1].hidden);
    click_widget(&mut ui, &mut b, btn_skip);
    assert!(!b.deck.slides[1].hidden);

    // 2. Morph toggle & playback
    let btn_morph = b.btn_morph;
    click_widget(&mut ui, &mut b, btn_morph);
    assert!(b.morph_enabled);
    assert_eq!(b.current_transition(), Transition::Morph);
    assert_eq!(b.magic_move_preview, Some(0.5));
    let (tr, frames) = b.play_transition(&mut ui);
    assert_eq!(tr, Transition::Morph);
    assert_eq!(frames, 8);

    // Toggle morph off
    click_widget(&mut ui, &mut b, btn_morph);
    assert!(!b.morph_enabled);
    assert_eq!(b.current_transition(), Transition::None);

    // 3. Timeline toggle + add build renders strip
    let btn_add_build = b.btn_add_build;
    click_widget(&mut ui, &mut b, btn_add_build);
    let btn_timeline = b.btn_timeline;
    click_widget(&mut ui, &mut b, btn_timeline);
    assert!(b.timeline_visible);
    assert!(ui.text_of(b.timeline_label).contains("Timeline"));

    // 4. Light-Table toggle
    let btn_lt = b.btn_light_table;
    click_widget(&mut ui, &mut b, btn_lt);
    assert!(b.light_table_mode);
    let lt_frame = b.preview_buf.clone();
    click_widget(&mut ui, &mut b, btn_lt);
    assert!(!b.light_table_mode);
    assert_ne!(lt_frame, b.preview_buf);

    // 5. Deck Review button triggers toast + populates review_label
    // Move element 0 slightly out of bounds to trigger a review diagnostic
    let mut bad_b = b.deck.slides[1].elements[0].bounds();
    bad_b.x = -32.0;
    b.deck.slides[1].elements[0].set_bounds(bad_b);

    let btn_review = b.btn_review;
    click_widget(&mut ui, &mut b, btn_review);
    assert!(b.show_review_toast);
    let rep = b.last_review.as_ref().expect("review report stored");
    assert!(!rep.is_clean());
    assert!(ui.text_of(b.review_label).contains("Review:"));

    // 6. Paint updated UI cleanly
    let mut pixels = vec![0u32; 1024 * 768];
    assert!(ui.paint(&mut pixels).is_some());

    // 7. Import Keynote (.key) bundle
    let apxl = br#"<?xml version="1.0" encoding="UTF-8"?>
<key:presentation xmlns:key="http://developer.apple.com/namespaces/keynote2">
  <key:slide title="Imported Key Slide">
    <key:p>Keynote App Import</key:p>
    <key:notes><key:p>Keynote notes intact</key:p></key:notes>
  </key:slide>
</key:presentation>"#;
    let key_bytes = build_store_zip(&[("index.apxl", apxl)]);
    let (k_slides, k_notes) = b.import_keynote(&key_bytes, &mut ui).expect("import_keynote");
    assert_eq!(k_slides, 1);
    assert_eq!(k_notes, 1);
    assert_eq!(ui.text_of(b.field_title), "Keynote App Import");
}

#[test]
fn test_burlesque_headless_cli_inspect_review_apply() {
    use burlesque::cli::run_headless_cli;
    use burlesque_core::ops::JsonValue;

    let uniq = std::process::id();
    let dir = std::env::temp_dir().join(format!("burlesque_cli_test_{}", uniq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");

    let ops_path = dir.join("ops.json");
    let pptx_out = dir.join("deck.pptx");
    let key_path = dir.join("sample.key");
    let md_bad_path = dir.join("bad.md");

    let ops_json = r##"[
      { "op": "set_metadata", "title": "CLI Test Deck", "author": "Carbon5" },
      { "op": "set_theme", "name": "midnight_slate" },
      { "op": "add_slide", "title": "CLI Opening", "body": "Hello OmaShow", "transition": "morph", "notes": "CLI speaker note" },
      { "op": "add_shape", "slide_index": 0, "shape": "rounded_rect", "corner_radius": 18, "x": 160, "y": 420, "w": 240, "h": 140, "fill": "#38BDF8", "morph_key": "cli-box", "rotation_deg": 10 },
      { "op": "add_build", "slide_index": 0, "element_index": 2, "effect": "fade", "trigger": "on_click", "delay_ms": 0, "duration_ms": 300 }
    ]"##;
    std::fs::write(&ops_path, ops_json).expect("write ops.json");

    // 1. Direct CLI call: --apply --ops <ops.json> --out <deck.pptx> --inspect
    let outcome = run_headless_cli(&[
        "burlesque".into(),
        "--apply".into(),
        "--ops".into(),
        ops_path.to_string_lossy().into_owned(),
        "--out".into(),
        pptx_out.to_string_lossy().into_owned(),
        "--inspect".into(),
    ])
    .expect("run_headless_cli --apply")
    .expect("handled");
    assert_eq!(outcome.exit_code, 0);
    let snap = JsonValue::parse(&outcome.stdout).expect("valid inspect json");
    assert_eq!(snap.get("slide_count").unwrap().as_usize(), Some(1));
    assert!(pptx_out.exists());

    // 2. Binary execution: burlesque --inspect <deck.pptx>
    let bin = env!("CARGO_BIN_EXE_burlesque");
    let out = std::process::Command::new(bin)
        .args(["--inspect", pptx_out.to_str().unwrap()])
        .output()
        .expect("run burlesque --inspect");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let inspect_stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let inspected = JsonValue::parse(inspect_stdout.trim()).expect("valid binary inspect json");
    assert_eq!(inspected.get("slide_count").unwrap().as_usize(), Some(1));

    // 3. Binary execution: burlesque --inspect <sample.key>
    let plist = br#"<plist><dict><key>title</key><string>CLI Keynote Title</string></dict></plist>"#;
    let apxl = br#"<key:presentation><key:slide><key:p>Keynote CLI Slide</key:p></key:slide></key:presentation>"#;
    std::fs::write(&key_path, build_store_zip(&[("Metadata/Properties.plist", plist), ("index.apxl", apxl)])).unwrap();
    let key_out = std::process::Command::new(bin)
        .args(["--inspect", key_path.to_str().unwrap()])
        .output()
        .expect("run burlesque --inspect .key");
    assert!(key_out.status.success());
    let key_json = JsonValue::parse(std::str::from_utf8(&key_out.stdout).unwrap().trim()).unwrap();
    assert_eq!(
        key_json.get("metadata").unwrap().get("title").unwrap().as_str(),
        Some("CLI Keynote Title")
    );

    // 4. --review on clean pptx deck, placeholder md deck, and --strict error deck
    let clean_rev = run_headless_cli(&[
        "burlesque".into(),
        "--review".into(),
        pptx_out.to_string_lossy().into_owned(),
    ])
    .expect("run_headless_cli --review clean")
    .expect("handled");
    assert_eq!(clean_rev.exit_code, 0);

    std::fs::write(&md_bad_path, "# Click to add title\n").unwrap();
    let rev_res = run_headless_cli(&[
        "burlesque".into(),
        "--review".into(),
        md_bad_path.to_string_lossy().into_owned(),
    ])
    .expect("run_headless_cli --review")
    .expect("handled");
    let rev_json = JsonValue::parse(&rev_res.stdout).expect("review json");
    assert!(rev_json.get("warning_count").unwrap().as_usize().unwrap_or(0) > 0);

    let err_ops_path = dir.join("err_deck.json");
    std::fs::write(
        &err_ops_path,
        r#"[{"op":"add_slide","title":"Zero"},{"op":"add_shape","slide_index":0,"w":0,"h":0}]"#,
    )
    .unwrap();
    let strict_res = run_headless_cli(&[
        "burlesque".into(),
        "--review".into(),
        err_ops_path.to_string_lossy().into_owned(),
        "--strict".into(),
    ])
    .expect("run_headless_cli --review --strict")
    .expect("handled");
    assert_eq!(strict_res.exit_code, 2);

    let _ = std::fs::remove_dir_all(&dir);
}

