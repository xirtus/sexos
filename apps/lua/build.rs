// SPDX-License-Identifier: GPL-3.0-only
//! Compiles the official Lua 5.4 interpreter (vendor/lua-5.4.7, MIT) against musl's
//! headers for the ordinary-program target, with `lua.c`'s `main` renamed so the Rust
//! side can start it, exactly as apps/chello does for its C program. Lua is built in
//! its plain ISO C configuration (no LUA_USE_POSIX/LINUX): `io.popen` and C modules
//! (`dlopen`) report that they are unsupported, everything else runs on sex-libc.
//! Without musl headers (or on the host) the program still builds and says so:
//! `cfg(c_program)` marks a build that has the C part.
use std::{env, path::PathBuf, process::Command};

const SOURCES: &[&str] = &[
    "lapi", "lcode", "lctype", "ldebug", "ldo", "ldump", "lfunc", "lgc", "llex", "lmem",
    "lobject", "lopcodes", "lparser", "lstate", "lstring", "ltable", "ltm", "lundump",
    "lvm", "lzio", "lauxlib", "lbaselib", "lcorolib", "ldblib", "liolib", "lmathlib",
    "loadlib", "loslib", "lstrlib", "ltablib", "lutf8lib", "linit", "lua",
];

fn main() {
    println!("cargo:rustc-link-arg-bins=-zstack-size=8388608");
    let src = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../vendor/lua-5.4.7/src");
    println!("cargo:rerun-if-changed={}", src.display());
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
    let mut objs = Vec::new();
    for name in SOURCES {
        let obj = out.join(format!("{name}.o"));
        let status = Command::new("clang")
            .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-std=gnu99", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
            .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector", "-Wall", "-Wno-unused"])
            .arg("-isystem").arg(format!("{}/include", resource.trim()))
            .arg("-isystem").arg(&musl)
            .args(["-DLUA_COMPAT_5_3", "-Dmain=sex_c_main", "-c"]).arg(src.join(format!("{name}.c"))).arg("-o").arg(&obj)
            .status().expect("clang");
        assert!(status.success(), "compiling {name}.c failed");
        objs.push(obj);
    }
    let lib = out.join("liblua.a");
    let _ = std::fs::remove_file(&lib);
    assert!(Command::new("llvm-ar").arg("crs").arg(&lib).args(&objs).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=lua");
}
