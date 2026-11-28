// SPDX-License-Identifier: GPL-3.0-only
//! ICE agents on an in-memory network: two agents to connected, one behind a NAT that
//! finds itself through a fake STUN server, relayed through a fake TURN server over UDP
//! and over a TCP stream, a role conflict, an ICE lite peer, and consent running out.

use sex_ice::stun::{self, attr, code, method, Class, Framer, Message};
use sex_ice::{turn, Agent, Config, Event, Kind, Received, Route, State};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, SocketAddr};

fn a(s: &str) -> SocketAddr { s.parse().unwrap() }

const REALM: &str = "sexos.test";
const USER: &str = "alice";
const PASS: &str = "secret";

/// A TURN server as RFC 8656 has it, small: one realm, one user, a fixed nonce.
struct FakeTurn {
    addr: SocketAddr,
    next_port: u16,
    /// By the client's transport address (or "stream:N").
    allocs: HashMap<String, Alloc>,
    nonce: String,
    stale_once: bool,
}
struct Alloc { client: Client, relayed: SocketAddr, perms: HashSet<IpAddr>, channels: HashMap<u16, SocketAddr>, lifetime: u32 }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Client { Udp(SocketAddr), Stream(usize) }
/// What the server sends: to a client or, from a relayed address, to a peer.
enum Out { Udp { from: SocketAddr, to: SocketAddr, data: Vec<u8> }, Stream { agent: usize, data: Vec<u8> } }

