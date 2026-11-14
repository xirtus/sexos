// SPDX-License-Identifier: GPL-3.0-only
use leather_samples::{Action, Document, Kind, Sample, TEXT_LIMIT};
use weft::{Response, Strings, Theme, Ui};
#[test]
fn editor_roundtrips_multiline_unicode_and_empty_lines() {
    let mut d = Document::parse("first\n\n雪\n".as_bytes()).unwrap();
    assert_eq!(d.text(), "first\n\n雪\n");
    d.next();
    assert!(d.update("middle"));
    d.add();
    assert!(d.update("inserted"));
    d.previous();
    assert_eq!(d.lines[d.line], "middle");
    assert_eq!(d.text(), "first\nmiddle\ninserted\n雪\n");
}
#[test]
fn editor_rejects_binary_invalid_utf8_and_oversize_without_changing_state() {
    assert!(Document::parse(&[255]).is_err());
    assert!(Document::parse(b"a\0b").is_err());
    assert!(Document::parse(&vec![b'a'; TEXT_LIMIT + 1]).is_err());
    let mut d = Document::parse(b"hello").unwrap();
    let original = d.clone();
    assert!(!d.update("a\nb"));
    assert_eq!(d, original);
    assert!(!d.update(&"a".repeat(TEXT_LIMIT + 1)));
    assert_eq!(d, original);
}
#[test]
fn editor_enforces_limit_across_all_lines() {
    let mut d = Document::parse(&vec![b'x'; TEXT_LIMIT]).unwrap();
    assert!(!d.add());
    assert!(d.update("x"));
    assert!(d.add());
    assert!(!d.update(&"x".repeat(TEXT_LIMIT)));
    assert!(d.update("end"));
}
#[test]
fn actual_weft_controls_drive_editor_and_io_actions() {
    let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
    let mut s = Sample::build(&mut ui, Kind::Stitch);
    ui.set_text(s.content, "one");
    s.on(&mut ui, &Response::Changed(s.content));
    s.on(&mut ui, &Response::Clicked(s.add));
    ui.set_text(s.content, "two");
    s.on(&mut ui, &Response::Changed(s.content));
    assert_eq!(
        s.on(&mut ui, &Response::Clicked(s.primary)),
        Some(Action::Save("draft.txt".into(), "one\ntwo".into()))
    );
    assert_eq!(
        s.on(&mut ui, &Response::Clicked(s.secondary)),
        Some(Action::Load("draft.txt".into()))
    );
    let mut pixels = vec![0; 800 * 600];
    assert!(ui.paint(&mut pixels).is_some());
    assert!(pixels.windows(2).any(|p| p[0] != p[1]));
}
#[test]
fn hello_image_and_network_models_are_copyable_widgets() {
    for kind in [Kind::Hello, Kind::Lantern, Kind::Postcard] {
        let mut ui = Ui::new(800, 600, Theme::light(), Strings::new("en"));
        let mut s = Sample::build(&mut ui, kind);
        let action = s.on(&mut ui, &Response::Clicked(s.primary));
        match kind {
            Kind::Hello => assert_eq!(ui.text_of(s.status), "Hello, SexOS! Greeting 1"),
            Kind::Lantern => assert_eq!(action, Some(Action::Image("".into()))),
            Kind::Postcard => {
                assert_eq!(action, Some(Action::Fetch("https://example.com/".into())))
            }
            _ => unreachable!(),
        }
        let mut pixels = vec![0; 800 * 600];
        assert!(ui.paint(&mut pixels).is_some());
    }
}
