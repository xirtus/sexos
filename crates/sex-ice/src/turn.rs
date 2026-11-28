// SPDX-License-Identifier: GPL-3.0-only
//! A TURN client (RFC 8656) for one allocation on one server: allocate with long-term
//! credentials, refresh, permissions, channels, and data both ways. No I/O: everything
//! it sends goes to the server (`poll_transmit`), over UDP or a TCP or TLS stream the
//! program holds; what the server sends comes in through `handle` (one datagram) or
//! `handle_stream` (stream bytes, cut into messages here).

use crate::rand::Rng;
use crate::stun::{self, attr, code, method, Class, Framer, Message};
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};

/// How the program reaches the server. StreamYard-style services also offer TURN on
/// TCP or TLS port 443 for networks that block UDP; TLS is the program's stream
/// (through the network service's checked TLS), the bytes here are the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport { Udp, Tcp, Tls }
impl Transport { pub fn stream(self) -> bool { self != Transport::Udp } }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config { pub server: SocketAddr, pub username: String, pub password: String, pub transport: Transport }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Allocated { relayed: SocketAddr, mapped: Option<SocketAddr>, lifetime: u32 },
    /// The allocation could not be made or was lost: the code and reason the server gave
    /// (0 when it never answered).
    Failed { code: u16, reason: String },
    Permitted(IpAddr),
    ChannelBound { peer: SocketAddr, channel: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Idle, Allocating, Allocated, Failed, Closed }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind { Allocate, Refresh(u32), Permission(IpAddr), Channel(SocketAddr, u16) }

#[derive(Debug)]
struct Pending { tid: [u8; 12], kind: Kind, raw: Vec<u8>, sent: u64, rto: u64, tries: u32, stale_retries: u32 }

#[derive(Debug)]
struct Permission { ip: IpAddr, refresh_at: u64, ready: bool }
#[derive(Debug)]
struct Channel { peer: SocketAddr, number: u16, refresh_at: u64, ready: bool }

/// Lifetimes from RFC 8656: permissions last 300 s and channels 600 s; refresh each a
/// minute early.
const PERMISSION_REFRESH: u64 = 240_000;
const CHANNEL_REFRESH: u64 = 540_000;
const RTO: u64 = 500;
const TRIES: u32 = 7;
/// On a stream there is no retransmission; a request that gets no answer in this long fails.
const STREAM_TIMEOUT: u64 = 39_500;

#[derive(Debug)]
pub struct Client {
    pub config: Config,
    rng: Rng,
    state: State,
    realm: Option<String>,
    nonce: Option<Vec<u8>>,
    key: Option<[u8; 16]>,
    relayed: Option<SocketAddr>,
    mapped: Option<SocketAddr>,
    refresh_at: u64,
    pending: Vec<Pending>,
    permissions: Vec<Permission>,
    channels: Vec<Channel>,
    next_channel: u16,
    out: VecDeque<Vec<u8>>,
    events: VecDeque<Event>,
    framer: Framer,
}

impl Client {
    pub fn new(config: Config, rng: Rng) -> Self {
        Client {
            config, rng, state: State::Idle, realm: None, nonce: None, key: None, relayed: None, mapped: None, refresh_at: 0,
            pending: Vec::new(), permissions: Vec::new(), channels: Vec::new(), next_channel: 0x4000,
            out: VecDeque::new(), events: VecDeque::new(), framer: Framer::default(),
        }
    }
    pub fn state(&self) -> State { self.state }
    pub fn relayed(&self) -> Option<SocketAddr> { self.relayed }
    pub fn mapped(&self) -> Option<SocketAddr> { self.mapped }
    pub fn server(&self) -> SocketAddr { self.config.server }
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> { self.out.pop_front() }
    pub fn poll_event(&mut self) -> Option<Event> { self.events.pop_front() }

    /// Asks for an allocation.
    pub fn start(&mut self, now: u64) {
        if self.state != State::Idle { return; }
        self.state = State::Allocating;
        self.request(now, Kind::Allocate);
    }

    /// Gives the allocation back (a Refresh with lifetime 0).
    pub fn close(&mut self, now: u64) {
        if self.state == State::Allocated { self.request(now, Kind::Refresh(0)); }
        self.state = State::Closed;
    }

