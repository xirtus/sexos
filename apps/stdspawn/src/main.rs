// SPDX-License-Identifier: GPL-3.0-only
//! A test program for `std::process` on SexOS (crates/sex-std/src/process.rs), and the
//! child it starts. Run with no arguments it starts copies of itself and checks what
//! comes back; the last line is the score scripts/process_gate.py reads.
//!
//! As a child: `stdspawn exit N` ends with status N, `stdspawn sleep S` runs S seconds
//! and ends with 0, `stdspawn args A B C` ends with 0 if its arguments are exactly
//! `a b`, `` and `c"d\e` (quoting survives the trip), else 1.
use std::io::ErrorKind;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sex_std::process::CommandExt;

fn child(args: &[String]) -> Option<i32> {
    match args.first().map(String::as_str)? {
        "exit" => Some(args.get(1).and_then(|n| n.parse().ok()).unwrap_or(1)),
        "sleep" => {
            println!("sleeping");
            std::thread::sleep(Duration::from_secs(args.get(1).and_then(|n| n.parse().ok()).unwrap_or(1)));
            Some(0)
        }
        "args" => Some(if args[1..] == ["a b", "", "c\"d\\e"] { 0 } else { 1 }),
        _ => None,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = child(&args) {
        sex_std::record(&format!("[stdspawn.child] args={args:?} exit={code}"));
        std::process::exit(code);
    }
    // `run +net stdspawn net`: a run that has the network may hand it on.
    if args.first().map(String::as_str) == Some("net") {
        let status = Command::new("stdspawn").args(["exit", "0"]).network(true).status();
        let ok = status.as_ref().is_ok_and(|s| s.success());
        println!("{} network handed on: {status:?}", if ok { "ok  " } else { "FAIL" });
        sex_std::record(&format!("[stdspawn.net] {} {status:?}", if ok { "ok" } else { "fail" }));
        return;
    }
    println!("std::process on SexOS");
    let (mut passed, mut total) = (0, 0);
    let mut check = |name: &str, ok: bool, what: String| {
        total += 1;
        if ok { passed += 1; }
        println!("{} {name}: {what}", if ok { "ok  " } else { "FAIL" });
        sex_std::record(&format!("[stdspawn.check] {} {name}: {what}", if ok { "ok" } else { "FAIL" }));
    };

    let me = std::process::id();
    check("process::id", me > 0, format!("{me}"));

    let status = Command::new("stdspawn").args(["exit", "7"]).status();
    check("status of exit 7", status.as_ref().is_ok_and(|s| s.code() == Some(7) && !s.success()), format!("{status:?}"));

    let status = Command::new("/bin/stdspawn").args(["exit", "0"]).status();
    check("/bin/NAME and success", status.as_ref().is_ok_and(|s| s.success() && s.code() == Some(0)), format!("{status:?}"));

    let status = Command::new("stdspawn").args(["args", "a b", "", "c\"d\\e"]).status();
    check("arguments with blanks and quotes", status.as_ref().is_ok_and(|s| s.success()), format!("{status:?}"));

    // A short run: not ended at once, then ended with 0.
    let short = Command::new("stdspawn").args(["sleep", "2"]).spawn();
    let ok = match short {
        Ok(mut c) => {
            let early = c.try_wait();
            let id = c.id();
            let end = c.wait();
            let after = c.try_wait();
            let what = format!("id={id} early={early:?} end={end:?} after={after:?}");
            let good = id > 0 && id != me && matches!(early, Ok(None)) && end.as_ref().is_ok_and(|s| s.success()) && after.as_ref().is_ok_and(|s| s.is_some_and(|s| s.success()));
            (good, what)
        }
        Err(e) => (false, format!("{e:?}")),
    };
    check("try_wait before the end, wait after", ok.0, ok.1);

    // A long run, ended by its starter: no status, so no code and no success.
    let long = Command::new("stdspawn").args(["sleep", "600"]).spawn();
    let ok = match long {
        Ok(mut c) => {
            let running = c.try_wait();
            let killed = c.kill();
            let t = Instant::now();
            let end = c.wait();
            let what = format!("running={running:?} kill={killed:?} end={end:?} in {:?}", t.elapsed());
            (matches!(running, Ok(None)) && killed.is_ok() && end.as_ref().is_ok_and(|s| s.code().is_none() && !s.success()) && t.elapsed() < Duration::from_secs(60), what)
        }
        Err(e) => (false, format!("{e:?}")),
    };
    check("kill", ok.0, ok.1);

    let missing = Command::new("no-such-program").status();
    check("missing program", missing.as_ref().is_err_and(|e| e.kind() == ErrorKind::NotFound), format!("{missing:?}"));
    let bad = Command::new("../escape").status();
    check("a path outside the program store", bad.as_ref().is_err_and(|e| e.kind() == ErrorKind::NotFound), format!("{bad:?}"));

    // This run was started without the network, so it cannot hand it on.
    let net = Command::new("stdspawn").args(["exit", "0"]).network(true).status();
    check("network refused", net.as_ref().is_err_and(|e| e.kind() == ErrorKind::PermissionDenied), format!("{net:?}"));

    let piped = Command::new("stdspawn").args(["exit", "0"]).stdout(Stdio::piped()).spawn().map(|_| ());
    check("no pipes", piped.as_ref().is_err_and(|e| e.kind() == ErrorKind::Unsupported), format!("{piped:?}"));
    let output = Command::new("stdspawn").args(["exit", "0"]).output();
    check("output() says unsupported", output.as_ref().is_err_and(|e| e.kind() == ErrorKind::Unsupported), format!("{:?}", output.map(|o| o.status)));
    let quiet = Command::new("stdspawn").args(["exit", "4"]).stdout(Stdio::null()).stderr(Stdio::null()).output();
    check("output() with null streams", quiet.as_ref().is_ok_and(|o| o.status.code() == Some(4) && o.stdout.is_empty()), format!("{:?}", quiet.map(|o| o.status)));

    // A run with its own window (it prints nothing, so it never waits to be closed).
    let windowed = Command::new("stdspawn").args(["exit", "3"]).windowed(true).status();
    check("windowed run", windowed.as_ref().is_ok_and(|s| s.code() == Some(3)), format!("{windowed:?}"));

    println!("passed {passed}/{total}");
    sex_std::record(&format!("[stdspawn.{}] passed={passed}/{total}", if passed == total { "ok" } else { "fail" }));
}
