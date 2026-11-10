// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Organ systems: a PKU hardware key belongs to a group of trusted services, not
//! to one program. Sharing costs those services isolation from each other, never
//! authority, which capabilities still decide. It leaves keys for what needs
//! isolating most: programs nobody has vetted.

/// Key for a BOOT service, by its fixed spawn position.
pub const fn service_key(domain: u8) -> u8 {
    match domain {
        5 => 4,  // Senses: sexusb with sexinput.
        6 => 1,  // Core: silkbar with sexdisplay.
        8 => 11, // Storage: sexstore with sexfiles.
        10 => 4, // Senses: sexbell with sexinput and sexusb.
        other => other,
    }
}

/// Program records all hold this key. Only the running record's pages are present,
/// so sharing it costs them nothing against each other.
pub const RECORD_KEY: u8 = 10;
/// Guests: each program row runs on a key no service holds, and no other live
/// row holds. Row 0 only anchors the chain and is gone before row 1 starts.
/// Rows from the third on share this key and take turns being present.
pub const SHARED_PROGRAM_KEY: u8 = 8;
pub const fn program_key(ordinal: u8) -> u8 {
    match ordinal {
        0 | 1 => 5,
        2 => 6,
        _ => SHARED_PROGRAM_KEY,
    }
}
