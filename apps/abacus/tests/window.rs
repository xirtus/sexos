// SPDX-License-Identifier: GPL-3.0-only
//! Abacus's window on the host: keypad clicks, typing, history and menus.
use abacus::{Abacus, Note};
use weft::input::{code, Button, Event, Key, Pointer, PointerKind};
use weft::{Response, Strings, Theme, Ui};

fn run(ui: &mut Ui, a: &mut Abacus, rs: Vec<Response>) -> Vec<String> {
    let mut out = Vec::new();
    for r in rs { if let Some(Note::Result { value, .. }) = a.on(ui, &r) { out.push(value); } }
    out
}
fn click(ui: &mut Ui, a: &mut Abacus, label: &str) -> Vec<String> {
    ui.layout();
    let id = a.keys.iter().find(|k| k.1 == label).unwrap().0;
    let c = ui.rect(id).center();
    let mut rs = Vec::new();
    for kind in [PointerKind::Move, PointerKind::Down(Button::Left), PointerKind::Up(Button::Left)] { rs.extend(ui.handle(Event::Pointer(Pointer { x: c.x, y: c.y, kind }))); }
    run(ui, a, rs)
}

#[test]
fn keypad_and_keyboard_work_sums_out() {
    let mut ui = Ui::new(560, 640, Theme::light(), Strings::new("en"));
    let mut a = Abacus::build(&mut ui);
    for k in ["1", "2", "+", "3", "0"] { click(&mut ui, &mut a, k); }
    assert_eq!(ui.text_of(a.expr), "12+30");
    assert_eq!(click(&mut ui, &mut a, "="), vec!["42".to_string()]);
    assert_eq!(ui.text_of(a.result), "42");
    // Typed: ans × 2 then Enter.
    let mut rs = Vec::new();
    for c in "ans*2".chars() { rs.extend(ui.handle(Event::Key(Key::typed(c)))); }
    rs.extend(ui.handle(Event::Key(Key::new(code::ENTER))));
    assert_eq!(run(&mut ui, &mut a, rs), vec!["84".to_string()]);
    assert_eq!(ui.selection(a.history).map(|s| s.count()), Some(0));
    assert_eq!(a.past.len(), 2);
    // Escape clears through its command; the palette finds Radians.
    let rs = ui.handle(Event::Key(Key::new(code::ESCAPE)));
    run(&mut ui, &mut a, rs);
    assert_eq!(ui.text_of(a.result), "0");
    let mut rs = ui.handle(Event::Key(Key::ctrl(0x19).shift()));
    for c in "radi".chars() { rs.extend(ui.handle(Event::Key(Key::typed(c)))); }
    rs.extend(ui.handle(Event::Key(Key::new(code::ENTER))));
    run(&mut ui, &mut a, rs);
    assert_eq!(a.angle, abacus::calc::Angle::Radians);
    // Errors show in the result line.
    for c in "1/0".chars() { ui.handle(Event::Key(Key::typed(c))); }
    let rs = ui.handle(Event::Key(Key::new(code::ENTER)));
    assert!(run(&mut ui, &mut a, rs).is_empty());
    assert_eq!(ui.text_of(a.result), "Can't divide by zero");
    let mut px = vec![0u32; 560 * 640];
    assert!(ui.paint(&mut px).is_some());
}
