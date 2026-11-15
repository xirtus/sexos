//! Stop-and-wait one-word control replies, scoped to one immutable job and Files
//! epoch. No short digest is used as authority. Wrapper authenticates Shell's
//! stamped PD3/task3 and current job ownership on every request and send.
pub const WORDS: usize = 22;
pub const CLAIM_TAG: u64 = 0xd1;
pub struct Control {
    pub ticket: u32,
    pub epoch: u64,
    pub claim: [u64; 3],
    info: [u64; WORDS],
    next: usize,
    ack: bool,
    pending: Option<Option<usize>>,
    last: Option<usize>,
    until: u64,
}
impl Control {
    pub fn new(claim: [u64; 3], epoch: u64, info: [u8; 176], now: u64) -> Option<Self> {
        let ticket = u32::try_from(claim[0]).ok()?;
        if ticket == 0 || ticket > 0xff_ffff || epoch == 0 {
            return None;
        }
        Some(Self {
            ticket,
            epoch,
            claim,
            info: core::array::from_fn(|i| {
                u64::from_le_bytes(info[i * 8..i * 8 + 8].try_into().unwrap())
            }),
            next: 0,
            ack: false,
            pending: Some(None),
            last: None,
            until: now.checked_add(5_000)?,
        })
    }
    pub fn valid(&self, epoch: u64, now: u64) -> bool {
        self.epoch == epoch && now < self.until
    }
    pub fn repeat_claim(&mut self, claim: [u64; 3], epoch: u64, now: u64) -> bool {
        if !self.valid(epoch, now)
            || claim != self.claim
            || self.next != 0
            || self.pending.is_some_and(|v| v.is_some())
        {
            return false;
        }
        self.pending = Some(None);
        true
    }
    pub fn fetch(&mut self, words: [u64; 3], now: u64) -> bool {
        let Ok(selector) = usize::try_from(words[1]) else {
            return false;
        };
        if !self.valid(words[2], now)
            || words[0] != self.ticket as u64
            || !self.ack
            || selector >= WORDS
        {
            return false;
        }
        if let Some(pending) = self.pending {
            return pending == Some(selector);
        }
        if selector != self.next && self.last != Some(selector) {
            return false;
        }
        self.pending = Some(Some(selector));
        true
    }
    pub fn reply(&self) -> Option<u64> {
        Some(match self.pending? {
            None => CLAIM_TAG << 56 | (self.ticket as u64) << 32,
            Some(i) => self.info[i],
        })
    }
    pub fn delivered(&mut self) {
        match self.pending.take() {
            Some(None) => self.ack = true,
            Some(Some(i)) => {
                if i == self.next {
                    self.next += 1;
                }
                self.last = Some(i)
            }
            None => {}
        }
    }
    pub fn complete(&self) -> bool {
        self.ack && self.next == WORDS && self.pending.is_none()
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        for word in &mut self.info {
            unsafe { core::ptr::write_volatile(word, 0) }
        }
        for word in &mut self.claim {
            unsafe { core::ptr::write_volatile(word, 0) }
        }
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }
}
