// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Chastity autofill on existing BOOT Shell Domain routes. Scalar names/tokens
//! describe runs; only held routes + kernel-stamped/live PD/task authorize use.
//! No pointers/shared memory, no clipboard, no file-open side channel.
//! Shell is a trusted broker and sees credential registers; it must never log
//! packets. Packets deliberately implement neither Debug nor Clone/Copy.
pub const REGISTER: u64 = 0x3E0;
pub const UNREGISTER: u64 = 0x3E1;
pub const BEGIN: u64 = 0x3E2;
pub const ORIGIN: u64 = 0x3E3;
pub const ALLOW: u64 = 0x3E4;
pub const DENY: u64 = 0x3E5;
pub const DATA: u64 = 0x3E6;
pub const FINISH: u64 = 0x3E7;
pub const CANCEL: u64 = 0x3E8;
pub const REGISTERED: u64 = 0x3E9;
pub const READY: u64 = 0x3EA;
pub const OFFER: u64 = 0x3EB;
pub const CONSUMER: u64 = 0x3EC;
pub const APPROVED: u64 = 0x3ED;
pub const ACK: u64 = 0x3EE;
pub const STATUS: u64 = 0x3EF;
pub const MAX_ORIGIN: usize = 512;
pub const MAX_CREDENTIAL: usize = 4096;
pub const MAX_DATA: usize = MAX_CREDENTIAL * 2;
pub const CONSENT_TICKS: u64 = 120_000;
pub const TRANSFER_TICKS: u64 = 30_000;
pub fn request(op: u64) -> bool {
    (REGISTER..=CANCEL).contains(&op)
}
pub fn response(op: u64) -> bool {
    (REGISTERED..=STATUS).contains(&op) || matches!(op, ORIGIN | DATA | FINISH | CANCEL)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subject {
    pub pd: u32,
    pub task: u32,
}
impl Subject {
    pub const SHELL: Self = Self { pd: 3, task: 3 };
    pub const fn valid(self) -> bool {
        self.pd != 0 && self.task != 0
    }
    pub const fn word(self) -> u64 {
        self.pd as u64 | (self.task as u64) << 32
    }
    pub fn decode(v: u64) -> Option<Self> {
        let s = Self {
            pd: v as u32,
            task: (v >> 32) as u32,
        };
        s.valid().then_some(s)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Status {
    Ok = 0,
    Busy = 1,
    Denied = 2,
    Invalid = 3,
    Locked = 4,
    Stale = 5,
    Declined = 6,
    Unavailable = 7,
}
impl Status {
    pub fn decode(v: u64) -> Option<Self> {
        Some(match v {
            0 => Self::Ok,
            1 => Self::Busy,
            2 => Self::Denied,
            3 => Self::Invalid,
            4 => Self::Locked,
            5 => Self::Stale,
            6 => Self::Declined,
            7 => Self::Unavailable,
            _ => return None,
        })
    }
}
pub struct Packet {
    pub opcode: u64,
    pub args: [u64; 3],
}
impl Packet {
    pub const fn new(opcode: u64, a: u64, b: u64, c: u64) -> Self {
        Self {
            opcode,
            args: [a, b, c],
        }
    }
    pub fn erase(&mut self) {
        unsafe {
            core::ptr::write_volatile(&mut self.opcode, 0);
            for a in &mut self.args {
                core::ptr::write_volatile(a, 0)
            }
        }
    }
}
impl Drop for Packet {
    fn drop(&mut self) {
        self.erase();
    }
}
pub const CHUNK: usize = 8;
pub fn chunk(op: u64, token: u32, epoch: u64, offset: usize, bytes: &[u8]) -> Option<Packet> {
    if token == 0
        || epoch == 0
        || offset > MAX_DATA
        || offset % CHUNK != 0
        || bytes.is_empty()
        || bytes.len() > CHUNK
        || offset + bytes.len() > MAX_DATA
    {
        return None;
    }
    let mut data = [0u8; CHUNK];
    data[..bytes.len()].copy_from_slice(bytes);
    let packet = Packet::new(
        op,
        token as u64 | (offset as u64) << 32 | (bytes.len() as u64) << 48,
        epoch,
        u64::from_le_bytes(data),
    );
    for byte in &mut data {
        unsafe { core::ptr::write_volatile(byte, 0) }
    }
    Some(packet)
}
pub fn chunk_meta(packet: &Packet) -> Option<(u32, u64, usize, usize)> {
    let meta = packet.args[0];
    let token = meta as u32;
    let epoch = packet.args[1];
    let offset = ((meta >> 32) & 65535) as usize;
    let count = ((meta >> 48) & 255) as usize;
    if meta >> 56 != 0
        || token == 0
        || epoch == 0
        || offset % CHUNK != 0
        || !(1..=CHUNK).contains(&count)
        || offset + count > MAX_DATA
    {
        return None;
    }
    let mut data = packet.args[2].to_le_bytes();
    let clean = data[count..].iter().all(|v| *v == 0);
    for byte in &mut data {
        unsafe { core::ptr::write_volatile(byte, 0) }
    }
    clean.then_some((token, epoch, offset, count))
}
pub fn chunk_copy(packet: &Packet, out: &mut [u8]) -> Option<usize> {
    let (_, _, offset, count) = chunk_meta(packet)?;
    if offset + count > out.len() {
        return None;
    }
    let mut data = packet.args[2].to_le_bytes();
    out[offset..offset + count].copy_from_slice(&data[..count]);
    for byte in &mut data {
        unsafe { core::ptr::write_volatile(byte, 0) }
    }
    Some(count)
}
pub fn lengths(word: u64) -> Option<(usize, usize)> {
    let a = (word & 65535) as usize;
    let b = ((word >> 16) & 65535) as usize;
    (word >> 32 == 0 && a <= MAX_CREDENTIAL && b > 0 && b <= MAX_CREDENTIAL).then_some((a, b))
}

/// Exact opaque account scope, rendered without Unicode/control ambiguity.
/// International hostnames must be supplied in their ASCII/punycode form.
pub fn valid_origin(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes.len() <= MAX_ORIGIN
        && bytes.iter().all(|b| (0x21..=0x7e).contains(b))
}

/// Shell queries Files' exact immutable current binding after OP_BIND. Six
/// replies carry 3 name bytes each, exact task ID, and part; never a name claim
/// from the program. A task number is correlation, not authority: Files' caller
/// envelope and Shell's own held lifecycle catalog must both attest the run.
pub const IDENTIFY: u64 = 0x3F0;
pub fn identity_reply(task: u32, part: usize, name: [u8; 3]) -> u64 {
    (task as u64) << 32
        | ((0xA8 | part as u8) as u64) << 24
        | name[0] as u64
        | (name[1] as u64) << 8
        | (name[2] as u64) << 16
}
pub fn identity_decode(value: u64) -> Option<(u32, usize, [u8; 3])> {
    let tag = ((value >> 24) & 255) as u8;
    let task = (value >> 32) as u32;
    let part = (tag & 7) as usize;
    (tag & 0xF8 == 0xA8 && task != 0 && part < 6).then_some((
        task,
        part,
        [value as u8, (value >> 8) as u8, (value >> 16) as u8],
    ))
}
