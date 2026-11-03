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
        "document.write(window.location)", "if (1) { while(1){} }", "var a = 1; a.foo", "if (0) x = 1", "document.write(1) document.write(2)", "var x = 'unterminated",
        "document.write(1); eval('2')", "document.body.innerHTML = 'x'", "x => 1"] {
        assert!(run(src.as_bytes()).is_err(), "{src}");
    }
}

#[test]
fn bounds_hold() {
    let deep = format!("document.write({}1{})", "(".repeat(40), ")".repeat(40));
    assert_eq!(out(&deep).err(), Some(Error::Limit));
    let negs = format!("document.write({}1)", "- ".repeat(40));
    assert_eq!(out(&negs).err(), Some(Error::Limit));
    let big = format!("var s = '{}'; s = s + s; s = s + s; s = s + s; s = s + s;", "a".repeat(600));
    assert_eq!(out(&big).err(), Some(Error::Limit));
    let mut many = String::new();
    for i in 0..17 { many += &format!("var v{i} = {i};"); }
    assert_eq!(out(&many).err(), Some(Error::Limit));
    let long = format!("document.write('{}')", "x".repeat(1100));
    assert_eq!(out(&long).err(), Some(Error::Limit));
    assert_eq!(run(&vec![b' '; script::MAX_SOURCE + 1]).err(), Some(Error::Limit));
}

#[test]
fn branches_comparisons_and_helpers() {
    let src = "var n = 7\nif (n % 2 === 0) { document.write('even') } else if (n > 5 && n < 10) { document.write('odd, mid') } else document.write('odd')";
    assert_eq!(out(src).unwrap(), "odd, mid");
    assert_eq!(out("var s = 'Sex' + 'OS'; document.write(s.length, ' ', s.toUpperCase(), ' ', s.toLowerCase().length)").unwrap(), "5 SEXOS 5");
    assert_eq!(out("document.write(1 < 2, ' ', 'b' > 'a', ' ', '2' == 2, ' ', '2' === 2, ' ', !0, ' ', null_safe = 1)").err(), Some(Error::Unsupported));
    assert_eq!(out("document.write(1 < 2, ' ', 'b' > 'a', ' ', '2' == 2, ' ', '2' === 2, ' ', !0)").unwrap(), "true true true false true");
    assert_eq!(out("document.write(0 || 'x', ' ', 1 && 'y', ' ', 3 > 2 ? 'big' : 'small', ' ', NaN < 1)").unwrap(), "x y big false");
    assert_eq!(out("document.write(Math.floor(-2.5), Math.ceil(2.1), Math.round(2.5), Math.abs(-3), Math.max(1, 9, 4), Math.min(), Math.max(1, NaN))").unwrap(), "-33339InfinityNaN");
    // Untaken branches still must be valid subset code, and write nothing.
    assert_eq!(out("var a = 1; if (false) { a = 2; document.write('no') } document.write(a)").unwrap(), "1");
    assert_eq!(out("if (true) document.write('a'); else document.write('b')").unwrap(), "a");
}

#[test]
fn loops_run_within_the_step_budget() {
    assert_eq!(out("for (var i = 1; i <= 5; i++) document.write(i, i < 5 ? ',' : '')").unwrap(), "1,2,3,4,5");
    assert_eq!(out("var n = 10, s = 0").err(), Some(Error::Syntax));
    assert_eq!(out("var n = 10; var s = 0; while (n > 0) { s += n; n-- } document.write(s)").unwrap(), "55");
    assert_eq!(out("var t = ''; for (let i = 0; i < 3; i += 1) { for (let j = 0; j < 2; j++) t = t + i + j + ' ' } document.write(t)").unwrap(), "00 01 10 11 20 21 ");
    assert_eq!(out("var k = 0; for (;;) { k++ } document.write(k)").err(), Some(Error::Limit), "endless loop spends the budget");
    assert_eq!(out("while (true) {}").err(), Some(Error::Limit));
    assert_eq!(out("var x = 0; while (false) { y = 1 } document.write(x)").err(), Some(Error::Unsupported), "body checked even when never run");
    assert_eq!(out("var x = 3; while (false) x = 9; document.write(x)").unwrap(), "3");
    assert!(out("var i = 0; while (i < 9000) i++; document.write(i)").is_ok());
    assert_eq!(out("var i = 0; while (i < 20000) i++").err(), Some(Error::Limit));
}

