// SPDX-License-Identifier: GPL-3.0-only
mod common;
use common::*;
use spindle_shell::ast::*;
use spindle_shell::parser::parse;
use spindle_shell::{Span, Value};

fn pipeline(src: &str) -> Pipeline {
    let b = parse(src).unwrap_or_else(|e| panic!("{}", e.render(src, false)));
    assert_eq!(b.stmts.len(), 1);
    match b.stmts.into_iter().next().unwrap() { Statement::Pipeline(p) => p, s => panic!("{:?}", s) }
}

fn call(e: &Element) -> &Call { match e { Element::Call(c) => c, other => panic!("{:?}", other) } }

#[test]
fn pipelines_and_statements() {
    let p = pipeline("ls | where size > 1mb | sort-by name");
    assert_eq!(p.elements.len(), 3);
    let names: Vec<&str> = p.elements.iter().map(|e| call(e).name.as_str()).collect();
    assert_eq!(names, ["ls", "where", "sort-by"]);
    let b = parse("ls; pwd\necho a").unwrap();
    assert_eq!(b.stmts.len(), 3);
    assert_eq!(parse("ls # a comment").unwrap().stmts.len(), 1);
}

#[test]
fn condition_precedence() {
    let p = pipeline("where size > 1mb and name =~ rs or not type == dir");
    let Arg::Positional(e) = &call(&p.elements[0]).args[0] else { panic!() };
    let ExprKind::Binary { op: Op::Or, lhs, rhs, .. } = &e.kind else { panic!("{:?}", e) };
    assert!(matches!(lhs.kind, ExprKind::Binary { op: Op::And, .. }));
    assert!(matches!(rhs.kind, ExprKind::Not(_)));
    let ExprKind::Binary { lhs: gt, .. } = &lhs.kind else { panic!() };
    let ExprKind::Binary { op: Op::Gt, rhs: size, .. } = &gt.kind else { panic!() };
    assert_eq!(size.kind, ExprKind::Literal(Value::Filesize(1_000_000)));
    // Comparisons without spaces.
    let p = pipeline("where size>=10KiB");
    let Arg::Positional(e) = &call(&p.elements[0]).args[0] else { panic!() };
    assert!(matches!(&e.kind, ExprKind::Binary { op: Op::Ge, rhs, .. } if rhs.kind == ExprKind::Literal(Value::Filesize(10 * 1024))));
}

#[test]
fn literals_with_units() {
    use spindle_shell::units::parse_literal as l;
    assert_eq!(l("1mb"), Some(Value::Filesize(1_000_000)));
    assert_eq!(l("10KiB"), Some(Value::Filesize(10_240)));
    assert_eq!(l("2gb"), Some(Value::Filesize(2_000_000_000)));
    assert_eq!(l("1.5MiB"), Some(Value::Filesize(1_572_864)));
    assert_eq!(l("5sec"), Some(Value::Duration(5_000_000_000)));
    assert_eq!(l("2min"), Some(Value::Duration(120_000_000_000)));
    assert_eq!(l("100ms"), Some(Value::Duration(100_000_000)));
    assert_eq!(l("-3"), Some(Value::Int(-3)));
    assert_eq!(l("0x1f"), Some(Value::Int(31)));
    assert_eq!(l("1_000"), Some(Value::Int(1000)));
    assert_eq!(l("2.5e3"), Some(Value::Float(2500.0)));
    assert_eq!(l("2026-11-14"), Some(Value::Date(1_794_614_400)));
    assert_eq!(l("2026-11-14T10:30:00Z"), Some(Value::Date(1_794_614_400 + 37_800)));
    assert_eq!(l("true"), Some(Value::Bool(true)));
    assert_eq!(l("5apples"), None);
    assert_eq!(l("-r"), None);
}

#[test]
fn strings_and_escapes() {
    assert_eq!(val(r#""a\tb\n\u{1F600}\"""#), Value::string("a\tb\n😀\""));
    assert_eq!(val(r"'raw\n'"), Value::string("raw\\n"));
    assert_eq!(val("echo `two words`"), Value::string("two words"));
}

#[test]
fn vars_cell_paths_lists_records() {
    let p = pipeline("$x.name.0");
    let Element::Expr(e) = &p.elements[0] else { panic!() };
    let ExprKind::Var { name, path } = &e.kind else { panic!() };
    assert_eq!(name, "x");
    assert_eq!(path, &vec![
        spindle_shell::PathMember::Field("name".into(), Span::new(3, 7)),
        spindle_shell::PathMember::Index(0, Span::new(8, 9)),
    ]);
    assert_eq!(val("[1 2, 3]"), Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]));
    assert_eq!(val("{a: 1, b:x}"), Value::Record(spindle_shell::Record::new().with("a", Value::Int(1)).with("b", Value::string("x"))));
    assert_eq!(val("let r = {a: {b: [10 20]}}; $r.a.b.1"), Value::Int(20));
}

