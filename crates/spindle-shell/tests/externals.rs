// SPDX-License-Identifier: GPL-3.0-only
//! Programs and jobs through the resumable engine, step by step as a terminal drives it.
mod common;
use common::*;
use spindle_shell::{Engine, ExternalResult, JobState, Step, Value};

fn result(out: &str, code: i32) -> ExternalResult { ExternalResult { output: out.as_bytes().to_vec(), exit_code: code } }

#[test]
fn last_stage_program_is_not_captured() {
    let mut e = Engine::new();
    e.color = false;
    let mut h = host();
    let Step::RunExternal { print, request } = e.run("echo before; greet world", &mut h, 80) else { panic!() };
    assert_eq!(print, "before\n");
    assert_eq!(request.name, "greet");
    assert_eq!(request.args, vec!["world".to_string()]);
    assert_eq!(request.cwd, "/home");
    assert_eq!(request.input, None);
    assert!(!request.capture);
    assert!(e.is_waiting());
    // The terminal showed the output itself and resumes with nothing.
    let Step::Done(out) = e.resume(result("", 0), &mut h) else { panic!() };
    assert_eq!(out.text, "");
    assert_eq!(out.exit_code, 0);
    assert!(!e.is_waiting());
}

#[test]
fn program_output_feeds_structured_stages() {
    let mut e = Engine::new();
    e.color = false;
    let mut h = host();
    let Step::RunExternal { request, .. } = e.run("greet a b | lines | length", &mut h, 80) else { panic!() };
    assert!(request.capture);
    let Step::Done(out) = e.resume(result("hello a b\nfrom greet\n", 0), &mut h) else { panic!() };
    assert_eq!(out.value, Value::Int(2));
    assert_eq!(out.text, "2\n");
}

#[test]
fn structured_values_reach_programs_as_text() {
    let mut e = Engine::new();
    let mut h = host();
    let Step::RunExternal { request, .. } = e.run("ls | where type == dir | get name | ^upper", &mut h, 80) else { panic!() };
    assert_eq!(request.input.as_deref(), Some(&b"docs\nsrc\n"[..]));
    let Step::RunExternal { request, .. } = e.run("open people.csv | select name age | upper", &mut h, 80) else { panic!() };
    assert_eq!(String::from_utf8(request.input.unwrap()).unwrap(), "name   age\nAda     36\nLinus   28\nGrace   45\n");
    let Step::RunExternal { request, .. } = e.run("'raw' | upper", &mut h, 80) else { panic!() };
    assert_eq!(request.input.as_deref(), Some(&b"raw"[..]));
    // Lists spread into arguments; flags of programs pass through as words.
    let Step::RunExternal { request, .. } = e.run("upper -x --long [a b] 1kb", &mut h, 80) else { panic!() };
    assert_eq!(request.args, vec!["-x", "--long", "a", "b", "1000 B"]);
}

#[test]
fn two_programs_in_a_row() {
    let out = ok("greet x | upper | lines");
    assert_eq!(out, "╭───┬────────────╮\n│ 0 │ HELLO X    │\n│ 1 │ FROM GREET │\n╰───┴────────────╯\n");
    let mut e = Engine::new();
    let mut h = host();
    let Step::RunExternal { request: r1, .. } = e.run("greet x | upper", &mut h, 80) else { panic!() };
    assert!(r1.capture);
    let Step::RunExternal { request: r2, .. } = e.resume(result("hi\n", 0), &mut h) else { panic!() };
    assert_eq!(r2.input.as_deref(), Some(&b"hi\n"[..]));
    assert!(!r2.capture);
}

#[test]
fn binary_output_stays_binary_and_exit_codes_carry() {
    let mut e = Engine::new();
    let mut h = host();
    let _ = e.run("upper | describe", &mut h, 80);
    let Step::Done(out) = e.resume(ExternalResult { output: vec![0xff, 0xfe], exit_code: 0 }, &mut h) else { panic!() };
    assert_eq!(out.value, Value::string("binary"));
    let mut e = Engine::new();
    let o = run(&mut e, &mut h, "fail");
    assert_eq!(o.text, "it broke\n");
    assert_eq!(o.exit_code, 3);
}

