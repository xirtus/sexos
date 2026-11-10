// SPDX-License-Identifier: GPL-3.0-only
//! Compiles the C++ demo against libc++ (built by scripts/build_libcxx.sh) with the
//! flags of x86_64-sex-app (x87 and SSE, no AVX, no red zone) and links libc++.
//! Without a built libc++ (or on the host) the program still builds and says so:
//! `cfg(libcxx)` marks a build that has C++.
use std::{env, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let cxx = env::var("SEXOS_LIBCXX").map(PathBuf::from).unwrap_or_else(|_| root.join("target/libcxx"));
    println!("cargo:rerun-if-env-changed=SEXOS_LIBCXX");
    println!("cargo:rerun-if-changed=src/demo.cpp");
    println!("cargo:rerun-if-changed={}", cxx.join("lib/libc++.a").display());
    println!("cargo::rustc-check-cfg=cfg(libcxx)");
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("sex") && !target.ends_with("-none") {
        return;
    }
    if !cxx.join("lib/libc++.a").exists() {
        println!("cargo:warning=libc++ not built (looked in {}): run scripts/build_libcxx.sh", cxx.display());
        return;
    }
    println!("cargo:rustc-cfg=libcxx");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let resource = String::from_utf8(Command::new("clang").arg("-print-resource-dir").output().unwrap().stdout).unwrap();
    let obj = out.join("demo.o");
    let status = Command::new("clang++")
        .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-nostdinc++", "-std=c++20", "-O2", "-fPIC"])
        .args(["-ffunction-sections", "-fdata-sections", "-fno-exceptions", "-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector"])
        .arg("-isystem").arg(cxx.join("include/c++/v1"))
        .arg("-isystem").arg(format!("{}/include", resource.trim()))
        .arg("-isystem").arg(cxx.join("include-c"))
        .args(["-D_GNU_SOURCE", "-c", "src/demo.cpp", "-o"]).arg(&obj)
        .status().expect("clang++");
    assert!(status.success(), "compiling src/demo.cpp failed");
    let lib = out.join("libcppdemo.a");
    let _ = std::fs::remove_file(&lib);
    assert!(Command::new("llvm-ar").arg("crs").arg(&lib).arg(&obj).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={}", cxx.join("lib").display());
    // libc++ asks for -lpthread and -lc by pragma; sex-libc provides both.
    println!("cargo:rustc-link-arg=--no-dependent-libraries");
    for l in ["cppdemo", "c++", "c++abi"] {
        println!("cargo:rustc-link-lib=static={l}");
    }
}
