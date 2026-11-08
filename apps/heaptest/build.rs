// Asks the loader for a 4 MiB stack; the heap gate checks it is honoured.
fn main() { println!("cargo:rustc-link-arg-bins=-zstack-size=4194304"); }
