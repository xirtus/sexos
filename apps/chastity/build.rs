// The loader refuses code holding WRPKRU/XRSTOR bytes at any offset, and a RIP-relative
// displacement can hold them by chance. The build then links again with the read-only
// data in another order (SEX_LAYOUT_SEED), which moves every displacement.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("hermit") { return; }
    println!("cargo:rustc-link-arg-bins=-zstack-size=8388608");
    println!("cargo:rerun-if-env-changed=SEX_LAYOUT_SEED");
    if let Some(seed) = std::env::var("SEX_LAYOUT_SEED").ok().filter(|s| !s.is_empty() && s != "0") {
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.rodata*={seed}");
        // A call between functions can hold them too: move the functions as well.
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.text*={seed}");
    }
}
