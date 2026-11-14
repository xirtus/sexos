// SPDX-License-Identifier: GPL-3.0-only
//! Golden tests for the text protocols (OSC 8, 52, 7, 9, 777, 133), scrollback search,
//! quick-select hints and copy mode.
extern crate std;
use super::*;
use crate::copy_mode::{Action, CopyMode};
use crate::hints::{self, HintKind, Resolve};
use crate::keys::{Key, Mods};
use alloc::string::ToString;
use alloc::vec;
use std::format;

fn run(cols: usize, rows: usize, bytes: &[u8]) -> Screen {
    let mut s = Screen::new(cols, rows);
    let mut p = Parser::new();
    s.feed(&mut p, bytes);
    s
}
fn feed(s: &mut Screen, bytes: &[u8]) { let mut p = Parser::new(); s.feed(&mut p, bytes); }

#[test]
fn cell_stays_sixteen_bytes() {
    assert_eq!(core::mem::size_of::<Cell>(), 16);
}

// ---- OSC 8 ----

#[test]
fn osc8_links_cells_and_ends() {
    let s = run(30, 2, b"see \x1b]8;;https://example.com/a;b\x1b\\here\x1b]8;;\x1b\\ done");
    assert_eq!(s.text(), "see here done\n");
    assert_eq!(s.link_at(3, 0), None);
    assert_eq!(s.link_at(4, 0), Some("https://example.com/a;b"));
    assert_eq!(s.link_at(7, 0), Some("https://example.com/a;b"));
    assert_eq!(s.link_at(8, 0), None);
    assert_eq!(s.link_at(9, 0), None);
}

#[test]
fn osc8_survives_sgr_reset_and_reuses_entries() {
    let s = run(40, 2, b"\x1b]8;id=x;file:///a\x07A\x1b[0mB\x1b]8;;\x07 \x1b]8;id=x;file:///a\x07C\x1b]8;;\x07");
    assert_eq!(s.link_at(1, 0), Some("file:///a"));
    assert_eq!(s.visible(0, 0).link, s.visible(3, 0).link);
    assert_eq!(s.links_live(), 1);
}

#[test]
fn osc8_links_are_collected_when_their_lines_leave_the_history() {
    let mut s = Screen::new(20, 2);
    s.set_history_max(4);
    let mut p = Parser::new();
    for i in 0..2000 { s.feed(&mut p, format!("\x1b]8;;http://h/{i}\x1b\\L{i}\x1b]8;;\x1b\\\r\n").as_bytes()); }
    // Only the links on the 6 kept lines (and whatever the last sweep left) stay in the table.
    assert!(s.links_live() <= 512, "{} live", s.links_live());
    assert_eq!(s.link_at(0, 0), Some("http://h/1999"));
}

// ---- OSC 52, 7, 9, 777 ----

#[test]
fn osc52_set_and_query_become_events() {
    let mut s = run(10, 1, b"\x1b]52;c;aGVsbG8gd29ybGQ=\x07\x1b]52;p;?\x07\x1b]52;;Zm9v\x1b\\\x1b]52;c;!!\x07");
    assert_eq!(s.take_events(), vec![
        Event::Clipboard { target: 'c', set: b"hello world".to_vec() },
        Event::ClipboardQuery { target: 'p' },
        Event::Clipboard { target: 'c', set: b"foo".to_vec() },
    ]);
    assert!(s.take_events().is_empty());
    s.reply_clipboard('c', b"hi!");
    assert_eq!(s.take_replies(), b"\x1b]52;c;aGkh\x1b\\".to_vec());
}

#[test]
fn osc52_large_payloads_arrive_whole_or_not_at_all() {
    let text = "x".repeat(9000);
    let mut b64 = Vec::new();
    osc::base64_encode(text.as_bytes(), &mut b64);
    let mut bytes = b"\x1b]52;c;".to_vec();
    bytes.extend_from_slice(&b64);
    bytes.push(7);
    let mut s = run(10, 1, &bytes);
    assert_eq!(s.take_events(), vec![Event::Clipboard { target: 'c', set: text.into_bytes() }]);
    // Too long for the parser: dropped rather than cut short.
    let mut bytes = b"\x1b]52;c;".to_vec();
    bytes.extend(core::iter::repeat_n(b'Q', 20000));
    bytes.push(7);
    let mut s = run(10, 1, &bytes);
    assert!(s.take_events().is_empty());
}

