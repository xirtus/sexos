// SPDX-License-Identifier: GPL-3.0-only
//! `std::process::Command`: not on SexOS yet, and why.
//!
//! std's Hermit port has no process support at all (library/std/src/sys/process is
//! `unsupported` for Hermit), so `Command::new(..).spawn()`, `.status()` and
//! `.output()` return an error of kind [`std::io::ErrorKind::Unsupported`] without
//! calling this crate. `std::process::exit`, `abort` and `id` work.
//!
//! Starting another program from a program is not something its grants allow today,
//! so there is nothing honest for this crate to offer in its place:
//!
//! * Only Spindle may ask Shell to run a program (`program_store::OP_RUN`, which Shell
//!   accepts from Spindle's domain alone); a program has no route to Shell for it.
//! * A run has no parent: nothing hands its exit status back to whoever started it,
//!   and its standard streams are its own window and the serial record, so there are
//!   no pipes to read `output()` from.
//!
//! What it needs (Shell work, not this library): a grant in a program's BOOT row to
//! ask Shell for a run of a named program with arguments; Shell telling the asker the
//! child's row and task, and its exit status when it finishes; and, for
//! `Stdio::piped`, a byte channel between the two runs that `sys_read`/`sys_write`
//! can carry. With those, a `sex_std::process::Command` would map onto them (std
//! itself would still refuse before reaching this crate).

/// The error `std::process::Command` gives on SexOS, for a program that wants to say
/// so itself before trying.
pub fn unsupported() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, "a SexOS program cannot start another program yet")
}
