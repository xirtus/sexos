// SPDX-License-Identifier: GPL-3.0-only
//! Links FFmpeg (built by scripts/build_ffmpeg.sh) and compiles sex-media's C bridge with
//! the flags FFmpeg was built with (x86_64-sex-app: x87 and SSE, no AVX, no red zone).
//! Without a built FFmpeg (or on the host) the crate still builds, and every file is
//! refused as unsupported: `cfg(ffmpeg)` marks a build that can decode.
use std::{env, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let ff = env::var("SEXOS_FFMPEG").map(PathBuf::from).unwrap_or_else(|_| root.join("target/ffmpeg"));
    println!("cargo:rerun-if-env-changed=SEXOS_FFMPEG");
    // Restored caches keep old modification times, so a build script run cached with
    // one FFmpeg profile is not rerun when a later job restores another profile's
    // archives (the link list then lacks x264/aom). CI names the profile here; a
    // changed value reruns this script whatever the file times say.
    println!("cargo:rerun-if-env-changed=SEXOS_FFMPEG_PROFILE");
    println!("cargo:rerun-if-changed=src/bridge.c");
    println!("cargo:rerun-if-changed={}", ff.join("lib/libavcodec.a").display());
    println!("cargo::rustc-check-cfg=cfg(ffmpeg)");
    println!("cargo::rustc-check-cfg=cfg(ffmpeg_recording)");
    println!("cargo::rustc-check-cfg=cfg(ffmpeg_export)");
    let recording_marker = ff.join("sexos-recording-v1");
    if recording_marker.exists() { println!("cargo:rerun-if-changed={}", recording_marker.display()); }
    let target = env::var("TARGET").unwrap_or_default();
    println!("cargo:rerun-if-env-changed=SEXOS_MEDIA_HOST_FFMPEG");
    if !target.contains("sex") && !target.ends_with("-none") {
        // Host tests: with SEXOS_MEDIA_HOST_FFMPEG=1, the bridges are built against the
        // system's FFmpeg (pkg-config) so the encoders' output can be decoded back.
        if env::var("SEXOS_MEDIA_HOST_FFMPEG").is_ok_and(|v| v == "1") { host_ffmpeg(); }
        return; // host checks only; the program itself is built for SexOS
    }
    if !ff.join("lib/libavcodec.a").exists() {
        println!("cargo:warning=FFmpeg not built (looked in {}): media will be refused. Run scripts/build_ffmpeg.sh", ff.display());
        return;
    }
    println!("cargo:rustc-cfg=ffmpeg");
    // 1: the recorder (FFV1/Matroska); 2: that and Splicy's export (WebM/MP4).
    let profile = std::fs::read(&recording_marker).unwrap_or_default();
    let export = profile == b"2\n";
    let recording = export || profile == b"1\n";
    if recording { println!("cargo:rustc-cfg=ffmpeg_recording"); }
    if export { println!("cargo:rustc-cfg=ffmpeg_export"); }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let resource = String::from_utf8(Command::new("clang").arg("-print-resource-dir").output().unwrap().stdout).unwrap();
    let obj = out.join("ffmpeg_bridge.o");
    let status = Command::new("clang")
        .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
        .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector"])
        .arg("-isystem").arg(format!("{}/include", resource.trim()))
        .arg("-isystem").arg(ff.join("include-c"))
        .arg("-I").arg(ff.join("include"))
        .arg("-c").arg("src/bridge.c").arg("-o").arg(&obj)
        .status().expect("clang");
    assert!(status.success(), "compiling src/bridge.c failed");
    let lib = out.join("libmediabridge.a");
    let _ = std::fs::remove_file(&lib);
    // The SIMD bench (src/bench.c): baseline flags like the bridge; the codecs choose
    // their AVX code at run time.
    println!("cargo:rerun-if-changed=src/bench.c");
    let bench = out.join("bench.o");
    assert!(Command::new("clang")
        .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-std=c17", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
        .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector", "-Wall", "-Werror", "-D_GNU_SOURCE"])
        .arg("-isystem").arg(format!("{}/include", resource.trim()))
        .arg("-isystem").arg(ff.join("include-c"))
        .arg("-I").arg(ff.join("include"))
        .args(["-c", "src/bench.c", "-o"]).arg(&bench)
        .status().expect("clang bench").success(), "compiling src/bench.c failed");
    let mut objects = vec![obj, bench];
    if recording {
        println!("cargo:rerun-if-changed=src/recording/bridge.c");
        let object = out.join("recording_bridge.o");
        assert!(Command::new("clang")
            .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-std=c17", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
            .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector", "-Wall", "-Werror"])
            .arg("-isystem").arg(format!("{}/include", resource.trim()))
            .arg("-isystem").arg(ff.join("include-c"))
            .arg("-I").arg(ff.join("include"))
            .args(["-c", "src/recording/bridge.c", "-o"]).arg(&object)
            .status().expect("clang recording bridge").success());
        objects.push(object);
    }
    if export {
        println!("cargo:rerun-if-changed=src/export/bridge.c");
        let object = out.join("export_bridge.o");
        assert!(Command::new("clang")
            .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-std=c17", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
            .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector", "-Wall", "-Werror", "-D_GNU_SOURCE"])
            .arg("-isystem").arg(format!("{}/include", resource.trim()))
            .arg("-isystem").arg(ff.join("include-c"))
            .arg("-I").arg(ff.join("include"))
            .args(["-c", "src/export/bridge.c", "-o"]).arg(&object)
            .status().expect("clang export bridge").success());
        objects.push(object);
        // The live recorder (MediaRecorder) uses the same encoders.
        println!("cargo:rerun-if-changed=src/live/bridge.c");
        let object = out.join("live_bridge.o");
        assert!(Command::new("clang")
            .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-std=c17", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
            .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector", "-Wall", "-Werror", "-D_GNU_SOURCE"])
            .arg("-isystem").arg(format!("{}/include", resource.trim()))
            .arg("-isystem").arg(ff.join("include-c"))
            .arg("-I").arg(ff.join("include"))
            .args(["-c", "src/live/bridge.c", "-o"]).arg(&object)
            .status().expect("clang live bridge").success());
        objects.push(object);
    }
    assert!(Command::new("llvm-ar").arg("crs").arg(&lib).args(&objects).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={}", ff.join("lib").display());
    // dav1d (AV1) sits beside FFmpeg's archives when it was built with them.
    // So do libvpx (VP9 export) and the other export encoders in the export profile.
    // FFmpeg's registry references export encoders even in decoding consumers.
    let shine = ff.join("lib/libshine.a");
    if shine.exists() { println!("cargo:rerun-if-changed={}", shine.display()); }
    for l in ["mediabridge", "avformat", "avcodec", "shine", "dav1d", "vpx", "aom", "x264", "swscale", "swresample", "avutil"] {
        if matches!(l, "shine" | "dav1d" | "vpx" | "aom" | "x264") && !ff.join(format!("lib/lib{l}.a")).exists() { continue; }
        println!("cargo:rustc-link-lib=static={l}");
    }
}

/// The bridges against the host's FFmpeg, linked dynamically, for host tests only.
fn host_ffmpeg() {
    let pkg = Command::new("pkg-config").args(["--cflags", "--libs", "libavformat", "libavcodec", "libswscale", "libswresample", "libavutil"]).output();
    let Ok(pkg) = pkg.map_err(|_| ()).and_then(|o| if o.status.success() { Ok(o) } else { Err(()) }) else {
        println!("cargo:warning=SEXOS_MEDIA_HOST_FFMPEG=1 but pkg-config finds no FFmpeg development files");
        return;
    };
    let flags = String::from_utf8(pkg.stdout).unwrap();
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut objects = Vec::new();
    for (src, name) in [("src/bridge.c", "bridge"), ("src/recording/bridge.c", "recording"), ("src/export/bridge.c", "export"), ("src/live/bridge.c", "live"), ("src/bench.c", "bench")] {
        println!("cargo:rerun-if-changed={src}");
        let object = out.join(format!("host_{name}.o"));
        let mut cc = Command::new("cc");
        cc.args(["-std=gnu17", "-O2", "-fPIC", "-D_GNU_SOURCE", "-c", src, "-o"]).arg(&object);
        for f in flags.split_whitespace().filter(|f| f.starts_with("-I")) { cc.arg(f); }
        assert!(cc.status().expect("cc").success(), "compiling {src} against the host's FFmpeg failed");
        objects.push(object);
    }
    let lib = out.join("libmediahost.a");
    let _ = std::fs::remove_file(&lib);
    assert!(Command::new("ar").arg("crs").arg(&lib).args(&objects).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=mediahost");
    for f in flags.split_whitespace() {
        if let Some(d) = f.strip_prefix("-L") { println!("cargo:rustc-link-search=native={d}"); }
        if let Some(l) = f.strip_prefix("-l") { println!("cargo:rustc-link-lib={l}"); }
    }
    for cfg in ["ffmpeg", "ffmpeg_recording", "ffmpeg_export"] { println!("cargo:rustc-cfg={cfg}"); }
}
