// SPDX-License-Identifier: GPL-3.0-only
mod common;
use common::*;
use spindle_shell::{Engine, Record, Value};

fn s(x: &str) -> Value { Value::string(x) }
fn strs(xs: &[&str]) -> Value { Value::List(xs.iter().map(|x| s(x)).collect()) }
fn ints(xs: &[i64]) -> Value { Value::List(xs.iter().map(|x| Value::Int(*x)).collect()) }

#[test]
fn ls_lists_patterns_and_hidden() {
    assert_eq!(val("ls | get name"), strs(&["archive.tar", "config.toml", "data.json", "disk.img", "docs", "notes.txt", "people.csv", "src", "video.mkv"]));
    assert_eq!(val("ls -a | where name starts-with . | get name"), strs(&[".hidden"]));
    assert_eq!(val("ls src/*.rs | get name"), strs(&["src/lib.rs", "src/main.rs"]));
    assert_eq!(val("ls *.t?t | get name"), strs(&["notes.txt"]));
    assert_eq!(val("ls notes.txt | get 0.size"), Value::Filesize(32));
    assert_eq!(val("ls | get 0 | columns"), strs(&["name", "type", "size", "modified"]));
    assert_eq!(val("ls | where type == dir | get name"), strs(&["docs", "src"]));
    assert!(err("ls nowhere").contains("can't list `nowhere`: not found"));
}

#[test]
fn cd_pwd_and_paths() {
    let mut e = Engine::new();
    let mut h = host();
    assert_eq!(run(&mut e, &mut h, "cd src; pwd").value, s("/home/src"));
    assert_eq!(run(&mut e, &mut h, "ls | length").value, Value::Int(3));
    assert_eq!(run(&mut e, &mut h, "cd ..; pwd").value, s("/home"));
    assert_eq!(run(&mut e, &mut h, "cd /; cd; pwd").value, s("/home"));
    let out = run(&mut e, &mut h, "cd notes.txt");
    assert_eq!(out.text, "error: `notes.txt` is not a folder\n  cd notes.txt\n     ^^^^^^^^^ a file\n");
}

#[test]
fn file_changes() {
    let mut e = Engine::new();
    let mut h = host();
    let ok_run = |e: &mut Engine, h: &mut spindle_shell::MemHost, l: &str| { let o = run(e, h, l); assert!(o.error.is_none(), "{}: {}", l, o.text); o };
    ok_run(&mut e, &mut h, "mkdir new; touch new/a.txt; cp notes.txt new; mv new/a.txt new/b.txt");
    assert_eq!(h.file("/home/new/b.txt"), Some(&b""[..]));
    assert_eq!(h.file("/home/new/notes.txt").map(|d| d.len()), Some(32));
    ok_run(&mut e, &mut h, "cp -r new copy; rm -r new");
    assert!(!h.exists("/home/new") && h.exists("/home/copy/b.txt"));
    let out = run(&mut e, &mut h, "rm copy");
    assert!(out.text.contains("`copy` is a folder") && out.text.contains("use `rm -r`"), "{}", out.text);
    ok_run(&mut e, &mut h, "rm copy/*.txt");
    assert!(!h.exists("/home/copy/b.txt") && h.exists("/home/copy"));
    // save and redirects.
    ok_run(&mut e, &mut h, "'one' | save log.txt; 'two' | save -a log.txt");
    assert_eq!(h.file("/home/log.txt"), Some(&b"onetwo"[..]));
    let out = run(&mut e, &mut h, "'x' | save log.txt");
    assert!(out.text.contains("`log.txt` already exists") && out.text.contains("save -f"), "{}", out.text);
    ok_run(&mut e, &mut h, "'x' | save -f log.txt; 'y' >> log.txt");
    assert_eq!(h.file("/home/log.txt"), Some(&b"xy"[..]));
    ok_run(&mut e, &mut h, "open people.csv | where age > 30 | save old.json");
    assert_eq!(val_in(&mut e, &mut h, "open old.json | get name"), strs(&["Ada", "Grace"]));
    ok_run(&mut e, &mut h, "open people.csv | select name age > p.csv");
    assert_eq!(std::str::from_utf8(h.file("/home/p.csv").unwrap()).unwrap(), "name,age\nAda,36\nLinus,28\nGrace,45\n");
    ok_run(&mut e, &mut h, "[a b] > list.txt");
    assert_eq!(h.file("/home/list.txt"), Some(&b"a\nb\n"[..]));
}

