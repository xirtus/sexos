//! Offline external-plugin jobs. This boundary contains crashes, not hostile native code.
#![no_std]
extern crate alloc;
pub mod clap;
pub mod elf;
pub mod imports;
pub mod job;
pub mod ladspa;
pub mod lv2;
// Turtle manifest + preset parsing for Moan's plugin catalog (no native code is loaded by the app).
pub mod lv2_turtle;
#[cfg(all(
    feature = "sexos",
    target_arch = "x86_64",
    any(target_os = "none", target_os = "hermit")
))]
pub mod mapped;
#[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "hermit")))]
pub mod nonce;
#[cfg(all(
    feature = "sexos",
    target_arch = "x86_64",
    any(target_os = "none", target_os = "hermit")
))]
pub mod supervisor;
pub mod native;
pub mod ttl;
pub mod vst;
