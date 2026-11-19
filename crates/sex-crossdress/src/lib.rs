// SPDX-License-Identifier: GPL-3.0-only
//! Crossdress: Windows programs on SexOS, the Drag-cell way
//! (`docs/handoff/WINDOWS_COMPAT_STUDY_2026_11_16.md`, `docs/handoff/CROSSDRESS_2026_11_16.md`).
//!
//! A Windows program runs unmodified inside a Drag cell: a VT-x guest (SexOS syscall 110)
//! with its own page tables and PKRU and no guest operating system. The PE code never
//! runs in SexOS's single address space, so the loader's WRPKRU byte check, fixed image
//! bases and the low 4 GiB stop being problems. Everything a Windows kernel would do is
//! done by this crate in the host program, on VM exits.
//!
//! - [`pe`]: PE32+ parsing, layout, relocations, imports and exports.
//! - [`cell`]: the cell's memory, page tables, GDT/IDT/TSS and ring-0 exception stubs.
//! - [`process`]: one process and thread, import thunks, the exit loop, vectored
//!   exception dispatch.
//! - [`win32`]: Windows structure layouts, status codes and the KERNEL32 subset.
//! - [`nt`]: Wine's ntdll.dll in a cell: KUSER_SHARED_DATA, the syscall and Unix call
//!   dispatchers, and the NT system calls served by the host (CW3).
//! - [`wine`]: a Windows program started the Wine way (LdrInitializeThunk) with Wine's
//!   own loader, kernel32 and kernelbase (CW3b).
//! - [`clap`]: a Windows CLAP plugin DLL in a cell, driven by the host (the yabridge
//!   role): descriptor, parameters, activate, sample-accurate `process`.
//!
//! Host-testable: the kernel is reached only through `drag_vmm::vmm::Vcpu`, and
//! `tests/kvm.rs` runs the same cell on Linux KVM where /dev/kvm exists.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod cell;
pub mod clap;
pub mod nt;
pub mod pe;
pub mod process;
pub mod win32;
pub mod wine;

/// The CW1 test program (`test/hello.c`, built by `test/build.sh`): a real 64-bit
/// Windows console program that prints, checks its TEB/PEB, allocates memory and
/// survives four hardware exceptions through a vectored exception handler.
pub static TEST_EXE: &[u8] = include_bytes!("../test/hello.exe");

/// The line the test program prints last when every check passed.
pub const TEST_PASS: &str = "crossdress: all checks passed";

/// The plugin test (`test/gain.c`, built by `test/build_gain.sh`): a real 64-bit Windows
/// CLAP audio plugin DLL, "Crossdress Test Gain".
pub static TEST_CLAP: &[u8] = include_bytes!("../test/gain.clap");

/// The CW3 test program (`test/wine_hello.c`, built by `test/build_wine_hello.sh`): runs
/// on Wine's own KERNEL32, loaded by Wine's ntdll in the cell.
pub static WINE_HELLO_EXE: &[u8] = include_bytes!("../test/wine_hello.exe");

/// The line the Wine test program prints last when every check passed.
pub const WINE_PASS: &str = "crossdress: wine checks passed";