#[test]
fn base64_round_trips() {
    for n in 0..20 {
        let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
        let mut enc = Vec::new();
        osc::base64_encode(&data, &mut enc);
        assert_eq!(osc::base64_decode(&enc), Some(data));
    }
}

#[test]
fn notifications_cwd_and_progress() {
    let mut s = run(10, 1, concat!(
        "\x1b]9;build done\x07",
        "\x1b]777;notify;Make;all; good\x07",
        "\x1b]7;file://box/home/me/My%20Dir\x07",
        "\x1b]9;4;1;42\x07\x1b]9;4;0;0\x07\x1b]9;4;3\x07",
        "\x1b]9;9;\"/tmp\"\x07",
    ).as_bytes());
    assert_eq!(s.take_events(), vec![
        Event::Notify { title: String::new(), body: "build done".to_string() },
        Event::Notify { title: "Make".to_string(), body: "all; good".to_string() },
        Event::Cwd("/home/me/My Dir".to_string()),
        Event::Progress(Progress::Set(42)),
        Event::Progress(Progress::Clear),
        Event::Progress(Progress::Indeterminate),
        Event::Cwd("/tmp".to_string()),
    ]);
    assert_eq!(s.cwd(), Some("/tmp"));
    // The title still works alongside.
    let mut s = run(10, 1, b"\x1b]2;t\x07");
    assert_eq!(s.take_title(), Some(&b"t"[..]));
    assert!(s.take_events().is_empty());
}

// ---- OSC 133 ----

/// A shell session: three commands, each prompt `$ `, with OSC 133 marks.
fn session(s: &mut Screen, p: &mut Parser, cmd: &str, out: &[&str], exit: i32) {
    s.feed(p, b"\x1b]133;A\x07$ \x1b]133;B\x07");
    s.feed(p, cmd.as_bytes());
    s.feed(p, b"\r\n\x1b]133;C\x07");
    for l in out { s.feed(p, l.as_bytes()); s.feed(p, b"\r\n"); }
    s.feed(p, format!("\x1b]133;D;{exit}\x07").as_bytes());
}

#[test]
fn osc133_marks_prompts_and_selects_output() {
    let mut s = Screen::new(20, 5);
    let mut p = Parser::new();
    session(&mut s, &mut p, "ls", &["a", "b"], 0);
    session(&mut s, &mut p, "false", &[], 1);
    session(&mut s, &mut p, "echo hi", &["hi"], 0);
    s.feed(&mut p, b"\x1b]133;A\x07$ ");
    // Rows: 0 "$ ls", 1 a, 2 b, 3 "$ false", 4 "$ echo hi", 5 hi, 6 "$ ".
    assert_eq!(s.prompt_rows(), vec![0, 3, 4, 6]);
    let c = s.commands();
    assert_eq!(c[0].exit, Some(0));
    assert_eq!(c[1].exit, Some(1));
    assert_eq!(c[0].input, Some(Point { row: 0, col: 2 }));
    assert_eq!(c[3].end, None);
    assert_eq!(s.command_output(0), Some((Point { row: 1, col: 0 }, Point { row: 2, col: 0 })));
    assert_eq!(s.command_output(3), None);
    assert!(s.select_command_output(5));
    assert_eq!(s.selection_text(), "hi");
    // Jumping between prompts scrolls the view.
    assert_eq!(s.history_len(), 2);
    assert_eq!(s.jump_prompt(-1), Some(0));
    assert_eq!(s.view_offset(), 2);
    assert_eq!(s.row_text(0), "$ ls");
    assert_eq!(s.jump_prompt(-1), None);
    // Row 3 cannot reach the top (the history is short): the view stops at the bottom.
    assert_eq!(s.jump_prompt(1), Some(3));
    assert_eq!(s.view_offset(), 0);
    assert_eq!(s.jump_prompt(1), None);
    assert_eq!(s.jump_prompt(-1), Some(0));
}

