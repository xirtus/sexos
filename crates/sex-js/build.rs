// SPDX-License-Identifier: GPL-3.0-only
//! The native tier: translates quickjs.wasm to C with wasm2c (wabt) and compiles it with
//! clang, so SexOS runs QuickJS as machine code instead of interpreting it on wasmi.
//! Built for SexOS targets when wasm2c is on PATH (SEXJS_NATIVE=0 turns it off); on the
//! host only with SEXJS_NATIVE=1. `cfg(sexjs_native)` marks a build that has it.
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(sexjs_native)");
    println!("cargo:rerun-if-env-changed=SEXJS_NATIVE");
    for f in ["quickjs.wasm", "native/rt.c", "native/include"] { println!("cargo:rerun-if-changed={f}"); }
    let target = env::var("TARGET").unwrap_or_default();
    let sex = target.contains("sex");
    let want = env::var("SEXJS_NATIVE").ok();
    if want.as_deref() == Some("0") || (!sex && want.as_deref() != Some("1")) { return; }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let c = out.join("qjs.c");
    match Command::new("wasm2c").args(["quickjs.wasm", "-n", "sexjs", "-o"]).arg(&c).status() {
        Ok(s) if s.success() => {}
        _ => {
            println!("cargo:warning=wasm2c (wabt) not found: JavaScript stays on the wasmi interpreter");
            return;
        }
    }
    let rt_h = String::from_utf8(Command::new("wasm2c").arg("--version").output().unwrap().stdout).unwrap();
    let rt_dir = ["/usr/include", "/usr/local/include", "/usr/src/wasm2c"].iter().map(PathBuf::from)
        .find(|d| d.join("wasm-rt.h").exists()).unwrap_or_else(|| panic!("wasm-rt.h for wasm2c {} not found", rt_h.trim()));
    // Copied beside qjs.c: its own directory (/usr/include) would bring the host libc.
    std::fs::copy(rt_dir.join("wasm-rt.h"), out.join("wasm-rt.h")).unwrap();
    let resource = String::from_utf8(Command::new("clang").arg("-print-resource-dir").output().unwrap().stdout).unwrap();
    let mut objs = Vec::new();
    for (src, name) in [(c.clone(), "qjs.o"), (PathBuf::from("native/rt.c"), "rt.o")] {
        let obj = out.join(name);
        let mut cc = Command::new("clang");
        if sex {
            // C99: no thread-locals (a page's engine runs on one thread).
            cc.args(["--target=x86_64-unknown-none-elf", "-msse2", "-mno-avx", "-mno-red-zone", "-std=gnu99"]);
        } else {
            // C11: the trap state is thread-local, for tests that run engines side by side.
            cc.arg("-std=gnu11");
        }
        // Only our own headers (native/include) and clang's: no libc is linked.
        cc.args(["-c", "-O2", "-fPIC", "-nostdinc", "-fno-stack-protector", "-ffunction-sections", "-fdata-sections", "-w"])
            .arg("-isystem").arg(format!("{}/include", resource.trim()))
            .arg("-Inative/include").arg("-I").arg(&out)
            // Explicit bounds checks keep scripts inside linear memory; deep JS recursion
            // stops at the call depth wasmi allows (1024), so native frames fit the stack.
            .args(["-DWASM_RT_MEMCHECK_GUARD_PAGES=0", "-DWASM_RT_MEMCHECK_BOUNDS_CHECK=1", "-DWASM_RT_USE_MMAP=0", "-DWASM_RT_SKIP_SIGNAL_RECOVERY=1", "-DWASM_RT_MAX_CALL_STACK_DEPTH=1024"])
            .arg(&src).arg("-o").arg(&obj);
        assert!(cc.status().expect("clang").success(), "compiling {} failed", src.display());
        objs.push(obj);
    }
    let lib = out.join("libsexjs_native.a");
    let _ = std::fs::remove_file(&lib);
    assert!(Command::new("llvm-ar").arg("crs").arg(&lib).args(&objs).status().expect("llvm-ar").success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=sexjs_native");
    println!("cargo:rustc-cfg=sexjs_native");
}
