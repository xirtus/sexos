#![no_std]

extern crate alloc;
extern crate spin;

pub mod pdx;
pub mod vfs;
pub mod spindle_history;
mod spindle_probe;
mod document;
mod document_journal;
pub mod history_journal;
pub mod history_disk;
pub mod messages;
pub mod trampoline;
pub mod backends;

/// RamFS proof module: built-in contract validation tests.
/// Compile with SEXFILES_RAMFS_PROOF=1 to enable startup proof.
pub mod proof;
pub mod sexobject;
