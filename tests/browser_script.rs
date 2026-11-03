// SPDX-License-Identifier: GPL-3.0-only
//! Host tests of the Browser's bounded inline script subset.
extern crate alloc;
#[path="../apps/kaleidoscope/src/regex.rs"] mod regex;
#[path="../apps/kaleidoscope/src/fmath.rs"] mod fmath;
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
        "document.write(window.location)", "if (1) { while(1){} }", "var a = 1; a.foo", "document.write(1) document.write(2)", "var x = 'unterminated",
        "document.write(1); eval('2')", "document.body.innerHTML = 'x'", "x => 1"] {
        assert!(run(src.as_bytes()).is_err(), "{src}");
    }
}

#[test]
fn bounds_hold() {
    let deep = format!("document.write({}1{})", "(".repeat(80), ")".repeat(80));
    assert_eq!(out(&deep).err(), Some(Error::Limit));
    let negs = format!("document.write({}1)", "- ".repeat(80));
    assert_eq!(out(&negs).err(), Some(Error::Limit));
    let big = format!("var s = '{}'; s = s + s; s = s + s; s = s + s; s = s + s;", "a".repeat(2400));
    assert_eq!(out(&big).err(), Some(Error::Limit));
    let mut many = String::new();
    for i in 0..49 { many += &format!("var v{i} = {i};"); }
    assert_eq!(out(&many).err(), Some(Error::Limit));
    let long = format!("document.write('{}')", "x".repeat(4100));
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
    assert_eq!(out("var n = 10, s = 0; document.write(n + s)").unwrap(), "10");
    assert_eq!(out("var n = 10; var s = 0; while (n > 0) { s += n; n-- } document.write(s)").unwrap(), "55");
    assert_eq!(out("var t = ''; for (let i = 0; i < 3; i += 1) { for (let j = 0; j < 2; j++) t = t + i + j + ' ' } document.write(t)").unwrap(), "00 01 10 11 20 21 ");
    assert_eq!(out("var k = 0; for (;;) { k++ } document.write(k)").err(), Some(Error::Limit), "endless loop spends the budget");
    assert_eq!(out("while (true) {}").err(), Some(Error::Limit));
    assert_eq!(out("var x = 0; while (false) { y = 1 } document.write(x)").unwrap(), "0", "a body that never runs is only parsed, as in JavaScript");
    assert_eq!(out("var x = 0; while (false) { y = = 1 }").err(), Some(Error::Syntax), "but it must parse");
    assert_eq!(out("var x = 3; while (false) x = 9; document.write(x)").unwrap(), "3");
    assert!(out("var i = 0; while (i < 9000) i++; document.write(i)").is_ok());
    assert_eq!(out("var i = 0; while (i < 60000) i++").err(), Some(Error::Limit));
}

#[test]
fn functions_with_parameters_and_return() {
    assert_eq!(out("function sq(x) { return x * x }\ndocument.write(sq(7), ' ', sq('3'))").unwrap(), "49 9");
    assert_eq!(out("var x = 'global'; function f(x) { return x + '!' } document.write(f('local'), ' ', x)").unwrap(), "local! global");
    assert_eq!(out("function sign(n) { if (n < 0) return -1; if (n > 0) return 1; return 0 } document.write(sign(-5), sign(0), sign(9))").unwrap(), "-101");
    assert_eq!(out("function sum(a, b, c) { var t = a + b; return t + (c === undefined ? 0 : c) } document.write(sum(1, 2), sum(1, 2, 3))").unwrap(), "36", "missing argument is undefined");
    assert_eq!(out("function noop() {} document.write(noop())").unwrap(), "undefined");
    assert_eq!(out("function fact(n) { if (n <= 1) return 1; return n * fact(n - 1) } document.write(fact(7), fact(15))").unwrap(), "50401307674368000");
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
    assert_eq!(out("document.write(`${'a' + `b`}`)").unwrap(), "ab", "nested templates");
    assert_eq!(out("document.write(`${1 + }`)").err(), Some(Error::Syntax));
}

#[test]
fn objects_keys_and_json() {
    assert_eq!(out("var o = {name: 'Quil', 'ver': 3}; o.ver = o.ver + 1; o['kind'] = 'editor'; document.write(o.name, ' ', o.ver, ' ', o.kind, ' ', o.none, ' ', o)").unwrap(), "Quil 4 editor undefined [object Object]");
    assert_eq!(out("var o = {a: 1, b: [1, 'x', true], c: {d: \"q\\\"\"}, e: undefined_name}").err(), Some(Error::Unsupported));
    assert_eq!(out("var o = {a: 1, b: [1, 'x', true], c: {d: 'q\"'}}; document.write(JSON.stringify(o), ' ', Object.keys(o).join('|'), ' ', JSON.stringify([1/0, 'a\\nb']))").unwrap(),
        "{\"a\":1,\"b\":[1,\"x\",true],\"c\":{\"d\":\"q\\\"\"}} a|b|c [null,\"a\\nb\"]");
    assert_eq!(out("var p = {}; var q = p; q.x = 5; document.write(p.x)").unwrap(), "5", "objects by reference");
    assert_eq!(out("var o = {}; o.self = o; JSON.stringify(o)").err(), Some(Error::Limit));
    assert_eq!(out("var n = 1; n.x = 2").err(), Some(Error::Unsupported));
    assert_eq!(out("var c = {n: 1}; c.n++; c.n += 10; c.n--; c.m = 'a'; c.m += 'b'; document.write(c.n, c.m)").unwrap(), "11ab");
}

