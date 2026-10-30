//! Quil workspace exchange on the pre-existing BOOT storage route.
//! Slot numbers select data inside that grant; they never convey authority.
pub const OP: u64 = 0x4D;
pub const INFO: u8 = 1;
pub const LOAD: u8 = 2;
pub const READ: u8 = 3;
pub const NAME: u8 = 4;
pub const END: u8 = 5;
pub const BEGIN_SAVE: u8 = 6;
pub const WRITE: u8 = 7;
pub const COMMIT: u8 = 8;
pub const CREATE: u8 = 9;
pub const ABORT: u8 = 10;
/// LOAD.b selects coherent transfer through an existing BOOT snapshot Region.
pub const SNAPSHOT_MODE: u64 = 1;
pub const MAX: usize = 12288;
/// Maximum admitted data requests per exact BOOT client, not an authority grant.
pub const WINDOW: usize = 4;
pub const SEQ_MAX: u32 = 0xFF_FFFF;
pub const INVALID: u32 = 1;
pub const DENIED: u32 = 2;
pub const STALE: u32 = 3;
pub const BUSY: u32 = 5;
pub const TIMEOUT: u32 = 8;
pub const IO: u32 = 9;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub seq: u32,
    pub kind: u8,
    pub object: u8,
    pub offset: u32,
    pub a: u64,
    pub b: u64,
}
impl Request {
    pub fn header(self) -> u64 {
        self.seq as u64
            | ((self.kind as u64) << 24)
            | ((self.offset as u64) << 32)
            | ((self.object as u64) << 56)
    }
    pub fn decode(h: u64, a: u64, b: u64) -> Option<Self> {
        let r = Self {
            seq: (h & 0xFF_FFFF) as u32,
            kind: (h >> 24) as u8,
            offset: ((h >> 32) & 0xFF_FFFF) as u32,
            object: (h >> 56) as u8,
            a,
            b,
        };
        if r.seq == 0 || !(INFO..=ABORT).contains(&r.kind) {
            return None;
        }
        Some(r)
    }
}
pub fn reply(seq: u32, result: Result<u32, u32>) -> u64 {
    let (t, v) = match result {
        Ok(v) => (0x55u64, v),
        Err(e) => (0xE5u64, e),
    };
    (t << 56) | ((seq as u64) << 32) | v as u64
}
pub fn decode_reply(v: u64, seq: u32) -> Option<Result<u32, u32>> {
    if (v >> 32) & 0xFF_FFFF != seq as u64 {
        return None;
    }
    match v >> 56 {
        0x55 => Some(Ok(v as u32)),
        0xE5 => Some(Err(v as u32)),
        _ => None,
    }
}
pub fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}
