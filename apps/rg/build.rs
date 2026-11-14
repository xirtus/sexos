// An ordinary program's stack, as on Linux.
fn main() {
    // Native fixture builds use their platform linker; these flags are for LLD
    // and the SexOS program loader only.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("hermit") {
        return;
    }
    println!("cargo:rustc-link-arg-bins=-zstack-size=8388608");
    // A relative call to rust_dealloc in regex-syntax drop glue can contain
    // WRPKRU bytes inside its displacement. Honor the trace's existing layout
    // retries, as Play/Web do, without changing the loader's byte policy.
    println!("cargo:rerun-if-env-changed=SEX_LAYOUT_SEED");
    if let Some(seed) = std::env::var("SEX_LAYOUT_SEED")
        .ok()
        .filter(|s| !s.is_empty() && s != "0")
    {
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.rodata*={seed}");
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.text*={seed}");
    }
}
