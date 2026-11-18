//! IPC v2 message layout shared by the kernel and every program (plan PR #405, phase 2).
//!
//! A message is up to eight 64-bit message registers (rdi, rsi, rdx, r10, r8, r9,
//! r12, r13 on x86_64) plus, optionally, the first `buffer_len` bytes of the
//! caller's own 4 KiB IPC buffer. The buffer already lives in the caller's region,
//! so on a single address space nothing is copied: the kernel checks the length and
//! hands the receiver the buffer's address. The first register is always the
//! [`MessageInfo`] word, so seven registers carry payload.
//!
//! Endpoints are rendezvous points: a call finds its server already waiting (the
//! fastpath) or parks until the server's next receive. Every delivered message
//! carries the 64-bit badge minted into the caller's endpoint capability, so a
//! server knows who is calling without asking the kernel.

/// Message registers per call, including the [`MessageInfo`] word.
pub const MESSAGE_REGISTERS: usize = 8;
/// Payload registers (all but the info word).
pub const PAYLOAD_REGISTERS: usize = MESSAGE_REGISTERS - 1;
/// Size of each thread's IPC buffer.
pub const IPC_BUFFER_BYTES: usize = 4096;
/// The IPC buffer's alignment (one page, so validating it is one range check).
pub const IPC_BUFFER_ALIGN: usize = 4096;

/// First message register: what the rest of the message holds.
///
/// Layout (low to high): bits 0-3 payload registers used (0..=7), bits 4-16 IPC
/// buffer bytes used (0..=4096), bits 17-31 reserved (zero), bits 32-63 a label
/// the protocol defines (an opcode, usually).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageInfo(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageInfoError {
    TooManyRegisters,
    BufferTooLong,
    ReservedBits,
}

const REGS_MASK: u64 = 0xF;
const BUF_SHIFT: u32 = 4;
const BUF_MASK: u64 = 0x1FFF;
const RESERVED_MASK: u64 = 0xFFFE_0000;

impl MessageInfo {
    pub const EMPTY: Self = Self(0);

    pub const fn new(label: u32, registers: usize, buffer_len: usize) -> Result<Self, MessageInfoError> {
        if registers > PAYLOAD_REGISTERS {
            return Err(MessageInfoError::TooManyRegisters);
        }
        if buffer_len > IPC_BUFFER_BYTES {
            return Err(MessageInfoError::BufferTooLong);
        }
        Ok(Self(((label as u64) << 32) | ((buffer_len as u64) << BUF_SHIFT) | registers as u64))
    }

    /// Decodes a word that came from untrusted code; rejects anything malformed.
    pub const fn from_raw(raw: u64) -> Result<Self, MessageInfoError> {
        if raw & RESERVED_MASK != 0 {
            return Err(MessageInfoError::ReservedBits);
        }
        if (raw & REGS_MASK) as usize > PAYLOAD_REGISTERS {
            return Err(MessageInfoError::TooManyRegisters);
        }
        if ((raw >> BUF_SHIFT) & BUF_MASK) as usize > IPC_BUFFER_BYTES {
            return Err(MessageInfoError::BufferTooLong);
        }
        Ok(Self(raw))
    }

    pub const fn raw(self) -> u64 { self.0 }
    pub const fn label(self) -> u32 { (self.0 >> 32) as u32 }
    pub const fn registers(self) -> usize { (self.0 & REGS_MASK) as usize }
    pub const fn buffer_len(self) -> usize { ((self.0 >> BUF_SHIFT) & BUF_MASK) as usize }
}

/// A whole message as registers: `words[0]` is the info word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub words: [u64; MESSAGE_REGISTERS],
}

impl Message {
    pub const fn empty() -> Self { Self { words: [0; MESSAGE_REGISTERS] } }

    /// Builds a message from a label and up to seven payload words.
    pub fn new(label: u32, payload: &[u64]) -> Result<Self, MessageInfoError> {
        let info = MessageInfo::new(label, payload.len(), 0)?;
        let mut words = [0; MESSAGE_REGISTERS];
        words[0] = info.raw();
        words[1..=payload.len()].copy_from_slice(payload);
        Ok(Self { words })
    }

    pub fn info(&self) -> Result<MessageInfo, MessageInfoError> { MessageInfo::from_raw(self.words[0]) }

    /// The payload registers the info word says are in use. Unused registers are
    /// not part of the message (the kernel never copies them).
    pub fn payload(&self) -> &[u64] {
        let used = (self.words[0] & REGS_MASK) as usize;
        &self.words[1..1 + used.min(PAYLOAD_REGISTERS)]
    }
}

/// Checks that an IPC buffer pointer is page-aligned and that the bytes in use lie
/// inside the caller's region `[region_base, region_base + region_len)`. Returns
/// the buffer address on success.
pub const fn validate_ipc_buffer(buffer: u64, info: MessageInfo, region_base: u64, region_len: u64) -> Option<u64> {
    let used = info.buffer_len() as u64;
    if used == 0 {
        return Some(buffer);
    }
    if buffer % IPC_BUFFER_ALIGN as u64 != 0 || buffer < region_base {
        return None;
    }
    let offset = buffer - region_base;
    match offset.checked_add(used) {
        Some(end) if end <= region_len => Some(buffer),
        _ => None,
    }
}

