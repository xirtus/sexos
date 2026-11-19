// SPDX-License-Identifier: GPL-3.0-only
//! A test program for `std::process` on SexOS (crates/sex-std/src/process.rs), and the
//! child it starts. Run with no arguments it starts copies of itself and checks what
//! comes back; the last line is the score scripts/process_gate.py reads.
//!
//! As a child: `stdspawn exit N` ends with status N, `stdspawn sleep S` runs S seconds
//! and ends with 0, `stdspawn args A B C` ends with 0 if its arguments are exactly
//! `a b`, `` and `c"d\e` (quoting survives the trip), else 1. For pipes, environment
//! and working folder: `cat` copies stdin to stdout and says how much on stderr;
//! `streams` writes a line to each and ends with 5; `big N M` writes N bytes to stdout
//! and M to stderr, interleaved; `env K` prints K's value; `envis K V` ends with 0 when
//! K is V; `cwd` prints its working folder and the file `note.txt` there.
use std::io::{ErrorKind, Read, Write};
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
        "cat" => {
            let mut all = Vec::new();
            let r = std::io::stdin().read_to_end(&mut all);
            std::io::stdout().write_all(&all).ok()?;
            eprintln!("cat read {} bytes ({r:?})", all.len());
            Some(0)
        }
        "streams" => {
            println!("to stdout");
            eprintln!("to stderr");
            Some(5)
        }
        "big" => {
            let n: usize = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(0);
            let m: usize = args.get(2).and_then(|n| n.parse().ok()).unwrap_or(0);
            let (mut out, mut err) = (std::io::stdout().lock(), std::io::stderr());
            let (mut o, mut e) = (0, 0);
            while o < n || e < m {
                let a = (n - o).min(4000);
                if a > 0 { out.write_all(&pattern(o, a)).ok()?; o += a; }
                let b = (m - e).min(1000);
                if b > 0 { err.write_all(&pattern(e, b)).ok()?; e += b; }
            }
            out.flush().ok()?;
            Some(0)
        }
        "env" => {
            println!("{}", std::env::var(args.get(1)?).unwrap_or_else(|_| "<unset>".into()));
            Some(0)
        }
        "envis" => Some(if std::env::var(args.get(1)?).ok().as_deref() == args.get(2).map(String::as_str) { 0 } else { 1 }),
        "cwd" => {
            let here = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|e| format!("{e:?}"));
            let note = std::fs::read_to_string("note.txt").unwrap_or_else(|e| format!("{e:?}"));
            println!("{here}|{note}");
            Some(0)
        }
        _ => None,
    }
}

