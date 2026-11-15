//! Trusted helper image pin and staging lease. Policy handshake owns admission.
//! No public IPC begins a lease yet; ordinary image writes are centrally fenced.
use sha2::{Digest, Sha256};

mod approved {
    include!(concat!(env!("OUT_DIR"), "/moanplug_pin.rs"));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    pub serial: u64,
    pub shell_epoch: u64,
    pub files_epoch: u64,
    pub parent: u32,
    pub parent_task: u32,
    pub cookie: u32,
    pub nonce: [u8; 16],
}

pub struct Staging {
    current: Option<Lease>,
    serial: u64,
}
impl Staging {
    pub const fn new() -> Self {
        Self {
            current: None,
            serial: 0,
        }
    }
    pub fn busy(&self) -> bool {
        self.current.is_some()
    }
    /// Called only after authenticated policy admission and candidate loading.
    /// Hashes the exact region that synchronous kind3 will copy, then fences ALL
    /// program_store writers until its authenticated completion receipt.
    pub fn begin(
        &mut self,
        mut subject: Lease,
        image: &[u8],
        length: usize,
        digest: [u8; 32],
    ) -> Option<Lease> {
        if self.busy()
            || subject.shell_epoch == 0
            || subject.files_epoch == 0
            || subject.parent == 0
            || subject.parent_task == 0
            || subject.cookie == 0
            || subject.nonce == [0; 16]
            || length == 0
            || image.len() != length
            || <[u8; 32]>::from(Sha256::digest(image)) != digest
        {
            return None;
        }
        let serial = self.serial.checked_add(1)?;
        subject.serial = serial;
        self.serial = serial;
        self.current = Some(subject);
        Some(subject)
    }
    /// Epoch mismatch, stale completion or foreign job leaves the fence closed.
    pub fn copied(&mut self, lease: Lease, shell_epoch: u64, files_epoch: u64) -> bool {
        if self.current != Some(lease)
            || shell_epoch != lease.shell_epoch
            || files_epoch != lease.files_epoch
        {
            return false;
        }
        self.current = None;
        true
    }
}

static STAGING: spin::Mutex<Staging> = spin::Mutex::new(Staging::new());
pub fn busy() -> bool {
    STAGING.lock().busy()
}
pub fn attest_and_lease(subject: Lease, image: &[u8]) -> Option<Lease> {
    STAGING
        .lock()
        .begin(subject, image, approved::LENGTH, approved::SHA256)
}
pub fn copied(lease: Lease, shell_epoch: u64, files_epoch: u64) -> bool {
    STAGING.lock().copied(lease, shell_epoch, files_epoch)
}
