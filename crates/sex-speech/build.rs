// SPDX-License-Identifier: GPL-3.0-only
//! Compiles eSpeak NG 1.52.0 (vendor/espeak-ng-1.52.0, GPL-3.0-or-later) and the
//! SexOS file shim (csrc/) into a static library, the way apps/lua compiles Lua:
//! - for SexOS programs (`target_os = "hermit"`, x86_64-sex-std.json) against musl's
//!   headers, linked with sex-libc;
//! - for the host (cargo test) with the host's own C compiler and C library.
//! Without a C compiler or (for SexOS) musl headers the crate still builds and
//! `speak` reports `Error::Unavailable`; `cfg(espeak)` marks a build with the voice.
use std::{env, path::{Path, PathBuf}, process::Command};

/// eSpeak NG's runtime sources. The data compilers (compiledata, compiledict, spect)
/// are linked only because the public API names them; SexOS never calls them.
const ESPEAK: &[&str] = &[
    "common", "dictionary", "encoding", "error", "espeak_api", "ieee80", "intonation",
    "klatt", "langopts", "mnemonics", "numbers", "phoneme", "phonemelist", "readclause",
    "setlengths", "soundicon", "speech", "ssml", "synthdata", "synthesize", "tr_languages",
    "translate", "translateword", "voices", "wavegen", "compiledata", "compiledict", "spect",
];
const UCD: &[&str] = &["case", "categories", "ctype", "proplist", "scripts", "tostring"];

fn main() {
    let here = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor = here.join("../../vendor/espeak-ng-1.52.0/src");
    println!("cargo:rerun-if-changed={}", vendor.display());
    println!("cargo:rerun-if-changed={}", here.join("csrc").display());
    println!("cargo:rerun-if-env-changed=MUSL_INCLUDE");
    println!("cargo::rustc-check-cfg=cfg(espeak)");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let mut flags: Vec<String> = vec!["-std=gnu99".into(), "-O2".into(), "-fPIC".into(), "-ffunction-sections".into(), "-fdata-sections".into(), "-w".into()];
    if os == "hermit" {
        let musl = env::var("MUSL_INCLUDE").map(PathBuf::from).ok().or_else(|| {
            ["/usr/lib/musl/include", "/usr/include/x86_64-linux-musl", "/usr/local/musl/include"].iter().map(PathBuf::from).find(|d| d.join("stdio.h").exists())
        });
        let Some(musl) = musl else {
            println!("cargo:warning=musl headers not found: set MUSL_INCLUDE (apt install musl-dev); the voice is left out");
            return;
        };
        let Some(resource) = Command::new("clang").arg("-print-resource-dir").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()) else {
            println!("cargo:warning=clang not found; the voice is left out");
            return;
        };
        flags.extend(["--target=x86_64-unknown-none-elf", "-nostdinc", "-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector"].map(String::from));
        flags.extend(["-isystem".into(), format!("{resource}/include"), "-isystem".into(), musl.display().to_string()]);
    } else if os != "linux" && os != "macos" {
        // Bare-metal targets (x86_64-sex-app.json) have no C library to link against.
        return;
    }
    if Command::new("clang").arg("--version").output().is_err() {
        println!("cargo:warning=clang not found; the voice is left out");
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let includes = [vendor.join("include"), vendor.join("libespeak-ng"), vendor.join("ucd-tools/src/include"), here.join("csrc")];
    let mut objs = Vec::new();
    let mut compile = |src: &Path, name: &str, shim: bool| {
        let obj = out.join(format!("{name}.o"));
        let mut cmd = Command::new("clang");
        cmd.args(&flags);
        for i in &includes { cmd.arg("-I").arg(i); }
        cmd.arg("-DPATH_ESPEAK_DATA=\"/sexos/espeak-ng-data\"").arg("-DLIBESPEAK_NG_EXPORT=1");
        if shim { cmd.arg("-include").arg(here.join("csrc/sexos_espeak_shim.h")); }
        cmd.arg("-c").arg(src).arg("-o").arg(&obj);
        let status = cmd.status().expect("clang");
        assert!(status.success(), "compiling {} failed", src.display());
        objs.push(obj);
    };
    for name in ESPEAK { compile(&vendor.join(format!("libespeak-ng/{name}.c")), name, true); }
    for name in UCD { compile(&vendor.join(format!("ucd-tools/src/{name}.c")), &format!("ucd_{name}"), false); }
    compile(&here.join("csrc/sexos_espeak_shim.c"), "sexos_espeak_shim", false);
    let lib = out.join("libsexespeak.a");
    let _ = std::fs::remove_file(&lib);
    let ar = if Command::new("llvm-ar").arg("--version").output().is_ok() { "llvm-ar" } else { "ar" };
    assert!(Command::new(ar).arg("crs").arg(&lib).args(&objs).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=sexespeak");
    if os != "hermit" { println!("cargo:rustc-link-lib=m"); }
    println!("cargo:rustc-cfg=espeak");
}
