// SPDX-License-Identifier: GPL-3.0-only
//! Golden tests: bytes in, screen text, colours and replies out.
extern crate std;
use super::*;
use crate::keys::{encode, paste, Key, Mods};
use alloc::string::ToString;

fn run(cols: usize, rows: usize, bytes: &[u8]) -> Screen {
    let mut s = Screen::new(cols, rows);
    let mut p = Parser::new();
    s.feed(&mut p, bytes);
    s
}

#[test]
fn plain_text_and_newlines() {
    let s = run(10, 3, b"hello\r\nworld");
    assert_eq!(s.text(), "hello\nworld\n");
    assert_eq!(s.cursor(), (5, 1));
}

#[test]
fn wraps_at_the_last_column_and_scrolls_into_history() {
    let s = run(4, 2, b"abcdefghij");
    assert_eq!(s.text(), "efgh\nij");
    assert_eq!(s.history_len(), 1);
}

#[test]
fn pending_wrap_lets_the_last_column_be_written_without_scrolling() {
    let s = run(4, 2, b"\x1b[2;1Habcd");
    assert_eq!(s.text(), "\nabcd");
    assert_eq!(s.history_len(), 0);
}

#[test]
fn cursor_positioning_and_erase() {
    let s = run(10, 3, b"xxxxxxxxxx\x1b[1;4H\x1b[K\x1b[3;2Hz");
    assert_eq!(s.text(), "xxx\n\n z");
}

#[test]
fn erase_display_below_and_above() {
    let s = run(4, 3, b"aaaa\r\nbbbb\r\ncccc\x1b[2;2H\x1b[J");
    assert_eq!(s.text(), "aaaa\nb\n");
    let s = run(4, 3, b"aaaa\r\nbbbb\r\ncccc\x1b[2;2H\x1b[1J");
    assert_eq!(s.text(), "\n  bb\ncccc");
}

#[test]
fn sgr_colours_basic_256_and_truecolour() {
    let s = run(20, 1, b"\x1b[31ma\x1b[1;42mb\x1b[38;5;208mc\x1b[38;2;1;2;3md\x1b[48:2::9:8:7me\x1b[0mf\x1b[95mg");
    let c = |x| s.visible(x, 0);
    assert_eq!(c(0).fg, Color::Indexed(1));
    assert_eq!((c(1).flags & BOLD, c(1).bg), (BOLD, Color::Indexed(2)));
    assert_eq!(c(2).fg, Color::Indexed(208));
    assert_eq!(c(3).fg, Color::Rgb(1, 2, 3));
    assert_eq!(c(4).bg, Color::Rgb(9, 8, 7));
    assert_eq!((c(5).fg, c(5).bg, c(5).flags), (Color::Default, Color::Default, 0));
    assert_eq!(c(6).fg, Color::Indexed(13));
    // Bold brightens the eight basic colours; inverse swaps.
    assert_eq!(colors(&c(1)).0, palette(9));
    let s = run(4, 1, b"\x1b[1;31mx\x1b[0;7my");
    assert_eq!(colors(&s.visible(0, 0)).0, palette(9));
    assert_eq!(colors(&s.visible(1, 0)), (DEFAULT_BG, DEFAULT_FG));
}

#[test]
fn background_colour_erase() {
    let s = run(4, 1, b"\x1b[44m\x1b[2K");
    assert_eq!(s.visible(3, 0).bg, Color::Indexed(4));
}

#[test]
fn insert_and_delete_characters() {
    let s = run(8, 1, b"abcdef\x1b[1;3H\x1b[2@XY");
    assert_eq!(s.text(), "abXYcdef");
    let s = run(8, 1, b"abcdef\x1b[1;2H\x1b[3P");
    assert_eq!(s.text(), "aef");
    let s = run(8, 1, b"abcdef\x1b[1;2H\x1b[2X");
    assert_eq!(s.text(), "a  def");
}

#[test]
fn insert_and_delete_lines_inside_a_scroll_region() {
    let s = run(3, 5, b"1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r\x1b[2;1H\x1b[L");
    assert_eq!(s.text(), "1\n\n2\n3\n5");
    let s = run(3, 5, b"1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r\x1b[2;1H\x1b[M");
    assert_eq!(s.text(), "1\n3\n4\n\n5");
    assert_eq!(s.history_len(), 0);
}

