// SPDX-License-Identifier: GPL-3.0-only
//! An ordinary Rust program: nothing here knows it runs on SexOS. It checks the parts
//! of `std` a program leans on and prints what it found; `run stdhello a "b c"` shows
//! its arguments too. The last line is the score the program gate reads.
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use sex_std as _;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `run stdhello vt`: the console as a terminal. Colours, a cursor move, and a cursor
    // position report whose answer arrives on standard input, as from any terminal.
    if args.get(1).map(String::as_str) == Some("vt") {
        print!("\x1b[2J\x1b[H\x1b[1;31mred\x1b[0m \x1b[42mgreen\x1b[0m\x1b[3;5Hthird row\x1b[6n");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut reply = [0u8; 32];
        let n = std::io::Read::read(&mut std::io::stdin(), &mut reply).unwrap_or(0);
        let reply = String::from_utf8_lossy(&reply[..n]).replace('\x1b', "ESC");
        println!("\n[stdhello.vt] reply={reply}");
        return;
    }
    println!("Hello from Rust std on SexOS.");
    println!("arguments: {:?}", &args[1..]);
    let mut passed = 0;
    let mut total = 0;
    let mut check = |name: &str, ok: bool| {
        total += 1;
        if ok { passed += 1; }
        println!("{} {name}", if ok { "ok  " } else { "FAIL" });
    };

    let mut words = HashMap::new();
    for w in "the quick brown fox jumps over the lazy dog the end".split(' ') { *words.entry(w).or_insert(0) += 1; }
    check("HashMap and String", words["the"] == 3 && format!("{:.3}", std::f64::consts::PI) == "3.142");

    let path = "stdhello-check.txt";
    let wrote = std::fs::write(path, b"line one\nline two\n").is_ok();
    let read = std::fs::read_to_string(path).unwrap_or_default();
    check("fs::write then read_to_string", wrote && read == "line one\nline two\n");

    let appended = std::fs::OpenOptions::new().append(true).open(path).and_then(|mut f| f.write_all(b"three\n")).is_ok();
    let mut back = String::new();
    let _ = std::fs::File::open(path).and_then(|mut f| f.read_to_string(&mut back));
    check("append and File::read_to_string", appended && back.ends_with("two\nthree\n"));

    let _ = std::fs::create_dir("stdhello-dir");
    let _ = std::fs::write("stdhello-dir/inner.txt", b"x");
    let listed: Vec<String> = std::fs::read_dir("stdhello-dir").map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    check("create_dir and read_dir", listed == ["inner.txt"]);
    let meta = std::fs::metadata(path);
    check("metadata", meta.as_ref().is_ok_and(|m| m.is_file() && m.len() == 24));
    let removed = std::fs::remove_file("stdhello-dir/inner.txt").is_ok() && std::fs::remove_dir("stdhello-dir").is_ok() && std::fs::remove_file(path).is_ok();
    check("remove_file and remove_dir", removed && std::fs::metadata(path).is_err());
    check("no way out of the folder", std::fs::read("../../anything").is_err());

    let counter = Arc::new(Mutex::new(0u64));
    let handles: Vec<_> = (0..4).map(|i| {
        let counter = Arc::clone(&counter);
        std::thread::spawn(move || { for _ in 0..1000 { *counter.lock().unwrap() += 1; } i })
    }).collect();
    let ids: u32 = handles.into_iter().map(|h| h.join().unwrap()).sum();
    check("threads, Arc and Mutex", *counter.lock().unwrap() == 4000 && ids == 6);

    let (tx, rx) = mpsc::channel();
    let producer = std::thread::spawn(move || { for i in 0..100u32 { tx.send(i).unwrap(); } });
    let sum: u32 = rx.iter().sum();
    producer.join().unwrap();
    check("mpsc channel", sum == 4950);

    let start = Instant::now();
    std::thread::sleep(Duration::from_millis(50));
    let slept = start.elapsed();
    check("Instant and sleep", slept >= Duration::from_millis(50) && slept < Duration::from_secs(5));
    check("SystemTime after 2026", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).is_ok_and(|d| d.as_secs() > 1_767_225_600));

    let caught = std::thread::spawn(|| -> u32 { "7".parse::<u32>().unwrap() * 6 }).join().ok();
    check("thread result", caught == Some(42));
    check("available_parallelism", std::thread::available_parallelism().is_ok_and(|n| n.get() >= 1));
    let mut sorted: Vec<u64> = (0..10_000u64).map(|i| (i * 7919) % 10_007).collect();
    sorted.sort_unstable();
    check("sort", sorted.windows(2).all(|w| w[0] <= w[1]));

    println!("passed {passed}/{total}");
    sex_std::record(&format!("[stdhello.{}] passed={passed}/{total}", if passed == total { "ok" } else { "fail" }));
}