#[test]
fn functions_with_parameters_and_return() {
    assert_eq!(out("function sq(x) { return x * x }\ndocument.write(sq(7), ' ', sq('3'))").unwrap(), "49 9");
    assert_eq!(out("var x = 'global'; function f(x) { return x + '!' } document.write(f('local'), ' ', x)").unwrap(), "local! global");
    assert_eq!(out("function sign(n) { if (n < 0) return -1; if (n > 0) return 1; return 0 } document.write(sign(-5), sign(0), sign(9))").unwrap(), "-101");
    assert_eq!(out("function sum(a, b, c) { var t = a + b; return t + (c === undefined ? 0 : c) } document.write(sum(1, 2))").err(), Some(Error::Unsupported), "no undefined name");
    assert_eq!(out("function noop() {} document.write(noop())").unwrap(), "undefined");
    assert_eq!(out("function fact(n) { if (n <= 1) return 1; return n * fact(n - 1) } document.write(fact(2))").unwrap(), "2");
    assert_eq!(out("function fact(n) { if (n <= 1) return 1; return n * fact(n - 1) } document.write(fact(30))").err(), Some(Error::Limit), "recursion bounded by depth");
    assert_eq!(out("document.write(later(1)); function later(x) { return x }").err(), Some(Error::Unsupported), "declare before use");
    assert_eq!(out("function f() { while (true) {} } f()").err(), Some(Error::Limit));
    assert_eq!(out("function f(a) { document.write(a) } f(1); f(2)").unwrap(), "12");
    assert_eq!(out("function f(a) { return a; document.write('never') } document.write(f(5))").unwrap(), "5");
}

#[test]
fn arrays_by_reference() {
    assert_eq!(out("var a = [3, 1, 2]; document.write(a, ' ', a.length, ' ', a[0] + a[2], ' ', a[9])").unwrap(), "3,1,2 3 5 undefined");
    assert_eq!(out("var a = []; for (var i = 0; i < 10; i++) a.push(i * i); document.write(a.join(' '))").unwrap(), "0 1 4 9 16 25 36 49 64 81");
    assert_eq!(out("var a = [1]; var b = a; b.push(2); a[0] = 'x'; document.write(a, '|', b)").unwrap(), "x,2|x,2");
    assert_eq!(out("var w = ['Sex', 'OS']; document.write(w.join(''), ' ', 'abc'[1], [[1, 2], 3])").unwrap(), "SexOS b1,2,3");
    assert_eq!(out("var a = [1]; a[5] = 2").err(), Some(Error::Unsupported), "no holes");
    assert_eq!(out("var a = []; while (true) a.push(1)").err(), Some(Error::Limit));
    assert_eq!(out("var s = 0; var a = [5, 6, 7]; for (var i = 0; i < a.length; i++) s += a[i]; document.write(s)").unwrap(), "18");
}

#[test]
fn string_methods_and_conversions() {
    assert_eq!(out("var s = '  SexOS rocks  '.trim(); document.write(s.indexOf('OS'), ' ', s.indexOf('zz'), ' ', s.charAt(1), ' ', s.slice(-5), ' ', s.substring(5, 0))").unwrap(), "3 -1 e rocks SexOS");
    assert_eq!(out("var p = 'a,b,,c'.split(','); document.write(p.length, ' ', p.join('|'), ' ', 'abc'.split('').join('-'), ' ', 'x'.split().length)").unwrap(), "4 a|b||c a-b-c 1");
    assert_eq!(out("document.write(String(12) + 3, ' ', Number('4') + 3, ' ', parseInt(' 42px'), ' ', parseInt('-7'), ' ', parseInt('px'), ' ', Number('x'))").unwrap(), "123 7 42 -7 NaN NaN");
    assert_eq!(out("document.write('abc'.slice(1, -1), 'abc'.slice(5), 'abc'.substring(2, 1))").unwrap(), "bb");
}

#[test]
fn more_math() {
    assert_eq!(out("document.write(Math.sqrt(144), ' ', Math.sqrt(2), ' ', Math.sqrt(-1), ' ', Math.pow(2, 10), ' ', Math.pow(2, -2), ' ', Math.pow(2, 0.5))").unwrap(), "12 1.4142135623730951 NaN 1024 0.25 NaN");
    assert_eq!(out("document.write(Math.trunc(-2.7), Math.sign(-3), Math.sign(0), ' ', Math.PI)").unwrap(), "-2-10 3.141592653589793");
    assert_eq!(out("Math.random()").err(), Some(Error::Unsupported));
}

#[test]
fn break_continue_and_templates() {
    assert_eq!(out("for (var i = 0; i < 10; i++) { if (i % 2 === 1) continue; if (i > 6) break; document.write(i) }").unwrap(), "0246");
    assert_eq!(out("var n = 0; while (true) { n++; if (n === 5) break } document.write(n)").unwrap(), "5");
    assert_eq!(out("for (var i = 0; i < 3; i++) { for (var j = 0; j < 3; j++) { if (j === 1) break; document.write(i, j, ' ') } }").unwrap(), "00 10 20 ");
    assert_eq!(out("break").err(), Some(Error::Syntax));
    assert_eq!(out("function f() { break } for (;;) { f() }").err(), Some(Error::Syntax), "break does not cross a call");
    let name = "var who = 'SexOS'; var n = 3; document.write(`Hello ${who}, ${n} x ${n} = ${n * n}!\\n`)";
    assert_eq!(out(name).unwrap(), "Hello SexOS, 3 x 3 = 9!\n");
    assert_eq!(out("document.write(`${'a' + `b`}`)").err().is_some(), true, "no nested templates");
    assert_eq!(out("document.write(`${1 + }`)").err(), Some(Error::Syntax));
}
