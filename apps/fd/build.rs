// An ordinary program's stack, as on Linux.
// The loader refuses code holding WRPKRU/XRSTOR bytes at any offset; a displacement can
// hold them by chance. The build then links again with sections in another order
// (SEX_LAYOUT_SEED, see scripts/sexos_build_trace.sh), which moves every displacement.
fn main() {
    println!("cargo:rustc-link-arg-bins=-zstack-size=8388608");
    println!("cargo:rerun-if-env-changed=SEX_LAYOUT_SEED");
    if let Some(seed) = std::env::var("SEX_LAYOUT_SEED").ok().filter(|s| !s.is_empty() && s != "0") {
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.rodata*={seed}");
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.text*={seed}");
    }
}
