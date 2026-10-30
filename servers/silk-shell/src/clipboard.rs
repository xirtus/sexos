// SPDX-License-Identifier: GPL-3.0-only
// Fixed BOOT participants and register-only text custody. Caller identity and
// liveness come from the held lifecycle catalog, never from request arguments.
use sex_pdx::{app_lifecycle::CatalogInfo, clipboard::*, text_selection::Source};

pub const TOTAL_TURNS: u32 = 65_536;
pub const STALL_TURNS: u32 = 4_096;
pub const TOTAL_TICKS: u64 = 15_000;
pub const STALL_TICKS: u64 = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission { Accepted, Full, Rejected(u64) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Publication {
    pub owner: Subject,
    pub source: Source,
    pub len: usize,
    pub generation: u32,
    pub hash: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Effect { Intent, Begin, Read, Data(usize), Finish, Receipt }
struct Pending { packet: Packet, effect: Effect }
struct Slot {
    participant: Option<Participant>,
    retired: bool,
    live: bool,
    ticket: u32,
    pending: Option<Pending>,
    // Only scalar Meta receipts are retained. No second copy of a data chunk.
    receipt: Option<Response>,
}
impl Slot {
    const fn new() -> Self {
        Self { participant: None, retired: false, live: false, ticket: 0, pending: None, receipt: None }
    }
    fn erase_pending(&mut self) {
        if let Some(p) = self.pending.as_mut() { p.packet.erase(); }
        self.pending = None;
    }
    fn put(&mut self, response: Response, effect: Effect) {
        self.erase_pending();
        if matches!(response, Response::Meta { .. }) { self.receipt = Some(response); }
        self.pending = Some(Pending { packet: response.wire(), effect });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { Offered, Upload, Read }
struct Transaction {
    owner: usize,
    ticket: u32,
    action: Action,
    mode: Mode,
    intent_admitted: bool,
    ready: bool,
    poisoned: bool,
    source: Source,
    len: usize,
    offset: usize,
    producer: usize,
    generation: u32,
    hash: u64,
    born: u64,
    progress: u64,
    last_now: u64,
    turns: u32,
    stalled: u32,
}

pub struct Broker {
    slots: [Slot; 4],
    transaction: Option<Transaction>,
    publication: Option<Publication>,
    stage: [u8; MAX_BYTES],
    committed: [u8; MAX_BYTES],
    read_snapshot: [u8; MAX_BYTES],
    ticket: u32,
    generation: u32,
    cursor: usize,
}

fn erase(bytes: &mut [u8]) {
    for byte in bytes { unsafe { core::ptr::write_volatile(byte, 0); } }
}
fn meta(ticket: u32, phase: Phase, status: Status, value: Option<Publication>) -> Response {
    let (len, generation, hash) = if status == Status::Ok {
        value.map(|p| (p.len, p.generation, p.hash)).unwrap_or((0, 0, 0))
    } else { (0, 0, 0) };
    Response::Meta { ticket, phase, status, len, generation, hash }
}

impl Broker {
    pub const fn new() -> Self {
        Self {
            slots: [const { Slot::new() }; 4], transaction: None, publication: None,
            stage: [0; MAX_BYTES], committed: [0; MAX_BYTES], read_snapshot: [0; MAX_BYTES],
            ticket: 0, generation: 0, cursor: 0,
        }
    }
    /// Registration never replaces a catalog or clears a retirement fence.
    pub fn register(&mut self, catalog: CatalogInfo) -> Result<Participant, Status> {
        let p = Participant::from_catalog(catalog).ok_or(Status::Denied)?;
        let index = match p.subject.pd { 9 => 0, 15 => 1, 16 => 2, 18 => 3, _ => return Err(Status::Denied) };
        let slot = &mut self.slots[index];
        match slot.participant {
            Some(old) if old == p => Ok(old),
            Some(_) => Err(Status::Denied),
            None => { slot.participant = Some(p); Ok(p) }
        }
    }
    fn index(&self, subject: Subject) -> Option<usize> {
        self.slots.iter().position(|s| s.participant.is_some_and(|p| p.subject == subject))
    }
    fn admitted_subject(&mut self, subject: Subject, live: bool) -> Result<usize, Status> {
        let index = self.index(subject).ok_or(Status::Denied)?;
        if !live || self.slots[index].retired { return Err(Status::Denied); }
        self.slots[index].live = true;
        Ok(index)
    }
    pub fn publication(&self) -> Option<Publication> { self.publication }
    /// Includes an unfinished transaction with no queued reply: Shell must
    /// continue bounded polling so frozen-clock turn deadlines still run.
    pub fn pending(&self) -> bool { self.transaction.is_some() || self.outbox_pending() }
    pub fn outbox_pending(&self) -> bool { self.slots.iter().any(|s| s.pending.is_some()) }
    pub fn busy(&self) -> bool { self.transaction.is_some() }

    /// Called only after Shell has authorized the explicit interaction using
    /// its held catalog/route. A numeric subject alone never authorizes input.
    pub fn issue(&mut self, subject: Subject, action: Action, live: bool, now: u64) -> Result<u32, Status> {
        let index = self.admitted_subject(subject, live)?;
        let p = self.slots[index].participant.unwrap();
        if !p.allows(action) { return Err(Status::Denied); }
        self.deadline(now, false);
        if self.transaction.is_some() || self.slots[index].pending.is_some() { return Err(Status::Busy); }
        let ticket = self.ticket.checked_add(1).ok_or(Status::Exhausted)?;
        self.ticket = ticket;
        self.slots[index].ticket = ticket;
        self.slots[index].receipt = None;
        self.slots[index].put(Response::Intent { ticket, action }, Effect::Intent);
        self.transaction = Some(Transaction {
            owner: index, ticket, action, mode: Mode::Offered, intent_admitted: false, ready: false,
            poisoned: false, source: Source { generation: 0, revision: 0 }, len: 0, offset: 0,
            producer: index, generation: 0, hash: 0, born: now, progress: now, last_now: now, turns: 0, stalled: 0,
        });
        Ok(ticket)
    }
    fn progress(&mut self, now: u64) {
        if let Some(t) = self.transaction.as_mut() { t.progress = now; t.stalled = 0; }
    }
    fn clear_transaction(&mut self) {
        erase(&mut self.stage); erase(&mut self.read_snapshot); self.transaction = None;
    }
    fn fail(&mut self, index: usize, ticket: u32, phase: Phase, status: Status) {
        if self.transaction.as_ref().is_some_and(|t| t.owner == index && t.ticket == ticket) {
            self.clear_transaction();
        }
        self.slots[index].put(meta(ticket, phase, status, None), Effect::Receipt);
    }
    fn deadline(&mut self, now: u64, turn: bool) {
        let Some(t) = self.transaction.as_mut() else { return; };
        if turn { t.turns = t.turns.saturating_add(1); t.stalled = t.stalled.saturating_add(1); }
        let expired = now < t.last_now || now.saturating_sub(t.born) >= TOTAL_TICKS
            || now.saturating_sub(t.progress) >= STALL_TICKS || t.turns >= TOTAL_TURNS || t.stalled >= STALL_TURNS;
        t.last_now = now;
        if expired {
            let phase = match t.mode { Mode::Upload => Phase::Publish, Mode::Read => Phase::Finish,
                Mode::Offered if t.action == Action::Paste => Phase::Read, Mode::Offered => Phase::Begin };
            let (index, ticket) = (t.owner, t.ticket);
            self.fail(index, ticket, phase, Status::Uncertain);
        }
    }
    fn poison(&mut self) {
        erase(&mut self.stage);
        if let Some(t) = self.transaction.as_mut() { t.poisoned = true; }
    }
    /// No response is queued for unauthenticated, foreign-ticket or busy
    /// traffic. It therefore cannot overwrite another participant's outbox.
    pub fn receive(&mut self, subject: Subject, request: Request, live: bool, now: u64) -> Result<(), Status> {
        let index = self.admitted_subject(subject, live)?;
        let ticket = request.ticket();
        if ticket == 0 || self.slots[index].ticket != ticket { return Err(Status::Denied); }
        self.deadline(now, false);
        // Even direct typed callers must satisfy the actual wire ABI.
        let mut packet = request.wire();
        let canonical = Request::decode(packet.opcode, packet.args) == Some(request);
        packet.erase();
        if !canonical {
            if matches!(request, Request::Write { .. }) && self.transaction.as_ref().is_some_and(|t| t.owner == index && t.ticket == ticket && t.mode == Mode::Upload) { self.poison(); }
            return Err(Status::Invalid);
        }
        if matches!(request, Request::Cancel { .. }) {
            if self.transaction.as_ref().is_some_and(|t| t.owner == index && t.ticket == ticket) { self.clear_transaction(); }
            self.slots[index].put(meta(ticket, Phase::Cancel, Status::Ok, None), Effect::Receipt);
            return Ok(());
        }
        if matches!(request, Request::Query { .. }) {
            if self.slots[index].pending.is_some() { return Ok(()); }
            let response = self.slots[index].receipt.ok_or(Status::Busy)?;
            self.slots[index].put(response, Effect::Receipt);
            return Ok(());
        }
        let t = self.transaction.as_ref().filter(|t| t.owner == index && t.ticket == ticket).ok_or(Status::Denied)?;
        if !t.intent_admitted || self.slots[index].pending.is_some() { return Err(Status::Busy); }
        match request {
            Request::Begin { len, source, .. } => {
                if t.mode != Mode::Offered || t.action == Action::Paste { return Err(Status::Denied); }
                erase(&mut self.stage);
                let t = self.transaction.as_mut().unwrap();
                t.mode = Mode::Upload; t.len = len; t.source = source;
                self.slots[index].put(Response::Meta { ticket, phase: Phase::Begin, status: Status::Ok, len, generation: 0, hash: 0 }, Effect::Begin);
                self.progress(now);
            }
            Request::Write { offset, count, mut bytes, .. } => {
                let valid = t.mode == Mode::Upload && t.ready && !t.poisoned && offset == t.offset
                    && count == core::cmp::min(16, t.len.saturating_sub(t.offset)) && count != 0 && text(&bytes[..count]);
                if !valid {
                    erase(&mut bytes);
                    if t.mode == Mode::Upload { self.poison(); }
                    return Err(Status::Invalid);
                }
                self.stage[offset..offset + count].copy_from_slice(&bytes[..count]); erase(&mut bytes);
                self.transaction.as_mut().unwrap().offset += count;
                self.progress(now);
            }
            Request::Publish { len, hash: supplied, .. } => {
                if t.mode != Mode::Upload || !t.ready { return Err(Status::Denied); }
                if t.poisoned || len != t.len || t.offset != t.len || hash(&self.stage[..t.len]) != supplied {
                    self.fail(index, ticket, Phase::Publish, Status::Invalid); return Err(Status::Invalid);
                }
                let Some(generation) = self.generation.checked_add(1) else {
                    self.fail(index, ticket, Phase::Publish, Status::Exhausted); return Err(Status::Exhausted);
                };
                let publication = Publication { owner: subject, source: t.source, len, generation, hash: supplied };
                // Old publication and all unused tails are erased before the
                // replacement becomes readable. No buffer is moved/copied whole.
                erase(&mut self.committed); self.committed[..len].copy_from_slice(&self.stage[..len]);
                self.generation = generation; self.publication = Some(publication);
                self.clear_transaction();
                self.slots[index].put(meta(ticket, Phase::Publish, Status::Ok, Some(publication)), Effect::Receipt);
            }
            Request::Read { .. } => {
                if t.mode != Mode::Offered || t.action != Action::Paste { return Err(Status::Denied); }
                let Some(publication) = self.publication else {
                    self.fail(index, ticket, Phase::Read, Status::Empty); return Ok(());
                };
                let producer = self.index(publication.owner).ok_or(Status::Invalidated)?;
                if self.slots[producer].retired || !self.slots[producer].live {
                    self.fail(index, ticket, Phase::Read, Status::Invalidated); return Err(Status::Invalidated);
                }
                erase(&mut self.read_snapshot); self.read_snapshot[..publication.len].copy_from_slice(&self.committed[..publication.len]);
                let t = self.transaction.as_mut().unwrap();
                t.mode = Mode::Read; t.len = publication.len; t.producer = producer; t.source = publication.source;
                t.generation = publication.generation; t.hash = publication.hash;
                self.slots[index].put(meta(ticket, Phase::Read, Status::Ok, Some(publication)), Effect::Read);
                self.progress(now);
            }
            Request::Get { offset, .. } => {
                if t.mode != Mode::Read || !t.ready || offset != t.offset || offset >= t.len { return Err(Status::Invalid); }
                let count = core::cmp::min(16, t.len - offset); let mut bytes = [0; 16];
                bytes[..count].copy_from_slice(&self.read_snapshot[offset..offset + count]);
                self.slots[index].put(Response::Data { ticket, offset, count, bytes }, Effect::Data(count)); erase(&mut bytes);
            }
            Request::Finish { generation, hash, .. } => {
                if t.mode != Mode::Read || !t.ready || t.offset != t.len || t.generation != generation || t.hash != hash { return Err(Status::Invalid); }
                let value = Publication { owner: self.slots[t.producer].participant.unwrap().subject, source: t.source, len: t.len, generation, hash };
                self.slots[index].put(meta(ticket, Phase::Finish, Status::Ok, Some(value)), Effect::Finish);
            }
            Request::Cancel { .. } | Request::Query { .. } => unreachable!(),
        }
        Ok(())
    }
    /// At most four IPC attempts, round-robin, with no busy wait. Full is
    /// pre-effect and retains identical packet words; only Accepted advances.
    pub fn poll<F: FnMut(Participant, Packet) -> Admission>(&mut self, now: u64, budget: usize, mut send: F) -> usize {
        self.deadline(now, true);
        let mut attempts = 0;
        for _ in 0..4 {
            if attempts >= core::cmp::min(4, budget) { break; }
            let index = self.cursor; self.cursor = (self.cursor + 1) % 4;
            let slot = &self.slots[index];
            let Some(pending) = slot.pending.as_ref() else { continue; };
            let participant = slot.participant.unwrap();
            let (mut packet, effect) = (pending.packet, pending.effect);
            let result = send(participant, packet); packet.erase(); attempts += 1;
            match result {
                Admission::Full => {},
                Admission::Rejected(_) => { self.retire(participant.subject); },
                Admission::Accepted => {
                    self.slots[index].erase_pending();
                    if let Some(t) = self.transaction.as_mut().filter(|t| t.owner == index && t.ticket == self.slots[index].ticket) {
                        let progressed = match effect {
                            Effect::Intent => { t.intent_admitted = true; true },
                            Effect::Begin | Effect::Read => { t.ready = true; true },
                            Effect::Data(count) => { t.offset += count; true },
                            Effect::Finish => { self.clear_transaction(); true },
                            Effect::Receipt => false,
                        };
                        if progressed { self.progress(now); }
                    }
                }
            }
        }
        attempts
    }
    /// Shell calls this after an exact held lifecycle terminal attestation and
    /// before its own cleanup ACK. Replays never restore registration or bytes.
    pub fn retire(&mut self, subject: Subject) -> bool {
        let Some(index) = self.index(subject) else { return false; };
        self.slots[index].retired = true; self.slots[index].live = false;
        self.slots[index].erase_pending(); self.slots[index].receipt = None;
        if self.publication.is_some_and(|p| p.owner == subject) { erase(&mut self.committed); self.publication = None; }
        if let Some(t) = self.transaction.as_ref() {
            if t.owner == index { self.clear_transaction(); }
            else if t.mode == Mode::Read && t.producer == index {
                let (consumer, ticket) = (t.owner, t.ticket);
                self.fail(consumer, ticket, Phase::Finish, Status::Invalidated);
            }
        }
        true
    }
}
impl Drop for Broker {
    fn drop(&mut self) {
        erase(&mut self.stage); erase(&mut self.committed); erase(&mut self.read_snapshot);
        for slot in &mut self.slots { slot.erase_pending(); slot.receipt = None; }
    }
}