#[test]
fn externals_background_and_redirects() {
    let p = pipeline("^ls -la src");
    let c = call(&p.elements[0]);
    assert!(c.forced_external && c.builtin.is_none());
    assert_eq!(c.args.len(), 2);
    let p = pipeline("make all &");
    assert!(p.background);
    assert!(call(&p.elements[0]).builtin.is_none());
    let p = pipeline("ls | to json > out.json");
    let r = p.redirect.unwrap();
    assert!(!r.append);
    assert_eq!(r.path.kind, ExprKind::BareWord("out.json".into()));
    assert!(pipeline("echo hi >> log.txt").redirect.unwrap().append);
    // In a condition `>` compares.
    assert!(pipeline("ls | where size > 1mb").redirect.is_none());
    // Two-word commands.
    assert_eq!(call(&pipeline("math sum").elements[0]).name, "math sum");
    // Glued words stay one argument for programs.
    let p = pipeline("env A=b");
    let Arg::Positional(e) = &call(&p.elements[0]).args[0] else { panic!() };
    assert_eq!(e.kind, ExprKind::BareWord("A=b".into()));
}

#[test]
fn let_statements() {
    let b = parse("let big = ls | where size > 1mb").unwrap();
    let Statement::Let { name, pipeline, .. } = &b.stmts[0] else { panic!() };
    assert_eq!(name, "big");
    assert_eq!(pipeline.elements.len(), 2);
}

fn err_at(src: &str) -> String { parse(src).unwrap_err().render(src, false) }

#[test]
fn parse_errors_point_at_the_token() {
    assert_eq!(err_at("ls | | sort-by name"), "error: expected a command after `|`\n  ls | | sort-by name\n     ^ nothing after this pipe\n");
    assert_eq!(err_at("ls |"), "error: expected a command after `|`\n  ls |\n     ^ nothing after this pipe\n");
    assert_eq!(err_at("echo [1 2"), "error: this `[` is not closed\n  echo [1 2\n       ^ opened here\n  help: add `]` at the end of the list\n");
    assert_eq!(err_at("echo \"abc"), "error: this string is not closed\n  echo \"abc\n       ^^^^\n");
    assert_eq!(err_at("echo \"a\\qb\""), "error: unknown escape `\\q`: use \\n \\t \\r \\0 \\e \\\\ \\\" \\' or \\u{..}\n  echo \"a\\qb\"\n         ^^\n");
    assert_eq!(err_at("ls && pwd"), "error: `&&` is not Spindle syntax: use `;` to run one command after another, or `and` in a condition\n  ls && pwd\n     ^^\n");
    assert_eq!(err_at("sort-by -x"), "error: `sort-by` has no flag `-x`\n  sort-by -x\n           ^ unknown flag\n  help: its flags are --reverse (-r), --ignore-case (-i)\n");
    assert_eq!(err_at("ls | where size >"), "error: `>` needs a value on its right\n  ls | where size >\n                  ^ nothing after this\n");
    assert_eq!(err_at("ls | where size > 1mb name"), "error: unexpected `name` in the condition\n  ls | where size > 1mb name\n                        ^^^^ expected an operator such as `and`, or the end\n");
    assert_eq!(err_at("str"), "error: `str` needs a subcommand\n  str\n  ^^^ which one?\n  help: try str contains, str replace, str upcase, str downcase, str trim, str length\n");
    assert_eq!(err_at("math total"), "error: `math` has no subcommand `total`\n  math total\n       ^^^^^ unknown subcommand\n  help: try math sum, math avg, math min, math max\n");
    assert_eq!(err_at("let = 5"), "error: `let` needs a name\n  let = 5\n  ^^^ like `let x = ls`\n");
    assert_eq!(err_at("let x 5"), "error: expected `=` after `let x`\n  let x 5\n       ^ like `let x = 5`\n");
    assert_eq!(err_at("ls > out.txt | length"), "error: a redirect must end its pipeline\n  ls > out.txt | length\n     ^^^^^^^^^ nothing can follow this\n  help: to keep going, use `| save file | ...` or put the redirect last\n");
    assert_eq!(err_at("echo (ls"), "error: this `(` is not closed\n  echo (ls\n       ^ opened here\n  help: add `)` at the end\n");
    assert_eq!(err_at("ls )"), "error: unexpected `)`\n  ls )\n     ^ nothing to close here\n");
    assert_eq!(err_at("ls & pwd"), "error: unexpected `pwd`\n  ls & pwd\n       ^^^ not expected here\n");
}

#[test]
fn errors_on_later_lines_show_that_line() {
    let src = "ls\necho [1";
    assert_eq!(err_at(src), "error: this `[` is not closed\n  echo [1\n       ^ opened here\n  help: add `]` at the end of the list\n");
}

#[test]
fn caret_under_wide_characters() {
    let src = "echo 漢字 | lss";
    let e = spindle_shell::ShellError::at("x", Span::new(src.find("lss").unwrap(), src.len()), "here");
    assert_eq!(e.render(src, false), "error: x\n  echo 漢字 | lss\n              ^^^ here\n");
}
