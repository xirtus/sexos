// SPDX-License-Identifier: GPL-3.0-only
//! Host test for desktop scripting (slice S4): the `silk` wire, words and
//! answer pieces in crates/sex-pdx/src/silk_ctl.rs, and the Shell's request
//! checks, answer lines, event ring and feeds in
//! servers/silk-shell/src/silk_ctl.rs, built against a `sex_pdx` of only
//! `silk_ctl`.
#[path = "../servers/silk-shell/src/silk_ctl.rs"]
mod silk_ctl;

use sex_pdx::silk_ctl::{self as wire, Assembler, Got, Piece, Request};
use silk_ctl::*;

fn win(frame: u32, sid: u64, scene: u8) -> Win {
    let mut w = Win { frame, sid, scene, x: 0, y: 48, w: 640, h: 752, ..Win::EMPTY };
    w.title[..6].copy_from_slice(b"Abacus");
    w.title_len = 6;
    w
}

#[test]
fn words_become_requests() {
    assert_eq!(wire::parse(b"windows"), Ok(Request::List));
    assert_eq!(wire::parse(b"  list "), Ok(Request::List));
    assert_eq!(wire::parse(b"focus 12"), Ok(Request::Focus { frame: 12 }));
    assert_eq!(wire::parse(b"move 12 3"), Ok(Request::Move { frame: 12, workspace: 3 }));
    assert_eq!(wire::parse(b"layout 2 dwindle"), Ok(Request::Layout { workspace: 2, layout: 1 }));
    assert_eq!(wire::parse(b"layout 9 grid"), Ok(Request::Layout { workspace: 9, layout: 3 }));
    assert_eq!(wire::parse(b"float 7"), Ok(Request::Float { frame: 7 }));
    assert_eq!(wire::parse(b"close 7"), Ok(Request::Close { frame: 7 }));
    assert_eq!(wire::parse(b"events"), Ok(Request::Events { since: 0 }));
    assert_eq!(wire::parse(b"events since 41"), Ok(Request::Events { since: 41 }));
    assert_eq!(wire::parse(b"events follow"), Ok(Request::Follow { count: 0 }));
    assert_eq!(wire::parse(b"events follow 5"), Ok(Request::Follow { count: 5 }));
    // Usage for nothing or an unknown word; a message for a bad value.
    assert_eq!(wire::parse(b""), Err(""));
    assert_eq!(wire::parse(b"dance"), Err(""));
    assert_eq!(wire::parse(b"windows 3"), Err(""));
    assert!(wire::parse(b"move 12 0").unwrap_err().contains("1 to 9"));
    assert!(wire::parse(b"move 12 10").unwrap_err().contains("1 to 9"));
    assert!(wire::parse(b"move x 3").unwrap_err().contains("FRAME"));
    assert!(wire::parse(b"move 12").is_err());
    assert!(wire::parse(b"focus -1").is_err());
    assert!(wire::parse(b"focus 99999999999").is_err());
    assert!(wire::parse(b"layout 1 spiral").unwrap_err().contains("master"));
    assert!(wire::parse(b"layout 1 plugin:/home/a.wasm").unwrap_err().contains("plugins"));
    assert!(wire::parse(b"events follow 70000").is_err());
}

#[test]
fn requests_round_trip_and_bad_words_are_refused() {
    let all = [Request::List, Request::Focus { frame: 12 }, Request::Move { frame: 12, workspace: 3 },
        Request::Layout { workspace: 9, layout: 3 }, Request::Float { frame: 1 }, Request::Close { frame: u32::MAX },
        Request::Events { since: 77 }, Request::Follow { count: 65535 }, Request::Unfollow];
    for (i, r) in all.iter().enumerate() {
        let tag = 0xA000 + i as u16;
        assert_eq!(Request::decode(r.wire(tag)), Some((tag, *r)), "{r:?}");
    }
    let move3 = Request::Move { frame: 12, workspace: 3 }.wire(1);
    assert_eq!(Request::decode([move3[0], move3[1], 0]), None, "workspace 0");
    assert_eq!(Request::decode([move3[0], move3[1], 10]), None, "workspace 10");
    assert_eq!(Request::decode([move3[0], 1 << 32, 3]), None, "frame past u32");
    assert_eq!(Request::decode([move3[0] | 1 << 40, 12, 3]), None, "high bits");
    assert_eq!(Request::decode([0, 0, 0]), None, "kind 0");
    assert_eq!(Request::decode([99, 0, 0]), None, "unknown kind");
    assert_eq!(Request::decode([4 | 1 << 8, 1, 4]), None, "layout index 4");
    assert_eq!(Request::decode([1 | 1 << 8, 5, 0]), None, "list takes nothing");
}

