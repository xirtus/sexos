// SPDX-License-Identifier: GPL-3.0-only
//! Compiles the ordinary C program in src/chello.c against musl's headers (the C types
//! sex-libc implements) for the ordinary-program target, with its `main` renamed so
//! the Rust side can start it. Without musl headers (or on the host) the program still
//! builds and says so: `cfg(c_program)` marks a build that has the C part.
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rustc-link-arg-bins=-zstack-size=8388608");
    println!("cargo:rerun-if-changed=src/chello.c");
    println!("cargo:rerun-if-env-changed=MUSL_INCLUDE");
    println!("cargo::rustc-check-cfg=cfg(c_program)");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("hermit") { return; }
    let musl = env::var("MUSL_INCLUDE").map(PathBuf::from).ok().or_else(|| {
        ["/usr/lib/musl/include", "/usr/include/x86_64-linux-musl", "/usr/local/musl/include"].iter().map(PathBuf::from).find(|d| d.join("stdio.h").exists())
    });
    let Some(musl) = musl else {
        println!("cargo:warning=musl headers not found: set MUSL_INCLUDE (apt install musl-dev)");
        return;
    };
    println!("cargo:rustc-cfg=c_program");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let resource = String::from_utf8(Command::new("clang").arg("-print-resource-dir").output().expect("clang").stdout).unwrap();
    let obj = out.join("chello.o");
    let status = Command::new("clang")
        .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-std=c17", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
        .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector", "-Wall", "-Werror"])
        .arg("-isystem").arg(format!("{}/include", resource.trim()))
        .arg("-isystem").arg(&musl)
        .args(["-D_GNU_SOURCE", "-Dmain=sex_c_main", "-c", "src/chello.c", "-o"]).arg(&obj)
        .status().expect("clang");
    assert!(status.success(), "compiling src/chello.c failed");
    let lib = out.join("libchello.a");
    let _ = std::fs::remove_file(&lib);
    assert!(Command::new("llvm-ar").arg("crs").arg(&lib).arg(&obj).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=chello");
}
