// What this system may do with the disk, decided once at start by `sex_disk::probe`
// (see `probe_layout` and `place` in main.rs). Plain data, so host tests can include it.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lens {
    /// Not probed yet: only the probe's own raw reads run.
    Unknown,
    /// Blank, or SexOS's legacy whole-disk install.
    Whole,
    /// A GPT disk shared with other systems: SexOS reads and writes only its store.
    Store { base: u64, sectors: u64 },
    /// Another system's disk (one of the FOREIGN_* reasons). Nothing but the installer's
    /// raw requests reach it.
    Foreign(u8),
    /// The installer rewrote the layout under the running system: its old view may still
    /// be read, never written, until the next start.
    Locked { base: u64 },
}
pub const FOREIGN_GPT: u8 = 1;
pub const FOREIGN_MBR: u8 = 2;
pub const FOREIGN_FS: u8 = 3;
pub const FOREIGN_DAMAGED: u8 = 4;
pub static mut LENS: Lens = Lens::Unknown;
/// Set for the length of one installer request (BLOCK_RAW_READ / BLOCK_RAW_WRITE).
pub static mut RAW: bool = false;
