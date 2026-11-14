// SPDX-License-Identifier: GPL-3.0-only
//! Collects every protocol operation `sex-pdx` names (`pub const OP_*` with a literal
//! value), so the fuzzer's protocol mode sends real operation numbers to every service
//! and covers a protocol as soon as one is added.
use std::{env, fs, path::Path};
fn main() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/sex-pdx/src");
    println!("cargo:rerun-if-changed={}", src.display());
    let mut files: Vec<_> = fs::read_dir(&src).unwrap().filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x == "rs")).collect();
    files.sort();
    let mut ops: Vec<(String, u64)> = Vec::new();
    for f in &files {
        let module = f.file_stem().unwrap().to_string_lossy().to_string();
        println!("cargo:rerun-if-changed={}", f.display());
        for line in fs::read_to_string(f).unwrap().lines() {
            let l = line.trim();
            let Some(rest) = l.strip_prefix("pub const ") else { continue };
            let Some((name, rest)) = rest.split_once(':') else { continue };
            let name = name.trim();
            if !(name == "OP" || name.starts_with("OP_")) { continue; }
            let Some((ty, value)) = rest.split_once('=') else { continue };
            if !matches!(ty.trim(), "u64" | "u32" | "u16" | "u8") { continue; }
            let v = value.trim().trim_end_matches(';').replace('_', "");
            let parsed = if let Some(h) = v.strip_prefix("0x") { u64::from_str_radix(h, 16).ok() } else { v.parse().ok() };
            if let Some(n) = parsed { ops.push((format!("{}::{}", module, name), n)); }
        }
    }
    let mut out = String::from("/// Every literal `OP_*` in sex-pdx: (module::name, value).\npub const OPS: &[(&str, u64)] = &[\n");
    for (n, v) in &ops { out += &format!("    ({:?}, {:#x}),\n", n, v); }
    out += "];\n";
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("ops.rs"), out).unwrap();
}
