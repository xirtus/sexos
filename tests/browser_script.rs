// SPDX-License-Identifier: GPL-3.0-only
//! Host tests of the Browser's bounded inline script subset.
extern crate alloc;
#[path="../apps/kaleidoscope/src/script.rs"] mod script;
use script::{run, Error};

fn out(src: &str) -> Result<String, Error> { run(src.as_bytes()).map(|v| String::from_utf8(v).unwrap()) }

#[test]
fn writes_strings_numbers_and_variables() {
    assert_eq!(out(r#"document.write("2+2 = " + (2+2));"#).unwrap(), "2+2 = 4");
    assert_eq!(out("var a = 6; let b = 7\nconst who = 'SexOS'\ndocument.write(who, ': ', a*b)").unwrap(), "SexOS: 42");
    assert_eq!(out("x = 1;").err(), Some(Error::Unsupported), "undeclared assignment");
    assert_eq!(out("var n = 1; n = n + 1; document.writeln(n); document.write('done')").unwrap(), "2\ndone");
    assert_eq!(out(r#"document.write("tab\there \"q\" \\ end")"#).unwrap(), "tab\there \"q\" \\ end");
    assert_eq!(out("// comment\n/* block */ document.write(1)").unwrap(), "1");
}

#[test]
fn numbers_print_like_javascript() {
    let cases = [("0.1+0.2", "0.30000000000000004"), ("1/3", "0.3333333333333333"), ("10/4", "2.5"), ("1/0", "Infinity"),
        ("-1/0", "-Infinity"), ("0/0", "NaN"), ("7%3", "1"), ("-7%3", "-1"), ("5.5%2", "1.5"), ("1e21", "1e+21"),
        ("1e-7", "1e-7"), ("123456789*1000", "123456789000"), ("-0", "0"), ("'3'*'4'", "12"), ("'3'+4", "34"),
        ("'a'-1", "NaN"), ("+''", "0"), ("2*(3+4)", "14"), (".5+.25", "0.75")];
    for (expr, want) in cases { assert_eq!(out(&format!("document.write({expr})")).unwrap(), want, "{expr}"); }
}

#[test]
fn anything_outside_the_subset_writes_nothing() {
    for src in ["while(true){}", "document.write(1); for(;;){}", "alert(1)", "fetch('/x')", "document.cookie = 'a'",
        "document.write(window.location)", "function f(){}", "document.write(1) document.write(2)", "var x = 'unterminated",
        "document.write(1); eval('2')", "document.body.innerHTML = 'x'", "x => 1", "document.write(`t`)"] {
        assert!(run(src.as_bytes()).is_err(), "{src}");
    }
}

#[test]
fn bounds_hold() {
    let deep = format!("document.write({}1{})", "(".repeat(40), ")".repeat(40));
    assert_eq!(out(&deep).err(), Some(Error::Limit));
    let negs = format!("document.write({}1)", "-".repeat(40));
    assert_eq!(out(&negs).err(), Some(Error::Limit));
    let big = format!("var s = '{}'; s = s + s; s = s + s;", "a".repeat(600));
    assert_eq!(out(&big).err(), Some(Error::Limit));
    let mut many = String::new();
    for i in 0..17 { many += &format!("var v{i} = {i};"); }
    assert_eq!(out(&many).err(), Some(Error::Limit));
    let long = format!("document.write('{}')", "x".repeat(1100));
    assert_eq!(out(&long).err(), Some(Error::Limit));
    assert_eq!(run(&vec![b' '; script::MAX_SOURCE + 1]).err(), Some(Error::Limit));
}
