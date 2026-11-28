// SPDX-License-Identifier: GPL-3.0-only
//! An ICE agent (RFC 8445) for one component (WebRTC bundles everything and muxes RTCP,
//! so one is all a browser needs): host, server-reflexive and relayed candidates,
//! trickle both ways, connectivity checks paced at 50 ms, regular nomination, role
//! conflicts, and consent freshness (RFC 7675) once connected.
//!
//! No I/O. The program binds its UDP sockets, gives their addresses with `add_host`,
//! then feeds in what arrives (`receive_udp`, `receive_stream`) and the time
//! (`handle_timeout` at `poll_timeout`), and sends what `poll_transmit` hands out.
//! Datagrams that are not STUN (DTLS, RTP) come back from `receive_udp` as `Data`.

use crate::rand::Rng;
use crate::stun::{self, attr, code, method, Class, Message};
use crate::turn;
use alloc::borrow::Cow;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};
pub use sex_sdp::CandidateType as Kind;

/// Pacing between new checks (Ta).
pub const PACE: u64 = 50;
const CHECK_RTO: u64 = 250;
const CHECK_TRIES: u32 = 7;
const GATHER_RTO: u64 = 500;
const GATHER_TRIES: u32 = 5;
/// How long the controlling side waits for a better pair after the first one works.
const NOMINATION_WAIT: u64 = 500;
/// Consent (RFC 7675): a check every 4 to 6 s; no answer for 30 s ends the session.
const CONSENT_MIN: u64 = 4_000;
const CONSENT_SPREAD: u64 = 2_000;
const CONSENT_DISCONNECTED: u64 = 10_000;
const CONSENT_LOST: u64 = 30_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub kind: Kind,
    /// The address the peer sends to.
    pub address: SocketAddr,
    /// Where this side sends from: the host socket, or the relayed address itself.
    pub base: SocketAddr,
    /// raddr/rport: the base of a server-reflexive one, the mapped address of a relayed one.
    pub related: Option<SocketAddr>,
    pub priority: u32,
    pub foundation: String,
    pub component: u16,
}

impl Candidate {
    /// Its `a=candidate` form.
    pub fn to_sdp(&self) -> sex_sdp::Candidate {
        sex_sdp::Candidate {
            foundation: self.foundation.clone(), component: self.component, transport: "udp".into(), priority: self.priority,
            address: format!("{}", self.address.ip()), port: self.address.port(), kind: self.kind,
            related: self.related.map(|r| (format!("{}", r.ip()), r.port())), tcptype: None, extensions: Vec::new(),
        }
    }
    /// A remote candidate from the description or a trickled one. None for TCP
    /// candidates, other components, and mDNS `.local` names (which need a lookup first:
    /// resolve the name and set `address`).
    pub fn from_sdp(c: &sex_sdp::Candidate) -> Option<Candidate> {
        if c.transport != "udp" { return None; }
        let ip: IpAddr = c.address.parse().ok()?;
        let address = SocketAddr::new(ip, c.port);
        Some(Candidate {
            kind: c.kind, address, base: address, priority: c.priority, foundation: c.foundation.clone(), component: c.component,
            related: c.related.as_ref().and_then(|(a, p)| Some(SocketAddr::new(a.parse().ok()?, *p))),
        })
    }
}

