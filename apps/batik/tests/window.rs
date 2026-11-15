// SPDX-License-Identifier: GPL-3.0-only
//! Batik Weft window host tests: widget events, painting, layers, adjustments, undo/redo, and rendering.

use batik::weft;
use batik::weft::input::{Button, Event, Pointer, PointerKind};
use batik::weft::{Strings, Theme, Ui};
use batik::{Batik, Tool};

fn click_widget(ui: &mut Ui, batik: &mut Batik, id: weft::widget::Id) {
    ui.layout();
    let c = ui.rect(id).center();
    let _ = batik.step(
        ui,
        Event::Pointer(Pointer {
            x: c.x,
            y: c.y,
            kind: PointerKind::Move,
        }),
    );
    let _ = batik.step(
        ui,
        Event::Pointer(Pointer {
            x: c.x,
            y: c.y,
            kind: PointerKind::Down(Button::Left),
        }),
    );
    let _ = batik.step(
        ui,
        Event::Pointer(Pointer {
            x: c.x,
            y: c.y,
            kind: PointerKind::Up(Button::Left),
        }),
    );
}

#[test]
fn test_batik_window_ui_interactions() {
    let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
    let mut batik = Batik::build(&mut ui);
    ui.layout();

    // 1. Initial State
    assert_eq!(batik.layer_count(), 1);
    assert_eq!(batik.tool, Tool::Brush);
    assert_eq!(batik.undo_count(), 0);
    assert_eq!(batik.pixel0(), 0xFFFF_FFFF); // initial solid white

    // 2. Select Red Color
    let swatch_red = batik
        .controls
        .iter()
        .find(|(name, _)| *name == "color_red")
        .unwrap()
        .1;
    click_widget(&mut ui, &mut batik, swatch_red);
    assert_eq!(batik.current_color, 0xFFFF_0000);

    // 3. Paint a brush stroke on the canvas
    let cr = ui.rect(batik.canvas_id);
    let start_x = cr.x + 20.0;
    let start_y = cr.y + 20.0;

    let _ = batik.step(
        &mut ui,
        Event::Pointer(Pointer {
            x: start_x,
            y: start_y,
            kind: PointerKind::Down(Button::Left),
        }),
    );
    let _ = batik.step(
        &mut ui,
        Event::Pointer(Pointer {
            x: start_x + 10.0,
            y: start_y + 10.0,
            kind: PointerKind::Move,
        }),
    );
    let _ = batik.step(
        &mut ui,
        Event::Pointer(Pointer {
            x: start_x + 10.0,
            y: start_y + 10.0,
            kind: PointerKind::Up(Button::Left),
        }),
    );

    assert_eq!(batik.undo_count(), 1);
    let painted_pixel = batik.pixel_at(20, 20);
    assert_eq!(painted_pixel, 0xFFFF_0000, "stroke painted red");

    // 4. Invert Adjustment
    let btn_invert = batik.btn_invert;
    click_widget(&mut ui, &mut batik, btn_invert);
    assert_eq!(batik.undo_count(), 2);
    let inverted_pixel = batik.pixel_at(20, 20);
    assert_eq!(inverted_pixel, 0xFF00_FFFF, "red inverted to cyan");

    // 5. Undo restores painted red
    let btn_undo = batik.btn_undo;
    click_widget(&mut ui, &mut batik, btn_undo);
    assert_eq!(batik.undo_count(), 1);
    assert_eq!(batik.pixel_at(20, 20), 0xFFFF_0000, "undo restored red");

    // 6. Redo restores inverted cyan
    let btn_redo = batik.btn_redo;
    click_widget(&mut ui, &mut batik, btn_redo);
    assert_eq!(batik.undo_count(), 2);
    assert_eq!(batik.pixel_at(20, 20), 0xFF00_FFFF, "redo restored cyan");

    // 7. Add new layer
    let btn_new_layer = batik.btn_new_layer;
    click_widget(&mut ui, &mut batik, btn_new_layer);
    assert_eq!(batik.layer_count(), 2);

    // 8. Test PNG and ORA encoding
    let png_bytes = batik.encode_png().expect("encode png");
    assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    let ora_bytes = batik.encode_ora().expect("encode ora");
    assert!(ora_bytes.starts_with(b"PK\x03\x04"));

    // 9. Paint interface into pixel buffer
    let mut pixels = vec![0u32; 800 * 600];
    assert!(ui.paint(&mut pixels).is_some());

    // Verify non-blank rendered pixels
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