impl FakeTurn {
    fn new(addr: SocketAddr) -> Self { FakeTurn { addr, next_port: 50000, allocs: HashMap::new(), nonce: "n0".into(), stale_once: true } }
    fn key() -> [u8; 16] { stun::long_term_key(USER, REALM, PASS) }
    fn send(&self, c: Client, data: Vec<u8>, out: &mut Vec<Out>) {
        match c { Client::Udp(to) => out.push(Out::Udp { from: self.addr, to, data }), Client::Stream(agent) => out.push(Out::Stream { agent, data }) }
    }
    fn from_client(&mut self, c: Client, b: &[u8], out: &mut Vec<Out>) {
        let id = format!("{:?}", c);
        if let Some((ch, data)) = stun::read_channel_data(b) {
            let al = &self.allocs[&id];
            if let Some(peer) = al.channels.get(&ch) { out.push(Out::Udp { from: al.relayed, to: *peer, data: data.into() }); }
            return;
        }
        let m = Message::decode(b).unwrap();
        if m.class == Class::Indication && m.method == method::SEND {
            let al = &self.allocs[&id];
            let peer = m.xor_address(attr::XOR_PEER_ADDRESS).unwrap();
            if al.perms.contains(&peer.ip()) { out.push(Out::Udp { from: al.relayed, to: peer, data: m.get(attr::DATA).unwrap().into() }); }
            return;
        }
        assert_eq!(m.class, Class::Request);
        let mut r = m.reply(false);
        if !m.has(attr::MESSAGE_INTEGRITY) {
            r.add_error(code::UNAUTHORIZED, "Unauthorized").add_str(attr::REALM, REALM).add_str(attr::NONCE, &self.nonce);
            return self.send(c, r.encode(None, false), out);
        }
        assert_eq!(m.get_str(attr::USERNAME), Some(USER));
        assert_eq!(m.get_str(attr::REALM), Some(REALM));
        if !stun::check_integrity(b, &Self::key()) {
            r.add_error(code::UNAUTHORIZED, "Unauthorized").add_str(attr::REALM, REALM).add_str(attr::NONCE, &self.nonce);
            return self.send(c, r.encode(None, false), out);
        }
        // Once, on the first signed request after the allocation, say the nonce is stale.
        if m.method != method::ALLOCATE && self.stale_once {
            self.stale_once = false;
            self.nonce = "n1".into();
            r.add_error(code::STALE_NONCE, "Stale Nonce").add_str(attr::REALM, REALM).add_str(attr::NONCE, &self.nonce);
            return self.send(c, r.encode(None, false), out);
        }
        assert_eq!(m.get_str(attr::NONCE), Some(self.nonce.as_str()));
        let mut ok = m.reply(true);
        match m.method {
            method::ALLOCATE => {
                assert_eq!(m.get(attr::REQUESTED_TRANSPORT), Some(&[17u8, 0, 0, 0][..]));
                let relayed = SocketAddr::new(self.addr.ip(), self.next_port);
                self.next_port += 1;
                self.allocs.insert(id.clone(), Alloc { client: c, relayed, perms: HashSet::new(), channels: HashMap::new(), lifetime: 600 });
                ok.add_xor_address(attr::XOR_RELAYED_ADDRESS, relayed).add_u32(attr::LIFETIME, 600);
                if let Client::Udp(from) = c { ok.add_xor_address(attr::XOR_MAPPED_ADDRESS, from); }
            }
            method::CREATE_PERMISSION => {
                let peer = m.xor_address(attr::XOR_PEER_ADDRESS).unwrap();
                self.allocs.get_mut(&id).unwrap().perms.insert(peer.ip());
            }
            method::CHANNEL_BIND => {
                let peer = m.xor_address(attr::XOR_PEER_ADDRESS).unwrap();
                let ch = (m.get_u32(attr::CHANNEL_NUMBER).unwrap() >> 16) as u16;
                let al = self.allocs.get_mut(&id).unwrap();
                al.channels.insert(ch, peer);
                al.perms.insert(peer.ip());
            }
            method::REFRESH => {
                let l = m.get_u32(attr::LIFETIME).unwrap();
                if l == 0 { self.allocs.remove(&id); } else { self.allocs.get_mut(&id).unwrap().lifetime = l; }
                ok.add_u32(attr::LIFETIME, l);
            }
            _ => panic!("method {}", m.method),
        }
        self.send(c, ok.encode(Some(&Self::key()), false), out);
    }
    /// A datagram from a peer to a relayed address.
    fn to_relayed(&self, from: SocketAddr, to: SocketAddr, data: &[u8], out: &mut Vec<Out>) -> bool {
        let Some(al) = self.allocs.values().find(|al| al.relayed == to) else { return false };
        if !al.perms.contains(&from.ip()) { return true; }
        let stream = matches!(al.client, Client::Stream(_));
        let bytes = match al.channels.iter().find(|(_, p)| **p == from) {
            Some((ch, _)) => stun::channel_data(*ch, data, stream),
            None => {
                let mut m = Message::new(Class::Indication, method::DATA, [9; 12]);
                m.add_xor_address(attr::XOR_PEER_ADDRESS, from).add(attr::DATA, data);
                m.encode(None, false)
            }
        };
        self.send(al.client, bytes, out);
        true
    }
}

struct Sim {
    now: u64,
    agents: Vec<Agent>,
    /// Private address -> public address (an endpoint-independent NAT).
    nat: Vec<(SocketAddr, SocketAddr)>,
    stun: Option<SocketAddr>,
    turn: Option<FakeTurn>,
    framers: HashMap<usize, Framer>,
    wire: VecDeque<(SocketAddr, SocketAddr, Vec<u8>)>,
    /// Signal candidates to the other agent.
    trickle: bool,
    /// Datagrams to these are dropped.
    blackhole: HashSet<SocketAddr>,
    /// Application data each agent got.
    got: Vec<Vec<(SocketAddr, Vec<u8>)>>,
    events: Vec<Vec<Event>>,
    stun_hits: usize,
    hosts: Vec<Vec<SocketAddr>>,
}