/// Bytes `at..at + n` of a pattern a reader can check.
fn pattern(at: usize, n: usize) -> Vec<u8> { (at..at + n).map(|i| b'a' + (i % 23) as u8).collect() }

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

    // Pipes (Files' relay slots): output() takes both streams and the status.
    let out = Command::new("stdspawn").arg("streams").output();
    check("output() takes stdout, stderr and status", out.as_ref().is_ok_and(|o| o.status.code() == Some(5) && o.stdout == b"to stdout\n" && o.stderr == b"to stderr\n"),
          format!("{:?}", out.as_ref().map(|o| (o.status, String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned()))));

    // stdin and stdout piped, more than the 4 KiB input ring at once.
    let sent = pattern(7, 20_000);
    let piped = Command::new("stdspawn").arg("cat").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let ok = match piped {
        Ok(mut c) => {
            let wrote = c.stdin.take().map(|mut i| i.write_all(&sent));
            let mut back = Vec::new();
            let read = c.stdout.take().map(|mut o| o.read_to_end(&mut back));
            let mut said = String::new();
            let _ = c.stderr.take().map(|mut e| e.read_to_string(&mut said));
            let end = c.wait();
            (back == sent && end.as_ref().is_ok_and(|s| s.success()) && said.trim() == "cat read 20000 bytes (Ok(20000))",
             format!("wrote={wrote:?} read={read:?} same={} said={said:?} end={end:?}", back == sent))
        }
        Err(e) => (false, format!("{e:?}")),
    };
    check("stdin and stdout piped", ok.0, ok.1);

    // Far more than the rings hold, on both streams at once: neither side is stuck.
    let big = Command::new("stdspawn").args(["big", "300000", "70000"]).output();
    check("output() of more than the rings hold", big.as_ref().is_ok_and(|o| o.status.success() && o.stdout == pattern(0, 300_000) && o.stderr == pattern(0, 70_000)),
          format!("{:?}", big.as_ref().map(|o| (o.status, o.stdout.len(), o.stderr.len()))));

    // A reader that goes away: the child's writes vanish and it still ends.
    let gone = Command::new("stdspawn").args(["big", "500000", "0"]).stdout(Stdio::piped()).spawn().and_then(|mut c| { drop(c.stdout.take()); c.wait() });
    check("stdout dropped early", gone.as_ref().is_ok_and(|s| s.success()), format!("{gone:?}"));

    // Writing to a child that ended is a broken pipe. (wait() closes stdin, so take it first.)
    let ended = Command::new("stdspawn").args(["exit", "0"]).stdin(Stdio::piped()).spawn().map(|mut c| { let stdin = c.stdin.take(); let _ = c.wait(); stdin.map(|mut i| i.write_all(b"late")) });
    check("stdin of an ended child", matches!(ended, Ok(Some(Err(ref e))) if e.kind() == ErrorKind::BrokenPipe), format!("{ended:?}"));

    // Null stdin is at its end at once.
    let empty = Command::new("stdspawn").arg("cat").stdin(Stdio::null()).output();
    check("null stdin", empty.as_ref().is_ok_and(|o| o.status.success() && o.stdout.is_empty()), format!("{:?}", empty.as_ref().map(|o| o.status)));

    // A child's pipe handed to another command is not supported.
    let handed = Command::new("stdspawn").arg("streams").stdout(Stdio::piped()).spawn().and_then(|mut c| {
        let r = Command::new("stdspawn").arg("cat").stdin(Stdio::from(c.stdout.take().unwrap())).status();
        let _ = c.wait();
        r
    });
    check("a child's pipe to another child", handed.as_ref().is_err_and(|e| e.kind() == ErrorKind::Unsupported), format!("{handed:?}"));

    // The environment: the command's own, and this run's handed on.
    let env = Command::new("stdspawn").args(["env", "SEXOS_TEST"]).env("SEXOS_TEST", "bar baz").output();
    check("env", env.as_ref().is_ok_and(|o| o.stdout == b"bar baz\n"), format!("{:?}", env.as_ref().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())));
    // SAFETY: no other thread reads the environment now.
    unsafe { std::env::set_var("SEXOS_INHERITED", "yes") };
    let inherited = Command::new("stdspawn").args(["envis", "SEXOS_INHERITED", "yes"]).status();
    let cleared = Command::new("stdspawn").args(["envis", "SEXOS_INHERITED", "yes"]).env_clear().status();
    check("env inherited and cleared", inherited.as_ref().is_ok_and(|s| s.success()) && cleared.as_ref().is_ok_and(|s| s.code() == Some(1)), format!("{inherited:?} {cleared:?}"));

    // The working folder: one of this program's own, handed to the child as a grant.
    let made = std::fs::create_dir_all("work").and_then(|_| std::fs::write("work/note.txt", "from the parent"));
    let cwd = Command::new("stdspawn").arg("cwd").current_dir("work").output();
    let want = "@/data/stdspawn/work|from the parent\n";
    check("current_dir", cwd.as_ref().is_ok_and(|o| o.stdout == want.as_bytes()), format!("made={made:?} {:?}", cwd.as_ref().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())));
    let outside = Command::new("stdspawn").arg("cwd").current_dir("@/data/someone-else").status();
    let missing = Command::new("stdspawn").arg("cwd").current_dir("no-such-folder").status();
    let file = Command::new("stdspawn").arg("cwd").current_dir("work/note.txt").status();
    // Grants are space-separated words: a folder with ` @/ ` in it must not become one.
    let sneaky = std::fs::create_dir_all("x @/ y").and_then(|_| Command::new("stdspawn").arg("cwd").current_dir("x @/ y").status());
    let refused = |r: &std::io::Result<std::process::ExitStatus>| r.as_ref().is_err_and(|e| e.kind() == ErrorKind::NotFound);
    check("current_dir not held, missing, a file, or with a space", refused(&outside) && refused(&missing) && refused(&file) && refused(&sneaky),
          format!("{outside:?} {missing:?} {file:?} {sneaky:?}"));
    let here = std::env::current_dir();
    let moved = std::env::set_current_dir("work").and_then(|_| std::fs::read_to_string("note.txt"));
    // A child's relative folder starts from this run's working folder.
    let _ = std::fs::create_dir_all("sub").and_then(|_| std::fs::write("sub/note.txt", "in sub"));
    let nested = Command::new("stdspawn").arg("cwd").current_dir("sub").output();
    let back = std::env::set_current_dir("/");
    check("current_dir from the working folder", nested.as_ref().is_ok_and(|o| o.stdout == b"@/data/stdspawn/work/sub|in sub\n"),
          format!("{:?}", nested.as_ref().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())));
    check("set_current_dir", here.as_ref().is_ok_and(|p| p.as_os_str() == "/") && moved.as_deref().is_ok_and(|n| n == "from the parent") && back.is_ok(),
          format!("here={here:?} moved={moved:?} back={back:?}"));

    let quiet = Command::new("stdspawn").args(["exit", "4"]).stdout(Stdio::null()).stderr(Stdio::null()).output();
    check("output() with null streams", quiet.as_ref().is_ok_and(|o| o.status.code() == Some(4) && o.stdout.is_empty()), format!("{:?}", quiet.map(|o| o.status)));

    // A run with its own window (it prints nothing, so it never waits to be closed).
    let windowed = Command::new("stdspawn").args(["exit", "3"]).windowed(true).status();
    check("windowed run", windowed.as_ref().is_ok_and(|s| s.code() == Some(3)), format!("{windowed:?}"));

    println!("passed {passed}/{total}");
    sex_std::record(&format!("[stdspawn.{}] passed={passed}/{total}", if passed == total { "ok" } else { "fail" }));
}
