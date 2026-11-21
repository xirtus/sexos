// SPDX-License-Identifier: GPL-3.0-only
//! Startup programs (Settings' Startup programs page, `startup.programs`).
//!
//! Once per boot, when the Shell holds the person's settings and the screen is not
//! locked, it starts each chosen program from the program store by name, one at a
//! time, as `run NAME` does with no filesystem ticket: the run gets its program row
//! and nothing more (no network, no grants it was not given). A Shell started again
//! after a crash starts nothing: the programs from before are still running.
use sex_config::{id, PROGRAMS};

/// The next program to try; PROGRAMS.len() when done.
static mut NEXT: usize = 0;
static mut DONE: bool = false;

/// From the main loop.
pub unsafe fn poll() {
    if DONE || !crate::config_watch::current() || crate::lock::locked() { return; }
    if sex_pdx::crash_log::restarts() > 0 { DONE = true; sex_pdx::serial_record!("[shell.startup.skipped] reason=restarted"); return; }
    while NEXT < PROGRAMS.len() && !crate::config_watch::holds(id::STARTUP, NEXT) { NEXT += 1; }
    let Some(name) = PROGRAMS.get(NEXT) else {
        DONE = true;
        sex_pdx::serial_record!("[shell.startup.done] chosen={}", crate::config_watch::value(id::STARTUP).count_ones());
        return;
    };
    let Some(packed) = sex_pdx::program_store::pack_name(name.as_bytes()) else { NEXT += 1; return };
    // Another start is under way (a person's, or the last of these): try again later.
    if !crate::program_start_own(packed) { return; }
    sex_pdx::serial_record!("[shell.startup.run] program={}", name);
    NEXT += 1;
}