#[test]
fn null_undefined_typeof_and_to_fixed() {
    assert_eq!(out("var a; document.write(a)").unwrap(), "undefined");
    assert_eq!(out("var n = null; document.write(n, ' ', n == undefined, ' ', n === undefined, ' ', n === null, ' ', n + 1, ' ', !n)").unwrap(), "null true false true 1 true");
    assert_eq!(out("document.write(typeof 1, typeof 'x', typeof true, typeof undefined, typeof nothing, typeof null, typeof [1], typeof {a: 1})").unwrap(), "numberstringbooleanundefinedundefinedobjectobjectobject");
    assert_eq!(out("document.write((3.14159).toFixed(2), ' ', (2.5).toFixed(0), ' ', (-1.005).toFixed(2), ' ', (1e-7).toFixed(3), ' ', (12).toFixed(1), ' ', (0.000001).toFixed(7))").unwrap(), "3.14 3 -1.00 0.000 12.0 0.0000010");
    assert_eq!(out("var x = 1.23456; document.write(x.toFixed(4))").unwrap(), "1.2346");
    assert_eq!(out("document.write(JSON.stringify({a: null, b: [null, 1]}))").unwrap(), "{\"a\":null,\"b\":[null,1]}");
}

#[test]
fn compound_assignment_do_while_and_switch() {
    assert_eq!(out("var x = 10; x *= 3; x /= 4; x %= 5; x -= 0.5; document.write(x)").unwrap(), "2");
    assert_eq!(out("var i = 0; do { i++ } while (i < 3); document.write(i); do { i = 100 } while (false) document.write(' ', i)").unwrap(), "3 100");
    assert_eq!(out("var n = 0; do { n++; if (n === 2) continue; if (n > 4) break } while (true); document.write(n)").unwrap(), "5");
    let sw = |v: &str| out(&format!("var r = ''; switch ({v}) {{ case 1: r += 'one'; case 2: r += 'two'; break; case 'x': r = 'ex'; break; default: r = 'other' }} document.write(r)")).unwrap();
    assert_eq!(sw("1"), "onetwo");
    assert_eq!(sw("2"), "two");
    assert_eq!(sw("'x'"), "ex");
    assert_eq!(sw("'1'"), "other", "strict comparison");
    assert_eq!(out("for (var i = 0; i < 4; i++) { switch (i) { case 1: continue; case 3: break } document.write(i) }").unwrap(), "023");
}

#[test]
fn array_methods() {
    assert_eq!(out("var a = [3, 'b', 10, 1]; document.write(a.indexOf(10), a.indexOf('10'), a.includes('b'), ' ', a.slice(1, -1), ' ', a.concat([7], 8).length, ' ', a.sort(), ' ', a.reverse())").unwrap(), "2-1true b,10 6 1,10,3,b b,3,10,1");
    assert_eq!(out("var q = [1, 2, 3]; var f = q.shift(); var l = q.pop(); document.write(f, l, ' ', q, ' ', q.length, ' ', [].pop())").unwrap(), "13 2 1 undefined");
    assert_eq!(out("[1].sort(cmp)").is_err(), true);
}

#[test]
fn more_string_methods() {
    assert_eq!(out("var s = 'a-b-c'; document.write(s.replace('-', '+'), ' ', s.replaceAll('-', ''), ' ', s.startsWith('a-'), s.endsWith('c'), s.includes('b-'), ' ', s.lastIndexOf('-'))").unwrap(), "a+b-c abc truetruetrue 3");
    assert_eq!(out("document.write('ab'.repeat(3), ' ', '7'.padStart(3, '0'), ' ', 'x'.padEnd(4, '.'), '|', 'long'.padStart(2))").unwrap(), "ababab 007 x...|long");
    assert_eq!(out("'a'.repeat(-1)").err(), Some(Error::Unsupported));
}

#[test]
fn function_values_arrows_and_callbacks() {
    assert_eq!(out("var nums = [5, 1, 4, 2, 3]; document.write(nums.map(x => x * 10).join(' '), ' | ', nums.filter(function (n) { return n % 2 === 1 }), ' | ', nums.reduce((a, b) => a + b, 0), ' ', nums.reduce((a, b) => a > b ? a : b))").unwrap(), "50 10 40 20 30 | 5,1,3 | 15 5");
    assert_eq!(out("var nums = [5, 1, 4]; nums.sort((a, b) => a - b); var seen = ''; nums.forEach((n, i) => { seen += i + ':' + n + ' ' }); document.write(seen, nums.find(n => n > 1), nums.findIndex(n => n > 9), nums.some(n => n === 4), nums.every(n => n > 0))").unwrap(), "0:1 1:4 2:5 4-1truetrue");
    assert_eq!(out("function twice(f, x) { return f(f(x)) } var inc = x => x + 1; document.write(twice(inc, 5), ' ', typeof inc, ' ', twice(function (s) { return s + '!' }, 'hi'))").unwrap(), "7 function hi!!");
    assert_eq!(out("var add = (a, b) => { var s = a + b; return s * 2 }; document.write(add(2, 3), ' ', [1, 2, 3].map(n => [n, n * n]).join(';'))").unwrap(), "10 1,1;2,4;3,9");
    assert_eq!(out("[1, 2].map(n => undefined_thing)").err(), Some(Error::Unsupported));
    assert_eq!(out("var a = [1]; a.forEach(n => a.push(n)); document.write(a.length)").unwrap(), "2", "visits the elements present at the start, like JavaScript");
    assert_eq!(out("[3].sort(5)").err(), Some(Error::Unsupported));
}

#[test]
fn number_helpers_object_helpers_in_and_delete() {
    assert_eq!(out("document.write(Number.isInteger(5), Number.isInteger(5.5), Number.isFinite(1 / 0), Number.isNaN('x'), isNaN('x'), ' ', parseFloat('3.25kg'), ' ', parseFloat('.5'), ' ', parseFloat('x'), ' ', (255).toString(16), ' ', (-10).toString(2), ' ', (1.5).toString())").unwrap(), "truefalsefalsefalsetrue 3.25 0.5 NaN ff -1010 1.5");
    assert_eq!(out("var o = {a: 1, b: 2}; delete o.a; document.write('a' in o, 'b' in o, ' ', Object.values(o), ' ', JSON.stringify(Object.entries({x: 1, y: 'z'})), ' ', Array.isArray([]), Array.isArray(o), ' ', 1 in [5, 6], 2 in [5, 6])").unwrap(), "falsetrue 2 [[\"x\",1],[\"y\",\"z\"]] truefalse truefalse");
    assert_eq!(out("document.write((1.5).toString(2))").err(), Some(Error::Unsupported));
}

