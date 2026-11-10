//! Files' single shared image-buffer transaction. This owns no capture authority.
//! The dispatcher checks the actual Shell sender/task and supplies fresh typed
//! peer snapshots; ordinary image writes must refuse while `busy()` is true.
use crate::recording_journal::Fence;
use sex_pdx::desktop_recording::{BindStatus, ImageKind, ImageReply, ImageRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerState {
    Live,
    /// Same epoch, but no longer live. May finish an exact cleanup release;
    /// cannot receive a positive image-read response or start another read.
    Closing,
    Unknown,
    /// Old execution cannot still copy from the image: a new service epoch or
    /// an attested Dead state. Ending/Exited alone must not imply quiescence.
    Quiescent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Reading,
    Ready,
    Owned,
    Releasing,
}
pub enum Action {
    Read,
    Retained,
    Refused(ImageReply),
}
pub struct Service {
    owner: Option<ImageRequest>,
    phase: Phase,
    pending: Option<ImageReply>,
    length: u32,
    delivered: bool,
    revoked: bool,
    release_requested: bool,
    quarantine: Option<u64>,
}
impl Service {
    pub const fn new() -> Self {
        Self {
            owner: None,
            phase: Phase::Reading,
            pending: None,
            length: 0,
            delivered: false,
            revoked: false,
            release_requested: false,
            quarantine: None,
        }
    }
    pub fn owner(&self) -> Option<ImageRequest> {
        self.owner
    }
    pub fn busy(&self) -> bool {
        self.owner.is_some() || self.quarantine.is_some()
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some() || self.release_requested || self.quarantine.is_some()
    }
    pub fn fence(&self) -> Fence {
        if let Some(epoch) = self.quarantine {
            return Fence::Quarantined(epoch);
        }
        match self.owner {
            None => Fence::Free,
            Some(owner) => {
                if self.phase == Phase::Releasing {
                    Fence::Releasing(owner)
                } else {
                    Fence::Held(owner)
                }
            }
        }
    }
    /// Restore only a conservative resource obligation. A former Files epoch
    /// never resurrects permission to deliver an image-read success.
    pub fn restore(fence: Fence) -> Self {
        let mut service = Self::new();
        match fence {
            Fence::Free => {}
            Fence::Quarantined(epoch) => service.quarantine = Some(epoch),
            Fence::Held(owner) | Fence::Releasing(owner) => {
                service.owner = Some(owner);
                service.delivered = true;
                service.revoked = true;
                service.phase = Phase::Owned;
                if matches!(fence, Fence::Releasing(_)) {
                    let mut release = owner;
                    release.kind = ImageKind::Release;
                    service.phase = Phase::Releasing;
                    service.pending = ImageReply::new(release, BindStatus::Ok, 0);
                }
            }
        }
        service
    }
    pub fn quarantine(&mut self, epoch: u64) {
        *self = Self::restore(Fence::Quarantined(epoch));
    }
    /// A corrupt/torn unknown owner is never Free merely because Files restarted.
    /// Record a known Shell epoch, then require its change or actual Dead state.
    pub fn observe_quarantine(&mut self, epoch: u64, dead: bool) {
        let Some(previous) = self.quarantine else {
            return;
        };
        if epoch == 0 {
            return;
        }
        if dead || (previous != 0 && epoch != previous) {
            self.clear();
        } else if previous == 0 {
            self.quarantine = Some(epoch);
        }
    }
    fn clear(&mut self) {
        *self = Self::new();
    }
    fn same(a: ImageRequest, b: ImageRequest) -> bool {
        a.sequence == b.sequence && a.shell_epoch == b.shell_epoch && a.files_epoch == b.files_epoch
    }
    pub fn request(&mut self, request: ImageRequest, state: OwnerState) -> Action {
        let refuse = |status| {
            Action::Refused(ImageReply::new(request, status, 0).expect("VALID_IMAGE_REQUEST"))
        };
        if self.quarantine.is_some() {
            return refuse(BindStatus::Busy);
        }
        if request.kind == ImageKind::Read {
            if let Some(owner) = self.owner {
                if Self::same(owner, request) && matches!(self.phase, Phase::Reading | Phase::Ready)
                {
                    return Action::Retained;
                }
                return refuse(BindStatus::Busy);
            }
            if state != OwnerState::Live {
                return refuse(if state == OwnerState::Unknown {
                    BindStatus::Busy
                } else {
                    BindStatus::Denied
                });
            }
            self.owner = Some(request);
            self.phase = Phase::Reading;
            return Action::Read;
        }
        let Some(owner) = self.owner else {
            return refuse(BindStatus::Denied);
        };
        if !Self::same(owner, request) {
            return refuse(BindStatus::Denied);
        }
        if state == OwnerState::Quiescent {
            self.clear();
            return refuse(BindStatus::Denied);
        }
        if self.phase == Phase::Reading {
            return refuse(BindStatus::Busy);
        }
        self.release_requested = true;
        if state != OwnerState::Unknown {
            self.prepare_release();
        }
        Action::Retained
    }
    fn prepare_release(&mut self) {
        let Some(mut owner) = self.owner else {
            return;
        };
        owner.kind = ImageKind::Release;
        self.pending = ImageReply::new(owner, BindStatus::Ok, 0);
        self.phase = Phase::Releasing;
        self.release_requested = false;
    }
    /// Called exactly once after the synchronous unbound read returns. No nested
    /// receive may call this service while that read owns the image-buffer slice.
    pub fn read_finished(&mut self, request: ImageRequest, status: BindStatus, bytes: u32) -> bool {
        if self.phase != Phase::Reading
            || self.owner.is_none_or(|owner| !Self::same(owner, request))
        {
            return false;
        }
        let Some(reply) = ImageReply::new(request, status, bytes) else {
            return false;
        };
        self.length = bytes;
        self.pending = if self.revoked {
            ImageReply::new(request, BindStatus::Denied, 0)
        } else {
            Some(reply)
        };
        self.phase = Phase::Ready;
        true
    }
    /// Retain every response through provider backpressure. A positive read is
    /// revoked when the initiating service closes. Already-delivered read bytes
    /// stay owned until exact release or proven old-execution quiescence.
    pub fn poll(&mut self, state: OwnerState, send: impl FnOnce(ImageReply) -> bool) {
        if self.owner.is_none() {
            return;
        }
        if state == OwnerState::Quiescent {
            self.clear();
            return;
        }
        if state == OwnerState::Unknown {
            return;
        }
        if state == OwnerState::Closing {
            self.revoked = true;
            if self.phase == Phase::Ready {
                let owner = self.owner.unwrap();
                self.pending = ImageReply::new(owner, BindStatus::Denied, 0);
            }
        }
        if self.release_requested {
            self.prepare_release();
        }
        let Some(reply) = self.pending else {
            return;
        };
        if self.revoked && self.phase == Phase::Ready && reply.status == BindStatus::Ok {
            return;
        }
        if !send(reply) {
            return;
        }
        self.pending = None;
        if self.phase == Phase::Releasing
            || (self.phase == Phase::Ready && reply.status != BindStatus::Ok && !self.delivered)
        {
            self.clear();
        } else if self.phase == Phase::Ready {
            self.delivered = true;
            self.phase = Phase::Owned;
        }
    }
}
