// SPDX-License-Identifier: GPL-3.0-only
//! Boot-only shell readiness, authenticated by the kernel-supplied caller ID.
//! BOOT_DAG grants shell's private slot 9 to sexinput before graph completion.
//! This notification establishes receiver initialization, not new authority.
use crate::PdxMessage;

/// Private shell table slot; preserves its initial self-signal cap at slot 3.
pub const SLOT_SHELL_INPUT_READY: u64 = 9;
pub const OP_SHELL_READY: u64 = 0x102;
pub const SHELL_READY_VERSION: u64 = 1;
pub const SHELL_PD_ID: u32 = 3;
pub const INPUT_PD_ID: u32 = 4;
pub const STARTUP_INBOX_CAPACITY: usize = 256;

pub fn is_shell_ready(msg: &PdxMessage) -> bool {
    msg.type_id == OP_SHELL_READY
        && msg.caller_pd == SHELL_PD_ID
        && msg.arg0 == SHELL_READY_VERSION
        && msg.arg1 == 0
        && msg.arg2 == 0
}

/// Poll once per normal shell loop after full shell initialization. Failure
/// retains the notification for retry; no wait here may prevent shell draining.
pub struct ReadyPublisher { pending: bool }

impl ReadyPublisher {
    pub const fn new() -> Self { Self { pending: true } }

    /// True only for the poll that successfully enqueues the notification.
    pub fn poll(&mut self) -> bool {
        if self.pending
            && crate::pdx_call_checked(SLOT_SHELL_INPUT_READY, OP_SHELL_READY, SHELL_READY_VERSION, 0, 0).is_ok()
        {
            self.pending = false;
            true
        } else {
            false
        }
    }

    /// A pending publisher must not enter an indefinite blocking receive.
    pub fn is_pending(&self) -> bool { self.pending }
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
        if msg.type_id == OP_SHELL_READY {
            if is_shell_ready(&msg) {
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
    pub fn wait_for_shell(&mut self) -> Result<(), WaitError> {
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

