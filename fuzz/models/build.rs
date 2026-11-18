// SPDX-License-Identifier: GPL-3.0-only
//! Makes loom-checkable copies of the kernel's lock-free queues. Loom needs its own
//! atomics, which have no `const fn new`, so each copy swaps `core::sync::atomic` for
//! `crate::shim::atomic`, drops `const` from `new`, builds `[const { .. }; N]` arrays
//! with `core::array::from_fn`, and leaves out the in-file unit tests. The ring's
//! payload `UnsafeCell` becomes a loom-tracked cell. Every rewrite
//! must apply: if the kernel source drifts, this fails loudly instead of checking a
//! different algorithm.
use std::{env, fs, path::PathBuf};

fn rewrite(src: &str, name: &str, extra: &[(&str, &str)], arrays_expected: bool) -> String {
    let mut out = src.split("#[cfg(test)]").next().unwrap().to_string();
    let must = |out: &mut String, from: &str, to: &str| {
        assert!(out.contains(from), "{name}: kernel source changed, `{from}` not found; update fuzz/models/build.rs");
        *out = out.replace(from, to);
    };
    must(&mut out, "core::sync::atomic", "crate::shim::atomic");
    for (from, to) in extra { must(&mut out, from, to); }
    // Spin loops must yield to loom, or a spinning thread starves the one it waits on.
    out = out.replace("core::hint::spin_loop()", "crate::shim::spin_loop()");
    out = out.replace("pub const fn new()", "pub fn new()");
    let mut arrays = 0;
    while let Some(start) = out.find("[const {") {
        let close = out[start..].find("}; ").map(|e| start + e).unwrap_or_else(|| panic!("{name}: unterminated [const {{ .. }}; N]"));
        let end = out[close..].find(']').map(|e| close + e).unwrap();
        let body = out[start + "[const {".len()..close].to_string();
        out.replace_range(start..end + 1, &format!("core::array::from_fn(|_| {{{body}}})"));
        arrays += 1;
    }
    assert!(!arrays_expected || arrays > 0, "{name}: no [const {{ .. }}; N] array found; update fuzz/models/build.rs");
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
    // IPC v2: the endpoint queue's cell is loom-tracked too. The notification copy
    // stops before IrqRoutes, whose `[EMPTY; LINES]` needs a const atomic.
    let endpoint: &[(&str, &str)] = &[("use core::cell::UnsafeCell;", "use crate::shim::cell::UnsafeCell;"),
                                      ("core::hint::spin_loop();", "crate::shim::spin_loop();")];
    let notification: &[(&str, &str)] = &[("#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub enum BindError", "#[cfg(any())]\nmod irq_routes_left_out {\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub enum BindError")];
    // Authority v2: tables shrunk to a few slots and objects so loom can explore
    // them; the global OBJECTS static (no const atomics under loom) is left out and
    // a loom constructor replaces the zeroed-heap one.
    let table: &[(&str, &str)] = &[("use crate::capability::", "use super::capability::"),
                                   ("pub const MAX_OBJECTS: usize = 4096;", "pub const MAX_OBJECTS: usize = 4;"),
                                   ("pub const MAX_INCARNATIONS: usize = 1024;", "pub const MAX_INCARNATIONS: usize = 2;"),
                                   ("pub const SLOT_COUNT: usize = 1024;", "pub const SLOT_COUNT: usize = 4;"),
                                   ("pub static OBJECTS: Generations = Generations::new();", ""),
                                   ("core::hint::spin_loop();", "crate::shim::spin_loop();")];
    for (file, name, extra, arrays) in [("kernel/src/scheduler_queue.rs", "scheduler_queue", &[][..], true),
                                        ("kernel/src/ipc_ring.rs", "ipc_ring", ring_cell, true),
                                        ("kernel/src/ipc_v2/endpoint.rs", "endpoint", endpoint, false),
                                        ("kernel/src/ipc_v2/notification.rs", "notification", notification, false),
                                        ("kernel/src/capability_table.rs", "capability_table", table, true)] {
        let path = root.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        let src = fs::read_to_string(&path).unwrap();
        let mut text = rewrite(&src, name, extra, arrays);
        if name == "notification" { text.push_str("\n}\n"); }
        if name == "capability_table" { text.push_str(LOOM_TABLE); }
        fs::write(out.join(format!("loom_{name}.rs")), text).unwrap();
    }
}

/// Loom atomics cannot be zero bytes from the heap, so the loom copy builds its table
/// field by field: the same empty, unsealed state `new_boxed` makes.
const LOOM_TABLE: &str = "
impl CapabilityTable {
    pub fn loom_new() -> Self {
        Self {
            slots: core::array::from_fn(|_| Slot { head: AtomicU64::new(0), a: AtomicU64::new(0), b: AtomicU64::new(0), c: AtomicU64::new(0) }),
            next_slot: AtomicU32::new(0),
            sealed: AtomicBool::new(false),
        }
    }
}
";