impl Sim {
    fn new(agents: Vec<Agent>) -> Self {
        let n = agents.len();
        Sim {
            now: 1000, agents, nat: Vec::new(), stun: None, turn: None, framers: HashMap::new(), wire: VecDeque::new(), trickle: true,
            blackhole: HashSet::new(), got: vec![Vec::new(); n], events: vec![Vec::new(); n], stun_hits: 0, hosts: Vec::new(),
        }
    }
    fn exchange_credentials(&mut self) {
        let (u0, p0) = { let c = self.agents[0].local_credentials(); (c.0.to_string(), c.1.to_string()) };
        let (u1, p1) = { let c = self.agents[1].local_credentials(); (c.0.to_string(), c.1.to_string()) };
        self.agents[0].set_remote_credentials(&u1, &p1);
        self.agents[1].set_remote_credentials(&u0, &p0);
    }
    fn public(&self, from: SocketAddr) -> SocketAddr { self.nat.iter().find(|(p, _)| *p == from).map_or(from, |x| x.1) }
    fn private(&self, to: SocketAddr) -> SocketAddr { self.nat.iter().find(|(_, p)| *p == to).map_or(to, |x| x.0) }
    fn owner(&self, to: SocketAddr) -> Option<usize> {
        self.agents.iter().position(|ag| ag.local_candidates().iter().any(|c| c.kind == Kind::Host && c.base == to))
            .or_else(|| self.agents.iter().position(|ag| ag.local_candidates().iter().any(|c| c.base == to)))
            .or_else(|| self.hosts.iter().position(|h| h.contains(&to)))
    }
    fn step(&mut self) {
        for i in 0..self.agents.len() {
            while let Some(t) = self.agents[i].poll_transmit() {
                match t.route {
                    Route::Udp { from, to } => { let f = self.public(from); self.wire.push_back((f, to, t.data)); }
                    Route::Stream { .. } => {
                        let f = self.framers.entry(i).or_default();
                        f.push(&t.data);
                        let mut msgs = Vec::new();
                        while let Some(m) = f.next() { msgs.push(m); }
                        let mut out = Vec::new();
                        for m in msgs { self.turn.as_mut().unwrap().from_client(Client::Stream(i), &m, &mut out); }
                        self.emit(out);
                    }
                }
            }
            while let Some(e) = self.agents[i].poll_event() {
                if let (true, Event::Candidate(c)) = (self.trickle, &e) {
                    let other = 1 - i;
                    let line = c.to_sdp().to_line();
                    assert!(self.agents[other].add_remote_candidate_line(self.now, &line), "{}", line);
                }
                if e == Event::GatheringDone && self.trickle { self.agents[1 - i].end_of_remote_candidates(); }
                self.events[i].push(e);
            }
        }
        let mut wire = std::mem::take(&mut self.wire);
        while let Some((from, to, data)) = wire.pop_front() {
            if self.blackhole.contains(&to) || self.blackhole.contains(&from) { continue; }
            if Some(to) == self.stun {
                self.stun_hits += 1;
                let m = Message::decode(&data).unwrap();
                assert_eq!((m.class, m.method), (Class::Request, method::BINDING));
                assert_eq!(stun::check_fingerprint(&data), Some(true));
                let mut r = m.reply(true);
                r.add_xor_address(attr::XOR_MAPPED_ADDRESS, from);
                self.wire.push_back((to, from, r.encode(None, true)));
                continue;
            }
            if let Some(t) = &mut self.turn {
                let mut out = Vec::new();
                if to == t.addr { t.from_client(Client::Udp(from), &data, &mut out); self.emit(out); continue; }
                if t.to_relayed(from, to, &data, &mut out) { self.emit(out); continue; }
            }
            let local = self.private(to);
            let Some(i) = self.owner(local) else { continue };
            if let Received::Data { from, data } = self.agents[i].receive_udp(self.now, local, from, &data) {
                self.got[i].push((from, data.into_owned()));
            }
        }
        for ag in self.agents.iter_mut() { ag.handle_timeout(self.now); }
        self.now += 5;
    }
    fn emit(&mut self, out: Vec<Out>) {
        for o in out {
            match o {
                Out::Udp { from, to, data } => self.wire.push_back((from, to, data)),
                Out::Stream { agent, data } => {
                    let now = self.now;
                    for (from, d) in self.agents[agent].receive_stream(now, 0, &data) { self.got[agent].push((from, d)); }
                }
            }
        }
    }
    fn run_until(&mut self, ms: u64, done: impl Fn(&Sim) -> bool) -> bool {
        let end = self.now + ms;
        while self.now < end {
            self.step();
            if done(self) { return true; }
        }
        false
    }
    fn connected(&self) -> bool { self.agents.iter().all(|a| a.state() == State::Connected) }
}

