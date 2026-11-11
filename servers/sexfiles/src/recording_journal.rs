//! One captured, KEEP-registered page retains resource obligations, never role or
//! read authority. Raw validated integers avoid resurrecting Rust enum/pointer
//! representations. This is service restart RAM, not power-loss persistence.
use core::sync::atomic::{AtomicU64, Ordering};
use sex_pdx::desktop_recording::{ImageKind, ImageRequest, RoleSequence};
const VERSION: u64 = 1;
const SLOT_WORDS: usize = 16;
const BODY_WORDS: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fence {
    Free,
    Held(ImageRequest),
    Releasing(ImageRequest),
    Quarantined(u64),
}
impl Fence {
    fn words(self) -> [u64; BODY_WORDS] {
        let mut words = [0; BODY_WORDS];
        match self {
            Self::Free => {}
            Self::Held(owner) | Self::Releasing(owner) => {
                words[0] = if matches!(self, Self::Held(_)) { 1 } else { 2 };
                words[1] = owner.sequence.word() as u64;
                words[2] = owner.shell_epoch;
                words[3] = owner.files_epoch as u64;
            }
            Self::Quarantined(epoch) => {
                words[0] = 3;
                words[2] = epoch;
            }
        }
        words
    }
    fn decode(words: [u64; BODY_WORDS]) -> Option<Self> {
        if words[4..].iter().any(|word| *word != 0) {
            return None;
        }
        match words[0] {
            0 if words[1..].iter().all(|word| *word == 0) => Some(Self::Free),
            1 | 2 => {
                let owner = ImageRequest::new(
                    ImageKind::Read,
                    RoleSequence::decode(words[1])?,
                    words[2],
                    u32::try_from(words[3]).ok()?,
                )?;
                Some(if words[0] == 1 {
                    Self::Held(owner)
                } else {
                    Self::Releasing(owner)
                })
            }
            3 if words[1] == 0 && words[3] == 0 => Some(Self::Quarantined(words[2])),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Uninitialized,
    Ambiguous,
    Exhausted,
    Interrupted,
}
#[derive(Clone, Copy)]
struct Record {
    slot: usize,
    generation: u64,
    fence: Fence,
}
#[repr(C, align(4096))]
pub struct Journal {
    words: [AtomicU64; 512],
}
const _: () = assert!(core::mem::size_of::<Journal>() == 4096);
impl Journal {
    pub const fn new() -> Self {
        Self {
            words: [const { AtomicU64::new(0) }; 512],
        }
    }
    pub fn address(&self) -> u64 {
        self as *const Self as u64
    }
    fn hash(generation: u64, body: [u64; BODY_WORDS]) -> u64 {
        // Corruption check only; private KEEP bytes confer no identity/authority.
        let mut hash = 0xcbf29ce484222325u64;
        for word in [generation, VERSION].into_iter().chain(body) {
            for byte in word.to_le_bytes() {
                hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
            }
        }
        hash
    }
    fn record(&self, slot: usize) -> Result<Option<Record>, Error> {
        let at = slot * SLOT_WORDS;
        let generation = self.words[at].load(Ordering::SeqCst);
        if generation == 0 {
            return Ok(None);
        }
        if self.words[at + 1].load(Ordering::SeqCst) != VERSION {
            return Err(Error::Ambiguous);
        }
        let body = core::array::from_fn(|index| self.words[at + 2 + index].load(Ordering::SeqCst));
        if self.words[at + 10].load(Ordering::SeqCst) != Self::hash(generation, body)
            || (11..SLOT_WORDS).any(|index| self.words[at + index].load(Ordering::SeqCst) != 0)
            || self.words[at].load(Ordering::SeqCst) != generation
        {
            return Err(Error::Ambiguous);
        }
        Ok(Some(Record {
            slot,
            generation,
            fence: Fence::decode(body).ok_or(Error::Ambiguous)?,
        }))
    }
    fn latest(&self) -> Result<Record, Error> {
        let a = self.record(0)?;
        let b = self.record(1)?;
        match (a, b) {
            (None, None) => Err(Error::Uninitialized),
            (Some(record), None) | (None, Some(record)) => {
                let other = (1 - record.slot) * SLOT_WORDS;
                let dirty = (1..SLOT_WORDS)
                    .any(|index| self.words[other + index].load(Ordering::SeqCst) != 0);
                // A torn/zeroed commit beside Free cannot prove that a newer
                // Held obligation never existed. Conservative fences may remain.
                if dirty && record.fence == Fence::Free {
                    Err(Error::Ambiguous)
                } else {
                    Ok(record)
                }
            }
            (Some(a), Some(b)) if a.generation != b.generation => {
                Ok(if a.generation > b.generation { a } else { b })
            }
            _ => Err(Error::Ambiguous),
        }
    }
    pub fn load(&self) -> Result<Fence, Error> {
        self.latest().map(|record| record.fence)
    }
    pub fn all_zero(&self) -> bool {
        self.words
            .iter()
            .all(|word| word.load(Ordering::SeqCst) == 0)
    }
    /// Caller proves cold BOOT before any image admission. A restart must not use
    /// an all-zero or malformed page to infer Free.
    pub fn initialize_cold(&self) -> Result<(), Error> {
        if !self.all_zero() {
            return Err(Error::Ambiguous);
        }
        self.write(0, 1, Fence::Free, || true)
    }
    pub fn checkpoint(&self, fence: Fence) -> Result<(), Error> {
        self.checkpoint_inner(fence, || true)
    }
    fn checkpoint_inner(&self, fence: Fence, allow: impl FnMut() -> bool) -> Result<(), Error> {
        let old = self.latest()?;
        if old.fence == fence {
            return Ok(());
        }
        let next = old.generation.checked_add(1).ok_or(Error::Exhausted)?;
        self.write(1 - old.slot, next, fence, allow)
    }
    /// Replace ambiguous custody only with a conservative quarantine, never Free.
    /// An exhausted/corrupt MAX generation remains blocked until a cold reboot.
    pub fn quarantine(&self, epoch: u64) -> Result<(), Error> {
        let a = self.words[0].load(Ordering::SeqCst);
        let b = self.words[SLOT_WORDS].load(Ordering::SeqCst);
        let next = a.max(b).checked_add(1).ok_or(Error::Exhausted)?;
        let slot = if a > b { 1 } else { 0 };
        self.write(slot, next, Fence::Quarantined(epoch), || true)?;
        self.words[(1 - slot) * SLOT_WORDS].store(0, Ordering::SeqCst);
        Ok(())
    }
    fn write(
        &self,
        slot: usize,
        generation: u64,
        fence: Fence,
        mut allow: impl FnMut() -> bool,
    ) -> Result<(), Error> {
        let body = fence.words();
        if Fence::decode(body) != Some(fence) || generation == 0 {
            return Err(Error::Ambiguous);
        }
        let at = slot * SLOT_WORDS;
        let mut put = |index: usize, value: u64| {
            if !allow() {
                return Err(Error::Interrupted);
            }
            self.words[at + index].store(value, Ordering::SeqCst);
            Ok(())
        };
        put(0, 0)?;
        put(1, VERSION)?;
        for (index, word) in body.into_iter().enumerate() {
            put(index + 2, word)?;
        }
        put(10, Self::hash(generation, body))?;
        for index in 11..SLOT_WORDS {
            put(index, 0)?;
        }
        // Every field is complete before this final atomic commit marker.
        put(0, generation)
    }
    #[cfg(test)]
    pub fn checkpoint_cut(&self, fence: Fence, stores: usize) -> Result<(), Error> {
        let mut left = stores;
        self.checkpoint_inner(fence, || {
            if left == 0 {
                false
            } else {
                left -= 1;
                true
            }
        })
    }
    #[cfg(test)]
    pub fn seed_record(&self, slot: usize, generation: u64, fence: Fence) -> Result<(), Error> {
        self.write(slot, generation, fence, || true)
    }
    #[cfg(test)]
    pub fn duplicate_slot(&self) {
        for index in 0..SLOT_WORDS {
            self.words[SLOT_WORDS + index]
                .store(self.words[index].load(Ordering::SeqCst), Ordering::SeqCst);
        }
    }
    #[cfg(test)]
    pub fn fault_word(&self, index: usize, value: u64) {
        self.words[index].store(value, Ordering::SeqCst);
    }
}
