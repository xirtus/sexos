// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! What Tatami's Updates, Storage and About pages show: one read-only Files request (same
//! wire and region as [`crate::fs`]) that only the system's `tatami` program may send,
//! the identity Files already trusts with settings (`fs_service::program_is`). It changes
//! nothing: no rollback, no package change, no mount.
//!
//! `offset` names the topic; the reply's value is the length of `key=value` lines in the
//! data area. A key Files cannot answer is left out, never guessed; Tatami says so.
//!
//! - [`UPDATES`]: `slot=a|b|none` (none: started from a stick or CD, not an installed
//!   system), `version=N` (the last system that passed its trial; 0 as installed),
//!   `candidate=N`, `trial=none|pending|started|failed`, `other=1|0` (the other slot
//!   holds a system the boot menu can start), `release_key=1|0` (this build takes signed
//!   updates), `packages=N` (updates the kept catalogs offer; left out with no catalog),
//!   then one `package=NAME VERSION` line each.
//! - [`STORAGE`]: `disk=SECTORS LAYOUT` (`LAYOUT`: whole, store, foreign, locked,
//!   unknown), `filesystem=sexfs TOTAL FREE` (4 KiB blocks), `home=none|locked|unlocked`
//!   (the encrypted home, XTS), one `volume=FORMAT SECTORS FREE SOURCE RO NAME` per volume
//!   under `/media` (`FREE` in 4 KiB blocks, `-` if the volume did not say; `SOURCE`:
//!   image, usb, disk), and `health=none` while no driver reads the disk's health log.
//! - [`ABOUT`]: `slot=` and `version=` as above, `image_bytes=N` and `image_sha256=HEX` of
//!   the boot image the running system came with (the hash a release index names).

pub const KIND: u8 = 36;
pub const UPDATES: u64 = 1;
pub const STORAGE: u64 = 2;
pub const ABOUT: u64 = 3;
/// The one program that may ask.
pub const PROGRAM: &[u8] = b"tatami";

// Files reads one request-kind byte from every client: this one lands on no other.
const _: () = {
    let others = [crate::fs::RUNS, crate::fs::MOVE_IF_ID, crate::package::KIND, crate::console_relay::KIND, crate::restricted_job::KIND, crate::home_lock::CHANGE];
    let mut i = 0;
    while i < others.len() { assert!(others[i] != KIND); i += 1; }
    assert!(KIND < crate::disks::DISKS || KIND > crate::disks::DISK_ANSWER);
};
