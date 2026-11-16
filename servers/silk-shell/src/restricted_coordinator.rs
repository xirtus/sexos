//! Finite offline-helper coordinator. Native hooks supply held lifecycle/peer
//! facts and durable reservation; no target, digest or task is trusted from a
//! parent payload. All private control words use the disjoint tagged transport.
use sex_pdx::{restricted_helper as w, restricted_job as job};
use w::{Packet, Phase, Subject};
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Files,
    Display,
    Net,
    Bell,
}
impl Service {
    fn at(i: usize) -> Self {
        [Self::Files, Self::Display, Self::Net, Self::Bell][i]
    }
    pub fn pd(self) -> u32 {
        match self {
            Self::Files => 11,
            Self::Display => 1,
            Self::Net => 13,
            Self::Bell => 10,
        }
    }
}
#[derive(Clone, Copy)]
pub struct Reservation {
    pub previous: Subject,
    pub slot: usize,
    pub fresh: bool,
    pub generation: u64,
}
pub enum Activation {
    Pending,
    Active,
    Failed,
}
pub trait Platform {
    fn now(&mut self) -> u64;
    fn self_epoch(&mut self) -> Option<u64>;
    fn peer_epoch(&mut self, service: Service) -> Option<u64>;
    fn live(&mut self, subject: Subject) -> bool;
    fn reserve(&mut self) -> Option<Reservation>;
    fn bound(&mut self, reservation: Reservation, child: Subject) -> bool;
    fn send(&mut self, service: Service, opcode: u64, words: [u64; 3]) -> bool;
    fn copy(&mut self, reservation: Reservation, length: u32) -> Option<Subject>;
    fn activate(&mut self, reservation: Reservation, child: Subject) -> Activation;
    fn started(&mut self, parent: Subject, cookie: u32, child: Subject) -> bool;
    fn refused(&mut self, parent: Subject, cookie: u32);
    fn ready(&mut self, child: Subject, value: u64) -> bool;
    fn kill(&mut self, child: Subject);
    fn retired(&mut self, child: Subject) -> bool;
    /// Metadata-only witness after every complete accepted reply credit.
    fn policy_acked(&mut self, _service: Service, _phase: u8, _child: Subject, _generation: u64) {}
}
#[derive(Clone, Copy)]
enum Stage {
    Claim,
    Describe(u8, u8),
    Policy {
        peer: u8,
        phase: u8,
        selector: u8,
        part: u8,
    },
    Load,
    Serial(u8),
    Copy,
    Copied,
    Activate,
}
/// Admission to activation. The policy walk is a few dozen Shell/Files round
/// trips; with a busy desktop (a full-frame app repainting every ~100 ms) it
/// took ~4.9 s in the Moan offline UI gate, so 5 s refused a healthy helper.
const BOOTSTRAP_TICKS: u64 = 15000;

