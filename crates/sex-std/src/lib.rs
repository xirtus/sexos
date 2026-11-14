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
//! What std's Hermit port refuses on its own (`rename`, `File::set_len`,
//! `File::sync_all`, `remove_dir_all`, `canonicalize`, a working `copy`) is in
//! [`fs`]; why `std::process::Command` cannot start another program yet is in
//! [`process`].
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
#[cfg(target_os = "hermit")]
pub mod fs;
#[cfg(target_os = "hermit")]
pub mod process;

pub use sex_app;
pub use sex_pdx;

/// The terminal this program's console is: its size, a raw or cooked keyboard, echo,
/// and Ctrl+C for a program that never reads or writes. A run started in Spindle's
/// terminal asks it through Files (`sex_pdx::console_relay`); a program in its own window
/// gets that window's size and keeps its cooked line.
#[cfg(target_os = "hermit")]
pub mod terminal {
    use sex_pdx::console_relay as c;
    /// Columns and rows (80 by 24 until the terminal says).
    pub fn size() -> (u16, u16) { let (cols, rows, _) = crate::console::size(); (cols, rows) }
    /// A count that moves on every resize: look now and then to notice one (where a Unix
    /// program would wait for SIGWINCH).
    pub fn size_changes() -> u32 { crate::console::size().2 }
    /// Keys as they are typed, unbuffered, without the terminal's echo, with Ctrl+C a byte
    /// (an editor), or back to a cooked line. False without a terminal.
    pub fn set_raw(on: bool) -> bool { crate::console::set_mode(if on { c::RAW | c::NO_ECHO | c::NO_SIGNALS } else { 0 }) }
    /// The terminal shows typed text, or not (a password).
    pub fn set_echo(on: bool) -> bool {
        let m = crate::console::mode();
        crate::console::set_mode(if on { m & !c::NO_ECHO } else { m | c::NO_ECHO })
    }
    /// The mode bits now (`console_relay::RAW`, `NO_ECHO`, `NO_SIGNALS`), and setting them.
    pub fn mode() -> u8 { crate::console::mode() }
    pub fn set_mode(mode: u8) -> bool { crate::console::set_mode(mode) }
    /// Whether Ctrl+C was pressed since the last look. Reading, writing and sleeping look
    /// on their own (and end the run with status 130); a long computation that does none
    /// of them calls this now and then. One that never does is stopped by a second Ctrl+C,
    /// which Spindle's terminal sends to Shell: nothing can interrupt it sooner from outside.
    pub fn interrupted() -> bool { crate::console::interrupted() }
}

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
