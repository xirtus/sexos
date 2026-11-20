// SPDX-License-Identifier: GPL-3.0-only
// Driver sources included by #[path] swap their register access for the
// harness's device model under this cfg (see servers/sexbell/src/hda.rs).
fn main() { println!("cargo:rustc-cfg=device_model"); println!("cargo::rustc-check-cfg=cfg(device_model)"); }