struct Transaction {
    parent: Subject,
    ticket: u32,
    cookie: u32,
    shell: u64,
    epochs: [u64; 4],
    until: u64,
    job_until: u64,
    raw: [u8; 176],
    info: Option<job::Info>,
    reservation: Option<Reservation>,
    child: Option<Subject>,
    challenge: [[u64; 2]; 4],
    word: u64,
    length: u32,
    serial: u64,
    stage: Stage,
    sent: bool,
    sent_at: u64,
}
impl Drop for Transaction {
    fn drop(&mut self) {
        for b in &mut self.raw {
            unsafe { core::ptr::write_volatile(b, 0) }
        }
        for pair in &mut self.challenge {
            for word in pair {
                unsafe { core::ptr::write_volatile(word, 0) }
            }
        }
        if let Some(info) = self.info.as_mut() {
            for array in [
                &mut info.plugin_hash,
                &mut info.input_hash,
                &mut info.output_hash,
            ] {
                for b in array {
                    unsafe { core::ptr::write_volatile(b, 0) }
                }
            }
            for b in &mut info.nonce {
                unsafe { core::ptr::write_volatile(b, 0) }
            }
        }
        unsafe { core::ptr::write_volatile(&mut self.word, 0) }
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }
}
struct Live {
    parent: Subject,
    child: Subject,
    ticket: u32,
    cookie: u32,
    generation: u64,
    nonce: [u64; 2],
    shell: u64,
    epochs: [u64; 4],
    until: u64,
    ready_until: u64,
    ready: bool,
    ready_pending: bool,
    reported: bool,
    cancelled: bool,
}
impl Drop for Live {
    fn drop(&mut self) {
        for n in &mut self.nonce {
            unsafe { core::ptr::write_volatile(n, 0) }
        }
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }
}
pub struct Coordinator {
    transaction: Option<Transaction>,
    live: [Option<Live>; 2],
}
fn epochs(p: &mut impl Platform) -> Option<[u64; 4]> {
    let mut out = [0; 4];
    for (i, e) in out.iter_mut().enumerate() {
        *e = p.peer_epoch(Service::at(i)).filter(|e| *e != 0)?;
    }
    Some(out)
}
fn phase(n: u8) -> Phase {
    match n {
        1 => Phase::Quarantine,
        2 => Phase::EchoChallenge,
        3 => Phase::Coordinator,
        4 => Phase::Owner,
        5 => Phase::Nonce,
        6 => Phase::PluginHashLo,
        7 => Phase::PluginHashHi,
        8 => Phase::InputHashLo,
        9 => Phase::InputHashHi,
        10 => Phase::Bind,
        11 => Phase::Seal,
        _ => unreachable!(),
    }
}
fn pair(bytes: &[u8]) -> [u64; 2] {
    core::array::from_fn(|i| u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap()))
}
impl Transaction {
    fn packet(&self, peer: u8, n: u8) -> Packet {
        let r = self.reservation.unwrap();
        let i = self.info.as_ref().unwrap();
        let (a, b) = match n {
            1 => (r.previous.word(), self.epochs[peer as usize]),
            2 => (
                self.challenge[peer as usize][0],
                self.challenge[peer as usize][1],
            ),
            3 => (self.shell, self.epochs[peer as usize]),
            4 => (self.parent.word(), self.cookie as u64),
            5 => {
                let p = pair(&i.nonce);
                (p[0], p[1])
            }
            6 => {
                let p = pair(&i.plugin_hash[..16]);
                (p[0], p[1])
            }
            7 => {
                let p = pair(&i.plugin_hash[16..]);
                (p[0], p[1])
            }
            8 => {
                let p = pair(&i.input_hash[..16]);
                (p[0], p[1])
            }
            9 => {
                let p = pair(&i.input_hash[16..]);
                (p[0], p[1])
            }
            10 => (self.child.unwrap().word(), self.ticket as u64),
            11 => (self.child.unwrap().word(), self.epochs[peer as usize]),
            _ => unreachable!(),
        };
        Packet {
            generation: r.generation,
            phase: phase(n),
            a,
            b,
        }
    }
    fn command(&self) -> Option<(Service, u64, [u64; 3])> {
        Some(match self.stage {
            Stage::Claim => (
                Service::Files,
                job::OP_CLAIM,
                [self.ticket as u64, self.parent.word(), self.cookie as u64],
            ),
            Stage::Describe(i, part) => (
                Service::Files,
                job::OP_DESCRIBE_TAGGED,
                [
                    self.ticket as u64,
                    i as u64 | (part as u64) << 8,
                    self.epochs[0],
                ],
            ),
            Stage::Policy {
                peer,
                phase,
                selector,
                part,
            } => {
                let packet = self.packet(peer, phase);
                let words = packet.wire()?;
                if selector == 0 && part == 0 {
                    (Service::at(peer as usize), w::OP_POLICY_TAGGED, words)
                } else {
                    (
                        Service::at(peer as usize),
                        w::OP_FETCH_TAGGED,
                        [
                            words[0],
                            selector as u64 | (part as u64) << 8,
                            self.epochs[peer as usize],
                        ],
                    )
                }
            }
            Stage::Load => (
                Service::Files,
                job::OP_LOAD,
                [
                    self.ticket as u64,
                    self.reservation?.generation,
                    self.reservation?.previous.word(),
                ],
            ),
            Stage::Serial(part) => (
                Service::Files,
                job::OP_DESCRIBE_TAGGED,
                [self.ticket as u64, 22 | (part as u64) << 8, self.epochs[0]],
            ),
            Stage::Copied => (
                Service::Files,
                job::OP_COPIED,
                [
                    self.ticket as u64,
                    self.reservation?.generation,
                    self.serial,
                ],
            ),
            Stage::Copy | Stage::Activate => return None,
        })
    }
    fn set(&mut self, stage: Stage) {
        self.stage = stage;
        self.sent = false;
        self.sent_at = 0;
    }
}
impl Coordinator {
    pub const fn new() -> Self {
        Self {
            transaction: None,
            live: [const { None }; 2],
        }
    }
    pub fn busy(&self) -> bool {
        self.transaction.is_some()
    }
    pub fn begin(
        &mut self,
        parent: Subject,
        ticket: u32,
        cookie: u32,
        files_epoch: u64,
        p: &mut impl Platform,
    ) -> bool {
        if ticket == 0 || ticket > 0xff_ffff || cookie == 0 || files_epoch == 0 || !p.live(parent) {
            return false;
        }
        if self.transaction.as_ref().is_some_and(|t| {
            t.parent == parent
                && t.ticket == ticket
                && t.cookie == cookie
                && t.epochs[0] == files_epoch
        }) {
            return true;
        }
        if let Some(l) = self.live.iter_mut().flatten().find(|l| {
            l.parent == parent
                && l.ticket == ticket
                && l.cookie == cookie
                && l.epochs[0] == files_epoch
        }) {
            if !l.cancelled {
                l.reported = false;
            }
            return true;
        }
        if self
            .live
            .iter()
            .flatten()
            .any(|l| l.parent == parent && (l.ticket == ticket || l.cookie == cookie))
        {
            return false;
        }
        if self.busy()
            || self.live.iter().all(Option::is_some)
            || ticket == 0
            || ticket > 0xff_ffff
            || cookie == 0
            || !p.live(parent)
        {
            return false;
        }
        let Some(shell) = p.self_epoch().filter(|e| *e != 0) else {
            return false;
        };
        let Some(epochs) = epochs(p).filter(|e| e[0] == files_epoch) else {
            return false;
        };
        let now = p.now();
        let Some(until) = now.checked_add(BOOTSTRAP_TICKS) else {
            return false;
        };
        let Some(job_until) = now.checked_add(120000) else {
            return false;
        };
        self.transaction = Some(Transaction {
            parent,
            ticket,
            cookie,
            shell,
            epochs,
            until,
            job_until,
            raw: [0; 176],
            info: None,
            reservation: None,
            child: None,
            challenge: [[0; 2]; 4],
            word: 0,
            length: 0,
            serial: 0,
            stage: Stage::Claim,
            sent: false,
            sent_at: 0,
        });
        true
    }
    fn fail(&mut self, p: &mut impl Platform) {
        if let Some(t) = self.transaction.take() {
            if let Some(child) = t.child {
                // Once a private copy exists, neither an activation failure nor
                // a successful force_end request proves actual retirement.
                // Keep cleanup and the exact parent/child relation until the
                // lifecycle reports retired; only then publish Started/Done.
                p.kill(child);
                let slot = self
                    .live
                    .iter_mut()
                    .find(|s| s.is_none())
                    .expect("reserved cleanup slot");
                *slot = Some(Live {
                    parent: t.parent,
                    child,
                    ticket: t.ticket,
                    cookie: t.cookie,
                    generation: t.reservation.unwrap().generation,
                    nonce: [0; 2],
                    shell: t.shell,
                    epochs: t.epochs,
                    until: t.job_until,
                    ready_until: 0,
                    ready: false,
                    ready_pending: false,
                    reported: false,
                    cancelled: true,
                });
            } else {
                p.refused(t.parent, t.cookie);
            }
        }
    }
    pub fn cancel(
        &mut self,
        parent: Subject,
        ticket: u32,
        cookie: u32,
        files_epoch: u64,
        p: &mut impl Platform,
    ) -> bool {
        if self.transaction.as_ref().is_some_and(|t| {
            t.parent == parent
                && t.ticket == ticket
                && t.cookie == cookie
                && t.epochs[0] == files_epoch
        }) {
            self.fail(p);
            return true;
        }
        if let Some(l) = self.live.iter_mut().flatten().find(|l| {
            l.parent == parent
                && l.ticket == ticket
                && l.cookie == cookie
                && l.epochs[0] == files_epoch
        }) {
            l.cancelled = true;
            p.kill(l.child);
            return true;
        }
        false
    }
    pub fn poll(&mut self, p: &mut impl Platform) {
        for slot in &mut self.live {
            let Some(l) = slot.as_mut() else { continue };
            if p.retired(l.child) {
                if !p.live(l.parent) {
                    *slot = None;
                    continue;
                }
                if !l.reported {
                    l.reported = p.started(l.parent, l.cookie, l.child);
                }
                if l.reported {
                    *slot = None;
                }
                continue;
            }
            let now = p.now();
            if now >= l.until
                || (!l.ready && now >= l.ready_until)
                || !p.live(l.parent)
                || p.self_epoch() != Some(l.shell)
                || epochs(p) != Some(l.epochs)
            {
                l.cancelled = true;
            }
            if l.cancelled {
                p.kill(l.child);
                continue;
            }
            if !l.reported {
                l.reported = p.started(l.parent, l.cookie, l.child);
            }
            if l.ready_pending
                && p.ready(
                    l.child,
                    0xd4 << 56 | (l.ticket as u64) << 32 | (l.generation & 0xffff_ffff),
                )
            {
                l.ready_pending = false;
                l.ready = true;
            }
        }
        let Some(t) = self.transaction.as_mut() else {
            return;
        };
        if p.now() >= t.until
            || !p.live(t.parent)
            || p.self_epoch() != Some(t.shell)
            || epochs(p) != Some(t.epochs)
        {
            self.fail(p);
            return;
        }
        match t.stage {
            Stage::Copy => {
                let r = t.reservation.unwrap();
                let Some(child) = p.copy(r, t.length) else {
                    self.fail(p);
                    return;
                };
                if child.pd != r.previous.pd || child.task == 0 {
                    self.fail(p);
                    return;
                }
                // Record only the validated owned-row identity before subsequent
                // checks, so any failure attempts exact dormant cleanup.
                t.child = Some(child);
                if !p.bound(r, child)
                    || !p.live(t.parent)
                    || p.self_epoch() != Some(t.shell)
                    || epochs(p) != Some(t.epochs)
                {
                    self.fail(p);
                    return;
                }
                t.set(Stage::Copied);
            }
            Stage::Activate => {
                let r = t.reservation.unwrap();
                let child = t.child.unwrap();
                match p.activate(r, child) {
                    Activation::Pending => return,
                    Activation::Failed => {
                        self.fail(p);
                        return;
                    }
                    Activation::Active => {
                        if p.self_epoch() != Some(t.shell)
                            || epochs(p) != Some(t.epochs)
                            || !p.live(t.parent)
                        {
                            p.kill(child);
                            self.fail(p);
                            return;
                        }
                        let i = t.info.as_ref().unwrap();
                        let l = Live {
                            parent: t.parent,
                            child,
                            ticket: t.ticket,
                            cookie: t.cookie,
                            generation: r.generation,
                            nonce: pair(&i.nonce),
                            shell: t.shell,
                            epochs: t.epochs,
                            until: t.job_until,
                            ready_until: p.now().saturating_add(5000),
                            ready: false,
                            ready_pending: false,
                            reported: false,
                            cancelled: false,
                        };
                        let slot = self.live.iter_mut().find(|s| s.is_none()).unwrap();
                        *slot = Some(l);
                        self.transaction = None;
                        return;
                    }
                }
            }
            _ => {}
        }
        let Some(t) = self.transaction.as_mut() else {
            return;
        };
        if !t.sent || p.now().saturating_sub(t.sent_at) >= 50 {
            if let Some((service, opcode, words)) = t.command() {
                if p.send(service, opcode, words) {
                    t.sent = true;
                    t.sent_at = p.now();
                }
            }
        }
    }
    /// A Refused receipt can only mean no private child was ever copied.
    pub fn copied_cookie(&self, parent: Subject, cookie: u32) -> bool {
        self.transaction
            .as_ref()
            .is_some_and(|t| t.parent == parent && t.cookie == cookie && t.child.is_some())
            || self
                .live
                .iter()
                .flatten()
                .any(|l| l.parent == parent && l.cookie == cookie)
    }
    pub fn pending(&self) -> bool {
        self.busy() || self.live.iter().any(Option::is_some)
    }
    pub fn retire(&mut self, actor: Subject, p: &mut impl Platform) {
        if self
            .transaction
            .as_ref()
            .is_some_and(|t| t.parent == actor || t.child == Some(actor))
        {
            self.fail(p);
        }
        for l in self.live.iter_mut().flatten() {
            if l.child == actor {
                l.ready_pending = false;
            }
            if l.parent == actor {
                l.cancelled = true;
                p.kill(l.child);
            }
        }
    }
    /// Done must not overtake a retained Started event under parent backpressure.
    pub fn holds_done(&self, child: Subject) -> bool {
        self.live
            .iter()
            .flatten()
            .any(|l| l.child == child && !l.reported)
    }
    pub fn ready(&mut self, actor: Subject, words: [u64; 3], p: &mut impl Platform) -> bool {
        let Some(l) = self.live.iter_mut().flatten().find(|l| l.child == actor) else {
            return false;
        };
        if l.cancelled
            || words != [l.generation, l.nonce[0], l.nonce[1]]
            || !p.live(actor)
            || !p.live(l.parent)
            || p.now() >= l.until
            || p.now() >= l.ready_until
            || p.self_epoch() != Some(l.shell)
            || epochs(p) != Some(l.epochs)
        {
            return false;
        }
        l.ready_pending = true;
        true
    }
    pub fn reply(&mut self, caller: Subject, value: u64, p: &mut impl Platform) -> bool {
        let Some(t) = self.transaction.as_mut() else {
            return false;
        };
        let Some((service, _, _)) = t.command() else {
            return false;
        };
        if caller
            != (Subject {
                pd: service.pd(),
                task: service.pd(),
            })
            || !t.sent
        {
            return false;
        }
        let belongs = match t.stage {
            Stage::Claim => value >> 56 == 0xd1 && ((value >> 32) & 0xff_ffff) == t.ticket as u64,
            Stage::Load => value >> 56 == 0xd2 && ((value >> 32) & 0xff_ffff) == t.ticket as u64,
            Stage::Copied => value >> 56 == 0xd3 && ((value >> 32) & 0xff_ffff) == t.ticket as u64,
            Stage::Describe(..) | Stage::Policy { .. } | Stage::Serial(..) => w::is_chunk(value),
            _ => false,
        };
        if !belongs {
            return false;
        }
        if p.now() >= t.until
            || !p.live(t.parent)
            || p.self_epoch() != Some(t.shell)
            || epochs(p) != Some(t.epochs)
        {
            self.fail(p);
            return true;
        }
        let invalid = match t.stage {
            Stage::Claim => {
                if value != 0xd1 << 56 | (t.ticket as u64) << 32 {
                    true
                } else {
                    t.set(Stage::Describe(0, 0));
                    false
                }
            }
            Stage::Describe(selector, part) => {
                if let Some(v) = w::unchunk(value, t.ticket as u64, 0, selector, part) {
                    if part == 0 {
                        t.word = v as u64;
                        t.set(Stage::Describe(selector, 1));
                    } else {
                        let word = t.word | (v as u64) << 32;
                        t.raw[selector as usize * 8..selector as usize * 8 + 8]
                            .copy_from_slice(&word.to_le_bytes());
                        if selector < 21 {
                            t.set(Stage::Describe(selector + 1, 0));
                        } else if let Some(info) = job::Info::decode(&t.raw).filter(|i| {
                            i.ticket == t.ticket
                                && i.parent == t.parent
                                && i.cookie == t.cookie
                                && i.files_epoch == t.epochs[0]
                                && i.child.is_none()
                                && i.output_hash == [0; 32]
                        }) {
                            t.info = Some(info);
                            if let Some(r) = p.reserve().filter(|r| {
                                r.previous.pd != t.parent.pd
                                    && r.generation != 0
                                    && r.generation <= w::MAX_GENERATION
                            }) {
                                t.reservation = Some(r);
                                t.set(Stage::Policy {
                                    peer: 0,
                                    phase: 1,
                                    selector: 0,
                                    part: 0,
                                });
                            } else {
                                self.fail(p);
                                return true;
                            }
                        } else {
                            self.fail(p);
                            return true;
                        }
                    }
                    false
                } else {
                    false
                }
            }
            Stage::Policy {
                peer,
                phase,
                selector,
                part,
            } => {
                let packet = t.packet(peer, phase);
                if let Some(v) = w::unchunk(value, packet.generation, phase, selector, part) {
                    if part == 0 {
                        t.word = v as u64;
                        t.set(Stage::Policy {
                            peer,
                            phase,
                            selector,
                            part: 1,
                        });
                        false
                    } else {
                        let word = t.word | (v as u64) << 32;
                        let expected = match selector {
                            0 => Some(packet.wire().unwrap()[0]),
                            1 => Some(packet.a),
                            2 => Some(packet.b),
                            _ => None,
                        };
                        if expected.is_some_and(|expected| word != expected) {
                            true
                        } else {
                            if selector >= 3 {
                                t.challenge[peer as usize][selector as usize - 3] = word;
                            }
                            let limit = if phase == 1 { 4 } else { 2 };
                            if selector == limit {
                                let r = t.reservation.unwrap();
                                p.policy_acked(
                                    Service::at(peer as usize),
                                    phase,
                                    t.child.unwrap_or(r.previous),
                                    r.generation,
                                );
                            }
                            if selector < limit {
                                t.set(Stage::Policy {
                                    peer,
                                    phase,
                                    selector: selector + 1,
                                    part: 0,
                                });
                            } else if phase < 9 {
                                t.set(Stage::Policy {
                                    peer,
                                    phase: phase + 1,
                                    selector: 0,
                                    part: 0,
                                });
                            } else if phase == 9 {
                                if t.challenge[peer as usize] == [0; 2] {
                                    self.fail(p);
                                    return true;
                                }
                                if peer < 3 {
                                    t.set(Stage::Policy {
                                        peer: peer + 1,
                                        phase: 1,
                                        selector: 0,
                                        part: 0,
                                    });
                                } else {
                                    t.set(Stage::Load);
                                }
                            } else if phase == 10 {
                                t.set(Stage::Policy {
                                    peer,
                                    phase: 11,
                                    selector: 0,
                                    part: 0,
                                });
                            } else if peer < 3 {
                                t.set(Stage::Policy {
                                    peer: peer + 1,
                                    phase: 10,
                                    selector: 0,
                                    part: 0,
                                });
                            } else {
                                t.set(Stage::Activate);
                            }
                            false
                        }
                    }
                } else {
                    false
                }
            }
            Stage::Load => {
                let length = value as u32;
                if value >> 32 != (0xd2 << 24 | t.ticket as u64)
                    || length == 0
                    || length > 32 * 1024 * 1024
                {
                    true
                } else {
                    t.length = length;
                    t.set(Stage::Serial(0));
                    false
                }
            }
            Stage::Serial(part) => {
                if let Some(v) = w::unchunk(value, t.ticket as u64, 0, 22, part) {
                    if part == 0 {
                        t.word = v as u64;
                        t.set(Stage::Serial(1));
                    } else {
                        t.serial = t.word | (v as u64) << 32;
                        if t.serial == 0 {
                            self.fail(p);
                            return true;
                        }
                        t.set(Stage::Copy);
                    }
                    false
                } else {
                    false
                }
            }
            Stage::Copied => {
                if value != 0xd3 << 56 | (t.ticket as u64) << 32 {
                    true
                } else {
                    t.set(Stage::Policy {
                        peer: 0,
                        phase: 10,
                        selector: 0,
                        part: 0,
                    });
                    false
                }
            }
            Stage::Copy | Stage::Activate => false,
        };
        if invalid {
            self.fail(p);
        }
        true
    }
}