fn val_in(e: &mut Engine, h: &mut spindle_shell::MemHost, l: &str) -> Value {
    let o = run(e, h, l);
    assert!(o.error.is_none(), "{}", o.text);
    o.value
}

#[test]
fn open_parses_by_extension() {
    assert_eq!(val("open data.json | get name"), s("spindle"));
    assert_eq!(val("open data.json | get items | where done == false | get title"), strs(&["b"]));
    assert_eq!(val("open people.csv | get 1.city"), s("Helsinki, FI"));
    assert_eq!(val("open people.csv | get age | math max"), Value::Int(45));
    assert_eq!(val("open config.toml | get display.scale"), Value::Float(1.5));
    assert_eq!(val("open config.toml | get user.name"), strs(&["ada", "bob"]));
    assert_eq!(val("open notes.txt | length"), Value::Int(3));
    assert_eq!(val("open --raw notes.txt | lines | first"), s("buy milk"));
    assert_eq!(val("open src/readme.md"), s("# hi\n"));
}

#[test]
fn filters() {
    assert_eq!(val("seq 5 | where $it > 2"), ints(&[3, 4, 5]));
    assert_eq!(val("seq 10 | skip 2 | take 3"), ints(&[3, 4, 5]));
    assert_eq!(val("seq 10 | first 2"), ints(&[1, 2]));
    assert_eq!(val("seq 10 | last 2"), ints(&[9, 10]));
    assert_eq!(val("seq 10 | first"), Value::Int(1));
    assert_eq!(val("seq 10 | last"), Value::Int(10));
    assert_eq!(val("seq 3 | reverse"), ints(&[3, 2, 1]));
    assert_eq!(val("[b a b c a] | uniq"), strs(&["b", "a", "c"]));
    assert_eq!(val("[3 1 2] | sort-by"), ints(&[1, 2, 3]));
    assert_eq!(val("[B a C] | sort-by -i"), strs(&["a", "B", "C"]));
    assert_eq!(val("seq 2 2 8"), ints(&[2, 4, 6, 8]));
    assert_eq!(val("seq 3 1"), ints(&[3, 2, 1]));
    assert_eq!(val("ls | length"), Value::Int(9));
    assert_eq!(val("open people.csv | select name | get 0"), Value::Record(Record::new().with("name", s("Ada"))));
    assert_eq!(val("open people.csv | reject age city | get name"), strs(&["Ada", "Linus", "Grace"]));
    assert_eq!(val("open people.csv | sort-by city name | get name"), strs(&["Grace", "Linus", "Ada"]));
    assert_eq!(val("open people.csv | sort-by age -r | get name"), strs(&["Grace", "Ada", "Linus"]));
    assert_eq!(val("open people.csv | where name in [Ada Grace] | length"), Value::Int(2));
    assert_eq!(val("open people.csv | where city =~ ^Hel | get name"), strs(&["Linus"]));
    assert_eq!(val("open people.csv | where city !~ on | get name"), strs(&["Linus"]));
    assert_eq!(val("open people.csv | where not (age < 30) and name != Ada | get name"), strs(&["Grace"]));
    assert_eq!(val("ls | where modified > 2026-11-01T12:30:00Z | get name"), strs(&["disk.img", "docs", "src"]));
}

#[test]
fn group_update_insert_wrap() {
    assert_eq!(val("ls | group-by type | get dir | length"), Value::Int(2));
    assert_eq!(val("ls | group-by type | columns"), strs(&["file", "dir"]));
    assert_eq!(val("open people.csv | update age (age + 1) | get age"), ints(&[37, 29, 46]));
    assert_eq!(val("open people.csv | update city unknown | get city.0"), s("unknown"));
    assert_eq!(val("open people.csv | insert adult (age >= 30) | get adult"), Value::List(vec![Value::Bool(true), Value::Bool(false), Value::Bool(true)]));
    assert_eq!(val("open people.csv | insert old ($in == null) | get old.0"), Value::Bool(true));
    assert_eq!(val("ls | where size > 1mb | update size (size / 1kb) | get size.0"), Value::Float(2500.0));
    assert_eq!(val("seq 2 | wrap n"), Value::List(vec![Value::Record(Record::new().with("n", Value::Int(1))), Value::Record(Record::new().with("n", Value::Int(2)))]));
    assert!(err("open people.csv | insert age 1").contains("column `age` already exists"));
    assert!(err("open people.csv | update agee 1").contains("did you mean `age`?"));
}