#[test]
fn osc133_marks_follow_the_history_trim() {
    let mut s = Screen::new(20, 3);
    s.set_history_max(5);
    let mut p = Parser::new();
    for i in 0..10 { session(&mut s, &mut p, &format!("c{i}"), &["out"], 0); }
    let rows = s.prompt_rows();
    assert!(!rows.is_empty());
    for r in rows { assert!(s.line_text(r).starts_with("$ c"), "row {r}: {:?}", s.line_text(r)); }
    // Clearing the scrollback forgets the marks in it.
    s.feed(&mut p, b"\x1b[3J");
    for r in s.prompt_rows() { assert!(s.line_text(r).starts_with("$ c")); }
}

// ---- Search ----

#[test]
fn search_plain_case_and_soft_wrap() {
    let s = run(6, 3, b"Hello world\r\nhello again");
    // "Hello " / "world" (wrapped) / "hello " / "again", the first scrolled into history.
    assert_eq!(s.history_len(), 1);
    let m = s.search("hello", false, true);
    assert_eq!(m, vec![Match { start: Point { row: 2, col: 0 }, end: Point { row: 2, col: 4 } }]);
    let m = s.search("HELLO", false, false);
    assert_eq!(m.len(), 2);
    // Across the soft wrap: "o w" starts on row 0 and ends on row 1.
    let m = s.search("o wor", false, true);
    assert_eq!(m, vec![Match { start: Point { row: 0, col: 4 }, end: Point { row: 1, col: 2 } }]);
    // A hard line break is not joined.
    assert!(s.search("world hello", false, true).is_empty());
    assert!(s.search("", false, true).is_empty());
}

#[cfg(feature = "regex")]
#[test]
fn search_regex() {
    let s = run(30, 3, b"error: 12 failed\r\nok: 3 passed\r\nError: 7 failed");
    let m = s.search(r"\d+ failed", true, true);
    assert_eq!(m.len(), 2);
    assert_eq!(m[0], Match { start: Point { row: 0, col: 7 }, end: Point { row: 0, col: 15 } });
    assert_eq!(s.search("^error", true, true).len(), 1);
    assert_eq!(s.search("^error", true, false).len(), 2);
    assert!(s.search("(unclosed", true, true).is_empty());
}

#[test]
fn highlights_paint_and_follow_the_history() {
    let mut s = Screen::new(10, 2);
    s.set_history_max(100);
    let mut p = Parser::new();
    s.feed(&mut p, b"one\r\nfind me\r\nthree\r\nfour");
    let m = s.search("me", false, true);
    assert_eq!(m[0].start, Point { row: 1, col: 5 });
    s.set_highlights(m, Some(0));
    s.scroll_to(1);
    assert_eq!(s.row_text(0), "find me");
    assert_eq!(s.highlight(5, 0), Highlight::Current);
    assert_eq!(s.highlight(6, 0), Highlight::Current);
    assert_eq!(s.highlight(4, 0), Highlight::None);
    // Dropping a history line moves the stored highlight with it.
    s.set_history_max(1);
    assert_eq!(s.highlights()[0].start, Point { row: 0, col: 5 });
    s.scroll_to(0);
    assert_eq!(s.row_text(0), "find me");
    assert_eq!(s.highlight(5, 0), Highlight::Current);
    s.clear_highlights();
    assert_eq!(s.highlight(5, 0), Highlight::None);
}

// ---- Hints ----

