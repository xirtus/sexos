// SPDX-License-Identifier: GPL-3.0-only
mod common;
use common::*;
use spindle_shell::{render, Engine, Record, Value};

#[test]
fn golden_big_files_sorted_by_name() {
    assert_eq!(ok("ls | where size > 1mb | sort-by name"), "\
╭───┬─────────────┬──────┬─────────┬──────────────────────╮
│ # │    name     │ type │  size   │       modified       │
├───┼─────────────┼──────┼─────────┼──────────────────────┤
│ 0 │ archive.tar │ file │ 2.4 MiB │ 2026-11-01T12:01:00Z │
│ 1 │ disk.img    │ file │   3 GiB │ 2026-11-01T13:00:00Z │
│ 2 │ video.mkv   │ file │ 700 MiB │ 2026-11-01T12:00:00Z │
╰───┴─────────────┴──────┴─────────┴──────────────────────╯
");
}

#[test]
fn golden_sort_reverse_by_size() {
    assert_eq!(ok("ls | where size > 1mb | sort-by size -r | select name size"), "\
╭───┬─────────────┬─────────╮
│ # │    name     │  size   │
├───┼─────────────┼─────────┤
│ 0 │ disk.img    │   3 GiB │
│ 1 │ video.mkv   │ 700 MiB │
│ 2 │ archive.tar │ 2.4 MiB │
╰───┴─────────────┴─────────╯
");
}

#[test]
fn colours_dirs_sizes_and_headers() {
    let mut e = Engine::new();
    let mut h = host();
    let mut text = String::new();
    let programs = host().programs;
    e.color = true;
    let out = e.run_sync("ls | where type == dir", &mut h, 80, &mut |_, r| { let mut m = spindle_shell::MemHost::new(); m.programs = programs.clone(); m.run(r) });
    text.push_str(&out.text);
    assert!(text.contains("\x1b[1;34mdocs\x1b[0m"), "{}", text);
    assert!(text.contains("\x1b[2m0 B\x1b[0m"), "sizes dim: {}", text);
    assert!(text.contains("\x1b[1;32mname\x1b[0m"), "header: {}", text);
    assert!(text.contains("\x1b[1;32m0\x1b[0m"), "index: {}", text);
    // The same table without colour has the same visible text.
    let plain: String = strip_ansi(&text);
    assert_eq!(plain, ok("ls | where type == dir"));
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for c in s.chars() {
        if esc { if c.is_ascii_alphabetic() { esc = false; } continue; }
        if c == '\x1b' { esc = true; continue; }
        out.push(c);
    }
    out
}

#[test]
fn errors_render_red() {
    let v = Value::Error(Box::new(spindle_shell::ShellError::new("disk on fire")));
    assert_eq!(render::render(&v, 80, true), "\x1b[31merror: disk on fire\x1b[0m\n");
    let t = Value::List(vec![Value::Record(Record::new().with("a", v))]);
    assert!(render::render(&t, 80, true).contains("\x1b[31merror: disk on fire\x1b[0m"));
}

#[test]
fn narrow_terminal_drops_columns_then_cuts() {
    let mut e = Engine::new();
    let mut h = host();
    let out = run_w(&mut e, &mut h, "ls | where size > 1mb", 40);
    assert_eq!(out.text, "\
╭───┬─────────────┬──────┬─────────┬───╮
│ # │    name     │ type │  size   │ … │
├───┼─────────────┼──────┼─────────┼───┤
│ 0 │ archive.tar │ file │ 2.4 MiB │ … │
│ 1 │ disk.img    │ file │   3 GiB │ … │
│ 2 │ video.mkv   │ file │ 700 MiB │ … │
╰───┴─────────────┴──────┴─────────┴───╯
");
    for line in out.text.lines() { assert!(spindle_shell::width::str_width(line) <= 40, "{}", line); }
    let out = run_w(&mut e, &mut h, "ls | where size > 1mb | select name", 12);
    assert_eq!(out.text, "\
╭───┬──────╮
│ # │ name │
├───┼──────┤
│ 0 │ arc… │
│ 1 │ dis… │
│ 2 │ vid… │
╰───┴──────╯
");
}

#[test]
fn cjk_cells_are_two_wide() {
    let t = Value::List(vec![
        Value::Record(Record::new().with("name", Value::string("漢字.txt")).with("n", Value::Int(1))),
        Value::Record(Record::new().with("name", Value::string("abc")).with("n", Value::Int(22))),
    ]);
    assert_eq!(render::render(&t, 0, false), "\
╭───┬──────────┬────╮
│ # │   name   │ n  │
├───┼──────────┼────┤
│ 0 │ 漢字.txt │  1 │
│ 1 │ abc      │ 22 │
╰───┴──────────┴────╯
");
    // Cut on a cell boundary: a wide character never straddles the edge.
    assert_eq!(spindle_shell::width::truncate("漢字漢字", 6), "漢字…");
    assert_eq!(spindle_shell::width::truncate("漢字漢字", 5), "漢字…");
}

#[test]
fn records_lists_and_scalars() {
    assert_eq!(ok("{name: spindle, size: 2kb}"), "\
╭──────┬─────────╮
│ name │ spindle │
│ size │   2 KiB │
╰──────┴─────────╯
");
    assert_eq!(ok("[a b]"), "╭───┬───╮\n│ 0 │ a │\n│ 1 │ b │\n╰───┴───╯\n");
    assert_eq!(ok("[]"), "╭────────────╮\n│ empty list │\n╰────────────╯\n");
    assert_eq!(ok("42"), "42\n");
    assert_eq!(ok("1.0"), "1.0\n");
    assert_eq!(ok("'text'"), "text\n");
    assert_eq!(ok("null"), "");
    assert_eq!(ok("open data.json | select tags items"), "\
╭───────┬────────────────╮
│ tags  │ [list 2 items] │
│ items │ [table 2 rows] │
╰───────┴────────────────╯
");
}

#[test]
fn table_command_and_plain_text() {
    assert_eq!(val("[1 2] | table"), Value::string("╭───┬───╮\n│ 0 │ 1 │\n│ 1 │ 2 │\n╰───┴───╯\n"));
    assert_eq!(ok("open people.csv | to text"), "name   age  city\nAda     36  London\nLinus   28  Helsinki, FI\nGrace   45  Arlington\n");
}

#[test]
fn binary_shows_as_hex() {
    let out = render::render(&Value::Binary(vec![0, 1, b'A']), 80, false);
    assert_eq!(out, "binary, 3 bytes\n00000000  00 01 41                                         ..A\n");
}
