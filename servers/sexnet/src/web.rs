//! Serves the open-internet flows: a few TCP flows, each held by one caller under the
//! caller's own number for it. A client's bytes pass through its own BOOT region, and
//! each call is served in one turn.
use crate::jobs::Identity;
use sex_net::{Engine, OpenBuffers, OpenState};
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::web as w;
use sex_tls::open::{Session, Stage};
use sex_tls::Failure;

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec::Vec;
static mut DNS_RX: [u8; 512] = [0; 512];
static mut DNS_TX: [u8; 512] = [0; 512];
static mut DNS_RX_META: [sex_net::DnsMetadata; 2] = [sex_net::DnsMetadata::EMPTY; 2];
static mut DNS_TX_META: [sex_net::DnsMetadata; 2] = [sex_net::DnsMetadata::EMPTY; 2];

const TLS_READ_BYTES: usize = 16640;
const TLS_WRITE_BYTES: usize = 4096;
/// Flows are made when a caller asks and none is free, and kept for the next caller.
/// This many is a ceiling against one runaway program, not a working limit: about
/// 100 KiB each, made only as needed.
const MAX_FLOWS: usize = 4096;
/// Program runs Shell has bound to the network, by row: the run's task.
const PROGRAM_ROWS: usize = sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Program);
/// A question to the resolver is one at a time; another caller waits this long at most.
const RESOLVE_HOLD_TICKS: u64 = 5_000;
/// One flow of the service: who holds it (and under which of their own numbers) and,
/// when encrypted, its TLS session and the wire bytes still to send.
struct Slot { owner: Option<(Identity, u8)>, tls: Option<Session>, out: [u8; 1400], out_at: usize, out_len: usize, read: &'static mut [u8], write: &'static mut [u8] }
fn zeroed(n: usize) -> Option<&'static mut [u8]> {
    let mut v: Vec<u8> = Vec::new();
    v.try_reserve_exact(n).ok()?;
    v.resize(n, 0);
    Some(Box::leak(v.into_boxed_slice()))
}
impl Slot {
    /// A slot for a new flow, with its own TLS buffers; `None` when memory is short.
    fn new() -> Option<Slot> { Some(Slot { owner: None, tls: None, out: [0; 1400], out_at: 0, out_len: 0, read: zeroed(TLS_READ_BYTES)?, write: zeroed(TLS_WRITE_BYTES)? }) }
    /// Moves encrypted bytes between flow `i` and its session.
    fn pump(&mut self, i: usize, engine: &mut Engine<'static>) {
        let Some(tls) = self.tls.as_mut() else { return };
        for _ in 0..4 {
            let mut chunk = [0u8; 1400];
            let room = tls.wire_room().min(chunk.len());
            if room > 0 {
                if let Ok(n) = engine.open_receive(i, &mut chunk[..room]) { if n > 0 { tls.offer_wire(&chunk[..n]); } }
            }
            if matches!(engine.open_state(i), OpenState::PeerClosed | OpenState::Closed) { tls.peer_closed(); }
            tls.poll();
            if self.out_at == self.out_len { self.out_len = tls.take_wire(&mut self.out); self.out_at = 0; }
            if self.out_at < self.out_len {
                if let Ok(n) = engine.open_send(i, &self.out[self.out_at..self.out_len]) { self.out_at += n; }
            }
        }
    }
    fn free(&mut self, i: usize, engine: Option<&mut Engine<'static>>) {
        self.owner = None;
        self.tls = None;
        self.out_at = 0; self.out_len = 0;
        if let Some(e) = engine { e.open_close(i); }
    }
}
pub struct Service { bound: [u32; PROGRAM_ROWS], slots: Vec<Slot>, resolving: Option<(Identity, u64)>, attached: bool, announced: bool, tls_used: bool }
fn program_index(target: u32) -> Option<usize> {
    sex_pdx::app_catalog::INCARNATIONS.iter().filter(|i| i.app == sex_pdx::app_catalog::App::Program).position(|i| i.target == target)
}
impl Service {
    pub const fn new() -> Self { Self { bound: [0; PROGRAM_ROWS], slots: Vec::new(), resolving: None, attached: false, announced: false, tls_used: false } }
    /// One more flow, when every flow made so far is held.
    fn grow(&mut self, engine: &mut Engine<'static>) -> Option<usize> {
        if self.slots.len() >= MAX_FLOWS { return None; }
        self.slots.try_reserve(1).ok()?;
        let slot = Slot::new()?;
        let i = engine.open_add_flow().ok()?;
        // The engine's flows and these slots are made together, one for one.
        debug_assert_eq!(i, self.slots.len());
        self.slots.push(slot);
        sex_pdx::serial_record!("[net.open.flows] made={}", self.slots.len());
        Some(i)
    }
    pub fn busy(&self) -> bool { self.slots.iter().any(|s| s.tls.is_some()) }
    /// Moves encrypted bytes between the flows and their sessions, each turn of the service.
    pub fn pump(&mut self, engine: &mut Engine<'static>) {
        for (i, slot) in self.slots.iter_mut().enumerate() { slot.pump(i, engine); }
    }
    fn seed_and_time() -> Option<([u8; 32], u32)> {
        let plan = sex_pdx::browser_sources::PLANS[0];
        let mut seed = [0u8; 32];
        for chunk in seed.chunks_exact_mut(8) {
            let mut word = None;
            for _ in 0..64 { if let Ok(w) = sex_pdx::entropy_word(plan.entropy_slot()) { word = Some(w.value); break; } }
            chunk.copy_from_slice(&word?.to_le_bytes());
        }
        let seconds = sex_pdx::clock_snapshot(plan.clock_slot()).ok()?.seconds;
        Some((seed, u32::try_from(seconds).ok()?))
    }
    /// Shell's word that this run of a program row may use the network.
    pub fn bind(&mut self, target: u32, task: u32) {
        if let Some(k) = program_index(target) { self.bound[k] = task; sex_pdx::serial_record!("[net.program.bound] target={} task={}", target, task); }
    }
    /// A program that ended holds nothing here.
    pub fn retire(&mut self, who: Identity, mut engine: Option<&mut Engine<'static>>) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.owner.is_some_and(|(o, _)| o == who) { slot.free(i, engine.as_deref_mut()); }
        }
        if self.resolving.is_some_and(|(o, _)| o == who) { self.resolving = None; }
    }
    pub fn handle(&mut self, who: Identity, a: u64, value: u64, engine: Option<&mut Engine<'static>>) -> u64 {
        let seq = (a & 0xFF_FFFF) as u32;
        let kind = (a >> 32) as u8;
        let number = w::flow_of(a);
        let Some(engine) = engine else { return w::reply(seq, w::OFFLINE, 0) };
        let spindles = sex_pdx::app_catalog::SPINDLE_TARGETS;
        let program = program_index(who.pd);
        // A program reaches the network only in a run Shell bound, because the person asked.
        if program.is_some_and(|k| self.bound[k] != who.task || who.task == 0) { return w::reply(seq, w::DENIED, 0); }
        let index = spindles.iter().position(|t| *t == who.pd)
            .or_else(|| sex_pdx::browser_sources::PLANS.iter().position(|p| p.target() == who.pd).map(|i| spindles.len() + i))
            .or_else(|| program.map(|k| w::PROGRAM_INDEX_BASE + k));
        let Some(index) = index else { return w::reply(seq, w::DENIED, 0) };
        let Ok(view) = sex_pdx::memory_region_view(w::PROVIDER_SLOT_BASE + index as u64, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE) else { return w::reply(seq, w::FAILED, 0) };
        let region = unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) };
        if number >= w::CLIENT_FLOWS { return w::reply(seq, w::INVALID, 0); }
        if !self.attached {
            let b = unsafe { OpenBuffers {
                dns_rx_meta: &mut *core::ptr::addr_of_mut!(DNS_RX_META), dns_rx: &mut *core::ptr::addr_of_mut!(DNS_RX),
                dns_tx_meta: &mut *core::ptr::addr_of_mut!(DNS_TX_META), dns_tx: &mut *core::ptr::addr_of_mut!(DNS_TX) } };
            if engine.open_attach(b).is_err() { return w::reply(seq, w::FAILED, 0); }
            self.attached = true;
        }
        // The caller's flow by its own number. CONNECT takes a free one when it has none.
        let mine = self.slots.iter().position(|s| s.owner == Some((who, number)));
        let flow = match kind {
            w::CONNECT => match mine.or_else(|| self.slots.iter().position(|s| s.owner.is_none())).or_else(|| self.grow(engine)) {
                Some(i) => { self.slots[i].owner = Some((who, number)); Some(i) }
                None => return w::reply(seq, w::BUSY, 0),
            },
            w::UP | w::RESOLVE => None,
            // A flow that is not the caller's is closed, as far as the caller can tell.
            w::STATE if mine.is_none() => return w::reply(seq, w::OK, w::STATE_CLOSED),
            w::CLOSE if mine.is_none() => return w::reply(seq, w::OK, 0),
            _ => match mine { Some(i) => Some(i), None => return w::reply(seq, w::FAILED, 0) },
        };
        let i = flow.unwrap_or(0);
        let now = sex_pdx::get_ticks();
        match kind {
            w::UP => match engine.open_lease() {
                Some(l) => {
                    if !self.announced {
                        self.announced = true;
                        let (g, d) = (l.gateway.unwrap_or([0; 4]), l.dns.unwrap_or([0; 4]));
                        sex_pdx::serial_record!("[net.dhcp.lease] address={}.{}.{}.{}/{} gateway={}.{}.{}.{} dns={}.{}.{}.{}", l.address[0], l.address[1], l.address[2], l.address[3], l.prefix, g[0], g[1], g[2], g[3], d[0], d[1], d[2], d[3]);
                    }
                    w::reply(seq, w::OK, 0)
                }
                None => w::reply(seq, w::WAIT, 0),
            },
            w::RESOLVE => {
                // One question at a time: another caller's waits until it is answered.
                if self.resolving.is_some_and(|(o, at)| o != who && now.saturating_sub(at) < RESOLVE_HOLD_TICKS) { return w::reply(seq, w::BUSY, 0); }
                // value 0 asks for the next answer to the question already sent.
                if value != 0 {
                    self.resolving = Some((who, now));
                    let len = value as usize;
                    if len > 253 { return w::reply(seq, w::INVALID, 0); }
                    let mut name = [0u8; 253];
                    name[..len].copy_from_slice(&region[..len]);
                    // Test builds only: a name that makes the service fall over, to show it comes back.
                    if option_env!("SEXOS_NET_CRASH_PROBE").is_some() && &name[..len] == b"crash.invalid" { sex_pdx::serial_record!("[sexnet.crash.probe]"); unsafe { core::ptr::write_volatile(8 as *mut u8, 1) }; }
                    return match engine.open_resolve(&name[..len]) { Ok(()) => w::reply(seq, w::WAIT, 0), Err(_) => { self.resolving = None; w::reply(seq, w::NO_NAME, 0) } };
                }
                if self.resolving.map(|(o, _)| o) != Some(who) { return w::reply(seq, w::NO_NAME, 0); }
                match engine.open_resolved() {
                    Ok(Some(address)) => { self.resolving = None; region[..4].copy_from_slice(&address); w::reply(seq, w::OK, 0) }
                    Ok(None) => w::reply(seq, w::WAIT, 0),
                    Err(_) => { self.resolving = None; w::reply(seq, w::NO_NAME, 0) }
                }
            }
            w::TLS_START => {
                let len = value as usize;
                if len == 0 || len > 253 { return w::reply(seq, w::INVALID, 0); }
                let mut name = [0u8; 253];
                name[..len].copy_from_slice(&region[..len]);
                let Ok(host) = core::str::from_utf8(&name[..len]) else { return w::reply(seq, w::INVALID, 0) };
                let Some((mut seed, seconds)) = Self::seed_and_time() else { return w::reply(seq, w::FAILED, 0) };
                let slot = &mut self.slots[i];
                // The old session lets go of this flow's buffers before a new one takes them.
                slot.tls = None;
                slot.out_at = 0; slot.out_len = 0;
                // The session borrows the slot's buffers for as long as it lives; the
                // slot is never freed, and a new session starts only after the old one
                // is dropped above.
                let (read, write) = unsafe { (&mut *(&mut *slot.read as *mut [u8]), &mut *(&mut *slot.write as *mut [u8])) };
                match Session::start(host, &mut seed, seconds, read, write) {
                    Ok(session) => { slot.tls = Some(session); self.tls_used = true; w::reply(seq, w::OK, 0) }
                    Err(_) => w::reply(seq, w::FAILED, 0),
                }
            }
            w::TLS_STAGE => {
                let Some(tls) = self.slots[i].tls.as_ref() else { return w::reply(seq, w::FAILED, 0) };
                let value = match tls.stage() {
                    Stage::Handshake => w::TLS_HANDSHAKE,
                    Stage::Ready | Stage::Finished => w::TLS_READY,
                    Stage::Failed(Some(Failure::Certificate)) => w::TLS_FAILED_CERTIFICATE,
                    Stage::Failed(Some(Failure::Signature)) => w::TLS_FAILED_SIGNATURE,
                    Stage::Failed(_) => w::TLS_FAILED_OTHER,
                };
                w::reply(seq, w::OK, value)
            }
            w::FINISH_REQUEST => {
                if let Some(tls) = self.slots[i].tls.as_mut() { tls.finish_request(); }
                w::reply(seq, w::OK, 0)
            }
            w::CONNECT => {
                let address = ((value >> 16) as u32).to_be_bytes();
                let slot = &mut self.slots[i];
                slot.tls = None;
                slot.out_at = 0; slot.out_len = 0;
                match engine.open_connect(i, address, value as u16) { Ok(()) => w::reply(seq, w::OK, 0), Err(_) => { slot.free(i, Some(engine)); w::reply(seq, w::FAILED, 0) } }
            }
            w::STATE if self.slots[i].tls.is_some() => {
                let drained = self.slots[i].tls.as_ref().is_some_and(|t| t.drained());
                w::reply(seq, w::OK, if drained { w::STATE_PEER_CLOSED } else { w::STATE_OPEN })
            }
            w::STATE => {
                let state = match engine.open_state(i) {
                    OpenState::Opening => w::STATE_OPENING, OpenState::Open => w::STATE_OPEN,
                    OpenState::PeerClosed => w::STATE_PEER_CLOSED, OpenState::Closed => w::STATE_CLOSED,
                };
                w::reply(seq, w::OK, state)
            }
            w::SEND => {
                let len = (value as usize).min(w::REGION_BYTES);
                let mut own = [0u8; 1024];
                let take = len.min(own.len());
                own[..take].copy_from_slice(&region[..take]);
                if let Some(tls) = self.slots[i].tls.as_mut() { return w::reply(seq, w::OK, tls.send(&own[..take]) as u32); }
                match engine.open_send(i, &own[..take]) { Ok(n) => w::reply(seq, w::OK, n as u32), Err(_) => w::reply(seq, w::FAILED, 0) }
            }
            w::RECEIVE => {
                let len = (value as usize).min(w::REGION_BYTES);
                if let Some(tls) = self.slots[i].tls.as_mut() { return w::reply(seq, w::OK, tls.receive(&mut region[..len]) as u32); }
                match engine.open_receive(i, &mut region[..len]) { Ok(n) => w::reply(seq, w::OK, n as u32), Err(_) => w::reply(seq, w::FAILED, 0) }
            }
            w::CLOSE => { self.slots[i].free(i, Some(engine)); w::reply(seq, w::OK, 0) }
            _ => w::reply(seq, w::INVALID, 0),
        }
    }
}
