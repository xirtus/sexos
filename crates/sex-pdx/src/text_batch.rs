//! Register-only immutable text replacement. Surface numbers are data, never authority.
//! The display must independently check the actual sender, live incarnation and owner.
pub const BEGIN: u64 = 0x276;
pub const CHUNK: u64 = 0x277;
pub const COMMIT: u64 = 0x278;
pub const CAPACITY: usize = 512;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Begin {
        sequence: u32,
        len: usize,
        color: u32,
    },
    Chunk {
        sequence: u32,
        offset: usize,
        count: usize,
        bytes: [u8; 8],
    },
    Commit {
        sequence: u32,
        hash: u64,
    },
}
impl Request {
    pub fn decode(op: u64, a: u64, b: u64) -> Option<Self> {
        match op {
            BEGIN if a >> 48 == 0 && b >> 32 == 0 => {
                let sequence = a as u32;
                let len = (a >> 32) as usize;
                (sequence != 0 && len <= CAPACITY).then_some(Self::Begin {
                    sequence,
                    len,
                    color: b as u32 | 0xff000000,
                })
            }
            CHUNK if b & 0x00000000ff000000 == 0 => {
                let sequence = (b >> 32) as u32;
                let offset = ((b >> 8) & 0xffff) as usize;
                let count = (b & 255) as usize;
                let bytes = a.to_le_bytes();
                (sequence != 0
                    && offset < CAPACITY
                    && offset % 8 == 0
                    && (1..=8).contains(&count)
                    && bytes[count..].iter().all(|&v| v == 0)
                    && bytes[..count]
                        .iter()
                        .all(|&v| v == b'\n' || (32..=126).contains(&v)))
                .then_some(Self::Chunk {
                    sequence,
                    offset,
                    count,
                    bytes,
                })
            }
            COMMIT if a != 0 && a <= u32::MAX as u64 => Some(Self::Commit {
                sequence: a as u32,
                hash: b,
            }),
            _ => None,
        }
    }
    pub fn wire(self) -> (u64, u64, u64) {
        match self {
            Self::Begin {
                sequence,
                len,
                color,
            } => (BEGIN, sequence as u64 | (len as u64) << 32, color as u64),
            Self::Chunk {
                sequence,
                offset,
                count,
                bytes,
            } => (
                CHUNK,
                u64::from_le_bytes(bytes),
                (sequence as u64) << 32 | (offset as u64) << 8 | count as u64,
            ),
            Self::Commit { sequence, hash } => (COMMIT, sequence as u64, hash),
        }
    }
}
pub fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, &v| {
        (h ^ v as u64).wrapping_mul(0x100000001b3)
    })
}

/// Text state was atomically applied by display. This is not a pixel receipt.
pub const APPLIED_TAG: u8 = 0x56;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Applied {
    pub surface: u16,
    pub sequence: u32,
}
impl Applied {
    pub fn encode(self) -> Option<u64> {
        (self.surface != 0 && self.sequence != 0).then_some(
            APPLIED_TAG as u64 | (self.surface as u64) << 8 | (self.sequence as u64) << 24,
        )
    }
    pub fn decode(caller: u32, value: u64) -> Option<Self> {
        if caller != 1 || value as u8 != APPLIED_TAG || value >> 56 != 0 {
            return None;
        }
        let result = Self {
            surface: (value >> 8) as u16,
            sequence: (value >> 24) as u32,
        };
        result.encode().filter(|&v| v == value).map(|_| result)
    }
}
