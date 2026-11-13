// SPDX-License-Identifier: GPL-3.0-only
//! sexpkg: makes SexOS packages and store catalogs on a build machine.
//!
//!   sexpkg build KEY MANIFEST OUT ENTRY=FILE...
//!       Packs FILEs under entry names (`bin/PROGRAM`, `data/PATH`) with MANIFEST's
//!       `key=value` lines and signs the result with KEY.
//!   sexpkg public KEY            prints KEY's public half (what `pkg trust` takes)
//!   sexpkg verify PUB PACKAGE    checks a package against a public key, lists it
//!   sexpkg index KEY DIR         writes DIR/sexos-store.txt and .sig for DIR/*.sexpkg
//!
//! KEY is a file holding a 64-digit hex secret seed, "-" for SEXPKG_SECRET, or "test"
//! for the gates' public test key (trusted only after `pkg trust` with its public half).
use sex_pkg::{index, Manifest};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::exit;

/// The gates' key. Its secret is public on purpose.
const TEST_SEED: [u8; 32] = *b"SexOS package gates: test only!!";
fn die(msg: &str) -> ! { eprintln!("sexpkg: {msg}"); exit(1) }
fn read(p: &str) -> Vec<u8> { std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}"))) }
fn seed(key: &str) -> [u8; 32] {
    let text = match key {
        "test" => return TEST_SEED,
        "-" => std::env::var("SEXPKG_SECRET").unwrap_or_else(|_| die("SEXPKG_SECRET is not set")),
        p => String::from_utf8(read(p)).unwrap_or_else(|_| die("key file is not text")),
    };
    let line: String = text.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
    sex_pkg::unhex::<32>(line.trim().as_bytes()).unwrap_or_else(|| die("the key is not 64 hex digits"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["build", key, manifest, out, entries @ ..] if !entries.is_empty() => {
            let m = Manifest::parse(&String::from_utf8(read(manifest)).unwrap_or_else(|_| die("manifest is not UTF-8"))).unwrap_or_else(|e| die(e.why()));
            let data: Vec<(String, Vec<u8>)> = entries.iter().map(|e| {
                let (name, file) = e.split_once('=').unwrap_or_else(|| die("entries are NAME=FILE"));
                (name.to_string(), read(file))
            }).collect();
            let files: Vec<(&str, &[u8])> = data.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
            let pkg = sex_pkg::build(&m, &files, seed(key)).unwrap_or_else(|e| die(e.why()));
            std::fs::write(out, &pkg).unwrap_or_else(|e| die(&format!("{out}: {e}")));
            println!("{out}: {} {} ({} files, {} bytes)", m.name, m.version, files.len(), pkg.len());
        }
        ["public", key] => println!("{}", sex_pkg::hex(&sex_pkg::public_key(seed(key)))),
        ["verify", public, pkg] => {
            let k = sex_pkg::parse_keys(&read(public));
            let k = if k.is_empty() { vec![sex_pkg::unhex::<32>(public.as_bytes()).unwrap_or_else(|| die("PUB is neither a key file nor 64 hex digits"))] } else { k };
            let bytes = read(pkg);
            let (layout, key) = sex_pkg::verify(&bytes, &k).unwrap_or_else(|e| die(e.why()));
            print!("{}publisher={}\n", layout.manifest.to_text(), sex_pkg::hex(&key));
            for e in layout.entries { println!("  {:?} {} bytes", e.target, e.size); }
        }
        ["index", key, dir] => {
            let mut items = Vec::new();
            let mut names: Vec<_> = std::fs::read_dir(dir).unwrap_or_else(|e| die(&format!("{dir}: {e}"))).filter_map(|e| e.ok()).map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "sexpkg")).collect();
            names.sort();
            let s = seed(key);
            for p in names {
                let bytes = std::fs::read(&p).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
                let (layout, _) = sex_pkg::verify(&bytes, &[sex_pkg::public_key(s)]).unwrap_or_else(|e| die(&format!("{}: {}", p.display(), e.why())));
                let m = layout.manifest;
                items.push(index::Item { name: m.name, version: m.version, file: p.file_name().unwrap().to_string_lossy().into(), bytes: bytes.len() as u64, sha256: Sha256::digest(&bytes).into(), grants: m.grants, summary: m.summary });
            }
            let mut text = String::from("# SexOS store\n");
            for i in &items { text.push_str(&index::line(i)); }
            let dir = Path::new(dir);
            std::fs::write(dir.join(index::INDEX_FILE), &text).unwrap_or_else(|e| die(&e.to_string()));
            std::fs::write(dir.join(index::SIG_FILE), sex_pkg::hex(&index::sign(text.as_bytes(), s)) + "\n").unwrap_or_else(|e| die(&e.to_string()));
            println!("{} packages in {}", items.len(), dir.join(index::INDEX_FILE).display());
        }
        _ => die("usage: sexpkg (build KEY MANIFEST OUT ENTRY=FILE... | public KEY | verify PUB PACKAGE | index KEY DIR)"),
    }
}
