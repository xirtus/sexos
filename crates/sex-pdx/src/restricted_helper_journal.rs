//! Private KEEP page containing only deny tombstones, never positive authority.
//! Register before intake/replay, recover before all periodic provider work.
use crate::restricted_helper::{Subject, MAX_GENERATION};
use core::sync::atomic::{AtomicU64, Ordering};
const MAGIC: u64 = 0x4d4f414e_44454e59;
const VERSION: u64 = 1;
pub const CAPACITY: usize = 16;
const WORDS: usize = 512;
const HEADER: usize = 8;
const STRIDE: usize = 4;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tombstone {
    pub subject: Subject,
    pub generation: u64,
}
#[derive(Clone, Copy)]
pub struct Ledger {
    all_denied: bool,
    entries: [Option<Tombstone>; CAPACITY],
}
impl Ledger {
    pub const fn empty() -> Self {
        Self {
            all_denied: false,
            entries: [None; CAPACITY],
        }
    }
    pub const fn quarantine_all() -> Self {
        Self {
            all_denied: true,
            entries: [None; CAPACITY],
        }
    }
    pub fn all_denied(&self) -> bool {
        self.all_denied
    }
    /// Caller separately limits this check to actual Program catalog rows.
    pub fn denied(&self, pd: u32) -> bool {
        self.all_denied || self.entries.iter().flatten().any(|e| e.subject.pd == pd)
    }
    pub fn quarantine(&mut self, subject: Subject, generation: u64) -> bool {
        if subject.pd == 0 || subject.task == 0 || generation == 0 || generation > MAX_GENERATION {
            return false;
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.is_some_and(|e| e.subject.pd == subject.pd))
        {
            let previous = entry.unwrap();
            if generation < previous.generation {
                return false;
            }
            if generation == previous.generation {
                return previous.subject == subject;
            }
            *entry = Some(Tombstone {
                subject,
                generation,
            });
            return true;
        }
        let Some(entry) = self.entries.iter_mut().find(|e| e.is_none()) else {
            return false;
        };
        *entry = Some(Tombstone {
            subject,
            generation,
        });
        true
    }
    pub fn words(&self) -> [u64; WORDS] {
        let mut words = [0; WORDS];
        words[0] = MAGIC;
        words[1] = VERSION;
        words[2] = self.all_denied as u64;
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some(entry) = entry {
                let at = HEADER + index * STRIDE;
                words[at] = entry.subject.word();
                words[at + 1] = entry.generation;
                words[at + 2] = 1;
            }
        }
        words[3] = checksum(&words);
        words
    }
    /// An invalid/restarted cold page loses availability, never denial fences.
    pub fn recover(words: &[u64; WORDS], self_epoch: Option<u64>) -> Self {
        if self_epoch == Some(1) && words.iter().all(|w| *w == 0) {
            return Self::empty();
        }
        if self_epoch.is_none()
            || self_epoch == Some(0)
            || words[0] != MAGIC
            || words[1] != VERSION
            || words[2] > 1
            || words[3] != checksum(words)
            || words[4..HEADER].iter().any(|w| *w != 0)
            || words[HEADER + CAPACITY * STRIDE..].iter().any(|w| *w != 0)
        {
            return Self::quarantine_all();
        }
        let mut ledger = Self {
            all_denied: words[2] != 0,
            entries: [None; CAPACITY],
        };
        for index in 0..CAPACITY {
            let at = HEADER + index * STRIDE;
            if words[at..at + STRIDE].iter().all(|w| *w == 0) {
                continue;
            }
            let Some(subject) = Subject::decode(words[at]) else {
                return Self::quarantine_all();
            };
            if words[at + 2] != 1
                || words[at + 3] != 0
                || words[at + 1] == 0
                || words[at + 1] > MAX_GENERATION
                || ledger
                    .entries
                    .iter()
                    .flatten()
                    .any(|e| e.subject.pd == subject.pd)
            {
                return Self::quarantine_all();
            }
            ledger.entries[index] = Some(Tombstone {
                subject,
                generation: words[at + 1],
            });
        }
        ledger
    }
}
fn checksum(words: &[u64; WORDS]) -> u64 {
    // Private-page corruption/torn-commit check; not a capability or a MAC.
    let mut hash = 0xcbf29ce484222325u64;
    for (index, word) in words.iter().enumerate() {
        if index == 3 {
            continue;
        }
        for byte in word.to_le_bytes() {
            hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3)
        }
    }
    hash
}
#[repr(C, align(4096))]
pub struct Journal {
    words: [AtomicU64; WORDS],
}
impl Journal {
    pub const fn new() -> Self {
        Self {
            words: [const { AtomicU64::new(0) }; WORDS],
        }
    }
    pub fn address(&self) -> u64 {
        self as *const Self as u64
    }
    pub fn load(&self, self_epoch: Option<u64>, keep_succeeded: bool) -> Ledger {
        if !keep_succeeded {
            return Ledger::quarantine_all();
        }
        let before = self.words[0].load(Ordering::SeqCst);
        let words = core::array::from_fn(|i| self.words[i].load(Ordering::SeqCst));
        if before != self.words[0].load(Ordering::SeqCst) {
            return Ledger::quarantine_all();
        }
        Ledger::recover(&words, self_epoch)
    }
    /// Sole provider main thread calls; never concurrent with another writer.
    /// Invalidate first and publish magic last, so interrupted writes deny all.
    pub fn persist(&self, ledger: &Ledger) {
        let words = ledger.words();
        self.words[0].store(0, Ordering::SeqCst);
        for (index, word) in words.iter().enumerate().skip(1) {
            self.words[index].store(*word, Ordering::SeqCst)
        }
        self.words[0].store(MAGIC, Ordering::SeqCst);
    }
}