    pub fn has_permission(&self, ip: IpAddr) -> bool { self.permissions.iter().any(|p| p.ip == ip && p.ready) }

    /// Lets `ip` send to the relayed address (and this side send to it).
    pub fn permit(&mut self, now: u64, ip: IpAddr) {
        if self.permissions.iter().any(|p| p.ip == ip) { return; }
        self.permissions.push(Permission { ip, refresh_at: u64::MAX, ready: false });
        if self.state == State::Allocated { self.request(now, Kind::Permission(ip)); }
    }

    /// Binds a channel to `peer`, so data to and from it goes in 4-byte headers.
    pub fn bind_channel(&mut self, now: u64, peer: SocketAddr) {
        if self.channels.iter().any(|c| c.peer == peer) || self.next_channel > 0x4FFF { return; }
        let number = self.next_channel;
        self.next_channel += 1;
        self.channels.push(Channel { peer, number, refresh_at: u64::MAX, ready: false });
        if self.state == State::Allocated { self.request(now, Kind::Channel(peer, number)); }
    }

    /// Sends `data` to `peer` through the relay: as ChannelData on a bound channel, else
    /// in a Send indication. False when there is no allocation yet.
    pub fn send_to(&mut self, peer: SocketAddr, data: &[u8]) -> bool {
        if self.state != State::Allocated { return false; }
        let bytes = match self.channels.iter().find(|c| c.peer == peer && c.ready) {
            Some(c) => stun::channel_data(c.number, data, self.config.transport.stream()),
            None => {
                let mut m = Message::new(Class::Indication, method::SEND, self.rng.transaction());
                m.add_xor_address(attr::XOR_PEER_ADDRESS, peer).add(attr::DATA, data);
                m.encode(None, false)
            }
        };
        self.out.push_back(bytes);
        true
    }

    /// Bytes read from the stream to the server. Returns what peers sent.
    pub fn handle_stream(&mut self, now: u64, bytes: &[u8]) -> Vec<(SocketAddr, Vec<u8>)> {
        self.framer.push(bytes);
        let mut got = Vec::new();
        while let Some(m) = self.framer.next() {
            if let Some(d) = self.handle(now, &m) { got.push(d); }
        }
        got
    }

    /// One message from the server. Returns what a peer sent, if it was data.
    pub fn handle(&mut self, now: u64, b: &[u8]) -> Option<(SocketAddr, Vec<u8>)> {
        if let Some((ch, data)) = stun::read_channel_data(b) {
            let c = self.channels.iter().find(|c| c.number == ch && c.ready)?;
            return Some((c.peer, data.into()));
        }
        let m = Message::decode(b).ok()?;
        if m.class == Class::Indication {
            if m.method != method::DATA { return None; }
            let peer = m.xor_address(attr::XOR_PEER_ADDRESS)?;
            return Some((peer, m.get(attr::DATA)?.into()));
        }
        let i = self.pending.iter().position(|p| p.tid == m.transaction)?;
        // A reply that claims integrity must carry it under our key.
        if let Some(k) = self.key {
            if m.has(attr::MESSAGE_INTEGRITY) && !stun::check_integrity(b, &k) { return None; }
        }
        let p = self.pending.remove(i);
        if m.class == Class::Success { self.succeeded(now, p.kind, &m); } else { self.errored(now, p, &m); }
        None
    }

