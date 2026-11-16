// SPDX-License-Identifier: GPL-3.0-only
//! Host test for program tray items and notification actions (slice S2): the
//! wires and layouts in crates/sex-pdx/src/program_tray.rs and the Shell's
//! item table, menus, cards, reply field and cleanup in
//! servers/silk-shell/src/tray_items.rs, built against a `sex_pdx` of only
//! `program_tray`.
#[path = "../servers/silk-shell/src/tray_items.rs"]
mod tray_items;

use sex_pdx::program_tray::{self as pt, ActionsMsg, CardHit, CardLayout, Event, Hit, ItemMsg, Line, MenuLayout, Receiver, ReplyAssembler, Section};
use tray_items::*;

const PD: u32 = 41;
const TASK: u32 = 7;

fn screen() -> Screen {
    // 1280 wide at scale 1: status icons from 1024, workspace dots end at 748.
    Screen { width: 1280, height: 800, bar: 24, scale: 1, right: 1024, left: 756, y: 0, size: 24, gap: 4 }
}

fn add(t: &mut Tray, pd: u32, task: u32, id: u8, label: &[u8], rows: &[&[u8]]) {
    assert!(pt::item_calls(id, label, 0x3574E8, b"A tooltip", rows, |w| {
        let m = ItemMsg::decode(w[0], w[1], w[2]).expect("item word decodes");
        assert_eq!(m.wire(), w);
        t.item_msg(pd, task, m)
    }));
}

fn ask(t: &mut Tray, pd: u32, task: u32, tag: u32, actions: &[&[u8]], reply: Option<&[u8]>, now: u64) {
    assert!(pt::actions_calls(tag, actions, reply, |w| {
        let m = ActionsMsg::decode(w[0], w[1], w[2]).expect("actions word decodes");
        assert_eq!(m.wire(), w);
        t.actions_msg(pd, task, m, now)
    }));
}

/// Everything Display would be sent, through its receiver.
fn display(t: &mut Tray, s: &Screen, r: &mut Receiver) {
    let v = t.view(s);
    for section in Section::ALL {
        let mut committed = None;
        assert!(pt::view_calls(&v, section, |w| { committed = r.push(w[0], w[1], w[2]); true }));
        assert_eq!(committed, Some(section));
        t.sent(section);
    }
    assert_eq!(r.shown, v, "Display shows what the Shell sent");
}

fn outs(t: &mut Tray) -> Vec<Out> {
    let mut v = Vec::new();
    while let Some(o) = t.next_out() { v.push(o); t.told(); }
    v
}

fn type_text(t: &mut Tray, text: &str) {
    for c in text.bytes() {
        let (code, shift) = (0u8..=0x39).flat_map(|k| [(k, false), (k, true)]).find(|&(k, sh)| pt::key_char(k, sh) == Some(c)).expect("typable");
        if shift { assert_eq!(t.key(0x2A, 1, 0), Key::Pass); }
        assert_eq!(t.key(code, 1, 0), Key::Typed);
        assert_eq!(t.key(code, 0, 0), Key::Pass, "releases pass");
        if shift { assert_eq!(t.key(0x2A, 0, 0), Key::Pass); }
    }
}

#[test]
fn opcodes_are_the_free_ones() {
    assert_eq!([pt::OP_TRAY_ITEM, pt::OP_TRAY_EVENT, pt::OP_NOTIFY_ACTIONS, pt::OP_TRAY_VIEW], [0x2EA, 0x2EB, 0x2EC, 0x2ED]);
}

