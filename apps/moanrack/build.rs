//! Qualification fixtures: the unchanged official CLAP C template plus two
//! explicitly modified crash/hang negatives. These are not bundled effects.
use std::{env, fs, path::PathBuf, process::Command};
fn execute(command: &mut Command) {
    let status = command.status().expect("tool executable");
    assert!(status.success(), "fixture compilation failed");
}
fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") {
        return;
    }
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let fixture = root.join("vendor/clap-fixture");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let source = fs::read_to_string(fixture.join("src/plugin-template.c")).unwrap();
    let clang = env::var("CLANG").unwrap_or("clang".into());
    let linker = env::var("LD_LLD").unwrap_or("ld.lld".into());
    let headers = PathBuf::from(
        env::var_os("SEXOS_LIBCXX").expect("SEXOS_LIBCXX required: freestanding musl headers"),
    )
    .join("include-c");
    let resource = Command::new(&clang)
        .arg("--print-resource-dir")
        .output()
        .expect("clang resources");
    assert!(resource.status.success());
    let resource =
        PathBuf::from(String::from_utf8(resource.stdout).unwrap().trim()).join("include");
    for (name, code) in [
        ("swap", ""),
        ("crash", "__builtin_trap();"),
        ("hang", "for (;;) { __asm__ volatile(\"pause\"); }"),
    ] {
        let process = "   const uint32_t nframes = process->frames_count;";
        assert!(source.contains(process));
        let body = source.replacen(process, &format!("   {code}\n{process}"), 1);
        let c = out.join(format!("{name}.c"));
        let o = out.join(format!("{name}.o"));
        fs::write(&c, body).unwrap();
        execute(
            Command::new(&clang)
                .args([
                    "--target=x86_64-unknown-none-elf",
                    "-std=c11",
                    "-fPIC",
                    "-O2",
                    "-DNDEBUG",
                    "-ffreestanding",
                    "-fno-stack-protector",
                    "-mno-red-zone",
                    "-msse2",
                    "-mno-avx",
                    "-nostdinc",
                    "-isystem",
                ])
                .arg(&headers)
                .arg("-isystem")
                .arg(&resource)
                .arg("-I")
                .arg(fixture.join("include"))
                .arg("-c")
                .arg(c)
                .arg("-o")
                .arg(&o),
        );
        execute(
            Command::new(&linker)
                .args([
                    "-shared",
                    "--hash-style=sysv",
                    "-z",
                    "max-page-size=4096",
                    "-o",
                ])
                .arg(out.join(format!("{name}.clap")))
                .arg(o),
        );
    }
    let vst = env::var_os("SEXOS_VST_FIXTURE").expect("source-built SEXOS_VST_FIXTURE required");
    fs::copy(&vst, out.join("gain.vst3")).expect("source-built VST fixture");
    println!("cargo:rerun-if-changed={}", PathBuf::from(vst).display());
    println!("cargo:rerun-if-env-changed=SEXOS_VST_FIXTURE");
    println!("cargo:rerun-if-changed=../../vendor/clap-fixture");
    for key in ["SEXOS_LIBCXX", "CLANG", "LD_LLD", "SEX_LAYOUT_SEED"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    if let Ok(seed) = env::var("SEX_LAYOUT_SEED") {
        if !seed.is_empty() && seed != "0" {
            println!("cargo:rustc-link-arg-bins=--shuffle-sections=.rodata*={seed}");
            println!("cargo:rustc-link-arg-bins=--shuffle-sections=.text*={seed}");
        }
    }
}
