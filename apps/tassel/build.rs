// A relative displacement can contain bytes the program loader refuses.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("hermit") { return; }
    println!("cargo:rustc-link-arg-bins=-zstack-size=8388608");
    println!("cargo:rerun-if-env-changed=SEX_LAYOUT_SEED");
    if let Some(seed) = std::env::var("SEX_LAYOUT_SEED").ok().filter(|s| !s.is_empty() && s != "0") {
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.rodata*={seed}");
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.text*={seed}");
    }
}