#[test]
fn scroll_region_keeps_header_and_footer() {
    let s = run(6, 4, b"head\x1b[4;1Hfoot\x1b[2;3r\x1b[3;1Ha\r\nb\r\nc");
    assert_eq!(s.text(), "head\nb\nc\nfoot");
    // A region not starting at the top never feeds the history.
    assert_eq!(s.history_len(), 0);
}

#[test]
fn reverse_index_at_top_scrolls_down() {
    let s = run(3, 3, b"a\r\nb\r\nc\x1b[H\x1bM");
    assert_eq!(s.text(), "\na\nb");
}

#[test]
fn tabs_stop_every_eight_columns() {
    let s = run(20, 1, b"a\tb\tc");
    assert_eq!(s.text(), "a       b       c");
    let s = run(20, 1, b"\x1b[3g\x1b[1;5H\x1bH\r\tx");
    assert_eq!(s.text(), "    x");
}

#[test]
fn cursor_report_and_device_attributes() {
    let mut s = run(10, 5, b"\x1b[3;7H\x1b[6n\x1b[c");
    assert_eq!(s.take_replies(), b"\x1b[3;7R\x1b[?62;22c".to_vec());
    assert!(s.take_replies().is_empty());
}

#[test]
fn alternate_screen_keeps_the_primary_and_history() {
    let mut s = run(5, 2, b"one\r\ntwo\r\nthree");
    let mut p = Parser::new();
    assert_eq!(s.history_len(), 1);
    s.feed(&mut p, b"\x1b[?1049h\x1b[HFULL");
    assert!(s.alternate());
    assert_eq!(s.text(), "FULL\n");
    s.feed(&mut p, b"\x1b[?1049l");
    assert_eq!(s.text(), "two\nthree");
    assert_eq!(s.cursor(), (5 - 1, 1));
}

#[test]
fn title_and_bell() {
    let mut s = run(5, 1, b"\x1b]0;my title\x07\x07");
    assert_eq!(s.take_title(), Some(&b"my title"[..]));
    assert_eq!(s.take_title(), None);
    assert_eq!(s.take_bells(), 1);
}

#[test]
fn utf8_split_across_feeds() {
    let mut s = Screen::new(5, 1);
    let mut p = Parser::new();
    let bytes = "é→".as_bytes();
    s.feed(&mut p, &bytes[..1]);
    s.feed(&mut p, &bytes[1..]);
    assert_eq!(s.text(), "é→");
}

#[test]
fn modes_cursor_keys_paste_and_cursor_visibility() {
    let s = run(5, 1, b"\x1b[?1h\x1b[?2004h\x1b[?25l");
    assert!(s.app_cursor && s.bracketed_paste && !s.cursor_visible);
    let s = run(5, 1, b"\x1b[?1h\x1b[?1l");
    assert!(!s.app_cursor);
}

#[test]
fn scrollback_view_and_selection_copy() {
    let mut s = run(6, 2, b"alpha\r\nbeta\r\ngamma\r\ndelta");
    assert_eq!(s.history_len(), 2);
    s.scroll_view(10);
    assert_eq!(s.view_offset(), 2);
    assert_eq!(s.text(), "alpha\nbeta");
    let (a, b) = (s.point_at(0, 0), s.point_at(5, 1));
    s.select(b, a);
    assert_eq!(s.selection_text(), "alpha\nbeta");
    assert!(s.selected(2, 1));
    // New output follows the bottom again.
    let mut p = Parser::new();
    s.feed(&mut p, b"!");
    assert_eq!(s.view_offset(), 0);
}

#[test]
fn soft_wrapped_lines_copy_as_one() {
    let mut s = run(4, 3, b"abcdefg\r\nxy");
    let (a, b) = (s.point_at(0, 0), s.point_at(3, 2));
    s.select(a, b);
    assert_eq!(s.selection_text(), "abcdefg\nxy");
}

#[test]
fn word_selection() {
    let s = run(20, 1, b"ls /data/term ok");
    let p = s.point_at(6, 0);
    let (a, b) = s.word_at(p);
    assert_eq!((a.col, b.col), (3, 12));
}

