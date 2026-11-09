// SPDX-License-Identifier: GPL-3.0-only
//! Links FFmpeg (built by scripts/build_ffmpeg.sh) and compiles Play's C bridge with
//! the flags FFmpeg was built with (x86_64-sex-app: x87 and SSE, no AVX, no red zone).
use std::{env, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let ff = env::var("SEXOS_FFMPEG").map(PathBuf::from).unwrap_or_else(|_| root.join("target/ffmpeg"));
    println!("cargo:rerun-if-env-changed=SEXOS_FFMPEG");
    println!("cargo:rerun-if-changed=src/ffmpeg.c");
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("sex") && !target.ends_with("-none") {
        return; // host checks only; the program itself is built for SexOS
    }
    assert!(ff.join("lib/libavcodec.a").exists(), "FFmpeg not built: run scripts/build_ffmpeg.sh (looked in {})", ff.display());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let resource = String::from_utf8(Command::new("clang").arg("-print-resource-dir").output().unwrap().stdout).unwrap();
    let obj = out.join("ffmpeg_bridge.o");
    let status = Command::new("clang")
        .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
        .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector"])
        .arg("-isystem").arg(format!("{}/include", resource.trim()))
        .arg("-isystem").arg(ff.join("include-c"))
        .arg("-I").arg(ff.join("include"))
        .arg("-c").arg("src/ffmpeg.c").arg("-o").arg(&obj)
        .status().expect("clang");
    assert!(status.success(), "compiling src/ffmpeg.c failed");
    let lib = out.join("libplaybridge.a");
    let _ = std::fs::remove_file(&lib);
    assert!(Command::new("llvm-ar").arg("crs").arg(&lib).arg(&obj).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={}", ff.join("lib").display());
    for l in ["playbridge", "avformat", "avcodec", "swscale", "swresample", "avutil"] {
        println!("cargo:rustc-link-lib=static={l}");
    }
}
