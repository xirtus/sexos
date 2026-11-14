// SPDX-License-Identifier: GPL-3.0-only
extern crate std;
use super::*;
use crate::find::{self, Options};
use crate::highlight::{line, Highlighter, Kind, Lang, Span, State};
use alloc::vec::Vec;

fn doc(s: &str) -> Doc { Doc::new(s.as_bytes().to_vec()) }
fn text(d: &Doc) -> &str { core::str::from_utf8(d.text()).unwrap() }
fn typed(d: &mut Doc, s: &str) { for c in s.chars() { let mut b = [0; 4]; if c == '\n' { d.newline() } else { d.insert(c.encode_utf8(&mut b).as_bytes()) } } }

#[test]
fn typing_merges_into_word_sized_undo_steps() {
    let mut d = doc("");
    typed(&mut d, "hello world again");
    assert_eq!(text(&d), "hello world again");
    // "hello ", "world ", "again": three steps.
    assert!(d.undo());
    assert_eq!(text(&d), "hello world ");
    assert!(d.undo());
    assert_eq!(text(&d), "hello ");
    assert!(d.undo());
    assert_eq!(text(&d), "");
    assert!(!d.undo());
    assert!(d.redo() && d.redo());
    assert_eq!(text(&d), "hello world ");
}

#[test]
fn the_gate_sequence_edit_undo_five_redo_two() {
    // What the Jot gate does: five separate edits, undo all five, redo two.
    let mut d = doc("base\n");
    d.move_to(Motion::End_, false);
    for w in ["one ", "two ", "three ", "four ", "five "] { typed(&mut d, w); d.seal(); }
    assert_eq!(text(&d), "base\none two three four five ");
    for _ in 0..5 { assert!(d.undo()); }
    assert_eq!(text(&d), "base\n");
    assert!(!d.modified());
    d.redo(); d.redo();
    assert_eq!(text(&d), "base\none two ");
    assert!(d.modified());
}

#[test]
fn backspace_and_delete_merge_and_restore_the_cursor() {
    let mut d = doc("abcdef");
    d.move_to(Motion::End_, false);
    for _ in 0..3 { d.backspace(); }
    assert_eq!(text(&d), "abc");
    d.undo();
    assert_eq!((text(&d), d.cursor), ("abcdef", 6));
    d.move_to(Motion::Start, false);
    d.delete(); d.delete();
    assert_eq!((text(&d), d.cursor), ("cdef", 0));
    d.undo();
    assert_eq!(text(&d), "abcdef");
}

#[test]
fn undo_tree_keeps_the_branch_a_new_edit_would_lose() {
    let mut d = doc("");
    typed(&mut d, "first ");
    d.undo();
    typed(&mut d, "second ");
    assert_eq!(text(&d), "second ");
    d.undo();
    assert_eq!(text(&d), "");
    // Redo follows the newest branch; switching reaches the older one.
    assert!(d.switch_branch());
    d.redo();
    assert_eq!(text(&d), "first ");
}

#[test]
fn saved_point_survives_undo_and_redo() {
    let mut d = doc("x");
    d.move_to(Motion::End_, false);
    typed(&mut d, "y ");
    d.mark_saved();
    assert!(!d.modified());
    d.undo();
    assert!(d.modified());
    d.redo();
    assert!(!d.modified());
}

#[test]
fn selection_typing_cut_and_movement() {
    let mut d = doc("hello world");
    d.move_to(Motion::WordRight, false);
    assert_eq!(d.cursor, 5);
    d.move_to(Motion::WordRight, true);
    assert_eq!(d.selected(), b" world");
    d.insert(b"!");
    assert_eq!(text(&d), "hello!");
    d.select_all();
    assert_eq!(d.cut(), b"hello!");
    assert_eq!(text(&d), "");
    d.undo();
    assert_eq!(text(&d), "hello!");
}

#[test]
fn up_and_down_keep_the_column_across_short_lines() {
    let mut d = doc("long line here\nab\nanother long one");
    d.move_to(Motion::To(10), false);
    d.move_to(Motion::Down, false);
    assert_eq!(d.cursor, 17); // end of "ab"
    d.move_to(Motion::Down, false);
    assert_eq!(d.column(d.cursor), 10);
    d.move_to(Motion::Up, false);
    d.move_to(Motion::Up, false);
    assert_eq!(d.cursor, 10);
}