#[test]
fn words_round_trip_and_bad_words_are_refused() {
    let label = [b'A', b'b', 0];
    for m in [ItemMsg::Begin { id: 1, label, label_len: 2, colour: 0x123456, rows: 8 }, ItemMsg::Commit { id: 0 }, ItemMsg::Remove { id: 1 },
        ItemMsg::Text { id: 0, field: pt::field_row(7), part: 0, len: 16, a: 1, b: 2 }, ItemMsg::Text { id: 0, field: pt::FIELD_TIP, part: 1, len: 3, a: 1, b: 0 }] {
        let w = m.wire();
        assert_eq!(ItemMsg::decode(w[0], w[1], w[2]), Some(m));
    }
    // Item 2, nine rows, a row's second piece, a third tooltip piece: refused.
    assert_eq!(ItemMsg::decode(ItemMsg::Commit { id: 2 }.wire()[0], 0, 0), None);
    assert_eq!(ItemMsg::decode(ItemMsg::Begin { id: 0, label, label_len: 2, colour: 0, rows: 9 }.wire()[0], 0, 0), None);
    assert_eq!(ItemMsg::decode(ItemMsg::Text { id: 0, field: 1, part: 1, len: 1, a: 0, b: 0 }.wire()[0], 0, 0), None);
    assert_eq!(ItemMsg::decode(ItemMsg::Text { id: 0, field: 0, part: 2, len: 1, a: 0, b: 0 }.wire()[0], 0, 0), None);
    assert_eq!(ItemMsg::decode(0x99, 0, 0), None);
    for m in [ActionsMsg::Begin { tag: u32::MAX, actions: 3, reply: true }, ActionsMsg::Commit, ActionsMsg::Text { field: pt::FIELD_PLACEHOLDER, part: 0, len: 5, a: 9, b: 0 }] {
        let w = m.wire();
        assert_eq!(ActionsMsg::decode(w[0], w[1], w[2]), Some(m));
    }
    assert_eq!(ActionsMsg::decode(ActionsMsg::Begin { tag: 1, actions: 4, reply: false }.wire()[0], 1, 0), None);
    for e in [Event::Activated { item: 1 }, Event::Menu { item: 0, row: 7 }, Event::Action { tag: 0xDEAD_BEEF, action: 2 },
        Event::Gone { tag: 5, why: pt::GONE_EXPIRED }, Event::Reply { tag: 9, part: 1, total: 20, a: 3, b: 4 }] {
        let w = e.wire();
        assert_eq!(Event::decode(w[0], w[1], w[2]), Some(e));
    }
    assert_eq!(Event::decode(Event::Menu { item: 0, row: 8 }.wire()[0], 0, 0), None);
    assert_eq!(Event::decode(Event::Reply { tag: 9, part: 2, total: 20, a: 0, b: 0 }.wire()[0], 0, 0), None, "a piece past the end");
}

#[test]
fn replies_survive_pieces_and_restart_cleanly() {
    for text in [&b""[..], b"hi", b"exactly sixteen!", b"a reply longer than one piece, and longer than two pieces too"] {
        let mut a = ReplyAssembler::new();
        let mut got = None;
        for e in pt::reply_events(77, text) { got = a.push(e); }
        let (tag, line) = got.expect("whole reply");
        assert_eq!((tag, line.as_bytes()), (77, text));
    }
    // A piece out of order is dropped; the next first piece starts over.
    let pieces: Vec<Event> = pt::reply_events(3, b"0123456789abcdefXYZ").collect();
    let mut a = ReplyAssembler::new();
    assert_eq!(a.push(pieces[1]), None);
    assert_eq!(a.push(pieces[0]), None);
    assert_eq!(a.push(pieces[1]).unwrap().1.as_bytes(), b"0123456789abcdefXYZ");
    // Control bytes never reach a program (a newline parts summary and body).
    let mut l = Line::<16>::EMPTY;
    let (x, y) = pt::piece_words(b"a\nb\x07");
    assert!(l.put(0, 4, x, y));
    assert_eq!(l.as_bytes(), b"a\nb?");
}

#[test]
fn items_line_up_left_of_the_status_icons_oldest_first() {
    let s = screen();
    let mut t = Tray::new();
    add(&mut t, PD, TASK, 0, b"=", &[b"Show", b"Quit"]);
    add(&mut t, 42, 1, 1, b"Mail", &[]);
    let (placed, n) = t.placed(&s);
    assert_eq!(n, 2);
    assert_eq!(placed[0].1, (1024 - 4 - 28, 0, 24, 24));
    assert_eq!(placed[1].1, (1024 - 4 - 56, 0, 24, 24));
    let v = t.view(&s);
    assert_eq!(v.icons()[1].label_len, 3, "labels hold three characters");
    // An item being begun again is hidden until committed, and keeps its place.
    assert!(t.item_msg(PD, TASK, ItemMsg::Begin { id: 0, label: *b"=  ", label_len: 1, colour: 1, rows: 0 }));
    assert_eq!(t.placed(&s).1, 1);
    assert!(t.item_msg(PD, TASK, ItemMsg::Commit { id: 0 }));
    assert_eq!(t.view(&s).icons()[0].x, 992);
    // No room past the workspace dots.
    for i in 0..10 { assert_eq!(pt::icon_rect(i, 1024, 756, 0, 24, 4).is_some(), i < 9); }
    // The table holds eight items; a ninth is refused.
    let mut full = Tray::new();
    for pd in 0..8 { add(&mut full, 100 + pd, 1, 0, b"x", &[]); }
    assert!(!full.item_msg(200, 1, ItemMsg::Begin { id: 0, label: *b"x\0\0", label_len: 1, colour: 0, rows: 0 }));
    // Pieces for an item not begun, or from another run, are refused.
    assert!(!t.item_msg(PD, TASK, ItemMsg::Text { id: 0, field: 0, part: 0, len: 1, a: 1, b: 0 }));
    assert!(!t.item_msg(PD, TASK + 1, ItemMsg::Remove { id: 0 }));
    assert!(t.item_msg(PD, TASK, ItemMsg::Remove { id: 0 }));
    assert_eq!(t.placed(&s).1, 1);
}

