// SPDX-License-Identifier: GPL-3.0-only
//! Error types for Corset (Disks and partitions engine).

use core::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoErrorKind {
    OutOfBounds,
    MisalignedBuffer,
    DeviceFailure,
    ReadOnly,
    FlushFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoError {
    pub kind: IoErrorKind,
    pub lba: u64,
    pub sectors: u64,
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "I/O error {:?} at LBA {} ({} sectors)",
            self.kind, self.lba, self.sectors
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CorsetError {
    Io(IoError),
    InvalidParameter(&'static str),
    NoSpace(&'static str),
    AlreadyExists(&'static str),
    NotFound(&'static str),
    MbrCorrupt(&'static str),
    GptCorrupt(&'static str),
    PartitionOverlap {
        first: u64,
        last: u64,
        existing_first: u64,
        existing_last: u64,
    },
    PartitionShrinkTooSmall {
        requested: u64,
        minimum: u64,
    },
    UnsupportedSectorSize(u32),
    VolumeTooSmallForFat32 {
        sectors: u64,
        required: u64,
    },
    VolumeTooSmallForExFat {
        sectors: u64,
        required: u64,
    },
    FilesystemCorrupt(&'static str),
    FilesystemMountFailed(&'static str),
    SmartDataCorrupt(&'static str),
    CryptNotEncrypted,
    CryptWrongPassword,
    CryptCorrupt(&'static str),
    CryptNoMemory,
}

impl fmt::Display for CorsetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{}", e),
            Self::InvalidParameter(msg) => write!(f, "Invalid parameter: {}", msg),
            Self::NoSpace(msg) => write!(f, "No space: {}", msg),
            Self::AlreadyExists(msg) => write!(f, "Already exists: {}", msg),
            Self::NotFound(msg) => write!(f, "Not found: {}", msg),
            Self::MbrCorrupt(msg) => write!(f, "MBR corrupt: {}", msg),
            Self::GptCorrupt(msg) => write!(f, "GPT corrupt: {}", msg),
            Self::PartitionOverlap { first, last, existing_first, existing_last } => write!(
                f,
                "Partition [{}..{}] overlaps existing partition [{}..{}]",
                first, last, existing_first, existing_last
            ),
            Self::PartitionShrinkTooSmall { requested, minimum } => write!(
                f,
                "Partition shrink to {} sectors is below minimum {}",
                requested, minimum
            ),
            Self::UnsupportedSectorSize(sz) => write!(f, "Unsupported sector size: {}", sz),
            Self::VolumeTooSmallForFat32 { sectors, required } => write!(
                f,
                "Volume ({} sectors) is too small for FAT32 (minimum {} sectors)",
                sectors, required
            ),
            Self::VolumeTooSmallForExFat { sectors, required } => write!(
                f,
                "Volume ({} sectors) is too small for exFAT (minimum {} sectors)",
                sectors, required
            ),
            Self::FilesystemCorrupt(msg) => write!(f, "Filesystem corrupt: {}", msg),
            Self::FilesystemMountFailed(msg) => write!(f, "Filesystem mount failed: {}", msg),
            Self::SmartDataCorrupt(msg) => write!(f, "SMART data corrupt: {}", msg),
            Self::CryptNotEncrypted => write!(f, "Volume is not an encrypted BDSM volume"),
            Self::CryptWrongPassword => write!(f, "Incorrect passphrase for encrypted volume"),
            Self::CryptCorrupt(msg) => write!(f, "Encrypted volume header corrupt: {}", msg),
            Self::CryptNoMemory => write!(f, "Not enough memory for key derivation"),
        }
    }
}

impl From<sex_crypt::Error> for CorsetError {
    fn from(e: sex_crypt::Error) -> Self {
        match e {
            sex_crypt::Error::NotEncrypted => Self::CryptNotEncrypted,
            sex_crypt::Error::WrongPassword => Self::CryptWrongPassword,
            sex_crypt::Error::Corrupt => Self::CryptCorrupt("corrupt header or invalid parameters"),
            sex_crypt::Error::NoMemory => Self::CryptNoMemory,
        }
    }
}

impl From<sex_volumes::Error> for CorsetError {
    fn from(e: sex_volumes::Error) -> Self {
        match e {
            sex_volumes::Error::Io => Self::Io(IoError {
                kind: IoErrorKind::DeviceFailure,
                lba: 0,
                sectors: 0,
            }),
            sex_volumes::Error::Corrupt => Self::FilesystemCorrupt("on-disk structure corrupt"),
            sex_volumes::Error::NotFound => Self::NotFound("file or directory not found"),
            sex_volumes::Error::Exists => Self::AlreadyExists("file or directory exists"),
            sex_volumes::Error::NoSpace => Self::NoSpace("filesystem full"),
            sex_volumes::Error::ReadOnly => Self::Io(IoError {
                kind: IoErrorKind::ReadOnly,
                lba: 0,
                sectors: 0,
            }),
            sex_volumes::Error::TooBig => Self::InvalidParameter("filesystem size too big"),
            _ => Self::FilesystemMountFailed("unsupported volume feature or format"),
        }
    }
}

impl From<sex_disk::Error> for CorsetError {
    fn from(e: sex_disk::Error) -> Self {
        match e {
            sex_disk::Error::Io => Self::Io(IoError {
                kind: IoErrorKind::DeviceFailure,
                lba: 0,
                sectors: 0,
            }),
            sex_disk::Error::Corrupt => Self::GptCorrupt("disk structure corrupt"),
            sex_disk::Error::Unsupported => Self::InvalidParameter("unsupported disk format"),
            sex_disk::Error::NoSpace => Self::NoSpace("no space on disk"),
            sex_disk::Error::NotFound => Self::NotFound("partition or table not found"),
            sex_disk::Error::Exists => Self::AlreadyExists("partition exists"),
            sex_disk::Error::Invalid => Self::InvalidParameter("invalid disk parameter"),
            _ => Self::GptCorrupt("unknown disk error"),
        }
    }
}