#[test]
fn math() {
    assert_eq!(val("[1 2 3 4] | math sum"), Value::Int(10));
    assert_eq!(val("[1 2 3 4] | math avg"), Value::Float(2.5));
    assert_eq!(val("[1.5 2] | math sum"), Value::Float(3.5));
    assert_eq!(val("[3 9 1] | math min"), Value::Int(1));
    assert_eq!(val("[3 9 1] | math max"), Value::Int(9));
    assert_eq!(val("ls src | get size | math sum"), Value::Filesize(26));
    assert_eq!(val("[1sec 3sec] | math avg"), Value::Duration(2_000_000_000));
    assert_eq!(val("open people.csv | math sum"), Value::Record(Record::new().with("age", Value::Int(109))));
    assert_eq!(val("7 / 2"), Value::Float(3.5));
    assert_eq!(val("6 / 2"), Value::Int(3));
    assert_eq!(val("7 mod 3"), Value::Int(1));
    assert_eq!(val("2 ** 10"), Value::Int(1024));
    assert_eq!(val("(1 + 2) * 3"), Value::Int(9));
    assert_eq!(val("1 + 2 * 3"), Value::Int(7));
    assert_eq!(val("1mb + 1kb"), Value::Filesize(1_001_000));
    assert_eq!(val("2min - 30sec"), Value::Duration(90_000_000_000));
    assert_eq!(val("2026-11-14 + 1day"), Value::Date(1_794_614_400 + 86_400));
    assert_eq!(val("'a' ++ 'b'"), s("ab"));
    assert!(err("[a b] | math sum").contains("can't"));
}

#[test]
fn strings() {
    assert_eq!(val("'Hello World' | str upcase"), s("HELLO WORLD"));
    assert_eq!(val("'Hello World' | str downcase"), s("hello world"));
    assert_eq!(val("'  pad  ' | str trim"), s("pad"));
    assert_eq!(val("'漢字abc' | str length"), Value::Int(5));
    assert_eq!(val("'a.b.c' | str replace . /"), s("a/b.c"));
    assert_eq!(val("'a.b.c' | str replace -a . /"), s("a/b/c"));
    assert_eq!(val("'Spindle' | str contains pin"), Value::Bool(true));
    assert_eq!(val("'Spindle' | str contains -i SPIN"), Value::Bool(true));
    assert_eq!(val("[ab cd] | str upcase"), strs(&["AB", "CD"]));
    assert_eq!(val("open people.csv | str upcase name city | get 0.city"), s("LONDON"));
    assert_eq!(val("'a,b,,c' | split row ','"), strs(&["a", "b", "", "c"]));
    assert_eq!(val("\"x\\ny\\r\\nz\\n\" | lines"), strs(&["x", "y", "z"]));
    assert!(err("open people.csv | str upcase").contains("needs the columns to change"));
    assert!(err("open people.csv | str upcase age").contains("holds a int, not text"));
}