#[test]
fn answers_come_back_whole_in_pieces() {
    let long = b"frame=12 sid=358 ws=3 x=0 y=48 w=640 h=752 state=tiled tab=1/1 focus=0 title=Abacus";
    let mut asm = Assembler::new();
    asm.expect(9);
    let ps: Vec<Piece> = wire::pieces(9, long, false, false).collect();
    assert_eq!(ps.len(), long.len().div_ceil(16));
    // Another answer's pieces are not taken.
    let stray = wire::pieces(8, b"error stale", true, true).next().unwrap();
    assert_eq!(asm.take(&Piece::decode(stray.wire()).unwrap()), Got::Nothing);
    for (i, p) in ps.iter().enumerate() {
        let got = asm.take(&Piece::decode(p.wire()).unwrap());
        assert_eq!(got, if i + 1 == ps.len() { Got::Line } else { Got::Nothing });
    }
    assert_eq!(asm.line(), &long[..]);
    // The last line ends the answer, and says whether it failed.
    let end: Vec<Piece> = wire::pieces(9, b"error no-window frame=4", true, true).collect();
    let mut last = Got::Nothing;
    for p in &end { last = asm.take(p); }
    assert_eq!(last, Got::LineAndEnd { failed: true });
    assert_eq!(asm.line(), b"error no-window frame=4");
    // An empty line is one empty piece; too long is cut; control bytes are '?'.
    let empty: Vec<Piece> = wire::pieces(9, b"", true, false).collect();
    assert_eq!(empty.len(), 1);
    assert_eq!(asm.take(&empty[0]), Got::LineAndEnd { failed: false });
    assert_eq!(asm.line(), b"");
    let big = [b'x'; 200];
    assert_eq!(wire::pieces(1, &big, false, false).map(|p| p.len as usize).sum::<usize>(), wire::LINE_MAX);
    let p = wire::pieces(1, b"a\x07b", false, false).next().unwrap();
    assert_eq!(p.text(), b"a?b");
    // Malformed headers are refused.
    let w = p.wire();
    assert!(Piece::decode([w[0] | 17 << 16, w[1], w[2]]).is_none(), "too long");
    assert!(Piece::decode([w[0] | (wire::FAILED as u64) << 24, w[1], w[2]]).is_none(), "failed without end");
    assert!(Piece::decode([w[0] | 1 << 40, w[1], w[2]]).is_none(), "high bits");
}

#[test]
fn window_lines_are_stable_key_value_words() {
    let mut w = win(12, 358, 2);
    let l = window_line(&w, false);
    assert_eq!(l.as_bytes(), b"frame=12 sid=358 ws=3 x=0 y=48 w=640 h=752 state=tiled tab=1/1 focus=0 title=Abacus");
    w.floating = true; w.tabs = 3; w.tab = 1; w.title_len = 0; w.x = -20;
    assert_eq!(window_line(&w, true).as_bytes(), b"frame=12 sid=358 ws=3 x=-20 y=48 w=640 h=752 state=float tab=2/3 focus=1 title=-");
    w.zoomed = true;
    assert!(core::str::from_utf8(window_line(&w, true).as_bytes()).unwrap().contains("state=zoom"));
    w.minimized = true;
    assert!(core::str::from_utf8(window_line(&w, true).as_bytes()).unwrap().contains("state=hidden"));
    // The widest line still fits one Spindle line.
    let mut wide = win(u32::MAX, u64::MAX, 8);
    wide.x = i32::MIN; wide.y = i32::MIN; wide.w = u32::MAX; wide.h = u32::MAX; wide.title = [b'W'; 16]; wide.title_len = 16;
    assert!(window_line(&wide, true).as_bytes().len() <= wire::LINE_MAX);
    assert_eq!(list_end(2, 0, 12).as_bytes(), b"ok windows=2 ws=1 focus=12");
}

