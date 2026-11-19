// SPDX-License-Identifier: GPL-3.0-only
//! Narrow autofill relay on Shell's held per-program Domain routes. No ambient
//! addressing or app-asserted identity. At most one live consent/transfer; finite
//! outbox stores one secret chunk at a time and wipes it on admission/revocation.
use sex_pdx::autofill::*;
const ROWS: usize = 16;
const OUTBOX: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub subject: Subject,
    pub route: u64,
    pub name: [u8; 16],
}
impl Endpoint {
    pub fn provider(self) -> bool {
        self.name == *b"chastity\0\0\0\0\0\0\0\0"
    }
}
#[derive(Clone, Copy)]
struct Provider {
    endpoint: Endpoint,
    epoch: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Origin,
    Consent,
    Header,
    Data,
    Finishing,
}
struct Transaction {
    client: Endpoint,
    provider: Provider,
    token: u32,
    sequence: u32,
    origin_len: usize,
    origin_at: usize,
    data_len: usize,
    data_at: usize,
    phase: Phase,
    waiting: bool,
    deadline: u64,
    last: u64,
}
#[derive(Clone, Copy)]
enum Effect {
    None,
    Origin(usize),
    Header,
    Data(usize),
    Done,
}
struct Pending {
    endpoint: Endpoint,
    packet: Packet,
    effect: Effect,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Accepted,
    Full,
    Rejected,
}
pub struct Broker {
    endpoints: [Option<Endpoint>; ROWS],
    provider: Option<Provider>,
    transaction: Option<Transaction>,
    queue: [Option<Pending>; OUTBOX],
    next: u32,
    head: usize,
    queued: usize,
    revoked: Option<(Subject, u64)>,
    cancelled: [Option<(Subject, u32, u64)>; ROWS],
}
impl Broker {
    pub const fn new() -> Self {
        Self {
            endpoints: [None; ROWS],
            provider: None,
            transaction: None,
            queue: [const { None }; OUTBOX],
            next: 0,
            head: 0,
            queued: 0,
            revoked: None,
            cancelled: [None; ROWS],
        }
    }
    /// Called only after Shell attests an active loaded run through its held catalog.
    pub fn bind(&mut self, e: Endpoint) {
        if !e.subject.valid() || e.route == 0 {
            return;
        }
        if let Some(i) = self
            .endpoints
            .iter()
            .position(|v| v.is_some_and(|old| old.subject.pd == e.subject.pd))
        {
            if let Some(old) = self.endpoints[i] {
                self.retire(old.subject);
            }
            self.endpoints[i] = Some(e);
        } else if let Some(i) = self.endpoints.iter().position(Option::is_none) {
            self.endpoints[i] = Some(e);
        }
    }
    pub fn label(&mut self, s: Subject, name: [u8; 16]) {
        if let Some(e) = self.endpoints.iter_mut().flatten().find(|e| e.subject == s) {
            e.name = name;
        }
    }
    fn endpoint(&self, s: Subject) -> Option<Endpoint> {
        self.endpoints
            .iter()
            .flatten()
            .find(|e| e.subject == s)
            .copied()
    }
    fn discard_holes(&mut self) {
        while self.queued > 0 && self.queue[self.head].is_none() {
            self.head = (self.head + 1) % OUTBOX;
            self.queued -= 1;
        }
    }
    fn put(&mut self, e: Endpoint, packet: Packet, effect: Effect) -> bool {
        self.discard_holes();
        if self.queued == OUTBOX {
            return false;
        }
        let at = (self.head + self.queued) % OUTBOX;
        self.queue[at] = Some(Pending {
            endpoint: e,
            packet,
            effect,
        });
        self.queued += 1;
        true
    }
    fn metadata(&mut self, e: Endpoint, p: Packet) {
        self.discard_holes();
        if self.transaction.is_some() && self.queued >= OUTBOX - 3 {
            return;
        }
        self.put(e, p, Effect::None);
    }
    fn cancel_receipt(&mut self, e: Endpoint, id: u32, epoch: u64) {
        if let Some(slot) = self
            .cancelled
            .iter_mut()
            .find(|v| v.is_some_and(|v| v.0 == e.subject) || v.is_none())
        {
            *slot = Some((e.subject, id, epoch));
        }
        self.metadata(
            e,
            Packet::new(CANCELLED, id as u64, epoch, e.subject.word()),
        );
    }
    /// Authenticated right-narrowing revocations need no fresh LIVE query. A
    /// stale actor can remove only its own exact run/epoch, never release data.
    /// Some(owners) means wipe their current native intake; None is idempotent
    /// acknowledgement only and must not disturb a newer epoch's pending work.
    pub fn revocation_scope(&self, s: Subject, p: &Packet) -> Option<Option<[Subject; 2]>> {
        let [a, b, c] = p.args;
        if p.opcode == UNREGISTER && b == 0 && c == 0 {
            if self
                .provider
                .is_some_and(|v| v.endpoint.subject == s && v.epoch == a)
            {
                return Some(Some(self.transaction.as_ref().map_or([s, s], |t| {
                    [t.client.subject, t.provider.endpoint.subject]
                })));
            }
            if self.revoked == Some((s, a)) {
                return Some(None);
            }
        }
        if let Some(t) = &self.transaction {
            let owner = s == t.client.subject || s == t.provider.endpoint.subject;
            let token = a == t.token as u64 && b == t.provider.epoch && c == 0;
            let seq = s == t.client.subject && a == 0 && b == 0 && c == t.sequence as u64;
            if (p.opcode == CANCEL && owner && (token || seq))
                || (p.opcode == DENY && s == t.provider.endpoint.subject && token)
            {
                return Some(Some([t.client.subject, t.provider.endpoint.subject]));
            }
        }
        if p.opcode == CANCEL {
            let (id, epoch) = if a == 0 && b == 0 && c > 0 && c <= u32::MAX as u64 {
                (c as u32, 0)
            } else if a > 0 && a <= u32::MAX as u64 && c == 0 {
                (a as u32, b)
            } else {
                return None;
            };
            if self
                .cancelled
                .iter()
                .flatten()
                .any(|v| *v == (s, id, epoch))
            {
                return Some(None);
            }
        }
        None
    }
    fn status(&mut self, e: Endpoint, id: u32, epoch: u64, status: Status) {
        // Unrelated status traffic cannot consume the active transaction's
        // forward+ACK slots. Secret flow always has reserved capacity.
        self.discard_holes();
        if self.transaction.is_some() && self.queued >= OUTBOX - 3 {
            return;
        }
        self.put(
            e,
            Packet::new(STATUS, id as u64, epoch, status as u64),
            Effect::None,
        );
    }
    fn stop(&mut self, status: Status) {
        let transaction = self.transaction.take();
        for slot in &mut self.queue {
            *slot = None;
        }
        self.head = 0;
        self.queued = 0;
        if let Some(t) = transaction {
            self.status(t.client, t.token, t.provider.epoch, status);
            self.put(
                t.provider.endpoint,
                Packet::new(CANCEL, t.token as u64, t.provider.epoch, status as u64),
                Effect::None,
            );
        }
    }
    pub fn retire(&mut self, s: Subject) {
        for item in &mut self.cancelled {
            if item.is_some_and(|v| v.0 == s) {
                *item = None;
            }
        }
        if self
            .transaction
            .as_ref()
            .is_some_and(|t| t.client.subject == s || t.provider.endpoint.subject == s)
        {
            self.stop(Status::Stale);
        }
        if self.provider.is_some_and(|p| p.endpoint.subject == s) {
            self.provider = None;
        }
        for e in &mut self.endpoints {
            if e.is_some_and(|e| e.subject == s) {
                *e = None;
            }
        }
        for slot in &mut self.queue {
            if slot.as_ref().is_some_and(|v| v.endpoint.subject == s) {
                *slot = None;
            }
        }
    }
    pub fn pending(&self) -> bool {
        self.queue.iter().any(Option::is_some)
    }
    pub fn wake(&self, now: u64) -> Option<u64> {
        self.transaction
            .as_ref()
            .map(|t| t.deadline.saturating_sub(now).min(1000).max(1))
    }
    fn expired(&mut self, now: u64) {
        if self
            .transaction
            .as_ref()
            .is_some_and(|t| now < t.last || now >= t.deadline)
        {
            self.stop(Status::Stale);
        }
        if let Some(t) = &mut self.transaction {
            t.last = now;
        }
    }
    /// Caller identity/liveness are authenticated externally before this function.
    /// Malformed traffic cannot change another run's active transaction.
    pub fn receive(&mut self, s: Subject, p: &Packet, focused: bool, now: u64) -> Status {
        self.expired(now);
        self.discard_holes();
        let Some(e) = self.endpoint(s) else {
            return Status::Denied;
        };
        let a = p.args[0];
        let b = p.args[1];
        let c = if p.opcode == DATA || p.opcode == ORIGIN {
            0
        } else {
            p.args[2]
        };
        if p.opcode == REGISTER {
            if !e.provider() || a == 0 || b != 0 || c != 0 || self.revoked == Some((s, a)) {
                self.status(e, 0, a, Status::Denied);
                return Status::Denied;
            }
            if self
                .provider
                .is_some_and(|v| v.endpoint != e || v.epoch != a)
            {
                self.status(e, 0, a, Status::Busy);
                return Status::Busy;
            }
            if self.transaction.is_some() {
                return Status::Busy;
            }
            self.provider = Some(Provider {
                endpoint: e,
                epoch: a,
            });
            self.put(
                e,
                Packet::new(REGISTERED, a, e.subject.word(), 0),
                Effect::None,
            );
            return Status::Ok;
        }
        if p.opcode == UNREGISTER {
            if self.revocation_scope(s, p).is_none() {
                return Status::Denied;
            }
            if self
                .provider
                .is_some_and(|v| v.endpoint.subject == s && v.epoch == a)
            {
                self.stop(Status::Locked);
                self.provider = None;
                self.revoked = Some((s, a));
            }
            self.metadata(e, Packet::new(REVOKED, a, s.word(), 0));
            return Status::Ok;
        }
        if p.opcode == CANCEL && self.revocation_scope(s, p) == Some(None) {
            let (id, epoch) = if a == 0 { (c as u32, 0) } else { (a as u32, b) };
            self.cancel_receipt(e, id, epoch);
            return Status::Ok;
        }
        if p.opcode == BEGIN {
            if !focused {
                self.status(e, a as u32, 0, Status::Denied);
                return Status::Denied;
            }
            if a == 0 || a > u32::MAX as u64 || b == 0 || b > MAX_ORIGIN as u64 || c != 0 {
                return Status::Invalid;
            }
            if self.transaction.is_some() || self.pending() {
                self.status(e, a as u32, 0, Status::Busy);
                return Status::Busy;
            }
            let Some(provider) = self.provider else {
                self.status(e, a as u32, 0, Status::Unavailable);
                return Status::Unavailable;
            };
            if provider.endpoint.subject == s {
                return Status::Denied;
            }
            let Some(token) = self.next.checked_add(1) else {
                return Status::Denied;
            };
            self.next = token;
            self.transaction = Some(Transaction {
                client: e,
                provider,
                token,
                sequence: a as u32,
                origin_len: b as usize,
                origin_at: 0,
                data_len: 0,
                data_at: 0,
                phase: Phase::Origin,
                waiting: false,
                deadline: now.saturating_add(CONSENT_TICKS),
                last: now,
            });
            self.put(
                e,
                Packet::new(READY, a | (token as u64) << 32, provider.epoch, s.word()),
                Effect::None,
            );
            self.put(
                provider.endpoint,
                Packet::new(OFFER, token as u64 | b << 32, s.word(), provider.epoch),
                Effect::None,
            );
            for part in 0..2 {
                self.put(
                    provider.endpoint,
                    Packet::new(
                        CONSUMER,
                        token as u64 | (part as u64) << 32,
                        provider.epoch,
                        u64::from_le_bytes(e.name[part * 8..part * 8 + 8].try_into().unwrap()),
                    ),
                    Effect::None,
                );
            }
            return Status::Ok;
        }
        let Some(t) = self.transaction.as_ref() else {
            return Status::Stale;
        };
        let client = t.client;
        let provider = t.provider;
        let token = t.token;
        if s != client.subject && s != provider.endpoint.subject {
            return Status::Denied;
        }
        if p.opcode == CANCEL && s == client.subject && a == 0 && b == 0 && c == t.sequence as u64 {
            self.stop(Status::Declined);
            self.cancel_receipt(e, c as u32, 0);
            return Status::Ok;
        }
        if p.opcode == CANCEL && a == token as u64 && b == provider.epoch && c == 0 {
            self.stop(Status::Declined);
            self.cancel_receipt(e, token, provider.epoch);
            return Status::Ok;
        }
        if p.opcode == DENY
            && s == provider.endpoint.subject
            && a == token as u64
            && b == provider.epoch
            && c == 0
        {
            self.stop(Status::Declined);
            return Status::Ok;
        }
        if p.opcode == ORIGIN && s == client.subject {
            let Some((id, epoch, offset, count)) = chunk_meta(p) else {
                return Status::Invalid;
            };
            if id != token
                || epoch != provider.epoch
                || t.phase != Phase::Origin
                || t.waiting
                || offset != t.origin_at
                || offset + count > t.origin_len
            {
                return Status::Denied;
            }
            if !self.put(
                provider.endpoint,
                Packet::new(ORIGIN, p.args[0], p.args[1], p.args[2]),
                Effect::Origin(offset + count),
            ) {
                return Status::Busy;
            }
            self.transaction.as_mut().unwrap().waiting = true;
            return Status::Ok;
        }
        if p.opcode == ALLOW && s == provider.endpoint.subject {
            if self.queued >= OUTBOX - 1 {
                return Status::Busy;
            }
            if !focused {
                self.stop(Status::Denied);
                return Status::Denied;
            }
            let Some((user, password)) = lengths(c) else {
                return Status::Invalid;
            };
            if a != token as u64 || b != provider.epoch || t.phase != Phase::Consent || t.waiting {
                return Status::Denied;
            }
            let t = self.transaction.as_mut().unwrap();
            t.data_len = user + password;
            t.phase = Phase::Header;
            t.waiting = true;
            t.deadline = now.saturating_add(TRANSFER_TICKS);
            self.put(
                client,
                Packet::new(APPROVED, token as u64, provider.epoch, c),
                Effect::Header,
            );
            return Status::Ok;
        }
        if p.opcode == DATA && s == provider.endpoint.subject {
            let Some((id, epoch, offset, count)) = chunk_meta(p) else {
                return Status::Invalid;
            };
            if id != token
                || epoch != provider.epoch
                || t.phase != Phase::Data
                || t.waiting
                || offset != t.data_at
                || offset + count > t.data_len
            {
                return Status::Denied;
            }
            if !self.put(
                client,
                Packet::new(DATA, p.args[0], p.args[1], p.args[2]),
                Effect::Data(offset + count),
            ) {
                return Status::Busy;
            }
            self.transaction.as_mut().unwrap().waiting = true;
            return Status::Ok;
        }
        if p.opcode == FINISH
            && s == provider.endpoint.subject
            && a == token as u64
            && b == provider.epoch
            && c == 0
            && t.phase == Phase::Data
            && !t.waiting
            && t.data_at == t.data_len
        {
            self.transaction.as_mut().unwrap().phase = Phase::Finishing;
            self.put(
                client,
                Packet::new(FINISH, token as u64, provider.epoch, 0),
                Effect::Done,
            );
            return Status::Ok;
        }
        Status::Denied
    }
    /// Refresh BOTH endpoints before any admission, including pending secret DATA.
    /// None liveness is transient contention; false permanently revokes that run.
    pub fn poll(
        &mut self,
        now: u64,
        mut live: impl FnMut(Subject) -> Option<bool>,
        mut admit: impl FnMut(Endpoint, &Packet) -> Admission,
    ) {
        self.expired(now);
        if let Some(t) = self.transaction.as_ref() {
            let a = t.client.subject;
            let b = t.provider.endpoint.subject;
            match (live(a), live(b)) {
                (Some(false), _) => {
                    self.retire(a);
                }
                (_, Some(false)) => {
                    self.retire(b);
                }
                (Some(true), Some(true)) => {}
                _ => return,
            }
        }
        if let Some(p) = self.provider {
            if live(p.endpoint.subject) == Some(false) {
                self.retire(p.endpoint.subject);
            }
        }
        self.discard_holes();
        if self.queued == 0 {
            return;
        }
        let index = self.head;
        let queued = self.queue[index].as_ref().unwrap();
        match live(queued.endpoint.subject) {
            Some(true) => {}
            Some(false) => {
                let s = queued.endpoint.subject;
                self.retire(s);
                return;
            }
            None => return,
        }
        match admit(queued.endpoint, &queued.packet) {
            Admission::Full => return,
            Admission::Rejected => {
                let s = queued.endpoint.subject;
                self.retire(s);
                return;
            }
            Admission::Accepted => {}
        }
        let effect = self.queue[index].as_ref().unwrap().effect;
        // Wipe the actual static queue payload before clearing its enum tag.
        // Moving/taking a secret Packet would leave stale bytes in its old slot.
        self.queue[index].as_mut().unwrap().packet.erase();
        self.queue[index] = None;
        self.head = (self.head + 1) % OUTBOX;
        self.queued -= 1;
        let Some(t) = self.transaction.as_mut() else {
            return;
        };
        let token = t.token;
        let epoch = t.provider.epoch;
        let target = match effect {
            Effect::None => return,
            Effect::Origin(next) => {
                t.origin_at = next;
                t.waiting = false;
                if next == t.origin_len {
                    t.phase = Phase::Consent;
                }
                t.client
            }
            Effect::Header => {
                t.phase = Phase::Data;
                t.waiting = false;
                t.provider.endpoint
            }
            Effect::Data(next) => {
                t.data_at = next;
                t.waiting = false;
                t.provider.endpoint
            }
            Effect::Done => {
                let e = t.provider.endpoint;
                self.transaction = None;
                self.put(
                    e,
                    Packet::new(ACK, token as u64, epoch, u64::MAX),
                    Effect::None,
                );
                return;
            }
        };
        let offset = match effect {
            Effect::Origin(n) | Effect::Data(n) => n,
            _ => 0,
        };
        self.put(
            target,
            Packet::new(ACK, token as u64, epoch, offset as u64),
            Effect::None,
        );
    }
}
