//! Bounded immutable offline-job request. Bytes are copied from the client's
//! grant before decoding; decoding alone conveys no file or launch authority.
pub const KIND: u8 = 42;
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
pub const OP_DESCRIBE_TAGGED: u64 = 0x4d50_5014;
pub const OP_START: u64 = 0x4d50_5020;
pub const OP_CANCEL: u64 = 0x4d50_5021;
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

/// Descriptive full-width job transcript. Hashes/nonce are intentionally omitted
/// from Debug; this record conveys no authority without the authenticated route.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub ticket: u32,
    pub frames: u32,
    pub files_epoch: u64,
    pub nonce: [u8; 16],
    pub plugin_hash: [u8; 32],
    pub input_hash: [u8; 32],
    pub output_hash: [u8; 32],
    pub parent: crate::restricted_helper::Subject,
    pub child: Option<crate::restricted_helper::Subject>,
    pub generation: u64,
    pub format: u32,
    pub cookie: u32,
    pub coordinator_epoch: u64,
}
impl Info {
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != INFO_BYTES || &bytes[..8] != b"MOANINF2" {
            return None;
        }
        let u32at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let u64at = |i| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
        let ticket = u32at(8);
        let frames = u32at(12);
        let files_epoch = u64at(16);
        let nonce = bytes[24..40].try_into().ok()?;
        let parent = crate::restricted_helper::Subject::decode(u64at(136))?;
        let childword = u64at(144);
        let child = if childword == 0 {
            None
        } else {
            Some(crate::restricted_helper::Subject::decode(childword)?)
        };
        let generation = u64at(152);
        let format = u32at(160);
        let cookie = u32at(164);
        let coordinator_epoch = u64at(168);
        if ticket == 0
            || ticket > 0xff_ffff
            || frames == 0
            || frames > 48_000 * 60
            || files_epoch == 0
            || nonce == [0; 16]
            || format > 1
            || cookie == 0
            || generation > crate::restricted_helper::MAX_GENERATION
        {
            return None;
        }
        if child.is_some() && (generation == 0 || coordinator_epoch == 0)
            || child.is_none() && (generation != 0 || coordinator_epoch != 0)
        {
            return None;
        }
        Some(Self {
            ticket,
            frames,
            files_epoch,
            nonce,
            plugin_hash: bytes[40..72].try_into().ok()?,
            input_hash: bytes[72..104].try_into().ok()?,
            output_hash: bytes[104..136].try_into().ok()?,
            parent,
            child,
            generation,
            format,
            cookie,
            coordinator_epoch,
        })
    }
}

/// Full epoch binding on parent->Shell start/cancel, including queued messages
/// surviving a Shell restart. Ticket24+cookie32 share one word without truncation.
pub fn start_words(
    ticket: u32,
    cookie: u32,
    files_epoch: u64,
    shell_epoch: u64,
) -> Option<[u64; 3]> {
    if ticket == 0 || ticket > 0xff_ffff || cookie == 0 || files_epoch == 0 || shell_epoch == 0 {
        return None;
    }
    Some([
        ticket as u64 | (cookie as u64) << 24,
        files_epoch,
        shell_epoch,
    ])
}
pub fn start_decode(words: [u64; 3]) -> Option<(u32, u32, u64, u64)> {
    if words[0] >> 56 != 0 {
        return None;
    }
    let ticket = (words[0] & 0xff_ffff) as u32;
    let cookie = (words[0] >> 24) as u32;
    start_words(ticket, cookie, words[1], words[2])?;
    Some((ticket, cookie, words[1], words[2]))
}

/// Read-only observation, never a launch/kill capability. Clients must compare
/// `known` with their own authenticated Started receipt, not a guessed subject.
pub const OP_STATUS: u64 = 0x4d50_5022;
pub const OP_STATUS_EVENT: u64 = 0x4d50_5023;
pub const STATUS_UNKNOWN: u8 = 0;
pub const STATUS_LIVE: u8 = 1;
pub const STATUS_RETIRED: u8 = 2;
pub const STATUS_UNAVAILABLE: u8 = 3;
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub cookie: u32,
    pub ticket: u32,
    pub state: u8,
    pub recovery_complete: bool,
    pub current: Option<crate::restricted_helper::Subject>,
    pub shell_epoch: u64,
}
impl Observation {
    pub fn words(self) -> Option<[u64; 3]> {
        if self.cookie == 0
            || self.ticket == 0
            || self.ticket > 0xff_ffff
            || self.state > STATUS_UNAVAILABLE
            || self.shell_epoch == 0
            || (self.state != STATUS_UNKNOWN && self.current.is_none())
        {
            return None;
        }
        Some([
            self.cookie as u64
                | (self.ticket as u64) << 32
                | (self.state as u64) << 56
                | (self.recovery_complete as u64) << 60,
            self.current.map_or(0, |s| s.word()),
            self.shell_epoch,
        ])
    }
    pub fn decode(words: [u64; 3]) -> Option<Self> {
        if words[0] >> 61 != 0 {
            return None;
        }
        let value = Self {
            cookie: words[0] as u32,
            ticket: ((words[0] >> 32) & 0xff_ffff) as u32,
            state: ((words[0] >> 56) & 15) as u8,
            recovery_complete: (words[0] >> 60) & 1 != 0,
            current: if words[1] == 0 {
                None
            } else {
                Some(crate::restricted_helper::Subject::decode(words[1])?)
            },
            shell_epoch: words[2],
        };
        (value.words()? == words).then_some(value)
    }
}
pub fn status_words(
    cookie: u32,
    ticket: u32,
    known: Option<crate::restricted_helper::Subject>,
    shell_epoch: u64,
) -> Option<[u64; 3]> {
    if cookie == 0 || ticket == 0 || ticket > 0xff_ffff || shell_epoch == 0 {
        return None;
    }
    if let Some(s) = known {
        crate::restricted_helper::Subject::decode(s.word())?;
    }
    Some([
        cookie as u64 | (ticket as u64) << 32,
        known.map_or(0, |s| s.word()),
        shell_epoch,
    ])
}
pub fn status_decode(
    words: [u64; 3],
) -> Option<(u32, u32, Option<crate::restricted_helper::Subject>, u64)> {
    if words[0] >> 56 != 0 {
        return None;
    }
    let known = if words[1] == 0 {
        None
    } else {
        Some(crate::restricted_helper::Subject::decode(words[1])?)
    };
    let cookie = words[0] as u32;
    let ticket = ((words[0] >> 32) & 0xff_ffff) as u32;
    (status_words(cookie, ticket, known, words[2])? == words)
        .then_some((cookie, ticket, known, words[2]))
}