#[test]
fn click_opens_menu_and_a_row_reaches_the_program() {
    let s = screen();
    let mut t = Tray::new();
    let mut r = Receiver::new();
    add(&mut t, PD, TASK, 0, b"=", &[b"Show answer", b"Clear", b"Scientific", b"Ask me a sum"]);
    display(&mut t, &s, &mut r);
    let icon = r.shown.icons()[0];
    // The tooltip shows while the pointer is over the item.
    t.pointer(&s, icon.x as usize + 5, 5);
    assert_ne!(t.dirty() & Section::Tip.bit(), 0);
    display(&mut t, &s, &mut r);
    assert_eq!(r.shown.tip.unwrap().text.as_bytes(), b"A tooltip");
    let Press::MenuOpened { rows, panel, row_h, pad, .. } = t.press(&s, icon.x as usize + 5, 5, 0) else { panic!("menu") };
    assert_eq!(rows, 4);
    display(&mut t, &s, &mut r);
    assert!(r.shown.tip.is_none(), "the tooltip goes with the menu");
    let m = r.shown.menu.unwrap();
    let l = MenuLayout::new(m.x as usize, m.y as usize, m.rows as usize, s.width, s.height, s.scale).unwrap();
    assert_eq!((l.panel, l.row_h, l.pad), (panel, row_h, pad), "Display lays the menu out where the Shell does");
    assert_eq!(m.text[3].as_bytes(), b"Ask me a sum");
    let (x, y) = (panel.0 + pad + 20, panel.1 + pad + 2 * row_h + row_h / 2);
    t.pointer(&s, x, y);
    display(&mut t, &s, &mut r);
    assert_eq!(r.shown.menu.unwrap().hover, Some(2));
    assert_eq!(l.hit(x, y), Hit::Row(2));
    assert_eq!(t.press(&s, x, y, 0), Press::Chose { pd: PD, id: 0, row: 2 });
    assert_eq!(outs(&mut t), vec![Out::Event { pd: PD, task: TASK, event: Event::Menu { item: 0, row: 2 } }]);
    display(&mut t, &s, &mut r);
    assert!(r.shown.menu.is_none());
    // A press outside an open menu closes it and is taken; Esc closes it too.
    t.press(&s, icon.x as usize + 5, 5, 0);
    assert_eq!(t.press(&s, 10, 600, 0), Press::MenuClosed);
    t.press(&s, icon.x as usize + 5, 5, 0);
    assert_eq!(t.key(0x01, 1, 0), Key::MenuClosed);
    assert!(!t.menu_open());
    // An item without a menu is activated; anywhere else passes.
    add(&mut t, PD, TASK, 1, b"!", &[]);
    let r1 = t.placed(&s).0[1].1;
    assert_eq!(t.press(&s, r1.0 + 1, r1.1 + 1, 0), Press::Activated { pd: PD, id: 1 });
    assert_eq!(outs(&mut t), vec![Out::Event { pd: PD, task: TASK, event: Event::Activated { item: 1 } }]);
    assert_eq!(t.press(&s, 400, 400, 0), Press::Pass);
}

