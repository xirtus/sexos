//! Offline external-plugin jobs. This boundary contains crashes, not hostile native code.
#![no_std]
extern crate alloc;
pub mod clap;
pub mod elf;
pub mod job;
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
pub mod vst;
