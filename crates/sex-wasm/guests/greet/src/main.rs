// SPDX-License-Identifier: GPL-3.0-only
//! A plain Rust program built for wasm32-wasip1, run by sex-wasm in tests.
//! Rebuild: cargo build --release --target wasm32-wasip1, then copy
//! target/wasm32-wasip1/release/greet.wasm to ../../tests/greet.wasm.
use std::collections::HashMap;
use std::io::BufRead;
fn main() {
    let who = std::env::args().nth(1).unwrap_or_else(|| "world".into());
    println!("Hello, {who}, from Rust on Wasm.");
    println!("Type words; an empty line ends.");
    let mut counts: HashMap<String, usize> = HashMap::new();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        if line.is_empty() { break; }
        for w in line.split_whitespace() { *counts.entry(w.to_string()).or_default() += 1; }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort();
    for (w, n) in v { println!("{w}: {n}"); }
    let t = std::time::Instant::now();
    let _ = t.elapsed();
    std::process::exit(5);
}
