// An ordinary program's stack, as on Linux.
fn main() { println!("cargo:rustc-link-arg-bins=-zstack-size=8388608"); }
