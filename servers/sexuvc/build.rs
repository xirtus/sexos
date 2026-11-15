fn main() {
    println!("cargo:rerun-if-env-changed=SEX_LAYOUT_SEED");
    if let Some(seed) = std::env::var("SEX_LAYOUT_SEED")
        .ok()
        .filter(|s| !s.is_empty() && s != "0")
    {
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.rodata*={seed}");
        println!("cargo:rustc-link-arg-bins=--shuffle-sections=.text*={seed}");
    }
}