#[test]
fn checks_find_the_window_and_say_what_to_do() {
    let wins = [win(12, 358, 0), win(13, 359, 2), Win { minimized: true, ..win(14, 360, 0) }];
    assert_eq!(check(Request::List, &wins, 0), Ok(Act::List));
    assert_eq!(check(Request::Move { frame: 12, workspace: 3 }, &wins, 0), Ok(Act::Move { frame: 12, sid: 358, from: 0, to: 2 }));
    assert_eq!(check(Request::Move { frame: 13, workspace: 3 }, &wins, 0), Ok(Act::Stay { frame: 13, scene: 2 }));
    assert_eq!(check(Request::Move { frame: 99, workspace: 3 }, &wins, 0), Err(Refusal::NoWindow(99)));
    assert_eq!(check(Request::Focus { frame: 13 }, &wins, 0), Ok(Act::Focus { frame: 13, sid: 359, scene: 2 }));
    assert_eq!(check(Request::Focus { frame: 14 }, &wins, 0), Err(Refusal::Hidden(14)));
    assert_eq!(check(Request::Float { frame: 12 }, &wins, 0), Ok(Act::Float { frame: 12, sid: 358, on: true, shown: true }));
    assert_eq!(check(Request::Float { frame: 13 }, &wins, 0), Ok(Act::Float { frame: 13, sid: 359, on: true, shown: false }));
    assert_eq!(check(Request::Close { frame: 14 }, &wins, 0), Ok(Act::Close { frame: 14, sid: 360 }));
    assert_eq!(check(Request::Layout { workspace: 3, layout: 2 }, &wins, 0), Ok(Act::Layout { scene: 2, layout: 2 }));
    assert_eq!(Refusal::NoWindow(99).line().as_bytes(), b"error no-window frame=99");
}

type W = Watch<24, 9, 8>;

#[test]
fn events_are_what_changed_between_looks() {
    let mut w = W::new();
    let l = [0u8; 9];
    // The first look only learns what is there.
    assert_eq!(w.observe(&[(10, 153, 0)], 153, 10, 0, &l), 0);
    assert_eq!(w.last(), 0);
    // A program opens and takes focus.
    assert_eq!(w.observe(&[(10, 153, 0), (12, 358, 0)], 358, 12, 0, &l), 2);
    // Moved to workspace 3; focus back to Spindle.
    assert_eq!(w.observe(&[(10, 153, 0), (12, 358, 2)], 153, 10, 0, &l), 2);
    // Workspace 3 shown in dwindle; nothing else changed.
    let mut l3 = l; l3[2] = 1;
    assert_eq!(w.observe(&[(10, 153, 0), (12, 358, 2)], 153, 10, 2, &l3), 2);
    // Closed. The same look again adds nothing.
    assert_eq!(w.observe(&[(10, 153, 0)], 153, 10, 2, &l3), 1);
    assert_eq!(w.observe(&[(10, 153, 0)], 153, 10, 2, &l3), 0);
    let lines: Vec<String> = w.since(0).map(|e| String::from_utf8(e.line().as_bytes().to_vec()).unwrap()).collect();
    assert_eq!(lines, [
        "event seq=1 kind=opened frame=12 sid=358 ws=1",
        "event seq=2 kind=focused frame=12 sid=358",
        "event seq=3 kind=moved frame=12 sid=358 from=1 to=3",
        "event seq=4 kind=focused frame=10 sid=153",
        "event seq=5 kind=workspace from=1 to=3",
        "event seq=6 kind=layout ws=3 layout=dwindle",
        "event seq=7 kind=closed frame=12 sid=358",
    ]);
    assert_eq!(w.since(5).map(|e| e.seq).collect::<Vec<_>>(), [6, 7]);
    assert_eq!(w.since(7).count(), 0);
    assert_eq!(w.since(1000).count(), 0);
    // A frame showing another tab (a group) is not a new window; focus to none is not an event.
    assert_eq!(w.observe(&[(10, 154, 0)], 0, 0, 2, &l3), 0);
}

#[test]
fn the_ring_keeps_the_newest() {
    let mut w = W::new();
    let l = [0u8; 9];
    w.observe(&[], 0, 0, 0, &l);
    for i in 0..20u8 { w.observe(&[], 0, 0, i % 2, &l); }
    assert_eq!(w.last(), 19);
    let kept: Vec<u32> = w.since(0).map(|e| e.seq).collect();
    assert_eq!(kept, (12..=19).collect::<Vec<_>>(), "eight kept, oldest first");
    assert_eq!(w.since(17).map(|e| e.seq).collect::<Vec<_>>(), [18, 19]);
}

