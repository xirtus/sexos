//! Narrow coordinator-to-provider policy transcript. Transport wrappers must
//! authenticate held routes/epochs and persist deny state before returning ACK.
//! This module grants no file rights and exposes no IPC dispatcher itself.
pub const OP_POLICY: u64 = 0x4d50_5001;
pub const OP_ACK: u64 = 0x4d50_5002;
pub const OP_READY: u64 = 0x4d50_5003;
pub const OP_FETCH: u64 = 0x4d50_5004;
pub const OP_FETCH_TAGGED: u64 = 0x4d50_5005;
pub const OP_POLICY_TAGGED: u64 = 0x4d50_5006;
/// Disjoint from legacy Files identity replies, whose tag is in bits24..31.
/// Full64 words are transported as two32 chunks; context bits correlate only.
pub fn chunk(context: u64, phase: u8, selector: u8, part: u8, value: u32) -> Option<u64> {
    if phase > 11 || selector > 31 || part > 1 {
        return None;
    }
    Some(
        0xd5 << 56
            | (context & 63) << 50
            | (phase as u64) << 46
            | (part as u64) << 45
            | (selector as u64) << 40
            | ((value as u64 >> 24) & 255) << 32
            | 0xd5 << 24
            | (value as u64 & 0xff_ffff),
    )
}
pub fn is_chunk(word: u64) -> bool {
    word >> 56 == 0xd5 && ((word >> 24) & 255) == 0xd5
}
pub fn unchunk(word: u64, context: u64, phase: u8, selector: u8, part: u8) -> Option<u32> {
    if !is_chunk(word)
        || phase > 11
        || selector > 31
        || part > 1
        || ((word >> 50) & 63) != context & 63
        || ((word >> 46) & 15) != phase as u64
        || ((word >> 45) & 1) != part as u64
        || ((word >> 40) & 31) != selector as u64
    {
        return None;
    }
    Some((word & 0xff_ffff) as u32 | (((word >> 32) & 255) as u32) << 24)
}
pub const MAX_GENERATION: u64 = (1u64 << 56) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subject {
    pub pd: u32,
    pub task: u32,
}
impl Subject {
    pub const fn word(self) -> u64 {
        ((self.pd as u64) << 32) | self.task as u64
    }
    pub fn decode(word: u64) -> Option<Self> {
        let subject = Self {
            pd: (word >> 32) as u32,
            task: word as u32,
        };
        (subject.pd != 0 && subject.task != 0).then_some(subject)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Quarantine = 1,
    EchoChallenge = 2,
    Coordinator = 3,
    Owner = 4,
    Nonce = 5,
    PluginHashLo = 6,
    PluginHashHi = 7,
    InputHashLo = 8,
    InputHashHi = 9,
    Bind = 10,
    Seal = 11,
}
impl Phase {
    fn decode(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Quarantine,
            2 => Self::EchoChallenge,
            3 => Self::Coordinator,
            4 => Self::Owner,
            5 => Self::Nonce,
            6 => Self::PluginHashLo,
            7 => Self::PluginHashHi,
            8 => Self::InputHashLo,
            9 => Self::InputHashHi,
            10 => Self::Bind,
            11 => Self::Seal,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub generation: u64,
    pub phase: Phase,
    pub a: u64,
    pub b: u64,
}
impl Packet {
    pub fn wire(self) -> Option<[u64; 3]> {
        if self.generation == 0 || self.generation > MAX_GENERATION {
            return None;
        }
        Some([
            ((self.phase as u64) << 56) | self.generation,
            self.a,
            self.b,
        ])
    }
    pub fn decode(w: [u64; 3]) -> Option<Self> {
        let generation = w[0] & MAX_GENERATION;
        (generation != 0).then_some(Self {
            generation,
            phase: Phase::decode((w[0] >> 56) as u8)?,
            a: w[1],
            b: w[2],
        })
    }
}
/// Facts must come from current held peer/catalog/lifecycle queries, never from
/// packet arguments. Initial quarantine accepts only Shell's reserved inactive row (never activated or fully retired).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Authority {
    pub caller: Subject,
    pub provider_epoch: u64,
    pub reserved: Subject,
    pub inactive: bool,
    /// Fresh provider-private CPU entropy for initial quarantine only.
    pub challenge: [u64; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub generation: u64,
    /// Trusted coordinator declaration, not a remotely observed peer epoch.
    pub coordinator_epoch: u64,
    pub provider_epoch: u64,
    pub previous: Subject,
    pub child: Subject,
    pub parent: Subject,
    pub cookie: u32,
    pub nonce: [u64; 2],
    pub plugin_hash: [u64; 4],
    pub input_hash: [u64; 4],
    pub ticket: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accepted {
    Quarantined,
    Staged,
    Sealed,
    Duplicate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Authority,
    Generation,
    Order,
    ChangedRetry,
    Invalid,
    Poisoned,
}

pub struct Transcript {
    binding: Option<Binding>,
    last: Option<Packet>,
    next: u8,
    poisoned: bool,
    challenge: [u64; 2],
}
impl Transcript {
    pub const fn new() -> Self {
        Self {
            binding: None,
            last: None,
            next: 1,
            poisoned: false,
            challenge: [0; 2],
        }
    }
    pub fn binding(&self) -> Option<Binding> {
        self.binding
    }
    pub fn sealed(&self) -> bool {
        self.next == 12 && !self.poisoned
    }
    pub fn poisoned(&self) -> bool {
        self.poisoned
    }
    /// ACKs must echo the accepted packet only AFTER a successful durable fence
    /// update and required scrub. A positive return here alone is not authority.
    pub fn accept(
        &mut self,
        p: Packet,
        authority: Authority,
        child_catalog: Option<Subject>,
    ) -> Result<Accepted, Refused> {
        if authority.caller != (Subject { pd: 3, task: 3 }) || authority.provider_epoch == 0 {
            return Err(Refused::Authority);
        }
        if p.generation == 0 || p.generation > MAX_GENERATION {
            return Err(Refused::Generation);
        }
        if let Some(binding) = self.binding {
            if binding.provider_epoch != authority.provider_epoch {
                self.poisoned = true;
                return Err(Refused::Authority);
            }
            if p.generation < binding.generation {
                return Err(Refused::Generation);
            }
            if p.generation == binding.generation {
                if self.poisoned {
                    return Err(Refused::Poisoned);
                }
                if let Some(last) = self.last {
                    if p == last {
                        return Ok(Accepted::Duplicate);
                    }
                    if p.phase == last.phase {
                        self.poisoned = true;
                        return Err(Refused::ChangedRetry);
                    }
                }
            }
        }
        if p.phase == Phase::Quarantine {
            if self.binding.is_some_and(|b| p.generation <= b.generation) {
                return Err(Refused::Generation);
            }
            let old = Subject::decode(p.a).ok_or(Refused::Invalid)?;
            if old != authority.reserved
                || !authority.inactive
                || p.b != authority.provider_epoch
                || authority.challenge == [0; 2]
            {
                return Err(Refused::Authority);
            }
            self.binding = Some(Binding {
                generation: p.generation,
                coordinator_epoch: 0,
                provider_epoch: authority.provider_epoch,
                previous: old,
                child: Subject { pd: 0, task: 0 },
                parent: Subject { pd: 0, task: 0 },
                cookie: 0,
                nonce: [0; 2],
                plugin_hash: [0; 4],
                input_hash: [0; 4],
                ticket: 0,
            });
            self.last = Some(p);
            self.challenge = authority.challenge;
            self.next = 2;
            self.poisoned = false;
            return Ok(Accepted::Quarantined);
        }
        let Some(mut b) = self.binding else {
            return Err(Refused::Order);
        };
        if p.generation != b.generation || p.phase as u8 != self.next {
            self.poisoned = true;
            return Err(Refused::Order);
        }
        match p.phase {
            Phase::EchoChallenge => {
                if [p.a, p.b] != self.challenge {
                    self.poisoned = true;
                    return Err(Refused::Authority);
                }
            }
            Phase::Coordinator => {
                if p.a == 0 || p.b != b.provider_epoch {
                    return Err(Refused::Authority);
                }
                b.coordinator_epoch = p.a;
            }
            Phase::Owner => {
                b.parent = Subject::decode(p.a).ok_or(Refused::Invalid)?;
                b.cookie = u32::try_from(p.b)
                    .ok()
                    .filter(|v| *v != 0)
                    .ok_or(Refused::Invalid)?;
                if b.parent.pd == b.previous.pd {
                    return Err(Refused::Invalid);
                }
            }
            Phase::Nonce => {
                if p.a == 0 && p.b == 0 {
                    return Err(Refused::Invalid);
                }
                b.nonce = [p.a, p.b]
            }
            Phase::PluginHashLo => b.plugin_hash[..2].copy_from_slice(&[p.a, p.b]),
            Phase::PluginHashHi => b.plugin_hash[2..].copy_from_slice(&[p.a, p.b]),
            Phase::InputHashLo => b.input_hash[..2].copy_from_slice(&[p.a, p.b]),
            Phase::InputHashHi => b.input_hash[2..].copy_from_slice(&[p.a, p.b]),
            Phase::Bind => {
                let child = Subject::decode(p.a).ok_or(Refused::Invalid)?;
                if child.pd != b.previous.pd || child_catalog != Some(child) || p.b == 0 {
                    return Err(Refused::Authority);
                }
                b.child = child;
                b.ticket = p.b;
            }
            Phase::Seal => {
                if Subject::decode(p.a) != Some(b.child)
                    || child_catalog != Some(b.child)
                    || p.b != b.provider_epoch
                {
                    return Err(Refused::Authority);
                }
            }
            Phase::Quarantine => unreachable!(),
        }
        self.binding = Some(b);
        self.last = Some(p);
        self.next += 1;
        Ok(if p.phase == Phase::Seal {
            Accepted::Sealed
        } else {
            Accepted::Staged
        })
    }
}

impl core::fmt::Debug for Packet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Packet")
            .field("generation", &self.generation)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

/// Full command acknowledgment over the existing one-word SYS29 reply path.
/// The provider creates this only after durable denial/scrub has completed.
/// Delivery state advances only after successful kernel reply admission.
pub struct ReplyCredits {
    packet: Packet,
    epoch: u64,
    challenge: [u64; 2],
    first_sent: bool,
    tagged: bool,
    first_low: bool,
    part: u8,
    last_tagged: Option<(u64, u8)>,
    next: u64,
    last: u64,
}
impl ReplyCredits {
    pub fn new(packet: Packet, epoch: u64, challenge: [u64; 2]) -> Option<Self> {
        packet.wire()?;
        if epoch == 0 || (packet.phase == Phase::Quarantine && challenge == [0; 2]) {
            return None;
        }
        Some(Self {
            packet,
            epoch,
            challenge,
            first_sent: false,
            tagged: false,
            first_low: false,
            part: 0,
            last_tagged: None,
            next: 1,
            last: 0,
        })
    }
    pub fn new_tagged(packet: Packet, epoch: u64, challenge: [u64; 2]) -> Option<Self> {
        let mut credits = Self::new(packet, epoch, challenge)?;
        credits.tagged = true;
        Some(credits)
    }
    pub fn first(&self, epoch: u64) -> Option<u64> {
        if epoch != self.epoch {
            return None;
        }
        let word = self.packet.wire()?[0];
        if self.tagged {
            chunk(
                self.packet.generation,
                self.packet.phase as u8,
                0,
                0,
                word as u32,
            )
        } else {
            Some(word)
        }
    }
    pub fn first_delivered(&mut self, epoch: u64) -> bool {
        if epoch != self.epoch {
            return false;
        }
        if self.tagged {
            self.first_low = true;
        } else {
            self.first_sent = true;
        }
        true
    }
    pub fn complete(&self) -> bool {
        self.first_sent && self.next > self.limit()
    }
    fn limit(&self) -> u64 {
        if self.packet.phase == Phase::Quarantine {
            4
        } else {
            2
        }
    }
    /// request = [exact phase/generation word0, selector1..4, provider-own epoch].
    /// caller identity is checked by the runtime before touching reply credits.
    pub fn fetch(&self, request: [u64; 3], epoch: u64) -> Option<u64> {
        if self.tagged {
            return None;
        }
        self.fetch_word(request, epoch)
    }
    fn fetch_word(&self, request: [u64; 3], epoch: u64) -> Option<u64> {
        if !self.first_sent
            || epoch != self.epoch
            || request[2] != self.epoch
            || request[0] != self.packet.wire()?[0]
            || request[1] == 0
            || request[1] > self.limit()
            || (request[1] != self.next && request[1] != self.last)
        {
            return None;
        }
        Some(match request[1] {
            1 => self.packet.a,
            2 => self.packet.b,
            3 => self.challenge[0],
            4 => self.challenge[1],
            _ => return None,
        })
    }
    pub fn fetch_delivered(&mut self, request: [u64; 3], epoch: u64) -> bool {
        if self.fetch(request, epoch).is_none() {
            return false;
        }
        if request[1] == self.next {
            self.last = self.next;
            self.next += 1;
        }
        true
    }
    pub fn fetch_tagged(&self, request: [u64; 3], epoch: u64) -> Option<u64> {
        if !self.tagged
            || epoch != self.epoch
            || request[2] != epoch
            || request[0] != self.packet.wire()?[0]
            || request[1] & !0x1ff != 0
        {
            return None;
        }
        let selector = request[1] & 255;
        let part = ((request[1] >> 8) & 1) as u8;
        let value = if selector == 0 {
            if !self.first_low || part != 1 || (self.first_sent && self.last_tagged != Some((0, 1)))
            {
                return None;
            }
            self.packet.wire()?[0]
        } else {
            if self.last_tagged != Some((selector, part))
                && (selector != self.next || part != self.part)
            {
                return None;
            }
            self.fetch_word([request[0], selector, request[2]], epoch)?
        };
        chunk(
            self.packet.generation,
            self.packet.phase as u8,
            selector as u8,
            part,
            (value >> (part as u32 * 32)) as u32,
        )
    }
    pub fn fetch_tagged_delivered(&mut self, request: [u64; 3], epoch: u64) -> bool {
        if self.fetch_tagged(request, epoch).is_none() {
            return false;
        }
        let selector = request[1] & 255;
        let part = ((request[1] >> 8) & 1) as u8;
        if self.last_tagged == Some((selector, part)) {
            return true;
        }
        if selector == 0 {
            self.first_sent = true;
        } else if part == 0 {
            self.part = 1;
        } else {
            self.last = self.next;
            self.next += 1;
            self.part = 0;
        }
        self.last_tagged = Some((selector, part));
        true
    }
}

/// Fresh provider challenge from CPU CSPRNG, never ticks/counters or a fallback.
/// Availability checks precede instructions; bounded retries fail closed.
#[cfg(target_arch = "x86_64")]
pub fn fresh_challenge() -> Option<[u64; 2]> {
    let seed = core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 18) != 0;
    let random = core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0;
    if !seed && !random {
        return None;
    }
    let mut result = [0; 2];
    for chunk in &mut result {
        let mut success = false;
        for _ in 0..128 {
            let mut word: u64;
            let ok: u8;
            unsafe {
                if seed {
                    core::arch::asm!("rdseed {word}","setc {ok}",word=out(reg)word,ok=out(reg_byte)ok,options(nomem,nostack));
                } else {
                    core::arch::asm!("rdrand {word}","setc {ok}",word=out(reg)word,ok=out(reg_byte)ok,options(nomem,nostack));
                }
            }
            if ok != 0 {
                *chunk = word;
                success = true;
                break;
            }
            // Do not retain a failed instruction's output as challenge material.
            unsafe {
                core::ptr::write_volatile(&mut word, 0);
            }
            core::hint::spin_loop();
        }
        if !success {
            for word in &mut result {
                unsafe { core::ptr::write_volatile(word, 0) }
            }
            return None;
        }
    }
    (result != [0; 2]).then_some(result)
}
#[cfg(not(target_arch = "x86_64"))]
pub fn fresh_challenge() -> Option<[u64; 2]> {
    None
}