fn seed(n: u8) -> [u8; 32] { [n; 32] }

fn pair(hosts: [&[&str]; 2]) -> Sim {
    let mut agents = Vec::new();
    for (i, hs) in hosts.iter().enumerate() {
        let mut ag = Agent::new(Config::new(i == 0, seed(i as u8 + 1)));
        for h in hs.iter() { ag.add_host(0, a(h)); }
        agents.push(ag);
    }
    let mut s = Sim::new(agents);
    s.hosts = hosts.iter().map(|hs| hs.iter().map(|h| a(h)).collect()).collect();
    s
}

fn start(s: &mut Sim) {
    s.exchange_credentials();
    let now = s.now;
    for ag in s.agents.iter_mut() { ag.start(now); }
}

fn data_both_ways(s: &mut Sim) {
    let t0 = s.agents[0].send(b"\x16\xfe\xfd hello from 0").unwrap();
    let t1 = s.agents[1].send(b"\x16\xfe\xfd hello from 1").unwrap();
    for (i, t) in [(0usize, t0), (1, t1)] {
        match t.route {
            Route::Udp { from, to } => { let f = s.public(from); s.wire.push_back((f, to, t.data)); }
            Route::Stream { .. } => {
                let f = s.framers.entry(i).or_default();
                f.push(&t.data);
                let mut out = Vec::new();
                while let Some(m) = f.next() { s.turn.as_mut().unwrap().from_client(Client::Stream(i), &m, &mut out); }
                s.emit(out);
            }
        }
    }
    s.run_until(200, |s| s.got.iter().all(|g| !g.is_empty()));
    assert_eq!(s.got[0].last().unwrap().1, b"\x16\xfe\xfd hello from 1");
    assert_eq!(s.got[1].last().unwrap().1, b"\x16\xfe\xfd hello from 0");
}

#[test]
fn two_agents_connect() {
    let mut s = pair([&["10.0.0.1:5000", "[fd00::1]:5000"], &["10.0.0.2:6000", "[fd00::2]:6000"]]);
    start(&mut s);
    assert!(s.run_until(5_000, Sim::connected), "{:?}", s.events);
    s.step();
    // IPv6 host pairs rank first.
    let (l0, r0) = s.agents[0].selected().unwrap();
    assert_eq!((l0, r0), (a("[fd00::1]:5000"), a("[fd00::2]:6000")));
    assert_eq!(s.agents[1].selected().unwrap(), (r0, l0));
    assert!(s.events[0].contains(&Event::GatheringDone));
    assert!(s.events[0].iter().any(|e| matches!(e, Event::Selected { .. })));
    data_both_ways(&mut s);
    // Consent keeps it up for a minute.
    assert!(!s.run_until(60_000, |s| !s.connected()));
}

#[test]
fn consent_runs_out() {
    let mut s = pair([&["10.0.0.1:5000"], &["10.0.0.2:6000"]]);
    start(&mut s);
    assert!(s.run_until(5_000, Sim::connected));
    // The peer goes away.
    s.blackhole.insert(a("10.0.0.2:6000"));
    let lost_at = s.now;
    assert!(s.run_until(15_000, |s| s.agents[0].state() == State::Disconnected));
    assert!(s.run_until(40_000, |s| s.agents[0].state() == State::Failed));
    let after = s.now - lost_at;
    assert!((30_000..36_000).contains(&after), "{}", after);
}

