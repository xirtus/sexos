// SPDX-License-Identifier: GPL-3.0-only
//! Detects an optional gitignored `qupid_seed_pack.json` beside `Cargo.toml` so private
//! local builds can bake an external `ProviderPack` into Qupid without touching tracked
//! public sources or setting Cargo feature flags.
use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=qupid_seed_pack.json");
    println!("cargo::rustc-check-cfg=cfg(qupid_seed_pack)");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    if manifest_dir.join("qupid_seed_pack.json").is_file() {
        println!("cargo:rustc-cfg=qupid_seed_pack");
    }
}