/// Bits in a notification word that a bound device interrupt sets. Notification
/// words are 64 independent bits OR-ed together by signalers; this names one.
pub const fn notification_bit(bit: u32) -> u64 {
    if bit < 64 { 1u64 << bit } else { 0 }
}

// ---------------------------------------------------------------------------
// System calls 105 SYS_ENDPOINT and 106 SYS_NOTIFY.
//
// rdi = op | slot << 8; rsi = info word; rdx r10 r8 = payload. Back: rax status,
// rdi badge, rsi info (bits for a notification, a slot for CREATE/GRANT), rdx r10 r8.
// The syscall entry saves five argument registers today, so a message carries the
// info word and three payload registers; the other four of the eight come when the
// entry saves r12-r15.

pub const SYS_ENDPOINT: u64 = 105;
pub const SYS_NOTIFY: u64 = 106;
/// Payload registers the kernel carries now (MessageInfo allows up to 7).
pub const REGISTER_PAYLOAD: usize = 3;

pub const OP_CREATE: u64 = 0;
pub const OP_SEND: u64 = 1;
pub const OP_CALL: u64 = 2;
pub const OP_RECV: u64 = 3;
pub const OP_REPLY: u64 = 4;
pub const OP_REPLY_RECV: u64 = 5;
/// Woken with E_AGAIN: wait again for the same result (never resend).
pub const OP_WAIT: u64 = 6;
pub const OP_GRANT: u64 = 7;
pub const OP_DESTROY: u64 = 8;
/// A copy in this domain's own table: rights rdx (narrower), badge r10 (unbadged source only).
pub const OP_DERIVE: u64 = 9;
pub const OP_SIGNAL: u64 = 1;
pub const OP_NOTIFY_WAIT: u64 = 2;
pub const OP_POLL: u64 = 3;

pub const E_BADMSG: u64 = 0xFFFF_FFFF_FFFF_FFF0;
pub const E_RIGHTS: u64 = 0xFFFF_FFFF_FFFF_FFF1;
pub const E_FULL: u64 = 0xFFFF_FFFF_FFFF_FFF2;
/// The object is gone (destroyed, or its owner's incarnation ended).
pub const E_CLOSED: u64 = 0xFFFF_FFFF_FFFF_FFF3;
/// Woken before the result: ask again with OP_WAIT (or RECV).
pub const E_AGAIN: u64 = 0xFFFF_FFFF_FFFF_FFF4;
/// The other side died, or there is no call to answer.
pub const E_GONE: u64 = 0xFFFF_FFFF_FFFF_FFF5;
/// Someone else already waits on this object.
pub const E_BUSY: u64 = 0xFFFF_FFFF_FFFF_FFF6;

/// What came back: status, the sender's badge, and the message (or a value in words[0]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Received { pub badge: u64, pub words: [u64; 4] }

#[inline]
fn raw(nr: u64, op: u64, slot: u32, w: [u64; 4]) -> (u64, Received) {
    let (status, badge, a, b, c, d): (u64, u64, u64, u64, u64, u64);
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr => status,
            inlateout("rdi") op | (slot as u64) << 8 => badge,
            inlateout("rsi") w[0] => a,
            inlateout("rdx") w[1] => b,
            inlateout("r10") w[2] => c,
            inlateout("r8") w[3] => d,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    (status, Received { badge, words: [a, b, c, d] })
}

/// One endpoint operation; a spurious wake is retried with OP_WAIT, so callers only
/// see the real result.
pub fn endpoint(op: u64, slot: u32, words: [u64; 4]) -> Result<Received, u64> {
    let (mut status, mut got) = raw(SYS_ENDPOINT, op, slot, words);
    while status == E_AGAIN { (status, got) = raw(SYS_ENDPOINT, OP_WAIT, slot, [0; 4]); }
    if status == 0 { Ok(got) } else { Err(status) }
}

/// A call: `label` and up to three words; the answer's words come back.
pub fn call(slot: u32, label: u32, payload: [u64; 3]) -> Result<Received, u64> {
    let info = MessageInfo::new(label, REGISTER_PAYLOAD, 0).map_err(|_| E_BADMSG)?.raw();
    endpoint(OP_CALL, slot, [info, payload[0], payload[1], payload[2]])
}

/// A new endpoint owned by this domain; returns its slot.
pub fn create_endpoint() -> Result<u32, u64> { endpoint(OP_CREATE, 0, [0; 4]).map(|r| r.words[0] as u32) }

/// One notification operation (E_AGAIN retried as a fresh wait).
pub fn notify(op: u64, slot: u32, value: u64) -> Result<u64, u64> {
    loop {
        match raw(SYS_NOTIFY, op, slot, [value, 0, 0, 0]) {
            (0, got) => return Ok(got.words[0]),
            (E_AGAIN, _) => continue,
            (e, _) => return Err(e),
        }
    }
}