#[test]
fn stun_server_gives_a_reflexive_candidate() {
    let mut ag = Agent::new(Config::new(true, seed(9)));
    ag.add_host(0, a("192.168.1.10:5000"));
    ag.add_stun_server(a("198.51.100.1:3478"));
    let mut s = Sim::new(vec![ag]);
    s.hosts = vec![vec![a("192.168.1.10:5000")]];
    s.nat.push((a("192.168.1.10:5000"), a("203.0.113.5:40000")));
    s.stun = Some(a("198.51.100.1:3478"));
    s.trickle = false;
    let now = s.now;
    s.agents[0].start(now);
    assert!(s.run_until(2_000, |s| s.events[0].contains(&Event::GatheringDone)));
    let srflx: Vec<_> = s.events[0].iter().filter_map(|e| match e { Event::Candidate(c) if c.kind == Kind::ServerReflexive => Some(c.clone()), _ => None }).collect();
    assert_eq!(srflx.len(), 1);
    assert_eq!((srflx[0].address, srflx[0].base, srflx[0].related), (a("203.0.113.5:40000"), a("192.168.1.10:5000"), Some(a("192.168.1.10:5000"))));
    let line = srflx[0].to_sdp().to_line();
    assert!(line.contains(" 203.0.113.5 40000 typ srflx raddr 192.168.1.10 rport 5000"), "{}", line);
    assert!(srflx[0].priority < s.agents[0].local_candidates()[0].priority);

    // A server that never answers: gathering still finishes, after the retries.
    let mut ag = Agent::new(Config::new(true, seed(10)));
    ag.add_host(0, a("192.168.1.10:5000"));
    ag.add_stun_server(a("198.51.100.99:3478"));
    let mut s = Sim::new(vec![ag]);
    s.hosts = vec![vec![a("192.168.1.10:5000")]];
    s.trickle = false;
    let now = s.now;
    s.agents[0].start(now);
    assert!(s.run_until(30_000, |s| s.events[0].contains(&Event::GatheringDone)));
}

#[test]
fn through_a_nat_with_stun() {
    // Agent 0 sits behind a NAT; its host address is unreachable from agent 1.
    let mut s = pair([&["192.168.1.10:5000"], &["198.51.100.7:6000"]]);
    s.nat.push((a("192.168.1.10:5000"), a("203.0.113.5:40000")));
    s.agents[0].add_stun_server(a("198.51.100.1:3478"));
    s.stun = Some(a("198.51.100.1:3478"));
    s.blackhole.insert(a("192.168.1.10:5000"));
    start(&mut s);
    assert!(s.run_until(10_000, Sim::connected), "{:?} {:?}", s.agents[0].state(), s.agents[1].state());
    assert!(s.stun_hits >= 1);
    // Agent 1 sees agent 0 at its public address.
    assert_eq!(s.agents[1].selected().unwrap().1, a("203.0.113.5:40000"));
    data_both_ways(&mut s);
}

fn relayed(transport: turn::Transport) {
    let mut s = pair([&["192.168.1.10:5000"], &["198.51.100.7:6000"]]);
    s.agents[0] = {
        let mut c = Config::new(true, seed(1));
        c.relay_only = true;
        let mut ag = Agent::new(c);
        ag.add_host(0, a("192.168.1.10:5000"));
        ag.add_turn_server(turn::Config { server: a("192.0.2.50:3478"), username: USER.into(), password: PASS.into(), transport });
        ag
    };
    s.nat.push((a("192.168.1.10:5000"), a("203.0.113.5:40000")));
    s.turn = Some(FakeTurn::new(a("192.0.2.50:3478")));
    start(&mut s);
    assert!(s.run_until(10_000, Sim::connected), "{:?} {:?} {:?}", s.agents[0].state(), s.agents[1].state(), s.events[0]);
    let locals = s.agents[0].local_candidates();
    assert!(locals.iter().all(|c| c.kind == Kind::Relay), "relay only");
    let relay = locals[0].address;
    assert_eq!(relay, a("192.0.2.50:50000"));
    if !transport.stream() { assert_eq!(locals[0].related, Some(a("203.0.113.5:40000"))); }
    assert_eq!(s.agents[0].selected().unwrap(), (relay, a("198.51.100.7:6000")));
    assert_eq!(s.agents[1].selected().unwrap().1, relay);
    // After selection the client binds a channel: data then goes in ChannelData.
    s.run_until(500, |_| false);
    assert_eq!(s.turn.as_ref().unwrap().allocs.values().next().unwrap().channels.len(), 1);
    data_both_ways(&mut s);
    // Refreshes keep it for longer than the 600 s lifetime; permissions and channels too.
    assert!(!s.run_until(700_000, |s| !s.connected()));
    assert!(s.turn.as_ref().unwrap().allocs.len() == 1);
    let now = s.now;
    s.agents[0].close(now);
    s.step();
    assert!(s.turn.as_ref().unwrap().allocs.is_empty(), "allocation given back");
}

