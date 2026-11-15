//! Sole-dispatcher immutable job table. Positive rights are RAM-only and never
//! recovered from the deny journal. Restart drops every entry before replay.
use crate::restricted_job::{Error, Job};
use crate::restricted_storage::Storage;
use sex_pdx::restricted_helper::{Binding, Subject};
pub const CAPACITY: usize = 2;
pub const LIFETIME: u64 = 120_000;
pub struct Entry {
    pub storage: Storage,
    pub job: Job,
    pub format: u8,
    pub epoch: u64,
    expires: u64,
    claimed: bool,
    admitted: bool,
    association: Option<Binding>,
}
impl Entry {
    pub fn live(&self, now: u64, epoch: u64) -> bool {
        self.epoch != 0 && self.epoch == epoch && now < self.expires
    }
    pub fn claim(
        &mut self,
        parent: Subject,
        cookie: u32,
        now: u64,
        epoch: u64,
    ) -> Result<(), Error> {
        if !self.live(now, epoch)
            || self.job.parent() != parent
            || self.job.cookie() != cookie
            || self.claimed
        {
            return Err(Error::Identity);
        }
        self.claimed = true;
        Ok(())
    }
    pub fn bind(&mut self, binding: Binding, now: u64, epoch: u64) -> Result<(), Error> {
        if !self.claimed || !self.live(now, epoch) || binding.provider_epoch != epoch {
            return Err(Error::Identity);
        }
        self.job.bind(binding)?;
        self.association = Some(binding);
        Ok(())
    }
    /// Descriptive transcript, never a capability. Parent output hashes are
    /// available only after validated private output has actually been persisted.
    pub fn info(&self, result: bool) -> Result<[u8; 176], Error> {
        let mut out = [0; 176];
        out[..8].copy_from_slice(b"MOANINF2");
        out[8..12].copy_from_slice(&self.job.id().to_le_bytes());
        out[12..16].copy_from_slice(&(self.job.frames() as u32).to_le_bytes());
        out[16..24].copy_from_slice(&self.epoch.to_le_bytes());
        out[24..40].copy_from_slice(&self.job.nonce());
        out[40..72].copy_from_slice(&self.job.plugin_hash());
        out[72..104].copy_from_slice(&self.job.input_hash());
        if result {
            out[104..136].copy_from_slice(
                &self
                    .job
                    .result(self.job.parent(), self.job.id(), self.job.nonce())?
                    .1,
            );
        }
        out[136..144].copy_from_slice(&self.job.parent().word().to_le_bytes());
        if let Some(b) = self.association {
            out[144..152].copy_from_slice(&b.child.word().to_le_bytes());
            out[152..160].copy_from_slice(&b.generation.to_le_bytes());
        }
        out[160..164].copy_from_slice(&(self.format as u32).to_le_bytes());
        out[164..168].copy_from_slice(&self.job.cookie().to_le_bytes());
        Ok(out)
    }
    pub fn parent(&self, parent: Subject, nonce: [u8; 16], now: u64, epoch: u64) -> bool {
        self.live(now, epoch) && self.job.parent() == parent && self.job.nonce() == nonce
    }
}
pub struct Jobs {
    entries: [Option<Entry>; CAPACITY],
    next: u32,
}
impl Jobs {
    pub const fn new() -> Self {
        Self {
            entries: [None, None],
            next: 1,
        }
    }
    /// IDs are never reused within a Files lifetime, including failed captures.
    /// Caller authenticates current parent and obtains all paths before capture.
    pub fn prepare(
        &mut self,
        epoch: u64,
        now: u64,
        format: u8,
        create: impl FnOnce(u32) -> Result<(Storage, Job), Error>,
    ) -> Result<u32, Error> {
        if epoch == 0 || format > 1 || self.next > 0xff_ffff {
            return Err(Error::State);
        }
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Budget)?;
        let id = self.next;
        self.next += 1;
        let (storage, job) = create(id)?;
        if job.id() != id {
            return Err(Error::Identity);
        }
        let expires = now.checked_add(LIFETIME).ok_or(Error::Bounds)?;
        self.entries[slot] = Some(Entry {
            storage,
            job,
            epoch,
            format,
            expires,
            claimed: false,
            admitted: false,
            association: None,
        });
        Ok(id)
    }
    pub fn get(&mut self, id: u32) -> Option<&mut Entry> {
        self.entries.iter_mut().flatten().find(|e| e.job.id() == id)
    }
    pub fn child(&mut self, actor: Subject) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .flatten()
            .find(|e| e.job.binding().is_some_and(|b| b.child == actor))
    }
    pub fn release(
        &mut self,
        id: u32,
        parent: Subject,
        nonce: [u8; 16],
        now: u64,
        epoch: u64,
    ) -> bool {
        let Some(i) = self.entries.iter().position(|e| {
            e.as_ref()
                .is_some_and(|e| e.job.id() == id && e.parent(parent, nonce, now, epoch))
        }) else {
            return false;
        };
        // Dropping the private job zeroizes complete fixed-capacity buffers.
        self.entries[i] = None;
        true
    }
    pub fn sweep(
        &mut self,
        now: u64,
        epoch: u64,
        mut parent_live: impl FnMut(Subject) -> bool,
        mut binding_live: impl FnMut(Binding) -> bool,
    ) {
        for slot in &mut self.entries {
            let Some(e) = slot else { continue };
            if !e.live(now, epoch) || !parent_live(e.job.parent()) {
                *slot = None;
                continue;
            }
            if let Some(binding) = e.job.binding() {
                if binding_live(binding) {
                    e.admitted = true;
                } else if e.admitted {
                    e.job.retire_child();
                }
            }
        }
    }
    pub fn pending(&self) -> bool {
        self.entries.iter().any(Option::is_some)
    }
}
