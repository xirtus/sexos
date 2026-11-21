// SPDX-License-Identifier: GPL-3.0-only
//! One asynchronous Files turn. Only this module touches the recorder's client
//! region. A delivered request retains the region until its exact authenticated
//! reply; cancellation does not make it reusable. Epoch uncertainty fences it.
use sex_pdx::{desktop_recording as w, fs as f, memory_region as m};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    Busy,
    Invalid,
    Fenced,
    Exhausted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    pub kind: u8,
    pub status: u8,
    pub value: u32,
}
struct Pending {
    request: f::Request,
    sent: bool,
}
pub struct Files {
    base: usize,
    epoch: u64,
    next: u32,
    pending: Option<Pending>,
    candidate: Option<Reply>,
    fenced: bool,
}
impl Files {
    pub fn open(epoch: u64) -> Result<Self, Error> {
        if epoch == 0 {
            return Err(Error::Invalid);
        }
        let peer = sex_pdx::crash_log::peer_state(sex_pdx::SLOT_STORAGE)
            .map_err(|_| Error::Unavailable)?;
        if peer.epoch != epoch || peer.life != sex_pdx::crash_log::PeerLife::Live {
            return Err(Error::Unavailable);
        }
        let rights = m::QUERY_VIEW | m::READ | m::WRITE;
        let info = sex_pdx::memory_region_info(f::CLIENT_SLOT).map_err(|_| Error::Unavailable)?;
        let view = sex_pdx::memory_region_full_view(f::CLIENT_SLOT, rights)
            .map_err(|_| Error::Unavailable)?;
        let ordinal = sex_pdx::app_catalog::by_target(w::TARGET)
            .ok_or(Error::Invalid)?
            .ordinal as usize;
        let id = f::REGION_ID_BASE
            .checked_add((f::PROGRAM_INDEX_BASE + ordinal) as u32)
            .ok_or(Error::Invalid)?;
        if info.region_id != id
            || view.region_id != id
            || info.length != f::REGION_BYTES as u64
            || view.length != info.length
            || info.rights != rights
            || info.peer_pd != Some(w::FILES)
            || view.dma_address.is_some()
            || view.virtual_address == 0
            || view.virtual_address % 4096 != 0
            || view
                .virtual_address
                .checked_add(view.length)
                .is_none_or(|end| end > m::USER_VA_END)
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            base: usize::try_from(view.virtual_address).map_err(|_| Error::Invalid)?,
            epoch,
            next: 0,
            pending: None,
            candidate: None,
            fenced: false,
        })
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn fenced(&self) -> bool {
        self.fenced
    }
    fn live(&mut self) -> bool {
        match sex_pdx::crash_log::peer_state(sex_pdx::SLOT_STORAGE) {
            Ok(peer)
                if peer.epoch == self.epoch && peer.life == sex_pdx::crash_log::PeerLife::Live =>
            {
                true
            }
            Ok(peer) if peer.epoch != self.epoch => {
                self.fenced = true;
                false
            }
            _ => false,
        }
    }
    pub fn start(&mut self, kind: u8, path: &[u8], offset: u64, data: &[u8]) -> Result<(), Error> {
        if self.fenced {
            return Err(Error::Fenced);
        }
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        if !matches!(kind, f::CREATE | f::WRITE | f::SYNC)
            || path.len() > w::NAME_BYTES
            || path.contains(&b'/')
            || path.first() == Some(&b'@')
            || path.iter().any(u8::is_ascii_control)
            || data.len() > sex_media::recording::TURN_BYTES
            || data.len() > f::IO_MAX
            || kind != f::WRITE && !data.is_empty()
            || kind == f::SYNC && !path.is_empty()
            || kind != f::SYNC && path.is_empty()
        {
            return Err(Error::Invalid);
        }
        if !self.live() {
            return Err(if self.fenced {
                Error::Fenced
            } else {
                Error::Unavailable
            });
        }
        let next = self
            .next
            .checked_add(1)
            .filter(|seq| *seq <= 0xff_ffff)
            .ok_or(Error::Exhausted)?;
        // No request owns these bytes. Files writes its stat cache separately;
        // no shared Rust reference is held across a provider turn.
        unsafe {
            core::ptr::write_bytes(self.base as *mut u8, 0, f::PATH_BYTES);
            core::ptr::copy_nonoverlapping(path.as_ptr(), self.base as *mut u8, path.len());
            core::ptr::copy_nonoverlapping(
                data.as_ptr(),
                (self.base + f::PATH_BYTES) as *mut u8,
                data.len(),
            );
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.next = next;
        self.pending = Some(Pending {
            request: f::Request {
                seq: next,
                kind,
                path_len: path.len() as u16,
                offset,
                length: data.len() as u64,
            },
            sent: false,
        });
        Ok(())
    }
    pub fn poll(&mut self) {
        if self.fenced || !self.live() {
            return;
        }
        let Some(p) = self.pending.as_mut() else {
            return;
        };
        if p.sent {
            return;
        }
        let a = p.request.args();
        match sex_pdx::pdx_call_checked(sex_pdx::SLOT_STORAGE, f::OP, a[0], a[1], a[2]) {
            Ok(_) => p.sent = true,
            Err(sex_pdx::ERR_SERVICE_NOT_READY) => {}
            Err(_) => self.fenced = true,
        }
    }
    /// Keep an authenticated candidate even while a peer-state snapshot is
    /// temporarily unavailable. Dropping it would lose a successfully delivered
    /// provider ACK and strand the client's buffer lease forever.
    pub fn receive(&mut self, message: &sex_pdx::PdxMessage) -> bool {
        if message.type_id != 1 || message.caller_pd != w::FILES || message.caller_task != w::FILES
        {
            return false;
        }
        let Some((sequence, status, value)) = f::decode_reply(message.arg0) else {
            return false;
        };
        let Some(pending) = self.pending.as_ref() else {
            return true;
        };
        if !pending.sent || sequence != pending.request.seq || status >= f::PENDING {
            return true;
        }
        if self.candidate.is_none() {
            self.candidate = Some(Reply {
                kind: pending.request.kind,
                status,
                value,
            })
        }
        true
    }
    pub fn take_reply(&mut self) -> Option<Reply> {
        if self.fenced || self.candidate.is_none() || !self.live() {
            return None;
        }
        let candidate = self.candidate?;
        // Bracket positive admission with matching epoch/live snapshots. A
        // later restart cannot bless any old queued reply for a new request.
        if !self.live() {
            return None;
        }
        self.candidate = None;
        self.pending = None;
        Some(candidate)
    }
}
