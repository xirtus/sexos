// SPDX-License-Identifier: GPL-3.0-only
//! Partition type GUIDs Corset names, and the GPT attribute bits it sets.
use sex_disk::gpt::{self, Guid};

/// FAT, exFAT and NTFS volumes other systems mount (what Windows, macOS and GNOME Disks
/// give a data partition).
pub const BASIC_DATA: Guid = Guid::new(0xEBD0A0A2, 0xB9E5, 0x4433, [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7]);
pub const LINUX_FS: Guid = Guid::new(0x0FC63DAF, 0x8483, 0x4772, [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4]);
pub const LINUX_SWAP: Guid = Guid::new(0x0657FD6D, 0xA4AB, 0x43C4, [0x84, 0xE5, 0x09, 0x33, 0xC8, 0x4B, 0x4F, 0x4F]);
pub const LINUX_LUKS: Guid = Guid::new(0xCA7D7CCB, 0x63ED, 0x4C53, [0x86, 0x1C, 0x17, 0x42, 0x53, 0x60, 0x59, 0xCC]);
pub const MS_RESERVED: Guid = Guid::new(0xE3C9E316, 0x0B5C, 0x4DB8, [0x81, 0x7D, 0xF9, 0x2D, 0xF0, 0x02, 0x15, 0xAE]);
pub const MS_RECOVERY: Guid = Guid::new(0xDE94BBA4, 0x06D1, 0x4D40, [0xA1, 0x6A, 0xBF, 0xD5, 0x01, 0x79, 0xD6, 0xAC]);
pub const APPLE_APFS: Guid = Guid::new(0x7C3457EF, 0x0000, 0x11AA, [0xAA, 0x11, 0x00, 0x30, 0x65, 0x43, 0xEC, 0xAC]);
/// A sexfs volume of its own (not the system store, whose layout the disk driver knows).
pub const SEXFS: Guid = Guid::new(0x472765be, 0xf668, 0x4105, [0xb6, 0xb8, 0xb5, 0xda, 0x8d, 0x94, 0xa7, 0xa8]);
/// A sex-crypt (SEXCRYPT header, XTS-AES-256) encrypted volume.
pub const SEXCRYPT: Guid = Guid::new(0x8b5d3845, 0x4177, 0x455e, [0xb6, 0xc9, 0xe1, 0x44, 0x28, 0xfa, 0xc0, 0x05]);
pub use gpt::{TYPE_ESP as ESP, TYPE_SEXOS_BOOT as SEXOS_BOOT, TYPE_SEXOS_DATA as SEXOS_STORE};

/// Types Corset names, for its type menu and its listing.
pub const KNOWN: &[(Guid, &str)] = &[
    (BASIC_DATA, "Basic data (FAT, exFAT, NTFS)"),
    (SEXFS, "SexOS filesystem"),
    (SEXCRYPT, "SexOS encrypted"),
    (gpt::TYPE_ESP, "EFI System"),
    (LINUX_FS, "Linux filesystem"),
    (LINUX_SWAP, "Linux swap"),
    (LINUX_LUKS, "Linux LUKS"),
    (MS_RESERVED, "Microsoft reserved"),
    (MS_RECOVERY, "Windows recovery"),
    (APPLE_APFS, "Apple APFS"),
    (gpt::TYPE_SEXOS_BOOT, "SexOS system slot"),
    (gpt::TYPE_SEXOS_DATA, "SexOS store"),
];

pub fn name(t: Guid) -> &'static str {
    KNOWN.iter().find(|(g, _)| *g == t).map_or("Unknown type", |(_, n)| n)
}

/// Types whose partitions belong to a running system: changing them is refused.
pub fn is_system(t: Guid) -> bool { t == gpt::TYPE_SEXOS_BOOT || t == gpt::TYPE_SEXOS_DATA }

/// GPT attribute bits (UEFI 2.10 §5.3.3, and the Microsoft basic data bits).
pub mod attr {
    /// Platform required: firmware must not touch or delete it.
    pub const REQUIRED: u64 = 1 << 0;
    /// No EFI Block IO protocol for it.
    pub const NO_BLOCK_IO: u64 = 1 << 1;
    /// Legacy BIOS bootable.
    pub const LEGACY_BOOT: u64 = 1 << 2;
    /// Basic data: read only.
    pub const READ_ONLY: u64 = 1 << 60;
    /// Basic data: shadow copy.
    pub const SHADOW_COPY: u64 = 1 << 61;
    pub const HIDDEN: u64 = 1 << 62;
    pub const NO_AUTOMOUNT: u64 = 1 << 63;
    /// Bits Corset lets a person set; 3..=47 are reserved by UEFI.
    pub const SETTABLE: u64 = REQUIRED | NO_BLOCK_IO | LEGACY_BOOT | READ_ONLY | SHADOW_COPY | HIDDEN | NO_AUTOMOUNT | (0xFFF << 48);
    pub const NAMES: &[(u64, &str)] = &[(REQUIRED, "required"), (NO_BLOCK_IO, "no-block-io"), (LEGACY_BOOT, "legacy-boot"),
        (READ_ONLY, "read-only"), (SHADOW_COPY, "shadow-copy"), (HIDDEN, "hidden"), (NO_AUTOMOUNT, "no-automount")];
}
