//! Serves the open-internet flows: TCP flows (outbound, or listening for a peer) and
//! UDP sockets, over IPv4 or IPv6, each held by one caller under the caller's own
//! number for it. A client's bytes pass through its own BOOT region, and
//! each call is served in one turn.
use crate::jobs::Identity;
use sex_net::{Address, Engine, OpenBuffers, OpenState};
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::web as w;
use sex_tls::open::{Session, Stage};
use sex_tls::Failure;

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec::Vec;
// Room for a name's A and AAAA questions and both their answers at once.
static mut DNS_RX: [u8; 2048] = [0; 2048];
static mut DNS_TX: [u8; 1024] = [0; 1024];
static mut DNS_RX_META: [sex_net::DnsMetadata; 4] = [sex_net::DnsMetadata::EMPTY; 4];
static mut DNS_TX_META: [sex_net::DnsMetadata; 4] = [sex_net::DnsMetadata::EMPTY; 4];
const _: () = assert!(sex_net::PROGRAM_PORT_MAX == w::PROGRAM_PORT_MAX);

/// Flows are made when a caller asks and none is free, and kept for the next caller.
/// This many is a ceiling against one runaway program, not a working limit: about
/// 100 KiB each, made only as needed.
const MAX_FLOWS: usize = 4096;
/// The same ceiling for UDP sockets (about 32 KiB each).
const MAX_SOCKETS: usize = 4096;
/// Program runs Shell has bound to the network, by row: the run's task.
const PROGRAM_ROWS: usize = sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Program);
/// A question to the resolver is one at a time; another caller waits this long at most.
const RESOLVE_HOLD_TICKS: u64 = 5_000;
/// One flow of the service: who holds it (and under which of their own numbers) and,
/// when encrypted, its TLS session and the wire bytes still to send. `listening` is the
/// port a listening flow listens on.
struct Slot { owner: Option<(Identity, u8)>, listening: u16, tls: Option<Session>, told: bool, out: [u8; 1400], out_at: usize, out_len: usize }
impl Slot {
    /// A slot for a new flow. Its TLS session, when it has one, sizes its own queues.
    fn new() -> Option<Slot> { Some(Slot { owner: None, listening: 0, tls: None, told: false, out: [0; 1400], out_at: 0, out_len: 0 }) }
    /// Moves encrypted bytes between flow `i` and its session.
    fn pump(&mut self, i: usize, engine: &mut Engine<'static>) {
        let Some(tls) = self.tls.as_mut() else { return };
        for _ in 0..4 {
            let mut chunk = [0u8; 1400];
            let room = tls.wire_room().min(chunk.len());
            let mut drained = false;
            if room > 0 {
                match engine.open_receive(i, &mut chunk[..room]) { Ok(0) | Err(_) => drained = true, Ok(n) => tls.offer_wire(&chunk[..n]) }
            }
            // Closed only once the flow holds nothing more for the session.
            if drained && matches!(engine.open_state(i), OpenState::PeerClosed | OpenState::Closed) { tls.peer_closed(); }
            tls.poll();
            if self.out_at == self.out_len { self.out_len = tls.take_wire(&mut self.out); self.out_at = 0; }
            if self.out_at < self.out_len {
                if let Ok(n) = engine.open_send(i, &self.out[self.out_at..self.out_len]) { self.out_at += n; }
            }
        }
        // What the site agreed to, once per session: the version, suite and key exchange.
        if !self.told && tls.stage() == Stage::Ready {
            self.told = true;
            if let Some((version, suite, group)) = tls.agreed() { sex_pdx::serial_record!("[net.tls.agreed] flow={} version={:?} suite={:?} group={:?}", i, version, suite, group); }
        }
    }
    fn free(&mut self, i: usize, engine: Option<&mut Engine<'static>>) {
        self.owner = None;
        self.listening = 0;
        self.tls = None;
        self.out_at = 0; self.out_len = 0;
        if let Some(e) = engine { e.open_close(i); }
    }
}
pub struct Service { bound: [u32; PROGRAM_ROWS], slots: Vec<Slot>, sockets: Vec<Option<(Identity, u8)>>, resolving: Option<(Identity, u64)>, attached: bool, announced: bool, announced6: bool, tls_used: bool }
fn address(family: u8, bytes: &[u8]) -> Option<Address> {
    match family {
        w::FAMILY_V4 => Some(Address::V4(bytes.try_into().ok()?)),
        w::FAMILY_V6 => Some(Address::V6(bytes.try_into().ok()?)),
        _ => None,
    }
}
fn put(out: &mut [u8], a: Address) {
    match a { Address::V4(a) => w::put_address(out, w::FAMILY_V4, &a), Address::V6(a) => w::put_address(out, w::FAMILY_V6, &a) }
}
fn hex6(a: &[u8; 16]) -> [u16; 8] { core::array::from_fn(|i| u16::from_be_bytes([a[2 * i], a[2 * i + 1]])) }
fn program_index(target: u32) -> Option<usize> {
    sex_pdx::app_catalog::INCARNATIONS.iter().filter(|i| i.app == sex_pdx::app_catalog::App::Program).position(|i| i.target == target)
}
impl Service {
    pub const fn new() -> Self { Self { bound: [0; PROGRAM_ROWS], slots: Vec::new(), sockets: Vec::new(), resolving: None, attached: false, announced: false, announced6: false, tls_used: false } }
    /// The IP stack under the service was replaced (Wi-Fi came up or went down):
    /// flows, sockets and the lease note belonged to the old one. Run-Shell bindings stay.
    pub fn reset(&mut self) { let bound = self.bound; *self = Self::new(); self.bound = bound; }
    /// A free flow for a new connection or listener (not `except`): one the stack is done
    /// with first, so a connection still finishing is left to finish; then a new one.
    fn free_slot(&mut self, engine: &mut Engine<'static>, except: Option<usize>) -> Option<usize> {
        let free = |i: usize, s: &Slot| s.owner.is_none() && Some(i) != except;
        self.slots.iter().enumerate().position(|(i, s)| free(i, s) && engine.open_idle(i))
            .or_else(|| self.grow(engine))
            .or_else(|| self.slots.iter().enumerate().position(|(i, s)| free(i, s)))
    }
    /// One more UDP socket.
    fn grow_sockets(&mut self, engine: &mut Engine<'static>) -> Option<usize> {
        if self.sockets.len() >= MAX_SOCKETS { return None; }
        self.sockets.try_reserve(1).ok()?;
        let i = engine.open_add_udp().ok()?;
        debug_assert_eq!(i, self.sockets.len());
        self.sockets.push(None);
        Some(i)
    }
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
    /// The link went away and its stack with it (a USB adapter pulled out): every flow and
    /// socket is gone. Programs keep their right to the network for the next link.
    pub fn reset_link(&mut self) {
        let bound = self.bound;
        *self = Self::new();
        self.bound = bound;
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
        for (i, socket) in self.sockets.iter_mut().enumerate() {
            if socket.is_some_and(|(o, _)| o == who) { *socket = None; if let Some(e) = engine.as_deref_mut() { e.open_udp_close(i); } }
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
        // The caller's flow by its own number. CONNECT and LISTEN take a free one when it
        // has none. UDP sockets are numbered by the caller in the same way, apart.
        let mine = self.slots.iter().position(|s| s.owner == Some((who, number)));
        let socket = self.sockets.iter().position(|s| *s == Some((who, number)));
        match kind {
            w::UDP_BIND => {
                let Some(k) = socket.or_else(|| self.sockets.iter().position(|s| s.is_none())).or_else(|| self.grow_sockets(engine)) else { return w::reply(seq, w::BUSY, 0) };
                if value > u16::MAX as u64 { return w::reply(seq, w::INVALID, 0); }
                return match engine.open_udp_bind(k, value as u16) {
                    Ok(port) => { self.sockets[k] = Some((who, number)); w::reply(seq, w::OK, port as u32) }
                    Err(e) => { self.sockets[k] = None; engine.open_udp_close(k); w::reply(seq, if e == sex_net::Error::Busy { w::IN_USE } else { w::FAILED }, 0) }
                };
            }
            w::UDP_SEND => {
                let Some(k) = socket else { return w::reply(seq, w::FAILED, 0) };
                let len = value as usize;
                if len > w::REGION_BYTES - w::ENDPOINT_BYTES { return w::reply(seq, w::INVALID, 0); }
                let Some((family, bytes, port)) = w::endpoint_of(&region[..w::ENDPOINT_BYTES]) else { return w::reply(seq, w::INVALID, 0) };
                let Some(to) = address(family, bytes) else { return w::reply(seq, w::INVALID, 0) };
                let payload = &region[w::ENDPOINT_BYTES..w::ENDPOINT_BYTES + len];
                return match engine.open_udp_send(k, to, port, payload) {
                    Ok(true) => w::reply(seq, w::OK, len as u32), Ok(false) => w::reply(seq, w::WAIT, 0),
                    Err(sex_net::Error::Length) => w::reply(seq, w::INVALID, 0), Err(_) => w::reply(seq, w::FAILED, 0),
                };
            }
            w::UDP_RECEIVE => {
                let Some(k) = socket else { return w::reply(seq, w::FAILED, 0) };
                let room = (value as usize).min(w::REGION_BYTES - w::ENDPOINT_BYTES);
                let (head, body) = region.split_at_mut(w::ENDPOINT_BYTES);
                return match engine.open_udp_receive(k, &mut body[..room]) {
                    Ok(Some((from, port, n))) => { put(head, from); head[17..19].copy_from_slice(&port.to_be_bytes()); w::reply(seq, w::OK, n as u32) }
                    Ok(None) => w::reply(seq, w::WAIT, 0),
                    Err(_) => w::reply(seq, w::FAILED, 0),
                };
            }
            w::CLOSE => if let Some(k) = socket {
                self.sockets[k] = None; engine.open_udp_close(k);
                if mine.is_none() { return w::reply(seq, w::OK, 0); }
            },
            _ => {}
        }
        let flow = match kind {
            w::CONNECT | w::CONNECT_TO | w::LISTEN => match mine.or_else(|| self.free_slot(engine, None)) {
                Some(i) => { self.slots[i].owner = Some((who, number)); self.slots[i].listening = 0; Some(i) }
                None => return w::reply(seq, w::BUSY, 0),
            },
            w::UP | w::RESOLVE | w::RESOLVE_ALL | w::ADDRESSES => None,
            // A flow that is not the caller's is closed, as far as the caller can tell.
            w::STATE if mine.is_none() => return w::reply(seq, w::OK, w::STATE_CLOSED),
            w::CLOSE if mine.is_none() => return w::reply(seq, w::OK, 0),
            _ => match mine { Some(i) => Some(i), None => return w::reply(seq, w::FAILED, 0) },
        };
        let i = flow.unwrap_or(0);
        let now = sex_pdx::get_ticks();
        match kind {
            w::UP => {
                if let Some(l) = engine.open_lease().filter(|_| !self.announced) {
                    self.announced = true;
                    let (g, d) = (l.gateway.unwrap_or([0; 4]), l.dns.unwrap_or([0; 4]));
                    sex_pdx::serial_record!("[net.dhcp.lease] address={}.{}.{}.{}/{} gateway={}.{}.{}.{} dns={}.{}.{}.{}", l.address[0], l.address[1], l.address[2], l.address[3], l.prefix, g[0], g[1], g[2], g[3], d[0], d[1], d[2], d[3]);
                }
                if let Some(a) = engine.open_global6().filter(|_| !self.announced6) {
                    self.announced6 = true;
                    let (a, r) = (hex6(&a), hex6(&engine.open_router6().unwrap_or([0; 16])));
                    // Every group, in hex; programs show the short form.
                    sex_pdx::serial_record!("[net.ipv6.slaac] address={:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x} router={:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}", a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]);
                }
                // Either family is enough: a DHCP lease, or a global IPv6 address.
                if engine.open_ready() { w::reply(seq, w::OK, 0) } else { w::reply(seq, w::WAIT, 0) }
            }
            w::ADDRESSES => {
                let mut n = 0;
                for (a, prefix) in engine.open_addresses() {
                    if (n + 1) * (w::ADDRESS_BYTES + 1) > w::REGION_BYTES { break; }
                    let at = n * (w::ADDRESS_BYTES + 1);
                    put(&mut region[at..], a); region[at + w::ADDRESS_BYTES] = prefix; n += 1;
                }
                w::reply(seq, w::OK, n as u32)
            }
            w::RESOLVE | w::RESOLVE_ALL => {
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
                    let asked = if kind == w::RESOLVE { engine.open_resolve(&name[..len]) } else { engine.open_resolve_all(&name[..len]) };
                    return match asked { Ok(()) => w::reply(seq, w::WAIT, 0), Err(_) => { self.resolving = None; w::reply(seq, w::NO_NAME, 0) } };
                }
                if self.resolving.map(|(o, _)| o) != Some(who) { return w::reply(seq, w::NO_NAME, 0); }
                if kind == w::RESOLVE_ALL {
                    return match engine.open_resolved_all() {
                        Ok(Some(found)) => {
                            self.resolving = None;
                            // IPv6 first: it was asked only when the machine can use it.
                            let mut n = 0;
                            for a in [found.v6.map(Address::V6), found.v4.map(Address::V4)].into_iter().flatten() { put(&mut region[n * w::ADDRESS_BYTES..], a); n += 1; }
                            w::reply(seq, w::OK, n as u32)
                        }
                        Ok(None) => w::reply(seq, w::WAIT, 0),
                        Err(_) => { self.resolving = None; w::reply(seq, w::NO_NAME, 0) }
                    };
                }
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
                slot.tls = None;
                slot.out_at = 0; slot.out_len = 0;
                match Session::start(host, &mut seed, seconds) {
                    Ok(session) => { slot.tls = Some(session); slot.told = false; self.tls_used = true; w::reply(seq, w::OK, 0) }
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
            w::CONNECT | w::CONNECT_TO => {
                let to = if kind == w::CONNECT { Some(Address::V4(((value >> 16) as u32).to_be_bytes())) }
                    else { w::address_of(&region[..w::ADDRESS_BYTES]).and_then(|(f, b)| address(f, b)) };
                let port = if kind == w::CONNECT { value as u16 } else { u16::try_from(value).unwrap_or(0) };
                let slot = &mut self.slots[i];
                slot.tls = None;
                slot.out_at = 0; slot.out_len = 0;
                let Some(to) = to.filter(|_| port != 0) else { slot.free(i, Some(engine)); return w::reply(seq, w::INVALID, 0) };
                match engine.open_connect(i, to, port) { Ok(()) => w::reply(seq, w::OK, 0), Err(_) => { slot.free(i, Some(engine)); w::reply(seq, w::FAILED, 0) } }
            }
            w::LISTEN => {
                let slot = &mut self.slots[i];
                slot.tls = None;
                slot.out_at = 0; slot.out_len = 0;
                let port = u16::try_from(value).unwrap_or(0);
                match engine.open_listen(i, port) {
                    Ok(()) => { slot.listening = port; w::reply(seq, w::OK, 0) }
                    Err(e) => { slot.free(i, Some(engine)); w::reply(seq, if e == sex_net::Error::Busy { w::IN_USE } else { w::FAILED }, 0) }
                }
            }
            w::ACCEPT => {
                let port = self.slots[i].listening;
                let Ok(new) = u8::try_from(value) else { return w::reply(seq, w::INVALID, 0) };
                if port == 0 || new >= w::CLIENT_FLOWS || self.slots.iter().any(|s| s.owner == Some((who, new))) { return w::reply(seq, w::INVALID, 0); }
                if !engine.open_connected(i) { return w::reply(seq, w::WAIT, 0); }
                // The connection keeps its flow under the new number; another flow takes
                // over listening under the old one.
                let Some(j) = self.free_slot(engine, Some(i)) else { return w::reply(seq, w::BUSY, 0) };
                if engine.open_listen(j, port).is_err() { return w::reply(seq, w::FAILED, 0); }
                self.slots[j].owner = Some((who, number)); self.slots[j].listening = port;
                self.slots[i].owner = Some((who, new)); self.slots[i].listening = 0;
                if let Some((a, p)) = engine.open_peer(i) { put(region, a); region[17..19].copy_from_slice(&p.to_be_bytes()); }
                w::reply(seq, w::OK, 0)
            }
            w::SHUTDOWN => { engine.open_shutdown(i); w::reply(seq, w::OK, 0) }
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
                if let Some(tls) = self.slots[i].tls.as_mut() {
                    // A session whose task has ended takes nothing more (a kept connection
                    // the site closed): fail, so the caller starts over on a new one.
                    if !matches!(tls.stage(), Stage::Handshake | Stage::Ready) { return w::reply(seq, w::FAILED, 0); }
                    return w::reply(seq, w::OK, tls.send(&own[..take]) as u32);
                }
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