#[test]
fn home_toggles_indent_and_newline_keeps_it() {
    let mut d = doc("    code");
    d.move_to(Motion::End_, false);
    d.move_to(Motion::Home, false);
    assert_eq!(d.cursor, 4);
    d.move_to(Motion::Home, false);
    assert_eq!(d.cursor, 0);
    d.move_to(Motion::End_, false);
    d.newline();
    typed(&mut d, "more");
    assert_eq!(text(&d), "    code\n    more");
}

#[test]
fn utf8_moves_by_characters() {
    let mut d = doc("aé→b");
    d.move_to(Motion::End_, false);
    d.backspace();
    d.backspace();
    assert_eq!(text(&d), "aé");
    d.move_to(Motion::Left, false);
    assert_eq!(d.cursor, 1);
}

#[test]
fn find_wraps_and_replace_all_is_one_step() {
    let mut d = doc("cat Cat cat concat");
    let o = Options::default();
    assert!(find::find_next(&mut d, b"cat", false, o));
    assert_eq!(d.selection(), Some((0, 3)));
    assert!(find::find_next(&mut d, b"cat", false, o));
    assert_eq!(d.selection(), Some((8, 11)));
    assert!(find::find_next(&mut d, b"cat", false, o));
    assert_eq!(d.selection(), Some((15, 18)));
    assert!(find::find_next(&mut d, b"cat", false, o));
    assert_eq!(d.selection(), Some((0, 3)));
    let ci = Options { ignore_case: true, whole_word: true };
    assert_eq!(find::all(d.text(), b"cat", ci).len(), 3);
    assert_eq!(find::replace_all(&mut d, b"cat", b"dog", ci), 3);
    assert_eq!(text(&d), "dog dog dog concat");
    d.undo();
    assert_eq!(text(&d), "cat Cat cat concat");
}

#[test]
fn replace_next_steps_through_matches() {
    let mut d = doc("a-a-a");
    let o = Options::default();
    find::find_next(&mut d, b"a", false, o);
    find::replace_next(&mut d, b"a", b"bb", o);
    assert_eq!(text(&d), "bb-a-a");
    assert_eq!(d.selected(), b"a");
    find::find_next(&mut d, b"a", true, o);
    assert_eq!(d.selection(), Some((5, 6)));
}

fn kinds(spans: &[Span], t: &str) -> Vec<(std::string::String, Kind)> {
    spans.iter().map(|s| (t[s.start..s.end].into(), s.kind)).collect()
}

#[test]
fn rust_tokens() {
    let t = r#"pub fn main() -> Result<u8, E> { let x = 42; println!("hi {x}"); 'a' } // done"#;
    let (spans, _) = line(Lang::Rust, t.as_bytes(), State::default());
    let k = kinds(&spans, t);
    for want in [("pub", Kind::Keyword), ("fn", Kind::Keyword), ("Result", Kind::Type), ("u8", Kind::Type), ("let", Kind::Keyword), ("42", Kind::Number), ("println!", Kind::Macro), ("\"hi {x}\"", Kind::String), ("'a'", Kind::String), ("// done", Kind::Comment)] {
        assert!(k.contains(&(want.0.into(), want.1)), "{want:?} in {k:?}");
    }
    let (spans, _) = line(Lang::Rust, b"fn f<'a>(x: &'a str)", State::default());
    assert!(kinds(&spans, "fn f<'a>(x: &'a str)").contains(&("'a".into(), Kind::Type)));
}

#[test]
fn block_comments_and_strings_cross_lines() {
    let mut h = Highlighter::new(Lang::Rust);
    let t = "let a = 1; /* start\nstill comment\nend */ let b = \"x\ny\";\nfn";
    let v = h.visible(t.as_bytes(), 0, 5);
    assert_eq!(v[1], [Span { start: 0, end: 13, kind: Kind::Comment }]);
    assert_eq!(v[2][0], Span { start: 0, end: 6, kind: Kind::Comment });
    assert_eq!(v[3][0], Span { start: 0, end: 2, kind: Kind::String });
    assert_eq!(v[4][0].kind, Kind::Keyword);
    // Asking for a later window alone gives the same answer.
    let mut h2 = Highlighter::new(Lang::Rust);
    assert_eq!(h2.visible(t.as_bytes(), 3, 2), v[3..5]);
}

