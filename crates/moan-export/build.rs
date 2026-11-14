// SPDX-License-Identifier: GPL-3.0-only
use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo::rustc-check-cfg=cfg(moan_codecs)");
    for name in ["SEXOS_FFMPEG", "MOAN_EXPORT_HOST_FFMPEG"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=src/bridge.c");
    let guest = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none");
    if !guest && env::var_os("CARGO_FEATURE_HOST_CODECS").is_none() {
        return;
    }
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let prefix = if guest {
        env::var_os("SEXOS_FFMPEG")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("target/ffmpeg"))
    } else {
        env::var_os("MOAN_EXPORT_HOST_FFMPEG")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("target/moan-host-ffmpeg"))
    };
    let marker = prefix.join("sexos-audio-export-v1");
    println!("cargo:rerun-if-changed={}", marker.display());
    assert!(std::fs::read(&marker).is_ok_and(|b|b==b"1\n") && prefix.join("lib/libshine.a").exists(),
        "Moan's real WAV/FLAC/MP3 export profile missing at {}. Run scripts/build_ffmpeg.sh (guest) or scripts/build_moan_host_ffmpeg.sh (native host)",prefix.display());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = out.join("moan_audio_bridge.o");
    let mut cc = Command::new("clang");
    cc.args([
        "-std=c17",
        "-O2",
        "-fPIC",
        "-ffunction-sections",
        "-fdata-sections",
        "-Wall",
        "-Werror",
    ]);
    if guest {
        let resource = Command::new("clang")
            .arg("-print-resource-dir")
            .output()
            .unwrap();
        let resource = String::from_utf8(resource.stdout).unwrap();
        cc.args([
            "--target=x86_64-unknown-none-elf",
            "-nostdinc",
            "-msse2",
            "-mno-avx",
            "-mno-red-zone",
            "-fno-stack-protector",
        ])
        .arg("-isystem")
        .arg(format!("{}/include", resource.trim()))
        .arg("-isystem")
        .arg(prefix.join("include-c"));
    }
    assert!(cc
        .arg("-I")
        .arg(prefix.join("include"))
        .args(["-c", "src/bridge.c", "-o"])
        .arg(&object)
        .status()
        .expect("clang audio bridge")
        .success());
    let library = out.join("libmoanaudiobridge.a");
    let _ = std::fs::remove_file(&library);
    assert!(Command::new("llvm-ar")
        .arg("crs")
        .arg(&library)
        .arg(object)
        .status()
        .unwrap()
        .success());
    println!("cargo:rustc-cfg=moan_codecs");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    for name in [
        "moanaudiobridge",
        "avformat",
        "avcodec",
        "shine",
        "dav1d",
        "swscale",
        "swresample",
        "avutil",
    ] {
        if !prefix.join(format!("lib/lib{name}.a")).exists()
            && ["dav1d", "swscale", "swresample"].contains(&name)
        {
            continue;
        }
        println!("cargo:rustc-link-lib=static={name}");
    }
}