    fn succeeded(&mut self, now: u64, kind: Kind, m: &Message) {
        match kind {
            Kind::Allocate => {
                let Some(relayed) = m.xor_address(attr::XOR_RELAYED_ADDRESS) else {
                    return self.fail(400, "no relayed address");
                };
                let lifetime = m.get_u32(attr::LIFETIME).unwrap_or(600);
                self.relayed = Some(relayed);
                self.mapped = m.xor_address(attr::XOR_MAPPED_ADDRESS);
                self.state = State::Allocated;
                self.refresh_at = now + refresh_in(lifetime);
                self.events.push_back(Event::Allocated { relayed, mapped: self.mapped, lifetime });
                // What was asked for before the allocation goes out now.
                let perms: Vec<IpAddr> = self.permissions.iter().map(|p| p.ip).collect();
                for ip in perms { self.request(now, Kind::Permission(ip)); }
                let chans: Vec<(SocketAddr, u16)> = self.channels.iter().map(|c| (c.peer, c.number)).collect();
                for (p, n) in chans { self.request(now, Kind::Channel(p, n)); }
            }
            Kind::Refresh(0) => {}
            Kind::Refresh(_) => { self.refresh_at = now + refresh_in(m.get_u32(attr::LIFETIME).unwrap_or(600)); }
            Kind::Permission(ip) => {
                if let Some(p) = self.permissions.iter_mut().find(|p| p.ip == ip) {
                    if !p.ready { self.events.push_back(Event::Permitted(ip)); }
                    p.ready = true;
                    p.refresh_at = now + PERMISSION_REFRESH;
                }
            }
            Kind::Channel(peer, number) => {
                if let Some(c) = self.channels.iter_mut().find(|c| c.number == number) {
                    if !c.ready { self.events.push_back(Event::ChannelBound { peer, channel: number }); }
                    c.ready = true;
                    c.refresh_at = now + CHANNEL_REFRESH;
                }
                // A channel also installs a permission for its peer.
                match self.permissions.iter_mut().find(|p| p.ip == peer.ip()) {
                    Some(p) => { if !p.ready { self.events.push_back(Event::Permitted(peer.ip())); } p.ready = true; p.refresh_at = now + PERMISSION_REFRESH; }
                    None => {
                        self.permissions.push(Permission { ip: peer.ip(), refresh_at: now + PERMISSION_REFRESH, ready: true });
                        self.events.push_back(Event::Permitted(peer.ip()));
                    }
                }
            }
        }
    }

    fn errored(&mut self, now: u64, p: Pending, m: &Message) {
        let (c, reason) = m.error_code().unwrap_or((code::SERVER_ERROR, String::new()));
        if let Some(n) = m.get(attr::NONCE) { self.nonce = Some(n.into()); }
        match c {
            // The realm and nonce to sign with. A 401 to a signed retry means the
            // credentials are wrong.
            code::UNAUTHORIZED if p.stale_retries == 0 => {
                let Some(realm) = m.get_str(attr::REALM) else { return self.fail(c, "no realm") };
                self.key = Some(stun::long_term_key(&self.config.username, realm, &self.config.password));
                self.realm = Some(realm.into());
                self.resend(now, p.kind, p.stale_retries + 1);
            }
            code::STALE_NONCE if p.stale_retries < 3 => self.resend(now, p.kind, p.stale_retries + 1),
            // The allocation is gone (or this one was made twice): start over once.
            code::ALLOCATION_MISMATCH if matches!(p.kind, Kind::Refresh(_)) => {
                self.state = State::Idle;
                self.relayed = None;
                for q in self.permissions.iter_mut() { q.ready = false; }
                for q in self.channels.iter_mut() { q.ready = false; }
                self.start(now);
            }
            _ => match p.kind {
                Kind::Allocate | Kind::Refresh(1..) => self.fail(c, &reason),
                Kind::Refresh(0) => {}
                Kind::Permission(ip) => self.permissions.retain(|q| q.ip != ip),
                Kind::Channel(_, n) => self.channels.retain(|q| q.number != n),
            },
        }
    }

    fn fail(&mut self, code: u16, reason: &str) {
        if self.state == State::Failed { return; }
        self.state = State::Failed;
        self.pending.clear();
        self.events.push_back(Event::Failed { code, reason: reason.into() });
    }

    fn resend(&mut self, now: u64, kind: Kind, stale: u32) {
        self.request(now, kind);
        if let Some(p) = self.pending.last_mut() { p.stale_retries = stale; }
    }

