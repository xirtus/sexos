// SPDX-License-Identifier: GPL-3.0-only
//! Compiles SDL2 (vendor/SDL2-2.30.9, zlib) into `libSDL2.a` for the ordinary-program
//! target. It uses musl's headers, the same way apps/lua builds Lua. SDL's sources are
//! the vendored ones, unmodified. `c/SDL_config.h` replaces SDL's configuration in a
//! copy of its headers, and the SexOS video, sound and input backends in `c/` take the
//! slots of SDL's dummy drivers.
//!
//! The copied headers' folder is passed on to programs that link SDL: their build
//! scripts read it as `DEP_SDL2_INCLUDE`. Without musl headers (or on the host) the
//! crate still builds, without SDL, and `cfg(sdl)` is not set.
use std::{env, fs, path::{Path, PathBuf}, process::Command, sync::Mutex};

/// Every .c file in these folders of SDL's `src` (the conditional ones compile to
/// nothing in this configuration).
const FOLDERS: &[&str] = &[
    "", "atomic", "audio", "cpuinfo", "dynapi", "events", "file", "filesystem/dummy",
    "haptic", "haptic/dummy", "libm", "loadso/dummy", "locale", "locale/dummy", "misc",
    "misc/dummy", "power", "render", "render/software", "sensor", "sensor/dummy",
    "stdlib", "timer", "timer/unix", "video", "video/yuv2rgb",
];
/// Threads: pthreads, with SDL's generic semaphore.
const FILES: &[&str] = &[
    "thread/SDL_thread.c", "thread/pthread/SDL_systhread.c", "thread/pthread/SDL_sysmutex.c",
    "thread/pthread/SDL_syscond.c", "thread/pthread/SDL_systls.c", "thread/generic/SDL_syssem.c",
];
const BACKENDS: &[&str] = &["SDL_sexosvideo.c", "SDL_sexosaudio.c", "SDL_sexosstubs.c"];

fn c_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir).unwrap().filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "c")).collect();
    v.sort();
    v
}

fn main() {
    let here = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let sdl = here.join("../../vendor/SDL2-2.30.9");
    println!("cargo:rerun-if-changed={}", sdl.display());
    println!("cargo:rerun-if-changed=c");
    println!("cargo:rerun-if-env-changed=MUSL_INCLUDE");
    println!("cargo::rustc-check-cfg=cfg(sdl)");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("hermit") { return; }
    let musl = env::var("MUSL_INCLUDE").map(PathBuf::from).ok().or_else(|| {
        ["/usr/lib/musl/include", "/usr/include/x86_64-linux-musl", "/usr/local/musl/include"].iter().map(PathBuf::from).find(|d| d.join("stdio.h").exists())
    });
    let Some(musl) = musl else {
        println!("cargo:warning=musl headers not found: set MUSL_INCLUDE (apt install musl-dev)");
        return;
    };
    println!("cargo:rustc-cfg=sdl");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    // SDL's public headers, with this configuration in place of its own.
    let include = out.join("include");
    let _ = fs::remove_dir_all(&include);
    fs::create_dir_all(include.join("SDL2")).unwrap();
    for e in fs::read_dir(sdl.join("include")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "h") {
            let name = p.file_name().unwrap();
            fs::copy(&p, include.join(name)).unwrap();
            fs::copy(&p, include.join("SDL2").join(name)).unwrap();
        }
    }
    fs::copy(here.join("c/SDL_config.h"), include.join("SDL_config.h")).unwrap();
    fs::copy(here.join("c/SDL_config.h"), include.join("SDL2/SDL_config.h")).unwrap();
    println!("cargo:include={}", include.display());

    let src = sdl.join("src");
    let mut sources: Vec<PathBuf> = FOLDERS.iter().flat_map(|f| c_files(&src.join(f))).collect();
    sources.extend(FILES.iter().map(|f| src.join(f)));
    sources.extend(BACKENDS.iter().map(|f| here.join("c").join(f)));

    let resource = String::from_utf8(Command::new("clang").arg("-print-resource-dir").output().expect("clang").stdout).unwrap();
    let resource = format!("{}/include", resource.trim());
    let objs = out.join("obj");
    fs::create_dir_all(&objs).unwrap();
    let work = Mutex::new(sources.into_iter().enumerate().collect::<Vec<_>>());
    let done = Mutex::new(Vec::new());
    let jobs = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                let Some((n, file)) = work.lock().unwrap().pop() else { break };
                let obj = objs.join(format!("{n:03}-{}.o", file.file_stem().unwrap().to_string_lossy()));
                let status = Command::new("clang")
                    .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-std=gnu11", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
                    .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector", "-w"])
                    .arg("-isystem").arg(&resource)
                    .arg("-isystem").arg(&musl)
                    .arg("-I").arg(&include)
                    .arg("-I").arg(&src)
                    .arg("-I").arg(here.join("c"))
                    // SDL's dynamic API needs dlopen, which programs do not have.
                    .args(["-DDYNAPI_NEEDS_DLOPEN=1", "-c"]).arg(&file).arg("-o").arg(&obj)
                    .status().expect("clang");
                assert!(status.success(), "compiling {} failed", file.display());
                done.lock().unwrap().push(obj);
            });
        }
    });
    let mut objs = done.into_inner().unwrap();
    objs.sort();
    let lib = out.join("libSDL2.a");
    let _ = fs::remove_file(&lib);
    assert!(Command::new("llvm-ar").arg("crs").arg(&lib).args(&objs).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=SDL2");
}
