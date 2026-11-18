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