    fn request(&mut self, now: u64, kind: Kind) {
        let tid = self.rng.transaction();
        let mut m = match kind {
            Kind::Allocate => {
                let mut m = Message::request(method::ALLOCATE, tid);
                // UDP relaying (17), whatever the transport to the server is.
                m.add(attr::REQUESTED_TRANSPORT, &[17, 0, 0, 0]);
                if self.config.server.is_ipv6() { m.add(attr::REQUESTED_ADDRESS_FAMILY, &[2, 0, 0, 0]); }
                m
            }
            Kind::Refresh(life) => { let mut m = Message::request(method::REFRESH, tid); m.add_u32(attr::LIFETIME, life); m }
            Kind::Permission(ip) => {
                let mut m = Message::request(method::CREATE_PERMISSION, tid);
                m.add_xor_address(attr::XOR_PEER_ADDRESS, SocketAddr::new(ip, 0));
                m
            }
            Kind::Channel(peer, n) => {
                let mut m = Message::request(method::CHANNEL_BIND, tid);
                m.add_u32(attr::CHANNEL_NUMBER, (n as u32) << 16).add_xor_address(attr::XOR_PEER_ADDRESS, peer);
                m
            }
        };
        m.add_str(attr::SOFTWARE, "SexOS sex-ice");
        let raw = match (self.key, &self.realm, &self.nonce) {
            (Some(k), Some(realm), Some(nonce)) => {
                m.add_str(attr::USERNAME, &self.config.username).add_str(attr::REALM, realm).add(attr::NONCE, nonce);
                m.encode(Some(&k), false)
            }
            _ => m.encode(None, false),
        };
        self.out.push_back(raw.clone());
        self.pending.push(Pending { tid, kind, raw, sent: now, rto: RTO, tries: 1, stale_retries: 0 });
    }

    /// Retransmissions (over UDP), refreshes and timeouts.
    pub fn handle_timeout(&mut self, now: u64) {
        let stream = self.config.transport.stream();
        let mut lost = None;
        for p in self.pending.iter_mut() {
            if stream {
                if now >= p.sent + STREAM_TIMEOUT { lost = Some(p.kind); }
            } else if now >= p.sent + p.rto {
                if p.tries >= TRIES { lost = Some(p.kind); continue; }
                p.tries += 1;
                p.sent = now;
                p.rto = (p.rto * 2).min(8_000);
                self.out.push_back(p.raw.clone());
            }
        }
        if let Some(k) = lost {
            self.pending.retain(|p| p.kind != k);
            match k {
                Kind::Allocate | Kind::Refresh(1..) => self.fail(0, "no answer from the TURN server"),
                Kind::Refresh(0) => {}
                Kind::Permission(ip) => self.permissions.retain(|q| q.ip != ip),
                Kind::Channel(_, n) => self.channels.retain(|q| q.number != n),
            }
        }
        if self.state != State::Allocated { return; }
        if now >= self.refresh_at && !self.pending.iter().any(|p| matches!(p.kind, Kind::Refresh(_))) {
            self.refresh_at = u64::MAX;
            self.request(now, Kind::Refresh(600));
        }
        let due: Vec<IpAddr> = self.permissions.iter_mut().filter(|p| p.ready && now >= p.refresh_at).map(|p| { p.refresh_at = u64::MAX; p.ip }).collect();
        for ip in due { self.request(now, Kind::Permission(ip)); }
        let due: Vec<(SocketAddr, u16)> = self.channels.iter_mut().filter(|c| c.ready && now >= c.refresh_at).map(|c| { c.refresh_at = u64::MAX; (c.peer, c.number) }).collect();
        for (p, n) in due { self.request(now, Kind::Channel(p, n)); }
    }

    /// When `handle_timeout` next has work.
    pub fn poll_timeout(&self) -> Option<u64> {
        let stream = self.config.transport.stream();
        let mut t = self.pending.iter().map(|p| if stream { p.sent + STREAM_TIMEOUT } else { p.sent + p.rto }).min();
        if self.state == State::Allocated {
            let refresh = self.permissions.iter().map(|p| p.refresh_at).chain(self.channels.iter().map(|c| c.refresh_at)).chain([self.refresh_at]).min();
            t = match (t, refresh) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) };
        }
        t.filter(|t| *t != u64::MAX)
    }
}

fn refresh_in(lifetime: u32) -> u64 { (lifetime as u64).saturating_sub(60).max(lifetime as u64 / 2) * 1000 }