#[test]
fn formats_round_trip() {
    assert_eq!(val("{a: [1 2.5 true null], b: \"q\\\"\"} | to json -r"), s(r#"{"a":[1,2.5,true,null],"b":"q\""}"#));
    assert_eq!(val("'{\"a\": [1, {\"b\": \"\\u00e9\"}]}' | from json | get a.1.b"), s("é"));
    assert_eq!(val("open people.csv | to json | from json | get 2.name"), s("Grace"));
    assert_eq!(val("open people.csv | to tsv | from tsv | get age"), ints(&[36, 28, 45]));
    assert_eq!(val("'a;b\n1;2' | from csv -s ';' | get b.0"), Value::Int(2));
    assert_eq!(val("'1,2\n3,4' | from csv -n | get column1"), ints(&[2, 4]));
    assert_eq!(val("'x = 1\n[t]\ny = [1, 2]' | from toml | get t.y.1"), Value::Int(2));
    assert_eq!(val("ls | where size > 1mb | get size | to json -r"), s("[2500000,3221225472,734003200]"));
    assert!(err("'{bad' | from json").contains("can't parse the input"));
    assert_eq!(val("seq 3 | to text"), s("1\n2\n3"));
}

#[test]
fn values_and_variables() {
    assert_eq!(val("echo a b"), strs(&["a", "b"]));
    assert_eq!(val("echo 1kb"), Value::Filesize(1000));
    assert_eq!(val("let x = (ls | length); $x + 1"), Value::Int(10));
    assert_eq!(val("let t = (open people.csv); $t.name.1"), s("Linus"));
    assert_eq!(val("ls | where size > 1mb | length | $in * 2"), Value::Int(6));
    assert_eq!(val("ls | describe"), s("table<name, type, size, modified> (9 rows)"));
    assert_eq!(val("date now"), Value::Date(T0 + 86_400));
    assert_eq!(val("sys | get cpus"), Value::Int(4));
    assert_eq!(val("ps | get name"), strs(&["silk-shell"]));
    let mut e = Engine::new();
    let mut h = host();
    run(&mut e, &mut h, "let big = (ls | where size > 1mb)");
    assert_eq!(run(&mut e, &mut h, "$big | length").value, Value::Int(3));
    assert_eq!(e.var("big").map(|v| v.type_name()), Some("table"));
}

#[test]
fn help_lists_and_explains() {
    let v = val("help");
    let Value::List(rows) = &v else { panic!() };
    assert!(rows.len() > 50);
    assert_eq!(val("help | where command == where | get description.0"), s("Keep the rows where the condition holds: where size > 1mb and name =~ rs"));
    assert_eq!(val("help math | get command"), strs(&["math sum", "math avg", "math min", "math max"]));
    assert_eq!(ok("help sort-by"), "Sort rows by columns (or a list by its values)\n\nUsage:\n  sort-by <column>...\n\nFlags:\n  -r, --reverse      largest first\n  -i, --ignore-case  sort text without regard to case\n");
    assert!(err("help sortby").contains("did you mean `sort-by`?"));
}

#[test]
fn runtime_errors_have_spans() {
    assert_eq!(err("ls | where siz > 1mb"), "error: no column `siz`\n  ls | where siz > 1mb\n             ^^^ not found\n  help: did you mean `size`?\n");
    assert_eq!(err("ls | where name > 1mb"), "error: can't compare string with filesize\n  ls | where name > 1mb\n                  ^ string > filesize\n");
    assert_eq!(err("lss"), "error: unknown command `lss`\n  lss\n  ^^^ not a built-in command or a program\n  help: did you mean `ls`?\n");
    assert_eq!(err("$nope"), "error: unknown variable `$nope`\n  $nope\n  ^^^^^ not defined\n  help: define it with `let nope = ...`\n");
    assert_eq!(err("ls | get 99"), "error: row 99 is past the end\n  ls | get 99\n           ^^ there are 9 rows\n");
    assert_eq!(err("ls | first x"), "error: expected a count, got x\n  ls | first x\n             ^ not a whole number of zero or more\n");
    assert_eq!(err("ls | where name"), "error: the condition must be true or false, but it is a string\n  ls | where name\n             ^^^^ not a condition\n  help: compare it with something, like `where size > 1mb`\n");
    assert_eq!(err("mv a"), "error: `mv` needs two paths\n  mv a\n      ^ missing two paths\n  help: usage: mv <from> <to>\n");
    assert_eq!(err("'x' | length"), "error: `length` counts rows, and the input is text\n  'x' | length\n        ^^^^^^\n  help: use `str length` for characters, or `lines | length` for lines\n");
    // An error stops the rest of the line, keeping what came before.
    let mut e = Engine::new();
    let mut h = host();
    let out = run(&mut e, &mut h, "echo first; lss; echo never");
    assert!(out.text.starts_with("first\nerror: unknown command `lss`"), "{}", out.text);
    assert!(!out.text.contains("\nnever\n"), "{}", out.text);
}