#[test]
fn hints_find_the_usual_things() {
    let s = run(80, 8, concat!(
        "see https://example.com/x_(y). and www.rust-lang.org,\r\n",
        "src/main.rs:12:5 and ./run.sh or lib.rs:7 not lib.rs\r\n",
        "commit 3f2a9c1d merged; abcdefg is a word\r\n",
        "ping 192.168.1.20:8080 or fe80::1 at 12:30:45\r\n",
        "mail me@host.example.org, pid 4242 not 123\r\n",
    ).as_bytes());
    let h = hints::find(&s);
    let got: Vec<(HintKind, &str)> = h.iter().map(|h| (h.kind, h.text.as_str())).collect();
    assert_eq!(got, vec![
        (HintKind::Url, "https://example.com/x_(y)"),
        (HintKind::Url, "www.rust-lang.org"),
        (HintKind::Path, "src/main.rs:12:5"),
        (HintKind::Path, "./run.sh"),
        (HintKind::Path, "lib.rs:7"),
        (HintKind::Hash, "3f2a9c1d"),
        (HintKind::Ip, "192.168.1.20:8080"),
        (HintKind::Ip, "fe80::1"),
        (HintKind::Email, "me@host.example.org"),
        (HintKind::Number, "4242"),
    ]);
    assert_eq!((h[0].label.as_str(), h[1].label.as_str()), ("a", "s"));
    assert_eq!((h[0].col, h[0].row, h[0].end_col), (4, 0, 28));
    assert_eq!(hints::resolve(&h, "s"), Resolve::Exact(1));
    assert_eq!(hints::resolve(&h, "x"), Resolve::None);
}

#[test]
fn hints_follow_soft_wraps() {
    let s = run(10, 3, b"> https://a.example/b");
    let h = hints::find(&s);
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].text, "https://a.example/b");
    assert_eq!((h[0].col, h[0].row, h[0].end_col, h[0].end_row), (2, 0, 0, 2));
}

#[test]
fn hint_labels_are_prefix_free_and_short() {
    for n in [0, 1, 26, 27, 100, 676, 677, 2000] {
        let l = hints::labels(n);
        assert_eq!(l.len(), n);
        for (i, a) in l.iter().enumerate() {
            for (j, b) in l.iter().enumerate() { if i != j { assert!(!b.starts_with(a.as_str()), "{a} prefixes {b} (n={n})"); } }
        }
        if n <= 26 { assert!(l.iter().all(|s| s.len() == 1)); }
    }
    let l = hints::labels(30);
    assert_eq!(l.iter().filter(|s| s.len() == 1).count(), 25);
    assert_eq!(&l[..2], &["a".to_string(), "s".to_string()]);
}

// ---- Selection ----

#[test]
fn selection_modes_and_soft_wraps() {
    let mut s = run(5, 4, b"abcdefgh\r\n12345\r\nxyz");
    // Rows: "abcde" (wraps), "fgh", "12345", "xyz".
    s.select(Point { row: 0, col: 2 }, Point { row: 1, col: 1 });
    assert_eq!(s.selection_text(), "cdefg");
    s.select_mode(Point { row: 0, col: 3 }, Point { row: 2, col: 1 }, SelectionMode::Line);
    assert_eq!(s.selection_text(), "abcdefgh\n12345");
    assert!(s.selected(0, 0) && s.selected(4, 2));
    s.select_mode(Point { row: 3, col: 2 }, Point { row: 0, col: 1 }, SelectionMode::Block);
    assert_eq!(s.selection_text(), "bc\ngh\n23\nyz");
    assert!(s.selected(1, 1) && !s.selected(0, 1) && !s.selected(3, 1));
}

// ---- Copy mode ----

fn k(c: char) -> Key { Key::Char(c) }
const NO: Mods = Mods { shift: false, alt: false, control: false };
const CTRL: Mods = Mods { shift: false, alt: false, control: true };

#[test]
fn copy_mode_moves_selects_and_yanks() {
    let mut s = Screen::new(20, 3);
    feed(&mut s, b"first line here\r\nsecond: word-two\r\nthird");
    let mut cm = CopyMode::new(&mut s);
    assert_eq!(cm.cursor(), Point { row: 2, col: 5 });
    assert_eq!(s.copy_cursor(), Some(Point { row: 2, col: 5 }));
    for key in [k('k'), k('0')] { assert_eq!(cm.key(&mut s, key, NO), Action::Redraw); }
    assert_eq!(cm.cursor(), Point { row: 1, col: 0 });
    cm.key(&mut s, k('w'), NO);
    assert_eq!(cm.cursor(), Point { row: 1, col: 6 }); // ':'
    cm.key(&mut s, k('w'), NO);
    assert_eq!(cm.cursor(), Point { row: 1, col: 8 }); // "word"
    cm.key(&mut s, k('e'), NO);
    assert_eq!(cm.cursor(), Point { row: 1, col: 11 });
    cm.key(&mut s, k('b'), NO);
    assert_eq!(cm.cursor(), Point { row: 1, col: 8 });
    cm.key(&mut s, k('v'), NO);
    cm.key(&mut s, k('$'), NO);
    assert_eq!(s.selection_text(), "word-two");
    assert_eq!(cm.key(&mut s, k('y'), NO), Action::Yank("word-two".to_string()));
    assert_eq!(s.copy_cursor(), None);
    assert_eq!(s.selection(), None);
}