#[test]
fn spread_and_destructuring() {
    assert_eq!(out("var a = [3, 9, 1]; var b = [0, ...a, 10]; function sum3(x, y, z) { return x + y + z } document.write(b, ' ', Math.max(...a), ' ', sum3(...a), ' ', [...'ab'.split('')].length)").unwrap(), "0,3,9,1,10 9 13 2");
    assert_eq!(out("var [first, second, third] = [1, 2]; var {name, age} = {name: 'Ann', x: 1}; document.write(first, second, third, ' ', name, ' ', age)").unwrap(), "12undefined Ann undefined");
    assert_eq!(out("var [x] = 5").err(), Some(Error::Unsupported));
    assert_eq!(out("[...5]").err(), Some(Error::Unsupported));
}

#[test]
fn char_codes() {
    assert_eq!(out("document.write('A'.charCodeAt(0), ' ', 'abc'.charCodeAt(9), ' ', String.fromCharCode(72, 105, 33), ' ', 'abc'.split('').map(c => String.fromCharCode(c.charCodeAt(0) - 32)).join(''), ' ', String.fromCharCode(0xe9))").unwrap(), "65 NaN Hi! ABC \u{e9}");
}

#[test]
fn block_scope_const_escapes_methods_helpers_and_exceptions() {
    assert_eq!(out("let x = 1; { let x = 2; document.write(x) } document.write(x); for (let i = 0; i < 2; i++) {} document.write(typeof i)").unwrap(), "21undefined");
    assert_eq!(out("const k = 1; k = 2").err(), Some(Error::Unsupported));
    assert_eq!(out("let a = 1; let a = 2").err(), Some(Error::Syntax));
    assert_eq!(out("document.write('\\u0041\\x42\\u{1F600}'.length, ' ', '\\u00e9')").unwrap(), "6 \u{e9}");
    assert_eq!(out("var counter = {n: 0, inc: function () { this.n = this.n + 1; return this }, get: function () { return this.n }}; counter.inc().inc(); document.write(counter.get())").unwrap(), "2");
    assert_eq!(out("document.write(Array.from('abc').join('-'), ' ', Array.from({length: 3}).length, ' ', [1, [2, 3], [4]].flat(), ' ', [1, 2, 3].at(-1), ' ', new_fill = 0)").err(), Some(Error::Unsupported));
    assert_eq!(out("document.write(Array.from('abc').join('-'), ' ', [0, 0, 0].fill(7, 1), ' ', [1, [2, 3], [4]].flat(), ' ', [1, 2, 3].at(-1), ' ', 'xyz'.at(-1), '|', '  t  '.trimStart(), '|', '  t  '.trimEnd(), '|')").unwrap(), "a-b-c 0,7,7 1,2,3,4 3 z|t  |  t|");
    assert_eq!(out("var log = ''; try { log += 'a'; throw 'boom'; log += 'never' } catch (e) { log += ' caught ' + e } finally { log += ' done' } document.write(log)").unwrap(), "a caught boom done");
    assert_eq!(out("function risky(n) { if (n > 1) throw {code: n}; return n } var r = []; for (var i = 0; i < 3; i++) { try { r.push(risky(i)) } catch (err) { r.push('E' + err.code) } } document.write(r)").unwrap(), "0,1,E2");
    assert_eq!(out("throw 'uncaught'").err(), Some(Error::Thrown));
    assert_eq!(out("try { while (true) {} } catch (e) {}").err(), Some(Error::Limit), "limits are not catchable");
}

#[test]
fn regular_expressions() {
    assert_eq!(out(r"var re = /(\d+)-(\d+)/; document.write(re.test('a 12-34 b'), re.test('none'), ' ', 'a 12-34 b'.match(re).join('|'), ' ', 'x1y22z333'.match(/\d+/g), ' ', 'abc'.search(/c/), ' ', re)").unwrap(), "truefalse 12-34|12|34 1,22,333 2 /(\\d+)-(\\d+)/");
    assert_eq!(out(r"document.write('2024-11-03'.replace(/(\d+)-(\d+)-(\d+)/, '$3.$2.$1'), ' ', 'a.b.c'.replace(/\./g, '/'), ' ', 'Hello'.replace(/l/gi, '[$&]'), ' ', 'one two'.replace(/\w+/g, w => w.toUpperCase()))").unwrap(), "03.11.2024 a/b/c He[l][l]o ONE TWO");
    assert_eq!(out(r"document.write('a, b,c ,  d'.split(/\s*,\s*/).join('|'), ' ', 'k=v'.split(/(=)/).length, ' ', 'no'.match(/x/))").unwrap(), "a|b|c|d 3 null");
    assert_eq!(out(r"var n = 10 / 2 / 5; document.write(n)").unwrap(), "1", "division still divides");
    assert_eq!(out(r"'a'.replaceAll(/a/, 'b')").err(), Some(Error::Unsupported));
    assert_eq!(out(r"document.write(/(/.test('x'))").err(), Some(Error::Syntax));
    assert_eq!(out(r"document.write(/(a*)*b/.test('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'))").err(), Some(Error::Limit));
}

