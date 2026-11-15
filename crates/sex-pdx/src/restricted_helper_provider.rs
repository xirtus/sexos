//! Bounded provider-side policy runtime. Server hooks supply actual scrub/job
//! effects; no ACK is published until those effects and the durable deny fence.
use crate::restricted_helper::{
    self as w, Accepted, Authority, Binding, Packet, Phase, ReplyCredits, Subject, Transcript,
};
use crate::restricted_helper_journal::{Journal, Ledger};
pub const TRANSCRIPT_TICKS: u64 = 5_000;
pub const JOB_TICKS: u64 = 120_000;
#[derive(Clone, Copy)]
pub struct Message {
    pub caller: Subject,
    pub opcode: u64,
    pub words: [u64; 3],
}
#[derive(Clone, Copy)]
pub enum Effect {
    Scrub(Subject),
    Install(Binding),
    Expire(Binding),
}
pub trait Platform {
    fn epoch(&mut self) -> Option<u64>;
    fn keep(&mut self, address: u64) -> bool;
    fn challenge(&mut self) -> Option<[u64; 2]>;
    fn catalog(&mut self, target: u32) -> Option<Subject>;
    fn now(&mut self) -> u64;
    fn reply(&mut self, value: u64) -> bool;
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingReply {
    First,
    Fetch([u64; 3]),
}
#[derive(Clone, Copy)]
struct Committed {
    binding: Binding,
    until: u64,
}
pub struct Provider {
    ledger: Ledger,
    epoch: Option<u64>,
    transcript: Option<Transcript>,
    credits: Option<ReplyCredits>,
    last: Option<Packet>,
    challenge: [u64; 2],
    effect: Option<Effect>,
    reply: Option<PendingReply>,
    deadline: u64,
    committed: [Option<Committed>; 16],
    committed_generation: u64,
}
impl Provider {
    pub const fn new() -> Self {
        Self {
            ledger: Ledger::quarantine_all(),
            epoch: None,
            transcript: None,
            credits: None,
            last: None,
            challenge: [0; 2],
            effect: None,
            reply: None,
            deadline: 0,
            committed: [None; 16],
            committed_generation: 0,
        }
    }
    /// Call before deferred replay, requests, retained replies or periodic rings.
    pub fn initialize(&mut self, journal: &Journal, p: &mut impl Platform) {
        *self = Self::new();
        let epoch = p.epoch();
        let kept = p.keep(journal.address());
        self.ledger = journal.load(epoch, kept);
        // Commit even the known-empty cold state BEFORE normal publication.
        // Otherwise an unused service's first restart sees zero bytes/epoch2
        // and needlessly quarantines every ordinary Program row.
        if kept {
            journal.persist(&self.ledger);
        }
        self.epoch = epoch;
        self.committed_generation = self.ledger.maximum_generation();
        // Recovered tombstones confer no live job, transcript or reply authority.
    }
    pub fn denied(&self, pd: u32) -> bool {
        self.ledger.denied(pd)
    }
    /// External job maps are provisional until a complete Seal ACK. Every data
    /// path must check this predicate; reset/unknown epoch/global denial override
    /// a stale external map. Previously sealed jobs keep only their exact120s lease.
    pub fn job_live(&self, binding: Binding, now: u64) -> bool {
        !self.ledger.all_denied()
            && self
                .committed
                .iter()
                .flatten()
                .any(|job| job.binding == binding && now < job.until)
    }
    pub fn pending(&self) -> bool {
        self.deadline != 0
            || self.reply.is_some()
            || self.effect.is_some()
            || self.committed.iter().any(|job| job.is_some())
    }
    pub fn binding(&self) -> Option<Binding> {
        if !self.sealed() {
            return None;
        }
        self.transcript.as_ref().and_then(|t| t.binding())
    }
    pub fn sealed(&self) -> bool {
        self.effect.is_none()
            && self.transcript.as_ref().is_some_and(|t| t.sealed())
            && self.credits.as_ref().is_some_and(|c| c.complete())
    }
    pub fn message(&mut self, msg: Message, journal: &Journal, p: &mut impl Platform) -> bool {
        if msg.opcode != w::OP_POLICY && msg.opcode != w::OP_FETCH {
            return false;
        }
        if msg.caller != (Subject { pd: 3, task: 3 }) {
            return true;
        }
        let Some(epoch) = self.epoch else { return true };
        if p.epoch() != Some(epoch) {
            self.abandon();
            self.ledger = Ledger::quarantine_all();
            return true;
        }
        if msg.opcode == w::OP_FETCH {
            if self.effect.is_some() {
                return true;
            }
            let Some(credits) = self.credits.as_ref() else {
                return true;
            };
            if credits.fetch(msg.words, epoch).is_none() {
                return true;
            }
            if self.reply.is_none() || self.reply == Some(PendingReply::Fetch(msg.words)) {
                self.reply = Some(PendingReply::Fetch(msg.words));
            }
            return true;
        }
        let Some(packet) = Packet::decode(msg.words) else {
            return true;
        };
        let duplicate = self.last == Some(packet);
        if !duplicate
            && (self.effect.is_some() || self.credits.as_ref().is_some_and(|c| !c.complete()))
        {
            return true;
        }
        let mut challenge = self.challenge;
        if packet.phase == Phase::Quarantine && !duplicate {
            if self.ledger.all_denied() || packet.generation <= self.ledger.maximum_generation() {
                return true;
            }
            let Some(subject) = Subject::decode(packet.a) else {
                return true;
            };
            if p.catalog(subject.pd) != Some(subject) {
                return true;
            }
            let Some(fresh) = p.challenge().filter(|v| *v != [0; 2]) else {
                return true;
            };
            challenge = fresh;
            self.transcript = Some(Transcript::new());
        }
        let Some(previous) = self
            .transcript
            .as_ref()
            .and_then(|t| t.binding())
            .map(|b| b.previous)
            .or_else(|| {
                if packet.phase == Phase::Quarantine {
                    Subject::decode(packet.a)
                } else {
                    None
                }
            })
        else {
            return true;
        };
        let child = if matches!(packet.phase, Phase::Bind | Phase::Seal) {
            Subject::decode(packet.a).and_then(|s| p.catalog(s.pd))
        } else {
            None
        };
        // Only trusted Shell can reserve the row. Providers attest catalog equality;
        // info(LIVE) does not distinguish an unused Constructed row from live work.
        let authority = Authority {
            caller: msg.caller,
            provider_epoch: epoch,
            reserved: previous,
            inactive: true,
            challenge,
        };
        let Some(t) = self.transcript.as_mut() else {
            return true;
        };
        let Ok(accepted) = t.accept(packet, authority, child) else {
            return true;
        };
        if accepted == Accepted::Quarantined {
            if !self.ledger.quarantine(previous, packet.generation) {
                self.abandon();
                return true;
            }
            journal.persist(&self.ledger);
            for job in &mut self.committed {
                if job.is_some_and(|job| job.binding.child.pd == previous.pd) {
                    *job = None;
                }
            }
            self.challenge = challenge;
            self.effect = Some(Effect::Scrub(previous));
            self.deadline = p.now().saturating_add(TRANSCRIPT_TICKS);
        } else if accepted == Accepted::Sealed {
            let binding = t.binding().unwrap();
            if !self
                .ledger
                .bind_task(binding.previous, binding.child, binding.generation)
            {
                self.abandon();
                return true;
            }
            journal.persist(&self.ledger);
            self.effect = Some(Effect::Install(binding));
        }
        self.last = Some(packet);
        if self.effect.is_none() {
            self.arm_reply(packet, epoch)
        }
        true
    }
    fn arm_reply(&mut self, packet: Packet, epoch: u64) {
        self.credits = ReplyCredits::new(
            packet,
            epoch,
            if packet.phase == Phase::Quarantine {
                self.challenge
            } else {
                [0; 2]
            },
        );
        self.reply = Some(PendingReply::First);
    }
    fn abandon(&mut self) {
        self.transcript = None;
        self.credits = None;
        self.last = None;
        self.reply = None;
        self.effect = None;
        self.deadline = 0;
        for word in &mut self.challenge {
            unsafe { core::ptr::write_volatile(word, 0) }
        }
    }
    /// Retry effects and reply admission in the existing provider poll, never wait.
    /// Effects must themselves revalidate any retained job/task facts before success.
    pub fn poll(&mut self, p: &mut impl Platform, mut effect: impl FnMut(Effect) -> bool) {
        let Some(epoch) = self.epoch else { return };
        if p.epoch() != Some(epoch) {
            self.ledger = Ledger::quarantine_all();
            self.abandon();
            return;
        }
        let now = p.now();
        for job in &mut self.committed {
            if let Some(current) = job.filter(|job| now >= job.until) {
                // Expired data rights are already false even if cleanup retries.
                if effect(Effect::Expire(current.binding)) {
                    *job = None;
                }
            }
        }
        if self.deadline != 0 && p.now() >= self.deadline {
            self.abandon();
            return;
        }
        if let Some(pending) = self.effect {
            if !effect(pending) {
                return;
            }
            self.effect = None;
            if let Some(packet) = self.last {
                self.arm_reply(packet, epoch)
            }
        }
        let Some(reply) = self.reply else { return };
        let Some(credits) = self.credits.as_mut() else {
            return;
        };
        let word = match reply {
            PendingReply::First => credits.first(epoch),
            PendingReply::Fetch(request) => credits.fetch(request, epoch),
        };
        let Some(word) = word else {
            self.reply = None;
            return;
        };
        if !p.reply(word) {
            return;
        }
        match reply {
            PendingReply::First => {
                credits.first_delivered(epoch);
            }
            PendingReply::Fetch(request) => {
                credits.fetch_delivered(request, epoch);
            }
        }
        self.reply = None;
        if self.sealed() {
            let binding = self.transcript.as_ref().and_then(|t| t.binding()).unwrap();
            if binding.generation > self.committed_generation
                && !self
                    .committed
                    .iter()
                    .flatten()
                    .any(|job| job.binding == binding)
            {
                let Some(slot) = self.committed.iter_mut().find(|job| job.is_none()) else {
                    self.abandon();
                    return;
                };
                *slot = Some(Committed {
                    binding,
                    until: now.saturating_add(JOB_TICKS),
                });
                self.committed_generation = binding.generation;
            }
            self.deadline = 0;
        }
    }
}
