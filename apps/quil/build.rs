// The main task's 1 MiB stack comes from the ELF (`-z stack-size`): the kernel commits
// it page by page as it is touched, instead of a 1 MiB heap block zeroed at every start.
fn main() { println!("cargo:rustc-link-arg-bins=-zstack-size=1048576"); }
