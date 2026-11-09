// Asks the loader for a 2 MiB stack: layout recurses per nesting level, and page
// scripts call back into it.
fn main() { println!("cargo:rustc-link-arg-bins=-zstack-size=2097152"); }
