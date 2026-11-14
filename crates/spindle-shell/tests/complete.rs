// SPDX-License-Identifier: GPL-3.0-only
mod common;
use common::*;
use spindle_shell::{CompletionKind, Engine, Style};

fn texts(line: &str) -> Vec<String> {
    let mut e = Engine::new();
    let mut h = host();
    run(&mut e, &mut h, "let answer = 42");
    e.complete(line, line.len(), &h).into_iter().map(|c| c.text).collect()
}

#[test]
fn commands_and_subcommands() {
    assert_eq!(texts("so"), ["sort-by"]);
    assert_eq!(texts("ls | wh"), ["where"]);
    assert_eq!(texts("gr"), ["greet", "group-by"]);
    assert_eq!(texts("math "), ["avg", "max", "min", "sum"]);
    assert_eq!(texts("str up"), ["upcase"]);
    let mut e = Engine::new();
    let h = host();
    let c = e.complete("ls | sor", 8, &h);
    assert_eq!(c[0].replace, 5..8);
    assert_eq!(c[0].kind, CompletionKind::Command);
    assert!(!c[0].description.is_empty());
    e.color = false;
}

#[test]
fn flags_variables_columns_paths() {
    assert_eq!(texts("ls | sort-by name --r"), ["--reverse"]);
    assert_eq!(texts("ls | sort-by -"), ["--ignore-case", "--reverse"]);
    assert_eq!(texts("echo $an"), ["$answer"]);
    assert_eq!(texts("ls | where si"), ["size"]);
    assert_eq!(texts("ls | where size > 1mb | sort-by "), ["modified", "name", "size", "type"]);
    assert_eq!(texts("open people.csv | select c"), ["city"]);
    assert_eq!(texts("open no"), ["notes.txt"]);
    assert_eq!(texts("open src/m"), ["src/main.rs"]);
    assert_eq!(texts("cd "), ["docs/", "src/"]);
    assert_eq!(texts("greet d"), ["data.json", "disk.img", "docs/"]);
    assert_eq!(texts("help whe"), ["where"]);
    // Columns are not guessed through programs or commands that change things.
    assert!(texts("greet | where x").is_empty());
}

#[test]
fn completion_mid_line() {
    let e = Engine::new();
    let h = host();
    let line = "ls | wh | length";
    let c = e.complete(line, 7, &h);
    assert_eq!(c.len(), 1);
    assert_eq!((c[0].text.as_str(), c[0].replace.clone()), ("where", 5..7));
}

#[test]
fn highlight_styles() {
    let e = Engine::new();
    let line = "let x = ls -a | where size > 1mb | ^rg \"a\" $x.name # hi";
    let got: Vec<(&str, Style)> = e.highlight(line).into_iter().map(|(r, s)| (&line[r], s)).collect();
    assert_eq!(got, vec![
        ("let", Style::Keyword), ("x", Style::Variable), ("=", Style::Operator), ("ls", Style::Command), ("-a", Style::Flag),
        ("|", Style::Punctuation), ("where", Style::Command), ("size", Style::Text), (">", Style::Operator), ("1mb", Style::Number),
        ("|", Style::Punctuation), ("^rg", Style::External), ("\"a\"", Style::String), ("$x.name", Style::Variable), ("# hi", Style::Comment),
    ]);
    let got: Vec<(&str, Style)> = e.highlight("math sum; foo 'x").into_iter().map(|(r, s)| (&"math sum; foo 'x"[r], s)).collect();
    assert_eq!(got, vec![("math", Style::Command), ("sum", Style::Command), (";", Style::Punctuation), ("foo", Style::External), ("'x", Style::Error)]);
    assert_eq!(Style::Command.ansi(), "\x1b[1;36m");
}
