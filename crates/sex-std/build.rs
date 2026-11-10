// The time this crate was built, for `SystemTime` until programs have a civil clock
// (see src/sys.rs). SOURCE_DATE_EPOCH wins, for reproducible builds.
fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let now = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    });
    println!("cargo:rustc-env=SEX_STD_BUILT={now}");
}