#[test]
fn resize_keeps_the_cursor_line() {
    let mut s = run(10, 4, b"1\r\n2\r\n3\r\n4");
    s.resize(10, 2);
    assert_eq!(s.text(), "3\n4");
    assert_eq!(s.cursor(), (1, 1));
    s.resize(10, 4);
    assert_eq!(s.text(), "1\n2\n3\n4");
    s.resize(3, 4);
    assert_eq!(s.cols(), 3);
}

#[test]
fn history_is_bounded() {
    let mut s = Screen::new(5, 2);
    s.set_history_max(3);
    let mut p = Parser::new();
    for i in 0..10 { s.feed(&mut p, i.to_string().as_bytes()); s.feed(&mut p, b"\r\n"); }
    assert_eq!(s.history_len(), 3);
}

#[test]
fn reset_and_decaln() {
    let s = run(3, 2, b"\x1b#8");
    assert_eq!(s.text(), "EEE\nEEE");
    let s = run(3, 2, b"abc\x1b[31m\x1bcx");
    assert_eq!(s.text(), "x\n");
    assert_eq!(s.visible(0, 0).fg, Color::Default);
}

#[test]
fn dirty_rows_track_changes() {
    let mut s = Screen::new(5, 3);
    let _ = s.take_dirty();
    let mut p = Parser::new();
    s.feed(&mut p, b"\x1b[2;1Hx");
    assert_eq!(s.take_dirty(), vec![false, true, false]);
}

#[test]
fn a_vttest_style_box_draws_exactly() {
    // The frame vttest's first screen draws: corners with `+`, edges with `*` and `-`,
    // using absolute moves, then a message centred inside it.
    let mut bytes = alloc::vec::Vec::new();
    bytes.extend_from_slice(b"\x1b[2J\x1b[1;1H+--------+\x1b[5;1H+--------+");
    for r in 2..5 { bytes.extend_from_slice(std::format!("\x1b[{r};1H*\x1b[{r};10H*").as_bytes()); }
    bytes.extend_from_slice(b"\x1b[3;3HVT ok");
    let s = run(10, 5, &bytes);
    assert_eq!(s.text(), "+--------+\n*        *\n* VT ok  *\n*        *\n+--------+");
}

#[test]
fn keys_encode_like_xterm() {
    let none = Mods::default();
    let ctrl = Mods { control: true, ..none };
    assert_eq!(encode(Key::Char('a'), none, false), b"a");
    assert_eq!(encode(Key::Char('c'), ctrl, false), [3]);
    assert_eq!(encode(Key::Char('x'), Mods { alt: true, ..none }, false), b"\x1bx");
    assert_eq!(encode(Key::Up, none, false), b"\x1b[A");
    assert_eq!(encode(Key::Up, none, true), b"\x1bOA");
    assert_eq!(encode(Key::Right, ctrl, true), b"\x1b[1;5C");
    assert_eq!(encode(Key::Enter, none, false), b"\r");
    assert_eq!(encode(Key::Backspace, none, false), [0x7f]);
    assert_eq!(encode(Key::Delete, none, false), b"\x1b[3~");
    assert_eq!(encode(Key::PageUp, Mods { shift: true, ..none }, false), b"\x1b[5;2~");
    assert_eq!(encode(Key::F(1), none, false), b"\x1bOP");
    assert_eq!(encode(Key::F(5), none, false), b"\x1b[15~");
    assert_eq!(encode(Key::F(12), none, false), b"\x1b[24~");
    assert_eq!(encode(Key::Tab, Mods { shift: true, ..none }, false), b"\x1b[Z");
    assert_eq!(encode(Key::Char('é'), none, false), "é".as_bytes());
}

#[test]
fn paste_is_bracketed_and_cannot_escape() {
    assert_eq!(paste(b"a\nb", false), b"a\rb");
    assert_eq!(paste(b"x\x1b[201~y", true), b"\x1b[200~xy\x1b[201~");
    assert_eq!(paste(b"a\r\nb", false), b"a\rb");
}

#[test]
fn the_vtcheck_fixture_matches_its_expected_screen() {
    let s = run(30, 8, check::FIXTURE.as_bytes());
    let rows: alloc::vec::Vec<String> = (0..6).map(|r| s.row_text(r)).collect();
    assert_eq!(rows, check::ROWS);
    for (x, y, fg, bg) in check::COLORS { assert_eq!(colors(&s.visible(x, y)), (fg, bg), "cell {x},{y}"); }
    assert_eq!(s.cursor(), (0, 6));
}