#[test]
fn notification_reply_reaches_the_program() {
    let s = screen();
    let mut t = Tray::new();
    let mut r = Receiver::new();
    ask(&mut t, PD, TASK, 1, &[b"Clear"], Some(b"Type a sum"), 1_000);
    // Another program's notification gets no card.
    assert_eq!(t.notification(99, b"Other\nbody", b"other", 1_100), None);
    assert_eq!(t.notification(PD, b"Abacus\nReply with a sum to work out", b"sex-calc", 1_200), Some((1, 0)));
    display(&mut t, &s, &mut r);
    let c = r.shown.card.unwrap();
    assert_eq!((c.actions, c.reply, c.summary(), c.body(), c.labels[0].as_bytes(), c.placeholder.as_bytes(), c.from.as_bytes()),
        (1, true, &b"Abacus"[..], &b"Reply with a sum to work out"[..], &b"Clear"[..], &b"Type a sum"[..], &b"sex-calc"[..]));
    let l = CardLayout::new(s.width, s.bar, s.scale, 1, true).unwrap();
    let f = l.field.unwrap();
    // Keys pass until the field is clicked.
    assert_eq!(t.key(0x02, 1, 0), Key::Pass);
    assert_eq!(t.press(&s, f.0 + 4, f.1 + 4, 1_300), Press::ReplyFocused { pd: PD, tag: 1 });
    type_text(&mut t, "12-5");
    assert_eq!(t.key(0x0E, 1, 0), Key::Typed);
    type_text(&mut t, "6 Ok?");
    display(&mut t, &s, &mut r);
    assert!(r.shown.reply.active);
    assert_eq!(r.shown.reply.text.as_bytes(), b"12-6 Ok?");
    // Esc leaves the field; a click on it comes back; Enter sends.
    assert_eq!(t.key(0x01, 1, 0), Key::Left);
    assert_eq!(t.key(0x02, 1, 0), Key::Pass);
    t.press(&s, f.0 + 4, f.1 + 4, 1_400);
    assert_eq!(t.key(0x1C, 1, 1_500), Key::Sent { pd: PD, tag: 1, len: 8 });
    let o = outs(&mut t);
    assert_eq!(o.len(), 1);
    let Out::Reply { pd, task, tag, text } = o[0] else { panic!("reply") };
    assert_eq!((pd, task, tag, text.as_bytes()), (PD, TASK, 1, &b"12-6 Ok?"[..]));
    // Through the wire to the program.
    let mut a = ReplyAssembler::new();
    let mut got = None;
    for e in pt::reply_events(tag, text.as_bytes()) { let w = e.wire(); got = a.push(Event::decode(w[0], w[1], w[2]).unwrap()); }
    assert_eq!(got.unwrap().1.as_bytes(), b"12-6 Ok?");
    display(&mut t, &s, &mut r);
    assert!(r.shown.card.is_none() && !r.shown.reply.active && r.shown.reply.text.is_empty());
    assert_eq!(t.key(0x02, 1, 0), Key::Pass, "no card, no typing");
}

#[test]
fn buttons_close_expiry_and_the_queue() {
    let s = screen();
    let mut t = Tray::new();
    for tag in 1..=5 {
        ask(&mut t, PD, TASK, tag, &[b"Yes", b"No", b"Later"], None, 0);
        assert!(t.notification(PD, b"Q", b"p", 10).is_some());
    }
    // Four cards fit; the fifth pushed out the oldest waiting one (tag 2).
    assert_eq!(t.queued(), CARD_QUEUE);
    assert_eq!(outs(&mut t), vec![Out::Event { pd: PD, task: TASK, event: Event::Gone { tag: 2, why: pt::GONE_REPLACED } }]);
    let l = CardLayout::new(s.width, s.bar, s.scale, 3, false).unwrap();
    assert_eq!(l.field, None);
    let b = l.actions[2];
    assert_eq!(l.hit(b.0 + 1, b.1 + 1), CardHit::Action(2));
    assert_eq!(t.press(&s, b.0 + 1, b.1 + 1, 20), Press::CardAction { pd: PD, tag: 1, action: 2 });
    let x = l.close;
    assert_eq!(t.press(&s, x.0 + 2, x.1 + 2, 20), Press::CardClosed { pd: PD, tag: 3 });
    // A click on the card's empty part is taken; outside it passes.
    assert_eq!(t.press(&s, l.panel.0 + 2, l.panel.1 + l.panel.3 - 2, 20), Press::Taken);
    assert_eq!(t.press(&s, 100, 500, 20), Press::Pass);
    // Unanswered, the card showing goes after its time.
    assert_eq!(t.tick(20 + CARD_MS - 1), None);
    assert_eq!(t.tick(20 + CARD_MS + 1), Some((PD, 4)));
    assert_eq!(outs(&mut t), vec![
        Out::Event { pd: PD, task: TASK, event: Event::Action { tag: 1, action: 2 } },
        Out::Event { pd: PD, task: TASK, event: Event::Gone { tag: 3, why: pt::GONE_CLOSED } },
        Out::Event { pd: PD, task: TASK, event: Event::Gone { tag: 4, why: pt::GONE_EXPIRED } },
    ]);
    assert_eq!(t.card().unwrap().tag, 5);
    // An action list Bell never forwarded is forgotten.
    ask(&mut t, 50, 1, 9, &[b"x"], None, 100);
    t.tick(100 + PAIR_MS + 1);
    assert_eq!(t.notification(50, b"late", b"p", 100 + PAIR_MS + 2), None);
}

