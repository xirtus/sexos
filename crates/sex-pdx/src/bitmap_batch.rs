//! Register-only finite RGBA replacement. Coordinates and surface IDs are data;
//! display independently attests the actual sender, surface owner and incarnation.
pub const BEGIN: u64 = 0x27A;
pub const CHUNK: u64 = 0x27B;
pub const COMMIT: u64 = 0x27C;
pub const CLEAR: u64 = 0x27D;
pub const CAPACITY: usize = 48 * 48;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub x: u16,
    pub y: u16,
    pub width: u8,
    pub height: u8,
    pub scale: u8,
}
impl Geometry {
    pub fn valid(self) -> bool {
        (1..=48).contains(&self.width)
            && (1..=48).contains(&self.height)
            && (1..=4).contains(&self.scale)
    }
    pub fn len(self) -> usize {
        self.width as usize * self.height as usize
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Begin {
        sequence: u32,
        geometry: Geometry,
    },
    Chunk {
        sequence: u32,
        offset: usize,
        count: usize,
        pixels: [u32; 2],
    },
    Commit {
        sequence: u32,
        hash: u64,
    },
    Clear {
        sequence: u32,
    },
}
impl Request {
    pub fn decode(op: u64, a: u64, b: u64) -> Option<Self> {
        match op {
            BEGIN if a >> 56 == 0 && b >> 32 == 0 => {
                let sequence = a as u32;
                let geometry = Geometry {
                    x: b as u16,
                    y: (b >> 16) as u16,
                    width: (a >> 32) as u8,
                    height: (a >> 40) as u8,
                    scale: (a >> 48) as u8,
                };
                (sequence != 0 && geometry.valid()).then_some(Self::Begin { sequence, geometry })
            }
            CHUNK if b & 0xff00_0000 == 0 => {
                let sequence = (b >> 32) as u32;
                let offset = (b & 0xffff) as usize;
                let count = ((b >> 16) & 255) as usize;
                let pixels = [a as u32, (a >> 32) as u32];
                (sequence != 0
                    && offset < CAPACITY
                    && offset % 2 == 0
                    && (1..=2).contains(&count)
                    && pixels[count..].iter().all(|&v| v == 0))
                .then_some(Self::Chunk {
                    sequence,
                    offset,
                    count,
                    pixels,
                })
            }
            COMMIT if a != 0 && a <= u32::MAX as u64 => Some(Self::Commit {
                sequence: a as u32,
                hash: b,
            }),
            CLEAR if a != 0 && a <= u32::MAX as u64 && b == 0 => {
                Some(Self::Clear { sequence: a as u32 })
            }
            _ => None,
        }
    }
    pub fn wire(self) -> (u64, u64, u64) {
        match self {
            Self::Begin {
                sequence,
                geometry: g,
            } => (
                BEGIN,
                sequence as u64
                    | (g.width as u64) << 32
                    | (g.height as u64) << 40
                    | (g.scale as u64) << 48,
                g.x as u64 | (g.y as u64) << 16,
            ),
            Self::Chunk {
                sequence,
                offset,
                count,
                pixels,
            } => (
                CHUNK,
                pixels[0] as u64 | (pixels[1] as u64) << 32,
                (sequence as u64) << 32 | offset as u64 | (count as u64) << 16,
            ),
            Self::Commit { sequence, hash } => (COMMIT, sequence as u64, hash),
            Self::Clear { sequence } => (CLEAR, sequence as u64, 0),
        }
    }
}
/// Byte-order-defined FNV over the complete row-major AARRGGBB pixel words.
/// This checks transaction integrity; it grants no authority or authenticity.
pub fn hash(pixels: &[u32]) -> u64 {
    pixels
        .iter()
        .flat_map(|p| p.to_le_bytes())
        .fold(0xcbf29ce484222325, |h, v| {
            (h ^ v as u64).wrapping_mul(0x100000001b3)
        })
}
/// State publication receipt, never a framebuffer/pixel receipt.
pub const APPLIED_TAG: u8 = 0x57;
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