#[test]
fn c_and_js_tokens() {
    let t = "#include <stdio.h>";
    assert_eq!(line(Lang::C, t.as_bytes(), State::default()).0[0].kind, Kind::Macro);
    let t = "static int count = 0x1F; // n";
    let k = kinds(&line(Lang::C, t.as_bytes(), State::default()).0, t);
    assert!(k.contains(&("static".into(), Kind::Keyword)) && k.contains(&("int".into(), Kind::Type)) && k.contains(&("0x1F".into(), Kind::Number)));
    let t = "const s = `a ${b}`; let c = 'x';";
    let k = kinds(&line(Lang::Js, t.as_bytes(), State::default()).0, t);
    assert!(k.contains(&("`a ${b}`".into(), Kind::String)) && k.contains(&("'x'".into(), Kind::String)) && k.contains(&("const".into(), Kind::Keyword)));
}

#[test]
fn markdown_and_toml() {
    let mut h = Highlighter::new(Lang::Markdown);
    let t = "# Title\nsome *em* and `code` and [link](http://x)\n```\nfn x\n```\n- item";
    let v = h.visible(t.as_bytes(), 0, 6);
    assert_eq!(v[0][0].kind, Kind::Heading);
    let l1 = "some *em* and `code` and [link](http://x)";
    let k = kinds(&v[1], l1);
    assert!(k.contains(&("*em*".into(), Kind::Emphasis)) && k.contains(&("`code`".into(), Kind::Code)) && k.contains(&("[link](http://x)".into(), Kind::Link)));
    assert_eq!(v[3][0].kind, Kind::Code);
    assert_eq!(v[5][0].kind, Kind::Punct);
    let t = "[package]\nname = \"jot\" # c\nversion = 1\nok = true";
    let mut h = Highlighter::new(Lang::Toml);
    let v = h.visible(t.as_bytes(), 0, 4);
    assert_eq!(v[0][0].kind, Kind::Heading);
    assert_eq!(kinds(&v[1], "name = \"jot\" # c"), [("name".into(), Kind::Key), ("\"jot\"".into(), Kind::String), ("# c".into(), Kind::Comment)]);
    assert_eq!(v[2][1].kind, Kind::Number);
    assert_eq!(v[3][1].kind, Kind::Keyword);
}

#[test]
fn languages_from_names() {
    assert_eq!(Lang::from_name(b"main.rs"), Lang::Rust);
    assert_eq!(Lang::from_name(b"a/b/README.md"), Lang::Markdown);
    assert_eq!(Lang::from_name(b"Cargo.toml"), Lang::Toml);
    assert_eq!(Lang::from_name(b"x.H"), Lang::C);
    assert_eq!(Lang::from_name(b"notes.txt"), Lang::Plain);
    assert_eq!(Lang::from_name(b"Makefile"), Lang::Plain);
}

// ---- Encodings, line ends, wrap, Markdown, sessions (O12) ----

use crate::encoding::{decode, encode, Encoding, Eol, Format};
use alloc::vec;

#[test]
fn files_round_trip_in_every_encoding_and_line_end() {
    let text = "Grüße, “quotes” and € — ok\nsecond line\n".as_bytes();
    for enc in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Windows1252] {
        for eol in [Eol::Lf, Eol::CrLf] {
            let f = Format { encoding: enc, eol };
            let (bytes, lost) = encode(text, f);
            assert_eq!(lost, 0, "{enc:?}");
            let (back, got) = decode(&bytes);
            // Windows-1252 without a mark reads back as itself only when not valid UTF-8.
            assert_eq!(back, text, "{enc:?} {eol:?}");
            assert_eq!(got, f, "{enc:?} {eol:?}");
        }
    }
}