#[test]
fn a_typed_reply_keeps_its_card_past_the_time() {
    let s = screen();
    let mut t = Tray::new();
    ask(&mut t, PD, TASK, 4, &[], Some(b"Reply"), 0);
    t.notification(PD, b"Hi", b"p", 0);
    let f = CardLayout::new(s.width, s.bar, s.scale, 0, true).unwrap().field.unwrap();
    t.press(&s, f.0 + 1, f.1 + 1, 0);
    type_text(&mut t, "x");
    assert_eq!(t.tick(CARD_MS * 2), None);
    assert!(t.card().is_some());
    // Enter with nothing typed sends nothing.
    assert_eq!(t.key(0x0E, 1, 0), Key::Typed);
    assert_eq!(t.key(0x1C, 1, 0), Key::Taken);
    assert!(outs(&mut t).is_empty());
}

#[test]
fn an_ended_program_leaves_nothing_behind() {
    let s = screen();
    let mut t = Tray::new();
    let mut r = Receiver::new();
    add(&mut t, PD, TASK, 0, b"=", &[b"a"]);
    add(&mut t, 60, 2, 0, b"b", &[]);
    ask(&mut t, PD, TASK, 1, &[b"ok"], Some(b"r"), 0);
    t.notification(PD, b"x", b"p", 0);
    ask(&mut t, 60, 2, 2, &[b"ok"], None, 0);
    t.notification(60, b"y", b"p", 0);
    let icon = t.placed(&s).0[0].1;
    t.press(&s, icon.0 + 1, icon.1 + 1, 0);
    assert!(t.menu_open());
    display(&mut t, &s, &mut r);
    let gone = t.retain(|pd, task| !(pd == PD && task == TASK), 5);
    assert_eq!(gone, 2);
    assert!(!t.menu_open());
    assert_eq!(t.items().count(), 1);
    assert_eq!(t.card().unwrap().pd, 60, "the next card shows");
    display(&mut t, &s, &mut r);
    assert_eq!(r.shown.icon_count, 1);
    assert!(r.shown.menu.is_none());
    // A new run of the same program starts with a clean slate.
    assert!(!t.item_msg(PD, TASK + 1, ItemMsg::Commit { id: 0 }));
}

#[test]
fn display_never_shows_a_half_sent_section() {
    let s = screen();
    let mut t = Tray::new();
    let mut r = Receiver::new();
    ask(&mut t, PD, TASK, 1, &[b"One", b"Two"], Some(b"Say"), 0);
    t.notification(PD, b"A long enough summary to need pieces\nand a body", b"p", 0);
    let v = t.view(&s);
    let mut calls = Vec::new();
    pt::view_calls(&v, Section::Card, |w| { calls.push(w); true });
    assert!(calls.len() > 3);
    for w in &calls[..calls.len() - 1] { assert_eq!(r.push(w[0], w[1], w[2]), None); }
    assert!(r.shown.card.is_none(), "nothing shows before the commit");
    // A section begun again (the Shell retrying) starts over.
    for w in &calls { r.push(w[0], w[1], w[2]); }
    assert_eq!(r.shown.card, v.card);
    // Scales 2 and 3 lay the card out in proportion and inside the screen.
    for scale in 1..=3 {
        let l = CardLayout::new(1280 * scale, 24 * scale, scale, 3, true).unwrap();
        assert!(l.panel.0 + l.panel.2 <= 1280 * scale);
        assert_eq!(l.panel.2, CardLayout::WIDTH * scale);
    }
    assert_eq!(CardLayout::new(300, 24, 1, 1, true), None);
}
