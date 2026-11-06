//! Display-private staging. Nothing in a partial transaction changes visible text.
use sex_pdx::text_batch::{hash, selected_hash, Request, CAPACITY};
/// Captured multipart identity, descriptive only. Main must keep intake closed
/// between capture, publication and release. This is not a capability or a
/// replay-resistant token across arbitrary resets/reuse of the same tuple.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    owner: u32,
    task: u32,
    surface: u64,
    sequence: u32,
}
impl Context {
    pub const fn owner(&self) -> u32 { self.owner }
    pub const fn task(&self) -> u32 { self.task }
    pub const fn surface(&self) -> u64 { self.surface }
    pub const fn sequence(&self) -> u32 { self.sequence }
}
/// Read-only borrowed view of this Stage's sealed, unreleased inline payload.
/// Projection must copy into Display-owned scene backing; no producer pointer
/// or additional authority is introduced by this view.
pub struct Pending<'a> {
    context: Context,
    len: usize,
    color: u32,
    selection: (u16, u16),
    bytes: &'a [u8; CAPACITY],
}
impl<'a> Pending<'a> {
    pub const fn context(&self) -> Context { self.context }
    pub const fn len(&self) -> usize { self.len }
    pub const fn color(&self) -> u32 { self.color }
    pub const fn selection(&self) -> (u16, u16) { self.selection }
    pub const fn bytes(&self) -> &'a [u8; CAPACITY] { self.bytes }
}
const COVERAGE_WORDS: usize = CAPACITY.div_ceil(8).div_ceil(64);
#[derive(Clone, Copy)]
pub struct Stage {
    owner: u32,
    // Admitted multipart context correlation, not task-routed reply authority.
    // Completion transport remains addressed to owner PD.
    task: u32,
    sid: u64,
    sequence: u32,
    len: usize,
    color: u32,
    /// One bit per 8-byte chunk of `bytes`.
    coverage: [u64; COVERAGE_WORDS],
    selection_start: u16,
    selection_end: u16,
    selection_present: bool,
    sealed: bool,
    poisoned: bool,
    reply_pending: bool,
    reply_released: bool,
    bytes: [u8; CAPACITY],
}
impl Stage {
    pub const EMPTY: Self = Self {
        owner: 0,
        task: 0,
        sid: 0,
        sequence: 0,
        len: 0,
        color: 0,
        coverage: [0; COVERAGE_WORDS],
        selection_start: 0,
        selection_end: 0,
        selection_present: false,
        sealed: false,
        poisoned: false,
        reply_pending: false,
        reply_released: false,
        bytes: [0; CAPACITY],
    };
    pub fn diagnostic(&self)->(u32,u64,u32,usize,u64,bool,bool){(self.owner,self.sid,self.sequence,self.len,self.coverage[0],self.sealed,self.poisoned)}
    pub fn reset(&mut self) {
        // Explicitly erase inline text and selection before terminal reuse.
        for byte in &mut self.bytes { unsafe { core::ptr::write_volatile(byte, 0); } }
        unsafe {
            core::ptr::write_volatile(&mut self.selection_start, 0);
            core::ptr::write_volatile(&mut self.selection_end, 0);
            core::ptr::write_volatile(&mut self.selection_present, false);
        }
        *self = Self::EMPTY;
    }
    /// Only expose selection after the same transaction's successful commit.
    pub fn selection(&self) -> (u16, u16) {
        if self.sealed { (self.selection_start, self.selection_end) } else { (0, 0) }
    }
    /// Every chunk of the declared length has arrived.
    fn complete(&self) -> bool {
        let chunks = (self.len + 7) / 8;
        (0..COVERAGE_WORDS).all(|w| {
            let want = chunks.saturating_sub(w * 64);
            self.coverage[w] == if want >= 64 { u64::MAX } else { (1u64 << want) - 1 }
        })
    }
    pub fn retire(&mut self, owner: u32, task: u32) {
        if (self.owner, self.task) == (owner, task) {
            self.reset();
        }
    }
    /// A sealed transaction still needs a successful Display publication.
    pub fn needs_publication(&self) -> bool { self.reply_pending && !self.reply_released }
    pub fn pending(&self) -> Option<Pending<'_>> {
        if !self.sealed || self.poisoned || !self.needs_publication() { return None; }
        Some(Pending {
            context: Context { owner: self.owner, task: self.task, surface: self.sid, sequence: self.sequence },
            len: self.len,
            color: self.color,
            selection: self.selection(),
            bytes: &self.bytes,
        })
    }
    /// Release only the still-unreleased context captured for this publication.
    /// The same closed-intake/SceneLease boundary as release_after_publication
    /// is required; tuple matching alone is not proof that rendering occurred.
    pub fn release_matching(&mut self, context: Context) -> bool {
        if !self.pending().is_some_and(|pending| pending.context() == context) { return false; }
        self.release_after_publication()
    }
    /// Main calls this only after successful publication and SceneLease release.
    /// This changes eligibility, not the retained PD-routed reply's identity.
    pub fn release_after_publication(&mut self) -> bool {
        if !self.needs_publication() { return false; }
        self.reply_released = true;
        true
    }
    /// One retained completion attempt per display pass. Actual reply transport
    /// preserves sender identity. Legacy syscall 29 collapses Full and denial
    /// into nonzero status, so retain every nonzero result until admitted or
    /// authoritative surface/app cleanup cancels it. Never replay the mutation.
    pub fn poll_reply(&mut self, mut reply: impl FnMut(u32, u64) -> u64) {
        if !self.reply_pending || !self.reply_released {
            return;
        }
        let Ok(surface) = u16::try_from(self.sid) else {
            self.reply_pending = false;
            return;
        };
        let Some(value) = (sex_pdx::text_batch::Applied {
            surface,
            sequence: self.sequence,
        })
        .encode() else {
            self.reply_pending = false;
            return;
        };
        if reply(self.owner, value) == 0 {
            self.reply_pending = false;
            self.reply_released = false;
        }
    }
    /// Caller identity and live surface ownership must already be attested by display.
    /// A new (owner,task,sid) binding cannot consume a previous binding's partial bytes.
    pub fn receive(
        &mut self,
        owner: u32,
        task: u32,
        sid: u64,
        request: Request,
    ) -> Option<(usize, u32, &[u8; CAPACITY])> {
        // Applied carries a 16-bit SID. Reject unencodable values before any
        // staging/reset/poison effect; this is a wire bound, not surface authority.
        if owner == 0 || task == 0 || sid == 0 || sid > u16::MAX as u64 {
            return None;
        }
        match request {
            Request::Begin {
                sequence,
                len,
                color,
            } => {
                // Neither an unreleased nor a Full-retained completion may be
                // replaced or poisoned by another Begin, even another tuple.
                if self.reply_pending { return None; }
                if sequence == 0 || len > CAPACITY {
                    return None;
                }
                if (self.owner, self.task, self.sid) != (owner, task, sid) {
                    self.reset();
                }
                if sequence < self.sequence {
                    return None;
                }
                if sequence == self.sequence {
                    if self.len != len || self.color != color {
                        self.poisoned = true;
                    }
                    return None;
                }
                self.reset();
                *self = Self {
                    owner,
                    task,
                    sid,
                    sequence,
                    len,
                    color,
                    ..Self::EMPTY
                };
            }
            Request::Chunk {
                sequence,
                offset,
                count,
                bytes,
            } => {
                if (self.owner, self.task, self.sid, self.sequence) != (owner, task, sid, sequence)
                    || self.sealed
                    || self.poisoned
                {
                    return None;
                }
                if self.selection_present || offset >= self.len
                    || offset % 8 != 0
                    || count != (self.len - offset).min(8)
                    || bytes[count..].iter().any(|&v| v != 0)
                {
                    self.poisoned = true;
                    return None;
                }
                let (word, bit) = (offset / 8 / 64, 1u64 << (offset / 8 % 64));
                if self.coverage[word] & bit != 0 && self.bytes[offset..offset + count] != bytes[..count]
                {
                    self.poisoned = true;
                    return None;
                }
                self.bytes[offset..offset + count].copy_from_slice(&bytes[..count]);
                self.coverage[word] |= bit;
            }
            Request::Select { sequence, start, end } => {
                if (self.owner, self.task, self.sid, self.sequence) != (owner, task, sid, sequence)
                    || self.sealed || self.poisoned { return None; }
                if !self.complete() || start > end || end > self.len
                    || !sex_pdx::text_encoding::is_boundary(&self.bytes[..self.len],start)
                    || !sex_pdx::text_encoding::is_boundary(&self.bytes[..self.len],end)
                    || (start == end && start != 0)
                    || (self.selection_present && (self.selection_start as usize, self.selection_end as usize) != (start, end)) {
                    self.poisoned = true;
                    return None;
                }
                self.selection_start = start as u16;
                self.selection_end = end as u16;
                self.selection_present = true;
            }
            Request::Commit {
                sequence,
                hash: expected,
            } => {
                if (self.owner, self.task, self.sid, self.sequence) != (owner, task, sid, sequence)
                    || self.sealed
                    || self.poisoned
                {
                    return None;
                }
                let integrity = if self.selection_present {
                    selected_hash(&self.bytes[..self.len], self.selection_start as usize, self.selection_end as usize)
                } else { Some(hash(&self.bytes[..self.len])) };
                if !self.complete() || integrity != Some(expected)
                    || !sex_pdx::text_encoding::valid_text(&self.bytes[..self.len]) {
                    self.poisoned = true;
                    return None;
                }
                self.sealed = true;
                self.reply_pending = true;
                return Some((self.len, self.color, &self.bytes));
            }
        }
        None
    }
}