#[test]
fn copy_mode_line_and_block_selection() {
    let mut s = Screen::new(10, 3);
    feed(&mut s, b"abcd\r\nefgh\r\nijkl");
    let mut cm = CopyMode::new(&mut s);
    cm.key(&mut s, k('g'), NO);
    cm.key(&mut s, k('V'), NO);
    cm.key(&mut s, k('j'), NO);
    assert_eq!(s.selection_text(), "abcd\nefgh");
    cm.key(&mut s, k('l'), NO);
    cm.key(&mut s, k('v'), CTRL);
    cm.key(&mut s, Key::Down, NO);
    cm.key(&mut s, Key::Right, NO);
    // Anchor stays where V started (0,0); block from col 0 to 2 over rows 0..=2.
    assert_eq!(s.selection().map(|(_, _, m)| m), Some(SelectionMode::Block));
    assert_eq!(s.selection_text(), "abc\nefg\nijk");
    assert_eq!(cm.key(&mut s, Key::Escape, NO), Action::Exit);
    assert_eq!(s.selection(), None);
}

#[test]
fn copy_mode_search_and_scrolling() {
    let mut s = Screen::new(12, 3);
    s.set_history_max(100);
    let mut p = Parser::new();
    for i in 0..20 { s.feed(&mut p, format!("line {i}\r\n").as_bytes()); }
    s.feed(&mut p, b"$ ");
    let mut cm = CopyMode::new(&mut s);
    for c in "?line 1".chars() { cm.key(&mut s, k(c), NO); }
    assert_eq!(cm.search_prompt(), Some(("line 1", false)));
    cm.key(&mut s, Key::Enter, NO);
    // Backward from the bottom: "line 19" (row 19) is the first match before the cursor.
    assert_eq!(cm.cursor(), Point { row: 19, col: 0 });
    cm.key(&mut s, k('n'), NO);
    assert_eq!(cm.cursor(), Point { row: 18, col: 0 });
    cm.key(&mut s, k('N'), NO);
    assert_eq!(cm.cursor(), Point { row: 19, col: 0 });
    for _ in 0..10 { cm.key(&mut s, k('n'), NO); }
    assert_eq!(cm.cursor(), Point { row: 1, col: 0 });
    // The view follows the cursor.
    assert_eq!(s.row_text(0), "line 1");
    assert_eq!(s.highlight(0, 0), Highlight::Current);
    cm.key(&mut s, k('u'), CTRL);
    assert_eq!(cm.cursor(), Point { row: 0, col: 0 });
    assert_eq!(s.row_text(0), "line 0");
    cm.key(&mut s, k('G'), NO);
    assert_eq!(s.view_offset(), 0);
    assert_eq!(cm.key(&mut s, k('q'), NO), Action::Exit);
    assert_eq!(s.highlight(0, 0), Highlight::None);
}

#[test]
fn copy_mode_keeps_its_place_when_history_is_trimmed() {
    let mut s = Screen::new(10, 2);
    s.set_history_max(3);
    let mut p = Parser::new();
    s.feed(&mut p, b"a\r\nb\r\nc\r\nd");
    let mut cm = CopyMode::new(&mut s);
    cm.key(&mut s, k('k'), NO);
    assert_eq!(s.line_text(cm.cursor().row), "c");
    s.feed(&mut p, b"\r\ne\r\nf");
    cm.key(&mut s, k('l'), NO);
    assert_eq!(s.line_text(cm.cursor().row), "c");
}
