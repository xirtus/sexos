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

fn typed(ui: &mut Ui, a: &mut Abacus, text: &str) -> Vec<String> {
    let mut rs = Vec::new();
    for c in text.chars() { rs.extend(ui.handle(Event::Key(Key::typed(c)))); }
    rs.extend(ui.handle(Event::Key(Key::new(code::ENTER))));
    run(ui, a, rs)
}
fn notes(ui: &mut Ui, a: &mut Abacus, rs: Vec<Response>) -> Vec<Note> { rs.iter().filter_map(|r| a.on(ui, r)).collect() }

#[test]
fn memory_sign_recall_and_carry_on() {
    let mut ui = Ui::new(560, 720, Theme::light(), Strings::new("en"));
    let mut a = Abacus::build(&mut ui);
    assert_eq!(a.keys.len(), 35);
    assert_eq!(typed(&mut ui, &mut a, "6*7"), vec!["42".to_string()]);
    // An operator straight after an answer carries on from it; a digit starts afresh.
    assert_eq!(typed(&mut ui, &mut a, "+8"), vec!["50".to_string()]);
    assert_eq!(a.past[0].0, "ans+8");
    assert_eq!(typed(&mut ui, &mut a, "3"), vec!["3".to_string()]);
    // M+ with an empty line adds the answer; M− takes what the line works out to.
    click(&mut ui, &mut a, "M+");
    assert_eq!(a.memory, 3.0);
    assert_eq!(ui.text_of(a.mem_line), "M = 3");
    for k in ["4", "×", "1", "0"] { click(&mut ui, &mut a, k); }
    click(&mut ui, &mut a, "M−");
    assert_eq!(a.memory, -37.0);
    click(&mut ui, &mut a, "C");
    click(&mut ui, &mut a, "MC");
    assert_eq!(ui.text_of(a.mem_line), "");
    for k in ["4", "0"] { click(&mut ui, &mut a, k); }
    click(&mut ui, &mut a, "M+");
    click(&mut ui, &mut a, "C");
    // MR and ± put the memory in, negated.
    click(&mut ui, &mut a, "MR");
    assert_eq!(ui.text_of(a.expr), "40");
    click(&mut ui, &mut a, "±");
    assert_eq!(ui.text_of(a.expr), "-(40)");
    assert_eq!(click(&mut ui, &mut a, "="), vec!["-40".to_string()]);
    // x² and 1/x from the Functions menu's commands wrap the answer.
    let rs = vec![Response::Command("abacus.square".into())];
    run(&mut ui, &mut a, rs);
    assert_eq!(ui.text_of(a.expr), "(ans)^2");
    assert_eq!(click(&mut ui, &mut a, "="), vec!["1600".to_string()]);
    // Up twice picks the answer before last, Enter puts it in (bracketed: it is negative).
    ui.focus(Some(a.expr));
    let mut rs = ui.handle(Event::Key(Key::new(code::UP)));
    rs.extend(ui.handle(Event::Key(Key::new(code::UP))));
    run(&mut ui, &mut a, rs);
    assert_eq!(a.picked, Some(1));
    assert_eq!(ui.selected_row(a.history), Some(1));
    let rs = ui.handle(Event::Key(Key::new(code::ENTER)));
    assert!(matches!(notes(&mut ui, &mut a, rs).as_slice(), [Note::Recall { index: 1, value }] if value == "-40"));
    assert_eq!(ui.text_of(a.expr), "(-40)");
    assert_eq!(typed(&mut ui, &mut a, "/2"), vec!["-20".to_string()]);
    // Ctrl+D switches degrees and radians.
    let rs = ui.handle(Event::Key(Key::ctrl(0x20)));
    assert!(matches!(notes(&mut ui, &mut a, rs).as_slice(), [Note::Angle(false)]));
    assert_eq!(a.angle, abacus::calc::Angle::Radians);
    // The Functions menu puts a function in; errors say what is wrong.
    run(&mut ui, &mut a, vec![Response::Command("abacus.fn.exp".into())]);
    assert_eq!(ui.text_of(a.expr), "exp(");
    let rs = ui.handle(Event::Key(Key::new(code::ENTER)));
    assert!(matches!(notes(&mut ui, &mut a, rs).as_slice(), [Note::Error(m)] if m == "Brackets don't match"));
    let mut px = vec![0u32; 560 * 720];
    assert!(ui.paint(&mut px).is_some());
}
