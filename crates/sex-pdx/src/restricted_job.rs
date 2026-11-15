//! Bounded immutable offline-job request. Bytes are copied from the client's
//! grant before decoding; decoding alone conveys no file or launch authority.
pub const KIND: u8 = 30;
pub const PREPARE: u64 = 1;
pub const RESULT: u64 = 2;
pub const CANCEL: u64 = 3;
pub const RELEASE: u64 = 4;
pub const BOOT_INFO: u64 = 5;
pub const INFO_BYTES: usize = 176;
pub const HEADER: usize = 48;
pub const PATH_MAX: usize = 192;
pub const MAX: usize = HEADER + 3 * PATH_MAX;
pub const OP_CLAIM: u64 = 0x4d50_5010;
pub const OP_DESCRIBE: u64 = 0x4d50_5011;
pub const OP_LOAD: u64 = 0x4d50_5012;
pub const OP_COPIED: u64 = 0x4d50_5013;
#[derive(Clone, Copy)]
pub struct Prepare<'a> {
    pub format: u8,
    pub cookie: u32,
    pub nonce: [u8; 16],
    pub paths: [&'a [u8]; 3],
}
pub fn path(path: &[u8]) -> bool {
    path.len() >= 3
        && path.len() <= PATH_MAX
        && path.starts_with(b"@/")
        && !path.iter().any(|b| *b <= 32 || *b == 127)
        && !path[2..]
            .split(|b| *b == b'/')
            .any(|p| p.is_empty() || p == b"." || p == b"..")
}
impl<'a> Prepare<'a> {
    pub fn decode(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < HEADER
            || bytes.len() > MAX
            || &bytes[..8] != b"MOANJOB2"
            || bytes[8] > 1
            || bytes[9..12] != [0; 3]
            || bytes[38..48] != [0; 10]
        {
            return None;
        }
        let cookie = u32::from_le_bytes(bytes[12..16].try_into().ok()?);
        let nonce = bytes[16..32].try_into().ok()?;
        if cookie == 0 || nonce == [0; 16] {
            return None;
        }
        let mut at = HEADER;
        let mut paths = [&[][..]; 3];
        for i in 0..3 {
            let n = u16::from_le_bytes(bytes[32 + i * 2..34 + i * 2].try_into().ok()?) as usize;
            paths[i] = bytes.get(at..at.checked_add(n)?)?;
            if !path(paths[i]) {
                return None;
            }
            at += n;
        }
        if at != bytes.len() || paths[0] == paths[1] || paths[0] == paths[2] || paths[1] == paths[2]
        {
            return None;
        }
        Some(Self {
            format: bytes[8],
            cookie,
            nonce,
            paths,
        })
    }
    pub fn encode(&self, bytes: &mut [u8]) -> Option<usize> {
        if self.format > 1
            || self.cookie == 0
            || self.nonce == [0; 16]
            || self.paths.iter().any(|p| !path(p))
            || self.paths[0] == self.paths[1]
            || self.paths[0] == self.paths[2]
            || self.paths[1] == self.paths[2]
        {
            return None;
        }
        let n = HEADER.checked_add(self.paths.iter().map(|p| p.len()).sum::<usize>())?;
        if n > MAX || bytes.len() < n {
            return None;
        }
        bytes[..n].fill(0);
        bytes[..8].copy_from_slice(b"MOANJOB2");
        bytes[8] = self.format;
        bytes[12..16].copy_from_slice(&self.cookie.to_le_bytes());
        bytes[16..32].copy_from_slice(&self.nonce);
        let mut at = HEADER;
        for i in 0..3 {
            bytes[32 + i * 2..34 + i * 2]
                .copy_from_slice(&(self.paths[i].len() as u16).to_le_bytes());
            bytes[at..at + self.paths[i].len()].copy_from_slice(self.paths[i]);
            at += self.paths[i].len();
        }
        Some(n)
    }
}
