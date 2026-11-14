// SPDX-License-Identifier: GPL-3.0-only
//! Compiles the SDL game in src/driftspark.c the way apps/chello compiles C. It
//! uses musl's headers and SDL's headers, which crates/sex-sdl passes on as
//! `DEP_SDL2_INCLUDE`. Its `main` is renamed so that the Rust side can start it.
//! Without them (or on the host) the program still builds and says so:
//! `cfg(c_program)` marks a build that has the C part.
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rustc-link-arg-bins=-zstack-size=8388608");
    println!("cargo:rerun-if-changed=src/driftspark.c");
    println!("cargo:rerun-if-env-changed=MUSL_INCLUDE");
    println!("cargo::rustc-check-cfg=cfg(c_program)");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("hermit") { return; }
    let Ok(sdl) = env::var("DEP_SDL2_INCLUDE") else {
        println!("cargo:warning=SDL was not built (musl headers missing): set MUSL_INCLUDE (apt install musl-dev)");
        return;
    };
    let musl = env::var("MUSL_INCLUDE").map(PathBuf::from).ok().or_else(|| {
        ["/usr/lib/musl/include", "/usr/include/x86_64-linux-musl", "/usr/local/musl/include"].iter().map(PathBuf::from).find(|d| d.join("stdio.h").exists())
    }).expect("musl headers");
    println!("cargo:rustc-cfg=c_program");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let resource = String::from_utf8(Command::new("clang").arg("-print-resource-dir").output().expect("clang").stdout).unwrap();
    let obj = out.join("driftspark.o");
    let status = Command::new("clang")
        .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-std=gnu11", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
        .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector", "-Wall", "-Werror"])
        .arg("-isystem").arg(format!("{}/include", resource.trim()))
        .arg("-isystem").arg(&musl)
        .arg("-I").arg(&sdl)
        .args(["-D_GNU_SOURCE", "-Dmain=sex_c_main", "-c", "src/driftspark.c", "-o"]).arg(&obj)
        .status().expect("clang");
    assert!(status.success(), "compiling src/driftspark.c failed");
    let lib = out.join("libdriftspark.a");
    let _ = std::fs::remove_file(&lib);
    assert!(Command::new("llvm-ar").arg("crs").arg(&lib).arg(&obj).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=driftspark");
}