#[test]
fn relayed_over_udp() { relayed(turn::Transport::Udp); }

#[test]
fn relayed_over_tcp() { relayed(turn::Transport::Tcp); }

#[test]
fn turn_wrong_password() {
    let mut s = pair([&["192.168.1.10:5000"], &["198.51.100.7:6000"]]);
    let mut ag = Agent::new(Config::new(true, seed(1)));
    ag.add_host(0, a("192.168.1.10:5000"));
    ag.add_turn_server(turn::Config { server: a("192.0.2.50:3478"), username: USER.into(), password: "nope".into(), transport: turn::Transport::Udp });
    s.agents[0] = ag;
    s.turn = Some(FakeTurn::new(a("192.0.2.50:3478")));
    start(&mut s);
    assert!(s.run_until(3_000, |s| s.events[0].iter().any(|e| matches!(e, Event::TurnFailed { code: 401, .. }))), "{:?}", s.events[0]);
    // The host path still connects.
    assert!(s.run_until(5_000, Sim::connected));
}

#[test]
fn role_conflict_resolves() {
    // Both think they control (both made offers, say).
    let mut s = pair([&["10.0.0.1:5000"], &["10.0.0.2:6000"]]);
    s.agents[1] = { let mut ag = Agent::new(Config::new(true, seed(2))); ag.add_host(0, a("10.0.0.2:6000")); ag };
    start(&mut s);
    assert!(s.run_until(10_000, Sim::connected));
    assert_ne!(s.agents[0].controlling(), s.agents[1].controlling());
    data_both_ways(&mut s);
}

#[test]
fn lite_peer_and_late_trickle() {
    // The peer's candidates arrive after checks could have begun, from its SDP.
    let mut s = pair([&["10.0.0.1:5000"], &["10.0.0.2:6000"]]);
    s.trickle = false;
    s.agents[0].set_remote_lite(true);
    assert!(s.agents[0].controlling());
    start(&mut s);
    s.run_until(300, |_| false);
    assert_eq!(s.agents[0].state(), State::New);
    let sdp = "candidate:1 1 udp 2130706431 10.0.0.2 6000 typ host";
    let now = s.now;
    assert!(s.agents[0].add_remote_candidate_line(now, sdp));
    assert!(!s.agents[0].add_remote_candidate_line(now, "candidate:2 1 tcp 2130706431 10.0.0.2 443 typ host tcptype passive"));
    assert!(!s.agents[0].add_remote_candidate_line(now, "candidate:3 1 udp 2130706431 abcd.local 6000 typ host"));
    assert!(s.run_until(3_000, Sim::connected));
    // Agent 1 learned agent 0 as peer-reflexive from its checks.
    assert_eq!(s.agents[1].selected().unwrap().1, a("10.0.0.1:5000"));
}

#[test]
fn checks_with_bad_credentials_are_refused() {
    let mut s = pair([&["10.0.0.1:5000"], &["10.0.0.2:6000"]]);
    s.exchange_credentials();
    let ufrag = s.agents[1].local_credentials().0.to_string();
    s.agents[0].set_remote_credentials(&ufrag, "not-the-password-at-all");
    let now = s.now;
    for ag in s.agents.iter_mut() { ag.start(now); }
    // Agent 0's checks fail integrity at agent 1, so no pair works for the controlling
    // side and it never nominates; agent 1 waits for a nomination that never comes.
    assert!(!s.run_until(15_000, |s| s.agents.iter().any(|a| a.state() == State::Connected)));
}
