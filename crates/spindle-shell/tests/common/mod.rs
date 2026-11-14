// SPDX-License-Identifier: GPL-3.0-only
//! The fake system the tests run against.
#![allow(dead_code)]

use spindle_shell::{Engine, MemHost, Output, Record, Value};

/// 2026-11-01T12:00:00Z
pub const T0: i64 = 1_793_534_400;

fn upper(args: &[String], input: Option<&[u8]>) -> (Vec<u8>, i32) {
    let mut out = input.map(|i| i.to_ascii_uppercase()).unwrap_or_default();
    for a in args { out.extend_from_slice(a.to_uppercase().as_bytes()); out.push(b'\n'); }
    (out, 0)
}

fn greet(args: &[String], _: Option<&[u8]>) -> (Vec<u8>, i32) {
    (format!("hello {}\nfrom greet\n", args.join(" ")).into_bytes(), 0)
}

fn fail(_: &[String], _: Option<&[u8]>) -> (Vec<u8>, i32) { (b"it broke\n".to_vec(), 3) }

/// /home with files of known sizes, two programs, a job, a process and a sys record.
pub fn host() -> MemHost {
    let mut h = MemHost::new();
    h.now = T0 + 86_400;
    h.add_dir("/home/src");
    h.add_dir("/home/docs");
    h.add_sized("/home/video.mkv", 734_003_200, T0);
    h.add_sized("/home/archive.tar", 2_500_000, T0 + 60);
    h.add_sized("/home/disk.img", 3 * 1024 * 1024 * 1024, T0 + 3600);
    h.add_file("/home/notes.txt", b"buy milk\ncall mum\nfix the build\n", T0);
    h.add_file("/home/.hidden", b"secret", T0);
    h.add_file("/home/data.json", br#"{"name": "spindle", "version": 3, "tags": ["shell", "tables"], "items": [{"title": "a", "done": true, "n": 1}, {"title": "b", "done": false, "n": 2}]}"#, T0);
    h.add_file("/home/people.csv", b"name,age,city\nAda,36,London\nLinus,28,\"Helsinki, FI\"\nGrace,45,Arlington\n", T0);
    h.add_file("/home/config.toml", b"# settings\ntitle = \"SexOS\"\n[display]\nwidth = 1920\nscale = 1.5\nvsync = true\n[[user]]\nname = \"ada\"\n[[user]]\nname = \"bob\"\n", T0);
    h.add_file("/home/src/main.rs", b"fn main() {}\n", T0);
    h.add_file("/home/src/lib.rs", b"//! lib\n", T0);
    h.add_file("/home/src/readme.md", b"# hi\n", T0);
    h.cwd = String::from("/home");
    h.home = String::from("/home");
    h.add_program("upper", upper);
    h.add_program("greet", greet);
    h.add_program("fail", fail);
    h.sys = Value::Record(Record::new().with("host", Value::string("sexbox")).with("cpus", Value::Int(4)).with("memory", Value::Filesize(8 << 30)));
    h.processes.push(spindle_shell::ProcessInfo { pid: 7, name: "silk-shell".into(), state: "running".into(), memory: Some(12 << 20) });
    h
}

/// Runs a line to the end without colour, 80 cells wide.
pub fn run(e: &mut Engine, h: &mut MemHost, line: &str) -> Output { run_w(e, h, line, 80) }

pub fn run_w(e: &mut Engine, h: &mut MemHost, line: &str, cols: usize) -> Output {
    e.color = false;
    // The engine hands programs back as requests; run them with the fixture's programs.
    let programs = host().programs;
    e.run_sync(line, h, cols, &mut |_, req| {
        let mut runner = MemHost::new();
        runner.programs = programs.clone();
        runner.run(req)
    })
}

/// The text of a line that must succeed.
pub fn ok(line: &str) -> String {
    let mut e = Engine::new();
    let mut h = host();
    let out = run(&mut e, &mut h, line);
    assert!(out.error.is_none(), "`{}` failed:\n{}", line, out.text);
    out.text
}

/// The value of a line that must succeed.
pub fn val(line: &str) -> Value {
    let mut e = Engine::new();
    let mut h = host();
    let out = run(&mut e, &mut h, line);
    assert!(out.error.is_none(), "`{}` failed:\n{}", line, out.text);
    out.value
}

/// The rendered error of a line that must fail.
pub fn err(line: &str) -> String {
    let mut e = Engine::new();
    let mut h = host();
    let out = run(&mut e, &mut h, line);
    assert!(out.error.is_some(), "`{}` should fail but gave:\n{}", line, out.text);
    out.text
}
