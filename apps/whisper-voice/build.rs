// eSpeak NG keeps large buffers on the stack while it translates a clause.
fn main() { println!("cargo:rustc-link-arg-bins=-zstack-size=8388608"); }