#[test]
fn feeds_count_down_and_are_replaced_per_caller() {
    let mut f = Feeds::<2>::new();
    assert_eq!(f.start(12, 1, 5, 2, 10), Ok(None));
    assert_eq!(f.start(17, 1, 6, 0, 10), Ok(None));
    assert_eq!(f.start(24, 1, 7, 1, 10), Err(()), "full");
    // The same caller asking again replaces its feed.
    let old = f.start(12, 1, 8, 2, 11).unwrap().unwrap();
    assert_eq!(old.tag, 5);
    let mut done = vec![];
    for slot in f.iter_mut() {
        let Some(feed) = slot.as_mut() else { continue };
        let a = feed.sent(12);
        let b = feed.sent(13);
        done.push((feed.pd, a, b, feed.after));
    }
    assert_eq!(done, [(12, false, true, 13), (17, false, false, 13)], "a count of 0 never ends");
    assert_eq!(f.stop(17).map(|x| x.tag), Some(6));
    assert_eq!(f.stop(17), None);
    assert_eq!(f.retain(|pd, _| pd != 12), 1);
    assert!(!f.any());
}

#[test]
fn answers_fit_the_outbox_whole_or_say_so() {
    assert_eq!(line_pieces(0), 1);
    assert_eq!(line_pieces(16), 1);
    assert_eq!(line_pieces(17), 2);
    assert_eq!(line_pieces(500), END_ROOM);
    assert_eq!(END_ROOM, wire::LINE_MAX.div_ceil(wire::PIECE));
    // Each 20-byte line takes two pieces; the end keeps END_ROOM back.
    assert_eq!(fit(END_ROOM + 6, [20, 20, 20]), (3, true));
    assert_eq!(fit(END_ROOM + 5, [20, 20, 20]), (2, false));
    assert_eq!(fit(END_ROOM, [20]), (0, false));
    assert_eq!(fit(END_ROOM, []), (0, true));
    assert_eq!(fit(END_ROOM - 1, []), (0, false));
    // The first line that does not fit stops it: no later, shorter line skips ahead.
    assert_eq!(fit(END_ROOM + 3, [50, 10]), (0, false));
    let busy = busy_line("windows");
    assert!(busy.as_bytes().starts_with(b"error busy windows"));
    assert!(busy.as_bytes().len() <= wire::LINE_MAX);
}

#[test]
fn events_name_only_what_was_delivered() {
    let mut w = Watch::<4, 2, 8>::new();
    w.observe(&[], 0, 0, 0, &[0, 0]);
    for i in 1..=5u32 { w.observe(&[(i, i as u64, 0)], 0, 0, 0, &[0, 0]); }
    let all: Vec<_> = w.since(0).collect();
    let lens: Vec<_> = all.iter().map(|e| e.line().as_bytes().len()).collect();
    let per = line_pieces(lens[0]);
    // Room for three events and the end: three go out, the end names the third.
    let (n, whole) = fit(END_ROOM + 3 * per, lens.iter().copied());
    assert_eq!((n, whole), (3, false));
    let last = all[n - 1].seq;
    let end = events_end(n, last, whole);
    assert_eq!(end.as_bytes(), format!("error overflow events=3 last={} (more waiting; ask again with since)", last).as_bytes());
    // Asking again from there gets the rest, whole.
    let rest: Vec<_> = w.since(last).map(|e| e.line().as_bytes().len()).collect();
    assert_eq!(fit(END_ROOM + 10 * per, rest.iter().copied()), (rest.len(), true));
    assert_eq!(events_end(rest.len(), w.last(), true).as_bytes(), format!("ok events={} last={}", rest.len(), w.last()).as_bytes());
}

#[test]
fn a_feed_left_behind_catches_up_without_a_new_change() {
    let mut f = Feeds::<2>::new();
    assert!(!f.behind(9));
    f.start(5, 1, 0x10, 3, 4).unwrap();
    assert!(!f.behind(4), "caught up when it started");
    // Two events came, but the outbox took none: it is behind until sent.
    assert!(f.behind(6));
    for slot in f.iter_mut() { if let Some(x) = slot.as_mut() { x.sent(5); } }
    assert!(f.behind(6));
    for slot in f.iter_mut() { if let Some(x) = slot.as_mut() { x.sent(6); } }
    assert!(!f.behind(6));
}