#[test]
fn let_and_redirects_capture() {
    let mut e = Engine::new();
    let mut h = host();
    let o = run(&mut e, &mut h, "let g = (^greet); let h = greet me; $h | lines | first");
    assert!(o.error.is_some(), "programs are refused inside ( ): {}", o.text);
    assert!(o.text.contains("the program `greet` can't run inside ( )"), "{}", o.text);
    let o = run(&mut e, &mut h, "let h = greet me; $h | lines | first");
    assert_eq!(o.value, Value::string("hello me"));
    let o = run(&mut e, &mut h, "greet file > out.txt");
    assert!(o.error.is_none(), "{}", o.text);
    assert_eq!(h.file("/home/out.txt"), Some(&b"hello file\nfrom greet\n"[..]));
}

#[test]
fn unknown_and_forced_programs() {
    let mut e = Engine::new();
    let mut h = host();
    let o = run(&mut e, &mut h, "greeet");
    assert_eq!(o.text, "error: unknown command `greeet`\n  greeet\n  ^^^^^^ not a built-in command or a program\n  help: did you mean `greet`?\n");
    // `^` skips the check: the program runner decides.
    let Step::RunExternal { request, .. } = e.run("^ls -l", &mut h, 80) else { panic!() };
    assert_eq!((request.name.as_str(), request.args.clone()), ("ls", vec!["-l".to_string()]));
}

#[test]
fn background_jobs() {
    let mut e = Engine::new();
    let mut h = host();
    let o = run(&mut e, &mut h, "greet slow &");
    assert_eq!(o.text, "[1] greet\n");
    assert_eq!(o.value, Value::Int(1));
    let o = run(&mut e, &mut h, "'in' | upper &");
    assert_eq!(o.value, Value::Int(2));
    assert_eq!(ok_in(&mut e, &mut h, "jobs | get command"), Value::List(vec![Value::string("greet slow"), Value::string("upper")]));
    ok_in(&mut e, &mut h, "kill 2");
    assert_eq!(h.jobs[1].state, JobState::Done(-9));
    // fg waits through the terminal.
    let Step::WaitJob { job, capture, .. } = e.run("fg 1", &mut h, 80) else { panic!() };
    assert_eq!((job, capture), (1, false));
    assert_eq!(h.foregrounded, vec![1]);
    let Step::Done(o) = e.resume(result("", 0), &mut h) else { panic!() };
    assert!(o.error.is_none());
    // fg with no number takes the newest running job.
    let Step::WaitJob { job, .. } = e.run("fg", &mut h, 80) else { panic!() };
    assert_eq!(job, 1);
    e.cancel();
    let o = run(&mut e, &mut h, "fg 2");
    assert!(o.text.contains("can't bring job 2 to the front: it has already ended"), "{}", o.text);
    let o = run(&mut e, &mut h, "kill 9");
    assert!(o.text.contains("can't stop job 9: no such job"), "{}", o.text);
    let o = run(&mut e, &mut h, "ls &");
    assert!(o.text.contains("ends in a built-in command"), "{}", o.text);
}

fn ok_in(e: &mut Engine, h: &mut spindle_shell::MemHost, l: &str) -> Value {
    let o = run(e, h, l);
    assert!(o.error.is_none(), "{}", o.text);
    o.value
}

#[test]
fn a_new_line_drops_a_waiting_one() {
    let mut e = Engine::new();
    let mut h = host();
    let _ = e.run("greet; echo after", &mut h, 80);
    assert!(e.is_waiting());
    let Step::Done(o) = e.run("echo new", &mut h, 80) else { panic!() };
    assert_eq!(o.value, Value::string("new"));
    // Resuming with nothing waiting is harmless.
    assert!(matches!(e.resume(result("x", 0), &mut h), Step::Done(_)));
}
