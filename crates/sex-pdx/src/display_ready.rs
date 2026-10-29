//! Boot-time display readiness over boot-granted Domain capabilities.
//!
//! The fixed genesis assigns SexDisplay PD 1; kernel init asserts that binding.
//! The authenticated caller field, not a payload PD or a global memory flag,
//! establishes who announced readiness. This is startup-only: restart epochs
//! and post-startup availability are outside this protocol.
use crate::{PdxMessage, SLOT_DISPLAY, SLOT_SHELL, SLOT_SILKBAR};

pub const OP_DISPLAY_READY: u64 = 0x101;
pub const DISPLAY_READY_VERSION: u64 = 1;
pub const DISPLAY_PD_ID: u32 = 1;
pub const STARTUP_INBOX_CAPACITY: usize = 256;
pub const STARTUP_SEND_ATTEMPTS: usize = 64;

pub fn is_display_ready(msg: &PdxMessage) -> bool {
    msg.type_id == OP_DISPLAY_READY
        && msg.caller_pd == DISPLAY_PD_ID
        && msg.arg0 == DISPLAY_READY_VERSION
        && msg.arg1 == 0
        && msg.arg2 == 0
}

/// Each recipient remains pending until its own enqueue succeeds. Poll once per
/// display event-loop pass, so a full client ring never stalls display draining.
pub struct ReadyPublisher {
    pending: u8,
}

impl ReadyPublisher {
    pub const fn new() -> Self { Self { pending: 0b11 } }

    /// Bit 0 reports newly notified shell; bit 1 newly notified SilkBar.
    /// Errors retain the bit for the next pass; accepted notifications are not
    /// sent again. Call only after display initialization is complete.
    pub fn poll(&mut self) -> u8 {
        let mut sent = 0;
        for (bit, slot) in [(1, SLOT_SHELL), (2, SLOT_SILKBAR)] {
            if self.pending & bit != 0
                && crate::pdx_call_checked(slot, OP_DISPLAY_READY, DISPLAY_READY_VERSION, 0, 0).is_ok()
            {
                self.pending &= !bit;
                sent |= bit;
            }
        }
        sent
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observe {
    Ready,
    Deferred,
    RejectedControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitError {
    InboxFull,
}

/// Per-client storage, not shared cross-domain state. Keep this in the client's
/// static storage so the 64 KiB task stack does not hold a second message ring.
pub struct StartupInbox<const N: usize> {
    messages: [Option<PdxMessage>; N],
    head: usize,
    len: usize,
    ready: bool,
}

impl<const N: usize> StartupInbox<N> {
    pub const fn new() -> Self {
        assert!(N > 0);
        Self { messages: [None; N], head: 0, len: 0, ready: false }
    }

    /// Reject malformed/forged readiness controls. Ordinary messages retain
    /// their full values and FIFO order. On overflow ownership stays with caller.
    pub fn observe(&mut self, msg: PdxMessage) -> Result<Observe, PdxMessage> {
        if msg.type_id == OP_DISPLAY_READY {
            if is_display_ready(&msg) {
                self.ready = true;
                return Ok(Observe::Ready);
            }
            return Ok(Observe::RejectedControl);
        }
        if self.len == N { return Err(msg); }
        self.messages[(self.head + self.len) % N] = Some(msg);
        self.len += 1;
        Ok(Observe::Deferred)
    }

    /// No timeout turns missing readiness into success. One receive and one
    /// cooperative yield per pass bounds work and lets the publisher progress.
    /// Refuse overflow BEFORE dequeuing another message; no unrelated work is
    /// dropped. The client must fail startup explicitly on InboxFull.
    pub fn wait_for_display(&mut self) -> Result<(), WaitError> {
        while !self.ready {
            if self.len == N { return Err(WaitError::InboxFull); }
            if let Some(msg) = crate::pdx_try_listen_raw(0) {
                // Capacity was checked before receive; this cannot overflow.
                assert!(self.observe(msg).is_ok());
            }
            crate::sys_yield();
        }
        Ok(())
    }

    /// Drain deferred messages before receiving new ordinary runtime messages.
    pub fn pop(&mut self) -> Option<PdxMessage> {
        if self.len == 0 { return None; }
        let msg = self.messages[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        msg
    }

    pub fn is_empty(&self) -> bool { self.len == 0 }
}

/// Preserve the first shell display request on temporary ring saturation.
/// Exhaustion is explicit, never a successful startup with a missing request.
pub fn send_startup_display(opcode: u64, arg0: u64, arg1: u64, arg2: u64) -> Result<u64, u64> {
    for _ in 0..STARTUP_SEND_ATTEMPTS {
        match crate::pdx_call_checked(SLOT_DISPLAY, opcode, arg0, arg1, arg2) {
            Err(crate::ERR_SERVICE_NOT_READY) => crate::sys_yield(),
            result => return result,
        }
    }
    Err(crate::ERR_SERVICE_NOT_READY)
}
