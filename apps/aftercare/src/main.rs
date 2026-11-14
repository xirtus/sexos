// SPDX-License-Identifier: GPL-3.0-only
//! Aftercare: backups. It keeps encrypted, deduplicated snapshots of one folder in a
//! repository on a USB disk (under `/media`) or in any folder the person gives it, and
//! restores any file or folder from any snapshot. The engine is crates/aftercare-core.
//!
//! `run aftercare @/home/ann @/media/USB` gives it those two folders for this run (a
//! program sees only what it is given); the choices are kept in `aftercare.conf` in its
//! own folder.
//!
//! Keys (home): S source, D destination, U unlock or create the repository (password),
//! B back up now (Esc cancels), L snapshots (Enter browses, Delete forgets one; in a
//! snapshot R restores the highlighted file or folder to where it was, T to another
//! folder), K keep policy, F forget by that policy and prune, C check (Shift+C reads and
//! verifies all data), T the daily backup time (UTC), X clear stale locks, F11 closes.
//!
//! The daily backup runs while Aftercare is open and unlocked at that time; a scheduler
//! that starts it in the background is a later slice, as are SFTP destinations and a
//! sexfs snapshot before updates.
// Off SexOS only the host tests use the modules.
#![cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod rng;
mod settings;
mod store;
#[cfg(target_os = "hermit")]
mod ui;

fn main() {
    #[cfg(target_os = "hermit")]
    ui::main();
    #[cfg(not(target_os = "hermit"))]
    println!("Aftercare runs on SexOS: build it for x86_64-sex-std.json.");
}
