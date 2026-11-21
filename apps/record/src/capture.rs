// SPDX-License-Identifier: GPL-3.0-only
//! Bounded main-thread capture client. No borrowed shared pixel slice escapes.
//! Stop replaces pending Acquire/ACK work with a reserved, independent Release.
use sex_pdx::{desktop_recording as w, desktop_recording_capture as c};
const FIRST: u32 = 0x80000;
const RELEASE: u32 = 0xfffff;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    Invalid,
    Fenced,
    Exhausted,
    Denied,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Acquire,
    Ack,
    Release,
}
struct Pending {
    kind: Kind,
    sequence: w::RoleSequence,
    args: [u64; 3],
    sent: bool,
    candidate: Option<c::FrameReply>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub publication_ms: u64,
    pub generation: u64,
}
pub struct Capture {
    bank: c::Bank,
    pub scope: c::Scope,
    next: u32,
    pending: Option<Pending>,
    snapshot: Option<c::Snapshot>,
    copied: usize,
    owned: Option<Frame>,
    active: bool,
    stopping: bool,
    released: bool,
    fenced: bool,
}
impl Capture {
    pub fn open(scope: c::Scope) -> Result<Self, Error> {
        if !scope.valid() {
            return Err(Error::Invalid);
        }
        let info = sex_pdx::memory_region_info(c::SLOT).map_err(|_| Error::Unavailable)?;
        let view = sex_pdx::memory_region_full_view(
            c::SLOT,
            sex_pdx::memory_region::QUERY_VIEW | sex_pdx::memory_region::READ,
        )
        .map_err(|_| Error::Unavailable)?;
        // Both values are actual held-Region syscall results, never wire values.
        let bank = unsafe { c::Bank::from_resolved(info, view, false) }.ok_or(Error::Invalid)?;
        if bank.service_epoch() != scope.display_epoch {
            return Err(Error::Unavailable);
        }
        let mut value = Self {
            bank,
            scope,
            next: FIRST,
            pending: None,
            snapshot: None,
            copied: 0,
            owned: None,
            active: false,
            stopping: false,
            released: false,
            fenced: false,
        };
        if !value.live() {
            return Err(Error::Unavailable);
        }
        Ok(value)
    }
    fn live(&mut self) -> bool {
        for (slot, epoch) in [
            (sex_pdx::SLOT_SHELL, self.scope.shell_epoch),
            (sex_pdx::SLOT_DISPLAY, self.scope.display_epoch),
        ] {
            match sex_pdx::crash_log::peer_state(slot) {
                Ok(peer) if peer.epoch != epoch => {
                    self.fenced = true;
                    self.active = false;
                    self.owned = None;
                    self.snapshot = None;
                    return false;
                }
                Ok(peer) if peer.life == sex_pdx::crash_log::PeerLife::Live => {}
                _ => return false,
            }
        }
        if self.bank.service_epoch() != self.scope.display_epoch {
            self.fenced = true;
            self.active = false;
            return false;
        }
        true
    }
    fn cleanup_live(&self) -> bool {
        sex_pdx::crash_log::peer_state(sex_pdx::SLOT_DISPLAY).is_ok_and(|peer| {
            peer.epoch == self.scope.display_epoch
                && peer.life == sex_pdx::crash_log::PeerLife::Live
        })
    }
    fn sequence(&mut self) -> Result<w::RoleSequence, Error> {
        if self.next >= RELEASE {
            return Err(Error::Exhausted);
        }
        let value =
            w::RoleSequence::new(self.scope.shell_epoch, self.next).ok_or(Error::Exhausted)?;
        self.next += 1;
        Ok(value)
    }
    pub fn activate(&mut self) -> Result<(), Error> {
        if self.stopping || self.fenced || !self.live() {
            return Err(Error::Fenced);
        }
        self.active = true;
        Ok(())
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some() || self.snapshot.is_some() || self.owned.is_some()
    }
    pub fn released(&self) -> bool {
        self.released
    }
    pub fn fenced(&self) -> bool {
        self.fenced
    }
    pub fn acquire(&mut self) -> Result<(), Error> {
        if !self.active || self.stopping || self.pending() {
            return Err(Error::Invalid);
        }
        if !self.live() {
            return Err(if self.fenced {
                Error::Fenced
            } else {
                Error::Unavailable
            });
        }
        let sequence = self.sequence()?;
        let args = c::ControlRequest {
            kind: c::ControlKind::Acquire,
            sequence,
            scope: self.scope,
        }
        .args()
        .ok_or(Error::Invalid)?;
        self.pending = Some(Pending {
            kind: Kind::Acquire,
            sequence,
            args,
            sent: false,
            candidate: None,
        });
        Ok(())
    }
    /// Reserved nonce cannot be consumed by ordinary frame activity. This does
    /// not wait for codec/Files or an old frame reply; no further copy is allowed.
    pub fn stop(&mut self) {
        if self.stopping {
            return;
        }
        self.stopping = true;
        self.active = false;
        self.snapshot = None;
        self.owned = None;
        self.copied = 0;
        let Some(sequence) = w::RoleSequence::new(self.scope.shell_epoch, RELEASE) else {
            self.fenced = true;
            return;
        };
        let Some(args) = (c::FrameAck {
            kind: c::AckKind::Release,
            sequence,
            session: self.scope.session,
            generation: 0,
            display_epoch: self.scope.display_epoch,
        })
        .args() else {
            self.fenced = true;
            return;
        };
        self.pending = Some(Pending {
            kind: Kind::Release,
            sequence,
            args,
            sent: false,
            candidate: None,
        });
    }
    pub fn receive(&mut self, m: &sex_pdx::PdxMessage) -> bool {
        if m.type_id != 1 || m.caller_pd != w::DISPLAY || m.caller_task != w::DISPLAY {
            return false;
        }
        let Some(reply) = c::FrameReply::decode(m.caller_pd, m.arg0) else {
            return false;
        };
        if let Some(p) = self.pending.as_mut() {
            if p.sent && reply.matches(p.sequence, self.scope.record_task) && p.candidate.is_none()
            {
                p.candidate = Some(reply)
            }
        }
        true
    }
    /// Always run before encoder or Files work. On temporary peer uncertainty,
    /// retain exact replies and pause; changed epochs permanently fence capture.
    pub fn poll(&mut self) -> Result<(), Error> {
        let cleanup = self
            .pending
            .as_ref()
            .is_some_and(|p| p.kind == Kind::Release);
        if cleanup {
            // An old Shell epoch cannot authorize a frame, but must not prevent
            // surrendering the exact held session to the unchanged Display.
            if !self.cleanup_live() {
                return Err(Error::Unavailable);
            }
        } else {
            if self.fenced {
                return Err(Error::Fenced);
            }
            if !self.live() {
                return Err(if self.fenced {
                    Error::Fenced
                } else {
                    Error::Unavailable
                });
            }
        }
        let Some(p) = self.pending.as_mut() else {
            return Ok(());
        };
        if !p.sent {
            let op = if p.kind == Kind::Acquire {
                c::OP_CONTROL
            } else {
                c::OP_FRAME
            };
            match sex_pdx::pdx_call_checked(
                sex_pdx::SLOT_DISPLAY,
                op,
                p.args[0],
                p.args[1],
                p.args[2],
            ) {
                Ok(_) => p.sent = true,
                Err(sex_pdx::ERR_SERVICE_NOT_READY) => return Ok(()),
                Err(_) => {
                    self.fenced = true;
                    return Err(Error::Fenced);
                }
            }
        }
        let Some(reply) = p.candidate else {
            return Ok(());
        };
        let kind = p.kind;
        let sequence = p.sequence;
        if if cleanup {
            !self.cleanup_live()
        } else {
            !self.live()
        } {
            return Err(if self.fenced {
                Error::Fenced
            } else {
                Error::Unavailable
            });
        }
        self.pending = None;
        match (kind, reply.status) {
            (Kind::Acquire, c::FrameStatus::Ready) if self.active && !self.stopping => {
                let snapshot = self.bank.snapshot().ok_or(Error::Invalid)?;
                if snapshot.header.scope != self.scope || snapshot.header.sequence != sequence {
                    return Err(Error::Invalid);
                }
                self.snapshot = Some(snapshot);
                self.copied = 0;
            }
            (Kind::Acquire, c::FrameStatus::Busy) => {}
            (Kind::Ack, c::FrameStatus::Acked) if self.active && !self.stopping => {}
            (Kind::Release, c::FrameStatus::Released) => self.released = true,
            _ => {
                self.active = false;
                self.snapshot = None;
                self.owned = None;
                self.fenced = true; // Failure, never a Released/Saved receipt.
                return Err(Error::Denied);
            }
        }
        Ok(())
    }
    /// Copies at most CHUNK_PIXELS per main-loop turn, then queues an exact frame
    /// ACK. Only its actual reply permits `take_frame` and codec execution.
    pub fn copy_turn(&mut self, private: &mut [u32]) -> Result<(), Error> {
        if !self.active || self.stopping {
            return Ok(());
        }
        let Some(snapshot) = self.snapshot else {
            return Ok(());
        };
        if !self.live() {
            return Err(if self.fenced {
                Error::Fenced
            } else {
                Error::Unavailable
            });
        }
        let count = self
            .bank
            .copy_chunk(snapshot, private, self.copied)
            .ok_or(Error::Invalid)?;
        self.copied = self.copied.checked_add(count).ok_or(Error::Invalid)?;
        if self.copied == c::PIXELS {
            if self.bank.snapshot() != Some(snapshot) || !self.live() {
                return Err(Error::Fenced);
            }
            let sequence = self.sequence()?;
            let args = c::FrameAck {
                kind: c::AckKind::Frame,
                sequence,
                session: self.scope.session,
                generation: snapshot.header.generation,
                display_epoch: self.scope.display_epoch,
            }
            .args()
            .ok_or(Error::Invalid)?;
            self.pending = Some(Pending {
                kind: Kind::Ack,
                sequence,
                args,
                sent: false,
                candidate: None,
            });
            self.owned = Some(Frame {
                publication_ms: snapshot.header.publication_ms,
                generation: snapshot.header.generation,
            });
            self.snapshot = None;
        }
        Ok(())
    }
    pub fn take_frame(&mut self) -> Option<Frame> {
        if self.pending.is_some() || !self.active || self.stopping || !self.live() {
            return None;
        }
        self.owned.take()
    }
}