#[test]
fn crlf_files_edit_as_lf_and_save_as_crlf() {
    let (t, f) = decode(b"a\r\nb\r\nc");
    assert_eq!((t.as_slice(), f.eol), (&b"a\nb\nc"[..], Eol::CrLf));
    let mut d = Doc::new(t);
    d.move_to(Motion::End_, false);
    d.newline();
    d.insert(b"d");
    assert_eq!(encode(d.text(), f).0, b"a\r\nb\r\nc\r\nd");
    // A lone \r is kept; mostly-LF files stay LF.
    assert_eq!(decode(b"x\ry\nz\n").1.eol, Eol::Lf);
    assert_eq!(encode("€ and ☃".as_bytes(), Format { encoding: Encoding::Windows1252, eol: Eol::Lf }), (b"\x80 and ?".to_vec(), 1));
    assert_eq!(decode(b"caf\xe9").0, "café".as_bytes());
}

#[test]
fn soft_wrap_breaks_after_spaces() {
    use crate::wrap::{row_of, rows};
    let line = b"the quick brown fox jumps";
    let r = rows(line, 10);
    let parts: Vec<&[u8]> = r.iter().map(|(a, b)| &line[*a..*b]).collect();
    assert_eq!(parts, vec![&b"the quick "[..], b"brown fox ", b"jumps"]);
    assert_eq!(row_of(&r, 0), 0);
    assert_eq!(row_of(&r, 10), 1);
    assert_eq!(row_of(&r, line.len()), 2);
    let long = b"abcdefghijklmnop";
    let r = rows(long, 5);
    assert_eq!(r, vec![(0, 5), (5, 10), (10, 15), (15, 16)]);
    assert_eq!(rows(b"", 5), vec![(0, 0)]);
    assert_eq!(rows("ééééé".as_bytes(), 2), vec![(0, 4), (4, 8), (8, 10)]);
}

#[test]
fn markdown_blocks_and_inline_styles() {
    use crate::markdown::{inline, parse, Style};
    let md = "# Title\n\nSome **bold** and *it* and `code`,\njoined line.\n\n- one\n- [x] done\n1. first\n\n> quoted\n> more\n\n```\nfn x() {}\n```\n\n---\nsnake_case_word and [a link](http://x)\n";
    let b = parse(md);
    assert_eq!(b[0].runs, vec![("Title".into(), Style::Heading(1))]);
    assert_eq!(b[1].runs, vec![("Some ".into(), Style::Plain), ("bold".into(), Style::Strong), (" and ".into(), Style::Plain), ("it".into(), Style::Emphasis),
        (" and ".into(), Style::Plain), ("code".into(), Style::Code), (", joined line.".into(), Style::Plain)]);
    assert_eq!((b[2].marker.as_str(), b[3].marker.as_str(), b[4].marker.as_str()), ("• ", "☑ ", "1. "));
    assert_eq!(b[5].runs[0].0, "quoted more");
    assert!(b[6].code && b[6].runs[0].0 == "fn x() {}");
    assert_eq!(b[7].runs[0].1, Style::Rule);
    assert_eq!(b[8].runs, vec![("snake_case_word and ".into(), Style::Plain), ("a link".into(), Style::Link)]);
    assert_eq!(b[8].line, 17);
    assert_eq!(inline("~~gone~~ **_both_**", Style::Plain), vec![("gone".into(), Style::Strike), (" ".into(), Style::Plain), ("both".into(), Style::Strong)]);
    assert_eq!(parse("Title\n=====\ntext")[0].runs[0].1, Style::Heading(1));
}

#[test]
fn sessions_survive_and_reject_damage() {
    use crate::session::{read, write, Saved};
    let tabs = vec![
        Saved { path: b"notes.txt".to_vec(), text: b"unsaved\nwork".to_vec(), cursor: 7, format: Format::default() },
        Saved { path: Vec::new(), text: "new ✓".as_bytes().to_vec(), cursor: 0, format: Format { encoding: Encoding::Utf16Le, eol: Eol::CrLf } },
    ];
    let bytes = write(&tabs);
    assert_eq!(read(&bytes), Some(tabs.clone()));
    assert_eq!(read(&bytes[..bytes.len() - 3]), None, "cut short by a crash");
    let mut bad = bytes.clone();
    bad[20] ^= 1;
    assert_eq!(read(&bad), None);
    assert_eq!(read(&write(&[])), Some(Vec::new()));
}
