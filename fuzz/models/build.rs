// SPDX-License-Identifier: GPL-3.0-only
//! Makes loom-checkable copies of the kernel's lock-free queues. Loom needs its own
//! atomics, which have no `const fn new`, so each copy swaps `core::sync::atomic` for
//! `crate::shim::atomic`, drops `const` from `new`, builds `[const { .. }; N]` arrays
//! with `core::array::from_fn`, and leaves out the in-file unit tests. The ring's
//! payload `UnsafeCell` becomes a loom-tracked cell. Every rewrite
//! must apply: if the kernel source drifts, this fails loudly instead of checking a
//! different algorithm.
use std::{env, fs, path::PathBuf};

fn rewrite(src: &str, name: &str, extra: &[(&str, &str)]) -> String {
    let mut out = src.split("#[cfg(test)]").next().unwrap().to_string();
    let must = |out: &mut String, from: &str, to: &str| {
        assert!(out.contains(from), "{name}: kernel source changed, `{from}` not found; update fuzz/models/build.rs");
        *out = out.replace(from, to);
    };
    must(&mut out, "core::sync::atomic", "crate::shim::atomic");
    must(&mut out, "pub const fn new()", "pub fn new()");
    for (from, to) in extra { must(&mut out, from, to); }
    let mut arrays = 0;
    while let Some(start) = out.find("[const {") {
        let end = out[start..].find("}; N]").map(|e| start + e).unwrap_or_else(|| panic!("{name}: unterminated [const {{ .. }}; N]"));
        let body = out[start + "[const {".len()..end].to_string();
        out.replace_range(start..end + "}; N]".len(), &format!("core::array::from_fn(|_| {{{body}}})"));
        arrays += 1;
    }
    assert!(arrays > 0, "{name}: no [const {{ .. }}; N] array found; update fuzz/models/build.rs");
    // Inner doc comments cannot follow `mod x {` once included.
    out.lines().map(|l| l.replacen("//!", "//", 1)).collect::<Vec<_>>().join("\n")
}

fn main() {
    println!("cargo:rustc-check-cfg=cfg(loom)");
    println!("cargo:rustc-check-cfg=cfg(kani)");
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    // The ring's payload cell becomes a loom-tracked cell, so a producer write that
    // is not ordered before the consumer's read is reported as a causality violation.
    let ring_cell: &[(&str, &str)] = &[("use core::cell::UnsafeCell;", "use crate::shim::cell::UnsafeCell;")];
    for (file, name, extra) in [("kernel/src/scheduler_queue.rs", "scheduler_queue", &[][..]), ("kernel/src/ipc_ring.rs", "ipc_ring", ring_cell)] {
        let path = root.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        let src = fs::read_to_string(&path).unwrap();
        fs::write(out.join(format!("loom_{name}.rs")), rewrite(&src, name, extra)).unwrap();
    }
}