#[test]
fn json_parse() {
    assert_eq!(out(r#"var o = JSON.parse('{"name": "Quil", "tags": ["editor", "draft"], "n": -1.5e2, "ok": true, "none": null, "u": "\\u00e9\\n"}'); document.write(o.name, ' ', o.tags[1], ' ', o.n, ' ', o.ok, ' ', o.none, ' ', o.u.length, ' ', JSON.stringify(JSON.parse('[1, {"a": [2]}]')))"#).unwrap(), "Quil draft -150 true null 3 [1,{\"a\":[2]}]");
    assert_eq!(out(r#"try { JSON.parse('{bad}') } catch (e) { document.write(e) }"#).unwrap(), "SyntaxError: bad JSON");
    assert_eq!(out(r#"JSON.parse('[1]x')"#).err(), Some(Error::Thrown));
}

#[test]
fn new_classes_and_dates() {
    assert_eq!(out("function Point(x, y) { this.x = x; this.y = y } var p = new Point(2, 3); document.write(p.x + p.y, ' ', new Array(3).length, ' ', new Array(1, 2), ' ', new Object() !== null)").unwrap(), "5 3 1,2 true");
    assert_eq!(out("class Counter { constructor(start) { this.n = start } inc() { this.n++; return this } get() { return this.n } } var c = new Counter(5); c.inc().inc(); document.write(c.get(), ' ', typeof c.inc)").unwrap(), "7 function");
    assert_eq!(out("try { throw new Error('nope') } catch (e) { document.write(e.name, ': ', e.message) }").unwrap(), "Error: nope");
    assert_eq!(out("var d = new Date(2012, 11, 21); document.write(d.getFullYear(), '-', d.getMonth() + 1, '-', d.getDate(), ' ', d.getDay(), ' ', d.toISOString(), ' ', d.toDateString())").unwrap(), "2012-12-21 5 2012-12-21T00:00:00.000Z Fri Dec 21 2012");
    assert_eq!(out("var d = new Date('2024-02-29T13:45:07.250Z'); document.write(d.getHours(), ':', d.getMinutes(), ':', d.getSeconds(), '.', d.getMilliseconds(), ' ', new Date(2024, 1, 30).getDate(), ' ', Date.UTC(1970, 0, 2), ' ', new Date(0).toISOString())").unwrap(), "13:45:7.250 1 86400000 1970-01-01T00:00:00.000Z");
    assert_eq!(out("new Date()").err().is_some(), true, "no clock");
    assert_eq!(out("document.write(Date.now())").err(), Some(Error::Unsupported));
    assert_eq!(out("class B extends A {}").err(), Some(Error::Unsupported), "unknown parent");
}

#[test]
fn class_inheritance_and_super() {
    assert_eq!(out("class Shape { constructor(name) { this.name = name } area() { return 0 } describe() { return this.name + ' ' + this.area() } }\nclass Sq extends Shape { constructor(s) { super('square'); this.s = s } area() { return this.s * this.s } describe() { return '[' + super.describe() + ']' } }\nclass Unit extends Sq {}\nvar a = new Sq(3); var u = new Unit(1); document.write(a.describe(), ' ', u.describe(), ' ', new Shape('dot').describe(), ' ', u.name)").unwrap(), "[square 9] [square 1] dot 0 square");
    assert_eq!(out("class A { f() { return super.f() } } var a = new A(); a.f()").err(), Some(Error::Syntax), "no parent");
    assert_eq!(out("class A {} class B extends A { g() { return super.nope() } } var b = new B(); b.g()").err(), Some(Error::Unsupported));
}

#[test]
fn short_circuit_nullish_and_optional_chaining() {
    assert_eq!(out("var n = 0; function hit() { n++; return 'x' } var a = true || hit(); var b = false && hit(); var c = 1 ? 'y' : hit(); var d = 'z' ?? hit(); document.write(a, b, c, d, n)").unwrap(), "truefalseyz0");
    assert_eq!(out("var o = null; document.write(o && o.x, ' ', o ?? 'dflt', ' ', 0 ?? 1, ' ', '' || 'e', ' ', undefined ?? null ?? 3)").unwrap(), "null dflt 0 e 3");
    assert_eq!(out("var cfg = {user: {name: 'Ann', tags: ['a', 'b']}}; var none; document.write(cfg.user?.name, ' ', cfg.admin?.name, ' ', none?.a.b.c, ' ', cfg?.user?.tags?.[1], ' ', none?.[0], ' ', cfg.admin?.name ?? 'guest')").unwrap(), "Ann undefined undefined b undefined guest");
    assert_eq!(out("var a = 1, b, c = a + 1; var a; document.write(a, b, c)").unwrap(), "1undefined2");
    assert_eq!(out("const q;").err(), Some(Error::Syntax));
    assert_eq!(out("var n = 0; function f() { n++; return 0 } var x = null; var y = x?.[f()]; document.write(y, n, ' ', true ? 1 : .5, ' ', false?.5:2)").unwrap(), "undefined0 1 2");
}

#[test]
fn for_of_in_defaults_rest_and_assign() {
    assert_eq!(out("var t = ''; for (const x of [1, 2, 3]) { if (x === 2) continue; t += x } for (let c of 'ab') t += c.toUpperCase(); for (var k in {p: 1, q: 2}) t += k; for (var i in [7, 8]) t += typeof i + i; document.write(t)").unwrap(), "13ABpqstring0string1");
    assert_eq!(out("function first(xs) { for (var x of xs) { if (x > 1) return x } return 0 } var s = 0; for (var y of [1, 2, 3, 4]) { if (y > 2) break; s += y } document.write(first([1, 5, 9]), ' ', s, y)").unwrap(), "5 33");
    assert_eq!(out("for (var x of {a: 1}) document.write(x)").err(), Some(Error::Unsupported), "plain objects are not iterable");
    assert_eq!(out("var n = 0; function h() { n++; return 9 } function f(a = h(), b = a * 2, ...more) { return a + b + more.length } var g = (x, y = 1) => x * y; document.write(f(1), ' ', n, ' ', f(), ' ', n, ' ', f(1, 1, 5, 5), ' ', g(3), g(3, 2))").unwrap(), "3 0 27 1 4 36");
    assert_eq!(out("function f(...a, b) {}").err(), Some(Error::Syntax));
    assert_eq!(out("var o = Object.assign({}, {a: 1}, null, {b: 2, a: 3}); var d = {x: 1}; document.write(JSON.stringify(o), Object.assign(d, {y: 2}) === d, d.y)").unwrap(), "{\"a\":3,\"b\":2}true2");
}

#[test]
fn more_builtins_and_split_limit() {
    assert_eq!(out("document.write([3, 1, 2].toString(), ' ', [1, 2, 3].findLast(x => x < 3), [1, 2, 3].findLastIndex(x => x > 5), ' ', [1, 2, 1].lastIndexOf(1), ' ', [1, 2].flatMap(x => [x, x * 10]))").unwrap(), "3,1,2 2-1 2 1,10,2,20");
    assert_eq!(out("var a = [1, 2, 3, 4, 5]; var r = a.splice(1, 2, 'x', 'y'); var b = [1, 2, 3]; b.splice(1); var c = []; for (var i = 0; i < 6; i++) c.unshift(i); document.write(a, ' ', r, ' ', b, ' ', c, ' ', c.unshift(9))").unwrap(), "1,x,y,4,5 2,3 1 5,4,3,2,1,0 7");
    assert_eq!(out("document.write(Array.of(7, 8), ' ', Object.fromEntries([['a', 1], ['b', [2]]]).b[0], ' ', 'é'.codePointAt(0), 'A'.codePointAt(0))").unwrap(), "7,8 2 23365");
    assert_eq!(out("document.write(encodeURIComponent('a b&c/é'), ' ', decodeURIComponent('a%20b%C3%A9'))").unwrap(), "a%20b%26c%2F%C3%A9 a bé");
    assert_eq!(out("decodeURIComponent('%E0%A4%A')").err(), Some(Error::Unsupported), "URIError");
    assert_eq!(out(r"document.write('a-b-c'.split('-', 2), ' ', 'a1b2c'.split(/\d/, 1), ' ', 'abc'.split('', 0).length)").unwrap(), "a,b a 0");
}

#[test]
fn static_methods_and_instanceof() {
    assert_eq!(out("class A { static make(x) { return new A(x) } constructor(x) { this.x = x } } class B extends A {} var a = A.make(3); var b = new B(1); document.write(a.x, ' ', a instanceof A, b instanceof A, a instanceof B, ' ', B.make(5).x, ' ', [] instanceof Array, {} instanceof Object, 5 instanceof Object)").unwrap(), "3 truetruefalse 5 truetruefalse");
    assert_eq!(out("function P() { this.k = 1 } var p = new P(); class S { static() { return 1 } } var s = new S(); document.write(p instanceof P, {} instanceof P, s.static())").unwrap(), "truefalse1");
    assert_eq!(out("try { throw new TypeError('t') } catch (e) { document.write(e instanceof Error, e instanceof TypeError, e instanceof RangeError, ' ', {} instanceof Error) }").unwrap(), "truetruefalse false");
    assert_eq!(out("document.write(1 instanceof Nope)").err(), Some(Error::Unsupported));
    assert_eq!(out("class C { static v() { return 2 } } document.write(C.w())").err(), Some(Error::Unsupported));
}

#[test]
fn exponent_bitwise_logical_assignment_and_expression_statements() {
    assert_eq!(out("var x = 5; x **= 2; document.write(x, ' ', 2 ** 3 ** 2, ' ', -(2 ** 2))").unwrap(), "25 512 -4");
    assert_eq!(out("document.write(1 < 2 < 3, ' ', 7 & 3, 5 | 2, 5 ^ 1, ' ', ~5, ' ', 1 << 4, ' ', -16 >> 2, ' ', -1 >>> 28, ' ', 1 << 31, ' ', 2 ** 32 | 0, ' ', 5.7 | 0, ' ', 3 & 1 === 1, ' ', true && 'y', 1 && 2 || 3)").unwrap(), "true 374 -6 16 -4 15 -2147483648 0 5 1 y2");
    assert_eq!(out("var n = 0; function bump() { n++; return 7 } var a = 1; a ||= bump(); var b = null; b ??= 3; var c = 1; c &&= 9; var f = 6; f |= 1; f &= 5; f ^= 4; var o = {k: null}; o.k ??= 'set'; document.write(a, b, c, n, f, o.k)").unwrap(), "13901set");
    assert_eq!(out("var s = 0; [1, 2].forEach(function (x) { s += x }); var i = 0; ++i; ++i; --i; (function () { s += 10 })(); document.write(s, i, typeof null, void 0)").unwrap(), "131objectundefined");
    assert_eq!(out("document.write(Number('0x10'), ' ', Number('0b101'), ' ', Number('inf'), ' ', Number('-Infinity'), ' ', Number(' 12 '), ' ', +'0o17', ' ', '3' * '4', ' ', Number('-0x10'))").unwrap(), "16 5 NaN -Infinity 12 15 12 NaN");
    assert_eq!(out("var r = (function (a) { return a * 2 })(4); var fs = [x => x + 1]; var mk = function () { return function (y) { return y * 3 } }; document.write(r, fs[0](1), (() => 'k')(), mk()(2))").unwrap(), "82k6");
}

#[test]
fn calls_take_up_to_six_arguments() {
    assert_eq!(out("function f(a, b, c, d, e, g) { return [a, b, c, d, e, g].join('') } var a = [1, 2, 3, 4, 5]; a.splice(1, 1, 'x', 'y', 'z'); document.write(f(1, 2, 3, 4, 5, 6), ' ', a, ' ', new Date(2024, 0, 2, 3, 4, 5).toISOString())").unwrap(), "123456 1,x,y,z,3,4,5 2024-01-02T03:04:05.000Z");
    assert_eq!(out("function f() {} f(1, 2, 3, 4, 5, 6, 7)").err(), Some(Error::Limit));
}

#[test]
fn increments_in_expressions_and_member_chain_assignment() {
    assert_eq!(out("var i = 0; var a = [5, 6]; function f() { var n = 1; return n++ } var k = 3; var j = ++k + k--; var c = 0; while (c++ < 3) {} for (var t = 0, s = ''; t < 3; ) s += t++; document.write(a[i++], i, f(), k, j, c, s)").unwrap(), "511384012");
    assert_eq!(out("var a = 1\n++a\ndocument.write(a)").unwrap(), "2", "ASI before a prefix ++");
    assert_eq!(out("var o = {a: {b: [1, 2]}}; o.a.b[1] = 9; o.a.c = 'k'; o['a'].b[0] -= 1; var m = [[0, 0], [0, 0]]; m[1][0] = 7; var x = [1]; x[0]++; x[0] += 5; var xs = [1, 2, 3]; xs.length = 1; document.write(JSON.stringify(o), m, ' ', x, xs, xs.length)").unwrap(), "{\"a\":{\"b\":[0,9],\"c\":\"k\"}}0,0,7,0 711");
    assert_eq!(out("class S { constructor() { this.items = [] } add(x) { this.items[this.items.length] = x; return this } } var s = new S(); s.add(4).add(5); document.write(s.items)").unwrap(), "4,5");
    assert_eq!(out("var xs = [1]; xs.length = 5").err(), Some(Error::Unsupported), "growing length");
    assert_eq!(out("var n = 1; n.x = 2").err(), Some(Error::Unsupported));
}

#[test]
fn object_literal_shorthand_spread_and_richer_destructuring() {
    assert_eq!(out("var a = 1; var o = {a, b() { return this.a + 1 }, 2: 'two', ['c' + 1]: 3, f() {}, }; var p = {...{a: 1, b: 1}, b: 2, ...null}; document.write(JSON.stringify(o), o.b(), typeof o.f, JSON.stringify(p), JSON.stringify([o.f]))").unwrap(), "{\"a\":1,\"2\":\"two\",\"c1\":3}2function{\"a\":1,\"b\":2}[null]");
    assert_eq!(out("var o = {...[1]}").err(), Some(Error::Unsupported));
    assert_eq!(out("const {name, price = 1} = {name: 'x'}; const [first, ...others] = [1, 2, 3]; var [a, , c = 9, d = 4] = [1, 2, undefined, 0]; document.write(name, price, first, '|', others, '|', a, c, d)").unwrap(), "x11|2,3|190");
    assert_eq!(out("var n = 0; function f() { n++; return 5 } var {x = f(), y: why = f(), ...more} = {x: 1, z: 3, w: 4}; document.write(x, why, n, JSON.stringify(more))").unwrap(), "151{\"z\":3,\"w\":4}");
    assert_eq!(out("var t = ''; for (const [k, v] of Object.entries({a: 1, b: 2})) t += k + '=' + v + ';'; for (let {n, m = 0} of [{n: 1}, {n: 2, m: 3}]) t += n + m; function g() { for (const [a] of [[7]]) return a } document.write(t, g())").unwrap(), "a=1;b=2;157");
}

#[test]
fn closures_and_function_scoped_var() {
    assert_eq!(out("var x = 1; function f(x) { x = 5; var y = 2 } function g() { var x = 2 } f(9); g(); document.write(x, typeof y)").unwrap(), "1undefined");
    assert_eq!(out("function counter() { let c = 0; return () => ++c } var a = counter(); var b = counter(); a(); a(); document.write(a(), b())").unwrap(), "31", "each call's closure has its own variables");
    assert_eq!(out("var api = (function () { var n = 10; return { inc() { n++; return this }, get() { return n } } })(); api.inc().inc(); document.write(api.get())").unwrap(), "12", "methods returned together share variables");
    assert_eq!(out("function adder(k) { return function (x) { return x + k } } var add5 = adder(5); function mk(x) { return {x, get() { return x }} } document.write([1, 2].map(add5), ' ', mk(4).get(), mk(7).get())").unwrap(), "6,7 47");
    assert_eq!(out("function f() { return f } document.write(typeof f()())").unwrap(), "function", "a function returning itself is not a closure");
}

#[test]
fn getters_and_setters() {
    assert_eq!(out("var o = { first: 'Ada', last: 'L', get full() { return this.first + ' ' + this.last }, set full(v) { var p = v.split(' '); this.first = p[0]; this.last = p[1] } }; document.write(o.full); o.full = 'Alan T'; document.write('|', o.first, o.full)").unwrap(), "Ada L|AlanAlan T");
    assert_eq!(out("class T { constructor() { this._c = 0 } get c() { return this._c } set c(v) { this._c = v * 2 } get() { return 'm' } } class U extends T {} var t = new U(); t.c = 5; document.write(t.c, t.get())").unwrap(), "10m");
    assert_eq!(out("var o = { get x() { return 3 } }; o.x = 9; var get = 1; var p = {get, set: 2}; document.write(o.x, p.get, p.set)").unwrap(), "312", "no setter: the assignment is ignored");
}

#[test]
fn labelled_break_and_continue() {
    assert_eq!(out("outer: for (var i = 0; i < 3; i++) { for (var j = 0; j < 3; j++) { if (j == 1) continue outer; if (i == 2) break outer; document.write(i, j, ' ') } }").unwrap(), "00 10 ");
    assert_eq!(out("var t = ''; blk: { t += 'a'; break blk; t += 'b' } var n = 0; loop1: while (true) { do { n++; if (n > 4) break loop1 } while (true) } document.write(t, n)").unwrap(), "a5");
    assert_eq!(out("a: for (var x of [1, 2, 3]) { switch (x) { case 2: continue a; case 3: break a } document.write(x) } switch (1) { case 1: document.write('c') }").unwrap(), "1c");
    assert_eq!(out("break nowhere").err(), Some(Error::Syntax));
}

#[test]
fn builtin_conversions_as_function_values() {
    assert_eq!(out("var f = Number; document.write([1, 2, 3].map(String).join('-'), ' ', ['1', '2.5'].map(Number), ' ', [0, 'a', '', null, 3].filter(Boolean), ' ', Boolean(''), Boolean('x'), typeof String, typeof Boolean, ' ', f('7') + 1)").unwrap(), "1-2-3 1,2.5 a,3 falsetruefunctionfunction 8");
    assert_eq!(out("var String = 5; document.write(String)").unwrap(), "5", "a script's own binding wins");
}

#[test]
fn sets_maps_and_assignment_expressions() {
    assert_eq!(out("var s = new Set([1, 2, 2, 3]); s.add(4).add(1); var z = new Set(); z.add(NaN).add(NaN); var o = {}; z.add(o).add(o).add({}); document.write(s.size, s.has(2), s.has(9), ' ', [...s], ' ', [...new Set('hello')].join(''), ' ', z.size, z instanceof Set, JSON.stringify(z))").unwrap(), "4truefalse 1,2,3,4 helo 3true{}");
    assert_eq!(out("var m = new Map([['a', 1]]); m.set('b', 2).set('a', 3); var t = ''; for (const [k, v] of m) t += k + v; m.forEach((v, k) => t += '|' + k + '=' + v); document.write(m.size, m.get('a'), m.get('z'), ' ', t, ' ', m.delete('a'), m.size, ' ', [...m.keys()], ' ', Array.from(m.values()), m instanceof Map)").unwrap(), "23undefined a3b2|a=3|b=2 true1 b 2true");
    assert_eq!(out("var a, b; a = b = 5; var o = {}; var r = (o.k = 2) * 3; var n = 0; [1, 2, 3].forEach(x => n += x); var x = 1; if (x = 2) document.write(a, b, r, o.k, n, x)").unwrap(), "556262");
}

#[test]
fn number_formatting_regex_exec_and_object_helpers() {
    assert_eq!(out("document.write((1234.5).toLocaleString(), ' ', (-1234567.891).toLocaleString(), ' ', (0.0004).toLocaleString(), ' ', (12.345).toPrecision(3), ' ', (123456).toPrecision(2), ' ', (0.000001234).toPrecision(2), ' ', (9.99).toPrecision(2), ' ', (0).toPrecision(3))").unwrap(), "1,234.5 -1,234,567.891 0 12.3 1.2e+5 0.0000012 10 0.00");
    assert_eq!(out(r"var re = /(\w)(\d)/g; var m; var t = ''; while ((m = re.exec('a1 b2')) !== null) t += m[1] + re.lastIndex; document.write(t, re.lastIndex, ' ', /x(y)/.exec('axy'), ' ', 'a1b2'.matchAll(/(\w)(\d)/g).map(m => m[2]).join('+'))").unwrap(), "a2b50 xy,y 1+2");
    assert_eq!(out("document.write('b'.localeCompare('a'), ['b', 'a', 'C', 'A'].sort((x, y) => x.localeCompare(y)))").unwrap(), "1a,A,b,C");
    assert_eq!(out("var o = Object.freeze({a: 1}); o.a = 2; o.b = 3; var a = [7, 8]; var t = ''; for (const [i, v] of a.entries()) t += i + '' + v; document.write(o.a, o.b, Object.isFrozen(o), ' ', Number.MAX_SAFE_INTEGER, ' ', t, a.keys(), ' ', ({a: 1}).hasOwnProperty('a'), Object.hasOwn({k: 0}, 'z'))").unwrap(), "1undefinedtrue 9007199254740991 07180,1 truefalse");
}

#[test]
fn nested_templates_named_function_expressions_and_more_builtins() {
    assert_eq!(out(r#"var a = [1, 2]; var o = {n: 2}; document.write(`list: ${a.map(x => `<${x}>`).join("")}`, ' ', `${o.n > 1 ? `${o.n} items` : "one"}`, ' ', `x\`y`, `${`${`deep`}`}`, `a${"`"}b`)"#).unwrap(), "list: <1><2> 2 items x`ydeepa`b");
    assert_eq!(out("var f = function fact(n) { return n < 2 ? 1 : n * fact(n - 1) }; var {a: {b}, c: [d, e = 4]} = {a: {b: 7}, c: [1]}; var t = ''; for (const {p: {q}} of [{p: {q: 1}}, {p: {q: 2}}]) t += q; document.write(f(5), b, d, e, t)").unwrap(), "12071412");
    assert_eq!(out("document.write([1, 2, 3].reduceRight((a, b) => a + '' + b), ' ', ['a', 'b'].reduceRight((a, b) => a + b, '>'), ' ', 'abcb'.replace('b', m => m.toUpperCase()), 'abcb'.replaceAll('b', (m, i) => i), ' ', Array.from({length: 3}, (_, i) => i * i), Array.from([1, 2], x => x * 10))").unwrap(), "321 >ba aBcba1c3 0,1,410,20");
}

#[test]
fn destructured_parameters_pretty_json_and_write_expressions() {
    assert_eq!(out("function f({x, y = 2}, [p, q]) { return x + y + p + q } document.write(Object.entries({a: 1, b: 2}).map(([k, v]) => k + v).join(), ' ', f({x: 1}, [10, 20]), ' '); [[1, 2]].forEach(([a, b]) => document.write(a + b))").unwrap(), "a1,b2 33 3");
    assert_eq!(out("document.write(JSON.stringify({a: [1, {b: 2}], e: [], o: {}}, null, 2))").unwrap(), "{\n  \"a\": [\n    1,\n    {\n      \"b\": 2\n    }\n  ],\n  \"e\": [],\n  \"o\": {}\n}");
    assert_eq!(out("document.write(JSON.stringify({a: 1, b: 'x', c: {a: 2, z: 3}}, ['a', 'c']), JSON.stringify([1, 'a'], null, '--'))").unwrap(), "{\"a\":1,\"c\":{\"a\":2}}[\n--1,\n--\"a\"\n]");
    assert_eq!(out("JSON.stringify({}, (k, v) => v)").err(), Some(Error::Unsupported), "function replacer");
}

#[test]
fn larger_budgets_for_bigger_pages() {
    assert_eq!(out("var big = Array.from({length: 400}, (_, i) => i); var ob = {}; for (var i = 0; i < 40; i++) ob['k' + i] = i; document.write('big ', big.reduce((a, b) => a + b), ' ', Object.keys(ob).length, ' ', 'a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p,q,r,s'.split(',').length)").unwrap(), "big 79800 40 19");
    assert!(out("var i = 0; while (i < 45000) i++; document.write(i)").is_ok(), "50,000 steps");
}

#[test]
fn promises_run_as_ordered_microtasks_after_the_script() {
    assert_eq!(out("Promise.resolve(1).then(v => document.write('a')); Promise.resolve(2).then(v => document.write('b')); document.write('c')").unwrap(), "cab");
    assert_eq!(out("new Promise((res, rej) => { document.write('exec '); res(5) }).then(v => v * 2).then(v => document.write(v))").unwrap(), "exec 10");
    assert_eq!(out("Promise.reject(new Error('bad')).then(() => document.write('no')).catch(e => document.write('caught ', e.message)).finally(() => document.write(' fin'))").unwrap(), "caught bad fin");
    assert_eq!(out("Promise.all([1, Promise.resolve(2), new Promise(r => r(3))]).then(xs => document.write(xs.join('+'))); new Promise(() => { throw new Error('x') }).catch(e => document.write(' ', e.message)); Promise.resolve(1).then(() => { throw 7 }).then(() => 0, e => document.write(' r', e))").unwrap(), " x1+2+3 r7", "JavaScript job order: Promise.all settles after its items' reactions");
    assert_eq!(out("var p = Promise.resolve(Promise.resolve(9)); p.then(v => document.write(v)); document.write(p instanceof Promise)").unwrap(), "true9");
    assert_eq!(out("async function f() {}").err(), Some(Error::Unsupported), "no async functions");
}

#[test]
fn optional_calls_tagged_templates_and_string_raw() {
    assert_eq!(out("var f = null; var g = x => x * 2; document.write(f?.(1), g?.(4), ({m: null}).m?.(), ({k: () => 3}).k?.())").unwrap(), "undefined8undefined3");
    assert_eq!(out(r#"function tag(s, a, b) { return s.join("|") + ":" + a + "," + b } var x = 1; document.write(tag`A${x}B${x + 1}C`, ' ', String.raw`a\nb${1 + 1}\t`)"#).unwrap(), "A|B|C:1,2 a\\nb2\\t");
}

#[test]
fn elementary_functions_stay_within_a_few_ulps_of_std() {
    let ulps = |a: f64, b: f64| if a == b { 0 } else { (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs() };
    let mut x = -40.0f64;
    while x < 40.0 {
        let (s, c) = fmath::sin_cos(x);
        assert!(ulps(s, x.sin()) <= 4 || (s - x.sin()).abs() < 1e-16, "sin {x}: {s} vs {}", x.sin());
        assert!(ulps(c, x.cos()) <= 4 || (c - x.cos()).abs() < 1e-16, "cos {x}");
        assert!(ulps(fmath::exp(x), x.exp()) <= 4, "exp {x}: {} vs {}", fmath::exp(x), x.exp());
        assert!(ulps(fmath::atan(x), x.atan()) <= 4, "atan {x}");
        if x > 0.0 { assert!(ulps(fmath::ln(x), x.ln()) <= 4, "ln {x}: {} vs {}", fmath::ln(x), x.ln()); }
        x += 0.0937;
    }
    for &(y, x) in &[(1.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (0.0, -1.0), (-0.0, -1.0), (3.0, 0.0), (-2.0, 5.0)] {
        assert!(ulps(fmath::atan2(y, x), f64::atan2(y, x)) <= 4, "atan2 {y} {x}");
    }
    assert_eq!(fmath::exp(0.0), 1.0);
    assert_eq!(fmath::ln(1.0), 0.0);
    assert!(fmath::ln(-1.0).is_nan());
    assert_eq!(fmath::exp(1000.0), f64::INFINITY);
    assert_eq!(out("document.write(Math.round(Math.sin(Math.PI / 6) * 1e6) / 1e6, ' ', Math.log(Math.E), ' ', Math.log10(1000), ' ', Math.log2(8), ' ', Math.cbrt(27), ' ', Math.hypot(3, 4), ' ', Math.atan2(1, 1) * 4 === Math.PI, ' ', Math.exp(0), ' ', Math.cos(0))").unwrap(), "0.5 1 3 3 3 5 true 1 1");
}

#[test]
fn class_fields_static_fields_and_private_names() {
    assert_eq!(out("class Counter { count = 0; #step = 2; static made = 0; constructor() { Counter.made++ } inc() { this.count += this.#step; return this } } var a = new Counter(); a.inc().inc(); new Counter(); document.write(a.count, ' ', Counter.made, ' ', JSON.stringify(a), Object.keys(a))").unwrap(), "4 2 {\"count\":4}count");
    assert_eq!(out("class P { x = 1; y = this.x + 1 } class Q extends P { z = this.y * 10; static label = 'q' } class S { static a = 1; static b = S.a + 1; static c } var q = new Q(); document.write(q.x, q.y, q.z, Q.label, S.b, S.c)").unwrap(), "1220q2undefined");
    assert_eq!(out("class C { a = 1; #b = 2; m() {} } var c = new C(); c.extra = () => 0; var t = ''; for (var k in c) t += k; document.write(Object.keys(c), ' ', t)").unwrap(), "a,extra aextra", "methods and #private fields are not listed");
}
