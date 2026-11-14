// SPDX-License-Identifier: GPL-3.0-only
//! Burlesque Weft window host tests: slides, layout, editing, presenter view, PDF export, rendering.

use burlesque::weft;
use burlesque::weft::input::{Button, Event, Pointer, PointerKind};
use burlesque::weft::{Response, Strings, Theme, Ui};
use burlesque::Burlesque;
use burlesque_core::layout::SlideLayout;

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

    // 5. Edit Slide Title and Body
    let field_title = b.field_title;
    ui.set_text(field_title, "SexOS Architecture");
    ui.set_text(b.field_body, "- Microkernel\n- PKU Domains\n---\n- Weft Toolkit\n- Zero-copy PDX");
    let _ = b.on_response(&mut ui, &Response::Changed(field_title));

    let current = b.deck.get_slide(b.current_slide).unwrap();
    assert_eq!(current.title, "SexOS Architecture");
    assert_eq!(current.elements.len(), 3); // Title, Col1, Col2

    // 6. Presenter View Toggle
    let btn_pres = b.btn_presenter;
    click_widget(&mut ui, &mut b, btn_pres);
    assert_eq!(b.presenter_mode, true);
    assert!(b.presenter_session.is_some());

    // Toggle back
    click_widget(&mut ui, &mut b, btn_pres);
    assert_eq!(b.presenter_mode, false);
    assert!(b.presenter_session.is_none());

    // 7. Delete Slide
    let btn_del = b.btn_delete_slide;
    click_widget(&mut ui, &mut b, btn_del);
    assert_eq!(b.deck.slide_count(), 1);
    assert_eq!(b.current_slide, 0);

    // 8. PDF Export
    let pdf_bytes = b.export_pdf().expect("pdf export");
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF\n") || pdf_bytes.ends_with(b"%%EOF"));

    // 9. Paint interface into pixel buffer
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