pub fn type_preference(k: Kind) -> u32 {
    match k { Kind::Host => 126, Kind::PeerReflexive => 110, Kind::ServerReflexive => 100, Kind::Relay => 0 }
}
/// RFC 8445 5.1.2.1.
pub fn priority(k: Kind, local_preference: u16, component: u16) -> u32 {
    (type_preference(k) << 24) | ((local_preference as u32) << 8) | (256 - component.min(256) as u32)
}
/// RFC 8445 6.1.2.3: G is the controlling side's candidate priority.
pub fn pair_priority(g: u32, d: u32) -> u64 {
    let (g, d) = (g as u64, d as u64);
    (1 << 32) * g.min(d) + 2 * g.max(d) + (g > d) as u64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { New, Checking, Connected, Disconnected, Failed, Closed }

/// How the program sends a `Transmit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// From the host socket bound at `from`, to `to`.
    Udp { from: SocketAddr, to: SocketAddr },
    /// On the stream to TURN server number `server` (as `add_turn_server` numbered it).
    Stream { server: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transmit { pub route: Route, pub data: Vec<u8> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A local candidate to signal to the peer (trickle).
    Candidate(Candidate),
    /// Every local candidate has been found (an empty `a=end-of-candidates`).
    GatheringDone,
    State(State),
    /// The pair data goes over now: the local base (or relayed address) and the peer.
    Selected { local: SocketAddr, remote: SocketAddr },
    /// A TURN server could not give an allocation.
    TurnFailed { server: usize, code: u16, reason: String },
}

/// What `receive_udp` made of a datagram.
#[derive(Debug, PartialEq, Eq)]
pub enum Received<'a> {
    /// STUN or TURN traffic, handled here.
    Ice,
    /// Something else (DTLS, SRTP): for the next layer, with who sent it.
    Data { from: SocketAddr, data: Cow<'a, [u8]> },
}

#[derive(Clone, Debug)]
pub struct Config {
    pub controlling: bool,
    /// Entropy for credentials, tie-breaker and transaction ids.
    pub seed: [u8; 32],
    /// Only relayed candidates (RTCIceTransportPolicy "relay").
    pub relay_only: bool,
    /// The SOFTWARE attribute on checks; None leaves it out.
    pub software: Option<&'static str>,
}
impl Config {
    pub fn new(controlling: bool, seed: [u8; 32]) -> Self { Config { controlling, seed, relay_only: false, software: None } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PairState { Waiting, InProgress, Succeeded, Failed }

#[derive(Debug)]
struct Pair {
    local: usize,
    remote: usize,
    state: PairState,
    tid: [u8; 12],
    raw: Vec<u8>,
    sent: u64,
    rto: u64,
    tries: u32,
    /// This check carried USE-CANDIDATE.
    nominating: bool,
    nominated: bool,
    /// Controlled: the peer nominated it before our check succeeded.
    nominate_on_success: bool,
    triggered: bool,
    /// The role the last check went out with.
    as_controlling: bool,
}

#[derive(Debug)]
struct Gather { server: SocketAddr, base: SocketAddr, tid: [u8; 12], raw: Vec<u8>, sent: u64, rto: u64, tries: u32, done: bool }

#[derive(Debug)]
struct Relay { client: turn::Client, base: Option<SocketAddr>, done: bool, candidate: Option<usize> }

#[derive(Debug)]
pub struct Agent {
    rng: Rng,
    controlling: bool,
    tie_breaker: u64,
    relay_only: bool,
    software: Option<&'static str>,
    local_ufrag: String,
    local_pwd: String,
    remote_ufrag: Option<String>,
    remote_pwd: Option<String>,
    remote_lite: bool,
    hosts: Vec<SocketAddr>,
    stun_servers: Vec<SocketAddr>,
    locals: Vec<Candidate>,
    remotes: Vec<Candidate>,
    pairs: Vec<Pair>,
    gathers: Vec<Gather>,
    relays: Vec<Relay>,
    out: VecDeque<Transmit>,
    events: VecDeque<Event>,
    state: State,
    started: bool,
    gathering_done: bool,
    remote_done: bool,
    selected: Option<usize>,
    next_pace: u64,
    first_success: Option<u64>,
    consent_tids: VecDeque<[u8; 12]>,
    consent_ok: u64,
    next_consent: u64,
}

impl Agent {
    pub fn new(config: Config) -> Self {
        let mut rng = Rng::new(config.seed);
        let tie_breaker = rng.u64();
        let local_ufrag = rng.ice_chars(8);
        let local_pwd = rng.ice_chars(24);
        Agent {
            rng, controlling: config.controlling, tie_breaker, relay_only: config.relay_only, software: config.software,
            local_ufrag, local_pwd, remote_ufrag: None, remote_pwd: None, remote_lite: false,
            hosts: Vec::new(), stun_servers: Vec::new(), locals: Vec::new(), remotes: Vec::new(), pairs: Vec::new(),
            gathers: Vec::new(), relays: Vec::new(), out: VecDeque::new(), events: VecDeque::new(),
            state: State::New, started: false, gathering_done: false, remote_done: false, selected: None,
            next_pace: 0, first_success: None, consent_tids: VecDeque::new(), consent_ok: 0, next_consent: u64::MAX,
        }
    }

    /// This side's ice-ufrag and ice-pwd.
    pub fn local_credentials(&self) -> (&str, &str) { (&self.local_ufrag, &self.local_pwd) }
    pub fn set_remote_credentials(&mut self, ufrag: &str, pwd: &str) {
        self.remote_ufrag = Some(ufrag.into());
        self.remote_pwd = Some(pwd.into());
    }
    /// The peer is an ICE lite agent (`a=ice-lite`, as SFUs are): it never checks, so
    /// this side controls.
    pub fn set_remote_lite(&mut self, lite: bool) { self.remote_lite = lite; if lite { self.controlling = true; } }
    pub fn controlling(&self) -> bool { self.controlling }
    pub fn state(&self) -> State { self.state }
    pub fn local_candidates(&self) -> &[Candidate] { &self.locals }
    /// The selected pair: local base (or relayed address) and peer.
    pub fn selected(&self) -> Option<(SocketAddr, SocketAddr)> {
        let p = &self.pairs[self.selected?];
        Some((self.locals[p.local].base, self.remotes[p.remote].address))
    }

    /// A bound UDP socket's address (one per interface and family). One added after
    /// `start` is gathered at once.
    pub fn add_host(&mut self, now: u64, address: SocketAddr) {
        if self.hosts.contains(&address) { return; }
        self.hosts.push(address);
        if self.started { self.gather_host(now, address); self.check_gathering_done(); }
    }
    pub fn add_stun_server(&mut self, server: SocketAddr) { self.stun_servers.push(server); }
    /// Returns the server's number, which `Route::Stream` and `receive_stream` use.
    pub fn add_turn_server(&mut self, config: turn::Config) -> usize {
        let rng = self.rng.fork();
        self.relays.push(Relay { client: turn::Client::new(config, rng), base: None, done: false, candidate: None });
        self.relays.len() - 1
    }
    pub fn turn_server(&self, i: usize) -> Option<&turn::Config> { self.relays.get(i).map(|r| &r.client.config) }

    /// Starts gathering: host candidates at once, then server-reflexive and relayed ones
    /// as the servers answer.
    pub fn start(&mut self, now: u64) {
        if self.started { return; }
        self.started = true;
        self.next_pace = now;
        for h in self.hosts.clone() { self.gather_host(now, h); }
        for i in 0..self.relays.len() {
            let server = self.relays[i].client.server();
            if !self.relays[i].client.config.transport.stream() {
                // Over UDP the allocation is made from a host socket of the same family.
                match self.hosts.iter().find(|h| h.is_ipv4() == server.is_ipv4()) {
                    Some(h) => self.relays[i].base = Some(*h),
                    None => { self.relays[i].done = true; continue; }
                }
            }
            self.relays[i].client.start(now);
            self.flush_relay(i);
        }
        self.check_gathering_done();
    }

    fn gather_host(&mut self, now: u64, h: SocketAddr) {
        let lp = self.local_preference(h);
        if !self.relay_only {
            self.add_local(now, Kind::Host, h, h, None, lp, "");
        }
        for s in self.stun_servers.clone() {
            if s.is_ipv4() != h.is_ipv4() { continue; }
            let tid = self.rng.transaction();
            let mut m = Message::request(method::BINDING, tid);
            if let Some(sw) = self.software { m.add_str(attr::SOFTWARE, sw); }
            let raw = m.encode(None, true);
            self.out.push_back(Transmit { route: Route::Udp { from: h, to: s }, data: raw.clone() });
            self.gathers.push(Gather { server: s, base: h, tid, raw, sent: now, rto: GATHER_RTO, tries: 1, done: false });
        }
    }

    /// IPv6 before IPv4 (RFC 8421), then the order the program gave the sockets in.
    fn local_preference(&self, h: SocketAddr) -> u16 {
        let n = self.hosts.iter().position(|x| *x == h).unwrap_or(0).min(1000) as u16;
        if h.is_ipv6() { 65535 - n * 16 } else { 32767 - n * 16 }
    }

    fn add_local(&mut self, now: u64, kind: Kind, address: SocketAddr, base: SocketAddr, related: Option<SocketAddr>, lp: u16, server: &str) -> Option<usize> {
        if self.locals.iter().any(|c| c.address == address && c.kind == kind) { return None; }
        let c = Candidate {
            kind, address, base, related, priority: priority(kind, lp, 1),
            foundation: foundation(kind, base.ip(), server), component: 1,
        };
        self.events.push_back(Event::Candidate(c.clone()));
        self.locals.push(c);
        let li = self.locals.len() - 1;
        // A server-reflexive candidate is checked from its base, which is already paired.
        if kind != Kind::ServerReflexive {
            for ri in 0..self.remotes.len() { self.add_pair(now, li, ri); }
        }
        Some(li)
    }

    /// A candidate the peer signalled (trickled, or from its description).
    pub fn add_remote_candidate(&mut self, now: u64, c: Candidate) {
        if c.component != 1 { return; }
        if let Some(r) = self.remotes.iter_mut().find(|r| r.address == c.address) {
            // A peer-reflexive one learned from a check takes the signalled type.
            if r.kind == Kind::PeerReflexive { r.kind = c.kind; r.foundation = c.foundation; }
            return;
        }
        self.remotes.push(c);
        let ri = self.remotes.len() - 1;
        for li in 0..self.locals.len() {
            if self.locals[li].kind != Kind::ServerReflexive { self.add_pair(now, li, ri); }
        }
        if self.state == State::New && self.started { self.set_state(State::Checking); }
    }
    /// A trickled `candidate:...` string. False when it was not usable here.
    pub fn add_remote_candidate_line(&mut self, now: u64, line: &str) -> bool {
        match sex_sdp::Candidate::parse_line(line).as_ref().and_then(Candidate::from_sdp) {
            Some(c) => { self.add_remote_candidate(now, c); true }
            None => false,
        }
    }
    /// The peer has sent all its candidates.
    pub fn end_of_remote_candidates(&mut self) { self.remote_done = true; }

    fn add_pair(&mut self, now: u64, li: usize, ri: usize) -> Option<usize> {
        let (l, r) = (&self.locals[li], &self.remotes[ri]);
        if l.base.is_ipv4() != r.address.is_ipv4() { return None; }
        if let Some(i) = self.pairs.iter().position(|p| p.local == li && p.remote == ri) { return Some(i); }
        if l.kind == Kind::Relay {
            let ip = r.address.ip();
            if let Some(ri) = self.relays.iter().position(|x| x.candidate == Some(li)) {
                self.relays[ri].client.permit(now, ip);
                self.flush_relay(ri);
            }
        }
        self.pairs.push(Pair {
            local: li, remote: ri, state: PairState::Waiting, tid: [0; 12], raw: Vec::new(), sent: 0, rto: CHECK_RTO, tries: 0,
            nominating: false, nominated: false, nominate_on_success: false, triggered: false, as_controlling: false,
        });
        Some(self.pairs.len() - 1)
    }

    fn pair_prio(&self, p: &Pair) -> u64 {
        let (l, r) = (self.locals[p.local].priority, self.remotes[p.remote].priority);
        if self.controlling { pair_priority(l, r) } else { pair_priority(r, l) }
    }

    fn set_state(&mut self, s: State) {
        if self.state != s { self.state = s; self.events.push_back(Event::State(s)); }
    }

    pub fn poll_transmit(&mut self) -> Option<Transmit> { self.out.pop_front() }
    pub fn poll_event(&mut self) -> Option<Event> { self.events.pop_front() }

    /// Sends application data (DTLS) on the selected pair. None before one is selected.
    pub fn send(&mut self, data: &[u8]) -> Option<Transmit> {
        let p = &self.pairs[self.selected?];
        let (li, to) = (p.local, self.remotes[p.remote].address);
        if self.locals[li].kind != Kind::Relay {
            return Some(Transmit { route: Route::Udp { from: self.locals[li].base, to }, data: data.into() });
        }
        let ri = self.relays.iter().position(|r| r.candidate == Some(li))?;
        if !self.relays[ri].client.send_to(to, data) { return None; }
        let bytes = self.relays[ri].client.poll_transmit()?;
        Some(Transmit { route: self.relay_route(ri), data: bytes })
    }

    fn relay_route(&self, ri: usize) -> Route {
        match self.relays[ri].base {
            Some(from) => Route::Udp { from, to: self.relays[ri].client.server() },
            None => Route::Stream { server: ri },
        }
    }

    /// A datagram that reached host socket `local` from `from`.
    pub fn receive_udp<'a>(&mut self, now: u64, local: SocketAddr, from: SocketAddr, data: &'a [u8]) -> Received<'a> {
        if let Some(ri) = self.relays.iter().position(|r| r.base == Some(local) && r.client.server() == from) {
            let got = self.relays[ri].client.handle(now, data);
            self.after_relay(now, ri);
            return match got {
                Some((peer, payload)) => match self.relay_payload(now, ri, peer, payload) {
                    Some((f, d)) => Received::Data { from: f, data: Cow::Owned(d) },
                    None => Received::Ice,
                },
                None => Received::Ice,
            };
        }
        if stun::is_stun(data) {
            // An answer from a STUN server, by its transaction; else ICE on a host candidate.
            let tid = &data[8..20];
            if self.gathers.iter().any(|g| g.base == local && g.server == from && g.tid == tid) {
                self.handle_gather(now, local, from, data);
            } else if let Some(li) = self.locals.iter().position(|c| c.base == local && c.kind == Kind::Host) {
                self.handle_stun(now, li, from, data);
            }
            return Received::Ice;
        }
        Received::Data { from, data: Cow::Borrowed(data) }
    }

    /// Bytes read from the TCP or TLS stream to TURN server `server`. Returns the
    /// application data peers sent through it.
    pub fn receive_stream(&mut self, now: u64, server: usize, bytes: &[u8]) -> Vec<(SocketAddr, Vec<u8>)> {
        let Some(r) = self.relays.get_mut(server) else { return Vec::new() };
        let got = r.client.handle_stream(now, bytes);
        self.after_relay(now, server);
        got.into_iter().filter_map(|(peer, d)| self.relay_payload(now, server, peer, d)).collect()
    }

    fn relay_payload(&mut self, now: u64, ri: usize, peer: SocketAddr, d: Vec<u8>) -> Option<(SocketAddr, Vec<u8>)> {
        if stun::is_stun(&d) {
            if let Some(li) = self.relays[ri].candidate { self.handle_stun(now, li, peer, &d); }
            return None;
        }
        Some((peer, d))
    }

    fn flush_relay(&mut self, ri: usize) {
        let route = self.relay_route(ri);
        while let Some(b) = self.relays[ri].client.poll_transmit() { self.out.push_back(Transmit { route, data: b }); }
    }

    fn after_relay(&mut self, now: u64, ri: usize) {
        while let Some(e) = self.relays[ri].client.poll_event() {
            match e {
                turn::Event::Allocated { relayed, mapped, .. } => {
                    self.relays[ri].done = true;
                    let server = format!("{}", self.relays[ri].client.server());
                    let li = self.add_local(now, Kind::Relay, relayed, relayed, mapped, 65535 - ri as u16, &server);
                    self.relays[ri].candidate = li;
                    // Pairs made by add_local asked for permissions before the
                    // candidate was known: ask now.
                    if let Some(li) = li {
                        let ips: Vec<IpAddr> = self.pairs.iter().filter(|p| p.local == li).map(|p| self.remotes[p.remote].address.ip()).collect();
                        for ip in ips { self.relays[ri].client.permit(now, ip); }
                    }
                    self.check_gathering_done();
                }
                turn::Event::Failed { code, reason } => {
                    self.relays[ri].done = true;
                    self.events.push_back(Event::TurnFailed { server: ri, code, reason });
                    self.check_gathering_done();
                }
                turn::Event::Permitted(_) | turn::Event::ChannelBound { .. } => {}
            }
        }
        self.flush_relay(ri);
    }

    fn check_gathering_done(&mut self) {
        if self.gathering_done || !self.started { return; }
        if self.gathers.iter().all(|g| g.done) && self.relays.iter().all(|r| r.done) {
            self.gathering_done = true;
            self.events.push_back(Event::GatheringDone);
        }
    }

    fn handle_gather(&mut self, now: u64, local: SocketAddr, from: SocketAddr, data: &[u8]) {
        let Ok(m) = Message::decode(data) else { return };
        let Some(gi) = self.gathers.iter().position(|g| g.tid == m.transaction && g.base == local && g.server == from && !g.done) else { return };
        self.gathers[gi].done = true;
        if m.class == Class::Success && !self.relay_only {
            if let Some(mapped) = m.mapped() {
                if mapped != local {
                    let lp = self.local_preference(local);
                    self.add_local(now, Kind::ServerReflexive, mapped, local, Some(local), lp, &format!("{}", from));
                }
            }
        }
        self.check_gathering_done();
    }

    /// Sends `data` from local candidate `li` to `to`.
    fn send_from(&mut self, li: usize, to: SocketAddr, data: Vec<u8>) {
        if self.locals[li].kind != Kind::Relay {
            self.out.push_back(Transmit { route: Route::Udp { from: self.locals[li].base, to }, data });
            return;
        }
        if let Some(ri) = self.relays.iter().position(|r| r.candidate == Some(li)) {
            self.relays[ri].client.send_to(to, &data);
            self.flush_relay(ri);
        }
    }

    fn handle_stun(&mut self, now: u64, li: usize, from: SocketAddr, raw: &[u8]) {
        let Ok(m) = Message::decode(raw) else { return };
        if m.method != method::BINDING { return; }
        if stun::check_fingerprint(raw) == Some(false) { return; }
        match m.class {
            Class::Request => self.handle_request(now, li, from, raw, &m),
            Class::Success | Class::Error => self.handle_response(now, li, from, raw, &m),
            Class::Indication => {}
        }
    }

    fn reply_error(&mut self, li: usize, to: SocketAddr, m: &Message, code: u16, reason: &str, sign: bool) {
        let mut r = m.reply(false);
        r.add_error(code, reason);
        let key = self.local_pwd.clone();
        let bytes = r.encode(if sign { Some(key.as_bytes()) } else { None }, true);
        self.send_from(li, to, bytes);
    }

    fn handle_request(&mut self, now: u64, li: usize, from: SocketAddr, raw: &[u8], m: &Message) {
        // USERNAME is "our ufrag:their ufrag", signed with our password.
        let user_ok = m.get_str(attr::USERNAME).and_then(|u| u.split_once(':')).is_some_and(|(ours, theirs)| {
            ours == self.local_ufrag && self.remote_ufrag.as_deref().map_or(true, |r| r == theirs)
        });
        if !m.has(attr::MESSAGE_INTEGRITY) || !m.has(attr::USERNAME) { return self.reply_error(li, from, m, code::BAD_REQUEST, "Bad Request", false); }
        if !user_ok || !stun::check_integrity(raw, self.local_pwd.as_bytes()) {
            return self.reply_error(li, from, m, code::UNAUTHORIZED, "Unauthorized", false);
        }
        let unknown = m.unknown_required();
        if !unknown.is_empty() {
            let mut r = m.reply(false);
            r.add_error(code::UNKNOWN_ATTRIBUTE, "Unknown Attribute");
            let v: Vec<u8> = unknown.iter().flat_map(|t| t.to_be_bytes()).collect();
            r.add(attr::UNKNOWN_ATTRIBUTES, &v);
            let bytes = r.encode(Some(self.local_pwd.clone().as_bytes()), true);
            return self.send_from(li, from, bytes);
        }
        // Role conflicts (RFC 8445 7.3.1.1).
        if let Some(theirs) = m.get_u64(if self.controlling { attr::ICE_CONTROLLING } else { attr::ICE_CONTROLLED }) {
            let we_win = self.tie_breaker >= theirs;
            if self.controlling == we_win {
                return self.reply_error(li, from, m, code::ROLE_CONFLICT, "Role Conflict", true);
            }
            self.switch_role();
        }
        let mut r = m.reply(true);
        r.add_xor_address(attr::XOR_MAPPED_ADDRESS, from);
        let bytes = r.encode(Some(self.local_pwd.clone().as_bytes()), true);
        self.send_from(li, from, bytes);

        // The sender is a remote candidate; one not signalled yet is peer-reflexive.
        let ri = match self.remotes.iter().position(|r| r.address == from) {
            Some(i) => i,
            None => {
                let prio = m.get_u32(attr::PRIORITY).unwrap_or_else(|| priority(Kind::PeerReflexive, 65535, 1));
                self.remotes.push(Candidate {
                    kind: Kind::PeerReflexive, address: from, base: from, related: None, priority: prio,
                    foundation: format!("{}", self.remotes.len() + 100), component: 1,
                });
                self.remotes.len() - 1
            }
        };
        let Some(pi) = self.add_pair(now, li, ri) else { return };
        if self.state == State::New { self.set_state(State::Checking); }
        let use_candidate = m.has(attr::USE_CANDIDATE) && !self.controlling;
        let p = &mut self.pairs[pi];
        match p.state {
            PairState::Succeeded => {
                if use_candidate { p.nominated = true; self.select(now, pi); }
            }
            PairState::InProgress => { if use_candidate { p.nominate_on_success = true; } }
            PairState::Waiting | PairState::Failed => {
                if use_candidate { p.nominate_on_success = true; }
                p.state = PairState::Waiting;
                p.triggered = true;
                p.tries = 0;
            }
        }
    }

    fn switch_role(&mut self) {
        self.controlling = !self.controlling;
        // Priorities flip with the role; pending checks carry the old role, so redo them.
        for p in self.pairs.iter_mut() {
            if p.state == PairState::InProgress { p.state = PairState::Waiting; p.tries = 0; }
        }
    }

    fn handle_response(&mut self, now: u64, li: usize, from: SocketAddr, raw: &[u8], m: &Message) {
        let Some(pwd) = self.remote_pwd.clone() else { return };
        if !stun::check_integrity(raw, pwd.as_bytes()) { return; }
        if let Some(i) = self.consent_tids.iter().position(|t| *t == m.transaction) {
            self.consent_tids.remove(i);
            if m.class == Class::Success {
                self.consent_ok = now;
                if self.state == State::Disconnected { self.set_state(State::Connected); }
            }
            return;
        }
        let Some(pi) = self.pairs.iter().position(|p| p.tid == m.transaction && p.state == PairState::InProgress) else { return };
        // Answers must come back on the same pair (symmetry).
        if self.pairs[pi].local != li || self.remotes[self.pairs[pi].remote].address != from {
            self.pairs[pi].state = PairState::Failed;
            return;
        }
        if m.class == Class::Error {
            match m.error_code().map(|c| c.0) {
                Some(code::ROLE_CONFLICT) => {
                    // The role the check carried lost: take the other and check again.
                    if self.pairs[pi].as_controlling == self.controlling { self.switch_role(); }
                    let p = &mut self.pairs[pi];
                    p.state = PairState::Waiting;
                    p.triggered = true;
                    p.tries = 0;
                }
                _ => self.pairs[pi].state = PairState::Failed,
            }
            return;
        }
        let p = &mut self.pairs[pi];
        p.state = PairState::Succeeded;
        if self.first_success.is_none() { self.first_success = Some(now); }
        let nominated = (p.nominating && self.controlling) || (p.nominate_on_success && !self.controlling);
        if nominated { p.nominated = true; self.select(now, pi); }
    }

    fn select(&mut self, now: u64, pi: usize) {
        let better = match self.selected { Some(s) => self.pair_prio(&self.pairs[pi]) > self.pair_prio(&self.pairs[s]) || s == pi, None => true };
        if !better { return; }
        let changed = self.selected != Some(pi);
        self.selected = Some(pi);
        if changed {
            let (l, r) = (self.locals[self.pairs[pi].local].base, self.remotes[self.pairs[pi].remote].address);
            self.events.push_back(Event::Selected { local: l, remote: r });
            // Relayed data goes in ChannelData from now on.
            if let Some(ri) = self.relays.iter().position(|x| x.candidate == Some(self.pairs[pi].local)) {
                self.relays[ri].client.bind_channel(now, r);
                self.flush_relay(ri);
            }
        }
        self.consent_ok = now;
        if self.next_consent == u64::MAX { self.next_consent = now + CONSENT_MIN + self.rng.below(CONSENT_SPREAD); }
        self.set_state(State::Connected);
    }

    fn check_request(&mut self, pi: usize, nominate: bool) -> ([u8; 12], Vec<u8>) {
        let tid = self.rng.transaction();
        let (Some(ru), Some(rp)) = (self.remote_ufrag.clone(), self.remote_pwd.clone()) else { return (tid, Vec::new()) };
        let mut m = Message::request(method::BINDING, tid);
        m.add_str(attr::USERNAME, &format!("{}:{}", ru, self.local_ufrag));
        if let Some(sw) = self.software { m.add_str(attr::SOFTWARE, sw); }
        let lp = (self.locals[self.pairs[pi].local].priority >> 8) & 0xFFFF;
        m.add_u32(attr::PRIORITY, priority(Kind::PeerReflexive, lp as u16, 1));
        if self.controlling {
            m.add_u64(attr::ICE_CONTROLLING, self.tie_breaker);
            if nominate { m.add_flag(attr::USE_CANDIDATE); }
        } else {
            m.add_u64(attr::ICE_CONTROLLED, self.tie_breaker);
        }
        (tid, m.encode(Some(rp.as_bytes()), true))
    }

    fn start_check(&mut self, now: u64, pi: usize, nominate: bool) {
        let (tid, raw) = self.check_request(pi, nominate);
        if raw.is_empty() { return; }
        let p = &mut self.pairs[pi];
        p.tid = tid;
        p.raw = raw.clone();
        p.state = PairState::InProgress;
        p.sent = now;
        p.rto = CHECK_RTO;
        p.tries = 1;
        p.triggered = false;
        p.nominating = nominate;
        p.as_controlling = self.controlling;
        let (li, to) = (p.local, self.remotes[p.remote].address);
        self.send_from(li, to, raw);
    }

    /// Whether a pair may be checked now: relayed ones wait for their permission.
    fn ready(&self, p: &Pair) -> bool {
        if self.locals[p.local].kind != Kind::Relay { return true; }
        let ip = self.remotes[p.remote].address.ip();
        self.relays.iter().any(|r| r.candidate == Some(p.local) && r.client.has_permission(ip))
    }

    pub fn handle_timeout(&mut self, now: u64) {
        if !self.started || matches!(self.state, State::Failed | State::Closed) { return; }
        // Gathering retransmissions.
        for g in self.gathers.iter_mut().filter(|g| !g.done) {
            if now >= g.sent + g.rto {
                if g.tries >= GATHER_TRIES { g.done = true; continue; }
                g.tries += 1;
                g.sent = now;
                g.rto *= 2;
                self.out.push_back(Transmit { route: Route::Udp { from: g.base, to: g.server }, data: g.raw.clone() });
            }
        }
        for ri in 0..self.relays.len() {
            self.relays[ri].client.handle_timeout(now);
            self.after_relay(now, ri);
        }
        self.check_gathering_done();

        // Check retransmissions.
        let mut resend = Vec::new();
        for (i, p) in self.pairs.iter_mut().enumerate() {
            if p.state == PairState::InProgress && now >= p.sent + p.rto {
                if p.tries >= CHECK_TRIES { p.state = PairState::Failed; continue; }
                p.tries += 1;
                p.sent = now;
                p.rto = (p.rto * 2).min(3_000);
                resend.push(i);
            }
        }
        for i in resend {
            let (li, to, raw) = (self.pairs[i].local, self.remotes[self.pairs[i].remote].address, self.pairs[i].raw.clone());
            self.send_from(li, to, raw);
        }

        // One new check per Ta: triggered first, then the best waiting pair.
        if now >= self.next_pace && self.remote_pwd.is_some() {
            let pick = self.pairs.iter().enumerate()
                .filter(|(_, p)| p.state == PairState::Waiting && self.ready(p))
                .max_by_key(|(_, p)| (p.triggered, self.pair_prio(p)))
                .map(|(i, _)| i);
            if let Some(i) = pick {
                // Against an ICE lite peer the first working pair is as good as any:
                // nominate on the first check (it only answers, and keeps the last
                // nominated pair).
                let nominate = self.controlling && self.selected.is_none() && self.remote_lite;
                self.start_check(now, i, nominate);
                self.next_pace = now + PACE;
            }
        }

        // Regular nomination by the controlling side.
        if self.controlling && self.selected.is_none() && !self.pairs.iter().any(|p| p.nominating && p.state == PairState::InProgress) {
            if let Some(first) = self.first_success {
                let best = self.pairs.iter().enumerate().filter(|(_, p)| p.state == PairState::Succeeded).max_by_key(|(_, p)| self.pair_prio(p)).map(|(i, _)| i);
                if let Some(b) = best {
                    let bp = self.pair_prio(&self.pairs[b]);
                    let better_pending = self.pairs.iter().any(|p| matches!(p.state, PairState::Waiting | PairState::InProgress) && self.pair_prio(p) > bp);
                    if !better_pending || now >= first + NOMINATION_WAIT { self.start_check(now, b, true); }
                }
            }
        }

        // Consent freshness.
        if let Some(s) = self.selected {
            if now >= self.next_consent {
                let (tid, raw) = self.check_request(s, false);
                if !raw.is_empty() {
                    self.consent_tids.push_back(tid);
                    if self.consent_tids.len() > 8 { self.consent_tids.pop_front(); }
                    let (li, to) = (self.pairs[s].local, self.remotes[self.pairs[s].remote].address);
                    self.send_from(li, to, raw);
                }
                self.next_consent = now + CONSENT_MIN + self.rng.below(CONSENT_SPREAD);
            }
            let quiet = now.saturating_sub(self.consent_ok);
            if quiet >= CONSENT_LOST { self.set_state(State::Failed); return; }
            if quiet >= CONSENT_DISCONNECTED && self.state == State::Connected { self.set_state(State::Disconnected); }
        } else if self.remote_done && self.gathering_done && !self.pairs.is_empty()
            && self.pairs.iter().all(|p| p.state == PairState::Failed) {
            self.set_state(State::Failed);
        }
    }

    /// When `handle_timeout` next has work.
    pub fn poll_timeout(&self) -> Option<u64> {
        if !self.started || matches!(self.state, State::Failed | State::Closed) { return None; }
        let mut t: Option<u64> = None;
        let mut at = |x: u64| t = Some(t.map_or(x, |y: u64| y.min(x)));
        for g in self.gathers.iter().filter(|g| !g.done) { at(g.sent + g.rto); }
        for r in &self.relays { if let Some(x) = r.client.poll_timeout() { at(x); } }
        for p in self.pairs.iter() {
            if p.state == PairState::InProgress { at(p.sent + p.rto); }
            if p.state == PairState::Waiting && self.remote_pwd.is_some() { at(self.next_pace); }
        }
        if self.controlling && self.selected.is_none() {
            if let Some(f) = self.first_success { at(f + NOMINATION_WAIT); at(self.next_pace); }
        }
        if self.selected.is_some() {
            at(self.next_consent);
            at(self.consent_ok + if self.state == State::Connected { CONSENT_DISCONNECTED } else { CONSENT_LOST });
        }
        t
    }

    /// Stops: gives TURN allocations back.
    pub fn close(&mut self, now: u64) {
        for ri in 0..self.relays.len() { self.relays[ri].client.close(now); self.flush_relay(ri); }
        self.set_state(State::Closed);
    }
}

/// The same for candidates of one type, base address and server (RFC 8445 5.1.1.3).
fn foundation(kind: Kind, base: IpAddr, server: &str) -> String {
    let mut h: u32 = 0x811C_9DC5;
    let mut eat = |b: &[u8]| for x in b { h = (h ^ *x as u32).wrapping_mul(0x0100_0193); };
    eat(kind.as_str().as_bytes());
    match base { IpAddr::V4(a) => eat(&a.octets()), IpAddr::V6(a) => eat(&a.octets()) }
    eat(server.as_bytes());
    format!("{}", h)
}
