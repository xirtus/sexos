//! Trusted helper image pin and staging lease. Policy handshake owns admission.
//! No public IPC begins a lease yet; ordinary image writes are centrally fenced.
use sha2::{Digest, Sha256};

mod approved {
    include!(concat!(env!("OUT_DIR"), "/moanplug_pin.rs"));
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Lease {
    pub serial: u64,
    pub shell_epoch: u64,
    pub files_epoch: u64,
    pub parent: u32,
    pub parent_task: u32,
    pub cookie: u32,
    pub nonce: [u8; 16],
}

impl core::fmt::Debug for Lease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lease")
            .field("serial", &self.serial)
            .field("shell_epoch", &self.shell_epoch)
            .field("files_epoch", &self.files_epoch)
            .field("parent", &self.parent)
            .field("parent_task", &self.parent_task)
            .finish_non_exhaustive()
    }
}

pub struct Staging {
    current: Option<Lease>,
    serial: u64,
    writing: bool,
    ordinary: Option<(u32, u64, u64)>, // generation, Shell epoch, Files epoch
    ordinary_serial: u32,
}
impl Staging {
    pub const fn new() -> Self {
        Self {
            current: None,
            serial: 0,
            writing: false,
            ordinary: None,
            ordinary_serial: 0,
        }
    }
    pub fn busy(&self) -> bool {
        self.current.is_some() || self.writing || self.ordinary.is_some()
    }
    pub fn writer_begin(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        self.writing = true;
        true
    }
    pub fn writer_end(&mut self) {
        assert!(self.writing, "IMAGE_WRITER_UNBALANCED");
        self.writing = false;
    }
    /// Convert a completed ordinary load's writer into an exclusive lease.
    /// The shared IMAGE cannot be written again until Shell's private copy ends.
    pub fn seal_ordinary(&mut self, shell_epoch: u64, files_epoch: u64) -> Option<u32> {
        if !self.writing || self.current.is_some() || self.ordinary.is_some() || shell_epoch == 0 || files_epoch == 0 {
            return None;
        }
        let next = self.ordinary_serial.checked_add(1)?;
        if next > sex_pdx::program_store::MAX_IMAGE_GENERATION { return None; }
        self.ordinary_serial = next;
        self.ordinary = Some((next, shell_epoch, files_epoch));
        self.writing = false;
        Some(next)
    }
    pub fn ordinary_consumed(&mut self, generation: u32, shell_epoch: u64, files_epoch: u64) -> bool {
        if self.ordinary != Some((generation, shell_epoch, files_epoch)) { return false; }
        self.ordinary = None;
        true
    }
    pub fn ordinary_recover(&mut self, shell_epoch: u64, shell_dead: bool) -> bool {
        if self.ordinary.is_some_and(|(_, epoch, _)| epoch != shell_epoch || shell_dead) {
            self.ordinary = None;
            return true;
        }
        false
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
/// One guard covers the complete staging write, including nested block waits.
/// Both writers and attestation acquire the same mutex before touching state.
/// A future concurrent dispatcher cannot overlap a writer with hashing/lease.
#[must_use]
pub struct Writer {
    _local: core::marker::PhantomData<*mut ()>,
    sealed: bool,
}
pub fn writer() -> Option<Writer> {
    STAGING.lock().writer_begin().then(|| Writer {
        _local: core::marker::PhantomData,
        sealed: false,
    })
}
impl Writer {
    pub fn seal_ordinary(&mut self, shell_epoch: u64, files_epoch: u64) -> Option<u32> {
        let generation = STAGING.lock().seal_ordinary(shell_epoch, files_epoch)?;
        self.sealed = true;
        Some(generation)
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        if !self.sealed { STAGING.lock().writer_end(); }
    }
}
/// Shell's incarnation as Files sees it through route 6. Images built without the
/// recorder hold no such route; Files then cannot observe Shell's restarts and uses
/// `UNOBSERVED_SHELL`, which only Shell's own consume or next load releases.
pub const UNOBSERVED_SHELL: u64 = u64::MAX;
pub fn shell_epoch() -> u64 {
    match sex_pdx::crash_log::peer_state(6) {
        Ok(peer) if peer.live() => peer.epoch,
        Ok(_) => 0,
        Err(_) => UNOBSERVED_SHELL,
    }
}
/// Only the exact current Shell run can release the image. A dead or restarted
/// Shell can no longer be inside its synchronous kernel copy.
pub fn recover_ordinary() {
    use sex_pdx::crash_log::{self, PeerLife};
    let Some((_, held_epoch, _)) = STAGING.lock().ordinary else { return; };
    if held_epoch == UNOBSERVED_SHELL { return; }
    if let Ok(peer) = crash_log::peer_state(6) {
        if STAGING.lock().ordinary_recover(peer.epoch, peer.life == PeerLife::Dead) {
            sex_pdx::serial_record!("[files.image.recovered] shell_epoch={} current_epoch={}", held_epoch, peer.epoch);
        }
    }
}
/// Shell copies each loaded image synchronously and reports it consumed before it
/// asks for another. A new load from Shell while an unobserved lease is still held
/// therefore means that Shell run ended mid-copy: release it rather than refuse
/// every later program.
pub fn release_unobserved_before_load() {
    let mut staging = STAGING.lock();
    if let Some((generation, UNOBSERVED_SHELL, _)) = staging.ordinary {
        staging.ordinary = None;
        drop(staging);
        sex_pdx::serial_record!("[files.image.recovered] generation={} shell_epoch=unobserved reason=next_load", generation);
    }
}
pub fn ordinary_consumed(generation: u32, files_epoch: u64) -> bool {
    use sex_pdx::crash_log;
    let epoch = shell_epoch();
    epoch != 0 && crash_log::self_epoch() == Ok(files_epoch)
        && STAGING.lock().ordinary_consumed(generation, epoch, files_epoch)
}
pub fn attest_and_lease(subject: Lease, image: &[u8]) -> Option<Lease> {
    STAGING
        .lock()
        .begin(subject, image, approved::LENGTH, approved::SHA256)
}
pub fn copied(lease: Lease, shell_epoch: u64, files_epoch: u64) -> bool {
    STAGING.lock().copied(lease, shell_epoch, files_epoch)
}

#[cfg(test)]
mod ordinary_tests {
    use super::*;
    #[test]
    fn recorder_writer_cannot_replace_image_before_exact_copy_receipt() {
        let mut stage = Staging::new();
        assert!(stage.writer_begin());
        let first = stage.seal_ordinary(11, 20).unwrap();
        assert!(!stage.writer_begin()); // queued recorder load fails closed
        assert!(!stage.ordinary_consumed(first + 1, 11, 20));
        assert!(!stage.ordinary_consumed(first, 12, 20));
        assert!(!stage.ordinary_consumed(first, 11, 21));
        assert!(!stage.ordinary_recover(11, false)); // live Shell may be copying
        assert!(!stage.writer_begin());
        assert!(stage.ordinary_consumed(first, 11, 20));
        assert!(stage.writer_begin());
        stage.writer_end();
    }
    #[test]
    fn shell_restart_retires_abandoned_copy_but_old_receipt_cannot_release_next() {
        let mut stage = Staging::new();
        assert!(stage.writer_begin());
        let old = stage.seal_ordinary(11, 20).unwrap();
        assert!(stage.ordinary_recover(12, false));
        assert!(stage.writer_begin());
        let new = stage.seal_ordinary(12, 20).unwrap();
        assert!(!stage.ordinary_consumed(old, 11, 20));
        assert!(!stage.writer_begin());
        assert!(stage.ordinary_consumed(new, 12, 20));
    }
}
