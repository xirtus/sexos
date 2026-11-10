// SPDX-License-Identifier: GPL-3.0-only
//! Rust `std` for SexOS programs.
//!
//! A program built for `x86_64-sex-std.json` links the real Rust standard library:
//! `println!`, `std::fs`, `std::thread`, `std::sync`, `std::time`, `HashMap` and the
//! crates that need them build unchanged. This crate answers the calls std makes.
//!
//! How: std already has a port for Hermit, a unikernel whose system interface is a
//! library ABI (`sys_read`, `sys_spawn2`, `sys_futex_wait`, ...) rather than traps into
//! a POSIX kernel. That is the shape SexOS needs too, so the target names Hermit's
//! interface and this crate provides it, inside the program, over the program's own
//! grants: no POSIX in the kernel or in any service, and no authority beyond the
//! program's BOOT row. What each call becomes:
//!
//! * files: Files, in the program's own folder, which is `/` to the program
//!   (`sex_app::files::direct`). `..` never leaves it.
//! * stdout and stderr: the program's window as a console, and the serial record.
//!   stdin: lines typed into that window.
//! * threads: the program's workers (`sex_app::thread`), on its own key and cores;
//!   `Mutex`, `Condvar` and parking: the kernel's wait and wake on an address.
//! * time: the kernel's tick for `Instant`, its civil time (UTC) for `SystemTime`.
//! * memory: the program's heap (`sex_app::heap`), from its own window.
//! * sockets: the program's network flows (`sex_app::net::direct`), for a run started
//!   with `run +net`: TCP connections and listeners, UDP, and name lookup.
//!
//! A program uses it with `use sex_std as _;` (or calls [`window`] for pixels) and a
//! normal `fn main`.
#![cfg_attr(target_os = "hermit", feature(thread_local, rustc_private))]

pub mod path;
pub mod args;
#[cfg(target_os = "hermit")]
mod console;
#[cfg(target_os = "hermit")]
pub mod sys;
#[cfg(target_os = "hermit")]
mod net;

pub use sex_app;
pub use sex_pdx;

/// The program's window, for a program that draws or reads keys itself. After the
/// first call, standard output still goes to the serial record but no longer replaces
/// what the window shows. Call it from one task.
#[cfg(target_os = "hermit")]
pub fn window() -> &'static mut sex_app::Window { console::take_window() }

/// This run's program row (target) and task, as the serial record names them.
#[cfg(target_os = "hermit")]
pub fn identity() -> (u32, u32) {
    sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT).map_or((0, 0), |i| (i.target_pd, i.task_id))
}

/// Records a line in the serial record (the system log gates and crash reports read).
#[cfg(target_os = "hermit")]
pub fn record(line: &str) { sex_pdx::serial_record!("{}", line); }
