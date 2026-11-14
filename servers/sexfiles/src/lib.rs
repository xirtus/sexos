#![no_std]

extern crate alloc;
extern crate spin;

mod defer_queue;
pub mod pdx;
pub mod vfs;
pub mod spindle_history;
mod spindle_probe;
mod program_store;
mod fs_service;
mod config_store;
mod password_store;
mod recording_role;
mod recording_image;
mod recording_journal;
mod recording_runtime;
mod installer;
mod updater;
mod packages;
mod volumes;
mod home;
mod document;
mod document_journal;
pub mod history_journal;
pub mod history_disk;
pub mod crash_keeper;
pub mod messages;
pub mod trampoline;
pub mod backends;

/// RamFS proof module: built-in contract validation tests.
/// Compile with SEXFILES_RAMFS_PROOF=1 to enable startup proof.
pub mod proof;
pub mod sexobject;
