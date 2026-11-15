// An ordinary program's stack, as on Linux. The loader refuses code holding WRPKRU or
// XRSTOR bytes; when a displacement holds them by chance the build links again with
// sections in another order (SEX_LAYOUT_SEED, scripts/sexos_build_trace.sh).
fn main() {
    println!("cargo:rustc-link-arg-bins=-zstack-size=8388608");
    println!("cargo:rerun-if-env-changed=SEX_LAYOUT_SEED");
    if let Some(seed) = std::env::var("SEX_LAYOUT_SEED").ok().filter(|s| !s.is_empty() && s != "0") {
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.rodata*={seed}");
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.text*={seed}");
    }
}
